//! One grammar for what a read publishes and which rows it keeps.
//!
//! An [`Expression`] is one clause of the statement a record read or write
//! runs: the [`Selector`] that says which columns come out, computed how and
//! typed as what, or the [`Filter`] that says which rows stay. Both are built
//! from one recursive tree, the [`Term`], and both have one canonical text
//! that re-parses to the same value - which is what lets an expression cross
//! a process boundary as a string, sit in record options, and evolve without
//! the options learning a new shape.
//!
//! ```text
//! select id, price * size as notional float64   -- Expression::Selector
//! where ccy = 'EUR' and price > 100             -- Expression::Filter
//! ```
//!
//! # A term is not a value
//!
//! [`Scalar`](crate::Scalar) is the codec's lossless value tree: structural,
//! serializable, and meaningful on its own. A [`Term`] is a computation whose
//! meaning depends on a schema. They meet at exactly two points, a
//! [`Term::Literal`] going in and evaluation producing a `Scalar` coming out,
//! and keeping them apart is what lets `Scalar` stay plain structural data
//! while a term carries schema-dependent meaning.
//!
//! # The four stages
//!
//! ```text
//! text ──parse──▶ Term ──simplify──▶ ──bind(schema)──▶ Bound ──▶ Scalar | ArrayRef | mask
//! ```
//!
//! 1. **Parse** ([`FromStr`](std::str::FromStr)) - one recursive grammar,
//!    re-entered by every nested construct, with byte-positioned errors.
//! 2. **Type** ([`Term::field`]) - the output [`Field`] resolved
//!    against a schema, recursively, and the only place output types are
//!    decided.
//! 3. **Bind** ([`Term::bind`]) - the tree is [simplified](Term::simplify),
//!    names become indices, literals are converted once to the type they are
//!    compared against, constants fold, and conjuncts are ordered
//!    cheapest-first. This happens **once per stream**, never per batch and
//!    never per row.
//! 4. **Apply** - the one [`Bound`] answers three ways: row at a time over
//!    [`Scalar`](crate::Scalar), vectorized over an Arrow `RecordBatch`, and
//!    three-valued over container statistics so a file, a manifest, or a
//!    directory is skipped without being read. A [`Selector`] and a
//!    [`Filter`] each apply to a field, a row, an Arrow array, an Arrow batch
//!    and an Arrow reader through the same bound tree, so every application
//!    of an expression lives here and nowhere else.
//!
//! The scalar tier compiles with no Arrow at all; only the vectorized tier is
//! behind the `arrow` feature.

mod attribute;
mod bind;
mod display;
mod eval;
mod explain;
mod filter;
mod literal;
mod parser;
mod path;
mod plan;
mod pushdown;
mod records;
mod selector;
mod serde;
mod term;
mod transform;
mod typing;
mod user;

#[cfg(feature = "arrow")]
mod arrow;

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Field, Result};

pub use attribute::{Attribute, Attributes, Cost, Handle, read_handle};
pub use bind::Bound;
pub use filter::{Filter, IntoFilter};
pub use literal::Literal;
pub use parser::needs_quoting;
pub use path::{FieldPath, FieldSegment};
pub use plan::{IntoPlan, Location, Ordering, Plan, Source, Target, Verb, Write};
pub use pushdown::{Bounds, ColumnBounds, Residual};
pub use records::Records;
pub use selector::{BoundSelector, IntoSelector, Projection, Selector};
pub use term::{IntoTerm, Term, col, lit};
pub(crate) use transform::{
    TRANSFORM_EXPRESSION_KEY, TRANSFORM_FUNCTION_KEY, TRANSFORM_KEYS, TRANSFORM_SOURCES_KEY,
    canonicalize_transform_expression, canonicalize_transform_function,
};
pub use user::{
    FunctionSignature, UserFunction, UserRef, lookup_function, register_function,
    registered_functions, unregister_function,
};

