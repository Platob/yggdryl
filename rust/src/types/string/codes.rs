//! The registered codes: the identifiers of a trade, each its own datatype.
//!
//! A country, a currency, a market identifier and a CFI classification are
//! not four names for an ASCII width. Each is a distinct logical type over a
//! published registry, each has exactly one storage width, and a column of
//! one is never a column of another however alike their bytes look. So each
//! is a [`DataType`] variant of its own, with its own Arrow extension name,
//! its own typed field and scalar, and its own fixed-width cast path.
//!
//! The value contract is the fixed US-ASCII string's, stated once here: a
//! value is ASCII text - every byte at most `0x7F` - of at most the width in
//! bytes, with no NUL byte; storage pads with trailing `\0` to exactly the
//! width, and every string rendering trims the padding.
//! [`DataType::fixed_byte_width`] answers for a code exactly as it does for a
//! fixed string, so [`DataType::ascii_packed`] and [`super::StringEnum`] work
//! over a code with nothing added: an enum member is still the integer its
//! value packs into.
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
//! identifier, and six for ISO 10962's classification.

use smol_str::{SmolStr, format_smolstr};

use super::Str;
use crate::{DataType, Error, Result, Scalar};

/// The Arrow extension name of the country code.
pub(crate) const COUNTRY_EXTENSION_NAME: &str = "yggdryl.country";

/// The Arrow extension name of the currency code.
pub(crate) const CURRENCY_EXTENSION_NAME: &str = "yggdryl.currency";

/// The Arrow extension name of the market identifier code.
pub(crate) const MIC_EXTENSION_NAME: &str = "yggdryl.mic";

/// The Arrow extension name of the classification code.
pub(crate) const CFI_EXTENSION_NAME: &str = "yggdryl.cfi";

/// The Arrow extension name of the securities identification number.
pub(crate) const ISIN_EXTENSION_NAME: &str = "yggdryl.isin";

/// The Arrow extension name of FIX's side of a trade.
pub(crate) const SIDE_EXTENSION_NAME: &str = "yggdryl.side";

/// The Arrow extension name of a captured line's direction.
pub(crate) const DIRECTION_EXTENSION_NAME: &str = "yggdryl.msgdirection";

/// The Arrow extension name of a thing's state.
pub(crate) const STATE_EXTENSION_NAME: &str = "yggdryl.state";

/// The Arrow extension name of how long an order stands.
pub(crate) const TIMEINFORCE_EXTENSION_NAME: &str = "yggdryl.timeinforce";

/// The storage width of ISO 3166-1's country code.
pub(crate) const COUNTRY_WIDTH: usize = 2;

/// The storage width of ISO 4217's currency code.
pub(crate) const CURRENCY_WIDTH: usize = 3;

/// The storage width of ISO 10383's market identifier code.
pub(crate) const MIC_WIDTH: usize = 4;

/// The storage width of ISO 10962's classification code.
pub(crate) const CFI_WIDTH: usize = 6;

/// The storage width of an ISO 6166 securities identification number.
///
/// Two letters of prefix, nine of national number and one check digit:
/// twelve, which the standard fixes and the check digit closes.
pub(crate) const ISIN_WIDTH: usize = 12;

/// The storage width of FIX's side of a trade.
///
/// The standard's values are one character and a venue's are not going to be
/// five, so four is room without waste.
pub(crate) const SIDE_WIDTH: usize = 4;

/// The storage width of a captured line's direction.
///
/// `SENT` and `RECV`, spelled out rather than abbreviated because the stored
/// bytes are what a reader sees.
pub(crate) const DIRECTION_WIDTH: usize = 4;

/// The storage width of a thing's state.
///
/// Two decimal digits of rank and up to eight name bytes. Ten because the
/// rank has to leave room for a name a person can read, and because widening
/// later would change a discriminant, which is a wire contract.
pub(crate) const STATE_WIDTH: usize = 10;

/// The storage width of how long an order stands.
///
/// The standard's values are one character; eight leaves room for venue codes.
pub(crate) const TIMEINFORCE_WIDTH: usize = 8;

