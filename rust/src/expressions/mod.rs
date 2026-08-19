//! One expression value, parsed from SQL-like text and evaluated three ways.
//!
//! An [`Expr`] is a value like [`Value`] or [`DataType`] is a value: it names a
//! computation over the columns of a row without knowing where those rows live,
//! it round-trips through its own canonical [`Display`](std::fmt::Display), and
//! it carries no schema, no handle, and no table format. Binding it against a
//! struct [`Field`] once produces a [`Bound`] plan, and that one plan answers
//! every question the rest of the crate asks:
//!
//! - **row at a time**, over a [`Value`] record ([`Bound::matches`]);
//! - **vectorized**, over an Arrow `RecordBatch` ([`Bound::mask`]);
//! - **three-valued over statistics**, so a file, a manifest, or a partition
//!   directory is skipped without being read ([`Bound::evaluate_stats`]).
//!
//! The three agree by construction, because they read the same plan. That is
//! the whole point: a filter that prunes and a filter that selects must never
//! be two implementations of one comparison.
//!
//! ```
//! use yggdryl::expressions::Expr;
//!
//! # fn main() -> yggdryl::Result<()> {
//! let filter: Expr = "venue = 'XNAS' AND price > 10".parse()?;
//! assert_eq!(filter.to_string(), "venue = 'XNAS' AND price > 10");
//! assert_eq!(filter.columns(), vec!["venue".to_owned(), "price".to_owned()]);
//! # Ok(())
//! # }
//! ```
//!
//! # Null semantics
//!
//! Evaluation is SQL three-valued logic, in every one of the three evaluators.
//! A comparison with a null operand is *unknown*, not false; a filter keeps a
//! row only on `true`; and [`Expr::is_null`] is the only way to select absence.
//! `venue != 'XNAS'` therefore does **not** select the rows whose venue is
//! null - `venue IS NULL` is how that is asked.

mod apply;
#[cfg(feature = "arrow")]
mod arrow;
mod bound;
mod eval;
mod graph;
mod optimize;
mod parser;
mod select;
mod stats;
#[cfg(test)]
mod tests;

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Result, Value};

pub use apply::{Applicable, Apply, Program};
#[cfg(feature = "arrow")]
pub use arrow::{ArrowApplicable, ArrowApply};
pub use bound::{Bound, BoundColumn, BoundPredicate, ColumnRef, Step};
pub use graph::Node;
pub use graph::{NodeId, Plan};
pub use optimize::Explanation;
pub use select::{BoundSelection, Selection, SelectionItem};
pub use stats::{Certainty, ColumnStats, StatsSource};

/// The prelude a Rust caller imports to write predicates in one line.
///
/// A trait method does not exist until its trait is in scope, and this module
/// has three traits a caller reaches for constantly, so one `use` brings the
/// expression vocabulary and both apply verbs in together.
pub mod prelude {
    #[cfg(feature = "arrow")]
    pub use super::ArrowApply;
    pub use super::{Apply, Bound, Expr, Selection, col, lit};
}

/// How deep an expression may nest before the parser and the evaluators refuse.
///
/// The limit is the schema grammar's, because the two parsers run over the same
/// kind of caller-controlled text and a stack is a stack: an expression that
/// nests past it is a typed error naming the limit, never an aborted process.
pub const RECURSION_LIMIT: usize = DataType::PARSE_RECURSION_LIMIT;

/// A comparison between two expressions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareOp {
    /// `=`
    Eq,
    /// `<>`
    NotEq,
    /// `<`
    Lt,
    /// `<=`
    LtEq,
    /// `>`
    Gt,
    /// `>=`
    GtEq,
}

impl CompareOp {
    /// The canonical SQL spelling of this operator.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::NotEq => "<>",
            Self::Lt => "<",
            Self::LtEq => "<=",
            Self::Gt => ">",
            Self::GtEq => ">=",
        }
    }

    /// The operator that answers the same question with the operands swapped.
    ///
    /// This is what lets the optimizer orient every comparison as
    /// `column op literal` without changing what it asks.
    #[must_use]
    pub const fn flipped(self) -> Self {
        match self {
            Self::Eq => Self::Eq,
            Self::NotEq => Self::NotEq,
            Self::Lt => Self::Gt,
            Self::LtEq => Self::GtEq,
            Self::Gt => Self::Lt,
            Self::GtEq => Self::LtEq,
        }
    }

    /// The operator that answers the negation of this one.
    ///
    /// Under three-valued logic this is only usable where the operands are
    /// known non-null, so the callers that use it say why they may.
    #[must_use]
    pub const fn negated(self) -> Self {
        match self {
            Self::Eq => Self::NotEq,
            Self::NotEq => Self::Eq,
            Self::Lt => Self::GtEq,
            Self::LtEq => Self::Gt,
            Self::Gt => Self::LtEq,
            Self::GtEq => Self::Lt,
        }
    }

    /// Answer this comparison from an [`Ordering`](std::cmp::Ordering).
    #[must_use]
    pub const fn answers(self, ordering: std::cmp::Ordering) -> bool {
        use std::cmp::Ordering::{Equal, Greater, Less};
        matches!(
            (self, ordering),
            (Self::Eq, Equal)
                | (Self::NotEq, Less | Greater)
                | (Self::Lt, Less)
                | (Self::LtEq, Less | Equal)
                | (Self::Gt, Greater)
                | (Self::GtEq, Greater | Equal)
        )
    }
}

impl fmt::Display for CompareOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// An arithmetic operator over two expressions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArithOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Mod,
}

impl ArithOp {
    /// The canonical SQL spelling of this operator.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Mod => "%",
        }
    }
}

impl fmt::Display for ArithOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The closed set of scalar functions the grammar spells.
///
/// It is closed deliberately: an open function registry is a plugin system, and
/// a plugin system cannot promise that the row evaluator, the vectorized
/// evaluator, and the statistics evaluator agree. A name outside this set is a
/// parse error listing the vocabulary it is not in.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Function {
    /// The first non-null argument.
    Coalesce,
    /// Characters of a string, or bytes of a binary value.
    Length,
    /// Lowercased text.
    Lower,
    /// Uppercased text.
    Upper,
    /// Text with leading and trailing whitespace removed.
    Trim,
    /// `substring(text, start)` or `substring(text, start, length)`, 1-based
    /// like SQL's own - deliberately unlike the 0-based `[]` accessor, and said
    /// so in the documentation next to both.
    Substring,
    /// Absolute value.
    Abs,
    /// `truncate(value, width)`: the largest multiple of `width` at or below
    /// `value`, which is the shape a bucketed or truncated partition stores.
    Truncate,
    /// The calendar year of a date or timestamp.
    Year,
    /// The calendar month, 1 through 12.
    Month,
    /// The calendar day of month, 1 through 31.
    Day,
    /// The clock hour, 0 through 23.
    Hour,
    /// The clock minute, 0 through 59.
    Minute,
    /// The clock second, 0 through 59.
    Second,
}

impl Function {
    /// Every function this grammar knows, in canonical spelling.
    pub const ALL: [Self; 14] = [
        Self::Coalesce,
        Self::Length,
        Self::Lower,
        Self::Upper,
        Self::Trim,
        Self::Substring,
        Self::Abs,
        Self::Truncate,
        Self::Year,
        Self::Month,
        Self::Day,
        Self::Hour,
        Self::Minute,
        Self::Second,
    ];

