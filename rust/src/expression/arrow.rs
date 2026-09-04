//! Vectorized evaluation: one resolved tree, one `RecordBatch` at a time.
//!
//! This tier is an optimization of [`eval`](super::eval), never a second
//! definition. Where Arrow owns a kernel for an operator - every comparison,
//! every cast - the kernel runs. Where it does not, the row evaluator runs and
//! its answers are gathered into an array, which is slower and *cannot
//! disagree*. A cast between text and a temporal is the one kernel with a
//! reading in front of it: the column reads through the same
//! [`Scalar::from_temporal_text`](crate::Scalar) and spells through the same
//! classic form a row does, and Arrow's kernel answers only the spellings this
//! crate cannot read. The property test asserts the equality the design is
//! built to make cheap.
//!
//! # What runs as a kernel
//!
//! Comparisons through [`arrow_ord::cmp`], null tests read straight off the
//! validity buffer, `and`/`or`/`not` as three-valued buffer arithmetic, and
//! `in` and `between` lowered onto those. Arithmetic and the string functions
//! go through the row path: `arrow-arith` and `arrow-string` are not
//! dependencies of this workspace, and adding two crates to vectorize
//! operations a predicate rarely leads with would be a poor trade.
//!
//! # Where a copy is unavoidable
//!
//! A mask that keeps every row copies nothing: [`Bound::filter`] hands the
//! input batch straight back, so its columns stay pointer-identical. A mask
//! that keeps some rows must copy, because a `RecordBatch` is a dense
//! representation and there is no way to say "these rows" without moving them.
//! A projection reorders `ArrayRef`s and never touches a buffer.

use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BooleanArray, Datum, RecordBatch, RecordBatchReader, Scalar as ArrowScalar,
    UInt32Array,
};
use arrow_buffer::{BooleanBuffer, NullBuffer};
use arrow_ord::cmp;
use arrow_ord::sort::{SortColumn, SortOptions, lexsort_to_indices};
use arrow_schema::{ArrowError, SchemaRef};

use super::Comparison;
use super::apply::{ApplyExpression, ApplyExpressionStream};
use super::bind::{Bound, BoundStatement, Kind, Node};
use super::eval::Row;
use super::parser::{Direction, NullsOrder};
use super::selector::Attributes;
use crate::arrow::value::{array_from_values, value_from_array};
use crate::arrow::{BatchReader, Error, Result};
use crate::field::cast::cast_field_array;
use crate::{Field, Scalar};

/// One evaluated operand: a full column, or one value standing for every row.
///
/// Keeping a constant as one row rather than expanding it is what lets a
/// comparison against a literal reach Arrow's scalar kernel, which reads the
/// constant once instead of once per row.
enum Vector {
    /// One value per row.
    Column(ArrayRef),
    /// One value for every row, held as a one-row array.
    Constant(ArrayRef),
}

impl Vector {
    /// Borrow this operand as a comparison operand.
    fn datum(&self) -> Box<dyn Datum + '_> {
        match self {
            Self::Column(array) => Box::new(array.clone()),
            Self::Constant(array) => Box::new(ArrowScalar::new(array.clone())),
        }
    }

    /// This operand as a full column of `rows` rows.
    fn into_column(self, rows: usize) -> Result<ArrayRef> {
        match self {
            Self::Column(array) => Ok(array),
            // Expanding is the one place a constant costs memory, and it
            // happens only when the caller asked for the values themselves
            // rather than for a comparison against them.
            Self::Constant(array) => {
                let indices = UInt32Array::from(vec![0_u32; rows]);
                arrow_select::take::take(array.as_ref(), &indices, None).map_err(Error::Arrow)
            }
        }
    }

    /// This operand as a boolean column of `rows` rows.
    fn into_boolean(self, rows: usize) -> Result<BooleanArray> {
        boolean_column(self.into_column(rows)?)
    }
}

