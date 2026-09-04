#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "holder/arrowfs/mod.rs"]
mod arrowfs;
#[path = "holder/buffered.rs"]
mod buffered;
#[path = "holder/listing.rs"]
mod listing;

use criterion::{criterion_group, criterion_main};

criterion_group!(
    holder,
    arrowfs::bytes::byte_benchmarks,
    arrowfs::record::record_benchmarks,
    arrowfs::listing::listing_benchmarks,
    buffered::buffered_benchmarks,
    listing::listing_benchmarks,
);
criterion_main!(holder);
