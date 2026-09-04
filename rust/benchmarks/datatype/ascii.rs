//! The ASCII widths: parse and display, Arrow projection, and the cast plan
//! both ways over a 10k-row currency column.

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, StringArray};
use criterion::{Criterion, Throughput};
use yggdryl::{ArrowCast, AsciiEnum, DataType, Field};

const ROWS: usize = 10_000;

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    Field::new(
        "row",
        DataType::from_fields(fields).expect("the benchmark fields are valid"),
        false,
    )
}

pub(crate) fn ascii_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("ascii");
    group.bench_function("parse_display_round_trip", |bencher| {
        bencher.iter(|| {
            let dtype =
                DataType::from_str(black_box("ascii(4)")).expect("the static spelling must parse");
            DataType::from_str(black_box(&dtype.to_string()))
                .expect("canonical display output must round-trip")
        });
    });
    group.bench_function("field_arrow_projection", |bencher| {
        let field = DataType::FixedAscii(4).required_field("ccy");
        bencher.iter(|| {
            black_box(&field)
                .clone()
                .into_arrow()
                .expect("the benchmark field is valid")
        });
    });

    // The prebuilt vocabularies: building one names every code in its
    // constant, which is what a schema pays once when it declares the column.
    group.bench_function("vocabulary_prebuilt", |bencher| {
        bencher.iter(|| {
            AsciiEnum::from_logical_name(black_box("mic"))
                .expect("mic is a registered logical name")
        });
    });

    let codes = ["USD", "EUR", "JPY", "GBP"];

    let text: ArrayRef = Arc::new(StringArray::from_iter_values(
        (0..ROWS).map(|index| codes[index % codes.len()]),
    ));
    let target = DataType::FixedAscii(4).required_field("ccy");
    group.throughput(Throughput::Elements(ROWS as u64));
    group.bench_function("utf8_to_ascii32_ingest", |bencher| {
        bencher.iter(|| {
            black_box(&target)
                .cast_arrow_array(Arc::clone(&text), false)
                .expect("the codes fit the width")
        });
    });

    // The padded column under the ASCII root's own schema, so the render
    // sees the extension identity exactly as a stored column carries it.
    let padded = target
        .cast_arrow_array(Arc::clone(&text), false)
        .expect("the codes fit the width");
    let batch = RecordBatch::try_new(
        root([DataType::FixedAscii(4).required_field("ccy")])
            .into_arrow_schema()
            .expect("the benchmark root is valid"),
        vec![padded],
    )
    .expect("the padded column matches its schema");
    let text_root = root([DataType::Utf8.required_field("ccy")]);
    group.bench_function("ascii32_to_utf8_render", |bencher| {
        bencher.iter(|| {
            black_box(&text_root)
                .cast_arrow_batch(batch.clone(), false)
                .expect("the stored codes are valid")
        });
    });
    // Naming the members of one declared vocabulary: the packed code of
    // every value, which is what a reader of the schema computes once.
    group.bench_function("vocabulary_into_members", |bencher| {
        let declared = AsciiEnum::from_logical_name("mic").expect("mic is registered");
        bencher.iter(|| {
            declared
                .into_members(black_box(&DataType::Mic))
                .expect("every prebuilt code fits its width")
        });
    });
    group.finish();
}
