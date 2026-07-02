# Data model

!!! note "Rust core only"
    The `yggdryl-data` crate is the Apache Arrow-centralized **data-model layer**,
    built on `yggdryl-core`. It defines the physical type system — data types, fields
    and scalars — for zero-copy FFI and Arrow interop, and gains Python and Node tabs
    when the bindings expose it. `Int64` and `Int64Scalar` are the first concrete
    types; more land one file per type as the layer grows.

The type system is three layers of traits. None carries a lifetime parameter
(FFI-clean); the untyped base is `Debug + Send + Sync` so schemas are printable and
shareable across threads and FFI, and `RawDataType` is object-safe for
`Box<dyn RawDataType>` schemas.

## The first concrete type: `Int64`

`Int64` is a fixed-width [primitive](#categories) whose native Rust type is `i64`,
stored little-endian in eight bytes (Arrow C Data Interface format `"l"`):

```rust
use yggdryl_data::{DataType, Int64, Int64Scalar, Primitive, RawDataType, RawScalar};

fn main() {
    // A physical type descriptor.
    assert_eq!(Int64.name(), "int64");
    assert_eq!(Int64.arrow_format(), "l");
    assert_eq!((Int64.byte_width(), Int64.bit_width()), (Some(8), Some(64)));

    // DataType<i64>: the codec bridging i64 to and from Arrow bytes.
    assert_eq!(Int64.native_to_bytes(&-1), vec![0xFF; 8]);
    assert_eq!(Int64.native_from_bytes(&[0xFF; 8]).unwrap(), -1);

    // Int64Scalar: a single i64 value, or null.
    let scalar = Int64Scalar::new(42);
    assert_eq!(scalar.value(), Some(&42));
    assert!(Int64Scalar::null().is_null());
}
```

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
