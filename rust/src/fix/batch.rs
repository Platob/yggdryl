//! A capture in, columns out.
//!
//! Plumbing between two things that already exist: the [readers](super::FixCodec)
//! that turn a line into a message, and the crate's one batch reader that
//! every format returns and every consumer is written against. Nothing here
//! is a second parser, a second streaming shape, or a second byte accounting.
//!
//! Returning [`BatchReader`](crate::arrow::BatchReader) is what makes a day of
//! capture writable to Parquet with no code in this module knowing what
//! Parquet is. A FIX-specific batch iterator would be one adapter away from
//! every existing consumer, and the adapter is the bug: bounds, casts and
//! projection would each have to be re-implemented or quietly lost.
//!
//! # The schema is decided before the first row is read
//!
//! A batch reader answers `schema()` before it yields anything, so the columns
//! come from the options and the dictionary and never from the data. A schema
//! inferred from the capture's first line is the one thing a column consumer
//! cannot survive.
//!
//! Three groups: what it is, what it means, what was sent.
//!
//! | group | columns |
//! | --- | --- |
//! | identity | `msgtype`, `branch`, `version`, `msghash`, `direction` |
//! | meaning | one per lifted facet, typed as that facet's field is typed |
//! | arrival | `entries`, a list of `tag`/`branch`/`key`/`value` |
//!
//! `entries` is not optional. The facet columns are a convenience over a
//! subset; the entries list *is* the row, and it is what makes a batch a
//! lossless capture rather than one reader's summary of it. A caller wanting
//! facets alone projects the batch afterwards, which already exists.
//!
//! A column per tag seen is deliberately not the shape: it makes the schema
//! depend on the data, gives a mixed capture a thousand mostly-null columns,
//! and is reconstructible from `entries` by whoever actually wants it.

use std::borrow::Cow;
use std::sync::Arc;

use smol_str::SmolStr;

use arrow_array::types::{GenericBinaryType, GenericStringType};
use arrow_array::{Array, ArrayRef, GenericByteArray, RecordBatch, StringArray};
use arrow_schema::DataType as ArrowDataType;

use crate::MimeType;
use crate::arrow::BatchReader;
use crate::arrow::value::value_from_array;
use crate::media::IORecordOptions;
use crate::types::MsgDirection;
use crate::{DataType, Error, Field, Level, Metadata, Result, Scalar, Version};

use super::build::Fill;
use super::codec::FixCodec;
use super::msg::FixMsg;
use super::record::{RowParameters, empty, transform_bytes};
use super::{ENTRIES_COLUMN, FixBranch, FixRegistry};

/// The column a payload is read from when the options name none.
///
/// Reading one record is not this surface, so the column it defaults to is
/// defined beside the record readers rather than here; the name is carried
/// here because the options that set it are this page's.
pub use super::record::DEFAULT_PAYLOAD_COLUMN;

/// The separator FIX itself uses.
pub const SOH: u8 = 0x01;

/// The column a row states its direction in, which outranks the reading the
/// batch reader would otherwise make from the line.
use super::record::DIRECTION_COLUMN;

/// How a capture is read into columns.
///
/// The shared record options say how batches are shaped and bounded; the rest
/// say how a line becomes a message, and each is the per-stream form of an
/// argument the byte readers take per call.
#[derive(Clone, Debug)]
pub struct FixOptions {
    /// The root's name.
    pub name: SmolStr,
    /// A declared datatype, which wins over the default shape.
    ///
    /// Its columns are filled the way every fixed row is: by the tag each
    /// column's field carries, or the one its name spells. A column carrying
    /// neither is left for whoever read the capture to fill.
    pub dtype: Option<DataType>,
    /// Metadata carried onto the root.
    pub metadata: Metadata,
    /// Whether a completion cast may lose information.
    pub safe: bool,
    /// Bytes per batch, whichever of this and `batch_row_size` binds first.
    pub batch_byte_size: Option<u64>,
    /// Rows per batch.
    pub batch_row_size: Option<usize>,
    /// The bound on how many result rows flow in total.
    pub max_row_size: Option<u64>,
    /// The bound on the result rows' Arrow bytes.
    pub max_byte_size: Option<u64>,
    /// The publication cadence.
    pub commit_row_size: Option<usize>,
    /// The compression level a sink writes at.
    pub level: Level,
    /// Columns a merge matches on.
    pub merge_by_names: Vec<String>,
    /// Columns a projection keeps.
    pub select_by_names: Vec<String>,
    /// Partition values a scan is filtered to.
    pub filter_partitions: Vec<(String, String)>,

