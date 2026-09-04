//! The WKB reader against payload size: decode and single-pass bounds.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use yggdryl::types::geospatial::wkb::{self, Geometry};

/// A little-endian XY point: order byte, type code 1, then x and y.
fn point(x: f64, y: f64) -> Vec<u8> {
    let mut bytes = vec![1, 1, 0, 0, 0];
    bytes.extend(x.to_le_bytes());
    bytes.extend(y.to_le_bytes());
    bytes
}

/// A little-endian XY linestring walking `vertices` positions.
fn line_string(vertices: u32) -> Vec<u8> {
    let mut bytes = vec![1, 2, 0, 0, 0];
    bytes.extend(vertices.to_le_bytes());
    for index in 0..vertices {
        bytes.extend(f64::from(index).to_le_bytes());
        bytes.extend((f64::from(index) / 2.0).to_le_bytes());
    }
    bytes
}

/// A geometry collection over the given members, each already spelled as WKB.
fn collection(members: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = vec![1, 7, 0, 0, 0];
    bytes.extend(
        u32::try_from(members.len())
            .expect("fixture member counts fit u32")
            .to_le_bytes(),
    );
    for member in members {
        bytes.extend_from_slice(member);
    }
    bytes
}

pub(crate) fn geospatial_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("geospatial");
    // Three payload sizes: one point, a thousand-vertex path, and a
    // collection nesting another collection, so decode and bounds report
    // throughput against the bytes each actually walks.
    let nested = collection(&[
        point(1.0, 2.0),
        line_string(16),
        collection(&[point(3.0, 4.0), line_string(8)]),
    ]);
    let payloads = [
        ("point", point(10.0, 20.0)),
        ("line_string_1e3", line_string(1_000)),
        ("nested_collection", nested),
    ];

    for (name, bytes) in &payloads {
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(BenchmarkId::new("decode", name), bytes, |bencher, bytes| {
            bencher
                .iter(|| Geometry::from_slice(black_box(bytes)).expect("the fixture is valid WKB"));
        });
        group.bench_with_input(
            BenchmarkId::new("bounding_box", name),
            bytes,
            |bencher, bytes| {
                bencher.iter(|| {
                    wkb::bounding_box(black_box(bytes)).expect("the fixture is valid WKB")
                });
            },
        );
    }
    group.finish();
}
