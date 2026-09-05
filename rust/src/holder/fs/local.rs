//! Local-disk reference implementation of the Arrow filesystem seam.

use std::any::Any;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Error, IOKind, Result};

use super::{
    ByteReader, ByteWriter, FileInfo, FileInfos, FileSelector, FileSystem, OutputMetadata,
    RandomAccessReader, mask_uri, stream::StreamFailure,
};

/// The stateless local filesystem.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalFileSystem;

impl LocalFileSystem {
    /// Construct the local filesystem.
    pub fn new() -> Self {
        Self
    }
}

impl FileSystem for LocalFileSystem {
    fn type_name(&self) -> &str {
        "file"
    }

    fn equals(&self, other: &dyn FileSystem) -> bool {
        other.as_any().is::<Self>()
    }

    fn normalize_path(&self, path: &str) -> Result<String> {
        Ok(std::path::Path::new(path)
            .to_string_lossy()
            .replace('\\', "/"))
    }

    fn file_info(&self, path: &str) -> Result<FileInfo> {
        match std::fs::metadata(path) {
            Ok(metadata) if metadata.is_dir() => Ok(FileInfo::directory(
                path,
                metadata.modified().ok().and_then(system_time_ns),
            )),
            Ok(metadata) => Ok(FileInfo::file(
                path,
                metadata.len(),
                metadata.modified().ok().and_then(system_time_ns),
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(FileInfo::not_found(path))
            }
            Err(error) => Err(Error::from_io_at(error, "path", mask_uri(path))),
        }
    }

    fn list(&self, selector: &FileSelector) -> FileInfos {
        let first = level(&selector.base_dir, selector.allow_not_found);
        if !selector.recursive {
            return first;
        }
        FileInfos::new(LocalListing::new(first))
    }

    fn create_dir(&self, path: &str, recursive: bool) -> Result<()> {
        let result = if recursive {
            std::fs::create_dir_all(path)
        } else {
            std::fs::create_dir(path)
        };
        match result {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                match std::fs::metadata(path) {
                    Ok(metadata) if metadata.is_dir() => Ok(()),
                    Ok(_) => Err(Error::conflict("directory", "file", mask_uri(path))),
                    Err(metadata_error) => Err(Error::from_io_at(
                        metadata_error,
                        "directory",
                        mask_uri(path),
                    )),
                }
            }
            Err(error) => Err(Error::from_io_at(error, "directory", mask_uri(path))),
        }
    }

    fn delete_dir(&self, path: &str) -> Result<()> {
        reject_resolved_root(path, "delete_dir at filesystem root")?;
        std::fs::remove_dir(path)
            .map_err(|error| Error::from_io_at(error, "directory", mask_uri(path)))
    }

    fn delete_dir_recursive(&self, path: &str) -> Result<()> {
        reject_resolved_root(path, "recursive delete at filesystem root")?;
        std::fs::remove_dir_all(path)
            .map_err(|error| Error::from_io_at(error, "directory", mask_uri(path)))
    }

    fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> Result<()> {
        let resolved = match resolve_non_root(
            path,
            "delete_dir_contents at filesystem root; use delete_root_dir_contents",
        ) {
            Ok(resolved) => resolved,
            Err(error) if missing_dir_ok && error.is_absent() => return Ok(()),
            Err(error) => return Err(error),
        };
        let entries = match std::fs::read_dir(&resolved) {
            Ok(entries) => entries,
            Err(error) if missing_dir_ok && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(());
            }
            Err(error) => {
                return Err(Error::from_io_at(error, "directory", mask_uri(path)));
            }
        };
        for entry in entries {
            let entry = entry.map_err(Error::Io)?;
            let kind = entry.file_type().map_err(Error::Io)?;
            if kind.is_dir() {
                std::fs::remove_dir_all(entry.path()).map_err(Error::Io)?;
            } else {
                std::fs::remove_file(entry.path()).map_err(Error::Io)?;
            }
        }
        Ok(())
    }

    fn delete_root_dir_contents(&self) -> Result<()> {
        Err(Error::unsupported("delete_root_dir_contents", "file"))
    }

    fn delete_file(&self, path: &str) -> Result<()> {
        std::fs::remove_file(path).map_err(|error| file_operation_error(error, path))
    }

    fn copy_file(&self, source: &str, target: &str) -> Result<()> {
        copy_then_publish(source, target)
    }

    fn move_file(&self, source: &str, target: &str) -> Result<()> {
        std::fs::rename(source, target)
            .map_err(|error| Error::from_io_at(error, "file", mask_uri(source)))
    }

    fn open_input_file(&self, path: &str) -> Result<Box<dyn RandomAccessReader>> {
        let file = std::fs::File::open(path).map_err(|error| file_operation_error(error, path))?;
        reject_directory(&file, path)?;
        Ok(Box::new(LocalReader::new(file)))
    }

    fn open_input_stream(&self, path: &str) -> Result<Box<dyn ByteReader>> {
        let file = std::fs::File::open(path).map_err(|error| file_operation_error(error, path))?;
        reject_directory(&file, path)?;
        Ok(Box::new(LocalReader::new(file)))
    }

    fn open_output_stream(
        &self,
        path: &str,
        _metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|error| file_operation_error(error, path))?;
        Ok(Box::new(LocalWriter::new(file, 0)))
    }

    fn open_append_stream(
        &self,
        path: &str,
        _metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        let file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .map_err(|error| file_operation_error(error, path))?;
        let position = file.metadata().map_err(Error::Io)?.len();
        Ok(Box::new(LocalWriter::new(file, position)))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn level(path: &str, allow_not_found: bool) -> FileInfos {
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if allow_not_found && error.kind() == std::io::ErrorKind::NotFound => {
            return FileInfos::empty();
        }
        Err(error) => {
            return FileInfos::failing(Error::from_io_at(error, "directory", mask_uri(path)));
        }
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return FileInfos::failing(Error::Io(error)),
        };
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => return FileInfos::failing(Error::Io(error)),
        };
        let child = entry.path().to_string_lossy().replace('\\', "/");
        let mtime_ns = metadata.modified().ok().and_then(system_time_ns);
        found.push(if metadata.is_dir() {
            FileInfo::directory(child, mtime_ns)
        } else {
            FileInfo::file(child, metadata.len(), mtime_ns)
        });
    }
    found.sort_by(|left, right| left.path.cmp(&right.path));
    FileInfos::new(found.into_iter().map(Ok))
}

