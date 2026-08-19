//! The rewrite rules, and the fixed-point driver that runs them.
//!
//! Every rewrite here is **semantics-preserving under three-valued logic, or it
//! declines**. A slower plan is always acceptable; a different answer never is.
//! Two consequences are worth stating before the rules, because both are places
//! an optimizer usually goes wrong:
//!
//! - `a = a` is **not** `TRUE` and `a <> a` is **not** `FALSE`: both are
//!   unknown when `a` is null. Nothing here folds a comparison whose operands
//!   are not literals, which is what keeps that true by construction.
//! - A contradiction like `a > 5 AND a < 3` is **not** `FALSE` either - it is
//!   unknown when `a` is null, and unknown is not false outside a filter. The
//!   fold therefore fires only where the schema proves the column non-nullable,
//!   and declines everywhere else. The same condition guards an empty set
//!   intersection.
//!
//! Rewriting is a bottom-up rebuild through the plan's interning table rather
//! than in-place pointer surgery: interning makes a node's identity its
//! content, so mutating one in place would silently change every other reader
//! of the same content. See [`graph`](super::graph) for why that trade is the
//! right one.

use std::collections::HashMap;
use std::fmt;

use super::bound::{coerce_value, family};
use super::graph::{Node, NodeId, Plan};
use super::{ArithOp, CompareOp, Value};
use crate::{DataType, Field};

/// How many rebuild passes the driver runs before it stops.
///
/// Hitting the cap is a bug the test suite surfaces rather than a hang in
/// production, which is why it exists at all: a rule pair that undoes itself
/// would otherwise spin.
const MAX_PASSES: usize = 16;

/// How many operands a disjunction may distribute over before CNF is declined.
///
/// Conjunctive normal form is what lets each conjunct be pushed independently,
/// and it is also what explodes on a wide predicate. Past this product the
/// original shape is kept and only what is already extractable is pushed - a
/// plan that pushes less is better than a plan that explodes.
const CNF_PRODUCT_GUARD: usize = 64;

/// Below this many nodes the optimizer is skipped entirely.
///
/// A predicate of two nodes has nothing to gain and still pays for the passes,
/// so the threshold is a measured number rather than a guess - see the
/// `optimize` benchmark group.
const MIN_NODES: usize = 3;

/// What the optimizer did, in the order it did it.
#[derive(Clone, Debug, Default)]
pub struct Explanation {
    fired: Vec<(&'static str, NodeId)>,
    passes: usize,
    declined: Vec<&'static str>,
}

impl Explanation {
    /// The rules that fired, in order, with the node each fired on.
    #[must_use]
    pub fn fired(&self) -> &[(&'static str, NodeId)] {
        &self.fired
    }

    /// How many rebuild passes reached the fixed point.
    #[must_use]
    pub const fn passes(&self) -> usize {
        self.passes
    }

    /// The rules that declined, so a missing rewrite is auditable.
    #[must_use]
    pub fn declined(&self) -> &[&'static str] {
        &self.declined
    }

    /// Record that a rule fired.
    fn fire(&mut self, rule: &'static str, id: NodeId) {
        self.fired.push((rule, id));
    }

    /// Record that a rule declined, once per rule.
    fn decline(&mut self, rule: &'static str) {
        if !self.declined.contains(&rule) {
            self.declined.push(rule);
        }
    }
}

impl fmt::Display for Explanation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "optimizer: {} pass(es)", self.passes)?;
        for (rule, id) in &self.fired {
            writeln!(formatter, "  fired {rule} on {id}")?;
        }
        for rule in &self.declined {
            writeln!(formatter, "  declined {rule}")?;
        }
        Ok(())
    }
}

/// Run every rule to a fixed point, discarding the report.
pub(super) fn run(plan: &mut Plan, root: NodeId, schema: Option<&Field>) -> NodeId {
    run_explained(plan, root, schema).0
}

/// Run every rule to a fixed point, reporting what fired.
pub(super) fn run_explained(
    plan: &mut Plan,
    root: NodeId,
    schema: Option<&Field>,
) -> (NodeId, Explanation) {
    let mut explanation = Explanation::default();
    if plan.reachable(root).len() < MIN_NODES {
        return (root, explanation);
    }
    let mut current = root;
    for pass in 1..=MAX_PASSES {
        explanation.passes = pass;
        let mut rewriter = Rewriter {
            schema,
            explanation: &mut explanation,
            done: HashMap::new(),
        };
        let next = rewriter.visit(plan, current);
        if next == current {
            return (current, explanation);
        }
        current = next;
    }
    // Reaching the cap means two rules are undoing each other. The plan is
    // still correct - every rule preserved semantics - so the answer is the
    // last one, and the report says how it got there.
    explanation.decline("fixed-point cap reached");
    (current, explanation)
}

