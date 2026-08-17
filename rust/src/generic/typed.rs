//! A value and the datatype it belongs to, kept together.
//!
//! [`Value::data_type`] names the datatype a value already is, and a
//! [`crate::Field`] validates a whole row against a schema. [`TypedValue`] is
//! the pair in between: one value and one datatype, checked against each other,
//! for a caller holding a single value with no row and no schema around it.
//!
//! The pairing also carries the same compile-time markers a [`crate::Field`]
//! does, so a caller who knows which datatype is coming can say so in the type:
//! [`Int64Value`] is a `TypedValue` that cannot hold anything but an `Int64`,
//! and [`TypedValue`] with no marker holds any datatype at all. The markers are
//! exactly [`crate::FieldType`]'s - one family names the variants, and a value
//! and a field spell the same one.
//!
//! A null is accepted by every datatype. Nullability is a property of the
//! column, not of the value, so the value model accepts a null wherever a value
//! goes and the schema beside it says whether that was allowed.
//!
//! ```
//! use yggdryl::generic::Int64Value;
//! use yggdryl::{DataType, TypedValue, Value};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let price = TypedValue::from_parts(DataType::Int64, Value::from(7_i64))?;
//! assert_eq!(price.data_type(), &DataType::Int64);
//! assert_eq!(price.value(), &Value::I64(7));
//!
//! // The value is checked against the datatype, so a pairing that exists holds.
//! assert!(TypedValue::from_parts(DataType::Int64, Value::from("seven")).is_err());
//!
//! // A value can also name its own datatype.
//! assert_eq!(TypedValue::from_value(Value::from(1.5))?.data_type(), &DataType::Float64);
//!
//! // A marker fixes the datatype at compile time; the value is still checked.
//! let typed = Int64Value::new(Value::from(7_i64))?;
//! assert_eq!(typed.data_type(), &DataType::Int64);
//! assert!(Int64Value::try_from_parts(DataType::Utf8, Value::from("seven")).is_err());
//!
//! // A null is accepted by every datatype, and `is_null` is how it reads back.
//! assert!(TypedValue::from_parts(DataType::Int64, Value::Null)?.is_null());
//! assert!(!price.is_null());
//! # Ok(())
//! # }
//! ```

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use serde::de::Error as _;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::field::{binary, decimal, floating, integer, nested, scalar, temporal};
use crate::{AnyType, DataType, Error, FieldType, Result, Value};

/// A datatype and one value it accepts.
///
/// The value is validated against the datatype on construction, through the
/// same walk a column value takes, so a pairing that exists is a pairing that
/// holds. A null is accepted by every datatype, because a null is what a
/// nullable column stores.
///
/// `K` is a zero-sized [`crate::FieldType`] marker naming the datatype variant
/// this pairing is allowed to hold. It defaults to [`AnyType`], which allows
/// every variant, so `TypedValue` with no marker is the dynamic pairing and
/// `TypedValue<K>` is the narrowed one. The marker adds no storage: a narrowed
/// pairing is the same two words a dynamic one is.
///
/// `DataType` is an unordered vocabulary, so a pairing over one answers
/// equality and hashing but not ordering.
pub struct TypedValue<K: FieldType = AnyType> {
    data_type: DataType,
    value: Value,
    marker: PhantomData<K>,
}

impl<K: FieldType> TypedValue<K> {
    /// Pair a datatype with a value it accepts, checking the marker too.
    ///
    /// # Errors
    ///
    /// Returns an error when the datatype is not this marker's variant, or
    /// when the value is neither null nor a value the datatype accepts.
    pub fn try_from_parts(data_type: DataType, value: Value) -> Result<Self> {
        ensure_marker::<K>(&data_type)?;
        Self::from_checked_parts(data_type, value)
    }

    /// Pair a value with the datatype it already names, checking the marker.
    ///
    /// # Errors
    ///
    /// Returns an error when the value names no single datatype, which is what
    /// [`Value::data_type`] reports, or when that datatype is not this
    /// marker's variant.
    pub fn try_from_value(value: Value) -> Result<Self> {
        let data_type = value.data_type()?;
        ensure_marker::<K>(&data_type)?;
        Ok(Self {
            data_type,
            value,
            marker: PhantomData,
        })
    }

    /// The datatype this value belongs to.
    pub const fn data_type(&self) -> &DataType {
        &self.data_type
    }

    /// The value itself.
    pub const fn value(&self) -> &Value {
        &self.value
    }

