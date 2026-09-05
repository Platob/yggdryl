//! UTF-8 field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(Utf8Type, "utf8", crate::DataType::Utf8);
define_field_types!(LargeUtf8Type, "large_utf8", crate::DataType::LargeUtf8);
define_field_types!(Utf8ViewType, "utf8_view", crate::DataType::Utf8View);

/// A UTF-8-typed field.
pub type Utf8Field = TypedField<Utf8Type>;
/// A large UTF-8-typed field.
pub type LargeUtf8Field = TypedField<LargeUtf8Type>;
/// A UTF-8-view-typed field.
pub type Utf8ViewField = TypedField<Utf8ViewType>;
