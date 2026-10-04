# yggdryl-market-data in Rust

The leaves, `MarketData`, the iterators and views live in `yggdryl::graph`;
`Decimal`, `Side`, `MarketDataKind`, `State`, `Ccy`, `Mic`, `IdType`,
`IdSource`, `Identifier`, `Identifiers`, `Uuid` and `Limit` at the crate root. Getters and setters are
trait methods: import `yggdryl::graph::{Element, Event, Market, Operation}` as
needed. Setters do not finalize - call `finalize()` once the facts are in.

## Build an order event from named facts

Set the facts, then `finalize` derives the identity and what the facts imply:
the national number an ISIN embeds, the side an order's or an execution's
cross code is stored under.

```rust
use yggdryl::graph::{Element, Event, Market, Operation, OrderEvent};
use yggdryl::{Ccy, Decimal, IdKey, IdType, Identifier, MarketDataKind, Side, State};

// Instants are i64 nanoseconds since the Unix epoch, UTC.
const T: i64 = 1_700_000_000_000_000_000;
let mut order = OrderEvent::at(T);
order.set_crosscode("O-1001".to_owned());
order.set_side(Side::Buy, true);
order.set_price(Some("189.50".parse()?), true);
order.set_quantity(Some(Decimal::from_int(100)), true);
order.set_currency(Ccy::new("USD")?, true);
order.set_ticker(Some("AAPL".into()), true);
// An identifier is a source, a type and a value, unique by `src:type`; a code is held to its type's shape.
order.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "US0378331005")?)?;
order.insert_identifier(Identifier::new(IdKey::base(IdType::OrderId), "O-1001")?)?;
order.finalize();

// A dated identity is a UUIDv7: its millisecond leads.
assert_eq!(order.get_curruuid(), order.time_uuid()?);
assert!(order.get_curruuid().to_string().starts_with("018bcfe5-6800-7"));
// A cross code is stored as `{kind}:{side}:{base}`: one chain per side.
assert_eq!(order.get_crosscode(), "10:1:O-1001");
assert_ne!(order.get_crossuuid(), order.get_curruuid(), "the cross code names a chain");
// Derived on finalize: the CUSIP inside the ISIN; the ISIN itself reads as `isincode`.
assert_eq!(order.get_securityids().to_string(), "[cusip=037833100, derived:cusip=037833100, isin=US0378331005]");
assert_eq!(order.get_isincode(), Some("US0378331005"));
assert_eq!(order.get_lastpx(), None, "a price is never a last execution");
assert_eq!(order.get_bidpx(), order.get_price(), "a buy's price is its bid");
assert!(order.get_fxrates().is_empty(), "nothing fills the rates");
assert_eq!(*order.get_state(), State::Unknown);
assert_eq!(order.kind().marketdatakind(), MarketDataKind::Order);
```

## Build undated leaves, quotes and book entries

An undated leaf's identity is its content; `at` dates it. A quote is one
element holding a bid and an ask leg - the six `bid*`/`ask*` facts - stored
under side `0`; its `side` is a tag, so a quote stating a side and a price
states the leg that side takes. A market-data entry carries a `BookRef`.

```rust
use yggdryl::graph::{BookRef, Element, Market, MarketKind, MdUpdateAction, Order, OrderEvent, QuoteEvent};
use yggdryl::{Ccy, Decimal, MarketDataKind, Side, Uuid};

const T: i64 = 1_700_000_000_000_000_000;
let mut order = Order::new();
order.set_crosscode("O-1001".to_owned());
order.set_side(Side::Buy, true);
order.set_price(Some("189.50".parse()?), true);
order.finalize();
assert_eq!(order.kind(), MarketKind::Order);
assert_eq!(order.get_curruuid(), Uuid::from_v8(u128::from(order.get_currhashcode())));
let event: OrderEvent = order.at(T);
assert!(!event.is_after(&event));

// A two-sided quote: its bid and ask are its two legs, and it tags no side.
let mut quote = QuoteEvent::at(T);
quote.set_crosscode("Q-7".to_owned());
quote.set_ticker(Some("AAPL".into()), true);
quote.set_bidpx(Some("189.48".parse()?), true);
quote.set_bidqty(Some(Decimal::from_int(300)), true);
quote.set_bidccy(Some(Ccy::new("USD")?), true);
quote.set_askpx(Some("189.52".parse()?), true);
quote.set_askqty(Some(Decimal::from_int(100)), true);
quote.set_askccy(Some(Ccy::new("USD")?), true);
quote.finalize();
assert_eq!((quote.get_side(), quote.get_crosscode()), (Side::Unknown, "14:0:Q-7"));
assert_eq!(quote.kind().marketdatakind(), MarketDataKind::Quotation);

// An offer: tagged `SELL`, its price is its ask leg; a quote's code stays under side 0.
let mut offer = QuoteEvent::at(T);
offer.set_crosscode("Q-8".to_owned());
offer.set_ticker(Some("AAPL".into()), true);
offer.set_side(Side::Sell, true);
offer.set_price(Some("189.52".parse()?), true);
offer.set_quantity(Some(Decimal::from_int(100)), true);
offer.finalize();
assert_eq!(offer.get_crosscode(), "14:0:Q-8");
assert_eq!(offer.get_askpx(), Some("189.52".parse()?));
// Placed in a book by its control; the scope is a fact, the rest walk-time.
let mut entry = offer.clone().with_book(BookRef {
    action: MdUpdateAction::read("new"),
    position: Some(1),
    scope: Some("L2".into()),
    ..BookRef::default()
});
entry.finalize();
assert_eq!(entry.action().map(MdUpdateAction::as_str), Some("0"));
assert_eq!(entry.scope(), "L2");
assert_ne!(entry.get_curruuid(), offer.get_curruuid(), "the scope digests");
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
    event.set_side(Side::Buy, true);
    event.set_price(Some("189.50".parse()?), true);
    event.set_quantity(Some(Decimal::from_int(100)), true);
    event.finalize();
    Ok(event)
};
let placed = event(T, "New")?;
let filled = event(T + 1_000_000_000, "PartiallyFilled")?.with_previous(&placed).expect("a later event follows");
// A later instant keeps its own place: the first there.
assert_eq!((filled.get_prevuuid(), filled.get_prevunix(), filled.get_seqnum()), (Some(placed.get_curruuid()), Some(T), 0));
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
`identifiers`), yields a twin as a restatement rather than a successor, retires a
chain at a terminal state and emits one `EXPIRED` at a deadline. An order's or
an execution's chain is keyed by side - a quote's is one chain whatever side
it tags - a chain lives within one `marketdatakind` (an order and an execution
under one cross code are two chains), and every walked element leaves stating
`creaunix`.

```rust
use yggdryl::graph::{Element, Event, EventIterator, Market, OrderEvent};
use yggdryl::{Side, State};

