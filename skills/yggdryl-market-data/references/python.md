# yggdryl-market-data in Python

`from yggdryl import graph` - every leaf is built from named facts keyed by
column name (`...` skips one, `None` clears it) and is finalized and immutable
on construction. Decimals, codes and identities read back as `Scalar`
(`.as_py()`); the enum facts - `side`, `state`, `marketdatakind` - as members
of the `IntEnum`s `yggdryl.Side`, `yggdryl.State` and `yggdryl.MarketDataKind`;
instants are `int` nanoseconds since the epoch, UTC; `securityids`, `identifiers` and
`partyids` take a list of `yggdryl.Identifier` and read back as an
`yggdryl.Identifiers` map keyed `src:type`.

## Build an order event from named facts

The constructor takes the instant positionally and every other fact by its
column name; the identity and what the facts imply are derived on the spot.

```python
from datetime import datetime, timezone
from decimal import Decimal

from yggdryl import Identifier, MarketDataKind, Side, State, graph

EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)

def nanos(moment: datetime) -> int:
    """An aware datetime as integer nanoseconds since the epoch, UTC, with no float."""
    delta = moment - EPOCH
    return (delta.days * 86_400 + delta.seconds) * 1_000_000_000 + delta.microseconds * 1_000

T = nanos(datetime(2023, 11, 14, 22, 13, 20, tzinfo=timezone.utc))
assert T == 1_700_000_000_000_000_000

order = graph.OrderEvent(
    T,
    crosscode="O-1001",
    side=Side.BUYS,
    price=Decimal("189.50"),
    quantity=100,
    currency="USD",
    ticker="AAPL",
    # An identifier is a source, a type and a value, unique by `src:type`; a code is held to its type's shape.
    securityids=[Identifier("isin", "US0378331005")],
    identifiers=[Identifier("orderid", "O-1001")],
)
# A dated identity is a UUIDv7: its millisecond leads.
assert order.curruuid.as_py().startswith("018bcfe5-6800-7")
# A cross code is stored as `{kind}:{side}:{base}`: one chain per side.
assert order.crosscode == "10:1:O-1001"
assert order.crossuuid != order.curruuid, "the cross code names a chain"
# Derived on construction: the CUSIP inside the ISIN; the ISIN itself reads as `isincode`.
assert [str(id) for id in order.securityids] == ["cusip=037833100", "derived:cusip=037833100", "isin=US0378331005"]
assert order.securityids.get("cusip") == "037833100"
assert order.isincode == "US0378331005"
assert order.lastpx is None, "a price is never a last execution"
assert order.bidpx == order.price, "a buy's price is its bid"
assert order.fxrates == {}, "nothing fills the rates"
assert order.side is Side.BUYS and order.state is State.UNKNOWN
assert order.marketdatakind is MarketDataKind.ORDR
```

## Build undated leaves, quotes and book entries

An undated leaf's identity is its content; `at` dates it. A quote is one
element holding a bid and an ask leg - the six `bid*`/`ask*` facts - stored
under side `0`; its `side` is a tag, so a quote stating a side and a price
states the leg that side takes. A market-data entry carries a `BookRef`.

```python
from decimal import Decimal

from yggdryl import MarketDataKind, Side, graph

T = 1_700_000_000_000_000_000
order = graph.Order(crosscode="O-1001", side="BUYS", price=Decimal("189.50"))
assert order.kind == "order" and order.marketdatakind is MarketDataKind.ORDR
event = order.at(T)
assert isinstance(event, graph.OrderEvent) and event.currunix == T
assert event.into_element() == order

# A two-sided quote: its bid and ask are its two legs, and it tags no side.
quote = graph.QuoteEvent(
    T,
    crosscode="Q-7",
    ticker="AAPL",
    bidpx=Decimal("189.48"),
    bidqty=300,
    bidccy="USD",
    askpx=Decimal("189.52"),
    askqty=100,
    askccy="USD",
)
assert (quote.side, quote.crosscode) == (Side.UNKN, "14:0:Q-7")
assert quote.askpx is not None and quote.askpx.as_py() == Decimal("189.52")
assert quote.marketdatakind is MarketDataKind.QUOT

# An offer: tagged `SELL`, its price is its ask leg; a quote's code stays under side 0.
offer = graph.QuoteEvent(T, crosscode="Q-8", ticker="AAPL", side="SELL", price=Decimal("189.52"), quantity=100)
assert offer.crosscode == "14:0:Q-8"
assert offer.askpx is not None and offer.askpx.as_py() == Decimal("189.52")
# Placed in a book by its control; the scope is a fact, the rest walk-time.
entry = offer.with_book(graph.BookRef(action="new", position=1, scope="L2"))
assert (entry.action, entry.book.position, entry.scope) == ("0", 1, "L2")
assert entry.curruuid != offer.curruuid, "the scope digests"
```

