"""`python/src/excel.rs` and `yggdryl/excel.py`: the workbook, its sheets and cells.

Every behaviour here is the core's; what is pinned is the redirection - the
spellings a Python caller writes, the natives that come back, and the
protocols (equality, hashing, pickling, copying) each class answers.
"""

from __future__ import annotations

import copy
import datetime
import pathlib
import pickle
import subprocess
import sys

import pyarrow as pa
import pytest

import yggdryl
from yggdryl import IOBase, RecordOptions, Scalar, Serie
from yggdryl.excel import (
    DEFAULT_SHEET_NAME,
    MAX_CELL_TEXT,
    MAX_COLUMNS,
    MAX_ROWS,
    MAX_SHEET_NAME,
    Cell,
    CellRange,
    CellRef,
    Excel,
    Row,
    Sheet,
    Workbook,
)

XLSX = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"


def trades() -> pa.Table:
    """The rows every test lays out: an id, a nullable symbol, a price, a flag."""
    return pa.table(
        {
            "id": pa.array([1, 2, 3], pa.int64()),
            "symbol": pa.array(["AAPL", None, "MSFT"], pa.string()),
            "price": pa.array([187.5, 410.25, -0.5], pa.float64()),
            "live": pa.array([True, False, True], pa.bool_()),
        }
    )


def written(path: pathlib.Path) -> IOBase:
    """A `.xlsx` file holding `trades()` under the default sheet."""
    handle = IOBase(path / "trades.xlsx")
    handle.overwrite_arrow_table(trades())
    return handle


class TestModule:
    def test_the_module_exports_the_classes_and_the_grid_constants(self) -> None:
        assert MAX_ROWS == 1_048_576
        assert MAX_COLUMNS == 16_384
        assert MAX_CELL_TEXT == 32_767
        assert MAX_SHEET_NAME == 31
        assert DEFAULT_SHEET_NAME == "Sheet1"
        assert yggdryl.Excel is Excel
        assert yggdryl.media.Excel is Excel
        assert issubclass(Excel, yggdryl.Media)
        assert set(yggdryl.excel.__all__) >= {"Workbook", "Sheet", "Row", "Cell", "CellRef", "CellRange"}


class TestRecordOptions:
    def test_a_workbook_has_a_sheet_a_header_and_a_range_and_nothing_else_does(self) -> None:
        options = RecordOptions("trades.xlsx")
        assert str(options.mime_type) == XLSX
        assert options.sheet is None
        assert options.header is True
        assert options.range is None

        options.sheet = "Trades"
        options.header = False
        options.range = "A3:F"
        assert options.sheet == "Trades"
        assert options.header is False
        assert options.range == CellRange("A3:F")
        options.range = CellRange("B2:C3")
        assert str(options.range) == "B2:C3"
        options.range = None
        options.sheet = None
        assert options.range is None
        assert options.sheet is None

        with pytest.raises(ValueError, match="sheet name"):
            options.sheet = "a/b"

        ipc = RecordOptions("trades.arrows")
        assert ipc.sheet is None
        assert ipc.header is None
        assert ipc.range is None
        with pytest.raises(ValueError, match="expected Excel options"):
            ipc.sheet = "Trades"
        # The header is one knob a CSV and a workbook share.
        with pytest.raises(ValueError, match="expected CSV or Excel options"):
            ipc.header = False
        with pytest.raises(ValueError, match="expected Excel options"):
            ipc.range = "A1:B2"

    def test_the_settings_survive_pickling_and_freeze_after_hashing(self) -> None:
        options = RecordOptions("trades.xlsx")
        options.sheet = "Trades"
        options.header = False
        options.range = "A2:D"
        restored = pickle.loads(pickle.dumps(options))
        assert restored == options
        assert restored.sheet == "Trades"
        assert restored.header is False
        assert restored.range == CellRange("A2:D")
        assert hash(restored) == hash(options)
        for attribute, value in (("sheet", "Other"), ("header", True), ("range", "A1:B2")):
            with pytest.raises(TypeError, match="hashed RecordOptions"):
                setattr(options, attribute, value)


