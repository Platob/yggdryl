//! `explain`: the plan as a tree a person can read.
//!
//! [`Display`](std::fmt::Display) prints an expression as the one line that
//! parses back; this prints it as the tree it is, one node per line, branches
//! drawn with box-drawing characters. An unbound term shows its structure. A
//! bound one shows what binding decided: each node's output datatype and
//! nullability, the column index a column resolved to, the literal a
//! constant folded into, and the cost class that ordered the operands.
//!
//! ```
//! use yggdryl::{DataType, StructureType, Term};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let schema = DataType::from(StructureType::from_fields([
//!     DataType::utf8().nullable_field("ccy"),
//!     DataType::Int64.nullable_field("size"),
//! ])?)
//! .required_field("trades");
//! let bound: Term = "ccy = 'EUR' and size > 2 * 50".parse::<Term>()?.bind(&schema)?.term().clone();
//! assert_eq!(
//!     bound.explain(),
//!     [
//!         "and",
//!         "├─ =",
//!         "│  ├─ column ccy",
//!         "│  └─ literal 'EUR'",
//!         "└─ >",
//!         "   ├─ column size",
//!         "   └─ literal 100",
//!     ]
//!     .join("\n")
//! );
//! # Ok(())
//! # }
//! ```

use std::fmt::Write as _;

use super::bind::{Bound, Kind, Node, Step, StepKind};
use super::filter::Filter;
use super::path::{FieldPath, FieldSegment};
use super::plan::{Ordering, Plan, Source, Target, Write};
use super::selector::{BoundSelector, Projection, Selector};
use super::term::Term;
use super::{Expression, Literal, Safety};
use crate::Field;

/// One node of the rendered tree: a label and what hangs under it.
#[derive(Debug, Default)]
struct Tree {
    label: String,
    children: Vec<Tree>,
}

impl Tree {
    fn leaf(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            children: Vec::new(),
        }
    }

    fn node(label: impl Into<String>, children: Vec<Self>) -> Self {
        Self {
            label: label.into(),
            children,
        }
    }

    /// Render this tree, the root unindented and every branch drawn.
    fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&self.label);
        self.render_children(&mut out, "");
        out
    }

    fn render_children(&self, out: &mut String, prefix: &str) {
        let last = self.children.len().saturating_sub(1);
        for (index, child) in self.children.iter().enumerate() {
            let branch = if index == last { "└─ " } else { "├─ " };
            let _ = write!(out, "\n{prefix}{branch}{}", child.label);
            let deeper = if index == last { "   " } else { "│  " };
            child.render_children(out, &format!("{prefix}{deeper}"));
        }
    }
}

/// The steps of a path, spelled as the grammar spells them.
fn steps(held: impl IntoIterator<Item = FieldSegment>) -> String {
    FieldPath::new(held).to_string()
}

/// The label of a path leaf: a bare column, or the path it walks.
fn path_label(held: &[FieldSegment]) -> String {
    match held {
        [FieldSegment::Field(name)] => format!("column {name}"),
        _ => format!("path {}", steps(held.iter().cloned())),
    }
}

/// The branch a predicate segment hangs under its path.
fn where_branch(predicate: Tree) -> Tree {
    Tree::node("where", vec![predicate])
}

impl Term {
    /// The tree of this term, one node per line.
    ///
    /// The module documentation shows the shape.
    #[must_use]
    pub fn explain(&self) -> String {
        self.tree().render()
    }

