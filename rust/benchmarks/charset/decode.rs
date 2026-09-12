//! What a decode costs per byte, and what the borrow saves over a transcode.

use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::Charset;

use super::{SIZES, label, payload};

/// The charsets whose decode shapes differ: a validation, a table, a scan.
const MEASURED: [Charset; 4] = [
    Charset::Utf8,
    Charset::Ascii,
    Charset::Latin1,
    Charset::Cp1252,
];

/// Every measured charset at every size, over a payload that must transcode.
///
/// One byte in twenty is above US-ASCII, which is the shape of a real export:
/// the ASCII runs are copied whole and the accented bytes are one table lookup
/// each, so the row is dominated by the scan rather than by the lookups.
pub(crate) fn decode_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("charset_decode");
    for length in SIZES {
        let bytes = payload(length, 5);
        group.throughput(Throughput::Bytes(length as u64));
        let size = label(length);
        for charset in MEASURED {
            if charset.decode(&bytes).is_err() {
                // US-ASCII and UTF-8 refuse this payload by contract; their
                // rows are the borrow group below.
                continue;
            }
            group.bench_function(format!("{charset}/{size}"), |bencher| {
                bencher.iter(|| black_box(charset.decode(black_box(&bytes))));
            });
        }
        // UTF-16 is measured on its own payload, because the same bytes are
        // not a document in it.
        let utf16 = Charset::Utf16Le
            .encode(&Charset::Latin1.decode(&bytes).expect("latin-1 holds it"))
            .expect("every scalar has a UTF-16 form")
            .into_owned();
        group.throughput(Throughput::Bytes(utf16.len() as u64));
        group.bench_function(format!("utf-16le/{size}"), |bencher| {
            bencher.iter(|| black_box(Charset::Utf16Le.decode(black_box(&utf16))));
        });
    }
    group.finish();
}

/// The same call over a payload that is already UTF-8.
///
/// This is the row the `# Borrowing` claim rests on: an all-ASCII payload is
/// answered by a scan and a borrow, so it must be an order of magnitude
/// cheaper than the transcode above and must not vary by charset.
pub(crate) fn borrow_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("charset_borrow");
    for length in SIZES {
        let bytes = payload(length, 0);
        group.throughput(Throughput::Bytes(length as u64));
        let size = label(length);
        for charset in MEASURED {
            group.bench_function(format!("{charset}/{size}"), |bencher| {
                bencher.iter(|| black_box(charset.decode(black_box(&bytes))));
            });
        }
        group.bench_function(format!("std_from_utf8/{size}"), |bencher| {
            bencher.iter(|| black_box(std::str::from_utf8(black_box(&bytes))));
        });
    }
    group.finish();
}
