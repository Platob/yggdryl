//! The value stream: any value as one byte stream, and back.
//!
//! The same shapes the canonical feed is measured over, because the two
//! walk one tree two ways - the feed into a digest, the encoding into
//! bytes a column stores - so a stray allocation in one shows beside the
//! other. Encoding is measured whole and as the stream of chunks a leaf at
//! a time, decoding from the whole; a text past the compression bound is
//! the one shape whose payload is a zstd frame.

use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::{COMPRESS_FROM, DataType, Scalar};

/// A leaf, a number, a decimal, a row, a wide record, a deep nest, and a
/// text long enough to compress.
fn corpus() -> Vec<(&'static str, Scalar)> {
    let wide = Scalar::from_struct(
        (0..64).map(|index| (format!("column_{index:03}"), Scalar::from(index))),
    )
    .expect("the generated record names are unique");
    let mut deep = Scalar::from("leaf");
    for _ in 0..32 {
        deep = Scalar::from_sequence([deep, Scalar::from(1)]);
    }
    let row = Scalar::from_sequence([
        Scalar::from("AAPL"),
        Scalar::from(100),
        Scalar::d128(18_723, 2),
        Scalar::from("XNAS"),
    ]);
    let long = "the quick brown fox jumps over the lazy dog; ".repeat(COMPRESS_FROM / 40 + 1);
    vec![
        ("leaf", Scalar::from("AAPL")),
        ("integer", Scalar::from(18_723)),
        ("decimal", Scalar::d128(18_723, 2)),
        ("row", row),
        ("wide_record", wide),
        ("deep_nest", deep),
        ("compressed_text", Scalar::from(long.as_str())),
    ]
}

/// Encoding whole and as chunks, decoding, and the round trip through a
/// datatype's own doors.
pub(crate) fn variant_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("variant");
    for (name, value) in corpus() {
        let bytes = value.into_value_bytes();
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_function(format!("encode/{name}"), |bencher| {
            bencher.iter(|| black_box(&value).into_value_bytes());
        });
        group.bench_function(format!("encode_stream/{name}"), |bencher| {
            bencher.iter(|| {
                black_box(&value)
                    .encode_value_stream_bytes()
                    .map(|chunk| chunk.len())
                    .sum::<usize>()
            });
        });
        group.bench_function(format!("decode/{name}"), |bencher| {
            bencher.iter(|| Scalar::decode_value_bytes(black_box(&bytes)).expect("its own bytes"));
        });
    }
    group.finish();

    // A column encodes what its datatype holds: the cast on the way in and
    // the way out, over one row of trades.
    let dtype = DataType::from_str(
        "struct<symbol: utf8 not null, quantity: int64, price: decimal128(12, 2), venue: utf8>",
    )
    .expect("a valid datatype fixture");
    let row = Scalar::from_struct([
        ("symbol", Scalar::from("AAPL")),
        ("quantity", Scalar::from(100_i64)),
        ("price", Scalar::d128(18_723, 2)),
        ("venue", Scalar::from("XNAS")),
    ])
    .expect("unique names");
    let bytes = dtype
        .encode_value_bytes(&row)
        .expect("a row of the datatype");
    let mut group = criterion.benchmark_group("variant_datatype");
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_function("encode/trade", |bencher| {
        bencher.iter(|| {
            black_box(&dtype)
                .encode_value_bytes(black_box(&row))
                .expect("a row of the datatype")
        });
    });
    group.bench_function("decode/trade", |bencher| {
        bencher.iter(|| {
            black_box(&dtype)
                .decode_value_bytes(black_box(&bytes))
                .expect("its own bytes")
        });
    });
    group.finish();
}
