//! String field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(Utf8Type, "utf8", crate::DataType::Utf8);
define_field_types!(LargeUtf8Type, "large_utf8", crate::DataType::LargeUtf8);
define_field_types!(Utf8ViewType, "utf8_view", crate::DataType::Utf8View);
define_field_types!(StringType, "string", crate::DataType::String(_));

/// A UTF-8-typed field.
pub type Utf8Field = TypedField<Utf8Type>;
/// A large UTF-8-typed field.
pub type LargeUtf8Field = TypedField<LargeUtf8Type>;
/// A UTF-8-view-typed field.
pub type Utf8ViewField = TypedField<Utf8ViewType>;
/// A field of strings declaring a charset, a bound, or both.
///
/// The three markers above are the plain UTF-8 layouts, which are datatypes
/// of their own; this one is every string that declares more than its layout,
/// whichever of the five layouts it declares.
pub type StringField = TypedField<StringType>;
