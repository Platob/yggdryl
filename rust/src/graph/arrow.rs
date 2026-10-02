//! The graph's Arrow rows: one lifted `marketdata` shape every
//! [`MarketData`] leaf is written in and read back from.
//!
//! A row is `marketdatakind` - the [`MarketDataKind`]
//! its leaf stands under - then the fifteen [`EventColumn`]s, the
//! twenty-eight [`MarketColumn`]s, the four [`OperationColumn`]s, the
//! `bookscope` a market-data entry states, and the nested columns a
//! composite leaf fills: `alive` and `deltas` (a book's entries, alive on
//! either side and applied since the book before it), `executions` (a
//! trade's or a book's), and `bidlimits` and `asklimits` - a book's two
//! sides as their price levels, one [`Limit`] each, best
//! first and the unpriced one last, an empty side an empty list. Every fact
//! is its own typed column; a leaf leaves null what it does not state. A
//! nested operation row is `marketdatakind`, the event, market and
//! operation columns and `bookscope`, and nests nothing.
//!
//! Written column by column from the typed leaves, and read back
//! tolerantly: the reader's columns are resolved by name once per stream,
//! any subset in any order, a foreign column ignored and a column of
//! another castable type cast through one plan. Every stated identity and
//! every stated fact - a book's price levels included - must be the one the
//! rebuilt leaf derives, and a null cell states nothing.
//!
//! The leaf a row is read back as is its `marketdatakind` and its shape:
//! an undated `ORDR`, `QUOT` or `EXEC` row is an order, a quote or an
//! execution, a dated one the event; a `TRAD` row is a trade and a `BOOK`
//! row a book or a snapshot control, and both must be dated - a dated
//! `BOOK` row is a book where it states its `alive` entries, even none, and
//! a snapshot control where its `alive` cell is null, or holds no entry
//! beside the `curruuid` a snapshot control derives: a table may store a
//! null list as an empty one.

use std::iter::FusedIterator;
use std::sync::Arc;

use arrow_array::builder::{ArrayBuilder, StringBuilder};
use arrow_array::{
    ArrayRef, BooleanArray, Decimal128Array, FixedSizeBinaryArray, ListArray, MapArray,
    RecordBatch, RecordBatchOptions, StringArray, StructArray, TimestampNanosecondArray,
    UInt8Array, UInt16Array, UInt64Array,
};
use arrow_buffer::{NullBuffer, OffsetBuffer, ScalarBuffer};
use arrow_schema::{ArrowError, DataType as ArrowType, FieldRef, Fields, SchemaRef};
use smol_str::{SmolStr, format_smolstr};

use super::facts::{MarketEventFacts, OperationEventFacts};
use super::{
    BookEvent, BookRef, Element, ElementColumn, Event, EventColumn, ExecutionEvent, ExecutionKind,
    Market, MarketColumn, MarketData, MarketKind, Operation, OperationColumn, OperationElement,
    OperationEvent, OperationKind, OrderKind, QuoteKind, SnapshotEvent, TradeEvent,
};
use super::{FxRates, Metadata};
use crate::arrow::BatchReader;
use crate::path::{Path, Segment};
use crate::serie::{
    BooleanSerie, DateTimeNanosecondSerie, Decimal128Serie, FixedBytesSerie, MapSerie, SerieSerie,
    UInt8Serie, UInt16Serie, UInt64Serie, Utf8StringSerie,
};
use crate::{
    ArrowCastOptions, Ccy, Cfi, CodeValue, DataType, Decimal, Error, Field, Limit, MarketDataKind,
    Mic, Result, Serie, SerieReader, Side, State, StructType, TimeInForce, Unit, Uuid,
};
use crate::{IdSource, IdType, Identifier, Identifiers};

// The column names the graph module shares: the root and its nested columns,
// which a view names to exclude them and a book names its entries by.
pub(super) const ROOT: &str = "marketdata";
const OPERATION_ROW: &str = "operationevent";
const BOOKSCOPE: &str = "bookscope";
pub(super) const ALIVE: &str = "alive";
pub(super) const DELTAS: &str = "deltas";
pub(super) const EXECUTIONS: &str = "executions";
pub(super) const BIDLIMITS: &str = "bidlimits";
pub(super) const ASKLIMITS: &str = "asklimits";
/// Every category a row may state, as a refusal lists them.
const KINDS: &str = "ORDR, QUOT, EXEC, TRAD or BOOK";
/// A near-enough width of one operation row's fixed leaves - its clocks,
/// identities, codes and decimals - which the byte bound charges per row.
const OPERATION_ROW_BYTES: u64 = 512;
/// The bytes of one identity's `FixedSizeBinary` slot.
const UUID_WIDTH: i32 = 16;

impl MarketData {
    /// The canonical Arrow row field every leaf is written in.
    ///
    /// The required struct `marketdata`: [`ElementColumn::ALL`] and
    /// [`EventColumn::ALL`] - every one nullable, since an undated leaf
    /// states no clock - then [`MarketColumn::ALL`], which opens with the
    /// `marketdatakind` (the [`MarketDataKind`] the leaf stands under) and
    /// its `marketdatatype`, [`OperationColumn::ALL`], the `bookscope` a
    /// market-data entry states, then the nullable nested columns: `alive`
    /// and `deltas` (a book's operation rows), `executions` (a trade's or
    /// a book's), and `bidlimits` and `asklimits` (a book's price levels).
    ///
    /// ```
    /// use yggdryl::graph::MarketData;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let field = MarketData::field()?;
    /// assert_eq!(field.name(), "marketdata");
    /// assert_eq!(field.fields()[0].name(), "curruuid");
    /// assert_eq!(field.fields()[6].name(), "currunix");
    /// assert!(field.fields()[6].is_nullable());
    /// assert_eq!(field.fields()[15].name(), "marketdatakind");
    /// assert!(!field.fields()[15].is_nullable());
    /// assert_eq!(field.field_len(), 6 + 9 + 34 + 5 + 1 + 5);
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
    /// [`Self::field`], every leaf laid out column by column, and a FIX
    /// message as the leaves it splits into
    /// ([`FixMsg::into_market_data`](crate::FixMsg::into_market_data)) -
    /// one row each, so every row reads back as the leaf it is.
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
            split: std::collections::VecDeque::new(),
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
    /// batch. A row's `marketdatakind` and its shape name its leaf - the
    /// module docs say how; a missing or unknown category, an undated
    /// `TRAD` or `BOOK` row, a dated `BOOK` row in a batch with no `alive`
    /// column, or a fact the leaf requires - a trade's `executions` - is
    /// refused at the first row that needs it. A stated
    /// `isincode` fills an absent `isin` security identifier and must be
    /// the one `securityids` states. Every identity a row states must be the one its rebuilt
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
    Element(ElementColumn),
    Event(EventColumn),
    Market(MarketColumn),
    Operation(OperationColumn),
    BookScope,
    Alive,
    Deltas,
    Executions,
    BidLimits,
    AskLimits,
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
}