const T: i64 = 1_700_000_000_000_000_000;
const SECOND: i64 = 1_000_000_000;
let event = |second: i64, order: &str, side: Side, state: &str| {
    let mut event = OrderEvent::at(T + second * SECOND);
    event.set_crosscode(order.to_owned());
    event.set_side(side, true);
    event.set_state(State::from_spelling(state).expect("a shipped state"));
    event.finalize();
    event
};
// Unsorted: one report logged twice, and O-1001 reopened after its fill.
let arrived = vec![
    event(3, "O-1001", Side::Unknown, "Filled"),
    event(0, "O-1001", Side::Unknown, "New"),
    event(2, "O-1001", Side::Unknown, "PartiallyFilled"),
    event(2, "O-1001", Side::Unknown, "PartiallyFilled"),
    event(4, "O-1001", Side::Unknown, "New"),
];
let chained: Vec<OrderEvent> = EventIterator::new(arrived, false).collect();
// Each step is at an instant of its own, so each is the first there; the
// chain is in `prevuuid`.
let places: Vec<u64> = chained.iter().map(Event::get_seqnum).collect();
assert_eq!(places, [0, 0, 0, 0, 0]);
assert_eq!(chained[3].get_prevuuid(), Some(chained[1].get_curruuid()));
assert_eq!(chained[1].get_curruuid(), chained[2].get_curruuid(), "a twin, not a successor");
assert_eq!(chained[4].get_prevuuid(), None, "the fill ended the chain");
// A chain's creation instant is its first element's, carried along it.
assert!(chained[..4].iter().all(|held| held.get_creaunix() == Some(T)));

// One identifier, two sides: two chains. A report stating no side joins the
// one side alive under its code, and a NEW over a live NEW reads UPDATED.
let walked: Vec<OrderEvent> = EventIterator::new(
    vec![
        event(0, "O-2002", Side::Buy, "New"),
        event(1, "O-2002", Side::Sell, "New"),
        event(2, "O-2002", Side::Buy, "New"),
    ],
    true,
)
.collect();
assert_eq!((walked[1].get_crosscode(), walked[1].get_seqnum()), ("10:2:O-2002", 0));
assert_eq!(walked[2].get_prevuuid(), Some(walked[0].get_curruuid()));
assert_eq!(*walked[2].get_state(), State::Updated);
let joined: Vec<OrderEvent> = EventIterator::new(
    vec![event(0, "O-3003", Side::Buy, "New"), event(1, "O-3003", Side::Unknown, "Canceled")],
    true,
)
.collect();
assert_eq!(joined[1].get_prevuuid(), Some(joined[0].get_curruuid()));
assert_eq!((joined[1].get_side(), joined[1].get_crosscode()), (Side::Buy, "10:1:O-3003"));

// A 10 ms grid: a view of the living order per tick, then its deadline.
const MS: i64 = 1_000_000;
let mut expiring = OrderEvent::at(T + 50 * MS);
expiring.set_crosscode("O-4004".to_owned());
expiring.set_exprunix(Some(T + 70 * MS));
expiring.finalize();
let timed: Vec<OrderEvent> = EventIterator::new([expiring], true).with_snapshot_ns(10 * MS).collect();
let view = timed
    .iter()
    .find(|held| held.get_snapunix().is_some() && held.get_currunix() == T + 60 * MS)
    .expect("a view");
