//! One recursive, typed tree: the value a projection computes and the operand
//! a filter compares.
//!
//! A [`Term`] is a *plan over data*. Its meaning depends on a schema: a
//! [`Field`] is decided for it once, against a struct root, and everything
//! else - how a literal is converted, how two operands are compared, what
//! array a batch produces - follows from that answer. It is the one tree
//! behind both clauses of an [`Expression`](super::Expression): a
//! [`Selector`](super::Selector) publishes terms as columns, and a
//! [`Filter`](super::Filter) is a term that answers a boolean.
//!
//! # A term is not a value
//!
//! [`Scalar`](crate::Scalar) is the codec's lossless value tree: structural,
//! serializable, and meaningful on its own. A `Term` is a computation whose
//! meaning depends on a schema. They meet at exactly two points -
//! [`Term::Literal`] going in, and evaluation producing a `Scalar` coming out -
//! and keeping them apart is what lets `Scalar` stay plain structural data
//! while a term carries schema-dependent meaning.
//!
//! # Nesting is shared
//!
//! Children are held through [`Arc`], so cloning a large predicate bumps
//! reference counts rather than copying a tree, and an operand list with no
//! elements carries no allocation.

use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::attribute::Attribute;
use super::literal::Literal;
use super::path::FieldSegment;
use super::{Comparison, Function, NODE_LIMIT, Operator, RECURSION_LIMIT, Safety};
use crate::{DataType, Error, Result, Scalar};

/// One recursive, typed value tree.
///
/// The variants are grouped by what kind of node they are, and each group is
/// documented as a group rather than variant by variant, because what matters
/// about `Or` is what matters about `And`.
///
/// ```
/// use yggdryl::expression::Term;
///
/// # fn main() -> yggdryl::Result<()> {
/// let term: Term = "price * size".parse()?;
/// assert_eq!(term.to_string(), "price * size");
/// assert_eq!(term.columns(), vec!["price".to_owned(), "size".to_owned()]);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Hash, ::serde::Serialize, ::serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Term {
    // ---- Leaves: nodes with no term children ------------------------------
    /// A constant, carrying the datatype it belongs to.
    ///
    /// A literal is a [`Literal`] and never a bare Rust primitive, so
    /// `decimal '1.50'` stays an exact decimal at scale two all the way to the
    /// comparison rather than becoming an integer that happens to print alike.
    Literal(Literal),
    /// A path from the row root: the column it starts at and the steps that
    /// reach inside it, so `trade.legs[0]['ccy']` is one leaf resolved once.
    ///
    /// The first step names a top-level column. A computed container is
    /// reached with [`Function::Get`] and [`Function::Slice`] instead, so a
    /// path is always something a reader can push down.
    Path(Arc<[FieldSegment]>),
    /// An attribute of the *handle* rather than of the rows - `&holder.size`,
    /// `&holder.partition['year']`. See [`Attribute`] for the cost table.
    Attribute(Attribute),
    /// A late-bound value, supplied when the term is bound.
    Parameter(SmolStr),

    // ---- Logical: n-ary, flattened at construction ------------------------
    /// Conjunction. Empty is `true`.
    And(Arc<[Term]>),
    /// Disjunction. Empty is `false`.
    Or(Arc<[Term]>),
    /// Three-valued negation: `not unknown` is unknown.
    Not(Box<Term>),

    // ---- Comparison: everything that answers a boolean about values -------
    /// A comparison of two terms.
    Compare(Box<Term>, Comparison, Box<Term>),
    /// Set membership. `x in ()` is refused by the parser rather than folded,
    /// because an empty list is always a typo.
    In(Box<Term>, Arc<[Term]>),
    /// An inclusive range test, which lowers to two comparisons.
    Between(Box<Term>, Box<Term>, Box<Term>),
    /// `is null` - one of the two operators that answer true or false about a
    /// null rather than unknown.
    IsNull(Box<Term>),
    /// `is not null`.
    IsNotNull(Box<Term>),
    /// SQL `like` / `ilike`, with `_` and `%` wildcards.
    Like {
        /// The text being matched.
        value: Box<Term>,
        /// The pattern.
        pattern: Box<Term>,
        /// Whether the match ignores case.
        case_insensitive: bool,
        /// The character that escapes a wildcard, when the clause names one.
        escape: Option<char>,
    },
    /// A path-glob match with the `.gitignore` rule this crate already uses for
    /// listings: no separator matches at any depth, a separator anchors at the
    /// root. Delegates to [`Url::matches_glob`](crate::Url::matches_glob).
    Glob(Box<Term>, Box<Term>),

    // ---- Arithmetic -------------------------------------------------------
    /// Arithmetic over two operands, with the promotion rules
    /// [`Term::field`] states - a decimal never becomes a float to be added.
    Arithmetic(Box<Term>, Operator, Box<Term>),
    /// Arithmetic negation.
    Negate(Box<Term>),

    // ---- Scalar functions -------------------------------------------------
    /// A call into the closed [`Function`] set.
    Function(Function, Arc<[Term]>),

    // ---- Shape: nodes that change or build a type -------------------------
    /// A schema-directed cast, reaching the one cast this crate owns.
    Cast(Box<Term>, DataType, Safety),
    /// A searched conditional: `case when c then v ... else v end`.
    Case {
        /// The `when`/`then` pairs, tried in order.
        branches: Arc<[(Term, Term)]>,
        /// The `else` value; absent means null.
        otherwise: Option<Box<Term>>,
    },
    /// Build a struct value from named children.
    Struct(Arc<[(SmolStr, Term)]>),
    /// Build a list value from its elements.
    List(Arc<[Term]>),
    /// Build a map value from its entries.
    Map(Arc<[(Term, Term)]>),
}

/// One path with one more step, the steps copied once.
fn extended(held: &[FieldSegment], segment: FieldSegment) -> Arc<[FieldSegment]> {
    let mut steps: Vec<FieldSegment> = Vec::with_capacity(held.len() + 1);
    steps.extend(held.iter().cloned());
    steps.push(segment);
    Arc::from(steps)
}

