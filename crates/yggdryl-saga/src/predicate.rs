//! The [`Predicate`] — a boolean [`Expression`] used to filter a
//! [`Frame`](crate::Frame). Its [`optimize`](Predicate::optimize) types every
//! literal against the target column (casting an untyped/string value to the
//! column's type, e.g. an ISO date → `timestamp`), which is what makes a filter
//! pushable into typed storage (Parquet/CSV).

use std::fmt;

#[allow(unused_imports)]
use crate::log_event;
use crate::{DataType, Expression, ExpressionError, PrimitiveType, Scalar, Schema};

/// A comparison operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CompareOp {
    /// `=`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
}

impl CompareOp {
    /// The operator symbol.
    pub fn as_str(&self) -> &'static str {
        match self {
            CompareOp::Eq => "=",
            CompareOp::Ne => "!=",
            CompareOp::Lt => "<",
            CompareOp::Le => "<=",
            CompareOp::Gt => ">",
            CompareOp::Ge => ">=",
        }
    }
}

impl fmt::Display for CompareOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A boolean predicate over a frame's columns: a leaf comparison, a
/// [`Between`](Predicate::Between) range, an [`In`](Predicate::In) /
/// [`NotIn`](Predicate::NotIn) collection membership, or a null check — combined
/// with `and` / `or` / `not`.
///
/// Each leaf holds one or more literal [`Scalar`]s, usually starting **untyped**
/// ([`Any`](DataType::Any)) — e.g. `col("ts") > "2024-01-01"`;
/// [`optimize`](Predicate::optimize) resolves the column's type from the schema and
/// casts every literal (a comparison's value, a range's bounds, each collection
/// value) to it, so the predicate is typed and pushable.
///
/// [`merge`](Predicate::merge) / [`simplify`](Predicate::simplify) consolidate a
/// conjunction for pushdown — flattening `AND`s, dropping duplicates, and folding a
/// same-column `>=` lower bound and `<=` upper bound into one `Between`.
///
/// ```
/// use yggdryl_saga::{CompareOp, Field, Predicate, Scalar, Schema};
///
/// let schema = Schema::new(vec![Field::new(
///     "ts",
///     yggdryl_saga::DataType::from_str("timestamp(ns, UTC)").unwrap(),
///     false,
/// )]);
///
/// // An untyped string literal …
/// let p = Predicate::compare("ts", CompareOp::Ge, Scalar::any("2024-01-01"));
/// // … is typed against the column for pushdown.
/// let typed = p.optimize(&schema).unwrap();
/// if let Predicate::Compare { value, .. } = &typed {
///     assert!(value.data_type().is_temporal());
/// }
/// ```
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Predicate {
    /// `column <op> value`.
    Compare {
        /// The column under test.
        column: String,
        /// The comparison operator.
        op: CompareOp,
        /// The literal compared against.
        value: Scalar,
    },
    /// `column BETWEEN low AND high` (both bounds inclusive).
    Between {
        /// The column under test.
        column: String,
        /// The inclusive lower bound.
        low: Scalar,
        /// The inclusive upper bound.
        high: Scalar,
    },
    /// `column IN (values…)` — membership in a collection of literals.
    In {
        /// The column under test.
        column: String,
        /// The candidate values.
        values: Vec<Scalar>,
    },
    /// `column NOT IN (values…)`.
    NotIn {
        /// The column under test.
        column: String,
        /// The excluded values.
        values: Vec<Scalar>,
    },
    /// `column IS NULL`.
    IsNull(String),
    /// `column IS NOT NULL`.
    IsNotNull(String),
    /// Logical conjunction.
    And(Box<Predicate>, Box<Predicate>),
    /// Logical disjunction.
    Or(Box<Predicate>, Box<Predicate>),
    /// Logical negation.
    Not(Box<Predicate>),
}

impl Predicate {
    /// `column <op> value`.
    pub fn compare(column: impl Into<String>, op: CompareOp, value: Scalar) -> Predicate {
        Predicate::Compare {
            column: column.into(),
            op,
            value,
        }
    }

