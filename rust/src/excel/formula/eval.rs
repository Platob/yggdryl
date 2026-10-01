//! One explicit-stack evaluator over the typed formula arena.

use super::aggregate::Accumulator;
use super::functions::Function;
use super::number;
use super::parser::{BinaryOp, EvaluationPolicy, Expr, Node, UnaryOp};
use super::shape::Held;
use super::value::{Operand, Outcome, ReferenceId, Unevaluated};
use crate::excel::cell::{DateSystem, ExcelError};
use crate::{Arithmetic, Scalar};

/// Values required by a range consumer. Numeric aggregates can avoid rendering
/// referenced text, but errors and unresolved dependencies always remain visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RangeRead {
    Values,
    Numbers,
}

/// A formula-scoped workbook reader. Reference IDs name descriptors owned by
/// this context, never a borrow stored in the reusable evaluator buffers.
/// The owner resets its arena only after evaluate returns (also on refusal).
pub(crate) trait Context {
    fn system(&self) -> DateSystem;
    fn reference(&mut self, reference: &super::reference::Reference) -> crate::Result<Outcome>;
    fn scalar(&mut self, value: Operand) -> crate::Result<Outcome>;
    /// Visit outcomes in sparse source order. Numbers omits known referenced
    /// text/Boolean/blank before rendering. Both modes must retain errors and
    /// held/circular dependencies, even when their prior cache is nonnumeric.
    fn visit_range(
        &mut self,
        id: ReferenceId,
        read: RangeRead,
        visit: impl FnMut(Outcome),
    ) -> crate::Result<()>;
}

/// Scratch is reused across formula evaluations and workbook passes.
#[derive(Debug, Default)]
pub(crate) struct Evaluator {
    stack: Vec<(usize, bool)>,
    values: Vec<Option<Outcome>>,
}

impl Evaluator {
    pub(crate) fn evaluate(
        &mut self,
        expression: &Expr,
        context: &mut impl Context,
    ) -> crate::Result<Outcome> {
        let result = self.run(expression, context);
        // No ID or earlier result survives arena reuse, including a late read
        // refusal. Vector capacity stays with the calculation owner.
        self.stack.clear();
        self.values.clear();
        result
    }

    fn run(&mut self, expression: &Expr, context: &mut impl Context) -> crate::Result<Outcome> {
        let system = context.system();
        self.stack.clear();
        self.values.clear();
        self.values.resize_with(expression.nodes.len(), || None);
        self.stack.push((expression.root, false));
        while let Some((id, ready)) = self.stack.pop() {
            let node = &expression.nodes[id];
            if !ready {
                match node.evaluation_children() {
                    EvaluationPolicy::Leaf => {
                        self.values[id] = Some(match node {
                            Node::Literal(value) => match Operand::literal(value) {
                                Operand::Number(value) => Self::numeric(value),
                                Operand::Error(ExcelError::Unrecognized) => {
                                    Outcome::Uncomputed(Unevaluated::Held(Held::Unrecognized))
                                }
                                value => Outcome::Computed(value),
                            },
                            Node::Reference(reference) => context.reference(reference)?,
                            _ => {
                                unreachable!("the policy marks only literals/references as leaves")
                            }
                        });
                    }
                    EvaluationPolicy::Strict(children) => {
                        self.stack.push((id, true));
                        children.visit_reverse(|child| self.stack.push((child, false)));
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
                        self.values[id] = Some(Outcome::Uncomputed(reason));
                    }
                }
                continue;
            }
            let resolved = match node {
                Node::Group(child) => self.take(*child),
                Node::Unary { op, value } => {
                    let value = Self::scalar(self.take(*value), context)?;
                    Self::unary(*op, value, system)
                }
                Node::Percent(child) => {
                    let value = Self::scalar(self.take(*child), context)?;
                    Self::percent(value, system)
                }
                Node::Binary { op, left, right } => {
                    let left = Self::scalar(self.take(*left), context)?;
                    let right = Self::scalar(self.take(*right), context)?;
                    Self::binary(*op, left, right, id == expression.root, system)
                }
                Node::Call {
                    function: Some(function @ (Function::Abs | Function::Sqrt)),
                    args,
                } => match args.as_ref() {
                    [Some(argument)] => {
                        let value = Self::scalar(self.take(*argument), context)?;
                        match function {
                            Function::Abs => Self::absolute(value, system),
                            Function::Sqrt => Self::square_root(value, system),
                            _ => unreachable!("the call was matched above"),
                        }
                    }
                    _ => Outcome::Uncomputed(Unevaluated::Function(*function)),
                },
                Node::Call {
                    function: Some(Function::Sum),
                    args,
                } => self.sum(args, context)?,
                Node::Call {
                    function: Some(Function::Round),
                    args,
                } => {
                    let [Some(value), Some(places)] = args.as_ref() else {
                        unreachable!("the strict call policy proved two present arguments")
                    };
                    let value = Self::scalar(self.take(*value), context)?;
                    let places = Self::scalar(self.take(*places), context)?;
                    Self::round(value, places, system)
                }
                Node::Call {
                    function: Some(Function::Mod),
                    args,
                } => {
                    let [Some(left), Some(right)] = args.as_ref() else {
                        unreachable!("the strict call policy proved two present arguments")
                    };
                    let left = Self::scalar(self.take(*left), context)?;
                    let right = Self::scalar(self.take(*right), context)?;
                    Self::modulus(left, right, system)
                }
                _ => unreachable!("the ready stack contains operators with children"),
            };
            self.values[id] = Some(resolved);
        }
        let result = self.take(expression.root);
        Self::scalar(result, context)
    }