impl Term {
    /// Return a deterministic hash of the canonical term text.
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_display(self)
    }

    /// The term that is true for every row.
    #[must_use]
    pub fn always_true() -> Self {
        Self::literal(true)
    }

    /// The term that is true for no row.
    #[must_use]
    pub fn always_false() -> Self {
        Self::literal(false)
    }

    /// Hold a constant, inferring the datatype it belongs to.
    ///
    /// # Panics
    ///
    /// Never: every [`Scalar`] this crate builds names a datatype, and one
    /// that does not is held as the null it is.
    #[must_use]
    pub fn literal(value: impl Into<Scalar>) -> Self {
        Self::Literal(Literal::infer(value.into()).unwrap_or_else(|_| Literal::null()))
    }

    /// Hold a constant under an exact datatype.
    ///
    /// # Errors
    ///
    /// Returns an error when the value and the datatype disagree.
    pub fn typed_literal(dtype: DataType, value: Scalar) -> Result<Self> {
        Ok(Self::Literal(Literal::new(dtype, value)?))
    }

    /// Read a term from the scalar that spells one.
    ///
    /// Text parses through the term grammar, so `"price"` is a column and
    /// `"'EUR'"` a literal; every other scalar is the literal it is. This is
    /// the reading every binding's inputs cross through, so a Python or
    /// JavaScript value and its text mean the same thing.
    ///
    /// # Errors
    ///
    /// Returns a parse error for text that is not a term.
    pub fn from_scalar(value: &Scalar) -> Result<Self> {
        match value.as_str() {
            Some(text) => text.parse(),
            None => Ok(Self::literal(value.clone())),
        }
    }

    /// Name a top-level column.
    #[must_use]
    pub fn column(name: impl Into<SmolStr>) -> Self {
        Self::Path(Arc::from([FieldSegment::Field(name.into())]))
    }

    /// Name a handle attribute.
    #[must_use]
    pub const fn attribute(attribute: Attribute) -> Self {
        Self::Attribute(attribute)
    }

    /// Name a late-bound value.
    #[must_use]
    pub fn parameter(name: impl Into<SmolStr>) -> Self {
        Self::Parameter(name.into())
    }

    /// Reach inside this term's value.
    ///
    /// On a path the steps extend the chain, which is what keeps `a.b.c` one
    /// leaf and makes equality between two spellings of the same path
    /// structural. On anything else each step becomes the call that reads it -
    /// [`Function::Get`] for a child, a position or a key, [`Function::Slice`]
    /// for a run - because only a column has a place to push a path down to.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] on a predicate step after a computed value: a
    /// [`FieldSegment::Where`] keeps the elements of the list a column holds,
    /// and no call in the closed function set reads a predicate, so there is
    /// no term to build. [`Self::filter_elements`] is the same refusal for one
    /// step.
    pub fn path(self, segments: impl IntoIterator<Item = FieldSegment>) -> Result<Self> {
        segments.into_iter().try_fold(self, Self::step)
    }

    /// Take one step, the way [`Self::path`] takes each of its steps.
    fn step(self, segment: FieldSegment) -> Result<Self> {
        Ok(match segment {
            FieldSegment::Field(name) => self.child(name),
            FieldSegment::Index(position) => self.at(position),
            FieldSegment::Range { start, end } => self.slice(start, end),
            FieldSegment::Key(key) => self.extend_or(FieldSegment::Key(key.clone()), |base| {
                Self::call(Function::Get, [base, Self::Literal(key)])
            }),
            FieldSegment::Where(predicate) => match self {
                Self::Path(held) => Self::Path(extended(&held, FieldSegment::Where(predicate))),
                base => {
                    return Err(Error::Parse {
                        target: "expression",
                        position: 0,
                        reason: format_smolstr!(
                            "expected a column path to keep elements of by [{predicate}], got \
                             the computed value {base}; a predicate segment reads the list a \
                             column holds"
                        ),
                    });
                }
            },
        })
    }

    /// Extend a path by one step, or read a computed value through `call`.
    ///
    /// The one place a step meets a base that is not a path: a child, a
    /// position, a key and a run each have a call that reads them off any
    /// value, so these steps never refuse.
    fn extend_or(self, segment: FieldSegment, call: impl FnOnce(Self) -> Self) -> Self {
        match self {
            Self::Path(held) => Self::Path(extended(&held, segment)),
            base => call(base),
        }
    }

    /// Reach one struct child, or one string-keyed map entry.
    #[must_use]
    pub fn child(self, name: impl Into<SmolStr>) -> Self {
        let name = name.into();
        self.extend_or(FieldSegment::Field(name.clone()), |base| {
            Self::call(
                Function::Get,
                [base, Self::literal(Scalar::from(name.as_str()))],
            )
        })
    }

    /// Reach one list element by position, 0-based.
    #[must_use]
    pub fn at(self, index: i64) -> Self {
        self.extend_or(FieldSegment::Index(index), |base| {
            Self::call(Function::Get, [base, Self::literal(index)])
        })
    }

    /// Reach a half-open run of list elements.
    #[must_use]
    pub fn slice(self, start: Option<i64>, end: Option<i64>) -> Self {
        self.extend_or(FieldSegment::Range { start, end }, |base| {
            Self::call(
                Function::Slice,
                [
                    base,
                    start.map_or_else(|| Self::literal(Scalar::Null), Self::literal),
                    end.map_or_else(|| Self::literal(Scalar::Null), Self::literal),
                ],
            )
        })
    }

    /// Keep the elements of the list this path reaches that `predicate`
    /// answers true for, the predicate reading the element's own fields.
    ///
    /// # Errors
    ///
    /// Returns an error when this term is a computed value rather than a
    /// path: a predicate segment reads the list a column holds.
    pub fn filter_elements(self, predicate: Self) -> Result<Self> {
        self.step(FieldSegment::filter(predicate))
    }

    /// Conjoin every term, flattening nested conjunctions.
    ///
    /// Flattening at construction is what makes pushdown, display, and equality
    /// stable: `a and (b and c)` and `(a and b) and c` are one value.
    #[must_use]
    pub fn all(operands: impl IntoIterator<Item = Self>) -> Self {
        let mut flat = Vec::new();
        for operand in operands {
            match operand {
                Self::And(inner) => flat.extend(inner.iter().cloned()),
                other => flat.push(other),
            }
        }
        match flat.len() {
            0 => Self::always_true(),
            1 => flat.swap_remove(0),
            _ => Self::And(Arc::from(flat)),
        }
    }

    /// Disjoin every term, flattening nested disjunctions.
    #[must_use]
    pub fn any(operands: impl IntoIterator<Item = Self>) -> Self {
        let mut flat = Vec::new();
        for operand in operands {
            match operand {
                Self::Or(inner) => flat.extend(inner.iter().cloned()),
                other => flat.push(other),
            }
        }
        match flat.len() {
            0 => Self::always_false(),
            1 => flat.swap_remove(0),
            _ => Self::Or(Arc::from(flat)),
        }
    }

    /// Build `self and other`.
    #[must_use]
    pub fn and(self, other: Self) -> Self {
        Self::all([self, other])
    }

    /// Build `self or other`.
    #[must_use]
    pub fn or(self, other: Self) -> Self {
        Self::any([self, other])
    }

    /// Build `not self`, folding a double negation.
    ///
    /// Folding is sound in Kleene logic - negation is its own inverse there,
    /// unknown included - which is why it happens at construction rather than
    /// waiting for a rewrite pass that might not run.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> Self {
        match self {
            Self::Not(inner) => *inner,
            other => Self::Not(Box::new(other)),
        }
    }

    /// Build a comparison.
    #[must_use]
    pub fn compare(self, comparison: Comparison, other: Self) -> Self {
        Self::Compare(Box::new(self), comparison, Box::new(other))
    }

    /// Build `self = other`.
    ///
    /// This shadows [`PartialEq::eq`] for an owned receiver: a predicate reads
    /// better as `col("a").eq(lit(3))` than as any spelling that avoids the
    /// collision, and structural equality stays available as `==`.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn eq(self, other: Self) -> Self {
        self.compare(Comparison::Eq, other)
    }

    /// Build `self <> other`. Shadows [`PartialEq::ne`]; see [`Self::eq`].
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn ne(self, other: Self) -> Self {
        self.compare(Comparison::NotEq, other)
    }

    /// Build `self < other`. Shadows [`PartialOrd::lt`]; see [`Self::eq`].
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn lt(self, other: Self) -> Self {
        self.compare(Comparison::Lt, other)
    }

    /// Build `self <= other`. Shadows [`PartialOrd::le`]; see [`Self::eq`].
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn le(self, other: Self) -> Self {
        self.compare(Comparison::LtEq, other)
    }

    /// Build `self > other`. Shadows [`PartialOrd::gt`]; see [`Self::eq`].
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn gt(self, other: Self) -> Self {
        self.compare(Comparison::Gt, other)
    }

    /// Build `self >= other`. Shadows [`PartialOrd::ge`]; see [`Self::eq`].
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn ge(self, other: Self) -> Self {
        self.compare(Comparison::GtEq, other)
    }

    /// Build `self in (...)`.
    #[must_use]
    pub fn is_in(self, list: impl IntoIterator<Item = Self>) -> Self {
        Self::In(Box::new(self), list.into_iter().collect())
    }

    /// Build `self between low and high`.
    #[must_use]
    pub fn between(self, low: Self, high: Self) -> Self {
        Self::Between(Box::new(self), Box::new(low), Box::new(high))
    }

    /// Build `self is null`.
    #[must_use]
    pub fn is_null(self) -> Self {
        Self::IsNull(Box::new(self))
    }

    /// Build `self is not null`.
    #[must_use]
    pub fn is_not_null(self) -> Self {
        Self::IsNotNull(Box::new(self))
    }

    /// Build `self like pattern`.
    #[must_use]
    pub fn like(self, pattern: Self) -> Self {
        Self::Like {
            value: Box::new(self),
            pattern: Box::new(pattern),
            case_insensitive: false,
            escape: None,
        }
    }

    /// Build `self ilike pattern`.
    #[must_use]
    pub fn ilike(self, pattern: Self) -> Self {
        Self::Like {
            value: Box::new(self),
            pattern: Box::new(pattern),
            case_insensitive: true,
            escape: None,
        }
    }

    /// Build `self glob pattern`.
    #[must_use]
    pub fn glob(self, pattern: Self) -> Self {
        Self::Glob(Box::new(self), Box::new(pattern))
    }

    /// Build an arithmetic node.
    #[must_use]
    pub fn arithmetic(self, operator: Operator, other: Self) -> Self {
        Self::Arithmetic(Box::new(self), operator, Box::new(other))
    }

    /// Build `self + other`.
    ///
    /// Named beside [`subtract`](Self::subtract) rather than through
    /// `std::ops::Add`, because an operator on a term would read as a value
    /// computed now and this builds one computed per row.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, other: Self) -> Self {
        self.arithmetic(Operator::Add, other)
    }

    /// Build `self - other`.
    #[must_use]
    pub fn subtract(self, other: Self) -> Self {
        self.arithmetic(Operator::Sub, other)
    }

    /// Build `self * other`.
    #[must_use]
    pub fn multiply(self, other: Self) -> Self {
        self.arithmetic(Operator::Mul, other)
    }

    /// Build `self / other`.
    #[must_use]
    pub fn divide(self, other: Self) -> Self {
        self.arithmetic(Operator::Div, other)
    }

    /// Build `self % other`.
    #[must_use]
    pub fn remainder(self, other: Self) -> Self {
        self.arithmetic(Operator::Rem, other)
    }

    /// Build `-self`, folding the negation of a number into the number.
    ///
    /// Folding here rather than in a rewrite pass is what makes `-1` one node
    /// in every spelling: written in text, built by hand, or read back from
    /// [`Display`](std::fmt::Display). A negation of anything else is its own
    /// node, because only a constant can be negated without evaluating it.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn neg(self) -> Self {
        if let Self::Literal(held) = &self {
            if let Some(negated) = negate_value(held.value()) {
                if let Ok(folded) = Literal::new(held.dtype().clone(), negated) {
                    return Self::Literal(folded);
                }
            }
        }
        Self::Negate(Box::new(self))
    }

    /// Build a call into the closed function set.
    #[must_use]
    pub fn call(function: Function, arguments: impl IntoIterator<Item = Self>) -> Self {
        Self::Function(function, arguments.into_iter().collect())
    }

    /// Convert to another datatype, refusing what the target cannot hold.
    #[must_use]
    pub fn cast(self, dtype: DataType) -> Self {
        Self::Cast(Box::new(self), dtype, Safety::Strict)
    }

    /// Convert to another datatype, nulling what the target cannot hold.
    #[must_use]
    pub fn try_cast(self, dtype: DataType) -> Self {
        Self::Cast(Box::new(self), dtype, Safety::Safe)
    }

    /// Build a searched conditional.
    #[must_use]
    pub fn case(branches: impl IntoIterator<Item = (Self, Self)>, otherwise: Option<Self>) -> Self {
        Self::Case {
            branches: branches.into_iter().collect(),
            otherwise: otherwise.map(Box::new),
        }
    }

    /// Return whether this term is a constant.
    #[must_use]
    #[inline]
    pub const fn is_literal(&self) -> bool {
        matches!(self, Self::Literal(_))
    }

    /// Borrow the constant this term holds, if it holds one.
    #[must_use]
    #[inline]
    pub const fn as_literal(&self) -> Option<&Literal> {
        match self {
            Self::Literal(value) => Some(value),
            _ => None,
        }
    }

    /// Borrow the column name this term reads, if it reads one directly.
    #[must_use]
    #[inline]
    pub fn as_column(&self) -> Option<&str> {
        match self {
            Self::Path(steps) => match steps.as_ref() {
                [FieldSegment::Field(name)] => Some(name),
                _ => None,
            },
            _ => None,
        }
    }

    /// Borrow the path this term reads, if it is one.
    #[must_use]
    #[inline]
    pub fn as_path(&self) -> Option<&[FieldSegment]> {
        match self {
            Self::Path(steps) => Some(steps),
            _ => None,
        }
    }

    /// The top-level column a path term starts at, if it is one.
    #[must_use]
    pub fn root_column(&self) -> Option<&str> {
        match self {
            Self::Path(steps) => match steps.first() {
                Some(FieldSegment::Field(name)) => Some(name),
                _ => None,
            },
            _ => None,
        }
    }

    /// Return whether this term is the constant `true`.
    #[must_use]
    pub fn is_always_true(&self) -> bool {
        match self {
            Self::Literal(held) => held.value().as_bool() == Some(true),
            Self::And(operands) => operands.is_empty(),
            _ => false,
        }
    }

    /// Return whether this term is the constant `false`.
    #[must_use]
    pub fn is_always_false(&self) -> bool {
        match self {
            Self::Literal(held) => held.value().as_bool() == Some(false),
            Self::Or(operands) => operands.is_empty(),
            _ => false,
        }
    }

    /// Return whether this term is a null constant - the answer that is
    /// unknown for every row.
    #[must_use]
    pub fn is_null_literal(&self) -> bool {
        matches!(self, Self::Literal(held) if held.is_null())
    }

    /// Visit every direct child of this node, in evaluation order.
    ///
    /// One traversal serves every walk in the module, so a variant added later
    /// is wired into all of them by editing exactly one function. A path's
    /// children are the predicates its segments carry: they nest, they name
    /// parameters and attributes, and they count against the budget, even
    /// though the columns they read are the element's and not the row's.
    pub(crate) fn for_each_child<'node>(&'node self, mut visit: impl FnMut(&'node Self)) {
        match self {
            Self::Literal(_) | Self::Attribute(_) | Self::Parameter(_) => {}
            Self::Path(steps) => steps
                .iter()
                .filter_map(FieldSegment::as_predicate)
                .for_each(visit),
            Self::And(operands) | Self::Or(operands) | Self::List(operands) => {
                operands.iter().for_each(visit);
            }
            Self::Not(inner)
            | Self::IsNull(inner)
            | Self::IsNotNull(inner)
            | Self::Negate(inner)
            | Self::Cast(inner, _, _) => visit(inner),
            Self::Compare(left, _, right) | Self::Arithmetic(left, _, right) => {
                visit(left);
                visit(right);
            }
            Self::In(value, list) => {
                visit(value);
                list.iter().for_each(visit);
            }
            Self::Between(value, low, high) => {
                visit(value);
                visit(low);
                visit(high);
            }
            Self::Like { value, pattern, .. } | Self::Glob(value, pattern) => {
                visit(value);
                visit(pattern);
            }
            Self::Function(_, arguments) => arguments.iter().for_each(visit),
            Self::Case {
                branches,
                otherwise,
            } => {
                for (when, then) in branches.iter() {
                    visit(when);
                    visit(then);
                }
                if let Some(otherwise) = otherwise {
                    visit(otherwise);
                }
            }
            Self::Struct(children) => {
                for (_, child) in children.iter() {
                    visit(child);
                }
            }
            Self::Map(entries) => {
                for (key, value) in entries.iter() {
                    visit(key);
                    visit(value);
                }
            }
        }
    }

    /// How deep this term nests, counting itself as one level.
    ///
    /// The walk is iterative, so measuring a deliberately deep tree cannot
    /// itself overflow the stack: the tree a parser built is bounded, but a
    /// caller can build one by hand and is entitled to a typed refusal.
    #[must_use]
    pub fn depth(&self) -> usize {
        let mut deepest = 0;
        let mut pending = vec![(self, 1_usize)];
        while let Some((node, depth)) = pending.pop() {
            deepest = deepest.max(depth);
            node.for_each_child(|child| pending.push((child, depth + 1)));
        }
        deepest
    }

    /// How many nodes this term holds.
    #[must_use]
    pub fn node_count(&self) -> usize {
        let mut counted = 0;
        let mut pending = vec![self];
        while let Some(node) = pending.pop() {
            counted += 1;
            if counted > NODE_LIMIT {
                return counted;
            }
            node.for_each_child(|child| pending.push(child));
        }
        counted
    }

    /// Refuse a term past the depth or node budget.
    ///
    /// Checked once, before any recursive walk, so a walk never has to.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the limit and what was reached.
    pub fn check_budget(&self) -> Result<()> {
        let depth = self.depth();
        if depth > RECURSION_LIMIT {
            return Err(Error::Parse {
                target: "expression",
                position: 0,
                reason: format_smolstr!(
                    "expected nesting within the hard limit of {RECURSION_LIMIT}, got {depth}"
                ),
            });
        }
        let nodes = self.node_count();
        if nodes > NODE_LIMIT {
            return Err(Error::Parse {
                target: "expression",
                position: 0,
                reason: format_smolstr!(
                    "expected at most {NODE_LIMIT} nodes, got at least {nodes}"
                ),
            });
        }
        Ok(())
    }

    /// Every top-level column this term reads, deduplicated in first-seen
    /// order.
    ///
    /// This is what drives projection pushdown: a read decodes exactly the
    /// columns the predicate and the projection name, and no more. A path
    /// reads the column it starts at and nothing else: the names inside its
    /// predicate segments are the element's fields, which the column already
    /// carries.
    #[must_use]
    pub fn columns(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        let mut pending: Vec<&Self> = vec![self];
        let mut children: Vec<&Self> = Vec::new();
        while let Some(node) = pending.pop() {
            if let Self::Path(_) = node {
                if let Some(name) = node.root_column() {
                    if !names.iter().any(|held| held.eq_ignore_ascii_case(name)) {
                        names.push(name.to_owned());
                    }
                }
                continue;
            }
            children.clear();
            node.for_each_child(|child| children.push(child));
            pending.extend(children.iter().rev().copied());
        }
        names
    }

    /// Every handle attribute this term reads, in first-seen order.
    #[must_use]
    pub fn attributes(&self) -> Vec<Attribute> {
        let mut found: Vec<Attribute> = Vec::new();
        self.walk(&mut |node| {
            if let Self::Attribute(attribute) = node {
                if !found.contains(attribute) {
                    found.push(attribute.clone());
                }
            }
        });
        found
    }

    /// Every parameter this term names, in first-seen order.
    #[must_use]
    pub fn parameters(&self) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        self.walk(&mut |node| {
            if let Self::Parameter(name) = node {
                if !found.iter().any(|held| held == name.as_str()) {
                    found.push(name.to_string());
                }
            }
        });
        found
    }

    /// Return whether this term reads no column, no handle attribute and no
    /// parameter: what [`columns`](Self::columns),
    /// [`has_attributes`](Self::has_attributes) and
    /// [`parameters`](Self::parameters) together answer, stopping at the
    /// first node that reads anything. A path reads the column it starts
    /// at, so one rooted at a column is never constant; one rooted
    /// elsewhere is constant exactly where its predicate segments are.
    #[must_use]
    pub(crate) fn is_constant(&self) -> bool {
        let mut pending: Vec<&Self> = Vec::with_capacity(8);
        pending.push(self);
        while let Some(node) = pending.pop() {
            match node {
                Self::Path(_) if node.root_column().is_some() => return false,
                Self::Attribute(_) | Self::Parameter(_) => return false,
                _ => node.for_each_child(|child| pending.push(child)),
            }
        }
        true
    }

    /// Return whether this term reads any handle attribute.
    ///
    /// A predicate that reads none can be answered by the rows alone; one that
    /// reads only attributes can be answered by the listing alone. Both are
    /// worth knowing before anything is opened.
    #[must_use]
    pub fn has_attributes(&self) -> bool {
        let mut found = false;
        self.walk(&mut |node| {
            if matches!(node, Self::Attribute(_)) {
                found = true;
            }
        });
        found
    }

    /// Walk every node of this term, depth-first, in evaluation order.
    ///
    /// The walk is iterative so a deliberately deep tree cannot overflow the
    /// stack, and children are pushed in reverse so popping them restores the
    /// order they are written in. Order is not cosmetic here: `columns()`
    /// promises first-seen order and a projection pushdown reads it.
    pub(crate) fn walk<'node>(&'node self, visit: &mut impl FnMut(&'node Self)) {
        let mut pending: Vec<&'node Self> = vec![self];
        let mut children: Vec<&'node Self> = Vec::new();
        while let Some(node) = pending.pop() {
            visit(node);
            children.clear();
            node.for_each_child(|child| children.push(child));
            pending.extend(children.iter().rev().copied());
        }
    }

    /// The top-level `and` operands, flattened.
    ///
    /// Pushdown is per conjunct and a residual is the conjuncts a layer did not
    /// settle, so this is the shape every layer of a read consumes.
    #[must_use]
    pub fn conjuncts(&self) -> Vec<Self> {
        match self {
            Self::And(operands) => operands.iter().flat_map(Self::conjuncts).collect(),
            other if other.is_always_true() => Vec::new(),
            other => vec![other.clone()],
        }
    }

    /// Rebuild this term with every node `replace` answers for swapped out.
    ///
    /// Written once so a rewrite never has to re-list every variant.
    pub(crate) fn map(
        &self,
        replace: &mut dyn FnMut(&Self) -> Result<Option<Self>>,
    ) -> Result<Self> {
        if let Some(replaced) = replace(self)? {
            return Ok(replaced);
        }
        Ok(match self {
            Self::Literal(_) | Self::Attribute(_) | Self::Parameter(_) => self.clone(),
            Self::Path(steps) => {
                if steps.iter().all(|step| step.as_predicate().is_none()) {
                    return Ok(self.clone());
                }
                let mut mapped = Vec::with_capacity(steps.len());
                for step in steps.iter() {
                    mapped.push(match step {
                        FieldSegment::Where(predicate) => {
                            FieldSegment::Where(Box::new(predicate.map(replace)?))
                        }
                        other => other.clone(),
                    });
                }
                Self::Path(Arc::from(mapped))
            }
            Self::And(operands) => Self::And(map_slice(operands, replace)?),
            Self::Or(operands) => Self::Or(map_slice(operands, replace)?),
            Self::Not(inner) => Self::Not(Box::new(inner.map(replace)?)),
            Self::Compare(left, comparison, right) => Self::Compare(
                Box::new(left.map(replace)?),
                *comparison,
                Box::new(right.map(replace)?),
            ),
            Self::In(value, list) => {
                Self::In(Box::new(value.map(replace)?), map_slice(list, replace)?)
            }
            Self::Between(value, low, high) => Self::Between(
                Box::new(value.map(replace)?),
                Box::new(low.map(replace)?),
                Box::new(high.map(replace)?),
            ),
            Self::IsNull(inner) => Self::IsNull(Box::new(inner.map(replace)?)),
            Self::IsNotNull(inner) => Self::IsNotNull(Box::new(inner.map(replace)?)),
            Self::Like {
                value,
                pattern,
                case_insensitive,
                escape,
            } => Self::Like {
                value: Box::new(value.map(replace)?),
                pattern: Box::new(pattern.map(replace)?),
                case_insensitive: *case_insensitive,
                escape: *escape,
            },
            Self::Glob(value, pattern) => Self::Glob(
                Box::new(value.map(replace)?),
                Box::new(pattern.map(replace)?),
            ),
            Self::Arithmetic(left, operator, right) => Self::Arithmetic(
                Box::new(left.map(replace)?),
                *operator,
                Box::new(right.map(replace)?),
            ),
            Self::Negate(inner) => Self::Negate(Box::new(inner.map(replace)?)),
            Self::Function(function, arguments) => {
                Self::Function(function.clone(), map_slice(arguments, replace)?)
            }
            Self::Cast(inner, dtype, safety) => {
                Self::Cast(Box::new(inner.map(replace)?), dtype.clone(), *safety)
            }
            Self::Case {
                branches,
                otherwise,
            } => {
                let mut mapped_branches = Vec::with_capacity(branches.len());
                for (when, then) in branches.iter() {
                    mapped_branches.push((when.map(replace)?, then.map(replace)?));
                }
                Self::Case {
                    branches: Arc::from(mapped_branches),
                    otherwise: match otherwise {
                        Some(otherwise) => Some(Box::new(otherwise.map(replace)?)),
                        None => None,
                    },
                }
            }
            Self::Struct(children) => {
                let mut mapped_children = Vec::with_capacity(children.len());
                for (name, value) in children.iter() {
                    mapped_children.push((name.clone(), value.map(replace)?));
                }
                Self::Struct(Arc::from(mapped_children))
            }
            Self::List(items) => Self::List(map_slice(items, replace)?),
            Self::Map(entries) => {
                let mut mapped_entries = Vec::with_capacity(entries.len());
                for (key, value) in entries.iter() {
                    mapped_entries.push((key.map(replace)?, value.map(replace)?));
                }
                Self::Map(Arc::from(mapped_entries))
            }
        })
    }

    /// The same term with fewer nodes and the same answer for every row.
    ///
    /// Every rewrite here is exact under three-valued logic - it produces the
    /// same `true`, `false` or unknown a row produced before - which is what
    /// lets it run before pushdown without a pruning decision ever losing a
    /// row. What it does:
    ///
    /// * flattens nested `and` / `or`, drops the operand that changes nothing
    ///   (`true` in `and`, `false` in `or`), and settles the one that decides
    ///   everything (`false` in `and`, `true` in `or`);
    /// * drops an operand repeated in the same conjunction or disjunction;
    /// * pushes `not` inward: through `and` and `or` by De Morgan, through a
    ///   comparison by [`Comparison::negated`], through a null test into its
    ///   opposite - so a negation never hides a comparison a range can prune;
    /// * orients a comparison as `term op literal`, so every rule below sees
    ///   one shape;
    /// * turns `a = 1 or a = 2` into `a in (1, 2)`, merges `in` lists on the
    ///   same term, and turns `a in (1)` back into `a = 1`;
    /// * intersects `in` lists conjoined on the same term, when the
    ///   intersection is not empty;
    /// * folds a comparison against the null literal into the unknown it
    ///   always is, and the two distinctness tests against it into the null
    ///   test they are.
    ///
    /// Nothing here evaluates: a constant subtree is folded at
    /// [`bind`](Self::bind), by the evaluator that knows the types.
    #[must_use]
    pub fn simplify(&self) -> Self {
        match self {
            Self::And(operands) => simplify_conjunction(operands),
            Self::Or(operands) => simplify_disjunction(operands),
            Self::Not(inner) => simplify_not(inner.simplify()),
            Self::Compare(left, comparison, right) => {
                simplify_compare(left.simplify(), *comparison, right.simplify())
            }
            Self::In(value, list) => {
                let value = value.simplify();
                let mut items: Vec<Self> = Vec::with_capacity(list.len());
                for item in list.iter() {
                    let item = item.simplify();
                    if !items.contains(&item) {
                        items.push(item);
                    }
                }
                membership(value, items)
            }
            Self::Between(value, low, high) => Self::Between(
                Box::new(value.simplify()),
                Box::new(low.simplify()),
                Box::new(high.simplify()),
            ),
            Self::IsNull(inner) => Self::IsNull(Box::new(inner.simplify())),
            Self::IsNotNull(inner) => Self::IsNotNull(Box::new(inner.simplify())),
            Self::Like {
                value,
                pattern,
                case_insensitive,
                escape,
            } => Self::Like {
                value: Box::new(value.simplify()),
                pattern: Box::new(pattern.simplify()),
                case_insensitive: *case_insensitive,
                escape: *escape,
            },
            Self::Glob(value, pattern) => {
                Self::Glob(Box::new(value.simplify()), Box::new(pattern.simplify()))
            }
            Self::Arithmetic(left, operator, right) => Self::Arithmetic(
                Box::new(left.simplify()),
                *operator,
                Box::new(right.simplify()),
            ),
            Self::Negate(inner) => Self::Negate(Box::new(inner.simplify())),
            Self::Function(function, arguments) => Self::Function(
                function.clone(),
                arguments.iter().map(Self::simplify).collect(),
            ),
            Self::Cast(inner, dtype, safety) => {
                Self::Cast(Box::new(inner.simplify()), dtype.clone(), *safety)
            }
            Self::Case {
                branches,
                otherwise,
            } => Self::Case {
                branches: branches
                    .iter()
                    .map(|(when, then)| (when.simplify(), then.simplify()))
                    .collect(),
                otherwise: otherwise.as_ref().map(|held| Box::new(held.simplify())),
            },
            Self::Struct(children) => Self::Struct(
                children
                    .iter()
                    .map(|(name, value)| (name.clone(), value.simplify()))
                    .collect(),
            ),
            Self::List(items) => Self::List(items.iter().map(Self::simplify).collect()),
            Self::Map(entries) => Self::Map(
                entries
                    .iter()
                    .map(|(key, value)| (key.simplify(), value.simplify()))
                    .collect(),
            ),
            Self::Path(steps) => {
                if steps.iter().all(|step| step.as_predicate().is_none()) {
                    return self.clone();
                }
                Self::Path(
                    steps
                        .iter()
                        .map(|step| match step {
                            FieldSegment::Where(predicate) => {
                                FieldSegment::Where(Box::new(predicate.simplify()))
                            }
                            other => other.clone(),
                        })
                        .collect(),
                )
            }
            Self::Literal(_) | Self::Attribute(_) | Self::Parameter(_) => self.clone(),
        }
    }
}

