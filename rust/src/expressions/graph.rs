//! The plan graph: one arena DAG with children *and* parents.
//!
//! A rewrite engine over a bare tree can only ever append micro-rules. It
//! cannot ask "is this node read anywhere else", "where else is this column
//! compared", or "is this subexpression already computed" - and those three
//! questions are where the real optimizations live. So binding produces this
//! instead: an arena of nodes indexed by [`NodeId`], each holding its children
//! in order and the set of nodes that read it, with structurally identical
//! subtrees interned to one id.
//!
//! Interning is what makes the graph a DAG rather than a tree, and it buys
//! three things at build time for free:
//!
//! - **common subexpressions** collapse, so `price * 1.1` written three times
//!   is one node evaluated once per batch;
//! - **duplicate conjuncts** become [`NodeId`] equality rather than a deep
//!   structural compare;
//! - **cycles are impossible**, because a child is always an
//!   already-inserted, lower id - no back-edges and no run-time cycle check.
//!
//! Rewriting is a bottom-up rebuild through the same interning table rather
//! than in-place pointer surgery. That is a deliberate trade: interning makes a
//! node's identity its content, so mutating a node in place would silently
//! change every other reader of the same content, and the rebuild keeps the
//! invariant by construction. The parent sets are what the *rules* consult -
//! whether a rewrite would be seen by more than one reader, and whether a node
//! is still read at all.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use smol_str::SmolStr;

use super::bound::ColumnRef;
use super::{ArithOp, CompareOp, Expr, Function};
use crate::{DataType, Value};

/// One node's place in a [`Plan`]'s arena.
///
/// An id is only meaningful inside the plan that produced it; indexing one plan
/// with another's id is the single documented panic in this module.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct NodeId(u32);

impl NodeId {
    /// Name an arena offset directly.
    ///
    /// Only the arena itself and its own tests build one: an id is meaningful
    /// exactly inside the plan that produced it, which is why there is no
    /// public constructor for one.
    #[must_use]
    #[cfg(test)]
    pub(super) const fn from_index(index: usize) -> Self {
        Self(index as u32)
    }

    /// The arena offset this id addresses.
    #[must_use]
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "#{}", self.0)
    }
}

/// One resolved node of a plan.
///
/// This is the [`Expr`] vocabulary with every child replaced by a [`NodeId`]
/// and every column replaced by a reference that may already be resolved to a
/// slot chain. It is deliberately the *same* vocabulary: a plan that could
/// express something an expression cannot would be a second language.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Node {
    /// A column, resolved to a slot chain once the plan has a schema.
    Column(Arc<ColumnRef>),
    /// A constant, folded into the column's own type where one was in reach.
    Literal(Value),
    /// A schema-directed cast.
    Cast {
        /// What is converted.
        child: NodeId,
        /// The target type.
        data_type: DataType,
        /// Whether an unconvertible value becomes null rather than an error.
        safe: bool,
    },
    /// A comparison.
    Compare {
        /// Which comparison.
        op: CompareOp,
        /// The left operand.
        left: NodeId,
        /// The right operand.
        right: NodeId,
    },
    /// Conjunction; empty is `TRUE`.
    And(Vec<NodeId>),
    /// Disjunction; empty is `FALSE`.
    Or(Vec<NodeId>),
    /// Three-valued negation.
    Not(NodeId),
    /// `IS NULL`.
    IsNull(NodeId),
    /// `IS NOT NULL`.
    IsNotNull(NodeId),
    /// Set membership.
    In {
        /// The value looked up.
        child: NodeId,
        /// The set.
        list: Vec<NodeId>,
        /// Whether membership is negated.
        negated: bool,
    },
    /// An inclusive range test.
    Between {
        /// The value bounded.
        child: NodeId,
        /// The inclusive lower bound.
        low: NodeId,
        /// The inclusive upper bound.
        high: NodeId,
        /// Whether the bound is negated.
        negated: bool,
    },
    /// A wildcard text match.
    Like {
        /// The text matched.
        child: NodeId,
        /// The pattern.
        pattern: NodeId,
        /// The wildcard escape, when one was named.
        escape: Option<char>,
        /// Whether the match is negated.
        negated: bool,
        /// Whether the match ignores ASCII case.
        case_insensitive: bool,
    },
    /// A literal prefix test - the one text predicate statistics can prune.
    StartsWith {
        /// The text tested.
        child: NodeId,
        /// The required prefix.
        prefix: SmolStr,
    },
    /// Arithmetic.
    Arithmetic {
        /// Which operator.
        op: ArithOp,
        /// The left operand.
        left: NodeId,
        /// The right operand.
        right: NodeId,
    },
    /// Arithmetic negation.
    Neg(NodeId),
    /// One of the closed set of scalar functions.
    Function {
        /// Which function.
        name: Function,
        /// Its arguments, in order.
        args: Vec<NodeId>,
    },
    /// The one conditional.
    Case {
        /// The `WHEN`/`THEN` pairs, tried in order.
        branches: Vec<(NodeId, NodeId)>,
        /// The `ELSE` value; absent means null.
        otherwise: Option<NodeId>,
    },
    /// A name for the column this node computes.
    Alias {
        /// What is named.
        child: NodeId,
        /// The name.
        name: SmolStr,
    },
}

