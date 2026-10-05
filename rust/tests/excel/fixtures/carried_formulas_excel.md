# Excel carried-formula cut comparison

On 2026-10-01, Excel for Windows 16.0 build 20430.0 opened three Rust cut outputs without repair, saved each once as `.xlsx`, and reopened each. The same SaveAs cycle was applied to the native Excel outputs. The compared `Data` and `Other` sheet snapshots had the same cells, conditional formats, and data validations after the cycle; each case returned no visible difference. The Rust packages had positive unique conditional-format priorities and matching linked legacy/x14 GUID sets both before and after Excel saved them. The active Excel session was retained and owned workbooks were closed (`cleanup_completed=true`).

| Case | Saved-before SHA-256 | Result |
| --- | --- | --- |
| Cross-sheet partial CF, interior | `3b9f28807c338968ba3304912983e9d80bb64997cecde6beccffbdaf5a38207d` | Match after equal SaveAs cycle |
| Cross-sheet partial DV, interior | `efdd8d802406adcd09da43440d78a92a852dbef21d6692f39bfa605605903826` | Match after equal SaveAs cycle |
| Same-sheet multi-component CF | `5dd53e78d2d091b05cb68e5a2d149031fd866d980a63cab454fc39eeb8f5ccb8` | Match after equal SaveAs cycle |

The fixture source is the corresponding `*.before.xlsx` under the native reports `carried-retained-formulas` or `carried-scalar-disjoint`. The Rust exporter and COM inspector are `.scratch/export-carried-formula-native.ps1` and `.scratch/compare-carried-formula-native.py`; the recorded run is `rust/target/excel-desktop/carried-formula-compare-active/results.json` (`passed=true`, `failures=[]`). The saved native and Rust XML is intentionally not byte-identical: Excel rewrites priorities during SaveAs. This comparison proves observable parity for these three cases and package acceptance, not every possible formula or partial-owner shape.
