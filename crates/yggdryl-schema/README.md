# yggdryl-schema

The Arrow-centralized schema layer of **yggdryl**: typed data types and fields.

- `DataType` — the base trait every concrete type implements: Arrow interop
  (`to_arrow` / `from_arrow`) and byte round-trips (`to_bytes` / `from_bytes`).
- `PrimitiveType` / `LogicalType` / `NestedType` — the category subtraits tying
  a type to its native Rust value, its physical anchor, or its child fields.
- One module per type category (`integer`, `float`, `decimal`, `string`,
  `binary`, `temporal`, `list`), one file per type.
- `Field<T>` — a named, typed schema slot mapping to `arrow_schema::Field`.

See the crate's module doc comments for the design, and `CLAUDE.md` at the
repository root for contributor rules.