/// One bottom-up rebuild pass.
struct Rewriter<'a> {
    schema: Option<&'a Field>,
    explanation: &'a mut Explanation,
    done: HashMap<NodeId, NodeId>,
}

impl Rewriter<'_> {
    /// Rebuild one node and everything below it.
    fn visit(&mut self, plan: &mut Plan, id: NodeId) -> NodeId {
        if let Some(done) = self.done.get(&id) {
            return *done;
        }
        let Some(node) = plan.get(id).cloned() else {
            return id;
        };
        let mut children = Vec::new();
        node.for_each_child(|child| children.push(child));
        let mut mapped = HashMap::new();
        for child in children {
            let rebuilt = self.visit(plan, child);
            mapped.insert(child, rebuilt);
        }
        let rebuilt = node.map_children(|child| mapped.get(&child).copied().unwrap_or(child));
        let new_id = self.rewrite(plan, rebuilt, id);
        self.done.insert(id, new_id);
        new_id
    }

    /// Apply every rule that fits one node, then intern the result.
    fn rewrite(&mut self, plan: &mut Plan, node: Node, was: NodeId) -> NodeId {
        let node = self.normalize(plan, node, was);
        let id = plan.insert(node);
        self.carry_type(plan, was, id);
        if let Some(folded) = self.fold_constant(plan, id) {
            self.explanation.fire("constant folding", id);
            let literal = plan.insert(Node::Literal(folded));
            self.carry_type(plan, id, literal);
            return literal;
        }
        id
    }

    /// Keep the datatype binding computed, so a rewrite never loses it.
    fn carry_type(&self, plan: &mut Plan, from: NodeId, to: NodeId) {
        if plan.data_type(to).is_some() {
            return;
        }
        if let Some(data_type) = plan.data_type(from).cloned() {
            plan.set_data_type(to, data_type);
        }
    }

    /// The shape rules: normalization, lowering, coalescing, and casts.
    #[allow(clippy::too_many_lines)]
    fn normalize(&mut self, plan: &mut Plan, node: Node, was: NodeId) -> Node {
        match node {
            Node::Not(child) => self.push_not(plan, child, was),
            Node::Between {
                child,
                low,
                high,
                negated,
            } => {
                // `BETWEEN` *is* two comparisons, nulls included, and two
                // comparisons are what range coalescing and statistics prune.
                self.explanation.fire("between to comparisons", was);
                let lower = plan.insert(Node::Compare {
                    op: if negated {
                        CompareOp::Lt
                    } else {
                        CompareOp::GtEq
                    },
                    left: child,
                    right: low,
                });
                let upper = plan.insert(Node::Compare {
                    op: if negated {
                        CompareOp::Gt
                    } else {
                        CompareOp::LtEq
                    },
                    left: child,
                    right: high,
                });
                plan.set_data_type(lower, DataType::Boolean);
                plan.set_data_type(upper, DataType::Boolean);
                if negated {
                    Node::Or(vec![lower, upper])
                } else {
                    Node::And(vec![lower, upper])
                }
            }
            Node::In {
                child,
                list,
                negated,
            } => self.narrow_in(plan, child, list, negated, was),
            Node::Like {
                child,
                pattern,
                escape,
                negated,
                case_insensitive,
            } => self.narrow_like(plan, child, pattern, escape, negated, case_insensitive, was),
            Node::Compare { op, left, right } => self.orient(plan, op, left, right, was),
            Node::Cast {
                child,
                data_type,
                safe,
            } => self.narrow_cast(plan, child, data_type, safe, was),
            Node::And(operands) => self.combine(plan, operands, true, was),
            Node::Or(operands) => self.combine(plan, operands, false, was),
            other => other,
        }
    }

    /// Push a negation toward the leaves, where it disappears.
    ///
    /// Every step is a three-valued identity: De Morgan holds in Kleene logic,
    /// `NOT (a = b)` is `a <> b` (both unknown on a null operand), and the two
    /// null tests are the only two-valued operators there are.
    fn push_not(&mut self, plan: &mut Plan, child: NodeId, was: NodeId) -> Node {
        let Some(inner) = plan.get(child).cloned() else {
            return Node::Not(child);
        };
        let rule = "not pushed to the leaves";
        match inner {
            Node::Not(grandchild) => {
                self.explanation.fire(rule, was);
                // A double negation is not the grandchild in general - but it
                // is here, because Kleene negation is its own inverse.
                plan.get(grandchild)
                    .cloned()
                    .unwrap_or(Node::Literal(Value::Null))
            }
            Node::And(operands) => {
                self.explanation.fire(rule, was);
                Node::Or(
                    operands
                        .into_iter()
                        .map(|id| self.negate(plan, id))
                        .collect(),
                )
            }
            Node::Or(operands) => {
                self.explanation.fire(rule, was);
                Node::And(
                    operands
                        .into_iter()
                        .map(|id| self.negate(plan, id))
                        .collect(),
                )
            }
            Node::Compare { op, left, right } => {
                self.explanation.fire(rule, was);
                Node::Compare {
                    op: op.negated(),
                    left,
                    right,
                }
            }
            Node::IsNull(inner) => {
                self.explanation.fire(rule, was);
                Node::IsNotNull(inner)
            }
            Node::IsNotNull(inner) => {
                self.explanation.fire(rule, was);
                Node::IsNull(inner)
            }
            Node::In {
                child,
                list,
                negated,
            } => {
                self.explanation.fire(rule, was);
                Node::In {
                    child,
                    list,
                    negated: !negated,
                }
            }
            Node::Like {
                child,
                pattern,
                escape,
                negated,
                case_insensitive,
            } => {
                self.explanation.fire(rule, was);
                Node::Like {
                    child,
                    pattern,
                    escape,
                    negated: !negated,
                    case_insensitive,
                }
            }
            _ => Node::Not(child),
        }
    }

    /// Wrap one node in a negation, interned.
    fn negate(&mut self, plan: &mut Plan, id: NodeId) -> NodeId {
        let negated = plan.insert(Node::Not(id));
        plan.set_data_type(negated, DataType::Boolean);
        negated
    }

    /// Narrow a set membership: sort it, deduplicate it, collapse the ends.
    fn narrow_in(
        &mut self,
        plan: &mut Plan,
        child: NodeId,
        list: Vec<NodeId>,
        negated: bool,
        was: NodeId,
    ) -> Node {
        let literals: Option<Vec<Value>> = list
            .iter()
            .map(|id| plan.get(*id).and_then(Node::as_literal).cloned())
            .collect();
        let Some(mut values) = literals else {
            return Node::In {
                child,
                list,
                negated,
            };
        };
        // A sorted, deduplicated list makes plan equality decidable and both
        // the statistics evaluator and the vectorized path faster.
        values.sort();
        values.dedup();
        if values.len() == 1 {
            self.explanation.fire("one-element IN to equality", was);
            let right = plan.insert(Node::Literal(values.swap_remove(0)));
            self.carry_type(plan, child, right);
            return Node::Compare {
                op: if negated {
                    CompareOp::NotEq
                } else {
                    CompareOp::Eq
                },
                left: child,
                right,
            };
        }
        let sorted: Vec<NodeId> = values
            .into_iter()
            .map(|value| {
                let id = plan.insert(Node::Literal(value));
                self.carry_type(plan, child, id);
                id
            })
            .collect();
        if sorted != list {
            self.explanation
                .fire("IN list sorted and deduplicated", was);
        }
        Node::In {
            child,
            list: sorted,
            negated,
        }
    }

    /// Fold a wildcard-free or prefix-only `LIKE` into something prunable.
    #[allow(clippy::too_many_arguments)]
    fn narrow_like(
        &mut self,
        plan: &mut Plan,
        child: NodeId,
        pattern: NodeId,
        escape: Option<char>,
        negated: bool,
        case_insensitive: bool,
        was: NodeId,
    ) -> Node {
        let held = Node::Like {
            child,
            pattern,
            escape,
            negated,
            case_insensitive,
        };
        if negated || case_insensitive {
            return held;
        }
        let Some(text) = plan
            .get(pattern)
            .and_then(Node::as_literal)
            .and_then(Value::as_str)
        else {
            return held;
        };
        let Some(literal) = literal_prefix(text, escape) else {
            return held;
        };
        match literal {
            // A pattern with no wildcard at all is an equality, which prunes
            // on an exact bound rather than only on a range.
            Prefix::Exact(exact) => {
                self.explanation.fire("wildcard-free LIKE to equality", was);
                let right = plan.insert(Node::Literal(Value::String(exact.into())));
                self.carry_type(plan, child, right);
                Node::Compare {
                    op: CompareOp::Eq,
                    left: child,
                    right,
                }
            }
            // A trailing `%` and nothing else is the one text predicate a
            // statistics range can answer, which is the whole point of the
            // dedicated node.
            Prefix::Leading(prefix) => {
                self.explanation.fire("prefix LIKE to StartsWith", was);
                Node::StartsWith {
                    child,
                    prefix: prefix.into(),
                }
            }
        }
    }

    /// Orient a comparison as `column op literal`, and move a cast off it.
    fn orient(
        &mut self,
        plan: &mut Plan,
        op: CompareOp,
        left: NodeId,
        right: NodeId,
        was: NodeId,
    ) -> Node {
        let (op, left, right) = if plan.get(left).and_then(Node::as_literal).is_some()
            && plan.get(right).and_then(Node::as_literal).is_none()
        {
            self.explanation
                .fire("comparison oriented column-first", was);
            (op.flipped(), right, left)
        } else {
            (op, left, right)
        };
        // The highest-value rule in the set: a cast wrapping a column destroys
        // statistics and row-group pruning outright, while the same comparison
        // against a converted literal prunes perfectly.
        if let Some((column, literal)) = self.move_cast_to_literal(plan, left, right) {
            self.explanation
                .fire("cast moved from column to literal", was);
            return Node::Compare {
                op,
                left: column,
                right: literal,
            };
        }
        Node::Compare { op, left, right }
    }

    /// Move a cast off the column side onto the literal, when that is exact.
    ///
    /// The proof is per pair and per value: the conversion must be a widening
    /// one - order-preserving and injective - and the literal must survive a
    /// round trip through it unchanged. Anything unproven declines, which is
    /// why a literal that does not fit the column's own width keeps its cast
    /// rather than silently changing what the comparison asks.
    fn move_cast_to_literal(
        &mut self,
        plan: &mut Plan,
        left: NodeId,
        right: NodeId,
    ) -> Option<(NodeId, NodeId)> {
        let Node::Cast {
            child,
            data_type: wide,
            ..
        } = plan.get(left)?.clone()
        else {
            return None;
        };
        let literal = plan.get(right)?.as_literal()?.clone();
        let narrow = plan.data_type(child)?.clone();
        if !is_widening(&narrow, &wide) {
            self.explanation
                .decline("cast moved from column to literal");
            return None;
        }
        let narrowed = coerce_value(&literal, &narrow)?;
        // The round trip is the proof: if converting back does not reproduce
        // exactly what was written, the rewrite would change the question.
        if coerce_value(&narrowed, &wide)? != literal {
            self.explanation
                .decline("cast moved from column to literal");
            return None;
        }
        let folded = plan.insert(Node::Literal(narrowed));
        plan.set_data_type(folded, narrow);
        Some((child, folded))
    }

    /// Drop a cast that changes nothing, and collapse a cast of a cast.
    fn narrow_cast(
        &mut self,
        plan: &mut Plan,
        child: NodeId,
        data_type: DataType,
        safe: bool,
        was: NodeId,
    ) -> Node {
        if plan.data_type(child) == Some(&data_type) {
            self.explanation.fire("cast to the type already held", was);
            return plan
                .get(child)
                .cloned()
                .unwrap_or(Node::Literal(Value::Null));
        }
        if let Some(Node::Cast {
            child: inner,
            data_type: middle,
            safe: inner_safe,
        }) = plan.get(child).cloned()
        {
            let held = plan.data_type(inner).cloned();
            // The inner cast is droppable exactly when it could not have lost
            // anything, so the outer one sees the same value either way.
            if held.is_some_and(|held| is_widening(&held, &middle)) {
                self.explanation.fire("cast of cast collapsed", was);
                return Node::Cast {
                    child: inner,
                    data_type,
                    safe: safe || inner_safe,
                };
            }
            self.explanation.decline("cast of cast collapsed");
        }
        Node::Cast {
            child,
            data_type,
            safe,
        }
    }

    /// Flatten, absorb, deduplicate, coalesce, and order one connective.
    #[allow(clippy::too_many_lines)]
    fn combine(&mut self, plan: &mut Plan, operands: Vec<NodeId>, all: bool, was: NodeId) -> Node {
        let mut flat: Vec<NodeId> = Vec::with_capacity(operands.len());
        for operand in operands {
            match plan.get(operand) {
                Some(Node::And(inner)) if all => flat.extend(inner.iter().copied()),
                Some(Node::Or(inner)) if !all => flat.extend(inner.iter().copied()),
                _ => flat.push(operand),
            }
        }
        // `TRUE` absorbs through a conjunction and `FALSE` through a
        // disjunction; the other constant settles the whole node.
        let absorbing = Value::Bool(!all);
        let neutral = Value::Bool(all);
        if flat
            .iter()
            .any(|id| plan.get(*id).and_then(Node::as_literal) == Some(&absorbing))
        {
            self.explanation
                .fire("constant absorbed the connective", was);
            return Node::Literal(absorbing);
        }
        let before = flat.len();
        flat.retain(|id| plan.get(*id).and_then(Node::as_literal) != Some(&neutral));
        if flat.len() != before {
            self.explanation.fire("neutral constant dropped", was);
        }
        // Interning made structural identity into id identity, so removing a
        // duplicate conjunct is a set operation rather than a deep compare.
        let before = flat.len();
        let mut seen = std::collections::HashSet::new();
        flat.retain(|id| seen.insert(*id));
        if flat.len() != before {
            self.explanation.fire("duplicate operand dropped", was);
        }
        if self.absorb(plan, &mut flat, all) {
            self.explanation.fire("absorption", was);
        }
        if all {
            match self.coalesce_ranges(plan, &mut flat) {
                Outcome::Changed => {
                    self.explanation
                        .fire("overlapping comparisons coalesced", was);
                }
                Outcome::Contradiction => {
                    self.explanation
                        .fire("contradictory range folded to FALSE", was);
                    return Node::Literal(Value::Bool(false));
                }
                Outcome::Unchanged => {}
            }
            match self.intersect_sets(plan, &mut flat) {
                Outcome::Changed => {
                    self.explanation.fire("set memberships intersected", was);
                }
                Outcome::Contradiction => {
                    self.explanation
                        .fire("set memberships intersected to FALSE", was);
                    return Node::Literal(Value::Bool(false));
                }
                Outcome::Unchanged => {}
            }
        } else if self.equalities_to_set(plan, &mut flat) {
            self.explanation.fire("OR of equalities to an IN list", was);
        }
        if !all {
            if let Some(distributed) = self.distribute(plan, &flat) {
                self.explanation.fire("disjunction distributed to CNF", was);
                return distributed;
            }
        }
        match flat.len() {
            0 => Node::Literal(neutral),
            1 => plan
                .get(flat[0])
                .cloned()
                .unwrap_or(Node::Literal(Value::Null)),
            _ => {
                if all {
                    // Evaluation is side-effect free and three-valued `AND` is
                    // commutative, so ordering cheapest-first is free and lets
                    // a short-circuit skip the expensive operand.
                    flat.sort_by_key(|id| (cost(plan, *id), *id));
                    Node::And(flat)
                } else {
                    Node::Or(flat)
                }
            }
        }
    }

    /// `p AND (p OR q) -> p` and `p OR (p AND q) -> p`.
    fn absorb(&mut self, plan: &Plan, operands: &mut Vec<NodeId>, all: bool) -> bool {
        let held: std::collections::HashSet<NodeId> = operands.iter().copied().collect();
        let before = operands.len();
        operands.retain(|id| {
            let opposite = match plan.get(*id) {
                Some(Node::Or(inner)) if all => inner,
                Some(Node::And(inner)) if !all => inner,
                _ => return true,
            };
            !opposite.iter().any(|inner| held.contains(inner))
        });
        operands.len() != before
    }

    /// Keep only the tightest bound per column and per direction.
    fn coalesce_ranges(&mut self, plan: &mut Plan, operands: &mut Vec<NodeId>) -> Outcome {
        let mut tightest: HashMap<(NodeId, bool), (usize, NodeId, Value, CompareOp)> =
            HashMap::new();
        let mut order: Vec<Option<NodeId>> = operands.iter().copied().map(Some).collect();
        let mut changed = false;
        for (position, operand) in operands.iter().enumerate() {
            let Some(Node::Compare { op, left, right }) = plan.get(*operand) else {
                continue;
            };
            let upward = match op {
                CompareOp::Gt | CompareOp::GtEq => true,
                CompareOp::Lt | CompareOp::LtEq => false,
                _ => continue,
            };
            if plan.get(*left).and_then(Node::as_column).is_none() {
                continue;
            }
            let Some(value) = plan.get(*right).and_then(Node::as_literal).cloned() else {
                continue;
            };
            let key = (*left, upward);
            match tightest.get(&key) {
                Some((held_at, _, held_value, held_op)) => {
                    // The tighter bound is the larger one going up and the
                    // smaller one going down; an equal value keeps the strict
                    // operator, which is the tighter of the two.
                    let tighter = match value.cmp(held_value) {
                        std::cmp::Ordering::Equal => strictness(*op) > strictness(*held_op),
                        std::cmp::Ordering::Greater => upward,
                        std::cmp::Ordering::Less => !upward,
                    };
                    if tighter {
                        order[*held_at] = None;
                        tightest.insert(key, (position, *operand, value, *op));
                    } else {
                        order[position] = None;
                    }
                    changed = true;
                }
                None => {
                    tightest.insert(key, (position, *operand, value, *op));
                }
            }
        }
        if changed {
            *operands = order.into_iter().flatten().collect();
        }
        // A contradictory pair is deliberately *not* folded to FALSE: it is
        // unknown when the column is null, and unknown is not false outside a
        // filter. Only a column the schema proves non-nullable may fold.
        if self.fold_contradiction(plan, operands) {
            return Outcome::Contradiction;
        }
        if changed {
            Outcome::Changed
        } else {
            Outcome::Unchanged
        }
    }

    /// Fold a provably empty range, but only where no null can occur.
    fn fold_contradiction(&mut self, plan: &Plan, operands: &[NodeId]) -> bool {
        let mut bounds: HashMap<NodeId, Bounds> = HashMap::new();
        for operand in operands {
            let Some(Node::Compare { op, left, right }) = plan.get(*operand) else {
                continue;
            };
            let Some(value) = plan.get(*right).and_then(Node::as_literal).cloned() else {
                continue;
            };
            let entry = bounds.entry(*left).or_default();
            match op {
                CompareOp::Gt | CompareOp::GtEq => entry.0 = Some((value, *op)),
                CompareOp::Lt | CompareOp::LtEq => entry.1 = Some((value, *op)),
                _ => {}
            }
        }
        for (column, (lower, upper)) in bounds {
            let (Some((lower, lower_op)), Some((upper, upper_op))) = (lower, upper) else {
                continue;
            };
            let empty = match lower.cmp(&upper) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Equal => strictness(lower_op) > 0 || strictness(upper_op) > 0,
                std::cmp::Ordering::Less => false,
            };
            if !empty {
                continue;
            }
            if !self.is_non_nullable(plan, column) {
                self.explanation
                    .decline("contradictory range folded to FALSE");
                continue;
            }
            return true;
        }
        false
    }

    /// Return whether the schema proves a node can never be null.
    fn is_non_nullable(&self, plan: &Plan, id: NodeId) -> bool {
        let Some(schema) = self.schema else {
            return false;
        };
        let _ = schema;
        plan.get(id)
            .and_then(Node::as_column)
            .and_then(|column| column.bound())
            .is_some_and(|bound| !bound.field().is_nullable())
    }

    /// Intersect two memberships on one column into one.
    fn intersect_sets(&mut self, plan: &mut Plan, operands: &mut Vec<NodeId>) -> Outcome {
        // The memberships are read out of the plan first, so nothing borrows
        // it while the merged node is being inserted.
        let mut found: Vec<(usize, NodeId, Vec<Value>)> = Vec::new();
        for (position, operand) in operands.iter().enumerate() {
            let Some(Node::In {
                child,
                list,
                negated: false,
            }) = plan.get(*operand)
            else {
                continue;
            };
            let Some(values): Option<Vec<Value>> = list
                .iter()
                .map(|id| plan.get(*id).and_then(Node::as_literal).cloned())
                .collect()
            else {
                continue;
            };
            found.push((position, *child, values));
        }
        let mut order: Vec<Option<NodeId>> = operands.iter().copied().map(Some).collect();
        let mut held: HashMap<NodeId, (usize, Vec<Value>)> = HashMap::new();
        let mut changed = false;
        for (position, child, values) in found {
            let Some((held_at, previous)) = held.get(&child).cloned() else {
                held.insert(child, (position, values));
                continue;
            };
            let intersection: Vec<Value> = previous
                .into_iter()
                .filter(|value| values.contains(value))
                .collect();
            if intersection.is_empty() {
                // An empty intersection is `FALSE` only where no null can
                // occur, for the same reason a contradictory range is.
                if !self.is_non_nullable(plan, child) {
                    self.explanation.decline("set memberships intersected");
                    continue;
                }
                return Outcome::Contradiction;
            }
            let list: Vec<NodeId> = intersection
                .iter()
                .map(|value| plan.insert(Node::Literal(value.clone())))
                .collect();
            let merged = plan.insert(Node::In {
                child,
                list,
                negated: false,
            });
            plan.set_data_type(merged, DataType::Boolean);
            order[held_at] = None;
            order[position] = Some(merged);
            held.insert(child, (position, intersection));
            changed = true;
        }
        if changed {
            *operands = order.into_iter().flatten().collect();
        }
        if changed {
            Outcome::Changed
        } else {
            Outcome::Unchanged
        }
    }

    /// `a = 1 OR a = 2 OR a = 3 -> a IN (1, 2, 3)`.
    fn equalities_to_set(&mut self, plan: &mut Plan, operands: &mut Vec<NodeId>) -> bool {
        let mut grouped: HashMap<NodeId, Vec<(usize, Value)>> = HashMap::new();
        for (position, operand) in operands.iter().enumerate() {
            let Some(Node::Compare {
                op: CompareOp::Eq,
                left,
                right,
            }) = plan.get(*operand)
            else {
                continue;
            };
            if plan.get(*left).and_then(Node::as_column).is_none() {
                continue;
            }
            let Some(value) = plan.get(*right).and_then(Node::as_literal).cloned() else {
                continue;
            };
            grouped.entry(*left).or_default().push((position, value));
        }
        let mut order: Vec<Option<NodeId>> = operands.iter().copied().map(Some).collect();
        let mut changed = false;
        // Grouping is by node id, and a `HashMap` iterates in no fixed order,
        // so the columns are visited in plan order to keep the result stable.
        let mut columns: Vec<NodeId> = grouped.keys().copied().collect();
        columns.sort_unstable();
        for column in columns {
            let Some(members) = grouped.get(&column) else {
                continue;
            };
            if members.len() < 2 {
                continue;
            }
            let mut values: Vec<Value> = members.iter().map(|(_, value)| value.clone()).collect();
            values.sort();
            values.dedup();
            let list: Vec<NodeId> = values
                .into_iter()
                .map(|value| plan.insert(Node::Literal(value)))
                .collect();
            let merged = plan.insert(Node::In {
                child: column,
                list,
                negated: false,
            });
            plan.set_data_type(merged, DataType::Boolean);
            for (position, _) in members {
                order[*position] = None;
            }
            order[members[0].0] = Some(merged);
            changed = true;
        }
        if changed {
            *operands = order.into_iter().flatten().collect();
        }
        changed
    }

    /// Distribute a disjunction over a conjunction, under a hard guard.
    fn distribute(&mut self, plan: &mut Plan, operands: &[NodeId]) -> Option<Node> {
        let groups: Vec<Vec<NodeId>> = operands
            .iter()
            .map(|id| match plan.get(*id) {
                Some(Node::And(inner)) => inner.clone(),
                _ => vec![*id],
            })
            .collect();
        if groups.iter().all(|group| group.len() == 1) {
            return None;
        }
        let product = groups
            .iter()
            .try_fold(1_usize, |held, group| held.checked_mul(group.len()))?;
        if product > CNF_PRODUCT_GUARD {
            self.explanation.decline("disjunction distributed to CNF");
            return None;
        }
        let mut clauses: Vec<Vec<NodeId>> = vec![Vec::new()];
        for group in &groups {
            let mut next = Vec::with_capacity(clauses.len() * group.len());
            for clause in &clauses {
                for member in group {
                    let mut extended = clause.clone();
                    extended.push(*member);
                    next.push(extended);
                }
            }
            clauses = next;
        }
        let conjuncts: Vec<NodeId> = clauses
            .into_iter()
            .map(|mut clause| {
                clause.sort_unstable();
                clause.dedup();
                if clause.len() == 1 {
                    clause[0]
                } else {
                    let id = plan.insert(Node::Or(clause));
                    plan.set_data_type(id, DataType::Boolean);
                    id
                }
            })
            .collect();
        let mut conjuncts = conjuncts;
        conjuncts.sort_unstable();
        conjuncts.dedup();
        Some(Node::And(conjuncts))
    }

    /// Fold a node every one of whose operands is already constant.
    ///
    /// Folding runs over the whole graph rather than only over literal-only
    /// leaves, which is what lets `1 + 2 * 3` become `7` in one pass. A node
    /// that reads a column never folds, which is what keeps `a = a` unknown.
    fn fold_constant(&self, plan: &Plan, id: NodeId) -> Option<Value> {
        let node = plan.get(id)?;
        if matches!(
            node,
            Node::Literal(_) | Node::Column(_) | Node::Alias { .. }
        ) {
            return None;
        }
        let mut constant = true;
        node.for_each_child(|child| {
            if !matches!(plan.get(child), Some(Node::Literal(_))) {
                constant = false;
            }
        });
        if !constant {
            return None;
        }
        // The row is irrelevant to a node that reads no column, so the empty
        // one is what proves the node really is constant.
        super::eval::evaluate(plan, id, &Value::Null).ok()
    }
}

