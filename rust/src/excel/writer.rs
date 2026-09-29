//! The streamed worksheet writer: a worksheet part rendered row by row off
//! the record columns of a stream, as the archive reads it.
//!
//! What Excel owns is spelled here and nowhere else: a boolean as `t="b"`, a
//! date, time, naive datetime or duration as its serial under the matching
//! style, a float that is not a number as the `#NUM!` error, and text as an
//! inline string (`t="inlineStr"`), so a stream of a million distinct strings
//! costs one batch and no table. Every other leaf is spelled as the XML codec
//! spells it and written as text: a decimal's digits, a code, an identifier,
//! a zoned datetime in ISO 8601 with its offset, bytes in base64. A nested
//! value is its JSON text, because a cell is flat.
//!
//! The writer is a [`Read`]: the archive pulls it as it deflates the member,
//! so the part is never held whole. It writes no `dimension`, which cannot be
//! known before the last row, and `sheetData` even when empty.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use smol_str::format_smolstr;

use crate::{DataType, Error, Field, Result, Scalar, Serie, SerieReader};

use super::cell::{CellRef, DateSystem, MAX_COLUMNS, MAX_ROWS, cell_text};

/// How one column's cells are written, decided once per stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// `t="b"`.
    Boolean,
    /// A serial under the format's style, or text where the value is zoned.
    Temporal,
    /// Digits, or `#NUM!` for NaN and infinity.
    Float,
    /// Text read off the column's own storage, no value built.
    Text,
    /// Any other leaf, spelled as the XML codec spells it.
    Leaf,
    /// A struct, a sequence or a map, as JSON text.
    Nested,
}

impl Kind {
    fn of(dtype: &DataType) -> Self {
        if matches!(dtype, DataType::Boolean) {
            return Self::Boolean;
        }
        if matches!(
            dtype,
            DataType::Float16 | DataType::Float32 | DataType::Float64
        ) {
            return Self::Float;
        }
        if dtype
            .id()
            .temporal_kind()
            .is_some_and(|kind| !matches!(kind, crate::TemporalKind::Interval))
        {
            return Self::Temporal;
        }
        if dtype.string_parameters().is_some() {
            return Self::Text;
        }
        if dtype.is_nested() {
            return Self::Nested;
        }
        Self::Leaf
    }
}

/// Whether a text column's storage bytes are its UTF-8: true for the UTF-8
/// and US-ASCII leaves, false for windows-1252, which rides binary storage.
fn utf8_stored(child: &Serie) -> bool {
    child
        .field()
        .and_then(|field| field.dtype().string_parameters())
        .is_some_and(|parameters| {
            matches!(
                parameters.charset(),
                crate::Charset::Utf8 | crate::Charset::Ascii
            )
        })
}

/// The worksheet part of a record stream, rendered as it is read.
pub(crate) struct SheetXml {
    batches: Option<SerieReader>,
    root: Field,
    kinds: Vec<Kind>,
    system: DateSystem,
    /// The crate's temporal styles sit after the styles already in the
    /// workbook this sheet joins.
    style_offset: u32,
    /// The top-left cell the rows start at.
    anchor: CellRef,
    /// Whether the anchor's row names the columns.
    header: bool,
    /// The zero-based row the next record is written at.
    row: u32,
    /// The sheet's name, for the refusal a row past the grid gets.
    sheet: smol_str::SmolStr,
    pending: VecDeque<u8>,
    scratch: Vec<u8>,
    reference: String,
    stage: Stage,
    /// Where a failure lands, shared with the caller: the archive reading
    /// this stream sees an `io::Error`, the caller the typed refusal.
    failure: Arc<Mutex<Option<Error>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Prologue,
    Rows,
    Epilogue,
    Done,
}