impl Column {
    const fn name(self) -> &'static str {
        match self {
            Self::Element(column) => column.name(),
            Self::Event(column) => column.name(),
            Self::Market(column) => column.name(),
            Self::Operation(column) => column.name(),
            Self::BookScope => BOOKSCOPE,
            Self::Alive => ALIVE,
            Self::Deltas => DELTAS,
            Self::Executions => EXECUTIONS,
            Self::BidLimits => BIDLIMITS,
            Self::AskLimits => ASKLIMITS,
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
            Self::Element(column) => column.field()?,
            Self::Event(column) => column.field()?,
            Self::Market(column) => column.field()?,
            Self::Operation(column) => column.field()?,
            Self::BookScope => {
                let mut field = DataType::utf8().nullable_field(BOOKSCOPE);
                field.set_display("Book Scope")?;
                field
            }
            Self::Alive | Self::Deltas | Self::Executions => {
                DataType::serie(operation_row_field()?).nullable_field(self.name())
            }
            Self::BidLimits | Self::AskLimits => {
                DataType::serie(Limit::field()).nullable_field(self.name())
            }
        };
        // A root states only what its leaf does, so every identity, clock
        // and nested column may be null there; a nested row's own columns
        // keep the nullability its fact has.
        Ok(match (role, self) {
            (Role::Read, _) | (Role::Root, Self::Element(_) | Self::Event(_)) => {
                field.with_nullable(true)
            }
            _ => field,
        })
    }

    /// The storage a landed column of this fact holds.
    const fn storage(self) -> Storage {
        match self {
            Self::Element(column) => match column {
                ElementColumn::CurrUuid | ElementColumn::CrossUuid => Storage::Uuid,
                ElementColumn::CrossCode => Storage::Text,
                ElementColumn::CurrHashCode | ElementColumn::CrossHashCode => Storage::UInt64,
                ElementColumn::SrcUuids => Storage::Uuids,
            },
            Self::Event(column) => match column {
                EventColumn::CurrUnix
                | EventColumn::CreaUnix
                | EventColumn::RecdUnix
                | EventColumn::ExprUnix
                | EventColumn::PrevUnix
                | EventColumn::SnapUnix => Storage::Clock,
                EventColumn::PrevUuid => Storage::Uuid,
                EventColumn::SeqNum => Storage::UInt64,
                EventColumn::State => Storage::State,
            },
            Self::Market(column) => match column {
                MarketColumn::MarketDataKind => Storage::MarketDataKind,
                MarketColumn::MarketDataType => Storage::MarketDataType,
                MarketColumn::Price
                | MarketColumn::StopPx
                | MarketColumn::Quantity
                | MarketColumn::DisplayQty
                | MarketColumn::HiddenQty
                | MarketColumn::CxlQty
                | MarketColumn::LastPx
                | MarketColumn::LastQty
                | MarketColumn::AvgPx
                | MarketColumn::CumQty
                | MarketColumn::LeavesQty
                | MarketColumn::PrevPx
                | MarketColumn::PrevQty
                | MarketColumn::SpotRate
                | MarketColumn::ForwardPoints
                | MarketColumn::BidPx
                | MarketColumn::BidQty
                | MarketColumn::AskPx
                | MarketColumn::AskQty => Storage::Decimal,
                MarketColumn::Ticker => Storage::Text,
                MarketColumn::ExecUnix => Storage::Clock,
                MarketColumn::SecurityIds => Storage::Ids,
                MarketColumn::Metadata => Storage::Pairs,
                MarketColumn::Currency | MarketColumn::BidCcy | MarketColumn::AskCcy => {
                    Storage::Code(Code::Ccy)
                }
                MarketColumn::Unit => Storage::Code(Code::Unit),
                MarketColumn::Side => Storage::Side,
                MarketColumn::IsinCode => Storage::Code(Code::Isin),
                MarketColumn::CfiCode => Storage::Code(Code::Cfi),
                MarketColumn::MicCode => Storage::Code(Code::Mic),
                MarketColumn::FxRates => Storage::Rates,
            },
            Self::Operation(column) => match column {
                OperationColumn::Tradable => Storage::Boolean,
                OperationColumn::OrdQty => Storage::Decimal,
                OperationColumn::TimeInForce => Storage::TimeInForce,
                OperationColumn::Identifiers | OperationColumn::PartyIds => Storage::Ids,
            },
            Self::BookScope => Storage::Text,
            Self::Alive | Self::Deltas | Self::Executions => Storage::Nested,
            Self::BidLimits | Self::AskLimits => Storage::Limits,
        }
    }
}

/// Every root column, in canonical row order.
fn root_columns() -> Vec<Column> {
    let mut columns = operation_columns();
    columns.extend([
        Column::Alive,
        Column::Deltas,
        Column::Executions,
        Column::BidLimits,
        Column::AskLimits,
    ]);
    columns
}

