//! UTF-8 datatype family.

use smol_str::format_smolstr;

use crate::{DataType, DataTypeId, Error};

/// One Arrow UTF-8 datatype.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum TextType {
    /// UTF-8 with 32-bit offsets.
    Utf8,
    /// UTF-8 with 64-bit offsets.
    LargeUtf8,
    /// UTF-8 view layout.
    Utf8View,
}

impl TextType {
    /// Return the exact datatype identifier.
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Utf8 => DataTypeId::Utf8,
            Self::LargeUtf8 => DataTypeId::LargeUtf8,
            Self::Utf8View => DataTypeId::Utf8View,
        }
    }
}

impl From<TextType> for DataType {
    fn from(value: TextType) -> Self {
        match value {
            TextType::Utf8 => Self::Utf8,
            TextType::LargeUtf8 => Self::LargeUtf8,
            TextType::Utf8View => Self::Utf8View,
        }
    }
}

impl TryFrom<&DataType> for TextType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self, Self::Error> {
        match value {
            DataType::Utf8 => Ok(Self::Utf8),
            DataType::LargeUtf8 => Ok(Self::LargeUtf8),
            DataType::Utf8View => Ok(Self::Utf8View),
            other => Err(Error::InvalidDataType {
                kind: "text",
                reason: format_smolstr!("expected a text datatype, got {other}"),
            }),
        }
    }
}
