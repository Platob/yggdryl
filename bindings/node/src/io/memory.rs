//! The `yggdryl.memory` namespace — the in-heap byte source and its seek anchor.
//!
//! Mirrors `yggdryl_core::io::memory`'s concrete source [`Heap`] (an owned byte buffer with a
//! read/write cursor and `Vec`-like capacity), plus the [`Whence`] seek anchor. Every method
//! is a thin one- or two-line delegation to `yggdryl_core` — the positioned primitives and
//! typed accessors of `IOBase`, the cursor stream of `IOCursor`, bounded `IOSlice` windows,
//! and `Whence`-relative seeks — with no logic in the binding. (The memory-mapped file source
//! moved with the core to the `yggdryl.local` namespace — see [`crate::io::local`].)
//!
//! `IOBase` is the **central access path**, so every class here also carries its graph
//! surface — as a **leaf** node for *discovery*: `name`, the always-empty streamed
//! `ls(recursive?)` (the shared [`NoChildren`] iterable) with the collected `children()`
//! (an empty array), and the `rm()` / `rmfile()` / `rmdir()` trio throwing the core's
//! guided refusal (an in-memory source has no removable backing). A [`Heap`] is still
//! **addressable**, though: `join(segment)` composes a child address (a new independent
//! buffer at `mem://heap/<segment>`) and `parent()` navigates back, so the same uniform
//! graph API works over an in-memory buffer as over a filesystem node (the [`Cursor`] and
//! [`Slice`] views stay pure leaves — `parent()` is `null`).
//!
//! Numeric idioms: byte offset and length **parameters** are JS `number`s typed as `u32`, so a
//! single heap addresses up to 4 GiB in memory. **Returned** sizes, capacity, the cursor
//! position, and seek results cross as `i64` (a JS number, exact to 2^53) so a value past
//! `u32::MAX` never wraps; **bit** offsets are `i64` in both directions, because a heap past
//! 512 MiB already has bit indexes above 2^32. A byte value is a `u8`, an `i32` value an
//! `i32`, and an `i64` value a JS `number` — accurate only up to ±2^53, so keep 64-bit values
//! below that. Byte arrays cross as `Buffer`; bulk typed arrays
//! (`preadI32Array` / `pwriteI64Array` / …) as `Array<number>`. Every source also carries its
//! metadata (`headers` — returned as a copy, `mode`, `kind`, from the `io` namespace) and UTF-8
//! text accessors. Every failing typed read, seek, slice, or UTF-8 decode surfaces as a thrown
//! `Error` carrying the core's guided text unchanged.

use napi::bindgen_prelude::{Buffer, Generator, ToNapiValue, Unknown};
use napi_derive::napi;

use crate::headers::Headers;
use crate::io::kind::IOKind;
use crate::io::mode::IOMode;
use crate::uri::Uri;
use yggdryl_core::io::memory as core;
use yggdryl_core::io::memory::IOBase;
use yggdryl_core::io::Serializable;

/// Maps any core error to a thrown JS `Error` (its guided text).
pub(crate) fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// Validates a JS **bit** offset: bit offsets cross as `i64` (a JS number, exact to 2^53) so
/// bits past 2^32 — every bit of a heap beyond 512 MiB — stay addressable; a negative offset
/// is rejected with a guided error naming the offending value.
pub(crate) fn to_bit_offset(offset: i64) -> napi::Result<u64> {
    u64::try_from(offset).map_err(|_| {
        to_error(format!(
            "invalid bit offset {offset}: expected a non-negative bit offset (LSB-first, bit 0 \
             is the least-significant bit of byte 0)"
        ))
    })
}

/// Pre-checks a bulk typed read against the bytes available **before** allocating the result
/// array, so a hostile `count` fails fast with the core's guided EOF text instead of first
/// materializing a giant allocation.
pub(crate) fn check_bulk_read(
    byte_size: u64,
    offset: u32,
    count: u32,
    width: u32,
) -> napi::Result<()> {
    let available = byte_size.saturating_sub(offset as u64);
    let requested = count as u64 * width as u64;
    if requested > available {
        return Err(to_error(core::IoError::UnexpectedEof {
            offset: offset as u64 + available,
            requested: requested as usize,
            available: available as usize,
        }));
    }
    Ok(())
}

/// Where a seek offset is measured from — the POSIX `lseek` `whence`: the **start** of the data
/// (`SEEK_SET`), the **current** cursor position (`SEEK_CUR`), or the **end** (`SEEK_END`).
#[napi(namespace = "memory")]
pub enum Whence {
    /// From the start of the data (absolute) — POSIX `SEEK_SET`.
    Start,
    /// From the current cursor position — POSIX `SEEK_CUR`.
    Current,
    /// From the end of the data — POSIX `SEEK_END`.
    End,
}

impl From<Whence> for core::Whence {
    fn from(value: Whence) -> Self {
        match value {
            Whence::Start => core::Whence::Start,
            Whence::Current => core::Whence::Current,
            Whence::End => core::Whence::End,
        }
    }
}

/// The yield type of an always-empty stream. The module exists so the generated `.d.ts`
/// yield type is TypeScript's `never` — napi-rs derives the type name from the **last path
/// segment** of `Generator::Yield`, and `never` is the honest typing for an iterator that
/// produces no item, ever (the Rust type is uninhabited, so the conversion is unreachable).
mod never {
    /// The uninhabited yield of [`NoChildren`](super::NoChildren) — nothing is ever produced.
    #[allow(non_camel_case_types)]
    pub enum never {}
}

impl ToNapiValue for never::never {
    unsafe fn to_napi_value(
        _env: napi::sys::napi_env,
        val: never::never,
    ) -> napi::Result<napi::sys::napi_value> {
        match val {}
    }
}

