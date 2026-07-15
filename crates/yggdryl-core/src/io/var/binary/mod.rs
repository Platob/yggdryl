//! `io::var::binary` — the variable-length **opaque binary** kind: the [`Binary`] marker (which
//! accepts any bytes) and the `Binary*` aliases + `&[u8]` ergonomics over the shared `Byte*`
//! generics.
//!
//! DESIGN: only the `i32`-offset [`Binary`] ships today. A `LargeBinary` (`i64` offsets) is
//! reserved at [`DataTypeId::LargeBinary`](crate::io::DataTypeId::LargeBinary) — see the note on
//! [`string`](crate::io::var::string) for the offset-axis follow-up.

use super::{ByteField, ByteScalar, ByteSerie, ByteType, VarElement};
use crate::io::{DataTypeId, IoError};

/// A variable-length **opaque binary** element (`i32` offsets) — any bytes are valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Binary;

impl VarElement for Binary {
    const NAME: &'static str = "binary";
    const TYPE_ID: DataTypeId = DataTypeId::Binary;

    fn validate(_bytes: &[u8]) -> Result<(), IoError> {
        Ok(())
    }
}

/// Binary scalar ergonomics.
impl ByteScalar<Binary> {
    /// A present scalar from raw bytes (any bytes are valid, so infallible).
    pub fn of(value: &[u8]) -> Self {
        Self::from_bytes_unchecked(value)
    }
}

/// The typed descriptor of the opaque-binary type — [`ByteType<Binary>`](ByteType).
pub type BinaryDataType = ByteType<Binary>;
/// A named, nullable binary column descriptor — [`ByteField<Binary>`](ByteField).
pub type BinaryField = ByteField<Binary>;
/// One nullable binary value — [`ByteScalar<Binary>`](ByteScalar).
pub type BinaryScalar = ByteScalar<Binary>;
/// A nullable column of binary values — [`ByteSerie<Binary>`](ByteSerie).
pub type BinarySerie = ByteSerie<Binary>;
