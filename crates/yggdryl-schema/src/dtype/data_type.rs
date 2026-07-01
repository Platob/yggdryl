//! The [`DataType`] base trait.

use crate::dtype::DataTypeId;

/// A data type, generic over the native value type `T` it describes. It knows its
/// [`DataTypeId`], its [`type_name`](DataType::type_name), and — through
/// [`default`](DataType::default) — the default value of `T`. Each concrete type
/// also carries a category marker (currently [`PrimitiveType`](crate::PrimitiveType)).
///
/// ```
/// use yggdryl_schema::{DataType, DataTypeId, Int32Type};
///
/// let dt = Int32Type::new();
/// assert_eq!(dt.type_id(), DataTypeId::Int32);
/// assert_eq!(dt.type_name(), "int32");
/// assert_eq!(dt.default(), 0i32); // the default value of the native type
/// ```
pub trait DataType<T> {
    /// The discriminant identifying this type.
    fn type_id(&self) -> DataTypeId;

    /// The type's name (e.g. `"int32"`).
    fn type_name(&self) -> &str;

    /// The default value of the native type `T` — its zero.
    fn default(&self) -> T
    where
        T: Default,
    {
        T::default()
    }
}
