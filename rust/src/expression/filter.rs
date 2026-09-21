//! The `where` block: a term that answers a boolean, and what it keeps.
//!
//! A [`Filter`] is a [`Term`] with one promise added: bound against a schema
//! it answers `true`, `false` or unknown, and a row is kept exactly when the
//! answer is `true`. The promise is checked where the schema is known, since
//! [`bind`](Filter::bind) refuses a term that types to anything but a boolean,
//! and never at construction, because `price` is a perfectly good filter over
//! a schema that declares it boolean and a nonsense one everywhere else.
//!
//! Everything a filter can do it does through the one tree: the same term
//! prunes a listing through its holder attributes, prunes a container through
//! its statistics, and filters a batch through the vectorized tier. The
//! `(column, value)` pairs record options take are sugar that builds one; there
//! is no second implementation behind them.

use std::str::FromStr;

use super::attribute::Attribute;
use super::term::Term;
use crate::{DataType, Error, Field, Result};

/// A term that answers a boolean: which rows to keep.
///
/// ```
/// use yggdryl::expression::Filter;
///
/// # fn main() -> yggdryl::Result<()> {
/// let filter: Filter = "ccy = 'EUR' and price > 100".parse()?;
/// assert_eq!(filter.to_string(), "ccy = 'EUR' and price > 100");
/// assert_eq!(filter.columns(), vec!["ccy".to_owned(), "price".to_owned()]);
/// # Ok(())
/// # }
/// ```
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize, ::serde::Deserialize,
)]
#[serde(transparent)]
pub struct Filter(Term);

impl Filter {
    /// Keep the rows a term answers `true` for.
    #[must_use]
    pub const fn new(term: Term) -> Self {
        Self(term)
    }

    /// The filter that keeps every row.
    #[must_use]
    pub fn always_true() -> Self {
        Self(Term::always_true())
    }

    /// The filter that keeps no row.
    #[must_use]
    pub fn always_false() -> Self {
        Self(Term::always_false())
    }

    /// Borrow the term this filter answers.
    #[must_use]
    pub const fn term(&self) -> &Term {
        &self.0
    }

    /// Consume this filter and answer its term.
    #[must_use]
    pub fn into_term(self) -> Term {
        self.0
    }

