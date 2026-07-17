//! The `yggdryl.local` namespace — the local-filesystem family: the lazy [`LocalIO`] single
//! access point and the raw memory-mapped [`Mmap`] it builds on.
//!
//! Mirrors `yggdryl_core::io::local`. [`LocalIO`] is one **lazy** handle over any path (file,
//! folder, or nothing yet) that decides per call how to serve I/O: constructing / probing /
//! navigating touches nothing, a read before any write opens the file ad hoc (a missing
//! node reads as empty), and the first write auto-creates the missing parent
//! folders + the file, memory-maps it, and keeps the mapping (`isMapped`) so later access
//! runs at memory speed. `close()` releases the mapping and the handle **stays usable** —
//! back to its lazy state. The same handle carries the filesystem graph — the `IOBase`
//! graph surface, mirrored from the core: `name` /
//! `parent()` / `join`, the **streamed** `ls(recursive?)` (a [`LocalEntries`] iterable) with
//! the collected `children()` convenience, and the shape-checked `rm()` / `rmfile()` /
//! `rmdir()` plus `mkdir()`.
//!
//! A `LocalIO` **directory node is a memory tree**: its `byteSize()` is the lazy streamed
//! sum of its subtree, and the ordinary byte surface (`pread*` / `pwrite*`) reads and
//! writes across its **name-sorted child file blocks** as one contiguous region (child
//! directories recurse; a middle block never grows — only the last block absorbs bytes
//! past the end, and a write into an **empty** directory throws the guided fix naming
//! `join a file name onto this directory`). DESIGN: the core's generic `tree_byte_size` /
//! `blocks` / `tree_pread_byte_array` / `tree_pwrite_byte_array` helpers are deliberately
//! **not** mirrored as named methods — they are the internal pattern behind that routing,
//! and the binding reaches the behavior through the ordinary byte surface on a directory
//! node.
//!
//! [`Mmap`] is the raw memory-mapped file `LocalIO` builds on (usable directly when a
//! pre-existing file and explicit control are wanted); it moved here from `yggdryl.memory`
//! with the core. It is a **leaf** of the IO graph — `name` is its file name, `ls()` /
//! `children()` stream and collect nothing, and `rm()` / `rmfile()` really unlink the
//! mapped file (`rmdir()` gives the guided file error). Every method on both classes is a
//! thin one- or two-line delegation to
//! `yggdryl_core` with no logic in the binding; the numeric idioms (byte-offset and length
//! **parameters** as `u32`, **returned** sizes / capacities / positions as `i64` — a JS
//! number, exact to 2^53 — and bit offsets as `i64` in both directions) match the
//! `yggdryl.memory` classes, and every failing operation surfaces as a thrown `Error`
//! carrying the core's guided text unchanged.

use napi::bindgen_prelude::{Buffer, Either, Generator, JsError, ToNapiValue, Unknown};
use napi_derive::napi;

use crate::headers::Headers;
use crate::io::kind::IOKind;
use crate::io::memory::{check_bulk_read, to_bit_offset, to_error, NoChildren, Whence};
use crate::io::mode::IOMode;
use crate::uri::Uri;
use yggdryl_core::io::local as core;
use yggdryl_core::io::memory::IOBase;
use yggdryl_core::io::IoError;

/// The one local-filesystem handle — a **lazy** node over any path (file, folder, or nothing
/// yet) that decides itself, per call, how to serve reads and writes. Mirrors
/// `yggdryl_core::io::local::LocalIO`:
///
/// - **Constructing / probing / navigating touches nothing.** `kind` / `exists()` /
///   `isFile()` / `isDir()` ask the disk per call; `join` / `parent()` are pure path math.
/// - **Reads pick their own path.** Before any write, a read opens the file ad hoc with one
///   positioned OS read (a missing node reads as empty; a directory reads as its memory
///   tree — see below). After the handle has written, reads are served from its
///   memory-mapped backing.
/// - **Writes auto-create and self-optimize.** The first write creates the missing parent
///   folders and the file, memory-maps it, and keeps the mapping (`isMapped` turns `true`).
/// - **The graph is the same handle.** Navigation, streamed discovery (`ls`, a
///   [`LocalEntries`] iterable) with the collected `children()`, and CRUD (`rm` / `rmfile` /
///   `rmdir` / `mkdir`) all live here.
/// - **A directory is a memory tree.** A directory node serves the *byte* contract too:
///   `byteSize()` is the lazy streamed sum of its subtree, and `pread*` / `pwrite*` route
///   across its name-sorted child file blocks as one contiguous region (child directories
///   recurse; a middle block never grows — only the last block absorbs bytes past the end).
///
/// `close()` releases the mapped backing eagerly (truncating the file to its logical
/// length) — unlike [`Mmap.close`](Mmap::close) the handle is **still usable** afterwards:
/// it simply returns to its lazy state. `copy()` yields a fresh lazy handle to the same
/// path. A `LocalIO` is a **live handle, not a value**: it compares by path (`equals`) and
/// deliberately has no `hashCode` / `serializeBytes`.
#[napi(js_name = "LocalIO", namespace = "local")]
pub struct LocalIO {
    pub(crate) inner: core::LocalIO,
}

#[napi(namespace = "local")]
impl LocalIO {
    /// A **lazy** handle for the addressed path — nothing is touched or created. The
    /// generic, type-inferring entry: a **string** dispatches to the core `from_path`, a
    /// [`Uri`] (`file://…` or a plain-path URI) to `from_uri` — the latter throws the core's
    /// guided `Error` on an unsupported scheme or an empty path.
    #[napi(constructor)]
    pub fn new(source: Either<String, &Uri>) -> napi::Result<Self> {
        match source {
            Either::A(path) => Ok(Self {
                inner: core::LocalIO::from_path(&path),
            }),
            Either::B(uri) => core::LocalIO::from_uri(&uri.inner)
                .map(|inner| Self { inner })
                .map_err(to_error),
        }
    }

    // ---- lifecycle: path / mkdir / flush / close ---------------------------------------

    /// The underlying filesystem path.
    #[napi(getter)]
    pub fn path(&self) -> String {
        self.inner.as_std_path().to_string_lossy().into_owned()
    }

    /// Whether the handle currently holds its optimized memory-mapped backing (it does from
    /// the first write until `close()`).
    #[napi(getter)]
    pub fn is_mapped(&self) -> bool {
        self.inner.is_mapped()
    }

    /// Auto-creates the directory tree at this path (like `mkdir -p`) — the explicit form
    /// when a **folder** itself is the goal (file-bound writes auto-create their parents on
    /// their own).
    #[napi]
    pub fn mkdir(&self) -> napi::Result<()> {
        self.inner.mkdir().map_err(to_error)
    }

    /// Flushes the mapped backing (if the handle has one) to disk — a no-op before the
    /// first write (ad-hoc reads/writes go straight to the OS).
    #[napi]
    pub fn flush(&self) -> napi::Result<()> {
        self.inner.flush().map_err(to_error)
    }

    /// Releases the mapped backing eagerly (truncating the file to its logical length) —
    /// after which the handle is **still usable**: it simply returns to its lazy state.
    /// Idempotent. Call before removing a file this handle has written (on Windows a mapped
    /// file cannot be deleted).
    #[napi]
    pub fn close(&mut self) {
        self.inner.close();
    }

    // ---- predicates (probes, not states) -----------------------------------------------

    /// Whether this node is a regular **file** — derived from `kind`; asks the disk per call.
    #[napi]
    pub fn is_file(&self) -> bool {
        self.inner.is_file()
    }

