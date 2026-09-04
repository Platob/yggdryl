//! Compile-time datatype markers over the generic [`Field`] value.

use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::Deref;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::SmolStr;

use super::Field;
use crate::{DataType, Error, Result};

pub(crate) mod sealed {
    pub trait Sealed {}
}

/// A sealed compile-time marker for exactly one [`DataType`] variant.
///
/// Marker implementations validate the variant only. Parameters such as a
/// decimal precision, timestamp unit, or list child remain in the wrapped
/// [`Field`], so the typed view never duplicates schema state.
///
/// [`AnyType`] is the one marker that names no variant: it accepts every
/// datatype, which is what a value or field that has not been narrowed yet
/// carries.
pub trait FieldType: sealed::Sealed + Copy + Default + fmt::Debug + Send + Sync + 'static {
    /// The canonical, parameter-independent datatype name.
    const NAME: &'static str;

    /// Returns whether `dtype` has this marker's variant.
    fn matches(dtype: &DataType) -> bool;
}

/// The marker every datatype satisfies.
///
/// A marker usually narrows a value to one variant. This one narrows nothing,
/// so it is what an unnarrowed pairing such as [`crate::TypedScalar`] carries by
/// default: the datatype is still checked against the value, and the marker
/// simply has no opinion about which datatype that was.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AnyType;

impl sealed::Sealed for AnyType {}

impl FieldType for AnyType {
    const NAME: &'static str = "any";

    fn matches(_dtype: &DataType) -> bool {
        true
    }
}

/// An owned field whose datatype variant is checked at construction.
///
/// `TypedField<K>` contains exactly one [`Field`]; `K` is a zero-sized marker.
/// Immutable dereferencing exposes all generic field reads and projections.
/// There is deliberately no `DerefMut` or `as_field_mut`, because replacing
/// the datatype through an unchecked generic reference could violate `K`.
#[repr(transparent)]
pub struct TypedField<K: FieldType> {
    field: Field,
    marker: PhantomData<K>,
}

impl<K: FieldType> TypedField<K> {
    /// Checks and wraps an existing generic field without changing its state.
    pub fn try_from_field(field: Field) -> Result<Self> {
        field.validate()?;
        Self::from_validated_field(field)
    }

    fn from_validated_field(field: Field) -> Result<Self> {
        ensure_marker::<K>(field.dtype())?;
        Ok(Self {
            field,
            marker: PhantomData,
        })
    }

    /// Builds a typed field from a validated datatype of the marker's variant.
    ///
    /// Static aliases also expose a shorter infallible `new(name, nullable)`.
    pub fn try_new(name: impl Into<SmolStr>, dtype: DataType, nullable: bool) -> Result<Self> {
        Self::try_from_field(Field::new(name, dtype, nullable))
    }

    /// Builds a typed field from a datatype and complete metadata snapshot.
    pub fn try_from_parts<I, M, V>(
        name: impl Into<SmolStr>,
        dtype: DataType,
        nullable: bool,
        metadata: I,
    ) -> Result<Self>
    where
        I: IntoIterator<Item = (M, V)>,
        M: Into<String>,
        V: Into<String>,
    {
        Self::from_validated_field(Field::from_parts(name, dtype, nullable, metadata)?)
    }

    /// Borrows the generic field without allocating.
    pub const fn as_field(&self) -> &Field {
        &self.field
    }

    /// Borrows a checked typed reference without allocating.
    pub const fn as_typed_ref(&self) -> TypedFieldRef<'_, K> {
        TypedFieldRef {
            field: &self.field,
            marker: PhantomData,
        }
    }

    /// Consumes the marker wrapper and returns the exact generic field.
    pub fn into_field(self) -> Field {
        self.field
    }

    /// Changes the name while retaining the datatype marker.
    pub fn set_name(&mut self, name: impl Into<SmolStr>) {
        self.field.set_name(name);
    }

    /// Returns this typed field with a different name.
    pub fn with_name(mut self, name: impl Into<SmolStr>) -> Self {
        self.set_name(name);
        self
    }

    /// Changes nullability while retaining the datatype marker.
    pub fn set_nullable(&mut self, nullable: bool) {
        self.field.set_nullable(nullable);
    }

    /// Returns this typed field with different nullability.
    pub fn with_nullable(mut self, nullable: bool) -> Self {
        self.set_nullable(nullable);
        self
    }

    /// Replaces the datatype after validating both its parameters and marker.
    ///
    /// An error leaves this typed field unchanged.
    pub fn set_dtype(&mut self, dtype: DataType) -> Result<()> {
        ensure_marker::<K>(&dtype)?;
        self.field.set_dtype(dtype)
    }

    /// Returns this typed field with another datatype of the same variant.
    pub fn try_with_dtype(mut self, dtype: DataType) -> Result<Self> {
        self.set_dtype(dtype)?;
        Ok(self)
    }

    /// Inserts or replaces one metadata entry.
    pub fn insert_metadata(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Option<String>> {
        self.field.insert_metadata(key, value)
    }

    /// Replaces the complete metadata snapshot atomically.
    pub fn set_metadata<I, M, V>(&mut self, values: I) -> Result<()>
    where
        I: IntoIterator<Item = (M, V)>,
        M: Into<String>,
        V: Into<String>,
    {
        self.field.set_metadata(values)
    }

    /// Overlays validated metadata atomically.
    pub fn update_metadata<I, M, V>(&mut self, values: I) -> Result<()>
    where
        I: IntoIterator<Item = (M, V)>,
        M: Into<String>,
        V: Into<String>,
    {
        self.field.update_metadata(values)
    }

    /// Removes one metadata entry and returns its prior value.
    pub fn remove_metadata(&mut self, key: &str) -> Option<String> {
        self.field.remove_metadata(key)
    }

    /// Removes all metadata while retaining the datatype marker.
    pub fn clear_metadata(&mut self) {
        self.field.clear_metadata();
    }
}

