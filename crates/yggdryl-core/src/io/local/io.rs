//! [`LocalIO`] — the **single access point** to the local filesystem: one lazy handle that
//! decides per call how to read and write.

use std::fs::{self, File};
use std::path::{Path as StdPath, PathBuf};

use super::{absolutize, file_err, read_at, uri_to_path, Mmap};
use crate::headers::Headers;
use crate::io::memory::{cursor_methods, IOBase, IoError, Whence};
use crate::io::{IOKind, IOMode};
use crate::uri::Uri;

/// The one local-filesystem handle — a **lazy** node over any path (file, folder, or nothing
/// yet) that **decides itself, per call, how to serve reads and writes**:
///
/// - **Constructing / probing / navigating touches nothing.** `kind` / `exists` / `is_file` /
///   `is_dir` ask the disk per call; `join_str` / `parent` are pure path math.
/// - **Reads pick their own path.** Before any write, a read opens the file ad hoc with one
///   positioned OS read (a missing or directory node reads as empty). After the handle has
///   written, reads are served from its **memory-mapped backing** — zero-allocation memory
///   access.
/// - **Writes auto-create and self-optimize.** The first write creates the missing parent
///   folders and the file, memory-maps it, and keeps the mapping — every later read/write on
///   this handle runs at memory speed with `Heap`-style amortized growth. No `mkdir`, no
///   `touch`, no separate "file object".
/// - **The graph is the same handle.** [`IOBase`](crate::io::memory::IOBase)'s graph surface —
///   navigation, streamed discovery (`ls` / `ls_recursive`), and CRUD (`rm` / `rmfile` /
///   `rmdir`) — all live here; [`mkdir`](LocalIO::mkdir) auto-creates a directory tree when a
///   folder itself is the goal.
/// - **A directory is a memory tree.** A directory node serves the *byte* contract too: its
///   [`byte_size`](IOBase::byte_size) is the lazy streamed sum of its subtree, and
///   `pread` / `pwrite` route across its **name-sorted child blocks** as one contiguous
///   region (the generic `tree_*` pattern on `IOBase` — object-store families inherit the
///   same behavior). Reads recurse through child directories; writes stay capped inside
///   each block (only the last block grows).
///
/// [`close`](LocalIO::close) releases the mapped backing eagerly (truncating the file to its
/// logical length); [`clone`](Clone::clone) yields a fresh **lazy** handle to the same path
/// (the mapping is not shared). DESIGN: the path *value* lives in [`uri`](IOBase::uri);
/// `LocalIO` is a live handle — it compares by path and carries no byte codec.
///
/// ```
/// use yggdryl_core::io::local::LocalIO;
/// use yggdryl_core::io::memory::IOBase;
///
/// let root = LocalIO::from_path(std::env::temp_dir().join("yggdryl_localio_doc"));
/// let mut note = root.join_str("deep/nested/note.txt");
/// assert!(!note.exists()); // lazy — nothing on disk yet
///
/// note.pwrite_utf8(0, "hello"); // auto-creates deep/, nested/, the file — and maps it
/// assert!(note.is_file());
/// assert_eq!(note.pread_utf8(0, 5).unwrap(), "hello"); // now served from the mapping
/// note.close(); // release the mapping (Windows cannot delete mapped files)
///
/// let names: Vec<String> = root
///     .ls_recursive()
///     .unwrap()
///     .map(|entry| entry.unwrap().name())
///     .collect();
/// assert!(names.contains(&"note.txt".to_string()));
///
/// // A directory is a memory tree: the root reads as one contiguous byte region.
/// assert_eq!(root.byte_size(), 5);
/// assert_eq!(root.pread_utf8(0, 5).unwrap(), "hello");
///
/// root.rmdir(true).unwrap();
/// assert!(!root.exists());
/// ```
#[derive(Debug)]
pub struct LocalIO {
    path: PathBuf,
    headers: Headers,
    mode: IOMode,
    /// The lazily-materialized optimized backing: `None` until the first write (reads before
    /// that open ad hoc), then a live [`Mmap`] serving everything at memory speed.
    map: Option<Mmap>,
    /// The built-in cursor — bytes from the start; may sit past the end after a seek.
    position: u64,
}

