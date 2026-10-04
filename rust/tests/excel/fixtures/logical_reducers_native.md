# AND / OR / XOR native observations

Excel 16.0 build 20430.0, French UI 1036, country 33. The guarded active
desktop run observed all 190 listed cells and restored application state.
All 380 before/after-save to cached-XML comparisons matched exactly. Workbook,
manifest, and source-report hashes are retained in the JSON fixture.

The Rust equivalence scope is 166 typed numeric, Boolean, range, error, and
source-control results. The other 24 direct-text calls remain explicitly
uncomputed: French native #VALUE! results do not establish en-US text coercion.
Referenced text and blanks are ignored; referenced Boolean values participate.
With no admitted logical values the result is #VALUE!. Errors propagate even
after a decisive FALSE in AND or TRUE in OR, and the first explicit error wins.

The fixture covers both date systems and checks force/idle recalculation.
It does not establish omitted-argument, array, or lazy selector behavior.
Root ran scripts/check_excel_desktop.py functions --active against the exact
manifest whose hash is recorded in the fixture. No Rust equality is implied
by a successful desktop run; the separate Rust test proves the scoped results.