/// The always-empty child stream of a **leaf** source — what `ls(recursive?)` returns on
/// [`Heap`], [`Cursor`], [`Slice`], and the raw [`Mmap`](crate::io::local::Mmap): a real JS
/// iterable (`[Symbol.iterator]`, so `for..of` and spread work directly, exactly like the
/// streaming [`LocalEntries`](crate::io::local::LocalEntries)) that yields nothing. Mirrors
/// `yggdryl_core::io::memory::NoChildren` — the graph stays streamed even where it is empty.
#[napi(iterator, namespace = "memory")]
pub struct NoChildren {}

#[napi(namespace = "memory")]
impl Generator for NoChildren {
    type Yield = never::never;
    type Next = Unknown;
    type Return = Unknown;

    fn next(&mut self, _value: Option<Unknown>) -> Option<Self::Yield> {
        None
    }
}

impl NoChildren {
    /// Drives the core leaf `ls` / `ls_recursive` (a leaf's stream is empty by contract)
    /// and wraps it — the shared front door every leaf class's `ls(recursive?)` delegates to.
    pub(crate) fn over<T: IOBase>(source: &T, recursive: Option<bool>) -> napi::Result<NoChildren> {
        if recursive.unwrap_or(false) {
            let _ = source.ls_recursive().map_err(to_error)?;
        } else {
            let _ = source.ls().map_err(to_error)?;
        }
        Ok(NoChildren {})
    }
}

#[napi(namespace = "memory")]
impl NoChildren {
    /// A short debug string — always `NoChildren(<empty>)` (mirrors `LocalEntries.toString`).
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        "NoChildren(<empty>)".to_string()
    }
}

/// An in-heap byte buffer with a read/write cursor and amortized capacity — the concrete
/// in-memory source behind the byte-access contract.
///
/// It grows like a `Vec`: `Heap.withCapacity` pre-allocates, `capacity` reports the current
/// allocation, and `reserve` amortizes future writes. Equality is over the **stored bytes only**
/// (the cursor is transient I/O state), so two heaps holding the same bytes compare equal
/// regardless of where their cursors sit.
#[napi(namespace = "memory")]
pub struct Heap {
    pub(crate) inner: core::Heap,
}

#[napi(namespace = "memory")]
impl Heap {
    /// Builds a heap: from a **copy** of `data`'s bytes when given, else an empty buffer with the
    /// cursor at `0`. The generic, type-inferring entry — pass a `Buffer` or nothing.
    #[napi(constructor)]
    pub fn new(data: Option<Buffer>) -> Self {
        let inner = match data {
            Some(buffer) => core::Heap::from_slice(buffer.as_ref()),
            None => core::Heap::new(),
        };
        Self { inner }
    }

    /// An empty heap that can hold `capacity` bytes before reallocating — like
    /// `Vec::with_capacity`. Cursor at `0`.
    #[napi(factory)]
    pub fn with_capacity(capacity: u32) -> Self {
        Self {
            inner: core::Heap::with_capacity(capacity as usize),
        }
    }

    // ---- size + capacity ---------------------------------------------------------------

    /// The total length in bytes — an `i64` (a JS number, exact to 2^53) so a size past
    /// `u32::MAX` never wraps.
    #[napi]
    pub fn byte_size(&self) -> i64 {
        self.inner.byte_size() as i64
    }

    /// The total length in bits — `byteSize * 8`. Returned as an `i64` (a JS number, exact to
    /// 2^53) rather than `u32`, because a heap anywhere near the documented 4 GiB byte range has
    /// a bit count above `u32::MAX` (it exceeds it once the heap reaches 512 MiB) — so a `u32`
    /// would silently wrap. `2^35` bits (a full 4 GiB) is well within a JS number's exact range.
    #[napi]
    pub fn bit_size(&self) -> i64 {
        self.inner.bit_size() as i64
    }

    /// Whether the storage is empty (`byteSize == 0`).
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The number of bytes the storage can hold before it must reallocate — like
    /// `Vec::capacity`. An `i64` (a JS number, exact to 2^53): `Vec` growth doubles, so the
    /// allocation can legitimately exceed `u32::MAX`.
    #[napi]
    pub fn capacity(&self) -> i64 {
        self.inner.capacity() as i64
    }

    /// Reserves capacity for at least `additional` more bytes past the current `byteSize`,
    /// amortizing later writes — like `Vec::reserve`.
    #[napi]
    pub fn reserve(&mut self, additional: u32) {
        self.inner.reserve(additional as u64);
    }

    /// The spare room already allocated — `capacity - byteSize`, the bytes that can be
    /// appended before the next reallocation. An `i64` (JS number) like `capacity`.
    #[napi]
    pub fn spare_capacity(&self) -> i64 {
        self.inner.spare_capacity() as i64
    }

    /// Reserves capacity for **exactly** `additional` more bytes — no amortized
    /// over-allocation, for a caller that knows the final size.
    #[napi]
    pub fn reserve_exact(&mut self, additional: u32) {
        self.inner.reserve_exact(additional as u64);
    }

    /// **Checked** reservation: where `reserve` would abort the process on overflow or
    /// allocator failure, this throws a guided `Error` instead.
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

    /// **Checked** `ensureCapacity` — throws a guided `Error` instead of aborting.
    #[napi]
    pub fn try_ensure_capacity(&mut self, total: i64) -> napi::Result<()> {
        let total = u64::try_from(total).unwrap_or(u64::MAX);
        self.inner.try_ensure_capacity(total).map_err(to_error)
    }

    /// Releases spare capacity back to the allocator, shrinking toward `byteSize`.
    #[napi]
    pub fn shrink_to_fit(&mut self) {
        self.inner.shrink_to_fit();
    }

    /// Shrinks the allocation toward `minCapacity` (never below `byteSize`).
    #[napi]
    pub fn shrink_to(&mut self, min_capacity: u32) {
        self.inner.shrink_to(min_capacity as u64);
    }

