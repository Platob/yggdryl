//! The compile step: one [`Term`] and one schema become one [`Bound`].
//!
//! Everything that can be decided before the first row is decided here, once:
//!
//! * every parameter is substituted, so nothing is late-bound during a scan;
//! * the tree is [simplified](Term::simplify), so a negation never hides a
//!   comparison and a run of equalities is one membership test;
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

use smol_str::{SmolStr, format_smolstr};

use super::attribute::{Attribute, Attributes, Cost};
use super::eval::{Row, convert};
use super::path::FieldSegment;
use super::typing::{column_index, common_type};
use super::{Comparison, Filter, Function, Literal, Operator, Safety, Term, named};
use crate::{DataType, Error, Field, Result, Scalar};

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

/// The resolved form of every [`Term`] variant.
#[derive(Clone, Debug)]
pub(crate) enum Kind {
    /// A constant, already in this node's declared datatype.
    Literal(Scalar),
    /// A column, by index into the bound schema.
    Column(usize),
    /// A path into a value: the column it starts at and the steps taken
    /// inside it, each typed once.
    Path(Box<Node>, Vec<Step>),
    /// A holder attribute.
    Attribute(Attribute),
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

/// One step of a bound path, and the field it reaches.
///
/// A predicate segment is the one step that carries a term of its own: the
/// term is lowered here against the element struct, so the two row evaluators
/// answer it from a resolved tree and never bind per row.
#[derive(Clone, Debug)]
pub(crate) struct Step {
    pub(crate) kind: StepKind,
    /// The field this step reaches, typed by [`FieldSegment::apply_field`].
    pub(crate) field: Field,
}

/// What one bound step does.
#[derive(Clone, Debug)]
pub(crate) enum StepKind {
    /// A step the segment answers by itself.
    Segment(FieldSegment),
    /// A predicate over the element struct, resolved against it; the
    /// element struct is the item of the field the step reaches.
    Where(Box<Node>),
}

impl Step {
    /// The segment this step stands for, the predicate rebuilt as bound.
    pub(crate) fn segment(&self) -> FieldSegment {
        match &self.kind {
            StepKind::Segment(segment) => segment.clone(),
            StepKind::Where(predicate) => FieldSegment::filter(rebuild(predicate)),
        }
    }

    /// The element struct a predicate step keeps elements of.
    ///
    /// Total for a predicate step: binding typed the field as a list of the
    /// element struct, so the item is there to borrow.
    pub(crate) fn element(&self) -> Option<&Field> {
        match &self.kind {
            StepKind::Where(_) => super::path::list_item(self.field.dtype()),
            StepKind::Segment(_) => None,
        }
    }
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
    ///
    /// A path's children are its base and nothing else: the predicate a step
    /// carries reads the element struct, so its column indices are not the
    /// row's and must never be gathered as if they were.
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

/// One term, resolved against one schema, ready to answer.
///
/// A `Bound` is built once per stream and answers three ways: row at a time
/// over [`Scalar`], vectorized over an Arrow batch, and three-valued over
/// container statistics. All three walk the same resolved tree.
#[derive(Clone, Debug)]
pub struct Bound {
    schema: Field,
    term: Term,
    node: Node,
}

impl Bound {
    /// The struct root this term was bound against.
    #[must_use]
    pub const fn schema(&self) -> &Field {
        &self.schema
    }

    /// The term as it stands after substitution, simplification, folding, and
    /// ordering.
    ///
    /// This is what a log line should print: it is the plan that will actually
    /// run, not the text the caller wrote.
    #[must_use]
    pub const fn term(&self) -> &Term {
        &self.term
    }

    /// The output field this term produces.
    #[must_use]
    pub const fn field(&self) -> &Field {
        &self.node.field
    }

    /// Return whether this term answers a boolean.
    #[must_use]
    pub fn is_predicate(&self) -> bool {
        matches!(self.node.field.dtype(), DataType::Boolean | DataType::Null)
    }

    /// The schema column indices this term reads, ascending.
    ///
    /// This is projection pushdown: a reader decodes these and no others.
    #[must_use]
    pub fn column_indices(&self) -> Vec<usize> {
        self.node.column_indices()
    }

