use std::hint::black_box;
use std::io::Cursor;

use criterion::{Criterion, Throughput};
use yggdryl::{from_yaml_scalar, from_yaml_scalar_with_field, into_yaml_scalar, yaml};

use crate::fixtures::{nested, representative, typed};

pub fn yaml_benchmarks(criterion: &mut Criterion) {
    let value = representative();
    let encoded = yaml::into_bytes(&value).unwrap();
    let mut group = criterion.benchmark_group("codec/yaml");
    group.throughput(Throughput::Bytes(encoded.len() as u64));
    group.bench_function("encode_representative", |bencher| {
        bencher.iter(|| yaml::into_bytes(black_box(&value)).unwrap());
    });
    group.bench_function("encode_utf8_representative", |bencher| {
        bencher.iter(|| yaml::into_utf8(black_box(&value)).unwrap());
    });
    group.bench_function("encode_scalar_entry", |bencher| {
        bencher.iter(|| into_yaml_scalar(black_box(&value)).unwrap());
    });
    let mut writer_output = Vec::with_capacity(encoded.len());
    group.bench_function("write_representative", |bencher| {
        bencher.iter(|| {
            writer_output.clear();
            yaml::into_writer(black_box(&value), &mut writer_output).unwrap();
            black_box(writer_output.len())
        });
    });
    group.bench_function("decode_representative", |bencher| {
        bencher.iter(|| yaml::from_bytes(black_box(&encoded)).unwrap());
    });
    group.bench_function("decode_scalar_entry", |bencher| {
        bencher.iter(|| from_yaml_scalar(black_box(&encoded)).unwrap());
    });
    let encoded_text = std::str::from_utf8(&encoded).unwrap();
    group.bench_function("decode_borrowed_str", |bencher| {
        bencher.iter(|| yaml::from_utf8(black_box(encoded_text)).unwrap());
    });

    let round_trip = representative();
    let round_trip_encoded = yaml::into_bytes(&round_trip).unwrap();
    group.throughput(Throughput::Bytes(round_trip_encoded.len() as u64));
    group.bench_function("representative_round_trip", |bencher| {
        bencher.iter(|| {
            let bytes = yaml::into_bytes(black_box(&round_trip)).unwrap();
            yaml::from_bytes(&bytes).unwrap()
        });
    });

    let deep = nested(64);
    let deep_encoded = yaml::into_bytes(&deep).unwrap();
    group.throughput(Throughput::Bytes(deep_encoded.len() as u64));
    group.bench_function("decode_depth_64", |bencher| {
        bencher.iter(|| yaml::from_bytes(black_box(&deep_encoded)).unwrap());
    });

    let (typed, field) = typed();
    let typed_encoded = yaml::into_bytes(&typed).unwrap();
    group.throughput(Throughput::Bytes(typed_encoded.len() as u64));
    group.bench_function("encode_typed_natural", |bencher| {
        bencher.iter(|| yaml::into_bytes(black_box(&typed)).unwrap());
    });
    group.bench_function("decode_typed_natural_with_field", |bencher| {
        bencher.iter(|| {
            yaml::from_bytes_with_field(black_box(&typed_encoded), black_box(&field)).unwrap()
        });
    });
    group.bench_function("decode_scalar_entry_with_field", |bencher| {
        bencher.iter(|| {
            from_yaml_scalar_with_field(black_box(&typed_encoded), black_box(&field)).unwrap()
        });
    });

    let documents = yaml::into_bytes_all(&vec![value; 100]).unwrap();
    group.throughput(Throughput::Bytes(documents.len() as u64));
    let documents_text = std::str::from_utf8(&documents).unwrap();
    group.bench_function("decode_borrowed_str_100_documents", |bencher| {
        bencher.iter(|| yaml::from_utf8_all(black_box(documents_text)).unwrap());
    });
    group.bench_function("stream_100_documents", |bencher| {
        bencher.iter(|| {
            let mut cursor = Cursor::new(black_box(&documents));
            let mut count = 0;
            for value in yaml::from_reader_iter(&mut cursor) {
                black_box(value.unwrap());
                count += 1;
            }
            count
        });
    });
    group.finish();
}
