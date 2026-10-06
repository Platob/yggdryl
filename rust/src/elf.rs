//! ISO 20275 entity legal form codes.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use smol_str::SmolStr;

use crate::code::{code_value, folded_code};
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

/// One ISO 20275 Entity Legal Form code, held by its shape.
///
/// Four ASCII letters or digits naming one legal form in GLEIF's code list -
/// `2HBR` is a German GmbH. A legal form, not an identity: the code carries
/// no check character, so every well-shaped value ranks the same, and
/// whether the list assigns it is not a question the value answers. Lower
/// case folds once at construction; four bytes stay inline.
#[repr(transparent)]
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Elf(SmolStr);

impl<'de> Deserialize<'de> for Elf {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        SmolStr::deserialize(deserializer)
            .and_then(|value| Self::new(value).map_err(serde::de::Error::custom))
    }
}

impl Elf {
    /// Validate and construct an Entity Legal Form code: four ASCII letters
    /// or digits, upper-cased.
    ///
    /// ```
    /// use yggdryl::{CodeValue, Elf};
    ///
    /// let gmbh = Elf::new("2hbr")?;
    /// assert_eq!(gmbh.as_str(), "2HBR");
    /// assert!(gmbh.is_real());
    /// assert!(Elf::new("2HB").is_err(), "three characters");
    /// assert!(Elf::new("2H-R").is_err(), "punctuation");
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the text is not four ASCII letters or digits.
    pub fn new(value: impl AsRef<str>) -> Result<Self> {
        folded_code::<ELF_WIDTH>("elf", value.as_ref(), Self::refusal).map(Self)
    }

    /// Borrow the code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Borrow the compact storage without copying the code.
    #[must_use]
    pub const fn storage(&self) -> &SmolStr {
        &self.0
    }

    /// Whether `text` is exactly the spelling this type stores: four
    /// upper-case letters or digits.
    #[must_use]
    pub fn is_canonical(text: &str) -> bool {
        Self::refusal(text).is_none()
    }

    fn refusal(folded: &str) -> Option<&'static str> {
        let bytes = folded.as_bytes();
        if bytes.len() != ELF_WIDTH {
            return Some("expected four characters");
        }
        if !bytes
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        {
            return Some("expected four letters or digits");
        }
        None
    }
}

impl fmt::Display for Elf {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

code_value!(Elf, Elf, ELF_WIDTH);

/// The Arrow extension name of an entity legal form code.
pub(crate) const ELF_EXTENSION_NAME: &str = "yggdryl.elf";

/// The exact width of an entity legal form code.
pub(crate) const ELF_WIDTH: usize = 4;

impl DataType {
    /// Creates ISO 20275's four-character entity legal form datatype.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::elf(), DataType::Elf);
    /// assert_eq!(DataType::elf().to_string(), "elf");
    /// assert_eq!(DataType::elf().code_width(), Some(4));
    /// ```
    #[must_use]
    pub const fn elf() -> Self {
        Self::Elf
    }
}

define_field_types!(ElfType, Elf);
