#!/usr/bin/env python3
"""Write the dev server's fixtures from the design's §8.3 examples.

The shapes are §8.3's exactly; the values are extended into a coherent
workbook: 3 worksheets + 1 chartsheet + 1 hidden sheet, 27+ styles, merges,
a frozen pane (rows and columns), hidden rows and columns, and a few thousand
cells. Cells are written as objects; dev.py slices them into tiles.
"""
import json
import pathlib
import random
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from functions_catalog import catalog  # noqa: E402

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE / "fixtures"
OUT.mkdir(exist_ok=True)

MAX_ROWS = 1048576
# Widths are the file's <col width> units, padding included: Excel's default
# column, 64 px at maxDigitWidth 7.
DEFAULT_WIDTH = 9.140625
MAX_COLUMNS = 16384


def col_name(c):
    s = ""
    n = c + 1
    while n:
        n, r = divmod(n - 1, 26)
        s = chr(65 + r) + s
    return s


def ref(r, c):
    return f"{col_name(c)}{r + 1}"


def style(font=None, fill=None, border=None, alignment=None, fmt="General"):
    base_font = {"name": "Calibri", "size": 11, "bold": False, "italic": False,
                 "underline": "none", "strike": False, "color": "#000000"}
    base_font.update(font or {})
    base_align = {"horizontal": "general", "vertical": "bottom", "wrap": False,
                  "indent": 0, "rotation": 0, "shrink": False}
    base_align.update(alignment or {})
    return {"font": base_font, "fill": fill or {"pattern": "none"}, "border": border or {},
            "alignment": base_align, "numberFormat": fmt}


thin = {"style": "thin", "color": "#000000"}
STYLES = [
    style(),                                                                   # 0 default
    style(font={"bold": True}),                                                # 1 bold
    style(font={"italic": True}),                                              # 2 italic
    # 3: §8.3's example style, a header
    style(font={"bold": True, "color": "#FFFFFF"}, fill={"pattern": "solid", "color": "#1F4E78"},
          border={"bottom": thin}, alignment={"horizontal": "center", "vertical": "center"}),
    style(fill={"pattern": "solid", "color": "#F2F2F2"}),                      # 4 column style (hidden cols)
    style(fmt='"$"#,##0.00'),                                                  # 5 currency (unused)
    style(fmt="m/d/yyyy"),                                                     # 6 date
    style(fmt='"$"#,##0.00'),                                                  # 7 currency (§8.3 cells example)
    style(fmt="0.00%"),                                                        # 8 percent
    style(alignment={"wrap": True, "vertical": "top"}),                        # 9 wrap top
    style(font={"size": 18, "bold": True, "color": "#1F4E78"},
          alignment={"horizontal": "center", "vertical": "center"}),          # 10 title
    style(border={"left": thin, "right": thin, "top": thin, "bottom": thin}), # 11 all borders
    # 12: §8.3's example styled blank; the §8.3 styles example is a yellow solid fill
    style(font={"bold": True}, fill={"pattern": "solid", "color": "#FFFF00"},
          border={"bottom": {"style": "thin", "color": "#000000"}}),
    style(border={"bottom": {"style": "medium", "color": "#1F4E78"}}),        # 13 medium bottom
    style(font={"italic": True, "underline": "single", "color": "#C00000"}),  # 14 red italic underline
    style(font={"strike": True, "color": "#7F7F7F"}),                          # 15 strike gray
    style(alignment={"horizontal": "right", "indent": 2}),                     # 16 right indent 2
    style(alignment={"horizontal": "center", "vertical": "center"}),          # 17 center middle
    style(fill={"pattern": "mediumGray", "color": "#4472C4", "background": "#FFFFFF"}),  # 18 pattern
    style(font={"bold": True}, border={"top": thin, "bottom": {"style": "double", "color": "#000000"}}),  # 19 total
    style(border={"left": {"style": "dashed", "color": "#2F5597"}, "right": {"style": "dashed", "color": "#2F5597"},
                  "top": {"style": "dotted", "color": "#2F5597"}, "bottom": {"style": "dashDot", "color": "#2F5597"}}),  # 20
    style(fill={"pattern": "solid", "color": "#E2EFDA"}, fmt="#,##0"),         # 21 green number
    style(font={"name": "Consolas", "size": 10}),                              # 22 mono
    style(font={"name": "Georgia", "size": 14, "italic": True}),               # 23 serif
    style(alignment={"shrink": True}),                                         # 24 shrink to fit
    style(alignment={"horizontal": "left", "indent": 1}),                      # 25 left indent 1
    style(fmt="dddd, mmmm d, yyyy"),                                           # 26 long date
    style(fill={"pattern": "solid", "color": "#FCE4D6"},
          alignment={"horizontal": "center", "vertical": "center", "wrap": True},
          border={"left": {"style": "thick", "color": "#C65911"}, "right": {"style": "thick", "color": "#C65911"},
                  "top": {"style": "thick", "color": "#C65911"}, "bottom": {"style": "thick", "color": "#C65911"}}),  # 27 merged block
    style(font={"size": 11}, alignment={"horizontal": "right"}),               # 28 right text
    style(fill={"pattern": "solid", "color": "#DDEBF7"}, font={"bold": True}),  # 29 pivot header
    style(fill={"pattern": "solid", "color": "#DDEBF7"}, fmt="#,##0.00"),       # 30 pivot values
    style(fmt='_("$"* #,##0.00_);_("$"* \\(#,##0.00\\);_("$"* "-"??_);_(@_)'),  # 31 accounting
    style(fmt="@*."),                                                           # 32 text then dot fill
]

