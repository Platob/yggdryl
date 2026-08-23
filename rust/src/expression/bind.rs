//! The compile step: one [`Expression`] and one schema become one [`Bound`].
//!
//! Everything that can be decided before the first row is decided here, once:
//!
//! * every parameter is substituted, so nothing is late-bound during a scan;
//! * every column name becomes an index into the schema, so no row lookup ever
//!   compares a string;
//! * every literal is converted into the type it is compared against, so
//!   `price > 100` on a `decimal(9,2)` column is an exact decimal comparison
//!   rather than a comparison between two different kinds of number;
//! * every subtree whose operands are all constant is *evaluated* and replaced
//!   by its result - folding is not a second interpreter, it is the same one
//!   run early;
//! * every pattern is checked to be constant, because a pattern that changes
//!   per row is a different operation and pretending otherwise would make the
//!   vectorized tier silently slower than the scalar one;
//! * the operands of every `and` and `or` are ordered cheapest-first, so a
//!   free attribute test runs before a stat and a stat runs before a decode.
//!
//! What comes out is a resolved tree the three evaluators walk. They share it,
//! which is the mechanism - not the intention - behind scalar and vectorized
//! agreeing.

use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::eval::{Row, convert};
use super::parser::{Direction, NullsOrder, Statement};
use super::selector::{Attributes, Cost, Selector};
use super::typing::common_type;
use super::{Comparison, Expression, Function, Operator, Safety, Segment};
use crate::{DataType, Error, Field, Result, Scalar, TypedScalar};

/// What one node costs to answer, in units of "a free attribute read".
///
/// The numbers are ordinals, not measurements: what matters is that a stat
/// outranks every free attribute and a column decode outranks a stat, because
/// that is the order in which a reader would rather be wrong.
const COST_FREE_ATTRIBUTE: u32 = 1;
const COST_STAT: u32 = 64;
const COST_COLUMN: u32 = 1024;

/// A resolved node: an output field, a resolved operation, and a cost.
#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub(crate) field: Field,
    pub(crate) kind: Kind,
    pub(crate) cost: u32,
}

/// The resolved form of every [`Expression`] variant.
#[derive(Clone, Debug)]
pub(crate) enum Kind {
    /// A constant, already in this node's declared datatype.
    Literal(Scalar),
    /// A column, by index into the bound schema.
    Column(usize),
    /// A path into a value.
    Path(Box<Node>, Arc<[Segment]>),
    /// A holder attribute.
    Attribute(Selector),
    /// Conjunction, operands ordered cheapest-first.
    And(Vec<Node>),
    /// Disjunction, operands ordered cheapest-first.
    Or(Vec<Node>),
    /// Three-valued negation.
    Not(Box<Node>),
    /// A comparison whose operands are already in one type.
    Compare(Box<Node>, Comparison, Box<Node>),
    /// Set membership.
    In(Box<Node>, Vec<Node>),
    /// An inclusive range test.
    Between(Box<Node>, Box<Node>, Box<Node>),
    /// A null test.
    IsNull(Box<Node>),
    /// A not-null test.
    IsNotNull(Box<Node>),
    /// A `like` with a constant pattern.
    Like {
        /// The text matched.
        value: Box<Node>,
        /// The constant pattern.
        pattern: SmolStr,
        /// Whether the match ignores case.
        case_insensitive: bool,
        /// The wildcard escape, when the clause named one.
        escape: Option<char>,
    },
    /// A glob with a constant pattern.
    Glob(Box<Node>, SmolStr),
    /// Arithmetic in one type.
    Arithmetic(Box<Node>, Operator, Box<Node>),
    /// Arithmetic negation.
    Negate(Box<Node>),
    /// A call into the closed function set.
    Function(Function, Vec<Node>),
    /// A conversion into this node's declared datatype.
    Cast(Box<Node>, Safety),
    /// A searched conditional.
    Case {
        /// The `when`/`then` pairs, in order.
        branches: Vec<(Node, Node)>,
        /// The `else` value.
        otherwise: Option<Box<Node>>,
    },
    /// A struct built from its children, in declared order.
    Struct(Vec<Node>),
    /// A list built from its elements.
    List(Vec<Node>),
    /// A map built from its entries.
    Map(Vec<(Node, Node)>),
}

impl Node {
    /// Return whether this node is a constant.
    pub(crate) const fn as_literal(&self) -> Option<&Scalar> {
        match &self.kind {
            Kind::Literal(value) => Some(value),
            _ => None,
        }
    }

    /// The column index this node reads, when it reads exactly one directly.
    pub(crate) const fn as_column(&self) -> Option<usize> {
        match &self.kind {
            Kind::Column(index) => Some(*index),
            _ => None,
        }
    }

    /// Visit every direct child of this node.
    pub(crate) fn for_each_child<'node>(&'node self, mut visit: impl FnMut(&'node Self)) {
        match &self.kind {
            Kind::Literal(_) | Kind::Column(_) | Kind::Attribute(_) => {}
            Kind::Path(base, _) => visit(base),
            Kind::And(operands)
            | Kind::Or(operands)
            | Kind::In(_, operands)
            | Kind::Function(_, operands)
            | Kind::Struct(operands)
            | Kind::List(operands) => {
                if let Kind::In(value, _) = &self.kind {
                    visit(value);
                }
                operands.iter().for_each(visit);
            }
            Kind::Not(inner)
            | Kind::IsNull(inner)
            | Kind::IsNotNull(inner)
            | Kind::Negate(inner)
            | Kind::Cast(inner, _)
            | Kind::Glob(inner, _)
            | Kind::Like { value: inner, .. } => visit(inner),
            Kind::Compare(left, _, right) | Kind::Arithmetic(left, _, right) => {
                visit(left);
                visit(right);
            }
            Kind::Between(value, low, high) => {
                visit(value);
                visit(low);
                visit(high);
            }
            Kind::Case {
                branches,
                otherwise,
            } => {
                for (when, then) in branches {
                    visit(when);
                    visit(then);
                }
                if let Some(otherwise) = otherwise {
                    visit(otherwise);
                }
            }
            Kind::Map(entries) => {
                for (key, value) in entries {
                    visit(key);
                    visit(value);
                }
            }
        }
    }

