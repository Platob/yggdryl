//! The Parquet Variant encoding against value shape, and against the
//! format's own implementation on the same value.
//!
//! Three shapes, because the encoding's cost is the shape's: one leaf,
//! where the whole payload is a header and a count; one object, where the
//! keys are gathered, sorted and written once; and one wide object, where
//! the dictionary and the four-byte widths are what the bytes are. The
//! reference rows build the same object through `parquet-variant`, the
//! Apache crate, so a number here is read beside the format's own.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use parquet_variant::{Variant as Reference, VariantBuilder};
use yggdryl::{Scalar, Variant};

use crate::bench_profile::corpus;

/// One object of `fields` small columns, the shape a variant column holds.
fn object(fields: usize) -> Scalar {
    let names: Vec<String> = (0..fields)
        .map(|index| format!("field{index:04}"))
        .collect();
    Scalar::from_struct(
        names
            .iter()
            .enumerate()
            .map(|(index, name)| (name.as_str(), Scalar::from(index as i64))),
    )
    .expect("the fixture builds")
}

/// The same object through the format's own builder.
fn reference_object(fields: usize) -> (Vec<u8>, Vec<u8>) {
    let mut builder = VariantBuilder::new();
    let mut entries = builder.new_object();
    for index in 0..fields {
        entries.insert(&format!("field{index:04}"), index as i64);
    }
    entries.finish();
    builder.finish()
}

pub(crate) fn variant_benchmarks(criterion: &mut Criterion) {
    let wide = corpus(300, 8);
    let values = [
        ("leaf", Scalar::from(7_i64)),
        ("object", object(4)),
        ("object_wide", object(wide)),
    ];

    let mut group = criterion.benchmark_group("variant");
    for (name, value) in &values {
        let encoded = Variant::encode(value).expect("the fixture encodes");
        let bytes = encoded.metadata().len() + encoded.value().len();
        group.throughput(Throughput::Bytes(bytes as u64));
        group.bench_with_input(BenchmarkId::new("encode", name), value, |bencher, value| {
            bencher.iter(|| Variant::encode(black_box(value)).expect("the fixture encodes"));
        });
        group.bench_with_input(
            BenchmarkId::new("decode", name),
            &encoded,
            |bencher, encoded| {
                bencher.iter(|| black_box(encoded).scalar().expect("the fixture decodes"));
            },
        );
    }

    // The same object, built and read by the format's own crate: the two
    // rows are the same bytes on the same value, so the ratio is the one
    // worth reading.
    let reference = reference_object(4);
    group.bench_function("encode/object_reference", |bencher| {
        bencher.iter(|| black_box(reference_object(4)));
    });
    group.bench_function("decode/object_reference", |bencher| {
        bencher.iter(|| {
            let read = Reference::try_new(black_box(&reference.0), black_box(&reference.1))
                .expect("the fixture is a variant");
            read.as_object()
                .expect("an object")
                .iter()
                .for_each(|entry| {
                    black_box(entry);
                });
        });
    });
    group.finish();
}