    /// `column = value`.
    pub fn eq(column: impl Into<String>, value: Scalar) -> Predicate {
        Predicate::compare(column, CompareOp::Eq, value)
    }

    /// `column != value`.
    pub fn ne(column: impl Into<String>, value: Scalar) -> Predicate {
        Predicate::compare(column, CompareOp::Ne, value)
    }

    /// `column < value`.
    pub fn lt(column: impl Into<String>, value: Scalar) -> Predicate {
        Predicate::compare(column, CompareOp::Lt, value)
    }

    /// `column <= value`.
    pub fn le(column: impl Into<String>, value: Scalar) -> Predicate {
        Predicate::compare(column, CompareOp::Le, value)
    }

    /// `column > value`.
    pub fn gt(column: impl Into<String>, value: Scalar) -> Predicate {
        Predicate::compare(column, CompareOp::Gt, value)
    }

    /// `column >= value`.
    pub fn ge(column: impl Into<String>, value: Scalar) -> Predicate {
        Predicate::compare(column, CompareOp::Ge, value)
    }

    /// `column BETWEEN low AND high` (inclusive).
    pub fn between(column: impl Into<String>, low: Scalar, high: Scalar) -> Predicate {
        Predicate::Between {
            column: column.into(),
            low,
            high,
        }
    }

    /// `column IN (values…)` — true when the column equals any of the literals.
    pub fn is_in(column: impl Into<String>, values: impl IntoIterator<Item = Scalar>) -> Predicate {
        Predicate::In {
            column: column.into(),
            values: values.into_iter().collect(),
        }
    }

    /// `column NOT IN (values…)`.
    pub fn not_in(
        column: impl Into<String>,
        values: impl IntoIterator<Item = Scalar>,
    ) -> Predicate {
        Predicate::NotIn {
            column: column.into(),
            values: values.into_iter().collect(),
        }
    }

    /// `column IS NULL`.
    pub fn is_null(column: impl Into<String>) -> Predicate {
        Predicate::IsNull(column.into())
    }

    /// `column IS NOT NULL`.
    pub fn is_not_null(column: impl Into<String>) -> Predicate {
        Predicate::IsNotNull(column.into())
    }

    /// `self AND other`.
    pub fn and(self, other: Predicate) -> Predicate {
        Predicate::And(Box::new(self), Box::new(other))
    }

    /// `self OR other`.
    pub fn or(self, other: Predicate) -> Predicate {
        Predicate::Or(Box::new(self), Box::new(other))
    }

    /// `NOT self`.
    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> Predicate {
        Predicate::Not(Box::new(self))
    }

    /// Conjuncts this predicate with `other` and [`simplify`](Predicate::simplify)s
    /// the result — the way to fold an extra restriction into an existing filter
    /// and keep it in tight pushdown form.
    pub fn merge(self, other: Predicate) -> Predicate {
        self.and(other).simplify()
    }

    /// Normalises a conjunction for pushdown: flattens nested `AND`s, drops exact
    /// duplicate conjuncts, and folds a same-column lower bound (`>=`) and upper
    /// bound (`<=`) into a single [`Between`](Predicate::Between). `OR` / `NOT`
    /// subtrees are simplified recursively; a leaf is returned unchanged.
    pub fn simplify(self) -> Predicate {
        match self {
            Predicate::And(..) => {
                let mut conjuncts = Vec::new();
                flatten_and(self, &mut conjuncts);
                // Simplify each conjunct, then drop exact duplicates (order-stable).
                let mut parts: Vec<Predicate> = Vec::new();
                for part in conjuncts.into_iter().map(Predicate::simplify) {
                    if !parts.contains(&part) {
                        parts.push(part);
                    }
                }
                merge_bounds(&mut parts);
                rebuild_and(parts)
            }
            Predicate::Or(a, b) => Predicate::Or(Box::new(a.simplify()), Box::new(b.simplify())),
            Predicate::Not(p) => Predicate::Not(Box::new(p.simplify())),
            leaf => leaf,
        }
    }

