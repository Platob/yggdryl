# yggdryl-warehouse in Python

`from yggdryl.warehouse import ...` (every class is also on `yggdryl`).
`Catalog`, `Namespace` and `Table` are `IOBase` subclasses and are never built
directly: `type(object)` is the implementation - `MemoryCatalog`,
`FolderCatalog`, `MemoryNamespace`, `FolderNamespace`, `MediaTable`. A path is
dotted text or a sequence of parts; properties are a `dict[str, str]` and are
also taken as keywords (`region="eu-west-1"`), merged after the mapping,
every value spelled as text, `None` clearing and `...` not given.

## Register a folder and resolve a path

```python
import pathlib
import tempfile

from yggdryl.warehouse import FolderCatalog, MediaTable, Warehouse

root = pathlib.Path(tempfile.mkdtemp())
(root / "eu").mkdir()
(root / "eu" / "trades.csv").write_text("symbol,price\nAAPL,187.5\nMSFT,410.25\n")

warehouse = Warehouse()
warehouse.register(FolderCatalog("market", root))  # a handle, a Url, a str or a Path

trades = warehouse.table("market.eu.trades")
assert type(trades) is MediaTable
assert trades.path == ("market", "eu", "trades")
assert trades.storage == "text/csv"
assert trades.field().name == "trades"
# Every record verb is IOBase's.
assert trades.row_size() == 2
assert trades.read_arrow_reader().read_all().num_rows == 2
assert [row["symbol"] for row in trades.read_records()] == ["AAPL", "MSFT"]
```

## Register a table at any location

```python
import pathlib
import tempfile

from yggdryl.warehouse import MediaTable, MemoryCatalog, MemoryNamespace, Warehouse

root = pathlib.Path(tempfile.mkdtemp())
(root / "blob").write_text("symbol,price\nAAPL,187.5\n")

# A declared schema is renamed after the table; a stated `media_type` types
# an extensionless location.
table = MediaTable(
    "lake.raw.trades",
    root / "blob",
    dtype="struct<symbol: utf8, price: float64>",
    media_type="text/csv",
)
warehouse = Warehouse()
warehouse.register(table)
assert type(warehouse.catalog("lake")) is MemoryCatalog
assert type(warehouse.namespace("lake.raw")) is MemoryNamespace
resolved = warehouse.table("lake.raw.trades")
assert resolved.field().name == "trades"
assert resolved.properties == {"media_type": "text/csv"}
assert resolved.read_arrow_reader().read_all().num_rows == 1

# A taken name conflicts; `replace` swaps and answers what was there.
try:
    warehouse.register(MediaTable("lake.raw.trades", root / "other.csv"))
except ValueError as error:
    assert "got an existing table" in str(error)
assert str(warehouse.replace(MediaTable("lake.raw.trades", root / "other.csv"))) == "lake.raw.trades"
assert str(warehouse.unregister("lake.raw.trades")) == "lake.raw.trades"
```

## Walk one level with the mapping views

```python
import pathlib
import tempfile

from yggdryl.warehouse import FolderCatalog, FolderNamespace, MediaTable

root = pathlib.Path(tempfile.mkdtemp())
for leaf in ("trades.csv", "eu/fills.csv", "eu/lake/part-0.csv", "README.md"):
    (root / leaf).parent.mkdir(parents=True, exist_ok=True)
    (root / leaf).write_text("symbol,price\nAAPL,187.5\n")
market = FolderCatalog("market", root)

assert list(market.namespaces) == ["eu"]
assert list(market.tables) == ["trades"]  # the markdown is no table
assert "eu.fills" in market.tables and "asia" not in market.namespaces
eu = market.namespaces["eu"]
assert type(eu) is FolderNamespace
assert sorted(eu.tables) == ["fills", "lake"]
assert market.tables["eu.lake"].storage == "directory"  # a folder under a namespace is a table
assert [(name, type(child)) for name, child in eu.tables.items()] == [("fills", MediaTable), ("lake", MediaTable)]
assert market.tables.get("nowhere") is None
try:
    market.tables["nowhere"]
except KeyError as error:
    assert 'expected a table at "market.nowhere", got nothing' in str(error)
# `children()` yields every child, whichever kind, as its class.
assert sorted(type(child).__name__ for child in market.children()) == ["FolderNamespace", "MediaTable"]
```

## Write through a view

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl.warehouse import MediaTable, MemoryNamespace

