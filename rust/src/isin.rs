//! ISO 6166 securities identification numbers.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::code::code_value;
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, StringEnum, Value};

/// One ISO 6166 international securities identification number, held by
/// its shape.
///
/// Twelve bytes: a two-letter prefix, nine alphanumerics of national number
/// and one check digit, which is the Luhn digit of the eleven before it read
/// with each letter expanded to the two digits of its alphabet position.
/// The shape is what [`Isin::new`] admits; whether the digit closes the
/// number ([`Isin::is_closed`]) and whether its prefix is one an agency numbers
/// under ([`Isin::is_listed_prefix`]) are the two readings its
/// [`rank`](CodeValue::rank) counts, so a masked line's `XX0000000001` and a
/// typo are values of a lower rank - which every merge replaces by a real
/// number whatever the order - rather than refusals, and a derivation that
/// needs the number to be real ([`securityid::embedded`](crate::securityid::embedded),
/// a country of issue) asks `is_closed` first.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Isin(SmolStr);

impl Isin {
    /// The number that states none: ISO 3166's user-assigned `XX` over a
    /// national number of nothing, which closes nowhere and is listed
    /// nowhere - the lowest rank there is, so any stated number replaces it.
    /// What a book is keyed by where its inputs state neither an ISIN nor a
    /// ticker ([`Market::book_crosscode`](crate::graph::Market::book_crosscode)).
    pub const NONE: &str = "XX0000000000";

    /// The two-letter prefixes ISO 6166 gives an agency rather than a
    /// country: the ECB's `EU`, the DSB's `EZ` for derivatives, the
    /// international and substitute agencies `XA` to `XS`, and `XT` for a
    /// referential instrument - a digital token, a crypto-asset.
    const AGENCY_PREFIXES: [&str; 10] =
        ["EU", "EZ", "XA", "XB", "XC", "XD", "XF", "XK", "XS", "XT"];