/// One full column read as the boolean it must be.
fn boolean_column(array: ArrayRef) -> Result<BooleanArray> {
    array
        .as_any()
        .downcast_ref::<BooleanArray>()
        .cloned()
        .ok_or_else(|| {
            Error::IncompatibleSchema(format!(
                "expected a boolean operand, got {}",
                array.data_type()
            ))
        })
}

/// What one batch evaluation knows besides the batch.
struct Context<'batch> {
    schema: &'batch Field,
    batch: &'batch RecordBatch,
    /// Bound-schema index to batch column index, resolved once per call.
    columns: Vec<Option<usize>>,
    holder: Option<&'batch dyn Attributes>,
}

impl<'batch> Context<'batch> {
    fn new(
        schema: &'batch Field,
        batch: &'batch RecordBatch,
        holder: Option<&'batch dyn Attributes>,
    ) -> Self {
        // Matching by name rather than by position is what lets a bound
        // expression survive a reader that projected its columns away or
        // reordered them, which every columnar reader is entitled to do.
        let columns = schema
            .fields()
            .iter()
            .map(|field| {
                batch
                    .schema_ref()
                    .fields()
                    .iter()
                    .position(|held| held.name().eq_ignore_ascii_case(field.name()))
            })
            .collect();
        Self {
            schema,
            batch,
            columns,
            holder,
        }
    }

    fn column(&self, index: usize) -> Result<ArrayRef> {
        let held = self.columns.get(index).copied().flatten().ok_or_else(|| {
            Error::IncompatibleSchema(format!(
                "expected the batch to carry column {:?}",
                self.schema.get_field(index).map_or("?", crate::Field::name)
            ))
        })?;
        Ok(self.batch.column(held).clone())
    }
}

/// One batch applies to one column of answers.
///
/// This is the vectorized tier's application: the batch is the target, the
/// output is one `ArrayRef` with one answer per row. The holder-augmented
/// variants on [`Bound`] generalize it with an extra participant the trait's
/// shape has no room for.
impl ApplyExpression for RecordBatch {
    type Output = ArrayRef;

    fn apply_expression(&self, bound: &Bound) -> crate::Result<ArrayRef> {
        let context = Context::new(bound.schema(), self, None);
        let vector = evaluate(bound.node(), &context).map_err(crate::Error::from)?;
        vector
            .into_column(self.num_rows())
            .map_err(crate::Error::from)
    }
}

/// One reader applies to the reader that yields only matching rows.
///
/// The stream's application is the *filtering* reader - see
/// [`ApplyExpressionStream`] for why selection is the only application a
/// stream can make lazily. The predicate is bound once, here, never per batch.
impl ApplyExpressionStream for BatchReader {
    type Output = BatchReader;

    fn apply_expression_stream(self, bound: &Bound) -> crate::Result<BatchReader> {
        let schema = self.schema();
        Ok(Box::new(Filtered {
            inner: self,
            bound: bound.clone(),
            schema,
        }))
    }
}

impl Bound {
    /// Evaluate this expression over one batch, producing one column.
    ///
    /// The batch target's [`ApplyExpression`], spelled from the expression's
    /// side and answered in this tier's own error type.
    ///
    /// # Errors
    ///
    /// Returns an error when the batch does not carry a column the expression
    /// reads, or when a strict cast refuses a value.
    pub fn evaluate(&self, batch: &RecordBatch) -> Result<ArrayRef> {
        batch.apply_expression(self).map_err(Error::from)
    }

    /// Evaluate this expression over one batch alongside its holder.
    ///
    /// The holder answers every `&holder.*` attribute, which is what lets a
    /// predicate mix a question about the file with a question about the rows
    /// and still run as one pass.
    ///
    /// # Errors
    ///
    /// Returns an error when the batch is missing a column, or the holder
    /// cannot answer an attribute.
    pub fn evaluate_with(
        &self,
        batch: &RecordBatch,
        holder: Option<&dyn Attributes>,
    ) -> Result<ArrayRef> {
        // Without a holder this is exactly the batch target's application.
        let Some(holder) = holder else {
            return self.evaluate(batch);
        };
        let context = Context::new(self.schema(), batch, Some(holder));
        evaluate(self.node(), &context)?.into_column(batch.num_rows())
    }