    /// Whether this node is a **directory** — derived from `kind`; asks the disk per call.
    #[napi]
    pub fn is_dir(&self) -> bool {
        self.inner.is_dir()
    }

    /// Whether something **exists** at this path — `isFile() || isDir()`; asks the disk per
    /// call.
    #[napi]
    pub fn exists(&self) -> bool {
        self.inner.exists()
    }

    // ---- size + capacity ---------------------------------------------------------------

    /// The length in bytes — the mapped logical length once mapped, the on-disk file size
    /// before any write, `0` for a missing node; a **directory** reports its memory-tree
    /// size, the lazy streamed sum of its whole subtree (recomputed live per call). An
    /// `i64` (a JS number, exact to 2^53) so a size past `u32::MAX` never wraps.
    #[napi]
    pub fn byte_size(&self) -> i64 {
        self.inner.byte_size() as i64
    }

    /// The total length in bits — `byteSize * 8`, an `i64` (a JS number, exact to 2^53)
    /// like `Heap.bitSize`, because a file past 512 MiB already has bit indexes above
    /// `u32::MAX`.
    #[napi]
    pub fn bit_size(&self) -> i64 {
        self.inner.bit_size() as i64
    }

    /// Whether the node is empty (`byteSize == 0`).
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The capacity in bytes — the mapped (file) extent once the handle has written, else
    /// simply `byteSize`. An `i64` (a JS number, exact to 2^53).
    #[napi]
    pub fn capacity(&self) -> i64 {
        self.inner.capacity() as i64
    }

    /// The spare room already available — `capacity - byteSize`. An `i64` (JS number) like
    /// `capacity`.
    #[napi]
    pub fn spare_capacity(&self) -> i64 {
        self.inner.spare_capacity() as i64
    }

    /// Reserves capacity for at least `additional` more bytes past the current `byteSize`
    /// (materializing the mapped backing if needed). Best-effort — prefer `tryReserve` to
    /// see failures.
    #[napi]
    pub fn reserve(&mut self, additional: u32) {
        self.inner.reserve(additional as u64);
    }

    /// Reserves capacity for **exactly** `additional` more bytes — no amortized
    /// over-allocation, for a caller that knows the final size.
    #[napi]
    pub fn reserve_exact(&mut self, additional: u32) {
        self.inner.reserve_exact(additional as u64);
    }

    /// **Checked** reservation: throws a guided `Error` when the backing cannot be
    /// materialized or grown (e.g. the node is a directory) instead of silently doing
    /// nothing.
    #[napi]
    pub fn try_reserve(&mut self, additional: i64) -> napi::Result<()> {
        let additional = u64::try_from(additional).unwrap_or(u64::MAX);
        self.inner.try_reserve(additional).map_err(to_error)
    }

    /// **Checked exact** reservation — `tryReserve` without the amortized over-allocation.
    #[napi]
    pub fn try_reserve_exact(&mut self, additional: i64) -> napi::Result<()> {
        let additional = u64::try_from(additional).unwrap_or(u64::MAX);
        self.inner.try_reserve_exact(additional).map_err(to_error)
    }

    /// Ensures the **total** capacity is at least `total` bytes (the absolute-target form of
    /// `reserve`); a no-op when already satisfied, never shrinks.
    #[napi]
    pub fn ensure_capacity(&mut self, total: u32) {
        self.inner.ensure_capacity(total as u64);
    }

    /// **Checked** `ensureCapacity` — throws a guided `Error` when the backing cannot grow.
    #[napi]
    pub fn try_ensure_capacity(&mut self, total: i64) -> napi::Result<()> {
        let total = u64::try_from(total).unwrap_or(u64::MAX);
        self.inner.try_ensure_capacity(total).map_err(to_error)
    }

    /// Releases spare capacity, truncating the mapped backing toward `byteSize` (a no-op on
    /// a lazy handle).
    #[napi]
    pub fn shrink_to_fit(&mut self) {
        self.inner.shrink_to_fit();
    }

    /// Shrinks the mapped extent toward `minCapacity` (never below `byteSize`; a no-op on a
    /// lazy handle).
    #[napi]
    pub fn shrink_to(&mut self, min_capacity: u32) {
        self.inner.shrink_to(min_capacity as u64);
    }

    // ---- byte-array primitives ---------------------------------------------------------

    /// Reads up to `length` bytes at `offset` into a fresh `Buffer` — short (or empty) near
    /// the end, empty on a missing node. A **directory** reads as its memory tree: the
    /// name-sorted child file blocks stitched into one contiguous region (child directories
    /// recurse). Never moves the cursor.
    #[napi]
    pub fn pread_byte_array(&self, offset: u32, length: u32) -> Buffer {
        self.inner.pread_vec(offset as u64, length as usize).into()
    }

    /// Writes `data` at `offset`, auto-creating parents + the file on the first write and
    /// keeping the mapped backing; returns the number of bytes written (`0` on a read-only
    /// handle). A **directory** routes the write across its memory-tree blocks: a write
    /// inside a block stays capped at that block's end (a middle block never grows), bytes
    /// past the end grow the **last** block, and an empty directory writes nothing (the
    /// full/typed writes report the guided fix). Never moves the cursor.
    #[napi]
    pub fn pwrite_byte_array(&mut self, offset: u32, data: Buffer) -> u32 {
        self.inner.pwrite_byte_array(offset as u64, data.as_ref()) as u32
    }

    // ---- typed positioned accessors: byte / bit / i32 / i64 ----------------------------

    /// Reads the single byte at `offset`, or throws if it is past the end.
    #[napi]
    pub fn pread_byte(&self, offset: u32) -> napi::Result<u8> {
        self.inner.pread_byte(offset as u64).map_err(to_error)
    }

    /// Writes the single byte `value` at `offset`, growing the file as needed — or throws
    /// the guided `Error` on a read-only handle (or an empty directory, which has no block
    /// to grow into).
    #[napi]
    pub fn pwrite_byte(&mut self, offset: u32, value: u8) -> napi::Result<()> {
        self.inner
            .pwrite_byte(offset as u64, value)
            .map_err(to_error)
    }

    /// Reads the bit at absolute **bit** `offset` (LSB-first: bit `offset % 8` of byte
    /// `offset / 8`), or throws if its byte is past the end. The offset is an `i64` (exact
    /// to 2^53) so every bit of a file beyond 512 MiB stays addressable; a negative offset
    /// throws.
    #[napi]
    pub fn pread_bit(&self, offset: i64) -> napi::Result<bool> {
        self.inner
            .pread_bit(to_bit_offset(offset)?)
            .map_err(to_error)
    }

    /// Sets or clears the bit at absolute **bit** `offset` (LSB-first), read-modify-writing
    /// its byte and growing the file (zero-filled) if the bit is past the end. The offset is
    /// an `i64` (exact to 2^53); a negative offset throws.
    #[napi]
    pub fn pwrite_bit(&mut self, offset: i64, value: bool) -> napi::Result<()> {
        let offset = to_bit_offset(offset)?;
        self.inner.pwrite_bit(offset, value).map_err(to_error)
    }

