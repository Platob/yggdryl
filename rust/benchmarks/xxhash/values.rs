//! The canonical value feed, and the two `stable_hash` sinks over it.

use std::hint::black_box;

use criterion::Criterion;
use yggdryl::xxhash::Xxh3_64;
use yggdryl::{DataType, DigestAlgorithm, Field, Scalar, Uri};

/// A leaf, a wide record, and a deep nest: the three shapes the feed walks
/// differently, and the three where a stray allocation would show.
fn corpus() -> Vec<(&'static str, Scalar)> {
    let wide = Scalar::from_record(
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
    vec![
        ("leaf", Scalar::from("AAPL")),
        ("integer", Scalar::from(18_723)),
        ("decimal", Scalar::d128(18_723, 2)),
        ("row", row),
        ("wide_record", wide),
        ("deep_nest", deep),
    ]
}

/// Feeding a value, against digesting one and hashing one.
///
/// The feed row reuses one state, which is what an Arrow column does; the
/// digest row builds a fresh state per value, which is what a caller asking
/// for one digest pays. The gap between them is XXH3's secret allocation,
/// not the feed's - the feed itself allocates nothing, which
/// `tests/allocations.rs` pins rather than this measuring.
pub(crate) fn value_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("xxhash_value");
    for (name, value) in corpus() {
        group.bench_function(format!("feed/{name}"), |bencher| {
            let mut state = Xxh3_64::new();
            bencher.iter(|| {
                state.clear();
                black_box(&value).write_bytes(&mut state);
                state.as_u64()
            });
        });
        group.bench_function(format!("digest/{name}"), |bencher| {
            bencher.iter(|| black_box(&value).digest(DigestAlgorithm::Xxh3_64));
        });
        group.bench_function(format!("stable_hash/{name}"), |bencher| {
            bencher.iter(|| black_box(&value).stable_hash());
        });
    }
    group.finish();
}

/// The two `stable_hash` sinks on the short renderings they actually see.
///
/// A field name, a URI, and a datatype expression are a handful of bytes each,
/// which is where a byte-at-a-time fold is competitive with a wide one. The
/// numbers are reported as they are: one hash contract is worth more than a
/// few nanoseconds on a short string.
pub(crate) fn stable_hash_benchmarks(criterion: &mut Criterion) {
    let field = Field::new("settlement_currency", DataType::Utf8, true);
    let uri = Uri::from_str("s3://warehouse/trades/2026/02/01/part-0000.parquet")
        .expect("a valid URI fixture");
    let dtype = DataType::from_str(
        "struct<symbol: utf8 not null, quantity: int64, price: decimal128(12, 2)>",
    )
    .expect("a valid datatype fixture");

    // The bytes each rendering produces, hashed on their own. The gap between
    // a `*_rendered` row and its `*_hash` row is the canonical rendering, not
    // the hash: at a handful of bytes the algorithm is the small half. The
    // pre-swap FNV-1a numbers are not reproducible from this tree, and
    // deliberately so - the fold is deleted, and one hash contract is the
    // point.
    let field_text = field.to_string();
    let uri_text = uri.to_string();
    let dtype_text = dtype.to_string();

    let mut group = criterion.benchmark_group("xxhash_stable_hash");
    group.bench_function("field_hash", |bencher| {
        bencher.iter(|| black_box(&field).stable_hash());
    });
    group.bench_function("field_bytes_only", |bencher| {
        bencher.iter(|| yggdryl::xxhash::xxh3_64(black_box(field_text.as_bytes())));
    });
    group.bench_function("uri_hash", |bencher| {
        bencher.iter(|| black_box(&uri).stable_hash());
    });
    group.bench_function("uri_bytes_only", |bencher| {
        bencher.iter(|| yggdryl::xxhash::xxh3_64(black_box(uri_text.as_bytes())));
    });
    group.bench_function("datatype_hash", |bencher| {
        bencher.iter(|| black_box(&dtype).stable_hash());
    });
    group.bench_function("datatype_bytes_only", |bencher| {
        bencher.iter(|| yggdryl::xxhash::xxh3_64(black_box(dtype_text.as_bytes())));
    });
    group.finish();
}
