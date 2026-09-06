//! The S3 backend against a real socket, counting what goes over it.
//!
//! Every test here runs against [`server::FakeS3`], an in-process store that
//! speaks the S3 REST API and records each request. That is what lets the
//! suite assert the thing this backend is designed for - the *number of round
//! trips* an operation costs - rather than only that it produced the right
//! bytes. A change that makes a read cost two requests instead of one is a
//! failing test, not a silent regression.

#[path = "server.rs"]
pub(crate) mod server;

mod accounting;
mod encryption;
mod properties;
mod protocol;
mod roles;

use std::sync::Arc;

use server::FakeS3;

use super::{Credentials, File, Folder, Path, S3Options, client::Client};
use crate::Url;

/// The bucket every fixture writes into.
const BUCKET: &str = "trades";

/// A running fake store with `bucket` created.
fn store() -> FakeS3 {
    let store = FakeS3::start();
    store.create_bucket(BUCKET);
    store
}

/// Options that reach `store` and consult nothing outside the test.
fn options(store: &FakeS3) -> S3Options {
    S3Options::default()
        .with_environment(false)
        .with_endpoint(store.endpoint())
        .with_region("us-east-1")
        .with_path_style(true)
        .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI"))
}

/// One client against `store`, so a test can read its counters.
fn client(store: &FakeS3) -> Arc<Client> {
    let url = Url::from_str(&format!("s3://{BUCKET}/")).expect("a valid location");
    Arc::new(Client::new(&url, options(store)).expect("a client"))
}

/// The object `key` on `store`, through a shared client.
fn file(store: &FakeS3, key: &str) -> File {
    File::new(client(store), location(key)).expect("an object handle")
}

/// The object `key`, under options a test tightened.
///
/// The options already name the endpoint, so the store is not passed again.
fn file_with(key: &str, options: S3Options) -> File {
    let url = location(key);
    let client = Arc::new(Client::new(&url, options).expect("a client"));
    File::new(client, url).expect("an object handle")
}

/// The prefix `key` on `store`, through a shared client.
fn folder(store: &FakeS3, key: &str) -> Folder {
    Folder::new(client(store), location(key)).expect("a prefix handle")
}

/// The location `key` on `store`, through a shared client.
fn path(store: &FakeS3, key: &str) -> Path {
    Path::new(client(store), location(key)).expect("a location handle")
}

/// The canonical location of `key` in the fixture bucket.
fn location(key: &str) -> Url {
    Url::from_str(&format!("s3://{BUCKET}/{key}")).expect("a valid location")
}

/// A payload of `size` bytes that does not compress to nothing.
fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|index| (index % 251) as u8).collect()
}
