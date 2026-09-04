//! The registered codes: the identifiers of a trade, each its own datatype.
//!
//! A country, a currency, a market identifier and a CFI classification are
//! not four names for an ASCII width. Each is a distinct logical type over a
//! published registry, each has exactly one storage width, and a column of
//! one is never a column of another however alike their bytes look. So each
//! is a [`DataType`] variant of its own, with its own Arrow extension name,
//! its own typed field and scalar, and its own fixed-width cast path.
//!
//! The value contract is the ASCII contract, unchanged and stated once in
//! [`super::ascii`]: a value is ASCII text - every byte at most `0x7F` - of
//! at most the width in bytes, with no NUL byte; storage pads with trailing
//! `\0` to exactly the width, and every string rendering trims the padding.
//! [`DataType::ascii_width`] answers for a code exactly as it does for a
//! width, so [`DataType::ascii_packed`], [`super::AsciiEnum`] and
//! [`super::AsciiDictionary`] work over a code with nothing added: an enum
//! member is still the integer its value packs into.
//!
//! What a code adds over the width that would hold it is identity and a
//! constant. The identity crosses Arrow as its own extension name, so a
//! currency column reads back a currency and not three anonymous bytes. The
//! constant is the width: it is known at compile time for each code, so the
//! ingest and render paths here are monomorphized per width rather than
//! reading a length out of the datatype on every row.
//!
//! The widths are the ones the standards fix: two bytes for ISO 3166-1's
//! country code, three for ISO 4217's currency, four for ISO 10383's market
//! identifier, and six for ISO 10962's classification. Six is a width no
//! ASCII variant has, which is the point: `cfi` stores the six bytes it is
//! rather than the eight the next width up would pad it to.

use crate::{DataType, Error, Result};

use super::ascii::ascii_text_sized;

/// The Arrow extension name of the country code.
pub(crate) const COUNTRY_EXTENSION_NAME: &str = "yggdryl.country";

/// The Arrow extension name of the currency code.
pub(crate) const CURRENCY_EXTENSION_NAME: &str = "yggdryl.currency";

/// The Arrow extension name of the market identifier code.
pub(crate) const MIC_EXTENSION_NAME: &str = "yggdryl.mic";

/// The Arrow extension name of the classification code.
pub(crate) const CFI_EXTENSION_NAME: &str = "yggdryl.cfi";

/// The storage width of ISO 3166-1's country code.
pub(crate) const COUNTRY_WIDTH: usize = 2;

/// The storage width of ISO 4217's currency code.
pub(crate) const CURRENCY_WIDTH: usize = 3;

/// The storage width of ISO 10383's market identifier code.
pub(crate) const MIC_WIDTH: usize = 4;

/// The storage width of ISO 10962's classification code.
pub(crate) const CFI_WIDTH: usize = 6;

impl DataType {
    /// Every registered code, with its canonical name and storage width.
    ///
    /// The one listing: the parser, the Arrow extension table and every
    /// binding read the codes from here rather than repeating four arms.
    pub const CODES: &'static [(&'static str, DataType, i32)] = &[
        ("country", DataType::Country, COUNTRY_WIDTH as i32),
        ("currency", DataType::Currency, CURRENCY_WIDTH as i32),
        ("mic", DataType::Mic, MIC_WIDTH as i32),
        ("cfi", DataType::Cfi, CFI_WIDTH as i32),
    ];

    /// Creates ISO 3166-1's two-letter country code.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::country(), DataType::Country);
    /// assert_eq!(DataType::country().to_string(), "country");
    /// assert_eq!(DataType::country().ascii_width(), Some(2));
    /// ```
    #[must_use]
    pub const fn country() -> Self {
        Self::Country
    }

    /// Creates ISO 4217's three-letter currency code.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::currency(), DataType::Currency);
    /// assert_eq!(DataType::currency().to_string(), "currency");
    /// assert_eq!(DataType::currency().ascii_width(), Some(3));
    /// ```
    #[must_use]
    pub const fn currency() -> Self {
        Self::Currency
    }

    /// Creates ISO 10383's four-character market identifier code.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::mic(), DataType::Mic);
    /// assert_eq!(DataType::mic().to_string(), "mic");
    /// assert_eq!(DataType::mic().ascii_width(), Some(4));
    /// ```
    #[must_use]
    pub const fn mic() -> Self {
        Self::Mic
    }

    /// Creates ISO 10962's six-character classification code.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::cfi(), DataType::Cfi);
    /// assert_eq!(DataType::cfi().to_string(), "cfi");
    /// // Six bytes, which is a width no ASCII variant has.
    /// assert_eq!(DataType::cfi().ascii_width(), Some(6));
    /// ```
    #[must_use]
    pub const fn cfi() -> Self {
        Self::Cfi
    }

    /// The canonical name of a registered code, `None` for every other type.
    ///
    /// This is the code's identity: it names the datatype, and the Arrow
    /// extension the column carries is `yggdryl.` followed by it.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::Currency.code_name(), Some("currency"));
    /// assert_eq!(DataType::Ascii24.code_name(), None);
    /// ```
    #[must_use]
    pub const fn code_name(&self) -> Option<&'static str> {
        match self {
            Self::Country => Some("country"),
            Self::Currency => Some("currency"),
            Self::Mic => Some("mic"),
            Self::Cfi => Some("cfi"),
            _ => None,
        }
    }

    /// Whether this is one of the registered codes.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert!(DataType::Mic.is_code());
    /// assert!(!DataType::Ascii32.is_code());
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
        DataType::Mic => Some(MIC_EXTENSION_NAME),
        DataType::Cfi => Some(CFI_EXTENSION_NAME),
        _ => None,
    }
}

