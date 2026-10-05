# yggdryl-logging in Python

`import logging` - once `yggdryl` is imported, the standard module *is* the
core's logging: each core logger is the Python logger of the same dotted
name, so levels, filters, formatters, handlers and `basicConfig` are
Python's own. `from yggdryl.logging import FileHandler, TerminalHandler,
TerminalFormatter, Deduplicate, deduplicate, is_deduplicating` adds what the
core brings: a handler writing through any location, the terminal line, and
deduplication.

## Set up an application

`logging.basicConfig(level=logging.INFO, handlers=[TerminalHandler()])` is
the whole setup for the core's terminal line on standard error - time,
glyph and level, `[threadName]`, logger, `funcName:lineno`, message.
`TerminalHandler(stream=None, color=None)` is a `logging.StreamHandler`
(standard error when `stream` is `None`) spelling records with
`TerminalFormatter(color)`; it colours by the stream's `isatty()` and the
colour variables unless `color` says. `basicConfig()` without `handlers`
stays Python's own format.

```python
import io
import logging
import re

from yggdryl.logging import TerminalHandler

# A stream is given here to read the line back; `color=False` keeps it plain.
stream = io.StringIO()
handler = TerminalHandler(stream, color=False)
logging.basicConfig(level=logging.INFO, handlers=[handler])
feed = logging.getLogger("skills.logging.feed")


def open_feed() -> None:
    feed.info("opened %d venues", 3)


open_feed()
feed.debug("below the root's level")
pattern = (
    r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3} • INFO     \[MainThread\] "
    r"skills\.logging\.feed open_feed:\d+ › opened 3 venues\n"
)
assert re.fullmatch(pattern, stream.getvalue()), stream.getvalue()

root = logging.getLogger()
root.removeHandler(handler)
root.setLevel(logging.WARNING)
```

## Spell a record as the terminal line

`TerminalFormatter(color=False)` is a `logging.Formatter` rendered natively,
one call per record. It reads the time from `record.created` and
`record.msecs` and the level as `record.levelname` names it, so a level
`logging.addLevelName` registered prints its name, and an exception or a
stack the record carries hangs under the message.

```python
import logging

from yggdryl.logging import TerminalFormatter

# 2023-11-14 22:13:20.5 UTC, raised in `open` at line 42, on the thread Python names.
record = logging.LogRecord(
    "trades.feed", logging.INFO, "src/feed.py", 42, "opened %d venues", (3,), None, func="open"
)
record.created, record.msecs = 1_700_000_000.5, 500.0
assert TerminalFormatter().format(record) == (
    "2023-11-14 22:13:20,500 • INFO     [MainThread] trades.feed open:42 › opened 3 venues"
)
assert TerminalFormatter(color=True).format(record).startswith(
    "\x1b[2m2023-11-14 22:13:20,500\x1b[0m \x1b[32m• INFO    \x1b[0m \x1b[2m[MainThread]\x1b[0m"
)

logging.addLevelName(25, "NOTICE")
notice = logging.LogRecord("trades.feed", 25, "src/feed.py", 43, "halted", None, None, func="open")
notice.created, notice.msecs = 1_700_000_000.5, 500.0
assert " • NOTICE   [MainThread] trades.feed open:43 › halted" in TerminalFormatter().format(notice)
```

## The core's records are Python records

A core record is made by its logger's `makeRecord` - `pathname` the Rust
file, `lineno` the Rust line, `funcName` `(unknown function)` - and handed
to its `handle`, so Python's filters, handlers, propagation and `disabled`
apply. `logging.getLogger("yggdryl")` is the one switch; a level changed at
any time applies to the next record, and a record no logger handles never
takes the interpreter. A crate the build depends on reaches `logging` only
at `WARNING` and above. `yggdryl.aws.session` reports each AWS credential
source asked (`DEBUG`), the one that answered (`INFO`) and a set passed over
(`WARNING`), key ids masked.

```python
import logging
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase
from yggdryl.iceberg import IcebergTable, assign_field_ids

records: list[logging.LogRecord] = []


