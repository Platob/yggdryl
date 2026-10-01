# Rich package parts

`excel_package::rich_parts` embeds these exact bytes so its synthetic package
opens in Excel without repair while retaining opaque package edges.

- `rich_theme.xml`: the existing rich theme colors and fonts with the complete
  format scheme from an openpyxl-authored Office theme. The prior empty
  `a:fmtScheme` caused Excel `Workbooks.Open` to fail.
- `rich_drawing.xml`, `rich_chart.xml`, `rich_vml.xml`: Excel-authored graph
  from `rust/tests/excel/fixtures/rich_excel.xlsx`, with the native chart
  subsequently constrained to one B2:B4 series and the drawing anchor to
  `editAs="oneCell"` after a native open check. The separate synthetic chart
  style/color relationship parts were invalid in this package and are not
  included. The original two-run `comments1.xml` remains in the generator.
- `rich_printer.bin`: Excel-authored `printerSettings1.bin` for the built-in
  Microsoft Print to PDF driver, 5,428 bytes, SHA-256
  `10a10e7cb4f1ba6350cd603f38abec9ea4422b8c3a9e2c9603b9615c68fd5144`.
  Its printable UTF-16 content includes only the driver name, a GUID, and
  generic driver security field names. It is opaque binary data and must not
  be decoded as UTF-8.

The original package (with a local E2:E4 shared formula group and explicit
cross-sheet A2:A4 formulas), its unchanged Rust rewrite and its edited output
passed `scripts/check_excel_desktop.py fidelity` in Excel 16.0 build 20430.0
on 2026-09-30 UTC. The original and unchanged rewrite had equal feature
snapshots. Independent native save/reopen comparisons passed as well;
Excel's note-box coordinate quantization was measured on both legs. The
public Rust tests separately pin raw member preservation and structural edits.
