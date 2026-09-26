# yggdryl-records in Python

`from yggdryl import IOBase, RecordOptions, TextOptions`; Iceberg is `from yggdryl.iceberg import Table`. Every record method takes keyword-only `options=` plus the option properties by name (`select=`, `filter=`, `field=`, `merge_by=`, `max_row_size=`, `commit_row_size=`, `compression=`, `rowheader=`, ...), each set on a copy.

## Which encoding will this handle use?

The suffix or media type decides; `record_options()` answers the settings for that encoding, refuses an encoding the build does not implement, and an absent resource reads as no rows.

```python
import pathlib
import tempfile

import pytest

from yggdryl import IOBase, RecordOptions

root = pathlib.Path(tempfile.mkdtemp())

# The media type names the encoding: no format argument anywhere.
assert str(RecordOptions("trades.parquet").mime_type) == "application/vnd.apache.parquet"
assert str(RecordOptions("trades.arrows").mime_type) == "application/vnd.apache.arrow.stream"
assert RecordOptions("trades.arrows").max_row_group_size is None  # a Parquet-only setting

# A handle's own options, and the class naming the medium it composed.
handle = IOBase(root / "trades.avro")
assert type(handle).__name__ == "Avro"
assert handle.record_options().block_codec == "deflate"

# Absent reads as empty; an unimplemented encoding is named, never guessed.
assert IOBase(root / "absent.arrows").read_arrow_reader().read_all().num_rows == 0
with pytest.raises(ValueError, match="text/csv"):
    IOBase(root / "trades.csv").record_options()
```

## Write batches and stream them back

`read_arrow_reader` answers a `pyarrow.RecordBatchReader` over the C Stream interface: iterate it, or hand it to the next writer; only the current batch is alive.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase

schema = pa.schema([pa.field("id", pa.int64(), nullable=False), pa.field("venue", pa.string())])
table = pa.table({"id": [1, 2, 3], "venue": ["XNAS", "XNYS", None]}, schema=schema)

source = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
source.overwrite_arrow_table(table)
assert source.read_arrow_field().name == "row"
assert (source.row_size, source.column_size) == (3, 2)

# Stream from one handle into another: nothing is collected on the way.
target = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")
target.overwrite_arrow_reader(source.read_arrow_reader())

rows = sum(batch.num_rows for batch in target.read_arrow_reader())
assert rows == 3
```

## Read only the columns and rows I need

Put `select`, `filter`, a declared `field` and `max_row_size` on the read: the medium projects and prunes, the field casts in the same pass, and the limit stops pulling.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase

handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")
handle.overwrite_arrow_table(
    pa.table({"id": list(range(10)), "venue": ["XNAS", "XNYS"] * 5, "px": [1.5] * 10})
)

# Properties by name, each set on a copy of the handle's options.
picked = handle.read_arrow_reader(select=["id"], filter="id > 3", max_row_size=2).read_all()
assert picked.column_names == ["id"]
assert picked.column("id").to_pylist() == [4, 5]

# A declared field projects and casts in one pass.
narrow = pa.schema([pa.field("id", pa.int32(), nullable=False)])
assert handle.read_arrow_reader(field=narrow).schema == narrow

# The same sections as one options object, reusable across calls.
options = handle.record_options()
options.plan = "select id, venue where venue = 'XNYS' limit 3"
assert handle.read_arrow_reader(options=options).read_all().column("id").to_pylist() == [1, 3, 5]
```

## Refuse a value the declared field cannot convert

A nullable declared column takes a value it cannot convert as null while `safe` holds, and `safe` is the default; `safe=False` (or a `not null` column) refuses it by path.

