//! The [`Field`] base trait.

use crate::dtype::DataType;
use crate::field::Metadata;

/// A named column, generic over the native value type `T` of its data type. It
/// carries its `name`, its [`DataType`] (`dtype`), optional byte-keyed [`Metadata`],
/// and — through [`default`](Field::default) — the default value of `T`. Mirrors
/// [`DataType`].
///
/// ```
/// use yggdryl_schema::{DataType, DataTypeId, Field, Int32Field};
///
/// let field = Int32Field::new("count");
/// assert_eq!(field.name(), "count");
/// assert_eq!(field.dtype().type_id(), DataTypeId::Int32);
/// assert_eq!(field.default(), 0i32);
/// ```
pub trait Field<T> {
    /// The concrete data type this field carries.
    type DType: DataType<T>;

    /// The field's name.
    fn name(&self) -> &str;

    /// The field's data type.
    fn dtype(&self) -> &Self::DType;

    /// The field's metadata, if any.
    fn metadata(&self) -> Option<&Metadata>;

    /// The default value of the native type `T` — the field's data type's default.
    fn default(&self) -> T
    where
        T: Default,
    {
        self.dtype().default()
    }
}
