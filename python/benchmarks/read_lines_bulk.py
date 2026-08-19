"""Time the Arrow line projection over a bulk rotated log corpus.

Run from ``python/`` against a **release** wheel (a debug build understates
the native side by an order of magnitude)::

    .venv/Scripts/python benchmarks/read_lines_bulk.py --min-time 0.2 --repeat 3

``benchmarks/read_lines.py`` answers "how fast is the projection against the
``re`` loop a Python engineer would write". This target answers a different
question and carries no plain-Python baseline: what the *production shape*
costs - a folder of rotated ``.log.gz`` leaves, 200k anonymized OMS records,
read as a stream. Every case below is one claim:

``read folder gzip``
    Throughput over the production shape: eight ``app-N.log.gz`` leaves read
    in name-sorted order, each decoded by its own media type, each restarting
    ``rownum`` at 1, no batch spanning two leaves. This is the headline number.
``read folder plain``
    The same records with the leaves uncompressed. Against the row above, the
    difference is the *net* cost of storing the corpus gzip-coded on local
    storage, not the inflate in isolation: the plain leaves move the whole
    decoded payload off disk while the coded ones move about a fifth of it and
    then inflate, and those two effects pull in opposite directions. That net
    is the trade a production reader actually makes. For inflate alone both
    sides must move identical source bytes, which is what the Rust
    ``lines_arrow/parse/{plain,gzip}`` pair measures with in-memory handles.
``read single gzip``
    The same records in one ``app.log.gz``. Against ``read folder gzip``, the
    difference is exactly what the folder shape costs: per-leaf open, media
    type, and batch boundaries.
``read folder utf8``
    The same drain of the same gzip folder with ``capture_types`` declaring
    ``thread_id`` and ``latency_us`` as ``utf8``, which turns the strict
    native cast off. Only the cast differs from ``read folder gzip``, and
    both rows only count rows, so *that* difference is what typing two
    captures on every record costs - 2 x RECORDS values. This is the same
    pairing the Rust ``lines_gzip/casts/{typed,text}`` cases make.
``typed accessors``
    Parse *and aggregate*. ``(?<thread_id>\\d+)`` and ``(?<latency_us>\\d+)``
    infer ``int64`` from the closed inference table, so the aggregate is
    ``pyarrow.compute`` over already-typed columns with no Python-side
    conversion at all. Minus ``read folder gzip``, that is the aggregation.
``text captures + py cast``
    The identical aggregate over the ``utf8`` parse, so the conversion falls
    to the consumer. Minus ``read folder utf8``, that is the aggregation plus
    the Python-side conversion.

    Read these two aggregate rows against *their own* parse row, never
    against each other: the two sides do not convert the same volume. The
    native cast types every record's two captures (2,000,000 values), while
    the consumer converts only the rows that survive the ``level`` filter
    (about a third of them). Subtracting one aggregate row from the other
    would compare a whole-corpus cast against a filtered one and call the
    difference a verdict. The consumer side uses ``pyarrow.compute.cast``,
    the fastest conversion a Python caller has - C-speed and vectorized - so
    its cost is a floor; an ``int()`` loop over ``to_pylist()`` is far worse.

Then a scale sweep over 1/8, 1/4, 1/2 and 1/1 of the corpus. The claim there
is not a speed but a *shape*: throughput per decoded byte stays flat as the
corpus grows ~8x, so nothing in the read is quadratic.

Throughput is reported in **decoded** bytes - what the parser actually
consumes - and in rows/s. The gzip wire size is printed once in the header and
never used as a throughput denominator: a compressed byte is not a parsed
byte, and reporting it as one would inflate the gzip rows by the compression
ratio.

Peak memory
-----------

``--measure-memory`` is the strongest claim a bulk benchmark can make, and it
needs its own process per corpus size: ``resource.getrusage(RUSAGE_SELF)``
reports ``ru_maxrss``, a high-water mark for the *whole process lifetime*, so
one process measuring four sizes would report the largest four times. Each
size therefore re-execs this script with a hidden ``--rss-probe <records>``
flag and the parent reports each child's peak beside its decoded corpus size.
A ``--rss-probe 0`` child does no work at all and reports the interpreter and
PyArrow import floor, so the reader can subtract it. Memory staying roughly
flat while the corpus grows ~8x is the streaming contract - one batch in
memory at a time, content codings decoded as streams - measured rather than
asserted.

The **parent** writes each probed corpus and passes its path down; the child
only reads it. That split is what makes the column mean what it says: a
high-water mark covers the whole process lifetime, so a child that generated
its own fixture would fold the generator's peak into the number attributed to
the streaming read.

``ru_maxrss`` is in **kilobytes on Linux** and in **bytes on macOS**; the
probe normalizes with a ``sys.platform`` check before reporting.

The probe is launched through a deliberately lean trampoline interpreter
(``python -I -S -c``) rather than spawned straight from this process, and that
indirection is load-bearing, not ceremony. On Linux a forked child's ``mm``
starts as a copy-on-write image of its parent's, so its high-water mark is
already the parent's resident size when ``execve`` folds it into the new
process's accounting: a probe spawned directly from a parent holding PyArrow
and the fixtures reports the *parent's* ~120 MiB no matter how little it uses,
which is a floor masquerading as a measurement. ``signal->maxrss`` is not
inherited, only the mm's high-water is, so putting a ~10 MiB interpreter in
between gives each probe an honest number of its own.

The corpus specification lives in ``benchmarks/_corpus.py`` and is shared
byte-for-byte with the Rust and Node targets of the same name, so the three
languages' numbers are comparable. Do not "improve" it. Every 50th record is a
three-line stack trace that the projection folds into a single row, which is
why the row-count assertion below is load-bearing: a parser that split those
into three rows, or dropped the continuations, would still look fast.
"""

