# yggdryl-fix in Python

`from yggdryl.fix import FixCodec, FixMsg, FixRegistry, fix_schema` - batches in
and out are `pyarrow.RecordBatchReader`s, lines are `bytes`. The examples load
`Path("config/fix")`, the committed dictionary of a yggdryl checkout (not
shipped in the wheel): point it at your copy, or set `YGGDRYL_FIX_REGISTRY`.

## Load the committed dictionary once and share it

`FixRegistry.from_handle` takes a path, a `Url` or an `IOBase`; install it as
the process default and every `registry=None` door reads it.

```python
from pathlib import Path

from yggdryl.fix import FixCodec, FixRegistry, fix_schema

# `config/fix` of a yggdryl checkout: the dictionary is not shipped in the wheel.
registry = FixRegistry.from_handle(Path("config/fix"))
assert registry.msgtype("D").name == "newordersingle"
assert registry.msgtype("8").name == "executionreport"

# Installed before anything resolves the default, it is what every default reads.
FixRegistry.install_env(registry)
assert FixRegistry.from_env() == registry
orders = FixCodec(include_msgtypes=["D"])  # no registry: the process default
assert orders.registry == registry
assert fix_schema().index_of("msgtype") == fix_schema(registry).index_of("msgtype")
```

## Look fields up in the one namespace

A bare `int` is a tag, a `str` a folded name or a dotted path; a field identity
(`field.fix.id`) is only ever looked up through `field_by_id`.

```python
from pathlib import Path

from yggdryl.fix import FixRegistry

registry = FixRegistry.from_handle(Path("config/fix"))

assert registry.field(55).name == "symbol"
assert registry[55] == registry.field_by_name("Sym_Bol")
assert str(registry.field_by_tag(453).dtype) == "int32"
# The counter names the group it opens; a path reaches through the group.
assert registry.field_by_counter(453).name == "parties"
assert registry.field_by_path("Parties.PartyID").fix.tag == 448
# A name reads four word pairs either way: offer/ask, size/qty, bid/demand, px/price.
assert registry.field_by_name("AskPrice").fix.tag == 133
assert registry.field_by_name("DemandQty").fix.tag == 134

# The identity is the tag and the folded name; a bare integer is never one.
held = registry.field(55).fix.id
assert registry.field_by_id(held).name == "symbol"
assert registry.get_field(held) is None

# A field reads its values by a named code set the dictionary holds once.
side = registry.field(54)
assert side.fix.codeset == "sidecodeset"
codes = registry.codeset_of(side)
assert codes is not None and codes[0]["value"] == "1" and codes[0]["name"] == "Buy"
```

## Put FIX facts on a field

`field.fix` is the protocol view over the field's `FIX:` metadata; a refused
write raises and leaves the field unchanged.

```python
import pytest

from yggdryl import Field

field = Field("OrderQty", "decimal128(20, 8)")
field.fix.tag = 38
field.fix.names = ["Qty", "Quantity"]
field.fix.branches = ["Venue", "desk"]

assert field.fix.tag == 38
assert field.metadata["FIX:names"] == '["Qty","Quantity"]'
assert field.fix.branches == ["desk", "venue"]
# Derived on every read from the tag and the folded name, never stored.
spelled = Field("order_qty", "int64")
spelled.fix.tag = 38
assert spelled.fix.id == field.fix.id
assert "FIX:id" not in field.metadata

with pytest.raises(ValueError, match="FIX:tag"):
    field.fix.tag = 0
assert field.fix.tag == 38
```

## Decode one captured line

`parse_line` takes a whole captured line - verb, prose and remarks included - and
answers a lazy `FixMessages` stream: unpack it.

