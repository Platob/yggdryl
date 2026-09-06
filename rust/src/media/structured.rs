//! Structured text as Arrow rows.
//!
//! JSON, JSON Lines, YAML, and TOML are not record encodings: they carry
//! documents, not typed columns, so [`RecordOptions`](super::RecordOptions)
//! does not name them and no reader in [`crate::iobase`] speaks them. This
//! module is the one bridge between them and Arrow, and it holds no format
//! knowledge of its own - framing, limits, and every leaf spelling stay with
//! [`crate::text`], and every value crossing stays with [`crate::arrow`].
//!
//! The two directions are deliberately asymmetric, because the formats are:
//!
//! | Direction | JSON, TOML | JSON Lines, YAML |
//! | --- | --- | --- |
//! | Read | one document, then one batch | every document, then one batch |
//! | Write | one document, rows held | streamed, one batch of rows at a time |
//!
//! A text document has no frame to read a prefix of, so a read holds the
//! parsed document. A write does have one wherever the format is
//! document-per-row, and there nothing but the current batch is held.

use std::collections::VecDeque;

use smol_str::SmolStr;

use crate::arrow::{ArrowValue, BatchReader};
use crate::text::{Formatting, Plan, Structured};
use crate::{Error, Field, IOBase, Result, Scalar};

/// Read a handle's structured text document as one Arrow value.
///
/// `field` is the root the rows land under. Without one the rows name their
/// own root, inferred from what the document proves - which is the same rule
/// every other schema-free read in the project follows.
///
/// Rows come from the document's own shape: one document that is a sequence
/// holds them, and every other document is one row. TOML has no top-level
/// sequence, so its rows are the array of tables stored under the root's name.
///
/// # Errors
///
/// Returns a read, decompression, format, parse, inference, or cast failure.
pub(crate) fn read_arrow_value<H: IOBase + ?Sized>(
    handle: &H,
    field: Option<&Field>,
) -> Result<ArrowValue> {
    let format = Structured::for_handle(handle)?;
    let name = field.map_or(crate::media::DEFAULT_ROOT_NAME, Field::name);
    let documents = crate::text::from_io_all(handle)?;
    let rows = rows_of(documents, format, name);

    let rows = Scalar::from_sequence(rows);
    let root = match field {
        Some(field) => field.clone(),
        None => rows.inferred_struct_field()?,
    };
    let canonical = rows
        .as_sequence()
        .unwrap_or_default()
        .iter()
        .map(|row| root.from_natural_value(row.clone()))
        .collect::<Result<Vec<_>>>()?;
    Ok(ArrowValue::from_rows(
        &root,
        &Scalar::from_sequence(canonical),
    )?)
}

/// Replace a handle's contents with one Arrow value as structured text.
///
/// Rows are restated in their natural named shape by
/// [`Field::into_natural_value`](crate::Field::into_natural_value), so a row
/// renders as the object the format is expected to contain rather than as a
/// positional array.
///
/// A document-per-row format - JSON Lines and YAML - never holds more than the
/// batch currently being encoded. JSON writes one array and TOML one array of
/// tables under the root's name, so both hold the rows they are framing.
///
/// # Errors
///
/// Returns a schema, value, encoding, compression, or write failure.
pub(crate) fn write_arrow_value<H: IOBase + ?Sized>(
    handle: &mut H,
    value: ArrowValue,
    formatting: Formatting,
) -> Result<()> {
    let plan = Plan::infer(handle)?;
    let format = Structured::from_format(plan.format());
    let root = value.root()?;
    let name = SmolStr::new(root.name());
    let batches = value.into_reader()?;

    let mut encoded = Vec::new();
    {
        let mut writer = plan
            .codec()
            .writer_with_level(&mut encoded, formatting.level());
        match format {
            // Document per row: the framing is per value, so the rows travel
            // as an iterator and only the current batch is ever held.
            Structured::Jsonl | Structured::Yaml => {
                let mut rows = Rows::new(batches, root);
                crate::text::into_writer_all_with_formatting(
                    &mut rows,
                    &mut writer,
                    plan.format(),
                    formatting,
                )?;
                rows.into_result()?;
            }
            // One document: the frame encloses every row, so they are held.
            Structured::Json | Structured::Toml => {
                let mut rows = Rows::new(batches, root);
                let held = rows.by_ref().collect::<Vec<_>>();
                rows.into_result()?;
                let document = Scalar::from_sequence(held);
                let document = if matches!(format, Structured::Toml) {
                    Scalar::from_record([(name, document)])?
                } else {
                    document
                };
                crate::text::into_writer_with_formatting(
                    &document,
                    &mut writer,
                    plan.format(),
                    formatting,
                )?;
            }
        }
        writer.finish()?;
    }
    handle.write_all_bytes(&encoded)
}

/// Select the rows a parsed document set holds.
fn rows_of(documents: Vec<Scalar>, format: Structured, name: &str) -> Vec<Scalar> {
    let [document] = documents.as_slice() else {
        // Several documents are several rows, which is what a document-per-row
        // format writes.
        return documents;
    };
    if matches!(format, Structured::Toml) {
        // TOML has no top-level sequence: an array of tables under the root's
        // name is the shape a table takes, and anything else is one row.
        return match document.get_key_str(name).and_then(Scalar::as_sequence) {
            Some(rows) => rows.to_vec(),
            None => documents,
        };
    }
    match document.as_sequence() {
        Some(rows) => rows.to_vec(),
        None => documents,
    }
}

/// The rows of a batch stream, in their natural named shape, one at a time.
///
/// The writer this feeds takes an iterator with no failure channel, so a row
/// that cannot be restated stops the iteration and is reported by
/// [`Self::into_result`] afterwards - never swallowed, and never rendered as a
/// partial document, because nothing reaches the handle until the encoder is
/// finished.
struct Rows {
    batches: Option<BatchReader>,
    root: Field,
    buffered: VecDeque<Scalar>,
    failure: Option<Error>,
}

impl Rows {
    const fn new(batches: BatchReader, root: Field) -> Self {
        Self {
            batches: Some(batches),
            root,
            buffered: VecDeque::new(),
            failure: None,
        }
    }

    /// Report the failure that stopped the iteration, if one did.
    fn into_result(self) -> Result<()> {
        self.failure.map_or(Ok(()), Err)
    }

    /// Refill the buffer from the next batch, or report the end of the stream.
    fn fill(&mut self) -> Result<bool> {
        let Some(batches) = self.batches.as_mut() else {
            return Ok(false);
        };
        let Some(batch) = batches.next() else {
            self.batches = None;
            return Ok(false);
        };
        let batch = batch.map_err(crate::arrow::from_reader_error)?;
        let rows = crate::arrow::batch_to_value(&batch)?;
        let Some(rows) = rows.as_sequence() else {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: SmolStr::new_static("a record batch reads back as a sequence of rows"),
            });
        };
        for row in rows {
            self.buffered
                .push_back(self.root.into_natural_value(row.clone())?);
        }
        Ok(true)
    }
}

impl Iterator for Rows {
    type Item = Scalar;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(row) = self.buffered.pop_front() {
                return Some(row);
            }
            if self.failure.is_some() {
                return None;
            }
            match self.fill() {
                // An empty batch is legal and says nothing about the end.
                Ok(true) => {}
                Ok(false) => return None,
                Err(error) => {
                    self.failure = Some(error);
                    return None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
