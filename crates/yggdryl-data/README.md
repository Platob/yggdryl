# yggdryl-data

The Apache Arrow-centralized **data-model layer** for yggdryl, built on
`yggdryl-core`. It defines the physical type system — data types, fields and scalars —
designed for zero-copy FFI and Arrow interop.

The type system is three layers of traits, plus concrete types (one file per type
under `src/datatype/`). `Int64` and `Int64Scalar` are the first concrete case.

## Untyped base

FFI-facing descriptors, all `Debug + Send + Sync` (schemas are printed and shared
across threads / FFI); no lifetime parameters.

- **`RawDataType`** — a physical type descriptor: `name`, the Arrow C Data Interface
  `arrow_format` string, and fixed `byte_width` / `bit_width` (or `None`). Object-safe,
  so a heterogeneous schema can hold `Box<dyn RawDataType>`.
- **`RawField<D: RawDataType>`** — a named, nullable column (`name`, `data_type`,
  `is_nullable`).
- **`RawScalar<D: RawDataType>`** — a single, possibly-null value (`data_type`,
  `is_null`, `value` of an associated `Value: ?Sized`).

## Typed

The same, tied to a native Rust type `T`.

- **`DataType<T>: RawDataType`** — adds the codec bridging a native `T` to and from its
  Arrow bytes: `native_to_bytes` / `native_from_bytes`.
- **`Field<T>: RawField<Self::Type>`** — a field whose data type is a `DataType<T>`.
- **`Scalar<T>: RawScalar<Self::Type, Value = T>`** — a scalar whose value is `T`.

## Categories

How a type is shaped (each refines `RawDataType`).

- **`Primitive`** — a fixed-width, childless physical type (integers, floats, boolean).
- **`Logical`** — a type layered over a physical `Storage` type (e.g. a timestamp over
  `int64`).
- **`Nested`** — a type composed of child fields (`struct`, `list`, `map`).

## First concrete case

```rust
use yggdryl_data::{DataType, Int64, Int64Scalar, Primitive, RawDataType, RawScalar};

// Int64 is a fixed-width primitive whose native type is i64.
assert_eq!((Int64.name(), Int64.arrow_format(), Int64.byte_width()), ("int64", "l".to_string(), Some(8)));
assert_eq!(Int64.native_to_bytes(&-1), vec![0xFF; 8]);
assert_eq!(Int64.native_from_bytes(&[0xFF; 8]).unwrap(), -1);

// Int64Scalar is a single i64 value, or null.
let scalar = Int64Scalar::new(42);
assert_eq!(scalar.value(), Some(&42));
assert!(Int64Scalar::null().is_null());
```
