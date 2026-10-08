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
//! | residual | `fixentries`, a sorted `map<utf8, utf8>` keyed by each field's `tag:name` |
//!
//! `fixentries` is not optional. The facet columns are a convenience over a
//! subset; the columns and the residual map together *are* the message, and
//! they are what makes a batch a lossless capture rather than one reader's
//! summary of it. A caller wanting facets alone projects the batch
//! afterwards, which already exists. A key no dictionary resolved is no
//! field: `metadata` holds it, or `fixentries` under `0:<key>` where an
//! identifier map holds it with its value.
//!
//! A column per tag seen is deliberately not the shape: it makes the schema
//! depend on the data, gives a mixed capture a thousand mostly-null columns,
//! and is reconstructible from `fixentries` by whoever actually wants it.
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
use arrow_schema::ArrowError;

use smallvec::SmallVec;
use smol_str::SmolStr;

use crate::arrow::BatchReader;
use crate::arrow::rows::{Closing, canonical_closing_reader};
use crate::arrow::scalar_memory_size;
use crate::graph::{ElementColumn, EventColumn};
use crate::logging::warning::warned;
use crate::serie::{Proof, Resolved, land_batch};
use crate::text::TextOptions;
use crate::{
    DataType, DataTypeKind, Error, Field, Result, Scalar, Serie, StreamChunkedSerie,
    Utf8StringSerie,
};

