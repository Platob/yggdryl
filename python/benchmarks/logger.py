"""Smoke-sized logging boundary benchmarks, each beside the stdlib's own.

Named `logger.py` because a script named `logging.py` would shadow the
standard library module it measures against.

Run after installing a release wheel with::

    python benchmarks/logger.py --iterations 20000
"""

from __future__ import annotations

import argparse
import logging
import tempfile
import timeit
from pathlib import Path

from yggdryl.logging import Deduplicate, FileHandler, TerminalFormatter

RECORD = logging.LogRecord(
    "trades.feed", logging.INFO, __file__, 42, "opened %d venues", (3,), None, "open"
)


def _logger(name: str, handler: logging.Handler) -> logging.Logger:
    logger = logging.getLogger(name)
    logger.propagate = False
    logger.setLevel(logging.INFO)
    logger.addHandler(handler)
    return logger


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, default=20_000)
    arguments = parser.parse_args()
    if arguments.iterations <= 0:
        parser.error("--iterations must be positive")

    with tempfile.TemporaryDirectory() as folder:
        native = FileHandler(Path(folder, "native.log"), capacity=1 << 16)
        native.setFormatter(logging.Formatter(logging.BASIC_FORMAT))
        stdlib = logging.FileHandler(Path(folder, "stdlib.log"))
        stdlib.setFormatter(logging.Formatter(logging.BASIC_FORMAT))
        basic = logging.Formatter(logging.BASIC_FORMAT)
        terminal = TerminalFormatter()
        to_stdlib = _logger("bench.stdlib", stdlib)
        to_native = _logger("bench.native", native)
        # On the logger: `logging.NullHandler.handle` runs no filter.
        repeated = _logger("bench.repeated", logging.NullHandler())
        repeated.addFilter(Deduplicate())
        cases = [
            # The stdlib's format of one record, then the core's terminal one.
            ("logging.Formatter.format", lambda: basic.format(RECORD)),
            ("TerminalFormatter.format", lambda: terminal.format(RECORD)),
            # One record through a handler writing to a file, held under 64 KiB by
            # the core's handler, written at once by the stdlib's.
            ("logging.FileHandler", lambda: to_stdlib.info("opened 3 venues")),
            ("FileHandler (64 KiB)", lambda: to_native.info("opened 3 venues")),
            # A repeat counted by its hash and dropped by the filter.
            ("Deduplicate repeat", lambda: repeated.info("opened 3 venues")),
        ]
        for name, operation in cases:
            operation()
            elapsed = timeit.timeit(operation, number=arguments.iterations)
            nanoseconds = elapsed * 1e9 / arguments.iterations
            print(f"{name:26s} {nanoseconds:9.1f} ns/op")
        native.close()
        stdlib.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
