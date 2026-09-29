//! Rendering record batches as CSV through the handle's coding and charset.
//!
//! Each batch lands as one record column and is walked row by row, each
//! column's cell rendered by the reading its leaf resolved once per batch: a
//! text leaf lends its bytes where they lie, every other leaf spells its
//! canonical text, a byte leaf as base64 and a nested value as compact JSON.
//! A cell that would not read back as it was written is quoted; nothing else
//! is.

use std::io::Write;

use base64::Engine as _;
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
    /// blank, it spells an absent value, or it would open a comment record.
    fn needs_quoting(&self, cell: &[u8], first: bool) -> bool {
        if memchr::memchr3(self.separator, b'\n', b'\r', cell).is_some() {
            return true;
        }
        if let Some(quote) = self.quote {
            if memchr::memchr(quote, cell).is_some() {
                return true;
            }
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
    fn push_cell(&self, out: &mut Vec<u8>, cell: &[u8], first: bool) -> bool {
        if !self.needs_quoting(cell, first) {
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
}

/// How one column's cells are spelled, resolved once per batch.
enum CellWriter {
    /// Plain text storage whose layout is its whole contract: the row's
    /// bytes where they lie.
    Text,
    /// A byte leaf, as base64.
    Bytes,
    /// A nested value, as compact JSON keyed by the field's names.
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
                let len =
                    base64::encoded_len(bytes.len(), true).ok_or_else(|| Error::InvalidRecord {
                        path: format_smolstr!("$.{}", field.name()),
                        reason: SmolStr::new_static("a byte value too long to spell as base64"),
                    })?;
                cell.resize(len, 0);
                let written = base64::engine::general_purpose::STANDARD
                    .encode_slice(bytes, cell)
                    .map_err(|error| Error::InvalidRecord {
                        path: format_smolstr!("$.{}", field.name()),
                        reason: format_smolstr!("{error}"),
                    })?;
                cell.truncate(written);
            }
            Self::Json => {
                let natural = crate::text::typed::into_natural(value, field)?;
                cell.extend_from_slice(crate::into_json_scalar(&natural)?.as_bytes());
            }
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
    let mut out = Vec::with_capacity(crate::DEFAULT_STREAM_BATCH_SIZE);
    let mut cell = Vec::new();
    if header {
        for (at, child) in root.fields().iter().enumerate() {
            if at > 0 {
                out.push(dialect.separator);
            }
            if !dialect.push_cell(&mut out, child.name().as_bytes(), at == 0) {
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
                if writer.render(column, field, row, &mut cell)? {
                    if !dialect.push_cell(&mut out, &cell, at == 0) {
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
                    out.extend_from_slice(spelling.as_bytes());
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