class TestRecordPath:
    def test_records_round_trip_through_a_named_file(self, tmp_path: pathlib.Path) -> None:
        handle = written(tmp_path)
        assert type(handle).__name__ == "Excel"
        assert isinstance(handle, Excel)
        assert isinstance(handle, yggdryl.Media)
        # A number cell is a float64: the file knows no other number. The
        # declared field reads the ids back as the int64 they were.
        inferred = handle.read_arrow_reader().read_all()
        assert inferred.column("id").to_pylist() == [1.0, 2.0, 3.0]
        assert inferred.column("symbol").to_pylist() == ["AAPL", None, "MSFT"]
        assert handle.read_arrow_reader(field=trades().schema).read_all() == trades()
        assert handle.row_size() == 3
        assert handle.column_size() == 4

    def test_the_options_address_a_sheet_and_a_range(self, tmp_path: pathlib.Path) -> None:
        handle = IOBase(tmp_path / "book.xlsx")
        handle.overwrite_arrow_table(trades(), sheet="Trades")
        handle.overwrite_arrow_table(
            pa.table({"note": ["a", "b"]}), sheet="Notes"
        )
        assert Workbook.open(handle).sheet_names == ["Trades", "Notes"]
        read = handle.read_arrow_reader(sheet="Notes").read_all()
        assert read.column("note").to_pylist() == ["a", "b"]
        # A range's first row is its header; a header-less read names the
        # columns by their letters.
        narrowed = handle.read_arrow_reader(sheet="Trades", range="B1:C").read_all()
        assert narrowed.column_names == ["symbol", "price"]
        # Without a header the columns are named by their letters; the
        # header row itself is text over numbers, so the range leaves it out.
        lettered = handle.read_arrow_reader(sheet="Trades", header=False, range="A2:D").read_all()
        assert lettered.column_names == ["A", "B", "C", "D"]
        assert lettered.num_rows == 3
        with pytest.raises(ValueError, match="Trades!A2.*declare a field"):
            handle.read_arrow_reader(sheet="Trades", header=False)

    def test_a_coded_workbook_name_is_refused_at_every_door(self, tmp_path: pathlib.Path) -> None:
        # A workbook is a ZIP package deflated inside, so `.xlsx.gz` names a
        # file no spreadsheet opens: the write refuses it before a byte is
        # written, and a read and `Workbook.open` refuse it too.
        for name, codec in (("trades.xlsx.gz", "gzip"), ("trades.xlsx.zst", "zstd")):
            refused = f"expected an uncompressed xlsx handle, got {codec} coding"
            handle = IOBase(tmp_path / name)
            with pytest.raises(ValueError, match=refused):
                handle.overwrite_arrow_table(trades())
            assert not (tmp_path / name).exists()
            with pytest.raises(ValueError, match=refused):
                handle.read_arrow_reader()
            with pytest.raises(ValueError, match=refused):
                Workbook.open(tmp_path / name)

    def test_a_missing_sheet_reads_as_nothing_and_a_write_adds_it(self, tmp_path: pathlib.Path) -> None:
        handle = written(tmp_path)
        assert handle.read_arrow_reader(sheet="Missing").read_all().num_rows == 0
        handle.append_arrow_table(pa.table({"note": ["a"]}), sheet="Notes")
        assert Workbook.open(handle).sheet_names == ["Sheet1", "Notes"]
        assert handle.read_arrow_reader(sheet="Notes").read_all().column("note").to_pylist() == ["a"]