    /// The canonical lowercase name of this function.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Coalesce => "coalesce",
            Self::Length => "length",
            Self::Lower => "lower",
            Self::Upper => "upper",
            Self::Trim => "trim",
            Self::Substring => "substring",
            Self::Abs => "abs",
            Self::Truncate => "truncate",
            Self::Year => "year",
            Self::Month => "month",
            Self::Day => "day",
            Self::Hour => "hour",
            Self::Minute => "minute",
            Self::Second => "second",
        }
    }

    /// Return whether this function reads a calendar field off a temporal.
    #[must_use]
    pub const fn is_calendar(self) -> bool {
        matches!(
            self,
            Self::Year | Self::Month | Self::Day | Self::Hour | Self::Minute | Self::Second
        )
    }

    /// The inclusive argument-count range this function accepts.
    #[must_use]
    pub const fn arity(self) -> (usize, usize) {
        match self {
            // `coalesce` is the one variadic: it is the null-handling idiom
            // every SQL dialect spells the same way and capping it would be
            // arbitrary.
            Self::Coalesce => (1, usize::MAX),
            Self::Substring => (2, 3),
            Self::Truncate => (2, 2),
            _ => (1, 1),
        }
    }

    /// Resolve a name, ASCII case-insensitively, including dialect aliases.
    ///
    /// The aliases are the ones every SQL flavor spells differently for the
    /// same operation; they resolve to the one canonical variant, so the
    /// evaluator never learns that a dialect exists.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let lowered = name.to_ascii_lowercase();
        Some(match lowered.as_str() {
            "coalesce" | "ifnull" | "nvl" | "isnull" => Self::Coalesce,
            "length" | "len" | "char_length" | "character_length" => Self::Length,
            "lower" | "lcase" => Self::Lower,
            "upper" | "ucase" => Self::Upper,
            "trim" | "btrim" => Self::Trim,
            "substring" | "substr" => Self::Substring,
            "abs" => Self::Abs,
            "truncate" | "trunc" => Self::Truncate,
            "year" => Self::Year,
            "month" => Self::Month,
            "day" | "dayofmonth" => Self::Day,
            "hour" => Self::Hour,
            "minute" => Self::Minute,
            "second" => Self::Second,
            _ => return None,
        })
    }
}

impl fmt::Display for Function {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One step of a column path: reaching inside the value a column holds.
///
/// Written once in the grammar, resolved once at bind time against the
/// container's datatype, and applied identically by all three evaluators.
/// Indices are **0-based** and ranges are **half-open**, matching
/// [`Value::get`] and Rust's own slicing - stated here because a reader
/// arriving from a 1-based dialect must be told exactly once.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accessor {
    /// `a.b` - a struct child by name, or a string-keyed map entry.
    Child(SmolStr),
    /// `a['k']` - a map entry by key, or a struct child when the key is text.
    Key(Value),
    /// `a[0]`, `a[-1]` - one item by position; a negative index counts back
    /// from the end, and an index outside the value is null rather than an
    /// error, because a ragged list must not abort a scan.
    Index(i64),
    /// `a[1:3]`, `a[1:]`, `a[:3]`, `a[:]` - a half-open range of items. An
    /// out-of-range bound clamps and an inverted range is empty; neither is an
    /// error, for the same reason.
    Range {
        /// The inclusive lower bound, or the start when absent.
        start: Option<i64>,
        /// The exclusive upper bound, or the end when absent.
        end: Option<i64>,
    },
}

impl fmt::Display for Accessor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Child(name) => {
                formatter.write_str(".")?;
                write_identifier(formatter, name)
            }
            Self::Key(value) => write!(formatter, "[{}]", Literal(value)),
            Self::Index(index) => write!(formatter, "[{index}]"),
            Self::Range { start, end } => {
                formatter.write_str("[")?;
                if let Some(start) = start {
                    write!(formatter, "{start}")?;
                }
                formatter.write_str(":")?;
                if let Some(end) = end {
                    write!(formatter, "{end}")?;
                }
                formatter.write_str("]")
            }
        }
    }
}

/// A named column and the chain of accessors that reaches inside its value.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct Column {
    /// The root column name, exactly as the caller spelled it.
    name: SmolStr,
    /// What reaching inside that column's value takes, in order.
    path: Arc<[Accessor]>,
}

impl Column {
    /// Name a root column with no accessors.
    #[must_use]
    pub fn new(name: impl Into<SmolStr>) -> Self {
        Self {
            name: name.into(),
            path: Arc::from([]),
        }
    }

    /// Name a root column and the accessors that reach inside it.
    #[must_use]
    pub fn with_path(name: impl Into<SmolStr>, path: impl IntoIterator<Item = Accessor>) -> Self {
        Self {
            name: name.into(),
            path: path.into_iter().collect(),
        }
    }

    /// Borrow the root column name.
    #[must_use]
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Borrow the accessor chain, which is empty for a bare column.
    #[must_use]
    #[inline]
    pub fn path(&self) -> &[Accessor] {
        &self.path
    }

    /// Return this column with one more accessor on the end.
    #[must_use]
    pub fn push(&self, accessor: Accessor) -> Self {
        let mut path = Vec::with_capacity(self.path.len() + 1);
        path.extend_from_slice(&self.path);
        path.push(accessor);
        Self {
            name: self.name.clone(),
            path: Arc::from(path),
        }
    }
}

impl fmt::Display for Column {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_identifier(formatter, &self.name)?;
        for accessor in self.path.iter() {
            write!(formatter, "{accessor}")?;
        }
        Ok(())
    }
}