impl DataType {
    /// Every registered code, with its canonical name and storage width.
    ///
    /// The one listing: the parser, the Arrow extension table and every
    /// binding read the codes from here rather than repeating four arms.
    pub const CODES: &'static [(&'static str, DataType, usize)] = &[
        ("country", DataType::Country, COUNTRY_WIDTH),
        ("currency", DataType::Currency, CURRENCY_WIDTH),
        ("mic", DataType::Mic, MIC_WIDTH),
        ("cfi", DataType::Cfi, CFI_WIDTH),
        ("isin", DataType::Isin, ISIN_WIDTH),
        ("side", DataType::Side, SIDE_WIDTH),
        ("msgdirection", DataType::MsgDirection, DIRECTION_WIDTH),
        ("state", DataType::State, STATE_WIDTH),
        ("timeinforce", DataType::TimeInForce, TIMEINFORCE_WIDTH),
    ];

    /// Creates ISO 3166-1's two-letter country code.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::country(), DataType::Country);
    /// assert_eq!(DataType::country().to_string(), "country");
    /// assert_eq!(DataType::country().fixed_byte_width(), Some(2));
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
    /// assert_eq!(DataType::currency().fixed_byte_width(), Some(3));
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
    /// assert_eq!(DataType::mic().fixed_byte_width(), Some(4));
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
    /// assert_eq!(DataType::cfi().fixed_byte_width(), Some(6));
    /// ```
    #[must_use]
    pub const fn cfi() -> Self {
        Self::Cfi
    }

    /// Creates ISO 6166's twelve-character securities identification number.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::isin(), DataType::Isin);
    /// assert_eq!(DataType::isin().to_string(), "isin");
    /// assert_eq!(DataType::isin().fixed_byte_width(), Some(12));
    /// ```
    #[must_use]
    pub const fn isin() -> Self {
        Self::Isin
    }

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
            Self::Mic => Some("mic"),
            Self::Cfi => Some("cfi"),
            Self::Isin => Some("isin"),
            Self::Side => Some("side"),
            Self::MsgDirection => Some("msgdirection"),
            Self::State => Some("state"),
            Self::TimeInForce => Some("timeinforce"),
            _ => None,
        }
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
        DataType::Isin => Some(ISIN_EXTENSION_NAME),
        DataType::Side => Some(SIDE_EXTENSION_NAME),
        DataType::MsgDirection => Some(DIRECTION_EXTENSION_NAME),
        DataType::State => Some(STATE_EXTENSION_NAME),
        DataType::TimeInForce => Some(TIMEINFORCE_EXTENSION_NAME),
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
        ISIN_EXTENSION_NAME => DataType::Isin,
        SIDE_EXTENSION_NAME => DataType::Side,
        DIRECTION_EXTENSION_NAME => DataType::MsgDirection,
        STATE_EXTENSION_NAME => DataType::State,
        TIMEINFORCE_EXTENSION_NAME => DataType::TimeInForce,
        // The name this datatype was first published under, so a column
        // written before the rename still reads as what it is.
        "yggdryl.direction" => DataType::MsgDirection,
        _ => return None,
    };
    (usize::try_from(width).is_ok_and(|width| dtype.fixed_byte_width() == Some(width)))
        .then_some(dtype)
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
        DataType::Mic => code_text::<MIC_WIDTH>(bytes),
        DataType::Cfi => code_text::<CFI_WIDTH>(bytes),
        DataType::Isin => code_text::<ISIN_WIDTH>(bytes),
        DataType::Side => code_text::<SIDE_WIDTH>(bytes),
        DataType::MsgDirection => code_text::<DIRECTION_WIDTH>(bytes),
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

