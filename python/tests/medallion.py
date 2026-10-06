"""The capture pipeline over two catalogs, table to table on series.

`bronze` and `silver` are two catalogs - two Amazon S3 Tables table buckets
live, two local Iceberg warehouse folders in the suite - and
`record_keeping` the namespace in each. One function per stage, each taking
a UTC window, half-open on `currunix`; every read is `read_serie` with the
window pushed into it, so the scan prunes by the quarter hour each table is
partitioned by, every stage runs through the codec's serie doors, and every
write is `overwrite_serie`, which replaces the partitions its rows fall in
and no other: running a stage again over a window rewrites that window.

```text
capture                              -> parse_log_messages         -> bronze.record_keeping.log_messages
bronze.record_keeping.log_messages   -> parse_fix_messages_raw     -> bronze.record_keeping.fix_messages
bronze.record_keeping.fix_messages   -> parse_fix_messages_refined -> silver.record_keeping.fix_messages
silver.record_keeping.fix_messages   -> parse_books                -> silver.record_keeping.books
silver.record_keeping.books          -> parse_orders               -> silver.record_keeping.orders
                                     -> parse_quotes               -> silver.record_keeping.quotes
                                     -> parse_executions           -> silver.record_keeping.executions
```

The window rule: a row is windowed by `currunix`, and a stage reads the
rows of its source inside the window alone. A walk - the lifecycle, the book
fold - starts at the window with no state from before it: a chain that
began earlier follows no predecessor inside the window, and a book's first
instant in the window rebuilds over the empty book. A window therefore
opens on a quarter-hour boundary, where the book stage's snapshot grid
makes every book whole.
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
from collections.abc import Iterable
from typing import Any

import yggdryl
from yggdryl import Catalog, Field, IOBase, IOResult, IsinRegistry, Table, TextOptions
from yggdryl.fix import ULBRIDGE_ROWHEADER, FixCodec, FixRegistry
from yggdryl.graph import MarketData
from yggdryl.iceberg import IcebergCatalog

NAMESPACE = "record_keeping"

# The primary key every table of the pipeline declares: when a row happened,
# which object it came from, its place there and the hash of what it states -
# the instant and the content alone repeat wherever two lines are one text.
PRIMARY_KEY = ("currunix", "crosshashcode", "seqnum", "currhashcode")

# What every table of the pipeline requires of each row: its key and the code
# of its chain.
REQUIRED = (*PRIMARY_KEY, "crosscode")

# The partition every table computes for each row it is written: the quarter
# of an hour the row's instant falls in.
PARTUNIX = "time_bucket('15 minutes', currunix) as partunix"
PARTUNIX_DESCRIPTION = (
    "The quarter of an hour the row's instant falls in: currunix floored to fifteen minutes."
)

# The book stage's snapshot grid: every book whole each quarter of an hour.
QUARTER_MILLIS = 900_000

# What a table of the pipeline is created with.
TABLE_PROPERTIES = {"format-version": "3"}


def window_filter(start: dt.datetime, end: dt.datetime) -> str:
    """The `where` of one window: `currunix >= start and currunix < end`.

    Both instants state UTC; a naive instant, or one in another zone, is
    refused by name rather than read as something it is not.
    """
    for name, instant in (("start", start), ("end", end)):
        if instant.utcoffset() != dt.timedelta(0):
            raise ValueError(f"expected {name} in UTC, got {instant!r}")
    if end <= start:
        raise ValueError(f"expected a window with end after start, got {start!r} to {end!r}")
    text = lambda instant: instant.astimezone(dt.timezone.utc).isoformat().replace("+00:00", "Z")  # noqa: E731
    return f"currunix >= '{text(start)}' and currunix < '{text(end)}'"


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
    """`row` as Iceberg states it, laid out as every table of the pipeline is.

    Partitioned by `partunix` - the quarter hour the table computes for every
    row written to it - and whatever else `partition_by` names, sorted by it,
    the instant, the place within the instant and the content hash, with the
    instant, the object, the place and the hash its primary key, every
    required column non-null,
    numbered by this table alone.
    """
    schema = unnumbered(row.into_scheme_compat("iceberg")).with_partition_by(list(partition_by))
    for name in REQUIRED:
        column = schema[name]
        column.set_nullable(False)
        schema[name] = column
    partunix = schema["partunix"]
    partunix.set_description(PARTUNIX_DESCRIPTION)
    schema["partunix"] = partunix
    schema.sort.by = ["partunix", "currunix", "seqnum", "currhashcode"]
    schema = yggdryl.iceberg.assign_field_ids(schema)
    # Iceberg names a key by the identifiers of its columns, which exist once
    # the schema is numbered.
    key = sorted(schema[name].parquet_field_id for name in PRIMARY_KEY)
    schema.iceberg.update({"identifier-field-ids": ",".join(map(str, key))})
    return schema


def table_of(
    catalog: Catalog, name: str, row: Field, partition_by: Iterable[str] = (PARTUNIX,)
) -> Table:
    """The table `name` of the catalog's `record_keeping` namespace, opened
    as it is or created from `row` laid out as `declared` lays it out - the
    namespace made on the way."""
    namespace = catalog.namespaces.open_or_create(NAMESPACE)
    return namespace.tables.open_or_create(name, declared(row, partition_by), **TABLE_PROPERTIES)


def source_of(catalog: Catalog, name: str) -> Table:
    """The table `name` of the catalog's `record_keeping` namespace, which
    the stage reads from."""
    return catalog.table(f"{NAMESPACE}.{name}")


def stored_rows(table: Table, start: dt.datetime, end: dt.datetime) -> yggdryl.SerieReader:
    """The rows `table` holds inside the window, in the table's own order,
    as the row the stage wrote: the partition column the table computed
    taken off, the window pushed into the read."""
    return table.read_serie(select="* exclude (partunix)", filter=window_filter(start, end))


def instruments(silver: Catalog) -> IsinRegistry:
    """The registry of the instruments the pipeline meets, bound to
    `silver.record_keeping.instruments`: the table opened as it is or created
    from the registry's own row, unpartitioned, so the registry loads what an
    earlier run committed and commits what this run's lifecycle learns. Hand
    it to the codec (`FixCodec(..., isin_registry=...)`) and commit it after
    the lifecycle stage (`commit_instruments`)."""
    namespace = silver.namespaces.open_or_create(NAMESPACE)
    row = yggdryl.iceberg.assign_field_ids(
        unnumbered(IsinRegistry.field().into_scheme_compat("iceberg"))
    )
    table = namespace.tables.open_or_create("instruments", row, **TABLE_PROPERTIES)
    return IsinRegistry.from_url(table.url)


def commit_instruments(registry: IsinRegistry) -> IOResult:
    """What the lifecycle learned of the instruments it met, to the table the
    registry is bound to - `silver.record_keeping.instruments` - as one
    snapshot replacing every row, only where the registry moved: a run that
    learned nothing new writes nothing."""
    return registry.commit()


def parse_log_messages(
    bronze: Catalog,
    logs: IOBase | str,
    start: dt.datetime,
    end: dt.datetime,
    *,
    rowheader: str = ULBRIDGE_ROWHEADER,
) -> IOResult:
    """The capture - log objects under a glob, read through the native backend
    their location selects - to `bronze.record_keeping.log_messages`: one
    stream of text rows, the row header lifting each line's captures."""
    options = TextOptions()
    options.rowheader = rowheader
    options.timezone = "UTC"
    options.start_rownum = 1
    source = logs if isinstance(logs, IOBase) else IOBase(logs)
    lines = source.read_serie(options=options, filter=window_filter(start, end))
    return table_of(bronze, "log_messages", lines.field).overwrite_serie(lines)


