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
//! | identity | `currunix`, `creatunix`, `currhashcode`, `crosshashcode`, `curruuid`, `crossuuid`, `snapunix`, `sendingtime` |
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

use smol_str::SmolStr;

use crate::arrow::BatchReader;
use crate::arrow::rows::{Closing, ROW_OVERHEAD, appended_bytes, canonical_closing_reader};
use crate::arrow::value::value_from_array;
use crate::{DataType, DataTypeKind, Error, Field, Result, Scalar};

use super::build::{BEGINSTRING_COLUMN, DIRECTION_COLUMN, version_of};
use super::build::{Fill, RowExtras};
use super::codec::{FixCodec, SOH};
use super::msg::FixMsg;
use super::{FIXENTRIES_COLUMN, FixEntry, FixMessages, FixRegistry};

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
    /// `rownum` beside it - so this takes it whole rather than
    /// through a row-at-a-time boundary. The schema is decided before the
    /// first row, from the source's schema and the dictionary: the capture's
    /// own columns lead, the fixed FIX columns follow
    /// ([`fix_schema_carrying`](super::fix_schema_carrying)), and a capture
    /// column named as a FIX column yields to it.
    ///
    /// Each row is read cell by cell out of the arrays and parsed through the
    /// same funnel as a line: the payload as [`Self::parse_line`] reads it,
    /// the `beginstring` column as
    /// [`Self::parse_text_line`] reads the captures of those names, the
    /// `msgdirection` column as the direction the row states, and every
    /// other column named after a field the dictionary knows - `pluginid`
    /// among them - filling that field where the line left it unsaid. Where each
    /// column sits and which field it fills is decided once from the schema,
    /// so no row copies the codec or asks the dictionary a question the row
    /// before it asked.
    ///
    /// [The capture's own columns](FixMsg::from_row) fill nothing: the
    /// carried ones, and the two the crate tags - a `sourceurl` column and a
    /// `recordedat` one - are read off the source row and written straight
    /// into the row this answers, each at its own column, because where a
    /// line was read from is this reader's statement and never the message's.
    /// This is the one door that can state them, and it is why they survive
    /// a parse without a message holding one.
    ///
    /// A line the reader
    /// cannot classify yields no message; malformed-body recovery still obeys
    /// the mandatory field contract. A bulk configuration document is one row per
    /// configuration it named and no row where it named none, each repeating
    /// its source row's carried columns. Tag 385, the column
    /// [`MSGDIRECTION_TAG_NAME`](super::MSGDIRECTION_TAG_NAME) names, takes
    /// the row's stated `msgdirection`, else the reading over the prose in
    /// front of its payload, else [`Self::try_with_direction`]'s pin.
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
        let columns = Columns::resolve(&carrier, self.payload_column(), &kept, self, &field)?;
        let rows = Rows {
            source,
            columns,
            codec: self.clone(),
            held: None,
        };
        // A bulk configuration document expands into one row per
        // configuration, each repeating its source row's own cells and
        // the first carrying the row's whole charge.
        let rows = rows.flat_map(|held| {
            let (messages, restated, charge) = match held {
                Ok(held) => held,
                Err(error) => (FixMessages::from_result(Err(error)), Vec::new(), 0),
            };
            messages.enumerate().map(move |(index, message)| {
                message.map(|message| {
                    let charge = if index == 0 { charge } else { 0 };
                    (message, restated.clone(), charge)
                })
            })
        });
        // The rows are filled against the same schema the reader publishes; a
        // clone shares it rather than building a second one, and the tag each
        // column answers for is read off it once rather than once per row.
        let schema = field.clone();
        let plan = super::schema::column_plan(&schema, self.registry())?;
        let target = self.batch_byte_size();
        let row_target = self.batch_row_size();
        let mut carried = Carried::default();
        let rows = rows.map(move |held| match held {
            Err(error) => Closing(Err(error), false),
            Ok((message, restated, charge)) => {
                let closes = closes(&mut carried, charge, target, row_target);
                let row = row_of(&message, &schema, &plan, restated);
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

    /// Walks a stream of batches of FIX rows as one lifecycle.
    ///
    /// [`Self::lifecycle`] over batches: each row becomes the message it
    /// holds through [`FixMsg::from_row`] - any schema that constructor
    /// accepts, the fixed row carrying a capture's columns or not - the
    /// messages are walked as [`Self::lifecycle`] walks them, and each is
    /// written back under the **same** schema, so a carried column returns
    /// to its place. Nothing is parsed again, and the arrival record is
    /// carried through untouched. Batches close on the raw bytes of each
    /// message's arrival record, against [`Self::with_batch_byte_size`].
    ///
    /// # A row's own cells stay with the message, not with the position
    ///
    /// A walk answers messages in their own order, which is not the order
    /// the rows arrived in, and a message holds nothing about the reading it
    /// arrived through - so [the capture's own columns](FixMsg::from_row)
    /// would be left behind by that permutation if they were paired by
    /// position. They are not. The rows are read, put in the walk's order
    /// here, each beside the cells of the row it came out of, and walked
    /// [streamed](Self::lifecycle): one answer per message, in the order
    /// they were handed over, so the `body` a row was cut from and the
    /// `sourceurl` it names still belong to the message that came out of
    /// that line. A refused message type never enters the walk, exactly as
    /// in [`Self::lifecycle`], and it takes its cells with it.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the source's schema does not
    /// make a root field or a column plan; a row that is not a FIX row and
    /// the source reader's own failure are error batches.
    pub fn lifecycle_arrow_reader(&self, source: BatchReader) -> Result<BatchReader> {
        let schema = Self::row_field(source.schema().as_ref())?;
        // The schema in is the schema out, so every capture column is stated
        // back at the column it was read from.
        let restating = capture_restating(&schema, &schema, self.registry())?;
        let mut failures: Vec<Error> = Vec::new();
        let mut held: Vec<(FixMsg, Vec<(usize, Scalar)>)> = Vec::new();
        for read in self.messages_restating(source, restating) {
            match read {
                Ok(pair) => held.push(pair),
                Err(error) => failures.push(error),
            }
        }
        // What the walk itself would refuse, refused here instead, so a
        // message and its row's cells leave together.
        held.retain(|(message, _)| self.reads_msgtype(message.header().msgtype()));
        // The walk's own order, stably, which is what lets it stream below.
        held.sort_by(|left, right| crate::graph::iterator::order(&left.0, &right.0));
        let (messages, cells): (Vec<FixMsg>, Vec<Vec<(usize, Scalar)>>) = held.into_iter().unzip();
        let mut cells: std::collections::VecDeque<Vec<(usize, Scalar)>> = cells.into();
        let walked = FixCodec::lifecycle_sorted(messages.into_iter().map(Ok))
            .map(move |held| held.map(|message| (message, cells.pop_front().unwrap_or_default())));
        // A failure has no row to be written into, so it is yielded ahead of
        // the walk - where a collected walk has always put one.
        let rows = failures.into_iter().map(Err).chain(walked);
        self.arrow_reader_restating(schema, rows)
    }

    /// A stream of batches of FIX rows as the stream of messages it holds.
    ///
    /// Each row is one message through [`FixMsg::from_row`] under the
    /// source's schema, its entries rebuilt from the
    /// [`FIXENTRIES_COLUMN`](super::FIXENTRIES_COLUMN) where the schema carries it,
    /// so a batch written by [`Self::parse_text_arrow_reader`] comes back as
    /// the messages that made it - re-emitting its lines, digesting, restating
    /// and stamping as they did - at the cost of the values it already holds
    /// and no parse. One batch is held at a time. A source batch of another
    /// schema than the first is a conflict item, and a row the schema does
    /// not make a message of is an error item; either fuses the stream.
    ///
    /// This is one half of what [`Self::lifecycle_arrow_reader`] composes,
    /// public because [`FixDedup`](super::FixDedup) composes over batches
    /// the same way: the messages a batch holds, through the stage, into
    /// [`Self::arrow_reader`] under the schema read off the batch.
    pub fn messages(
        &self,
        source: BatchReader,
    ) -> impl Iterator<Item = Result<FixMsg>> + Send + use<> {
        Messages::over(Arc::clone(self.registry()), source, Vec::new())
            .map(|held| held.map(|(message, _)| message))
    }

    /// [`Self::messages`], each message beside the capture's own cells of
    /// the row it came out of.
    ///
    /// `restating` is what [`capture_restating`] read off the two schemas:
    /// which of the source's columns are the capture's, and where each is
    /// stated in the rows the caller writes. A message holds none of them,
    /// so this is how a door that reads rows and writes rows keeps them.
    fn messages_restating(&self, source: BatchReader, restating: Vec<(usize, usize)>) -> Messages {
        Messages::over(Arc::clone(self.registry()), source, restating)
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
    /// Owned messages and their fallible counterparts are accepted directly.
    ///
    /// # Errors
    ///
    /// Returns the Arrow layer's refusal when `schema` does not make an Arrow
    /// schema.
    pub fn arrow_reader<I>(&self, schema: Field, messages: I) -> Result<BatchReader>
    where
        I: IntoIterator,
        I::Item: Into<Result<FixMsg>>,
        I::IntoIter: Send + 'static,
    {
        let root = schema.clone();
        let target = self.batch_byte_size();
        let row_target = self.batch_row_size();
        let mut carried = Carried::default();
        let rows = messages
            .into_iter()
            .fuse()
            .map(move |held| match held.into() {
                Err(error) => Closing(Err(error), false),
                Ok(message) => {
                    // A message with no arrival record - built by hand, or read
                    // back from rows that carried only lifted columns - has no
                    // wire to be measured by, and charging it the bare row width
                    // would leave such a stream with no bound but the row count:
                    // it is charged the leaves of the row it fills instead.
                    let wire =
                        (!message.entries().is_empty()).then(|| wire_size(message.entries()));
                    let row = message.into_row(&schema);
                    let charge =
                        wire.unwrap_or_else(|| row.as_ref().map_or(ROW_OVERHEAD, appended_bytes));
                    let closes = closes(&mut carried, charge, target, row_target);
                    Closing(row, closes)
                }
            });
        Ok(canonical_closing_reader(&root, rows)?)
    }

    /// [`Self::arrow_reader`] over messages that arrive beside the
    /// capture's own cells, each cell stated at its own column.
    ///
    /// The door for a pass that reads rows and writes rows: a message holds
    /// nothing about the reading it arrived through, so what the row said
    /// for itself comes back from the row rather than from the message.
    /// Rows are charged exactly as [`Self::arrow_reader`] charges them.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when `schema` does not make a
    /// column plan, and the Arrow layer's when it does not make an Arrow
    /// schema.
    fn arrow_reader_restating<I>(&self, schema: Field, rows: I) -> Result<BatchReader>
    where
        I: IntoIterator<Item = Result<(FixMsg, Vec<(usize, Scalar)>)>>,
        I::IntoIter: Send + 'static,
    {
        let root = schema.clone();
        let plan = super::schema::column_plan(&schema, self.registry())?;
        let target = self.batch_byte_size();
        let row_target = self.batch_row_size();
        let mut carried = Carried::default();
        let rows = rows.into_iter().fuse().map(move |held| match held {
            Err(error) => Closing(Err(error), false),
            Ok((message, restated)) => {
                let wire = (!message.entries().is_empty()).then(|| wire_size(message.entries()));
                let row = row_of(&message, &schema, &plan, restated);
                let charge =
                    wire.unwrap_or_else(|| row.as_ref().map_or(ROW_OVERHEAD, appended_bytes));
                let closes = closes(&mut carried, charge, target, row_target);
                Closing(row, closes)
            }
        });
        Ok(canonical_closing_reader(&root, rows)?)
    }

    /// A stream of messages as the rows one message field holds them.
    ///
    /// The third verb, and the one a consumer reads by. A parse lands every
    /// line in the [fixed row](super::fix_schema), filled with what each
    /// message implies, and a [lifecycle](Self::lifecycle) walk states what
    /// each follows; this answers the same messages under whatever field a
    /// consumer reads
    /// by - a venue's own message type, the [fixed row](super::fix_schema)
    /// itself, which keeps every column a capture lands in, or any Struct
    /// root a caller built for the table it is writing.
    ///
    /// Each row is [`FixMsg::into_row`] under `field`: the field's columns in
    /// its order, each filled by the tag its own field carries, the crate's
    /// derivations answering the columns a message did not state, and a value
    /// the column will not hold nulled rather than refused. `field` is read
    /// once here and never per message.
    ///
    /// A column the message does not carry is read off its arrival record
    /// first, which is what makes formatting a narrow row into a wider field
    /// answer more than the narrow row did. What the record cannot say it
    /// does not say: a bridge's packed occurrence, a composed key and a
    /// row-header capture are the codec's readings of a dialect, recorded as
    /// the pairs the bridge wrote, so a row that dropped
    /// their columns has dropped them.
    ///
    /// Nothing is parsed again and nothing is collected: the iterator is the
    /// stream, so ten million messages cost one at a time.
    /// [`Self::format_arrow_reader`] is the same pass over batches, which is
    /// what a capture already in Arrow uses.
    ///
    /// ```
    /// # fn main() -> yggdryl::Result<()> {
    /// # use std::sync::Arc;
    /// # use yggdryl::holder::local::Folder;
    /// # use yggdryl::{FixCodec, FixRegistry, fix_schema};
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// # let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    /// let codec = FixCodec::new(Arc::clone(&registry));
    /// let target = fix_schema(&registry, "fix")?;
    /// let messages = codec.parse_line(b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|10=0|")?;
    ///
    /// let rows: Vec<_> = codec
    ///     .format_messages(messages, &target)
    ///     .collect::<yggdryl::Result<Vec<_>>>()?;
    /// let at = target.index_of("symbol").expect("a symbol column");
    /// assert_eq!(rows[0].as_sequence().expect("a row")[at].as_str(), Some("AAPL"));
    /// # Ok(())
    /// # }
    /// ```
    pub fn format_messages<'codec, 'field, I>(
        &'codec self,
        messages: I,
        field: &'field Field,
    ) -> impl Iterator<Item = Result<Scalar>> + use<'codec, 'field, I>
    where
        I: IntoIterator,
        I::Item: Into<Result<FixMsg>>,
    {
        messages
            .into_iter()
            .fuse()
            .map(move |held| held.into()?.into_row(field))
    }

    /// A stream of batches of stable FIX rows as batches under one message field.
    ///
    /// The Arrow twin of [`Self::format_messages`], and the last stage of the
    /// pipeline a capture runs: text lines in Arrow batches, parsed, then
    /// formatted here into the columns a consumer reads. The schema is
    /// decided before the first row, from the source's carried columns and
    /// `field`, so a reader is written against it without a batch in hand.
    ///
    /// Two paths, and the source's own schema decides which. A source that
    /// already carries `field`'s columns is cast batch by batch through the
    /// crate's one [Arrow cast](crate::arrow::cast_reader) - column kernels,
    /// no row loop, one plan for the whole stream, and a value that will not
    /// convert nulled rather than refused - and a source that is already
    /// exactly `field` is handed back untouched. A source carrying the
    /// arrival record instead is read back as
    /// [messages](Self::messages) and re-filled, because that is the only way
    /// a column the row does not carry can be answered at all: the record is
    /// the message, and lifting out of it is what the record is for.
    ///
    /// Batches close on the raw bytes of each message's arrival record
    /// against [`Self::with_batch_byte_size`], exactly as
    /// [`Self::arrow_reader`] closes them.
    ///
    /// # Errors
    ///
    /// Returns the Arrow layer's refusal when `field` does not make an Arrow
    /// schema, and the cast's when the source's columns cannot be planned
    /// into it; the source reader's own failure is an error batch.
    pub fn format_arrow_reader(&self, source: BatchReader, field: &Field) -> Result<BatchReader> {
        let read = Self::row_field(source.schema().as_ref())?;
        // The capture's own columns lead the formatted row exactly as they
        // lead the parsed one: where a line was read from is what a monitor
        // orders and joins on, and a format is a reading of the message, not
        // a reason to lose the frame around it. This pass is one row out per
        // row in, in order, so each row's own cells are carried straight
        // across.
        let target = super::fix_schema_carrying(&read, field)?;
        // A source holding the record can answer every column of every
        // target, so it is read as messages and filled. One that does not is
        // a projection already, and a cast is what a projection needs.
        if read.index_of(FIXENTRIES_COLUMN).is_none() {
            return Ok(crate::arrow::cast_reader(
                source,
                &target,
                crate::ArrowCastOptions::new(),
            )?);
        }
        // The capture's own columns are the source row's statement about
        // its line, and this pass keeps the row it read: they are carried
        // over from the batch rather than asked of the message, which holds
        // none of them.
        let restating = capture_restating(&read, &target, self.registry())?;
        let rows = self.messages_restating(source, restating);
        self.arrow_reader_restating(target, rows)
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
    /// [`FIXENTRIES_COLUMN`](super::FIXENTRIES_COLUMN) cannot be written and says
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
        if field.index_of(FIXENTRIES_COLUMN).is_none() {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static(FIXENTRIES_COLUMN),
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

/// What a batch has taken so far, against the two bounds it closes on.
#[derive(Default)]
struct Carried {
    bytes: u64,
    rows: usize,
}

/// Whether the batch closes after a row charged `charge` bytes.
///
/// Two bounds, and the batch closes on whichever it reaches first: the bytes
/// keep a batch about the same size whatever shape arrived, and the rows keep
/// a batch of very small messages from holding millions of them before a
/// consumer sees one. Each running total crosses its target and starts again
/// from nothing, so the row that crosses it is the last of its batch, a byte
/// target of zero closes after every row, and a row target of zero leaves the
/// bytes to decide.
fn closes(carried: &mut Carried, charge: u64, target: u64, rows: usize) -> bool {
    carried.bytes = carried.bytes.saturating_add(charge);
    carried.rows = carried.rows.saturating_add(1);
    if carried.bytes >= target || (rows > 0 && carried.rows >= rows) {
        *carried = Carried::default();
        return true;
    }
    false
}

/// The raw bytes one message's entries are, walked without allocating: the
/// length of every name and value at every level, plus the per-row width.
fn wire_size(entries: &[FixEntry]) -> u64 {
    entries.iter().fold(ROW_OVERHEAD, |sum, entry| {
        sum + entry.name().len() as u64
            + entry.value().map_or(0, str::len) as u64
            + wire_size(entry.entries())
    })
}

/// Where a row schema's own capture columns sit.
///
/// [The capture's own columns](FixMsg::from_row): the two the crate tags,
/// `sourceurl` and `recordedat`, and every column no tag and no counter
/// names - the body a line was cut from, its place in the object, its media
/// type, what a bound dropped. A namespaced key is not one of them: that is
/// a bridge's own statement, which the message keeps in its metadata.
fn capture_columns(schema: &Field, plan: &super::schema::Columns) -> Vec<usize> {
    schema
        .fields()
        .iter()
        .zip(plan.iter())
        .enumerate()
        .filter(|(_, (column, planned))| match planned.tag {
            Some(tag) => super::identity::is_capture_tag(tag),
            None => {
                planned.counter.is_none()
                    && column.name() != FIXENTRIES_COLUMN
                    && !column.name().contains('.')
            }
        })
        .map(|(at, _)| at)
        .collect()
}

/// Each of `source`'s capture columns as the column it is read from beside
/// the column it is stated at in `target`, ascending by target.
///
/// The one reading both row-to-row doors take, so the cells a row stated
/// land where the rows they write hold them: a carried column leads the
/// target in the order it was kept, and a tagged one lands at the column
/// the target gives its tag. A column `target` has no place for is dropped,
/// because a row cannot state what it has no column for.
///
/// # Errors
///
/// Returns the schema grammar's refusal when `source` does not make a
/// column plan.
fn capture_restating(
    source: &Field,
    target: &Field,
    registry: &FixRegistry,
) -> Result<Vec<(usize, usize)>> {
    let plan = super::schema::column_plan(source, registry)?;
    let mut restating: Vec<(usize, usize)> = Vec::new();
    for at in capture_columns(source, &plan) {
        let column = &source.fields()[at];
        let placed = match plan[at].tag {
            Some(tag) => super::fix_column_of(target, tag),
            None => target
                .fields()
                .iter()
                .position(|held| crate::types::folds_equal(held.name(), column.name())),
        };
        if let Some(placed) = placed {
            restating.push((at, placed));
        }
    }
    // One column is stated once, by the leftmost source that reaches it.
    restating.sort_by_key(|(_, at)| *at);
    restating.dedup_by_key(|(_, at)| *at);
    Ok(restating)
}

/// Whether a column of `dtype` carries a payload the codec can read: text or
/// bytes in any layout, a dictionary or run-end encoding of one included.
fn carries_payload(dtype: &DataType) -> bool {
    match dtype {
        DataType::Dictionary(held) => carries_payload(&held.value),
        DataType::RunEndEncoded(held) => carries_payload(held.values.dtype()),
        other => matches!(
            other.kind(),
            DataTypeKind::Text | DataTypeKind::Bytes | DataTypeKind::Code
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
/// `restated` is the capture's own cells, each beside the column it is
/// stated at and in ascending order. Expanded messages repeat the same
/// source row's cells.
fn row_of(
    message: &FixMsg,
    schema: &Field,
    plan: &super::schema::Columns,
    restated: Vec<(usize, Scalar)>,
) -> Result<Scalar> {
    // Capture values land before the field contract runs.
    let held = message.row_values(schema, plan, restated)?;
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
    [payload, BEGINSTRING_COLUMN, DIRECTION_COLUMN]
        .iter()
        .any(|held| crate::types::folds_equal(held, name))
}

struct Columns {
    /// The column the frame is read from, proven there and readable.
    payload: usize,
    beginstring: Option<usize>,
    /// The column stating the row's direction: tag 385's own name.
    direction: Option<usize>,
    /// The columns whose names reach a field, each beside the field it fills.
    ///
    /// Resolved once from the schema and the dictionary: a column named after
    /// nothing the dictionary knows is never read per row for it, and the
    /// dictionary is never probed per row for one it does know.
    fills: Vec<(usize, Field, i32)>,
    /// The capture's own cells, each as the source column it is read from
    /// beside the target column it is stated at, in ascending target order.
    ///
    /// Both kinds at once: the carried columns, which lead the row in the
    /// order they were kept, and the two the crate tags - `sourceurl`,
    /// `recordedat` - wherever the fixed columns put them. No message holds
    /// any of them, so this is the whole of what says where a row's line
    /// came from and when it was written down.
    restated: Vec<(usize, usize)>,
    /// Each source column's datatype, so a cell is read under its own.
    dtypes: Vec<DataType>,
}

impl Columns {
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when `carrier` has no column named
    /// `payload`, or that column holds neither text nor bytes.
    fn resolve(
        carrier: &Field,
        payload: &str,
        kept: &[usize],
        codec: &FixCodec,
        target: &Field,
    ) -> Result<Self> {
        let fields = carrier.fields();
        let named = |wanted: &str| {
            fields
                .iter()
                .position(|held| crate::types::folds_equal(held.name(), wanted))
        };
        let payload_at = payload_column_of(carrier, payload, named(payload))?;
        let reached = |held: &Field| codec.fill_target(held.name()).map(|(_, tag)| tag);
        let fills = fields
            .iter()
            .enumerate()
            .filter(|(_, held)| !is_parameter(held.name(), payload))
            .filter_map(|(at, held)| {
                let (field, tag) = codec.fill_target(held.name())?;
                // The capture's own column fills no field: it is the
                // reader's statement about the line, and it is stated on
                // the row below rather than on the message.
                (!super::identity::is_capture_tag(tag)).then_some((at, field, tag))
            })
            .collect();
        // The carried columns lead the row, in the order they were kept; the
        // tagged ones land at the column the fixed schema gives their tag.
        let mut restated: Vec<(usize, usize)> = kept
            .iter()
            .enumerate()
            .map(|(at, source)| (*source, at))
            .collect();
        for (source, held) in fields.iter().enumerate() {
            // The payload column is the payload whatever it is called, as it
            // is for a fill: a codec reading its frames out of a column
            // named `sourceurl` states no source object.
            if is_parameter(held.name(), payload) {
                continue;
            }
            let Some(tag) = reached(held).filter(|tag| super::identity::is_capture_tag(*tag))
            else {
                continue;
            };
            if let Some(at) = super::fix_column_of(target, tag) {
                restated.push((source, at));
            }
        }
        // One column is stated once, by the leftmost source that reaches it:
        // `mtime` and `recordedat` both answer tag 65028, and a row cannot
        // hold one column twice. Stably, so which one wins is the schema's
        // order and never the sort's.
        restated.sort_by_key(|(_, at)| *at);
        restated.dedup_by_key(|(_, at)| *at);
        Ok(Self {
            payload: payload_at,
            beginstring: named(BEGINSTRING_COLUMN),
            direction: named(DIRECTION_COLUMN),
            fills,
            restated,
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
    /// One row of one batch as the messages it carries and the capture's
    /// own cells, each beside the column it is stated at.
    ///
    /// An ordinary line is one message; a bulk configuration document is one
    /// per configuration, and the row's own cells are stated on each of them.
    fn row(&self, batch: &RecordBatch, row: usize) -> Result<(FixMessages, Vec<(usize, Scalar)>)> {
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
        // The direction a row states outranks any reading of its line, and
        // the codec's pin fills what neither states.
        let direction = stated(self.columns.direction)?
            .and_then(|held| {
                held.as_str()
                    .and_then(|text| self.codec.msgdirection().code(text))
            })
            .map(SmolStr::new);
        // The cells that fill fields, read only where the row states them.
        let mut cells: Vec<(&Field, i32, Scalar)> = Vec::with_capacity(self.columns.fills.len());
        for (at, field, tag) in &self.columns.fills {
            let Some(value) = stated(Some(*at))? else {
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
            version: beginstring
                .as_ref()
                .and_then(Scalar::as_str)
                .and_then(version_of),
            fills: &fills,
            direction: direction.as_deref(),
            direction_pin: self.codec.direction(),
        };
        let messages = self.codec.parse_bytes_with(extras, &payload);
        // By position: which columns are the capture's and where each one
        // lands were decided from the schema, and a row of that schema
        // arrives in that order.
        let restated = self
            .columns
            .restated
            .iter()
            .map(|(source, at)| cell(*source).map(|value| (*at, value)))
            .collect::<Result<Vec<_>>>()?;
        Ok((messages, restated))
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
    type Item = Result<(FixMessages, Vec<(usize, Scalar)>, u64)>;

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
                        .map(|(messages, restated)| (messages, restated, charge)),
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

/// The messages a stream of batches of FIX rows holds, each beside the
/// capture's own cells of the row it came out of, read a batch at a time.
///
/// One batch is held as the Struct array it is and read row by row into the
/// row value the schema declares, which [`FixMsg::from_row`] makes a message
/// of. The schema is read once, off the source; a schema that does not make
/// a root is the one item the stream yields.
///
/// A message holds nothing about the reading it arrived through, so the
/// cells a row states for itself - where its line was read from, when it
/// was recorded, the body it was cut from, its place in the object - are
/// picked out of the row value that is already in hand and handed over
/// beside the message. Whoever writes the rows back states them again.
struct Messages {
    registry: Arc<FixRegistry>,
    source: BatchReader,
    schema: Field,
    /// Each of the capture's own columns as the source column it is read
    /// from beside the target column it is stated at, ascending by target.
    restating: Vec<(usize, usize)>,
    /// The refusal the source's schema earned, yielded once.
    pending: Option<Error>,
    /// The batch being read, beside the row the next pull reads.
    held: Option<(StructArray, usize)>,
    done: bool,
}

impl Messages {
    fn over(
        registry: Arc<FixRegistry>,
        source: BatchReader,
        restating: Vec<(usize, usize)>,
    ) -> Self {
        let (schema, pending) = match FixCodec::row_field(source.schema().as_ref()) {
            Ok(schema) => (schema, None),
            Err(error) => (DataType::Null.required_field(ROOT_NAME), Some(error)),
        };
        Self {
            registry,
            source,
            schema,
            restating,
            pending,
            held: None,
            done: false,
        }
    }

    /// The capture's own cells one row states, each at the column it is
    /// stated back at; nothing where this stream carries none.
    fn cells(&self, row: &Scalar) -> Vec<(usize, Scalar)> {
        if self.restating.is_empty() {
            return Vec::new();
        }
        let held = row.as_sequence().unwrap_or_default();
        self.restating
            .iter()
            .filter_map(|(source, at)| held.get(*source).map(|value| (*at, value.clone())))
            .collect()
    }
}

impl Iterator for Messages {
    type Item = Result<(FixMsg, Vec<(usize, Scalar)>)>;

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
                            let cells = self.cells(&row);
                            let message =
                                FixMsg::from_row(Arc::clone(&self.registry), &self.schema, &row)?;
                            Ok((message, cells))
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