root = pathlib.Path(tempfile.mkdtemp())
out = MemoryNamespace("lake.out", objects=[MediaTable("lake.out.rows", root / "rows.arrows")])

# Any shape the record surface takes, under the table's own options; the
# properties beside `options` are this write's record settings.
written = out.tables.append("rows", pa.table({"id": [1, 2]}))
assert str(written) == "lake.out.rows"
out.tables.append("rows", [{"id": 3}])
assert out.tables["rows"].row_size() == 3
assert out.tables.overwrite("rows", pa.table({"id": [7]})).row_size() == 1

# A memory level creates nothing: register the table first.
try:
    out.tables.append("absent", pa.table({"id": [1]}))
except ValueError as error:
    assert 'filesystem "MemoryNamespace" does not support creating a table' == str(error)
```

## State properties once

```python
import pathlib
import tempfile

from yggdryl.warehouse import MediaTable, MemoryCatalog, MemoryNamespace, Warehouse

root = pathlib.Path(tempfile.mkdtemp())
(root / "trades.csv").write_text("symbol,price\nAAPL,187.5\n")

trades = MediaTable("lake.eu.trades", root / "trades.csv", codec="gzip")
eu = MemoryNamespace("lake.eu", region="eu-west-1", objects=[trades])
lake = MemoryCatalog("lake", properties={"token": "t", "region": "global"}, objects=[eu])

# The parent's order, the child's values, the child's own appended.
assert lake.table("eu.trades").properties == {"token": "t", "region": "eu-west-1", "codec": "gzip"}
assert list(lake.namespace("eu").properties) == ["token", "region"]

# The deepest registered object whose URL holds a location, on a path boundary.
warehouse = Warehouse()
warehouse.register(lake)
assert warehouse.properties_for(root / "trades.csv") == {"codec": "gzip"}
assert warehouse.properties_for(root / "elsewhere.csv") == {}
```

## An object is a handle

```python
import pathlib
import tempfile

from yggdryl import IOBase
from yggdryl.warehouse import FolderCatalog

root = pathlib.Path(tempfile.mkdtemp())
(root / "eu").mkdir()
(root / "eu" / "trades.csv").write_text("symbol,price\nAAPL,187.5\n")

market = FolderCatalog("market", root)
assert isinstance(market, IOBase) and market.kind() == "catalog" and market.is_dir()
assert sorted(str(child) for child in market.ls(recursive=True)) == ["market.eu", "market.eu.trades"]
eu = market / "eu"
assert eu.kind() == "namespace"
trades = market.joinpath("eu/trades")
assert trades.kind() == "file"  # a leaf table's kind is its storage's
assert trades.row_size() == 1
try:
    eu.read_bytes()
except ValueError as error:
    assert "got a namespace" in str(error)
```

## The system warehouse

```python
import os
import pathlib
import tempfile

from yggdryl.warehouse import MediaTable, MemoryCatalog, SystemWarehouse

assert type(SystemWarehouse.catalog("local")) is MemoryCatalog
root = pathlib.Path(tempfile.mkdtemp())
(root / "trades.csv").write_text("symbol,price\nAAPL,187.5\n")
catalog = f"docs_skill_{os.getpid()}"
SystemWarehouse.register(MediaTable(f"{catalog}.raw.trades", root / "trades.csv"))
try:
    assert SystemWarehouse.table(f"{catalog}.raw.trades").row_size() == 1
finally:
    assert str(SystemWarehouse.unregister(catalog)) == catalog
```

## Gotchas in Python

- `name`, `path`, `description`, `modified`, `properties`, `storage`,
  `namespace_levels`, `namespaces` and `tables` are properties; `field()`,
  `kind()`, `row_size()`, `children()` are methods.
- A missing key in `namespaces[...]`/`tables[...]` is a `KeyError` carrying
  the core's message; a conflict (two leaves of one name) and every other
  refusal is a `ValueError`.
- A `str` path is dotted grammar: `tables["eu west"]` is two parts `eu` and
  `west`; write `tables['"eu west"']` or build with `["lake", "eu west"]`.
- `Catalog(...)`, `Namespace(...)` and `Table(...)` raise `TypeError`: build
  the implementation.
- `warehouse.register(IOBase(path))` is a `TypeError`: register a
  `MediaTable`, a `FolderCatalog` or another warehouse object.
- `update_properties` is refused by every implementation here.
