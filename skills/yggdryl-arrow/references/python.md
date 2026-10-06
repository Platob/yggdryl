# yggdryl-arrow in Python

`from yggdryl import Serie, ChunkedSerie, StreamChunkedSerie, ArrowCastPlan, Field`.
Every door crosses the Arrow C Data Interface and shares buffers with
pyarrow; cast options are the keywords `safe=True` and
`representation="value"`.

## Build a column from Python values

`Serie.from_scalars` sends every row through the field's value contract once
and lays the buffers out once; `from_default` repeats the field's canonical
default and `lit` any value, each as a constant column holding the one row.

```python
from yggdryl import Field, Serie

price = Field("price", "int64", nullable=False)
serie = Serie.from_scalars(price, [125, 126, 127])
assert len(serie) == 3 and serie.field == price
assert serie.as_py() == [125, 126, 127]

# A required field has no room for None: the whole column is refused by path.
try:
    Serie.from_scalars(price, [None])
except ValueError as error:
    assert "$.price" in str(error), error
else:
    raise AssertionError("a required field refuses a null")

# Defaults: a required field repeats its present default, a nullable one null.
assert Serie.from_default(price, 2).as_py() == [0, 0]
assert Serie.from_default(Field("symbol", "utf8"), 2).as_py() == [None, None]
assert len(Serie.empty(price)) == 0

# A constant: one value held once, laid out only when something exports it.
constant = Serie.lit(price, 125, 1_000_000)
assert constant.is_lit and constant[999_999].as_py() == 125
assert constant.resident_size() < 1_024
```

## Land a pyarrow array, sharing its buffers

`Serie.from_arrow_array(array, field)` compiles one plan from the array's
layout to the field. An exact layout is the identity: the same buffers, no
row read. Any other layout is cast.

```python
import pyarrow as pa
from yggdryl import Field, Serie

source = pa.array([125, 126, 127], pa.int64())
field = Field("price", "int64", nullable=False)

held = Serie.from_arrow_array(source, field)
out = held.into_arrow_array()
assert out.buffers()[1].address == source.buffers()[1].address  # zero copy

# No field: the column of its own layout, named `item`.
assert Serie.from_arrow_array(pa.array([1, None])).field == Field("item", "int64")

# Another layout is cast once, on the way in.
wide = Serie.from_arrow_array(source, Field("price", "float64", nullable=False))
assert wide.into_arrow_array().type == pa.float64()
assert Serie.from_arrow_array(pa.array(["12", "1234"]), field).as_py() == [12, 1234]
```

## Land a record batch or table as a record column

A batch is a record column under a non-null struct root: children are matched
by name (ASCII case-insensitive), returned in the root's order, extra columns
dropped, missing nullable ones all-null.

```python
import pyarrow as pa
from yggdryl import Field, Serie

root = Field("trade", "struct<id: int64 not null, symbol: utf8, venue: utf8>", nullable=False)
batch = pa.record_batch({
    "SYMBOL": pa.array(["ACME"]),
    "id": pa.array([7], pa.int32()),
    "extra": pa.array([1.5]),
})

trades = Serie.from_arrow_batch(batch, root)
assert trades.names == ["id", "symbol", "venue"]
assert trades.as_py() == [{"id": 7, "symbol": "ACME", "venue": None}]
assert trades.into_arrow_batch().schema.names == ["id", "symbol", "venue"]

# With no root the batch is the record `row` of its own schema, columns shared.
assert Serie.from_arrow_batch(batch).field.name == "row"

# A pyarrow Table (or any stream) drains into one column.
table = pa.table({"id": [1, 2], "symbol": ["AAPL", "MSFT"], "venue": ["X", "Y"]})
assert len(Serie.from_arrow_reader(table, root)) == 2
```

## Take pandas, polars, NumPy or any Arrow exporter in

`Serie.from_(value, field=None)` is the one ladder over every columnar
runtime; `ChunkedSerie.from_` and `StreamChunkedSerie.from_` read the same ladder
as chunks and as a stream. A declared field is applied by the core's cast.

