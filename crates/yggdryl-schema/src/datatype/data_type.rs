//! The base trait every yggdryl data type implements.

use core::fmt::{Debug, Display};
use core::hash::Hash;

use arrow_schema::DataType as ArrowDataType;

use crate::{DataTypeError, DataTypeId};

/// A yggdryl data type: the typed description of a value's physical layout
/// and semantics.
///
/// Every concrete type — one per file, grouped one module per category —
/// implements this trait; category subtraits ([`PrimitiveType`],
/// [`LogicalType`], [`NestedType`]) refine it. The Arrow mapping is total and
/// reversible for the supported subset: `from_arrow` is the only inbound
/// conversion and validates fully, and `to_arrow` always round-trips back.
///
/// The trait is deliberately not object safe (`from_arrow` and `from_bytes`
/// are constructors); the object-safe erasure arrives with the `Datum` layer
/// above this crate.
///
/// ```
/// use yggdryl_schema::{DataType, DataTypeError, Int8};
///
/// let arrow = Int8.to_arrow();
/// assert_eq!(Int8::from_arrow(&arrow), Ok(Int8));
/// assert!(matches!(
///     Int8::from_arrow(&arrow_schema::DataType::Utf8),
///     Err(DataTypeError::ArrowTypeMismatch { .. })
/// ));
/// ```
///
/// [`PrimitiveType`]: crate::PrimitiveType
/// [`LogicalType`]: crate::LogicalType
/// [`NestedType`]: crate::NestedType
pub trait DataType: Clone + Debug + Display + Eq + Hash + Send + Sync + Sized + 'static {
    /// The identifier of this type's constructor, shared by every
    /// parameterization.
    const TYPE_ID: DataTypeId;

    /// The identifier of this value's type constructor.
    fn type_id(&self) -> DataTypeId {
        Self::TYPE_ID
    }

    /// The Arrow data type this type maps to.
    fn to_arrow(&self) -> ArrowDataType;

    /// Validates and converts an Arrow data type back into this type — the
    /// only inbound conversion.
    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError>;

    /// Serializes the type to its canonical byte encoding.
    fn to_bytes(&self) -> Vec<u8>;

    /// Deserializes the type from the encoding produced by
    /// [`to_bytes`](DataType::to_bytes), validating fully.
    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError>;
}