    /// Type-optimises the predicate against `schema`: each comparison's literal is
    /// cast to the type of the column it is compared with (so an untyped
    /// [`Any`](DataType::Any) or string value becomes a typed one), and every
    /// referenced column is checked to exist. This is the transform that makes a
    /// filter pushable into typed storage.
    pub fn optimize(self, schema: &Schema) -> Result<Predicate, ExpressionError> {
        match self {
            Predicate::Compare { column, op, value } => {
                let target = column_type(schema, &column)?;
                Ok(Predicate::Compare {
                    value: cast_to(value, &target)?,
                    column,
                    op,
                })
            }
            Predicate::Between { column, low, high } => {
                let target = column_type(schema, &column)?;
                Ok(Predicate::Between {
                    low: cast_to(low, &target)?,
                    high: cast_to(high, &target)?,
                    column,
                })
            }
            Predicate::In { column, values } => {
                let target = column_type(schema, &column)?;
                let values = values
                    .into_iter()
                    .map(|v| cast_to(v, &target))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Predicate::In { column, values })
            }
            Predicate::NotIn { column, values } => {
                let target = column_type(schema, &column)?;
                let values = values
                    .into_iter()
                    .map(|v| cast_to(v, &target))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Predicate::NotIn { column, values })
            }
            Predicate::IsNull(column) => {
                if schema.index_of(&column).is_none() {
                    return Err(ExpressionError::ColumnNotFound(column));
                }
                Ok(Predicate::IsNull(column))
            }
            Predicate::IsNotNull(column) => {
                if schema.index_of(&column).is_none() {
                    return Err(ExpressionError::ColumnNotFound(column));
                }
                Ok(Predicate::IsNotNull(column))
            }
            Predicate::And(a, b) => Ok(Predicate::And(
                Box::new(a.optimize(schema)?),
                Box::new(b.optimize(schema)?),
            )),
            Predicate::Or(a, b) => Ok(Predicate::Or(
                Box::new(a.optimize(schema)?),
                Box::new(b.optimize(schema)?),
            )),
            Predicate::Not(p) => Ok(Predicate::Not(Box::new(p.optimize(schema)?))),
        }
    }
}

impl Expression for Predicate {
    /// A predicate always yields a boolean.
    fn data_type(&self, _schema: &Schema) -> Result<DataType, ExpressionError> {
        Ok(PrimitiveType::Boolean.into())
    }

    fn columns(&self) -> Vec<String> {
        let mut out = Vec::new();
        collect_columns(self, &mut out);
        out
    }
}

/// Gathers the referenced column names (in order, with duplicates) from a tree.
fn collect_columns(predicate: &Predicate, out: &mut Vec<String>) {
    match predicate {
        Predicate::Compare { column, .. }
        | Predicate::Between { column, .. }
        | Predicate::In { column, .. }
        | Predicate::NotIn { column, .. }
        | Predicate::IsNull(column)
        | Predicate::IsNotNull(column) => out.push(column.clone()),
        Predicate::And(a, b) | Predicate::Or(a, b) => {
            collect_columns(a, out);
            collect_columns(b, out);
        }
        Predicate::Not(p) => collect_columns(p, out),
    }
}

/// Resolves the declared type of `column` in `schema`, or
/// [`ColumnNotFound`](ExpressionError::ColumnNotFound).
fn column_type(schema: &Schema, column: &str) -> Result<DataType, ExpressionError> {
    schema
        .field_by_name(column)
        .map(|f| f.data_type().clone())
        .ok_or_else(|| ExpressionError::ColumnNotFound(column.to_string()))
}

/// Casts a literal to `target` (a no-op when it already matches), logging the cast.
fn cast_to(value: Scalar, target: &DataType) -> Result<Scalar, ExpressionError> {
    if value.data_type() == target {
        return Ok(value);
    }
    log_event!(
        debug,
        "Predicate::optimize casting literal {value} -> {target}"
    );
    Ok(value.cast(target)?)
}

/// Walks an `AND` tree, pushing every non-`AND` leaf into `out` (left to right).
fn flatten_and(predicate: Predicate, out: &mut Vec<Predicate>) {
    match predicate {
        Predicate::And(a, b) => {
            flatten_and(*a, out);
            flatten_and(*b, out);
        }
        leaf => out.push(leaf),
    }
}

