# Two column fields and three value fields: native tabular layout

Excel 16.0 build 20430.0 (French UI 1036) created the single pivot from the
answer-free `layout_c2v3` input. Refresh, SaveAs and reopen snapshots matched;
the guarded active-session run passed with cleanup complete. The compact JSON
records SHA-256 of the input, manifest, report, native saved workbook and each
saved part, plus the exact typed 8×28 Value2 grid. Rust equivalence is unrun.

The source has one row field (Region), two column fields (Product, Year), three
SUM value fields, five observed leaf Product/Year tuples and a blank Product
item. Native `location` is `A3:AB10`, `firstHeaderRow=1`, `firstDataRow=4`,
`firstDataCol=1`; the column axis is Product, Year, Values. The 27 data
columns consist of 15 leaf-item columns, nine subtotal columns and three
grand-total columns. Consequently the layout width counts rendered column
groups including subtotals: five leaf tuples + three Product subtotals + one
grand group, each with three value columns, plus one row-label column. A count
of distinct leaf tuples alone would be wrong for this case.

This is one observed c=2/v=3 shape. It does not prove arbitrary hierarchy,
sort, subtotal policy, localized labels or a Rust-produced pivot package.
