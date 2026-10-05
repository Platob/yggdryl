# IF / IFERROR / IFNA / CHOOSE native observations

Excel16.0 build20430.0, French UI1036/country33. All120 cells were observed,
saved, reopened and compared through240 exact before/after/cache observations;
the guarded desktop run passed and cleanup completed. The JSON retains the
input/report/manifest hashes and raw COM transport alongside cached XML.

Rust matches116 exact typed results and explicitly holds four IF direct-text
coercions: French #VALUE! results do not settle en-US text interpretation.
This is scoped evidence, not whole-workbook or whole-function conformance.

Observed:
- IF evaluates only the selected branch. Absent false argument is Boolean
  FALSE; an explicitly empty selected argument is numeric0. Empty condition
  selects false. Inactive self/mutual references and unknown functions do not
  prevent the selected result from computing.
- IFERROR catches #DIV/0! and #N/A; IFNA catches only #N/A. They return values,
  not reference identity. Empty selected fallback is numeric0; empty text stays
  text. Unused fallback unknown functions are skipped.
- CHOOSE truncates1.9 to1, accepts TRUE and numeric text2, rejects zero/negative/
  past-end indices with #VALUE!, and evaluates only the selected alternative.
  An explicitly empty selected alternative is numeric0.
- ISREF(IF(...reference...)) and ISREF(CHOOSE(...reference...)) are TRUE;
  equivalent IFERROR/IFNA calls are FALSE. SUM over an IF/CHOOSE selected B1:B3
  returns60. At a host outside B1:B3, IFERROR scalar-intersects to #VALUE! and
  substitutes0; IFNA preserves that #VALUE!. Both epoch observations agree.

True active cycles were not authored. The public cycle regression separately
uses the core contract: preserve prior caches, report actual SCC members, and
restore computation when the selected branch stops requiring the cycle.
Excel's initial zero cache is never used as a cycle oracle.

The twelve mirrored/public selector tests and two graph tests pass. Repeated
scalar, sparse-range and suspended-chain passes allocate zero at 64 and 4,096
formulas. The loaded-package I/O pin adds zero source reads. The complete Excel
suite at this boundary passed 1,244 tests; two local export helpers were ignored.
