//! The codec's Arrow twins: a capture in, columns out, and back.
//!
//! Plumbing between two things that already exist: the [codec](super::FixCodec)
//! that turns a line into a message, and the crate's one batch reader that
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
//! come from the source's schema and the dictionary and never from the data.
//! A schema inferred from the capture's first line is the one thing a column
//! consumer cannot survive.
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
//!
//! # Batches close on the bytes they were read from
//!
//! A batch is cut by a running total of raw bytes against the codec's
//! [`batch_byte_size`](super::FixCodec::with_batch_byte_size), never by a
//! walk over the values it holds. Reading a capture, the statistic is the
//! payload column's byte length in each input batch, read once from its
//! offsets buffer and spread evenly over that batch's rows; reading messages,
//! it is the length of each message's arrival record - and, for a message
//! that has none, the leaves of the row it fills, so a stream of lifted-only
//! rows is bounded too. Several small input batches accumulate into one
//! output batch, one input batch larger than the target is split by rows in
//! proportion, and a batch always holds at least one row.

use std::borrow::Cow;
use std::sync::Arc;

use arrow_array::types::{GenericBinaryType, GenericStringType};
use arrow_array::{Array, ArrayRef, GenericByteArray, RecordBatch, StructArray};
use arrow_schema::DataType as ArrowDataType;

use crate::arrow::BatchReader;
use crate::arrow::rows::{Closing, ROW_OVERHEAD, appended_bytes, canonical_closing_reader};
use crate::arrow::value::value_from_array;
use crate::types::MsgDirection;
use crate::{DataType, DataTypeKind, Error, Field, Result, Scalar};

use super::build::{
    BEGINSTRING_COLUMN, CLOCK_COLUMN, DIRECTION_COLUMN, PLUGINID_COLUMN, version_of,
};
use super::build::{Fill, RowExtras};
use super::codec::{FixCodec, SOH};
use super::msg::FixMsg;
use super::{ENTRIES_COLUMN, FixEntry, FixMessages, FixRegistry};

/// The name the fixed row's root takes: what the schema is asked for, and
/// what a batch of FIX rows is read back under.
const ROOT_NAME: &str = "fix";

impl FixCodec {
    /// The root a batch of rows is read against, from its Arrow schema.
    fn row_field(schema: &arrow_schema::Schema) -> Result<Field> {
        Ok(crate::arrow::field_from_arrow_schema(ROOT_NAME, schema)?)
    }

