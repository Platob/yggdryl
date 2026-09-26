# yggdryl-market-data in Rust

The leaves, `MarketData`, the iterators and views live in `yggdryl::graph`;
`Decimal`, `Side`, `Ccy`, `State`, `SecType`, `SecurityId`, `Uuid` and `Limit`
at the crate root. Getters and setters are trait methods: import
`yggdryl::graph::{Element, Event, Market, Operation}` as needed. Setters do not
finalize - call `finalize()` once the facts are in.

## Build an order event from named facts

Set the facts, then `finalize` derives the identity and what the facts imply:
the national number an ISIN embeds, the lane a priced side quotes.

```rust
use yggdryl::graph::{Element, Event, Market, Operation, OrderEvent};
use yggdryl::{Ccy, Decimal, SecType, SecurityId, Side};

// Instants are i64 nanoseconds since the Unix epoch, UTC.
const T: i64 = 1_700_000_000_000_000_000;
let mut order = OrderEvent::at(T);
order.set_crosscode("O-1001".to_owned());
order.set_side(Side::Buy);
order.set_price(Some("189.50".parse()?));
order.set_quantity(Some(Decimal::from_int(100)));
order.set_currency(Ccy::new("USD")?);
order.set_ticker(Some("AAPL".into()));
order.insert_securityid(SecurityId::new(SecType::read("ISIN")?, "US0378331005")?)?;
order.insert_altid("ORDERID", "O-1001")?;
order.finalize();

// A dated identity is a UUIDv7: its millisecond leads.
assert_eq!(order.get_curruuid(), order.time_uuid()?);
assert!(order.get_curruuid().to_string().starts_with("018bcfe5-6800-7"));
assert_ne!(order.get_crossuuid(), order.get_curruuid(), "the cross code names a chain");
// Derived on finalize: the CUSIP inside the ISIN, and the bid lane of a priced buy.
assert_eq!(order.get_securityids().get("CUSIP"), Some("037833100"));
assert_eq!(order.get_bid().and_then(|lane| lane.price), Some("189.5".parse()?));
assert_eq!(order.get_lastpx(), None, "a price is never a last execution");
assert_eq!(order.get_state().as_str(), "00UNKNOWN");
```

## Build undated leaves, quotes and book entries

An undated leaf's identity is its content; `at` dates it. A quote names its side
through the one lane it states, and a market-data entry carries a `BookRef`.

```rust
use yggdryl::graph::{BookRef, Element, Lane, Market, MarketKind, MdUpdateAction, Operation, Order, OrderEvent, QuoteEvent};
use yggdryl::{Ccy, Decimal, Side, Uuid};

let mut order = Order::new();
order.set_crosscode("O-1001".to_owned());
order.set_side(Side::Buy);
order.set_price(Some("189.50".parse()?));
order.finalize();
assert_eq!(order.kind(), MarketKind::Order);
assert_eq!(order.get_curruuid(), Uuid::from_v8(u128::from(order.get_currhashcode())));
let event: OrderEvent = order.at(1_700_000_000_000_000_000);
assert!(!event.is_after(&event));

// One lane, no side of its own: the quote takes the lane's side.
let mut offer = QuoteEvent::at(1_700_000_000_000_000_000);
offer.set_ticker(Some("AAPL".into()));
offer.set_ask(Some(Lane {
    price: Some("189.52".parse()?),
    quantity: Some(Decimal::from_int(100)),
    currency: Some(Ccy::new("USD")?),
    ..Lane::default()
}));
offer.finalize();
assert_eq!(offer.get_side(), Side::Sell);
assert_eq!(offer.get_price(), Some("189.52".parse()?));

// The book control digests into the entry.
let mut entry = offer.clone().with_book(BookRef {
    action: MdUpdateAction::read("new"),
    position: Some(1),
    ..BookRef::default()
});
entry.finalize();
assert_eq!(entry.action().map(MdUpdateAction::as_str), Some("0"));
assert_ne!(entry.get_curruuid(), offer.get_curruuid());
```

## Chain two events and merge two statements of one

`with_previous` states an event as the one after its predecessor; `merge_with`
folds another statement of the same event (the later recording leads, sources
union).

