//! The [`AnyScalar`] dynamic scalar.

use crate::Scalar;
use yggdryl_schema::{Any, AnyField, AnyType, DataTypeId, I256, U256};

/// A scalar of any type, resolved at run time — the dynamic counterpart of the typed
/// [`Scalar`] impls (mirroring [`AnyField`]). It pairs an [`Any`] value with an
/// [`AnyField`], builds straight from any native type, and is the child scalar a
/// [`StructScalar`](crate::StructScalar) holds.
///
/// ```
/// use yggdryl_scalar::{AnyScalar, Scalar};
/// use yggdryl_schema::{Any, DataType, DataTypeId};
///
/// let scalar = AnyScalar::from(9u8).with_name("age".to_string());
/// assert_eq!(*scalar.value(), Any::UInt8(9));
/// assert_eq!(scalar.name(), "age");
/// assert_eq!(scalar.field().any_type().type_id(), DataTypeId::UInt8);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AnyScalar {
    field: AnyField,
    value: Any,
}

impl AnyScalar {
    /// The scalar from its explicit field and value.
    pub fn from_parts(field: AnyField, value: Any) -> Self {
        Self { field, value }
    }

    /// A copy carrying `field`.
    pub fn with_field(&self, field: AnyField) -> Self {
        Self {
            field,
            value: self.value.clone(),
        }
    }

    /// A copy renamed to `name`.
    pub fn with_name(&self, name: String) -> Self {
        self.with_field(self.field.with_name(name))
    }

    /// A copy holding `value`.
    pub fn with_value(&self, value: Any) -> Self {
        Self {
            field: self.field.clone(),
            value,
        }
    }
}

/// Generates the `From<native>` builders, each pairing the native value with an
/// unnamed primitive field of the matching type.
macro_rules! any_scalar_from_native {
    ($($native:ty => $variant:ident),+ $(,)?) => {$(
        impl From<$native> for AnyScalar {
            fn from(value: $native) -> Self {
                Self::from_parts(
                    AnyField::new("", AnyType::primitive(DataTypeId::$variant)),
                    Any::$variant(value),
                )
            }
        }
    )+};
}

any_scalar_from_native! {
    i8 => Int8, i16 => Int16, i32 => Int32, i64 => Int64, i128 => Int128, I256 => Int256,
    u8 => UInt8, u16 => UInt16, u32 => UInt32, u64 => UInt64, u128 => UInt128, U256 => UInt256,
}

impl Scalar<Any> for AnyScalar {
    type Field = AnyField;

    fn field(&self) -> &AnyField {
        &self.field
    }

    fn value(&self) -> &Any {
        &self.value
    }
}
