//! Typed views over the generic [`Field`] and [`Scalar`] values.
//!
//! Two kinds of proof live here. A compile-time marker ([`FieldType`]) narrows
//! a [`Field`] to one datatype variant without copying it: [`TypedField`]
//! owns the field, [`TypedFieldRef`] borrows it. A runtime pairing
//! ([`TypedScalar`], [`TypedRecord`]) borrows a field and carries the value
//! that field's own contract answered for it, so a reader downstream takes the
//! datatype, the name and the value from one place and re-derives none of
//! them. [`UncheckedTypedScalar`] is the pairing before that proof, for a
//! reader that wants the field's reading of a wire value without committing
//! to it.

use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::ops::Deref;

use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::{SmolStr, format_smolstr};

use super::Field;
use crate::{DataType, Error, Result, Scalar};

mod record;
mod shared;

pub use record::TypedRecord;

pub(crate) mod sealed {
    pub trait Sealed {}
}

/// A sealed compile-time marker for exactly one [`DataType`] variant.
///
/// Marker implementations validate the variant only. Parameters such as a
/// decimal precision, datetime unit, or list child remain in the wrapped
/// [`Field`], so the typed view never duplicates schema state.
pub trait FieldType: sealed::Sealed + Copy + Default + fmt::Debug + Send + Sync + 'static {
    /// The canonical, parameter-independent datatype name.
    const NAME: &'static str;

    /// Returns whether `dtype` has this marker's variant.
    fn matches(dtype: &DataType) -> bool;
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
    /// This typed spelling is the Rust counterpart of the cached struct-root
    /// accessor the bindings install on a field class: `into_field()` on a
    /// Python `@scalar` dataclass, the `intoStructField` getter in JavaScript.
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

/// One [`Field`] and the value it holds, proven together.
///
/// The value is exactly what [`Field::scalar`] answers for it: checked
/// against the datatype, rewritten into the representation the datatype
/// declares, and refused as null where the field is not nullable. The pairing
/// borrows the field rather than copying its datatype, so a reader downstream
/// takes the datatype, the name, and the value from one place - and a
/// borrowed view over a value already canonical allocates nothing, which is
/// what lets a row be typed cell by cell without a schema copy per cell.
///
/// Equality, ordering and hashing read the datatype and the value, never the
/// field's name, nullability, or metadata: two pairings of one value under
/// two columns of the same datatype are the same typed value, exactly as a
/// bare [`Scalar`] is one value whichever column stored it. The same rule
/// holds for [`TypedRecord`].
///
/// ```
/// use yggdryl::{DataType, Field, Scalar, TypedScalar};
///
/// # fn main() -> yggdryl::Result<()> {
/// let price = Field::new("price", DataType::Int32, false);
/// let typed = TypedScalar::new(&price, 7_i64)?;
/// assert_eq!(typed.name(), "price");
/// assert_eq!(typed.dtype(), &DataType::Int32);
/// // The value was narrowed to the width the field declares.
/// assert_eq!(typed.value(), &Scalar::from(7_i32));
/// assert_eq!(typed.as_i64(), Some(7));
/// // Nullability is the field's rule, so a required column refuses a null.
/// assert!(TypedScalar::new(&price, Scalar::Null).is_err());
/// assert!(TypedScalar::new(&price, "seven").is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct TypedScalar<'a> {
    field: &'a Field,
    value: Scalar,
}

impl<'a> TypedScalar<'a> {
    /// Pair a field with a value it accepts.
    ///
    /// This is [`Field::scalar`], and nothing else: the pairing exists exactly
    /// when the field's own contract answered a value.
    ///
    /// # Errors
    ///
    /// Returns an error naming the field when the value is not one its
    /// datatype accepts, or is null under a field that is not nullable.
    pub fn new(field: &'a Field, value: impl Into<Scalar>) -> Result<Self> {
        Ok(Self {
            field,
            value: field.scalar(value)?,
        })
    }

