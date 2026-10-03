//! The keys two sides are joined on: one pair of terms per key, the left
//! side's and the right side's, and the one intake every join verb reads
//! its `by` through.
//!
//! A key is spelled as DuckDB spells it: a bare term is a `using` key - the
//! same term over both sides, `id` - and an equality names a left term and
//! a right term, `venue = market`, `lower(venue) = mic`. Every spelling of a
//! list resolves once into a [`JoinKeys`]; the sides are bound against
//! their own roots where the join runs, never here.

use std::fmt;
use std::str::FromStr;

use smol_str::SmolStr;

use super::selector::Selector;
use super::term::Term;
use super::{Comparison, Error, Result};
use crate::Scalar;

/// One key of a join: the term each side computes, equal rows matching.
///
/// ```
/// use yggdryl::expression::JoinKey;
///
/// # fn main() -> yggdryl::Result<()> {
/// let using: JoinKey = "id".parse()?;
/// assert!(using.is_using());
/// assert_eq!(using.to_string(), "id");
/// let pair: JoinKey = "lower(venue) = mic".parse()?;
/// assert!(!pair.is_using());
/// assert_eq!(pair.left().to_string(), "lower(venue)");
/// assert_eq!(pair.right().to_string(), "mic");
/// # Ok(())
/// # }
/// ```
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize, ::serde::Deserialize,
)]
pub struct JoinKey {
    left: Term,
    right: Term,
}

impl JoinKey {
    /// A left term against a right term.
    #[must_use]
    pub const fn new(left: Term, right: Term) -> Self {
        Self { left, right }
    }

    /// One term over both sides: DuckDB's `using`.
    #[must_use]
    pub fn using(term: Term) -> Self {
        Self {
            right: term.clone(),
            left: term,
        }
    }

    /// The left side's term.
    #[must_use]
    pub const fn left(&self) -> &Term {
        &self.left
    }

    /// The right side's term.
    #[must_use]
    pub const fn right(&self) -> &Term {
        &self.right
    }

    /// Whether both sides compute one term: what a join coalesces into one
    /// output column when it is a bare column.
    #[must_use]
    pub fn is_using(&self) -> bool {
        self.left == self.right
    }

    /// The column both sides name bare, when the key is one.
    #[must_use]
    pub fn using_column(&self) -> Option<&str> {
        if !self.is_using() {
            return None;
        }
        self.left.as_column()
    }

    /// Read one key from the scalar that spells it: text in the key grammar,
    /// or a two-item list of the left and right terms, each as
    /// [`Term::from_scalar`] reads them.
    ///
    /// # Errors
    ///
    /// Returns a parse error for text that is not a key, and an error naming
    /// the shape for anything else.
    pub fn from_scalar(value: &Scalar) -> Result<Self> {
        if let Some(text) = value.as_str() {
            return text.parse();
        }
        if let Some([left, right]) = value.sequence_rows().as_deref() {
            return Ok(Self::new(
                Term::from_scalar(left)?,
                Term::from_scalar(right)?,
            ));
        }
        Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(
                "the text of a join key (`id`, `venue = market`) or a [left, right] pair of terms",
                format_args!("{value:?}"),
            ),
        })
    }

    /// Read one key out of a parsed term: an equality is a pair, anything
    /// else is the same term over both sides.
    pub(crate) fn from_term(term: Term) -> Result<Self> {
        match term {
            Term::Compare(left, Comparison::Eq, right) => Ok(Self::new(*left, *right)),
            Term::Compare(_, comparison, _) => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: smol_str::format_smolstr!(
                    "a join key is an equality or one term over both sides, got `{comparison}`"
                ),
            }),
            same => Ok(Self::using(same)),
        }
    }
}

impl JoinKey {
    /// The key as the equality it reads as: what the plan's `on` clause
    /// writes, every side parenthesized where the grammar needs it.
    pub(crate) fn equality(&self) -> Term {
        Term::Compare(
            Box::new(self.left.clone()),
            Comparison::Eq,
            Box::new(self.right.clone()),
        )
    }
}

impl fmt::Display for JoinKey {
    /// The bare term for a key over both sides, else the equality; a shared
    /// term that is itself an equality is written as the pair, since bare it
    /// would read back as two terms.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_using() && !matches!(self.left, Term::Compare(_, Comparison::Eq, _)) {
            write!(formatter, "{}", self.left)
        } else {
            write!(formatter, "{}", self.equality())
        }
    }
}