```rust
use yggdryl::graph::{Element, Event, Market, OrderEvent};
use yggdryl::{Decimal, Side, State, Uuid};

const T: i64 = 1_700_000_000_000_000_000;
let event = |unix: i64, state: &str| -> yggdryl::Result<OrderEvent> {
    let mut event = OrderEvent::at(unix);
    event.set_crosscode("O-1001".to_owned());
    event.set_state(State::from_spelling(state).expect("a shipped state"));
    event.set_side(Side::Buy);
    event.set_price(Some("189.50".parse()?));
    event.set_quantity(Some(Decimal::from_int(100)));
    event.finalize();
    Ok(event)
};
let placed = event(T, "New")?;
let filled = event(T + 1_000_000_000, "PartiallyFilled")?.with_previous(&placed).expect("a later event follows");
assert_eq!((filled.get_prevuuid(), filled.get_prevunix(), filled.get_seqnum()), (Some(placed.get_curruuid()), Some(T), 1));
assert_eq!(filled.get_crossuuid(), placed.get_crossuuid(), "one chain");
assert_eq!(filled.get_prevpx(), Some("189.50".parse()?));
// Never itself, never one that happened after it.
assert!(placed.clone().with_previous(&filled).is_none());

// One report recorded by two hops: recording clocks and sources are not content.
let hop = |recorded: i64, line: u128| -> yggdryl::Result<OrderEvent> {
    let mut report = event(T + 1_000_000_000, "PartiallyFilled")?;
    report.set_recdunix(Some(recorded));
    report.set_srcuuids(vec![Uuid::from_v8(line)]);
    report.finalize();
    Ok(report)
};
let (gateway, oms) = (hop(T + 1_002_000_000, 1)?, hop(T + 1_005_000_000, 2)?);
assert_eq!(gateway.get_curruuid(), oms.get_curruuid());
let merged = oms.merge_with(&gateway).expect("another statement");
assert_eq!(merged.get_recdunix(), Some(T + 1_002_000_000));
assert_eq!(merged.get_srcuuids(), [Uuid::from_v8(1), Uuid::from_v8(2)]);
```

## Walk a stream into chains

`EventIterator` chains a stream by cross identity (and by a live element's
`altids`), yields a twin as a restatement rather than a successor, retires a
chain at a terminal state and emits one `95EXPIRED` at a deadline.

```rust
use yggdryl::graph::{Element, Event, EventIterator, OrderEvent};
use yggdryl::State;

const T: i64 = 1_700_000_000_000_000_000;
const SECOND: i64 = 1_000_000_000;
let event = |second: i64, order: &str, state: &str| {
    let mut event = OrderEvent::at(T + second * SECOND);
    event.set_crosscode(order.to_owned());
    event.set_state(State::from_spelling(state).expect("a shipped state"));
    event.finalize();
    event
};
// Unsorted: one report logged twice, and O-1001 reopened after its fill.
let arrived = vec![
    event(3, "O-1001", "Filled"),
    event(0, "O-1001", "New"),
    event(2, "O-1001", "PartiallyFilled"),
    event(2, "O-1001", "PartiallyFilled"),
    event(4, "O-1001", "New"),
];
let chained: Vec<OrderEvent> = EventIterator::new(arrived, false).collect();
let places: Vec<u64> = chained.iter().map(Event::get_seqnum).collect();
assert_eq!(places, [0, 1, 1, 2, 0]);
assert_eq!(chained[1].get_curruuid(), chained[2].get_curruuid(), "a twin, not a successor");
assert_eq!(chained[4].get_prevuuid(), None, "the fill ended the chain");

// A 10 ms grid: a view of the living order per tick, then its deadline.
const MS: i64 = 1_000_000;
let mut expiring = OrderEvent::at(T + 50 * MS);
expiring.set_crosscode("O-3003".to_owned());
expiring.set_exprtime(Some(T + 70 * MS));
expiring.finalize();
let timed: Vec<OrderEvent> = EventIterator::new([expiring], true).with_snapshot_ns(10 * MS).collect();
assert!(timed.iter().any(|held| held.get_snapunix() == Some(T + 60 * MS)));
let expired = timed.last().expect("the deadline event");
assert_eq!((expired.get_currunix(), expired.get_state().as_str()), (T + 70 * MS, "95EXPIRED"));
```

## Build a composite trade

