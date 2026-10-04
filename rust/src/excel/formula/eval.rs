//! One explicit-stack evaluator over the typed formula arena.

use std::collections::{HashMap, HashSet};

use super::aggregate::{Accumulator, LogicalAccumulator};
use super::criteria::Criterion;
use super::functions::Function;
use super::lexer::{self, Kind};
use super::shape;
use super::criteria::Wildcard;
use super::number;
use super::parser::{BinaryOp, EvaluationPolicy, Expr, Node, ReferenceUse, Selection, UnaryOp};
use super::reference::{Coord, Reference, SheetSpec, Target};
use super::shape::Held;
use super::value::{ArrayId, NameId, Operand, Outcome, ReferenceId, Unevaluated};
use crate::excel::cell::{CellRange, CellRef, DateSystem, ExcelError, MAX_COLUMNS, MAX_ROWS};
use crate::excel::entry;
use crate::{Arithmetic, Scalar};

/// Values required by a range consumer. Aggregates can avoid rendering
/// referenced text, but errors and unresolved dependencies always remain visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RangeRead {
    Values,
    /// All logical positions, with physically absent runs emitted once with
    /// their multiplicity. Stored empty strings remain individual values.
    ValuesWithBlanks,
    /// Dense compact runs classified without rendering present text.
    BlankPresence,
    /// Aligned numeric source positions, with text/Boolean/blank skipped
    /// before typed text rendering; absent runs retain their multiplicity.
    NumericDense,
    /// Resumable ordered one-dimensional read with a reached-prefix dependency.
    Lookup,
    Numbers,
    Logical,
    /// Presence only: the reader must not allocate or render a cell's text.
    Presence,
    /// A-family aggregates count referenced text as zero and Booleans as
    /// zero/one without rendering or cloning their source values.
    AggregateA,
    /// Ignore nested SUBTOTAL formula cells; 100-series codes also omit
    /// manually hidden rows. The workbook's same sparse visitor applies it.
    Subtotal { exclude_hidden: bool },
}

#[derive(Clone, Copy)]
enum RankArgument {
    K(f64),
    Rank { target: f64, ascending: bool },
}

impl RangeRead {
    pub(crate) const fn includes_values(self) -> bool {
        matches!(self, Self::Values | Self::ValuesWithBlanks | Self::Lookup | Self::Subtotal { .. })
    }
}

/// A lookup reader suspended before its next unresolved source cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RangeProgress { Complete, Paused(u64) }

/// One resolved reference or an existing named expression arena.
pub(crate) enum ReferenceResult {
    Value(Outcome),
    Name(NameId),
}

/// A formula-scoped workbook reader. Reference IDs name descriptors owned by
/// this context, never a borrow stored in the reusable evaluator buffers.
/// The owner resets its arena only after evaluate returns (also on refusal).
pub(crate) trait Context<'w> {
    fn system(&self) -> DateSystem;
    fn text_compatibility(&self) -> super::text::Compatibility;
    fn clock_serial(&mut self, today: bool) -> crate::Result<f64>;
    fn random_u64(&mut self) -> u64;
    fn reference(
        &mut self,
        name: Option<NameId>,
        reference: &super::reference::Reference,
    ) -> crate::Result<ReferenceResult>;
    fn expression(&self, name: Option<NameId>) -> Result<&'w Expr, Held>;
    fn host(&self) -> CellRef;
    /// Single-sheet geometry, without reading any cell value. None is 3-D.
    fn reference_geometry(&self, id: ReferenceId) -> Option<CellRange>;
    /// Reference algebra preserves source areas rather than reading values.
    fn reference_combine(&mut self, op: BinaryOp, left: ReferenceId, right: ReferenceId) -> Outcome;
    fn reference_is_union(&self, id: ReferenceId) -> bool;
    /// Select one zero-based area for INDEX's reference form.
    fn reference_nth(&mut self, id: ReferenceId, index: usize) -> Outcome;
    /// Narrow geometry through the same legacy intersection owner as scalar
    /// reads, retaining a reference handle instead of reading its value.
    fn intersection(&mut self, id: ReferenceId) -> Outcome;
    /// Retain the source sheet identity with newly resolved geometry. Value
    /// dependencies are admitted only when the caller consumes this handle.
    fn reference_range(&mut self, id: ReferenceId, range: CellRange) -> Outcome;
    /// Establish dependency readiness before consuming a retained handle.
    /// A suspended caller retains this evaluator and its descriptor arena.
    fn ready(&mut self, _id: ReferenceId, _usage: ReferenceUse) -> crate::Result<bool> { Ok(true) }
    fn ready_subtotal(&mut self, _id: ReferenceId, _exclude_hidden: bool) -> crate::Result<bool> { Ok(true) }
    fn scalar(&mut self, value: Operand) -> crate::Result<Outcome>;
    /// Visit outcomes in sheet/row/column order. ValuesWithBlanks and Lookup emit
    /// absent positions as compact blank runs; every other outcome has count1.
    /// Numbers omits known text/Boolean/blank before rendering, while Logical
    /// retains Booleans. All modes retain errors and held dependencies before
    /// inspecting their old cache type. Consumers must bound output before
    /// expanding a multiplicity (one worksheet can contain 2^34 positions).
    fn visit_range(
        &mut self,
        id: ReferenceId,
        read: RangeRead,
        from: u64,
        visit: impl FnMut(Outcome, u64) -> std::ops::ControlFlow<()>,
    ) -> crate::Result<RangeProgress>;
}

#[derive(Debug)]
enum Frame {
    Node {
        name: Option<NameId>,
        id: usize,
        base: usize,
        usage: ReferenceUse,
        ready: bool,
    },
    Return {
        name: NameId,
        root: usize,
        base: usize,
        destination: usize,
    },
    Selection {
        name: Option<NameId>,
        id: usize,
        base: usize,
        usage: ReferenceUse,
    },
    Branch {
        base: usize,
        child: usize,
        destination: usize,
        scalar: bool,
        usage: ReferenceUse,
    },
    ArraySelection {
        name: Option<NameId>, id: usize, base: usize,
        selector: ArraySelector, test: ArrayId, choices: std::ops::Range<usize>, next: usize, waiting: bool,
    },
    Lookup {
        name: Option<NameId>,
        id: usize,
        base: usize,
        usage: ReferenceUse,
    },
}

/// Pending is control flow, never an operand or a published cell cache.
pub(crate) enum Evaluation {
    Complete(Outcome),
    Paused,
}

/// Source origin changes aggregate coercion; arrays are not worksheet cells.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ArgumentSource { Direct, Reference, Array }

enum AggregateMode { Plain, Ranked(RankArgument), Filtered(RangeRead) }

enum SelectionStep {
    Complete(Outcome),
    Branch { child: usize, scalar: bool },
}

#[derive(Clone, Copy, Debug)]
enum SearchMode {
    FirstEqual,
    LastEqual,
    LastAtMost,
    LastAtLeast,
    NearestAtMost,
    NearestAtLeast,
}

enum SearchResult {
    Found(u64),
    Missing,
    Stopped(Outcome),
}

enum SearchProgress {
    Complete(SearchResult),
    Paused,
}

#[derive(Debug)]
enum LookupDelivery {
    Match,
    Table {
        source: ReferenceId,
        start: CellRef,
        index: u32,
        vertical: bool,
    },
    Vector {
        result: ReferenceId,
        start: CellRef,
        vertical: bool,
    },
    Xlookup {
        result: ReferenceId,
        start: CellRef,
        vertical: bool,
        fallback: Option<Option<usize>>,
    },
}

#[derive(Debug)]
struct LookupState {
    key: Operand,
    source: ReferenceId,
    mode: SearchMode,
    pattern: Option<Wildcard>,
    next: u64,
    found: Option<u64>,
    best_number: Option<f64>,
    delivery: LookupDelivery,
}

enum LookupStart<T> {
    Complete(T),
    Scan(LookupState),
}

impl LookupState {
    fn new(
        key: Operand,
        source: ReferenceId,
        mode: SearchMode,
        wildcard: bool,
        delivery: LookupDelivery,
    ) -> Self {
        let pattern = match &key {
            Operand::Text(text)
                if wildcard
                    && text
                        .as_str()
                        .bytes()
                        .any(|b| matches!(b, b'*' | b'?' | b'~')) =>
            {
                Some(Wildcard::new(text.as_str(), false, true))
            }
            _ => None,
        };
        Self {
            key,
            source,
            mode,
            pattern,
            next: 0,
            found: None,
            best_number: None,
            delivery,
        }
    }
}

/// Scalar kernels shared by ordinary evaluation and requested array elements.
#[derive(Clone, Copy, Debug)]
enum ElementOp {
    Unary(UnaryOp), Percent, Binary { op: BinaryOp, root: bool }, Absolute, Round,
}

