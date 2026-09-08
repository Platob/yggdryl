# Python

The PyO3 binding holds the same native values the Rust core does, behind the protocols Python code expects.

## Contract

| Name | Documented in |
| --- | --- |
| `DataType` | [DataType](../types/datatype.md) |
| `Field`, `field`, `scalar`, `fields` | [Field](../types/field.md) and this page |
| `Scalar` | this page and [Scalar](../types/scalar.md) |
| `ArrowValue`, `arrow.SHAPES` | this page and [Values](../arrow/values.md) |
| `Expression`, `Bound`, `Statement`, `BoundStatement` | [Expression](../expression/index.md) |
| `Uri`, `Url`, `Urn` | [URI](../uri/index.md) |
| `IOBase`, and the role classes in `yggdryl.holder`, `yggdryl.coding`, `yggdryl.media` | this page and [Holder](../holder/index.md) |
| `RecordOptions` | [RecordOptions](../media/options.md), [Arrow IPC](../media/ipc.md), [Parquet](../media/parquet.md) |
| `iceberg` | [Iceberg](../media/iceberg/index.md) |
| `MimeType`, `MediaType`, `Timezone` | [Scalar](../types/scalar.md) |
| `enums` | [ASCII](../types/ascii.md) and this page |
| `json`, `toml`, `yaml` | [Structured text](../text/index.md) and the format pages |
| `avro` | [Apache Avro](../media/avro.md) schema, container, single-object, and batch media |
| `gzip`, `zlib`, `zstd` | [gzip](../coding/gzip.md), [zlib](../coding/zlib.md), [zstd](../coding/zstd.md) |
| `xxhash` | [xxHash](../xxhash/index.md) |
| `txhash` | [TxHash](../txhash/index.md) |
| `refresh_logging` | this page |
| the `ygg` command, installed on PATH by the wheel | [CLI](../fix/cli.md) |

## Use

A constructor accepts the obvious spelling of its argument and converts once, in Rust. There is no Python-side parser.

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

`from_value` is the generic entry point on every wrapper: a native value, a string, a PyArrow value, or a type annotation. `DataType.from_regex(pattern, autotype=True)` reaches the core's named-capture inference.

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
| `len`, iteration, indexing, `get`, `path`, containment, `keys` / `values` / `items` | child values stay native |
| `set`, `remove` | persistent: a rebuilt `Scalar`, source intact |
| `add`, `subtract`, `multiply`, `divide`, `remainder`, `negate`, `absolute` | checked native arithmetic, mirrored by the Python operators |

All values are hashable. `kind` and `dtype` expose the exact native type, which survives Arrow, pickle, and repr round trips.

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

## Arrow values

`ArrowValue` is the Arrow-shaped sibling of `Scalar`: one value across the four shapes Arrow spells a payload in, paired with the exact `Field` that types it. The contract is [Values](../arrow/values.md), and `yggdryl.arrow.SHAPES` names the shapes in widening order. JavaScript is not bound.

`from_py(value, field=None, *, safe=True, nullability="default", representation="value")` is the one entry point, and the value's own type decides the shape. `field` is the declared shape: given one, the core's single recursive cast applies it, one compiled plan for a stream and one batch at a time, under the [cast policy](../types/cast.md) those three keywords carry.

| Value handed to `from_py` | Shape |
| --- | --- |
| `pyarrow.Scalar`, and any other value a `Scalar` can hold | `scalar` |
| `pyarrow.Array`; `pyarrow.ChunkedArray`, whose chunks are combined; a pandas or polars `Series`; a one-dimensional `numpy.ndarray`; an `__arrow_c_array__` exporter | `array` |
| `pyarrow.RecordBatch`; a `numpy` record array, one column per member | `batch` |
| `pyarrow.Table`, which may hold many chunks; `pyarrow.RecordBatchReader`; a `Dataset` or `Scanner`; a pandas `DataFrame`; a polars `DataFrame` or `LazyFrame`; an `__arrow_c_stream__` exporter | `stream` |
| another `ArrowValue` | its own shape, taken rather than copied |

