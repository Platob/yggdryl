//! The **large** variable-length element types [`LargeBinary`] and [`LargeUtf8`] — the same
//! offsets + data layout as [`Binary`](crate::typed::Binary) / [`Utf8`](crate::typed::Utf8) but with
//! **`i64` offsets** (Arrow's `Large*`), so a column whose packed data exceeds the `i32` offset
//! range still addresses every element.

use crate::datatype_id::DataTypeId;
use crate::typed::varbyte::VarLenType;
use crate::typed::VarType;

/// Large variable-length **binary** — arbitrary byte sequences (`Vec<u8>`) over an **`i64`-offsets +
/// data** layout. Identical to [`Binary`](crate::typed::Binary) apart from the wider offset element;
/// used with the [`VarSerie`](crate::typed::VarSerie) carrier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LargeBinary;

impl VarType for LargeBinary {
    type Owned = Vec<u8>;
    const DATA_TYPE_ID: DataTypeId = DataTypeId::LargeBinary;

    fn to_owned(bytes: &[u8]) -> Option<Vec<u8>> {
        Some(bytes.to_vec())
    }

    fn owned_bytes(value: &Vec<u8>) -> &[u8] {
        value
    }
}

impl VarLenType for LargeBinary {
    type Offset = i64;
}

/// Large variable-length **UTF-8 string** — valid UTF-8 byte sequences (`String`) over an
/// **`i64`-offsets** + data layout. Identical to [`Utf8`](crate::typed::Utf8) apart from the wider
/// offset element; decoding a non-UTF-8 slice yields `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LargeUtf8;

impl VarType for LargeUtf8 {
    type Owned = String;
    const DATA_TYPE_ID: DataTypeId = DataTypeId::LargeUtf8;

    fn to_owned(bytes: &[u8]) -> Option<String> {
        core::str::from_utf8(bytes).ok().map(str::to_string)
    }

    fn owned_bytes(value: &String) -> &[u8] {
        value.as_bytes()
    }
}

impl VarLenType for LargeUtf8 {
    type Offset = i64;
}
