//! A memory filesystem that tallies every call reaching it by name, for the
//! cost pins of a surface that takes a [`Holder`](yggdryl::holder::Holder):
//! `Counted` wraps a byte handle, and `Holder` has no variant for a counted
//! one to arrive as, so a store behind an [`FsFolder`] is where the count is
//! taken.

// Each target that declares this module reads the tally its own way.
#![allow(dead_code)]

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use yggdryl::Result;
use yggdryl::fs::{
    ByteReader, ByteWriter, FileInfo, FileInfos, FileSelector, FileSystem, FsFolder,
    MemoryFileSystem, OutputMetadata, RandomAccessReader,
};

/// A memory filesystem that tallies every vtable call reaching it.
#[derive(Debug, Default)]
pub struct CountingFileSystem {
    inner: MemoryFileSystem,
    calls: Mutex<BTreeMap<&'static str, usize>>,
    sizeless: AtomicBool,
    name: Option<&'static str>,
    /// The one thread this filesystem answers on, where it is bound to one.
    bound: Mutex<Option<std::thread::ThreadId>>,
    /// The calls that reached it from any other thread while bound.
    off_thread: AtomicUsize,
}

impl CountingFileSystem {
    /// A counting filesystem answering `name` as its type, as a binding's
    /// filesystem names the store it speaks to (`s3`, `gcs`, `mock`).
    pub fn named(name: &'static str) -> Self {
        Self {
            name: Some(name),
            ..Self::default()
        }
    }

    /// Report every file with no size from here on, as a store that cannot
    /// size its objects does: a reader must then read a file to count it.
    pub fn set_sizeless(&self, sizeless: bool) {
        self.sizeless.store(sizeless, Ordering::Relaxed);
    }

    /// `info` as this filesystem reports it: without a size when sizeless.
    fn reported(&self, mut info: FileInfo) -> FileInfo {
        if self.sizeless.load(Ordering::Relaxed) {
            info.size = None;
        }
        info
    }

    /// Answer only on the calling thread from here on, as a filesystem a
    /// JavaScript handler implements does: `is_thread_bound` says so, and
    /// every call from another thread is counted.
    pub fn bind_to_current_thread(&self) {
        *self.bound.lock().expect("the bound thread") = Some(std::thread::current().id());
    }

    /// The calls that reached this filesystem from another thread while it
    /// was bound to one.
    pub fn off_thread_calls(&self) -> usize {
        self.off_thread.load(Ordering::Relaxed)
    }

    /// The calls made so far, in all.
    pub fn calls(&self) -> usize {
        self.calls.lock().expect("the tally").values().sum()
    }

    /// The calls `operation` makes, on top of what was made before it.
    pub fn cost(&self, operation: impl FnOnce()) -> usize {
        let before = self.calls();
        operation();
        self.calls() - before
    }

    /// The calls `operation` makes, by name: `list=1 file_info=2`, or `none`.
    pub fn costs(&self, operation: impl FnOnce()) -> String {
        self.calls.lock().expect("the tally").clear();
        operation();
        let tally = self.calls.lock().expect("the tally");
        if tally.is_empty() {
            return "none".to_owned();
        }
        tally
            .iter()
            .map(|(name, count)| format!("{name}={count}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn count(&self, name: &'static str) {
        if self
            .bound
            .lock()
            .expect("the bound thread")
            .is_some_and(|bound| bound != std::thread::current().id())
        {
            self.off_thread.fetch_add(1, Ordering::Relaxed);
        }
        *self
            .calls
            .lock()
            .expect("the tally")
            .entry(name)
            .or_insert(0) += 1;
    }
}

/// A folder named `name` over a fresh counting filesystem, with the tally
/// beside it.
pub fn counted_folder(name: &str) -> (Arc<CountingFileSystem>, FsFolder) {
    let filesystem = Arc::new(CountingFileSystem::default());
    let folder = FsFolder::from_path(Arc::clone(&filesystem) as Arc<dyn FileSystem>, name, None)
        .expect("a valid location");
    (filesystem, folder)
}

impl FileSystem for CountingFileSystem {
    fn type_name(&self) -> &str {
        self.name.unwrap_or_else(|| self.inner.type_name())
    }

    fn is_thread_bound(&self) -> bool {
        self.bound.lock().expect("the bound thread").is_some()
    }

    fn equals(&self, other: &dyn FileSystem) -> bool {
        self.count("equals");
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| std::ptr::eq(self, other))
    }

    fn normalize_path(&self, path: &str) -> Result<String> {
        self.count("normalize_path");
        self.inner.normalize_path(path)
    }

    fn file_info(&self, path: &str) -> Result<FileInfo> {
        self.count("file_info");
        self.inner.file_info(path).map(|info| self.reported(info))
    }

    fn list(&self, selector: &FileSelector) -> FileInfos {
        self.count("list");
        let sizeless = self.sizeless.load(Ordering::Relaxed);
        FileInfos::new(self.inner.list(selector).map(move |info| {
            info.map(|mut info| {
                if sizeless {
                    info.size = None;
                }
                info
            })
        }))
    }

    fn create_dir(&self, path: &str, recursive: bool) -> Result<()> {
        self.count("create_dir");
        self.inner.create_dir(path, recursive)
    }

    fn delete_dir(&self, path: &str) -> Result<()> {
        self.count("delete_dir");
        self.inner.delete_dir(path)
    }

    fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> Result<()> {
        self.count("delete_dir_contents");
        self.inner.delete_dir_contents(path, missing_dir_ok)
    }

    fn delete_root_dir_contents(&self) -> Result<()> {
        self.count("delete_root_dir_contents");
        self.inner.delete_root_dir_contents()
    }

    fn delete_file(&self, path: &str) -> Result<()> {
        self.count("delete_file");
        self.inner.delete_file(path)
    }

    fn copy_file(&self, source: &str, target: &str) -> Result<()> {
        self.count("copy_file");
        self.inner.copy_file(source, target)
    }

    fn move_file(&self, source: &str, target: &str) -> Result<()> {
        self.count("move_file");
        self.inner.move_file(source, target)
    }

    fn open_input_file(&self, path: &str) -> Result<Box<dyn RandomAccessReader>> {
        self.count("open_input_file");
        self.inner.open_input_file(path)
    }

    fn open_input_stream(&self, path: &str) -> Result<Box<dyn ByteReader>> {
        self.count("open_input_stream");
        self.inner.open_input_stream(path)
    }

    fn open_output_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        self.count("open_output_stream");
        self.inner.open_output_stream(path, metadata)
    }

    fn open_append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>> {
        self.count("open_append_stream");
        self.inner.open_append_stream(path, metadata)
    }

    fn create_file(&self, path: &str, bytes: &[u8]) -> Result<()> {
        self.count("create_file");
        self.inner.create_file(path, bytes)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