struct LocalListing {
    initial: Option<FileInfos>,
    frontier: BinaryHeap<Reverse<FileInfo>>,
    pending_error: Option<Error>,
    failed: bool,
}

impl LocalListing {
    fn new(initial: FileInfos) -> Self {
        Self {
            initial: Some(initial),
            frontier: BinaryHeap::new(),
            pending_error: None,
            failed: false,
        }
    }

    fn add_level(&mut self, mut entries: FileInfos) -> Result<()> {
        for entry in &mut entries {
            self.frontier.push(Reverse(entry?));
        }
        Ok(())
    }
}

impl Iterator for LocalListing {
    type Item = Result<FileInfo>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        if let Some(error) = self.pending_error.take() {
            self.failed = true;
            return Some(Err(error));
        }
        if let Some(initial) = self.initial.take()
            && let Err(error) = self.add_level(initial)
        {
            self.failed = true;
            return Some(Err(error));
        }
        let Reverse(info) = self.frontier.pop()?;
        if info.kind == IOKind::Directory {
            match std::fs::symlink_metadata(&info.path) {
                Ok(metadata) if metadata.file_type().is_dir() => {
                    if let Err(error) = self.add_level(level(&info.path, false)) {
                        self.pending_error = Some(error);
                    }
                }
                Ok(_) => {}
                Err(error) => self.pending_error = Some(Error::Io(error)),
            }
        }
        Some(Ok(info))
    }
}

fn system_time_ns(value: SystemTime) -> Option<i64> {
    match value.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_nanos()).ok(),
        Err(error) => i64::try_from(error.duration().as_nanos())
            .ok()
            .and_then(i64::checked_neg),
    }
}

fn resolve_non_root(path: &str, operation: &'static str) -> Result<PathBuf> {
    let path_value = Path::new(path);
    if path_value.as_os_str().is_empty()
        || path_value
            .components()
            .all(|component| matches!(component, std::path::Component::CurDir))
    {
        return Err(Error::unsupported(operation, "file"));
    }
    let resolved = std::fs::canonicalize(path)
        .map_err(|error| Error::from_io_at(error, "directory", mask_uri(path)))?;
    if is_local_root(&resolved) {
        return Err(Error::unsupported(operation, "file"));
    }
    Ok(resolved)
}

fn reject_resolved_root(path: &str, operation: &'static str) -> Result<()> {
    resolve_non_root(path, operation).map(|_| ())
}

fn is_local_root(path: &Path) -> bool {
    path.has_root()
        && !path
            .components()
            .any(|component| matches!(component, std::path::Component::Normal(_)))
}