```python
import numpy as np
import pyarrow as pa

from yggdryl import ArrowValue
from yggdryl.arrow import SHAPES

assert SHAPES == ("scalar", "array", "batch", "stream")

table = pa.table({"symbol": ["AAPL", "MSFT"], "size": [100, 250]})

# A held batch knows its length; a table crosses over the C stream, and a
# stream states its schema before its first batch and nothing else.
held = ArrowValue.from_py(table.to_batches()[0])
assert (held.shape, held.row_size, held.column_size) == ("batch", 2, 2)
streamed = ArrowValue.from_py(table)
assert (streamed.shape, streamed.row_size, streamed.is_streamed) == ("stream", None, True)

assert ArrowValue.from_py(pa.scalar(7, pa.int64())).into_arrow_scalar().as_py() == 7
assert ArrowValue.from_py(np.array([1.5, 2.5])).shape == "array"
assert ArrowValue.from_py(pa.chunked_array([[1, 2], [3]])).into_numpy().tolist() == [1, 2, 3]

# A record dtype names its members, so it is rows.
records = np.array([("AAPL", 100)], dtype=[("symbol", "U4"), ("size", "i8")])
assert ArrowValue.from_py(records).column_size == 2

# The declared field is applied by the core's cast, and the shape survives it.
prices = ArrowValue.from_py(pa.array([1, 2, 3]), "price: float64 not null")
assert prices.shape == "array"
assert prices.into_arrow_array().type == pa.float64()
```

| Property | Answers |
| --- | --- |
| `shape` | one of `SHAPES` |
| `field` | the exact `Field`: one element for a scalar or a column, the non-null Struct root for a table or a stream |
| `row_size` | the rows, or `None` for a stream that has not been drained |
| `column_size` | the columns one row carries |
| `is_streamed` | whether reading this value consumes it |
| `is_consumed` | whether its stream has already been read |

| Call | Answers |
| --- | --- |
| `cast(field, *, safe, nullability, representation)` | another `ArrowValue` of the same shape, under the same policy `from_py` takes |
| `into_arrow_scalar()` | `pyarrow.Scalar`; anything but one row is a `ValueError` |
| `into_arrow_array()` | `pyarrow.Array` |
| `into_arrow_batch()` | `pyarrow.RecordBatch` |
| `into_arrow_table()` | `pyarrow.Table`, read from this value's own stream |
| `into_arrow_reader()` | `pyarrow.RecordBatchReader`, the cheapest crossing: nothing is collected |
| `into_pandas()` / `into_polars()` | one frame |
| `into_numpy()` | one `numpy.ndarray`; NumPy has no null mask and no nested layout, so this crossing copies |
| `into_scalar()` | the native `Scalar`: one value for a scalar, a sequence for every other shape |
| `as_py()` | Python's own types, named by the `Field`, so a row is a `dict` |

Only a stream is one-shot. A scalar, a column, and a table share their Arrow buffers back and stay readable; a stream has nothing to share until it is drained, so reading a consumed one is a `ValueError`.

```python
import pyarrow as pa
import pytest

from yggdryl import ArrowValue

table = pa.table({"symbol": ["AAPL", "MSFT"], "size": [100, 250]})
root = "row: struct<symbol: utf8 not null, size: int64 not null> not null"

# The Field is what puts the names back on a canonical positional row.
held = ArrowValue.from_py(table.to_batches()[0], root)
assert held.as_py() == [
    {"symbol": "AAPL", "size": 100},
    {"symbol": "MSFT", "size": 250},
]
assert held.into_pandas().shape == (2, 2)
assert held.into_arrow_batch().num_rows == 2
assert not held.is_consumed

# A canonical row is positional, which is what the native model speaks.
assert held.cast(root).into_scalar().as_py() == [["AAPL", 100], ["MSFT", 250]]

streamed = ArrowValue.from_py(table)
assert streamed.into_arrow_table().num_rows == 2
assert streamed.is_consumed
with pytest.raises(ValueError, match="crosses once"):
    streamed.into_arrow_reader()
```

## Python value protocols

Wrappers fall into three identity classes, and an immutable one compares, orders, hashes, copies, and pickles by its complete native identity. `stable_hash()` is the deterministic native `u64`, which `hash()` remaps to `Py_hash_t` without changing equal-value agreement.

- immutable: `DataType`, `MimeType`, `Timezone`, `Scalar`, `Digest`, `Expression`, `Statement`, Avro schemas and containers, the frozen Iceberg `Compaction`, `PartitionField`, `PartitionSpec`, `Snapshot`, `ManifestFile`, `DataFile`, `ScanPlan`.
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
field.digest["role"] = "holder"
field.identity.update({"role": "primary", "nulls": "distinct"})
field.partition.transform = "year"
field.partition.sources = ["event"]

assert field.iceberg["doc"] == "closing price"
assert dict(field.postgres.items()) == {"type": "numeric"}
assert len(field.iceberg) == 1
assert "doc" not in field.postgres
assert field.digest["role"] == "holder"
assert field.identity["role"] == "primary"
assert field.partition.transform == "year"

