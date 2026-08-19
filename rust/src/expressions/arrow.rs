//! Vectorized evaluation: one mask per batch, one filter per batch.
//!
//! The same [`Bound`] the rows are filtered with is read here column at a
//! time. Comparisons go through [`arrow_ord::cmp`] against a scalar built once
//! from the already-folded literal - never a per-batch cast of the literal, and
//! never an `ArrayFormatter` per row, which is what this replaces. Every
//! conjunct is combined into one [`BooleanArray`] and
//! [`filter_record_batch`](arrow_select::filter::filter_record_batch) runs once
//! per batch, never once per predicate.
//!
//! # Kleene logic, in the buffers
//!
//! Arrow's boolean kernels for three-valued `AND`/`OR` live in `arrow-arith`,
//! which this crate deliberately does not depend on. They are eleven lines of
//! buffer arithmetic, so they are written here rather than paid for as a crate:
//! a result is *false* when either side is false whatever the other is, *true*
//! when both are true, and null otherwise - which is exactly what the row
//! evaluator answers, and the parity test proves it.
//!
//! # Where a kernel does not exist
//!
//! Arithmetic, the scalar functions, and `CASE` have no kernel in the crates
//! the `arrow` feature already links. Those nodes fall back to the row path
//! *for that node only*: each operand is read once into a column of values and
//! the node itself runs per row, so nothing above or below it pays. On the
//! `expression_by_family` benchmark that is roughly **two orders of magnitude**
//! slower per row than a kernel-backed comparison, which is exactly why the
//! predicates a read pushes down are the ones that have kernels.
//!
//! It is paid instead of adding `arrow-arith` and `arrow-string` to three
//! manifests for kernels a *filter* rarely reaches - a predicate is
//! overwhelmingly comparisons, null tests, and set membership, all of which are
//! kernel-backed here. A caller whose filter is mostly arithmetic is better
//! served by a computed column in the selection, which runs once per batch.

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BooleanArray, Datum, RecordBatch, Scalar, StructArray};
use arrow_buffer::{BooleanBuffer, NullBuffer};
use arrow_schema::SchemaRef;

use super::apply::{Apply, Program};
use super::bound::{Bound, BoundColumn, BoundPredicate, Step};
use super::graph::{Node, NodeId, Plan};
use super::select::BoundSelection;
use crate::arrow::{BatchReader, Error, Result};
use crate::field::cast::ArrowCast;
use crate::{DataType, Field, Value};

impl Bound {
    /// Evaluate this plan over a batch, producing one column.
    ///
    /// # Errors
    ///
    /// Returns an error when the batch disagrees with the schema this plan was
    /// bound to, or when a kernel refuses the values it was handed.
    pub fn evaluate_batch(&self, batch: &RecordBatch) -> Result<ArrayRef> {
        let mut vectorizer = Vectorizer::new(self.plan(), batch);
        vectorizer.column(self.root())
    }
}

impl BoundPredicate {
    /// Build the one boolean mask this predicate selects with.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Bound::evaluate_batch`] returns, plus an error when
    /// the plan does not evaluate to a boolean over this batch.
    pub fn mask(&self, batch: &RecordBatch) -> Result<BooleanArray> {
        let array = self.bound().evaluate_batch(batch)?;
        array
            .as_any()
            .downcast_ref::<BooleanArray>()
            .cloned()
            .ok_or_else(|| {
                Error::IncompatibleSchema(format!(
                    "a filter must evaluate to a boolean column, got {}",
                    array.data_type()
                ))
            })
    }

    /// Keep the rows of one batch that match.
    ///
    /// The mask's nulls are dropped by the filter, which *is* three-valued
    /// logic by construction: a row a comparison answered unknown for is not
    /// kept, exactly as the row evaluator does not keep it.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Self::mask`] returns.
    pub fn filter_batch(&self, batch: &RecordBatch) -> Result<RecordBatch> {
        if self.is_always_true() {
            return Ok(batch.clone());
        }
        let mask = self.mask(batch)?;
        arrow_select::filter::filter_record_batch(batch, &mask).map_err(Error::from)
    }

