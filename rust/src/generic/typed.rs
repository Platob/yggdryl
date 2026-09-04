//! A value and the datatype it belongs to, kept together.
//!
//! [`Scalar::dtype`] names the datatype a value already is, and a
//! [`crate::Field`] validates a whole row against a schema. [`TypedScalar`] is
//! the pair in between: one value and one datatype, checked against each other,
//! for a caller holding a single value with no row and no schema around it.
//!
//! The pairing also carries the same compile-time markers a [`crate::Field`]
//! does, so a caller who knows which datatype is coming can say so in the type:
//! [`Int64Scalar`] is a `TypedScalar` that cannot hold anything but an `Int64`,
//! and [`TypedScalar`] with no marker holds any datatype at all. The markers are
//! exactly [`crate::FieldType`]'s - one family names the variants, and a value
//! and a field spell the same one.
//!
//! A null is accepted by every datatype. Nullability is a property of the
//! column, not of the value, so the value model accepts a null wherever a value
//! goes and the schema beside it says whether that was allowed.
//!
//! ```
//! use yggdryl::generic::Int64Scalar;
//! use yggdryl::{DataType, TypedScalar, Scalar};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let price = TypedScalar::from_parts(DataType::Int64, Scalar::from(7_i64))?;
//! assert_eq!(price.dtype(), &DataType::Int64);
//! assert_eq!(price.value(), &Scalar::I64(7));
//!
//! // The value is checked against the datatype, so a pairing that exists holds.
//! assert!(TypedScalar::from_parts(DataType::Int64, Scalar::from("seven")).is_err());
//!
//! // A value can also name its own datatype.
//! assert_eq!(TypedScalar::from_value(Scalar::from(1.5))?.dtype(), &DataType::Float64);
//!
//! // A marker fixes the datatype at compile time; the value is still checked.
//! let typed = Int64Scalar::new(Scalar::from(7_i64))?;
//! assert_eq!(typed.dtype(), &DataType::Int64);
//! assert!(Int64Scalar::try_from_parts(DataType::Utf8, Scalar::from("seven")).is_err());
//!
//! // A null is accepted by every datatype, and `is_null` is how it reads back.
//! assert!(TypedScalar::from_parts(DataType::Int64, Scalar::Null)?.is_null());
//! assert!(!price.is_null());
//! # Ok(())
//! # }
//! ```

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use serde::de::Error as _;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::types::{
    ascii, boolean, bytes, decimal, floating, geospatial, guid, integer, nested, temporal, text,
};
use crate::{AnyType, DataType, Error, FieldType, Result, Scalar};

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
        ensure_marker::<K>(&dtype)?;
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
        ensure_marker::<K>(&dtype)?;
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
        ensure_marker::<J>(&self.dtype)?;
        Ok(TypedScalar {
            dtype: self.dtype,
            value: self.value,
            marker: PhantomData,
        })
    }

    /// Build the pairing without re-checking the marker.
    fn from_checked_parts(dtype: DataType, value: Scalar) -> Result<Self> {
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
        ensure_marker::<K>(&dtype)?;
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
    /// assert_eq!(typed.value(), &Scalar::I64(7));
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
fn ensure_marker<K: FieldType>(dtype: &DataType) -> Result<()> {
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

/// Give one statically known datatype an infallible-datatype constructor.
macro_rules! static_value_constructor {
    ($marker:path, $dtype:expr) => {
        impl TypedScalar<$marker> {
            /// Pairs this statically known datatype with a value it accepts.
            ///
            /// # Errors
            ///
            /// Returns an error when the value is neither null nor a value the
            /// datatype accepts.
            pub fn new(value: Scalar) -> Result<Self> {
                Self::from_checked_parts($dtype, value)
            }
        }
    };
}

/// Name one datatype's value pairing.
macro_rules! typed_value_alias {
    ($alias:ident, $marker:path, $name:literal) => {
        #[doc = concat!("A `", $name, "`-typed value paired with its datatype.")]
        pub type $alias = TypedScalar<$marker>;
    };
}

typed_value_alias!(NullScalar, boolean::Null, "null");
typed_value_alias!(BooleanScalar, boolean::Boolean, "boolean");
typed_value_alias!(Int8Scalar, integer::Int8, "int8");
typed_value_alias!(Int16Scalar, integer::Int16, "int16");
typed_value_alias!(Int32Scalar, integer::Int32, "int32");
typed_value_alias!(Int64Scalar, integer::Int64, "int64");
typed_value_alias!(UInt8Scalar, integer::UInt8, "uint8");
typed_value_alias!(UInt16Scalar, integer::UInt16, "uint16");
typed_value_alias!(UInt32Scalar, integer::UInt32, "uint32");
typed_value_alias!(UInt64Scalar, integer::UInt64, "uint64");
typed_value_alias!(Float16Scalar, floating::Float16, "float16");
typed_value_alias!(Float32Scalar, floating::Float32, "float32");
typed_value_alias!(Float64Scalar, floating::Float64, "float64");
typed_value_alias!(TimestampScalar, temporal::Timestamp, "timestamp");
typed_value_alias!(Date32Scalar, temporal::Date32, "date32");
typed_value_alias!(Date64Scalar, temporal::Date64, "date64");
typed_value_alias!(Time32Scalar, temporal::Time32, "time32");
typed_value_alias!(Time64Scalar, temporal::Time64, "time64");
typed_value_alias!(Duration32Scalar, temporal::Duration32, "duration32");
typed_value_alias!(Duration64Scalar, temporal::Duration64, "duration64");
typed_value_alias!(IntervalScalar, temporal::Interval, "interval");
typed_value_alias!(BinaryScalar, bytes::Binary, "binary");
typed_value_alias!(
    FixedSizeBinaryScalar,
    bytes::FixedSizeBinary,
    "fixed_size_binary"
);
typed_value_alias!(LargeBinaryScalar, bytes::LargeBinary, "large_binary");
typed_value_alias!(BinaryViewScalar, bytes::BinaryView, "binary_view");
typed_value_alias!(Utf8Scalar, text::Utf8, "utf8");
typed_value_alias!(LargeUtf8Scalar, text::LargeUtf8, "large_utf8");
typed_value_alias!(Utf8ViewScalar, text::Utf8View, "utf8_view");
typed_value_alias!(AsciiScalar, ascii::Ascii, "ascii");
typed_value_alias!(FixedAsciiScalar, ascii::FixedAscii, "fixed_ascii");
typed_value_alias!(CountryScalar, ascii::Country, "country");
typed_value_alias!(CurrencyScalar, ascii::Currency, "currency");
typed_value_alias!(MicScalar, ascii::Mic, "mic");
typed_value_alias!(CfiScalar, ascii::Cfi, "cfi");
typed_value_alias!(ListScalar, nested::List, "list");
typed_value_alias!(ListViewScalar, nested::ListView, "list_view");
typed_value_alias!(
    FixedSizeListScalar,
    nested::FixedSizeList,
    "fixed_size_list"
);
typed_value_alias!(LargeListScalar, nested::LargeList, "large_list");
typed_value_alias!(
    LargeListViewScalar,
    nested::LargeListView,
    "large_list_view"
);
typed_value_alias!(StructScalar, nested::Struct, "struct");
typed_value_alias!(UnionScalar, nested::Union, "union");
typed_value_alias!(DictionaryScalar, nested::Dictionary, "dictionary");
typed_value_alias!(Decimal32Scalar, decimal::Decimal32, "decimal32");
typed_value_alias!(Decimal64Scalar, decimal::Decimal64, "decimal64");
typed_value_alias!(Decimal128Scalar, decimal::Decimal128, "decimal128");
typed_value_alias!(Decimal256Scalar, decimal::Decimal256, "decimal256");
typed_value_alias!(MapScalar, nested::Map, "map");
typed_value_alias!(VariantScalar, nested::Variant, "variant");
typed_value_alias!(GuidScalar, guid::Guid, "guid");
typed_value_alias!(GeometryScalar, geospatial::Geometry, "geometry");
typed_value_alias!(GeographyScalar, geospatial::Geography, "geography");
typed_value_alias!(
    RunEndEncodedScalar,
    nested::RunEndEncoded,
    "run_end_encoded"
);

static_value_constructor!(boolean::Null, DataType::Null);
static_value_constructor!(boolean::Boolean, DataType::Boolean);
static_value_constructor!(integer::Int8, DataType::Int8);
static_value_constructor!(integer::Int16, DataType::Int16);
static_value_constructor!(integer::Int32, DataType::Int32);
static_value_constructor!(integer::Int64, DataType::Int64);
static_value_constructor!(integer::UInt8, DataType::UInt8);
static_value_constructor!(integer::UInt16, DataType::UInt16);
static_value_constructor!(integer::UInt32, DataType::UInt32);
static_value_constructor!(integer::UInt64, DataType::UInt64);
static_value_constructor!(floating::Float16, DataType::Float16);
static_value_constructor!(floating::Float32, DataType::Float32);
static_value_constructor!(floating::Float64, DataType::Float64);
static_value_constructor!(temporal::Date32, DataType::Date32);
static_value_constructor!(temporal::Date64, DataType::Date64);
static_value_constructor!(bytes::Binary, DataType::Binary);
static_value_constructor!(bytes::LargeBinary, DataType::LargeBinary);
static_value_constructor!(bytes::BinaryView, DataType::BinaryView);
static_value_constructor!(text::Utf8, DataType::Utf8);
static_value_constructor!(text::LargeUtf8, DataType::LargeUtf8);
static_value_constructor!(text::Utf8View, DataType::Utf8View);
static_value_constructor!(ascii::Ascii, DataType::Ascii);
static_value_constructor!(ascii::Country, DataType::Country);
static_value_constructor!(ascii::Currency, DataType::Currency);
static_value_constructor!(ascii::Mic, DataType::Mic);
static_value_constructor!(ascii::Cfi, DataType::Cfi);
static_value_constructor!(nested::Variant, DataType::Variant);
static_value_constructor!(guid::Guid, DataType::Guid);

#[cfg(test)]
mod tests;
