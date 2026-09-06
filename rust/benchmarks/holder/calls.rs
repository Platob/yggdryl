//! What every derived surface costs, in calls to storage and in time.
//!
//! `IOBase` is the one boundary a layer crosses to reach storage, and on a
//! store each crossing is a round trip - so for anything running against S3 or
//! a network filesystem, the *call count* dominates the wall clock by orders
//! of magnitude. A read that costs one call and 60 microseconds locally costs
//! one round trip in production; the same read costing two calls costs two.
//!
//! So each row is named with what it actually cost:
//!
//! ```text
//! calls/bytes/whole read [read_all_bytes=1]
//! calls/records/parquet row count [read_range_bytes=2 size=2 media_type=2 is_container=2]
//! ```
//!
//! The count comes from [`Counted`], which forwards every call to the handle
//! underneath and tallies it, and it is measured once before the timing loop
//! rather than asserted here: the assertions live in
//! `rust/tests/iobase_calls.rs`, which fails when a count changes. This
//! benchmark exists so the count is *visible* beside the time, and so the two
//! can be read against each other - a change that halves the time and doubles
//! the calls is a regression everywhere it matters.
//!
//! The handle underneath is an in-memory [`Buffer`], deliberately: it makes a
//! call as cheap as it can possibly be, so what the timings show is the cost
//! of the layer rather than of the storage. The counts are the same over any
//! backend, because they are a property of the layer.

use std::hint::black_box;
use std::sync::Arc;

use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, Criterion};
use yggdryl::holder::Buffer;
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::holder::counted::{Calls, Counted};
use yggdryl::{DigestAlgorithm, IOBase, Url};

/// The fixture every byte case reads: 1 MiB that does not compress to nothing.
const PAYLOAD: usize = 1024 * 1024;

/// A payload of `size` bytes.
fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|index| (index % 251) as u8).collect()
}

/// A tallied handle over `bytes`, named so its media type is what `url` says.
fn source(bytes: Vec<u8>, url: &str) -> Counted<Buffer> {
    let url = Url::from_str(url).expect("a valid location");
    let mut buffer = Buffer::from_bytes(bytes);
    buffer.set_media_type(url.media_type());
    Counted::new(buffer)
}

/// Time `operation`, naming the row with what one run of it costs in calls.
///
/// The count is taken from one run before the loop, so the name states the
/// steady-state cost. An operation whose first run differs from its later
/// ones, a cache warming or a footer being parsed, is measured separately
/// where that difference is the point.
fn measured(
    group: &mut BenchmarkGroup<'_, WallTime>,
    name: &str,
    calls: &Arc<Calls>,
    mut operation: impl FnMut(),
) {
    calls.reset();
    operation();
    let counted = calls.snapshot();
    group.bench_function(format!("{name} [{counted}]"), |bencher| {
        bencher.iter(&mut operation);
    });
}

/// The byte surfaces: whole, ranged, streamed, digested, and the metadata.
fn byte_calls(criterion: &mut Criterion) {
    let handle = source(payload(PAYLOAD), "file:///lake/part.bin");
    let calls = Arc::clone(handle.calls());
    let mut group = criterion.benchmark_group("calls/bytes");

    measured(&mut group, "whole read", &calls, || {
        black_box(handle.read_all_bytes().expect("a read"));
    });
    measured(&mut group, "footer read", &calls, || {
        black_box(
            handle
                .read_range_bytes(PAYLOAD as u64 - 8192, 8192)
                .expect("a read"),
        );
    });
    measured(&mut group, "stream drain", &calls, || {
        for chunk in handle.pstream_bytes(0, 64 * 1024).expect("a stream") {
            black_box(chunk.expect("bytes"));
        }
    });
    measured(&mut group, "whole digest", &calls, || {
        black_box(handle.read_digest(DigestAlgorithm::Xxh3).expect("a digest"));
    });
    measured(&mut group, "ranged digest", &calls, || {
        black_box(
            handle
                .read_range_digest(0, 16, DigestAlgorithm::Xxh3)
                .expect("a digest"),
        );
    });
    measured(&mut group, "length", &calls, || {
        black_box(handle.size());
    });
    measured(&mut group, "kind", &calls, || {
        black_box(handle.kind());
    });

    // The two `std::io` drains, which without an override cost one call per
    // doubling of the reader's buffer rather than one for the value.
    measured(&mut group, "reader drained to the end", &calls, || {
        use std::io::Read;
        let mut into = Vec::new();
        handle.reader_at(0).read_to_end(&mut into).expect("a read");
        black_box(into);
    });
    group.finish();
}

/// A content coding, where the hazard is measuring the value before reading it.
fn coding_calls(criterion: &mut Criterion) {
    let payload = payload(PAYLOAD);
    let encoded = yggdryl::Codec::Gzip.dump(&payload).expect("an encoding");
    let inner = source(encoded, "file:///lake/part.bin.gz");
    let calls = Arc::clone(inner.calls());
    let coding = yggdryl::coding::Coding::new(inner, yggdryl::Codec::Gzip);
    let mut group = criterion.benchmark_group("calls/coding");

    measured(&mut group, "whole read", &calls, || {
        black_box(coding.read_all_bytes().expect("a read"));
    });
    measured(&mut group, "ranged read", &calls, || {
        black_box(coding.read_range_bytes(0, 8192).expect("a read"));
    });
    measured(&mut group, "decoded length", &calls, || {
        black_box(coding.size());
    });
    group.finish();
}