    /// Parses a stream of Arrow batches of capture rows into a stream of
    /// batches of FIX rows.
    ///
    /// The batch a text reader answers with is already the shape this wants -
    /// one row per line, the payload in the column
    /// [`Self::with_payload_column`] names and the capture's own `url`,
    /// `rownum` and `direction` beside it - so this takes it whole rather than
    /// through a row-at-a-time boundary. The schema is decided before the
    /// first row, from the source's schema and the dictionary: the capture's
    /// own columns lead, the fixed FIX columns follow
    /// ([`fix_schema_carrying`](super::fix_schema_carrying)), and a capture
    /// column named as a FIX column yields to it.
    ///
    /// Each row is read cell by cell out of the arrays and parsed through the
    /// same funnel as a line: the payload as [`Self::parse_line`] reads it,
    /// the `pluginid`, `beginstring` and `timestamp` columns as
    /// [`Self::parse_text_line`] reads the captures of those names - `pluginid`
    /// both filling its
    /// own column and naming the dialect the row is read under - the
    /// `direction` column, which this reader alone reads, and every other
    /// column named after a field the dictionary knows filling that field
    /// where the line left it unsaid. `direction` is a parameter to both
    /// readers even so, because a column the record reader left to the fills
    /// would silently land on a field. Where each column sits and which
    /// field it fills is decided once from the schema, and a row's dialect
    /// once per distinct plugin name, so no row copies the codec or asks the
    /// dictionary a question the row before it asked. A line the reader
    /// refuses is a row holding an empty message, never a row lost, so a row
    /// in is a row out; a bulk configuration document is one row per MBean,
    /// each repeating its source row's carried columns. The direction column
    /// [`MSGDIRECTION_TAG`](super::MSGDIRECTION_TAG) names takes the row's
    /// `direction` column, else the verb in front of its payload, else
    /// [`Self::with_direction`].
    ///
    /// Batches close on the raw bytes of the payload column, read once per
    /// input batch from its offsets and spread over the batch's rows, against
    /// [`Self::with_batch_byte_size`]; a source batch of another schema than
    /// the first is a conflict, because every cell is read by the position the
    /// declared schema gave it.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the source's schema or the
    /// dictionary does not make a root field, and [`Error::InvalidRecord`]
    /// naming the payload column when the source has no column of that name
    /// or its column holds neither text nor bytes - a source that would parse
    /// nothing is refused before a row is read rather than answered as rows
    /// of empty messages; the source reader's own failure is an error batch.
    pub fn parse_text_arrow_reader(&self, source: BatchReader) -> Result<BatchReader> {
        let read = super::fix_schema(self.registry(), ROOT_NAME)?;
        let carrier = Self::row_field(source.schema().as_ref())?;
        // Which of the capture's own columns survive the FIX columns' claim on
        // a name, and where every column a row is read from sits, are decided
        // once here, from the schema, rather than per row.
        let kept = super::schema::carried(&carrier, &read);
        let field = super::fix_schema_carrying(&carrier, &read)?;
        let columns = Columns::resolve(&carrier, self.payload_column(), kept, self)?;
        let rows = Rows {
            source,
            columns,
            codec: self.clone(),
            held: None,
        };
        // A bulk configuration document expands into one row per
        // configuration, each repeating its source row's carried columns and
        // the first carrying the row's whole charge.
        let rows = rows.flat_map(|held| {
            let (messages, direction, front, charge) = match held {
                Ok(held) => held,
                Err(error) => (FixMessages::from_result(Err(error)), None, Vec::new(), 0),
            };
            messages.enumerate().map(move |(index, message)| {
                message.map(|message| {
                    let charge = if index == 0 { charge } else { 0 };
                    (message, direction, front.clone(), charge)
                })
            })
        });
        // The one column no message carries, found once: the direction is
        // read from the line in front of the frame, which is gone by the time
        // a row is built. Its two values are built once, as the column holds
        // them.
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
        let plan = super::schema::column_plan(&schema)?;
        let target = self.batch_byte_size();
        let mut carried = 0_u64;
        let rows = rows.map(move |held| match held {
            Err(error) => Closing(Err(error), false),
            Ok((message, direction, front, charge)) => {
                let closes = closes(&mut carried, charge, target);
                let direction = match (direction, &directions) {
                    (Some(MsgDirection::SENT), Some((sent, _))) => sent.clone(),
                    (Some(MsgDirection::RECV), Some((_, recv))) => recv.clone(),
                    _ => Scalar::Null,
                };
                let row = row_of(&message, &schema, &plan, direction_at, direction, front);
                Closing(row, closes)
            }
        });
        // Every value in a row went through the contract of the field it
        // lands under - the message's own through the dictionary's fields,
        // the derived ones through their columns, the carried ones through
        // the Arrow reading - so the funnel is told so rather than made to
        // find it out on every leaf of every row.
        Ok(canonical_closing_reader(&field, rows)?)
    }

    /// Fills a stream of batches of FIX rows with what each message implies.
    ///
    /// [`Self::enrich_messages`] over batches: each row becomes the message
    /// it holds through [`FixMsg::from_row`] - any schema that constructor
    /// accepts, the fixed row carrying a capture's columns or not - is filled
    /// as [`Self::enrich_message`] fills one, and is written back through
    /// [`FixMsg::into_row`] under the **same** schema, so a carried column
    /// returns to its place. Nothing is parsed again, and the arrival record
    /// is carried through untouched. Batches close on the raw bytes of each
    /// message's arrival record, against [`Self::with_batch_byte_size`].
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the source's schema does not
    /// make a root field; a row that is not a FIX row and the source
    /// reader's own failure are error batches.
    pub fn enrich_messages_arrow_reader(&self, source: BatchReader) -> Result<BatchReader> {
        let schema = Self::row_field(source.schema().as_ref())?;
        let codec = self.clone();
        let filled = self
            .messages(source)
            .map(move |held| held.and_then(|message| codec.enrich_message(message)));
        self.arrow_reader(schema, filled)
    }