fn map_slice(
    operands: &[Term],
    replace: &mut dyn FnMut(&Term) -> Result<Option<Term>>,
) -> Result<Arc<[Term]>> {
    let mut mapped = Vec::with_capacity(operands.len());
    for operand in operands {
        mapped.push(operand.map(replace)?);
    }
    Ok(Arc::from(mapped))
}

/// Simplify a conjunction: flatten, absorb constants, drop repeats, and
/// intersect memberships of one term.
fn simplify_conjunction(operands: &[Term]) -> Term {
    let mut flat: Vec<Term> = Vec::with_capacity(operands.len());
    for operand in operands {
        let operand = operand.simplify();
        match operand {
            // A `false` operand settles an `and` however unknown the rest is.
            ref held if held.is_always_false() => return Term::always_false(),
            ref held if held.is_always_true() => {}
            Term::And(inner) => {
                for held in inner.iter() {
                    push_unique(&mut flat, held.clone());
                }
            }
            other => push_unique(&mut flat, other),
        }
    }
    let flat = intersect_memberships(flat);
    Term::all(flat)
}

/// Simplify a disjunction: flatten, absorb constants, drop repeats, and merge
/// equalities and memberships of one term into one list.
fn simplify_disjunction(operands: &[Term]) -> Term {
    let mut flat: Vec<Term> = Vec::with_capacity(operands.len());
    for operand in operands {
        let operand = operand.simplify();
        match operand {
            // A `true` operand settles an `or` the same way.
            ref held if held.is_always_true() => return Term::always_true(),
            ref held if held.is_always_false() => {}
            Term::Or(inner) => {
                for held in inner.iter() {
                    push_unique(&mut flat, held.clone());
                }
            }
            other => push_unique(&mut flat, other),
        }
    }
    let flat = merge_memberships(flat);
    Term::any(flat)
}

