//! The [`Field`] base trait.

use crate::dtype::DataType;
use crate::field::Metadata;

/// A named column, generic over the native value type `T` of its data type. It
/// carries its `name`, its [`DataType`] (`dtype`), whether it is
/// [`nullable`](Field::nullable), optional byte-keyed [`Metadata`], and — through
/// [`default`](Field::default) — its default value (`None` when nullable, otherwise
/// `Some` of the type's default). Mirrors [`DataType`].
///
/// ```
/// use yggdryl_schema::{DataType, DataTypeId, Field, Int32Field};
///
/// let field = Int32Field::new("count");
/// assert_eq!(field.name(), "count");
/// assert_eq!(field.dtype().type_id(), DataTypeId::Int32);
/// assert!(!field.nullable()); // non-nullable by default
/// assert_eq!(field.default(), Some(0i32));
/// assert_eq!(field.with_nullable(true).default(), None); // nullable → no default
/// ```
pub trait Field<T> {
    /// The concrete data type this field carries.
    type DType: DataType<T>;

    /// The field's name.
    fn name(&self) -> &str;

    /// The field's data type.
    fn dtype(&self) -> &Self::DType;

    /// Whether this field admits null values.
    fn nullable(&self) -> bool;

    /// The field's metadata, if any.
    fn metadata(&self) -> Option<&Metadata>;

    /// The field's default value: `None` when the field is
    /// [`nullable`](Field::nullable), otherwise `Some` of its data type's
    /// [`default`](DataType::default).
    fn default(&self) -> Option<T>
    where
        T: Default,
    {
        if self.nullable() {
            None
        } else {
            Some(self.dtype().default())
        }
    }
}
