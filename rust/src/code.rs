//! What every registered code shares: the trait, the two builders,
//! and the Arrow extension name a code is recognised by.

use smol_str::SmolStr;

use crate::ascii::ascii_text_sized;
use crate::value::family_value;
use crate::{
    BLOOMBERG_EXTENSION_NAME, CFI_EXTENSION_NAME, COUNTRY_EXTENSION_NAME, CURRENCY_EXTENSION_NAME,
    CUSIP_EXTENSION_NAME, ISIN_EXTENSION_NAME, MIC_EXTENSION_NAME, SEDOL_EXTENSION_NAME,
    SIDE_EXTENSION_NAME, STATE_EXTENSION_NAME, TIMEINFORCE_EXTENSION_NAME,
};
use crate::{
    BLOOMBERG_WIDTH, CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, CUSIP_WIDTH, ISIN_WIDTH, MIC_WIDTH,
    SEDOL_WIDTH, SIDE_WIDTH, STATE_WIDTH, TIMEINFORCE_WIDTH,
};
use crate::{
    BloombergCode, CfiCode, Country, Currency, CusipCode, IsinCode, MicCode, SedolCode, Side,
    State, TimeInForce,
};
use crate::{DataType, Error, Result};

// ------------------------------------------------------------------------
// The registered codes' values: an identity, held in the width it fixes.
//
// A code is at most twelve US-ASCII bytes, so every value here lives inside
// the crate's compact string and never touches the heap: the text is
// validated once when it is built and never changed after. Equality, order
// and hashing read the text; the family enum keeps the identity in front of
// it, so a currency is never a country however alike their bytes look.
// ------------------------------------------------------------------------

family_value!(
    /// The code family as one value: any of the eleven registered codes.
    ///
    /// ```
    /// use yggdryl::{Code, Currency, DataType, FamilyValue, Scalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let held = Code::from(Currency::new("EUR")?);
    /// assert_eq!(held.dtype()?, DataType::Currency);
    /// assert_eq!(held.clone().into_scalar(), Scalar::Currency(Currency::new("EUR")?));
    /// assert_eq!(Code::from_scalar(&Scalar::Currency(Currency::new("EUR")?)), Some(held));
    /// assert_eq!(Code::from_scalar(&Scalar::from("EUR")), None);
    /// # Ok(())
    /// # }
    /// ```
    Code, Code, [Country, Currency, MicCode, CfiCode, Side, State, TimeInForce, IsinCode, CusipCode, SedolCode, BloombergCode]
);