from __future__ import annotations

import argparse
import gc
import gzip
import json
import pathlib
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import timeit
from dataclasses import dataclass
from typing import Callable, Sequence

import pyarrow as pa
import pyarrow.compute as pc

from yggdryl import IOBase

try:  # `resource` is Unix-only; only --measure-memory needs it.
    import resource
except ImportError:  # pragma: no cover - Windows
    resource = None  # type: ignore[assignment]

RECORDS = 200_000
LEAVES = 8

# Lines generated per write, so building a fixture costs constant memory
# whatever the record count. The --rss-probe child builds its own corpus, and
# a builder whose footprint grew with the corpus would poison the very
# high-water mark the probe exists to report.
CHUNK = 4_096

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from _corpus import PATTERN, line as _shared_line  # noqa: E402

# The capture that infers int64 off its own `\d+` sub-pattern, declared back
# to utf8 for the "what does the typed accessor cost" row.
TEXT_CAPTURES = {"port": "utf8"}

# The level the aggregate filters on; one of the three the generator cycles.
ERROR_LEVEL = "WARNING"

# Level 6 is the DEFLATE default and what the crate's own gzip writes, so the
# wire the parser decodes here is the wire it decodes in production.
COMPRESS_LEVEL = 6

# The scale sweep's denominators. Labels are `scale 1/N`, not record counts,
# so a row keeps its identity when --records changes.
SWEEP = (8, 4, 2, 1)

# The lean interpreter each --rss-probe child is launched through. A child
# forked from this process would inherit this process's resident set as its own
# high-water mark; forked from ~10 MiB of trampoline it reports what it really
# used. It relays both streams and the exit status so a failing probe is not
# silently read as an empty one.
TRAMPOLINE = (
    "import subprocess, sys\n"
    "done = subprocess.run(sys.argv[1:], capture_output=True, text=True)\n"
    "sys.stderr.write(done.stderr)\n"
    "sys.stdout.write(done.stdout)\n"
    "raise SystemExit(done.returncode)\n"
)

ROOT = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-log-bulk-"))


def _line(index: int) -> str:
    """Render record ``index`` of the shared corpus specification.

    Every fiftieth record is a three-line record the projection folds into
    one row with ``lines=3``; a naive splitlines loop would miscount it.
    """
    return _shared_line(index, continuations=True)


@dataclass(frozen=True)
class Corpus:
    """One written fixture and the byte count the parser will consume."""

    path: pathlib.Path
    records: int
    decoded_bytes: int
    wire_bytes: int


@dataclass(frozen=True)
class Benchmark:
    """One measured operation and the units its throughput is reported in."""

    name: str
    operation: Callable[[], object]
    rows: int
    decoded_bytes: int


