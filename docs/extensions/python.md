# Python

A native view of the same values the Rust core holds, with the protocols Python code expects.

```python
from yggdryl import DataType, Field, Url

# Every argument accepts the obvious Python spelling of itself.
schema = Field("row", DataType.from_fields([Field("id", "int64", nullable=False)]), nullable=False)
location = Url.from_path("C:/market data/trades.arrows")

assert schema.dtype.id == "struct"
assert str(location) == "file:///C:/market%20data/trades.arrows"
assert str(location.media_type.base) == "application/vnd.apache.arrow.stream"
```

This page documents the Python boundary only: what the package adds on top of the core, and how it
converts what you hand it. The behaviour itself is documented once, on the
[core pages](../index.md).

## Build from the repository

```console
cd python
python -m venv .venv
.venv/Scripts/python -m pip install maturin ".[test]"
.venv/Scripts/python -m maturin develop
.venv/Scripts/python -m pytest
```

On Linux and macOS the interpreter is `.venv/bin/python`.

## What it exposes

| Name | Documented in |
| --- | --- |
| `DataType` | [datatype](../datatype.md) |
| `Field`, `field`, `scalar`, `fields` | [field](../field.md) and this page |
| `Scalar` | this page and [text](../text.md) |
| `Expression`, `Bound`, `Statement`, `BoundStatement` | [expression](../expression.md) |
| `Uri`, `Url`, `Urn` | [uri](../uri.md) |
| `IOBase` | [io](../io.md) |
| `RecordOptions` | [io](../io.md), [ipc](../ipc.md), [parquet](../parquet.md) |
| `iceberg` | [iceberg](../iceberg.md) |
| `MimeType`, `MediaType`, `Timezone` | [enums](../generic.md) |
| `enums` | [generic](../generic.md) and this page |
| `json`, `toml`, `yaml` | [text](../text.md) and the format pages |
| `avro` | [Avro](../avro.md) schema, container, single-object, and batch media |
| `gzip`, `zlib`, `zstd` | [gzip](../gzip.md), [zlib](../zlib.md), [zstd](../zstd.md) |

The three coding modules carry the whole-buffer pair - `loads` and `dumps`, plus `loads_raw` and
`dumps_raw` on `zlib` - under the standard library's own module names, so swapping `import gzip`
for `from yggdryl import gzip` changes the engine and nothing else. Their streaming `reader`/
`writer` and the transparent `Gzip<H>`-style handles stay Rust-only: both are built on Rust's
`Read`/`Write`, which Python has no native spelling for. A handle still applies the coding its own
name declares without being told, and `IOBase.codec` is what asks it which one that is.

## Inference at the boundary

A constructor accepts the obvious spelling of its argument and converts once, in Rust. There is no
Python-side parser.

```python
from yggdryl import DataType, Field, MediaType, MimeType, Url

# A datatype expression is a datatype.
assert str(Field("id", "int64", nullable=False).dtype) == "int64"
assert DataType("list<int32>").id == "list"

# A media type is its canonical name.
assert str(MimeType("application/json")) == "application/json"
assert str(MediaType("application/json")) == "application/json"

# A path is a location.
assert str(Url.from_path("C:/tmp/a.json")) == "file:///C:/tmp/a.json"
```

`from_value` is the generic entry point on every wrapper: it inspects what it was handed - a native
value, a string, a PyArrow value, a Python type annotation - and dispatches to the matching core
constructor.

## Native `Scalar`

`Scalar` is a Python view of the Rust tree. Family factories select the native
width; `from_py` chooses the natural Python shape.

```python
from decimal import Decimal

import pyarrow as pa

from yggdryl import Scalar

price = Scalar.decimal("1234567890123456789012345678901234567890", 2)
assert price.kind == "d256"
assert price.as_py() == Decimal("12345678901234567890123456789012345678.90")
assert Scalar.float(1.5, 32).kind == "f32"
assert Scalar.date(1).kind == "date32"
assert Scalar.time(1, "us").kind == "time64"
assert Scalar.datetime(0, "s", "UTC").zone == "UTC"
assert Scalar.duration(1, "ms").kind == "duration32"

values = Scalar.from_arrow_array(pa.array([1, 2], type=pa.int16()))
assert values.into_arrow_array().type == pa.int16()

tree = Scalar.from_py({"legs": [{"id": 1}]})
assert tree["legs"][0]["id"].as_py() == 1
assert tree.set("venue", "XNAS")["venue"].as_utf8() == "XNAS"
```

`from_arrow_scalar`, `from_arrow_array`, `from_arrow_batch`, and
`from_arrow_table` import through Arrow C Data/C Stream. Their `into_arrow_*`
counterparts preserve exact physical types. Pass `field=` to cast to a declared
shape; empty output collections require it because they cannot infer a type.
Array conversion uses one native builder. A table is imported through Arrow C
Stream batch by batch, then owned as rows because a `Scalar` is materialized by
definition.