fn copy_then_publish(source: &str, target: &str) -> Result<()> {
    let source_open =
        std::fs::File::open(source).map_err(|error| file_operation_error(error, source))?;
    drop(source_open);
    let (temporary_path, reservation) = temporary_file(target)?;
    let result = (|| {
        std::fs::copy(source, &temporary_path)
            .map_err(|error| file_operation_error(error, source))?;
        drop(reservation);
        // `rename` is the platform replace-existing primitive, including on
        // Windows. Publication occurs only after the complete copy is closed.
        std::fs::rename(&temporary_path, target)
            .map_err(|error| Error::from_io_at(error, "file", mask_uri(target)))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    result
}

fn temporary_file(target: &str) -> Result<(PathBuf, std::fs::File)> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let target = Path::new(target);
    let parent = target.parent().ok_or_else(|| invalid_file_path(target))?;
    target
        .file_name()
        .ok_or_else(|| invalid_file_path(target))?;
    for _ in 0..1024 {
        let candidate = parent.join(format!(
            ".yggdryl-copy-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(Error::from_io_at(
                    error,
                    "file",
                    mask_uri(&target.to_string_lossy()),
                ));
            }
        }
    }
    Err(Error::conflict(
        "temporary file",
        "file",
        mask_uri(&target.to_string_lossy()),
    ))
}

fn invalid_file_path(path: &Path) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!(
            "expected a file path, got {:?}",
            mask_uri(&path.to_string_lossy())
        ),
    ))
}

fn reject_directory(file: &std::fs::File, path: &str) -> Result<()> {
    if file.metadata().map_err(Error::Io)?.is_dir() {
        return Err(is_directory(path));
    }
    Ok(())
}

fn file_operation_error(error: std::io::Error, path: &str) -> Error {
    if matches!(
        error.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Other
    ) && std::fs::metadata(path).is_ok_and(|metadata| metadata.is_dir())
    {
        return is_directory(path);
    }
    Error::from_io_at(error, "file", mask_uri(path))
}

fn is_directory(path: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::IsADirectory,
        format!("expected a file at {:?}, got a directory", mask_uri(path)),
    ))
}

struct LocalReader {
    file: Option<std::fs::File>,
    position: u64,
}

impl LocalReader {
    fn new(file: std::fs::File) -> Self {
        Self {
            file: Some(file),
            position: 0,
        }
    }

    fn file(&mut self) -> Result<&mut std::fs::File> {
        self.file.as_mut().ok_or_else(|| closed("input"))
    }
}

impl ByteReader for LocalReader {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        let read = self.file()?.read(buffer).map_err(Error::Io)?;
        self.position += read as u64;
        Ok(read)
    }

    fn tell(&self) -> u64 {
        self.position
    }

    fn close(&mut self) -> Result<()> {
        self.file.take();
        Ok(())
    }

    fn closed(&self) -> bool {
        self.file.is_none()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl RandomAccessReader for LocalReader {
    fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let position = self.position;
        let file = self.file()?;
        file.seek(SeekFrom::Start(offset)).map_err(Error::Io)?;
        let result = file.read(buffer).map_err(Error::Io);
        file.seek(SeekFrom::Start(position)).map_err(Error::Io)?;
        result
    }

    fn seek(&mut self, from: SeekFrom) -> Result<u64> {
        let position = self.file()?.seek(from).map_err(Error::Io)?;
        self.position = position;
        Ok(position)
    }

    fn into_random_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

struct LocalWriter {
    file: Option<std::fs::File>,
    position: u64,
    failure: Option<StreamFailure>,
}

impl LocalWriter {
    fn new(file: std::fs::File, position: u64) -> Self {
        Self {
            file: Some(file),
            position,
            failure: None,
        }
    }

    fn file(&mut self) -> Result<&mut std::fs::File> {
        self.file.as_mut().ok_or_else(|| closed("output"))
    }

    fn remember(&mut self, error: Error) -> Error {
        if self.failure.is_none() {
            self.failure = Some(StreamFailure::from_error(&error));
        }
        error
    }
}

impl ByteWriter for LocalWriter {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if let Some(failure) = &self.failure {
            return Err(failure.error());
        }
        let result = self.file()?.write(bytes).map_err(Error::Io);
        match result {
            Ok(written) => {
                self.position += written as u64;
                Ok(written)
            }
            Err(error) => Err(self.remember(error)),
        }
    }

    fn tell(&self) -> u64 {
        self.position
    }

    fn flush(&mut self) -> Result<()> {
        if let Some(failure) = &self.failure {
            return Err(failure.error());
        }
        let result = self.file()?.flush().map_err(Error::Io);
        match result {
            Ok(()) => Ok(()),
            Err(error) => Err(self.remember(error)),
        }
    }

    fn close(&mut self) -> Result<()> {
        let Some(mut file) = self.file.take() else {
            return self
                .failure
                .as_ref()
                .map_or(Ok(()), |failure| Err(failure.error()));
        };
        if let Some(failure) = &self.failure {
            drop(file);
            return Err(failure.error());
        };
        if let Err(error) = file.flush() {
            return Err(self.remember(Error::Io(error)));
        }
        Ok(())
    }

    fn closed(&self) -> bool {
        self.file.is_none()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

fn closed(kind: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::NotConnected,
        format!("{kind} stream is closed"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
