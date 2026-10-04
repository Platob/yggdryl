# Native temporal formula observations

Excel 16.0 build 20430.0 (French UI) calculated two owned workbooks, one
for each date system, and saved them. The guarded run passed with cleanup
complete. All 248 observed COM Value2 results agree with saved cache type and
value before and after SaveAs (223 numeric, 25 errors). This is native
evidence, not a Rust equivalence result.

Manifest SHA-256: `7b67c56f2810900dcefa49a34967b716cc102cde96c48752a9d249e3c2878d55`.
1900 input/saved SHA-256: `ffdce04ff63e73adce47a95c370757cbdb6db13f542e1ac9692c1636c794dc15` /
`29394d315766ee893311058a2fc796b4216600b7c4d7451ac24119f8c7099ff6`.
1904 input/saved SHA-256: `00bfec577be47b51b223c9e2818284b5033f10c169a30ab0d128bc55834fd4ee` /
`997ed929d061efca3c1ad85b4e3daa8a5daa407a8fd4c823a6cd8d4216afeb30`.

The formatter's existing 1900 phantom calendar agrees with observed
YEAR/MONTH/DAY: serial 0 is 1900-01-00 and serial 60 is 1900-02-29.
WEEKDAY type 1 at 59/60/61 returned 3/4/5; sampled return type 21 returned
`#NUM!`. Every unprefixed `DAYS(...)` returned `#NAME?`; these inputs
do not establish DAYS arithmetic because the function registry marks DAYS
with the future prefix. DATEVALUE/TIMEVALUE strings are French-locale
observations and are not a locale-neutral parser contract.

The compact JSON stores each formula, exact cached text/type, and COM Float64
bits when numeric. Source workbooks and raw saved packages remain in the
native report directory while local; only the compact evidence is promoted.