impl SheetXml {
    /// The part for `batches`, whose records are `root`'s, starting at
    /// `anchor` in sheet `sheet`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the root holds more columns than
    /// fit from the anchor, before a byte is rendered.
    pub(crate) fn new(
        batches: SerieReader,
        root: Field,
        sheet: impl Into<smol_str::SmolStr>,
        system: DateSystem,
        style_offset: u32,
        anchor: CellRef,
        header: bool,
    ) -> Result<Self> {
        let columns = root.fields().len();
        if u64::from(anchor.column()) + columns as u64 > u64::from(MAX_COLUMNS) {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected at most {} columns from {anchor}, got {columns}",
                    MAX_COLUMNS - anchor.column()
                ),
            });
        }
        let kinds = root
            .fields()
            .iter()
            .map(|field| Kind::of(field.dtype()))
            .collect();
        Ok(Self {
            batches: Some(batches),
            root,
            kinds,
            system,
            style_offset,
            anchor,
            header,
            row: anchor.row(),
            sheet: sheet.into(),
            pending: VecDeque::new(),
            scratch: Vec::with_capacity(crate::DEFAULT_STREAM_BATCH_SIZE),
            reference: String::with_capacity(12),
            stage: Stage::Prologue,
            failure: Arc::new(Mutex::new(None)),
        })
    }

    /// The slot a failure lands in, to read once the archive has drained the
    /// stream.
    pub(crate) fn failure(&self) -> Arc<Mutex<Option<Error>>> {
        Arc::clone(&self.failure)
    }

    /// Render the next piece into the scratch buffer.
    fn render(&mut self) -> Result<bool> {
        self.scratch.clear();
        match self.stage {
            Stage::Prologue => {
                write!(
                    self.scratch,
                    "<worksheet xmlns=\"{}\" xmlns:r=\"{}\"><sheetData>",
                    super::NAMESPACE,
                    super::RELATIONSHIPS_NAMESPACE
                )?;
                if self.header {
                    self.open_row()?;
                    for (offset, field) in self.root.fields().iter().enumerate() {
                        self.reference.clear();
                        CellRef::new(self.row, self.anchor.column() + offset as u32)
                            .write_a1(&mut self.reference);
                        write!(
                            self.scratch,
                            "<c r=\"{}\" t=\"inlineStr\"><is>",
                            self.reference
                        )?;
                        super::shared_strings::write_text_element(&mut self.scratch, field.name())?;
                        write!(self.scratch, "</is></c>")?;
                    }
                    write!(self.scratch, "</row>")?;
                    self.row += 1;
                }
                self.stage = Stage::Rows;
                Ok(true)
            }
            Stage::Rows => {
                let Some(batch) = self.batches.as_mut().and_then(Iterator::next) else {
                    self.batches = None;
                    self.stage = Stage::Epilogue;
                    return Ok(true);
                };
                let batch = batch?;
                self.render_batch(&batch)?;
                Ok(true)
            }
            Stage::Epilogue => {
                write!(self.scratch, "</sheetData></worksheet>")?;
                self.stage = Stage::Done;
                Ok(true)
            }
            Stage::Done => Ok(false),
        }
    }

    fn open_row(&mut self) -> Result<()> {
        if self.row >= MAX_ROWS {
            return Err(Error::InvalidRecord {
                path: format_smolstr!(
                    "{}!{}",
                    self.sheet,
                    CellRef::new(self.row, self.anchor.column())
                ),
                reason: format_smolstr!(
                    "expected at most {MAX_ROWS} rows in a worksheet, got a row {} past them",
                    self.row + 1
                ),
            });
        }
        write!(self.scratch, "<row r=\"{}\">", self.row + 1)?;
        Ok(())
    }

    /// Render every row of one record column.
    fn render_batch(&mut self, batch: &Serie) -> Result<()> {
        let Some(record) = batch.as_struct() else {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected a record column for the rows of a sheet, got {}",
                    batch.field().map_or("a run", Field::name)
                ),
            });
        };
        let children = record.children();
        if children.len() != self.kinds.len() {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected {} columns for the sheet, got {}",
                    self.kinds.len(),
                    children.len()
                ),
            });
        }
        for index in 0..batch.len() {
            self.open_row()?;
            for (offset, child) in children.iter().enumerate() {
                // Copied out before the cell is written, so the row loop
                // borrows nothing of the writer and allocates nothing.
                let kind = self.kinds[offset];
                let column = self.anchor.column() + offset as u32;
                self.reference.clear();
                CellRef::new(self.row, column).write_a1(&mut self.reference);
                let reference = std::mem::take(&mut self.reference);
                let written = self.write_cell(child, index, kind, &reference);
                self.reference = reference;
                written.map_err(|error| Error::InvalidRecord {
                    path: format_smolstr!("{}!{}", self.sheet, CellRef::new(self.row, column)),
                    reason: super::cell::wire_reason(&error),
                })?;
            }
            write!(self.scratch, "</row>")?;
            self.row += 1;
        }
        Ok(())
    }

    /// Write the cell of `child` at `index`, nothing for a null.
    fn write_cell(
        &mut self,
        child: &Serie,
        index: usize,
        kind: Kind,
        reference: &str,
    ) -> Result<()> {
        if child.is_null(index)? {
            return Ok(());
        }
        match kind {
            // UTF-8 and US-ASCII ride Arrow's text storage, whose bytes are
            // the text; a windows-1252 leaf's bytes are not, and its value
            // is read through the leaf below.
            Kind::Text if child.is_string_storage() && utf8_stored(child) => {
                let Some(bytes) = child.value_bytes(index) else {
                    return Ok(());
                };
                let text = std::str::from_utf8(bytes).map_err(|error| Error::InvalidRecord {
                    path: smol_str::SmolStr::new_static("$"),
                    reason: format_smolstr!("the column holds bytes that are not UTF-8: {error}"),
                })?;
                // A fixed slot's NUL padding is the storage's, not the text's.
                let text = text.trim_end_matches('\0');
                self.write_text_cell(reference, text)
            }
            Kind::Boolean => {
                let value = child.scalar(index)?;
                write!(
                    self.scratch,
                    "<c r=\"{reference}\" t=\"b\"><v>{}</v></c>",
                    u8::from(value.as_bool().unwrap_or(false))
                )?;
                Ok(())
            }
            Kind::Float => {
                let value = child.scalar(index)?;
                let number = value.as_f64().unwrap_or(f64::NAN);
                if number.is_finite() {
                    // A whole number is written as Excel writes it, `1` for
                    // 1.0: what a reader declaring an integer column parses.
                    if number.fract() == 0.0 && number.abs() < 1e15 {
                        write!(
                            self.scratch,
                            "<c r=\"{reference}\"><v>{}</v></c>",
                            number as i64
                        )?;
                    } else {
                        let mut buffer = ryu::Buffer::new();
                        write!(
                            self.scratch,
                            "<c r=\"{reference}\"><v>{}</v></c>",
                            buffer.format(number)
                        )?;
                    }
                } else {
                    write!(
                        self.scratch,
                        "<c r=\"{reference}\" t=\"e\"><v>#NUM!</v></c>"
                    )?;
                }
                Ok(())
            }
            Kind::Temporal => {
                let value = child.scalar(index)?;
                match self.system.serial_of(&value)? {
                    Some((serial, format)) => {
                        write!(
                            self.scratch,
                            "<c r=\"{reference}\" s=\"{}\"><v>{}</v></c>",
                            format.style_index() + self.style_offset,
                            super::cell::serial_text(serial)
                        )?;
                        Ok(())
                    }
                    // A zoned datetime, an interval: text, with what it carries.
                    None => self.write_text_cell(reference, &cell_text(&value)),
                }
            }
            Kind::Nested => {
                let value = child.scalar(index)?;
                let text = value.into_json()?;
                self.write_text_cell(reference, &text)
            }
            Kind::Text | Kind::Leaf => {
                let value = child.scalar(index)?;
                match &value {
                    Scalar::Null => Ok(()),
                    crate::string_scalars!(text) => self.write_text_cell(reference, text.as_str()),
                    Scalar::Int8(_)
                    | Scalar::Int16(_)
                    | Scalar::Int32(_)
                    | Scalar::Int64(_)
                    | Scalar::UInt8(_)
                    | Scalar::UInt16(_)
                    | Scalar::UInt32(_)
                    | Scalar::UInt64(_)
                    | Scalar::Int128(_)
                    | Scalar::UInt128(_)
                    | Scalar::Decimal32(_)
                    | Scalar::Decimal64(_)
                    | Scalar::Decimal128(_)
                    | Scalar::Decimal256(_)
                    | Scalar::Decimal(_)
                    | Scalar::BigDecimal(_) => {
                        write!(self.scratch, "<c r=\"{reference}\"><v>")?;
                        crate::xml::write_leaf_text(&mut self.scratch, &value, "cell")?;
                        write!(self.scratch, "</v></c>")?;
                        Ok(())
                    }
                    _ => self.write_text_cell(reference, &cell_text(&value)),
                }
            }
        }
    }

    /// Write an inline string cell.
    fn write_text_cell(&mut self, reference: &str, text: &str) -> Result<()> {
        let length = text.chars().count();
        if length > super::cell::MAX_CELL_TEXT {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected at most {} characters in a cell, got {length}",
                    super::cell::MAX_CELL_TEXT
                ),
            });
        }
        write!(self.scratch, "<c r=\"{reference}\" t=\"inlineStr\"><is>")?;
        super::shared_strings::write_text_element(&mut self.scratch, text)?;
        write!(self.scratch, "</is></c>")?;
        Ok(())
    }
}

impl Read for SheetXml {
    fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
        if target.is_empty() {
            return Ok(0);
        }
        while self.pending.is_empty() {
            if self.stage == Stage::Done {
                return Ok(0);
            }
            match self.render() {
                Ok(true) => self.pending.extend(self.scratch.drain(..)),
                Ok(false) => return Ok(0),
                Err(error) => {
                    let message = error.to_string();
                    self.stage = Stage::Done;
                    if let Ok(mut slot) = self.failure.lock() {
                        *slot = Some(error);
                    }
                    return Err(std::io::Error::other(message));
                }
            }
        }
        let read = self.pending.len().min(target.len());
        for (slot, byte) in target.iter_mut().zip(self.pending.drain(..read)) {
            *slot = byte;
        }
        Ok(read)
    }
}