    /// A stream of batches of FIX rows as the stream of messages it holds.
    ///
    /// Each row is one message through [`FixMsg::from_row`] under the
    /// source's schema, its entries rebuilt from the
    /// [`ENTRIES_COLUMN`](super::ENTRIES_COLUMN) where the schema carries it,
    /// so a batch written by [`Self::parse_text_arrow_reader`] comes back as
    /// the messages that made it - re-emitting its lines, digesting, restating
    /// and stamping as they did - at the cost of the values it already holds
    /// and no parse. One batch is held at a time. A source batch of another
    /// schema than the first is a conflict item, and a row the schema does
    /// not make a message of is an error item; either fuses the stream.
    ///
    /// This is one half of what [`Self::enrich_messages_arrow_reader`]
    /// composes, public because [`Self::lifecycle`], [`FixDedup`](super::FixDedup)
    /// and [`FixMsg::into_latest`] compose over batches the same way: the
    /// messages a batch holds, through the stage, into [`Self::arrow_reader`]
    /// under the schema read off the batch.
    pub fn messages(
        &self,
        source: BatchReader,
    ) -> impl Iterator<Item = Result<FixMsg>> + Send + use<> {
        Messages::over(Arc::clone(self.registry()), source)
    }

    /// A stream of messages as a stream of batches of FIX rows under `schema`.
    ///
    /// Each message fills one row through [`FixMsg::into_row`]: the schema's
    /// columns in its order, each by its tag or its group's counter, a column
    /// carrying neither by the child of its name. The other half of what the
    /// Arrow twins compose; [`fix_schema`](super::fix_schema) is the schema a
    /// parsed message fills whole, and a schema read off a batch by
    /// [`Self::messages`] is the one its messages return to. Batches close on
    /// the raw bytes of each message's arrival record - the lengths of every
    /// key and value it holds, walked without allocating - against
    /// [`Self::with_batch_byte_size`]. An error item yields the completed
    /// prefix, then the error, and fuses the reader.
    ///
    /// # Errors
    ///
    /// Returns the Arrow layer's refusal when `schema` does not make an Arrow
    /// schema.
    pub fn arrow_reader<I>(&self, schema: Field, messages: I) -> Result<BatchReader>
    where
        I: IntoIterator<Item = Result<FixMsg>>,
        I::IntoIter: Send + 'static,
    {
        let root = schema.clone();
        let target = self.batch_byte_size();
        let mut carried = 0_u64;
        let rows = messages.into_iter().map(move |held| match held {
            Err(error) => Closing(Err(error), false),
            Ok(message) => {
                // A message with no arrival record - built by hand, or read
                // back from rows that carried only lifted columns - has no
                // wire to be measured by, and charging it the bare row width
                // would leave such a stream with no bound but the row count:
                // it is charged the leaves of the row it fills instead.
                let wire = (!message.entries().is_empty()).then(|| wire_size(message.entries()));
                let row = message.into_row(&schema);
                let charge =
                    wire.unwrap_or_else(|| row.as_ref().map_or(ROW_OVERHEAD, appended_bytes));
                let closes = closes(&mut carried, charge, target);
                Closing(row, closes)
            }
        });
        Ok(canonical_closing_reader(&root, rows)?)
    }