/// Every column of an operation row, in canonical order: the element's,
/// the event's, the market's and the operation's facts, then the scope.
fn operation_columns() -> Vec<Column> {
    ElementColumn::ALL
        .map(Column::Element)
        .into_iter()
        .chain(EventColumn::ALL.map(Column::Event))
        .chain(MarketColumn::ALL.map(Column::Market))
        .chain(OperationColumn::ALL.map(Column::Operation))
        .chain([Column::BookScope])
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

/// The item of `alive`, `deltas` and `executions`: one dated operation.
fn operation_row_field() -> Result<Field> {
    struct_field(OPERATION_ROW, &operation_columns(), Role::Operation, false)
}

/// Whether a market column holds a decimal: every price and quantity.
const fn is_decimal(column: MarketColumn) -> bool {
    matches!(
        column,
        MarketColumn::Price
            | MarketColumn::StopPx
            | MarketColumn::Quantity
            | MarketColumn::DisplayQty
            | MarketColumn::HiddenQty
            | MarketColumn::CxlQty
            | MarketColumn::LastPx
            | MarketColumn::LastQty
            | MarketColumn::AvgPx
            | MarketColumn::CumQty
            | MarketColumn::LeavesQty
            | MarketColumn::PrevPx
            | MarketColumn::PrevQty
            | MarketColumn::SpotRate
            | MarketColumn::ForwardPoints
            | MarketColumn::BidPx
            | MarketColumn::BidQty
            | MarketColumn::AskPx
            | MarketColumn::AskQty
    )
}

/// The decimal a market states under a decimal column.
fn market_decimal<E: Market + ?Sized>(column: MarketColumn, market: &E) -> Option<Decimal> {
    match column {
        MarketColumn::Price => market.get_price(),
        MarketColumn::StopPx => market.get_stoppx(),
        MarketColumn::Quantity => market.get_quantity(),
        MarketColumn::DisplayQty => market.get_displayqty(),
        MarketColumn::HiddenQty => market.get_hiddenqty(),
        MarketColumn::CxlQty => market.get_cxlqty(),
        MarketColumn::LastPx => market.get_lastpx(),
        MarketColumn::LastQty => market.get_lastqty(),
        MarketColumn::AvgPx => market.get_avgpx(),
        MarketColumn::CumQty => market.get_cumqty(),
        MarketColumn::LeavesQty => market.get_leavesqty(),
        MarketColumn::PrevPx => market.get_prevpx(),
        MarketColumn::PrevQty => market.get_prevqty(),
        MarketColumn::SpotRate => market.get_spotrate(),
        MarketColumn::ForwardPoints => market.get_forwardpoints(),
        MarketColumn::BidPx => market.get_bidpx(),
        MarketColumn::BidQty => market.get_bidqty(),
        MarketColumn::AskPx => market.get_askpx(),
        MarketColumn::AskQty => market.get_askqty(),
        _ => None,
    }
}

/// Records a decimal market column's value.
fn set_market_decimal<E: Market + ?Sized>(
    column: MarketColumn,
    market: &mut E,
    value: Option<Decimal>,
) {
    match column {
        MarketColumn::Price => market.set_price(value, true),
        MarketColumn::StopPx => market.set_stoppx(value, true),
        MarketColumn::Quantity => market.set_quantity(value, true),
        MarketColumn::DisplayQty => market.set_displayqty(value, true),
        MarketColumn::HiddenQty => market.set_hiddenqty(value, true),
        MarketColumn::CxlQty => market.set_cxlqty(value, true),
        MarketColumn::LastPx => market.set_lastpx(value, true),
        MarketColumn::LastQty => market.set_lastqty(value, true),
        MarketColumn::AvgPx => market.set_avgpx(value, true),
        MarketColumn::CumQty => market.set_cumqty(value, true),
        MarketColumn::LeavesQty => market.set_leavesqty(value, true),
        MarketColumn::PrevPx => market.set_prevpx(value, true),
        MarketColumn::PrevQty => market.set_prevqty(value, true),
        MarketColumn::SpotRate => market.set_spotrate(value, true),
        MarketColumn::ForwardPoints => market.set_forwardpoints(value, true),
        MarketColumn::BidPx => market.set_bidpx(value, true),
        MarketColumn::BidQty => market.set_bidqty(value, true),
        MarketColumn::AskPx => market.set_askpx(value, true),
        MarketColumn::AskQty => market.set_askqty(value, true),
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
    /// The category the row states: the leaf's, or the one a FIX message's
    /// dictionary files it under.
    marketdatakind: crate::MarketDataKind,
    element: &'a dyn Element,
    event: Option<&'a dyn Event>,
    market: &'a dyn Market,
    operation: Option<&'a dyn Operation>,
    control: Option<&'a BookRef>,
    executions: Option<&'a [ExecutionEvent]>,
    book: Option<&'a BookEvent>,
}

impl<'a> Row<'a> {
    fn of(value: &'a MarketData) -> Self {
        let kind = value.kind();
        match value {
            MarketData::Order(leaf) => Self::undated(kind, leaf, leaf, Some(leaf)),
            MarketData::Quote(leaf) => Self::undated(kind, leaf, leaf, Some(leaf)),
            MarketData::Execution(leaf) => Self::undated(kind, leaf, leaf, Some(leaf)),
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
            // A message is written as the leaves it splits into, at the
            // writer's intake; held whole, it states its own category.
            MarketData::Fix(message) => Self {
                marketdatakind: message.msgcat(),
                ..Self::operation(kind, message.as_ref(), None)
            },
        }
    }

    fn execution(execution: &'a ExecutionEvent) -> Self {
        Self::operation(MarketKind::ExecutionEvent, execution, execution.book())
    }

    fn undated(
        kind: MarketKind,
        element: &'a dyn Element,
        market: &'a dyn Market,
        operation: Option<&'a dyn Operation>,
    ) -> Self {
        Self {
            marketdatakind: kind.marketdatakind(),
            element,
            event: None,
            market,
            operation,
            control: None,
            executions: None,
            book: None,
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
            Column::Alive => self.book.map(|book| book.alive().map(Self::of).collect()),
            Column::Deltas => self.book.map(|book| book.deltas().map(Self::of).collect()),
            Column::Executions => self
                .executions
                .map(|executions| executions.iter().map(Self::execution).collect()),
            _ => None,
        }
    }
}

/// The typed facts a row states under one leaf column, each read through
/// the trait that answers it: no cell is built as a value.
impl<'a> Row<'a> {
    fn clock(&self, column: Column) -> Option<i64> {
        let (Column::Event(column), Some(event)) = (column, self.event) else {
            return match column {
                Column::Market(MarketColumn::ExecUnix) => self.market.get_execunix(),
                _ => None,
            };
        };
        match column {
            EventColumn::CurrUnix => Some(event.get_currunix()),
            EventColumn::CreaUnix => event.get_creaunix(),
            EventColumn::RecdUnix => event.get_recdunix(),
            EventColumn::ExprUnix => event.get_exprunix(),
            EventColumn::PrevUnix => event.get_prevunix(),
            EventColumn::SnapUnix => event.get_snapunix(),
            _ => None,
        }
    }

    fn uuid(&self, column: Column) -> Option<Uuid> {
        match column {
            Column::Element(ElementColumn::CurrUuid) => Some(self.element.get_curruuid()),
            Column::Element(ElementColumn::CrossUuid) => Some(self.element.get_crossuuid()),
            Column::Event(EventColumn::PrevUuid) => self.event?.get_prevuuid(),
            _ => None,
        }
    }

    /// The sources a row states; none where it names none.
    fn uuids(&self, column: Column) -> Option<&'a [Uuid]> {
        let element: &'a dyn Element = self.element;
        match column {
            Column::Element(ElementColumn::SrcUuids) => {
                Some(element.get_srcuuids()).filter(|uuids| !uuids.is_empty())
            }
            _ => None,
        }
    }

    fn u64(&self, column: Column) -> Option<u64> {
        match column {
            Column::Element(ElementColumn::CurrHashCode) => Some(self.element.get_currhashcode()),
            Column::Element(ElementColumn::CrossHashCode) => Some(self.element.get_crosshashcode()),
            Column::Event(EventColumn::SeqNum) => {
                Some(self.event?.get_seqnum()).filter(|seqnum| *seqnum != 0)
            }
            _ => None,
        }
    }

    /// The code of a one-byte enum column: the kind and the side.
    fn code8(&self, column: Column) -> Option<u8> {
        match column {
            Column::Market(MarketColumn::MarketDataKind) => Some(self.marketdatakind.code()),
            Column::Market(MarketColumn::Side) => Some(self.market.get_side().code()),
            Column::Operation(OperationColumn::TimeInForce) => {
                self.operation?.get_timeinforce().map(|held| held.code())
            }
            _ => None,
        }
    }

    /// The code of a two-byte enum column: the state and the type.
    fn code16(&self, column: Column) -> Option<u16> {
        match column {
            Column::Event(EventColumn::State) => Some(self.event?.get_state().code()),
            Column::Market(MarketColumn::MarketDataType) => {
                Some(self.market.get_marketdatatype().code())
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

    fn decimal(&self, column: Column) -> Option<Decimal> {
        match column {
            Column::Market(column) => market_decimal(column, self.market),
            Column::Operation(OperationColumn::OrdQty) => self.operation?.get_ordqty(),
            _ => None,
        }
    }

    /// The text a row states under a text or code column: a code as the
    /// text its storage holds.
    fn text(&self, column: Column) -> Option<&'a str> {
        let element: &'a dyn Element = self.element;
        let market: &'a dyn Market = self.market;
        match column {
            Column::Element(ElementColumn::CrossCode) => {
                Some(element.get_crosscode()).filter(|code| !code.is_empty())
            }
            Column::Market(MarketColumn::Currency) => Some(market.get_currency().as_str()),
            Column::Market(MarketColumn::BidCcy) => market.get_bidccy().map(Ccy::as_str),
            Column::Market(MarketColumn::AskCcy) => market.get_askccy().map(Ccy::as_str),
            Column::Market(MarketColumn::Unit) => Some(market.get_unit().as_str()),
            Column::Market(MarketColumn::IsinCode) => market.get_isincode(),
            Column::Market(MarketColumn::CfiCode) => market.get_cficode().map(Cfi::as_str),
            Column::Market(MarketColumn::MicCode) => market.get_miccode().map(Mic::as_str),
            Column::Market(MarketColumn::Ticker) => market.get_ticker(),
            Column::BookScope => self.control?.scope.as_deref(),
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
            Column::Market(MarketColumn::Metadata) => {
                let metadata = self.market.get_metadata();
                for (key, value) in metadata {
                    push(key, value);
                }
                !metadata.is_empty()
            }
            _ => false,
        }
    }

    /// The identifiers a row states under an identifier column, none where
    /// it states none.
    fn ids(&self, column: Column) -> Option<&'a Identifiers> {
        let ids = match column {
            Column::Market(MarketColumn::SecurityIds) => {
                let market: &'a dyn Market = self.market;
                market.get_securityids()
            }
            Column::Operation(OperationColumn::Identifiers) => self.operation?.get_identifiers(),
            Column::Operation(OperationColumn::PartyIds) => self.operation?.get_partyids(),
            _ => return None,
        };
        (!ids.is_empty()).then_some(ids)
    }

    /// The price levels a book row states for the side `column` names,
    /// best first; none for a row that is no book.
    fn limits(&self, column: Column) -> Option<Vec<Limit>> {
        let side = match column {
            Column::BidLimits => Side::Buy,
            Column::AskLimits => Side::Sell,
            _ => return None,
        };
        Some(self.book?.limits(side).collect())
    }
}

/// What one value charges a batch's byte bound: a fixed width per
/// operation row it lays out, nested rows included. A book's price levels
/// are not charged: they are at most one per entry already charged, and far
/// narrower than its row.
fn charge(value: &MarketData) -> u64 {
    let nested = match value {
        MarketData::TradeEvent(trade) => trade.executions().len(),
        MarketData::BookEvent(book) => {
            book.alive_len() + book.deltas_len() + book.executions().len()
        }
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
    /// A list of operation rows: its item field and the rows' shape.
    nested: Option<(FieldRef, Box<Shape>)>,
}

impl Shape {
    fn new(columns: &[Column], fields: &Fields) -> Result<Self> {
        let slots = columns
            .iter()
            .zip(fields.iter())
            .map(|(column, arrow)| {
                let nested = match (column, arrow.data_type()) {
                    (
                        Column::Alive | Column::Deltas | Column::Executions,
                        ArrowType::List(item),
                    ) => Some((
                        Arc::clone(item),
                        Box::new(Self::new(&operation_columns(), struct_children(item)?)?),
                    )),
                    _ => None,
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
        let Some((item, shape)) = &self.nested else {
            return self.leaf(rows);
        };
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
            Storage::Side | Storage::MarketDataKind | Storage::TimeInForce => Arc::new(
                rows.iter()
                    .map(|row| row.code8(column))
                    .collect::<UInt8Array>(),
            ),
            Storage::State | Storage::MarketDataType => Arc::new(
                rows.iter()
                    .map(|row| row.code16(column))
                    .collect::<UInt16Array>(),
            ),
            Storage::Boolean => Arc::new(
                rows.iter()
                    .map(|row| row.boolean(column))
                    .collect::<BooleanArray>(),
            ),
            Storage::Decimal => Arc::new(
                rows.iter()
                    .map(|row| row.decimal(column).map(Decimal::units))
                    .collect::<Decimal128Array>()
                    .with_data_type(datatype.clone()),
            ),
            Storage::Text | Storage::Code(_) => Arc::new(
                rows.iter()
                    .map(|row| row.text(column))
                    .collect::<StringArray>(),
            ),
            Storage::Pairs => {
                let (entries, children, sorted) = map_parts(column, datatype, Storage::Pairs)?;
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
                    sorted,
                )?)
            }
            Storage::Ids => {
                // A sorted map from each identifier's key `src:type` to its
                // row, written in the order the set holds them, which is
                // the key's.
                let (entries, children, sorted) = map_parts(column, datatype, Storage::Ids)?;
                let ArrowType::Struct(item) = children[1].data_type() else {
                    return Err(unlanded(column, Storage::Ids));
                };
                let mut keys = StringBuilder::new();
                let mut cells: [StringBuilder; 3] = std::array::from_fn(|_| StringBuilder::new());
                let mut offsets = Vec::with_capacity(rows.len() + 1);
                offsets.push(0_i32);
                let mut valid = Vec::with_capacity(rows.len());
                for row in rows {
                    let ids = row.ids(column);
                    for id in ids.into_iter().flatten() {
                        keys.append_value(id.key());
                        cells[0].append_value(id.src());
                        cells[1].append_value(id.kind());
                        cells[2].append_value(id.value());
                    }
                    valid.push(ids.is_some());
                    offsets.push(offset(keys.len())?);
                }
                let items = StructArray::try_new(
                    item.clone(),
                    cells
                        .iter_mut()
                        .map(|cell| Arc::new(cell.finish()) as ArrayRef)
                        .collect(),
                    None,
                )?;
                let entries_array = StructArray::try_new(
                    children.clone(),
                    vec![Arc::new(keys.finish()), Arc::new(items)],
                    None,
                )?;
                Arc::new(MapArray::try_new(
                    Arc::clone(entries),
                    OffsetBuffer::new(ScalarBuffer::from(offsets)),
                    entries_array,
                    validity(&valid),
                    sorted,
                )?)
            }
            Storage::Rates => {
                // Null where the element states no rate, so a row stating
                // none lays out no entry.
                let (entries, children, sorted) = map_parts(column, datatype, Storage::Rates)?;
                let mut keys = StringBuilder::new();
                let mut values: Vec<i128> = Vec::new();
                let mut offsets = Vec::with_capacity(rows.len() + 1);
                offsets.push(0_i32);
                let mut valid = Vec::with_capacity(rows.len());
                for row in rows {
                    let rates = row.market.get_fxrates();
                    for (target, rate) in rates {
                        keys.append_value(target.as_str());
                        values.push(rate.units());
                    }
                    valid.push(!rates.is_empty());
                    offsets.push(offset(values.len())?);
                }
                let values =
                    Decimal128Array::from(values).with_data_type(children[1].data_type().clone());
                let entries_array = StructArray::try_new(
                    children.clone(),
                    vec![Arc::new(keys.finish()), Arc::new(values)],
                    None,
                )?;
                Arc::new(MapArray::try_new(
                    Arc::clone(entries),
                    OffsetBuffer::new(ScalarBuffer::from(offsets)),
                    entries_array,
                    validity(&valid),
                    sorted,
                )?)
            }
            Storage::Limits => {
                let ArrowType::List(item) = datatype else {
                    return Err(unlanded(column, Storage::Limits));
                };
                let cells: Vec<Option<Vec<Limit>>> =
                    rows.iter().map(|row| row.limits(column)).collect();
                let cells: Vec<Option<&[Limit]>> = cells.iter().map(Option::as_deref).collect();
                limits_array(column, item, &cells)?
            }
            Storage::Nested => return Err(unlanded(column, Storage::Nested)),
        })
    }
}

/// A map column's entries field, the entries' two children and whether
/// its keys are sorted.
fn map_parts(
    column: Column,
    datatype: &ArrowType,
    storage: Storage,
) -> Result<(&FieldRef, &Fields, bool)> {
    let ArrowType::Map(entries, sorted) = datatype else {
        return Err(unlanded(column, storage));
    };
    let ArrowType::Struct(children) = entries.data_type() else {
        return Err(unlanded(column, storage));
    };
    Ok((entries, children, *sorted))
}

/// A `serie<limit>` over one cell per row: the price levels a book states
/// for one side, null on a row that is no book.
fn limits_array(column: Column, item: &FieldRef, cells: &[Option<&[Limit]>]) -> Result<ArrayRef> {
    let fields = struct_children(item)?;
    let ArrowType::List(uuid) = fields[2].data_type() else {
        return Err(unlanded(column, Storage::Limits));
    };
    let mut offsets = Vec::with_capacity(cells.len() + 1);
    offsets.push(0_i32);
    let mut valid = Vec::with_capacity(cells.len());
    let mut limits: Vec<&Limit> = Vec::new();
    for cell in cells {
        valid.push(cell.is_some());
        limits.extend(cell.iter().copied().flatten());
        offsets.push(offset(limits.len())?);
    }
    let decimal = |at: usize, read: fn(&Limit) -> Option<Decimal>| -> ArrayRef {
        Arc::new(
            limits
                .iter()
                .map(|limit| read(limit).map(Decimal::units))
                .collect::<Decimal128Array>()
                .with_data_type(fields[at].data_type().clone()),
        )
    };
    let mut uuid_offsets = Vec::with_capacity(limits.len() + 1);
    uuid_offsets.push(0_i32);
    let mut held = 0;
    for limit in &limits {
        held += limit.uuids.len();
        uuid_offsets.push(offset(held)?);
    }
    let uuids = FixedSizeBinaryArray::try_from_sparse_iter_with_size(
        limits
            .iter()
            .flat_map(|limit| &limit.uuids)
            .map(|uuid| Some(uuid.into_bytes())),
        UUID_WIDTH,
    )?;
    let uuids: ArrayRef = Arc::new(ListArray::try_new(
        Arc::clone(uuid),
        OffsetBuffer::new(ScalarBuffer::from(uuid_offsets)),
        Arc::new(uuids),
        None,
    )?);
    let values: ArrayRef = Arc::new(StructArray::try_new(
        fields.clone(),
        vec![
            decimal(0, |limit| limit.price),
            decimal(1, |limit| Some(limit.quantity)),
            uuids,
            Arc::new(
                limits
                    .iter()
                    .map(|limit| Some(limit.tradable))
                    .collect::<BooleanArray>(),
            ),
        ],
        None,
    )?);
    list(item, offsets, values, &valid)
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
    /// The leaves a FIX message pulled from the source split into, written
    /// in order before the source is pulled again: a message's row is each
    /// leaf it reports, so every row reads back as what it is.
    split: std::collections::VecDeque<MarketData>,
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
            let item = match self.split.pop_front() {
                Some(leaf) => Ok(leaf),
                None => {
                    let Some(item) = self.source.next() else {
                        self.done = true;
                        break;
                    };
                    match item.into() {
                        Ok(MarketData::Fix(message)) => match message.into_market_data() {
                            Ok(leaves) => {
                                self.split.extend(leaves);
                                continue;
                            }
                            Err(error) => Err(error),
                        },
                        other => other,
                    }
                }
            };
            let ordinal = self.ordinal;
            match item.and_then(|value| checked(&value, ordinal).map(|()| value)) {
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
        MarketData::OrderEvent(operation) => validate_operation_for_write(operation, &here),
        MarketData::QuoteEvent(operation) => validate_operation_for_write(operation, &here),
        MarketData::ExecutionEvent(operation) => validate_operation_for_write(operation, &here),
        MarketData::TradeEvent(trade) => validate_trade_for_write(trade, &here),
        MarketData::BookEvent(book) => validate_book_for_write(book, &here),
        MarketData::SnapshotEvent(control) => validate_control_for_write(control, &here),
        // A message settles its own identity as it is built and written;
        // the row states what it answers.
        MarketData::Fix(_) => Ok(()),
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
        .map_err(|error| path.reroot(error))?;
    if trade.data() == canonical.data() {
        return Ok(());
    }
    IdentityClaims::from_element(trade).validate(&canonical, path)?;
    let mut stated = trade.data().clone();
    normalize_identity(&mut stated, canonical.data());
    validate_operation_facts(&stated, canonical.data(), path)
}

fn validate_book_for_write(book: &BookEvent, path: &Path<'_>) -> Result<()> {
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
                        at(&item, MarketColumn::MarketDataKind.name()),
                        format_smolstr!(
                            "expected order_event or quote_event on a book, got {}",
                            other.kind().as_str()
                        ),
                    ));
                }
            }
        }
        Ok(())
    };
    entries(ALIVE, &mut book.alive())?;
    entries(DELTAS, &mut book.deltas())?;
    let executions = path.field(EXECUTIONS);
    for (index, execution) in book.executions().iter().enumerate() {
        validate_operation_for_write(execution, &executions.child(Segment::Index(index)))?;
    }
    book.validate_parts().map_err(|error| path.reroot(error))?;
    let canonical = book.canonical_event().map_err(|error| path.reroot(error))?;
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
    /// A state: the `uint16` code of its member.
    State,
    /// A side: the `uint8` code of its member.
    Side,
    /// A market data category: the `uint8` code of its member.
    MarketDataKind,
    /// A market data type: the `uint16` code of its member.
    MarketDataType,
    /// A time in force: the `uint8` code of its member.
    TimeInForce,
    Boolean,
    Decimal,
    Text,
    /// A registered code: the text its storage holds, built into the
    /// code's value by the code's own constructor.
    Code(Code),
    /// A `map<utf8, utf8>`: metadata.
    Pairs,
    /// A `serie<identifier>`: securities, alternate identifiers, parties.
    Ids,
    /// A `map<ccy, decimal>`: the FX rates, target to the rate to divide by.
    Rates,
    /// A `serie<limit>`: a book side's price levels.
    Limits,
    /// A nested column, which no leaf holds.
    Nested,
}

