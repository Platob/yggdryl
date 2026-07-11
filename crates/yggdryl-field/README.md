# yggdryl-field

The **Field** layer of [yggdryl](https://github.com/Platob/yggdryl) — a named,
nullable [`yggdryl-dtype`](../yggdryl-dtype) data type as a trait hierarchy
(`Field` / `TypedField<DT, T>`, the `PrimitiveField` category, and logical/nested
scaffolding) with concrete primitive fields (`I64Field`, `BooleanField`, …) that
convert to and from `arrow_schema::Field`.

Depends on `yggdryl-dtype` and `yggdryl-core`.
