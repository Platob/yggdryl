pub(crate) mod decode;
pub(crate) mod encode;
pub(crate) mod streaming;

/// The payload sizes a transcode's cost changes shape at.
///
/// The first three are where the per-call cost still shows, the middle two are
/// where the word-at-a-time ASCII scan starts to pay, and the last is a page
/// of the size a record reader actually fills.
pub(crate) const SIZES: [usize; 6] = [
    64,
    1024,
    16 * 1024,
    64 * 1024,
    crate::bench_profile::corpus(1024 * 1024, 128 * 1024),
    crate::bench_profile::corpus(16 * 1024 * 1024, 256 * 1024),
];

/// A deterministic payload of `length` bytes, `high` of every hundred of which
/// are above US-ASCII.
///
/// The mix is the whole measurement: a legacy export is mostly ASCII with a
/// scattering of accented names, and the borrow that costs nothing and the
/// transcode that costs a string are the same code path at two mixes.
pub(crate) fn payload(length: usize, high: usize) -> Vec<u8> {
    let row = b"symbol,desk,price\nAAPL,London,187.23\n";
    let mut bytes: Vec<u8> = row.iter().copied().cycle().take(length).collect();
    if high == 0 {
        return bytes;
    }
    let stride = 100 / high.min(100);
    for index in (0..bytes.len()).step_by(stride.max(1)) {
        // `0xE9` is `é` in every single-byte charset measured here.
        bytes[index] = 0xE9;
    }
    bytes
}

/// Format a byte count the way the size labels read.
pub(crate) fn label(length: usize) -> String {
    match length {
        length if length >= 1024 * 1024 => format!("{}MiB", length / (1024 * 1024)),
        length if length >= 1024 => format!("{}KiB", length / 1024),
        length => format!("{length}B"),
    }
}
