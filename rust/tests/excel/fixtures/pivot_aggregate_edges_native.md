# Native pivot aggregate edges

`pivot_aggregate_edges_native.json` and
`pivot_error_order_{forward,reversed}_native.json`, and
`pivot_matrix_error_order_native.json` record Microsoft Excel
16.0, build 20430.0, Windows 64-bit, UI LCID 1036. Each run created eleven
PivotTables, refreshed them, saved, closed, reopened and inspected the owned
workbook. All snapshots agree; each report passed with cleanup completed.
The JSON retains input worksheet XML, typed answers, error-cell XML and
input, manifest and native-report SHA-256 hashes.

The edge corpus distinguishes missing cells from cached formula-empty text
(`t="str"`, formula `""`, empty `v`). Count includes formula-empty text,
Booleans and errors; CountNumbers excludes all three. A physically blank
group stays empty under all eleven aggregates. Other numeric folds ignore
text and Booleans but propagate source errors. The first source error within
a group is retained: NA-then-DIV and DIV-then-NA give different errors.

Totals visit displayed groups. In the edge corpus, the first displayed error
group is DivOnly, so the grand total is `#DIV/0!`, although the first source
group is NAOnly. In both order-control fixtures, A_NA sorts before Z_DIV and
the grand total is `#N/A`, even when source order is reversed. Count and
CountNumbers totals in these controls are four and two respectively.
Together these observations distinguish displayed-group order from source
order or a fixed priority between these two error codes.

The matrix control places `#N/A` at displayed row A/column Z and `#DIV/0!`
at row B/column A, with B/A first in the source. The grand result is `#N/A`
for all nine error-propagating aggregates. Row totals are NA then DIV,
column totals are DIV then NA, demonstrating row-major displayed order
at the grand intersection. Numeric singleton cells produce sample-variance
errors without overriding an authored error elsewhere in the same total.

Errors derived from an aggregate are separate from source errors. A sample
variance over a singleton can return `#DIV/0!` while the grand variance is
valid, as the companion `pivot_all_aggregates_native.json` demonstrates.
These are bounded native observations, not a claim of Rust equivalence or
of arbitrary nested, descending or locale-dependent error order.