impl LocalIO {
    /// A lazy handle for `path` — nothing is touched or created. A **relative** path is made
    /// absolute against the current working directory, so the handle always carries a full
    /// absolute path (and reports a `file://` [`uri`](IOBase::uri)).
    pub fn from_path(path: impl AsRef<StdPath>) -> LocalIO {
        LocalIO {
            path: absolutize(path.as_ref()),
            headers: Headers::new(),
            mode: IOMode::ReadWrite,
            map: None,
            position: 0,
        }
    }

    /// A lazy handle addressed by `uri` (`file://…` or a plain-path URI).
    pub fn from_uri(uri: &Uri) -> Result<LocalIO, IoError> {
        Ok(Self::from_path(uri_to_path(uri)?))
    }

    /// A **lazy** handle to a temporary **file** in the system temp directory. `name` sets the
    /// file name; the default (`None`) is a process-unique name. Like any `LocalIO` it is
    /// lazy — the file is created on the **first write** — so this only picks the path.
    ///
    /// ```
    /// use yggdryl_core::io::local::LocalIO;
    /// use yggdryl_core::io::memory::IOBase;
    ///
    /// let mut scratch = LocalIO::tmpfile(None); // unique name, nothing on disk yet
    /// assert!(!scratch.exists());
    /// scratch.pwrite_utf8(0, "temp data"); // now created + mapped
    /// assert_eq!(scratch.pread_utf8(0, 9).unwrap(), "temp data");
    /// scratch.close();
    /// scratch.rmfile(true).unwrap();
    /// ```
    pub fn tmpfile(name: Option<&str>) -> LocalIO {
        let name = name
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}.tmp", Self::unique_tmp_name()));
        LocalIO::from_path(std::env::temp_dir().join(name))
    }

    /// A **lazy** handle to a temporary **folder** in the system temp directory. `name` sets
    /// the folder name; the default (`None`) is a process-unique name. Lazy — call
    /// [`mkdir`](LocalIO::mkdir) to create it, or just write a child (which auto-creates this
    /// folder as a parent).
    ///
    /// ```
    /// use yggdryl_core::io::local::LocalIO;
    /// use yggdryl_core::io::memory::IOBase;
    ///
    /// let work = LocalIO::tmpfolder(None);
    /// let mut file = work.join_str("out.bin"); // writing the child auto-creates `work`
    /// file.pwrite_byte_array(0, b"x");
    /// assert!(work.is_dir());
    /// file.close();
    /// work.rmdir(true).unwrap();
    /// ```
    pub fn tmpfolder(name: Option<&str>) -> LocalIO {
        let name = name
            .map(str::to_string)
            .unwrap_or_else(Self::unique_tmp_name);
        LocalIO::from_path(std::env::temp_dir().join(name))
    }

    /// Alias of [`tmpfolder`](LocalIO::tmpfolder) under the familiar `tmpdir` name (mirrors
    /// Python's `tempfile` vocabulary) — a **lazy** handle to a temporary folder.
    pub fn tmpdir(name: Option<&str>) -> LocalIO {
        Self::tmpfolder(name)
    }

    /// A process-unique base name (`yggdryl-<pid>-<counter>`) for the temp builders — no
    /// randomness needed: the pid plus a monotonic counter is unique within and across runs.
    fn unique_tmp_name() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        format!("yggdryl-{}-{}", std::process::id(), n)
    }

    /// The underlying filesystem path.
    pub fn as_std_path(&self) -> &StdPath {
        &self.path
    }

    /// The child node at `segment` (which may be a multi-segment relative path like
    /// `"a/b/c.txt"`) — **lazy** and **infallible**: the ergonomic inherent form of the
    /// graph-uniform [`join`](IOBase::join), which it delegates to (composing the child's
    /// address through the URI). Nothing is touched or created.
    pub fn join_str(&self, segment: &str) -> LocalIO {
        // `join` is infallible for a local node (its own fallback guarantees `Ok`).
        self.join(segment)
            .expect("join is infallible for a local node")
    }

    /// Auto-creates the directory tree at this path (like `mkdir -p`) — the explicit form
    /// when a **folder** itself is the goal (file-bound writes auto-create their parents on
    /// their own).
    pub fn mkdir(&self) -> Result<(), IoError> {
        fs::create_dir_all(&self.path).map_err(|e| file_err("create", &self.path, &e))
    }

    /// Flushes the mapped backing (if the handle has one) to disk.
    pub fn flush(&self) -> Result<(), IoError> {
        match &self.map {
            Some(map) => map.flush(),
            None => Ok(()), // nothing buffered: ad-hoc reads/writes go straight to the OS
        }
    }

    /// Releases the mapped backing eagerly (truncating the file to its logical length) —
    /// after which the handle is **still usable**: it simply returns to its lazy state.
    /// Idempotent. Call before removing a file this handle has written (Windows cannot
    /// delete a mapped file).
    ///
    /// While a handle holds its mapping, the on-disk file carries the mapping's **capacity
    /// padding** (amortized-growth headroom), so *other* handles to the same path observe
    /// the padded length until this one closes (or drops) and truncates back to the logical
    /// length. One writer at a time is the intended model.
    pub fn close(&mut self) {
        self.map = None;
    }

    /// Whether the handle currently holds its optimized mapped backing.
    pub fn is_mapped(&self) -> bool {
        self.map.is_some()
    }

    /// Builds a **standalone [`Mmap`]** over this node's file, **reusing the handle's own
    /// parameters** — its path, its [`IOMode`] (a read-only handle maps read-only, a
    /// read-write one maps read-write and auto-creates the missing parents + file, exactly
    /// like the first write), and its [`headers`](IOBase::headers), which are copied onto the
    /// returned mapping so a known `Content-Type` / cached size travels with it. The mapping is
    /// independent of this handle's own lazy [`is_mapped`](LocalIO::is_mapped) backing — the
    /// direct front door to the memory-mapped source when a caller wants to hold it itself.
    ///
    /// ```
    /// use yggdryl_core::io::local::LocalIO;
    /// use yggdryl_core::io::memory::IOBase;
    ///
    /// let node = LocalIO::tmpfile(None);
    /// let mut map = node.mmap().unwrap(); // creates + maps the file, read-write
    /// map.pwrite_utf8(0, "mapped");
    /// assert_eq!(map.pread_utf8(0, 6).unwrap(), "mapped");
    /// drop(map); // releasing the mapping writes back + lets the file be removed
    /// node.rmfile(true).unwrap();
    /// ```
    pub fn mmap(&self) -> Result<Mmap, IoError> {
        let mut map = if self.mode.is_writable() {
            if let Some(parent) = self.path.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    fs::create_dir_all(parent).map_err(|e| file_err("create", parent, &e))?;
                }
            }
            Mmap::create_path(&self.path)?
        } else {
            Mmap::open_path_readonly(&self.path)?
        };
        *map.headers_mut() = self.headers.clone(); // known headers travel with the mapping
        Ok(map)
    }

    /// **Eagerly memory-maps** the existing file so **every subsequent read (and write) runs at
    /// memory speed** — the explicit counterpart of the automatic map-on-first-write, for
    /// **read-heavy or concurrent** workloads. Without it, reads on a never-written handle open
    /// the file ad hoc (one positioned OS read per call); after it, they are served from the
    /// kept mapping with zero syscalls. Honors the handle's [`IOMode`](IOMode) (a read-only
    /// handle maps read-only). Because the backing [`Mmap`] is `Send + Sync`, a loaded handle
    /// shared across threads (e.g. behind an `Arc`) serves **concurrent readers** from one shared
    /// mapping. A no-op when already mapped or when the file does not exist yet (reads stay lazy —
    /// call it once the file is present); errors only if an existing file cannot be mapped.
    ///
    /// ```
    /// use yggdryl_core::io::local::LocalIO;
    /// use yggdryl_core::io::memory::IOBase;
    ///
    /// let mut w = LocalIO::tmpfile(None);
    /// w.pwrite_utf8(0, "cached read");
    /// w.close(); // file exists on disk, handle back to lazy
    ///
    /// let mut r = LocalIO::from_path(w.as_std_path());
    /// r.load().unwrap();        // map it once…
    /// assert!(r.is_mapped());
    /// assert_eq!(r.pread_utf8(0, 11).unwrap(), "cached read"); // …served from memory
    /// r.close();
    /// r.rmfile(true).unwrap();
    /// ```
    pub fn load(&mut self) -> Result<(), IoError> {
        if self.map.is_some() || !self.path.is_file() {
            return Ok(()); // already mapped, or nothing to map yet (reads stay ad-hoc/empty)
        }
        self.map = Some(if self.mode.is_writable() {
            Mmap::open_path(&self.path)?
        } else {
            Mmap::open_path_readonly(&self.path)?
        });
        Ok(())
    }

    /// The **disk capacity** of the volume backing this path — total and free bytes — as a
    /// [`MemoryInfo`](crate::io::MemoryInfo), the local-filesystem answer to "how much room is
    /// there?" (the same value type a GPU device reports for its VRAM, and an object store will
    /// report for its quota). Resolved through the platform route (Windows `GetDiskFreeSpaceExW`),
    /// walking up to the nearest existing ancestor so a not-yet-created path still resolves its
    /// volume; [`MemoryInfo::unknown`](crate::io::MemoryInfo::unknown) where no native route exists.
    ///
    /// ```
    /// use yggdryl_core::io::local::LocalIO;
    ///
    /// let info = LocalIO::from_path(std::env::temp_dir()).memory_info();
    /// assert!(info.total() >= info.available());
    /// ```
    pub fn memory_info(&self) -> crate::io::MemoryInfo {
        crate::io::disk_memory(&self.path)
    }

    /// Sets the access [`IOMode`] label in place (writes check it before touching the disk).
    pub fn set_mode(&mut self, mode: IOMode) {
        self.mode = mode;
    }

    /// Opens the file for one ad-hoc read, or `None` when nothing readable exists.
    fn open_read(&self) -> Option<File> {
        File::open(&self.path)
            .ok()
            .filter(|f| f.metadata().map(|m| m.is_file()).unwrap_or(false))
    }

    /// `Ok(())` when removing a **missing** node is allowed (`exist_ok`), else the guided
    /// "nothing here to remove" error naming the `exist_ok` fix.
    fn missing_ok(&self, exist_ok: bool) -> Result<(), IoError> {
        if exist_ok {
            Ok(())
        } else {
            Err(IoError::FileIo {
                op: "remove",
                path: self.path.to_string_lossy().into_owned(),
                detail: "nothing exists here to remove; pass exist_ok=true to skip a missing node"
                    .to_string(),
            })
        }
    }

    /// The guided error for a write-shaped call on a read-only handle.
    fn read_only_err(&self) -> IoError {
        IoError::FileIo {
            op: "write",
            path: self.path.to_string_lossy().into_owned(),
            detail: "the handle is read-only (IOMode::Read); set_mode(ReadWrite) to write"
                .to_string(),
        }
    }

    /// Materializes the mapped backing (auto-creating missing parents + the file) — the
    /// write path's self-optimization step.
    fn ensure_map(&mut self) -> Result<&mut Mmap, IoError> {
        if self.map.is_none() {
            if let Some(parent) = self.path.parent() {
                if !parent.as_os_str().is_empty() && !parent.exists() {
                    fs::create_dir_all(parent).map_err(|e| file_err("create", parent, &e))?;
                }
            }
            self.map = Some(Mmap::create_path(&self.path)?);
        }
        Ok(self.map.as_mut().expect("just ensured"))
    }

    cursor_methods!();
}