`TradeEvent::from_parts` is the one door: a root event and its sided
executions, canonicalized so input order never changes the trade.

```rust
use yggdryl::graph::{Element, Event, ExecutionEvent, Market, OrderEvent, TradeEvent};
use yggdryl::{Decimal, Side};

const T: i64 = 1_700_000_000_000_000_000;
let fill = |code: &str, side: Side| -> yggdryl::Result<ExecutionEvent> {
    let mut execution = ExecutionEvent::at(T);
    execution.set_crosscode(code.to_owned());
    execution.set_side(side);
    execution.set_lastpx(Some("189.50".parse()?));
    execution.set_lastqty(Some(Decimal::from_int(100)));
    Ok(execution)
};
let mut root = OrderEvent::at(T);
root.set_crosscode("T-1".to_owned());
root.set_ticker(Some("AAPL".into()));
let trade = TradeEvent::from_parts(&root, vec![fill("E-SELL", Side::Sell)?, fill("E-BUY", Side::Buy)?])?;
let codes: Vec<&str> = trade.executions().iter().map(Element::get_crosscode).collect();
assert_eq!(codes, ["E-BUY", "E-SELL"]);
assert!(trade.is_execution());
let again = TradeEvent::from_parts(&root, vec![fill("E-BUY", Side::Buy)?, fill("E-SELL", Side::Sell)?])?;
assert_eq!(again.get_curruuid(), trade.get_curruuid());
// Each execution at the trade's instant; none at all is refused.
let mut late = fill("E-LATE", Side::Buy)?;
late.set_currunix(T + 1);
assert!(TradeEvent::from_parts(&root, vec![late]).unwrap_err().to_string().contains("$.executions[0].currunix"));
assert!(TradeEvent::from_parts(&root, Vec::new()).is_err());
```

## Carry any leaf as one value

`MarketData` is the enum over every leaf: `From` in, `TryFrom` or `as_*` out,
and it answers `Element` and `Market` by delegating, so generic code reads it
through the traits.

```rust
use yggdryl::graph::{Element, Market, MarketData, MarketKind, OrderEvent, QuoteEvent};
use yggdryl::Side;

// Generic over anything that stands in a market.
fn label(value: &(impl Element + Market)) -> String {
    format!("{}:{}", value.get_crosscode(), value.get_side().as_str())
}

let mut order = OrderEvent::at(1_700_000_000_000_000_000);
order.set_crosscode("O-1001".to_owned());
order.set_side(Side::Buy);
order.finalize();

let value = MarketData::from(order.clone());
assert_eq!(value.kind(), MarketKind::OrderEvent);
assert_eq!(value.kind().as_str(), "order_event");
assert!(value.is_event());
assert_eq!(value.as_order_event(), Some(&order));
assert_eq!(label(&value), label(&order));
assert!(QuoteEvent::try_from(value.clone()).is_err(), "another kind is refused at $.kind");
assert_eq!(OrderEvent::try_from(value)?, order);
```

## Write and read the marketdata row

`MarketData::arrow_reader` streams values into bounded batches of the lifted
row; `from_arrow_reader` reads any batch stream back, tolerant of a subset of
columns in any order.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use yggdryl::arrow::batch_reader;
use yggdryl::graph::{BookEvent, Element, Event, MarketData, Order, OrderEvent};

let mut order = OrderEvent::at(1_700_000_000_000_000_000);
order.set_crosscode("O-1001".to_owned());
order.finalize();
let mut undated = Order::new();
undated.finalize();
let values = vec![
    MarketData::from(undated),
    MarketData::from(order),
    MarketData::from(BookEvent::new(1_700_000_001_000_000_000, "AAPL")),
];

// 59 columns: kind, 16 event, 19 market, 8 operation, 5 book control, 3 book facts, 7 nested.
assert_eq!(MarketData::field()?.field_len(), 59);
let batches: Vec<RecordBatch> = MarketData::arrow_reader(values.clone(), Some(1_000), None)?.collect::<Result<_, _>>()?;
let read: Vec<MarketData> = MarketData::from_arrow_reader(batch_reader(batches[0].schema(), batches))?
    .collect::<yggdryl::Result<_>>()?;
assert_eq!(read, values);