/// The one expression value: a computation over a row, free of any schema.
///
/// Nesting is shared through [`Arc`], so cloning a large predicate bumps
/// reference counts rather than copying a tree, and an operand list with no
/// elements carries no allocation.
///
/// # Two name collisions worth knowing before you write one
///
/// - `==` is **structural** equality between two expressions, which is what
///   [`Eq`] and [`Hash`] mean here and what the optimizer's hash-consing needs.
///   [`Expr::eq`] *builds* an equality predicate. `a == b` asks "are these the
///   same expression"; `a.eq(b)` produces the expression `a = b`.
/// - [`From<&str>`](Expr::from) makes a **string literal**, exactly as
///   [`Value::from`] does. [`FromStr`] *parses*. `Expr::from("a > 1")` is the
///   five-character string; `"a > 1".parse::<Expr>()?` is the predicate.
///
/// ```
/// use yggdryl::expressions::{Expr, col, lit};
///
/// # fn main() -> yggdryl::Result<()> {
/// let built = col("price").gt(lit(10)).and(col("venue").eq(lit("XNAS")));
/// let parsed: Expr = "price > 10 AND venue = 'XNAS'".parse()?;
/// assert_eq!(built, parsed);
/// assert_eq!(Expr::from("a > 1"), Expr::literal(yggdryl::Value::from("a > 1")));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Expr {
    /// A column, possibly reached into.
    Column(Column),
    /// A constant. `NULL` is [`Value::Null`].
    Literal(Value),
    /// A schema-directed cast, the crate's one cast reached from a predicate.
    Cast {
        /// What is being converted.
        expr: Arc<Expr>,
        /// The type it is converted to.
        data_type: DataType,
        /// Whether a value the target cannot hold becomes null rather than an
        /// error, matching `IORecordOptions::safe`.
        safe: bool,
    },
    /// A comparison of two expressions.
    Compare {
        /// Which comparison.
        op: CompareOp,
        /// The left operand.
        left: Arc<Expr>,
        /// The right operand.
        right: Arc<Expr>,
    },
    /// Conjunction of every operand. An empty conjunction is `TRUE`.
    And(Arc<[Expr]>),
    /// Disjunction of every operand. An empty disjunction is `FALSE`.
    Or(Arc<[Expr]>),
    /// Three-valued negation: `NOT unknown` is unknown.
    Not(Arc<Expr>),
    /// `IS NULL` - one of the only two operators that answer true or false
    /// about a null rather than unknown.
    IsNull(Arc<Expr>),
    /// `IS NOT NULL`.
    IsNotNull(Arc<Expr>),
    /// `IN (...)` and `NOT IN (...)`.
    In {
        /// The value being looked up.
        expr: Arc<Expr>,
        /// The set it is looked up in.
        list: Arc<[Expr]>,
        /// Whether the membership is negated.
        negated: bool,
    },
    /// `BETWEEN low AND high`, which the optimizer lowers to two comparisons.
    Between {
        /// The value being bounded.
        expr: Arc<Expr>,
        /// The inclusive lower bound.
        low: Arc<Expr>,
        /// The inclusive upper bound.
        high: Arc<Expr>,
        /// Whether the bound is negated.
        negated: bool,
    },
    /// SQL `LIKE` / `ILIKE`, with `_` and `%` wildcards.
    Like {
        /// The text being matched.
        expr: Arc<Expr>,
        /// The pattern.
        pattern: Arc<Expr>,
        /// The character that escapes a wildcard, when the clause names one.
        escape: Option<char>,
        /// Whether the match is negated.
        negated: bool,
        /// Whether the match ignores ASCII case (`ILIKE`).
        case_insensitive: bool,
    },
    /// A literal prefix test - the one text predicate a statistics range can
    /// prune, which is why `LIKE 'x%'` folds to it.
    StartsWith {
        /// The text being tested.
        expr: Arc<Expr>,
        /// The prefix it must start with.
        prefix: SmolStr,
    },
    /// Arithmetic over two operands.
    Arithmetic {
        /// Which operator.
        op: ArithOp,
        /// The left operand.
        left: Arc<Expr>,
        /// The right operand.
        right: Arc<Expr>,
    },
    /// Arithmetic negation.
    Neg(Arc<Expr>),
    /// One of the closed set of scalar functions.
    Function {
        /// Which function.
        name: Function,
        /// Its arguments, in order.
        args: Arc<[Expr]>,
    },
    /// `CASE WHEN ... THEN ... ELSE ... END`, the one conditional.
    Case {
        /// The `WHEN`/`THEN` pairs, tried in order.
        branches: Arc<[(Expr, Expr)]>,
        /// The `ELSE` value; absent means null.
        otherwise: Option<Arc<Expr>>,
    },
    /// A name for the column an expression computes.
    Alias {
        /// What is being named.
        expr: Arc<Expr>,
        /// The name.
        name: SmolStr,
    },
}

/// Build a column reference. The free spelling of [`Expr::column`].
#[must_use]
pub fn col(name: impl Into<SmolStr>) -> Expr {
    Expr::column(name)
}

/// Build a literal. The free spelling of [`Expr::literal`].
#[must_use]
pub fn lit(value: impl Into<Value>) -> Expr {
    Expr::literal(value)
}

impl Expr {
    /// The always-true expression, which is what an empty conjunction means.
    #[must_use]
    pub fn always_true() -> Self {
        Self::Literal(Value::Bool(true))
    }

    /// The always-false expression, which is what an empty disjunction means.
    #[must_use]
    pub fn always_false() -> Self {
        Self::Literal(Value::Bool(false))
    }

    /// Name a root column.
    #[must_use]
    pub fn column(name: impl Into<SmolStr>) -> Self {
        Self::Column(Column::new(name))
    }

    /// Name a column and the accessors that reach inside its value.
    #[must_use]
    pub fn column_path(name: impl Into<SmolStr>, path: impl IntoIterator<Item = Accessor>) -> Self {
        Self::Column(Column::with_path(name, path))
    }

    /// Hold a constant.
    #[must_use]
    pub fn literal(value: impl Into<Value>) -> Self {
        Self::Literal(value.into())
    }

    /// Return whether this expression is a literal of any kind.
    #[must_use]
    #[inline]
    pub const fn is_literal(&self) -> bool {
        matches!(self, Self::Literal(_))
    }

    /// Borrow the constant this expression holds, if it holds one.
    #[must_use]
    #[inline]
    pub const fn as_literal(&self) -> Option<&Value> {
        match self {
            Self::Literal(value) => Some(value),
            _ => None,
        }
    }

    /// Borrow the column this expression names, if it names one.
    #[must_use]
    #[inline]
    pub const fn as_column(&self) -> Option<&Column> {
        match self {
            Self::Column(column) => Some(column),
            _ => None,
        }
    }

    /// Return whether this expression is the literal `TRUE`.
    #[must_use]
    pub fn is_always_true(&self) -> bool {
        matches!(self, Self::Literal(Value::Bool(true)))
            || matches!(self, Self::And(operands) if operands.is_empty())
    }

    /// Return whether this expression is the literal `FALSE`.
    #[must_use]
    pub fn is_always_false(&self) -> bool {
        matches!(self, Self::Literal(Value::Bool(false)))
            || matches!(self, Self::Or(operands) if operands.is_empty())
    }

    /// Reach inside this expression's value by struct child or map key name.
    #[must_use]
    pub fn child(self, name: impl Into<SmolStr>) -> Self {
        self.accessor(Accessor::Child(name.into()))
    }

    /// Reach inside this expression's value by map key.
    #[must_use]
    pub fn key(self, key: impl Into<Value>) -> Self {
        self.accessor(Accessor::Key(key.into()))
    }

    /// Reach inside this expression's value by position, 0-based.
    #[must_use]
    pub fn at(self, index: i64) -> Self {
        self.accessor(Accessor::Index(index))
    }

    /// Take a half-open range of this expression's items.
    #[must_use]
    pub fn slice(self, start: Option<i64>, end: Option<i64>) -> Self {
        self.accessor(Accessor::Range { start, end })
    }

    /// Extend a column path, refusing to accessor anything that is not one.
    ///
    /// An accessor is a *path* step, and a path belongs to a column: reaching
    /// inside `lower(x)` would mean materializing a value to index it, which
    /// this grammar deliberately has no way to spell. A non-column receiver
    /// therefore yields itself unchanged rather than inventing a node, and the
    /// parser refuses the same shape with a byte position.
    #[must_use]
    fn accessor(self, accessor: Accessor) -> Self {
        match self {
            Self::Column(column) => Self::Column(column.push(accessor)),
            other => other,
        }
    }

    /// Convert to another datatype, erroring on a value the target cannot hold.
    #[must_use]
    pub fn cast_to(self, data_type: DataType) -> Self {
        Self::Cast {
            expr: Arc::new(self),
            data_type,
            safe: false,
        }
    }

    /// Convert to another datatype, nulling a value the target cannot hold.
    #[must_use]
    pub fn try_cast_to(self, data_type: DataType) -> Self {
        Self::Cast {
            expr: Arc::new(self),
            data_type,
            safe: true,
        }
    }

    /// Name the column this expression computes.
    #[must_use]
    pub fn alias(self, name: impl Into<SmolStr>) -> Self {
        Self::Alias {
            expr: Arc::new(self),
            name: name.into(),
        }
    }

    /// Build a comparison against another expression.
    #[must_use]
    pub fn compare(self, op: CompareOp, other: impl Into<Self>) -> Self {
        Self::Compare {
            op,
            left: Arc::new(self),
            right: Arc::new(other.into()),
        }
    }