    /// The selection this predicate makes over one batch.
    ///
    /// Unknown is not true, so a null answer is a `false` in the mask and the
    /// returned array carries no nulls of its own.
    ///
    /// # Errors
    ///
    /// Returns an error when the expression is not a predicate, or the batch
    /// is missing a column it reads.
    pub fn filter_mask(&self, batch: &RecordBatch) -> Result<BooleanArray> {
        self.filter_mask_with(batch, None)
    }

    /// The selection this predicate makes over one batch alongside its holder.
    ///
    /// # Errors
    ///
    /// Returns an error when the expression is not a predicate, the batch is
    /// missing a column, or the holder fails.
    pub fn filter_mask_with(
        &self,
        batch: &RecordBatch,
        holder: Option<&dyn Attributes>,
    ) -> Result<BooleanArray> {
        if !self.is_predicate() {
            return Err(Error::IncompatibleSchema(format!(
                "expected a boolean expression to filter with, got {}",
                self.field().dtype()
            )));
        }
        let answered = boolean_column(self.evaluate_with(batch, holder)?)?;
        Ok(certain(&answered))
    }

    /// Keep the rows of one batch this predicate answers true for.
    ///
    /// A mask that keeps every row returns the input batch itself - the same
    /// `ArrayRef`s, not copies - because filtering nothing is not a reason to
    /// move a buffer.
    ///
    /// # Errors
    ///
    /// Returns an error when the expression is not a predicate, or the batch
    /// is missing a column it reads.
    pub fn filter(&self, batch: &RecordBatch) -> Result<RecordBatch> {
        self.filter_with(batch, None)
    }

    /// Keep the rows one batch's holder and rows both answer true for.
    ///
    /// # Errors
    ///
    /// Returns an error when the expression is not a predicate, the batch is
    /// missing a column, or the holder fails.
    pub fn filter_with(
        &self,
        batch: &RecordBatch,
        holder: Option<&dyn Attributes>,
    ) -> Result<RecordBatch> {
        let mask = self.filter_mask_with(batch, holder)?;
        if mask.true_count() == mask.len() {
            return Ok(batch.clone());
        }
        arrow_select::filter::filter_record_batch(batch, &mask).map_err(Error::Arrow)
    }

    /// Wrap a reader so every batch it yields is filtered by this predicate.
    ///
    /// The stream target's [`ApplyExpressionStream`], spelled from the
    /// expression's side.
    #[must_use]
    pub fn filter_reader(self, inner: BatchReader) -> BatchReader {
        inner
            .apply_expression_stream(&self)
            .unwrap_or_else(|_| unreachable!("wrapping a reader performs no fallible work"))
    }
}

/// Read a mask as a certainty: unknown becomes `false`, and no nulls remain.
fn certain(answered: &BooleanArray) -> BooleanArray {
    match answered.nulls() {
        None => answered.clone(),
        Some(nulls) => BooleanArray::new(answered.values() & nulls.inner(), None),
    }
}

/// One reader's batches, each filtered by one bound predicate.
struct Filtered {
    inner: BatchReader,
    bound: Bound,
    schema: SchemaRef,
}

impl Iterator for Filtered {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        let batch = match self.inner.next()? {
            Ok(batch) => batch,
            Err(error) => return Some(Err(error)),
        };
        Some(
            self.bound
                .filter(&batch)
                .map_err(|error| ArrowError::ExternalError(Box::new(error))),
        )
    }
}

