# Information functions: 266 native observations

The source report is `rust/target/excel-desktop/p5-information-native-observed/results.json`
(SHA-256 `152ba5180bef48a6833defaf9ab22e46da1527403309a85fcf0d1377b63dca48`).
It records Excel 16.0 build 20430.0, UI language 1036, `passed=true`,
`cleanup_completed=true`, no failures, and 266 observed cases. Every pre-save
and post-save COM result agrees in type/value with the saved XML cache; there
are 218 Boolean, 28 number, and 20 error caches. Formula and Formula2 agree
for all 266 cases, and no Value2 changed after SaveAs. The authored manifest
SHA-256 is `d9c6d67894ffcc6d1a465bcc6d913cbad519bc3f3af871cf317f6b7537a728db`;
the 1900 and 1904 workbook hashes are respectively
`fb2564dbe0068fc230f499f6e968b650246c8ba9f88ea86dc009fa036a1f7801`
and `a33847d3e7d6d7c91619ad4bf61b03142514ca1192b39eb143fd5f41cf91cb70`.
`.scratch/p5-information-native-fixture.json` contains all 266 exact answers,
typed Value2 transports, formulas, saved caches, and source-cell provenance;
its SHA-256 is `4d5d3e9055ebc99dea8f2e6a268f89fa80e4d7412599115f8dac28d8a6efa410`.
The original run records native execution. The later Rust replay matches all
266 typed caches, including the 32 reference-identity cases
(`local-p5-isref-geometry-green.log`: 10 information tests passed).

In both date systems, the five value classifiers follow typed operands:
ISBLANK is true only for the physically absent A9 reference; A8, a formula
returning empty text, and literal `""` are false. ISLOGICAL is true for Boolean
direct/reference arguments only. ISNUMBER is true for number direct/reference
and the date-styled A12 reference. ISTEXT is true for text, including empty
text; ISNONTEXT is its observed complement across all tested types, including
blank and errors. Each consumes #N/A or #DIV/0! as a value and returns a
Boolean rather than propagating it.

ISERR returns true for #DIV/0! and false for #N/A; ISERROR returns true for
both; ISNA returns true only for #N/A. ERROR.TYPE returns numeric 2 for
#DIV/0!, 7 for #N/A, and #N/A for tested nonerrors (text, number, blank).
These outcomes match direct errors and referenced computed errors. `NA()`
returns #N/A. `N` preserves numbers, maps Boolean true/false to 1/0, maps
numeric/empty text and blank to 0, propagates #N/A, and converts date-styled
A12 to epoch-specific serial 45292 (1900) or 43830 (1904).

ISEVEN/ISODD truncate tested fractions toward zero: -3.7 is odd, 2.9 is
even. They accept numeric text `"2"` direct and referenced, map blank to zero,
reject Boolean TRUE with #VALUE!, and propagate referenced #DIV/0!. This
corpus does not settle their broader text grammar or extreme floating parity.
ISREF distinguishes reference origin: direct scalar/error expressions false;
single-cell references true even if their target is blank or error; grouped
and range references true; `Inputs!A1+0` false. Its range/reference identity
consumer must not be conflated with scalar coercion or SUM's value-reading
range policy. Supplemental safe/self-reference and 3-D fixtures establish
the geometry behavior used by the shared dependency policy. Numeric children
still require value edges. The 22 unsettled `@`/SINGLE identity cases remain
uncomputed; their provenance survives grouping and defined-name memoization.

The Rust slice exactly matches five value classifiers including dated ISNUMBER (112),
ISERR/ISERROR/ISNA (42), ERROR.TYPE for the two observed errors and tested
nonerrors (14), N including the date-styled reference (24), NA (2), and
ISEVEN/ISODD (40), and ISREF (32). The workbook calculation owner passes exact date serials
as typed `Operand::Number`; that path has its own native 12-case replay.
Use one typed Operand classification boundary and the existing evaluator and
Node policy owners; do not bind a generic Term per cell. `Scalar::is_null`,
`Scalar::is_number`, and generic `Term::is_null` already own generic null and
number tests, but their semantics cannot infer formula-reference identity or
error-as-value. The exact raw date serial is already supplied before this
classification boundary.
