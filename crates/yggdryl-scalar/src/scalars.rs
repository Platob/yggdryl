//! The per-type scalar implementations: one type per family member
//! (`Int64Scalar`, `Utf8Scalar`, …), each its own implementation over the
//! generic [`Scalar`] engine, so the `Int64Type` / `Int64Field` /
//! `Int64Scalar` / `Int64Array` family reads the same at every layer.
//!
//! Every family member derefs to its [`Scalar`], so all the engine's
//! accessors ([`as_native`](Scalar::as_native), [`as_i64`](Scalar::as_i64),
//! [`as_str`](Scalar::as_str), …) are available directly; the constructors
//! here drop the redundant data-type argument wherever the type has no
//! parameters.

use core::ops::Deref;

use arrow_buffer::Buffer;
use yggdryl_schema::{
    BinaryType, BooleanType, Date32Type, Date64Type, Decimal128Type, Decimal256Type, DurationType,
    FixedSizeBinaryType, Float32Type, Float64Type, Int16Type, Int32Type, Int64Type, Int8Type,
    LargeBinaryType, LargeUtf8Type, PrimitiveType, Time32Type, Time32Unit, Time64Type, Time64Unit,
    TimeUnit, TimestampType, UInt16Type, UInt32Type, UInt64Type, UInt8Type, Utf8Type,
};

use crate::{Scalar, ScalarError};

/// Generates the shared shape of a scalar family member: the newtype, its
/// deref to the [`Scalar`] engine, the `From` conversions and the byte
/// decoder.
macro_rules! scalar_common {
    ([$($g:tt)*] [$($p:tt)*] $name:ident, $ty:ty) => {
        impl<$($g)*> Deref for $name<$($p)*> {
            type Target = Scalar<$ty>;

            fn deref(&self) -> &Scalar<$ty> {
                &self.0
            }
        }

        impl<$($g)*> From<Scalar<$ty>> for $name<$($p)*> {
            fn from(scalar: Scalar<$ty>) -> Self {
                Self(scalar)
            }
        }

        impl<$($g)*> From<$name<$($p)*>> for Scalar<$ty> {
            fn from(scalar: $name<$($p)*>) -> Self {
                scalar.0
            }
        }

        impl<$($g)*> $name<$($p)*> {
            /// Deserializes the scalar from the encoding produced by
            /// [`Scalar::to_bytes`], validating fully.
            pub fn from_bytes(bytes: &[u8]) -> Result<Self, ScalarError> {
                Scalar::from_bytes(bytes).map(Self)
            }
        }
    };
}

/// A fixed-width scalar of a parameter-free type: the constructors drop the
/// data-type argument entirely.
macro_rules! native_scalar {
    ($(#[$doc:meta])* $name:ident, $ty:ty) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        #[cfg_attr(
            feature = "serde",
            derive(serde::Serialize, serde::Deserialize),
            serde(transparent)
        )]
        pub struct $name(Scalar<$ty>);

        scalar_common!([] [] $name, $ty);

        impl $name {
            /// The null scalar.
            pub fn null() -> Self {
                Self(Scalar::null(<$ty>::default()))
            }

            /// Builds the scalar from a native value over a fresh buffer.
            pub fn from_native(value: <$ty as PrimitiveType>::Native) -> Self {
                Self(Scalar::from_native(<$ty>::default(), value))
            }

            /// Builds the scalar from an optional value buffer (`None` =
            /// null), validating it against the type's layout.
            pub fn from_parts(buffer: Option<Buffer>) -> Result<Self, ScalarError> {
                Scalar::from_parts(<$ty>::default(), buffer).map(Self)
            }
        }
    };
}