```python
import numpy as np
import pandas as pd
import polars as pl
import pyarrow as pa
from yggdryl import ChunkedSerie, Serie, StreamChunkedSerie

frame = pd.DataFrame({"symbol": ["AAPL", "MSFT"], "size": [100, 250]})
assert Serie.from_(frame).as_py() == [
    {"symbol": "AAPL", "size": 100},
    {"symbol": "MSFT", "size": 250},
]
assert Serie.from_(pl.DataFrame({"size": [1, 2]})).child("size").as_py() == [1, 2]
assert Serie.from_(np.array([1.5, 2.5])).as_py() == [1.5, 2.5]
assert Serie.from_(pa.scalar(7, pa.int64())).as_py() == [7]

# The declared field casts; a field expression is a field.
prices = Serie.from_(pa.array([1, 2, 3]), "price: float64 not null")
assert prices.into_arrow_array().type == pa.float64()

# Chunks: Serie.from_ joins them (a copy), ChunkedSerie.from_ keeps them apart.
chunked = pa.chunked_array([[1, 2], [3]])
assert len(Serie.from_(chunked)) == 3
assert ChunkedSerie.from_(chunked).num_chunks == 2

# A frame as a stream, not drained until pulled.
assert [len(s) for s in StreamChunkedSerie.from_(frame)] == [2]

# Out: the PyCapsule interface and the runtime converters, buffers shared.
record = Serie.from_(pa.table({"size": [100, 250]}))
assert pa.table(record).column("size").to_pylist() == [100, 250]
assert record.into_pandas()["size"].tolist() == [100, 250]
assert record.into_polars()["size"].to_list() == [100, 250]
```

## Stream a large reader under one plan

`StreamChunkedSerie.from_arrow_reader(reader, root)` compiles one plan from the
stream's schema before a batch is pulled and holds at most one source batch.
`into_arrow_reader()` is the transport face: batches cast as they are read,
never landed.

```python
import pyarrow as pa
from yggdryl import Field, StreamChunkedSerie

root = Field("row", "struct<id: int64, symbol: utf8 not null>", nullable=False)
table = pa.table({"id": pa.array([1, 2, 3], pa.int32()), "symbol": ["A", "B", None]})

series = StreamChunkedSerie.from_arrow_reader(table.to_reader(max_chunksize=1), root)
assert series.field == root
assert next(series).as_py() == [{"id": 1, "symbol": "A"}]
assert next(series).child("id").as_py() == [2]

# The third batch holds a null in a required column: refused at its pull.
try:
    next(series)
except ValueError as error:
    assert "$.symbol" in str(error), error
else:
    raise AssertionError("a null in a required column is refused")

# Transport: a pyarrow reader that casts as it is read, handed over once.
good = pa.table({"id": pa.array([1, 2], pa.int32()), "symbol": ["A", "B"]})
reader = StreamChunkedSerie.from_arrow_reader(good, root).into_arrow_reader()
assert isinstance(reader, pa.RecordBatchReader)
assert reader.read_all().schema.field("id").type == pa.int64()
```

## Compile a cast once and apply it to many batches

`ArrowCastPlan(source, target)` does every schema-dependent decision once; a
`pyarrow.Schema` source is the record `row`. `apply` takes a `Serie`, a
`RecordBatch`, an array, a `ChunkedSerie`, a `ChunkedArray` or a `Table`.

