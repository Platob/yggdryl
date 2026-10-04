# Native ordered comparison observations

Excel 16.0 build 20430.0, French UI 1036/country 33, observed all 842 listed
cases (421 per epoch). Both date systems gave identical typed caches. Every
before/after SaveAs value agrees with its saved cache; cleanup completed.
The JSON retains raw COM transport, saved formula/cache and input/report hashes.

All six operators share the observed fifteen-digit numeric equivalence:
`0.1+0.2` versus `0.3`, and `1` versus `1+2^-49`, answer equal, neither less nor
greater. A distinct `1+2^-46` remains greater. Mixed kinds order numbers before
text before Booleans. Blank compares as zero, empty text or FALSE according to
the other operand. Referenced values retain their kind; empty formula text is
not numeric zero. Error values propagate, with the left error winning.

Text collation cannot be replaced by byte order or Unicode lowercasing:
native `a-b > ab`, `+ > ^`, `ß = SS`, and decomposed e-plus-accent equals é.
The core slice therefore compares ASCII alphanumeric strings case-insensitively
and recognizes identical strings of any content. Other text pairs retain their
old caches. Coverage is explicit: 698 computed comparisons/source controls and
144 held collation observations. This is not a claim of full Excel collation.

The full replay and numeric/contextual controls passed three Rust tests in
`local-p5-comparisons-green.log`, after two runtime failures before the fix.

The manifest came from `.scratch/build-p5-comparison-probes.py`, using the
existing bounded reference-input producer. Root alone ran the active desktop
harness. Microsoft documents the six operators and Boolean result contract:
https://support.microsoft.com/en-us/excel/using-calculation-operators-in-excel-formulas

Numeric representation context:
https://learn.microsoft.com/en-us/troubleshoot/microsoft-365-apps/excel/floating-point-arithmetic-inaccurate-result
