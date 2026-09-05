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
    /// UTF-8 text in variable, large, and view layouts.
    Text,
    /// Validated ASCII text and registered fixed-width codes.
    Ascii,
    /// Byte strings in variable, fixed, large, and view layouts.
    Bytes,
    /// Lists, structs, unions, maps, wrappers, and self-describing values.
    Nested,
    /// Geometries and geographies carried as Well-Known Binary.
    ///
    /// One family for both: a geometry lives on a planar coordinate system
    /// and a geography on a sphere or spheroid, and everything family-uniform
    /// (WKB payloads, bounding-box statistics, the refusal of min/max) is the
    /// same for the pair.
    Geospatial,
    /// One 128-bit universally unique identifier.
    ///
    /// It is not `Binary` wearing an extension name: no binary-family
    /// behavior - concatenation, prefix matching, a variable width - is
    /// correct for an identifier, and it is not `String` either, because the
    /// hyphenated spelling is a rendering of sixteen bytes rather than the
    /// value itself.
    Guid,
}

impl DataTypeKind {
    /// Every category in canonical order.
    pub const ALL: [Self; 12] = [
        Self::Null,
        Self::Boolean,
        Self::Integer,
        Self::Floating,
        Self::Decimal,
        Self::Temporal,
        Self::Text,
        Self::Ascii,
        Self::Bytes,
        Self::Nested,
        Self::Geospatial,
        Self::Guid,
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
            Self::Text => "text",
            Self::Ascii => "ascii",
            Self::Bytes => "bytes",
            Self::Nested => "nested",
            Self::Geospatial => "geospatial",
            Self::Guid => "guid",
        }
    }

    /// Return whether the category holds child fields or a nested value.
    ///
    /// Exact nested shape and wrapper behavior belong to
    /// [`crate::types::NestedType`].
    pub const fn is_nested(self) -> bool {
        matches!(self, Self::Nested)
    }

    /// Return whether the category is a fixed-width or exact number.
    pub const fn is_numeric(self) -> bool {
        matches!(self, Self::Integer | Self::Floating | Self::Decimal)
    }

    /// Return whether the category stores an opaque or textual byte payload.
    pub const fn is_bytes(self) -> bool {
        matches!(self, Self::Bytes | Self::Text | Self::Ascii)
    }

    /// Return whether values of the category have a total order.
    pub const fn is_ordered(self) -> bool {
        !matches!(self, Self::Nested | Self::Geospatial)
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
    fn nested_is_the_one_coarse_family_for_every_nested_shape() {
        assert!(DataTypeKind::Nested.is_nested());
        assert!(!DataTypeKind::Bytes.is_nested());
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
