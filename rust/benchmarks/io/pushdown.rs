//! Column pushdown: what a subset read costs against what a whole read costs.
//!
//! Each case reports the bytes it materializes as its Criterion throughput, so
//! the claim "the subset read moves less data" is a measured number and not an
//! inference from elapsed time. The Parquet pair is the one where the saving is
//! also a decoding saving: its column chunks are separately addressable, so a
//! masked read never decompresses what it skipped. The Arrow IPC pair shares
//! the allocation saving but still reads the whole message body, because an IPC
//! record batch is one contiguous message. The Avro pair sits between the two:
//! rows interleave columns, so the whole block is still decompressed, but an
//! unselected column's bytes are skipped instead of decoded.
//!
//! The subset is declared as the read's schema, which is what pushes it into
//! the encoding. It also casts, so the measured saving is what survives after
//! the declared shape has been produced rather than a projection nobody used.

use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::generic::IORecordOptions;
use yggdryl::io::IOBase;

use super::{materialized, narrow, stored};

pub(crate) fn projection_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("io_pushdown");

    for (label, name) in [
        ("ipc", "bench.arrows"),
        ("parquet", "bench.parquet"),
        ("avro", "bench.avro"),
    ] {
        if label == "parquet" && !cfg!(feature = "parquet") {
            continue;
        }
        let handle = stored(name);
        let whole_options = handle.record_options().expect("an implemented encoding");
        let subset_options = whole_options.clone().with_schema(narrow());

        // Measured once, outside the loop: the fixture never changes.
        let whole_bytes = materialized(
            handle
                .read_arrow_batch_reader(&whole_options)
                .expect("the fixture must read"),
        );
        let subset_bytes = materialized(
            handle
                .read_arrow_batch_reader(&subset_options)
                .expect("the fixture must read"),
        );
        assert!(
            subset_bytes < whole_bytes,
            "{label}: a projected read must move less than a whole one, \
             got {subset_bytes} against {whole_bytes}"
        );

        group.throughput(Throughput::Bytes(whole_bytes));
        group.bench_function(format!("{label}/whole"), |bencher| {
            bencher.iter(|| {
                materialized(
                    handle
                        .read_arrow_batch_reader(black_box(&whole_options))
                        .expect("the fixture must read"),
                )
            });
        });

        group.throughput(Throughput::Bytes(subset_bytes));
        group.bench_function(format!("{label}/subset"), |bencher| {
            bencher.iter(|| {
                materialized(
                    handle
                        .read_arrow_batch_reader(black_box(&subset_options))
                        .expect("the fixture must read"),
                )
            });
        });
    }

    group.finish();
}
