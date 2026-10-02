//! Declared result schemas: resolve a projection without opening record bytes.

use criterion::{Criterion, Throughput};
use std::hint::black_box;
use yggdryl::holder::Buffer;
use yggdryl::ipc::IpcOptions;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{DataType, IOMedia, StructType};

pub(crate) fn options_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("record_result_field");
    let source = Buffer::from_bytes(b"unread header".to_vec());
    for columns in [
        crate::bench_profile::corpus(64, 4),
        crate::bench_profile::corpus(4_096, 8),
    ] {
        let field = StructType::from_fields(
            (0..columns).map(|index| DataType::Int64.required_field(format!("c{index:04}"))),
        )
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let identity = RecordOptions::Ipc(IpcOptions::new().with_field(field.clone()));
        let selected = identity
            .clone()
            .with_select("c0000 as key")
            .unwrap()
            .with_filter("key > 0")
            .unwrap();
        assert_eq!(source.read_arrow_field(&identity).unwrap(), field);
        assert_eq!(
            source.read_arrow_field(&selected).unwrap().fields()[0].name(),
            "key"
        );
        group.throughput(Throughput::Elements(columns as u64));
        for (name, options) in [("identity", identity), ("selected", selected)] {
            group.bench_function(format!("{name}/{columns}"), |bencher| {
                bencher.iter(|| black_box(source.read_arrow_field(black_box(&options)).unwrap()));
            });
        }
    }
    group.finish();
}
