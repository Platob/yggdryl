#!/usr/bin/env python3
"""Exchange Excel workbooks with openpyxl, both directions.

Self-consistency proves nothing about an exchange format, so this driver runs
the same rows through the Python implementation Excel users reach for:

1. ``cargo test --test interop excel::`` writes ``target/excel-interop/from-rust.xlsx``.
2. openpyxl reads it with ``data_only=True`` and asserts every cell, the type
   it reads each as, and the number formats of the temporal cells.
3. openpyxl writes ``target/excel-interop/from-openpyxl.xlsx`` with the same
   rows from Python natives - dates, datetimes, timedeltas, booleans, integers,
   floats and text - and a second sheet, plus ``from-openpyxl-1904.xlsx``
   under the 1904 date system.
4. The same cargo target runs again; its reading half opens both workbooks
   and asserts the cells and the record rows. That half prints ``SKIPPED``
   when a file is absent, and this driver fails on that word, so a skipped
   half can never read as a pass.

openpyxl is a checking tool of this script only - never a dependency of the
crate. Where the two disagree by design the assertions say so: openpyxl leaves
``_xHHHH_`` escapes unread and writes a literal one as it stands, which Excel
and this crate both read as the character it spells.
"""

from __future__ import annotations

import datetime
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
EXCHANGE = REPO / "rust" / "target" / "excel-interop"

HEADER = ["id", "symbol", "price", "live", "traded", "at", "took"]
LONG = "€ " + "x" * 300

# The rows both sides assert, matching rust/tests/interop/excel.rs.
ROWS: list[tuple[object, ...]] = [
    (1, " 12", 0.0, True, datetime.date(1900, 1, 1), datetime.datetime(2024, 1, 2, 3, 4, 5, 678000), datetime.timedelta(hours=1)),
    (2, "a ", -1.5, False, datetime.date(1900, 2, 28), datetime.datetime(1970, 1, 1), datetime.timedelta(0)),
    (3, "tab\tX_x0041_", 1e-9, True, datetime.date(1900, 3, 1), datetime.datetime(2000, 2, 29, 23, 59, 59), datetime.timedelta(hours=25)),
    (4, None, 1e15, False, None, datetime.datetime(9999, 12, 31), datetime.timedelta(milliseconds=1)),
    (5, LONG, 0.1 + 0.2, True, datetime.date(2024, 2, 29), datetime.datetime(2024, 2, 29, 12), datetime.timedelta(hours=12, milliseconds=500)),
]

# The third symbol the Rust writer wrote, as openpyxl reads it: the crate
# escapes a control character, a carriage return and a literal `_x0041_`
# under ST_Xstring, which openpyxl does not decode.
RUST_ESCAPED_SYMBOL = "SOH_x0001_CR_x000D_X_x005F_x0041_"


def run_cargo(allow_skip: bool) -> str:
    """Run the Rust half, failing on a skipped read unless it is expected."""
    result = subprocess.run(
        ["cargo", "test", "--locked", "--test", "interop", "excel::", "--", "--nocapture"],
        cwd=REPO / "rust",
        capture_output=True,
        text=True,
        check=False,
    )
    output = result.stdout + result.stderr
    if result.returncode != 0:
        raise SystemExit(f"cargo test failed:\n{output}")
    if not allow_skip and "SKIPPED" in output:
        raise SystemExit(f"the Rust reading half skipped:\n{output}")
    return output