    // ---- byte-array primitives ---------------------------------------------------------

    /// Reads up to `length` bytes at `offset` into a fresh `Buffer` — short (or empty) near the
    /// end. Never moves the cursor.
    #[napi]
    pub fn pread_byte_array(&self, offset: u32, length: u32) -> Buffer {
        self.inner.pread_vec(offset as u64, length as usize).into()
    }

    /// Writes `data` at `offset`, growing the storage (and zero-filling any gap) as needed;
    /// returns the number of bytes written (always `data.length`). Never moves the cursor.
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

    /// Writes the single byte `value` at `offset`, growing the storage as needed.
    #[napi]
    pub fn pwrite_byte(&mut self, offset: u32, value: u8) -> napi::Result<()> {
        self.inner
            .pwrite_byte(offset as u64, value)
            .map_err(to_error)
    }

    /// Reads the bit at absolute **bit** `offset` (LSB-first: bit `offset % 8` of byte
    /// `offset / 8`), or throws if its byte is past the end. The offset is an `i64` (exact to
    /// 2^53) so every bit of a heap beyond 512 MiB stays addressable; a negative offset throws.
    #[napi]
    pub fn pread_bit(&self, offset: i64) -> napi::Result<bool> {
        self.inner
            .pread_bit(to_bit_offset(offset)?)
            .map_err(to_error)
    }