    /// Return whether this node reads any column of the row.
    pub(crate) fn reads_rows(&self) -> bool {
        if matches!(self.kind, Kind::Column(_)) {
            return true;
        }
        let mut found = false;
        self.for_each_child(|child| found |= child.reads_rows());
        found
    }

    /// Every column index this node reads, deduplicated in ascending order.
    pub(crate) fn column_indices(&self) -> Vec<usize> {
        let mut found = Vec::new();
        self.collect_columns(&mut found);
        found.sort_unstable();
        found.dedup();
        found
    }

    fn collect_columns(&self, found: &mut Vec<usize>) {
        if let Kind::Column(index) = self.kind {
            found.push(index);
        }
        self.for_each_child(|child| child.collect_columns(found));
    }

    /// The top-level `and` operands of this node, already ordered.
    pub(crate) fn conjuncts(&self) -> Vec<&Self> {
        match &self.kind {
            Kind::And(operands) => operands.iter().collect(),
            _ => vec![self],
        }
    }
}

/// One expression, resolved against one schema, ready to answer.
///
/// A `Bound` is built once per stream and answers three ways: row at a time
/// over [`Scalar`], vectorized over an Arrow batch, and three-valued over
/// container statistics. All three walk the same resolved tree.
#[derive(Clone, Debug)]
pub struct Bound {
    schema: Field,
    expression: Expression,
    node: Node,
}

impl Bound {
    /// The struct root this expression was bound against.
    #[must_use]
    pub const fn schema(&self) -> &Field {
        &self.schema
    }

    /// The expression as it stands after substitution, folding, and ordering.
    ///
    /// This is what a log line should print: it is the plan that will actually
    /// run, not the text the caller wrote.
    #[must_use]
    pub const fn expression(&self) -> &Expression {
        &self.expression
    }

    /// The output field this expression produces.
    #[must_use]
    pub const fn field(&self) -> &Field {
        &self.node.field
    }

    /// Return whether this expression answers a boolean.
    #[must_use]
    pub fn is_predicate(&self) -> bool {
        matches!(
            self.node.field.data_type(),
            DataType::Boolean | DataType::Null
        )
    }

    /// The schema column indices this expression reads, ascending.
    ///
    /// This is projection pushdown: a reader decodes these and no others.
    #[must_use]
    pub fn column_indices(&self) -> Vec<usize> {
        self.node.column_indices()
    }

    /// The schema column names this expression reads, in index order.
    #[must_use]
    pub fn column_names(&self) -> Vec<String> {
        let fields = self.schema.fields();
        self.column_indices()
            .into_iter()
            .filter_map(|index| fields.get(index).map(|field| field.name().to_owned()))
            .collect()
    }

    /// Return whether answering this expression requires reading rows.
    #[must_use]
    pub fn reads_rows(&self) -> bool {
        self.node.reads_rows()
    }

    /// The resolved tree, for the evaluators in this module.
    pub(crate) const fn node(&self) -> &Node {
        &self.node
    }

    /// Evaluate this expression for one row.
    ///
    /// The row is a [`Scalar::Sequence`] of column values in schema order. This
    /// is the row target's [`ApplyExpression`](super::ApplyExpression), spelled
    /// from the expression's side.
    ///
    /// # Errors
    ///
    /// Returns an error when the row does not match the bound schema, a strict
    /// cast refuses a value, or checked arithmetic overflows, divides by zero,
    /// or cannot represent an exact decimal result.
    pub fn eval(&self, row: &Scalar) -> Result<Scalar> {
        super::ApplyExpression::apply_expression(row, self)
    }

    /// Evaluate this expression for one row alongside a holder.
    ///
    /// # Errors
    ///
    /// Returns an error when the row does not match, or the holder cannot
    /// answer an attribute it is asked for.
    pub fn eval_with(&self, row: &Scalar, holder: &dyn Attributes) -> Result<Scalar> {
        let values = row_values(row, &self.schema)?;
        self.node.eval(&Row::new(Some(values), Some(holder)))
    }

    /// Answer this predicate for one row, reading unknown as "no".
    ///
    /// SQL keeps a row when the predicate is true, and unknown is not true.
    ///
    /// # Errors
    ///
    /// Returns an error when the row does not match the bound schema.
    pub fn matches(&self, row: &Scalar) -> Result<bool> {
        Ok(self.eval(row)?.as_bool().unwrap_or(false))
    }

    /// Answer this predicate for one row alongside a holder.
    ///
    /// # Errors
    ///
    /// Returns an error when the row does not match, or the holder fails.
    pub fn matches_with(&self, row: &Scalar, holder: &dyn Attributes) -> Result<bool> {
        Ok(self.eval_with(row, holder)?.as_bool().unwrap_or(false))
    }

