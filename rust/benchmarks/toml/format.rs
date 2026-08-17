use std::hint::black_box;
use std::io::Cursor;

use criterion::{Criterion, Throughput};
use yggdryl::text;
use yggdryl::{Value, toml};

use crate::fixtures::{exotic, nested, representative};

pub fn toml_benchmarks(criterion: &mut Criterion) {
    let value = representative();
    let encoded = toml::to_vec(&value).unwrap();
    let encoded_text = std::str::from_utf8(&encoded).unwrap();
    let mut group = criterion.benchmark_group("codec/toml");
    group.throughput(Throughput::Bytes(encoded.len() as u64));
    group.bench_function("encode_representative", |bencher| {
        bencher.iter(|| toml::to_vec(black_box(&value)).unwrap());
    });
    let mut writer_output = Vec::with_capacity(encoded.len());
    group.bench_function("write_representative", |bencher| {
        bencher.iter(|| {
            writer_output.clear();
            toml::to_writer(&mut writer_output, black_box(&value)).unwrap();
            black_box(writer_output.len())
        });
    });
    group.bench_function("decode_representative", |bencher| {
        bencher.iter(|| toml::from_slice(black_box(&encoded)).unwrap());
    });
    group.bench_function("decode_borrowed_str", |bencher| {
        bencher.iter(|| toml::from_str(black_box(encoded_text)).unwrap());
    });
    group.bench_function("decode_reader", |bencher| {
        bencher.iter(|| toml::from_reader(Cursor::new(black_box(&encoded))).unwrap());
    });
    group.bench_function("infer_and_decode_borrowed_str", |bencher| {
        bencher.iter(|| text::from_str_inferred(black_box(encoded_text)).unwrap());
    });

    let exotic = exotic();
    let exotic_encoded = toml::to_vec(&exotic).unwrap();
    group.throughput(Throughput::Bytes(exotic_encoded.len() as u64));
    group.bench_function("encode_exotic_envelope", |bencher| {
        bencher.iter(|| toml::to_vec(black_box(&exotic)).unwrap());
    });
    group.bench_function("decode_exotic_envelope", |bencher| {
        bencher.iter(|| toml::from_slice(black_box(&exotic_encoded)).unwrap());
    });

    let deep = Value::from_mapping([(Value::from("value"), nested(48))]).unwrap();
    let deep_encoded = toml::to_vec(&deep).unwrap();
    group.throughput(Throughput::Bytes(deep_encoded.len() as u64));
    group.bench_function("decode_depth_49", |bencher| {
        bencher.iter(|| toml::from_slice(black_box(&deep_encoded)).unwrap());
    });

    let wide = Value::from_mapping(
        (0_i64..1_024).map(|index| (Value::from(format!("key_{index}")), Value::I64(index))),
    )
    .unwrap();
    let wide_encoded = toml::to_vec(&wide).unwrap();
    group.throughput(Throughput::Bytes(wide_encoded.len() as u64));
    group.bench_function("decode_wide_mapping_1024", |bencher| {
        bencher.iter(|| toml::from_slice(black_box(&wide_encoded)).unwrap());
    });
    group.finish();
}