    /// Build `self = other`.
    ///
    /// This deliberately shadows [`PartialEq::eq`] for an owned receiver: a
    /// predicate reads better as `col("a").eq(3)` than as any spelling that
    /// avoids the collision, and structural equality stays available as `==`.
    /// The trade is documented on the type.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn eq(self, other: impl Into<Self>) -> Self {
        self.compare(CompareOp::Eq, other)
    }

    /// Build `self <> other`. Shadows [`PartialEq::ne`]; see [`Self::eq`].
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn ne(self, other: impl Into<Self>) -> Self {
        self.compare(CompareOp::NotEq, other)
    }

    /// Build `self < other`. Shadows [`PartialOrd::lt`]; see [`Self::eq`].
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn lt(self, other: impl Into<Self>) -> Self {
        self.compare(CompareOp::Lt, other)
    }

    /// Build `self <= other`. Shadows [`PartialOrd::le`]; see [`Self::eq`].
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn le(self, other: impl Into<Self>) -> Self {
        self.compare(CompareOp::LtEq, other)
    }

    /// Build `self > other`. Shadows [`PartialOrd::gt`]; see [`Self::eq`].
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn gt(self, other: impl Into<Self>) -> Self {
        self.compare(CompareOp::Gt, other)
    }

    /// Build `self >= other`. Shadows [`PartialOrd::ge`]; see [`Self::eq`].
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn ge(self, other: impl Into<Self>) -> Self {
        self.compare(CompareOp::GtEq, other)
    }

    /// Build `self AND other`, flattening an operand that is already a
    /// conjunction so a chain of `and` calls is one n-ary node.
    #[must_use]
    pub fn and(self, other: impl Into<Self>) -> Self {
        Self::all([self, other.into()])
    }

    /// Build `self OR other`, flattening the same way [`Self::and`] does.
    #[must_use]
    pub fn or(self, other: impl Into<Self>) -> Self {
        Self::any([self, other.into()])
    }

    /// Build `NOT self`. Shadows [`std::ops::Not::not`]; `!expr` builds the
    /// same node.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> Self {
        Self::Not(Arc::new(self))
    }

    /// Conjoin every expression, flattening nested conjunctions.
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

    /// Build `self IS NULL`.
    #[must_use]
    pub fn is_null(self) -> Self {
        Self::IsNull(Arc::new(self))
    }

    /// Build `self IS NOT NULL`.
    #[must_use]
    pub fn is_not_null(self) -> Self {
        Self::IsNotNull(Arc::new(self))
    }

    /// Build `self IN (...)`.
    #[must_use]
    pub fn is_in(self, list: impl IntoIterator<Item = Self>) -> Self {
        Self::In {
            expr: Arc::new(self),
            list: list.into_iter().collect(),
            negated: false,
        }
    }

    /// Build `self NOT IN (...)`.
    #[must_use]
    pub fn is_not_in(self, list: impl IntoIterator<Item = Self>) -> Self {
        Self::In {
            expr: Arc::new(self),
            list: list.into_iter().collect(),
            negated: true,
        }
    }

    /// Build `self BETWEEN low AND high`.
    #[must_use]
    pub fn between(self, low: impl Into<Self>, high: impl Into<Self>) -> Self {
        Self::Between {
            expr: Arc::new(self),
            low: Arc::new(low.into()),
            high: Arc::new(high.into()),
            negated: false,
        }
    }

    /// Build `self LIKE pattern`.
    #[must_use]
    pub fn like(self, pattern: impl Into<Self>) -> Self {
        Self::Like {
            expr: Arc::new(self),
            pattern: Arc::new(pattern.into()),
            escape: None,
            negated: false,
            case_insensitive: false,
        }
    }

    /// Build `self ILIKE pattern`.
    #[must_use]
    pub fn ilike(self, pattern: impl Into<Self>) -> Self {
        Self::Like {
            expr: Arc::new(self),
            pattern: Arc::new(pattern.into()),
            escape: None,
            negated: false,
            case_insensitive: true,
        }
    }

    /// Build a literal prefix test.
    #[must_use]
    pub fn starts_with(self, prefix: impl Into<SmolStr>) -> Self {
        Self::StartsWith {
            expr: Arc::new(self),
            prefix: prefix.into(),
        }
    }

    /// Build an arithmetic node.
    #[must_use]
    pub fn arithmetic(self, op: ArithOp, other: impl Into<Self>) -> Self {
        Self::Arithmetic {
            op,
            left: Arc::new(self),
            right: Arc::new(other.into()),
        }
    }

    /// Build a call to one of the closed set of functions.
    #[must_use]
    pub fn call(name: Function, args: impl IntoIterator<Item = Self>) -> Self {
        Self::Function {
            name,
            args: args.into_iter().collect(),
        }
    }

    /// Build a `CASE` expression from its branches and optional `ELSE`.
    #[must_use]
    pub fn case(branches: impl IntoIterator<Item = (Self, Self)>, otherwise: Option<Self>) -> Self {
        Self::Case {
            branches: branches.into_iter().collect(),
            otherwise: otherwise.map(Arc::new),
        }
    }

    /// Every column path this expression reads, deduplicated in first-seen
    /// order.
    ///
    /// This is what drives projection pushdown: a read decodes exactly the
    /// columns the filter and the selection actually name, and no more.
    #[must_use]
    pub fn columns(&self) -> Vec<String> {
        let mut seen = Vec::new();
        self.collect_columns(&mut seen);
        let mut names = Vec::with_capacity(seen.len());
        for column in seen {
            let name = column.name().to_owned();
            if !names
                .iter()
                .any(|held: &String| held.eq_ignore_ascii_case(&name))
            {
                names.push(name);
            }
        }
        names
    }

    /// Every column reference this expression holds, in first-seen order.
    #[must_use]
    pub fn column_refs(&self) -> Vec<Column> {
        let mut seen = Vec::new();
        self.collect_columns(&mut seen);
        seen
    }

    /// Walk every child of this node in evaluation order.
    ///
    /// One traversal serves `columns`, depth checking, and every rule that
    /// needs to look below a node, so a new variant is wired in exactly once.
    fn for_each_child<'node>(&'node self, mut visit: impl FnMut(&'node Self)) {
        match self {
            Self::Column(_) | Self::Literal(_) => {}
            Self::Cast { expr, .. }
            | Self::Not(expr)
            | Self::IsNull(expr)
            | Self::IsNotNull(expr)
            | Self::Neg(expr)
            | Self::Alias { expr, .. }
            | Self::StartsWith { expr, .. } => visit(expr),
            Self::Compare { left, right, .. } | Self::Arithmetic { left, right, .. } => {
                visit(left);
                visit(right);
            }
            Self::And(operands) | Self::Or(operands) => operands.iter().for_each(visit),
            Self::In { expr, list, .. } => {
                visit(expr);
                list.iter().for_each(visit);
            }
            Self::Between {
                expr, low, high, ..
            } => {
                visit(expr);
                visit(low);
                visit(high);
            }
            Self::Like { expr, pattern, .. } => {
                visit(expr);
                visit(pattern);
            }
            Self::Function { args, .. } => args.iter().for_each(visit),
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
        }
    }

    /// Collect column references depth-first, in evaluation order.
    fn collect_columns(&self, into: &mut Vec<Column>) {
        if let Self::Column(column) = self {
            if !into.contains(column) {
                into.push(column.clone());
            }
            return;
        }
        self.for_each_child(|child| child.collect_columns(into));
    }

    /// How deep this expression nests, counting itself as one level.
    ///
    /// The walk is iterative rather than recursive so measuring the depth of a
    /// deliberately deep tree cannot itself overflow the stack - the tree it
    /// measures was built by a bounded parser, but a caller can build one by
    /// hand and is entitled to a typed refusal rather than an abort.
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

    /// Refuse an expression that nests past [`RECURSION_LIMIT`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the limit and the depth reached.
    pub fn check_depth(&self) -> Result<()> {
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
        Ok(())
    }

    /// The top-level `AND` operands, flattened.
    ///
    /// Pruning is per conjunct and a residual is the conjuncts a partition
    /// tuple did not settle, so this is the shape every layer of a read
    /// consumes.
    #[must_use]
    pub fn conjuncts(&self) -> Vec<Self> {
        match self {
            Self::And(operands) => operands.iter().flat_map(Self::conjuncts).collect(),
            other if other.is_always_true() => Vec::new(),
            other => vec![other.clone()],
        }
    }

    /// Simplify without a schema, through the one optimizer.
    ///
    /// This is a *view* on the engine of [`optimize`](crate::expressions), not
    /// a second implementation: it builds the plan graph, runs the rules that
    /// need no schema, and reads an [`Expr`] back out. Every rewrite is
    /// semantics-preserving under three-valued logic, and a rule that cannot
    /// prove itself declines rather than guessing.
    ///
    /// ```
    /// use yggdryl::expressions::Expr;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let wide: Expr = "a = 1 OR a = 2 OR a = 3".parse()?;
    /// assert_eq!(wide.simplify().to_string(), "a IN (1, 2, 3)");
    ///
    /// // `a = a` is unknown when `a` is null, so it is never folded to TRUE.
    /// let reflexive: Expr = "a = a".parse()?;
    /// assert_eq!(reflexive.simplify().to_string(), "a = a");
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn simplify(&self) -> Self {
        let mut plan = Plan::new();
        let root = plan.insert_expr(self);
        let root = optimize::run(&mut plan, root, None);
        plan.to_expr(root)
    }

    /// Bind this expression against a struct root, producing the one plan
    /// every evaluator reads.
    ///
    /// Binding happens **once per read** - never per batch, never per row.
    /// It resolves each name to a slot chain, computes each node's result
    /// datatype, folds every literal into the column's own type, and runs the
    /// optimizer with the schema in hand.
    ///
    /// # Errors
    ///
    /// Returns an error naming the columns the schema does have when this
    /// expression names one it does not, or naming both sides when a
    /// comparison has no common type.
    pub fn bind(&self, schema: &crate::Field) -> Result<Bound> {
        Bound::new(self, schema, bound::Strictness::STRICT)
    }

    /// Bind the way the `(column, value)` pair vocabulary binds.
    ///
    /// The folder route has always tolerated a filter column the rows do not
    /// carry and a value the column's type cannot read; that tolerance belongs
    /// to the *sugar*, not to the expression, so it is a binding mode rather
    /// than a branch inside the evaluator. A column the schema lacks drops out
    /// of the predicate, and a literal the type cannot hold folds the conjunct
    /// it appears in to `FALSE` - which is exactly "matches nothing".
    ///
    /// # Errors
    ///
    /// Returns an error for a failure tolerance cannot absorb, such as a
    /// comparison between two types with no common comparison type.
    pub fn bind_tolerant(&self, schema: &crate::Field) -> Result<Bound> {
        Bound::new(self, schema, bound::Strictness::TOLERANT)
    }

    /// Bind the way a route with a *declared* schema binds.
    ///
    /// A column is a claim - naming one the schema does not declare is an
    /// error listing what it does declare - while a value is still text, so a
    /// literal the column's type cannot read makes the comparison match
    /// nothing rather than failing the scan. That is exactly the split a table
    /// format has always had, and it is a binding mode rather than a branch
    /// inside the evaluator.
    ///
    /// # Errors
    ///
    /// Returns an error naming the columns the schema does have when this
    /// expression names one it does not.
    pub fn bind_declared(&self, schema: &crate::Field) -> Result<Bound> {
        Bound::new(self, schema, bound::Strictness::DECLARED)
    }
}

