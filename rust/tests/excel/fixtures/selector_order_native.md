# Native IFS/SWITCH scope

Excel16.0 build20430.0, French UI1036/country33. Both1900/1904 matrices
passed130 observations and260 exact before/after/cache comparisons, cleanup
true. Separately eleven fresh owned-workbook Worksheet.CircularReference
controls passed with iteration disabled and cleanup true.

All conditions/keys are dependencies, even after a prior match: later IFS
condition/self and later SWITCH key/self are circular. Unselected result or
default self-references are not circular. Native values from circular cases
are transport observations only; Rust must retain its prior cache and report
actual SCC membership instead of adopting Excel's default zero.

The selected branch retains reference and explicit-intersection origin.
Later error and unknown-function outcomes do not override an earlier match.
The first tested error and selected-result error propagate. Unmatched calls
return #N/A; explicitly omitted values become zero.

Rust fixture targets120 exact computed outcomes and10 explicit unresolved
locale-text comparisons/coercions (six IFS logical text, two punctuation
comparisons, two nonidentical Unicode comparisons). Every observation remains
listed. This does not establish arbitrary locale collation or array behavior.

See fixture JSON for exact input/report hashes, raw COM observations, native
saved caches and the owned-cycle control lifecycle. Primary references:
https://support.microsoft.com/en-us/excel/functions/ifs-function
https://support.microsoft.com/en-us/excel/functions/switch-function
https://learn.microsoft.com/en-us/office/vba/api/excel.worksheet.circularreference
