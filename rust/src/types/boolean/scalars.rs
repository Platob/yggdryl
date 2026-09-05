//! Null and Boolean values and typed scalar aliases.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::types::Scalar;
use crate::types::typed::define_scalar_type;

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
        Self::Bool(value)
    }
}

define_scalar_type!(NullScalar, super::NullType, "null", crate::DataType::Null);
define_scalar_type!(
    BooleanScalar,
    super::BooleanType,
    "boolean",
    crate::DataType::Boolean
);