fn push_unique(operands: &mut Vec<Term>, operand: Term) {
    if !operands.contains(&operand) {
        operands.push(operand);
    }
}

/// The term and the values an equality or membership test compares it with.
fn membership_parts(term: &Term) -> Option<(&Term, Vec<Term>)> {
    match term {
        Term::Compare(left, Comparison::Eq, right) => Some((left, vec![right.as_ref().clone()])),
        Term::In(value, list) => Some((value, list.iter().cloned().collect())),
        _ => None,
    }
}

/// `a = 1 or a = 2 or a in (3)` is `a in (1, 2, 3)`, exactly: both evaluate
/// `a` once and answer unknown for a null `a`.
fn merge_memberships(operands: Vec<Term>) -> Vec<Term> {
    let mut merged: Vec<Term> = Vec::with_capacity(operands.len());
    let mut groups: Vec<(Term, Vec<Term>, usize)> = Vec::new();
    for operand in operands {
        match membership_parts(&operand) {
            Some((left, values)) => {
                if let Some((_, held, _)) = groups.iter_mut().find(|(term, _, _)| term == left) {
                    for value in values {
                        if !held.contains(&value) {
                            held.push(value);
                        }
                    }
                } else {
                    // The group takes the slot its first member had, so the
                    // disjunction keeps the order it was written in.
                    groups.push((left.clone(), values, merged.len()));
                    merged.push(Term::always_false());
                }
            }
            None => merged.push(operand),
        }
    }
    for (left, values, slot) in groups {
        merged[slot] = membership(left, values);
    }
    merged
}

