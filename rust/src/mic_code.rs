//! ISO 10383 market identifier codes.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::code::{code_leaf, code_value};
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

code_leaf!(MicCode, MIC_WIDTH);

impl MicCode {
    /// ISO 10383's code for no market.
    const NONE: &str = "XXXX";

    /// The market stated as none: ISO 10383's `XXXX`, which a merge takes
    /// the other market over.
    #[must_use]
    pub fn none() -> Self {
        Self(SmolStr::new_static(Self::NONE))
    }

    /// The better of two markets: this one, unless it is `XXXX`.
    fn merged(self, other: &Self) -> Self {
        if self.as_str() == Self::NONE {
            other.clone()
        } else {
            self
        }
    }
}

code_value!(MicCode, MicCode, MIC_WIDTH, merge = MicCode::merged);

/// The Arrow extension name of the market identifier code.
pub(crate) const MIC_EXTENSION_NAME: &str = "yggdryl.mic";

/// The most bytes ISO 10383's market identifier code may be.
pub(crate) const MIC_WIDTH: usize = 4;

impl DataType {
    /// Creates ISO 10383's four-character market identifier code.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::mic(), DataType::MicCode);
    /// assert_eq!(DataType::mic().to_string(), "mic");
    /// assert_eq!(DataType::mic().code_width(), Some(4));
    /// ```
    #[must_use]
    pub const fn mic() -> Self {
        Self::MicCode
    }
}

// /// A MIC-typed field: ISO 10383's market identifier.
define_field_types!(MicCodeType, MicCode);