    /// Return whether a holder is *not ruled out* by this predicate.
    ///
    /// The holder target's [`ApplyExpression`](super::ApplyExpression) answers
    /// three-valued - what the holder alone settles - and this reads its
    /// answer conservatively: only a proven `false` excludes, so an unknown
    /// keeps the holder. It may keep a file the rows will later discard, and
    /// it may never discard a file that would have matched.
    ///
    /// # Errors
    ///
    /// Returns the holder's failure when a stat attribute cannot be read.
    pub fn matches_holder(&self, holder: &dyn Attributes) -> Result<bool> {
        Ok(super::ApplyExpression::apply_expression(holder, self)?.as_bool() != Some(false))
    }
}

/// Borrow one row's column values.
pub(crate) fn row_values<'row>(row: &'row Scalar, schema: &Field) -> Result<&'row [Scalar]> {
    let values = row.as_sequence().ok_or_else(|| Error::InvalidRecord {
        path: SmolStr::new(schema.name()),
        reason: format_smolstr!(
            "expected an ordered sequence of {} column values, got {}",
            schema.field_len(),
            row.kind()
        ),
    })?;
    if values.len() != schema.field_len() {
        return Err(Error::InvalidRecord {
            path: SmolStr::new(schema.name()),
            reason: format_smolstr!(
                "expected {} column values, got {}",
                schema.field_len(),
                values.len()
            ),
        });
    }
    Ok(values)
}

impl std::fmt::Display for Bound {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.expression)
    }
}

/// A [`Statement`] resolved against one schema.
#[derive(Clone, Debug)]
pub struct BoundStatement {
    schema: Field,
    output: Field,
    projections: Vec<Bound>,
    predicate: Option<Bound>,
    ordering: Vec<(Bound, Direction, Option<NullsOrder>)>,
    limit: Option<u64>,
}

impl BoundStatement {
    /// The struct root this statement was bound against.
    #[must_use]
    pub const fn schema(&self) -> &Field {
        &self.schema
    }

    /// The struct root this statement produces.
    #[must_use]
    pub const fn output(&self) -> &Field {
        &self.output
    }

    /// The bound projections, in output order. Empty means every column.
    #[must_use]
    pub fn projections(&self) -> &[Bound] {
        &self.projections
    }

    /// The bound predicate, when the statement had one.
    #[must_use]
    pub const fn predicate(&self) -> Option<&Bound> {
        self.predicate.as_ref()
    }

    /// The bound ordering keys, in priority order.
    #[must_use]
    pub fn ordering(&self) -> &[(Bound, Direction, Option<NullsOrder>)] {
        &self.ordering
    }

    /// The row limit, when the statement had one.
    #[must_use]
    pub const fn limit(&self) -> Option<u64> {
        self.limit
    }

    /// Return whether this statement selects every column unchanged.
    #[must_use]
    pub fn is_all(&self) -> bool {
        self.projections.is_empty()
    }
}

impl Expression {
    /// Resolve this expression against a schema.
    ///
    /// # Errors
    ///
    /// Returns an error when a column is unknown, two operands share no type,
    /// a pattern is not constant, or a parameter was not supplied.
    pub fn bind(&self, schema: &Field) -> Result<Bound> {
        self.bind_with(schema, &[])
    }

    /// Resolve this expression against a schema, supplying its parameters.
    ///
    /// # Errors
    ///
    /// Returns an error when a parameter is missing or the expression cannot
    /// be resolved.
    pub fn bind_with(&self, schema: &Field, parameters: &[(&str, Scalar)]) -> Result<Bound> {
        schema.require_struct()?;
        self.check_budget()?;
        let supplied = substitute(self, parameters)?;
        let binder = Binder { schema };
        let node = binder.lower(&supplied, None)?;
        Ok(Bound {
            schema: schema.clone(),
            expression: rebuild(&node),
            node,
        })
    }
}

impl Statement {
    /// Resolve this statement against a schema.
    ///
    /// # Errors
    ///
    /// Returns an error when any projection, the predicate, or an ordering key
    /// cannot be resolved.
    pub fn bind(&self, schema: &Field) -> Result<BoundStatement> {
        self.bind_with(schema, &[])
    }

    /// Resolve this statement against a schema, supplying its parameters.
    ///
    /// # Errors
    ///
    /// Returns an error when a parameter is missing or a part cannot resolve.
    pub fn bind_with(
        &self,
        schema: &Field,
        parameters: &[(&str, Scalar)],
    ) -> Result<BoundStatement> {
        schema.require_struct()?;
        let mut projections = Vec::with_capacity(self.projections().len());
        let mut fields = Vec::with_capacity(self.projections().len());
        for projection in self.projections() {
            let bound = projection.expression().bind_with(schema, parameters)?;
            fields.push(bound.field().clone().with_name(projection.name()));
            projections.push(bound);
        }
        let output = if fields.is_empty() {
            schema.clone()
        } else {
            schema
                .clone()
                .try_with_data_type(DataType::from_fields(fields)?)?
        };
        let predicate = match self.predicate() {
            Some(predicate) => {
                let bound = predicate.bind_with(schema, parameters)?;
                if !bound.is_predicate() {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::new_static("$"),
                        reason: format_smolstr!(
                            "expected a boolean `where` clause, got {}",
                            bound.field().data_type()
                        ),
                    });
                }
                Some(bound)
            }
            None => None,
        };
        let mut ordering = Vec::with_capacity(self.ordering().len());
        for order in self.ordering() {
            ordering.push((
                order.expression().bind_with(schema, parameters)?,
                order.direction(),
                order.nulls(),
            ));
        }
        Ok(BoundStatement {
            schema: schema.clone(),
            output,
            projections,
            predicate,
            ordering,
            limit: self.limit(),
        })
    }
}