    /// Return a deterministic hash of the canonical filter text.
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_display(self)
    }

    /// Conjoin every filter, flattening nested conjunctions.
    #[must_use]
    pub fn all(operands: impl IntoIterator<Item = Self>) -> Self {
        Self(Term::all(operands.into_iter().map(Self::into_term)))
    }

    /// Disjoin every filter, flattening nested disjunctions.
    #[must_use]
    pub fn any(operands: impl IntoIterator<Item = Self>) -> Self {
        Self(Term::any(operands.into_iter().map(Self::into_term)))
    }

    /// Build `self and other`.
    #[must_use]
    pub fn and(self, other: Self) -> Self {
        Self(self.0.and(other.0))
    }

    /// Build `self or other`.
    #[must_use]
    pub fn or(self, other: Self) -> Self {
        Self(self.0.or(other.0))
    }

    /// Build `not self`, folding a double negation.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> Self {
        Self(self.0.not())
    }

    /// Return whether this filter keeps every row by construction.
    #[must_use]
    pub fn is_always_true(&self) -> bool {
        self.0.is_always_true()
    }

    /// Return whether this filter keeps no row by construction.
    #[must_use]
    pub fn is_always_false(&self) -> bool {
        self.0.is_always_false()
    }

    /// The top-level `and` operands, flattened, each a filter of its own.
    ///
    /// Pushdown is per conjunct and a residual is the conjuncts a layer did not
    /// settle, so this is the shape every layer of a read consumes.
    #[must_use]
    pub fn conjuncts(&self) -> Vec<Self> {
        self.0.conjuncts().into_iter().map(Self).collect()
    }

    /// Every top-level column this filter reads, in first-seen order.
    #[must_use]
    pub fn columns(&self) -> Vec<String> {
        self.0.columns()
    }

    /// Every handle attribute this filter reads, in first-seen order.
    #[must_use]
    pub fn attributes(&self) -> Vec<Attribute> {
        self.0.attributes()
    }

    /// Every parameter this filter names, in first-seen order.
    #[must_use]
    pub fn parameters(&self) -> Vec<String> {
        self.0.parameters()
    }

    /// Return whether this filter reads any handle attribute.
    #[must_use]
    pub fn has_attributes(&self) -> bool {
        self.0.has_attributes()
    }

    /// The same filter with fewer nodes and the same answer for every row.
    ///
    /// [`Term::simplify`] carries the rules, every one exact under
    /// three-valued logic; this is the form pushdown and binding run on.
    #[must_use]
    pub fn simplify(&self) -> Self {
        Self(self.0.simplify())
    }

    /// Refuse a filter past the depth or node budget.
    ///
    /// # Errors
    ///
    /// [`Term::check_budget`] carries the rule.
    pub fn check_budget(&self) -> Result<()> {
        self.0.check_budget()
    }

    /// The boolean field this filter produces against a struct root schema.
    ///
    /// # Errors
    ///
    /// Returns an error when the term cannot be typed against the schema, or
    /// types to anything but a boolean.
    pub fn field(&self, schema: &Field) -> Result<Field> {
        let field = self.0.field(schema)?;
        require_boolean(&field)?;
        Ok(field)
    }

    /// The schema a filter leaves untouched, once it has proven it binds.
    ///
    /// A filter changes which rows flow and never their shape, so its
    /// application to a schema is the schema - after the one check that
    /// matters, that the filter is a boolean over it.
    ///
    /// # Errors
    ///
    /// Returns the same error [`Self::field`] would.
    pub fn apply_field(&self, root: &Field) -> Result<Field> {
        self.apply_datatype(root.dtype())?;
        Ok(root.clone())
    }

    /// The struct datatype a filter leaves untouched, once it has proven it
    /// is a boolean over it.
    ///
    /// # Errors
    ///
    /// Returns the same error [`Self::field`] would.
    pub fn apply_datatype(&self, dtype: &DataType) -> Result<DataType> {
        self.field(&Field::new(
            crate::media::DEFAULT_ROOT_NAME,
            dtype.clone(),
            false,
        ))?;
        Ok(dtype.clone())
    }

    /// Read a filter from the scalar that spells one.
    ///
    /// Text parses as the clause, a boolean is the constant it names, null
    /// keeps every row, and a sequence is the conjunction of the filters its
    /// items spell - one item per condition, all of them required.
    ///
    /// # Errors
    ///
    /// Returns a parse error for text that is not a predicate, and an error
    /// naming the scalar for any other shape.
    pub fn from_scalar(value: &crate::Scalar) -> Result<Self> {
        if let Some(text) = value.as_str() {
            return text.parse();
        }
        if value.is_null() {
            return Ok(Self::always_true());
        }
        if let crate::Scalar::Boolean(kept) = value {
            return Ok(if kept.get() {
                Self::always_true()
            } else {
                Self::always_false()
            });
        }
        if let Some(items) = value.sequence_rows() {
            return items?.iter().try_fold(Self::always_true(), |held, item| {
                Ok(held.and(Self::from_scalar(item)?))
            });
        }
        Err(Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$"),
            reason: crate::text::expected_got(
                "the text of a where clause, a boolean, a sequence of conditions, or null",
                format_args!("{value:?}"),
            ),
        })
    }

    /// Answer this filter for one row, reading unknown as "no".
    ///
    /// The one-row spelling of [`Bound::matches`](super::Bound::matches),
    /// bound here and answered at once; a caller with many rows binds once.
    ///
    /// # Errors
    ///
    /// Returns an error when the filter does not bind against `root`, or the
    /// row does not match it.
    pub fn apply_scalar(&self, root: &Field, row: &crate::Scalar) -> Result<bool> {
        self.bind(root)?.matches(row)
    }
}

/// Refuse an output field that is not a boolean.
pub(crate) fn require_boolean(field: &Field) -> Result<()> {
    if matches!(field.dtype(), DataType::Boolean | DataType::Null) {
        return Ok(());
    }
    Err(Error::InvalidRecord {
        path: smol_str::SmolStr::new_static("$"),
        reason: smol_str::format_smolstr!(
            "expected a boolean `where` clause, got {}",
            field.dtype()
        ),
    })
}

impl From<Term> for Filter {
    fn from(term: Term) -> Self {
        Self(term)
    }
}

impl From<Filter> for Term {
    fn from(filter: Filter) -> Self {
        filter.0
    }
}

impl FromStr for Filter {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        super::parser::parse_filter(input)
    }
}

impl TryFrom<&str> for Filter {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        value.parse()
    }
}

/// Anything a call site may hand over where a filter is wanted.
///
/// Text *parses* here. That is the whole point of the trait: an
/// `impl Into<Filter>` for `&str` would quietly make `"ccy = 'EUR'"` a string
/// literal, a perfectly valid term that filters nothing and reports no error,
/// and a filter that silently matches everything is the worst failure this
/// module could have. Parsing is fallible, so the conversion is fallible, and
/// a typo arrives as a byte-positioned parse error at the call.
pub trait IntoFilter {
    /// Produce the filter this value stands for.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the value is text that is not a filter, or
    /// an error naming the clause when it is an expression of another kind.
    fn into_filter(self) -> Result<Filter>;
}

impl IntoFilter for Filter {
    fn into_filter(self) -> Result<Self> {
        Ok(self)
    }
}

impl IntoFilter for &Filter {
    fn into_filter(self) -> Result<Filter> {
        Ok(self.clone())
    }
}

impl IntoFilter for Term {
    fn into_filter(self) -> Result<Filter> {
        Ok(Filter(self))
    }
}