    /// Reads a little-endian `i32` (4 bytes) at `offset`, or throws if fewer bytes remain.
    #[napi]
    pub fn pread_i32(&self, offset: u32) -> napi::Result<i32> {
        self.inner.pread_i32(offset as u64).map_err(to_error)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at `offset`, growing as needed.
    #[napi]
    pub fn pwrite_i32(&mut self, offset: u32, value: i32) -> napi::Result<()> {
        self.inner
            .pwrite_i32(offset as u64, value)
            .map_err(to_error)
    }

    /// Reads a little-endian `i64` (8 bytes) at `offset`, or throws if fewer bytes remain.
    /// The returned JS `number` is exact only up to ±2^53.
    #[napi]
    pub fn pread_i64(&self, offset: u32) -> napi::Result<i64> {
        self.inner.pread_i64(offset as u64).map_err(to_error)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at `offset`, growing as needed.
    /// Keep `value` below ±2^53 so the JS `number` stays exact.
    #[napi]
    pub fn pwrite_i64(&mut self, offset: u32, value: i64) -> napi::Result<()> {
        self.inner
            .pwrite_i64(offset as u64, value)
            .map_err(to_error)
    }

    // ---- utf8 text ---------------------------------------------------------------------

    /// Reads up to `length` **bytes** at `offset` and decodes them as UTF-8 text (clamped
    /// near the end), or throws a guided `Error` on invalid UTF-8 — including a multi-byte
    /// character cut by the range.
    #[napi]
    pub fn pread_utf8(&self, offset: u32, length: u32) -> napi::Result<String> {
        self.inner
            .pread_utf8(offset as u64, length as usize)
            .map_err(to_error)
    }

    /// Writes `text`'s UTF-8 bytes at `offset` (auto-creating + growing as needed); returns
    /// the number of **bytes** written (not characters — `0` on a read-only handle or an
    /// empty directory).
    #[napi]
    pub fn pwrite_utf8(&mut self, offset: u32, text: String) -> u32 {
        self.inner.pwrite_utf8(offset as u64, &text) as u32
    }

    // ---- bulk typed arrays -------------------------------------------------------------

    /// **Bulk typed read** of `count` little-endian `i32`s at `offset` into a fresh array,
    /// or throws if fewer bytes remain — checked **before** the result array is allocated,
    /// so a hostile `count` fails fast instead of allocating.
    #[napi]
    pub fn pread_i32_array(&self, offset: u32, count: u32) -> napi::Result<Vec<i32>> {
        check_bulk_read(self.inner.byte_size(), offset, count, 4)?;
        let mut values = vec![0i32; count as usize];
        self.inner
            .pread_i32_array(offset as u64, &mut values)
            .map_err(to_error)?;
        Ok(values)
    }

    /// **Bulk typed write** of all of `values` as little-endian `i32`s at `offset`, growing
    /// as needed.
    #[napi]
    pub fn pwrite_i32_array(&mut self, offset: u32, values: Vec<i32>) -> napi::Result<()> {
        self.inner
            .pwrite_i32_array(offset as u64, &values)
            .map_err(to_error)
    }

    /// **Bulk typed read** of `count` little-endian `i64`s at `offset` into a fresh array,
    /// or throws if fewer bytes remain — checked **before** the result array is allocated,
    /// so a hostile `count` fails fast instead of allocating. Each JS `number` is exact only
    /// up to ±2^53.
    #[napi]
    pub fn pread_i64_array(&self, offset: u32, count: u32) -> napi::Result<Vec<i64>> {
        check_bulk_read(self.inner.byte_size(), offset, count, 8)?;
        let mut values = vec![0i64; count as usize];
        self.inner
            .pread_i64_array(offset as u64, &mut values)
            .map_err(to_error)?;
        Ok(values)
    }

    /// **Bulk typed write** of all of `values` as little-endian `i64`s at `offset`, growing
    /// as needed. Keep each value below ±2^53 so the JS `number`s stay exact.
    #[napi]
    pub fn pwrite_i64_array(&mut self, offset: u32, values: Vec<i64>) -> napi::Result<()> {
        self.inner
            .pwrite_i64_array(offset as u64, &values)
            .map_err(to_error)
    }

    // ---- repeated-value fills ----------------------------------------------------------

    /// **Repeated-value fill.** Writes `count` copies of the byte `value` starting at
    /// `offset` (growing as needed) — the byte-level `memset`; no full array is ever
    /// materialized.
    #[napi]
    pub fn pwrite_byte_repeat(&mut self, offset: u32, value: u8, count: u32) -> napi::Result<()> {
        self.inner
            .pwrite_byte_repeat(offset as u64, value, count as usize)
            .map_err(to_error)
    }

    /// **Repeated-value fill** of `count` little-endian `i32` copies of `value` at `offset`
    /// — no full array is ever materialized.
    #[napi]
    pub fn pwrite_i32_repeat(&mut self, offset: u32, value: i32, count: u32) -> napi::Result<()> {
        self.inner
            .pwrite_i32_repeat(offset as u64, value, count as usize)
            .map_err(to_error)
    }

    /// **Repeated-value fill** of `count` little-endian `i64` copies of `value` at `offset`
    /// — no full array is ever materialized. Keep `value` below ±2^53 so it stays exact.
    #[napi]
    pub fn pwrite_i64_repeat(&mut self, offset: u32, value: i64, count: u32) -> napi::Result<()> {
        self.inner
            .pwrite_i64_repeat(offset as u64, value, count as usize)
            .map_err(to_error)
    }

    // ---- cursor: position / seek -------------------------------------------------------

    /// The current cursor position (bytes from the start) — an `i64` (exact to 2^53), so a
    /// position past `u32::MAX` (a seek can land anywhere) never wraps. May sit past the
    /// end.
    #[napi(getter)]
    pub fn position(&self) -> i64 {
        self.inner.position() as i64
    }

    /// Moves the cursor to an absolute `position` (past the end is allowed).
    #[napi]
    pub fn set_position(&mut self, position: u32) {
        self.inner.set_position(position as u64);
    }

    /// Seeks to `whence + offset` and returns the new position (an `i64`, exact to 2^53). A
    /// position past the end is allowed; seeking before the start throws a guided `Error`.
    #[napi]
    pub fn seek(&mut self, whence: Whence, offset: i64) -> napi::Result<i64> {
        self.inner
            .seek(whence.into(), offset)
            .map(|position| position as i64)
            .map_err(to_error)
    }

    /// Resets the cursor to the start.
    #[napi]
    pub fn rewind(&mut self) {
        self.inner.rewind();
    }

    // ---- cursor: stream read / write ---------------------------------------------------

    /// Reads up to `length` bytes from the current position into a fresh `Buffer`, advancing
    /// the cursor by the number read (short near the end).
    #[napi]
    pub fn read(&mut self, length: u32) -> Buffer {
        self.inner.read_vec(length as usize).into()
    }

    /// Writes `data` at the current position, advancing the cursor by the number written
    /// (auto-creating + growing as needed); returns that count.
    #[napi]
    pub fn write(&mut self, data: Buffer) -> u32 {
        self.inner.write(data.as_ref()) as u32
    }

    /// Reads the next byte at the cursor, advancing it by 1, or throws at the end.
    #[napi]
    pub fn read_byte(&mut self) -> napi::Result<u8> {
        self.inner.read_byte().map_err(to_error)
    }

    /// Writes the byte `value` at the cursor, advancing it by 1.
    #[napi]
    pub fn write_byte(&mut self, value: u8) -> napi::Result<()> {
        self.inner.write_byte(value).map_err(to_error)
    }

    /// Reads a little-endian `i32` (4 bytes) at the cursor, advancing it by 4, or throws.
    #[napi]
    pub fn read_i32(&mut self) -> napi::Result<i32> {
        self.inner.read_i32().map_err(to_error)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at the cursor, advancing it by 4.
    #[napi]
    pub fn write_i32(&mut self, value: i32) -> napi::Result<()> {
        self.inner.write_i32(value).map_err(to_error)
    }

    /// Reads a little-endian `i64` (8 bytes) at the cursor, advancing it by 8, or throws.
    /// The returned JS `number` is exact only up to ±2^53.
    #[napi]
    pub fn read_i64(&mut self) -> napi::Result<i64> {
        self.inner.read_i64().map_err(to_error)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at the cursor, advancing it by 8.
    /// Keep `value` below ±2^53 so the JS `number` stays exact.
    #[napi]
    pub fn write_i64(&mut self, value: i64) -> napi::Result<()> {
        self.inner.write_i64(value).map_err(to_error)
    }

    /// Reads from the current position **to the end** into a fresh `Buffer`, advancing the
    /// cursor to the end.
    #[napi]
    pub fn read_to_end(&mut self) -> Buffer {
        self.inner.read_to_end().into()
    }

    /// Reads up to `length` **bytes** from the cursor and decodes them as UTF-8 text
    /// (clamped near the end), advancing the cursor by the bytes read, or throws on invalid
    /// UTF-8 (leaving the cursor put).
    #[napi]
    pub fn read_utf8(&mut self, length: u32) -> napi::Result<String> {
        self.inner.read_utf8(length as usize).map_err(to_error)
    }

    /// Writes `text`'s UTF-8 bytes at the cursor, advancing it; returns the number of
    /// **bytes** written (not characters).
    #[napi]
    pub fn write_utf8(&mut self, text: String) -> u32 {
        self.inner.write_utf8(&text) as u32
    }

    // ---- address (uri) -----------------------------------------------------------------

    /// The [`Uri`] that addresses this node — its filesystem path as a scheme-less,
    /// POSIX-slash URI (the exact input the constructor accepts back).
    #[napi(getter)]
    pub fn uri(&self) -> Uri {
        Uri {
            inner: self.inner.uri(),
        }
    }

    // ---- metadata (headers / mode / kind) ----------------------------------------------

    /// The metadata [`Headers`] attached to this handle — **a copy**: edits to the returned
    /// map do not write back. Call `setHeaders` to store an updated map.
    #[napi(getter)]
    pub fn headers(&self) -> Headers {
        Headers {
            inner: self.inner.headers().clone(),
        }
    }

    /// Replaces the whole [`Headers`] metadata map in place.
    #[napi]
    pub fn set_headers(&mut self, headers: &Headers) {
        *self.inner.headers_mut() = headers.inner.clone();
    }

    /// How this handle may be accessed — see [`IOMode`] (`ReadWrite` by default; writes
    /// check it before touching the disk).
    #[napi(getter)]
    pub fn mode(&self) -> IOMode {
        self.inner.mode().into()
    }

    /// Sets the access [`IOMode`] label in place (writes check it before touching the disk).
    #[napi]
    pub fn set_mode(&mut self, mode: IOMode) {
        self.inner.set_mode(mode.into());
    }

    /// What this node **is right now** — [`IOKind.File`], [`IOKind.Directory`], or
    /// [`IOKind.Missing`]; a probe per call, not a stored state.
    #[napi(getter)]
    pub fn kind(&self) -> IOKind {
        self.inner.kind().into()
    }

    // ---- the filesystem graph (the IOBase graph surface) -------------------------------

    /// The last path segment — the node's own name (empty for a root).
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name()
    }

    /// The parent node, or `null` at a filesystem root — a fresh **lazy** handle; pure path
    /// math, nothing is touched.
    #[napi]
    pub fn parent(&self) -> Option<LocalIO> {
        self.inner.parent().map(|inner| LocalIO { inner })
    }

    /// The child node at `segment` (which may be a multi-segment relative path like
    /// `"a/b/c.txt"`) — a fresh **lazy** handle; nothing is touched or created.
    #[napi]
    pub fn join(&self, segment: String) -> LocalIO {
        LocalIO {
            inner: self.inner.join_str(&segment),
        }
    }

    /// **Streams** the node's children as a [`LocalEntries`] iterable of lazy handles —
    /// the direct children by default, the **entire subtree** (depth-first) with
    /// `recursive: true`. Entries are produced as the caller pulls (`for (const entry of
    /// node.ls())`) — never a pre-collected tree; use `children()` for the collected
    /// direct-children convenience. A file or missing node streams nothing. Throws a guided
    /// `Error` when the directory cannot be listed — up front for the node itself, or from
    /// the yielding step for an entry inside the walk.
    #[napi]
    pub fn ls(&self, recursive: Option<bool>) -> napi::Result<LocalEntries> {
        let inner = if recursive.unwrap_or(false) {
            Entries::Walk(self.inner.ls_recursive().map_err(to_error)?)
        } else {
            Entries::Children(Box::new(self.inner.ls().map_err(to_error)?))
        };
        Ok(LocalEntries { inner })
    }

    /// The direct children as an array of lazy handles — the collected convenience over
    /// the streaming `ls()`.
    #[napi]
    pub fn children(&self) -> napi::Result<Vec<LocalIO>> {
        self.inner
            .children()
            .map(|nodes| nodes.into_iter().map(|inner| LocalIO { inner }).collect())
            .map_err(to_error)
    }

    /// Removes **whatever exists** at this node — a file is unlinked, a directory is
    /// removed with its whole subtree; a missing node is a no-op. The generic form of
    /// `rmfile` / `rmdir`.
    #[napi]
    pub fn rm(&self) -> napi::Result<()> {
        self.inner.rm().map_err(to_error)
    }

    /// Removes this node **as a file** — throws the guided `Error` when the node is a
    /// directory (use `rmdir`) and is a no-op when missing.
    #[napi]
    pub fn rmfile(&self) -> napi::Result<()> {
        self.inner.rmfile().map_err(to_error)
    }

    /// Removes this node **as a directory**, recursively — throws the guided `Error` when
    /// the node is a file (use `rmfile`) and is a no-op when missing.
    #[napi]
    pub fn rmdir(&self) -> napi::Result<()> {
        self.inner.rmdir().map_err(to_error)
    }

    // ---- live-handle value mirrors -----------------------------------------------------

    /// Path identity — two handles are equal iff they address the same path (the mapped
    /// state, cursor, and metadata are transient).
    #[napi]
    pub fn equals(&self, other: &LocalIO) -> bool {
        self.inner == other.inner
    }

    /// A fresh **lazy** handle to the same path — the mapped backing is deliberately not
    /// shared (two live mappings of one file would alias).
    #[napi]
    pub fn copy(&self) -> LocalIO {
        LocalIO {
            inner: self.inner.clone(),
        }
    }

    /// A short debug string of the form `LocalIO(<path>, <N bytes>)`.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "LocalIO({}, {} bytes)",
            self.inner.as_std_path().display(),
            self.inner.byte_size()
        )
    }
}

/// The core streamed iterator a [`LocalEntries`] wraps — one level
/// (`yggdryl_core::io::local::LocalChildren`) or the depth-first subtree walk
/// (`yggdryl_core::io::local::LocalWalk`); both are owned iterators, so the binding holds
/// them directly (the one-level iterator boxed — its OS `ReadDir` state dwarfs the walk's).
enum Entries {
    Children(Box<core::LocalChildren>),
    Walk(core::LocalWalk),
}

/// The yielded-item wrapper for [`LocalEntries`]. The module exists so the wrapper's type
/// name is `LocalIO` — napi-rs derives the generated `.d.ts` yield type from the **last path
/// segment** of `Generator::Yield`, and what an `Ok` entry converts into really is a
/// [`LocalIO`](super::LocalIO) class instance; an `Err` entry never becomes a value at all
/// (it throws instead).
mod entry {
    /// One streamed `ls` entry — `Ok` converts to a [`super::LocalIO`] instance, `Err`
    /// throws the core's guided text.
    pub struct LocalIO(pub Result<yggdryl_core::io::local::LocalIO, yggdryl_core::io::IoError>);
}

impl ToNapiValue for entry::LocalIO {
    unsafe fn to_napi_value(
        env: napi::sys::napi_env,
        val: entry::LocalIO,
    ) -> napi::Result<napi::sys::napi_value> {
        match val.0 {
            Ok(inner) => unsafe { LocalIO::to_napi_value(env, LocalIO { inner }) },
            Err(error) => {
                // Throw the core's guided text through the standard (NUL-safe) error path,
                // then hand back `undefined`: the pending exception surfaces from the
                // caller's `next()`, so a failing entry throws the usual guided `Error`.
                unsafe { JsError::from(to_error(error)).throw_into(env) };
                let mut undefined = std::ptr::null_mut();
                napi::check_status!(
                    unsafe { napi::sys::napi_get_undefined(env, &mut undefined) },
                    "Failed to get undefined for a failing ls entry"
                )?;
                Ok(undefined)
            }
        }
    }
}

/// The **streaming** iterable returned by [`ls`](LocalIO::ls) — entries are produced one at
/// a time as the caller pulls (house rule: discovery is streamed, never a pre-collected
/// tree). The class is a real JS iterable (`[Symbol.iterator]`), so `for..of` and spread
/// work directly; each entry is a fresh lazy [`LocalIO`], and an entry that cannot be
/// produced throws the guided `Error` (the core text unchanged) from `next()`. The stream
/// is **one pass**: every `[Symbol.iterator]()` call continues the same underlying walk,
/// exactly like a JS generator object.
#[napi(iterator, namespace = "local")]
pub struct LocalEntries {
    inner: Entries,
}

#[napi(namespace = "local")]
impl Generator for LocalEntries {
    type Yield = entry::LocalIO;
    type Next = Unknown;
    type Return = Unknown;