    /// Sets or clears the bit at absolute **bit** `offset` (LSB-first), read-modify-writing its
    /// byte and growing the storage (zero-filled) if the bit is past the end. The offset is an
    /// `i64` (exact to 2^53); a negative offset throws.
    #[napi]
    pub fn pwrite_bit(&mut self, offset: i64, value: bool) -> napi::Result<()> {
        self.inner
            .pwrite_bit(to_bit_offset(offset)?, value)
            .map_err(to_error)
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

    /// Reads a little-endian `i64` (8 bytes) at `offset`, or throws if fewer bytes remain. The
    /// returned JS `number` is exact only up to ±2^53.
    #[napi]
    pub fn pread_i64(&self, offset: u32) -> napi::Result<i64> {
        self.inner.pread_i64(offset as u64).map_err(to_error)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at `offset`, growing as needed. Keep
    /// `value` below ±2^53 so the JS `number` stays exact.
    #[napi]
    pub fn pwrite_i64(&mut self, offset: u32, value: i64) -> napi::Result<()> {
        self.inner
            .pwrite_i64(offset as u64, value)
            .map_err(to_error)
    }

    // ---- utf8 text ---------------------------------------------------------------------

    /// Reads up to `length` **bytes** at `offset` and decodes them as UTF-8 text (clamped near
    /// the end), or throws a guided `Error` on invalid UTF-8 — including a multi-byte
    /// character cut by the range.
    #[napi]
    pub fn pread_utf8(&self, offset: u32, length: u32) -> napi::Result<String> {
        self.inner
            .pread_utf8(offset as u64, length as usize)
            .map_err(to_error)
    }

    /// Writes `text`'s UTF-8 bytes at `offset` (growing as needed); returns the number of
    /// **bytes** written (not characters).
    #[napi]
    pub fn pwrite_utf8(&mut self, offset: u32, text: String) -> u32 {
        self.inner.pwrite_utf8(offset as u64, &text) as u32
    }

    // ---- bulk typed arrays -------------------------------------------------------------

    /// **Bulk typed read** of `count` little-endian `i32`s at `offset` into a fresh array, or
    /// throws if fewer bytes remain — checked **before** the result array is allocated, so a
    /// hostile `count` fails fast instead of allocating.
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

    /// **Bulk typed read** of `count` little-endian `i64`s at `offset` into a fresh array, or
    /// throws if fewer bytes remain — checked **before** the result array is allocated, so a
    /// hostile `count` fails fast instead of allocating. Each JS `number` is exact only up to
    /// ±2^53.
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

    /// **Repeated-value fill.** Writes `count` copies of the byte `value` starting at `offset`
    /// (growing as needed) — the byte-level `memset`; no full array is ever materialized.
    #[napi]
    pub fn pwrite_byte_repeat(&mut self, offset: u32, value: u8, count: u32) -> napi::Result<()> {
        self.inner
            .pwrite_byte_repeat(offset as u64, value, count as usize)
            .map_err(to_error)
    }

    /// **Repeated-value fill** of `count` little-endian `i32` copies of `value` at `offset` —
    /// no full array is ever materialized.
    #[napi]
    pub fn pwrite_i32_repeat(&mut self, offset: u32, value: i32, count: u32) -> napi::Result<()> {
        self.inner
            .pwrite_i32_repeat(offset as u64, value, count as usize)
            .map_err(to_error)
    }

    /// **Repeated-value fill** of `count` little-endian `i64` copies of `value` at `offset` —
    /// no full array is ever materialized. Keep `value` below ±2^53 so it stays exact.
    #[napi]
    pub fn pwrite_i64_repeat(&mut self, offset: u32, value: i64, count: u32) -> napi::Result<()> {
        self.inner
            .pwrite_i64_repeat(offset as u64, value, count as usize)
            .map_err(to_error)
    }

    // ---- cursor: position / seek -------------------------------------------------------

    /// The current cursor position (bytes from the start) — an `i64` (exact to 2^53), so a
    /// position past `u32::MAX` (a seek can land anywhere) never wraps. May sit past the end.
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

    /// Reads up to `length` bytes from the current position into a fresh `Buffer`, advancing the
    /// cursor by the number read (short near the end).
    #[napi]
    pub fn read(&mut self, length: u32) -> Buffer {
        self.inner.read_vec(length as usize).into()
    }

    /// Writes `data` at the current position, advancing the cursor by the number written (growing
    /// the storage as needed); returns that count (always `data.length`).
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

    /// Reads a little-endian `i64` (8 bytes) at the cursor, advancing it by 8, or throws. The
    /// returned JS `number` is exact only up to ±2^53.
    #[napi]
    pub fn read_i64(&mut self) -> napi::Result<i64> {
        self.inner.read_i64().map_err(to_error)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at the cursor, advancing it by 8. Keep
    /// `value` below ±2^53 so the JS `number` stays exact.
    #[napi]
    pub fn write_i64(&mut self, value: i64) -> napi::Result<()> {
        self.inner.write_i64(value).map_err(to_error)
    }

    /// Reads from the current position **to the end** into a fresh `Buffer`, advancing the cursor
    /// to the end.
    #[napi]
    pub fn read_to_end(&mut self) -> Buffer {
        self.inner.read_to_end().into()
    }

    /// Reads up to `length` **bytes** from the cursor and decodes them as UTF-8 text (clamped
    /// near the end), advancing the cursor by the bytes read, or throws on invalid UTF-8
    /// (leaving the cursor put).
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

    // ---- slice + value semantics -------------------------------------------------------

    /// The window `[offset, offset + length)` as an independent `Heap` owning a copy of the range
    /// (addressed from its own `0`), or throws a guided `Error` if it runs past the end.
    #[napi]
    pub fn slice(&self, offset: u32, length: u32) -> napi::Result<Heap> {
        self.inner
            .slice(offset as u64, length as u64)
            .map(|inner| Heap { inner })
            .map_err(to_error)
    }

    // ---- address (uri) -----------------------------------------------------------------

    /// The [`Uri`] that addresses this heap — always the stable synthetic `mem://heap` (a
    /// heap stores no address; an anonymous in-memory buffer has no other identity).
    #[napi(getter)]
    pub fn uri(&self) -> Uri {
        Uri {
            inner: self.inner.uri(),
        }
    }

    // ---- metadata (headers / mode / kind) ----------------------------------------------

    /// The metadata [`Headers`] attached to this heap — **a copy**: edits to the returned map
    /// do not write back. Call `setHeaders` (or `withHeaders`) to store an updated map.
    #[napi(getter)]
    pub fn headers(&self) -> Headers {
        Headers {
            inner: self.inner.headers().clone(),
        }
    }

    /// Replaces the whole [`Headers`] metadata map in place.
    #[napi]
    pub fn set_headers(&mut self, headers: &Headers) {
        self.inner.set_headers(headers.inner.clone());
    }

    /// Returns a copy of this heap with its [`Headers`] metadata replaced.
    #[napi]
    pub fn with_headers(&self, headers: &Headers) -> Heap {
        Heap {
            inner: self.inner.clone().with_headers(headers.inner.clone()),
        }
    }

    /// How this heap may be accessed — see [`IOMode`] (`ReadWrite` by default; it is
    /// in-memory).
    #[napi(getter)]
    pub fn mode(&self) -> IOMode {
        self.inner.mode().into()
    }

    /// Sets the access [`IOMode`] in place.
    #[napi]
    pub fn set_mode(&mut self, mode: IOMode) {
        self.inner.set_mode(mode.into());
    }

    /// Returns a copy of this heap with its access [`IOMode`] set.
    #[napi]
    pub fn with_mode(&self, mode: IOMode) -> Heap {
        Heap {
            inner: self.inner.clone().with_mode(mode.into()),
        }
    }

    /// What this source is — always [`IOKind.Heap`] for an in-memory heap.
    #[napi(getter)]
    pub fn kind(&self) -> IOKind {
        self.inner.kind().into()
    }

    /// Whether this source is a regular **file** — derived from `kind`; always `false` for
    /// an in-memory heap.
    #[napi]
    pub fn is_file(&self) -> bool {
        self.inner.is_file()
    }

    /// Whether this source is a **directory** — derived from `kind`; always `false` for an
    /// in-memory heap.
    #[napi]
    pub fn is_dir(&self) -> bool {
        self.inner.is_dir()
    }

    /// Whether something **exists** at this source's address — always `true`: a live
    /// in-memory buffer exists although it is neither file nor directory.
    #[napi]
    pub fn exists(&self) -> bool {
        self.inner.exists()
    }

    // ---- the graph surface (a heap is a leaf node) -------------------------------------

    /// The node's own name — always the empty string: the synthetic `mem://heap` address
    /// has no path segment to take a name from.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name()
    }

    /// The parent node, or `null` — the inverse of `join`: an addressed heap
    /// (`mem://heap/logs/app.bin`) reports its directory address (`mem://heap/logs`), a bare
    /// `mem://heap` root reports `null`. (A heap is a **leaf** for *discovery* — it streams
    /// no children — but it is still addressable, so navigation composes through the URI.)
    #[napi]
    pub fn parent(&self) -> Option<Heap> {
        self.inner.parent().map(|inner| Heap { inner })
    }

    /// The child node at `segment` — a **new, independent in-memory buffer** whose address is
    /// composed by joining `segment` onto this heap's URI (`Uri.joinpath`), so
    /// `child.parent()` addresses this node again. `segment` may be multi-segment (`"a/b/c"`),
    /// and a spaced segment percent-encodes in the address (`"my dir/f"` →
    /// `mem://heap/my%20dir/f`). Pure address algebra — the child owns no bytes yet, and
    /// writing it never touches this heap. The named mirror of Python's `__truediv__` (JS has
    /// no `/` operator); throws the core's guided `Error` on a non-navigable source.
    #[napi]
    pub fn join(&self, segment: String) -> napi::Result<Heap> {
        self.inner
            .join(&segment)
            .map(|inner| Heap { inner })
            .map_err(to_error)
    }

    /// Streams this node's children — always the empty [`NoChildren`] iterable: a heap is
    /// a **leaf** and streams nothing (`recursive` is accepted for the uniform
    /// `ls(recursive?)` shape and changes nothing on a leaf).
    #[napi]
    pub fn ls(&self, recursive: Option<bool>) -> napi::Result<NoChildren> {
        NoChildren::over(&self.inner, recursive)
    }

    /// The direct children, collected — always an empty array (a heap is a leaf).
    #[napi]
    pub fn children(&self) -> napi::Result<Vec<Heap>> {
        self.inner
            .children()
            .map(|nodes| nodes.into_iter().map(|inner| Heap { inner }).collect())
            .map_err(to_error)
    }

    /// Removing a heap — always the guided refusal: an in-memory source has no removable
    /// backing; address a filesystem node (e.g. `LocalIO`) instead.
    #[napi]
    pub fn rm(&self) -> napi::Result<()> {
        self.inner.rm().map_err(to_error)
    }

    /// Removing a heap **as a file** — always the guided refusal (no removable backing).
    #[napi]
    pub fn rmfile(&self) -> napi::Result<()> {
        self.inner.rmfile().map_err(to_error)
    }

    /// Removing a heap **as a directory** — always the guided refusal (no removable
    /// backing).
    #[napi]
    pub fn rmdir(&self) -> napi::Result<()> {
        self.inner.rmdir().map_err(to_error)
    }

    // ---- cursor / window views ---------------------------------------------------------

    /// An independent [`Cursor`] over a copy of this heap, positioned at the start.
    #[napi]
    pub fn cursor(&self) -> Cursor {
        Cursor {
            inner: self.inner.clone().cursor(),
        }
    }

    /// A bounded [`Slice`] view `[offset, offset + length)` over a copy of this heap (addressed
    /// from its own `0`), or throws a guided `Error` if it runs past the end.
    #[napi]
    pub fn window(&self, offset: u32, length: u32) -> napi::Result<Slice> {
        self.inner
            .clone()
            .window(offset as u64, length as u64)
            .map(|inner| Slice { inner })
            .map_err(to_error)
    }

    /// A copy of the stored bytes as a `Buffer` (the cursor is not included).
    #[napi]
    pub fn to_bytes(&self) -> Buffer {
        self.inner.as_slice().to_vec().into()
    }

    /// An explicit copy of this heap — bytes, cursor, headers, and mode all copied.
    #[napi]
    pub fn copy(&self) -> Heap {
        Heap {
            inner: self.inner.clone(),
        }
    }

    /// Content equality — equal iff the stored bytes are equal, regardless of cursor position.
    #[napi]
    pub fn equals(&self, other: &Heap) -> bool {
        self.inner == other.inner
    }

    /// The heap's value form: a copy of the stored bytes — the same identity `equals` uses
    /// (the cursor, address, headers, and mode are transient metadata and are not serialized).
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        Serializable::serialize_bytes(&self.inner).into()
    }

    /// Reconstructs a heap from bytes produced by `serializeBytes` — the exact inverse.
    #[napi(factory)]
    pub fn deserialize_bytes(data: Buffer) -> napi::Result<Heap> {
        <core::Heap as Serializable>::deserialize_bytes(data.as_ref())
            .map(|inner| Heap { inner })
            .map_err(to_error)
    }

    /// A short debug string of the form `Heap(len=N)`.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!("Heap(len={})", self.inner.byte_size())
    }
}

