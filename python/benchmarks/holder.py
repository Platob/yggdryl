"""Gate Arrow filesystem wrapper throughput against direct PyArrow.

Run against a release extension::

    python benchmarks/holder.py --repeat 5

The default fixture is 64 MiB and both stream paths use 64 KiB chunks. Each
pair is warmed before measurement. The process exits unsuccessfully when a
wrapper median is more than 25% slower than its direct PyArrow equivalent.
"""

from __future__ import annotations

import argparse
import pathlib
import shutil
import statistics
import tempfile
import time
from collections.abc import Callable
from typing import Any

import pyarrow.fs as pafs
from yggdryl import IOBase

MIB = 1024 * 1024


def _write(stream: Any, chunk: bytes, total: int) -> int:
    written = 0
    with stream as output:
        while written < total:
            size = min(len(chunk), total - written)
            count = output.write(memoryview(chunk)[:size])
            if count != size:
                raise RuntimeError(f"short write: expected {size}, got {count}")
            written += count
    return written


def _read(stream: Any, chunk_size: int) -> int:
    read = 0
    with stream as source:
        while chunk := source.read(chunk_size):
            read += len(chunk)
    return read


def _measure_pair(
    direct: Callable[[], int], wrapper: Callable[[], int], repeat: int
) -> tuple[float, float]:
    direct()
    wrapper()
    direct_samples: list[float] = []
    wrapper_samples: list[float] = []
    for index in range(repeat):
        ordered = ((direct, direct_samples), (wrapper, wrapper_samples))
        if index % 2:
            ordered = tuple(reversed(ordered))
        for operation, samples in ordered:
            started = time.perf_counter()
            operation()
            samples.append(time.perf_counter() - started)
    return statistics.median(direct_samples), statistics.median(wrapper_samples)


def _throughput(size: int, seconds: float) -> float:
    return size / MIB / seconds


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--size-mib", type=int, default=64)
    parser.add_argument("--chunk-kib", type=int, default=64)
    parser.add_argument("--repeat", type=int, default=5)
    arguments = parser.parse_args()
    if arguments.size_mib < 64:
        parser.error("--size-mib must be at least 64")
    if arguments.chunk_kib < 1:
        parser.error("--chunk-kib must be positive")
    if arguments.repeat < 1:
        parser.error("--repeat must be positive")

    total = arguments.size_mib * MIB
    chunk_size = arguments.chunk_kib * 1024
    chunk = bytes(index % 251 for index in range(chunk_size))
    root = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-filesystem-bench-"))
    filesystem = pafs.LocalFileSystem()

    def path(name: str) -> str:
        return (root / name).as_posix()

    source = path("source.bin")
    direct_write_path = path("direct-write.bin")
    wrapper_write_path = path("wrapper-write.bin")
    direct_copy_path = path("direct-copy.bin")
    wrapper_copy_path = path("wrapper-copy.bin")
    source_handle = IOBase.from_fs(filesystem, source)

    def direct_write() -> int:
        return _write(
            filesystem.open_output_stream(
                direct_write_path, compression=None, buffer_size=chunk_size
            ),
            chunk,
            total,
        )

    def wrapper_write() -> int:
        return _write(
            IOBase.from_fs(filesystem, wrapper_write_path).open_output_stream(
                compression=None, buffer_size=chunk_size
            ),
            chunk,
            total,
        )

    def direct_read() -> int:
        return _read(
            filesystem.open_input_stream(
                source, compression=None, buffer_size=chunk_size
            ),
            chunk_size,
        )

    def wrapper_read() -> int:
        return _read(
            source_handle.open_input_stream(compression=None, buffer_size=chunk_size),
            chunk_size,
        )

    def direct_copy() -> int:
        filesystem.copy_file(source, direct_copy_path)
        return filesystem.get_file_info(direct_copy_path).size

    def wrapper_copy() -> int:
        return source_handle.copy_into(IOBase.from_fs(filesystem, wrapper_copy_path))

    try:
        _write(
            filesystem.open_output_stream(
                source, compression=None, buffer_size=chunk_size
            ),
            chunk,
            total,
        )
        benchmarks = (
            ("read", direct_read, wrapper_read),
            ("write", direct_write, wrapper_write),
            ("copy", direct_copy, wrapper_copy),
        )
        failures: list[str] = []
        print(f"payload={arguments.size_mib} MiB chunk={arguments.chunk_kib} KiB")
        print(
            f"{'operation':10} {'direct median':>16} {'wrapper median':>16} {'ratio':>8}"
        )
        for name, direct, wrapper in benchmarks:
            direct_median, wrapper_median = _measure_pair(
                direct, wrapper, arguments.repeat
            )
            ratio = wrapper_median / direct_median
            print(
                f"{name:10} {_throughput(total, direct_median):12.1f} MiB/s "
                f"{_throughput(total, wrapper_median):12.1f} MiB/s {ratio:8.3f}"
            )
            if ratio > 1.25:
                failures.append(
                    f"{name} wrapper/direct ratio {ratio:.3f} exceeds 1.250"
                )
        if failures:
            raise SystemExit("\n".join(failures))
    finally:
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
