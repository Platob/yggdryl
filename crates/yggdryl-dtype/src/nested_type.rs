//! [`NestedType`] — the nested category of [`DataType`] (scaffolding).

use crate::DataType;

/// A nested data type — one composed of child types (a list of a value type, a struct
/// of fields, a map of key/value).
///
/// This category trait is **scaffolding**: it establishes the nested layer of the
/// hierarchy so future concrete types (`List`, `Struct`, `Map`) slot in beside the
/// primitives without reshaping the API. No concrete nested types exist yet.
///
/// ```
/// use yggdryl_dtype::{DataType, NestedType};
/// fn name_of<N: NestedType>(nested: &N) -> &'static str {
///     nested.name()
/// }
/// ```
pub trait NestedType: DataType {}
