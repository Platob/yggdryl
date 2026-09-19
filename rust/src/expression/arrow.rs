//! Vectorized evaluation: one resolved tree, one `RecordBatch` at a time.
//!
//! This tier is an optimization of [`eval`](super::eval), never a second
//! definition. Where Arrow owns a kernel for an operator - every comparison,
//! every cast, a struct child, a list element - the kernel runs. Where it does
//! not, the row evaluator runs and its answers are gathered into an array,
//! which is slower and *cannot disagree*. A cast between text and a temporal
//! is the one kernel with a reading in front of it: the column reads through
//! the same [`Scalar::from_temporal_text`](crate::Scalar) and spells through
//! the same classic form a row does, and Arrow's kernel answers only the
//! spellings this crate cannot read. The property test asserts the equality
//! the design is built to make cheap.
//!
//! # What runs as a kernel
//!
//! Comparisons through [`arrow_ord::cmp`], null tests read straight off the
//! validity buffer, `and`/`or`/`not` as three-valued buffer arithmetic, `in`
//! and `between` lowered onto those, a struct child as the child array itself,
//! and a list element or run through [`arrow_select::take`]. Arithmetic and
//! the string functions go through the row path: `arrow-arith` and
//! `arrow-string` are not dependencies of this workspace, and adding two
//! crates to vectorize operations a predicate rarely leads with would be a
//! poor trade.
//!
//! # Where a copy is unavoidable
//!
//! A mask that keeps every row copies nothing: [`Bound::filter`] hands the
//! input batch straight back, so its columns stay pointer-identical. A mask
//! that keeps some rows must copy, because a `RecordBatch` is a dense
//! representation and there is no way to say "these rows" without moving them.
//! A projection of bare columns reorders `ArrayRef`s and never touches a
//! buffer; a struct child is the child array, shared.

use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BooleanArray, Datum, FixedSizeListArray, LargeListArray, ListArray,
    RecordBatch, RecordBatchOptions, RecordBatchReader, Scalar as ArrowScalar, StructArray,
    UInt32Array, UInt64Array,
};
use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder, NullBuffer, OffsetBuffer};
use arrow_ord::cmp;
use arrow_schema::{ArrowError, SchemaRef};

use super::attribute::Attributes;
use super::bind::{Bound, Kind, Node, StepKind};
use super::eval::{Row, keep_elements};
use super::path::{FieldSegment, resolve_index, resolve_range};
use super::{Comparison, Expression, Filter};
use crate::arrow::value::{array_from_values, value_from_array};
use crate::arrow::{BatchReader, Error, Result, field_from_arrow_schema};
use crate::cast::{ArrowCastOptions, cast_field_array};
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
        // term survive a reader that projected its columns away or reordered
        // them, which every columnar reader is entitled to do.
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

impl Bound {
    /// Evaluate this term over one batch, producing one column.
    ///
    /// # Errors
    ///
    /// Returns an error when the batch does not carry a column the term
    /// reads, or when a strict cast refuses a value.
    pub fn evaluate(&self, batch: &RecordBatch) -> Result<ArrayRef> {
        let context = Context::new(self.schema(), batch, None);
        evaluate(self.node(), &context)?.into_column(batch.num_rows())
    }

