# Empty pivot cache: native acceptance of one controlled package

The control is Excel 16.0 build 20430.0's own five-pivot workbook (SHA-256 cdb20d3f40f5b5ac7f14ee00d2ecdb839b8927e9d4907bfbcdc2bb937a4a054b). The candidate differs only in the selected pivot's cache definition and records parts: `recordCount=0 saveData=0 refreshOnLoad=1` and an empty `pivotCacheRecords count=0`. The input workbook hashes and the two-member difference were checked before native opening.

The guarded active-Excel oracle opened both owned workbooks without reported repair, required `RefreshTable=True`, saved separate owned copies, reopened them and completed cleanup. The selected `P6_source_order_grand` pivot showed East 0, West 3, grand 3 and `$A$3:$B$6` before manual refresh and after refresh/save/reopen in both books. The oracle snapshots only this selected pivot, so the result does not claim equivalence for the other four pivots.

Excel's SaveAs changed the candidate definition to `recordCount=4` and retained `saveData=0 refreshOnLoad=1`, but removed the empty records part, its relationship part and the definition's `r:id`. This establishes acceptance and a correct selected pivot after refresh for this candidate, not byte-preservation of the empty cache or acceptance of every future Rust-authored pivot part. The complete typed observations and hashes are in `pivot_empty_cache_native.json`.
