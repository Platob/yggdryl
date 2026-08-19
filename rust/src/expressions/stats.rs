//! Three-valued evaluation over column statistics.
//!
//! This is what makes a filter *prune* rather than merely filter: given the
//! bounds and null counts a manifest, a Parquet footer, or a `column=value`
//! directory already carries, the same plan the rows are filtered with answers
//! whether a file can hold a matching row at all.
//!
//! # The one rule that outranks the others
//!
//! [`Certainty::Maybe`] is always safe. [`Certainty::AlwaysFalse`] must be
//! *provable*, because a wrong one loses rows silently - and a lost row is the
//! single failure this module must never have. A missing statistic, an
//! unsupported node, an accessor no statistic bounds: all of them answer
//! `Maybe`, and every rule below has a test that feeds it deliberately coarse
//! statistics to prove it still does.
//!
//! [`Certainty::AlwaysTrue`] is claimed only where it is provable too, because
//! it is what lets a residual drop a conjunct - and dropping a conjunct that
//! was not actually settled would keep rows that should have gone.

use super::bound::{BoundColumn, compare};
use super::graph::{Node, NodeId, Plan};
use super::{CompareOp, Value};

/// What statistics can prove about a predicate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Certainty {
    /// Every row matches; the conjunct can be dropped from the residual.
    AlwaysTrue,
    /// No row matches; the file, manifest, or directory can be skipped whole.
    AlwaysFalse,
    /// The statistics leave room for both, so the rows have to answer.
    Maybe,
}

impl Certainty {
    /// Kleene conjunction over two certainties.
    #[must_use]
    pub const fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::AlwaysFalse, _) | (_, Self::AlwaysFalse) => Self::AlwaysFalse,
            (Self::AlwaysTrue, Self::AlwaysTrue) => Self::AlwaysTrue,
            _ => Self::Maybe,
        }
    }

    /// Kleene disjunction over two certainties.
    #[must_use]
    pub const fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::AlwaysTrue, _) | (_, Self::AlwaysTrue) => Self::AlwaysTrue,
            (Self::AlwaysFalse, Self::AlwaysFalse) => Self::AlwaysFalse,
            _ => Self::Maybe,
        }
    }

    /// Kleene negation.
    ///
    /// Unknown negates to unknown, which is why `NOT` never turns a `Maybe`
    /// into a decision.
    #[must_use]
    pub const fn not(self) -> Self {
        match self {
            Self::AlwaysTrue => Self::AlwaysFalse,
            Self::AlwaysFalse => Self::AlwaysTrue,
            Self::Maybe => Self::Maybe,
        }
    }

    /// Return whether this certainty allows a match at all.
    #[must_use]
    #[inline]
    pub const fn is_possible(self) -> bool {
        !matches!(self, Self::AlwaysFalse)
    }
}

impl std::fmt::Display for Certainty {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::AlwaysTrue => "always true",
            Self::AlwaysFalse => "always false",
            Self::Maybe => "maybe",
        })
    }
}

/// What one source knows about one column.
///
/// Every field is optional because every one of them is genuinely optional in
/// the formats this reads: a manifest list written without summaries says
/// nothing, a Parquet footer may carry counts without bounds, and a partition
/// directory carries a value with no counts at all.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ColumnStats {
    /// The smallest value the column holds, if it is known.
    pub lower: Option<Value>,
    /// The largest value the column holds, if it is known.
    pub upper: Option<Value>,
    /// How many rows hold no value, if it is known.
    pub null_count: Option<u64>,
    /// How many rows the column has at all, if it is known.
    pub value_count: Option<u64>,
}

impl ColumnStats {
    /// Statistics that say nothing, so nothing may be skipped on them.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            lower: None,
            upper: None,
            null_count: None,
            value_count: None,
        }
    }

    /// The statistics a constant column has: one value, no nulls.
    ///
    /// This is what a `column=value` directory answers, and it is why
    /// directory pruning is a special case of statistics pruning rather than a
    /// second code path.
    #[must_use]
    pub fn constant(value: Value) -> Self {
        let null = matches!(value, Value::Null);
        Self {
            lower: (!null).then(|| value.clone()),
            upper: (!null).then_some(value),
            null_count: Some(u64::from(null)),
            value_count: Some(1),
        }
    }

    /// Statistics with a known range and no nulls.
    #[must_use]
    pub fn range(lower: Value, upper: Value) -> Self {
        Self {
            lower: Some(lower),
            upper: Some(upper),
            null_count: Some(0),
            value_count: None,
        }
    }

    /// Return these statistics with a null count.
    #[must_use]
    pub fn with_null_count(mut self, null_count: u64) -> Self {
        self.null_count = Some(null_count);
        self
    }

    /// Return these statistics with a total row count.
    #[must_use]
    pub fn with_value_count(mut self, value_count: u64) -> Self {
        self.value_count = Some(value_count);
        self
    }

    /// Return whether the column is known to hold no null at all.
    #[must_use]
    pub fn has_no_nulls(&self) -> bool {
        self.null_count == Some(0)
    }

    /// Return whether the column is known to hold nothing but nulls.
    #[must_use]
    pub fn is_all_null(&self) -> bool {
        match (self.null_count, self.value_count) {
            (Some(nulls), Some(values)) => values > 0 && nulls == values,
            _ => false,
        }
    }
}

