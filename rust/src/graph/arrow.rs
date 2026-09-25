//! The graph's Arrow rows: one lifted `marketdata` shape every
//! [`MarketData`] leaf is written in and read back from.
//!
//! A row is `kind` - the [`MarketKind`] spelling of its leaf - then the
//! sixteen [`EventColumn`]s, the nineteen [`MarketColumn`]s, the eight
//! [`OperationColumn`]s, the five book-control columns a market-data entry
//! states, and the nested columns a composite leaf fills: `executions` (a
//! trade's or a book's), `bidside` and `askside` (a book's), the
//! `snapshotpartitions` a book's last replacement covered, and `live` and
//! `deltas` (a book side's). Every fact is its own typed column; a leaf
//! leaves null what it does not state. A nested operation row is `kind`,
//! the event, market, operation and book-control columns; a side row is the
//! six element facts, the market columns, and its `live` and `deltas`
//! operation rows.
//!
//! Written column by column from the typed leaves, and read back
//! tolerantly: the reader's columns are resolved by name once per stream,
//! any subset in any order, a foreign column ignored and a column of
//! another castable type cast through one plan. Every stated identity and
//! every stated fact must be the one the rebuilt leaf derives.

use std::collections::BTreeSet;
use std::iter::FusedIterator;
use std::sync::Arc;

use arrow_array::builder::{ArrayBuilder, StringBuilder};
use arrow_array::{
    ArrayRef, BooleanArray, Decimal128Array, FixedSizeBinaryArray, Int32Array, ListArray, MapArray,
    RecordBatch, RecordBatchOptions, StringArray, StructArray, TimestampNanosecondArray,
    UInt32Array, UInt64Array,
};
use arrow_buffer::{NullBuffer, OffsetBuffer, ScalarBuffer};
use arrow_schema::{ArrowError, DataType as ArrowType, FieldRef, Fields, SchemaRef};
use smol_str::{SmolStr, format_smolstr};

use super::book::SnapshotPartition;
use super::facts::{MarketEventFacts, MarketFacts, OperationEventFacts};
use super::{
    BookEvent, BookRef, BookSide, Element, Event, EventColumn, ExecutionEvent, ExecutionKind,
    Market, MarketColumn, MarketData, MarketKind, MdUpdateAction, Operation, OperationColumn,
    OperationElement, OperationEvent, OperationKind, OrderKind, QuoteKind, SnapshotEvent,
    TradeEvent,
};
use super::{Lane, Metadata};
use crate::arrow::BatchReader;
use crate::idmap::IdMap;
use crate::path::{Path, Segment};
use crate::securityid::{SecType, SecurityId, SecurityIds};
use crate::serie::{
    BooleanSerie, DateTimeNanosecondSerie, Decimal128Serie, FixedBytesSerie, Int32Serie, MapSerie,
    SerieSerie, UInt32Serie, UInt64Serie, Utf8StringSerie,
};
use crate::{
    ArrowCastOptions, Ccy, CfiCode, CodeValue, DataType, Decimal18, Error, Field, MicCode, Result,
    Serie, SerieReader, Side, State, StructType, TimeInForce, Unit, Uuid,
};

const ROOT: &str = "marketdata";
const KIND: &str = "kind";
const OPERATION_ROW: &str = "operationevent";
const PARTITION_ROW: &str = "snapshotpartition";
const EXECUTIONS: &str = "executions";
const BIDSIDE: &str = "bidside";
const ASKSIDE: &str = "askside";
const SNAPSHOT_PARTITIONS: &str = "snapshotpartitions";
const LIVE: &str = "live";
const DELTAS: &str = "deltas";
/// Every kind a row may state, as a refusal lists them.
const KINDS: &str = "order, quote, execution, book_side, order_event, quote_event, \
                     execution_event, trade_event, book_event or snapshot_event";
/// The element facts a side row states: an element's own, and no clock.
const SIDE_ELEMENT_COLUMNS: [EventColumn; 6] = [
    EventColumn::CurrUuid,
    EventColumn::CrossUuid,
    EventColumn::CrossCode,
    EventColumn::CurrHashCode,
    EventColumn::CrossHashCode,
    EventColumn::SrcUuids,
];
/// A near-enough width of one operation row's fixed leaves - its clocks,
/// identities, codes and decimals - which the byte bound charges per row.
const OPERATION_ROW_BYTES: u64 = 512;
/// The bytes of one identity's `FixedSizeBinary` slot.
const UUID_WIDTH: i32 = 16;

impl MarketData {
    /// The canonical Arrow row field every leaf is written in.
    ///
    /// The required struct `marketdata`: `kind` (a [`MarketKind`] spelling),
    /// then [`EventColumn::ALL`] - every one nullable, since an undated leaf
    /// states no clock - [`MarketColumn::ALL`], [`OperationColumn::ALL`],
    /// the five book-control columns (`mdupdateaction`, `bookscope`,
    /// `mdentrypositionno`, `mdentrypx`, `mdentrysize`), then the nullable
    /// nested columns: `executions` (a trade's or a book's operation rows),
    /// `bidside` and `askside` (a book's sides), `snapshotpartitions` (a
    /// book's) and `live` and `deltas` (a book side's operation rows).
    ///
    /// ```
    /// use yggdryl::graph::MarketData;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let field = MarketData::field()?;
    /// assert_eq!(field.name(), "marketdata");
    /// assert_eq!(field.fields()[0].name(), "kind");
    /// assert_eq!(field.fields()[1].name(), "currunix");
    /// assert!(field.fields()[1].is_nullable());
    /// assert_eq!(field.field_len(), 1 + 16 + 19 + 8 + 5 + 6);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when a field cannot be built.
    pub fn field() -> Result<Field> {
        struct_field(ROOT, &root_columns(), Role::Root, false)
    }

    /// Streams values into bounded Arrow record batches under
    /// [`Self::field`], every leaf laid out column by column.
    ///
    /// The source is not pulled until the returned reader is pulled. A `None`
    /// row size uses the crate default and zero normalizes to one; a stated
    /// byte size closes a nonempty batch once the rows already in it reach the
    /// bound, so one oversized row is still emitted alone. A value is written
    /// only as its canonical self: an identity or a fact its finalize would
    /// change is refused, located on its row. A source or refusal error
    /// follows the completed prefix and fuses the reader.
    ///
    /// # Errors
    ///
    /// Returns an error when the row field cannot be built.
    pub fn arrow_reader<I>(
        elements: I,
        batch_row_size: Option<usize>,
        batch_byte_size: Option<u64>,
    ) -> Result<BatchReader>
    where
        I: IntoIterator,
        I::Item: Into<Result<MarketData>>,
        I::IntoIter: Send + 'static,
    {
        let schema = Self::field()?.into_arrow_schema()?;
        let shape = Shape::new(&root_columns(), schema.fields())?;
        Ok(Box::new(Writer {
            source: elements.into_iter(),
            shape,
            schema,
            batch_row_size: batch_row_size
                .unwrap_or(crate::arrow::rows::DEFAULT_BATCH_ROW_SIZE)
                .max(1),
            batch_byte_size,
            ordinal: 0,
            pending_error: None,
            done: false,
        }))
    }

    /// Reads values back from record batches, one value per row.
    ///
    /// Tolerant of the batch's shape: its root columns are resolved by name
    /// once per stream, whatever their case, any subset in any order; a
    /// column this shape does not name is ignored, and a column of another
    /// castable type is cast through one plan compiled before the first
    /// batch. A row's `kind` names its leaf; a missing or unknown kind, or
    /// a fact the kind requires - `currunix` for a dated leaf, a trade's
    /// `executions`, a book's two sides - is refused at the first row that
    /// needs it. Every identity a row states must be the one its rebuilt
    /// leaf derives, and an identity column that stands must state one - a
    /// null `curruuid`, `crossuuid`, `currhashcode` or `crosshashcode` is
    /// refused - while an absent one states nothing; every other fact a
    /// row states must be the one that leaf settles on, and a null cell of
    /// one states nothing. The stream fuses after an error.
    ///
    /// # Errors
    ///
    /// Returns an error when two columns name one fact, or a column cannot
    /// be cast to the type its fact is read at.
    pub fn from_arrow_reader(
        batches: BatchReader,
    ) -> Result<impl FusedIterator<Item = Result<MarketData>> + Send + 'static> {
        let schema = batches.schema();
        let mut present: Vec<Column> = Vec::new();
        for field in schema.fields() {
            let Some(column) = Column::of_name(field.name()) else {
                continue;
            };
            if present.contains(&column) {
                return Err(invalid(
                    SmolStr::new_static("$"),
                    format_smolstr!(
                        "expected one column naming {}, got a second: {:?}",
                        column.name(),
                        field.name()
                    ),
                ));
            }
            present.push(column);
        }
        // Canonical order, whatever the source's: the cast reorders.
        let columns: Vec<Column> = root_columns()
            .into_iter()
            .filter(|column| present.contains(column))
            .collect();
        let target = struct_field(ROOT, &columns, Role::Read, false)?;
        let batches =
            SerieReader::from_arrow_reader(Some(&target), batches, ArrowCastOptions::new())?;
        Ok(Rows {
            batches,
            layouts: Layouts::new(columns),
            landed: None,
            row: 0,
            ordinal: 0,
            done: false,
        })
    }
}

// ------------------------------------------------------------------------
// The columns
// ------------------------------------------------------------------------

/// One column of a row this module writes: a fact, or a nested one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Column {
    Kind,
    Event(EventColumn),
    Market(MarketColumn),
    Operation(OperationColumn),
    Control(Control),
    Executions,
    BidSide,
    AskSide,
    SnapshotPartitions,
    Live,
    Deltas,
}

/// Which struct a column's field is built for.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Role {
    /// The written `marketdata` root.
    Root,
    /// The read root: every column nullable, so absence is the reader's to
    /// judge rather than the cast's to repair.
    Read,
    /// A nested operation row.
    Operation,
    /// A nested side row.
    Side,
}

