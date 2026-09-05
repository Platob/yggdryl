# Python

The PyO3 binding holds the same native values the Rust core does, behind the protocols Python code expects.

## Contract

| Name | Documented in |
| --- | --- |
| `DataType` | [DataType](../types/datatype.md) |
| `Field`, `field`, `scalar`, `fields` | [Field](../types/field.md) and this page |
| `Scalar` | this page and [Scalar](../types/scalar.md) |
| `Expression`, `Bound`, `Statement`, `BoundStatement` | [Expression](../expression/index.md) |
| `Uri`, `Url`, `Urn` | [URI](../uri/index.md) |
| `IOBase` | [Holder](../holder/index.md) |
| `RecordOptions` | [RecordOptions](../media/options.md), [Arrow IPC](../media/ipc.md), [Parquet](../media/parquet.md) |
| `iceberg` | [Iceberg](../media/iceberg/index.md) |
| `MimeType`, `MediaType`, `Timezone` | [Scalar](../types/scalar.md) |
| `enums` | [ASCII](../types/ascii.md) and this page |
| `json`, `toml`, `yaml` | [Structured text](../text/index.md) and the format pages |
| `avro` | [Apache Avro](../media/avro.md) |
| `gzip`, `zlib`, `zstd` | [gzip](../coding/gzip.md), [zlib](../coding/zlib.md), [zstd](../coding/zstd.md) |
| `xxhash` | [xxHash](../xxhash/index.md) |

## Use

A constructor accepts the obvious spelling of its argument and converts once, in Rust.
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
`from_value` is the generic entry point on every wrapper. `DataType.from_regex(pattern, autotype=True)` reaches the core's named-capture inference.

## Native `Scalar`

`Scalar` is a Python view of the Rust tree, and `from_py` chooses the natural Python shape.
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
| Call | Behavior |
| --- | --- |
| `float(value, width=64)`, `decimal(coefficient, scale=0)` | the exact variant stays visible in `kind` |
| `date(count, unit="d", timezone=None)`, `time` / `datetime` / `duration` `(count, unit, timezone=None)` | only `datetime` takes a zone |
| `from_arrow_scalar` / `_array` / `_batch` / `_table` | Arrow C Data or C Stream; a table arrives batch by batch, then is owned as rows |
| `into_arrow_*` | exact physical types; `field=` casts to a declared shape |
| `as_bytes`, `as_utf8`, `as_json_bytes`, `as_json_utf8` | the scalar payload, then the core's natural JSON writer |
| `len`, iteration, indexing, `get`, `path`, `keys` / `values` / `items` | child values stay native |
| `set`, `remove` | persistent: a rebuilt `Scalar`, source intact |

All values are hashable, and `kind` survives Arrow, pickle, and repr round trips.
```python
import datetime as dt
import zoneinfo
from decimal import Decimal

from yggdryl.text import json

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
Pass a `Field` when strings or numbers need an exact decimal, binary, or temporal reading. [`yggdryl.text.codec`](../text/index.md) adds `from_io` / `from_stream` and `into_io` / `into_stream` for a dynamic format.

## Python value protocols

Wrappers fall into three identity classes, and `stable_hash()` is the deterministic native `u64` that `hash()` remaps to `Py_hash_t`.

- immutable: `DataType`, `MimeType`, `Timezone`, `Scalar`, `Expression`, `Statement`, Avro schemas and containers, the frozen Iceberg `Compaction`, `PartitionField`, `PartitionSpec`, `Snapshot`, `ManifestFile`, `DataFile`, `ScanPlan`.
- mutable until built-in `hash()` locks the instance: `Field`, `MediaType`, `Uri`, `Url`, `Urn`, `RecordOptions`, `IcebergOptions`.
- unhashable: `IOBase`, cursors, listings, iterators, catalog, table and namespace views, schema updates, bound expressions and statements, Avro blocks, metadata views.
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
`Expression` keeps expression parsing for strings, while other Python operands become native literal `Scalar` nodes, including in reflected operators.

## What a Python value loses

Everything else is written as the closest natural shape, and its class does not survive the round trip.
```python
import pathlib
import uuid
from collections import deque

from yggdryl.text import json

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
A `fold` on an *aware* datetime survives, because the offset it selects is baked into the UTC-relative count.

