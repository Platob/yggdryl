//! The **integer** element types — `i8`…`u128` — each a byte-granular fixed-width type. Every one
//! is a single [`fixed_numeric!`](super::fixed_numeric) line except `u8`, which routes through the
//! source's byte primitives (there is no typed `u8` array — `u8` *is* the byte type).

use crate::datatype_id::DataTypeId;
use crate::io::memory::{Aggregate, IOBase, IoError};
use crate::typed::{DataType, Decoder, Encoder, Reduce};

fixed_numeric!(
    /// The signed 8-bit integer type (`i8`).
    Int8, i8, I8, i64,
    pwrite_i8_array, pread_i8_array, sum_i8, min_i8, max_i8, mean_i8
);
fixed_numeric!(
    /// The signed 16-bit integer type (`i16`).
    Int16, i16, I16, i64,
    pwrite_i16_array, pread_i16_array, sum_i16, min_i16, max_i16, mean_i16
);
fixed_numeric!(
    /// The unsigned 16-bit integer type (`u16`).
    UInt16, u16, U16, i64,
    pwrite_u16_array, pread_u16_array, sum_u16, min_u16, max_u16, mean_u16
);
fixed_numeric!(
    /// The signed 32-bit integer type (`i32`).
    Int32, i32, I32, i64,
    pwrite_i32_array, pread_i32_array, sum_i32, min_i32, max_i32, mean_i32
);
fixed_numeric!(
    /// The unsigned 32-bit integer type (`u32`).
    UInt32, u32, U32, i64,
    pwrite_u32_array, pread_u32_array, sum_u32, min_u32, max_u32, mean_u32
);
fixed_numeric!(
    /// The signed 64-bit integer type (`i64`).
    Int64, i64, I64, i128,
    pwrite_i64_array, pread_i64_array, sum_i64, min_i64, max_i64, mean_i64
);
fixed_numeric!(
    /// The unsigned 64-bit integer type (`u64`).
    UInt64, u64, U64, i128,
    pwrite_u64_array, pread_u64_array, sum_u64, min_u64, max_u64, mean_u64
);
fixed_numeric!(
    /// The signed 128-bit integer type (`i128`).
    Int128, i128, I128, i128,
    pwrite_i128_array, pread_i128_array, sum_i128, min_i128, max_i128, mean_i128
);
fixed_numeric!(
    /// The unsigned 128-bit integer type (`u128`).
    UInt128, u128, U128, u128,
    pwrite_u128_array, pread_u128_array, sum_u128, min_u128, max_u128, mean_u128
);

/// The unsigned 8-bit integer type (`u8`) — the **byte** type, so it uses the source's byte
/// primitives directly (`pwrite_byte_array` / `pread_exact`) rather than a typed `u8` array.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct UInt8;

impl DataType for UInt8 {
    type Native = u8;
    const DATA_TYPE_ID: DataTypeId = DataTypeId::U8;
}

impl Encoder for UInt8 {
    fn encode<W: IOBase>(dst: &mut W, index: u64, value: u8) -> Result<(), IoError> {
        Self::encode_slice(dst, index, &[value])
    }
    fn encode_slice<W: IOBase>(dst: &mut W, start: u64, values: &[u8]) -> Result<(), IoError> {
        dst.pwrite_byte_array(start, values);
        Ok(())
    }
}

impl Decoder for UInt8 {
    fn decode<R: IOBase>(src: &R, index: u64) -> Result<u8, IoError> {
        let mut out = [0u8; 1];
        Self::decode_slice(src, index, &mut out)?;
        Ok(out[0])
    }
    fn decode_slice<R: IOBase>(src: &R, start: u64, out: &mut [u8]) -> Result<(), IoError> {
        src.pread_exact(start, out)
    }
}

impl Reduce for UInt8 {
    type Sum = i64;
    fn sum<R: IOBase>(src: &R, start: u64, count: usize) -> Result<i64, IoError> {
        Aggregate::sum_u8(src, start, count)
    }
    fn min<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<u8>, IoError> {
        Aggregate::min_u8(src, start, count)
    }
    fn max<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<u8>, IoError> {
        Aggregate::max_u8(src, start, count)
    }
    fn mean<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<f64>, IoError> {
        Aggregate::mean_u8(src, start, count)
    }
}