The factories are `float(value, width=64)`, `decimal(coefficient, scale=0)`,
`date(count, unit="d", timezone=None)`, and `time`, `datetime`, or `duration`
with `(count, unit, timezone=None)`. Exact physical variants remain visible in
`kind` and survive Arrow, pickle, and repr round trips. Only `datetime` accepts
a non-`NAIVE` zone; the core rejects it for date, time, and duration.

All values are hashable. `kind` and `dtype` expose their exact native type;
`as_bytes` and `as_utf8` expose the matching scalar payload, while
`as_json_bytes` and `as_json_utf8` use the core's natural JSON writer.
`len`, iteration, indexing, `get`, `path`, containment, and
`keys` / `values` / `items` keep child values native. `set` and `remove` are
persistent updates: they return a rebuilt `Scalar` and leave the source intact.

The same conversion pair backs codecs, expressions, and records:

```python
import datetime as dt
import zoneinfo
from decimal import Decimal

from yggdryl import json

value = {
    "price": Decimal("10.50"),
    "on": dt.date(2026, 8, 15),
    "since_midnight": dt.time(12, 30),
    "took": dt.timedelta(seconds=90),
    "at": dt.datetime(2026, 8, 15, 12, 30, tzinfo=zoneinfo.ZoneInfo("Europe/Paris")),
    "payload": b"\x00\xff",
}

restored = json.loads(json.dumps(value))

# The scale is data, so a price written to two places comes back to two.
assert str(restored["price"]) == "10.50"
# Binary uses interoperable base64 text. A Field is what turns it back into
# bytes; a schemaless read cannot distinguish base64 from ordinary text.
assert restored["payload"] == "AP8="
# A temporal travels as its classic ISO string, the loosely typed deal a
# schemaless wire makes; a field class or a schema recovers the typed
# reading. The zone survives as the zone name, not as the offset it
# happened to be at.
assert restored["on"] == "2026-08-15"
assert restored["took"] == "PT90.000000S"
assert restored["at"] == "2026-08-15T12:30:00.000000+02:00[Europe/Paris]"
```

| Python | Native value | Notes |
| --- | --- | --- |
| `None`, `bool`, `int`, `float`, `str`, `bytes` | `Null`, `Bool`, integer, `F64`, `String`, `Bytes` | an `int` up to 128 bits stays numeric |
| `decimal.Decimal` | `D128` or `D256` | coefficient and scale, never a float |
| `datetime.date` | `Date32` | days since the epoch |
| `datetime.time` | `Time64(us)` | zoned times are refused because Arrow time has no timezone parameter |
| `datetime.datetime` | `DateTime64(us)` | UTC-relative count plus non-null zone (`NAIVE` when absent) |
| `datetime.timedelta` | `Duration64(us)` | elapsed microseconds |
| `list`, `tuple` | `Sequence` | |
| `dict` | `Mapping` | keys are values too, not only strings |
| dataclass, named tuple, attribute object | `Record` | sorted string names; no second schema model |

Text codecs emit natural documents with no private value tags. Pass a `Field`
when strings or numbers need an exact decimal, binary, or temporal reading.
When the format is dynamic, [`yggdryl.codec`](../text.md#raw-document-codecs)
provides `from_io` / `from_stream` and `into_io` / `into_stream`: suffix or
core content inference selects the existing JSON, YAML, TOML, or JSON Lines
implementation, with no second parser in Python.

## Python value protocols

Canonical immutable wrappers compare, order, hash, copy, and pickle by their
complete native identity. This includes `DataType`, `MimeType`, `Timezone`,
`Scalar`, `Expression`, `Statement`, Avro schemas and containers, and the frozen
Iceberg `Compaction`, `PartitionField`, `PartitionSpec`, `Snapshot`,
`ManifestFile`, `DataFile`, and `ScanPlan` count report. `stable_hash()` returns
the deterministic native `u64`; Python's built-in `hash()` remaps that value to
`Py_hash_t` without changing equal-value hash agreement. An Avro fingerprint is
still Parsing Canonical Form, not the schema's complete behavioral identity.
Snapshot v1 `manifests`, v3 key and lineage fields, manifest encryption metadata, and every
Iceberg data-file count, bound, split, encryption, delete, and row-lineage
field stay available on these views.

Mutable identity wrappers - `Field`, `MediaType`, `Uri`, `Url`, `Urn`,
`RecordOptions`, and `IcebergOptions` - stay mutable until built-in `hash()` is
called. Hashing locks that instance against every equality-affecting mutation;
`stable_hash()` alone does not lock it, and an ordinary copy or unpickle is
unlocked. A field cached by the class decorator is independently read-only.

Operational objects have no invented identity: `IOBase`, cursors, listings and
iterators, catalog/table/namespace views, schema updates, bound expressions and
statements, Avro blocks, and metadata views are explicitly unhashable. Metadata
views still compare by their current content, like ordinary mapping views.
`ScanPlan` is the exception because its five stored counts are the entire public
bounded report, not a hidden executable plan.

```python
import copy
import pickle

from yggdryl import Expression, IOBase, Uri, Scalar

value = Scalar.from_py({"id": 1})
assert {value: "row"}[copy.copy(value)] == "row"
assert pickle.loads(pickle.dumps(value)) == value
assert Scalar.from_py(12).divide(3).as_py() == 4
assert isinstance((Expression("price") + 2) * 3, Expression)

location = Uri("https://example.com/data.json")
hash(location)
archive = location.joinpath("2026", "part.parquet")
assert archive == location / "2026/part.parquet"
try:
    location.set_extension("parquet")
except TypeError:
    pass
else:
    raise AssertionError("hashing must lock equality-affecting mutation")

try:
    hash(IOBase.from_bytes())
except TypeError:
    pass
else:
    raise AssertionError("a live handle has no value hash")
```

`Scalar`'s named arithmetic (`add`, `subtract`, `multiply`, `divide`,
`remainder`, `negate`, `absolute`) and Python operators are native checked
operations: invalid types raise `TypeError`, overflow raises `OverflowError`,
division by zero raises `ZeroDivisionError`, and inexact integer division raises
`ArithmeticError`. `Expression` exposes the same binary spellings as builders;
strings keep expression parsing, while other Python operands become native
literal `Scalar` nodes, including in reflected operators.

