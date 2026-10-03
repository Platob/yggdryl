//! Rendering record batches as CSV through the handle's coding and charset.
//!
//! Each batch lands as one record column and is walked row by row, each
//! column's cell rendered by the reading its leaf resolved once per batch: a
//! text leaf lends its bytes where they lie, every other leaf spells its
//! canonical text, a byte leaf as base64 and a nested value as compact JSON.
//! A cell that would not read back as it was written is quoted; nothing else
//! is. A null is its spelling verbatim, since a quoted cell is never absent,
//! so a spelling a record cannot hold as it stands is refused where a null
//! is written; and a record is never a blank line, which a reader skips as
//! the separator between records rather than reading it as one.

use std::io::Write;

use smol_str::{SmolStr, format_smolstr};

use crate::arrow::{BatchReader, field_from_arrow_schema};
use crate::media::IORecordOptions as _;
use crate::text::expected_got;
use crate::text::transport::transports;
use crate::{
    ArrowCastOptions, Charset, Codec, DataType, DataTypeKind, Error, Field, Result, Serie,
    SerieReader,
};

use super::options::CsvOptions;

/// Encode `batches` as one CSV document under the handle's coding.
///
/// The header is written first where `header` says so; nothing is handed
/// back until the last batch is encoded.
///
/// # Errors
///
/// Returns a schema, value, encoding or compression failure.
pub(crate) fn encoded(
    batches: BatchReader,
    options: &CsvOptions,
    charset: Charset,
    codec: Codec,
    header: bool,
) -> Result<Vec<u8>> {
    let mut encoded = Vec::new();
    {
        let mut encoder = codec.writer_with_level(&mut encoded, options.level());
        render_declared(batches, options, charset, header, &mut encoder)?;
        encoder.finish()?;
    }
    Ok(encoded)
}

/// Render the records in the charset the handle declares, inside the coding.
///
/// UTF-8 and US-ASCII are written as they are; any other charset goes
/// through its writer, finished before the coding writer is - the rule the
/// plain-text medium writes by, so a declared handle reads its own bytes
/// back.
///
/// # Errors
///
/// Returns a schema, value or write failure, and a scalar the charset
/// cannot spell.
pub(crate) fn render_declared(
    batches: BatchReader,
    options: &CsvOptions,
    charset: Charset,
    header: bool,
    target: &mut impl Write,
) -> Result<()> {
    if !transports(charset) {
        return render(batches, options, header, target);
    }
    let mut writer = charset.writer(target);
    render(batches, options, header, &mut writer)?;
    writer.finish()
}

/// The dialect a write spells cells in, read off the options once.
struct Dialect<'a> {
    separator: u8,
    quote: Option<u8>,
    escape: Option<u8>,
    comment: Option<u8>,
    null_values: &'a [SmolStr],
    terminator: &'a [u8],
}

impl Dialect<'_> {
    /// Whether `cell` has to be quoted to read back as it is: it holds the
    /// separator, the quote or a line break, it opens or closes with a
    /// blank, it spells an absent value, it would open a comment record, or
    /// it is empty and alone in its record, which would be a blank line.
    fn needs_quoting(&self, cell: &[u8], first: bool, alone: bool) -> bool {
        if alone && cell.is_empty() {
            return true;
        }
        if memchr::memchr3(self.separator, b'\n', b'\r', cell).is_some() {
            return true;
        }
        if let Some(quote) = self.quote
            && memchr::memchr(quote, cell).is_some()
        {
            return true;
        }
        if cell
            .first()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
            || cell.last().is_some_and(|byte| matches!(byte, b' ' | b'\t'))
        {
            return true;
        }
        if first && self.comment.is_some() && cell.first().copied() == self.comment {
            return true;
        }
        self.null_values
            .iter()
            .any(|spelling| spelling.as_bytes() == cell)
    }

    /// Write `cell`, quoted where it has to be; `false` where it had to be
    /// and the dialect quotes nothing.
    fn push_cell(&self, out: &mut Vec<u8>, cell: &[u8], first: bool, alone: bool) -> bool {
        if !self.needs_quoting(cell, first, alone) {
            out.extend_from_slice(cell);
            return true;
        }
        let Some(quote) = self.quote else {
            return false;
        };
        out.push(quote);
        for &byte in cell {
            if byte == quote {
                out.push(self.escape.unwrap_or(quote));
            } else if Some(byte) == self.escape {
                out.push(byte);
            }
            out.push(byte);
        }
        out.push(quote);
        true
    }

    /// Why the null spelling `spelling` cannot stand verbatim in a record,
    /// at its first cell where `first`: it holds the separator, the quote or
    /// a line break, or it opens the record with the comment byte.
    fn unwritable_null(&self, spelling: &str, first: bool) -> Option<SmolStr> {
        let bytes = spelling.as_bytes();
        if bytes.contains(&self.separator) {
            return Some(format_smolstr!(
                "which holds the separator {:?}",
                char::from(self.separator)
            ));
        }
        if let Some(quote) = self.quote.filter(|quote| bytes.contains(quote)) {
            return Some(format_smolstr!(
                "which holds the quote {:?}",
                char::from(quote)
            ));
        }
        if memchr::memchr2(b'\n', b'\r', bytes).is_some() {
            return Some(SmolStr::new_static("which holds a line break"));
        }
        match self.comment {
            Some(comment) if first && bytes.first() == Some(&comment) => Some(format_smolstr!(
                "which opens the record with the comment byte {:?}",
                char::from(comment)
            )),
            _ => None,
        }
    }
}

