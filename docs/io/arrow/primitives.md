# Arrow interop — primitives

The fixed-width numeric family (see [Types → Fixed-width](../fixed/index.md)) splits into two Arrow
behaviours:

- **The native subset — zero-copy.** `u8`…`i64` and `f16`/`f32`/`f64` have a real Arrow
  `PrimitiveType`, so a `Buffer` / `Serie` converts to and from an `arrow_array::PrimitiveArray` by
  **sharing the allocation** (an `Arc` bump, never a payload copy). This is the `ArrowNative`
  capability.
- **The wider widths — closest-fit schema.** `u96`/`i96`, `u128`, `u256` have no Arrow primitive,
  so `to_arrow()` returns the **closest optimized representation**: `Decimal128(38,0)` /
  `Decimal256(76,0)` for `i128`/`i256` (a scale-0 decimal is an integer), and `FixedSizeBinary(N)`
  for `u128`/`u96`/`i96`/`u256`. These mappings are **lossy** — `FixedSizeBinary` drops the integer
  tag, and a scale-0 decimal under-covers the very top of the integer's range — so the exact type is
  recovered from [metadata](metadata.md).

=== "Python"

    ```python
    from yggdryl.types import I32Serie

    # The numeric columns don't expose a direct Arrow-array bridge; they interop across
    # languages through the byte codec — the same bytes the Rust core reads to build (and
    # is built from) its Arrow arrays.
    col = I32Serie([1, None, 3])
    assert I32Serie.deserialize_bytes(col.serialize_bytes()) == col
    # (The zero-copy pyarrow bridge is on the nested and temporal columns — see those pages.)
    ```

=== "Node"

    ```js
    const { I32Serie } = require('yggdryl').types

    // Node has no Arrow-array bridge; the shared wire form is the byte codec.
    const col = new I32Serie([1, null, 3])
    assert(I32Serie.deserializeBytes(col.serializeBytes()).equals(col))
    ```

=== "Rust"

    ```rust
    # #[cfg(feature = "arrow")]
    # fn demo() {
    use yggdryl_core::io::DataType;
    use yggdryl_core::io::fixed::{Buffer, PrimitiveType, Serie, TypedField};

    // Buffer <-> Arrow: zero-copy (a shared Arc).
    let buffer = Buffer::<i32>::from_vec(vec![1, 2, 3, 4]);
    let array = buffer.to_arrow_array();                       // PrimitiveArray<Int32Type>
    let back = Buffer::<i32>::from_arrow_array(&array);
    assert!(back.to_arrow_buffer().ptr_eq(array.values().inner())); // same allocation

    // Serie <-> Arrow: values zero-copy, validity (LSB-first, 1 = valid) preserved.
    let col = Serie::from_options(&[Some(1i32), None, Some(3)]);
    assert_eq!(Serie::<i32>::from_arrow_array(&col.to_arrow_array()), col);

    // DataType and Field convert to/from Arrow's schema types.
    assert_eq!(PrimitiveType::<i32>::new().to_arrow(), arrow_schema::DataType::Int32);
    let arrow_field = TypedField::<i64>::new("id", false).to_arrow();
    assert_eq!(TypedField::<i64>::from_arrow(&arrow_field), Some(TypedField::new("id", false)));
    # }
    ```

## The closest-fit mapping

`to_arrow()` is **total** — it never fails, returning the exact primitive or the closest optimized
representation:

| yggdryl type | Arrow `DataType` | zero-copy? | lossy? |
| --- | --- | --- | --- |
| `u8`…`u32`, `i8`…`i64` | `UInt8`…`Int64` | yes (`ArrowNative`) | no |
| `f16` / `f32` / `f64` | `Float16` / `Float32` / `Float64` | yes (`ArrowNative`) | no |
| `i128` / `i256` | `Decimal128(38,0)` / `Decimal256(76,0)` | schema only | tops of range |
| `u128` | `FixedSizeBinary(16)` | schema only | drops integer tag |
| `u96` / `i96` | `FixedSizeBinary(12)` | schema only | drops integer/sign tag |
| `u256` | `FixedSizeBinary(32)` | schema only | drops integer tag |

The native paths allocate **nothing** for the buffer and dense-column value paths (see the
[benchmark notes](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/arrow.md)). A
lossy mapping still round-trips exactly because `to_arrow` tags the field — see
[Metadata & round-tripping](metadata.md). Fixed-size byte columns (`FixedBinary` / `FixedUtf8`) map
to `FixedSizeBinary(N)` and are covered there too.