# The bare name is all the view needs; the full key is what the field stores.
assert field.iceberg.key("doc") == "iceberg:doc"
assert field.metadata["iceberg:doc"] == "closing price"
assert len(field.metadata) == 7

del field.iceberg["doc"]
assert not field.iceberg
```

Every well-known [protocol](../types/protocol.md) is an attribute, including `digest`, `identity` and `partition`, and `field.protocol(name)` takes one known only at runtime. `identity` holds arbitrary inert strings, while `digest["role"]` accepts only `"holder"`.

`partition` is the one view with a typed vocabulary of its own: `sources` names the field paths a column derives from and `transform` names the [expression](../expression/grammar.md) function that produces it, both answered only by `field.partition`. `apply_arrow_batch` is answered by `field.partition` and `field.digest` alike - it is the one verb both declaring protocols share - and [`field.apply_arrow_batch`](../types/field.md#applying-a-schemas-declarations) runs every step over one batch.

```python
import pyarrow as pa

from yggdryl import DataType, Field

year = Field("year", "int32", nullable=True)
year.partition.sources = ["event"]
year.partition.transform = "dayofmonth"

# A dialect alias resolves on the way in, so one canonical name is stored.
assert year.partition.transform == "day"

root = Field(
    "row",
    DataType.from_fields([Field("event", "date32", nullable=False), year]),
    nullable=False,
)
batch = pa.record_batch({"event": pa.array([19_723], pa.date32())})

assert root.partition.apply_arrow_batch(batch).column("year").to_pylist() == [1]
# Cast, then partition, then digest, each separately switchable.
assert root.apply_arrow_batch(batch, digest=False).column("year").to_pylist() == [1]
```

A schema also names the columns a path spells out, which a partitioned write and an Iceberg spec both read.

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

### Row digests

A Struct field's direct children define a row digest: every child except a `holder`, in declaration order. A holder narrows that on itself with `digest:sources`, so the fields it reads stay unmarked.

```python
import pyarrow as pa

from yggdryl import DataType, Field

identifier = Field("id", "int64", nullable=False)
price = Field("price", "int64", nullable=False)
holder = Field("row_digest", "uint64", nullable=False)
holder.digest["role"] = "holder"

fallback = Field(
    "row", DataType.from_fields([identifier, price, holder]), nullable=False
)
assert fallback.digest_field_names == ["id", "price"]
assert fallback.digest_field_len == 2
assert len(fallback.only_digest_fields().dtype) == 2

# A holder narrows its own input; the fields it reads stay unmarked.
holder.digest["sources"] = '["id"]'
assert dict(identifier.digest) == {}
assert holder.digest["sources"] == '["id"]'

batch = pa.record_batch(
    {"id": pa.array([1], pa.int64()), "price": pa.array([2], pa.int64())}
)
assert fallback.apply_arrow_batch(batch).column("row_digest").null_count == 0
```

Digest holders accept `int32`/`uint32` for XXH32 and `int64`/`uint64` for the 64-bit algorithms. `field.cast_arrow_array(values, representation="bits")` performs the same reversible [same-width reading](../types/cast.md#reading-the-bits) outside holder filling.

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

`@scalar(...)` forwards every dataclass option, and `Class.field()` caches one frozen Struct field per decorated class. Global conversion also accepts a native [`Field`](../types/field.md), a PyArrow Schema, Field, or DataType, or a dataclass class or instance.

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

The import preserves exact physical layout and metadata, and `into_dataclass` derives its annotations from that native graph.

## ASCII vocabularies as enums

`yggdryl.enums` carries the core's static spellings (`DATA_TYPE_IDS`, `CODECS`, `LEVELS`, and the rest) and the enum bases. `fixed_ascii(width)` builds one cached class per [ASCII width](../types/ascii.md), and a member *is* the integer its value packs into.

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

A `@scalar` attribute typed with one of these, or any `AsciiCode` subclass, carries that class's members as the field's declaration.

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
lake = IOBase(root / "lake" / "year=2024").mkdir()
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

`IOBase.from_fs` takes any `pyarrow.fs.FileSystem` and returns this same class: S3, GCS, Azure, a `SubTreeFileSystem`, an `fsspec` filesystem, or your own `FileSystemHandler`. It is recognized without importing `pyarrow` and handed to the core's seven-method [vtable](../holder/backends/filesystems.md).

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

### Handle roles

A location's name declares a content coding and a record implementation, and construction composes both. It reads the name and touches no store, so `IOBase(...)` answers the class that composition names.

| Construction | Handle |
| --- | --- |
| `IOBase("trades.txt.gz")` | `Text` over `Gzip` over `Path` |
| `IOBase("archive.bin.gz")` | `Gzip` |
| `IOBase("trades.parquet")`, `"trades.arrows"`, `"trades.avro"` | `Parquet`, `Ipc`, `Avro` |
| `IOBase("trades.log")` | `Text` |
| `IOBase("trades.json")`, `IOBase("trades")` | `Path`, because nothing this build reads is declared |
| `IOBase("trades.parquet.gz")` | `Path`, left uncomposed so the Parquet writer refuses it |
| `IOBase.from_bytes(b"...")` | `Buffer` |
| `IOBase.from_fs(fs, "k.txt.gz")` | `Text` over `Gzip` over `FsPath` |

The classes live in [`yggdryl.holder`](../holder/index.md), [`yggdryl.coding`](../coding/index.md), and [`yggdryl.media`](../media/index.md), and every one of them is an `IOBase` subclass that adds no state. `type(handle)` names the outermost layer, `repr(handle)` the whole composition, and `into_handle()` descends one layer.

```python
import pathlib
import tempfile