```python
from pathlib import Path

import pytest

from yggdryl.fix import FixCodec, FixRegistry

codec = FixCodec(FixRegistry.from_handle(Path("config/fix")))

message, = codec.parse_line(b"recv 8=FIX.4.4|35=D|453=1|448=BROKER|452=1|10=000|")
assert message.by_tag(453).as_py() == 1
assert message.by_path("Parties[0].PartyID").as_py() == "BROKER"

# Two frames on one line are two messages; a sentence is none.
both = b"8=FIX.4.4|35=D|11=A|10=001|8=FIX.4.4|35=8|37=O1|10=002|"
assert len(list(codec.parse_line(both))) == 2
assert list(codec.parse_line(b"After Enrichment -> ACCOUNT=A1 SIDE=1")) == []
# The single-frame door refuses a body holding a second frame.
with pytest.raises(ValueError, match="expected one frame"):
    codec.parse_fix_line(both)
```

## Read only the message types you need

The type filter is read off the `35=` a row states, before a message is built,
so a refused keepalive costs one look.

```python
from pathlib import Path

from yggdryl.fix import FixCodec, FixRegistry

registry = FixRegistry.from_handle(Path("config/fix"))
lines = [
    b"8=FIX.4.4|35=0|112=TEST|10=0|",
    b"8=FIX.4.4|35=D|11=A|55=AAPL|10=0|",
    b"8=FIX.4.4|35=8|37=O1|10=0|",
]

# The default refuses Heartbeat, TestRequest and the untyped row.
codec = FixCodec(registry)
assert codec.exclude_msgtypes == ["0", "1", "unknown"]
assert len(list(codec.parse_lines(lines))) == 2

# Naming what to read, in any spelling the dictionary resolves, is the whole answer.
orders = FixCodec(registry, include_msgtypes=["NewOrderSingle"])
assert orders.include_msgtypes == ["D"]
assert len(list(orders.parse_lines(lines))) == 1

# An empty refusal reads the session whole, as an audit does.
audit = FixCodec(registry, exclude_msgtypes=[])
assert len(list(audit.parse_lines(lines))) == 3

# Routing before any parse: the stated type, read without a dictionary.
assert FixCodec.infer_msgtype_bytes(b"8=FIX.4.4|35=AE|") == b"AE"
```

## Read typed facts off a message

A message holds each fact once: the header on `header()`, the market reading as
properties, everything else in the content row the dictionary typed.

```python
from decimal import Decimal
from pathlib import Path

from yggdryl import MarketDataKind, Side
from yggdryl.fix import FixCodec, FixRegistry

codec = FixCodec(FixRegistry.from_handle(Path("config/fix")))
message = codec.parse_fix_line(b"8=FIX.4.4|35=D|52=20260102-10:15:30|11=A1|55=AAPL|54=1|38=100|44=10.5|202=105|10=0|")

assert (message.header().beginstring, message.header().msgtype) == ("FIX.4.4", "D")
# The category its type files under, and the option strike it identifies.
assert message.msgcat is MarketDataKind.ORDR
assert message.strikeprice is not None and message.strikeprice.as_py() == Decimal(105)
# A coded value reads as its member; the wire keeps its code.
assert message.by_tag(54).as_py() is Side.BUYS
assert message.side is Side.BUYS
assert message.quantity is not None and message.quantity.as_py() == Decimal(100)
assert message.by_name("symbol").as_py() == "AAPL"
# The first stated OrderID, ClOrdID, ... names the order's chain, stored under its side.
assert message.crosscode == "10:1:A1"
# The names it goes by are identifiers: a source, a type and a value.
assert str(message.identifiers) == "[fix:clordid=A1]"
# Instants are int nanoseconds since the epoch, UTC.
assert message.currunix == 1_767_348_930_000_000_000
# The entries are the content row as (tag, name, value, children) tuples.
assert [name for _, name, _, _ in message.entries()] == ["symbol", "side", "strikeprice", "timeinforce"]
```

## Compose a message and write facts

`FixMsg(root, value, registry)` builds a message from a root `Field` and any
value the `Scalar` boundary reads; `set` and `remove` write a typed fact's
holder or the row, and settle the identity again.