impl FromStr for JoinKey {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        let mut keys = super::parser::parse_join_keys(input)?;
        match keys.len() {
            1 => Ok(keys.0.swap_remove(0)),
            count => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: smol_str::format_smolstr!("expected one join key, got {count}"),
            }),
        }
    }
}

/// The keys of one join, most significant first: what every join verb
/// resolves its `by` into, and what the plan's `join ... on` clause holds.
///
/// ```
/// use yggdryl::expression::{IntoJoinKeys, JoinKeys};
///
/// # fn main() -> yggdryl::Result<()> {
/// let keys: JoinKeys = "id, venue = market".parse()?;
/// assert_eq!(keys.len(), 2);
/// assert_eq!(keys.to_string(), "id, venue = market");
/// assert_eq!(["id", "venue = market"].into_join_keys()?, keys);
/// # Ok(())
/// # }
/// ```
#[derive(
    Clone,
    Debug,
    Default,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    ::serde::Serialize,
    ::serde::Deserialize,
)]
#[serde(transparent)]
pub struct JoinKeys(pub(crate) Vec<JoinKey>);

impl JoinKeys {
    /// The keys, in order.
    #[must_use]
    pub fn new(keys: impl IntoIterator<Item = JoinKey>) -> Self {
        Self(keys.into_iter().collect())
    }

    /// Every key, most significant first.
    #[must_use]
    pub fn keys(&self) -> &[JoinKey] {
        &self.0
    }

    /// How many keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no key was stated: what a join refuses.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The keys, lent in order.
    pub fn iter(&self) -> std::slice::Iter<'_, JoinKey> {
        self.0.iter()
    }

    /// Read the keys from the scalar that spells them: text in the key
    /// grammar, a list of key texts or `[left, right]` pairs, or a mapping
    /// from each left term to its right term - each as
    /// [`JoinKey::from_scalar`] and [`Term::from_scalar`] read them.
    ///
    /// # Errors
    ///
    /// Returns a parse error for text that is not a key list, and an error
    /// naming the shape for anything else.
    pub fn from_scalar(value: &Scalar) -> Result<Self> {
        if let Some(text) = value.as_str() {
            return text.parse();
        }
        if let Some(entries) = value.as_mapping() {
            return entries
                .iter()
                .map(|(left, right)| {
                    Ok(JoinKey::new(
                        Term::from_scalar(left)?,
                        Term::from_scalar(right)?,
                    ))
                })
                .collect::<Result<Vec<_>>>()
                .map(Self);
        }
        if let Some(entries) = value.as_struct() {
            return entries
                .iter()
                .map(|(left, right)| Ok(JoinKey::new(left.parse()?, Term::from_scalar(right)?)))
                .collect::<Result<Vec<_>>>()
                .map(Self);
        }
        if let Some(items) = value.sequence_rows() {
            return items
                .iter()
                .map(JoinKey::from_scalar)
                .collect::<Result<Vec<_>>>()
                .map(Self);
        }
        Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(
                "the text of a join key list, a list of keys, or a mapping of left terms to right terms",
                format_args!("{value:?}"),
            ),
        })
    }
}

impl fmt::Display for JoinKeys {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, key) in self.0.iter().enumerate() {
            if index != 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{key}")?;
        }
        Ok(())
    }
}

impl FromStr for JoinKeys {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        super::parser::parse_join_keys(input)
    }
}

