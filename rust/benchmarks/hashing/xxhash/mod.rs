pub(crate) mod arrow;
pub(crate) mod handles;
pub(crate) mod oneshot;
pub(crate) mod values;

/// The payload sizes a digest's cost changes shape at.
///
/// The first six are XXH3's own size branches - a call's fixed cost dominates
/// below 240 bytes - and the last four are where the streaming kernel does.
pub(crate) const SIZES: [usize; 10] = [
    1,
    4,
    16,
    64,
    128,
    240,
    1024,
    64 * 1024,
    crate::bench_profile::corpus(1024 * 1024, 256 * 1024),
    crate::bench_profile::corpus(64 * 1024 * 1024, 1024 * 1024),
];

/// Format a byte count the way the size labels read.
pub(crate) fn label(length: usize) -> String {
    match length {
        length if length >= 1024 * 1024 => format!("{}MiB", length / (1024 * 1024)),
        length if length >= 1024 => format!("{}KiB", length / 1024),
        length => format!("{length}B"),
    }
}