```python
import pytest

from yggdryl import DataType, Field
from yggdryl.fix import FixMsg, FixRegistry

def tagged(name: str, tag: int) -> Field:
    field = Field(name, "utf8")
    field.fix.tag = tag
    return field

msgtype, clordid, symbol = tagged("MsgType", 35), tagged("ClOrdID", 11), tagged("Symbol", 55)
registry = FixRegistry.from_fields([msgtype, clordid, symbol])
root = Field("NewOrderSingle", DataType.from_fields([msgtype, clordid, symbol]), nullable=False)
message = FixMsg(root, {"MsgType": "D", "ClOrdID": "A1", "Symbol": "AAPL"}, registry)
assert message.header().msgtype == "D"
assert message.crosscode == "10:0:A1"

before = message.currhashcode
message.set("Symbol", "MSFT")
assert message.by_tag(55).as_py() == "MSFT"
assert message.currhashcode != before, "a write settles the identity again"
assert message.remove(55).as_py() == "MSFT"
assert message.get_by_tag(55) is None
with pytest.raises(KeyError):
    message.set("nosuchfield", "x")
```

## Encode a message back to the wire

`into_text` / `into_bytes` re-emit what the message states - header, lifted
fields, entries, trailer - with SOH unless you pass a separator; `digest`
hashes those bytes whatever the separator.

```python
from pathlib import Path

from yggdryl.fix import FixCodec, FixRegistry

codec = FixCodec(FixRegistry.from_handle(Path("config/fix")))
message = codec.parse_fix_line(b"8=FIX.4.4|35=D|52=20260102-10:15:30.000|55=AAPL|54=1|38=100|10=000|")

# Header, lifted quantity, entries (the side as its wire code, the derived
# day order), then the trailer.
text = message.into_text("|")
assert text == "8=FIX.4.4|35=D|52=20260102-10:15:30|38=100|55=AAPL|54=1|59=0|10=000|"
assert message.into_bytes() == text.replace("|", "\x01").encode()
assert message.into_text() == text.replace("|", "\x01")

# Emission is idempotent, and the digest ignores the separator.
again = codec.parse_fix_line(text.encode())
assert again.into_text("|") == text
assert again.digest() == message.digest()
```

## Stream a capture into Arrow rows

`parse_text_arrow_reader` takes a table, batch or reader with a payload column
(`body` by default) and answers a `pyarrow.RecordBatchReader` of FIX rows, one
per message, the capture's own columns following the shared ones; parsing is pooled across
`threads`.

```python
from pathlib import Path

import pyarrow as pa

from yggdryl.fix import FixCodec, FixRegistry

registry = FixRegistry.from_handle(Path("config/fix"))
capture = pa.table(
    {
        "url": ["file:///s.log"] * 3,
        "rownum": pa.array([7, 8, 9], pa.int64()),
        "body": [
            "recv 8=FIX.4.4|35=D|11=A|55=AAPL|10=0|",
            "heartbeat emitted seq=7",
            "8=FIX.4.4|35=D|11=B|55=MSFT|10=0|8=FIX.4.4|35=8|37=O1|11=B|10=0|",
        ],
    }
)

codec = FixCodec(registry, threads=4, batch_row_size=10_000)
read = codec.parse_text_arrow_reader(capture)
# The schema is decided before a row is read: the shared columns lead, the capture follows them, `fixentries` closes.
assert read.schema.names[0] == "curruuid"
at = read.schema.names.index("url")
assert read.schema.names[at - 1 : at + 3] == ["partyids", "url", "rownum", "body"]
assert read.schema.names[-1] == "fixentries"

held = read.read_all()
# One row per message: the sentence carried none, the last line two.
assert held.column("rownum").to_pylist() == [7, 9, 9]
assert held.column("symbol").to_pylist() == ["AAPL", "MSFT", None]
```

## Read a log file with a row header

Let the text reader cut lines and capture the row header, then hand its batches
to the codec: an `mtime` capture dates the line, and every other capture either
fills the FIX field it is named after or leads the row as context.