    fn next(&mut self, _value: Option<Unknown>) -> Option<Self::Yield> {
        match &mut self.inner {
            Entries::Children(children) => children.next(),
            Entries::Walk(walk) => walk.next(),
        }
        .map(entry::LocalIO)
    }
}

#[napi(namespace = "local")]
impl LocalEntries {
    /// A short debug string naming the stream's shape — `LocalEntries(<children>)` or
    /// `LocalEntries(<recursive walk>)` (mirrors the Python `repr`).
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        match &self.inner {
            Entries::Children(_) => "LocalEntries(<children>)".to_string(),
            Entries::Walk(_) => "LocalEntries(<recursive walk>)".to_string(),
        }
    }
}

/// The guided error for a method called on a mapping after `close()`.
fn closed_err() -> napi::Error {
    to_error("the mapping is closed; reopen it with Mmap.open / Mmap.openReadonly / Mmap.create")
}

/// A **memory-mapped file** — the on-disk source behind the byte-access contract, sharing
/// [`Heap`](crate::io::memory::Heap)'s full surface (positioned + typed + bulk access, the
/// built-in cursor stream, capacity management, metadata) over a file instead of an owned
/// buffer.
///
/// A mapping is opened by path or [`Uri`] through the factories (`Mmap.open` /
/// `Mmap.openReadonly` / `Mmap.create` — there is no plain constructor). `byteSize` is the
/// **logical** length; `capacity` is the mapped (file) extent, which grows **amortized**
/// (doubling, page-aligned) when a write lands past the end — the same allocation curve as
/// [`Heap`](crate::io::memory::Heap). `close()` unmaps deterministically and truncates the
/// on-disk file back to the logical length; JavaScript has no deterministic drop, and on
/// Windows a mapped file cannot be deleted while a view is open, so call it as soon as the
/// mapping is done.
///
/// Unlike [`Heap`](crate::io::memory::Heap), a mapping is a **live OS resource, not a
/// value**: two independent mappings of one file would alias, so it is deliberately not
/// clonable, equatable, or serializable — there is no `equals`, `copy`, `serializeBytes`,
/// `withHeaders`, or `withMode` (use the in-place `setHeaders` / `setMode`).
#[napi(namespace = "local")]
pub struct Mmap {
    /// `None` once `close()` has run — every later use throws the guided closed error.
    inner: Option<core::Mmap>,
}

