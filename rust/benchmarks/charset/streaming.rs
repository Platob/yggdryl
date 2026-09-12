//! The chunked door beside the whole-buffer one, and the handle over both.

use std::hint::black_box;
use std::io::Read as _;

use criterion::{Criterion, Throughput};
use yggdryl::charset::Transcoded;
use yggdryl::holder::Buffer;
use yggdryl::{Charset, IOBase};

use super::{label, payload};

/// The sizes a stream's per-chunk bookkeeping is worth measuring at.
const STREAMED: [usize; 3] = [
    64 * 1024,
    crate::bench_profile::corpus(1024 * 1024, 128 * 1024),
    crate::bench_profile::corpus(16 * 1024 * 1024, 256 * 1024),
];

/// A chunked decode, a reader, and a transcoding handle over one payload.
///
/// All three answer the same bytes, so the rows are what each door adds over
/// the whole-buffer decode: a carry check per chunk, a buffer copy per read,
/// and a handle's stream plus its media type.
pub(crate) fn streaming_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("charset_streaming");
    for length in STREAMED {
        let bytes = payload(length, 5);
        group.throughput(Throughput::Bytes(length as u64));
        let size = label(length);

        group.bench_function(format!("decode/{size}"), |bencher| {
            bencher.iter(|| black_box(Charset::Cp1252.decode(black_box(&bytes))));
        });
        group.bench_function(format!("decoder_chunks/{size}"), |bencher| {
            bencher.iter(|| {
                let mut decoder = Charset::Cp1252.decoder();
                let mut text = String::new();
                for chunk in black_box(&bytes).chunks(8 * 1024) {
                    decoder.push(chunk, &mut text).expect("windows-1252");
                }
                decoder.finish().expect("a whole payload");
                black_box(text)
            });
        });
        group.bench_function(format!("reader/{size}"), |bencher| {
            bencher.iter(|| {
                let mut text = String::new();
                Charset::Cp1252
                    .reader(std::io::Cursor::new(black_box(&bytes)))
                    .read_to_string(&mut text)
                    .expect("windows-1252");
                black_box(text)
            });
        });

        let handle = Transcoded::new(Buffer::from_bytes(bytes.clone()), Charset::Cp1252);
        group.bench_function(format!("handle/{size}"), |bencher| {
            bencher.iter(|| black_box(handle.read_all_bytes().expect("windows-1252")));
        });

        // Random access, which is the row the resume index exists for: eight
        // scattered windows, against the same eight over a plain buffer. The
        // index is built by the first call and shared by the rest, so the row
        // is the seek rather than the scan.
        let decoded = handle.size();
        let offsets: Vec<u64> = (0..8).map(|step| decoded * step / 8).collect();
        let plain = Buffer::from_bytes(handle.read_all_bytes().expect("windows-1252"));
        group.throughput(Throughput::Bytes(offsets.len() as u64 * 4096));
        group.bench_function(format!("random_decoded/{size}"), |bencher| {
            bencher.iter(|| {
                for offset in black_box(&offsets) {
                    black_box(handle.read_range_bytes(*offset, 4096).expect("a window"));
                }
            });
        });
        group.bench_function(format!("random_plain/{size}"), |bencher| {
            bencher.iter(|| {
                for offset in black_box(&offsets) {
                    black_box(plain.read_range_bytes(*offset, 4096).expect("a window"));
                }
            });
        });
    }
    group.finish();
}
