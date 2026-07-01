# yggdryl-scalar

The scalar value layer for yggdryl. A **scalar** is a single value paired with the
schema [`Field`] that describes it, so it carries its `name`, `dtype`, nullability and
`metadata` alongside the value.

The [`Scalar`]`<T>` base trait abstracts primitive scalars (mirroring
`DataType<T>` / `Field<T>`): every scalar exposes `field`, `value`, and — through the
field — `name` / `dtype` / `metadata`. The types wear **simple names** since they live
under the `scalar` namespace: the concrete primitive scalars ([`Int8`]…[`UInt256`])
build from native Rust values, the dynamic [`Any`] does the same at run time (with
atomic accessors like `as_i32`), and the nested [`Struct`] is built from a
**collection** of scalars — the way scalars compose into rows.

[`Field`]: yggdryl_schema::Field
[`Scalar`]: crate::Scalar