/// Replace every parameter with the value supplied for it.
fn substitute(expression: &Expression, parameters: &[(&str, Scalar)]) -> Result<Expression> {
    if parameters.is_empty() && expression.parameters().is_empty() {
        return Ok(expression.clone());
    }
    map_children(expression, &mut |node| match node {
        Expression::Parameter(name) => {
            let supplied = parameters
                .iter()
                .find(|(held, _)| held.eq_ignore_ascii_case(name))
                .ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected a value for parameter :{name}"),
                })?;
            Ok(Some(Expression::literal(supplied.1.clone())))
        }
        _ => Ok(None),
    })
}

/// Rebuild an expression, letting `replace` swap any node for another.
///
/// Written once so a rewrite never has to re-list twenty-three variants.
fn map_children(
    expression: &Expression,
    replace: &mut dyn FnMut(&Expression) -> Result<Option<Expression>>,
) -> Result<Expression> {
    if let Some(replaced) = replace(expression)? {
        return Ok(replaced);
    }
    // Taking the callback as a trait object rather than a generic is what keeps
    // this one function instead of one instantiation per nesting depth.
    let mapped = map_children;
    Ok(match expression {
        Expression::Literal(_)
        | Expression::Column(_)
        | Expression::Attribute(_)
        | Expression::Parameter(_) => expression.clone(),
        Expression::Path(base, steps) => {
            Expression::Path(Box::new(mapped(base, replace)?), steps.clone())
        }
        Expression::And(operands) => Expression::And(map_slice(operands, replace)?),
        Expression::Or(operands) => Expression::Or(map_slice(operands, replace)?),
        Expression::Not(inner) => Expression::Not(Box::new(mapped(inner, replace)?)),
        Expression::Compare(left, comparison, right) => Expression::Compare(
            Box::new(mapped(left, replace)?),
            *comparison,
            Box::new(mapped(right, replace)?),
        ),
        Expression::In(value, list) => {
            Expression::In(Box::new(mapped(value, replace)?), map_slice(list, replace)?)
        }
        Expression::Between(value, low, high) => Expression::Between(
            Box::new(mapped(value, replace)?),
            Box::new(mapped(low, replace)?),
            Box::new(mapped(high, replace)?),
        ),
        Expression::IsNull(inner) => Expression::IsNull(Box::new(mapped(inner, replace)?)),
        Expression::IsNotNull(inner) => Expression::IsNotNull(Box::new(mapped(inner, replace)?)),
        Expression::Like {
            value,
            pattern,
            case_insensitive,
            escape,
        } => Expression::Like {
            value: Box::new(mapped(value, replace)?),
            pattern: Box::new(mapped(pattern, replace)?),
            case_insensitive: *case_insensitive,
            escape: *escape,
        },
        Expression::Glob(value, pattern) => Expression::Glob(
            Box::new(mapped(value, replace)?),
            Box::new(mapped(pattern, replace)?),
        ),
        Expression::Arithmetic(left, operator, right) => Expression::Arithmetic(
            Box::new(mapped(left, replace)?),
            *operator,
            Box::new(mapped(right, replace)?),
        ),
        Expression::Negate(inner) => Expression::Negate(Box::new(mapped(inner, replace)?)),
        Expression::Function(function, arguments) => {
            Expression::Function(*function, map_slice(arguments, replace)?)
        }
        Expression::Cast(inner, data_type, safety) => Expression::Cast(
            Box::new(mapped(inner, replace)?),
            data_type.clone(),
            *safety,
        ),
        Expression::Case {
            branches,
            otherwise,
        } => {
            let mut mapped_branches = Vec::with_capacity(branches.len());
            for (when, then) in branches.iter() {
                mapped_branches.push((mapped(when, replace)?, mapped(then, replace)?));
            }
            Expression::Case {
                branches: Arc::from(mapped_branches),
                otherwise: match otherwise {
                    Some(otherwise) => Some(Box::new(mapped(otherwise, replace)?)),
                    None => None,
                },
            }
        }
        Expression::Struct(children) => {
            let mut mapped_children = Vec::with_capacity(children.len());
            for (name, value) in children.iter() {
                mapped_children.push((name.clone(), mapped(value, replace)?));
            }
            Expression::Struct(Arc::from(mapped_children))
        }
        Expression::List(items) => Expression::List(map_slice(items, replace)?),
        Expression::Map(entries) => {
            let mut mapped_entries = Vec::with_capacity(entries.len());
            for (key, value) in entries.iter() {
                mapped_entries.push((mapped(key, replace)?, mapped(value, replace)?));
            }
            Expression::Map(Arc::from(mapped_entries))
        }
    })
}

fn map_slice(
    operands: &[Expression],
    replace: &mut dyn FnMut(&Expression) -> Result<Option<Expression>>,
) -> Result<Arc<[Expression]>> {
    let mut mapped = Vec::with_capacity(operands.len());
    for operand in operands {
        mapped.push(map_children(operand, replace)?);
    }
    Ok(Arc::from(mapped))
}

/// Everything that turns one typed expression into one resolved node.
struct Binder<'schema> {
    schema: &'schema Field,
}

