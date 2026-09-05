//! ASCII values and typed scalar aliases.

use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::types::typed::define_scalar_type;
use crate::{DataType, DataTypeId, DataTypeKind, Result, Scalar, ScalarFamily, ScalarValue, types};

/// Borrowing access shared by every ASCII representation.
pub trait AsciiValue: crate::ScalarValue {
    /// The fixed byte width, or `None` when the datatype carries it.
    const WIDTH: Option<i32>;

    /// Borrow the validated ASCII text.
    fn as_str(&self) -> &str;
}

/// Variable-width validated ASCII text.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Ascii(SmolStr);

impl Ascii {
    /// Validate and construct variable-width ASCII text.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = types::ascii_free_text(value.as_ref().as_bytes())?;
        Ok(Self(SmolStr::new(value)))
    }

    /// Borrow the validated text.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for Ascii {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// ASCII text carrying its fixed padded storage width.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct FixedAscii {
    value: SmolStr,
    width: i32,
}

impl FixedAscii {
    /// Validate text against one positive fixed width.
    pub fn new(value: impl AsRef<str>, width: i32) -> Result<Self> {
        let _ = crate::DataType::ascii(width)?;
        let value = types::ascii_text(width, value.as_ref().as_bytes())?;
        Ok(Self {
            value: SmolStr::new(value),
            width,
        })
    }

    /// Borrow the validated text.
    pub fn as_str(&self) -> &str {
        self.value.as_str()
    }

    /// Return the padded storage width.
    pub const fn width(&self) -> i32 {
        self.width
    }
}

impl fmt::Display for FixedAscii {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

macro_rules! ascii_code_leaf {
    ($name:ident, $width:expr) => {
        #[doc = concat!("One validated `", stringify!($name), "` code.")]
        #[repr(transparent)]
        #[derive(
            Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(SmolStr);

        impl $name {
            /// Validate and construct this registered code width.
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                let value = types::ascii_text($width, value.as_ref().as_bytes())?;
                Ok(Self(SmolStr::new(value)))
            }

            /// Borrow the validated code.
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

ascii_code_leaf!(Country, 2);
ascii_code_leaf!(Currency, 3);
ascii_code_leaf!(Mic, 4);
ascii_code_leaf!(Cfi, 6);

/// One exact ASCII storage or registered-code representation.
///
/// `AsciiFamily` carries the suffix because Rust cannot place the family enum
/// and its required `Ascii` leaf in the same type namespace.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub enum AsciiFamily {
    /// Variable-width ASCII.
    Ascii(Ascii),
    /// Fixed-width ASCII.
    FixedAscii(FixedAscii),
    /// ISO 3166-1 alpha-2 country code.
    Country(Country),
    /// ISO 4217 currency code.
    Currency(Currency),
    /// ISO 10383 market identifier code.
    Mic(Mic),
    /// ISO 10962 classification code.
    Cfi(Cfi),
}

impl AsciiFamily {
    /// Borrow the validated text independently of its storage identity.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Ascii(value) => value.as_str(),
            Self::FixedAscii(value) => value.as_str(),
            Self::Country(value) => value.as_str(),
            Self::Currency(value) => value.as_str(),
            Self::Mic(value) => value.as_str(),
            Self::Cfi(value) => value.as_str(),
        }
    }
}