WORKBOOK = {
    "generation": 3, "revision": 42, "saved": 40, "dirty": True, "readOnly": False,
    "location": "file:///d/book.xlsx", "name": "book.xlsx", "dateSystem": "1900", "timezone": "Europe/Paris",
    "activeSheet": 1,
    "sheets": [
        {"key": 1, "name": "Data", "kind": "worksheet", "state": "visible"},
        {"key": 2, "name": "Chart1", "kind": "chartsheet", "state": "visible"},
        {"key": 3, "name": "Report", "kind": "worksheet", "state": "visible"},
        {"key": 4, "name": "Far", "kind": "worksheet", "state": "visible"},
        {"key": 5, "name": "Secret", "kind": "worksheet", "state": "hidden"},
    ],
    "names": [{"name": "Rates", "scope": None, "text": "=Data!$F$2:$F$9"}],
    "pivots": [{"sheet": 3, "name": "PivotTable1", "range": "A30:F39", "editable": True, "reason": None}],
    "calc": {"uncomputed": 3, "circular": [{"sheet": 1, "ref": "B2"}], "circularCount": 1},
    "undo": {"label": "Typing '=SUM(A1:A3)' in B4"}, "redo": {"label": None},
    "styles": len(STYLES),
}


def cell(cells, r, c, text, style=0, flags=0, color=None, fill=None, shorter=None, entry=None):
    item = {"r": r, "c": c, "text": text, "style": style, "flags": flags}
    if color is not None:
        item["color"] = color
    if fill is not None:
        item["fill"] = fill
    if shorter is not None:
        item["shorter"] = shorter
    item["entry"] = entry if entry is not None else text
    cells[(r, c)] = item


def money(v):
    return f"${v:,.2f}"


def data_sheet():
    rnd = random.Random(7)
    cells = {}
    cell(cells, 0, 0, "Region", 3)
    cell(cells, 0, 1, "Symbol", 3)
    cell(cells, 0, 2, "Date", 3)
    cell(cells, 0, 3, "Amount", 3)
    cell(cells, 0, 4, "", 3, 16)
    cell(cells, 0, 7, "Notes", 3)
    cell(cells, 0, 8, "Qty", 3)
    cell(cells, 0, 9, "N", 3)
    regions = ["North", "South", "East", "West"]
    symbols = ["AAPL", "MSFT", "GOOG", "AMZN", "NVDA", "TSLA", "META", "IBM"]
    for r in range(1, 501):
        cell(cells, r, 0, regions[r % 4])
        cell(cells, r, 1, symbols[rnd.randrange(len(symbols))])
        month, day = 1 + (r % 12), 1 + (r % 28)
        cell(cells, r, 2, f"{month}/{day}/2024", 6, 1, entry=f"{month}/{day}/2024")
        amount = round(rnd.uniform(10, 5000), 2)
        if r % 50 == 0:
            cell(cells, r, 3, money(amount), 7, 5, entry=f"=SUM(D{r - 8}:D{r})")
        else:
            cell(cells, r, 3, money(amount), 7, 1, entry=f"{amount}")
        if r <= 60:
            q = rnd.randrange(1, 900)
            cell(cells, r, 8, str(q), 0, 1)
    # A narrow column J: numbers that do not fit show ####, General ones pick a shorter spelling.
    cell(cells, 1, 9, "123456", 0, 1)
    cell(cells, 2, 9, "0.123456789", 0, 1, shorter=["0.12346", "0.123", "0.1"])
    cell(cells, 3, 9, "7", 0, 1)
    # Long notes overflow from H into I and J where those are empty.
    for r in (69, 70, 71):
        cell(cells, r, 7, f"Row {r + 1}: a long note that runs past the Notes column into the empty cells beside it", 0)
    cell(cells, 72, 7, "Stops at a filled neighbour: this text is clipped at column I", 0)
    cell(cells, 72, 8, "42", 0, 1)
    # §8.3's cells example lives at D2.
    cells[(1, 3)] = {"r": 1, "c": 3, "text": "$1,234.50", "style": 7, "flags": 5, "entry": "=SUM(D3:D9)"}
    layout = {
        "defaults": {"columnWidth": DEFAULT_WIDTH, "rowHeight": 15, "maxDigitWidth": 7},
        "columns": [[0, 0, 20.7, False, 0], [5, 6, None, True, 4], [7, 7, 40, False, None], [9, 9, 5, False, None]],
        "rows": [[0, 0, 30, False, None], [7, 9, None, True, None]],
        "merges": ["D1:E1"],
        "frozen": {"rows": 1, "columns": 0},
        "dimension": "A1:J501",
        "cells": len(cells),
        "pivots": [],
        "arrays": ["B1:B3"],
        "blocking": ["oleObjects"],
        "tables": [{"name": "Table1", "range": "A1:D20"}],
    }
    return layout, cells