/// How one column's cells are spelled, resolved once per batch.
enum CellWriter {
    /// Plain text storage whose layout is its whole contract: the row's
    /// bytes where they lie.
    Text,
    /// A byte leaf, as base64.
    Bytes,
    /// A nested value, as the compact JSON a cast into text writes: a
    /// struct keyed by its field's names in declaration order.
    Json,
    /// Every other leaf, as its canonical text.
    Value,
}

impl CellWriter {
    fn of(column: &Serie, field: &Field) -> Self {
        if column.is_string_storage()
            && !matches!(column, Serie::FixedString(_))
            && field.dtype().layout_is_contract()
        {
            return Self::Text;
        }
        match encoded_dtype(field.dtype()).kind() {
            DataTypeKind::Bytes => Self::Bytes,
            DataTypeKind::Nested => Self::Json,
            _ => Self::Value,
        }
    }

    /// Spell row `row` of `column` into `cell`, `false` where the row is
    /// absent.
    fn render(
        &self,
        column: &Serie,
        field: &Field,
        row: usize,
        cell: &mut Vec<u8>,
    ) -> Result<bool> {
        cell.clear();
        if let Self::Text = self {
            return Ok(match column.value_bytes(row) {
                Some(bytes) => {
                    cell.extend_from_slice(bytes);
                    true
                }
                None => false,
            });
        }
        let value = column.scalar(row)?;
        if value.is_null() {
            return Ok(false);
        }
        match self {
            Self::Text => unreachable!("text rendered above"),
            Self::Bytes => {
                let bytes = value.as_bytes().ok_or_else(|| Error::InvalidRecord {
                    path: format_smolstr!("$.{}", field.name()),
                    reason: expected_got("a byte value", value.kind()),
                })?;
                crate::bytes::base64_into(bytes, cell);
            }
            Self::Json => crate::json::into_field_vec(&value, field.dtype(), cell)?,
            Self::Value => {
                let text = crate::string::str_from_value(&value).ok_or_else(|| {
                    Error::InvalidRecord {
                        path: format_smolstr!("$.{}", field.name()),
                        reason: expected_got("a value with a text spelling", value.kind()),
                    }
                })??;
                cell.extend_from_slice(text.as_bytes());
            }
        }
        Ok(true)
    }
}

/// The datatype a column's cells are, under the dictionary or run-end
/// encoding that only lays them out: a dictionary of text is text.
pub(crate) fn encoded_dtype(dtype: &DataType) -> &DataType {
    match dtype {
        DataType::Dictionary(dictionary) => encoded_dtype(dictionary.value()),
        DataType::RunEndEncoded(encoded) => encoded_dtype(encoded.values().dtype()),
        other => other,
    }
}

