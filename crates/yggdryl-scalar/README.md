# yggdryl-scalar

The **Scalar** layer of [yggdryl](https://github.com/Platob/yggdryl) — a single,
always-present value of a [`yggdryl-dtype`](../yggdryl-dtype) data type as a trait
hierarchy (`Scalar` / `TypedScalar<DT, T>`, the `PrimitiveScalar` category, and
logical/nested scaffolding) with concrete primitive scalars (`I64Scalar`,
`BooleanScalar`, …) that round-trip through little-endian bytes. Scalars are
non-nullable — nullability is modelled separately (a `NullType` value and, later,
union types).

Depends on `yggdryl-field`, `yggdryl-dtype`, and `yggdryl-core`.
