# Bounded exact variance domain

The192-case native run `p5-variance-exact-native` passed with cleanup completed.
It covers all eight VAR/STDEV spellings in both date systems. It retains raw
COM observations and saved XML; before/after SaveAs use the same owned book.
Earlier floating-order disagreement reports remain valid and are not replaced.

One Accumulator mode admits finite integral inputs only. Exact signed sum,
sum-of-squares, sum squared, n*sumsquares and n*n stay within2^53. The final mean
must be dyadic: n's odd factor divides the sum. Scale by the remaining binary
denominator; the scaled sum-of-squares must also stay within2^53. Therefore every
centered difference and squared term is exactly representable, all nonnegative
partial sums remain exact, and the centered sum equals the raw moment expression.
The final division alone rounds. STDEV then calls the existing shared square root.
Widened integer guard arithmetic prevents a rounded2^53+1 from passing the bound.
The plan holds only one optional square sum alongside existing prefix/count/flags;
it requires neither retained values nor a second source pass.

Last-bound (+/-47453132), translations, dyadic means, empty, singleton and constant
controls agree exactly. Past-bound (+/-47453133) is still finite in native Excel,
but this proof's population cross-product bound fails; the old cache remains held.
Nonintegral inputs, nondyadic means and unproved larger moments likewise return
Uncomputed(NumericPolicy), never an inferred magnitude switch or fitted correction.
Empty/singleton count errors and input errors retain their observed error semantics.

Public runtime checks consume all192 new observations plus192 primary variance
observations, each in original and native-saved spelling:576 exact checks and192
explicitly held checks. All nine variance/SUBTOTAL/atomicity tests passed in
`handoff/local-p5-variance-exact-green.log`. Outside-domain native answers remain
in the fixture; their Rust checks require preservation of the prior cache.
The pure prototype also checked earlier stable/scale inputs:252 admitted numeric
and32 count-error observations in total, with no mismatches; that is not Rust proof.
