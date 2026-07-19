//! The **integer** element types — `i8`…`u128` — each a byte-granular fixed-width type. Every one
//! is a single [`fixed_numeric!`](super::fixed_numeric) line except `u8`, which routes through the
//! source's byte primitives (there is no typed `u8` array — `u8` *is* the byte type).

use crate::datatype_id::DataTypeId;
use crate::io::memory::{Aggregate, IOBase, IoError};
use crate::typed::{DataType, Decoder, Encoder, Reduce};

fixed_numeric!(
    /// The signed 8-bit integer type (`i8`).
    Int8, i8, I8, i64,
    pwrite_i8_array, pread_i8_array, pwrite_i8_repeat, sum_i8, min_i8, max_i8, mean_i8,
    std_i8, var_i8, median_i8, first_i8, last_i8, count_ge_i8
);
fixed_numeric!(
    /// The signed 16-bit integer type (`i16`).
    Int16, i16, I16, i64,
    pwrite_i16_array, pread_i16_array, pwrite_i16_repeat, sum_i16, min_i16, max_i16, mean_i16,
    std_i16, var_i16, median_i16, first_i16, last_i16, count_ge_i16
);
fixed_numeric!(
    /// The unsigned 16-bit integer type (`u16`).
    UInt16, u16, U16, i64,
    pwrite_u16_array, pread_u16_array, pwrite_u16_repeat, sum_u16, min_u16, max_u16, mean_u16,
    std_u16, var_u16, median_u16, first_u16, last_u16, count_ge_u16
);
fixed_numeric!(
    /// The signed 32-bit integer type (`i32`).
    Int32, i32, I32, i64,
    pwrite_i32_array, pread_i32_array, pwrite_i32_repeat, sum_i32, min_i32, max_i32, mean_i32,
    std_i32, var_i32, median_i32, first_i32, last_i32, count_ge_i32
);
fixed_numeric!(
    /// The unsigned 32-bit integer type (`u32`).
    UInt32, u32, U32, i64,
    pwrite_u32_array, pread_u32_array, pwrite_u32_repeat, sum_u32, min_u32, max_u32, mean_u32,
    std_u32, var_u32, median_u32, first_u32, last_u32, count_ge_u32
);
fixed_numeric!(
    /// The signed 64-bit integer type (`i64`).
    Int64, i64, I64, i128,
    pwrite_i64_array, pread_i64_array, pwrite_i64_repeat, sum_i64, min_i64, max_i64, mean_i64,
    std_i64, var_i64, median_i64, first_i64, last_i64, count_ge_i64
);
fixed_numeric!(
    /// The unsigned 64-bit integer type (`u64`).
    UInt64, u64, U64, i128,
    pwrite_u64_array, pread_u64_array, pwrite_u64_repeat, sum_u64, min_u64, max_u64, mean_u64,
    std_u64, var_u64, median_u64, first_u64, last_u64, count_ge_u64
);
fixed_numeric!(
    /// The signed 128-bit integer type (`i128`).
    Int128, i128, I128, i128,
    pwrite_i128_array, pread_i128_array, pwrite_i128_repeat, sum_i128, min_i128, max_i128, mean_i128,
    std_i128, var_i128, median_i128, first_i128, last_i128, count_ge_i128
);
fixed_numeric!(
    /// The unsigned 128-bit integer type (`u128`).
    UInt128, u128, U128, u128,
    pwrite_u128_array, pread_u128_array, pwrite_u128_repeat, sum_u128, min_u128, max_u128, mean_u128,
    std_u128, var_u128, median_u128, first_u128, last_u128, count_ge_u128
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
    fn std<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<f64>, IoError> {
        Aggregate::std_u8(src, start, count)
    }
    fn var<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<f64>, IoError> {
        Aggregate::var_u8(src, start, count)
    }
    fn median<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<f64>, IoError> {
        Aggregate::median_u8(src, start, count)
    }
    fn first<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<u8>, IoError> {
        Aggregate::first_u8(src, start, count)
    }
    fn last<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<u8>, IoError> {
        Aggregate::last_u8(src, start, count)
    }
    fn count_ge<R: IOBase>(
        src: &R,
        start: u64,
        count: usize,
        threshold: u8,
    ) -> Result<usize, IoError> {
        Aggregate::count_ge_u8(src, start, count, threshold)
    }
}
