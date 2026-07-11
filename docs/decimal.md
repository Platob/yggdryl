# Decimals

A **fixed-width decimal** is an integer **mantissa** scaled by a power of ten, so the
represented value is `mantissa × 10^(−scale)`. `yggdryl` exposes the four widths matching
Apache Arrow — `Decimal32`, `Decimal64`, `Decimal128`, `Decimal256` — each byte-based
(the mantissa's little-endian bytes followed by one scale byte), with value semantics
(equal **iff** the serialised bytes are equal), `f64` / integer conversion, rescaling, and
widening / narrowing between the widths.

!!! note "Mantissa marshalling"
    Python integers are arbitrary-precision, so every width's mantissa is a plain `int`.
    In Node the `Decimal32` mantissa is a `number`, while the wider `i64` / `i128` / `i256`
    mantissas marshal as `bigint` (a `number` cannot hold them). The `Decimal256` mantissa
    has no native FFI integer, so it bridges through the value's decimal string (Python) or
    its sign and 64-bit words (Node) — a `Decimal256` mantissa beyond 128 bits round-trips
    exactly either way.

## Construct, value, and scale

The constructor takes the mantissa and a `scale` (default `0`) and **range-checks** the
mantissa against the width: a value that does not fit raises a guided error naming the
accepted range, rather than silently truncating.

=== "Python"

    ```python
    from yggdryl.decimal import Decimal64

    d = Decimal64(12345, 2)       # 123.45
    assert d.mantissa == 12345
    assert d.scale == 2
    assert d.bits == 64
    assert abs(d.to_f64() - 123.45) < 1e-9
    assert d.to_i128() == 123      # integer part, truncated toward zero

    Decimal64(7)                   # scale defaults to 0
    ```

=== "Node"

    ```js
    const { Decimal64 } = require('yggdryl').decimal

    const d = new Decimal64(12345n, 2)   // 123.45 (bigint mantissa)
    console.assert(d.mantissa === 12345n)
    console.assert(d.scale === 2)
    console.assert(d.bits === 64)
    console.assert(Math.abs(d.toF64() - 123.45) < 1e-9)
    console.assert(d.toI128() === 123n)  // integer part (a bigint)

    new Decimal64(7n)                     // scale defaults to 0
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{Decimal, Decimal64};

    let d = Decimal64::new(12_345, 2); // 123.45
    assert_eq!(d.mantissa(), 12_345);
    assert_eq!(d.scale(), 2);
    assert!((d.to_f64() - 123.45).abs() < 1e-9);
    assert_eq!(d.to_i128(), Some(123));
    ```

## From a float, and rescaling

`from_f64` approximates a float at a chosen scale; `rescale` re-expresses a value at a new
scale, raising a guided error if the rescaled mantissa no longer fits the width.

=== "Python"

    ```python
    from yggdryl.decimal import Decimal32, Decimal64

    assert Decimal32.from_f64(1.5, 1) == Decimal32(15, 1)

    d = Decimal64(123, 0)
    assert d.rescale(2) == Decimal64(12300, 2)   # 123.00

    try:
        Decimal32(2_000_000_000, 0).rescale(2)   # overflows i32
    except ValueError as error:
        assert "wider decimal" in str(error)
    ```

=== "Node"

    ```js
    const { Decimal32, Decimal64 } = require('yggdryl').decimal

    console.assert(Decimal32.fromF64(1.5, 1).equals(new Decimal32(15, 1)))

    const d = new Decimal64(123n, 0)
    console.assert(d.rescale(2).equals(new Decimal64(12300n, 2)))

    try {
      new Decimal32(2000000000, 0).rescale(2)    // overflows i32
    } catch (error) {
      console.assert(/wider decimal/.test(error.message))
    }
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{Decimal32, Decimal64, DecimalError};

    assert_eq!(Decimal32::from_f64(1.5, 1), Decimal32::new(15, 1));
    assert_eq!(Decimal64::new(123, 0).rescale(2).unwrap(), Decimal64::new(12_300, 2));
    assert!(matches!(
        Decimal32::new(2_000_000_000, 0).rescale(2),
        Err(DecimalError::Overflow { bits: 32 })
    ));
    ```

## Byte round-trip and value semantics

A decimal serialises to its mantissa's little-endian bytes followed by the scale byte
(`5` / `9` / `17` / `33` bytes). Two decimals are equal **iff** those bytes are equal, so
the same numeric value at a different scale is **not** equal (`1.0` ≠ `1`).

=== "Python"

    ```python
    import pickle
    from yggdryl.decimal import Decimal64

    d = Decimal64(-4200, 2)                        # -42.00
    assert len(d.serialize_bytes()) == 9
    assert Decimal64.deserialize_bytes(d.serialize_bytes()) == d

    assert Decimal64(10, 1) != Decimal64(1, 0)     # equal value, different scale
    assert pickle.loads(pickle.dumps(d)) == d      # value semantics + pickle
    ```

=== "Node"

    ```js
    const { Decimal64 } = require('yggdryl').decimal

    const d = new Decimal64(-4200n, 2)             // -42.00
    console.assert(d.serializeBytes().length === 9)
    console.assert(Decimal64.deserializeBytes(d.serializeBytes()).equals(d))

    console.assert(!new Decimal64(10n, 1).equals(new Decimal64(1n, 0)))
    console.assert(new Decimal64(12345n, 2).hashCode() === new Decimal64(12345n, 2).hashCode())
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Decimal64;

    let d = Decimal64::new(-4200, 2);
    assert_eq!(d.serialize_bytes().len(), 9);
    assert_eq!(Decimal64::deserialize_bytes(&d.serialize_bytes()).unwrap(), d);
    assert_ne!(Decimal64::new(10, 1), Decimal64::new(1, 0));
    ```

## Widening and narrowing

The three narrow widths widen to `Decimal256` (always exact); `Decimal256` narrows back to
`Decimal128` when the mantissa fits, raising a guided error otherwise. A `Decimal256`
mantissa beyond 128 bits is held and byte-round-tripped exactly.

=== "Python"

    ```python
    from yggdryl.decimal import Decimal32, Decimal128, Decimal256

    assert Decimal32(12345, 2).to_decimal256() == Decimal256(12345, 2)
    assert Decimal256(999, 1).try_to_decimal128() == Decimal128(999, 1)

    big = Decimal256(2**200 + 123, 3)              # far beyond i128
    assert big.mantissa == 2**200 + 123            # exact
    assert big.to_i128() is None
    assert Decimal256.deserialize_bytes(big.serialize_bytes()) == big
    ```

=== "Node"

    ```js
    const { Decimal32, Decimal128, Decimal256 } = require('yggdryl').decimal

    console.assert(new Decimal32(12345, 2).toDecimal256().equals(new Decimal256(12345n, 2)))
    console.assert(new Decimal256(999n, 1).tryToDecimal128().equals(new Decimal128(999n, 1)))

    const big = new Decimal256(2n ** 200n + 123n, 3)  // far beyond i128
    console.assert(big.mantissa === 2n ** 200n + 123n) // exact
    console.assert(big.toI128() === null)
    console.assert(Decimal256.deserializeBytes(big.serializeBytes()).equals(big))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{Decimal, Decimal128, Decimal256, Decimal32, i256};

    assert_eq!(Decimal32::new(12_345, 2).to_decimal256(), Decimal256::new(i256::from_i128(12_345), 2));
    assert_eq!(Decimal256::new(i256::from_i128(999), 1).try_to_decimal128().unwrap(), Decimal128::new(999, 1));
    ```