```python
import pyarrow as pa
from yggdryl import ArrowCastPlan, ChunkedSerie, Field

schema = pa.schema([pa.field("id", pa.int32(), nullable=False)])
root = Field("row", "struct<id: int64 not null>", nullable=False)

plan = ArrowCastPlan(schema, root)
plan.preflight()  # the whole recursion over no rows
assert plan.target == root and not plan.is_identity
assert (plan.safe, plan.representation) == (True, "value")

for offset in range(3):
    batch = pa.record_batch([pa.array([offset], pa.int32())], schema=schema)
    assert plan.apply(batch).child("id").as_py() == [offset]

# A table is its batches: one plan, every chunk, kept apart.
table = pa.Table.from_batches([pa.record_batch([pa.array([1, 2], pa.int32())], schema=schema)] * 2)
cast = plan.apply(table)
assert isinstance(cast, ChunkedSerie) and cast.num_chunks == 2

# An identity plan hands back the same buffers.
same = ArrowCastPlan(root, root)
assert same.is_identity

# A missing required column is refused when the plan is compiled.
try:
    ArrowCastPlan(pa.schema([pa.field("other", pa.int32())]), root)
except ValueError as error:
    assert "$.id" in str(error), error
else:
    raise AssertionError("a missing required column is refused at compile time")
```

## Cast a column in hand

`serie.cast(target)` is one plan for one column; a column already under the
target is itself. A `DataType` target is the required field named `value`.

```python
from yggdryl import DataType, Field, Serie

ids = Serie.from_scalars(Field("id", "int32"), [1, None])

assert ids.cast(Field("id", "int64")).as_py() == [1, None]
assert ids.cast(ids.field) == ids

try:
    ids.cast(DataType("int64"))
except ValueError as error:
    assert str(error) == "required Arrow field $.value holds 1 null values"
else:
    raise AssertionError("a DataType target is required")

# A schema-free run has no buffers to cast: type its rows instead.
try:
    Serie([1, 2]).cast(Field("id", "int64"))
except ValueError as error:
    assert "run" in str(error), error
else:
    raise AssertionError("a run is refused")
assert Serie.from_scalars(Field("id", "int64"), Serie([1, 2]).rows()).as_py() == [1, 2]
```

## Decide what a bad or missing value becomes

Nullability of the target field decides absence; `safe` decides only whether
a present value that fails to convert becomes null, and only where the column
may hold null. An empty text cell is null before `safe` is asked, except
in an interval column, which parses `""` and fails like any bad value.

```python
import pyarrow as pa
from yggdryl import Field, Serie

broken = pa.array(["1", "not a number", ""])
nullable = Field("n", "int64")
required = Field("n", "int64", nullable=False)

# Nullable + safe (default): the failure and the empty cell are null.
assert Serie.from_arrow_array(broken, nullable).as_py() == [1, None, None]

# Nullable + safe=False: the failed conversion is refused by value.
try:
    Serie.from_arrow_array(broken, nullable, safe=False)
except ValueError as error:
    assert "not a number" in str(error), error
else:
    raise AssertionError("safe=False refuses the value")

# Required: refused whatever safe says; never filled with a default.
for safe in (True, False):
    try:
        Serie.from_arrow_array(broken, required, safe=safe)
    except ValueError as error:
        assert "not a number" in str(error), error
    else:
        raise AssertionError("a required column refuses the value")

# An empty cell into a required column is a null, refused by path.
try:
    Serie.from_arrow_array(pa.array([""]), required, safe=False)
except ValueError as error:
    assert str(error) == "required Arrow field $.n holds 1 null values"
else:
    raise AssertionError("an empty cell is a null")
```

## Reinterpret bits between same-width types

`representation="bits"` shares the value buffer between two layouts of one
byte width. Other pairs convert as usual.

```python
import pyarrow as pa
from yggdryl import Field, Serie

source = pa.array([0, 2**64 - 1], type=pa.uint64())
signed = Serie.from_arrow_array(source, Field("digest", "int64"), representation="bits")
assert signed.as_py() == [0, -1]

raw = Serie.from_arrow_array(source, Field("digest", "fixed_size_binary(8)"), representation="bits")
assert raw.as_py()[1] == b"\xff" * 8
assert raw.cast(Field("digest", "uint64"), representation="bits").into_arrow_array().equals(source)

# Four bytes are not eight: an ordinary widening.
widened = Serie.from_arrow_array(pa.array([7], pa.uint32()), Field("id", "int64"), representation="bits")
assert widened.as_py() == [7]
```

## Read values fast

