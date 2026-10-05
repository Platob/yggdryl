Excel 16.0 build 20430, French UI 1036, active attached session. Two owned
Excel-saved pivot packages were each opened, refreshed, saved, and reopened;
both reports passed with cleanup true and no failures. The control/candidate
packages differ only in `xl/pivotTables/pivotTable1.xml`.

In the first candidate, `dataCaption="CUSTOM_VALUES"`,
`grandTotalCaption="CUSTOM_GRAND"`, and `item@n` for Boolean cache indexes 2
and 3 survive refresh and SaveAs. The visible Boolean item labels and grand
label become the explicit strings after refresh and remain so after reopen.
The second candidate sets `item@n="CUSTOM_BLANK"` on the cache's blank item;
the visible blank label becomes that string and survives refresh/reopen.
Unmodified controls retain locale-provided French labels. The input worksheet
cache is an observation before RefreshTable, not an equality claim against
the refreshed result. The compact fixture records input/saved pivot attributes,
label variants, and SHA-256 hashes for both reports and packages.

Source reports:
`rust/target/excel-desktop/p6-pivot-label-native/results.json` and
`rust/target/excel-desktop/p6-pivot-blank-label-native/results.json`.
Compact artifact: `.scratch/pivot_caption_native.json` (copy to
`rust/tests/excel/fixtures/pivot_caption_native.json` after review).

The English-label collision control reuses the same saved native pivot: an explicit Boolean item `n="TRUE"` coexists with a text item `TRUE`. Excel 16.0 (French UI) retained both equal-looking visible labels through RefreshTable, SaveAs and reopen; it did not rename either item. The raw report remains at `rust/target/excel-desktop/p6-pivot-english-label-native/results.json`.
