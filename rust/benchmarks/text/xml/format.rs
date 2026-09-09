use std::hint::black_box;
use std::io::Cursor;

use criterion::{Criterion, Throughput};
use yggdryl::text;
use yggdryl::text::xml;
use yggdryl::{Scalar, from_xml_scalar, from_xml_scalar_with_field, into_xml_scalar};

use crate::fixtures::{representative, typed};

/// One document, which is the one-entry record an XML root names.
fn document(value: Scalar) -> Scalar {
    Scalar::from_record([("row", value)]).expect("one entry")
}

/// A chain of nested elements, which is how XML spells depth.
fn deep(depth: usize) -> Scalar {
    (0..depth).fold(Scalar::from("0"), |value, _| {
        Scalar::from_record([("a", value)]).expect("one entry")
    })
}

pub fn xml_benchmarks(criterion: &mut Criterion) {
    let value = document(representative());
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
    group.bench_function("infer_and_decode_borrowed_str", |bencher| {
        bencher.iter(|| text::from_utf8_inferred(black_box(encoded_text)).unwrap());
    });

    let (typed, field) = typed();
    let typed_document = document(typed);
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

    let nested = deep(48);
    let nested_encoded = xml::into_bytes(&nested).unwrap();
    group.throughput(Throughput::Bytes(nested_encoded.len() as u64));
    group.bench_function("decode_depth_49", |bencher| {
        bencher.iter(|| xml::from_bytes(black_box(&nested_encoded)).unwrap());
    });

    let wide = document(
        Scalar::from_record(
            (0_i64..1_024).map(|index| (format!("key_{index}"), Scalar::from(index))),
        )
        .unwrap(),
    );
    let wide_encoded = xml::into_bytes(&wide).unwrap();
    group.throughput(Throughput::Bytes(wide_encoded.len() as u64));
    group.bench_function("decode_wide_element_1024", |bencher| {
        bencher.iter(|| xml::from_bytes(black_box(&wide_encoded)).unwrap());
    });

    let escaped = document(
        Scalar::from_record([("body", Scalar::from("<tag> & \"quoted\" ".repeat(256)))]).unwrap(),
    );
    let escaped_encoded = xml::into_bytes(&escaped).unwrap();
    group.throughput(Throughput::Bytes(escaped_encoded.len() as u64));
    group.bench_function("encode_escaped_text", |bencher| {
        bencher.iter(|| xml::into_bytes(black_box(&escaped)).unwrap());
    });
    group.bench_function("decode_escaped_text", |bencher| {
        bencher.iter(|| xml::from_bytes(black_box(&escaped_encoded)).unwrap());
    });
    group.finish();
}