/// How deep a term may nest before the parser and every walk refuse.
///
/// A term that nests past it is a typed error naming the limit, never an
/// aborted process - which is what decides the number. This grammar's descent
/// spends far more stack per level than the schema grammar's, so the two cannot
/// share one: measured on a two-mebibyte thread, the size a test thread and a
/// modest spawned thread both get, an unoptimised build reaches 56 levels and
/// overflows before 64. Half of what it reaches is the limit, so the refusal
/// arrives with the stack still less than half spent and the promise above
/// holds on the smallest stack a caller is likely to run on.
pub const RECURSION_LIMIT: usize = 32;

/// How many nodes one term may hold.
///
/// Depth alone does not bound a term: a flat `IN` list of a million literals
/// is one level deep and still unbounded work. The node budget is checked
/// once, before any recursive walk, so a walk never has to check.
pub const NODE_LIMIT: usize = 100_000;

/// A comparison between two terms.
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
    /// Every comparison this grammar knows, in canonical spelling.
    pub const ALL: [Self; 8] = [
        Self::Eq,
        Self::NotEq,
        Self::Lt,
        Self::LtEq,
        Self::Gt,
        Self::GtEq,
        Self::IsDistinctFrom,
        Self::IsNotDistinctFrom,
    ];

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
/// [`Term::Negate`] is its own node instead.
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

/// The closed set of scalar functions this grammar spells, and the one door
/// out of it.
///
/// Closed deliberately: an open registry is a plugin system, and a plugin
/// system cannot promise that the scalar evaluator, the vectorized evaluator,
/// and the statistics evaluator agree about a function none of them knows. A
/// bare name outside this set is a parse error listing the vocabulary it is
/// not in. A *qualified* name - `namespace.name(...)` - is a
/// [user-defined function](UserFunction): registered with a signature rather than
/// known to the grammar, typed and called by the two row evaluators through
/// that signature, and opaque to the statistics evaluator.
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize, ::serde::Deserialize,
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
    /// [`FieldSegment`], for when the key is computed rather than written.
    ///
    /// Spelled `get` rather than `element_at` deliberately: several engines
    /// ship an `element_at` and they disagree about whether its index is
    /// 0-based or 1-based, so the familiar name cannot be used without
    /// inheriting an argument about what it means.
    Get,
    /// `slice(list, start [, end])` - the functional spelling of a
    /// [`FieldSegment::Range`], 0-based and half-open, a null bound meaning
    /// the list's own end.
    Slice,
    /// A registered [user-defined function](UserFunction), by qualified name.
    User(UserRef),
}

impl Function {
    /// Every function this grammar knows, in canonical spelling.
    pub const ALL: [Self; 19] = [
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
        Self::Slice,
    ];