    /// Return whether the value is null.
    ///
    /// This is [`Value::is_null`] on the value inside, which is how a caller
    /// asks whether the pairing holds a value or records its absence for the
    /// datatype beside it.
    pub const fn is_null(&self) -> bool {
        self.value.is_null()
    }

    /// Consume this pairing and return both halves.
    pub fn into_parts(self) -> (DataType, Value) {
        (self.data_type, self.value)
    }

    /// Consume this pairing and return the value alone.
    pub fn into_value(self) -> Value {
        self.value
    }

    /// Widen this pairing to the marker every datatype satisfies.
    ///
    /// Nothing is checked and nothing is copied: the marker is zero-sized, so
    /// this only forgets which variant the type system was tracking.
    pub fn into_any(self) -> TypedValue {
        TypedValue {
            data_type: self.data_type,
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
    pub fn try_into_typed<J: FieldType>(self) -> Result<TypedValue<J>> {
        ensure_marker::<J>(&self.data_type)?;
        Ok(TypedValue {
            data_type: self.data_type,
            value: self.value,
            marker: PhantomData,
        })
    }

    /// Build the pairing without re-checking the marker.
    fn from_checked_parts(data_type: DataType, value: Value) -> Result<Self> {
        crate::field::validate_data_type_value_for(&data_type, &value)?;
        Ok(Self {
            data_type,
            value,
            marker: PhantomData,
        })
    }
}

impl TypedValue {
    /// Pair a datatype with a value it accepts.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is neither null nor a value the
    /// datatype accepts.
    pub fn from_parts(data_type: DataType, value: Value) -> Result<Self> {
        Self::from_checked_parts(data_type, value)
    }

    /// Pair a value with the datatype it already names.
    ///
    /// # Errors
    ///
    /// Returns an error when the value names no single datatype, which is what
    /// [`Value::data_type`] reports.
    pub fn from_value(value: Value) -> Result<Self> {
        Ok(Self {
            data_type: value.data_type()?,
            value,
            marker: PhantomData,
        })
    }
}

/// Report a datatype that is not the marker's variant.
fn ensure_marker<K: FieldType>(data_type: &DataType) -> Result<()> {
    if K::matches(data_type) {
        Ok(())
    } else {
        Err(Error::InvalidDataType {
            kind: "TypedValue",
            reason: format!(
                "marker {} requires datatype {}, got {}",
                std::any::type_name::<K>(),
                K::NAME,
                data_type.name()
            )
            .into(),
        })
    }
}

impl<K: FieldType> Clone for TypedValue<K> {
    fn clone(&self) -> Self {
        Self {
            data_type: self.data_type.clone(),
            value: self.value.clone(),
            marker: PhantomData,
        }
    }
}

impl<K: FieldType> fmt::Debug for TypedValue<K> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TypedValue")
            .field("data_type", &self.data_type)
            .field("value", &self.value)
            .finish()
    }
}

impl<K: FieldType> PartialEq for TypedValue<K> {
    fn eq(&self, other: &Self) -> bool {
        self.data_type == other.data_type && self.value == other.value
    }
}

impl<K: FieldType> Eq for TypedValue<K> {}

impl<K: FieldType> Hash for TypedValue<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.data_type.hash(state);
        self.value.hash(state);
    }
}

impl<K: FieldType> Serialize for TypedValue<K> {
    /// Write the two halves, and never the marker: a marker is a compile-time
    /// fact about which variant may appear, not data the pairing carries.
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut structure = serializer.serialize_struct("TypedValue", 2)?;
        structure.serialize_field("data_type", &self.data_type)?;
        structure.serialize_field("value", &self.value)?;
        structure.end()
    }
}

impl<'de, K: FieldType> Deserialize<'de> for TypedValue<K> {
    /// Read a pairing back through the constructor that validates one.
    ///
    /// Deriving this would accept a datatype and a value that never agreed,
    /// which is exactly the state [`TypedValue::try_from_parts`] exists to
    /// refuse.
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // This mirror must stay field-for-field identical to `TypedValue`.
        #[derive(Deserialize)]
        struct StructuralTypedValue {
            data_type: DataType,
            value: Value,
        }

        let structural = StructuralTypedValue::deserialize(deserializer)?;
        Self::try_from_parts(structural.data_type, structural.value).map_err(D::Error::custom)
    }
}

impl<K: FieldType> TryFrom<Value> for TypedValue<K> {
    type Error = Error;

    fn try_from(value: Value) -> Result<Self> {
        Self::try_from_value(value)
    }
}

impl<K: FieldType> From<TypedValue<K>> for Value {
    fn from(typed: TypedValue<K>) -> Self {
        typed.into_value()
    }
}

