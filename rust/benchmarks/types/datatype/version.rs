//! The two hot operations owned by the generic version value.

use std::hint::black_box;

use criterion::Criterion;
use yggdryl::Version;

pub(crate) fn version_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("version");
    group.bench_function("parse", |bencher| {
        bencher.iter(|| {
            black_box("5.0.10")
                .parse::<Version>()
                .expect("the static version is valid")
        });
    });
    group.bench_function("parse_service_pack", |bencher| {
        bencher.iter(|| {
            black_box("5.0sp250")
                .parse::<Version>()
                .expect("the compact FIX version is valid")
        });
    });

    let left = "5.0.2"
        .parse::<Version>()
        .expect("the static version is valid");
    let right = "5.0.10"
        .parse::<Version>()
        .expect("the static version is valid");
    group.bench_function("compare", |bencher| {
        bencher.iter(|| black_box(left).cmp(black_box(&right)));
    });
    group.bench_function("parse_maximum", |bencher| {
        bencher.iter(|| {
            black_box("255.255.65535")
                .parse::<Version>()
                .expect("fixed width maximum")
        });
    });
    group.bench_function("native_parts", |bencher| {
        bencher.iter(|| {
            let version = black_box(Version::new(255, 255, 65535));
            black_box((version.major(), version.minor(), version.patch()))
        });
    });
    group.finish();
}