    fn scalar(value: Outcome, context: &mut impl Context) -> crate::Result<Outcome> {
        match value {
            Outcome::Computed(value @ Operand::Reference(_)) => context.scalar(value),
            value => Ok(value),
        }
    }

    fn take(&mut self, id: usize) -> Outcome {
        self.values[id].take().expect("the child was scheduled")
    }

    fn unary(op: UnaryOp, value: Outcome, system: DateSystem) -> Outcome {
        let value = match value.operand() {
            Ok(value) => value,
            Err(reason) => return Outcome::Uncomputed(reason),
        };
        if matches!(op, UnaryOp::ImplicitIntersection | UnaryOp::Positive) {
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

    fn sum(
        &mut self,
        args: &[Option<usize>],
        context: &mut impl Context,
    ) -> crate::Result<Outcome> {
        let system = context.system();
        let mut accumulator = Accumulator::default();
        let mut explicit_error = None;
        let mut aggregate_error = None;
        let mut unresolved = None;
        let mut feed = |outcome: Outcome, referenced: bool| match outcome.operand() {
            Err(reason) => {
                unresolved.get_or_insert(reason);
            }
            Ok(Operand::Error(error)) => {
                explicit_error.get_or_insert(error);
            }
            Ok(Operand::Reference(_)) => {
                unresolved.get_or_insert(Unevaluated::Reference);
            }
            Ok(Operand::Blank | Operand::Text(_) | Operand::Boolean(_)) if referenced => {}
            Ok(value) => match value.number(system) {
                Some(Ok(value)) => {
                    if let Err(error) = accumulator.push_number(value) {
                        aggregate_error.get_or_insert(error);
                    }
                }
                Some(Err(error)) => {
                    explicit_error.get_or_insert(error);
                }
                None => {
                    unresolved.get_or_insert(Unevaluated::NumericPolicy);
                }
            },
        };
        for argument in args {
            let Some(argument) = argument else {
                feed(
                    Outcome::Uncomputed(Unevaluated::Function(Function::Sum)),
                    false,
                );
                continue;
            };
            match self.take(*argument) {
                Outcome::Computed(Operand::Reference(id)) => {
                    context.visit_range(id, RangeRead::Numbers, |value| feed(value, true))?;
                }
                value => feed(value, false),
            }
        }
        if let Some(reason) = unresolved {
            return Ok(Outcome::Uncomputed(reason));
        }
        if let Some(error) = explicit_error {
            return Ok(Outcome::Computed(Operand::Error(error)));
        }
        if let Some(error) = aggregate_error {
            return Ok(Outcome::Computed(Operand::Error(error)));
        }
        Ok(match accumulator.finish_sum() {
            Ok(Some(value)) => Outcome::Computed(Operand::Number(value)),
            Ok(None) => Outcome::Uncomputed(Unevaluated::NumericPolicy),
            Err(error) => Outcome::Computed(Operand::Error(error)),
        })
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
        if op == BinaryOp::Equal {
            return match (left, right) {
                (Operand::Number(left), Operand::Number(right)) => match number::equal(left, right)
                {
                    Ok(value) => Outcome::Computed(Operand::Boolean(value)),
                    Err(error) => Outcome::Computed(Operand::Error(error)),
                },
                (Operand::Number(_), Operand::Text(_)) | (Operand::Text(_), Operand::Number(_)) => {
                    Outcome::Computed(Operand::Boolean(false))
                }
                (Operand::Blank, Operand::Text(text)) | (Operand::Text(text), Operand::Blank) => {
                    Outcome::Computed(Operand::Boolean(text.as_str().is_empty()))
                }
                _ => Outcome::Uncomputed(Unevaluated::Coercion),
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
            _ => Outcome::Uncomputed(Unevaluated::Binary(op)),
        }
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
                held,
                fail,
                calls: [0; 3],
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

        /// Reference intake, scalar intersection, and visited range values.
        #[must_use]
        pub const fn calls(&self) -> [usize; 3] {
            self.calls
        }

        /// A failed read must leave no live result or range ID in the buffers.
        #[must_use]
        pub fn is_clear(&self) -> bool {
            self.evaluator.stack.is_empty() && self.evaluator.values.iter().all(Option::is_none)
        }
    }

    struct Fixture {
        held: bool,
        fail: bool,
        calls: [usize; 3],
    }

    impl super::Context for Fixture {
        fn system(&self) -> DateSystem {
            DateSystem::Year1900
        }
        fn reference(
            &mut self,
            _reference: &super::super::reference::Reference,
        ) -> Result<Outcome> {
            self.calls[0] += 1;
            Ok(Outcome::Computed(Operand::Reference(
                super::super::value::ReferenceId(0),
            )))
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
            mut visit: impl FnMut(Outcome),
        ) -> Result<()> {
            assert_eq!(read, super::RangeRead::Numbers);
            assert_eq!(id.0, 0);
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
                visit(if at == 2 && self.held {
                    Outcome::Uncomputed(super::super::value::Unevaluated::Reference)
                } else {
                    Outcome::Computed(value)
                });
            }
            Ok(())
        }
    }
}
