//! The per-type field implementations: one type per family member
//! (`Int64Field`, `Utf8Field`, …), each its own [`Field`] implementation
//! delegating to the generic [`TypedField`] engine, so the
//! `Int64Type` / `Int64Field` / `Int64Scalar` / `Int64Array` family reads
//! the same at every layer.

use core::fmt;
use std::collections::BTreeMap;

use crate::{
    AnyDataType, BinaryType, BooleanType, DataType, Date32Type, Date64Type, Decimal128Type,
    Decimal256Type, DurationType, Field, FixedSizeBinaryType, Float32Type, Float64Type, Int16Type,
    Int32Type, Int64Type, Int8Type, LargeBinaryType, LargeListType, LargeUtf8Type, ListType,
    MapType, StructType, Time32Type, Time32Unit, Time64Type, Time64Unit, TimeUnit, TimestampType,
    TypedField, Utf8Type,
};

/// Generates one field implementation: a newtype over [`TypedField`]
/// implementing [`Field`] by one-line delegation, with `From` conversions to
/// and from the engine. The `@generic` arm covers the unit- and
/// item-parameterized types.
macro_rules! field_type {
    (@generic $(#[$doc:meta])* [$($g:tt)*] [$($p:tt)*] $name:ident, $ty:ty) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        #[cfg_attr(
            feature = "serde",
            derive(serde::Serialize, serde::Deserialize),
            serde(transparent)
        )]
        pub struct $name<$($g)*>(TypedField<$ty>);

        impl<$($g)*> Field for $name<$($p)*> {
            type DataType = $ty;

            fn from_parts(
                name: impl Into<String>,
                data_type: $ty,
                nullable: bool,
                metadata: BTreeMap<String, String>,
            ) -> Self {
                Self(TypedField::from_parts(name, data_type, nullable, metadata))
            }

            fn name(&self) -> &str {
                self.0.name()
            }

            fn data_type(&self) -> &$ty {
                self.0.data_type()
            }

            fn nullable(&self) -> bool {
                self.0.nullable()
            }

            fn metadata(&self) -> &BTreeMap<String, String> {
                self.0.metadata()
            }
        }

        impl<$($g)*> fmt::Display for $name<$($p)*> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl<$($g)*> From<TypedField<$ty>> for $name<$($p)*> {
            fn from(field: TypedField<$ty>) -> Self {
                Self(field)
            }
        }

        impl<$($g)*> From<$name<$($p)*>> for TypedField<$ty> {
            fn from(field: $name<$($p)*>) -> Self {
                field.0
            }
        }
    };
    ($(#[$doc:meta])* $name:ident, $ty:ty) => {
        field_type!(@generic $(#[$doc])* [] [] $name, $ty);
    };
}

field_type!(
    /// A field of [`BooleanType`].
    BooleanField, BooleanType
);
field_type!(
    /// A field of [`Int8Type`].
    Int8Field, Int8Type
);
field_type!(
    /// A field of [`Int16Type`].
    Int16Field, Int16Type
);
field_type!(
    /// A field of [`Int32Type`].
    Int32Field, Int32Type
);
field_type!(
    /// A field of [`Int64Type`].
    ///
    /// ```
    /// use yggdryl_schema::{Field, Int64Field, Int64Type};
    ///
    /// let field = Int64Field::from_parts("id", Int64Type, false, Default::default());
    /// assert_eq!(field.name(), "id");
    /// assert_eq!(Int64Field::from_arrow(&field.to_arrow()), Ok(field));
    /// ```
    Int64Field, Int64Type
);
field_type!(
    /// A field of [`UInt8Type`](crate::UInt8Type).
    UInt8Field, crate::UInt8Type
);
field_type!(
    /// A field of [`UInt16Type`](crate::UInt16Type).
    UInt16Field, crate::UInt16Type
);
field_type!(
    /// A field of [`UInt32Type`](crate::UInt32Type).
    UInt32Field, crate::UInt32Type
);
field_type!(
    /// A field of [`UInt64Type`](crate::UInt64Type).
    UInt64Field, crate::UInt64Type
);
field_type!(
    /// A field of [`Float32Type`].
    Float32Field, Float32Type
);
field_type!(
    /// A field of [`Float64Type`].
    Float64Field, Float64Type
);
field_type!(
    /// A field of [`Decimal128Type`].
    Decimal128Field, Decimal128Type
);
field_type!(
    /// A field of [`Decimal256Type`].
    Decimal256Field, Decimal256Type
);
field_type!(
    /// A field of [`Utf8Type`].
    Utf8Field, Utf8Type
);
field_type!(
    /// A field of [`LargeUtf8Type`].
    LargeUtf8Field, LargeUtf8Type
);
field_type!(
    /// A field of [`BinaryType`].
    BinaryField, BinaryType
);
field_type!(
    /// A field of [`LargeBinaryType`].
    LargeBinaryField, LargeBinaryType
);
field_type!(
    /// A field of [`FixedSizeBinaryType`].
    FixedSizeBinaryField, FixedSizeBinaryType
);
field_type!(
    /// A field of [`Date32Type`].
    Date32Field, Date32Type
);
field_type!(
    /// A field of [`Date64Type`].
    Date64Field, Date64Type
);
field_type!(@generic
    /// A field of [`Time32Type`] over the unit `U`.
    [U: Time32Unit] [U] Time32Field, Time32Type<U>
);
field_type!(@generic
    /// A field of [`Time64Type`] over the unit `U`.
    [U: Time64Unit] [U] Time64Field, Time64Type<U>
);
field_type!(@generic
    /// A field of [`TimestampType`] over the unit `U`.
    [U: TimeUnit] [U] TimestampField, TimestampType<U>
);
field_type!(@generic
    /// A field of [`DurationType`] over the unit `U`.
    [U: TimeUnit] [U] DurationField, DurationType<U>
);
field_type!(@generic
    /// A field of [`ListType`] over the item type `T`.
    [T: DataType] [T] ListField, ListType<T>
);
field_type!(@generic
    /// A field of [`LargeListType`] over the item type `T`.
    [T: DataType] [T] LargeListField, LargeListType<T>
);
field_type!(
    /// A field of [`StructType`].
    StructField, StructType
);
field_type!(
    /// A field of [`MapType`].
    MapField, MapType
);
field_type!(
    /// A field of the erased [`AnyDataType`].
    AnyField, AnyDataType
);