impl fmt::Display for AsciiFamily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl PartialEq for AsciiFamily {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for AsciiFamily {}

impl PartialOrd for AsciiFamily {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AsciiFamily {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for AsciiFamily {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

macro_rules! ascii_value {
    ($leaf:ident, $marker:ty, $variant:ident, $id:ident, $dtype:ident, $width:expr) => {
        impl ScalarValue for $leaf {
            type Family = AsciiFamily;
            type Type = $marker;

            const ID: DataTypeId = DataTypeId::$id;
            const KIND: DataTypeKind = DataTypeKind::Ascii;

            fn dtype(&self) -> Result<DataType> {
                Ok(DataType::$dtype)
            }

            fn into_family(self) -> Self::Family {
                AsciiFamily::$variant(self)
            }

            fn from_family(family: &Self::Family) -> Option<&Self> {
                match family {
                    AsciiFamily::$variant(value) => Some(value),
                    _ => None,
                }
            }

            fn into_scalar(self) -> Scalar {
                Scalar::Ascii(AsciiFamily::$variant(self))
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::Ascii(AsciiFamily::$variant(value)) => Some(value),
                    _ => None,
                }
            }
        }

        impl AsciiValue for $leaf {
            const WIDTH: Option<i32> = $width;

            fn as_str(&self) -> &str {
                <$leaf>::as_str(self)
            }
        }
    };
}

ascii_value!(Ascii, super::AsciiType, Ascii, Ascii, Ascii, None);
ascii_value!(
    Country,
    super::CountryType,
    Country,
    Country,
    Country,
    Some(2)
);
ascii_value!(
    Currency,
    super::CurrencyType,
    Currency,
    Currency,
    Currency,
    Some(3)
);
ascii_value!(Mic, super::MicType, Mic, Mic, Mic, Some(4));
ascii_value!(Cfi, super::CfiType, Cfi, Cfi, Cfi, Some(6));

impl ScalarValue for FixedAscii {
    type Family = AsciiFamily;
    type Type = super::FixedAsciiType;

    const ID: DataTypeId = DataTypeId::FixedAscii;
    const KIND: DataTypeKind = DataTypeKind::Ascii;

    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::FixedAscii(self.width()))
    }

    fn into_family(self) -> Self::Family {
        AsciiFamily::FixedAscii(self)
    }

    fn from_family(family: &Self::Family) -> Option<&Self> {
        match family {
            AsciiFamily::FixedAscii(value) => Some(value),
            _ => None,
        }
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Ascii(AsciiFamily::FixedAscii(self))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Ascii(AsciiFamily::FixedAscii(value)) => Some(value),
            _ => None,
        }
    }
}

impl AsciiValue for FixedAscii {
    const WIDTH: Option<i32> = None;

    fn as_str(&self) -> &str {
        Self::as_str(self)
    }
}

impl ScalarFamily for AsciiFamily {
    const KIND: DataTypeKind = DataTypeKind::Ascii;

    fn id(&self) -> DataTypeId {
        match self {
            Self::Ascii(_) => DataTypeId::Ascii,
            Self::FixedAscii(_) => DataTypeId::FixedAscii,
            Self::Country(_) => DataTypeId::Country,
            Self::Currency(_) => DataTypeId::Currency,
            Self::Mic(_) => DataTypeId::Mic,
            Self::Cfi(_) => DataTypeId::Cfi,
        }
    }

    fn dtype(&self) -> Result<DataType> {
        match self {
            Self::Ascii(_) => Ok(DataType::Ascii),
            Self::FixedAscii(value) => ScalarValue::dtype(value),
            Self::Country(_) => Ok(DataType::Country),
            Self::Currency(_) => Ok(DataType::Currency),
            Self::Mic(_) => Ok(DataType::Mic),
            Self::Cfi(_) => Ok(DataType::Cfi),
        }
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Ascii(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Ascii(value) => Some(value),
            _ => None,
        }
    }
}

define_scalar_type!(
    AsciiScalar,
    super::AsciiType,
    "ascii",
    crate::DataType::Ascii
);
define_scalar_type!(FixedAsciiScalar, super::FixedAsciiType, "fixed_ascii");
define_scalar_type!(
    CountryScalar,
    super::CountryType,
    "country",
    crate::DataType::Country
);
define_scalar_type!(
    CurrencyScalar,
    super::CurrencyType,
    "currency",
    crate::DataType::Currency
);
define_scalar_type!(MicScalar, super::MicType, "mic", crate::DataType::Mic);
define_scalar_type!(CfiScalar, super::CfiType, "cfi", crate::DataType::Cfi);