    /// Read natural text under a field, then pair the value it spells.
    ///
    /// This is the one door from text to a typed value: the bytes a document
    /// spells base64 are substituted, and then the reading is
    /// [`Field::scalar`], which accepts every spelling a document leaves
    /// behind - a number, a date, a code - and answers the exact value.
    ///
    /// ```
    /// use yggdryl::{DataType, Field, Scalar, TypedScalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let size = Field::new("size", DataType::Int64, false);
    /// assert_eq!(TypedScalar::parse_str(&size, "42")?.value(), &Scalar::from(42_i64));
    /// assert!(TypedScalar::parse_str(&size, "forty-two").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the field when the text spells no value the
    /// field accepts.
    pub fn parse_str(field: &'a Field, text: &str) -> Result<Self> {
        let value = crate::text::prepare_text(Scalar::from(text), field)?;
        Ok(Self::from_checked(field, value))
    }

    /// Pair a value with the shared field of the datatype it already names.
    ///
    /// [`Scalar::dtype`] reads what the value is, and
    /// [`DataType::shared_field`] answers the one nullable `value` field the
    /// crate prebuilt for that datatype, so a leaf value becomes a typed value
    /// without building a field - the pairing borrows a field that lives for
    /// the whole program. A datatype with no shared field, such as a struct
    /// or a list, is paired through [`Self::new`] under a field of the
    /// caller's own.
    ///
    /// ```
    /// use yggdryl::{DataType, Scalar, TypedScalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let typed = TypedScalar::infer(Scalar::from(7_i64))?;
    /// assert_eq!(typed.dtype(), &DataType::Int64);
    /// assert_eq!(typed.name(), "value");
    /// assert!(typed.field().is_nullable());
    ///
    /// let row = Scalar::from_sequence([Scalar::from(1_i64)]);
    /// let refused = TypedScalar::infer(row).unwrap_err().to_string();
    /// assert!(refused.contains("list"), "{refused}");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the value names no single datatype, which is
    /// what [`Scalar::dtype`] reports, or names one the crate prebuilds no
    /// shared field for, in which case the error names that datatype.
    pub fn infer(value: Scalar) -> Result<TypedScalar<'static>> {
        let dtype = value.dtype()?;
        let Some(field) = dtype.shared_field() else {
            return Err(Error::InvalidDataType {
                kind: "TypedScalar",
                reason: format_smolstr!(
                    "no shared field is prebuilt for datatype {dtype}; pair the value under a \
                     field of your own through TypedScalar::new"
                ),
            });
        };
        TypedScalar::new(field, value)
    }

    /// Pair a field with a value its contract already answered.
    ///
    /// Row canonicalization proves every cell of a row in one walk, and this
    /// is how those cells become pairings without a second walk each.
    pub(crate) const fn from_checked(field: &'a Field, value: Scalar) -> Self {
        Self { field, value }
    }

    /// The field this value belongs to.
    pub const fn field(&self) -> &'a Field {
        self.field
    }

    /// The datatype this value belongs to.
    pub const fn dtype(&self) -> &'a DataType {
        self.field.dtype()
    }

    /// The name of the field this value belongs to.
    pub fn name(&self) -> &'a str {
        self.field.name()
    }

    /// The value itself.
    pub const fn value(&self) -> &Scalar {
        &self.value
    }

    /// Return whether the value is null.
    ///
    /// A null here is what a nullable column stores for an absent value; a
    /// field that is not nullable never pairs with one.
    pub const fn is_null(&self) -> bool {
        self.value.is_null()
    }

    /// Return a string slice when the value is text, ASCII, or an enum member.
    pub fn as_str(&self) -> Option<&str> {
        self.value.as_str()
    }

    /// Return the payload when the value is bytes.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        self.value.as_bytes()
    }

    /// Return a boolean when the value is one.
    pub const fn as_bool(&self) -> Option<bool> {
        self.value.as_bool()
    }

    /// Read the value as an `i64`, when it is an integer that fits.
    pub const fn as_i64(&self) -> Option<i64> {
        self.value.as_i64()
    }

    /// Read the value as a `u64`, when it is an integer that fits.
    pub const fn as_u64(&self) -> Option<u64> {
        self.value.as_u64()
    }

    /// Read the value as an `i128`, when it is a signed integer that fits.
    pub const fn as_i128(&self) -> Option<i128> {
        self.value.as_i128()
    }

    /// Read the value as an `f64`, when it is a float of any width.
    pub fn as_f64(&self) -> Option<f64> {
        self.value.as_f64()
    }

    /// Return the sign and magnitude of an integer of any width.
    pub const fn as_integer(&self) -> Option<super::Integer> {
        self.value.as_integer()
    }

    /// Return a float of any width.
    pub const fn as_float(&self) -> Option<super::Floating> {
        self.value.as_float()
    }

    /// Return the unscaled coefficient and scale of a decimal of any width.
    pub fn as_decimal(&self) -> Option<(crate::I256, i8)> {
        self.value.as_decimal()
    }

    /// Return a temporal of any family.
    pub const fn as_temporal(&self) -> Option<&super::Temporal> {
        self.value.as_temporal()
    }

    /// Return sequence children when the value is a sequence.
    pub fn as_sequence(&self) -> Option<&[Scalar]> {
        self.value.as_sequence()
    }

    /// Return record fields in name order when the value is a record.
    pub fn as_record(&self) -> Option<&std::collections::BTreeMap<SmolStr, Scalar>> {
        self.value.as_record()
    }

    /// Look up a sequence index.
    pub fn get(&self, index: usize) -> Option<&Scalar> {
        self.value.get(index)
    }

    /// Look up a record field or a text mapping key.
    pub fn get_key_str(&self, key: &str) -> Option<&Scalar> {
        self.value.get_key_str(key)
    }

    /// Return a deterministic hash of the value.
    ///
    /// The field is proof, not content: the hash is the value's own canonical
    /// feed, so a typed value and the bare [`Scalar`] inside it hash alike,
    /// and so does the same value under another field of the same datatype.
    pub fn stable_hash(&self) -> u64 {
        self.value.stable_hash()
    }

    /// Consume this pairing and return the value alone.
    pub fn into_value(self) -> Scalar {
        self.value
    }

    /// Consume this pairing and return both halves.
    pub fn into_parts(self) -> (&'a Field, Scalar) {
        (self.field, self.value)
    }

    /// Consume this pairing and return the value's canonical text.
    ///
    /// The spelling is the one a text column stores for the value - the
    /// canonical form each family prints, a temporal spelled the classic way,
    /// bytes as their characters, a geometry as WKT - and it is what
    /// [`Display`](fmt::Display) writes. A value that spells no text of its
    /// own, such as a row or an interval, writes its family's own form.
    pub fn into_str(self) -> SmolStr {
        match crate::types::text::text_from_value(&self.value) {
            Some(Ok(text)) => text,
            _ => format_smolstr!("{self}"),
        }
    }
}