def parse_fix_messages_raw(
    bronze: Catalog, codec: FixCodec, start: dt.datetime, end: dt.datetime
) -> IOResult:
    """`bronze.record_keeping.log_messages` to `bronze.record_keeping.fix_messages`
    by the parallel text-serie parse alone: no lifecycle."""
    parsed = codec.parse_text_serie(stored_rows(source_of(bronze, "log_messages"), start, end))
    return table_of(bronze, "fix_messages", parsed.field).overwrite_serie(parsed)


def parse_fix_messages_refined(
    bronze: Catalog, silver: Catalog, codec: FixCodec, start: dt.datetime, end: dt.datetime
) -> IOResult:
    """`bronze.record_keeping.fix_messages`, read in its order, walked by the
    lifecycle over sorted input, to `silver.record_keeping.fix_messages`."""
    walked = codec.lifecycle_serie(stored_rows(source_of(bronze, "fix_messages"), start, end))
    return table_of(silver, "fix_messages", walked.field).overwrite_serie(walked)


def parse_books(
    silver: Catalog, codec: FixCodec, start: dt.datetime, end: dt.datetime
) -> IOResult:
    """`silver.record_keeping.fix_messages`, read in its order, folded into
    books every quarter of an hour, to `silver.record_keeping.books` -
    partitioned by `partunix` and `cficode`, each book keyed by its
    instrument's ISIN where it holds one, and holding every event of its
    tick among its `deltas`."""
    messages = codec.messages_serie(stored_rows(source_of(silver, "fix_messages"), start, end))
    books = codec.book_serie(messages, QUARTER_MILLIS)
    table = table_of(silver, "books", books.field, (PARTUNIX, "cficode"))
    return table.overwrite_serie(books)


