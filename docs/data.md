# Data model

!!! note "Rust core only"
    The `yggdryl-data` crate is the Apache Arrow-centralized **data-model layer**,
    built on `yggdryl-core`. It defines the physical type system — data types, fields
    and scalars — for zero-copy FFI and Arrow interop, and gains Python and Node tabs
    when the bindings expose it. The `integer` module is the first concrete family —
    every signed and unsigned integer, each a data type, field and scalar; more
    families land as the layer grows.

The type system is three layers of traits. None carries a lifetime parameter
(FFI-clean); the untyped base is `Debug + Send + Sync` so schemas are printable and
shareable across threads and FFI, and `RawDataType` is object-safe for
`Box<dyn RawDataType>` schemas.

## The concrete types: the `integer` module

The `integer` module holds every Apache Arrow signed and unsigned integer — `Int8` …
`Int64`, `UInt8` … `UInt64` — one module per type, one file per concern (`data_type`,
`field`, `scalar`). Each is a fixed-width [primitive](#categories) with a little-endian
byte codec, a nullable field and a possibly-null scalar; they share one shape, so a
single crate-internal macro generates each per-type file.

`Int64`, native Rust `i64`, is stored little-endian in eight bytes (Arrow C Data
Interface format `"l"`):

```rust
use yggdryl_data::{
    DataType, DataTypeId, Int64, Int64Field, Int64Scalar, RawDataType, RawField, RawScalar,
};

fn main() {
    // A physical type descriptor; `ID` is the matching classifier.
    assert_eq!(Int64.name(), "int64");
    assert_eq!(Int64.arrow_format(), "l");
    assert_eq!((Int64.byte_width(), Int64.bit_width()), (Some(8), Some(64)));
    assert_eq!(Int64::ID, DataTypeId::Int64);

    // DataType<i64>: the codec bridging i64 to and from Arrow bytes.
    assert_eq!(Int64.native_to_bytes(&-1), vec![0xFF; 8]);
    assert_eq!(Int64.native_from_bytes(&[0xFF; 8]).unwrap(), -1);

    // Int64Field: a named, nullable column of int64.
    let id = Int64Field::new("id", false);
    assert_eq!((id.name(), id.is_nullable()), ("id", false));

    // Int64Scalar: a single i64 value, or null.
    let scalar = Int64Scalar::new(42);
    assert_eq!(scalar.value(), Some(&42));
    assert!(Int64Scalar::null().is_null());
}
```

The other widths follow the same surface — swap `Int64` / `i64` / `"l"` for
`Int8` / `i8` / `"c"`, `UInt32` / `u32` / `"I"`, and so on.

## The trait layers

### Untyped base

- **`RawDataType`** — a physical type descriptor: `name`, the Arrow C Data Interface
  `arrow_format` string, and fixed `byte_width` / `bit_width` (`None` for variable or
  nested types).
- **`RawField<D: RawDataType>`** — a named, nullable column: `name`, `data_type`,
  `is_nullable`.
- **`RawScalar<D: RawDataType>`** — a single, possibly-null value: `data_type`,
  `is_null`, `value` of an associated `Value: ?Sized`.

### Typed

The same, tied to a native Rust type `T`:

- **`DataType<T>: RawDataType`** — adds the byte codec `native_to_bytes` /
  `native_from_bytes` (a length mismatch on decode returns
  `DataError::InvalidByteLength`).
- **`Field<T>: RawField<Self::Type>`** — a field whose data type is a `DataType<T>`.
- **`Scalar<T>: RawScalar<Self::Type, Value = T>`** — a scalar whose value is `T`.

```rust
use yggdryl_data::{DataType, Int64, Primitive, RawDataType, RawScalar, Scalar};

// Generic code composes across the layers.
fn first_byte<D: DataType<i64>>(data_type: &D, value: i64) -> u8 {
    data_type.native_to_bytes(&value)[0]
}
fn is_null<S: Scalar<i64>>(scalar: &S) -> bool {
    scalar.is_null()
}
fn width<P: Primitive>(primitive: &P) -> Option<usize> {
    primitive.byte_width()
}

fn main() {
    assert_eq!(first_byte(&Int64, 5), 5);
    assert_eq!(width(&Int64), Some(8));
}
```

### Categories

How a type is shaped (each refines `RawDataType`):

- **`Primitive`** — a fixed-width, childless physical type (integers, floats, boolean).
- **`Logical`** — a type layered over a physical `Storage` type, e.g. a timestamp over
  `int64`; `storage()` returns the backing `RawDataType`.
- **`Nested`** — a type composed of child fields (`struct`, `list`, `map`);
  `child_count()` reports how many.

## Type ids

`DataTypeId` is a `Copy` tag with one variant per Arrow type — independent of any
parameters — used to switch on or group types cheaply. `DataTypeId::ALL` lists every
id; each carries its `name`, its Arrow C Data Interface `arrow_format` (static for
parameterless types, `None` for parameterized/logical ones), and the `is_primitive` /
`is_nested` classification.

```rust
use yggdryl_data::DataTypeId;

fn main() {
    assert_eq!(DataTypeId::Int64.name(), "int64");
    assert_eq!(DataTypeId::Int64.arrow_format(), Some("l"));
    assert!(DataTypeId::Int64.is_primitive());
    assert!(DataTypeId::Struct.is_nested());
    assert_eq!(DataTypeId::Decimal128.arrow_format(), None); // parameterized
    assert!(DataTypeId::ALL.contains(&DataTypeId::Utf8));
}
```