    /// Wrap a reader so only matching rows flow, binding once and streaming.
    ///
    /// The returned reader answers `schema()` immediately - the schema is the
    /// one it was handed, because filtering does not change a schema - holds at
    /// most one batch, and never materializes a vector.
    ///
    /// # Errors
    ///
    /// Returns an error when the reader's schema disagrees with the schema
    /// this predicate was bound to, *before* the first batch is pulled.
    pub fn filter_batch_reader(&self, inner: BatchReader) -> Result<BatchReader> {
        if self.is_always_true() {
            return Ok(inner);
        }
        let schema = inner.schema();
        check_schema(self.bound(), schema.as_ref())?;
        Ok(Box::new(Filtered {
            inner,
            predicate: self.clone(),
            schema,
        }))
    }
}

impl BoundSelection {
    /// Project one batch into the batch this selection produces.
    ///
    /// # Errors
    ///
    /// Returns whatever evaluating one of the items returns.
    pub fn project_batch(&self, batch: &RecordBatch) -> Result<RecordBatch> {
        if self.is_everything() {
            return Ok(batch.clone());
        }
        let schema = crate::arrow::schema_from_field(self.root())?;
        let mut columns = Vec::with_capacity(self.items().len());
        for (item, field) in self.items().iter().zip(self.root().fields()) {
            let array = item.evaluate_batch(batch)?;
            // The declared field is the authority on nullability, dictionary
            // options, and extension identity, so the computed column is put
            // into exactly the shape `apply_field` reported.
            columns.push(field.cast_arrow_array(array, true)?);
        }
        RecordBatch::try_new(schema, columns).map_err(Error::from)
    }

    /// Wrap a reader so every batch is projected, binding once and streaming.
    ///
    /// # Errors
    ///
    /// Returns an error when the reader's schema disagrees with the schema
    /// this selection was bound against.
    pub fn project_batch_reader(&self, inner: BatchReader) -> Result<BatchReader> {
        if self.is_everything() {
            return Ok(inner);
        }
        check_schema_root(self.schema(), inner.schema().as_ref())?;
        let schema = crate::arrow::schema_from_field(self.root())?;
        Ok(Box::new(Projected {
            inner,
            selection: self.clone(),
            schema,
        }))
    }
}

/// Refuse a batch whose columns disagree with what a plan was bound to.
fn check_schema(bound: &Bound, schema: &arrow_schema::Schema) -> Result<()> {
    check_schema_root(bound.schema(), schema)
}

