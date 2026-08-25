"""What wrapping a foreign Arrow filesystem costs, beside PyArrow's own calls.

Run after ``maturin develop --release`` with::

    python benchmarks/arrowfs.py --min-time 0.2 --repeat 7

Every row here is a *wrapper overhead* measurement. The same payload lands in
the same place twice: once through a Yggdryl handle over a
``pyarrow.fs.LocalFileSystem``, and once through PyArrow's own calls against
that same filesystem - ``open_output_stream``/``open_input_file`` for bytes and
``pyarrow.parquet`` for records. PyArrow is the trusted baseline because it is
the implementation the wrapper delegates to: every byte of transport below the
vtable was written by the Arrow project, so what these numbers isolate is the
seven-method boundary and the staging that whole-value publication requires.

The ranged-read row is the one worth reading twice. An Arrow filesystem serves
a range without fetching the whole object, and the wrapper is supposed to keep
that property rather than materialize an object to hand back a footer - so a
ranged read should cost about what PyArrow's own ``read_at`` costs, not what
reading the whole file costs.

Timings are only meaningful from a release build; a debug build measures the
compiler, not the boundary.
"""

from __future__ import annotations

import argparse
import gc
import pathlib
import platform
import shutil
import statistics
import tempfile
import timeit
from dataclasses import dataclass
from typing import Callable

import pyarrow as pa
import pyarrow.fs as pafs
import pyarrow.parquet as pq

from yggdryl import IOBase

ROW_COUNT = 65_536
BATCH_SIZE = 8_192
PAYLOAD_BYTES = 512 * 1024
RANGE_BYTES = 4_096

SCHEMA = pa.schema(
    [
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string(), nullable=False),
        pa.field("venue", pa.string(), nullable=False),
        pa.field("price", pa.float64(), nullable=False),
    ]
)

TABLE = pa.Table.from_batches(
    tuple(
        pa.record_batch(
            {
                "id": list(range(start, start + BATCH_SIZE)),
                "symbol": ["AAPL"] * BATCH_SIZE,
                "venue": ["XNAS"] * BATCH_SIZE,
                "price": [float(start)] * BATCH_SIZE,
            },
            schema=SCHEMA,
        )
        for start in range(0, ROW_COUNT, BATCH_SIZE)
    ),
    schema=SCHEMA,
)

PAYLOAD = bytes(index % 251 for index in range(PAYLOAD_BYTES))

ROOT = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-arrowfs-bench-"))
LOCAL = pafs.LocalFileSystem()


def _path(name: str) -> str:
    """The filesystem-relative spelling both sides address."""
    return (ROOT / name).as_posix()


# The fixtures both sides read. Written once, before anything is timed.
_SOURCE_BYTES = _path("source.bin")
_SOURCE_PARQUET = _path("source.parquet")


@dataclass(frozen=True)
class Benchmark:
    """One measured operation and the unit its throughput is reported in."""

    name: str
    operation: Callable[[], object]
    units: int
    #: Already plural, so the rate reads naturally without guessing a suffix.
    unit_name: str


def _wrapper_write_bytes() -> object:
    handle = IOBase.from_arrow_fs(LOCAL, _path("sink-wrapper.bin"))
    with handle:
        handle.write_bytes(PAYLOAD)
    return handle.size


def _pyarrow_write_bytes() -> object:
    # The same whole-value replacement through PyArrow's own stream.
    with LOCAL.open_output_stream(_path("sink-pyarrow.bin")) as sink:
        sink.write(PAYLOAD)
    return len(PAYLOAD)


def _wrapper_read_bytes() -> object:
    return len(IOBase.from_arrow_fs(LOCAL, _SOURCE_BYTES).read_bytes())


def _pyarrow_read_bytes() -> object:
    with LOCAL.open_input_file(_SOURCE_BYTES) as source:
        return len(source.readall())


def _wrapper_read_range() -> object:
    handle = IOBase.from_arrow_fs(LOCAL, _SOURCE_BYTES)
    return len(handle.pread(PAYLOAD_BYTES - RANGE_BYTES, RANGE_BYTES))


def _pyarrow_read_range() -> object:
    with LOCAL.open_input_file(_SOURCE_BYTES) as source:
        return len(source.read_at(RANGE_BYTES, PAYLOAD_BYTES - RANGE_BYTES))


def _wrapper_write_parquet() -> object:
    handle = IOBase.from_arrow_fs(LOCAL, _path("sink-wrapper.parquet"))
    with handle:
        handle.overwrite_arrow_table(TABLE)
    return handle.size


def _pyarrow_write_parquet() -> object:
    pq.write_table(TABLE, _path("sink-pyarrow.parquet"), filesystem=LOCAL)
    return ROW_COUNT