impl TypedField<super::nested::Struct> {
    /// Consumes a checked Struct wrapper and returns its generic Struct field.
    ///
    /// This typed spelling is the Rust counterpart of the cached
    /// `into_struct_field` class accessor exposed by field-decorated dataclasses.
    /// The returned value is still the one canonical [`Field`]; the marker has
    /// already proved that its datatype is Struct.
    pub fn into_struct_field(self) -> Field {
        self.field
    }
}

/// A borrowed, allocation-free proof that a [`Field`] has datatype marker `K`.
#[repr(transparent)]
pub struct TypedFieldRef<'field, K: FieldType> {
    field: &'field Field,
    marker: PhantomData<K>,
}

impl<'field, K: FieldType> TypedFieldRef<'field, K> {
    /// Checks and borrows a generic field without cloning it.
    pub fn try_from_field(field: &'field Field) -> Result<Self> {
        field.validate()?;
        ensure_marker::<K>(field.dtype())?;
        Ok(Self {
            field,
            marker: PhantomData,
        })
    }

    /// Returns the checked generic field reference.
    pub const fn as_field(self) -> &'field Field {
        self.field
    }
}

fn ensure_marker<K: FieldType>(dtype: &DataType) -> Result<()> {
    if K::matches(dtype) {
        Ok(())
    } else {
        Err(Error::InvalidDataType {
            kind: "TypedField",
            reason: format!(
                "marker {} requires datatype {}, got {}",
                std::any::type_name::<K>(),
                K::NAME,
                dtype.name()
            )
            .into(),
        })
    }
}

impl<K: FieldType> Clone for TypedField<K> {
    fn clone(&self) -> Self {
        Self {
            field: self.field.clone(),
            marker: PhantomData,
        }
    }
}

impl<K: FieldType> fmt::Debug for TypedField<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("TypedField")
            .field(&K::NAME)
            .field(&self.field)
            .finish()
    }
}

impl<K: FieldType> fmt::Display for TypedField<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.field.fmt(formatter)
    }
}

impl<K: FieldType> PartialEq for TypedField<K> {
    fn eq(&self, other: &Self) -> bool {
        self.field == other.field
    }
}

impl<K: FieldType> Eq for TypedField<K> {}

impl<K: FieldType> PartialOrd for TypedField<K> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: FieldType> Ord for TypedField<K> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.field.cmp(&other.field)
    }
}

impl<K: FieldType> Hash for TypedField<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.field.hash(state);
    }
}

impl<K: FieldType> Serialize for TypedField<K> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.field.serialize(serializer)
    }
}

impl<'de, K: FieldType> Deserialize<'de> for TypedField<K> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let field = Field::deserialize(deserializer)?;
        Self::from_validated_field(field).map_err(serde::de::Error::custom)
    }
}

impl<K: FieldType> Deref for TypedField<K> {
    type Target = Field;

    fn deref(&self) -> &Self::Target {
        &self.field
    }
}

impl<K: FieldType> AsRef<Field> for TypedField<K> {
    fn as_ref(&self) -> &Field {
        &self.field
    }
}

impl<K: FieldType> Borrow<Field> for TypedField<K> {
    fn borrow(&self) -> &Field {
        &self.field
    }
}

impl<K: FieldType> TryFrom<Field> for TypedField<K> {
    type Error = Error;

    fn try_from(field: Field) -> Result<Self> {
        Self::try_from_field(field)
    }
}

impl<K: FieldType> From<TypedField<K>> for Field {
    fn from(field: TypedField<K>) -> Self {
        field.into_field()
    }
}

impl<'field, K: FieldType> Copy for TypedFieldRef<'field, K> {}

impl<'field, K: FieldType> Clone for TypedFieldRef<'field, K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: FieldType> fmt::Debug for TypedFieldRef<'_, K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("TypedFieldRef")
            .field(&K::NAME)
            .field(&self.field)
            .finish()
    }
}