## What a Python value loses

Everything else is written as the closest natural shape, and its class does not survive the round
trip. This is a deliberate trade: a name over an untyped payload is not a type, because nothing
checks that the payload matches the name, so the binding carries the shape and lets the reader supply
the type.

```python
import pathlib
import uuid
from collections import deque

from yggdryl import json

value = {
    "tags": {"b", "a"},
    "queue": deque([1, 2], maxlen=8),
    "id": uuid.UUID("12345678-1234-5678-1234-567812345678"),
    "path": pathlib.PurePosixPath("lake/trades.arrow"),
}

restored = json.loads(json.dumps(value))

assert restored == {
    "tags": ["a", "b"],
    "queue": [1, 2],
    "id": "12345678-1234-5678-1234-567812345678",
    "path": "lake/trades.arrow",
}
```

| Python | Written as | What is lost |
| --- | --- | --- |
| `set`, `frozenset` | sequence, sorted | the type, and the original iteration order |
| `collections.deque` | sequence | the type and `maxlen` |
| `tuple` | sequence | that it was a tuple |
| `bytearray`, `memoryview` | bytes | the type |
| `uuid.UUID` | its text | the type |
| `pathlib.Path` and any `__fspath__` | its file-system string | the flavour, and on Windows the separator is `\` |
| `complex` | `[real, imag]` | the type |
| `range`, `slice` | `[start, stop, step]` | the type |
| `OrderedDict`, `Counter`, `defaultdict` | mapping | the type, and a `defaultdict`'s factory |
| a named tuple | record of its members | the class |
| an `enum.Enum` member | its value | the class |
| a dataclass | record of its fields | the class |
| any other object | record of its `__dict__` | the class |
| an `int` wider than 128 bits | its decimal text | that it was a number |
| `datetime.fold` | nothing | which reading of a repeated hour a *naive* value was |

A `fold` on an *aware* datetime does survive, because the offset it selects is baked into the
UTC-relative count the value carries.

Two losses are refusals rather than silent damage: a decimal whose coefficient needs more than 256
bits or whose exponent has no scale in `-128..=127` raises `OverflowError`, and a temporal finer than
a microsecond - which `datetime` cannot hold - raises `ValueError` instead of truncating.

## Reading a class back

Nothing in a document names a Python class, so the class comes from the call. `cls=` converts the
decoded mapping through the native Struct `Field` cached behind the class's
`field()` staticmethod; it never imports a module named by untrusted input.

```python
from yggdryl import json, scalar

@scalar
class Trade:
    trade_id: int
    symbol: str

encoded = json.dumps(Trade(1, "AAPL"))

# Without a target the document is what it says it is: data.
assert json.loads(encoded) == {"trade_id": 1, "symbol": "AAPL"}
assert json.loads(encoded, cls=Trade) == Trade(1, "AAPL")
assert Trade.field()["trade_id"].dtype.id == "int64"
```

A dataclass used as a dictionary *key* reads back asymmetrically: JSON and YAML
have no non-string mapping keys, so its untyped form is the tuple of its
entries. Supplying the decorated class as the target restores the declared
shape.

## Field metadata is a mapping

`field.metadata` implements the mapping protocol over the field's metadata, so ordinary Python
idioms work and the ordering is the native one. It is a live view of the field, not a copy.

Item access on the `Field` *itself* reaches a nested child, never a metadata key - the same thing
`DataType` subscripting means - so a schema walk gets one answer from every node.

```python
from yggdryl import Field

field = Field("trade", "int64", nullable=False, metadata={"source": "book"})
field.metadata["venue"] = "XPAR"

assert field.metadata["source"] == "book"
assert "venue" in field.metadata
assert len(field.metadata) == 2
assert sorted(field.metadata.keys()) == ["source", "venue"]
assert dict(field.metadata.items())["venue"] == "XPAR"