impl ElementOp {
    fn apply(self, left: Outcome, right: Option<Outcome>, system: DateSystem) -> Outcome {
        match self {
            Self::Unary(op) => Evaluator::unary(op, left, false, system),
            Self::Percent => Evaluator::percent(left, system),
            Self::Binary { op, root } => Evaluator::binary(op, left, right.expect("binary operation owns two operands"), root, system),
            Self::Absolute => Evaluator::absolute(left, system),
            Self::Round => Evaluator::round(left, right.expect("ROUND owns two operands"), system),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ArraySelector { If, Choose }

impl ArraySelector {
    /// One coercion owner for scalar and array selector positions.
    fn index(self, value: Outcome, count: usize, system: DateSystem) -> Result<usize, Outcome> {
        let value = value.operand().map_err(Outcome::Uncomputed)?;
        match self {
            Self::If => match value.logical() {
                Some(Ok(value)) => Ok(usize::from(!value)),
                Some(Err(error)) => Err(Outcome::Computed(Operand::Error(error))),
                None => Err(Outcome::Uncomputed(Unevaluated::Function(Function::If))),
            },
            Self::Choose => match value.number(system) {
                Some(Ok(index)) => {
                    let index = Scalar::from(index).checked_trunc()
                        .expect("CHOOSE binds a Float64 index").as_f64()
                        .expect("the shared truncation preserves Float64");
                    if !index.is_finite() || index < 1.0 || index > count as f64 {
                        Err(Outcome::Computed(Operand::Error(ExcelError::Value)))
                    } else { Ok(index as usize - 1) }
                }
                Some(Err(error)) => Err(Outcome::Computed(Operand::Error(error))),
                None => Err(Outcome::Uncomputed(Unevaluated::Function(Function::Choose))),
            },
        }
    }
}

#[derive(Debug)]
struct ArrayChoice { child: Option<usize>, selected: bool, value: Outcome }

#[derive(Debug)]
enum ArrayOperation {
    Element { op: ElementOp, left: Outcome, right: Option<Outcome> },
    Selection { selector: ArraySelector, test: ArrayId, choices: std::ops::Range<usize> },
}

#[derive(Debug)]
struct ArrayPlan {
    operation: ArrayOperation, rows: usize, columns: usize,
    // One last coordinate per plan bounds repeated named-array DAG work.
    cached: Option<(usize, usize, Outcome)>,
}

#[derive(Debug)]
enum ElementFrame {
    Read { value: Outcome, row: usize, column: usize },
    Apply { index: usize, row: usize, column: usize },
    Select { index: usize, row: usize, column: usize },
    Cache { index: usize, row: usize, column: usize },
}

#[derive(Debug)]
struct NamedResult {
    outcome: Outcome,
    volatile: bool,
}

/// Scratch retains peak active arena slots and one result per reached name,
/// not AST copies. Named siblings reuse value slots after returning. Active
/// identities prevent alias loops without recursion or a depth-linear scan.
#[derive(Debug, Default)]
pub(crate) struct Evaluator {
    stack: Vec<Frame>,
    values: Vec<Option<Outcome>>,
    // Plans retain scalar operands once; element scratch is depth-bounded.
    // Capacity survives warm passes, while every formula clears its handles.
    arrays: Vec<ArrayPlan>,
    array_choices: Vec<ArrayChoice>,
    element_stack: Vec<ElementFrame>,
    element_values: Vec<Outcome>,
    lookup: Option<LookupState>,
    names: HashSet<NameId>,
    // Deterministic values and held outcomes retain range origin. Descriptors
    // remain valid until this map clears at the formula evaluation boundary.
    name_results: HashMap<NameId, NamedResult>,
    name_volatility: Vec<bool>,
    // One compiled pattern plus peak transition capacity, shared across calls.
    search: super::criteria::Search,
    formatter: super::text::Formatter,
    volatile: bool,
    #[cfg(feature = "internals")]
    visited: usize,
    #[cfg(feature = "internals")]
    element_steps: usize,
}

impl Evaluator {
    #[cfg(feature = "internals")]
    pub(crate) const fn visited(&self) -> usize { self.visited }

    #[cfg(feature = "internals")]
    pub(crate) const fn array_steps(&self) -> usize { self.element_steps }

    #[cfg(feature = "internals")]
    pub(crate) fn evaluate<'w>(
        &mut self,
        expression: &'w Expr,
        context: &mut impl Context<'w>,
    ) -> crate::Result<Outcome> {
        self.begin(expression);
        let result = match self.resume(expression, context) {
            Ok(Evaluation::Complete(value)) => Ok(value),
            Ok(Evaluation::Paused) => Err(crate::Error::Conflict {
                expected: "a context whose formula dependencies are ready",
                actual: "a suspended dependency outside the workbook scheduler",
                path: "$.formula.calculation".into(),
            }),
            Err(error) => Err(error),
        };
        self.clear();
        result
    }

    /// Release handles and values without dropping retained scratch capacity.
    pub(crate) fn clear(&mut self) {
        self.stack.clear();
        self.values.clear();
        self.arrays.clear();
        self.array_choices.clear();
        self.element_stack.clear();
        self.element_values.clear();
        self.lookup = None;
        self.names.clear();
        self.name_results.clear();
        self.name_volatility.clear();
    }

    pub(crate) fn volatile(&self) -> bool { self.volatile }

    pub(crate) fn begin(&mut self, expression: &Expr) {
        self.clear();
        self.volatile = false;
        #[cfg(feature = "internals")]
        { self.visited = 0; self.element_steps = 0; }
        self.values.resize_with(expression.nodes.len(), || None);
        self.stack.push(Frame::Node { name: None, id: expression.root, base: 0, usage: ReferenceUse::Scalar, ready: false });
    }

    pub(crate) fn resume<'w>(&mut self, expression: &'w Expr, context: &mut impl Context<'w>) -> crate::Result<Evaluation> {
        let system = context.system();
        while let Some(frame) = self.stack.pop() {
            let (name, id, base, usage, ready) = match frame {
                Frame::Node {
                    name,
                    id,
                    base,
                    usage,
                    ready,
                } => (name, id, base, usage, ready),
                Frame::Lookup {
                    name,
                    id,
                    base,
                    usage,
                } => {
                    let mut state = self
                        .lookup
                        .take()
                        .expect("a lookup continuation owns its scan");
                    match Self::search_axis(context, &mut state)? {
                        SearchProgress::Paused => {
                            self.lookup = Some(state);
                            self.stack.push(Frame::Lookup {
                                name,
                                id,
                                base,
                                usage,
                            });
                            return Ok(Evaluation::Paused);
                        }
                        SearchProgress::Complete(result) => {
                            match Self::deliver_lookup(state.delivery, result, context) {
                                SelectionStep::Complete(value) => {
                                    self.values[base + id] = Some(value)
                                }
                                SelectionStep::Branch { child, scalar } => {
                                    let usage = if scalar { ReferenceUse::Scalar } else { usage };
                                    self.stack.push(Frame::Branch {
                                        base,
                                        child,
                                        destination: base + id,
                                        scalar,
                                        usage,
                                    });
                                    self.stack.push(Frame::Node {
                                        name,
                                        id: child,
                                        base,
                                        usage,
                                        ready: false,
                                    });
                                }
                            }
                        }
                    }
                    continue;
                }
                Frame::Selection {
                    name,
                    id,
                    base,
                    usage,
                } => {
                    let arena = match name {
                        Some(name) => context
                            .expression(Some(name))
                            .expect("the named arena was proved"),
                        None => expression,
                    };
                    let EvaluationPolicy::Select(selection) = arena.nodes[id].evaluation_children()
                    else {
                        unreachable!("only selector nodes create this continuation")
                    };
                    let mut ready: crate::Result<bool> = Ok(true);
                    selection.visit_inputs_reverse(|input, usage| {
                        if let Ok(prior) = &ready {
                            let prior = *prior;
                            ready = self
                                .ready(base, input, usage, context)
                                .map(|next| prior && next);
                        }
                    });
                    if !ready? {
                        self.stack.push(Frame::Selection {
                            name,
                            id,
                            base,
                            usage,
                        });
                        return Ok(Evaluation::Paused);
                    }
                    if let Selection::Xlookup { args } = selection {
                        match self.select_xlookup(args, base, context)? {
                            LookupStart::Complete(SelectionStep::Complete(value)) => {
                                self.values[base + id] = Some(value)
                            }
                            LookupStart::Complete(SelectionStep::Branch { child, scalar }) => {
                                let usage = if scalar { ReferenceUse::Scalar } else { usage };
                                self.stack.push(Frame::Branch {
                                    base,
                                    child,
                                    destination: base + id,
                                    scalar,
                                    usage,
                                });
                                self.stack.push(Frame::Node {
                                    name,
                                    id: child,
                                    base,
                                    usage,
                                    ready: false,
                                });
                            }
                            LookupStart::Scan(state) => {
                                debug_assert!(self.lookup.is_none());
                                self.lookup = Some(state);
                                self.stack.push(Frame::Lookup {
                                    name,
                                    id,
                                    base,
                                    usage,
                                });
                            }
                        }
                        continue;
                    }
                    if self.array_selection(selection, name, id, base, context) {
                        continue;
                    }
                    match self.selection(selection, base, context)? {
                        SelectionStep::Complete(value) => self.values[base + id] = Some(value),
                        SelectionStep::Branch { child, scalar } => {
                            let usage = if scalar { ReferenceUse::Scalar } else { usage };
                            self.stack.push(Frame::Branch {
                                base,
                                child,
                                destination: base + id,
                                scalar,
                                usage,
                            });
                            self.stack.push(Frame::Node {
                                name,
                                id: child,
                                base,
                                usage,
                                ready: false,
                            });
                        }
                    }
                    continue;
                }
                Frame::ArraySelection { name, id, base, selector, test, choices, mut next, mut waiting } => {
                    while next < choices.end {
                        let choice = &self.array_choices[next];
                        let Some(child) = choice.child.filter(|_| choice.selected) else { next += 1; continue; };
                        if !waiting {
                            self.stack.push(Frame::ArraySelection { name, id, base, selector, test, choices: choices.clone(), next, waiting: true });
                            self.stack.push(Frame::Node { name, id: child, base, usage: ReferenceUse::Geometry, ready: false });
                            break;
                        }
                        let point = match self.values[base + child].as_ref().expect("selected branch was evaluated") {
                            Outcome::Computed(Operand::Reference(reference))
                            | Outcome::Intersection { value: Operand::Reference(reference), .. } =>
                                context.reference_geometry(*reference).is_some_and(|range| range.cell_count() == 1),
                            _ => false,
                        };
                        if point && !self.ready(base, child, ReferenceUse::Scalar, context)? {
                            self.stack.push(Frame::ArraySelection { name, id, base, selector, test, choices, next, waiting: true });
                            return Ok(Evaluation::Paused);
                        }
                        let value = self.take(base, child);
                        // A captured point reference is read once, never per
                        // array position. Multi-cell branch references remain
                        // a named boundary until array reference lifting exists.
                        let value = match value {
                            Outcome::Computed(Operand::Reference(reference))
                            | Outcome::Intersection { value: Operand::Reference(reference), .. } => {
                                match context.reference_geometry(reference) {
                                    Some(range) if range.cell_count() == 1 => context.scalar(Operand::Reference(reference))?,
                                    _ => Outcome::Uncomputed(Unevaluated::Array),
                                }
                            }
                            value => value,
                        };
                        self.array_choices[next].value = value;
                        next += 1; waiting = false;
                    }
                    if next == choices.end {
                        self.values[base + id] = Some(self.finish_array_selection(selector, test, choices, context));
                    }
                    continue;
                }
                Frame::Branch {
                    base,
                    child,
                    destination,
                    scalar,
                    usage,
                } => {
                    if !self.ready(base, child, usage, context)? {
                        self.stack.push(Frame::Branch {
                            base,
                            child,
                            destination,
                            scalar,
                            usage,
                        });
                        return Ok(Evaluation::Paused);
                    }
                    let value = self.take(base, child);
                    self.values[destination] = Some(if scalar {
                        self.project(value, context)?
                    } else {
                        match value {
                            Outcome::Array { array, .. } => Outcome::Array { array, implicit: true },
                            value => value,
                        }
                    });
                    continue;
                }
                Frame::Return {
                    name,
                    root,
                    base,
                    destination,
                } => {
                    let value = self.take(base, root);
                    let volatile = self
                        .name_volatility
                        .pop()
                        .expect("each name has one return");
                    if volatile {
                        if let Some(parent) = self.name_volatility.last_mut() {
                            *parent = true;
                        }
                    }
                    // A held outcome produced no volatile value. Reusing its
                    // refusal is safe and bounds still-unsupported alias DAGs.
                    // Computed volatile results must run once per occurrence.
                    if !volatile || matches!(value, Outcome::Uncomputed(_)) {
                        self.name_results.insert(
                            name,
                            NamedResult {
                                outcome: value.clone(),
                                volatile,
                            },
                        );
                    }
                    self.values.truncate(base);
                    self.values[destination] = Some(value);
                    self.names.remove(&name);
                    continue;
                }
            };
            let arena = match name {
                Some(name) => context
                    .expression(Some(name))
                    .expect("entry proved the immutable name arena"),
                None => expression,
            };
            #[cfg(feature = "internals")]
            {
                if !ready {
                    self.visited += 1;
                }
            }
            let node = &arena.nodes[id];
            if !ready {
                // Registry volatility applies even to a currently held call.
                // Propagation prevents caching computed volatile ancestors.
                if let Node::Call { function: Some(function), .. } = node
                    && function.info().volatile
                {
                    self.volatile = true;
                    if let Some(current) = self.name_volatility.last_mut() { *current = true; }
                }
                match node.evaluation_children() {
                    EvaluationPolicy::Leaf => {
                        self.values[base + id] = Some(match node {
                            Node::Literal(value) => Self::literal(value),
                            Node::Array(_) => Outcome::Array { array: ArrayId::Literal { name, node: id }, implicit: false },
                            Node::Reference(reference) => match context.reference(name, reference)? {
                                ReferenceResult::Value(value) => value,
                                ReferenceResult::Name(name) => match self.name_results.get(&name) {
                                    Some(cached) => {
                                        if cached.volatile
                                            && let Some(parent) = self.name_volatility.last_mut()
                                        {
                                            *parent = true;
                                        }
                                        cached.outcome.clone()
                                    }
                                    None => match context.expression(Some(name)) {
                                    Err(reason) => Outcome::Uncomputed(Unevaluated::Held(reason)),
                                    Ok(named) if self.names.insert(name) => {
                                        self.name_volatility.push(false);
                                        let next = self.values.len();
                                        self.values.resize_with(next + named.nodes.len(), || None);
                                        self.stack.push(Frame::Return {
                                            name, root: named.root, base: next, destination: base + id,
                                        });
                                        self.stack.push(Frame::Node {
                                            name: Some(name), id: named.root, base: next, usage, ready: false,
                                        });
                                        continue;
                                    }
                                    Ok(_) => Outcome::Uncomputed(Unevaluated::Reference),
                                    },
                                },
                            },
                            _ => {
                                unreachable!("the policy marks only literals/references as leaves")
                            }
                        });
                    }
                    EvaluationPolicy::Strict(children) => {
                        self.stack.push(Frame::Node { name, id, base, usage, ready: true });
                        children.visit_references(usage, |id, usage| self.stack.push(Frame::Node { name, id, base, usage, ready: false }));
                    }
                    EvaluationPolicy::Select(selection) => {
                        self.stack.push(Frame::Selection { name, id, base, usage });
                        selection.visit_inputs_reverse(|input, usage| {
                            self.stack.push(Frame::Node { name, id: input, base, usage, ready: false });
                        });
                    }
                    EvaluationPolicy::Held => {
                        let reason = match node {
                            Node::Held(reason) => Unevaluated::Held(*reason),
                            Node::Call { function, .. } => match function {
                                Some(function) => Unevaluated::Function(*function),
                                None => Unevaluated::Held(Held::UnknownFunction),
                            },
                            Node::Array(_) => Unevaluated::Array,
                            Node::Spill(_) => Unevaluated::Spill,
                            Node::Binary { op, .. } => Unevaluated::Binary(*op),
                            _ => unreachable!("strict nodes and leaves have their own policy"),
                        };
                        self.values[base + id] = Some(Outcome::Uncomputed(reason));
                    }
                }
                continue;
            }
            let EvaluationPolicy::Strict(children) = node.evaluation_children() else {
                unreachable!("only strict nodes use the ready-node continuation")
            };
            let mut ready: crate::Result<bool> = Ok(true);
            children.visit_references(usage, |child, usage| {
                if let Ok(prior) = &ready {
                    let prior = *prior;
                    ready = self
                        .ready(base, child, usage, context)
                        .map(|next| prior && next);
                }
            });
            if !ready? {
                self.stack.push(Frame::Node {
                    name,
                    id,
                    base,
                    usage,
                    ready: true,
                });
                return Ok(Evaluation::Paused);
            }
            if let Node::Call {
                function:
                    Some(
                        function @ (Function::Match
                        | Function::Vlookup
                        | Function::Hlookup
                        | Function::Lookup),
                    ),
                args,
            } = node
            {
                let started = match function {
                    Function::Match => self.match_index(args, base, context)?,
                    Function::Vlookup | Function::Hlookup => {
                        self.table_lookup(*function, args, base, context)?
                    }
                    Function::Lookup => self.vector_lookup(args, base, context)?,
                    _ => unreachable!("lookup policy was matched above"),
                };
                match started {
                    LookupStart::Complete(value) => self.values[base + id] = Some(value),
                    LookupStart::Scan(state) => {
                        debug_assert!(self.lookup.is_none());
                        self.lookup = Some(state);
                        self.stack.push(Frame::Lookup {
                            name,
                            id,
                            base,
                            usage,
                        });
                    }
                }
                continue;
            }
            let resolved = match node {
                Node::Group(child) => self.take(base, *child),
                Node::Unary { op, value } => {
                    let value = self.take(base, *value);
                    let referenced = matches!(&value,
                        Outcome::Computed(Operand::Reference(_))
                        | Outcome::Intersection { referenced: true, .. });
                    let value = if *op == UnaryOp::ImplicitIntersection {
                        match value {
                            Outcome::Computed(Operand::Reference(id))
                            | Outcome::Intersection { value: Operand::Reference(id), .. } => context.intersection(id),
                            value => self.project(value, context)?,
                        }
                    } else {
                        self.element(ElementOp::Unary(*op), value, None, context)?
                    };
                    if *op == UnaryOp::ImplicitIntersection {
                        Self::unary(*op, value, referenced, system)
                    } else { value }
                }
                Node::Percent(child) => {
                    let value = self.take(base, *child);
                    self.element(ElementOp::Percent, value, None, context)?
                }
                Node::Binary { op: op @ (BinaryOp::Range | BinaryOp::Intersection | BinaryOp::Union), left, right } => {
                    Self::reference_binary(*op, self.take(base, *left), self.take(base, *right), context)
                }
                Node::Binary { op, left, right } => {
                    let left = self.take(base, *left);
                    let right = self.take(base, *right);
                    self.element(ElementOp::Binary { op: *op, root: id == arena.root }, left, Some(right), context)?
                }
                Node::Call {
                    function: Some(function @ (Function::True | Function::False)),
                    args,
                } => {
                    let [] = args.as_ref() else {
                        unreachable!("the strict policy proved zero arguments")
                    };
                    Outcome::Computed(Operand::Boolean(*function == Function::True))
                }
                Node::Call {
                    function: Some(Function::Not),
                    args,
                } => {
                    let [Some(argument)] = args.as_ref() else {
                        unreachable!("the strict policy proved one present argument")
                    };
                    let value = Self::scalar(self.take(base, *argument), context)?;
                    Self::logical_not(value)
                }
                Node::Call {
                    function: Some(Function::Pi),
                    args,
                } => {
                    let [] = args.as_ref() else { unreachable!("the strict policy proved zero arguments") };
                    Self::numeric(std::f64::consts::PI)
                }
                Node::Call {
                    function: Some(Function::Na),
                    args,
                } => {
                    let [] = args.as_ref() else {
                        unreachable!("the strict policy proved zero arguments")
                    };
                    Outcome::Computed(Operand::Error(ExcelError::NA))
                }
                Node::Call {
                    function: Some(function @ (Function::Now | Function::Today | Function::Rand)),
                    args,
                } => {
                    let [] = args.as_ref() else {
                        unreachable!("the strict policy proved zero arguments")
                    };
                    match function {
                        Function::Now => Self::numeric(context.clock_serial(false)?),
                        Function::Today => Self::numeric(context.clock_serial(true)?),
                        Function::Rand => Self::numeric(
                            (context.random_u64() >> 11) as f64 / 9_007_199_254_740_992.0,
                        ),
                        _ => unreachable!("the call was matched above"),
                    }
                }
                Node::Call {
                    function: Some(Function::Randbetween),
                    args,
                } => {
                    let [Some(bottom), Some(top)] = args.as_ref() else {
                        unreachable!("the strict policy proved two present arguments")
                    };
                    let bottom = Self::scalar(self.take(base, *bottom), context)?;
                    let top = Self::scalar(self.take(base, *top), context)?;
                    Self::randbetween(bottom, top, system, context)
                }
                Node::Call {
                    function: Some(function @ (Function::Row | Function::Column | Function::Rows | Function::Columns)),
                    args,
                } => {
                    let value = args.first().and_then(|id| *id).map(|id| self.take(base, id));
                    self.geometry(*function, value, context)
                }
                Node::Call { function: Some(Function::Address), args } => {
                    self.address(args, base, context)?
                }
                Node::Call {
                    function: Some(Function::Isref),
                    args,
                } => {
                    let [Some(argument)] = args.as_ref() else {
                        unreachable!("the strict policy proved one present argument")
                    };
                    match self.take(base, *argument) {
                        Outcome::Computed(Operand::Reference(id)) => {
                            Outcome::Computed(Operand::Boolean(context.reference_is_union(id)
                                || context.reference_geometry(id).is_some()))
                        }
                        Outcome::Computed(_) => Outcome::Computed(Operand::Boolean(false)),
                        Outcome::Array { .. } => Outcome::Computed(Operand::Boolean(false)),
                        Outcome::Intersection { .. } => {
                            Outcome::Uncomputed(Unevaluated::Function(Function::Isref))
                        }
                        Outcome::Uncomputed(reason) => Outcome::Uncomputed(reason),
                    }
                }
                Node::Call {
                    function: Some(function @ (
                        Function::ErrorDotType | Function::Isblank | Function::Iserr
                        | Function::Iserror | Function::Iseven | Function::Islogical
                        | Function::Isna | Function::Isnontext | Function::Isnumber
                        | Function::Isodd | Function::Istext | Function::N
                    )),
                    args,
                } => {
                    let [Some(argument)] = args.as_ref() else {
                        unreachable!("the strict policy proved one present argument")
                    };
                    let value = Self::scalar(self.take(base, *argument), context)?;
                    Self::information(*function, value, system)
                }
                Node::Call {
                    function: Some(function @ (Function::Year | Function::Month | Function::Day)),
                    args,
                } => {
                    let [Some(argument)] = args.as_ref() else {
                        unreachable!("strict policy proved one calendar argument")
                    };
                    let value = Self::scalar(self.take(base, *argument), context)?;
                    Self::calendar_extract(*function, value, system)
                }
                Node::Call { function: Some(Function::Date), args } => {
                    let [Some(year), Some(month), Some(day)] = args.as_ref() else {
                        unreachable!("strict policy proved three present DATE arguments")
                    };
                    Self::date([
                        Self::scalar(self.take(base, *year), context)?,
                        Self::scalar(self.take(base, *month), context)?,
                        Self::scalar(self.take(base, *day), context)?,
                    ], system)
                }
                Node::Call { function: Some(Function::Time), args } => {
                    let [Some(hour), Some(minute), Some(second)] = args.as_ref() else {
                        unreachable!("strict policy proved three present TIME arguments")
                    };
                    Self::time([
                        Self::scalar(self.take(base, *hour), context)?,
                        Self::scalar(self.take(base, *minute), context)?,
                        Self::scalar(self.take(base, *second), context)?,
                    ], system)
                }
                Node::Call {
                    function: Some(function @ (Function::Edate | Function::Eomonth)), args,
                } => {
                    let [Some(serial), Some(months)] = args.as_ref() else {
                        unreachable!("strict policy proved two present month-shift arguments")
                    };
                    Self::month_shift(*function,
                        Self::scalar(self.take(base, *serial), context)?,
                        Self::scalar(self.take(base, *months), context)?, system)
                }
                Node::Call { function: Some(function @ (Function::Pv | Function::Fv | Function::Pmt)), args } => {
                    self.annuity(*function, args, base, context)?
                }
                Node::Call { function: Some(Function::Npv), args } => {
                    self.npv(base, args, context)?
                }
                Node::Call { function: Some(Function::Days), args } => {
                    let [Some(end), Some(start)] = args.as_ref() else {
                        unreachable!("strict policy proved two present DAYS arguments")
                    };
                    let end = Self::scalar(self.take(base, *end), context)?;
                    let start = Self::scalar(self.take(base, *start), context)?;
                    Self::days(end, start, system)
                }
                Node::Call {
                    function: Some(function @ (Function::Hour | Function::Minute | Function::Second)), args,
                } => {
                    let [Some(argument)] = args.as_ref() else {
                        unreachable!("strict policy proved one clock-part argument")
                    };
                    let value = Self::scalar(self.take(base, *argument), context)?;
                    Self::clock_extract(*function, value, system)
                }
                Node::Call {
                    function: Some(function @ (Function::Datevalue | Function::Timevalue)), args,
                } => {
                    let [Some(argument)] = args.as_ref() else {
                        unreachable!("strict policy proved one text-temporal argument")
                    };
                    let value = Self::scalar(self.take(base, *argument), context)?;
                    Self::parsed_temporal(*function, value, system)
                }
                Node::Call { function: Some(Function::Log), args } => {
                    let (value, radix) = match args.as_ref() {
                        [Some(value)] => (*value, None),
                        [Some(value), Some(radix)] => (*value, Some(*radix)),
                        _ => unreachable!("strict policy proved one or two logarithm arguments"),
                    };
                    let value = Self::scalar(self.take(base, value), context)?;
                    let radix = match radix {
                        Some(radix) => Some(Self::scalar(self.take(base, radix), context)?),
                        None => None,
                    };
                    Self::logarithm(value, radix, system)
                }
                Node::Call { function: Some(Function::Weekday), args } => {
                    let (serial, code) = match args.as_ref() {
                        [Some(serial)] => (*serial, None),
                        [Some(serial), Some(code)] => (*serial, Some(*code)),
                        _ => unreachable!("strict policy proved one or two weekday arguments"),
                    };
                    let value = Self::scalar(self.take(base, serial), context)?;
                    let code = match code {
                        Some(code) => Some(Self::scalar(self.take(base, code), context)?),
                        None => None,
                    };
                    Self::weekday(value, code, system)
                }
                Node::Call {
                    function: Some(function @ (Function::Exp | Function::Ln | Function::Log10
                        | Function::Degrees | Function::Radians | Function::Cos | Function::Asin
                        | Function::Sin | Function::Tan | Function::Acos | Function::Atan)),
                    args,
                } => {
                    let [Some(argument)] = args.as_ref() else {
                        unreachable!("the strict policy proved one present math argument")
                    };
                    let value = Self::scalar(self.take(base, *argument), context)?;
                    Self::pure_math(*function, value, system)
                }
                Node::Call {
                    function: Some(function @ (Function::T | Function::Clean | Function::Trim
                        | Function::Left | Function::Right | Function::Mid | Function::Exact
                        | Function::Char | Function::Code | Function::Proper | Function::Rept | Function::Lower | Function::Upper | Function::Substitute | Function::Find | Function::Replace)), args,
                } => {
                    let mut values = std::array::from_fn(|_| Outcome::Computed(Operand::Blank));
                    for (index, argument) in args.iter().enumerate() {
                        values[index] = Self::scalar(self.take(base, argument.expect("the strict policy proved present arguments")), context)?;
                    }
                    function.text(values, args.len(), context.text_compatibility(), system)
                }
                Node::Call {
                    function: Some(function @ (Function::Concat | Function::Concatenate | Function::Textjoin)), args,
                } => self.join(*function, base, args, context)?,
                Node::Call { function: Some(Function::Search), args } => {
                    let mut values = std::array::from_fn(|_| Outcome::Computed(Operand::Blank));
                    for (index, argument) in args.iter().enumerate() {
                        values[index] = Self::scalar(self.take(base, argument.expect("typed SEARCH arity")), context)?;
                    }
                    self.search.evaluate(values, args.len(), context.text_compatibility(), system)
                }
                Node::Call { function: Some(Function::Value), args } => {
                    let value = Self::scalar(self.take(base, args[0].expect("typed VALUE argument")), context)?;
                    match value.operand() {
                        Err(reason) => Outcome::Uncomputed(reason),
                        Ok(Operand::Text(text)) => match entry::value_number(text.as_str(), system) {
                            Some(Ok(value)) => Self::numeric(value),
                            Some(Err(error)) => Outcome::Computed(Operand::Error(error)),
                            None => Outcome::Uncomputed(Unevaluated::Coercion),
                        },
                        Ok(Operand::Blank) => Self::numeric(0.0),
                        Ok(Operand::Number(value)) => Self::numeric(value),
                        Ok(Operand::Error(error)) => Outcome::Computed(Operand::Error(error)),
                        Ok(Operand::Boolean(_)) => Outcome::Computed(Operand::Error(ExcelError::Value)),
                        Ok(Operand::Reference(_)) => unreachable!("Context scalarized VALUE's reference"),
                    }
                }
                Node::Call { function: Some(Function::Text), args } => {
                    let value = Self::scalar(self.take(base, args[0].expect("typed TEXT argument")), context)?;
                    let code = Self::scalar(self.take(base, args[1].expect("typed TEXT format")), context)?;
                    self.formatter.text(value, code, system)
                }
                Node::Call { function: Some(Function::Len), args } => {
                    let value = Self::scalar(self.take(base, args[0].expect("the strict policy proved the argument")), context)?;
                    context.text_compatibility().length(value)
                }
                Node::Call {
                    function: Some(function @ (Function::Abs | Function::Sqrt | Function::Fact | Function::Sign | Function::Int)),
                    args,
                } => match args.as_ref() {
                    [Some(argument)] => {
                        let value = self.take(base, *argument);
                        if *function == Function::Abs {
                            self.element(ElementOp::Absolute, value, None, context)?
                        } else {
                        let value = Self::scalar(value, context)?;
                        match function {
                            Function::Sqrt => Self::square_root(value, system),
                            Function::Fact => Self::factorial(value, system),
                            Function::Sign | Function::Int => Self::signed_integer(*function, value, system),
                            _ => unreachable!("the call was matched above"),
                        }
                        }
                    }
                    _ => Outcome::Uncomputed(Unevaluated::Function(*function)),
                },
                Node::Call { function: Some(Function::Sumproduct), args } =>
                    self.sum_product(base, args, context)?,
                Node::Call { function: Some(Function::Countblank), args } =>
                    self.count_blank(base, args, context)?,
                Node::Call {
                    function: Some(function @ (Function::Sum | Function::Count | Function::Counta
                        | Function::Min | Function::Max | Function::Average | Function::Averagea
                        | Function::Mina | Function::Maxa | Function::Product
                        | Function::Median | Function::Mode | Function::ModeDotSngl
                        | Function::Var | Function::Varp | Function::VarDotS | Function::VarDotP
                        | Function::Stdev | Function::Stdevp | Function::StdevDotS | Function::StdevDotP)),
                    args,
                } => self.aggregate(*function, base, args, AggregateMode::Plain, context)?,
                Node::Call { function: Some(Function::Subtotal), args } => {
                    match self.subtotal(base, args, context)? {
                        Some(value) => value,
                        None => {
                            self.stack.push(Frame::Node { name, id, base, usage, ready: true });
                            return Ok(Evaluation::Paused);
                        }
                    }
                }
                Node::Call { function: Some(function @ (Function::Countif | Function::Countifs
                    | Function::Sumif | Function::Sumifs | Function::Averageif
                    | Function::Averageifs | Function::Maxifs | Function::Minifs)), args } =>
                    self.criteria(*function, base, args, context)?,
                Node::Call {
                    function: Some(function @ (Function::Large | Function::Small
                        | Function::Percentile | Function::PercentileDotInc
                        | Function::Quartile | Function::QuartileDotInc
                        | Function::Rank | Function::RankDotEq)), args,
                } => self.order_statistic(*function, base, args, context)?,
                Node::Call {
                    function: Some(function @ (Function::Gcd | Function::Lcm)), args,
                } => self.integer_math(*function, base, args, context)?,
                Node::Call {
                    function: Some(function @ (Function::And | Function::Or | Function::Xor)),
                    args,
                } => self.logical(*function, base, args, context)?,
                Node::Call {
                    function: Some(Function::Trunc),
                    args,
                } => {
                    let [Some(value), rest @ ..] = args.as_ref() else {
                        unreachable!("the strict policy proved one or two present arguments")
                    };
                    let value = Self::scalar(self.take(base, *value), context)?;
                    let places = match rest {
                        [] => Outcome::Computed(Operand::Number(0.0)),
                        [Some(places)] => Self::scalar(self.take(base, *places), context)?,
                        _ => unreachable!("the strict policy proved TRUNC arity"),
                    };
                    Self::truncate(value, places, system)
                }
                Node::Call {
                    function: Some(function @ (Function::Even | Function::Odd)),
                    args,
                } => {
                    let [Some(value)] = args.as_ref() else {
                        unreachable!("the strict policy proved one parity argument")
                    };
                    let value = Self::scalar(self.take(base, *value), context)?;
                    Self::rounding_unary(*function, value, system)
                }
                Node::Call {
                    function: Some(function @ (Function::Roundup | Function::Rounddown
                        | Function::Quotient | Function::Ceiling | Function::Floor
                        | Function::Mround | Function::CeilingDotMath | Function::FloorDotMath)),
                    args,
                } => {
                    let [Some(value), rest @ ..] = args.as_ref() else {
                        unreachable!("the strict policy proved a present first argument")
                    };
                    let value = Self::scalar(self.take(base, *value), context)?;
                    let (significance, mode) = match rest {
                        [] => (Outcome::Computed(Operand::Number(1.0)),
                               Outcome::Computed(Operand::Number(0.0))),
                        [Some(second)] => (Self::scalar(self.take(base, *second), context)?,
                                           Outcome::Computed(Operand::Number(0.0))),
                        [Some(second), Some(mode)] =>
                            (Self::scalar(self.take(base, *second), context)?,
                             Self::scalar(self.take(base, *mode), context)?),
                        _ => unreachable!("the strict policy proved the rounding arity"),
                    };
                    Self::rounding_binary(*function, value, significance, mode, system)
                }
                Node::Call { function: Some(function @ (Function::Index | Function::Offset)), args } => {
                    self.indexed_reference(*function, args, base, context)?
                }
                Node::Call { function: Some(Function::Indirect), args } => {
                    self.indirect(args, base, context)?
                }
                Node::Call {
                    function: Some(Function::Round),
                    args,
                } => {
                    let [Some(value), Some(places)] = args.as_ref() else {
                        unreachable!("the strict call policy proved two present arguments")
                    };
                    let value = self.take(base, *value);
                    let places = self.take(base, *places);
                    self.element(ElementOp::Round, value, Some(places), context)?
                }
                Node::Call {
                    function: Some(Function::Atan2),
                    args,
                } => {
                    let [Some(x), Some(y)] = args.as_ref() else {
                        unreachable!("the strict policy proved two coordinate arguments")
                    };
                    let x = Self::scalar(self.take(base, *x), context)?;
                    let y = Self::scalar(self.take(base, *y), context)?;
                    Self::atan2(x, y, system)
                }
                Node::Call {
                    function: Some(Function::Power),
                    args,
                } => {
                    let [Some(left), Some(right)] = args.as_ref() else {
                        unreachable!("the strict call policy proved two present arguments")
                    };
                    let left = Self::scalar(self.take(base, *left), context)?;
                    let right = Self::scalar(self.take(base, *right), context)?;
                    Self::binary(BinaryOp::Power, left, right, false, system)
                }
                Node::Call {
                    function: Some(Function::Mod),
                    args,
                } => {
                    let [Some(left), Some(right)] = args.as_ref() else {
                        unreachable!("the strict call policy proved two present arguments")
                    };
                    let left = Self::scalar(self.take(base, *left), context)?;
                    let right = Self::scalar(self.take(base, *right), context)?;
                    Self::modulus(left, right, system)
                }
                _ => unreachable!("the ready stack contains operators with children"),
            };
            self.values[base + id] = Some(resolved);
        }
        if !self.ready(0, expression.root, ReferenceUse::Scalar, context)? {
            return Ok(Evaluation::Paused);
        }
        let result = self.take(0, expression.root);
        self.project(result, context).map(Evaluation::Complete)
    }

    fn ready<'w>(&self, base: usize, id: usize, usage: ReferenceUse, context: &mut impl Context<'w>) -> crate::Result<bool> {
        match self.values[base + id].as_ref().expect("the child was evaluated") {
            Outcome::Computed(Operand::Reference(id))
            | Outcome::Intersection { value: Operand::Reference(id), .. } => context.ready(*id, usage),
            _ => Ok(true),
        }
    }

