"""Excel reads and writes beside openpyxl, on identical workbooks.

Run after ``maturin develop --release`` with::

    python benchmarks/media/excel.py --repeat 5

Every read case writes one workbook with this crate and reads that same file
two ways: ``openpyxl.load_workbook(read_only=True)`` walking every row (one
Python object per cell, the reader Excel users reach for), and
``IOBase.read_arrow_reader(...).read_all()`` on this side, declared and
inferred. The ratio column is openpyxl's time over this crate's, so above one
is in this crate's favor. Before anything is timed, the rows openpyxl reads
are compared with this crate's, value for value.

The write cases time ``openpyxl.Workbook.append`` per row and ``save`` against
``IOBase.overwrite_arrow_table`` for the same table, each into a file of its
own; the random-access cases time ``Workbook.open`` plus one cell, and a
sheet laid out from a ``Serie`` and read back as one.

openpyxl is not a dependency of the crate - it is a checking tool of this
script only (``pip install openpyxl``); without it the script names what is
missing and stops.
"""

from __future__ import annotations

import argparse
import datetime
import gc
import pathlib
import platform
import shutil
import tempfile
import time
from collections.abc import Callable

import numpy as np
import pyarrow as pa

from yggdryl import IOBase, Serie
from yggdryl.excel import Sheet, Workbook

SYMBOLS = np.array(["AAPL", "MSFT", "GOOG", "AMZN", "NVDA", "META", "TSLA", "BP"])
EPOCH = datetime.date(2024, 1, 1)


def trades(rows: int) -> pa.Table:
    """Five mixed columns: a key, a code, a price, a flag and a date."""
    rng = np.random.default_rng(7)
    ids = np.arange(rows, dtype=np.int64)
    return pa.table(
        {
            "id": ids,
            "symbol": SYMBOLS[ids % len(SYMBOLS)],
            "price": np.round(rng.random(rows) * 1_000.0, 4),
            "live": (ids % 2 == 0),
            "traded": pa.array((ids % 365).astype("int32")).cast(pa.date32()),
        },
        schema=pa.schema(
            [
                pa.field("id", pa.int64(), nullable=False),
                pa.field("symbol", pa.string(), nullable=False),
                pa.field("price", pa.float64(), nullable=False),
                pa.field("live", pa.bool_(), nullable=False),
                pa.field("traded", pa.date32(), nullable=False),
            ]
        ),
    )


def _best(operation: Callable[[], object], repeat: int) -> float:
    """The fastest of ``repeat`` timed calls, after one warm-up call."""
    operation()
    laps = []
    for _ in range(repeat):
        gc.collect()
        started = time.perf_counter()
        operation()
        laps.append(time.perf_counter() - started)
    return min(laps)


def _openpyxl_rows(path: pathlib.Path) -> list[tuple[object, ...]]:
    """Every row past the header, as openpyxl's read-only walk answers them."""
    from openpyxl import load_workbook

    workbook = load_workbook(path, read_only=True, data_only=True)
    try:
        sheet = workbook[workbook.sheetnames[0]]
        rows = iter(sheet.iter_rows(values_only=True))
        next(rows)
        return list(rows)
    finally:
        workbook.close()


def _openpyxl_write(path: pathlib.Path, table: pa.Table) -> None:
    """The table as openpyxl writes it: one ``append`` per row, then ``save``."""
    from openpyxl import Workbook as PyWorkbook

    workbook = PyWorkbook(write_only=True)
    sheet = workbook.create_sheet("Sheet1")
    sheet.append(table.column_names)
    for row in table.to_pylist():
        sheet.append(list(row.values()))
    workbook.save(path)


def _check(path: pathlib.Path, table: pa.Table) -> None:
    """openpyxl and this crate read the same rows off the file this crate wrote."""
    ours = IOBase(path).read_arrow_reader(field=table.schema).read_all()
    if ours != table:
        raise SystemExit("this crate did not read back the table it wrote")
    theirs = _openpyxl_rows(path)
    for index, (row, want) in enumerate(zip(theirs, table.to_pylist(), strict=True)):
        got = dict(zip(table.column_names, row, strict=True))
        got["traded"] = got["traded"].date() if isinstance(got["traded"], datetime.datetime) else got["traded"]
        if got != want:
            raise SystemExit(f"row {index}: openpyxl read {got!r}, this crate wrote {want!r}")


def main() -> None:
    """Time each read and write case both ways and print one row per case."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeat", type=int, default=5)
    parser.add_argument("--rows", type=int, default=100_000, help="rows in the large workbook")
    parser.add_argument("--filter", default="", help="run only cases naming this text")
    arguments = parser.parse_args()
    if arguments.repeat < 1:
        parser.error("--repeat must be positive")
    try:
        import openpyxl
    except ImportError as error:
        raise SystemExit(f"excel benchmark: SKIPPED ({error}); pip install openpyxl")

    large = trades(arguments.rows)
    small = trades(10_000)
    folder = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-excel-bench-"))
    try:
        fixtures = {}
        for label, table in (("large", large), ("small", small)):
            path = folder / f"{label}.xlsx"
            IOBase(path).overwrite_arrow_table(table)
            fixtures[label] = (path, table)
        _check(*fixtures["small"])

        print(f"machine: {platform.machine()} {platform.system()} {platform.release()}")
        print(f"runtime: python {platform.python_version()}, pyarrow {pa.__version__}, openpyxl {openpyxl.__version__}")
        print(f"rows: large={arguments.rows} small=10000, columns=5")
        print()
        print("| Case | openpyxl | yggdryl | Ratio |")
        print("| --- | --- | --- | --- |")

        def report(name: str, theirs: Callable[[], object], ours: Callable[[], object]) -> None:
            if arguments.filter and arguments.filter not in name:
                return
            other = _best(theirs, arguments.repeat)
            mine = _best(ours, arguments.repeat)
            print(f"| {name} | {other * 1e3:.1f} ms | {mine * 1e3:.1f} ms | {other / mine:.1f}x |")

        for label, (path, table) in fixtures.items():
            rows = table.num_rows
            report(
                f"read {rows} rows, declared field",
                lambda path=path: _openpyxl_rows(path),
                lambda path=path, table=table: IOBase(path).read_arrow_reader(field=table.schema).read_all(),
            )
            report(
                f"read {rows} rows, inferred field",
                lambda path=path: _openpyxl_rows(path),
                lambda path=path: IOBase(path).read_arrow_reader().read_all(),
            )
            report(
                f"write {rows} rows",
                lambda path=path, table=table: _openpyxl_write(path.with_name("theirs.xlsx"), table),
                lambda path=path, table=table: IOBase(path.with_name("ours.xlsx")).overwrite_arrow_table(table),
            )

        path, table = fixtures["small"]
        report(
            "open the workbook and read one cell",
            lambda: openpyxl.load_workbook(path, read_only=True)["Sheet1"]["C3"].value,
            lambda: Workbook.open(path)["Sheet1"]["C3"].as_py(),
        )
        serie = Serie.from_(table)
        report(
            "a sheet from 10000 rows and back",
            lambda: [list(row) for row in openpyxl.load_workbook(path, read_only=True)["Sheet1"].iter_rows(values_only=True)],
            lambda: Sheet.from_serie("Sheet1", serie).into_serie(table.schema),
        )
        print()
        print("Regenerate: python python/benchmarks/media/excel.py --repeat 5")
    finally:
        shutil.rmtree(folder, ignore_errors=True)


if __name__ == "__main__":
    main()