impl DataType {
    /// The integer an ASCII value packs into: its storage bytes, big-endian.
    ///
    /// Storage pads with trailing NUL to the width, so the packed integer
    /// orders exactly as the text does and is the same integer in every
    /// process - what a stable hash and a portable enum member both need. An
    /// ASCII byte is at most `0x7F`, so the sign bit is never set and the
    /// value is never negative: four bytes fill an `i32`, eight an `i64`, and
    /// sixteen the whole `i128`.
    ///
    /// Only a fixed US-ASCII width has one. A variable string takes a value
    /// of any length, so there is no integer its bytes always fit; another
    /// charset can set the sign bit; and a width above sixteen bytes
    /// outgrows the widest integer this crate carries. All three are refused
    /// rather than truncated.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// // `USD` stores as `USD\0` under `fixed_ascii(4)`, which is that
    /// // big-endian `i32`; under `currency` it is the three bytes alone.
    /// let ccy = DataType::fixed_ascii(4)?;
    /// assert_eq!(ccy.ascii_packed(b"USD")?, 0x5553_4400);
    /// assert_eq!(ccy.ascii_packed(b"USD\0")?, 0x5553_4400);
    /// assert_eq!(ccy.ascii_value(0x5553_4400)?, "USD");
    /// assert_eq!(DataType::Currency.ascii_packed(b"USD")?, 0x0055_5344);
    ///
    /// // The order of the integers is the order of the text.
    /// assert!(ccy.ascii_packed(b"EUR")? < ccy.ascii_packed(b"USD")?);
    ///
    /// // Twelve bytes need 96 bits, and sixteen the whole `i128`.
    /// let twelve = DataType::fixed_ascii(12)?;
    /// let isin = twelve.ascii_packed(b"US0378331005")?;
    /// assert_eq!(twelve.ascii_value(isin)?, "US0378331005");
    /// assert!(isin > i128::from(u64::MAX));
    ///
    /// assert!(ccy.ascii_packed(b"EURO!").is_err());
    /// assert!(DataType::ascii().ascii_packed(b"USD").is_err());
    /// assert!(DataType::fixed_ascii(17)?.ascii_packed(b"USD").is_err());
    /// assert!(DataType::fixed_utf8(4)?.ascii_packed(b"USD").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the accepted datatypes when this has no
    /// US-ASCII width or its width outgrows an `i128`, and one naming the
    /// width when `value` is not ASCII text that fits it.
    pub fn ascii_packed(&self, value: &[u8]) -> Result<i128> {
        let width = self.packed_width()?;
        let text = ascii_text(width, value)?;
        let mut slot = [0_u8; 16];
        // The text fits the width, and the width fits the slot.
        slot[..text.len()].copy_from_slice(text.as_bytes());
        Ok(i128::from_be_bytes(slot) >> (8 * (16 - width as i128)))
    }

    /// The ASCII value a packed integer carries, without its padding.
    ///
    /// The inverse of [`Self::ascii_packed`], and it refuses exactly what that
    /// refuses: an integer wider than the width, a negative one, and one whose
    /// bytes are not the padded storage of an ASCII value.
    ///
    /// # Errors
    ///
    /// Returns an error naming the accepted widths when this is not one, and
    /// one naming the width when `packed` is not the storage of an ASCII value
    /// of it.
    pub fn ascii_value(&self, packed: i128) -> Result<Str> {
        let width = self.packed_width()?;
        let bytes = packed.to_be_bytes();
        let (above, stored) = bytes.split_at(bytes.len() - width);
        if above.iter().any(|byte| *byte != 0) {
            return Err(ascii_refusal(
                Some(width),
                format_smolstr!("the integer {packed}, which is wider than the width"),
            ));
        }
        let text = ascii_text(width, stored)?;
        Str::new(text).try_with_parameters(match self {
            Self::String(parameters) => *parameters,
            // A code is its own fixed US-ASCII slot.
            _ => super::StringParameters::ascii(super::StringLayout::FixedString)
                .try_with_bound(width as u32)?,
        })
    }

    /// The width a packed integer may be built from, refusing the rest.
    fn packed_width(&self) -> Result<usize> {
        let ascii = self.is_code()
            || self
                .string_parameters()
                .is_some_and(|parameters| parameters.charset() == crate::Charset::Ascii);
        match self.fixed_byte_width() {
            Some(width) if ascii && width <= PACKED_LIMIT => Ok(width),
            _ => Err(ascii_values_refusal(self)),
        }
    }
}

/// The widest fixed ASCII storage one `i128` holds, in bytes.
const PACKED_LIMIT: usize = 16;