impl Default for Selection {
    fn default() -> Self {
        Self::everything()
    }
}

/// Anything that names a predicate: an expression, or text this grammar reads.
///
/// This exists rather than a bare [`TryInto<Expr>`] bound for one reason, and
/// it is the most confusable pair in this API: [`From<&str>`](Expr::from)
/// builds a **string literal**, exactly as [`Value::from`] does, so a
/// `TryInto<Expr>` bound would have accepted `"venue = \'XNAS\'"` and quietly
/// made it the seventeen-character string. A filter is a predicate, so text in
/// filter position is *parsed*, and the trait is what says so at the type
/// level.
pub trait IntoFilter {
    /// Read this as the predicate it names.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with a byte offset when text is not an
    /// expression this grammar reads.
    fn into_filter(self) -> Result<Expr>;
}

impl IntoFilter for Expr {
    fn into_filter(self) -> Result<Self> {
        Ok(self)
    }
}

impl IntoFilter for &Expr {
    fn into_filter(self) -> Result<Expr> {
        Ok(self.clone())
    }
}

impl IntoFilter for &str {
    fn into_filter(self) -> Result<Expr> {
        self.parse()
    }
}

impl IntoFilter for String {
    fn into_filter(self) -> Result<Expr> {
        self.parse()
    }
}

impl IntoFilter for &String {
    fn into_filter(self) -> Result<Expr> {
        self.parse()
    }
}

/// Anything that names a projection: a selection, or text this grammar reads.
///
/// The same reasoning as [`IntoFilter`], for the same reason.
pub trait IntoProjection {
    /// Read this as the projection it names.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] with a byte offset when text is not a
    /// projection this grammar reads.
    fn into_projection(self) -> Result<Selection>;
}

impl IntoProjection for Selection {
    fn into_projection(self) -> Result<Self> {
        Ok(self)
    }
}

impl IntoProjection for &Selection {
    fn into_projection(self) -> Result<Selection> {
        Ok(self.clone())
    }
}

impl IntoProjection for &str {
    fn into_projection(self) -> Result<Selection> {
        self.parse()
    }
}

impl IntoProjection for String {
    fn into_projection(self) -> Result<Selection> {
        self.parse()
    }
}

/// Read one text as the datatype names it, or answer that it cannot be.
///
/// This is what a `column=value` directory name and a `(column, value)` pair
/// both need: the layout writes text, and a comparison has to happen in the
/// column's own type. It is the one entry point for that, so a folder, a table
/// format, and the pair sugar cannot read the same directory differently.
#[must_use]
pub fn coerce_text(text: &str, data_type: &DataType) -> Option<Value> {
    coerce_value(&Value::from(text), data_type)
}

/// Read one value as the datatype names it, or answer that it cannot be.
///
/// The value-level sibling of [`ArrowCast`](crate::ArrowCast): the row
/// evaluator, the statistics evaluator, and literal folding all run in builds
/// with no Arrow at all, so the conversion they need cannot be an array cast.
/// It is public because every statistics source outside this module needs it -
/// a Parquet footer, an Iceberg manifest, and a partition directory each hold
/// bounds in their own spelling and must present them in the column\'s type.
#[must_use]
pub fn coerce_value(value: &Value, data_type: &DataType) -> Option<Value> {
    bound::coerce_value(value, data_type)
}

