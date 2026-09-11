use std::hint::black_box;
use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, Throughput};
use yggdryl::{FixCodec, FixRegistry, Scalar, UlPlugin};

fn document(count: usize) -> Scalar {
    let values = (0..count).map(|index| {
        let name = format!("Plugin{index:04}");
        let object = format!(
            "com.ullink.ulbridge.sessioninterfaces.plugins:name={name},plugin-type=FIX,type=Plugin"
        );
        let attributes = Scalar::from_record([
            ("Name", Scalar::from(name)),
            ("CurrentPort", Scalar::from(9000_i64)),
        ])
        .expect("bounded static attributes");
        (object, attributes)
    });
    Scalar::from_record([
        (
            "request",
            Scalar::from_record([
                (
                    "mbean",
                    Scalar::from("com.ullink.ulbridge.sessioninterfaces.plugins:*"),
                ),
                ("type", Scalar::from("read")),
            ])
            .expect("request fields"),
        ),
        (
            "value",
            Scalar::from_record(values).expect("distinct ObjectNames"),
        ),
        ("status", Scalar::from(200_i64)),
    ])
    .expect("response fields")
}

pub fn benchmarks(criterion: &mut Criterion) {
    let registry = FixRegistry::new()
        .with_ulbridge_fields()
        .expect("UL fields");
    assert!(
        registry
            .field(yggdryl::MBEAN_TAG_NAME.0)
            .expect("the bridge's first field")
            .as_fix()
            .has_branch(yggdryl::ULBRIDGE_DIALECT)
    );
    let codec = FixCodec::new(Arc::new(registry));
    let mut group = criterion.benchmark_group("fix/ulconfig");
    for count in [1, 32, crate::bench_profile::corpus(256, 64)] {
        let document = document(count);
        assert_eq!(
            UlPlugin::from_json_scalar(&document).unwrap().count(),
            count
        );
        let first = UlPlugin::from_json_scalar(&document)
            .unwrap()
            .next()
            .unwrap();
        assert!(
            first
                .into_fixmsg(&codec)
                .unwrap()
                .get_by_name("SessionInterface")
                .is_some()
        );
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(
            BenchmarkId::new("stable_hash_one_state_allocation", count),
            &first,
            |b, configuration| {
                b.iter(|| black_box(configuration.stable_hash()));
            },
        );
        group.bench_with_input(
            BenchmarkId::new("first", count),
            &document,
            |b, document| {
                b.iter(|| {
                    UlPlugin::from_json_scalar(black_box(document))
                        .expect("valid wildcard response")
                        .next()
                });
            },
        );
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::new("drain", count),
            &document,
            |b, document| {
                b.iter(|| {
                    UlPlugin::from_json_scalar(black_box(document))
                        .expect("valid wildcard response")
                        .map(black_box)
                        .count()
                });
            },
        );
        group.bench_with_input(
            BenchmarkId::new("messages", count),
            &document,
            |b, document| {
                b.iter(|| {
                    for configuration in UlPlugin::from_json_scalar(black_box(document))
                        .expect("valid wildcard response")
                    {
                        black_box(configuration.into_fixmsg(&codec).expect("flat UL row"));
                    }
                });
            },
        );
    }
    group.finish();
}