macro_rules! code_leaf {
    ($name:ident, $width:expr) => {
        #[doc = concat!("One validated `", stringify!($name), "` code.")]
        #[repr(transparent)]
        #[derive(
            Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(SmolStr);

        impl $name {
            /// Validate and construct this registered code.
            ///
            /// # Errors
            ///
            /// Returns an error naming the width when the text is not ASCII
            /// text that fits it.
            pub fn new(value: impl AsRef<str>) -> Result<Self> {
                let value = crate::ascii_text($width, value.as_ref().as_bytes())?;
                Ok(Self(SmolStr::new(value)))
            }

            /// Borrow the validated code.
            #[must_use]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }

            /// Borrow the shared storage without copying the code.
            #[must_use]
            pub const fn storage(&self) -> &SmolStr {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

/// The value one character of a securities identifier reads as: a digit as
/// itself, a letter as ten plus its position in the alphabet, from `A` at
/// ten to `Z` at thirty-five.
///
/// The one reading CUSIP and SEDOL share, over an upper-cased byte; anything
/// else is not part of an identifier.
pub(crate) const fn identifier_value(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some((byte - b'0') as u32),
        b'A'..=b'Z' => Some((byte - b'A') as u32 + 10),
        _ => None,
    }
}

macro_rules! code_value {
    ($leaf:ident, $id:ident, $width:expr $(, merge = $merge:expr)?) => {
        impl Value for $leaf {

            fn dtype(&self) -> Result<DataType> {
                Ok(DataType::$id)
            }

            fn into_scalar(self) -> Scalar {
                Scalar::$leaf(self)
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::$leaf(value) => Some(value),
                    _ => None,
                }
            }
        }

        impl CodeValue for $leaf {
            const WIDTH: usize = $width;

            fn as_str(&self) -> &str {
                <$leaf>::as_str(self)
            }

            fn storage(&self) -> &SmolStr {
                <$leaf>::storage(self)
            }

            $(
                fn merge_with(self, other: &Self) -> Self {
                    $merge(self, other)
                }
            )?
        }

        impl From<$leaf> for Scalar {
            fn from(value: $leaf) -> Self {
                Self::$leaf(value)
            }
        }
    };
}

impl DataType {
    /// The canonical name of a registered code, `None` for every other type.
    ///
    /// This is the code's identity: it names the datatype, and the Arrow
    /// extension the column carries is `yggdryl.` followed by it.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::Currency.code_name(), Some("currency"));
    /// assert_eq!(DataType::fixed_ascii(3)?.code_name(), None);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn code_name(&self) -> Option<&'static str> {
        match self {
            Self::Country => Some("country"),
            Self::Currency => Some("currency"),
            Self::MicCode => Some("mic"),
            Self::CfiCode => Some("cfi"),
            Self::IsinCode => Some("isin"),
            Self::CusipCode => Some("cusip"),
            Self::SedolCode => Some("sedol"),
            Self::Side => Some("side"),
            Self::State => Some("state"),
            Self::TimeInForce => Some("timeinforce"),
            Self::BloombergCode => Some("bloomberg"),
            _ => None,
        }
    }

    /// The most bytes a registered code's value may be, `None` for every
    /// other type.
    ///
    /// The number its standard fixes, and a maximum rather than a layout: a
    /// code stores as the text it is, so `fixed_byte_width` answers `None`
    /// and this answers the bound the value rule holds a cell to.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::Currency.code_width(), Some(3));
    /// assert_eq!(DataType::Currency.fixed_byte_width(), None);
    /// assert_eq!(DataType::fixed_ascii(3)?.code_width(), None);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn code_width(&self) -> Option<usize> {
        self.id().code_width()
    }

    /// Whether this is one of the registered codes.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert!(DataType::MicCode.is_code());
    /// assert!(!DataType::ascii().is_code());
    /// ```
    #[must_use]
    pub const fn is_code(&self) -> bool {
        self.code_name().is_some()
    }
}

/// The Arrow extension name one registered code rides.
pub(crate) const fn code_extension_name(dtype: &DataType) -> Option<&'static str> {
    match dtype {
        DataType::Country => Some(COUNTRY_EXTENSION_NAME),
        DataType::Currency => Some(CURRENCY_EXTENSION_NAME),
        DataType::MicCode => Some(MIC_EXTENSION_NAME),
        DataType::CfiCode => Some(CFI_EXTENSION_NAME),
        DataType::BloombergCode => Some(BLOOMBERG_EXTENSION_NAME),
        DataType::IsinCode => Some(ISIN_EXTENSION_NAME),
        DataType::CusipCode => Some(CUSIP_EXTENSION_NAME),
        DataType::SedolCode => Some(SEDOL_EXTENSION_NAME),
        DataType::Side => Some(SIDE_EXTENSION_NAME),
        DataType::State => Some(STATE_EXTENSION_NAME),
        DataType::TimeInForce => Some(TIMEINFORCE_EXTENSION_NAME),
        _ => None,
    }
}

/// The code one Arrow extension name imports as.
///
/// The name alone, because a code's storage is Arrow's `Utf8` and the caller
/// has already checked it: a `yggdryl.currency` over anything else stays the
/// storage it is rather than silently becoming a currency.
pub(crate) fn code_for_extension(name: &str) -> Option<DataType> {
    match name {
        COUNTRY_EXTENSION_NAME => Some(DataType::Country),
        CURRENCY_EXTENSION_NAME => Some(DataType::Currency),
        MIC_EXTENSION_NAME => Some(DataType::MicCode),
        CFI_EXTENSION_NAME => Some(DataType::CfiCode),
        BLOOMBERG_EXTENSION_NAME => Some(DataType::BloombergCode),
        ISIN_EXTENSION_NAME => Some(DataType::IsinCode),
        CUSIP_EXTENSION_NAME => Some(DataType::CusipCode),
        SEDOL_EXTENSION_NAME => Some(DataType::SedolCode),
        SIDE_EXTENSION_NAME => Some(DataType::Side),
        STATE_EXTENSION_NAME => Some(DataType::State),
        TIMEINFORCE_EXTENSION_NAME => Some(DataType::TimeInForce),
        _ => None,
    }
}

