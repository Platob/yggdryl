//! Selection: the ordered projection of named expressions a read produces.
//!
//! A selection of bare columns is exactly what `select_by_names` already
//! selects, and produces exactly the root [`Field`] it already produces - that
//! identity is a test, because it is what lets the existing surface become
//! sugar without changing an answer. A selection that computes something adds
//! the column its expression names, evaluated *after* the encoding's own
//! projection of the columns it reads, never instead of it.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::bound::Bound;
use super::{Expr, write_identifier};
use crate::{DataType, Error, Field, Result, Value};

/// One selected expression, with the name its column will carry.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct SelectionItem {
    /// What is computed.
    expr: Expr,
    /// The name it is given, when the caller named one.
    alias: Option<SmolStr>,
}

impl SelectionItem {
    /// Select an expression under the name it already carries.
    #[must_use]
    pub fn new(expr: Expr) -> Self {
        match expr {
            Expr::Alias { expr, name } => Self {
                expr: (*expr).clone(),
                alias: Some(name),
            },
            other => Self {
                expr: other,
                alias: None,
            },
        }
    }

    /// Select an expression under an explicit name.
    #[must_use]
    pub fn aliased(expr: Expr, alias: impl Into<SmolStr>) -> Self {
        Self {
            expr,
            alias: Some(alias.into()),
        }
    }

    /// Borrow what is computed.
    #[must_use]
    #[inline]
    pub const fn expr(&self) -> &Expr {
        &self.expr
    }

    /// The name this column will carry.
    ///
    /// An alias when one was written, and the expression's own canonical
    /// spelling otherwise - which is SQL's rule and what makes a computed
    /// column addressable without the caller having to name it.
    #[must_use]
    pub fn name(&self) -> String {
        self.alias
            .as_ref()
            .map_or_else(|| self.expr.to_string(), ToString::to_string)
    }

    /// Return whether this item is a bare column with no computation.
    #[must_use]
    pub fn is_bare_column(&self) -> bool {
        self.alias.is_none()
            && self
                .expr
                .as_column()
                .is_some_and(|column| column.path().is_empty())
    }
}

impl fmt::Display for SelectionItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.expr)?;
        if let Some(alias) = &self.alias {
            formatter.write_str(" AS ")?;
            write_identifier(formatter, alias)?;
        }
        Ok(())
    }
}

/// An ordered projection: what a read yields and in what order.
///
/// An empty selection selects everything, which is why [`Default`] is
/// implemented for this type and deliberately not for [`Expr`] - a defaulted
/// projection is "no narrowing", while a defaulted predicate would be an
/// always-true filter arriving by accident.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct Selection {
    items: Arc<[SelectionItem]>,
}

impl Selection {
    /// The selection that narrows nothing.
    #[must_use]
    pub fn everything() -> Self {
        Self {
            items: Arc::from([]),
        }
    }

    /// Select these expressions, in this order.
    #[must_use]
    pub fn new(items: impl IntoIterator<Item = SelectionItem>) -> Self {
        Self {
            items: items.into_iter().collect(),
        }
    }

    /// Select these expressions, taking each one's own name.
    #[must_use]
    pub fn from_exprs(exprs: impl IntoIterator<Item = Expr>) -> Self {
        Self::new(exprs.into_iter().map(SelectionItem::new))
    }

    /// Select these columns by name, which is what `select_by_names` selects.
    #[must_use]
    pub fn from_names<N: AsRef<str>>(names: impl IntoIterator<Item = N>) -> Self {
        Self::from_exprs(
            names
                .into_iter()
                .map(|name| Expr::column(name.as_ref().to_owned())),
        )
    }

    /// Borrow the items, in order.
    #[must_use]
    #[inline]
    pub fn items(&self) -> &[SelectionItem] {
        &self.items
    }

    /// How many columns this selection produces.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Return whether this selection narrows nothing.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Return whether every item is a bare column.
    ///
    /// A projection built only of bare columns still pushes down as a
    /// projection - a Parquet mask, an IPC projection - exactly as today, so
    /// the encodings need this answer before they decide what to decode.
    #[must_use]
    pub fn is_projection(&self) -> bool {
        self.items.iter().all(SelectionItem::is_bare_column)
    }

    /// The column names this selection reads, deduplicated in first-seen order.
    ///
    /// This is the set an encoding's own projection is narrowed to, and it is
    /// deliberately the columns *read*, not the columns produced: a computed
    /// column reads what it computes from.
    #[must_use]
    pub fn columns(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for item in self.items.iter() {
            for name in item.expr.columns() {
                if !names.iter().any(|held| held.eq_ignore_ascii_case(&name)) {
                    names.push(name);
                }
            }
        }
        names
    }

    /// Return this selection with more items appended.
    #[must_use]
    pub fn then(&self, more: impl IntoIterator<Item = SelectionItem>) -> Self {
        let mut items: Vec<SelectionItem> = self.items.to_vec();
        items.extend(more);
        Self::new(items)
    }

