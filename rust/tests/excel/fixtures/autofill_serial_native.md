# Native AutoFill serial observations

Excel 16.0 build 20430.0 in the attached session opened two newly authored openpyxl workbooks with built-in date number format 14. Source numeric cells were written before COM; Excel received only `Range.AutoFill` with `xlFillSeries=2` or `xlFillCopy=1`, then saved the owned books. The run completed 2/2 books, 60 Value2/saved-numeric observations, no failures, and `cleanup_completed=true`.

In both date systems, formatted 59,60 continues 61,62; single 59 continues 60,61; single 60 continues 61,62; reverse fill from 61 produces 59,60,61; 59,61 continues 63,65. A sub-millisecond 45292.000000001 copied retains its original IEEE bits, while `xlFillSeries` produces exact 45293 and 45294 (the fraction does not propagate). General 59,60 continues 61,62. These are observations, not a universal claim about month/year patterns or text formats.

The machine-readable fixture holds the input and Excel-saved package SHA-256 digests and each cell's Value2 and saved `<v>` IEEE bits. Source report SHA-256: `786cbebaf97ef61d7e2ece75163f4d114d1e6418a73dd45dff8436a862f33265`. The public `fill_series_matches_sixty_native_serial_observations` test now compares all 60 observations, including exact saved numeric bits. It passes alongside the existing fill behavior tests; month/year and mixed temporal-type inference remain outside this fixture.