    /// The schema column names this term reads, in index order.
    #[must_use]
    pub fn column_names(&self) -> Vec<String> {
        let fields = self.schema.fields();
        self.column_indices()
            .into_iter()
            .filter_map(|index| fields.get(index).map(|field| field.name().to_owned()))
            .collect()
    }

    /// Return whether answering this term requires reading rows.
    #[must_use]
    pub fn reads_rows(&self) -> bool {
        self.node.reads_rows()
    }

    /// The resolved tree, for the evaluators in this module.
    pub(crate) const fn node(&self) -> &Node {
        &self.node
    }

    /// Evaluate this term for one row.
    ///
    /// The row is a [`crate::types::sequence::Sequence`] of column values in
    /// schema order.
    ///
    /// # Errors
    ///
    /// Returns an error when the row does not match the bound schema, a strict
    /// cast refuses a value, or checked arithmetic overflows, divides by zero,
    /// or cannot represent an exact decimal result.
    pub fn eval(&self, row: &Scalar) -> Result<Scalar> {
        let values = row_values(row, &self.schema)?;
        self.node.eval(&Row::new(Some(values), None))
    }

    /// Evaluate this term over a row held as its column values.
    ///
    /// `values` holds the bound schema's columns in order, exactly as
    /// [`Self::eval`] reads them out of a sequence: the door a pass that
    /// fills a row evaluates through, writing each answer into the values it
    /// reads the next one from, so no row is rebuilt to be read.
    ///
    /// # Errors
    ///
    /// Returns an error when `values` does not hold one value per column of
    /// the schema, a strict cast refuses a value, or checked arithmetic
    /// overflows, divides by zero, or cannot represent an exact decimal
    /// result.
    pub(crate) fn eval_values(&self, values: &[Scalar]) -> Result<Scalar> {
        self.node
            .eval(&Row::new(Some(sized(values, &self.schema)?), None))
    }

    /// Evaluate this term for one row alongside a holder.
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

    /// What a holder alone settles about this predicate, three-valued.
    ///
    /// Only the conjuncts a holder can answer are evaluated - the ones that
    /// read no column. Every other conjunct leaves the conjunction unknown,
    /// and an unknown conjunct excludes nothing, which is what keeps a listing
    /// filter conservative: it may keep a file the rows will later discard,
    /// and it may never discard a file that would have matched.
    ///
    /// The conjuncts run cheapest-first and stop at the first `false`, so a
    /// predicate answerable from the path alone performs no backend call.
    ///
    /// # Errors
    ///
    /// Returns the holder's failure when a stat attribute cannot be read.
    pub fn settle_holder(&self, holder: &dyn Attributes) -> Result<Scalar> {
        let row = Row::new(None, Some(holder));
        let mut unknown = false;
        for conjunct in self.node.conjuncts() {
            if conjunct.reads_rows() {
                unknown = true;
                continue;
            }
            match conjunct.eval(&row)?.as_bool() {
                Some(false) => return Ok(Scalar::from(false)),
                Some(true) => {}
                None => unknown = true,
            }
        }
        Ok(if unknown {
            Scalar::Null
        } else {
            Scalar::from(true)
        })
    }

    /// Return whether a holder is *not ruled out* by this predicate.
    ///
    /// [`Self::settle_holder`] read conservatively: only a proven `false`
    /// excludes, so an unknown keeps the holder.
    ///
    /// # Errors
    ///
    /// Returns the holder's failure when a stat attribute cannot be read.
    pub fn matches_holder(&self, holder: &dyn Attributes) -> Result<bool> {
        Ok(self.settle_holder(holder)?.as_bool() != Some(false))
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
    sized(values, schema)
}

/// The values, proven one per column of the schema.
fn sized<'row>(values: &'row [Scalar], schema: &Field) -> Result<&'row [Scalar]> {
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
        write!(formatter, "{}", self.term)
    }
}

impl Term {
    /// Resolve this term against a schema.
    ///
    /// # Errors
    ///
    /// Returns an error when a column is unknown, two operands share no type,
    /// a pattern is not constant, or a parameter was not supplied.
    pub fn bind(&self, schema: &Field) -> Result<Bound> {
        self.bind_with(schema, &[])
    }