#[cfg(feature = "arrow")]
impl<'a> TypedScalar<'a> {
    /// Decode row 0 of a one-row Arrow array under the field.
    ///
    /// [`crate::arrow::scalar_value`] reads the array - exactly one row, the
    /// field's exact physical layout - and the reading is then paired the
    /// way every value is, through [`Field::scalar`]: an Arrow reading spells
    /// a value physically, a float16 as its narrow float, and the pairing
    /// holds the canonical one.
    ///
    /// ```
    /// use yggdryl::{DataType, Field, Scalar, TypedScalar};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("size", DataType::Int64, false);
    /// let array = TypedScalar::new(&field, 7_i64)?.into_arrow_array()?;
    /// let typed = TypedScalar::from_arrow_array(&field, array.as_ref())?;
    /// assert_eq!(typed.value(), &Scalar::from(7));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the array does not hold exactly one row of the
    /// field's exact physical layout, or when the decoded value is not one
    /// the field accepts.
    pub fn from_arrow_array(
        field: &'a Field,
        array: &dyn arrow_array::Array,
    ) -> crate::arrow::Result<Self> {
        let value = crate::arrow::scalar_value(field, array)?;
        Self::new(field, value).map_err(crate::arrow::Error::from)
    }

    /// Materialize this pairing as an exact one-row Arrow array.
    ///
    /// The field decides nullability, dictionary options, and extension
    /// identity, exactly as [`crate::arrow::scalar_array`] reads them; the
    /// pairing is that function's validated half, so the array is built
    /// without a second walk over the value.
    ///
    /// ```
    /// use yggdryl::{DataType, Field, TypedScalar};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let field = Field::new("size", DataType::Int64, true);
    /// let array = TypedScalar::new(&field, 7_i64)?.into_arrow_array()?;
    /// assert_eq!(array.len(), 1);
    /// assert_eq!(array.data_type(), &arrow_schema::DataType::Int64);
    /// // A null is what the nullable column stores, so it projects too.
    /// assert!(TypedScalar::new(&field, yggdryl::Scalar::Null)?.into_arrow_array()?.is_null(0));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the physical Arrow layout cannot represent the
    /// value.
    pub fn into_arrow_array(self) -> crate::arrow::Result<arrow_array::ArrayRef> {
        crate::arrow::value::array_from_values(self.field, &[&self.value])
    }
}