/// A moving read/write position over an in-heap byte source — the concrete cursor that
/// `read`/`write`/`seek` advance. Mirrors `yggdryl_core::io::memory::IOCursor<Heap>`: it owns a
/// copy of the source and a position, and is itself a byte source (its positioned `pread_*` /
/// `pwrite_*` and `uri` delegate to the wrapped heap).
#[napi(namespace = "memory")]
pub struct Cursor {
    pub(crate) inner: core::IOCursor<core::Heap>,
}

#[napi(namespace = "memory")]
impl Cursor {
    /// Builds a cursor over a **copy** of `data`'s bytes when given, else an empty heap, with the
    /// position at `0`.
    #[napi(constructor)]
    pub fn new(data: Option<Buffer>) -> Self {
        let heap = match data {
            Some(buffer) => core::Heap::from_slice(buffer.as_ref()),
            None => core::Heap::new(),
        };
        Self {
            inner: heap.cursor(),
        }
    }

    /// Wraps an **existing** [`Heap`] (a copy of it) in a cursor positioned at the start — the
    /// factory counterpart to the `new(data?)` constructor.
    #[napi(factory)]
    pub fn over(heap: &Heap) -> Cursor {
        Cursor {
            inner: heap.inner.clone().cursor(),
        }
    }

    // ---- position / seek ---------------------------------------------------------------

    /// The current position (bytes from the start) — an `i64` (exact to 2^53), so a position
    /// past `u32::MAX` never wraps. May sit past the end after a seek.
    #[napi(getter)]
    pub fn position(&self) -> i64 {
        self.inner.position() as i64
    }

    /// Moves the position to an absolute `position` (past the end is allowed).
    #[napi]
    pub fn set_position(&mut self, position: u32) {
        self.inner.set_position(position as u64);
    }

    /// Seeks to `whence + offset` and returns the new position (an `i64`, exact to 2^53);
    /// seeking before the start throws.
    #[napi]
    pub fn seek(&mut self, whence: Whence, offset: i64) -> napi::Result<i64> {
        self.inner
            .seek(whence.into(), offset)
            .map(|position| position as i64)
            .map_err(to_error)
    }

    /// Resets the position to the start.
    #[napi]
    pub fn rewind(&mut self) {
        self.inner.rewind();
    }

    // ---- stream read / write -----------------------------------------------------------