impl Node {
    /// Visit every child id of this node, in evaluation order.
    pub(super) fn for_each_child(&self, mut visit: impl FnMut(NodeId)) {
        match self {
            Self::Column(_) | Self::Literal(_) => {}
            Self::Cast { child, .. }
            | Self::Not(child)
            | Self::IsNull(child)
            | Self::IsNotNull(child)
            | Self::Neg(child)
            | Self::Alias { child, .. }
            | Self::StartsWith { child, .. } => visit(*child),
            Self::Compare { left, right, .. } | Self::Arithmetic { left, right, .. } => {
                visit(*left);
                visit(*right);
            }
            Self::And(operands) | Self::Or(operands) => operands.iter().copied().for_each(visit),
            Self::In { child, list, .. } => {
                visit(*child);
                list.iter().copied().for_each(visit);
            }
            Self::Between {
                child, low, high, ..
            } => {
                visit(*child);
                visit(*low);
                visit(*high);
            }
            Self::Like { child, pattern, .. } => {
                visit(*child);
                visit(*pattern);
            }
            Self::Function { args, .. } => args.iter().copied().for_each(visit),
            Self::Case {
                branches,
                otherwise,
            } => {
                for (when, then) in branches {
                    visit(*when);
                    visit(*then);
                }
                if let Some(otherwise) = otherwise {
                    visit(*otherwise);
                }
            }
        }
    }

    /// Return this node with every child id mapped through `remap`.
    ///
    /// One function rebuilds every variant, so a rule never has to spell out
    /// how a node is reassembled and a new variant is wired in exactly once.
    pub(super) fn map_children(&self, mut remap: impl FnMut(NodeId) -> NodeId) -> Self {
        match self {
            Self::Column(column) => Self::Column(Arc::clone(column)),
            Self::Literal(value) => Self::Literal(value.clone()),
            Self::Cast {
                child,
                data_type,
                safe,
            } => Self::Cast {
                child: remap(*child),
                data_type: data_type.clone(),
                safe: *safe,
            },
            Self::Compare { op, left, right } => Self::Compare {
                op: *op,
                left: remap(*left),
                right: remap(*right),
            },
            Self::And(operands) => Self::And(operands.iter().map(|id| remap(*id)).collect()),
            Self::Or(operands) => Self::Or(operands.iter().map(|id| remap(*id)).collect()),
            Self::Not(child) => Self::Not(remap(*child)),
            Self::IsNull(child) => Self::IsNull(remap(*child)),
            Self::IsNotNull(child) => Self::IsNotNull(remap(*child)),
            Self::In {
                child,
                list,
                negated,
            } => Self::In {
                child: remap(*child),
                list: list.iter().map(|id| remap(*id)).collect(),
                negated: *negated,
            },
            Self::Between {
                child,
                low,
                high,
                negated,
            } => Self::Between {
                child: remap(*child),
                low: remap(*low),
                high: remap(*high),
                negated: *negated,
            },
            Self::Like {
                child,
                pattern,
                escape,
                negated,
                case_insensitive,
            } => Self::Like {
                child: remap(*child),
                pattern: remap(*pattern),
                escape: *escape,
                negated: *negated,
                case_insensitive: *case_insensitive,
            },
            Self::StartsWith { child, prefix } => Self::StartsWith {
                child: remap(*child),
                prefix: prefix.clone(),
            },
            Self::Arithmetic { op, left, right } => Self::Arithmetic {
                op: *op,
                left: remap(*left),
                right: remap(*right),
            },
            Self::Neg(child) => Self::Neg(remap(*child)),
            Self::Function { name, args } => Self::Function {
                name: *name,
                args: args.iter().map(|id| remap(*id)).collect(),
            },
            Self::Case {
                branches,
                otherwise,
            } => Self::Case {
                branches: branches
                    .iter()
                    .map(|(when, then)| (remap(*when), remap(*then)))
                    .collect(),
                otherwise: otherwise.map(&mut remap),
            },
            Self::Alias { child, name } => Self::Alias {
                child: remap(*child),
                name: name.clone(),
            },
        }
    }