    /// Which column carries the bytes parsed.
    pub payload_column: SmolStr,
    /// The separator a numeric frame is written with.
    pub separator: u8,
    /// The dialect, where the caller pins one.
    pub branch: Option<FixBranch>,
    /// The version built messages are read at.
    pub version: Option<Version>,
    /// The spellings that mean "nothing was sent".
    pub null_values: Vec<String>,
    /// The direction a line with no verb in front of its payload took.
    ///
    /// `SENT` by default: a session's own log is written by the side doing the
    /// sending, so its unmarked lines are the ones it sent and its inbound
    /// lines are the ones it bothered to mark. A capture taken from the other
    /// side sets `RECV`, and one whose silence really means unknown sets
    /// `None`. Any verb a line does carry beats this.
    pub direction: Option<&'static str>,
    /// Whether an adjacent republication is dropped.
    ///
    /// Off by default, and it must be: with it on, the output row count no
    /// longer equals the input line count, so a batch stops aligning with its
    /// source by position and cannot be joined back to it. What went is
    /// counted rather than silent.
    pub dedup: bool,
    /// Whether each message is filled with what it implies.
    ///
    /// Off by default: a derived value is indistinguishable from a stated one
    /// once it is in the row, so filling has to be asked for. See
    /// [`FixCodec::enrich_fixmsg`](super::FixCodec::enrich_fixmsg).
    pub enrich: bool,
}

/// The batch size a FIX read targets when the caller states none.
///
/// A capture is tens of millions of lines and the row shape varies by three
/// orders of magnitude between a heartbeat and a market-data snapshot, so a
/// row bound alone makes memory unpredictable: the same bound is a few
/// megabytes of one and gigabytes of the other. Targeting bytes instead keeps
/// a batch about the same size whatever arrived, and 128 MiB is large enough
/// that the per-batch cost - building the arrays, crossing a reader boundary,
/// writing a row group - is amortized to nothing, while still leaving several
/// batches in flight on an ordinary machine.
///
/// It is a target rather than a ceiling. The estimate accumulates per row
/// from what was appended, because an in-progress builder cannot be measured
/// the way a finished batch can, and a non-zero bound always yields at least
/// one row - so one enormous message can never produce an empty batch.
pub const DEFAULT_BATCH_BYTE_SIZE: u64 = 128 * 1024 * 1024;

impl Default for FixOptions {
    fn default() -> Self {
        Self {
            name: SmolStr::new_static("fix"),
            dtype: None,
            metadata: Metadata::default(),
            safe: true,
            batch_byte_size: Some(DEFAULT_BATCH_BYTE_SIZE),
            batch_row_size: None,
            max_row_size: None,
            max_byte_size: None,
            commit_row_size: None,
            level: Level::default(),
            merge_by_names: Vec::new(),
            select_by_names: Vec::new(),
            filter_partitions: Vec::new(),
            payload_column: SmolStr::new_static(DEFAULT_PAYLOAD_COLUMN),
            separator: SOH,
            branch: None,
            version: None,
            null_values: super::DEFAULT_NULL_VALUES
                .iter()
                .map(|held| (*held).to_owned())
                .collect(),
            direction: Some(MsgDirection::SENT),
            dedup: false,
            enrich: false,
        }
    }
}

impl IORecordOptions for FixOptions {
    crate::record_options_fields!();
}

impl FixOptions {
    /// Opens the default options.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the payload from a differently named column.
    #[must_use]
    pub fn with_payload_column(mut self, column: impl Into<SmolStr>) -> Self {
        self.payload_column = column.into();
        self
    }

    /// Writes and splits numeric frames with `separator`.
    #[must_use]
    pub const fn with_separator(mut self, separator: u8) -> Self {
        self.separator = separator;
        self
    }

    /// Pins the dialect, so no row infers one.
    #[must_use]
    pub fn with_branch(mut self, branch: FixBranch) -> Self {
        self.branch = Some(branch);
        self
    }

