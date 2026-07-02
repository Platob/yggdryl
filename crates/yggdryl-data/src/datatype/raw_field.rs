//! The [`RawField`] base trait: a named, nullable column of a [`RawDataType`].

use super::RawDataType;

/// A named, nullable column of a data type — the base trait mirroring an Apache
/// Arrow `Field`.
///
/// It pairs a [`name`](RawField::name) with a [`data_type`](RawField::data_type) of
/// type `D` and a [`is_nullable`](RawField::is_nullable) flag, so a schema is a
/// sequence of fields. It is parameterised by the data type `D` (rather than boxing
/// it) so the concrete type is preserved for zero-cost, monomorphised access.
///
/// ```
/// use yggdryl_data::{RawDataType, RawField};
///
/// struct Int32;
/// impl RawDataType for Int32 {
///     fn name(&self) -> &str { "int32" }
///     fn arrow_format(&self) -> String { "i".to_string() }
///     fn byte_width(&self) -> Option<usize> { Some(4) }
/// }
///
/// struct Column {
///     name: String,
///     data_type: Int32,
///     nullable: bool,
/// }
///
/// impl RawField<Int32> for Column {
///     fn name(&self) -> &str {
///         &self.name
///     }
///     fn data_type(&self) -> &Int32 {
///         &self.data_type
///     }
///     fn is_nullable(&self) -> bool {
///         self.nullable
///     }
/// }
///
/// let id = Column { name: "id".to_string(), data_type: Int32, nullable: false };
/// assert_eq!(id.name(), "id");
/// assert_eq!(id.data_type().name(), "int32");
/// assert!(!id.is_nullable());
/// ```
pub trait RawField<D: RawDataType> {
    /// The field's name.
    fn name(&self) -> &str;

    /// The field's data type.
    fn data_type(&self) -> &D;

    /// Whether values in this field may be null.
    fn is_nullable(&self) -> bool;
}