/// A clone is a fresh **lazy** handle to the same path — the mapped backing is deliberately
/// not shared (two live mappings of one file would alias).
impl Clone for LocalIO {
    fn clone(&self) -> Self {
        LocalIO {
            path: self.path.clone(),
            headers: self.headers.clone(),
            mode: self.mode,
            map: None,
            position: 0,
        }
    }
}

/// Handles compare by path (the value identity lives in `uri()`).
impl PartialEq for LocalIO {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}
impl Eq for LocalIO {}

impl IOBase for LocalIO {
    fn byte_size(&self) -> u64 {
        match &self.map {
            Some(map) => map.byte_size(),
            None => match fs::metadata(&self.path) {
                Ok(meta) if meta.is_file() => meta.len(),
                // A directory is a memory tree: its size is the lazy sum of its subtree.
                Ok(meta) if meta.is_dir() => self.tree_byte_size(),
                _ => 0,
            },
        }
    }

    fn capacity(&self) -> u64 {
        match &self.map {
            Some(map) => map.capacity(),
            None => self.byte_size(),
        }
    }

    fn reserve(&mut self, additional: u64) {
        if self.mode.is_writable() {
            if let Ok(map) = self.ensure_map() {
                map.reserve(additional);
            }
        }
    }

    fn reserve_exact(&mut self, additional: u64) {
        if self.mode.is_writable() {
            if let Ok(map) = self.ensure_map() {
                map.reserve_exact(additional);
            }
        }
    }