/// `a in (1, 2) and a in (2, 3)` is `a in (2)`, exactly, when the lists
/// share a value; two lists that share none stay apart, because their
/// conjunction is unknown for a null `a` and `false` is not.
fn intersect_memberships(operands: Vec<Term>) -> Vec<Term> {
    let mut merged: Vec<Term> = Vec::with_capacity(operands.len());
    let mut groups: Vec<(Term, Vec<Term>, usize)> = Vec::new();
    for operand in operands {
        match membership_parts(&operand) {
            Some((left, values)) => {
                if let Some((_, held, _)) = groups.iter_mut().find(|(term, _, _)| term == left) {
                    let narrowed: Vec<Term> = held
                        .iter()
                        .filter(|value| values.contains(value))
                        .cloned()
                        .collect();
                    if narrowed.is_empty() {
                        merged.push(operand);
                    } else {
                        *held = narrowed;
                    }
                } else {
                    groups.push((left.clone(), values, merged.len()));
                    merged.push(Term::always_false());
                }
            }
            None => merged.push(operand),
        }
    }
    for (left, values, slot) in groups {
        merged[slot] = membership(left, values);
    }
    merged
}

/// The membership test of one term against one list, in its smallest form.
fn membership(value: Term, values: Vec<Term>) -> Term {
    match values.len() {
        0 => Term::In(Box::new(value), Arc::from([])),
        1 => simplify_compare(
            value,
            Comparison::Eq,
            values.into_iter().next().unwrap_or_else(Term::always_false),
        ),
        _ => Term::In(Box::new(value), Arc::from(values)),
    }
}