/// What one coalescing rule did.
///
/// A contradiction is its own answer rather than an emptied operand list,
/// because an empty conjunction is `TRUE` - conflating the two would turn a
/// predicate that matches nothing into one that matches everything.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Outcome {
    /// Nothing fired.
    Unchanged,
    /// The operand list was rewritten.
    Changed,
    /// The operands cannot all hold, and no null can make that unknown.
    Contradiction,
}

/// The lower and upper bound one column carries in one conjunction.
type Bounds = (Option<(Value, CompareOp)>, Option<(Value, CompareOp)>);

/// How strict a bound operator is, for picking the tighter of two.
const fn strictness(op: CompareOp) -> u8 {
    match op {
        CompareOp::Gt | CompareOp::Lt => 1,
        _ => 0,
    }
}

/// Return whether every value of `from` survives a conversion to `to`.
///
/// Only pairs proven here move a cast off a column, so the list is short on
/// purpose: an unproven pair declines and the cast stays where it is.
#[must_use]
pub(super) fn is_widening(from: &DataType, to: &DataType) -> bool {
    use DataType as D;

    if from == to {
        return true;
    }
    match (family(from), family(to)) {
        (super::bound::Family::Integer, super::bound::Family::Integer) => integer_width(from)
            .is_some_and(|narrow| {
                integer_width(to)
                    .is_some_and(|wide| narrow.0 <= wide.0 && (narrow.1 == wide.1 || wide.1))
            }),
        (super::bound::Family::Decimal, super::bound::Family::Decimal) => {
            match (decimal_parts(from), decimal_parts(to)) {
                // A wider precision at the same scale keeps every digit; a
                // different scale restates and can drop one.
                (Some((narrow_p, narrow_s)), Some((wide_p, wide_s))) => {
                    narrow_s == wide_s && narrow_p <= wide_p
                }
                _ => false,
            }
        }
        (super::bound::Family::Integer, super::bound::Family::Decimal) => {
            decimal_parts(to).is_some_and(|(_, scale)| scale >= 0)
        }
        (super::bound::Family::Date, super::bound::Family::NaiveTimestamp) => true,
        (super::bound::Family::NaiveTimestamp, super::bound::Family::NaiveTimestamp)
        | (super::bound::Family::Timestamp, super::bound::Family::Timestamp) => {
            matches!((from, to), (D::Timestamp(narrow, _), D::Timestamp(wide, _))
                if unit_rank(*narrow) <= unit_rank(*wide))
        }
        _ => false,
    }
}

