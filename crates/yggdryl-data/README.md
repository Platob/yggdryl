# yggdryl-data

The Apache Arrow-centralized **data-model layer** for yggdryl, built on
`yggdryl-core`. It defines the physical type system — data types, fields and scalars —
designed for zero-copy FFI and Arrow interop.

The type system is three layers of traits (one file per trait at the crate root),
plus concrete types grouped into per-family modules. The [`integer`](src/integer)
module is the first: every signed and unsigned integer, one module per type, one file
per concern (`data_type`, `field`, `scalar`).

## Untyped base

FFI-facing descriptors, all `Debug + Send + Sync` (schemas are printed and shared
across threads / FFI); no lifetime parameters. Every layer converts to and from its
Apache Arrow equivalent via `to_arrow` / `from_arrow` (the `arrow-schema` and
`arrow-array` subset crates are re-exported so downstream code shares the exact
versions).

- **`RawDataType`** — a physical type descriptor: `name`, the Arrow C Data Interface
  `arrow_format` string, and fixed `byte_width` / `bit_width` (or `None`); mirrors an
  `arrow_schema::DataType`. Object-safe, so a heterogeneous schema can hold
  `Box<dyn RawDataType>` (`from_arrow`, returning `Self`, is `Self: Sized`).
- **`RawField<D: RawDataType>`** — a named, nullable column (`name`, `data_type`,
  `is_nullable`); mirrors an `arrow_schema::Field` (`to_arrow` is defaulted from the
  three accessors). The model carries exactly those three properties: `from_arrow`
  refuses an extension-typed field (`ARROW:extension:name` metadata is a different
  logical type) and deliberately drops any other Arrow metadata, logging a `warn`
  when the `log` cargo feature is on.
- **`RawScalar<D: RawDataType>`** — a single, possibly-null value (`data_type`,
  `is_null`, `value` of an associated `Value: ?Sized`); mirrors Arrow's own scalar
  representation, a one-element `arrow_array::ArrayRef`.

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

## Type ids

`DataTypeId` is a `Copy` classifier with one variant per Arrow type (independent of
parameters). `DataTypeId::ALL` enumerates every id; each has a `name`, its parameterless
Arrow `arrow_format` (or `None`), and `is_primitive` / `is_nested` predicates.

## The integer module

Every signed and unsigned integer, from `Int8` / `UInt8` to `Int64` / `UInt64`. Each
type is a fixed-width `Primitive` with a little-endian byte codec, a nullable field and
a possibly-null scalar; the three share one shape, so a crate-internal macro generates
each per-type file.

```rust
use yggdryl_data::{
    arrow_schema, DataType, Int64, Int64Field, Int64Scalar, RawDataType, RawField, RawScalar,
};

// Int64 is a fixed-width primitive whose native type is i64.
assert_eq!((Int64.name(), Int64.arrow_format(), Int64.byte_width()), ("int64", "l".to_string(), Some(8)));
assert_eq!(Int64::ID, yggdryl_data::DataTypeId::Int64);
assert_eq!(Int64.native_to_bytes(&-1), vec![0xFF; 8]);
assert_eq!(Int64.native_from_bytes(&[0xFF; 8]).unwrap(), -1);

// It mirrors the arrow-schema type, both ways.
assert_eq!(Int64.to_arrow(), arrow_schema::DataType::Int64);
assert!(Int64::from_arrow(&arrow_schema::DataType::Utf8).is_err());

// Int64Field is a named, nullable column of int64; to_arrow / from_arrow mirror
// an arrow_schema::Field.
let id = Int64Field::new("id", false);
assert_eq!((id.name(), id.is_nullable()), ("id", false));
assert_eq!(Int64Field::from_arrow(&id.to_arrow()).unwrap(), id);

// Int64Scalar is a single i64 value, or null — built from a native value, and
// mirrored as Arrow's own scalar representation: a one-element array.
let scalar = Int64Scalar::from(42);
assert_eq!(scalar.value(), Some(&42));
assert_eq!(Int64Scalar::from(None), Int64Scalar::null());
assert_eq!(Int64Scalar::from_arrow(scalar.to_arrow().as_ref()).unwrap(), scalar);
```

The other widths follow the same surface — swap `Int64` / `i64` / `"l"` for
`Int8` / `i8` / `"c"`, `UInt32` / `u32` / `"I"`, and so on.
