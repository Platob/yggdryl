# Casting — the `Converter` and `cast`

Every fixed value type can be **cast** to another: a `Scalar<T>`, `Serie<T>`, or `Buffer<T>` casts
to type `U` with the compile-time-generic `cast::<U>()`, which delegates to the [`Converter`] trait.
A cast to the **same** type is a no-op that shares the backing buffer (no data copy). Across the
numeric family every pair is reachable directly; every value also bridges to and from **UTF-8** and
**binary** — the two universal formats — so anything reaches anything.

!!! note "Increment status"
    This increment covers the numeric primitives (`u8`…`i128`, `f16`/`f32`/`f64`), null
    passthrough, and the UTF-8 / binary bridges — all mirrored in **Python and Node** as
    per-target `to_<type>` methods (see [In Python and Node](#in-python-and-node)). The wide
    integers, decimals, temporal, and fixed-size byte types are reached today **through** the
    UTF-8 / binary bridges; direct `Converter` impls for them are the next increment.

## In Python and Node

Each numeric `Scalar` / `Serie` exposes a **`to_<type>` method per numeric target** (`to_i64`,
`to_f64`, …) delegating to the core `cast`, plus the universal `to_utf8` / `to_binary` bridges (and
their reverse on `Utf8Scalar` / `BinaryScalar`). Integer targets are range-checked (a guided
`ValueError` / thrown `Error`); a null casts to a null of the target.

=== "Python"

    ```python
    from yggdryl.types import I32Scalar, I32Serie, Utf8Scalar, BinaryScalar

    assert I32Scalar(300).to_i64().value == "300"        # widen (i64 crosses as a string)
    assert I32Scalar(300).to_f64().value == 300.0        # precision-lossy path
    assert I32Scalar().to_i64().is_null                  # null -> null

    col = I32Serie([1, None, 3]).to_i64()                # a whole column, nulls preserved
    assert col.to_options() == ["1", None, "3"]

    # Universal bridges — any <-> utf8 / binary.
    assert I32Scalar(-7).to_utf8().to_i32().value == -7
    assert BinaryScalar(I32Scalar(-7).to_binary().value).to_i32().value == -7
    ```

=== "Node"

    ```js
    const { I32Scalar, I32Serie, BinaryScalar } = require('yggdryl').types

    assert(new I32Scalar(300).toI64().value === '300')   // widen (i64 crosses as a string)
    assert(new I32Scalar(300).toF64().value === 300.0)   // precision-lossy path
    assert(new I32Scalar().toI64().isNull)               // null -> null

    const col = new I32Serie([1, null, 3]).toI64()       // a whole column, nulls preserved
    assert(JSON.stringify(col.toOptions()) === JSON.stringify(['1', null, '3']))

    // Universal bridges — any <-> utf8 / binary.
    assert(new I32Scalar(-7).toUtf8().toI32().value === -7)
    assert(new BinaryScalar(new I32Scalar(-7).toBinary().value).toI32().value === -7)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::{Scalar, Serie};
    use yggdryl_core::io::var::BinaryScalar;

    assert_eq!(Scalar::of(300i32).cast::<i64>().unwrap(), Scalar::of(300i64));
    assert!(Scalar::of(300i32).cast::<u8>().is_err());   // 300 > u8::MAX
    assert_eq!(Scalar::<i32>::null().cast::<i64>().unwrap(), Scalar::null());

    let col = Serie::from_options(&[Some(1i32), None, Some(3)]);
    assert_eq!(col.cast::<i64>().unwrap().to_options(), [Some(1i64), None, Some(3)]);

    let bytes = Scalar::of(-7i32).to_binary();
    assert_eq!(bytes.read_to::<i32>().unwrap(), Scalar::of(-7i32));
    ```

## Numeric casts — range-checked, same-type is free

```rust
use yggdryl_core::io::fixed::{Scalar, Serie};

// Integers are range-checked; the error names the offending value.
assert_eq!(Scalar::of(300i32).cast::<i64>().unwrap(), Scalar::of(300i64));
assert!(Scalar::of(300i32).cast::<u8>().is_err());          // 300 > u8::MAX
assert_eq!(Scalar::of(3.9f64).cast::<i32>().unwrap(), Scalar::of(3i32)); // truncates

// A whole column converts element-for-element, nulls preserved.
let col = Serie::from_options(&[Some(1i32), None, Some(3)]);
assert_eq!(col.cast::<i64>().unwrap().to_options(), [Some(1i64), None, Some(3)]);

// Casting to the SAME type shares the Arc-backed buffer — no data copy.
let same = col.cast::<i32>().unwrap();
assert_eq!(same, col);
```

A non-finite float (`NaN` / `±∞`) cannot become an integer (`CastError::NotFinite`); an
out-of-range value is `CastError::OutOfRange`, naming the value and the target type.

## Float → integer: the exact range rule

The float → integer path first **rejects non-finite** inputs (`NaN` / `±∞` → `NotFinite`), then
**truncates toward zero** (`3.9 → 3`, `-3.9 → -3`), then **range-checks against an exact bound**.
The bound is deliberately *not* `T::MAX as f64`: for the wide types that conversion **rounds up**
(`i64::MAX` → 2⁶³, `u64::MAX` → 2⁶⁴, `i128::MAX` → 2¹²⁷), so a naive `value <= MAX as f64` test
would admit exactly that power and then saturate silently. yggdryl instead gates on the *first
magnitude the type cannot hold* — a signed N-bit type accepts `[MIN, 2^(N-1))`, an unsigned one
`[0, 2^N)`. Every bound is an exact power of two, representable in `f64`, so the check is exact and
the accepted value round-trips. `NumericSerie::cast` (the whole-column form) applies the same rule
element-for-element.

=== "Python"

    ```python
    from yggdryl.types import I32Scalar, F64Scalar

    assert F64Scalar(3.9).to_i32().value == 3        # truncates toward zero
    assert F64Scalar(-3.9).to_i32().value == -3
    # 2**31 is the first magnitude i32 cannot hold -> out of range (not a silent saturate).
    try:
        F64Scalar(2.0 ** 31).to_i32()
    except (ValueError, OverflowError) as error:
        assert "range" in str(error).lower()
    ```

=== "Node"

    ```js
    const { F64Scalar } = require('yggdryl').types

    assert(new F64Scalar(3.9).toI32().value === 3)   // truncates toward zero
    assert(new F64Scalar(-3.9).toI32().value === -3)
    // 2**31 is the first magnitude i32 cannot hold -> guided range error.
    assert.throws(() => new F64Scalar(2 ** 31).toI32(), /range/i)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::Scalar;

    assert_eq!(Scalar::of(3.9f64).cast::<i32>().unwrap(), Scalar::of(3i32));   // truncates
    assert_eq!(Scalar::of(-3.9f64).cast::<i32>().unwrap(), Scalar::of(-3i32));
    assert!(Scalar::of(2f64.powi(31)).cast::<i32>().is_err());    // exact bound: 2^31 rejected
    assert!(Scalar::of(f64::NAN).cast::<i32>().is_err());         // NotFinite
    ```

## `NumericCast` — the coercion capability

The numeric conversions ride one **capability sub-trait**, `NumericCast`, implemented for every
numeric primitive (`u8`…`i128`, `f16`/`f32`/`f64`). It spells the exact `i128` (integer) and `f64`
(float) bridges the casts route through — and, since the same closed numeric space powers the
vectorized arithmetic, it *also* carries the element kernels (`add_wrapping`, `div_checked`, …) that
the [element-wise ops](ops.md) fast path dispatches to. So "castable" and "does arithmetic" are the
**same** capability: a column has the numeric ops exactly when its element is `NumericCast`, checked
once at compile time rather than per-op on the hot path.

## The universal UTF-8 and binary bridges

Any value formats to a UTF-8 string and parses back; any value serializes to its canonical
little-endian bytes and reads back. These two bridges make **anything reachable from anything**,
even without a direct numeric path.

```rust
use yggdryl_core::io::fixed::Scalar;
use yggdryl_core::io::var::{BinaryScalar, Utf8Scalar};

// any -> utf8 (Display) and utf8 -> any (parse).
assert_eq!(Scalar::of(42i32).to_utf8().as_str(), Some("42"));
assert_eq!(Utf8Scalar::of("42").parse_to::<i64>().unwrap(), Scalar::of(42i64));

// any -> binary (canonical LE bytes) and binary -> any.
let bytes = Scalar::of(0x0102_0304i32).to_binary();
assert_eq!(bytes.read_to::<i32>().unwrap(), Scalar::of(0x0102_0304i32));
```

## The `Converter` trait

The entry points delegate to [`Converter<To>`], which spells the cast contract at four
granularities — [`cast_value`], [`cast_scalar`], [`cast_serie`], [`cast_buffer`] — with the
scalar / serie / buffer forms mutualized over `cast_value` (a null stays a null). It can be driven
directly when the source and target are named:

```rust
use yggdryl_core::io::fixed::Serie;
use yggdryl_core::io::Converter;

let col = Serie::from_options(&[Some(1i32), None, Some(3)]);
let floats = <i32 as Converter<f64>>::cast_serie(&col).unwrap();
assert_eq!(floats.to_options(), [Some(1.0f64), None, Some(3.0)]);
```

## Design notes

- **Same-type casts share the buffer.** `Serie<T>::cast::<T>()` / `Buffer<T>::cast::<T>()` detect
  the identity via `TypeId` and clone the `Arc`-backed buffer — `0 allocs / op` (see the
  [benchmark report](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/converter.md)).
  A cross-type cast builds one new buffer.
- **Integers are exact; floats are lossy by contract.** Integer↔integer goes through an exact
  `i128` intermediate with a range check; anything touching a float goes through `f64`, so it is
  precision-lossy (documented), and a non-finite float is rejected rather than silently truncated.
- **The bridges are the universal fallback.** Because every value type already has a `Display`/
  `FromStr` string form and a little-endian byte codec, UTF-8 and binary are the two hubs through
  which any pair is reachable — the direct `Converter` impls are just the fast paths.
