//! What an encode costs per byte, including the reverse table lookup.

use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::Charset;

use super::{SIZES, label, payload};

/// Encoding text back into one byte per scalar.
///
/// The binary search over the reverse table is the cost this measures; the
/// ASCII runs never reach it, which is why the row moves with the mix rather
/// than with the size alone.
pub(crate) fn encode_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("charset_encode");
    for length in SIZES {
        let size = label(length);
        for high in [0_usize, 5, 50] {
            let bytes = payload(length, high);
            let text = Charset::Latin1
                .decode(&bytes)
                .expect("latin-1 assigns every byte")
                .into_owned();
            group.throughput(Throughput::Bytes(text.len() as u64));
            for charset in [Charset::Latin1, Charset::Cp1252, Charset::Utf16Le] {
                group.bench_function(format!("{charset}/{high}pct/{size}"), |bencher| {
                    bencher.iter(|| black_box(charset.encode(black_box(&text))));
                });
            }
        }
    }
    group.finish();
}
