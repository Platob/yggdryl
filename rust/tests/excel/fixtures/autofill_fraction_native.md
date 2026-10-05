# Native fractional Date AutoFill observations

Excel 16.0 build 20430.0 in the attached session opened two newly authored openpyxl books with built-in Date format 14. Source cells were numeric OOXML. Excel only ran `Range.AutoFill` (`xlFillSeries=2`), saved the owned books, and the observer captured `Value2` and saved `<v>` text. Both epochs passed, with `cleanup_completed=true`.

In both date systems, single sources 45292.5, 45292.001, approximately one millisecond, and 45292.00000001 all generated exact whole-day serials 45293, 45294. Pair 45292.5/45293.5 generated 45294 and 45295. Original source cells retained their fractions. The earlier 60-cell fixture separately covers 1900 day 59/60, reverse fill, General format, and sub-millisecond Copy; this 32-cell fixture extends it without replacing it.

The JSON pins both input and saved workbook SHA-256, source report SHA-256, and each observed cell's `Value2` and saved-cache IEEE-754 bits. It does not establish a universal rule for all fractional step patterns or month/year inference.

The public `fill_series_matches_fractional_date_native_observations` test now reproduces all 32 observations, retaining the original fractions while generating whole-day Date cells. The combined fill slice passes 12 tests.
