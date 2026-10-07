//! What every registered code shares: the trait, the two builders,
//! and the Arrow extension name a code is recognised by.

use smol_str::SmolStr;

use crate::ascii::ascii_text_sized;
use crate::{
    BBG_EXTENSION_NAME, BIC_EXTENSION_NAME, CCY_EXTENSION_NAME, CFI_EXTENSION_NAME,
    COUNTRY_EXTENSION_NAME, CUSIP_EXTENSION_NAME, DTI_EXTENSION_NAME, ELF_EXTENSION_NAME,
    FIGI_EXTENSION_NAME, FISN_EXTENSION_NAME, FOREX_EXTENSION_NAME, ISIN_EXTENSION_NAME,
    LEI_EXTENSION_NAME, MIC_EXTENSION_NAME, RIC_EXTENSION_NAME, SEDOL_EXTENSION_NAME,
    UNIT_EXTENSION_NAME,
};
use crate::{
    BBG_WIDTH, BIC_WIDTH, CCY_WIDTH, CFI_WIDTH, COUNTRY_WIDTH, CUSIP_WIDTH, DTI_WIDTH, ELF_WIDTH,
    FIGI_WIDTH, FISN_WIDTH, FOREX_WIDTH, ISIN_WIDTH, LEI_WIDTH, MIC_WIDTH, RIC_WIDTH, SEDOL_WIDTH,
    UNIT_WIDTH,
};
use crate::{DataType, Error, Result};

// ------------------------------------------------------------------------
// The registered codes' values: an identity, held in the width it fixes.
//
// The short codes live inside the crate's compact string and never touch the
// heap; Bloomberg's wider text uses the same value owner. The text is validated
// once when it is built and never changed after. Equality, order and hashing
// read the text; the `Scalar` variant keeps the identity in front of it, so
// a currency is never a country however alike their bytes look.
// ------------------------------------------------------------------------

macro_rules! code_leaf {
    // The one-line doc every code takes unless it states its own.
    ($name:ident, $width:expr) => {
        $crate::code::code_leaf!(
            $name,
            $width,
            doc = concat!("One validated `", stringify!($name), "` code.")
        );
    };
    ($name:ident, $width:expr, doc = $doc:expr) => {
        #[doc = $doc]
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

/// `value` checked as US-ASCII within a code's width, upper-cased, and held
/// to the code's shape: the one door every case-folding code's `new` runs.
///
/// `refusal` reads the upper-cased spelling and answers why it is not the
/// code's shape, or `None` where it is.
///
/// # Errors
///
/// The width refusal [`ascii_text`](crate::ascii_text) gives, else the
/// shape's own reason under the code's name, with the spelling it saw.
pub(crate) fn folded_code<const WIDTH: usize>(
    kind: &'static str,
    value: &str,
    refusal: impl Fn(&str) -> Option<&'static str>,
) -> Result<SmolStr> {
    let value = crate::ascii_text(WIDTH, value.as_bytes())?;
    let mut bytes = [0_u8; WIDTH];
    for (target, byte) in bytes.iter_mut().zip(value.bytes()) {
        *target = byte.to_ascii_uppercase();
    }
    let folded = std::str::from_utf8(&bytes[..value.len()]).expect("validated ASCII");
    if let Some(reason) = refusal(folded) {
        return Err(Error::InvalidDataType {
            kind,
            reason: smol_str::format_smolstr!("{reason}, got {value:?}"),
        });
    }
    Ok(SmolStr::new(folded))
}

macro_rules! code_value {
    ($leaf:ident, $id:ident, $width:expr $(, merge = $merge:expr)? $(, rank = $rank:expr, max_rank = $max_rank:expr)?) => {
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

            $(
                const MAX_RANK: u8 = $max_rank;

                fn rank(&self) -> u8 {
                    $rank(self)
                }
            )?
        }

        impl From<$leaf> for Scalar {
            fn from(value: $leaf) -> Self {
                Self::$leaf(value)
            }
        }

        #[doc = concat!(
            "Reads a `", stringify!($leaf), "` as its `new` does: `str::parse` is that door."
        )]
        #[doc = $crate::code::code_parse_example!($leaf)]
        impl ::std::str::FromStr for $leaf {
            type Err = $crate::Error;

            fn from_str(value: &str) -> ::std::result::Result<Self, Self::Err> {
                <$leaf>::new(value)
            }
        }

        /// Borrows the validated code, as `as_str` does.
        impl AsRef<str> for $leaf {
            fn as_ref(&self) -> &str {
                <$leaf>::as_str(self)
            }
        }
    };
}

