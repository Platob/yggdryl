# yggdryl-data

The Apache Arrow-centralized **data-model layer** for yggdryl, built on
`yggdryl-core`. It defines the physical type system — data types, fields and scalars —
designed for zero-copy FFI and Arrow interop.

> **Scaffold.** This crate currently holds only the abstract base traits. Concrete
> types (`Int32`, `Utf8`, `Boolean`, …), their scalars, and the Arrow C Data Interface
> bridges land here as the layer grows — one file per type under `src/datatype/`.

## Base traits

- **`RawDataType`** — a physical type descriptor: its `name`, its Arrow C Data
  Interface `arrow_format` string, and its fixed `byte_width` / `bit_width` (or `None`
  for variable-width types).
- **`RawField<D: RawDataType>`** — a named, nullable column of a data type `D`
  (`name`, `data_type`, `is_nullable`), mirroring an Arrow `Field`.
- **`RawScalar<D: RawDataType>`** — a single, possibly-null value of a data type `D`
  (`data_type`, `is_null`, `value` of an associated `Value` type), mirroring an Arrow
  `Scalar`.

```rust
use yggdryl_data::RawDataType;

struct Int32;

impl RawDataType for Int32 {
    fn name(&self) -> &str {
        "int32"
    }
    fn arrow_format(&self) -> String {
        "i".to_string()
    }
    fn byte_width(&self) -> Option<usize> {
        Some(4)
    }
}

// bit_width defaults to eight times the byte width.
assert_eq!((Int32.arrow_format(), Int32.bit_width()), ("i".to_string(), Some(32)));
```

The traits carry no lifetime parameters (FFI-clean); the parameterisation by `D`
keeps concrete types monomorphised for zero-cost access.
