use std::hint::black_box;
use std::io::Cursor;

use criterion::{Criterion, Throughput};
use yggdryl::json;

use crate::fixtures::{exotic, nested, representative};

pub fn json_benchmarks(criterion: &mut Criterion) {
    let value = representative();
    let encoded = json::to_vec(&value).unwrap();
    let mut group = criterion.benchmark_group("codec/json");
    group.throughput(Throughput::Bytes(encoded.len() as u64));
    group.bench_function("encode_representative", |bencher| {
        bencher.iter(|| json::to_vec(black_box(&value)).unwrap());
    });
    let mut writer_output = Vec::with_capacity(encoded.len());
    group.bench_function("write_representative", |bencher| {
        bencher.iter(|| {
            writer_output.clear();
            json::to_writer(&mut writer_output, black_box(&value)).unwrap();
            black_box(writer_output.len())
        });
    });
    group.bench_function("decode_representative", |bencher| {
        bencher.iter(|| json::from_slice(black_box(&encoded)).unwrap());
    });
    let encoded_text = std::str::from_utf8(&encoded).unwrap();
    group.bench_function("decode_borrowed_str", |bencher| {
        bencher.iter(|| json::from_str(black_box(encoded_text)).unwrap());
    });

    let deep = nested(64);
    let deep_encoded = json::to_vec(&deep).unwrap();
    group.throughput(Throughput::Bytes(deep_encoded.len() as u64));
    group.bench_function("decode_depth_64", |bencher| {
        bencher.iter(|| json::from_slice(black_box(&deep_encoded)).unwrap());
    });

    let exotic = exotic();
    let exotic_encoded = json::to_vec(&exotic).unwrap();
    group.throughput(Throughput::Bytes(exotic_encoded.len() as u64));
    group.bench_function("encode_exotic_envelope", |bencher| {
        bencher.iter(|| json::to_vec(black_box(&exotic)).unwrap());
    });
    group.bench_function("decode_exotic_envelope", |bencher| {
        bencher.iter(|| json::from_slice(black_box(&exotic_encoded)).unwrap());
    });

    let lines = (0..1_000_u64)
        .map(|value| format!("{value}\n"))
        .collect::<String>()
        .into_bytes();
    group.throughput(Throughput::Bytes(lines.len() as u64));
    let lines_text = std::str::from_utf8(&lines).unwrap();
    group.bench_function("decode_borrowed_str_1000_json_lines", |bencher| {
        bencher.iter(|| json::from_lines_str(black_box(lines_text)).unwrap());
    });
    group.bench_function("stream_1000_json_lines", |bencher| {
        bencher.iter(|| {
            let mut count = 0;
            for value in json::LinesReader::new(Cursor::new(black_box(&lines))) {
                black_box(value.unwrap());
                count += 1;
            }
            count
        });
    });
    group.finish();
}
