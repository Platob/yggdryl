//! Ordered software/protocol versions as one generic scalar value.
//!
//! A version is a four-byte numeric value: an eight-bit major, an eight-bit
//! minor, and a sixteen-bit patch. Missing minor and patch components are zero;
//! canonical text omits trailing zero components. A compact FIX `SP` suffix
//! supplies the numeric patch, case-insensitively: `5.0sp250` becomes `5.0.250`.
//! Qualifiers are not stored: a patch tail that states no number is folded into
//! the sixteen bits by the crate's stable XXH3 instead of refusing the version,
//! so parsing fails only on the major, the minor, or empty text. Parsing and numeric comparison allocate nothing, and cloning
//! copies the four-byte value. This is neither an ASCII-width
//! datatype nor a static coded vocabulary. Arrow stores the canonical text as
//! Utf8; its extension name preserves the datatype on a field round trip.
//! Arrow's own string ordering is consequently lexicographic—[`Version::cmp`]
//! is the ordering contract.

pub use value::Version;

use crate::TypedField;
use crate::types::typed::define_field_types;

#[cfg(feature = "arrow")]
/// Arrow casts owned by the version datatype.
pub(crate) mod casts {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, StringArray};
    use arrow_buffer::BooleanBuffer;
    use arrow_schema::DataType as ArrowDataType;

    use crate::arrow::{Error, Result};
    use crate::types::budget::MaterializationBudget;
    use crate::types::cast::{arrow_cast_exposed, downcast};
    use crate::types::nested::casts::is_exposed;
    use crate::{DataType, Field, Version};

    /// Parse and canonicalize every exposed text cell into version Utf8 storage.
    pub(crate) fn ingest_version_array(
        array: &ArrayRef,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let text = if array.data_type() == &ArrowDataType::Utf8 {
            Arc::clone(array)
        } else {
            arrow_cast_exposed(
                array,
                &ArrowDataType::Utf8,
                true,
                exposure,
                &Field::new(field.name(), DataType::utf8(), true),
                budget,
            )?
        };
        let source = downcast::<StringArray>(text.as_ref())?;
        budget.add_array(field.dtype(), source.len())?;
        let mut values = Vec::with_capacity(source.len());
        let mut payload = 0_usize;
        for index in 0..source.len() {
            if !is_exposed(exposure, index) || source.is_null(index) {
                values.push(None);
                continue;
            }
            let raw = source.value(index);
            let version = raw.parse::<Version>().map_err(|error| {
                Error::IncompatibleSchema(format!(
                    "field {:?} row {index}: {raw:?} does not read as version: {error}",
                    field.name()
                ))
            })?;
            let canonical = version.to_string();
            payload = payload.saturating_add(canonical.len());
            values.push(Some(canonical));
        }
        budget.add_bytes(payload)?;
        Ok(Arc::new(StringArray::from(values)))
    }

    /// Return whether an Arrow layout holds one of the three text forms.
    pub(crate) fn is_text_layout(dtype: &ArrowDataType) -> bool {
        matches!(
            dtype,
            ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View
        )
    }
}

// ------------------------------------------------------------------------
// Version field marker and typed aliases.
// ------------------------------------------------------------------------

define_field_types!(VersionType, Version, crate::DataType::Version);

/// A version-typed field.
pub type VersionField = TypedField<VersionType>;
/// Canonical parsing, rendering, and ordering for [`Version`].
mod value {
    use std::cmp::Ordering;
    use std::fmt;
    use std::str::FromStr;

    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use smol_str::SmolStr;

    use crate::{DataType, Error, Result, Scalar, Value};

    /// Three numeric version components in four bytes.
    ///
    /// Major and minor are unsigned bytes; patch is an unsigned 16-bit number.
    /// The numeric tuple owns equality and ordering. Text omits trailing zero
    /// components, so `5`, `5.0`, and `5.0.0` are the same value.
    /// A case-insensitive FIX `SP` suffix supplies the patch: `5.0sp250` is
    /// `5.0.250`, with no separately stored qualifier.
    ///
    /// ```
    /// use yggdryl::Version;
    /// let version = "5.0sp250".parse::<Version>()?;
    /// assert_eq!(version, Version::new(5, 0, 250));
    /// assert_eq!(version.to_string(), "5.0.250");
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # The patch is best effort
    ///
    /// Major and minor are strict: a version whose first two components are not
    /// decimal numbers under 256 is refused, and so is empty text. The patch is
    /// not. A tail stating no number - a qualifier, a fourth component, an
    /// extension pack - folds into the patch's sixteen bits through the crate's
    /// stable XXH3 rather than refusing the version, so anything that names a
    /// major parses.
    ///
    /// A folded patch is an identity, not a quantity: it orders arbitrarily
    /// against a stated one, two unlike tails can fold together, and the
    /// canonical text states the fold rather than the tail it came from. What it
    /// buys is that the same tail always reads as the same version.
    ///
    /// ```
    /// use yggdryl::Version;
    /// let qualified = "1.0-rc1".parse::<Version>()?;
    /// assert_eq!((qualified.major(), qualified.minor()), (1, 0));
    /// assert_eq!(qualified, "1.0-rc1".parse::<Version>()?);
    /// assert_ne!(qualified, "1.0-rc2".parse::<Version>()?);
    /// assert!("".parse::<Version>().is_err());
    /// assert!("256.0".parse::<Version>().is_err());
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    #[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
    #[repr(C)]
    pub struct Version {
        major: u8,
        minor: u8,
        patch: u16,
    }

