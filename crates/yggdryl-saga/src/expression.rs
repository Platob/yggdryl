//! The [`Expression`] trait and its leaf nodes — a [`Col`] reference and a [`Lit`]
//! literal. An expression knows the columns it touches and the [`DataType`] it
//! yields against a [`Schema`], which is what lets a [`Predicate`](crate::Predicate)
//! type-check and cast its literals for pushdown.

use std::fmt;

use crate::cast::CastError;
use crate::{DataType, Scalar, Schema};

/// Error returned while resolving or optimising an [`Expression`] /
/// [`Predicate`](crate::Predicate).
#[derive(Debug, Clone, PartialEq)]
pub enum ExpressionError {
    /// A referenced column is absent from the schema.
    ColumnNotFound(String),
    /// A literal could not be cast to the column's type.
    Cast(CastError),
    /// The expression is not type-correct (e.g. an undefined comparison).
    Type(String),
}

impl fmt::Display for ExpressionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExpressionError::ColumnNotFound(name) => write!(f, "column '{name}' not found"),
            ExpressionError::Cast(err) => write!(f, "{err}"),
            ExpressionError::Type(detail) => write!(f, "type error: {detail}"),
        }
    }
}

impl std::error::Error for ExpressionError {}

impl From<CastError> for ExpressionError {
    fn from(err: CastError) -> ExpressionError {
        ExpressionError::Cast(err)
    }
}

/// A node in an expression tree.
///
/// An expression resolves to a [`DataType`] against a schema and reports the
/// columns it references — the two facts the engine needs to type-check a filter
/// and decide projection/predicate pushdown.
pub trait Expression {
    /// The type this expression yields when evaluated against `schema`.
    fn data_type(&self, schema: &Schema) -> Result<DataType, ExpressionError>;

    /// The names of the columns this expression references.
    fn columns(&self) -> Vec<String>;
}

/// A reference to a column by name (`col("price")`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Col {
    name: String,
}

impl Col {
    /// References the column named `name`.
    pub fn new(name: impl Into<String>) -> Col {
        Col { name: name.into() }
    }

    /// The referenced column name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Expression for Col {
    fn data_type(&self, schema: &Schema) -> Result<DataType, ExpressionError> {
        schema
            .field_by_name(&self.name)
            .map(|f| f.data_type().clone())
            .ok_or_else(|| ExpressionError::ColumnNotFound(self.name.clone()))
    }

    fn columns(&self) -> Vec<String> {
        vec![self.name.clone()]
    }
}

/// A literal value (`lit(Scalar::int64(42))`).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Lit {
    value: Scalar,
}

impl Lit {
    /// Wraps a [`Scalar`] as a literal expression.
    pub fn new(value: Scalar) -> Lit {
        Lit { value }
    }

    /// The wrapped scalar.
    pub fn value(&self) -> &Scalar {
        &self.value
    }
}

impl Expression for Lit {
    fn data_type(&self, _schema: &Schema) -> Result<DataType, ExpressionError> {
        Ok(self.value.data_type().clone())
    }

    fn columns(&self) -> Vec<String> {
        Vec::new()
    }
}

/// References a column by name — the terse constructor for [`Col`].
pub fn col(name: impl Into<String>) -> Col {
    Col::new(name)
}

/// Wraps a value as a literal — the terse constructor for [`Lit`].
pub fn lit(value: Scalar) -> Lit {
    Lit::new(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Field, PrimitiveType};

    fn schema() -> Schema {
        Schema::new(vec![
            Field::new(
                "ts",
                crate::DataType::from_str("timestamp(ns, UTC)").unwrap(),
                false,
            ),
            Field::new("px", PrimitiveType::Float64.into(), true),
        ])
    }

    #[test]
    fn col_resolves_type_from_schema() {
        let c = col("px");
        assert_eq!(c.columns(), ["px"]);
        assert_eq!(
            c.data_type(&schema()).unwrap(),
            DataType::from(PrimitiveType::Float64)
        );
        assert!(matches!(
            col("nope").data_type(&schema()),
            Err(ExpressionError::ColumnNotFound(_))
        ));
    }

    #[test]
    fn lit_reports_its_own_type() {
        let l = lit(Scalar::any("2024-01-01"));
        assert!(l.columns().is_empty());
        assert!(l.data_type(&schema()).unwrap().is_any());
    }
}
