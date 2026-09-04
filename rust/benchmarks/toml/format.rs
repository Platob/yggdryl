use std::hint::black_box;
use std::io::Cursor;

use criterion::{Criterion, Throughput};
use yggdryl::text;
use yggdryl::{Scalar, from_toml_scalar, from_toml_scalar_with_field, into_toml_scalar, toml};

use crate::fixtures::{nested, representative, typed};

pub fn toml_benchmarks(criterion: &mut Criterion) {
    let value = representative();
    let encoded = toml::into_bytes(&value).unwrap();
    let encoded_text = std::str::from_utf8(&encoded).unwrap();
    let mut group = criterion.benchmark_group("codec/toml");
    group.throughput(Throughput::Bytes(encoded.len() as u64));
    group.bench_function("encode_representative", |bencher| {
        bencher.iter(|| toml::into_bytes(black_box(&value)).unwrap());
    });
    group.bench_function("encode_utf8_representative", |bencher| {
        bencher.iter(|| toml::into_utf8(black_box(&value)).unwrap());
    });
    group.bench_function("encode_scalar_entry", |bencher| {
        bencher.iter(|| into_toml_scalar(black_box(&value)).unwrap());
    });
    let mut writer_output = Vec::with_capacity(encoded.len());
    group.bench_function("write_representative", |bencher| {
        bencher.iter(|| {
            writer_output.clear();
            toml::into_writer(black_box(&value), &mut writer_output).unwrap();
            black_box(writer_output.len())
        });
    });
    group.bench_function("decode_representative", |bencher| {
        bencher.iter(|| toml::from_bytes(black_box(&encoded)).unwrap());
    });
    group.bench_function("decode_scalar_entry", |bencher| {
        bencher.iter(|| from_toml_scalar(black_box(&encoded)).unwrap());
    });
    group.bench_function("decode_borrowed_str", |bencher| {
        bencher.iter(|| toml::from_utf8(black_box(encoded_text)).unwrap());
    });
    group.bench_function("decode_reader", |bencher| {
        bencher.iter(|| toml::from_reader(Cursor::new(black_box(&encoded))).unwrap());
    });
    group.bench_function("infer_and_decode_borrowed_str", |bencher| {
        bencher.iter(|| text::from_utf8_inferred(black_box(encoded_text)).unwrap());
    });

    let (typed, field) = typed();
    let typed_encoded = toml::into_bytes(&typed).unwrap();
    group.throughput(Throughput::Bytes(typed_encoded.len() as u64));
    group.bench_function("encode_typed_natural", |bencher| {
        bencher.iter(|| toml::into_bytes(black_box(&typed)).unwrap());
    });
    group.bench_function("decode_typed_natural_with_field", |bencher| {
        bencher.iter(|| {
            toml::from_bytes_with_field(black_box(&typed_encoded), black_box(&field)).unwrap()
        });
    });
    group.bench_function("decode_scalar_entry_with_field", |bencher| {
        bencher.iter(|| {
            from_toml_scalar_with_field(black_box(&typed_encoded), black_box(&field)).unwrap()
        });
    });

    let deep = Scalar::from_record([("value", nested(48))]).unwrap();
    let deep_encoded = toml::into_bytes(&deep).unwrap();
    group.throughput(Throughput::Bytes(deep_encoded.len() as u64));
    group.bench_function("decode_depth_49", |bencher| {
        bencher.iter(|| toml::from_bytes(black_box(&deep_encoded)).unwrap());
    });

    let wide = Scalar::from_record(
        (0_i64..1_024).map(|index| (format!("key_{index}"), Scalar::I64(index))),
    )
    .unwrap();
    let wide_encoded = toml::into_bytes(&wide).unwrap();
    group.throughput(Throughput::Bytes(wide_encoded.len() as u64));
    group.bench_function("decode_wide_mapping_1024", |bencher| {
        bencher.iter(|| toml::from_bytes(black_box(&wide_encoded)).unwrap());
    });
    group.finish();
}