    impl Version {
        /// The maximum number of numeric components.
        pub const MAX_PARTS: usize = 3;

        /// The lower bound of the numeric version value space.
        pub const MIN: Self = Self::new(0, 0, 0);

        /// The upper bound of the numeric version value space.
        pub const MAX: Self = Self::new(u8::MAX, u8::MAX, u16::MAX);

        /// Constructs a version from its exact-width numeric components.
        ///
        /// ```
        /// use yggdryl::Version;
        /// let version = Version::new(5, 2, 300);
        /// assert_eq!(version.to_string(), "5.2.300");
        /// assert_eq!((version.major(), version.minor(), version.patch()), (5, 2, 300));
        /// ```
        pub const fn new(major: u8, minor: u8, patch: u16) -> Self {
            Self {
                major,
                minor,
                patch,
            }
        }

        /// The major component.
        pub const fn major(self) -> u8 {
            self.major
        }

        /// The minor component, zero when omitted in text.
        pub const fn minor(self) -> u8 {
            self.minor
        }

        /// The patch component, zero when omitted in text.
        pub const fn patch(self) -> u16 {
            self.patch
        }

        /// Number of UTF-8 bytes in the canonical rendering.
        pub(crate) fn rendered_len(&self) -> usize {
            decimal_digits(u16::from(self.major))
                + if self.minor != 0 || self.patch != 0 {
                    1 + decimal_digits(u16::from(self.minor))
                } else {
                    0
                }
                + if self.patch != 0 {
                    1 + decimal_digits(self.patch)
                } else {
                    0
                }
        }
    }

    const _: () = assert!(std::mem::size_of::<Version>() == 4);

    const fn decimal_digits(value: u16) -> usize {
        match value {
            0..=9 => 1,
            10..=99 => 2,
            100..=999 => 3,
            1_000..=9_999 => 4,
            _ => 5,
        }
    }

    impl FromStr for Version {
        type Err = Error;

        #[allow(clippy::cast_possible_truncation)] // Each component is bounded before its narrowing cast.
        fn from_str(text: &str) -> Result<Self> {
            let bytes = text.as_bytes();
            let (major, mut position) = component(bytes, 0, u32::from(u8::MAX), "major")?;
            let mut minor = 0_u32;
            if bytes.get(position) == Some(&b'.') && digit_at(bytes, position + 1) {
                let (value, after) = component(bytes, position + 1, u32::from(u8::MAX), "minor")?;
                minor = value;
                position = after;
            }
            Ok(Self::new(
                major as u8,
                minor as u8,
                patch_of(&text[position..]),
            ))
        }
    }

    /// Whether a decimal digit stands at `position`.
    fn digit_at(bytes: &[u8], position: usize) -> bool {
        matches!(bytes.get(position), Some(b'0'..=b'9'))
    }

    /// One strict decimal component, bounded before it narrows.
    ///
    /// Major and minor stay strict because they are what a version is ordered by
    /// first: a byte that is not a digit there is a refusal, not a fallback.
    fn component(bytes: &[u8], start: usize, maximum: u32, what: &'static str) -> Result<(u32, usize)> {
        let mut position = start;
        let mut value = 0_u32;
        while let Some(byte @ b'0'..=b'9') = bytes.get(position).copied() {
            value = value * 10 + u32::from(byte - b'0');
            if value > maximum {
                return Err(parse_error(
                    position,
                    if what == "major" {
                        "expected a major version in 0..=255"
                    } else {
                        "expected a minor version in 0..=255"
                    },
                ));
            }
            position += 1;
        }
        if position == start {
            return Err(parse_error(
                start,
                if what == "major" {
                    "expected a decimal major version"
                } else {
                    "expected a decimal minor version"
                },
            ));
        }
        Ok((value, position))
    }

