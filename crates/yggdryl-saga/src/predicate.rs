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

/// A boolean predicate over a frame's columns: a leaf comparison or null check,
/// combined with `and` / `or` / `not`.
///
/// A comparison holds a column name, an operator and a literal [`Scalar`]. The
/// literal usually starts **untyped** ([`Any`](DataType::Any)) — e.g.
/// `col("ts") > "2024-01-01"`; [`optimize`](Predicate::optimize) then resolves the
/// column's type from the schema and casts the literal to it, so the comparison is
/// typed and pushable.
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

    /// Type-optimises the predicate against `schema`: each comparison's literal is
    /// cast to the type of the column it is compared with (so an untyped
    /// [`Any`](DataType::Any) or string value becomes a typed one), and every
    /// referenced column is checked to exist. This is the transform that makes a
    /// filter pushable into typed storage.
    pub fn optimize(self, schema: &Schema) -> Result<Predicate, ExpressionError> {
        match self {
            Predicate::Compare { column, op, value } => {
                let target = schema
                    .field_by_name(&column)
                    .map(|f| f.data_type().clone())
                    .ok_or_else(|| ExpressionError::ColumnNotFound(column.clone()))?;
                let value = if value.data_type() == &target {
                    value
                } else {
                    log_event!(
                        debug,
                        "Predicate::optimize casting {} literal {} -> {target}",
                        column,
                        value
                    );
                    value.cast(&target)?
                };
                Ok(Predicate::Compare { column, op, value })
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
        | Predicate::IsNull(column)
        | Predicate::IsNotNull(column) => out.push(column.clone()),
        Predicate::And(a, b) | Predicate::Or(a, b) => {
            collect_columns(a, out);
            collect_columns(b, out);
        }
        Predicate::Not(p) => collect_columns(p, out),
    }
}

impl fmt::Display for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Predicate::Compare { column, op, value } => write!(f, "{column} {op} {value}"),
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
}
