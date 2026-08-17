use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use yggdryl::{MediaType, MimeType, Scheme, Uri, Url, Urn};

const NETWORK_URI: &str =
    "https://user@example.test:8443/archive/2026/report.tar.zst?download=1#summary";
const WINDOWS_PATH: &str = r"C:\Users\Ada Lovelace\market data\trades.parquet";
const UNC_PATH: &str = r"\\market-data\shared\prices\2026\ticks.arrow";

fn parsing_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("resource_parse");
    group.bench_function("uri_canonical", |bencher| {
        bencher.iter(|| Uri::from_str(black_box(NETWORK_URI)).expect("the static URI must parse"));
    });
    group.bench_function("url_canonical", |bencher| {
        bencher.iter(|| Url::from_str(black_box(NETWORK_URI)).expect("the static URL must parse"));
    });
    group.bench_function("urn_canonical", |bencher| {
        bencher.iter(|| {
            Urn::from_str(black_box("urn:uuid:123e4567-e89b-12d3-a456-426614174000"))
                .expect("the static URN must parse")
        });
    });
    group.bench_function("known_scheme", |bencher| {
        bencher.iter(|| {
            Scheme::from_str(black_box("POSTGRES")).expect("the static scheme must parse")
        });
    });
    group.bench_function("custom_scheme", |bencher| {
        bencher
            .iter(|| Scheme::from_str(black_box("git+ssh")).expect("the static scheme must parse"));
    });
    group.bench_function("windows_drive_normalization", |bencher| {
        bencher.iter(|| {
            Uri::from_path(black_box(WINDOWS_PATH)).expect("the static Windows path must normalize")
        });
    });
    group.bench_function("windows_unc_normalization", |bencher| {
        bencher.iter(|| {
            Uri::from_path(black_box(UNC_PATH)).expect("the static UNC path must normalize")
        });
    });
    group.bench_function("display_parse_round_trip", |bencher| {
        let uri = Uri::from_str(NETWORK_URI).expect("the static URI must parse");
        bencher.iter(|| {
            let canonical = black_box(&uri).to_string();
            Uri::from_str(black_box(&canonical)).expect("canonical URI display must round-trip")
        });
    });
    group.finish();
}

fn value_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("resource_value");
    let uri = Uri::from_str(NETWORK_URI).expect("the static URI must parse");
    let file = Uri::from_path(WINDOWS_PATH).expect("the static Windows path must normalize");
    let encoded = MediaType::from_parts(MimeType::CSV, [MimeType::GZIP, MimeType::ZSTD])
        .expect("the static encodings must be valid");

    group.bench_function("clone", |bencher| {
        bencher.iter(|| black_box(&uri).clone());
    });
    group.bench_function("stable_hash", |bencher| {
        bencher.iter(|| black_box(&uri).stable_hash());
    });
    group.bench_function("component_access", |bencher| {
        bencher.iter(|| {
            let value = black_box(&uri);
            black_box((
                value.scheme().as_str(),
                value.authority().as_str(),
                value.path().as_str(),
                value.query(),
                value.fragment(),
            ))
        });
    });
    group.bench_function("path_segment_iteration", |bencher| {
        bencher.iter(|| black_box(&uri).path_segments().map(str::len).sum::<usize>());
    });
    group.bench_function("extension_iteration", |bencher| {
        bencher.iter(|| black_box(&uri).extensions().map(str::len).sum::<usize>());
    });
    group.bench_function("stem_access", |bencher| {
        bencher.iter(|| black_box(&uri).stem());
    });
    group.bench_function("media_type_inference", |bencher| {
        bencher.iter(|| black_box(&uri).media_type());
    });
    group.bench_function("media_type_mutation", |bencher| {
        bencher.iter(|| {
            let mut value = black_box(&uri).clone();
            value
                .set_media_type(black_box(encoded.clone()))
                .expect("the static media type must have preferred extensions");
            value
        });
    });
    group.bench_function("file_path_projection", |bencher| {
        bencher.iter(|| {
            black_box(&file)
                .to_path()
                .expect("the static file URI must project")
        });
    });
    group.finish();
}

criterion_group!(resource_identifiers, parsing_benchmarks, value_benchmarks);
criterion_main!(resource_identifiers);
