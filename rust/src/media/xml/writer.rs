//! Rows rendered as the elements of one XML document.

use std::io::Write;

use arrow_array::RecordBatch;
use smol_str::SmolStr;

use crate::arrow::BatchReader;
use crate::text::xml::wire::{self, Layout};
use crate::text::{Formatting, Indent};
use crate::{Error, Result, Scalar};

/// How a document lays its row elements out.
///
/// The rows are the level a data file is read at, so the default is the one
/// that makes a file readable without making its rows verbose: one row element
/// per line, indented one level, with the row's own children beside it. An
/// explicit width lays the children out too, and no indent at all writes the
/// document without a byte of layout.
#[derive(Clone, Copy)]
pub(crate) struct RowLayout {
    /// What one document level costs, or `None` for no layout.
    unit: Option<&'static [u8]>,
    /// How one row's own children are laid out.
    row: Layout,
}

impl From<Formatting> for RowLayout {
    fn from(value: Formatting) -> Self {
        Self {
            unit: match value.indent() {
                Indent::None => None,
                Indent::Default => Some(b"  "),
                indent => indent.unit(),
            },
            row: Layout::from(value),
        }
    }
}

impl RowLayout {
    /// Return how many bytes introduce one row element.
    pub(crate) fn row_prefix_size(self) -> usize {
        self.unit.map_or(0, |unit| unit.len() + 1)
    }

    /// Return how many bytes introduce the document element's end tag.
    pub(crate) const fn close_prefix_size(self) -> usize {
        if self.unit.is_some() { 1 } else { 0 }
    }

    /// Write the newline and indent one row element opens with.
    fn open_row<W: Write>(self, target: &mut W) -> Result<()> {
        if let Some(unit) = self.unit {
            target.write_all(b"\n")?;
            target.write_all(unit)?;
        }
        Ok(())
    }

    /// Write the newline the document element's end tag opens with.
    fn close_document<W: Write>(self, target: &mut W) -> Result<()> {
        if self.unit.is_some() {
            target.write_all(b"\n")?;
        }
        Ok(())
    }
}

/// Write the document element's start tag.
pub(crate) fn write_document_start<W: Write>(target: &mut W, root: &str) -> Result<()> {
    wire::check_name(root)?;
    write!(target, "<{root}>")?;
    Ok(())
}

/// Write the document element's end tag.
pub(crate) fn write_document_end<W: Write>(
    target: &mut W,
    root: &str,
    layout: RowLayout,
) -> Result<()> {
    layout.close_document(target)?;
    write!(target, "</{root}>")?;
    Ok(())
}

/// Write one row value as its own element, after the layout that leads it.
pub(crate) fn write_row<W: Write>(
    target: &mut W,
    row: &str,
    value: &Scalar,
    layout: RowLayout,
) -> Result<()> {
    layout.open_row(target)?;
    write_row_body(target, row, value, layout)
}

/// Write one row element alone, which is exactly what its span holds.
pub(crate) fn write_row_body<W: Write>(
    target: &mut W,
    row: &str,
    value: &Scalar,
    layout: RowLayout,
) -> Result<()> {
    wire::write_element(target, row, value, layout.row, 1)
}

/// Write every row of one batch.
///
/// The batch's own column names are the element names, so the document a write
/// produces reads back as the columns that were written.
pub(crate) fn write_batch<W: Write>(
    target: &mut W,
    row: &str,
    batch: &RecordBatch,
    layout: RowLayout,
) -> Result<()> {
    let names = column_names(batch);
    let rows = crate::arrow::batch_to_value(batch)?;
    let rows = rows.as_sequence().ok_or_else(|| Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: SmolStr::new_static("expected a batch to read as a sequence of rows"),
    })?;
    for (index, values) in rows.iter().enumerate() {
        let values = values.as_sequence().ok_or_else(|| Error::InvalidRecord {
            path: smol_str::format_smolstr!("$[{index}]"),
            reason: SmolStr::new_static("expected one row to read as a sequence of values"),
        })?;
        let entries: Vec<(&str, &Scalar)> = names
            .iter()
            .map(SmolStr::as_str)
            .zip(values.iter())
            .collect();
        layout.open_row(target)?;
        wire::write_children(target, row, entries.into_iter(), layout.row, 1)?;
    }
    Ok(())
}

/// Write a complete document holding every row the reader yields.
pub(crate) fn write_document<W: Write>(
    target: &mut W,
    batches: BatchReader,
    root: &str,
    row: &str,
    layout: RowLayout,
) -> Result<()> {
    write_document_start(target, root)?;
    for batch in batches {
        let batch = batch.map_err(crate::arrow::from_reader_error)?;
        write_batch(target, row, &batch, layout)?;
    }
    write_document_end(target, root, layout)
}

fn column_names(batch: &RecordBatch) -> Vec<SmolStr> {
    batch
        .schema()
        .fields()
        .iter()
        .map(|field| SmolStr::new(field.name()))
        .collect()
}
