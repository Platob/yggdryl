//! In-memory reference implementation of the Arrow filesystem seam.

use std::any::Any;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::io::SeekFrom;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Error, IOKind, Result};

use super::{
    ByteReader, ByteWriter, FileInfo, FileInfos, FileSelector, FileSystem, OutputMetadata,
    RandomAccessReader, mask_uri, stream::StreamFailure,
};

#[derive(Clone)]
struct Entry {
    bytes: Arc<Vec<u8>>,
    mtime_ns: Option<i64>,
}

#[derive(Default)]
struct State {
    files: BTreeMap<String, Entry>,
    directories: BTreeSet<String>,
}

/// One isolated in-memory filesystem equality domain.
#[derive(Clone, Default)]
pub struct MemoryFileSystem {
    state: Arc<Mutex<State>>,
}

impl MemoryFileSystem {
    /// Create an empty filesystem.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>> {
        self.state.lock().map_err(|_| {
            Error::Io(std::io::Error::other(
                "the memory filesystem lock was poisoned",
            ))
        })
    }
}

impl std::fmt::Debug for MemoryFileSystem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryFileSystem")
            .finish_non_exhaustive()
    }
}

fn beneath<'path>(base: &str, path: &'path str) -> Option<&'path str> {
    if base.is_empty() {
        return (!path.is_empty()).then_some(path);
    }
    path.strip_prefix(base)?.strip_prefix('/')
}

fn info(state: &State, path: &str) -> FileInfo {
    if path.is_empty() {
        return FileInfo::directory(path, None);
    }
    if let Some(entry) = state.files.get(path) {
        return FileInfo::file(path, entry.bytes.len() as u64, entry.mtime_ns);
    }
    if state.directories.contains(path)
        || state
            .files
            .keys()
            .any(|candidate| beneath(path, candidate).is_some())
        || state
            .directories
            .iter()
            .any(|candidate| beneath(path, candidate).is_some())
    {
        return FileInfo::directory(path, None);
    }
    FileInfo::not_found(path)
}

fn now_ns() -> Option<i64> {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_nanos()).ok(),
        Err(error) => i64::try_from(error.duration().as_nanos())
            .ok()
            .and_then(i64::checked_neg),
    }
}

impl FileSystem for MemoryFileSystem {
    fn type_name(&self) -> &str {
        "memory"
    }