del field.metadata["venue"]
assert "venue" not in field.metadata
```

Typed identifiers and typed HTTP values (`parquet_field_id`, `alias`, `comment`, `content_type`,
`etag`, and the rest) are attributes rather than map keys, because they are validated. Rust splits
that list by whose vocabulary it is - the `http:` headers live on `field.as_http()` there, while
`parquet_field_id` and the straight `alias`/`comment`/`display`/`location` keys stay on `Field` -
so a Python name that reads `content_type` is `as_http().content_type()` on the other side.

One protocol's properties are a mapping of their own, and it is a live view of the same field rather
than a copy of part of it.

```python
from yggdryl import Field

field = Field("price", "int64", nullable=False)
field.iceberg["doc"] = "closing price"
field.postgres.update({"type": "numeric"})

assert field.iceberg["doc"] == "closing price"
assert dict(field.postgres.items()) == {"type": "numeric"}
assert len(field.iceberg) == 1
assert "doc" not in field.postgres

# The bare name is all the view needs; the full key is what the field stores.
assert field.iceberg.key("doc") == "iceberg:doc"
assert field.metadata["iceberg:doc"] == "closing price"
assert len(field.metadata) == 2

del field.iceberg["doc"]
assert not field.iceberg
```

Every well-known protocol is an attribute - `iceberg`, `postgres`, `http`, `arrow`, `spark`, `s3`,
and the rest - and `field.protocol(name)` takes one that is only known at runtime. There is no
`https` attribute, because HTTPS shares the canonical `http:` namespace. Rust spells the same
accessors `as_iceberg()` / `as_iceberg_mut()`, and two of them differ by more than the prefix:
`arrow` is `as_arrow_properties` and `field_properties` is `as_field_properties`, because `as_arrow`
and `as_field` already mean something else on a Rust field.

!!! note "Rust-only"
    Rust's per-protocol view *types* - `HttpField`, `IcebergField`, and the sixteen others, each
    carrying its protocol's typed vocabulary, and each dereferencing to the whole `Field` it borrows
    - have no Python counterpart yet. `field.iceberg` answers the generic property mapping above,
    and the validated HTTP values remain attributes on the field itself.

A schema also says which of its columns a path spells out, which is what a partitioned write and an
Iceberg spec both read.

```python
from yggdryl import DataType, Field

schema = Field(
    "row",
    DataType.from_fields([
        Field("year", "int32", nullable=False),
        Field("price", "int64", nullable=False),
    ]),
    nullable=False,
).with_partition_fields(["year"])

assert schema.partition_field_names == ["year"]
assert schema.dtype["year"].is_partition
assert len(schema.without_partition_fields().dtype) == 1
```

## Field classes

The `@scalar` decorator lives beside the Python `Scalar` boundary and compiles
class annotations into one native Struct `Field` while leaving a standard
dataclass.

```python
import dataclasses

from yggdryl import field, scalar

@scalar
class Trade:
    trade_id: int
    symbol: str

trade = Trade(trade_id=1, symbol="AAPL")

assert dataclasses.is_dataclass(Trade)
trade_field = Trade.field()
assert Trade.field() is trade_field
assert field(Trade) is trade_field
assert field(trade) is trade_field
assert trade_field.name == "Trade"
assert [child.name for child in trade_field] == ["trade_id", "symbol"]
```

`@scalar(...)` forwards every dataclass option. `Class.field()`
resolves one frozen native Struct field, caches it once per decorated class,
and returns that same object on every call. No codec, dictionary, or Arrow
methods are injected into the class. An undecorated subclass reuses the
nearest decorated base's root; decorate the subclass to make it a distinct
schema owner.

The [core field guide](../field.md#converting-to-one-native-field) owns the
canonical cross-runtime signatures and error contract. Python's global
conversion accepts a native Field, a PyArrow Schema/Field/DataType, or a
dataclass class/instance; `name` has identical rename semantics for each input
kind.

An Arrow schema takes the inverse route through the native value:

```python
import pyarrow as pa

from yggdryl import Field

row = Field.from_arrow_schema(
    pa.schema([pa.field("trade_id", pa.uint32(), nullable=False)]),
    name="Trade",
)
Trade = row.into_dataclass()

assert Trade.field() is row
assert Trade.field().into_arrow_schema().field("trade_id").type == pa.uint32()
```

The import preserves exact physical layout and metadata. Rebuilding the
dataclass derives annotations from that native graph rather than passing it
through annotation inference again.

## ASCII vocabularies as enums

`yggdryl.enums` carries the core's static spellings - `DATA_TYPE_IDS`, `CODECS`, `LEVELS` and the
rest, listed on the [generic page](../generic.md) - and the three enum bases a caller declares a
vocabulary with. `Ascii16`, `Ascii24`, `Ascii32`, `Ascii64`, `Ascii96`, and `Ascii128` are the
[ASCII widths](../datatype.md#ascii-widths-and-the-registered-names); a subclass names its
values as text and a member *is* the integer that value packs into, so the code is the same in
every process, is exactly what the column stores, and orders as the text does.

```python
from yggdryl import DataType
from yggdryl.enums import Ascii32