    /// The patch a tail states, or the stable hash of a tail that states no number.
    ///
    /// The tail is whatever follows the major and minor. `.250` and a
    /// case-insensitive FIX `sp250` both state the number 250. Anything else -
    /// a qualifier, a fourth component, an extension pack, trailing bytes - is
    /// hashed into the patch rather than refused, so a version always parses.
    fn patch_of(tail: &str) -> u16 {
        if tail.is_empty() {
            return 0;
        }
        let digits = match tail.as_bytes() {
            [b'.', rest @ ..] => rest,
            [b'S' | b's', b'P' | b'p', rest @ ..] => rest,
            _ => return hashed_patch(tail),
        };
        if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
            return hashed_patch(tail);
        }
        let mut value = 0_u32;
        for byte in digits {
            value = value * 10 + u32::from(byte - b'0');
            if value > u32::from(u16::MAX) {
                return hashed_patch(tail);
            }
        }
        value as u16
    }

    /// A tail no number can be read from, folded into the patch's sixteen bits.
    ///
    /// The fold lands in `1..=65535` so a tail that says something never renders
    /// as a version that says nothing. Sixteen bits cannot separate every tail
    /// there is: two unlike tails can fold together, and a fold can equal a patch
    /// some other version states as a number.
    fn hashed_patch(tail: &str) -> u16 {
        let hash = crate::hashing::xxhash::xxh3(tail.as_bytes());
        let folded = (hash ^ (hash >> 16) ^ (hash >> 32) ^ (hash >> 48)) as u16;
        1 + (folded % u16::MAX)
    }

    fn parse_error(position: usize, reason: &'static str) -> Error {
        Error::Parse {
            target: "version",
            position,
            reason: SmolStr::new_static(reason),
        }
    }

    impl fmt::Display for Version {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "{}", self.major)?;
            if self.minor != 0 || self.patch != 0 {
                write!(formatter, ".{}", self.minor)?;
            }
            if self.patch != 0 {
                write!(formatter, ".{}", self.patch)?;
            }
            Ok(())
        }
    }

    impl PartialOrd for Version {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for Version {
        fn cmp(&self, other: &Self) -> Ordering {
            (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch))
        }
    }

    impl Serialize for Version {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.collect_str(self)
        }
    }

    impl<'de> Deserialize<'de> for Version {
        fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let text = <&str>::deserialize(deserializer)?;
            text.parse().map_err(D::Error::custom)
        }
    }

    impl Value for Version {
        fn dtype(&self) -> Result<DataType> {
            Ok(DataType::Version)
        }

        fn into_scalar(self) -> Scalar {
            Scalar::Version(self)
        }

        fn from_scalar(value: &Scalar) -> Option<&Self> {
            match value {
                Scalar::Version(value) => Some(value),
                _ => None,
            }
        }
    }

    impl From<Version> for Scalar {
        fn from(value: Version) -> Self {
            Self::Version(value)
        }
    }

    impl TryFrom<&str> for Version {
        type Error = Error;

        fn try_from(value: &str) -> Result<Self> {
            value.parse()
        }
    }

    impl TryFrom<String> for Version {
        type Error = Error;

        fn try_from(value: String) -> Result<Self> {
            value.parse()
        }
    }
}

/// The Arrow extension name preserving [`crate::DataType::Version`] over
/// its Utf8 storage.
pub(crate) const VERSION_EXTENSION_NAME: &str = "yggdryl.version";

#[cfg(test)]
/// The version invariant an integration test cannot reach.
///
/// `rendered_len` is crate-private: it is the length the digest feed writes
/// before a version's canonical text, so it has to agree with what `Display`
/// actually renders for every reachable value. Everything a caller can observe
/// lives in `tests/types/version.rs`.
mod tests {
    use crate::Version;

    #[test]
    fn the_rendered_length_agrees_with_what_display_writes() {
        for text in [
            "0",
            "1",
            "5",
            "5.0.2",
            "5.0.250",
            "1.0.256",
            "255.255.65535",
            "5.0SP2",
            "5.0sp250",
        ] {
            let value: Version = text.parse().unwrap();
            assert_eq!(value.rendered_len(), value.to_string().len(), "{text}");
        }
        for major in [0_u8, 1, 9, 10, 99, 100, 255] {
            for minor in [0_u8, 7, 42, 255] {
                for patch in [0_u16, 1, 250, 1_000, 65_535] {
                    let value = Version::new(major, minor, patch);
                    assert_eq!(value.rendered_len(), value.to_string().len(), "{value}");
                }
            }
        }
    }
}
