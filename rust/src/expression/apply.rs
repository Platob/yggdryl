//! How a bound expression applies to a target.
//!
//! [`Bound`] used to own one method per thing an expression could run over - a
//! row, a batch, a holder, a set of statistics - which is inside-out: every new
//! target meant another method on `Bound`, and a target defined outside this
//! module could not be one at all. [`ApplyExpression`] inverts the ownership.
//! The *target* says what applying an expression to it produces and how, so a
//! listing entry, a cache, or a foreign table implements the trait beside its
//! own type and this module never learns it exists. `Bound` keeps only the
//! verbs that mean more than "apply" - [`Bound::matches`] is
//! apply-then-require-boolean, [`Bound::filter`] is apply-then-select through
//! the shared kernel - and each is a short composition over the trait.
//!
//! # The targets
//!
//! * One row ([`Value`]) applies to the [`Value`] the expression computes -
//!   the scalar tier, compiled with no Arrow at all.
//! * One Arrow `RecordBatch` applies to one column of answers, an `ArrayRef` -
//!   the vectorized tier, implemented in [`arrow`](super::arrow) behind the
//!   `arrow` feature.
//! * One holder (`dyn `[`Attributes`]) applies to what the holder alone can
//!   settle, three-valued: `Bool(false)` when a conjunct it can answer rules
//!   it out, `Bool(true)` when it settles every conjunct, and `Null` when a
//!   conjunct needs the rows - implemented here, because the holder walk is
//!   the scalar evaluator asked one conjunct at a time.
//! * One container's statistics ([`Bounds`](super::Bounds)) apply to the
//!   three-valued `Option<bool>` pruning runs on - implemented in
//!   [`pushdown`](super::pushdown) beside the pruning rules.
//!
//! # The stream target consumes its receiver
//!
//! A `BatchReader` cannot be applied through `&self`: a stream is wrapped, not
//! borrowed, so [`ApplyExpressionStream`] is the consuming sibling. Its output
//! is deliberately the *filtering* reader rather than a stream of evaluated
//! columns: a stream cannot hand back its answers without draining itself, but
//! it can yield only the rows the predicate keeps, one batch at a time. So
//! apply-then-select is what applying an expression to a stream *means* -
//! selection is the only application a stream can make lazily - and the
//! shape-changing case stays on
//! [`BoundStatement::project_reader`](super::BoundStatement::project_reader).

use super::bind::{Bound, row_values};
use super::eval::Row;
use super::selector::Attributes;
use crate::{Result, Value};

/// How a bound expression applies to this target.
///
/// The output is an associated type rather than a generic parameter because a
/// target has exactly one natural application: a row produces a value, a batch
/// produces a column, statistics produce a certainty. A generic parameter
/// would let one target claim several and force every caller to name which.
pub trait ApplyExpression {
    /// What applying an expression to this target produces.
    type Output;

    /// Apply `bound` to this target.
    ///
    /// # Errors
    ///
    /// Returns an error when the target does not match the schema the
    /// expression was bound against, or when evaluation itself fails - a
    /// strict cast refusing a value, a backing store failing a stat.
    fn apply_expression(&self, bound: &Bound) -> Result<Self::Output>;
}

/// How a bound expression applies to a target it must consume.
///
/// The stream sibling of [`ApplyExpression`]: a reader is applied by being
/// wrapped, and wrapping takes ownership. See the module docs for why the
/// stream's application is the filtering reader.
pub trait ApplyExpressionStream {
    /// What applying an expression to this target produces.
    type Output;

    /// Apply `bound` to this target, consuming it.
    ///
    /// # Errors
    ///
    /// Returns an error when the target cannot be wrapped for this expression.
    fn apply_expression_stream(self, bound: &Bound) -> Result<Self::Output>;
}

/// One row applies to the value the expression computes.
///
/// The row is a [`Value::Sequence`] of column values in schema order.
impl ApplyExpression for Value {
    type Output = Value;

    fn apply_expression(&self, bound: &Bound) -> Result<Value> {
        let values = row_values(self, bound.schema())?;
        bound.node().eval(&Row::new(Some(values), None))
    }
}

/// One holder applies to what it can settle by itself, three-valued.
///
/// Only the conjuncts a holder can answer are evaluated - the ones that read
/// no column. Every other conjunct leaves the conjunction unknown, and an
/// unknown conjunct excludes nothing, which is what keeps a listing filter
/// conservative: it may keep a file the rows will later discard, and it may
/// never discard a file that would have matched.
///
/// The conjuncts run cheapest-first and stop at the first `false`, so a
/// predicate answerable from the path alone performs no backend call.
impl<'holder> ApplyExpression for dyn Attributes + 'holder {
    type Output = Value;

    fn apply_expression(&self, bound: &Bound) -> Result<Value> {
        let row = Row::new(None, Some(self));
        let mut unknown = false;
        for conjunct in bound.node().conjuncts() {
            if conjunct.reads_rows() {
                unknown = true;
                continue;
            }
            match conjunct.eval(&row)?.as_bool() {
                Some(false) => return Ok(Value::Bool(false)),
                Some(true) => {}
                None => unknown = true,
            }
        }
        Ok(if unknown {
            Value::Null
        } else {
            Value::Bool(true)
        })
    }
}
