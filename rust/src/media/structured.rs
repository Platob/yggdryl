//! Structured text as Arrow rows.
//!
//! JSON, JSON Lines, YAML, TOML, and XML are not record encodings: they carry
//! documents, not typed columns, so [`RecordOptions`](super::RecordOptions)
//! does not name them and no reader in [`crate::iobase`] speaks them. This
//! module is the one bridge between them and Arrow, and it holds no format
//! knowledge of its own - framing, limits, and every leaf spelling stay with
//! [`crate::text`], and every value crossing stays with [`crate::Serie`].
//!
//! The two directions are deliberately asymmetric, because the formats are:
//!
//! | Direction | JSON, TOML, XML | JSON Lines, YAML |
//! | --- | --- | --- |
//! | Read | one document, then one batch | every document, then one batch |
//! | Write | one document, rows held | streamed, one batch of rows at a time |
//!
//! A text document has no frame to read a prefix of, so a read holds the
//! parsed document. A write does have one wherever the format is
//! document-per-row, and there nothing but the current batch is held.

use std::collections::VecDeque;

use smol_str::SmolStr;

use crate::text::{Format, Formatting, Plan};
use crate::{Error, Field, IOBase, Result, Scalar, Serie, SerieReader};

/// Read a handle's structured text document as one record column.
///
/// `field` is the root the rows land under. Without one the rows name their
/// own root, inferred from what the document proves - which is the same rule
/// every other schema-free read in the project follows.
///
/// Rows come from the document's own shape: one document that is a sequence
/// holds them, and every other document is one row. TOML has no top-level
/// sequence, so its rows are the array of tables stored under the root's name;
/// an XML document is its root element, so its rows are the root's children
/// named after the root field, the root itself when it has none, and no row
/// at all when the root is empty.
///
/// # Errors
///
/// Returns a read, decompression, format, parse, inference, or cast failure.
pub(crate) fn read_arrow<H: IOBase + ?Sized>(handle: &H, field: Option<&Field>) -> Result<Serie> {
    let format = Format::from_handle(handle)?;
    let name = field.map_or(crate::media::DEFAULT_ROOT_NAME, Field::name);
    let documents = crate::text::from_io_all(handle)?;
    let rows = rows_of(documents, format, name, field.is_some());
    // XML states less than a field does - a child read once, text where a
    // number is declared - and its codec restates that before the contract.
    let shape = |row: Scalar, root: &Field| match format {
        Format::Xml => crate::xml::shaped(row, root),
        _ => row,
    };

    let rows = Scalar::from_sequence(rows);
    let root = match field {
        Some(field) => field.clone(),
        None => rows.inferred_struct_field()?,
    };
    // Every row goes through the field's own value contract, which is what
    // restates a document's number at the scale a decimal column declares and
    // its text at the unit a temporal one does - and is why the rows are laid
    // out with no second pass.
    let canonical = rows
        .sequence_rows()
        .unwrap_or_default()
        .iter()
        .map(|row| root.from_natural_value(shape(row.clone(), &root)))
        .collect::<Result<Vec<_>>>()?;
    let borrowed: Vec<&Scalar> = canonical.iter().collect();
    crate::serie::from_canonical_rows(std::sync::Arc::new(root), &borrowed)
}