impl Binder<'_> {
    /// Lower one expression, converting it into `want` when one is named.
    #[allow(clippy::too_many_lines)]
    fn lower(&self, expression: &Expression, want: Option<&DataType>) -> Result<Node> {
        let node = match expression {
            Expression::Literal(held) => {
                let target = want.unwrap_or_else(|| held.data_type());
                let value = convert(target, held.value(), Safety::Strict)?;
                self.leaf(
                    expression,
                    target.clone(),
                    value.is_null(),
                    Kind::Literal(value),
                    0,
                )
            }
            Expression::Column(name) => {
                let index = self.index_of(name)?;
                let field = self.schema.fields()[index].clone();
                Node {
                    cost: COST_COLUMN,
                    field,
                    kind: Kind::Column(index),
                }
            }
            Expression::Path(base, steps) => {
                let base = self.lower(base, None)?;
                let field = expression.field(self.schema)?;
                let cost = base.cost + 1;
                Node {
                    field,
                    kind: Kind::Path(Box::new(base), steps.clone()),
                    cost,
                }
            }
            Expression::Attribute(selector) => Node {
                field: selector.field(),
                cost: match selector.cost() {
                    Cost::Free => COST_FREE_ATTRIBUTE,
                    Cost::Stat => COST_STAT,
                },
                kind: Kind::Attribute(selector.clone()),
            },
            Expression::Parameter(name) => {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected a value for parameter :{name}"),
                });
            }
            Expression::And(operands) | Expression::Or(operands) => {
                let mut lowered = Vec::with_capacity(operands.len());
                for operand in operands.iter() {
                    lowered.push(self.lower(operand, Some(&DataType::Boolean))?);
                }
                // Cheapest-first, stably: two conjuncts that cost the same keep
                // the order the caller wrote, so a predicate reads the way it
                // was written whenever ordering has nothing to say.
                lowered.sort_by_key(|node| node.cost);
                let nullable = lowered.iter().any(|node| node.field.is_nullable());
                let cost = lowered.iter().map(|node| node.cost).max().unwrap_or(0);
                let kind = if matches!(expression, Expression::And(_)) {
                    Kind::And(lowered)
                } else {
                    Kind::Or(lowered)
                };
                Node {
                    field: named(expression, DataType::Boolean, nullable),
                    kind,
                    cost,
                }
            }
            Expression::Not(inner) => {
                let inner = self.lower(inner, Some(&DataType::Boolean))?;
                let (nullable, cost) = (inner.field.is_nullable(), inner.cost);
                Node {
                    field: named(expression, DataType::Boolean, nullable),
                    kind: Kind::Not(Box::new(inner)),
                    cost,
                }
            }
            Expression::Compare(left, comparison, right) => {
                let shared = self.shared_type(left, right)?;
                let left = self.lower(left, Some(&shared))?;
                let right = self.lower(right, Some(&shared))?;
                let nullable = !comparison.is_two_valued()
                    && (left.field.is_nullable() || right.field.is_nullable());
                let cost = left.cost + right.cost;
                Node {
                    field: named(expression, DataType::Boolean, nullable),
                    kind: Kind::Compare(Box::new(left), *comparison, Box::new(right)),
                    cost,
                }
            }
            Expression::In(value, list) => {
                let mut shared = self.type_of(value)?;
                for item in list.iter() {
                    shared = common_type(&shared, &self.type_of(item)?).ok_or_else(|| {
                        incompatible("an `in` list that shares a type with its value")
                    })?;
                }
                let value = self.lower(value, Some(&shared))?;
                let mut lowered = Vec::with_capacity(list.len());
                for item in list.iter() {
                    lowered.push(self.lower(item, Some(&shared))?);
                }
                let nullable = value.field.is_nullable()
                    || lowered.iter().any(|node| node.field.is_nullable());
                let cost = value.cost + lowered.iter().map(|node| node.cost).sum::<u32>();
                Node {
                    field: named(expression, DataType::Boolean, nullable),
                    kind: Kind::In(Box::new(value), lowered),
                    cost,
                }
            }
            Expression::Between(value, low, high) => {
                let mut shared = self.type_of(value)?;
                for bound in [low, high] {
                    shared = common_type(&shared, &self.type_of(bound)?)
                        .ok_or_else(|| incompatible("`between` bounds that share a type"))?;
                }
                let value = self.lower(value, Some(&shared))?;
                let low = self.lower(low, Some(&shared))?;
                let high = self.lower(high, Some(&shared))?;
                let nullable = value.field.is_nullable()
                    || low.field.is_nullable()
                    || high.field.is_nullable();
                let cost = value.cost + low.cost + high.cost;
                Node {
                    field: named(expression, DataType::Boolean, nullable),
                    kind: Kind::Between(Box::new(value), Box::new(low), Box::new(high)),
                    cost,
                }
            }
            Expression::IsNull(inner) | Expression::IsNotNull(inner) => {
                let inner = self.lower(inner, None)?;
                let cost = inner.cost;
                let kind = if matches!(expression, Expression::IsNull(_)) {
                    Kind::IsNull(Box::new(inner))
                } else {
                    Kind::IsNotNull(Box::new(inner))
                };
                Node {
                    field: named(expression, DataType::Boolean, false),
                    kind,
                    cost,
                }
            }
            Expression::Like {
                value,
                pattern,
                case_insensitive,
                escape,
            } => {
                let value = self.lower(value, Some(&DataType::Utf8))?;
                let pattern = self.constant_pattern(pattern, "like")?;
                // A pattern with no wildcard left in it is an equality, and
                // saying so here is what lets it reach a comparison kernel and
                // a statistics bound instead of a character walk.
                if !*case_insensitive && !has_wildcard(&pattern, *escape) {
                    let literal = Node {
                        field: Field::new("pattern", DataType::Utf8, false),
                        kind: Kind::Literal(Scalar::String(unescape(&pattern, *escape))),
                        cost: 0,
                    };
                    let (nullable, cost) = (value.field.is_nullable(), value.cost);
                    return self.coerce(
                        Node {
                            field: named(expression, DataType::Boolean, nullable),
                            kind: Kind::Compare(Box::new(value), Comparison::Eq, Box::new(literal)),
                            cost,
                        },
                        want,
                    );
                }
                let (nullable, cost) = (value.field.is_nullable(), value.cost);
                Node {
                    field: named(expression, DataType::Boolean, nullable),
                    kind: Kind::Like {
                        value: Box::new(value),
                        pattern,
                        case_insensitive: *case_insensitive,
                        escape: *escape,
                    },
                    cost,
                }
            }
            Expression::Glob(value, pattern) => {
                let value = self.lower(value, Some(&DataType::Utf8))?;
                let pattern = self.constant_pattern(pattern, "glob")?;
                let (nullable, cost) = (value.field.is_nullable(), value.cost);
                Node {
                    field: named(expression, DataType::Boolean, nullable),
                    kind: Kind::Glob(Box::new(value), pattern),
                    cost,
                }
            }
            Expression::Arithmetic(left, operator, right) => {
                let field = expression.field(self.schema)?;
                let operand =
                    arithmetic_operand_type(&field, self.type_of(left)?, self.type_of(right)?);
                let left = self.lower(left, operand.as_ref())?;
                let right = self.lower(right, operand.as_ref())?;
                let cost = left.cost + right.cost;
                Node {
                    field,
                    kind: Kind::Arithmetic(Box::new(left), *operator, Box::new(right)),
                    cost,
                }
            }
            Expression::Negate(inner) => {
                let field = expression.field(self.schema)?;
                let inner = self.lower(inner, None)?;
                let cost = inner.cost;
                Node {
                    field,
                    kind: Kind::Negate(Box::new(inner)),
                    cost,
                }
            }
            Expression::Function(function, arguments) => {
                let field = expression.field(self.schema)?;
                let mut lowered = Vec::with_capacity(arguments.len());
                let unified = matches!(function, Function::Coalesce | Function::IfNull)
                    .then(|| field.data_type().clone());
                for argument in arguments.iter() {
                    lowered.push(self.lower(argument, unified.as_ref())?);
                }
                let cost = lowered.iter().map(|node| node.cost).sum::<u32>() + 1;
                Node {
                    field,
                    kind: Kind::Function(*function, lowered),
                    cost,
                }
            }
            Expression::Cast(inner, data_type, safety) => {
                let inner = self.lower(inner, None)?;
                let nullable = inner.field.is_nullable() || matches!(safety, Safety::Safe);
                let cost = inner.cost + 1;
                Node {
                    field: named(expression, data_type.clone(), nullable),
                    kind: Kind::Cast(Box::new(inner), *safety),
                    cost,
                }
            }
            Expression::Case {
                branches,
                otherwise,
            } => {
                let field = expression.field(self.schema)?;
                let target = field.data_type().clone();
                let mut lowered = Vec::with_capacity(branches.len());
                let mut cost = 0;
                for (when, then) in branches.iter() {
                    let when = self.lower(when, Some(&DataType::Boolean))?;
                    let then = self.lower(then, Some(&target))?;
                    cost += when.cost + then.cost;
                    lowered.push((when, then));
                }
                let otherwise = match otherwise {
                    Some(otherwise) => {
                        let otherwise = self.lower(otherwise, Some(&target))?;
                        cost += otherwise.cost;
                        Some(Box::new(otherwise))
                    }
                    None => None,
                };
                Node {
                    field,
                    kind: Kind::Case {
                        branches: lowered,
                        otherwise,
                    },
                    cost,
                }
            }
            Expression::Struct(children) => {
                let field = expression.field(self.schema)?;
                let mut lowered = Vec::with_capacity(children.len());
                for (index, (_, value)) in children.iter().enumerate() {
                    let target = field.get_field(index).map(|held| held.data_type().clone());
                    lowered.push(self.lower(value, target.as_ref())?);
                }
                let cost = lowered.iter().map(|node| node.cost).sum::<u32>();
                Node {
                    field,
                    kind: Kind::Struct(lowered),
                    cost,
                }
            }
            Expression::List(items) => {
                let field = expression.field(self.schema)?;
                let target = list_item_type(&field);
                let mut lowered = Vec::with_capacity(items.len());
                for item in items.iter() {
                    lowered.push(self.lower(item, target.as_ref())?);
                }
                let cost = lowered.iter().map(|node| node.cost).sum::<u32>();
                Node {
                    field,
                    kind: Kind::List(lowered),
                    cost,
                }
            }
            Expression::Map(entries) => {
                let field = expression.field(self.schema)?;
                let (key_type, value_type) = map_entry_types(&field);
                let mut lowered = Vec::with_capacity(entries.len());
                for (key, value) in entries.iter() {
                    lowered.push((
                        self.lower(key, key_type.as_ref())?,
                        self.lower(value, value_type.as_ref())?,
                    ));
                }
                let cost = lowered
                    .iter()
                    .map(|(key, value)| key.cost + value.cost)
                    .sum::<u32>();
                Node {
                    field,
                    kind: Kind::Map(lowered),
                    cost,
                }
            }
        };
        let node = fold(node)?;
        self.coerce(node, want)
    }

    /// Build one leaf node.
    fn leaf(
        &self,
        expression: &Expression,
        data_type: DataType,
        nullable: bool,
        kind: Kind,
        cost: u32,
    ) -> Node {
        let _ = self;
        Node {
            field: named(expression, data_type, nullable),
            kind,
            cost,
        }
    }

    /// Convert a lowered node into `want`, when it is not already there.
    fn coerce(&self, node: Node, want: Option<&DataType>) -> Result<Node> {
        let _ = self;
        let Some(want) = want else {
            return Ok(node);
        };
        if node.field.data_type() == want {
            return Ok(node);
        }
        // A constant is converted now; anything else grows a cast the
        // evaluators run per row or per batch.
        if let Kind::Literal(value) = &node.kind {
            let converted = convert(want, value, Safety::Strict)?;
            let nullable = converted.is_null();
            return Ok(Node {
                field: node
                    .field
                    .try_with_data_type(want.clone())?
                    .with_nullable(nullable),
                kind: Kind::Literal(converted),
                cost: node.cost,
            });
        }
        let nullable = node.field.is_nullable();
        let cost = node.cost + 1;
        let field = node
            .field
            .clone()
            .try_with_data_type(want.clone())?
            .with_nullable(nullable);
        Ok(Node {
            field,
            kind: Kind::Cast(Box::new(node), Safety::Strict),
            cost,
        })
    }

    /// The column index a name resolves to, ASCII case-insensitively.
    fn index_of(&self, name: &str) -> Result<usize> {
        // A schema that declares two columns differing only in case makes an
        // unquoted reference genuinely ambiguous, and first-match-wins is the
        // one resolution rule nobody can debug. Both names are reported.
        let matches: Vec<usize> = self
            .schema
            .fields()
            .iter()
            .enumerate()
            .filter(|(_, field)| field.name().eq_ignore_ascii_case(name))
            .map(|(index, _)| index)
            .collect();
        if matches.len() > 1 {
            let names: Vec<&str> = matches
                .iter()
                .filter_map(|index| self.schema.get_field(*index).map(Field::name))
                .collect();
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected {name:?} to name one column, got {}; quote the one meant",
                    names.join(" and ")
                ),
            });
        }
        matches
            .first()
            .copied()
            .ok_or_else(|| super::typing::unknown_column(name, self.schema))
    }

    fn type_of(&self, expression: &Expression) -> Result<DataType> {
        Ok(expression.field(self.schema)?.data_type().clone())
    }

    /// The type two compared operands meet in.
    ///
    /// A literal is narrowed into the other side's type when it fits exactly,
    /// which is what keeps `int32_column = 1` an `int32` comparison instead of
    /// widening a whole column to `int64` per batch. When it does not fit, the
    /// promotion table decides and neither side loses anything.
    fn shared_type(&self, left: &Expression, right: &Expression) -> Result<DataType> {
        let left_type = self.type_of(left)?;
        let right_type = self.type_of(right)?;
        for (literal, other) in [(left, &right_type), (right, &left_type)] {
            if let Expression::Literal(held) = literal {
                // Narrowing is only ever a *choice between* types the promotion
                // table already accepts. Without that guard a `1` would narrow
                // into a text column and `s > 1` would quietly become a string
                // comparison, which is the exact silent widening this module
                // exists to refuse.
                if held.value().is_null() || common_type(held.data_type(), other).is_none() {
                    continue;
                }
                if fits(other, held) {
                    return Ok(other.clone());
                }
            }
        }
        common_type(&left_type, &right_type).ok_or_else(|| {
            incompatible(&format!(
                "comparable operands, got {left_type} and {right_type}"
            ))
        })
    }

    /// Read the constant pattern a match operator requires.
    fn constant_pattern(&self, pattern: &Expression, operator: &str) -> Result<SmolStr> {
        let lowered = self.lower(pattern, Some(&DataType::Utf8))?;
        match lowered.as_literal().and_then(Scalar::as_str) {
            Some(text) => Ok(SmolStr::new(text)),
            None => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected a constant `{operator}` pattern; a pattern that changes per row is \
                     a different operation and this grammar does not spell it"
                ),
            }),
        }
    }
}