impl RecordBatchReader for Filtered {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

impl BoundStatement {
    /// Apply this statement's predicate and projections to one batch.
    ///
    /// A projection that is a bare column reuses the batch's own `ArrayRef`,
    /// so selecting and reordering columns copies no buffer.
    ///
    /// # Errors
    ///
    /// Returns an error when the batch is missing a column, or a projection
    /// cannot be evaluated.
    pub fn project(&self, batch: &RecordBatch) -> Result<RecordBatch> {
        let filtered = match self.predicate() {
            Some(predicate) => predicate.filter(batch)?,
            None => batch.clone(),
        };
        if self.is_all() {
            return Ok(filtered);
        }
        let mut columns = Vec::with_capacity(self.projections().len());
        for projection in self.projections() {
            columns.push(projection.evaluate(&filtered)?);
        }
        let schema = crate::arrow::arrow_schema_from_field(self.output())?;
        RecordBatch::try_new(schema, columns).map_err(Error::Arrow)
    }

    /// Sort one batch by this statement's ordering keys.
    ///
    /// Sorting is not part of [`Self::project_reader`], because a sort over a
    /// stream is a sort over everything the stream will ever yield and this
    /// module does not decide that a caller may buffer it. A caller that has
    /// one batch - or has collected them - sorts it here.
    ///
    /// # Errors
    ///
    /// Returns an error when an ordering key cannot be evaluated.
    pub fn sort(&self, batch: &RecordBatch) -> Result<RecordBatch> {
        if self.ordering().is_empty() {
            return Ok(batch.clone());
        }
        let mut keys = Vec::with_capacity(self.ordering().len());
        for (bound, direction, nulls) in self.ordering() {
            let descending = matches!(direction, Direction::Descending);
            keys.push(SortColumn {
                values: bound.evaluate(batch)?,
                options: Some(SortOptions {
                    descending,
                    // SQL's default puts nulls where the ordering's extreme is,
                    // which is last ascending and first descending.
                    nulls_first: match nulls {
                        Some(NullsOrder::First) => true,
                        Some(NullsOrder::Last) => false,
                        None => descending,
                    },
                }),
            });
        }
        let limit = self.limit().and_then(|limit| usize::try_from(limit).ok());
        let indices = lexsort_to_indices(&keys, limit).map_err(Error::Arrow)?;
        let columns = batch
            .columns()
            .iter()
            .map(|column| arrow_select::take::take(column.as_ref(), &indices, None))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::Arrow)?;
        RecordBatch::try_new(batch.schema(), columns).map_err(Error::Arrow)
    }

    /// Wrap a reader so every batch it yields is filtered and projected.
    ///
    /// The row limit is applied across the stream; the ordering is not, for
    /// the reason [`Self::sort`] gives.
    ///
    /// # Errors
    ///
    /// Returns an error when the output schema cannot be materialized.
    pub fn project_reader(self, inner: BatchReader) -> Result<BatchReader> {
        let schema = crate::arrow::arrow_schema_from_field(self.output())?;
        Ok(Box::new(Projected {
            inner,
            statement: self,
            schema,
            taken: 0,
        }))
    }
}

/// One reader's batches, each filtered, projected, and counted against a limit.
struct Projected {
    inner: BatchReader,
    statement: BoundStatement,
    schema: SchemaRef,
    taken: u64,
}

impl Iterator for Projected {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(limit) = self.statement.limit() {
                if self.taken >= limit {
                    return None;
                }
            }
            let batch = match self.inner.next()? {
                Ok(batch) => batch,
                Err(error) => return Some(Err(error)),
            };
            let projected = match self.statement.project(&batch) {
                Ok(projected) => projected,
                Err(error) => {
                    return Some(Err(ArrowError::ExternalError(Box::new(error))));
                }
            };
            let rows = u64::try_from(projected.num_rows()).unwrap_or(u64::MAX);
            let projected = match self.statement.limit() {
                Some(limit) if self.taken + rows > limit => {
                    let keep = usize::try_from(limit - self.taken).unwrap_or(0);
                    projected.slice(0, keep)
                }
                _ => projected,
            };
            self.taken += u64::try_from(projected.num_rows()).unwrap_or(0);
            if projected.num_rows() == 0 {
                continue;
            }
            return Some(Ok(projected));
        }
    }
}

