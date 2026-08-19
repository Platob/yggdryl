//! [`Loading`], the options value every structured-text read takes.
//!
//! The read-side counterpart to [`Formatting`](crate::text::Formatting): one
//! value carrying what a load may do beyond parsing, so a new capability
//! becomes a field rather than a `_with_<thing>` variant of every entry point.

use crate::text::{Limits, Placeholders};

/// What a structured-text load may do beyond parsing.
///
/// Defaults to exactly what the plain loaders do: the default [`Limits`] and no
/// substitution at all. Both additions are opt-in, and
/// [`Placeholders`](crate::text::Placeholders) documents why the environment is
/// a second switch on top of the first.
///
/// ```
/// use yggdryl::text::{Format, Loading, Placeholders};
/// use yggdryl::Value;
///
/// # fn main() -> yggdryl::Result<()> {
/// // Placeholders left off: the document is parsed and nothing is walked.
/// let plain = yggdryl::text::from_str_with("a: \"{{ X }}\"\n", Format::Yaml, &Loading::new())?;
/// assert_eq!(plain.get_key_str("a").and_then(Value::as_str), Some("{{ X }}"));
///
/// // Turned on, and resolving entirely from the supplied mapping.
/// let loading = Loading::new()
///     .with_placeholders(Placeholders::new().with_variable("X", Value::from("resolved")));
/// let filled = yggdryl::text::from_str_with("a: \"{{ X }}\"\n", Format::Yaml, &loading)?;
/// assert_eq!(filled.get_key_str("a").and_then(Value::as_str), Some("resolved"));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Loading {
    /// The resource limits applied while decoding.
    limits: Limits,
    /// `None` - the default - means no substitution pass runs at all.
    placeholders: Option<Placeholders>,
}

impl Loading {
    /// The plain load: default limits, no substitution.
    #[must_use]
    pub fn new() -> Self {
        Self {
            limits: Limits::default(),
            placeholders: None,
        }
    }

    /// Apply explicit resource limits.
    #[must_use]
    pub const fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Resolve `{{ }}` placeholders from `placeholders` after parsing.
    #[must_use]
    pub fn with_placeholders(mut self, placeholders: Placeholders) -> Self {
        self.placeholders = Some(placeholders);
        self
    }

    /// The resource limits this load applies.
    #[must_use]
    pub const fn limits(&self) -> Limits {
        self.limits
    }

    /// Where placeholders resolve from, or `None` when substitution is off.
    #[must_use]
    pub const fn placeholders(&self) -> Option<&Placeholders> {
        self.placeholders.as_ref()
    }
}

impl From<Limits> for Loading {
    fn from(limits: Limits) -> Self {
        Self::new().with_limits(limits)
    }
}
