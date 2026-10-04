# AVERAGE, AVERAGEA, MINA, MAXA and PRODUCT

Microsoft Excel 16.0 build 20430.0 (Windows 64-bit, UI LCID 1036) produced
408 observations across both date systems. The guarded run passed, cleanup
completed, and all 816 raw/after-save comparisons matched the saved caches.
Report SHA-256: `f5515756df2110ec5faeac260ad39d8a11c3ed3975a8b8d180c63574bfceffdf`.
Manifest SHA-256: `e2e29de3b93595dc6a45842497a3db36b622a13dfe160baaf5507b6cb1131a9a`.
The paired JSON retains every formula, typed cache, numeric bits, source cell
and workbook hash. Rust's public workbook replay matches all 408 observations;
the aggregate module's 13 targeted tests pass, including the prior SUM cases.

Input order matters. `AVERAGE(1E16,1,-1E16)` returns zero; exchanging the last
two arguments returns binary64 one third. PRODUCT returns zero for
`(0,9E307,9E307)` but `#NUM!` for `(9E307,9E307,0)`. Its intermediate underflow
controls publish zero. The private refusal test proves that an overflowing
product push leaves its last finite prefix unchanged; the evaluator retains
the first error for the complete formula.

AVERAGE and PRODUCT ignore referenced text and Booleans. The A-family counts
referenced text as zero and Booleans as zero/one. Direct numeric text follows
the direct-argument coercion rule instead. MINA/MAXA preserve the native raw
binary extrema even when the comparison operator's fifteen-digit rule would
call the two inputs equal. All five functions use the existing Accumulator;
the reference reader maps A-family text/Booleans before rendering or cloning.

These observations do not cover array or omitted arguments. Subnormal source
values for MINA/MAXA/PRODUCT and a subnormal final AVERAGE quotient remain
explicitly uncomputed pending the separate raw-value/cache policy. The corpus
also does not establish precedence between numeric overflow and a different
later explicit error.
