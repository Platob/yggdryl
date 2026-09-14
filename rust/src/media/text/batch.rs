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
use crate::{DataType, Result, Scalar};

use super::line::TextLine;
use super::options::TextOptions;
use super::plan::{TextColumn, TextPlan, TextSource};

/// Turn decoded lines into one Arrow batch.
///
/// # Errors
///
/// Returns the plan's refusals, and any row-level failure the columns raise.
pub fn into_arrow_batch<I>(lines: I, options: &TextOptions) -> Result<arrow_array::RecordBatch>
where
    I: IntoIterator,
    I::Item: Into<Result<TextLine>>,
{
    let plan = options.plan()?;
    let field = plan.field(options.name.clone())?;
    let rows = lines
        .into_iter()
        .map(|line| line.into().and_then(|line| row_of(&plan, &line, options)))
        .collect::<Result<Vec<_>>>()?;
    // One batch, so the row bound is the whole input rather than a cadence.
    let mut reader = crate::arrow::rows::result_reader(
        &field,
        rows.into_iter().map(Ok),
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
    I: IntoIterator,
    I::Item: Into<Result<TextLine>>,
    I::IntoIter: Send + 'static,
{
    let plan = Arc::new(options.plan()?);
    let field = plan.field(options.name.clone())?;
    let shared = Arc::new(options.clone());
    let rows = lines.into_iter().map({
        let plan = Arc::clone(&plan);
        let shared = Arc::clone(&shared);
        move |line| line.into().and_then(|line| row_of(&plan, &line, &shared))
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
            .shared_url()
            .map_or(Scalar::Null, |url| Scalar::Url(Arc::clone(url))),
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
    super::arrow::parse_capture(text, dtype, options.timezone()).map_err(|reason| {
        // The column's name is walked to only where the error is built: the
        // path that parses is every row of every read, and it needs no name.
        let name = options.capture_names().nth(index).unwrap_or_default();
        super::arrow::row_error(line.index(), None, line.url(), name, reason)
    })
}

/// The other spellings a column is commonly written under.
///
/// Intake is where flexibility belongs: a batch that came from somewhere else
/// names its columns the way that producer named them, and refusing every
/// spelling but this crate's own would make the reverse direction useless.
/// Meaning stays exact - a matched column is read at the plan's own datatype,
/// and nothing here guesses what a value means, only what a column is called.
const ALIASES: [(&str, &[&str]); 6] = [
    ("url", &["source", "uri", "path", "file", "location"]),
    (
        "rownum",
        &["row_number", "rownumber", "line_number", "lineno", "row"],
    ),
    (
        "mtime",
        &["timestamp", "time", "ts", "written_at", "event_time"],
    ),
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
    field: crate::Field,
    capture_size: usize,
    start_rownum: i64,
}

impl Intake {
    fn resolve(
        plan: &TextPlan,
        schema: &arrow_schema::Schema,
        options: &TextOptions,
    ) -> Result<Self> {
        let field = plan.field(options.name.clone())?;
        let mut positions = Vec::with_capacity(plan.columns().len());
        for (column, child) in plan.columns().iter().zip(field.fields()) {
            let at = locate(schema, column);
            if let Some(at) = at {
                let expected = child.clone().into_arrow_ref()?;
                let actual = schema.field(at);
                if expected.data_type() != actual.data_type() {
                    return Err(crate::Error::InvalidRecord {
                        path: smol_str::format_smolstr!("$.{}", column.name),
                        reason: smol_str::format_smolstr!(
                            "expected Arrow datatype {:?}, got {:?}",
                            expected.data_type(),
                            actual.data_type()
                        ),
                    });
                }
            }
            positions.push(at);
        }
        Ok(Self {
            positions,
            field,
            capture_size: options.capture_names().len(),
            start_rownum: options.start_rownum.unwrap_or_default(),
        })
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
    let intake = Intake::resolve(&plan, batch.schema_ref(), options)?;
    let mut lines = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        lines.push(line_of(&plan, &intake, batch, row, row as u64)?);
    }
    Ok(lines)
}

/// Read streamed Arrow batches back into decoded lines.
///
/// One batch and row cursor are held; rows are decoded only when pulled.
/// The declared schema is resolved once. A changed schema or any row/source
/// error refuses once and fuses the iterator.
///
/// # Errors
///
/// Returns the plan's refusals before any batch is pulled.
pub fn from_arrow_reader(
    batches: BatchReader,
    options: &TextOptions,
) -> Result<impl std::iter::FusedIterator<Item = Result<TextLine>> + Send + 'static> {
    let plan = options.plan()?;
    let schema = batches.schema();
    let intake = Intake::resolve(&plan, &schema, options)?;
    let mut batches = batches;
    let mut pending: Option<(arrow_array::RecordBatch, usize)> = None;
    let mut ordinal = 0_u64;
    let mut done = false;
    Ok(std::iter::from_fn(move || {
        if done {
            return None;
        }
        loop {
            if let Some((batch, row)) = pending.as_mut() {
                if *row < batch.num_rows() {
                    let result = line_of(&plan, &intake, batch, *row, ordinal);
                    *row += 1;
                    ordinal += 1;
                    if result.is_err() {
                        done = true;
                        pending = None;
                    }
                    return Some(result);
                }
            }
            pending = None;
            let batch = match batches.next() {
                Some(Ok(batch)) => batch,
                Some(Err(error)) => {
                    done = true;
                    return Some(Err(crate::arrow::from_reader_error(error).into()));
                }
                None => {
                    done = true;
                    return None;
                }
            };
            if batch.schema_ref() != &schema {
                done = true;
                return Some(Err(crate::Error::InvalidRecord {
                    path: smol_str::format_smolstr!("$[{ordinal}]"),
                    reason: SmolStr::new_static(
                        "expected the reader's declared Arrow schema, got a different batch schema",
                    ),
                }));
            }
            pending = Some((batch, 0));
        }
    })
    .fuse())
}

/// One line read back out of one batch row.
fn line_of(
    plan: &TextPlan,
    intake: &Intake,
    batch: &arrow_array::RecordBatch,
    row: usize,
    ordinal: u64,
) -> Result<TextLine> {
    let mut line = TextLine::from_bytes(ordinal, super::TextBytes::new())?;
    let mut captures = vec![None; intake.capture_size];
    // The plan built the field one child per column, so the three walk together.
    for ((column, child), at) in plan
        .columns()
        .iter()
        .zip(intake.field.fields())
        .zip(&intake.positions)
    {
        let Some(at) = *at else {
            continue;
        };
        let value =
            crate::arrow::value::value_from_array(child.dtype(), batch.column(at).as_ref(), row)
                .map_err(|error| {
                    super::arrow::row_error(
                        ordinal,
                        None,
                        line.url(),
                        &column.name,
                        smol_str::format_smolstr!("{}", crate::text::elide_display(&error)),
                    )
                })?;
        let value = child.scalar(value).map_err(|error| {
            super::arrow::row_error(
                ordinal,
                None,
                line.url(),
                &column.name,
                smol_str::format_smolstr!("{}", crate::text::elide_display(&error)),
            )
        })?;
        apply(
            &mut line,
            &mut captures,
            column,
            &value,
            (ordinal, intake.start_rownum),
        )?;
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
        None => {
            let text = crate::types::string::str_from_value(value).ok_or_else(|| {
                crate::Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: SmolStr::new_static("expected a scalar with canonical text"),
                }
            })??;
            super::TextBytes::from_bytes(text.as_str()).map(Some)
        }
    }
}

/// Put one read value where its column says it belongs.
fn apply(
    line: &mut TextLine,
    captures: &mut [Option<super::TextBytes>],
    column: &TextColumn,
    value: &Scalar,
    (ordinal, start_rownum): (u64, i64),
) -> Result<()> {
    if value.is_null() {
        return Ok(());
    }
    // Located by the stream ordinal `line_of` reports, never by a restored
    // row number an earlier column of this same row may already have set.
    let refused = |reason| super::arrow::row_error(ordinal, None, line.url(), &column.name, reason);
    let text = |value| {
        read_bytes(value).map_err(|error| {
            refused(smol_str::format_smolstr!(
                "{}",
                crate::text::elide_display(&error)
            ))
        })
    };
    match &column.source {
        TextSource::Url => {
            if let Scalar::Url(url) = value {
                line.set_url(Some(Arc::clone(url)));
            }
        }
        TextSource::Rownum => {
            let number = value
                .as_i128()
                .and_then(|number| u64::try_from(number - i128::from(start_rownum)).ok())
                .ok_or_else(|| {
                    refused(SmolStr::new_static(
                        "expected a row number at or after start_rownum",
                    ))
                })?;
            line.set_index(number);
        }
        TextSource::Timestamp => {
            if let Some(count) = value.temporal_count() {
                line.set_timestamp(Some(i128::from(count)));
            }
        }
        TextSource::BodyType => {
            if let Some(text) = value.as_str() {
                let bodytype = crate::MimeType::from_str(text).map_err(|error| {
                    refused(smol_str::format_smolstr!(
                        "{}",
                        crate::text::elide_display(&error)
                    ))
                })?;
                line.set_bodytype(Some(bodytype));
            }
        }
        TextSource::Body => {
            if let Some(held) = text(value)? {
                line.set_body(held)?;
            }
        }
        TextSource::DroppedByteSize => {
            let size = value
                .as_i128()
                .and_then(|size| u64::try_from(size).ok())
                .ok_or_else(|| {
                    refused(SmolStr::new_static(
                        "expected a nonnegative dropped byte count fitting u64",
                    ))
                })?;
            line.set_dropped_byte_size(Some(size));
        }
        TextSource::Capture(index) => captures[*index] = text(value)?,
        TextSource::Entry(path) => {
            if let Some(held) = text(value)? {
                line.set_entry_by_path(path, held)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use arrow_array::{ArrayRef, Int64Array, RecordBatch, RecordBatchIterator, StringArray};
    use arrow_schema::{DataType as ArrowType, Field as ArrowField, Schema};

    use super::*;
    use crate::media::text::TextBytes;

    fn line(index: u64, body: &str) -> TextLine {
        TextLine::from_bytes(index, TextBytes::from_bytes(body).expect("bytes")).expect("line")
    }

    fn batch(fields: Vec<ArrowField>, columns: Vec<ArrayRef>) -> RecordBatch {
        RecordBatch::try_new(Arc::new(Schema::new(fields)), columns).expect("batch")
    }

    fn bodies(values: Vec<Option<&str>>) -> RecordBatch {
        batch(
            vec![ArrowField::new("body", ArrowType::Utf8, true)],
            vec![Arc::new(StringArray::from(values))],
        )
    }

    fn reader(
        schema: arrow_schema::SchemaRef,
        steps: Vec<Option<std::result::Result<RecordBatch, arrow_schema::ArrowError>>>,
        pulls: &Arc<AtomicUsize>,
    ) -> BatchReader {
        let pulls = Arc::clone(pulls);
        let mut steps = steps.into_iter();
        Box::new(RecordBatchIterator::new(
            std::iter::from_fn(move || {
                pulls.fetch_add(1, Ordering::SeqCst);
                steps.next().flatten()
            }),
            schema,
        ))
    }

    #[test]
    fn forward_doors_accept_owned_and_fallible_lines_and_preserve_errors() {
        let options = TextOptions::new();
        let owned = into_arrow_batch([line(0, "one")], &options).expect("owned");
        let fallible = into_arrow_batch([Ok(line(0, "one"))], &options).expect("fallible");
        assert_eq!(owned, fallible);
        let error = crate::Error::InvalidRecord {
            path: "$.source".into(),
            reason: "original failure".into(),
        };
        let error =
            into_arrow_batch([Err::<TextLine, _>(error)], &options).expect_err("source error");
        assert!(matches!(error, crate::Error::InvalidRecord { path, reason }
            if path == "$.source" && reason == "original failure"));
        let source = vec![Ok(line(0, "one")), Ok(line(1, "two"))];
        let read = into_arrow_reader(source, &options).expect("reader");
        assert_eq!(
            read.map(|held| held.expect("batch").num_rows())
                .sum::<usize>(),
            2
        );
        let read = into_arrow_reader([line(0, "one")], &options).expect("owned reader");
        assert_eq!(
            read.map(|held| held.expect("batch").num_rows())
                .sum::<usize>(),
            1
        );
    }

    #[test]
    fn reverse_is_lazy_one_row_at_a_time_and_fuses_a_bad_later_row() {
        let source = bodies(vec![Some("first"), None, Some("never")]);
        let pulls = Arc::new(AtomicUsize::new(0));
        let source = reader(source.schema(), vec![Some(Ok(source))], &pulls);
        let mut read = from_arrow_reader(source, &TextOptions::new()).expect("plan");
        assert_eq!(pulls.load(Ordering::SeqCst), 0);
        assert_eq!(read.next().expect("first").expect("valid").body(), "first");
        assert_eq!(pulls.load(Ordering::SeqCst), 1);
        let error = read.next().expect("error").expect_err("null body");
        assert!(error.to_string().contains("$[1].body"), "{error}");
        assert!(read.next().is_none());
        assert!(read.next().is_none());
        assert_eq!(pulls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn source_schema_drift_and_errors_fuse_without_pulling_later_batches() {
        let good = bodies(vec![Some("first")]);
        let drift = batch(
            vec![ArrowField::new("payload", ArrowType::Utf8, false)],
            vec![Arc::new(StringArray::from(vec!["other"]))],
        );
        for problem in [
            Ok(drift),
            Err(arrow_schema::ArrowError::ParseError(
                "source sentinel".into(),
            )),
        ] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let source = reader(
                good.schema(),
                vec![
                    Some(Ok(good.clone())),
                    Some(problem),
                    Some(Ok(good.clone())),
                ],
                &pulls,
            );
            let mut read = from_arrow_reader(source, &TextOptions::new()).expect("plan");
            assert!(read.next().expect("prefix").is_ok());
            assert!(read.next().expect("failure").is_err());
            assert!(read.next().is_none());
            assert!(read.next().is_none());
            assert_eq!(pulls.load(Ordering::SeqCst), 2);
        }
    }

    #[test]
    fn eof_fuses_even_a_source_that_would_resume() {
        let good = bodies(vec![Some("never")]);
        let pulls = Arc::new(AtomicUsize::new(0));
        let source = reader(good.schema(), vec![None, Some(Ok(good))], &pulls);
        let mut read = from_arrow_reader(source, &TextOptions::new()).expect("plan");
        assert!(read.next().is_none());
        assert!(read.next().is_none());
        assert_eq!(pulls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn row_numbers_remove_the_configured_offset_and_missing_columns_use_stream_ordinals() {
        for start in [-9, 0, 12] {
            let mut options = TextOptions::new();
            options.start_rownum = Some(start);
            let original = [line(2, "one"), line(7, "two")];
            let batch = into_arrow_batch(original, &options).expect("batch");
            let rows = from_arrow_batch(&batch, &options).expect("reverse");
            assert_eq!(rows.iter().map(TextLine::index).collect::<Vec<_>>(), [2, 7]);
        }
        let first = bodies(vec![Some("one"), Some("two")]);
        let last = bodies(vec![Some("three")]);
        let empty = first.slice(0, 0);
        let pulls = Arc::new(AtomicUsize::new(0));
        let source = reader(
            first.schema(),
            vec![Some(Ok(first)), Some(Ok(empty)), Some(Ok(last))],
            &pulls,
        );
        let rows = from_arrow_reader(source, &TextOptions::new())
            .expect("plan")
            .collect::<Result<Vec<_>>>()
            .expect("rows");
        assert_eq!(
            rows.iter().map(TextLine::index).collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }

    #[test]
    fn malformed_present_numbers_types_and_media_are_refused() {
        let mut options = TextOptions::new();
        options.start_rownum = Some(10);
        let source = batch(
            vec![ArrowField::new("rownum", ArrowType::Int64, false)],
            vec![Arc::new(Int64Array::from(vec![9]))],
        );
        let error = from_arrow_batch(&source, &options).expect_err("offset underflow");
        assert!(error.to_string().contains("$[0].rownum"));
        let source = batch(
            vec![ArrowField::new("rownum", ArrowType::Utf8, false)],
            vec![Arc::new(StringArray::from(vec!["wrong"]))],
        );
        assert!(
            from_arrow_batch(&source, &options)
                .expect_err("wrong layout")
                .to_string()
                .contains("$.rownum")
        );
        options.parse_mimetype = true;
        let source = batch(
            vec![ArrowField::new("mimetype", ArrowType::Utf8, false)],
            vec![Arc::new(StringArray::from(vec!["not a mime type"]))],
        );
        assert!(
            from_arrow_batch(&source, &options)
                .expect_err("bad mime")
                .to_string()
                .contains("$[0].mimetype")
        );
        // A row number restored by an earlier column never relocates a later refusal.
        let source = batch(
            vec![
                ArrowField::new("rownum", ArrowType::Int64, false),
                ArrowField::new("mimetype", ArrowType::Utf8, false),
            ],
            vec![
                Arc::new(Int64Array::from(vec![15])),
                Arc::new(StringArray::from(vec!["not a mime type"])),
            ],
        );
        let error = from_arrow_batch(&source, &options).expect_err("bad mime after rownum");
        assert!(error.to_string().contains("$[0].mimetype"), "{error}");
    }

    #[test]
    fn capture_holes_keep_indices_and_typed_values_have_canonical_text() {
        let options = TextOptions::new()
            .try_with_rowheader(r"^(?<first>[A-Z]+) (?<second>\d+) (?<third>[A-Z]+)")
            .expect("captures");
        let source = batch(
            vec![ArrowField::new("second", ArrowType::Int64, true)],
            vec![Arc::new(Int64Array::from(vec![Some(7), None]))],
        );
        let rows = from_arrow_batch(&source, &options).expect("captures");
        assert_eq!(rows[0].captures().len(), 3);
        assert_eq!(
            (rows[0].capture(0), rows[0].capture(1), rows[0].capture(2)),
            (None, Some("7"), None)
        );
        assert!(rows[1].captures().iter().all(Option::is_none));
        let source = line(0, "body")
            .with_captures(vec![
                Some(TextBytes::from_bytes("AA").expect("text")),
                Some(TextBytes::from_bytes("0007").expect("text")),
                Some(TextBytes::from_bytes("ZZ").expect("text")),
            ])
            .expect("captures");
        let batch = into_arrow_batch([source], &options).expect("typed batch");
        let rows = from_arrow_batch(&batch, &options).expect("rows");
        assert_eq!(rows[0].capture(1), Some("7"));
    }
}
