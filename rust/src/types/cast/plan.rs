//! One record-batch cast, compiled once and applied to many batches.
//!
//! Everything a cast decides from two schemas - which source column answers
//! which target field, the order the children come back in, the recursive type
//! dispatch, the target schema, and the kernel options - is a function of the
//! schemas alone. A reader whose batches all carry one schema was nonetheless
//! paying for that decision on every batch, because the entry points construct
//! the recursive plan inline from `array.data_type()`. [`ArrowCastPlan`] is
//! that work lifted out: compile once, apply per batch, and let only the
//! masks, offsets, and dictionary reachability a batch actually carries vary.

use std::fmt;
use std::sync::Arc;

use arrow_array::{RecordBatch, RecordBatchOptions, StructArray};
use arrow_schema::{DataType as ArrowDataType, Schema, SchemaRef};

use crate::Field;
use crate::arrow::{Error, Result, arrow_schema_from_field};
use crate::types::budget::MaterializationBudget;

use super::{ArrayCastPlan, ArrowCastOptions, Deferred, downcast};

/// A compiled Arrow record-batch cast: immutable, shareable, and reusable.
///
/// ```
/// use std::sync::Arc;
///
/// use arrow_array::{ArrayRef, Int32Array, RecordBatch};
/// use yggdryl::{ArrowCastOptions, ArrowCastPlan, DataType};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let root = DataType::from_fields([DataType::Int64.required_field("id")])?
///     .required_field("row");
/// let source = RecordBatch::try_from_iter([(
///     "id",
///     Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef,
/// )])?;
///
/// // The schemas decide everything; the plan is compiled from them alone.
/// let plan = ArrowCastPlan::compile(source.schema_ref(), &root, ArrowCastOptions::new())?;
/// assert_eq!(plan.as_schema().field(0).name(), "id");
///
/// // The same plan answers every batch of that schema.
/// let cast = plan.apply(source)?;
/// assert_eq!(cast.column(0).data_type(), &arrow_schema::DataType::Int64);
/// # Ok(())
/// # }
/// ```
pub struct ArrowCastPlan {
    field: Field,
    source: SchemaRef,
    schema: SchemaRef,
    options: ArrowCastOptions,
    root: ArrayCastPlan,
}

impl ArrowCastPlan {
    /// Compiles the cast from one source schema to one non-null Struct root.
    ///
    /// Every failure the two schemas alone can produce - an unsupported
    /// conversion, an ambiguous case-insensitive name, a required field no
    /// source column carries under
    /// [`Nullability::Strict`](super::Nullability::Strict) - is raised here,
    /// before a single batch exists.
    ///
    /// # Errors
    ///
    /// Returns an error unless `target` is a bounded, non-nullable Struct
    /// root, or when the recursive cast cannot be planned.
    pub fn compile(source: &Schema, target: &Field, options: ArrowCastOptions) -> Result<Self> {
        Self::compile_deferring(source, target, options, Deferred::default())
    }

    /// Compiles the cast, leaving the columns a named protocol still has to
    /// materialize out of the strict nullability check.
    pub(crate) fn compile_deferring(
        source: &Schema,
        target: &Field,
        options: ArrowCastOptions,
        deferred: Deferred,
    ) -> Result<Self> {
        target.validate_bounded()?;
        if target.is_nullable() {
            return Err(Error::IncompatibleSchema(
                "record-batch cast target Struct Field must be non-nullable".to_owned(),
            ));
        }
        if target.dtype().as_fields().is_none() {
            return Err(Error::IncompatibleSchema(format!(
                "record-batch cast target {:?} must have a struct datatype",
                target.name()
            )));
        }
        let source_type = ArrowDataType::Struct(source.fields().clone());
        let root = ArrayCastPlan::new_root(target, &source_type, options, deferred)?;
        Ok(Self {
            field: target.clone(),
            source: Arc::new(source.clone()),
            schema: arrow_schema_from_field(target)?,
            options,
            root,
        })
    }

    /// The root Field every batch is reconciled to.
    pub const fn as_field(&self) -> &Field {
        &self.field
    }