## Field metadata is a mapping

`field.metadata` is a live mapping view of the field in the native ordering. Item access on the `Field` itself reaches a nested child, never a metadata key.
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
Typed identifiers and typed HTTP values (`parquet_field_id`, `alias`, `comment`, `content_type`, `etag`) are validated attributes, not map keys. One protocol's properties are a live mapping of their own.
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
Every well-known [protocol](../types/protocol.md) is an attribute, and `field.protocol(name)` takes one known only at runtime. A schema also names the columns a path spells out, which a partitioned write and an Iceberg spec both read.
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

The `@scalar` decorator compiles class annotations into one native Struct `Field` while leaving a standard dataclass.
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
`@scalar(...)` forwards every dataclass option, and `Class.field()` caches one frozen Struct field per decorated class. Global conversion also accepts a native [`Field`](../types/field.md), a PyArrow Schema, Field, or DataType.
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
The import preserves exact physical layout and metadata.

## ASCII vocabularies as enums

`yggdryl.enums` carries the core's static spellings and the enum bases. `fixed_ascii(width)` builds one cached class per [ASCII width](../types/ascii.md), and a member *is* the integer its value packs into.
```python
from yggdryl import DataType
from yggdryl.enums import fixed_ascii

class Currency(fixed_ascii(4)):
    USD = "USD"
    EUR = "EUR"

# A member is its value's own storage bytes, read big-endian.
assert int(Currency.USD) == 0x55534400
assert int(Currency.USD).to_bytes(4, "big") == b"USD\x00"
assert Currency.EUR < Currency.USD
assert Currency.dtype() == DataType.ascii(4)

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
Sixteen bytes need the whole 128-bit integer, which Python holds natively.
```python
from yggdryl.enums import fixed_ascii

class Isin(fixed_ascii(16)):
    APPLE = "US0378331005"

assert int(Isin.APPLE) == 0x55533033373833333130303500000000
assert Isin.APPLE.into_str() == "US0378331005"
```
A class declares itself onto a field under the reserved `field:enum` key, so the declaration crosses Arrow, a file, and the other binding.
```python
from yggdryl import AsciiEnum, Field
from yggdryl.enums import AsciiCode, fixed_ascii

class Side(fixed_ascii(4)):
    BUY = "B"
    SELL = "S"

field = Side.field("side", nullable=False)
assert field.dtype.id == "fixed_ascii"
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
A value read back that the class did not declare registers once, announced on the `yggdryl.enums.ascii` logger at `INFO`.
```python
import logging

from yggdryl.enums import fixed_ascii

class Side(fixed_ascii(4)):
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
The declared members are the declaration, so `as_enum()` and `field()` carry only what the class body names.

### The registered vocabularies

`Country`, `Currency`, `MIC`, and `CFI` arrive declared over `CountryCode`, `CurrencyCode`, `MicCode`, and `CfiCode`. Subclass the base rather than the shipped class to declare your own over the same datatype.
```python
from yggdryl import DataType
from yggdryl.enums import CFI, Country, Currency, MIC

assert Currency.dtype() == DataType("currency")
assert Currency.dtype() != DataType.ascii(3)
assert int(Currency.USD) == 0x555344
assert (Country.FR, MIC.XPAR, CFI.ESVUFR) == (Country("FR"), MIC("XPAR"), CFI("ESVUFR"))
assert f"{MIC.XPAR} settles {Currency.EUR}" == "XPAR settles EUR"

# The standards are registries that keep growing, so a code no member declares
# is read under its own packed value rather than refused.
assert MIC.from_str("XLON").into_str() == "XLON"
```
A `@scalar` attribute typed with one of these carries that class's members as the field's declaration.
```python
from yggdryl import scalar
from yggdryl.enums import Currency, MIC

@scalar
class Fill:
    venue: MIC
    settlement: Currency

venue, settlement = Fill.field()
assert (venue.dtype.id, settlement.dtype.id) == ("mic", "currency")
assert settlement.ascii_enum.name == "Currency"
assert settlement.ascii_enum.get("USD") == "USD"
```
`yggdryl.types` names the same four codes as factories, for a field built without a class.
```python
from yggdryl import DataType, types

