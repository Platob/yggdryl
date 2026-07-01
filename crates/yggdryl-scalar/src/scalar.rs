//! The [`Scalar`] trait.

use yggdryl_core::{I256, U256};
use yggdryl_schema::DataTypeId;

use crate::Any;

/// A scalar value — it reports its [`DataTypeId`] and promotes to the dynamic [`Any`].
/// Implemented by [`Any`], [`Struct`](crate::Struct) and the native Rust values
/// (`i8`…`u128`, [`I256`] / [`U256`]), so nested structures hold and build from any
/// scalar uniformly.
///
/// ```
/// use yggdryl_scalar::{Any, Scalar};
/// use yggdryl_schema::DataTypeId;
///
/// assert_eq!(7i32.type_id(), DataTypeId::Int32);
/// assert_eq!(7i32.to_any(), Any::Int32(7));
/// assert!(!7i32.is_null());
/// ```
pub trait Scalar {
    /// The type discriminant of this scalar's value.
    fn type_id(&self) -> DataTypeId;

    /// Whether this scalar is the null value.
    fn is_null(&self) -> bool {
        false
    }

    /// This scalar as the dynamic [`Any`] value.
    fn to_any(&self) -> Any;
}

/// Implements [`Scalar`] for each native primitive, mapping it to its [`DataTypeId`]
/// and its [`Any`] variant.
macro_rules! primitive_scalars {
    ($($native:ty => $variant:ident),+ $(,)?) => {$(
        impl Scalar for $native {
            fn type_id(&self) -> DataTypeId {
                DataTypeId::$variant
            }

            fn to_any(&self) -> Any {
                Any::$variant(*self)
            }
        }
    )+};
}

primitive_scalars! {
    i8 => Int8, i16 => Int16, i32 => Int32, i64 => Int64, i128 => Int128, I256 => Int256,
    u8 => UInt8, u16 => UInt16, u32 => UInt32, u64 => UInt64, u128 => UInt128, U256 => UInt256,
}

impl Scalar for Any {
    fn type_id(&self) -> DataTypeId {
        Any::type_id(self)
    }

    fn is_null(&self) -> bool {
        Any::is_null(self)
    }

    fn to_any(&self) -> Any {
        self.clone()
    }
}