/// A fixed-width scalar of a parameterized type: the constructors take the
/// data type first, like the engine's. The unit-generic arm spells out the
/// serde bounds the engine's `try_from`-based impls need.
macro_rules! native_param_scalar {
    (@ctors [$($g:tt)*] [$($p:tt)*] $name:ident, $ty:ty) => {
        scalar_common!([$($g)*] [$($p)*] $name, $ty);

        impl<$($g)*> $name<$($p)*> {
            /// The null scalar of the given type.
            pub fn null(data_type: $ty) -> Self {
                Self(Scalar::null(data_type))
            }

            /// Builds the scalar from a native value over a fresh buffer.
            pub fn from_native(data_type: $ty, value: <$ty as PrimitiveType>::Native) -> Self {
                Self(Scalar::from_native(data_type, value))
            }

            /// Builds the scalar from an optional value buffer (`None` =
            /// null), validating it against the type's layout.
            pub fn from_parts(data_type: $ty, buffer: Option<Buffer>) -> Result<Self, ScalarError> {
                Scalar::from_parts(data_type, buffer).map(Self)
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
        pub struct $name(Scalar<$ty>);

        native_param_scalar!(@ctors [] [] $name, $ty);
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
        pub struct $name<U: $bound>(Scalar<$ty>);

        native_param_scalar!(@ctors [U: $bound] [U] $name, $ty);
    };
}

native_scalar!(
    /// A scalar of [`Int8Type`].
    Int8Scalar, Int8Type
);
native_scalar!(
    /// A scalar of [`Int16Type`].
    Int16Scalar, Int16Type
);
native_scalar!(
    /// A scalar of [`Int32Type`].
    Int32Scalar, Int32Type
);
native_scalar!(
    /// A scalar of [`Int64Type`].
    ///
    /// ```
    /// use yggdryl_scalar::Int64Scalar;
    ///
    /// assert_eq!(Int64Scalar::from_native(42).as_i64(), Some(42));
    /// assert!(Int64Scalar::null().is_null());
    /// ```
    Int64Scalar, Int64Type
);
native_scalar!(
    /// A scalar of [`UInt8Type`].
    UInt8Scalar, UInt8Type
);
native_scalar!(
    /// A scalar of [`UInt16Type`].
    UInt16Scalar, UInt16Type
);
native_scalar!(
    /// A scalar of [`UInt32Type`].
    UInt32Scalar, UInt32Type
);
native_scalar!(
    /// A scalar of [`UInt64Type`].
    UInt64Scalar, UInt64Type
);
native_scalar!(
    /// A scalar of [`Float32Type`].
    Float32Scalar, Float32Type
);
native_scalar!(
    /// A scalar of [`Float64Type`].
    Float64Scalar, Float64Type
);
native_scalar!(
    /// A scalar of [`Date32Type`].
    Date32Scalar, Date32Type
);
native_scalar!(
    /// A scalar of [`Date64Type`].
    Date64Scalar, Date64Type
);
native_param_scalar!(
    /// A scalar of [`Decimal128Type`].
    Decimal128Scalar, Decimal128Type
);
native_param_scalar!(
    /// A scalar of [`Decimal256Type`].
    Decimal256Scalar, Decimal256Type
);
native_param_scalar!(
    /// A scalar of [`Time32Type`] over the unit `U`.
    [U: Time32Unit] [U] Time32Scalar, Time32Type<U>
);
native_param_scalar!(
    /// A scalar of [`Time64Type`] over the unit `U`.
    [U: Time64Unit] [U] Time64Scalar, Time64Type<U>
);
native_param_scalar!(
    /// A scalar of [`TimestampType`] over the unit `U`.
    [U: TimeUnit] [U] TimestampScalar, TimestampType<U>
);
native_param_scalar!(
    /// A scalar of [`DurationType`] over the unit `U`.
    [U: TimeUnit] [U] DurationScalar, DurationType<U>
);

/// A scalar of [`BooleanType`]: one byte holding 0 or 1.
///
/// ```
/// use yggdryl_scalar::BooleanScalar;
///
/// assert_eq!(BooleanScalar::from_bool(true).as_bool(), Some(true));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(transparent)
)]
pub struct BooleanScalar(Scalar<BooleanType>);

scalar_common!([] [] BooleanScalar, BooleanType);

impl BooleanScalar {
    /// The null scalar.
    pub fn null() -> Self {
        Self(Scalar::null(BooleanType))
    }

    /// Builds the scalar from a boolean.
    pub fn from_bool(value: bool) -> Self {
        Self(Scalar::from_bool(value))
    }

    /// Builds the scalar from an optional value buffer (`None` = null),
    /// validating it against the type's layout.
    pub fn from_parts(buffer: Option<Buffer>) -> Result<Self, ScalarError> {
        Scalar::from_parts(BooleanType, buffer).map(Self)
    }
}

