# Native numeric text conversion

Excel16.0 build20430, UI1036; three successful native runs cover676 cases and
1352 exact native value/cache comparisons. Each observation is before and after
SaveAs in the same open owned workbook, not after reopening. All three runs
reported cleanup completed. The JSON preserves each source hash and raw cache.

The576-case signed mantissa/exponent sweep supports the existing General
formatter with20 output characters, sign outside that budget: decimal intake
and fifteen-digit rounding stay in Digits; fixed/scientific selection stays in
the existing general owner. Formula conversion must not use width11 display
General or the formula-bar entry_number point-only threshold. A Python decimal
projection agreed with the576 observations; Rust tests, not that projection,
establish Rust equivalence. Boolean-to-text remains locale-dependent and held.

The84-case source also pins REPT capacity in UTF-16 units and SUBSTITUTE output
overflow. CLEAN/TRIM controls involving CHAR are retained observations but
explicitly held by the current function slice until CHAR and concatenation
are implemented. All676 observations are consumed by computed/held assertions.