    fn literal(value: &super::parser::Literal) -> Outcome {
        match Operand::literal(value) {
            Operand::Number(value) => Self::numeric(value),
            Operand::Error(ExcelError::Unrecognized) => Outcome::Uncomputed(Unevaluated::Held(Held::Unrecognized)),
            value => Outcome::Computed(value),
        }
    }

    /// Borrow constants in the already-resolved root or named arena. The
    /// parser proves nonempty rectangular rows and literal/signed-number cells.
    fn array<'w>(name: Option<NameId>, node: usize,
        context: &impl Context<'w>) -> Result<(&'w Expr, &'w [Box<[usize]>]), Held> {
        let arena = context.expression(name)?;
        let Node::Array(rows) = &arena.nodes[node] else { unreachable!("array outcome names its parsed node") };
        Ok((arena, rows))
    }

    fn array_item(arena: &Expr, node: usize, system: DateSystem) -> Outcome {
        match &arena.nodes[node] {
            Node::Literal(value) => Self::literal(value),
            Node::Unary { op, value } => {
                let Node::Literal(value) = &arena.nodes[*value] else { unreachable!("array sign has one numeric literal") };
                Self::unary(*op, Self::literal(value), false, system)
            }
            _ => unreachable!("the array parser accepts only constants"),
        }
    }

    /// Explicit @, scalar-only consumers and cell publication take top-left.
    fn project<'w>(&mut self, value: Outcome, context: &mut impl Context<'w>) -> crate::Result<Outcome> {
        match value {
            Outcome::Array { array, .. } => Ok(self.array_element(array, 0, 0, context)),
            value => Self::scalar(value, context),
        }
    }

    fn array_shape<'w>(&self, array: ArrayId, context: &impl Context<'w>) -> (usize, usize) {
        match array {
            ArrayId::Literal { name, node } => {
                let (_, rows) = Self::array(name, node, context).expect("a reached array has a parsed arena");
                (rows.len(), rows[0].len())
            }
            ArrayId::Mapped(index) => {
                let plan = &self.arrays[index];
                (plan.rows, plan.columns)
            }
        }
    }

    /// Resolve scalar references once, then retain only the operation and its
    /// operands. Singleton axes broadcast; other missing positions are #N/A.
    /// The retained plans and element stacks are bounded by reached expression
    /// nodes, never by the Cartesian product of array dimensions.
    fn element<'w>(&mut self, op: ElementOp, left: Outcome, right: Option<Outcome>,
        context: &mut impl Context<'w>) -> crate::Result<Outcome> {
        let left = self.element_argument(left, context)?;
        let right = match right {
            Some(value) => Some(self.element_argument(value, context)?),
            None => None,
        };
        let mut dimensions = None;
        for value in std::iter::once(&left).chain(right.iter()) {
            if let Outcome::Array { array, .. } = value {
                let (rows, columns) = self.array_shape(*array, context);
                let (old_rows, old_columns) = dimensions.unwrap_or((1, 1));
                dimensions = Some((old_rows.max(rows), old_columns.max(columns)));
            }
        }
        match dimensions {
            None => Ok(op.apply(left, right, context.system())),
            Some((rows, columns)) => {
                let index = self.arrays.len();
                self.arrays.push(ArrayPlan { operation: ArrayOperation::Element { op, left, right }, rows, columns, cached: None });
                Ok(Outcome::Array { array: ArrayId::Mapped(index), implicit: false })
            }
        }
    }

    /// Admit only choices selected by at least one array position. Branch
    /// evaluation remains on the existing resumable formula stack; this scan
    /// never evaluates a branch or copies its expression.
    fn array_selection<'w>(&mut self, selection: Selection<'_>, name: Option<NameId>,
        id: usize, base: usize, context: &impl Context<'w>) -> bool {
        let (selector, child) = match selection {
            Selection::If { test: Some(child), .. } => (ArraySelector::If, child),
            Selection::Choose { index: Some(child), .. } => (ArraySelector::Choose, child),
            _ => return false,
        };
        let Some(Outcome::Array { array: test, .. }) = &self.values[base + child] else { return false; };
        let test = *test;
        self.take(base, child);
        let start = self.array_choices.len();
        match selection {
            Selection::If { yes, no, .. } => {
                self.array_choices.push(ArrayChoice { child: yes, selected: false, value: Self::numeric(0.0) });
                self.array_choices.push(ArrayChoice { child: no.flatten(), selected: false,
                    value: if no.is_none() { Outcome::Computed(Operand::Boolean(false)) } else { Self::numeric(0.0) } });
            }
            Selection::Choose { choices, .. } => {
                self.array_choices.extend(choices.iter().map(|child| ArrayChoice { child: *child, selected: false, value: Self::numeric(0.0) }));
            }
            _ => unreachable!("only IF and CHOOSE admit array selectors"),
        }
        let choices = start..self.array_choices.len();
        let (rows, columns) = self.array_shape(test, context);
        for row in 0..rows {
            for column in 0..columns {
                let value = self.array_element(test, row, column, context);
                if let Ok(index) = selector.index(value, choices.len(), context.system()) {
                    self.array_choices[start + index].selected = true;
                }
            }
        }
        self.stack.push(Frame::ArraySelection { name, id, base, selector, test, choices, next: start, waiting: false });
        true
    }

    fn finish_array_selection<'w>(&mut self, selector: ArraySelector, test: ArrayId,
        choices: std::ops::Range<usize>, context: &impl Context<'w>) -> Outcome {
        let (mut rows, mut columns) = self.array_shape(test, context);
        for choice in &self.array_choices[choices.clone()] {
            if choice.selected && let Outcome::Array { array, .. } = &choice.value {
                let (next_rows, next_columns) = self.array_shape(*array, context);
                rows = rows.max(next_rows); columns = columns.max(next_columns);
            }
        }
        let index = self.arrays.len();
        self.arrays.push(ArrayPlan { operation: ArrayOperation::Selection { selector, test, choices }, rows, columns, cached: None });
        // Native legacy formulas insert @ at the function boundary when an
        // operator consumes this result; direct array reducers retain it.
        Outcome::Array { array: ArrayId::Mapped(index), implicit: true }
    }
    fn element_argument<'w>(&mut self, value: Outcome, context: &mut impl Context<'w>) -> crate::Result<Outcome> {
        match value {
            value @ Outcome::Array { implicit: true, .. } => self.project(value, context),
            value @ Outcome::Array { .. } => Ok(value),
            value => Self::scalar(value, context),
        }
    }

    /// Execute one requested position with an explicit stack. Captured scalar
    /// values (including volatile calls) are never evaluated again per element.
    fn array_element<'w>(&mut self, array: ArrayId, row: usize, column: usize,
        context: &impl Context<'w>) -> Outcome {
        debug_assert!(self.element_stack.is_empty() && self.element_values.is_empty());
        self.element_stack.push(ElementFrame::Read { value: Outcome::Array { array, implicit: false }, row, column });
        while let Some(frame) = self.element_stack.pop() {
            match frame {
                ElementFrame::Read { value: Outcome::Array { array, .. }, row, column } => {
                    let (rows, columns) = self.array_shape(array, context);
                    let row = if rows == 1 { 0 } else { row };
                    let column = if columns == 1 { 0 } else { column };
                    if row >= rows || column >= columns {
                        self.element_values.push(Outcome::Computed(Operand::Error(ExcelError::NA)));
                        continue;
                    }
                    match array {
                        ArrayId::Literal { name, node } => {
                            let (arena, rows) = Self::array(name, node, context).expect("a reached array has a parsed arena");
                            self.element_values.push(Self::array_item(arena, rows[row][column], context.system()));
                        }
                        ArrayId::Mapped(index) => {
                            let plan = &self.arrays[index];
                            if let Some((old_row, old_column, value)) = &plan.cached
                                && (*old_row, *old_column) == (row, column)
                            {
                                self.element_values.push(value.clone());
                                continue;
                            }
                            match &plan.operation {
                                ArrayOperation::Element { left, right, .. } => {
                                    self.element_stack.push(ElementFrame::Apply { index, row, column });
                                    if let Some(right) = right {
                                        self.element_stack.push(ElementFrame::Read { value: right.clone(), row, column });
                                    }
                                    self.element_stack.push(ElementFrame::Read { value: left.clone(), row, column });
                                }
                                ArrayOperation::Selection { test, .. } => {
                                    self.element_stack.push(ElementFrame::Select { index, row, column });
                                    self.element_stack.push(ElementFrame::Read { value: Outcome::Array { array: *test, implicit: false }, row, column });
                                }
                            }
                        }
                    }
                }
                ElementFrame::Read { value, .. } => self.element_values.push(value),
                ElementFrame::Apply { index, row, column } => {
                    #[cfg(feature = "internals")]
                    { self.element_steps += 1; }
                    let plan = &mut self.arrays[index];
                    let ArrayOperation::Element { op, right, .. } = &plan.operation else { unreachable!("element continuation has an element plan") };
                    let right = right.is_some().then(|| self.element_values.pop().expect("a binary element has its right value"));
                    let left = self.element_values.pop().expect("an element has its left value");
                    let value = op.apply(left, right, context.system());
                    plan.cached = Some((row, column, value.clone()));
                    self.element_values.push(value);
                }
                ElementFrame::Select { index, row, column } => {
                    #[cfg(feature = "internals")]
                    { self.element_steps += 1; }
                    let ArrayOperation::Selection { selector, choices, .. } = &self.arrays[index].operation else { unreachable!("selection continuation has a selection plan") };
                    let test = self.element_values.pop().expect("a selector has its condition");
                    match selector.index(test, choices.len(), context.system()) {
                        Ok(choice) => {
                            let choice = &self.array_choices[choices.start + choice];
                            debug_assert!(choice.selected, "mask scanning admitted every consumed choice");
                            self.element_stack.push(ElementFrame::Cache { index, row, column });
                            self.element_stack.push(ElementFrame::Read { value: choice.value.clone(), row, column });
                        }
                        Err(value) => {
                            self.arrays[index].cached = Some((row, column, value.clone()));
                            self.element_values.push(value);
                        }
                    }
                }
                ElementFrame::Cache { index, row, column } => {
                    let value = self.element_values.last().expect("selected branch produced one element").clone();
                    self.arrays[index].cached = Some((row, column, value));
                }
            }
        }
        let value = self.element_values.pop().expect("one requested position yields one outcome");
        debug_assert!(self.element_values.is_empty());
        value
    }
    fn scalar<'w>(value: Outcome, context: &mut impl Context<'w>) -> crate::Result<Outcome> {
        match value {
            Outcome::Computed(value @ Operand::Reference(_))
            | Outcome::Intersection { value: value @ Operand::Reference(_), .. } => context.scalar(value),
            Outcome::Intersection { value, .. } => Ok(Outcome::Computed(value)),
            Outcome::Array { .. } => Ok(Outcome::Uncomputed(Unevaluated::Array)),
            value => Ok(value),
        }
    }

    /// Scan one axis in source order. Blank runs preserve ordinal
    /// positions without materializing any grid cell. Exact lookup stops at
    /// the first hit, before a later error; sorted approximate retains one
    /// eligible ordinal and stops when the ordered threshold is passed.
    fn search_axis<'w>(
        context: &mut impl Context<'w>,
        state: &mut LookupState,
    ) -> crate::Result<SearchProgress> {
        let mut matcher = state.pattern.as_ref().map(Wildcard::matcher);
        let mut stopped = None;
        let progress = context.visit_range(
            state.source,
            RangeRead::Lookup,
            state.next,
            |value, count| {
                let first = state.next + 1;
                state.next += count;
                if count > 1 {
                    return std::ops::ControlFlow::Continue(());
                }
                let candidate = match value.operand() {
                    Ok(Operand::Blank) => return std::ops::ControlFlow::Continue(()),
                    Ok(Operand::Error(error)) => {
                        stopped = Some(Outcome::Computed(Operand::Error(error)));
                        return std::ops::ControlFlow::Break(());
                    }
                    Ok(value) => value,
                    Err(reason) => {
                        stopped = Some(Outcome::Uncomputed(reason));
                        return std::ops::ControlFlow::Break(());
                    }
                };
                if matches!(
                    state.mode,
                    SearchMode::NearestAtMost | SearchMode::NearestAtLeast
                ) && (!matches!(&candidate, Operand::Number(_))
                    || !matches!(&state.key, Operand::Number(_)))
                {
                    stopped = Some(Outcome::Uncomputed(Unevaluated::Function(
                        Function::Xlookup,
                    )));
                    return std::ops::ControlFlow::Break(());
                }
                let order = match (&mut matcher, &candidate) {
                    (Some(matcher), Operand::Text(text)) => {
                        Some(Ok(if matcher.is_match(text.as_str()) {
                            std::cmp::Ordering::Equal
                        } else {
                            std::cmp::Ordering::Less
                        }))
                    }
                    (Some(_), _) => Some(Ok(std::cmp::Ordering::Less)),
                    (None, _) => candidate.order(&state.key),
                };
                let order = match order {
                    Some(Ok(order)) => order,
                    Some(Err(error)) => {
                        stopped = Some(Outcome::Computed(Operand::Error(error)));
                        return std::ops::ControlFlow::Break(());
                    }
                    None => {
                        stopped = Some(Outcome::Uncomputed(Unevaluated::TextCompatibility));
                        return std::ops::ControlFlow::Break(());
                    }
                };
                match state.mode {
                    SearchMode::FirstEqual
                    | SearchMode::NearestAtMost
                    | SearchMode::NearestAtLeast
                        if order.is_eq() =>
                    {
                        state.found = Some(first);
                        std::ops::ControlFlow::Break(())
                    }
                    SearchMode::LastEqual if order.is_eq() => {
                        state.found = Some(first);
                        std::ops::ControlFlow::Continue(())
                    }
                    SearchMode::LastAtMost if order.is_le() => {
                        state.found = Some(first);
                        std::ops::ControlFlow::Continue(())
                    }
                    SearchMode::LastAtLeast if order.is_ge() => {
                        state.found = Some(first);
                        std::ops::ControlFlow::Continue(())
                    }
                    SearchMode::NearestAtMost if order.is_lt() => {
                        if let (Operand::Number(candidate), Operand::Number(_)) =
                            (&candidate, &state.key)
                        {
                            if state.best_number.is_none_or(|best| *candidate > best) {
                                state.best_number = Some(*candidate);
                                state.found = Some(first);
                            }
                        } else {
                            stopped = Some(Outcome::Uncomputed(Unevaluated::Function(
                                Function::Xlookup,
                            )));
                            return std::ops::ControlFlow::Break(());
                        }
                        std::ops::ControlFlow::Continue(())
                    }
                    SearchMode::NearestAtLeast if order.is_gt() => {
                        if let (Operand::Number(candidate), Operand::Number(_)) =
                            (&candidate, &state.key)
                        {
                            if state.best_number.is_none_or(|best| *candidate < best) {
                                state.best_number = Some(*candidate);
                                state.found = Some(first);
                            }
                        } else {
                            stopped = Some(Outcome::Uncomputed(Unevaluated::Function(
                                Function::Xlookup,
                            )));
                            return std::ops::ControlFlow::Break(());
                        }
                        std::ops::ControlFlow::Continue(())
                    }
                    SearchMode::LastAtMost | SearchMode::LastAtLeast => {
                        std::ops::ControlFlow::Break(())
                    }
                    SearchMode::FirstEqual
                    | SearchMode::LastEqual
                    | SearchMode::NearestAtMost
                    | SearchMode::NearestAtLeast => std::ops::ControlFlow::Continue(()),
                }
            },
        )?;
        Ok(match progress {
            RangeProgress::Paused(at) => {
                state.next = at;
                SearchProgress::Paused
            }
            RangeProgress::Complete => SearchProgress::Complete(match (stopped, state.found) {
                (Some(outcome), _) => SearchResult::Stopped(outcome),
                (None, Some(index)) => SearchResult::Found(index),
                (None, None) => SearchResult::Missing,
            }),
        })
    }

    fn deliver_lookup<'w>(
        delivery: LookupDelivery,
        result: SearchResult,
        context: &mut impl Context<'w>,
    ) -> SelectionStep {
        let complete = |value| SelectionStep::Complete(Outcome::Computed(value));
        match result {
            SearchResult::Stopped(outcome) => SelectionStep::Complete(outcome),
            SearchResult::Missing => match delivery {
                LookupDelivery::Xlookup {
                    fallback: Some(Some(child)),
                    ..
                } => SelectionStep::Branch {
                    child,
                    scalar: true,
                },
                LookupDelivery::Xlookup {
                    fallback: Some(None),
                    ..
                } => complete(Operand::Number(0.0)),
                _ => complete(Operand::Error(ExcelError::NA)),
            },
            SearchResult::Found(position) => {
                let offset = position as u32 - 1;
                match delivery {
                    LookupDelivery::Match => {
                        SelectionStep::Complete(Self::numeric(position as f64))
                    }
                    LookupDelivery::Table {
                        source,
                        start,
                        index,
                        vertical,
                    } => {
                        let at = if vertical {
                            CellRef::new(start.row() + offset, start.column() + index - 1)
                        } else {
                            CellRef::new(start.row() + index - 1, start.column() + offset)
                        };
                        SelectionStep::Complete(
                            context.reference_range(source, CellRange::new(at, at)),
                        )
                    }
                    LookupDelivery::Vector {
                        result,
                        start,
                        vertical,
                    }
                    | LookupDelivery::Xlookup {
                        result,
                        start,
                        vertical,
                        ..
                    } => {
                        let at = if vertical {
                            CellRef::new(start.row() + offset, start.column())
                        } else {
                            CellRef::new(start.row(), start.column() + offset)
                        };
                        SelectionStep::Complete(
                            context.reference_range(result, CellRange::new(at, at)),
                        )
                    }
                }
            }
        }
    }

    fn select_xlookup<'w>(
        &mut self,
        args: &[Option<usize>],
        base: usize,
        context: &mut impl Context<'w>,
    ) -> crate::Result<LookupStart<SelectionStep>> {
        let held = || {
            LookupStart::Complete(SelectionStep::Complete(Outcome::Uncomputed(
                Unevaluated::Function(Function::Xlookup),
            )))
        };
        let complete =
            |value| LookupStart::Complete(SelectionStep::Complete(Outcome::Computed(value)));
        let key = Self::scalar(self.take(base, args[0].expect("XLOOKUP key")), context)?;
        let key = match key.operand() {
            Ok(Operand::Error(error)) => return Ok(complete(Operand::Error(error))),
            Ok(value) => value,
            Err(reason) => {
                return Ok(LookupStart::Complete(SelectionStep::Complete(
                    Outcome::Uncomputed(reason),
                )));
            }
        };
        let source = match self
            .take(base, args[1].expect("XLOOKUP lookup array"))
            .operand()
        {
            Ok(Operand::Reference(id)) => id,
            Ok(Operand::Error(error)) => return Ok(complete(Operand::Error(error))),
            Ok(_) => return Ok(held()),
            Err(reason) => {
                return Ok(LookupStart::Complete(SelectionStep::Complete(
                    Outcome::Uncomputed(reason),
                )));
            }
        };
        let target = match self
            .take(base, args[2].expect("XLOOKUP return array"))
            .operand()
        {
            Ok(Operand::Reference(id)) => id,
            Ok(Operand::Error(error)) => return Ok(complete(Operand::Error(error))),
            Ok(_) => return Ok(held()),
            Err(reason) => {
                return Ok(LookupStart::Complete(SelectionStep::Complete(
                    Outcome::Uncomputed(reason),
                )));
            }
        };
        let (Some(source_range), Some(target_range)) = (
            context.reference_geometry(source),
            context.reference_geometry(target),
        ) else {
            return Ok(held());
        };
        let source_length = source_range.row_size().max(source_range.column_size());
        let target_length = target_range.row_size().max(target_range.column_size());
        if (source_range.row_size() != 1 && source_range.column_size() != 1)
            || (target_range.row_size() != 1 && target_range.column_size() != 1)
            || source_length != target_length
        {
            return Ok(held());
        }
        let mut match_mode = 0.0;
        let mut search_mode = 1.0;
        for (slot, result) in [(4, &mut match_mode), (5, &mut search_mode)] {
            if let Some(child) = args.get(slot).copied().flatten() {
                let value = Self::scalar(self.take(base, child), context)?;
                let value = match value.operand() {
                    Ok(value) => value,
                    Err(reason) => {
                        return Ok(LookupStart::Complete(SelectionStep::Complete(
                            Outcome::Uncomputed(reason),
                        )));
                    }
                };
                *result = match Self::coerce_number(value, context.system()) {
                    Ok(value) => value,
                    Err(value) => return Ok(LookupStart::Complete(SelectionStep::Complete(value))),
                };
            }
        }
        let mode = match (match_mode, search_mode) {
            (0.0, 1.0) | (2.0, 1.0) => SearchMode::FirstEqual,
            (0.0, -1.0) | (2.0, -1.0) => SearchMode::LastEqual,
            (-1.0, 1.0) => SearchMode::NearestAtMost,
            (1.0, 1.0) => SearchMode::NearestAtLeast,
            _ => return Ok(held()),
        };
        Ok(LookupStart::Scan(LookupState::new(
            key,
            source,
            mode,
            match_mode == 2.0,
            LookupDelivery::Xlookup {
                result: target,
                start: target_range.start(),
                vertical: target_range.column_size() == 1,
                fallback: args.get(3).copied(),
            },
        )))
    }

    fn match_index<'w>(
        &mut self,
        args: &[Option<usize>],
        base: usize,
        context: &mut impl Context<'w>,
    ) -> crate::Result<LookupStart<Outcome>> {
        let held =
            || LookupStart::Complete(Outcome::Uncomputed(Unevaluated::Function(Function::Match)));
        let key = Self::scalar(self.take(base, args[0].expect("MATCH key")), context)?;
        let key = match key.operand() {
            Ok(Operand::Error(error)) => {
                return Ok(LookupStart::Complete(Outcome::Computed(Operand::Error(
                    error,
                ))));
            }
            Ok(value) => value,
            Err(reason) => return Ok(LookupStart::Complete(Outcome::Uncomputed(reason))),
        };
        let source = match self.take(base, args[1].expect("MATCH source")).operand() {
            Ok(Operand::Reference(id)) => id,
            Ok(Operand::Error(error)) => {
                return Ok(LookupStart::Complete(Outcome::Computed(Operand::Error(
                    error,
                ))));
            }
            Ok(_) => return Ok(held()),
            Err(reason) => return Ok(LookupStart::Complete(Outcome::Uncomputed(reason))),
        };
        let Some(range) = context.reference_geometry(source) else {
            return Ok(held());
        };
        if range.row_size() != 1 && range.column_size() != 1 {
            return Ok(held());
        }
        let mode = if let Some(child) = args.get(2).copied().flatten() {
            let value = Self::scalar(self.take(base, child), context)?;
            let value = match value.operand() {
                Ok(value) => value,
                Err(reason) => return Ok(LookupStart::Complete(Outcome::Uncomputed(reason))),
            };
            match Self::coerce_number(value, context.system()) {
                Ok(value) => value,
                Err(value) => return Ok(LookupStart::Complete(value)),
            }
        } else {
            1.0
        };
        let mode = match mode {
            0.0 => SearchMode::FirstEqual,
            1.0 => SearchMode::LastAtMost,
            -1.0 => SearchMode::LastAtLeast,
            _ => return Ok(held()),
        };
        Ok(LookupStart::Scan(LookupState::new(
            key,
            source,
            mode,
            matches!(mode, SearchMode::FirstEqual),
            LookupDelivery::Match,
        )))
    }

    fn table_lookup<'w>(
        &mut self,
        function: Function,
        args: &[Option<usize>],
        base: usize,
        context: &mut impl Context<'w>,
    ) -> crate::Result<LookupStart<Outcome>> {
        let held = || LookupStart::Complete(Outcome::Uncomputed(Unevaluated::Function(function)));
        let key = Self::scalar(self.take(base, args[0].expect("table lookup key")), context)?;
        let key = match key.operand() {
            Ok(Operand::Error(error)) => {
                return Ok(LookupStart::Complete(Outcome::Computed(Operand::Error(
                    error,
                ))));
            }
            Ok(value) => value,
            Err(reason) => return Ok(LookupStart::Complete(Outcome::Uncomputed(reason))),
        };
        let table = match self
            .take(base, args[1].expect("table lookup range"))
            .operand()
        {
            Ok(Operand::Reference(id)) => id,
            Ok(Operand::Error(error)) => {
                return Ok(LookupStart::Complete(Outcome::Computed(Operand::Error(
                    error,
                ))));
            }
            Ok(_) => return Ok(held()),
            Err(reason) => return Ok(LookupStart::Complete(Outcome::Uncomputed(reason))),
        };
        let Some(range) = context.reference_geometry(table) else {
            return Ok(held());
        };
        let index = Self::scalar(
            self.take(base, args[2].expect("table lookup index")),
            context,
        )?;
        let index = match index.operand() {
            Ok(value) => match Self::coerce_number(value, context.system()) {
                Ok(value) => value,
                Err(value) => return Ok(LookupStart::Complete(value)),
            },
            Err(reason) => return Ok(LookupStart::Complete(Outcome::Uncomputed(reason))),
        };
        if !index.is_finite() || index < 1.0 {
            return Ok(LookupStart::Complete(Outcome::Computed(Operand::Error(
                ExcelError::Value,
            ))));
        }
        let vertical = function == Function::Vlookup;
        let limit = if vertical {
            range.column_size()
        } else {
            range.row_size()
        };
        if index.trunc() > f64::from(limit) {
            return Ok(LookupStart::Complete(Outcome::Computed(Operand::Error(
                ExcelError::Ref,
            ))));
        }
        let approximate = if let Some(child) = args.get(3).copied().flatten() {
            let value = Self::scalar(self.take(base, child), context)?;
            match value.operand() {
                Ok(value) => match value.logical() {
                    Some(Ok(value)) => value,
                    Some(Err(error)) => {
                        return Ok(LookupStart::Complete(Outcome::Computed(Operand::Error(
                            error,
                        ))));
                    }
                    None => return Ok(held()),
                },
                Err(reason) => return Ok(LookupStart::Complete(Outcome::Uncomputed(reason))),
            }
        } else {
            true
        };
        let mode = if approximate {
            SearchMode::LastAtMost
        } else {
            SearchMode::FirstEqual
        };
        let start = range.start();
        let axis = if vertical {
            CellRange::new(start, CellRef::new(range.end().row(), start.column()))
        } else {
            CellRange::new(start, CellRef::new(start.row(), range.end().column()))
        };
        let key_axis = context.reference_range(table, axis);
        let Ok(Operand::Reference(key_axis)) = key_axis.operand() else {
            return Ok(held());
        };
        Ok(LookupStart::Scan(LookupState::new(
            key,
            key_axis,
            mode,
            !approximate,
            LookupDelivery::Table {
                source: table,
                start,
                index: index.trunc() as u32,
                vertical,
            },
        )))
    }

    fn vector_lookup<'w>(
        &mut self,
        args: &[Option<usize>],
        base: usize,
        context: &mut impl Context<'w>,
    ) -> crate::Result<LookupStart<Outcome>> {
        let held =
            || LookupStart::Complete(Outcome::Uncomputed(Unevaluated::Function(Function::Lookup)));
        let key = Self::scalar(self.take(base, args[0].expect("LOOKUP key")), context)?;
        let key = match key.operand() {
            Ok(Operand::Error(error)) => {
                return Ok(LookupStart::Complete(Outcome::Computed(Operand::Error(
                    error,
                ))));
            }
            Ok(value) => value,
            Err(reason) => return Ok(LookupStart::Complete(Outcome::Uncomputed(reason))),
        };
        let source = match self.take(base, args[1].expect("LOOKUP vector")).operand() {
            Ok(Operand::Reference(id)) => id,
            Ok(Operand::Error(error)) => {
                return Ok(LookupStart::Complete(Outcome::Computed(Operand::Error(
                    error,
                ))));
            }
            Ok(_) => return Ok(held()),
            Err(reason) => return Ok(LookupStart::Complete(Outcome::Uncomputed(reason))),
        };
        let result = match self
            .take(base, args[2].expect("LOOKUP result vector"))
            .operand()
        {
            Ok(Operand::Reference(id)) => id,
            Ok(Operand::Error(error)) => {
                return Ok(LookupStart::Complete(Outcome::Computed(Operand::Error(
                    error,
                ))));
            }
            Ok(_) => return Ok(held()),
            Err(reason) => return Ok(LookupStart::Complete(Outcome::Uncomputed(reason))),
        };
        let (Some(source_range), Some(result_range)) = (
            context.reference_geometry(source),
            context.reference_geometry(result),
        ) else {
            return Ok(held());
        };
        let source_length = source_range.row_size().max(source_range.column_size());
        let result_length = result_range.row_size().max(result_range.column_size());
        if (source_range.row_size() != 1 && source_range.column_size() != 1)
            || (result_range.row_size() != 1 && result_range.column_size() != 1)
            || source_length != result_length
        {
            return Ok(held());
        }
        Ok(LookupStart::Scan(LookupState::new(
            key,
            source,
            SearchMode::LastAtMost,
            false,
            LookupDelivery::Vector {
                result,
                start: result_range.start(),
                vertical: result_range.column_size() == 1,
            },
        )))
    }

    /// Parse a runtime A1 string once per volatile evaluation using the
    /// formula lexer's single reference grammar, then retain its identity in
    /// the existing context arena. The consumer admits selected value edges.
    fn indirect<'w>(
        &mut self, args: &[Option<usize>], base: usize, context: &mut impl Context<'w>,
    ) -> crate::Result<Outcome> {
        let child = args[0].expect("the strict policy proved INDIRECT text");
        let text = Self::scalar(self.take(base, child), context)?;
        let text = match text.operand() {
            Ok(Operand::Text(text)) => text,
            Ok(Operand::Error(error)) => return Ok(Outcome::Computed(Operand::Error(error))),
            Ok(_) => return Ok(Outcome::Uncomputed(Unevaluated::Coercion)),
            Err(reason) => return Ok(Outcome::Uncomputed(reason)),
        };
        if let Some(mode) = args.get(1).copied().flatten() {
            let mode = Self::scalar(self.take(base, mode), context)?;
            match mode.operand() {
                Ok(Operand::Error(error)) => return Ok(Outcome::Computed(Operand::Error(error))),
                Ok(value) => match value.logical() {
                    Some(Ok(true)) => {},
                    Some(Ok(false)) => return Ok(Outcome::Uncomputed(Unevaluated::Function(Function::Indirect))),
                    Some(Err(error)) => return Ok(Outcome::Computed(Operand::Error(error))),
                    None => return Ok(Outcome::Uncomputed(Unevaluated::Coercion)),
                },
                Err(reason) => return Ok(Outcome::Uncomputed(reason)),
            }
        }
        let mut lexemes = lexer::lex(text.as_str().trim());
        if lexemes.len() != 1 {
            return Ok(Outcome::Computed(Operand::Error(ExcelError::Ref)));
        }
        let Kind::Reference(raw) = lexemes.remove(0).kind else {
            return Ok(Outcome::Computed(Operand::Error(ExcelError::Ref)));
        };
        let reference = shape::relative(&raw, context.host());
        Ok(match context.reference(None, &reference)? {
            // An unresolved name in reference text is INDIRECT's #REF!,
            // whereas the same spelling entered as a formula is #NAME?.
            ReferenceResult::Value(Outcome::Computed(Operand::Error(ExcelError::Name))) =>
                Outcome::Computed(Operand::Error(ExcelError::Ref)),
            ReferenceResult::Value(value) => value,
            ReferenceResult::Name(_) => Outcome::Uncomputed(Unevaluated::Function(Function::Indirect)),
        })
    }

    fn indexed_reference<'w>(
        &mut self,
        function: Function,
        args: &[Option<usize>],
        base: usize,
        context: &mut impl Context<'w>,
    ) -> crate::Result<Outcome> {
        let held = || Outcome::Uncomputed(Unevaluated::Function(function));
        let error = |error| Outcome::Computed(Operand::Error(error));
        let Some(source) = args[0] else { return Ok(error(ExcelError::Ref)); };
        let source = self.take(base, source);
        let source = match source.operand() {
            Ok(Operand::Reference(id)) => id,
            Ok(Operand::Error(value)) => return Ok(error(value)),
            Err(reason) => return Ok(Outcome::Uncomputed(reason)),
            _ => return Ok(held()),
        };
        let mut indices = [None; 4];
        for (target, child) in indices.iter_mut().zip(&args[1..]) {
            if let Some(child) = child {
                let value = Self::scalar(self.take(base, *child), context)?;
                let value = match value.operand() {
                    Ok(value) => value,
                    Err(reason) => return Ok(Outcome::Uncomputed(reason)),
                };
                *target = Some(match Self::coerce_number(value, context.system()) {
                    Ok(value) => value,
                    Err(value) => return Ok(value),
                });
            }
        }
        let source = if function == Function::Index {
            let area = indices[2].unwrap_or(1.0);
            if area < 1.0 { return Ok(error(ExcelError::Value)); }
            if area >= usize::MAX as f64 { return Ok(error(ExcelError::Ref)); }
            match context.reference_nth(source, area.trunc() as usize - 1).operand() {
                Ok(Operand::Reference(id)) => id,
                Ok(Operand::Error(value)) => return Ok(error(value)),
                Err(reason) => return Ok(Outcome::Uncomputed(reason)),
                _ => return Ok(held()),
            }
        } else {
            if context.reference_is_union(source) { return Ok(error(ExcelError::Value)); }
            source
        };
        let Some(range) = context.reference_geometry(source) else { return Ok(held()); };
        let selected = if function == Function::Index {
            let (mut row, mut column) = (indices[0].unwrap_or(0.0), indices[1].unwrap_or(0.0));
            if row < 0.0 || column < 0.0 { return Ok(error(ExcelError::Value)); }
            row = row.trunc();
            column = column.trunc();
            if args.len() == 2 && range.row_size() > 1 && range.column_size() > 1 {
                return Ok(error(ExcelError::Ref));
            }
            if args.len() == 2 && range.row_size() == 1 {
                column = row;
                row = 1.0;
            }
            if row > f64::from(range.row_size()) || column > f64::from(range.column_size()) {
                return Ok(error(ExcelError::Ref));
            }
            let (first_row, last_row) = if row == 0.0 {
                (range.start().row(), range.end().row())
            } else {
                let row = range.start().row() + row as u32 - 1;
                (row, row)
            };
            let (first_column, last_column) = if column == 0.0 {
                (range.start().column(), range.end().column())
            } else {
                let column = range.start().column() + column as u32 - 1;
                (column, column)
            };
            CellRange::new(CellRef::new(first_row, first_column), CellRef::new(last_row, last_column))
        } else {
            let row = f64::from(range.start().row()) + indices[0].unwrap_or(0.0).trunc();
            let column = f64::from(range.start().column()) + indices[1].unwrap_or(0.0).trunc();
            let rows = indices[2].unwrap_or(f64::from(range.row_size())).trunc();
            let columns = indices[3].unwrap_or(f64::from(range.column_size())).trunc();
            // Excel's signed dimensions extend from the displaced anchor
            // toward the preceding row/column, with the anchor included.
            let (first_row, last_row) = if rows < 0.0 { (row + rows + 1.0, row) }
                else { (row, row + rows - 1.0) };
            let (first_column, last_column) = if columns < 0.0 { (column + columns + 1.0, column) }
                else { (column, column + columns - 1.0) };
            if rows == 0.0 || columns == 0.0 || first_row < 0.0 || first_column < 0.0
                || last_row >= f64::from(crate::excel::MAX_ROWS)
                || last_column >= f64::from(crate::excel::MAX_COLUMNS)
            {
                return Ok(error(ExcelError::Ref));
            }
            CellRange::new(CellRef::new(first_row as u32, first_column as u32),
                CellRef::new(last_row as u32, last_column as u32))
        };
        Ok(context.reference_range(source, selected))
    }

    fn geometry<'w>(
        &self,
        function: Function,
        value: Option<Outcome>,
        context: &impl Context<'w>,
    ) -> Outcome {
        let dimensions = matches!(function, Function::Rows | Function::Columns);
        let range = match value {
            None => CellRange::new(context.host(), context.host()),
            Some(Outcome::Computed(Operand::Reference(id)))
            | Some(Outcome::Intersection { value: Operand::Reference(id), .. }) => {
                if context.reference_is_union(id) {
                    return Outcome::Computed(Operand::Error(ExcelError::Ref));
                }
                let Some(range) = context.reference_geometry(id) else {
                    return Outcome::Computed(Operand::Error(ExcelError::Value));
                };
                range
            }
            Some(Outcome::Array { array, .. }) if dimensions => {
                let (rows, columns) = self.array_shape(array, context);
                return Self::numeric(if function == Function::Rows { rows as f64 } else { columns as f64 });
            }
            Some(Outcome::Intersection { value: Operand::Error(error), .. }) => {
                return Outcome::Computed(Operand::Error(error));
            }
            Some(Outcome::Computed(Operand::Error(error))) if dimensions => {
                return Outcome::Computed(Operand::Error(error));
            }
            Some(Outcome::Computed(_)) | Some(Outcome::Intersection { .. }) if dimensions => {
                return Self::numeric(1.0);
            }
            Some(Outcome::Uncomputed(reason)) => return Outcome::Uncomputed(reason),
            // Native rejected the authored scalar/array ROW/COLUMN files at
            // Open; there is no observed cache to turn into a guessed error.
            Some(_) => return Outcome::Uncomputed(Unevaluated::Function(function)),
        };
        Self::numeric(f64::from(match function {
            Function::Row => range.start().row() + 1,
            Function::Column => range.start().column() + 1,
            Function::Rows => range.row_size(),
            Function::Columns => range.column_size(),
            _ => unreachable!("only geometry calls reach this owner"),
        }))
    }

    fn selection<'w>(
        &mut self,
        selection: Selection<'_>,
        base: usize,
        context: &mut impl Context<'w>,
    ) -> crate::Result<SelectionStep> {
        let system = context.system();
        let project_input = matches!(selection, Selection::Error { .. });
        let mut input = |child| match child {
            Some(child) => {
                let value = self.take(base, child);
                if project_input { self.project(value, context) } else { Self::scalar(value, context) }
            },
            None => Ok(Outcome::Computed(Operand::Blank)),
        };
        let complete = |value| SelectionStep::Complete(Outcome::Computed(value));
        let held = |reason| SelectionStep::Complete(Outcome::Uncomputed(reason));
        let branch = |child, scalar| match child {
            Some(child) => SelectionStep::Branch { child, scalar },
            None => complete(Operand::Number(0.0)),
        };
        Ok(match selection {
            Selection::If { test, yes, no } => {
                match ArraySelector::If.index(input(test)?, 2, system) {
                    Ok(0) => branch(yes, false),
                    Ok(_) => match no {
                        Some(no) => branch(no, false),
                        None => complete(Operand::Boolean(false)),
                    },
                    Err(value) => SelectionStep::Complete(value),
                }
            }
            Selection::Error {
                value,
                fallback,
                na_only,
            } => match input(value)?.operand() {
                Err(reason) => held(reason),
                Ok(Operand::Error(error)) if !na_only || error == ExcelError::NA => {
                    branch(fallback, true)
                }
                Ok(Operand::Blank) => complete(Operand::Number(0.0)),
                Ok(value) => complete(value),
            },
            Selection::Choose { index, choices } => {
                match ArraySelector::Choose.index(input(index)?, choices.len(), system) {
                    Ok(index) => branch(choices[index], false),
                    Err(value) => SelectionStep::Complete(value),
                }
            }
            Selection::Ifs { pairs } => {
                for pair in pairs.chunks_exact(2) {
                    let value = match input(pair[0])?.operand() {
                        Ok(value) => value,
                        Err(reason) => return Ok(held(reason)),
                    };
                    match value.logical() {
                        Some(Ok(true)) => return Ok(branch(pair[1], false)),
                        Some(Ok(false)) => {}
                        Some(Err(error)) => return Ok(complete(Operand::Error(error))),
                        None => return Ok(held(Unevaluated::Function(Function::Ifs))),
                    }
                }
                complete(Operand::Error(ExcelError::NA))
            }
            Selection::Xlookup { .. } => {
                unreachable!("XLOOKUP is handled before the shared scalar selectors")
            }
            Selection::Switch {
                expression,
                pairs,
                fallback,
            } => {
                let value = match input(expression)?.operand() {
                    Ok(Operand::Error(error)) => return Ok(complete(Operand::Error(error))),
                    Ok(value) => value,
                    Err(reason) => return Ok(held(reason)),
                };
                for pair in pairs.chunks_exact(2) {
                    let key = match input(pair[0])?.operand() {
                        Ok(value) => value,
                        Err(reason) => return Ok(held(reason)),
                    };
                    match value.order(&key) {
                        Some(Ok(order)) if crate::expression::Comparison::Eq.answers(order) => {
                            return Ok(branch(pair[1], false));
                        }
                        Some(Ok(_)) => {}
                        Some(Err(error)) => return Ok(complete(Operand::Error(error))),
                        None => return Ok(held(Unevaluated::Coercion)),
                    }
                }
                match fallback {
                    Some(fallback) => branch(fallback, false),
                    None => complete(Operand::Error(ExcelError::NA)),
                }
            }
        })
    }

    fn take(&mut self, base: usize, id: usize) -> Outcome {
        self.values[base + id].take().expect("the child was scheduled")
    }

    fn unary(op: UnaryOp, value: Outcome, referenced: bool, system: DateSystem) -> Outcome {
        let value = match value.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        if op == UnaryOp::ImplicitIntersection {
            return Outcome::Intersection { value, referenced };
        }
        if op == UnaryOp::Positive {
            return Outcome::Computed(value);
        }
        let value = match Self::coerce_number(value, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        match op {
            UnaryOp::Negative => Self::numeric(-value),
            UnaryOp::Positive | UnaryOp::ImplicitIntersection => {
                unreachable!("handled before coercion")
            }
        }
    }

    fn absolute(value: Outcome, system: DateSystem) -> Outcome {
        let value = match value.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let value = match Self::coerce_number(value, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        match Scalar::from(value).checked_abs() {
            Ok(result) => Self::numeric(
                result
                    .as_f64()
                    .expect("the shared magnitude preserves a float64 operand"),
            ),
            Err(_) => Outcome::Uncomputed(Unevaluated::NumericPolicy),
        }
    }

    fn calendar_extract(function: Function, value: Outcome, system: DateSystem) -> Outcome {
        let value = match value.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let serial = match Self::coerce_number(value, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        let Some(calendar) = system.calendar(serial) else {
            return Outcome::Computed(Operand::Error(ExcelError::Num));
        };
        let part = match function {
            Function::Year => calendar.year,
            Function::Month => calendar.month,
            Function::Day => calendar.day,
            _ => unreachable!("only YEAR/MONTH/DAY reach this extraction"),
        };
        Self::numeric(part as f64)
    }

    fn three_numbers(values: [Outcome; 3], system: DateSystem) -> Result<[f64; 3], Outcome> {
        let mut parts = [0.0; 3];
        let mut held = None;
        let mut error = None;
        for (index, outcome) in values.into_iter().enumerate() {
            match outcome.operand() {
                Err(reason) => { held.get_or_insert(reason); }
                Ok(Operand::Error(value)) => { error.get_or_insert(value); }
                Ok(value) => match Self::coerce_number(value, system) {
                    Ok(value) => parts[index] = value,
                    Err(Outcome::Uncomputed(reason)) => { held.get_or_insert(reason); }
                    Err(Outcome::Computed(Operand::Error(value))) => { error.get_or_insert(value); }
                    Err(_) => unreachable!("numeric coercion returns held or typed error"),
                },
            }
        }
        if let Some(reason) = held {
            return Err(Outcome::Uncomputed(reason));
        }
        if let Some(error) = error {
            return Err(Outcome::Computed(Operand::Error(error)));
        }
        Ok(parts)
    }

    fn date(values: [Outcome; 3], system: DateSystem) -> Outcome {
        let parts = match Self::three_numbers(values, system) {
            Ok(parts) => parts,
            Err(outcome) => return outcome,
        };
        match system.date_serial(parts[0], parts[1], parts[2]) {
            Some(serial) => Self::numeric(serial as f64),
            None => Outcome::Computed(Operand::Error(ExcelError::Num)),
        }
    }

    fn time(values: [Outcome; 3], system: DateSystem) -> Outcome {
        let parts = match Self::three_numbers(values, system) {
            Ok(parts) => parts,
            Err(outcome) => return outcome,
        };
        if parts.iter().any(|value| !value.is_finite() || !(0.0..=32_767.0).contains(value)) {
            return Outcome::Computed(Operand::Error(ExcelError::Num));
        }
        let hour = parts[0].trunc() as i64;
        let minute = parts[1].trunc() as i64;
        let second = parts[2].trunc() as i64;
        let seconds = hour * 3_600 + minute * 60 + second;
        Self::numeric((seconds as f64 / 86_400.0).fract())
    }

    fn month_shift(function: Function, serial: Outcome, months: Outcome, system: DateSystem) -> Outcome {
        let (serial, months) = match (serial.operand(), months.operand()) {
            (Err(reason), _) | (_, Err(reason)) => return Outcome::Uncomputed(reason),
            (Ok(Operand::Error(error)), _) | (_, Ok(Operand::Error(error))) =>
                return Outcome::Computed(Operand::Error(error)),
            (Ok(serial), Ok(months)) => (serial, months),
        };
        let serial = match Self::coerce_number(serial, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        let months = match Self::coerce_number(months, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        match system.month_serial(serial, months, function == Function::Eomonth) {
            Some(day) => Self::numeric(day as f64),
            None => Outcome::Computed(Operand::Error(ExcelError::Num)),
        }
    }

    fn clock_extract(function: Function, value: Outcome, system: DateSystem) -> Outcome {
        let value = match value.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let serial = match Self::coerce_number(value, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        let Some(seconds) = system.clock_second(serial) else {
            return Outcome::Computed(Operand::Error(ExcelError::Num));
        };
        let part = match function {
            Function::Hour => seconds / 3_600,
            Function::Minute => (seconds / 60) % 60,
            Function::Second => seconds % 60,
            _ => unreachable!("only HOUR/MINUTE/SECOND extract clock parts"),
        };
        Self::numeric(part as f64)
    }

    fn parsed_temporal(function: Function, value: Outcome, system: DateSystem) -> Outcome {
        let value = match value.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let text = match value {
            Operand::Text(text) => text,
            Operand::Error(error) => return Outcome::Computed(Operand::Error(error)),
            _ => return Outcome::Computed(Operand::Error(ExcelError::Value)),
        };
        let parsed = match function {
            Function::Datevalue => entry::date_value(text.as_str(), system),
            Function::Timevalue => Some(entry::time_value(text.as_str())),
            _ => unreachable!("only DATEVALUE/TIMEVALUE parse text"),
        };
        match parsed {
            Some(Ok(value)) => Self::numeric(value),
            Some(Err(error)) => Outcome::Computed(Operand::Error(error)),
            None => Outcome::Uncomputed(Unevaluated::TextCompatibility),
        }
    }

    fn days(end: Outcome, start: Outcome, system: DateSystem) -> Outcome {
        let (end, start) = match (end.operand(), start.operand()) {
            (Err(reason), _) | (_, Err(reason)) => return Outcome::Uncomputed(reason),
            (Ok(Operand::Error(error)), _) | (_, Ok(Operand::Error(error))) => {
                return Outcome::Computed(Operand::Error(error));
            }
            (Ok(end), Ok(start)) => (end, start),
        };
        let end = match Self::coerce_number(end, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        let start = match Self::coerce_number(start, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        let (Some(end), Some(start)) = (system.serial_day(end), system.serial_day(start)) else {
            return Outcome::Computed(Operand::Error(ExcelError::Num));
        };
        Self::numeric((end - start) as f64)
    }

    fn weekday(value: Outcome, code: Option<Outcome>, system: DateSystem) -> Outcome {
        let value = match value.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let serial = match Self::coerce_number(value, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        let code = match code {
            Some(code) => {
                let code = match code.operand() {
                    Ok(value) => value,
                    Err(reason) => return Outcome::Uncomputed(reason),
                };
                match Self::coerce_number(code, system) {
                    Ok(value) => value,
                    Err(outcome) => return outcome,
                }
            }
            None => 1.0,
        };
        let Some(calendar) = system.calendar(serial) else {
            return Outcome::Computed(Operand::Error(ExcelError::Num));
        };
        let weekday = calendar.weekday;
        // Native fractional return codes truncate before the mode lookup.
        let code = code.trunc();
        let result = if code == 1.0 {
            weekday + 1
        } else if code == 2.0 {
            (weekday + 6).rem_euclid(7) + 1
        } else if code == 3.0 {
            (weekday + 6).rem_euclid(7)
        } else if (11.0..=17.0).contains(&code) {
            let first = (code as i64 - 10).rem_euclid(7);
            (weekday - first).rem_euclid(7) + 1
        } else {
            // WEEKDAY's documented return-type table is closed.
            return Outcome::Computed(Operand::Error(ExcelError::Num));
        };
        Self::numeric(result as f64)
    }

    fn logarithm(value: Outcome, radix: Option<Outcome>, system: DateSystem) -> Outcome {
        let mut operands = [1.0, 10.0];
        for (index, value) in [Some(value), radix].into_iter().enumerate() {
            if let Some(value) = value {
                let value = match value.operand() {
                    Ok(value) => value,
                    Err(reason) => return Outcome::Uncomputed(reason),
                };
                operands[index] = match Self::coerce_number(value, system) {
                    Ok(value) => value,
                    Err(outcome) => return outcome,
                };
            }
        }
        let [value, radix] = operands;
        if value <= 0.0 || radix <= 0.0 {
            return Outcome::Computed(Operand::Error(ExcelError::Num));
        }
        if radix == 1.0 {
            return Outcome::Computed(Operand::Error(ExcelError::Div0));
        }
        // Native LOG takes the shared decimal kernel for base ten and the
        // shared natural logarithm ratio for other bases; the two orders
        // have different observable binary64 tails.
        let source = Scalar::from(value);
        let result = if radix == 10.0 { source.checked_log10() } else {
            source.checked_ln().and_then(|value| {
                Scalar::from(radix).checked_ln().and_then(|divisor| value.checked_div(&divisor))
            })
        };
        match result {
            Ok(value) => Self::numeric(value.as_f64().expect("bound logarithm preserves Float64")),
            Err(_) => Outcome::Uncomputed(Unevaluated::NumericPolicy),
        }
    }

    fn pure_math(function: Function, value: Outcome, system: DateSystem) -> Outcome {
        let value = match value.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let value = match Self::coerce_number(value, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        if matches!(function, Function::Ln | Function::Log10) && value <= 0.0 {
            return Outcome::Computed(Operand::Error(ExcelError::Num));
        }
        // The native endpoint-sensitive negative ASIN region differs from
        // the generic binary64 kernel; retain its preexisting cached value.
        if function == Function::Asin
            && value > -1.0 && value < -std::f64::consts::FRAC_1_SQRT_2 {
            return Outcome::Uncomputed(Unevaluated::NumericPolicy);
        }
        if function == Function::Acos {
            if !(-1.0..=1.0).contains(&value) {
                return Outcome::Computed(Operand::Error(ExcelError::Num));
            }
            if value.is_subnormal() || (-1.0 < value && value < 0.0) {
                return Outcome::Uncomputed(Unevaluated::NumericPolicy);
            }
            if value == -1.0 {
                return Self::numeric(std::f64::consts::PI);
            }
            // For the nonnegative half-domain, Excel's observed inverse
            // cosine is pi/2 minus the shared inverse-sine kernel.
            let source = Scalar::from(value);
            return match source.checked_asin() {
                Ok(sine) => Self::numeric(std::f64::consts::FRAC_PI_2
                    - sine.as_f64().expect("bound inverse sine preserves Float64")),
                Err(_) => Outcome::Uncomputed(Unevaluated::NumericPolicy),
            };
        }
        // The native SIN, COS and TAN domain ends at |angle| = 2^27.
        if matches!(function, Function::Cos | Function::Sin | Function::Tan)
            && value.abs() >= 134_217_728.0 {
            return Outcome::Computed(Operand::Error(ExcelError::Num));
        }
        if (function == Function::Sin
                && (value.is_subnormal() || value.abs() > std::f64::consts::FRAC_PI_2))
            || (function == Function::Tan
                && (value.is_subnormal() || value.abs() > std::f64::consts::FRAC_PI_4)) {
            return Outcome::Uncomputed(Unevaluated::NumericPolicy);
        }
        // Native large-angle reduction differs from the shared libm kernel.
        // Until that reduction is reproduced, retain its cache outside the
        // principal-angle domain verified by the native fixture.
        if function == Function::Cos && value.abs() > std::f64::consts::PI {
            return Outcome::Uncomputed(Unevaluated::NumericPolicy);
        }
        let source = Scalar::from(value);
        let result = match function {
            Function::Exp => source.checked_exp(),
            Function::Ln => source.checked_ln(),
            Function::Log10 => source.checked_log10(),
            Function::Degrees => source.checked_degrees(),
            Function::Radians => source.checked_radians(),
            Function::Cos => source.checked_cos(),
            Function::Asin => source.checked_asin(),
            Function::Sin => source.checked_sin(),
            Function::Tan => source.checked_tan(),
            Function::Atan => source.checked_atan(),
            _ => unreachable!("the ready call was matched as pure math"),
        };
        match result {
            Ok(value) => Self::numeric(value.as_f64().expect("bound pure math preserves Float64")),
            Err(_) => Outcome::Uncomputed(Unevaluated::NumericPolicy),
        }
    }

    fn atan2(x: Outcome, y: Outcome, system: DateSystem) -> Outcome {
        let x = match x.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let x = match Self::coerce_number(x, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        let y = match y.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let y = match Self::coerce_number(y, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        if x.is_subnormal() || y.is_subnormal() {
            return Outcome::Uncomputed(Unevaluated::NumericPolicy);
        }
        if x == 0.0 && y == 0.0 {
            return Outcome::Computed(Operand::Error(ExcelError::Div0));
        }
        // Excel takes x first and y second; the generic kernel takes y,x.
        match Scalar::from(y).checked_atan2(&Scalar::from(x)) {
            Ok(value) => Self::numeric(value.as_f64().expect("bound coordinates preserve Float64")),
            Err(_) => Outcome::Uncomputed(Unevaluated::NumericPolicy),
        }
    }

    fn factorial(value: Outcome, system: DateSystem) -> Outcome {
        let value = match value.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let value = match Self::coerce_number(value, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        if !value.is_finite() {
            return Outcome::Uncomputed(Unevaluated::NumericPolicy);
        }
        // Excel checks the sign before truncation, then restricts the result
        // domain. The shared kernel owns the descending Float64 product.
        if value < 0.0 || value.trunc() > 170.0 {
            return Outcome::Computed(Operand::Error(ExcelError::Num));
        }
        match Scalar::from(value.trunc()).checked_factorial() {
            Ok(result) => Self::numeric(
                result.as_f64().expect("bound factorial preserves Float64"),
            ),
            Err(_) => Outcome::Uncomputed(Unevaluated::NumericPolicy),
        }
    }

    fn square_root(value: Outcome, system: DateSystem) -> Outcome {
        let value = match value.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let value = match Self::coerce_number(value, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        if value < 0.0 {
            return Outcome::Computed(Operand::Error(ExcelError::Num));
        }
        match Scalar::from(value).checked_sqrt() {
            Ok(result) => Self::numeric(
                result
                    .as_f64()
                    .expect("the shared root preserves a float64 operand"),
            ),
            Err(_) => Outcome::Uncomputed(Unevaluated::NumericPolicy),
        }
    }

    fn signed_integer(function: Function, value: Outcome, system: DateSystem) -> Outcome {
        let value = match value.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let value = match Self::coerce_number(value, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        match function {
            Function::Sign => match Scalar::from(value).checked_sign() {
                Ok(value) => Self::numeric(value.as_f64().expect("bound sign preserves float64")),
                Err(_) => Outcome::Uncomputed(Unevaluated::NumericPolicy),
            },
            Function::Int => match number::integer_floor(value) {
                Some(Ok(value)) => Outcome::Computed(Operand::Number(value)),
                Some(Err(error)) => Outcome::Computed(Operand::Error(error)),
                None => Outcome::Uncomputed(Unevaluated::NumericPolicy),
            },
            _ => unreachable!("only SIGN/INT use this bound kernel"),
        }
    }

    fn logical_not(value: Outcome) -> Outcome {
        let operand = match value.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let reference = matches!(&operand, Operand::Reference(_));
        match operand.logical() {
            Some(Ok(value)) => Outcome::Computed(Operand::Boolean(!value)),
            Some(Err(error)) => Outcome::Computed(Operand::Error(error)),
            None if reference => Outcome::Uncomputed(Unevaluated::Reference),
            None => Outcome::Uncomputed(Unevaluated::Coercion),
        }
    }

    fn information(function: Function, value: Outcome, system: DateSystem) -> Outcome {
        let operand = match value.operand() {
            Ok(operand) => operand,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        if matches!(&operand, Operand::Reference(_)) {
            return Outcome::Uncomputed(Unevaluated::Reference);
        }
        match function {
            Function::Isblank => Outcome::Computed(Operand::Boolean(matches!(operand, Operand::Blank))),
            Function::Islogical =>
                Outcome::Computed(Operand::Boolean(matches!(operand, Operand::Boolean(_)))),
            Function::Isnontext =>
                Outcome::Computed(Operand::Boolean(!matches!(operand, Operand::Text(_)))),
            Function::Isnumber =>
                Outcome::Computed(Operand::Boolean(matches!(operand, Operand::Number(_)))),
            Function::Istext =>
                Outcome::Computed(Operand::Boolean(matches!(operand, Operand::Text(_)))),
            Function::Iserr => Outcome::Computed(Operand::Boolean(
                matches!(operand, Operand::Error(error) if error != ExcelError::NA),
            )),
            Function::Iserror =>
                Outcome::Computed(Operand::Boolean(matches!(operand, Operand::Error(_)))),
            Function::Isna => Outcome::Computed(Operand::Boolean(
                matches!(operand, Operand::Error(ExcelError::NA)),
            )),
            Function::ErrorDotType => match operand {
                Operand::Error(error) => {
                    let code = match error {
                        ExcelError::Null => Some(1.0),
                        ExcelError::Div0 => Some(2.0),
                        ExcelError::Value => Some(3.0),
                        ExcelError::Ref => Some(4.0),
                        ExcelError::Name => Some(5.0),
                        ExcelError::Num => Some(6.0),
                        ExcelError::NA => Some(7.0),
                        ExcelError::GettingData => Some(8.0),
                        _ => None,
                    };
                    match code {
                        Some(code) => Outcome::Computed(Operand::Number(code)),
                        None => Outcome::Uncomputed(Unevaluated::Function(function)),
                    }
                }
                _ => Outcome::Computed(Operand::Error(ExcelError::NA)),
            },
            Function::Iseven | Function::Isodd => match operand.integer_number(system) {
                Some(Ok(value)) => match number::odd(value) {
                    Some(Ok(odd)) => Outcome::Computed(Operand::Boolean(
                        if function == Function::Isodd { odd } else { !odd },
                    )),
                    Some(Err(error)) => Outcome::Computed(Operand::Error(error)),
                    None => Outcome::Uncomputed(Unevaluated::NumericPolicy),
                },
                Some(Err(error)) => Outcome::Computed(Operand::Error(error)),
                None => Outcome::Uncomputed(Unevaluated::Reference),
            },
            Function::N => match operand {
                Operand::Number(value) => Self::numeric(value),
                Operand::Boolean(value) => Self::numeric(if value { 1.0 } else { 0.0 }),
                Operand::Blank | Operand::Text(_) => Self::numeric(0.0),
                Operand::Error(error) => Outcome::Computed(Operand::Error(error)),
                Operand::Reference(_) => unreachable!("resolved above"),
            },
            _ => unreachable!("the ready call matched an information function"),
        }
    }

    /// The node policy owns omitted-slot eligibility. Reducers traverse every
    /// admitted argument so a later error or held source remains visible.
    fn visit_arguments<'w>(
        &mut self,
        base: usize,
        args: &[Option<usize>],
        read: RangeRead,
        context: &mut impl Context<'w>,
        mut visit: impl FnMut(Outcome, ArgumentSource, u64),
    ) -> crate::Result<()> {
        for argument in args {
            let value = match argument {
                Some(argument) => self.take(base, *argument),
                None => Outcome::Computed(Operand::Blank),
            };
            match value {
                Outcome::Computed(Operand::Reference(id)) => {
                    // The dense text consumers accept one rectangle; a union
                    // contributes an argument error in its original order.
                    if read == RangeRead::ValuesWithBlanks && context.reference_is_union(id) {
                        visit(Outcome::Computed(Operand::Error(ExcelError::Value)), ArgumentSource::Reference, 1);
                        continue;
                    }
                    context.visit_range(id, read, 0, |value, count| {
                        visit(value, ArgumentSource::Reference, count);
                        std::ops::ControlFlow::Continue(())
                    })?;
                }
                Outcome::Intersection { value, referenced } => {
                    // SINGLE retains one narrowed reference across names and
                    // consumers, never the full range or an early scalar cache.
                    let value = match value {
                        value @ Operand::Reference(_) => context.scalar(value)?,
                        value => Outcome::Computed(value),
                    };
                    visit(value, if referenced { ArgumentSource::Reference } else { ArgumentSource::Direct }, 1);
                }
                Outcome::Array { array, .. } => {
                    let (rows, columns) = self.array_shape(array, context);
                    for row in 0..rows {
                        for column in 0..columns {
                            visit(self.array_element(array, row, column, context), ArgumentSource::Array, 1);
                        }
                    }
                }
                value => visit(value, ArgumentSource::Direct, 1),
            }
        }
        Ok(())
    }

    fn npv<'w>(
        &mut self,
        base: usize,
        args: &[Option<usize>],
        context: &mut impl Context<'w>,
    ) -> crate::Result<Outcome> {
        let system = context.system();
        let rate = match args[0] {
            Some(id) => Self::scalar(self.take(base, id), context)?,
            None => Outcome::Computed(Operand::Blank),
        };
        let rate = match rate.operand() {
            Ok(rate) => rate,
            Err(reason) => return Ok(Outcome::Uncomputed(reason)),
        };
        let mut explicit_error = None;
        let rate = match rate.number(system) {
            Some(Ok(rate)) => rate,
            Some(Err(error)) => { explicit_error = Some(error); 0.0 },
            None => return Ok(Outcome::Uncomputed(Unevaluated::Coercion)),
        };
        let mut discounted = Accumulator::discounted(rate);
        let mut numeric_error = None;
        let mut unresolved = None;
        self.visit_arguments(base, &args[1..], RangeRead::Numbers, context, |outcome, source, count| {
            debug_assert_eq!(count, 1, "NPV uses sparse numeric range reads");
            let value = match outcome.operand() {
                Err(reason) => { unresolved.get_or_insert(reason); return; }
                Ok(Operand::Blank | Operand::Boolean(_) | Operand::Text(_)) if source != ArgumentSource::Direct => return,
                Ok(value) => value,
            };
            match value.number(system) {
                Some(Ok(value)) => {
                    if let Err(error) = discounted.push_discounted(value) { numeric_error.get_or_insert(error); }
                }
                Some(Err(error)) => { explicit_error.get_or_insert(error); }
                None => { unresolved.get_or_insert(Unevaluated::Coercion); }
            }
        })?;
        if let Some(reason) = unresolved { return Ok(Outcome::Uncomputed(reason)); }
        if let Some(error) = explicit_error.or(numeric_error) {
            return Ok(Outcome::Computed(Operand::Error(error)));
        }
        Ok(match discounted.finish_discounted() {
            Ok(Some(value)) => Outcome::Computed(Operand::Number(value)),
            Ok(None) => Outcome::Uncomputed(Unevaluated::NumericPolicy),
            Err(error) => Outcome::Computed(Operand::Error(error)),
        })
    }
    fn join<'w>(
        &mut self,
        function: Function,
        base: usize,
        args: &[Option<usize>],
        context: &mut impl Context<'w>,
    ) -> crate::Result<Outcome> {
        let (delimiter, ignore, args) = if function == Function::Textjoin {
            let delimiter = Self::scalar(self.take(base, args[0].expect("strict policy proved delimiter")), context)?;
            let ignore = Self::scalar(self.take(base, args[1].expect("strict policy proved ignore_empty")), context)?;
            (delimiter, ignore, &args[2..])
        } else {
            (Outcome::Computed(Operand::Blank), Outcome::Computed(Operand::Boolean(true)), args)
        };
        let mut output = super::text::Join::new(delimiter, ignore);
        if function == Function::Concatenate {
            for argument in args {
                let value = self.take(base, argument.expect("strict policy proved argument"));
                let value = self.project(value, context)?;
                output.push(value, 1);
            }
        } else {
            self.visit_arguments(base, args, RangeRead::ValuesWithBlanks, context,
                |outcome, _, count| output.push(outcome, count))?;
        }
        Ok(output.finish())
    }

    fn logical<'w>(
        &mut self,
        function: Function,
        base: usize,
        args: &[Option<usize>],
        context: &mut impl Context<'w>,
    ) -> crate::Result<Outcome> {
        let mut accumulator = LogicalAccumulator::default();
        let mut explicit_error = None;
        let mut unresolved = None;
        self.visit_arguments(base, args, RangeRead::Logical, context, |outcome, source, count| {
            debug_assert_eq!(count, 1, "Logical is a sparse range mode");
            match outcome.operand() {
                Err(reason) => { unresolved.get_or_insert(reason); }
                Ok(Operand::Blank | Operand::Text(_)) if source != ArgumentSource::Direct => {}
                Ok(value) => match value.logical() {
                    Some(Ok(value)) => accumulator.push(value),
                    Some(Err(error)) => { explicit_error.get_or_insert(error); }
                    None => { unresolved.get_or_insert(Unevaluated::Function(function)); }
                },
            }
        })?;
        if let Some(reason) = unresolved {
            return Ok(Outcome::Uncomputed(reason));
        }
        if let Some(error) = explicit_error {
            return Ok(Outcome::Computed(Operand::Error(error)));
        }
        Ok(Outcome::Computed(match accumulator.finish(function) {
            Ok(value) => Operand::Boolean(value),
            Err(error) => Operand::Error(error),
        }))
    }

    fn order_statistic<'w>(
        &mut self,
        function: Function,
        base: usize,
        args: &[Option<usize>],
        context: &mut impl Context<'w>,
    ) -> crate::Result<Outcome> {
        let system = context.system();
        let (sources, argument) = if matches!(function, Function::Rank | Function::RankDotEq) {
            let target = Self::scalar(self.take(base, args[0].expect("typed RANK target")), context)?;
            let target = match target.operand() {
                Ok(value) => match Self::coerce_number(value, system) {
                    Ok(value) => value,
                    Err(outcome) => return Ok(outcome),
                },
                Err(reason) => return Ok(Outcome::Uncomputed(reason)),
            };
            let ascending = if args.len() == 3 {
                let order = Self::scalar(self.take(base, args[2].expect("typed RANK order")), context)?;
                match order.operand() {
                    Ok(value) => match Self::coerce_number(value, system) {
                        Ok(value) => value != 0.0,
                        Err(outcome) => return Ok(outcome),
                    },
                    Err(reason) => return Ok(Outcome::Uncomputed(reason)),
                }
            } else { false };
            (&args[1..2], RankArgument::Rank { target, ascending })
        } else {
            let k = Self::scalar(self.take(base, args[1].expect("typed rank coordinate")), context)?;
            let k = match k.operand() {
                Ok(value) => match Self::coerce_number(value, system) {
                    Ok(value) => value,
                    Err(outcome) => return Ok(outcome),
                },
                Err(reason) => return Ok(Outcome::Uncomputed(reason)),
            };
            (&args[..1], RankArgument::K(k))
        };
        self.aggregate(function, base, sources, AggregateMode::Ranked(argument), context)
    }

    /// Fold each aligned product without row materialization. Scalar calls
    /// occupy one logical position; a scalar mixed with a larger rectangle
    /// has incompatible shape, as in the native cache.
    fn product_factor(
        product: &mut f64,
        value: Operand,
        uncertain: &mut bool,
        explicit_error: &mut Option<ExcelError>,
        numeric_error: &mut Option<ExcelError>,
    ) {
        match value.sumproduct_factor() {
            Ok(value) => {
                *uncertain |= value.is_subnormal();
                let next = Arithmetic::Mul.apply_float(*product, value);
                *uncertain |= next.is_subnormal();
                if next.is_finite() { *product = next; }
                else { numeric_error.get_or_insert(ExcelError::Num); }
            }
            Err(error) => { explicit_error.get_or_insert(error); }
        }
    }

    fn sum_product<'w>(
        &mut self,
        base: usize,
        args: &[Option<usize>],
        context: &mut impl Context<'w>,
    ) -> crate::Result<Outcome> {
        let mut sources = Vec::with_capacity(args.len());
        let mut shape = None;
        let mut mismatch = false;
        let mut direct_error = None;
        for child in args {
            let outcome = self.take(base, child.expect("typed SUMPRODUCT argument"));
            let size = match &outcome {
                Outcome::Computed(Operand::Reference(id)) => {
                    if context.reference_is_union(*id) {
                        return Ok(Outcome::Computed(Operand::Error(ExcelError::Value)));
                    }
                    let Some(area) = context.reference_geometry(*id) else {
                        return Ok(Outcome::Uncomputed(Unevaluated::Reference));
                    };
                    (area.row_size() as usize, area.column_size() as usize)
                }
                Outcome::Array { array, .. } => self.array_shape(*array, context),
                Outcome::Uncomputed(reason) => return Ok(Outcome::Uncomputed(*reason)),
                Outcome::Computed(Operand::Error(error))
                | Outcome::Intersection { value: Operand::Error(error), .. } => {
                    direct_error.get_or_insert(*error); (1, 1)
                }
                _ => (1, 1),
            };
            if shape.is_some_and(|prior| prior != size) { mismatch = true; }
            shape.get_or_insert(size);
            sources.push(outcome);
        }
        if mismatch && direct_error.is_some() {
            return Ok(Outcome::Uncomputed(Unevaluated::NumericPolicy));
        }
        if mismatch { return Ok(Outcome::Computed(Operand::Error(ExcelError::Value))); }
        if let Some(error) = direct_error { return Ok(Outcome::Computed(Operand::Error(error))); }

        let (rows, columns) = shape.expect("typed SUMPRODUCT has at least one argument");
        let count = rows as u64 * columns as u64;
        let mut total = Accumulator::default();
        let mut explicit_error = None;
        let mut numeric_error = None;
        let mut uncertain = false;
        let mut cursor = 0;
        while cursor < count {
            let mut span = count - cursor;
            let mut product = 1.0;
            for source in &sources {
                let value = match source {
                    Outcome::Computed(Operand::Reference(id)) => {
                        let mut observed = None;
                        let progress = context.visit_range(*id, RangeRead::NumericDense, cursor, |value, run| {
                            observed = Some((value, run));
                            std::ops::ControlFlow::Break(())
                        })?;
                        if matches!(progress, RangeProgress::Paused(_)) {
                            return Ok(Outcome::Uncomputed(Unevaluated::Reference));
                        }
                        let (value, run) = observed.expect("dense sparse visitor returns an aligned run");
                        span = span.min(run);
                        value
                    }
                    Outcome::Array { array, .. } => {
                        span = 1;
                        self.array_element(*array, cursor as usize / columns, cursor as usize % columns, context)
                    }
                    value => { span = 1; Self::scalar(value.clone(), context)? }
                };
                match value.operand() {
                    Ok(value) => Self::product_factor(&mut product, value, &mut uncertain,
                        &mut explicit_error, &mut numeric_error),
                    Err(reason) => return Ok(Outcome::Uncomputed(reason)),
                }
            }
            if explicit_error.is_none() && numeric_error.is_none() {
                let admitted = if product == 0.0 && span > 1 {
                    total.push_zero_n(span)
                } else {
                    debug_assert_eq!(span, 1, "nonzero values are individual source cells");
                    total.push_number(product)
                };
                if let Err(error) = admitted { numeric_error.get_or_insert(error); }
            }
            cursor += span;
        }
        if uncertain { return Ok(Outcome::Uncomputed(Unevaluated::NumericPolicy)); }
        if let Some(error) = explicit_error { return Ok(Outcome::Computed(Operand::Error(error))); }
        if let Some(error) = numeric_error { return Ok(Outcome::Computed(Operand::Error(error))); }
        Ok(match total.finish_sum() {
            Ok(Some(value)) => Outcome::Computed(Operand::Number(value)),
            Ok(None) => Outcome::Uncomputed(Unevaluated::NumericPolicy),
            Err(error) => Outcome::Computed(Operand::Error(error)),
        })
    }

    /// Read one aligned ordinal run from the existing context reader. The
    /// callback can stop after a failed predicate without admitting later
    /// source dependencies; all consulted run lengths bound the next cursor.
    fn aligned_run<'w>(
        context: &mut impl Context<'w>,
        ranges: &[ReferenceId],
        cursor: u64,
        read: RangeRead,
        mut visit: impl FnMut(usize, Outcome) -> bool,
    ) -> crate::Result<Option<(u64, bool)>> {
        let mut span = u64::MAX;
        for (index, id) in ranges.iter().enumerate() {
            let mut observed = None;
            let progress = context.visit_range(*id, read, cursor, |value, run| {
                observed = Some((value, run));
                std::ops::ControlFlow::Break(())
            })?;
            if matches!(progress, RangeProgress::Paused(_)) { return Ok(None); }
            let (value, run) = observed.expect("dense sparse visitor returns a source run");
            span = span.min(run);
            if !visit(index, value) { return Ok(Some((span, false))); }
        }
        Ok(Some((span, true)))
    }

    /// Aligned criteria read the same sparse descriptor owner as ordinary
    /// ranges. Each predicate and its wildcard transition state is compiled
    /// once; an absent-cell run advances without materializing rows.
    fn criteria<'w>(
        &mut self,
        function: Function,
        base: usize,
        args: &[Option<usize>],
        context: &mut impl Context<'w>,
    ) -> crate::Result<Outcome> {
        let held = || Outcome::Uncomputed(Unevaluated::Function(function));
        let (pairs, result_arg) = match function {
            Function::Countif | Function::Countifs => (args, None),
            Function::Sumif | Function::Averageif => (&args[..2], args.get(2).copied().flatten()),
            Function::Sumifs | Function::Averageifs
            | Function::Maxifs | Function::Minifs => (&args[1..], args[0]),
            _ => unreachable!("criteria call owns six functions"),
        };
        let mut ranges = Vec::with_capacity(pairs.len() / 2);
        let mut criteria = Vec::with_capacity(pairs.len() / 2);
        for pair in pairs.chunks_exact(2) {
            let range = self.take(base, pair[0].expect("strict policy proved range"));
            let range = match range.operand() {
                Ok(Operand::Reference(id)) => id,
                Ok(Operand::Error(error)) => return Ok(Outcome::Computed(Operand::Error(error))),
                Err(reason) => return Ok(Outcome::Uncomputed(reason)),
                _ => return Ok(held()),
            };
            if context.reference_is_union(range) {
                return Ok(Outcome::Computed(Operand::Error(ExcelError::Value)));
            }
            let value = Self::scalar(self.take(base,
                pair[1].expect("strict policy proved criterion")), context)?;
            let value = match value.operand() {
                Ok(value) => value,
                Err(reason) => return Ok(Outcome::Uncomputed(reason)),
            };
            let Some(criterion) = Criterion::new(value) else { return Ok(held()); };
            ranges.push(range);
            criteria.push(criterion);
        }
        let Some(first) = context.reference_geometry(ranges[0]) else { return Ok(held()); };
        let count = first.cell_count();
        let legacy = matches!(function, Function::Countif | Function::Sumif | Function::Averageif);
        for range in &ranges[1..] {
            let Some(shape) = context.reference_geometry(*range) else { return Ok(held()); };
            if shape.row_size() != first.row_size() || shape.column_size() != first.column_size() {
                return Ok(Outcome::Computed(Operand::Error(ExcelError::Value)));
            }
        }
        let result = if let Some(child) = result_arg {
            let result = self.take(base, child);
            let result = match result.operand() {
                Ok(Operand::Reference(id)) => id,
                Ok(Operand::Error(error)) => return Ok(Outcome::Computed(Operand::Error(error))),
                Err(reason) => return Ok(Outcome::Uncomputed(reason)),
                _ => return Ok(held()),
            };
            if context.reference_is_union(result) {
                return Ok(Outcome::Computed(Operand::Error(ExcelError::Value)));
            }
            let Some(shape) = context.reference_geometry(result) else { return Ok(held()); };
            if legacy {
                let start = shape.start();
                let end_row = start.row().checked_add(first.row_size() - 1)
                    .filter(|row| *row < crate::excel::cell::MAX_ROWS);
                let end_col = start.column().checked_add(first.column_size() - 1)
                    .filter(|column| *column < crate::excel::cell::MAX_COLUMNS);
                let (Some(row), Some(column)) = (end_row, end_col) else { return Ok(held()); };
                let rectangle = CellRange::new(start, CellRef::new(row, column));
                match context.reference_range(result, rectangle).operand() {
                    Ok(Operand::Reference(id)) => Some(id),
                    _ => return Ok(held()),
                }
            } else {
                if shape.row_size() != first.row_size() || shape.column_size() != first.column_size() {
                    return Ok(Outcome::Computed(Operand::Error(ExcelError::Value)));
                }
                Some(result)
            }
        } else if matches!(function, Function::Sumif | Function::Averageif) {
            Some(ranges[0])
        } else {
            None
        };
        let mut matchers: Vec<_> = criteria.iter().map(Criterion::matcher).collect();
        let mut accumulator = Accumulator::default();
        let mut explicit_error = None;
        let mut aggregate_error = None;
        let mut numeric_policy = false;
        let mut cursor = 0;
        while cursor < count {
            let mut stopped = None;
            let Some((mut span, matched)) = Self::aligned_run(context, &ranges, cursor,
                RangeRead::ValuesWithBlanks, |index, outcome| {
                    let value = match outcome.operand() {
                        Ok(value) => value,
                        Err(reason) => {
                            stopped = Some(Outcome::Uncomputed(reason));
                            return false;
                        }
                    };
                    match matchers[index].matches(&value) {
                        Some(result) => result,
                        None => { stopped = Some(held()); false }
                    }
                })? else {
                    return Ok(Outcome::Uncomputed(Unevaluated::Reference));
                };
            if let Some(outcome) = stopped { return Ok(outcome); }
            if matched {
                if result.is_none() {
                    if let Err(error) = accumulator.push_count_n(span) {
                        aggregate_error.get_or_insert(error);
                    }
                } else {
                    let mut observed = None;
                    let progress = context.visit_range(result.expect("result checked"),
                        RangeRead::NumericDense, cursor, |value, run| {
                            observed = Some((value, run));
                            std::ops::ControlFlow::Break(())
                        })?;
                    if matches!(progress, RangeProgress::Paused(_)) {
                        return Ok(Outcome::Uncomputed(Unevaluated::Reference));
                    }
                    let (value, run) = observed.expect("dense sparse visitor returns a result run");
                    span = span.min(run);
                    match value.operand() {
                        Err(reason) => return Ok(Outcome::Uncomputed(reason)),
                        Ok(Operand::Number(number)) => {
                            debug_assert_eq!(span, 1, "stored numeric result is one position");
                            if matches!(function, Function::Maxifs | Function::Minifs)
                                && number.is_subnormal() {
                                numeric_policy = true;
                            } else if let Err(error) = accumulator.push_number(number) {
                                aggregate_error.get_or_insert(error);
                            }
                        }
                        Ok(Operand::Error(error)) => { explicit_error.get_or_insert(error); }
                        Ok(Operand::Reference(_)) => return Ok(held()),
                        Ok(Operand::Blank | Operand::Text(_) | Operand::Boolean(_)) => {}
                    }
                }
            }
            cursor += span;
        }
        if let Some(error) = explicit_error {
            return Ok(Outcome::Computed(Operand::Error(error)));
        }
        if let Some(error) = aggregate_error {
            return Ok(Outcome::Computed(Operand::Error(error)));
        }
        if numeric_policy { return Ok(Outcome::Uncomputed(Unevaluated::NumericPolicy)); }
        let result = match function {
            Function::Countif | Function::Countifs => Ok(accumulator.finish_counta()),
            Function::Sumif | Function::Sumifs => accumulator.finish_sum(),
            Function::Averageif | Function::Averageifs => accumulator.finish_average(),
            Function::Maxifs => Ok(Some(accumulator.finish_max())),
            Function::Minifs => Ok(Some(accumulator.finish_min())),
            _ => unreachable!("criteria call owns six functions"),
        };
        Ok(match result {
            Ok(Some(value)) => Outcome::Computed(Operand::Number(value)),
            Ok(None) => Outcome::Uncomputed(Unevaluated::NumericPolicy),
            Err(error) => Outcome::Computed(Operand::Error(error)),
        })
    }

    /// SUBTOTAL only consumes references; its code selects an existing
    /// aggregate mode, while the range reader suppresses nested subtotals
    /// and the hidden rows excluded by 100-series codes.
    fn subtotal<'w>(
        &mut self,
        base: usize,
        args: &[Option<usize>],
        context: &mut impl Context<'w>,
    ) -> crate::Result<Option<Outcome>> {
        // The code survives a suspended source preflight. Its static Scalar
        // dependency was already ready before this call started.
        let code_id = args[0].expect("strict policy proved the subtotal code");
        let code = Self::scalar(self.values[base + code_id].as_ref()
            .expect("the strict child was evaluated").clone(), context)?;
        let code = match code.operand() {
            Err(reason) => return Ok(Some(Outcome::Uncomputed(reason))),
            Ok(Operand::Error(error)) => return Ok(Some(Outcome::Computed(Operand::Error(error)))),
            Ok(Operand::Number(value)) if value.is_finite() && value.fract() == 0.0 => value,
            _ => return Ok(Some(Outcome::Uncomputed(Unevaluated::Function(Function::Subtotal)))),
        };
        let (function, exclude_hidden) = match code as i32 {
            1 | 101 => (Function::Average, code >= 100.0),
            2 | 102 => (Function::Count, code >= 100.0),
            3 | 103 => (Function::Counta, code >= 100.0),
            4 | 104 => (Function::Max, code >= 100.0),
            5 | 105 => (Function::Min, code >= 100.0),
            6 | 106 => (Function::Product, code >= 100.0),
            9 | 109 => (Function::Sum, code >= 100.0),
            7 | 107 => (Function::Stdev, code >= 100.0),
            8 | 108 => (Function::Stdevp, code >= 100.0),
            10 | 110 => (Function::Var, code >= 100.0),
            11 | 111 => (Function::Varp, code >= 100.0),
            _ => return Ok(Some(Outcome::Computed(Operand::Error(ExcelError::Value)))),
        };
        // Direct scalar ref arguments could not be entered in the guarded
        // source workbook; no speculative scalar admission here.
        for child in &args[1..] {
            let child = child.expect("strict policy proved subtotal references");
            let Some(Outcome::Computed(Operand::Reference(id))) = self.values[base + child].as_ref()
                else { return Ok(Some(Outcome::Uncomputed(Unevaluated::Function(Function::Subtotal)))); };
            if !context.reference_is_union(*id) && context.reference_geometry(*id).is_none() {
                return Ok(Some(Outcome::Computed(Operand::Error(ExcelError::Value))));
            }
        }
        let mut ready = true;
        for child in &args[1..] {
            let child = child.expect("strict policy proved subtotal references");
            let Some(Outcome::Computed(Operand::Reference(id))) = self.values[base + child].as_ref()
                else { unreachable!("the reference policy was proved above") };
            // Every source is registered, even when an earlier one pauses:
            // no range is partially published and every future change is watched.
            ready &= context.ready_subtotal(*id, exclude_hidden)?;
        }
        if !ready { return Ok(None); }
        self.values[base + code_id].take();
        self.aggregate(function, base, &args[1..], AggregateMode::Filtered(
            RangeRead::Subtotal { exclude_hidden }), context).map(Some)
    }

    /// Count absent cells and formula-empty text without materializing
    /// a worksheet-sized range or cloning present source text.
    fn count_blank<'w>(
        &mut self,
        base: usize,
        args: &[Option<usize>],
        context: &mut impl Context<'w>,
    ) -> crate::Result<Outcome> {
        let source = self.take(base, args[0].expect("typed COUNTBLANK source"));
        let id = match source.operand() {
            Ok(Operand::Reference(id)) if context.reference_is_union(id) =>
                return Ok(Outcome::Computed(Operand::Error(ExcelError::Value))),
            Ok(Operand::Reference(id)) if context.reference_geometry(id).is_some() => id,
            Ok(Operand::Error(error)) => return Ok(Outcome::Computed(Operand::Error(error))),
            Err(reason) => return Ok(Outcome::Uncomputed(reason)),
            _ => return Ok(Outcome::Uncomputed(Unevaluated::Function(Function::Countblank))),
        };
        let mut count = Accumulator::default();
        let mut failed = None;
        let mut unresolved = None;
        let progress = context.visit_range(id, RangeRead::BlankPresence, 0, |value, run| {
            match value.operand() {
                Ok(Operand::Blank) => {
                    if let Err(error) = count.push_count_n(run) { failed.get_or_insert(error); }
                }
                Ok(Operand::Text(text)) if text.as_str().is_empty() => {
                    if let Err(error) = count.push_count_n(run) { failed.get_or_insert(error); }
                }
                Err(reason) => { unresolved.get_or_insert(reason); }
                _ => {}
            }
            std::ops::ControlFlow::Continue(())
        })?;
        if matches!(progress, RangeProgress::Paused(_)) {
            return Ok(Outcome::Uncomputed(Unevaluated::Reference));
        }
        if let Some(reason) = unresolved { return Ok(Outcome::Uncomputed(reason)); }
        if let Some(error) = failed { return Ok(Outcome::Computed(Operand::Error(error))); }
        Ok(match count.finish_counta() {
            Some(value) => Outcome::Computed(Operand::Number(value)),
            None => Outcome::Uncomputed(Unevaluated::NumericPolicy),
        })
    }

    fn aggregate<'w>(
        &mut self,
        function: Function,
        base: usize,
        args: &[Option<usize>],
        mode: AggregateMode,
        context: &mut impl Context<'w>,
    ) -> crate::Result<Outcome> {
        let (rank, read_override) = match mode {
            AggregateMode::Plain => (None, None),
            AggregateMode::Ranked(rank) => (Some(rank), None),
            AggregateMode::Filtered(read) => (None, Some(read)),
        };
        let system = context.system();
        let mut accumulator = if rank.is_some() || matches!(function,
            Function::Median | Function::Mode | Function::ModeDotSngl) {
            Accumulator::ranked()
        } else if matches!(function, Function::Var | Function::Varp | Function::VarDotS | Function::VarDotP
            | Function::Stdev | Function::Stdevp | Function::StdevDotS | Function::StdevDotP) {
            Accumulator::variance()
        } else {
            Accumulator::default()
        };
        let mut explicit_error = None;
        let mut aggregate_error = None;
        let mut unresolved = None;
        let mut feed = |outcome: Outcome, source: ArgumentSource| match outcome.operand() {
            Err(reason) => {
                unresolved.get_or_insert(reason);
            }
            Ok(Operand::Reference(_)) => {
                unresolved.get_or_insert(Unevaluated::Reference);
            }
            Ok(Operand::Blank) if source != ArgumentSource::Direct => {}
            Ok(_) if function == Function::Counta => {
                if let Err(error) = accumulator.push_present() {
                    aggregate_error.get_or_insert(error);
                }
            }
            Ok(Operand::Boolean(value)) if source == ArgumentSource::Reference && matches!(function,
                Function::Averagea | Function::Mina | Function::Maxa) => {
                if let Err(error) = accumulator.push_number(if value { 1.0 } else { 0.0 }) {
                    aggregate_error.get_or_insert(error);
                }
            }
            Ok(Operand::Text(_)) if (source == ArgumentSource::Reference && matches!(function,
                Function::Averagea | Function::Mina | Function::Maxa))
                || (source == ArgumentSource::Array && function == Function::Averagea) => {
                if let Err(error) = accumulator.push_number(0.0) {
                    aggregate_error.get_or_insert(error);
                }
            }
            Ok(Operand::Error(_)) if function == Function::Count => {}
            Ok(Operand::Error(error)) => {
                explicit_error.get_or_insert(error);
            }
            Ok(Operand::Blank | Operand::Text(_) | Operand::Boolean(_)) if source != ArgumentSource::Direct => {}
            Ok(Operand::Text(_) | Operand::Boolean(_)) if matches!(function,
                Function::Mode | Function::ModeDotSngl | Function::Large | Function::Small
                | Function::Percentile | Function::PercentileDotInc
                | Function::Quartile | Function::QuartileDotInc
                | Function::Rank | Function::RankDotEq) => {}
            Ok(value) => match value.number(system) {
                Some(Ok(value)) => {
                    if matches!(function, Function::Min | Function::Max | Function::Mina
                        | Function::Maxa | Function::Product | Function::Median
                        | Function::Mode | Function::ModeDotSngl | Function::Large
                        | Function::Small | Function::Percentile | Function::PercentileDotInc
                        | Function::Quartile | Function::QuartileDotInc
                        | Function::Rank | Function::RankDotEq) && value.is_subnormal() {
                        unresolved.get_or_insert(Unevaluated::NumericPolicy);
                    } else {
                        let admitted = if function == Function::Product {
                            accumulator.push_product(value)
                        } else {
                            accumulator.push_number(value)
                        };
                        if let Err(error) = admitted { aggregate_error.get_or_insert(error); }
                    }
                }
                Some(Err(_)) if function == Function::Count => {}
                Some(Err(error)) => {
                    explicit_error.get_or_insert(error);
                }
                None => {
                    unresolved.get_or_insert(Unevaluated::NumericPolicy);
                }
            },
        };
        let read = read_override.unwrap_or(match function {
            Function::Counta => RangeRead::Presence,
            Function::Averagea | Function::Mina | Function::Maxa => RangeRead::AggregateA,
            _ => RangeRead::Numbers,
        });
        self.visit_arguments(base, args, read, context, |outcome, source, count| {
            debug_assert_eq!(count, 1, "numeric aggregates use sparse range modes");
            feed(outcome, source);
        })?;
        if let Some(reason) = unresolved {
            return Ok(Outcome::Uncomputed(reason));
        }
        if let Some(error) = explicit_error {
            return Ok(Outcome::Computed(Operand::Error(error)));
        }
        if let Some(error) = aggregate_error {
            return Ok(Outcome::Computed(Operand::Error(error)));
        }
        let result = match function {
            Function::Count => Ok(accumulator.finish_count()),
            Function::Counta => Ok(accumulator.finish_counta()),
            Function::Min => Ok(Some(accumulator.finish_min())),
            Function::Max => Ok(Some(accumulator.finish_max())),
            Function::Sum => accumulator.finish_sum(),
            Function::Average | Function::Averagea => accumulator.finish_average(),
            Function::Mina => Ok(Some(accumulator.finish_min())),
            Function::Maxa => Ok(Some(accumulator.finish_max())),
            Function::Product => Ok(Some(accumulator.finish_product())),
            Function::Var | Function::Varp | Function::VarDotS | Function::VarDotP
            | Function::Stdev | Function::Stdevp | Function::StdevDotS | Function::StdevDotP =>
                accumulator.finish_variance(matches!(function, Function::Var | Function::VarDotS
                    | Function::Stdev | Function::StdevDotS)),
            Function::Median => accumulator.finish_median(),
            Function::Mode | Function::ModeDotSngl => accumulator.finish_mode(),
            Function::Large | Function::Small => {
                let Some(RankArgument::K(k)) = rank else { unreachable!("typed rank coordinate") };
                accumulator.finish_kth(k, function == Function::Large)
            }
            Function::Percentile | Function::PercentileDotInc
            | Function::Quartile | Function::QuartileDotInc => {
                let Some(RankArgument::K(k)) = rank else { unreachable!("typed percentile coordinate") };
                let k = if matches!(function, Function::Quartile | Function::QuartileDotInc) {
                    if !(0.0..=4.0).contains(&k) {
                        // Native integer out-of-range arguments are #NUM!;
                        // the domain/truncation order for fractional outside
                        // values has not been observed.
                        if k.fract() != 0.0 {
                            return Ok(Outcome::Uncomputed(Unevaluated::NumericPolicy));
                        }
                        return Ok(Outcome::Computed(Operand::Error(ExcelError::Num)));
                    }
                    k.trunc() / 4.0
                } else { k };
                accumulator.finish_percentile(k)
            }
            Function::Rank | Function::RankDotEq => {
                let Some(RankArgument::Rank { target, ascending }) = rank else {
                    unreachable!("typed RANK parameters")
                };
                accumulator.finish_rank(target, ascending)
            },
            _ => unreachable!("the node policy selects the numeric aggregate"),
        };
        Ok(match result {
            Ok(Some(value)) if matches!(function, Function::Stdev | Function::Stdevp
                | Function::StdevDotS | Function::StdevDotP) =>
                Self::square_root(Outcome::Computed(Operand::Number(value)), system),
            Ok(Some(value)) => Outcome::Computed(Operand::Number(value)),
            Ok(None) => Outcome::Uncomputed(Unevaluated::NumericPolicy),
            Err(error) => Outcome::Computed(Operand::Error(error)),
        })
    }

    /// Excel alone resolves source text/Boolean/blank arguments and the
    /// 15-digit numeric boundary. The integer pair operation stays Scalar-owned.
    fn integer_math<'w>(
        &mut self,
        function: Function,
        base: usize,
        args: &[Option<usize>],
        context: &mut impl Context<'w>,
    ) -> crate::Result<Outcome> {
        let system = context.system();
        let mut result: Option<Scalar> = None;
        let mut input_error = None;
        let mut domain_error = None;
        let mut pair_overflow = false;
        let mut unresolved = None;
        self.visit_arguments(base, args, RangeRead::Values, context, |outcome, source, count| {
            debug_assert_eq!(count, 1, "Values walks only stored cells");
            match outcome.operand() {
                Err(reason) => { unresolved.get_or_insert(reason); }
                Ok(Operand::Error(error)) => { input_error.get_or_insert(error); }
                Ok(Operand::Blank) if source != ArgumentSource::Direct => {}
                Ok(value) => {
                    let number = match value.integer_number(system) {
                        Some(Ok(number)) => number,
                        Some(Err(error)) => {
                            input_error.get_or_insert(error);
                            return;
                        }
                        None => {
                            unresolved.get_or_insert(Unevaluated::NumericPolicy);
                            return;
                        }
                    };
                    if !number.is_finite() || number < 0.0
                        || number > 9_007_199_254_740_992.0 {
                        domain_error.get_or_insert(ExcelError::Num);
                        return;
                    }
                    // The computed 2^53 boundary is accepted; larger
                    // integer arguments have Excel's #NUM! domain.
                    let next = Scalar::from(number.trunc() as u128);
                    if function == Function::Lcm && next.as_u128() == Some(0) {
                        // A valid zero clears only provisional pair overflow.
                        // Invalid inputs and later explicit errors stay visible.
                        pair_overflow = false;
                        result = Some(next);
                        return;
                    }
                    if pair_overflow {
                        return;
                    }
                    result = match result.take() {
                        None => Some(next),
                        Some(prior) => {
                            let combined = if function == Function::Gcd {
                                prior.checked_gcd(&next)
                            } else {
                                prior.checked_lcm(&next)
                            };
                            match combined {
                                Ok(value) => Some(value),
                                Err(_) => {
                                    // The pair kernel has one exact u64 result domain.
                                    // A later LCM zero can annihilate this interim value.
                                    pair_overflow = true;
                                    Some(prior)
                                }
                            }
                        }
                    };
                }
            }
        })?;
        if let Some(reason) = unresolved { return Ok(Outcome::Uncomputed(reason)); }
        if let Some(error) = input_error.or(domain_error) {
            return Ok(Outcome::Computed(Operand::Error(error)));
        }
        if pair_overflow {
            return Ok(Outcome::Computed(Operand::Error(ExcelError::Num)));
        }
        let answer = result.and_then(|value| value.as_u128()).unwrap_or(0);
        if answer > 9_007_199_254_740_992 {
            return Ok(Outcome::Computed(Operand::Error(ExcelError::Num)));
        }
        Ok(Outcome::Computed(Operand::Number(answer as f64)))
    }

    fn modulus(left: Outcome, right: Outcome, system: DateSystem) -> Outcome {
        let left = left.operand();
        let right = right.operand();
        let (left, right) = match (left, right) {
            (Err(reason), _) | (_, Err(reason)) => return Outcome::Uncomputed(reason),
            (Ok(Operand::Error(error)), _) | (_, Ok(Operand::Error(error))) => {
                return Outcome::Computed(Operand::Error(error));
            }
            (Ok(left), Ok(right)) => (left, right),
        };
        let left = match Self::coerce_number(left, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        let right = match Self::coerce_number(right, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        match number::modulus(left, right) {
            Some(Ok(value)) => Outcome::Computed(Operand::Number(value)),
            Some(Err(error)) => Outcome::Computed(Operand::Error(error)),
            None => Outcome::Uncomputed(Unevaluated::NumericPolicy),
        }
    }

    fn rounding_unary(function: Function, value: Outcome, system: DateSystem) -> Outcome {
        let value = match value.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let value = match Self::coerce_number(value, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        match number::parity_round(value, function == Function::Odd) {
            Some(Ok(value)) => Outcome::Computed(Operand::Number(value)),
            Some(Err(error)) => Outcome::Computed(Operand::Error(error)),
            None => Outcome::Uncomputed(Unevaluated::NumericPolicy),
        }
    }

    fn rounding_binary(
        function: Function, value: Outcome, second: Outcome, mode: Outcome, system: DateSystem,
    ) -> Outcome {
        let (value, second, mode) = match (value.operand(), second.operand(), mode.operand()) {
            (Err(reason), _, _) | (_, Err(reason), _) | (_, _, Err(reason)) =>
                return Outcome::Uncomputed(reason),
            (Ok(Operand::Error(error)), _, _) | (_, Ok(Operand::Error(error)), _)
            | (_, _, Ok(Operand::Error(error))) =>
                return Outcome::Computed(Operand::Error(error)),
            (Ok(value), Ok(second), Ok(mode)) => (value, second, mode),
        };
        if matches!(function, Function::Quotient | Function::Mround)
            && (matches!(&value, Operand::Boolean(_)) || matches!(&second, Operand::Boolean(_))) {
            return Outcome::Computed(Operand::Error(ExcelError::Value));
        }
        let value = match Self::coerce_number(value, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        let second = match Self::coerce_number(second, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        let result = match function {
            Function::Roundup | Function::Rounddown =>
                number::round_direction(value, second, function == Function::Roundup),
            Function::Quotient => number::quotient(value, second),
            Function::Ceiling | Function::Floor | Function::Mround
            | Function::CeilingDotMath | Function::FloorDotMath => {
                let mode = match Self::coerce_number(mode, system) {
                    Ok(value) => value,
                    Err(outcome) => return outcome,
                };
                number::multiple(function, value, second, mode)
            }
            _ => unreachable!("the rounding family was matched above"),
        };
        match result {
            Some(Ok(value)) => Outcome::Computed(Operand::Number(value)),
            Some(Err(error)) => Outcome::Computed(Operand::Error(error)),
            None => Outcome::Uncomputed(Unevaluated::NumericPolicy),
        }
    }

    fn truncate(value: Outcome, places: Outcome, system: DateSystem) -> Outcome {
        let (value, places) = match (value.operand(), places.operand()) {
            (Err(reason), _) | (_, Err(reason)) => return Outcome::Uncomputed(reason),
            (Ok(Operand::Error(error)), _) | (_, Ok(Operand::Error(error))) =>
                return Outcome::Computed(Operand::Error(error)),
            (Ok(value), Ok(places)) => (value, places),
        };
        let value = match Self::coerce_number(value, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        let places = match Self::coerce_number(places, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        match number::truncate(value, places) {
            Some(Ok(value)) => Outcome::Computed(Operand::Number(value)),
            Some(Err(error)) => Outcome::Computed(Operand::Error(error)),
            None => Outcome::Uncomputed(Unevaluated::NumericPolicy),
        }
    }

    fn round(value: Outcome, places: Outcome, system: DateSystem) -> Outcome {
        let value = value.operand();
        let places = places.operand();
        let (value, places) = match (value, places) {
            (Err(reason), _) | (_, Err(reason)) => return Outcome::Uncomputed(reason),
            (Ok(Operand::Error(error)), _) | (_, Ok(Operand::Error(error))) => {
                return Outcome::Computed(Operand::Error(error));
            }
            (Ok(value), Ok(places)) => (value, places),
        };
        let value = match Self::coerce_number(value, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        let places = match Self::coerce_number(places, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        match number::round(value, places) {
            Some(Ok(value)) => Outcome::Computed(Operand::Number(value)),
            Some(Err(error)) => Outcome::Computed(Operand::Error(error)),
            None => Outcome::Uncomputed(Unevaluated::NumericPolicy),
        }
    }

    fn percent(value: Outcome, system: DateSystem) -> Outcome {
        let value = match value.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        match Self::coerce_number(value, system) {
            Ok(value) => Self::numeric(Arithmetic::Div.apply_float(value, 100.0)),
            Err(outcome) => outcome,
        }
    }

    fn reference_binary<'w>(
        op: BinaryOp, left: Outcome, right: Outcome, context: &mut impl Context<'w>,
    ) -> Outcome {
        let left = match left.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let right = match right.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        match (left, right) {
            (Operand::Error(error), _) | (_, Operand::Error(error)) => Outcome::Computed(Operand::Error(error)),
            (Operand::Reference(left), Operand::Reference(right)) => context.reference_combine(op, left, right),
            _ => Outcome::Uncomputed(Unevaluated::Binary(op)),
        }
    }

    fn binary(
        op: BinaryOp,
        left: Outcome,
        right: Outcome,
        root: bool,
        system: DateSystem,
    ) -> Outcome {
        let left = match left.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        let right = match right.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        if let Operand::Error(error) = &left {
            return Outcome::Computed(Operand::Error(*error));
        }
        if let Operand::Error(error) = &right {
            return Outcome::Computed(Operand::Error(*error));
        }
        if op == BinaryOp::Concat {
            let mut output = super::text::Join::new(
                Outcome::Computed(Operand::Blank),
                Outcome::Computed(Operand::Boolean(true)),
            );
            output.push(Outcome::Computed(left), 1);
            output.push(Outcome::Computed(right), 1);
            return output.finish();
        }
        if let Some(comparison) = op.comparison() {
            return match left.order(&right) {
                Some(Ok(order)) => Outcome::Computed(Operand::Boolean(comparison.answers(order))),
                Some(Err(error)) => Outcome::Computed(Operand::Error(error)),
                None => Outcome::Uncomputed(Unevaluated::Coercion),
            };
        }
        let left = match Self::coerce_number(left, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        let right = match Self::coerce_number(right, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        match op {
            BinaryOp::Add | BinaryOp::Subtract => {
                let right = if op == BinaryOp::Subtract {
                    -right
                } else {
                    right
                };
                match number::add(left, right, root) {
                    Some(Ok(value)) => Outcome::Computed(Operand::Number(value)),
                    Some(Err(error)) => Outcome::Computed(Operand::Error(error)),
                    None => Outcome::Uncomputed(Unevaluated::NumericPolicy),
                }
            }
            BinaryOp::Multiply => Self::numeric(Arithmetic::Mul.apply_float(left, right)),
            BinaryOp::Divide if right == 0.0 => Outcome::Computed(Operand::Error(ExcelError::Div0)),
            BinaryOp::Divide => Self::numeric(Arithmetic::Div.apply_float(left, right)),
            BinaryOp::Power => Self::power(left, right),
            _ => Outcome::Uncomputed(Unevaluated::Binary(op)),
        }
    }

    fn annuity<'w>(
        &mut self,
        function: Function,
        args: &[Option<usize>],
        base: usize,
        context: &mut impl Context<'w>,
    ) -> crate::Result<Outcome> {
        let mut numbers = [0.0; 5];
        for (index, argument) in args.iter().enumerate() {
            let Some(argument) = argument else { continue; };
            let value = Self::scalar(self.take(base, *argument), context)?;
            let value = match value.operand() {
                Ok(value) => value,
                Err(reason) => return Ok(Outcome::Uncomputed(reason)),
            };
            match value.number(context.system()) {
                Some(Ok(value)) => numbers[index] = value,
                Some(Err(error)) => return Ok(Outcome::Computed(Operand::Error(error))),
                None => return Ok(Outcome::Uncomputed(Unevaluated::Coercion)),
            }
        }
        Ok(match number::annuity(function, numbers) {
            Some(Ok(value)) => Outcome::Computed(Operand::Number(value)),
            Some(Err(error)) => Outcome::Computed(Operand::Error(error)),
            None => Outcome::Uncomputed(Unevaluated::NumericPolicy),
        })
    }

    /// Apply the shared native kernel only where Excel's observed numeric
    /// domain is determined by the two values. Negative nonintegral powers
    /// remain held until exponent-expression semantics can be represented.
    fn power(base: f64, exponent: f64) -> Outcome {
        if base == 0.0 {
            if exponent == 0.0 {
                return Outcome::Computed(Operand::Error(ExcelError::Num));
            }
            if exponent < 0.0 {
                return Outcome::Computed(Operand::Error(ExcelError::Div0));
            }
        }
        if base < 0.0 && exponent.fract() != 0.0 {
            return Outcome::Uncomputed(Unevaluated::NumericPolicy);
        }
        match Scalar::from(base).checked_pow(&Scalar::from(exponent)) {
            Ok(value) => Self::numeric(value.as_f64().expect("the shared power preserves float64")),
            Err(_) => Outcome::Uncomputed(Unevaluated::NumericPolicy),
        }
    }

    fn randbetween<'w>(
        bottom: Outcome,
        top: Outcome,
        system: DateSystem,
        context: &mut impl Context<'w>,
    ) -> Outcome {
        let (bottom, top) = match (bottom.operand(), top.operand()) {
            (Err(reason), _) | (_, Err(reason)) => return Outcome::Uncomputed(reason),
            (Ok(Operand::Error(error)), _) | (_, Ok(Operand::Error(error))) => {
                return Outcome::Computed(Operand::Error(error));
            }
            (Ok(bottom), Ok(top)) => (bottom, top),
        };
        let bottom = match Self::coerce_number(bottom, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        let top = match Self::coerce_number(top, system) {
            Ok(value) => value,
            Err(outcome) => return outcome,
        };
        // Native singleton fractions are known, but the two-bound interval
        // policy has not been proved. Keep every fractional interval held.
        if !bottom.is_finite() || !top.is_finite()
            || bottom.fract() != 0.0 || top.fract() != 0.0
            || bottom.abs() > 9_007_199_254_740_991.0
            || top.abs() > 9_007_199_254_740_991.0
        {
            return Outcome::Uncomputed(Unevaluated::NumericPolicy);
        }
        if bottom > top {
            return Outcome::Computed(Operand::Error(ExcelError::Num));
        }
        let low = bottom as i64;
        let span = (top as i64 as i128 - i128::from(low) + 1) as u64;
        let threshold = span.wrapping_neg() % span;
        loop {
            let draw = context.random_u64();
            if draw >= threshold {
                return Self::numeric((i128::from(low) + i128::from(draw % span)) as f64);
            }
        }
    }

    /// ADDRESS produces text through the same A1 spelling owner as a
    /// reference. R1C1 letters depend on Excel's UI locale (native French
    /// uses L/C), which the workbook does not type, so that mode stays held.
    fn address<'w>(
        &mut self, args: &[Option<usize>], base: usize, context: &mut impl Context<'w>,
    ) -> crate::Result<Outcome> {
        let failure = |error| Outcome::Computed(Operand::Error(error));
        let mut numeric = [0.0, 0.0, 1.0];
        for (index, number) in numeric.iter_mut().enumerate().take(args.len().min(3)) {
            let child = args[index].expect("the strict policy proved ADDRESS arguments");
            let value = Self::scalar(self.take(base, child), context)?;
            let value = match value.operand() {
                Ok(value) => value,
                Err(reason) => return Ok(Outcome::Uncomputed(reason)),
            };
            *number = match Self::coerce_number(value, context.system()) {
                Ok(value) => value,
                Err(value) => return Ok(value),
            };
        }
        let [row, column, mode] = numeric;
        if !row.is_finite() || !column.is_finite() || !mode.is_finite()
            || row < 1.0 || row >= f64::from(MAX_ROWS + 1)
            || column < 1.0 || column >= f64::from(MAX_COLUMNS + 1)
            || mode < 1.0 || mode >= 5.0
        {
            return Ok(failure(ExcelError::Value));
        }
        let mode = mode.trunc() as u8;
        let a1 = match args.get(3).copied().flatten() {
            None => true,
            Some(child) => {
                let value = Self::scalar(self.take(base, child), context)?;
                match value.operand() {
                    Ok(Operand::Error(error)) => return Ok(failure(error)),
                    Ok(value) => match value.logical() {
                        Some(Ok(value)) => value,
                        Some(Err(error)) => return Ok(failure(error)),
                        None => return Ok(Outcome::Uncomputed(Unevaluated::Coercion)),
                    },
                    Err(reason) => return Ok(Outcome::Uncomputed(reason)),
                }
            }
        };
        if !a1 {
            return Ok(Outcome::Uncomputed(Unevaluated::Function(Function::Address)));
        }
        let sheet = match args.get(4).copied().flatten() {
            None => None,
            Some(child) => {
                let value = Self::scalar(self.take(base, child), context)?;
                Some(match value.operand() {
                    Ok(Operand::Text(text)) => text.to_string(),
                    Ok(Operand::Number(value)) if value.is_finite() && value.fract() == 0.0 => value.to_string(),
                    Ok(Operand::Error(error)) => return Ok(failure(error)),
                    Ok(_) => return Ok(Outcome::Uncomputed(Unevaluated::Coercion)),
                    Err(reason) => return Ok(Outcome::Uncomputed(reason)),
                })
            }
        };
        let row = row.trunc() as u32 - 1;
        let column = column.trunc() as u32 - 1;
        let row = if mode <= 2 { Coord::Absolute(row) } else { Coord::Relative(row as i32) };
        let column = if mode == 1 || mode == 3 { Coord::Absolute(column) } else { Coord::Relative(column as i32) };
        let reference = Reference {
            sheet: sheet.as_ref().filter(|text| !text.is_empty()).map_or(SheetSpec::Own,
                |text| SheetSpec::Named { name: text.as_str().into(), quoted: false }),
            target: Target::Cell { row, column },
        };
        let rendered = reference.a1(CellRef::new(0, 0)).to_string();
        let rendered = if sheet.as_deref() == Some("") { format!("!{rendered}") } else { rendered };
        Ok(Outcome::Computed(Operand::Text(crate::Str::from(rendered))))
    }

    fn coerce_number(value: Operand, system: DateSystem) -> Result<f64, Outcome> {
        match value.number(system) {
            Some(Ok(value)) => Ok(value),
            Some(Err(error)) => Err(Outcome::Computed(Operand::Error(error))),
            None => Err(Outcome::Uncomputed(Unevaluated::NumericPolicy)),
        }
    }

    fn numeric(value: f64) -> Outcome {
        match number::finite(value) {
            Ok(value) => Outcome::Computed(Operand::Number(value)),
            Err(error) => Outcome::Computed(Operand::Error(error)),
        }
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! The evaluator context seam, with a bounded fake range and faulting reads.
    use super::super::value::{Operand, Outcome};
    use super::Evaluator;
    use crate::excel::{DateSystem, Formula};
    use crate::{Result, Scalar};

    /// Reusable evaluator exercised independently of Workbook graph integration.
    #[derive(Default)]
    pub struct ContextEvaluator {
        evaluator: Evaluator,
        calls: [usize; 3],
    }

    impl ContextEvaluator {
        /// Evaluate against a range of number/text/Boolean/blank/number values.
        /// `held` blocks the third value; `fail` refuses that read instead.
        ///
        /// # Errors
        /// Returns a source read refusal or invalid test formula.
        pub fn evaluate(
            &mut self,
            formula: &Formula,
            held: bool,
            fail: bool,
        ) -> Result<Option<Scalar>> {
            let expression = formula
                .expression()
                .map_err(|_| crate::Error::InvalidRecord {
                    path: "context-fixture".into(),
                    reason: "expected a compiled expression".into(),
                })?;
            let mut context = Fixture {
                root: expression,
                held,
                fail,
                calls: [0; 3],
                lookup: None,
            };
            let result = self.evaluator.evaluate(expression, &mut context);
            self.calls = context.calls;
            let outcome = result?;
            Ok(match outcome {
                Outcome::Computed(Operand::Number(value)) => Some(Scalar::from(value)),
                Outcome::Computed(Operand::Text(value)) => Some(Scalar::from(value)),
                Outcome::Computed(Operand::Boolean(value)) => Some(Scalar::from(value)),
                Outcome::Computed(Operand::Blank) => Some(Scalar::Null),
                _ => None,
            })
        }

        /// Evaluate a long lookup with a test-only pending source cell.
        ///
        /// The callback count proves a resumed scan starts at its retained
        /// ordinal rather than replaying the already visited prefix.
        pub fn evaluate_lookup(
            &mut self,
            formula: &Formula,
            length: u64,
            match_at: u64,
            pause_at: Option<u64>,
        ) -> Result<Option<Scalar>> {
            let expression = formula.expression().map_err(|_| crate::Error::InvalidRecord {
                path: "context-fixture".into(),
                reason: "expected a compiled lookup expression".into(),
            })?;
            self.evaluator.begin(expression);
            let mut context = Fixture {
                root: expression,
                held: false,
                fail: false,
                calls: [0; 3],
                lookup: Some(LookupProbe {
                    length,
                    match_at,
                    pause_at,
                    ready: false,
                }),
            };
            let result = loop {
                match self.evaluator.resume(expression, &mut context) {
                    Ok(super::Evaluation::Complete(outcome)) => break Ok(outcome),
                    Ok(super::Evaluation::Paused) => {
                        context.lookup.as_mut().expect("lookup probe").ready = true;
                    }
                    Err(error) => break Err(error),
                }
            };
            self.calls = context.calls;
            self.evaluator.clear();
            Ok(match result? {
                Outcome::Computed(Operand::Number(value)) => Some(Scalar::from(value)),
                _ => None,
            })
        }

        /// Reference intake, scalar intersection, and visited range values.
        #[must_use]
        pub const fn calls(&self) -> [usize; 3] {
            self.calls
        }

        /// Retained array execution slots, excluding the borrowed immutable AST.
        /// Plan/choice/element-frame/element-value capacities expose the bound
        /// independently of the Cartesian output area.
        #[must_use]
        pub fn array_capacity(&self) -> [usize; 4] {
            [self.evaluator.arrays.capacity(), self.evaluator.array_choices.capacity(),
             self.evaluator.element_stack.capacity(), self.evaluator.element_values.capacity()]
        }

        /// A failed read must leave no live result or range ID in the buffers.
        #[must_use]
        pub fn is_clear(&self) -> bool {
            self.evaluator.stack.is_empty() && self.evaluator.values.iter().all(Option::is_none) && self.evaluator.names.is_empty() && self.evaluator.name_results.is_empty() && self.evaluator.name_volatility.is_empty() && self.evaluator.arrays.is_empty() && self.evaluator.array_choices.is_empty() && self.evaluator.element_stack.is_empty() && self.evaluator.element_values.is_empty()
        }
    }

    struct Fixture<'w> {
        root: &'w super::Expr,
        held: bool,
        fail: bool,
        calls: [usize; 3],
        lookup: Option<LookupProbe>,
    }

    struct LookupProbe {
        length: u64,
        match_at: u64,
        pause_at: Option<u64>,
        ready: bool,
    }

    impl<'w> super::Context<'w> for Fixture<'w> {
        fn system(&self) -> DateSystem {
            DateSystem::Year1900
        }
        fn text_compatibility(&self) -> super::super::text::Compatibility {
            super::super::text::Compatibility::default()
        }
        fn reference_range(&mut self, _id: super::ReferenceId, _range: super::CellRange) -> Outcome {
            unreachable!("the fixture does not evaluate indexed references")
        }
        fn clock_serial(&mut self, _today: bool) -> Result<f64> {
            unreachable!("the fixture only exercises reference evaluation")
        }
        fn random_u64(&mut self) -> u64 {
            unreachable!("the fixture only exercises reference evaluation")
        }
        fn reference(
            &mut self,
            _name: Option<super::super::value::NameId>,
            _reference: &super::super::reference::Reference,
        ) -> Result<super::ReferenceResult> {
            self.calls[0] += 1;
            Ok(super::ReferenceResult::Value(Outcome::Computed(Operand::Reference(
                super::super::value::ReferenceId(0),
            ))))
        }
        fn expression(&self, name: Option<super::NameId>) -> std::result::Result<&'w super::Expr, super::Held> {
            assert!(name.is_none(), "this context never returns a named expression");
            Ok(self.root)
        }
        fn host(&self) -> super::CellRef { super::CellRef::new(0, 0) }
        fn reference_combine(&mut self, op: super::BinaryOp, _left: super::ReferenceId, _right: super::ReferenceId) -> Outcome {
            Outcome::Uncomputed(super::Unevaluated::Binary(op))
        }
        fn reference_is_union(&self, _id: super::ReferenceId) -> bool { false }
        fn reference_nth(&mut self, id: super::ReferenceId, index: usize) -> Outcome {
            if index == 0 { Outcome::Computed(Operand::Reference(id)) }
            else { Outcome::Computed(Operand::Error(super::ExcelError::Ref)) }
        }
        fn reference_geometry(&self, id: super::super::value::ReferenceId) -> Option<super::CellRange> {
            assert!(id.0 <= 1);
            let end = self.lookup.as_ref().map_or(4, |probe| {
                u32::try_from(probe.length - 1).expect("bounded lookup fixture")
            });
            Some(super::CellRange::new(super::CellRef::new(0, 0), super::CellRef::new(if id.0 == 0 { end } else { 0 }, 0)))
        }
        fn intersection(&mut self, id: super::super::value::ReferenceId) -> Outcome {
            assert!(id.0 <= 1);
            Outcome::Computed(Operand::Reference(super::super::value::ReferenceId(1)))
        }
        fn scalar(&mut self, value: Operand) -> Result<Outcome> {
            self.calls[1] += 1;
            Ok(match value {
                Operand::Reference(_) => Outcome::Computed(Operand::Number(2.0)),
                value => Outcome::Computed(value),
            })
        }
        fn visit_range(
            &mut self,
            id: super::super::value::ReferenceId,
            read: super::RangeRead,
            _from: u64,
            mut visit: impl FnMut(Outcome, u64) -> std::ops::ControlFlow<()>,
        ) -> Result<super::RangeProgress> {
            assert_eq!(id.0, 0);
            if let Some(probe) = &self.lookup {
                assert_eq!(read, super::RangeRead::Lookup);
                for at in _from..probe.length {
                    if probe.pause_at == Some(at) && !probe.ready {
                        return Ok(super::RangeProgress::Paused(at));
                    }
                    self.calls[2] += 1;
                    let value = if at == probe.match_at { 1.0 } else { 2.0 };
                    if visit(Outcome::Computed(Operand::Number(value)), 1).is_break() {
                        break;
                    }
                }
                return Ok(super::RangeProgress::Complete);
            }
            assert_eq!(read, super::RangeRead::Numbers);
            for (at, value) in [
                Operand::Number(2.0),
                Operand::Text(crate::Str::new("3")),
                Operand::Boolean(true),
                Operand::Blank,
                Operand::Number(5.0),
            ]
            .into_iter()
            .enumerate()
            {
                self.calls[2] += 1;
                if at == 2 && self.fail {
                    return Err(crate::Error::InvalidRecord {
                        path: "context-fixture!A3".into(),
                        reason: "expected an available source value, got a read refusal".into(),
                    });
                }
                if visit(if at == 2 && self.held {
                    Outcome::Uncomputed(super::super::value::Unevaluated::Reference)
                } else {
                    Outcome::Computed(value)
                }, 1).is_break() { break; }
            }
            Ok(super::RangeProgress::Complete)
        }
    }
}