assert types.mic("venue").dtype == DataType("mic")
assert types.currency("ccy", nullable=False).dtype == DataType("currency")
```
## `pathlib`-shaped storage

`IOBase` is the core storage handle under the method names `pathlib.Path` already uses. The core trait is positional and fully random-access, so there are no modes and no cursor.
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
`is_io()` is the general capability check: a byte value or a tabular media is `True`, a container holding neither is `False`. `row_size` and `column_size` describe the whole record media, independent of any projection, filter, or limit.

`Url` answers the `PurePath` half under the same names, plus `exists`, `is_dir`, and `is_file` for a local URL.
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
`IOBase.from_fs` takes any `pyarrow.fs.FileSystem` and returns this same class, so everything above works over S3, GCS, Azure, or your own handler. The filesystem is recognized without importing `pyarrow`.
```python
import pathlib
import tempfile

import pyarrow.fs as pafs

from yggdryl import IOBase

root = pathlib.Path(tempfile.mkdtemp())
handle = IOBase.from_fs(pafs.LocalFileSystem(), (root / "trades.arrows").as_posix())

# An Arrow filesystem replaces whole files, so the write publishes on close.
with handle:
    handle.write_bytes(b"AAPL")

assert handle.read_bytes() == b"AAPL"
assert (root / "trades.arrows").read_bytes() == b"AAPL"

# IOBase(fs, path) infers the same thing the classmethod spells out.
assert str(IOBase(pafs.LocalFileSystem(), (root / "trades.arrows").as_posix()).url) == str(handle.url)
```
`handle.partitions` and `url.partitions` return the `column=value` pairs a Hive path spells out, and `handle.children_where({"year": "2024"})` yields the leaves carrying them.

### Bytes and ranges

`read_range_bytes` and `append_bytes` are the core's methods under their own names, stated in [bytes](../holder/iobase/bytes.md). `read_range` chooses the answer's type from `cls`, and `append` chooses how to read the buffer.
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
`append` encodes text as UTF-8 exactly as `write_text` does, and returns the byte offset it landed at.

## Records use typed adapters

The handle exposes one read vocabulary and explicit write intent, configured only through `options=`. The write name says what Python holds and what the operation means.
| Python value | Replace | Add rows | Keyed update/insert |
| --- | --- | --- | --- |
| `RecordBatchReader` or foreign Arrow C stream reader | `overwrite_arrow_reader` | `append_arrow_reader` | `merge_arrow_reader` |
| one `pyarrow.Table` | `overwrite_arrow_table` | `append_arrow_table` | `merge_arrow_table` |
| one `pyarrow.RecordBatch` | `overwrite_arrow_batch` | `append_arrow_batch` | `merge_arrow_batch` |
| iterable of mappings, sequences, or dataclass instances | `overwrite_records` | `append_records` | `merge_records` |
`record_options()` derives the encoding, `read_arrow_field()` returns the native root `Field`, and `read_arrow_reader()` streams `pyarrow.RecordBatch` values. One configurable entry point takes the mode instead.
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
`mode` is `"overwrite"`, `"append"`, or `"merge"`, required and never inferred from `merge_by_names`. Row iterables stay streaming, grouped into at most `options.batch_row_size` rows.
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
The selected method is authoritative, and `merge_by_names` only supplies identity to a `merge_*` call. `options.commit_row_size` is the publication cadence, defaulting to one publication after successful end of input.
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
A positive `N` publishes every complete `N`-row group and the final remainder. Overwrite uses overwrite for the first group and append thereafter, while append and merge keep their intent.
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
`read_records()` lowers only the current Arrow batch: no class yields plain mappings, a dataclass type builds one instance per row.

### Record options

Configure field, selection, batch sizing, and merge keys on one [`RecordOptions`](../media/options.md) value. `TextOptions` adds the pre-read row-header schema and row numbering of [plain-text records](../media/text.md).

## pandas and polars

Neither library is a dependency, and neither is imported when `yggdryl` loads. A value is recognized by its *type's* module and qualified name.
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
The suffix says whether the call takes exactly one frame.
| Streaming frames | Exactly one frame |
| --- | --- |
| `read_pandas()` / `read_polars()` | `read_pandas_frame()` / `read_polars_frame()` |
| `overwrite_pandas(frames)` / `overwrite_polars(frames)` | `overwrite_pandas_frame(frame)` / `overwrite_polars_frame(frame)` |
| `append_pandas(frames)` / `append_polars(frames)` | `append_pandas_frame(frame)` / `append_polars_frame(frame)` |
| `merge_pandas(frames)` / `merge_polars(frames)` | `merge_pandas_frame(frame)` / `merge_polars_frame(frame)` |
## An Iceberg table end to end

`yggdryl.media.iceberg` carries the catalog, the table, the schema-evolution builder, and compaction. PyArrow is the rows boundary both ways, and every read returns a `pyarrow.RecordBatchReader`.
```python
import pathlib
import shutil
import tempfile

