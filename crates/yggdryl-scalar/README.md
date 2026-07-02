# yggdryl-scalar

The scalar container layer of **yggdryl**: one typed value, owned by us, laid
out per the Apache Arrow columnar spec (hard rule 9 in `CLAUDE.md`).

- `Scalar<T>` — a data type plus one element's value bytes in an
  `arrow-buffer` `Buffer`; `None` is null. Extracting a scalar from a larger
  container is a zero-copy buffer slice.
- `ScalarType` — the subtrait tying each schema data type to its one-element
  layout, so construction is validated and invalid scalars are
  unrepresentable.

See the crate's module doc comments for the design, and `CLAUDE.md` at the
repository root for contributor rules.