    /// Resolve this term against a schema, supplying its parameters.
    ///
    /// # Errors
    ///
    /// Returns an error when a parameter is missing or the term cannot be
    /// resolved.
    pub fn bind_with(&self, schema: &Field, parameters: &[(&str, Scalar)]) -> Result<Bound> {
        schema.require_struct()?;
        self.check_budget()?;
        let supplied = substitute(self, parameters)?.simplify();
        let binder = Binder { schema };
        let node = binder.lower(&supplied, None)?;
        Ok(Bound {
            schema: schema.clone(),
            term: rebuild(&node),
            node,
        })
    }
}

impl Filter {
    /// Resolve this filter against a schema.
    ///
    /// # Errors
    ///
    /// Returns an error when the term cannot be resolved, or when it answers
    /// anything but a boolean.
    pub fn bind(&self, schema: &Field) -> Result<Bound> {
        self.bind_with(schema, &[])
    }

    /// Resolve this filter against a schema, supplying its parameters.
    ///
    /// # Errors
    ///
    /// Returns an error when a parameter is missing, the term cannot be
    /// resolved, or it answers anything but a boolean.
    pub fn bind_with(&self, schema: &Field, parameters: &[(&str, Scalar)]) -> Result<Bound> {
        let bound = self.term().bind_with(schema, parameters)?;
        super::filter::require_boolean(bound.field())?;
        Ok(bound)
    }
}

/// Replace every parameter with the value supplied for it.
fn substitute(term: &Term, parameters: &[(&str, Scalar)]) -> Result<Term> {
    if parameters.is_empty() && term.parameters().is_empty() {
        return Ok(term.clone());
    }
    term.map(&mut |node| match node {
        Term::Parameter(name) => {
            let supplied = parameters
                .iter()
                .find(|(held, _)| held.eq_ignore_ascii_case(name))
                .ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected a value for parameter :{name}"),
                })?;
            Ok(Some(Term::literal(supplied.1.clone())))
        }
        _ => Ok(None),
    })
}

/// Everything that turns one typed term into one resolved node.
struct Binder<'schema> {
    schema: &'schema Field,
}