    /// The literal this node holds, if it is one.
    #[must_use]
    pub const fn as_literal(&self) -> Option<&Value> {
        match self {
            Self::Literal(value) => Some(value),
            _ => None,
        }
    }

    /// The column this node reads, if it reads one directly.
    #[must_use]
    pub const fn as_column(&self) -> Option<&Arc<ColumnRef>> {
        match self {
            Self::Column(column) => Some(column),
            _ => None,
        }
    }

    /// A short stable name for this node's kind, for the plan's `Display`.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Column(_) => "column",
            Self::Literal(_) => "literal",
            Self::Cast { .. } => "cast",
            Self::Compare { .. } => "compare",
            Self::And(_) => "and",
            Self::Or(_) => "or",
            Self::Not(_) => "not",
            Self::IsNull(_) => "is_null",
            Self::IsNotNull(_) => "is_not_null",
            Self::In { .. } => "in",
            Self::Between { .. } => "between",
            Self::Like { .. } => "like",
            Self::StartsWith { .. } => "starts_with",
            Self::Arithmetic { .. } => "arithmetic",
            Self::Neg(_) => "neg",
            Self::Function { .. } => "function",
            Self::Case { .. } => "case",
            Self::Alias { .. } => "alias",
        }
    }
}

/// The arena every bound expression and every rewrite rule reads.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    nodes: Vec<Node>,
    parents: Vec<Vec<NodeId>>,
    types: Vec<Option<DataType>>,
    interned: HashMap<Node, NodeId>,
    /// Comparison-shaped nodes by the lowercased column they read.
    ///
    /// "Every comparison on `venue`" is a lookup rather than a walk, which is
    /// what lets range and set coalescing run to a fixed point cheaply.
    by_column: HashMap<SmolStr, Vec<NodeId>>,
}