def _write_leaf(target: pathlib.Path, start: int, stop: int, *, coded: bool) -> int:
    """Write records ``start``..``stop`` to ``target``, returning decoded bytes."""
    decoded = 0
    with target.open("wb") as raw:
        stream = (
            gzip.GzipFile(fileobj=raw, mode="wb", compresslevel=COMPRESS_LEVEL, mtime=0)
            if coded
            else raw
        )
        try:
            for chunk in range(start, stop, CHUNK):
                payload = "".join(
                    _line(index) for index in range(chunk, min(chunk + CHUNK, stop))
                ).encode()
                decoded += len(payload)
                stream.write(payload)
        finally:
            if coded:
                stream.close()
    return decoded


def _wire_bytes(path: pathlib.Path) -> int:
    """The bytes actually on disk - never a throughput denominator."""
    if path.is_dir():
        return sum(leaf.stat().st_size for leaf in path.iterdir())
    return path.stat().st_size


def _folder(root: pathlib.Path, name: str, records: int, *, coded: bool) -> Corpus:
    """Write ``records`` across ``LEAVES`` rotated leaves in one folder."""
    folder = root / name
    folder.mkdir(parents=True, exist_ok=True)
    suffix = ".log.gz" if coded else ".log"
    per_leaf = records // LEAVES
    decoded = 0
    for leaf in range(LEAVES):
        decoded += _write_leaf(
            folder / f"app-{leaf}{suffix}",
            leaf * per_leaf,
            (leaf + 1) * per_leaf,
            coded=coded,
        )
    return Corpus(folder, records, decoded, _wire_bytes(folder))


def _single(root: pathlib.Path, name: str, records: int) -> Corpus:
    """Write every record into one gzip-coded leaf."""
    folder = root / name
    folder.mkdir(parents=True, exist_ok=True)
    target = folder / "app.log.gz"
    decoded = _write_leaf(target, 0, records, coded=True)
    return Corpus(target, records, decoded, _wire_bytes(target))


def _read_rows(corpus: Corpus, *, text: bool = False) -> int:
    """Stream the whole projection, counting rows and holding no batch.

    With ``text`` true the two numeric captures are declared ``utf8``, which
    turns the strict native cast off. Nothing else differs, which is what
    makes the pair of timings a measurement of the cast alone.
    """
    capture_types = TEXT_CAPTURES if text else None
    rows = 0
    for batch in IOBase(corpus.path).read_arrow_lines(PATTERN, capture_types=capture_types):
        rows += batch.num_rows
    return rows


def _aggregate(corpus: Corpus, *, text: bool) -> tuple[int, int]:
    """Filter on ``level`` and total the ``port`` capture, batch by batch.

    With ``text`` false the capture arrives as ``int64`` from the native cast
    and ``pyarrow.compute`` reads it directly. With ``text`` true it is
    declared ``utf8``, the native cast is off, and the consumer pays for the
    conversion here. Both branches compute the identical numbers, which is
    what makes the pair of timings a comparison and not two unrelated
    measurements.
    """
    capture_types = TEXT_CAPTURES if text else None
    reader = IOBase(corpus.path).read_arrow_lines(PATTERN, capture_types=capture_types)
    rows = 0
    ports = 0
    for batch in reader:
        kept = batch.filter(pc.equal(batch.column("level"), ERROR_LEVEL))
        if kept.num_rows == 0:
            continue
        port_column = kept.column("port")
        if text:
            port_column = pc.cast(port_column, pa.int64())
        rows += kept.num_rows
        ports += pc.sum(port_column).as_py()
    return rows, ports


def _measure(
    benchmark: Benchmark,
    *,
    minimum_seconds: float,
    repeat: int,
) -> tuple[float, float, int]:
    """Warm up once, size the loop, then report median and best per operation."""
    benchmark.operation()
    number = 1
    while number < 4_096:
        if timeit.timeit(benchmark.operation, number=number) >= minimum_seconds:
            break
        number *= 2
    gc.collect()
    samples = timeit.repeat(benchmark.operation, number=number, repeat=repeat)
    per_operation = [sample / number for sample in samples]
    return statistics.median(per_operation), min(per_operation), number


def _sweep_sizes(records: int) -> tuple[tuple[str, int], ...]:
    """Label and record count for each sweep step, largest reusing the corpus.

    A step whose exact fraction would round below one record per leaf is
    dropped rather than clamped up: three rows labelled 1/8, 1/4 and 1/2 that
    all read the same floor corpus would be three labels for one measurement.
    """
    steps = []
    for denominator in SWEEP:
        size = records // denominator // LEAVES * LEAVES
        if size >= LEAVES:
            steps.append((f"scale 1/{denominator}", size))
    return tuple(steps)