    /// Pins the version built messages are read at.
    ///
    /// A value is translated through the code spellings that version declares;
    /// no column is renamed or retyped by it.
    #[must_use]
    pub const fn with_version(mut self, version: Version) -> Self {
        self.version = Some(version);
        self
    }

    /// Sets the direction a line with no verb takes.
    #[must_use]
    pub const fn with_direction(mut self, direction: Option<&'static str>) -> Self {
        self.direction = direction;
        self
    }

    /// Drops each row whose digest equals the one before it.
    ///
    /// Switching this on surrenders the row-in/row-out correspondence every
    /// other path here keeps: the output no longer aligns with the input by
    /// position. What was dropped is counted, never silent.
    #[must_use]
    pub const fn with_dedup(mut self, dedup: bool) -> Self {
        self.dedup = dedup;
        self
    }

    /// Replaces the spellings that mean "nothing was sent".
    #[must_use]
    pub fn with_null_values<I, S>(mut self, spellings: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.null_values = spellings.into_iter().map(Into::into).collect();
        self
    }

    /// The reader these options describe.
    fn reader(&self, registry: Arc<FixRegistry>) -> FixCodec {
        let mut reader = FixCodec::new(registry).with_null_values(self.null_values.clone());
        if let Some(branch) = &self.branch {
            reader = reader.with_branch(branch);
        }
        if let Some(version) = self.version {
            reader = reader.with_version(version);
        }
        reader
    }

    /// The root this reader answers `schema()` with, built without reading.
    ///
    /// A declared field wins, exactly as it does for every other reader in the
    /// crate; with none declared, this is the shape.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the declared datatype or one
    /// of the built columns does not make a field.
    pub fn source_field(&self, registry: &FixRegistry) -> Result<Field> {
        if let Some(dtype) = &self.dtype {
            return Ok(dtype.clone().required_field(self.name.clone()));
        }
        super::fix_schema(registry, self.name.clone())
    }
}

/// FIX rows as ordinary record batches.
pub struct FixBatchReader;

impl FixBatchReader {
    /// Rows of bytes in, batches out.
    ///
    /// A row in is a row out. Nothing in a row's content can fail a batch: a
    /// row that could not be typed is `unknown`, a row with no pairs is a row
    /// with no entries, and the `Result` is for I/O only. The output row count
    /// equals the input line count, which is what lets a capture be joined
    /// back to its source by position - the one exception is `dedup`, which
    /// says so where it is switched on.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the options do not make a
    /// root field.
    pub fn from_rows<I>(
        registry: Arc<FixRegistry>,
        rows: I,
        options: FixOptions,
    ) -> Result<BatchReader>
    where
        I: IntoIterator<Item = Result<Vec<u8>>>,
        I::IntoIter: Send + 'static,
    {
        let field = options.source_field(&registry)?;
        let reader = options.reader(Arc::clone(&registry));
        let default = options.direction;
        let enrich = options.enrich;
        let messages = rows.into_iter().map(move |row| {
            let row = row?;
            // A row in is a row out: a line the reader refuses is not a line
            // lost, it is a message with nothing in it, and the count still
            // matches the capture's.
            let message = reader
                .transform_line(&row, enrich)
                .unwrap_or_else(|_| empty(&reader, super::build::RowExtras::NONE));
            let direction = direction_of(&row, default);
            Ok((message, direction, Vec::new()))
        });
        Self::stream(field, messages, &options)
    }

    /// A column of frames in, batches out - a capture already in Arrow.
    ///
    /// The source's other columns are carried through unchanged, ahead of the
    /// FIX columns: a capture's arrival timestamp and file offset are what a
    /// monitor orders and joins on, and because the row counts match exactly
    /// carrying them is a slice rather than a join. A column that was read as
    /// a parameter is still carried, because a monitor needs to see the value
    /// it supplied rather than infer that it was used.
    ///
    /// This is [`Self::from_rows`] with one column named, over the record
    /// constructor - not a second body of code that would drift from it.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the options do not make a
    /// root field, or the source reader's own failure.
    pub fn from_column(
        registry: Arc<FixRegistry>,
        source: BatchReader,
        column: &str,
        options: FixOptions,
    ) -> Result<BatchReader> {
        let options = options.with_payload_column(column);
        options
            .reader(registry)
            .with_payload_column(options.payload_column.clone())
            .transform_arrow_reader(source, &options, options.enrich)
    }

