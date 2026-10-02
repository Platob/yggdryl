# Native temporal serial observations

Excel 16.0 build 20430.0 (UI language 1036)
calculated only the two owned `Cases` sheets through the existing functions
harness. The run passed 12/12 cases, both before/after SaveAs cache comparisons
for each case (24/24), with no failures and `cleanup_completed=true`.

The authored and Excel-saved worksheet XML retain A2=59, A3=60, A4=60.5,
and A5=45292.000000001 at identical parsed IEEE-754 bits in both date systems.
The source XML text may be re-spelled; the bits are the assertion. B3's `=A3`
returns numeric 60 and displays `2/29/00` in the 1900 system, while the same
numeric serial displays `3/1/04` in the 1904 system. B6=`A3-A2` is 1. B5's
reference retains IEEE bits `40e61d8000000089`; the saved cache spells
`45292.000000000997`. B7's residual is `9.9680619314312935E-10`.

B4 and B5 show `########` at the authored column width; that is a display
width limitation, not a numeric error. The report's Value2 bits and saved `<v>`
are the evidence for those cells. This is native execution evidence only:
`rust_equivalence_checked=false`, and the Rust serial regression test has not
been run here.

Inputs: `b74a3d89ed02fba4ef7866d5015938de4e8eff6c819bda44a15703e28c174b1b` (1900),
`bce5ab5a2ee528ac7b8123ef15e4d671b74c9a63d3163218047ba31216dbf048` (1904). Excel-saved packages:
`e275ba096e76a3a8311a9f3c32c1b394cfb8b0404d20ef4ca802257e5afc8488` (1900),
`f752e53b89e227d646f69730445a696dd5e1caddaf19ff2379cc07dd645e0ac7` (1904).
Manifest SHA-256: `31b6c2e2c33a9a65cde58ae57f5e91dc56d1368c49b0922b60be89500ec2f166`. Results SHA-256: `091e4fb13e32c85c6f70a62d7e89ceadb7aaa6d0e509970da5def87703627383`.
Compact machine-readable observations: `temporal_serial_native.json`.
