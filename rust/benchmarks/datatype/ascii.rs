//! The ASCII widths: parse and display, Arrow projection, and the cast plan
//! both ways over a 10k-row currency column.

use std::hint::black_box;
use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, StringArray};
use criterion::{BatchSize, Criterion, Throughput};
use yggdryl::{ArrowCast, AsciiDictionary, DataType, Field};

const ROWS: usize = 10_000;

/// Distinct four-byte values, so a miss benchmark measures the append.
const MISSES: usize = 64;

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
                DataType::from_str(black_box("ascii32")).expect("the static spelling must parse");
            DataType::from_str(black_box(&dtype.to_string()))
                .expect("canonical display output must round-trip")
        });
    });
    group.bench_function("field_arrow_projection", |bencher| {
        let field = DataType::Ascii32.required_field("ccy");
        bencher.iter(|| {
            black_box(&field)
                .clone()
                .into_arrow()
                .expect("the benchmark field is valid")
        });
    });

    let codes = ["USD", "EUR", "JPY", "GBP"];

    // The per-column vocabulary: a hit is one lookup, a miss appends. The
    // miss batch starts from an empty vocabulary so the growth is measured
    // rather than amortized away by the earlier iterations.
    let misses: Vec<String> = (0..MISSES).map(|index| format!("{index:04}")).collect();
    group.bench_function("dictionary_push_hit", |bencher| {
        let mut dictionary = AsciiDictionary::from_values(DataType::Ascii32, codes)
            .expect("the benchmark codes fit ascii32");
        bencher.iter(|| {
            dictionary
                .push(black_box("JPY"))
                .expect("the code is registered")
        });
    });
    group.bench_function("dictionary_push_miss", |bencher| {
        let empty = AsciiDictionary::new(DataType::Ascii32).expect("ascii32 is a width");
        bencher.iter_batched_ref(
            || empty.clone(),
            |dictionary| {
                for value in &misses {
                    dictionary
                        .push(black_box(value))
                        .expect("the value fits the width");
                }
            },
            BatchSize::SmallInput,
        );
    });
    let text: ArrayRef = Arc::new(StringArray::from_iter_values(
        (0..ROWS).map(|index| codes[index % codes.len()]),
    ));
    let target = DataType::Ascii32.required_field("ccy");
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
        root([DataType::Ascii32.required_field("ccy")])
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
    // Encoding the same 10k-row column as codes over the four-value
    // vocabulary: every value is a hit, so this is the steady state a caller
    // reaches after the first batch registered the column.
    group.bench_function("dictionary_into_arrow_array", |bencher| {
        let column: Vec<Option<&str>> = (0..ROWS)
            .map(|index| Some(codes[index % codes.len()]))
            .collect();
        let mut dictionary = AsciiDictionary::from_values(DataType::Ascii32, codes)
            .expect("the benchmark codes fit ascii32");
        bencher.iter(|| {
            dictionary
                .into_arrow_array(black_box(&column).iter().copied())
                .expect("the codes fit the width")
        });
    });
    group.finish();
}
