//! The fixed-point **decimal** element types — `Decimal32` / `Decimal64` / `Decimal128` /
//! `Decimal256` — each a signed unscaled integer (`i32` / `i64` / `i128` / [`I256`]) packed at its
//! byte width. `Decimal32`…`Decimal128` are one [`fixed_decimal!`] line over the source's typed
//! integer arrays; `Decimal256` writes its 32 bytes directly. All four share the
//! [`Decimal`](crate::typed::Decimal) trait (precision + scale-aware `format`); precision/scale live
//! in the [`Field`](crate::typed::Field) metadata.

use super::I256;
use crate::datatype_id::DataTypeId;
use crate::io::memory::{IOBase, IoError};
use crate::typed::{DataType, Decimal, Decoder, Encoder};

/// The implementation rule for a decimal whose unscaled value is a **native integer** — the marker
/// plus its `DataType` / `Encoder` / `Decoder` / `Decimal` impls over the source's typed integer
/// arrays. Params: `Marker, native int, DataTypeId, MAX_PRECISION, pwrite_array, pread_array`.
macro_rules! fixed_decimal {
    (
        $(#[$doc:meta])*
        $marker:ident, $native:ty, $id:ident, $prec:expr, $warr:ident, $rarr:ident
    ) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $marker;

        impl DataType for $marker {
            type Native = $native;
            const DATA_TYPE_ID: DataTypeId = DataTypeId::$id;
        }

        impl Encoder for $marker {
            fn encode<W: IOBase>(dst: &mut W, index: u64, value: $native) -> Result<(), IoError> {
                <Self as Encoder>::encode_slice(dst, index, &[value])
            }
            fn encode_slice<W: IOBase>(
                dst: &mut W,
                start: u64,
                values: &[$native],
            ) -> Result<(), IoError> {
                dst.$warr(start * core::mem::size_of::<$native>() as u64, values)
            }
        }

        impl Decoder for $marker {
            fn decode<R: IOBase>(src: &R, index: u64) -> Result<$native, IoError> {
                let mut out = [<$native>::default(); 1];
                <Self as Decoder>::decode_slice(src, index, &mut out)?;
                Ok(out[0])
            }
            fn decode_slice<R: IOBase>(
                src: &R,
                start: u64,
                out: &mut [$native],
            ) -> Result<(), IoError> {
                src.$rarr(start * core::mem::size_of::<$native>() as u64, out)
            }
        }

        impl Decimal for $marker {
            const MAX_PRECISION: u32 = $prec;
        }
    };
}

fixed_decimal!(
    /// 32-bit fixed-point decimal — an unscaled `i32` (up to 9 significant digits).
    Decimal32, i32, Decimal32, 9,
    pwrite_i32_array, pread_i32_array
);
fixed_decimal!(
    /// 64-bit fixed-point decimal — an unscaled `i64` (up to 18 significant digits).
    Decimal64, i64, Decimal64, 18,
    pwrite_i64_array, pread_i64_array
);
fixed_decimal!(
    /// 128-bit fixed-point decimal — an unscaled `i128` (up to 38 significant digits).
    Decimal128, i128, Decimal128, 38,
    pwrite_i128_array, pread_i128_array
);

/// 256-bit fixed-point decimal — an unscaled [`I256`] (up to 76 significant digits). There is no
/// typed `i256` array, so it reads/writes its 32 little-endian bytes per element directly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Decimal256;

impl DataType for Decimal256 {
    type Native = I256;
    const DATA_TYPE_ID: DataTypeId = DataTypeId::Decimal256;
}

impl Encoder for Decimal256 {
    fn encode<W: IOBase>(dst: &mut W, index: u64, value: I256) -> Result<(), IoError> {
        Self::encode_slice(dst, index, &[value])
    }
    fn encode_slice<W: IOBase>(dst: &mut W, start: u64, values: &[I256]) -> Result<(), IoError> {
        #[cfg(target_endian = "little")]
        {
            // `I256` is `#[repr(C)] { lo, hi }`, so on little-endian its 32 bytes ARE its wire bytes:
            // the whole slice is one `memcpy`, no per-element write, no temporary. SAFETY: `values`
            // is `values.len() * 32` contiguous bytes of plain data (no padding).
            let bytes = unsafe {
                core::slice::from_raw_parts(values.as_ptr().cast::<u8>(), values.len() * 32)
            };
            dst.pwrite_byte_array(start * 32, bytes);
        }
        #[cfg(target_endian = "big")]
        {
            for (offset, value) in values.iter().enumerate() {
                dst.pwrite_byte_array((start + offset as u64) * 32, &value.to_le_bytes());
            }
        }
        Ok(())
    }
}

impl Decoder for Decimal256 {
    fn decode<R: IOBase>(src: &R, index: u64) -> Result<I256, IoError> {
        let mut out = [I256::ZERO; 1];
        Self::decode_slice(src, index, &mut out)?;
        Ok(out[0])
    }
    fn decode_slice<R: IOBase>(src: &R, start: u64, out: &mut [I256]) -> Result<(), IoError> {
        #[cfg(target_endian = "little")]
        {
            // SAFETY: read the wire bytes straight into the `I256` slice's memory — `out` is
            // `out.len() * 32` contiguous bytes of plain data.
            let need = out.len() * 32;
            let bytes =
                unsafe { core::slice::from_raw_parts_mut(out.as_mut_ptr().cast::<u8>(), need) };
            src.pread_exact(start * 32, bytes)
        }
        #[cfg(target_endian = "big")]
        {
            for (offset, slot) in out.iter_mut().enumerate() {
                let mut bytes = [0u8; 32];
                src.pread_exact((start + offset as u64) * 32, &mut bytes)?;
                *slot = I256::from_le_bytes(bytes);
            }
            Ok(())
        }
    }
}

impl Decimal for Decimal256 {
    const MAX_PRECISION: u32 = 76;
}