    fn try_reserve(&mut self, additional: u64) -> Result<(), IoError> {
        if !self.mode.is_writable() {
            return Err(self.read_only_err());
        }
        self.ensure_map()?.try_reserve(additional)
    }

    fn try_reserve_exact(&mut self, additional: u64) -> Result<(), IoError> {
        if !self.mode.is_writable() {
            return Err(self.read_only_err());
        }
        self.ensure_map()?.try_reserve_exact(additional)
    }

    fn shrink_to_fit(&mut self) {
        if let Some(map) = &mut self.map {
            map.shrink_to_fit();
        }
    }

    fn shrink_to(&mut self, min_capacity: u64) {
        if let Some(map) = &mut self.map {
            map.shrink_to(min_capacity);
        }
    }

    #[inline]
    fn as_bytes(&self) -> Option<&[u8]> {
        // Zero-copy only when self-optimized (mapped); an ad-hoc read has no contiguous view.
        self.map.as_ref().and_then(Mmap::as_bytes)
    }

    fn truncate(&mut self, len: u64) -> Result<(), IoError> {
        if !self.mode.is_writable() {
            return Err(self.read_only_err());
        }
        if self.map.is_none() && self.is_dir() {
            return Err(IoError::FileIo {
                op: "truncate",
                path: self.path.to_string_lossy().into_owned(),
                detail: "the node is a directory; truncate a file, not a folder".to_string(),
            });
        }
        // Auto-create + map, then resize the mapping (its own header sync is a harmless no-op).
        self.ensure_map()?.truncate(len)?;
        self.sync_size_headers();
        Ok(())
    }