    /// Writes a stream of batches of FIX rows back to the wire, streamed.
    ///
    /// The encode direction of the same exchange: each row is the message
    /// [`Self::messages`] reads out of it, and the line written is
    /// [`FixMsg::into_bytes`] with the separator [`Self::with_separator`]
    /// pinned, else [`SOH`], then a newline. The wire is rebuilt from the
    /// arrival record, never from the columns: the facets are a lossy
    /// projection by construction, and rebuilding a frame from them would
    /// emit a message that was never sent. A batch without the
    /// [`ENTRIES_COLUMN`](super::ENTRIES_COLUMN) cannot be written and says
    /// so before a row is read. A row in is a line out - a row whose message
    /// held no pairs is an empty line - and the count of lines is answered.
    ///
    /// One batch is pulled, its rows written, and it is dropped. The source is
    /// never concatenated and no output buffer bigger than a row is held.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the source has no entries column,
    /// the source reader's own failure, a row's refusal to be a message, or
    /// the sink's write failure.
    pub fn write_arrow_reader(
        &self,
        source: BatchReader,
        mut sink: impl std::io::Write,
    ) -> Result<u64> {
        let field = Self::row_field(source.schema().as_ref())?;
        if field.index_of(ENTRIES_COLUMN).is_none() {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static(ENTRIES_COLUMN),
                reason: crate::text::expected_got(
                    "a batch carrying its arrival record",
                    "one holding only lifted columns",
                ),
            });
        }
        let separator = self.separator().unwrap_or(SOH);
        let mut written = 0_u64;
        for message in self.messages(source) {
            let mut line = message?.into_bytes(separator);
            line.push(b'\n');
            sink.write_all(&line)?;
            written += 1;
        }
        sink.flush()?;
        Ok(written)
    }
}

/// Whether the batch closes after a row charged `charge` bytes.
///
/// The running total crosses the target and starts again from nothing, so
/// the row that crosses it is the last of its batch and a target of zero
/// closes after every row.
fn closes(carried: &mut u64, charge: u64, target: u64) -> bool {
    *carried = carried.saturating_add(charge);
    if *carried >= target {
        *carried = 0;
        return true;
    }
    false
}

/// The raw bytes one arrival record is, walked without allocating: the
/// length of every key and value at every level, plus the per-row width.
fn wire_size(entries: &[FixEntry]) -> u64 {
    entries.iter().fold(ROW_OVERHEAD, |sum, entry| {
        sum + entry.key().len() as u64 + entry.value().len() as u64 + wire_size(entry.children())
    })
}

/// Whether a column of `dtype` carries a payload the codec can read: text or
/// bytes in any layout, a dictionary or run-end encoding of one included.
fn carries_payload(dtype: &DataType) -> bool {
    match dtype {
        DataType::Dictionary(held) => carries_payload(&held.value),
        DataType::RunEndEncoded(held) => carries_payload(held.values.dtype()),
        other => matches!(
            other.kind(),
            DataTypeKind::Text | DataTypeKind::Bytes | DataTypeKind::Ascii
        ),
    }
}

/// Where the column named `payload` sits in `carrier`, or the refusal a
/// source earns when it is not there to be read or holds nothing the codec
/// reads: named by the column, saying what was expected and what the source
/// carries instead.
///
/// A source that answers none of the intake steps fails here rather than
/// yielding a row of empty messages per row, which would be a capture with
/// no data and no error.
fn payload_column_of(carrier: &Field, payload: &str, at: Option<usize>) -> Result<usize> {
    let actual = match at {
        None => {
            let names: Vec<&str> = carrier.fields().iter().map(Field::name).collect();
            format!("a source carrying only [{}]", names.join(", "))
        }
        Some(at) if !carries_payload(carrier.fields()[at].dtype()) => {
            format!("a column of {}", carrier.fields()[at].dtype())
        }
        Some(at) => return Ok(at),
    };
    Err(Error::InvalidRecord {
        path: smol_str::SmolStr::new(payload),
        reason: crate::text::expected_got(
            format_args!("a text or binary column named {payload}"),
            actual,
        ),
    })
}

