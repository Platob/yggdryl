//! What a member read costs, by the coding the member is stored under.
//!
//! The backend's one performance claim is that a stored member is the
//! archive's own bytes over a range, so a positional read of one is a
//! positional read of the archive and nothing more. The three read legs are
//! reported together because that claim only means something beside the two
//! that do decode: a closed compressed read, which decodes to the offset and
//! keeps nothing, and an opened one, which decodes once and keeps the member.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};
use yggdryl::holder::zip::{Archive, Node};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::{Codec, IOBase};

/// The member size the read legs are measured at.
const MEMBER_LEN: usize = 1 << 20;

/// The byte range one positional read asks for.
const READ_LEN: usize = 512;

/// The decoded distance between the restart points a member is written with.
const STRIDE: u64 = 64 * 1024;

/// An offset inside a unit rather than on the point that begins it.
///
/// Half the member lands exactly on a point at this stride, which would
/// measure the one case a seek never has to decode for.
const MID_UNIT: u64 = (MEMBER_LEN / 2) as u64 + STRIDE / 2;

/// How many members the listing and write legs are measured over.
const MEMBERS: usize = crate::bench_profile::corpus(2_000, 100);

/// A payload that compresses, so a stored and a deflated member differ.
fn payload(len: usize) -> Vec<u8> {
    b"symbol,price,venue\nAAPL,187.23,XNAS\n"
        .iter()
        .copied()
        .cycle()
        .take(len)
        .collect()
}

/// An in-memory archive holding one member under `codec`.
fn one_member(codec: Codec) -> Node {
    let root = Archive::new(Holder::buffer(Buffer::new())).mount();
    root.archive()
        .write_member_with("blob.bin", &payload(MEMBER_LEN), codec)
        .expect("the member writes");
    root.archive().flush().expect("the directory publishes");
    root
}

/// An in-memory archive of `count` members across ten directories.
fn many_members(count: usize) -> Node {
    let root = Archive::new(Holder::buffer(Buffer::new())).mount();
    let payload = payload(64);
    for member in 0..count {
        root.archive()
            .write_member(
                &format!("part={:02}/part-{member:06}.csv", member % 10),
                &payload,
            )
            .expect("the member writes");
    }
    root.archive().flush().expect("the directory publishes");
    root
}