import pyarrow as pa

from yggdryl.media.iceberg import Catalog

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
[Iceberg](../media/iceberg/index.md) shows each of these steps beside its Rust and JavaScript form.

## Reading a class back

Nothing in a document names a Python class, so the class comes from the call. `cls=` converts the decoded mapping through the native Struct `Field` cached behind the class's `field()` staticmethod.
```python
from yggdryl import scalar
from yggdryl.text import json

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
A dataclass used as a dictionary *key* reads back as the tuple of its entries, because JSON and YAML have no non-string keys.

## Digests

`yggdryl.xxhash` carries the four one-shot functions, the four resumable states, and `Digest`. `IOBase.read_digest` and `Scalar.digest` reach the same native path, at the algorithm's native width.
```python
from yggdryl import Scalar, xxhash

assert xxhash.xxh3(b"abc") == 0x78AF5F94892F3950
assert xxhash.xxh3("abc") == xxhash.xxh3(memoryview(b"abc"))

digest = xxhash.digest(b"abc", "xxh3-64")
assert str(digest) == "xxh3-64:78af5f94892f3950"
assert xxhash.Digest(str(digest)) == digest
assert int(Scalar.from_py("AAPL").digest()) == Scalar.from_py("AAPL").stable_hash()
```
A `bytes` or `str` is hashed in place, and any other buffer is read through one bounded 64 KiB window ([xxHash](../xxhash/index.md)).

## FIX registry at the boundary

`yggdryl.fix` carries `FixRegistry`, `FixMsg`, `global_registry()`, `install_global_registry()`, `STANDARD_BRANCH` (`"standard"`) and `STANDARD_TAG_LIMIT` (`5000`). The `fix:` vocabulary is six typed properties on the `field.fix` view: `branch`, `id`, `tag`, `tags`, `aliases`, `description`.

| Crossing | Rule |
| --- | --- |
| keys | an `int` is a tag, a `str` a name or dotted path in the standard branch; a colon-bearing string is a name, never an identifier |
| branches and identifiers | both cross as `str`, parsed once by the core; neither has a Python class |
| `field.fix.branch`, `field.fix.id` | `"standard"` when the key is absent, `None` exactly when `fix:tag` is absent; assigning an id moves both halves at once |
| lookups | `field_by_name` and `field_by_path` take the branch as their leading argument; `field_by_tag` means the standard branch |
| locations | `from_handle` and `write_into` take an `IOBase`, `Url`, `str`, or `PathLike`; a write creates `primitive/<branch>/` and `nested/<branch>/` |
| absence | a `KeyError` carrying the native message, while the `get_` twins answer `None` |
| `FixMsg` | immutable: equality over schema, value and dictionary, `hash()`, `copy` / `deepcopy`, and a pickle carrying the registry |

[FIX](../fix/index.md) owns resolution, folding, merging, sharding and validation.
```python
import copy
import pathlib
import pickle

import pytest

from yggdryl import DataType, Field, IOBase, Url
from yggdryl.fix import STANDARD_BRANCH, STANDARD_TAG_LIMIT, FixMsg, FixRegistry

seed = pathlib.Path("config/fix").resolve()

# One folder, named however Python names one - the coercion `Catalog` uses.
for location in (seed, str(seed), seed.as_uri(), Url(seed), IOBase(seed)):
    assert len(FixRegistry.from_handle(location)) == 34
registry = FixRegistry.from_handle(seed)

# A key is an int tag or a str name; a bool is neither, and a tag that would
# not fit i32 raises rather than narrowing.
assert registry[55] == registry["symbol"] == registry.field_by_tag(55)
with pytest.raises(TypeError, match="not bool"):
    registry[True]
