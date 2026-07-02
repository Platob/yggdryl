# yggdryl-array

The array container layer of **yggdryl**: typed columns, owned by us, laid out
exactly per the Apache Arrow columnar spec (hard rule 9 in `CLAUDE.md`).

- `Array` — the abstract base every array implements: a data type, a length
  and a validity bitmap.
- `PrimitiveArray<T>` — the fixed-width implementation over an `arrow-buffer`
  `ScalarBuffer<Native>` plus an optional `NullBuffer` validity bitmap.
  Slicing and `scalar_at` extraction are zero-copy buffer slices.
- One family member per type (`Int64Array`, `Float64Array`, …), each its own
  implementation over the generic engine with constructors that drop the
  redundant data-type argument.

Boolean, variable-size and nested arrays land next, implementing the same
`Array` base. See the crate's module doc comments for the design, and
`CLAUDE.md` at the repository root for contributor rules.