def report_sheet():
    cells = {}
    # §8.3's tile example, exactly, in tile (0, 0).
    cell(cells, 0, 0, "Region", 3, 0)
    cell(cells, 1, 3, "$1,234.50", 7, 5, entry="=SUM(D3:D9)")
    cell(cells, 3, 4, "4.5", 0, 13, color="#FF0000", entry="=E3*1.5")
    cell(cells, 4, 1, "", 12, 16)
    cell(cells, 5, 2, "0.333333333", 0, 1, shorter=["0.33333", "0.333", "0.3"], entry="0.333333333")
    cell(cells, 0, 3, "Merged D1:E1", 17)
    cell(cells, 1, 0, "Quarter", 1)
    cell(cells, 2, 0, "Q1", 25)
    cell(cells, 3, 0, "Q2", 25)
    cell(cells, 4, 0, "Q3", 25)
    cell(cells, 5, 0, "Q4", 25)

    cell(cells, 7, 0, "Styles showcase", 10)
    cell(cells, 9, 0, "Wrap", 1)
    cell(cells, 9, 1, "Wrapped text breaks between words and stays inside its cell, top aligned.", 9)
    cell(cells, 10, 0, "Borders", 1)
    for c, v in ((1, "10"), (2, "20"), (3, "30")):
        cell(cells, 10, c, v, 11, 1)
    cell(cells, 11, 0, "Total", 1)
    cell(cells, 11, 1, "60", 19, 5, entry="=SUM(B11:D11)")
    cell(cells, 11, 3, "dashed", 20)
    cell(cells, 12, 0, "Fonts", 1)
    cell(cells, 12, 1, "red italic", 14)
    cell(cells, 12, 2, "struck", 15)
    cell(cells, 12, 3, "right+2", 16)
    cell(cells, 12, 4, "middle", 17)
    cell(cells, 13, 0, "Fills", 1)
    cell(cells, 13, 1, "pattern", 18)
    cell(cells, 13, 2, "1,234", 21, 1)
    cell(cells, 13, 3, "Consolas", 22)
    cell(cells, 13, 4, "Georgia", 23)
    cell(cells, 14, 0, "Shrink", 1)
    cell(cells, 14, 2, "shrunk to fit", 24)
    cell(cells, 14, 3, "indent 1", 25)
    cell(cells, 14, 4, "Monday, January 1, 2024", 26, 1, entry="1/1/2024")
    cell(cells, 15, 0, "Kinds", 1)
    cell(cells, 15, 1, "TRUE", 0, 2)
    cell(cells, 15, 2, "#DIV/0!", 0, 3 | 4, entry="=1/0")
    cell(cells, 15, 3, "12.50%", 8, 1, entry="12.5%")
    cell(cells, 16, 0, "####", 1)
    cell(cells, 16, 2, "$1,234,567.89", 7, 1, entry="1234567.89")
    cell(cells, 17, 1, "Left-aligned text spills right across the empty cells", 0)
    cell(cells, 18, 5, "Right-aligned text spills to the left", 28)
    cell(cells, 19, 3, "Centered text spills both ways", 17)
    cell(cells, 21, 1, "A merged block B22:D24, centered and wrapped", 27)
    for r in range(21, 24):
        for c in range(1, 4):
            if (r, c) != (21, 1):
                cell(cells, r, c, "", 27, 16)
    # A pivot-shaped block, flagged 64.
    cell(cells, 29, 0, "Region", 29, 64)
    cell(cells, 29, 1, "Sum of Amount", 29, 64)
    for i, region in enumerate(["East", "North", "South", "West"]):
        cell(cells, 30 + i, 0, region, 0, 64)
        cell(cells, 30 + i, 1, f"{(i + 1) * 1234.5:,.2f}", 30, 1 | 64)
    cell(cells, 34, 0, "Grand Total", 29, 64)
    cell(cells, 34, 1, f"{sum((i + 1) * 1234.5 for i in range(4)):,.2f}", 30, 1 | 64)
    # Accounting pads after the "$": the tile's fill is [char, offset into t].
    cell(cells, 10, 7, "Accounting", 1)
    cell(cells, 10, 8, " $1,234.50 ", 31, 1, fill=[" ", 2], entry="1234.5")
    cell(cells, 11, 8, " $(56.25)", 31, 1, fill=[" ", 2], entry="-56.25")
    cell(cells, 12, 8, " $-   ", 31, 1, fill=[" ", 2], entry="0")
    cell(cells, 13, 7, "Text fill", 1)
    cell(cells, 13, 8, "Total", 32, 0, fill=[".", 5])
    # A block to scroll under the frozen panes.
    for r in range(40, 160):
        cell(cells, r, 0, f"Item {r - 39}", 0)
        for c in range(1, 10):
            cell(cells, r, c, str((r * 7 + c * 13) % 997), 0, 1)
    layout = {
        "defaults": {"columnWidth": DEFAULT_WIDTH, "rowHeight": 15, "maxDigitWidth": 7},
        "columns": [[0, 0, 16, False, None], [1, 1, 22, False, None], [2, 2, 6, False, None], [6, 6, None, True, 4],
                    [8, 8, 14, False, None]],
        "rows": [[0, 0, 30, False, None], [7, 7, 28, False, None], [9, 9, 60, False, None]],
        "merges": ["D1:E1", "A8:F8", "B22:D24"],
        "frozen": {"rows": 2, "columns": 1},
        "dimension": "A1:J160",
        "cells": len(cells),
        "pivots": [{"name": "PivotTable1", "range": "A30:F39", "editable": True}],
        "arrays": [],
        "blocking": [],
        "tables": [],
    }
    return layout, cells