    /// Releases the mapped backing (the `IOBase` trait hook — same effect as the inherent
    /// [`close`](LocalIO::close)): the handle returns to lazy, and the now-unmapped file can be
    /// removed even on Windows (which refuses to unlink a mapped file).
    fn close(&mut self) {
        self.map = None;
    }

    fn kind(&self) -> IOKind {
        if self.map.is_some() {
            return IOKind::File; // a mapped backing is by construction a live file
        }
        match fs::metadata(&self.path) {
            Ok(meta) if meta.is_dir() => IOKind::Directory,
            Ok(_) => IOKind::File,
            // DESIGN: `metadata` follows symlinks, so a dangling link errors — probe the
            // link itself so the node stays a removable File instead of a phantom Missing.
            Err(_) if fs::symlink_metadata(&self.path).is_ok() => IOKind::File,
            Err(_) => IOKind::Missing,
        }
    }

    fn uri(&self) -> Uri {
        // A local node reports a `file://` URL over its absolute path.
        Uri::from_file_path(&self.path.to_string_lossy())
    }

    #[inline]
    fn headers(&self) -> &Headers {
        &self.headers
    }

    #[inline]
    fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    #[inline]
    fn mode(&self) -> IOMode {
        self.mode
    }

    fn pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize {
        // The handle decides: mapped backing when it has one (memory speed), one ad-hoc
        // positioned OS read for a file, the memory tree for a directory; a missing node
        // reads as empty.
        match &self.map {
            Some(map) => map.pread_byte_array(offset, buf),
            None => match self.open_read() {
                Some(file) => read_at(&file, offset, buf).unwrap_or(0),
                None if self.is_dir() => self.tree_pread_byte_array(offset, buf),
                None => 0,
            },
        }
    }

