//! The **variable-length UTF-8** string element type [`Utf8`] — owned as a `String`.

use crate::datatype_id::DataTypeId;
use crate::typed::VarType;

/// Variable-length **UTF-8 string** — each element is a valid UTF-8 byte sequence, owned as a
/// `String`. Used with the [`VarSerie`](crate::typed::VarSerie) (offsets + data) layout; decoding a
/// non-UTF-8 slice yields `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Utf8;

impl VarType for Utf8 {
    type Owned = String;
    const DATA_TYPE_ID: DataTypeId = DataTypeId::Utf8;

    fn to_owned(bytes: &[u8]) -> Option<String> {
        core::str::from_utf8(bytes).ok().map(str::to_string)
    }

    fn owned_bytes(value: &String) -> &[u8] {
        value.as_bytes()
    }
}
