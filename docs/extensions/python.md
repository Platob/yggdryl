# Python

A native view of the same values the Rust core holds, with the protocols Python code expects.

```python
from yggdryl import DataType, Field, Url

# Every argument accepts the obvious Python spelling of itself.
schema = Field("row", DataType.from_fields([Field("id", "int64", nullable=False)]), nullable=False)
location = Url.from_path("C:/market data/trades.arrows")

assert schema.data_type.id == "struct"
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
| `Field`, `fields` | [field](../field.md) |
| `Uri`, `Url`, `Urn` | [uri](../uri.md) |
| `IOBase` | [io](../io.md) |
| `RecordOptions` | [io](../io.md), [ipc](../ipc.md), [parquet](../parquet.md) |
| `schema_from_pattern` | [io](../io.md) |
| `iceberg` | [iceberg](../iceberg.md) |
| `MimeType`, `MediaType`, `Timezone` | [enums](../enums.md) |
| `json`, `toml`, `yaml` | [text](../text.md) and the format pages |
| `gzip`, `zlib`, `zstd` | [gzip](../gzip.md), [zlib](../zlib.md), [zstd](../zstd.md) |
| `Record`, `record`, `from_dict`, `to_dict`, `schema_field`, `schema_fields` | this page |

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
assert str(Field("id", "int64", nullable=False).data_type) == "int64"
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

## What a Python value becomes

One pair of Rust functions converts in both directions, so `dumps` and `loads` cannot disagree about
what a value is. Nine Python types have a native value of their own and cross unchanged.

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
assert restored["payload"] == value["payload"]
# A temporal travels as its classic ISO string, the loosely typed deal a
# schemaless wire makes; a record class or a schema recovers the typed
# reading. The zone survives as the zone name, not as the offset it
# happened to be at.
assert restored["on"] == "2026-08-15"
assert restored["took"] == "PT90.000000S"
assert restored["at"] == "2026-08-15T12:30:00.000000+02:00[Europe/Paris]"
```

| Python | Native value | Notes |
| --- | --- | --- |
| `None`, `bool`, `int`, `float`, `str`, `bytes` | `Null`, `Bool`, integer, `Float`, `String`, `Bytes` | an `int` up to 128 bits keeps its width |
| `decimal.Decimal` | `Decimal` | coefficient and scale, never a float |
| `datetime.date` | `Date` | days since the epoch |
| `datetime.time` | `Time` | microseconds since midnight |
| `datetime.datetime` | `Timestamp` | microseconds since the epoch, UTC, plus the zone |
| `datetime.timedelta` | `Duration` | elapsed microseconds |
| `list`, `tuple` | `Sequence` | |
| `dict` | `Mapping` | keys are values too, not only strings |

The temporal spellings and the `decimal` envelope are the shared cross-language vocabulary, so a
document written here reads back the same way in Rust and JavaScript - never as a Python-shaped
wrapper.

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
| a named tuple | mapping of its members | the class |
| an `enum.Enum` member | its value | the class |
| a dataclass or record | mapping of its fields | the class |
| any other object | mapping of its `__dict__` | the class |
| an `int` wider than 128 bits | its decimal text | that it was a number |
| `datetime.fold` | nothing | which reading of a repeated hour a *naive* value was |
| a `tzinfo` on a `datetime.time` | nothing | the zone; a time of day has no zone field |

A `fold` on an *aware* datetime does survive, because the offset it selects is baked into the
UTC-relative count the value carries.

Two losses are refusals rather than silent damage: a decimal whose coefficient needs more than 128
bits or whose exponent has no scale in `-128..=127` raises `OverflowError`, and a temporal finer than
a microsecond - which `datetime` cannot hold - raises `ValueError` instead of truncating.

## Reading a class back

Nothing in a document names a Python class, so the class comes from the call. `cls=` converts the
decoded mapping through the same safe caster `from_dict` uses, which is the only path that validates
annotations and never imports a module named by untrusted input.

```python
from yggdryl import json, record

@record
class Trade:
    trade_id: int
    symbol: str

encoded = Trade(1, "AAPL").into_json()

# Without a target the document is what it says it is: data.
assert json.loads(encoded) == {"trade_id": 1, "symbol": "AAPL"}
assert json.loads(encoded, cls=Trade) == Trade(1, "AAPL")
assert Trade.from_json(encoded) == Trade(1, "AAPL")
```

A record used as a dictionary *key* is the one shape that reads back asymmetrically: JSON and YAML
have no unhashable keys, so it decodes as the tuple of its entries, and an annotation naming the
record type reads that tuple back as the record.

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

Typed identifiers and typed HTTP values (`parquet_field_id`, `alias`, `content_type`, `etag`, and
the rest) are attributes rather than map keys, because they are validated.

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
`https` attribute, because HTTPS shares the canonical `http:` namespace.

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
assert schema.data_type["year"].is_partition
assert len(schema.without_partition_fields().data_type) == 1
```

## Records

The Python-only records layer compiles class annotations into a native schema field and converts
instances through the core.

```python
from yggdryl import record, to_dict

@record
class Trade:
    trade_id: int
    symbol: str

trade = Trade(trade_id=1, symbol="AAPL")