impl IntoIterator for JoinKeys {
    type Item = JoinKey;
    type IntoIter = std::vec::IntoIter<JoinKey>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a JoinKeys {
    type Item = &'a JoinKey;
    type IntoIter = std::slice::Iter<'a, JoinKey>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// Any value that stands for the keys of a join: what every join verb takes
/// as its `by`.
///
/// The spellings, each resolved once: the text of a key list (`"id, venue =
/// market"` - a bare term is the same term over both sides, an equality a
/// left and a right term); one [`JoinKey`], a vector, a slice or an array of
/// them; a list of key texts; a list of `(left, right)` term pairs; a pair of
/// [`Selector`]s matched projection by projection; and a
/// [`Scalar`] - a text, a list of texts or pairs, or a mapping of left terms
/// to right terms - which is how a binding's keys cross
/// ([`JoinKeys::from_scalar`]).
pub trait IntoJoinKeys {
    /// Produce the keys this value stands for.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the value is text that is not a key list,
    /// and an error naming the shape - two selectors of different lengths
    /// among them - for anything else.
    fn into_join_keys(self) -> Result<JoinKeys>;
}

impl IntoJoinKeys for JoinKeys {
    fn into_join_keys(self) -> Result<JoinKeys> {
        Ok(self)
    }
}

impl IntoJoinKeys for &JoinKeys {
    fn into_join_keys(self) -> Result<JoinKeys> {
        Ok(self.clone())
    }
}

impl IntoJoinKeys for JoinKey {
    fn into_join_keys(self) -> Result<JoinKeys> {
        Ok(JoinKeys(vec![self]))
    }
}

impl IntoJoinKeys for Vec<JoinKey> {
    fn into_join_keys(self) -> Result<JoinKeys> {
        Ok(JoinKeys(self))
    }
}

impl IntoJoinKeys for &[JoinKey] {
    fn into_join_keys(self) -> Result<JoinKeys> {
        Ok(JoinKeys(self.to_vec()))
    }
}

impl<const N: usize> IntoJoinKeys for [JoinKey; N] {
    fn into_join_keys(self) -> Result<JoinKeys> {
        Ok(JoinKeys(self.into()))
    }
}

impl IntoJoinKeys for &str {
    fn into_join_keys(self) -> Result<JoinKeys> {
        self.parse()
    }
}

impl IntoJoinKeys for String {
    fn into_join_keys(self) -> Result<JoinKeys> {
        self.parse()
    }
}

impl IntoJoinKeys for &String {
    fn into_join_keys(self) -> Result<JoinKeys> {
        self.parse()
    }
}

/// One key per text, each read through the key grammar.
fn keys_of<'a>(texts: impl IntoIterator<Item = &'a str>) -> Result<JoinKeys> {
    texts
        .into_iter()
        .map(str::parse)
        .collect::<Result<Vec<_>>>()
        .map(JoinKeys)
}

impl IntoJoinKeys for Vec<&str> {
    fn into_join_keys(self) -> Result<JoinKeys> {
        keys_of(self)
    }
}

impl IntoJoinKeys for &[&str] {
    fn into_join_keys(self) -> Result<JoinKeys> {
        keys_of(self.iter().copied())
    }
}

impl<const N: usize> IntoJoinKeys for [&str; N] {
    fn into_join_keys(self) -> Result<JoinKeys> {
        keys_of(self)
    }
}

impl IntoJoinKeys for Vec<String> {
    fn into_join_keys(self) -> Result<JoinKeys> {
        keys_of(self.iter().map(String::as_str))
    }
}

impl<const N: usize> IntoJoinKeys for [String; N] {
    fn into_join_keys(self) -> Result<JoinKeys> {
        keys_of(self.iter().map(String::as_str))
    }
}

impl IntoJoinKeys for Vec<(Term, Term)> {
    fn into_join_keys(self) -> Result<JoinKeys> {
        Ok(JoinKeys(
            self.into_iter()
                .map(|(left, right)| JoinKey::new(left, right))
                .collect(),
        ))
    }
}

impl<const N: usize> IntoJoinKeys for [(Term, Term); N] {
    fn into_join_keys(self) -> Result<JoinKeys> {
        Ok(JoinKeys(
            self.into_iter()
                .map(|(left, right)| JoinKey::new(left, right))
                .collect(),
        ))
    }
}

impl IntoJoinKeys for (Selector, Selector) {
    /// The left selector's projections against the right's, position by
    /// position; an `unnest` on either side is refused, because a key is
    /// one value per row.
    fn into_join_keys(self) -> Result<JoinKeys> {
        (&self.0, &self.1).into_join_keys()
    }
}

impl IntoJoinKeys for (&Selector, &Selector) {
    fn into_join_keys(self) -> Result<JoinKeys> {
        let (left, right) = self;
        left.refuse_unnest("in a join key")?;
        right.refuse_unnest("in a join key")?;
        if left.projections().len() != right.projections().len() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: smol_str::format_smolstr!(
                    "expected as many left keys as right keys, got {} and {}",
                    left.projections().len(),
                    right.projections().len()
                ),
            });
        }
        Ok(JoinKeys(
            left.projections()
                .iter()
                .zip(right.projections())
                .map(|(left, right)| JoinKey::new(left.term().clone(), right.term().clone()))
                .collect(),
        ))
    }
}

impl IntoJoinKeys for &Scalar {
    fn into_join_keys(self) -> Result<JoinKeys> {
        JoinKeys::from_scalar(self)
    }
}

impl IntoJoinKeys for Scalar {
    fn into_join_keys(self) -> Result<JoinKeys> {
        JoinKeys::from_scalar(&self)
    }
}