impl RecordBatchReader for Projected {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// Evaluate one resolved node over one batch.
fn evaluate(node: &Node, context: &Context<'_>) -> Result<Vector> {
    // A subtree that reads no column is the same value for every row, holder
    // attributes included. Answering it once and pinning it is both the
    // constant path and the attribute path.
    if !node.reads_rows() {
        let value = node.eval(&Row::new(None, context.holder))?;
        return Ok(Vector::Constant(array_from_values(&node.field, &[&value])?));
    }
    match &node.kind {
        Kind::Column(index) => Ok(Vector::Column(context.column(*index)?)),
        Kind::And(operands) => kleene(operands, context, true),
        Kind::Or(operands) => kleene(operands, context, false),
        Kind::Not(inner) => {
            let answered = evaluate(inner, context)?.into_boolean(context.batch.num_rows())?;
            Ok(Vector::Column(Arc::new(BooleanArray::new(
                !answered.values(),
                answered.nulls().cloned(),
            ))))
        }
        Kind::Compare(left, comparison, right) => {
            let left = evaluate(left, context)?;
            let right = evaluate(right, context)?;
            Ok(Vector::Column(Arc::new(compare(
                &left,
                *comparison,
                &right,
            )?)))
        }
        Kind::In(value, list) => {
            let value = evaluate(value, context)?;
            let mut answered: Option<BooleanArray> = None;
            for item in list {
                let item = evaluate(item, context)?;
                let equal = compare(&value, Comparison::Eq, &item)?;
                answered = Some(match answered {
                    None => equal,
                    Some(held) => kleene_pair(&held, &equal, false),
                });
            }
            Ok(Vector::Column(Arc::new(answered.unwrap_or_else(|| {
                BooleanArray::new(BooleanBuffer::new_unset(context.batch.num_rows()), None)
            }))))
        }
        Kind::Between(value, low, high) => {
            let value = evaluate(value, context)?;
            let low = evaluate(low, context)?;
            let high = evaluate(high, context)?;
            let above = compare(&value, Comparison::GtEq, &low)?;
            let below = compare(&value, Comparison::LtEq, &high)?;
            Ok(Vector::Column(Arc::new(kleene_pair(&above, &below, true))))
        }
        Kind::IsNull(inner) | Kind::IsNotNull(inner) => {
            let rows = context.batch.num_rows();
            let array = evaluate(inner, context)?.into_column(rows)?;
            let present = array.nulls().map_or_else(
                || BooleanBuffer::new_set(rows),
                |nulls| nulls.inner().clone(),
            );
            let values = if matches!(node.kind, Kind::IsNull(_)) {
                !&present
            } else {
                present
            };
            Ok(Vector::Column(Arc::new(BooleanArray::new(values, None))))
        }
        Kind::Cast(inner, safety) => {
            let rows = context.batch.num_rows();
            let array = evaluate(inner, context)?.into_column(rows)?;
            // The operand's Field keeps its extension identity in the cast:
            // an ASCII column meets a text literal as its trimmed text.
            let source = inner.field.clone().into_arrow_ref()?;
            Ok(Vector::Column(cast_field_array(
                &node.field,
                Some(source.metadata()),
                array,
                safety.is_safe(),
            )?))
        }
        // Arithmetic, the string functions, path steps, and the constructors
        // have no kernel available here, so they take the row evaluator. It is
        // the same code the scalar tier runs, which is why the two cannot
        // disagree about them.
        _ => fallback(node, context),
    }
}

/// Answer one comparison through Arrow's kernel for it.
fn compare(left: &Vector, comparison: Comparison, right: &Vector) -> Result<BooleanArray> {
    let left = left.datum();
    let right = right.datum();
    let kernel = match comparison {
        Comparison::Eq => cmp::eq,
        Comparison::NotEq => cmp::neq,
        Comparison::Lt => cmp::lt,
        Comparison::LtEq => cmp::lt_eq,
        Comparison::Gt => cmp::gt,
        Comparison::GtEq => cmp::gt_eq,
        Comparison::IsDistinctFrom => cmp::distinct,
        Comparison::IsNotDistinctFrom => cmp::not_distinct,
    };
    kernel(left.as_ref(), right.as_ref()).map_err(Error::Arrow)
}

/// Three-valued `and` or `or` over a whole operand list.
fn kleene(operands: &[Node], context: &Context<'_>, conjunction: bool) -> Result<Vector> {
    let rows = context.batch.num_rows();
    let mut answered: Option<BooleanArray> = None;
    for operand in operands {
        let next = evaluate(operand, context)?.into_boolean(rows)?;
        answered = Some(match answered {
            None => next,
            Some(held) => kleene_pair(&held, &next, conjunction),
        });
    }
    Ok(Vector::Column(Arc::new(answered.unwrap_or_else(|| {
        // An empty conjunction is true and an empty disjunction is false.
        let values = if conjunction {
            BooleanBuffer::new_set(rows)
        } else {
            BooleanBuffer::new_unset(rows)
        };
        BooleanArray::new(values, None)
    }))))
}

/// Three-valued `and` or `or` of two masks.
///
/// Written over the buffers directly because `arrow-arith` is not a dependency
/// of this workspace. The rule is the one SQL states: a `false` operand settles
/// an `and` however unknown the other is, and a `true` operand settles an `or`.
fn kleene_pair(left: &BooleanArray, right: &BooleanArray, conjunction: bool) -> BooleanArray {
    let length = left.len().min(right.len());
    let left_valid = validity(left, length);
    let right_valid = validity(right, length);
    let left_values = left.values().slice(0, length);
    let right_values = right.values().slice(0, length);
    let (values, left_settles, right_settles) = if conjunction {
        // A `false` operand settles an `and` however unknown the other is.
        let (not_left, not_right) = (!&left_values, !&right_values);
        (
            &left_values & &right_values,
            &left_valid & &not_left,
            &right_valid & &not_right,
        )
    } else {
        // A `true` operand settles an `or` the same way.
        (
            &left_values | &right_values,
            &left_valid & &left_values,
            &right_valid & &right_values,
        )
    };
    let both_known = &left_valid & &right_valid;
    let settled = &left_settles | &right_settles;
    let valid = &both_known | &settled;
    BooleanArray::new(values, Some(NullBuffer::new(valid)))
}

/// One mask's validity, as a buffer that is set wherever it is known.
fn validity(array: &BooleanArray, length: usize) -> BooleanBuffer {
    array.nulls().map_or_else(
        || BooleanBuffer::new_set(length),
        |nulls| nulls.inner().slice(0, length),
    )
}

/// Evaluate one node row by row and gather the answers into an array.
fn fallback(node: &Node, context: &Context<'_>) -> Result<Vector> {
    let rows = context.batch.num_rows();
    let indices = node.column_indices();
    let mut row = vec![Scalar::Null; context.schema.field_len()];
    let mut columns = Vec::with_capacity(indices.len());
    for index in &indices {
        let field = context.schema.get_field(*index).ok_or_else(|| {
            Error::IncompatibleSchema(format!("expected the schema to carry column {index}"))
        })?;
        columns.push((*index, field.dtype().clone(), context.column(*index)?));
    }
    let mut answers = Vec::with_capacity(rows);
    for position in 0..rows {
        for (index, dtype, array) in &columns {
            row[*index] = value_from_array(dtype, array.as_ref(), position)?;
        }
        answers.push(node.eval(&Row::new(Some(&row), context.holder))?);
    }
    let borrowed: Vec<&Scalar> = answers.iter().collect();
    Ok(Vector::Column(array_from_values(&node.field, &borrowed)?))
}