/// One message as the fixed row its columns are read from.
///
/// `front` is the capture's own columns, already in schema order, and it leads
/// the row. Expanded messages repeat this same source-row prefix.
fn row_of(
    message: &FixMsg,
    schema: &Field,
    plan: &[super::schema::Column],
    direction_at: Option<usize>,
    direction: Scalar,
    front: Vec<Scalar>,
) -> Result<Scalar> {
    // The row is the schema's whole width already: a carried column is named
    // by no tag, so it comes back null and is filled here rather than spliced
    // in, which keeps a column position an index into the row itself.
    let mut held = message.row_values(schema, plan)?;
    for (slot, value) in held.iter_mut().zip(front) {
        *slot = value;
    }
    if let Some(slot) = direction_at.and_then(|at| held.get_mut(at)) {
        *slot = direction;
    }
    Ok(Scalar::from_sequence(held))
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

    /// The bytes the whole column carries, read once off its offsets: the
    /// last minus the first, never a sum over the rows.
    fn bytes(&self) -> u64 {
        fn span<O: arrow_array::OffsetSizeTrait>(offsets: &[O]) -> u64 {
            match (offsets.first(), offsets.last()) {
                (Some(first), Some(last)) => (*last - *first).as_usize() as u64,
                _ => 0,
            }
        }
        match self {
            Self::Binary(held) => span(held.value_offsets()),
            Self::LargeBinary(held) => span(held.value_offsets()),
            Self::Utf8(held) => span(held.value_offsets()),
            Self::LargeUtf8(held) => span(held.value_offsets()),
        }
    }
}