def _mib(count: int) -> float:
    return count / 1_048_576


def _peak_rss_bytes() -> int:
    """``ru_maxrss`` normalized to bytes - kilobytes on Linux, bytes on macOS."""
    assert resource is not None
    peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return peak if sys.platform == "darwin" else peak * 1_024


def _rss_probe(records: int, source: str | None) -> None:
    """Read one already-written corpus and report this process's peak RSS.

    Runs as a child of ``--measure-memory``. ``records`` of zero reads nothing
    and reports the interpreter and PyArrow import floor. The corpus is
    written by the *parent* and named by ``source``, never built here:
    ``ru_maxrss`` is a high-water mark for the whole process, so generating a
    fixture in this process would fold the generator's own peak into the
    number this column attributes to the streaming read.
    """
    rows = 0
    if records and source is not None:
        rows = _read_rows(Corpus(pathlib.Path(source), records, 0, 0))
        assert rows == records, f"expected {records} rows, projected {rows}"
    print(
        json.dumps(
            {
                "records": records,
                "rows": rows,
                "peak_rss_bytes": _peak_rss_bytes(),
            }
        )
    )


def _run_probe(records: int, source: pathlib.Path | None = None) -> dict[str, int]:
    """Re-exec this script to read one written corpus and report its peak."""
    command = [
        sys.executable,
        "-I",
        "-S",
        "-c",
        TRAMPOLINE,
        sys.executable,
        str(pathlib.Path(__file__).resolve()),
        "--rss-probe",
        str(records),
    ]
    if source is not None:
        command += ["--rss-probe-path", str(source)]
    completed = subprocess.run(command, capture_output=True, text=True)
    if completed.returncode:
        raise RuntimeError(f"rss probe for {records} records failed:\n{completed.stderr}")
    return json.loads(completed.stdout.strip().splitlines()[-1])


def _print_table(
    benchmarks: Sequence[Benchmark],
    *,
    minimum_seconds: float,
    repeat: int,
) -> None:
    """Time every benchmark with the collector off and print one row each."""
    print(
        f"{'benchmark':26} {'median':>12} {'best':>12} "
        f"{'rows/s':>14} {'MiB/s decoded':>15}"
    )
    print("-" * 84)
    gc.disable()
    try:
        for benchmark in benchmarks:
            median, best, iterations = _measure(
                benchmark,
                minimum_seconds=minimum_seconds,
                repeat=repeat,
            )
            print(
                f"{benchmark.name:26} "
                f"{median * 1_000:9.3f} ms "
                f"{best * 1_000:9.3f} ms "
                f"{benchmark.rows / median:14,.0f} "
                f"{_mib(benchmark.decoded_bytes) / median:15.1f} "
                f"({iterations} iterations)"
            )
    finally:
        gc.enable()


