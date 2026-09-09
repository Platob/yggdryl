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
use super::record::{column, column_bytes, column_text, empty, transform_record};
use super::{ENTRIES_COLUMN, FixBranch, FixMessages, FixRegistry};

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
    ///
    /// Its columns are filled the way every fixed row is: by the tag each
    /// column's name spells, so a declared root names its columns `35` and
    /// `55` rather than `msgtype` and `symbol`. A column no tag names is
    /// left for whoever read the capture to fill.
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
    /// Off by default. Bulk configuration documents already expand into one
    /// row per configuration; enabling this also removes adjacent duplicates.
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
    /// This also applies to successive messages expanded from a bulk document.
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
    /// Ordinary lines yield one message; bulk configuration documents yield
    /// one per configuration. Empty or unrecognized ordinary lines retain an
    /// empty message. Input I/O and errors yielded by a parsed document stop
    /// the output stream after its completed prefix.
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
        let messages = rows.into_iter().flat_map(move |row| {
            let (messages, direction) = match row {
                Ok(row) => (
                    reader
                        .transform_line(&row, enrich)
                        .unwrap_or_else(|_| FixMessages::one(empty(&reader))),
                    direction_of(&row, default),
                ),
                Err(error) => (FixMessages::from_result(Err(error)), None),
            };
            messages.map(move |message| message.map(|message| (message, direction, Vec::new())))
        });
        Self::stream(field, messages, &options)
    }

    /// A column of frames in, batches out - a capture already in Arrow.
    ///
    /// Each emitted message carries its source row's other columns ahead of
    /// the FIX columns. An expanded configuration document repeats the source
    /// timestamp, offset, and stated direction on every emitted row.
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
        let kept = super::schema::carried(&carrier, &read);
        let field = super::fix_schema_carrying(&carrier, &read)?;
        let reader = options.reader(Arc::clone(&registry));
        let payload = options.payload_column.clone();
        let default = options.direction;
        let enrich = options.enrich;
        let source_schema = source.schema();

        let records = source
            .flat_map(move |batch| {
                let (batch, mut error) = match batch {
                    Ok(batch) if batch.schema() == source_schema => (Some(batch), None),
                    Ok(_) => (
                        None,
                        Some(Error::conflict(
                            "the capture reader's declared Arrow schema",
                            "a different batch schema",
                            "FIX capture",
                        )),
                    ),
                    Err(error) => (
                        None,
                        Some(crate::Error::from(crate::arrow::from_reader_error(error))),
                    ),
                };
                let carrier = carrier.clone();
                let mut index = 0;
                std::iter::from_fn(move || {
                    if let Some(error) = error.take() {
                        return Some(Err(error));
                    }
                    let batch = batch.as_ref()?;
                    if index == batch.num_rows() {
                        return None;
                    }
                    let row = batch
                        .columns()
                        .iter()
                        .zip(carrier.fields())
                        .map(|(column, field)| {
                            crate::arrow::value::value_from_array(
                                field.dtype(),
                                column.as_ref(),
                                index,
                            )
                            .map_err(crate::Error::from)
                        })
                        .collect::<Result<Vec<_>>>();
                    index += 1;
                    Some(row.map(Scalar::from_sequence))
                })
            })
            .flat_map(move |row| {
                let (messages, direction, front) = match row {
                    Ok(row) => {
                        let record = named(&names, &row);
                        let bytes = column_bytes(&record, &payload).unwrap_or_default();
                        let messages = FixMessages::from_result(transform_record(
                            &reader, &record, &bytes, enrich,
                        ));
                        let direction = stated(&record).or_else(|| direction_of(&bytes, default));
                        let front = kept
                            .iter()
                            .map(|at| {
                                record
                                    .get(*at)
                                    .map_or(Scalar::Null, |(_, held)| held.clone())
                            })
                            .collect::<Vec<_>>();
                        (messages, direction, front)
                    }
                    Err(error) => (FixMessages::from_result(Err(error)), None, Vec::new()),
                };
                messages
                    .map(move |message| message.map(|message| (message, direction, front.clone())))
            });
        Self::stream(field, records, options)
    }

    /// The one stream both constructors end in.
    fn stream<I>(field: Field, messages: I, options: &FixOptions) -> Result<BatchReader>
    where
        I: Iterator<Item = Result<(FixMsg, Option<&'static str>, Vec<Scalar>)>> + Send + 'static,
    {
        let dedup = options.dedup;
        // The one column no message carries, found once: the direction is read
        // from the line in front of the frame, which is gone by the time a row
        // is built.
        let direction_at = field.index_of(&super::schema::rendered(super::MSGDIRECTION_TAG));
        // The rows are filled against the same schema the reader publishes; a
        // clone shares it rather than building a second one.
        let schema = field.clone();
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
                Some(row_of(&message, &schema, direction_at, direction, front))
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
/// the row. Expanded messages repeat this same source-row prefix.
fn row_of(
    message: &FixMsg,
    schema: &Field,
    direction_at: Option<usize>,
    direction: Option<&'static str>,
    front: Vec<Scalar>,
) -> Result<Scalar> {
    // The row is the schema's whole width already: a carried column is named
    // by no tag, so it comes back null and is filled here rather than spliced
    // in, which keeps a column position an index into the row itself.
    let mut held = message
        .into_row(schema)?
        .as_sequence()
        .map(<[Scalar]>::to_vec)
        .unwrap_or_default();
    for (slot, value) in held.iter_mut().zip(front) {
        *slot = value;
    }
    if let Some(slot) = direction_at.and_then(|at| held.get_mut(at)) {
        *slot = direction.map_or(Scalar::Null, Scalar::from);
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

fn named(names: &[SmolStr], row: &Scalar) -> Vec<(SmolStr, Scalar)> {
    let Some(values) = row.as_sequence() else {
        return Vec::new();
    };
    names.iter().cloned().zip(values.iter().cloned()).collect()
}

/// The direction a record states, which outranks any reading.
fn stated(record: &[(SmolStr, Scalar)]) -> Option<&'static str> {
    let held = column_text(record, DIRECTION_COLUMN)?;
    [MsgDirection::SENT, MsgDirection::RECV]
        .into_iter()
        .find(|known| known.eq_ignore_ascii_case(&held))
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
    /// Returns [`Error::Parse`] for a non-record, the first emitted failure,
    /// or [`Error::InvalidRecord`] when the record expands to zero or multiple
    /// messages. Use [`FixCodec::transform_record`] for bulk documents.
    pub fn from_record(
        registry: Arc<FixRegistry>,
        record: &Scalar,
        options: &FixOptions,
    ) -> Result<Self> {
        let mut messages = options
            .reader(registry)
            .with_payload_column(options.payload_column.clone())
            .transform_record(record, options.enrich)?;
        let message = messages
            .next()
            .transpose()?
            .ok_or_else(|| Error::InvalidRecord {
                path: options.payload_column.clone(),
                reason: "expected exactly one FIX message, got zero messages".into(),
            })?;
        if let Some(next) = messages.next() {
            next?;
            return Err(Error::InvalidRecord {
                path: options.payload_column.clone(),
                reason: "expected exactly one FIX message, got multiple messages".into(),
            });
        }
        Ok(message)
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
    if !names.iter().any(|held| held == ENTRIES_COLUMN) {
        return Err(Error::InvalidRecord {
            path: SmolStr::new(ENTRIES_COLUMN),
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
            let Some(held) = column(&record, ENTRIES_COLUMN).and_then(Scalar::as_sequence) else {
                continue;
            };
            let mut line = Vec::new();
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
