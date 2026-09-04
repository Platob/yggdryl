"""Smoke-sized URI boundary benchmarks.

Run after installing a release wheel with::

    python benchmarks/uri.py --iterations 2000
"""

from __future__ import annotations

import argparse
import timeit

from yggdryl import Uri

TEXT = "https://example.com/archive/data.parquet?download=true#rows"
VALUE = Uri.from_str(TEXT)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, default=2_000)
    arguments = parser.parse_args()
    if arguments.iterations <= 0:
        parser.error("--iterations must be positive")

    cases = [
        ("parse", lambda: Uri.from_str(TEXT)),
        ("join", lambda: VALUE.joinpath("part-0.parquet")),
        ("stable hash", VALUE.stable_hash),
    ]
    for name, operation in cases:
        elapsed = timeit.timeit(operation, number=arguments.iterations)
        nanoseconds = elapsed * 1e9 / arguments.iterations
        print(f"{name:12s} {nanoseconds:9.1f} ns/op")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
