# yggdryl-schema

The Arrow-flavoured schema layer for yggdryl. Defines the `DataType` base trait
(`type_id` / `type_name`) and its category markers `PrimitiveType`, `LogicalType`
(`inner_type`) and `NestedType` (`children_fields` / `child_field_at` /
`child_field_by`), the `DataTypeId` discriminant, the `Field` type (name + data type
+ optional byte-keyed metadata, with functional `copy` / `with_*` updates), and the
first concrete type, `BinaryType`.

See `CLAUDE.md` for the contributor rules.