```python
import pathlib
import tempfile

from yggdryl import IOBase, TextOptions
from yggdryl.fix import FixCodec, FixRegistry

registry = FixRegistry.from_handle(pathlib.Path("config/fix"))
options = TextOptions()
options.rowheader = r"^(?P<mtime>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) (?P<level>IN|OUT) +"
options.timezone = "UTC"

with tempfile.TemporaryDirectory() as directory:
    log = pathlib.Path(directory) / "session.log"
    log.write_bytes(
        b"2026-01-02 10:15:30.250 IN  8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|10=0|\n"
        b"2026-01-02 10:15:30.500 OUT 8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|10=0|\n"
        b"2026-01-02 10:15:31.000 OUT 8=FIX.4.4|35=0|10=0|\n"
    )
    handle = IOBase(log).into_text(options)

    # Bulk: the text reader's batches straight into FIX rows.
    rows = FixCodec(registry, threads=2).parse_text_arrow_reader(handle.read_arrow_reader()).read_all()
    assert rows.num_rows == 2, "the heartbeat is refused by default"
    assert rows.column("level").to_pylist() == ["IN", "OUT"]
    # The line's clock became each message's instant; no SendingTime was invented.
    assert rows.column("sendingtime").null_count == 2

    # Line by line: tell the codec what the captures are called, once.
    lines = list(handle.read_text_lines())
    assert len(lines) == 3
    codec = FixCodec(registry, capture_names=list(options.capture_names))
    messages = list(codec.parse_text_lines(lines))
    assert [message.recdunix for message in messages] == [1_767_348_930_250_000_000, 1_767_348_930_500_000_000]
```

## Land messages in the fixed row and back

`fix_schema` is the one row every message answers as; `arrow_reader` and
`messages` cross between messages and batches, any record medium stores the
batches, and `write_arrow_reader` writes them back as wire lines.

```python
import io
import pathlib
import tempfile

from yggdryl import IOBase
from yggdryl.fix import FixCodec, FixMsg, FixRegistry, fix_schema

registry = FixRegistry.from_handle(pathlib.Path("config/fix"))
codec = FixCodec(registry, separator=ord("|"))
schema = fix_schema(registry, "fix")
lines = [b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|18=G|9999=x|10=0|", b"8=FIX.4.4|35=8|17=E1|37=O9|31=12.75|32=50|10=0|"]
parsed = list(codec.parse_lines(lines))

# One message as one row, and back; a column is found by name.
row = parsed[0].into_row(schema)
assert row.as_py()[schema.index_of("msgtype")] == "D"
# What no column holds is `fixentries`, keyed `tag:name`; a key no dictionary
# resolves is no field, and lands in `metadata` under its own spelling.
assert row.as_py()[schema.index_of("fixentries")] == {"18:execinst": "G"}
assert row.as_py()[schema.index_of("metadata")] == {"9999": "x"}
assert FixMsg.from_row(schema, row, registry).into_row(schema) == row

with tempfile.TemporaryDirectory() as directory:
    # A stream of messages as batches, landed in Parquet without a per-row detour.
    stored = IOBase(pathlib.Path(directory) / "capture.parquet")
    stored.overwrite_arrow_reader(codec.arrow_reader(schema, parsed))
    again = list(codec.messages(stored.read_arrow_reader()))
    assert [message.currhashcode for message in again] == [message.currhashcode for message in parsed]

    # And out to the wire, one line per row.
    sink = io.BytesIO()
    assert codec.write_arrow_reader(stored.read_arrow_reader(), sink) == 2
    assert sink.getvalue().decode().splitlines()[0].startswith("8=FIX.4.4|35=D|11=ORDER-1|18=G|9999=x|")
```

## Chain an order's lifecycle

