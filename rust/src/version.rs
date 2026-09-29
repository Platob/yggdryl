//! Ordered software/protocol versions as one generic scalar value.
//!
//! A version is a sixteen-bit major, a sixteen-bit minor, and an optional
//! patch held as text. A missing minor is zero and a missing patch is none;
//! canonical text omits a zero minor no patch follows. A patch stating a
//! number is held as its digits, so `5.0.0` is `5` and `5.0.007` is `5.0.7`,
//! and a compact FIX `SP` suffix states one case-insensitively: `5.0sp250`
//! becomes `5.0.250`. Any other tail is the patch as written - a qualifier, a
//! fourth component - so parsing fails only on the major, the minor, or empty
//! text. Parsing a patch of up to 23 bytes, comparing and rendering allocate
//! nothing. This is neither an ASCII-width datatype nor a static coded
//! vocabulary. Arrow stores the canonical text as Utf8; its extension name
//! preserves the datatype on a field round trip. Arrow's own string ordering
//! is consequently lexicographic—[`Version::cmp`] is the ordering contract.

pub use value::Version;

use crate::typed::define_field_types;

/// Arrow casts owned by the version datatype.
pub(crate) mod casts {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, StringArray};
    use arrow_buffer::BooleanBuffer;
    use arrow_schema::DataType as ArrowDataType;

    use crate::arrow::{Error, Result};
    use crate::budget::MaterializationBudget;
    use crate::cast::columns::is_exposed;
    use crate::cast::{arrow_cast_exposed, downcast};
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

define_field_types!(VersionType, Version);

/// Canonical parsing, rendering, and ordering for [`Version`].
mod value {
    use std::cmp::Ordering;
    use std::fmt::{self, Write as _};
    use std::str::FromStr;

    use serde::de::Visitor;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use smol_str::SmolStr;

    use crate::{DataType, Error, Result, Scalar, Value};

    /// A sixteen-bit major and minor, and an optional patch held as text.
    ///
    /// The major and minor are numbers; the patch is what the version states
    /// after them. A patch stating a number is held as that number's digits,
    /// so `5`, `5.0` and `5.0.0` are one value and `5.0.007` is `5.0.7`. A
    /// case-insensitive FIX `SP` suffix states the patch number too: `5.0sp250`
    /// is `5.0.250`, with no separately stored qualifier.
    ///
    /// ```
    /// use yggdryl::Version;
    /// let version = "5.0sp250".parse::<Version>()?;
    /// assert_eq!(version, Version::new(5, 0, Some("250")));
    /// assert_eq!(version.to_string(), "5.0.250");
    /// assert_eq!((version.major(), version.minor(), version.patch()), (5, 0, Some("250")));
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # The patch is best effort
    ///
    /// The numbers are strict: text that does not open with a decimal major
    /// under 65536 is refused, empty text included, and so is a minor whose
    /// digits pass 65535. The patch is not. A tail stating no number - a
    /// qualifier, a fourth component, an extension pack - is the patch as
    /// written rather than a refusal, so anything that names a major parses and
    /// nothing it states is lost.
    ///
    /// ```
    /// use yggdryl::Version;
    /// let qualified = "1.0-rc1".parse::<Version>()?;
    /// assert_eq!((qualified.major(), qualified.minor()), (1, 0));
    /// assert_eq!(qualified.patch(), Some("-rc1"));
    /// assert_eq!(qualified.to_string(), "1.0-rc1");
    /// assert!(qualified < "1.0-rc2".parse::<Version>()?);
    /// assert!("".parse::<Version>().is_err());
    /// assert!("65536.0".parse::<Version>().is_err());
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Order
    ///
    /// The major, then the minor, then the patch: no patch first, then two
    /// patches in natural order - a run of digits compares as the number it
    /// spells and every other byte as itself - so `5.0.2 < 5.0.10` and
    /// `1.0-rc2 < 1.0-rc10`. Patches that order equal that way, `rc01` and
    /// `rc1`, fall back to their bytes, which keeps the order total and in
    /// agreement with equality.
    #[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
    pub struct Version {
        major: u16,
        minor: u16,
        /// Never empty; a number is held as its digits without leading zeros,
        /// and zero as no patch, so equal versions hold equal bytes.
        patch: Option<SmolStr>,
    }

    impl Version {
        /// The lower bound of the version order: `0`, no minor, no patch.
        pub const MIN: Self = Self {
            major: 0,
            minor: 0,
            patch: None,
        };