/// The raw bytes one payload column holds, as the batching statistic.
///
/// Read off the offsets for the four byte layouts; any other layout answers
/// the memory its buffers occupy, which is the one whole-column answer Arrow
/// gives without a walk.
fn raw_bytes(column: &ArrayRef) -> u64 {
    Payloads::over(column).map_or_else(
        || column.get_buffer_memory_size() as u64,
        |held| held.bytes(),
    )
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

/// Where each column a row is read from sits, decided once per stream.
///
/// The parameter columns are found by the fold every record column is found
/// by, so a batch and a record name them the same way; the carried columns
/// are the capture's own, in the order they lead the row.
/// Whether a column is one this reader takes as a parameter or the payload,
/// rather than one that could fill a field by its name.
///
/// A batch is columns, so this is the batch reader's own question: a line
/// states the same facts as row-header captures, which the codec resolves by
/// name once and reads by position.
fn is_parameter(name: &str, payload: &str) -> bool {
    [payload, BEGINSTRING_COLUMN, CLOCK_COLUMN, DIRECTION_COLUMN]
        .iter()
        .any(|held| crate::types::folds_equal(held, name))
}

struct Columns {
    /// The column the frame is read from, proven there and readable.
    payload: usize,
    beginstring: Option<usize>,
    direction: Option<usize>,
    /// The column stating the row's own clock, which stamps the message.
    clock: Option<usize>,
    /// The column naming the plugin that logged the row.
    ///
    /// Its own position, not a fill's: the dialect a row is read under is
    /// stated by this column whether or not the dictionary also holds a
    /// field for it to fill, and a registry that dropped the crate's own
    /// `pluginid` still reads its rows under the dialect they name. The cell
    /// is decoded once and the same value fills the field where there is
    /// one. A payload column spelled so is the payload alone, as the record
    /// reader has it.
    pluginid: Option<usize>,
    /// The columns whose names reach a field, each beside the field it fills.
    ///
    /// Resolved once from the schema and the dictionary: a column named after
    /// nothing the dictionary knows is never read per row for it, and the
    /// dictionary is never probed per row for one it does know.
    fills: Vec<(usize, Field, i32)>,
    kept: Vec<usize>,
    /// Each source column's datatype, so a cell is read under its own.
    dtypes: Vec<DataType>,
}

impl Columns {
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when `carrier` has no column named
    /// `payload`, or that column holds neither text nor bytes.
    fn resolve(carrier: &Field, payload: &str, kept: Vec<usize>, codec: &FixCodec) -> Result<Self> {
        let fields = carrier.fields();
        let named = |wanted: &str| {
            fields
                .iter()
                .position(|held| crate::types::folds_equal(held.name(), wanted))
        };
        let payload_at = payload_column_of(carrier, payload, named(payload))?;
        let fills = fields
            .iter()
            .enumerate()
            .filter(|(_, held)| !is_parameter(held.name(), payload))
            .filter_map(|(at, held)| {
                let (field, tag) = codec.fill_target(held.name())?;
                Some((at, field, tag))
            })
            .collect();
        Ok(Self {
            payload: payload_at,
            beginstring: named(BEGINSTRING_COLUMN),
            direction: named(DIRECTION_COLUMN),
            clock: named(CLOCK_COLUMN),
            pluginid: named(PLUGINID_COLUMN).filter(|at| *at != payload_at),
            fills,
            kept,
            dtypes: fields.iter().map(|held| held.dtype().clone()).collect(),
        })
    }
}

/// The capture's rows, the messages each carries, read a batch at a time.
///
/// One batch is held and read cell by cell, straight out of its arrays: the
/// payload as the bytes it is, a parameter column as the text it holds, a
/// carried column as the value it becomes. Nothing converts a batch whole
/// and nothing is copied that the message does not keep, so a row costs its
/// parse and the few cells the row actually reads.
struct Rows {
    source: BatchReader,
    columns: Columns,
    codec: FixCodec,
    /// The batch being read, the row the next pull reads, and the raw bytes
    /// each of its rows is charged: the payload column's bytes over the
    /// batch's rows, read once when the batch was pulled.
    held: Option<(RecordBatch, usize, u64)>,
}

impl Rows {
    /// One row of one batch as the messages it carries, the direction it moved
    /// and the capture's own columns carried in front of it.
    ///
    /// An ordinary line is one message; a bulk configuration document is one
    /// per configuration, and the row's carried columns lead each of them.
    fn row(
        &self,
        batch: &RecordBatch,
        row: usize,
    ) -> Result<(FixMessages, Option<&'static str>, Vec<Scalar>)> {
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
        let at = self.columns.payload;
        let payload = payload_bytes(&self.columns.dtypes[at], batch.column(at), row)?;
        let beginstring = stated(self.columns.beginstring)?;
        let clock = stated(self.columns.clock)?;
        // The direction a row states outranks any reading of its line.
        let direction = stated(self.columns.direction)?
            .and_then(|held| {
                let text = held.as_str()?;
                [MsgDirection::SENT, MsgDirection::RECV]
                    .into_iter()
                    .find(|known| known.eq_ignore_ascii_case(text))
            })
            .or_else(|| MsgDirection::infer_bytes(&payload))
            .or(self.codec.direction());
        // The plugin that logged the row, read by its own column and decoded
        // once: it names the row's dialect whether or not the dictionary also
        // holds a field of that name, and where it does the same value is
        // what fills it.
        let mut plugin = stated(self.columns.pluginid)?;
        let branch = plugin
            .as_ref()
            .and_then(Scalar::as_str)
            .and_then(|named| self.codec.dialect_of(named));
        // The cells that fill fields, read only where the row states them.
        let mut cells: Vec<(&Field, i32, Scalar)> = Vec::with_capacity(self.columns.fills.len());
        for (at, field, tag) in &self.columns.fills {
            let held = if Some(*at) == self.columns.pluginid {
                plugin.take()
            } else {
                stated(Some(*at))?
            };
            let Some(value) = held else {
                continue;
            };
            cells.push((field, *tag, value));
        }
        let fills: Vec<Fill<'_>> = cells
            .iter()
            .map(|(field, tag, value)| Fill {
                field,
                tag: *tag,
                value,
            })
            .collect();
        let extras = RowExtras {
            branch,
            version: beginstring
                .as_ref()
                .and_then(Scalar::as_str)
                .and_then(version_of),
            clock: clock.as_ref(),
            fills: &fills,
        };
        let messages = self.codec.parse_bytes_with(extras, &payload);
        // By position: the columns kept were decided from the schema, and a
        // row of that schema arrives in that order.
        let front = self
            .columns
            .kept
            .iter()
            .map(|at| cell(*at))
            .collect::<Result<Vec<_>>>()?;
        Ok((messages, direction, front))
    }

    /// The raw bytes each row of one batch is charged: the payload column's
    /// bytes, read once off its offsets, over the rows - rounded up so a
    /// batch of tiny rows still closes - plus the width every row costs.
    fn charge_of(&self, batch: &RecordBatch) -> u64 {
        let rows = batch.num_rows().max(1) as u64;
        let payload = raw_bytes(batch.column(self.columns.payload));
        payload.div_ceil(rows) + ROW_OVERHEAD
    }
}

impl Iterator for Rows {
    type Item = Result<(FixMessages, Option<&'static str>, Vec<Scalar>, u64)>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let next = match &mut self.held {
                Some((batch, at, charge)) if *at < batch.num_rows() => {
                    let row = *at;
                    *at += 1;
                    Some((row, *charge))
                }
                _ => None,
            };
            if let Some((row, charge)) = next {
                let (batch, ..) = self.held.as_ref()?;
                return Some(
                    self.row(batch, row)
                        .map(|(messages, direction, front)| (messages, direction, front, charge)),
                );
            }
            // The batch is spent, or none is held yet: the next one is pulled
            // and the spent one dropped, so one batch is ever in hand.
            match self.source.next() {
                Some(Ok(batch)) if batch.schema() != self.source.schema() => {
                    // Every column is read by the position the declared schema
                    // gave it, so a batch of another schema is a conflict
                    // rather than a row read from the wrong column.
                    self.held = None;
                    return Some(Err(Error::conflict(
                        "the capture reader's declared Arrow schema",
                        "a different batch schema",
                        "FIX capture",
                    )));
                }
                Some(Ok(batch)) => {
                    let charge = self.charge_of(&batch);
                    self.held = Some((batch, 0, charge));
                }
                Some(Err(error)) => {
                    self.held = None;
                    return Some(Err(crate::arrow::from_reader_error(error).into()));
                }
                None => return None,
            }
        }
    }
}

