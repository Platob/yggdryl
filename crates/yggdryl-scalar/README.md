# yggdryl-scalar

The scalar value layer for yggdryl. A **scalar** is a single value paired with the
schema [`Field`] that describes it, so it carries its `name`, `dtype`, nullability and
`metadata` alongside the value.

The [`Scalar`]`<T>` base trait abstracts primitive scalars (mirroring
`DataType<T>` / `Field<T>`): every scalar exposes `field`, `value`, and — through the
field — `name` / `dtype` / `metadata`. The concrete primitive scalars
([`Int8Scalar`]…[`UInt256Scalar`]) build from native Rust values, the dynamic
[`AnyScalar`] does the same at run time, and the nested [`StructScalar`] is built from
a **collection** of scalars — the way scalars compose into rows.

[`Field`]: yggdryl_schema::Field
[`Scalar`]: crate::Scalar
