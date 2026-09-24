//! The unit a quantity is stated in.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::code::{code_leaf, code_value};
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

code_leaf!(
    Unit,
    UNIT_WIDTH,
    doc = "The unit a quantity is stated in - FIX's `UnitOfMeasure(996)` - held as the \
text it is, at most thirty-two ASCII bytes.

```
use yggdryl::Unit;

# fn main() -> yggdryl::Result<()> {
let shares = Unit::new(\"Shares\")?;
assert_eq!(shares.as_str(), \"Shares\");
assert!(!shares.is_none());
# Ok(())
# }
```"
);

impl Unit {
    /// The unit stated as none: the empty text, which a merge takes the
    /// other unit over.
    #[must_use]
    pub fn none() -> Self {
        Self(SmolStr::new_static(""))
    }

    /// Whether this unit is stated as none.
    #[must_use]
    pub fn is_none(&self) -> bool {
        self.0.is_empty()
    }

    /// The better of two units: this one, unless it is none.
    fn merged(self, other: &Self) -> Self {
        if self.is_none() { other.clone() } else { self }
    }
}

code_value!(Unit, Unit, UNIT_WIDTH, merge = Unit::merged);

/// The Arrow extension name of the unit a quantity is stated in.
pub(crate) const UNIT_EXTENSION_NAME: &str = "yggdryl.unit";

/// The most bytes a unit may be.
///
/// FIX's `UnitOfMeasure(996)` values are short words - `Bbl`, `MWh`,
/// `Shares` - and a venue's own may be longer; thirty-two is the bound the
/// Bloomberg identifier takes, and it is a maximum, not a layout.
pub(crate) const UNIT_WIDTH: usize = 32;

impl DataType {
    /// Creates the unit a quantity is stated in.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::unit(), DataType::Unit);
    /// assert_eq!(DataType::unit().to_string(), "unit");
    /// assert_eq!(DataType::unit().code_width(), Some(32));
    /// ```
    #[must_use]
    pub const fn unit() -> Self {
        Self::Unit
    }
}

// /// A unit-typed field: what a quantity is stated in.
define_field_types!(UnitType, Unit);