/// A page cache, where a hit should reach the handle for nothing at all.
fn cache_calls(criterion: &mut Criterion) {
    let inner = source(payload(PAYLOAD), "file:///lake/part.bin");
    let calls = Arc::clone(inner.calls());
    let cached = inner.buffered(BufferedOptions::default());
    let mut group = criterion.benchmark_group("calls/cache");

    // Warmed first, so the row measures the hit rather than the fill.
    black_box(cached.read_range_bytes(0, 8192).expect("a read"));
    measured(&mut group, "warm ranged read", &calls, || {
        black_box(cached.read_range_bytes(0, 8192).expect("a read"));
    });
    measured(&mut group, "length", &calls, || {
        black_box(cached.size());
    });
    group.finish();
}

/// Listings and partition selection over a lake of a hundred files.
fn listing_calls(criterion: &mut Criterion) {
    use yggdryl::holder::fs::{BoundLocation, FileSystem, MemoryFileSystem, located};

    const PARTITIONS: usize = 20;
    const PER_PARTITION: usize = 5;

    let filesystem: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    for partition in 0..PARTITIONS {
        let directory = format!("/lake/year=2026/month={partition:02}");
        filesystem
            .create_dir(&directory, true)
            .expect("a directory");
        for part in 0..PER_PARTITION {
            let mut sink = filesystem
                .open_output_stream(&format!("{directory}/part-{part}.txt"), None)
                .expect("an output stream");
            sink.write(b"AAPL,187.23\n").expect("a write");
            sink.close().expect("a close");
        }
    }

    let lake = located(BoundLocation::new(filesystem, "/lake", None).expect("a location"));
    let counted = Counted::new(lake);
    let calls = Arc::clone(counted.calls());
    let mut group = criterion.benchmark_group("calls/listing");

    measured(&mut group, "recursive listing of 100", &calls, || {
        black_box(counted.ls(true, false).count());
    });
    measured(&mut group, "glob over 100", &calls, || {
        black_box(counted.glob("**/*.txt", false).expect("a glob").count());
    });
    measured(&mut group, "one partition of twenty", &calls, || {
        black_box(
            counted
                .children_where(&[("month", "05")], false)
                .expect("a selection")
                .count(),
        );
    });
    measured(&mut group, "partitions of the location", &calls, || {
        black_box(counted.partitions());
    });
    group.finish();
}

pub(crate) fn call_benchmarks(criterion: &mut Criterion) {
    byte_calls(criterion);
    coding_calls(criterion);
    cache_calls(criterion);
    listing_calls(criterion);
    #[cfg(feature = "arrow")]
    records::record_call_benchmarks(criterion);
}

/// The record encodings, where a dimension must never decode a row.
#[cfg(feature = "arrow")]
mod records {
    use std::hint::black_box;
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use criterion::Criterion;
    use yggdryl::holder::Buffer;
    use yggdryl::holder::counted::Counted;
    use yggdryl::{IOBase, IOMedia, Url};

    use super::measured;

    /// Rows enough that a dimension answered by decoding them would show.
    const ROWS: usize = 50_000;

    fn batch(rows: usize) -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            ArrowField::new("id", ArrowDataType::Int64, false),
            ArrowField::new("symbol", ArrowDataType::Utf8, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from((0..rows as i64).collect::<Vec<_>>())),
                Arc::new(StringArray::from(
                    (0..rows)
                        .map(|index| format!("SYM{index:04}"))
                        .collect::<Vec<_>>(),
                )),
            ],
        )
        .expect("a batch")
    }

    /// A tallied handle holding `ROWS` rows in the encoding `url` names.
    fn written(url: &str) -> Counted<Buffer> {
        let media_type = Url::from_str(url).expect("a valid location").media_type();
        let mut sink = Buffer::new();
        sink.set_media_type(media_type.clone());
        let options = sink.record_options().expect("record options");
        sink.overwrite_arrow_batch(batch(ROWS), &options)
            .expect("a write");
        let mut source = Buffer::from_bytes(sink.read_all_bytes().expect("the bytes"));
        source.set_media_type(media_type);
        Counted::new(source)
    }

    fn surfaces(criterion: &mut Criterion, label: &str, url: &str) {
        let handle = written(url);
        let calls = Arc::clone(handle.calls());
        let options = handle.record_options().expect("record options");
        let mut group = criterion.benchmark_group(format!("calls/records/{label}"));

        measured(&mut group, "schema", &calls, || {
            black_box(handle.read_arrow_field(&options).expect("a field"));
        });
        measured(&mut group, "column count", &calls, || {
            black_box(handle.column_size().expect("columns"));
        });
        measured(&mut group, "row count", &calls, || {
            black_box(handle.row_size().expect("rows"));
        });
        measured(&mut group, "full read", &calls, || {
            for batch in handle.read_arrow_reader(&options).expect("a reader") {
                black_box(batch.expect("a batch"));
            }
        });
        group.finish();
    }

    pub(crate) fn record_call_benchmarks(criterion: &mut Criterion) {
        surfaces(criterion, "ipc", "file:///lake/part.arrow");
        surfaces(criterion, "avro", "file:///lake/part.avro");
        #[cfg(feature = "parquet")]
        surfaces(criterion, "parquet", "file:///lake/part.parquet");
    }
}
