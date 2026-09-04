//! Corpus sizing shared by benchmark targets.
//!
//! `cargo test --all-targets` executes every Criterion target in an
//! unoptimized build. Its purpose is a smoke check, so it uses small fixtures;
//! `cargo bench` keeps the full corpus that produces reportable measurements.

/// Select the full corpus in optimized builds and the smoke corpus otherwise.
pub(crate) const fn corpus(full: usize, smoke: usize) -> usize {
    if cfg!(debug_assertions) { smoke } else { full }
}