    /// Bind every item against a struct root.
    ///
    /// # Errors
    ///
    /// Returns an error naming the columns the schema does have when an item
    /// names one it does not, or naming both names when two items would
    /// produce the same column.
    pub fn bind(&self, schema: &Field) -> Result<BoundSelection> {
        BoundSelection::new(self, schema)
    }
}

impl fmt::Display for Selection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.items.is_empty() {
            return formatter.write_str("*");
        }
        for (position, item) in self.items.iter().enumerate() {
            if position > 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{item}")?;
        }
        Ok(())
    }
}

impl FromStr for Selection {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self> {
        if text.trim() == "*" {
            return Ok(Self::everything());
        }
        Ok(Self::from_exprs(super::parser::parse_selection(text)?))
    }
}

impl TryFrom<&str> for Selection {
    type Error = Error;

    fn try_from(text: &str) -> Result<Self> {
        text.parse()
    }
}

impl FromIterator<SelectionItem> for Selection {
    fn from_iter<I: IntoIterator<Item = SelectionItem>>(items: I) -> Self {
        Self::new(items)
    }
}

impl FromIterator<Expr> for Selection {
    fn from_iter<I: IntoIterator<Item = Expr>>(exprs: I) -> Self {
        Self::from_exprs(exprs)
    }
}

impl<'items> IntoIterator for &'items Selection {
    type Item = &'items SelectionItem;
    type IntoIter = std::slice::Iter<'items, SelectionItem>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

/// A selection resolved against a schema: one plan per produced column.
#[derive(Clone, Debug)]
pub struct BoundSelection {
    items: Vec<Bound>,
    root: Field,
    schema: Field,
}

impl BoundSelection {
    /// Bind every item of a selection.
    fn new(selection: &Selection, schema: &Field) -> Result<Self> {
        schema.require_struct()?;
        if selection.is_empty() {
            // Selecting everything produces the schema untouched, which is
            // what makes an absent selection cost nothing at all.
            return Ok(Self {
                items: Vec::new(),
                root: schema.clone(),
                schema: schema.clone(),
            });
        }
        let mut items = Vec::with_capacity(selection.len());
        let mut fields: Vec<Field> = Vec::with_capacity(selection.len());
        for item in selection.items() {
            let bound = item.expr.bind(schema)?;
            let name = item.name();
            if let Some(clash) = fields
                .iter()
                .find(|held| held.name().eq_ignore_ascii_case(&name))
            {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new(name.clone()),
                    reason: smol_str::format_smolstr!(
                        "expected each selected column to name a distinct column, got {name:?} twice (already {:?})",
                        clash.name()
                    ),
                });
            }
            // A bare column keeps the field it selected, metadata, field id,
            // and nullability included, which is what makes a bare-column
            // selection byte-identical to today's `select_by_names`.
            let field = match bound.bound_columns().first() {
                Some(column) if item.is_bare_column() => {
                    column.root_field().clone().with_name(name)
                }
                _ => Field::new(name, bound.data_type(), true),
            };
            fields.push(field);
            items.push(bound);
        }
        let root = DataType::from_fields(fields)?.required_field(schema.name());
        Ok(Self {
            items,
            root,
            schema: schema.clone(),
        })
    }

    /// The struct root this selection produces.
    #[must_use]
    #[inline]
    pub const fn root(&self) -> &Field {
        &self.root
    }

    /// The struct root this selection was bound against.
    #[must_use]
    #[inline]
    pub const fn schema(&self) -> &Field {
        &self.schema
    }

    /// One plan per produced column, in order.
    #[must_use]
    #[inline]
    pub fn items(&self) -> &[Bound] {
        &self.items
    }

    /// Return whether this selection narrows nothing.
    #[must_use]
    #[inline]
    pub fn is_everything(&self) -> bool {
        self.items.is_empty()
    }

    /// Every stored column this selection reads.
    #[must_use]
    pub fn columns(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for item in &self.items {
            for name in item.columns() {
                if !names.iter().any(|held| held.eq_ignore_ascii_case(&name)) {
                    names.push(name);
                }
            }
        }
        names
    }

    /// Project one row into the row this selection produces.
    ///
    /// # Errors
    ///
    /// Returns whatever evaluating one of the items returns.
    pub fn evaluate(&self, row: &Value) -> Result<Value> {
        if self.items.is_empty() {
            return Ok(row.clone());
        }
        let mut values = Vec::with_capacity(self.items.len());
        for item in &self.items {
            values.push(item.evaluate(row)?);
        }
        Value::record(self.root.data_type().clone(), values)
    }
}

impl fmt::Display for BoundSelection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.items.is_empty() {
            return formatter.write_str("*");
        }
        for (position, item) in self.items.iter().enumerate() {
            if position > 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{item}")?;
        }
        Ok(())
    }
}
