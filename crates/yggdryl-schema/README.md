# yggdryl-schema

The Arrow-centralized schema layer of **yggdryl**: typed data types and fields.

- `DataType` — the base trait every concrete type implements: Arrow interop
  (`to_arrow` / `from_arrow`), byte round-trips (`to_bytes` / `from_bytes`) and
  the stable constructor identifier (`type_id`).
- `DataTypeId` — the append-only integer id of each type constructor, shared by
  every parameterization (`DataTypeId::List` for any `List<T>`).
- `PrimitiveType` / `LogicalType` / `NestedType` — the category subtraits tying
  a type to its native Rust value, its physical anchor, or its child fields.
- `AnyDataType` — the erased data type: one variant per constructor, so
  heterogeneous collections (struct fields, map entries) hold any type.
- One module per type category (`integer`, `float`, `decimal`, `string`,
  `binary`, `temporal`, `list`, plus `Struct` and `Map`), one file per type.
- `Field` — the abstract base for schema fields (name, data type, nullability,
  metadata, plus the provided Arrow and byte conversions); the generic
  `TypedField<T>` is the implementation covering every data type.

See the crate's module doc comments for the design, and `CLAUDE.md` at the
repository root for contributor rules.