/// The registered codes a row states, each riding its own code storage.
#[derive(Clone, Copy, Debug)]
enum Code {
    Ccy,
    Unit,
    Isin,
    Cfi,
    Mic,
}

impl Code {
    /// The text storage a landed column of this code holds.
    fn storage(self, serie: &Serie) -> Option<&Arc<Utf8StringSerie>> {
        match (self, serie) {
            (Self::Ccy, Serie::Ccy(held))
            | (Self::Unit, Serie::Unit(held))
            | (Self::Isin, Serie::Isin(held))
            | (Self::Cfi, Serie::Cfi(held))
            | (Self::Mic, Serie::Mic(held)) => Some(held),
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
    Boolean(Arc<BooleanSerie>),
    Decimal(Arc<Decimal128Serie>),
    /// A state's member codes, each proven where the column landed.
    State(Arc<UInt16Serie>),
    /// A side's member codes, each proven where the column landed.
    Side(Arc<UInt8Serie>),
    /// A market data category's member codes, each proven where the column
    /// landed.
    MarketDataKind(Arc<UInt8Serie>),
    /// A market data type's member codes, each proven where the column
    /// landed.
    MarketDataType(Arc<UInt16Serie>),
    /// A time in force's member codes, each proven where the column landed.
    TimeInForce(Arc<UInt8Serie>),
    Text(Arc<Utf8StringSerie>),
    /// A code's text storage.
    Code(Arc<Utf8StringSerie>),
    Pairs(Arc<MapSerie>, Arc<Utf8StringSerie>, Arc<Utf8StringSerie>),
    /// An identifier map, its text keys and its rows' three text
    /// children: source, type and value.
    Ids(
        Arc<MapSerie>,
        Arc<Utf8StringSerie>,
        Box<[Arc<Utf8StringSerie>; 3]>,
    ),
    /// The rates map, its currency keys and its decimal rates.
    Rates(Arc<MapSerie>, Arc<Utf8StringSerie>, Arc<Decimal128Serie>),
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
            (Storage::State, Serie::State(held)) => Self::State(Arc::clone(held)),
            (Storage::Side, Serie::Side(held)) => Self::Side(Arc::clone(held)),
            (Storage::TimeInForce, Serie::TimeInForce(held)) => Self::TimeInForce(Arc::clone(held)),
            (Storage::MarketDataKind, Serie::MarketDataKind(held)) => {
                Self::MarketDataKind(Arc::clone(held))
            }
            (Storage::MarketDataType, Serie::MarketDataType(held)) => {
                Self::MarketDataType(Arc::clone(held))
            }
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
            (Storage::Ids, Serie::SortedMap(map) | Serie::Map(map)) => {
                match (map.keys(), map.values().children()) {
                    (
                        Serie::Utf8String(keys),
                        [
                            Serie::Utf8String(src),
                            Serie::Utf8String(kind),
                            Serie::Utf8String(value),
                        ],
                    ) => Self::Ids(
                        Arc::clone(map),
                        Arc::clone(keys),
                        Box::new([Arc::clone(src), Arc::clone(kind), Arc::clone(value)]),
                    ),
                    _ => return Err(unlanded(column, storage)),
                }
            }
            (Storage::Rates, Serie::SortedMap(map) | Serie::Map(map)) => {
                match (map.keys(), map.values()) {
                    (Serie::Ccy(keys), Serie::Decimal128(values)) => {
                        Self::Rates(Arc::clone(map), Arc::clone(keys), Arc::clone(values))
                    }
                    _ => return Err(unlanded(column, storage)),
                }
            }
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

    /// The state one cell states; `None` for a null.
    fn state(&self, row: usize) -> Option<State> {
        match self {
            Self::State(held) => held.value(row).and_then(State::from_code),
            _ => None,
        }
    }

    /// The side one cell states; `None` for a null.
    fn side(&self, row: usize) -> Option<Side> {
        match self {
            Self::Side(held) => held.value(row).and_then(Side::from_code),
            _ => None,
        }
    }

    /// The time in force one cell states; `None` for a null.
    fn timeinforce(&self, row: usize) -> Option<TimeInForce> {
        match self {
            Self::TimeInForce(held) => held.value(row).and_then(TimeInForce::from_code),
            _ => None,
        }
    }

    /// The market data type one cell states; `None` for a null.
    fn marketdatatype(&self, row: usize) -> Option<crate::MarketDataType> {
        match self {
            Self::MarketDataType(held) => {
                held.value(row).and_then(crate::MarketDataType::from_code)
            }
            _ => None,
        }
    }

    /// The market data category one cell states; `None` for a null.
    fn marketdatakind(&self, row: usize) -> Option<MarketDataKind> {
        match self {
            Self::MarketDataKind(held) => held.value(row).and_then(MarketDataKind::from_code),
            _ => None,
        }
    }

    fn boolean(&self, row: usize) -> Option<bool> {
        match self {
            Self::Boolean(held) => held.value(row),
            _ => None,
        }
    }

    fn decimal(&self, row: usize) -> Option<Decimal> {
        match self {
            Self::Decimal(held) => held.value(row).and_then(Decimal::from_units),
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

    /// The identifiers one identifier cell states; `None` for a null. An
    /// identifier its type refuses, or a key that is not the `src:type` of
    /// the row it keys, is refused naming the column.
    fn ids(&self, row: usize, path: &Path<'_>, name: &str) -> Result<Option<Identifiers>> {
        let Self::Ids(map, keys, cells) = self else {
            return Ok(None);
        };
        let Some(range) = map.range(row) else {
            return Ok(None);
        };
        let located = |error: Error| invalid(at(path, name), format_smolstr!("{error}"));
        let mut ids = Identifiers::new();
        for item in range {
            let text = |cell: usize| cells[cell].value(item).unwrap_or_default();
            let id = text(0)
                .parse()
                .and_then(|src| Identifier::new(src, text(1).parse()?, text(2)))
                .map_err(located)?;
            let key = keys.value(item).unwrap_or_default();
            if id.key() != key {
                return Err(invalid(
                    at(path, name),
                    format_smolstr!("expected the key {}, got {key:?}", id.key()),
                ));
            }
            ids.insert(id);
        }
        Ok(Some(ids))
    }

    /// The rates one map cell states; `None` for a null. A key no
    /// currency is, a null rate or a target stated twice is refused.
    fn rates(&self, row: usize, path: &Path<'_>, name: &str) -> Result<Option<FxRates>> {
        let Self::Rates(map, keys, values) = self else {
            return Ok(None);
        };
        let Some(range) = map.range(row) else {
            return Ok(None);
        };
        let mut rates = FxRates::new();
        for at in range {
            let Some(key) = keys.value(at) else {
                return Err(invalid(
                    self::at(path, name),
                    "expected a ccy key, got null",
                ));
            };
            let target = Ccy::new(key)
                .map_err(|error| invalid(self::at(path, name), format_smolstr!("{error}")))?;
            let Some(rate) = values.value(at).and_then(Decimal::from_units) else {
                return Err(invalid(
                    self::at(path, name),
                    format_smolstr!("expected the rate to {key}, got null"),
                ));
            };
            if rates.insert(target, rate).is_some() {
                return Err(invalid(
                    self::at(path, name),
                    format_smolstr!("expected one rate per target currency, got {key} twice"),
                ));
            }
        }
        Ok(Some(rates))
    }

    /// The rates one map cell states must be the ones `derived` holds, each
    /// looked up by its target rather than rebuilt into a map; a null cell
    /// states nothing.
    fn check_rates(
        &self,
        row: usize,
        derived: &FxRates,
        path: &Path<'_>,
        name: &str,
    ) -> Result<()> {
        let Self::Rates(map, keys, values) = self else {
            return Ok(());
        };
        let Some(range) = map.range(row) else {
            return Ok(());
        };
        let stated = range.len();
        for at in range {
            let key = keys.value(at);
            let rate = values.value(at).and_then(Decimal::from_units);
            let held = key
                .and_then(|key| Ccy::new(key).ok())
                .and_then(|target| derived.get(&target).copied());
            if held.is_none() || held != rate {
                return Err(differs(path, name, derived, &(key, rate)));
            }
        }
        if stated != derived.len() {
            return Err(invalid(
                self::at(path, name),
                format_smolstr!(
                    "expected the value derived from the row {derived:?}, got {stated} rates"
                ),
            ));
        }
        Ok(())
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
        }
    }
}

/// One landed struct - a batch's root or a list's items - its columns
/// narrowed once to their leaves.
struct Landed {
    element: [Option<Leaf>; ElementColumn::ALL.len()],
    event: [Option<Leaf>; EventColumn::ALL.len()],
    market: [Option<Leaf>; MarketColumn::ALL.len()],
    operation: [Option<Leaf>; OperationColumn::ALL.len()],
    bookscope: Option<Leaf>,
    alive: Option<Operations>,
    deltas: Option<Operations>,
    executions: Option<Operations>,
    bidlimits: Option<Limits>,
    asklimits: Option<Limits>,
}

/// A landed list of operation rows.
struct Operations {
    list: Arc<SerieSerie>,
    items: Box<Landed>,
}

/// The landed price levels of a book side: the list, its items' price,
/// quantity and tradable flag, and the lists of the entries' identities
/// with their items.
struct Limits {
    list: Arc<SerieSerie>,
    price: Leaf,
    quantity: Leaf,
    uuids: (Arc<SerieSerie>, Arc<FixedBytesSerie>),
    tradable: Leaf,
}

impl Landed {
    fn new(children: &[Serie], layout: &[(Column, usize)], layouts: &Layouts) -> Result<Self> {
        let mut landed = Self {
            element: std::array::from_fn(|_| None),
            event: std::array::from_fn(|_| None),
            market: std::array::from_fn(|_| None),
            operation: std::array::from_fn(|_| None),
            bookscope: None,
            alive: None,
            deltas: None,
            executions: None,
            bidlimits: None,
            asklimits: None,
        };
        for (column, at) in layout {
            let serie = &children[*at];
            match column {
                Column::Element(held) => {
                    landed.element[position(&ElementColumn::ALL, held)] =
                        Some(Leaf::of(*column, serie)?);
                }
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
                Column::BookScope => landed.bookscope = Some(Leaf::of(*column, serie)?),
                Column::Alive => landed.alive = Some(Operations::new(*column, serie, layouts)?),
                Column::Deltas => landed.deltas = Some(Operations::new(*column, serie, layouts)?),
                Column::Executions => {
                    landed.executions = Some(Operations::new(*column, serie, layouts)?);
                }
                Column::BidLimits => landed.bidlimits = Some(Limits::new(*column, serie)?),
                Column::AskLimits => landed.asklimits = Some(Limits::new(*column, serie)?),
            }
        }
        Ok(landed)
    }

    /// Root row `row` as the leaf its `marketdatakind` and its shape name:
    /// dated where it states `currunix`, and a dated book a book or a
    /// snapshot control by whether it states its `alive` entries.
    fn value(&self, row: usize, path: &Path<'_>) -> Result<MarketData> {
        let dated = self
            .event_leaf(EventColumn::CurrUnix)
            .and_then(|leaf| leaf.clock(row))
            .is_some();
        match (self.category(row, path)?, dated) {
            (MarketDataKind::Order, false) => {
                self.element::<OrderKind>(row, path).map(MarketData::from)
            }
            (MarketDataKind::Order, true) => self
                .operation_event::<OrderKind>(row, path)
                .map(MarketData::from),
            (MarketDataKind::Quotation, false) => {
                self.element::<QuoteKind>(row, path).map(MarketData::from)
            }
            (MarketDataKind::Quotation, true) => self
                .operation_event::<QuoteKind>(row, path)
                .map(MarketData::from),
            (MarketDataKind::Execution, false) => self
                .element::<ExecutionKind>(row, path)
                .map(MarketData::from),
            (MarketDataKind::Execution, true) => self
                .operation_event::<ExecutionKind>(row, path)
                .map(MarketData::from),
            (MarketDataKind::Trade, false) => Err(invalid(
                at(path, MarketColumn::MarketDataKind.name()),
                "expected a dated TRAD row, got currunix null",
            )),
            (MarketDataKind::Trade, true) => self.trade(row, path).map(MarketData::from),
            (MarketDataKind::Book, false) => Err(invalid(
                at(path, MarketColumn::MarketDataKind.name()),
                "expected a dated BOOK row, got currunix null",
            )),
            (MarketDataKind::Book, true) => self.dated_book(row, path),
            (other, _) => Err(invalid(
                at(path, MarketColumn::MarketDataKind.name()),
                format_smolstr!("expected {KINDS}, got {}", other.as_str()),
            )),
        }
    }

    /// The category a row states; a null is refused.
    fn category(&self, row: usize, path: &Path<'_>) -> Result<MarketDataKind> {
        self.category_of(row).ok_or_else(|| {
            invalid(
                at(path, MarketColumn::MarketDataKind.name()),
                format_smolstr!("expected {KINDS}, got null"),
            )
        })
    }

    fn category_of(&self, row: usize) -> Option<MarketDataKind> {
        self.market_leaf(MarketColumn::MarketDataKind)
            .and_then(|leaf| leaf.marketdatakind(row))
    }

    /// The landed leaf of one element column, where the struct holds it.
    fn element_leaf(&self, column: ElementColumn) -> Option<&Leaf> {
        self.element[position(&ElementColumn::ALL, &column)].as_ref()
    }

    /// The landed leaf of one event column, where the struct holds it.
    fn event_leaf(&self, column: EventColumn) -> Option<&Leaf> {
        self.event[position(&EventColumn::ALL, &column)].as_ref()
    }

    /// The landed leaf of one market column, where the struct holds it.
    fn market_leaf(&self, column: MarketColumn) -> Option<&Leaf> {
        self.market[position(&MarketColumn::ALL, &column)].as_ref()
    }

    /// A dated `BOOK` row: a book where it states its `alive` entries -
    /// none is a statement too - and a snapshot control where the cell is
    /// null. A table may store a null list as an empty one - PyIceberg
    /// reads a null list of structs back as `[]` - so a row whose `alive`
    /// holds no entry is the snapshot control where the `curruuid` it
    /// states is the one that control derives, which no book shares: a
    /// book's identity folds in its sides. A batch that landed no `alive`
    /// column says neither, and the row is refused rather than typed.
    fn dated_book(&self, row: usize, path: &Path<'_>) -> Result<MarketData> {
        let Some(alive) = &self.alive else {
            return Err(invalid(
                at(path, ALIVE),
                "expected the alive column that tells a book_event from a snapshot_event, \
                 got none",
            ));
        };
        let entries = alive.list.range(row);
        if entries.as_ref().is_none_or(std::ops::Range::is_empty) {
            let (control, claims) = self.snapshot_control(row, path)?;
            let states_control = matches!(
                claims.curruuid,
                Claim::Stated(stated) if stated == control.get_curruuid()
            );
            if entries.is_none() || states_control {
                return self
                    .checked_snapshot(row, control, claims, path)
                    .map(MarketData::from);
            }
        }
        self.book(row, path).map(MarketData::from)
    }

    /// The instant a dated leaf requires.
    fn currunix(&self, row: usize, path: &Path<'_>) -> Result<i64> {
        self.event_leaf(EventColumn::CurrUnix)
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
        self.read_crosscode(row, &mut element);
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
        self.read_event(row, target, &mut claims);
        self.read_market(row, target, path)?;
        self.read_crosscode(row, target);
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
        data.set_marketdatakind(crate::MarketDataKind::Trade);
        let claims = self.read_operation_event(row, &mut data, path)?;
        let Some(executions) = self.executions(row, path)? else {
            return Err(invalid(
                at(path, EXECUTIONS),
                "expected the executions of a trade_event, got null",
            ));
        };
        let trade = TradeEvent::from_facts(data, executions).map_err(|error| path.reroot(error))?;
        self.check_operation_event(row, &trade, claims, path)?;
        Ok(trade)
    }

    /// The snapshot control a row's event and market facts and its scope
    /// make, beside the identities the row claims, neither checked.
    fn snapshot_control(
        &self,
        row: usize,
        path: &Path<'_>,
    ) -> Result<(SnapshotEvent, IdentityClaims)> {
        let mut event = MarketEventFacts::default();
        event.set_marketdatakind(crate::MarketDataKind::Book);
        let mut claims = IdentityClaims::default();
        event.set_currunix(self.currunix(row, path)?);
        self.read_event(row, &mut event, &mut claims);
        self.read_market(row, &mut event, path)?;
        self.read_crosscode(row, &mut event);
        let control = SnapshotEvent::from_control(event, self.control(row).unwrap_or_default());
        Ok((control, claims))
    }

    /// `control`, once every identity and fact its row states is the one
    /// it derives.
    fn checked_snapshot(
        &self,
        row: usize,
        control: SnapshotEvent,
        claims: IdentityClaims,
        path: &Path<'_>,
    ) -> Result<SnapshotEvent> {
        claims.validate(&control, path)?;
        self.check_event(row, &control, path)?;
        self.check_market(row, &control, path)?;
        Ok(control)
    }

    fn book(&self, row: usize, path: &Path<'_>) -> Result<BookEvent> {
        let mut event = MarketEventFacts::default();
        event.set_marketdatakind(crate::MarketDataKind::Book);
        let mut claims = IdentityClaims::default();
        event.set_currunix(self.currunix(row, path)?);
        self.read_event(row, &mut event, &mut claims);
        self.read_market(row, &mut event, path)?;
        self.read_crosscode(row, &mut event);
        let alive = Self::entries(self.alive.as_ref(), ALIVE, row, path)?;
        let deltas = Self::entries(self.deltas.as_ref(), DELTAS, row, path)?;
        let executions = self.executions(row, path)?.unwrap_or_default();
        let book = BookEvent::from_parts(event, alive, deltas, executions)
            .map_err(|error| path.reroot(error))?;
        claims.validate(&book, path)?;
        self.check_event(row, &book, path)?;
        self.check_market(row, &book, path)?;
        self.check_book(row, &book, path)?;
        Ok(book)
    }

    /// The price levels a book row states must be the ones its sides
    /// derive, level for level; a null cell states nothing.
    fn check_book(&self, row: usize, book: &BookEvent, path: &Path<'_>) -> Result<()> {
        for (landed, name, side) in [
            (&self.bidlimits, BIDLIMITS, Side::Buy),
            (&self.asklimits, ASKLIMITS, Side::Sell),
        ] {
            let Some(landed) = landed else {
                continue;
            };
            let Some(stated) = landed.read(row, &path.field(name))? else {
                continue;
            };
            let derived: Vec<Limit> = book.limits(side).collect();
            if stated != derived {
                return Err(differs(path, name, &derived, &stated));
            }
        }
        Ok(())
    }

    /// The orders and quotes one of a book's lists holds for row `row`:
    /// none where it states none.
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
                match list.items.category(at, &item)? {
                    MarketDataKind::Order => list
                        .items
                        .operation_event::<OrderKind>(at, &item)
                        .map(MarketData::from),
                    MarketDataKind::Quotation => list
                        .items
                        .operation_event::<QuoteKind>(at, &item)
                        .map(MarketData::from),
                    other => Err(invalid(
                        self::at(&item, MarketColumn::MarketDataKind.name()),
                        format_smolstr!("expected ORDR or QUOT on a book, got {}", other.as_str()),
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
                // A nested execution states its category or leaves it to
                // the list it stands in.
                if let Some(kind) = list.items.category_of(at)
                    && kind != MarketDataKind::Execution
                {
                    return Err(invalid(
                        self::at(&item, MarketColumn::MarketDataKind.name()),
                        format_smolstr!("expected EXEC, got {}", kind.as_str()),
                    ));
                }
                list.items.operation_event::<ExecutionKind>(at, &item)
            })
            .collect::<Result<Vec<_>>>()
            .map(Some)
    }

    /// The one book-control fact a row states, its scope.
    fn control(&self, row: usize) -> Option<BookRef> {
        let scope = self.bookscope.as_ref()?.text(row)?;
        Some(BookRef {
            scope: Some(SmolStr::new(scope)),
            ..BookRef::default()
        })
    }

    /// The element facts a row states - the identities, the cross code and
    /// the sources - onto `target`, each stated identity claimed.
    fn read_element<E: Element + ?Sized>(
        &self,
        row: usize,
        target: &mut E,
        claims: &mut IdentityClaims,
    ) {
        for (column, leaf) in ElementColumn::ALL.into_iter().zip(&self.element) {
            let Some(leaf) = leaf else {
                continue;
            };
            match column {
                ElementColumn::CurrUuid => {
                    let uuid = leaf.uuid(row);
                    if let Some(uuid) = uuid {
                        target.set_curruuid(uuid);
                    }
                    claims.curruuid = Claim::of(uuid);
                }
                ElementColumn::CrossUuid => {
                    let uuid = leaf.uuid(row);
                    if let Some(uuid) = uuid {
                        target.set_crossuuid(uuid);
                    }
                    claims.crossuuid = Claim::of(uuid);
                }
                // Read last, by `read_crosscode`, once its prefix's facts
                // stand.
                ElementColumn::CrossCode => {}
                ElementColumn::CurrHashCode => {
                    let code = leaf.u64(row);
                    if let Some(code) = code {
                        target.set_currhashcode(code);
                    }
                    claims.currhashcode = Claim::of(code);
                }
                ElementColumn::CrossHashCode => {
                    let code = leaf.u64(row);
                    if let Some(code) = code {
                        target.set_crosshashcode(code);
                    }
                    claims.crosshashcode = Claim::of(code);
                }
                ElementColumn::SrcUuids => {
                    if let Some(uuids) = leaf.uuids(row) {
                        target.set_srcuuids(uuids.collect());
                    }
                }
            }
        }
    }

    /// The cross code a row states onto `target`, once the facts its stored
    /// prefix spells - the category the holder stamped and the side the
    /// row states - stand, so a canonical code is held as read rather than
    /// spelled again under each fact as it lands.
    fn read_crosscode<E: Element + ?Sized>(&self, row: usize, target: &mut E) {
        let leaf = ElementColumn::ALL
            .iter()
            .position(|column| *column == ElementColumn::CrossCode)
            .and_then(|at| self.element.get(at)?.as_ref());
        if let Some(code) = leaf
            .and_then(|leaf| leaf.text(row))
            .filter(|code| !code.is_empty())
        {
            target.set_crosscode(code.to_owned());
        }
    }

    /// Every event fact a row states onto `target` but the instant, which
    /// the caller reads first as the fact a dated leaf requires.
    fn read_event<E: Event + ?Sized>(
        &self,
        row: usize,
        target: &mut E,
        claims: &mut IdentityClaims,
    ) {
        self.read_element(row, target, claims);
        for (column, leaf) in EventColumn::ALL.into_iter().zip(&self.event) {
            let Some(leaf) = leaf else {
                continue;
            };
            match column {
                EventColumn::CreaUnix => target.set_creaunix(leaf.clock(row)),
                EventColumn::RecdUnix => target.set_recdunix(leaf.clock(row)),
                EventColumn::ExprUnix => target.set_exprunix(leaf.clock(row)),
                EventColumn::PrevUnix => target.set_prevunix(leaf.clock(row)),
                EventColumn::SnapUnix => target.set_snapunix(leaf.clock(row)),
                EventColumn::PrevUuid => target.set_prevuuid(leaf.uuid(row)),
                EventColumn::SeqNum => target.set_seqnum(leaf.u64(row).unwrap_or(0)),
                EventColumn::State => {
                    if let Some(state) = leaf.state(row) {
                        target.set_state(state);
                    }
                }
                // The instant is the caller's to read first.
                EventColumn::CurrUnix => {}
            }
        }
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
                target.set_ticker(leaf.text(row).map(SmolStr::new), true);
            } else if column == MarketColumn::ExecUnix {
                target.set_execunix(leaf.clock(row), true);
            } else if column == MarketColumn::SecurityIds {
                let Some(ids) = leaf.ids(row, path, column.name())? else {
                    continue;
                };
                let located = |error: Error| path.field(column.name()).reroot(error);
                target.set_securityids(ids, true).map_err(located)?;
            } else if column == MarketColumn::IsinCode {
                // A projection of `securityids`, read after it: it fills an
                // absent ISIN and must agree with a stated one.
                let Some(stated) = leaf.text(row) else {
                    continue;
                };
                match target.get_isincode() {
                    Some(held) if held != stated => {
                        return Err(invalid(
                            at(path, column.name()),
                            format_smolstr!(
                                "expected the securityids isin {held:?}, got {stated:?}"
                            ),
                        ));
                    }
                    Some(_) => {}
                    None => {
                        let located = |error: Error| {
                            invalid(at(path, column.name()), format_smolstr!("{error}"))
                        };
                        let id = Identifier::new(IdSource::Base, IdType::Isin, stated)
                            .map_err(located)?;
                        target.insert_securityid(id).map_err(located)?;
                    }
                }
            } else if column == MarketColumn::FxRates {
                if let Some(rates) = leaf.rates(row, path, column.name())? {
                    target.set_fxrates(rates, true);
                }
            } else if column == MarketColumn::Metadata {
                if let Some(pairs) = leaf.pairs(row) {
                    // Inserted one by one: collecting stages the entries in
                    // a buffer the map does not keep.
                    let mut metadata = Metadata::new();
                    for (key, value) in pairs {
                        metadata.insert(SmolStr::new(key), SmolStr::new(value));
                    }
                    target.set_metadata(Some(metadata), true);
                }
            } else {
                let name = column.name();
                match column {
                    MarketColumn::Currency => {
                        if let Some(held) = leaf.code(row, path, name, |text| Ccy::new(text))? {
                            target.set_currency(held, true);
                        }
                    }
                    MarketColumn::BidCcy => {
                        if let Some(held) = leaf.code(row, path, name, |text| Ccy::new(text))? {
                            target.set_bidccy(Some(held), true);
                        }
                    }
                    MarketColumn::AskCcy => {
                        if let Some(held) = leaf.code(row, path, name, |text| Ccy::new(text))? {
                            target.set_askccy(Some(held), true);
                        }
                    }
                    MarketColumn::Unit => {
                        if let Some(held) = leaf.code(row, path, name, |text| Unit::new(text))? {
                            target.set_unit(held, true);
                        }
                    }
                    MarketColumn::Side => {
                        if let Some(held) = leaf.side(row) {
                            target.set_side(held, true);
                        }
                    }
                    MarketColumn::MarketDataType => {
                        if let Some(held) = leaf.marketdatatype(row) {
                            target.set_marketdatatype(held, true);
                        }
                    }
                    MarketColumn::CfiCode => {
                        if let Some(held) = leaf.code(row, path, name, |text| Cfi::new(text))? {
                            target.set_cficode(Some(held), true);
                        }
                    }
                    MarketColumn::MicCode => {
                        if let Some(held) = leaf.code(row, path, name, |text| Mic::new(text))? {
                            target.set_miccode(Some(held), true);
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
                OperationColumn::Tradable => target.set_tradable(leaf.boolean(row), true),
                OperationColumn::Identifiers | OperationColumn::PartyIds => {
                    let Some(ids) = leaf.ids(row, path, column.name())? else {
                        continue;
                    };
                    let located = |error: Error| path.field(column.name()).reroot(error);
                    if column == OperationColumn::Identifiers {
                        target.set_identifiers(ids, true).map_err(located)?;
                    } else {
                        target.set_partyids(ids, true).map_err(located)?;
                    }
                }
                OperationColumn::OrdQty => target.set_ordqty(leaf.decimal(row), true),
                OperationColumn::TimeInForce => {
                    let held = leaf.timeinforce(row);
                    if held.is_some() {
                        target.set_timeinforce(held, true);
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
        if let Some(leaf) = self.element_leaf(ElementColumn::CrossCode)
            && let Some(code) = leaf
                .text(row)
                .filter(|code| *code != canonical.get_crosscode())
        {
            return Err(differs(
                path,
                ElementColumn::CrossCode.name(),
                &canonical.get_crosscode(),
                &code,
            ));
        }
        if let Some(uuids) = self
            .element_leaf(ElementColumn::SrcUuids)
            .and_then(|leaf| leaf.uuids(row))
            && !uuids.eq(canonical.get_srcuuids().iter().copied())
        {
            return Err(invalid(
                at(path, ElementColumn::SrcUuids.name()),
                format_smolstr!(
                    "expected the value derived from the row {:?}",
                    canonical.get_srcuuids()
                ),
            ));
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
                EventColumn::RecdUnix => clock(canonical.get_recdunix())?,
                EventColumn::ExprUnix => clock(canonical.get_exprunix())?,
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
                EventColumn::State => match leaf.state(row) {
                    Some(stated) if stated != *canonical.get_state() => {
                        return Err(differs(path, column.name(), canonical.get_state(), &stated));
                    }
                    _ => {}
                },
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
            } else if column == MarketColumn::ExecUnix {
                match leaf.clock(row) {
                    Some(stated) if Some(stated) != canonical.get_execunix() => {
                        return Err(differs(
                            path,
                            column.name(),
                            &canonical.get_execunix(),
                            &stated,
                        ));
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
                check_ids(leaf, row, canonical.get_securityids(), path, column.name())?;
            } else if column == MarketColumn::IsinCode {
                match leaf.text(row) {
                    Some(stated) if Some(stated) != canonical.get_isincode() => {
                        return Err(differs(
                            path,
                            column.name(),
                            &canonical.get_isincode(),
                            &stated,
                        ));
                    }
                    _ => {}
                }
            } else if column == MarketColumn::FxRates {
                leaf.check_rates(row, canonical.get_fxrates(), path, column.name())?;
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
                    MarketColumn::Side => match leaf.side(row) {
                        Some(stated) if stated != canonical.get_side() => {
                            return Err(differs(path, name, &canonical.get_side(), &stated));
                        }
                        _ => {}
                    },
                    MarketColumn::MarketDataType => match leaf.marketdatatype(row) {
                        Some(stated) if stated != canonical.get_marketdatatype() => {
                            return Err(differs(
                                path,
                                name,
                                &canonical.get_marketdatatype(),
                                &stated,
                            ));
                        }
                        _ => {}
                    },
                    MarketColumn::CfiCode => check_code(
                        leaf,
                        row,
                        canonical.get_cficode(),
                        |text| Cfi::new(text),
                        path,
                        name,
                    )?,
                    MarketColumn::MicCode => check_code(
                        leaf,
                        row,
                        canonical.get_miccode(),
                        |text| Mic::new(text),
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
                OperationColumn::Identifiers | OperationColumn::PartyIds => {
                    let derived = if column == OperationColumn::Identifiers {
                        canonical.get_identifiers()
                    } else {
                        canonical.get_partyids()
                    };
                    check_ids(leaf, row, derived, path, column.name())?;
                }
                OperationColumn::OrdQty => match leaf.decimal(row) {
                    Some(stated) if Some(stated) != canonical.get_ordqty() => {
                        return Err(differs(
                            path,
                            column.name(),
                            &canonical.get_ordqty(),
                            &stated,
                        ));
                    }
                    _ => {}
                },
                OperationColumn::TimeInForce => match leaf.timeinforce(row) {
                    Some(stated) if Some(&stated) != canonical.get_timeinforce() => {
                        return Err(differs(
                            path,
                            column.name(),
                            &canonical.get_timeinforce(),
                            &stated,
                        ));
                    }
                    _ => {}
                },
            }
        }
        Ok(())
    }
}

/// A stated identifier cell must name the identifiers its canonical leaf
/// holds - each key, source, type and value - each stated entry looked up
/// in place and matched once, so the check builds nothing.
fn check_ids(
    leaf: &Leaf,
    row: usize,
    held: &Identifiers,
    path: &Path<'_>,
    name: &str,
) -> Result<()> {
    let Leaf::Ids(map, keys, cells) = leaf else {
        return Ok(());
    };
    let Some(range) = map.range(row) else {
        return Ok(());
    };
    // One bit per held identifier, inline up to sixty-four of them.
    let mut matched: smallvec::SmallVec<[u64; 1]> = smallvec::smallvec![0; held.len().div_ceil(64)];
    let mut stated = 0_usize;
    for item in range {
        let text = |cell: usize| cells[cell].value(item).unwrap_or_default();
        let (src, kind, value) = (text(0), text(1), text(2));
        let key = keys.value(item).unwrap_or_default();
        let found = held.iter().position(|id| {
            id.src().is_spelled(src)
                && id.kind().is_spelled(kind)
                && key.split_once(':') == Some((id.src().as_str(), id.kind().as_str()))
                && id.value() == value.trim()
        });
        let Some(at) = found.filter(|at| matched[at / 64] & (1 << (at % 64)) == 0) else {
            return Err(differs(
                path,
                name,
                held,
                &format_args!("{src}:{kind}={value}"),
            ));
        };
        matched[at / 64] |= 1 << (at % 64);
        stated += 1;
    }
    if stated != held.len() {
        return Err(differs(
            path,
            name,
            held,
            &format_args!("{stated} identifiers"),
        ));
    }
    Ok(())
}

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

impl Limits {
    /// A landed `serie<limit>` column.
    fn new(column: Column, serie: &Serie) -> Result<Self> {
        let Serie::Serie(list) = serie else {
            return Err(unlanded(column, Storage::Nested));
        };
        let [
            Serie::Decimal128(price),
            Serie::Decimal128(quantity),
            Serie::Serie(uuids),
            Serie::Boolean(tradable),
        ] = list.items().children()
        else {
            return Err(unlanded(column, Storage::Nested));
        };
        let Serie::Uuid(items) = uuids.items() else {
            return Err(unlanded(column, Storage::Uuids));
        };
        Ok(Self {
            list: Arc::clone(list),
            price: Leaf::Decimal(Arc::clone(price)),
            quantity: Leaf::Decimal(Arc::clone(quantity)),
            uuids: (Arc::clone(uuids), Arc::clone(items)),
            tradable: Leaf::Boolean(Arc::clone(tradable)),
        })
    }

    /// The limits one cell states, in their stated order; `None` for a
    /// null cell, which states nothing. A limit stating no quantity, no
    /// entries or no tradable flag, or an entry that is no uuid, is refused
    /// where it stands, under `here` - the column's.
    fn read(&self, row: usize, here: &Path<'_>) -> Result<Option<Vec<Limit>>> {
        let Some(range) = self.list.range(row) else {
            return Ok(None);
        };
        let (lists, items) = &self.uuids;
        range
            .enumerate()
            .map(|(index, at)| {
                let item = here.child(Segment::Index(index));
                let Some(quantity) = self.quantity.decimal(at) else {
                    return Err(invalid(
                        self::at(&item, "quantity"),
                        "expected a decimal, got null",
                    ));
                };
                let Some(entries) = lists.range(at) else {
                    return Err(invalid(
                        self::at(&item, "uuids"),
                        "expected a serie of uuids, got null",
                    ));
                };
                let entries_path = item.field("uuids");
                let uuids = entries
                    .enumerate()
                    .map(|(entry, held)| {
                        items
                            .value(held)
                            .and_then(|bytes| Uuid::from_bytes(bytes).ok())
                            .ok_or_else(|| {
                                invalid(
                                    entries_path.child(Segment::Index(entry)).render(),
                                    "expected a uuid, got null",
                                )
                            })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let Some(tradable) = self.tradable.boolean(at) else {
                    return Err(invalid(
                        self::at(&item, "tradable"),
                        "expected a boolean, got null",
                    ));
                };
                Ok(Limit {
                    price: self.price.decimal(at),
                    quantity,
                    uuids,
                    tradable,
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(Some)
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
            if let Some((landed, len)) = &self.landed
                && self.row < *len
            {
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

fn invalid(path: impl Into<SmolStr>, reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: path.into(),
        reason: reason.into(),
    }
}