/// Render a value the way this grammar spells a literal.
///
/// Kept beside [`Expr`]'s [`Display`](fmt::Display) rather than on [`Value`]
/// because it is SQL's spelling of a value, not the value's own: a string
/// doubles its quote, a temporal wears its type keyword, and a decimal keeps
/// the scale it was written with.
pub(crate) struct Literal<'value>(pub(crate) &'value Value);

impl fmt::Display for Literal<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Value::Null => formatter.write_str("NULL"),
            Value::Bool(true) => formatter.write_str("TRUE"),
            Value::Bool(false) => formatter.write_str("FALSE"),
            Value::I8(inner) => write!(formatter, "{inner}"),
            Value::I16(inner) => write!(formatter, "{inner}"),
            Value::I32(inner) => write!(formatter, "{inner}"),
            Value::I64(inner) => write!(formatter, "{inner}"),
            Value::I128(inner) => write!(formatter, "{inner}"),
            Value::U8(inner) => write!(formatter, "{inner}"),
            Value::U16(inner) => write!(formatter, "{inner}"),
            Value::U32(inner) => write!(formatter, "{inner}"),
            Value::U64(inner) => write!(formatter, "{inner}"),
            Value::U128(inner) => write!(formatter, "{inner}"),
            Value::F32(inner) => write_float(formatter, f64::from(inner.as_f32())),
            Value::F64(inner) => write_float(formatter, inner.as_f64()),
            Value::Decimal(unscaled, scale) => write_decimal(formatter, *unscaled, *scale),
            Value::String(text) => write_quoted(formatter, text, '\''),
            Value::Bytes(bytes) => {
                formatter.write_str("X'")?;
                for byte in bytes.iter() {
                    write!(formatter, "{byte:02X}")?;
                }
                formatter.write_str("'")
            }
            Value::Date(days) => match crate::generic::iso::format_date(*days) {
                Some(text) => write!(formatter, "DATE '{text}'"),
                None => write!(formatter, "CAST({days} AS date32)"),
            },
            Value::Time(count, unit) => match crate::generic::iso::format_time(*count, *unit) {
                Some(text) => write!(formatter, "TIME '{text}'"),
                None => write!(formatter, "CAST({count} AS time64(ns))"),
            },
            Value::Timestamp(count, unit, zone) => {
                match crate::generic::iso::format_timestamp(*count, *unit, zone) {
                    Some(text) => write!(formatter, "TIMESTAMP '{text}'"),
                    None => write!(formatter, "CAST({count} AS timestamp(ns, 'UTC'))"),
                }
            }
            Value::DateTime(count, unit) => {
                match crate::generic::iso::format_datetime(*count, *unit) {
                    Some(text) => write!(formatter, "TIMESTAMP '{text}'"),
                    None => write!(formatter, "CAST({count} AS timestamp(ns))"),
                }
            }
            Value::Duration(count, unit) => {
                match crate::generic::iso::format_duration(*count, *unit) {
                    Some(text) => write!(formatter, "INTERVAL '{text}'"),
                    None => write!(formatter, "CAST({count} AS duration(ns))"),
                }
            }
            // A sequence is what a range accessor produces and what a folded
            // list holds; the grammar reads it back as a parenthesized list.
            Value::Sequence(items) => {
                formatter.write_str("(")?;
                for (position, item) in items.iter().enumerate() {
                    if position > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{}", Literal(item))?;
                }
                formatter.write_str(")")
            }
            // A mapping and a record have no literal spelling in SQL, so they
            // are rendered as the diagnostic shape they are - a caller reaches
            // inside one with an accessor rather than writing one down.
            Value::Mapping(entries) => {
                formatter.write_str("{")?;
                for (position, (key, value)) in entries.iter().enumerate() {
                    if position > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{}: {}", Literal(key), Literal(value))?;
                }
                formatter.write_str("}")
            }
            Value::Record(_, values) => {
                formatter.write_str("{")?;
                for (position, value) in values.iter().enumerate() {
                    if position > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{}", Literal(value))?;
                }
                formatter.write_str("}")
            }
        }
    }
}

/// Write a float so the grammar reads it back as a float.
///
/// SQL has no float literal syntax of its own - a fractional literal is exact
/// numeric - so an exponent is what distinguishes the two here, and the two
/// non-finite readings get the keywords the parser reserves for them.
fn write_float(formatter: &mut fmt::Formatter<'_>, number: f64) -> fmt::Result {
    if number.is_nan() {
        return formatter.write_str("NAN");
    }
    if number.is_infinite() {
        return formatter.write_str(if number > 0.0 {
            "INFINITY"
        } else {
            "-INFINITY"
        });
    }
    write!(formatter, "{number:e}")
}

/// Write an exact decimal with the scale it was built with.
fn write_decimal(formatter: &mut fmt::Formatter<'_>, unscaled: i128, scale: i8) -> fmt::Result {
    if scale <= 0 {
        // A negative scale multiplies, and the grammar has no spelling for a
        // trailing implied zero run, so the value is written out in full.
        let factor = 10_i128
            .checked_pow(u32::try_from(-i32::from(scale)).unwrap_or(0))
            .unwrap_or(1);
        return write!(formatter, "{}", unscaled.saturating_mul(factor));
    }
    let digits = usize::try_from(scale).unwrap_or(0);
    let negative = unscaled < 0;
    let magnitude = unscaled.unsigned_abs().to_string();
    let padded = if magnitude.len() <= digits {
        format!("{}{magnitude}", "0".repeat(digits - magnitude.len() + 1))
    } else {
        magnitude
    };
    let split = padded.len() - digits;
    if negative {
        formatter.write_str("-")?;
    }
    write!(formatter, "{}.{}", &padded[..split], &padded[split..])
}

/// Write `text` inside `delimiter`, doubling the delimiter to embed it.
fn write_quoted(formatter: &mut fmt::Formatter<'_>, text: &str, delimiter: char) -> fmt::Result {
    formatter.write_char_(delimiter)?;
    for character in text.chars() {
        if character == delimiter {
            formatter.write_char_(delimiter)?;
        }
        formatter.write_char_(character)?;
    }
    formatter.write_char_(delimiter)
}

/// A `write_char` that does not need `fmt::Write` in scope at every call site.
trait WriteChar {
    /// Write one character.
    fn write_char_(&mut self, character: char) -> fmt::Result;
}

impl WriteChar for fmt::Formatter<'_> {
    fn write_char_(&mut self, character: char) -> fmt::Result {
        use fmt::Write;
        self.write_char(character)
    }
}

/// Return whether a name may be written without an encapsulator.
///
/// The bar is deliberately conservative: an identifier is emitted bare only
/// when it matches `[A-Za-z_][A-Za-z0-9_]*` and is not a reserved word, so the
/// canonical spelling of a name is stable no matter what the grammar gains.
#[must_use]
pub(crate) fn is_bare_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    if characters.any(|character| !(character.is_ascii_alphanumeric() || character == '_')) {
        return false;
    }
    !parser::is_reserved_word(name)
}

/// Write an identifier, quoting it only when it needs quoting.
///
/// The three input encapsulators of the grammar collapse to the one ANSI
/// spelling here: a grammar may accept `"x"`, `` `x` ``, and `[x]` on the way
/// in and still have exactly one canonical form on the way out.
pub(crate) fn write_identifier(formatter: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    if is_bare_identifier(name) {
        formatter.write_str(name)
    } else {
        write_quoted(formatter, name, '"')
    }
}

/// The binding strength of a node, loosest first, for minimal parentheses.
///
/// `Display` emits a parenthesis exactly when a child binds looser than its
/// parent, which is what makes the canonical spelling both minimal and
/// re-parseable to the same tree.
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
enum Precedence {
    Or,
    And,
    Not,
    Comparison,
    Additive,
    Multiplicative,
    Unary,
    Primary,
}

