# yggdryl in Python

`pip install yggdryl` (Python 3.10+, `pyarrow>=18`). The wheel carries every
part of the core - Parquet, Iceberg, the object stores - and the `yggdryl`
command. The package mirrors the crate: one module per type at the root
(`yggdryl.integer`, `yggdryl.temporal`, `yggdryl.string`, ...) and one per
implementation (`yggdryl.json`, `yggdryl.iceberg`, `yggdryl.xxhash`), with
every type and factory re-exported from `yggdryl` itself. It ships `py.typed`
and stubs, so `mypy --strict` checks calls.

## Arguments: omitted, `None`, and properties by name

An optional `RecordOptions`/`TextOptions` property left out keeps its
default; passing it as `None` is a value and clears it back to that default.
Every record read and write takes `options` and, beside it, any
`RecordOptions` property by name, applied to a copy.

This is not the rule for `Field`/`DataType` metadata. `Field.set_alias`,
`set_comment` and `set_display` take a required `str` and raise `TypeError`
on `None`; the `alias`/`comment`/`display` properties are read-only outright
(assignment raises `AttributeError`). Clear one with `remove_alias()`,
`remove_comment()` or `remove_display()`.

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
Checked arithmetic on `Scalar` raises `ArithmeticError` subclasses
(`OverflowError`, `ZeroDivisionError`).

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

Importing `yggdryl` makes Python's `logging` the host of the core's own
logging tree: each core logger is the Python logger of the same dotted name
(`yggdryl.iceberg.table`), so its level, filters, handlers and ancestors apply
as they do to any record logged in Python, and `logging.getLogger("yggdryl")`
is the one switch for the whole package. The core reports one record per
operation, never per row; a crate the build depends on reaches `logging` only
at `WARNING` and above, so `DEBUG` narrates this package and nothing else.

A level changed at any time applies to the next record - `setLevel`,
`logging.disable`, `basicConfig` and `dictConfig` all drop the cached levels
the native side keeps - so there is no refresh call. A record no Python logger
handles never takes the interpreter.

`logging.basicConfig(level=logging.INFO, handlers=[TerminalHandler()])` is the
whole setup for the core's terminal line on standard error: the time, the
level's glyph and `record.levelname` (a name `logging.addLevelName`
registered included), `[threadName]`, the logger, `funcName:lineno`, the
message, an exception or a stack hanging under it -
`2026-10-03 14:05:09,123 • INFO     [MainThread] trades.feed open:42 › opened 3 venues`.
`yggdryl.logging.TerminalHandler(stream=None, color=None)` is a
`logging.StreamHandler` (standard error when `stream` is `None`) spelling
records with `TerminalFormatter(color)`, rendered natively; unless `color` is
stated it is decided from `stream.isatty()`: `NO_COLOR` off, else
`FORCE_COLOR` or `CLICOLOR_FORCE` on, else `TERM=dumb` off, else a terminal.
`basicConfig()` without `handlers` and `logging.lastResort` stay Python's own.

`yggdryl.logging.FileHandler(location, mode="append", capacity=0,
flush_level=logging.ERROR, level=logging.NOTSET)` is a `logging.Handler`
writing through any location the package reads (`str`, `os.PathLike`, `Url`,
`IOBase`, an object in a bucket): one append per publish, each record at once
by default, as the terminal line, plain, unless a formatter is set. `mode` is
`"append"` or `"overwrite"` (`"a"` and `"w"` are refused). A `capacity`
holds records back until that many bytes are held or a record at
`flush_level` arrives - state one over a remote store, where an append is a
whole `PUT`. `flush()`, `close()` and `logging.shutdown` (which
Python runs at exit) publish what is held. An `IOBase` given as `location` is
taken whole and answers nothing after; every argument is checked before it is
taken, so a refused constructor leaves it usable.

`yggdryl.logging.deduplicate(logger, enabled=True)` (a name or a
`logging.Logger`; `None` takes the ancestors' choice again) makes the core
drop its own repeated records where it logs them, before `logging` is asked:
the first is said, the 10th, 100th, 1000th as `message (seen N times)`.
`yggdryl.logging.is_deduplicating(logger)` asks, and
`yggdryl.logging.Deduplicate(name="")` is the same counting as a
`logging.Filter` for any record, Python's own included; a malformed call
(`logger.warning("%d items", "x")`) passes it uncounted, for the handler's
`handleError` to report.

```python
import io
import logging
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase
from yggdryl.iceberg import IcebergTable, assign_field_ids
from yggdryl.logging import FileHandler, TerminalHandler

schema = pa.schema([pa.field("id", pa.int64(), nullable=False), pa.field("venue", pa.string())])

with tempfile.TemporaryDirectory() as folder:
    root = pathlib.Path(folder)
    location = root / "logs" / "app.log"
    handler = FileHandler(location, capacity=1 << 16)
    handler.setFormatter(logging.Formatter("%(levelname)s %(name)s %(message)s"))

    # The core's loggers hang off `yggdryl`, like any package's.
    package = logging.getLogger("yggdryl")
    package.addHandler(handler)
    package.setLevel(logging.INFO)

    IcebergTable.create(IOBase(root / "trades"), assign_field_ids(schema))
    handler.flush()
    lines = location.read_text(encoding="utf-8").splitlines()
    assert any(line.startswith("INFO yggdryl.iceberg.table created iceberg table at") for line in lines)

    # A level changed now applies to the next record.
    package.setLevel(logging.ERROR)
    IcebergTable.create(IOBase(root / "quiet"), assign_field_ids(schema))
    handler.flush()
    assert location.read_text(encoding="utf-8").splitlines() == lines

    package.removeHandler(handler)
    package.setLevel(logging.NOTSET)
    handler.close()

# The terminal line, into any stream; `color=False` keeps it plain whatever
# the environment says.
stream = io.StringIO()
feed = logging.getLogger("skills.yggdryl.feed")
feed.addHandler(TerminalHandler(stream, color=False))
feed.propagate = False
feed.warning("crossed")
line = stream.getvalue()
assert " ! WARNING  [MainThread] skills.yggdryl.feed " in line, line
assert line.endswith(" › crossed\n"), line
```

## Gotchas in Python

- `IOBase(...)` answers the native class for the location (`LocalPath`, `Parquet`, `Buffer`, ...); all share one surface, so code against `IOBase`.
- Whole-resource bytes are `read_bytes`/`write_bytes`; ranges are `read_range_bytes(offset, length)`.
- pyarrow objects cross zero-copy over the Arrow C Data Interface; converting through `to_pylist()` defeats that - keep batches as batches.
- A `str` given to a structured-text loader (`json.loads`, `toml.loads`) is content, never a path: pass `pathlib.Path` for a file.
- Datatype arguments accept their own expression (`"int64"`, `"decimal(18,4)"`) or a Python type (`int`, `str`, `Decimal`); parse once and reuse the `DataType` in hot code.