/// Anything that can say what it knows about a column's values.
///
/// The trait is deliberately tiny and deliberately knows nothing: an Iceberg
/// manifest summary, an Iceberg data file's bounds, a Parquet row group, and a
/// `column=value` directory each implement it without this module learning that
/// any of them exists. Dependencies point one way.
pub trait StatsSource {
    /// What is known about one column, or nothing.
    fn stats(&self, column: &BoundColumn) -> Option<ColumnStats>;
}

impl StatsSource for () {
    /// A source that knows nothing, which is the identity for pruning.
    fn stats(&self, _column: &BoundColumn) -> Option<ColumnStats> {
        None
    }
}

impl<S: StatsSource + ?Sized> StatsSource for &S {
    fn stats(&self, column: &BoundColumn) -> Option<ColumnStats> {
        (**self).stats(column)
    }
}

/// Decide what `source` can prove about the plan rooted at `root`.
pub(super) fn evaluate(plan: &Plan, root: NodeId, source: &dyn StatsSource) -> Certainty {
    let Some(node) = plan.get(root) else {
        return Certainty::Maybe;
    };
    match node {
        Node::Literal(Value::Bool(true)) => Certainty::AlwaysTrue,
        Node::Literal(Value::Bool(false) | Value::Null) => Certainty::AlwaysFalse,
        Node::And(operands) => operands
            .iter()
            .fold(Certainty::AlwaysTrue, |held, operand| {
                held.and(evaluate(plan, *operand, source))
            }),
        Node::Or(operands) => operands
            .iter()
            .fold(Certainty::AlwaysFalse, |held, operand| {
                held.or(evaluate(plan, *operand, source))
            }),
        Node::Not(child) => evaluate(plan, *child, source).not(),
        Node::Alias { child, .. } => evaluate(plan, *child, source),
        Node::IsNull(child) => match column_stats(plan, *child, source) {
            Some(stats) if stats.has_no_nulls() => Certainty::AlwaysFalse,
            Some(stats) if stats.is_all_null() => Certainty::AlwaysTrue,
            _ => Certainty::Maybe,
        },
        Node::IsNotNull(child) => match column_stats(plan, *child, source) {
            Some(stats) if stats.is_all_null() => Certainty::AlwaysFalse,
            // A column with no nulls answers true for every row it has; a
            // column with no rows at all has nothing to answer for, and the
            // file that holds it is skipped for having no rows, not here.
            Some(stats) if stats.has_no_nulls() => Certainty::AlwaysTrue,
            _ => Certainty::Maybe,
        },
        Node::Compare { op, left, right } => comparison(plan, *op, *left, *right, source),
        Node::In {
            child,
            list,
            negated,
        } => {
            if *negated {
                // `NOT IN` is only provable when every stored value is one of
                // the listed ones, which statistics never say.
                return Certainty::Maybe;
            }
            let Some(stats) = column_stats(plan, *child, source) else {
                return Certainty::Maybe;
            };
            let mut certainty = Certainty::AlwaysFalse;
            for item in list {
                let Some(value) = plan.get(*item).and_then(Node::as_literal) else {
                    return Certainty::Maybe;
                };
                certainty = certainty.or(bounded(&stats, CompareOp::Eq, value));
            }
            certainty
        }
        Node::Between {
            child,
            low,
            high,
            negated,
        } => {
            let Some(stats) = column_stats(plan, *child, source) else {
                return Certainty::Maybe;
            };
            let (Some(low), Some(high)) = (
                plan.get(*low).and_then(Node::as_literal),
                plan.get(*high).and_then(Node::as_literal),
            ) else {
                return Certainty::Maybe;
            };
            let certainty =
                bounded(&stats, CompareOp::GtEq, low).and(bounded(&stats, CompareOp::LtEq, high));
            if *negated { certainty.not() } else { certainty }
        }
        Node::StartsWith { child, prefix } => {
            let Some(stats) = column_stats(plan, *child, source) else {
                return Certainty::Maybe;
            };
            prefix_bounded(&stats, prefix)
        }
        // Everything else - arithmetic, functions, casts, `CASE`, `LIKE` with a
        // wildcard anywhere but the end - has no statistic that bounds it.
        _ => Certainty::Maybe,
    }
}