def main() -> None:
    """Write every fixture, assert the row counts, then time the cases."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--records", type=int, default=RECORDS)
    parser.add_argument("--min-time", type=float, default=0.2)
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--measure-memory", action="store_true")
    parser.add_argument("--rss-probe", type=int, default=None, help=argparse.SUPPRESS)
    parser.add_argument("--rss-probe-path", default=None, help=argparse.SUPPRESS)
    arguments = parser.parse_args()
    if arguments.min_time <= 0:
        parser.error("--min-time must be greater than zero")
    if arguments.repeat < 1:
        parser.error("--repeat must be positive")
    if arguments.rss_probe is None:
        if arguments.records <= 0:
            parser.error("--records must be greater than zero")
        if arguments.records % LEAVES:
            parser.error(f"--records must be a multiple of {LEAVES} rotated leaves")
    if (arguments.measure_memory or arguments.rss_probe is not None) and resource is None:
        parser.error("--measure-memory needs the Unix `resource` module")

    records = arguments.records
    try:
        if arguments.rss_probe is not None:
            _rss_probe(arguments.rss_probe, arguments.rss_probe_path)
            return

        folder_gzip = _folder(ROOT, "folder-gzip", records, coded=True)
        folder_plain = _folder(ROOT, "folder-plain", records, coded=False)
        single_gzip = _single(ROOT, "single-gzip", records)

        # A parser that split the three-line records, or dropped their
        # continuations, would still look fast - so every shape is asserted to
        # project exactly `records` rows before anything is timed.
        for corpus in (folder_gzip, folder_plain, single_gzip):
            rows = _read_rows(corpus)
            assert rows == records, f"{corpus.path.name}: {rows} rows, want {records}"
        # And the typed and text branches must agree on every number, or the
        # pair below would be timing two different jobs.
        typed_total = _aggregate(folder_gzip, text=False)
        text_total = _aggregate(folder_gzip, text=True)
        assert typed_total == text_total, f"{typed_total} != {text_total}"
        assert typed_total[0] == records // 3, typed_total

        sweep = []
        for label, size in _sweep_sizes(records):
            sweep.append(
                (
                    label,
                    folder_gzip
                    if size == records
                    else _folder(ROOT, f"sweep-{size}", size, coded=True),
                )
            )

        print(
            f"Python {platform.python_version()}, PyArrow {pa.__version__}; "
            f"{records:,} records over {LEAVES} rotated leaves, "
            f"{folder_gzip.decoded_bytes:,} decoded bytes "
            f"({folder_gzip.wire_bytes:,} gzip wire bytes, not a parse denominator)"
        )
        print(
            f"aggregate: {typed_total[0]:,} rows at level {ERROR_LEVEL!r}, "
            f"port total {typed_total[1]:,}"
        )
        print()

        benchmarks = (
            Benchmark(
                "read folder gzip",
                lambda: _read_rows(folder_gzip),
                records,
                folder_gzip.decoded_bytes,
            ),
            Benchmark(
                "read folder plain",
                lambda: _read_rows(folder_plain),
                records,
                folder_plain.decoded_bytes,
            ),
            Benchmark(
                "read single gzip",
                lambda: _read_rows(single_gzip),
                records,
                single_gzip.decoded_bytes,
            ),
            Benchmark(
                "read folder utf8",
                lambda: _read_rows(folder_gzip, text=True),
                records,
                folder_gzip.decoded_bytes,
            ),
            Benchmark(
                "typed accessors",
                lambda: _aggregate(folder_gzip, text=False),
                records,
                folder_gzip.decoded_bytes,
            ),
            Benchmark(
                "text captures + py cast",
                lambda: _aggregate(folder_gzip, text=True),
                records,
                folder_gzip.decoded_bytes,
            ),
        )
        _print_table(
            benchmarks,
            minimum_seconds=arguments.min_time,
            repeat=arguments.repeat,
        )

        print()
        print("scale sweep (gzip folder, one claim: MiB/s decoded stays flat)")
        _print_table(
            tuple(
                Benchmark(
                    f"{label} ({corpus.records:,} rec)",
                    lambda corpus=corpus: _read_rows(corpus),
                    corpus.records,
                    corpus.decoded_bytes,
                )
                for label, corpus in sweep
            ),
            minimum_seconds=arguments.min_time,
            repeat=arguments.repeat,
        )

        if arguments.measure_memory:
            print()
            print(
                "peak RSS per corpus size, each in its own process behind a lean "
                "trampoline, reading a corpus this process already wrote "
                "(ru_maxrss is a whole-process high-water mark, so the child "
                "must not build what it measures reading)"
            )
            print(
                f"{'probe':26} {'records':>12} {'decoded MiB':>14} "
                f"{'peak RSS MiB':>14} {'over floor MiB':>16}"
            )
            print("-" * 84)
            floor = _run_probe(0)["peak_rss_bytes"]
            print(
                f"{'floor (no corpus)':26} {0:>12} {0.0:>14.1f} "
                f"{_mib(floor):>14.1f} {0.0:>16.1f}"
            )
            # The memory sweep continues past the timing corpus, because the
            # claim is a *plateau*: residency stops growing once the corpus is
            # comfortably larger than the batch budget, and a corpus of only a
            # few batches cannot show that by itself.
            probes = list(_sweep_sizes(records)) + [
                (f"scale {factor}/1", records * factor) for factor in (2, 4)
            ]
            for label, size in probes:
                probed = _folder(ROOT, f"rss-{size}", size, coded=True)
                report = _run_probe(size, probed.path)
                assert report["rows"] == size, report
                print(
                    f"{label:26} {report['records']:>12,} "
                    f"{_mib(probed.decoded_bytes):>14.1f} "
                    f"{_mib(report['peak_rss_bytes']):>14.1f} "
                    f"{_mib(report['peak_rss_bytes'] - floor):>16.1f}"
                )
    finally:
        shutil.rmtree(ROOT, ignore_errors=True)


if __name__ == "__main__":
    main()