import pytest

from yggdryl import IOBase
from yggdryl.coding import Coded, Gzip
from yggdryl.holder import Path
from yggdryl.media import Media, Text

root = pathlib.Path(tempfile.mkdtemp())

handle = IOBase(root / "trades.txt.gz")
assert type(handle) is Text
assert repr(handle) == f'Text(Gzip(Path("{handle.url}")))'
# Text is a holder variant of its own, so it sits directly under IOBase.
assert not isinstance(handle, Media)

# The handle reads and writes the decoded value; the coding is a layer, and
# `codec` walks to it rather than reading this handle's own media type.
handle.write_bytes(b"AAPL\n")
assert handle.read_bytes() == b"AAPL\n"
assert handle.codec == "gzip"
assert str(handle.media_type) == "text/plain"

# Descending spends the handle it descended from.
coding = handle.into_handle()
assert type(coding) is Gzip and isinstance(coding, Coded)
assert type(coding.into_handle()) is Path
with pytest.raises(ValueError, match="consumed by a conversion"):
    handle.read_bytes()

# `Path`, `File`, `FsPath`, and `FsFile` commit to a byte role and skip the
# composition, which is how a coded name's stored bytes are addressed.
assert Path(root / "trades.txt.gz").read_bytes()[:2] == bytes.fromhex("1f8b")
```

`buffered`, `into_text`, and `into_coded` compose the same layers explicitly. Each answers the wrapper it built and spends the handle it took, because a wrapper owns the handle it wraps and a Python object cannot change class.

```python
import pathlib
import tempfile

import pytest

from yggdryl import IOBase
from yggdryl.coding import Zstd
from yggdryl.holder import Buffered, Folder

root = pathlib.Path(tempfile.mkdtemp())

cached = IOBase(root / "quotes.bin").buffered(page_size=4096)
assert type(cached) is Buffered
coded = cached.into_handle().into_coded("zstd")
assert type(coded) is Zstd
location = coded.url
assert repr(coded.into_text()) == f'Text(Zstd(Path("{location}")))'
with pytest.raises(ValueError, match="consumed by a conversion"):
    cached.read_bytes()

# `mkdir` and `create_dir` answer the container they created, for the same
# reason: a byte write here would have made this location a leaf.
assert type(IOBase(root / "lake").mkdir()) is Folder
assert type(Folder.temporary()) is Folder
```

`Folder.temporary()`, `Folder.home()`, and `Folder.config()` name the well-known roots and create nothing.

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

The selected method is authoritative, and `merge_by_names` only supplies identity to a `merge_*` call. `options.commit_row_size` is the publication cadence for every representation above, including pandas and polars, and defaults to `None`: one publication after successful end of input.

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

### Rows whatever the handle holds

`read_arrow_value(field=None)` and `write_arrow_value(value, mode="overwrite", field=None)` are the Arrow-shaped siblings of `read_scalar` and `write_scalar`, and the one pair that needs no prior knowledge of what a handle holds: a record encoding answers its batch stream, and a JSON, JSON Lines, YAML, or TOML document the batch its rows parse into. The written value crosses through [`ArrowValue`](#arrow-values), so every source `from_py` accepts reaches the same publication path.

```python
import pathlib
import tempfile

