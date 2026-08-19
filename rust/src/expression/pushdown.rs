//! Answering a predicate without reading the rows.
//!
//! Two questions, both conservative, both answered from the same resolved tree
//! the evaluators walk.
//!
//! *Can any row in this container match?* [`Bound::statistics_prune`] answers
//! it from per-column minimums, maximums, and null counts - the statistics a
//! Parquet footer, an Iceberg manifest, and a Hive path all carry in some
//! form. It is three-valued and it only ever answers `false` when it can
//! *prove* that no row matches; everything it cannot prove is a `true` that
//! costs one read.
//!
//! *Which part of this predicate can a directory layout answer?*
//! [`Bound::partition_split`] answers it by splitting the conjunction into the
//! part that reads only partition columns and holder attributes, and the
//! [`Residual`] that does not. The first part prunes the listing; the second
//! runs over the rows that survive. Splitting a conjunction is sound because
//! dropping conjuncts only ever widens what is kept.
//!
//! # The rule both obey
//!
//! A pruning decision may never lose a row. Every rule here is written so that
//! the failure mode of not knowing something is reading more, never returning
//! less - which is why the type they answer in has three values and the one
//! that means "skip it" is the one that needs proof.

use smol_str::SmolStr;

use super::bind::{Bound, Kind, Node};
use super::eval::{compare as compare_values, order};
use super::selector::Selector;
use super::{Comparison, Expression, Function};
use crate::{Field, Value};

/// What a statistics-level answer can be.
///
/// Three-valued for the reason SQL's logic is: a container whose statistics
/// overlap the predicate has not said yes, it has said *maybe*, and treating
/// maybe as yes is the only reading that cannot lose a row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Certainty {
    /// Every row in the container matches.
    Always,
    /// No row in the container matches.
    Never,
    /// The statistics do not settle it.
    Unknown,
}

impl Certainty {
    const fn of(known: Option<bool>) -> Self {
        match known {
            Some(true) => Self::Always,
            Some(false) => Self::Never,
            None => Self::Unknown,
        }
    }

    const fn negated(self) -> Self {
        match self {
            Self::Always => Self::Never,
            Self::Never => Self::Always,
            Self::Unknown => Self::Unknown,
        }
    }
}

/// One column's statistics: what it holds at least, at most, and how often not.
#[derive(Clone, Debug, Default)]
pub struct ColumnBounds {
    minimum: Option<Value>,
    maximum: Option<Value>,
    nulls: Option<u64>,
}

impl ColumnBounds {
    /// The smallest value the column holds, when it is known.
    #[must_use]
    pub const fn minimum(&self) -> Option<&Value> {
        self.minimum.as_ref()
    }

    /// The largest value the column holds, when it is known.
    #[must_use]
    pub const fn maximum(&self) -> Option<&Value> {
        self.maximum.as_ref()
    }

    /// How many rows hold null in this column, when it is known.
    #[must_use]
    pub const fn nulls(&self) -> Option<u64> {
        self.nulls
    }
}

/// The statistics of one container, by column name.
///
/// Built the same way from every source that has them - a Parquet row-group
/// footer, an Iceberg manifest entry, a Hive path whose partition value is
/// both the minimum and the maximum - so the pruning rule is written once.
#[derive(Clone, Debug, Default)]
pub struct Bounds {
    columns: Vec<(SmolStr, ColumnBounds)>,
    attributes: Vec<(Selector, ColumnBounds)>,
    rows: Option<u64>,
}

impl Bounds {
    /// Start from a container of a known - or unknown - number of rows.
    #[must_use]
    pub const fn new(rows: Option<u64>) -> Self {
        Self {
            columns: Vec::new(),
            attributes: Vec::new(),
            rows,
        }
    }

    /// Record one column's minimum, maximum, and null count.
    ///
    /// Any of the three may be absent, and an absent one only ever means the
    /// rules that needed it decline.
    #[must_use]
    pub fn with_column(
        mut self,
        name: impl Into<SmolStr>,
        minimum: Option<Value>,
        maximum: Option<Value>,
        nulls: Option<u64>,
    ) -> Self {
        self.columns.push((
            name.into(),
            ColumnBounds {
                minimum,
                maximum,
                nulls,
            },
        ));
        self
    }

