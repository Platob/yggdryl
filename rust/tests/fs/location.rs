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

mod fs {

    use std::sync::Arc;
    use yggdryl::fs::*;
    use yggdryl::{IOBase, Result};

    #[test]
    fn bound_locations_keep_raw_path_uri_and_filesystem_identity_separate() {
        let memory = MemoryFileSystem::new();
        let filesystem: Arc<dyn FileSystem> = Arc::new(memory.clone());
        let equal: Arc<dyn FileSystem> = Arc::new(memory);
        let different: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
        let uri = "s3://access:secret@bucket/v=a%2Fb.bin?session_token=hidden";
        let location = BoundLocation::new(
            Arc::clone(&filesystem),
            "bucket/v=a%2Fb.bin",
            Some(uri.to_owned()),
        )
        .unwrap();
        let same = BoundLocation::new(equal, "bucket/v=a%2Fb.bin", None::<String>).unwrap();
        let other = BoundLocation::new(different, "bucket/v=a%2Fb.bin", None::<String>).unwrap();

        assert_eq!(location.path(), "bucket/v=a%2Fb.bin");
        assert_eq!(location.uri(), Some(uri));
        assert!(location.same_location(&same));
        assert_eq!(location.identity(), same.identity());
        assert!(!location.same_location(&other));
        assert_ne!(location.identity(), other.identity());
        for diagnostic in [
            location.masked_uri().unwrap().to_owned(),
            format!("{location}"),
            format!("{location:?}"),
            format!("{:?}", location.identity()),
        ] {
            assert!(!diagnostic.contains("secret"));
            assert!(!diagnostic.contains("hidden"));
        }

        let parent = location.parent().unwrap().unwrap();
        assert_eq!(parent.path(), "bucket");
        assert!(parent.filesystem().equals(filesystem.as_ref()));
        let child = parent.child("v=a%2Fb//x%25+y://z").unwrap();
        assert_eq!(child.path(), "bucket/v=a%2Fb//x%25+y://z");
        assert!(child.filesystem().equals(filesystem.as_ref()));

        let absolute = BoundLocation::new(
            Arc::clone(&filesystem),
            "/tmp/value",
            Some("file:///tmp/value".to_owned()),
        )
        .unwrap();
        assert_eq!(absolute.parent().unwrap().unwrap().path(), "/tmp");
        assert_eq!(
            absolute.parent().unwrap().unwrap().uri(),
            Some("file:///tmp")
        );
        let absolute_root = absolute
            .parent()
            .unwrap()
            .unwrap()
            .parent()
            .unwrap()
            .unwrap();
        assert_eq!(absolute_root.path(), "/");
        assert_eq!(absolute_root.uri(), Some("file:///"));

        filesystem.create_dir("bucket", false).unwrap();
        let mut file = FsFile::new(location.clone());
        file.write_all_bytes(b"literal").unwrap();
        assert_eq!(file.read_all_bytes().unwrap(), b"literal");
        let folder = FsFolder::from_path(
            Arc::clone(&filesystem),
            "bucket",
            Some("s3://access:secret@bucket?session_token=hidden".to_owned()),
        )
        .unwrap();
        let listed = folder.ls(false, true).collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(listed.len(), 1);
        let listed_bound = listed[0].bound_location().unwrap();
        assert_eq!(listed_bound.path(), "bucket/v=a%2Fb.bin");
        assert!(listed_bound.filesystem().equals(filesystem.as_ref()));
        assert_eq!(
            listed_bound.uri(),
            Some("s3://access:secret@bucket/v=a%2Fb.bin?session_token=hidden")
        );
        let globbed = folder
            .glob("v=a%2F*.bin", true)
            .unwrap()
            .collect::<Result<Vec<_>>>()
            .unwrap();
        assert_eq!(globbed.len(), 1);
        assert_eq!(
            globbed[0].bound_location().unwrap().path(),
            "bucket/v=a%2Fb.bin"
        );
    }

    #[test]
    fn native_filesystem_errors_never_expose_credentials_in_opaque_paths() {
        let filesystem = MemoryFileSystem::new();
        let path = "s3://access:do-not-leak@bucket/missing?session_token=also-hidden";
        let error = match filesystem.open_input_file(path) {
            Ok(_) => panic!("missing path unexpectedly opened"),
            Err(error) => error,
        };
        for rendered in [error.to_string(), format!("{error:?}")] {
            assert!(!rendered.contains("do-not-leak"), "{rendered}");
            assert!(!rendered.contains("also-hidden"), "{rendered}");
        }
    }
}
