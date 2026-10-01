Microsoft Excel 16.0, build 20430.0, Windows 64-bit; UI LCID 1036, country 33.

`vml_excel.json` records Excel's saved VML note anchors for a common source,
then row 3 insertion, column A insertion, and both. The source note belongs
to A4 and its box starts above row 3. The note's owner moves both anchor
corners; pixel offsets remain equal across these four native outputs.
`shift::vml_note_band_translates_both_anchor_corners_with_its_owner` reads
these answers and checks saved undo/redo as well as the shifted package.

A separate native no-edit control saved/reopened the same anchored input
twice. Excel itself quantized its pixel offsets. The native edited workbook
and Rust output matched exactly on first inspection and again after each
received one Excel SaveAs/reopen. The desktop harness retains both exact
comparisons; it applies no coordinate tolerance. This evidence does not
establish a portable CSS-to-anchor conversion or VML control geometry.

Local evidence: `vml-band-native/results.json` and
`vml-save-cycles/results.json` under `rust/target/excel-desktop/`.
Both reports completed without failures and restored the active Excel
session. The fixture omits local paths and unrelated workbook content.
