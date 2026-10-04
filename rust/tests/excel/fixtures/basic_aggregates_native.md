# Native basic aggregates

Excel 16.0 build 20430.0, Windows 64-bit, UI language 1036, produced
`basic_aggregates_native.json` (296 observations) and
`aggregate_intersection_native.json` (84 observations). Each observation's
Value2 before and after SaveAs agrees exactly with the saved OOXML cache;
both owned-workbook runs passed, including cleanup. The JSON files retain
manifest, input workbook and report hashes, typed answers and formulas.

The Rust `basic_aggregates_` filter passed three tests: every one of the
380 native cases, exact numeric bits, repeated calculation without revision
changes, unchanged incremental work, sparse whole-column updates, and held
dependencies whose old caches look like ignorable text. The separate
`aggregate_intersections_keep_reference_origin_distinct_from_scalar_origin`
test failed before its fix and passed afterward.

MIN/MAX use actual finite binary values: the native pair `1/3` and the
entered `0.333333333333333` has distinct extrema even though the formula
comparison policy considers them equal. COUNT ignores reference errors;
COUNTA counts errors and empty formula text while skipping absent cells.
The presence-only sparse reader does not render a referenced value to count
it. Direct numeric text still uses the existing Entry grammar.

Explicit intersection preserves where an argument came from:
`SUM(@A1)` is zero for a Boolean or text cell, whereas `SUM(@TRUE)` is one.
The same distinction is pinned for COUNT, COUNTA, MIN, MAX and AND.
Grouping and names retain the provenance; arithmetic consumes it.
These observations do not establish array or omitted-argument semantics.

Native commands were the local-only `scripts/check_excel_desktop.py
functions --active` harness with manifests in
`.scratch/p5-basic-aggregate-inputs` and
`.scratch/p5-aggregate-intersection-inputs`. Results are retained under
`rust/target/excel-desktop/p5-basic-aggregates` and
`rust/target/excel-desktop/p5-aggregate-intersection`.