```python
import pathlib
import tempfile

import pyarrow as pa
import pytest

from yggdryl import IOBase

handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "raw.parquet")
handle.overwrite_arrow_table(pa.table({"v": ["1", "x"]}))
declared = "row: struct<v: int32> not null"

# Default: the unconvertible "x" becomes null without a word.
assert handle.read_arrow_reader(field=declared).read_all().to_pylist() == [{"v": 1}, {"v": None}]

# safe=False names the column and the value instead.
with pytest.raises(pa.ArrowInvalid, match=r"\$\.v"):
    handle.read_arrow_reader(field=declared, safe=False).read_all()
```

## Write and read plain rows or dataclasses

`*_records` takes mappings or `@scalar` dataclass instances; `read_records()` yields dicts, `read_records(Cls)` instances. The stream still batches underneath.

```python
import pathlib
import tempfile

from yggdryl import IOBase, scalar

@scalar(frozen=True)
class Trade:
    id: int
    venue: str

handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
handle.overwrite_records([Trade(1, "XNAS"), Trade(2, "XNYS")])
handle.append_records([Trade(3, "XLON")])

assert list(handle.read_records(Trade, filter="id >= 2")) == [Trade(2, "XNYS"), Trade(3, "XLON")]
assert next(handle.read_records()) == {"id": 1, "venue": "XNAS"}
```

## Append, and upsert by key

Overwrite replaces, append keeps the stored rows, merge updates rows whose `merge_by` key matches and appends the rest. Merge without a key is refused.

```python
import pathlib
import tempfile

import pyarrow as pa
import pytest

from yggdryl import IOBase

schema = pa.schema([pa.field("id", pa.int64(), nullable=False), pa.field("symbol", pa.string())])
rows = lambda ids, symbols: pa.record_batch({"id": ids, "symbol": symbols}, schema=schema)

handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
handle.overwrite_arrow_batch(rows([1, 2], ["AAPL", "MSFT"]))
handle.append_arrow_batch(rows([3], ["NVDA"]))
handle.merge_arrow_batch(rows([2, 9], ["MSFT.O", "AMD"]), merge_by=["id"])

stored = {row["id"]: row["symbol"] for row in handle.read_records()}
assert stored == {1: "AAPL", 2: "MSFT.O", 3: "NVDA", 9: "AMD"}

with pytest.raises(ValueError, match="merge_by"):
    handle.merge_arrow_batch(rows([1], ["X"]))
```

## Choose the write mode at run time

`write_arrow_reader|table|batch` and `write_records` take the mode as a string; `write_arrow`/`read_arrow` take and answer a `SerieReader` and are also the record door of JSON, JSON Lines, YAML, TOML and XML handles.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase, Serie, SerieReader

root = pathlib.Path(tempfile.mkdtemp())
handle = IOBase(root / "trades.arrows")
for mode in ("overwrite", "append"):
    handle.write_arrow_table(pa.table({"id": [1, 2]}), mode)
assert handle.row_size == 4

# A document handle takes rows through write_arrow, one document per row.
lines = IOBase(root / "quotes.jsonl")
lines.write_arrow(pa.table({"symbol": ["AAPL", "MSFT"], "size": [100, 200]}))
assert lines.read_bytes().count(b"\n") == 2
read = lines.read_arrow()
assert isinstance(read, SerieReader)
assert len(Serie.from_(read)) == 2

# A document is written whole: write_arrow on it takes "overwrite" only.
try:
    lines.write_arrow(pa.table({"symbol": ["NVDA"], "size": [300]}), "append")
    raise AssertionError("append to a document must be refused")
except ValueError as refused:
    assert "expected overwrite, got append" in str(refused)
```

## Bound memory on large writes

`commit_row_size=N` publishes every N rows (a committed prefix survives a later failure); unset commits once; `0` is refused before any input is pulled. `batch_row_size` bounds the batches a Parquet read yields.

```python
import pathlib
import tempfile

import pyarrow as pa
import pytest

from yggdryl import IOBase

root = pathlib.Path(tempfile.mkdtemp())
table = pa.table({"id": list(range(10))})

handle = IOBase(root / "trades.parquet")
handle.overwrite_arrow_table(table, commit_row_size=4)
assert handle.row_size == 10

