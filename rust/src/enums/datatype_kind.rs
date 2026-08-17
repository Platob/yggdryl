use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::{Error, Result};

/// A coarse datatype category shared by every variant of one family.
///
/// [`DataTypeKind`] mirrors the responsibility split of the `datatype` module,
/// so behavior that is uniform across a family dispatches on one value instead
/// of re-listing variants. Use [`crate::DataTypeId`] when a specific variant
/// matters and this value when only the family does.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DataTypeKind {
    /// The unit type carrying only nulls.
    Null,
    /// Boolean values.
    Boolean,
    /// Signed and unsigned fixed-width integers.
    Integer,
    /// IEEE binary floating point.
    Floating,
    /// Exact base-10 decimals with a precision and scale.
    Decimal,
    /// Dates, times, timestamps, durations, and calendar intervals.
    Temporal,
    /// Byte strings in variable, fixed, large, and view layouts.
    Binary,
    /// UTF-8 text in variable, large, and view layouts.
    String,
    /// Ordered sequences of one element field.
    List,
    /// Ordered named child fields.
    Struct,
    /// Tagged alternatives over child fields.
    Union,
    /// Key/value entries backed by a struct element.
    Map,
    /// Dictionary-encoded values over a key index.
    Dictionary,
    /// Run-end encoded values over run boundaries.
    RunEndEncoded,
}

impl DataTypeKind {
    /// Every category in canonical order.
    pub const ALL: [Self; 14] = [
        Self::Null,
        Self::Boolean,
        Self::Integer,
        Self::Floating,
        Self::Decimal,
        Self::Temporal,
        Self::Binary,
        Self::String,
        Self::List,
        Self::Struct,
        Self::Union,
        Self::Map,
        Self::Dictionary,
        Self::RunEndEncoded,
    ];

    /// Parse a canonical lowercase category name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the unrecognized input and the accepted
    /// vocabulary.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the canonical lowercase name without allocating.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean => "boolean",
            Self::Integer => "integer",
            Self::Floating => "floating",
            Self::Decimal => "decimal",
            Self::Temporal => "temporal",
            Self::Binary => "binary",
            Self::String => "string",
            Self::List => "list",
            Self::Struct => "struct",
            Self::Union => "union",
            Self::Map => "map",
            Self::Dictionary => "dictionary",
            Self::RunEndEncoded => "run_end_encoded",
        }
    }

    /// Return whether the category holds child fields or a nested value.
    ///
    /// [`Self::Dictionary`] and [`Self::RunEndEncoded`] are wrappers: they are
    /// nested only when their value type is, so this predicate reports `false`
    /// for them and [`Self::is_wrapper`] identifies them instead.
    pub const fn is_nested(self) -> bool {
        matches!(self, Self::List | Self::Struct | Self::Union | Self::Map)
    }

    /// Return whether the category transparently encodes another value type.
    pub const fn is_wrapper(self) -> bool {
        matches!(self, Self::Dictionary | Self::RunEndEncoded)
    }

    /// Return whether the category is a fixed-width or exact number.
    pub const fn is_numeric(self) -> bool {
        matches!(self, Self::Integer | Self::Floating | Self::Decimal)
    }

    /// Return whether the category stores an opaque or textual byte payload.
    pub const fn is_bytes(self) -> bool {
        matches!(self, Self::Binary | Self::String)
    }

    /// Return whether values of the category have a total order.
    pub const fn is_ordered(self) -> bool {
        !matches!(self, Self::Union | Self::Map | Self::Struct | Self::List)
    }
}

impl FromStr for DataTypeKind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| value.eq_ignore_ascii_case(kind.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "datatype kind",
                position: 0,
                reason: format_smolstr!(
                    "expected one of {}, got {value:?}",
                    canonical_vocabulary()
                ),
            })
    }
}

fn canonical_vocabulary() -> String {
    DataTypeKind::ALL
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

impl fmt::Display for DataTypeKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for DataTypeKind {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DataTypeKind {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = <&str>::deserialize(deserializer)?;
        Self::from_str(value).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::DataTypeKind;

    #[test]
    fn names_round_trip_case_insensitively() {
        for kind in DataTypeKind::ALL {
            assert_eq!(DataTypeKind::from_str(kind.as_str()).unwrap(), kind);
            assert_eq!(
                DataTypeKind::from_str(&kind.as_str().to_uppercase()).unwrap(),
                kind
            );
        }
    }

    #[test]
    fn unknown_name_reports_the_input_and_vocabulary() {
        let error = DataTypeKind::from_str("int32").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("\"int32\""), "{message}");
        assert!(message.contains("integer"), "{message}");
    }

    #[test]
    fn wrappers_are_not_reported_as_nested() {
        assert!(DataTypeKind::Dictionary.is_wrapper());
        assert!(!DataTypeKind::Dictionary.is_nested());
        assert!(DataTypeKind::Struct.is_nested());
        assert!(!DataTypeKind::Struct.is_wrapper());
    }

    #[test]
    fn categories_are_unique() {
        let mut names: Vec<_> = DataTypeKind::ALL.iter().map(|kind| kind.as_str()).collect();
        names.sort_unstable();
        let total = names.len();
        names.dedup();
        assert_eq!(names.len(), total);
    }
}
