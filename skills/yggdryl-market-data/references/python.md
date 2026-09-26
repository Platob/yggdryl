# yggdryl-market-data in Python

`from yggdryl import graph` - every leaf is built from named facts keyed by
column name (`...` skips one, `None` clears it) and is finalized and immutable
on construction. Decimals, codes and identities read back as `Scalar`
(`.as_py()`); instants are `int` nanoseconds since the epoch, UTC.

## Build an order event from named facts

The constructor takes the instant positionally and every other fact by its
column name; the identity and what the facts imply are derived on the spot.

```python
from datetime import datetime, timezone
from decimal import Decimal

from yggdryl import graph

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
    side="BUY",
    price=Decimal("189.50"),
    quantity=100,
    currency="USD",
    ticker="AAPL",
    securityids={"ISIN": "US0378331005"},
    altids={"ORDERID": "O-1001"},
)
# A dated identity is a UUIDv7: its millisecond leads.
assert order.curruuid.as_py().startswith("018bcfe5-6800-7")
assert order.crossuuid != order.curruuid, "the cross code names a chain"
# Derived on construction: the CUSIP inside the ISIN, and the bid lane of a priced buy.
assert order.securityids == {"CUSIP": "037833100", "ISIN": "US0378331005"}
assert order.bid is not None and order.bid.price == order.price
assert order.lastpx is None, "a price is never a last execution"
assert order.state.as_py() == "00UNKNOWN"
```

## Build undated leaves, quotes and book entries

An undated leaf's identity is its content; `at` dates it. A quote names its side
through the one lane it states, and a market-data entry carries a `BookRef`.

```python
from decimal import Decimal

from yggdryl import graph

order = graph.Order(crosscode="O-1001", side="BUY", price=Decimal("189.50"))
assert order.kind == "order"
event = order.at(1_700_000_000_000_000_000)
assert isinstance(event, graph.OrderEvent) and event.currunix == 1_700_000_000_000_000_000
assert event.into_element() == order

# One lane, no side of its own: the quote takes the lane's side.
offer = graph.QuoteEvent(
    1_700_000_000_000_000_000,
    ticker="AAPL",
    ask=graph.Lane(price=Decimal("189.52"), quantity=100, currency="USD"),
)
assert offer.side.as_py() == "SELL"
assert offer.price is not None and offer.price.as_py() == Decimal("189.52")

# The book control digests into the entry.
entry = offer.with_book(graph.BookRef(action="new", position=1))
assert (entry.action, entry.book.position) == ("0", 1)
assert entry.curruuid != offer.curruuid
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
    return graph.OrderEvent(unix, crosscode="O-1001", state=state, side="BUY", price=Decimal("189.50"), quantity=100)

placed = event(T, "NEW")
filled = event(T + 1_000_000_000, "PARTIALLY_FILLED").with_previous(placed)
assert filled is not None
assert (filled.prevuuid, filled.prevunix, filled.seqnum) == (placed.curruuid, T, 1)
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
element's `altids`), yields a twin as a restatement rather than a successor,
retires a chain at a terminal state and emits one `95EXPIRED` at a deadline.

```python
from yggdryl import graph

T = 1_700_000_000_000_000_000
SECOND = 1_000_000_000

def event(second: int, order: str, state: str) -> graph.OrderEvent:
    return graph.OrderEvent(T + second * SECOND, crosscode=order, state=state)

# Unsorted: one report logged twice, and O-1001 reopened after its fill.
arrived = [
    event(3, "O-1001", "FILLED"),
    event(0, "O-1001", "NEW"),
    event(2, "O-1001", "PARTIALLY_FILLED"),
    event(2, "O-1001", "PARTIALLY_FILLED"),
    event(4, "O-1001", "NEW"),
]
chained = [value.as_order_event() for value in graph.EventIterator(arrived, sorted=False)]
assert [held.seqnum for held in chained] == [0, 1, 1, 2, 0]
assert chained[1].curruuid == chained[2].curruuid, "a twin, not a successor"
assert chained[4].prevuuid is None, "the fill ended the chain"