def _wrapper_read_parquet() -> object:
    handle = IOBase.from_arrow_fs(LOCAL, _SOURCE_PARQUET)
    return handle.read_arrow_reader().read_all().num_rows


def _pyarrow_read_parquet() -> object:
    return pq.read_table(_SOURCE_PARQUET, filesystem=LOCAL).num_rows


def _wrapper_list() -> object:
    return len(IOBase.from_arrow_fs(LOCAL, _path("lake")).ls(recursive=True))


def _pyarrow_list() -> object:
    selector = pafs.FileSelector(_path("lake"), recursive=True)
    return len(LOCAL.get_file_info(selector))


BENCHMARKS = (
    Benchmark("bytes write wrapper", _wrapper_write_bytes, PAYLOAD_BYTES, "bytes"),
    Benchmark("bytes write PyArrow", _pyarrow_write_bytes, PAYLOAD_BYTES, "bytes"),
    Benchmark("bytes read wrapper", _wrapper_read_bytes, PAYLOAD_BYTES, "bytes"),
    Benchmark("bytes read PyArrow", _pyarrow_read_bytes, PAYLOAD_BYTES, "bytes"),
    Benchmark("range read wrapper", _wrapper_read_range, RANGE_BYTES, "bytes"),
    Benchmark("range read PyArrow", _pyarrow_read_range, RANGE_BYTES, "bytes"),
    Benchmark("parquet write wrapper", _wrapper_write_parquet, ROW_COUNT, "rows"),
    Benchmark("parquet write PyArrow", _pyarrow_write_parquet, ROW_COUNT, "rows"),
    Benchmark("parquet read wrapper", _wrapper_read_parquet, ROW_COUNT, "rows"),
    Benchmark("parquet read PyArrow", _pyarrow_read_parquet, ROW_COUNT, "rows"),
    Benchmark("listing wrapper", _wrapper_list, 16, "entries"),
    Benchmark("listing PyArrow", _pyarrow_list, 16, "entries"),
)


def _measure(
    benchmark: Benchmark,
    *,
    minimum_seconds: float,
    repeat: int,
) -> tuple[float, float, int]:
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


def _rate(units: int, seconds: float, unit_name: str) -> str:
    return f"{units / seconds:,.0f} {unit_name}/s"


def _seed() -> None:
    """Write every fixture both sides read, before anything is timed."""
    with LOCAL.open_output_stream(_SOURCE_BYTES) as sink:
        sink.write(PAYLOAD)
    pq.write_table(TABLE, _SOURCE_PARQUET, filesystem=LOCAL)
    for year in ("2024", "2025"):
        for month in ("01", "02"):
            leaf = ROOT / "lake" / f"year={year}" / f"month={month}"
            leaf.mkdir(parents=True, exist_ok=True)
            for part in range(2):
                (leaf / f"part-{part}.parquet").write_bytes(b"PAR1")


def main() -> None:
    """Seed the fixtures, then time each operation against its baseline."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--min-time", type=float, default=0.2)
    parser.add_argument("--repeat", type=int, default=7)
    arguments = parser.parse_args()
    if arguments.min_time <= 0:
        parser.error("--min-time must be greater than zero")
    if arguments.repeat < 1:
        parser.error("--repeat must be positive")

    try:
        _seed()

        # A wrapper that stopped reading ranges would still be correct and
        # would still look fast enough, so the range read is asserted to be a
        # range read before it is timed.
        handle = IOBase.from_arrow_fs(LOCAL, _SOURCE_BYTES)
        assert len(handle.pread(PAYLOAD_BYTES - RANGE_BYTES, RANGE_BYTES)) == RANGE_BYTES
        assert handle.read_bytes() == PAYLOAD
        assert _wrapper_read_parquet() == _pyarrow_read_parquet()

        print(
            f"Python {platform.python_version()}, PyArrow {pa.__version__}; "
            f"{ROW_COUNT:,} rows, {PAYLOAD_BYTES:,} payload bytes"
        )
        print(f"{'benchmark':32} {'median':>12} {'best':>12} {'throughput':>20}")
        print("-" * 80)
        for benchmark in BENCHMARKS:
            median, best, _ = _measure(
                benchmark,
                minimum_seconds=arguments.min_time,
                repeat=arguments.repeat,
            )
            print(
                f"{benchmark.name:32} {median * 1e6:>10.2f}us {best * 1e6:>10.2f}us "
                f"{_rate(benchmark.units, median, benchmark.unit_name):>20}"
            )
    finally:
        shutil.rmtree(ROOT, ignore_errors=True)


if __name__ == "__main__":
    main()