// Dated at its tick: the identity is the tick's, the content the order's,
// and its snapshot instant the one the order was stated at.
assert_eq!(view.get_snapunix(), Some(T + 50 * MS));
assert_eq!((view.get_currunix(), view.get_seqnum()), (T + 60 * MS, 0));
assert_eq!(view.get_currhashcode(), timed[0].get_currhashcode());
let expired = timed.last().expect("the deadline event");
assert_eq!((expired.get_currunix(), *expired.get_state()), (T + 70 * MS, State::Expired));
```

## Build a composite trade

`TradeEvent::from_parts` is the one door: a root event and its sided
executions, canonicalized so input order never changes the trade. The trade
itself is not sided: it keeps the root's base code under its own kind and side
`0` (`21:0:T-1`).

```rust
use yggdryl::graph::{Element, Event, ExecutionEvent, Market, OrderEvent, TradeEvent};
use yggdryl::{Decimal, Side};

const T: i64 = 1_700_000_000_000_000_000;
let fill = |code: &str, side: Side| -> yggdryl::Result<ExecutionEvent> {
    let mut execution = ExecutionEvent::at(T);
    execution.set_crosscode(code.to_owned());
    execution.set_side(side, true);
    execution.set_lastpx(Some("189.50".parse()?), true);
    execution.set_lastqty(Some(Decimal::from_int(100)), true);
    Ok(execution)
};
let mut root = OrderEvent::at(T);
root.set_crosscode("T-1".to_owned());
root.set_ticker(Some("AAPL".into()), true);
let trade = TradeEvent::from_parts(&root, vec![fill("E-SELL", Side::Sell)?, fill("E-BUYS", Side::Buy)?])?;
let codes: Vec<&str> = trade.executions().iter().map(Element::get_crosscode).collect();
assert_eq!(codes, ["8:1:E-BUYS", "8:2:E-SELL"]);
assert!(trade.is_execution());
let again = TradeEvent::from_parts(&root, vec![fill("E-BUYS", Side::Buy)?, fill("E-SELL", Side::Sell)?])?;
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
use yggdryl::{MarketDataKind, Side};

// Generic over anything that stands in a market.
fn label(value: &(impl Element + Market)) -> String {
    format!("{}/{}", value.get_crosscode(), value.get_side().as_str())
}

let mut order = OrderEvent::at(1_700_000_000_000_000_000);
order.set_crosscode("O-1001".to_owned());
order.set_side(Side::Buy, true);
order.finalize();

let value = MarketData::from(order.clone());
assert_eq!(value.kind(), MarketKind::OrderEvent);
assert_eq!(value.kind().as_str(), "order_event");
assert_eq!(value.marketdatakind(), MarketDataKind::Order);
assert!(value.is_event());
assert_eq!(value.as_order_event(), Some(&order));
assert_eq!(label(&value), "10:1:O-1001/BUYS");
assert_eq!(label(&value), label(&order));
assert!(QuoteEvent::try_from(value.clone()).is_err(), "another kind is refused");
assert_eq!(OrderEvent::try_from(value)?, order);
```

## Write and read the marketdata row

`MarketData::arrow_reader` streams values into bounded batches of the lifted
row; `from_arrow_reader` reads any batch stream back, tolerant of a subset of
columns in any order. `marketdatakind` and, for a dated leaf, `currunix` are
the minimum.

```rust
use std::sync::Arc;