`lifecycle` is the one cross-message stage: it collects the finite capture,
sorts it, folds repeated deliveries and chains each message to the live one of
its order and side under one `crossuuid`, within one market data kind (`msgcat`); a
report stating no side joins the one side alive under its identifiers. A fill's
execution, split off at the parse, is a chain of its own and never restates,
follows or ends its order. A codec pinned `sorted_lifecycle=True` reads a source already in
instant order as it comes, one epoch hour at a time, and answers the same walk. The walk yields
each `curruuid` once within `dedup_window_ms` of event time, one minute unless
the codec says otherwise; `dedup_window_ms=None` yields every restated twin too.
A snapshot grid's view is the live message as of its tick: dated at it, so its
`curruuid` is that instant's, with the live message's content and place.

```python
from pathlib import Path

from yggdryl import MarketDataKind, Side, State
from yggdryl.fix import FixCodec, FixRegistry, fix_schema

registry = FixRegistry.from_handle(Path("config/fix"))
codec = FixCodec(registry)
lines = [
    b"8=FIX.4.4|35=8|37=O1|150=F|39=2|14=100|151=0|31=10.5|32=100|55=AAPL|52=20260102-10:15:33.100|10=0|",
    b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|52=20260102-10:15:30.250|10=0|",
    b"8=FIX.4.4|35=8|11=A1|37=O1|150=0|39=0|55=AAPL|52=20260102-10:15:30.500|10=0|",
]
# Parsed, nothing follows anything: each names only the chain it spells.
parsed = list(codec.parse_lines(lines))
assert all(held.prevuuid is None for held in parsed)
# Each takes its place at its instant: the execution stands after its report.
assert [held.seqnum for held in parsed] == [0, 1, 0, 0]
# Three lines, four messages: the fill's report and the execution it reports.
assert len(parsed) == 4

order, ack, fill, execution = codec.lifecycle(parsed)
# Sorted by event time, joined by the identifiers each message went by; each
# follows one of an earlier instant, so each keeps its own place.
assert (order.seqnum, ack.seqnum, fill.seqnum) == (0, 0, 0)
assert ack.prevuuid == order.curruuid and fill.prevuuid == ack.curruuid
assert ack.crossuuid == fill.crossuuid == order.crossuuid
# The reports stated no side: they joined the buy alive under A1 and O1.
assert all(held.side is Side.BUYS and held.crosscode == "10:1:A1" for held in (ack, fill))
assert (fill.msgcat, fill.state) == (MarketDataKind.ORDR, State.FILLED)
# Every walked message states when its chain began.
assert ack.creaunix == fill.creaunix == order.currunix
assert (execution.msgcat, execution.state) == (MarketDataKind.EXEC, State.FILLED)
assert (execution.seqnum, execution.prevuuid) == (1, None)

# Rows already in Arrow chain in place, under the schema they were read with.
rows = codec.arrow_reader(fix_schema(registry), codec.parse_lines(lines))
chained = codec.lifecycle_arrow_reader(rows).read_all()
# Two chains: the order's, and its fill's execution.
assert chained.num_rows == 4 and len(set(chained.column("crossuuid").to_pylist())) == 2
```

## Follow a replace chain's parents

A message that states an identifier again under another value is a step in
its chain: `lifecycle` keeps the value before it as the type's parent
(`orderid` leaves `parentorderid` and the chain's first as `origorderid`,
`clordid` leaves `origclordid`), and joins a replace to its order by that
parent too. `registry.parents_of("orderid")` lists them, nearest first, from
the `FIX:parents` a field states.

