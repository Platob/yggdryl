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
//! | identity | `currunix`, `creaunix`, `currhashcode`, `crosshashcode`, `curruuid`, `crossuuid`, `snapunix`, `sendingtime` |
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
//! A batch is cut by a running total of bytes against the codec's
//! [`batch_byte_size`](super::FixCodec::with_batch_byte_size): the statistic
//! is what each row lands as - the leaves of every column it fills and a
//! per-row width - so a row carrying its arrival record, a lifted-only row
//! and a row of typed facts alone are each charged what they occupy. One
//! statistic for every door, because every door writes its rows through
//! [`arrow_reader`](super::FixCodec::arrow_reader): several small input
//! batches accumulate into one output batch, one input batch larger than
//! the target is split by rows in proportion, and a batch always holds at
//! least one row.

use std::borrow::Cow;
use std::sync::Arc;

use arrow_array::RecordBatch;

use smol_str::SmolStr;

use crate::arrow::BatchReader;
use crate::arrow::rows::{Closing, appended_bytes, canonical_closing_reader};
use crate::graph::EventColumn;
use crate::serie::{Proof, land_batch};
use crate::text::TextOptions;
use crate::{DataType, DataTypeKind, Error, Field, Result, Scalar, Serie, Utf8StringSerie};

use super::build::{BEGINSTRING_COLUMN, DIRECTION_COLUMN, version_of};
use super::build::{Fill, RowExtras};
use super::codec::{FixCodec, SOH, Spread};
use super::msg::FixMsg;
use super::{FIXENTRIES_COLUMN, FixMessages};

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
    /// [`Self::parse_arrow_messages`] into [`Self::arrow_reader`]: the rows
    /// parsed into the messages they carry, each carrying its row's own
    /// cells, and the messages written under the schema decided before the
    /// first row - the capture's own columns leading, the fixed FIX columns
    /// following ([`fix_schema_carrying`](super::fix_schema_carrying)), a
    /// capture column named as a FIX column yielding to it. Batches close as
    /// `arrow_reader` closes them, on the bytes each row lands as against
    /// [`Self::with_batch_byte_size`].
    ///
    /// One thread reads rows where they stand. Several threads hand one owned
    /// input batch to each worker, at most one batch per worker ahead, then
    /// flatten its rows in source order before this reader closes output.
    ///
    /// # Errors
    ///
    /// Returns [`Self::parse_arrow_messages`]'s refusals, and the Arrow
    /// layer's when the schema does not make an Arrow schema.
    pub fn parse_text_arrow_reader(&self, source: BatchReader) -> Result<BatchReader> {
        let read = super::fix_schema(self.registry(), ROOT_NAME)?;
        let carrier = Self::row_field(source.schema().as_ref())?;
        let field = super::fix_schema_carrying(&carrier, &read)?;
        let reader = self.row_reader(&carrier, &read)?;
        let schema = field.clone();
        // A row is parsed and every message it carries filled into its
        // fixed row on the one thread that was handed the row, so no
        // message crosses a thread between the two halves; the rows come
        // back in row order and the batches close on the thread that pulls
        // them.
        let rows = if self.threads() == 1 {
            Spread::Sequential(
                crate::parallel::ordered(
                    BatchRows::over(source, Arc::clone(&reader)),
                    1,
                    self.chunk(),
                    move |held: Result<(Arc<Landed>, usize)>| -> Vec<Result<Charged>> {
                        carried_messages(held.and_then(|(batch, row)| reader.row(&batch, row)))
                            .map(|message| charged(message, &schema))
                            .collect()
                    },
                )
                .flatten(),
            )
        } else {
            Spread::Threaded(
                crate::parallel::ordered(
                    CaptureBatches::over(source),
                    self.threads(),
                    1,
                    move |held| -> Vec<Result<Charged>> {
                        match held.and_then(|batch| reader.land(&batch)) {
                            Err(error) => vec![Err(error)],
                            Ok(batch) => {
                                let mut rows = Vec::with_capacity(batch.records.len());
                                for row in 0..batch.records.len() {
                                    rows.extend(
                                        carried_messages(reader.row(&batch, row))
                                            .map(|message| charged(message, &schema)),
                                    );
                                }
                                rows
                            }
                        }
                    },
                )
                .with_lane_depth(1)
                .flatten(),
            )
        };
        self.closing_reader(field, rows)
    }

    /// Parses a stream of Arrow batches of capture rows into the stream of
    /// messages the rows carry.
    ///
    /// The batch a text reader answers with is already the shape this
    /// wants: one row per line, the payload in the column
    /// [`Self::with_payload_column`] names and the capture's own columns
    /// beside it, so it is taken whole rather than through a row-at-a-time
    /// boundary. Each row is read cell by cell out of the arrays and parsed
    /// through the same funnel as a line: the payload as
    /// [`Self::parse_line`] reads it, the `beginstring` column as
    /// [`Self::parse_text_line`] reads the captures of those names, the
    /// `msgdirection` column as the direction the row states, the
    /// `currunix` column - the text reader's `mtime` - as when the row's
    /// line was written, which dates a message stating no `SendingTime(52)`
    /// as that door dates it, and every other column
    /// named after a field the dictionary knows, `msgpluginid` among them,
    /// filling that field where the line left it unsaid. Where each column
    /// sits and which field it fills is decided once from the schema, so no
    /// row copies the codec or asks the dictionary a question the row
    /// before it asked.
    ///
    /// [The capture's own columns](FixMsg::carried) - the carried ones, and
    /// the one the crate tags, `sourceurl` - fill nothing: each is read off
    /// the row and carried by every message parsed out of it, under the
    /// column's name, so whoever writes the messages back as rows states
    /// them again at their columns. Every message states the row's line,
    /// dated by its `currunix` cell, as its one source.
    ///
    /// A line the reader cannot classify yields no message; malformed-body
    /// recovery still obeys the mandatory field contract. A bulk
    /// configuration document is one message per configuration it named
    /// and none where it named none, each carrying its row's cells. Tag
    /// 385, the column [`MSGDIRECTION_TAG_NAME`](super::MSGDIRECTION_TAG_NAME)
    /// names, takes the row's stated `msgdirection`, else the reading over
    /// the prose in front of its payload, else [`Self::try_with_direction`]'s
    /// pin. A source batch of another schema than the first is a conflict
    /// item, because every cell is read by the position the declared schema
    /// gave it; the source reader's own failure is an error item.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the source's schema or the
    /// dictionary does not make a root field, and [`Error::InvalidRecord`]
    /// naming the payload column when the source has no column of that name
    /// or its column holds neither text nor bytes - a source that would
    /// parse nothing is refused before a row is read rather than answered
    /// as empty messages.
    ///
    /// One thread reads rows where they stand. Several threads hand one owned
    /// input batch to each worker, at most one batch per worker ahead, and
    /// flatten each worker's rows in source order.
    pub fn parse_arrow_messages(
        &self,
        source: BatchReader,
    ) -> Result<impl Iterator<Item = Result<FixMsg>> + Send + use<>> {
        let read = super::fix_schema(self.registry(), ROOT_NAME)?;
        let carrier = Self::row_field(source.schema().as_ref())?;
        let reader = self.row_reader(&carrier, &read)?;
        let threads = self.threads();
        if threads == 1 {
            let rows = BatchRows::over(source, Arc::clone(&reader));
            return Ok(Spread::Sequential(rows.flat_map(move |held| {
                carried_messages(held.and_then(|(batch, row)| reader.row(&batch, row)))
            })));
        }
        let rows = crate::parallel::ordered(
            CaptureBatches::over(source),
            threads,
            1,
            move |held| -> Vec<Result<FixMsg>> {
                match held.and_then(|batch| reader.land(&batch)) {
                    Err(error) => vec![Err(error)],
                    Ok(batch) => {
                        let mut messages = Vec::with_capacity(batch.records.len());
                        for row in 0..batch.records.len() {
                            messages.extend(carried_messages(reader.row(&batch, row)));
                        }
                        messages
                    }
                }
            },
        )
        .with_lane_depth(1)
        .flatten();
        // A bulk configuration document expands into one message per
        // configuration, each carrying its source row's own cells.
        Ok(Spread::Threaded(rows))
    }

    /// What every row of `carrier` is read through: where each column sits
    /// and which field it fills, decided once from the schema rather than
    /// per row.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming the payload column when
    /// `carrier` has no column of that name or its column holds neither
    /// text nor bytes.
    fn row_reader(&self, carrier: &Field, read: &Field) -> Result<Arc<RowReader>> {
        // Which of the capture's own columns survive the FIX columns' claim on
        // a name, and where every column a row is read from sits, are decided
        // once here, from the schema, rather than per row.
        let kept = super::schema::carried(carrier, read);
        let columns = Columns::resolve(carrier, self.payload_column(), &kept, self)?;
        Ok(Arc::new(RowReader {
            root: Arc::new(carrier.clone()),
            columns,
            codec: self.clone(),
            options: Arc::new(TextOptions::new()),
        }))
    }

    /// Walks a stream of batches of FIX rows as one lifecycle.
    ///
    /// [`Self::messages`] into [`Self::lifecycle`] into [`Self::arrow_reader`]
    /// under the schema read off the source: each row becomes the message it
    /// holds through [`FixMsg::from_row`] - any schema that constructor
    /// accepts, the fixed row carrying a capture's columns or not - the
    /// messages are walked as `lifecycle` walks them, and each is written
    /// back under the **same** schema. Projected columns and residual entries
    /// reconstruct the content without parsing a source line. [The capture's own
    /// columns](FixMsg::carried) travel with the message the walk moves, so
    /// the `body` a row was cut from and the `sourceurl` it names still
    /// belong to the message that came out of that line whatever order the
    /// walk answers in. A refused message type never enters the walk,
    /// exactly as in `lifecycle`, and takes its row with it. Batches close as
    /// `arrow_reader` closes them, on the bytes each row lands as.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the source's schema does not
    /// make a root field, and the Arrow layer's when it does not make an
    /// Arrow schema; a row that is not a FIX row and the source reader's own
    /// failure are error batches.
    pub fn lifecycle_arrow_reader(&self, source: BatchReader) -> Result<BatchReader> {
        let schema = Self::row_field(source.schema().as_ref())?;
        let walked = self.lifecycle(self.messages(source));
        self.arrow_reader(schema, walked)
    }

    /// A stream of batches of FIX rows as the market operations its
    /// messages are, in [`MarketData::field`](crate::graph::MarketData::field)
    /// rows.
    ///
    /// [`Self::messages`] into [`Self::market_arrow_reader`]: each row is read
    /// as its own message, its market facts derived from what the row
    /// states, and the capture is expanded, sorted and batched as that door
    /// does it - so a row that is not a FIX row, or a failure of the source
    /// reader, is the reader's only item. Over rows no walk wrote, it answers
    /// the leaves `market_arrow_reader` answers for their messages. A walked
    /// capture reaches the sorted door as messages -
    /// `market_arrow_reader(codec.lifecycle(messages))` - never as the rows
    /// [`Self::lifecycle_arrow_reader`] writes: what a walk settles from a
    /// message's predecessors - its `prevpx` and `prevqty`, a side or a
    /// ticker it carries forward, an execution instant - is no cell of the
    /// row, so a walked row read here is read without it.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the source's schema does not
    /// make a root field, before a row is read.
    pub fn market_operations_arrow_reader(&self, source: BatchReader) -> Result<BatchReader> {
        Self::row_field(source.schema().as_ref())?;
        self.market_arrow_reader(self.messages(source))
    }

    /// A stream of batches of FIX rows as the stream of messages it holds.
    ///
    /// Each row is one message through [`FixMsg::from_row`] under the
    /// source's schema, its entries rebuilt from projected columns and the
    /// [`FIXENTRIES_COLUMN`](super::FIXENTRIES_COLUMN) where the schema carries it.
    /// [The capture's own cells](FixMsg::carried) travel with the rebuilt message.
    /// Recorded event identities survive; emitted wire may reorder or normalize
    /// represented content. No source line is parsed again.
    /// One thread holds one batch at a time. Several threads retain bounded
    /// row chunks, which can span batches, and yield messages in source order.
    /// A source batch of another schema than the first is a conflict item,
    /// and a row the schema does not make a message of is an error item;
    /// either fuses the stream.
    ///
    /// The one half every door that reads rows composes with
    /// [`Self::arrow_reader`]: `lifecycle_arrow_reader` walks between the
    /// two, `format_arrow_reader` refills between them, and
    /// [`FixDedup`](super::FixDedup) filters between them the same way.
    pub fn messages(
        &self,
        source: BatchReader,
    ) -> impl Iterator<Item = Result<FixMsg>> + Send + use<> {
        // The schema is read once, off the source; a schema that does not
        // make a root is the one item the stream yields.
        let (schema, refused) = match Self::row_field(source.schema().as_ref()) {
            Ok(schema) => (schema, None),
            Err(error) => (DataType::Null.required_field(ROOT_NAME), Some(error)),
        };
        let registry = Arc::clone(self.registry());
        let root = Arc::new(schema.clone());
        let read = crate::parallel::ordered(
            StructRows::over(source, root, refused),
            self.threads(),
            self.chunk(),
            move |held: Result<(Arc<Serie>, usize)>| {
                let (records, row) = held?;
                let row = records.scalar(row)?;
                FixMsg::from_landed_row(Arc::clone(&registry), &schema, &row)
            },
        );
        // An error among the rows ends the stream where it stands, as it
        // does read one at a time.
        Fused::over(read)
    }

    /// A stream of messages as a stream of batches of FIX rows under `schema`.
    ///
    /// Each message fills one row through [`FixMsg::into_row`]: the schema's
    /// columns in its order, each by its tag or its group's counter, a column
    /// carrying neither by the child of its name. The other half of what the
    /// Arrow twins compose; [`fix_schema`](super::fix_schema) is the schema a
    /// parsed message fills whole, and a schema read off a batch by
    /// [`Self::messages`] is the one its messages return to. Batches close on
    /// the bytes each row lands as - the leaves of every column it fills and
    /// a per-row width - against [`Self::with_batch_byte_size`], whichever of
    /// it and [`Self::with_batch_row_size`] binds first. An error item yields
    /// the completed prefix, then the error, and fuses the reader.
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
        // Each message fills its row on some thread where the codec reads
        // on several, in the messages' order, and is charged there what the
        // row lands as; the batches then close on the rows as they come, on
        // the one thread that pulls them.
        let rows = crate::parallel::ordered(
            messages.into_iter().map(|held| held.into()),
            self.threads(),
            self.chunk(),
            move |held: Result<FixMsg>| charged(held, &schema),
        );
        self.closing_reader(root, rows)
    }

    /// The batches a stream of charged rows under `root` closes into: on
    /// the bytes each row lands as against [`Self::with_batch_byte_size`],
    /// whichever of it and [`Self::with_batch_row_size`] binds first. An
    /// error item yields the completed prefix, then the error, and fuses
    /// the reader.
    fn closing_reader<I>(&self, root: Field, rows: I) -> Result<BatchReader>
    where
        I: Iterator<Item = Result<Charged>> + Send + 'static,
    {
        let target = self.batch_byte_size();
        let row_target = self.batch_row_size();
        let mut carried = Carried::default();
        let rows = rows.map(move |row| match row {
            Err(error) => Closing(Err(error), false),
            Ok((charge, row)) => {
                let closes = closes(&mut carried, charge, target, row_target);
                Closing(Ok(row), closes)
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
    /// # use yggdryl::local::LocalFolder;
    /// # use yggdryl::{FixCodec, FixRegistry, fix_schema};
    /// # let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// # let registry = Arc::new(FixRegistry::from_handle(&LocalFolder::new(root)?)?);
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
    /// crate's one [Arrow cast](crate::SerieReader) - column kernels,
    /// no row loop, one plan for the whole stream, and a value that will not
    /// convert nulled rather than refused - and a source that is already
    /// exactly `field` is handed back untouched. A source carrying the
    /// arrival record instead is read back as
    /// [messages](Self::messages) and re-filled through
    /// [`Self::arrow_reader`], because that is the only way a column the row
    /// does not carry can be answered at all: the record is the message, and
    /// lifting out of it is what the record is for.
    ///
    /// Batches close as [`Self::arrow_reader`] closes them, on the bytes
    /// each row lands as against [`Self::with_batch_byte_size`].
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
            return Ok(crate::SerieReader::from_arrow_reader(
                Some(&target),
                source,
                crate::ArrowCastOptions::new(),
            )?
            .into_arrow_reader());
        }
        // The capture's own columns are the source row's statement about
        // its line, and this pass keeps the row it read: each message
        // carries them out of its row and states them again at its columns.
        self.arrow_reader(target, self.messages(source))
    }

    /// Writes a stream of batches of FIX rows back to the wire, streamed.
    ///
    /// The encode direction of the same exchange: each row is the message
    /// [`Self::messages`] reads out of it, and the line written is
    /// [`FixMsg::into_bytes`] with the separator [`Self::with_separator`]
    /// pinned, else [`SOH`], then a newline. The wire combines projected
    /// ordinary fields with residual entries; residual content owns any
    /// overlapping tag or group. Represented content may reorder or normalize.
    /// A batch without the
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
                    "a batch carrying its residual entries",
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

/// The payload column of one landed batch, narrowed once to where its bytes
/// are borrowed from.
///
/// Text and bytes in an offsets, view or fixed layout lend each row's run
/// where it lies, and a code its characters; a fixed-width string reads
/// through its field, which trims the padding, and so do the encodings - a
/// dictionary, a run-end - which hold no run of their own. A payload is read
/// by the codec and copied only into what the message keeps of it.
enum Payload {
    /// A text or byte storage leaf, proven one once.
    Stored(Serie),
    Code(Utf8StringSerie),
    Cell(Serie),
}

impl Payload {
    fn of(column: &Serie) -> Self {
        if matches!(column, Serie::FixedString(_)) {
            return Self::Cell(column.clone());
        }
        if column.is_string_storage() || column.is_byte_storage() {
            return Self::Stored(column.clone());
        }
        match column.as_utf8() {
            Some(code) => Self::Code(code.clone()),
            None => Self::Cell(column.clone()),
        }
    }

    /// The bytes one row carries, empty where it carries none.
    fn get(&self, row: usize) -> Result<Cow<'_, [u8]>> {
        Ok(match self {
            Self::Stored(held) => Cow::Borrowed(held.value_bytes(row).unwrap_or_default()),
            Self::Code(held) => Cow::Borrowed(held.value(row).map_or(&[][..], str::as_bytes)),
            Self::Cell(held) => {
                let value = held.scalar(row)?;
                value
                    .as_bytes()
                    .map(<[u8]>::to_vec)
                    .or_else(|| value.as_str().map(|text| text.as_bytes().to_vec()))
                    .map_or(Cow::Borrowed(&[][..]), Cow::Owned)
            }
        })
    }
}

/// One capture batch landed under its carrier, the rows it states proven
/// once, and its payload column narrowed once.
struct Landed {
    records: Serie,
    payload: Payload,
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
        .any(|held| crate::folds_equal(held, name))
}

struct Columns {
    /// The column the frame is read from, proven there and readable.
    payload: usize,
    beginstring: Option<usize>,
    /// The column stating the row's direction: tag 385's own name.
    direction: Option<usize>,
    /// The column stating when the row's line was written - the text
    /// reader's own `mtime` - which dates the line the messages state as
    /// their source.
    mtime: Option<usize>,
    /// The columns whose names reach a field, each beside the field it fills.
    ///
    /// Resolved once from the schema and the dictionary: a column named after
    /// nothing the dictionary knows is never read per row for it, and the
    /// dictionary is never probed per row for one it does know.
    fills: Vec<(usize, Field, i32)>,
    /// The capture's own cells, each as the source column it is read from
    /// beside the name it is carried under, in the order they lead the row.
    ///
    /// Both kinds at once: the carried columns, and the one the crate tags -
    /// `sourceurl`. Every message parsed out of a row carries them, and
    /// states each again at the column of its name.
    carried: Vec<(usize, SmolStr)>,
    /// The carrier's `curruuid`, where it carries the event columns: the
    /// identity of the line each row is, which every message the row
    /// answers for states as its one source.
    source: Option<usize>,
}

impl Columns {
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when `carrier` has no column named
    /// `payload`, or that column holds neither text nor bytes.
    fn resolve(carrier: &Field, payload: &str, kept: &[usize], codec: &FixCodec) -> Result<Self> {
        let fields = carrier.fields();
        let named = |wanted: &str| {
            fields
                .iter()
                .position(|held| crate::folds_equal(held.name(), wanted))
        };
        let payload_at = payload_column_of(carrier, payload, named(payload))?;
        let reached = |held: &Field| codec.fill_target(held.name()).map(|(_, tag)| tag);
        // A carrier's event columns are the carrier's own facts - the line
        // each row is, dated, identified and placed as the text reader
        // states it - and fill nothing on the message: its `curruuid` is
        // the message's source, and the rest say nothing about the message.
        let fills = fields
            .iter()
            .enumerate()
            .filter(|(_, held)| !is_parameter(held.name(), payload))
            .filter(|(_, held)| EventColumn::of_name(held.name()).is_none())
            .filter_map(|(at, held)| {
                let (field, tag) = codec.fill_target(held.name())?;
                // The capture's own column fills no field: it is the
                // reader's statement about the line, carried by the message
                // rather than stated on it.
                (!super::identity::is_capture_tag(tag)).then_some((at, field, tag))
            })
            .collect();
        let mut carried: Vec<(usize, SmolStr)> = kept
            .iter()
            .map(|source| (*source, SmolStr::new(fields[*source].name())))
            .collect();
        for (source, held) in fields.iter().enumerate() {
            // The payload column is the payload whatever it is called, as it
            // is for a fill: a codec reading its frames out of a column
            // named `sourceurl` states no source object.
            if is_parameter(held.name(), payload) || carried.iter().any(|(at, _)| *at == source) {
                continue;
            }
            if reached(held).is_some_and(super::identity::is_capture_tag) {
                carried.push((source, SmolStr::new(held.name())));
            }
        }
        Ok(Self {
            payload: payload_at,
            beginstring: named(BEGINSTRING_COLUMN),
            direction: named(DIRECTION_COLUMN),
            mtime: named(EventColumn::CurrUnix.name()),
            source: named(EventColumn::CurrUuid.name()),
            fills,
            carried,
        })
    }
}

/// One row's fixed row, charged what it lands as: the leaves of every
/// column and a per-row width, which is what the row costs the batch it
/// lands in, a typed fact and an entry alike, a lifted-only row and one
/// carrying its record alike.
type Charged = (u64, Scalar);

/// `message` filled into its row under `schema`, and charged.
fn charged(message: Result<FixMsg>, schema: &Field) -> Result<Charged> {
    let row = message?.into_row(schema)?;
    Ok((appended_bytes(&row), row))
}

/// The capture's own cells one row states, each under the column's name.
type Cells = Vec<(SmolStr, Scalar)>;

/// The messages one row answered, each carrying the row's own cells; a
/// row that refused is its refusal, once.
fn carried_messages(held: Result<(FixMessages, Cells)>) -> impl Iterator<Item = Result<FixMsg>> {
    let (messages, carried) = match held {
        Ok(held) => held,
        Err(error) => (FixMessages::from_result(Err(error)), Vec::new()),
    };
    messages.map(move |message| {
        message.map(|mut message| {
            message.set_carried(carried.clone());
            message
        })
    })
}

/// What every row of one carrier is read through, shared by every thread
/// the codec reads on.
///
/// A batch lands once under the carrier, which proves the rows its layout
/// does not - a code no registry holds is refused there, naming its row -
/// and a row is then read cell by cell through the columns' own leaves:
/// the payload as the bytes it is, a parameter column as the text it
/// holds, a carried column as the value it becomes. Nothing is copied that
/// the message does not keep, so a row costs its parse and the few cells
/// the row actually reads.
struct RowReader {
    /// The carrier every batch lands under, resolved once off the source.
    root: Arc<Field>,
    columns: Columns,
    codec: FixCodec,
    /// The options every row's line is read under, shared once: a row
    /// states its captures as columns, so the line reads no header of its own.
    options: Arc<TextOptions>,
}

impl RowReader {
    /// Land one batch under the carrier, narrowing its payload column once.
    fn land(&self, batch: &RecordBatch) -> Result<Landed> {
        let records = land_batch(&self.root, batch, &Proof::Unproven)?;
        let payload = Payload::of(&records.children()[self.columns.payload]);
        Ok(Landed { records, payload })
    }

    /// One row of one landed batch as the messages it carries and the
    /// capture's own cells, each under the column it was read from.
    ///
    /// An ordinary line is one message; a bulk configuration document is one
    /// per configuration, and the row's own cells are carried by each.
    fn row(&self, batch: &Landed, row: usize) -> Result<(FixMessages, Cells)> {
        let (columns, codec, options) = (&self.columns, &self.codec, &self.options);
        let cells = batch.records.children();
        let cell = |at: usize| cells[at].scalar(row);
        // A column absent, null or empty is silence.
        let stated = |at: Option<usize>| -> Result<Option<Scalar>> {
            at.map(cell)
                .transpose()
                .map(|held| held.filter(|value| !value.is_null()))
        };
        let payload = batch.payload.get(row)?;
        let beginstring = stated(columns.beginstring)?;
        // When the row's line was written, in the clock the line counts in:
        // the instant the line states as the event it is. The epoch is
        // silence, because a line nothing dated reads as the epoch and an
        // undated carrier must not date its messages.
        let mtime = stated(columns.mtime)?
            .and_then(|held| held.temporal_count_at(crate::TimeUnit::Nanosecond))
            .filter(|count| *count != 0);
        // The direction a row states outranks any reading of its line, and
        // the codec's pin fills what neither states.
        let direction = stated(columns.direction)?
            .and_then(|held| {
                held.as_str()
                    .and_then(|text| codec.msgdirection().code(text))
            })
            .map(SmolStr::new);
        // The cells that fill fields, read only where the row states them.
        let mut cells: Vec<(&Field, i32, Scalar)> = Vec::with_capacity(columns.fills.len());
        for (at, field, tag) in &columns.fills {
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
        // The line each row is, where the carrier states its identity: the
        // one source every message of the row states, exactly as the line
        // door states it for the same line.
        let source = stated(columns.source)?.and_then(|held| match held {
            Scalar::Uuid(uuid) => Some(uuid),
            _ => None,
        });
        let extras = RowExtras {
            version: beginstring
                .as_ref()
                .and_then(Scalar::as_str)
                .and_then(version_of),
            fills: &fills,
            direction: direction.as_deref(),
            direction_pin: codec.direction(),
            source,
            recdunix: None,
            originator: None,
            conversation: None,
        };
        let messages = codec.parse_row_with(extras, &payload, mtime, options);
        // The capture's own cells, read where the row states them: a null
        // cell is nothing carried, and the column it would land in answers
        // null without it.
        let mut carried: Cells = Vec::with_capacity(columns.carried.len());
        for (source, name) in &columns.carried {
            if let Some(value) = stated(Some(*source))? {
                carried.push((name.clone(), value));
            }
        }
        Ok((messages, carried))
    }
}

/// The capture batches one source declares, validated once before either its
/// row-at-a-time sequential reader or its whole-batch worker jobs consume it.
struct CaptureBatches {
    source: BatchReader,
}

impl CaptureBatches {
    const fn over(source: BatchReader) -> Self {
        Self { source }
    }
}

impl Iterator for CaptureBatches {
    type Item = Result<RecordBatch>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.source.next() {
            Some(Ok(batch)) if batch.schema() != self.source.schema() => {
                Some(Err(Error::conflict(
                    "the capture reader's declared Arrow schema",
                    "a different batch schema",
                    "FIX capture",
                )))
            }
            Some(Ok(batch)) => Some(Ok(batch)),
            Some(Err(error)) => Some(Err(crate::arrow::from_reader_error(error).into())),
            None => None,
        }
    }
}

/// The rows of a stream of validated capture batches, each beside the batch
/// it is a row of, landed once, in row order.
///
/// Every cell is read by the position the declared schema gave it, so a
/// batch of another schema than the first is a conflict item; a batch whose
/// rows the carrier refuses at the landing, and the source reader's own
/// failure, are error items; the stream goes on past each to the next
/// batch. The puller holds one batch, and a row keeps its own alive until it
/// is read.
struct BatchRows {
    source: CaptureBatches,
    reader: Arc<RowReader>,
    /// The batch being read, and the row the next pull reads.
    held: Option<(Arc<Landed>, usize)>,
}

impl BatchRows {
    const fn over(source: BatchReader, reader: Arc<RowReader>) -> Self {
        Self {
            source: CaptureBatches::over(source),
            reader,
            held: None,
        }
    }
}

impl Iterator for BatchRows {
    type Item = Result<(Arc<Landed>, usize)>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((batch, at)) = &mut self.held {
                if *at < batch.records.len() {
                    let row = *at;
                    *at += 1;
                    return Some(Ok((Arc::clone(batch), row)));
                }
            }
            // The batch is spent, or none is held yet: the next validated one
            // is pulled, landed, and the spent one dropped.
            match self.source.next().map(|batch| self.reader.land(&batch?)) {
                Some(Ok(batch)) => self.held = Some((Arc::new(batch), 0)),
                Some(Err(error)) => {
                    self.held = None;
                    return Some(Err(error));
                }
                None => return None,
            }
        }
    }
}

