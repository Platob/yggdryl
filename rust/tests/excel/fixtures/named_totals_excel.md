# Excel named-table totals resize oracle

`named_totals_excel.json` records three named-table body replacements opened, calculated, saved, and reopened by Microsoft Excel 16.0 build 20430.0 (Windows 64-bit; UI language 1036). The input was authored by openpyxl through `scripts/check_excel_interop.py` as `from-openpyxl-named-totals.xlsx` (SHA-256 `b6e89bbf1c2a7fb1b943019e2dc7cf224e044306f8e3371500a76201ee6f59c0`). The Rust writer replaced only the `Quantities` table body; Excel independently performed the same shrink, equal-height, and grow edits. The comparator reopened each result, called `Worksheet("Data").Calculate` on its owned workbook, and checked the table range, auto-filter range, totals-row count, visible cells, and `Range.Value2` before and after an Excel save cycle. All three cases passed without a repair dialog; owned workbook cleanup completed. No application-wide recalculation was used.

| Body edit | Table | Auto-filter | Totals cell | `SUBTOTAL(109,[qty])` result |
| --- | --- | --- | --- | ---: |
| Shrink | D1:E3 | D1:E2 | E3 | 6 |
| Equal | D1:E4 | D1:E3 | E4 | 13 |
| Grow | D1:E5 | D1:E4 | E5 | 21 |

The JSON records source and Rust-output hashes for all cases, tracing these observations to the exchange run.

This oracle covers table geometry, formulas, calculated totals, and the inspected cell values. It does not compare styles or unrelated package parts.