    fn tree(&self) -> Tree {
        let children = |terms: &[Self]| terms.iter().map(Self::tree).collect::<Vec<_>>();
        match self {
            Self::Literal(literal) => Tree::leaf(format!("literal {literal}")),
            // A predicate segment is the one step with a tree of its own,
            // drawn under the path it keeps elements of.
            Self::Path(held) => Tree::node(
                path_label(held),
                held.iter()
                    .filter_map(FieldSegment::as_predicate)
                    .map(|predicate| where_branch(predicate.tree()))
                    .collect(),
            ),
            Self::Attribute(attribute) => Tree::leaf(format!("attribute &holder.{attribute}")),
            Self::Parameter(name) => Tree::leaf(format!("parameter :{name}")),
            Self::And(operands) => Tree::node("and", children(operands)),
            Self::Or(operands) => Tree::node("or", children(operands)),
            Self::Not(inner) => Tree::node("not", vec![inner.tree()]),
            Self::Compare(left, comparison, right) => {
                Tree::node(comparison.to_string(), vec![left.tree(), right.tree()])
            }
            Self::In(value, list) => {
                Tree::node("in", vec![value.tree(), Tree::node("list", children(list))])
            }
            Self::Between(value, low, high) => {
                Tree::node("between", vec![value.tree(), low.tree(), high.tree()])
            }
            Self::IsNull(inner) => Tree::node("is null", vec![inner.tree()]),
            Self::IsNotNull(inner) => Tree::node("is not null", vec![inner.tree()]),
            Self::Like {
                value,
                pattern,
                case_insensitive,
                escape,
            } => Tree::node(
                like_label(*case_insensitive, *escape),
                vec![value.tree(), pattern.tree()],
            ),
            Self::Glob(value, pattern) => Tree::node("glob", vec![value.tree(), pattern.tree()]),
            Self::Arithmetic(left, operator, right) => {
                Tree::node(operator.to_string(), vec![left.tree(), right.tree()])
            }
            Self::Negate(inner) => Tree::node("negate", vec![inner.tree()]),
            Self::Function(function, arguments) => {
                Tree::node(format!("call {function}"), children(arguments))
            }
            Self::Cast(inner, dtype, safety) => {
                Tree::node(cast_label(dtype, *safety), vec![inner.tree()])
            }
            Self::Case {
                branches,
                otherwise,
            } => Tree::node("case", case_children(branches, otherwise.as_deref())),
            Self::Struct(fields) => Tree::node(
                "struct",
                fields
                    .iter()
                    .map(|(name, value)| Tree::node(format!("{name} ="), vec![value.tree()]))
                    .collect(),
            ),
            Self::List(items) => Tree::node("list", children(items)),
            Self::Map(entries) => Tree::node(
                "map",
                entries
                    .iter()
                    .map(|(key, value)| Tree::node("entry", vec![key.tree(), value.tree()]))
                    .collect(),
            ),
        }
    }
}

fn like_label(case_insensitive: bool, escape: Option<char>) -> String {
    let mut label = String::from(if case_insensitive { "ilike" } else { "like" });
    if let Some(escape) = escape {
        let _ = write!(label, " escape '{escape}'");
    }
    label
}

fn cast_label(dtype: &crate::DataType, safety: Safety) -> String {
    match safety {
        Safety::Strict => format!("cast {dtype}"),
        Safety::Safe => format!("try_cast {dtype}"),
    }
}

fn case_children(branches: &[(Term, Term)], otherwise: Option<&Term>) -> Vec<Tree> {
    let mut children: Vec<Tree> = branches
        .iter()
        .map(|(when, then)| {
            Tree::node(
                "when",
                vec![when.tree(), Tree::node("then", vec![then.tree()])],
            )
        })
        .collect();
    if let Some(otherwise) = otherwise {
        children.push(Tree::node("else", vec![otherwise.tree()]));
    }
    children
}

/// The type suffix of a bound node: its datatype and nullability.
fn typed(field: &Field) -> String {
    let nullability = if field.is_nullable() {
        "null"
    } else {
        "not null"
    };
    format!("{} {nullability}", field.dtype())
}

impl Bound {
    /// The plan as a tree: every node with the datatype it produces, the
    /// column it resolved to, the constant it folded into, and its cost.
    ///
    /// ```
    /// use yggdryl::{DataType, StructureType, Term};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let schema = DataType::from(StructureType::from_fields([DataType::Int32.nullable_field("size")])?)
    ///     .required_field("trades");
    /// let bound = "size > 2 * 50".parse::<Term>()?.bind(&schema)?;
    /// assert_eq!(
    ///     bound.explain(),
    ///     [
    ///         "> : boolean null [cost 1024]",
    ///         "├─ column size #0 : int32 null [cost 1024]",
    ///         "└─ literal int32 '100' : int32 not null [cost 0]",
    ///     ]
    ///     .join("\n")
    /// );
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn explain(&self) -> String {
        node_tree(self.node(), self.schema()).render()
    }
}

