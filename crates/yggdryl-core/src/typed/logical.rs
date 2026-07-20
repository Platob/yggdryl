//! [`LogicalType`] — the **logical → physical** storage descriptor a type contributes to the
//! isolated any→any converter ([`convert_column`](crate::typed::convert_column)).
//!
//! A **logical** type is a thin label over a **physical** byte layout that a cheaper representation
//! already handles: a fixed-point **decimal** is stored as its **unscaled backing integer**
//! (`Decimal128` over an `i128`), and a **UTF-8** string is stored as the **same bytes** a `Binary`
//! column holds. `LogicalType` states that mapping — its [`LOGICAL_ID`](LogicalType::LOGICAL_ID) and
//! the [`physical_dtype`](LogicalType::physical_dtype) it decays to — so the bulk converter can
//! **reuse the physical kernel** it already has (the numeric [`resize_dtype`](crate::io::memory::IOBase::resize_dtype)
//! for decimals, the offsets+data reinterpret for strings) instead of re-deriving the conversion per
//! logical type.
//!
//! ```
//! use yggdryl_core::datatype_id::DataTypeId;
//! use yggdryl_core::typed::LogicalType;
//! use yggdryl_core::typed::fixedbyte::Decimal128;
//! use yggdryl_core::typed::varbyte::Utf8;
//!
//! // A decimal decays to its unscaled integer; a utf8 string to the same binary bytes.
//! assert_eq!(Decimal128::physical_dtype(), DataTypeId::I128);
//! assert_eq!(Utf8::physical_dtype(), DataTypeId::Binary);
//! ```

use crate::datatype_id::DataTypeId;

/// A **logical** element type over a cheaper **physical** storage. The converter routes a logical
/// value through its physical form — a `decimal → numeric` cast reinterprets the unscaled integer,
/// a `utf8 → binary` cast reinterprets the identical bytes — so the one optimized physical kernel is
/// reused rather than a per-type scalar loop.
///
// DESIGN: the trait is deliberately **two hooks** — the logical id and the physical dtype. A
// value-level `logical Value <-> physical Value` map is intentionally omitted: the bulk converter
// operates on the shared physical **buffer** (a decimal column's data buffer already *is* an
// `i{32,64,128}` buffer; a utf8 column's data buffer already *is* the binary bytes), so there is
// nothing to translate per element — adding a value hook would be an unused abstraction. The
// runtime companion `physical_dtype` in the converter mirrors these impls for the id-dispatched
// path, and a test pins the two in lockstep.
pub trait LogicalType {
    /// The logical type's own [`DataTypeId`] (e.g. [`Decimal128`](DataTypeId::Decimal128),
    /// [`Utf8`](DataTypeId::Utf8)).
    const LOGICAL_ID: DataTypeId;

    /// The **physical** element type the logical values are stored as — the representation the bulk
    /// converter operates on. A decimal decays to its unscaled signed integer (`Decimal32` → `I32`,
    /// …); a variable/fixed UTF-8 string decays to the matching binary layout (`Utf8` → `Binary`,
    /// `LargeUtf8` → `LargeBinary`, `FixedUtf8` → `FixedBinary`) whose bytes are byte-for-byte
    /// identical.
    fn physical_dtype() -> DataTypeId;
}

/// Implements [`LogicalType`] for a type whose physical form is a **different** dtype (the common
/// case). One line per `logical => physical` pair.
macro_rules! impl_logical {
    ( $( $logical:ty => ($logical_id:ident, $physical_id:ident) ),+ $(,)? ) => {$(
        impl LogicalType for $logical {
            const LOGICAL_ID: DataTypeId = DataTypeId::$logical_id;
            fn physical_dtype() -> DataTypeId {
                DataTypeId::$physical_id
            }
        }
    )+};
}

use crate::typed::fixedbyte::{
    Decimal128, Decimal16, Decimal256, Decimal32, Decimal64, Decimal8, FixedUtf8,
};
use crate::typed::varbyte::{LargeUtf8, Utf8};

impl_logical! {
    // Decimals decay to their unscaled backing integer (same bytes, same width).
    Decimal8   => (Decimal8, I8),
    Decimal16  => (Decimal16, I16),
    Decimal32  => (Decimal32, I32),
    Decimal64  => (Decimal64, I64),
    Decimal128 => (Decimal128, I128),
    // UTF-8 strings decay to the byte-identical binary layout.
    Utf8       => (Utf8, Binary),
    LargeUtf8  => (LargeUtf8, LargeBinary),
    FixedUtf8  => (FixedUtf8, FixedBinary),
}

impl LogicalType for Decimal256 {
    const LOGICAL_ID: DataTypeId = DataTypeId::Decimal256;
    // DESIGN: `Decimal256`'s unscaled physical is a 256-bit signed integer, which has **no**
    // `DataTypeId` of its own (there is no `i256` numeric column type — `I256` exists only as this
    // decimal's backing). So its physical dtype is itself: the converter reads this as "no simpler
    // numeric physical", and a `decimal256 ↔ numeric` cross-cast (which would have to funnel through
    // the `f64` carrier and lose its > 2^53 magnitude) surfaces a guided error instead. Its faithful
    // conversions — same-dtype, and `decimal256 → utf8` via the scale-aware `to_decimal_string` — do
    // not need a numeric physical.
    fn physical_dtype() -> DataTypeId {
        DataTypeId::Decimal256
    }
}