/// Return whether a literal is representable in a datatype without loss.
fn fits(data_type: &DataType, held: &TypedScalar) -> bool {
    let Ok(converted) = convert(data_type, held.value(), Safety::Strict) else {
        return false;
    };
    // A conversion that cannot be undone lost something, and a lost digit
    // turns `=` into a quiet lie.
    convert(held.data_type(), &converted, Safety::Strict).is_ok_and(|back| &back == held.value())
}

/// Evaluate a node whose operands are all constant, replacing it with its value.
fn fold(node: Node) -> Result<Node> {
    if matches!(node.kind, Kind::Literal(_)) {
        return Ok(node);
    }
    let mut constant = true;
    node.for_each_child(|child| constant &= matches!(child.kind, Kind::Literal(_)));
    if !constant {
        return Ok(node);
    }
    // An attribute reads the holder and a column reads the row, so neither is
    // constant even with no children at all.
    if matches!(node.kind, Kind::Column(_) | Kind::Attribute(_)) {
        return Ok(node);
    }
    let Ok(value) = node.eval(&Row::new(None, None)) else {
        // A constant subtree that fails - a strict cast that refuses, say -
        // keeps its node so the failure arrives where the caller can see the
        // row it happened on, rather than at bind time on no row at all.
        return Ok(node);
    };
    Ok(Node {
        field: node.field.with_nullable(value.is_null()),
        kind: Kind::Literal(value),
        cost: 0,
    })
}