/// An integer datatype's width in bits and whether it is signed.
const fn integer_width(data_type: &DataType) -> Option<(u8, bool)> {
    use DataType as D;
    Some(match data_type {
        D::Int8 => (8, true),
        D::Int16 => (16, true),
        D::Int32 => (32, true),
        D::Int64 => (64, true),
        D::UInt8 => (8, false),
        D::UInt16 => (16, false),
        D::UInt32 => (32, false),
        D::UInt64 => (64, false),
        _ => return None,
    })
}

/// A decimal datatype's precision and scale.
const fn decimal_parts(data_type: &DataType) -> Option<(u8, i8)> {
    use DataType as D;
    match data_type {
        D::Decimal32 { precision, scale }
        | D::Decimal64 { precision, scale }
        | D::Decimal128 { precision, scale }
        | D::Decimal256 { precision, scale } => Some((*precision, *scale)),
        _ => None,
    }
}

/// How fine a temporal resolution is, so a widening one is comparable.
const fn unit_rank(unit: crate::TimeUnit) -> u8 {
    match unit {
        crate::TimeUnit::Second => 0,
        crate::TimeUnit::Millisecond => 1,
        crate::TimeUnit::Microsecond => 2,
        crate::TimeUnit::Nanosecond => 3,
        _ => 4,
    }
}

