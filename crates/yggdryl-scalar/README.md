# yggdryl-scalar

The value + dynamic-schema layer for yggdryl. A **scalar** is a value: a native Rust
primitive (`i8`…`u128`, `I256` / `U256`), the dynamic [`Any`], or a nested [`Struct`]
(an array of `Any`).

The generic [`Scalar`] trait unifies them — every scalar reports its `type_id`, whether
it `is_null`, and promotes to [`Any`] via `to_any` — so nested structures hold and build
from any scalar uniformly (e.g. `Struct::from_scalars([1i32, 2i32])`). The schema that
describes the values ([`AnyType`] / [`AnyField`] and the nested [`StructType`] /
[`StructField`]) is generic over these scalar values, and an Arrow schema is a
[`StructField`].

[`Scalar`]: crate::Scalar
[`Any`]: crate::Any
[`Struct`]: crate::Struct
[`AnyType`]: crate::AnyType
[`AnyField`]: crate::AnyField
[`StructType`]: crate::StructType
[`StructField`]: crate::StructField