Python binds no typed leaf accessors (`as_int64` is Rust only). Do not loop
`scalar(i)`: hand the column to pyarrow once, zero copy, and compute there.
`into_numpy` copies (a null becomes `nan`).

```python
import pyarrow as pa
import pyarrow.compute as pc
from yggdryl import Field, Serie

source = pa.array([125, None, 127], pa.int64())
prices = Serie.from_arrow_array(source, Field("price", "int64"))

array = prices.into_arrow_array()  # the same buffers
assert array.buffers()[1].address == source.buffers()[1].address
assert pc.sum(array).as_py() == 252
assert pa.array(prices).null_count == 1  # PyCapsule, also zero copy

values = prices.into_numpy()  # a copy: NumPy has no null mask
assert values[0] == 125 and values[1] != values[1]
```

## Read and write rows, children and slices

Every write is spelled over `splice`, proves its rows through the field
first and leaves the column unchanged on refusal. Nested cells are written by
path; no child is handed out mutably.

```python
from yggdryl import Field, Serie

prices = Serie.from_scalars(Field("price", "int64"), [1])
prices.push(2)
prices.extend([3, None])
prices.set(0, 10)
prices.insert(1, 5)
assert prices.remove(1).as_py() == 5
assert prices.pop().as_py() is None
prices.splice(0, 1, [7, 8])
assert prices.as_py() == [7, 8, 2, 3]
assert prices[1].as_py() == 8 and prices.get(99) is None
assert prices[1:3].as_py() == [8, 2]  # a zero-copy window
prices.truncate(2)
assert prices.as_py() == [7, 8]

root = Field("row", "struct<id: int64 not null, tags: serie<item: utf8 not null>>", nullable=False)
rows = Serie.from_scalars(root, [[1, ["a", "b"]], [2, None]])
rows.set_cell("id", 1, 20)
assert rows.child("id").as_py() == [1, 20]
assert rows.get_child_by_path("tags.item").as_py() == ["a", "b"]
assert rows.child("tags").offsets == [0, 2, 2]

try:
    rows.push([3, "not a list"])
except ValueError:
    pass
else:
    raise AssertionError("a row the field refuses is refused")
assert len(rows) == 2  # unchanged
```

## Sort, deduplicate, group and window