/// Push a negation into the node it negates, where that node has a negation.
fn simplify_not(inner: Term) -> Term {
    match inner {
        Term::Not(held) => *held,
        Term::Literal(held) => match held.value().as_bool() {
            Some(value) => Term::literal(!value),
            None if held.is_null() => Term::Literal(held),
            None => Term::Not(Box::new(Term::Literal(held))),
        },
        // De Morgan holds in Kleene logic, so the negation moves inward
        // without changing which rows are unknown.
        Term::And(operands) => simplify_disjunction(
            &operands
                .iter()
                .map(|operand| Term::Not(Box::new(operand.clone())))
                .collect::<Vec<_>>(),
        ),
        Term::Or(operands) => simplify_conjunction(
            &operands
                .iter()
                .map(|operand| Term::Not(Box::new(operand.clone())))
                .collect::<Vec<_>>(),
        ),
        Term::Compare(left, comparison, right) => Term::Compare(left, comparison.negated(), right),
        Term::IsNull(held) => Term::IsNotNull(held),
        Term::IsNotNull(held) => Term::IsNull(held),
        other => Term::Not(Box::new(other)),
    }
}

/// One comparison, oriented `term op literal` and folded against a null.
fn simplify_compare(left: Term, comparison: Comparison, right: Term) -> Term {
    let (left, comparison, right) = if left.is_literal() && !right.is_literal() {
        (right, comparison.flipped(), left)
    } else {
        (left, comparison, right)
    };
    if right.is_null_literal() || left.is_null_literal() {
        let other = if right.is_null_literal() { left } else { right };
        return match comparison {
            // The two-valued tests are exactly the null tests against null.
            Comparison::IsDistinctFrom => Term::IsNotNull(Box::new(other)),
            Comparison::IsNotDistinctFrom => Term::IsNull(Box::new(other)),
            // Every other comparison with a null operand is unknown, whatever
            // the other side holds.
            _ => Term::literal(Scalar::Null),
        };
    }
    Term::Compare(Box::new(left), comparison, Box::new(right))
}