impl Column {
    const fn name(self) -> &'static str {
        match self {
            Self::Kind => KIND,
            Self::Event(column) => column.name(),
            Self::Market(column) => column.name(),
            Self::Operation(column) => column.name(),
            Self::Control(column) => column.name(),
            Self::Executions => EXECUTIONS,
            Self::BidSide => BIDSIDE,
            Self::AskSide => ASKSIDE,
            Self::SnapshotPartitions => SNAPSHOT_PARTITIONS,
            Self::Live => LIVE,
            Self::Deltas => DELTAS,
        }
    }

    /// The root column one name spells, whatever its case.
    fn of_name(name: &str) -> Option<Self> {
        root_columns()
            .into_iter()
            .find(|column| crate::folds_equal(column.name(), name))
    }

    /// The column's field in the struct `role` names.
    fn field(self, role: Role) -> Result<Field> {
        let field = match self {
            Self::Kind => {
                let mut field = Field::new(KIND, DataType::utf8(), role != Role::Root);
                field.set_display("Kind")?;
                field
            }
            Self::Event(column) => column.field()?,
            Self::Market(column) => column.field()?,
            Self::Operation(column) => column.field()?,
            Self::Control(column) => column.field()?,
            Self::Executions | Self::Live | Self::Deltas => {
                DataType::serie(operation_row_field()?).nullable_field(self.name())
            }
            Self::BidSide | Self::AskSide => {
                struct_field(self.name(), &side_columns(), Role::Side, true)?
            }
            Self::SnapshotPartitions => snapshot_partitions_field()?,
        };
        // A root states only what its leaf does, so every clock and nested
        // column may be null there; a nested row's own columns keep the
        // nullability its fact has. A side's operation lists are always
        // stated.
        Ok(match (role, self) {
            (Role::Read, _) | (Role::Root, Self::Event(_)) => field.with_nullable(true),
            (Role::Side, Self::Live | Self::Deltas) => field.with_nullable(false),
            _ => field,
        })
    }

    /// The storage a landed column of this fact holds.
    const fn storage(self) -> Storage {
        match self {
            Self::Kind => Storage::Text,
            Self::Event(column) => match column {
                EventColumn::CurrUnix
                | EventColumn::CreaUnix
                | EventColumn::ExecUnix
                | EventColumn::RecdUnix
                | EventColumn::ExprTime
                | EventColumn::PrevUnix
                | EventColumn::SnapUnix => Storage::Clock,
                EventColumn::CurrUuid | EventColumn::CrossUuid | EventColumn::PrevUuid => {
                    Storage::Uuid
                }
                EventColumn::CrossCode => Storage::Text,
                EventColumn::CurrHashCode | EventColumn::CrossHashCode | EventColumn::SeqNum => {
                    Storage::UInt64
                }
                EventColumn::SrcUuids => Storage::Uuids,
                EventColumn::State => Storage::Code(Code::State),
            },
            Self::Market(column) => {
                if is_decimal(column) {
                    Storage::Decimal
                } else {
                    match column {
                        MarketColumn::Ticker => Storage::Text,
                        MarketColumn::SecurityIds | MarketColumn::Metadata => Storage::Pairs,
                        MarketColumn::Currency => Storage::Code(Code::Ccy),
                        MarketColumn::Unit => Storage::Code(Code::Unit),
                        MarketColumn::Side => Storage::Code(Code::Side),
                        MarketColumn::CfiCode => Storage::Code(Code::Cfi),
                        _ => Storage::Code(Code::Mic),
                    }
                }
            }
            Self::Operation(column) => match column {
                OperationColumn::MarketOperationId => Storage::Int32,
                OperationColumn::Tradable => Storage::Boolean,
                OperationColumn::TimeInForce => Storage::Code(Code::TimeInForce),
                OperationColumn::AccountIds
                | OperationColumn::UserIds
                | OperationColumn::AltIds => Storage::Pairs,
                OperationColumn::Bid | OperationColumn::Ask => Storage::Lane,
            },
            Self::Control(column) => match column {
                Control::Action | Control::Scope => Storage::Text,
                Control::Position => Storage::UInt32,
                Control::EntryPx | Control::EntrySize => Storage::Decimal,
            },
            Self::Executions
            | Self::BidSide
            | Self::AskSide
            | Self::SnapshotPartitions
            | Self::Live
            | Self::Deltas => Storage::Nested,
        }
    }
}

/// Every root column, in canonical row order.
fn root_columns() -> Vec<Column> {
    let mut columns = operation_columns();
    columns.extend([
        Column::Executions,
        Column::BidSide,
        Column::AskSide,
        Column::SnapshotPartitions,
        Column::Live,
        Column::Deltas,
    ]);
    columns
}

/// Every column of an operation row, in canonical order.
fn operation_columns() -> Vec<Column> {
    std::iter::once(Column::Kind)
        .chain(EventColumn::ALL.map(Column::Event))
        .chain(MarketColumn::ALL.map(Column::Market))
        .chain(OperationColumn::ALL.map(Column::Operation))
        .chain(Control::ALL.map(Column::Control))
        .collect()
}

/// Every column of a side row, in canonical order.
fn side_columns() -> Vec<Column> {
    SIDE_ELEMENT_COLUMNS
        .map(Column::Event)
        .into_iter()
        .chain(MarketColumn::ALL.map(Column::Market))
        .chain([Column::Live, Column::Deltas])
        .collect()
}

fn struct_field(name: &str, columns: &[Column], role: Role, nullable: bool) -> Result<Field> {
    let fields = columns
        .iter()
        .map(|column| column.field(role))
        .collect::<Result<Vec<_>>>()?;
    Ok(Field::new(
        name,
        DataType::from(StructType::from_fields(fields)?),
        nullable,
    ))
}

/// The item of `executions`, `live` and `deltas`: one dated operation.
fn operation_row_field() -> Result<Field> {
    struct_field(OPERATION_ROW, &operation_columns(), Role::Operation, false)
}

fn snapshot_partitions_field() -> Result<Field> {
    let entry = DataType::from(StructType::from_fields(vec![
        DataType::utf8().nullable_field("symbol"),
        DataType::utf8().required_field("scope"),
    ])?)
    .required_field(PARTITION_ROW);
    Ok(DataType::serie(entry).nullable_field(SNAPSHOT_PARTITIONS))
}

/// One of the five book-control columns a market-data entry states.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Control {
    Action,
    Scope,
    Position,
    EntryPx,
    EntrySize,
}

impl Control {
    const ALL: [Self; 5] = [
        Self::Action,
        Self::Scope,
        Self::Position,
        Self::EntryPx,
        Self::EntrySize,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::Action => "mdupdateaction",
            Self::Scope => "bookscope",
            Self::Position => "mdentrypositionno",
            Self::EntryPx => "mdentrypx",
            Self::EntrySize => "mdentrysize",
        }
    }

    fn field(self) -> Result<Field> {
        let (datatype, display) = match self {
            Self::Action => (DataType::utf8(), "MD Update Action"),
            Self::Scope => (DataType::utf8(), "Book Scope"),
            Self::Position => (DataType::UInt32, "MD Entry Position"),
            Self::EntryPx => (DataType::DECIMAL, "MD Entry Price"),
            Self::EntrySize => (DataType::DECIMAL, "MD Entry Size"),
        };
        let mut field = datatype.nullable_field(self.name());
        field.set_display(display)?;
        Ok(field)
    }
}

/// Whether a market column holds a decimal: every price and quantity.
const fn is_decimal(column: MarketColumn) -> bool {
    matches!(
        column,
        MarketColumn::Price
            | MarketColumn::Quantity
            | MarketColumn::LastPx
            | MarketColumn::LastQty
            | MarketColumn::AvgPx
            | MarketColumn::CumQty
            | MarketColumn::LeavesQty
            | MarketColumn::PrevPx
            | MarketColumn::PrevQty
            | MarketColumn::SpotRate
            | MarketColumn::ForwardPoints
    )
}

/// The decimal a market states under a decimal column.
fn market_decimal<E: Market + ?Sized>(column: MarketColumn, market: &E) -> Option<Decimal18> {
    match column {
        MarketColumn::Price => market.get_price(),
        MarketColumn::Quantity => market.get_quantity(),
        MarketColumn::LastPx => market.get_lastpx(),
        MarketColumn::LastQty => market.get_lastqty(),
        MarketColumn::AvgPx => market.get_avgpx(),
        MarketColumn::CumQty => market.get_cumqty(),
        MarketColumn::LeavesQty => market.get_leavesqty(),
        MarketColumn::PrevPx => market.get_prevpx(),
        MarketColumn::PrevQty => market.get_prevqty(),
        MarketColumn::SpotRate => market.get_spotrate(),
        MarketColumn::ForwardPoints => market.get_forwardpoints(),
        _ => None,
    }
}

/// Records a decimal market column's value.
fn set_market_decimal<E: Market + ?Sized>(
    column: MarketColumn,
    market: &mut E,
    value: Option<Decimal18>,
) {
    match column {
        MarketColumn::Price => market.set_price(value),
        MarketColumn::Quantity => market.set_quantity(value),
        MarketColumn::LastPx => market.set_lastpx(value),
        MarketColumn::LastQty => market.set_lastqty(value),
        MarketColumn::AvgPx => market.set_avgpx(value),
        MarketColumn::CumQty => market.set_cumqty(value),
        MarketColumn::LeavesQty => market.set_leavesqty(value),
        MarketColumn::PrevPx => market.set_prevpx(value),
        MarketColumn::PrevQty => market.set_prevqty(value),
        MarketColumn::SpotRate => market.set_spotrate(value),
        MarketColumn::ForwardPoints => market.set_forwardpoints(value),
        _ => {}
    }
}

// ------------------------------------------------------------------------
// Writing
// ------------------------------------------------------------------------

/// One row as the writer reads it: the leaf's facts through the traits it
/// answers, and the nested rows it holds.
#[derive(Clone, Copy)]
struct Row<'a> {
    kind: MarketKind,
    element: &'a dyn Element,
    event: Option<&'a dyn Event>,
    market: &'a dyn Market,
    operation: Option<&'a dyn Operation>,
    control: Option<&'a BookRef>,
    executions: Option<&'a [ExecutionEvent]>,
    book: Option<&'a BookEvent>,
    side: Option<&'a BookSide>,
}

impl<'a> Row<'a> {
    fn of(value: &'a MarketData) -> Self {
        let kind = value.kind();
        match value {
            MarketData::Order(leaf) => Self::undated(kind, leaf, leaf, Some(leaf)),
            MarketData::Quote(leaf) => Self::undated(kind, leaf, leaf, Some(leaf)),
            MarketData::Execution(leaf) => Self::undated(kind, leaf, leaf, Some(leaf)),
            MarketData::BookSide(side) => Self {
                side: Some(side),
                ..Self::undated(kind, side, side, None)
            },
            MarketData::OrderEvent(leaf) => Self::operation(kind, leaf, leaf.book()),
            MarketData::QuoteEvent(leaf) => Self::operation(kind, leaf, leaf.book()),
            MarketData::ExecutionEvent(leaf) => Self::operation(kind, leaf, leaf.book()),
            MarketData::TradeEvent(trade) => Self {
                executions: Some(trade.executions()),
                ..Self::operation(kind, trade, None)
            },
            MarketData::BookEvent(book) => Self {
                executions: Some(book.executions()),
                book: Some(book),
                ..Self::dated(kind, book.as_ref())
            },
            MarketData::SnapshotEvent(control) => Self {
                control: Some(control.book()),
                ..Self::dated(kind, control)
            },
        }
    }

    fn execution(execution: &'a ExecutionEvent) -> Self {
        Self::operation(MarketKind::ExecutionEvent, execution, execution.book())
    }

    fn side(side: &'a BookSide) -> Self {
        Self {
            side: Some(side),
            ..Self::undated(MarketKind::BookSide, side, side, None)
        }
    }

    fn undated(
        kind: MarketKind,
        element: &'a dyn Element,
        market: &'a dyn Market,
        operation: Option<&'a dyn Operation>,
    ) -> Self {
        Self {
            kind,
            element,
            event: None,
            market,
            operation,
            control: None,
            executions: None,
            book: None,
            side: None,
        }
    }

    fn dated<E: Event + Market>(kind: MarketKind, event: &'a E) -> Self {
        Self {
            event: Some(event),
            ..Self::undated(kind, event, event, None)
        }
    }

    fn operation<E: Event + Market + Operation>(
        kind: MarketKind,
        event: &'a E,
        control: Option<&'a BookRef>,
    ) -> Self {
        Self {
            operation: Some(event),
            control,
            ..Self::dated(kind, event)
        }
    }

    /// The nested operation rows this row holds under `column`, or nothing
    /// where its leaf holds no such list.
    fn nested(&self, column: Column) -> Option<Vec<Self>> {
        match column {
            Column::Executions => self
                .executions
                .map(|executions| executions.iter().map(Self::execution).collect()),
            Column::Live => self.side.map(|side| side.live().map(Self::of).collect()),
            Column::Deltas => self
                .side
                .map(|side| side.deltas().iter().map(Self::of).collect()),
            _ => None,
        }
    }
}

/// The typed facts a row states under one leaf column, each read through
/// the trait that answers it: no cell is built as a value.
impl<'a> Row<'a> {
    fn clock(&self, column: Column) -> Option<i64> {
        let (Column::Event(column), Some(event)) = (column, self.event) else {
            return None;
        };
        match column {
            EventColumn::CurrUnix => Some(event.get_currunix()),
            EventColumn::CreaUnix => event.get_creaunix(),
            EventColumn::ExecUnix => event.get_execunix(),
            EventColumn::RecdUnix => event.get_recdunix(),
            EventColumn::ExprTime => event.get_exprtime(),
            EventColumn::PrevUnix => event.get_prevunix(),
            EventColumn::SnapUnix => event.get_snapunix(),
            _ => None,
        }
    }