    /// The body [`FixCodec::transform_arrow_reader`] is, with the codec in hand.
    pub(super) fn from_codec(
        reader: &FixCodec,
        source: BatchReader,
        options: &FixOptions,
    ) -> Result<BatchReader> {
        let registry = Arc::clone(reader.registry());
        let read = options.source_field(&registry)?;
        let carrier = crate::arrow::field_from_arrow_schema("row", source.schema().as_ref())?;
        // Which of the capture's own columns survive the FIX columns' claim on
        // a name, and where every column a row is read from sits, are decided
        // once here, from the schema, rather than per row.
        let kept = super::schema::carried(&carrier, &read);
        let field = super::fix_schema_carrying(&carrier, &read)?;
        let reader = options.reader(Arc::clone(&registry));
        let columns = Columns::resolve(&carrier, &options.payload_column, kept, &reader);
        let rows = Rows {
            source,
            columns,
            reader,
            default: options.direction,
            enrich: options.enrich,
            held: None,
        };
        Self::stream(field, rows, options)
    }

    /// The one stream both constructors end in.
    fn stream<I>(field: Field, messages: I, options: &FixOptions) -> Result<BatchReader>
    where
        I: Iterator<Item = Result<(FixMsg, Option<&'static str>, Vec<Scalar>)>> + Send + 'static,
    {
        let dedup = options.dedup;
        // The one column no message carries, found once: the direction is read
        // from the line in front of the frame, which is gone by the time a row
        // is built. Its two values are built once, as the column holds them.
        let direction_at = super::schema::fix_column_of(&field, super::MSGDIRECTION_TAG);
        let directions = direction_at.map(|at| {
            let column = &field.fields()[at];
            let held = |direction: &str| {
                column
                    .scalar(Scalar::from(direction))
                    .unwrap_or(Scalar::Null)
            };
            (held(MsgDirection::SENT), held(MsgDirection::RECV))
        });
        // The rows are filled against the same schema the reader publishes; a
        // clone shares it rather than building a second one, and the tag each
        // column answers for is read off it once rather than once per row.
        let schema = field.clone();
        let tags = super::schema::fix_column_tags(&schema);
        let mut last: Option<u128> = None;
        let rows = messages.filter_map(move |held| match held {
            Err(error) => Some(Err(error)),
            Ok((message, direction, front)) => {
                if dedup {
                    let digest = message.digest();
                    if last == Some(digest) {
                        return None;
                    }
                    last = Some(digest);
                }
                let direction = match (direction, &directions) {
                    (Some(MsgDirection::SENT), Some((sent, _))) => sent.clone(),
                    (Some(MsgDirection::RECV), Some((_, recv))) => recv.clone(),
                    _ => Scalar::Null,
                };
                Some(row_of(
                    &message,
                    &schema,
                    &tags,
                    direction_at,
                    direction,
                    front,
                ))
            }
        });
        // Every value in a row went through the contract of the field it
        // lands under - the message's own through the dictionary's fields,
        // the derived ones through their columns, the carried ones through
        // the Arrow reading - so the funnel is told so rather than made to
        // find it out on every leaf of every row.
        Ok(crate::arrow::rows::canonical_result_reader(
            &field,
            rows,
            options.batch_row_size(),
            options.batch_byte_size(),
            options.commit_row_size(),
            options.max_row_size(),
        )?)
    }
}

/// One message as the fixed row its columns are read from.
///
/// `front` is the capture's own columns, already in schema order, and it leads
/// the row: a monitor orders and joins on the arrival time and the file offset
/// the capture supplied, and because the row counts match exactly, carrying
/// them is a slice rather than a join.
fn row_of(
    message: &FixMsg,
    schema: &Field,
    tags: &[Option<i32>],
    direction_at: Option<usize>,
    direction: Scalar,
    front: Vec<Scalar>,
) -> Result<Scalar> {
    // The row is the schema's whole width already: a carried column is named
    // by no tag, so it comes back null and is filled here rather than spliced
    // in, which keeps a column position an index into the row itself.
    let mut held = message.row_values(schema, tags)?;
    for (slot, value) in held.iter_mut().zip(front) {
        *slot = value;
    }
    if let Some(slot) = direction_at.and_then(|at| held.get_mut(at)) {
        *slot = direction;
    }
    Ok(Scalar::from_sequence(held))
}

/// What a whole payload column says about itself, without building a message.
///
/// Three `Utf8` arrays the length of the input: the media type each record
/// infers, the raw `MsgType` its frame spells, and the direction it moved.
/// This is the column form of the three readings the byte readers already
/// make per line, for a stage that classifies a capture before deciding what
/// to parse -- the same shallow scan and the same verbs, so the classifying
/// stage and the parsing one can never disagree.
///
/// No message is built and nothing is resolved against a dictionary, which is
/// what makes this cheap enough to run over every line of a capture.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] when the column holds neither bytes nor
/// text.
pub fn classify_arrow_array(
    column: &ArrayRef,
    default: Option<&'static str>,
) -> Result<(ArrayRef, ArrayRef, ArrayRef)> {
    let payloads = Payloads::over(column).ok_or_else(|| Error::InvalidRecord {
        path: SmolStr::new_static(""),
        reason: crate::text::expected_got(
            format_args!("a binary or utf8 payload column"),
            format_args!("{}", column.data_type()),
        ),
    })?;
    let lines: Vec<&[u8]> = (0..column.len()).map(|row| payloads.get(row)).collect();
    let inferred: Vec<MimeType> = lines
        .iter()
        .map(|line| MimeType::infer_bytes(line))
        .collect();
    let protocol: Vec<&str> = inferred.iter().map(MimeType::as_str).collect();
    let msgtype: Vec<Option<&str>> = lines
        .iter()
        .map(|line| {
            crate::mime_type::line::inspect(line)
                .msgtype()
                .and_then(|value| std::str::from_utf8(value).ok())
        })
        .collect();
    let direction: Vec<Option<&str>> = lines
        .iter()
        .map(|line| direction_of(line, default))
        .collect();
    Ok((
        Arc::new(StringArray::from(protocol)),
        Arc::new(StringArray::from(msgtype)),
        Arc::new(StringArray::from(direction)),
    ))
}

/// One payload column as the byte slices it holds.
///
/// The four layouts text and binary arrive in, downcast once and read per
/// row as a borrowed slice: a payload is read by the codec and copied only
/// into what the message keeps of it.
enum Payloads<'batch> {
    Binary(&'batch GenericByteArray<GenericBinaryType<i32>>),
    LargeBinary(&'batch GenericByteArray<GenericBinaryType<i64>>),
    Utf8(&'batch GenericByteArray<GenericStringType<i32>>),
    LargeUtf8(&'batch GenericByteArray<GenericStringType<i64>>),
}

impl<'batch> Payloads<'batch> {
    /// The column's slices, or nothing where its layout is none of the four.
    fn over(column: &'batch ArrayRef) -> Option<Self> {
        use arrow_array::cast::AsArray;

        Some(match column.data_type() {
            ArrowDataType::Binary => Self::Binary(column.as_bytes::<GenericBinaryType<i32>>()),
            ArrowDataType::LargeBinary => {
                Self::LargeBinary(column.as_bytes::<GenericBinaryType<i64>>())
            }
            ArrowDataType::Utf8 => Self::Utf8(column.as_bytes::<GenericStringType<i32>>()),
            ArrowDataType::LargeUtf8 => {
                Self::LargeUtf8(column.as_bytes::<GenericStringType<i64>>())
            }
            _ => return None,
        })
    }

    /// The bytes one row carries, empty where it carries none.
    fn get(&self, row: usize) -> &'batch [u8] {
        match self {
            Self::Binary(held) if !held.is_null(row) => held.value(row),
            Self::LargeBinary(held) if !held.is_null(row) => held.value(row),
            Self::Utf8(held) if !held.is_null(row) => held.value(row).as_bytes(),
            Self::LargeUtf8(held) if !held.is_null(row) => held.value(row).as_bytes(),
            _ => &[],
        }
    }
}

/// One row's payload as the bytes it is.
///
/// Borrowed from the column where the layout is one of the four, and read
/// out of the cell where it is another: a column of any text or byte layout
/// still carries a payload, and a null carries none.
fn payload_bytes<'batch>(
    dtype: &DataType,
    column: &'batch ArrayRef,
    row: usize,
) -> Result<Cow<'batch, [u8]>> {
    if column.is_null(row) {
        return Ok(Cow::Borrowed(&[]));
    }
    if let Some(held) = Payloads::over(column) {
        return Ok(Cow::Borrowed(held.get(row)));
    }
    let held = value_from_array(dtype, column.as_ref(), row)?;
    Ok(held
        .as_bytes()
        .map(<[u8]>::to_vec)
        .or_else(|| held.as_str().map(|text| text.as_bytes().to_vec()))
        .map_or(Cow::Borrowed(&[]), Cow::Owned))
}

/// A field a row's own column fills, beside the tag it carries: resolved
/// once per stream, non-null as a built child is.
type Filled = Option<(Field, i32)>;

/// Where each column a row is read from sits, decided once per stream.
///
/// The parameter columns are found by the fold every record column is found
/// by, so a batch and a record name them the same way; the carried columns
/// are the capture's own, in the order they lead the row.
struct Columns {
    payload: Option<usize>,
    branch: Option<usize>,
    beginstring: Option<usize>,
    separator: Option<usize>,
    direction: Option<usize>,
    /// The column stating the row's own clock, which stamps the message.
    clock: Option<usize>,
    /// The column naming the plugin that logged the row, which fills the
    /// plugin session the row's direction says it moved between.
    plugin: Option<usize>,
    /// The columns whose names reach a field, each beside the field it fills.
    ///
    /// Resolved once from the schema and the dictionary: a column named after
    /// nothing the dictionary knows is never read per row for it, and the
    /// dictionary is never probed per row for one it does know.
    fills: Vec<(usize, Field, i32)>,
    /// The plugin session fields a sent and a received row fill.
    sessions: (Filled, Filled),
    kept: Vec<usize>,
    /// Each source column's datatype, so a cell is read under its own.
    dtypes: Vec<DataType>,
}

impl Columns {
    fn resolve(carrier: &Field, payload: &str, kept: Vec<usize>, reader: &FixCodec) -> Self {
        let fields = carrier.fields();
        let named = |wanted: &str| {
            fields
                .iter()
                .position(|held| crate::types::folds_equal(held.name(), wanted))
        };
        let fills = fields
            .iter()
            .enumerate()
            .filter(|(_, held)| !super::record::is_parameter(held.name(), payload))
            .filter_map(|(at, held)| {
                let (field, tag) = reader.fill_target(held.name())?;
                Some((at, field, tag))
            })
            .collect();
        let registry = reader.registry();
        let session =
            |direction: &'static str| super::record::plugin_session(registry, Some(direction));
        Self {
            payload: named(payload),
            branch: named(super::record::BRANCH_COLUMN),
            beginstring: named(super::record::BEGINSTRING_COLUMN),
            separator: named(super::record::SEPARATOR_COLUMN),
            direction: named(DIRECTION_COLUMN),
            clock: named(super::record::CLOCK_COLUMN),
            plugin: named(super::record::PLUGIN_COLUMN),
            fills,
            sessions: (session(MsgDirection::SENT), session(MsgDirection::RECV)),
            kept,
            dtypes: fields.iter().map(|held| held.dtype().clone()).collect(),
        }
    }
}

/// The capture's rows, one message each, read a batch at a time.
///
/// One batch is held and read cell by cell, straight out of its arrays: the
/// payload as the bytes it is, a parameter column as the text it holds, a
/// carried column as the value it becomes. Nothing converts a batch whole
/// and nothing is copied that the message does not keep, so a row costs its
/// parse and the few cells the row actually reads.
struct Rows {
    source: BatchReader,
    columns: Columns,
    reader: FixCodec,
    default: Option<&'static str>,
    enrich: bool,
    /// The batch being read, beside the row the next pull reads.
    held: Option<(RecordBatch, usize)>,
}

impl Rows {
    /// One row of one batch as the message it is, the direction it moved and
    /// the capture's own columns carried in front of it.
    fn row(
        &self,
        batch: &RecordBatch,
        row: usize,
    ) -> Result<(FixMsg, Option<&'static str>, Vec<Scalar>)> {
        let cell = |at: usize| {
            value_from_array(&self.columns.dtypes[at], batch.column(at).as_ref(), row)
                .map_err(Error::from)
        };
        // A column absent, null or empty is silence.
        let stated = |at: Option<usize>| -> Result<Option<Scalar>> {
            at.map(cell)
                .transpose()
                .map(|held| held.filter(|value| !value.is_null()))
        };
        let payload = self
            .columns
            .payload
            .map(|at| payload_bytes(&self.columns.dtypes[at], batch.column(at), row))
            .transpose()?
            .unwrap_or_default();
        let branch = stated(self.columns.branch)?;
        let beginstring = stated(self.columns.beginstring)?;
        let separator = stated(self.columns.separator)?;
        let clock = stated(self.columns.clock)?;
        // The direction a row states outranks any reading of its line, and it
        // is decided before the build: it picks which plugin session the
        // row's plugin fills.
        let direction = stated(self.columns.direction)?
            .and_then(|held| {
                let text = held.as_str()?;
                [MsgDirection::SENT, MsgDirection::RECV]
                    .into_iter()
                    .find(|known| known.eq_ignore_ascii_case(text))
            })
            .or_else(|| direction_of(&payload, self.default));
        // The cells that fill fields, read only where the row states them.
        let mut cells: Vec<(&Field, i32, Scalar)> =
            Vec::with_capacity(self.columns.fills.len() + 1);
        for (at, field, tag) in &self.columns.fills {
            if let Some(value) = stated(Some(*at))? {
                cells.push((field, *tag, value));
            }
        }
        if let Some(plugin) = stated(self.columns.plugin)? {
            let session = match direction {
                Some(MsgDirection::SENT) => self.columns.sessions.0.as_ref(),
                Some(MsgDirection::RECV) => self.columns.sessions.1.as_ref(),
                _ => None,
            };
            if let Some((field, tag)) = session {
                cells.push((field, *tag, plugin));
            }
        }
        let fills: Vec<Fill<'_>> = cells
            .iter()
            .map(|(field, tag, value)| Fill {
                field,
                tag: *tag,
                value,
            })
            .collect();
        let parameters = RowParameters {
            branch: branch.as_ref().and_then(Scalar::as_str),
            beginstring: beginstring.as_ref().and_then(Scalar::as_str),
            separator: separator.as_ref().and_then(Scalar::as_str),
            clock: clock.as_ref(),
            fills: &fills,
        };
        let message = transform_bytes(&self.reader, parameters, &payload, self.enrich)?;
        // By position: the columns kept were decided from the schema, and a
        // row of that schema arrives in that order.
        let front = self
            .columns
            .kept
            .iter()
            .map(|at| cell(*at))
            .collect::<Result<Vec<_>>>()?;
        Ok((message, direction, front))
    }
}

impl Iterator for Rows {
    type Item = Result<(FixMsg, Option<&'static str>, Vec<Scalar>)>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let next = match &mut self.held {
                Some((batch, at)) if *at < batch.num_rows() => {
                    let row = *at;
                    *at += 1;
                    Some(row)
                }
                _ => None,
            };
            if let Some(row) = next {
                let (batch, _) = self.held.as_ref()?;
                return Some(self.row(batch, row));
            }
            // The batch is spent, or none is held yet: the next one is pulled
            // and the spent one dropped, so one batch is ever in hand.
            match self.source.next() {
                Some(Ok(batch)) => self.held = Some((batch, 0)),
                Some(Err(error)) => {
                    self.held = None;
                    return Some(Err(crate::arrow::from_reader_error(error).into()));
                }
                None => return None,
            }
        }
    }
}