impl Expr {
    /// How tightly this node binds, for parenthesizing its children.
    const fn precedence(&self) -> Precedence {
        match self {
            Self::Or(_) => Precedence::Or,
            Self::And(_) => Precedence::And,
            Self::Not(_) => Precedence::Not,
            Self::Compare { .. }
            | Self::In { .. }
            | Self::Between { .. }
            | Self::Like { .. }
            | Self::IsNull(_)
            | Self::IsNotNull(_)
            | Self::StartsWith { .. } => Precedence::Comparison,
            Self::Arithmetic { op, .. } => match op {
                ArithOp::Add | ArithOp::Sub => Precedence::Additive,
                ArithOp::Mul | ArithOp::Div | ArithOp::Mod => Precedence::Multiplicative,
            },
            Self::Neg(_) => Precedence::Unary,
            // An alias is written at the loosest level so `a + b AS c` needs
            // no parentheses, and a selection is where one ever appears.
            Self::Alias { .. } => Precedence::Or,
            Self::Column(_)
            | Self::Literal(_)
            | Self::Cast { .. }
            | Self::Function { .. }
            | Self::Case { .. } => Precedence::Primary,
        }
    }
}

/// Write `child` inside parentheses when it binds looser than `parent`.
fn write_operand(
    formatter: &mut fmt::Formatter<'_>,
    child: &Expr,
    parent: Precedence,
) -> fmt::Result {
    if child.precedence() < parent {
        write!(formatter, "({child})")
    } else {
        write!(formatter, "{child}")
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let here = self.precedence();
        match self {
            Self::Column(column) => write!(formatter, "{column}"),
            Self::Literal(value) => write!(formatter, "{}", Literal(value)),
            Self::Cast {
                expr,
                data_type,
                safe,
            } => {
                let verb = if *safe { "TRY_CAST" } else { "CAST" };
                write!(formatter, "{verb}({expr} AS {data_type})")
            }
            Self::Compare { op, left, right } => {
                write_operand(formatter, left, Precedence::Additive)?;
                write!(formatter, " {op} ")?;
                write_operand(formatter, right, Precedence::Additive)
            }
            Self::And(operands) => {
                if operands.is_empty() {
                    return formatter.write_str("TRUE");
                }
                for (position, operand) in operands.iter().enumerate() {
                    if position > 0 {
                        formatter.write_str(" AND ")?;
                    }
                    write_operand(formatter, operand, here)?;
                }
                Ok(())
            }
            Self::Or(operands) => {
                if operands.is_empty() {
                    return formatter.write_str("FALSE");
                }
                for (position, operand) in operands.iter().enumerate() {
                    if position > 0 {
                        formatter.write_str(" OR ")?;
                    }
                    write_operand(formatter, operand, here)?;
                }
                Ok(())
            }
            Self::Not(expr) => {
                formatter.write_str("NOT ")?;
                write_operand(formatter, expr, Precedence::Comparison)
            }
            Self::IsNull(expr) => {
                write_operand(formatter, expr, Precedence::Additive)?;
                formatter.write_str(" IS NULL")
            }
            Self::IsNotNull(expr) => {
                write_operand(formatter, expr, Precedence::Additive)?;
                formatter.write_str(" IS NOT NULL")
            }
            Self::In {
                expr,
                list,
                negated,
            } => {
                write_operand(formatter, expr, Precedence::Additive)?;
                formatter.write_str(if *negated { " NOT IN (" } else { " IN (" })?;
                for (position, item) in list.iter().enumerate() {
                    if position > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{item}")?;
                }
                formatter.write_str(")")
            }
            Self::Between {
                expr,
                low,
                high,
                negated,
            } => {
                write_operand(formatter, expr, Precedence::Additive)?;
                formatter.write_str(if *negated {
                    " NOT BETWEEN "
                } else {
                    " BETWEEN "
                })?;
                write_operand(formatter, low, Precedence::Additive)?;
                formatter.write_str(" AND ")?;
                write_operand(formatter, high, Precedence::Additive)
            }
            Self::Like {
                expr,
                pattern,
                escape,
                negated,
                case_insensitive,
            } => {
                write_operand(formatter, expr, Precedence::Additive)?;
                if *negated {
                    formatter.write_str(" NOT")?;
                }
                formatter.write_str(if *case_insensitive {
                    " ILIKE "
                } else {
                    " LIKE "
                })?;
                write_operand(formatter, pattern, Precedence::Additive)?;
                if let Some(escape) = escape {
                    formatter.write_str(" ESCAPE ")?;
                    write_quoted(formatter, &escape.to_string(), '\'')?;
                }
                Ok(())
            }
            // A prefix test has no SQL keyword of its own, so it is spelled as
            // the `LIKE` it folded from - which round-trips and folds again.
            Self::StartsWith { expr, prefix } => {
                write_operand(formatter, expr, Precedence::Additive)?;
                formatter.write_str(" LIKE ")?;
                write_quoted(formatter, &format!("{}%", escape_like(prefix)), '\'')
            }
            Self::Arithmetic { op, left, right } => {
                write_operand(formatter, left, here)?;
                write!(formatter, " {op} ")?;
                // The right operand of a subtraction or a division needs a
                // parenthesis at equal precedence, because the operators do
                // not associate: `a - (b - c)` is not `a - b - c`.
                write_operand(formatter, right, next_tighter(here))
            }
            Self::Neg(expr) => {
                formatter.write_str("-")?;
                write_operand(formatter, expr, Precedence::Unary)
            }
            Self::Function { name, args } => {
                write!(formatter, "{name}(")?;
                for (position, arg) in args.iter().enumerate() {
                    if position > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{arg}")?;
                }
                formatter.write_str(")")
            }
            Self::Case {
                branches,
                otherwise,
            } => {
                formatter.write_str("CASE")?;
                for (when, then) in branches.iter() {
                    write!(formatter, " WHEN {when} THEN {then}")?;
                }
                if let Some(otherwise) = otherwise {
                    write!(formatter, " ELSE {otherwise}")?;
                }
                formatter.write_str(" END")
            }
            Self::Alias { expr, name } => {
                write!(formatter, "{expr} AS ")?;
                write_identifier(formatter, name)
            }
        }
    }
}

/// The precedence one step tighter, for non-associative right operands.
const fn next_tighter(precedence: Precedence) -> Precedence {
    match precedence {
        Precedence::Or => Precedence::And,
        Precedence::And => Precedence::Not,
        Precedence::Not => Precedence::Comparison,
        Precedence::Comparison => Precedence::Additive,
        Precedence::Additive => Precedence::Multiplicative,
        Precedence::Multiplicative | Precedence::Unary => Precedence::Unary,
        Precedence::Primary => Precedence::Primary,
    }
}

