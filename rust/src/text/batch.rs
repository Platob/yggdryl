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
use crate::{DataType, Result, Scalar, Url};

use super::line::TextLine;
use super::options::TextOptions;
use super::plan::{TextColumn, TextPlan, TextSource};

/// Turn decoded lines into one Arrow batch.
///
/// Every line handed in becomes a row: the options are read for the column
/// plan the rows take, never for the `where`, the `select` or the row bounds,
/// which the record surface applies once over the batch this builds. A caller
/// holding lines already past that seam shapes them with
/// [`IORecordOptions::apply_arrow_batch`](crate::media::IORecordOptions::apply_arrow_batch).
///
/// # Errors
///
/// Returns the plan's refusals, and any row-level failure the columns raise.
pub fn into_arrow_batch<I>(lines: I, options: &TextOptions) -> Result<arrow_array::RecordBatch>
where
    I: IntoIterator,
    I::Item: Into<Result<TextLine>>,
{
    let plan = options.line_plan()?;
    let field = plan.field(SmolStr::new(options.name()))?;
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
/// Lines are pulled one batch at a time and never all held at once, and every
/// one of them becomes a row: as in [`into_arrow_batch`], the `where`, the
/// `select` and the row bounds belong to the record surface that reads a
/// handle, and only the batch shape is read from the options here.
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
    let plan = Arc::new(options.line_plan()?);
    let field = plan.field(SmolStr::new(options.name()))?;
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
        // The line's own reading of the fact, which refuses by name a
        // capture the fact's type cannot read.
        TextSource::Event(column) => line.event_fact(*column)?.unwrap_or(Scalar::Null),
        TextSource::BodyType => Scalar::from(line.bodytype().as_str()),
        TextSource::Body => Scalar::from(line.body()),
        TextSource::DroppedByteSize => line.dropped_byte_size().map_or(Scalar::Null, Scalar::from),
        TextSource::Capture(index) => capture_value(line, *index, dtype, options)?,
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
        super::arrow::row_error(line.index(), None, line.sourceurl(), name, reason)
    })
}