    /// Reads up to `length` bytes from the current position into a fresh `Buffer`, advancing the
    /// position by the number read (short near the end).
    #[napi]
    pub fn read(&mut self, length: u32) -> Buffer {
        self.inner.read_vec(length as usize).into()
    }

    /// Writes `data` at the current position, advancing the position by the number written
    /// (growing the storage as needed); returns that count.
    #[napi]
    pub fn write(&mut self, data: Buffer) -> u32 {
        self.inner.write(data.as_ref()) as u32
    }

    /// Reads the next byte at the position, advancing it by 1, or throws at the end.
    #[napi]
    pub fn read_byte(&mut self) -> napi::Result<u8> {
        self.inner.read_byte().map_err(to_error)
    }

    /// Writes the byte `value` at the position, advancing it by 1.
    #[napi]
    pub fn write_byte(&mut self, value: u8) -> napi::Result<()> {
        self.inner.write_byte(value).map_err(to_error)
    }

    /// Reads a little-endian `i32` (4 bytes) at the position, advancing it by 4, or throws.
    #[napi]
    pub fn read_i32(&mut self) -> napi::Result<i32> {
        self.inner.read_i32().map_err(to_error)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at the position, advancing it by 4.
    #[napi]
    pub fn write_i32(&mut self, value: i32) -> napi::Result<()> {
        self.inner.write_i32(value).map_err(to_error)
    }

    /// Reads a little-endian `i64` (8 bytes) at the position, advancing it by 8, or throws.
    #[napi]
    pub fn read_i64(&mut self) -> napi::Result<i64> {
        self.inner.read_i64().map_err(to_error)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at the position, advancing it by 8.
    #[napi]
    pub fn write_i64(&mut self, value: i64) -> napi::Result<()> {
        self.inner.write_i64(value).map_err(to_error)
    }

    /// Reads from the current position **to the end** into a fresh `Buffer`, advancing the
    /// position to the end.
    #[napi]
    pub fn read_to_end(&mut self) -> Buffer {
        self.inner.read_to_end().into()
    }

    /// Reads up to `length` **bytes** from the position and decodes them as UTF-8 text
    /// (clamped near the end), advancing the position by the bytes read, or throws on invalid
    /// UTF-8 (leaving the position put).
    #[napi]
    pub fn read_utf8(&mut self, length: u32) -> napi::Result<String> {
        self.inner.read_utf8(length as usize).map_err(to_error)
    }

    /// Writes `text`'s UTF-8 bytes at the position, advancing it; returns the number of
    /// **bytes** written (not characters).
    #[napi]
    pub fn write_utf8(&mut self, text: String) -> u32 {
        self.inner.write_utf8(&text) as u32
    }

    // ---- IOBase: size + positioned typed accessors -------------------------------------

    /// The total length in bytes of the wrapped source — an `i64` (exact to 2^53), matching
    /// `Heap.byteSize`.
    #[napi]
    pub fn byte_size(&self) -> i64 {
        self.inner.byte_size() as i64
    }

    /// The total length in bits — `byteSize * 8` (an `i64`, matching `Heap.bitSize`).
    #[napi]
    pub fn bit_size(&self) -> i64 {
        self.inner.bit_size() as i64
    }

    /// Reads the single byte at absolute `offset`, or throws if it is past the end.
    #[napi]
    pub fn pread_byte(&self, offset: u32) -> napi::Result<u8> {
        self.inner.pread_byte(offset as u64).map_err(to_error)
    }

    /// Writes the single byte `value` at absolute `offset`, growing the storage as needed.
    #[napi]
    pub fn pwrite_byte(&mut self, offset: u32, value: u8) -> napi::Result<()> {
        self.inner
            .pwrite_byte(offset as u64, value)
            .map_err(to_error)
    }

    /// Reads the bit at absolute **bit** `offset` (LSB-first), or throws if its byte is past the
    /// end. The offset is an `i64` (exact to 2^53); a negative offset throws.
    #[napi]
    pub fn pread_bit(&self, offset: i64) -> napi::Result<bool> {
        self.inner
            .pread_bit(to_bit_offset(offset)?)
            .map_err(to_error)
    }

    /// Sets or clears the bit at absolute **bit** `offset` (LSB-first), growing the storage as
    /// needed. The offset is an `i64` (exact to 2^53); a negative offset throws.
    #[napi]
    pub fn pwrite_bit(&mut self, offset: i64, value: bool) -> napi::Result<()> {
        self.inner
            .pwrite_bit(to_bit_offset(offset)?, value)
            .map_err(to_error)
    }

    /// Reads a little-endian `i32` (4 bytes) at absolute `offset`, or throws if fewer bytes remain.
    #[napi]
    pub fn pread_i32(&self, offset: u32) -> napi::Result<i32> {
        self.inner.pread_i32(offset as u64).map_err(to_error)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at absolute `offset`, growing as needed.
    #[napi]
    pub fn pwrite_i32(&mut self, offset: u32, value: i32) -> napi::Result<()> {
        self.inner
            .pwrite_i32(offset as u64, value)
            .map_err(to_error)
    }

    /// Reads a little-endian `i64` (8 bytes) at absolute `offset`, or throws if fewer bytes remain.
    #[napi]
    pub fn pread_i64(&self, offset: u32) -> napi::Result<i64> {
        self.inner.pread_i64(offset as u64).map_err(to_error)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at absolute `offset`, growing as needed.
    #[napi]
    pub fn pwrite_i64(&mut self, offset: u32, value: i64) -> napi::Result<()> {
        self.inner
            .pwrite_i64(offset as u64, value)
            .map_err(to_error)
    }

    /// Reads up to `length` **bytes** at absolute `offset` and decodes them as UTF-8 text
    /// (clamped near the end), or throws a guided `Error` on invalid UTF-8. Never moves the
    /// position.
    #[napi]
    pub fn pread_utf8(&self, offset: u32, length: u32) -> napi::Result<String> {
        self.inner
            .pread_utf8(offset as u64, length as usize)
            .map_err(to_error)
    }

    /// Writes `text`'s UTF-8 bytes at absolute `offset` (growing as needed); returns the
    /// number of **bytes** written (not characters). Never moves the position.
    #[napi]
    pub fn pwrite_utf8(&mut self, offset: u32, text: String) -> u32 {
        self.inner.pwrite_utf8(offset as u64, &text) as u32
    }

    // ---- source access -----------------------------------------------------------------

    /// The [`Uri`] addressing the wrapped source.
    #[napi(getter)]
    pub fn uri(&self) -> Uri {
        Uri {
            inner: self.inner.uri(),
        }
    }

    /// The wrapped source's metadata [`Headers`] — **a copy** (delegates to the source).
    #[napi(getter)]
    pub fn headers(&self) -> Headers {
        Headers {
            inner: self.inner.headers().clone(),
        }
    }

    /// The wrapped source's access [`IOMode`] (delegates to the source).
    #[napi(getter)]
    pub fn mode(&self) -> IOMode {
        self.inner.mode().into()
    }

    /// The wrapped source's [`IOKind`] (delegates to the source).
    #[napi(getter)]
    pub fn kind(&self) -> IOKind {
        self.inner.kind().into()
    }

    /// Whether the wrapped source is a regular **file** — the core derivation
    /// `kind == IOKind.File`; `false` over a heap.
    #[napi]
    pub fn is_file(&self) -> bool {
        self.inner.is_file()
    }

    /// Whether the wrapped source is a **directory** — the core derivation
    /// `kind == IOKind.Directory`; `false` over a heap.
    #[napi]
    pub fn is_dir(&self) -> bool {
        self.inner.is_dir()
    }

    /// Whether something **exists** at the wrapped source's address — forwards the source's
    /// own notion (`true` over a live heap, which exists although it is neither file nor
    /// directory).
    #[napi]
    pub fn exists(&self) -> bool {
        self.inner.exists()
    }

    // ---- the graph surface (a cursor view is a leaf node) ------------------------------

    /// The node's own name — always the empty string (the wrapped heap's `mem://heap`
    /// address has no path segment).
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name()
    }

    /// The parent node — always `null`: a cursor view is a leaf of the IO graph.
    #[napi]
    pub fn parent(&self) -> Option<Cursor> {
        self.inner.parent().map(|inner| Cursor { inner })
    }

    /// Streams this node's children — always the empty [`NoChildren`] iterable (a leaf
    /// streams nothing; `recursive` changes nothing on a leaf).
    #[napi]
    pub fn ls(&self, recursive: Option<bool>) -> napi::Result<NoChildren> {
        NoChildren::over(&self.inner, recursive)
    }

    /// The direct children, collected — always an empty array (a cursor view is a leaf).
    #[napi]
    pub fn children(&self) -> napi::Result<Vec<Cursor>> {
        self.inner
            .children()
            .map(|nodes| nodes.into_iter().map(|inner| Cursor { inner }).collect())
            .map_err(to_error)
    }

    /// Removing a cursor view — always the guided refusal: an in-memory source has no
    /// removable backing; address a filesystem node (e.g. `LocalIO`) instead.
    #[napi]
    pub fn rm(&self) -> napi::Result<()> {
        self.inner.rm().map_err(to_error)
    }

    /// Removing a cursor view **as a file** — always the guided refusal.
    #[napi]
    pub fn rmfile(&self) -> napi::Result<()> {
        self.inner.rmfile().map_err(to_error)
    }

    /// Removing a cursor view **as a directory** — always the guided refusal.
    #[napi]
    pub fn rmdir(&self) -> napi::Result<()> {
        self.inner.rmdir().map_err(to_error)
    }

    /// A copy of the wrapped [`Heap`] source (the position is not carried).
    #[napi]
    pub fn inner(&self) -> Heap {
        Heap {
            inner: self.inner.inner().clone(),
        }
    }

    /// A copy of the wrapped source's stored bytes as a `Buffer`.
    #[napi]
    pub fn to_bytes(&self) -> Buffer {
        self.inner.inner().as_slice().to_vec().into()
    }

    /// A short debug string of the form `Cursor(pos=P, len=N)`.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "Cursor(pos={}, len={})",
            self.inner.position(),
            self.inner.byte_size()
        )
    }
}

/// A bounded, fixed-length window over an in-heap byte source, addressed from its own `0`.
/// Mirrors `yggdryl_core::io::memory::IOSlice<Heap>`: it owns a copy of the source and presents the
/// range `[offset, offset + length)`. A write past the window's end is clamped (it can never
/// grow the source beyond the window).
#[napi(namespace = "memory")]
pub struct Slice {
    pub(crate) inner: core::IOSlice<core::Heap>,
}

#[napi(namespace = "memory")]
impl Slice {
    /// The window `[offset, offset + length)` over a **copy** of `heap`, or throws a guided
    /// `Error` if it runs past the source's end.
    #[napi(factory)]
    pub fn over(heap: &Heap, offset: u32, length: u32) -> napi::Result<Slice> {
        core::IOSlice::new(heap.inner.clone(), offset as u64, length as u64)
            .map(|inner| Slice { inner })
            .map_err(to_error)
    }

