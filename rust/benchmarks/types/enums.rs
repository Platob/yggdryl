use std::hint::black_box;
use std::path::Path;

use criterion::Criterion;
use yggdryl::{IOMode, MediaType, MimeType};

const KNOWN_MIME: &str = "application/vnd.apache.parquet";
const UPPERCASE_MIME: &str = "APPLICATION/VND.APACHE.PARQUET";
const CUSTOM_MIME: &str = "application/vnd.acme.market-depth+json";
const COMPOUND_FILE: &str = "orders.2026.csv.gz.zst";
const CONTENT_TYPE: &str = "text/csv; charset=\"utf-8\"";
const WIDE_CONTENT_TYPE: &str = "application/vnd.acme+json; a=1; b=2; c=3; d=4; e=5; f=6; g=7; h=8; i=9; j=10; k=11; l=12; m=13; n=14; o=15; p=16; q1=17; r=18; s=19; t=20; u=21; v=22; w=23; x=24; y=25; z=26; aa=27; ab=28; ac=29; ad=30; ae=31; af=32";

pub(crate) fn mime_parsing(criterion: &mut Criterion) {
    let path = Path::new("warehouse/2026/orders.parquet");
    let mut group = criterion.benchmark_group("mime_parse");
    group.bench_function("known_canonical", |bencher| {
        bencher.iter(|| MimeType::from_str(black_box(KNOWN_MIME)).expect("fixture must parse"));
    });
    group.bench_function("known_uppercase", |bencher| {
        bencher.iter(|| MimeType::from_str(black_box(UPPERCASE_MIME)).expect("fixture must parse"));
    });
    group.bench_function("custom_structured", |bencher| {
        bencher.iter(|| MimeType::from_str(black_box(CUSTOM_MIME)).expect("fixture must parse"));
    });
    group.bench_function("extension", |bencher| {
        bencher
            .iter(|| MimeType::from_extension(black_box("parquet")).expect("fixture must infer"));
    });
    group.bench_function("borrowed_path", |bencher| {
        bencher.iter(|| MimeType::from_path(black_box(path)).expect("fixture must infer"));
    });
    group.finish();
}

pub(crate) fn media_inference(criterion: &mut Criterion) {
    let canonical = MediaType::from_file_name(COMPOUND_FILE).to_string();
    let mut group = criterion.benchmark_group("media_infer");
    group.bench_function("compound_suffix", |bencher| {
        bencher.iter(|| MediaType::from_file_name(black_box(COMPOUND_FILE)));
    });
    group.bench_function("encoding_only", |bencher| {
        bencher.iter(|| MediaType::from_file_name(black_box("payload.gz")));
    });
    group.bench_function("unknown_before_encoding", |bencher| {
        bencher.iter(|| MediaType::from_file_name(black_box("orders.csv.backup.gz")));
    });
    group.bench_function("content_type_one_parameter", |bencher| {
        bencher.iter(|| {
            MimeType::from_content_type(black_box(CONTENT_TYPE)).expect("fixture must parse")
        });
    });
    group.bench_function("content_type_wide_parameters", |bencher| {
        bencher.iter(|| {
            MimeType::from_content_type(black_box(WIDE_CONTENT_TYPE)).expect("fixture must parse")
        });
    });
    group.bench_function("content_headers", |bencher| {
        bencher.iter(|| {
            MediaType::from_content_headers(
                black_box(Some(CONTENT_TYPE)),
                black_box(Some("gzip, zstd")),
            )
            .expect("fixture must parse")
        });
    });
    group.bench_function("display_parse_round_trip", |bencher| {
        bencher.iter(|| MediaType::from_str(black_box(&canonical)).expect("fixture must parse"));
    });
    group.finish();
}

pub(crate) fn write_modes_and_io_identity(criterion: &mut Criterion) {
    let encoded = MediaType::from_parts(MimeType::CSV, [MimeType::GZIP]).unwrap();
    let mut group = criterion.benchmark_group("enum_accessors");
    group.bench_function("mime_is_io", |bencher| {
        bencher.iter(|| black_box(MimeType::CSV).is_io());
    });
    group.bench_function("media_is_io", |bencher| {
        bencher.iter(|| black_box(&encoded).is_io());
    });
    group.bench_function("write_mode_parse", |bencher| {
        bencher.iter(|| IOMode::from_str(black_box("overwrite")).unwrap());
    });
    group.bench_function("write_mode_name", |bencher| {
        bencher.iter(|| black_box(IOMode::Merge).as_str());
    });
    group.finish();
}
