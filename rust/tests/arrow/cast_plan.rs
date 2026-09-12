//! One compiled plan, many batches, and what a reader over it owes its caller.
//!
//! The plan is the schema-dependent half of a cast made once. These cases pin
//! what that buys and what it must not cost: an exact cast still hands the
//! caller's own object back, a reader plans once for a whole stream, a failure
//! reaches the caller when the batch it belongs to is pulled, and the source is
//! released the moment it can yield nothing more.

use std::sync::Arc;

use super::root;
use arrow_array::{ArrayRef, Int32Array, Int64Array, RecordBatch, RecordBatchReader, StringArray};
use arrow_schema::{ArrowError, DataType as ArrowDataType, Field as ArrowField, Schema, SchemaRef};
use yggdryl::arrow::{BatchReader, cast_reader};
use yggdryl::{ArrowCast, ArrowCastOptions, ArrowCastPlan, DataType, Field, Nullability};

fn stored() -> SchemaRef {
    Arc::new(Schema::new(vec![
        ArrowField::new("id", ArrowDataType::Int32, false),
        ArrowField::new("symbol", ArrowDataType::Utf8, true),
    ]))
}

fn batch(offset: i32) -> RecordBatch {
    RecordBatch::try_new(
        stored(),
        vec![
            Arc::new(Int32Array::from(vec![offset, offset + 1])),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
        ],
    )
    .unwrap()
}

fn target() -> Field {
    root([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
}

/// A reader that counts what has been pulled and refuses to be pulled again
/// once it has been dropped - which is how "released early" is observed.
struct Counted {
    schema: SchemaRef,
    remaining: usize,
    pulled: Arc<std::sync::atomic::AtomicUsize>,
}

impl Iterator for Counted {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        self.pulled
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Some(Ok(batch(0)))
    }
}

impl RecordBatchReader for Counted {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.pulled
            .fetch_add(1_000, std::sync::atomic::Ordering::Relaxed);
    }
}

fn counted(batches: usize) -> (BatchReader, Arc<std::sync::atomic::AtomicUsize>) {
    let pulled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let reader = Counted {
        schema: stored(),
        remaining: batches,
        pulled: Arc::clone(&pulled),
    };
    (Box::new(reader), pulled)
}

#[test]
fn one_plan_answers_every_batch_of_its_schema() {
    let plan = ArrowCastPlan::compile(&stored(), &target(), ArrowCastOptions::new()).unwrap();
    assert_eq!(plan.as_schema().field(0).data_type(), &ArrowDataType::Int64);
    assert_eq!(plan.as_field(), &target());
    assert_eq!(plan.as_source_schema().as_ref(), stored().as_ref());
    assert_eq!(plan.as_options(), &ArrowCastOptions::new());

    // Reusing the plan and casting each batch on its own are the same answer.
    for offset in 0..4 {
        let source = batch(offset);
        let planned = plan.apply(source.clone()).unwrap();
        let alone = target()
            .cast_arrow_batch(source, ArrowCastOptions::new())
            .unwrap();
        assert_eq!(planned, alone);
    }
}

#[test]
fn a_plan_refuses_a_batch_of_another_schema() {
    let plan = ArrowCastPlan::compile(&stored(), &target(), ArrowCastOptions::new()).unwrap();
    let other = RecordBatch::try_new(
        Arc::new(Schema::new(vec![ArrowField::new(
            "id",
            ArrowDataType::Int64,
            false,
        )])),
        vec![Arc::new(Int64Array::from(vec![1]))],
    )
    .unwrap();

    let message = plan.apply(other).unwrap_err().to_string();
    assert!(
        message.contains("differs from the schema this cast plan was compiled for"),
        "{message}"
    );
}

#[test]
fn an_exact_plan_hands_the_caller_its_own_batch_back() {
    let exact = target();
    let schema = exact.clone().into_arrow_schema().unwrap();
    let source = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        ],
    )
    .unwrap();

    let plan = ArrowCastPlan::compile(&schema, &exact, ArrowCastOptions::new()).unwrap();
    let cast = plan.apply(source.clone()).unwrap();

    // Not merely equal: the same schema and the same column allocations.
    assert!(Arc::ptr_eq(&cast.schema(), &source.schema()));
    for (before, after) in source.columns().iter().zip(cast.columns()) {
        assert!(Arc::ptr_eq(before, after));
    }
}

