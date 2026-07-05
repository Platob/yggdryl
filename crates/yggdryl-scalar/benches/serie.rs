//! Benchmarks for the serie scalars: buffer-backed construction and element
//! access, the generic-vs-concrete comparison, the zero-copy Arrow round trip and
//! the bulk core-IO bridge (`from_io` / `pwrite_io`).

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use yggdryl_scalar::yggdryl_core::{ByteBuffer, RawIOBase, Whence};
use yggdryl_scalar::yggdryl_dtype as dtype;
use yggdryl_scalar::{Int64Scalar, Int64Serie, Int8Serie, Scalar, TypedSerie, UInt8Serie};

type Int64SerieGeneric = TypedSerie<dtype::Int64Type, Int64Scalar>;

const N: usize = 4096;

fn serie(c: &mut Criterion) {
    let mut group = c.benchmark_group("serie");
    group.throughput(Throughput::Elements(N as u64));

    group.bench_function("int64_serie_from_vec", |b| {
        b.iter_batched(
            || (0..N as i64).collect::<Vec<_>>(),
            |values| black_box(Int64Serie::from(values)),
            criterion::BatchSize::LargeInput,
        )
    });

    // The narrowest width, to expose any width-dependence in the same paths.
    group.bench_function("int8_serie_from_vec", |b| {
        b.iter_batched(
            || (0..N).map(|value| value as i8).collect::<Vec<_>>(),
            |values| black_box(Int8Serie::from(values)),
            criterion::BatchSize::LargeInput,
        )
    });

    let numbers = Int64Serie::from((0..N as i64).collect::<Vec<_>>());
    group.bench_function("int64_serie_values_borrow", |b| {
        b.iter(|| black_box(black_box(&numbers).values()))
    });

    group.bench_function("int64_serie_value_at", |b| {
        b.iter(|| {
            for index in 0..N {
                let _ = black_box(numbers.value_at::<i64>(black_box(index)));
            }
        })
    });

    // Logical equality walks both element buffers (and null buffers) once.
    let same = Int64Serie::from((0..N as i64).collect::<Vec<_>>());
    group.bench_function("int64_serie_eq", |b| {
        b.iter(|| black_box(black_box(&numbers) == black_box(&same)))
    });

    group.finish();
}

fn arrow(c: &mut Criterion) {
    let mut group = c.benchmark_group("serie_arrow");
    group.throughput(Throughput::Elements(N as u64));

    let numbers = Int64Serie::from((0..N as i64).collect::<Vec<_>>());
    group.bench_function("int64_serie_to_arrow", |b| {
        b.iter(|| black_box(numbers.to_arrow_scalar().into_inner()))
    });

    // The bare element-array conversion: a reference-count bump, no serie shell.
    group.bench_function("int64_serie_to_arrow_array", |b| {
        b.iter(|| black_box(numbers.to_arrow_array()))
    });

    let arrow = numbers.to_arrow_scalar().into_inner();
    group.bench_function("int64_serie_from_arrow", |b| {
        b.iter(|| black_box(Int64Serie::from_arrow(black_box(arrow.as_ref())).unwrap()))
    });

    // The generic scalar accessor, for comparison: one inner Arrow round trip per
    // element against the buffer-backed direct read above.
    let generic = Int64SerieGeneric::from_arrow(arrow.as_ref()).unwrap();
    group.bench_function("serie_scalar_at", |b| {
        b.iter(|| {
            for index in 0..N {
                black_box(generic.scalar_at(black_box(index)));
            }
        })
    });

    // The iterator against the same generic serie: it reconstitutes the element
    // column once and slices per step, where the `scalar_at` loop above
    // reconstitutes it on every call — so this measures the linear-vs-quadratic gap.
    group.bench_function("serie_iter_scalars", |b| {
        b.iter(|| {
            for scalar in generic.iter_scalars() {
                black_box(scalar);
            }
        })
    });

    group.finish();
}

