# Pivot date items: 1900 and 1904 native observations

Two corrected answer-free workbooks contain General numeric and builtin-date-14
cells for serials 0, 1, 59, 60, 60.5 and 61 under each date system. Excel 16.0
build 20430.0 (French UI 1036) created a native PivotTable in each owned
workbook. Both refreshed, saved and reopened with identical typed Value2 grids;
the guarded active-session runs passed and cleaned up. The JSON records input,
manifest, report, saved-workbook and saved-part SHA-256, source cell XML style
and raw serial, ordered cache items/record references, and typed Value2.
Rust equivalence is unrun.

For each epoch, cache sharedItems alternate numeric `<n>` and date `<d>` for
the same raw serial, yielding 12 distinct source items. Source style determines
that item kind; COM Value2 reports the same numeric serial for both displayed
kinds, so that value alone cannot be an item key. Date cache encoding differs
by epoch. For 1900, date-styled serials 0, 1, 59, 60, 60.5, 61 become
1899-12-30, 1899-12-31, 1900-02-27, 1900-02-28,
1900-02-28T12:00:00, 1900-03-01. For 1904 they become 1904-01-01,
1904-01-02, 1904-02-29, 1904-03-01, 1904-03-01T12:00:00,
1904-03-02. This 1900 cache-date rule is distinct from the worksheet
`DateSystem::millis_from_serial` phantom-day projection before serial 60; the
pivot item owner must retain source raw serial and style/format provenance.

These twelve serials do not establish arbitrary format codes, timezone
conversion, date grouping, numeric signed-zero identity or locale-independent
item ordering. No Rust-produced pivot is claimed.