/// Refuse a stream whose columns the bound root cannot be read from.
///
/// Only the columns the plan actually reads have to be there: a reader that
/// carries more is projected, and a reader that carries fewer is a typed error
/// naming the first column that is missing rather than a silent null column.
fn check_schema_root(root: &Field, schema: &arrow_schema::Schema) -> Result<()> {
    for field in root.fields() {
        let present = schema
            .fields()
            .iter()
            .any(|held| held.name().eq_ignore_ascii_case(field.name()));
        if !present {
            return Err(Error::IncompatibleSchema(format!(
                "expected the column {:?} this expression was bound to, got a stream of {}",
                field.name(),
                schema
                    .fields()
                    .iter()
                    .map(|held| held.name().as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
    }
    Ok(())
}

/// A reader that drops the rows one predicate does not keep.
struct Filtered {
    inner: BatchReader,
    predicate: BoundPredicate,
    schema: SchemaRef,
}

impl Iterator for Filtered {
    type Item = std::result::Result<RecordBatch, arrow_schema::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        let batch = match self.inner.next()? {
            Ok(batch) => batch,
            Err(error) => return Some(Err(error)),
        };
        Some(
            self.predicate
                .filter_batch(&batch)
                .map_err(|error| arrow_schema::ArrowError::ComputeError(error.to_string())),
        )
    }
}

impl arrow_array::RecordBatchReader for Filtered {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// A reader that projects every batch through one selection.
struct Projected {
    inner: BatchReader,
    selection: BoundSelection,
    schema: SchemaRef,
}

impl Iterator for Projected {
    type Item = std::result::Result<RecordBatch, arrow_schema::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        let batch = match self.inner.next()? {
            Ok(batch) => batch,
            Err(error) => return Some(Err(error)),
        };
        Some(
            self.selection
                .project_batch(&batch)
                .map_err(|error| arrow_schema::ArrowError::ComputeError(error.to_string())),
        )
    }
}

impl arrow_array::RecordBatchReader for Projected {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// One node's value over a batch: a full column or a pinned one-row scalar.
#[derive(Clone)]
enum Held {
    /// One value per row.
    Full(ArrayRef),
    /// One value for every row, pinned so a kernel broadcasts it.
    ///
    /// The one-row array is kept beside the pinned view because `Scalar` only
    /// lends a `&dyn Array` back, and both a re-cast and a broadcast need the
    /// shared pointer rather than a borrow.
    Pinned {
        /// The kernel-facing view.
        scalar: Scalar<ArrayRef>,
        /// The one-row array the view pins.
        array: ArrayRef,
    },
}

impl Held {
    /// Borrow this value as something a kernel accepts.
    fn datum(&self) -> &dyn Datum {
        match self {
            Self::Full(array) => array,
            Self::Pinned { scalar, .. } => scalar,
        }
    }

    /// The Arrow datatype this value holds.
    fn data_type(&self) -> &arrow_schema::DataType {
        match self {
            Self::Full(array) => array.data_type(),
            Self::Pinned { array, .. } => array.data_type(),
        }
    }

    /// Spread this value over `rows` rows, for the row-at-a-time fallback.
    fn expand(&self, rows: usize) -> Result<ArrayRef> {
        match self {
            Self::Full(array) => Ok(Arc::clone(array)),
            Self::Pinned { array, .. } => {
                let indices = arrow_array::UInt32Array::from(vec![0_u32; rows]);
                arrow_select::take::take(array.as_ref(), &indices, None).map_err(Error::from)
            }
        }
    }
}

/// Pin one one-row array as a broadcastable scalar.
fn pinned(array: ArrayRef) -> Held {
    Held::Pinned {
        scalar: Scalar::new(Arc::clone(&array)),
        array,
    }
}

/// The column-at-a-time evaluator over one batch.
struct Vectorizer<'batch> {
    plan: &'batch Plan,
    batch: &'batch RecordBatch,
    /// One entry per already-computed node.
    ///
    /// The plan interns structurally identical subtrees to one id, so this map
    /// is what turns that into *computed once per batch*: `price * 1.1`
    /// written three times is one node and one evaluation.
    cache: HashMap<NodeId, Held>,
}

impl<'batch> Vectorizer<'batch> {
    /// Start evaluating one plan over one batch.
    fn new(plan: &'batch Plan, batch: &'batch RecordBatch) -> Self {
        Self {
            plan,
            batch,
            cache: HashMap::new(),
        }
    }

    /// Evaluate one node into a full column.
    fn column(&mut self, id: NodeId) -> Result<ArrayRef> {
        let held = self.held(id)?;
        held.expand(self.batch.num_rows())
    }

    /// Evaluate one node, keeping a scalar pinned where it is one.
    #[allow(clippy::too_many_lines)]
    fn held(&mut self, id: NodeId) -> Result<Held> {
        if let Some(held) = self.cache.get(&id) {
            return Ok(held.clone());
        }
        let Some(node) = self.plan.get(id) else {
            return Err(Error::IncompatibleSchema(
                "the plan refers to a node it does not hold".to_owned(),
            ));
        };
        let held = match node {
            Node::Literal(value) => {
                // The literal was folded into the column's own type at bind
                // time, so the scalar is built once here and never re-cast.
                let field = self.field_of(id, value)?;
                pinned(crate::arrow::scalar_array(&field, value)?)
            }
            Node::Column(column) => match column.bound() {
                Some(bound) => Held::Full(self.read_column(bound)?),
                None => {
                    let field = Field::new("column", DataType::Null, true);
                    pinned(crate::arrow::scalar_array(&field, &Value::Null)?)
                }
            },
            Node::Alias { child, .. } => self.held(*child)?,
            Node::Cast {
                child,
                data_type,
                safe,
            } => {
                let array = self.column(*child)?;
                Held::Full(data_type.cast_arrow_array(array, *safe)?)
            }
            Node::Compare { op, left, right } => {
                let (left, right) = self.aligned(*left, *right)?;
                let mask = compare(*op, left.datum(), right.datum())?;
                Held::Full(Arc::new(mask))
            }
            Node::And(operands) => Held::Full(Arc::new(self.connective(operands, true)?)),
            Node::Or(operands) => Held::Full(Arc::new(self.connective(operands, false)?)),
            Node::Not(child) => {
                let mask = self.boolean(*child)?;
                Held::Full(Arc::new(kleene_not(&mask)))
            }
            Node::IsNull(child) => {
                let array = self.column(*child)?;
                Held::Full(Arc::new(null_test(array.as_ref(), true)))
            }
            Node::IsNotNull(child) => {
                let array = self.column(*child)?;
                Held::Full(Arc::new(null_test(array.as_ref(), false)))
            }
            Node::In {
                child,
                list,
                negated,
            } => {
                // A membership is a disjunction of equalities, so Kleene `OR`
                // gives it SQL's null behavior with no extra rule.
                let mut mask: Option<BooleanArray> = None;
                for item in list {
                    let (left, right) = self.aligned(*child, *item)?;
                    let one = compare(super::CompareOp::Eq, left.datum(), right.datum())?;
                    mask = Some(match mask {
                        Some(held) => kleene(&held, &one, false),
                        None => one,
                    });
                }
                let mask =
                    mask.unwrap_or_else(|| BooleanArray::from(vec![false; self.batch.num_rows()]));
                Held::Full(Arc::new(if *negated { kleene_not(&mask) } else { mask }))
            }
            Node::StartsWith { child, prefix } => {
                let array = self.column(*child)?;
                Held::Full(Arc::new(prefix_test(array.as_ref(), prefix)?))
            }
            Node::Like {
                child,
                pattern,
                escape,
                negated,
                case_insensitive,
            } => {
                let text = self.column(*child)?;
                let pattern = self.column(*pattern)?;
                Held::Full(Arc::new(like_test(
                    text.as_ref(),
                    pattern.as_ref(),
                    *escape,
                    *negated,
                    *case_insensitive,
                )?))
            }
            // Arithmetic, the scalar functions, `CASE`, and `BETWEEN` on a
            // non-lowered shape have no kernel in the linked crates, so this
            // node - and only this node - is answered a row at a time.
            other => Held::Full(self.row_fallback(id, &other.clone())?),
        };
        self.cache.insert(id, held.clone());
        Ok(held)
    }

    /// The field a node's value is materialized under.
    fn field_of(&self, id: NodeId, value: &Value) -> Result<Field> {
        let data_type = match self.plan.data_type(id) {
            Some(data_type) => data_type.clone(),
            None => value.data_type()?,
        };
        Ok(Field::new("literal", data_type, true))
    }

    /// Read one bound column out of the batch, following its slot chain.
    fn read_column(&self, column: &BoundColumn) -> Result<ArrayRef> {
        let index = self
            .batch
            .schema()
            .fields()
            .iter()
            .position(|field| field.name().eq_ignore_ascii_case(column.name()))
            .ok_or_else(|| {
                Error::IncompatibleSchema(format!(
                    "expected the column {:?} this expression was bound to, got a batch of {}",
                    column.name(),
                    self.batch
                        .schema()
                        .fields()
                        .iter()
                        .map(|field| field.name().as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?;
        let mut held = Arc::clone(self.batch.column(index));
        let mut field = column.root_field().clone();
        for step in column.steps() {
            match step {
                // A struct child is a zero-copy slice of the array that is
                // already there, which is the one accessor that costs nothing.
                Step::Child { index, name } => {
                    let structs = held.as_any().downcast_ref::<StructArray>().ok_or_else(|| {
                        Error::IncompatibleSchema(format!(
                            "expected a struct column to read {name:?} from, got {}",
                            held.data_type()
                        ))
                    })?;
                    let child = structs.column(*index);
                    field = field
                        .get_field(*index)
                        .cloned()
                        .unwrap_or_else(|| Field::new(name.clone(), DataType::Null, true));
                    held = Arc::clone(child);
                }
                // A map entry, a list element, and every range need offsets
                // arithmetic the linked crates do not expose as one kernel, so
                // they are answered a row at a time - see the module note.
                other => {
                    return self.step_fallback(held.as_ref(), &field, column, other);
                }
            }
        }
        Ok(held)
    }

    /// Apply the remaining accessor steps a row at a time.
    fn step_fallback(
        &self,
        array: &dyn Array,
        field: &Field,
        column: &BoundColumn,
        from: &Step,
    ) -> Result<ArrayRef> {
        let rows = crate::arrow::array_to_value(field, array)?;
        let Some(rows) = rows.as_sequence() else {
            return Err(Error::IncompatibleSchema(
                "reading a column produced no rows".to_owned(),
            ));
        };
        let remaining: Vec<Step> = column
            .steps()
            .iter()
            .skip_while(|step| *step != from)
            .cloned()
            .collect();
        let values: Vec<Value> = rows
            .iter()
            .map(|row| super::eval::apply_steps(row, &remaining))
            .collect();
        let leaf = column.field().clone().with_nullable(true);
        values_to_array(&leaf, &values, self.batch.num_rows())
    }

    /// Bring two operands to one physical type so a kernel accepts them.
    ///
    /// Binding already made the two agree *logically*; this is the physical
    /// half - an `Int32` column beside an `Int64` literal - and it converts the
    /// scalar side, once per batch, never the column.
    fn aligned(&mut self, left: NodeId, right: NodeId) -> Result<(Held, Held)> {
        let left = self.held(left)?;
        let right = self.held(right)?;
        if left.data_type() == right.data_type() {
            return Ok((left, right));
        }
        let target = crate::DataType::from_arrow(left.data_type())?;
        let converted = match &right {
            Held::Pinned { array, .. } => pinned(target.cast_arrow_array(Arc::clone(array), true)?),
            Held::Full(array) => Held::Full(target.cast_arrow_array(Arc::clone(array), true)?),
        };
        Ok((left, converted))
    }

    /// Evaluate one node as a boolean mask.
    fn boolean(&mut self, id: NodeId) -> Result<BooleanArray> {
        let array = self.column(id)?;
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

    /// Combine every operand of a connective with Kleene logic.
    fn connective(&mut self, operands: &[NodeId], all: bool) -> Result<BooleanArray> {
        let mut held: Option<BooleanArray> = None;
        for operand in operands {
            let mask = self.boolean(*operand)?;
            held = Some(match held {
                Some(held) => kleene(&held, &mask, all),
                None => mask,
            });
        }
        Ok(held.unwrap_or_else(|| BooleanArray::from(vec![all; self.batch.num_rows()])))
    }

    /// Answer one node a row at a time, materializing only that node.
    fn row_fallback(&mut self, id: NodeId, node: &Node) -> Result<ArrayRef> {
        let rows = self.batch.num_rows();
        let mut children = Vec::new();
        node.for_each_child(|child| children.push(child));
        // Each operand is read once into values; the node itself then runs the
        // row evaluator per row, which is why only this node pays.
        let mut materialized: HashMap<NodeId, Vec<Value>> = HashMap::new();
        for child in children {
            let array = self.column(child)?;
            let data_type = self.plan.data_type(child).cloned().unwrap_or_else(|| {
                crate::DataType::from_arrow(array.data_type()).unwrap_or(DataType::Null)
            });
            let field = Field::new("operand", data_type, true);
            let sequence = crate::arrow::array_to_value(&field, array.as_ref())?;
            materialized.insert(
                child,
                sequence
                    .as_sequence()
                    .map(<[Value]>::to_vec)
                    .unwrap_or_default(),
            );
        }
        let mut values = Vec::with_capacity(rows);
        for row in 0..rows {
            values.push(super::eval::evaluate_with(
                self.plan,
                id,
                &|child: NodeId| {
                    materialized
                        .get(&child)
                        .and_then(|column| column.get(row))
                        .cloned()
                        .unwrap_or(Value::Null)
                },
            )?);
        }
        let data_type = self.plan.data_type(id).cloned().unwrap_or(DataType::Null);
        values_to_array(&Field::new("computed", data_type, true), &values, rows)
    }
}

/// Materialize a column of values under one field.
///
/// One pass through the crate's own schema-directed builder rather than one
/// one-row array per row plus a concatenation - the difference is a factor of
/// tens on the fallback path, which is the path that needed it most.
fn values_to_array(field: &Field, values: &[Value], rows: usize) -> Result<ArrayRef> {
    debug_assert_eq!(values.len(), rows, "one value per row");
    let borrowed: Vec<&Value> = values.iter().collect();
    crate::arrow::value::array_from_values(field, &borrowed)
}

/// Run one comparison kernel.
fn compare(op: super::CompareOp, left: &dyn Datum, right: &dyn Datum) -> Result<BooleanArray> {
    use super::CompareOp as C;
    let compared = match op {
        C::Eq => arrow_ord::cmp::eq(left, right),
        C::NotEq => arrow_ord::cmp::neq(left, right),
        C::Lt => arrow_ord::cmp::lt(left, right),
        C::LtEq => arrow_ord::cmp::lt_eq(left, right),
        C::Gt => arrow_ord::cmp::gt(left, right),
        C::GtEq => arrow_ord::cmp::gt_eq(left, right),
    };
    compared.map_err(Error::from)
}

/// Kleene `AND` or `OR` over two masks.
///
/// The identity that makes this eleven lines: a result is *known* when either
/// operand is known to settle it - false for `AND`, true for `OR` - or when
/// both operands are known; and its value is the ordinary bitwise answer over
/// the operands' known-true bits.
fn kleene(left: &BooleanArray, right: &BooleanArray, all: bool) -> BooleanArray {
    let left_valid = validity(left);
    let right_valid = validity(right);
    let left_true = left.values() & &left_valid;
    let right_true = right.values() & &right_valid;
    let (values, settled) = if all {
        let left_false = &left_valid & &!left.values();
        let right_false = &right_valid & &!right.values();
        (&left_true & &right_true, &left_false | &right_false)
    } else {
        (&left_true | &right_true, &left_true | &right_true)
    };
    let valid = &settled | &(&left_valid & &right_valid);
    BooleanArray::new(values, Some(NullBuffer::new(valid)))
}

/// Kleene negation: unknown negates to unknown.
fn kleene_not(mask: &BooleanArray) -> BooleanArray {
    BooleanArray::new(!mask.values(), mask.nulls().cloned())
}

/// The bits of a mask that are known, as a buffer.
fn validity(mask: &BooleanArray) -> BooleanBuffer {
    mask.nulls().map_or_else(
        || BooleanBuffer::new_set(mask.len()),
        |nulls| nulls.inner().clone(),
    )
}

/// `IS NULL` / `IS NOT NULL`, which are the two-valued operators.
fn null_test(array: &dyn Array, wanted: bool) -> BooleanArray {
    let nulls = array.nulls().map_or_else(
        || BooleanBuffer::new_set(array.len()),
        |nulls| nulls.inner().clone(),
    );
    // `nulls` is the *validity* buffer, so a set bit means a value is there.
    BooleanArray::new(if wanted { !&nulls } else { nulls }, None)
}

/// A literal prefix test over any text column.
fn prefix_test(array: &dyn Array, prefix: &str) -> Result<BooleanArray> {
    let field = Field::new("text", DataType::Utf8, true);
    let text = field.cast_arrow_array(arrow_array::make_array(array.to_data()), true)?;
    let text = text
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .ok_or_else(|| Error::IncompatibleSchema("a prefix test needs a text column".to_owned()))?;
    Ok(text
        .iter()
        .map(|value| value.map(|value| value.starts_with(prefix)))
        .collect())
}

/// A wildcard match over any text column, against a per-row pattern.
fn like_test(
    array: &dyn Array,
    pattern: &dyn Array,
    escape: Option<char>,
    negated: bool,
    case_insensitive: bool,
) -> Result<BooleanArray> {
    let field = Field::new("text", DataType::Utf8, true);
    let text = field.cast_arrow_array(arrow_array::make_array(array.to_data()), true)?;
    let patterns = field.cast_arrow_array(arrow_array::make_array(pattern.to_data()), true)?;
    let text = text
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .ok_or_else(|| Error::IncompatibleSchema("LIKE needs a text column".to_owned()))?;
    let patterns = patterns
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .ok_or_else(|| Error::IncompatibleSchema("LIKE needs a text pattern".to_owned()))?;
    Ok(text
        .iter()
        .zip(patterns.iter())
        .map(|(text, pattern)| {
            let (text, pattern) = (text?, pattern?);
            Some(super::eval::like_matches(text, pattern, escape, case_insensitive) != negated)
        })
        .collect())
}

/// Anything an expression can be applied to once Arrow is compiled in.
pub trait ArrowApplicable {
    /// What applying produces.
    type Output;

    /// The struct root that describes this carrier's rows.
    ///
    /// # Errors
    ///
    /// Returns an error when the carrier's schema cannot be read.
    fn carrier_schema(&self) -> crate::Result<Field>;

    /// Apply an already-lowered program.
    ///
    /// # Errors
    ///
    /// Returns whatever evaluating returns.
    fn run(self, program: &Program) -> Result<Self::Output>;
}

/// The Arrow half of the one apply verb.
///
/// Every [`Apply`] subject gets this for free, so the redirection table is
/// written once: an [`Expr`](super::Expr), a [`Bound`], a
/// [`Selection`](super::Selection), a `&str`, and the pair vocabulary all
/// reach every Arrow carrier without a second impl each.
pub trait ArrowApply: Apply {
    /// Apply to one Arrow column, given the field that describes it.
    ///
    /// This is the entry every other Arrow carrier is built from, and it is
    /// total over the type system: a datatype this expression cannot answer for
    /// is a typed refusal naming the datatype and the operation, never a panic
    /// and never a silently wrong array.
    ///
    /// # Errors
    ///
    /// Returns whatever binding or evaluating returns.
    fn apply_arrow_array(&self, field: &Field, array: ArrayRef) -> Result<ArrayRef> {
        let root = DataType::from_fields([field.clone()])
            .map_err(crate::arrow::Error::from)?
            .required_field("row");
        let schema = crate::arrow::schema_from_field(&root)?;
        let batch = RecordBatch::try_new(schema, vec![array])?;
        let program = self.program(&root).map_err(crate::arrow::Error::from)?;
        match &program {
            Program::Predicate(predicate) => Ok(Arc::new(predicate.mask(&batch)?)),
            Program::Compute(bound) => bound.evaluate_batch(&batch),
            Program::Project(selection) => {
                let projected = selection.project_batch(&batch)?;
                projected.columns().first().map(Arc::clone).ok_or_else(|| {
                    crate::arrow::Error::IncompatibleSchema(
                        "a projection over one column produced none".to_owned(),
                    )
                })
            }
        }
    }

    /// Apply to one pinned one-row Arrow value.
    ///
    /// # Errors
    ///
    /// Returns an error when the array does not hold exactly one row, plus
    /// whatever [`Self::apply_arrow_array`] returns.
    fn apply_arrow_scalar(
        &self,
        field: &Field,
        array: Scalar<ArrayRef>,
    ) -> Result<Scalar<ArrayRef>> {
        let inner = arrow_array::make_array(array.get().0.to_data());
        if inner.len() != 1 {
            return Err(crate::arrow::Error::IncompatibleSchema(format!(
                "a scalar takes exactly one row, got {}",
                inner.len()
            )));
        }
        Ok(Scalar::new(self.apply_arrow_array(field, inner)?))
    }

    /// Filter, compute, or project one batch.
    ///
    /// # Errors
    ///
    /// Returns whatever binding or evaluating returns.
    fn apply_arrow_batch(&self, batch: RecordBatch) -> Result<RecordBatch> {
        let root = crate::arrow::record_schema_from_arrow("row", batch.schema().as_ref())?;
        let program = self.program(&root).map_err(crate::arrow::Error::from)?;
        run_batch(&program, &batch)
    }

    /// Filter, compute, or project every batch of a stream, lazily.
    ///
    /// Binding happens when the stream is built, so the returned reader
    /// answers `schema()` with the result schema *before* the first batch is
    /// pulled, holds at most one batch, and never materializes a vector.
    ///
    /// # Errors
    ///
    /// Returns an error when the reader's schema disagrees with what this
    /// expression names, before the first batch is pulled.
    fn apply_arrow_batch_reader(&self, reader: BatchReader) -> Result<BatchReader> {
        let root = crate::arrow::record_schema_from_arrow("row", reader.schema().as_ref())?;
        let program = self.program(&root).map_err(crate::arrow::Error::from)?;
        match program {
            Program::Predicate(predicate) => predicate.filter_batch_reader(reader),
            Program::Project(selection) => selection.project_batch_reader(reader),
            Program::Compute(bound) => {
                let selection = super::Selection::from_exprs([bound.to_expr()]).bind(&root)?;
                selection.project_batch_reader(reader)
            }
        }
    }

    /// Apply to rows held as one struct column.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Self::apply_arrow_batch`] returns.
    fn apply_arrow_struct(&self, array: StructArray) -> Result<StructArray> {
        let batch = RecordBatch::from(array);
        Ok(StructArray::from(self.apply_arrow_batch(batch)?))
    }

    /// Apply to any Arrow carrier.
    ///
    /// # Errors
    ///
    /// Returns whatever the carrier's own application returns.
    fn apply_arrow<C: ArrowApplicable>(&self, carrier: C) -> Result<C::Output>
    where
        Self: Sized,
    {
        let schema = carrier.carrier_schema()?;
        let program = self.program(&schema).map_err(crate::arrow::Error::from)?;
        carrier.run(&program)
    }
}

impl<T: Apply + ?Sized> ArrowApply for T {}

/// Run one program over one batch.
fn run_batch(program: &Program, batch: &RecordBatch) -> Result<RecordBatch> {
    match program {
        Program::Predicate(predicate) => predicate.filter_batch(batch),
        Program::Project(selection) => selection.project_batch(batch),
        Program::Compute(bound) => {
            let array = bound.evaluate_batch(batch)?;
            let field = bound.field();
            let root = DataType::from_fields([field])
                .map_err(crate::arrow::Error::from)?
                .required_field("row");
            let schema = crate::arrow::schema_from_field(&root)?;
            RecordBatch::try_new(schema, vec![array]).map_err(crate::arrow::Error::from)
        }
    }
}

impl ArrowApplicable for RecordBatch {
    type Output = Self;

    fn carrier_schema(&self) -> crate::Result<Field> {
        crate::arrow::record_schema_from_arrow("row", self.schema().as_ref()).map_err(|error| {
            crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: smol_str::format_smolstr!("{error}"),
            }
        })
    }

    fn run(self, program: &Program) -> Result<Self::Output> {
        run_batch(program, &self)
    }
}

impl ArrowApplicable for StructArray {
    type Output = Self;

    fn carrier_schema(&self) -> crate::Result<Field> {
        RecordBatch::from(self.clone()).carrier_schema()
    }

    fn run(self, program: &Program) -> Result<Self::Output> {
        Ok(Self::from(run_batch(program, &RecordBatch::from(self))?))
    }
}

impl ArrowApplicable for BatchReader {
    type Output = Self;

    fn carrier_schema(&self) -> crate::Result<Field> {
        crate::arrow::record_schema_from_arrow("row", self.schema().as_ref()).map_err(|error| {
            crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: smol_str::format_smolstr!("{error}"),
            }
        })
    }

    fn run(self, program: &Program) -> Result<Self::Output> {
        match program {
            Program::Predicate(predicate) => predicate.filter_batch_reader(self),
            Program::Project(selection) => selection.project_batch_reader(self),
            Program::Compute(bound) => {
                let root = bound.schema().clone();
                let selection = super::Selection::from_exprs([bound.to_expr()]).bind(&root)?;
                selection.project_batch_reader(self)
            }
        }
    }
}

/// One column beside the field that describes it.
impl ArrowApplicable for (&Field, ArrayRef) {
    type Output = ArrayRef;

    fn carrier_schema(&self) -> crate::Result<Field> {
        Ok(DataType::from_fields([self.0.clone()])?.required_field("row"))
    }

    fn run(self, program: &Program) -> Result<Self::Output> {
        let (field, array) = self;
        let root = DataType::from_fields([field.clone()])
            .map_err(crate::arrow::Error::from)?
            .required_field("row");
        let schema = crate::arrow::schema_from_field(&root)?;
        let batch = RecordBatch::try_new(schema, vec![array])?;
        match program {
            Program::Predicate(predicate) => Ok(Arc::new(predicate.mask(&batch)?)),
            Program::Compute(bound) => bound.evaluate_batch(&batch),
            Program::Project(selection) => selection
                .project_batch(&batch)?
                .columns()
                .first()
                .map(Arc::clone)
                .ok_or_else(|| {
                    crate::arrow::Error::IncompatibleSchema(
                        "a projection over one column produced none".to_owned(),
                    )
                }),
        }
    }
}

/// A carrier that holds no data at all: just the shape of the answer.
impl ArrowApplicable for &SchemaRef {
    type Output = SchemaRef;

    fn carrier_schema(&self) -> crate::Result<Field> {
        crate::arrow::record_schema_from_arrow("row", self.as_ref()).map_err(|error| {
            crate::Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: smol_str::format_smolstr!("{error}"),
            }
        })
    }

    fn run(self, program: &Program) -> Result<Self::Output> {
        crate::arrow::schema_from_field(&program.result_root())
    }
}
