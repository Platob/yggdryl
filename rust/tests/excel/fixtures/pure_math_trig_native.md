# Native cosine, inverse sine and logarithms

Excel 16.0 build 20430.0 on Windows 64-bit, UI language 1036 and country33.
`pure_math_trig_native.json` retains82 COS/ASIN and source observations from
`pure_math_native.json`, with164 exact Value2/cache comparisons.
`logarithm_native.json` retains62 LOG/source observations; the separate
`logarithm_base_native.json` retains48 arbitrary-base observations and96 exact
comparisons. Each JSON retains source hashes. The guarded harness calculated,
read and saved each owned workbook, compared before/after SaveAs and the saved
OOXML cache, and completed cleanup. It did not reopen these saved workbooks.

The shared bound Float64 kernels own COS/ASIN. Excel alone maps nonfinite
results to NUM and cosine inputs with absolute value at least2^27 to NUM.
`trig_limit_native.json` retains54 COS/SIN/TAN observations at both signs of
that boundary (108 cache comparisons). COS values inside that large boundary
differ from generic argument reduction; the current Excel COS adapter holds
values outside[-pi,pi] while retaining their existing cache. The boundary
Rust test checks the18 COS observations (10NUM refusals and8 held results).
The SIN/TAN observations do not claim an implemented Rust path.

LOG uses the existing decimal-log kernel for base10 and the shared natural
log kernels followed by shared division for other bases. Base1 returns DIV/0;
nonpositive source/base returns NUM. This operation order matches all110
recorded LOG observations exactly, including its binary tails.
