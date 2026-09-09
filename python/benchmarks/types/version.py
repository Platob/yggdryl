"""Measure native Version construction, access, comparison, and Scalar crossings."""

from __future__ import annotations

import argparse
import statistics
import timeit

from yggdryl import Scalar, Version


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--iterations", type=int, default=10_000)
    count = parser.parse_args().iterations
    if count < 1:
        parser.error("--iterations must be positive")
    value = Version(5, 0, 300)
    earlier = Version(5, 0, 10)
    scalar = Scalar.from_py(value)
    for label, operation in [
        ("version/native parts", lambda: Version(5, 0, 300)),
        ("version/native parser", lambda: Version.from_str("5.0.300")),
        ("version/patch accessor", lambda: value.patch),
        ("version/numeric comparison", lambda: earlier < value),
        ("version/native stable hash", value.stable_hash),
        ("version/into Scalar", lambda: Scalar.from_py(value)),
        ("version/Scalar into Python", scalar.as_py),
    ]:
        samples = timeit.repeat(operation, number=count, repeat=5)
        print(f"{label:30} {statistics.median(samples) * 1e9 / count:10.1f} ns/op")


if __name__ == "__main__":
    main()
