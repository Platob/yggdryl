//! The **variable-length binary** element type [`Binary`] — arbitrary byte sequences (`Vec<u8>`).

use crate::datatype_id::DataTypeId;
use crate::typed::VarType;

/// Variable-length **binary** — each element is an arbitrary byte sequence, owned as a `Vec<u8>`.
/// Used with the [`VarSerie`](crate::typed::VarSerie) (offsets + data) layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Binary;

impl VarType for Binary {
    type Owned = Vec<u8>;
    const DATA_TYPE_ID: DataTypeId = DataTypeId::Binary;

    fn to_owned(bytes: &[u8]) -> Option<Vec<u8>> {
        Some(bytes.to_vec())
    }

    fn owned_bytes(value: &Vec<u8>) -> &[u8] {
        value
    }
}