fn node_tree(node: &Node, schema: &Field) -> Tree {
    let children = |nodes: &[Node]| {
        nodes
            .iter()
            .map(|held| node_tree(held, schema))
            .collect::<Vec<_>>()
    };
    let one = |held: &Node| node_tree(held, schema);
    let (label, children): (String, Vec<Tree>) = match &node.kind {
        Kind::Literal(value) => (
            format!(
                "literal {}",
                Literal::new(node.field.dtype().clone(), value.clone())
                    .map_or_else(|_| format!("{value:?}"), |literal| literal.to_string())
            ),
            Vec::new(),
        ),
        Kind::Column(index) => (
            format!(
                "column {} #{index}",
                schema
                    .fields()
                    .get(*index)
                    .map_or("?", |field| field.name())
            ),
            Vec::new(),
        ),
        Kind::Path(base, held) => {
            let mut branches = vec![one(base)];
            for step in held {
                if let (StepKind::Where(predicate), Some(element)) = (&step.kind, step.element()) {
                    branches.push(where_branch(node_tree(predicate, element)));
                }
            }
            (
                format!("path {}", steps(held.iter().map(Step::segment))),
                branches,
            )
        }
        Kind::Attribute(attribute) => (format!("attribute &holder.{attribute}"), Vec::new()),
        Kind::And(operands) => ("and".to_owned(), children(operands)),
        Kind::Or(operands) => ("or".to_owned(), children(operands)),
        Kind::Not(inner) => ("not".to_owned(), vec![one(inner)]),
        Kind::Compare(left, comparison, right) => {
            (comparison.to_string(), vec![one(left), one(right)])
        }
        Kind::In(value, list) => (
            "in".to_owned(),
            vec![one(value), Tree::node("list", children(list))],
        ),
        Kind::Between(value, low, high) => {
            ("between".to_owned(), vec![one(value), one(low), one(high)])
        }
        Kind::IsNull(inner) => ("is null".to_owned(), vec![one(inner)]),
        Kind::IsNotNull(inner) => ("is not null".to_owned(), vec![one(inner)]),
        Kind::Like {
            value,
            pattern,
            case_insensitive,
            escape,
        } => (
            format!("{} {pattern:?}", like_label(*case_insensitive, *escape)),
            vec![one(value)],
        ),
        Kind::Glob(value, pattern) => (format!("glob {pattern:?}"), vec![one(value)]),
        Kind::Arithmetic(left, operator, right) => {
            (operator.to_string(), vec![one(left), one(right)])
        }
        Kind::Negate(inner) => ("negate".to_owned(), vec![one(inner)]),
        Kind::Function(function, arguments) => (format!("call {function}"), children(arguments)),
        Kind::Cast(inner, safety) => (cast_label(node.field.dtype(), *safety), vec![one(inner)]),
        Kind::Case {
            branches,
            otherwise,
        } => {
            let mut held: Vec<Tree> = branches
                .iter()
                .map(|(when, then)| {
                    Tree::node("when", vec![one(when), Tree::node("then", vec![one(then)])])
                })
                .collect();
            if let Some(otherwise) = otherwise {
                held.push(Tree::node("else", vec![one(otherwise)]));
            }
            ("case".to_owned(), held)
        }
        Kind::Struct(fields) => (
            "struct".to_owned(),
            node.field
                .fields()
                .iter()
                .zip(fields)
                .map(|(field, value)| Tree::node(format!("{} =", field.name()), vec![one(value)]))
                .collect(),
        ),
        Kind::List(items) => ("list".to_owned(), children(items)),
        Kind::Map(entries) => (
            "map".to_owned(),
            entries
                .iter()
                .map(|(key, value)| Tree::node("entry", vec![one(key), one(value)]))
                .collect(),
        ),
    };
    Tree::node(
        format!("{label} : {} [cost {}]", typed(&node.field), node.cost),
        children,
    )
}

impl Projection {
    fn tree(&self) -> Tree {
        if self.is_column() {
            return self.term().tree();
        }
        let mut label = self.name().to_string();
        if let Some(dtype) = self.dtype() {
            let _ = write!(label, " : {dtype}");
            match self.nullable() {
                Some(true) => label.push_str(" null"),
                Some(false) => label.push_str(" not null"),
                None => {}
            }
        }
        Tree::node(label, vec![self.term().tree()])
    }
}

impl Selector {
    /// The tree of this selector: one branch per projection, named as it is
    /// published, each holding the term that computes it.
    #[must_use]
    pub fn explain(&self) -> String {
        self.tree().render()
    }

    fn tree(&self) -> Tree {
        if self.is_all() {
            return Tree::leaf("select *");
        }
        if self.projections().is_empty() {
            return Tree::node(
                "select * exclude",
                self.excluded()
                    .iter()
                    .map(|name| Tree::leaf(name.to_string()))
                    .collect(),
            );
        }
        Tree::node(
            "select",
            self.projections().iter().map(Projection::tree).collect(),
        )
    }
}

