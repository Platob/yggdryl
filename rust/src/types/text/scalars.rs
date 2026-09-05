//! UTF-8 values and typed scalar aliases.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::types::Scalar;
use crate::types::typed::define_scalar_type;

/// Borrowing access shared by every UTF-8 representation.
pub trait TextValue: crate::ScalarValue {
    /// Borrow the Unicode text.
    fn as_str(&self) -> &str;
}

macro_rules! text_leaf {
    ($name:ident) => {
        #[doc = concat!("One `", stringify!($name), "` text value.")]
        #[repr(transparent)]
        #[derive(
            Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(SmolStr);

        impl $name {
            /// Construct this text representation.
            pub fn new(value: impl Into<SmolStr>) -> Self {
                Self(value.into())
            }

            /// Borrow the Unicode text.
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }

            /// Consume this value and return its compact string.
            pub fn into_inner(self) -> SmolStr {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl From<SmolStr> for $name {
            fn from(value: SmolStr) -> Self {
                Self::new(value)
            }
        }
    };
}

text_leaf!(Utf8);
text_leaf!(LargeUtf8);
text_leaf!(Utf8View);

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

define_scalar_type!(Utf8Scalar, super::Utf8Type, "utf8", crate::DataType::Utf8);
define_scalar_type!(
    LargeUtf8Scalar,
    super::LargeUtf8Type,
    "large_utf8",
    crate::DataType::LargeUtf8
);
define_scalar_type!(
    Utf8ViewScalar,
    super::Utf8ViewType,
    "utf8_view",
    crate::DataType::Utf8View
);
