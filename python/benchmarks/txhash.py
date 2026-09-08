"""The coupling beside the digest it wraps, through the Python boundary.

Every row pairs a coupled answer with the plain digest answer for the same
bytes or rows, so the difference is the coupling plus this boundary's
conversion cost: reading the instant, laying it beside the digest, and for
the Arrow rows the ``fixed_size_binary`` column built per batch.

Usage:
    python benchmarks/txhash.py [--min-time 0.2] [--repeat 5]
"""

from __future__ import annotations

import argparse
import datetime as dt
import statistics
import sys
import timeit

import pyarrow as pa

from yggdryl import Field, Scalar, txhash, xxhash

PAYLOAD = b'{"id": 1234567, "venue": "XNAS", "price": "150.2500"}\n' * 20_000
INSTANT = 1_700_000_000_000_000

#: Where a call's fixed cost, the size branches, and the kernel dominate.
SIZES = [16, 240, 4096, 64 * 1024]

#: Large enough for per-row work to dominate one Python call.
BATCH_ROWS = 4_096


def _measure(label: str, callable_, size: int, min_time: float, repeat: int) -> None:
    timer = timeit.Timer(callable_)
    number, _ = timer.autorange()
    while timer.timeit(number) < min_time:
        number *= 2
    samples = [timer.timeit(number) / number for _ in range(repeat)]
    median = statistics.median(samples)
    throughput = size / median / (1000 * 1000 * 1000)
    print(f"{label:46s} {median * 1e9:12.1f} ns {throughput:8.2f} GB/s")


def _measure_rows(label: str, callable_, rows: int, min_time: float, repeat: int) -> None:
    timer = timeit.Timer(callable_)
    number, _ = timer.autorange()
    while timer.timeit(number) < min_time:
        number *= 2
    samples = [timer.timeit(number) / number for _ in range(repeat)]
    median = statistics.median(samples)
    throughput = rows / median / 1_000_000
    print(f"{label:46s} {median * 1e6:12.1f} us {throughput:8.2f} M row/s")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--min-time", type=float, default=0.2)
    parser.add_argument("--repeat", type=int, default=5)
    arguments = parser.parse_args()

    print(
        f"Python {sys.version.split()[0]}; median of {arguments.repeat}, "
        f"min {arguments.min_time}s per sample"
    )

    for size in SIZES:
        data = PAYLOAD[:size]
        _measure(f"xxh3  {size:9d} B", lambda: xxhash.xxh3(data), size, arguments.min_time, arguments.repeat)
        _measure(f"txh3  {size:9d} B", lambda: txhash.txh3(data, INSTANT), size, arguments.min_time, arguments.repeat)

    aware = dt.datetime(2023, 11, 14, 22, 13, 20, tzinfo=dt.timezone.utc)
    hasher = txhash.TxHasher(seed=7)
    row = Scalar.from_py([1_234_567, "XNAS", 150.25, "AAPL"])
    _measure("txh3 240 B (datetime instant)", lambda: txhash.txh3(PAYLOAD[:240], aware), 240, arguments.min_time, arguments.repeat)
    _measure("hasher.digest 240 B", lambda: hasher.digest(PAYLOAD[:240], INSTANT), 240, arguments.min_time, arguments.repeat)
    _measure("hasher.digest_scalar (four-column row)", lambda: hasher.digest_scalar(row, INSTANT), 1, arguments.min_time, arguments.repeat)
    _measure("Scalar.digest (four-column row)", lambda: row.digest(), 1, arguments.min_time, arguments.repeat)
    value = txhash.txh3(PAYLOAD[:240], INSTANT)
    spelled = str(value)
    raw = bytes(value)
    _measure("bytes(value)", lambda: bytes(value), 16, arguments.min_time, arguments.repeat)
    _measure("TxHash.from_bytes", lambda: txhash.TxHash.from_bytes("us", "xxh3-64", raw), 16, arguments.min_time, arguments.repeat)
    _measure("TxHash(str)", lambda: txhash.TxHash(spelled), 16, arguments.min_time, arguments.repeat)
    _measure("unix_of(datetime)", lambda: txhash.unix_of(aware), 1, arguments.min_time, arguments.repeat)

    batch = pa.record_batch(
        {
            "event": pa.array([INSTANT + index for index in range(BATCH_ROWS)], pa.timestamp("us", tz="UTC")),
            "symbol": pa.array(["AAPL" if index % 2 == 0 else "MSFT" for index in range(BATCH_ROWS)]),
            "price": pa.array([187.23 + index / 100 for index in range(BATCH_ROWS)]),
        }
    )
    times = batch.column("event")
    digests = xxhash.row_digests(batch)
    coupled = txhash.row_txhashes(batch, times)
    _measure_rows("xxhash.row_digests", lambda: xxhash.row_digests(batch), BATCH_ROWS, arguments.min_time, arguments.repeat)
    _measure_rows("txhash.row_txhashes", lambda: txhash.row_txhashes(batch, times), BATCH_ROWS, arguments.min_time, arguments.repeat)
    _measure_rows("txhash.compose", lambda: txhash.compose(times, digests), BATCH_ROWS, arguments.min_time, arguments.repeat)
    _measure_rows("txhash.decompose", lambda: txhash.decompose(coupled), BATCH_ROWS, arguments.min_time, arguments.repeat)
    _measure_rows("txhash.unix_array", lambda: txhash.unix_array(times), BATCH_ROWS, arguments.min_time, arguments.repeat)

    holder = Field("key", "fixed_size_binary[16]", nullable=False)
    holder.digest.set_holder()
    holder.digest.time = "event"
    root = Field(
        "row",
        yggdryl_fields(batch, holder),
        nullable=False,
    )
    plain = Field("row_digest", "uint64", nullable=False)
    plain.digest.set_holder()
    plain_root = Field("row", yggdryl_fields(batch, plain), nullable=False)
    _measure_rows("plain holder apply_arrow_batch", lambda: plain_root.digest.apply_arrow_batch(batch), BATCH_ROWS, arguments.min_time, arguments.repeat)
    _measure_rows("coupled holder apply_arrow_batch", lambda: root.digest.apply_arrow_batch(batch), BATCH_ROWS, arguments.min_time, arguments.repeat)
    return 0


def yggdryl_fields(batch: pa.RecordBatch, holder: Field):
    """The batch's own fields plus one holder, as a Struct datatype."""
    from yggdryl import DataType

    fields = [Field.from_arrow(field) for field in batch.schema]
    return DataType.from_fields([*fields, holder])


if __name__ == "__main__":
    raise SystemExit(main())
