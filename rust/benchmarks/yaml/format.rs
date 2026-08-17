use std::hint::black_box;
use std::io::Cursor;

use criterion::{Criterion, Throughput};
use yggdryl::yaml;

use crate::fixtures::{exotic, nested, representative};

pub fn yaml_benchmarks(criterion: &mut Criterion) {
    let value = representative();
    let encoded = yaml::to_vec(&value).unwrap();
    let mut group = criterion.benchmark_group("codec/yaml");
    group.throughput(Throughput::Bytes(encoded.len() as u64));
    group.bench_function("encode_representative", |bencher| {
        bencher.iter(|| yaml::to_vec(black_box(&value)).unwrap());
    });
    let mut writer_output = Vec::with_capacity(encoded.len());
    group.bench_function("write_representative", |bencher| {
        bencher.iter(|| {
            writer_output.clear();
            yaml::to_writer(&mut writer_output, black_box(&value)).unwrap();
            black_box(writer_output.len())
        });
    });
    group.bench_function("decode_representative", |bencher| {
        bencher.iter(|| yaml::from_slice(black_box(&encoded)).unwrap());
    });
    let encoded_text = std::str::from_utf8(&encoded).unwrap();
    group.bench_function("decode_borrowed_str", |bencher| {
        bencher.iter(|| yaml::from_str(black_box(encoded_text)).unwrap());
    });

    let round_trip = representative();
    let round_trip_encoded = yaml::to_vec(&round_trip).unwrap();
    group.throughput(Throughput::Bytes(round_trip_encoded.len() as u64));
    group.bench_function("representative_round_trip", |bencher| {
        bencher.iter(|| {
            let bytes = yaml::to_vec(black_box(&round_trip)).unwrap();
            yaml::from_slice(&bytes).unwrap()
        });
    });

    let deep = nested(64);
    let deep_encoded = yaml::to_vec(&deep).unwrap();
    group.throughput(Throughput::Bytes(deep_encoded.len() as u64));
    group.bench_function("decode_depth_64", |bencher| {
        bencher.iter(|| yaml::from_slice(black_box(&deep_encoded)).unwrap());
    });

    let exotic = exotic();
    let exotic_encoded = yaml::to_vec(&exotic).unwrap();
    group.throughput(Throughput::Bytes(exotic_encoded.len() as u64));
    group.bench_function("encode_exotic_envelope", |bencher| {
        bencher.iter(|| yaml::to_vec(black_box(&exotic)).unwrap());
    });
    group.bench_function("decode_exotic_envelope", |bencher| {
        bencher.iter(|| yaml::from_slice(black_box(&exotic_encoded)).unwrap());
    });

    let documents = yaml::to_vec_all(&vec![value; 100]).unwrap();
    group.throughput(Throughput::Bytes(documents.len() as u64));
    let documents_text = std::str::from_utf8(&documents).unwrap();
    group.bench_function("decode_borrowed_str_100_documents", |bencher| {
        bencher.iter(|| yaml::from_str_all(black_box(documents_text)).unwrap());
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