sizes = [batch.num_rows for batch in handle.read_arrow_reader(batch_row_size=4)]
assert sum(sizes) == 10 and max(sizes) <= 4

with pytest.raises(ValueError, match="commit_row_size"):
    handle.overwrite_arrow_table(table, commit_row_size=0)
```

## Parquet: compression, pruning, footer answers

Pages compress inside the file (`compression`, default `zstd(1)`); a read never names it. A `filter` skips row groups the footer rules out. An outer `.gz`/`.zst` on the name is refused.

```python
import pathlib
import tempfile

import pyarrow as pa
import pytest

from yggdryl import IOBase

root = pathlib.Path(tempfile.mkdtemp())
table = pa.table({"id": list(range(1_000)), "symbol": ["AAPL"] * 1_000})

handle = IOBase(root / "trades.parquet")
handle.overwrite_arrow_table(table, compression="snappy", max_row_group_size=250)

# row_size and the statistics come from the footer, never from decoding rows.
assert handle.row_size == 1_000
assert len(handle.read_parquet_statistics()["row_groups"]) == 4
assert handle.read_arrow_reader(filter="id >= 900").read_all().num_rows == 100

with pytest.raises(ValueError, match="parquet compresses"):
    IOBase(root / "trades.parquet.gz").overwrite_arrow_table(table)
```

## Avro: a container file, or bytes with a reader schema

A `.avro` handle is a record medium (`block_codec`: `null`, `deflate`, `snappy`, `zstandard`). `yggdryl.avro` is the scalar codec over bytes; a reader schema resolves renames, promotions and defaults.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase, avro

handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.avro")
handle.overwrite_arrow_table(pa.table({"symbol": ["AAPL"], "qty": [100]}), block_codec="zstandard")
assert handle.read_arrow_reader(select=["qty"]).read_all().to_pydict() == {"qty": [100]}

writer = {"type": "record", "name": "trade", "fields": [
    {"name": "symbol", "type": "string"}, {"name": "qty", "type": "int"}]}
reader = avro.Schema({"type": "record", "name": "trade", "fields": [
    {"name": "quantity", "aliases": ["qty"], "type": "long"},
    {"name": "note", "type": "string", "default": "none"}]})
encoded = avro.dumps([{"symbol": "AAPL", "qty": 100}], writer)
assert avro.loads(encoded, reader_schema=reader).rows == [{"note": "none", "quantity": 100}]
```

## Read a log file as typed rows

A `.log`/`.txt` handle reads one record per line (or per framed chain with `framing`): the sixteen event columns, `body`, then one column per named `rowheader` capture, typed by `autotype` (on by default).

```python
import pathlib
import tempfile

from yggdryl import IOBase, TextOptions

with tempfile.TemporaryDirectory() as directory:
    source = pathlib.Path(directory) / "app.log"
    source.write_bytes(b"[INFO] id=7 first\r\n detail A\r[WARN] id=9 second\n detail B")

    options = TextOptions()
    options.start_rownum = 1
    options.rowheader = r"^\[(?<level>[A-Z]+)\] id=(?<id>\d+) "
    options.framing = True

    rows = list(IOBase(source).into_text(options).read_records())
    assert [row["id"] for row in rows] == [7, 9]
    assert [row["body"] for row in rows] == ["first\n detail A", "second\n detail B"]
    assert [row["seqnum"] for row in rows] == [1, 3]

    # One-off: the same property by name on the read, no TextOptions object.
    levels = IOBase(source).read_arrow_reader(rowheader=r"^\[(?<level>[A-Z]+)\] ")
    assert levels.schema.names[-2:] == ["body", "level"]
```

## Partitioned folders: route on write, prune on read

Addressing a folder writes each row to its `column=value` leaf (the path carries the partition columns, the leaf does not) and reads them back typed. A `filter` equality prunes leaves by path before anything is decoded.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase, RecordOptions