    fn equals(&self, other: &dyn FileSystem) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| Arc::ptr_eq(&self.state, &other.state))
    }

    fn normalize_path(&self, path: &str) -> Result<String> {
        Ok(path.trim_matches('/').to_owned())
    }

    fn file_info(&self, path: &str) -> Result<FileInfo> {
        let state = self.lock()?;
        Ok(info(&state, path))
    }

    fn list(&self, selector: &FileSelector) -> FileInfos {
        let state = match self.lock() {
            Ok(state) => state,
            Err(error) => return FileInfos::failing(error),
        };
        match info(&state, &selector.base_dir).kind {
            IOKind::Unknown if selector.allow_not_found => return FileInfos::empty(),
            IOKind::Unknown => {
                return FileInfos::failing(Error::absent("directory", &selector.base_dir));
            }
            IOKind::File => {
                return FileInfos::failing(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    format!(
                        "expected a directory at {:?}, got a file",
                        mask_uri(&selector.base_dir)
                    ),
                )));
            }
            _ => {}
        }
        drop(state);
        if selector.recursive {
            FileInfos::new(MemoryListing::new(self.clone(), selector.base_dir.clone()))
        } else {
            match self.level(&selector.base_dir) {
                Ok(found) => FileInfos::new(found.into_iter().map(Ok)),
                Err(error) => FileInfos::failing(error),
            }
        }
    }

    fn create_dir(&self, path: &str, recursive: bool) -> Result<()> {
        let mut state = self.lock()?;
        if state.files.contains_key(path) {
            return Err(not_directory(path));
        }
        if recursive {
            let mut current = String::new();
            for (index, part) in path.split('/').enumerate() {
                if index > 0 {
                    current.push('/');
                }
                current.push_str(part);
                if current.is_empty() {
                    continue;
                }
                if state.files.contains_key(&current) {
                    return Err(not_directory(&current));
                }
                state.directories.insert(current.clone());
            }
        } else {
            if let Some((parent, _)) = path.rsplit_once('/') {
                if !parent.is_empty() && info(&state, parent).kind != IOKind::Directory {
                    return Err(Error::absent("parent directory", parent));
                }
            }
            state.directories.insert(path.to_owned());
        }
        Ok(())
    }

    fn delete_dir(&self, path: &str) -> Result<()> {
        if path.is_empty() {
            return Err(Error::unsupported(
                "delete_dir at filesystem root",
                "memory",
            ));
        }
        let mut state = self.lock()?;
        match info(&state, path).kind {
            IOKind::Unknown => return Err(Error::absent("directory", path)),
            IOKind::File => return Err(not_directory(path)),
            _ => {}
        }
        if state
            .files
            .keys()
            .any(|candidate| beneath(path, candidate).is_some())
            || state
                .directories
                .iter()
                .any(|candidate| beneath(path, candidate).is_some())
        {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::DirectoryNotEmpty,
                format!("directory {:?} is not empty", mask_uri(path)),
            )));
        }
        state.directories.remove(path);
        Ok(())
    }

    fn delete_dir_recursive(&self, path: &str) -> Result<()> {
        if path.is_empty() {
            return Err(Error::unsupported(
                "recursive delete at filesystem root",
                "memory",
            ));
        }
        let mut state = self.lock()?;
        match info(&state, path).kind {
            IOKind::Unknown => return Err(Error::absent("directory", path)),
            IOKind::File => return Err(not_directory(path)),
            _ => {}
        }
        state
            .files
            .retain(|candidate, _| beneath(path, candidate).is_none());
        state
            .directories
            .retain(|candidate| candidate != path && beneath(path, candidate).is_none());
        Ok(())
    }

    fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> Result<()> {
        if path.is_empty() {
            return Err(Error::unsupported(
                "delete_dir_contents at filesystem root; use delete_root_dir_contents",
                "memory",
            ));
        }
        let mut state = self.lock()?;
        match info(&state, path).kind {
            IOKind::Unknown if missing_dir_ok => return Ok(()),
            IOKind::Unknown => return Err(Error::absent("directory", path)),
            IOKind::File => return Err(not_directory(path)),
            _ => {}
        }
        state
            .files
            .retain(|candidate, _| beneath(path, candidate).is_none());
        state
            .directories
            .retain(|candidate| candidate == path || beneath(path, candidate).is_none());
        if !path.is_empty() {
            state.directories.insert(path.to_owned());
        }
        Ok(())
    }

    fn delete_root_dir_contents(&self) -> Result<()> {
        let mut state = self.lock()?;
        state.files.clear();
        state.directories.clear();
        Ok(())
    }

    fn delete_file(&self, path: &str) -> Result<()> {
        let mut state = self.lock()?;
        if state.files.remove(path).is_some() {
            return Ok(());
        }
        if info(&state, path).kind == IOKind::Directory {
            return Err(is_directory(path));
        }
        Err(Error::absent("file", path))
    }

    fn copy_file(&self, source: &str, target: &str) -> Result<()> {
        let mut state = self.lock()?;
        let source_entry = match state.files.get(source).cloned() {
            Some(entry) => entry,
            None if info(&state, source).kind == IOKind::Directory => {
                return Err(is_directory(source));
            }
            None => return Err(Error::absent("file", source)),
        };
        if info(&state, target).kind == IOKind::Directory {
            return Err(is_directory(target));
        }
        validate_file_parent(&state, target)?;
        state.files.insert(
            target.to_owned(),
            Entry {
                bytes: source_entry.bytes,
                mtime_ns: now_ns(),
            },
        );
        Ok(())
    }

    fn move_file(&self, source: &str, target: &str) -> Result<()> {
        let mut state = self.lock()?;
        let entry = match state.files.get(source).cloned() {
            Some(entry) => entry,
            None if info(&state, source).kind == IOKind::Directory => {
                return Err(is_directory(source));
            }
            None => return Err(Error::absent("file", source)),
        };
        if info(&state, target).kind == IOKind::Directory {
            return Err(is_directory(target));
        }
        validate_file_parent(&state, target)?;
        state.files.remove(source);
        state.files.insert(target.to_owned(), entry);
        Ok(())
    }

    fn open_input_file(&self, path: &str) -> Result<Box<dyn RandomAccessReader>> {
        Ok(Box::new(MemoryReader::new(self.bytes(path)?)))
    }

    fn open_input_stream(&self, path: &str) -> Result<Box<dyn ByteReader>> {
        Ok(Box::new(MemoryReader::new(self.bytes(path)?)))
    }

    fn open_output_stream(
        &self,
        path: &str,
        _metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        {
            let mut state = self.lock()?;
            if info(&state, path).kind == IOKind::Directory {
                return Err(is_directory(path));
            }
            validate_file_parent(&state, path)?;
            state.files.insert(
                path.to_owned(),
                Entry {
                    bytes: Arc::new(Vec::new()),
                    mtime_ns: now_ns(),
                },
            );
        }
        Ok(Box::new(MemoryWriter::new(
            Arc::clone(&self.state),
            path.to_owned(),
            0,
        )))
    }

    fn open_append_stream(
        &self,
        path: &str,
        _metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        let position = {
            let mut state = self.lock()?;
            match info(&state, path).kind {
                IOKind::File => state.files[path].bytes.len() as u64,
                IOKind::Directory => return Err(is_directory(path)),
                _ => {
                    validate_file_parent(&state, path)?;
                    state.files.insert(
                        path.to_owned(),
                        Entry {
                            bytes: Arc::new(Vec::new()),
                            mtime_ns: now_ns(),
                        },
                    );
                    0
                }
            }
        };
        Ok(Box::new(MemoryWriter::new(
            Arc::clone(&self.state),
            path.to_owned(),
            position,
        )))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl MemoryFileSystem {
    fn bytes(&self, path: &str) -> Result<Arc<Vec<u8>>> {
        let state = self.lock()?;
        state
            .files
            .get(path)
            .map(|entry| Arc::clone(&entry.bytes))
            .ok_or_else(|| match info(&state, path).kind {
                IOKind::Directory => is_directory(path),
                _ => Error::absent("file", path),
            })
    }

    fn level(&self, base: &str) -> Result<Vec<FileInfo>> {
        let state = self.lock()?;
        let mut found = BTreeMap::<String, FileInfo>::new();
        for path in state.directories.iter().chain(state.files.keys()) {
            let Some(relative) = beneath(base, path) else {
                continue;
            };
            let name = relative.split_once('/').map_or(relative, |(name, _)| name);
            let child = if base.is_empty() {
                name.to_owned()
            } else {
                format!("{base}/{name}")
            };
            found
                .entry(child.clone())
                .or_insert_with(|| info(&state, &child));
        }
        Ok(found.into_values().collect())
    }
}

fn validate_file_parent(state: &State, path: &str) -> Result<()> {
    if let Some((parent, _)) = path.rsplit_once('/') {
        if !parent.is_empty() {
            match info(state, parent).kind {
                IOKind::Directory => {}
                IOKind::File => return Err(not_directory(parent)),
                _ => return Err(Error::absent("parent directory", parent)),
            }
        }
    }
    Ok(())
}

struct MemoryListing {
    filesystem: MemoryFileSystem,
    initial: Option<String>,
    frontier: BinaryHeap<Reverse<FileInfo>>,
    pending_error: Option<Error>,
    failed: bool,
}

impl MemoryListing {
    fn new(filesystem: MemoryFileSystem, base: String) -> Self {
        Self {
            filesystem,
            initial: Some(base),
            frontier: BinaryHeap::new(),
            pending_error: None,
            failed: false,
        }
    }

    fn add_level(&mut self, base: &str) -> Result<()> {
        self.frontier
            .extend(self.filesystem.level(base)?.into_iter().map(Reverse));
        Ok(())
    }
}

impl Iterator for MemoryListing {
    type Item = Result<FileInfo>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        if let Some(error) = self.pending_error.take() {
            self.failed = true;
            return Some(Err(error));
        }
        if let Some(base) = self.initial.take() {
            if let Err(error) = self.add_level(&base) {
                self.failed = true;
                return Some(Err(error));
            }
        }
        let Reverse(info) = self.frontier.pop()?;
        if info.kind == IOKind::Directory {
            if let Err(error) = self.add_level(&info.path) {
                self.pending_error = Some(error);
            }
        }
        Some(Ok(info))
    }
}

