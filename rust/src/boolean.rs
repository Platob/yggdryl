//! Null and Boolean datatypes.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Scalar;
use crate::typed::define_field_types;
use crate::{DataType, Result, Value};

// ------------------------------------------------------------------------
// Parameterless Null and Boolean datatype variants need no constructors.
// ------------------------------------------------------------------------

// ------------------------------------------------------------------------
// Null and Boolean datatypes: neither carries a parameter.
// ------------------------------------------------------------------------

define_field_types!(NullType, Null);

define_field_types!(BooleanType, Boolean);

// ------------------------------------------------------------------------
// Null and Boolean values and typed scalar aliases.
// ------------------------------------------------------------------------

/// The one null value.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub struct Null;

impl fmt::Display for Null {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("null")
    }
}

/// One Boolean value.
#[repr(transparent)]
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct Boolean(bool);

impl Boolean {
    /// Construct a Boolean value.
    pub const fn new(value: bool) -> Self {
        Self(value)
    }

    /// Return the native Boolean.
    pub const fn get(self) -> bool {
        self.0
    }
}

impl fmt::Display for Boolean {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl From<bool> for Boolean {
    fn from(value: bool) -> Self {
        Self::new(value)
    }
}

impl From<Boolean> for bool {
    fn from(value: Boolean) -> Self {
        value.get()
    }
}

impl From<()> for Scalar {
    fn from((): ()) -> Self {
        Self::Null
    }
}

impl From<bool> for Scalar {
    fn from(value: bool) -> Self {
        Self::Boolean(Boolean::new(value))
    }
}

impl Value for Null {
    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Null)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Null
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        static NULL: Null = Null;
        value.is_null().then_some(&NULL)
    }
}

impl Value for Boolean {
    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Boolean)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Boolean(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Boolean(value) => Some(value),
            _ => None,
        }
    }
}

/// The spellings a boolean is read from: Arrow's string-to-boolean cast's,
/// so a cell and a column read one text alike.
const TRUE_SPELLINGS: [&str; 9] = ["true", "t", "tr", "tru", "yes", "y", "ye", "on", "1"];
const FALSE_SPELLINGS: [&str; 10] = [
    "false", "f", "fa", "fal", "fals", "no", "n", "off", "of", "0",
];

/// Read a boolean out of text, the way a column cast reads one.
///
/// A declared boolean takes every spelling Arrow's cast reads - `true`,
/// `yes`, `y`, `on`, `1` and the prefixes of `true` and `yes`; `false`, `no`,
/// `n`, `off`, `0` and the prefixes of `false` and `off` - ASCII
/// case-insensitive and trimmed, so FIX's `Y` and `N` and a bridge's `no`
/// are readings rather than refusals, and a row reads what its column's cast
/// reads. Inference proves a boolean only from what one prints
/// ([`prints_boolean`]).
pub(crate) fn boolean_from_text(text: &str) -> Option<Scalar> {
    let text = text.trim();
    let spells = |spellings: &[&str]| spellings.iter().any(|held| text.eq_ignore_ascii_case(held));
    if spells(&TRUE_SPELLINGS) {
        Some(Scalar::from(true))
    } else if spells(&FALSE_SPELLINGS) {
        Some(Scalar::from(false))
    } else {
        None
    }
}

/// Whether `text` is a boolean as one prints: `true` or `false`, the case
/// and the surrounding blanks not part of the spelling.
///
/// Inference reads what a value already is, so this - never the wider
/// [`boolean_from_text`] - is what proves a column boolean: `1` already is
/// an integer, and a column of them is not a column of flags.
pub(crate) fn prints_boolean(text: &str) -> bool {
    let text = text.trim();
    text.eq_ignore_ascii_case("true") || text.eq_ignore_ascii_case("false")
}

// ------------------------------------------------------------------------
// Arrow projection: the two logic-free datatypes are Arrow's own.
// ------------------------------------------------------------------------

mod arrow {
    use arrow_schema::DataType as ArrowDataType;
    use smol_str::format_smolstr;

    use super::{BooleanType, NullType};
    use crate::invalid;
    use crate::{DataType, Result};

    impl NullType {
        /// The Arrow storage a null column lays out.
        pub(crate) const fn arrow_storage() -> ArrowDataType {
            ArrowDataType::Null
        }
    }

    impl BooleanType {
        /// The Arrow storage a boolean column lays out.
        pub(crate) const fn arrow_storage() -> ArrowDataType {
            ArrowDataType::Boolean
        }
    }

    /// The datatype one of Arrow's two logic-free storages imports as.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage belongs to another family.
    pub(crate) fn from_arrow_storage(value: &ArrowDataType) -> Result<DataType> {
        match value {
            ArrowDataType::Null => Ok(DataType::Null),
            ArrowDataType::Boolean => Ok(DataType::Boolean),
            other => Err(invalid(
                "Boolean",
                format_smolstr!("expected a null or boolean storage, got {other}"),
            )),
        }
    }
}

pub(crate) use arrow::from_arrow_storage;
