use std::hint::black_box;
use std::io::Cursor;

use criterion::{Criterion, Throughput};
use yggdryl::text;
use yggdryl::text::{Formatting, xml};
use yggdryl::{from_xml_scalar, from_xml_scalar_with_field, into_xml_scalar};

use crate::fixtures::{representative, typed, xml_attributed, xml_document, xml_nested, xml_wide};

pub fn xml_benchmarks(criterion: &mut Criterion) {
    let value = xml_document("trade", representative());
    let encoded = xml::into_bytes(&value).unwrap();
    let encoded_text = std::str::from_utf8(&encoded).unwrap();
    let mut group = criterion.benchmark_group("codec/xml");
    group.throughput(Throughput::Bytes(encoded.len() as u64));
    group.bench_function("encode_representative", |bencher| {
        bencher.iter(|| xml::into_bytes(black_box(&value)).unwrap());
    });
    group.bench_function("encode_utf8_representative", |bencher| {
        bencher.iter(|| xml::into_utf8(black_box(&value)).unwrap());
    });
    group.bench_function("encode_scalar_entry", |bencher| {
        bencher.iter(|| into_xml_scalar(black_box(&value)).unwrap());
    });
    let mut writer_output = Vec::with_capacity(encoded.len());
    group.bench_function("write_representative", |bencher| {
        bencher.iter(|| {
            writer_output.clear();
            xml::into_writer(black_box(&value), &mut writer_output).unwrap();
            black_box(writer_output.len())
        });
    });
    group.bench_function("decode_representative", |bencher| {
        bencher.iter(|| xml::from_bytes(black_box(&encoded)).unwrap());
    });
    group.bench_function("decode_scalar_entry", |bencher| {
        bencher.iter(|| from_xml_scalar(black_box(&encoded)).unwrap());
    });
    group.bench_function("decode_borrowed_str", |bencher| {
        bencher.iter(|| xml::from_utf8(black_box(encoded_text)).unwrap());
    });
    group.bench_function("decode_reader", |bencher| {
        bencher.iter(|| xml::from_reader(Cursor::new(black_box(&encoded))).unwrap());
    });
    group.bench_function("stream_document", |bencher| {
        bencher.iter(|| {
            let mut count = 0;
            for value in xml::Reader::new(Cursor::new(black_box(&encoded))) {
                black_box(value.unwrap());
                count += 1;
            }
            count
        });
    });
    group.bench_function("infer_and_decode_borrowed_str", |bencher| {
        bencher.iter(|| text::from_utf8_inferred(black_box(encoded_text)).unwrap());
    });

    let (typed, field) = typed();
    let typed_document = xml_document(field.name(), typed);
    let typed_encoded = xml::into_bytes(&typed_document).unwrap();
    group.throughput(Throughput::Bytes(typed_encoded.len() as u64));
    group.bench_function("encode_typed_natural", |bencher| {
        bencher.iter(|| xml::into_bytes(black_box(&typed_document)).unwrap());
    });
    group.bench_function("decode_typed_natural_with_field", |bencher| {
        bencher.iter(|| {
            xml::from_bytes_with_field(black_box(&typed_encoded), black_box(&field)).unwrap()
        });
    });
    group.bench_function("decode_scalar_entry_with_field", |bencher| {
        bencher.iter(|| {
            from_xml_scalar_with_field(black_box(&typed_encoded), black_box(&field)).unwrap()
        });
    });

    let deep = xml_nested(64);
    let deep_encoded = xml::into_bytes(&deep).unwrap();
    group.throughput(Throughput::Bytes(deep_encoded.len() as u64));
    group.bench_function("decode_depth_64", |bencher| {
        bencher.iter(|| xml::from_bytes(black_box(&deep_encoded)).unwrap());
    });

    let wide = xml_wide(1_024);
    let wide_encoded = xml::into_bytes(&wide).unwrap();
    group.throughput(Throughput::Bytes(wide_encoded.len() as u64));
    group.bench_function("encode_wide_elements_1024", |bencher| {
        bencher.iter(|| xml::into_bytes(black_box(&wide)).unwrap());
    });
    group.bench_function("encode_wide_elements_1024_indented", |bencher| {
        bencher.iter(|| {
            xml::into_bytes_with_formatting(black_box(&wide), Formatting::indented(2)).unwrap()
        });
    });
    group.bench_function("decode_wide_elements_1024", |bencher| {
        bencher.iter(|| xml::from_bytes(black_box(&wide_encoded)).unwrap());
    });

    let attributed = xml_attributed(512);
    let attributed_encoded = xml::into_bytes(&attributed).unwrap();
    group.throughput(Throughput::Bytes(attributed_encoded.len() as u64));
    group.bench_function("encode_attributed_512", |bencher| {
        bencher.iter(|| xml::into_bytes(black_box(&attributed)).unwrap());
    });
    group.bench_function("decode_attributed_512", |bencher| {
        bencher.iter(|| xml::from_bytes(black_box(&attributed_encoded)).unwrap());
    });
    group.finish();
}
