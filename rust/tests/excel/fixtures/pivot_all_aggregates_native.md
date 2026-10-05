# Native pivot aggregates

`pivot_all_aggregates_native.json` records eleven PivotTables produced by
Microsoft Excel 16.0, build 20430.0, Windows 64-bit, UI LCID 1036. The owned
workbook was refreshed, saved, closed, reopened and inspected. All three
snapshots agree; the harness reported no failures and completed cleanup.

The fixture retains the exact input worksheet XML, authored source rows,
source/manifest/report SHA-256 hashes, typed `Value2` observations and saved
error-cell XML. The original empty-string source cell B7 is an empty
`inlineStr` element; Excel reads it as blank. It does not prove how a formula
returning an empty string is counted. French grand-total captions are recorded
as observed, not translated into cached data.

Blank-only groups remain empty for every aggregate. Text-only groups yield
zero for SUM, MIN, MAX, PRODUCT and CountNumbers, two for Count, and `#DIV/0!`
for Average and the variance/standard-deviation aggregates in this corpus.
The request names and OOXML `subtotal` spellings are retained separately.

This is native evidence, not a claim that Rust reproduces every observation.
In particular, the four grand variance/standard-deviation results have a
non-dyadic mean (24/7), outside the shared accumulator's currently proven
exact domain. An uncertain computation must refuse before publishing a pivot;
it must not substitute a blank or an approximate total.
