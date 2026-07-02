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
- `TimeUnit` — the abstract base for type-level time units, implemented by one
  unit struct per resolution (`Nanosecond` through `Year`) plus the erased
  `AnyTimeUnit` (and the width-restricted `Time32Unit` / `Time64Unit` markers
  with `AnyTime32Unit` / `AnyTime64Unit`); `TimeUnitId` is the value-level
  identifier.
- `Timestamp` / `Time` / `Date` / `Duration` — the abstract temporal bases.
  `TypedTimestamp<U>` and `TypedDuration<U>` implement the first two for every
  unit — Arrow's four native units map directly, the coarser ones anchor on
  `Int64` plus the `ygg.*` field metadata (see the `metadata` module, the
  single source of truth for those keys). `Time32<U>` / `Time64<U>` implement
  `Time` for the units each width holds, and `Date32` / `Date64` implement
  `Date` at day and millisecond resolution.
- One module per type category (`integer`, `float`, `decimal`, `string`,
  `binary`, `temporal`, `list`, plus `Struct` and `Map`), one file per type.
- `Field` — the abstract base for schema fields (name, data type, nullability,
  metadata, plus the provided Arrow and byte conversions); the generic
  `TypedField<T>` is the implementation covering every data type.

See the crate's module doc comments for the design, and `CLAUDE.md` at the
repository root for contributor rules.