/// The type both arithmetic operands are converted into, when there is one.
fn arithmetic_operand_type(field: &Field, left: DataType, right: DataType) -> Option<DataType> {
    // Temporal arithmetic keeps each side in its own type: adding a duration to
    // a timestamp is not an addition of two timestamps.
    if super::typing::temporal_parts(&left).is_some()
        || super::typing::temporal_parts(&right).is_some()
    {
        return None;
    }
    Some(field.data_type().clone())
}

/// The declared element type of a list field.
fn list_item_type(field: &Field) -> Option<DataType> {
    match field.data_type() {
        DataType::List(item)
        | DataType::ListView(item)
        | DataType::FixedSizeList(item, _)
        | DataType::LargeList(item)
        | DataType::LargeListView(item) => Some(item.data_type().clone()),
        _ => None,
    }
}

/// The declared key and value types of a map field.
fn map_entry_types(field: &Field) -> (Option<DataType>, Option<DataType>) {
    match field.data_type() {
        DataType::Map(map) => (
            map.entries()
                .get_field(0)
                .map(|held| held.data_type().clone()),
            map.entries()
                .get_field(1)
                .map(|held| held.data_type().clone()),
        ),
        _ => (None, None),
    }
}

/// Return whether a `like` pattern still holds a wildcard after escaping.
fn has_wildcard(pattern: &str, escape: Option<char>) -> bool {
    let mut characters = pattern.chars();
    while let Some(character) = characters.next() {
        if Some(character) == escape {
            let _ = characters.next();
            continue;
        }
        if character == '%' || character == '_' {
            return true;
        }
    }
    false
}