impl Mmap {
    /// The live mapping, or the guided closed error after `close()`.
    fn inner(&self) -> napi::Result<&core::Mmap> {
        self.inner.as_ref().ok_or_else(closed_err)
    }

    /// The live mapping, mutably, or the guided closed error after `close()`.
    fn inner_mut(&mut self) -> napi::Result<&mut core::Mmap> {
        self.inner.as_mut().ok_or_else(closed_err)
    }

    /// Wraps a freshly opened core mapping (an open failure throws its guided text).
    fn from_core(inner: Result<core::Mmap, IoError>) -> napi::Result<Mmap> {
        Ok(Mmap {
            inner: Some(inner.map_err(to_error)?),
        })
    }
}

#[napi(namespace = "local")]
impl Mmap {
    // ---- factories (the generic, type-inferring entries) -------------------------------

    /// Opens an **existing** file read-write — the generic entry: a **string** dispatches to
    /// the core `open_path`, a [`Uri`] (`file://…` or a plain path) to `open_uri`. Throws a
    /// guided `Error` naming the path if it is missing or inaccessible.
    #[napi(factory)]
    pub fn open(source: Either<String, &Uri>) -> napi::Result<Mmap> {
        Self::from_core(match source {
            Either::A(path) => core::Mmap::open_path(&path),
            Either::B(uri) => core::Mmap::open_uri(&uri.inner),
        })
    }

    /// Opens an **existing** file **read-only** (same string / [`Uri`] dispatch as `open`):
    /// reads work, the write primitives write nothing (count `0`), and full writes throw a
    /// guided `Error` naming the fix.
    #[napi(factory)]
    pub fn open_readonly(source: Either<String, &Uri>) -> napi::Result<Mmap> {
        Self::from_core(match source {
            Either::A(path) => core::Mmap::open_path_readonly(&path),
            Either::B(uri) => core::Mmap::open_uri_readonly(&uri.inner),
        })
    }

    /// Opens the file read-write, **creating it empty** if it does not exist — existing
    /// contents are kept, never truncated on open (same string / [`Uri`] dispatch as `open`).
    #[napi(factory)]
    pub fn create(source: Either<String, &Uri>) -> napi::Result<Mmap> {
        Self::from_core(match source {
            Either::A(path) => core::Mmap::create_path(&path),
            Either::B(uri) => core::Mmap::create_uri(&uri.inner),
        })
    }

    // ---- lifecycle: path / flush / close -----------------------------------------------

    /// The file path this mapping is backed by.
    #[napi(getter)]
    pub fn path(&self) -> napi::Result<String> {
        Ok(self.inner()?.path().to_string_lossy().into_owned())
    }

    /// Flushes the mapped bytes (and file metadata) to disk — `msync` / `FlushViewOfFile`
    /// plus an fsync. Throws a guided `Error` on OS failure.
    #[napi]
    pub fn flush(&self) -> napi::Result<()> {
        self.inner()?.flush().map_err(to_error)
    }

    /// **Closes** the mapping deterministically: unmaps the view and truncates the on-disk
    /// file back to the logical length (releasing the capacity padding) — exactly what
    /// garbage collection would eventually do, but at a moment the caller controls (on
    /// Windows a mapped file cannot be deleted while a view is open). Idempotent; after
    /// `close` every other method throws the guided closed error.
    // DESIGN: `close()` exists only on the binding — the core `Mmap` unmaps on drop, but JS
    // has no deterministic drop (a napi object frees on GC), so the binding holds the core
    // value in an `Option` and `close()` takes it, dropping (= unmapping + truncating) it
    // eagerly.
    #[napi]
    pub fn close(&mut self) {
        self.inner = None;
    }