class TestCellRef:
    def test_every_spelling_reads_and_the_reference_answers_its_parts(self) -> None:
        for value in ("B3", "$B$3", (2, 1), CellRef("B3")):
            reference = CellRef(value)
            assert (reference.row, reference.column) == (2, 1)
            assert str(reference) == "B3"
        assert repr(CellRef("B3")) == "CellRef('B3')"
        assert CellRef.column_name(0) == "A"
        assert CellRef.column_name(27) == "AB"
        assert CellRef.column_index("XFD") == MAX_COLUMNS - 1
        assert CellRef.column_index("XFE") is None
        assert CellRef("A1").is_in_grid()

    def test_a_reference_is_a_value_ordered_hashed_pickled_and_copied(self) -> None:
        a1, b2 = CellRef("A1"), CellRef("B2")
        assert a1 == CellRef((0, 0))
        assert a1 != b2
        assert a1 < b2 <= CellRef("B2")
        assert hash(a1) == hash(CellRef("A1"))
        assert pickle.loads(pickle.dumps(b2)) == b2
        assert copy.copy(b2) == b2
        assert copy.deepcopy(b2) == b2
        assert a1.__eq__(1) is NotImplemented

    def test_a_reference_outside_the_grid_or_of_another_sheet_is_refused(self) -> None:
        with pytest.raises(ValueError, match="sheet-qualified"):
            CellRef("Sheet1!A1")
        with pytest.raises(ValueError, match="row number"):
            CellRef("A0")
        with pytest.raises(ValueError, match="A to XFD"):
            CellRef("XFE1")
        with pytest.raises(ValueError):
            CellRef((MAX_ROWS, 0))
        with pytest.raises(TypeError, match="expected a CellRef"):
            CellRef(1.5)


class TestCellRange:
    def test_every_spelling_reads_and_the_range_answers_its_shape(self) -> None:
        closed = CellRange("C3:A1")
        assert str(closed) == "A1:C3"
        assert (closed.start, closed.end) == (CellRef("A1"), CellRef("C3"))
        assert (closed.row_size(), closed.column_size()) == (3, 3)
        assert not closed.is_row_open() and not closed.is_column_open()
        assert CellRange((CellRef("C3"), "A1")) == closed
        assert CellRange(("A1", "C3")) == closed

        columns = CellRange("A:C")
        assert columns.is_row_open()
        assert columns.row_size() == MAX_ROWS
        assert str(columns) == "A:C"
        rows = CellRange("3:5")
        assert rows.is_column_open()
        assert str(rows) == "3:5"
        assert str(CellRange("A3:F")) == "A3:F"
        assert CellRange.all().row_size() == MAX_ROWS
        assert CellRange.all().column_size() == MAX_COLUMNS
        assert repr(closed) == "CellRange('A1:C3')"

    def test_containment_and_iteration_walk_the_cells_row_by_row(self) -> None:
        span = CellRange("B2:C3")
        assert span.contains("C3") and span.contains((1, 1))
        assert not span.contains("A1")
        assert "B2" in span and "D4" not in span
        assert span.contains_row(2) and not span.contains_row(3)
        assert span.contains_column(1) and not span.contains_column(3)
        assert [str(cell) for cell in span] == ["B2", "C2", "B3", "C3"]

    def test_a_range_is_a_value_hashed_pickled_and_copied(self) -> None:
        span = CellRange("A1:C3")
        assert pickle.loads(pickle.dumps(span)) == span
        assert copy.deepcopy(span) == span
        assert hash(span) == hash(CellRange("A1:C3"))
        assert span < CellRange("A2:C3")
        with pytest.raises(ValueError, match="expected a cell range"):
            CellRange("A:3")