/// Replace a handle's contents with a stream of record columns as
/// structured text.
///
/// Rows are restated in their natural named shape by
/// [`Field::into_natural_value`](crate::Field::into_natural_value), so a row
/// renders as the object the format is expected to contain rather than as a
/// positional array.
///
/// A document-per-row format - JSON Lines and YAML - never holds more than the
/// batch currently being encoded. JSON writes one array, TOML one array of
/// tables under the root's name, and XML one document element holding one
/// child element per row named after the root, so all three hold the rows
/// they are framing.
///
/// # Errors
///
/// Returns a schema, value, encoding, compression, or write failure.
pub(crate) fn write_arrow<H: IOBase + ?Sized>(
    handle: &mut H,
    value: SerieReader,
    formatting: Formatting,
) -> Result<()> {
    let plan = Plan::infer(handle)?;
    let format = plan.format();
    let root = value.field().clone();
    let name = SmolStr::new(root.name());
    let batches = value;

    let mut encoded = Vec::new();
    {
        let mut coded = plan
            .codec()
            .writer_with_level(&mut encoded, formatting.level());
        {
            // Rendered text is encoded in the declared charset, and only then
            // compressed: the coding applies to the bytes a reader will meet.
            let mut writer = plan.charset().writer(&mut coded);
            plan.write_prolog(&mut writer)?;
            match format {
                // Document per row: the framing is per value, so the rows
                // travel as an iterator and only the current batch is ever
                // held.
                Format::JsonLines | Format::Yaml => {
                    let mut rows = Rows::new(batches, root, format);
                    crate::text::into_writer_all_with_formatting(
                        &mut rows,
                        &mut writer,
                        plan.format(),
                        formatting,
                    )?;
                    rows.into_result()?;
                }
                // One document: the frame encloses every row, so they are held.
                Format::Json | Format::Toml | Format::Xml => {
                    let mut rows = Rows::new(batches, root, format);
                    let held = rows.by_ref().collect::<Vec<_>>();
                    rows.into_result()?;
                    let document = Scalar::from_sequence(held);
                    let document = match format {
                        Format::Toml => Scalar::from_struct([(name, document)])?,
                        Format::Xml => Scalar::from_struct([(
                            SmolStr::new_static(crate::xml::DOCUMENT_ELEMENT),
                            Scalar::from_struct([(name, document)])?,
                        )])?,
                        _ => document,
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
        coded.finish()?;
    }
    handle.write_all_bytes(&encoded)
}

/// Select the rows a parsed document set holds; `declared` says whether
/// `name` was a field's own or the default.
fn rows_of(documents: Vec<Scalar>, format: Format, name: &str, declared: bool) -> Vec<Scalar> {
    let [document] = documents.as_slice() else {
        // Several documents are several rows, which is what a document-per-row
        // format writes.
        return documents;
    };
    if matches!(format, Format::Toml) {
        // TOML has no top-level sequence: an array of tables under the root's
        // name is the shape a table takes, and anything else is one row.
        return match document.get_key_str(name).and_then(Scalar::as_sequence) {
            Some(rows) => rows.to_vec(),
            None => documents,
        };
    }
    if matches!(format, Format::Xml) {
        // An XML document is its root element. The rows are the root's
        // children named after the root field - one such child being one
        // row. Where no field named one, they are the root's children of its
        // one child name, when those are records; otherwise the root is
        // itself the one row, unless it is empty, which is what a stream of
        // no rows was written as.
        let Some(root) = document
            .as_struct()
            .and_then(|entries| entries.values().next())
        else {
            return documents;
        };
        return match root {
            // `<data/>` and `<data></data>`: a stream of no rows writes the
            // second, and neither holds one.
            Scalar::Null => Vec::new(),
            crate::string_scalars!(text) if text.as_str().is_empty() => Vec::new(),
            Scalar::Struct(entries) => {
                let entries = entries.as_map();
                let rows = entries.get(name).or_else(|| {
                    if declared {
                        return None;
                    }
                    let mut children = entries
                        .iter()
                        .filter(|(key, _)| !key.starts_with('@') && !key.starts_with('#'));
                    match (children.next(), children.next()) {
                        (Some((_, rows)), None) if rows.as_struct().is_some() => Some(rows),
                        (Some((_, rows)), None)
                            if rows.as_sequence().is_some_and(|rows| {
                                rows.iter().all(|row| row.as_struct().is_some())
                            }) =>
                        {
                            Some(rows)
                        }
                        _ => None,
                    }
                });
                match rows {
                    Some(rows) => match rows.as_sequence() {
                        Some(rows) => rows.to_vec(),
                        None => vec![rows.clone()],
                    },
                    None if entries.is_empty() => Vec::new(),
                    None => vec![root.clone()],
                }
            }
            other => vec![other.clone()],
        };
    }
    match document.as_sequence() {
        Some(rows) => rows.to_vec(),
        None => documents,
    }
}

/// The rows of a stream of record columns, in their natural named shape, one
/// at a time.
///
/// The writer this feeds takes an iterator with no failure channel, so a row
/// that cannot be restated stops the iteration and is reported by
/// [`Self::into_result`] afterwards - never swallowed, and never rendered as a
/// partial document, because nothing reaches the handle until the encoder is
/// finished.
struct Rows {
    batches: Option<SerieReader>,
    root: Field,
    format: Format,
    buffered: VecDeque<Scalar>,
    failure: Option<Error>,
}

impl Rows {
    const fn new(batches: SerieReader, root: Field, format: Format) -> Self {
        Self {
            batches: Some(batches),
            root,
            format,
            buffered: VecDeque::new(),
            failure: None,
        }
    }

    /// Report the failure that stopped the iteration, if one did.
    fn into_result(self) -> Result<()> {
        self.failure.map_or(Ok(()), Err)
    }

    /// Refill the buffer from the next column, or report the end of the
    /// stream.
    fn fill(&mut self) -> Result<bool> {
        let Some(batches) = self.batches.as_mut() else {
            return Ok(false);
        };
        let Some(batch) = batches.next() else {
            self.batches = None;
            return Ok(false);
        };
        let records = batch?;
        for row in 0..records.len() {
            let natural = self.root.into_natural_value(records.scalar(row)?)?;
            // A sequence inside a sequence has no element to repeat, so XML
            // restates it under the item's own name before the writer.
            self.buffered.push_back(match self.format {
                Format::Xml => crate::xml::natural(natural, &self.root),
                _ => natural,
            });
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