    /// The schema batches must carry to be applied.
    pub const fn as_source_schema(&self) -> &SchemaRef {
        &self.source
    }

    /// The schema every applied batch carries, target metadata included.
    pub const fn as_schema(&self) -> &SchemaRef {
        &self.schema
    }

    /// The conversion and nullability policy this plan was compiled under.
    pub const fn as_options(&self) -> &ArrowCastOptions {
        &self.options
    }

    /// Runs the compiled cast over no rows.
    ///
    /// A caller planning a read rejects an impossible cast here rather than on
    /// the first batch: an empty batch of the source schema exercises the whole
    /// recursive plan without materializing a value. Row-dependent refusals -
    /// a null in a required column, a value the target cannot hold - have no
    /// rows to find and stay for [`Self::apply`].
    ///
    /// # Errors
    ///
    /// Returns any error the cast raises with no rows to read.
    pub fn preflight(&self) -> Result<()> {
        self.apply(RecordBatch::new_empty(Arc::clone(&self.source)))
            .map(|_| ())
    }

    /// Reconciles one batch of the source schema to the target root.
    ///
    /// An exact batch comes back as itself: the same `RecordBatch`, holding the
    /// same column `Arc`s, not a rebuilt one carrying equal arrays.
    ///
    /// # Errors
    ///
    /// Returns an error when the batch does not carry the schema this plan was
    /// compiled for, when a value cannot be converted, or when a required field
    /// is null under [`Nullability::Strict`](super::Nullability::Strict).
    pub fn apply(&self, batch: RecordBatch) -> Result<RecordBatch> {
        if batch.schema_ref().fields() != self.source.fields() {
            return Err(Error::IncompatibleSchema(format!(
                "batch schema {:?} differs from the schema this cast plan was compiled for, {:?}",
                batch.schema_ref().fields(),
                self.source.fields()
            )));
        }
        let row_count = batch.num_rows();
        let source = super::struct_array_from_batch(batch.clone());
        let mut budget = MaterializationBudget::default();
        let cast = self.root.cast(source, &mut budget)?;
        let columns = downcast::<StructArray>(cast.as_ref())?.columns();

        // Ownership allows handing the caller's own batch back only when every
        // column survived by pointer and the batch already declares the target
        // schema; anything else would be a different batch wearing that claim.
        if batch.schema_ref().as_ref() == self.schema.as_ref()
            && columns.len() == batch.num_columns()
            && columns
                .iter()
                .zip(batch.columns())
                .all(|(cast, source)| Arc::ptr_eq(cast, source))
        {
            return Ok(batch);
        }

        // A batch carries its row count even with no columns, which a struct
        // array cannot, so the count is restored explicitly.
        let options = RecordBatchOptions::new().with_row_count(Some(row_count));
        RecordBatch::try_new_with_options(Arc::clone(&self.schema), columns.to_vec(), &options)
            .map_err(Into::into)
    }

    /// Whether the plan hands every batch of its source schema straight back.
    ///
    /// A reader over such a plan is the reader itself, so nothing wraps it.
    /// Strictness is what makes an equal schema not enough: a non-null Arrow
    /// field can still carry a logical null inside a nested child, and refusing
    /// that is the whole point of asking.
    pub(crate) fn is_identity(&self) -> bool {
        !self.options.nullability().is_strict() && self.source.as_ref() == self.schema.as_ref()
    }
}

/// The plan by what decided it, not by the tree that decision produced.
///
/// The recursive dispatch is an implementation detail whose rendering would
/// bury the three things a reader of a failed plan needs: what it targets,
/// what it accepts, and under which policy.
impl fmt::Debug for ArrowCastPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArrowCastPlan")
            .field("field", &self.field)
            .field("source", &self.source)
            .field("schema", &self.schema)
            .field("options", &self.options)
            .finish()
    }
}

// A compiled plan is schema-derived state with no interior mutation on the
// cast path, so one plan serves every batch of a reader and every thread of a
// parallel scan. The assertion is here because nothing else would notice if a
// future plan field quietly gave that up.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ArrowCastPlan>();
};
