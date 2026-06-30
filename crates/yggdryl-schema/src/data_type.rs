//! The [`DataType`] base trait every yggdryl data type implements, plus the
//! [`PhysicalType`] / [`LogicalType`] / [`NestedType`] category markers.

use crate::data_type_id::DataTypeId;

/// Behaviour shared by every yggdryl data type.
///
/// A type knows its canonical [`name`](DataType::name) and its
/// [`type_id`](DataType::type_id); the category predicates
/// ([`is_physical`](DataType::is_physical),
/// [`is_logical`](DataType::is_logical), [`is_nested`](DataType::is_nested))
/// follow from the id's block by default, so an implementor only supplies the two
/// required accessors (plus the Arrow conversion under the `arrow` feature). Each
/// concrete type also implements the matching category marker — [`PhysicalType`],
/// [`LogicalType`] or [`NestedType`].
///
/// ```
/// use yggdryl_schema::{DataType, DataTypeId, PhysicalType};
///
/// struct Int32;
/// impl DataType for Int32 {
///     fn name(&self) -> &'static str { "int32" }
///     fn type_id(&self) -> DataTypeId { DataTypeId::Int32 }
/// #     #[cfg(feature = "arrow")]
/// #     fn to_arrow(&self) -> arrow_schema::DataType { arrow_schema::DataType::Int32 }
/// #     #[cfg(feature = "arrow")]
/// #     fn from_arrow(_dtype: &arrow_schema::DataType) -> Result<Self, yggdryl_schema::SchemaError> { Ok(Int32) }
/// }
/// impl PhysicalType for Int32 {}
///
/// fn assert_physical<T: PhysicalType>(value: &T) { assert!(value.is_physical()); }
///
/// assert_eq!(Int32.name(), "int32");
/// assert_physical(&Int32);
/// assert!(!Int32.is_nested());
/// ```
pub trait DataType {
    /// The canonical type name, e.g. `"int32"` or `"large_binary"`.
    fn name(&self) -> &'static str;

    /// The type's [`DataTypeId`] discriminant.
    fn type_id(&self) -> DataTypeId;

    /// Whether this is a physical (storage) type.
    fn is_physical(&self) -> bool {
        self.type_id().is_physical()
    }

    /// Whether this is a logical (reinterpreted) type.
    fn is_logical(&self) -> bool {
        self.type_id().is_logical()
    }

    /// Whether this is a nested (child-bearing) type.
    fn is_nested(&self) -> bool {
        self.type_id().is_nested()
    }

    /// The maximum number of bytes a value of this type may hold, or `None` if it
    /// is unbounded. Fixed- and max-size types report their width here, which the
    /// scalar layer uses to truncate over-long byte payloads.
    fn max_byte_size(&self) -> Option<i64> {
        None
    }

    /// Converts this type to its Apache Arrow equivalent.
    #[cfg(feature = "arrow")]
    fn to_arrow(&self) -> arrow_schema::DataType;

    /// Builds this type from its Apache Arrow equivalent, erroring on an Arrow
    /// type with no yggdryl match.
    #[cfg(feature = "arrow")]
    fn from_arrow(dtype: &arrow_schema::DataType) -> Result<Self, crate::SchemaError>
    where
        Self: Sized;
}

/// Marker for *physical* (storage) types — fixed-width or variable-length
/// binary-backed values that hold no child fields. Implement it for types whose
/// [`type_id`](DataType::type_id) is in the physical block, so
/// [`is_physical`](DataType::is_physical) is `true`.
pub trait PhysicalType: DataType {}

/// Marker for *logical* types — a physical type reinterpreted with extra meaning
/// (a date, a decimal, a dictionary, …). Implement it for types whose
/// [`type_id`](DataType::type_id) is in the logical block, so
/// [`is_logical`](DataType::is_logical) is `true`.
pub trait LogicalType: DataType {}

/// Marker for *nested* types — those built from one or more child fields (a list,
/// a struct, a map, …). Implement it for types whose
/// [`type_id`](DataType::type_id) is in the nested block, so
/// [`is_nested`](DataType::is_nested) is `true`.
pub trait NestedType: DataType {}
