//! Arrow batches built column-first from decoded lines.
//!
//! The plan settles every column, its name, its datatype and what fills it
//! before a byte is read, so this walks each line once and puts each field
//! where the plan already says it goes. Nothing here looks a column up by name,
//! re-derives a datatype, or builds a sorted map only to sort it back.

use std::sync::Arc;

use smol_str::SmolStr;

use crate::arrow::BatchReader;
use crate::media::IORecordOptions as _;
use crate::types::Integer;
use crate::{DataType, Result, Scalar};

use super::line::TextLine;
use super::options::TextOptions;
use super::plan::{TextColumn, TextPlan, TextSource};

/// Turn decoded lines into one Arrow batch.
///
/// # Errors
///
/// Returns the plan's refusals, and any row-level failure the columns raise.
pub fn into_arrow_batch(
    lines: impl IntoIterator<Item = TextLine>,
    options: &TextOptions,
) -> Result<arrow_array::RecordBatch> {
    let plan = options.plan()?;
    let field = plan.field(options.name.clone())?;
    let rows = lines
        .into_iter()
        .map(|line| row_of(&plan, &line, options))
        .collect::<Result<Vec<_>>>()?;
    // One batch, so the row bound is the whole input rather than a cadence.
    let mut reader = crate::arrow::rows::result_reader(
        &field,
        rows.into_iter().map(Ok).collect::<Vec<_>>(),
        Some(usize::MAX),
        None,
        None,
        None,
    )?;
    let schema = reader.schema();
    match reader.next() {
        Some(batch) => Ok(batch.map_err(crate::arrow::from_reader_error)?),
        // No lines is an empty batch under the same schema, never an error:
        // a read of an empty object answers its columns and no rows.
        None => Ok(arrow_array::RecordBatch::new_empty(schema)),
    }
}

/// Turn decoded lines into streamed Arrow batches.
///
/// Lines are pulled one batch at a time and never all held at once.
///
/// # Errors
///
/// Returns the plan's refusals before any line is pulled.
pub fn into_arrow_reader<I>(lines: I, options: &TextOptions) -> Result<BatchReader>
where
    I: IntoIterator<Item = Result<TextLine>>,
    I::IntoIter: Send + 'static,
{
    let plan = Arc::new(options.plan()?);
    let field = plan.field(options.name.clone())?;
    let shared = Arc::new(options.clone());
    let rows = lines.into_iter().map({
        let plan = Arc::clone(&plan);
        let shared = Arc::clone(&shared);
        move |line| line.and_then(|line| row_of(&plan, &line, &shared))
    });
    // The outer media pipeline applies a total row limit after projection and
    // filtering. Pull one framed row at a time for that limit so satisfying it
    // never parses a later logical record. A byte limit retains the requested
    // batch shape because its established accounting includes Arrow storage.
    let batch_row_size = if options.max_row_size().is_some() && options.max_byte_size().is_none() {
        Some(1)
    } else {
        options.batch_row_size()
    };
    Ok(crate::arrow::rows::result_reader(
        &field,
        rows,
        batch_row_size,
        options.batch_byte_size(),
        None,
        None,
    )?)
}

/// One row, in the plan's column order.
///
/// An ordered sequence rather than a name-keyed record: the plan fixed the
/// order before the read, so building a sorted map and canonicalizing it back
/// into this order would rediscover per row what is already known.
pub(crate) fn row_of(plan: &TextPlan, line: &TextLine, options: &TextOptions) -> Result<Scalar> {
    let mut values = Vec::with_capacity(plan.columns().len());
    for column in plan.columns() {
        values.push(value_of(&column.source, &column.dtype, line, options)?);
    }
    Ok(Scalar::from_sequence(values))
}

/// The value one column takes from one line.
fn value_of(
    source: &TextSource,
    dtype: &DataType,
    line: &TextLine,
    options: &TextOptions,
) -> Result<Scalar> {
    Ok(match source {
        TextSource::Url => line
            .url()
            .map_or(Scalar::Null, |url| Scalar::Url(Arc::new(url.clone()))),
        TextSource::Rownum => super::arrow::physical_rownum(options.start_rownum, line.index())?
            .map_or(Scalar::Null, Scalar::from),
        // The line counts in 128 bits and the column holds 64, so this is
        // where the narrowing happens - once, by name, refusing rather than
        // wrapping a count no nanosecond column can hold.
        TextSource::Timestamp => match line.timestamp() {
            Some(count) => {
                let count = i64::try_from(count).map_err(|_| {
                    super::arrow::row_error(
                        line.index(),
                        None,
                        line.url(),
                        super::options::MTIME_COLUMN,
                        smol_str::format_smolstr!(
                            "expected a nanosecond count a 64-bit column can hold, got {count}"
                        ),
                    )
                })?;
                Scalar::datetime64(count, crate::TimeUnit::Nanosecond, crate::Timezone::UTC)
                    .unwrap_or(Scalar::Null)
            }
            None => Scalar::Null,
        },
        TextSource::Direction => line.direction().map_or(Scalar::Null, |direction| {
            DataType::MsgDirection
                .scalar(Scalar::from(direction))
                .unwrap_or(Scalar::Null)
        }),
        TextSource::BodyType => Scalar::from(
            line.bodytype()
                .unwrap_or(&crate::MimeType::OCTET_STREAM)
                .as_str(),
        ),
        TextSource::Body => Scalar::from(line.body()),
        TextSource::DroppedByteSize => line.dropped_byte_size().map_or(Scalar::Null, Scalar::from),
        TextSource::Capture(index) => capture_value(line, *index, dtype, options)?,
        // A path naming something this line did not carry is a null, which is
        // the whole reason a caller lifts a path out of a shape that varies.
        TextSource::Entry(path) => line
            .get_entry_by_path(path)
            .map_or(Scalar::Null, |entry| Scalar::from(entry.value().as_ref())),
    })
}