    /// The statistics a Hive path spells out.
    ///
    /// A path-partitioned directory says every row below it holds one value in
    /// each partition column, which is the tightest statistic there is: the
    /// minimum and the maximum are the same value and nothing is null. The
    /// value is read through the column's own datatype, so `year=2024` prunes
    /// an `int32` partition column as a number rather than as text.
    #[must_use]
    pub fn from_partitions(schema: &Field, partitions: &[(String, String)]) -> Self {
        let mut bounds = Self::new(None);
        for (column, written) in partitions {
            let Some(field) = schema
                .fields()
                .iter()
                .find(|held| held.name().eq_ignore_ascii_case(column))
            else {
                continue;
            };
            let value = super::eval::convert(
                field.data_type(),
                &Value::from(written.as_str()),
                super::Safety::Safe,
            )
            .unwrap_or(Value::Null);
            if value.is_null() {
                continue;
            }
            bounds = bounds.with_column(field.name(), Some(value.clone()), Some(value), Some(0));
        }
        bounds
    }

    /// Record what a holder attribute is known to hold.
    ///
    /// A container's *identity* is a statistic too: a file at
    /// `year=2024/part-0.parquet` says its `year` partition is `2024` for every
    /// row it holds, and it says so without being opened. Recording it here is
    /// what lets `&holder.partition['year'] = '2024'` prune through the same
    /// rule a column minimum and maximum prune through.
    #[must_use]
    pub fn with_attribute(
        mut self,
        selector: Selector,
        minimum: Option<Value>,
        maximum: Option<Value>,
        nulls: Option<u64>,
    ) -> Self {
        self.attributes.push((
            selector,
            ColumnBounds {
                minimum,
                maximum,
                nulls,
            },
        ));
        self
    }

    /// The statistics an identifier states about itself.
    ///
    /// Every free selector answers exactly, so each one is a minimum equal to
    /// its maximum: a path does not bound its own name, it *is* its name.
    #[must_use]
    pub fn from_url(url: &crate::Url) -> Self {
        let mut bounds = Self::new(None);
        for selector in Selector::ALL {
            if !matches!(selector.cost(), super::selector::Cost::Free) {
                continue;
            }
            let value = selector.read_url(url);
            let nulls = Some(u64::from(value.is_null()));
            bounds = bounds.with_attribute(selector, Some(value.clone()), Some(value), nulls);
        }
        for (column, _) in url.hive_partitions() {
            let selector = Selector::Partition(SmolStr::new(&column));
            let value = selector.read_url(url);
            bounds = bounds.with_attribute(selector, Some(value.clone()), Some(value), Some(0));
        }
        bounds
    }

    /// Merge another set of statistics into this one, keeping both.
    ///
    /// A later entry never replaces an earlier one: the lookup takes the first
    /// match, so whichever source was added first is the one that answers.
    #[must_use]
    pub fn with(mut self, other: Self) -> Self {
        self.columns.extend(other.columns);
        self.attributes.extend(other.attributes);
        if self.rows.is_none() {
            self.rows = other.rows;
        }
        self
    }

    /// How many rows the container holds, when it is known.
    #[must_use]
    pub const fn row_count(&self) -> Option<u64> {
        self.rows
    }

    /// One attribute's statistics.
    #[must_use]
    pub fn attribute(&self, selector: &Selector) -> Option<&ColumnBounds> {
        self.attributes
            .iter()
            .find(|(held, _)| held == selector)
            .map(|(_, bounds)| bounds)
    }

    /// One column's statistics, ASCII case-insensitively.
    #[must_use]
    pub fn column(&self, name: &str) -> Option<&ColumnBounds> {
        self.columns
            .iter()
            .find(|(held, _)| held.eq_ignore_ascii_case(name))
            .map(|(_, bounds)| bounds)
    }
}

/// A predicate split into the part a layout can answer and the part it cannot.
///
/// The two halves conjoined are the original predicate, which is the property
/// that makes running them in two places sound.
#[derive(Clone, Debug)]
pub struct Residual {
    answerable: Expression,
    remaining: Expression,
}

impl Residual {
    /// The conjuncts a partition layout or a listing can settle by itself.
    #[must_use]
    pub const fn answerable(&self) -> &Expression {
        &self.answerable
    }

    /// The conjuncts that still need the rows.
    #[must_use]
    pub const fn remaining(&self) -> &Expression {
        &self.remaining
    }

