//! UTF-8 values and typed scalar aliases.

use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::types::Scalar;
use crate::types::typed::define_scalar_type;
use crate::{DataType, DataTypeId, DataTypeKind, Result, ScalarFamily, ScalarValue};

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

/// One exact UTF-8 storage representation.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub enum Text {
    /// UTF-8 with 32-bit offsets.
    Utf8(Utf8),
    /// UTF-8 with 64-bit offsets.
    LargeUtf8(LargeUtf8),
    /// UTF-8 view storage.
    Utf8View(Utf8View),
}

impl Text {
    /// Borrow the Unicode text independently of its storage layout.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Utf8(value) => value.as_str(),
            Self::LargeUtf8(value) => value.as_str(),
            Self::Utf8View(value) => value.as_str(),
        }
    }
}

impl fmt::Display for Text {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl PartialEq for Text {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Text {}

impl PartialOrd for Text {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Text {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for Text {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

const _: () = assert!(std::mem::size_of::<Text>() == 32);

macro_rules! text_value {
    ($leaf:ident, $marker:ty, $variant:ident, $id:ident, $dtype:ident) => {
        impl ScalarValue for $leaf {
            type Family = Text;
            type Type = $marker;

            const ID: DataTypeId = DataTypeId::$id;
            const KIND: DataTypeKind = DataTypeKind::Text;

            fn dtype(&self) -> Result<DataType> {
                Ok(DataType::$dtype)
            }

            fn into_family(self) -> Self::Family {
                Text::$variant(self)
            }

            fn from_family(family: &Self::Family) -> Option<&Self> {
                match family {
                    Text::$variant(value) => Some(value),
                    _ => None,
                }
            }

            fn into_scalar(self) -> Scalar {
                Scalar::Text(Text::$variant(self))
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::Text(Text::$variant(value)) => Some(value),
                    _ => None,
                }
            }
        }

        impl TextValue for $leaf {
            fn as_str(&self) -> &str {
                <$leaf>::as_str(self)
            }
        }
    };
}

text_value!(Utf8, super::Utf8Type, Utf8, Utf8, Utf8);
text_value!(
    LargeUtf8,
    super::LargeUtf8Type,
    LargeUtf8,
    LargeUtf8,
    LargeUtf8
);
text_value!(Utf8View, super::Utf8ViewType, Utf8View, Utf8View, Utf8View);

impl ScalarFamily for Text {
    const KIND: DataTypeKind = DataTypeKind::Text;

    fn id(&self) -> DataTypeId {
        match self {
            Self::Utf8(_) => DataTypeId::Utf8,
            Self::LargeUtf8(_) => DataTypeId::LargeUtf8,
            Self::Utf8View(_) => DataTypeId::Utf8View,
        }
    }

    fn dtype(&self) -> Result<DataType> {
        Ok(match self {
            Self::Utf8(_) => DataType::Utf8,
            Self::LargeUtf8(_) => DataType::LargeUtf8,
            Self::Utf8View(_) => DataType::Utf8View,
        })
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Text(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Text(value) => Some(value),
            _ => None,
        }
    }
}

impl From<&str> for Scalar {
    fn from(value: &str) -> Self {
        Self::Text(Text::Utf8(Utf8::new(value)))
    }
}

impl From<String> for Scalar {
    fn from(value: String) -> Self {
        Self::Text(Text::Utf8(Utf8::new(value)))
    }
}

impl From<SmolStr> for Scalar {
    fn from(value: SmolStr) -> Self {
        Self::Text(Text::Utf8(Utf8::new(value)))
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