use arrow_array::{Int32Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use yggdryl::arrow::batch_reader;
use yggdryl::graph::{BookEvent, Element, Event, MarketData, Order, OrderEvent};
use yggdryl::MarketDataKind;

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

// 62 columns: 6 element, 9 event, 34 market (marketdatakind first), 5 operation,
// the book controls bookscope, bookaction and bookposition, 5 nested.
let field = MarketData::field()?;
assert_eq!(field.field_len(), 62);
assert_eq!(field.fields()[15].name(), "marketdatakind");
let batches: Vec<RecordBatch> = MarketData::arrow_reader(values.clone(), Some(1_000), None)?.collect::<Result<_, _>>()?;
let read: Vec<MarketData> = MarketData::from_arrow_reader(batch_reader(batches[0].schema(), batches))?
    .collect::<yggdryl::Result<_>>()?;
assert_eq!(read, values);

// A foreign table: three columns, one the row does not name.
let schema = Arc::new(Schema::new(vec![
    Field::new("marketdatakind", DataType::Int32, false),
    Field::new("currunix", DataType::Int64, false),
    Field::new("crosscode", DataType::Utf8, true),
    Field::new("msgtype", DataType::Utf8, true),
]));
let foreign = RecordBatch::try_new(Arc::clone(&schema), vec![
    // Any integer column reads as codes.
    Arc::new(Int32Array::from(vec![i32::from(MarketDataKind::Order.code())])),
    Arc::new(Int64Array::from(vec![1_700_000_000_000_000_000])),
    Arc::new(StringArray::from(vec!["10:0:O-1001"])),
    Arc::new(StringArray::from(vec!["D"])),
])?;
let lifted: Vec<MarketData> = MarketData::from_arrow_reader(batch_reader(schema, [foreign]))?.collect::<yggdryl::Result<_>>()?;
let event = lifted[0].as_order_event().expect("a dated ORDR row is an order event");
assert_eq!((event.get_crosscode(), event.get_currunix()), ("10:0:O-1001", 1_700_000_000_000_000_000));
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

`BookIterator` folds sorted orders and quotes into one `BookEvent` per
instant and book key that moved it - the instrument's ISIN, else its ticker,
else `XX0000000000` - pruning every execution and trade. A book is complete
(`is_complete`) only at a snapshot tick; every other book states its deltas
alone beside the top of book they settled on, and `with_previous` over the
complete book before it rebuilds it whole. A filter over the `marketdata` row
narrows what folds.

```rust
use yggdryl::graph::{BookEvent, BookIterator, Element, Event, ExecutionEvent, Market, MarketData, Order, OrderEvent};
use yggdryl::{Decimal, IdKey, IdType, Identifier, Isin, Side, State};

const T: i64 = 1_700_000_000_000_000_000;
const SECOND: i64 = 1_000_000_000;
let bid = |unix: i64, code: &str, price: &str, quantity: i64| -> yggdryl::Result<MarketData> {
    let mut order = OrderEvent::at(unix);
    order.set_crosscode(code.to_owned());
    order.set_ticker(Some("AAPL".into()), true);
    order.set_side(Side::Buy, true);
    order.set_price(Some(price.parse()?), true);
    order.set_quantity(Some(Decimal::from_int(quantity)), true);
    order.set_state(State::New);
    order.finalize();
    Ok(MarketData::from(order))
};
let mut fill = ExecutionEvent::at(T + SECOND);
fill.set_crosscode("E-1".to_owned());
fill.set_ticker(Some("AAPL".into()), true);
fill.set_side(Side::Buy, true);
fill.set_lastpx(Some("189.52".parse()?), true);
fill.set_lastqty(Some(Decimal::from_int(100)), true);
fill.finalize();
let stream = vec![bid(T, "B-1", "189.48", 300)?, bid(T + SECOND, "B-2", "189.49", 200)?, MarketData::from(fill)];

let books = BookIterator::new(stream.clone().into_iter(), 0)?.collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!(books.len(), 2, "one book per instant that moved it; the execution is a delta of its own");
// No grid and no snapshot input: each book states its deltas alone and its top of book.
let last = &books[1];
assert!(!last.is_complete());
assert_eq!((last.get_currunix(), last.deltas().len(), last.alive().count()), (T + SECOND, 2, 0));
assert_eq!(last.best_price(Side::Buy), Some("189.49".parse()?));
// Rebuilt whole: the first over the empty book its key starts from, the next over it.
assert_eq!(books[0].get_prevuuid(), None);
let first = books[0].clone().with_previous(&BookEvent::keyed(T, "AAPL")).expect("a rebuild");
let whole = last.clone().with_previous(&first).expect("a rebuild");
assert!(whole.is_complete());
assert_eq!((whole.alive().count(), whole.get_curruuid()), (2, last.get_curruuid()), "depth persists");

// A 500 ms grid states the whole living book at each crossed tick.
let gridded = BookIterator::new(stream.clone().into_iter(), 500)?.collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!(gridded.len(), 3);
assert!(gridded.iter().all(BookEvent::is_complete));
// A filter narrows what folds, and never admits an execution.
let filtered = BookIterator::new(stream.into_iter(), 0)?.with_filter("marketdatakind = 'EXEC'")?;
assert_eq!(filtered.count(), 0);

// The book key: the instrument's ISIN, else the ticker, else `XX0000000000`.
let mut listed = OrderEvent::at(T);
listed.set_crosscode("L-1".to_owned());
listed.set_side(Side::Sell, true);
assert_eq!(listed.book_crosscode(), Isin::NONE);
listed.set_ticker(Some("AAPL".into()), true);
assert_eq!(listed.book_crosscode(), "AAPL");
listed.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "US0378331005")?)?;
listed.finalize();
let [book] = BookIterator::new([MarketData::from(listed)].into_iter(), 0)?.collect::<yggdryl::Result<Vec<_>>>()?.try_into().expect("one book");
assert_eq!((book.get_crosscode(), book.get_isincode(), book.get_ticker()), ("3:0:US0378331005", Some("US0378331005"), Some("AAPL")));
// A value a book does not fold - an undated order - is refused by its kind.
let undated = MarketData::from(Order::new());
assert!(BookIterator::new([undated].into_iter(), 0)?.next().expect("one result").is_err());
```

## Read a book

A complete book answers each side as its `limits` (one per price, best
first, the unpriced market level last) and its entries as `alive_on(side)`;
every book answers the readings of the first level that can trade:
`best_price`, `best_quantity`, the `bidpx`/`askpx` it states, `spread`; a
complete one `depth` and `imbalance` too. A book built by hand is complete.

```rust
use yggdryl::graph::{BookEvent, Element, Market, MarketData, Operation, OrderEvent};
use yggdryl::{Decimal, Limit, Side};

const T: i64 = 1_700_000_000_000_000_000;
let entry = |code: &str, side: Side, price: Option<&str>, quantity: i64| -> yggdryl::Result<OrderEvent> {
    let mut order = OrderEvent::at(T);
    order.set_crosscode(code.to_owned());
    order.set_ticker(Some("AAPL".into()), true);
    order.set_side(side, true);
    order.set_price(price.map(str::parse).transpose()?, true);
    order.set_quantity(Some(Decimal::from_int(quantity)), true);
    Ok(order)
};
let done = |mut order: OrderEvent| {
    order.finalize();
    MarketData::from(order)
};
let mut halted = entry("B-0", Side::Buy, Some("189.49"), 10)?;
halted.set_tradable(Some(false), true);
let mut book = BookEvent::new(T, "AAPL");
book.add_operations([
    done(halted),
    done(entry("B-1", Side::Buy, Some("189.48"), 300)?),
    done(entry("B-2", Side::Buy, Some("189.47"), 500)?),
    done(entry("A-1", Side::Sell, Some("189.52"), 100)?),
    done(entry("MKT", Side::Buy, None, 50)?),
])?;
let px = |text: &str| text.parse::<Decimal>();

// The level at 189.49 cannot trade: the best bid is the first that can.
let limits: Vec<Limit> = book.limits(Side::Buy).collect();
assert_eq!(limits.iter().map(|limit| limit.price).collect::<Vec<_>>(), [Some(px("189.49")?), Some(px("189.48")?), Some(px("189.47")?), None]);
assert!(!limits[0].tradable && limits[1].tradable);
assert_eq!(book.best_price(Side::Buy), Some(px("189.48")?));
assert_eq!((book.get_bidpx(), book.get_bidqty()), (Some(px("189.48")?), Some(Decimal::from_int(300))));
assert_eq!(book.get_askpx(), Some(px("189.52")?));
assert_eq!(book.spread(), Some(px("0.04")?));
assert_eq!(book.bbo_midpoint(), Some(px("189.50")?));
assert_eq!(book.get_price(), book.bbo_midpoint());
assert!(!book.is_locked() && !book.is_crossed());
assert_eq!(book.depth(Side::Buy, 2), Some(Decimal::from_int(310)));
assert!(book.is_complete());
assert_eq!((book.alive().count(), book.alive_on(Side::Buy).len(), book.alive_on(Side::Sell).len()), (5, 4, 1));
// The deltas are the five orders, in the order applied.
assert_eq!(book.deltas().map(Element::get_crosscode).collect::<Vec<_>>(), ["10:1:B-0", "10:1:B-1", "10:1:B-2", "10:2:A-1", "10:1:MKT"]);
```

## Replace a scope with a snapshot

A `SnapshotEvent` clears its `(book, scope)` partition on both sides - an
empty FIX `W` is one.

```rust
use yggdryl::graph::{BookEvent, Element, Market, MarketData, MdUpdateAction, OrderEvent, SnapshotEvent};
use yggdryl::{Decimal, Side};

const T: i64 = 1_700_000_000_000_000_000;
let order = |code: &str, side: Side, price: &str| -> yggdryl::Result<MarketData> {
    let mut order = OrderEvent::at(T);
    order.set_crosscode(code.to_owned());
    order.set_ticker(Some("AAPL".into()), true);
    order.set_side(side, true);
    order.set_price(Some(price.parse()?), true);
    order.set_quantity(Some(Decimal::from_int(100)), true);
    order.finalize();
    Ok(MarketData::from(order))
};
let mut book = BookEvent::new(T, "AAPL");
book.add_operations([order("B-1", Side::Buy, "189.48")?, order("A-1", Side::Sell, "189.52")?])?;
assert_eq!(book.alive().count(), 2);

let mut at = OrderEvent::at(T + 1_000_000_000);
at.set_ticker(Some("AAPL".into()), true);
at.finalize();
let control = SnapshotEvent::snapshot(&at, None);
assert_eq!(control.book().action, Some(MdUpdateAction::Snapshot));

book.add_operations([MarketData::from(control)])?;
assert_eq!(book.alive().count(), 0);
assert_eq!((book.get_price(), book.get_bidpx(), book.get_askpx()), (None, None, None));
```

## Read the stream through named views

A `MarketView` is one `Plan` over the `marketdata` row, applied by the
expression engine and bound once per reader; lifts turn nested facts into
columns.

```rust
use arrow_array::RecordBatch;
use yggdryl::graph::{Element, ExecutionEvent, Market, MarketData, MarketView, Operation, OrderEvent, TradeEvent};
use yggdryl::{FieldPath, IdKey, IdType, Identifier, Plan, Side};

const T: i64 = 1_700_000_000_000_000_000;
let mut order = OrderEvent::at(T);
order.set_crosscode("O-1001".to_owned());
order.set_side(Side::Buy, true);
order.insert_identifier(Identifier::new(IdKey::base(IdType::ClOrdId), "C-1")?)?;
order.finalize();
let fill = |code: &str, side: Side| {
    let mut execution = ExecutionEvent::at(T + 1_000_000_000);
    execution.set_crosscode(code.to_owned());
    execution.set_side(side, true);
    execution
};
let mut root = OrderEvent::at(T + 1_000_000_000);
root.set_crosscode("T-1".to_owned());
let trade = TradeEvent::from_parts(&root, vec![fill("E-1", Side::Buy), fill("E-2", Side::Sell)])?;
let stream = || MarketData::arrow_reader([MarketData::from(order.clone()), MarketData::from(trade.clone())], None, None);
let rows = |batches: Vec<RecordBatch>| batches.iter().map(RecordBatch::num_rows).sum::<usize>();

// A lift reaches one identifier of the map by its key.
let lifts: Vec<FieldPath> = vec!["identifiers['clordid'] as clordid".parse()?];
let orders = MarketData::apply_view(&MarketView::Orders, &lifts, stream()?)?;
assert_eq!(orders.schema().fields().last().map(|column| column.name().as_str()), Some("clordid"));
assert_eq!(rows(orders.collect::<Result<_, _>>()?), 1);

// One row per execution, beside the trade's own columns.
let trades = MarketData::apply_view(&MarketView::Trades, &[], stream()?)?;
assert!(trades.schema().index_of("execution.crosscode").is_ok());
assert_eq!(rows(trades.collect::<Result<_, _>>()?), 2);

// A view is a plan whose text reads back as the same plan.
let plan = MarketData::plan(&MarketView::read("Trades", None)?, &[])?;
assert_eq!(plan.to_string().parse::<Plan>()?, plan);
// A lifecycle follows the cross code as stored: side included.
let lifecycle = MarketView::read("lifecycle", Some("10:1:O-1001"))?;
assert_eq!(rows(MarketData::apply_view(&lifecycle, &[], stream()?)?.collect::<Result<_, _>>()?), 1);
```

## Turn a FIX capture into books

A FIX capture reaches the graph through the codec: `lifecycle` settles each
message, `book_arrow_reader` folds sorted messages into book rows - orders,
quotes and `W`/`X` entries, a trade entry pruned - and
`MarketData::from_arrow_reader` reads the books back. A `W` full refresh is a
snapshot input, so its book is complete; the `X` after it states its delta.

```rust
use std::sync::Arc;

use yggdryl::graph::MarketData;
use yggdryl::local::LocalFolder;
use yggdryl::{FixCodec, FixMsg, FixRegistry, MarketDataKind, Side};

// `config/fix` of a yggdryl checkout (see the yggdryl-fix skill).
let dictionary = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
let codec = FixCodec::new(Arc::new(FixRegistry::from_handle(&LocalFolder::new(dictionary)?)?));
let lines = [
    "8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|",
    "8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|",
];
let capture: Vec<FixMsg> = codec.parse_lines(lines).collect::<yggdryl::Result<_>>()?;

let rows = codec.book_arrow_reader(codec.lifecycle(capture), 0, None)?;
let books: Vec<MarketData> = MarketData::from_arrow_reader(rows)?.collect::<yggdryl::Result<_>>()?;
assert_eq!(books.len(), 2);
assert!(books.iter().all(|book| book.marketdatakind() == MarketDataKind::Book));
let first = books[0].as_book_event().expect("a book row");
assert!(first.is_complete());
let last = books[1].as_book_event().expect("a book row");
assert_eq!(last.best_price(Side::Buy).map(|price| price.to_string()).as_deref(), Some("101"));
// The bid's change and the trade entry (`269=2`) are its two deltas.
assert!(!last.is_complete());
assert_eq!(last.deltas().len(), 2);
```

## Fold books into candles

`CandleIterator` folds books sorted by their instant into one `Candle` per
book cross code and bucket: the best bid, the best ask, the mid and the spread
each an `Ohlc`, the touch when the bucket closed and how many books folded. It
reads a book's top of book alone, so delta books fold as complete ones do.
`CandleOptions` is the interval and the zone whose wall clock the buckets
align to; `Candle::field()` is the twenty-three-cell row candles cross as.

```rust
use yggdryl::graph::{BookIterator, Candle, CandleIterator, CandleOptions, Element, Event, Market, MarketData, QuoteEvent};
use yggdryl::{ArrowCastOptions, Decimal, Serie, Side, State, Timezone};

// 2023-11-14T22:13:20Z.
const T: i64 = 1_700_000_000_000_000_000;
const SECOND: i64 = 1_000_000_000;
let quote = |unix: i64, code: &str, side: Side, price: &str, quantity: i64| -> yggdryl::Result<MarketData> {
    let mut quote = QuoteEvent::at(unix);
    quote.set_crosscode(code.to_owned());
    quote.set_ticker(Some("AAPL".into()), true);
    quote.set_side(side, true);
    quote.set_price(Some(price.parse()?), true);
    quote.set_quantity(Some(Decimal::from_int(quantity)), true);
    quote.set_state(State::New);
    quote.finalize();
    Ok(MarketData::from(quote))
};
let stream = vec![
    quote(T, "B1", Side::Buy, "189.48", 300)?,
    quote(T, "A1", Side::Sell, "189.52", 100)?,
    quote(T + 20 * SECOND, "B2", Side::Buy, "189.50", 200)?,
    quote(T + 70 * SECOND, "A2", Side::Sell, "189.51", 50)?,
];
// Three books: the two quotes at T share one instant.
let books = || BookIterator::new(stream.clone().into_iter(), 0);

// Minute candles in UTC: the 22:13 and 22:14 buckets.
let candles = CandleIterator::new(books()?, CandleOptions::from_spelling("1m")?).collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!(candles.len(), 2);
let (first, second) = (&candles[0], &candles[1]);
assert_eq!((first.crosscode.as_str(), first.start, first.end), ("3:0:AAPL", 1_699_999_980 * SECOND, 1_700_000_040 * SECOND));
let bid = first.bid.expect("two books stated a bid");
assert_eq!((bid.open, bid.high, bid.low, bid.close), ("189.48".parse()?, "189.50".parse()?, "189.48".parse()?, "189.50".parse()?));
assert_eq!(first.spread.map(|spread| spread.close), Some("0.02".parse()?));
assert_eq!((first.bidqty, first.askqty), (Some(Decimal::from_int(200)), Some(Decimal::from_int(100))));
assert_eq!(first.books, 2);
// A2 undercuts A1: the second bucket's ask opens at 189.51 and the spread narrows.
assert_eq!(second.ask.map(|ask| ask.open), Some("189.51".parse()?));
assert_eq!(second.mid.map(|mid| mid.close), Some("189.505".parse()?));

// Buckets align to the zone's wall clock: 22:13:20Z is 23:13:20 in Zurich,
// so its daily candle opens at Zurich midnight, 23:00Z the day before.
let zurich = CandleOptions::from_spelling("1d")?.with_timezone(Timezone::from_str("Europe/Zurich")?);
let daily = CandleIterator::new(books()?, zurich).collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!((daily.len(), daily[0].start, daily[0].books), (1, 1_699_916_400 * SECOND, 3));
let utc = CandleIterator::new(books()?, CandleOptions::from_spelling("1d")?).collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!(utc[0].start, 1_699_920_000 * SECOND);

// Candles cross as rows of `Candle::field()`, and read back as the same values.
let field = Candle::field()?;
assert_eq!(field.field_len(), 23);
let batch = Candle::arrow_reader(candles.clone().into_iter().map(Ok), None)?.next().expect("one batch")?;
let rows = Serie::from_arrow_batch(Some(&field), &batch, ArrowCastOptions::default())?;
assert_eq!(Candle::from_scalar(&rows.scalar(1)?)?, candles[1]);
```

## Serve a table of books

`BookService` is the HTTP face of a `marketdata` table, keyed by the book key
(the ISIN, else the ticker, else `XX0000000000`) - the keys it holds, the
candles of a key over a range, the book at an instant rebuilt whole, the audit
of every alive entry and delta - and every reading is a method, so a program
asks without HTTP what the display's routes answer. A `ticker` argument names
a key, else the ticker one key's books state. `yggdryl market serve` is the
same service with the display in front of it.

```rust
use std::sync::Arc;

use yggdryl::graph::{BookIterator, BookQuery, BookService, BookServiceOptions, Element, Event, Market, MarketData, QuoteEvent};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::http::{Request, Server, Status};
use yggdryl::{Decimal, IOMedia, Side, State, Timezone, Url};

const SECOND: i64 = 1_000_000_000;
// 2026-01-05T10:00:00Z.
const T0: i64 = 1_767_607_200 * SECOND;
let quote = |unix: i64, code: &str, side: Side, price: i64| -> MarketData {
    let mut quote = QuoteEvent::at(unix);
    quote.set_crosscode(code.to_owned());
    quote.set_ticker(Some("ACME".into()), true);
    quote.set_side(side, true);
    quote.set_price(Some(Decimal::from_int(price)), true);
    quote.set_quantity(Some(Decimal::from_int(10)), true);
    quote.set_state(State::New);
    quote.finalize();
    MarketData::from(quote)
};
// Books written as `marketdata` rows into any record location: here a buffer named `books.arrows`.
let books = BookIterator::new(
    vec![quote(T0 + 5 * SECOND, "B1", Side::Buy, 100), quote(T0 + 65 * SECOND, "A1", Side::Sell, 102)].into_iter(),
    0,
)?
.map(|book| book.map(MarketData::from));
let mut holder = Holder::Buffer(Buffer::new().with_media_type(Url::from_str("file:///books.arrows")?.media_type()));
let options = holder.record_options()?;
holder.overwrite_arrow_reader(MarketData::arrow_reader(books, None, None)?, &options)?;

let service = Arc::new(BookService::new(BookServiceOptions::new()).with_table("books", holder));
// The readings, without HTTP: one filtered read of the table each.
let query = BookQuery {
    table: "books".into(),
    ticker: "ACME".into(),
    from: T0,
    to: T0 + 120 * SECOND,
    timezone: Timezone::UTC,
    interval: None, // a minute in `timezone`
    side: None,
};
let candles = service.candles(&query)?;
assert_eq!(candles.len(), 2);
assert_eq!(candles[0].bid.map(|bid| bid.close), Some(Decimal::from_int(100)));
assert_eq!(candles[1].ask.map(|ask| ask.open), Some(Decimal::from_int(102)));
let book = service.book("books", "ACME", T0 + 90 * SECOND)?.expect("the last book at or before 10:01:30");
assert_eq!(book.get_currunix(), T0 + 65 * SECOND);
// Stored as deltas, answered whole: rebuilt over the books before it.
assert!(book.is_complete());
assert_eq!(book.alive().count(), 2);
assert_eq!(service.tickers("books")?.sequence_rows().map(|listed| listed.len()), Some(1));

// The routes, on the crate's server: `{prefix}/api/...`, JSON, `Cache-Control: no-store`.
let server = Server::bind("127.0.0.1:0")?;
let endpoint = Arc::clone(&service).route(&server, "/")?;
let answer = Request::get(&format!("{endpoint}api/tickers?table=books"))?.send()?;
assert_eq!(answer.status(), Status::OK);
assert!(answer.text()?.contains("\"key\":\"ACME\""));
// The zones `tz` reads - UTC, then every zone this build has rules for - to offer a caller.
let zones = Request::get(&format!("{endpoint}api/timezones"))?.send()?.scalar()?;
let zones = zones.sequence_rows().expect("a list");
assert_eq!(zones[0].as_str(), Some("UTC"));
let refused = Request::get(&format!("{endpoint}api/candles?table=books&ticker=NONE&from=2026-01-05T10:00:00Z&to=2026-01-05T11:00:00Z"))?.send()?;
assert_eq!(refused.status(), Status::NOT_FOUND);
let error = refused.scalar()?;
assert_eq!(error.as_struct().and_then(|body| body["error"].as_str()), Some("expected a ticker at \"books/NONE\", got nothing"));
```

## Gotchas in Rust

- Setters never finalize: a leaf with stale derived facts is refused when
  written to Arrow. Call `finalize()` after the last `set_*`.
- `get_crosscode` answers the stored code `{kind}:{side}:{base}`: `10:1:O-1001`
  for a buy order, `10:0:O-1001` for `Side::Unknown`, and side `0` for every
  element `marketdatakind().is_sided()` answers `false` for - a quote, a trade,
  a book, a snapshot control - whatever side it states. `stored_crosscode(code)` says
  what a code is stored as, idempotent and converging whichever of the code,
  the kind and the side is stated last; a lifecycle view and a lookup name the
  stored spelling.
- `EventIterator::new(items, false)` collects to sort; pass `true` only for a
  stream you know is sorted, so it streams - an unsorted stream under `true` is
  not refused, it yields broken chains (a step before its live element
  yielded as it came, `prevuuid` null).
- `BookIterator::new(items, snapshot_millis)` takes an iterator
  (`.into_iter()`) of `MarketData` or `Result<MarketData>` and yields
  `Result<BookEvent>`; `with_filter(filter)` binds an expression over the
  `marketdata` row once, refusing a column the row does not carry. An `Err`
  item is a source's own failure or a value no book folds (an undated order, a
  `BookEvent`); every input `MarketDataKind::is_booked` refuses - an
  execution, a trade, a batch - is pruned in silence. An operation dated before
  its book and a group the book refuses are left out with a `log` warning, and
  an order or a quote resting on neither side is placed nowhere with one, yet
  still counts as the book's delta.
