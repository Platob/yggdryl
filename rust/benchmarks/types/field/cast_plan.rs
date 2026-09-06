//! What compiling a cast once buys, measured against compiling it per batch.
//!
//! The two arms answer the same question about the same batches, so the only
//! difference between them is where the schema-dependent work happens: once,
//! or once per batch. Small batches are the honest corpus for that - a batch
//! wide enough to dominate the plan would hide it - so 1, 10, and 1,000 of
//! them are measured, which is also the range a streamed read actually pulls.
//!
//! The last group is a gate rather than a measurement: it compares the two
//! warmed medians and refuses a build where reusing the plan is slower than
//! rebuilding it, because that would mean the plan has started carrying
//! per-batch work it has no business carrying.

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arrow_array::{ArrayRef, Int32Array, RecordBatch, StringArray};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema, SchemaRef};
use criterion::{Criterion, Throughput};
use yggdryl::{ArrowCast, ArrowCastOptions, ArrowCastPlan, DataType, Field};

/// Rows per batch: small on purpose, so the per-batch plan is what is timed.
const ROWS: usize = crate::bench_profile::corpus(64, 8);
/// Batch counts a streamed read actually pulls.
const COUNTS: [usize; 3] = [1, 10, crate::bench_profile::corpus(1_000, 32)];

fn stored() -> SchemaRef {
    Arc::new(Schema::new(vec![
        ArrowField::new("id", ArrowDataType::Int32, false),
        ArrowField::new("symbol", ArrowDataType::Utf8, true),
        ArrowField::new("unused", ArrowDataType::Int32, true),
    ]))
}

fn target() -> Field {
    Field::new(
        "row",
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("symbol"),
            DataType::Utf8.required_field("venue"),
        ])
        .expect("the benchmark root is valid"),
        false,
    )
}

fn batches(count: usize) -> Vec<RecordBatch> {
    let schema = stored();
    (0..count)
        .map(|index| {
            let base = i32::try_from(index).expect("the benchmark batch count fits an i32");
            let ids: Vec<i32> = (0..ROWS)
                .map(|row| base + i32::try_from(row).expect("the row index fits an i32"))
                .collect();
            let symbols: Vec<&str> = (0..ROWS)
                .map(|row| if row % 3 == 0 { "AAPL" } else { "MSFT" })
                .collect();
            RecordBatch::try_new(
                Arc::clone(&schema),
                vec![
                    Arc::new(Int32Array::from(ids)) as ArrayRef,
                    Arc::new(StringArray::from(symbols)) as ArrayRef,
                    Arc::new(Int32Array::from(vec![0; ROWS])) as ArrayRef,
                ],
            )
            .expect("the benchmark batch matches its schema")
        })
        .collect()
}

/// Compile once, then apply the plan to every batch.
fn compiled(root: &Field, source: &SchemaRef, batches: &[RecordBatch]) -> usize {
    let plan = ArrowCastPlan::compile(source, root, ArrowCastOptions::new())
        .expect("the benchmark cast is plannable");
    batches
        .iter()
        .map(|batch| {
            plan.apply(batch.clone())
                .expect("the benchmark batch fits the plan")
                .num_rows()
        })
        .sum()
}

/// Plan from the batch's own schema every time, as the entry point does.
fn per_batch(root: &Field, batches: &[RecordBatch]) -> usize {
    batches
        .iter()
        .map(|batch| {
            root.cast_arrow_batch(batch.clone(), ArrowCastOptions::new())
                .expect("the benchmark batch fits the root")
                .num_rows()
        })
        .sum()
}

/// The warmed median of `work` over `samples` runs.
fn median(samples: usize, mut work: impl FnMut()) -> Duration {
    for _ in 0..samples.min(8) {
        work();
    }
    let mut timings: Vec<Duration> = (0..samples)
        .map(|_| {
            let started = Instant::now();
            work();
            started.elapsed()
        })
        .collect();
    timings.sort_unstable();
    timings[timings.len() / 2]
}

pub fn benchmarks(criterion: &mut Criterion) {
    let root = target();
    let source = stored();

    let mut group = criterion.benchmark_group("cast_plan");
    for count in COUNTS {
        let corpus = batches(count);
        // Both arms return the same rows from the same batches; the assertion
        // is what makes the comparison a comparison rather than two numbers.
        let compiled_rows = compiled(&root, &source, &corpus);
        assert_eq!(
            compiled_rows,
            per_batch(&root, &corpus),
            "the two cast paths must answer the same rows"
        );
        assert_eq!(compiled_rows, count * ROWS);

        group.throughput(Throughput::Elements((count * ROWS) as u64));
        group.bench_function(format!("compiled_once/{count}"), |bencher| {
            bencher.iter(|| compiled(black_box(&root), black_box(&source), black_box(&corpus)));
        });
        group.bench_function(format!("planned_per_batch/{count}"), |bencher| {
            bencher.iter(|| per_batch(black_box(&root), black_box(&corpus)));
        });
    }
    group.finish();

    // The gate. A compiled plan does strictly less work than planning per
    // batch, so at every batch count it must not be slower; the margin absorbs
    // the scheduler noise a single-batch run is dominated by.
    for count in COUNTS {
        let corpus = batches(count);
        let samples = if count > 100 { 15 } else { 50 };
        let reused = median(samples, || {
            black_box(compiled(&root, &source, &corpus));
        });
        let rebuilt = median(samples, || {
            black_box(per_batch(&root, &corpus));
        });
        assert!(
            reused.as_secs_f64() <= rebuilt.as_secs_f64() * 1.25,
            "reusing one plan over {count} batches took {reused:?} against {rebuilt:?} for \
             planning each one"
        );
    }
}