/// The statistics for a node, when the node is a prunable bound column.
fn column_stats(plan: &Plan, id: NodeId, source: &dyn StatsSource) -> Option<ColumnStats> {
    let column = plan.get(id)?.as_column()?.bound()?;
    // A list element, a map entry, and every range have no statistic that
    // bounds them, so they never prune. See `BoundColumn::is_prunable`.
    if !column.is_prunable() {
        return None;
    }
    source.stats(column)
}

/// Decide one comparison, whichever side the column is on.
fn comparison(
    plan: &Plan,
    op: CompareOp,
    left: NodeId,
    right: NodeId,
    source: &dyn StatsSource,
) -> Certainty {
    if let (Some(stats), Some(value)) = (
        column_stats(plan, left, source),
        plan.get(right).and_then(Node::as_literal),
    ) {
        return bounded(&stats, op, value);
    }
    if let (Some(value), Some(stats)) = (
        plan.get(left).and_then(Node::as_literal),
        column_stats(plan, right, source),
    ) {
        // The stored column is on the right, so the question is the mirrored
        // one: `3 < price` asks exactly what `price > 3` asks.
        return bounded(&stats, op.flipped(), value);
    }
    Certainty::Maybe
}

/// Decide `column op value` from what is known about the column.
fn bounded(stats: &ColumnStats, op: CompareOp, value: &Value) -> Certainty {
    if matches!(value, Value::Null) {
        // A comparison with null is unknown for every row, so no row matches -
        // which is a provable `false` for a filter, not a `maybe`.
        return Certainty::AlwaysFalse;
    }
    if stats.is_all_null() {
        return Certainty::AlwaysFalse;
    }
    let lower = stats.lower.as_ref();
    let upper = stats.upper.as_ref();
    // Only a column with no nulls can prove a comparison true for every row;
    // with one null anywhere, the answer for that row is unknown.
    let provable_true = stats.has_no_nulls();
    match op {
        CompareOp::Eq => {
            if outside(lower, upper, value) {
                return Certainty::AlwaysFalse;
            }
            if provable_true && lower == upper && lower == Some(value) {
                return Certainty::AlwaysTrue;
            }
            Certainty::Maybe
        }
        CompareOp::NotEq => {
            if provable_true && lower == upper && lower == Some(value) {
                return Certainty::AlwaysFalse;
            }
            if provable_true && outside(lower, upper, value) {
                return Certainty::AlwaysTrue;
            }
            Certainty::Maybe
        }
        // `column < value` is impossible when even the smallest stored value
        // is not below it, and certain when the largest one is.
        CompareOp::Lt | CompareOp::LtEq => {
            if let Some(lower) = lower {
                if compare(op, lower, value) == Some(false) {
                    return Certainty::AlwaysFalse;
                }
            }
            if provable_true {
                if let Some(upper) = upper {
                    if compare(op, upper, value) == Some(true) {
                        return Certainty::AlwaysTrue;
                    }
                }
            }
            Certainty::Maybe
        }
        CompareOp::Gt | CompareOp::GtEq => {
            if let Some(upper) = upper {
                if compare(op, upper, value) == Some(false) {
                    return Certainty::AlwaysFalse;
                }
            }
            if provable_true {
                if let Some(lower) = lower {
                    if compare(op, lower, value) == Some(true) {
                        return Certainty::AlwaysTrue;
                    }
                }
            }
            Certainty::Maybe
        }
    }
}

/// Return whether a value falls outside a known range on either side.
fn outside(lower: Option<&Value>, upper: Option<&Value>, value: &Value) -> bool {
    if let Some(lower) = lower {
        if compare(CompareOp::Lt, value, lower) == Some(true) {
            return true;
        }
    }
    if let Some(upper) = upper {
        if compare(CompareOp::Gt, value, upper) == Some(true) {
            return true;
        }
    }
    false
}

/// Decide a literal prefix test from string bounds.
///
/// Truncating both bounds to the prefix's length is what makes this correct
/// even when the format truncated them first: `v <= upper` implies
/// `v[..n] <= upper[..n]` and `v >= lower` implies `v[..n] >= lower[..n]`, so
/// comparing at the prefix's own length only ever *widens* the range - and a
/// widened range can refuse a prefix, never wrongly admit a row.
fn prefix_bounded(stats: &ColumnStats, prefix: &str) -> Certainty {
    if stats.is_all_null() {
        return Certainty::AlwaysFalse;
    }
    let head = |value: Option<&Value>| -> Option<String> {
        let text = value?.as_str()?;
        Some(truncate_chars(text, prefix.chars().count()))
    };
    if let Some(lower) = head(stats.lower.as_ref()) {
        if prefix < lower.as_str() {
            return Certainty::AlwaysFalse;
        }
    }
    if let Some(upper) = head(stats.upper.as_ref()) {
        if prefix > upper.as_str() {
            return Certainty::AlwaysFalse;
        }
    }
    Certainty::Maybe
}

/// The first `count` characters of `text`, never splitting one.
fn truncate_chars(text: &str, count: usize) -> String {
    text.chars().take(count).collect()
}