    fn pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize {
        if data.is_empty() || !self.mode.is_writable() {
            return 0;
        }
        // A directory routes the write across its memory-tree blocks.
        if self.map.is_none() && self.is_dir() {
            return self.tree_pwrite_byte_array(offset, data);
        }
        // The first write self-optimizes: auto-create parents + file, map, keep the mapping.
        match self.ensure_map() {
            Ok(map) => map.pwrite_byte_array(offset, data),
            Err(_) => 0, // the full writes report the fix
        }
    }

    fn pwrite_all(&mut self, offset: u64, data: &[u8]) -> Result<(), IoError> {
        if !self.mode.is_writable() {
            return Err(self.read_only_err());
        }
        // An empty write is a no-op — never a reason to auto-create the file (`touch`).
        if data.is_empty() {
            return Ok(());
        }
        if self.map.is_none() && self.is_dir() {
            let written = self.tree_pwrite_byte_array(offset, data);
            if written == data.len() {
                return Ok(());
            }
            return Err(IoError::FileIo {
                op: "write",
                path: self.path.to_string_lossy().into_owned(),
                detail: format!(
                    "the directory tree absorbed {written} of {} bytes (an empty tree has \
                     no blocks, and only its last block can grow); join a file name onto \
                     this directory and write there",
                    data.len()
                ),
            });
        }
        self.ensure_map()?.pwrite_all(offset, data)
    }

    // ---- bulk typed access: once self-optimized (mapped), delegate straight to the map so
    // its direct contiguous conversion is reached; before that, reuse the shared staged
    // kernels over the byte methods (ad-hoc reads / the memory tree). ----

    fn pread_i32_array(&self, offset: u64, dst: &mut [i32]) -> Result<(), IoError> {
        match &self.map {
            Some(map) => map.pread_i32_array(offset, dst),
            None => crate::io::memory::stage_pread_i32_array(self, offset, dst),
        }
    }

    fn pread_i64_array(&self, offset: u64, dst: &mut [i64]) -> Result<(), IoError> {
        match &self.map {
            Some(map) => map.pread_i64_array(offset, dst),
            None => crate::io::memory::stage_pread_i64_array(self, offset, dst),
        }
    }

    fn pwrite_i32_array(&mut self, offset: u64, src: &[i32]) -> Result<(), IoError> {
        if self.map.is_none() && !self.is_dir() && self.mode.is_writable() {
            self.ensure_map()?; // a bulk write to a file self-optimizes, like the byte write
        }
        match &mut self.map {
            Some(map) => map.pwrite_i32_array(offset, src),
            None => crate::io::memory::stage_pwrite_i32_array(self, offset, src),
        }
    }

