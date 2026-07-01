//! The [`AnyValue`] dynamic value.

use yggdryl_core::{I256, U256};
use yggdryl_schema::DataTypeId;

use crate::StructValue;

/// Generates the `as_<type>` accessors, each returning the wrapped native value or
/// `None` when the value is of another type.
macro_rules! any_value_accessors {
    ($($variant:ident => $method:ident : $native:ty),+ $(,)?) => {$(
        #[doc = concat!("The wrapped `", stringify!($native), "`, or `None` if this value is another type.")]
        pub fn $method(&self) -> Option<$native> {
            match self {
                AnyValue::$variant(value) => Some(*value),
                _ => None,
            }
        }
    )+};
}

/// A value of any type — the dynamic value the scalar layer wraps (the counterpart of
/// [`AnyType`](crate::AnyType)). It covers the primitive values plus the recursive
/// [`StructValue`], so a struct value is an array of `AnyValue`. Defaults to
/// [`Null`](AnyValue::Null).
///
/// ```
/// use yggdryl_scalar::AnyValue;
/// use yggdryl_schema::DataTypeId;
///
/// assert_eq!(AnyValue::default(), AnyValue::Null);
/// assert_eq!(AnyValue::Int32(7).type_id(), DataTypeId::Int32);
/// assert_eq!(AnyValue::Int32(7).as_i32(), Some(7));
/// assert_eq!(AnyValue::Int32(7).as_i64(), None); // wrong type → None
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum AnyValue {
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
    /// A struct value — an array of `AnyValue`.
    Struct(StructValue),
}

impl AnyValue {
    /// The [`DataTypeId`] of this value's type. [`Null`](AnyValue::Null) reports
    /// [`Null`](DataTypeId::Null).
    pub fn type_id(&self) -> DataTypeId {
        match self {
            AnyValue::Null => DataTypeId::Null,
            AnyValue::Int8(_) => DataTypeId::Int8,
            AnyValue::Int16(_) => DataTypeId::Int16,
            AnyValue::Int32(_) => DataTypeId::Int32,
            AnyValue::Int64(_) => DataTypeId::Int64,
            AnyValue::Int128(_) => DataTypeId::Int128,
            AnyValue::Int256(_) => DataTypeId::Int256,
            AnyValue::UInt8(_) => DataTypeId::UInt8,
            AnyValue::UInt16(_) => DataTypeId::UInt16,
            AnyValue::UInt32(_) => DataTypeId::UInt32,
            AnyValue::UInt64(_) => DataTypeId::UInt64,
            AnyValue::UInt128(_) => DataTypeId::UInt128,
            AnyValue::UInt256(_) => DataTypeId::UInt256,
            AnyValue::Struct(_) => DataTypeId::Struct,
        }
    }

    /// Whether this is the [`Null`](AnyValue::Null) value.
    pub fn is_null(&self) -> bool {
        matches!(self, AnyValue::Null)
    }

    any_value_accessors! {
        Int8 => as_i8 : i8,
        Int16 => as_i16 : i16,
        Int32 => as_i32 : i32,
        Int64 => as_i64 : i64,
        Int128 => as_i128 : i128,
        Int256 => as_i256 : I256,
        UInt8 => as_u8 : u8,
        UInt16 => as_u16 : u16,
        UInt32 => as_u32 : u32,
        UInt64 => as_u64 : u64,
        UInt128 => as_u128 : u128,
        UInt256 => as_u256 : U256,
    }

    /// Whether this is a [`Struct`](AnyValue::Struct) value.
    pub fn is_struct(&self) -> bool {
        matches!(self, AnyValue::Struct(_))
    }

    /// The wrapped [`StructValue`], or `None` if this value is another type.
    pub fn as_struct(&self) -> Option<&StructValue> {
        match self {
            AnyValue::Struct(value) => Some(value),
            _ => None,
        }
    }
}
