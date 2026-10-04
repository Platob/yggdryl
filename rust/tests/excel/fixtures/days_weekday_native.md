# Prefixed DAYS and WEEKDAY mode observations

Excel 16.0 build 20430.0 (French UI) calculated and saved two owned
workbooks. The guarded run passed, cleanup completed, and all 140 typed COM
results agreed with saved cache type/value before and after SaveAs (89 numeric,
51 errors). This is native evidence, not Rust equivalence.

Manifest SHA-256: `f003397ee8b6dded6782cf7acde35d96d6e1b83bf4baf310aa7fdebb6bab4d3f`.
1900 input/saved SHA-256: `5fc9146094401df308bef13a897f597118c5bd867e83e7593026d9aab6141016` /
`59e37d2c1064f61db3f81f81f2a599902f55876060fa31c637262e91a8533cbd`.
1904 input/saved SHA-256: `f1ca53fd5248642ed55d4e9d772aacb3f50776ec85ac7ea52b3db2d94a879fa9` /
`1891d74a7788da9f8b17cfc203151dee791d43b9a082669df15b669d08a5f1ba`.

The OOXML spelling `_xlfn.DAYS` computes a signed difference of whole serial
days for the observed scalar, styled-reference and ISO-text cases. Fractional
60.5 minus 59.25 yields 1. Blank references coerce to zero; negative or
past-last-day serials yield `#NUM!`; bad text yields `#VALUE!`; explicit
errors propagate. The earlier unprefixed `DAYS` cases all yielded `#NAME?`
and never established this arithmetic.

WEEKDAY modes 12 through 16 select Tuesday through Saturday as day 1.
Fractional return codes 1.5, 2.5 and 11.9 truncate to 1, 2 and 11 in all
four observed serials and both epochs. Modes 0, 10, 18, 20 and 21 yielded
`#NUM!`. Modes not present in either temporal native fixture remain outside
the proved behavioral boundary.