/// Folds, per column, a single `>=` lower bound and `<=` upper bound into one
/// inclusive [`Between`](Predicate::Between). Columns with anything other than
/// exactly one of each are left untouched (we cannot order scalar values here).
fn merge_bounds(parts: &mut Vec<Predicate>) {
    // Candidate columns: those with exactly one Ge and exactly one Le and no other
    // comparison/range touching them.
    let columns: Vec<String> = parts
        .iter()
        .filter_map(|p| match p {
            Predicate::Compare { column, .. } => Some(column.clone()),
            _ => None,
        })
        .collect();
    let mut seen = Vec::new();
    for column in columns {
        if seen.contains(&column) {
            continue;
        }
        seen.push(column.clone());
        let ge = parts
            .iter()
            .filter(|p| is_bound(p, &column, CompareOp::Ge))
            .count();
        let le = parts
            .iter()
            .filter(|p| is_bound(p, &column, CompareOp::Le))
            .count();
        let other = parts
            .iter()
            .filter(|p| {
                touches_column(p, &column)
                    && !is_bound(p, &column, CompareOp::Ge)
                    && !is_bound(p, &column, CompareOp::Le)
            })
            .count();
        if ge != 1 || le != 1 || other != 0 {
            continue;
        }
        let low = take_bound(parts, &column, CompareOp::Ge);
        let high = take_bound(parts, &column, CompareOp::Le);
        if let (Some(low), Some(high)) = (low, high) {
            parts.push(Predicate::Between { column, low, high });
        }
    }
}

/// Whether `p` is `column <op> _`.
fn is_bound(p: &Predicate, column: &str, op: CompareOp) -> bool {
    matches!(p, Predicate::Compare { column: c, op: o, .. } if c == column && *o == op)
}

/// Whether `p` is any comparison/range/membership on `column`.
fn touches_column(p: &Predicate, column: &str) -> bool {
    matches!(p,
        Predicate::Compare { column: c, .. }
        | Predicate::Between { column: c, .. }
        | Predicate::In { column: c, .. }
        | Predicate::NotIn { column: c, .. }
        if c == column)
}

/// Removes the first `column <op> value` from `parts`, returning its value.
fn take_bound(parts: &mut Vec<Predicate>, column: &str, op: CompareOp) -> Option<Scalar> {
    let pos = parts.iter().position(|p| is_bound(p, column, op))?;
    match parts.remove(pos) {
        Predicate::Compare { value, .. } => Some(value),
        _ => None,
    }
}

/// Rebuilds a left-associative `AND` chain from conjuncts (a single conjunct is
/// returned bare; an empty list is impossible from a non-trivial `AND`).
fn rebuild_and(mut parts: Vec<Predicate>) -> Predicate {
    let mut acc = parts.remove(0);
    for part in parts {
        acc = Predicate::And(Box::new(acc), Box::new(part));
    }
    acc
}

