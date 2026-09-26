use std::hint::black_box;
use std::io::Cursor;

use criterion::{Criterion, Throughput};
use yggdryl::text;
use yggdryl::xml;
use yggdryl::{Scalar, from_xml_scalar, from_xml_scalar_with_field, into_xml_scalar};

use crate::fixtures::{representative, typed};

/// The representative record as the document it is: one root element.
fn document(value: Scalar) -> Scalar {
    Scalar::from_struct([("row", value)]).unwrap()
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
    let typed = document(typed);
    let typed_encoded = xml::into_bytes(&typed).unwrap();
    group.throughput(Throughput::Bytes(typed_encoded.len() as u64));
    group.bench_function("encode_typed_natural", |bencher| {
        bencher.iter(|| xml::into_bytes(black_box(&typed)).unwrap());
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

    // XML nests by element, so the deep fixture is records rather than the
    // sequences the other codecs nest - a sequence inside a sequence has no
    // element to repeat.
    let deep = document((0..48).fold(Scalar::from(0), |value, _| {
        Scalar::from_struct([("d", value)]).unwrap()
    }));
    let deep_encoded = xml::into_bytes(&deep).unwrap();
    group.throughput(Throughput::Bytes(deep_encoded.len() as u64));
    group.bench_function("decode_depth_49", |bencher| {
        bencher.iter(|| xml::from_bytes(black_box(&deep_encoded)).unwrap());
    });

    let wide = document(
        Scalar::from_struct(
            (0_i64..1_024).map(|index| (format!("key_{index}"), Scalar::from(index))),
        )
        .unwrap(),
    );
    let wide_encoded = xml::into_bytes(&wide).unwrap();
    group.throughput(Throughput::Bytes(wide_encoded.len() as u64));
    group.bench_function("decode_wide_record_1024", |bencher| {
        bencher.iter(|| xml::from_bytes(black_box(&wide_encoded)).unwrap());
    });

    // Repeated elements are what a table is in XML: a thousand rows of the
    // representative record under one document element.
    let rows = Scalar::from_struct([(
        "data",
        Scalar::from_struct([(
            "row",
            Scalar::from_sequence((0..1_000).map(|_| representative())),
        )])
        .unwrap(),
    )])
    .unwrap();
    let rows_encoded = xml::into_bytes(&rows).unwrap();
    group.throughput(Throughput::Bytes(rows_encoded.len() as u64));
    group.bench_function("decode_rows_1000", |bencher| {
        bencher.iter(|| xml::from_bytes(black_box(&rows_encoded)).unwrap());
    });
    group.bench_function("encode_rows_1000", |bencher| {
        bencher.iter(|| xml::into_bytes(black_box(&rows)).unwrap());
    });
    group.finish();
}
