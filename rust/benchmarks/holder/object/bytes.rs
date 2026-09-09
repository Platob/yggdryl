//! Byte-level S3 access, against `object_store` on the same store.
//!
//! Four shapes, because they are the four a reader over object storage
//! actually performs: the whole value, one range out of the middle (a footer
//! read), a full streamed drain, and a whole write.

use std::hint::black_box;
use std::io::Read as _;

use criterion::{Criterion, Throughput};
use futures::StreamExt as _;
use object_store::ObjectStoreExt as _;
use yggdryl::IOBase;
use yggdryl::holder::object::AwsOptions;

use super::{BUCKET, PAYLOAD, baseline, baseline_path, location, options, payload, runtime, store};

/// The range a footer read asks for, in bytes.
const FOOTER: usize = 8 * 1024;

pub(crate) fn byte_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("object_bytes");
    group.throughput(Throughput::Bytes(PAYLOAD as u64));

    let store = store();
    let bytes = payload(PAYLOAD);
    store.put(BUCKET, "bench/read.bin", &bytes);
    let handle = yggdryl::holder::object::file_with(&location("bench/read.bin"), options(&store))
        .expect("an object handle");
    let runtime = runtime();
    let external = baseline(&store);
    let key = baseline_path("bench/read.bin");

    // A whole read: one GET either way, so this is the transfer and the
    // decode path around it.
    group.bench_function("read_all/yggdryl", |bencher| {
        bencher.iter(|| black_box(black_box(&handle).read_all_bytes().expect("the object")));
    });
    group.bench_function("read_all/object_store", |bencher| {
        bencher.iter(|| {
            runtime.block_on(async {
                black_box(
                    external
                        .get(black_box(&key))
                        .await
                        .expect("the object")
                        .bytes()
                        .await
                        .expect("the bytes"),
                )
            })
        });
    });

    // A footer read, which is the shape that decides what a Parquet scan
    // costs: the range must be the transfer, not the file.
    group.throughput(Throughput::Bytes(FOOTER as u64));
    group.bench_function("read_footer/yggdryl", |bencher| {
        let offset = (PAYLOAD - FOOTER) as u64;
        bencher.iter(|| {
            black_box(
                black_box(&handle)
                    .read_range_bytes(offset, FOOTER)
                    .expect("the footer"),
            )
        });
    });
    group.bench_function("read_footer/object_store", |bencher| {
        let range = (PAYLOAD - FOOTER) as u64..PAYLOAD as u64;
        bencher.iter(|| {
            runtime.block_on(async {
                black_box(
                    external
                        .get_range(black_box(&key), range.clone())
                        .await
                        .expect("the footer"),
                )
            })
        });
    });

    // A streamed drain, which is what a record reader does: one request,
    // consumed in bounded pieces.
    group.throughput(Throughput::Bytes(PAYLOAD as u64));
    group.bench_function("stream_drain/yggdryl", |bencher| {
        bencher.iter(|| {
            let mut stream = black_box(&handle)
                .pstream_bytes(0, 64 * 1024)
                .expect("a stream");
            let mut window = vec![0_u8; 64 * 1024];
            let mut total = 0_usize;
            loop {
                let read = stream.read(&mut window).expect("a chunk");
                if read == 0 {
                    break;
                }
                total += read;
            }
            black_box(total)
        });
    });
    group.bench_function("stream_drain/object_store", |bencher| {
        bencher.iter(|| {
            runtime.block_on(async {
                let mut stream = external
                    .get(black_box(&key))
                    .await
                    .expect("the object")
                    .into_stream();
                let mut total = 0_usize;
                while let Some(chunk) = stream.next().await {
                    total += chunk.expect("a chunk").len();
                }
                black_box(total)
            })
        });
    });

    // A whole write: one PUT either way. The fixture endpoint is plain HTTP,
    // so the default policy hashes the body for `x-amz-content-sha256` -
    // which is what nothing else would protect it here. The unsigned leg is
    // the policy an HTTPS endpoint selects by default, and the one
    // `object_store` uses, so the three numbers separate the client from the
    // policy rather than confusing them.
    group.bench_function("write_all/yggdryl", |bencher| {
        let mut target =
            yggdryl::holder::object::file_with(&location("bench/write.bin"), options(&store))
                .expect("an object handle");
        bencher.iter(|| target.write_all_bytes(black_box(&bytes)).expect("a write"));
    });
    group.bench_function("write_all/yggdryl_unsigned_payload", |bencher| {
        let mut target = yggdryl::holder::object::file_with(
            &location("bench/write-unsigned.bin"),
            options(&store).with_aws(AwsOptions::default().with_payload_signing(false)),
        )
        .expect("an object handle");
        bencher.iter(|| target.write_all_bytes(black_box(&bytes)).expect("a write"));
    });
    group.bench_function("write_all/object_store", |bencher| {
        let target = baseline_path("bench/write-baseline.bin");
        bencher.iter(|| {
            runtime.block_on(async {
                external
                    .put(&target, bytes.clone().into())
                    .await
                    .expect("a write")
            })
        });
    });

    group.finish();
}
