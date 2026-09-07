//! A capture in, columns out.
//!
//! Plumbing between two things that already exist: the [readers](super::reader)
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

use std::sync::Arc;

use smol_str::SmolStr;

use arrow_array::{Array, ArrayRef, StringArray};

use crate::MimeType;
use crate::arrow::BatchReader;
use crate::media::IORecordOptions;
use crate::types::MsgDirection;
use crate::{DataType, Error, Field, Level, Metadata, Result, Scalar, Version};

use super::codec::FixCodec;
use super::msg::FixMsg;
use super::{FixBranch, FixRegistry};

/// The column a payload is read from when the options name none.
pub const DEFAULT_PAYLOAD_COLUMN: &str = "body";

/// The separator FIX itself uses.
pub const SOH: u8 = 0x01;

/// The columns a record supplies as parameters rather than as payload.
///
/// Each names an argument the byte readers already take, so a record carrying
/// only a payload behaves exactly as the byte reader behaves - which is what
/// makes this an entry point rather than a second contract.
const BRANCH_COLUMN: &str = "branch";
const BEGINSTRING_COLUMN: &str = "beginstring";
const SEPARATOR_COLUMN: &str = "sep";
const DIRECTION_COLUMN: &str = "direction";

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
    /// The version built messages are expressed in.
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
}