impl BoundSelector {
    /// The plan of this selector: one branch per published column, typed as
    /// the output field declares it, each holding its bound term.
    #[must_use]
    pub fn explain(&self) -> String {
        if self.is_identity() {
            return Tree::leaf("select * (identity, skipped)").render();
        }
        Tree::node(
            "select",
            self.output()
                .fields()
                .iter()
                .zip(self.projections())
                .map(|(field, bound)| {
                    Tree::node(
                        format!("{} : {}", field.name(), typed(field)),
                        vec![node_tree(bound.node(), bound.schema())],
                    )
                })
                .collect(),
        )
        .render()
    }
}

impl Filter {
    /// The tree of this filter, under its `where`.
    #[must_use]
    pub fn explain(&self) -> String {
        self.tree().render()
    }

    fn tree(&self) -> Tree {
        Tree::node("where", vec![self.term().tree()])
    }
}

impl Write {
    fn tree(&self) -> Tree {
        let mut children = Vec::new();
        let label = match self.target() {
            Some(target) => {
                children.extend(properties(target));
                format!("{} {}", self.verb(), target.location())
            }
            None => self.verb().word().to_owned(),
        };
        if !self.merge_by().is_empty() {
            children.push(Tree::node(
                "by",
                self.merge_by()
                    .projections()
                    .iter()
                    .map(Projection::tree)
                    .collect(),
            ));
        }
        Tree::node(label, children)
    }
}

impl Ordering {
    fn tree(&self) -> Tree {
        let mut label = String::from(if self.is_descending() { "desc" } else { "asc" });
        if self.is_nulls_first() {
            label.push_str(" nulls first");
        }
        Tree::node(label, vec![self.term().tree()])
    }
}

impl Plan {
    fn tree(&self) -> Tree {
        let mut children = Vec::new();
        if let Some(schema) = self.schema() {
            let mut declared = Vec::new();
            let label = match self.create_target() {
                Some(target) => {
                    declared.extend(properties(target));
                    format!("create {}", target.location())
                }
                None => "create".to_owned(),
            };
            declared.extend(schema.projections().iter().map(Projection::tree));
            children.push(Tree::node(label, declared));
        }
        if let Some(write) = self.write_section() {
            children.push(write.tree());
        }
        if let Some(from) = self.source() {
            children.push(match from {
                Source::Target(target) => {
                    let mut held = vec![Tree::leaf(target.location().to_string())];
                    held.extend(properties(target));
                    Tree::node("from", held)
                }
                Source::Plan(plan) => Tree::node("from", vec![plan.tree()]),
            });
        }
        if !self.filter_section().is_always_true() {
            children.push(self.filter_section().tree());
        }
        if !self.selector().is_all() {
            children.push(self.selector().tree());
        }
        if !self.ordering().is_empty() {
            children.push(Tree::node(
                "order by",
                self.ordering().iter().map(Ordering::tree).collect(),
            ));
        }
        if let Some(offset) = self.row_offset() {
            children.push(Tree::leaf(format!("offset {offset}")));
        }
        if let Some(limit) = self.row_limit() {
            children.push(Tree::leaf(format!("limit {limit}")));
        }
        Tree::node("plan", children)
    }
}

fn properties(target: &Target) -> Option<Tree> {
    if target.properties().is_empty() {
        return None;
    }
    Some(Tree::node(
        "with",
        target
            .properties()
            .iter()
            .map(|(name, value)| Tree::leaf(format!("{name} = {value:?}")))
            .collect(),
    ))
}

impl Expression {
    /// The tree of this expression, its clause, plan or sequence at the root.
    ///
    /// A plan lists its sections in the order they run: what it creates,
    /// what it writes, where it reads, then `where`, `select`, `order by`,
    /// `offset` and `limit`.
    #[must_use]
    pub fn explain(&self) -> String {
        self.tree().render()
    }

    fn tree(&self) -> Tree {
        match self {
            Self::Selector(selector) => selector.tree(),
            Self::Filter(filter) => filter.tree(),
            Self::Plan(plan) => plan.tree(),
            Self::Sequence(steps) => Tree::node("sequence", steps.iter().map(Self::tree).collect()),
        }
    }
}