/// Validates bytes as one code value of a width known at compile time.
///
/// The same rule [`ascii_text`] applies, with the width a
/// constant: the length comparison folds and the caller's per-row loop keeps
/// no width to read.
///
/// # Errors
///
/// Returns an error naming the width when the bytes are not ASCII text that
/// fits it.
#[inline]
pub(crate) fn code_text<const WIDTH: usize>(bytes: &[u8]) -> Result<&str> {
    ascii_text_sized(Some(WIDTH), bytes)
}

/// The trimmed text one code cell holds, validated at the code's own width.
///
/// The one dispatcher every per-value path goes through: it matches the code
/// once and then runs a validator whose width is a constant, so no arm reads
/// a length out of the datatype while walking rows.
///
/// # Errors
///
/// Returns an error naming the type when `dtype` is not a code, and one
/// naming the width when the bytes do not fit it.
#[inline]
pub(crate) fn code_cell_text<'a>(dtype: &DataType, bytes: &'a [u8]) -> Result<&'a str> {
    match dtype {
        DataType::Country => code_text::<COUNTRY_WIDTH>(bytes),
        DataType::Currency => code_text::<CURRENCY_WIDTH>(bytes),
        DataType::MicCode => code_text::<MIC_WIDTH>(bytes),
        DataType::CfiCode => code_text::<CFI_WIDTH>(bytes),
        DataType::BloombergCode => code_text::<BLOOMBERG_WIDTH>(bytes),
        DataType::IsinCode => code_text::<ISIN_WIDTH>(bytes),
        DataType::CusipCode => code_text::<CUSIP_WIDTH>(bytes),
        DataType::SedolCode => code_text::<SEDOL_WIDTH>(bytes),
        DataType::Side => code_text::<SIDE_WIDTH>(bytes),
        DataType::State => code_text::<STATE_WIDTH>(bytes),
        DataType::TimeInForce => code_text::<TIMEINFORCE_WIDTH>(bytes),
        _ => Err(code_refusal(dtype)),
    }
}

/// The refusal a datatype that is not a code answers with.
pub(crate) fn code_refusal(dtype: &DataType) -> Error {
    Error::InvalidDataType {
        kind: "code",
        reason: crate::text::expected_got(
            format_args!("one of the registered codes"),
            format_args!("{dtype}"),
        ),
    }
}

pub(crate) use code_leaf;
pub(crate) use code_value;

/// One spelling folded the way every name in this crate folds.
pub(crate) fn folded_spelling(spelling: &str) -> SmolStr {
    let mut held = smol_str::SmolStrBuilder::new();
    for byte in spelling.bytes() {
        if matches!(byte, b'_' | b'-' | b' ') {
            continue;
        }
        held.push(char::from(byte.to_ascii_lowercase()));
    }
    held.finish()
}

// ------------------------------------------------------------------------
// Arrow projection: a code is the ASCII text it is.
// ------------------------------------------------------------------------

mod arrow {
    use arrow_schema::DataType as ArrowDataType;
    use smol_str::format_smolstr;

    use super::code_extension_name;
    use crate::invalid;
    use crate::{DataType, Result};

    /// The Arrow storage one registered code lays out.
    ///
    /// A code is the ASCII text it is, so it rides Arrow's own text layout and
    /// the `yggdryl.{country,currency,mic,...}` name beside it carries the
    /// identity: three bytes under `yggdryl.currency` read back a currency.
    ///
    /// # Errors
    ///
    /// Returns an error when the datatype is not a registered code.
    pub(crate) fn arrow_storage(dtype: &DataType) -> Result<ArrowDataType> {
        if code_extension_name(dtype).is_some() {
            return Ok(ArrowDataType::Utf8);
        }
        Err(invalid(
            "code",
            format_smolstr!("expected a registered code datatype, got {dtype}"),
        ))
    }
}

pub(crate) use arrow::arrow_storage as code_arrow_storage;
