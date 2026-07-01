//! The [`AnyScalar`] dynamic scalar.

use crate::Scalar;
use yggdryl_schema::{Any, AnyField, AnyType, DataTypeId, Struct, I256, U256};

/// Generates the delegating `as_<type>` accessors — the scalar's atomic value
/// interface, forwarding to the wrapped [`Any`].
macro_rules! any_scalar_accessors {
    ($($method:ident : $native:ty),+ $(,)?) => {$(
        #[doc = concat!("The scalar's value as `", stringify!($native), "`, or `None` if it is another type.")]
        pub fn $method(&self) -> Option<$native> {
            self.value.$method()
        }
    )+};
}

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
/// // Atomic accessors read the value at its native type.
/// assert_eq!(scalar.as_u8(), Some(9));
/// assert_eq!(scalar.as_i8(), None); // wrong type → None
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

    /// Whether the scalar's value is [`Null`](Any::Null).
    pub fn is_null(&self) -> bool {
        self.value.is_null()
    }

    /// Whether the scalar's value is a struct.
    pub fn is_struct(&self) -> bool {
        self.value.is_struct()
    }

    any_scalar_accessors! {
        as_i8: i8,
        as_i16: i16,
        as_i32: i32,
        as_i64: i64,
        as_i128: i128,
        as_i256: I256,
        as_u8: u8,
        as_u16: u16,
        as_u32: u32,
        as_u64: u64,
        as_u128: u128,
        as_u256: U256,
    }

    /// The scalar's value as a [`Struct`], or `None` if it is another type.
    pub fn as_struct(&self) -> Option<&Struct> {
        self.value.as_struct()
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