import pyarrow as pa
import pytest

from yggdryl import IOBase

root = pathlib.Path(tempfile.mkdtemp())
quotes = pa.table({"symbol": ["AAPL", "MSFT"], "size": [100, 250]})

# A record encoding takes every mode and answers rows as they arrive.
stream = IOBase(root / "quotes.arrows")
stream.write_arrow_value(quotes.to_pandas())
stream.write_arrow_value(quotes, "append")
assert stream.read_arrow_value().shape == "stream"
assert stream.read_arrow_value().into_arrow_table().num_rows == 4

# A text document is one frame around its rows: written whole, read as one
# batch, and refused any other mode.
document = IOBase(root / "quotes.jsonl")
document.write_arrow_value(quotes)
assert b'"symbol":"AAPL"' in document.read_bytes()
assert document.read_arrow_value().row_size == 2
with pytest.raises(ValueError, match="overwrite"):
    document.write_arrow_value(quotes, "append")
```

### Record options

Configure field, selection, batch sizing, compression, and merge keys on one [`RecordOptions`](../media/options.md) value. `TextOptions` adds the pre-read row-header schema, logical framing, leading-fragment treatment, per-record decoded-byte retention, and row numbering of [plain-text records](../media/text.md).

## pandas and polars

Neither library is a dependency, and neither is imported when `yggdryl` loads. A value is recognized by its *type's* module and qualified name, and the import happens only when rows are read into a frame.

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

A dataclass used as a dictionary *key* reads back as the tuple of its entries, because JSON and YAML have no non-string keys. Supplying the decorated class as the target restores the declared shape.

## Digests

`yggdryl.xxhash` carries the four one-shot functions, the four resumable states, and `Digest`. `IOBase.read_digest` and `Scalar.digest` reach the same native path, and a one-shot answers a plain `int` at its native width.

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

Each resumable state also exposes `apply_arrow_batch(root, batch)`, which fills default digest holders row by row from the root Field's digest metadata.

`yggdryl.txhash` couples an instant with that digest. Every `unix` argument is an `int`, a `datetime`, a `date`, timestamp text, or a `Scalar`, and the column functions take and answer `pyarrow` arrays ([TxHash](../txhash/index.md)).

```python
import datetime as dt

from yggdryl import txhash, xxhash

value = txhash.txh3(b"abc", dt.datetime(2023, 11, 14, 22, 13, 20, tzinfo=dt.timezone.utc))
assert value.unix == 1_700_000_000_000_000
assert int(value.digest) == xxhash.xxh3(b"abc")
assert bytes(value)[:8] == value.unix.to_bytes(8, "big", signed=True)
assert txhash.TxHash(str(value)) == value
```

## Watching what the core does

The native core reports its work through `logging`, under this package's own logger. A record's name is the Rust module path it came from, so `yggdryl.media.iceberg.table` and its siblings all hang off `yggdryl` and one `setLevel` is the whole switch.

Debug is an operation starting; info is one done, carrying the counts a monitor watches. Nothing is reported per row, per batch, or per file: a commit is the unit, so ten times the rows is the same handful of records. A dependency of the build reaches `logging` only at warning and above, so enabling debug narrates this project and nothing else.

```python
import logging
import tempfile
from pathlib import Path

import pyarrow as pa

from yggdryl import IOBase, refresh_logging
from yggdryl.media.iceberg import Table, assign_field_ids

records: list[logging.LogRecord] = []


class Collect(logging.Handler):
    def emit(self, record: logging.LogRecord) -> None:
        records.append(record)


watcher = logging.getLogger("yggdryl")
watcher.addHandler(Collect())
watcher.setLevel(logging.INFO)
# The bridge caches each logger's effective level, so a level set after import
# reaches it through this call.
refresh_logging()

schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
with tempfile.TemporaryDirectory() as folder:
    table = Table.create(IOBase(Path(folder) / "trades"), assign_field_ids(schema))
    table.append(pa.record_batch({"id": [1, 2, 3]}, schema=schema))
    assert sum(batch.num_rows for batch in table.scan()) == 3