        /// Constructs a version from its components.
        ///
        /// The patch is canonicalized as the parser canonicalizes one: a number
        /// is held as its digits without leading zeros, and zero or empty text
        /// states no patch. It is taken as written otherwise - an `sp` prefix
        /// is the grammar's, so `Some("sp2")` is the patch `sp2`.
        ///
        /// ```
        /// use yggdryl::Version;
        /// let version = Version::new(5, 2, Some("0300"));
        /// assert_eq!(version.to_string(), "5.2.300");
        /// assert_eq!((version.major(), version.minor(), version.patch()), (5, 2, Some("300")));
        /// assert_eq!(Version::new(5, 0, Some("0")), Version::new(5, 0, None));
        /// ```
        pub fn new(major: u16, minor: u16, patch: Option<&str>) -> Self {
            Self {
                major,
                minor,
                patch: patch.and_then(canonical_patch),
            }
        }

        /// The major component.
        pub const fn major(&self) -> u16 {
            self.major
        }

        /// The minor component, zero when omitted in text.
        pub const fn minor(&self) -> u16 {
            self.minor
        }

        /// The patch, `None` when the version states none or states zero.
        pub fn patch(&self) -> Option<&str> {
            self.patch.as_deref()
        }

        /// Number of UTF-8 bytes in the canonical rendering.
        pub(crate) fn rendered_len(&self) -> usize {
            decimal_digits(self.major)
                + if self.minor != 0 || self.patch.is_some() {
                    1 + decimal_digits(self.minor)
                } else {
                    0
                }
                + self
                    .patch()
                    .map_or(0, |patch| usize::from(is_dotted(patch)) + patch.len())
        }
    }

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