## Chain two events and merge two statements of one

`with_previous` states an event as the one after its predecessor; `merge_with`
folds another statement of the same event (the later recording leads, sources
union). Both answer a new value, or `None` when nothing moved.

```python
from decimal import Decimal

from yggdryl import graph

T = 1_700_000_000_000_000_000

def event(unix: int, state: str) -> graph.OrderEvent:
    return graph.OrderEvent(unix, crosscode="O-1001", state=state, side="BUYS", price=Decimal("189.50"), quantity=100)

placed = event(T, "NEW")
filled = event(T + 1_000_000_000, "PARTIALLY_FILLED").with_previous(placed)
assert filled is not None
# A later instant keeps its own place: the first there.
assert (filled.prevuuid, filled.prevunix, filled.seqnum) == (placed.curruuid, T, 0)
assert filled.crossuuid == placed.crossuuid, "one chain"
assert filled.prevpx is not None and filled.prevpx.as_py() == Decimal("189.50")
# Never itself, never one that happened after it.
assert placed.with_previous(filled) is None

# One report recorded by two hops: recording clocks and sources are not content.
LINE_1 = "018bcfe5-6800-7000-8000-000000000001"
LINE_2 = "018bcfe5-6800-7000-8000-000000000002"
gateway = graph.OrderEvent(T + 1_000_000_000, crosscode="O-1001", recdunix=T + 1_002_000_000, srcuuids=[LINE_1])
oms = graph.OrderEvent(T + 1_000_000_000, crosscode="O-1001", recdunix=T + 1_005_000_000, srcuuids=[LINE_2])
assert gateway.curruuid == oms.curruuid
merged = oms.merge_with(gateway)
assert merged is not None and merged.recdunix == T + 1_002_000_000
assert [source.as_py() for source in merged.srcuuids] == [LINE_1, LINE_2]
```

## Walk a stream into chains