Every leaf answers every verb in one order - `Scalar`'s total order, absent
values last unless `nulls_first=True` (the plan's `order by` and DuckDB's
default, the opposite of Arrow's) - so a column and the run of its rows sort,
deduplicate and group alike. `into_*` answers a new serie under the same
field; `as_*` brings the serie into that state in place and chains
(`copy.copy` first to keep the original). `indices`, `mask` and `keys` are a
`Serie`, any columnar object or an iterable of values. A `ChunkedSerie`
answers the same: `is_sorted`, `into_reversed`, `into_filtered` and
`partition_by` chunk by chunk, the sorts and `into_unique` by merging the
chunks each sorted on its own, the rest through one join.

```python
import copy

import pyarrow as pa

from yggdryl import ChunkedSerie, Field, Serie

prices = Serie.from_arrow_array(
    pa.array([3, None, 1, 3], pa.int64()), Field("price", "int64", nullable=True)
)

# The order as positions: stable, absences last unless told otherwise.
assert prices.sort_indices().as_py() == [2, 0, 3, 1]
assert prices.sort_indices(descending=True, nulls_first=True).as_py() == [1, 0, 3, 2]

# The reads answer a new serie; this one is as it was.
assert prices.into_sorted().as_py() == [1, 3, 3, None]
assert prices.into_sorted().is_sorted()
assert (prices.is_unique(), prices.unique_count()) == (False, 3)
assert len(prices.into_unique()) == 3
assert prices.into_taken([2, 0]).as_py() == [1, 3]
assert len(prices.into_filtered([True, False, False, True])) == 2

# The writes chain in place; a primitive column holding its buffer alone sorts where it stands.
held = copy.copy(prices)
assert held.as_sorted().as_unique().as_reversed() is held
assert held.as_py() == [None, 3, 1]

# One KeySerie per distinct typed key, in first-occurrence order.
groups = prices.partition_by(Serie.from_scalars(Field("venue", "utf8", nullable=False), ["XNAS", "XNYS", "XNAS", "XNYS"]))
assert (len(groups), groups[0].key.as_py()) == (2, ["XNAS"])
assert groups[0].rows.child("price").as_py() == [3, 1]

# A window reads and writes a stretch where it stands, window-relative.
column = Serie.from_scalars(Field("price", "int64", nullable=False), [9, 3, 1, 2, 0])
assert column.window(1, 3).into_sorted().as_py() == [1, 2, 3]
column.window(1, 3).as_sorted()
assert column.as_py() == [9, 1, 2, 3, 0]
assert column.memory_size() > 0

# Chunks: the edge between two sorted chunks is read with no join.
chunked = ChunkedSerie.from_arrow_chunked_array(pa.chunked_array([[3, 1], [2, 3]], pa.int64()))
assert not chunked.is_sorted()
assert chunked.into_sorted().num_chunks == 1
assert chunked.into_reversed().num_chunks == 2
```

## Sort by keys, read the declared order

```python
import copy

from yggdryl import Field, Serie

root = Field("quote", "struct<venue: utf8 not null, price: int64 not null>", nullable=False)
quotes = Serie.from_scalars(root, [["XNYS", 1], ["XNAS", 2], ["XNYS", 3]])

sorted_quotes = quotes.into_sort_by("venue, price desc")
assert sorted_quotes.child("price").as_py() == [2, 3, 1]
assert sorted_quotes.declared_order() == ["venue", "price desc"]
# Answered off the declaration: the identity. Kept by a slice, cleared by a breaking write.
assert sorted_quotes.sort_indices_by("venue").as_py() == [0, 1, 2]
assert sorted_quotes.slice(1, 2).declared_order() == ["venue", "price desc"]
held = copy.copy(sorted_quotes)
held.push(["AAAA", 0])
assert held.declared_order() is None
```

## Spill a column to disk

```python
import copy

from yggdryl import Field, Serie, SpillOptions

prices = Serie.from_scalars(Field("price", "int64", nullable=False), range(1_024))
spilled = copy.copy(prices)
spilled.spill(SpillOptions(byte_size=0))
assert spilled.is_spilled() and spilled.resident_size() == 0
assert spilled[7].as_py() == 7  # read exactly as resident
spilled.push(1_024)  # a write brings the leaf back
assert not spilled.is_spilled()

# One-liners: a spilled copy, or the spill chained in place.
assert prices.into_spilled(byte_size=0).is_spilled() and not prices.is_spilled()
assert spilled.as_spilled(byte_size=0) is spilled and spilled.is_spilled()
```

## Join two record columns

```python
from yggdryl import Field, Serie, StreamChunkedSerie

trades = Serie.from_scalars(
    Field("trade", "struct<id: int64 not null, size: int64 not null>", nullable=False),
    [[1, 10], [2, 20], [3, 30]],
)
venues = Serie.from_scalars(
    Field("venue", "struct<id: int64 not null, venue: utf8 not null>", nullable=False),
    [[1, "XNAS"], [2, "XNYS"]],
)
inner = trades.join_with(venues, "id", "inner", build="right")  # `using`: id once
assert inner.child("venue").as_py() == ["XNAS", "XNYS"]
left = trades.join_with(venues, "id", "left", build="right")
assert left.child("venue").as_py() == ["XNAS", "XNYS", None]
assert len(trades.join_with(venues, "id", "anti", build="right")) == 1
# A stream probes lazily against the held side.
streamed = StreamChunkedSerie.from_serie(trades).join_with(venues, "id", "inner", build="right")
assert sum(len(batch) for batch in streamed) == 2
```

## Cut rows into windows by key

`window_by` and `windowBy` compute the key once and return `KeySeries`.
Each `KeySerie` exposes `key`, `rows`, and an optional absolute `rownum`.
Sorted windows regroup equal keys while retaining native payload pieces.
A stream returns `StreamKeySerie`; read each borrowed payload before advancing.

```python
from yggdryl import ChunkedSerie, Field, Serie, StreamChunkedSerie

root = Field("fill", "struct<venue: utf8 not null, qty: int64 not null>", nullable=False)
fills = Serie.from_scalars(root, [["XNAS", 5], ["XNYS", 2], ["XNAS", 3]])

# Each key once, in key order; the record names the window's key.
totals = {}
for group in fills.window_by("venue", sorted=True):
    totals[group.key[0].as_py()] = sum(row["qty"] for row in group.rows.as_py())
assert totals == {"XNAS": 8, "XNYS": 2}

# Across chunks: zero-copy pieces, no join, no record.
chunked = ChunkedSerie.from_series([fills.slice(0, 2), fills.slice(2, 1)], root)
xnas = chunked.window_by("venue", sorted=True)[0].rows
assert (len(xnas), len(list(xnas.into_chunked_stream(row_size=1)))) == (2, 2)

# A stream: one lazy reader per window - read each before taking the next.
places = []
for group in StreamChunkedSerie.from_serie(fills).window_by("venue"):
    places.append((group.rownum, sum(len(piece) for piece in group.rows.into_chunked_stream())))
assert places == [(0, 1), (1, 1), (2, 1)]
```

## Cut a stream into partitions

Stream partitioning yields `KeySerie` values as partitions close.
The key stays explicit and the payload remains native; `clustered` closes a
partition when the next key arrives. `max_open` bounds active partitions.

```python
from yggdryl import ChunkedSerie, Field, Serie, StreamChunkedSerie

root = Field("fill", "struct<venue: utf8 not null, qty: int64 not null>", nullable=False)
batches = [[["XLON", 1], ["XNAS", 2]], [["XNAS", 3], ["XNYS", 4]]]
stream = StreamChunkedSerie.from_chunked(
    ChunkedSerie.from_series([Serie.from_scalars(root, rows) for rows in batches], root)
)

# Sorted on the venue: each partition is handed over as soon as the next one opens.
sizes = [(group.key.as_py(), len(group.rows)) for group in stream.partition_by("venue", clustered=True)]
assert sizes == [(["XLON"], 1), (["XNAS"], 2), (["XNYS"], 1)]
```

## Keep chunks and batches apart

`ChunkedSerie` holds a chunked array or a table without concatenating;
`into_serie` is the one join. A declared field casts every chunk by one plan.

```python
import pyarrow as pa
from yggdryl import ChunkedSerie, Field, Serie

source = pa.chunked_array([[125, 126], [127]])
prices = ChunkedSerie.from_arrow_chunked_array(source, Field("price", "int64", nullable=False))
assert (len(prices), prices.num_chunks) == (3, 2)
assert prices.scalar(2).as_py() == 127
assert prices.slice(1, 2).num_chunks == 2

back = prices.into_arrow_chunked_array()
assert back.chunk(0).buffers()[1].address == source.chunk(0).buffers()[1].address

table = pa.Table.from_batches([
    pa.record_batch({"id": [1, 2], "symbol": ["AAPL", "MSFT"]}),
    pa.record_batch({"id": [3], "symbol": ["AMD"]}),
])
quotes = ChunkedSerie.from_(table)
assert quotes.num_chunks == 2
assert quotes.child("symbol").as_py() == ["AAPL", "MSFT", "AMD"]
assert quotes.into_arrow_table().equals(table)

wide = quotes.cast(Field("row", "struct<id: float64, symbol: utf8>", nullable=False))
assert wide.num_chunks == 2
joined = quotes.into_serie()  # the one concatenation
assert isinstance(joined, Serie) and joined == quotes
```

## Hand a held column on as a stream

`StreamChunkedSerie.from_serie` and `from_chunked` read held data as a stream with
no plan and no copy - what a record write takes. `reader.cast(root)` re-roots
the stream under one plan and consumes the reader.

```python
import pyarrow as pa
from yggdryl import ChunkedSerie, Field, Serie, StreamChunkedSerie

rows = Serie.from_scalars(Field("row", "struct<id: int64 not null>", nullable=False), [[1], [2]])
assert list(StreamChunkedSerie.from_serie(rows)) == [rows]

# A leaf column is the one child of a `row` record.
price = Serie.from_scalars(Field("price", "int64"), [1, 2])
assert next(StreamChunkedSerie.from_serie(price)).child("price").as_py() == [1, 2]

chunks = ChunkedSerie.from_(pa.chunked_array([[1, 2], [3]]))
assert [len(chunk) for chunk in StreamChunkedSerie.from_chunked(chunks)] == [2, 1]

wide = StreamChunkedSerie.from_serie(rows).cast(Field("row", "struct<id: float64 not null>", nullable=False))
assert [record.as_py() for record in wide] == [[{"id": 1.0}, {"id": 2.0}]]
```

## Hold a column as one value

`into_scalar()` makes a column one serie `Scalar` sharing its buffers, and
`as_serie()` borrows it back; neither reads a row. A stream is never a
`Scalar`: `Scalar.from_` drains one.

```python
import pyarrow as pa
from yggdryl import Field, Scalar, Serie

column = Serie.from_scalars(Field("price", "int64", nullable=False), [125, 126])
value = column.into_scalar()
assert value.kind == "serie" and value.as_py() == [125, 126]
assert value.as_serie() == column
assert Scalar.from_(pa.array([1, 2])).as_py() == [1, 2]

# Identity is the rows alone: not the field, not the width.
assert column == Serie([125, 126])
assert column == Serie.from_scalars(Field("size", "int32"), [125, 126])
```

## Convert schemas and types

A non-null struct `Field` is the schema; `Field.from_arrow_schema` reads a
pyarrow schema as the record `row`. Per type and per field, `from_arrow` /
`into_arrow` cross the C Data Interface.

```python
import pyarrow as pa
from yggdryl import DataType, Field

schema = pa.schema([pa.field("id", pa.int64(), nullable=False), pa.field("symbol", pa.string())])
root = Field.from_arrow_schema(schema)
assert root == Field("row", "struct<id: int64 not null, symbol: utf8>", nullable=False)
assert root.into_arrow_schema().equals(schema)

assert DataType.from_arrow(pa.int32()) == DataType("int32")
assert DataType("int64").into_arrow() == pa.int64()
assert Field.from_arrow(pa.field("x", pa.int32(), nullable=False)) == Field("x", "int32", nullable=False)
assert Field("price", "int64", nullable=False).into_arrow() == pa.field("price", pa.int64(), nullable=False)
```

## Merge two streams

`combined(left, right, schema=None)` chains two readers onto the root their
schemas merge into, lazily: shared columns by name, one-sided columns
nullable.

```python
import pyarrow as pa
from yggdryl import combined

left = pa.table({"id": [1]})
right = pa.table({"id": [2], "venue": ["XPAR"]})

joined = combined(left, right).read_all()
assert joined.column_names == ["id", "venue"]
assert joined.column("venue").to_pylist() == [None, "XPAR"]
```

## Gotchas in Python

- `Serie`, `ChunkedSerie` and `StreamChunkedSerie` are mutable or one-shot, so they
  are unhashable; compare by rows with `==`.
- `StreamChunkedSerie` crosses once: after `into_arrow_reader()`, `cast`, or being
  taken by `Serie.from_` / `StreamChunkedSerie.from_`, it raises `ValueError`.
- `Serie.from_` of a stream (a `Table`, a reader, a frame) drains it; use
  `StreamChunkedSerie.from_` to keep it lazy.
- A bare `str` passed as `field` is a field expression (`"price: int64 not
  null"`); a `Field` built with no `nullable` argument is nullable.
- `into_pandas()` / `into_polars()` of a leaf column answer a one-column frame
  named after the field.
- A NumPy array of more than one dimension is a `TypeError`.
- Malformed foreign buffers are a `ValueError` naming the slot, never a crash:
  every foreign array, batch and stream is validated before a row is read.
