//! `rust/src/fs/uri.rs`: what a filesystem URI resolves into.
//!
//! The resolution, the options it carries and the addressing it picks are all
//! `yggdryl::fs` API, so this reaches the crate the way a caller does.

use yggdryl::fs::{ResolvedFileSystem, ResolvedFileSystemUri, S3AddressingStyle};

fn s3(uri: &str) -> ResolvedFileSystemUri {
    ResolvedFileSystemUri::from_uri(uri, None).unwrap()
}

#[test]
fn required_s3_shapes_separate_configuration_bucket_and_literal_key() {
    for uri in [
        "s3://bucket/v=a%2Fb",
        "s3a://bucket/v=a%2Fb",
        "s3n://bucket/v=a%2Fb",
    ] {
        let resolved = s3(uri);
        assert_eq!(resolved.bucket(), Some("bucket"));
        assert_eq!(resolved.key(), Some("v=a%2Fb"));
        assert_eq!(resolved.path(), "bucket/v=a%2Fb");
    }
    assert_eq!(s3("s3://bucket/v=a%2fb").path(), "bucket/v=a%2fb");

    let credentialed = s3("s3://key:sec:ret@bucket/key");
    let ResolvedFileSystem::S3(options) = credentialed.filesystem() else {
        panic!("expected S3")
    };
    assert_eq!(options.access_key(), Some("key"));
    assert_eq!(options.secret_key(), Some("sec:ret"));
    assert_eq!(credentialed.path(), "bucket/key");

    let endpoint = s3("s3://key:secret@minio:9000/bucket/key");
    let ResolvedFileSystem::S3(options) = endpoint.filesystem() else {
        panic!("expected S3")
    };
    assert_eq!(options.endpoint_override(), Some("minio:9000"));
    assert_eq!(endpoint.bucket(), Some("bucket"));
    assert_eq!(endpoint.key(), Some("key"));

    let configured =
        s3("s3://bucket/key?endpoint_override=minio%3A9000&scheme=http&region=eu-west-1");
    let ResolvedFileSystem::S3(options) = configured.filesystem() else {
        panic!("expected S3")
    };
    assert_eq!(options.endpoint_override(), Some("minio:9000"));
    assert_eq!(options.transport(), "http");
    assert_eq!(options.region(), Some("eu-west-1"));

    let virtual_host = s3("s3://bucket.s3.eu-west-1.amazonaws.com/key");
    let ResolvedFileSystem::S3(options) = virtual_host.filesystem() else {
        panic!("expected S3")
    };
    assert_eq!(virtual_host.bucket(), Some("bucket"));
    assert_eq!(virtual_host.key(), Some("key"));
    assert_eq!(
        options.endpoint_override(),
        Some("s3.eu-west-1.amazonaws.com")
    );
    assert_eq!(options.region(), Some("eu-west-1"));
    assert_eq!(options.addressing_style(), S3AddressingStyle::Virtual);
}

#[test]
fn raw_object_path_characters_and_secrets_never_change_or_leak() {
    let resolved = s3("s3://access:never-show-this@bucket/v=a%2Fb//x%25+y?session_token=hidden");
    assert_eq!(resolved.path(), "bucket/v=a%2Fb//x%25+y");
    assert_eq!(resolved.key(), Some("v=a%2Fb//x%25+y"));
    assert_eq!(
        resolved.uri(),
        "s3://access:never-show-this@bucket/v=a%2Fb//x%25+y?session_token=hidden"
    );
    let debug = format!("{resolved:?}");
    assert!(!debug.contains("never-show-this"));
    assert!(!debug.contains("hidden"));
    assert!(!resolved.masked_uri().contains("never-show-this"));
    assert!(!resolved.masked_uri().contains("hidden"));
}