impl<K: FieldType> PartialEq for TypedFieldRef<'_, K> {
    fn eq(&self, other: &Self) -> bool {
        self.field == other.field
    }
}

impl<K: FieldType> Eq for TypedFieldRef<'_, K> {}

impl<K: FieldType> PartialOrd for TypedFieldRef<'_, K> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: FieldType> Ord for TypedFieldRef<'_, K> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.field.cmp(other.field)
    }
}

impl<K: FieldType> Hash for TypedFieldRef<'_, K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.field.hash(state);
    }
}

impl<K: FieldType> Deref for TypedFieldRef<'_, K> {
    type Target = Field;

    fn deref(&self) -> &Self::Target {
        self.field
    }
}

impl<K: FieldType> AsRef<Field> for TypedFieldRef<'_, K> {
    fn as_ref(&self) -> &Field {
        self.field
    }
}

impl<K: FieldType> Borrow<Field> for TypedFieldRef<'_, K> {
    fn borrow(&self) -> &Field {
        self.field
    }
}

macro_rules! define_field_types {
    ($(#[$meta:meta])* $marker:ident, $name:literal, $pattern:pat $(,)?) => {
        $(#[$meta])*
        #[doc = concat!("Compile-time marker for `", $name, "` fields.")]
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $marker;

        impl super::typed::sealed::Sealed for $marker {}

        impl super::typed::FieldType for $marker {
            const NAME: &'static str = $name;

            fn matches(dtype: &crate::DataType) -> bool {
                matches!(dtype, $pattern)
            }
        }
    };
}

pub(crate) use define_field_types;

macro_rules! static_field_constructor {
    ($marker:path, $dtype:expr) => {
        impl TypedField<$marker> {
            /// Constructs this statically known datatype without parsing or allocation.
            pub fn new(name: impl Into<SmolStr>, nullable: bool) -> Self {
                Self {
                    field: Field::new(name, $dtype, nullable),
                    marker: PhantomData,
                }
            }

            /// Constructs this statically known datatype with complete metadata.
            pub fn from_parts<I, M, V>(
                name: impl Into<SmolStr>,
                nullable: bool,
                metadata: I,
            ) -> Result<Self>
            where
                I: IntoIterator<Item = (M, V)>,
                M: Into<String>,
                V: Into<String>,
            {
                Self::from_validated_field(Field::from_parts(name, $dtype, nullable, metadata)?)
            }
        }
    };
}

static_field_constructor!(super::scalar::Null, DataType::Null);
static_field_constructor!(super::scalar::Boolean, DataType::Boolean);
static_field_constructor!(super::integer::Int8, DataType::Int8);
static_field_constructor!(super::integer::Int16, DataType::Int16);
static_field_constructor!(super::integer::Int32, DataType::Int32);
static_field_constructor!(super::integer::Int64, DataType::Int64);
static_field_constructor!(super::integer::UInt8, DataType::UInt8);
static_field_constructor!(super::integer::UInt16, DataType::UInt16);
static_field_constructor!(super::integer::UInt32, DataType::UInt32);
static_field_constructor!(super::integer::UInt64, DataType::UInt64);
static_field_constructor!(super::floating::Float16, DataType::Float16);
static_field_constructor!(super::floating::Float32, DataType::Float32);
static_field_constructor!(super::floating::Float64, DataType::Float64);
static_field_constructor!(super::temporal::Date32, DataType::Date32);
static_field_constructor!(super::temporal::Date64, DataType::Date64);
static_field_constructor!(super::binary::Binary, DataType::Binary);
static_field_constructor!(super::binary::LargeBinary, DataType::LargeBinary);
static_field_constructor!(super::binary::BinaryView, DataType::BinaryView);
static_field_constructor!(super::binary::Utf8, DataType::Utf8);
static_field_constructor!(super::binary::LargeUtf8, DataType::LargeUtf8);
static_field_constructor!(super::binary::Utf8View, DataType::Utf8View);
static_field_constructor!(super::ascii::Ascii16, DataType::Ascii16);
static_field_constructor!(super::ascii::Ascii24, DataType::Ascii24);
static_field_constructor!(super::ascii::Ascii32, DataType::Ascii32);
static_field_constructor!(super::ascii::Ascii64, DataType::Ascii64);
static_field_constructor!(super::ascii::Ascii96, DataType::Ascii96);
static_field_constructor!(super::ascii::Ascii128, DataType::Ascii128);
static_field_constructor!(super::ascii::Country, DataType::Country);
static_field_constructor!(super::ascii::Currency, DataType::Currency);
static_field_constructor!(super::ascii::Mic, DataType::Mic);
static_field_constructor!(super::ascii::Cfi, DataType::Cfi);
static_field_constructor!(super::nested::Variant, DataType::Variant);
static_field_constructor!(super::nested::Guid, DataType::Guid);
