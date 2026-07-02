# Data model

!!! note "Rust core only — scaffold"
    The `yggdryl-data` crate is the Apache Arrow-centralized **data-model layer**,
    built on `yggdryl-core`. It currently holds only the abstract base traits; concrete
    types (`Int32`, `Utf8`, `Boolean`, …), their scalars, and the Arrow C Data
    Interface bridges land here as the layer grows. It gains Python and Node tabs when
    the bindings expose it.

Three base traits describe the physical type system, designed for zero-copy FFI and
Arrow interop. None carries a lifetime parameter (FFI-clean); parameterising by the
data type `D` keeps concrete types monomorphised for zero-cost access.

## `RawDataType`

A physical type descriptor: its `name`, its Arrow C Data Interface `arrow_format`
string, and the fixed `byte_width` / `bit_width` of one value (or `None` for a
variable-width type such as `utf8`, or a sub-byte type such as `boolean`).

```rust
use yggdryl_data::RawDataType;

struct Int32;

impl RawDataType for Int32 {
    fn name(&self) -> &str {
        "int32"
    }
    fn arrow_format(&self) -> String {
        "i".to_string() // Arrow C Data Interface format for int32
    }
    fn byte_width(&self) -> Option<usize> {
        Some(4)
    }
}

fn main() {
    assert_eq!(Int32.arrow_format(), "i");
    assert_eq!(Int32.byte_width(), Some(4));
    assert_eq!(Int32.bit_width(), Some(32)); // default: eight times the byte width
}
```

## `RawField<D: RawDataType>`

A named, nullable column of a data type `D` — `name`, `data_type`, `is_nullable` —
mirroring an Arrow `Field`. A schema is a sequence of fields.

## `RawScalar<D: RawDataType>`

A single, possibly-null value of a data type `D` — `data_type`, `is_null`, and the
native `value` (of an associated `Value` type) when non-null — mirroring an Arrow
`Scalar`.

```rust
use yggdryl_data::{RawDataType, RawScalar};

struct Int32;
impl RawDataType for Int32 {
    fn name(&self) -> &str { "int32" }
    fn arrow_format(&self) -> String { "i".to_string() }
    fn byte_width(&self) -> Option<usize> { Some(4) }
}

struct Int32Scalar {
    data_type: Int32,
    value: Option<i32>,
}

impl RawScalar<Int32> for Int32Scalar {
    type Value = i32;
    fn data_type(&self) -> &Int32 {
        &self.data_type
    }
    fn is_null(&self) -> bool {
        self.value.is_none()
    }
    fn value(&self) -> Option<&i32> {
        self.value.as_ref()
    }
}

fn main() {
    let answer = Int32Scalar { data_type: Int32, value: Some(42) };
    assert_eq!(answer.data_type().name(), "int32");
    assert_eq!(answer.value(), Some(&42));

    let missing = Int32Scalar { data_type: Int32, value: None };
    assert!(missing.is_null());
}
```
