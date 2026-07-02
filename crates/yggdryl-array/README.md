# yggdryl-array

The array container layer of **yggdryl**: typed columns, owned by us, laid out
exactly per the Apache Arrow columnar spec (hard rule 9 in `CLAUDE.md`).

- `Array` — the abstract base every array implements: a data type, a length
  and a validity bitmap.
- `PrimitiveArray<T>` — the fixed-width implementation over an `arrow-buffer`
  `ScalarBuffer<Native>` plus an optional `NullBuffer` validity bitmap.
  Slicing and scalar extraction are zero-copy buffer slices.

Boolean, variable-size and nested arrays land next, implementing the same
`Array` base. See the crate's module doc comments for the design, and
`CLAUDE.md` at the repository root for contributor rules.