    fn pwrite_i64_array(&mut self, offset: u64, src: &[i64]) -> Result<(), IoError> {
        if self.map.is_none() && !self.is_dir() && self.mode.is_writable() {
            self.ensure_map()?;
        }
        match &mut self.map {
            Some(map) => map.pwrite_i64_array(offset, src),
            None => crate::io::memory::stage_pwrite_i64_array(self, offset, src),
        }
    }

    fn pwrite_byte_repeat(&mut self, offset: u64, value: u8, count: usize) -> Result<(), IoError> {
        if self.map.is_none() && !self.is_dir() && self.mode.is_writable() {
            self.ensure_map()?;
        }
        match &mut self.map {
            Some(map) => map.pwrite_byte_repeat(offset, value, count),
            None => crate::io::memory::stage_pwrite_byte_repeat(self, offset, value, count),
        }
    }

    fn pwrite_i32_repeat(&mut self, offset: u64, value: i32, count: usize) -> Result<(), IoError> {
        if self.map.is_none() && !self.is_dir() && self.mode.is_writable() {
            self.ensure_map()?;
        }
        match &mut self.map {
            Some(map) => map.pwrite_i32_repeat(offset, value, count),
            None => crate::io::memory::stage_pwrite_i32_repeat(self, offset, value, count),
        }
    }

    fn pwrite_i64_repeat(&mut self, offset: u64, value: i64, count: usize) -> Result<(), IoError> {
        if self.map.is_none() && !self.is_dir() && self.mode.is_writable() {
            self.ensure_map()?;
        }
        match &mut self.map {
            Some(map) => map.pwrite_i64_repeat(offset, value, count),
            None => crate::io::memory::stage_pwrite_i64_repeat(self, offset, value, count),
        }
    }

    // ---- the graph surface: LocalIO nodes form the local filesystem tree ----

    type Children = LocalChildren;
    type Walk = LocalWalk;

    fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn parent(&self) -> Option<LocalIO> {
        self.path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(LocalIO::from_path)
    }

    /// The child node at `segment`, its address composed by joining onto this node's URI
    /// ([`Uri::joinpath`](crate::uri::Uri::joinpath)) — so `child.parent()` addresses this
    /// node again. Lazy: pure address algebra, nothing on disk is touched until the child is
    /// read or written. Infallible for a local node (a locally-joined URI always resolves
    /// back to a path); the ergonomic [`join_str`](LocalIO::join_str) unwraps it.
    fn join(&self, segment: &str) -> Result<LocalIO, IoError> {
        let child = self.uri().joinpath(segment);
        Ok(LocalIO::from_uri(&child)
            .unwrap_or_else(|_| LocalIO::from_path(self.path.join(segment))))
    }

    fn ls(&self) -> Result<LocalChildren, IoError> {
        match fs::read_dir(&self.path) {
            Ok(read_dir) => Ok(LocalChildren {
                read_dir: Some((self.path.clone(), read_dir)),
            }),
            Err(_) if !self.is_dir() => Ok(LocalChildren { read_dir: None }),
            Err(e) => Err(file_err("list", &self.path, &e)),
        }
    }