        fn from_str(text: &str) -> Result<Self> {
            let bytes = text.as_bytes();
            let (major, mut position) = component(bytes, 0, &MAJOR)?;
            let mut minor = 0;
            if bytes.get(position) == Some(&b'.') && digit_at(bytes, position + 1) {
                let (value, after) = component(bytes, position + 1, &MINOR)?;
                minor = value;
                position = after;
            }
            Ok(Self {
                major,
                minor,
                patch: patch_of(&text[position..]),
            })
        }
    }

    /// Whether a decimal digit stands at `position`.
    fn digit_at(bytes: &[u8], position: usize) -> bool {
        matches!(bytes.get(position), Some(b'0'..=b'9'))
    }

    /// What a strict component is refused with: no digit, or too many.
    struct Component {
        missing: &'static str,
        overflow: &'static str,
    }

    const MAJOR: Component = Component {
        missing: "expected a decimal major version",
        overflow: "expected a major version in 0..=65535",
    };

    const MINOR: Component = Component {
        missing: "expected a decimal minor version",
        overflow: "expected a minor version in 0..=65535",
    };

    /// One strict decimal component, refused at the byte that overflows it.
    ///
    /// Major and minor stay strict because they are what a version is ordered by
    /// first: a byte that is not a digit there is a refusal, not a fallback.
    fn component(bytes: &[u8], start: usize, what: &Component) -> Result<(u16, usize)> {
        let mut position = start;
        let mut value = 0_u16;
        while let Some(byte @ b'0'..=b'9') = bytes.get(position).copied() {
            value = value
                .checked_mul(10)
                .and_then(|value| value.checked_add(u16::from(byte - b'0')))
                .ok_or_else(|| parse_error(position, what.overflow))?;
            position += 1;
        }
        if position == start {
            return Err(parse_error(start, what.missing));
        }
        Ok((value, position))
    }

    /// The patch a tail states.
    ///
    /// The tail is whatever follows the major and minor. A compact FIX service
    /// pack - `sp` in any case and nothing but digits after it, straight after
    /// the last number read - states the number it carries, so `5SP2` is
    /// `5.0.2` as `5.0SP2` is. Otherwise one leading `.`
    /// separates the patch, and what follows it is the patch as written: a
    /// qualifier, a fourth component and trailing bytes are all read rather
    /// than refused.
    fn patch_of(tail: &str) -> Option<SmolStr> {
        let patch = match tail.as_bytes() {
            [b'S' | b's', b'P' | b'p', digits @ ..]
                if !digits.is_empty() && digits.iter().all(u8::is_ascii_digit) =>
            {
                &tail[2..]
            }
            [b'.', ..] => &tail[1..],
            _ => tail,
        };
        canonical_patch(patch)
    }

    /// A patch in the one spelling equal versions share.
    ///
    /// A number is its digits without leading zeros, and zero - like empty
    /// text - states no patch; any other text is held as written.
    fn canonical_patch(patch: &str) -> Option<SmolStr> {
        if patch.bytes().all(|byte| byte.is_ascii_digit()) {
            let digits = patch.trim_start_matches('0');
            return (!digits.is_empty()).then(|| SmolStr::new(digits));
        }
        Some(SmolStr::new(patch))
    }

    /// Whether the canonical text writes a `.` before `patch`.
    ///
    /// A patch opening with a letter, a digit or a dot needs one: without it a
    /// digit would read as the minor, a dot as the separator, and `sp` with
    /// digits as a service pack. Any other opening byte - `-rc1`, `+meta`,
    /// `_EP2`, `界` - separates the patch on its own.
    fn is_dotted(patch: &str) -> bool {
        patch
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'.')
    }

    /// Natural order over two patches, total and agreeing with equality.
    ///
    /// A run of digits compares as the number it spells, leading zeros aside,
    /// and every other byte as itself; a patch that ends first orders first.
    /// Patches equal under that reading compare by their bytes.
    fn natural(left: &str, right: &str) -> Ordering {
        let (mut mine, mut theirs) = (left.as_bytes(), right.as_bytes());
        loop {
            match (mine.first(), theirs.first()) {
                (Some(a), Some(b)) if a.is_ascii_digit() && b.is_ascii_digit() => {
                    let (my_run, my_rest) = digit_run(mine);
                    let (their_run, their_rest) = digit_run(theirs);
                    let ordering = my_run
                        .len()
                        .cmp(&their_run.len())
                        .then_with(|| my_run.cmp(their_run));
                    if ordering.is_ne() {
                        return ordering;
                    }
                    (mine, theirs) = (my_rest, their_rest);
                }
                (Some(a), Some(b)) if a != b => return a.cmp(b),
                (Some(_), Some(_)) => (mine, theirs) = (&mine[1..], &theirs[1..]),
                (a, b) => {
                    return a.is_some().cmp(&b.is_some()).then_with(|| left.cmp(right));
                }
            }
        }
    }

    /// The digits a run opens with, leading zeros dropped, and what follows it.
    fn digit_run(bytes: &[u8]) -> (&[u8], &[u8]) {
        let end = bytes
            .iter()
            .position(|byte| !byte.is_ascii_digit())
            .unwrap_or(bytes.len());
        let (run, rest) = bytes.split_at(end);
        let first = run
            .iter()
            .position(|byte| *byte != b'0')
            .unwrap_or(run.len());
        (&run[first..], rest)
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
            if self.minor != 0 || self.patch.is_some() {
                write!(formatter, ".{}", self.minor)?;
            }
            if let Some(patch) = self.patch() {
                if is_dotted(patch) {
                    formatter.write_char('.')?;
                }
                formatter.write_str(patch)?;
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
            (self.major, self.minor)
                .cmp(&(other.major, other.minor))
                .then_with(|| match (self.patch(), other.patch()) {
                    (Some(mine), Some(theirs)) => natural(mine, theirs),
                    (mine, theirs) => mine.cmp(&theirs),
                })
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
            // A visitor rather than a borrowed `&str`: a patch may hold text a
            // format escapes, which a deserializer can only hand over unescaped
            // in a buffer of its own.
            struct VersionVisitor;

            impl Visitor<'_> for VersionVisitor {
                type Value = Version;

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("a version string")
                }

                fn visit_str<E>(self, text: &str) -> std::result::Result<Version, E>
                where
                    E: serde::de::Error,
                {
                    text.parse().map_err(E::custom)
                }
            }

            deserializer.deserialize_str(VersionVisitor)
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

// ------------------------------------------------------------------------
// Arrow projection: the canonical text, under this family's extension name.
// ------------------------------------------------------------------------

impl VersionType {
    /// The Arrow storage a version column lays out: its canonical text.
    pub(crate) const fn arrow_storage() -> arrow_schema::DataType {
        arrow_schema::DataType::Utf8
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/version.rs` pins and a caller cannot reach.
    //!
    //! `rendered_len` is crate-private: it is the length the digest feed
    //! writes before a version's canonical text, so it has to agree with what
    //! `Display` actually renders for every reachable value. The forwarder
    //! changes no visibility; everything a caller can observe lives in
    //! `rust/tests/root/version.rs`.

    use crate::Version;

    /// The byte count a version's canonical text renders to.
    #[must_use]
    pub fn rendered_len(value: &Version) -> usize {
        value.rendered_len()
    }
}