/// A total order over terms, consistent with structural equality.
///
/// Variant order is stable, followed by each variant's structural contents.
impl Ord for Term {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        let rank = variant_rank(self).cmp(&variant_rank(other));
        if rank != Ordering::Equal {
            return rank;
        }
        match (self, other) {
            (Self::Literal(left), Self::Literal(right)) => left.cmp(right),
            (Self::Parameter(left), Self::Parameter(right)) => left.cmp(right),
            (Self::Attribute(left), Self::Attribute(right)) => left.cmp(right),
            (Self::Path(left), Self::Path(right)) => left.iter().cmp(right.iter()),
            (Self::And(left), Self::And(right))
            | (Self::Or(left), Self::Or(right))
            | (Self::List(left), Self::List(right)) => left.iter().cmp(right.iter()),
            (Self::Not(left), Self::Not(right))
            | (Self::IsNull(left), Self::IsNull(right))
            | (Self::IsNotNull(left), Self::IsNotNull(right))
            | (Self::Negate(left), Self::Negate(right)) => left.cmp(right),
            (
                Self::Compare(left, left_op, left_right),
                Self::Compare(right, right_op, right_right),
            ) => left_op
                .cmp(right_op)
                .then_with(|| left.cmp(right))
                .then_with(|| left_right.cmp(right_right)),
            (Self::In(left, left_list), Self::In(right, right_list)) => left
                .cmp(right)
                .then_with(|| left_list.iter().cmp(right_list.iter())),
            (
                Self::Between(left, left_low, left_high),
                Self::Between(right, right_low, right_high),
            ) => left
                .cmp(right)
                .then_with(|| left_low.cmp(right_low))
                .then_with(|| left_high.cmp(right_high)),
            (
                Self::Like {
                    value: left,
                    pattern: left_pattern,
                    case_insensitive: left_case,
                    escape: left_escape,
                },
                Self::Like {
                    value: right,
                    pattern: right_pattern,
                    case_insensitive: right_case,
                    escape: right_escape,
                },
            ) => left
                .cmp(right)
                .then_with(|| left_pattern.cmp(right_pattern))
                .then_with(|| left_case.cmp(right_case))
                .then_with(|| left_escape.cmp(right_escape)),
            (Self::Glob(left, left_pattern), Self::Glob(right, right_pattern)) => left
                .cmp(right)
                .then_with(|| left_pattern.cmp(right_pattern)),
            (
                Self::Arithmetic(left, left_op, left_right),
                Self::Arithmetic(right, right_op, right_right),
            ) => left_op
                .cmp(right_op)
                .then_with(|| left.cmp(right))
                .then_with(|| left_right.cmp(right_right)),
            (Self::Function(left, left_args), Self::Function(right, right_args)) => left
                .cmp(right)
                .then_with(|| left_args.iter().cmp(right_args.iter())),
            (Self::Cast(left, left_type, left_safe), Self::Cast(right, right_type, right_safe)) => {
                left.cmp(right)
                    .then_with(|| left_type.cmp(right_type))
                    .then_with(|| left_safe.cmp(right_safe))
            }
            (
                Self::Case {
                    branches: left,
                    otherwise: left_else,
                },
                Self::Case {
                    branches: right,
                    otherwise: right_else,
                },
            ) => left
                .iter()
                .cmp(right.iter())
                .then_with(|| left_else.cmp(right_else)),
            (Self::Struct(left), Self::Struct(right)) => left.iter().cmp(right.iter()),
            (Self::Map(left), Self::Map(right)) => left.iter().cmp(right.iter()),
            // Every mixed pair was already settled by the variant rank.
            _ => Ordering::Equal,
        }
    }
}