    /// Return whether the layout can answer the whole predicate.
    ///
    /// When it can, a container that survives pruning needs no row filter at
    /// all, which is what turns a partition scan into a plain read.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.remaining.is_always_true()
    }

    /// Return whether the layout can answer none of it.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.answerable.is_always_true()
    }
}

impl Bound {
    /// Return whether any row of a container could match this predicate.
    ///
    /// `false` is a proof and `true` is the absence of one. A container with a
    /// known row count of zero never matches; a statistic that is missing only
    /// makes the rules that needed it decline.
    #[must_use]
    pub fn statistics_prune(&self, bounds: &Bounds) -> bool {
        self.statistics_certainty(bounds) != Some(false)
    }

    /// What a container's statistics settle about this predicate.
    ///
    /// `Some(true)` means every row matches and the rows need not be tested
    /// again; `Some(false)` means none does and the container can be skipped;
    /// `None` means the statistics do not say. The first answer is what turns
    /// a partition scan into a plain read - a file whose whole partition
    /// satisfies a conjunct does not have to re-test it row by row.
    #[must_use]
    pub fn statistics_certainty(&self, bounds: &Bounds) -> Option<bool> {
        // The statistics target never fails: a statistic it cannot read only
        // widens the answer to unknown, which is already the conservative one.
        super::ApplyExpression::apply_expression(bounds, self).unwrap_or(None)
    }

    /// Split this predicate into the part a partition layout answers and the
    /// rest.
    ///
    /// A conjunct is answerable when every column it reads is declared a
    /// partition field and it reads nothing else that needs a row. Holder
    /// attributes are answerable too, because a listing knows them.
    #[must_use]
    pub fn partition_split(&self) -> Residual {
        let fields = self.schema().fields();
        let mut answerable = Vec::new();
        let mut remaining = Vec::new();
        for conjunct in self.node().conjuncts() {
            let settled = conjunct
                .column_indices()
                .iter()
                .all(|index| fields.get(*index).is_some_and(crate::Field::is_partition));
            let rebuilt = super::bind::rebuild(conjunct);
            if settled {
                answerable.push(rebuilt);
            } else {
                remaining.push(rebuilt);
            }
        }
        Residual {
            answerable: Expression::all(answerable),
            remaining: Expression::all(remaining),
        }
    }
}

/// One container's statistics apply to the three-valued certainty pruning
/// runs on.
///
/// The implementation lives here rather than beside the trait because it is
/// the pruning rules below asked as one question, and the rules and their
/// caller belong on the same page.
impl super::ApplyExpression for Bounds {
    type Output = Option<bool>;

    fn apply_expression(&self, bound: &Bound) -> crate::Result<Option<bool>> {
        if self.row_count() == Some(0) {
            return Ok(Some(false));
        }
        Ok(match prune(bound.node(), bound.schema(), self) {
            Certainty::Always => Some(true),
            Certainty::Never => Some(false),
            Certainty::Unknown => None,
        })
    }
}

