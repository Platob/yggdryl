//! Null and Boolean datatypes.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::types::Scalar;
use crate::types::typed::define_field_types;
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

/// Read a boolean out of its canonical spelling.
///
/// `true` and `false` are what a boolean prints, so they are what it reads;
/// the case is not part of the spelling. A column keeps Arrow's wider reading
/// behind this one, exactly as a temporal column does.
pub(crate) fn boolean_from_text(text: &str) -> Option<Scalar> {
    match text.trim() {
        value if value.eq_ignore_ascii_case("true") => Some(Scalar::from(true)),
        value if value.eq_ignore_ascii_case("false") => Some(Scalar::from(false)),
        _ => None,
    }
}

// ------------------------------------------------------------------------
// Arrow projection: the two logic-free datatypes are Arrow's own.
// ------------------------------------------------------------------------

mod arrow {
    use arrow_schema::DataType as ArrowDataType;
    use smol_str::format_smolstr;

    use super::{BooleanType, NullType};
    use crate::types::invalid;
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