class Currency(Ascii32):
    USD = "USD"
    EUR = "EUR"

# A member is its value's own storage bytes, read big-endian.
assert int(Currency.USD) == 0x55534400
assert int(Currency.USD).to_bytes(4, "big") == b"USD\x00"
assert Currency.EUR < Currency.USD
assert Currency.dtype() == DataType("ascii32")

# The ASCII value is what a member renders as; `int(member)` asks for the code.
assert Currency.USD.into_str() == "USD"
assert f"{Currency.EUR}" == "EUR"

# The vocabulary is open: a value that was not declared reads back as a member
# under its own code, and every spelling of it is that one member.
jpy = Currency.from_str("JPY")
assert jpy is Currency("JPY") is Currency(0x4A505900)
assert [member.name for member in Currency] == ["USD", "EUR"]

# A value the width refuses is an error, not a silent unknown member.
try:
    Currency.from_str("EURO!")
except ValueError as error:
    assert "at most 4 bytes" in str(error)
else:
    raise AssertionError("a value wider than the width must be reported")
```

Sixteen bytes need the whole 128-bit integer, which Python holds natively:

```python
from yggdryl.enums import Ascii128

class Isin(Ascii128):
    APPLE = "US0378331005"

assert int(Isin.APPLE) == 0x55533033373833333130303500000000
assert Isin.APPLE.into_str() == "US0378331005"
```

A class declares itself onto a field, which stores its members under the reserved `field:enum`
key, so the enum crosses Arrow, a file, and the other binding as ordinary field metadata:

```python
from yggdryl import AsciiEnum, Field
from yggdryl.enums import Ascii32, AsciiCode

class Side(Ascii32):
    BUY = "B"
    SELL = "S"

field = Side.field("side", nullable=False)
assert field.dtype.id == "ascii32"
assert field.ascii_enum == AsciiEnum("Side", {"BUY": "B", "SELL": "S"})
assert field.get_property("field", "enum") == field.ascii_enum.into_json()

# The declaration is metadata, so the Arrow round trip carries it and it reads
# back as the class that wrote it.
recovered = AsciiCode.from_field(Field.from_arrow(field.into_arrow()))
assert recovered.__name__ == "Side"
assert [(member.name, int(member)) for member in recovered] == [
    (member.name, int(member)) for member in Side
]
```

A value read back that the class did not declare is announced on the `yggdryl.enums.ascii` logger
at `INFO`. It registers once and every later read answers the member that registration created, so
the record of a vocabulary read past its declaration is emitted exactly once per value:

```python
import logging

from yggdryl.enums import Ascii32

class Side(Ascii32):
    BUY = "B"
    SELL = "S"

records: list[logging.LogRecord] = []
handler = logging.Handler()
handler.emit = records.append  # type: ignore[method-assign]
logger = logging.getLogger("yggdryl.enums.ascii")
logger.addHandler(handler)
logger.setLevel(logging.INFO)
try:
    assert Side.from_str("X") is Side.from_str("X") is Side("X")
finally:
    logger.removeHandler(handler)
    logger.setLevel(logging.NOTSET)

assert [record.getMessage() for record in records] == [f"Side registered 'X' as {0x58000000}"]
```

The declared members are the declaration and a value read back is data, so `as_enum()` and
`field()` carry only what the class body names. `into_dictionary()` is the other direction: an
[`AsciiDictionary`](../datatype.md#the-dictionary-vocabulary-and-its-generated-enum) over the same
values, whose codes are positions in the column it encodes rather than the values themselves. Only
the leaf declares members - `AsciiCode` is the base the six widths share, and nothing subclasses
a vocabulary that already has members.

## Errors

A native error crosses unchanged and arrives as the idiomatic Python exception type - `ValueError`
for an invalid value, `TypeError` for an unusable argument - carrying the same message the Rust
error produced, including its path or byte offset.

```python
from yggdryl import DataType

try:
    DataType("decimal(0,0)")
except ValueError as error:
    assert "precision" in str(error)
else:
    raise AssertionError("an invalid precision must be reported")
```

## `pathlib`-shaped storage

`IOBase` is the core storage handle with the method names `pathlib.Path` already uses. The core
trait is positional and fully random-access, so there are no modes to open with and no cursor to
keep - `read_bytes`, `write_bytes`, `iterdir`, `glob`, `mkdir`, `touch`, and `unlink` mean here what
they mean on a `Path`, and each is answered by the core implementation for the backend the location
names.

```python
import pathlib
import tempfile

from yggdryl import IOBase, Url

root = pathlib.Path(tempfile.mkdtemp())

# Construction touches nothing, so a missing location is empty, not an error.
handle = IOBase(root / "trades.arrows")
assert not handle.exists()
assert handle.read_bytes() == b""

