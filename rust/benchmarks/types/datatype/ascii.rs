//! The US-ASCII strings and the registered codes: parse and display, Arrow
//! projection, the vocabulary a field declares, and the cast plan both ways
//! over a 10k-row currency column at each of the three widths a currency can
//! be stored under - the code, the fixed slot, and the bounded string.

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, StringArray};
use criterion::{BenchmarkId, Criterion, Throughput};
use yggdryl::{ArrowCast, ArrowCastOptions, DataType, Field, Scalar, StringEnum};

const ROWS: usize = crate::bench_profile::corpus(10_000, 1_024);

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    Field::new(
        "row",
        DataType::from_fields(fields).expect("the benchmark fields are valid"),
        false,
    )
}

/// The three columns a currency code is stored in: its own datatype, the
/// fixed four-byte slot, and the variable US-ASCII string bounded at four.
fn currency_columns() -> [(&'static str, DataType); 3] {
    [
        ("currency", DataType::Currency),
        (
            "fixed_ascii",
            DataType::fixed_ascii(4).expect("four bytes is a width"),
        ),
        (
            "ascii",
            DataType::from_str("ascii(4)").expect("four bytes is a bound"),
        ),
    ]
}

pub(crate) fn ascii_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("ascii");
    for spelling in ["fixed_ascii(4)", "ascii(4)", "currency"] {
        group.bench_function(
            BenchmarkId::new("parse_display_round_trip", spelling),
            |bencher| {
                bencher.iter(|| {
                    let dtype = DataType::from_str(black_box(spelling))
                        .expect("the static spelling must parse");
                    DataType::from_str(black_box(&dtype.to_string()))
                        .expect("canonical display output must round-trip")
                });
            },
        );
    }
    for (name, dtype) in currency_columns() {
        let field = dtype.required_field("ccy");
        group.bench_function(
            BenchmarkId::new("field_arrow_projection", name),
            |bencher| {
                bencher.iter(|| {
                    black_box(&field)
                        .clone()
                        .into_arrow()
                        .expect("the benchmark field is valid")
                });
            },
        );
    }

    // One code through the value door: the text a row carries becomes the
    // code its column declares, and the string the fixed slot declares.
    let text = Scalar::from("USD");
    for (name, dtype) in currency_columns() {
        group.bench_function(BenchmarkId::new("scalar_from_text", name), |bencher| {
            bencher.iter(|| {
                black_box(&dtype)
                    .scalar(black_box(text.clone()))
                    .expect("a currency code fits every width here")
            });
        });
    }

    // The prebuilt vocabularies: building one names every code in its
    // constant, which is what a schema pays once when it declares the column.
    group.bench_function("vocabulary_prebuilt", |bencher| {
        bencher.iter(|| {
            StringEnum::from_logical_name(black_box("mic"))
                .expect("mic is a registered logical name")
        });
    });

    let codes = ["USD", "EUR", "JPY", "GBP"];
    let column: ArrayRef = Arc::new(StringArray::from_iter_values(
        (0..ROWS).map(|index| codes[index % codes.len()]),
    ));
    let text_root = root([DataType::utf8().required_field("ccy")]);
    group.throughput(Throughput::Elements(ROWS as u64));
    for (name, dtype) in currency_columns() {
        let target = dtype.required_field("ccy");
        group.bench_function(BenchmarkId::new("utf8_ingest", name), |bencher| {
            bencher.iter(|| {
                black_box(&target)
                    .cast_arrow_array(
                        Arc::clone(&column),
                        ArrowCastOptions::new().with_safe(false),
                    )
                    .expect("the codes fit the width")
            });
        });

        // The stored column under its own root's schema, so the render sees
        // the extension identity exactly as a stored column carries it.
        let stored = target
            .cast_arrow_array(
                Arc::clone(&column),
                ArrowCastOptions::new().with_safe(false),
            )
            .expect("the codes fit the width");
        let batch = RecordBatch::try_new(
            root([target.clone()])
                .into_arrow_schema()
                .expect("the benchmark root is valid"),
            vec![stored],
        )
        .expect("the stored column matches its schema");
        group.bench_function(BenchmarkId::new("utf8_render", name), |bencher| {
            bencher.iter(|| {
                black_box(&text_root)
                    .cast_arrow_batch(batch.clone(), ArrowCastOptions::new().with_safe(false))
                    .expect("the stored codes are valid")
            });
        });
    }
    // Naming the members of one declared vocabulary: the packed code of
    // every value, which is what a reader of the schema computes once.
    group.bench_function("vocabulary_into_members", |bencher| {
        let declared = StringEnum::from_logical_name("mic").expect("mic is registered");
        bencher.iter(|| {
            declared
                .into_members(black_box(&DataType::Mic))
                .expect("every prebuilt code fits its width")
        });
    });
    group.finish();
}
