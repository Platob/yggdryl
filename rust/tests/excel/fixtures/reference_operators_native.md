# Reference operator native oracle

Excel 16.0 build 20430.0, Windows64, French UI1036. Two owned-workbook runs
recorded 116 original formulas plus 24 further colon shapes across both date systems. Every COM value matched its
saved cache before and after SaveAs, and cleanup completed. The observations
are from the same open workbook, not a reopen verification. Report, manifest,
input and Excel-saved package hashes accompany the cases.

Union preserves source order and overlapping cells, including duplicate areas.
A scalar union is VALUE; geometry functions over a union return REF. Union,
intersection and range across different worksheets return VALUE. Empty
intersection returns NULL. Aggregate consumption, selected unions, INDEX area
selection and rejecting a union in single-area functions are separately pinned.
These are observed contracts, not inferred answers from another spreadsheet
implementation. The Rust test replays both authored and Excel-saved spellings.

The added same-sheet colon-shape run establishes an enclosing rectangle
for two rectangular operands in either order and for a contained endpoint:
`SUM(C1:D2:A5:B6)=300`, `ROWS(...)=6`, `COLUMNS(...)=4`, and
`SUM(A1:C3:B2)=54` against its separately hashed 1..24 Values grid.
The source report and two saved workbook hashes are retained in the run.
