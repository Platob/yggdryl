//! Currency codes: ISO 4217's, and the digital-asset tickers past them.

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

    /// Whether this is the currency stated as none, `XXX`: rank zero,
    /// which any stated currency replaces. The ISO listing is no rank - a
    /// digital-asset ticker is a currency here, and `USD` never displaces
    /// `USDT`.
    #[must_use]
    pub fn is_none(&self) -> bool {
        self.as_str() == Self::NONE
    }

    /// [`CodeValue::rank`]: zero for `XXX`, one for anything stated.
    fn ranked(&self) -> u8 {
        u8::from(!self.is_none())
    }
}

code_value!(Ccy, Ccy, CCY_WIDTH, rank = Ccy::ranked, max_rank = 1);

/// The Arrow extension name of the currency code.
pub(crate) const CCY_EXTENSION_NAME: &str = "yggdryl.ccy";

/// The most bytes a currency code may be: ISO 4217's three letters, or a
/// digital-asset ticker - `USDT`, `DOGE`, `1INCH`, `BABYDOGE` - up to eight.
pub(crate) const CCY_WIDTH: usize = 8;

impl DataType {
    /// Creates the currency code: ISO 4217's three letters, or a
    /// digital-asset ticker of up to eight bytes.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::ccy(), DataType::Ccy);
    /// assert_eq!(DataType::ccy().to_string(), "ccy");
    /// assert_eq!(DataType::ccy().code_width(), Some(8));
    /// assert!(DataType::ccy().scalar("USDT").is_ok());
    /// assert!(DataType::ccy().scalar("TOOLONGCCY").is_err());
    /// ```
    #[must_use]
    pub const fn ccy() -> Self {
        Self::Ccy
    }
}

// /// A currency-typed field: ISO 4217 or a digital-asset ticker.
define_field_types!(CcyType, Ccy);