    /// Evaluate this term over one batch alongside its holder.
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
        let context = Context::new(self.schema(), batch, holder);
        evaluate(self.node(), &context)?.into_column(batch.num_rows())
    }

    /// The selection this predicate makes over one batch.
    ///
    /// Unknown is not true, so a null answer is a `false` in the mask and the
    /// returned array carries no nulls of its own.
    ///
    /// # Errors
    ///
    /// Returns an error when the term is not a predicate, or the batch is
    /// missing a column it reads.
    pub fn filter_mask(&self, batch: &RecordBatch) -> Result<BooleanArray> {
        self.filter_mask_with(batch, None)
    }

    /// The selection this predicate makes over one batch alongside its holder.
    ///
    /// # Errors
    ///
    /// Returns an error when the term is not a predicate, the batch is
    /// missing a column, or the holder fails.
    pub fn filter_mask_with(
        &self,
        batch: &RecordBatch,
        holder: Option<&dyn Attributes>,
    ) -> Result<BooleanArray> {
        if !self.is_predicate() {
            return Err(Error::IncompatibleSchema(format!(
                "expected a boolean term to filter with, got {}",
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
    /// Returns an error when the term is not a predicate, or the batch is
    /// missing a column it reads.
    pub fn filter(&self, batch: &RecordBatch) -> Result<RecordBatch> {
        self.filter_with(batch, None)
    }

    /// Keep the rows one batch's holder and rows both answer true for.
    ///
    /// # Errors
    ///
    /// Returns an error when the term is not a predicate, the batch is
    /// missing a column, or the holder fails.
    pub fn filter_with(
        &self,
        batch: &RecordBatch,
        holder: Option<&dyn Attributes>,
    ) -> Result<RecordBatch> {
        if self.keeps_everything() {
            return Ok(batch.clone());
        }
        let mask = self.filter_mask_with(batch, holder)?;
        if mask.true_count() == mask.len() {
            return Ok(batch.clone());
        }
        arrow_select::filter::filter_record_batch(batch, &mask).map_err(Error::Arrow)
    }

    /// Return whether this predicate folded to `true` at bind, so a filter
    /// over it is the identity and is skipped rather than masked.
    fn keeps_everything(&self) -> bool {
        self.node()
            .as_literal()
            .and_then(crate::Scalar::as_bool)
            .unwrap_or(false)
    }

    /// Wrap a reader so every batch it yields is filtered by this predicate.
    ///
    /// The predicate is bound once, here, never per batch, and a predicate
    /// that folded to `true` hands the reader back untouched.
    #[must_use]
    pub fn filter_reader(self, inner: BatchReader) -> BatchReader {
        if self.keeps_everything() {
            return inner;
        }
        let schema = inner.schema();
        Box::new(Filtered {
            inner,
            bound: self,
            schema,
        })
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

impl Filter {
    /// Wrap a reader so every batch it yields is filtered by this filter.
    ///
    /// This is the streamed entry: the filter is bound once against the
    /// reader's schema and each batch is masked as it arrives, so nothing is
    /// collected. A filter that is always true hands the reader back.
    ///
    /// # Errors
    ///
    /// Returns an error when the filter does not bind against the reader's
    /// schema, or answers anything but a boolean.
    pub fn apply_arrow_reader(&self, reader: BatchReader) -> crate::Result<BatchReader> {
        if self.is_always_true() {
            return Ok(reader);
        }
        let schema = field_from_arrow_schema(crate::media::DEFAULT_ROOT_NAME, &reader.schema())?;
        Ok(self.bind(&schema)?.filter_reader(reader))
    }

    /// Keep the rows of one batch this filter answers true for.
    ///
    /// One batch is the one-batch stream: it goes through
    /// [`Self::apply_arrow_reader`] and comes back as the batch that stream
    /// yields. A caller with many batches hands the stream over instead, or
    /// [binds](Self::bind) once and uses [`Bound::filter`].
    ///
    /// # Errors
    ///
    /// Returns an error when the filter does not bind against the batch, or
    /// answers anything but a boolean.
    pub fn apply_arrow_batch(&self, batch: &RecordBatch) -> crate::Result<RecordBatch> {
        collected(self.apply_arrow_reader(one_batch(batch))?)
    }

    /// Keep the rows of one struct array this filter answers true for.
    ///
    /// The struct's own null mask travels with it: a null struct is a row the
    /// filter answers unknown for, and unknown is not kept.
    ///
    /// # Errors
    ///
    /// Returns an error when the array is not a struct, or the filter does
    /// not bind against it.
    pub fn apply_arrow_array(&self, array: &ArrayRef) -> crate::Result<ArrayRef> {
        if self.is_always_true() {
            return Ok(Arc::clone(array));
        }
        let (_, batch) = struct_batch(array, "filter")?;
        let schema = field_from_arrow_schema(crate::media::DEFAULT_ROOT_NAME, &batch.schema())?;
        let mask = self.bind(&schema)?.filter_mask(&batch)?;
        if mask.true_count() == mask.len() {
            return Ok(Arc::clone(array));
        }
        arrow_select::filter::filter(array.as_ref(), &mask)
            .map_err(|error| crate::Error::from(Error::Arrow(error)))
    }
}

impl Expression {
    /// Apply this expression to a stream of batches.
    ///
    /// This is the one streamed entry every expression has, and the rule the
    /// others derive from: bind once, then batch by batch, never collecting.
    /// A `select` or `where` clause wraps the reader and yields as it is
    /// pulled. A plan keeps, shapes, orders and bounds the stream, then
    /// writes it when it has a target - a sink yields nothing, under the
    /// schema it wrote. A sequence threads the stream through its steps.
    ///
    /// # Errors
    ///
    /// Returns an error when the expression does not bind against the
    /// reader's schema, or a plan's target cannot be held or written.
    pub fn apply_arrow_reader(&self, reader: BatchReader) -> crate::Result<BatchReader> {
        match self {
            Self::Selector(selector) => selector.apply_arrow_reader(reader),
            Self::Filter(filter) => filter.apply_arrow_reader(reader),
            Self::Plan(plan) => plan.apply_arrow_reader(reader),
            Self::Sequence(steps) => steps
                .iter()
                .try_fold(reader, |reader, step| step.apply_arrow_reader(reader)),
        }
    }

    /// Apply this expression to one batch.
    ///
    /// One batch is the one-batch stream through
    /// [`Self::apply_arrow_reader`], collected back into the batch it yields;
    /// a plan that writes yields the empty batch of what it wrote.
    ///
    /// # Errors
    ///
    /// [`Self::apply_arrow_reader`] carries the rule.
    pub fn apply_arrow_batch(&self, batch: &RecordBatch) -> crate::Result<RecordBatch> {
        collected(self.apply_arrow_reader(one_batch(batch))?)
    }

    /// Apply this expression to one struct array.
    ///
    /// # Errors
    ///
    /// Returns an error when the array is not a struct, or the expression
    /// does not apply to it.
    pub fn apply_arrow_array(&self, array: &ArrayRef) -> crate::Result<ArrayRef> {
        match self {
            Self::Selector(selector) => selector.apply_arrow_array(array),
            Self::Filter(filter) => filter.apply_arrow_array(array),
            Self::Plan(_) | Self::Sequence(_) => {
                let (_, batch) = struct_batch(array, "run a plan over")?;
                let applied = self.apply_arrow_batch(&batch)?;
                Ok(Arc::new(StructArray::from(applied)))
            }
        }
    }
}

/// One batch as the stream it is.
pub(crate) fn one_batch(batch: &RecordBatch) -> BatchReader {
    crate::arrow::batch_reader(batch.schema(), [batch.clone()])
}

/// Everything one stream yields, as the batch it amounts to.
///
/// A stream that yields exactly one batch answers that batch itself, buffers
/// and all, which is what keeps the one-batch spelling of an application as
/// zero-copy as the streamed one. More than one batch is concatenated; none
/// is the empty batch of the stream's schema.
pub(crate) fn collected(reader: BatchReader) -> crate::Result<RecordBatch> {
    let schema = reader.schema();
    let mut batches = Vec::new();
    for batch in reader {
        batches.push(batch.map_err(crate::arrow::from_reader_error)?);
    }
    match batches.len() {
        0 => Ok(RecordBatch::new_empty(schema)),
        1 => Ok(batches.swap_remove(0)),
        _ => arrow_select::concat::concat_batches(&schema, &batches)
            .map_err(|error| crate::Error::from(Error::Arrow(error))),
    }
}

/// One struct array as the batch of its children, and the array itself.
pub(crate) fn struct_batch<'array>(
    array: &'array ArrayRef,
    verb: &str,
) -> crate::Result<(&'array StructArray, RecordBatch)> {
    let held = array
        .as_any()
        .downcast_ref::<StructArray>()
        .ok_or_else(|| {
            crate::Error::from(Error::IncompatibleSchema(format!(
                "expected a struct array to {verb}, got {}",
                array.data_type()
            )))
        })?;
    Ok((held, RecordBatch::from(held.clone())))
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
        Kind::Path(base, steps) => {
            let rows = context.batch.num_rows();
            let mut field = &base.field;
            let mut array = evaluate(base, context)?.into_column(rows)?;
            for step in steps {
                array = match &step.kind {
                    StepKind::Segment(segment) => {
                        segment_array(field, &step.field, &array, segment)?
                    }
                    StepKind::Where(predicate) => {
                        kept_elements(field, &step.field, &array, predicate, context.holder)?
                    }
                };
                field = &step.field;
            }
            Ok(Vector::Column(array))
        }
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
            let source = inner.field.clone().into_arrow_field_ref()?;
            Ok(Vector::Column(cast_field_array(
                &node.field,
                Some(source.metadata()),
                array,
                ArrowCastOptions::new().with_safe(safety.is_safe()),
            )?))
        }
        // A user function takes whole columns: its implementation answers the
        // batch at once, or row by row through the one array crossing.
        Kind::Function(super::Function::User(reference), arguments) => {
            let rows = context.batch.num_rows();
            let registered = super::user::lookup_function(reference)?;
            let mut fields = Vec::with_capacity(arguments.len());
            let mut columns = Vec::with_capacity(arguments.len());
            for argument in arguments {
                fields.push(argument.field.clone());
                columns.push(evaluate(argument, context)?.into_column(rows)?);
            }
            Ok(Vector::Column(registered.call_arrow(
                &fields,
                &columns,
                rows,
                &node.field,
            )?))
        }
        // Arithmetic, the string functions, and the constructors have no
        // kernel available here, so they take the row evaluator. It is the
        // same code the scalar tier runs, which is why the two cannot
        // disagree about them.
        _ => fallback(node, context),
    }
}

/// Take one path step through a column, kernel first and row walk otherwise.
///
/// `reached` is the field the step reaches, typed by the one
/// [`FieldSegment::apply_field`] the scalar tier types with, so the two tiers
/// cannot disagree about what a step produces.
fn segment_array(
    field: &Field,
    reached: &Field,
    array: &ArrayRef,
    segment: &FieldSegment,
) -> Result<ArrayRef> {
    if let Some(name) = segment.as_name() {
        if let Some(held) = array.as_any().downcast_ref::<StructArray>() {
            let position = held
                .fields()
                .iter()
                .position(|child| child.name().eq_ignore_ascii_case(name));
            if let Some(position) = position {
                let child = Arc::clone(&held.columns()[position]);
                // A child of a null struct is null, whatever its own buffer
                // says; the parent's mask is folded into it once.
                let stepped = match held.nulls() {
                    Some(nulls) if held.null_count() > 0 => {
                        with_nulls(&child, NullBuffer::union(child.nulls(), Some(nulls)))?
                    }
                    _ => child,
                };
                return Ok(stepped);
            }
        }
    }
    match segment {
        FieldSegment::Index(position) => {
            if let Some(stepped) = list_element(array, *position)? {
                return Ok(stepped);
            }
        }
        FieldSegment::Range { start, end } => {
            let item = reached.clone().into_arrow_field_ref()?;
            let item = match item.data_type() {
                arrow_schema::DataType::List(item) => Some(Arc::clone(item)),
                _ => None,
            };
            if let Some(item) = item {
                if let Some(stepped) = list_run(array, *start, *end, item)? {
                    return Ok(stepped);
                }
            }
        }
        // A predicate reaches here only as a segment bound by no one, which
        // the binder never produces; the row walk still answers it.
        FieldSegment::Field(_) | FieldSegment::Key(_) | FieldSegment::Where(_) => {}
    }
    // No kernel: a map key, a dictionary-encoded container, a list layout
    // without offsets. The row walk answers, gathered into a column.
    let rows = array.len();
    let mut values = Vec::with_capacity(rows);
    for row in 0..rows {
        let value = value_from_array(field.dtype(), array.as_ref(), row)?;
        values.push(segment.apply_scalar(field, &value)?);
    }
    let borrowed: Vec<&Scalar> = values.iter().collect();
    array_from_values(reached, &borrowed)
}

/// The elements of every list a predicate keeps, as one list column.
///
/// The predicate runs once over the flattened elements the rows cover, as
/// the batch of the element struct's children; the answer is read as a
/// certainty and a null element is dropped with it; the kept elements are
/// one `filter`; and the offsets are rebuilt from how many each row kept. A
/// null list stays null through the input's own mask. A layout with no
/// offsets to rebuild takes the row walk, which answers the same thing.
fn kept_elements(
    field: &Field,
    reached: &Field,
    array: &ArrayRef,
    predicate: &Node,
    holder: Option<&dyn Attributes>,
) -> Result<ArrayRef> {
    let element = super::path::list_item(reached.dtype()).ok_or_else(|| {
        Error::IncompatibleSchema(format!(
            "expected a list of structs to keep elements of, got {}",
            reached.dtype()
        ))
    })?;
    let rows = array.len();
    let Some(layout) = offsets(array) else {
        let mut values = Vec::with_capacity(rows);
        for row in 0..rows {
            let value = value_from_array(field.dtype(), array.as_ref(), row)?;
            values.push(keep_elements(element, predicate, &value, holder)?);
        }
        let borrowed: Vec<&Scalar> = values.iter().collect();
        return array_from_values(reached, &borrowed);
    };
    // Only the run of flattened elements the rows cover is asked about: a
    // sliced batch shares its child array with the rows around it.
    let (first, last) = if rows == 0 {
        (0, 0)
    } else {
        (layout.row(0).0, layout.row(rows - 1).1)
    };
    let window = layout.values().slice(first, last.saturating_sub(first));
    let Some(structs) = window.as_any().downcast_ref::<StructArray>() else {
        return Err(Error::IncompatibleSchema(format!(
            "expected a list of structs to keep elements of, got a list of {}",
            window.data_type()
        )));
    };
    let elements = RecordBatch::try_new_with_options(
        Arc::new(arrow_schema::Schema::new(structs.fields().clone())),
        structs.columns().to_vec(),
        &RecordBatchOptions::new().with_row_count(Some(structs.len())),
    )
    .map_err(Error::Arrow)?;
    let context = Context::new(element, &elements, holder);
    let answered = evaluate(predicate, &context)?.into_boolean(structs.len())?;
    let mut mask = certain(&answered);
    if let Some(nulls) = structs.nulls() {
        mask = BooleanArray::new(mask.values() & nulls.inner(), None);
    }
    let bits = mask.values();
    let mut keep = BooleanBufferBuilder::new(structs.len());
    let mut lengths = Vec::with_capacity(rows);
    let mut cursor = 0;
    for row in 0..rows {
        let (start, end) = layout.row(row);
        let (start, end) = (start - first, end.max(start) - first);
        // An element between two rows' runs belongs to neither row.
        keep.append_n(start.saturating_sub(cursor), false);
        if array.is_null(row) {
            keep.append_n(end - start, false);
            lengths.push(0);
        } else {
            keep.append_packed_range(bits.offset() + start..bits.offset() + end, bits.values());
            lengths.push(bits.slice(start, end - start).count_set_bits());
        }
        cursor = cursor.max(end);
    }
    keep.append_n(structs.len().saturating_sub(cursor), false);
    let kept = arrow_select::filter::filter(&window, &BooleanArray::new(keep.finish(), None))
        .map_err(Error::Arrow)?;
    // The item field is the input's own, so the kept values and the field
    // agree byte for byte on what an element is.
    let item = match array.data_type() {
        arrow_schema::DataType::List(item)
        | arrow_schema::DataType::LargeList(item)
        | arrow_schema::DataType::FixedSizeList(item, _) => Arc::clone(item),
        other => {
            return Err(Error::IncompatibleSchema(format!(
                "expected a list to keep elements of, got {other}"
            )));
        }
    };
    let list = ListArray::try_new(
        item,
        OffsetBuffer::from_lengths(lengths),
        kept,
        array.nulls().cloned(),
    )
    .map_err(Error::Arrow)?;
    Ok(Arc::new(list))
}

/// The same array under another validity mask.
fn with_nulls(array: &ArrayRef, nulls: Option<NullBuffer>) -> Result<ArrayRef> {
    let data = array
        .to_data()
        .into_builder()
        .nulls(nulls)
        .build()
        .map_err(Error::Arrow)?;
    Ok(arrow_array::make_array(data))
}

/// The list layouts a position or a run can be taken from with one kernel.
enum Offsets<'array> {
    Small(&'array [i32], &'array ArrayRef),
    Large(&'array [i64], &'array ArrayRef),
    Fixed(i32, &'array ArrayRef),
}

fn offsets(array: &ArrayRef) -> Option<Offsets<'_>> {
    let any = array.as_any();
    if let Some(list) = any.downcast_ref::<ListArray>() {
        return Some(Offsets::Small(list.value_offsets(), list.values()));
    }
    if let Some(list) = any.downcast_ref::<LargeListArray>() {
        return Some(Offsets::Large(list.value_offsets(), list.values()));
    }
    if let Some(list) = any.downcast_ref::<FixedSizeListArray>() {
        return Some(Offsets::Fixed(list.value_length(), list.values()));
    }
    None
}

impl Offsets<'_> {
    /// The `[start, end)` run of flattened positions row `row` holds.
    fn row(&self, row: usize) -> (usize, usize) {
        match self {
            Self::Small(offsets, _) => (
                usize::try_from(offsets[row]).unwrap_or(0),
                usize::try_from(offsets[row + 1]).unwrap_or(0),
            ),
            Self::Large(offsets, _) => (
                usize::try_from(offsets[row]).unwrap_or(0),
                usize::try_from(offsets[row + 1]).unwrap_or(0),
            ),
            Self::Fixed(length, _) => {
                let length = usize::try_from(*length).unwrap_or(0);
                (row * length, (row + 1) * length)
            }
        }
    }

    const fn values(&self) -> &ArrayRef {
        match self {
            Self::Small(_, values) | Self::Large(_, values) | Self::Fixed(_, values) => values,
        }
    }
}

/// One element of every list, by position, through one `take`.
fn list_element(array: &ArrayRef, position: i64) -> Result<Option<ArrayRef>> {
    let Some(layout) = offsets(array) else {
        return Ok(None);
    };
    let mut indices: Vec<Option<u64>> = Vec::with_capacity(array.len());
    for row in 0..array.len() {
        if array.is_null(row) {
            indices.push(None);
            continue;
        }
        let (start, end) = layout.row(row);
        indices.push(
            resolve_index(position, end.saturating_sub(start))
                .map(|index| u64::try_from(start + index).unwrap_or(u64::MAX)),
        );
    }
    let indices = UInt64Array::from(indices);
    arrow_select::take::take(layout.values().as_ref(), &indices, None)
        .map(Some)
        .map_err(Error::Arrow)
}

/// A run of every list, half-open, through one `take` and new offsets.
fn list_run(
    array: &ArrayRef,
    start: Option<i64>,
    end: Option<i64>,
    item: arrow_schema::FieldRef,
) -> Result<Option<ArrayRef>> {
    let Some(layout) = offsets(array) else {
        return Ok(None);
    };
    let mut indices: Vec<u64> = Vec::new();
    let mut lengths: Vec<usize> = Vec::with_capacity(array.len());
    for row in 0..array.len() {
        if array.is_null(row) {
            lengths.push(0);
            continue;
        }
        let (first, last) = layout.row(row);
        let (from, until) = resolve_range(start, end, last.saturating_sub(first));
        for index in from..until {
            indices.push(u64::try_from(first + index).unwrap_or(u64::MAX));
        }
        lengths.push(until - from);
    }
    let taken =
        arrow_select::take::take(layout.values().as_ref(), &UInt64Array::from(indices), None)
            .map_err(Error::Arrow)?;
    let run = ListArray::try_new(
        item,
        OffsetBuffer::from_lengths(lengths),
        taken,
        array.nulls().cloned(),
    )
    .map_err(Error::Arrow)?;
    Ok(Some(Arc::new(run)))
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