class Collect(logging.Handler):
    def emit(self, record: logging.LogRecord) -> None:
        records.append(record)


collect = Collect()
package = logging.getLogger("yggdryl")
package.addHandler(collect)
package.setLevel(logging.INFO)

schema = assign_field_ids(pa.schema([pa.field("id", pa.int64(), nullable=False)]))
with tempfile.TemporaryDirectory() as folder:
    IcebergTable.create(IOBase(pathlib.Path(folder) / "trades"), schema)
    [made] = [record for record in records if record.name == "yggdryl.iceberg.table"]
    assert made.levelno == logging.INFO
    assert made.getMessage().startswith("created iceberg table at")
    assert made.pathname.endswith("table.rs") and made.lineno > 0
    assert made.funcName == "(unknown function)"

    # A nearer logger's level wins from the next record on.
    logging.getLogger("yggdryl.iceberg").setLevel(logging.WARNING)
    records.clear()
    IcebergTable.create(IOBase(pathlib.Path(folder) / "quiet"), schema)
    assert [record for record in records if record.name.startswith("yggdryl.iceberg")] == []

logging.getLogger("yggdryl.iceberg").setLevel(logging.NOTSET)
package.removeHandler(collect)
package.setLevel(logging.NOTSET)
```

## Write a log through any location

`FileHandler(location, mode="append", capacity=0, flush_level=logging.ERROR,
level=logging.NOTSET)` is a `logging.Handler` writing Python and core
records alike through a `str`, `os.PathLike`, `Url` or `IOBase` - a local
file, an object in a bucket, a ZIP member - with one append per publish. A
`capacity` holds records back until that many bytes are held or a record at
`flush_level` (a level, or a name the core reads in any case) arrives: state
one over a remote store, where an append is a whole `PUT`. With no formatter
set it writes the terminal line, plain. `logging.shutdown`, which Python
runs at exit, publishes what it holds. An `IOBase` given as `location` is
taken whole and answers nothing after; `mode`, `flush_level` and `level` are
checked before it is taken, so a refused constructor leaves it usable.

```python
import logging
import pathlib
import tempfile

from yggdryl import IOBase
from yggdryl.logging import FileHandler

logger = logging.getLogger("skills.logging.file")
logger.propagate = False
logger.setLevel(logging.INFO)

with tempfile.TemporaryDirectory() as folder:
    path = pathlib.Path(folder) / "logs" / "feed.log"
    handler = FileHandler(path, capacity=64 * 1024, flush_level="error")
    handler.setFormatter(logging.Formatter("%(levelname)s %(message)s"))
    assert (handler.mode, handler.capacity, handler.flush_level) == ("append", 65_536, logging.ERROR)
    assert handler.url is not None and handler.url.startswith("file://")
    logger.addHandler(handler)

    logger.info("opened")
    assert not path.exists()  # held under the capacity
    logger.error("rejected")  # a record at the flush level publishes everything held
    assert path.read_text(encoding="utf-8") == "INFO opened\nERROR rejected\n"
    logger.info("closed")
    handler.flush()
    assert path.read_text(encoding="utf-8").endswith("INFO closed\n")
    logger.removeHandler(handler)
    handler.close()

    # `overwrite` replaces what the location holds with the first publish;
    # an `IOBase` given is taken whole.
    replaced = pathlib.Path(folder) / "replaced.log"
    replaced.write_text("yesterday\n", encoding="utf-8")
    fresh = FileHandler(IOBase(replaced), mode="overwrite")
    fresh.setFormatter(logging.Formatter())
    logger.addHandler(fresh)
    logger.info("today")
    assert replaced.read_text(encoding="utf-8") == "today\n"
    logger.removeHandler(fresh)
    fresh.close()

    for mode, reason in (("merge", "expected append or overwrite"), ("a", "invalid mode expression")):
        try:
            FileHandler(path, mode=mode)
        except ValueError as error:
            assert reason in str(error), error
        else:
            raise AssertionError(f"mode {mode!r} is refused")

