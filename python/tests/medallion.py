"""The capture pipeline over two catalogs, table to table on series.

`bronze` and `silver` are two catalogs - two Amazon S3 Tables table buckets
live, two local Iceberg warehouse folders in the suite - and
`record_keeping` the namespace in each. One function per stage, each taking
the :class:`Lake` the run writes through and a UTC window, half-open on
`transunix`; every read is `read_serie` with the window pushed into it, so
the scan prunes by the quarter hour each table is partitioned by, every
stage runs through the codec's serie doors, and the writes are of two
kinds. The first stage - the capture's lines into
`bronze.record_keeping.log_messages` - is the keyed append, `append_serie`
on a table whose `identifier-field-ids` is the pipeline's primary key: a
quarter-hour partition takes only the lines whose key it does not hold,
a line read again is skipped and no stored file is rewritten, so running
it again over a window appends what the window lacks and nothing else.
Every later stage writes with `overwrite_serie`, which replaces the
partitions its rows fall in and no other: its rows are derived, a
derivation can change, and running the stage again over a window rewrites
that window.
Every table is laid out as Apache Doris's Iceberg catalog reads one
(`into_scheme_compat("doris")`): an instant at microseconds under its zone,
since Doris maps no Iceberg v3 `timestamp_ns` or `timestamptz_ns`, and a
stage reads its source back as the row the stage before writes, known
before any table is read (`Lake.rows_of`), so a lake that wrote nothing
yet reads a table as the one that wrote it does.

```text
capture                              -> parse_log_messages         -> bronze.record_keeping.log_messages
bronze.record_keeping.log_messages   -> parse_fix_messages_raw     -> bronze.record_keeping.fix_messages
bronze.record_keeping.fix_messages   -> parse_fix_messages_refined -> silver.record_keeping.fix_messages
the codec's instruments              -> commit_instruments         -> silver.record_keeping.instruments
silver.record_keeping.fix_messages   -> parse_books                -> silver.record_keeping.books
silver.record_keeping.books          -> parse_events               -> silver.record_keeping.orders
                                                                   -> silver.record_keeping.quotes
                                                                   -> silver.record_keeping.executions
```

What the lifecycle learned of the instruments it met is committed right
after the FIX-message parse, as the stage `silver.instruments` between
`silver.fix_messages` and the books: `commit_instruments` writes the
codec's bound instruments to `silver.record_keeping.instruments` once the
refined messages are stored, so every instrument the window taught is in
the silver catalog before anything reads the refined messages. Every
market table - the FIX messages, the books, the orders, the quotes, the
executions - carries `instcode`, the cross code of the instrument a row is
about, which joins the instruments table on `crosscode` (or on one of its
`aliascodes`, a code it had before a re-key). When the
parse itself learns the instruments (the instrument phase's SPEC §10a) the
stage follows the raw parse instead.

The window rule: a row is windowed by `transunix`, and a stage reads the
rows of its source inside the window alone. A walk - the lifecycle, the book
fold - starts at the window with no state from before it: a chain that
began earlier follows no predecessor inside the window, and a book's first
instant in the window rebuilds over the empty book. A window therefore
opens on a quarter-hour boundary, where the book stage's snapshot grid
makes every book whole.

The cost rule: a request to a store is a round trip, so a run asks each
store the fewest. The lake opens each catalog's namespace once and keeps
every table a stage opened or created, so the next stage reads what the
one before wrote through the handle that wrote it - the document it
published, under the token it holds - rather than asking the catalog where
the table is again; and the books are read once for the three event tables,
the window's books held under the process spill bound and each table's kind
folded out of the held books. A write's `where` is not what splits them: it
names the rows the overwrite replaces across the whole table, where the
pipeline replaces the window's partitions alone.
"""

from __future__ import annotations

import argparse
import datetime as dt
import logging
import os
import pathlib
import sys
import threading
import time
from collections.abc import Callable, Iterable
from typing import Any