/// Answer one node from statistics, three-valued and always conservative.
fn prune(node: &Node, schema: &Field, bounds: &Bounds) -> Certainty {
    match &node.kind {
        Kind::Literal(value) => Certainty::of(value.as_bool()),
        Kind::And(operands) => {
            let mut certain = Certainty::Always;
            for operand in operands {
                match prune(operand, schema, bounds) {
                    Certainty::Never => return Certainty::Never,
                    Certainty::Unknown => certain = Certainty::Unknown,
                    Certainty::Always => {}
                }
            }
            certain
        }
        Kind::Or(operands) => {
            let mut certain = Certainty::Never;
            for operand in operands {
                match prune(operand, schema, bounds) {
                    Certainty::Always => return Certainty::Always,
                    Certainty::Unknown => certain = Certainty::Unknown,
                    Certainty::Never => {}
                }
            }
            certain
        }
        // `not unknown` is unknown, so the negation of an unproven answer stays
        // unproven and nothing is skipped on the strength of it.
        Kind::Not(inner) => prune(inner, schema, bounds).negated(),
        Kind::Compare(left, comparison, right) => match (oriented(left, right), comparison) {
            (Some((column, literal, flipped)), _) => {
                let comparison = if flipped {
                    comparison.flipped()
                } else {
                    *comparison
                };
                compare_bounds(column, comparison, literal, schema, bounds)
            }
            _ => Certainty::Unknown,
        },
        Kind::In(value, list) => {
            let Some(column) = column_bounds(value, schema, bounds) else {
                return Certainty::Unknown;
            };
            // A value list prunes only when *every* value is outside the range,
            // because one that is inside is a row that might match.
            let mut certain = Certainty::Never;
            for item in list {
                let Some(literal) = item.as_literal() else {
                    return Certainty::Unknown;
                };
                match compare_range(value, column, Comparison::Eq, literal) {
                    Certainty::Never => {}
                    _ => certain = Certainty::Unknown,
                }
            }
            certain
        }
        Kind::Between(value, low, high) => {
            let above = match low.as_literal() {
                Some(literal) => settle(value, Comparison::GtEq, literal, schema, bounds),
                None => Certainty::Unknown,
            };
            if above == Certainty::Never {
                return Certainty::Never;
            }
            let below = match high.as_literal() {
                Some(literal) => settle(value, Comparison::LtEq, literal, schema, bounds),
                None => Certainty::Unknown,
            };
            match (above, below) {
                (_, Certainty::Never) => Certainty::Never,
                (Certainty::Always, Certainty::Always) => Certainty::Always,
                _ => Certainty::Unknown,
            }
        }
        Kind::IsNull(inner) => nullness(inner, schema, bounds, true),
        Kind::IsNotNull(inner) => nullness(inner, schema, bounds, false),
        // `col like 'p%'` implies `col >= 'p'`, which is the one text rule a
        // range can settle. The prefix stops at the first wildcard.
        Kind::Like {
            value,
            pattern,
            case_insensitive,
            escape,
        } => {
            if *case_insensitive {
                return Certainty::Unknown;
            }
            match literal_prefix(pattern, *escape) {
                Some(prefix) => prefix_prune(value, &prefix, schema, bounds),
                None => Certainty::Unknown,
            }
        }
        Kind::Function(Function::StartsWith, arguments) => {
            match (
                arguments.first(),
                arguments.get(1).and_then(Node::as_literal),
            ) {
                (Some(value), Some(prefix)) => match prefix.as_str() {
                    Some(prefix) => prefix_prune(value, prefix, schema, bounds),
                    None => Certainty::Unknown,
                },
                _ => Certainty::Unknown,
            }
        }
        _ => Certainty::Unknown,
    }
}

/// Orient a comparison as `column op literal`, saying whether it was flipped.
fn oriented<'node>(
    left: &'node Node,
    right: &'node Node,
) -> Option<(&'node Node, &'node Value, bool)> {
    if let Some(literal) = right.as_literal() {
        return Some((left, literal, false));
    }
    if let Some(literal) = left.as_literal() {
        return Some((right, literal, true));
    }
    None
}

/// The statistics of the column a node reads, when it reads exactly one.
fn column_bounds<'bounds>(
    node: &Node,
    schema: &Field,
    bounds: &'bounds Bounds,
) -> Option<&'bounds ColumnBounds> {
    if let Kind::Attribute(selector) = &node.kind {
        return bounds.attribute(selector);
    }
    let index = node.as_column()?;
    let field = schema.get_field(index)?;
    bounds.column(field.name())
}

/// Settle one `column op literal` against the column's statistics.
fn compare_bounds(
    node: &Node,
    comparison: Comparison,
    literal: &Value,
    schema: &Field,
    bounds: &Bounds,
) -> Certainty {
    settle(node, comparison, literal, schema, bounds)
}

fn settle(
    node: &Node,
    comparison: Comparison,
    literal: &Value,
    schema: &Field,
    bounds: &Bounds,
) -> Certainty {
    let Some(column) = column_bounds(node, schema, bounds) else {
        return Certainty::Unknown;
    };
    // A column that is null everywhere answers unknown for every comparison,
    // and unknown is not true, so no row matches.
    if let (Some(nulls), Some(rows)) = (column.nulls, bounds.row_count()) {
        if nulls == rows && rows > 0 && !comparison.is_two_valued() {
            return Certainty::Never;
        }
    }
    compare_range(node, column, comparison, literal)
}

