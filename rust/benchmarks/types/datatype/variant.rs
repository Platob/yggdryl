//! The Parquet Variant encoding against value shape, and against the
//! format's own implementation on the same value.
//!
//! The encoding's cost follows the shape: one leaf,
//! where the whole payload is a header and a count; one object, where the
//! keys are gathered, sorted and written once; and one wide object, where
//! the dictionary and the four-byte widths are what the bytes are. The
//! reference rows exercise `parquet-variant`, the Apache crate. Its object
//! iterator borrows encoded values; the native decode materializes a Scalar.

use std::hint::black_box;
use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, Throughput};
use parquet_variant::{Variant as Reference, VariantBuilder};
use yggdryl::{DataType, Int64, Scalar, Serie, Value, Variant};

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

/// One ordered array whose final shared slice is the value the codec sees.
fn array(values: usize) -> Scalar {
    Scalar::from_sequence((0..values).map(|index| Scalar::from(index as i64)))
}

/// Two arrays around the same nested shape exercise recursive offsets and
/// the final shared sequence construction on decode.
fn nested(values: usize) -> Scalar {
    let children = array(values);
    Scalar::from_sequence([children.clone(), children])
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
        ("array", array(4)),
        ("nested", nested(4)),
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

    let primitive = Int64::new(7);
    let primitive_variant = Value::into_variant(&primitive).expect("the fixture encodes");
    let wrapped = Scalar::Variant(primitive_variant.clone());
    group.throughput(Throughput::Bytes(
        (primitive_variant.metadata().len() + primitive_variant.value().len()) as u64,
    ));
    group.bench_function("value_into_variant/int64", |bencher| {
        bencher.iter(|| Value::into_variant(black_box(&primitive)).expect("the fixture encodes"));
    });
    group.bench_function("value_from_variant/int64", |bencher| {
        bencher.iter(|| {
            black_box(<Int64 as Value>::from_variant(black_box(
                &primitive_variant,
            )))
            .expect("the fixture decodes")
        });
    });
    group.bench_function("scalar_into_variant/wrapped", |bencher| {
        bencher.iter(|| {
            black_box(&wrapped)
                .into_variant()
                .expect("the fixture projects")
        });
    });
    group.bench_function("scalar_from_variant/int64", |bencher| {
        bencher.iter(|| {
            Scalar::from_variant(black_box(&primitive_variant)).expect("the fixture decodes")
        });
    });

    let rows = corpus(1024, 16);
    let encoded_rows: Vec<Scalar> = (0..rows)
        .map(|_| Scalar::Variant(primitive_variant.clone()))
        .collect();
    let variant_field = Arc::new(DataType::Variant.required_field("payload"));
    group.throughput(Throughput::Elements(rows as u64));
    group.bench_with_input(
        BenchmarkId::new("arrow_column", rows),
        &encoded_rows,
        |bencher, values| {
            bencher.iter(|| {
                Serie::from_scalars(
                    Arc::clone(black_box(&variant_field)),
                    black_box(values).iter().cloned(),
                )
                .and_then(|serie| serie.require_arrow_array())
                .expect("the fixture builds an Arrow column")
            });
        },
    );

    // Reference construction and borrowed iteration have different output
    // ownership from native Scalar decoding; their timings are separate costs.
    let reference = reference_object(4);
    group.throughput(Throughput::Bytes(
        (reference.0.len() + reference.1.len()) as u64,
    ));
    group.bench_function("encode/object_reference", |bencher| {
        bencher.iter(|| black_box(reference_object(4)));
    });
    group.bench_function("iterate/object_reference", |bencher| {
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