/// Give one statically known datatype an infallible-datatype constructor.
macro_rules! static_value_constructor {
    ($marker:path, $data_type:expr) => {
        impl TypedValue<$marker> {
            /// Pairs this statically known datatype with a value it accepts.
            ///
            /// # Errors
            ///
            /// Returns an error when the value is neither null nor a value the
            /// datatype accepts.
            pub fn new(value: Value) -> Result<Self> {
                Self::from_checked_parts($data_type, value)
            }
        }
    };
}

/// Name one datatype's value pairing.
macro_rules! typed_value_alias {
    ($alias:ident, $marker:path, $name:literal) => {
        #[doc = concat!("A `", $name, "`-typed value paired with its datatype.")]
        pub type $alias = TypedValue<$marker>;
    };
}

typed_value_alias!(NullValue, scalar::Null, "null");
typed_value_alias!(BooleanValue, scalar::Boolean, "boolean");
typed_value_alias!(Int8Value, integer::Int8, "int8");
typed_value_alias!(Int16Value, integer::Int16, "int16");
typed_value_alias!(Int32Value, integer::Int32, "int32");
typed_value_alias!(Int64Value, integer::Int64, "int64");
typed_value_alias!(UInt8Value, integer::UInt8, "uint8");
typed_value_alias!(UInt16Value, integer::UInt16, "uint16");
typed_value_alias!(UInt32Value, integer::UInt32, "uint32");
typed_value_alias!(UInt64Value, integer::UInt64, "uint64");
typed_value_alias!(Float16Value, floating::Float16, "float16");
typed_value_alias!(Float32Value, floating::Float32, "float32");
typed_value_alias!(Float64Value, floating::Float64, "float64");
typed_value_alias!(TimestampValue, temporal::Timestamp, "timestamp");
typed_value_alias!(Date32Value, temporal::Date32, "date32");
typed_value_alias!(Date64Value, temporal::Date64, "date64");
typed_value_alias!(Time32Value, temporal::Time32, "time32");
typed_value_alias!(Time64Value, temporal::Time64, "time64");
typed_value_alias!(DurationValue, temporal::Duration, "duration");
typed_value_alias!(IntervalValue, temporal::Interval, "interval");
typed_value_alias!(BinaryValue, binary::Binary, "binary");
typed_value_alias!(
    FixedSizeBinaryValue,
    binary::FixedSizeBinary,
    "fixed_size_binary"
);
typed_value_alias!(LargeBinaryValue, binary::LargeBinary, "large_binary");
typed_value_alias!(BinaryViewValue, binary::BinaryView, "binary_view");
typed_value_alias!(Utf8Value, binary::Utf8, "utf8");
typed_value_alias!(LargeUtf8Value, binary::LargeUtf8, "large_utf8");
typed_value_alias!(Utf8ViewValue, binary::Utf8View, "utf8_view");
typed_value_alias!(ListValue, nested::List, "list");
typed_value_alias!(ListViewValue, nested::ListView, "list_view");
typed_value_alias!(FixedSizeListValue, nested::FixedSizeList, "fixed_size_list");
typed_value_alias!(LargeListValue, nested::LargeList, "large_list");
typed_value_alias!(LargeListViewValue, nested::LargeListView, "large_list_view");
typed_value_alias!(StructValue, nested::Struct, "struct");
typed_value_alias!(UnionValue, nested::Union, "union");
typed_value_alias!(DictionaryValue, nested::Dictionary, "dictionary");
typed_value_alias!(Decimal32Value, decimal::Decimal32, "decimal32");
typed_value_alias!(Decimal64Value, decimal::Decimal64, "decimal64");
typed_value_alias!(Decimal128Value, decimal::Decimal128, "decimal128");
typed_value_alias!(Decimal256Value, decimal::Decimal256, "decimal256");
typed_value_alias!(MapValue, nested::Map, "map");
typed_value_alias!(RunEndEncodedValue, nested::RunEndEncoded, "run_end_encoded");

static_value_constructor!(scalar::Null, DataType::Null);
static_value_constructor!(scalar::Boolean, DataType::Boolean);
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
static_value_constructor!(binary::Binary, DataType::Binary);
static_value_constructor!(binary::LargeBinary, DataType::LargeBinary);
static_value_constructor!(binary::BinaryView, DataType::BinaryView);
static_value_constructor!(binary::Utf8, DataType::Utf8);
static_value_constructor!(binary::LargeUtf8, DataType::LargeUtf8);
static_value_constructor!(binary::Utf8View, DataType::Utf8View);

#[cfg(test)]
mod tests;
