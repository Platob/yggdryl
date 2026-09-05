//! Compile-time datatype markers over the generic [`Field`] value.

use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::Deref;

use serde::de::Error as _;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::SmolStr;

use super::Field;
use crate::{DataType, Error, Result, Scalar};

pub(crate) mod sealed {
    pub trait Sealed {}
}

/// A sealed compile-time marker for exactly one [`DataType`] variant.
///
/// Marker implementations validate the variant only. Parameters such as a
/// decimal precision, datetime unit, or list child remain in the wrapped
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

impl TypedField<super::nested::StructType> {
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

impl Field {
    /// Checks this field's datatype and returns an allocation-free typed view.
    pub fn try_as_typed<K: FieldType>(&self) -> Result<TypedFieldRef<'_, K>> {
        TypedFieldRef::try_from_field(self)
    }

    /// Checks this field's datatype and consumes it into a typed field.
    pub fn try_into_typed<K: FieldType>(self) -> Result<TypedField<K>> {
        TypedField::try_from_field(self)
    }
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

/// A datatype and one value it accepts.
///
/// The value is validated against the datatype on construction, through the
/// same walk a column value takes, so a pairing that exists is a pairing that
/// holds. A null is accepted by every datatype, because a null is what a
/// nullable column stores.
///
/// `K` is a zero-sized [`crate::FieldType`] marker naming the datatype variant
/// this pairing is allowed to hold. It defaults to [`AnyType`], which allows
/// every variant, so `TypedScalar` with no marker is the dynamic pairing and
/// `TypedScalar<K>` is the narrowed one. The marker adds no storage: a narrowed
/// pairing is the same two words a dynamic one is.
///
/// Pairings order first by datatype and then by value, matching their exact
/// equality and hashing identity.
pub struct TypedScalar<K: FieldType = AnyType> {
    dtype: DataType,
    value: Scalar,
    marker: PhantomData<K>,
}

impl<K: FieldType> TypedScalar<K> {
    /// Pair a datatype with a value it accepts, checking the marker too.
    ///
    /// # Errors
    ///
    /// Returns an error when the datatype is not this marker's variant, or
    /// when the value is neither null nor a value the datatype accepts.
    pub fn try_from_parts(dtype: DataType, value: Scalar) -> Result<Self> {
        ensure_scalar_marker::<K>(&dtype)?;
        Self::from_checked_parts(dtype, value)
    }

    /// Pair a value with the datatype it already names, checking the marker.
    ///
    /// # Errors
    ///
    /// Returns an error when the value names no single datatype, which is what
    /// [`Scalar::dtype`] reports, or when that datatype is not this
    /// marker's variant.
    pub fn try_from_value(value: Scalar) -> Result<Self> {
        let dtype = value.dtype()?;
        ensure_scalar_marker::<K>(&dtype)?;
        Ok(Self {
            dtype,
            value,
            marker: PhantomData,
        })
    }

    /// The datatype this value belongs to.
    pub const fn dtype(&self) -> &DataType {
        &self.dtype
    }

    /// The value itself.
    pub const fn value(&self) -> &Scalar {
        &self.value
    }

    /// Return whether the value is null.
    ///
    /// This is [`Scalar::is_null`] on the value inside, which is how a caller
    /// asks whether the pairing holds a value or records its absence for the
    /// datatype beside it.
    pub const fn is_null(&self) -> bool {
        self.value.is_null()
    }

    /// Return a deterministic hash of the datatype/value pair.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    /// Consume this pairing and return both halves.
    pub fn into_parts(self) -> (DataType, Scalar) {
        (self.dtype, self.value)
    }

    /// Consume this pairing and return the value alone.
    pub fn into_value(self) -> Scalar {
        self.value
    }

    /// Widen this pairing to the marker every datatype satisfies.
    ///
    /// Nothing is checked and nothing is copied: the marker is zero-sized, so
    /// this only forgets which variant the type system was tracking.
    pub fn into_any(self) -> TypedScalar {
        TypedScalar {
            dtype: self.dtype,
            value: self.value,
            marker: PhantomData,
        }
    }