    /// The canonical lowercase name of this function.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::User(reference) => reference.as_str(),
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
            Self::Slice => "slice",
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
            "slice" | "array_slice" => Self::Slice,
            _ => return None,
        })
    }

    /// The inclusive argument-count range this function accepts.
    #[must_use]
    pub fn arity(&self) -> (usize, usize) {
        match self {
            // A user function's arity is its registered signature's; one not
            // registered is refused where it is typed, so nothing is bounded
            // here.
            Self::User(reference) => lookup_function(reference)
                .map_or((0, usize::MAX), |function| function.signature().arity()),
            // The two variadics are the ones every dialect spells variadically.
            Self::Coalesce | Self::Concat => (1, usize::MAX),
            Self::Substring | Self::Slice => (2, 3),
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
    pub fn is_calendar(&self) -> bool {
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

/// One expression: a clause, a plan, or a sequence of them.
///
/// A `select` clause publishes columns and a `where` clause keeps rows; both
/// leave the rows where they found them. A [`Plan`] surrounds them with
/// sections - where rows come from, where they go, in what order and how
/// many - and a sequence applies expressions one after another. All four
/// spell one way and parse back:
///
/// ```
/// use yggdryl::Expression;
///
/// # fn main() -> yggdryl::Result<()> {
/// let selector: Expression = "select ccy, price * size as notional".parse()?;
/// let filter: Expression = "where ccy = 'EUR' and price > 100".parse()?;
/// let plan: Expression = "select ccy from 'file:///lake/trades.parquet' where ccy = 'EUR'".parse()?;
/// let sequence: Expression = "where ccy = 'EUR'; select ccy, price".parse()?;
/// assert!(selector.is_selector());
/// assert!(filter.is_filter());
/// assert!(plan.is_plan());
/// assert_eq!(sequence.steps().len(), 2);
/// assert_eq!(filter.to_string(), "where ccy = 'EUR' and price > 100");
/// assert_eq!(sequence.to_string().parse::<Expression>()?, sequence);
/// # Ok(())
/// # }
/// ```
#[derive(
    Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, ::serde::Serialize, ::serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Expression {
    /// The `select` clause: the columns published, computed and typed.
    Selector(Selector),
    /// The `where` clause: the rows kept.
    Filter(Filter),
    /// A plan: clauses with the sections that surround them.
    Plan(Box<Plan>),
    /// Expressions applied one after another, each to what the one before
    /// it produced.
    Sequence(Vec<Expression>),
}

impl Expression {
    /// The `select` clause a value spells.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the value is text that is not a selector.
    pub fn select(selector: impl IntoSelector) -> Result<Self> {
        Ok(Self::Selector(selector.into_selector()?))
    }

    /// The `where` clause a value spells.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the value is text that is not a filter.
    pub fn filter(filter: impl IntoFilter) -> Result<Self> {
        Ok(Self::Filter(filter.into_filter()?))
    }

    /// The plan a value spells.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the value is text that is not an
    /// expression, or an error when it is a sequence.
    pub fn plan(plan: impl IntoPlan) -> Result<Self> {
        Ok(plan.into_plan()?.into_expression())
    }

    /// Read an expression from the scalar that spells one.
    ///
    /// Text parses as whichever clause, plan or sequence it is; a sequence
    /// is one step per item; a boolean is the constant filter; null is the
    /// identity, `select *`.
    ///
    /// # Errors
    ///
    /// Returns a parse error for text that does not parse, and an error
    /// naming the scalar for any other shape.
    pub fn from_scalar(value: &crate::Scalar) -> Result<Self> {
        if let Some(text) = value.as_str() {
            return text.parse();
        }
        if value.is_null() {
            return Ok(Self::Selector(Selector::all()));
        }
        if matches!(value, crate::Scalar::Boolean(_)) {
            return Filter::from_scalar(value).map(Self::Filter);
        }
        if let Some(items) = value.as_sequence() {
            let steps = items
                .iter()
                .map(Self::from_scalar)
                .collect::<Result<Vec<_>>>()?;
            return Ok(Self::sequence(steps));
        }
        Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(
                "the text of an expression, a sequence of steps, a boolean, or null",
                format_args!("{value:?}"),
            ),
        })
    }

    /// The sequence of expressions, applied in order.
    ///
    /// One expression is itself; none is the empty sequence, which changes
    /// nothing.
    #[must_use]
    pub fn sequence(steps: impl IntoIterator<Item = Self>) -> Self {
        let mut steps: Vec<Self> = steps.into_iter().collect();
        if steps.len() == 1 {
            return steps.swap_remove(0);
        }
        Self::Sequence(steps)
    }

    /// Return a deterministic hash of the canonical expression text.
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_display(self)
    }

    /// Return whether this is a `select` clause.
    #[must_use]
    pub const fn is_selector(&self) -> bool {
        matches!(self, Self::Selector(_))
    }

    /// Return whether this is a `where` clause.
    #[must_use]
    pub const fn is_filter(&self) -> bool {
        matches!(self, Self::Filter(_))
    }

    /// Return whether this is a plan.
    #[must_use]
    pub const fn is_plan(&self) -> bool {
        matches!(self, Self::Plan(_))
    }

    /// Return whether this is a sequence.
    #[must_use]
    pub const fn is_sequence(&self) -> bool {
        matches!(self, Self::Sequence(_))
    }

    /// Borrow the selector, when this is a `select` clause.
    #[must_use]
    pub const fn as_selector(&self) -> Option<&Selector> {
        match self {
            Self::Selector(selector) => Some(selector),
            _ => None,
        }
    }

    /// Borrow the filter, when this is a `where` clause.
    #[must_use]
    pub const fn as_filter(&self) -> Option<&Filter> {
        match self {
            Self::Filter(filter) => Some(filter),
            _ => None,
        }
    }

    /// Borrow the plan, when this is one.
    #[must_use]
    pub const fn as_plan(&self) -> Option<&Plan> {
        match self {
            Self::Plan(plan) => Some(plan),
            _ => None,
        }
    }

    /// The steps of this expression: its own, or the one step it is.
    #[must_use]
    pub fn steps(&self) -> &[Self] {
        match self {
            Self::Sequence(steps) => steps,
            _ => std::slice::from_ref(self),
        }
    }

    /// Every top-level column this expression reads, in first-seen order.
    ///
    /// The source side: what an encoding has to decode for the expression
    /// to run. A sequence reads what its first step reads.
    #[must_use]
    pub fn columns(&self) -> Vec<String> {
        match self {
            Self::Selector(selector) => selector.columns(),
            Self::Filter(filter) => filter.columns(),
            Self::Plan(plan) => plan.columns(),
            Self::Sequence(steps) => steps.first().map(Self::columns).unwrap_or_default(),
        }
    }

    /// Every handle attribute this expression reads, in first-seen order.
    #[must_use]
    pub fn attributes(&self) -> Vec<Attribute> {
        match self {
            Self::Selector(selector) => selector.attributes(),
            Self::Filter(filter) => filter.attributes(),
            Self::Plan(plan) => plan.attributes(),
            Self::Sequence(steps) => {
                let mut attributes = Vec::new();
                for step in steps {
                    for attribute in step.attributes() {
                        if !attributes.contains(&attribute) {
                            attributes.push(attribute);
                        }
                    }
                }
                attributes
            }
        }
    }

    /// Every parameter this expression names, in first-seen order.
    #[must_use]
    pub fn parameters(&self) -> Vec<String> {
        match self {
            Self::Selector(selector) => selector.parameters(),
            Self::Filter(filter) => filter.parameters(),
            Self::Plan(plan) => plan.parameters(),
            Self::Sequence(steps) => {
                let mut parameters = Vec::new();
                for step in steps {
                    for parameter in step.parameters() {
                        if !parameters.contains(&parameter) {
                            parameters.push(parameter);
                        }
                    }
                }
                parameters
            }
        }
    }

    /// Return whether applying this expression changes nothing.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        match self {
            Self::Selector(selector) => selector.is_all(),
            Self::Filter(filter) => filter.is_always_true(),
            Self::Plan(plan) => plan.is_identity(),
            Self::Sequence(steps) => steps.iter().all(Self::is_identity),
        }
    }

    /// The same expression over simplified terms.
    #[must_use]
    pub fn simplify(&self) -> Self {
        match self {
            Self::Selector(selector) => Self::Selector(selector.simplify()),
            Self::Filter(filter) => Self::Filter(filter.simplify()),
            Self::Plan(plan) => Self::Plan(Box::new(plan.simplify())),
            Self::Sequence(steps) => Self::Sequence(steps.iter().map(Self::simplify).collect()),
        }
    }

    /// Refuse an expression holding a term past the depth or node budget.
    ///
    /// # Errors
    ///
    /// [`Term::check_budget`] carries the rule.
    pub fn check_budget(&self) -> Result<()> {
        match self {
            Self::Selector(selector) => selector.check_budget(),
            Self::Filter(filter) => filter.check_budget(),
            Self::Plan(plan) => plan.check_budget(),
            Self::Sequence(steps) => steps.iter().try_for_each(Self::check_budget),
        }
    }

    /// The struct root this expression leaves a stream at, from `root`.
    ///
    /// A selector publishes its projections; a filter keeps the root once it
    /// has proven it binds; a plan declares or publishes; a sequence threads
    /// the root through its steps. This is the schema-only application, so a
    /// reader reports its shape before it yields anything.
    ///
    /// # Errors
    ///
    /// Returns an error when the expression does not bind against the root.
    pub fn apply_field(&self, root: &Field) -> Result<Field> {
        match self {
            Self::Plan(plan) => plan.field_from(root),
            Self::Selector(_) | Self::Filter(_) | Self::Sequence(_) => root
                .clone()
                .try_with_dtype(self.apply_datatype(root.dtype())?),
        }
    }

    /// The struct datatype this expression leaves a stream at, from the
    /// struct `dtype`: the question [`apply_field`](Self::apply_field) and
    /// every reader's declared shape are answered by.
    ///
    /// # Errors
    ///
    /// Returns an error when the expression does not bind against the
    /// datatype.
    pub fn apply_datatype(&self, dtype: &crate::DataType) -> Result<crate::DataType> {
        match self {
            Self::Selector(selector) => selector.apply_datatype(dtype),
            Self::Filter(filter) => filter.apply_datatype(dtype),
            Self::Plan(plan) => plan.apply_datatype(dtype),
            Self::Sequence(steps) => steps
                .iter()
                .try_fold(dtype.clone(), |dtype, step| step.apply_datatype(&dtype)),
        }
    }
}

