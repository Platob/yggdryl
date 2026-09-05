//! UTF-8 typed scalar aliases.

use smol_str::SmolStr;

use crate::types::Scalar;
use crate::types::typed::define_scalar_type;

/// Borrowing access shared by every UTF-8 representation.
pub trait TextValue: crate::ScalarValue {
    /// Borrow the Unicode text.
    fn as_str(&self) -> &str;
}

impl From<&str> for Scalar {
    fn from(value: &str) -> Self {
        Self::String(SmolStr::new(value))
    }
}

impl From<String> for Scalar {
    fn from(value: String) -> Self {
        Self::String(SmolStr::from(value))
    }
}

impl From<SmolStr> for Scalar {
    fn from(value: SmolStr) -> Self {
        Self::String(value)
    }
}

define_scalar_type!(Utf8Scalar, super::Utf8, "utf8", crate::DataType::Utf8);
define_scalar_type!(
    LargeUtf8Scalar,
    super::LargeUtf8,
    "large_utf8",
    crate::DataType::LargeUtf8
);
define_scalar_type!(
    Utf8ViewScalar,
    super::Utf8View,
    "utf8_view",
    crate::DataType::Utf8View
);