impl IntoFilter for &Term {
    fn into_filter(self) -> Result<Filter> {
        Ok(Filter(self.clone()))
    }
}

impl IntoFilter for &str {
    fn into_filter(self) -> Result<Filter> {
        self.parse()
    }
}

impl IntoFilter for &String {
    fn into_filter(self) -> Result<Filter> {
        self.parse()
    }
}

impl IntoFilter for String {
    fn into_filter(self) -> Result<Filter> {
        self.parse()
    }
}

impl IntoFilter for super::Expression {
    fn into_filter(self) -> Result<Filter> {
        match self {
            super::Expression::Filter(filter) => Ok(filter),
            other => Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: smol_str::format_smolstr!("expected a `where` clause, got `{other}`"),
            }),
        }
    }
}

impl Default for Filter {
    fn default() -> Self {
        Self::always_true()
    }
}

impl IntoFilter for &super::Expression {
    fn into_filter(self) -> Result<Filter> {
        self.clone().into_filter()
    }
}

impl Filter {
    /// The predicate one column-equals-value pair spells about the *rows*.
    ///
    /// This is the sugar half of "one representation": the `(&str, &str)`
    /// pairs every read option and every table method already take build a
    /// filter here and are answered by the one evaluator, rather than by a
    /// second comparison written per call site.
    ///
    /// The value is text because a directory name is text, so it is read
    /// through the column's own datatype - the same reading a partitioned read
    /// gives the directory - and read *safely*: a text the type cannot hold
    /// becomes null, which makes the pair match nothing rather than fail a
    /// whole scan. The literal folds at [`bind`](Self::bind), so the cast is
    /// paid once and never per row.
    ///
    /// The text `null` names the absence of a value, exactly as a partition
    /// directory spells it, so the pair `("price", "null")` becomes
    /// `price is null` rather than a comparison against four letters.
    #[must_use]
    pub fn partition_equals(column: &str, value: &str, dtype: &DataType) -> Self {
        let reference = Term::column(column);
        if value == crate::media::NULL_PARTITION {
            return Self(reference.is_null());
        }
        Self(reference.eq(Term::literal(value).try_cast(dtype.clone())))
    }

    /// The predicate one column-equals-value pair spells about the *holder*.
    ///
    /// The same pair, asked of the path rather than of the rows. A listing can
    /// answer this one without opening anything, which is why the two
    /// spellings are kept apart instead of one guessing which was meant.
    #[must_use]
    pub fn holder_partition_equals(column: &str, value: &str) -> Self {
        Self(Term::attribute(Attribute::Partition(column.into())).eq(Term::literal(value)))
    }

    /// Conjoin every pair as a predicate about the rows of a schema.
    ///
    /// A pair naming a column the schema does not declare is left out rather
    /// than refused: on a partitioned read the leaf's own path already
    /// answered for it, and a filter that has been answered elsewhere is not
    /// an error.
    #[must_use]
    pub fn all_partitions_equal<C: AsRef<str>, V: AsRef<str>>(
        schema: &Field,
        pairs: impl IntoIterator<Item = (C, V)>,
    ) -> Self {
        Self::all(pairs.into_iter().filter_map(|(column, value)| {
            schema
                .dtype()
                .get_field_by_name(column.as_ref())
                .map(|field| Self::partition_equals(column.as_ref(), value.as_ref(), field.dtype()))
        }))
    }

    /// Conjoin every pair as a predicate about the holder.
    #[must_use]
    pub fn all_holder_partitions_equal<C: AsRef<str>, V: AsRef<str>>(
        pairs: impl IntoIterator<Item = (C, V)>,
    ) -> Self {
        Self::all(
            pairs.into_iter().map(|(column, value)| {
                Self::holder_partition_equals(column.as_ref(), value.as_ref())
            }),
        )
    }

    /// The predicate that one holder's path *carries* a partition value.
    ///
    /// The difference from [`Self::holder_partition_equals`] is what happens
    /// when the path does not spell the column at all, and it is the whole
    /// difference between pruning and selecting. Pruning must keep what it
    /// cannot rule out, so a missing partition leaves the equality unknown and
    /// the file is read anyway. Selecting must return only what it can point
    /// at, so a missing partition has to be a `false`. The two spellings are
    /// kept apart rather than one of them guessing which was meant.
    #[must_use]
    pub fn holder_carries_partition(column: &str, value: &str) -> Self {
        let attribute = Term::attribute(Attribute::Partition(column.into()));
        Self(
            attribute
                .clone()
                .is_not_null()
                .and(attribute.eq(Term::literal(value))),
        )
    }

    /// Conjoin every pair as a predicate that the holder carries it.
    #[must_use]
    pub fn all_holder_partitions_carried<C: AsRef<str>, V: AsRef<str>>(
        pairs: impl IntoIterator<Item = (C, V)>,
    ) -> Self {
        Self::all(
            pairs.into_iter().map(|(column, value)| {
                Self::holder_carries_partition(column.as_ref(), value.as_ref())
            }),
        )
    }
}