/// Write a value's canonical text, or its family's own form when it has none.
///
/// `text_from_value` owns the spelling, and it declines exactly four shapes:
/// a null, a nested value, a temporal without a classic spelling, and a bytes
/// or geometry payload it read and refused. Only those reach the arms below,
/// each writing what its family prints - the hex of a payload, an interval's
/// components, a row as its sequence.
fn write_value(formatter: &mut fmt::Formatter<'_>, value: &Scalar) -> fmt::Result {
    match crate::types::text::text_from_value(value) {
        Some(Ok(text)) => formatter.write_str(&text),
        Some(Err(_)) | None => match value {
            Scalar::Null => formatter.write_str("null"),
            Scalar::Temporal(held) => fmt::Display::fmt(held, formatter),
            Scalar::Bytes(held) => fmt::Display::fmt(held, formatter),
            Scalar::Geospatial(held) => fmt::Display::fmt(held, formatter),
            Scalar::Nested(held) => fmt::Display::fmt(held, formatter),
            _ => unreachable!("every other family spells text, answered above"),
        },
    }
}

impl fmt::Debug for TypedScalar<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TypedScalar")
            .field("field", &self.field)
            .field("value", &self.value)
            .finish()
    }
}

impl fmt::Display for TypedScalar<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_value(formatter, &self.value)
    }
}

impl PartialEq for TypedScalar<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.dtype() == other.dtype() && self.value == other.value
    }
}

impl Eq for TypedScalar<'_> {}

impl PartialOrd for TypedScalar<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TypedScalar<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.dtype()
            .cmp(other.dtype())
            .then_with(|| self.value.cmp(&other.value))
    }
}

impl Hash for TypedScalar<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.dtype().hash(state);
        self.value.hash(state);
    }
}

impl Serialize for TypedScalar<'_> {
    /// Write the field and the value; the proof does not survive a
    /// serialization, which is why the pairing implements no `Deserialize`.
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut structure = serializer.serialize_struct("TypedScalar", 2)?;
        structure.serialize_field("field", self.field)?;
        structure.serialize_field("value", &self.value)?;
        structure.end()
    }
}

impl AsRef<Scalar> for TypedScalar<'_> {
    fn as_ref(&self) -> &Scalar {
        &self.value
    }
}

impl From<TypedScalar<'_>> for Scalar {
    fn from(typed: TypedScalar<'_>) -> Self {
        typed.into_value()
    }
}

/// A [`Field`] and a value it has not yet proven.
///
/// This is the pairing before the check [`TypedScalar`] carries: a wire
/// spelling and the field a reader resolved it to, held together so the
/// reader can ask for the field's reading of the value without committing to
/// it. Every numeric accessor casts on read - text `"42"` under an `Int64`
/// field answers `Some(42)` - by running the datatype's own value contract
/// and reading the family view of its answer, so a reading that fails
/// answers `None` rather than an error. [`Self::as_str`], [`Self::as_bytes`]
/// and [`Self::as_bool`] borrow instead: they hand back the held value only
/// when it already has that shape, and never read it - text `"true"` under
/// a Boolean field answers `None` until [`Self::checked`] proves it.
///
/// The state is unproven, so the pairing has no equality, hash, or serde:
/// [`Self::checked`] is the door to a value that does.
///
/// ```
/// use yggdryl::types::UncheckedTypedScalar;
/// use yggdryl::{DataType, Field, Scalar};
///
/// # fn main() -> yggdryl::Result<()> {
/// let size = Field::new("size", DataType::Int64, false);
/// let unchecked = UncheckedTypedScalar::from_str(&size, "42");
/// // The held text is borrowed as it is, and cast when read as a number.
/// assert_eq!(unchecked.as_str(), Some("42"));
/// assert_eq!(unchecked.as_i64(), Some(42));
/// assert_eq!(unchecked.value(), &Scalar::from("42"));
/// // Proving it rewrites the value into what the field stores.
/// let checked = unchecked.checked()?;
/// assert_eq!(checked.value(), &Scalar::from(42_i64));
///
/// let wrong = UncheckedTypedScalar::from_str(&size, "forty-two");
/// assert_eq!(wrong.as_i64(), None);
/// assert!(wrong.checked().is_err());
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct UncheckedTypedScalar<'a> {
    field: &'a Field,
    value: Scalar,
}

