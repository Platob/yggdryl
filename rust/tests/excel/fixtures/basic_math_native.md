# SIGN, INT, TRUNC and PI: native observations

Both fixtures were collected with the guarded active-session oracle against
Microsoft Excel 16.0 build 20430.0, Windows 64-bit, UI LCID 1036 and country
setting 33. The oracle opened, calculated, saved and reopened only its owned
workbooks; cleanup completed in both runs. Each JSON retains the source report
and manifest hashes, original formula, saved spelling, native bits and cache.

`basic_math_native.json` contains 154 observations across both date systems.
The native run failed two raw-value/cache comparisons: TRUNC of a computed
subnormal retained that value in memory but saved a zero cache. This run is
not relabelled successful. Rust matches 148 observations exactly and explicitly
holds six formulas whose inputs require the unavailable raw subnormal value.

`basic_math_edge_native.json` contains 190 passing observations around integer,
precision and exponent boundaries. Rust tests both the original and the
Excel-saved formula spelling for every case, matching all 380 results exactly.
Excel can rewrite a sixteen-digit input literal on save. INT and TRUNC first
use the existing fifteen-significant-digit decimal owner; their generic shared
Scalar floor/trunc kernels retain IEEE behavior, including signed zero.
Nonzero TRUNC precision discards decimal digits through the same stack buffer
used by number formatting, without a binary scale-and-divide approximation.

The three public `basic_math` tests pass. The private shared arithmetic test
passes, and the existing warm constant-formula allocation test, extended with
these four functions, measures zero allocations at 64 and 4,096 formulas.
These checks establish the exercised corpus, not all Excel numeric behavior.