said = [record.getMessage() for record in records]
assert any(message.startswith("created iceberg table at") for message in said)
assert any("wrote 3 rows as" in message for message in said)
assert any("data files to open" in message for message in said)
assert all(record.name.startswith("yggdryl") for record in records)
```

| Reported | Level | Carries |
| --- | --- | --- |
| a table created | info | location, format version, column count |
| a table opened | debug | location, metadata version |
| a scan planned | debug then info | manifests walked; files to open, files the filters excluded, manifests read and skipped |
| a snapshot written | debug then info | operation and snapshot id; rows, data files, bytes |
| a commit landed | info | metadata version and snapshot id |
| a commit beaten | debug | the version that won, the retry count, the wait |
| a compaction | debug then info | files and bytes rewritten, files produced |
| a schema evolved | info | the new schema id |
| snapshots expired | info | how many |
| an Arrow write session | debug then info | cadences published |

## FIX registry at the boundary

`yggdryl.fix` carries `FixRegistry`, `FixMsg`, `global_registry()`, `install_global_registry()`, `STANDARD_BRANCH` (`""`, what an absent `fix:branch` means), and `USER_TAG_MIN` (`5000`) and `USER_TAG_MAX` (`40000`), the half-open tag range a non-standard branch may claim. The `fix:` vocabulary is six typed properties on the `field.fix` view: `branch`, `id`, `tag`, `tags`, `aliases`, `description`.

| Crossing | Rule |
| --- | --- |
| keys | an `int` is a tag, a `str` a name or dotted path in the standard branch; a colon-bearing string is a name, never an identifier |
| branches and identifiers | both cross as `str`, parsed once by the core; neither has a Python class |
| `field.fix.branch`, `field.fix.id` | `""` when the key is absent, `None` exactly when `fix:tag` is absent; assigning `""` removes the key, and assigning a `"tag:branch"` id moves both halves at once |
| lookups | `field_by_name` and `field_by_path` take the branch after the name it qualifies, defaulting to the standard one; `field_by_tag` means the standard branch |
| locations | `from_handle` and `write_into` take an `IOBase`, `Url`, `str`, or `PathLike`; a write creates `primitive/<branch>/` and `nested/<branch>/` |
| absence | a `KeyError` carrying the native message, while the `get_` twins answer `None` |
| branch digests | `branch_by_bid` / `get_branch_by_bid` take the `int` an arrival entry carries and answer the `FixBranch` it names; only a declared branch resolves |
| `FixMsg.entries()` | `(tag, bid, key, value)` tuples, flattened pre-order, so a group's members follow the counter pair heading them |
| `FixMsg` | immutable: equality over schema, value and dictionary, `hash()`, `copy` / `deepcopy`, and a pickle carrying the registry |

[FIX](../fix/index.md) owns resolution, folding, merging, sharding and validation.

```python
import copy
import pathlib
import pickle

import pytest

from yggdryl import DataType, Field, IOBase, Url
from yggdryl.fix import STANDARD_BRANCH, USER_TAG_MAX, USER_TAG_MIN, FixMsg, FixRegistry

seed = pathlib.Path("config/fix").resolve()

# One folder, named however Python names one - the coercion `Catalog` uses.
held = len(FixRegistry.from_handle(seed))
for location in (seed, str(seed), seed.as_uri(), Url(seed), IOBase(seed)):
    assert len(FixRegistry.from_handle(location)) == held
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

# A branch and an identifier cross as text: a lookup takes the branch after
# the name it qualifies, an identifier is the tag then the branch, and a
# malformed one is a ValueError rather than a miss.
assert STANDARD_BRANCH == "" and (USER_TAG_MIN, USER_TAG_MAX) == (5000, 40000)
assert registry.field_by_name("SYMBOL").name == "symbol"
assert registry.field_by_path("NoPartyIDs.PartyID").fix.tag == 448
assert registry.field_by_id("55:").fix.id == "55:"
with pytest.raises(ValueError, match="fix branch"):
    registry.field_by_name("symbol", "2cme")
with pytest.raises(ValueError, match="fix identifier"):
    registry.field_by_id("55")
with pytest.raises(TypeError):
    registry.field_by_id(55)

# Absence is a KeyError carrying the native message; a refusal is a ValueError.
with pytest.raises(KeyError) as absent:
    registry.field_by_name("Nope")
assert absent.value.args[0] == 'expected a fix field at "name \\"Nope\\"", got nothing'
assert registry.get_field_by_name("Nope") is None
with pytest.raises(ValueError, match="fix:tag"):
    registry.insert(Field("Untagged", "utf8"))

# A tag the FIX specification assigns cannot move to another dictionary.
vendor = Field("TradeID", "utf8")
vendor.fix.id = "5001:CME"
assert vendor.fix.id == "5001:cme" and vendor.fix.branch == "cme"
with pytest.raises(ValueError, match="fix:branch"):
    vendor.fix.tag = 35