def _events_of_kind(
    silver: Catalog, name: str, kind: str, start: dt.datetime, end: dt.datetime
) -> IOResult:
    books = stored_rows(source_of(silver, "books"), start, end)
    rows = MarketData.deltas_serie(books, kind)
    return table_of(silver, name, rows.field).overwrite_serie(rows)


def parse_orders(silver: Catalog, start: dt.datetime, end: dt.datetime) -> IOResult:
    """The orders out of the books' deltas, laid flat, to
    `silver.record_keeping.orders`."""
    return _events_of_kind(silver, "orders", "ORDR", start, end)


def parse_quotes(silver: Catalog, start: dt.datetime, end: dt.datetime) -> IOResult:
    """The quotes out of the books' deltas, laid flat, to
    `silver.record_keeping.quotes`."""
    return _events_of_kind(silver, "quotes", "QUOT", start, end)


def parse_executions(silver: Catalog, start: dt.datetime, end: dt.datetime) -> IOResult:
    """The executions out of the books' deltas, laid flat, to
    `silver.record_keeping.executions`."""
    return _events_of_kind(silver, "executions", "EXEC", start, end)


STAGES = (
    "log_messages",
    "fix_messages",
    "instruments",
    "books",
    "orders",
    "quotes",
    "executions",
)


def run(
    bronze: Catalog,
    silver: Catalog,
    codec: FixCodec,
    logs: IOBase | str,
    start: dt.datetime,
    end: dt.datetime,
) -> dict[str, IOResult]:
    """Every stage over one window, in the diagram's order: what each wrote,
    keyed by the stage's target table under its catalog. The codec's own
    registry, where it holds one bound to a table (`instruments`), is
    committed right after the lifecycle that learned into it."""
    written = {
        "bronze.log_messages": parse_log_messages(bronze, logs, start, end),
        "bronze.fix_messages": parse_fix_messages_raw(bronze, codec, start, end),
        "silver.fix_messages": parse_fix_messages_refined(bronze, silver, codec, start, end),
    }
    registry = codec.isin_registry
    if registry is not None:
        written["silver.instruments"] = commit_instruments(registry)
    written.update(
        {
            "silver.books": parse_books(silver, codec, start, end),
            "silver.orders": parse_orders(silver, start, end),
            "silver.quotes": parse_quotes(silver, start, end),
            "silver.executions": parse_executions(silver, start, end),
        }
    )
    return written


STAGES_OF = {
    "bronze.log_messages": lambda bronze, silver, codec, logs, start, end: parse_log_messages(bronze, logs, start, end),
    "bronze.fix_messages": lambda bronze, silver, codec, logs, start, end: parse_fix_messages_raw(bronze, codec, start, end),
    "silver.fix_messages": lambda bronze, silver, codec, logs, start, end: parse_fix_messages_refined(bronze, silver, codec, start, end),
    "silver.instruments": lambda bronze, silver, codec, logs, start, end: commit_instruments(codec.isin_registry),
    "silver.books": lambda bronze, silver, codec, logs, start, end: parse_books(silver, codec, start, end),
    "silver.orders": lambda bronze, silver, codec, logs, start, end: parse_orders(silver, start, end),
    "silver.quotes": lambda bronze, silver, codec, logs, start, end: parse_quotes(silver, start, end),
    "silver.executions": lambda bronze, silver, codec, logs, start, end: parse_executions(silver, start, end),
}


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
    """The catalog `location` names: a URL - `s3tables://<bucket>` - through
    `Catalog.from_url`, a local path an Iceberg warehouse folder opened or
    created under `name`."""
    if "://" in location:
        return Catalog.from_url(location, name=name)
    return IcebergCatalog.open_or_create(name, pathlib.Path(location))


