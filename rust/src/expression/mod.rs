//! One recursive, typed filter and projection tree.
//!
//! An [`Expression`] is a *plan over data*, and it is the one representation
//! this workspace uses to say which rows to keep and which values to compute.
//! Before it existed the same question was asked five times in the weakest
//! possible language - a `(column, value)` text pair, on
//! [`IOBase::children_where`](crate::io::IOBase::children_where), on
//! `IORecordOptions::filter_partitions`, and on three `Table` methods - and
//! none of those spellings could express a range, a null test, a nested path,
//! or a question about the file rather than the rows. The pairs survive as
//! sugar that builds an `Expression`; there is no second implementation behind
//! them.
//!
//! # An expression is not a value
//!
//! [`Value`](crate::Value) is the codec's lossless value tree: structural,
//! serializable, and meaningful on its own. An `Expression` is a computation
//! whose meaning depends on a schema. They meet at exactly two points -
//! [`Expression::Literal`] going in, and evaluation producing a `Value` coming
//! out - and keeping them apart is what lets `Value` stay plain structural data
//! while an expression carries schema-dependent meaning.
//!
//! # The four stages
//!
//! ```text
//! text ──parse──▶ Expression ──bind(schema)──▶ Bound ──▶ Value | ArrayRef | mask
//! ```
//!
//! 1. **Parse** ([`FromStr`](std::str::FromStr)) - one recursive grammar,
//!    re-entered by every nested construct, with byte-positioned errors.
//! 2. **Type** ([`Expression::field`]) - the output [`Field`](crate::Field)
//!    resolved against a schema, recursively, and the only place output
//!    types are decided.
//! 3. **Bind** ([`Expression::bind`]) - names become indices, literals are
//!    converted once to the type they are compared against, constants fold,
//!    and conjuncts are ordered cheapest-first. This happens **once per
//!    stream**, never per batch and never per row.
//! 4. **Evaluate** - the one [`Bound`] answers three ways: row at a time over
//!    [`Value`](crate::Value), vectorized over an Arrow `RecordBatch`, and
//!    three-valued over container statistics so a file, a manifest, or a
//!    directory is skipped without being read. Each way is one target's
//!    [`ApplyExpression`] implementation - the target owns how an expression
//!    applies to it, and `Bound`'s verbs are compositions over that.
//!
//! The scalar tier compiles with no Arrow at all; only the vectorized tier is
//! behind the `arrow` feature.

mod apply;
mod bind;
mod display;
mod eval;
mod parser;
mod pushdown;
mod selector;
mod serde;
mod typing;

#[cfg(feature = "arrow")]
mod arrow;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Result, TypedValue};

pub use apply::{ApplyExpression, ApplyExpressionStream};
pub use bind::{Bound, BoundStatement};
pub use parser::{Direction, NullsOrder, Order, Projection, Statement, needs_quoting};
pub use pushdown::{Bounds, ColumnBounds, Residual};
pub use selector::{Attributes, Cost, Handle, Selector, read_handle};

/// How deep an expression may nest before the parser and every walk refuse.
///
/// The limit is the schema grammar's, because the two parsers run over the same
/// kind of caller-controlled text and a stack is a stack: an expression that
/// nests past it is a typed error naming the limit, never an aborted process.
pub const RECURSION_LIMIT: usize = DataType::PARSE_RECURSION_LIMIT;

/// How many nodes one expression may hold.
///
/// Depth alone does not bound an expression: a flat `IN` list of a million
/// literals is one level deep and still unbounded work. The node budget is
/// checked once, before any recursive walk, so a walk never has to check.
pub const NODE_LIMIT: usize = 100_000;

/// One step of a path into a nested value.
///
/// Written once in the grammar, resolved once against the container's datatype,
/// and applied identically by the scalar and the vectorized evaluators.
#[derive(Clone, Debug, Eq, PartialEq, Hash, ::serde::Serialize, ::serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Segment {
    /// `.name` - a struct child, resolved ASCII case-insensitively the way
    /// every cast in this crate resolves a name.
    Field(SmolStr),
    /// `[0]`, `[-1]` - one list element by position, 0-based, a negative index
    /// counting back from the end. Out of range is null rather than an error,
    /// because absence is not a failure on the read path anywhere else here.
    Index(i64),
    /// `['k']` - one map entry by key, the key read once through the map's own
    /// key type. A struct child may also be reached this way when the key is
    /// text, which is the spelling JSON tooling already uses.
    Key(TypedValue),
}

