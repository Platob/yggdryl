//! ISO 10383 market identifier codes.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::types;
use crate::types::code::{CodeValue, code_leaf, code_value};
use crate::types::typed::define_field_types;
use crate::{DataType, Result, Scalar, TypedField, Value};

code_leaf!(Mic, MIC_WIDTH);

impl Mic {
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

code_value!(Mic, Mic, MIC_WIDTH, merge = Mic::merged);

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
    /// assert_eq!(DataType::mic(), DataType::Mic);
    /// assert_eq!(DataType::mic().to_string(), "mic");
    /// assert_eq!(DataType::mic().code_width(), Some(4));
    /// ```
    #[must_use]
    pub const fn mic() -> Self {
        Self::Mic
    }
}

// /// A MIC-typed field: ISO 10383's market identifier.
define_field_types!(MicType, Mic, crate::DataType::Mic);

/// A field of this code.
pub type MicField = TypedField<MicType>;
