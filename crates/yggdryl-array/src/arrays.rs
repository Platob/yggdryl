//! The per-type array implementations: one type per family member
//! (`Int64Array`, `Float64Array`, …), each its own [`Array`] implementation
//! over the generic [`PrimitiveArray`] engine, so the `Int64Type` /
//! `Int64Field` / `Int64Scalar` / `Int64Array` family reads the same at
//! every layer.
//!
//! Every family member derefs to its [`PrimitiveArray`], so all the engine's
//! accessors ([`value`](PrimitiveArray::value),
//! [`scalar_at`](PrimitiveArray::scalar_at),
//! [`slice`](PrimitiveArray::slice), …) are available directly; the
//! constructors here drop the redundant data-type argument wherever the type
//! has no parameters. Boolean, variable-size and nested family members land
//! with the array types that back them.

use core::ops::Deref;

use arrow_buffer::{NullBuffer, ScalarBuffer};
use yggdryl_schema::{
    Date32Type, Date64Type, Decimal128Type, Decimal256Type, DurationType, Float32Type, Float64Type,
    Int16Type, Int32Type, Int64Type, Int8Type, PrimitiveType, Time32Type, Time32Unit, Time64Type,
    Time64Unit, TimeUnit, TimestampType, UInt16Type, UInt32Type, UInt64Type, UInt8Type,
};

use crate::{Array, ArrayError, PrimitiveArray};

/// Generates the shared shape of an array family member: the newtype, its
/// deref to the [`PrimitiveArray`] engine, the [`Array`] implementation, the
/// `From` conversions and the byte decoder.
macro_rules! array_common {
    ([$($g:tt)*] [$($p:tt)*] $name:ident, $ty:ty) => {
        impl<$($g)*> Deref for $name<$($p)*> {
            type Target = PrimitiveArray<$ty>;

            fn deref(&self) -> &PrimitiveArray<$ty> {
                &self.0
            }
        }

        impl<$($g)*> Array for $name<$($p)*> {
            type DataType = $ty;

            fn data_type(&self) -> &$ty {
                self.0.data_type()
            }

            fn len(&self) -> usize {
                Array::len(&self.0)
            }

            fn validity(&self) -> Option<&NullBuffer> {
                Array::validity(&self.0)
            }
        }

        impl<$($g)*> From<PrimitiveArray<$ty>> for $name<$($p)*> {
            fn from(array: PrimitiveArray<$ty>) -> Self {
                Self(array)
            }
        }

        impl<$($g)*> From<$name<$($p)*>> for PrimitiveArray<$ty> {
            fn from(array: $name<$($p)*>) -> Self {
                array.0
            }
        }

        impl<$($g)*> $name<$($p)*> {
            /// Deserializes the array from the encoding produced by
            /// [`PrimitiveArray::to_bytes`], validating fully.
            pub fn from_bytes(bytes: &[u8]) -> Result<Self, ArrayError> {
                PrimitiveArray::from_bytes(bytes).map(Self)
            }
        }
    };
}

/// A fixed-width array of a parameter-free type: the constructors drop the
/// data-type argument entirely.
macro_rules! native_array {
    ($(#[$doc:meta])* $name:ident, $ty:ty) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        #[cfg_attr(
            feature = "serde",
            derive(serde::Serialize, serde::Deserialize),
            serde(transparent)
        )]
        pub struct $name(PrimitiveArray<$ty>);

        array_common!([] [] $name, $ty);

        impl $name {
            /// Builds an all-valid array from native values.
            pub fn from_native(values: Vec<<$ty as PrimitiveType>::Native>) -> Self {
                Self(PrimitiveArray::from_native(<$ty>::default(), values))
            }

            /// Builds the array from optional natives; `None`s become nulls.
            pub fn from_options(values: Vec<Option<<$ty as PrimitiveType>::Native>>) -> Self {
                Self(PrimitiveArray::from_options(<$ty>::default(), values))
            }

            /// Builds the array from its parts, validating the lengths.
            pub fn from_parts(
                values: ScalarBuffer<<$ty as PrimitiveType>::Native>,
                validity: Option<NullBuffer>,
            ) -> Result<Self, ArrayError> {
                PrimitiveArray::from_parts(<$ty>::default(), values, validity).map(Self)
            }
        }
    };
}