    /// Narrow this pairing to another datatype marker.
    ///
    /// # Errors
    ///
    /// Returns an error naming both markers when the datatype is not the
    /// requested variant.
    pub fn try_into_typed<J: FieldType>(self) -> Result<TypedScalar<J>> {
        ensure_scalar_marker::<J>(&self.dtype)?;
        Ok(TypedScalar {
            dtype: self.dtype,
            value: self.value,
            marker: PhantomData,
        })
    }

    /// Build the pairing without re-checking the marker.
    pub(crate) fn from_checked_parts(dtype: DataType, value: Scalar) -> Result<Self> {
        crate::types::validate_dtype_value_for(&dtype, &value)?;
        Ok(Self {
            dtype,
            value,
            marker: PhantomData,
        })
    }
}

impl TypedScalar {
    /// Pair a datatype with a value it accepts.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is neither null nor a value the
    /// datatype accepts.
    pub fn from_parts(dtype: DataType, value: Scalar) -> Result<Self> {
        Self::from_checked_parts(dtype, value)
    }

    /// Pair a value with the datatype it already names.
    ///
    /// # Errors
    ///
    /// Returns an error when the value names no single datatype, which is what
    /// [`Scalar::dtype`] reports.
    pub fn from_value(value: Scalar) -> Result<Self> {
        Ok(Self {
            dtype: value.dtype()?,
            value,
            marker: PhantomData,
        })
    }
}

#[cfg(feature = "arrow")]
impl<K: FieldType> TypedScalar<K> {
    /// Materialize this pairing as an exact one-row Arrow array.
    ///
    /// The value projects through a synthetic non-nullable Field over
    /// [`Self::dtype`], so a null materializes only when it is the
    /// datatype's own canonical default - [`crate::DataType::Null`] and
    /// transparent logical wrappers with a null-only default. A null under any
    /// other datatype is a property of the column beside it, which is what
    /// [`crate::arrow::scalar_array`] with a nullable [`crate::Field`] spells.
    ///
    /// ```
    /// use yggdryl::{DataType, TypedScalar, Scalar};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let typed = TypedScalar::from_parts(DataType::Int64, Scalar::from(7_i64))?;
    /// let array = typed.into_arrow_array()?;
    /// assert_eq!(array.len(), 1);
    /// assert_eq!(array.data_type(), &arrow_schema::DataType::Int64);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the physical Arrow layout cannot represent the
    /// value, or when the value is a null the datatype's canonical default
    /// does not spell.
    pub fn into_arrow_array(self) -> crate::arrow::Result<arrow_array::ArrayRef> {
        let field = crate::Field::new("value", self.dtype.clone(), false);
        match crate::arrow::validate_scalar_value(&field, self.value.clone()) {
            Ok(value) => crate::arrow::value::array_from_values(&field, &[&value]),
            Err(error) => {
                // The same narrow exception the foreign-array import makes:
                // a datatype whose canonical default is logically null - Null
                // itself, null-only dictionaries, unions, run-end encodings -
                // stays projectable even though the synthetic Field is
                // non-nullable.
                if self.dtype.is_default_value(&self.value)? {
                    crate::arrow::value::array_from_values(&field, &[&self.value])
                } else {
                    Err(error)
                }
            }
        }
    }

    /// Decode row 0 of a one-row Arrow array, checking the marker too.
    ///
    /// # Errors
    ///
    /// Returns an error when the datatype is not this marker's variant, when
    /// the array does not hold exactly one row of the datatype's exact
    /// physical layout, or when the decoded value is not one the datatype
    /// accepts.
    pub fn try_from_arrow_array(
        dtype: DataType,
        array: &dyn arrow_array::Array,
    ) -> crate::arrow::Result<Self> {
        ensure_scalar_marker::<K>(&dtype)?;
        Self::decoded_from_arrow_array(dtype, array)
    }