import yggdryl
from yggdryl import Catalog, ChunkedSerie, Field, IOBase, IOResult, Instruments, Namespace, StreamChunkedSerie, Table, TextOptions
from yggdryl.http import process_stats
from yggdryl.fix import ULBRIDGE_ROWHEADER, FixCodec, FixRegistry, fix_schema, fix_schema_carrying
from yggdryl.graph import MarketData
from yggdryl.iceberg import IcebergCatalog

NAMESPACE = "record_keeping"

# The primary key every table of the pipeline declares: when a row happened,
# which object it came from, its place there and the hash of what it states -
# the instant and the content alone repeat wherever two lines are one text.
PRIMARY_KEY = ("transunix", "crosshashcode", "seqnum", "hashcode")

# What every table of the pipeline requires of each row: its key and the code
# of its chain.
REQUIRED = (*PRIMARY_KEY, "crosscode")

# The partition every table computes for each row it is written: the quarter
# of an hour the row's instant falls in.
PARTUNIX = "time_bucket('15 minutes', transunix) as partunix"
PARTUNIX_DESCRIPTION = (
    "The quarter of an hour the row's instant falls in: transunix floored to fifteen minutes."
)

# The book stage's snapshot grid: every book whole each quarter of an hour.
QUARTER_MILLIS = 900_000

# What a table of the pipeline is created with.
TABLE_PROPERTIES = {"format-version": "3"}

# The event tables the books are laid out in, each through the door of its
# list and the kind it keeps: the orders and the quotes are the books'
# delta, the executions among their events.
EVENTS: tuple[tuple[str, Callable[[Any, str | None], StreamChunkedSerie], str], ...] = (
    ("orders", MarketData.delta_serie, "ORDR"),
    ("quotes", MarketData.delta_serie, "QUOT"),
    ("executions", MarketData.events_serie, "EXEC"),
)


def window_filter(start: dt.datetime, end: dt.datetime) -> str:
    """The `where` of one window: `transunix >= start and transunix < end`.

    Both instants state UTC; a naive instant, or one in another zone, is
    refused by name rather than read as something it is not.
    """
    for name, instant in (("start", start), ("end", end)):
        if instant.utcoffset() != dt.timedelta(0):
            raise ValueError(f"expected {name} in UTC, got {instant!r}")
    if end <= start:
        raise ValueError(f"expected a window with end after start, got {start!r} to {end!r}")
    text = lambda instant: instant.astimezone(dt.timezone.utc).isoformat().replace("+00:00", "Z")  # noqa: E731
    return f"transunix >= '{text(start)}' and transunix < '{text(end)}'"


def unnumbered(field: Field) -> Field:
    """`field` with every identifier a source table's schema carried taken
    off, at every depth, through its structural document: a stage's table
    numbers its own schema."""

    def strip(node: Any) -> Any:
        if isinstance(node, dict):
            return {key: strip(value) for key, value in node.items() if key != "PARQUET:field_id"}
        if isinstance(node, list):
            return [strip(item) for item in node]
        return node

    return Field.from_dict(strip(field.into_dict()))


def declared(row: Field, partition_by: Iterable[str] = (PARTUNIX,)) -> Field:
    """`row` as Doris reads an Iceberg table, laid out as every table of the
    pipeline is: Iceberg's layout, an instant at microseconds
    (`into_scheme_compat("doris")`), the write truncating each nanosecond
    value to whole microseconds.

    Partitioned by `partunix` - the quarter hour the table computes for every
    row written to it - and whatever else `partition_by` names, sorted by it,
    the instant, the place within the instant and the content hash, with the
    instant, the object, the place and the hash its primary key, every
    required column non-null,
    numbered by this table alone.
    """
    schema = unnumbered(row.into_scheme_compat("doris")).with_partition_by(list(partition_by))
    for name in REQUIRED:
        column = schema[name]
        column.set_nullable(False)
        schema[name] = column
    partunix = schema["partunix"]
    partunix.set_description(PARTUNIX_DESCRIPTION)
    schema["partunix"] = partunix
    schema.sort.by = ["partunix", "transunix", "seqnum", "hashcode"]
    schema = yggdryl.iceberg.assign_field_ids(schema)
    # Iceberg names a key by the identifiers of its columns, which exist once
    # the schema is numbered.
    key = sorted(schema[name].parquet_field_id for name in PRIMARY_KEY)
    schema.iceberg.update({"identifier-field-ids": ",".join(map(str, key))})
    return schema