    /// The window length in bytes — an `i64` (exact to 2^53), matching `Heap.byteSize`.
    #[napi]
    pub fn byte_size(&self) -> i64 {
        self.inner.byte_size() as i64
    }

    /// The window's start offset within the source.
    #[napi(getter)]
    pub fn offset(&self) -> u32 {
        self.inner.offset() as u32
    }

    /// Reads up to `length` bytes at `offset` **within the window** into a fresh `Buffer` (short
    /// or empty near the window's end). Never moves any cursor.
    #[napi]
    pub fn pread_byte_array(&self, offset: u32, length: u32) -> Buffer {
        self.inner.pread_vec(offset as u64, length as usize).into()
    }

    /// Reads the single byte at `offset` within the window, or throws if it is past the window's end.
    #[napi]
    pub fn pread_byte(&self, offset: u32) -> napi::Result<u8> {
        self.inner.pread_byte(offset as u64).map_err(to_error)
    }

    /// Reads a little-endian `i32` (4 bytes) at `offset` within the window, or throws.
    #[napi]
    pub fn pread_i32(&self, offset: u32) -> napi::Result<i32> {
        self.inner.pread_i32(offset as u64).map_err(to_error)
    }

    /// Reads a little-endian `i64` (8 bytes) at `offset` within the window, or throws.
    #[napi]
    pub fn pread_i64(&self, offset: u32) -> napi::Result<i64> {
        self.inner.pread_i64(offset as u64).map_err(to_error)
    }