handle.write_text("AAPL")
assert handle.read_text() == "AAPL"
assert handle.size == 4
assert handle.is_io()

# Random access needs no mode.
handle.pwrite(0, b"MSFT")
assert handle.read_range_bytes(0, 4) == b"MSFT"

# Children resolve the way they do for a Path.
lake = IOBase(root / "lake" / "year=2024")
lake.mkdir()
(lake / "part-0.arrows").touch()
assert [entry.name for entry in lake.iterdir()] == ["part-0.arrows"]
assert len(list(IOBase(root / "lake").rglob("*.arrows"))) == 1
```

`is_io()` is the general capability check: an atomic byte value or a tabular
media returns `True`, while a container holding neither returns `False`.
`row_size` and `column_size` describe the whole logical record media,
independent of a projection, filter, or row limit used for a read. The native
core answers them lazily from IPC messages, Parquet metadata, Avro block
counts, text boundaries, or Iceberg manifests without decoding rows where the
format carries an exact count. Successful answers are retained only between
`open()` and `close()` and invalidated by writes through that handle.

`Url` answers the `PurePath` half under the same names - `name`, `stem`, `suffix`, `suffixes`,
`parts`, `parent`, `parents`, `joinpath`, `/`, `with_name`, `with_stem`, `with_suffix`, `match`,
`relative_to`, `is_relative_to`, `as_posix`, `as_uri` - plus `exists`, `is_dir`, and `is_file` for a
local URL.

```python
from yggdryl import Url

url = Url("file:///lake/trades/part-0.tar.gz")

assert url.name == "part-0.tar.gz"
assert url.suffix == ".gz"
assert url.suffixes == (".tar", ".gz")
assert url.parts == ("lake", "trades", "part-0.tar.gz")
assert str(url.parent) == "file:///lake/trades"
assert str(url.with_suffix(".parquet")) == "file:///lake/trades/part-0.tar.parquet"
assert url.match("*.gz")
assert url.relative_to(Url("file:///lake")) == "trades/part-0.tar.gz"
```

Where a `Path` would raise, this raises the same thing: `relative_to` on a location outside the
root is a `ValueError`, and `touch` on a directory is an `IsADirectoryError`. Where the two differ,
the difference is the point - a URL carries a scheme, so the same code addresses a local directory
and a bucket.

That is not a promise about a future backend. `IOBase.from_arrow_fs` takes any
`pyarrow.fs.FileSystem` and returns this same class, so everything above works unchanged over S3,
GCS, Azure, a `SubTreeFileSystem`, or a filesystem you wrote yourself as a `FileSystemHandler` -
which is also how an `fsspec` filesystem arrives. The boundary is only inference: the filesystem
is recognized without importing `pyarrow`, handed to the core's seven-method vtable, and never
seen again by the Python layer. [`arrowfs`](../arrowfs.md) documents the backend itself.

```python
import pathlib
import tempfile

import pyarrow.fs as pafs

from yggdryl import IOBase

root = pathlib.Path(tempfile.mkdtemp())
handle = IOBase.from_arrow_fs(pafs.LocalFileSystem(), (root / "trades.arrows").as_posix())

# An Arrow filesystem replaces whole files, so the write publishes on close.
with handle:
    handle.write_bytes(b"AAPL")

assert handle.read_bytes() == b"AAPL"
assert (root / "trades.arrows").read_bytes() == b"AAPL"

# IOBase(fs, path) infers the same thing the classmethod spells out.
assert str(IOBase(pafs.LocalFileSystem(), (root / "trades.arrows").as_posix()).url) == str(handle.url)
```

A Hive layout is readable from either side: `handle.partitions` and `url.partitions` return the
`column=value` pairs the path spells out, and `handle.children_where({"year": "2024"})` yields the
leaves carrying them, ready to rewrite.

### Bytes and ranges

`read_range_bytes` and `append_bytes` are the core's methods under their own names;
[io](../io.md#whole-values) states what they do. Over each sits one inferring entry point:
`read_range` chooses the answer's type from `cls`, and `append` chooses how to read the buffer it
was handed.

```python
import pytest

from yggdryl import IOBase

handle = IOBase.from_bytes(b"symbol,price\n")

# `cls` selects the answer's type; omitting it answers `bytes`.
assert handle.read_range(0, 6) == b"symbol"
assert handle.read_range(0, 6, cls=str) == "symbol"

# `append` takes text, a bytearray, and a memoryview as well as bytes.
assert handle.append("AAPL,1\n") == 13
assert handle.append(bytearray(b"MSFT,2\n")) == 20
assert handle.append(memoryview(b"NVDA,3\n")) == 27
assert handle.read_range(13, 7, cls=str) == "AAPL,1\n"

# A range it cannot decode is refused, not silently substituted.
with pytest.raises(ValueError):
    IOBase.from_bytes(b"\xff").read_range(0, 1, cls=str)