class Lake:
    """The two catalogs a run writes through, the codec and the capture, and
    what the run opened on the way.

    Each catalog's `record_keeping` namespace is opened once, and every
    table a stage opened or created is kept: a stage reads its source through
    the handle the stage before wrote it with - the document that handle
    published, under the token it holds - and a rerun writes each table
    through the handle that wrote it last. A table the lake holds is
    therefore read as of the last run that touched it; a writer outside the
    pipeline is seen when a commit under the held token is refused and read
    again where the pointer moved.

    A table stores its stage's row as Doris reads it - an instant at
    microseconds - and the stage that reads the table names the row it reads
    back: the text line's (`log_row`), the FIX row the codec parses that line
    into (`fix_row`) or the market row (`MarketData.field()`), each known
    from the lake's own options and none from what this lake wrote, so a
    fresh lake reads a table as the one that wrote it does.
    """

    def __init__(
        self,
        bronze: Catalog,
        silver: Catalog,
        codec: FixCodec,
        logs: IOBase | str,
        namespace: str = NAMESPACE,
        rowheader: str = ULBRIDGE_ROWHEADER,
    ) -> None:
        self.catalogs = {"bronze": bronze, "silver": silver}
        self.codec = codec
        self.logs = logs if isinstance(logs, IOBase) else IOBase(logs)
        self.namespace_name = namespace
        self.namespaces: dict[str, Namespace] = {}
        self.tables: dict[tuple[str, str], Table] = {}
        self.rowheader = rowheader

    def text_options(self) -> TextOptions:
        """What the capture is read with: the row header lifting each line's
        captures, offset-free clocks read in UTC, rows numbered from one."""
        options = TextOptions()
        options.rowheader = self.rowheader
        options.timezone = "UTC"
        options.start_rownum = 1
        return options

    def log_row(self) -> Field:
        """The text line's row `bronze.record_keeping.log_messages` holds:
        the field the capture is read under (`TextOptions.source_field`)."""
        return self.text_options().source_field()

    def fix_row(self) -> Field:
        """The FIX row both FIX tables hold: the text line's columns
        leading, the codec's fixed FIX columns following, as
        `FixCodec.parse_text_serie` lays a text line out."""
        return fix_schema_carrying(self.log_row(), fix_schema(self.codec.registry))

    def namespace(self, catalog: str) -> Namespace:
        """The catalog's `record_keeping` namespace, opened or created once."""
        held = self.namespaces.get(catalog)
        if held is None:
            held = self.catalogs[catalog].namespaces.open_or_create(self.namespace_name)
            self.namespaces[catalog] = held
        return held

    def table_of(
        self, catalog: str, name: str, row: Field, partition_by: Iterable[str] = (PARTUNIX,)
    ) -> Table:
        """The table `name` of the catalog's namespace, as the lake holds it,
        else opened as it is or created from `row` laid out as `declared`
        lays it out, and kept."""
        held = self.tables.get((catalog, name))
        if held is None:
            held = self.namespace(catalog).tables.open_or_create(
                name, declared(row, partition_by), **TABLE_PROPERTIES
            )
            self.tables[catalog, name] = held
        return held

    def source_of(self, catalog: str, name: str) -> Table:
        """The table `name` of the catalog's namespace, which a stage reads
        from: as the lake holds it, else resolved through the catalog and
        kept."""
        held = self.tables.get((catalog, name))
        if held is None:
            held = self.catalogs[catalog].table(f"{self.namespace_name}.{name}")
            self.tables[catalog, name] = held
        return held

    def rows_of(
        self, catalog: str, name: str, start: dt.datetime, end: dt.datetime, row: Field
    ) -> StreamChunkedSerie:
        """The rows of the table `name` inside the window, read back as
        `row`, the row its stage writes (`stored_rows`)."""
        return stored_rows(self.source_of(catalog, name), start, end, row)