class TestCell:
    def test_a_cell_states_its_kind_format_and_value_for_every_native(self) -> None:
        text = Cell("A1", "AAPL")
        assert (text.kind, text.format) == ("s", "general")
        assert text.value == Scalar.from_("AAPL")
        assert text.as_py() == "AAPL"
        assert text.text() == "AAPL"
        number = Cell((0, 1), 187.5)
        assert (number.kind, number.format, number.as_py()) == ("n", "general", 187.5)
        flag = Cell("C1", True)
        assert (flag.kind, flag.as_py()) == ("b", True)
        day = Cell("D1", datetime.date(2024, 1, 2))
        assert (day.kind, day.format) == ("n", "date")
        assert day.as_py() == datetime.date(2024, 1, 2)
        when = Cell("E1", datetime.datetime(2024, 1, 2, 3, 4, 5))
        assert when.format in {"datetime", "datetime_fraction"}
        assert when.as_py() == datetime.datetime(2024, 1, 2, 3, 4, 5)
        empty = Cell("F1", None)
        assert empty.is_null()
        assert empty.value == Scalar.from_(None)
        assert (empty.reference, empty.row, empty.column) == (CellRef("F1"), 0, 5)

    def test_a_formula_an_error_and_a_move_answer_new_cells(self) -> None:
        cell = Cell("A1", 3).with_formula("1+2")
        assert cell.formula == "1+2"
        assert cell.error is None
        moved = cell.at("B2")
        assert moved.reference == CellRef("B2")
        assert moved.formula == "1+2"
        failed = Cell("A1", None).with_error("#DIV/0!")
        assert failed.error == "#DIV/0!"
        assert failed.is_null()
        assert repr(cell) == "Cell('A1', 3)"

    def test_a_cell_is_a_value_hashed_pickled_and_copied(self) -> None:
        cell = Cell("A1", "AAPL").with_formula('"AA"&"PL"')
        again = pickle.loads(pickle.dumps(cell))
        assert again == cell
        assert again.formula == cell.formula
        assert hash(again) == hash(cell)
        assert copy.deepcopy(cell) == cell
        assert cell != Cell("A2", "AAPL")
        with pytest.raises(ValueError, match="1904"):
            Cell("A1", datetime.date(1903, 12, 31), date_system="1904")
        with pytest.raises(ValueError, match="date system"):
            Cell("A1", 1, date_system="1930")


