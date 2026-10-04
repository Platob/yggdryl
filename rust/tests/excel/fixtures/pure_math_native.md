# Native scalar math observations

Excel 16.0 build 20430.0 (French UI 1036, country 33) completed all 592
observations and 1,184 exact native/saved-cache comparisons in the 1900 and
1904 workbooks, with cleanup complete. Manifest SHA-256:
`317bd56ab7d23f827c249fdae7e81369e5062d2000cf073c7a543fa7032d8185`.
Report SHA-256:
`040caf9c385690fecf15a1a30a3754fdc9f17b1bf731248dcef096c56c303876`.
`pure_math_native.json` retains every observation; its SHA-256 is
`42863d31b57406ec5b8e9a369c18f3ca4a08926a3324ff85684f487a4ece42b0`.

The first implemented subset is EXP, LN, LOG10, DEGREES and RADIANS, plus
source controls. All 164 observations in `pure_math_first_native.json`
replay with exact cache types/bits in Rust after an observed failing test.
The shared Scalar kernels landed first, with scalar/Arrow expression parity,
typed-boundary tests and zero per-row allocations at 64 and 4,096 rows.
Excel adapts coercion, domain errors and finite-cache policy to those kernels;
it does not bind a generic expression for each cell.

The other eleven functions in the full corpus are native evidence only.
Host-library candidates for several trigonometric functions, LOG and FACT
have observed last-bit differences; some also change fifteen-digit results.
Those observations do not establish a portable exact implementation. GCD/LCM
also distinguish direct and referenced Boolean/blank inputs. Keep these
unresolved domains separate from the five verified functions, without
weakening the exact replay assertions.