/// Whether the `where` clause reads a name only the `select` clause
/// publishes - an alias, which DuckDB lets a `where` read - so it has to run
/// after the projection rather than before it.
///
/// `input` names the columns the rows carry before the projection. A filter
/// over stored columns runs first, which is what lets it prune; a filter that
/// names an alias runs over the published rows, because that is the only
/// place the alias exists.
pub(crate) fn filter_after_select<'a>(
    filter: &Filter,
    select: &Selector,
    input: impl IntoIterator<Item = &'a str>,
) -> bool {
    if filter.is_always_true() || select.is_all() {
        return false;
    }
    let input: Vec<&str> = input.into_iter().collect();
    let published = select.names();
    filter.columns().iter().any(|column| {
        !input.iter().any(|held| held.eq_ignore_ascii_case(column))
            && published
                .iter()
                .any(|name| name.eq_ignore_ascii_case(column))
    })
}

impl From<Selector> for Expression {
    fn from(selector: Selector) -> Self {
        Self::Selector(selector)
    }
}

impl From<Filter> for Expression {
    fn from(filter: Filter) -> Self {
        Self::Filter(filter)
    }
}

impl From<Plan> for Expression {
    fn from(plan: Plan) -> Self {
        plan.into_expression()
    }
}

impl std::str::FromStr for Expression {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self> {
        parser::parse_expression(input)
    }
}