```

`read_range` accepts `bytes`, `str`, or `None` and raises `TypeError` for anything else, the way
`read_scalar(cls=...)` does. `append` reads `bytes`, `bytearray`, `memoryview`, and `str`, encoding
text as UTF-8 exactly as `write_text` does, and returns the byte offset the append landed at.

## Records use typed adapters

The handle exposes one read vocabulary and explicit write intent. `record_options()` derives the
encoding from the handle, `read_arrow_field()` returns its native root `Field`, and
`read_arrow_reader()` streams `pyarrow.RecordBatch` values. Every record call accepts only
`options=`; configure `options.field`, projection, limits, compression, and merge keys on that one
value rather than as parallel keywords.

The write name says both what Python is holding and what the operation means:

| Python value | Replace | Add rows | Keyed update/insert |
| --- | --- | --- | --- |
| `RecordBatchReader` or foreign Arrow C stream reader | `overwrite_arrow_reader` | `append_arrow_reader` | `merge_arrow_reader` |
| one `pyarrow.Table` | `overwrite_arrow_table` | `append_arrow_table` | `merge_arrow_table` |
| one `pyarrow.RecordBatch` | `overwrite_arrow_batch` | `append_arrow_batch` | `merge_arrow_batch` |
| iterable of mappings, sequences, or dataclass instances | `overwrite_records` | `append_records` | `merge_records` |

The same shapes have one configurable entry point when mode comes from
configuration. Its canonical order is input, required mode, then the one
keyword-only options value:

```text
write_arrow_reader(reader, mode, *, options=None)
write_arrow_table(table, mode, *, options=None)
write_arrow_batch(batch, mode, *, options=None)
write_records(records, mode, *, options=None)
write_pandas(frames, mode, *, options=None)
write_pandas_frame(frame, mode, *, options=None)
write_polars(frames, mode, *, options=None)
write_polars_frame(frame, mode, *, options=None)
```

`mode` is `"overwrite"`, `"append"`, or `"merge"`. It is required and never
inferred from `merge_by_names`; the shape remains explicit in the method name.

These adapters are intentionally strict. A table passed to `overwrite_arrow_reader` is refused even
though PyArrow can export it as a stream: `overwrite_arrow_table` preserves the representation the
caller actually holds. A scanner participates by handing over `scanner.to_reader()`. Row iterables
stay streaming and are grouped into at most `options.batch_row_size` rows; when a commit cadence falls
inside that grouping, conversion ends the current batch at the exact cadence boundary instead.
Empty records cannot infer a shape and therefore require `options.field`.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase

schema = pa.schema([
    pa.field("id", pa.int64(), nullable=False),
    pa.field("venue", pa.string()),
])
first = pa.record_batch({"id": [1, 2], "venue": ["XNAS", "XNYS"]}, schema=schema)
more = pa.table({"id": [3], "venue": ["XLON"]}, schema=schema)
root = pathlib.Path(tempfile.mkdtemp())
handle = IOBase(root / "trades.parquet")

# The handle's name picks the encoding; no call takes a format argument.
handle.overwrite_arrow_batch(first)
handle.append_arrow_table(more)

merge = handle.record_options()
merge.merge_by_names = ["id"]
updated = pa.RecordBatchReader.from_batches(
    schema,
    [pa.record_batch({"id": [2, 4], "venue": ["XPAR", None]}, schema=schema)],
)
handle.merge_arrow_reader(updated, options=merge)
assert handle.read_arrow_reader().read_all().column("id").to_pylist() == [1, 2, 3, 4]

# The configurable spelling reaches the same primitive and validation.
handle.write_arrow_table(more, "append")

# A declared field selects and casts during the read.
selected = handle.record_options()
selected.field = pa.schema([pa.field("id", pa.int64(), nullable=False)])
assert handle.read_arrow_reader(options=selected).schema.names == ["id"]
```

The selected method is authoritative. `merge_by_names` supplies identity to a `merge_*` call; it
does not turn overwrite or append into merge. Invalid intent is rejected before Python exports or
iterates the input, so an error never consumes the head of a generator.

`options.commit_row_size` is the optional publication cadence for every representation in the
table above, including pandas and polars. Its default `None` publishes once after successful end of
input. A positive `N` publishes every complete `N`-row group and the final remainder; overwrite
uses overwrite for the first group and append thereafter, while append and merge retain their
intent for every group. If conversion, source reading, native casting, or publication then fails,
completed prefixes remain visible by design.

```python
import pathlib
import tempfile

from yggdryl import IOBase

handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
incremental = handle.record_options()
incremental.commit_row_size = 10_000

# The generator is never collected. A failure after row 10,000 leaves that
# complete prefix published, and conversion does not inspect row 10,001 first.
handle.append_records(
    ({"id": row_id, "venue": None} for row_id in range(5, 20_005)),
    options=incremental,
)
```

`commit_row_size = 0` is invalid and is rejected before any Python input is inspected. A zero
`max_row_size` or `max_byte_size` is different: append is a no-op; overwrite publishes a typed
empty value directly from an explicit `options.field` and therefore requires that field without
asking the input for a schema. Limits apply once to the whole incoming stream, before it is split
into commits, and remain incompatible with keyed merge.