    /// The name-sorted memory-tree blocks, **excluding symlinked directories** — so the
    /// tree recursion (`byte_size` / `pread` / `pwrite` over a directory) is acyclic: a
    /// directory symlink pointing at an ancestor can never make it recurse forever. A
    /// symlink to a regular file stays a normal block; the discovery surface (`ls` /
    /// `ls_recursive`) is unaffected — it still lists everything.
    fn blocks(&self) -> Vec<LocalIO> {
        let mut blocks: Vec<LocalIO> = match self.ls() {
            Ok(children) => children
                .filter_map(Result::ok)
                .filter(|child| {
                    match fs::symlink_metadata(&child.path) {
                        // A symlink whose target is a directory would recurse — skip it.
                        Ok(meta) if meta.file_type().is_symlink() => !fs::metadata(&child.path)
                            .map(|t| t.is_dir())
                            .unwrap_or(false),
                        _ => true,
                    }
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        blocks.sort_by_key(IOBase::name);
        blocks
    }

    fn ls_recursive(&self) -> Result<LocalWalk, IoError> {
        match fs::read_dir(&self.path) {
            Ok(read_dir) => Ok(LocalWalk {
                stack: vec![(self.path.clone(), read_dir)],
                pending: None,
            }),
            Err(_) if !self.is_dir() => Ok(LocalWalk {
                stack: Vec::new(),
                pending: None,
            }),
            Err(e) => Err(file_err("list", &self.path, &e)),
        }
    }

    fn rm(&self, exist_ok: bool) -> Result<(), IoError> {
        match self.kind() {
            IOKind::Directory => {
                fs::remove_dir_all(&self.path).map_err(|e| file_err("remove", &self.path, &e))
            }
            IOKind::Missing => self.missing_ok(exist_ok),
            _ => fs::remove_file(&self.path).map_err(|e| file_err("remove", &self.path, &e)),
        }
    }

    fn rmfile(&self, exist_ok: bool) -> Result<(), IoError> {
        match self.kind() {
            IOKind::Directory => Err(IoError::FileIo {
                op: "remove",
                path: self.path.to_string_lossy().into_owned(),
                detail: "the node is a directory; use rmdir (recursive) instead of rmfile"
                    .to_string(),
            }),
            IOKind::Missing => self.missing_ok(exist_ok),
            _ => fs::remove_file(&self.path).map_err(|e| file_err("remove", &self.path, &e)),
        }
    }

    fn rmdir(&self, exist_ok: bool) -> Result<(), IoError> {
        match self.kind() {
            IOKind::File => Err(IoError::FileIo {
                op: "remove",
                path: self.path.to_string_lossy().into_owned(),
                detail: "the node is a file; use rmfile instead of rmdir".to_string(),
            }),
            IOKind::Missing => self.missing_ok(exist_ok),
            _ => fs::remove_dir_all(&self.path).map_err(|e| file_err("remove", &self.path, &e)),
        }
    }
}

/// The streamed one-level child iterator of a [`LocalIO`] — lazy: entries are produced as
/// the caller pulls. A file or missing node streams nothing.
pub struct LocalChildren {
    read_dir: Option<(PathBuf, fs::ReadDir)>,
}

impl Iterator for LocalChildren {
    type Item = Result<LocalIO, IoError>;

    fn next(&mut self) -> Option<Self::Item> {
        let (path, read_dir) = self.read_dir.as_mut()?;
        let entry = read_dir.next()?;
        Some(match entry {
            Ok(e) => Ok(LocalIO::from_path(e.path())),
            Err(e) => Err(file_err("list", path, &e)),
        })
    }
}

/// The streamed depth-first recursive walker of a [`LocalIO`] subtree. Directory symlinks
/// are yielded but not descended into (a link cycle must not walk forever), and a subtree
/// that cannot be opened yields its guided error right after the directory's own entry.
pub struct LocalWalk {
    stack: Vec<(PathBuf, fs::ReadDir)>,
    pending: Option<IoError>,
}

impl Iterator for LocalWalk {
    type Item = Result<LocalIO, IoError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(err) = self.pending.take() {
            return Some(Err(err));
        }
        loop {
            let (dir_path, read_dir) = self.stack.last_mut()?;
            match read_dir.next() {
                Some(Ok(entry)) => {
                    let path = entry.path();
                    // `file_type` does not follow symlinks: descend only into real
                    // directories so a link cycle cannot make the walk endless.
                    let is_real_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    if is_real_dir {
                        match fs::read_dir(&path) {
                            Ok(inner) => self.stack.push((path.clone(), inner)),
                            Err(e) => self.pending = Some(file_err("list", &path, &e)),
                        }
                    }
                    return Some(Ok(LocalIO::from_path(path)));
                }
                Some(Err(e)) => {
                    let err = file_err("list", dir_path, &e);
                    return Some(Err(err));
                }
                None => {
                    self.stack.pop();
                }
            }
        }
    }
}