def instant(text: str) -> dt.datetime:
    """An ISO 8601 instant stating UTC - `2026-08-14T00:00:00Z`."""
    parsed = dt.datetime.fromisoformat(text.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise argparse.ArgumentTypeError(f"expected a UTC instant such as 2026-08-14T00:00:00Z, got {text!r}")
    return parsed.astimezone(dt.timezone.utc)


def report(catalogs: dict[str, Catalog]) -> None:
    """Every table of the pipeline: its rows, snapshots and files."""
    for stage in STAGES_OF:
        catalog_name, table_name = stage.split(".", 1)
        try:
            table = catalogs[catalog_name].table(f"{NAMESPACE}.{table_name}")
        except Exception as absent:  # noqa: BLE001 - a stage that wrote nothing created no table
            print(f"  {stage:<22} absent ({type(absent).__name__})")
            continue
        snapshots = len(getattr(table, "snapshots", ()))
        print(f"  {stage:<22} rows {table.row_size():>8}  snapshots {snapshots}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="The capture pipeline over two catalogs: every stage over one UTC window, streamed."
    )
    parser.add_argument("--bronze", required=True, help="the bronze catalog: s3tables://<bucket> or a warehouse folder")
    parser.add_argument("--silver", required=True, help="the silver catalog: s3tables://<bucket> or a warehouse folder")
    parser.add_argument("--logs", required=True, help="the capture: a URL or path, a glob over log objects")
    parser.add_argument("--start", required=True, type=instant, help="the window's first instant, UTC, on a quarter hour")
    parser.add_argument("--end", required=True, type=instant, help="the window's end, UTC, excluded")
    parser.add_argument("--dictionary", default=str(pathlib.Path(__file__).resolve().parents[2] / "config" / "fix"))
    parser.add_argument("--rowheader", default=ULBRIDGE_ROWHEADER)
    parser.add_argument("--threads", type=int, default=None, help="the codec's threads; the host's by default")
    parser.add_argument("--runs", type=int, default=1, help="how many times to run the window: a second run rewrites it")
    args = parser.parse_args(argv)
    # The crate logs through Python's `logging`: `YGGDRYL_LOG_LEVEL=INFO`
    # shows the identity walk (`yggdryl.aws.session`, key ids masked) and
    # every store's requests beside the stages.
    logging.basicConfig(level=os.environ.get("YGGDRYL_LOG_LEVEL", "WARNING").upper(), format="%(levelname)s %(name)s: %(message)s")

    catalogs = {"bronze": catalog_of(args.bronze, "bronze"), "silver": catalog_of(args.silver, "silver")}
    codec = FixCodec(FixRegistry.from_handle(args.dictionary), exclude_msgtypes=[], threads=args.threads)
    logs = IOBase(args.logs)
    print(f"window {args.start.isoformat()} -> {args.end.isoformat()}  resident at start {resident_bytes() >> 20} MiB")
    for run_at in range(1, args.runs + 1):
        print(f"run {run_at}")
        for stage, step in STAGES_OF.items():
            started = time.perf_counter()
            with Sampler() as sampler:
                result = step(catalogs["bronze"], catalogs["silver"], codec, logs, args.start, args.end)
            seconds = time.perf_counter() - started
            print(
                f"  {stage:<22} read {result.read_rows:>8}  wrote {result.written_rows:>8}"
                f"  skipped {result.skipped_rows:>6}  {seconds:7.2f}s  peak resident {sampler.peak >> 20:>6} MiB"
            )
        report(catalogs)
    return 0


if __name__ == "__main__":
    sys.exit(main())