root = pathlib.Path(tempfile.mkdtemp())
(root / "year=2024" / "month=01").mkdir(parents=True)
schema = pa.schema([
    pa.field("price", pa.int64(), nullable=False),
    pa.field("year", pa.int32(), nullable=False),
    pa.field("month", pa.string(), nullable=False),
])
lake = IOBase(root)
options = RecordOptions("part.arrows")
options.field = schema
lake.overwrite_arrow_batch(
    pa.record_batch({"price": [10, 20], "year": [2024, 2024], "month": ["01", "01"]}, schema=schema),
    options=options,
)

leaf = lake / "year=2024" / "month=01" / "part-0.arrows"
assert len(leaf.read_arrow_field().dtype) == 1  # only `price` is stored
assert lake.read_arrow_reader(options=options).read_all().column_names == ["price", "year", "month"]

options.filter = "year = 2024 and month = '01'"
assert options.partition_pairs() == [("year", "2024"), ("month", "01")]
assert lake.read_arrow_reader(options=options).read_all().num_rows == 2
assert [child.partitions for child in lake.children_where({"year": "2024"})] == [
    (("year", "2024"), ("month", "01"))
]
```

## Derive a partition column from another column

`PARTITION:sources` and `PARTITION:transform` on the derived field; `apply_arrow_batch` on the root fills it where absent or all null and leaves values alone.

```python
import pyarrow as pa

from yggdryl import DataType, Field

year = Field("year", "int32", nullable=True)
year.partition.sources = ["event"]
year.partition.transform = "year"

root = Field("row", DataType.from_fields([Field("event", "date32", nullable=False), year]), nullable=False)
batch = pa.record_batch({"event": pa.array([19_723, 20_089], pa.date32())})

filled = root.partition.apply_arrow_batch(batch)
assert filled.column_names == ["event", "year"]
assert filled.column("year").to_pylist() == [2024, 2025]
```

## Iceberg: create, append, upsert, scan

A table is a folder reached through one `IOBase`; no catalog is required. Scans are `pyarrow.RecordBatchReader`s planned from metadata; `merge` keys are the identity partition columns plus `merge_by`.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase
from yggdryl.iceberg import Table

schema = pa.schema([
    pa.field("id", pa.int64(), nullable=False),
    pa.field("venue", pa.string()),
    pa.field("px", pa.float64()),
])
root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")
table = Table.create(root, schema, ["venue"])
assert table.current_snapshot is None

table.append(pa.table({"id": [1, 2, 3], "venue": ["XNAS", "XNYS", "XNAS"], "px": [1.0, 2.0, 3.0]}, schema=schema))
first = table.current_snapshot.snapshot_id
table.merge(pa.table({"id": [3, 4], "venue": ["XNAS", "XNAS"], "px": [30.0, 4.0]}, schema=schema), ["id"])

assert sorted(table.scan().read_all().column("id").to_pylist()) == [1, 2, 3, 4]
assert table.scan_matching("px > 2.5").read_all().column("px").to_pylist() == [30.0, 4.0]
assert table.plan_matching("venue = 'XNYS'")["manifests_skipped"] >= 0
assert table.scan_at(first).read_all().num_rows == 3  # time travel

# The folder is also an ordinary record handle: filter and select push down.
reopened = IOBase(root.url.into_path())
assert reopened.read_arrow_reader(select=["id"], filter="venue = 'XNYS'").read_all().num_rows == 1
```

## Evolve an Iceberg schema

`update_schema()` records a chain and `commit()` writes one new schema; field IDs are kept and never reused. `evolve_schema(field)` replaces the schema whole.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import Field, IOBase
from yggdryl.iceberg import Table

root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")
table = Table.create(root, pa.schema([pa.field("id", pa.int32(), nullable=False)]))
table.append(pa.table({"id": pa.array([1], pa.int32())}))