    fn uuid(&self, column: Column) -> Option<Uuid> {
        match column {
            Column::Event(EventColumn::CurrUuid) => Some(self.element.get_curruuid()),
            Column::Event(EventColumn::CrossUuid) => Some(self.element.get_crossuuid()),
            Column::Event(EventColumn::PrevUuid) => self.event?.get_prevuuid(),
            _ => None,
        }
    }

    /// The sources a row states; none where it names none.
    fn uuids(&self, column: Column) -> Option<&'a [Uuid]> {
        let element: &'a dyn Element = self.element;
        match column {
            Column::Event(EventColumn::SrcUuids) => {
                Some(element.get_srcuuids()).filter(|uuids| !uuids.is_empty())
            }
            _ => None,
        }
    }

    fn u64(&self, column: Column) -> Option<u64> {
        match column {
            Column::Event(EventColumn::CurrHashCode) => Some(self.element.get_currhashcode()),
            Column::Event(EventColumn::CrossHashCode) => Some(self.element.get_crosshashcode()),
            Column::Event(EventColumn::SeqNum) => {
                Some(self.event?.get_seqnum()).filter(|seqnum| *seqnum != 0)
            }
            _ => None,
        }
    }

    fn u32(&self, column: Column) -> Option<u32> {
        match column {
            Column::Control(Control::Position) => self.control?.position,
            _ => None,
        }
    }

    fn i32(&self, column: Column) -> Option<i32> {
        match column {
            Column::Operation(OperationColumn::MarketOperationId) => {
                self.operation?.get_marketoperationid()
            }
            _ => None,
        }
    }

    fn boolean(&self, column: Column) -> Option<bool> {
        match column {
            Column::Operation(OperationColumn::Tradable) => self.operation?.get_tradable(),
            _ => None,
        }
    }

    fn decimal(&self, column: Column) -> Option<Decimal18> {
        match column {
            Column::Market(column) => market_decimal(column, self.market),
            Column::Control(Control::EntryPx) => self.control?.entry_px,
            Column::Control(Control::EntrySize) => self.control?.entry_size,
            _ => None,
        }
    }

    /// The text a row states under a text or code column: a code as the
    /// text its storage holds.
    fn text(&self, column: Column) -> Option<&'a str> {
        let element: &'a dyn Element = self.element;
        let market: &'a dyn Market = self.market;
        match column {
            Column::Kind => Some(self.kind.as_str()),
            Column::Event(EventColumn::CrossCode) => {
                Some(element.get_crosscode()).filter(|code| !code.is_empty())
            }
            Column::Event(EventColumn::State) => {
                let event: &'a dyn Event = self.event?;
                Some(event.get_state().as_str())
            }
            Column::Market(MarketColumn::Currency) => Some(market.get_currency().as_str()),
            Column::Market(MarketColumn::Unit) => Some(market.get_unit().as_str()),
            Column::Market(MarketColumn::Side) => Some(market.get_side().as_str()),
            Column::Market(MarketColumn::CfiCode) => market.get_cficode().map(CfiCode::as_str),
            Column::Market(MarketColumn::MicCode) => market.get_miccode().map(MicCode::as_str),
            Column::Market(MarketColumn::Ticker) => market.get_ticker(),
            Column::Operation(OperationColumn::TimeInForce) => {
                let operation: &'a dyn Operation = self.operation?;
                operation.get_tif().map(TimeInForce::as_str)
            }
            Column::Control(Control::Action) => self.control?.action.map(MdUpdateAction::as_str),
            Column::Control(Control::Scope) => self.control?.scope.as_deref(),
            _ => None,
        }
    }

    /// Appends the entries a row states under a map column; whether it
    /// states any.
    fn pairs(&self, column: Column, keys: &mut StringBuilder, values: &mut StringBuilder) -> bool {
        let mut push = |key: &str, value: &str| {
            keys.append_value(key);
            values.append_value(value);
        };
        match column {
            Column::Market(MarketColumn::SecurityIds) => {
                let ids = self.market.get_securityids();
                for id in ids.iter() {
                    push(id.key_str(), id.code());
                }
                !ids.is_empty()
            }
            Column::Market(MarketColumn::Metadata) => {
                let metadata = self.market.get_metadata();
                for (key, value) in metadata {
                    push(key, value);
                }
                !metadata.is_empty()
            }
            Column::Operation(column) => {
                let Some(operation) = self.operation else {
                    return false;
                };
                let ids = match column {
                    OperationColumn::AccountIds => operation.get_accountids(),
                    OperationColumn::UserIds => operation.get_userids(),
                    OperationColumn::AltIds => operation.get_altids(),
                    _ => return false,
                };
                for (key, value) in ids.iter() {
                    push(key, value);
                }
                !ids.is_empty()
            }
            _ => false,
        }
    }

    fn lane(&self, column: Column) -> Option<&'a Lane> {
        let operation: &'a dyn Operation = self.operation?;
        match column {
            Column::Operation(OperationColumn::Bid) => operation.get_bid(),
            Column::Operation(OperationColumn::Ask) => operation.get_ask(),
            _ => None,
        }
    }
}

/// What one value charges a batch's byte bound: a fixed width per
/// operation row it lays out, nested rows included.
fn charge(value: &MarketData) -> u64 {
    let side = |side: &BookSide| side.len() + side.deltas().len();
    let nested = match value {
        MarketData::TradeEvent(trade) => trade.executions().len(),
        MarketData::BookEvent(book) => {
            side(book.bid()) + side(book.ask()) + book.executions().len()
        }
        MarketData::BookSide(held) => side(held),
        _ => 0,
    };
    (1 + nested as u64) * (crate::arrow::rows::ROW_OVERHEAD + OPERATION_ROW_BYTES)
}

/// A struct's columns as the writer lays them out, resolved once per
/// stream: each column's Arrow field, what a nested column holds, and the
/// Arrow children the struct is assembled under.
struct Shape {
    slots: Vec<Slot>,
    fields: Fields,
}

struct Slot {
    column: Column,
    /// The column's Arrow field: the type a leaf column is laid out at.
    arrow: FieldRef,
    nested: Nested,
}

enum Nested {
    None,
    /// A list of operation rows: its item field and the rows' shape.
    Operations(FieldRef, Box<Shape>),
    /// A side struct's shape.
    Side(Box<Shape>),
    /// The snapshot partitions: the item field and the entries' Arrow
    /// children.
    Partitions(FieldRef, Fields),
}