assert to_dict(trade) == {"trade_id": 1, "symbol": "AAPL"}
assert Trade.schema_field().name == "Trade"
assert [field.name for field in Trade.schema_fields()] == ["trade_id", "symbol"]
```

Annotation inference, safe conversion, and Arrow materialization all delegate to the core: the
Python layer decides *which* core call to make, never how the conversion works.

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

# Random access needs no mode.
handle.pwrite(0, b"MSFT")
assert handle.pread(0, 4) == b"MSFT"

# Children resolve the way they do for a Path.
lake = IOBase(root / "lake" / "year=2024")
lake.mkdir()
(lake / "part-0.arrows").touch()
assert [entry.name for entry in lake.iterdir()] == ["part-0.arrows"]
assert len(list(IOBase(root / "lake").rglob("*.arrows"))) == 1
```

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

## Records cross as PyArrow readers

The same handle reads and writes records. A read returns a `pyarrow.RecordBatchReader` and a write
takes anything PyArrow exports an Arrow C stream from, so batches cross without a copy in either
direction and a resource larger than memory is never materialized to move it.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase

schema = pa.schema([
    pa.field("id", pa.int64(), nullable=False),
    pa.field("venue", pa.string(), nullable=False),
])
batch = pa.record_batch({"id": [1, 2], "venue": ["XNAS", "XNYS"]}, schema=schema)

root = pathlib.Path(tempfile.mkdtemp())

# The handle's name picks the encoding; no call takes a format argument.
for name in ("trades.arrows", "trades.parquet"):
    with IOBase(root / name) as handle:
        handle.write_arrow_batch_reader(batch)
        assert handle.read_arrow_batch_reader().read_all() == pa.Table.from_batches([batch])

# A schema on the options selects and casts in one pass: the columns it leaves
# out are skipped rather than read and discarded.
handle = IOBase(root / "trades.parquet")
options = handle.record_options()
options.schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
assert handle.read_arrow_batch_reader(options=options).schema.names == ["id"]
```

`record_options()` is the settings value for whichever encoding the media type names, and it is what
carries a Parquet row-group size or page compression. `with` is the scoped pair: leaving the block
publishes the resource at its exact length, which is what another reader needs to find a footer.

## Anything in, a reader out

`read_arrow`, `write_arrow`, and `append_arrow` are the same three methods with the argument widened
to whatever your last library handed you. Each one turns what it was given into one
`RecordBatchReader` and then calls the core method above it, so the widening is inference and never
a second way to write.

```python
import pathlib
import tempfile

import pyarrow as pa
import pyarrow.dataset as pads

from yggdryl import IOBase

schema = pa.schema([pa.field("id", pa.int64(), nullable=False), pa.field("venue", pa.string())])
table = pa.table({"id": [1, 2], "venue": ["XNAS", None]}, schema=schema)
root = pathlib.Path(tempfile.mkdtemp())

# A table, a dataset, a generator of tables, and plain rows all write.
for name, rows in (
    ("table.parquet", table),
    ("dataset.parquet", pads.dataset(table)),
    ("generated.parquet", (chunk for chunk in table.to_batches())),
    ("rows.parquet", [{"id": 1, "venue": "XNAS"}, {"id": 2, "venue": None}]),
):
    handle = IOBase(root / name)
    handle.write_arrow(rows)
    assert handle.read_arrow().read_all().num_rows == 2
```

| You are holding | What happens |
| --- | --- |
| `RecordBatchReader`, `Table`, `RecordBatch`, any `__arrow_c_stream__` exporter | the Arrow C stream, uncopied |
| `pyarrow.dataset.Dataset` or `Scanner` | its own reader, so the scan stays a scan |
| a `pandas` or `polars` frame | the frame's own Arrow export |
| a list or generator of any of those | chained, one item held at a time |
| an iterable of mappings | grouped into batches, typed by the declared schema or by the first batch |
| an iterable of sequences | the same, once a declared schema names the columns |

Nothing that could stream is collected. A generator is pulled one item at a time and each item is
dropped before the next is asked for, so a sequence of tables larger than memory writes exactly as a
reader would - and `write_arrow(rows)` on an unbounded generator of dictionaries groups them into
batches of `options.batch_size` rather than building a list first.

A Yggdryl record collection is the one row shape that does *not* belong here: `Record.from_dicts` and
`Record.into_arrow_record_batch_reader` already own that conversion with its cached schema, and the
reader they return is what `write_arrow` takes.

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

handle.write_pandas_frame(pd.DataFrame({"id": [1, 2], "venue": ["XNAS", "XNYS"]}))

# The plural name streams: one frame per batch, converted when it is pulled.
assert sum(len(frame) for frame in handle.read_pandas()) == 2
# The `_frame` name is the whole thing in one frame.
assert list(handle.read_pandas_frame()["venue"]) == ["XNAS", "XNYS"]
```

The eight names come in pairs, and the suffix is the whole difference:

| Streaming | One frame | What it does |
| --- | --- | --- |
| `read_pandas()` / `read_polars()` | `read_pandas_frame()` / `read_polars_frame()` | read one frame per batch, or every row in one |
| `write_pandas(frames)` / `write_polars(frames)` | `write_pandas_frame(frame)` / `write_polars_frame(frame)` | write a frame or an iterable of them, or exactly one |

The named entry points are strict: `write_pandas` handed a polars frame is a mistake worth naming,
and `write_arrow` already accepts both. A `polars.LazyFrame` is accepted and collected, because
polars offers no way to hand its rows over a batch at a time - that is polars' boundary, not this
one's.

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
assert catalog.list_tables("nyc") == ["nyc.trades"]
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

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Python](../notebooks/extensions_python-python.ipynb){ download }.

<!-- /notebooks -->
