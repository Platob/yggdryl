# yggdryl-scalar

The Apache Arrow-centralized **scalar layer** for yggdryl, built on
`yggdryl-dtype` and `yggdryl-core`. It defines the scalars of the model — single,
possibly-null values of a data type — designed for zero-copy FFI and Arrow
interop. It is the third of the three data layers (`yggdryl-dtype`,
`yggdryl-field`, `yggdryl-scalar`), each concern its own crate, so the concrete
types share one naming convention across the layers
(`yggdryl_scalar::Int64Scalar` holds one value of the `yggdryl_dtype::Int64Type`
type, whose column is a `yggdryl_field::Int64Field`).

The layer is four traits (one file per trait at the crate root), plus concrete
scalars grouped into per-family modules mirroring `yggdryl-dtype` (one file per
type): [`integer`](src/integer) holds every signed and unsigned integer,
[`binary`](src/binary.rs) the byte value (doubling as a `yggdryl-core`
positioned-IO resource), [`null`](src/null.rs) the always-null scalar,
[`optional`](src/optional.rs) the null-or-value variant, and
[`serie`](src/serie), [`map`](src/map.rs) and [`struct`](src/struct.rs) the nested
values (the union, dynamic at runtime, has no scalar).

## Untyped base

- **`Scalar`** — a single, possibly-null value, carrying its data type as the
  associated `DataType` (`data_type`, `is_null`, `value` of an associated
  `Value: ?Sized`); mirrors Arrow's own scalar
  representation, a one-element `arrow_array::ArrayRef`, via `to_arrow` /
  `from_arrow`. The `as_*` accessors
  (`as_i8` … `as_u64`, `as_f32` / `as_f64`, `as_bool`, `as_str`, `as_bytes`) read
  the value as a chosen Rust type: direct for the scalar's own type (`as_str` /
  `as_bytes` borrow, never copy), exact conversion otherwise, and an actionable
  `DataError` when null (`NullValue`), inexact (`InexactConversion`) or simply not
  convertible (`UnsupportedConversion`, the default) — a scalar overrides only the
  targets its value converts to.

## Typed

- **`TypedScalar<DT: DataType, T>: Scalar<DataType = DT, Value = T>`** — a scalar whose value
  is `T` (possibly unsized: a string scalar exposes `Option<&str>`).
- **`FromScalar`** — the native Rust targets readable out of any scalar, behind
  the generic accessors such as `Serie::get_at::<T>` (numbers, `bool`, `String`,
  `Vec<u8>`, a core `ByteBufferSlice`).
- **`ScalarFactory<T>: TypedDataType<T>`** — a typed data type builds its scalar.
  The scalar layer builds on the data types, never the other way around, so the
  "data type → scalar" factory lives here (implemented for every `yggdryl-dtype`
  type next to its scalar): `scalar(value)` holds a native value, `null_scalar()`
  the null scalar, and `default_scalar()` the type's default — a scalar holding
  `default_value`, except where the scalar models nullness (an optional defaults
  to its null variant).

## The concrete scalars

```rust
use yggdryl_scalar::{Int64Scalar, Scalar, TypedOptionalScalar};
use yggdryl_scalar::yggdryl_dtype as dtype;

// A single i64 value, or null — built from a native value, and mirrored as
// Arrow's own scalar representation: a one-element array.
let scalar = Int64Scalar::from(42);
assert_eq!(scalar.value(), Some(&42));
assert_eq!(scalar.as_i8().unwrap(), 42); // converted, exact-or-error
assert_eq!(Int64Scalar::from_arrow(scalar.to_arrow_scalar().as_ref()).unwrap(), scalar);

// The optional scalar: a value variant or the null variant, over union storage,
// with access redirected to the inner scalar.
let answer = TypedOptionalScalar::new(Int64Scalar::new(42));
assert_eq!(answer.as_i64().unwrap(), 42);
let missing: TypedOptionalScalar<dtype::Int64Type, Int64Scalar> = TypedOptionalScalar::null();
assert!(missing.is_null());
```

Every nested value holds **our own series** — the type-erased `AnySerie` column
(integers decomposed to raw buffers, anything else zero-copy Arrow); a list holds
its item serie, a map its entries serie, a struct an array of column series —
reconstituting Arrow arrays on demand and decomposing them on the way in. The
`NestedSerie` trait adds child access (`child_serie_at(index)` /
`child_serie_by(name)`), `RecordScalar` is the generic struct-row accessor
(`scalar_at` / `scalar_by`), and the base `Scalar` gains `as_serie` / `as_map` /
`as_struct` alongside the `as_*` contract.

The serie scalar is *our array*: `TypedSerie<D, S>` is backed by one zero-copy Arrow
child array — `to_arrow_scalar` / `from_arrow` are reference-count bumps — with the
scalar accessors `get_scalar_at(index)` / `get_at::<T>(index)` and `len` /
`is_empty`. Every integer type also has its concrete serie (`Int8Serie` …
`UInt64Serie`), borrowing the raw Arrow buffers themselves (`values()` borrows
the native element slice without copying; `from_io` / `pwrite_io` bridge to any
`yggdryl-core` positioned-IO resource in one bulk little-endian transfer). The
dynamic `Serie` / `MapScalar` / `OptionalScalar` bases erase the element type; the
typed generics `erase()` to them. `TypedMapScalar<K, V,
SK, SV>` holds a key–value entry sequence and `StructScalar` one row of
one-element Arrow columns; the `binary` scalar holds its bytes as a core
`ByteBuffer` (`io()` / `into_io()` plug into `RawIOBase` and the cursor / slice
adapters).