impl Shape {
    fn new(columns: &[Column], fields: &Fields) -> Result<Self> {
        let slots = columns
            .iter()
            .zip(fields.iter())
            .map(|(column, arrow)| {
                let nested = match (column, arrow.data_type()) {
                    (Column::Executions | Column::Live | Column::Deltas, ArrowType::List(item)) => {
                        Nested::Operations(
                            Arc::clone(item),
                            Box::new(Self::new(&operation_columns(), struct_children(item)?)?),
                        )
                    }
                    (Column::BidSide | Column::AskSide, ArrowType::Struct(children)) => {
                        Nested::Side(Box::new(Self::new(&side_columns(), children)?))
                    }
                    (Column::SnapshotPartitions, ArrowType::List(item)) => {
                        Nested::Partitions(Arc::clone(item), struct_children(item)?.clone())
                    }
                    _ => Nested::None,
                };
                Ok(Slot {
                    column: *column,
                    arrow: Arc::clone(arrow),
                    nested,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            slots,
            fields: fields.clone(),
        })
    }

    /// One batch of `rows` under `schema`.
    fn batch(&self, schema: &SchemaRef, rows: &[Row<'_>]) -> Result<RecordBatch> {
        let columns = self.columns(rows)?;
        let options = RecordBatchOptions::new().with_row_count(Some(rows.len()));
        Ok(RecordBatch::try_new_with_options(
            Arc::clone(schema),
            columns,
            &options,
        )?)
    }

    fn columns(&self, rows: &[Row<'_>]) -> Result<Vec<ArrayRef>> {
        self.slots.iter().map(|slot| slot.array(rows)).collect()
    }

    fn array(&self, rows: &[Row<'_>], nulls: Option<NullBuffer>) -> Result<ArrayRef> {
        Ok(Arc::new(StructArray::try_new(
            self.fields.clone(),
            self.columns(rows)?,
            nulls,
        )?))
    }
}

impl Slot {
    /// This column over `rows`, laid out in one pass.
    fn array(&self, rows: &[Row<'_>]) -> Result<ArrayRef> {
        match &self.nested {
            Nested::None => self.leaf(rows),
            Nested::Operations(item, shape) => {
                let mut offsets = Vec::with_capacity(rows.len() + 1);
                offsets.push(0_i32);
                let mut valid = Vec::with_capacity(rows.len());
                let mut items = Vec::new();
                for row in rows {
                    let nested = row.nested(self.column);
                    valid.push(nested.is_some());
                    items.extend(nested.into_iter().flatten());
                    offsets.push(offset(items.len())?);
                }
                let values = shape.array(&items, None)?;
                list(item, offsets, values, &valid)
            }
            Nested::Side(shape) => {
                // An absent side still lays out a slot, masked: the empty
                // side of the canonical book.
                let empty = BookSide::default();
                let mut valid = Vec::with_capacity(rows.len());
                let sides: Vec<Row<'_>> = rows
                    .iter()
                    .map(|row| {
                        let side = row.book.map(|book| match self.column {
                            Column::BidSide => book.bid(),
                            _ => book.ask(),
                        });
                        valid.push(side.is_some());
                        Row::side(side.unwrap_or(&empty))
                    })
                    .collect();
                shape.array(&sides, validity(&valid))
            }
            Nested::Partitions(item, fields) => {
                let mut offsets = Vec::with_capacity(rows.len() + 1);
                offsets.push(0_i32);
                let mut valid = Vec::with_capacity(rows.len());
                let mut partitions: Vec<&SnapshotPartition> = Vec::new();
                for row in rows {
                    let held = row
                        .book
                        .map(BookEvent::snapshot_partitions)
                        .filter(|held| !held.is_empty());
                    valid.push(held.is_some());
                    partitions.extend(held.into_iter().flatten());
                    offsets.push(offset(partitions.len())?);
                }
                let symbols: StringArray = partitions
                    .iter()
                    .map(|partition| partition.symbol.as_deref())
                    .collect();
                let scopes: StringArray = partitions
                    .iter()
                    .map(|partition| Some(partition.scope.as_str()))
                    .collect();
                let values: ArrayRef = Arc::new(StructArray::try_new(
                    fields.clone(),
                    vec![Arc::new(symbols), Arc::new(scopes)],
                    None,
                )?);
                list(item, offsets, values, &valid)
            }
        }
    }

    /// A leaf column over `rows`, laid out from the typed facts straight
    /// into the storage its [`Storage`] names.
    fn leaf(&self, rows: &[Row<'_>]) -> Result<ArrayRef> {
        let column = self.column;
        let datatype = self.arrow.data_type();
        Ok(match column.storage() {
            Storage::Clock => Arc::new(
                rows.iter()
                    .map(|row| row.clock(column))
                    .collect::<TimestampNanosecondArray>()
                    .with_data_type(datatype.clone()),
            ),
            Storage::Uuid => Arc::new(FixedSizeBinaryArray::try_from_sparse_iter_with_size(
                rows.iter()
                    .map(|row| row.uuid(column).map(Uuid::into_bytes)),
                UUID_WIDTH,
            )?),
            Storage::Uuids => {
                let ArrowType::List(item) = datatype else {
                    return Err(unlanded(column, Storage::Uuids));
                };
                let mut offsets = Vec::with_capacity(rows.len() + 1);
                offsets.push(0_i32);
                let mut valid = Vec::with_capacity(rows.len());
                let mut uuids: Vec<Uuid> = Vec::new();
                for row in rows {
                    let held = row.uuids(column);
                    valid.push(held.is_some());
                    uuids.extend(held.into_iter().flatten());
                    offsets.push(offset(uuids.len())?);
                }
                let values = FixedSizeBinaryArray::try_from_sparse_iter_with_size(
                    uuids.iter().map(|uuid| Some(uuid.into_bytes())),
                    UUID_WIDTH,
                )?;
                list(item, offsets, Arc::new(values), &valid)?
            }
            Storage::UInt64 => Arc::new(
                rows.iter()
                    .map(|row| row.u64(column))
                    .collect::<UInt64Array>(),
            ),
            Storage::UInt32 => Arc::new(
                rows.iter()
                    .map(|row| row.u32(column))
                    .collect::<UInt32Array>(),
            ),
            Storage::Int32 => Arc::new(
                rows.iter()
                    .map(|row| row.i32(column))
                    .collect::<Int32Array>(),
            ),
            Storage::Boolean => Arc::new(
                rows.iter()
                    .map(|row| row.boolean(column))
                    .collect::<BooleanArray>(),
            ),
            Storage::Decimal => Arc::new(
                rows.iter()
                    .map(|row| row.decimal(column).map(Decimal18::units))
                    .collect::<Decimal128Array>()
                    .with_data_type(datatype.clone()),
            ),
            Storage::Text | Storage::Code(_) => Arc::new(
                rows.iter()
                    .map(|row| row.text(column))
                    .collect::<StringArray>(),
            ),
            Storage::Pairs => {
                let ArrowType::Map(entries, sorted) = datatype else {
                    return Err(unlanded(column, Storage::Pairs));
                };
                let ArrowType::Struct(children) = entries.data_type() else {
                    return Err(unlanded(column, Storage::Pairs));
                };
                let mut keys = StringBuilder::new();
                let mut values = StringBuilder::new();
                let mut offsets = Vec::with_capacity(rows.len() + 1);
                offsets.push(0_i32);
                let mut valid = Vec::with_capacity(rows.len());
                for row in rows {
                    valid.push(row.pairs(column, &mut keys, &mut values));
                    offsets.push(offset(keys.len())?);
                }
                let entries_array = StructArray::try_new(
                    children.clone(),
                    vec![Arc::new(keys.finish()), Arc::new(values.finish())],
                    None,
                )?;
                Arc::new(MapArray::try_new(
                    Arc::clone(entries),
                    OffsetBuffer::new(ScalarBuffer::from(offsets)),
                    entries_array,
                    validity(&valid),
                    *sorted,
                )?)
            }
            Storage::Lane => {
                let ArrowType::Struct(children) = datatype else {
                    return Err(unlanded(column, Storage::Lane));
                };
                let lanes: Vec<Option<&Lane>> = rows.iter().map(|row| row.lane(column)).collect();
                let decimal = |at: usize, read: fn(&Lane) -> Option<Decimal18>| -> ArrayRef {
                    Arc::new(
                        lanes
                            .iter()
                            .map(|lane| lane.and_then(read).map(Decimal18::units))
                            .collect::<Decimal128Array>()
                            .with_data_type(children[at].data_type().clone()),
                    )
                };
                let currency: StringArray = lanes
                    .iter()
                    .map(|lane| {
                        lane.and_then(|lane| lane.currency.as_ref())
                            .map(Ccy::as_str)
                    })
                    .collect();
                let unit: StringArray = lanes
                    .iter()
                    .map(|lane| lane.and_then(|lane| lane.unit.as_ref()).map(Unit::as_str))
                    .collect();
                let valid: Vec<bool> = lanes.iter().map(Option::is_some).collect();
                Arc::new(StructArray::try_new(
                    children.clone(),
                    vec![
                        decimal(0, |lane| lane.price),
                        decimal(1, |lane| lane.spotrate),
                        decimal(2, |lane| lane.forwardpoints),
                        Arc::new(currency),
                        decimal(4, |lane| lane.quantity),
                        Arc::new(unit),
                    ],
                    validity(&valid),
                )?)
            }
            Storage::Nested => return Err(unlanded(column, Storage::Nested)),
        })
    }
}

fn struct_children(item: &FieldRef) -> Result<&Fields> {
    match item.data_type() {
        ArrowType::Struct(children) => Ok(children),
        other => Err(invalid(
            SmolStr::new_static("$"),
            format_smolstr!("expected a struct item, got {other}"),
        )),
    }
}

fn offset(len: usize) -> Result<i32> {
    i32::try_from(len).map_err(|_| {
        invalid(
            SmolStr::new_static("$"),
            format_smolstr!(
                "expected at most {} nested rows in one batch, got {len}",
                i32::MAX
            ),
        )
    })
}

fn validity(valid: &[bool]) -> Option<NullBuffer> {
    (!valid.iter().all(|held| *held)).then(|| NullBuffer::from(valid.to_vec()))
}

fn list(item: &FieldRef, offsets: Vec<i32>, values: ArrayRef, valid: &[bool]) -> Result<ArrayRef> {
    Ok(Arc::new(ListArray::try_new(
        Arc::clone(item),
        OffsetBuffer::new(ScalarBuffer::from(offsets)),
        values,
        validity(valid),
    )?))
}

/// The bounded batch reader over a stream of values.
struct Writer<I> {
    source: I,
    shape: Shape,
    schema: SchemaRef,
    batch_row_size: usize,
    batch_byte_size: Option<u64>,
    ordinal: u64,
    pending_error: Option<ArrowError>,
    done: bool,
}

impl<I> Iterator for Writer<I>
where
    I: Iterator,
    I::Item: Into<Result<MarketData>>,
{
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(error) = self.pending_error.take() {
            self.done = true;
            return Some(Err(error));
        }
        if self.done {
            return None;
        }
        let mut values = Vec::new();
        let mut charged = 0_u64;
        while values.len() < self.batch_row_size {
            if self
                .batch_byte_size
                .is_some_and(|bound| !values.is_empty() && charged >= bound)
            {
                break;
            }
            let Some(item) = self.source.next() else {
                self.done = true;
                break;
            };
            let ordinal = self.ordinal;
            match item
                .into()
                .and_then(|value| checked(&value, ordinal).map(|()| value))
            {
                Ok(value) => {
                    self.ordinal += 1;
                    charged += charge(&value);
                    values.push(value);
                }
                Err(error) => {
                    let error = external(error);
                    if values.is_empty() {
                        self.done = true;
                        return Some(Err(error));
                    }
                    self.pending_error = Some(error);
                    break;
                }
            }
        }
        if values.is_empty() {
            return None;
        }
        let rows: Vec<Row<'_>> = values.iter().map(Row::of).collect();
        let batch = self.shape.batch(&self.schema, &rows);
        if batch.is_err() {
            self.done = true;
            self.pending_error = None;
        }
        Some(batch.map_err(external))
    }
}

impl<I> arrow_array::RecordBatchReader for Writer<I>
where
    I: Iterator,
    I::Item: Into<Result<MarketData>>,
{
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

fn external(error: Error) -> ArrowError {
    ArrowError::ExternalError(Box::new(error))
}

// ------------------------------------------------------------------------
// The write check: a value is written only as its canonical self
// ------------------------------------------------------------------------

fn checked(value: &MarketData, ordinal: u64) -> Result<()> {
    let root = Path::root();
    let here = root.child(Segment::Index(ordinal as usize));
    match value {
        MarketData::Order(element) => validate_element_for_write(element, &here),
        MarketData::Quote(element) => validate_element_for_write(element, &here),
        MarketData::Execution(element) => validate_element_for_write(element, &here),
        MarketData::BookSide(side) => validate_side_for_write(side, &here).map(drop),
        MarketData::OrderEvent(operation) => validate_operation_for_write(operation, &here),
        MarketData::QuoteEvent(operation) => validate_operation_for_write(operation, &here),
        MarketData::ExecutionEvent(operation) => validate_operation_for_write(operation, &here),
        MarketData::TradeEvent(trade) => validate_trade_for_write(trade, &here),
        MarketData::BookEvent(book) => validate_book_for_write(book, &here),
        MarketData::SnapshotEvent(control) => validate_control_for_write(control, &here),
    }
}

fn validate_element_for_write<K: OperationKind>(
    element: &OperationElement<K>,
    path: &Path<'_>,
) -> Result<()> {
    let mut canonical = element.clone();
    canonical.finalize();
    if element == &canonical {
        return Ok(());
    }
    IdentityClaims::from_element(element).validate(&canonical, path)?;
    // An element's facts are its market and operation columns; the
    // identities the claims hold.
    first_market_difference(element, &canonical, path)?;
    first_operation_difference(element, &canonical, path)
}

fn validate_operation_for_write<K: OperationKind>(
    operation: &OperationEvent<K>,
    path: &Path<'_>,
) -> Result<()> {
    let mut canonical = operation.clone();
    canonical.finalize();
    if operation == &canonical {
        return Ok(());
    }
    IdentityClaims::from_element(operation).validate(&canonical, path)?;
    let mut stated = operation.facts().clone();
    normalize_identity(&mut stated, canonical.facts());
    validate_operation_facts(&stated, canonical.facts(), path)
}

fn validate_control_for_write(control: &SnapshotEvent, path: &Path<'_>) -> Result<()> {
    let mut canonical = control.event().clone();
    canonical.finalize();
    if control.event() == &canonical {
        return Ok(());
    }
    IdentityClaims::from_element(control.event()).validate(&canonical, path)?;
    let mut stated = control.event().clone();
    normalize_identity(&mut stated, &canonical);
    validate_market_facts(&stated, &canonical, path)
}

fn validate_trade_for_write(trade: &TradeEvent, path: &Path<'_>) -> Result<()> {
    let executions = path.field(EXECUTIONS);
    for (index, execution) in trade.executions().iter().enumerate() {
        validate_operation_for_write(execution, &executions.child(Segment::Index(index)))?;
    }
    let canonical = TradeEvent::from_facts(trade.data().clone(), trade.executions().to_vec())
        .map_err(|error| prefix_invalid(error, path))?;
    if trade.data() == canonical.data() {
        return Ok(());
    }
    IdentityClaims::from_element(trade).validate(&canonical, path)?;
    let mut stated = trade.data().clone();
    normalize_identity(&mut stated, canonical.data());
    validate_operation_facts(&stated, canonical.data(), path)
}

fn validate_side_for_write(side: &BookSide, path: &Path<'_>) -> Result<MarketFacts> {
    let entries = |list: &'static str, entries: &mut dyn Iterator<Item = &MarketData>| {
        let list = path.field(list);
        for (index, entry) in entries.enumerate() {
            let item = list.child(Segment::Index(index));
            match entry {
                MarketData::OrderEvent(operation) => {
                    validate_operation_for_write(operation, &item)?
                }
                MarketData::QuoteEvent(operation) => {
                    validate_operation_for_write(operation, &item)?
                }
                other => {
                    return Err(invalid(
                        at(&item, KIND),
                        format_smolstr!(
                            "expected order_event or quote_event on a book side, got {}",
                            other.kind().as_str()
                        ),
                    ));
                }
            }
        }
        Ok(())
    };
    entries(LIVE, &mut side.live())?;
    entries(DELTAS, &mut side.deltas().iter())?;
    let canonical = side
        .canonical_element()
        .map_err(|error| prefix_invalid(error, path))?;
    if side.element() == &canonical {
        return Ok(canonical);
    }
    IdentityClaims::from_element(side).validate(&canonical, path)?;
    let mut stated: MarketFacts = side.element().clone();
    normalize_identity(&mut stated, &canonical);
    validate_market_facts(&stated, &canonical, path)?;
    Ok(canonical)
}

fn validate_book_for_write(book: &BookEvent, path: &Path<'_>) -> Result<()> {
    let executions = path.field(EXECUTIONS);
    for (index, execution) in book.executions().iter().enumerate() {
        validate_operation_for_write(execution, &executions.child(Segment::Index(index)))?;
    }
    book.validate_parts()
        .map_err(|error| prefix_invalid(error, path))?;
    let bid = validate_side_for_write(book.bid(), &path.field(BIDSIDE))?;
    let ask = validate_side_for_write(book.ask(), &path.field(ASKSIDE))?;
    let canonical = book.canonical_event(&bid, &ask);
    if book.event() == &canonical {
        return Ok(());
    }
    IdentityClaims::from_element(book).validate(&canonical, path)?;
    let mut stated: MarketEventFacts = book.event().clone();
    normalize_identity(&mut stated, &canonical);
    validate_market_facts(&stated, &canonical, path)
}

/// What a row says about one identity: nothing, where it has no column
/// for it; a null, which an identity never is; or a value.
#[derive(Clone, Copy)]
enum Claim<T> {
    Absent,
    Null,
    Stated(T),
}

impl<T> Claim<T> {
    /// The claim a present column's cell makes.
    fn of(cell: Option<T>) -> Self {
        cell.map_or(Self::Null, Self::Stated)
    }
}

/// The identities a row states, each claimed where its column stands.
#[derive(Clone, Copy)]
struct IdentityClaims {
    curruuid: Claim<Uuid>,
    crossuuid: Claim<Uuid>,
    currhashcode: Claim<u64>,
    crosshashcode: Claim<u64>,
}

impl Default for IdentityClaims {
    fn default() -> Self {
        Self {
            curruuid: Claim::Absent,
            crossuuid: Claim::Absent,
            currhashcode: Claim::Absent,
            crosshashcode: Claim::Absent,
        }
    }
}

impl IdentityClaims {
    fn from_element(element: &(impl Element + ?Sized)) -> Self {
        Self {
            curruuid: Claim::Stated(element.get_curruuid()),
            crossuuid: Claim::Stated(element.get_crossuuid()),
            currhashcode: Claim::Stated(element.get_currhashcode()),
            crosshashcode: Claim::Stated(element.get_crosshashcode()),
        }
    }

    /// Every stated identity must be the one `canonical` derives, and a
    /// column that stands for one must state it: an identity is never
    /// null.
    fn validate<E: Element + ?Sized>(self, canonical: &E, path: &Path<'_>) -> Result<()> {
        fn check<T: PartialEq + std::fmt::Debug>(
            stated: Claim<T>,
            derived: T,
            path: &Path<'_>,
            name: &str,
        ) -> Result<()> {
            match stated {
                Claim::Stated(stated) if stated != derived => Err(invalid(
                    at(path, name),
                    format_smolstr!(
                        "expected the value derived from the row {derived:?}, got {stated:?}"
                    ),
                )),
                Claim::Null => Err(invalid(at(path, name), "expected a non-null identity fact")),
                _ => Ok(()),
            }
        }
        check(self.curruuid, canonical.get_curruuid(), path, "curruuid")?;
        check(self.crossuuid, canonical.get_crossuuid(), path, "crossuuid")?;
        check(
            self.currhashcode,
            canonical.get_currhashcode(),
            path,
            "currhashcode",
        )?;
        check(
            self.crosshashcode,
            canonical.get_crosshashcode(),
            path,
            "crosshashcode",
        )
    }
}

fn normalize_identity<E: Element + ?Sized, C: Element + ?Sized>(value: &mut E, canonical: &C) {
    value.set_curruuid(canonical.get_curruuid());
    value.set_crossuuid(canonical.get_crossuuid());
    value.set_currhashcode(canonical.get_currhashcode());
    value.set_crosshashcode(canonical.get_crosshashcode());
}

/// A stated market value must be its canonical self, named by the first
/// market column that differs.
fn validate_market_facts<E: Market + PartialEq>(
    stated: &E,
    canonical: &E,
    path: &Path<'_>,
) -> Result<()> {
    if stated == canonical {
        return Ok(());
    }
    first_market_difference(stated, canonical, path)?;
    Err(invalid(
        at(path, "currhashcode"),
        "value differs from its canonical finalized value",
    ))
}

/// [`validate_market_facts`] over an operation, its operation columns too.
fn validate_operation_facts<E: Market + Operation + PartialEq>(
    stated: &E,
    canonical: &E,
    path: &Path<'_>,
) -> Result<()> {
    if stated == canonical {
        return Ok(());
    }
    first_market_difference(stated, canonical, path)?;
    first_operation_difference(stated, canonical, path)?;
    Err(invalid(
        at(path, "currhashcode"),
        "value differs from its canonical finalized value",
    ))
}

fn first_market_difference<E: Market>(stated: &E, canonical: &E, path: &Path<'_>) -> Result<()> {
    for column in MarketColumn::ALL {
        let held = column.fact(stated);
        let derived = column.fact(canonical);
        if held != derived {
            return Err(differs(path, column.name(), &derived, &held));
        }
    }
    Ok(())
}

fn first_operation_difference<E: Operation>(
    stated: &E,
    canonical: &E,
    path: &Path<'_>,
) -> Result<()> {
    for column in OperationColumn::ALL {
        let held = column.fact(stated);
        let derived = column.fact(canonical);
        if held != derived {
            return Err(differs(path, column.name(), &derived, &held));
        }
    }
    Ok(())
}

fn differs(
    path: &Path<'_>,
    name: &str,
    derived: &dyn std::fmt::Debug,
    stated: &dyn std::fmt::Debug,
) -> Error {
    invalid(
        at(path, name),
        format_smolstr!("expected the value derived from the row {derived:?}, got {stated:?}"),
    )
}

// ------------------------------------------------------------------------
// Reading
// ------------------------------------------------------------------------

/// The storage a landed column holds, proven once per batch.
#[derive(Clone, Copy, Debug)]
enum Storage {
    Clock,
    Uuid,
    Uuids,
    UInt64,
    UInt32,
    Int32,
    Boolean,
    Decimal,
    Text,
    /// A registered code: the text its storage holds, built into the
    /// code's value by the code's own constructor.
    Code(Code),
    /// A `map<utf8, utf8>`: identifiers, securities, metadata.
    Pairs,
    /// A lane struct.
    Lane,
    /// A nested column, which no leaf holds.
    Nested,
}

/// The registered codes a row states, each riding its own code storage.
#[derive(Clone, Copy, Debug)]
enum Code {
    State,
    Ccy,
    Unit,
    Side,
    Cfi,
    Mic,
    TimeInForce,
}

impl Code {
    /// The text storage a landed column of this code holds.
    fn storage(self, serie: &Serie) -> Option<&Arc<Utf8StringSerie>> {
        match (self, serie) {
            (Self::State, Serie::State(held))
            | (Self::Ccy, Serie::Ccy(held))
            | (Self::Unit, Serie::Unit(held))
            | (Self::Side, Serie::Side(held))
            | (Self::Cfi, Serie::CfiCode(held))
            | (Self::Mic, Serie::MicCode(held))
            | (Self::TimeInForce, Serie::TimeInForce(held)) => Some(held),
            _ => None,
        }
    }
}

/// One landed column, narrowed to its storage leaf.
enum Leaf {
    Clock(Arc<DateTimeNanosecondSerie>),
    Uuid(Arc<FixedBytesSerie>),
    Uuids(Arc<SerieSerie>, Arc<FixedBytesSerie>),
    UInt64(Arc<UInt64Serie>),
    UInt32(Arc<UInt32Serie>),
    Int32(Arc<Int32Serie>),
    Boolean(Arc<BooleanSerie>),
    Decimal(Arc<Decimal128Serie>),
    Text(Arc<Utf8StringSerie>),
    /// A code's text storage.
    Code(Arc<Utf8StringSerie>),
    Pairs(Arc<MapSerie>, Arc<Utf8StringSerie>, Arc<Utf8StringSerie>),
    /// The lane struct, its four decimals and its two codes.
    Lane(
        Serie,
        [Arc<Decimal128Serie>; 4],
        Arc<Utf8StringSerie>,
        Arc<Utf8StringSerie>,
    ),
}

impl Leaf {
    fn of(column: Column, serie: &Serie) -> Result<Self> {
        let storage = column.storage();
        Ok(match (storage, serie) {
            (Storage::Clock, Serie::DateTimeNanosecond(held)) => Self::Clock(Arc::clone(held)),
            (Storage::Uuid, Serie::Uuid(held)) => Self::Uuid(Arc::clone(held)),
            (Storage::Uuids, Serie::Serie(list)) => match list.items() {
                Serie::Uuid(items) => Self::Uuids(Arc::clone(list), Arc::clone(items)),
                _ => return Err(unlanded(column, storage)),
            },
            (Storage::UInt64, Serie::UInt64(held)) => Self::UInt64(Arc::clone(held)),
            (Storage::UInt32, Serie::UInt32(held)) => Self::UInt32(Arc::clone(held)),
            (Storage::Int32, Serie::Int32(held)) => Self::Int32(Arc::clone(held)),
            (Storage::Boolean, Serie::Boolean(held)) => Self::Boolean(Arc::clone(held)),
            (Storage::Decimal, Serie::Decimal128(held)) => Self::Decimal(Arc::clone(held)),
            (Storage::Text, Serie::Utf8String(held)) => Self::Text(Arc::clone(held)),
            (Storage::Code(code), held) => match code.storage(held) {
                Some(held) => Self::Code(Arc::clone(held)),
                None => return Err(unlanded(column, storage)),
            },
            (Storage::Pairs, Serie::SortedMap(map) | Serie::Map(map)) => {
                match (map.keys(), map.values()) {
                    (Serie::Utf8String(keys), Serie::Utf8String(values)) => {
                        Self::Pairs(Arc::clone(map), Arc::clone(keys), Arc::clone(values))
                    }
                    _ => return Err(unlanded(column, storage)),
                }
            }
            (Storage::Lane, lane @ Serie::Struct(_)) => match lane.children() {
                [
                    Serie::Decimal128(price),
                    Serie::Decimal128(spotrate),
                    Serie::Decimal128(forwardpoints),
                    Serie::Ccy(currency),
                    Serie::Decimal128(quantity),
                    Serie::Unit(unit),
                ] => Self::Lane(
                    lane.clone(),
                    [
                        Arc::clone(price),
                        Arc::clone(spotrate),
                        Arc::clone(forwardpoints),
                        Arc::clone(quantity),
                    ],
                    Arc::clone(currency),
                    Arc::clone(unit),
                ),
                _ => return Err(unlanded(column, storage)),
            },
            _ => return Err(unlanded(column, storage)),
        })
    }

    fn clock(&self, row: usize) -> Option<i64> {
        match self {
            Self::Clock(held) => held.value(row),
            _ => None,
        }
    }

    fn uuid(&self, row: usize) -> Option<Uuid> {
        match self {
            Self::Uuid(held) => held
                .value(row)
                .and_then(|bytes| Uuid::from_bytes(bytes).ok()),
            _ => None,
        }
    }

    /// The identities one `serie<uuid>` cell states; `None` for a null.
    fn uuids(&self, row: usize) -> Option<impl Iterator<Item = Uuid> + '_> {
        match self {
            Self::Uuids(list, items) => list.range(row).map(|range| {
                range.filter_map(|at| {
                    items
                        .value(at)
                        .and_then(|bytes| Uuid::from_bytes(bytes).ok())
                })
            }),
            _ => None,
        }
    }

    fn u64(&self, row: usize) -> Option<u64> {
        match self {
            Self::UInt64(held) => held.value(row),
            _ => None,
        }
    }

    fn u32(&self, row: usize) -> Option<u32> {
        match self {
            Self::UInt32(held) => held.value(row),
            _ => None,
        }
    }

    fn i32(&self, row: usize) -> Option<i32> {
        match self {
            Self::Int32(held) => held.value(row),
            _ => None,
        }
    }

    fn boolean(&self, row: usize) -> Option<bool> {
        match self {
            Self::Boolean(held) => held.value(row),
            _ => None,
        }
    }

    fn decimal(&self, row: usize) -> Option<Decimal18> {
        match self {
            Self::Decimal(held) => held.value(row).and_then(Decimal18::from_units),
            _ => None,
        }
    }

    fn text(&self, row: usize) -> Option<&str> {
        match self {
            Self::Text(held) | Self::Code(held) => held.value(row),
            _ => None,
        }
    }

    /// The code one cell states, built by the code's own constructor;
    /// `None` for a null.
    fn code<T>(
        &self,
        row: usize,
        path: &Path<'_>,
        name: &str,
        build: fn(&str) -> Result<T>,
    ) -> Result<Option<T>> {
        self.text(row)
            .map(build)
            .transpose()
            .map_err(|error| invalid(at(path, name), format_smolstr!("{error}")))
    }

    /// The entries one map cell states, a null value passed over; `None`
    /// for a null.
    fn pairs(&self, row: usize) -> Option<impl Iterator<Item = (&str, &str)> + '_> {
        match self {
            Self::Pairs(map, keys, values) => map
                .range(row)
                .map(|range| range.filter_map(|at| Some((keys.value(at)?, values.value(at)?)))),
            _ => None,
        }
    }

    /// The lane one struct cell states; `None` for a null or a lane
    /// stating nothing.
    fn lane(&self, row: usize, path: &Path<'_>, name: &str) -> Result<Option<Lane>> {
        let Self::Lane(lane, [price, spotrate, forwardpoints, quantity], currency, unit) = self
        else {
            return Ok(None);
        };
        if lane.is_null(row).unwrap_or(true) {
            return Ok(None);
        }
        let decimal = |held: &Decimal128Serie| held.value(row).and_then(Decimal18::from_units);
        let located = |error: Error| invalid(at(path, name), format_smolstr!("{error}"));
        Ok(Lane {
            price: decimal(price),
            spotrate: decimal(spotrate),
            forwardpoints: decimal(forwardpoints),
            currency: currency
                .value(row)
                .map(Ccy::new)
                .transpose()
                .map_err(located)?,
            quantity: decimal(quantity),
            unit: unit
                .value(row)
                .map(Unit::new)
                .transpose()
                .map_err(located)?,
        }
        .stated())
    }
}

fn unlanded(column: Column, storage: Storage) -> Error {
    invalid(
        format_smolstr!("$.{}", column.name()),
        format_smolstr!("expected {storage:?} storage for the column, got another"),
    )
}

/// Which columns a landed struct holds, and where: resolved once per stream.
struct Layouts {
    root: Vec<(Column, usize)>,
    operation: Vec<(Column, usize)>,
    side: Vec<(Column, usize)>,
}

impl Layouts {
    fn new(root: Vec<Column>) -> Self {
        let indexed = |columns: Vec<Column>| {
            columns
                .into_iter()
                .enumerate()
                .map(|(at, column)| (column, at))
                .collect()
        };
        Self {
            root: indexed(root),
            operation: indexed(operation_columns()),
            side: indexed(side_columns()),
        }
    }
}

/// One landed struct - a batch's root, a list's items or a side - its
/// columns narrowed once to their leaves.
struct Landed {
    kind: Option<Leaf>,
    event: [Option<Leaf>; 16],
    market: [Option<Leaf>; 19],
    operation: [Option<Leaf>; 8],
    control: [Option<Leaf>; 5],
    executions: Option<Operations>,
    live: Option<Operations>,
    deltas: Option<Operations>,
    bidside: Option<LandedSide>,
    askside: Option<LandedSide>,
    partitions: Option<Partitions>,
}

/// A landed list of operation rows.
struct Operations {
    list: Arc<SerieSerie>,
    items: Box<Landed>,
}

/// A landed side struct: its validity, and its columns.
struct LandedSide {
    column: Serie,
    landed: Box<Landed>,
}

/// The landed snapshot partitions.
struct Partitions {
    list: Arc<SerieSerie>,
    symbol: Leaf,
    scope: Leaf,
}

impl Landed {
    fn new(children: &[Serie], layout: &[(Column, usize)], layouts: &Layouts) -> Result<Self> {
        let mut landed = Self {
            kind: None,
            event: std::array::from_fn(|_| None),
            market: std::array::from_fn(|_| None),
            operation: std::array::from_fn(|_| None),
            control: std::array::from_fn(|_| None),
            executions: None,
            live: None,
            deltas: None,
            bidside: None,
            askside: None,
            partitions: None,
        };
        for (column, at) in layout {
            let serie = &children[*at];
            match column {
                Column::Kind => landed.kind = Some(Leaf::of(*column, serie)?),
                Column::Event(held) => {
                    landed.event[position(&EventColumn::ALL, held)] =
                        Some(Leaf::of(*column, serie)?);
                }
                Column::Market(held) => {
                    landed.market[position(&MarketColumn::ALL, held)] =
                        Some(Leaf::of(*column, serie)?);
                }
                Column::Operation(held) => {
                    landed.operation[position(&OperationColumn::ALL, held)] =
                        Some(Leaf::of(*column, serie)?);
                }
                Column::Control(held) => {
                    landed.control[position(&Control::ALL, held)] = Some(Leaf::of(*column, serie)?);
                }
                Column::Executions => {
                    landed.executions = Some(Operations::new(*column, serie, layouts)?)
                }
                Column::Live => landed.live = Some(Operations::new(*column, serie, layouts)?),
                Column::Deltas => landed.deltas = Some(Operations::new(*column, serie, layouts)?),
                Column::BidSide => landed.bidside = Some(LandedSide::new(*column, serie, layouts)?),
                Column::AskSide => landed.askside = Some(LandedSide::new(*column, serie, layouts)?),
                Column::SnapshotPartitions => {
                    landed.partitions = Some(Partitions::new(*column, serie)?);
                }
            }
        }
        Ok(landed)
    }

    /// Root row `row` as the leaf its `kind` names.
    fn value(&self, row: usize, path: &Path<'_>) -> Result<MarketData> {
        match self.kind(row, path)? {
            MarketKind::Order => self.element::<OrderKind>(row, path).map(MarketData::from),
            MarketKind::Quote => self.element::<QuoteKind>(row, path).map(MarketData::from),
            MarketKind::Execution => self
                .element::<ExecutionKind>(row, path)
                .map(MarketData::from),
            MarketKind::BookSide => self.book_side(row, path).map(MarketData::from),
            MarketKind::OrderEvent => self
                .operation_event::<OrderKind>(row, path)
                .map(MarketData::from),
            MarketKind::QuoteEvent => self
                .operation_event::<QuoteKind>(row, path)
                .map(MarketData::from),
            MarketKind::ExecutionEvent => self
                .operation_event::<ExecutionKind>(row, path)
                .map(MarketData::from),
            MarketKind::TradeEvent => self.trade(row, path).map(MarketData::from),
            MarketKind::BookEvent => self.book(row, path).map(MarketData::from),
            MarketKind::SnapshotEvent => self.snapshot(row, path).map(MarketData::from),
        }
    }

    fn kind(&self, row: usize, path: &Path<'_>) -> Result<MarketKind> {
        let Some(text) = self.kind_text(row) else {
            return Err(invalid(
                at(path, KIND),
                format_smolstr!("expected {KINDS}, got null"),
            ));
        };
        MarketKind::read(text).ok_or_else(|| {
            invalid(
                at(path, KIND),
                format_smolstr!("expected {KINDS}, got {text:?}"),
            )
        })
    }

    fn kind_text(&self, row: usize) -> Option<&str> {
        self.kind.as_ref().and_then(|leaf| leaf.text(row))
    }

    /// The instant a dated leaf requires.
    fn currunix(&self, row: usize, path: &Path<'_>) -> Result<i64> {
        self.event[0]
            .as_ref()
            .and_then(|leaf| leaf.clock(row))
            .ok_or_else(|| {
                invalid(
                    at(path, EventColumn::CurrUnix.name()),
                    "expected the instant a dated leaf happened at, got null",
                )
            })
    }

    fn element<K: OperationKind>(
        &self,
        row: usize,
        path: &Path<'_>,
    ) -> Result<OperationElement<K>> {
        let mut element = OperationElement::<K>::new();
        let mut claims = IdentityClaims::default();
        self.read_element(row, &mut element, &mut claims);
        self.read_market(row, &mut element, path)?;
        self.read_operation(row, &mut element, path)?;
        element.finalize();
        claims.validate(&element, path)?;
        self.check_element(row, &element, path)?;
        self.check_market(row, &element, path)?;
        self.check_operation(row, &element, path)?;
        Ok(element)
    }

    fn operation_event<K: OperationKind>(
        &self,
        row: usize,
        path: &Path<'_>,
    ) -> Result<OperationEvent<K>> {
        let mut operation = OperationEvent::<K>::default();
        let claims = self.read_operation_event(row, &mut operation, path)?;
        operation.set_book(self.control(row));
        operation.finalize();
        self.check_operation_event(row, &operation, claims, path)?;
        Ok(operation)
    }

    /// Every fact a dated operation row states, onto `target`.
    fn read_operation_event<E: Event + Market + Operation + ?Sized>(
        &self,
        row: usize,
        target: &mut E,
        path: &Path<'_>,
    ) -> Result<IdentityClaims> {
        let mut claims = IdentityClaims::default();
        target.set_currunix(self.currunix(row, path)?);
        self.read_event(row, target, &mut claims, path)?;
        self.read_market(row, target, path)?;
        self.read_operation(row, target, path)?;
        Ok(claims)
    }

    fn check_operation_event<E: Event + Market + Operation + ?Sized>(
        &self,
        row: usize,
        canonical: &E,
        claims: IdentityClaims,
        path: &Path<'_>,
    ) -> Result<()> {
        claims.validate(canonical, path)?;
        self.check_event(row, canonical, path)?;
        self.check_market(row, canonical, path)?;
        self.check_operation(row, canonical, path)
    }

    fn trade(&self, row: usize, path: &Path<'_>) -> Result<TradeEvent> {
        let mut data = OperationEventFacts::default();
        let claims = self.read_operation_event(row, &mut data, path)?;
        let Some(executions) = self.executions(row, path)? else {
            return Err(invalid(
                at(path, EXECUTIONS),
                "expected the executions of a trade_event, got null",
            ));
        };
        let trade = TradeEvent::from_facts(data, executions)
            .map_err(|error| prefix_invalid(error, path))?;
        self.check_operation_event(row, &trade, claims, path)?;
        Ok(trade)
    }

    fn snapshot(&self, row: usize, path: &Path<'_>) -> Result<SnapshotEvent> {
        let mut event = MarketEventFacts::default();
        let mut claims = IdentityClaims::default();
        event.set_currunix(self.currunix(row, path)?);
        self.read_event(row, &mut event, &mut claims, path)?;
        self.read_market(row, &mut event, path)?;
        let control = SnapshotEvent::from_control(event, self.control(row).unwrap_or_default());
        claims.validate(&control, path)?;
        self.check_event(row, &control, path)?;
        self.check_market(row, &control, path)?;
        Ok(control)
    }

    fn book(&self, row: usize, path: &Path<'_>) -> Result<BookEvent> {
        let mut event = MarketEventFacts::default();
        let mut claims = IdentityClaims::default();
        event.set_currunix(self.currunix(row, path)?);
        self.read_event(row, &mut event, &mut claims, path)?;
        self.read_market(row, &mut event, path)?;
        let bid = Self::side_of(self.bidside.as_ref(), BIDSIDE, row, path)?;
        let ask = Self::side_of(self.askside.as_ref(), ASKSIDE, row, path)?;
        let executions = self.executions(row, path)?.unwrap_or_default();
        let snapshots = self.partitions(row, path)?;
        let book = BookEvent::from_parts(event, bid, ask, executions, snapshots)
            .map_err(|error| prefix_invalid(error, path))?;
        claims.validate(&book, path)?;
        self.check_event(row, &book, path)?;
        self.check_market(row, &book, path)?;
        Ok(book)
    }

    fn side_of(
        side: Option<&LandedSide>,
        name: &'static str,
        row: usize,
        path: &Path<'_>,
    ) -> Result<BookSide> {
        let here = path.field(name);
        let side = side.filter(|side| !side.column.is_null(row).unwrap_or(true));
        match side {
            Some(side) => side.landed.book_side(row, &here),
            None => Err(invalid(
                here.render(),
                "expected a side of a book_event, got null",
            )),
        }
    }

    /// Row `row` as a book side: its element and market facts, and the
    /// operations its `live` and `deltas` lists hold.
    fn book_side(&self, row: usize, path: &Path<'_>) -> Result<BookSide> {
        let mut element = MarketFacts::default();
        let mut claims = IdentityClaims::default();
        self.read_element(row, &mut element, &mut claims);
        self.read_market(row, &mut element, path)?;
        let live = Self::entries(self.live.as_ref(), LIVE, row, path)?;
        let deltas = Self::entries(self.deltas.as_ref(), DELTAS, row, path)?;
        let side = BookSide::from_parts(element, live, deltas)
            .map_err(|error| prefix_invalid(error, path))?;
        claims.validate(&side, path)?;
        self.check_element(row, &side, path)?;
        self.check_market(row, &side, path)?;
        Ok(side)
    }

    /// The orders and quotes one side list holds for row `row`: none where
    /// it states none.
    fn entries(
        list: Option<&Operations>,
        name: &'static str,
        row: usize,
        path: &Path<'_>,
    ) -> Result<Vec<MarketData>> {
        let Some((list, range)) = list.and_then(|list| Some((list, list.list.range(row)?))) else {
            return Ok(Vec::new());
        };
        let here = path.field(name);
        range
            .enumerate()
            .map(|(index, at)| {
                let item = here.child(Segment::Index(index));
                match list.items.kind(at, &item)? {
                    MarketKind::OrderEvent => list
                        .items
                        .operation_event::<OrderKind>(at, &item)
                        .map(MarketData::from),
                    MarketKind::QuoteEvent => list
                        .items
                        .operation_event::<QuoteKind>(at, &item)
                        .map(MarketData::from),
                    other => Err(invalid(
                        self::at(&item, KIND),
                        format_smolstr!(
                            "expected order_event or quote_event on a book side, got {}",
                            other.as_str()
                        ),
                    )),
                }
            })
            .collect()
    }

    /// The executions row `row` states; `None` where it states none.
    fn executions(&self, row: usize, path: &Path<'_>) -> Result<Option<Vec<ExecutionEvent>>> {
        let Some((list, range)) = self
            .executions
            .as_ref()
            .and_then(|list| Some((list, list.list.range(row)?)))
        else {
            return Ok(None);
        };
        let here = path.field(EXECUTIONS);
        range
            .enumerate()
            .map(|(index, at)| {
                let item = here.child(Segment::Index(index));
                // A nested execution states its kind or leaves it to the
                // list it stands in.
                if let Some(text) = list.items.kind_text(at) {
                    if MarketKind::read(text) != Some(MarketKind::ExecutionEvent) {
                        return Err(invalid(
                            self::at(&item, KIND),
                            format_smolstr!("expected execution_event, got {text:?}"),
                        ));
                    }
                }
                list.items.operation_event::<ExecutionKind>(at, &item)
            })
            .collect::<Result<Vec<_>>>()
            .map(Some)
    }

    fn partitions(&self, row: usize, path: &Path<'_>) -> Result<BTreeSet<SnapshotPartition>> {
        let Some((partitions, range)) = self
            .partitions
            .as_ref()
            .and_then(|held| Some((held, held.list.range(row)?)))
        else {
            return Ok(BTreeSet::new());
        };
        let here = path.field(SNAPSHOT_PARTITIONS);
        range
            .enumerate()
            .map(|(index, at)| {
                let Some(scope) = partitions.scope.text(at) else {
                    return Err(invalid(
                        self::at(&here.child(Segment::Index(index)), "scope"),
                        "expected a non-null scope, got null",
                    ));
                };
                Ok(SnapshotPartition {
                    symbol: partitions.symbol.text(at).map(SmolStr::new),
                    scope: SmolStr::new(scope),
                })
            })
            .collect()
    }

    fn control(&self, row: usize) -> Option<BookRef> {
        let [action, scope, position, px, size] = &self.control;
        let book = BookRef {
            action: action
                .as_ref()
                .and_then(|leaf| leaf.text(row))
                .and_then(MdUpdateAction::read),
            scope: scope
                .as_ref()
                .and_then(|leaf| leaf.text(row))
                .map(SmolStr::new),
            position: position.as_ref().and_then(|leaf| leaf.u32(row)),
            entry_px: px.as_ref().and_then(|leaf| leaf.decimal(row)),
            entry_size: size.as_ref().and_then(|leaf| leaf.decimal(row)),
        };
        book.is_stated().then_some(book)
    }

    /// The element facts a row states - the identities, the cross code and
    /// the sources - onto `target`, each stated identity claimed.
    fn read_element<E: Element + ?Sized>(
        &self,
        row: usize,
        target: &mut E,
        claims: &mut IdentityClaims,
    ) {
        for (column, leaf) in EventColumn::ALL.into_iter().zip(&self.event) {
            let Some(leaf) = leaf else {
                continue;
            };
            match column {
                EventColumn::CurrUuid => {
                    let uuid = leaf.uuid(row);
                    if let Some(uuid) = uuid {
                        target.set_curruuid(uuid);
                    }
                    claims.curruuid = Claim::of(uuid);
                }
                EventColumn::CrossUuid => {
                    let uuid = leaf.uuid(row);
                    if let Some(uuid) = uuid {
                        target.set_crossuuid(uuid);
                    }
                    claims.crossuuid = Claim::of(uuid);
                }
                EventColumn::CrossCode => {
                    if let Some(code) = leaf.text(row).filter(|code| !code.is_empty()) {
                        target.set_crosscode(code.to_owned());
                    }
                }
                EventColumn::CurrHashCode => {
                    let code = leaf.u64(row);
                    if let Some(code) = code {
                        target.set_currhashcode(code);
                    }
                    claims.currhashcode = Claim::of(code);
                }
                EventColumn::CrossHashCode => {
                    let code = leaf.u64(row);
                    if let Some(code) = code {
                        target.set_crosshashcode(code);
                    }
                    claims.crosshashcode = Claim::of(code);
                }
                EventColumn::SrcUuids => {
                    if let Some(uuids) = leaf.uuids(row) {
                        target.set_srcuuids(uuids.collect());
                    }
                }
                _ => {}
            }
        }
    }

    /// Every event fact a row states onto `target` but the instant, which
    /// the caller reads first as the fact a dated leaf requires.
    fn read_event<E: Event + ?Sized>(
        &self,
        row: usize,
        target: &mut E,
        claims: &mut IdentityClaims,
        path: &Path<'_>,
    ) -> Result<()> {
        self.read_element(row, target, claims);
        for (column, leaf) in EventColumn::ALL.into_iter().zip(&self.event) {
            let Some(leaf) = leaf else {
                continue;
            };
            match column {
                EventColumn::CreaUnix => target.set_creaunix(leaf.clock(row)),
                EventColumn::ExecUnix => target.set_execunix(leaf.clock(row)),
                EventColumn::RecdUnix => target.set_recdunix(leaf.clock(row)),
                EventColumn::ExprTime => target.set_exprtime(leaf.clock(row)),
                EventColumn::PrevUnix => target.set_prevunix(leaf.clock(row)),
                EventColumn::SnapUnix => target.set_snapunix(leaf.clock(row)),
                EventColumn::PrevUuid => target.set_prevuuid(leaf.uuid(row)),
                EventColumn::SeqNum => target.set_seqnum(leaf.u64(row).unwrap_or(0)),
                EventColumn::State => {
                    if let Some(state) =
                        leaf.code(row, path, column.name(), |text| State::new(text))?
                    {
                        target.set_state(state);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn read_market<E: Market + ?Sized>(
        &self,
        row: usize,
        target: &mut E,
        path: &Path<'_>,
    ) -> Result<()> {
        for (column, leaf) in MarketColumn::ALL.into_iter().zip(&self.market) {
            let Some(leaf) = leaf else {
                continue;
            };
            if is_decimal(column) {
                set_market_decimal(column, target, leaf.decimal(row));
            } else if column == MarketColumn::Ticker {
                target.set_ticker(leaf.text(row).map(SmolStr::new));
            } else if column == MarketColumn::SecurityIds {
                let Some(pairs) = leaf.pairs(row) else {
                    continue;
                };
                let located = |error: Error| prefix_invalid(error, &path.field(column.name()));
                let mut ids = SecurityIds::default();
                for (key, code) in pairs {
                    ids.insert(
                        SecurityId::new(SecType::read(key).map_err(located)?, code)
                            .map_err(located)?,
                    );
                }
                target.set_securityids(ids).map_err(located)?;
            } else if column == MarketColumn::Metadata {
                if let Some(pairs) = leaf.pairs(row) {
                    // Inserted one by one: collecting stages the entries in
                    // a buffer the map does not keep.
                    let mut metadata = Metadata::new();
                    for (key, value) in pairs {
                        metadata.insert(SmolStr::new(key), SmolStr::new(value));
                    }
                    target.set_metadata(Some(metadata));
                }
            } else {
                let name = column.name();
                match column {
                    MarketColumn::Currency => {
                        if let Some(held) = leaf.code(row, path, name, |text| Ccy::new(text))? {
                            target.set_currency(held);
                        }
                    }
                    MarketColumn::Unit => {
                        if let Some(held) = leaf.code(row, path, name, |text| Unit::new(text))? {
                            target.set_unit(held);
                        }
                    }
                    MarketColumn::Side => {
                        if let Some(held) = leaf.code(row, path, name, |text| Side::new(text))? {
                            target.set_side(held);
                        }
                    }
                    MarketColumn::CfiCode => {
                        if let Some(held) = leaf.code(row, path, name, |text| CfiCode::new(text))? {
                            target.set_cficode(Some(held));
                        }
                    }
                    MarketColumn::MicCode => {
                        if let Some(held) = leaf.code(row, path, name, |text| MicCode::new(text))? {
                            target.set_miccode(Some(held));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn read_operation<E: Operation + ?Sized>(
        &self,
        row: usize,
        target: &mut E,
        path: &Path<'_>,
    ) -> Result<()> {
        for (column, leaf) in OperationColumn::ALL.into_iter().zip(&self.operation) {
            let Some(leaf) = leaf else {
                continue;
            };
            match column {
                OperationColumn::MarketOperationId => target.set_marketoperationid(leaf.i32(row)),
                OperationColumn::Tradable => target.set_tradable(leaf.boolean(row)),
                OperationColumn::AccountIds
                | OperationColumn::UserIds
                | OperationColumn::AltIds => {
                    let Some(pairs) = leaf.pairs(row) else {
                        continue;
                    };
                    let located = |error: Error| prefix_invalid(error, &path.field(column.name()));
                    let mut ids = IdMap::new();
                    for (key, value) in pairs {
                        ids.insert(key, value).map_err(located)?;
                    }
                    match column {
                        OperationColumn::AccountIds => target.set_accountids(ids),
                        OperationColumn::UserIds => target.set_userids(ids),
                        _ => target.set_altids(ids),
                    }
                    .map_err(located)?;
                }
                OperationColumn::Bid => {
                    if let Some(lane) = leaf.lane(row, path, column.name())? {
                        target.set_bid(Some(lane));
                    }
                }
                OperationColumn::Ask => {
                    if let Some(lane) = leaf.lane(row, path, column.name())? {
                        target.set_ask(Some(lane));
                    }
                }
                OperationColumn::TimeInForce => {
                    let held =
                        leaf.code(row, path, column.name(), |text| TimeInForce::new(text))?;
                    if held.is_some() {
                        target.set_tif(held);
                    }
                }
            }
        }
        Ok(())
    }

    /// Every element fact the row states but the identities, which the
    /// claims hold, must be the one `canonical` settled on.
    fn check_element<E: Element + ?Sized>(
        &self,
        row: usize,
        canonical: &E,
        path: &Path<'_>,
    ) -> Result<()> {
        if let Some(leaf) = &self.event[9] {
            if let Some(code) = leaf
                .text(row)
                .filter(|code| *code != canonical.get_crosscode())
            {
                return Err(differs(
                    path,
                    EventColumn::CrossCode.name(),
                    &canonical.get_crosscode(),
                    &code,
                ));
            }
        }
        if let Some(uuids) = self.event[14].as_ref().and_then(|leaf| leaf.uuids(row)) {
            if !uuids.eq(canonical.get_srcuuids().iter().copied()) {
                return Err(invalid(
                    at(path, EventColumn::SrcUuids.name()),
                    format_smolstr!(
                        "expected the value derived from the row {:?}",
                        canonical.get_srcuuids()
                    ),
                ));
            }
        }
        Ok(())
    }

    fn check_event<E: Event + ?Sized>(
        &self,
        row: usize,
        canonical: &E,
        path: &Path<'_>,
    ) -> Result<()> {
        self.check_element(row, canonical, path)?;
        for (column, leaf) in EventColumn::ALL.into_iter().zip(&self.event) {
            let Some(leaf) = leaf else {
                continue;
            };
            let clock = |derived: Option<i64>| match leaf.clock(row) {
                Some(stated) if Some(stated) != derived => {
                    Err(differs(path, column.name(), &derived, &stated))
                }
                _ => Ok(()),
            };
            match column {
                EventColumn::CurrUnix => clock(Some(canonical.get_currunix()))?,
                EventColumn::CreaUnix => clock(canonical.get_creaunix())?,
                EventColumn::ExecUnix => clock(canonical.get_execunix())?,
                EventColumn::RecdUnix => clock(canonical.get_recdunix())?,
                EventColumn::ExprTime => clock(canonical.get_exprtime())?,
                EventColumn::PrevUnix => clock(canonical.get_prevunix())?,
                EventColumn::SnapUnix => clock(canonical.get_snapunix())?,
                EventColumn::PrevUuid => match leaf.uuid(row) {
                    Some(stated) if Some(stated) != canonical.get_prevuuid() => {
                        return Err(differs(
                            path,
                            column.name(),
                            &canonical.get_prevuuid(),
                            &stated,
                        ));
                    }
                    _ => {}
                },
                EventColumn::SeqNum => match leaf.u64(row) {
                    Some(stated) if stated != canonical.get_seqnum() => {
                        return Err(differs(
                            path,
                            column.name(),
                            &canonical.get_seqnum(),
                            &stated,
                        ));
                    }
                    _ => {}
                },
                EventColumn::State => check_code(
                    leaf,
                    row,
                    Some(canonical.get_state()),
                    |text| State::new(text),
                    path,
                    column.name(),
                )?,
                _ => {}
            }
        }
        Ok(())
    }

    fn check_market<E: Market + ?Sized>(
        &self,
        row: usize,
        canonical: &E,
        path: &Path<'_>,
    ) -> Result<()> {
        for (column, leaf) in MarketColumn::ALL.into_iter().zip(&self.market) {
            let Some(leaf) = leaf else {
                continue;
            };
            if is_decimal(column) {
                let derived = market_decimal(column, canonical);
                match leaf.decimal(row) {
                    Some(stated) if Some(stated) != derived => {
                        return Err(differs(path, column.name(), &derived, &stated));
                    }
                    _ => {}
                }
            } else if column == MarketColumn::Ticker {
                match leaf.text(row) {
                    Some(stated) if Some(stated) != canonical.get_ticker() => {
                        return Err(differs(
                            path,
                            column.name(),
                            &canonical.get_ticker(),
                            &stated,
                        ));
                    }
                    _ => {}
                }
            } else if column == MarketColumn::SecurityIds {
                let derived = canonical.get_securityids();
                check_pairs(
                    leaf,
                    row,
                    derived.len(),
                    |key| derived.get(key),
                    path,
                    column.name(),
                    derived,
                )?;
            } else if column == MarketColumn::Metadata {
                let derived = canonical.get_metadata();
                check_pairs(
                    leaf,
                    row,
                    derived.len(),
                    |key| derived.get(key).map(SmolStr::as_str),
                    path,
                    column.name(),
                    derived,
                )?;
            } else {
                let name = column.name();
                match column {
                    MarketColumn::Currency => check_code(
                        leaf,
                        row,
                        Some(canonical.get_currency()),
                        |text| Ccy::new(text),
                        path,
                        name,
                    )?,
                    MarketColumn::Unit => check_code(
                        leaf,
                        row,
                        Some(canonical.get_unit()),
                        |text| Unit::new(text),
                        path,
                        name,
                    )?,
                    MarketColumn::Side => check_code(
                        leaf,
                        row,
                        Some(&canonical.get_side()),
                        |text| Side::new(text),
                        path,
                        name,
                    )?,
                    MarketColumn::CfiCode => check_code(
                        leaf,
                        row,
                        canonical.get_cficode(),
                        |text| CfiCode::new(text),
                        path,
                        name,
                    )?,
                    MarketColumn::MicCode => check_code(
                        leaf,
                        row,
                        canonical.get_miccode(),
                        |text| MicCode::new(text),
                        path,
                        name,
                    )?,
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn check_operation<E: Operation + ?Sized>(
        &self,
        row: usize,
        canonical: &E,
        path: &Path<'_>,
    ) -> Result<()> {
        for (column, leaf) in OperationColumn::ALL.into_iter().zip(&self.operation) {
            let Some(leaf) = leaf else {
                continue;
            };
            match column {
                OperationColumn::MarketOperationId => match leaf.i32(row) {
                    Some(stated) if Some(stated) != canonical.get_marketoperationid() => {
                        return Err(differs(
                            path,
                            column.name(),
                            &canonical.get_marketoperationid(),
                            &stated,
                        ));
                    }
                    _ => {}
                },
                OperationColumn::Tradable => match leaf.boolean(row) {
                    Some(stated) if Some(stated) != canonical.get_tradable() => {
                        return Err(differs(
                            path,
                            column.name(),
                            &canonical.get_tradable(),
                            &stated,
                        ));
                    }
                    _ => {}
                },
                OperationColumn::AccountIds
                | OperationColumn::UserIds
                | OperationColumn::AltIds => {
                    let derived = match column {
                        OperationColumn::AccountIds => canonical.get_accountids(),
                        OperationColumn::UserIds => canonical.get_userids(),
                        _ => canonical.get_altids(),
                    };
                    check_pairs(
                        leaf,
                        row,
                        derived.len(),
                        |key| derived.get(key),
                        path,
                        column.name(),
                        derived,
                    )?;
                }
                OperationColumn::Bid | OperationColumn::Ask => {
                    let derived = match column {
                        OperationColumn::Bid => canonical.get_bid(),
                        _ => canonical.get_ask(),
                    };
                    match leaf.lane(row, path, column.name())? {
                        Some(stated) if Some(&stated) != derived => {
                            return Err(differs(path, column.name(), &derived, &stated));
                        }
                        _ => {}
                    }
                }
                OperationColumn::TimeInForce => check_code(
                    leaf,
                    row,
                    canonical.get_tif(),
                    |text| TimeInForce::new(text),
                    path,
                    column.name(),
                )?,
            }
        }
        Ok(())
    }
}

/// A stated map must hold exactly the entries its canonical leaf holds,
/// each looked up by its own key rather than rebuilt.
fn check_pairs<'a>(
    leaf: &Leaf,
    row: usize,
    len: usize,
    derived: impl Fn(&str) -> Option<&'a str>,
    path: &Path<'_>,
    name: &str,
    held: &dyn std::fmt::Debug,
) -> Result<()> {
    let Some(pairs) = leaf.pairs(row) else {
        return Ok(());
    };
    let mut stated = 0;
    for (key, value) in pairs {
        stated += 1;
        if derived(key) != Some(value) {
            return Err(differs(path, name, held, &(key, value)));
        }
    }
    if stated != len {
        return Err(invalid(
            at(path, name),
            format_smolstr!(
                "expected the value derived from the row {held:?}, got {stated} entries"
            ),
        ));
    }
    Ok(())
}

/// A stated code must be the one its canonical leaf states: its text
/// compared first, and built only where the text differs, since a code
/// reads more spellings than the one it stores.
fn check_code<T: CodeValue + std::fmt::Debug>(
    leaf: &Leaf,
    row: usize,
    derived: Option<&T>,
    build: fn(&str) -> Result<T>,
    path: &Path<'_>,
    name: &str,
) -> Result<()> {
    let Some(text) = leaf.text(row) else {
        return Ok(());
    };
    if derived.is_some_and(|derived| derived.as_str() == text) {
        return Ok(());
    }
    let stated =
        build(text).map_err(|error| invalid(at(path, name), format_smolstr!("{error}")))?;
    if derived == Some(&stated) {
        return Ok(());
    }
    Err(differs(path, name, &derived, &stated))
}

fn position<T: PartialEq>(all: &[T], held: &T) -> usize {
    all.iter()
        .position(|column| column == held)
        .expect("a column of the enumeration it lists")
}

impl Operations {
    fn new(column: Column, serie: &Serie, layouts: &Layouts) -> Result<Self> {
        let Serie::Serie(list) = serie else {
            return Err(unlanded(column, Storage::Nested));
        };
        Ok(Self {
            list: Arc::clone(list),
            items: Box::new(Landed::new(
                list.items().children(),
                &layouts.operation,
                layouts,
            )?),
        })
    }
}

impl LandedSide {
    fn new(column: Column, serie: &Serie, layouts: &Layouts) -> Result<Self> {
        if serie.as_struct().is_none() {
            return Err(unlanded(column, Storage::Nested));
        }
        Ok(Self {
            column: serie.clone(),
            landed: Box::new(Landed::new(serie.children(), &layouts.side, layouts)?),
        })
    }
}

impl Partitions {
    fn new(column: Column, serie: &Serie) -> Result<Self> {
        let Serie::Serie(list) = serie else {
            return Err(unlanded(column, Storage::Nested));
        };
        let text = |at: usize| match list.items().children().get(at) {
            Some(Serie::Utf8String(held)) => Ok(Leaf::Text(Arc::clone(held))),
            _ => Err(unlanded(column, Storage::Text)),
        };
        Ok(Self {
            list: Arc::clone(list),
            symbol: text(0)?,
            scope: text(1)?,
        })
    }
}

/// The lazy, fused read of every row of every batch.
struct Rows {
    batches: SerieReader,
    layouts: Layouts,
    landed: Option<(Landed, usize)>,
    row: usize,
    ordinal: u64,
    done: bool,
}

impl Iterator for Rows {
    type Item = Result<MarketData>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            if let Some((landed, len)) = &self.landed {
                if self.row < *len {
                    let row = self.row;
                    self.row += 1;
                    let ordinal = self.ordinal;
                    self.ordinal += 1;
                    let root = Path::root();
                    let here = root.child(Segment::Index(ordinal as usize));
                    let result = landed.value(row, &here);
                    if result.is_err() {
                        self.done = true;
                        self.landed = None;
                    }
                    return Some(result);
                }
            }
            self.landed = None;
            self.row = 0;
            let batch = match self.batches.next() {
                Some(Ok(batch)) => batch,
                Some(Err(error)) => {
                    self.done = true;
                    return Some(Err(invalid(
                        format_smolstr!("$[{}]", self.ordinal),
                        format_smolstr!("{error}"),
                    )));
                }
                None => {
                    self.done = true;
                    return None;
                }
            };
            match Landed::new(batch.children(), &self.layouts.root, &self.layouts) {
                Ok(landed) => self.landed = Some((landed, batch.len())),
                Err(error) => {
                    self.done = true;
                    return Some(Err(error));
                }
            }
        }
    }
}

impl FusedIterator for Rows {}

/// `path`'s child `name`, rendered.
fn at(path: &Path<'_>, name: &str) -> SmolStr {
    SmolStr::from(path.field(name).render())
}

/// An error a leaf's own validation located under `$`, restated under
/// `path`.
fn prefix_invalid(error: Error, path: &Path<'_>) -> Error {
    match error {
        Error::InvalidRecord {
            path: inner,
            reason,
        } => {
            let suffix = inner.strip_prefix('$').unwrap_or(inner.as_str());
            Error::InvalidRecord {
                path: format_smolstr!("{}{suffix}", path.render()),
                reason,
            }
        }
        other => other,
    }
}

fn invalid(path: impl Into<SmolStr>, reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: path.into(),
        reason: reason.into(),
    }
}
