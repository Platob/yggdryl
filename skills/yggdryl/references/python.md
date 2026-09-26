# yggdryl in Python

`pip install yggdryl` (Python 3.10+, `pyarrow>=18`). The wheel carries every
part of the core - Parquet, Iceberg, the object stores - and the `ygg`
command. The package mirrors the crate: one module per type at the root
(`yggdryl.integer`, `yggdryl.temporal`, `yggdryl.string`, ...) and one per
implementation (`yggdryl.json`, `yggdryl.iceberg`, `yggdryl.xxhash`), with
every type and factory re-exported from `yggdryl` itself. It ships `py.typed`
and stubs, so `mypy --strict` checks calls.

## Arguments: omitted, `None`, and properties by name

An optional argument left out keeps its default; `None` is a value and
clears. Every record read and write takes `options` and, beside it, any
`RecordOptions` property by name, applied to a copy.

```python
import pathlib
import tempfile

from yggdryl import IOBase

with tempfile.TemporaryDirectory() as folder:
    handle = IOBase(pathlib.Path(folder) / "prices.arrows")
    handle.overwrite_records([{"id": 1, "px": 1.5}, {"id": 2, "px": 2.5}])

    # A property by name is set on a copy of the handle's own options.
    only_ids = handle.read_arrow_reader(select="id").read_all()
    assert only_ids.column_names == ["id"]
    assert handle.read_arrow_reader().read_all().num_columns == 2
```

## Errors

Native refusals surface as `ValueError` (the input is not what the type
accepts), `TypeError` (the argument is the wrong kind), and `OSError`
subclasses for storage, each carrying the core's message with its location.

```python
from yggdryl import DataType

try:
    DataType("int8").scalar(1000)
except ValueError as refused:
    assert "int8" in str(refused)
else:
    raise AssertionError("1000 is not an int8")
```

## End to end: schema, value, records, document

```python
import pathlib
import tempfile
from decimal import Decimal

from yggdryl import DataType, Field, IOBase, json

# The schema: a non-null struct field whose children are the columns.
schema = Field(
    "trade",
    DataType.from_fields(
        [
            Field("id", "int64", nullable=False),
            Field("symbol", "utf8"),
            Field("price", DataType.decimal(18, 4), nullable=False),
        ]
    ),
    nullable=False,
)

# A value enters through its type and lands at the column's scale.
price = schema.dtype[2].scalar(Decimal("12.5"))
assert price.as_py() == Decimal("12.5000")

with tempfile.TemporaryDirectory() as folder:
    # The suffix picks the encoding: `.arrows` is an Arrow IPC stream.
    handle = IOBase(pathlib.Path(folder) / "trades.arrows")
    handle.overwrite_records(
        [{"id": 1, "symbol": "AAPL", "price": Decimal("224.62")}], field=schema
    )

    # Batches stream through a pyarrow.RecordBatchReader over the C Stream interface.
    reader = handle.read_arrow_reader()
    assert sum(batch.num_rows for batch in reader) == 1
    assert list(handle.read_records()) == [
        {"id": 1, "symbol": "AAPL", "price": Decimal("224.6200")}
    ]

# JSON, YAML, TOML and XML are byte-first: bytes out, bytes or text in.
assert json.loads(json.dumps({"symbol": "AAPL"})) == {"symbol": "AAPL"}
```

## Record classes

`@scalar` makes a standard dataclass whose `into_field()` is its schema; its
instances are rows every record writer accepts and every reader can rebuild.

```python
import pathlib
import tempfile

from yggdryl import IOBase, scalar


@scalar(frozen=True)
class Trade:
    id: int
    venue: str


assert [child.name for child in Trade.into_field()] == ["id", "venue"]

with tempfile.TemporaryDirectory() as folder:
    handle = IOBase(pathlib.Path(folder) / "trades.arrows")
    handle.overwrite_records([Trade(1, "XNAS"), Trade(2, "XNYS")])
    assert list(handle.read_records(Trade)) == [Trade(1, "XNAS"), Trade(2, "XNYS")]
```

## Logging

The core reports operations through `logging` under the `yggdryl` logger
(one record per operation, never per row). A level set after import reaches
the native bridge only through `yggdryl.refresh_logging()`.

## Gotchas in Python

- `IOBase(...)` answers the native class for the location (`LocalPath`, `Parquet`, `Buffer`, ...); all share one surface, so code against `IOBase`.
- Whole-resource bytes are `read_bytes`/`write_bytes`; ranges are `read_range_bytes(offset, length)`.
- pyarrow objects cross zero-copy over the Arrow C Data Interface; converting through `to_pylist()` defeats that - keep batches as batches.
- A `str` given to a structured-text loader (`json.loads`, `toml.load`) is content, never a path: pass `pathlib.Path` for a file.
- Datatype arguments accept their own expression (`"int64"`, `"decimal(18,4)"`) or a Python type (`int`, `str`, `Decimal`); parse once and reuse the `DataType` in hot code.
