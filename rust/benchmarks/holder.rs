#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "holder/buffered.rs"]
mod buffered;
#[path = "holder/calls.rs"]
mod calls;
#[path = "holder/fs/mod.rs"]
mod fs;
#[path = "holder/listing.rs"]
mod listing;
// Amazon S3 against `object_store` on one in-process store. The backend is a
// non-default feature, so the group compiles in only when it is.
#[cfg(feature = "s3")]
#[path = "holder/s3/mod.rs"]
mod s3;

use criterion::{criterion_group, criterion_main};

/// The S3 groups when the backend is not compiled in: nothing to register.
///
/// Stubs rather than a second `criterion_group!` list, so the target's
/// benchmarks are named in one place whatever the feature state.
#[cfg(not(feature = "s3"))]
mod s3 {
    pub(crate) mod bytes {
        pub(crate) fn byte_benchmarks(_: &mut criterion::Criterion) {}
    }
    pub(crate) mod listing {
        pub(crate) fn listing_benchmarks(_: &mut criterion::Criterion) {}
    }
    pub(crate) mod records {
        pub(crate) fn record_benchmarks(_: &mut criterion::Criterion) {}
    }
}

criterion_group!(
    holder,
    fs::local_parity::local_parity_benchmarks,
    fs::bytes::byte_benchmarks,
    fs::record::record_benchmarks,
    fs::listing::listing_benchmarks,
    buffered::buffered_benchmarks,
    calls::call_benchmarks,
    listing::listing_benchmarks,
    s3::bytes::byte_benchmarks,
    s3::listing::listing_benchmarks,
    s3::records::record_benchmarks,
);
criterion_main!(holder);