impl Default for FixOptions {
    fn default() -> Self {
        Self {
            name: SmolStr::new_static("fix"),
            dtype: None,
            metadata: Metadata::default(),
            safe: true,
            batch_byte_size: None,
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

    /// Pins the version built messages are expressed in.
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
        let messages = rows.into_iter().map(move |row| {
            let row = row?;
            // A row in is a row out: a line the reader refuses is not a line
            // lost, it is a message with nothing in it, and the count still
            // matches the capture's.
            let message = reader.read_line(&row).unwrap_or_else(|_| empty(&reader));
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
            .read_arrow_reader(source, &options)
    }

    /// The body [`FixCodec::read_arrow_reader`] is, with the codec in hand.
    pub(super) fn from_codec(
        reader: &FixCodec,
        source: BatchReader,
        options: &FixOptions,
    ) -> Result<BatchReader> {
        let registry = Arc::clone(reader.registry());
        let read = options.source_field(&registry)?;
        let carrier = crate::arrow::field_from_arrow_schema("row", source.schema().as_ref())?;
        let names: Vec<SmolStr> = carrier
            .dtype()
            .as_fields()
            .map(|fields| {
                fields
                    .iter()
                    .map(|held| SmolStr::new(held.name()))
                    .collect()
            })
            .unwrap_or_default();
        // Which of the capture's own columns survive the FIX columns' claim on
        // a name is decided once here, from the schema, rather than per row.
        let projection = super::FixProjection::carrying(&carrier, read)?;
        let field = projection.field().clone();
        let kept: Vec<usize> = projection.carried_positions().to_vec();
        let reader = options.reader(Arc::clone(&registry));
        let payload = options.payload_column.clone();
        let default = options.direction;

        let records = source
            .flat_map(move |batch| match batch {
                Ok(batch) => match crate::arrow::batch_to_value(&batch) {
                    Ok(rows) => rows
                        .as_sequence()
                        .map(<[Scalar]>::to_vec)
                        .unwrap_or_default()
                        .into_iter()
                        .map(Ok)
                        .collect::<Vec<_>>(),
                    Err(error) => vec![Err(error)],
                },
                Err(error) => vec![Err(crate::arrow::from_reader_error(error))],
            })
            .map(move |row| {
                let record = named(&names, &row?);
                let bytes = column_bytes(&record, &payload).unwrap_or_default();
                let message = read_record(&reader, &record, &bytes)?;
                let direction = stated(&record).or_else(|| direction_of(&bytes, default));
                // By position: the columns kept were decided from the schema,
                // and a row of that schema arrives in that order.
                let front = kept
                    .iter()
                    .map(|at| {
                        record
                            .get(*at)
                            .map_or(Scalar::Null, |(_, held)| held.clone())
                    })
                    .collect();
                Ok((message, direction, front))
            });
        Self::stream(field, records, options)
    }

    /// The one stream both constructors end in.
    fn stream<I>(field: Field, messages: I, options: &FixOptions) -> Result<BatchReader>
    where
        I: Iterator<Item = Result<(FixMsg, Option<&'static str>, Vec<Scalar>)>> + Send + 'static,
    {
        let dedup = options.dedup;
        // Resolved once for the whole capture: every row asks for the same
        // columns in the same order, and asking the dictionary per row is the
        // cost a fixed schema exists to remove.
        let projection = super::FixProjection::from_field(field.clone());
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
                Some(row_of(&message, &projection, direction, front))
            }
        });
        Ok(crate::arrow::rows::result_reader(
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
    projection: &super::FixProjection,
    direction: Option<&'static str>,
    front: Vec<Scalar>,
) -> Result<Scalar> {
    // The row is the projection's whole width already: a carried column
    // carries no tag, so it comes back null and is filled here rather than
    // spliced in, which keeps `position_of` an index into the row itself.
    let mut held = message
        .to_row(projection)
        .as_sequence()
        .map(<[Scalar]>::to_vec)
        .unwrap_or_default();
    for (slot, value) in held.iter_mut().zip(front) {
        *slot = value;
    }
    // The direction is the one column no message carries: it is read from the
    // line in front of the frame, which is gone by the time a row is built.
    if let Some(at) = projection.position_of(super::MSGDIRECTION_TAG) {
        if let Some(slot) = held.get_mut(at) {
            *slot = direction.map_or(Scalar::Null, Scalar::from);
        }
    }
    Ok(Scalar::from_sequence(held))
}

/// A row nobody could read, which is still a row.
fn empty(reader: &FixCodec) -> FixMsg {
    reader
        .read_pairs(std::iter::empty::<(&[u8], &[u8])>())
        .expect("an empty message builds")
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
    let lines = payload_lines(column)?;
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

/// One payload column's records as byte slices, whatever layout carries them.
fn payload_lines(column: &ArrayRef) -> Result<Vec<&[u8]>> {
    use arrow_array::cast::AsArray;
    use arrow_array::types::{GenericBinaryType, GenericStringType};

    let rows = column.len();
    let mut held: Vec<&[u8]> = Vec::with_capacity(rows);
    macro_rules! bytes_of {
        ($kind:ty, $text:expr) => {{
            let array = column.as_bytes::<$kind>();
            for row in 0..rows {
                held.push(if array.is_null(row) {
                    &[]
                } else if $text {
                    array.value(row).as_ref()
                } else {
                    array.value(row).as_ref()
                });
            }
        }};
    }
    match column.data_type() {
        arrow_schema::DataType::Binary => bytes_of!(GenericBinaryType<i32>, false),
        arrow_schema::DataType::LargeBinary => bytes_of!(GenericBinaryType<i64>, false),
        arrow_schema::DataType::Utf8 => bytes_of!(GenericStringType<i32>, true),
        arrow_schema::DataType::LargeUtf8 => bytes_of!(GenericStringType<i64>, true),
        other => {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static(""),
                reason: crate::text::expected_got(
                    format_args!("a binary or utf8 payload column"),
                    format_args!("{other}"),
                ),
            });
        }
    }
    Ok(held)
}

/// The direction a whole captured line moved.
fn direction_of(line: &[u8], default: Option<&'static str>) -> Option<&'static str> {
    let at = crate::mime_type::line::payload_at(line).unwrap_or(line.len());
    MsgDirection::at_payload(line, at, default)
}

/// One row's values beside the names its schema gave them.
fn named(names: &[SmolStr], row: &Scalar) -> Vec<(SmolStr, Scalar)> {
    let Some(values) = row.as_sequence() else {
        return Vec::new();
    };
    names.iter().cloned().zip(values.iter().cloned()).collect()
}

/// One column's bytes, however the column is typed.
fn column_bytes(record: &[(SmolStr, Scalar)], name: &str) -> Option<Vec<u8>> {
    let held = column(record, name)?;
    held.as_bytes()
        .map(<[u8]>::to_vec)
        .or_else(|| held.as_str().map(|text| text.as_bytes().to_vec()))
}

/// One column by name, absent when it states nothing.
fn column<'row>(record: &'row [(SmolStr, Scalar)], name: &str) -> Option<&'row Scalar> {
    record
        .iter()
        .find(|(held, _)| crate::types::folds_equal(held, name))
        .map(|(_, value)| value)
        .filter(|held| !held.is_null())
}

/// One column's text.
fn column_text(record: &[(SmolStr, Scalar)], name: &str) -> Option<String> {
    let held = column(record, name)?;
    held.as_str().map(ToOwned::to_owned)
}

/// The direction a record states, which outranks any reading.
fn stated(record: &[(SmolStr, Scalar)]) -> Option<&'static str> {
    let held = column_text(record, DIRECTION_COLUMN)?;
    [MsgDirection::SENT, MsgDirection::RECV]
        .into_iter()
        .find(|known| known.eq_ignore_ascii_case(&held))
}

/// One record read against one codec, the payload taken from `payload`.
///
/// [`FixCodec::read_record`] is the door; this is where the row's own columns
/// are applied, beside the option-driven path the batch reader takes.
pub(super) fn read_record_with(
    reader: &FixCodec,
    record: &Scalar,
    payload: &str,
) -> Result<FixMsg> {
    let Some(held) = record.as_record() else {
        return Err(Error::Parse {
            target: "fix record",
            position: 0,
            reason: crate::text::expected_got("a record", "another value"),
        });
    };
    let row: Vec<(SmolStr, Scalar)> = held
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let bytes = column_bytes(&row, payload).unwrap_or_default();
    read_record(reader, &row, &bytes)
}

/// Reads one record through the byte readers, per-row columns applied.
fn read_record(reader: &FixCodec, record: &[(SmolStr, Scalar)], bytes: &[u8]) -> Result<FixMsg> {
    // A column is the caller speaking per row and an option is the caller
    // speaking per stream, so both outrank the inference the readers fall back
    // on - and the column outranks the option, because it is the more specific
    // statement. A column absent, null or empty is silence, never an
    // instruction, and never an error.
    let mut reader = reader.clone();
    if let Some(name) = column_text(record, BRANCH_COLUMN) {
        if let Ok(branch) = FixBranch::from_str(&name) {
            reader = reader.with_branch(&branch);
        }
    }
    if let Some(held) = column_text(record, BEGINSTRING_COLUMN) {
        let spelling = held.strip_prefix("FIX.").unwrap_or(&held);
        if let Ok(version) = spelling.parse::<Version>() {
            reader = reader.with_version(version);
        }
    }
    if bytes.is_empty() {
        return Ok(empty(&reader));
    }
    // A stated separator is read as a numeric frame with that separator; with
    // none stated the reader picks its own dialect from the frame, which is
    // what a record carrying only a payload has to do.
    let stated = column_text(record, SEPARATOR_COLUMN).and_then(|held| held.bytes().next());
    let built = match stated {
        Some(separator) => reader
            .clone()
            .with_separator(separator)
            .read_fix_line(bytes),
        None => reader.read_line(bytes),
    };
    Ok(built.unwrap_or_else(|_| empty(&reader)))
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
            .read_record(record)
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
/// Returns [`Error::InvalidRecord`] when the source has no arrival-record
/// column - [`ENTRIES_COLUMN`](super::ENTRIES_COLUMN) -
/// the source reader's own failure, or the sink's write failure.
pub fn write_fix(
    source: BatchReader,
    mut sink: impl std::io::Write,
    options: &FixOptions,
) -> Result<u64> {
    let field = crate::arrow::field_from_arrow_schema("row", source.schema().as_ref())?;
    let names: Vec<SmolStr> = field
        .dtype()
        .as_fields()
        .map(|fields| {
            fields
                .iter()
                .map(|held| SmolStr::new(held.name()))
                .collect()
        })
        .unwrap_or_default();
    if !names.iter().any(|held| held == super::ENTRIES_COLUMN) {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static(super::ENTRIES_COLUMN),
            reason: crate::text::expected_got(
                "a batch carrying its arrival record",
                "one holding only lifted columns",
            ),
        });
    }
    let mut written = 0_u64;
    for batch in source {
        let batch = batch.map_err(crate::arrow::from_reader_error)?;
        let rows = crate::arrow::batch_to_value(&batch)?;
        for row in rows.as_sequence().unwrap_or_default() {
            let record = named(&names, row);
            let Some(held) = column(&record, super::ENTRIES_COLUMN).and_then(Scalar::as_sequence)
            else {
                continue;
            };
            let mut line = Vec::new();
            for entry in held {
                let Some(pair) = entry.as_sequence() else {
                    continue;
                };
                let (Some(key), Some(value)) = (
                    pair.get(2).and_then(Scalar::as_str),
                    pair.get(3).and_then(Scalar::as_str),
                ) else {
                    continue;
                };
                line.extend_from_slice(key.as_bytes());
                line.push(b'=');
                line.extend_from_slice(value.as_bytes());
                line.push(options.separator);
            }
            line.push(b'\n');
            sink.write_all(&line)?;
            written += 1;
        }
    }
    sink.flush()?;
    Ok(written)
}
