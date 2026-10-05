# Lookup fallback and key-axis native evidence

Excel 16.0 build 20430.0, UI language 1036; existing active instance, owned 1900/1904 input workbooks. The run passed, cleanup completed, and all 36 before/after SaveAs cache comparisons agreed with saved XML.

Input manifest SHA-256 `53d35a30112a2ed1bd6a2e5c70a8568c03ad9b26bdacf425f2fdf74b494e76f6`; native report SHA-256 `4ffee6ea40dbed47354b016271510e760f75ecce605b4cbaadb7a45c7d55494f`. The JSON fixture carries each formula, native Value2, and saved cache. This is native evidence only; Rust equivalence requires the mirrored test to run.

Both epochs returned 10 for found `XLOOKUP(1, ..., 1/0)` and `XLOOKUP(1, ..., Values!C1)` despite the error fallback. Missing-key `XLOOKUP(0, ..., 1/0)` returned `#DIV/0!`. VLOOKUP and HLOOKUP returned 10 with an error cell outside the selected return axis. Exact MATCH returned position 1 with a later error in its lookup array. These results bound fallback selection and scan order; they do not by themselves prove how circular graph edges are scheduled.