```python
from pathlib import Path

from yggdryl.fix import FixCodec, FixRegistry

KINDS = ("orderid", "parentorderid", "origorderid", "clordid", "origclordid")
reader = FixCodec(FixRegistry.from_handle(Path("config/fix").resolve()))
lines = [
    b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|38=10|44=100|52=20260921-10:00:00|10=0|",
    b"8=FIX.4.4|35=8|11=C1|37=O1|150=0|39=0|55=AAPL|52=20260921-10:00:01|10=0|",
    b"8=FIX.4.4|35=G|11=C2|41=C1|37=O2|55=AAPL|54=1|38=10|44=101|52=20260921-10:00:02|10=0|",
    b"8=FIX.4.4|35=G|11=C3|41=C2|37=O3|55=AAPL|54=1|38=10|44=102|52=20260921-10:00:03|10=0|",
]
chained = list(reader.lifecycle(reader.parse_lines(lines)))


def held(message):
    # What a message holds under each type, "-" where it holds none.
    return tuple(message.identifiers.get_from("fix", kind) or "-" for kind in KINDS)


assert held(chained[0]) == ("-", "-", "-", "C1", "-")
assert held(chained[1]) == ("O1", "-", "-", "C1", "-")
# Each replace names the value before it and the chain's first.
assert held(chained[2]) == ("O2", "O1", "O1", "C2", "C1")
assert held(chained[3]) == ("O3", "O2", "O1", "C3", "C2")
# One chain: the replaces joined the order by the parent they state.
assert all(message.crossuuid == chained[0].crossuuid for message in chained)
```

## Split fills, two-sided quotes and batches at the parse

The parse splits what a message reports, once, so nothing downstream states a
fill or a side twice: an execution report is its order's report (`msgcat`
`ORDR`, its own state) - one of no fill from its parse - and one that fills
adds one `EXEC` message reading `FILLED`, chained under its `ExecID(17)` as given, else
`TradeID=<TradeID(1003)>`; a trade (`AE`) adds one sided execution per
`NoSides(552)` occurrence; a quote stating a bid and an offer and no side adds
a `BUYS` and a `SELL` quote; a batch (`msgcat` `ORDB`, `QUOB`, `EXEB` or `TRDB`:
an order list, a mass order, a cross, a mass quote, a match report) adds one
message per entry, filed under its item (`ORDR`, `QUOT`, `EXEC`, `TRAD`),
chained by the order the entry names and split again as its category is.
Each split message names its source in `srcuuids`.

```python
from decimal import Decimal
from pathlib import Path

from yggdryl import MarketDataKind, Side, State
from yggdryl.fix import FixCodec, FixRegistry

codec = FixCodec(FixRegistry.from_handle(Path("config/fix")))

fill = b"8=FIX.4.4|35=8|52=20260921-10:00:00|17=E-1|37=O-9|11=C-9|39=1|150=F|55=AAPL|54=1|38=100|14=40|32=40|31=10.5|10=0|"
report, execution = codec.parse_line(fill)
assert (report.msgcat, report.state) == (MarketDataKind.ORDR, State.PARTIALLY_FILLED)
assert (execution.msgcat, execution.state) == (MarketDataKind.EXEC, State.FILLED)
assert report.curruuid in execution.srcuuids
# An order, quote or execution message stores its cross code under its side; the fill is a chain of its own.
assert (report.crosscode, execution.crosscode) == ("10:1:O-9", "8:1:E-1")

quote = b"8=FIX.4.4|35=S|52=20260921-10:00:00|117=Q1|55=AAPL|15=USD|132=99|134=7|133=101|135=8|10=0|"
quote, bid, ask = codec.parse_line(quote)
assert (quote.side, bid.side, ask.side) == (Side.UNKN, Side.BUYS, Side.SELL)
assert (bid.crosscode, ask.crosscode) == ("14:1:Q1", "14:2:Q1")
# Each side prices at its own level and keeps the pair its source stated.
assert bid.price is not None and bid.price.as_py() == Decimal(99)
assert ask.bidpx is not None and ask.bidpx.as_py() == Decimal(99)
assert bid.bidccy is not None and bid.bidccy.as_py() == "USD"
```

## Turn FIX into market data and books

`market_data` admits what a book folds - orders, one-sided quotes, executions,
`W`/`X` book messages - reads each as its one graph leaf (a book message one
per entry) and sorts them by the instant a book folds them at;
`graph.BookIterator` then walks them. Compose `lifecycle` in front when
predecessor state matters. `market_arrow_reader` writes the sorted leaves as
`marketdata` rows, and `market_data_arrow_reader` is its twin over batches of
FIX rows already in Arrow.