# A 10 ms grid: a view of the living order per tick, then its deadline.
MS = 1_000_000
expiring = graph.OrderEvent(T + 50 * MS, crosscode="O-3003", exprtime=T + 70 * MS)
timed = [value.as_order_event() for value in graph.EventIterator([expiring], snapshot_ns=10 * MS)]
assert any(held.snapunix == T + 60 * MS for held in timed)
assert (timed[-1].currunix, timed[-1].state.as_py()) == (T + 70 * MS, "95EXPIRED")
```

## Build a composite trade

`graph.TradeEvent.from_parts` is the one door: a root event and its sided
executions, canonicalized so input order never changes the trade.

```python
from decimal import Decimal

import pytest

from yggdryl import graph

T = 1_700_000_000_000_000_000

def fill(code: str, side: str, unix: int = T) -> graph.ExecutionEvent:
    return graph.ExecutionEvent(unix, crosscode=code, side=side, lastpx=Decimal("189.50"), lastqty=100)

root = graph.OrderEvent(T, crosscode="T-1", ticker="AAPL")
trade = graph.TradeEvent.from_parts(root, [fill("E-SELL", "SELL"), fill("E-BUY", "BUY")])
assert [execution.crosscode for execution in trade.executions] == ["E-BUY", "E-SELL"]
assert trade.is_execution
again = graph.TradeEvent.from_parts(root, [fill("E-BUY", "BUY"), fill("E-SELL", "SELL")])
assert again.curruuid == trade.curruuid
# Each execution at the trade's instant; none at all is refused.
with pytest.raises(ValueError, match=r"\$\.executions\[0\]\.currunix"):
    graph.TradeEvent.from_parts(root, [fill("E-LATE", "BUY", T + 1)])
with pytest.raises(ValueError):
    graph.TradeEvent.from_parts(root, [])
```

## Carry any leaf as one value

`graph.MarketData` holds any leaf and answers the element and market facts it
holds; `as_<leaf>()` borrows it back, `into_leaf()` answers it as its own class.

```python
from yggdryl import graph

order = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001", side="BUY")
value = graph.MarketData(order)
assert value.kind == "order_event"
assert "trade_event" in graph.MarketData.kinds
assert value.is_event
assert value.as_order_event() == order
assert value.as_quote_event() is None, "another kind is none of this value"
assert (value.crosscode, value.side.as_py()) == ("O-1001", "BUY")
assert value.into_leaf() == order
```

## Write and read the marketdata row

`MarketData.arrow_reader` streams leaves into bounded `pyarrow` batches of the
lifted row; `from_arrow_reader` reads any Arrow source back, tolerant of a
subset of columns in any order.

```python
import pyarrow as pa

from yggdryl import graph

order = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001")
values = [graph.Order(), order, graph.BookEvent(1_700_000_001_000_000_000, "AAPL")]

# 59 columns: kind, 16 event, 19 market, 8 operation, 5 book control, 3 book facts, 7 nested.
assert len(list(graph.MarketData.field())) == 59
reader = graph.MarketData.arrow_reader(values, batch_row_size=1_000)
assert isinstance(reader, pa.RecordBatchReader)
table = reader.read_all()
assert table.column("kind").to_pylist() == ["order", "order_event", "book_event"]
assert table.schema.field("currunix").type == pa.timestamp("ns", "UTC")
assert list(graph.MarketData.from_arrow_reader(table)) == [graph.MarketData(value) for value in values]

# A foreign table: three columns, one the row does not name.
foreign = pa.table(
    {
        "kind": ["order_event"],
        "currunix": pa.array([1_700_000_000_000_000_000], pa.int64()),
        "crosscode": ["O-1001"],
        "msgtype": ["D"],
    }
)
[lifted] = graph.MarketData.from_arrow_reader(foreign)
event = lifted.as_order_event()
assert event is not None and (event.crosscode, event.currunix) == ("O-1001", 1_700_000_000_000_000_000)
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
    assert pq.read_table(path).column("crosscode").to_pylist() == ["O-0", "O-1", "O-2"]
    read = list(graph.MarketData.from_arrow_reader(IOBase(path).read_arrow_reader()))
    assert [value.into_leaf() for value in read] == values
