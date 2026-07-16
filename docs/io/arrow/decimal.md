# Arrow interop — decimals

The columnar decimal types (see [Types → Decimals](../fixed/decimal.md)) fix one
`(precision, scale)` per column — Arrow's exact model — and store raw two's-complement
coefficients. So a `DecimalSerie` converts **zero-copy** to and from Arrow's decimal arrays: the
coefficient allocation is shared (an `Arc` bump), and the array carries the column's precision and
scale.

| width | value type | Arrow array | zero-copy |
| --- | --- | --- | --- |
| `d32`  | `D32`  | `Decimal32Array`  | yes |
| `d64`  | `D64`  | `Decimal64Array`  | yes |
| `d128` | `D128` | `Decimal128Array` | yes |
| `d256` | `D256` | `Decimal256Array` | yes |

`d128`/`d256` map to `Decimal128`/`Decimal256`; on Arrow 56 `d32`/`d64` map to the native
`Decimal32`/`Decimal64`. The mapping is **lossless**, but the byte width alone cannot express
`(precision, scale)`, so an erased `Field` carries them in reserved metadata keys — see
[Metadata & round-tripping](metadata.md).

=== "Python"

    ```python
    from yggdryl.decimal import D128Serie

    # The decimal columns interop across languages through the byte codec (the same bytes the
    # Rust core reads to build its zero-copy Decimal128Array).
    col = D128Serie(20, 2, ["123.45", None, "6"])
    assert D128Serie.deserialize_bytes(col.serialize_bytes()) == col
    ```

=== "Node"

    ```js
    const { D128Serie } = require('yggdryl').decimal

    // Node has no Arrow-array bridge; the shared wire form is the byte codec.
    const col = new D128Serie(20, 2, ['123.45', null, '6'])
    assert(D128Serie.deserializeBytes(col.serializeBytes()).equals(col))
    ```

=== "Rust"

    ```rust
    # #[cfg(feature = "arrow")]
    # {
    use yggdryl_core::io::fixed::{D128, D128Serie};

    let col = D128Serie::from_options(20, 2, &[Some(D128::new(12345, 2).unwrap()), None]).unwrap();
    let array = col.to_arrow_array();               // zero-copy Decimal128Array
    assert_eq!((array.precision(), array.scale()), (20, 2));
    assert_eq!(D128Serie::from_arrow_array(&array), col);
    # }
    ```

Value arithmetic and identity are stack-only (no per-op allocation, even at 256 bits); the array
conversion shares the coefficient buffer (see the
[benchmark report](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/decimal.md)).
