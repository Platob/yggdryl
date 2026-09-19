//! The object-store backend against a real socket, counting what goes over it.
//!
//! Every test here runs against [`server::FakeS3`], an in-process store that
//! speaks all three dialects - Amazon S3, Google's JSON API, Azure Blob
//! Storage - over one set of objects, and records each request. That is what
//! lets the suite assert the thing this backend is designed for - the *number
//! of round trips* an operation costs - rather than only that it produced the
//! right bytes. A change that makes a read cost two requests instead of one is
//! a failing test, not a silent regression.
//!
//! One store answering three dialects is also what lets a test write with one
//! and read with another, which is the cheapest possible check that the three
//! agree on what a key and a prefix are.

#[path = "server.rs"]
pub(crate) mod server;

mod accounting;
mod dialects;
mod encryption;
mod properties;
mod protocol;
mod roles;

use std::sync::Arc;

use server::FakeS3;

use super::{
    AzureOptions, Credentials, File, Folder, GoogleOptions, ObjectOptions, Path, Provider,
    client::Client,
};
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
fn options(store: &FakeS3) -> ObjectOptions {
    ObjectOptions::default()
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
fn file_with(key: &str, options: ObjectOptions) -> File {
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

/// Options that reach `store` as `provider`, consulting nothing outside the
/// test.
///
/// Each store is authorized the way that store is: S3 by its keys, Google by a
/// token the caller holds, Azure by the account key the emulators publish.
fn options_for(store: &FakeS3, provider: Provider) -> ObjectOptions {
    let options = ObjectOptions::default()
        .with_environment(false)
        .with_endpoint(store.endpoint())
        .with_region("us-east-1")
        .with_path_style(true);
    match provider {
        Provider::Aws => {
            options.with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI"))
        }
        Provider::Google => options.with_google(
            GoogleOptions::default()
                .with_access_token("ya29.test")
                .with_project("trading"),
        ),
        Provider::Azure => options.with_azure(
            AzureOptions::default()
                .with_account(server::AZURE_ACCOUNT)
                .with_account_key(server::AZURE_KEY),
        ),
    }
}

/// The object `key` in the fixture container on `provider`.
fn file_on(store: &FakeS3, provider: Provider, key: &str) -> File {
    let url = location_on(provider, key);
    let client = Arc::new(Client::new(&url, options_for(store, provider)).expect("a client"));
    File::new(client, url).expect("an object handle")
}

/// The prefix `key` in the fixture container on `provider`.
fn folder_on(store: &FakeS3, provider: Provider, key: &str) -> Folder {
    let url = location_on(provider, key);
    let client = Arc::new(Client::new(&url, options_for(store, provider)).expect("a client"));
    Folder::new(client, url).expect("a prefix handle")
}

/// The canonical location of `key` in the fixture container, on `provider`.
fn location_on(provider: Provider, key: &str) -> Url {
    Url::from_str(&format!("{}://{BUCKET}/{key}", provider.scheme().as_str()))
        .expect("a valid location")
}

/// A payload of `size` bytes that does not compress to nothing.
fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|index| (index % 251) as u8).collect()
}