```

## Fold a sorted stream into books

`graph.BookIterator` folds sorted operations into one `BookEvent` per touched
instant and symbol; depth persists, deltas and executions are each book's own.

```python
from decimal import Decimal

import pytest

from yggdryl import graph

T = 1_700_000_000_000_000_000
SECOND = 1_000_000_000

def bid(unix: int, code: str, price: str, quantity: int) -> graph.OrderEvent:
    return graph.OrderEvent(unix, crosscode=code, ticker="AAPL", side="BUY", price=Decimal(price), quantity=quantity)

fill = graph.ExecutionEvent(T + SECOND, crosscode="E-1", ticker="AAPL", side="BUY", lastpx=Decimal("189.52"), lastqty=100)
stream = [bid(T, "B-1", "189.48", 300), bid(T + SECOND, "B-2", "189.49", 200), fill]

books = list(graph.BookIterator(stream))
assert len(books) == 2, "one book per touched instant"
last = books[1]
assert (last.currunix, len(last.bid)) == (T + SECOND, 2), "depth persists"
assert len(last.bid.deltas) == 1
assert [execution.crosscode for execution in last.executions] == ["E-1"]

# A 500 ms grid adds the living book at each crossed tick; global consolidates.
assert len(list(graph.BookIterator(stream, snapshot_millis=500))) == 3
assert all(book.crosscode == graph.GLOBAL_SYMBOL for book in graph.BookIterator(stream, global_=True))
# Out of order is refused.
with pytest.raises(ValueError, match="sorted operation timestamp"):
    list(graph.BookIterator(list(reversed(stream))))
```

## Read a book

A side answers its bests, its `limits` (one per price, best first, the unpriced
market level last) and `depth`; a book its spread, midpoint and imbalance.

```python
from decimal import Decimal

from yggdryl import graph

T = 1_700_000_000_000_000_000

def entry(code: str, side: str, price: str | None, quantity: int) -> graph.OrderEvent:
    return graph.OrderEvent(
        T, crosscode=code, ticker="AAPL", side=side, price=None if price is None else Decimal(price), quantity=quantity
    )

book = graph.BookEvent(T, "AAPL").with_operations(
    [
        entry("B-1", "BUY", "189.48", 300),
        entry("B-2", "BUY", "189.47", 500),
        entry("A-1", "SELL", "189.52", 100),
        entry("MKT", "BUY", None, 50),
    ]
)

def value(scalar):
    assert scalar is not None
    return scalar.as_py()

assert value(book.bid.best_price) == Decimal("189.48")
assert value(book.spread) == Decimal("0.04")
assert value(book.bbo_midpoint) == Decimal("189.50")
assert book.price == book.bbo_midpoint
assert not book.is_locked and not book.is_crossed
assert value(book.imbalance(1)) == Decimal("0.5")
assert [limit.as_py()["price"] for limit in book.bid.limits] == [Decimal("189.48"), Decimal("189.47"), None]
assert value(book.bid.depth(2)) == 800
assert value(book.bid.depth(3)) == 850
```

## Replace a scope with a snapshot

A `SnapshotEvent` clears its `(symbol, scope)` partition on both sides - an empty
FIX `W` is one - and the book records what it replaced.

```python
from decimal import Decimal

from yggdryl import graph

T = 1_700_000_000_000_000_000

def order(code: str, side: str, price: str) -> graph.OrderEvent:
    return graph.OrderEvent(T, crosscode=code, ticker="AAPL", side=side, price=Decimal(price), quantity=100)

book = graph.BookEvent(T, "AAPL").with_operations([order("B-1", "BUY", "189.48"), order("A-1", "SELL", "189.52")])
assert (len(book.bid), len(book.ask)) == (1, 1)

control = graph.SnapshotEvent.snapshot(graph.OrderEvent(T + 1_000_000_000, ticker="AAPL"))
assert control.book.action == "snapshot"
after = book.with_operations([control])
assert after.bid.is_empty and after.ask.is_empty
assert after.price is None
assert after.snapshot_partitions == [graph.SnapshotPartition("", "AAPL")]
```

## Read the stream through named views

A view is one `Plan` over the `marketdata` row, applied by the expression
engine and bound once per reader; lifts turn nested facts into columns.

```python
from yggdryl import Plan, enums, graph