    /// Whether [`close`](Mmap::close) has released the mapping (the file-object idiom;
    /// mirrors the Python binding's `closed`).
    #[napi(getter)]
    pub fn closed(&self) -> bool {
        self.inner.is_none()
    }

    // ---- size + capacity ---------------------------------------------------------------

    /// The **logical** length in bytes — an `i64` (a JS number, exact to 2^53) so a size past
    /// `u32::MAX` never wraps.
    #[napi]
    pub fn byte_size(&self) -> napi::Result<i64> {
        Ok(self.inner()?.byte_size() as i64)
    }

    /// The total length in bits — `byteSize * 8`, an `i64` (a JS number, exact to 2^53) like
    /// `Heap.bitSize`, because a file past 512 MiB already has bit indexes above `u32::MAX`.
    #[napi]
    pub fn bit_size(&self) -> napi::Result<i64> {
        Ok(self.inner()?.bit_size() as i64)
    }

    /// Whether the file is empty (`byteSize == 0`).
    #[napi]
    pub fn is_empty(&self) -> napi::Result<bool> {
        Ok(self.inner()?.is_empty())
    }

    /// The mapped (file) extent in bytes — the room before the next remap, like
    /// `Vec::capacity`. An `i64` (a JS number, exact to 2^53).
    #[napi]
    pub fn capacity(&self) -> napi::Result<i64> {
        Ok(self.inner()?.capacity() as i64)
    }

    /// The spare room already mapped — `capacity - byteSize`, the bytes that can be appended
    /// before the next remap. An `i64` (JS number) like `capacity`.
    #[napi]
    pub fn spare_capacity(&self) -> napi::Result<i64> {
        Ok(self.inner()?.spare_capacity() as i64)
    }

    /// Reserves capacity for at least `additional` more bytes past the current `byteSize`,
    /// amortizing later remaps. Best-effort on a file — prefer `tryReserve` to see failures.
    #[napi]
    pub fn reserve(&mut self, additional: u32) -> napi::Result<()> {
        self.inner_mut()?.reserve(additional as u64);
        Ok(())
    }

    /// Reserves capacity for **exactly** `additional` more bytes — no amortized
    /// over-allocation, for a caller that knows the final size.
    #[napi]
    pub fn reserve_exact(&mut self, additional: u32) -> napi::Result<()> {
        self.inner_mut()?.reserve_exact(additional as u64);
        Ok(())
    }

    /// **Checked** reservation: throws a guided `Error` when the file cannot grow (or the
    /// mapping is read-only) instead of silently leaving the capacity unchanged.
    #[napi]
    pub fn try_reserve(&mut self, additional: i64) -> napi::Result<()> {
        let additional = u64::try_from(additional).unwrap_or(u64::MAX);
        self.inner_mut()?.try_reserve(additional).map_err(to_error)
    }

    /// **Checked exact** reservation — `tryReserve` without the amortized over-allocation.
    #[napi]
    pub fn try_reserve_exact(&mut self, additional: i64) -> napi::Result<()> {
        let additional = u64::try_from(additional).unwrap_or(u64::MAX);
        self.inner_mut()?
            .try_reserve_exact(additional)
            .map_err(to_error)
    }

    /// Ensures the **total** capacity is at least `total` bytes (the absolute-target form of
    /// `reserve`); a no-op when already satisfied, never shrinks.
    #[napi]
    pub fn ensure_capacity(&mut self, total: u32) -> napi::Result<()> {
        self.inner_mut()?.ensure_capacity(total as u64);
        Ok(())
    }

    /// **Checked** `ensureCapacity` — throws a guided `Error` when the file cannot grow.
    #[napi]
    pub fn try_ensure_capacity(&mut self, total: i64) -> napi::Result<()> {
        let total = u64::try_from(total).unwrap_or(u64::MAX);
        self.inner_mut()?
            .try_ensure_capacity(total)
            .map_err(to_error)
    }

    /// Truncates the on-disk file back toward `byteSize`, releasing the capacity padding.
    #[napi]
    pub fn shrink_to_fit(&mut self) -> napi::Result<()> {
        self.inner_mut()?.shrink_to_fit();
        Ok(())
    }

    /// Shrinks the mapped extent toward `minCapacity` (never below `byteSize`).
    #[napi]
    pub fn shrink_to(&mut self, min_capacity: u32) -> napi::Result<()> {
        self.inner_mut()?.shrink_to(min_capacity as u64);
        Ok(())
    }

    // ---- byte-array primitives ---------------------------------------------------------

    /// Reads up to `length` bytes at `offset` into a fresh `Buffer` — short (or empty) near
    /// the end. Never moves the cursor.
    #[napi]
    pub fn pread_byte_array(&self, offset: u32, length: u32) -> napi::Result<Buffer> {
        Ok(self
            .inner()?
            .pread_vec(offset as u64, length as usize)
            .into())
    }

    /// Writes `data` at `offset`, growing the file (and zero-filling any gap) as needed;
    /// returns the number of bytes written (`data.length` — or `0` on a read-only mapping).
    /// Never moves the cursor.
    #[napi]
    pub fn pwrite_byte_array(&mut self, offset: u32, data: Buffer) -> napi::Result<u32> {
        Ok(self
            .inner_mut()?
            .pwrite_byte_array(offset as u64, data.as_ref()) as u32)
    }

    // ---- typed positioned accessors: byte / bit / i32 / i64 ----------------------------

    /// Reads the single byte at `offset`, or throws if it is past the end.
    #[napi]
    pub fn pread_byte(&self, offset: u32) -> napi::Result<u8> {
        self.inner()?.pread_byte(offset as u64).map_err(to_error)
    }

    /// Writes the single byte `value` at `offset`, growing the file as needed.
    #[napi]
    pub fn pwrite_byte(&mut self, offset: u32, value: u8) -> napi::Result<()> {
        self.inner_mut()?
            .pwrite_byte(offset as u64, value)
            .map_err(to_error)
    }

    /// Reads the bit at absolute **bit** `offset` (LSB-first: bit `offset % 8` of byte
    /// `offset / 8`), or throws if its byte is past the end. The offset is an `i64` (exact to
    /// 2^53) so every bit of a file beyond 512 MiB stays addressable; a negative offset throws.
    #[napi]
    pub fn pread_bit(&self, offset: i64) -> napi::Result<bool> {
        self.inner()?
            .pread_bit(to_bit_offset(offset)?)
            .map_err(to_error)
    }

    /// Sets or clears the bit at absolute **bit** `offset` (LSB-first), read-modify-writing
    /// its byte and growing the file (zero-filled) if the bit is past the end. The offset is
    /// an `i64` (exact to 2^53); a negative offset throws.
    #[napi]
    pub fn pwrite_bit(&mut self, offset: i64, value: bool) -> napi::Result<()> {
        let offset = to_bit_offset(offset)?;
        self.inner_mut()?
            .pwrite_bit(offset, value)
            .map_err(to_error)
    }