def stored_rows(table: Table, start: dt.datetime, end: dt.datetime, row: Field) -> StreamChunkedSerie:
    """The rows `table` holds inside the window, in the table's own order,
    as the row the stage wrote: the partition column the table computed
    taken off, the window pushed into the read, and each batch cast onto
    `row` - the row before the table laid it out for Doris - so a
    microsecond instant reads back under the row's own unit."""
    rows = StreamChunkedSerie.from_serie(table.read_serie(select="* exclude (partunix)", filter=window_filter(start, end)))
    return rows.cast(row)


def instruments(silver: Catalog, namespace_name: str = NAMESPACE) -> tuple[Table, Instruments]:
    """The instruments the pipeline meets, bound to
    `silver.record_keeping.instruments`, and that table: opened as it is or
    created from the instruments' own row - one row per instrument keyed by
    its `crosscode`, partitioned by the code's first two characters, the
    `truncate(crosscode, 2)` the row declares, which stores no column - and
    laid over the seed (`Instruments.seeded_from_url`), so the collection
    holds the common instruments the crate ships beneath what an earlier run
    committed, the table's rows winning, and commits what this run's
    lifecycle learns. A table laid out before the instrument row, which
    holds no `crosscode` and which a load refuses by name, is dropped and
    created afresh: silver is replayed from bronze. The collection is clean
    after the load, so the seed is committed on the first run, with the
    first instruments its lifecycle learns, and is part of every snapshot
    after. Hand it to the codec (`FixCodec(..., instruments=...)`) and the
    table to the lake (`Lake.tables`), which the report reads it from: the
    refined parse commits it (`commit_instruments`)."""
    namespace = silver.namespaces.open_or_create(namespace_name)
    row = yggdryl.iceberg.assign_field_ids(
        unnumbered(Instruments.field().into_scheme_compat("doris"))
    )
    table = namespace.tables.open_or_create("instruments", row, **TABLE_PROPERTIES)
    if "crosscode" not in table.field():
        table.remove(True)
        table = namespace.tables.open_or_create("instruments", row, **TABLE_PROPERTIES)
    return table, Instruments.seeded_from_url(table)


def commit_instruments(held: Instruments) -> IOResult:
    """What the lifecycle learned of the instruments it met, with the seed and
    what earlier runs committed, to the table the collection is bound to -
    `silver.record_keeping.instruments` - as one snapshot replacing every row
    of every partition, one row per instrument, only where the collection
    moved: a run that learned no new fact and met no instrument later than
    its `lastunix` writes nothing."""
    return held.commit()


def parse_log_messages(
    lake: Lake,
    start: dt.datetime,
    end: dt.datetime,
) -> IOResult:
    """The capture - log objects under a glob, read through the native backend
    their location selects, one request per object - to
    `bronze.record_keeping.log_messages`: one stream of text rows, the row
    header lifting each line's captures (`Lake.text_options`).

    The write is the keyed append: the table states the pipeline's primary
    key as its ``identifier-field-ids``, so each quarter-hour partition takes
    only the lines whose key it does not hold yet - a line read again lands
    nowhere, counted in ``skipped_rows`` - and no stored file is rewritten,
    the partition groups reading the key columns of the files the key
    bounds keep alone. Every later stage overwrites the partitions its rows
    reach instead, because its rows are derived and a derivation can change."""
    lines = StreamChunkedSerie.from_serie(
        lake.logs.read_serie(options=lake.text_options(), filter=window_filter(start, end))
    )
    return lake.table_of("bronze", "log_messages", lines.field).append_serie(lines)


def parse_fix_messages_raw(lake: Lake, start: dt.datetime, end: dt.datetime) -> IOResult:
    """`bronze.record_keeping.log_messages` to `bronze.record_keeping.fix_messages`
    by the parallel text-serie parse alone: no lifecycle."""
    parsed = lake.codec.parse_text_serie(lake.rows_of("bronze", "log_messages", start, end, lake.log_row()))
    return lake.table_of("bronze", "fix_messages", parsed.field).overwrite_serie(parsed)


