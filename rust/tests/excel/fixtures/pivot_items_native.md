# Pivot item identity and order: native observations

Four answer-free workbooks from .scratch/p6-pivot-item-probes/ were opened
through the guarded active Excel 16.0 build 20430.0 session (French UI 1036).
All six PivotTables refreshed, saved and reopened with identical TableRange2,
fields and typed Value2 grids. Each run reports passed=true, cleanup=true and
no failures. The accompanying JSON records input/manifest/report/saved SHA-256,
source group-cell XML, cache shared items and record references, pivot row-item
indexes, and exact Value2 variants/number bits. Producer and compactor are
.scratch/build-p6-pivot-item-probes.py and
.scratch/compact-p6-pivot-item-native.py. Rust equivalence is unrun.

- East, east and EAST become one item with first spelling East and sum 7.
  Leading/trailing spaces remain distinct. Cache sharedItems retain first-seen
  order; visible rows reorder space variants. ASCII case folding is part of identity,
  and generic Scalar equality is insufficient.
- Authored empty inline text and an absent cell become one blank item (sum 3).
  Numeric 1 and 1.0 merge (sum 5), while text "1", TRUE, FALSE, text "TRUE"
  and blank are separate typed items. French visible Boolean labels are
  VRAI/FAUX; exact ordering is retained in the fixture.
- General numeric 59/60/61 and builtin-date-14 styled cells with the same
  source serials become distinct numeric and date items. Cache date values
  for the styled cells are 1900-02-27, 1900-02-28, 1900-03-01; COM Value2
  remains 59/60/61 for both kinds. This is an observed 1900 cache encoding,
  not a rule for 1904 or other dates.
- Source errors #N/A and #DIV/0! become distinct error axis items, separate
  from ordinary text and blank. The independent numeric measure sums to 15.
  Error-valued measure aggregation is not established.

The item dictionary needs one typed key resolved from the source cell and its
style/raw serial at intake, retaining the first spelling. Cache insertion
order and visible item order are separate facts. These six cases do not
establish locale-independent Unicode collation, every number width,
text-valued measures, filtered rows or arbitrary date formats.