impl Segment {
    /// Name a struct child.
    #[must_use]
    pub fn field(name: impl Into<SmolStr>) -> Self {
        Self::Field(name.into())
    }

    /// Name a list element by position.
    #[must_use]
    pub const fn index(position: i64) -> Self {
        Self::Index(position)
    }

    /// Name a map entry by key.
    ///
    /// # Errors
    ///
    /// Returns an error when the value and the datatype it is paired with
    /// disagree.
    pub fn key(value: crate::Value) -> Result<Self> {
        Ok(Self::Key(TypedValue::from_value(value)?))
    }
}

/// A comparison between two expressions.
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    ::serde::Serialize,
    ::serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    /// `=` - null on either side is unknown.
    Eq,
    /// `<>` - null on either side is unknown.
    NotEq,
    /// `<`
    Lt,
    /// `<=`
    LtEq,
    /// `>`
    Gt,
    /// `>=`
    GtEq,
    /// `is distinct from` - two-valued: null is a value that equals itself and
    /// differs from everything else, so this never answers unknown.
    IsDistinctFrom,
    /// `is not distinct from` - the two-valued equality, null included.
    IsNotDistinctFrom,
}

impl Comparison {
    /// The canonical text of this comparison.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::NotEq => "<>",
            Self::Lt => "<",
            Self::LtEq => "<=",
            Self::Gt => ">",
            Self::GtEq => ">=",
            Self::IsDistinctFrom => "is distinct from",
            Self::IsNotDistinctFrom => "is not distinct from",
        }
    }

    /// Return whether this comparison answers `true` or `false` for a null.
    ///
    /// Exactly two of them do, and everything that reasons about null
    /// propagation asks this rather than matching the variants again.
    #[must_use]
    pub const fn is_two_valued(self) -> bool {
        matches!(self, Self::IsDistinctFrom | Self::IsNotDistinctFrom)
    }

    /// The comparison that asks the same question with the operands swapped.
    ///
    /// This is what lets a rewrite orient every comparison as
    /// `column op literal` without changing what it asks.
    #[must_use]
    pub const fn flipped(self) -> Self {
        match self {
            Self::Lt => Self::Gt,
            Self::LtEq => Self::GtEq,
            Self::Gt => Self::Lt,
            Self::GtEq => Self::LtEq,
            other => other,
        }
    }

    /// The comparison that answers this one's negation.
    ///
    /// Sound under three-valued logic for every variant here: `not (a = b)` and
    /// `a <> b` are both unknown exactly when an operand is null, and the two
    /// distinctness tests are two-valued and are each other's complement.
    #[must_use]
    pub const fn negated(self) -> Self {
        match self {
            Self::Eq => Self::NotEq,
            Self::NotEq => Self::Eq,
            Self::Lt => Self::GtEq,
            Self::LtEq => Self::Gt,
            Self::Gt => Self::LtEq,
            Self::GtEq => Self::Lt,
            Self::IsDistinctFrom => Self::IsNotDistinctFrom,
            Self::IsNotDistinctFrom => Self::IsDistinctFrom,
        }
    }

    /// Answer this comparison from an [`Ordering`](std::cmp::Ordering).
    #[must_use]
    pub const fn answers(self, ordering: std::cmp::Ordering) -> bool {
        use std::cmp::Ordering::{Equal, Greater, Less};
        matches!(
            (self, ordering),
            (Self::Eq | Self::IsNotDistinctFrom, Equal)
                | (Self::NotEq | Self::IsDistinctFrom, Less | Greater)
                | (Self::Lt, Less)
                | (Self::LtEq, Less | Equal)
                | (Self::Gt, Greater)
                | (Self::GtEq, Greater | Equal)
        )
    }
}

