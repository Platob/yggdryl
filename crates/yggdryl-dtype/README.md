# yggdryl-dtype

The **DataType** layer of [yggdryl](https://github.com/Platob/yggdryl) — Apache Arrow
data types as a trait hierarchy (`DataType` / `TypedDataType<T>`, the `PrimitiveType`
category, and logical/nested scaffolding) with concrete primitive types
(`I64Type`, `F64Type`, `BooleanType`, …) that convert to and from
`arrow_schema::DataType`.

Depends only on `yggdryl-core`.
