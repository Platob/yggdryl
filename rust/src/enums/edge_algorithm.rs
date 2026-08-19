use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::{Error, Result};

/// How the edge between two geography vertices is interpolated.
///
/// A geometry connects vertices with straight planar lines, so it needs no
/// algorithm - and a geometry given one is refused by name. A geography lives
/// on a sphere or spheroid, where "the line between two points" has more than
/// one answer, and this value names which one a column's edges use. The
/// vocabulary is the one Parquet's `GEOGRAPHY` logical type and Iceberg v3
/// share; `Spherical` is both formats' default.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum EdgeAlgorithm {
    /// Great-circle edges on a perfect sphere.
    #[default]
    Spherical,
    /// Geodesic edges on a spheroid, by Vincenty's iterative formulae.
    Vincenty,
    /// Geodesic edges by the Thomas cubic-series approximation.
    Thomas,
    /// Geodesic edges by the Andoyer first-order approximation.
    Andoyer,
    /// Geodesic edges by Karney's exact algorithm.
    Karney,
}

impl EdgeAlgorithm {
    /// Every algorithm in canonical order, [`Self::Spherical`] first because
    /// it is the default both formats fill.
    pub const ALL: [Self; 5] = [
        Self::Spherical,
        Self::Vincenty,
        Self::Thomas,
        Self::Andoyer,
        Self::Karney,
    ];

    /// Parse a canonical lowercase algorithm name.
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
            Self::Spherical => "spherical",
            Self::Vincenty => "vincenty",
            Self::Thomas => "thomas",
            Self::Andoyer => "andoyer",
            Self::Karney => "karney",
        }
    }
}

impl FromStr for EdgeAlgorithm {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|algorithm| value.eq_ignore_ascii_case(algorithm.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "edge algorithm",
                position: 0,
                reason: format_smolstr!(
                    "expected one of {}, got {value:?}",
                    canonical_vocabulary()
                ),
            })
    }
}

fn canonical_vocabulary() -> String {
    EdgeAlgorithm::ALL
        .iter()
        .map(|algorithm| algorithm.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

impl AsRef<str> for EdgeAlgorithm {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for EdgeAlgorithm {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for EdgeAlgorithm {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EdgeAlgorithm {
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
    use super::EdgeAlgorithm;

    #[test]
    fn names_round_trip_case_insensitively() {
        for algorithm in EdgeAlgorithm::ALL {
            assert_eq!(
                EdgeAlgorithm::from_str(algorithm.as_str()).unwrap(),
                algorithm
            );
            assert_eq!(
                EdgeAlgorithm::from_str(&algorithm.as_str().to_uppercase()).unwrap(),
                algorithm
            );
        }
    }

    #[test]
    fn spherical_is_the_default_both_formats_fill() {
        assert_eq!(EdgeAlgorithm::default(), EdgeAlgorithm::Spherical);
    }

    #[test]
    fn unknown_name_reports_the_input_and_vocabulary() {
        let error = EdgeAlgorithm::from_str("euclidean").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("\"euclidean\""), "{message}");
        assert!(message.contains("spherical"), "{message}");
    }
}