/// Renders a comma-separated list of scalar values for `IN` / `NOT IN`.
fn join_values(values: &[Scalar]) -> String {
    values
        .iter()
        .map(Scalar::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

impl fmt::Display for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Predicate::Compare { column, op, value } => write!(f, "{column} {op} {value}"),
            Predicate::Between { column, low, high } => {
                write!(f, "{column} BETWEEN {low} AND {high}")
            }
            Predicate::In { column, values } => write!(f, "{column} IN [{}]", join_values(values)),
            Predicate::NotIn { column, values } => {
                write!(f, "{column} NOT IN [{}]", join_values(values))
            }
            Predicate::IsNull(column) => write!(f, "{column} IS NULL"),
            Predicate::IsNotNull(column) => write!(f, "{column} IS NOT NULL"),
            Predicate::And(a, b) => write!(f, "({a} AND {b})"),
            Predicate::Or(a, b) => write!(f, "({a} OR {b})"),
            Predicate::Not(p) => write!(f, "NOT {p}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Field;

    fn schema() -> Schema {
        Schema::new(vec![
            Field::new(
                "ts",
                DataType::from_str("timestamp(ns, UTC)").unwrap(),
                false,
            ),
            Field::new("px", PrimitiveType::Float64.into(), true),
        ])
    }

    #[test]
    fn optimize_types_the_literal_against_the_column() {
        let p = Predicate::ge("ts", Scalar::any("2024-01-01"));
        let typed = p.optimize(&schema()).unwrap();
        match typed {
            Predicate::Compare { value, .. } => {
                assert!(value.data_type().is_temporal());
                assert_eq!(value.as_i64(), Some(19723 * 86_400 * 1_000_000_000));
            }
            _ => panic!("expected a comparison"),
        }
    }

    #[test]
    fn optimize_recurses_and_checks_columns() {
        let p = Predicate::ge("ts", Scalar::any("2024-01-01"))
            .and(Predicate::gt("px", Scalar::utf8("100")));
        let typed = p.optimize(&schema()).unwrap();
        // Both literals are now typed to their columns.
        if let Predicate::And(a, b) = typed {
            assert!(
                matches!(*a, Predicate::Compare { ref value, .. } if value.data_type().is_temporal())
            );
            assert!(
                matches!(*b, Predicate::Compare { ref value, .. } if value.data_type().is_numeric())
            );
        } else {
            panic!("expected AND");
        }
        // An unknown column is rejected.
        assert!(matches!(
            Predicate::eq("nope", Scalar::int64(1)).optimize(&schema()),
            Err(ExpressionError::ColumnNotFound(_))
        ));
    }

    #[test]
    fn columns_and_display() {
        let p = Predicate::ge("ts", Scalar::any("2024-01-01")).and(Predicate::is_null("px"));
        assert_eq!(p.columns(), ["ts", "px"]);
        assert!(p.to_string().contains("AND"));
    }

    #[test]
    fn in_casts_every_value() {
        // A collection of untyped values, all cast to the column type.
        let p = Predicate::is_in(
            "px",
            [Scalar::utf8("1"), Scalar::any("2"), Scalar::int64(3)],
        );
        assert_eq!(p.columns(), ["px"]);
        let typed = p.optimize(&schema()).unwrap();
        match typed {
            Predicate::In { values, .. } => {
                assert_eq!(values.len(), 3);
                assert!(values.iter().all(|v| v.data_type().is_numeric()));
            }
            _ => panic!("expected IN"),
        }
        assert!(Predicate::not_in("ts", [Scalar::any("2024-01-01")])
            .optimize(&schema())
            .is_ok());
    }

    #[test]
    fn between_casts_both_bounds() {
        let p = Predicate::between("ts", Scalar::any("2024-01-01"), Scalar::any("2024-02-01"));
        let typed = p.optimize(&schema()).unwrap();
        match typed {
            Predicate::Between { low, high, .. } => {
                assert!(low.data_type().is_temporal());
                assert!(high.data_type().is_temporal());
            }
            _ => panic!("expected BETWEEN"),
        }
    }

    #[test]
    fn merge_folds_bounds_into_between() {
        // ts >= a  AND  ts <= b  merges into a single BETWEEN.
        let lower = Predicate::ge("ts", Scalar::any("2024-01-01"));
        let upper = Predicate::le("ts", Scalar::any("2024-02-01"));
        let merged = lower.merge(upper);
        assert!(
            matches!(merged, Predicate::Between { ref column, .. } if column == "ts"),
            "{merged}"
        );
    }

    #[test]
    fn simplify_flattens_and_dedups() {
        // Duplicate conjuncts collapse; nested ANDs flatten.
        let p = Predicate::gt("px", Scalar::int64(1))
            .and(Predicate::gt("px", Scalar::int64(1)))
            .and(Predicate::is_not_null("ts"));
        let simplified = p.simplify();
        // px > 1 appears once, alongside ts IS NOT NULL → exactly one AND node.
        assert_eq!(
            simplified.to_string().matches("AND").count(),
            1,
            "{simplified}"
        );
    }

    #[test]
    fn merge_keeps_distinct_columns_separate() {
        // Bounds on different columns are not folded together.
        let merged = Predicate::ge("ts", Scalar::any("2024-01-01"))
            .merge(Predicate::le("px", Scalar::int64(100)));
        assert!(matches!(merged, Predicate::And(..)));
    }
}
