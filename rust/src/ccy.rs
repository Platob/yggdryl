//! ISO 4217 currency codes.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::code::{code_leaf, code_value};
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

code_leaf!(Ccy, CCY_WIDTH);

impl Ccy {
    /// ISO 4217's code for no currency.
    const NONE: &str = "XXX";

    /// The currency stated as none: ISO 4217's `XXX`, which a merge takes
    /// the other currency over.
    #[must_use]
    pub fn none() -> Self {
        Self(SmolStr::new_static(Self::NONE))
    }

    /// The better of two currencies: this one, unless it is `XXX`.
    fn merged(self, other: &Self) -> Self {
        if self.as_str() == Self::NONE {
            other.clone()
        } else {
            self
        }
    }
}

code_value!(Ccy, Ccy, CCY_WIDTH, merge = Ccy::merged);

/// The Arrow extension name of the currency code.
pub(crate) const CCY_EXTENSION_NAME: &str = "yggdryl.ccy";

/// The most bytes ISO 4217's currency code may be.
pub(crate) const CCY_WIDTH: usize = 3;

impl DataType {
    /// Creates ISO 4217's three-letter currency code.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::ccy(), DataType::Ccy);
    /// assert_eq!(DataType::ccy().to_string(), "ccy");
    /// assert_eq!(DataType::ccy().code_width(), Some(3));
    /// ```
    #[must_use]
    pub const fn ccy() -> Self {
        Self::Ccy
    }
}

// /// A currency-typed field: ISO 4217.
define_field_types!(CcyType, Ccy);