impl TryFrom<&str> for Expression {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        value.parse()
    }
}

/// Anything a call site may hand over where an expression is wanted.
///
/// Text parses, with its clause keyword in front; a selector, a filter, a
/// plan, and a term - read as a filter - are the expressions they already
/// are; a field is the plan that declares it.
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

impl IntoExpression for Selector {
    fn into_expression(self) -> Result<Expression> {
        Ok(Expression::Selector(self))
    }
}

impl IntoExpression for Filter {
    fn into_expression(self) -> Result<Expression> {
        Ok(Expression::Filter(self))
    }
}

impl IntoExpression for Plan {
    fn into_expression(self) -> Result<Expression> {
        Ok(Plan::into_expression(self))
    }
}

impl IntoExpression for Term {
    fn into_expression(self) -> Result<Expression> {
        Ok(Expression::Filter(Filter::new(self)))
    }
}

impl IntoExpression for &Field {
    fn into_expression(self) -> Result<Expression> {
        Ok(Expression::Plan(Box::new(Plan::from_field(self))))
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

/// The one scalar conversion, for the datatypes that expose it.
pub(crate) fn convert_scalar(
    target: &crate::DataType,
    value: &crate::Scalar,
    strict: bool,
) -> Result<crate::Scalar> {
    eval::convert(
        target,
        value,
        if strict { Safety::Strict } else { Safety::Safe },
    )
}

/// The error a text that opens no clause and no section produces.
pub(crate) fn unknown_clause(got: &str) -> Error {
    Error::Parse {
        target: "expression",
        position: 0,
        reason: format_smolstr!(
            "expected `select`, `where`, `create`, `insert`, `upsert`, `delete` or `from` to open \
             an expression, got {}",
            crate::text::elide_to(got, 64)
        ),
    }
}

/// The error a record application raises before any row runs.
pub(crate) fn unwritable_rows(reason: impl std::fmt::Display) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: format_smolstr!("{reason}"),
    }
}

/// Name one output field after the text that produced it.
pub(crate) fn named(term: &Term, dtype: crate::DataType, nullable: bool) -> Field {
    Field::new(SmolStr::new(term.to_string()), dtype, nullable)
}
