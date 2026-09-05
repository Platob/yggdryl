//! ASCII values and typed scalar aliases.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::types::typed::define_scalar_type;
use crate::{Result, types};

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