class TestSheet:
    def test_a_sheet_is_built_cell_by_cell_and_read_back_by_reference(self) -> None:
        sheet = Sheet("Trades")
        assert (sheet.name, sheet.state, sheet.date_system) == ("Trades", "visible", "1900")
        assert len(sheet) == 0 and sheet.is_empty()
        assert sheet.dimension is None
        assert sheet["A1"] is None
        sheet["A1"] = "symbol"
        sheet[(0, 1)] = "price"
        assert sheet.set_cell("A2", "AAPL") is None
        sheet.set_cell("B2", 187.5)
        assert sheet.set_cell("B2", 190.0).as_py() == 187.5
        assert sheet["B2"].as_py() == 190.0
        assert sheet.scalar("B2") == Scalar.from_(190.0)
        assert sheet.scalar("Z9") == Scalar.from_(None)
        assert "A1" in sheet and "Z9" not in sheet
        assert len(sheet) == 2
        assert str(sheet.dimension) == "A1:B2"
        assert [cell.as_py() for cell in sheet.cells()] == ["symbol", "price", "AAPL", 190.0]
        assert [cell.as_py() for cell in sheet.column(0)] == ["symbol", "AAPL"]
        assert [row.index for row in sheet.rows()] == [0, 1]
        row = sheet.row(1)
        assert isinstance(row, Row)
        assert len(row) == 2 and row.cell(1).as_py() == 190.0
        assert [cell.reference for cell in row] == [CellRef("A2"), CellRef("B2")]
        assert repr(sheet) == "Sheet('Trades', 2 rows)"

        sliced = sheet["A2:B2"]
        assert isinstance(sliced, Sheet)
        assert [str(cell.reference) for cell in sliced.cells()] == ["A2", "B2"]
        assert sheet.slice("B2:B2") == sliced.slice("B:B")
        assert [cell.as_py() for cell in sheet.cells_in("B1:B2")] == ["price", 190.0]

        del sheet["A1"]
        assert sheet.remove_cell("A1") is None
        with pytest.raises(KeyError):
            del sheet["A1"]
        sheet.insert_cell(Cell("A1", "id"))
        assert sheet["A1"].as_py() == "id"
        sheet.insert_rows(1, 2)
        assert sheet["A4"].as_py() == "AAPL"
        sheet.remove_rows(1, 3)
        assert sheet["A2"].as_py() == "AAPL"

    def test_a_sheet_lays_a_table_out_and_reads_it_back_as_a_serie(self) -> None:
        sheet = Sheet.from_serie("Trades", trades())
        assert len(sheet) == 4
        assert [cell.as_py() for cell in sheet.row(0)] == ["id", "symbol", "price", "live"]
        assert sheet["B3"] is None  # the null symbol writes no cell
        serie = sheet.into_serie()
        assert isinstance(serie, Serie)
        table = serie.into_arrow_table()
        # A number cell is a float64: the file knows no other number.
        assert table.column("id").to_pylist() == [1.0, 2.0, 3.0]
        assert table.column("symbol").to_pylist() == ["AAPL", None, "MSFT"]
        assert table.column("live").to_pylist() == [True, False, True]

        declared = sheet.into_serie(trades().schema).into_arrow_table()
        assert declared == trades()
        lettered = sheet.slice("A2:D4").into_serie(header=False).into_arrow_table()
        assert lettered.column_names == ["A", "B", "C", "D"]
        assert lettered.num_rows == 3
        with pytest.raises(ValueError, match="Trades!A2.*declare a field"):
            sheet.into_serie(header=False)

        anchored = Sheet("Anchored")
        anchored.write_serie("B3", trades(), header=False)
        assert anchored["B3"].as_py() == 1.0 or anchored["B3"].as_py() == 1
        assert anchored["E5"].as_py() is True
        anchored.extend_from_serie(pa.table({"id": [4], "symbol": ["GOOG"], "price": [1.0], "live": [False]}))
        assert anchored["B6"].as_py() in (4, 4.0)
        assert anchored["C6"].as_py() == "GOOG"

    def test_a_sheet_name_and_state_follow_the_rules_of_the_file(self) -> None:
        sheet = Sheet("Trades", state="hidden", date_system="1904")
        assert (sheet.state, sheet.date_system) == ("hidden", "1904")
        sheet.state = "veryHidden"
        assert sheet.state == "veryHidden"
        sheet.name = "Fills"
        assert sheet.name == "Fills"
        with pytest.raises(ValueError, match="sheet name"):
            sheet.name = "a" * (MAX_SHEET_NAME + 1)
        with pytest.raises(ValueError, match="sheet name"):
            Sheet("History")
        with pytest.raises(ValueError, match="state"):
            sheet.state = "shy"

    def test_a_sheet_compares_pickles_and_copies_by_its_cells(self) -> None:
        sheet = Sheet.from_serie("Trades", trades())
        again = pickle.loads(pickle.dumps(sheet))
        assert again == sheet
        assert again.name == "Trades"
        assert copy.deepcopy(sheet) == sheet
        other = copy.copy(sheet)
        other["A1"] = "identifier"
        assert other != sheet
        with pytest.raises(TypeError):
            hash(sheet)