// A foreign table: three columns, one the row does not name.
let schema = Arc::new(Schema::new(vec![
    Field::new("kind", DataType::Utf8, false),
    Field::new("currunix", DataType::Int64, false),
    Field::new("crosscode", DataType::Utf8, true),
    Field::new("msgtype", DataType::Utf8, true),
]));
let foreign = RecordBatch::try_new(Arc::clone(&schema), vec![
    Arc::new(StringArray::from(vec!["order_event"])),
    Arc::new(Int64Array::from(vec![1_700_000_000_000_000_000])),
    Arc::new(StringArray::from(vec!["O-1001"])),
    Arc::new(StringArray::from(vec!["D"])),
])?;
let lifted: Vec<MarketData> = MarketData::from_arrow_reader(batch_reader(schema, [foreign]))?.collect::<yggdryl::Result<_>>()?;
let event = lifted[0].as_order_event().expect("an order event");
assert_eq!((event.get_crosscode(), event.get_currunix()), ("O-1001", 1_700_000_000_000_000_000));
```

## Persist a marketdata stream and read it back

Any record medium stores the batches as they stream; reading back is the same
`from_arrow_reader`.

```rust
use yggdryl::graph::{Element, MarketData, OrderEvent};
use yggdryl::holder::Buffer;
use yggdryl::{IOBase, IOMedia, MimeType};

let values: Vec<MarketData> = (0..3_i64)
    .map(|at| {
        let mut order = OrderEvent::at(1_700_000_000_000_000_000 + at);
        order.set_crosscode(format!("O-{at}"));
        order.finalize();
        MarketData::from(order)
    })
    .collect();

let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
let options = handle.record_options()?;
handle.overwrite_arrow_reader(MarketData::arrow_reader(values.clone(), None, None)?, &options)?;
let read: Vec<MarketData> = MarketData::from_arrow_reader(handle.read_arrow_reader(&options)?)?
    .collect::<yggdryl::Result<_>>()?;
assert_eq!(read, values);
```

## Fold a sorted stream into books

`BookIterator` folds sorted operations into one `BookEvent` per touched
instant and symbol; depth persists, deltas and executions are each book's own.

```rust
use yggdryl::graph::{BookIterator, BookSide, Element, Event, ExecutionEvent, Market, MarketData, OrderEvent};
use yggdryl::{Decimal, Side};

const T: i64 = 1_700_000_000_000_000_000;
const SECOND: i64 = 1_000_000_000;
let bid = |unix: i64, code: &str, price: &str, quantity: i64| -> yggdryl::Result<MarketData> {
    let mut order = OrderEvent::at(unix);
    order.set_crosscode(code.to_owned());
    order.set_ticker(Some("AAPL".into()));
    order.set_side(Side::Buy);
    order.set_price(Some(price.parse()?));
    order.set_quantity(Some(Decimal::from_int(quantity)));
    order.finalize();
    Ok(MarketData::from(order))
};
let mut fill = ExecutionEvent::at(T + SECOND);
fill.set_crosscode("E-1".to_owned());
fill.set_ticker(Some("AAPL".into()));
fill.set_side(Side::Buy);
fill.set_lastpx(Some("189.52".parse()?));
fill.set_lastqty(Some(Decimal::from_int(100)));
fill.finalize();
let stream = vec![bid(T, "B-1", "189.48", 300)?, bid(T + SECOND, "B-2", "189.49", 200)?, MarketData::from(fill)];

let books = BookIterator::new(stream.clone().into_iter(), 0, false)?.collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!(books.len(), 2, "one book per touched instant");
let last = &books[1];
assert_eq!((last.get_currunix(), last.bid().len()), (T + SECOND, 2), "depth persists");
assert_eq!(last.bid().deltas().len(), 1);
assert_eq!(last.executions().iter().map(Element::get_crosscode).collect::<Vec<_>>(), ["E-1"]);

// A 500 ms grid adds the living book at each crossed tick.
let gridded = BookIterator::new(stream.into_iter(), 500, false)?.collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!(gridded.len(), 3);
// A value a book does not fold is refused by its kind.
let side = MarketData::from(BookSide::new(Side::Buy)?);
assert!(BookIterator::new([side].into_iter(), 0, false)?.next().expect("one result").is_err());
```

## Read a book

A side answers its bests, its `limits` (one per price, best first, the unpriced
market level last) and `depth`; a book its spread, midpoint and imbalance.

```rust
use yggdryl::graph::{BookEvent, Element, Market, MarketData, OrderEvent};
use yggdryl::{Decimal, Limit, Side};

