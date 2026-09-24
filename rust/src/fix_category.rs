use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// The independently named categories in a FIX registry.
///
/// A message is a component carrying `FIX:msgtype`; there is no fourth
/// category for it.
///
/// ```
/// use yggdryl::{DataType, FixRegistry, StructType};
/// # fn main() -> yggdryl::Result<()> {
/// let mut registry = FixRegistry::new();
/// let component = DataType::from(StructType::from_fields([])?).required_field("Party");
/// registry.insert(component)?;
/// // A definition is filed by the shape it has: a Struct is a component,
/// // and every shape is reached through the one set of field doors.
/// assert_eq!(registry.field_by_name("Party")?.dtype(), &DataType::from(StructType::from_fields([])?));
/// assert!(registry.get_field_by_tag(448).is_none());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FixCategory {
    /// Tagged scalar wire fields, including repeating-group counters.
    Fields,
    /// Named Struct definitions; one carrying `FIX:msgtype` is a message.
    Components,
    /// Series of component occurrences, referencing a scalar counter.
    Groups,
}

impl FixCategory {
    /// Every category in public display order.
    pub const ALL: [Self; 3] = [Self::Fields, Self::Components, Self::Groups];

    /// The canonical category and storage-folder name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fields => "fields",
            Self::Components => "components",
            Self::Groups => "groups",
        }
    }

    /// Parses one canonical category name.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        value.parse()
    }
}

impl FromStr for FixCategory {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.as_str() == value)
            .ok_or_else(|| Error::Parse {
                target: "FIX category",
                position: 0,
                reason: crate::text::expected_got("fields, components, or groups", value),
            })
    }
}

impl fmt::Display for FixCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AsRef<str> for FixCategory {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