fn render(
    batches: BatchReader,
    options: &CsvOptions,
    header: bool,
    target: &mut impl Write,
) -> Result<()> {
    let root = field_from_arrow_schema(options.name(), batches.schema().as_ref())?;
    let rows = SerieReader::from_arrow_reader(Some(&root), batches, ArrowCastOptions::default())?;
    let dialect = Dialect {
        separator: options.separator(),
        quote: options.quote(),
        escape: options.escape(),
        comment: options.comment(),
        null_values: options.null_values(),
        terminator: options.linesep().as_bytes(),
    };
    let unquotable = |index: Option<u64>, name: &str, cell: &[u8]| Error::InvalidRecord {
        path: match index {
            Some(index) => format_smolstr!("$[{index}].{name}"),
            None => format_smolstr!("$.header.{name}"),
        },
        reason: expected_got(
            "a cell that needs no quoting - no separator, quote, line break, leading or \
             trailing blank, comment byte or null spelling - while quote is unset",
            format_args!(
                "{:?}",
                crate::text::elide_to(
                    &String::from_utf8_lossy(cell),
                    crate::text::ERROR_TEXT_LIMIT
                )
            ),
        ),
    };
    // A record of one cell spelling nothing would be a blank line, which
    // is no record at all: such a cell is written `""`, the empty text,
    // which a column that is not text reads as the null it may be.
    let alone = root.field_len() == 1;
    let lone_null = alone
        && root
            .fields()
            .first()
            .is_some_and(super::reader::reads_empty_as_null);
    let mut out = Vec::with_capacity(crate::DEFAULT_STREAM_BATCH_SIZE);
    let mut cell = Vec::new();
    if header {
        for (at, child) in root.fields().iter().enumerate() {
            if at > 0 {
                out.push(dialect.separator);
            }
            if !dialect.push_cell(&mut out, child.name().as_bytes(), at == 0, alone) {
                return Err(unquotable(None, child.name(), child.name().as_bytes()));
            }
        }
        out.extend_from_slice(dialect.terminator);
    }
    let mut index = 0_u64;
    for record in rows {
        let record = record?;
        let columns = record.children();
        let writers: Vec<CellWriter> = columns
            .iter()
            .zip(root.fields())
            .map(|(column, field)| CellWriter::of(column, field))
            .collect();
        for row in 0..record.len() {
            for (at, ((column, field), writer)) in
                columns.iter().zip(root.fields()).zip(&writers).enumerate()
            {
                if at > 0 {
                    out.push(dialect.separator);
                }
                let present = writer
                    .render(column, field, row, &mut cell)
                    .map_err(|error| at_cell(error, index, field.name()))?;
                if present {
                    if !dialect.push_cell(&mut out, &cell, at == 0, alone) {
                        return Err(unquotable(Some(index), field.name(), &cell));
                    }
                } else {
                    let spelling =
                        dialect
                            .null_values
                            .first()
                            .ok_or_else(|| Error::InvalidRecord {
                                path: format_smolstr!("$[{index}].{}", field.name()),
                                reason: SmolStr::new_static(
                                    "expected a null spelling in null_values to write a null \
                                     cell, got none",
                                ),
                            })?;
                    if let Some(holds) = dialect.unwritable_null(spelling, at == 0) {
                        return Err(Error::InvalidRecord {
                            path: SmolStr::new_static("$.null_values"),
                            reason: expected_got(
                                format_args!(
                                    "a null spelling a record holds as it stands - no separator, \
                                     quote or line break, and no comment byte opening the record \
                                     - to write the null at $[{index}].{}",
                                    field.name()
                                ),
                                format_args!("{spelling:?}, {holds}"),
                            ),
                        });
                    }
                    if alone && spelling.is_empty() {
                        match dialect.quote.filter(|_| lone_null) {
                            Some(quote) => out.extend_from_slice(&[quote, quote]),
                            None => return Err(lone_null_refused(&dialect, index, field)),
                        }
                    } else {
                        out.extend_from_slice(spelling.as_bytes());
                    }
                }
            }
            out.extend_from_slice(dialect.terminator);
            index += 1;
            if out.len() >= crate::DEFAULT_STREAM_BATCH_SIZE {
                target.write_all(&out)?;
                out.clear();
            }
        }
    }
    target.write_all(&out)?;
    Ok(())
}

/// Name a refusal raised rendering row `index` of column `name` by that
/// cell: the renderer's own reason, and the path below the column where it
/// named one.
fn at_cell(error: Error, index: u64, name: &str) -> Error {
    let reason = match error {
        Error::InvalidRecord { path, reason } => {
            let own = path.strip_prefix("$.").is_some_and(|rest| rest == name);
            if own || path == "$" {
                reason
            } else {
                format_smolstr!("{reason} at {path}")
            }
        }
        other => format_smolstr!("{}", crate::text::elide_display(&other)),
    };
    Error::InvalidRecord {
        path: format_smolstr!("$[{index}].{name}"),
        reason,
    }
}

/// The refusal for a null that is the only cell of its record where the
/// null spelling is empty and `""` does not spell it: the record would be a
/// blank line, which is no record, and nothing else reads back as absent.
fn lone_null_refused(dialect: &Dialect<'_>, index: u64, field: &Field) -> Error {
    let quoted = match dialect.quote {
        Some(_) => format_smolstr!("`\"\"` does not read as a null under {}", field.dtype()),
        None => SmolStr::new_static("quote is unset to write `\"\"`"),
    };
    Error::InvalidRecord {
        path: format_smolstr!("$[{index}].{}", field.name()),
        reason: expected_got(
            "a null spelling in null_values that is not empty, to write a null as the only cell \
             of a record",
            format_args!(
                "{:?}: an empty record is a blank line, which is no record, and {quoted}",
                dialect.null_values
            ),
        ),
    }
}