def parse_fix_messages_refined(lake: Lake, start: dt.datetime, end: dt.datetime) -> IOResult:
    """`bronze.record_keeping.fix_messages`, read in its order, walked by the
    lifecycle over sorted input, to `silver.record_keeping.fix_messages`. The
    lifecycle learns the instruments it meets into the codec's bound
    instruments as it walks, filling each row's `instcode`; the stage after
    this one commits them."""
    walked = lake.codec.lifecycle_serie(lake.rows_of("bronze", "fix_messages", start, end, lake.fix_row()))
    return lake.table_of("silver", "fix_messages", walked.field).overwrite_serie(walked)


def commit_instruments_stage(lake: Lake, start: dt.datetime, end: dt.datetime) -> dict[str, IOResult]:
    """The stage right after the FIX-message parse: the codec's bound
    instruments committed to `silver.record_keeping.instruments`
    (`commit_instruments`), once `silver.fix_messages` has drained the walk,
    so it holds every instrument the window taught. Answers nothing where the
    codec holds none; the window is read by nothing here - a commit is
    whole. The collection commits through a handle of its own, so a commit
    that wrote opens the table the lake keeps afresh - one read of where its
    document is - for the report to read what was committed."""
    held = lake.codec.instruments
    if held is None:
        return {}
    committed = commit_instruments(held)
    key = ("silver", "instruments")
    if committed.written_rows and key in lake.tables:
        lake.tables[key] = lake.catalogs["silver"].table(f"{lake.namespace_name}.instruments")
    return {"silver.instruments": committed}


def parse_books(lake: Lake, start: dt.datetime, end: dt.datetime) -> IOResult:
    """`silver.record_keeping.fix_messages`, read in its order, folded into
    books every quarter of an hour, to `silver.record_keeping.books` -
    partitioned by `partunix` like every other table, each book keyed by its
    instrument's ISIN where it holds one, and holding the orders and quotes
    its tick applied in its `delta` and every other event of its tick - the
    executions, the snapshot controls - among its `events`.

    The quarter hour alone is the partition: a window opens on a quarter
    hour, so a rerun replaces whole partitions, and nothing reads the books
    by their instrument class, which once partitioned them beside it. A day
    of ticks is one file per quarter hour that holds a book - 84 for this
    capture - each a request to write and one to read, however many books a
    tick holds."""
    messages = lake.codec.messages_serie(lake.rows_of("silver", "fix_messages", start, end, lake.fix_row()))
    books = lake.codec.book_serie(messages, QUARTER_MILLIS)
    return lake.table_of("silver", "books", books.field).overwrite_serie(books)


def parse_events(lake: Lake, start: dt.datetime, end: dt.datetime) -> dict[str, IOResult]:
    """`silver.record_keeping.books`, read once, to the three event tables.

    The window's books are held under the process spill bound, and each
    table is written its kind folded out of the held books through the door
    `EVENTS` names - the orders and the quotes out of the books' `delta`,
    the executions out of their `events` - so the books' files are read once
    rather than once per kind. Each write replaces the partitions its
    rows fall in, as every stage's does; a write's `where` would instead
    name the rows the overwrite replaces across the whole table.
    """
    books = ChunkedSerie.from_(lake.rows_of("silver", "books", start, end, MarketData.field()))
    written: dict[str, IOResult] = {}
    for name, door, kind in EVENTS:
        rows = door(StreamChunkedSerie.from_chunked(books), kind)
        written[f"silver.{name}"] = lake.table_of("silver", name, rows.field).overwrite_serie(rows)
    return written


Stage = Callable[[Lake, dt.datetime, dt.datetime], dict[str, IOResult]]

# Every stage in the diagram's order, each answering what it wrote keyed by
# the table it wrote under its catalog.
STAGES_OF: dict[str, Stage] = {
    "bronze.log_messages": lambda lake, start, end: {"bronze.log_messages": parse_log_messages(lake, start, end)},
    "bronze.fix_messages": lambda lake, start, end: {"bronze.fix_messages": parse_fix_messages_raw(lake, start, end)},
    "silver.fix_messages": lambda lake, start, end: {"silver.fix_messages": parse_fix_messages_refined(lake, start, end)},
    "silver.instruments": commit_instruments_stage,
    "silver.books": lambda lake, start, end: {"silver.books": parse_books(lake, start, end)},
    "silver.events": parse_events,
}