```python
from decimal import Decimal
from pathlib import Path

from yggdryl import MarketDataKind, Side, graph
from yggdryl.fix import FixCodec, FixRegistry, fix_schema

registry = FixRegistry.from_handle(Path("config/fix"))
codec = FixCodec(registry)
# The update arrives before the snapshot it follows.
lines = [
    b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|",
    b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|",
]
capture = list(codec.parse_lines(lines))
assert all(message.msgcat is MarketDataKind.BOOK for message in capture)

leaves = list(codec.market_data(codec.lifecycle(capture)))
assert len(leaves) == 4, "one leaf per entry"
assert leaves[-1].kind == "execution_event"
books = list(graph.BookIterator(leaves))
assert len(books) == 2
best = books[1].best_price(Side.BUYS)
assert best is not None and best.as_py() == Decimal(101)

# The book door does not sort: the same capture out of order is no error - the
# snapshot dated before the book it would fold into is left out, with a warning.
assert codec.book_arrow_reader(capture).read_all().num_rows == 1
# The sorted leaves as `marketdata` rows.
assert codec.market_arrow_reader(capture).read_all().num_rows == 4
# The same leaves off the capture's FIX rows.
fixed = codec.arrow_reader(fix_schema(registry, "fix"), capture)
assert codec.market_data_arrow_reader(fixed).read_all().num_rows == 4
```

## Build and commit a dictionary

Build fields, components, groups and code sets in memory - a set before the
field naming it - then `commit` writes the shard tree and `from_handle` reads it
back whole.

```python
import pathlib
import tempfile

import yggdryl
from yggdryl import DataType, Field
from yggdryl.fix import FixRegistry

count = Field("NoPartyIDs", "int32")
count.fix.tag = 453
party_id = Field("PartyID", "utf8")
party_id.fix.tag = 448
registry = FixRegistry.from_fields([count, party_id])

# A Struct files as a component, a Serie of one as a group.
member = registry.field(448)
member.fix.field_ref = "PartyID"
party = Field("Party", DataType.from_fields([member]), nullable=False)
registry.insert(party)
parties = yggdryl.serie("Parties", party)
parties.fix.counter = 453
parties.fix.component = "Party"
registry.insert(parties)

# The vocabulary first, then the field that reads by it.
registry.set_codeset("sidecodeset", [{"value": "1", "name": "Buy"}, {"value": "2", "name": "Sell"}])
side = Field("Side", "utf8")
side.fix.tag = 54
side.fix.codeset = "sidecodeset"
registry.insert(side)

with tempfile.TemporaryDirectory() as directory:
    root = pathlib.Path(directory) / "catalog"
    report = registry.commit(root)
    assert report["written"] and report["removed"] == []
    assert (root / "codesets" / "sidecodeset.json").is_file()
    reloaded = FixRegistry.from_handle(root)
    assert reloaded == registry
    assert reloaded.field_by_path("Parties.PartyID").fix.tag == 448
```

## Fold a venue CBlock into a dictionary