const T: i64 = 1_700_000_000_000_000_000;
let entry = |code: &str, side: Side, price: Option<&str>, quantity: i64| -> yggdryl::Result<MarketData> {
    let mut order = OrderEvent::at(T);
    order.set_crosscode(code.to_owned());
    order.set_ticker(Some("AAPL".into()));
    order.set_side(side);
    order.set_price(price.map(str::parse).transpose()?);
    order.set_quantity(Some(Decimal::from_int(quantity)));
    order.finalize();
    Ok(MarketData::from(order))
};
let mut book = BookEvent::new(T, "AAPL");
book.add_operations([
    entry("B-1", Side::Buy, Some("189.48"), 300)?,
    entry("B-2", Side::Buy, Some("189.47"), 500)?,
    entry("A-1", Side::Sell, Some("189.52"), 100)?,
    entry("MKT", Side::Buy, None, 50)?,
])?;
let px = |text: &str| text.parse::<Decimal>();

assert_eq!(book.bid().best_price(), Some(px("189.48")?));
assert_eq!(book.spread(), Some(px("0.04")?));
assert_eq!(book.bbo_midpoint(), Some(px("189.50")?));
assert_eq!(book.get_price(), book.bbo_midpoint());
assert!(!book.is_locked() && !book.is_crossed());
assert_eq!(book.imbalance(1), Some(px("0.5")?));
let limits: Vec<Limit> = book.bid().limits().collect();
assert_eq!(limits.iter().map(|limit| limit.price).collect::<Vec<_>>(), [Some(px("189.48")?), Some(px("189.47")?), None]);
assert_eq!(book.bid().depth(2), Some(Decimal::from_int(800)));
assert_eq!(book.bid().depth(3), Some(Decimal::from_int(850)));
```

## Replace a scope with a snapshot

A `SnapshotEvent` clears its `(symbol, scope)` partition on both sides - an empty
FIX `W` is one - and the book records what it replaced.

```rust
use yggdryl::graph::{BookEvent, Element, Market, MarketData, MdUpdateAction, OrderEvent, SnapshotEvent, SnapshotPartition};
use yggdryl::{Decimal, Side};

const T: i64 = 1_700_000_000_000_000_000;
let order = |code: &str, side: Side, price: &str| -> yggdryl::Result<MarketData> {
    let mut order = OrderEvent::at(T);
    order.set_crosscode(code.to_owned());
    order.set_ticker(Some("AAPL".into()));
    order.set_side(side);
    order.set_price(Some(price.parse()?));
    order.set_quantity(Some(Decimal::from_int(100)));
    order.finalize();
    Ok(MarketData::from(order))
};
let mut book = BookEvent::new(T, "AAPL");
book.add_operations([order("B-1", Side::Buy, "189.48")?, order("A-1", Side::Sell, "189.52")?])?;

let mut at = OrderEvent::at(T + 1_000_000_000);
at.set_ticker(Some("AAPL".into()));
at.finalize();
let control = SnapshotEvent::snapshot(&at, None);
assert_eq!(control.book().action, Some(MdUpdateAction::Snapshot));

book.add_operations([MarketData::from(control)])?;
assert!(book.bid().is_empty() && book.ask().is_empty());
assert_eq!(book.get_price(), None);
let replaced = SnapshotPartition { symbol: Some("AAPL".into()), scope: "".into() };
assert!(book.snapshot_partitions().iter().eq([&replaced]));
```

## Read the stream through named views

A `MarketView` is one `Plan` over the `marketdata` row, applied by the
expression engine and bound once per reader; lifts turn nested facts into
columns.

```rust
use arrow_array::RecordBatch;
use yggdryl::graph::{Element, ExecutionEvent, Market, MarketData, MarketView, OrderEvent, TradeEvent};
use yggdryl::{FieldPath, Plan, SecType, SecurityId, Side};