/// A binary arithmetic operator.
///
/// Negation is deliberately *not* here. It is unary, and a binary node with a
/// fictional left operand would be a lie the evaluator then has to remember;
/// [`Expression::Negate`] is its own node instead.
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    ::serde::Serialize,
    ::serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Operator {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
}

impl Operator {
    /// The canonical text of this operator.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Rem => "%",
        }
    }
}

/// Whether a cast nulls what it cannot convert, or refuses it.
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    Default,
    ::serde::Serialize,
    ::serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Safety {
    /// A value the target cannot hold is an error naming both sides.
    #[default]
    Strict,
    /// A value the target cannot hold becomes null, matching
    /// `IORecordOptions::safe` and Arrow's own cast policy.
    Safe,
}

impl Safety {
    /// Return whether an unconvertible value becomes null.
    #[must_use]
    #[inline]
    pub const fn is_safe(self) -> bool {
        matches!(self, Self::Safe)
    }

    /// The keyword this safety spells.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "cast",
            Self::Safe => "try_cast",
        }
    }
}

/// The closed set of scalar functions this grammar spells.
///
/// Closed deliberately: an open registry is a plugin system, and a plugin
/// system cannot promise that the scalar evaluator, the vectorized evaluator,
/// and the statistics evaluator agree about a function none of them knows. A
/// name outside this set is a parse error listing the vocabulary it is not in.
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    ::serde::Serialize,
    ::serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Function {
    /// Lowercased text.
    Lower,
    /// Uppercased text.
    Upper,
    /// Characters of text, or bytes of a binary value.
    Length,
    /// `substring(text, start [, length])`, 1-based as SQL's is - deliberately
    /// unlike the 0-based `[]` segment, and said so beside both.
    Substring,
    /// Text with leading and trailing whitespace removed.
    Trim,
    /// Whether text begins with a literal prefix - the one text predicate a
    /// statistics range can prune.
    StartsWith,
    /// Whether text ends with a literal suffix.
    EndsWith,
    /// Whether text holds a literal substring.
    Contains,
    /// Text joined end to end.
    Concat,
    /// The calendar year of a date or timestamp.
    Year,
    /// The calendar month, 1 through 12.
    Month,
    /// The calendar day of month, 1 through 31.
    Day,
    /// The clock hour, 0 through 23.
    Hour,
    /// `truncate(value, unit_or_width)` - a temporal floored to a unit, or a
    /// number floored to a multiple.
    Truncate,
    /// The first argument that is not null.
    Coalesce,
    /// `if_null(value, fallback)` - two-argument [`Self::Coalesce`], the
    /// spelling several dialects use.
    IfNull,
    /// How many items a list or a map holds.
    Size,
    /// `get(container, key_or_index)` - the functional spelling of a
    /// [`Segment`], for when the key is computed rather than written.
    ///
    /// Spelled `get` rather than `element_at` deliberately: several engines
    /// ship an `element_at` and they disagree about whether its index is
    /// 0-based or 1-based, so the familiar name cannot be used without
    /// inheriting an argument about what it means.
    Get,
}

impl Function {
    /// Every function this grammar knows, in canonical spelling.
    pub const ALL: [Self; 18] = [
        Self::Lower,
        Self::Upper,
        Self::Length,
        Self::Substring,
        Self::Trim,
        Self::StartsWith,
        Self::EndsWith,
        Self::Contains,
        Self::Concat,
        Self::Year,
        Self::Month,
        Self::Day,
        Self::Hour,
        Self::Truncate,
        Self::Coalesce,
        Self::IfNull,
        Self::Size,
        Self::Get,
    ];

