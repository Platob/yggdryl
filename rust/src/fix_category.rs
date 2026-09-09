use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// The independently named categories in a FIX registry.
///
/// ```
/// use yggdryl::{DataType, FixCategory, FixRegistry};
/// # fn main() -> yggdryl::Result<()> {
/// let mut registry = FixRegistry::new();
/// let component = DataType::from_fields([])?.required_field("Party");
/// registry.create_definition(FixCategory::Components, component)?;
/// assert_eq!(registry.definitions(FixCategory::Components).count(), 1);
/// assert!(registry.get_field("Party").is_none());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FixCategory {
    /// Tagged scalar wire fields, including repeating-group counters.
    Fields,
    /// Complete message Struct definitions.
    Messages,
    /// Reusable Struct definitions.
    Components,
    /// Lists of component occurrences, referencing a scalar counter.
    Groups,
}

impl FixCategory {
    /// Every category in public display order.
    pub const ALL: [Self; 4] = [Self::Fields, Self::Messages, Self::Components, Self::Groups];

    /// The canonical category and storage-folder name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fields => "fields",
            Self::Messages => "messages",
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
                reason: crate::text::expected_got("fields, messages, components, or groups", value),
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