const T: i64 = 1_700_000_000_000_000_000;
let mut order = OrderEvent::at(T);
order.set_crosscode("O-1001".to_owned());
order.set_side(Side::Buy);
order.insert_securityid(SecurityId::new(SecType::read("ISIN")?, "US0378331005")?)?;
order.finalize();
let fill = |code: &str, side: Side| {
    let mut execution = ExecutionEvent::at(T + 1_000_000_000);
    execution.set_crosscode(code.to_owned());
    execution.set_side(side);
    execution
};
let mut root = OrderEvent::at(T + 1_000_000_000);
root.set_crosscode("T-1".to_owned());
let trade = TradeEvent::from_parts(&root, vec![fill("E-1", Side::Buy), fill("E-2", Side::Sell)])?;
let stream = || MarketData::arrow_reader([MarketData::from(order.clone()), MarketData::from(trade.clone())], None, None);
let rows = |batches: Vec<RecordBatch>| batches.iter().map(RecordBatch::num_rows).sum::<usize>();

let lifts: Vec<FieldPath> = vec!["securityids['ISIN'] as isin".parse()?];
let orders = MarketData::apply_view(&MarketView::Orders, &lifts, stream()?)?;
assert_eq!(orders.schema().fields().last().map(|column| column.name().as_str()), Some("isin"));
assert_eq!(rows(orders.collect::<Result<_, _>>()?), 1);

// One row per execution, beside the trade's own columns.
let trades = MarketData::apply_view(&MarketView::Trades, &[], stream()?)?;
assert!(trades.schema().index_of("execution.crosscode").is_ok());
assert_eq!(rows(trades.collect::<Result<_, _>>()?), 2);

// A view is a plan whose text reads back as the same plan.
let plan = MarketData::plan(&MarketView::read("Trades", None)?, &[])?;
assert_eq!(plan.to_string().parse::<Plan>()?, plan);
let lifecycle = MarketView::read("lifecycle", Some("O-1001"))?;
assert_eq!(rows(MarketData::apply_view(&lifecycle, &[], stream()?)?.collect::<Result<_, _>>()?), 1);
```

## Turn a FIX capture into books

A FIX capture reaches the graph through the codec: `lifecycle` settles each
message, `book_arrow_reader` folds sorted messages into `book_event` rows, and
`MarketData::from_arrow_reader` reads the books back.

```rust
use std::sync::Arc;

use yggdryl::graph::MarketData;
use yggdryl::local::LocalFolder;
use yggdryl::{FixCodec, FixMsg, FixRegistry};

// `config/fix` of a yggdryl checkout (see the yggdryl-fix skill).
let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?));
let lines = [
    "8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|",
    "8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|",
];
let capture: Vec<FixMsg> = codec.parse_lines(lines).collect::<yggdryl::Result<_>>()?;

let rows = codec.book_arrow_reader(codec.lifecycle(capture), 0, false)?;
let books: Vec<MarketData> = MarketData::from_arrow_reader(rows)?.collect::<yggdryl::Result<_>>()?;
assert_eq!(books.len(), 2);
let last = books[1].as_book_event().expect("a book row");
assert_eq!(last.bid().best_price().map(|price| price.to_string()).as_deref(), Some("101"));
assert_eq!(last.executions().len(), 1);
```

## Serve a replay or mount the timeline

JavaScript-only: the replay service is `yggdryl/replay` and the browser
component `yggdryl/web/book-timeline.js` in the npm package. From Rust, write
the sorted **operations** (not the books - the service walks them itself) as
`marketdata` rows to a `.parquet` or `.arrow` file and point
`node node_modules/yggdryl/replay.js <file>` at it.

## Gotchas in Rust

- Setters never finalize: a leaf with stale derived facts is refused when
  written to Arrow. Call `finalize()` after the last `set_*`.
- `EventIterator::new(items, false)` collects to sort; pass `true` only for a
  stream you know is sorted, so it streams.
- `BookIterator::new` takes an iterator (`.into_iter()`) of `MarketData` or
  `Result<MarketData>` and yields `Result<BookEvent>`.
- `with_previous`/`merge_with` answer `Option`: `None` means nothing moved (its
  own predecessor, an earlier event, another element), not an error.
- `insert_securityid`, `insert_altid` fill an absent key only and answer
  whether they did; `set_securityids`/`set_altids` replace the whole set.
- The traits are object-safe except the verbs that take or return `Self`
  (`with_previous`, `merge_with`, `is_after` ...): `&dyn Event` reads every fact.