#[test]
fn preflight_reports_a_schema_failure_and_leaves_the_row_failures_alone() {
    let missing = root([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("venue"),
    ]);
    let strict = ArrowCastOptions::new().with_nullability(Nullability::Strict);

    // A required column no source carries is a schema failure, so it never
    // reaches preflight: compiling already refused it.
    assert!(ArrowCastPlan::compile(&stored(), &missing, strict).is_err());

    // A null in a required column is a row failure, so an empty preflight
    // passes and the refusal waits for a batch that has rows.
    let required = root([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("symbol"),
    ]);
    let plan = ArrowCastPlan::compile(&stored(), &required, strict).unwrap();
    plan.preflight().unwrap();

    let with_null = RecordBatch::try_new(
        stored(),
        vec![
            Arc::new(Int32Array::from(vec![1])),
            Arc::new(StringArray::from(vec![None::<&str>])),
        ],
    )
    .unwrap();
    assert!(plan.apply(with_null).is_err());
}

#[test]
fn a_reader_plans_once_and_casts_when_a_batch_is_pulled() {
    let (inner, pulled) = counted(3);
    let reader = cast_reader(inner, &target(), ArrowCastOptions::new()).unwrap();

    // The schema is answered before anything is pulled.
    assert_eq!(reader.schema().field(0).data_type(), &ArrowDataType::Int64);
    assert_eq!(pulled.load(std::sync::atomic::Ordering::Relaxed), 0);

    let batches: Vec<_> = reader.collect::<std::result::Result<_, _>>().unwrap();
    assert_eq!(batches.len(), 3);
    // Three pulls, then the drop marker: nothing was collected up front and
    // the source did not outlive its last batch.
    assert_eq!(pulled.load(std::sync::atomic::Ordering::Relaxed), 1_003);
}

#[test]
fn an_exact_reader_is_the_reader_itself() {
    let exact = Field::new(
        "row",
        DataType::from_fields([
            DataType::Int32.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
        ])
        .unwrap(),
        false,
    );
    let (inner, pulled) = counted(1);
    let reader = cast_reader(inner, &exact, ArrowCastOptions::new()).unwrap();
    drop(reader);

    // Nothing wrapped it, so dropping it dropped the source directly.
    assert_eq!(pulled.load(std::sync::atomic::Ordering::Relaxed), 1_000);
}

#[test]
fn a_strict_reader_reports_its_batch_at_the_pull_and_then_fuses() {
    let broken = RecordBatch::try_new(
        stored(),
        vec![
            Arc::new(Int32Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        ],
    )
    .unwrap();
    let required = root([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("symbol"),
    ]);
    let inner = yggdryl::arrow::batch_reader(stored(), [batch(0), broken, batch(9)]);
    let mut reader = cast_reader(
        inner,
        &required,
        ArrowCastOptions::new().with_nullability(Nullability::Strict),
    )
    .unwrap();

    assert!(reader.next().unwrap().is_ok());
    let error = reader.next().unwrap().unwrap_err();
    assert!(error.to_string().contains("$.symbol"), "{error}");
    // Fused: the third batch is never asked for, because the stream already
    // reported that it could not be honoured.
    assert!(reader.next().is_none());
}

#[test]
fn dropping_a_reader_early_releases_the_source() {
    let (inner, pulled) = counted(100);
    let mut reader = cast_reader(inner, &target(), ArrowCastOptions::new()).unwrap();
    assert!(reader.next().unwrap().is_ok());
    drop(reader);

    // One pull, then the drop marker: the other 99 batches were never decoded.
    assert_eq!(pulled.load(std::sync::atomic::Ordering::Relaxed), 1_001);
}

#[test]
fn a_compiled_plan_crosses_threads() {
    fn assert_send_sync<T: Send + Sync>(_: &T) {}

    let plan =
        Arc::new(ArrowCastPlan::compile(&stored(), &target(), ArrowCastOptions::new()).unwrap());
    assert_send_sync(&plan);

    let handles: Vec<_> = (0..4)
        .map(|offset| {
            let plan = Arc::clone(&plan);
            std::thread::spawn(move || plan.apply(batch(offset)).unwrap().num_rows())
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), 2);
    }
}

#[test]
fn a_cast_column_is_the_only_thing_a_plan_rebuilds() {
    let plan = ArrowCastPlan::compile(&stored(), &target(), ArrowCastOptions::new()).unwrap();
    let source = batch(0);
    let cast = plan.apply(source.clone()).unwrap();

    // `id` widened, so it is a new array; `symbol` was already exact and is
    // the very allocation the caller handed over.
    assert!(!Arc::ptr_eq(cast.column(0), source.column(0)));
    assert!(Arc::ptr_eq(cast.column(1), source.column(1)));
    let ids: &Int64Array = cast.column(0).as_any().downcast_ref().unwrap();
    assert_eq!(ids.values(), &[0, 1]);
    let _: &ArrayRef = cast.column(1);
}
