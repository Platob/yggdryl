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

from yggdryl.fix import FixCodec, FixRegistry, fix_schema, global_registry, install_global_registry

# `config/fix` of a yggdryl checkout: the dictionary is not shipped in the wheel.
registry = FixRegistry.from_handle(Path("config/fix"))
assert registry.msgtype("D").name == "newordersingle"
assert registry.msgtype("8").name == "executionreport"

# Installed before anything resolves the default, it is what every default reads.
install_global_registry(registry)
assert global_registry() == registry
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

from yggdryl.fix import FixCodec, FixRegistry

codec = FixCodec(FixRegistry.from_handle(Path("config/fix")))
message = codec.parse_fix_line(b"8=FIX.4.4|35=D|52=20260102-10:15:30|11=A1|55=AAPL|54=1|38=100|44=10.5|10=0|")

assert (message.header().beginstring, message.header().msgtype) == ("FIX.4.4", "D")
# A coded value reads as its name; the wire keeps its code.
assert message.by_tag(54).as_py() == "BUY"
assert message.side.as_py() == "BUY"
assert message.quantity is not None and message.quantity.as_py() == Decimal(100)
assert message.by_name("symbol").as_py() == "AAPL"
# The first stated OrderID, ClOrdID, ... names the order's chain.
assert message.crosscode == "A1"
assert message.altids == {"CLORDID": "A1"}
# Instants are int nanoseconds since the epoch, UTC.
assert message.currunix == 1_767_348_930_000_000_000
# The entries are the content row as (tag, name, value, children) tuples.
assert [name for _, name, _, _ in message.entries()] == ["symbol", "side", "timeinforce"]
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
assert message.crosscode == "A1"

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
per message, the capture's own columns leading; parsing is pooled across
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
# The schema is decided before a row is read: the capture leads, `fixentries` closes.
assert read.schema.names[:3] == ["url", "rownum", "body"]
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
lines = [b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|9999=x|10=0|", b"8=FIX.4.4|35=8|17=E1|37=O9|31=12.75|32=50|10=0|"]
parsed = list(codec.parse_lines(lines))

# One message as one row, and back; a column is found by name.
row = parsed[0].into_row(schema)
assert row.as_py()[schema.index_of("msgtype")] == "D"
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
    assert sink.getvalue().decode().splitlines()[0].startswith("8=FIX.4.4|35=D|11=ORDER-1|9999=x|")
```

## Chain an order's lifecycle

`lifecycle` is the one cross-message stage: it collects the finite capture,
sorts it, folds repeated deliveries and chains each message to the live one of
its order under one `crossuuid`.

```python
from pathlib import Path

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
assert all(held.seqnum == 0 and held.prevuuid is None for held in parsed)

order, ack, fill = codec.lifecycle(parsed)
# Sorted by event time, joined by the identifiers each message went by.
assert (order.seqnum, ack.seqnum, fill.seqnum) == (0, 1, 2)
assert ack.prevuuid == order.curruuid and fill.prevuuid == ack.curruuid
assert ack.crossuuid == fill.crossuuid == order.crossuuid
assert fill.state.as_py() == "80FILLED"

# Rows already in Arrow chain in place, under the schema they were read with.
rows = codec.arrow_reader(fix_schema(registry), codec.parse_lines(lines))
chained = codec.lifecycle_arrow_reader(rows).read_all()
assert len(set(chained.column("crossuuid").to_pylist())) == 1
```

## Turn FIX into market operations and books

`market_operations` admits what a book folds, expands each message into graph
leaves and sorts them by the instant a book folds them at; `graph.BookIterator`
then walks them. Compose `lifecycle` in front when predecessor state matters.
`market_arrow_reader` writes the sorted operations as `marketdata` rows, and
`market_operations_arrow_reader` is its twin over batches of FIX rows already
in Arrow.

```python
from decimal import Decimal
from pathlib import Path

import pytest

from yggdryl import graph
from yggdryl.fix import FixCodec, FixRegistry, fix_schema

registry = FixRegistry.from_handle(Path("config/fix"))
codec = FixCodec(registry)
# The update arrives before the snapshot it follows.
lines = [
    b"8=FIX.4.4|35=X|52=20260921-10:00:01|55=AAPL|268=2|279=1|269=0|278=B1|270=101|271=11|279=0|269=2|278=T1|270=101|271=2|10=0|",
    b"8=FIX.4.4|35=W|52=20260921-10:00:00|55=AAPL|268=2|269=0|278=B1|270=100|271=10|269=1|278=A1|270=102|271=12|10=0|",
]
capture = list(codec.parse_lines(lines))

operations = list(codec.market_operations(codec.lifecycle(capture)))
assert operations[-1].kind == "execution_event"
books = list(graph.BookIterator(operations))
assert len(books) == 2
assert books[1].bid.best_price is not None and books[1].bid.best_price.as_py() == Decimal(101)

# The book door is strict: the same capture out of order is refused.
with pytest.raises(ValueError):
    codec.book_arrow_reader(capture).read_all()
# The sorted operations as `marketdata` rows.
assert codec.market_arrow_reader(capture).read_all().num_rows == 4
# The same operations off the capture's FIX rows.
fixed = codec.arrow_reader(fix_schema(registry, "fix"), capture)
assert codec.market_operations_arrow_reader(fixed).read_all().num_rows == 4
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
`add_cfb_file` / `add_cfb_files` fold one or a glob of them into a held
registry (dialect defaulting to each file's stem), and `merge_with` folds a
whole other registry. Every fold is atomic.

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

    venue, roots = FixRegistry.from_cfb_file(folder / "alpha.cfb", "venue")
    assert venue.field(4).fix.branches == ["venue"] and roots == []

    registry = FixRegistry()
    files, added, merged = registry.add_cfb_files(folder, "*.cfb")
    assert files == 2
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
- `FixMessages` streams raise at `next()`: a refused line raises `ValueError`
  where it is met and the stream goes on past it.
- A registry is mutable and unhashable; while a codec, message or iterator
  shares it, mutation is refused. Build the dictionary, then the codecs.
- A hashed `FixMsg` is frozen: `set` after `hash(message)` is a `TypeError`;
  `copy.copy(message)` takes writes again.
- `snapshot_ns=` is exact integer nanoseconds; `default_sending_time=` takes an
  aware UTC `datetime` or a nanosecond `Scalar` - pin it for reproducible reads
  of undated frames.
- `book_arrow_reader(..., global_=True)` - the trailing underscore avoids the
  keyword.