`graph.EventIterator` chains a stream by cross identity (and by a live
element's `identifiers`), yields a twin as a restatement rather than a successor,
retires a chain at a terminal state and emits one `EXPIRED` at a deadline.
An order's or an execution's chain is keyed by side - a quote's is one chain
whatever side it tags - a chain lives within one `marketdatakind` (an order and
an execution under one cross code are two chains), and every walked element
leaves stating `creaunix`.

```python
from yggdryl import Side, State, graph

T = 1_700_000_000_000_000_000
SECOND = 1_000_000_000

def event(second: int, order: str, state: str, side: str = "UNKN") -> graph.OrderEvent:
    return graph.OrderEvent(T + second * SECOND, crosscode=order, state=state, side=side)

def walk(items: list[graph.OrderEvent], sorted: bool = True) -> list[graph.OrderEvent]:
    return [value.as_order_event() for value in graph.EventIterator(items, sorted=sorted)]

# Unsorted: one report logged twice, and O-1001 reopened after its fill.
arrived = [
    event(3, "O-1001", "FILLED"),
    event(0, "O-1001", "NEW"),
    event(2, "O-1001", "PARTIALLY_FILLED"),
    event(2, "O-1001", "PARTIALLY_FILLED"),
    event(4, "O-1001", "NEW"),
]
chained = walk(arrived, sorted=False)
# Each step is at an instant of its own, so each is the first there; the
# chain is in prevuuid.
assert [held.seqnum for held in chained] == [0, 0, 0, 0, 0]
assert chained[3].prevuuid == chained[1].curruuid
assert chained[1].curruuid == chained[2].curruuid, "a twin, not a successor"
assert chained[4].prevuuid is None, "the fill ended the chain"
# A chain's creation instant is its first element's, carried along it.
assert all(held.creaunix == T for held in chained[:4])

# One identifier, two sides: two chains. A report stating no side joins the
# one side alive under its code, and a NEW over a live NEW reads UPDATED.
walked = walk([event(0, "O-2002", "NEW", "BUYS"), event(1, "O-2002", "NEW", "SELL"), event(2, "O-2002", "NEW", "BUYS")])
assert (walked[1].crosscode, walked[1].seqnum) == ("10:2:O-2002", 0)
assert walked[2].prevuuid == walked[0].curruuid and walked[2].state is State.UPDATED
joined = walk([event(0, "O-3003", "NEW", "BUYS"), event(1, "O-3003", "CANCELED")])
assert joined[1].prevuuid == joined[0].curruuid
assert (joined[1].side, joined[1].crosscode) == (Side.BUYS, "10:1:O-3003")

# A 10 ms grid: a view of the living order per tick, then its deadline.
MS = 1_000_000
expiring = graph.OrderEvent(T + 50 * MS, crosscode="O-4004", exprunix=T + 70 * MS)
timed = [value.as_order_event() for value in graph.EventIterator([expiring], snapshot_ns=10 * MS)]
[view] = [held for held in timed if held.snapunix is not None and held.currunix == T + 60 * MS]
# Dated at its tick: the identity is the tick's, the content the order's,
# and its snapshot instant the one the order was stated at.
assert view.snapunix == T + 50 * MS
assert (view.currunix, view.seqnum) == (T + 60 * MS, 0)
assert view.currhashcode == expiring.currhashcode
assert (timed[-1].currunix, timed[-1].state) == (T + 70 * MS, State.EXPIRED)
```

## Build a composite trade

`graph.TradeEvent.from_parts` is the one door: a root event and its sided
executions, canonicalized so input order never changes the trade. The trade
itself is not sided: it keeps the root's cross code, a sided root's base code.

```python
from decimal import Decimal

import pytest

from yggdryl import graph

T = 1_700_000_000_000_000_000

def fill(code: str, side: str, unix: int = T) -> graph.ExecutionEvent:
    return graph.ExecutionEvent(unix, crosscode=code, side=side, lastpx=Decimal("189.50"), lastqty=100)

root = graph.OrderEvent(T, crosscode="T-1", ticker="AAPL")
trade = graph.TradeEvent.from_parts(root, [fill("E-SELL", "SELL"), fill("E-BUYS", "BUYS")])
assert [execution.crosscode for execution in trade.executions] == ["8:1:E-BUYS", "8:2:E-SELL"]
assert trade.is_execution
again = graph.TradeEvent.from_parts(root, [fill("E-BUYS", "BUYS"), fill("E-SELL", "SELL")])
assert again.curruuid == trade.curruuid
# Each execution at the trade's instant; none at all is refused.
with pytest.raises(ValueError, match=r"\$\.executions\[0\]\.currunix"):
    graph.TradeEvent.from_parts(root, [fill("E-LATE", "BUYS", T + 1)])
with pytest.raises(ValueError):
    graph.TradeEvent.from_parts(root, [])
```

## Carry any leaf as one value

`graph.MarketData` holds any leaf and answers the element and market facts it
holds; `as_<leaf>()` borrows it back, `into_leaf()` answers it as its own class.

```python
from yggdryl import MarketDataKind, Side, graph

order = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001", side="BUYS")
value = graph.MarketData(order)
assert value.kind == "order_event"
assert value.marketdatakind is MarketDataKind.ORDR
assert "trade_event" in graph.MarketData.kinds
assert value.is_event
assert value.as_order_event() == order
assert value.as_quote_event() is None, "another kind is none of this value"
assert (value.crosscode, value.side) == ("10:1:O-1001", Side.BUYS)
assert value.into_leaf() == order
```

## Write and read the marketdata row

`MarketData.arrow_reader` streams leaves into bounded `pyarrow` batches of the
lifted row; `from_arrow_reader` reads any Arrow source back, tolerant of a
subset of columns in any order. `marketdatakind` and, for a dated leaf,
`currunix` are the minimum.

```python
import pyarrow as pa

from yggdryl import MarketDataKind, graph

order = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001")
values = [graph.Order(), order, graph.BookEvent(1_700_000_001_000_000_000, "AAPL")]

# 62 columns: 6 element, 9 event, 34 market (marketdatakind first), 5 operation,
# the book controls bookscope, bookaction and bookposition, 5 nested.
field = graph.MarketData.field()
assert len(list(field)) == 62
assert [child.name for child in field][15] == "marketdatakind"
reader = graph.MarketData.arrow_reader(values, batch_row_size=1_000)
assert isinstance(reader, pa.RecordBatchReader)
table = reader.read_all()
# The column stores each member's code.
assert table.column("marketdatakind").to_pylist() == [int(MarketDataKind.ORDR), int(MarketDataKind.ORDR), int(MarketDataKind.BOOK)]
assert table.schema.field("currunix").type == pa.timestamp("ns", "UTC")
assert list(graph.MarketData.from_arrow_reader(table)) == [graph.MarketData(value) for value in values]

# A foreign table: three columns, one the row does not name.
foreign = pa.table(
    {
        "marketdatakind": pa.array([int(MarketDataKind.ORDR)], pa.int32()),
        "currunix": pa.array([1_700_000_000_000_000_000], pa.int64()),
        "crosscode": ["10:0:O-1001"],
        "msgtype": ["D"],
    }
)
[lifted] = graph.MarketData.from_arrow_reader(foreign)
event = lifted.as_order_event()
assert event is not None and (event.crosscode, event.currunix) == ("10:0:O-1001", 1_700_000_000_000_000_000)
```

## Persist a marketdata stream and read it back

Any record medium stores the batches as they stream - here Parquet by suffix -
and any Arrow reader (pyarrow, polars) reads the file; reading leaves back is
the same `from_arrow_reader`.

```python
import pathlib
import tempfile

import pyarrow.parquet as pq

from yggdryl import IOBase, graph

values = [graph.OrderEvent(1_700_000_000_000_000_000 + at, crosscode=f"O-{at}") for at in range(3)]

with tempfile.TemporaryDirectory() as directory:
    path = pathlib.Path(directory) / "marketdata.parquet"
    IOBase(path).overwrite_arrow_reader(graph.MarketData.arrow_reader(values))
    assert pq.read_table(path).column("crosscode").to_pylist() == ["10:0:O-0", "10:0:O-1", "10:0:O-2"]
    read = list(graph.MarketData.from_arrow_reader(IOBase(path).read_arrow_reader()))
    assert [value.into_leaf() for value in read] == values
```

## Fold a sorted stream into books

`graph.BookIterator(items, snapshot_millis=0, filter=None)` folds sorted
orders and quotes into one `BookEvent` per instant and book key that moved it -
the instrument's ISIN, else its ticker, else `XX0000000000` - pruning every
execution and trade. A book is complete (`is_complete`) only at a snapshot
tick; every other book states its deltas alone beside the top of book they
settled on, and `with_previous` over the complete book before it rebuilds it
whole. `filter` - a predicate over the `marketdata` row - narrows what folds.

```python
from decimal import Decimal

from yggdryl import Identifier, Side, graph

T = 1_700_000_000_000_000_000
SECOND = 1_000_000_000

def bid(unix: int, code: str, price: str, quantity: int) -> graph.OrderEvent:
    return graph.OrderEvent(unix, crosscode=code, ticker="AAPL", side="BUYS", price=Decimal(price), quantity=quantity, state="NEW")

fill = graph.ExecutionEvent(T + SECOND, crosscode="E-1", ticker="AAPL", side="BUYS", lastpx=Decimal("189.52"), lastqty=100)
stream = [bid(T, "B-1", "189.48", 300), bid(T + SECOND, "B-2", "189.49", 200), fill]

books = list(graph.BookIterator(stream))
assert len(books) == 2, "one book per instant that moved it; the execution is a delta of its own"
# No grid and no snapshot input: each book states its deltas alone and its top of book.
last = books[1]
assert not last.is_complete
assert (last.currunix, len(last.deltas), last.alive) == (T + SECOND, 2, [])
best = last.best_price(Side.BUYS)
assert best is not None and best.as_py() == Decimal("189.49")
# Rebuilt whole: the first over the empty book its key starts from, the next over it.
assert books[0].prevuuid is None
first = books[0].with_previous(graph.BookEvent.keyed(T, "AAPL"))
assert first is not None
whole = last.with_previous(first)
assert whole is not None and whole.is_complete
assert (len(whole.alive), whole.curruuid) == (2, last.curruuid), "depth persists"

# A 500 ms grid states the whole living book at each crossed tick.
gridded = list(graph.BookIterator(stream, snapshot_millis=500))
assert len(gridded) == 3 and all(book.is_complete for book in gridded)
# A filter narrows what folds, and never admits an execution.
assert list(graph.BookIterator(stream, filter="marketdatakind = 'EXEC'")) == []
# The book key: the instrument's ISIN, else the ticker, else `XX0000000000`.
[keyless] = graph.BookIterator([graph.OrderEvent(T, crosscode="L-1", side="SELL")])
assert (keyless.crosscode, keyless.ticker) == ("3:0:XX0000000000", None)
listed = graph.OrderEvent(T, crosscode="L-2", side="SELL", ticker="AAPL", securityids=[Identifier("isin", "US0378331005")])
[book] = graph.BookIterator([listed])
assert (book.crosscode, book.isincode, book.ticker) == ("3:0:US0378331005", "US0378331005", "AAPL")
# Out of order is no error: the operation dated before its book is left out,
# with a warning.
assert len(list(graph.BookIterator(list(reversed(stream))))) == 1
```

## Read a book

A complete book answers each side as its `limits` (one per price, best
first, the unpriced market level last) and its entries as `alive_on(side)`;
every book answers the readings of the first level that can trade:
`best_price`, `best_quantity`, the `bidpx`/`askpx` it states, `spread`; a
complete one `depth` and `imbalance` too. A book built by hand is complete. A
side is a `Side` member, its code or any spelling `Side` reads.

```python
from decimal import Decimal

from yggdryl import Scalar, Side, graph

T = 1_700_000_000_000_000_000

def entry(code: str, side: str, price: str | None, quantity: int, tradable: bool | None = None) -> graph.OrderEvent:
    return graph.OrderEvent(
        T,
        crosscode=code,
        ticker="AAPL",
        side=side,
        price=None if price is None else Decimal(price),
        quantity=quantity,
        tradable=tradable,
    )

book = graph.BookEvent(T, "AAPL").with_operations(
    [
        entry("B-0", "BUYS", "189.49", 10, tradable=False),
        entry("B-1", "BUYS", "189.48", 300),
        entry("B-2", "BUYS", "189.47", 500),
        entry("A-1", "SELL", "189.52", 100),
        entry("MKT", "BUYS", None, 50),
    ]
)

def value(scalar: Scalar | None) -> object:
    assert scalar is not None
    return scalar.as_py()

# The level at 189.49 cannot trade: the best bid is the first that can.
limits = [limit.as_py() for limit in book.limits(Side.BUYS)]
assert [limit["price"] for limit in limits] == [Decimal("189.49"), Decimal("189.48"), Decimal("189.47"), None]
assert (limits[0]["tradable"], limits[1]["tradable"]) == (False, True)
assert value(book.best_price(Side.BUYS)) == Decimal("189.48")
assert (value(book.bidpx), value(book.bidqty)) == (Decimal("189.48"), 300)
assert value(book.askpx) == Decimal("189.52")
assert value(book.spread) == Decimal("0.04")
assert value(book.bbo_midpoint) == Decimal("189.50")
assert book.price == book.bbo_midpoint
assert not book.is_locked and not book.is_crossed
assert value(book.depth("BUYS", 2)) == 310
assert book.is_complete
assert (len(book.alive), len(book.alive_on(Side.BUYS)), len(book.alive_on("SELL"))) == (5, 4, 1)
# The deltas are the five orders, in the order applied.
assert [delta.crosscode for delta in book.deltas] == ["10:1:B-0", "10:1:B-1", "10:1:B-2", "10:2:A-1", "10:1:MKT"]
```

## Replace a scope with a snapshot

A `SnapshotEvent` clears its `(book, scope)` partition on both sides - an
empty FIX `W` is one.

```python
from decimal import Decimal

from yggdryl import graph

T = 1_700_000_000_000_000_000

def order(code: str, side: str, price: str) -> graph.OrderEvent:
    return graph.OrderEvent(T, crosscode=code, ticker="AAPL", side=side, price=Decimal(price), quantity=100)

book = graph.BookEvent(T, "AAPL").with_operations([order("B-1", "BUYS", "189.48"), order("A-1", "SELL", "189.52")])
assert len(book.alive) == 2

control = graph.SnapshotEvent.snapshot(graph.OrderEvent(T + 1_000_000_000, ticker="AAPL"))
assert control.book.action == "snapshot"
after = book.with_operations([control])
assert after.alive == []
assert (after.price, after.bidpx, after.askpx) == (None, None, None)
```

## Read the stream through named views

A view is one `Plan` over the `marketdata` row, applied by the expression
engine and bound once per reader; lifts turn nested facts into columns.

```python
from yggdryl import Identifier, Plan, enums, graph

T = 1_700_000_000_000_000_000
order = graph.OrderEvent(
    T, crosscode="O-1001", side="BUYS", identifiers=[Identifier("clordid", "C-1")]
)
root = graph.OrderEvent(T + 1_000_000_000, crosscode="T-1")
trade = graph.TradeEvent.from_parts(
    root,
    [
        graph.ExecutionEvent(T + 1_000_000_000, crosscode="E-1", side="BUYS"),
        graph.ExecutionEvent(T + 1_000_000_000, crosscode="E-2", side="SELL"),
    ],
)

def stream():
    return graph.MarketData.arrow_reader([order, trade])

assert set(enums.MARKET_VIEWS) == {"orders", "quotes", "executions", "trades", "books", "lifecycle"}
# A lift reaches one identifier of the map by its key.
orders = graph.MarketData.apply_view("orders", stream(), ["identifiers['clordid'] as clordid"]).read_all()
assert orders.schema.names[-1] == "clordid"
assert orders.column("clordid").to_pylist() == ["C-1"]

# One row per execution, beside the trade's own columns.
trades = graph.MarketData.apply_view("trades", stream()).read_all()
assert sorted(trades.column("execution.crosscode").to_pylist()) == ["8:1:E-1", "8:2:E-2"]

# A view is a plan whose text reads back as the same plan.
plan = graph.MarketData.plan("trades")
assert Plan(str(plan)) == plan
# A lifecycle follows the cross code as stored: side included.
chain = graph.MarketData.apply_view("lifecycle", stream(), crosscode="10:1:O-1001").read_all()
assert chain.column("crosscode").to_pylist() == ["10:1:O-1001"]
```

## Turn a FIX capture into books

A FIX capture reaches the graph through the codec: `lifecycle` settles each
message, `book_arrow_reader` folds sorted messages into book rows - orders,
quotes and `W`/`X` entries, a trade entry pruned - and
`MarketData.from_arrow_reader` reads the books back. A `W` full refresh is a
snapshot input, so its book is complete; the `X` after it states its delta.

```python
from decimal import Decimal
from pathlib import Path

from yggdryl import MarketDataKind, Side, graph
from yggdryl.fix import FixCodec, FixRegistry

# `config/fix` of a yggdryl checkout (see the yggdryl-fix skill).
codec = FixCodec(FixRegistry.from_handle(Path("config/fix")))
lines = [
    b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|",
    b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|",
]
capture = list(codec.parse_lines(lines))

rows = codec.book_arrow_reader(codec.lifecycle(capture), snapshot_millis=0, filter=None)
values = list(graph.MarketData.from_arrow_reader(rows))
assert [value.marketdatakind for value in values] == [MarketDataKind.BOOK, MarketDataKind.BOOK]
first, last = values[0].as_book_event(), values[1].as_book_event()
assert first is not None and first.is_complete
assert last is not None
best = last.best_price(Side.BUYS)
assert best is not None and best.as_py() == Decimal(101)
# The bid's change and the trade entry (`269=2`) are its two deltas.
assert not last.is_complete and len(last.deltas) == 2
```

## Fold books into candles

`graph.candles(books, interval, timezone=None)` folds books sorted by their
instant into one `Candle` per book cross code and bucket - the best bid, the
best ask, the mid and the spread each a reading of `open`, `high`, `low`,
`close`, the touch when the bucket closed and how many books folded - and
`graph.CandleIterator(books, options)` is the same walk, lazy. It reads a
book's top of book alone, so delta books fold as complete ones do. The
interval is a spelling or a count of nanoseconds; the zone is what the
buckets' wall clock aligns to.

```python
from decimal import Decimal

import pytest

from yggdryl import Serie, graph

# 2023-11-14T22:13:20Z.
T = 1_700_000_000_000_000_000
SECOND = 1_000_000_000

def quote(unix: int, code: str, side: str, price: str, quantity: int) -> graph.QuoteEvent:
    return graph.QuoteEvent(unix, crosscode=code, ticker="AAPL", side=side, price=Decimal(price), quantity=quantity, state="NEW")

stream = [
    quote(T, "B1", "BUYS", "189.48", 300),
    quote(T, "A1", "SELL", "189.52", 100),
    quote(T + 20 * SECOND, "B2", "BUYS", "189.50", 200),
    quote(T + 70 * SECOND, "A2", "SELL", "189.51", 50),
]
# Three books: the two quotes at T share one instant.
books = list(graph.BookIterator(stream))

def reading(value: dict | None) -> tuple | None:
    return None if value is None else tuple(value[cell].as_py() for cell in ("open", "high", "low", "close"))

# Minute candles in UTC: the 22:13 and 22:14 buckets.
first, second = graph.candles(books, "1m")
assert (first.crosscode, first.start, first.end) == ("3:0:AAPL", 1_699_999_980 * SECOND, 1_700_000_040 * SECOND)
assert reading(first.bid) == (Decimal("189.48"), Decimal("189.50"), Decimal("189.48"), Decimal("189.50"))
assert reading(first.spread) == (Decimal("0.04"), Decimal("0.04"), Decimal("0.02"), Decimal("0.02"))
assert first.bidqty is not None and first.bidqty.as_py() == 200
assert first.books == 2
# A2 undercuts A1: the second bucket's ask opens at 189.51 and the spread narrows.
assert reading(second.ask) is not None and reading(second.ask)[0] == Decimal("189.51")
assert reading(second.mid) is not None and reading(second.mid)[3] == Decimal("189.505")
assert list(graph.CandleIterator(books, graph.CandleOptions("1m"))) == [first, second]

# Buckets align to the zone's wall clock: 22:13:20Z is 23:13:20 in Zurich,
# so its daily candle opens at Zurich midnight, 23:00Z the day before.
[daily] = graph.candles(books, "1d", "Europe/Zurich")
assert (daily.start, daily.books) == (1_699_916_400 * SECOND, 3)
[utc] = graph.candles(books, graph.CandleOptions("1d"))
assert utc.start == 1_699_920_000 * SECOND

# Candles cross as rows of Candle.field(), and read back as the same values.
field = graph.Candle.field()
assert len(list(field)) == 23
rows = Serie.from_scalars(field, [candle.into_scalar() for candle in (first, second)])
assert graph.Candle.from_scalar(rows.scalar(1)) == second
assert second.as_py()["askopen"] == Decimal("189.51")
# Books must arrive sorted; a bare quote is not a book.
with pytest.raises(ValueError, match=r"\$\.book\.currunix"):
    graph.candles(list(reversed(books)), "1m")
with pytest.raises(TypeError, match=r"expected book_event, got quote_event"):
    graph.candles([stream[0]], "1m")
```

## Gotchas in Python

- Seconds or milliseconds where nanoseconds are expected land in 1970; build
  instants with integer arithmetic, never `datetime.timestamp() * 1e9` (a float
  loses the last digits).
- `crosscode` answers the stored code `{kind}:{side}:{base}`: `"10:1:O-1001"`
  for a buy order (kind 10, side 1), `"10:0:O-1001"` for `Side.UNKN`, and a
  quote, a trade, a book or a snapshot control carries side `0` whatever side
  it states. The lifecycle view's `crosscode=` names the stored one.
- `side`, `state` and `marketdatakind` are `IntEnum` members: compare with
  `is Side.BUYS`, never `== "BUYS"`; a column stores `int(member)`. A side is
  never `None` - `Side.UNKN` is unstated.
- `graph.BookIterator(items, snapshot_millis=0, filter=None)` - `filter` a
  `Filter`, a `Term`, an `Expression` or a predicate's text over the
  `marketdata` row, refused where it names a column the row does not carry;
  `EventIterator(items, sorted=True, snapshot_ns=None)` defaults to trusting
  the order - an unsorted list is not refused, it yields broken chains (a step
  before its live element yielded as it came, `prevuuid` None); pass
  `sorted=False` for one you have not sorted.
- Prices and quantities take `Decimal("189.5")` (or an `int`): a float
  `price=189.5` is refused at `$.price` (`got f64`). `fxrates` takes
  `{"EUR": Decimal("1.1")}` and reads back as `dict[str, Scalar]`: each rate
  is a decimal `Scalar`, as `price` and `bidpx` are, so `.as_py()` is the
  `Decimal` - unlike `securityids`, `identifiers` and `partyids`, `Identifiers`
  maps keyed `src:type` of `Identifier`s whose `src`, `type`, `key` and `value`
  are `str`
  ([FX rates](https://platob.github.io/yggdryl/graph/market/#fx-rates)).
- Every verb answers a new value: `book.with_operations([...])` does not change
  `book`; only `with_previous` / `merge_with` answer `None` when nothing moved.
- A book refuses an undated `Order`: `BookIterator` at `$.operation.kind`,
  `with_operations` at `$.operations[i].kind`; an execution or a trade is
  pruned, no error and no book. What `BookIterator` finds wrong in the data -
  an operation dated before its book - it leaves out, and an order or a quote
  stating neither side it places nowhere (still the book's delta), each with a
  `logging` warning under `yggdryl.graph.book`, and no error.
- A book from a walk is complete only at a snapshot tick: test
  `book.is_complete` before reading `alive`, `alive_on`, `limits`, `depth` or
  `imbalance`, and rebuild a delta book with `book.with_previous(previous)` -
  over `graph.BookEvent.keyed(unix, key)` for a code's first book, whose
  `prevuuid` is `None`. `with_operations` on a delta book is refused at
  `$.alive`.
- A leaf compares equal to its own class only: compare `value.into_leaf()` or
  `value.as_order_event()` with a leaf, `MarketData` with `MarketData`.
- `book.limits(side)` answers struct `Scalar`s: `limit.as_py()` is a dict of
  `price`, `quantity`, `uuids`, `tradable`. `is_complete`, `alive`,
  `deltas`, `spread`, `is_crossed` and `is_locked` are properties;
  `alive_on`, `limits`, `best_price`, `best_quantity`, `depth` and
  `imbalance` take arguments.
- Identifier verbs (`insert_securityid`, `insert_identifier`, `insert_partyid`, ...)
  are Rust-only: state `securityids=`, `identifiers=` and `partyids=` - a list of
  `Identifier` or an `Identifiers` - when you build. A dict is no identifier
  map: build each `Identifier(src, type, value)`, or `Identifier.from_key(key,
  value)` from a full `src:type` key.