/// Roughly what one operand costs to evaluate, for cheapest-first ordering.
///
/// The numbers are ordinal rather than measured: what matters is that a null
/// test comes before a comparison, a comparison before a set, and anything
/// that decodes text or computes arithmetic comes last.
fn cost(plan: &Plan, id: NodeId) -> u32 {
    let Some(node) = plan.get(id) else { return 0 };
    let here = match node {
        Node::Literal(_) | Node::Column(_) => 0,
        Node::IsNull(_) | Node::IsNotNull(_) => 1,
        Node::Compare { .. } => 2,
        Node::In { .. } => 3,
        Node::StartsWith { .. } | Node::Between { .. } => 4,
        Node::And(_) | Node::Or(_) | Node::Not(_) | Node::Alias { .. } => 1,
        Node::Like { .. } => 8,
        Node::Arithmetic { .. } | Node::Neg(_) => 6,
        Node::Cast { .. } => 10,
        Node::Function { .. } | Node::Case { .. } => 12,
    };
    let mut below = 0;
    node.for_each_child(|child| below = below.max(cost(plan, child)));
    here + below
}

/// What a `LIKE` pattern reduces to when it holds at most a trailing `%`.
enum Prefix {
    /// No wildcard at all, so the pattern is an equality.
    Exact(String),
    /// Exactly one trailing `%`, so the pattern is a prefix test.
    Leading(String),
}

/// Read a pattern that is literal except for at most one trailing `%`.
fn literal_prefix(pattern: &str, escape: Option<char>) -> Option<Prefix> {
    let mut literal = String::with_capacity(pattern.len());
    let mut escaped = false;
    let mut characters = pattern.chars().peekable();
    while let Some(character) = characters.next() {
        if escaped {
            literal.push(character);
            escaped = false;
            continue;
        }
        if Some(character) == escape {
            escaped = true;
            continue;
        }
        match character {
            '%' if characters.peek().is_none() => return Some(Prefix::Leading(literal)),
            '%' | '_' => return None,
            other => literal.push(other),
        }
    }
    if escaped {
        // A trailing escape character escapes nothing, so the pattern is not
        // one this rule can read; the general matcher still answers it.
        return None;
    }
    Some(Prefix::Exact(literal))
}

/// The arithmetic operator a node applies, for the cost table's readability.
#[allow(dead_code)]
const fn arithmetic_of(node: &Node) -> Option<ArithOp> {
    match node {
        Node::Arithmetic { op, .. } => Some(*op),
        _ => None,
    }
}
