"""Python overhead for resolving a registered path and listing a folder catalog.

Run after ``maturin develop --release`` with::

    python benchmarks/warehouse.py --iterations 10000
"""

from __future__ import annotations

import argparse
import gc
import pathlib
import statistics
import tempfile
import timeit
from collections.abc import Callable

import pyarrow as pa

from yggdryl import IOBase
from yggdryl.warehouse import FolderCatalog, MediaTable, SystemWarehouse, Warehouse

ROOT = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-warehouse-"))
# A folder catalog of four schemas of four tables each: sixteen leaves one
# listing per level reaches.
for schema in ("eu", "us", "asia", "latam"):
    (ROOT / schema).mkdir()
    for table in ("trades", "fills", "quotes", "orders"):
        IOBase(ROOT / schema / f"{table}.arrows").overwrite_arrow_table(
            pa.table({"id": range(16)})
        )
MARKET = FolderCatalog("market", ROOT)
# A memory warehouse of one registered table three levels down, and the
# same table on the process's own registry.
TRADES = MediaTable("lake.eu.trades", ROOT / "eu" / "trades.arrows", tier="hot")
WAREHOUSE = Warehouse()
WAREHOUSE.register(TRADES)
WAREHOUSE.register(MARKET)
SystemWarehouse.register(MediaTable("bench.eu.trades", ROOT / "eu" / "trades.arrows"))


def _measure(name: str, operation: Callable[[], object], iterations: int) -> None:
    samples = timeit.repeat(operation, number=iterations, repeat=7)
    nanoseconds = statistics.median(samples) * 1_000_000_000 / iterations
    print(f"{name:31} {nanoseconds:12.1f} ns/op")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--iterations", type=int, default=10_000)
    arguments = parser.parse_args()
    if arguments.iterations < 1:
        parser.error("--iterations must be positive")

    gc.disable()
    try:
        # Describing an object: the path is read once, nothing is touched.
        _measure(
            "describe media table",
            lambda: MediaTable("lake.eu.trades", ROOT / "eu" / "trades.arrows"),
            arguments.iterations,
        )
        _measure(
            "describe folder catalog",
            lambda: FolderCatalog("market", ROOT),
            arguments.iterations,
        )
        # Resolving a registered path: three memory levels, the table cloned
        # out and described as its class - the parts spelling skips the
        # grammar, so the delta is what reading the dotted text costs.
        _measure(
            "resolve registered dotted",
            lambda: WAREHOUSE.table("lake.eu.trades"),
            arguments.iterations,
        )
        _measure(
            "resolve registered parts",
            lambda: WAREHOUSE.table(["lake", "eu", "trades"]),
            arguments.iterations,
        )
        _measure(
            "resolve on system warehouse",
            lambda: SystemWarehouse.table("bench.eu.trades"),
            arguments.iterations,
        )
        # The object's own facts, off the description with no store asked.
        _measure("table properties", lambda: TRADES.properties, arguments.iterations)
        _measure("table path", lambda: TRADES.path, arguments.iterations)
        # Listing a folder catalog: one listing of the level per question,
        # the namespaces four folders and the tables sixteen leaves deep.
        _measure(
            "list catalog namespaces",
            lambda: list(MARKET.namespaces),
            arguments.iterations // 10 or 1,
        )
        _measure(
            "resolve folder table",
            lambda: MARKET.table("eu.trades"),
            arguments.iterations // 10 or 1,
        )
        _measure(
            "list namespace tables",
            lambda: list(MARKET.namespaces["eu"].tables),
            arguments.iterations // 10 or 1,
        )
        _measure(
            "folder children described",
            lambda: list(MARKET.children()),
            arguments.iterations // 10 or 1,
        )
    finally:
        gc.enable()


if __name__ == "__main__":
    main()