use super::build::{BEGINSTRING_COLUMN, DIRECTION_COLUMN, version_of};
use super::build::{Fill, RowExtras};
use super::codec::{FixCodec, Placed, SOH, Spread};
use super::messages::source_failure;
use super::msg::FixMsg;
use super::{FIXENTRIES_COLUMN, FixMessages};
use crate::graph::element::InstantSequence;

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
    /// [`Self::with_batch_byte_size`]. A row the stream cannot read, or a
    /// message its row will not hold, is excluded with a deduplicated
    /// warning; the source reader's own failure ends the reader after the
    /// completed prefix.
    ///
    /// One thread reads rows where they stand. Several threads hand each
    /// worker one job, at most one job per worker ahead - an input batch, or
    /// one of the row ranges a batch past twice
    /// [`Self::PARALLEL_JOB_ROWS`] rows is cut into - then flatten its rows
    /// in source order before this reader closes output.
    /// A message's [place](crate::graph::Event::get_seqnum) among the
    /// messages of its instant is known only once the rows before it are
    /// read, so a worker places and fills every row past its job's first
    /// instant, and the messages of that first instant - which may continue
    /// the run the job before ended on - are placed and filled where the
    /// jobs meet.
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
        if self.threads() == 1 {
            let rows = BatchRows::over(source, Arc::clone(&reader));
            let messages = Placed::over(rows.flat_map(move |held| {
                carried_messages(held.and_then(|(batch, row)| reader.row(&batch, row)))
            }));
            return self.closing_reader(
                field,
                messages.filter_map(move |message| charged(message, &schema)),
            );
        }
        // A row is parsed and every message it carries filled into its
        // fixed row on the one thread that was handed the row, so no
        // message crosses a thread between the two halves - but the messages
        // of a job's first instant, which cross once to be placed.
        let worker = schema.clone();
        let batches = crate::parallel::ordered(
            SourceJobs::over(source, self.threads()),
            self.threads(),
            1,
            move |held| -> SeamedBatch {
                match held.and_then(|(batch, start)| reader.land(batch, start)) {
                    Err(error) => SeamedBatch::of(std::iter::once(Err(error)), &worker),
                    Ok(landed) => SeamedBatch::of(
                        landed.iter().flat_map(|batch| {
                            (0..batch.records.len())
                                .flat_map(|row| carried_messages(reader.row(batch, row)))
                        }),
                        &worker,
                    ),
                }
            },
        )
        .with_lane_depth(1);
        self.closing_reader(field, Seams::over(batches, schema))
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
    /// pin.
    ///
    /// Nothing a row holds ends the stream. A source batch of another schema
    /// than the first is excluded with a warning, because every cell is read
    /// by the position the declared schema gave it; a row whose cells the
    /// carrier refuses - a code no registry holds - is excluded with a
    /// warning naming it, the rest of its batch read; a message that does not
    /// build is excluded the same way. The source reader's own failure is
    /// yielded after the messages before it and ends the stream.
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
    /// One thread reads rows where they stand. Several threads hand each
    /// worker one job, at most one job per worker ahead - an input batch, or
    /// one of the row ranges a batch past twice
    /// [`Self::PARALLEL_JOB_ROWS`] rows is cut into - and flatten each
    /// worker's rows in source order.
    pub fn parse_arrow_messages(
        &self,
        source: BatchReader,
    ) -> Result<impl Iterator<Item = Result<FixMsg>> + Send + use<>> {
        let read = super::fix_schema(self.registry(), ROOT_NAME)?;
        let carrier = Self::row_field(source.schema().as_ref())?;
        let reader = self.row_reader(&carrier, &read)?;
        Ok(self.carried_arrow_messages(source, reader))
    }

    /// The messages every row of `source` carries, read through `reader`
    /// and placed among the messages of their instants in row order.
    fn carried_arrow_messages(
        &self,
        source: BatchReader,
        reader: Arc<RowReader>,
    ) -> impl Iterator<Item = Result<FixMsg>> + Send + use<> {
        let threads = self.threads();
        // A bulk configuration document expands into one message per
        // configuration, each carrying its source row's own cells.
        if threads == 1 {
            let rows = BatchRows::over(source, Arc::clone(&reader));
            return Placed::over(Spread::Sequential(rows.flat_map(move |held| {
                carried_messages(held.and_then(|(batch, row)| reader.row(&batch, row)))
            })));
        }
        let rows = crate::parallel::ordered(
            SourceJobs::over(source, threads),
            threads,
            1,
            move |held| -> Vec<Result<FixMsg>> {
                match held.and_then(|(batch, start)| reader.land(batch, start)) {
                    Err(error) => vec![Err(error)],
                    Ok(landed) => {
                        let rows = landed.iter().map(|batch| batch.records.len()).sum();
                        let mut messages = Vec::with_capacity(rows);
                        for batch in &landed {
                            for row in 0..batch.records.len() {
                                messages.extend(carried_messages(reader.row(batch, row)));
                            }
                        }
                        messages
                    }
                }
            },
        )
        .with_lane_depth(1)
        .flatten();
        Placed::over(Spread::Threaded(rows))
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
            root: Resolved::of(Arc::new(carrier.clone())),
            columns,
            codec: self.reading(),
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
    /// walk answers in. The rows are read as already cleaned, as
    /// `lifecycle` reads what it is handed: a message type the parse that
    /// wrote the table refused never reached a row, and one it admitted is
    /// walked. Batches close as `arrow_reader` closes them, on the bytes
    /// each row lands as.
    ///
    /// A row that is not a message is excluded with a deduplicated warning
    /// and the walk goes on without it; the source reader's own failure is
    /// the reader's error once every row read before it is walked and
    /// written.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the source's schema does not
    /// make a root field, and the Arrow layer's when it does not make an
    /// Arrow schema; a schema that makes no FIX row is the reader's error
    /// batch.
    ///
    /// The root keeps the source's metadata but its `SORT:by`: the walk
    /// answers in its own order and may date a message again, so an order
    /// the source declared is no longer proven of the rows written.
    pub fn lifecycle_arrow_reader(&self, source: BatchReader) -> Result<BatchReader> {
        let mut schema = Self::row_field(source.schema().as_ref())?;
        schema.as_sort_mut().remove_by();
        let walked = self.lifecycle(self.messages(source));
        self.arrow_reader(schema, walked)
    }

    /// A stream of batches of FIX rows as the market data its
    /// messages are, in [`MarketData::field`](crate::graph::MarketData::field)
    /// rows.
    ///
    /// [`Self::messages`] into [`Self::market_arrow_reader`]: each row is read
    /// as its own message, its market facts derived from what the row
    /// states, and the capture is expanded, sorted and batched as that door
    /// does it - a row that is not a message excluded with a warning, and a
    /// schema that makes no FIX row, or a failure of the source reader, the
    /// one error `messages` hands it. Over rows no walk wrote, it answers
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
    pub fn market_data_arrow_reader(&self, source: BatchReader) -> Result<BatchReader> {
        Self::row_field(source.schema().as_ref())?;
        self.market_arrow_reader(self.messages(source))
    }

    /// A stream of batches of FIX rows as the stream of messages it holds.
    ///
    /// Each row is one message through [`FixMsg::from_row`] under the
    /// source's schema, its entries rebuilt from projected columns and the
    /// [`FIXENTRIES_COLUMN`](super::FIXENTRIES_COLUMN) where the schema carries it.
    /// [The capture's own cells](FixMsg::carried) travel with the rebuilt message.
    /// Its `msgpluginside` is the row's own cell where the schema holds the
    /// column, and the role of [the source this codec reads
    /// under](Self::with_source) - `UKNW` under none - where it does not.
    /// Recorded event identities survive; emitted wire may reorder or normalize
    /// represented content. No source line is parsed again.
    /// One thread holds one batch at a time. Several threads retain bounded
    /// row chunks, which can span batches, and yield messages in source order.
    ///
    /// A schema that makes no FIX row is the one item the stream yields,
    /// before a row is read. Past it, nothing a row holds ends the stream: a
    /// source batch of another schema than the first is excluded with a
    /// warning, a row whose cells the schema refuses - a code no registry
    /// holds - is excluded with a warning naming it and the rest of its
    /// batch read, and a row that does not rebuild into a message is excluded
    /// the same way. The source reader's own failure is yielded after the
    /// messages before it and ends the stream.
    ///
    /// The one half every door that reads rows composes with
    /// [`Self::arrow_reader`]: `lifecycle_arrow_reader` walks between the
    /// two, `format_arrow_reader` refills between them, and
    /// [`FixDedup`](super::FixDedup) filters between them the same way.
    pub fn messages(
        &self,
        source: BatchReader,
    ) -> impl Iterator<Item = Result<FixMsg>> + Send + use<> {
        // The schema is read and planned once, off the source: a schema that
        // makes no FIX row is the caller's, and refused before a row.
        let registry = Arc::clone(self.registry());
        let planned = Self::row_field(source.schema().as_ref()).and_then(|schema| {
            super::schema::column_plan_of(&schema, &registry)?;
            Ok(schema)
        });
        let (schema, refused) = match planned {
            Ok(schema) => (schema, None),
            Err(error) => (DataType::Null.required_field(ROOT_NAME), Some(error)),
        };
        let root = Resolved::of(Arc::new(schema.clone()));
        let pluginside = self.pluginside();
        let read = crate::parallel::ordered(
            StructRows::over(source, root, refused.is_some()),
            self.threads(),
            self.chunk(),
            move |held: Result<(Arc<Serie>, usize)>| {
                let message = held.and_then(|(records, row)| {
                    FixMsg::from_landed_row(
                        Arc::clone(&registry),
                        &schema,
                        &records.scalar(row)?,
                        pluginside,
                    )
                });
                match message {
                    Ok(message) => Some(Ok(message)),
                    Err(error) => source_failure(
                        error,
                        "FIX row excluded: it does not rebuild into a message",
                    )
                    .map(Err),
                }
            },
        )
        .flatten();
        // The reader's own failure is the one error the rows yield, and it
        // ends them: nothing follows it.
        refused.map(Err).into_iter().chain(read)
    }

    /// A stream of messages as a stream of batches of FIX rows under `schema`.
    ///
    /// Each message fills one row through [`FixMsg::into_row`]: the schema's
    /// columns in its order, each by its tag or, for a group, by the counter
    /// tag that names it, a column carrying neither by the child of its name. The other half of what the
    /// Arrow twins compose; [`fix_schema`](super::fix_schema) is the schema a
    /// parsed message fills whole, and a schema read off a batch by
    /// [`Self::messages`] is the one its messages return to. Batches close on
    /// the bytes each row lands as - the leaves of every column it fills and
    /// a per-row width - against [`Self::with_batch_byte_size`], whichever of
    /// it and [`Self::with_batch_row_size`] binds first. A message the row
    /// will not hold - a required column it leaves empty - and an item the
    /// stream refused are excluded with a deduplicated warning; a source
    /// failure yields the completed prefix, then the error, and fuses the
    /// reader. Owned messages and their fallible counterparts are accepted
    /// directly.
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
        )
        .flatten();
        self.closing_reader(root, rows)
    }

    /// The batches a stream of charged rows under `root` closes into: on
    /// the bytes each row lands as against [`Self::with_batch_byte_size`],
    /// whichever of it and [`Self::with_batch_row_size`] binds first. The
    /// one error that reaches it is a source failure, which yields the
    /// completed prefix, then the error, and fuses the reader.
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
    /// once here and never per message. A message a column that cannot be
    /// null will not hold, and an item the stream refused, are excluded with
    /// a deduplicated warning; the source's own failure moves through in its
    /// place, after the rows before it.
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
        messages.into_iter().fuse().filter_map(move |held| {
            match held.into().and_then(|message| message.into_row(field)) {
                Ok(row) => Some(Ok(row)),
                Err(error) => source_failure(error, ROW_REFUSED).map(Err),
            }
        })
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
    /// crate's one [Arrow cast](crate::StreamChunkedSerie) - column kernels,
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
            return Ok(crate::StreamChunkedSerie::from_arrow_reader(
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
    /// A key no dictionary resolved is restored from the row's `metadata`,
    /// or from `fixentries` under `0:<key>` where an identifier map holds
    /// it, as the tag-zero entry a parse holds and is written as it arrived;
    /// a bridge's namespaced key is the message's metadata and is not.
    /// A batch without the
    /// [`FIXENTRIES_COLUMN`](super::FIXENTRIES_COLUMN) cannot be written and says
    /// so before a row is read. A row in is a line out - a row whose message
    /// held no pairs is an empty line, and a row that is not a message is
    /// excluded with a deduplicated warning, as [`Self::messages`] excludes
    /// it - and the count of lines is answered.
    ///
    /// One batch is pulled, its rows written, and it is dropped. The source is
    /// never concatenated and no output buffer bigger than a row is held.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the source has no entries column,
    /// the schema's refusal to make a FIX row, the source reader's own
    /// failure once the rows before it are written, or the sink's write
    /// failure.
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

/// The serie faces: each Arrow door's input taken as any [`Serie`] and
/// its output answered as a [`StreamChunkedSerie`], so a capture read with
/// [`read_serie`](crate::IOMedia::read_serie) reaches a table's
/// [`append_serie`](crate::IOMedia::append_serie) without leaving the serie
/// vocabulary.
///
/// Each is a redirect and moves no row. A source crosses in as the stream of
/// the batches it already is ([`StreamChunkedSerie::from_serie`] then
/// [`StreamChunkedSerie::into_arrow_reader`]): transport, so a reader another face
/// answered hands its door's own reader back. The answer is read under the
/// root its door writes, which is the reader's own schema, so its one plan
/// is the identity: handed on as a source it is the door's reader again,
/// untouched, and only a caller reading its records lands a batch - sharing
/// its buffers, and reading once each row of a leaf whose layout is not its
/// datatype's whole contract, because the batch crossed a reader.
impl FixCodec {
    /// [`Self::parse_text_arrow_reader`] over any [`Serie`], answered
    /// as a [`StreamChunkedSerie`] under the root that door writes - the capture's
    /// own columns leading, the fixed FIX columns following - which declares
    /// no order.
    ///
    /// # Errors
    ///
    /// Returns [`StreamChunkedSerie::from_serie`]'s refusal of a run or of a
    /// record column holding an absent row, then
    /// [`Self::parse_text_arrow_reader`]'s.
    pub fn parse_text_serie(&self, source: impl Into<Serie>) -> Result<StreamChunkedSerie> {
        let parsed = self.parse_text_arrow_reader(transported(source)?)?;
        serie_of(&Self::row_field(parsed.schema().as_ref())?, parsed)
    }

    /// [`Self::lifecycle_arrow_reader`] over any [`Serie`], answered
    /// as a [`StreamChunkedSerie`] under the source's root, its `SORT:by` removed as
    /// that door removes it.
    ///
    /// # Errors
    ///
    /// Returns [`StreamChunkedSerie::from_serie`]'s refusal of a run or of a
    /// record column holding an absent row, then
    /// [`Self::lifecycle_arrow_reader`]'s.
    pub fn lifecycle_serie(&self, source: impl Into<Serie>) -> Result<StreamChunkedSerie> {
        let walked = self.lifecycle_arrow_reader(transported(source)?)?;
        serie_of(&Self::row_field(walked.schema().as_ref())?, walked)
    }

    /// [`Self::market_data_arrow_reader`] over any [`Serie`], answered
    /// as a [`StreamChunkedSerie`] of [`MarketData::field`](crate::graph::MarketData::field)
    /// rows.
    ///
    /// # Errors
    ///
    /// Returns [`StreamChunkedSerie::from_serie`]'s refusal of a run or of a
    /// record column holding an absent row, then
    /// [`Self::market_data_arrow_reader`]'s, and the row field's when it
    /// cannot be built.
    pub fn market_data_serie(&self, source: impl Into<Serie>) -> Result<StreamChunkedSerie> {
        let market = self.market_data_arrow_reader(transported(source)?)?;
        serie_of(&crate::graph::MarketData::field()?, market)
    }

    /// [`Self::messages`] over any [`Serie`]: the messages its FIX rows
    /// hold, so a table read back feeds [`Self::lifecycle`] and the book
    /// doors directly.
    ///
    /// # Errors
    ///
    /// Returns [`StreamChunkedSerie::from_serie`]'s refusal of a run or of a
    /// record column holding an absent row; past it, every refusal is an
    /// item of the stream, as in [`Self::messages`].
    pub fn messages_serie<S>(
        &self,
        source: S,
    ) -> Result<impl Iterator<Item = Result<FixMsg>> + Send + use<S>>
    where
        S: Into<Serie>,
    {
        Ok(self.messages(transported(source)?))
    }

    /// [`Self::arrow_reader`] answered as a [`StreamChunkedSerie`] under `schema` as
    /// a record root ([`StreamChunkedSerie::root_of`]); a `SORT:by` it declares is
    /// the caller's, and verified as the records land.
    ///
    /// # Errors
    ///
    /// Returns [`Self::arrow_reader`]'s refusal, and an error when `schema`
    /// does not make a bounded record root.
    pub fn chunked_stream<I>(&self, schema: Field, messages: I) -> Result<StreamChunkedSerie>
    where
        I: IntoIterator,
        I::Item: Into<Result<FixMsg>>,
        I::IntoIter: Send + 'static,
    {
        let root = StreamChunkedSerie::root_of(&schema)?;
        serie_of(&root, self.arrow_reader(schema, messages)?)
    }
}

/// `source` as the stream of the batches it already is: transport, nothing
/// cast, copied or read.
fn transported(source: impl Into<Serie>) -> Result<BatchReader> {
    Ok(StreamChunkedSerie::from_serie(source.into())?.into_arrow_reader())
}

/// A door's `reader` read as the records it is under `root`, the field the
/// door wrote it under: one identity plan, compiled once from the schema.
pub(super) fn serie_of(root: &Field, reader: BatchReader) -> Result<StreamChunkedSerie> {
    Ok(StreamChunkedSerie::from_arrow_reader(
        Some(root),
        reader,
        crate::ArrowCastOptions::new(),
    )?)
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
        // A carrier's element and event columns are the carrier's own facts
        // - the line each row is, identified, dated and placed as the text
        // reader states it - and fill nothing on the message: its
        // `curruuid` is the message's source, and the rest say nothing
        // about the message.
        let fills = fields
            .iter()
            .enumerate()
            .filter(|(_, held)| !is_parameter(held.name(), payload))
            .filter(|(_, held)| {
                ElementColumn::of_name(held.name()).is_none()
                    && EventColumn::of_name(held.name()).is_none()
            })
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
            source: named(ElementColumn::CurrUuid.name()),
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

/// What a door filling rows warns about a message it excludes.
const ROW_REFUSED: &str = "FIX row excluded: its message was not read or does not fill the row";

/// `message` filled into its row under `schema`, and charged: none for a
/// message the row will not hold, or an item the stream refused, excluded
/// with a warning; a source failure as itself.
fn charged(message: Result<FixMsg>, schema: &Field) -> Option<Result<Charged>> {
    match message.and_then(|message| message.into_row(schema)) {
        Ok(row) => Some(Ok((scalar_memory_size(&row) as u64, row))),
        Err(error) => source_failure(error, ROW_REFUSED).map(Err),
    }
}

/// One capture job's messages as a worker read them - a batch's, or a row
/// range's of one: those of the job's first instant still to place, because
/// the run the job before ended on may continue into them, then every row
/// past that instant placed and filled here, and the run the job ends on.
struct SeamedBatch {
    head: Vec<Result<FixMsg>>,
    rest: Vec<Result<Charged>>,
    /// Where the batch's last run stands, once its rows were placed here.
    tail: Option<InstantSequence>,
}

impl SeamedBatch {
    fn of(messages: impl Iterator<Item = Result<FixMsg>>, schema: &Field) -> Self {
        let mut head = Vec::new();
        let mut rest = Vec::new();
        let mut first = None;
        let mut tail: Option<InstantSequence> = None;
        for message in messages {
            let placing = match (&tail, &message) {
                (Some(_), _) => true,
                (None, Ok(held)) => {
                    let unix = crate::graph::Event::get_currunix(held);
                    let opens = first.is_some_and(|first| first != unix);
                    first = Some(unix);
                    opens
                }
                (None, Err(_)) => false,
            };
            if !placing {
                head.push(message);
                continue;
            }
            let sequence = tail.get_or_insert_with(InstantSequence::default);
            rest.extend(charged(
                message.map(|mut held| {
                    sequence.place_naming_sources(&mut held);
                    held
                }),
                schema,
            ));
        }
        Self { head, rest, tail }
    }
}

/// The rows of seamed jobs in source order: each job's first instant
/// placed after the run the job before ended on, and filled here, then
/// the rows its worker filled.
struct Seams<I> {
    batches: I,
    schema: Field,
    sequence: InstantSequence,
    head: std::vec::IntoIter<Result<FixMsg>>,
    rest: std::vec::IntoIter<Result<Charged>>,
    tail: Option<InstantSequence>,
}

impl<I> Seams<I> {
    fn over(batches: I, schema: Field) -> Self {
        Self {
            batches,
            schema,
            sequence: InstantSequence::default(),
            head: Vec::new().into_iter(),
            rest: Vec::new().into_iter(),
            tail: None,
        }
    }
}

impl<I: Iterator<Item = SeamedBatch>> Iterator for Seams<I> {
    type Item = Result<Charged>;

    fn next(&mut self) -> Option<Result<Charged>> {
        loop {
            if let Some(message) = self.head.next() {
                let sequence = &mut self.sequence;
                let row = charged(
                    message.map(|mut held| {
                        sequence.place_naming_sources(&mut held);
                        held
                    }),
                    &self.schema,
                );
                if row.is_some() {
                    return row;
                }
                continue;
            }
            // The batch's own runs past its first instant stand where its
            // worker left them.
            if let Some(tail) = self.tail.take() {
                self.sequence = tail;
            }
            if let Some(row) = self.rest.next() {
                return Some(row);
            }
            let batch = self.batches.next()?;
            self.head = batch.head.into_iter();
            self.rest = batch.rest.into_iter();
            self.tail = batch.tail;
        }
    }
}

/// The capture's own cells one row states, each under the column's name.
type Cells = Vec<(SmolStr, Scalar)>;

/// The messages one row answered, each carrying the row's own cells; a
/// row that refused is its refusal, once - passed over with a warning, or
/// the source's failure.
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
/// does not - a code no registry holds is refused there, and its row
/// excluded with a warning naming it ([`landed`]) - and a row is then read
/// cell by cell through the columns' own leaves:
/// the payload as the bytes it is, a parameter column as the text it
/// holds, a carried column as the value it becomes. Nothing is copied that
/// the message does not keep, so a row costs its parse and the few cells
/// the row actually reads.
struct RowReader {
    /// The carrier every batch lands under, resolved once off the source.
    root: Resolved,
    columns: Columns,
    codec: FixCodec,
    /// The options every row's line is read under, shared once: a row
    /// states its captures as columns, so the line reads no header of its own.
    options: Arc<TextOptions>,
}

impl RowReader {
    /// Land one batch - or the rows of one from its row `start` - under the
    /// carrier, whole or the rows the landing accepts, narrowing each landed
    /// column's payload once.
    fn land(&self, batch: RecordBatch, start: usize) -> Result<SmallVec<[Landed; 1]>> {
        Ok(landed(&self.root, batch, start)?
            .into_iter()
            .map(|records| {
                let payload = Payload::of(&records.children()[self.columns.payload]);
                Landed { records, payload }
            })
            .collect())
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
        // The cells that fill fields, read only where the row states them,
        // on the stack while the carrier fills at most eight fields.
        let mut cells: SmallVec<[(&Field, i32, Scalar); 8]> =
            SmallVec::with_capacity(columns.fills.len());
        for (at, field, tag) in &columns.fills {
            let Some(value) = stated(Some(*at))? else {
                continue;
            };
            cells.push((field, *tag, value));
        }
        let fills: SmallVec<[Fill<'_>; 8]> = cells
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

/// The batches one source reader yields, each of the schema it declared:
/// what every door reading batches pulls through, before the rows are landed
/// on one thread or a batch is handed to the workers as [`SourceJobs`].
///
/// Every cell is read by the position the declared schema gave it, so a
/// batch of another schema is excluded with a warning. A failure of the
/// reader that is the source's own is yielded and ends the batches; one
/// refusing a batch it read is warned about and passed over.
struct SourceBatches {
    /// The reader still read: none once it ended or failed.
    source: Option<BatchReader>,
}

impl SourceBatches {
    const fn over(source: Option<BatchReader>) -> Self {
        Self { source }
    }
}

impl Iterator for SourceBatches {
    type Item = Result<RecordBatch>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let source = self.source.as_mut()?;
            match source.next() {
                Some(Ok(batch)) if batch.schema() != source.schema() => warned!(
                    "FIX batch excluded: its schema is not the one its reader declared",
                    "schema",
                    "a batch of {} rows",
                    batch.num_rows()
                ),
                Some(Ok(batch)) => return Some(Ok(batch)),
                Some(Err(error)) => {
                    if let Some(error) = read_failure(error) {
                        self.source = None;
                        return Some(Err(error));
                    }
                }
                None => {
                    self.source = None;
                    return None;
                }
            }
        }
    }
}

/// The jobs the Arrow capture pools hand their workers, each beside the row
/// of its source batch it starts at: every batch [`SourceBatches`] yields,
/// whole, or - past twice [`FixCodec::PARALLEL_JOB_ROWS`] rows - cut into
/// near-equal row ranges, four per thread or fewer where a range would hold
/// fewer rows than that.
///
/// A cut is a zero-copy slice, so a worker holds part of one input batch and
/// never more; a job never spans two batches, so never a source failure nor
/// a batch excluded for its schema; and jobs are never joined, so the first
/// answer waits on no batch after its own.
struct SourceJobs {
    batches: SourceBatches,
    /// The most jobs one batch is cut into.
    most: usize,
    /// The batch being cut, the row its next job starts at, and how many
    /// jobs are still to cut from it.
    cutting: Option<(RecordBatch, usize, usize)>,
}

impl SourceJobs {
    fn over(source: BatchReader, threads: usize) -> Self {
        Self {
            batches: SourceBatches::over(Some(source)),
            most: threads.saturating_mul(4),
            cutting: None,
        }
    }
}

impl Iterator for SourceJobs {
    type Item = Result<(RecordBatch, usize)>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cutting.is_none() {
            let batch = match self.batches.next()? {
                Ok(batch) => batch,
                Err(error) => return Some(Err(error)),
            };
            let rows = batch.num_rows();
            if rows <= 2 * FixCodec::PARALLEL_JOB_ROWS {
                return Some(Ok((batch, 0)));
            }
            let jobs = self.most.min(rows / FixCodec::PARALLEL_JOB_ROWS);
            self.cutting = Some((batch, 0, jobs));
        }
        let (batch, start, left) = self.cutting.as_mut()?;
        // Near-equal: the rows still uncut over the jobs still to cut, so
        // no two jobs of a batch differ by more than a row.
        let at = *start;
        let rows = (batch.num_rows() - at).div_ceil(*left);
        let job = batch.slice(at, rows);
        *start += rows;
        *left -= 1;
        if *left == 0 {
            self.cutting = None;
        }
        Some(Ok((job, at)))
    }
}

/// The failure a reader's `error` is, where it is the source's own; none
/// once a refusal of the batch it read is warned about.
fn read_failure(error: ArrowError) -> Option<Error> {
    let error = crate::arrow::from_reader_error(error);
    if error.is_source_failure() {
        return Some(error.into());
    }
    warned!(
        "FIX batch excluded: its reader refused it",
        landing_subject(&error),
        "{error}"
    );
    None
}

/// One batch landed under `root`: whole where it lands, else row by row,
/// each row the landing refuses - a code no registry holds - excluded with a
/// warning naming it, so one cell no column accepts costs its row and never
/// its batch. Only a batch holding a refused row is landed twice. `batch`
/// is its source batch's rows from `start`, so the warning names the row of
/// the source batch whatever range of it a job was cut to.
fn landed(
    root: &Resolved,
    batch: RecordBatch,
    start: usize,
) -> crate::arrow::Result<SmallVec<[Serie; 1]>> {
    match land_batch(root, batch.clone(), &Proof::Unproven) {
        Ok(records) => return Ok(smallvec::smallvec![records]),
        Err(error) if error.is_source_failure() => return Err(error),
        Err(_) => {}
    }
    let mut kept = SmallVec::new();
    for row in 0..batch.num_rows() {
        match land_batch(root, batch.slice(row, 1), &Proof::Unproven) {
            Ok(records) => kept.push(records),
            Err(error) if error.is_source_failure() => return Err(error),
            Err(error) => warned!(
                "FIX row excluded: the landing refuses a cell of it",
                landing_subject(&error),
                "{error}, in row {} of its batch",
                start + row
            ),
        }
    }
    Ok(kept)
}

/// The column a landing's refusal names, the key its warning is counted
/// under: a row landed on its own is row zero of its own array, so the path
/// names the column and never where the row stood.
fn landing_subject(error: &crate::arrow::Error) -> &str {
    match error {
        crate::arrow::Error::InvalidValue { path, .. }
        | crate::arrow::Error::RequiredField { path, .. } => path,
        crate::arrow::Error::Core(core) => super::messages::refused(core),
        _ => "batch",
    }
}

/// The rows of a stream of capture batches, each beside the batch it is a
/// row of, landed once ([`landed`]), in row order.
///
/// The stream goes on past every batch and row it passes over; the source
/// reader's own failure is its one error, and ends it. The puller holds one
/// batch, and a row keeps its own alive until it is read.
struct BatchRows {
    source: SourceBatches,
    reader: Arc<RowReader>,
    /// The batch being read, and the row the next pull reads.
    held: Option<(Arc<Landed>, usize)>,
    /// What a batch landed row by row holds past `held`.
    pending: smallvec::IntoIter<[Landed; 1]>,
}

impl BatchRows {
    fn over(source: BatchReader, reader: Arc<RowReader>) -> Self {
        Self {
            source: SourceBatches::over(Some(source)),
            reader,
            held: None,
            pending: SmallVec::new().into_iter(),
        }
    }
}

impl Iterator for BatchRows {
    type Item = Result<(Arc<Landed>, usize)>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((batch, at)) = &mut self.held
                && *at < batch.records.len()
            {
                let row = *at;
                *at += 1;
                return Some(Ok((Arc::clone(batch), row)));
            }
            if let Some(batch) = self.pending.next() {
                self.held = Some((Arc::new(batch), 0));
                continue;
            }
            // The batch is spent, or none is held yet: the next validated one
            // is pulled, landed, and the spent one dropped.
            self.held = None;
            match self.source.next().map(|batch| self.reader.land(batch?, 0)) {
                Some(Ok(landed)) => self.pending = landed.into_iter(),
                Some(Err(error)) => return Some(Err(error)),
                None => return None,
            }
        }
    }
}

/// The rows of a stream of batches of FIX rows, each beside the Struct its
/// batch is, landed once ([`landed`]), in row order: the source reader's own
/// failure is the one error, and ends them.
struct StructRows {
    source: SourceBatches,
    /// The root every batch lands under, resolved once off the source.
    root: Resolved,
    /// The record column being read, beside the row the next pull reads.
    held: Option<(Arc<Serie>, usize)>,
    /// What a batch landed row by row holds past `held`.
    pending: smallvec::IntoIter<[Serie; 1]>,
}

impl StructRows {
    /// The rows of `source` under `root`, or none where the schema was
    /// `refused` and nothing is read.
    fn over(source: BatchReader, root: Resolved, refused: bool) -> Self {
        Self {
            source: SourceBatches::over((!refused).then_some(source)),
            root,
            held: None,
            pending: SmallVec::new().into_iter(),
        }
    }
}

impl Iterator for StructRows {
    type Item = Result<(Arc<Serie>, usize)>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some((batch, at)) = &mut self.held
                && *at < batch.len()
            {
                let row = *at;
                *at += 1;
                return Some(Ok((Arc::clone(batch), row)));
            }
            if let Some(records) = self.pending.next() {
                self.held = Some((Arc::new(records), 0));
                continue;
            }
            self.held = None;
            let root = &self.root;
            match self
                .source
                .next()
                .map(|batch| -> Result<_> { Ok(landed(root, batch?, 0)?) })
            {
                Some(Ok(landed)) => self.pending = landed.into_iter(),
                Some(Err(error)) => return Some(Err(error)),
                None => return None,
            }
        }
    }
}