/// The messages a stream of batches of FIX rows holds, read a batch at a time.
///
/// One batch is held as the Struct array it is and read row by row into the
/// row value the schema declares, which [`FixMsg::from_row`] makes a message
/// of. The schema is read once, off the source; a schema that does not make
/// a root is the one item the stream yields.
struct Messages {
    registry: Arc<FixRegistry>,
    source: BatchReader,
    schema: Field,
    /// The refusal the source's schema earned, yielded once.
    pending: Option<Error>,
    /// The batch being read, beside the row the next pull reads.
    held: Option<(StructArray, usize)>,
    done: bool,
}

impl Messages {
    fn over(registry: Arc<FixRegistry>, source: BatchReader) -> Self {
        let (schema, pending) = match FixCodec::row_field(source.schema().as_ref()) {
            Ok(schema) => (schema, None),
            Err(error) => (DataType::Null.required_field(ROOT_NAME), Some(error)),
        };
        Self {
            registry,
            source,
            schema,
            pending,
            held: None,
            done: false,
        }
    }
}

impl Iterator for Messages {
    type Item = Result<FixMsg>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(error) = self.pending.take() {
            self.done = true;
            return Some(Err(error));
        }
        if self.done {
            return None;
        }
        loop {
            if let Some((batch, at)) = &mut self.held {
                if *at < batch.len() {
                    let row = *at;
                    *at += 1;
                    let read = value_from_array(self.schema.dtype(), batch, row)
                        .map_err(Error::from)
                        .and_then(|row| {
                            FixMsg::from_row(Arc::clone(&self.registry), &self.schema, &row)
                        });
                    if read.is_err() {
                        self.done = true;
                    }
                    return Some(read);
                }
            }
            match self.source.next() {
                Some(Ok(batch)) if batch.schema() != self.source.schema() => {
                    self.done = true;
                    return Some(Err(Error::conflict(
                        "the FIX reader's declared Arrow schema",
                        "a different batch schema",
                        "FIX rows",
                    )));
                }
                Some(Ok(batch)) => self.held = Some((StructArray::from(batch), 0)),
                Some(Err(error)) => {
                    self.done = true;
                    return Some(Err(crate::arrow::from_reader_error(error).into()));
                }
                None => {
                    self.done = true;
                    return None;
                }
            }
        }
    }
}

impl std::iter::FusedIterator for Messages {}