fn not_directory(path: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::NotADirectory,
        format!("expected a directory at {:?}, got a file", mask_uri(path)),
    ))
}

fn is_directory(path: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::IsADirectory,
        format!("expected a file at {:?}, got a directory", mask_uri(path)),
    ))
}

struct MemoryReader {
    bytes: Arc<Vec<u8>>,
    position: u64,
    closed: bool,
}

impl MemoryReader {
    fn new(bytes: Arc<Vec<u8>>) -> Self {
        Self {
            bytes,
            position: 0,
            closed: false,
        }
    }

    fn ensure_open(&self, operation: &'static str) -> Result<()> {
        if self.closed {
            Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                format!("cannot {operation} a closed input stream"),
            )))
        } else {
            Ok(())
        }
    }
}

impl ByteReader for MemoryReader {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        self.ensure_open("read")?;
        let read = read_slice(&self.bytes, self.position, buffer);
        self.position += read as u64;
        Ok(read)
    }

    fn tell(&self) -> u64 {
        self.position
    }

    fn close(&mut self) -> Result<()> {
        self.closed = true;
        Ok(())
    }

    fn closed(&self) -> bool {
        self.closed
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl RandomAccessReader for MemoryReader {
    fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.ensure_open("read")?;
        Ok(read_slice(&self.bytes, offset, buffer))
    }

    fn seek(&mut self, from: SeekFrom) -> Result<u64> {
        self.ensure_open("seek")?;
        self.position = seek_target(self.position, self.bytes.len() as u64, from)?;
        Ok(self.position)
    }

    fn into_random_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

fn read_slice(bytes: &[u8], offset: u64, buffer: &mut [u8]) -> usize {
    let Ok(offset) = usize::try_from(offset) else {
        return 0;
    };
    let Some(available) = bytes.get(offset..) else {
        return 0;
    };
    let read = available.len().min(buffer.len());
    buffer[..read].copy_from_slice(&available[..read]);
    read
}

struct MemoryWriter {
    state: Arc<Mutex<State>>,
    path: String,
    position: u64,
    closed: bool,
    failure: Option<StreamFailure>,
}

impl MemoryWriter {
    fn new(state: Arc<Mutex<State>>, path: String, position: u64) -> Self {
        Self {
            state,
            path,
            position,
            closed: false,
            failure: None,
        }
    }

    fn remember(&mut self, error: Error) -> Error {
        if self.failure.is_none() {
            self.failure = Some(StreamFailure::from_error(&error));
        }
        error
    }
}

impl ByteWriter for MemoryWriter {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if let Some(failure) = &self.failure {
            return Err(failure.error());
        }
        if self.closed {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "cannot write a closed output stream",
            )));
        }
        let result = (|| {
            let mut state = self
                .state
                .lock()
                .map_err(|_| Error::Io(std::io::Error::other("memory filesystem lock poisoned")))?;
            let entry = state
                .files
                .get_mut(&self.path)
                .ok_or_else(|| Error::absent("file", &self.path))?;
            let target = Arc::make_mut(&mut entry.bytes);
            if usize::try_from(self.position).ok() != Some(target.len()) {
                return Err(Error::unsupported("non-sequential write", "memory"));
            }
            target.try_reserve(bytes.len()).map_err(|error| {
                Error::Io(std::io::Error::other(format!(
                    "cannot grow memory file: {error}"
                )))
            })?;
            target.extend_from_slice(bytes);
            entry.mtime_ns = now_ns();
            Ok(bytes.len())
        })();
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
        if self.closed {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "cannot flush a closed output stream",
            )));
        }
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        if let Some(failure) = &self.failure {
            self.closed = true;
            return Err(failure.error());
        }
        self.closed = true;
        Ok(())
    }

    fn closed(&self) -> bool {
        self.closed
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

fn seek_target(current: u64, end: u64, from: SeekFrom) -> Result<u64> {
    let target = match from {
        SeekFrom::Start(position) => Some(position),
        SeekFrom::Current(delta) => current.checked_add_signed(delta),
        SeekFrom::End(delta) => end.checked_add_signed(delta),
    };
    target.ok_or_else(|| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "a seek cannot land before the start",
        ))
    })
}