/// The other spellings a column is commonly written under.
///
/// Intake is where flexibility belongs: a batch that came from somewhere else
/// names its columns the way that producer named them, and refusing every
/// spelling but this crate's own would make the reverse direction useless.
/// Meaning stays exact - a matched column is read at the plan's own datatype,
/// and nothing here guesses what a value means, only what a column is called.
const ALIASES: [(&str, &[&str]); 5] = [
    // The object a line came from and the place it holds are event facts
    // now, so the spellings a producer wrote them under resolve onto the
    // columns that state them rather than onto columns of their own.
    (
        "crosscode",
        &[
            "sourceurl",
            "url",
            "source",
            "uri",
            "path",
            "file",
            "location",
        ],
    ),
    (
        "seqnum",
        &[
            "rownum",
            "row_number",
            "rownumber",
            "line_number",
            "lineno",
            "row",
        ],
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
        TextSource::Event(column) => column.name(),
        TextSource::BodyType => "mimetype",
        TextSource::Body => "body",
        TextSource::DroppedByteSize => "dropped_byte_size",
        TextSource::Capture(_) => return None,
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
    /// Where the `body` column sits in the plan.
    ///
    /// Read before any other cell, because a line is the line it holds and
    /// [`TextLine`] has no door that takes an empty one: the body is what
    /// the line is made from, and the rest is stated on it.
    body_at: usize,
    capture_size: usize,
    /// Whether the batch carries a capture column at all: a line read back
    /// states its captures only where a column stated them, and matches
    /// its own header otherwise.
    captures_located: bool,
    start_rownum: i64,
    /// The options every line read back is built under, shared once.
    options: Arc<TextOptions>,
}

impl Intake {
    fn resolve(
        plan: &TextPlan,
        schema: &arrow_schema::Schema,
        options: &TextOptions,
    ) -> Result<Self> {
        let field = plan.field(SmolStr::new(options.name()))?;
        let mut positions = Vec::with_capacity(plan.columns().len());
        for (column, child) in plan.columns().iter().zip(field.fields()) {
            let at = locate(schema, column);
            if let Some(at) = at {
                let expected = child.clone().into_arrow_field_ref()?;
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
        let captures_located =
            plan.columns().iter().zip(&positions).any(|(column, at)| {
                at.is_some() && matches!(column.source, TextSource::Capture(_))
            });
        // The plan always states exactly one body column, so this is a
        // position and never a search.
        let body_at = plan
            .columns()
            .iter()
            .position(|column| matches!(column.source, TextSource::Body))
            .ok_or_else(|| crate::Error::InvalidRecord {
                path: SmolStr::new_static("$.body"),
                reason: SmolStr::new_static("expected the plan to state one body column"),
            })?;
        Ok(Self {
            positions,
            field,
            body_at,
            capture_size: options.capture_names().len(),
            captures_located,
            start_rownum: options.start_rownum.unwrap_or_default(),
            options: Arc::new(options.clone()),
        })
    }
}

/// Read one Arrow batch back into decoded lines.
///
/// The reverse of [`into_arrow_batch`], so a caller can round-trip a text read
/// through Arrow and get its lines back. A column the batch does not carry
/// leaves that field at its default rather than failing, because absence is not
/// a failure on the read path anywhere else here - the `body` alone excepted,
/// because a line is the line it holds and a row stating none is no line.
///
/// # Errors
///
/// Returns the plan's refusals, any value that will not read at the datatype
/// its column declares, a null under a column the plan declares non-nullable,
/// and a row whose body is absent, null or empty.
pub fn from_arrow_batch(
    batch: &arrow_array::RecordBatch,
    options: &TextOptions,
) -> Result<Vec<TextLine>> {
    let plan = options.line_plan()?;
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
    let plan = options.line_plan()?;
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
    // The row's body is already the line past its header, so it is taken as
    // it stands; the header's captures are this row's own columns.
    let mut line = TextLine::from_row(
        ordinal,
        body_of(plan, intake, batch, row, ordinal)?,
        Arc::clone(&intake.options),
    )?;
    let mut captures = vec![None; intake.capture_size];
    // The plan built the field one child per column, so the three walk together.
    for (index, ((column, child), at)) in plan
        .columns()
        .iter()
        .zip(intake.field.fields())
        .zip(&intake.positions)
        .enumerate()
    {
        // The body made the line above; restating it here would drop every
        // reading the columns after it state.
        if index == intake.body_at {
            continue;
        }
        let Some(at) = *at else {
            continue;
        };
        let value = cell_of(batch, at, row, ordinal, line.sourceurl(), column, child)?;
        apply(
            &mut line,
            &mut captures,
            column,
            &value,
            (ordinal, intake.start_rownum),
        )?;
    }
    if intake.captures_located {
        line.set_captures(captures)?;
    }
    Ok(line)
}

/// The body one batch row states, refused where it states none.
///
/// The one column whose absence is a failure. Everywhere else here a column
/// the batch does not carry leaves its fact at the default, because absence
/// is not a failure on the read path - but the body is what a line *is*, and
/// a row with no body column, a null cell or an empty one is no line. The
/// null is refused by the plan's own non-nullable column, on the way through
/// [`crate::Field::scalar`]; the other two are refused here, under the same
/// name.
fn body_of(
    plan: &TextPlan,
    intake: &Intake,
    batch: &arrow_array::RecordBatch,
    row: usize,
    ordinal: u64,
) -> Result<super::TextBytes> {
    let column = &plan.columns()[intake.body_at];
    let child = &intake.field.fields()[intake.body_at];
    // No line yet, so no URL to locate the refusal by: the row's own
    // `sourceurl` is one of the columns read after this one.
    let refused = |reason| super::arrow::row_error(ordinal, None, None, &column.name, reason);
    let Some(at) = intake.positions[intake.body_at] else {
        return Err(refused(SmolStr::new_static(
            "expected a body column, got a batch that carries none",
        )));
    };
    let value = cell_of(batch, at, row, ordinal, None, column, child)?;
    // A cell that states nothing at all is no row; an empty one is the line
    // a row header consumed whole, whose captures are what it states, and a
    // read emits those - so a read back takes them.
    let body = read_bytes(&value)
        .map_err(|error| {
            refused(smol_str::format_smolstr!(
                "{}",
                crate::text::elide_display(&error)
            ))
        })?
        .ok_or_else(|| refused(SmolStr::new_static("expected a line body, got a null one")))?;
    Ok(body)
}

/// One cell, read at the datatype and under the nullability its column
/// declares.
///
/// The plan's `nullable` is the check: a null under a column that cannot
/// hold one is refused here, by the column's name, rather than reaching a
/// builder that would answer for the whole batch.
fn cell_of(
    batch: &arrow_array::RecordBatch,
    at: usize,
    row: usize,
    ordinal: u64,
    url: Option<&crate::Url>,
    column: &TextColumn,
    child: &crate::Field,
) -> Result<Scalar> {
    // Both doors below refuse in their own error type, so the elision the
    // message is built through is the only thing they share.
    let refused = |error: &dyn std::fmt::Display| {
        super::arrow::row_error(
            ordinal,
            None,
            url,
            &column.name,
            smol_str::format_smolstr!("{}", crate::text::elide_display(&error)),
        )
    };
    let value =
        crate::arrow::value::value_from_array(child.dtype(), batch.column(at).as_ref(), row)
            .map_err(|error| refused(&error))?;
    child.scalar(value).map_err(|error| refused(&error))
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
            let text = crate::string::str_from_value(value).ok_or_else(|| {
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
    // The object is taken once, up front, because restoring a column writes
    // to the line the refusal would otherwise still be reading.
    let located = line.shared_url().cloned();
    let refused =
        |reason| super::arrow::row_error(ordinal, None, located.as_deref(), &column.name, reason);
    let text = |value| {
        read_bytes(value).map_err(|error| {
            refused(smol_str::format_smolstr!(
                "{}",
                crate::text::elide_display(&error)
            ))
        })
    };
    match &column.source {
        // What the batch states of the event is the line's word: a line
        // read back keeps the identity a message named as its source. The
        // columns never null spell nothing as the epoch and the nil
        // identity, and nothing stated is nothing to restate: the line
        // reads its own instant and identity, as it did before the batch.
        TextSource::Event(held) => {
            let nothing = match held {
                crate::graph::EventColumn::CurrUnix => value.temporal_count() == Some(0),
                crate::graph::EventColumn::CurrUuid | crate::graph::EventColumn::CrossUuid => {
                    matches!(value, Scalar::Uuid(uuid) if uuid.is_nil())
                }
                _ => false,
            };
            if !nothing {
                held.record(line, value);
            }
            match held {
                // The chain is the object a line came from, so a cross code
                // that reads as a URL locates the line again. One that does
                // not is an ordinary code and locates nothing, which is the
                // unlocated line it always was.
                crate::graph::EventColumn::CrossCode => {
                    if let Some(url) = value.as_str().and_then(|text| Url::from_str(text).ok()) {
                        line.set_sourceurl(Some(Arc::new(url)));
                    }
                }
                // The place in the chain is the row number, so it restores
                // the line's index under the same offset the read counted
                // from; a number before that offset is refused rather than
                // wrapped.
                crate::graph::EventColumn::SeqNum => {
                    let number = value
                        .as_i128()
                        .and_then(|number| u64::try_from(number - i128::from(start_rownum)).ok())
                        .ok_or_else(|| {
                            refused(SmolStr::new_static(
                                "expected a sequence number at or after start_rownum",
                            ))
                        })?;
                    line.set_index(number);
                }
                _ => {}
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
        // The body made the line before this loop opened, because a line
        // cannot be made without one; restating it here would drop every
        // reading stated since.
        TextSource::Body => {}
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
    }
    Ok(())
}
