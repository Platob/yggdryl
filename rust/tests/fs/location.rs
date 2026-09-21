//! `rust/src/fs/location.rs`: the binding a path, a URI and a masked URI make.
//!
//! A location keeps the filesystem it belongs to, the opaque path, the
//! caller's own URI spelling and the diagnostic form of it as four separate
//! facts, and every one of them is public, so this reaches the crate the way
//! a caller does.

use std::sync::Arc;

use yggdryl::fs::{BoundLocation, FileSystem, MemoryFileSystem, mask_uri};

#[test]
fn a_dot_child_is_the_location_itself() {
    let fs: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    let location = BoundLocation::new(
        fs,
        "bucket/table",
        Some("memory://bound/bucket/table".to_owned()),
    )
    .unwrap();
    for name in ["", "."] {
        let child = location.child(name).unwrap();
        assert_eq!(child.path(), "bucket/table", "{name:?}");
        assert_eq!(child.uri(), location.uri(), "{name:?}");
    }
    // A handle taken with `.` resolves what is below it where the
    // location itself would, not under a `./` segment of its own.
    let below = location
        .child(".")
        .unwrap()
        .child("data/part.parquet")
        .unwrap();
    assert_eq!(below.path(), "bucket/table/data/part.parquet");
    assert_eq!(
        below.uri(),
        Some("memory://bound/bucket/table/data/part.parquet")
    );
}

#[test]
fn injected_paths_stay_opaque_and_diagnostics_mask_credentials() {
    let fs: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    let location = BoundLocation::new(
        fs,
        "bucket/café/v=a%2Fb//x+y://z",
        Some("s3://key:secret@bucket/caf%C3%A9/v=a%2Fb//x+y://z?session_token=hidden".to_owned()),
    )
    .unwrap();
    assert_eq!(location.path(), "bucket/café/v=a%2Fb//x+y://z");
    assert_eq!(
        location.uri(),
        Some("s3://key:secret@bucket/caf%C3%A9/v=a%2Fb//x+y://z?session_token=hidden")
    );
    let safe = location.masked_uri().unwrap();
    assert!(!safe.contains("secret"));
    assert!(!safe.contains("hidden"));
    assert!(!format!("{location:?}").contains("secret"));

    let encoded = mask_uri("s3://bucket/key?secret%5Fkey=also-hidden");
    assert!(!encoded.contains("also-hidden"));

    let many = mask_uri(
        "copy s3://first:first-secret@one/a?X-Amz-Credential=first-credential&X-Amz-Signature=first-signature to s3://second:second-secret@two/b#X-Amz-Security-Token=second-token aws_secret_access_key=third-secret",
    );
    for secret in [
        "first-secret",
        "first-credential",
        "first-signature",
        "second-secret",
        "second-token",
        "third-secret",
    ] {
        assert!(!many.contains(secret), "leaked {secret}: {many}");
    }
    let mappings =
        mask_uri("secret_key: 'colon-secret' password = spaced-secret token:\"quoted-token\"");
    for secret in ["colon-secret", "spaced-secret", "quoted-token"] {
        assert!(!mappings.contains(secret), "leaked {secret}: {mappings}");
    }

    let repeated = BoundLocation::new(
        location.filesystem().clone(),
        "bucket/a//b",
        Some("s3://bucket/a//b".to_owned()),
    )
    .unwrap();
    let parent = repeated.parent().unwrap().unwrap();
    assert_eq!(parent.path(), "bucket/a/");
    assert_eq!(parent.uri(), Some("s3://bucket/a/"));
    let rejoined = parent.child("b").unwrap();
    assert_eq!(rejoined.path(), repeated.path());
    assert_eq!(rejoined.uri(), repeated.uri());
    let listed = parent.listed("bucket/a//b").unwrap();
    assert_eq!(listed.path(), repeated.path());
    assert_eq!(listed.uri(), repeated.uri());
}
