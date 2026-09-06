#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "holder/buffered.rs"]
mod buffered;
#[path = "holder/fs/mod.rs"]
mod fs;
#[path = "holder/listing.rs"]
mod listing;

use criterion::{criterion_group, criterion_main};

criterion_group!(
    holder,
    fs::local_parity::local_parity_benchmarks,
    fs::bytes::byte_benchmarks,
    fs::record::record_benchmarks,
    fs::listing::listing_benchmarks,
    buffered::buffered_benchmarks,
    listing::listing_benchmarks,
);
criterion_main!(holder);