    /// Decode a validated one-row array without re-checking the marker.
    fn decoded_from_arrow_array(
        dtype: DataType,
        array: &dyn arrow_array::Array,
    ) -> crate::arrow::Result<Self> {
        // A null is accepted by every datatype here, so the synthetic Field is
        // nullable; the exact-datatype, length, and bounded-shape checks still
        // run inside the shared scalar decoder.
        let field = crate::Field::new("value", dtype.clone(), true);
        let value = crate::arrow::scalar_value(&field, array)?;
        // The Arrow reading may spell a value physically - a float16 reads
        // back as its narrow float - so canonicalize through the same walk a
        // column value takes before the pairing holds it.
        let value = crate::arrow::validate_scalar_value(&field, value)?;
        Self::from_checked_parts(dtype, value).map_err(crate::arrow::Error::from)
    }
}

#[cfg(feature = "arrow")]
impl TypedScalar {
    /// Decode row 0 of a one-row Arrow array as a dynamic pairing.
    ///
    /// ```
    /// use yggdryl::{DataType, TypedScalar, Scalar};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let array = TypedScalar::from_parts(DataType::Int64, Scalar::from(7_i64))?.into_arrow_array()?;
    /// let typed = TypedScalar::from_arrow_array(DataType::Int64, array.as_ref())?;
    /// assert_eq!(typed.value(), &Scalar::from(7));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the array does not hold exactly one row of the
    /// datatype's exact physical layout, or when the decoded value is not one
    /// the datatype accepts.
    pub fn from_arrow_array(
        dtype: DataType,
        array: &dyn arrow_array::Array,
    ) -> crate::arrow::Result<Self> {
        Self::decoded_from_arrow_array(dtype, array)
    }
}

/// Report a datatype that is not the marker's variant.
fn ensure_scalar_marker<K: FieldType>(dtype: &DataType) -> Result<()> {
    if K::matches(dtype) {
        Ok(())
    } else {
        Err(Error::InvalidDataType {
            kind: "TypedScalar",
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

impl<K: FieldType> Clone for TypedScalar<K> {
    fn clone(&self) -> Self {
        Self {
            dtype: self.dtype.clone(),
            value: self.value.clone(),
            marker: PhantomData,
        }
    }
}

impl<K: FieldType> fmt::Debug for TypedScalar<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TypedScalar")
            .field("dtype", &self.dtype)
            .field("value", &self.value)
            .finish()
    }
}

impl<K: FieldType> PartialEq for TypedScalar<K> {
    fn eq(&self, other: &Self) -> bool {
        self.dtype == other.dtype && self.value == other.value
    }
}

impl<K: FieldType> Eq for TypedScalar<K> {}

impl<K: FieldType> PartialOrd for TypedScalar<K> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: FieldType> Ord for TypedScalar<K> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.dtype
            .cmp(&other.dtype)
            .then_with(|| self.value.cmp(&other.value))
    }
}

impl<K: FieldType> Hash for TypedScalar<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.dtype.hash(state);
        self.value.hash(state);
    }
}

impl<K: FieldType> Serialize for TypedScalar<K> {
    /// Write the two halves, and never the marker: a marker is a compile-time
    /// fact about which variant may appear, not data the pairing carries.
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut structure = serializer.serialize_struct("TypedScalar", 2)?;
        structure.serialize_field("dtype", &self.dtype)?;
        structure.serialize_field("value", &self.value)?;
        structure.end()
    }
}

impl<'de, K: FieldType> Deserialize<'de> for TypedScalar<K> {
    /// Read a pairing back through the constructor that validates one.
    ///
    /// Deriving this would accept a datatype and a value that never agreed,
    /// which is exactly the state [`TypedScalar::try_from_parts`] exists to
    /// refuse.
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // This mirror must stay field-for-field identical to `TypedScalar`.
        #[derive(Deserialize)]
        struct StructuralTypedScalar {
            dtype: DataType,
            value: Scalar,
        }

