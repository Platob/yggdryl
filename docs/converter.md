# Casting — the `Converter` and `cast`

Every fixed value type can be **cast** to another: a `Scalar<T>`, `Serie<T>`, or `Buffer<T>` casts
to type `U` with the compile-time-generic `cast::<U>()`, which delegates to the [`Converter`] trait.
A cast to the **same** type is a no-op that shares the backing buffer (no data copy). Across the
numeric family every pair is reachable directly; every value also bridges to and from **UTF-8** and
**binary** — the two universal formats — so anything reaches anything.

!!! note "Increment status"
    This first increment covers the numeric primitives (`u8`…`i128`, `f16`/`f32`/`f64`), null
    passthrough, and the UTF-8 / binary bridges. The wide integers, decimals, temporal, and
    fixed-size byte types are reached today **through** the UTF-8 / binary bridges; direct
    `Converter` impls for them, and the Python/Node mirrors of `cast`, are the next increment.

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
