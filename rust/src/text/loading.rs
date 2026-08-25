//! [`Loading`], the options value every structured-text read takes.
//!
//! The read-side counterpart to [`Formatting`](crate::text::Formatting): one
//! value carrying what a load may do beyond parsing, so a new capability
//! becomes a field rather than a `_with_<thing>` variant of every entry point.

use crate::Field;
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
/// use yggdryl::Scalar;
///
/// # fn main() -> yggdryl::Result<()> {
/// // Placeholders left off: the document is parsed and nothing is walked.
/// let plain = yggdryl::text::from_utf8_with("a: \"{{ X }}\"\n", Format::Yaml, &Loading::new())?;
/// assert_eq!(plain.get_key_str("a").and_then(Scalar::as_utf8), Some("{{ X }}"));
///
/// // Turned on, and resolving entirely from the supplied mapping.
/// let loading = Loading::new()
///     .with_placeholders(Placeholders::new().with_variable("X", Scalar::from("resolved")));
/// let filled = yggdryl::text::from_utf8_with("a: \"{{ X }}\"\n", Format::Yaml, &loading)?;
/// assert_eq!(filled.get_key_str("a").and_then(Scalar::as_utf8), Some("resolved"));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Loading {
    /// The resource limits applied while decoding.
    limits: Limits,
    /// `None` - the default - means no substitution pass runs at all.
    placeholders: Option<Placeholders>,
    /// Optional field used to recover exact types from natural text values.
    field: Option<Field>,
}

impl Loading {
    /// The plain load: default limits, no substitution.
    #[must_use]
    pub fn new() -> Self {
        Self {
            limits: Limits::default(),
            placeholders: None,
            field: None,
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

    /// Interpret the parsed natural value under `field`.
    #[must_use]
    pub fn with_field(mut self, field: Field) -> Self {
        self.field = Some(field);
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

    /// The field used for typed parsing, when declared.
    #[must_use]
    pub const fn field(&self) -> Option<&Field> {
        self.field.as_ref()
    }
}

impl From<Limits> for Loading {
    fn from(limits: Limits) -> Self {
        Self::new().with_limits(limits)
    }
}

#[cfg(test)]
mod tests {
    use std::hash::Hash;

    use super::Loading;
    use crate::text::{Format, Placeholders};
    use crate::{DataType, Field, Scalar};

    fn amount_field() -> Field {
        Field::new(
            "amount",
            DataType::decimal128(8, 2).expect("valid decimal"),
            false,
        )
    }

    #[test]
    fn field_recovers_an_exact_natural_value() {
        let loading = Loading::new().with_field(amount_field());
        let value =
            crate::text::from_utf8_with("\"12.50\"", Format::Json, &loading).expect("typed JSON");
        assert_eq!(value, Scalar::d128(1_250, 2));
        assert_eq!(loading.field().map(Field::name), Some("amount"));

        let (_, inferred_utf8) =
            crate::text::from_utf8_inferred_with_field("\"12.50\"", &amount_field())
                .expect("inferred typed UTF-8");
        let (_, inferred_bytes) =
            crate::text::from_bytes_inferred_with_field(b"\"12.50\"", &amount_field())
                .expect("inferred typed bytes");
        assert_eq!(inferred_utf8, value);
        assert_eq!(inferred_bytes, value);
    }

    #[test]
    fn placeholders_are_resolved_before_field_interpretation() {
        let loading = Loading::new()
            .with_placeholders(Placeholders::new().with_variable("AMOUNT", Scalar::from("12.50")))
            .with_field(amount_field());
        let value = crate::text::from_utf8_with("\"{{ AMOUNT }}\"\n", Format::Yaml, &loading)
            .expect("filled typed YAML");
        assert_eq!(value, Scalar::d128(1_250, 2));
    }

    #[test]
    fn loading_and_placeholders_have_map_semantic_value_traits() {
        fn assert_traits<T: Clone + Eq + Ord + Hash>() {}
        assert_traits::<Placeholders>();
        assert_traits::<Loading>();

        let first = Placeholders::new()
            .with_variable("A", Scalar::I8(1))
            .with_variable("B", Scalar::I8(2));
        let equal = Placeholders::new()
            .with_variable("B", Scalar::I8(2))
            .with_variable("A", Scalar::I8(1));
        assert_eq!(first, equal);
        assert_eq!(crate::stable_hash_of(&first), crate::stable_hash_of(&equal));

        let first = Loading::new().with_placeholders(first);
        let equal = Loading::new().with_placeholders(equal);
        assert_eq!(first, equal);
        assert_eq!(crate::stable_hash_of(&first), crate::stable_hash_of(&equal));
    }
}