/// The literal text a wildcard-free `like` pattern names.
fn unescape(pattern: &str, escape: Option<char>) -> SmolStr {
    let Some(escape) = escape else {
        return SmolStr::new(pattern);
    };
    let mut text = String::with_capacity(pattern.len());
    let mut characters = pattern.chars();
    while let Some(character) = characters.next() {
        if character == escape {
            if let Some(escaped) = characters.next() {
                text.push(escaped);
            }
            continue;
        }
        text.push(character);
    }
    SmolStr::new(text)
}

fn named(expression: &Expression, data_type: DataType, nullable: bool) -> Field {
    Field::new(SmolStr::new(expression.to_string()), data_type, nullable)
}

fn incompatible(expected: &str) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: format_smolstr!("expected {expected}"),
    }
}

/// Rebuild the expression a resolved tree stands for.
///
/// The result is what actually runs: folded, ordered, and with every literal in
/// the type it will be compared in. Printing it is how a caller sees what bind
/// decided without a second representation to keep in step.
pub(crate) fn rebuild(node: &Node) -> Expression {
    match &node.kind {
        Kind::Literal(value) => {
            TypedScalar::from_parts(node.field.data_type().clone(), value.clone())
                .map_or_else(|_| Expression::literal(value.clone()), Expression::Literal)
        }
        Kind::Column(_) => Expression::column(node.field.name()),
        Kind::Path(base, steps) => Expression::Path(Box::new(rebuild(base)), steps.clone()),
        Kind::Attribute(selector) => Expression::attribute(selector.clone()),
        Kind::And(operands) => Expression::And(operands.iter().map(rebuild).collect()),
        Kind::Or(operands) => Expression::Or(operands.iter().map(rebuild).collect()),
        Kind::Not(inner) => Expression::Not(Box::new(rebuild(inner))),
        Kind::Compare(left, comparison, right) => Expression::Compare(
            Box::new(rebuild(left)),
            *comparison,
            Box::new(rebuild(right)),
        ),
        Kind::In(value, list) => {
            Expression::In(Box::new(rebuild(value)), list.iter().map(rebuild).collect())
        }
        Kind::Between(value, low, high) => Expression::Between(
            Box::new(rebuild(value)),
            Box::new(rebuild(low)),
            Box::new(rebuild(high)),
        ),
        Kind::IsNull(inner) => Expression::IsNull(Box::new(rebuild(inner))),
        Kind::IsNotNull(inner) => Expression::IsNotNull(Box::new(rebuild(inner))),
        Kind::Like {
            value,
            pattern,
            case_insensitive,
            escape,
        } => Expression::Like {
            value: Box::new(rebuild(value)),
            pattern: Box::new(Expression::literal(Scalar::String(pattern.clone()))),
            case_insensitive: *case_insensitive,
            escape: *escape,
        },
        Kind::Glob(value, pattern) => Expression::Glob(
            Box::new(rebuild(value)),
            Box::new(Expression::literal(Scalar::String(pattern.clone()))),
        ),
        Kind::Arithmetic(left, operator, right) => {
            Expression::Arithmetic(Box::new(rebuild(left)), *operator, Box::new(rebuild(right)))
        }
        Kind::Negate(inner) => Expression::Negate(Box::new(rebuild(inner))),
        Kind::Function(function, arguments) => {
            Expression::Function(*function, arguments.iter().map(rebuild).collect())
        }
        Kind::Cast(inner, safety) => Expression::Cast(
            Box::new(rebuild(inner)),
            node.field.data_type().clone(),
            *safety,
        ),
        Kind::Case {
            branches,
            otherwise,
        } => Expression::Case {
            branches: branches
                .iter()
                .map(|(when, then)| (rebuild(when), rebuild(then)))
                .collect(),
            otherwise: otherwise.as_ref().map(|held| Box::new(rebuild(held))),
        },
        Kind::Struct(children) => Expression::Struct(
            node.field
                .fields()
                .iter()
                .map(|field| SmolStr::new(field.name()))
                .zip(children.iter().map(rebuild))
                .collect(),
        ),
        Kind::List(items) => Expression::List(items.iter().map(rebuild).collect()),
        Kind::Map(entries) => Expression::Map(
            entries
                .iter()
                .map(|(key, value)| (rebuild(key), rebuild(value)))
                .collect(),
        ),
    }
}