    /// Validate and construct a securities identification number: twelve
    /// ASCII bytes of the number's shape, upper-cased.
    ///
    /// Lower case is read as the upper case it spells, because the number
    /// is case-insensitive by construction: the check digit expands a letter
    /// by its position, which case does not change. The check digit is
    /// admitted as stated - a number it does not close is a value of rank
    /// one or zero, never a refusal.
    ///
    /// ```
    /// use yggdryl::{CodeValue, Isin};
    ///
    /// let apple = Isin::new("US0378331005").unwrap();
    /// assert_eq!(apple.as_str(), "US0378331005");
    /// assert_eq!(apple.prefix(), "US");
    /// assert_eq!(apple.nsin(), "037833100");
    /// assert_eq!(apple.check_digit(), 5);
    /// assert_eq!(apple.rank(), 2);
    /// assert_eq!(Isin::new("us0378331005").unwrap(), apple);
    /// // One digit off is a typo: a number that does not close, and ranks
    /// // below one that does.
    /// let typo = Isin::new("US0378331006").unwrap();
    /// assert!(!Isin::is_closed(typo.as_str()));
    /// assert_eq!(typo.rank(), 1);
    /// assert!(Isin::new("US037833100").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not twelve ASCII bytes of the
    /// number's shape: two letters, nine alphanumerics, a closing digit.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        let value = crate::ascii_text(ISIN_WIDTH, value.as_ref().as_bytes())?;
        let mut bytes = [0_u8; ISIN_WIDTH];
        for (target, byte) in bytes.iter_mut().zip(value.bytes()) {
            *target = byte.to_ascii_uppercase();
        }
        let folded = std::str::from_utf8(&bytes[..value.len()]).expect("validated ASCII");
        if let Some(reason) = Self::refusal(folded) {
            return Err(crate::Error::InvalidDataType {
                kind: "isin",
                reason: smol_str::format_smolstr!("{reason}, got {value:?}"),
            });
        }
        Ok(Self(SmolStr::new(folded)))
    }

    /// A number a landed `isin` column already holds, adopted as it
    /// stands: the landing read the cell under this type's rule, and a
    /// column holds the canonical spelling.
    pub(crate) fn from_proven(text: &str) -> Self {
        Self(SmolStr::new(text))
    }

    /// The number stated as none: [`Self::NONE`].
    #[must_use]
    pub fn none() -> Self {
        Self(SmolStr::new_static(Self::NONE))
    }

    /// Whether this is the number stated as none, [`Self::NONE`].
    #[must_use]
    pub fn is_none(&self) -> bool {
        self.as_str() == Self::NONE
    }

    /// Borrow the number.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the shared storage without copying the number.
    #[must_use]
    pub const fn storage(&self) -> &SmolStr {
        &self.0
    }

    /// The two-letter prefix: the country of the numbering agency, or one of
    /// the international prefixes such as `XS`.
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.as_str()[..2]
    }

    /// The nine-character national securities identifying number.
    #[must_use]
    pub fn nsin(&self) -> &str {
        &self.as_str()[2..11]
    }

    /// The check digit that closes the number.
    #[must_use]
    pub fn check_digit(&self) -> u8 {
        self.as_str().as_bytes()[11] - b'0'
    }

    /// Whether `text` is a number exactly as this type stores it: upper
    /// case, and of the shape [`Self::new`] admits.
    ///
    /// What a column holds is the canonical spelling, so bytes arriving
    /// through a cast are held to it rather than folded on every read. This
    /// is the strict question about the spelling and says nothing of the
    /// check digit, which is [`Self::is_closed`]' question: a column holds a
    /// masked number as the value it is.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        text.len() == ISIN_WIDTH
            && text.is_ascii()
            && !text.bytes().any(|byte| byte.is_ascii_lowercase())
            && Self::refusal(text).is_none()
    }

    /// Whether `text`, upper case, is a number its check digit closes: the
    /// Luhn digit of the eleven leading characters is the twelfth.
    ///
    /// The reading every derivation off a number asks first - the national
    /// identifier it embeds, the country of issue, the source a bare
    /// identifier is inferred under - because a number that does not close
    /// is a typo or a mask, and a typo typed as a security joins to the
    /// wrong one. Lower case closes nothing: it is not how a column spells
    /// a number.
    ///
    /// ```
    /// use yggdryl::Isin;
    ///
    /// assert!(Isin::is_closed("US0378331005"));
    /// assert!(!Isin::is_closed("US0378331006"));
    /// assert!(!Isin::is_closed("XX0000000001"));
    /// ```
    #[must_use]
    pub fn is_closed(text: &str) -> bool {
        let bytes = text.as_bytes();
        bytes.len() == ISIN_WIDTH
            && bytes[11].is_ascii_digit()
            && Self::closing_digit(&text[..11]) == Some(bytes[11] - b'0')
    }

    /// Whether `text` opens with a prefix some agency numbers under: an
    /// ISO 3166 country code [`StringEnum::COUNTRIES`] lists, or one of the
    /// agency prefixes - the ECB's `EU`, the DSB's `EZ`, the international
    /// `XS` and its neighbours, `XT` for a referential instrument. `ZZ`,
    /// which ISO 6166 gives a derivative no agency has numbered yet, and
    /// the user-assigned `XX` are listed nowhere.
    ///
    /// ```
    /// use yggdryl::Isin;
    ///
    /// assert!(Isin::is_listed_prefix("US0378331005"));
    /// assert!(Isin::is_listed_prefix("EZN11TD1F7K3"));
    /// assert!(Isin::is_listed_prefix("XT0000000000"));
    /// assert!(!Isin::is_listed_prefix("ZZ0000000008"));
    /// assert!(!Isin::is_listed_prefix("XX0000000001"));
    /// ```
    #[must_use]
    pub fn is_listed_prefix(text: &str) -> bool {
        let Some(prefix) = text.get(..2) else {
            return false;
        };
        StringEnum::COUNTRIES.binary_search(&prefix).is_ok()
            || Self::AGENCY_PREFIXES.contains(&prefix)
    }

    /// The rank `text` holds as a number: zero where it is not the upper
    /// case shape, else one for each of [`Self::is_closed`] and
    /// [`Self::is_listed_prefix`] - two for a real number, one for a typo
    /// under a listed prefix or a closing `ZZ` number, zero for a masked
    /// one. What [`CodeValue::rank`] answers for a value, read off the
    /// text alone.
    ///
    /// ```
    /// use yggdryl::Isin;
    ///
    /// assert_eq!(Isin::rank_of("US0378331005"), 2);
    /// assert_eq!(Isin::rank_of("US0378331006"), 1);
    /// assert_eq!(Isin::rank_of("ZZ0000000008"), 1);
    /// assert_eq!(Isin::rank_of("XX0000000001"), 0);
    /// assert_eq!(Isin::rank_of("not a number"), 0);
    /// ```
    #[must_use]
    pub fn rank_of(text: &str) -> u8 {
        if !Self::is_canonical(text) {
            return 0;
        }
        u8::from(Self::is_closed(text)) + u8::from(Self::is_listed_prefix(text))
    }

    /// The check digit that closes eleven leading characters, or `None`
    /// where they are not two letters and nine alphanumerics.
    ///
    /// ISO 6166 reads the eleven as digits - a letter as the two digits of
    /// its position from `A` at ten - and closes them with the Luhn digit,
    /// doubling every second digit from the right.
    #[must_use]
    pub fn closing_digit(body: &str) -> Option<u8> {
        let bytes = body.as_bytes();
        if bytes.len() != ISIN_WIDTH - 1
            || !bytes[..2].iter().all(u8::is_ascii_uppercase)
            || !bytes[2..].iter().all(u8::is_ascii_alphanumeric)
        {
            return None;
        }
        // Eleven characters expand to at most twenty-two digits.
        let mut digits = [0_u8; 2 * (ISIN_WIDTH - 1)];
        let mut held = 0;
        for byte in bytes {
            match byte {
                b'0'..=b'9' => {
                    digits[held] = byte - b'0';
                    held += 1;
                }
                b'A'..=b'Z' => {
                    let position = byte - b'A' + 10;
                    digits[held] = position / 10;
                    digits[held + 1] = position % 10;
                    held += 2;
                }
                _ => return None,
            }
        }
        let mut sum = 0_u32;
        for (from_right, digit) in digits[..held].iter().rev().enumerate() {
            let mut value = u32::from(*digit);
            if from_right % 2 == 0 {
                value *= 2;
                if value > 9 {
                    value -= 9;
                }
            }
            sum += value;
        }
        u8::try_from((10 - sum % 10) % 10).ok()
    }

    /// Why an upper-cased, twelve-byte spelling is not a number's shape,
    /// or nothing.
    fn refusal(folded: &str) -> Option<&'static str> {
        let bytes = folded.as_bytes();
        if bytes.len() != ISIN_WIDTH {
            return Some("expected twelve characters");
        }
        if !bytes[..2].iter().all(u8::is_ascii_uppercase) {
            return Some("expected a two-letter prefix");
        }
        if !bytes[2..11].iter().all(u8::is_ascii_alphanumeric) {
            return Some("expected nine alphanumerics after the prefix");
        }
        if !bytes[11].is_ascii_digit() {
            return Some("expected a closing check digit");
        }
        None
    }

    /// [`CodeValue::rank`]: [`Self::rank_of`] over the held number.
    fn ranked(&self) -> u8 {
        Self::rank_of(self.as_str())
    }
}

impl fmt::Display for Isin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_value!(Isin, Isin, ISIN_WIDTH, rank = Isin::ranked, max_rank = 2);

/// The Arrow extension name of the securities identification number.
pub(crate) const ISIN_EXTENSION_NAME: &str = "yggdryl.isin";

/// The most bytes an ISO 6166 securities identification number may be.
///
/// Two letters of prefix, nine of national number and one check digit:
/// twelve, which the standard fixes and the check digit closes.
pub(crate) const ISIN_WIDTH: usize = 12;

impl DataType {
    /// Creates ISO 6166's twelve-character securities identification number.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::isin(), DataType::Isin);
    /// assert_eq!(DataType::isin().to_string(), "isin");
    /// assert_eq!(DataType::isin().code_width(), Some(12));
    /// ```
    #[must_use]
    pub const fn isin() -> Self {
        Self::Isin
    }
}

// /// An ISIN-typed field: ISO 6166's securities identification number.
define_field_types!(IsinType, Isin);
