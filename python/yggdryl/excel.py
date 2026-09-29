"""Office Open XML workbooks: ``Workbook``, ``Sheet`` and ``Cell``, over the core.

A ``.xlsx`` handle already reads and writes one worksheet as records through
the ``IOBase`` surface; this module is the random-access side of the same
medium. A ``Workbook`` opens a package and hands out live ``Sheet`` views, a
``Sheet`` reads and writes any cell by its ``A1`` reference and lays its rows
out as a ``Serie``, and a ``Cell`` is one value with the kind, number format,
formula and error its file states. Every cell value is a ``Scalar``.
"""

from __future__ import annotations

from ._native import (
    EXCEL_DEFAULT_SHEET_NAME as DEFAULT_SHEET_NAME,
    EXCEL_MAX_CELL_TEXT as MAX_CELL_TEXT,
    EXCEL_MAX_COLUMNS as MAX_COLUMNS,
    EXCEL_MAX_ROWS as MAX_ROWS,
    EXCEL_MAX_SHEET_NAME as MAX_SHEET_NAME,
    Cell,
    CellRange,
    CellRef,
    Excel,
    ExcelRow as Row,
    Sheet,
    Workbook,
)

__all__ = [
    "DEFAULT_SHEET_NAME",
    "MAX_CELL_TEXT",
    "MAX_COLUMNS",
    "MAX_ROWS",
    "MAX_SHEET_NAME",
    "Cell",
    "CellRange",
    "CellRef",
    "Excel",
    "Row",
    "Sheet",
    "Workbook",
]