fn io(c: &mut Criterion) {
    let mut group = c.benchmark_group("serie_io");
    // The IO bridge moves bytes, so throughput is measured in bytes.
    group.throughput(Throughput::Bytes((N * 8) as u64));

    let numbers = Int64Serie::from((0..N as i64).collect::<Vec<_>>());
    group.bench_function("int64_serie_pwrite_io", |b| {
        let mut buffer = ByteBuffer::new();
        buffer.resize_bytes(N * 8).unwrap();
        b.iter(|| {
            numbers
                .pwrite_io(black_box(&mut buffer), 0, Whence::Start)
                .unwrap()
        })
    });

    let mut buffer = ByteBuffer::new();
    numbers.pwrite_io(&mut buffer, 0, Whence::Start).unwrap();
    group.bench_function("int64_serie_from_io", |b| {
        b.iter(|| black_box(Int64Serie::from_io(black_box(&buffer)).unwrap()))
    });

    // The 1-byte width: the same element count in an eighth of the bytes.
    let bytes = UInt8Serie::from((0..N).map(|value| value as u8).collect::<Vec<_>>());
    let mut narrow = ByteBuffer::new();
    bytes.pwrite_io(&mut narrow, 0, Whence::Start).unwrap();
    group.bench_function("uint8_serie_from_io", |b| {
        b.iter(|| black_box(UInt8Serie::from_io(black_box(&narrow)).unwrap()))
    });

    group.finish();
}

// The float serie: buffer-backed like the integers, bridged through positioned IO.
fn float(c: &mut Criterion) {
    use yggdryl_scalar::Float64Serie;

    let mut group = c.benchmark_group("float_serie");
    group.throughput(Throughput::Elements(N as u64));

    let values = (0..N).map(|value| value as f64 + 0.5).collect::<Vec<_>>();
    group.bench_function("float64_serie_from_vec", |b| {
        b.iter_batched(
            || values.clone(),
            |values| black_box(Float64Serie::from(values)),
            criterion::BatchSize::SmallInput,
        )
    });

    let weights = Float64Serie::from(values.clone());
    group.bench_function("float64_serie_value_at", |b| {
        b.iter(|| black_box(weights.value_at::<f64>(black_box(N / 2)).unwrap()))
    });

    let mut buffer = ByteBuffer::new();
    weights.pwrite_io(&mut buffer, 0, Whence::Start).unwrap();
    group.bench_function("float64_serie_from_io", |b| {
        b.iter(|| black_box(Float64Serie::from_io(black_box(&buffer)).unwrap()))
    });

    // The half-precision serie shares the buffer-backed path (2 bytes per element).
    let halves = yggdryl_scalar::Float16Serie::from(
        (0..N)
            .map(|value| yggdryl_scalar::half::f16::from_f64(value as f64 + 0.5))
            .collect::<Vec<_>>(),
    );
    let mut narrow = ByteBuffer::new();
    halves.pwrite_io(&mut narrow, 0, Whence::Start).unwrap();
    group.bench_function("float16_serie_from_io", |b| {
        b.iter(|| black_box(yggdryl_scalar::Float16Serie::from_io(black_box(&narrow)).unwrap()))
    });

    group.finish();
}

// The AnySerie type-erased column: zero-copy decomposition from Arrow and
// on-demand reconstitution — the internals behind every nested scalar.
fn any_serie(c: &mut Criterion) {
    use yggdryl_scalar::AnySerie;

    let mut group = c.benchmark_group("any_serie");
    group.throughput(Throughput::Elements(N as u64));

    let arrow: yggdryl_scalar::arrow_array::ArrayRef = std::sync::Arc::new(
        yggdryl_scalar::arrow_array::Int64Array::from((0..N as i64).collect::<Vec<_>>()),
    );
    group.bench_function("decompose_int64", |b| {
        b.iter(|| black_box(AnySerie::from_arrow(black_box(arrow.clone()))))
    });

    let decomposed = AnySerie::from_arrow(arrow.clone());
    group.bench_function("recompose_int64", |b| {
        b.iter(|| black_box(decomposed.to_arrow()))
    });

    group.bench_function("slice_int64", |b| {
        b.iter(|| black_box(decomposed.slice(black_box(N / 4), black_box(N / 2))))
    });

    group.finish();
}

criterion_group!(benches, serie, float, any_serie, arrow, io);
criterion_main!(benches);