    /// The canonical lowercase name of this function.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lower => "lower",
            Self::Upper => "upper",
            Self::Length => "length",
            Self::Substring => "substring",
            Self::Trim => "trim",
            Self::StartsWith => "starts_with",
            Self::EndsWith => "ends_with",
            Self::Contains => "contains",
            Self::Concat => "concat",
            Self::Year => "year",
            Self::Month => "month",
            Self::Day => "day",
            Self::Hour => "hour",
            Self::Truncate => "truncate",
            Self::Coalesce => "coalesce",
            Self::IfNull => "if_null",
            Self::Size => "size",
            Self::Get => "get",
        }
    }

    /// Resolve a name, ASCII case-insensitively, including dialect aliases.
    ///
    /// The aliases are the spellings other engines use for the same operation;
    /// they resolve to the one canonical variant, so the evaluators never learn
    /// that a dialect exists.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let lowered = name.to_ascii_lowercase();
        Some(match lowered.as_str() {
            "lower" | "lcase" => Self::Lower,
            "upper" | "ucase" => Self::Upper,
            "length" | "len" | "char_length" | "character_length" => Self::Length,
            "substring" | "substr" => Self::Substring,
            "trim" | "btrim" => Self::Trim,
            "starts_with" | "startswith" => Self::StartsWith,
            "ends_with" | "endswith" => Self::EndsWith,
            "contains" | "strpos_contains" => Self::Contains,
            "concat" => Self::Concat,
            "year" => Self::Year,
            "month" => Self::Month,
            "day" | "dayofmonth" => Self::Day,
            "hour" => Self::Hour,
            "truncate" | "trunc" | "date_trunc" => Self::Truncate,
            "coalesce" => Self::Coalesce,
            "if_null" | "ifnull" | "nvl" | "isnull" => Self::IfNull,
            "size" | "cardinality" => Self::Size,
            "get" => Self::Get,
            _ => return None,
        })
    }

    /// The inclusive argument-count range this function accepts.
    #[must_use]
    pub const fn arity(self) -> (usize, usize) {
        match self {
            // The two variadics are the ones every dialect spells variadically.
            Self::Coalesce | Self::Concat => (1, usize::MAX),
            Self::Substring => (2, 3),
            Self::StartsWith
            | Self::EndsWith
            | Self::Contains
            | Self::Truncate
            | Self::IfNull
            | Self::Get => (2, 2),
            _ => (1, 1),
        }
    }

    /// Return whether this function reads a calendar field off a temporal.
    #[must_use]
    pub const fn is_calendar(self) -> bool {
        matches!(self, Self::Year | Self::Month | Self::Day | Self::Hour)
    }

    /// Every function name this grammar accepts, for an error message.
    #[must_use]
    pub fn vocabulary() -> String {
        Self::ALL
            .iter()
            .map(|function| function.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// One recursive, typed filter and projection tree.
///
/// The variants are grouped by what kind of node they are, and each group is
/// documented as a group rather than variant by variant, because what matters
/// about `Or` is what matters about `And`.
///
/// Nesting is shared through [`Arc`], so cloning a large predicate bumps
/// reference counts rather than copying a tree, and an operand list with no
/// elements carries no allocation.
///
/// ```
/// use yggdryl::Expression;
///
/// # fn main() -> yggdryl::Result<()> {
/// let filter: Expression = "ccy = 'EUR' and price > 100".parse()?;
/// assert_eq!(filter.to_string(), "ccy = 'EUR' and price > 100");
/// assert_eq!(filter.columns(), vec!["ccy".to_owned(), "price".to_owned()]);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Hash, ::serde::Serialize, ::serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Expression {
    // ---- Leaves: nodes with no expression children -----------------------
    /// A constant, carrying the datatype it belongs to.
    ///
    /// A literal is a [`TypedValue`] and never a bare Rust primitive, so
    /// `decimal '1.50'` stays an exact decimal at scale two all the way to the
    /// comparison rather than becoming an integer that happens to print alike.
    Literal(TypedValue),
    /// A top-level column of the row, by name.
    Column(SmolStr),
    /// A path into a nested value: a base expression and the steps that reach
    /// inside it, so `trade.legs[0]['ccy']` is one node resolved recursively.
    Path(Box<Expression>, Arc<[Segment]>),
    /// An attribute of the *handle* rather than of the rows - `&holder.size`,
    /// `&holder.partition['year']`. See [`Selector`] for the cost table.
    Attribute(Selector),
    /// A late-bound value, supplied when the expression is bound.
    Parameter(SmolStr),

    // ---- Logical: n-ary, flattened at construction ------------------------
    /// Conjunction. Empty is `true`.
    And(Arc<[Expression]>),
    /// Disjunction. Empty is `false`.
    Or(Arc<[Expression]>),
    /// Three-valued negation: `not unknown` is unknown.
    Not(Box<Expression>),

    // ---- Comparison: everything that answers a boolean about values -------
    /// A comparison of two expressions.
    Compare(Box<Expression>, Comparison, Box<Expression>),
    /// Set membership. `x in ()` is refused by the parser rather than folded,
    /// because an empty list is always a typo.
    In(Box<Expression>, Arc<[Expression]>),
    /// An inclusive range test, which lowers to two comparisons.
    Between(Box<Expression>, Box<Expression>, Box<Expression>),
    /// `is null` - one of the two operators that answer true or false about a
    /// null rather than unknown.
    IsNull(Box<Expression>),
    /// `is not null`.
    IsNotNull(Box<Expression>),
    /// SQL `like` / `ilike`, with `_` and `%` wildcards.
    Like {
        /// The text being matched.
        value: Box<Expression>,
        /// The pattern.
        pattern: Box<Expression>,
        /// Whether the match ignores case.
        case_insensitive: bool,
        /// The character that escapes a wildcard, when the clause names one.
        escape: Option<char>,
    },
    /// A path-glob match with the `.gitignore` rule this crate already uses for
    /// listings: no separator matches at any depth, a separator anchors at the
    /// root. Delegates to [`Url::matches_glob`](crate::Url::matches_glob).
    Glob(Box<Expression>, Box<Expression>),

    // ---- Arithmetic -------------------------------------------------------
    /// Arithmetic over two operands, with the promotion rules
    /// [`Expression::field`] states - a decimal never becomes a float to be
    /// added.
    Arithmetic(Box<Expression>, Operator, Box<Expression>),
    /// Arithmetic negation.
    Negate(Box<Expression>),

    // ---- Scalar functions -------------------------------------------------
    /// A call into the closed [`Function`] set.
    Function(Function, Arc<[Expression]>),

    // ---- Shape: nodes that change or build a type -------------------------
    /// A schema-directed cast, reaching the one cast this crate owns.
    Cast(Box<Expression>, DataType, Safety),
    /// A searched conditional: `case when c then v ... else v end`.
    Case {
        /// The `when`/`then` pairs, tried in order.
        branches: Arc<[(Expression, Expression)]>,
        /// The `else` value; absent means null.
        otherwise: Option<Box<Expression>>,
    },
    /// Build a struct value from named children.
    Struct(Arc<[(SmolStr, Expression)]>),
    /// Build a list value from its elements.
    List(Arc<[Expression]>),
    /// Build a map value from its entries.
    Map(Arc<[(Expression, Expression)]>),
}

impl Expression {
    /// Return a deterministic hash of the canonical expression text.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_display(self)
    }

    /// The expression that is true for every row.
    #[must_use]
    pub fn always_true() -> Self {
        Self::literal(crate::Value::Bool(true))
    }

    /// The expression that is true for no row.
    #[must_use]
    pub fn always_false() -> Self {
        Self::literal(crate::Value::Bool(false))
    }

    /// Hold a constant, inferring the datatype it belongs to.
    ///
    /// # Panics
    ///
    /// Never: every [`Value`](crate::Value) this crate builds names a datatype,
    /// and one that does not is held as the null it is.
    #[must_use]
    pub fn literal(value: impl Into<crate::Value>) -> Self {
        let value = value.into();
        TypedValue::from_value(value).map_or_else(
            |_| {
                Self::Literal(
                    TypedValue::from_parts(DataType::Null, crate::Value::Null)
                        .unwrap_or_else(|_| unreachable!("null belongs to the null datatype")),
                )
            },
            Self::Literal,
        )
    }

    /// Hold a constant under an exact datatype.
    ///
    /// # Errors
    ///
    /// Returns an error when the value and the datatype disagree.
    pub fn typed_literal(data_type: DataType, value: crate::Value) -> Result<Self> {
        Ok(Self::Literal(TypedValue::from_parts(data_type, value)?))
    }

    /// Name a top-level column.
    #[must_use]
    pub fn column(name: impl Into<SmolStr>) -> Self {
        Self::Column(name.into())
    }

    /// Name a handle attribute.
    #[must_use]
    pub const fn attribute(selector: Selector) -> Self {
        Self::Attribute(selector)
    }

    /// Name a late-bound value.
    #[must_use]
    pub fn parameter(name: impl Into<SmolStr>) -> Self {
        Self::Parameter(name.into())
    }

    /// Reach inside this expression's value.
    ///
    /// A path onto a path extends the existing chain rather than nesting a
    /// second node, which is what keeps `a.b.c` one node and makes equality
    /// between two spellings of the same path structural.
    #[must_use]
    pub fn path(self, segments: impl IntoIterator<Item = Segment>) -> Self {
        let mut steps: Vec<Segment> = Vec::new();
        let base = match self {
            Self::Path(base, held) => {
                steps.extend(held.iter().cloned());
                *base
            }
            other => other,
        };
        steps.extend(segments);
        if steps.is_empty() {
            return base;
        }
        Self::Path(Box::new(base), Arc::from(steps))
    }

    /// Reach one struct child, or one string-keyed map entry.
    #[must_use]
    pub fn child(self, name: impl Into<SmolStr>) -> Self {
        self.path([Segment::Field(name.into())])
    }

    /// Reach one list element by position, 0-based.
    #[must_use]
    pub fn at(self, index: i64) -> Self {
        self.path([Segment::Index(index)])
    }

    /// Conjoin every expression, flattening nested conjunctions.
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

    /// Disjoin every expression, flattening nested disjunctions.
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
                if let Ok(folded) = TypedValue::from_parts(held.data_type().clone(), negated) {
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
    pub fn cast(self, data_type: DataType) -> Self {
        Self::Cast(Box::new(self), data_type, Safety::Strict)
    }

    /// Convert to another datatype, nulling what the target cannot hold.
    #[must_use]
    pub fn try_cast(self, data_type: DataType) -> Self {
        Self::Cast(Box::new(self), data_type, Safety::Safe)
    }

    /// Build a searched conditional.
    #[must_use]
    pub fn case(branches: impl IntoIterator<Item = (Self, Self)>, otherwise: Option<Self>) -> Self {
        Self::Case {
            branches: branches.into_iter().collect(),
            otherwise: otherwise.map(Box::new),
        }
    }

    /// Return whether this expression is a constant.
    #[must_use]
    #[inline]
    pub const fn is_literal(&self) -> bool {
        matches!(self, Self::Literal(_))
    }

    /// Borrow the constant this expression holds, if it holds one.
    #[must_use]
    #[inline]
    pub const fn as_literal(&self) -> Option<&TypedValue> {
        match self {
            Self::Literal(value) => Some(value),
            _ => None,
        }
    }

    /// Borrow the column name this expression reads, if it reads one directly.
    #[must_use]
    #[inline]
    pub fn as_column(&self) -> Option<&str> {
        match self {
            Self::Column(name) => Some(name),
            _ => None,
        }
    }

    /// Return whether this expression is the constant `true`.
    #[must_use]
    pub fn is_always_true(&self) -> bool {
        match self {
            Self::Literal(held) => matches!(held.value(), crate::Value::Bool(true)),
            Self::And(operands) => operands.is_empty(),
            _ => false,
        }
    }

    /// Return whether this expression is the constant `false`.
    #[must_use]
    pub fn is_always_false(&self) -> bool {
        match self {
            Self::Literal(held) => matches!(held.value(), crate::Value::Bool(false)),
            Self::Or(operands) => operands.is_empty(),
            _ => false,
        }
    }

    /// Visit every direct child of this node, in evaluation order.
    ///
    /// One traversal serves every walk in the module, so a variant added later
    /// is wired into all of them by editing exactly one function.
    pub(crate) fn for_each_child<'node>(&'node self, mut visit: impl FnMut(&'node Self)) {
        match self {
            Self::Literal(_) | Self::Column(_) | Self::Attribute(_) | Self::Parameter(_) => {}
            Self::Path(base, _) => visit(base),
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

    /// How deep this expression nests, counting itself as one level.
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

    /// How many nodes this expression holds.
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

    /// Refuse an expression past the depth or node budget.
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

    /// Every top-level column this expression reads, deduplicated in
    /// first-seen order.
    ///
    /// This is what drives projection pushdown: a read decodes exactly the
    /// columns the predicate and the projection name, and no more.
    #[must_use]
    pub fn columns(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        self.walk(&mut |node| {
            if let Self::Column(name) = node {
                if !names.iter().any(|held| held.eq_ignore_ascii_case(name)) {
                    names.push(name.to_string());
                }
            }
        });
        names
    }

    /// Every handle attribute this expression reads, in first-seen order.
    #[must_use]
    pub fn attributes(&self) -> Vec<Selector> {
        let mut found: Vec<Selector> = Vec::new();
        self.walk(&mut |node| {
            if let Self::Attribute(selector) = node {
                if !found.contains(selector) {
                    found.push(selector.clone());
                }
            }
        });
        found
    }

    /// Every parameter this expression names, in first-seen order.
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

    /// Return whether this expression reads any handle attribute.
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

    /// Walk every node of this expression, depth-first, in evaluation order.
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
}

/// A total order over expressions, consistent with structural equality.
///
/// Variant order is stable, followed by each variant's structural contents.
impl Ord for Expression {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        let rank = variant_rank(self).cmp(&variant_rank(other));
        if rank != Ordering::Equal {
            return rank;
        }
        match (self, other) {
            (Self::Literal(left), Self::Literal(right)) => left.cmp(right),
            (Self::Column(left), Self::Column(right))
            | (Self::Parameter(left), Self::Parameter(right)) => left.cmp(right),
            (Self::Attribute(left), Self::Attribute(right)) => left.cmp(right),
            (Self::Path(left, left_steps), Self::Path(right, right_steps)) => {
                left.cmp(right).then_with(|| left_steps.cmp(right_steps))
            }
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

impl PartialOrd for Expression {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Order the variants themselves, so a mixed pair never compares equal.
const fn variant_rank(expression: &Expression) -> u8 {
    match expression {
        Expression::Literal(_) => 0,
        Expression::Column(_) => 1,
        Expression::Path(_, _) => 2,
        Expression::Attribute(_) => 3,
        Expression::Parameter(_) => 4,
        Expression::And(_) => 5,
        Expression::Or(_) => 6,
        Expression::Not(_) => 7,
        Expression::Compare(_, _, _) => 8,
        Expression::In(_, _) => 9,
        Expression::Between(_, _, _) => 10,
        Expression::IsNull(_) => 11,
        Expression::IsNotNull(_) => 12,
        Expression::Like { .. } => 13,
        Expression::Glob(_, _) => 14,
        Expression::Arithmetic(_, _, _) => 15,
        Expression::Negate(_) => 16,
        Expression::Function(_, _) => 17,
        Expression::Cast(_, _, _) => 18,
        Expression::Case { .. } => 19,
        Expression::Struct(_) => 20,
        Expression::List(_) => 21,
        Expression::Map(_) => 22,
    }
}

/// A total order over path segments, consistent with structural equality.
impl Ord for Segment {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        let rank = |segment: &Self| match segment {
            Self::Field(_) => 0_u8,
            Self::Index(_) => 1,
            Self::Key(_) => 2,
        };
        let ordered = rank(self).cmp(&rank(other));
        if ordered != Ordering::Equal {
            return ordered;
        }
        match (self, other) {
            (Self::Field(left), Self::Field(right)) => left.cmp(right),
            (Self::Index(left), Self::Index(right)) => left.cmp(right),
            (Self::Key(left), Self::Key(right)) => left.cmp(right),
            _ => Ordering::Equal,
        }
    }
}

impl PartialOrd for Segment {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Negate one constant, when the constant is a number that can be negated.
///
/// Every signed family answers; an unsigned one does not, because `-1` is not
/// a `uint8` and silently widening it would change the type a comparison runs
/// in. A value that cannot be negated keeps its [`Expression::Negate`] node.
fn negate_value(value: &crate::Value) -> Option<crate::Value> {
    matches!(
        value,
        crate::Value::I8(_)
            | crate::Value::I16(_)
            | crate::Value::I32(_)
            | crate::Value::I64(_)
            | crate::Value::I128(_)
            | crate::Value::F16(_)
            | crate::Value::F32(_)
            | crate::Value::F64(_)
            | crate::Value::D128(..)
            | crate::Value::D256(..)
            | crate::Value::Duration32(..)
            | crate::Value::Duration64(..)
    )
    .then(|| value.checked_neg().ok())
    .flatten()
}

/// Anything a call site may hand over where an expression is wanted.
///
/// Text *parses* here. That is the whole point of the trait: an `impl
/// Into<Expression>` for `&str` would quietly make `"ccy = 'EUR'"` a string
/// literal - a perfectly valid expression that filters nothing and reports no
/// error - and a filter that silently matches everything is the worst failure
/// this module could have. Parsing is fallible, so the conversion is fallible,
/// and a typo arrives as a byte-positioned parse error at the call.
pub trait IntoExpression {
    /// Produce the expression this value stands for.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the value is text that is not an expression.
    fn into_expression(self) -> Result<Expression>;
}

impl IntoExpression for Expression {
    fn into_expression(self) -> Result<Self> {
        Ok(self)
    }
}

impl IntoExpression for &Expression {
    fn into_expression(self) -> Result<Expression> {
        Ok(self.clone())
    }
}

impl IntoExpression for &str {
    fn into_expression(self) -> Result<Expression> {
        self.parse()
    }
}

impl IntoExpression for &String {
    fn into_expression(self) -> Result<Expression> {
        self.parse()
    }
}

impl IntoExpression for String {
    fn into_expression(self) -> Result<Expression> {
        self.parse()
    }
}

impl Expression {
    /// The predicate one column-equals-value pair spells about the *rows*.
    ///
    /// This is the sugar half of "one representation": the `(&str, &str)`
    /// pairs every read option and every table method already take build an
    /// expression here and are answered by the one evaluator, rather than by a
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
    pub fn partition_equals(column: &str, value: &str, data_type: &DataType) -> Self {
        let reference = Self::column(column);
        if value == crate::io::NULL_PARTITION {
            return reference.is_null();
        }
        reference.eq(Self::literal(value).try_cast(data_type.clone()))
    }

    /// The predicate one column-equals-value pair spells about the *holder*.
    ///
    /// The same pair, asked of the path rather than of the rows. A listing can
    /// answer this one without opening anything, which is why the two
    /// spellings are kept apart instead of one guessing which was meant.
    #[must_use]
    pub fn holder_partition_equals(column: &str, value: &str) -> Self {
        Self::attribute(Selector::Partition(column.into())).eq(Self::literal(value))
    }

    /// Conjoin every pair as a predicate about the rows of a schema.
    ///
    /// A pair naming a column the schema does not declare is left out rather
    /// than refused: on a partitioned read the leaf's own path already
    /// answered for it, and a filter that has been answered elsewhere is not
    /// an error.
    #[must_use]
    pub fn all_partitions_equal<C: AsRef<str>, V: AsRef<str>>(
        schema: &crate::Field,
        pairs: impl IntoIterator<Item = (C, V)>,
    ) -> Self {
        Self::all(pairs.into_iter().filter_map(|(column, value)| {
            schema.get_field_by_name(column.as_ref()).map(|field| {
                Self::partition_equals(column.as_ref(), value.as_ref(), field.data_type())
            })
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
        let attribute = Self::attribute(Selector::Partition(column.into()));
        attribute
            .clone()
            .is_not_null()
            .and(attribute.eq(Self::literal(value)))
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

impl TryFrom<&str> for Expression {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        value.parse()
    }
}

/// Build a column reference. The free spelling of [`Expression::column`].
#[must_use]
pub fn col(name: impl Into<SmolStr>) -> Expression {
    Expression::column(name)
}

/// Build a constant. The free spelling of [`Expression::literal`].
#[must_use]
pub fn lit(value: impl Into<crate::Value>) -> Expression {
    Expression::literal(value)
}
