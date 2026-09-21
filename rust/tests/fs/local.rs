//! `rust/src/fs/local.rs`: the local-disk backend's hardening against a root.
//!
//! The volume-root guard and the non-root resolution are private helpers the
//! destructive operations go through, so a caller has no name for either;
//! they are reached through `yggdryl::internals`. Everything the backend does
//! that a caller can observe is in `rust/tests/holder/fs.rs`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use yggdryl::fs::{FileSelector, FileSystem, LocalFileSystem};
use yggdryl::internals::fs_local::{is_local_root, resolve_non_root};

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "yggdryl-local-hardening-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn as_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn non_recursive_create_is_idempotent_only_for_a_directory() {
    let temporary = TestDirectory::new();
    let filesystem = LocalFileSystem::new();
    let directory = temporary.0.join("directory");
    filesystem.create_dir(&as_path(&directory), false).unwrap();
    filesystem.create_dir(&as_path(&directory), false).unwrap();

    let file = temporary.0.join("file");
    std::fs::write(&file, b"value").unwrap();
    assert!(
        filesystem
            .create_dir(&as_path(&file), false)
            .unwrap_err()
            .is_conflict()
    );
    assert_eq!(std::fs::read(file).unwrap(), b"value");
}

#[test]
fn destructive_directory_resolution_rejects_a_volume_root_alias() {
    let temporary = TestDirectory::new();
    let canonical = std::fs::canonicalize(&temporary.0).unwrap();
    let mut top_level = canonical;
    loop {
        let parent = top_level.parent().unwrap();
        if is_local_root(parent) {
            break;
        }
        top_level = parent.to_owned();
    }
    let alias = top_level.join("..");
    assert!(is_local_root(&std::fs::canonicalize(&alias).unwrap()));
    assert!(
        resolve_non_root(&as_path(&alias), "test root guard")
            .unwrap_err()
            .is_unsupported()
    );

    let filesystem = LocalFileSystem::new();
    assert!(filesystem.delete_dir(".").unwrap_err().is_unsupported());
    assert!(
        filesystem
            .delete_dir_contents("", false)
            .unwrap_err()
            .is_unsupported()
    );
}

#[test]
fn delete_dir_contents_uses_the_resolved_directory_and_keeps_it() {
    let temporary = TestDirectory::new();
    let selected = temporary.0.join("selected");
    let dot_parent = selected.join("dot-parent");
    std::fs::create_dir_all(&dot_parent).unwrap();
    std::fs::write(selected.join("value"), b"value").unwrap();
    let alias = dot_parent.join("..");

    LocalFileSystem::new()
        .delete_dir_contents(&as_path(&alias), false)
        .unwrap();

    assert!(selected.is_dir());
    assert_eq!(std::fs::read_dir(selected).unwrap().count(), 0);
}

#[test]
fn recursive_listing_does_not_descend_through_directory_symlinks() {
    let temporary = TestDirectory::new();
    let listed = temporary.0.join("listed");
    let outside = temporary.0.join("outside");
    std::fs::create_dir(&listed).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("secret"), b"secret").unwrap();
    let link = listed.join("link");
    if !create_directory_symlink(&outside, &link) {
        return;
    }

    let paths = LocalFileSystem::new()
        .list(&FileSelector::new(as_path(&listed), true, false))
        .map(|entry| entry.unwrap().path)
        .collect::<Vec<_>>();
    let link = as_path(&link).replace('\\', "/");
    assert!(paths.iter().any(|path| path == &link));
    assert!(
        !paths
            .iter()
            .any(|path| path.starts_with(&format!("{link}/")))
    );
}

#[test]
fn copy_and_move_replace_files_without_exposing_partial_copy_state() {
    let temporary = TestDirectory::new();
    let filesystem = LocalFileSystem::new();
    let source = temporary.0.join("source");
    let moved = temporary.0.join("moved");
    let target = temporary.0.join("target");
    std::fs::write(&source, b"copied").unwrap();
    std::fs::write(&target, b"old").unwrap();

    filesystem
        .copy_file(&as_path(&source), &as_path(&target))
        .unwrap();
    assert_eq!(std::fs::read(&source).unwrap(), b"copied");
    assert_eq!(std::fs::read(&target).unwrap(), b"copied");

    std::fs::write(&moved, b"moved").unwrap();
    filesystem
        .move_file(&as_path(&moved), &as_path(&target))
        .unwrap();
    assert!(!moved.exists());
    assert_eq!(std::fs::read(&target).unwrap(), b"moved");

    let missing = temporary.0.join("missing");
    assert!(
        filesystem
            .copy_file(&as_path(&missing), &as_path(&target))
            .unwrap_err()
            .is_absent()
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"moved");

    let directory_target = temporary.0.join("directory-target");
    std::fs::create_dir(&directory_target).unwrap();
    assert!(
        filesystem
            .copy_file(&as_path(&source), &as_path(&directory_target))
            .is_err()
    );
    assert!(directory_target.is_dir());
    assert!(std::fs::read_dir(&temporary.0).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".yggdryl-copy-")
    }));
}

#[cfg(unix)]
fn create_directory_symlink(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).unwrap();
    true
}

#[cfg(windows)]
fn create_directory_symlink(target: &Path, link: &Path) -> bool {
    match std::os::windows::fs::symlink_dir(target, link) {
        Ok(()) => true,
        Err(error)
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(1314) =>
        {
            false
        }
        Err(error) => panic!("failed to create directory symlink: {error}"),
    }
}

#[cfg(not(any(unix, windows)))]
fn create_directory_symlink(_target: &Path, _link: &Path) -> bool {
    false
}
