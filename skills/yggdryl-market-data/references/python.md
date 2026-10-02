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
    # An identifier is a source, a type and a value, unique by `src:type`; a code is checked by its type.
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

An undated leaf's identity is its content; `at` dates it. A quote states its
own side and price, or the bid and ask it quotes as the six `bid*`/`ask*`
facts; a market-data entry carries a `BookRef`.

```python
from decimal import Decimal

from yggdryl import MarketDataKind, Side, graph

T = 1_700_000_000_000_000_000
order = graph.Order(crosscode="O-1001", side="BUYS", price=Decimal("189.50"))
assert order.kind == "order" and order.marketdatakind is MarketDataKind.ORDR
event = order.at(T)
assert isinstance(event, graph.OrderEvent) and event.currunix == T
assert event.into_element() == order

# A two-sided quote: its bid and ask are facts, and it takes no side.
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

# A sided offer, placed in a book by its control; the scope is a fact, the rest walk-time.
offer = graph.QuoteEvent(T, crosscode="Q-8", ticker="AAPL", side="SELL", price=Decimal("189.52"), quantity=100)
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
An order's, a quote's or an execution's chain is keyed by side, a chain lives
within one `marketdatakind` (an order and an execution under one cross code are
two chains), and every walked element leaves stating `creaunix`.

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

# 60 columns: 6 element, 9 event, 34 market (marketdatakind first), 5 operation, bookscope, 5 nested.
field = graph.MarketData.field()
assert len(list(field)) == 60
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

`graph.BookIterator` folds sorted operations into one `BookEvent` per touched
instant and book: an input's ticker, else its category `MIC:CFI`. Depth
persists; deltas and executions are each book's own.

```python
from decimal import Decimal

from yggdryl import graph

T = 1_700_000_000_000_000_000
SECOND = 1_000_000_000

def bid(unix: int, code: str, price: str, quantity: int) -> graph.OrderEvent:
    return graph.OrderEvent(unix, crosscode=code, ticker="AAPL", side="BUYS", price=Decimal(price), quantity=quantity)

fill = graph.ExecutionEvent(T + SECOND, crosscode="E-1", ticker="AAPL", side="BUYS", lastpx=Decimal("189.52"), lastqty=100)
stream = [bid(T, "B-1", "189.48", 300), bid(T + SECOND, "B-2", "189.49", 200), fill]

books = list(graph.BookIterator(stream))
assert len(books) == 2, "one book per touched instant"
last = books[1]
assert (last.currunix, len(last.alive)) == (T + SECOND, 2), "depth persists"
assert len(last.deltas) == 1
assert [execution.crosscode for execution in last.executions] == ["8:1:E-1"]

# A 500 ms grid adds the living book at each crossed tick.
assert len(list(graph.BookIterator(stream, snapshot_millis=500))) == 3
# No ticker: the book is the category, `XXXX` or `XXXXXX` for what is unstated.
[book] = graph.BookIterator([graph.OrderEvent(T, crosscode="L-1", side="SELL", miccode="XNAS")])
assert (book.crosscode, book.ticker) == ("3:0:XNAS:XXXXXX", None)
# Out of order is no error: the operation dated before its book is left out,
# with a warning.
assert len(list(graph.BookIterator(list(reversed(stream))))) == 1
```

## Read a book

A book answers each side as its `limits` (one per price, best first, the
unpriced market level last) and the readings of the first level that can
trade: `best_price`, `best_quantity`, the `bidpx`/`askpx` it states,
`spread`, `depth`, `imbalance`. A side is a `Side` member, its code or any
spelling `Side` reads.

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
assert len(book.alive) == 5
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
message, `book_arrow_reader` folds sorted messages into book rows, and
`MarketData.from_arrow_reader` reads the books back.

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

rows = codec.book_arrow_reader(codec.lifecycle(capture), snapshot_millis=0)
values = list(graph.MarketData.from_arrow_reader(rows))
assert [value.marketdatakind for value in values] == [MarketDataKind.BOOK, MarketDataKind.BOOK]
last = values[1].as_book_event()
assert last is not None
best = last.best_price(Side.BUYS)
assert best is not None and best.as_py() == Decimal(101)
assert len(last.executions) == 1
```

## Fold books into candles

`graph.candles(books, interval, timezone=None)` folds books sorted by their
instant into one `Candle` per cross code and bucket - the best bid, the best
ask, the mid and the spread each a reading of `open`, `high`, `low`, `close` -
and `graph.CandleIterator(books, options)` is the same walk, lazy. The
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
assert (first.books, first.executions, first.volume.as_py()) == (2, 0, 0)
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
assert len(list(field)) == 25
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
  trade, a book or a snapshot control carries side `0` whatever side it
  states. The lifecycle view's `crosscode=` names the stored one.
- `side`, `state` and `marketdatakind` are `IntEnum` members: compare with
  `is Side.BUYS`, never `== "BUYS"`; a column stores `int(member)`. A side is
  never `None` - `Side.UNKN` is unstated.
- `graph.BookIterator(items, snapshot_millis=0)`;
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
  `with_operations` at `$.operations[i].kind`. What `BookIterator` finds wrong
  in the data - an operation dated before its book, an order or a quote
  stating neither side - it leaves out, with a `logging` warning under
  `yggdryl.graph.book`, and no error.
- A leaf compares equal to its own class only: compare `value.into_leaf()` or
  `value.as_order_event()` with a leaf, `MarketData` with `MarketData`.
- `book.limits(side)` answers struct `Scalar`s: `limit.as_py()` is a dict of
  `price`, `quantity`, `uuids`, `tradable`. `alive`, `deltas`, `executions`,
  `spread`, `is_crossed` and `is_locked` are properties; `limits`,
  `best_price`, `best_quantity`, `depth` and `imbalance` take arguments.
- Identifier verbs (`insert_securityid`, `insert_identifier`, `insert_partyid`, ...)
  are Rust-only: state `securityids=`, `identifiers=` and `partyids=` - a list of
  `Identifier` or an `Identifiers` - when you build. A dict is no identifier
  map: build each `Identifier(src, type, value)`, or `Identifier.from_key(key,
  value)` from a full `src:type` key.