/// Settle one comparison against a `[minimum, maximum]` range.
fn compare_range(
    node: &Node,
    column: &ColumnBounds,
    comparison: Comparison,
    literal: &Value,
) -> Certainty {
    let data_type = node.field.data_type();
    let below = |value: &Value| -> Option<bool> {
        column
            .minimum
            .as_ref()
            .and_then(|minimum| order(data_type, value, minimum))
            .map(std::cmp::Ordering::is_lt)
    };
    let above = |value: &Value| -> Option<bool> {
        column
            .maximum
            .as_ref()
            .and_then(|maximum| order(data_type, value, maximum))
            .map(std::cmp::Ordering::is_gt)
    };
    let single = column
        .minimum
        .as_ref()
        .zip(column.maximum.as_ref())
        .and_then(|(minimum, maximum)| {
            (order(data_type, minimum, maximum)? == std::cmp::Ordering::Equal).then_some(minimum)
        });
    match comparison {
        Comparison::Eq | Comparison::IsNotDistinctFrom => {
            if below(literal) == Some(true) || above(literal) == Some(true) {
                return Certainty::Never;
            }
            // A column whose minimum equals its maximum holds one value, so an
            // equality against it is settled either way.
            match single {
                Some(held) => Certainty::of(
                    compare_values(data_type, held, Comparison::Eq, literal).as_bool(),
                ),
                None => Certainty::Unknown,
            }
        }
        Comparison::NotEq | Comparison::IsDistinctFrom => match single {
            Some(held) => {
                Certainty::of(compare_values(data_type, held, Comparison::NotEq, literal).as_bool())
            }
            None => Certainty::Unknown,
        },
        Comparison::Lt | Comparison::LtEq => {
            // Nothing is below the minimum, so a bound at or under it is empty.
            let strict = matches!(comparison, Comparison::Lt);
            if let Some(minimum) = &column.minimum {
                if let Some(ordering) = order(data_type, literal, minimum) {
                    if ordering.is_lt() || (strict && ordering.is_eq()) {
                        return Certainty::Never;
                    }
                }
            }
            if let Some(maximum) = &column.maximum {
                if let Some(ordering) = order(data_type, maximum, literal) {
                    if ordering.is_lt() || (!strict && ordering.is_eq()) {
                        return Certainty::Always;
                    }
                }
            }
            Certainty::Unknown
        }
        Comparison::Gt | Comparison::GtEq => {
            let strict = matches!(comparison, Comparison::Gt);
            if let Some(maximum) = &column.maximum {
                if let Some(ordering) = order(data_type, literal, maximum) {
                    if ordering.is_gt() || (strict && ordering.is_eq()) {
                        return Certainty::Never;
                    }
                }
            }
            if let Some(minimum) = &column.minimum {
                if let Some(ordering) = order(data_type, minimum, literal) {
                    if ordering.is_gt() || (!strict && ordering.is_eq()) {
                        return Certainty::Always;
                    }
                }
            }
            Certainty::Unknown
        }
    }
}

/// Settle a null test from a column's null count.
fn nullness(node: &Node, schema: &Field, bounds: &Bounds, asking_null: bool) -> Certainty {
    let Some(column) = column_bounds(node, schema, bounds) else {
        return Certainty::Unknown;
    };
    let Some(nulls) = column.nulls else {
        return Certainty::Unknown;
    };
    if nulls == 0 {
        return Certainty::of(Some(!asking_null));
    }
    match bounds.row_count() {
        Some(rows) if nulls == rows => Certainty::of(Some(asking_null)),
        _ => Certainty::Unknown,
    }
}

/// The literal prefix of a `like` pattern, up to its first wildcard.
fn literal_prefix(pattern: &str, escape: Option<char>) -> Option<String> {
    let mut prefix = String::new();
    let mut characters = pattern.chars();
    while let Some(character) = characters.next() {
        if Some(character) == escape {
            match characters.next() {
                Some(escaped) => prefix.push(escaped),
                None => break,
            }
            continue;
        }
        if character == '%' || character == '_' {
            break;
        }
        prefix.push(character);
    }
    (!prefix.is_empty()).then_some(prefix)
}

/// Settle a prefix match: it implies `column >= prefix`, and no more.
fn prefix_prune(node: &Node, prefix: &str, schema: &Field, bounds: &Bounds) -> Certainty {
    let Some(column) = column_bounds(node, schema, bounds) else {
        return Certainty::Unknown;
    };
    // A maximum below the prefix proves no row starts with it. The upper side
    // needs the successor of the prefix, which is not spellable for every
    // encoding, so only the lower side prunes.
    let data_type = node.field.data_type();
    let literal = Value::from(prefix);
    if let Some(maximum) = &column.maximum {
        if order(data_type, maximum, &literal).is_some_and(std::cmp::Ordering::is_lt) {
            return Certainty::Never;
        }
    }
    Certainty::Unknown
}
