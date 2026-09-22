//! UUID packing without hashing or a clock, the canonical rendering, the
//! datatype through the doors a caller uses, and the cast both ways between
//! an identifier column and the string and byte datatypes it reads into;
//! allocations are pinned separately.

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{ArrayRef, FixedSizeBinaryArray, RecordBatch, StringArray};
use criterion::{BenchmarkId, Criterion, Throughput};
use yggdryl::FieldValue as _;
use yggdryl::Uuid;
use yggdryl::{ArrowCastOptions, DataType, Field, StructType};

use super::doors;

const ROWS: usize = crate::bench_profile::corpus(10_000, 1_024);

fn root(field: Field) -> Field {
    Field::new(
        "row",
        DataType::from(StructType::from_fields([field]).expect("the benchmark fields are valid")),
        false,
    )
}

/// The `index`th identifier of the corpus: time-ordered, so the column is
/// what a stored one looks like.
fn identifier(index: usize) -> Uuid {
    Uuid::from_v7(
        1_645_557_742_000_000 + index as i64,
        index as u64,
        index as u64,
    )
    .expect("an in-range instant")
}

pub(crate) fn uuid_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("uuid");
    group.bench_function("from_v7", |bencher| {
        bencher.iter(|| {
            Uuid::from_v7(
                black_box(1_645_557_742_000_123),
                black_box(0x74b),
                black_box(0xfedc_ba98_7654_3210),
            )
            .expect("an in-range microsecond instant")
        });
    });
    group.bench_function("from_v8", |bencher| {
        bencher.iter(|| Uuid::from_v8(black_box(0x5c14_6b14_3c52_4afd_938a_375d_0df1_fbf6)));
    });

    // The canonical spelling into a caller's slot, which is what every
    // writer that wants a `&str` pays, against the owned string it replaces.
    let value = Uuid::from_v8(0x5c14_6b14_3c52_4afd_938a_375d_0df1_fbf6);
    group.bench_function("render_into_slot", |bencher| {
        let mut slot = [0_u8; Uuid::TEXT_LEN];
        bencher.iter(|| black_box(value).render(black_box(&mut slot)).len());
    });
    group.bench_function("render_to_string", |bencher| {
        bencher.iter(|| black_box(value).to_string());
    });

    // The datatype through every door.
    doors::leaf_doors(
        &mut group,
        &DataType::Uuid,
        &identifier(0).to_string().into(),
    );

    // The column both ways: text into the identifier's sixteen bytes, those
    // bytes back in under the one rule, and out again as every reading the
    // two families offer.
    let strict = ArrowCastOptions::new().with_safe(false);
    group.throughput(Throughput::Elements(ROWS as u64));
    let spellings: Vec<String> = (0..ROWS)
        .map(|index| identifier(index).to_string())
        .collect();
    let text: ArrayRef = Arc::new(StringArray::from_iter_values(spellings.iter()));
    let id = DataType::Uuid.required_field("id");
    group.bench_function("text_ingest", |bencher| {
        bencher.iter(|| {
            black_box(&id)
                .cast_arrow_array(Arc::clone(&text), strict)
                .expect("every spelling is an identifier")
        });
    });

    let stored = id
        .cast_arrow_array(Arc::clone(&text), strict)
        .expect("every spelling is an identifier");
    let raw: ArrayRef = Arc::new(
        stored
            .as_any()
            .downcast_ref::<FixedSizeBinaryArray>()
            .expect("an identifier column is its sixteen bytes")
            .clone(),
    );
    group.bench_function("bytes_ingest", |bencher| {
        bencher.iter(|| {
            black_box(&id)
                .cast_arrow_array(Arc::clone(&raw), strict)
                .expect("sixteen bytes are an identifier")
        });
    });

    // The stored column under its own root's schema, so each render sees
    // the `arrow.uuid` identity exactly as a stored column carries it.
    let batch = RecordBatch::try_new(
        root(id.clone())
            .into_arrow_schema()
            .expect("the benchmark root is valid"),
        vec![stored],
    )
    .expect("the stored column matches its schema");
    for spelling in ["utf8", "ascii", "utf8(36)", "binary"] {
        let target = root(
            DataType::from_str(spelling)
                .expect("the static spelling must parse")
                .required_field("id"),
        );
        group.bench_function(BenchmarkId::new("render", spelling), |bencher| {
            bencher.iter(|| {
                black_box(&target)
                    .cast_arrow_batch(batch.clone(), strict)
                    .expect("the stored identifiers are valid")
            });
        });
    }
    group.finish();
}
