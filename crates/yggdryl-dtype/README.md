# yggdryl-dtype

The Apache Arrow-centralized **data-type layer** for yggdryl, built on
`yggdryl-core`. It defines the physical and logical data types of the model,
designed for zero-copy FFI and Arrow interop — the first of the three data layers
(`yggdryl-dtype`, `yggdryl-field`, `yggdryl-scalar`), each concern its own crate,
so the concrete types share one bare name across the layers (`yggdryl_dtype::Int64`
describes the type, `yggdryl_field::Int64` names a column of it,
`yggdryl_scalar::Int64` holds one value of it).

The type system is two layers of traits plus categories (one file per trait at the
crate root), and concrete types grouped into per-family modules (one file per
type). The [`integer`](src/integer) module holds every signed and unsigned
integer; [`binary`](src/binary.rs) the variable-size byte type; [`null`](src/null.rs)
the storage-free null type; [`union`](src/union), [`optional`](src/optional),
[`list`](src/list), [`map`](src/map) and [`struct`](src/struct) the composite
families, each carrying its own raw/typed trait pair.

## Untyped base

FFI-facing descriptors, all `Debug + Send + Sync` (schemas are printed and shared
across threads / FFI); no lifetime parameters.

- **`RawDataType`** — a physical type descriptor: `name`, the Arrow C Data Interface
  `arrow_format` string, and fixed `byte_width` / `bit_width` (or `None`); mirrors an
  `arrow_schema::DataType` via `to_arrow` / `from_arrow` (the `arrow-schema` subset
  crate is re-exported so downstream code shares the exact version). Object-safe, so
  a heterogeneous schema can hold `Box<dyn RawDataType>` (`from_arrow`, returning
  `Self`, is `Self: Sized`).

## Typed

- **`DataType<T>: RawDataType`** — adds the codec bridging a native `T` to and from
  its Arrow bytes (`native_to_bytes` / `native_from_bytes`; a length mismatch on
  decode returns `DataError::InvalidByteLength`) and `default_value` (`0` for
  integers, empty for sequences, the *first* data type's default for a union). The
  default *scalar* of a type lives upstream, on `yggdryl-scalar`'s `DefaultScalar`
  trait.

## Categories

How a type is shaped (each refines `RawDataType`).

- **`Primitive`** — a fixed-width, childless physical type (integers, floats, boolean).
- **`RawLogical<S>` / `Logical<T>`** — a type layered over a physical storage type
  (e.g. a timestamp over `int64`); `Optional<D>` is the generic holder.
- **`RawNested` / `Nested<T>`** — a type composed of child fields (`struct`,
  `list`, `map`, `union`); `List<D>` and `Map<K, V>` are the generic typed
  holders, the dynamic `Struct` / `Union` raw-only.

Each composite family also carries its own raw/typed trait pair (`RawOptional` /
`TypedOptional`, `RawUnion` / `TypedUnion`, `RawList` / `TypedList`, `RawMap` /
`TypedMap`, `RawStruct` / `TypedStruct`); the concrete `Optional<D>`, `Union`,
`List<D>`, `Map<K, V>` and `Struct` implement the raw side, and the typed side
wherever the child types have codecs (the dynamic `Union` and `Struct`, whose
children are only known at runtime, stay raw-only).

## Type ids

`DataTypeId` is a `Copy` classifier with one variant per Arrow type (independent of
parameters). `DataTypeId::ALL` enumerates every id; each has a `name`, its parameterless
Arrow `arrow_format` (or `None`), and `is_primitive` / `is_nested` predicates.

## The integer module

Every signed and unsigned integer, from `Int8` / `UInt8` to `Int64` / `UInt64`. Each
type is a fixed-width `Primitive` with a little-endian byte codec; the eight share
one shape, so a crate-internal macro generates each per-type file.

```rust
use yggdryl_dtype::{arrow_schema, DataType, Int64, RawDataType};

// Int64 is a fixed-width primitive whose native type is i64.
assert_eq!((Int64.name(), Int64.arrow_format(), Int64.byte_width()), ("int64", "l".to_string(), Some(8)));
assert_eq!(Int64::ID, yggdryl_dtype::DataTypeId::Int64);
assert_eq!(Int64.native_to_bytes(&-1), vec![0xFF; 8]);
assert_eq!(Int64.native_from_bytes(&[0xFF; 8]).unwrap(), -1);

// It mirrors the arrow-schema type, both ways.
assert_eq!(Int64.to_arrow(), arrow_schema::DataType::Int64);
assert!(Int64::from_arrow(&arrow_schema::DataType::Utf8).is_err());
```

The other widths follow the same surface — swap `Int64` / `i64` / `"l"` for
`Int8` / `i8` / `"c"`, `UInt32` / `u32` / `"I"`, and so on.

## The composite modules: null, union, optional, list, map, struct

`Null` is the storage-free type whose every value is null. `Union` is Apache
Arrow's union — a value is exactly one of several child types, discriminated by a
type id — carrying its `UnionFields` and `UnionMode` losslessly, so `to_arrow` /
`from_arrow` round-trip any union (`Union::optional(&T)` names the sparse
two-variant union between null and a value type).

`Optional<D>` is the first concrete `Logical` type: a value of the value type `D`,
or null, physically stored as `Union::optional(&D)` (`storage()` returns the
union). Its Arrow surface delegates to the storage; its `DataType<T>` byte codec
delegates to the value type.

```rust
use yggdryl_dtype::{DataType, Int64, Optional, RawDataType, RawLogical};

let optional = Optional::new(Int64);
assert_eq!((optional.name(), optional.storage().name()), ("optional", "union"));
assert_eq!(optional.arrow_format(), "+us:0,1");
assert_eq!(optional.native_from_bytes(&42i64.to_le_bytes()).unwrap(), 42); // the value type's codec
```

`List<D>` and `Map<K, V>` are the generic nested holders — their typed byte codecs
concatenate the child codecs and split them back by fixed width (a variable-width
child errors with `DataError::IndeterminateElementWidth` — decode those from
Arrow) — and the dynamic `Struct` carries its Arrow `Fields` losslessly.