    /// Reads up to `length` **bytes** at `offset` within the window and decodes them as UTF-8
    /// text (clamped to the window's end), or throws a guided `Error` on invalid UTF-8. The
    /// window is fixed-length, so there is deliberately no `pwriteUtf8` — a slice cannot grow.
    #[napi]
    pub fn pread_utf8(&self, offset: u32, length: u32) -> napi::Result<String> {
        self.inner
            .pread_utf8(offset as u64, length as usize)
            .map_err(to_error)
    }

    /// Writes `data` at `offset` within the window, **clamped** to the window's end; returns the
    /// number of bytes actually written (short if it would overflow the window).
    #[napi]
    pub fn pwrite_byte_array(&mut self, offset: u32, data: Buffer) -> u32 {
        self.inner.pwrite_byte_array(offset as u64, data.as_ref()) as u32
    }

    /// The [`Uri`] addressing the wrapped source.
    #[napi(getter)]
    pub fn uri(&self) -> Uri {
        Uri {
            inner: self.inner.uri(),
        }
    }

    /// The wrapped source's metadata [`Headers`] — **a copy** (delegates to the source).
    #[napi(getter)]
    pub fn headers(&self) -> Headers {
        Headers {
            inner: self.inner.headers().clone(),
        }
    }

    /// The wrapped source's access [`IOMode`] (delegates to the source).
    #[napi(getter)]
    pub fn mode(&self) -> IOMode {
        self.inner.mode().into()
    }

    /// The wrapped source's [`IOKind`] (delegates to the source).
    #[napi(getter)]
    pub fn kind(&self) -> IOKind {
        self.inner.kind().into()
    }

    /// Whether the wrapped source is a regular **file** — the core derivation
    /// `kind == IOKind.File`; `false` over a heap.
    #[napi]
    pub fn is_file(&self) -> bool {
        self.inner.is_file()
    }

    /// Whether the wrapped source is a **directory** — the core derivation
    /// `kind == IOKind.Directory`; `false` over a heap.
    #[napi]
    pub fn is_dir(&self) -> bool {
        self.inner.is_dir()
    }

    /// Whether something **exists** at the wrapped source's address — forwards the source's
    /// own notion (`true` over a live heap, which exists although it is neither file nor
    /// directory).
    #[napi]
    pub fn exists(&self) -> bool {
        self.inner.exists()
    }

    // ---- the graph surface (a slice window is a leaf node) -----------------------------

    /// The node's own name — always the empty string (the wrapped heap's `mem://heap`
    /// address has no path segment).
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name()
    }

    /// The parent node — always `null`: a slice window is a leaf of the IO graph.
    #[napi]
    pub fn parent(&self) -> Option<Slice> {
        self.inner.parent().map(|inner| Slice { inner })
    }

    /// Streams this node's children — always the empty [`NoChildren`] iterable (a leaf
    /// streams nothing; `recursive` changes nothing on a leaf).
    #[napi]
    pub fn ls(&self, recursive: Option<bool>) -> napi::Result<NoChildren> {
        NoChildren::over(&self.inner, recursive)
    }

    /// The direct children, collected — always an empty array (a slice window is a leaf).
    #[napi]
    pub fn children(&self) -> napi::Result<Vec<Slice>> {
        self.inner
            .children()
            .map(|nodes| nodes.into_iter().map(|inner| Slice { inner }).collect())
            .map_err(to_error)
    }

    /// Removing a slice window — always the guided refusal: an in-memory source has no
    /// removable backing; address a filesystem node (e.g. `LocalIO`) instead.
    #[napi]
    pub fn rm(&self) -> napi::Result<()> {
        self.inner.rm().map_err(to_error)
    }

    /// Removing a slice window **as a file** — always the guided refusal.
    #[napi]
    pub fn rmfile(&self) -> napi::Result<()> {
        self.inner.rmfile().map_err(to_error)
    }

    /// Removing a slice window **as a directory** — always the guided refusal.
    #[napi]
    pub fn rmdir(&self) -> napi::Result<()> {
        self.inner.rmdir().map_err(to_error)
    }

    /// A copy of the wrapped [`Heap`] source (the whole source, not just the window).
    #[napi]
    pub fn inner(&self) -> Heap {
        Heap {
            inner: self.inner.inner().clone(),
        }
    }

    /// A copy of the window's bytes (addressed from its own `0`) as a `Buffer`.
    #[napi]
    pub fn to_bytes(&self) -> Buffer {
        let length = self.inner.byte_size() as usize;
        self.inner.pread_vec(0, length).into()
    }

    /// A short debug string of the form `Slice(offset=O, len=N)`.
    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "Slice(offset={}, len={})",
            self.inner.offset(),
            self.inner.byte_size()
        )
    }
}
