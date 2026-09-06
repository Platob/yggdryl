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
use yggdryl::holder::zip::{Archive, Folder};
use yggdryl::holder::{Buffer, Holder};
use yggdryl::{Codec, IOBase};

/// The member size the read legs are measured at.
const MEMBER_LEN: usize = 1 << 20;

/// The byte range one positional read asks for.
const READ_LEN: usize = 512;

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
fn one_member(codec: Codec) -> Folder {
    let root = Archive::new(Holder::buffer(Buffer::new())).mount();
    root.archive()
        .write_member_with("blob.bin", &payload(MEMBER_LEN), codec)
        .expect("the member writes");
    root.archive().flush().expect("the directory publishes");
    root
}

/// An in-memory archive of `count` members across ten directories.
fn many_members(count: usize) -> Folder {
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
    let stored_member = stored.as_file("blob.bin").expect("a member");
    group.bench_function("read/stored", |bencher| {
        bencher.iter(|| {
            black_box(&stored_member)
                .read_range_bytes((MEMBER_LEN / 2) as u64, READ_LEN)
                .expect("a range")
        });
    });

    // A closed compressed read decodes to the offset and retains nothing.
    let deflated = one_member(Codec::Deflate);
    let deflated_member = deflated.as_file("blob.bin").expect("a member");
    group.bench_function("read/deflate_closed", |bencher| {
        bencher.iter(|| {
            black_box(&deflated_member)
                .read_range_bytes((MEMBER_LEN / 2) as u64, READ_LEN)
                .expect("a range")
        });
    });

    // An opened one decodes once and answers from the member it holds.
    let mut opened_member = deflated.as_file("blob.bin").expect("a member");
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
        let member = root.as_file("blob.bin").expect("a member");
        group.bench_function(BenchmarkId::new("read_all", name), |bencher| {
            bencher.iter(|| black_box(&member).read_all_bytes().expect("the member"));
        });
    }

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
