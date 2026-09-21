//! `rust/src/object/mod.rs`: the fixtures every object-store suite builds on.
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
//!
//! Every handle below is built through the entry points `rust/src/object/mod.rs`
//! publishes - [`object::file_with`], [`object::folder_with`],
//! [`object::located_with`] - so the fixtures are themselves the pin on the
//! crate's own door: a location a suite spells is the location a handle
//! reports.

#![allow(dead_code)]

use yggdryl::object::{
    self, AzureOptions, Credentials, File, Folder, GoogleOptions, ObjectOptions, Path, Provider,
};

use crate::server::{self, FakeS3};

/// The bucket every fixture writes into.
pub const BUCKET: &str = "trades";

/// A running fake store with `bucket` created.
pub fn store() -> FakeS3 {
    let store = FakeS3::start();
    store.create_bucket(BUCKET);
    store
}

/// Options that reach `store` and consult nothing outside the test.
pub fn options(store: &FakeS3) -> ObjectOptions {
    ObjectOptions::default()
        .with_environment(false)
        .with_endpoint(store.endpoint())
        .with_region("us-east-1")
        .with_path_style(true)
        .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI"))
}

/// The object `key` on `store`.
pub fn file(store: &FakeS3, key: &str) -> File {
    file_with(key, options(store))
}

/// The object `key`, under options a test tightened.
///
/// The options already name the endpoint, so the store is not passed again.
pub fn file_with(key: &str, options: ObjectOptions) -> File {
    object::file_with(&location(key), options).expect("an object handle")
}

/// The prefix `key` on `store`.
pub fn folder(store: &FakeS3, key: &str) -> Folder {
    folder_with(key, options(store))
}

/// The prefix `key`, under options a test tightened.
pub fn folder_with(key: &str, options: ObjectOptions) -> Folder {
    object::folder_with(&location(key), options).expect("a prefix handle")
}

/// The location `key` on `store`.
pub fn path(store: &FakeS3, key: &str) -> Path {
    path_with(key, options(store))
}

/// The location `key`, under options a test tightened.
pub fn path_with(key: &str, options: ObjectOptions) -> Path {
    object::path_at_with(Provider::Aws, BUCKET, key, options).expect("a location handle")
}

/// The canonical location of `key` in the fixture bucket.
pub fn location(key: &str) -> String {
    format!("s3://{BUCKET}/{key}")
}

/// Options that reach `store` as `provider`, consulting nothing outside the
/// test.
///
/// Each store is authorized the way that store is: S3 by its keys, Google by a
/// token the caller holds, Azure by the account key the emulators publish.
pub fn options_for(store: &FakeS3, provider: Provider) -> ObjectOptions {
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
pub fn file_on(store: &FakeS3, provider: Provider, key: &str) -> File {
    file_on_with(provider, key, options_for(store, provider))
}

/// The object `key` on `provider`, under options a test tightened.
pub fn file_on_with(provider: Provider, key: &str, options: ObjectOptions) -> File {
    object::file_with(&location_on(provider, key), options).expect("an object handle")
}

/// The prefix `key` in the fixture container on `provider`.
pub fn folder_on(store: &FakeS3, provider: Provider, key: &str) -> Folder {
    folder_on_with(provider, key, options_for(store, provider))
}

/// The prefix `key` on `provider`, under options a test tightened.
pub fn folder_on_with(provider: Provider, key: &str, options: ObjectOptions) -> Folder {
    object::folder_with(&location_on(provider, key), options).expect("a prefix handle")
}

/// The location `key` on `provider`, under options a test tightened.
pub fn path_on_with(provider: Provider, key: &str, options: ObjectOptions) -> Path {
    object::path_at_with(provider, BUCKET, key, options).expect("a location handle")
}

/// The canonical location of `key` in the fixture container, on `provider`.
pub fn location_on(provider: Provider, key: &str) -> String {
    format!("{}://{BUCKET}/{key}", provider.scheme().as_str())
}

/// A payload of `size` bytes that does not compress to nothing.
pub fn payload(size: usize) -> Vec<u8> {
    (0..size).map(|index| (index % 251) as u8).collect()
}