impl Plan {
    /// An empty arena.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many distinct nodes the arena holds.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Return whether the arena holds no nodes.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Borrow one node.
    #[must_use]
    #[inline]
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index())
    }

    /// Borrow the nodes that read one node.
    ///
    /// A node with one parent may be rewritten where it stands; a node with
    /// several is read by more than one place, so a rule that would change
    /// what it computes has to decline or clone first; a node with none is
    /// dead and is never evaluated.
    #[must_use]
    #[inline]
    pub fn parents(&self, id: NodeId) -> &[NodeId] {
        self.parents
            .get(id.index())
            .map_or(&[] as &[NodeId], Vec::as_slice)
    }

    /// Return whether more than one node reads this one.
    #[must_use]
    #[inline]
    pub fn is_shared(&self, id: NodeId) -> bool {
        self.parents(id).len() > 1
    }

    /// The datatype this node evaluates to, once the plan has been bound.
    #[must_use]
    #[inline]
    pub fn data_type(&self, id: NodeId) -> Option<&DataType> {
        self.types.get(id.index()).and_then(Option::as_ref)
    }

    /// Record the datatype a node evaluates to.
    pub(super) fn set_data_type(&mut self, id: NodeId, data_type: DataType) {
        if let Some(slot) = self.types.get_mut(id.index()) {
            *slot = Some(data_type);
        }
    }

    /// Every comparison-shaped node reading the named column.
    #[must_use]
    pub fn comparisons_on(&self, column: &str) -> &[NodeId] {
        self.by_column
            .get(&SmolStr::new(column.to_ascii_lowercase()))
            .map_or(&[] as &[NodeId], Vec::as_slice)
    }

    /// Intern one node, returning the id of the structurally identical node.
    ///
    /// Every child must already be inserted, which is what makes a cycle
    /// unrepresentable rather than merely rejected.
    pub(super) fn insert(&mut self, node: Node) -> NodeId {
        if let Some(existing) = self.interned.get(&node) {
            return *existing;
        }
        let id = NodeId(u32::try_from(self.nodes.len()).unwrap_or(u32::MAX));
        node.for_each_child(|child| {
            if let Some(parents) = self.parents.get_mut(child.index()) {
                if !parents.contains(&id) {
                    parents.push(id);
                }
            }
        });
        if let Some(column) = self.comparison_column(&node) {
            self.by_column.entry(column).or_default().push(id);
        }
        self.interned.insert(node.clone(), id);
        self.nodes.push(node);
        self.parents.push(Vec::new());
        self.types.push(None);
        id
    }

    /// The column a comparison-shaped node reads, lowercased for lookup.
    ///
    /// Only the shapes the coalescing rules search for are indexed - a
    /// comparison, a set membership, a range, and a prefix test - because those
    /// are the ones a rule asks "what else touches this column" about.
    fn comparison_column(&self, node: &Node) -> Option<SmolStr> {
        let child = match node {
            Node::Compare { left, right, .. } => {
                let left_column = self.column_name(*left);
                if left_column.is_some() {
                    left_column
                } else {
                    self.column_name(*right)
                }
            }
            Node::In { child, .. }
            | Node::Between { child, .. }
            | Node::StartsWith { child, .. }
            | Node::IsNull(child)
            | Node::IsNotNull(child) => self.column_name(*child),
            _ => None,
        }?;
        Some(SmolStr::new(child.to_ascii_lowercase()))
    }

    /// The bare column name a node reads, if it reads one directly.
    fn column_name(&self, id: NodeId) -> Option<&str> {
        self.get(id)?.as_column().map(|column| column.name())
    }

    /// Insert an unbound expression, resolving nothing.
    ///
    /// This is what the schema-free [`Expr::simplify`](super::Expr::simplify)
    /// runs over: the same graph and the same rules, with every column left
    /// unresolved and every literal left in the type it was written with.
    pub(super) fn insert_expr(&mut self, expr: &Expr) -> NodeId {
        self.insert_expr_with(expr, &mut |_| None)
    }

    /// Insert an expression, letting `resolve` supply a bound column.
    pub(super) fn insert_expr_with(
        &mut self,
        expr: &Expr,
        resolve: &mut impl FnMut(&super::Column) -> Option<Arc<ColumnRef>>,
    ) -> NodeId {
        let node = match expr {
            Expr::Column(column) => Node::Column(
                resolve(column).unwrap_or_else(|| Arc::new(ColumnRef::unresolved(column.clone()))),
            ),
            Expr::Literal(value) => Node::Literal(value.clone()),
            Expr::Cast {
                expr,
                data_type,
                safe,
            } => Node::Cast {
                child: self.insert_expr_with(expr, resolve),
                data_type: data_type.clone(),
                safe: *safe,
            },
            Expr::Compare { op, left, right } => Node::Compare {
                op: *op,
                left: self.insert_expr_with(left, resolve),
                right: self.insert_expr_with(right, resolve),
            },
            Expr::And(operands) => Node::And(
                operands
                    .iter()
                    .map(|operand| self.insert_expr_with(operand, resolve))
                    .collect(),
            ),
            Expr::Or(operands) => Node::Or(
                operands
                    .iter()
                    .map(|operand| self.insert_expr_with(operand, resolve))
                    .collect(),
            ),
            Expr::Not(child) => Node::Not(self.insert_expr_with(child, resolve)),
            Expr::IsNull(child) => Node::IsNull(self.insert_expr_with(child, resolve)),
            Expr::IsNotNull(child) => Node::IsNotNull(self.insert_expr_with(child, resolve)),
            Expr::In {
                expr,
                list,
                negated,
            } => Node::In {
                child: self.insert_expr_with(expr, resolve),
                list: list
                    .iter()
                    .map(|item| self.insert_expr_with(item, resolve))
                    .collect(),
                negated: *negated,
            },
            Expr::Between {
                expr,
                low,
                high,
                negated,
            } => Node::Between {
                child: self.insert_expr_with(expr, resolve),
                low: self.insert_expr_with(low, resolve),
                high: self.insert_expr_with(high, resolve),
                negated: *negated,
            },
            Expr::Like {
                expr,
                pattern,
                escape,
                negated,
                case_insensitive,
            } => Node::Like {
                child: self.insert_expr_with(expr, resolve),
                pattern: self.insert_expr_with(pattern, resolve),
                escape: *escape,
                negated: *negated,
                case_insensitive: *case_insensitive,
            },
            Expr::StartsWith { expr, prefix } => Node::StartsWith {
                child: self.insert_expr_with(expr, resolve),
                prefix: prefix.clone(),
            },
            Expr::Arithmetic { op, left, right } => Node::Arithmetic {
                op: *op,
                left: self.insert_expr_with(left, resolve),
                right: self.insert_expr_with(right, resolve),
            },
            Expr::Neg(child) => Node::Neg(self.insert_expr_with(child, resolve)),
            Expr::Function { name, args } => Node::Function {
                name: *name,
                args: args
                    .iter()
                    .map(|arg| self.insert_expr_with(arg, resolve))
                    .collect(),
            },
            Expr::Case {
                branches,
                otherwise,
            } => Node::Case {
                branches: branches
                    .iter()
                    .map(|(when, then)| {
                        (
                            self.insert_expr_with(when, resolve),
                            self.insert_expr_with(then, resolve),
                        )
                    })
                    .collect(),
                otherwise: otherwise
                    .as_ref()
                    .map(|otherwise| self.insert_expr_with(otherwise, resolve)),
            },
            Expr::Alias { expr, name } => Node::Alias {
                child: self.insert_expr_with(expr, resolve),
                name: name.clone(),
            },
        };
        self.insert(node)
    }

    /// Read one node back out as the expression value it names.
    #[must_use]
    pub fn to_expr(&self, id: NodeId) -> Expr {
        let Some(node) = self.get(id) else {
            return Expr::always_true();
        };
        match node {
            Node::Column(column) => Expr::Column(column.column().clone()),
            Node::Literal(value) => Expr::Literal(value.clone()),
            Node::Cast {
                child,
                data_type,
                safe,
            } => Expr::Cast {
                expr: Arc::new(self.to_expr(*child)),
                data_type: data_type.clone(),
                safe: *safe,
            },
            Node::Compare { op, left, right } => Expr::Compare {
                op: *op,
                left: Arc::new(self.to_expr(*left)),
                right: Arc::new(self.to_expr(*right)),
            },
            Node::And(operands) => Expr::all(operands.iter().map(|operand| self.to_expr(*operand))),
            Node::Or(operands) => Expr::any(operands.iter().map(|operand| self.to_expr(*operand))),
            Node::Not(child) => Expr::Not(Arc::new(self.to_expr(*child))),
            Node::IsNull(child) => Expr::IsNull(Arc::new(self.to_expr(*child))),
            Node::IsNotNull(child) => Expr::IsNotNull(Arc::new(self.to_expr(*child))),
            Node::In {
                child,
                list,
                negated,
            } => Expr::In {
                expr: Arc::new(self.to_expr(*child)),
                list: list.iter().map(|item| self.to_expr(*item)).collect(),
                negated: *negated,
            },
            Node::Between {
                child,
                low,
                high,
                negated,
            } => Expr::Between {
                expr: Arc::new(self.to_expr(*child)),
                low: Arc::new(self.to_expr(*low)),
                high: Arc::new(self.to_expr(*high)),
                negated: *negated,
            },
            Node::Like {
                child,
                pattern,
                escape,
                negated,
                case_insensitive,
            } => Expr::Like {
                expr: Arc::new(self.to_expr(*child)),
                pattern: Arc::new(self.to_expr(*pattern)),
                escape: *escape,
                negated: *negated,
                case_insensitive: *case_insensitive,
            },
            Node::StartsWith { child, prefix } => Expr::StartsWith {
                expr: Arc::new(self.to_expr(*child)),
                prefix: prefix.clone(),
            },
            Node::Arithmetic { op, left, right } => Expr::Arithmetic {
                op: *op,
                left: Arc::new(self.to_expr(*left)),
                right: Arc::new(self.to_expr(*right)),
            },
            Node::Neg(child) => Expr::Neg(Arc::new(self.to_expr(*child))),
            Node::Function { name, args } => Expr::Function {
                name: *name,
                args: args.iter().map(|arg| self.to_expr(*arg)).collect(),
            },
            Node::Case {
                branches,
                otherwise,
            } => Expr::Case {
                branches: branches
                    .iter()
                    .map(|(when, then)| (self.to_expr(*when), self.to_expr(*then)))
                    .collect(),
                otherwise: otherwise
                    .as_ref()
                    .map(|otherwise| Arc::new(self.to_expr(*otherwise))),
            },
            Node::Alias { child, name } => Expr::Alias {
                expr: Arc::new(self.to_expr(*child)),
                name: name.clone(),
            },
        }
    }

    /// Every node reachable from `root`, in topological order.
    ///
    /// A node absent from this list is dead: nothing reads it, so nothing
    /// evaluates it. Collecting dead nodes is therefore a reachability walk
    /// rather than a refcount sweep, and the evaluator never sees one.
    #[must_use]
    pub fn reachable(&self, root: NodeId) -> Vec<NodeId> {
        let mut seen = vec![false; self.nodes.len()];
        let mut order = Vec::new();
        let mut pending = vec![(root, false)];
        while let Some((id, expanded)) = pending.pop() {
            if expanded {
                order.push(id);
                continue;
            }
            if seen.get(id.index()).copied().unwrap_or(true) {
                continue;
            }
            if let Some(slot) = seen.get_mut(id.index()) {
                *slot = true;
            }
            pending.push((id, true));
            if let Some(node) = self.get(id) {
                node.for_each_child(|child| pending.push((child, false)));
            }
        }
        order
    }

    /// Render the nodes reachable from `root` in topological order.
    ///
    /// The rendering is stable, so a plan is snapshot-testable and an
    /// optimizer regression shows up as a diff a reviewer can read.
    #[must_use]
    pub fn explain_from(&self, root: NodeId) -> String {
        use std::fmt::Write;

        let mut text = String::new();
        for id in self.reachable(root) {
            let Some(node) = self.get(id) else { continue };
            let mut children = Vec::new();
            node.for_each_child(|child| children.push(child.to_string()));
            let readers = self.parents(id).len();
            let _ = writeln!(
                text,
                "{id} {kind}({children}) readers={readers} = {expr}",
                kind = node.kind(),
                children = children.join(", "),
                expr = self.to_expr(id),
            );
        }
        text
    }
}

impl std::ops::Index<NodeId> for Plan {
    type Output = Node;

    /// Borrow a node, panicking on an id this plan did not produce.
    ///
    /// This is the one place in the module where a panic is normal, and it is
    /// normal for the reason `Index` always is: an id from another plan is a
    /// programming error rather than a caller-controlled input.
    fn index(&self, id: NodeId) -> &Node {
        self.get(id)
            .unwrap_or_else(|| panic!("{id} does not belong to this plan"))
    }
}