/// Escape the `LIKE` wildcards in a literal prefix.
///
/// A prefix folded out of `LIKE 'a_b%'` holds an underscore that was escaped;
/// writing it back unescaped would change what the pattern means, so the
/// backslash escape the parser accepts is put back with it.
fn escape_like(prefix: &str) -> String {
    let mut escaped = String::with_capacity(prefix.len());
    for character in prefix.chars() {
        if matches!(character, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

impl FromStr for Expr {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        parser::parse_expression(text)
    }
}

/// A string becomes a **string literal**, not a parsed predicate.
///
/// This matches [`Value::from`] exactly, and it is the one confusable pair in
/// this API: use [`FromStr`] to parse and [`Expr::column`] to name a column.
impl From<&str> for Expr {
    fn from(text: &str) -> Self {
        Self::Literal(Value::from(text))
    }
}

impl From<String> for Expr {
    fn from(text: String) -> Self {
        Self::Literal(Value::from(text))
    }
}

impl From<Value> for Expr {
    fn from(value: Value) -> Self {
        Self::Literal(value)
    }
}

impl From<Column> for Expr {
    fn from(column: Column) -> Self {
        Self::Column(column)
    }
}

/// Build a literal from anything that is already a [`Value`].
macro_rules! literal_from {
    ($($native:ty),* $(,)?) => {
        $(
            impl From<$native> for Expr {
                fn from(value: $native) -> Self {
                    Self::Literal(Value::from(value))
                }
            }
        )*
    };
}

literal_from!(
    bool, i8, i16, i32, i64, i128, u8, u16, u32, u64, u128, f32, f64
);

impl std::ops::BitAnd for Expr {
    type Output = Self;

    fn bitand(self, other: Self) -> Self {
        self.and(other)
    }
}

impl std::ops::BitOr for Expr {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        self.or(other)
    }
}

impl std::ops::Not for Expr {
    type Output = Self;

    fn not(self) -> Self {
        Self::Not(Arc::new(self))
    }
}

/// Implement one arithmetic operator over expressions and over literals.
macro_rules! arithmetic_operator {
    ($trait:ident, $method:ident, $op:ident) => {
        impl<Rhs: Into<Expr>> std::ops::$trait<Rhs> for Expr {
            type Output = Self;

            fn $method(self, other: Rhs) -> Self {
                self.arithmetic(ArithOp::$op, other)
            }
        }
    };
}

arithmetic_operator!(Add, add, Add);
arithmetic_operator!(Sub, sub, Sub);
arithmetic_operator!(Mul, mul, Mul);
arithmetic_operator!(Div, div, Div);
arithmetic_operator!(Rem, rem, Mod);

impl std::ops::Neg for Expr {
    type Output = Self;

    fn neg(self) -> Self {
        Self::Neg(Arc::new(self))
    }
}

/// A total order over expressions, consistent with structural equality.
///
/// [`DataType`] is `Eq + Hash` but not `Ord`, so the one place an ordering
/// needs it - a [`Expr::Cast`] node - compares canonical datatype text. That
/// text round-trips through the type grammar, so text equality is datatype
/// equality and the order stays consistent with `==`; it allocates only when
/// two casts are compared at the same position, which is why it is here rather
/// than in a hot path.
impl Ord for Expr {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        let rank = variant_rank(self).cmp(&variant_rank(other));
        if rank != Ordering::Equal {
            return rank;
        }
        match (self, other) {
            (Self::Column(left), Self::Column(right)) => left.cmp(right),
            (Self::Literal(left), Self::Literal(right)) => left.cmp(right),
            (
                Self::Cast {
                    expr: left,
                    data_type: left_type,
                    safe: left_safe,
                },
                Self::Cast {
                    expr: right,
                    data_type: right_type,
                    safe: right_safe,
                },
            ) => left
                .cmp(right)
                .then_with(|| left_type.to_string().cmp(&right_type.to_string()))
                .then_with(|| left_safe.cmp(right_safe)),
            (
                Self::Compare {
                    op: left_op,
                    left,
                    right,
                },
                Self::Compare {
                    op: right_op,
                    left: other_left,
                    right: other_right,
                },
            ) => left_op
                .cmp(right_op)
                .then_with(|| left.cmp(other_left))
                .then_with(|| right.cmp(other_right)),
            (Self::And(left), Self::And(right)) | (Self::Or(left), Self::Or(right)) => {
                left.iter().cmp(right.iter())
            }
            (Self::Not(left), Self::Not(right))
            | (Self::IsNull(left), Self::IsNull(right))
            | (Self::IsNotNull(left), Self::IsNotNull(right))
            | (Self::Neg(left), Self::Neg(right)) => left.cmp(right),
            (
                Self::In {
                    expr: left,
                    list: left_list,
                    negated: left_negated,
                },
                Self::In {
                    expr: right,
                    list: right_list,
                    negated: right_negated,
                },
            ) => left
                .cmp(right)
                .then_with(|| left_list.iter().cmp(right_list.iter()))
                .then_with(|| left_negated.cmp(right_negated)),
            (
                Self::Between {
                    expr: left,
                    low: left_low,
                    high: left_high,
                    negated: left_negated,
                },
                Self::Between {
                    expr: right,
                    low: right_low,
                    high: right_high,
                    negated: right_negated,
                },
            ) => left
                .cmp(right)
                .then_with(|| left_low.cmp(right_low))
                .then_with(|| left_high.cmp(right_high))
                .then_with(|| left_negated.cmp(right_negated)),
            (
                Self::Like {
                    expr: left,
                    pattern: left_pattern,
                    escape: left_escape,
                    negated: left_negated,
                    case_insensitive: left_case,
                },
                Self::Like {
                    expr: right,
                    pattern: right_pattern,
                    escape: right_escape,
                    negated: right_negated,
                    case_insensitive: right_case,
                },
            ) => left
                .cmp(right)
                .then_with(|| left_pattern.cmp(right_pattern))
                .then_with(|| left_escape.cmp(right_escape))
                .then_with(|| left_negated.cmp(right_negated))
                .then_with(|| left_case.cmp(right_case)),
            (
                Self::StartsWith {
                    expr: left,
                    prefix: left_prefix,
                },
                Self::StartsWith {
                    expr: right,
                    prefix: right_prefix,
                },
            ) => left.cmp(right).then_with(|| left_prefix.cmp(right_prefix)),
            (
                Self::Arithmetic {
                    op: left_op,
                    left,
                    right,
                },
                Self::Arithmetic {
                    op: right_op,
                    left: other_left,
                    right: other_right,
                },
            ) => left_op
                .cmp(right_op)
                .then_with(|| left.cmp(other_left))
                .then_with(|| right.cmp(other_right)),
            (
                Self::Function {
                    name: left_name,
                    args: left_args,
                },
                Self::Function {
                    name: right_name,
                    args: right_args,
                },
            ) => left_name
                .cmp(right_name)
                .then_with(|| left_args.iter().cmp(right_args.iter())),
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
            (
                Self::Alias {
                    expr: left,
                    name: left_name,
                },
                Self::Alias {
                    expr: right,
                    name: right_name,
                },
            ) => left.cmp(right).then_with(|| left_name.cmp(right_name)),
            // The rank comparison above already settled every mixed pair.
            _ => Ordering::Equal,
        }
    }
}

impl PartialOrd for Expr {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Order the variants themselves, so a mixed pair never compares equal.
const fn variant_rank(expr: &Expr) -> u8 {
    match expr {
        Expr::Literal(_) => 0,
        Expr::Column(_) => 1,
        Expr::Cast { .. } => 2,
        Expr::Compare { .. } => 3,
        Expr::And(_) => 4,
        Expr::Or(_) => 5,
        Expr::Not(_) => 6,
        Expr::IsNull(_) => 7,
        Expr::IsNotNull(_) => 8,
        Expr::In { .. } => 9,
        Expr::Between { .. } => 10,
        Expr::Like { .. } => 11,
        Expr::StartsWith { .. } => 12,
        Expr::Arithmetic { .. } => 13,
        Expr::Neg(_) => 14,
        Expr::Function { .. } => 15,
        Expr::Case { .. } => 16,
        Expr::Alias { .. } => 17,
    }
}