`FixRegistry.from_cfb_file` reads one Ullink CBlock (`.cfb`) into a registry
and its declared roots, stamping the dialect on everything it produced;
`add_cfb_file` folds one into a held registry, `add_cfb_files(location)`
folds what one location holds - a glob (`folder / "*.cfb"`) every file it
matches, a folder the `.cfb` files directly inside it, a file itself - and
`merge_with` folds a whole other registry, dialect defaulting to each file's
own stem. Each answers a `dict` - `sources`, `added`, `merged`, `restated`,
`dropped` and `failed` - and a fold keeps every declaration the dictionary
already holds: a field whose source stated another precision of the stored
datatype (a CBlock's `float` against `decimal128`, `string` against `ccy`)
folds under it and is counted in `restated`, and a contradiction is passed
over into `dropped` rather than refusing the whole source. `add_cfb_files`
folds each file as one mutation: a file it cannot read, parse or fold is left
out alone, one `{"source", "reason"}` entry in `failed`, while the rest fold;
`add_cfb_file` and `merge_with` raise only for a source that leaves nothing
to keep. What the reader cannot keep of a file is a `logging` warning
under `yggdryl.fix.cfb` naming the line, the column, the element and what the
reader did instead.

```python
import pathlib
import tempfile

from yggdryl.fix import FixRegistry

cblock = """<?xml version="1.0" encoding="US-ASCII"?>
<cplugin-configuration fix-version="4.4">
  <vocabulary><vocabulary-tag name="4" alt="AdvSide" type="char" /></vocabulary>
  <maps><map name="ADVSIDE"><entries><entry key="{name}" value="B" /></entries></map></maps>
</cplugin-configuration>
"""
with tempfile.TemporaryDirectory() as directory:
    folder = pathlib.Path(directory)
    (folder / "alpha.cfb").write_text(cblock.format(name="buy"))
    (folder / "beta.cfb").write_text(cblock.format(name="venue_buy"))
    (folder / "broken.cfb").write_text("<cplugin-configuration><vocabulary>")

    venue, roots = FixRegistry.from_cfb_file(folder / "alpha.cfb", "venue")
    assert venue.field(4).fix.branches == ["venue"] and roots == []

    # A folder holds the .cfb files directly inside it, a glob what it matches.
    registry = FixRegistry()
    report = registry.add_cfb_files(folder)
    assert report["sources"] == 2
    assert report["restated"] == 0, "both files type tag 4 alike"
    assert report["dropped"] == []
    # One file is one mutation: the broken one is left out, the others fold.
    [failed] = report["failed"]
    assert failed["source"].endswith("broken.cfb")
    globbed = FixRegistry()
    globbed.add_cfb_files(folder / "*.cfb")
    assert globbed == registry
    # Ascending URL order, each file stamped with its stem.
    assert registry.field(4).fix.branches == ["alpha", "beta"]
    # A code set only widens: the held name wins a shared value.
    [code] = registry.codeset_of(registry.field(4))
    assert (code["value"], code["name"], code["aliases"]) == ("B", "buy", ["venue_buy"])

    registry.merge_with(venue)
    assert registry.field(4).fix.branches == ["alpha", "beta", "venue"]
```

## Gotchas in Python

- Lines are `bytes`: `parse_lines(["8=..."])` raises `TypeError`; encode first.
- `FixMessages` streams raise at `next()` only for a source failure - a
  `TypeError` for an item that is not bytes, or what the iterable behind it
  raised - and that ends the stream. What a line states that cannot be read
  is defaulted or left out with a `logging` warning under
  `yggdryl.<module path>` (`yggdryl.fix.messages`), logged once per kind.
- A registry is mutable and unhashable; while a codec, message or iterator
  shares it, mutation is refused. Build the dictionary, then the codecs.
- A hashed `FixMsg` is frozen: `set` after `hash(message)` is a `TypeError`;
  `copy.copy(message)` takes writes again.
- `snapshot_ns=` is exact integer nanoseconds; `dedup_window_ms=` is whole
  milliseconds, not given keeping one minute and `None` clearing it;
  `sorted_lifecycle=` is a `bool`,
  `False` unless the source is in instant order; `default_sending_time=` takes an
  aware UTC `datetime` or a nanosecond `Scalar` - pin it for reproducible reads
  of undated frames.
- `side`, `state` and `msgcat` answer `IntEnum` members (`Side.BUYS`,
  `State.FILLED`, `MarketDataKind.ORDR`): compare with `is`, never with text.
- `book_arrow_reader(messages, snapshot_millis=0)` takes no mode beyond the
  grid: books are keyed by ticker, else by `MIC:CFI`.
