//! # yggdryl-version
//!
//! A standalone `major.minor.patch` [`Version`] type for the **yggdryl**
//! project, built on the [`yggdryl-core`](https://crates.io/crates/yggdryl-core)
//! parsing traits.

use std::fmt;

pub use yggdryl_core::{FromInput, Mapping, ToOutput};

/// Error returned when [`Version::from_`] cannot interpret its input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionError {
    /// The input was empty.
    Empty,
    /// More than three dot-separated components were given.
    TooManyComponents,
    /// A component was not a non-negative integer.
    InvalidNumber(String),
}

impl fmt::Display for VersionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionError::Empty => write!(f, "version is empty"),
            VersionError::TooManyComponents => {
                write!(f, "version has more than three components")
            }
            VersionError::InvalidNumber(part) => {
                write!(f, "version component '{part}' is not a number")
            }
        }
    }
}

impl std::error::Error for VersionError {}

/// A generic `major.minor.patch` version.
///
/// Ordering is numeric and field-major (`major`, then `minor`, then `patch`), so
/// `Version`s sort the way you would expect. Parsing accepts one, two or three
/// components; any that are omitted default to `0`.
///
/// ```
/// use yggdryl_version::{FromInput, Version};
///
/// let v = Version::from_str("1.4.2").unwrap();
/// assert_eq!((v.major(), v.minor(), v.patch()), (1, 4, 2));
/// assert_eq!(Version::from_str("2").unwrap(), Version::new(2, 0, 0));
/// assert!(Version::new(1, 4, 2) < Version::new(1, 10, 0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    /// Creates a version from its components.
    pub fn new(major: u64, minor: u64, patch: u64) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    /// The major component.
    pub fn major(&self) -> u64 {
        self.major
    }

    /// The minor component.
    pub fn minor(&self) -> u64 {
        self.minor
    }

    /// The patch component.
    pub fn patch(&self) -> u64 {
        self.patch
    }

    /// Returns a copy of this version, overriding any component for which `Some`
    /// is given and keeping `self`'s value otherwise. Call `copy(None, …)` to
    /// clone.
    pub fn copy(&self, major: Option<u64>, minor: Option<u64>, patch: Option<u64>) -> Version {
        Version {
            major: major.unwrap_or(self.major),
            minor: minor.unwrap_or(self.minor),
            patch: patch.unwrap_or(self.patch),
        }
    }

    /// Returns a copy with the major component replaced.
    pub fn with_major(mut self, major: u64) -> Version {
        self.major = major;
        self
    }

    /// Returns a copy with the minor component replaced.
    pub fn with_minor(mut self, minor: u64) -> Version {
        self.minor = minor;
        self
    }

    /// Returns a copy with the patch component replaced.
    pub fn with_patch(mut self, patch: u64) -> Version {
        self.patch = patch;
        self
    }
}

impl FromInput for Version {
    type Err = VersionError;

    /// Parses a `major[.minor[.patch]]` string. Every component must be a
    /// non-negative integer and there may be at most three; omitted components
    /// default to `0`.
    fn from_str(input: &str) -> Result<Version, VersionError> {
        if input.is_empty() {
            return Err(VersionError::Empty);
        }
        let mut parts = [0u64; 3];
        for (index, part) in input.split('.').enumerate() {
            if index == 3 {
                return Err(VersionError::TooManyComponents);
            }
            parts[index] = part
                .parse::<u64>()
                .map_err(|_| VersionError::InvalidNumber(part.to_string()))?;
        }
        Ok(Version {
            major: parts[0],
            minor: parts[1],
            patch: parts[2],
        })
    }

    /// Builds a [`Version`] from a [`Mapping`]. Recognised keys: `major`, `minor`
    /// and `patch`; any omitted default to `0`.
    fn from_mapping(fields: &Mapping) -> Result<Version, VersionError> {
        let component = |key: &str| -> Result<u64, VersionError> {
            match fields.get(key) {
                Some(value) => value
                    .parse::<u64>()
                    .map_err(|_| VersionError::InvalidNumber(value.clone())),
                None => Ok(0),
            }
        };
        Ok(Version {
            major: component("major")?,
            minor: component("minor")?,
            patch: component("patch")?,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}
impl ToOutput for Version {
    fn to_str(&self, _encode: bool) -> String {
        self.to_string()
    }

    fn to_mapping(&self) -> Mapping {
        Mapping::from([
            ("major".to_string(), self.major.to_string()),
            ("minor".to_string(), self.minor.to_string()),
            ("patch".to_string(), self.patch.to_string()),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_parse_full() {
        let v = Version::from_str("1.4.2").unwrap();
        assert_eq!((v.major(), v.minor(), v.patch()), (1, 4, 2));
    }
    #[test]
    fn version_parse_partial_defaults_to_zero() {
        assert_eq!(Version::from_str("2").unwrap(), Version::new(2, 0, 0));
        assert_eq!(Version::from_str("2.5").unwrap(), Version::new(2, 5, 0));
    }
    #[test]
    fn version_errors() {
        assert_eq!(Version::from_str(""), Err(VersionError::Empty));
        assert_eq!(
            Version::from_str("1.2.3.4"),
            Err(VersionError::TooManyComponents)
        );
        assert_eq!(
            Version::from_str("1.x.0"),
            Err(VersionError::InvalidNumber("x".to_string()))
        );
        assert_eq!(
            Version::from_str("1..0"),
            Err(VersionError::InvalidNumber(String::new()))
        );
    }
    #[test]
    fn version_from_mapping() {
        let fields = Mapping::from([
            ("major".to_string(), "1".to_string()),
            ("minor".to_string(), "4".to_string()),
        ]);
        assert_eq!(
            Version::from_mapping(&fields).unwrap(),
            Version::new(1, 4, 0)
        );
    }
    #[test]
    fn version_orders_numerically() {
        assert!(Version::new(1, 4, 2) < Version::new(1, 10, 0));
        assert!(Version::new(2, 0, 0) > Version::new(1, 99, 99));
        let mut versions = [
            Version::new(1, 2, 0),
            Version::new(1, 0, 5),
            Version::new(0, 9, 9),
        ];
        versions.sort();
        assert_eq!(
            versions,
            [
                Version::new(0, 9, 9),
                Version::new(1, 0, 5),
                Version::new(1, 2, 0),
            ]
        );
    }
    #[test]
    fn version_round_trips() {
        assert_eq!(Version::from_str("1.4.2").unwrap().to_string(), "1.4.2");
        assert_eq!(Version::from_str("3").unwrap().to_string(), "3.0.0");
    }
    #[test]
    fn version_builders() {
        let v = Version::new(1, 0, 0).with_minor(4).with_patch(2);
        assert_eq!(v, Version::new(1, 4, 2));
        assert_eq!(v.with_major(2), Version::new(2, 4, 2));
        assert_eq!(v, Version::new(1, 4, 2));
    }

    #[test]
    fn version_copy() {
        let v = Version::new(1, 4, 2);
        assert_eq!(v.copy(Some(2), None, None), Version::new(2, 4, 2));
    }

    #[test]
    fn version_to_output_round_trips() {
        let v = Version::new(1, 4, 2);
        assert_eq!(v.to_str(true), "1.4.2");
        let m = v.to_mapping();
        assert_eq!(m.get("minor"), Some(&"4".to_string()));
        assert_eq!(Version::from_(&m).unwrap(), v);
    }
}
