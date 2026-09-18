//! ISO 4217 currency codes.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::types;
use crate::types::code::{CodeValue, code_leaf, code_value};
use crate::types::typed::define_field_types;
use crate::{DataType, Result, Scalar, Value};

code_leaf!(Currency, CURRENCY_WIDTH);

impl Currency {
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

code_value!(Currency, Currency, CURRENCY_WIDTH, merge = Currency::merged);

/// The Arrow extension name of the currency code.
pub(crate) const CURRENCY_EXTENSION_NAME: &str = "yggdryl.currency";

/// The most bytes ISO 4217's currency code may be.
pub(crate) const CURRENCY_WIDTH: usize = 3;

impl DataType {
    /// Creates ISO 4217's three-letter currency code.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::currency(), DataType::Currency);
    /// assert_eq!(DataType::currency().to_string(), "currency");
    /// assert_eq!(DataType::currency().code_width(), Some(3));
    /// ```
    #[must_use]
    pub const fn currency() -> Self {
        Self::Currency
    }
}

// /// A currency-typed field: ISO 4217.
define_field_types!(CurrencyType, Currency);
