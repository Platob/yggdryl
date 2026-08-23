//! Structural serialization: an expression as data, not as a sentence.
//!
//! An expression already has a text form that round-trips
//! ([`Display`](std::fmt::Display) against
//! [`FromStr`](std::str::FromStr)), and for a log line or a URL that is the
//! right shape. This is the other one: the tree as a tagged document, the same
//! way [`DataType`](crate::DataType) and [`Field`](crate::Field) already cross
//! a wire in this crate.
//!
//! It exists because the two forms fail differently. Text is compact and a
//! person can read it, but a consumer has to hold a compatible grammar to get
//! anything out of it. The document is verbose and a consumer can walk it -
//! rewrite a column name, count the predicates, reject a function it does not
//! implement - without parsing anything.
//!
//! ```
//! use yggdryl::Expression;
//!
//! # fn main() -> yggdryl::Result<()> {
//! let filter: Expression = "ccy = 'EUR'".parse()?;
//! let document = filter.clone().into_json()?;
//! assert_eq!(Expression::from_json(&document)?, filter);
//! # Ok(())
//! # }
//! ```
//!
//! The shape is serde's externally tagged form, which is the one that survives
//! every variant this enum has: `{"column":"ccy"}`,
//! `{"compare":[{"column":"ccy"},"eq",{"literal":{…}}]}`. A literal carries its
//! own datatype, so a decimal stays a decimal across the wire exactly as it
//! does across the text form.

use super::Expression;
use super::parser::Statement;
use crate::{Error, Result};

impl Expression {
    /// Read an expression from its structural JSON document.
    ///
    /// # Errors
    ///
    /// Returns an error when the document is not a valid expression, or when
    /// the tree it describes is past the depth or node budget.
    pub fn from_json(input: &str) -> Result<Self> {
        let expression: Self = serde_json::from_str(input).map_err(Error::from)?;
        // A document arrives from outside, so it is budgeted the same way text
        // from outside is: the parser refuses a tree it cannot walk, and so
        // does this.
        expression.check_budget()?;
        Ok(expression)
    }

    /// Consume this expression and write it as a structural JSON document.
    ///
    /// # Errors
    ///
    /// Returns an error when the document cannot be produced.
    pub fn into_json(self) -> Result<String> {
        serde_json::to_string(&self).map_err(Error::from)
    }
}

impl Statement {
    /// Read a statement from its structural JSON document.
    ///
    /// # Errors
    ///
    /// Returns an error when the document is not a valid statement, or when
    /// any expression in it is past the depth or node budget.
    pub fn from_json(input: &str) -> Result<Self> {
        let statement: Self = serde_json::from_str(input).map_err(Error::from)?;
        if let Some(predicate) = statement.predicate() {
            predicate.check_budget()?;
        }
        for projection in statement.projections() {
            projection.expression().check_budget()?;
        }
        for order in statement.ordering() {
            order.expression().check_budget()?;
        }
        Ok(statement)
    }

    /// Consume this statement and write it as a structural JSON document.
    ///
    /// # Errors
    ///
    /// Returns an error when the document cannot be produced.
    pub fn into_json(self) -> Result<String> {
        serde_json::to_string(&self).map_err(Error::from)
    }
}