- A book from a walk is complete only at a snapshot tick: test
  `is_complete()` before reading `alive()`, `alive_on`, `limits`, `depth` or
  `imbalance`, and rebuild a delta book with `with_previous(&previous)` - over
  `BookEvent::keyed(unix, key)` for a code's first book, which names no
  `prevuuid`. `add_operations` on a delta book is refused at `$.alive`.
- `with_previous`/`merge_with` answer `Option`: `None` means nothing moved (its
  own predecessor, an earlier event, another element), not an error.
- `insert_securityid(id)` fills an absent key, or one holding a value of a
  lower rank (`IdType::rank`: a masked `XX0000000001` or a typo below a real
  ISIN), and takes back a `derived` identifier of its type it does not rank
  below (a CUSIP derived from the ISIN), answering whether it added - a
  derivation that outranks it stands; `insert_identifier`, `insert_partyid` and `insert_fxrate` fill
  an absent key (a target) only, a named source filling its type's base key;
  `set_securityids`, `set_identifiers` and `set_partyids` replace the whole map
  under `overwrite` and fill without it, `set_fxrates` replaces the map;
  `remove_securityid(&IdKey::base(IdType::Isin))` removes the type and takes
  every `derived` identifier back. `Identifier::new(key, value)` takes an
  `IdKey` - `IdKey::base(kind)`, `"oms:clordid".parse()?` - and refuses a value
  that states nothing and a code of another shape than its type's, so a verb
  never sees one; a code of the right shape that does not close is a value of
  a lower rank, never a refusal; `Identifier::from_key(name, value)` infers the key a bridge's own name
  spells (`OMS_ClOrdID` is `oms:clordid`).
- A follower fills a parent identifier from the type it replaces - `orderid`
  into `parentorderid` into `origorderid`, `clordid` into `origclordid` - as
  `Operation::parents_of(&kind)` lists the parents nearest first
  (`IdType::parents`, or the `FIX:parents` a FIX dictionary states);
  `Identifiers::fill_parents` and `follow_parents` are the verbs behind it.
- The traits are object-safe except the verbs that take or return `Self`
  (`with_previous`, `merge_with`, `is_after` ...): `&dyn Event` reads every fact.