/// Validates bytes as an ASCII value of at most `width` bytes and trims the
/// trailing NUL padding.
///
/// The one validator every arm calls: field validation and canonicalization,
/// Arrow ingest, and casts all answer the same trimmed text or the same
/// refusal naming the width.
///
/// # Errors
///
/// Returns an error naming the width when the trimmed bytes hold a NUL, a
/// non-ASCII byte, or more than `width` bytes.
pub(crate) fn ascii_text(width: usize, bytes: &[u8]) -> Result<&str> {
    ascii_text_sized(Some(width), bytes)
}

/// [`ascii_text`] over the width the caller already holds, if there is one.
///
/// The one body every shape runs: a fixed width passes its length, and a
/// registered code passes its constant, which lets the length check fold at
/// each code's call site.
#[inline]
pub(crate) fn ascii_text_sized(width: Option<usize>, bytes: &[u8]) -> Result<&str> {
    let text = crate::types::string::trim_padding(bytes);
    if let Some(position) = text.iter().position(|byte| *byte == 0) {
        return Err(ascii_refusal(
            width,
            format_smolstr!("a NUL byte at {position}"),
        ));
    }
    // The byte class is one fact and [`crate::Charset`] owns it: this is that
    // charset's own scan, which reads a machine word at a time, rather than a
    // second one written here.
    let position = crate::charset::ascii_len(text);
    if position < text.len() {
        return Err(ascii_refusal(
            width,
            format_smolstr!("a non-ASCII byte 0x{:02X} at {position}", text[position]),
        ));
    }
    if width.is_some_and(|width| text.len() > width) {
        return Err(ascii_refusal(
            width,
            format_smolstr!("{} bytes", text.len()),
        ));
    }
    // Every byte is ASCII, so the slice is UTF-8 by construction.
    std::str::from_utf8(text).map_err(|error| ascii_refusal(width, format_smolstr!("{error}")))
}

/// Pads `text` with trailing NUL into one storage slot.
///
/// The slot is one value of the fixed-width storage; `text` has already
/// passed [`ascii_text`] for that width, so it fits. It is the payload the
/// width stores, so a value answers with it whether or not an Arrow array is
/// being built around it.
pub(crate) fn ascii_padded(slot: &mut [u8], text: &str) {
    let length = text.len().min(slot.len());
    slot[..length].copy_from_slice(&text.as_bytes()[..length]);
    slot[length..].fill(0);
}

/// The bytes a code value carries, in any accepted spelling.
pub(crate) fn ascii_bytes(value: &Scalar) -> Option<&[u8]> {
    match value {
        Scalar::String(text) => Some(text.as_str().as_bytes()),
        Scalar::Code(code) => Some(code.as_str().as_bytes()),
        Scalar::Bytes(bytes) => Some(bytes.as_bytes()),
        _ => None,
    }
}

fn ascii_refusal(width: Option<usize>, actual: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: match width {
            Some(width) => crate::text::expected_got(
                format_args!("ASCII text of at most {width} bytes"),
                actual,
            ),
            None => crate::text::expected_got(format_args!("ASCII text"), actual),
        },
    }
}

/// Refuses a datatype that has no packed integer.
fn ascii_values_refusal(values: &DataType) -> Error {
    Error::InvalidDataType {
        kind: "ascii",
        reason: crate::text::expected_got(
            format_args!(
                "a fixed US-ASCII string of at most {PACKED_LIMIT} bytes, or a registered code"
            ),
            format_args!("{values}"),
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
            assert_eq!(dtype.fixed_byte_width(), Some(*width));
            assert_eq!(dtype.id().fixed_byte_width(), Some(*width));
            assert_eq!(dtype.to_string(), *name);
            assert_eq!(DataType::from_str(name).unwrap(), *dtype);
            assert!(dtype.is_code());
            assert!(!dtype.is_string());
        }
        assert!(!DataType::fixed_ascii(3).unwrap().is_code());
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
        let wrong = code_cell_text(&DataType::fixed_ascii(3).unwrap(), b"USD")
            .unwrap_err()
            .to_string();
        assert!(wrong.contains("registered codes"), "{wrong}");
    }
}