/// The example a code's `FromStr` carries: the currency's alone, so one
/// page shows the two conversions every code answers and the doctest runs
/// once rather than once per code.
macro_rules! code_parse_example {
    (Ccy) => {
        r#"

```
use yggdryl::Ccy;

# fn main() -> yggdryl::Result<()> {
let usd: Ccy = "USD".parse()?;
assert_eq!(usd, Ccy::new("USD")?);
assert_eq!(AsRef::<str>::as_ref(&usd), "USD");
// The width the code fixes refuses the same text `new` refuses.
assert!("TOOLONGCCY".parse::<Ccy>().is_err());
// Any code reads where text is asked for.
fn spelled(code: impl AsRef<str>) -> usize {
    code.as_ref().len()
}
assert_eq!(spelled(usd), 3);
# Ok(())
# }
```"#
    };
    ($other:ident) => {
        ""
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
    /// assert_eq!(DataType::Ccy.code_name(), Some("ccy"));
    /// assert_eq!(DataType::fixed_ascii(3)?.code_name(), None);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn code_name(&self) -> Option<&'static str> {
        match self {
            Self::Country => Some("country"),
            Self::Ccy => Some("ccy"),
            Self::Mic => Some("mic"),
            Self::Cfi => Some("cfi"),
            Self::Isin => Some("isin"),
            Self::Cusip => Some("cusip"),
            Self::Sedol => Some("sedol"),
            Self::Bbg => Some("bbg"),
            Self::Ric => Some("ric"),
            Self::Figi => Some("figi"),
            Self::Unit => Some("unit"),
            Self::Forex => Some("forex"),
            Self::Lei => Some("lei"),
            Self::Bic => Some("bic"),
            Self::Elf => Some("elf"),
            Self::Dti => Some("dti"),
            Self::Fisn => Some("fisn"),
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
    /// assert_eq!(DataType::Ccy.code_width(), Some(8));
    /// assert_eq!(DataType::Ccy.fixed_byte_width(), None);
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
    /// assert!(DataType::Mic.is_code());
    /// assert!(!DataType::ascii().is_code());
    /// ```
    #[must_use]
    pub const fn is_code(&self) -> bool {
        crate::DataTypeKind::Code.contains(self.id())
    }
}

/// The code one Arrow extension name imports as.
///
/// The name alone, because a code's storage is Arrow's `Utf8` and the caller
/// has already checked it: a `yggdryl.ccy` over anything else stays the
/// storage it is rather than silently becoming a currency.
pub(crate) fn code_for_extension(name: &str) -> Option<DataType> {
    match name {
        COUNTRY_EXTENSION_NAME => Some(DataType::Country),
        CCY_EXTENSION_NAME => Some(DataType::Ccy),
        MIC_EXTENSION_NAME => Some(DataType::Mic),
        CFI_EXTENSION_NAME => Some(DataType::Cfi),
        BBG_EXTENSION_NAME => Some(DataType::Bbg),
        RIC_EXTENSION_NAME => Some(DataType::Ric),
        FIGI_EXTENSION_NAME => Some(DataType::Figi),
        ISIN_EXTENSION_NAME => Some(DataType::Isin),
        CUSIP_EXTENSION_NAME => Some(DataType::Cusip),
        SEDOL_EXTENSION_NAME => Some(DataType::Sedol),
        UNIT_EXTENSION_NAME => Some(DataType::Unit),
        FOREX_EXTENSION_NAME => Some(DataType::Forex),
        LEI_EXTENSION_NAME => Some(DataType::Lei),
        BIC_EXTENSION_NAME => Some(DataType::Bic),
        ELF_EXTENSION_NAME => Some(DataType::Elf),
        DTI_EXTENSION_NAME => Some(DataType::Dti),
        FISN_EXTENSION_NAME => Some(DataType::Fisn),
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
        DataType::Ccy => code_text::<CCY_WIDTH>(bytes),
        DataType::Mic => code_text::<MIC_WIDTH>(bytes),
        DataType::Cfi => code_text::<CFI_WIDTH>(bytes),
        DataType::Bbg => code_text::<BBG_WIDTH>(bytes),
        DataType::Ric => code_text::<RIC_WIDTH>(bytes),
        DataType::Figi => code_text::<FIGI_WIDTH>(bytes),
        DataType::Isin => code_text::<ISIN_WIDTH>(bytes),
        DataType::Cusip => code_text::<CUSIP_WIDTH>(bytes),
        DataType::Sedol => code_text::<SEDOL_WIDTH>(bytes),
        DataType::Unit => code_text::<UNIT_WIDTH>(bytes),
        DataType::Forex => code_text::<FOREX_WIDTH>(bytes),
        DataType::Lei => code_text::<LEI_WIDTH>(bytes),
        DataType::Bic => code_text::<BIC_WIDTH>(bytes),
        DataType::Elf => code_text::<ELF_WIDTH>(bytes),
        DataType::Dti => code_text::<DTI_WIDTH>(bytes),
        DataType::Fisn => code_text::<FISN_WIDTH>(bytes),
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
pub(crate) use code_parse_example;
pub(crate) use code_value;

/// Whether `text` is one of the spellings that state no value.
///
/// A feed writes `null`, `none`, `n/a` or `[n/a]` where it has nothing to
/// say, and an identifier map or a security identifier reads any of them,
/// in any case, as the absence they are rather than as a value.
pub(crate) fn is_null_like(text: &str) -> bool {
    text.is_empty()
        || ["null", "none", "n/a", "[n/a]"]
            .iter()
            .any(|null| text.eq_ignore_ascii_case(null))
}

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

    use crate::invalid;
    use crate::{DataType, Result};

    /// The Arrow storage one registered code lays out.
    ///
    /// A code is the ASCII text it is, so it rides Arrow's own text layout and
    /// the `yggdryl.{country,ccy,mic,...}` name beside it carries the
    /// identity: the same text under `yggdryl.ccy` reads back a currency.
    ///
    /// # Errors
    ///
    /// Returns an error when the datatype is not a registered code.
    pub(crate) fn arrow_storage(dtype: &DataType) -> Result<ArrowDataType> {
        if dtype.is_code() {
            return Ok(ArrowDataType::Utf8);
        }
        Err(invalid(
            "code",
            format_smolstr!("expected a registered code datatype, got {dtype}"),
        ))
    }
}

pub(crate) use arrow::arrow_storage as code_arrow_storage;

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/string.rs` pins and a caller cannot reach.
    //!
    //! `code_for_extension`, `code_text` and `code_cell_text` are the three
    //! doors every registered code goes through, and a caller sees only the
    //! datatype they answer for. Each is behind a forwarder, so nothing here
    //! is more public than it was.

    use crate::{DataType, Result};

    /// The code a registered Arrow extension name answers for.
    #[must_use]
    pub fn code_for_extension(name: &str) -> Option<DataType> {
        super::code_for_extension(name)
    }

    /// Read a code's ASCII text out of at most `WIDTH` bytes.
    pub fn code_text<const WIDTH: usize>(bytes: &[u8]) -> Result<&str> {
        super::code_text::<WIDTH>(bytes)
    }

    /// Read one cell's text at the width `dtype`'s standard fixes.
    pub fn code_cell_text<'a>(dtype: &DataType, bytes: &'a [u8]) -> Result<&'a str> {
        super::code_cell_text(dtype, bytes)
    }
}