T = 1_700_000_000_000_000_000
order = graph.OrderEvent(T, crosscode="O-1001", side="BUY", securityids={"ISIN": "US0378331005"})
root = graph.OrderEvent(T + 1_000_000_000, crosscode="T-1")
trade = graph.TradeEvent.from_parts(
    root,
    [
        graph.ExecutionEvent(T + 1_000_000_000, crosscode="E-1", side="BUY"),
        graph.ExecutionEvent(T + 1_000_000_000, crosscode="E-2", side="SELL"),
    ],
)

def stream():
    return graph.MarketData.arrow_reader([order, trade])

assert set(enums.MARKET_VIEWS) >= {"orders", "trades", "books", "lifecycle"}
orders = graph.MarketData.apply_view("orders", stream(), ["securityids['ISIN'] as isin"]).read_all()
assert orders.schema.names[-1] == "isin"
assert orders.column("isin").to_pylist() == ["US0378331005"]

# One row per execution, beside the trade's own columns.
trades = graph.MarketData.apply_view("trades", stream()).read_all()
assert sorted(trades.column("execution.crosscode").to_pylist()) == ["E-1", "E-2"]

# A view is a plan whose text reads back as the same plan.
plan = graph.MarketData.plan("trades")
assert Plan(str(plan)) == plan
chain = graph.MarketData.apply_view("lifecycle", stream(), crosscode="O-1001").read_all()
assert chain.column("crosscode").to_pylist() == ["O-1001"]
```

## Turn a FIX capture into books

A FIX capture reaches the graph through the codec: `lifecycle` settles each
message, `book_arrow_reader` folds sorted messages into `book_event` rows, and
`MarketData.from_arrow_reader` reads the books back.

```python
from decimal import Decimal
from pathlib import Path

from yggdryl import graph
from yggdryl.fix import FixCodec, FixRegistry

# `config/fix` of a yggdryl checkout (see the yggdryl-fix skill).
codec = FixCodec(FixRegistry.from_handle(Path("config/fix")))
lines = [
    b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|",
    b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|",
]
capture = list(codec.parse_lines(lines))

books = [value.as_book_event() for value in graph.MarketData.from_arrow_reader(codec.book_arrow_reader(codec.lifecycle(capture)))]
assert len(books) == 2
last = books[1]
assert last is not None and last.bid.best_price is not None
assert last.bid.best_price.as_py() == Decimal(101)
assert len(last.executions) == 1
```

## Serve a replay or mount the timeline

JavaScript-only: the replay service is `yggdryl/replay` and the browser
component `yggdryl/web/book-timeline.js` in the npm package. From Python,
write the sorted **operations** (not the books - the service walks them
itself) as `marketdata` rows to a `.parquet` or `.arrow` file, as above, and
serve it with `node node_modules/yggdryl/replay.js marketdata.parquet`.

## Gotchas in Python

- Seconds or milliseconds where nanoseconds are expected land in 1970; build
  instants with integer arithmetic, never `datetime.timestamp() * 1e9` (a float
  loses the last digits).
- `graph.BookIterator(items, snapshot_millis=0, global_=False)` - note the
  trailing underscore; `EventIterator(items, sorted=True, snapshot_ns=None)`
  defaults to trusting the order.
- Every verb answers a new value: `book.with_operations([...])` does not change
  `book`; `with_previous` / `merge_with` answer `None` when nothing moved.
- A leaf compares equal to its own class only: compare `value.into_leaf()` or
  `value.as_order_event()` with a leaf, `MarketData` with `MarketData`.
- `book.bid.limits` are struct `Scalar`s: `limit.as_py()` is a dict of
  `price`, `quantity`, `uuids`.
- Identifier verbs (`insert_securityid`, `insert_altid`, ...) are Rust-only:
  state `securityids=`, `altids=`, `accountids=`, `userids=` when you build.
