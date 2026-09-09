use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use yggdryl::{MediaType, MimeType, Scheme, Uri, Url, Urn};

const NETWORK_URI: &str =
    "https://user@example.test:8443/archive/2026/report.tar.zst?download=1#summary";
const WINDOWS_PATH: &str = r"C:\Users\Ada Lovelace\market data\trades.parquet";
const UNC_PATH: &str = r"\\market-data\shared\prices\2026\ticks.arrow";
const S3_URI: &str = "s3://market-data.s3.eu-west-3.amazonaws.com/2026/trades.parquet";
const ESCAPED_URI: &str = "https://example.test/archive/Ada%20Lovelace/report.csv?as%20of=2026-01-02&note=a%26b&venue=XNAS";
/// A platform name every byte of which the segment syntax has to escape.
const ESCAPING_NAME: &str = "Ada Lovelace 100% caf\u{e9}/report #1?draft.csv";

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
    let credentials = Uri::from_str(NETWORK_URI).expect("the static URI must parse");
    let s3 = Uri::from_str(S3_URI).expect("the static S3 URI must parse");
    let lake = Url::from_str("file:///lake").expect("the static lake URL must parse");

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
                value.query(false).expect("a raw query never decodes"),
                value.fragment(false).expect("a raw fragment never decodes"),
            ))
        });
    });
    group.bench_function("credential_access", |bencher| {
        bencher.iter(|| {
            let value = black_box(&credentials);
            black_box((value.user(), value.password(), value.hostname()))
        });
    });
    group.bench_function("s3_location_access", |bencher| {
        bencher.iter(|| {
            let value = black_box(&s3);
            black_box((value.hostname(), value.bucket(), value.region()))
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
    group.bench_function("component_decoding", |bencher| {
        let escaped = Uri::from_str(ESCAPED_URI).expect("the static escaped URI must parse");
        bencher.iter(|| {
            let value = black_box(&escaped);
            black_box((
                value.path_text(true).expect("the static path must decode"),
                value.query(true).expect("the static query must decode"),
            ))
        });
    });
    group.bench_function("parameter_pairs_raw", |bencher| {
        let escaped = Uri::from_str(ESCAPED_URI).expect("the static escaped URI must parse");
        bencher.iter(|| {
            black_box(&escaped)
                .parameters(false)
                .expect("a raw view never decodes")
                .len()
        });
    });
    group.bench_function("parameter_pairs_decoded", |bencher| {
        let escaped = Uri::from_str(ESCAPED_URI).expect("the static escaped URI must parse");
        bencher.iter(|| {
            black_box(&escaped)
                .parameters(true)
                .expect("the static pairs must decode")
                .len()
        });
    });
    group.bench_function("parameter_write_back", |bencher| {
        let escaped = Uri::from_str(ESCAPED_URI).expect("the static escaped URI must parse");
        bencher.iter(|| {
            let mut value = black_box(&escaped).clone();
            let mut parameters = value
                .parameters(true)
                .expect("the static pairs must decode")
                .into_owned();
            parameters
                .insert("as of", "2026-01-02 09:30")
                .expect("a decoding view encodes what it is given");
            value
                .set_parameters(&parameters)
                .expect("the edited pairs must spell a query");
            value
        });
    });
    group.bench_function("file_path_projection", |bencher| {
        bencher.iter(|| {
            black_box(&file)
                .clone()
                .into_path()
                .expect("the static file URI must project")
        });
    });
    // The two join doors, measured against each other: the platform one pays
    // for encoding every component, the URI one takes text already spelled.
    group.bench_function("platform_join_clean", |bencher| {
        bencher.iter(|| {
            black_box(&lake)
                .join_path(black_box("year=2026/part-0.parquet"))
                .expect("a clean platform name must join")
        });
    });
    group.bench_function("platform_join_escaping", |bencher| {
        bencher.iter(|| {
            black_box(&lake)
                .join_path(black_box(ESCAPING_NAME))
                .expect("an escaping platform name must join")
        });
    });
    group.bench_function("uri_join", |bencher| {
        bencher.iter(|| {
            black_box(&lake)
                .joinpath(black_box("year=2026/part-0.parquet"))
                .expect("the static URI path must join")
        });
    });
    group.finish();
}

criterion_group!(resource_identifiers, parsing_benchmarks, value_benchmarks);
criterion_main!(resource_identifiers);