# Every table of the pipeline, in the order the stages write them.
TABLES = (
    "bronze.log_messages",
    "bronze.fix_messages",
    "silver.fix_messages",
    "silver.instruments",
    "silver.books",
    "silver.orders",
    "silver.quotes",
    "silver.executions",
)


def run(lake: Lake, start: dt.datetime, end: dt.datetime) -> dict[str, IOResult]:
    """Every stage over one window, in the diagram's order: what each wrote,
    keyed by the table it wrote under its catalog."""
    written: dict[str, IOResult] = {}
    for stage in STAGES_OF.values():
        written.update(stage(lake, start, end))
    return written


def resident_bytes() -> int:
    """The process's resident set now: the working set on Windows, `VmRSS`
    elsewhere."""
    if sys.platform == "win32":
        import ctypes
        import ctypes.wintypes as wintypes

        class Counters(ctypes.Structure):
            _fields_ = [
                ("cb", wintypes.DWORD),
                ("PageFaultCount", wintypes.DWORD),
                ("PeakWorkingSetSize", ctypes.c_size_t),
                ("WorkingSetSize", ctypes.c_size_t),
                ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
                ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
                ("PagefileUsage", ctypes.c_size_t),
                ("PeakPagefileUsage", ctypes.c_size_t),
            ]

        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.GetCurrentProcess.restype = wintypes.HANDLE
        kernel32.K32GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD]
        kernel32.K32GetProcessMemoryInfo.restype = wintypes.BOOL
        counters = Counters()
        counters.cb = ctypes.sizeof(Counters)
        if not kernel32.K32GetProcessMemoryInfo(kernel32.GetCurrentProcess(), ctypes.byref(counters), counters.cb):
            raise ctypes.WinError(ctypes.get_last_error())
        return int(counters.WorkingSetSize)
    for line in pathlib.Path("/proc/self/status").read_text(encoding="ascii").splitlines():
        if line.startswith("VmRSS:"):
            return int(line.split()[1]) * 1024
    return 0


def requests_since(before: dict[str, Any]) -> dict[str, dict[str, int]]:
    """What the process asked each host since `before`, a reading of
    `process_stats`, host by host: the cost of a stage in round trips, the
    control plane's apart from the store's."""
    since: dict[str, dict[str, int]] = {}
    for host, counts in process_stats().items():
        earlier = before.get(host, {})
        delta = {name: count - earlier.get(name, 0) for name, count in counts.items()}
        if delta["requests"]:
            since[host] = delta
    return since


def print_requests(indent: str, since: dict[str, dict[str, int]]) -> None:
    for host, counts in since.items():
        print(
            f"{indent}{host:<52} {counts['requests']:>5}  GET {counts['gets']:>4}  PUT {counts['puts']:>4}"
            f"  HEAD {counts['heads']:>3}  POST {counts['posts']:>3}  DELETE {counts['deletes']:>3}"
        )


class Sampler:
    """The peak resident set over a stretch, sampled every `period` seconds
    on a thread of its own, so a stage that holds a window in memory is
    seen holding it."""

    def __init__(self, period: float = 0.05) -> None:
        self.period = period
        self.peak = 0
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._sample, daemon=True)

    def _sample(self) -> None:
        while not self._stop.is_set():
            self.peak = max(self.peak, resident_bytes())
            self._stop.wait(self.period)

    def __enter__(self) -> Sampler:
        self.peak = resident_bytes()
        self._thread.start()
        return self

    def __exit__(self, *_: object) -> None:
        self._stop.set()
        self._thread.join()
        self.peak = max(self.peak, resident_bytes())


def catalog_of(location: str, name: str) -> Catalog:
    """The catalog `location` names: a URL - `s3tables://<bucket>`, or the
    table bucket's ARN, which spares the one listing a bare location pays to
    find its bucket - through `Catalog.from_url`, a local path an Iceberg
    warehouse folder opened or created under `name`."""
    if "://" in location or location.startswith("arn:"):
        return Catalog.from_url(location, name=name)
    return IcebergCatalog.open_or_create(name, pathlib.Path(location))