assert vendor.fix.id == "5001:cme"

# A message shares the dictionary it resolved against, so mutating it refuses.
root = Field("row", DataType.from_fields([registry.field_by_tag(55)]), nullable=False)
message = FixMsg(root, {"symbol": "AAPL"}, registry)
with pytest.raises(ValueError, match="shared with a message"):
    registry.remove(55)

# The message is a value: it hashes, copies and pickles, registry included.
assert copy.deepcopy(message) == message
assert pickle.loads(pickle.dumps(message)) == message
assert pickle.loads(pickle.dumps(message)).registry == registry
assert hash(message) == hash(FixMsg(root, message.value, registry))
assert message.branch == STANDARD_BRANCH
assert message.by_id("55:").as_py() == "AAPL"
assert message.get_by_id("5001:cme") is None
```

A `dict` is the obvious Python spelling of a named row, and the declared root is what says so. `FixMsg` reads one as the record its Struct field declares, while a `Map` field keeps its mapping.

## Edges

- `TextOptions.with_rownum` -> `None` or a signed 64-bit `int`; a `bool` is a `TypeError`, out of range an `OverflowError`.
- an empty output collection -> requires `field=`, because it cannot infer a type.
- a zoned `datetime.time`, or a zone on `date`, `time`, `duration` -> refused by the core.
- a decimal past 256 coefficient bits, or an exponent with no scale in `-128..=127` -> `OverflowError`.
- a temporal finer than a microsecond -> `ValueError`, not truncation.
- `Scalar` arithmetic -> `TypeError`, `OverflowError`, `ZeroDivisionError`, `ArithmeticError` on inexact integer division.
- a text codec document -> natural shapes with no private value tags.
- `yggdryl.text.codec` -> suffix or content inference selects the existing JSON, YAML, TOML, or JSON Lines implementation.
- an Avro fingerprint -> Parsing Canonical Form, not complete behavioral identity.
- `stable_hash()` -> never locks a mutable wrapper, and a copy or an unpickle arrives unlocked.
- metadata views -> unhashable, but compare by their current content like ordinary mapping views.
- Iceberg views -> keep snapshot v1 `manifests`, v3 key and lineage fields, manifest encryption metadata, and every data-file count, bound, split, encryption, delete, and row-lineage field.
- Rust's per-protocol view types (`HttpField`, `IcebergField`, `DigestField`, `IdentityField`, and sixteen others) -> no Python counterpart yet; the `partition:` vocabulary is the exception.
- `sources` or `transform` on another protocol's view -> `TypeError` naming that view's scheme; `apply_arrow_batch` is answered by `partition` and `digest` and refuses every other.
- `field.apply_arrow_batch` -> `cast`, then `partition`, then `digest`, all three on by default.
- a `partition` transform of two arguments, `truncate` among them -> `ValueError`, and the field is left unchanged.
- a derived column the batch already carries with values -> left alone; one absent or all-null is filled.
- `field.https` -> absent, because HTTPS shares the canonical `http:` namespace.
- `field.iceberg` in Rust -> `as_iceberg()` / `as_iceberg_mut()`; `arrow` is `as_arrow_properties` and `field_properties` is `as_field_properties`.
- `field.content_type` -> `as_http().content_type()` in Rust, where the `http:` headers live.
- a decorated class -> gains no codec, dictionary, or Arrow methods; an undecorated subclass reuses the nearest decorated base's root.
- a field cached by `@scalar` -> read-only independently of the `hash()` lock.
- `name=` on a global conversion -> identical rename semantics for a `Field`, a PyArrow input, and a dataclass.
- `DataType("ascii")` -> any length and no packed integer, so no vocabulary.
- an ASCII enum member -> the same code in every process, exactly what the column stores, ordered as its text is.
- a vocabulary that already has members -> never subclassed; `AsciiCode` is the base every width and code shares.
- `row_size`, `column_size` -> lazy, retained only between `open()` and `close()`, invalidated by writes through that handle.
- `relative_to` outside the root -> `ValueError`; `touch` on a directory -> `IsADirectoryError`.
- `read_range(cls=...)` outside `bytes`, `str`, and `None` -> `TypeError`, the way `read_scalar(cls=...)` refuses one.
- a `pyarrow.Table` handed to `overwrite_arrow_reader` -> refused; a scanner participates as `scanner.to_reader()`.
- an `ArrowValue` of shape `stream` -> one-shot; every other shape shares its Arrow buffers back and stays readable, and reading a consumed stream is a `ValueError`.
- `ArrowValue.from_py` of a `numpy` array of more than one dimension -> `TypeError`, because Arrow has no column of that shape.
- `write_arrow_value` on a JSON, JSON Lines, YAML, or TOML handle -> `"overwrite"` only; append and merge go through the record adapters.
- `ArrowValue` in JavaScript -> not bound, because a stream has no honest copied-IPC representation.
- empty records -> require `options.field`, and invalid intent is rejected before the input is iterated.
- a commit cadence falling inside a batch grouping -> conversion ends the current batch at the exact cadence boundary.
- `commit_row_size = 0` -> rejected before any Python input is inspected.
- a zero `max_row_size` or `max_byte_size` -> append is a no-op; overwrite publishes a typed empty value from `options.field`.
- a failure after a commit -> completed prefixes stay visible by design.
- limits -> applied once to the whole incoming stream, before it is split into commits, and refused with keyed merge.
- `overwrite_pandas` handed a polars frame -> refused; a `polars.LazyFrame` is accepted and collected.
- `update_schema()` on an exception -> no commit at all.
- an Iceberg commit -> anything that exports an Arrow C stream; a scan, a time travel, and an inspection table answer a `pyarrow.RecordBatchReader`.
- `cls=` on a decode -> the cached native Struct field, never a module named by untrusted input.
- streaming `reader` / `writer` and the `Hashed<H>` handle -> Rust-only, built on Rust's `Read` / `Write`.
- `from yggdryl.coding import gzip` -> the standard library's module names with `loads` / `dumps`; `zlib` adds `loads_raw` / `dumps_raw`.
- a handle applies the coding its own name declares, and `IOBase.codec` asks which one.
- `compress_into` / `decompress_into` on a handle presenting a decoded view -> refused, because a coded handle already codes what passes through it; address the stored bytes with `Path`, `File`, `FsPath`, or `FsFile`, or use `copy_into`.
- `open()` -> caches metadata for the composition already in place; it promotes nothing under the class.
- a handle `buffered`, `into_text`, `into_coded`, or `into_handle` already spent -> `ValueError`, because the wrapper it answered owns the value now.
- `apply_arrow_batch` -> retains a stored non-default holder without consuming the state; `force=True` recomputes it.
- a signed digest holder column -> high-bit results read as negative Python integers, and every digest bit is retained.
- a `fix:` property on another protocol's view -> `TypeError` naming that view's scheme.
- an absent registry folder -> loads empty and creates nothing; a retired `records/` folder -> `ValueError`.
- `registry[key]`, `registry.get`, `key in registry`, and `FixMsg[key]` -> the same int-tag or str-name pair.
- `FixRegistry` -> mutable, so unhashable, and equal by the fields it holds.
- a registry linked by a `FixMsg` or installed as the process default -> `insert`, `update`, and `remove` raise `ValueError`.
- `remove` -> reaches the standard branch only; `remove_by_id` is how a vendor field leaves.
- `msg.by_id` / `msg.get_by_id` -> name one dictionary exactly and do not tier; `msg.branch` comes from the root field.
- iterating a `FixMsg` -> `(name, Scalar)` pairs in the root's declared order; `value` answers a `Scalar`, `field` a `Field`.
- a native `Scalar` or a sequence in the root's own order -> crosses untouched, at every depth, including a repeating group's occurrence.
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
    python/.venv/bin/python -m pytest python/tests/txhash
    python/.venv/bin/python -m pytest python/tests/arrow
    python/.venv/bin/python -m pytest python/tests/fix
    python scripts/check_docs_examples.py --lang python
    ```

    ```bash
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/arrow.py --iterations 10000
    python/.venv/bin/python python/benchmarks/arrow.py --iterations 10000
    python/.venv/bin/python python/benchmarks/holder.py --min-time 0.2 --repeat 7
    python/.venv/bin/python python/benchmarks/holder/io.py --iterations 10000
    python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
    python/.venv/bin/python python/benchmarks/media.py --min-time 0.2 --repeat 7
    python/.venv/bin/python python/benchmarks/media/text.py --min-time 0.05 --repeat 3
    python/.venv/bin/python python/benchmarks/media/iceberg.py --min-time 0.2 --repeat 5
    python/.venv/bin/python python/benchmarks/text.py --iterations 10000
    python/.venv/bin/python python/benchmarks/uri.py --iterations 2000
    python/.venv/bin/python python/benchmarks/digest.py --min-time 0.2 --repeat 5
    python/.venv/bin/python python/benchmarks/fix.py --iterations 2000
    ```