/// A fixed-width array of a parameterized type: the constructors take the
/// data type first, like the engine's. The unit-generic arm spells out the
/// serde bounds the engine's `try_from`-based impls need.
macro_rules! native_param_array {
    (@ctors [$($g:tt)*] [$($p:tt)*] $name:ident, $ty:ty) => {
        array_common!([$($g)*] [$($p)*] $name, $ty);

        impl<$($g)*> $name<$($p)*> {
            /// Builds an all-valid array from native values.
            pub fn from_native(
                data_type: $ty,
                values: Vec<<$ty as PrimitiveType>::Native>,
            ) -> Self {
                Self(PrimitiveArray::from_native(data_type, values))
            }

            /// Builds the array from optional natives; `None`s become nulls.
            pub fn from_options(
                data_type: $ty,
                values: Vec<Option<<$ty as PrimitiveType>::Native>>,
            ) -> Self {
                Self(PrimitiveArray::from_options(data_type, values))
            }

            /// Builds the array from its parts, validating the lengths.
            pub fn from_parts(
                data_type: $ty,
                values: ScalarBuffer<<$ty as PrimitiveType>::Native>,
                validity: Option<NullBuffer>,
            ) -> Result<Self, ArrayError> {
                PrimitiveArray::from_parts(data_type, values, validity).map(Self)
            }
        }
    };
    ($(#[$doc:meta])* $name:ident, $ty:ty) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        #[cfg_attr(
            feature = "serde",
            derive(serde::Serialize, serde::Deserialize),
            serde(transparent)
        )]
        pub struct $name(PrimitiveArray<$ty>);

        native_param_array!(@ctors [] [] $name, $ty);
    };
    ($(#[$doc:meta])* [U: $bound:path] [U] $name:ident, $ty:ty) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        #[cfg_attr(
            feature = "serde",
            derive(serde::Serialize, serde::Deserialize),
            serde(
                transparent,
                bound(
                    serialize = "U: serde::Serialize",
                    deserialize = "U: serde::de::DeserializeOwned"
                )
            )
        )]
        pub struct $name<U: $bound>(PrimitiveArray<$ty>);

        native_param_array!(@ctors [U: $bound] [U] $name, $ty);
    };
}

native_array!(
    /// An array of [`Int8Type`].
    Int8Array, Int8Type
);
native_array!(
    /// An array of [`Int16Type`].
    Int16Array, Int16Type
);
native_array!(
    /// An array of [`Int32Type`].
    Int32Array, Int32Type
);
native_array!(
    /// An array of [`Int64Type`].
    ///
    /// ```
    /// use yggdryl_array::{Array, Int64Array};
    ///
    /// let column = Int64Array::from_options(vec![Some(1), None, Some(3)]);
    /// assert_eq!(column.len(), 3);
    /// assert_eq!(column.scalar_at(0).unwrap().as_i64(), Some(1));
    /// ```
    Int64Array, Int64Type
);
native_array!(
    /// An array of [`UInt8Type`].
    UInt8Array, UInt8Type
);
native_array!(
    /// An array of [`UInt16Type`].
    UInt16Array, UInt16Type
);
native_array!(
    /// An array of [`UInt32Type`].
    UInt32Array, UInt32Type
);
native_array!(
    /// An array of [`UInt64Type`].
    UInt64Array, UInt64Type
);
native_array!(
    /// An array of [`Float32Type`].
    Float32Array, Float32Type
);
native_array!(
    /// An array of [`Float64Type`].
    Float64Array, Float64Type
);
native_array!(
    /// An array of [`Date32Type`].
    Date32Array, Date32Type
);
native_array!(
    /// An array of [`Date64Type`].
    Date64Array, Date64Type
);
native_param_array!(
    /// An array of [`Decimal128Type`].
    Decimal128Array, Decimal128Type
);
native_param_array!(
    /// An array of [`Decimal256Type`].
    Decimal256Array, Decimal256Type
);
native_param_array!(
    /// An array of [`Time32Type`] over the unit `U`.
    [U: Time32Unit] [U] Time32Array, Time32Type<U>
);
native_param_array!(
    /// An array of [`Time64Type`] over the unit `U`.
    [U: Time64Unit] [U] Time64Array, Time64Type<U>
);
native_param_array!(
    /// An array of [`TimestampType`] over the unit `U`.
    [U: TimeUnit] [U] TimestampArray, TimestampType<U>
);
native_param_array!(
    /// An array of [`DurationType`] over the unit `U`.
    [U: TimeUnit] [U] DurationArray, DurationType<U>
);
