//! The [`Any`] dynamic scalar value.

use yggdryl_core::{I256, U256};
use yggdryl_schema::DataTypeId;

use crate::Struct;

/// Generates the `as_<type>` accessors, each returning the wrapped native value or
/// `None` when the value is of another type.
macro_rules! any_accessors {
    ($($variant:ident => $method:ident : $native:ty),+ $(,)?) => {$(
        #[doc = concat!("The wrapped `", stringify!($native), "`, or `None` if this value is another type.")]
        pub fn $method(&self) -> Option<$native> {
            match self {
                Any::$variant(value) => Some(*value),
                _ => None,
            }
        }
    )+};
}

/// A scalar of any type — the dynamic value. It is either null, a primitive, or a
/// nested [`Struct`] (an array of `Any`), so scalars nest recursively. Along with
/// [`Struct`] and the native Rust values it implements the [`Scalar`](crate::Scalar)
/// trait. Defaults to [`Null`](Any::Null).
///
/// ```
/// use yggdryl_scalar::{Any, Scalar};
/// use yggdryl_schema::DataTypeId;
///
/// let v = Any::from(7i32);
/// assert_eq!(v, Any::Int32(7));
/// assert_eq!(v.type_id(), DataTypeId::Int32);
/// assert_eq!(v.as_i32(), Some(7));
/// assert_eq!(v.as_i64(), None); // wrong type → None
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
    /// A nested struct value — an array of `Any`.
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

    any_accessors! {
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

    /// Whether this is a [`Struct`](Any::Struct) value.
    pub fn is_struct(&self) -> bool {
        matches!(self, Any::Struct(_))
    }

    /// The wrapped [`Struct`], or `None` if this value is another type.
    pub fn as_struct(&self) -> Option<&Struct> {
        match self {
            Any::Struct(value) => Some(value),
            _ => None,
        }
    }
}

/// Generates the `From<native>` builders.
macro_rules! any_from_native {
    ($($native:ty => $variant:ident),+ $(,)?) => {$(
        impl From<$native> for Any {
            fn from(value: $native) -> Self {
                Any::$variant(value)
            }
        }
    )+};
}

any_from_native! {
    i8 => Int8, i16 => Int16, i32 => Int32, i64 => Int64, i128 => Int128, I256 => Int256,
    u8 => UInt8, u16 => UInt16, u32 => UInt32, u64 => UInt64, u128 => UInt128, U256 => UInt256,
}

impl From<Struct> for Any {
    fn from(value: Struct) -> Self {
        Any::Struct(value)
    }
}