/// One row-header capture, read at the datatype the plan settled for it.
fn capture_value(
    line: &TextLine,
    index: usize,
    dtype: &DataType,
    options: &TextOptions,
) -> Result<Scalar> {
    let Some(text) = line.capture(index) else {
        return Ok(Scalar::Null);
    };
    let name = options.capture_names().nth(index).unwrap_or_default();
    super::arrow::parse_capture(text, dtype, options.timezone())
        .map_err(|reason| super::arrow::row_error(line.index(), None, line.url(), name, reason))
}

/// The other spellings a column is commonly written under.
///
/// Intake is where flexibility belongs: a batch that came from somewhere else
/// names its columns the way that producer named them, and refusing every
/// spelling but this crate's own would make the reverse direction useless.
/// Meaning stays exact - a matched column is read at the plan's own datatype,
/// and nothing here guesses what a value means, only what a column is called.
const ALIASES: [(&str, &[&str]); 7] = [
    ("url", &["source", "uri", "path", "file", "location"]),
    (
        "rownum",
        &["row_number", "rownumber", "line_number", "lineno", "row"],
    ),
    (
        "mtime",
        &["timestamp", "time", "ts", "written_at", "event_time"],
    ),
    ("direction", &["dir", "way"]),
    (
        "mimetype",
        &["bodytype", "content_type", "contenttype", "media_type"],
    ),
    (
        "body",
        &["payload", "message", "line", "text", "content", "raw"],
    ),
    (
        "dropped_byte_size",
        &["dropped", "dropped_bytes", "truncated_bytes"],
    ),
];

/// The name a fixed column carries before any rename.
const fn default_name_of(source: &TextSource) -> Option<&'static str> {
    Some(match source {
        TextSource::Url => "url",
        TextSource::Rownum => "rownum",
        TextSource::Timestamp => "mtime",
        TextSource::Direction => "direction",
        TextSource::BodyType => "mimetype",
        TextSource::Body => "body",
        TextSource::DroppedByteSize => "dropped_byte_size",
        TextSource::Capture(_) | TextSource::Entry(_) => return None,
    })
}

/// Where one planned column sits in an incoming schema.
///
/// Exact name first, because a caller who spelled it exactly meant it; then
/// ignoring case; then the spellings that column is commonly written under.
fn locate(schema: &arrow_schema::Schema, column: &TextColumn) -> Option<usize> {
    let wanted = column.name.as_str();
    if let Some(at) = schema
        .fields()
        .iter()
        .position(|held| held.name() == wanted)
    {
        return Some(at);
    }
    if let Some(at) = schema
        .fields()
        .iter()
        .position(|held| held.name().eq_ignore_ascii_case(wanted))
    {
        return Some(at);
    }
    // Keyed by the default name rather than the emitted one, so a renamed
    // column still finds the spellings it is known by.
    let default = default_name_of(&column.source)?;
    let aliases = ALIASES
        .iter()
        .find(|(name, _)| *name == default)
        .map(|(_, aliases)| *aliases)?;
    schema.fields().iter().position(|held| {
        held.name().eq_ignore_ascii_case(default)
            || aliases
                .iter()
                .any(|alias| held.name().eq_ignore_ascii_case(alias))
    })
}

/// One column-to-position mapping, resolved once against an incoming schema.
///
/// The per-row path then indexes an array rather than looking a name up, which
/// is the same boundary rule the read direction follows.
struct Intake {
    positions: Vec<Option<usize>>,
}

impl Intake {
    fn resolve(plan: &TextPlan, schema: &arrow_schema::Schema) -> Self {
        Self {
            positions: plan
                .columns()
                .iter()
                .map(|column| locate(schema, column))
                .collect(),
        }
    }
}

/// Read one Arrow batch back into decoded lines.
///
/// The reverse of [`into_arrow_batch`], so a caller can round-trip a text read
/// through Arrow and get its lines back. A column the batch does not carry
/// leaves that field at its default rather than failing, because absence is not
/// a failure on the read path anywhere else here.
///
/// # Errors
///
/// Returns the plan's refusals, and any value that will not read at the
/// datatype its column declares.
pub fn from_arrow_batch(
    batch: &arrow_array::RecordBatch,
    options: &TextOptions,
) -> Result<Vec<TextLine>> {
    let plan = options.plan()?;
    let intake = Intake::resolve(&plan, batch.schema_ref());
    let field = plan.field(options.name.clone())?;
    let mut lines = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        lines.push(line_of(&plan, &intake, batch, row, &field)?);
    }
    Ok(lines)
}