    /// Reads a little-endian `i32` (4 bytes) at `offset`, or throws if fewer bytes remain.
    #[napi]
    pub fn pread_i32(&self, offset: u32) -> napi::Result<i32> {
        self.inner()?.pread_i32(offset as u64).map_err(to_error)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at `offset`, growing as needed.
    #[napi]
    pub fn pwrite_i32(&mut self, offset: u32, value: i32) -> napi::Result<()> {
        self.inner_mut()?
            .pwrite_i32(offset as u64, value)
            .map_err(to_error)
    }

    /// Reads a little-endian `i64` (8 bytes) at `offset`, or throws if fewer bytes remain.
    /// The returned JS `number` is exact only up to ±2^53.
    #[napi]
    pub fn pread_i64(&self, offset: u32) -> napi::Result<i64> {
        self.inner()?.pread_i64(offset as u64).map_err(to_error)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at `offset`, growing as needed.
    /// Keep `value` below ±2^53 so the JS `number` stays exact.
    #[napi]
    pub fn pwrite_i64(&mut self, offset: u32, value: i64) -> napi::Result<()> {
        self.inner_mut()?
            .pwrite_i64(offset as u64, value)
            .map_err(to_error)
    }

    // ---- utf8 text ---------------------------------------------------------------------

    /// Reads up to `length` **bytes** at `offset` and decodes them as UTF-8 text (clamped
    /// near the end), or throws a guided `Error` on invalid UTF-8 — including a multi-byte
    /// character cut by the range.
    #[napi]
    pub fn pread_utf8(&self, offset: u32, length: u32) -> napi::Result<String> {
        self.inner()?
            .pread_utf8(offset as u64, length as usize)
            .map_err(to_error)
    }

    /// Writes `text`'s UTF-8 bytes at `offset` (growing as needed); returns the number of
    /// **bytes** written (not characters — `0` on a read-only mapping).
    #[napi]
    pub fn pwrite_utf8(&mut self, offset: u32, text: String) -> napi::Result<u32> {
        Ok(self.inner_mut()?.pwrite_utf8(offset as u64, &text) as u32)
    }

    // ---- bulk typed arrays -------------------------------------------------------------

    /// **Bulk typed read** of `count` little-endian `i32`s at `offset` into a fresh array, or
    /// throws if fewer bytes remain — checked **before** the result array is allocated, so a
    /// hostile `count` fails fast instead of allocating.
    #[napi]
    pub fn pread_i32_array(&self, offset: u32, count: u32) -> napi::Result<Vec<i32>> {
        check_bulk_read(self.inner()?.byte_size(), offset, count, 4)?;
        let mut values = vec![0i32; count as usize];
        self.inner()?
            .pread_i32_array(offset as u64, &mut values)
            .map_err(to_error)?;
        Ok(values)
    }

    /// **Bulk typed write** of all of `values` as little-endian `i32`s at `offset`, growing
    /// as needed.
    #[napi]
    pub fn pwrite_i32_array(&mut self, offset: u32, values: Vec<i32>) -> napi::Result<()> {
        self.inner_mut()?
            .pwrite_i32_array(offset as u64, &values)
            .map_err(to_error)
    }

    /// **Bulk typed read** of `count` little-endian `i64`s at `offset` into a fresh array, or
    /// throws if fewer bytes remain — checked **before** the result array is allocated, so a
    /// hostile `count` fails fast instead of allocating. Each JS `number` is exact only up to
    /// ±2^53.
    #[napi]
    pub fn pread_i64_array(&self, offset: u32, count: u32) -> napi::Result<Vec<i64>> {
        check_bulk_read(self.inner()?.byte_size(), offset, count, 8)?;
        let mut values = vec![0i64; count as usize];
        self.inner()?
            .pread_i64_array(offset as u64, &mut values)
            .map_err(to_error)?;
        Ok(values)
    }

    /// **Bulk typed write** of all of `values` as little-endian `i64`s at `offset`, growing
    /// as needed. Keep each value below ±2^53 so the JS `number`s stay exact.
    #[napi]
    pub fn pwrite_i64_array(&mut self, offset: u32, values: Vec<i64>) -> napi::Result<()> {
        self.inner_mut()?
            .pwrite_i64_array(offset as u64, &values)
            .map_err(to_error)
    }

    // ---- repeated-value fills ----------------------------------------------------------

    /// **Repeated-value fill.** Writes `count` copies of the byte `value` starting at
    /// `offset` (growing as needed) — the byte-level `memset`; no full array is ever
    /// materialized.
    #[napi]
    pub fn pwrite_byte_repeat(&mut self, offset: u32, value: u8, count: u32) -> napi::Result<()> {
        self.inner_mut()?
            .pwrite_byte_repeat(offset as u64, value, count as usize)
            .map_err(to_error)
    }

    /// **Repeated-value fill** of `count` little-endian `i32` copies of `value` at `offset` —
    /// no full array is ever materialized.
    #[napi]
    pub fn pwrite_i32_repeat(&mut self, offset: u32, value: i32, count: u32) -> napi::Result<()> {
        self.inner_mut()?
            .pwrite_i32_repeat(offset as u64, value, count as usize)
            .map_err(to_error)
    }

    /// **Repeated-value fill** of `count` little-endian `i64` copies of `value` at `offset` —
    /// no full array is ever materialized. Keep `value` below ±2^53 so it stays exact.
    #[napi]
    pub fn pwrite_i64_repeat(&mut self, offset: u32, value: i64, count: u32) -> napi::Result<()> {
        self.inner_mut()?
            .pwrite_i64_repeat(offset as u64, value, count as usize)
            .map_err(to_error)
    }

    // ---- cursor: position / seek -------------------------------------------------------

    /// The current cursor position (bytes from the start) — an `i64` (exact to 2^53), so a
    /// position past `u32::MAX` (a seek can land anywhere) never wraps. May sit past the end.
    #[napi(getter)]
    pub fn position(&self) -> napi::Result<i64> {
        Ok(self.inner()?.position() as i64)
    }

    /// Moves the cursor to an absolute `position` (past the end is allowed).
    #[napi]
    pub fn set_position(&mut self, position: u32) -> napi::Result<()> {
        self.inner_mut()?.set_position(position as u64);
        Ok(())
    }

    /// Seeks to `whence + offset` and returns the new position (an `i64`, exact to 2^53). A
    /// position past the end is allowed; seeking before the start throws a guided `Error`.
    #[napi]
    pub fn seek(&mut self, whence: Whence, offset: i64) -> napi::Result<i64> {
        self.inner_mut()?
            .seek(whence.into(), offset)
            .map(|position| position as i64)
            .map_err(to_error)
    }

    /// Resets the cursor to the start.
    #[napi]
    pub fn rewind(&mut self) -> napi::Result<()> {
        self.inner_mut()?.rewind();
        Ok(())
    }

    // ---- cursor: stream read / write ---------------------------------------------------

    /// Reads up to `length` bytes from the current position into a fresh `Buffer`, advancing
    /// the cursor by the number read (short near the end).
    #[napi]
    pub fn read(&mut self, length: u32) -> napi::Result<Buffer> {
        Ok(self.inner_mut()?.read_vec(length as usize).into())
    }

    /// Writes `data` at the current position, advancing the cursor by the number written
    /// (growing the file as needed); returns that count (`data.length` — or `0` on a
    /// read-only mapping).
    #[napi]
    pub fn write(&mut self, data: Buffer) -> napi::Result<u32> {
        Ok(self.inner_mut()?.write(data.as_ref()) as u32)
    }

    /// Reads the next byte at the cursor, advancing it by 1, or throws at the end.
    #[napi]
    pub fn read_byte(&mut self) -> napi::Result<u8> {
        self.inner_mut()?.read_byte().map_err(to_error)
    }

    /// Writes the byte `value` at the cursor, advancing it by 1.
    #[napi]
    pub fn write_byte(&mut self, value: u8) -> napi::Result<()> {
        self.inner_mut()?.write_byte(value).map_err(to_error)
    }

    /// Reads a little-endian `i32` (4 bytes) at the cursor, advancing it by 4, or throws.
    #[napi]
    pub fn read_i32(&mut self) -> napi::Result<i32> {
        self.inner_mut()?.read_i32().map_err(to_error)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at the cursor, advancing it by 4.
    #[napi]
    pub fn write_i32(&mut self, value: i32) -> napi::Result<()> {
        self.inner_mut()?.write_i32(value).map_err(to_error)
    }

    /// Reads a little-endian `i64` (8 bytes) at the cursor, advancing it by 8, or throws.
    /// The returned JS `number` is exact only up to ±2^53.
    #[napi]
    pub fn read_i64(&mut self) -> napi::Result<i64> {
        self.inner_mut()?.read_i64().map_err(to_error)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at the cursor, advancing it by 8.
    /// Keep `value` below ±2^53 so the JS `number` stays exact.
    #[napi]
    pub fn write_i64(&mut self, value: i64) -> napi::Result<()> {
        self.inner_mut()?.write_i64(value).map_err(to_error)
    }

    /// Reads from the current position **to the end** into a fresh `Buffer`, advancing the
    /// cursor to the end.
    #[napi]
    pub fn read_to_end(&mut self) -> napi::Result<Buffer> {
        Ok(self.inner_mut()?.read_to_end().into())
    }

    /// Reads up to `length` **bytes** from the cursor and decodes them as UTF-8 text (clamped
    /// near the end), advancing the cursor by the bytes read, or throws on invalid UTF-8
    /// (leaving the cursor put).
    #[napi]
    pub fn read_utf8(&mut self, length: u32) -> napi::Result<String> {
        self.inner_mut()?
            .read_utf8(length as usize)
            .map_err(to_error)
    }

    /// Writes `text`'s UTF-8 bytes at the cursor, advancing it; returns the number of
    /// **bytes** written (not characters — `0` on a read-only mapping).
    #[napi]
    pub fn write_utf8(&mut self, text: String) -> napi::Result<u32> {
        Ok(self.inner_mut()?.write_utf8(&text) as u32)
    }

    // ---- address (uri) -----------------------------------------------------------------

    /// The [`Uri`] that addresses this mapping — its file path as a scheme-less, POSIX-slash
    /// URI (the exact input `Mmap.open` accepts back).
    #[napi(getter)]
    pub fn uri(&self) -> napi::Result<Uri> {
        Ok(Uri {
            inner: self.inner()?.uri(),
        })
    }

    // ---- metadata (headers / mode / kind) ----------------------------------------------

    /// The metadata [`Headers`] attached to this mapping — **a copy**: edits to the returned
    /// map do not write back. Call `setHeaders` to store an updated map.
    #[napi(getter)]
    pub fn headers(&self) -> napi::Result<Headers> {
        Ok(Headers {
            inner: self.inner()?.headers().clone(),
        })
    }

    /// Replaces the whole [`Headers`] metadata map in place. (There is no `withHeaders` — a
    /// live mapping cannot be copied.)
    #[napi]
    pub fn set_headers(&mut self, headers: &Headers) -> napi::Result<()> {
        *self.inner_mut()?.headers_mut() = headers.inner.clone();
        Ok(())
    }

    /// How this mapping may be accessed — see [`IOMode`] (`ReadWrite` from `open` / `create`,
    /// `Read` from `openReadonly`).
    #[napi(getter)]
    pub fn mode(&self) -> napi::Result<IOMode> {
        Ok(self.inner()?.mode().into())
    }

    /// Sets the access [`IOMode`] label in place — the physical protection is fixed at open
    /// (use `openReadonly` for a truly unwritable mapping). (There is no `withMode` — a live
    /// mapping cannot be copied.)
    #[napi]
    pub fn set_mode(&mut self, mode: IOMode) -> napi::Result<()> {
        self.inner_mut()?.set_mode(mode.into());
        Ok(())
    }

    /// What this source is — always [`IOKind.File`] for a memory-mapped file.
    #[napi(getter)]
    pub fn kind(&self) -> napi::Result<IOKind> {
        Ok(self.inner()?.kind().into())
    }

    // ---- the graph surface (a mapping is a leaf with a removable file) -----------------

    /// The node's own name — the mapped **file's name** (the last path segment).
    #[napi(getter)]
    pub fn name(&self) -> napi::Result<String> {
        Ok(self.inner()?.name())
    }

    /// The parent node — always `null`: a raw mapping is a leaf of the IO graph (navigate
    /// with [`LocalIO`] when the tree itself is wanted).
    #[napi]
    pub fn parent(&self) -> napi::Result<Option<Mmap>> {
        Ok(self
            .inner()?
            .parent()
            .map(|inner| Mmap { inner: Some(inner) }))
    }

    /// Streams this node's children — always the empty [`NoChildren`] iterable: a mapped
    /// file is a **leaf** and streams nothing (`recursive` is accepted for the uniform
    /// `ls(recursive?)` shape and changes nothing on a leaf).
    #[napi]
    pub fn ls(&self, recursive: Option<bool>) -> napi::Result<NoChildren> {
        NoChildren::over(self.inner()?, recursive)
    }

    /// The direct children, collected — always an empty array (a mapped file is a leaf).
    #[napi]
    pub fn children(&self) -> napi::Result<Vec<Mmap>> {
        self.inner()?
            .children()
            .map(|nodes| {
                nodes
                    .into_iter()
                    .map(|inner| Mmap { inner: Some(inner) })
                    .collect()
            })
            .map_err(to_error)
    }

    /// Removes **whatever exists** at this node — for a mapping that is the file itself
    /// (delegates to `rmfile`); a missing file is a no-op. On Windows a file with a live
    /// mapped view cannot be deleted — `close()` this mapping's view first.
    #[napi]
    pub fn rm(&self) -> napi::Result<()> {
        self.inner()?.rm().map_err(to_error)
    }

    /// Removes this node **as a file** — really unlinks the mapped file (idempotent when
    /// already missing). On Windows a file with a live mapped view cannot be deleted —
    /// `close()` this mapping's view first.
    #[napi]
    pub fn rmfile(&self) -> napi::Result<()> {
        self.inner()?.rmfile().map_err(to_error)
    }

    /// Removes this node **as a directory** — always the guided `Error` "the node is a
    /// file; use rmfile instead of rmdir": a mapping is by construction a file.
    #[napi]
    pub fn rmdir(&self) -> napi::Result<()> {
        self.inner()?.rmdir().map_err(to_error)
    }

    // ---- predicates (isFile / isDir / exists) ------------------------------------------

    /// Whether this source is a regular **file** — always `true` for a live mapping.
    #[napi]
    pub fn is_file(&self) -> napi::Result<bool> {
        Ok(self.inner()?.is_file())
    }

    /// Whether this source is a **directory** — always `false` for a mapping.
    #[napi]
    pub fn is_dir(&self) -> napi::Result<bool> {
        Ok(self.inner()?.is_dir())
    }

    /// Whether the source **exists** — a live mapping is by construction a live file
    /// (`true`).
    #[napi]
    pub fn exists(&self) -> napi::Result<bool> {
        Ok(self.inner()?.exists())
    }

    // DESIGN: no `cursor()` / `window()` — the binding `Cursor` and `Slice` classes are
    // monomorphic over `Heap` (each owns a *copy* of its source), and a live mapping cannot
    // be copied. The built-in cursor stream above covers streaming; revisit if the binding
    // views ever become generic over sources.

    /// A short debug string of the form `Mmap(<path>, <N bytes>)` — or `Mmap(closed)` after
    /// `close()`, so string coercion never throws.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        match &self.inner {
            Some(map) => format!("Mmap({}, {} bytes)", map.path().display(), map.byte_size()),
            None => "Mmap(closed)".to_string(),
        }
    }
}