        let structural = StructuralTypedScalar::deserialize(deserializer)?;
        Self::try_from_parts(structural.dtype, structural.value).map_err(D::Error::custom)
    }
}

impl<K: FieldType> TryFrom<Scalar> for TypedScalar<K> {
    type Error = Error;

    fn try_from(value: Scalar) -> Result<Self> {
        Self::try_from_value(value)
    }
}

impl<K: FieldType> From<TypedScalar<K>> for Scalar {
    fn from(typed: TypedScalar<K>) -> Self {
        typed.into_value()
    }
}

/// Define one datatype's typed-scalar alias and, when possible, constructor.
macro_rules! define_scalar_type {
    ($alias:ident, $marker:path, $name:literal) => {
        #[doc = concat!("A `", $name, "`-typed value paired with its datatype.")]
        pub type $alias = $crate::TypedScalar<$marker>;
    };
    ($alias:ident, $marker:path, $name:literal, $dtype:expr) => {
        define_scalar_type!($alias, $marker, $name);

        impl $crate::TypedScalar<$marker> {
            /// Pairs this statically known datatype with a value it accepts.
            ///
            /// # Errors
            ///
            /// Returns an error when the value is neither null nor accepted.
            pub fn new(value: $crate::Scalar) -> $crate::Result<Self> {
                Self::from_checked_parts($dtype, value)
            }
        }
    };
}

pub(crate) use define_scalar_type;

macro_rules! define_field_types {
    ($(#[$meta:meta])* $marker:ident, $name:literal, $pattern:pat $(,)?) => {
        $(#[$meta])*
        #[doc = concat!("Compile-time marker for `", $name, "` fields.")]
        #[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $marker;

        impl $crate::types::typed::sealed::Sealed for $marker {}

        impl $crate::types::typed::FieldType for $marker {
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

static_field_constructor!(super::boolean::NullType, DataType::Null);
static_field_constructor!(super::boolean::BooleanType, DataType::Boolean);
static_field_constructor!(super::integer::Int8Type, DataType::Int8);
static_field_constructor!(super::integer::Int16Type, DataType::Int16);
static_field_constructor!(super::integer::Int32Type, DataType::Int32);
static_field_constructor!(super::integer::Int64Type, DataType::Int64);
static_field_constructor!(super::integer::UInt8Type, DataType::UInt8);
static_field_constructor!(super::integer::UInt16Type, DataType::UInt16);
static_field_constructor!(super::integer::UInt32Type, DataType::UInt32);
static_field_constructor!(super::integer::UInt64Type, DataType::UInt64);
static_field_constructor!(super::floating::Float16Type, DataType::Float16);
static_field_constructor!(super::floating::Float32Type, DataType::Float32);
static_field_constructor!(super::floating::Float64Type, DataType::Float64);
static_field_constructor!(super::temporal::Date32Type, DataType::Date32);
static_field_constructor!(super::temporal::Date64Type, DataType::Date64);
static_field_constructor!(super::bytes::BinaryType, DataType::Binary);
static_field_constructor!(super::bytes::LargeBinaryType, DataType::LargeBinary);
static_field_constructor!(super::bytes::BinaryViewType, DataType::BinaryView);
static_field_constructor!(super::text::Utf8Type, DataType::Utf8);
static_field_constructor!(super::text::LargeUtf8Type, DataType::LargeUtf8);
static_field_constructor!(super::text::Utf8ViewType, DataType::Utf8View);
static_field_constructor!(super::ascii::AsciiType, DataType::Ascii);
static_field_constructor!(super::ascii::CountryType, DataType::Country);
static_field_constructor!(super::ascii::CurrencyType, DataType::Currency);
static_field_constructor!(super::ascii::MicType, DataType::Mic);
static_field_constructor!(super::ascii::CfiType, DataType::Cfi);
static_field_constructor!(super::nested::VariantType, DataType::Variant);
static_field_constructor!(super::uuid::UuidType, DataType::Uuid);
static_field_constructor!(super::version::VersionType, DataType::Version);

#[cfg(test)]
mod tests;