/// Read streamed Arrow batches back into decoded lines.
///
/// Batches are pulled one at a time; nothing holds the whole stream.
///
/// # Errors
///
/// Returns the plan's refusals before any batch is pulled.
pub fn from_arrow_reader(
    batches: BatchReader,
    options: &TextOptions,
) -> Result<impl Iterator<Item = Result<TextLine>> + Send + 'static> {
    let plan = options.plan()?;
    let field = plan.field(options.name.clone())?;
    let mut batches = batches;
    let mut pending: std::vec::IntoIter<Result<TextLine>> = Vec::new().into_iter();
    Ok(std::iter::from_fn(move || {
        loop {
            if let Some(line) = pending.next() {
                return Some(line);
            }
            let batch = match batches.next()? {
                Ok(batch) => batch,
                Err(error) => return Some(Err(crate::arrow::from_reader_error(error).into())),
            };
            let intake = Intake::resolve(&plan, batch.schema_ref());
            let read: Vec<Result<TextLine>> = (0..batch.num_rows())
                .map(|row| line_of(&plan, &intake, &batch, row, &field))
                .collect();
            pending = read.into_iter();
        }
    }))
}

/// One line read back out of one batch row.
fn line_of(
    plan: &TextPlan,
    intake: &Intake,
    batch: &arrow_array::RecordBatch,
    row: usize,
    field: &crate::Field,
) -> Result<TextLine> {
    let mut line = TextLine::from_bytes(row as u64, super::TextBytes::new())?;
    let mut captures: Vec<Option<super::TextBytes>> = Vec::new();
    for (index, column) in plan.columns().iter().enumerate() {
        let Some(at) = intake.positions[index] else {
            if matches!(column.source, TextSource::Capture(_)) {
                captures.push(None);
            }
            continue;
        };
        let child = field
            .get_field(index)
            .ok_or_else(|| crate::Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: SmolStr::new_static("the plan and its field disagree on column count"),
            })?;
        let sliced = batch.column(at).slice(row, 1);
        let value = crate::arrow::scalar_value(child, sliced.as_ref())?;
        let value = value.get(0).cloned().unwrap_or(value);
        apply(&mut line, &mut captures, &column.source, &value)?;
    }
    line.set_captures(captures)?;
    Ok(line)
}

/// The bytes one read value carries, whichever way it spells them.
fn read_bytes(value: &Scalar) -> Result<Option<super::TextBytes>> {
    if value.is_null() {
        return Ok(None);
    }
    if let Some(held) = value.as_bytes() {
        return super::TextBytes::from_bytes(held).map(Some);
    }
    match value.as_str() {
        Some(text) => super::TextBytes::from_bytes(text).map(Some),
        None => Ok(None),
    }
}

/// Put one read value where its column says it belongs.
fn apply(
    line: &mut TextLine,
    captures: &mut Vec<Option<super::TextBytes>>,
    source: &TextSource,
    value: &Scalar,
) -> Result<()> {
    match source {
        TextSource::Url => {
            if let Scalar::Url(url) = value {
                line.set_url(Some(Arc::clone(url)));
            }
        }
        TextSource::Rownum => {
            if let Some(number) = value.as_integer().and_then(Integer::as_i128) {
                line.set_index(u64::try_from(number).unwrap_or_default());
            }
        }
        TextSource::Timestamp => {
            if let Some(held) = value.as_temporal() {
                line.set_timestamp(Some(i128::from(held.count())));
            }
        }
        TextSource::Direction => {
            if let Some(text) = value.as_str() {
                // The vocabulary is closed, so what a line carries is one of
                // its own constants rather than the caller's bytes.
                const KNOWN: [&str; 2] = [
                    crate::types::MsgDirection::SENT,
                    crate::types::MsgDirection::RECV,
                ];
                line.set_direction(
                    KNOWN
                        .into_iter()
                        .find(|known| known.eq_ignore_ascii_case(text)),
                );
            }
        }
        TextSource::BodyType => {
            if let Some(text) = value.as_str() {
                // A spelling the catalog does not know is not a failure here:
                // it is a batch someone else classified, and the line keeps
                // whatever it can make of it.
                line.set_bodytype(crate::MimeType::from_str(text).ok());
            }
        }
        TextSource::Body => {
            if let Some(held) = read_bytes(value)? {
                line.set_body(held)?;
            }
        }
        TextSource::DroppedByteSize => {
            if let Some(size) = value.as_integer().and_then(Integer::as_i128) {
                line.set_dropped_byte_size(u64::try_from(size).ok());
            }
        }
        TextSource::Capture(_) => captures.push(read_bytes(value)?),
        TextSource::Entry(path) => {
            if let Some(held) = read_bytes(value)? {
                line.set_entry_by_path(path, held)?;
            }
        }
    }
    Ok(())
}
