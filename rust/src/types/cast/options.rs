//! What a cast is allowed to do about failure, absence, and representation.
//!
//! Three independent questions travel together through every Arrow cast, and
//! confusing them is what a single `safe` flag invited. `safe` decides whether
//! a *present* value may be converted; [`Nullability`] decides whether a
//! declared value may be *absent* at all; [`Representation`] decides what a
//! same-width pair actually carries. A conversion that fails under `safe`
//! produces a null, and whether that null is then repaired or refused is the
//! nullability policy's answer, not the conversion's.

use std::fmt;
use std::str::FromStr;

use smol_str::format_smolstr;

use crate::{Error, Result};

/// What a cast does about a non-nullable target field the source cannot fill.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Nullability {
    /// Repair: a required field absent from the source, or null within it,
    /// takes the target's canonical [default](crate::Field::default_value).
    #[default]
    Default,
    /// Refuse: a required field must be carried by the source and hold a value
    /// in every exposed row, and the error names its full path.
    Strict,
}

impl Nullability {
    /// Every policy in canonical order.
    pub const ALL: [Self; 2] = [Self::Default, Self::Strict];

    /// Parse one canonical policy name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the complete accepted vocabulary.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the canonical lowercase spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Strict => "strict",
        }
    }

    /// Returns whether absence is refused rather than repaired.
    pub const fn is_strict(self) -> bool {
        matches!(self, Self::Strict)
    }
}

impl AsRef<str> for Nullability {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Nullability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Nullability {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = value.trim();
        Self::ALL
            .into_iter()
            .find(|policy| normalized.eq_ignore_ascii_case(policy.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "nullability",
                position: 0,
                reason: format_smolstr!(
                    "expected one of {}, got {value:?}",
                    Self::ALL
                        .iter()
                        .map(|policy| policy.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
    }
}

/// What a cast carries across two datatypes of the same physical width.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Representation {
    /// The value crosses: a number keeps its arithmetic meaning, and one
    /// outside the target's range is refused - or nulled, under `safe`.
    #[default]
    Value,
    /// The bits cross: two datatypes laid out as one fixed-width buffer of the
    /// same byte width are the same bytes under two readings, so every source
    /// bit pattern maps and the buffer is shared rather than rebuilt.
    /// `u64::MAX` reads as `-1_i64`, those eight bytes read as an
    /// `int64`/`uint64`/`float64`/`fixed_size_binary(8)` alike, and every one
    /// of those round-trips back.
    ///
    /// It is a preference, not a mode: a pair that is not laid out that way
    /// takes the ordinary conversion, so asking for bits never silently
    /// reinterprets something that is not the same bytes.
    Bits,
}

impl Representation {
    /// Every reading in canonical order.
    pub const ALL: [Self; 2] = [Self::Value, Self::Bits];

    /// Parse one canonical reading name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the complete accepted vocabulary.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the canonical lowercase spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Value => "value",
            Self::Bits => "bits",
        }
    }

    /// Returns whether a same-width pair is read as the same bytes.
    pub const fn is_bits(self) -> bool {
        matches!(self, Self::Bits)
    }
}

impl AsRef<str> for Representation {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Representation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Representation {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = value.trim();
        Self::ALL
            .into_iter()
            .find(|reading| normalized.eq_ignore_ascii_case(reading.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "representation",
                position: 0,
                reason: format_smolstr!(
                    "expected one of {}, got {value:?}",
                    Self::ALL
                        .iter()
                        .map(|reading| reading.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
    }
}

/// The three independent decisions every Arrow cast makes.
///
/// ```
/// use yggdryl::{ArrowCastOptions, Nullability, Representation};
///
/// // The default is today's contract: convert leniently, repair absence, and
/// // carry the value rather than the bytes under it.
/// let lenient = ArrowCastOptions::new();
/// assert!(lenient.is_safe());
/// assert_eq!(lenient.nullability(), Nullability::Default);
/// assert_eq!(lenient.representation(), Representation::Value);
///
/// // The three answers move independently.
/// let strict = ArrowCastOptions::new()
///     .with_nullability(Nullability::Strict)
///     .with_representation(Representation::Bits);
/// assert!(strict.is_safe());
/// assert!(strict.nullability().is_strict());
/// assert!(strict.representation().is_bits());
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ArrowCastOptions {
    safe: bool,
    nullability: Nullability,
    representation: Representation,
}

impl ArrowCastOptions {
    /// The lenient conversion and the repairing nullability policy.
    pub const fn new() -> Self {
        Self {
            safe: true,
            nullability: Nullability::Default,
            representation: Representation::Value,
        }
    }

    /// Set whether a failed conversion becomes null rather than an error.
    pub const fn with_safe(mut self, safe: bool) -> Self {
        self.safe = safe;
        self
    }

    /// Set what happens to a required field the source cannot fill.
    pub const fn with_nullability(mut self, nullability: Nullability) -> Self {
        self.nullability = nullability;
        self
    }

    /// Returns whether a failed conversion becomes null rather than an error.
    pub const fn is_safe(self) -> bool {
        self.safe
    }

    /// Set what a same-width pair carries: the value, or the bytes under it.
    pub const fn with_representation(mut self, representation: Representation) -> Self {
        self.representation = representation;
        self
    }

    /// Returns what happens to a required field the source cannot fill.
    pub const fn nullability(self) -> Nullability {
        self.nullability
    }

    /// Returns what a same-width pair carries.
    pub const fn representation(self) -> Representation {
        self.representation
    }

    /// Returns this policy with absence repaired rather than refused.
    ///
    /// A materializing protocol fills its own column after the cast, so the
    /// cast may not refuse the hole the protocol is about to close.
    pub(crate) const fn deferred(self) -> Self {
        self.with_nullability(Nullability::Default)
    }
}

impl Default for ArrowCastOptions {
    fn default() -> Self {
        Self::new()
    }
}