pub(crate) fn zip_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("io_zip");
    group.throughput(Throughput::Bytes(READ_LEN as u64));

    // A stored member never decodes, so this leg is the archive's own pread.
    let stored = one_member(Codec::Identity);
    let stored_member = stored.as_leaf("blob.bin").expect("a member");
    group.bench_function("read/stored", |bencher| {
        bencher.iter(|| {
            black_box(&stored_member)
                .read_range_bytes((MEMBER_LEN / 2) as u64, READ_LEN)
                .expect("a range")
        });
    });

    // A closed compressed read decodes from the restart point before the
    // offset, so what it costs is one unit rather than the whole prefix.
    let deflated = one_member(Codec::Deflate);
    let deflated_member = deflated.as_leaf("blob.bin").expect("a member");
    group.bench_function("read/deflate_closed", |bencher| {
        bencher.iter(|| {
            black_box(&deflated_member)
                .read_range_bytes(MID_UNIT, READ_LEN)
                .expect("a range")
        });
    });

    // The same read where the offset is a point, which decodes nothing but
    // the bytes it answers.
    group.bench_function("read/deflate_onpoint", |bencher| {
        bencher.iter(|| {
            black_box(&deflated_member)
                .read_range_bytes((MEMBER_LEN / 2) as u64, READ_LEN)
                .expect("a range")
        });
    });

    // A member written solid, which is what another writer's member is: every
    // read of one decodes from its first byte.
    let solid = {
        let root = Archive::new(Holder::buffer(Buffer::new()))
            .with_restart_stride(0)
            .mount();
        root.archive()
            .write_member_with("blob.bin", &payload(MEMBER_LEN), Codec::Deflate)
            .expect("the member writes");
        root.archive().flush().expect("the directory publishes");
        root
    };
    let solid_member = solid.as_leaf("blob.bin").expect("a member");
    group.bench_function("read/deflate_solid", |bencher| {
        bencher.iter(|| {
            black_box(&solid_member)
                .read_range_bytes(MID_UNIT, READ_LEN)
                .expect("a range")
        });
    });

    // A scan that walks backwards, which is where a map turns a quadratic
    // cost into a linear one.
    group.throughput(Throughput::Elements(16));
    for (name, member) in [
        ("mapped", &deflated_member),
        ("solid", &solid_member),
    ] {
        group.bench_function(BenchmarkId::new("read/backward", name), |bencher| {
            bencher.iter(|| {
                for step in (0..16).rev() {
                    black_box(member)
                        .read_range_bytes(step * STRIDE, READ_LEN)
                        .expect("a range");
                }
            });
        });
    }
    group.throughput(Throughput::Bytes(READ_LEN as u64));

    // An opened one decodes once and answers from the member it holds.
    let mut opened_member = deflated.as_leaf("blob.bin").expect("a member");
    opened_member.open().expect("the decoded member");
    group.bench_function("read/deflate_opened", |bencher| {
        bencher.iter(|| {
            black_box(&opened_member)
                .read_range_bytes((MEMBER_LEN / 2) as u64, READ_LEN)
                .expect("a range")
        });
    });

    // Whole-member reads, where the digest check is part of the answer.
    group.throughput(Throughput::Bytes(MEMBER_LEN as u64));
    for (name, root) in [("stored", &stored), ("deflate", &deflated)] {
        let member = root.as_leaf("blob.bin").expect("a member");
        group.bench_function(BenchmarkId::new("read_all", name), |bencher| {
            bencher.iter(|| black_box(&member).read_all_bytes().expect("the member"));
        });
    }

    // The route a caller actually takes: resolve a location, then read it.
    // A resolving location owns no state of its own, so every handle call it
    // makes is one the archive answers or one it has to issue.
    group.throughput(Throughput::Bytes(READ_LEN as u64));
    stored
        .child_by_path("blob.bin")
        .expect("a member")
        .read_range_bytes(0, READ_LEN)
        .expect("a range");
    group.bench_function("read/through_path", |bencher| {
        bencher.iter(|| {
            black_box(&stored)
                .child_by_path("blob.bin")
                .expect("a member")
                .read_range_bytes((MEMBER_LEN / 2) as u64, READ_LEN)
                .expect("a range")
        });
    });

    // A member streamed in from a reader, which is what a write of something
    // larger than memory costs against a write of something already in it.
    group.throughput(Throughput::Bytes(MEMBER_LEN as u64));
    let streamed = payload(MEMBER_LEN);
    for (name, stride) in [("mapped", STRIDE), ("solid", 0)] {
        group.bench_function(BenchmarkId::new("write/stream", name), |bencher| {
            bencher.iter(|| {
                let root = Archive::new(Holder::buffer(Buffer::new()))
                    .with_restart_stride(stride)
                    .mount();
                root.archive()
                    .write_member_from(
                        "blob.bin",
                        std::io::Cursor::new(black_box(&streamed)),
                        Codec::Deflate,
                    )
                    .expect("the member writes");
                root.archive().flush().expect("the directory publishes");
            });
        });
    }
    group.throughput(Throughput::Bytes(READ_LEN as u64));

    // Mounting parses the directory, which is the archive's one fixed cost.
    let image = {
        let path = std::env::temp_dir().join(format!("yggdryl-bench-zip-{MEMBERS}.zip"));
        let _ = std::fs::remove_file(&path);
        let lake = Archive::from_path(&path).expect("a local archive").mount();
        let payload = payload(64);
        for member in 0..MEMBERS {
            lake.archive()
                .write_member(
                    &format!("part={:02}/part-{member:06}.csv", member % 10),
                    &payload,
                )
                .expect("the member writes");
        }
        lake.archive().flush().expect("publishes");
        let image = std::fs::read(&path).expect("the archive reads");
        let _ = std::fs::remove_file(&path);
        image
    };
    group.throughput(Throughput::Elements(MEMBERS as u64));
    group.bench_function("mount/index", |bencher| {
        bencher.iter(|| {
            let root = Archive::new(Holder::buffer(Buffer::from_bytes(image.clone()))).mount();
            root.archive().open().expect("the index");
            black_box(root.archive().handle_reads())
        });
    });

    // Listing reads no member byte, so it is a walk of the index alone.
    group.throughput(Throughput::Elements(MEMBERS as u64));
    let lake = many_members(MEMBERS);
    group.bench_function("listing/first_entry", |bencher| {
        bencher.iter(|| black_box(&lake).ls(true, false).next().is_some());
    });
    group.bench_function("listing/drain", |bencher| {
        bencher.iter(|| black_box(&lake).ls(true, false).count());
    });
    group.bench_function("listing/glob", |bencher| {
        bencher.iter(|| {
            black_box(&lake)
                .glob("part=03/**/*.csv", false)
                .expect("a glob")
                .count()
        });
    });

    // Writing publishes one directory for the whole batch, not one per member.
    group.bench_function("write/members", |bencher| {
        bencher.iter(|| black_box(many_members(MEMBERS)).archive().size());
    });

    group.finish();
}