/// The code one Arrow extension name and storage width import as.
///
/// A name over the wrong width is not this code: the pair has to agree, so a
/// `yggdryl.currency` over four bytes stays the fixed binary it is rather
/// than silently becoming a currency.
pub(crate) fn code_for_extension(name: &str, width: i32) -> Option<DataType> {
    let dtype = match name {
        COUNTRY_EXTENSION_NAME => DataType::Country,
        CURRENCY_EXTENSION_NAME => DataType::Currency,
        MIC_EXTENSION_NAME => DataType::Mic,
        CFI_EXTENSION_NAME => DataType::Cfi,
        _ => return None,
    };
    (dtype.ascii_width() == Some(width)).then_some(dtype)
}

/// Validates bytes as one code value of a width known at compile time.
///
/// The same rule [`super::ascii::ascii_text`] applies, with the width a
/// constant: the length comparison folds and the caller's per-row loop keeps
/// no width to read.
///
/// # Errors
///
/// Returns an error naming the width when the bytes are not ASCII text that
/// fits it.
#[inline]
pub(crate) fn code_text<const WIDTH: usize>(bytes: &[u8]) -> Result<&str> {
    ascii_text_sized(WIDTH, bytes)
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
        DataType::Mic => code_text::<MIC_WIDTH>(bytes),
        DataType::Cfi => code_text::<CFI_WIDTH>(bytes),
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

#[cfg(test)]
mod tests {
    use super::{code_cell_text, code_for_extension, code_text};
    use crate::DataType;

    #[test]
    fn every_code_names_itself_and_its_width() {
        for (name, dtype, width) in DataType::CODES {
            assert_eq!(dtype.code_name(), Some(*name));
            assert_eq!(dtype.ascii_width(), Some(*width));
            assert_eq!(dtype.to_string(), *name);
            assert_eq!(DataType::from_str(name).unwrap(), *dtype);
            assert!(dtype.is_code());
        }
        assert!(!DataType::Ascii24.is_code());
    }

    #[test]
    fn an_extension_name_needs_its_own_width() {
        assert_eq!(
            code_for_extension("yggdryl.currency", 3),
            Some(DataType::Currency)
        );
        assert_eq!(code_for_extension("yggdryl.currency", 4), None);
        assert_eq!(code_for_extension("yggdryl.ascii", 3), None);
    }

    #[test]
    fn a_code_holds_ascii_text_up_to_its_width() {
        assert_eq!(code_text::<3>(b"USD").unwrap(), "USD");
        assert_eq!(code_text::<3>(b"US\0").unwrap(), "US");
        assert_eq!(code_text::<6>(b"ESVUFR").unwrap(), "ESVUFR");
        let refused = code_text::<3>(b"EURO").unwrap_err().to_string();
        assert!(refused.contains("at most 3 bytes"), "{refused}");
    }

    #[test]
    fn a_cell_is_validated_at_the_code_width() {
        assert_eq!(code_cell_text(&DataType::Currency, b"USD").unwrap(), "USD");
        assert_eq!(code_cell_text(&DataType::Country, b"FR").unwrap(), "FR");
        assert_eq!(code_cell_text(&DataType::Cfi, b"ESVUFR").unwrap(), "ESVUFR");
        let refused = code_cell_text(&DataType::Country, b"USD")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 2 bytes"), "{refused}");
        let wrong = code_cell_text(&DataType::Ascii24, b"USD")
            .unwrap_err()
            .to_string();
        assert!(wrong.contains("registered codes"), "{wrong}");
    }
}