Decorated dataclasses use their cached native class field directly:

```python
import pathlib
import tempfile

from yggdryl import IOBase, scalar

@scalar
class Trade:
    id: int
    venue: str | None

handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
cached = Trade.field()
handle.overwrite_records([Trade(1, "XNAS"), Trade(2, None)])
assert Trade.field() is cached
assert list(handle.read_records(Trade)) == [Trade(1, "XNAS"), Trade(2, None)]
assert list(handle.read_records()) == [
    {"id": 1, "venue": "XNAS"},
    {"id": 2, "venue": None},
]

empty = handle.record_options()
empty.field = Trade.field()
handle.overwrite_records([], options=empty)
```

`read_records()` lowers only the current Arrow batch. With no class it yields plain mappings; with
a stdlib or `@scalar` dataclass type it constructs one instance per row.

### Record options

Record methods select an explicit write intent or require `mode`. Configure
field, selection, batch sizing, and merge keys on one `RecordOptions` value and
pass it as `options=`.

## pandas and polars

Neither library is a dependency, and neither is imported when `yggdryl` loads. An incoming value is
recognized by its *type's* module and qualified name, so a caller who has never installed polars pays
nothing for its support and never sees an `ImportError` about it. The import happens only inside the
one call that cannot proceed without it: reading rows *into* a frame.

```python
import pathlib
import tempfile

import pandas as pd

from yggdryl import IOBase

handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")

handle.overwrite_pandas_frame(pd.DataFrame({"id": [1, 2], "venue": ["XNAS", "XNYS"]}))

# The plural name streams: one frame per batch, converted when it is pulled.
assert sum(len(frame) for frame in handle.read_pandas()) == 2
# The `_frame` name is the whole thing in one frame.
assert list(handle.read_pandas_frame()["venue"]) == ["XNAS", "XNYS"]
```

The suffix says whether the call takes exactly one frame. Intent stays explicit in both forms:

| Streaming frames | Exactly one frame |
| --- | --- |
| `read_pandas()` / `read_polars()` | `read_pandas_frame()` / `read_polars_frame()` |
| `overwrite_pandas(frames)` / `overwrite_polars(frames)` | `overwrite_pandas_frame(frame)` / `overwrite_polars_frame(frame)` |
| `append_pandas(frames)` / `append_polars(frames)` | `append_pandas_frame(frame)` / `append_polars_frame(frame)` |
| `merge_pandas(frames)` / `merge_polars(frames)` | `merge_pandas_frame(frame)` / `merge_polars_frame(frame)` |

The named entry points are strict: `overwrite_pandas` handed a polars frame is a mistake worth
naming. A `polars.LazyFrame` is accepted and collected, because polars offers no way to hand its
rows over a batch at a time - that is polars' boundary, not this one's.

## An Iceberg table end to end

An Iceberg table is the same handle one level up, and a warehouse of them is one more:
`yggdryl.iceberg` carries the catalog, the table, the schema-evolution builder, and compaction,
each documented on the [iceberg](../iceberg.md) core page. PyArrow is the rows boundary in
both directions - a commit takes anything that exports an Arrow C stream, and every read, whether
a scan, a time travel, or an inspection table, returns a `pyarrow.RecordBatchReader`.
`update_schema()` is a context manager, so a recorded chain commits once on a clean exit and not
at all on an exception.

```python
import pathlib
import shutil
import tempfile

import pyarrow as pa

from yggdryl.iceberg import Catalog

warehouse = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-")) / "warehouse"
catalog = Catalog(warehouse)

# Rows and a dotted name are enough: the first append creates the table.
columns = pa.schema([
    pa.field("id", pa.int64(), nullable=False),
    pa.field("venue", pa.string()),
])
table = catalog.append(
    "nyc.trades", pa.table({"id": [1, 2], "venue": ["XNAS", "XNYS"]}, schema=columns)
)
past = table.current_snapshot.snapshot_id
table.append(pa.table({"id": [3], "venue": [None]}, schema=columns))
assert list(catalog.namespace("nyc").tables) == ["trades"]
assert table.scan().read_all().num_rows == 3

# A column change is recorded on the update and committed once, on exit.
with table.update_schema() as update:
    update.add_column("", "price: float64")
assert table.scan().read_all().column("price").to_pylist() == [None, None, None]

# Undersized files rewrite as one replace commit that reports itself.
compaction = table.compact()
assert (compaction.files_before, compaction.files_after) == (2, 1)
assert table.scan().read_all().num_rows == 3

# And nothing rewrote history: the first snapshot reads as it was written.
assert table.scan_at(past).read_all().column("id").to_pylist() == [1, 2]

shutil.rmtree(warehouse.parent)
```

The folder is the table and the walk is the same everywhere: the [iceberg](../iceberg.md)
page shows each of these steps beside its Rust and JavaScript form.
