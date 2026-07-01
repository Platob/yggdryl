//! The [`Any`] dynamic value.

use yggdryl_core::{I256, U256};

use crate::dtype::DataTypeId;
use crate::value::Struct;

/// A value of any type — the dynamic counterpart of [`AnyType`](crate::AnyType). It
/// covers the primitive values plus the recursive [`Struct`], so a struct value is
/// an array of `Any`. Defaults to [`Null`](Any::Null).
///
/// ```
/// use yggdryl_schema::{Any, DataTypeId};
///
/// assert_eq!(Any::default(), Any::Null);
/// assert_eq!(Any::Int32(7).type_id(), DataTypeId::Int32);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Any {
    /// The null value.
    #[default]
    Null,
    /// An 8-bit signed integer.
    Int8(i8),
    /// A 16-bit signed integer.
    Int16(i16),
    /// A 32-bit signed integer.
    Int32(i32),
    /// A 64-bit signed integer.
    Int64(i64),
    /// A 128-bit signed integer.
    Int128(i128),
    /// A 256-bit signed integer.
    Int256(I256),
    /// An 8-bit unsigned integer.
    UInt8(u8),
    /// A 16-bit unsigned integer.
    UInt16(u16),
    /// A 32-bit unsigned integer.
    UInt32(u32),
    /// A 64-bit unsigned integer.
    UInt64(u64),
    /// A 128-bit unsigned integer.
    UInt128(u128),
    /// A 256-bit unsigned integer.
    UInt256(U256),
    /// A struct value — an array of `Any`.
    Struct(Struct),
}

impl Any {
    /// The [`DataTypeId`] of this value's type. [`Null`](Any::Null) reports
    /// [`Null`](DataTypeId::Null).
    pub fn type_id(&self) -> DataTypeId {
        match self {
            Any::Null => DataTypeId::Null,
            Any::Int8(_) => DataTypeId::Int8,
            Any::Int16(_) => DataTypeId::Int16,
            Any::Int32(_) => DataTypeId::Int32,
            Any::Int64(_) => DataTypeId::Int64,
            Any::Int128(_) => DataTypeId::Int128,
            Any::Int256(_) => DataTypeId::Int256,
            Any::UInt8(_) => DataTypeId::UInt8,
            Any::UInt16(_) => DataTypeId::UInt16,
            Any::UInt32(_) => DataTypeId::UInt32,
            Any::UInt64(_) => DataTypeId::UInt64,
            Any::UInt128(_) => DataTypeId::UInt128,
            Any::UInt256(_) => DataTypeId::UInt256,
            Any::Struct(_) => DataTypeId::Struct,
        }
    }

    /// Whether this is the [`Null`](Any::Null) value.
    pub fn is_null(&self) -> bool {
        matches!(self, Any::Null)
    }
}
