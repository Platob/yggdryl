use std::hint::black_box;
use std::io::Cursor;

use criterion::{Criterion, Throughput};
use yggdryl::text::json;
use yggdryl::{from_json_scalar, from_json_scalar_with_field, into_json_scalar};

use crate::fixtures::{nested, representative, typed};

pub fn json_benchmarks(criterion: &mut Criterion) {
    let value = representative();
    let encoded = json::into_bytes(&value).unwrap();
    let mut group = criterion.benchmark_group("codec/json");
    group.throughput(Throughput::Bytes(encoded.len() as u64));
    group.bench_function("encode_representative", |bencher| {
        bencher.iter(|| json::into_bytes(black_box(&value)).unwrap());
    });
    group.bench_function("encode_utf8_representative", |bencher| {
        bencher.iter(|| json::into_utf8(black_box(&value)).unwrap());
    });
    group.bench_function("encode_scalar_entry", |bencher| {
        bencher.iter(|| into_json_scalar(black_box(&value)).unwrap());
    });
    let mut writer_output = Vec::with_capacity(encoded.len());
    group.bench_function("write_representative", |bencher| {
        bencher.iter(|| {
            writer_output.clear();
            json::into_writer(black_box(&value), &mut writer_output).unwrap();
            black_box(writer_output.len())
        });
    });
    group.bench_function("decode_representative", |bencher| {
        bencher.iter(|| json::from_bytes(black_box(&encoded)).unwrap());
    });
    group.bench_function("decode_scalar_entry", |bencher| {
        bencher.iter(|| from_json_scalar(black_box(&encoded)).unwrap());
    });
    let encoded_text = std::str::from_utf8(&encoded).unwrap();
    group.bench_function("decode_borrowed_str", |bencher| {
        bencher.iter(|| json::from_utf8(black_box(encoded_text)).unwrap());
    });

    let deep = nested(64);
    let deep_encoded = json::into_bytes(&deep).unwrap();
    group.throughput(Throughput::Bytes(deep_encoded.len() as u64));
    group.bench_function("decode_depth_64", |bencher| {
        bencher.iter(|| json::from_bytes(black_box(&deep_encoded)).unwrap());
    });

    let (typed, field) = typed();
    let typed_encoded = json::into_bytes(&typed).unwrap();
    group.throughput(Throughput::Bytes(typed_encoded.len() as u64));
    group.bench_function("encode_typed_natural", |bencher| {
        bencher.iter(|| json::into_bytes(black_box(&typed)).unwrap());
    });
    group.bench_function("decode_typed_natural_with_field", |bencher| {
        bencher.iter(|| {
            json::from_bytes_with_field(black_box(&typed_encoded), black_box(&field)).unwrap()
        });
    });
    group.bench_function("decode_scalar_entry_with_field", |bencher| {
        bencher.iter(|| {
            from_json_scalar_with_field(black_box(&typed_encoded), black_box(&field)).unwrap()
        });
    });

    let lines = (0..1_000_u64)
        .map(|value| format!("{value}\n"))
        .collect::<String>()
        .into_bytes();
    group.throughput(Throughput::Bytes(lines.len() as u64));
    let lines_text = std::str::from_utf8(&lines).unwrap();
    group.bench_function("decode_borrowed_str_1000_json_lines", |bencher| {
        bencher.iter(|| json::from_lines_utf8(black_box(lines_text)).unwrap());
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