with pytest.raises(OverflowError):
    registry.field_by_tag(2**31)
with pytest.raises(TypeError, match="int tag or a str name"):
    registry[3.5]

# A branch and an identifier cross as text: the branch leads a name or path
# lookup, and a malformed one is a ValueError rather than a miss.
assert STANDARD_BRANCH == "standard" and STANDARD_TAG_LIMIT == 5000
assert registry.field_by_name(STANDARD_BRANCH, "ticker").name == "Symbol"
assert registry.field_by_path(STANDARD_BRANCH, "NoPartyIDs.PartyID").fix.tag == 448
assert registry.field_by_id("standard:55").fix.id == "standard:55"
with pytest.raises(ValueError, match="fix branch"):
    registry.field_by_name("2cme", "Symbol")
with pytest.raises(ValueError, match="fix identifier"):
    registry.field_by_id("55")
with pytest.raises(TypeError):
    registry.field_by_id(55)

# Absence is a KeyError carrying the native message; a refusal is a ValueError.
with pytest.raises(KeyError) as absent:
    registry.field_by_name(STANDARD_BRANCH, "Nope")
assert absent.value.args[0] == 'expected a fix field at "name \\"Nope\\"", got nothing'
assert registry.get_field_by_name(STANDARD_BRANCH, "Nope") is None
with pytest.raises(ValueError, match="fix:tag"):
    registry.insert(Field("Untagged", "utf8"))

# A tag the FIX specification assigns cannot move to another dictionary.
vendor = Field("TradeID", "utf8")
vendor.fix.id = "CME:5001"
assert vendor.fix.id == "cme:5001" and vendor.fix.branch == "cme"
with pytest.raises(ValueError, match="fix:branch"):
    vendor.fix.tag = 35
assert vendor.fix.id == "cme:5001"

# A message shares the dictionary it resolved against, so mutating it refuses.
root = Field("row", DataType.from_fields([registry.field_by_tag(55)]), nullable=False)
message = FixMsg(root, {"Symbol": "AAPL"}, registry)
with pytest.raises(ValueError, match="shared with a message"):
    registry.remove(55)