class TestWorkbook:
    def test_a_workbook_opens_from_a_handle_a_path_or_bytes_and_hands_out_live_views(
        self, tmp_path: pathlib.Path
    ) -> None:
        handle = written(tmp_path)
        for source in (handle, tmp_path / "trades.xlsx", str(tmp_path / "trades.xlsx")):
            workbook = Workbook.open(source)
            assert workbook.sheet_names == ["Sheet1"]
            assert len(workbook) == 1
            assert "Sheet1" in workbook and "sheet1" in workbook
            assert workbook.sheet_kind("Sheet1") == "worksheet"
            assert workbook.sheet_kind("Missing") is None
        workbook = Workbook.from_bytes(handle.read_bytes())
        assert repr(workbook) == "Workbook(['Sheet1'])"
        # In memory as well: a handle over bytes answers an identity rather
        # than a location, and opens from a copy of its bytes.
        memory = IOBase.from_bytes(handle.read_bytes())
        memory.media_type = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        assert Workbook.open(memory).sheet_names == ["Sheet1"]
        assert workbook.date_system == "1900"
        assert workbook.handle_reads >= 1

        sheet = workbook["Sheet1"]
        assert sheet is not None
        assert sheet["A1"].as_py() == "id"
        assert sheet["B2"].as_py() == "AAPL"
        assert sheet["B3"] is None
        assert workbook.sheet("sheet1")["C4"].as_py() == -0.5
        assert workbook.sheet_at(0).name == "Sheet1"
        assert workbook.sheet_at(1) is None
        assert workbook[-1].name == "Sheet1"
        assert workbook.get_sheet("Missing") is None
        with pytest.raises(KeyError, match="Missing"):
            workbook.sheet("Missing")
        with pytest.raises(KeyError):
            workbook["Missing"]
        with pytest.raises(IndexError):
            workbook[3]
        assert [view.name for view in workbook] == ["Sheet1"]

        # The view is live: a cell set through it is what the workbook writes.
        sheet["E1"] = "note"
        sheet[(3, 4)] = 2.5
        reopened = Workbook.from_bytes(workbook.into_bytes())
        assert reopened["Sheet1"]["E1"].as_py() == "note"
        assert reopened["Sheet1"]["E4"].as_py() == 2.5
        assert reopened["Sheet1"]["B2"].as_py() == "AAPL"

    def test_sheets_are_added_inserted_renamed_and_removed(self, tmp_path: pathlib.Path) -> None:
        workbook = Workbook()
        assert workbook.is_empty() and len(workbook) == 0
        trades_sheet = workbook.add_sheet("Trades")
        trades_sheet["A1"] = "symbol"
        trades_sheet["A2"] = "AAPL"
        assert workbook["Trades"]["A2"].as_py() == "AAPL"
        with pytest.raises(ValueError, match="no other sheet has"):
            workbook.add_sheet("trades")

        notes = Sheet.from_serie("Notes", pa.table({"note": ["a", "b"]}))
        assert workbook.insert_sheet(notes) is None
        # The object now views the sheet inside the workbook.
        notes["C1"] = "seen"
        assert workbook["Notes"]["C1"].as_py() == "seen"
        assert workbook.sheet_names == ["Trades", "Notes"]

        notes.name = "Remarks"
        assert workbook.sheet_names == ["Trades", "Remarks"]
        with pytest.raises(ValueError, match="no other sheet has"):
            notes.name = "TRADES"
        workbook.rename_sheet("Remarks", "Notes")
        assert workbook.sheet_names == ["Trades", "Notes"]

        replaced = workbook.insert_sheet(Sheet("Notes"))
        assert replaced is not None and replaced["C1"].as_py() == "seen"
        assert workbook["Notes"].is_empty()
        removed = workbook.remove_sheet("Notes")
        assert removed is not None and removed.name == "Notes"
        assert workbook.remove_sheet("Notes") is None
        with pytest.raises(KeyError):
            del workbook["Notes"]
        del workbook["Trades"]
        assert workbook.is_empty()

        workbook.add_sheet("Only")["A1"] = 1
        target = tmp_path / "written.xlsx"
        workbook.write_into(target)
        assert Workbook.open(target)["Only"]["A1"].as_py() == 1
        handle = IOBase(tmp_path / "second.xlsx")
        workbook.write_into(handle)
        assert Workbook.open(handle).sheet_names == ["Only"]
        assert handle.read_arrow_reader().read_all().num_rows == 0

    def test_the_date_system_is_the_workbooks_and_a_date_round_trips_under_it(self) -> None:
        workbook = Workbook()
        workbook.date_system = "1904"
        sheet = workbook.add_sheet("Dates")
        sheet["A1"] = datetime.date(2024, 2, 29)
        sheet["A2"] = datetime.datetime(2024, 2, 29, 12, 30)
        assert sheet.date_system == "1904"
        reopened = Workbook.from_bytes(workbook.into_bytes())
        assert reopened.date_system == "1904"
        assert reopened["Dates"]["A1"].as_py() == datetime.date(2024, 2, 29)
        assert reopened["Dates"]["A2"].as_py() == datetime.datetime(2024, 2, 29, 12, 30)
        with pytest.raises(ValueError, match="date system"):
            workbook.date_system = "1930"

    def test_bytes_that_are_not_a_package_are_refused_by_name(self) -> None:
        with pytest.raises(ValueError, match="BIFF"):
            Workbook.from_bytes(b"\xd0\xcf\x11\xe0" + b"\0" * 12)
        with pytest.raises(ValueError, match="ZIP package"):
            Workbook.from_bytes(b"not a package at all")
        assert Workbook.from_bytes(b"").is_empty()