/// A UTF-8 string scalar of a parameter-free type.
macro_rules! string_scalar {
    ($(#[$doc:meta])* $name:ident, $ty:ty) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        #[cfg_attr(
            feature = "serde",
            derive(serde::Serialize, serde::Deserialize),
            serde(transparent)
        )]
        pub struct $name(Scalar<$ty>);

        scalar_common!([] [] $name, $ty);

        impl $name {
            /// The null scalar.
            pub fn null() -> Self {
                Self(Scalar::null(<$ty>::default()))
            }

            /// Builds the scalar from a string value over a fresh buffer.
            pub fn from_string(value: impl AsRef<str>) -> Self {
                Self(Scalar::from_string(<$ty>::default(), value))
            }

            /// Builds the scalar from an optional value buffer (`None` =
            /// null), validating it is UTF-8.
            pub fn from_parts(buffer: Option<Buffer>) -> Result<Self, ScalarError> {
                Scalar::from_parts(<$ty>::default(), buffer).map(Self)
            }
        }
    };
}

string_scalar!(
    /// A scalar of [`Utf8Type`].
    ///
    /// ```
    /// use yggdryl_scalar::Utf8Scalar;
    ///
    /// assert_eq!(Utf8Scalar::from_string("ygg").as_str(), Some("ygg"));
    /// ```
    Utf8Scalar, Utf8Type
);
string_scalar!(
    /// A scalar of [`LargeUtf8Type`].
    LargeUtf8Scalar, LargeUtf8Type
);

/// An opaque-bytes scalar of a parameter-free type.
macro_rules! binary_scalar {
    ($(#[$doc:meta])* $name:ident, $ty:ty) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        #[cfg_attr(
            feature = "serde",
            derive(serde::Serialize, serde::Deserialize),
            serde(transparent)
        )]
        pub struct $name(Scalar<$ty>);

        scalar_common!([] [] $name, $ty);

        impl $name {
            /// The null scalar.
            pub fn null() -> Self {
                Self(Scalar::null(<$ty>::default()))
            }

            /// Builds the scalar from a byte value over a fresh buffer.
            pub fn from_binary(value: impl AsRef<[u8]>) -> Result<Self, ScalarError> {
                Scalar::from_binary(<$ty>::default(), value).map(Self)
            }

            /// Builds the scalar from an optional value buffer (`None` =
            /// null).
            pub fn from_parts(buffer: Option<Buffer>) -> Result<Self, ScalarError> {
                Scalar::from_parts(<$ty>::default(), buffer).map(Self)
            }
        }
    };
}

binary_scalar!(
    /// A scalar of [`BinaryType`].
    BinaryScalar, BinaryType
);
binary_scalar!(
    /// A scalar of [`LargeBinaryType`].
    LargeBinaryScalar, LargeBinaryType
);

/// A scalar of [`FixedSizeBinaryType`]: the byte width is part of the type,
/// so the constructors take it first.
///
/// ```
/// use yggdryl_scalar::FixedSizeBinaryScalar;
/// use yggdryl_schema::FixedSizeBinaryType;
///
/// let uuid_type = FixedSizeBinaryType::from_parts(16).unwrap();
/// let uuid = FixedSizeBinaryScalar::from_binary(uuid_type, [7u8; 16]).unwrap();
/// assert_eq!(uuid.as_binary(), Some(&[7u8; 16][..]));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(transparent)
)]
pub struct FixedSizeBinaryScalar(Scalar<FixedSizeBinaryType>);

scalar_common!([] [] FixedSizeBinaryScalar, FixedSizeBinaryType);

impl FixedSizeBinaryScalar {
    /// The null scalar of the given type.
    pub fn null(data_type: FixedSizeBinaryType) -> Self {
        Self(Scalar::null(data_type))
    }

    /// Builds the scalar from a byte value over a fresh buffer, validating
    /// the width.
    pub fn from_binary(
        data_type: FixedSizeBinaryType,
        value: impl AsRef<[u8]>,
    ) -> Result<Self, ScalarError> {
        Scalar::from_binary(data_type, value).map(Self)
    }

    /// Builds the scalar from an optional value buffer (`None` = null),
    /// validating the width.
    pub fn from_parts(
        data_type: FixedSizeBinaryType,
        buffer: Option<Buffer>,
    ) -> Result<Self, ScalarError> {
        Scalar::from_parts(data_type, buffer).map(Self)
    }
}
