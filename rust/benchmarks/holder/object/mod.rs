//! The S3 backend measured against `object_store`, on one in-process store.
//!
//! Both clients speak the S3 REST API to the same [`FakeS3`] over a real
//! socket, on the same payloads and the same keys, so the difference is the
//! two implementations rather than two networks. That is also what makes the
//! comparison a protocol check: a leg that measured faster by skipping a
//! request would fail the accounting assertions in
//! `holder::object::tests::accounting`, and one that spoke a dialect the store
//! does not would not complete at all.
//!
//! `object_store` is async, so its legs pay for a current-thread runtime to
//! block on. That cost is real for a caller who has no runtime already, which
//! is exactly the caller this crate is for - the tables name it rather than
//! subtracting it.

pub(crate) mod bytes;
pub(crate) mod listing;
pub(crate) mod records;

#[path = "../../../src/holder/object/tests/server.rs"]
pub(crate) mod server;

use object_store::aws::{AmazonS3, AmazonS3Builder};
use server::FakeS3;
use yggdryl::holder::object::{Credentials, ObjectOptions};

/// The bucket every fixture writes into.
pub(crate) const BUCKET: &str = "bench";
/// The access key both clients sign with.
const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
/// The secret both clients sign with.
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

/// Bytes per byte-level fixture, spanning several transfer chunks.
pub(crate) const PAYLOAD: usize = crate::bench_profile::corpus(4 * 1024 * 1024, 64 * 1024);
/// Leaves per listing fixture.
pub(crate) const LEAVES: usize = crate::bench_profile::corpus(2_000, 50);
/// Rows per record fixture.
pub(crate) const ROWS: i64 = crate::bench_profile::corpus(65_536, 1_024) as i64;

/// A running store with the fixture bucket, recording nothing.
///
/// Request recording is off: a benchmark measures the client, and holding a
/// log of every request would measure the harness instead.
pub(crate) fn store() -> FakeS3 {
    let store = FakeS3::start();
    store.create_bucket(BUCKET);
    store.set_recording(false);
    store
}

/// Options addressing `store`, consulting nothing outside the benchmark.
pub(crate) fn options(store: &FakeS3) -> ObjectOptions {
    ObjectOptions::default()
        .with_environment(false)
        .with_endpoint(store.endpoint())
        .with_region("us-east-1")
        .with_path_style(true)
        .with_credentials(Credentials::new(ACCESS_KEY, SECRET_KEY))
}

/// The canonical location of `key` in the fixture bucket.
pub(crate) fn location(key: &str) -> String {
    format!("s3://{BUCKET}/{key}")
}

/// The same store, reached through `object_store`'s own S3 client.
pub(crate) fn baseline(store: &FakeS3) -> AmazonS3 {
    AmazonS3Builder::new()
        .with_endpoint(store.endpoint())
        .with_bucket_name(BUCKET)
        .with_region("us-east-1")
        .with_access_key_id(ACCESS_KEY)
        .with_secret_access_key(SECRET_KEY)
        // The fixture speaks plain HTTP on a loopback port, and path style is
        // what a bare host without bucket subdomains can answer.
        .with_allow_http(true)
        .with_virtual_hosted_style_request(false)
        .build()
        .expect("a baseline S3 client")
}

/// The runtime `object_store`'s async surface is driven from.
///
/// Current-thread and no I/O driver beyond what `reqwest` installs: the
/// baseline should pay for a runtime, not for a thread pool nobody uses.
pub(crate) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime")
}

/// A repeating byte payload, incompressible enough to stay honest.
pub(crate) fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|index| (index % 251) as u8).collect()
}

/// Populate `store` with a partitioned tree of `leaves` parts.
pub(crate) fn tree(store: &FakeS3, root: &str, leaves: usize) {
    for leaf in 0..leaves {
        let year = 2024 + (leaf % 2);
        let month = 1 + (leaf / 2) % 12;
        store.put(
            BUCKET,
            &format!("{root}/year={year}/month={month:02}/part-{leaf:06}.parquet"),
            b"PAR1",
        );
    }
}

/// One `object_store` path for `key`.
pub(crate) fn baseline_path(key: &str) -> object_store::path::Path {
    object_store::path::Path::from(key)
}
