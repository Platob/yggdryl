//! The **boolean** element type [`Bit`] — one logical bit per element, packed LSB-first through the
//! source's bit primitives.

use crate::datatype_id::DataTypeId;
use crate::io::memory::{IOBase, IoError};
use crate::typed::{DataType, Decoder, Encoder};

/// The boolean type (`bool`) — a single **bit** per element (element index *is* the bit index).
/// Encodes/decodes through the source's LSB-first `pwrite_bit` / `pread_bit`; not
/// [`Reduce`](crate::typed::Reduce) (a boolean does not sum).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Bit;

impl DataType for Bit {
    type Native = bool;
    const DATA_TYPE_ID: DataTypeId = DataTypeId::Bool;
}

impl Encoder for Bit {
    fn encode<W: IOBase>(dst: &mut W, index: u64, value: bool) -> Result<(), IoError> {
        dst.pwrite_bit(index, value)
    }
    fn encode_slice<W: IOBase>(dst: &mut W, start: u64, values: &[bool]) -> Result<(), IoError> {
        // DESIGN: bit-by-bit for now; the vectorized path packs 8 bits/byte into a staged buffer
        // and does one `pwrite_byte_array` — wired once the bit-pack kernel lands.
        for (offset, &value) in values.iter().enumerate() {
            dst.pwrite_bit(start + offset as u64, value)?;
        }
        Ok(())
    }
}

impl Decoder for Bit {
    fn decode<R: IOBase>(src: &R, index: u64) -> Result<bool, IoError> {
        src.pread_bit(index)
    }
    fn decode_slice<R: IOBase>(src: &R, start: u64, out: &mut [bool]) -> Result<(), IoError> {
        for (offset, slot) in out.iter_mut().enumerate() {
            *slot = src.pread_bit(start + offset as u64)?;
        }
        Ok(())
    }
}