impl PartialOrd for Term {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Order the variants themselves, so a mixed pair never compares equal.
const fn variant_rank(term: &Term) -> u8 {
    match term {
        Term::Literal(_) => 0,
        Term::Path(_) => 1,
        Term::Attribute(_) => 2,
        Term::Parameter(_) => 3,
        Term::And(_) => 4,
        Term::Or(_) => 5,
        Term::Not(_) => 6,
        Term::Compare(_, _, _) => 7,
        Term::In(_, _) => 8,
        Term::Between(_, _, _) => 9,
        Term::IsNull(_) => 10,
        Term::IsNotNull(_) => 11,
        Term::Like { .. } => 12,
        Term::Glob(_, _) => 13,
        Term::Arithmetic(_, _, _) => 14,
        Term::Negate(_) => 15,
        Term::Function(_, _) => 16,
        Term::Cast(_, _, _) => 17,
        Term::Case { .. } => 18,
        Term::Struct(_) => 19,
        Term::List(_) => 20,
        Term::Map(_) => 21,
    }
}

/// Negate one constant, when the constant is a number that can be negated.
///
/// Every signed family answers; an unsigned one does not, because `-1` is not
/// a `uint8` and silently widening it would change the type a comparison runs
/// in. A value that cannot be negated keeps its [`Term::Negate`] node.
fn negate_value(value: &Scalar) -> Option<Scalar> {
    matches!(
        value,
        Scalar::Int8(_)
            | Scalar::Int16(_)
            | Scalar::Int32(_)
            | Scalar::Int64(_)
            | Scalar::Int128(_)
            | Scalar::Float16(_)
            | Scalar::Float32(_)
            | Scalar::Float64(_)
            | Scalar::Decimal32(_)
            | Scalar::Decimal64(_)
            | Scalar::Decimal128(_)
            | Scalar::Decimal256(_)
            | Scalar::Duration32(_)
            | Scalar::Duration64(_)
    )
    .then(|| value.checked_neg().ok())
    .flatten()
}

impl TryFrom<&str> for Term {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        value.parse()
    }
}

/// A field path is the column it names.
impl From<super::FieldPath> for Term {
    fn from(value: super::FieldPath) -> Self {
        Self::Path(value.shared_segments())
    }
}

/// Build a column reference. The free spelling of [`Term::column`].
#[must_use]
pub fn col(name: impl Into<SmolStr>) -> Term {
    Term::column(name)
}

/// Build a constant. The free spelling of [`Term::literal`].
#[must_use]
pub fn lit(value: impl Into<Scalar>) -> Term {
    Term::literal(value)
}
