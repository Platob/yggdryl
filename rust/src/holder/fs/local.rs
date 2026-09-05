//! Local-disk reference implementation of the Arrow filesystem seam.

use std::any::Any;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Error, IOKind, Result};

use super::{
    ByteReader, ByteWriter, FileInfo, FileInfos, FileSelector, FileSystem, OutputMetadata,
    RandomAccessReader, stream::StreamFailure,
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
            Err(error) => Err(Error::from_io_at(error, "path", path)),
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
        result.map_err(|error| Error::from_io_at(error, "directory", path))
    }

    fn delete_dir(&self, path: &str) -> Result<()> {
        if is_local_root(path) {
            return Err(Error::unsupported("delete_dir at filesystem root", "file"));
        }
        std::fs::remove_dir(path).map_err(|error| Error::from_io_at(error, "directory", path))
    }

    fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> Result<()> {
        if is_local_root(path) {
            return Err(Error::unsupported(
                "delete_dir_contents at filesystem root; use delete_root_dir_contents",
                "file",
            ));
        }
        let entries = match std::fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if missing_dir_ok && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(());
            }
            Err(error) => return Err(Error::from_io_at(error, "directory", path)),
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
        std::fs::copy(source, target)
            .map(|_| ())
            .map_err(|error| Error::from_io_at(error, "file", source))
    }

    fn move_file(&self, source: &str, target: &str) -> Result<()> {
        std::fs::rename(source, target).map_err(|error| Error::from_io_at(error, "file", source))
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
        Err(error) => return FileInfos::failing(Error::from_io_at(error, "directory", path)),
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
        if info.kind == IOKind::Directory
            && let Err(error) = self.add_level(level(&info.path, false))
        {
            self.pending_error = Some(error);
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

fn is_local_root(path: &str) -> bool {
    if path.is_empty() || matches!(path, "." | "./" | ".\\") {
        return true;
    }
    let path = std::path::Path::new(path);
    path.has_root()
        && !path
            .components()
            .any(|component| matches!(component, std::path::Component::Normal(_)))
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
    Error::from_io_at(error, "file", path)
}

fn is_directory(path: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::IsADirectory,
        format!("expected a file at {path:?}, got a directory"),
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