def far_sheet():
    cells = {}
    cell(cells, 0, 0, "Top-left")
    cell(cells, 0, MAX_COLUMNS - 1, "Top-right")
    cell(cells, MAX_ROWS - 1, 0, "Bottom-left")
    cell(cells, MAX_ROWS - 1, MAX_COLUMNS - 1, "Last cell")
    for r in range(500000, 500010):
        for c in range(8000, 8004):
            cell(cells, r, c, f"{r + 1}:{col_name(c)}")
    layout = {
        "defaults": {"columnWidth": DEFAULT_WIDTH, "rowHeight": 15, "maxDigitWidth": 7},
        "columns": [], "rows": [], "merges": [], "frozen": {"rows": 0, "columns": 0},
        "dimension": "A1:XFD1048576", "cells": len(cells), "pivots": [], "arrays": [], "blocking": [], "tables": [],
    }
    return layout, cells


def secret_sheet():
    cells = {}
    cell(cells, 0, 0, "hidden sheet")
    layout = {
        "defaults": {"columnWidth": DEFAULT_WIDTH, "rowHeight": 15, "maxDigitWidth": 7},
        "columns": [], "rows": [], "merges": [], "frozen": {"rows": 0, "columns": 0},
        "dimension": "A1", "cells": 1, "pivots": [], "arrays": [], "blocking": [], "tables": [],
    }
    return layout, cells


def write(name, value):
    (OUT / name).write_text(json.dumps(value, indent=None, separators=(",", ":")) + "\n")


write("workbook.json", WORKBOOK)
write("styles.json", STYLES)
write("functions.json", catalog())
write("media.json", {"records": ["application/vnd.apache.arrow.stream", "application/vnd.apache.parquet"],
                     "documents": ["application/json", "application/x-ndjson"]})
total = 0
for key, build in ((1, data_sheet), (3, report_sheet), (4, far_sheet), (5, secret_sheet)):
    layout, cells = build()
    write(f"sheet-{key}-layout.json", layout)
    write(f"sheet-{key}-cells.json", sorted(cells.values(), key=lambda item: (item["r"], item["c"])))
    total += len(cells)
print(f"wrote {len(list(OUT.iterdir()))} fixtures, {total} cells, {len(STYLES)} styles")