/// The direction a whole captured line moved.
fn direction_of(line: &[u8], default: Option<&'static str>) -> Option<&'static str> {
    MsgDirection::infer_bytes(line).or(default)
}

/// One row's values beside the names its schema gave them.
/// Emits one arrival entry and everything under it, in wire order.
///
/// The materialized levels are walked as they stand; the binary leaf is
/// decoded through the crate's one JSON parser and walked the same way. A
/// leaf that cannot be decoded is rejected, never skipped, because a
/// re-emitted line must reproduce a line that actually arrived - a hole where
/// a pair was is a different message.
///
/// # Errors
///
/// Returns the JSON reader's failure on an undecodable leaf.
fn emit_entry(pair: &[Scalar], separator: u8, line: &mut Vec<u8>) -> Result<()> {
    if let (Some(key), Some(value)) = (
        pair.get(2).and_then(Scalar::as_str),
        pair.get(3).and_then(Scalar::as_str),
    ) {
        line.extend_from_slice(key.as_bytes());
        line.push(b'=');
        line.extend_from_slice(value.as_bytes());
        line.push(separator);
    }
    let Some(tail) = pair.get(4) else {
        return Ok(());
    };
    if let Some(children) = tail.as_sequence() {
        for child in children {
            if let Some(held) = child.as_sequence() {
                emit_entry(held, separator, line)?;
            }
        }
        return Ok(());
    }
    if let Some(bytes) = tail.as_bytes() {
        if bytes.is_empty() {
            return Ok(());
        }
        let decoded = crate::from_json_scalar(bytes)?;
        for child in decoded.as_sequence().unwrap_or_default() {
            if let Some(held) = child.as_sequence() {
                emit_entry(held, separator, line)?;
            }
        }
    }
    Ok(())
}

impl FixMsg {
    /// One generic record in, one message out.
    ///
    /// The crate already has a generic record - a name-to-value map, one
    /// `Scalar` variant - and every row-oriented reader in it produces one, so
    /// taking that shape means this accepts a row from any of them with no
    /// conversion at the boundary. The payload column is read by the byte
    /// readers; every other named column is a parameter they already take.
    ///
    /// | column | supplies |
    /// | --- | --- |
    /// | the payload column, named by options | the bytes parsed |
    /// | `branch` | the dialect |
    /// | `beginstring` | the source version |
    /// | `sep` | the separator |
    /// | `direction` | the direction, stated |
    /// | `timestamp` | the row's own clock, which stamps the message |
    /// | any column named after a field | that field, where the message did not state it |
    ///
    /// A record carrying only a payload column behaves exactly as the byte
    /// reader behaves, which is what makes this an entry point rather than a
    /// second contract.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when the value is not a record at all.
    pub fn from_record(
        registry: Arc<FixRegistry>,
        record: &Scalar,
        options: &FixOptions,
    ) -> Result<Self> {
        options
            .reader(registry)
            .with_payload_column(options.payload_column.clone())
            .transform_record(record, options.enrich)
    }
}

/// Batches back to the wire, streamed.
///
/// The wire is rebuilt from each row's `entries`, never from the columns: the
/// facets are a lossy projection by construction, and rebuilding a frame from
/// them would emit a message that was never sent. A batch without an `entries`
/// column cannot be written and says so plainly.
///
/// One batch is pulled, its rows written, and it is dropped. The source is
/// never concatenated and no output buffer bigger than a row is held.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] when the source has no `entries` column,
/// the source reader's own failure, or the sink's write failure.
pub fn write_fix(
    source: BatchReader,
    mut sink: impl std::io::Write,
    options: &FixOptions,
) -> Result<u64> {
    let field = crate::arrow::field_from_arrow_schema("row", source.schema().as_ref())?;
    let Some(entries) = field
        .fields()
        .iter()
        .position(|held| held.name() == ENTRIES_COLUMN)
    else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new(ENTRIES_COLUMN),
            reason: crate::text::expected_got(
                "a batch carrying its arrival record",
                "one holding only lifted columns",
            ),
        });
    };
    let dtype = field.fields()[entries].dtype().clone();
    let mut written = 0_u64;
    let mut line = Vec::new();
    for batch in source {
        let batch = batch.map_err(crate::arrow::from_reader_error)?;
        let column = batch.column(entries);
        for row in 0..batch.num_rows() {
            // The arrival record alone is read out of the batch: the wire is
            // rebuilt from it and from nothing beside it.
            let record = value_from_array(&dtype, column.as_ref(), row)?;
            let Some(held) = record.as_sequence() else {
                continue;
            };
            line.clear();
            for entry in held {
                let Some(pair) = entry.as_sequence() else {
                    continue;
                };
                emit_entry(pair, options.separator, &mut line)?;
            }
            line.push(b'\n');
            sink.write_all(&line)?;
            written += 1;
        }
    }
    sink.flush()?;
    Ok(written)
}