impl<'a> UncheckedTypedScalar<'a> {
    /// Hold a value beside a field without checking it.
    pub fn new(field: &'a Field, value: impl Into<Scalar>) -> Self {
        Self {
            field,
            value: value.into(),
        }
    }

    /// Hold a wire spelling beside the field it was resolved to.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(field: &'a Field, text: &str) -> Self {
        Self::new(field, Scalar::from(text))
    }

    /// The field the value is held beside.
    pub const fn field(&self) -> &'a Field {
        self.field
    }

    /// The datatype the value will be read as.
    pub const fn dtype(&self) -> &'a DataType {
        self.field.dtype()
    }

    /// The name of the field the value is held beside.
    pub fn name(&self) -> &'a str {
        self.field.name()
    }

    /// The value as it was held, unread.
    pub const fn value(&self) -> &Scalar {
        &self.value
    }

    /// Return whether the held value is null.
    pub const fn is_null(&self) -> bool {
        self.value.is_null()
    }

    /// Prove the value under the field.
    ///
    /// # Errors
    ///
    /// Returns what [`TypedScalar::new`] returns for the held value.
    pub fn checked(self) -> Result<TypedScalar<'a>> {
        TypedScalar::new(self.field, self.value)
    }

    /// The datatype's reading of the held value, when it has one.
    fn read(&self) -> Option<Scalar> {
        self.field.dtype().scalar(self.value.clone()).ok()
    }

    /// Borrow the held value when it already is text.
    pub fn as_str(&self) -> Option<&str> {
        self.value.as_str()
    }

    /// Borrow the held value when it already is bytes.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        self.value.as_bytes()
    }

    /// Borrow the held value when it already is a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        self.value.as_bool()
    }

    /// The field's reading of the value as an `i64`, when it fits.
    pub fn as_i64(&self) -> Option<i64> {
        self.read()?.as_i64()
    }

    /// The field's reading of the value as a `u64`, when it fits.
    pub fn as_u64(&self) -> Option<u64> {
        self.read()?.as_u64()
    }

    /// The field's reading of the value as an `i128`, when it fits.
    pub fn as_i128(&self) -> Option<i128> {
        self.read()?.as_i128()
    }

    /// The field's reading of the value as an `f64`.
    pub fn as_f64(&self) -> Option<f64> {
        self.read()?.as_f64()
    }

    /// The field's reading of the value as an integer of any width.
    pub fn as_integer(&self) -> Option<super::Integer> {
        self.read()?.as_integer()
    }

    /// The field's reading of the value as a float of any width.
    pub fn as_float(&self) -> Option<super::Floating> {
        self.read()?.as_float()
    }

    /// The field's reading of the value as a decimal of any width.
    pub fn as_decimal(&self) -> Option<(crate::I256, i8)> {
        self.read()?.as_decimal()
    }

    /// The field's reading of the value as a temporal of any family.
    pub fn as_temporal(&self) -> Option<super::Temporal> {
        self.read()?.as_temporal().copied()
    }

    /// Consume the pairing and return the held value, unread.
    pub fn into_value(self) -> Scalar {
        self.value
    }
}

impl fmt::Debug for UncheckedTypedScalar<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UncheckedTypedScalar")
            .field("field", &self.field)
            .field("value", &self.value)
            .finish()
    }
}

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
static_field_constructor!(super::ascii::IsinType, DataType::Isin);
static_field_constructor!(super::ascii::SideType, DataType::Side);
static_field_constructor!(super::ascii::MsgDirectionType, DataType::MsgDirection);
static_field_constructor!(super::nested::VariantType, DataType::Variant);
static_field_constructor!(super::uuid::UuidType, DataType::Uuid);
static_field_constructor!(super::version::VersionType, DataType::Version);
static_field_constructor!(super::url::UrlType, DataType::Url);

#[cfg(test)]
mod tests;