table.update_schema().add_column("", Field("note", "utf8")).update_type("id", "int64").commit()
assert [child.name for child in table.schema.dtype] == ["id", "note"]
assert table.scan().read_all().column("note").to_pylist() == [None]
```

## Write a pandas or polars frame to a file, and read one back

Python only. `overwrite_/append_/merge_/write_` + `pandas_frame` or
`polars_frame` take one frame, the `pandas` / `polars` forms an iterable of
frames; `read_pandas_frame()` / `read_polars_frame()` answer the whole file and
`read_pandas()` / `read_polars()` a lazy iterator of one frame per batch.
`overwrite_arrow_batch` refuses a DataFrame (`TypeError`).

```python
import pathlib
import tempfile

import pandas as pd
import polars as pl

from yggdryl import IOBase

with tempfile.TemporaryDirectory() as folder:
    handle = IOBase(pathlib.Path(folder) / "trades.parquet")
    handle.overwrite_pandas_frame(pd.DataFrame({"id": [1, 2], "px": [1.5, 2.5]}))
    handle.append_polars_frame(pl.DataFrame({"id": [3], "px": [3.5]}))
    handle.merge_pandas_frame(pd.DataFrame({"id": [3, 4], "px": [9.0, 4.5]}), merge_by=["id"])

    frame = handle.read_polars_frame()
    assert isinstance(frame, pl.DataFrame)
    assert sorted(frame["id"].to_list()) == [1, 2, 3, 4]
    assert handle.read_pandas_frame().set_index("id").loc[3, "px"] == 9.0
    assert sum(len(chunk) for chunk in handle.read_pandas()) == 4
```

## Hand the file to polars or a pyarrow dataset lazily

Python only. `scan_polars()` answers a `polars.LazyFrame`, `scan_arrow()` a `pyarrow.dataset.Scanner`; a local Parquet leaf scans natively, anything else streams through the native reader.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase

target = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")
target.overwrite_arrow_table(pa.table({"symbol": ["AAPL", "MSFT"], "price": [187.23, 402.11]}))

assert target.scan_polars().select("symbol").collect().height == 2
assert target.scan_arrow().to_table().num_rows == 2
```

## Run a SQL-like write or read plan

`Plan` spells `insert into`, `insert overwrite`, `upsert into ... by (...)`, `delete from ... where`, and reads with `select ... from ... where ... limit ... offset`; `execute()` pushes the read sections into the source. Grammar: `yggdryl-expressions`.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import Plan

url = (pathlib.Path(tempfile.mkdtemp()) / "trades.arrows").as_uri()
Plan(f"create '{url}' (id int64 not null, name utf8)").execute().read_all()
Plan(f"insert into '{url}'").apply_arrow_batch(
    pa.record_batch({"id": pa.array([1, 2, 3], pa.int64()), "name": ["a", "b", "c"]})
)

read = Plan(f"select name from '{url}' where id > 1 order by id desc limit 1 offset 1")
assert read.execute().read_all().column("name").to_pylist() == ["b"]
```

## Gotchas in Python

- Options are keyword-only: `read_arrow_reader(options=o)` or `read_arrow_reader(select=[...])`; a positional options argument is a `TypeError`.
- `RecordOptions` has no `offset`: a plan's `offset` assigned through `options.plan` is dropped. Use `Plan(...).apply_arrow_reader(reader)` or `Plan.execute()` for an offset.
- JSON, JSON Lines, YAML, TOML and XML handles are not `*_records`/`*_arrow_*` targets; use `write_arrow`/`read_arrow`, or the codecs in `yggdryl-documents`. `write_arrow` on them accepts `"overwrite"` only - a document is written whole.
- A declared nullable column reads a value it cannot convert as null under the default `safe`; pass `safe=False` to have it refused.
- A folder's partition columns come from the path: a leaf read alone does not carry them.
- `Table` objects cache their metadata; after writing through another handle, `Table.open(...)` again.
- `update_schema().commit()` returns `None` in Python (JavaScript returns the schema id).