impl Binder<'_> {
    /// Lower one term, converting it into `want` when one is named.
    #[allow(clippy::too_many_lines)]
    fn lower(&self, term: &Term, want: Option<&DataType>) -> Result<Node> {
        let node = match term {
            Term::Literal(held) => {
                let target = want.unwrap_or_else(|| held.dtype());
                let value = convert(target, held.value(), Safety::Strict)?;
                Node {
                    field: named(term, target.clone(), value.is_null()),
                    kind: Kind::Literal(value),
                    cost: 0,
                }
            }
            Term::Path(steps) => {
                let (first, rest) = steps.split_first().ok_or_else(|| {
                    incompatible("a path that starts at a column, got the row itself")
                })?;
                let FieldSegment::Field(name) = first else {
                    return Err(incompatible(&format!(
                        "a path that starts at a column, got the step {first}"
                    )));
                };
                let index = column_index(name, self.schema)?;
                let column = self.schema.fields()[index].clone();
                let base = Node {
                    cost: COST_COLUMN,
                    field: column.clone(),
                    kind: Kind::Column(index),
                };
                if rest.is_empty() {
                    base
                } else {
                    let mut field = column;
                    let mut steps = Vec::with_capacity(rest.len());
                    let mut cost = COST_COLUMN + 1;
                    for segment in rest {
                        let reached = segment.apply_field(&field)?;
                        let kind = match segment {
                            FieldSegment::Where(predicate) => {
                                // The element struct is the row the predicate
                                // reads, so it is lowered against that and
                                // not against the schema of the path.
                                let element = super::path::element_field(&field)?;
                                let inner = Binder { schema: &element }
                                    .lower(predicate, Some(&DataType::Boolean))?;
                                cost += inner.cost;
                                StepKind::Where(Box::new(inner))
                            }
                            other => StepKind::Segment(other.clone()),
                        };
                        steps.push(Step {
                            kind,
                            field: reached.clone(),
                        });
                        field = reached;
                    }
                    Node {
                        field: field.with_name(SmolStr::new(term.to_string())),
                        kind: Kind::Path(Box::new(base), steps),
                        cost,
                    }
                }
            }
            Term::Attribute(attribute) => Node {
                field: attribute.field(),
                cost: match attribute.cost() {
                    Cost::Free => COST_FREE_ATTRIBUTE,
                    Cost::Stat => COST_STAT,
                },
                kind: Kind::Attribute(attribute.clone()),
            },
            Term::Parameter(name) => {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected a value for parameter :{name}"),
                });
            }
            Term::And(operands) | Term::Or(operands) => {
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
                let kind = if matches!(term, Term::And(_)) {
                    Kind::And(lowered)
                } else {
                    Kind::Or(lowered)
                };
                Node {
                    field: named(term, DataType::Boolean, nullable),
                    kind,
                    cost,
                }
            }
            Term::Not(inner) => {
                let inner = self.lower(inner, Some(&DataType::Boolean))?;
                let (nullable, cost) = (inner.field.is_nullable(), inner.cost);
                Node {
                    field: named(term, DataType::Boolean, nullable),
                    kind: Kind::Not(Box::new(inner)),
                    cost,
                }
            }
            Term::Compare(left, comparison, right) => {
                let shared = self.shared_type(left, right)?;
                let left = self.lower(left, Some(&shared))?;
                let right = self.lower(right, Some(&shared))?;
                let nullable = !comparison.is_two_valued()
                    && (left.field.is_nullable() || right.field.is_nullable());
                let cost = left.cost + right.cost;
                Node {
                    field: named(term, DataType::Boolean, nullable),
                    kind: Kind::Compare(Box::new(left), *comparison, Box::new(right)),
                    cost,
                }
            }
            Term::In(value, list) => {
                let shared = self.list_type(value, list)?;
                let value = self.lower(value, Some(&shared))?;
                let mut lowered = Vec::with_capacity(list.len());
                for item in list.iter() {
                    lowered.push(self.lower(item, Some(&shared))?);
                }
                let nullable = value.field.is_nullable()
                    || lowered.iter().any(|node| node.field.is_nullable());
                let cost = value.cost + lowered.iter().map(|node| node.cost).sum::<u32>();
                Node {
                    field: named(term, DataType::Boolean, nullable),
                    kind: Kind::In(Box::new(value), lowered),
                    cost,
                }
            }
            Term::Between(value, low, high) => {
                let shared =
                    self.list_type(value, &[low.as_ref().clone(), high.as_ref().clone()])?;
                let value = self.lower(value, Some(&shared))?;
                let low = self.lower(low, Some(&shared))?;
                let high = self.lower(high, Some(&shared))?;
                let nullable = value.field.is_nullable()
                    || low.field.is_nullable()
                    || high.field.is_nullable();
                let cost = value.cost + low.cost + high.cost;
                Node {
                    field: named(term, DataType::Boolean, nullable),
                    kind: Kind::Between(Box::new(value), Box::new(low), Box::new(high)),
                    cost,
                }
            }
            Term::IsNull(inner) | Term::IsNotNull(inner) => {
                let inner = self.lower(inner, None)?;
                let cost = inner.cost;
                let kind = if matches!(term, Term::IsNull(_)) {
                    Kind::IsNull(Box::new(inner))
                } else {
                    Kind::IsNotNull(Box::new(inner))
                };
                Node {
                    field: named(term, DataType::Boolean, false),
                    kind,
                    cost,
                }
            }
            Term::Like {
                value,
                pattern,
                case_insensitive,
                escape,
            } => {
                let value = self.lower(value, Some(&DataType::utf8()))?;
                let pattern = self.constant_pattern(pattern, "like")?;
                // A pattern with no wildcard left in it is an equality, and
                // saying so here is what lets it reach a comparison kernel and
                // a statistics bound instead of a character walk.
                if !*case_insensitive && !has_wildcard(&pattern, *escape) {
                    let literal = Node {
                        field: Field::new("pattern", DataType::utf8(), false),
                        kind: Kind::Literal(Scalar::from(unescape(&pattern, *escape))),
                        cost: 0,
                    };
                    let (nullable, cost) = (value.field.is_nullable(), value.cost);
                    return self.coerce(
                        Node {
                            field: named(term, DataType::Boolean, nullable),
                            kind: Kind::Compare(Box::new(value), Comparison::Eq, Box::new(literal)),
                            cost,
                        },
                        want,
                    );
                }
                let (nullable, cost) = (value.field.is_nullable(), value.cost);
                Node {
                    field: named(term, DataType::Boolean, nullable),
                    kind: Kind::Like {
                        value: Box::new(value),
                        pattern,
                        case_insensitive: *case_insensitive,
                        escape: *escape,
                    },
                    cost,
                }
            }
            Term::Glob(value, pattern) => {
                let value = self.lower(value, Some(&DataType::utf8()))?;
                let pattern = self.constant_pattern(pattern, "glob")?;
                let (nullable, cost) = (value.field.is_nullable(), value.cost);
                Node {
                    field: named(term, DataType::Boolean, nullable),
                    kind: Kind::Glob(Box::new(value), pattern),
                    cost,
                }
            }
            Term::Arithmetic(left, operator, right) => {
                let field = term.field(self.schema)?;
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
            Term::Negate(inner) => {
                let field = term.field(self.schema)?;
                let inner = self.lower(inner, None)?;
                let cost = inner.cost;
                Node {
                    field,
                    kind: Kind::Negate(Box::new(inner)),
                    cost,
                }
            }
            Term::Function(function, arguments) => {
                let field = term.field(self.schema)?;
                let mut lowered = Vec::with_capacity(arguments.len());
                let unified = matches!(function, Function::Coalesce | Function::IfNull)
                    .then(|| field.dtype().clone());
                for argument in arguments.iter() {
                    lowered.push(self.lower(argument, unified.as_ref())?);
                }
                let cost = lowered.iter().map(|node| node.cost).sum::<u32>() + 1;
                Node {
                    field,
                    kind: Kind::Function(function.clone(), lowered),
                    cost,
                }
            }
            Term::Cast(inner, dtype, safety) => {
                let inner = self.lower(inner, None)?;
                let nullable = inner.field.is_nullable() || matches!(safety, Safety::Safe);
                let cost = inner.cost + 1;
                Node {
                    field: named(term, dtype.clone(), nullable),
                    kind: Kind::Cast(Box::new(inner), *safety),
                    cost,
                }
            }
            Term::Case {
                branches,
                otherwise,
            } => {
                let field = term.field(self.schema)?;
                let target = field.dtype().clone();
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
            Term::Struct(children) => {
                let field = term.field(self.schema)?;
                let mut lowered = Vec::with_capacity(children.len());
                for (index, (_, value)) in children.iter().enumerate() {
                    let target = field.get_field(index).map(|held| held.dtype().clone());
                    lowered.push(self.lower(value, target.as_ref())?);
                }
                let cost = lowered.iter().map(|node| node.cost).sum::<u32>();
                Node {
                    field,
                    kind: Kind::Struct(lowered),
                    cost,
                }
            }
            Term::List(items) => {
                let field = term.field(self.schema)?;
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
            Term::Map(entries) => {
                let field = term.field(self.schema)?;
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

    /// Convert a lowered node into `want`, when it is not already there.
    fn coerce(&self, node: Node, want: Option<&DataType>) -> Result<Node> {
        let _ = self;
        let Some(want) = want else {
            return Ok(node);
        };
        if node.field.dtype() == want {
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
                    .try_with_dtype(want.clone())?
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
            .try_with_dtype(want.clone())?
            .with_nullable(nullable);
        Ok(Node {
            field,
            kind: Kind::Cast(Box::new(node), Safety::Strict),
            cost,
        })
    }

    fn type_of(&self, term: &Term) -> Result<DataType> {
        Ok(term.field(self.schema)?.dtype().clone())
    }

    /// The type two compared operands meet in.
    ///
    /// A literal is narrowed into the other side's type when it fits exactly,
    /// which is what keeps `int32_column = 1` an `int32` comparison instead of
    /// widening a whole column to `int64` per batch. When it does not fit, the
    /// promotion table decides and neither side loses anything.
    fn shared_type(&self, left: &Term, right: &Term) -> Result<DataType> {
        let left_type = self.type_of(left)?;
        let right_type = self.type_of(right)?;
        for (constant, other) in [(left, &right_type), (right, &left_type)] {
            if let Some(held) = self.constant(constant)? {
                // A constant that the other operand's type holds exactly is
                // read in that type: `ts > '2024-01-01'` compares timestamps,
                // `id = '7'` compares integers, `s = 1` compares text. The
                // conversion is exact and checked both ways, so a constant a
                // column cannot hold is never quietly rounded into it.
                if held.value().is_null() {
                    continue;
                }
                if fits(other, &held) {
                    return Ok(other.clone());
                }
            }
        }
        // Two operands with no common type still compare: as text, which
        // every value spells. That is the best-effort reading this crate
        // takes everywhere - a comparison that could mean something is not
        // refused for the shape it was written in.
        Ok(common_type(&left_type, &right_type).unwrap_or_else(DataType::utf8))
    }

    /// The type a value is compared with a list of operands in.
    ///
    /// The value's own type wins when every other operand is a constant it
    /// can hold without loss - the narrowing [`Self::shared_type`] makes for
    /// one comparison - so `n in (1, 2)` reads the column as it is stored and
    /// its statistics stay readable. Otherwise the operands meet at their
    /// common type, or are refused when they have none.
    fn list_type(&self, value: &Term, others: &[Term]) -> Result<DataType> {
        let value_type = self.type_of(value)?;
        let mut shared = Some(value_type.clone());
        let mut narrow = true;
        for other in others {
            let other_type = self.type_of(other)?;
            shared = shared.and_then(|held| common_type(&held, &other_type));
            narrow = narrow
                && self
                    .constant(other)?
                    .is_some_and(|held| held.value().is_null() || fits(&value_type, &held));
        }
        if narrow {
            return Ok(value_type);
        }
        // As for one comparison: operands that share no type meet as text.
        Ok(shared.unwrap_or_else(DataType::utf8))
    }

    /// The constant a term is, when it reads no row and no holder.
    ///
    /// A literal is itself; anything else that reads nothing - `2 * 50`, a
    /// cast of a literal, a parameter already substituted - is evaluated
    /// here, so what it compares with is chosen against its value and not
    /// against the widest type its spelling could have had.
    fn constant(&self, term: &Term) -> Result<Option<Literal>> {
        if let Term::Literal(held) = term {
            return Ok(Some(held.clone()));
        }
        if !term.columns().is_empty() || term.has_attributes() || !term.parameters().is_empty() {
            return Ok(None);
        }
        let node = fold(self.lower(term, None)?)?;
        let Kind::Literal(value) = &node.kind else {
            return Ok(None);
        };
        Ok(Literal::new(node.field.dtype().clone(), value.clone()).ok())
    }

    /// Read the constant pattern a match operator requires.
    fn constant_pattern(&self, pattern: &Term, operator: &str) -> Result<SmolStr> {
        let lowered = self.lower(pattern, Some(&DataType::utf8()))?;
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
fn fits(dtype: &DataType, held: &Literal) -> bool {
    let Ok(converted) = convert(dtype, held.value(), Safety::Strict) else {
        return false;
    };
    // Text is read, not rounded: a spelling the datatype parses is exactly
    // the value it parses to, whatever canonical spelling it prints back as.
    // A registered code is text by kind and reads a spelling the same way -
    // `state` reads the wire code `F` as the state it names - so it stands
    // with the datatypes that parse rather than with the text that compares.
    if super::typing::is_text(held.dtype()) && (!super::typing::is_text(dtype) || dtype.is_code()) {
        return true;
    }
    // A conversion that cannot be undone lost something, and a lost digit
    // turns `=` into a quiet lie.
    convert(held.dtype(), &converted, Safety::Strict).is_ok_and(|back| &back == held.value())
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
    Some(field.dtype().clone())
}

/// The declared element type of a list field.
fn list_item_type(field: &Field) -> Option<DataType> {
    super::path::list_item(field.dtype()).map(|item| item.dtype().clone())
}

/// The declared key and value types of a map field.
fn map_entry_types(field: &Field) -> (Option<DataType>, Option<DataType>) {
    match field.dtype() {
        DataType::Mapping(map) => (
            map.entries().get_field(0).map(|held| held.dtype().clone()),
            map.entries().get_field(1).map(|held| held.dtype().clone()),
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

fn incompatible(expected: &str) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: format_smolstr!("expected {expected}"),
    }
}

/// Rebuild the term a resolved tree stands for.
///
/// The result is what actually runs: folded, ordered, and with every literal in
/// the type it will be compared in. Printing it is how a caller sees what bind
/// decided without a second representation to keep in step.
pub(crate) fn rebuild(node: &Node) -> Term {
    match &node.kind {
        Kind::Literal(value) => Literal::new(node.field.dtype().clone(), value.clone())
            .map_or_else(|_| Term::literal(value.clone()), Term::Literal),
        Kind::Column(_) => Term::column(node.field.name()),
        // A bound path always starts at a column, and a path extends by any
        // step, so this cannot refuse.
        Kind::Path(base, steps) => rebuild(base)
            .path(steps.iter().map(Step::segment))
            .expect("a bound path starts at a column"),
        Kind::Attribute(attribute) => Term::attribute(attribute.clone()),
        Kind::And(operands) => Term::And(operands.iter().map(rebuild).collect()),
        Kind::Or(operands) => Term::Or(operands.iter().map(rebuild).collect()),
        Kind::Not(inner) => Term::Not(Box::new(rebuild(inner))),
        Kind::Compare(left, comparison, right) => Term::Compare(
            Box::new(rebuild(left)),
            *comparison,
            Box::new(rebuild(right)),
        ),
        Kind::In(value, list) => {
            Term::In(Box::new(rebuild(value)), list.iter().map(rebuild).collect())
        }
        Kind::Between(value, low, high) => Term::Between(
            Box::new(rebuild(value)),
            Box::new(rebuild(low)),
            Box::new(rebuild(high)),
        ),
        Kind::IsNull(inner) => Term::IsNull(Box::new(rebuild(inner))),
        Kind::IsNotNull(inner) => Term::IsNotNull(Box::new(rebuild(inner))),
        Kind::Like {
            value,
            pattern,
            case_insensitive,
            escape,
        } => Term::Like {
            value: Box::new(rebuild(value)),
            pattern: Box::new(Term::literal(Scalar::from(pattern.clone()))),
            case_insensitive: *case_insensitive,
            escape: *escape,
        },
        Kind::Glob(value, pattern) => Term::Glob(
            Box::new(rebuild(value)),
            Box::new(Term::literal(Scalar::from(pattern.clone()))),
        ),
        Kind::Arithmetic(left, operator, right) => {
            Term::Arithmetic(Box::new(rebuild(left)), *operator, Box::new(rebuild(right)))
        }
        Kind::Negate(inner) => Term::Negate(Box::new(rebuild(inner))),
        Kind::Function(function, arguments) => {
            Term::Function(function.clone(), arguments.iter().map(rebuild).collect())
        }
        Kind::Cast(inner, safety) => Term::Cast(
            Box::new(rebuild(inner)),
            node.field.dtype().clone(),
            *safety,
        ),
        Kind::Case {
            branches,
            otherwise,
        } => Term::Case {
            branches: branches
                .iter()
                .map(|(when, then)| (rebuild(when), rebuild(then)))
                .collect(),
            otherwise: otherwise.as_ref().map(|held| Box::new(rebuild(held))),
        },
        Kind::Struct(children) => Term::Struct(
            node.field
                .fields()
                .iter()
                .map(|field| SmolStr::new(field.name()))
                .zip(children.iter().map(rebuild))
                .collect(),
        ),
        Kind::List(items) => Term::List(items.iter().map(rebuild).collect()),
        Kind::Map(entries) => Term::Map(
            entries
                .iter()
                .map(|(key, value)| (rebuild(key), rebuild(value)))
                .collect(),
        ),
    }
}