logger.setLevel(logging.NOTSET)
logger.propagate = True
```

## Deduplicate repeats

Two doors, by who logs. A record Python logs is counted by a
`Deduplicate(name="")` filter - the core's hash of the logger, the level and
the message - passing the first, rewriting the 10th, 100th, 1000th as
`message (seen N times)` and dropping the rest. `deduplicate(logger,
enabled=True)` states the flag on the core's logger of that name, so the
core drops its *own* repeats where it logs them, before `logging` is asked;
`None` takes the ancestors' choice again. A call whose message cannot be
formatted (`logger.warning("%d items", "x")`) passes `Deduplicate` uncounted
and never raises into the caller: the handler's `handleError` reports it.

```python
import logging
import pathlib
import tempfile

from yggdryl.logging import Deduplicate, FileHandler, deduplicate, is_deduplicating

feed = logging.getLogger("skills.logging.dedup")
orders = feed.getChild("orders")
feed.setLevel(logging.INFO)
feed.propagate = False

with tempfile.TemporaryDirectory() as folder:
    path = pathlib.Path(folder) / "dedup.log"
    handler = FileHandler(path)
    handler.setFormatter(logging.Formatter())
    handler.addFilter(Deduplicate())
    feed.addHandler(handler)

    for _ in range(100):
        orders.warning("late %s", "fill")
    orders.error("late fill")  # another level is another key
    assert path.read_text(encoding="utf-8") == (
        "late fill\nlate fill (seen 10 times)\nlate fill (seen 100 times)\nlate fill\n"
    )
    feed.removeHandler(handler)
    handler.close()

# The core's own records: a logger below the named one included.
assert not is_deduplicating("yggdryl.iceberg.table")
deduplicate("yggdryl.iceberg")
assert is_deduplicating("yggdryl.iceberg.table")
deduplicate("yggdryl.iceberg", None)
assert not is_deduplicating("yggdryl.iceberg.table")

feed.setLevel(logging.NOTSET)
feed.propagate = True
```

## Where the core's warnings land

What a FIX parse, a market projection or a lifecycle walk passes over is
said as a `WARNING` on the logger of the raising module (`yggdryl.fix.*`),
once per kind for the whole process with its detail, then counted at each
tenfold occurrence - so the package's default `WARNING` level already shows
it, and a handler on `yggdryl` takes it.

```python
import logging

from yggdryl.fix import FixCodec, FixRegistry

said: list[logging.LogRecord] = []


class Collect(logging.Handler):
    def emit(self, record: logging.LogRecord) -> None:
        said.append(record)


collect = Collect()
logging.getLogger("yggdryl").addHandler(collect)

codec = FixCodec(FixRegistry())
for _ in range(10):
    # `52=bad` names no instant: the message is read and its clock left unstated.
    message = codec.parse_fix_line(b"8=FIX.4.4|35=D|52=bad|10=0|")
    assert [field for field, _ in message.anomalies] == ["sendingtime"]

clock = [record for record in said if "FIX clock left unstated" in record.getMessage()]
assert all(record.name.startswith("yggdryl.fix.") and record.levelno == logging.WARNING for record in clock)
first, counted = clock
assert "(sendingtime)" in first.getMessage()
assert "later occurrences are counted rather than repeated" in first.getMessage()
assert counted.getMessage().endswith("seen 10 times")

logging.getLogger("yggdryl").removeHandler(collect)
```

## Gotchas in Python

- `TerminalFormatter` reads the time from `record.created` and its
  milliseconds from `record.msecs`: a `logging.LogRecord` built by hand sets
  both, as in the recipe above.
- `FileHandler.close()` publishes what is held, and a record logged after it
  reopens the location and is published at once: remove a finished handler
  from its logger first, as the recipes do.
- A `KeyboardInterrupt` a Python handler raises over a core record is raised
  again in the main thread, so Ctrl-C still stops the program; any other
  exception it raises is reported as unraisable.
- A refused `FileHandler` argument is a `ValueError` (`mode="merge"`,
  `mode="a"`, an unknown `flush_level` name or `level` name) and a `level` of
  another kind a `TypeError`; nothing is half built for `logging.shutdown` to
  find.
