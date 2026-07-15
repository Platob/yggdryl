//! `io::fixed::integer` — the fixed-width **integer** primitives: unsigned `u8`…`u64`, `u96`,
//! `u128`, `u256` and signed `i8`…`i64`, `i96`, `i128`, `i256`. Each width is a handful of macro
//! invocations (grouped by signedness below) over the generic `fixed` building blocks; every one
//! reports [`DataTypeCategory::UnsignedInteger`](crate::io::DataTypeCategory::UnsignedInteger) or
//! [`SignedInteger`](crate::io::DataTypeCategory::SignedInteger), so `dt.is_integer()` /
//! `dt.is_signed()` drill down without matching. The Rust-native widths (`u8`…`u64`, `u128`,
//! `i8`…`i64`, `i128`) use the built-in primitive; the 96/256-bit widths (`u96`/`i96`/`u256`/
//! `i256`) are `[u8; N]` little-endian newtypes (Arrow has no matching primitive).

use crate::io::fixed::{
    fixed_buffer, fixed_dtype, fixed_field, fixed_native, fixed_scalar, fixed_serie, native_only,
    wide_int,
};

// ---- unsigned -----------------------------------------------------------------------
fixed_native!(u8, "u8", UInt8Type, U8);
fixed_dtype!(U8DataType, u8);
fixed_field!(U8Field, u8);
fixed_scalar!(U8Scalar, u8);
fixed_serie!(U8Serie, u8);
fixed_buffer!(U8Buffer, u8);
fixed_native!(u16, "u16", UInt16Type, U16);
fixed_dtype!(U16DataType, u16);
fixed_field!(U16Field, u16);
fixed_scalar!(U16Scalar, u16);
fixed_serie!(U16Serie, u16);
fixed_buffer!(U16Buffer, u16);
fixed_native!(u32, "u32", UInt32Type, U32);
fixed_dtype!(U32DataType, u32);
fixed_field!(U32Field, u32);
fixed_scalar!(U32Scalar, u32);
fixed_serie!(U32Serie, u32);
fixed_buffer!(U32Buffer, u32);
fixed_native!(u64, "u64", UInt64Type, U64);
fixed_dtype!(U64DataType, u64);
fixed_field!(U64Field, u64);
fixed_scalar!(U64Scalar, u64);
fixed_serie!(U64Serie, u64);
fixed_buffer!(U64Buffer, u64);
wide_int!(U96, 12, "u96", U96);
fixed_dtype!(U96DataType, U96);
fixed_field!(U96Field, U96);
fixed_scalar!(U96Scalar, U96);
fixed_serie!(U96Serie, U96);
fixed_buffer!(U96Buffer, U96);
native_only!(u128, "u128", U128);
fixed_dtype!(U128DataType, u128);
fixed_field!(U128Field, u128);
fixed_scalar!(U128Scalar, u128);
fixed_serie!(U128Serie, u128);
fixed_buffer!(U128Buffer, u128);
wide_int!(U256, 32, "u256", U256);
fixed_dtype!(U256DataType, U256);
fixed_field!(U256Field, U256);
fixed_scalar!(U256Scalar, U256);
fixed_serie!(U256Serie, U256);
fixed_buffer!(U256Buffer, U256);

// ---- signed -------------------------------------------------------------------------
fixed_native!(i8, "i8", Int8Type, I8);
fixed_dtype!(I8DataType, i8);
fixed_field!(I8Field, i8);
fixed_scalar!(I8Scalar, i8);
fixed_serie!(I8Serie, i8);
fixed_buffer!(I8Buffer, i8);
fixed_native!(i16, "i16", Int16Type, I16);
fixed_dtype!(I16DataType, i16);
fixed_field!(I16Field, i16);
fixed_scalar!(I16Scalar, i16);
fixed_serie!(I16Serie, i16);
fixed_buffer!(I16Buffer, i16);
fixed_native!(i32, "i32", Int32Type, I32);
fixed_dtype!(I32DataType, i32);
fixed_field!(I32Field, i32);
fixed_scalar!(I32Scalar, i32);
fixed_serie!(I32Serie, i32);
fixed_buffer!(I32Buffer, i32);
fixed_native!(i64, "i64", Int64Type, I64);
fixed_dtype!(I64DataType, i64);
fixed_field!(I64Field, i64);
fixed_scalar!(I64Scalar, i64);
fixed_serie!(I64Serie, i64);
fixed_buffer!(I64Buffer, i64);
wide_int!(I96, 12, "i96", I96);
fixed_dtype!(I96DataType, I96);
fixed_field!(I96Field, I96);
fixed_scalar!(I96Scalar, I96);
fixed_serie!(I96Serie, I96);
fixed_buffer!(I96Buffer, I96);
native_only!(i128, "i128", I128);
fixed_dtype!(I128DataType, i128);
fixed_field!(I128Field, i128);
fixed_scalar!(I128Scalar, i128);
fixed_serie!(I128Serie, i128);
fixed_buffer!(I128Buffer, i128);
wide_int!(I256, 32, "i256", I256);
fixed_dtype!(I256DataType, I256);
fixed_field!(I256Field, I256);
fixed_scalar!(I256Scalar, I256);
fixed_serie!(I256Serie, I256);
fixed_buffer!(I256Buffer, I256);

/// The project's in-memory byte buffer — the historical name for a `u8` buffer ([`U8Buffer`]).
/// Implements the whole byte-I/O family ([`IOBase`](crate::io::IOBase) /
/// [`IOCursor`](crate::io::IOCursor) / [`IOSlice`](crate::io::IOSlice)); it is the type the
/// Python / Node bindings hold.
pub type Bytes = U8Buffer;