def expect(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def read_with_openpyxl(path: Path) -> None:
    """Assert the workbook this crate wrote, through the reference reader."""
    from openpyxl import load_workbook

    workbook = load_workbook(path, data_only=True)
    expect(workbook.sheetnames == ["Sheet1"], f"unexpected sheets: {workbook.sheetnames}")
    sheet = workbook["Sheet1"]
    header = [cell.value for cell in sheet[1]]
    expect(header == HEADER, f"unexpected header: {header}")
    for index, row in enumerate(ROWS, start=2):
        cells = [cell for cell in sheet[index]]
        values = [cell.value for cell in cells]
        expected = list(row)
        if row[0] == 3:
            expected[1] = RUST_ESCAPED_SYMBOL
        for name, got, want, cell in zip(HEADER, values, expected, cells):
            if name == "traded" and want is not None:
                expect(isinstance(got, datetime.datetime), f"row {index} {name}: {got!r} is not a datetime")
                got = got.date()
                expect(cell.number_format == "yyyy-mm-dd", f"row {index} {name}: format {cell.number_format!r}")
            if name == "at":
                expect(isinstance(got, datetime.datetime), f"row {index} {name}: {got!r} is not a datetime")
                expect("hh:mm:ss" in cell.number_format, f"row {index} {name}: format {cell.number_format!r}")
            if name == "took":
                expect(isinstance(got, datetime.timedelta), f"row {index} {name}: {got!r} is not a timedelta")
                expect(cell.number_format == "[h]:mm:ss", f"row {index} {name}: format {cell.number_format!r}")
            if name == "id":
                # A whole number is written as Excel writes it, and reads as
                # the int openpyxl gives one.
                expect(isinstance(got, int) and not isinstance(got, bool), f"row {index} id: {got!r}")
            if name == "live":
                expect(isinstance(got, bool), f"row {index} live: {got!r}")
            if name == "price" and want == 1e15:
                got = float(got)
            expect(got == want, f"row {index} {name}: expected {want!r}, got {got!r}")


def write_with_openpyxl(path: Path, path_1904: Path) -> None:
    """Write the same rows from Python natives, and a 1904 workbook."""
    from openpyxl import Workbook
    from openpyxl.utils.datetime import CALENDAR_MAC_1904

    workbook = Workbook()
    sheet = workbook.active
    sheet.title = "Trades"
    sheet.append(HEADER)
    for row in ROWS:
        sheet.append(list(row))
    for index in range(2, len(ROWS) + 2):
        sheet.cell(row=index, column=5).number_format = "yyyy-mm-dd"
        sheet.cell(row=index, column=6).number_format = "yyyy-mm-dd hh:mm:ss.000"
        sheet.cell(row=index, column=7).number_format = "[h]:mm:ss"
    notes = workbook.create_sheet("Notes")
    notes["A1"] = "written by openpyxl"
    workbook.save(path)

    mac = Workbook()
    mac.epoch = CALENDAR_MAC_1904
    sheet = mac.active
    sheet["A1"] = datetime.date(2024, 2, 29)
    sheet["A1"].number_format = "yyyy-mm-dd"
    sheet["A2"] = datetime.date(1904, 1, 1)
    sheet["A2"].number_format = "yyyy-mm-dd"
    mac.save(path_1904)

    # What was written reads back through openpyxl as itself, so the Rust
    # reading half checks the exchange rather than a broken fixture.
    from openpyxl import load_workbook

    again = load_workbook(path, data_only=True)
    expect(again.sheetnames == ["Trades", "Notes"], "the fixture did not round-trip")
    expect(load_workbook(path_1904).epoch == CALENDAR_MAC_1904, "the 1904 fixture lost its epoch")


def main() -> int:
    try:
        import openpyxl  # noqa: F401
    except ImportError:
        raise SystemExit("openpyxl is not installed: pip install openpyxl") from None

    EXCHANGE.mkdir(parents=True, exist_ok=True)
    for name in ("from-rust.xlsx", "from-openpyxl.xlsx", "from-openpyxl-1904.xlsx"):
        (EXCHANGE / name).unlink(missing_ok=True)

    first = run_cargo(allow_skip=True)
    expect("excel-interop: wrote" in first, f"the Rust writing half did not report:\n{first}")
    read_with_openpyxl(EXCHANGE / "from-rust.xlsx")
    print("openpyxl read the workbook this crate wrote")

    write_with_openpyxl(EXCHANGE / "from-openpyxl.xlsx", EXCHANGE / "from-openpyxl-1904.xlsx")
    second = run_cargo(allow_skip=False)
    expect("excel-interop: read" in second, f"the Rust reading half did not report:\n{second}")
    expect("excel-interop: read 1904" in second, f"the 1904 half did not report:\n{second}")
    print("this crate read the workbooks openpyxl wrote")
    return 0


if __name__ == "__main__":
    sys.exit(main())