PARSE_WHILE_READ_SCRIPT = r"""
import threading
import time

import pyarrow as pa
import pyarrow.fs as pafs

from yggdryl import IOBase
from yggdryl.excel import Workbook


class Pausing(pafs.FileSystemHandler):
    # Every open sleeps - releasing the GIL - while the workbook is locked.
    def __init__(self, files):
        self.files = files

    def get_type_name(self):
        return "pausing"

    def normalize_path(self, path):
        return path.strip("/")

    def get_file_info(self, paths):
        return [
            pafs.FileInfo(key, pafs.FileType.File, size=len(self.files[key]))
            if key in self.files
            else pafs.FileInfo(key, pafs.FileType.NotFound)
            for key in (path.strip("/") for path in paths)
        ]

    def get_file_info_selector(self, selector):
        return []

    def create_dir(self, path, recursive):
        pass

    def delete_dir(self, path):
        pass

    def delete_dir_contents(self, path, missing_dir_ok=False):
        pass

    def delete_root_dir_contents(self):
        pass

    def delete_file(self, path):
        pass

    def move(self, src, dest):
        pass

    def copy_file(self, src, dest):
        pass

    def open_input_stream(self, path):
        time.sleep(0.05)
        return pa.BufferReader(self.files[path.strip("/")])

    def open_input_file(self, path):
        time.sleep(0.05)
        return pa.BufferReader(self.files[path.strip("/")])

    def open_output_stream(self, path, metadata=None):
        raise NotImplementedError(path)

    def open_append_stream(self, path, metadata=None):
        raise NotImplementedError(path)


written = Workbook()
for name in ("One", "Two"):
    written.add_sheet(name)["A1"] = name
filesystem = pafs.PyFileSystem(Pausing({"book.xlsx": bytes(written.into_bytes())}))
workbook = Workbook.open(IOBase.from_fs(filesystem, "book.xlsx"))
one = workbook.sheet("One")
answers = []


def parse():
    answers.append(workbook.sheet("Two")["A1"].as_py())


def read():
    time.sleep(0.02)
    answers.append((workbook.sheet_names, len(workbook), "Two" in workbook, one.name))


threads = [threading.Thread(target=parse), threading.Thread(target=read)]
for thread in threads:
    thread.start()
for thread in threads:
    thread.join()
assert sorted(answers, key=str) == [(["One", "Two"], 2, True, "One"), "Two"], answers
print("ok")
"""


def test_a_sheet_parsed_over_a_python_filesystem_never_holds_the_gil_waiting() -> None:
    """A sheet is parsed on first access under the workbook's lock, reading the
    package through its handle - here a ``pyarrow.fs.PyFileSystem`` handler
    that sleeps, releasing the GIL. A thread waiting on that lock attached -
    the workbook's names, its length, a live sheet view - would keep the
    parsing thread from taking the GIL back, and the process would hang. In a
    process of its own under a deadline, because the failure is a hang.
    """
    try:
        result = subprocess.run(
            [sys.executable, "-c", PARSE_WHILE_READ_SCRIPT],
            capture_output=True,
            text=True,
            check=False,
            timeout=120,
        )
    except subprocess.TimeoutExpired as hung:
        raise AssertionError("a read waiting on a sheet parse hung") from hung
    assert result.returncode == 0, result.stdout + result.stderr
    assert result.stdout.strip().endswith("ok")