/// The rows of a stream of batches of FIX rows, each beside the Struct its
/// batch is, in row order, ended by the first refusal: the schema's own, a
/// batch of another schema than the first, or the source reader's failure.
struct StructRows {
    source: BatchReader,
    /// The root every batch lands under, resolved once off the source.
    root: Arc<Field>,
    /// The refusal the source's schema earned, yielded once and first.
    refused: Option<Error>,
    /// The record column being read, beside the row the next pull reads.
    held: Option<(Arc<Serie>, usize)>,
    done: bool,
}

impl StructRows {
    const fn over(source: BatchReader, root: Arc<Field>, refused: Option<Error>) -> Self {
        Self {
            source,
            root,
            refused,
            held: None,
            done: false,
        }
    }
}

impl Iterator for StructRows {
    type Item = Result<(Arc<Serie>, usize)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if let Some(error) = self.refused.take() {
            self.done = true;
            return Some(Err(error));
        }
        loop {
            if let Some((batch, at)) = &mut self.held {
                if *at < batch.len() {
                    let row = *at;
                    *at += 1;
                    return Some(Ok((Arc::clone(batch), row)));
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
                // Each batch lands once, proving the rows its schema's
                // layout does not: a row it refuses is named here.
                Some(Ok(batch)) => match land_batch(&self.root, &batch, &Proof::Unproven) {
                    Ok(records) => self.held = Some((Arc::new(records), 0)),
                    Err(error) => {
                        self.done = true;
                        return Some(Err(error.into()));
                    }
                },
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

/// A stream ended by its first error: the error is yielded, and nothing
/// after it - the stream underneath is dropped there, its threads with it.
struct Fused<I> {
    inner: Option<I>,
}

impl<I> Fused<I> {
    const fn over(inner: I) -> Self {
        Self { inner: Some(inner) }
    }
}

impl<I, T> Iterator for Fused<I>
where
    I: Iterator<Item = Result<T>>,
{
    type Item = Result<T>;

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.inner.as_mut()?.next();
        if next.as_ref().is_none_or(Result::is_err) {
            self.inner = None;
        }
        next
    }
}

impl<I, T> std::iter::FusedIterator for Fused<I> where I: Iterator<Item = Result<T>> {}