# The message is a value: it hashes, copies and pickles, registry included.
assert copy.deepcopy(message) == message
assert pickle.loads(pickle.dumps(message)) == message
assert pickle.loads(pickle.dumps(message)).registry == registry
assert hash(message) == hash(FixMsg(root, message.value, registry))
assert message.branch == STANDARD_BRANCH
assert message.by_id("standard:55").as_py() == "AAPL"
assert message.get_by_id("cme:5001") is None
```
A `dict` is the obvious Python spelling of a named row, and the declared root is what says so. `FixMsg` reads one as the record its Struct field declares, while a `Map` field keeps its mapping.

## Edges

- `TextOptions.with_rownum` -> `None` or a signed 64-bit `int`; a `bool` is a `TypeError`, out of range an `OverflowError`.
- an empty output collection -> requires `field=`, because it cannot infer a type.
- a zoned `datetime.time`, or a zone on `date`, `time`, `duration` -> refused by the core.
- a decimal past 256 coefficient bits, or an exponent with no scale in `-128..=127` -> `OverflowError`.
- a temporal finer than a microsecond -> `ValueError`, not truncation.
- `Scalar` arithmetic -> `TypeError`, `OverflowError`, `ZeroDivisionError`, `ArithmeticError` on inexact integer division.
- built-in `hash()` on a mutable wrapper -> locks it against equality-affecting mutation; `stable_hash()` does not.
- `hash()` on a handle, cursor, view, or iterator -> `TypeError`.
- an Avro fingerprint -> Parsing Canonical Form, not complete behavioral identity.
- Rust's per-protocol view types (`HttpField`, `IcebergField`, and sixteen others) -> no Python counterpart yet.
- `field.https` -> absent, because HTTPS shares the canonical `http:` namespace.
- `arrow`, `field_properties` -> `as_arrow_properties`, `as_field_properties` on a Rust field.
- a decorated class -> gains no codec, dictionary, or Arrow methods; an undecorated subclass reuses the nearest decorated base's root.
- a value wider than the ASCII width -> `ValueError`, never a silent unknown member.
- `DataType("ascii")` -> any length and no packed integer, so no vocabulary.
- a missing location -> empty, not an error, because construction touches nothing.
- `row_size`, `column_size` -> lazy, retained only between `open()` and `close()`, invalidated by writes through that handle.
- `relative_to` outside the root -> `ValueError`; `touch` on a directory -> `IsADirectoryError`.
- a write through `IOBase.from_fs` -> published on close, because an Arrow filesystem replaces whole files.
- `read_range(cls=...)` outside `bytes`, `str`, `None` -> `TypeError`; a range it cannot decode -> `ValueError`.
- a `pyarrow.Table` handed to `overwrite_arrow_reader` -> refused; a scanner participates as `scanner.to_reader()`.
- empty records -> require `options.field`, and invalid intent is rejected before the input is iterated.
- `commit_row_size = 0` -> rejected before any Python input is inspected.
- a zero `max_row_size` or `max_byte_size` -> append is a no-op; overwrite publishes a typed empty value from `options.field`.
- a failure after a commit -> completed prefixes stay visible; limits apply once to the whole stream and refuse keyed merge.
- `overwrite_pandas` handed a polars frame -> refused; a `polars.LazyFrame` is accepted and collected.
- `update_schema()` -> commits once on a clean exit, not at all on an exception.
- `cls=` on a decode -> the cached native Struct field, never a module named by untrusted input.
- streaming `reader` / `writer` and the `Gzip<H>` and `Hashed<H>` handles -> Rust-only, built on Rust's `Read` / `Write`.
- `from yggdryl.coding import gzip` -> the standard library's module names with `loads` / `dumps`; `zlib` adds `loads_raw` / `dumps_raw`.
- a handle applies the coding its own name declares, and `IOBase.codec` asks which one.
- a `bool` FIX key -> refused by name, never 0 or 1; a tag outside `i32` -> `OverflowError`.
- a `fix:` property on another protocol's view -> `TypeError` naming that view's scheme.
- an absent registry folder -> loads empty and creates nothing; a retired `records/` folder -> `ValueError`.
- mutating a registry a `FixMsg` links or the installed default -> `ValueError`.
- every other native refusal -> the idiomatic Python exception with the Rust message, path or byte offset included.
```python
from yggdryl import DataType

try:
    DataType("decimal(0,0)")
except ValueError as error:
    assert "precision" in str(error)
else:
    raise AssertionError("an invalid precision must be reported")
```
## Commands

=== "Python"

    ```bash
    cd python
    python -m venv .venv
    .venv/bin/python -m pip install maturin ".[test]"
    .venv/bin/python -m maturin develop
    ```

    ```bash
    python/.venv/bin/python -m pytest python/tests
    python/.venv/bin/python -m pytest python/tests/types python/tests/test_enums.py
    python/.venv/bin/python -m pytest python/tests/holder
    python/.venv/bin/python -m pytest python/tests/coding
    python/.venv/bin/python -m pytest python/tests/media
    python/.venv/bin/python -m pytest python/tests/text
    python/.venv/bin/python -m pytest python/tests/uri
    python/.venv/bin/python -m pytest python/tests/expression
    python/.venv/bin/python -m pytest python/tests/xxhash
    python/.venv/bin/python -m pytest python/tests/fix
    python scripts/check_docs_examples.py --lang python
    ```

    ```bash
    python/.venv/bin/python python/benchmarks/types.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/arrow.py --iterations 10000
    python/.venv/bin/python python/benchmarks/holder.py --min-time 0.2 --repeat 7
    python/.venv/bin/python python/benchmarks/holder/io.py --iterations 10000
    python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
    python/.venv/bin/python python/benchmarks/media.py --min-time 0.2 --repeat 7
    python/.venv/bin/python python/benchmarks/media/text.py --min-time 0.05 --repeat 3
    python/.venv/bin/python python/benchmarks/media/iceberg.py --min-time 0.2 --repeat 5
    python/.venv/bin/python python/benchmarks/text.py --iterations 10000
    python/.venv/bin/python python/benchmarks/uri.py --iterations 2000
    python/.venv/bin/python python/benchmarks/xxhash.py --min-time 0.2 --repeat 5
    python/.venv/bin/python python/benchmarks/fix.py --iterations 2000
    ```
