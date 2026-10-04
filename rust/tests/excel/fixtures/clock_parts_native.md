# Clock-part and TIME observations

Excel 16.0 build 20430.0 (French UI) calculated and saved two owned
workbooks. The guarded run passed, cleanup completed, and all 120 typed COM
results agreed with saved cache type/value before and after SaveAs (92 numeric,
28 errors). This is native evidence, not Rust equivalence.

Manifest SHA-256: `48fe0f2d25e4abe0767d63217b6feb63fb821bc4d65eaea2cd33a4638c5c6401`.
1900 input/saved SHA-256: `2a31520933735330a35cd2665337b6b90d1c7986ef793e008f82ca2aa8a78c26` /
`a36843f8457a2578823e6c2d0a956aae717cef1692b13cacf6fef238e7e4d533`.
1904 input/saved SHA-256: `0432000a8927d3952e2391ef22ec30a94605c0435b50a3468aa1f647f3787598` /
`8c44834c262538245f9f836d6a72d54451d0e29c8555ba79b6fe56774cd6f580`.

HOUR, MINUTE and SECOND round to the nearest whole second for the observed
nonnegative serials. At 0.5 seconds SECOND returns 1; 59.9994 seconds returns
the next minute; a serial just 0.0004 seconds before midnight returns
00:00:00. Negative and past-last-day serials return `#NUM!`; explicit `#N/A`
propagates. Styled and blank references retain their source meaning.

TIME truncates each fractional component in the observed 12.9:34.5:56.7
case. Components 25 hours, 61 minutes and 61 seconds wrap into the fraction
of one day. A negative or over-32767 component returns `#NUM!`; `NA()`
propagates. The native cached float bits for wrapped and ordinary TIME results
are retained in the JSON fixture, since equivalent algebraic evaluation can
round to different IEEE values.