def instant(text: str) -> dt.datetime:
    """An ISO 8601 instant stating UTC - `2026-08-14T00:00:00Z`."""
    parsed = dt.datetime.fromisoformat(text.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise argparse.ArgumentTypeError(f"expected a UTC instant such as 2026-08-14T00:00:00Z, got {text!r}")
    return parsed.astimezone(dt.timezone.utc)


def report(lake: Lake) -> None:
    """Every table of the pipeline: its rows and snapshots, read off the
    handles the run holds."""
    for name in TABLES:
        catalog_name, table_name = name.split(".", 1)
        table = lake.tables.get((catalog_name, table_name))
        if table is None:
            print(f"  {name:<22} absent")
            continue
        snapshots = len(getattr(table, "snapshots", ()))
        print(f"  {name:<22} rows {table.row_size():>8}  snapshots {snapshots}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="The capture pipeline over two catalogs: every stage over one UTC window, streamed."
    )
    parser.add_argument("--bronze", required=True, help="the bronze catalog: s3tables://<bucket>, its ARN, or a warehouse folder")
    parser.add_argument("--silver", required=True, help="the silver catalog: s3tables://<bucket>, its ARN, or a warehouse folder")
    parser.add_argument("--logs", required=True, help="the capture: a URL or path, a glob over log objects")
    parser.add_argument("--start", required=True, type=instant, help="the window's first instant, UTC, on a quarter hour")
    parser.add_argument("--end", required=True, type=instant, help="the window's end, UTC, excluded")
    parser.add_argument("--dictionary", default=str(pathlib.Path(__file__).resolve().parents[2] / "config" / "fix"))
    parser.add_argument("--rowheader", default=ULBRIDGE_ROWHEADER)
    parser.add_argument("--threads", type=int, default=None, help="the codec's threads; the host's by default")
    parser.add_argument("--runs", type=int, default=1, help="how many times to run the window: a second run rewrites it")
    parser.add_argument("--namespace", default=NAMESPACE, help="the namespace of every table in both catalogs")
    args = parser.parse_args(argv)
    # The crate logs through Python's `logging`: `YGGDRYL_LOG_LEVEL=INFO`
    # shows the identity walk (`yggdryl.aws.session`, key ids masked) and
    # every store's requests beside the stages.
    logging.basicConfig(level=os.environ.get("YGGDRYL_LOG_LEVEL", "WARNING").upper(), format="%(levelname)s %(name)s: %(message)s")

    # The silver catalog first: the instruments the lifecycle learns into
    # are bound to its table, which the lake keeps for the report.
    silver = catalog_of(args.silver, "silver")
    table, held = instruments(silver, args.namespace)
    codec = FixCodec(
        FixRegistry.from_handle(args.dictionary), exclude_msgtypes=[], threads=args.threads, instruments=held
    )
    lake = Lake(
        catalog_of(args.bronze, "bronze"),
        silver,
        codec,
        IOBase(args.logs),
        namespace=args.namespace,
        rowheader=args.rowheader,
    )
    lake.tables["silver", "instruments"] = table
    print(f"window {args.start.isoformat()} -> {args.end.isoformat()}  resident at start {resident_bytes() >> 20} MiB")
    for run_at in range(1, args.runs + 1):
        print(f"run {run_at}")
        run_before = process_stats()
        for stage, step in STAGES_OF.items():
            before = process_stats()
            started = time.perf_counter()
            with Sampler() as sampler:
                results = step(lake, args.start, args.end)
            seconds = time.perf_counter() - started
            for name, result in results.items():
                print(
                    f"  {name:<22} read {result.read_rows:>8}  wrote {result.written_rows:>8}"
                    f"  skipped {result.skipped_rows:>6}"
                )
            print(f"  {stage:<22} {seconds:7.2f}s  peak resident {sampler.peak >> 20:>6} MiB")
            print_requests("      ", requests_since(before))
        before = process_stats()
        report(lake)
        print("  report")
        print_requests("      ", requests_since(before))
        print(f"  run {run_at} requests")
        print_requests("      ", requests_since(run_before))
    return 0


if __name__ == "__main__":
    sys.exit(main())
