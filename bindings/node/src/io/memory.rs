//! The `yggdryl.memory` namespace — the in-heap and memory-mapped byte sources and their
//! seek anchor.
//!
//! Mirrors `yggdryl_core::io::memory`'s concrete sources [`Heap`] (an owned byte buffer with a
//! read/write cursor and `Vec`-like capacity) and [`Mmap`] (the same surface memory-mapped over
//! a file), plus the [`Whence`] seek anchor. Every method is a thin one- or two-line delegation
//! to `yggdryl_core` — the positioned primitives and typed accessors of `IOBase`, the cursor
//! stream of `IOCursor`, bounded `IOSlice` windows, and `Whence`-relative seeks — with no logic
//! in the binding. The file-backed [`Mmap`] adds one binding-level method, `close()`, because
//! JavaScript has no deterministic drop (see its docs).
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

use napi::bindgen_prelude::{Buffer, Either};
use napi_derive::napi;

use crate::headers::Headers;
use crate::io::kind::IOKind;
use crate::io::mode::IOMode;
use crate::uri::Uri;
use yggdryl_core::io::local;
use yggdryl_core::io::memory as core;
use yggdryl_core::io::memory::IOBase;
use yggdryl_core::io::Serializable;

/// Maps any core error to a thrown JS `Error` (its guided text).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// Validates a JS **bit** offset: bit offsets cross as `i64` (a JS number, exact to 2^53) so
/// bits past 2^32 — every bit of a heap beyond 512 MiB — stay addressable; a negative offset
/// is rejected with a guided error naming the offending value.
fn to_bit_offset(offset: i64) -> napi::Result<u64> {
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
fn check_bulk_read(byte_size: u64, offset: u32, count: u32, width: u32) -> napi::Result<()> {
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

/// The guided error for a method called on a mapping after `close()`.
fn closed_err() -> napi::Error {
    to_error("the mapping is closed; reopen it with Mmap.open / Mmap.openReadonly / Mmap.create")
}

/// A **memory-mapped file** — the on-disk source behind the byte-access contract, sharing
/// [`Heap`]'s full surface (positioned + typed + bulk access, the built-in cursor stream,
/// capacity management, metadata) over a file instead of an owned buffer.
///
/// A mapping is opened by path or [`Uri`] through the factories (`Mmap.open` /
/// `Mmap.openReadonly` / `Mmap.create` — there is no plain constructor). `byteSize` is the
/// **logical** length; `capacity` is the mapped (file) extent, which grows **amortized**
/// (doubling, page-aligned) when a write lands past the end — the same allocation curve as
/// [`Heap`]. `close()` unmaps deterministically and truncates the on-disk file back to the
/// logical length; JavaScript has no deterministic drop, and on Windows a mapped file cannot
/// be deleted while a view is open, so call it as soon as the mapping is done.
///
/// Unlike [`Heap`], a mapping is a **live OS resource, not a value**: two independent mappings
/// of one file would alias, so it is deliberately not clonable, equatable, or serializable —
/// there is no `equals`, `copy`, `serializeBytes`, `withHeaders`, or `withMode` (use the
/// in-place `setHeaders` / `setMode`).
#[napi(namespace = "memory")]
pub struct Mmap {
    /// `None` once `close()` has run — every later use throws the guided closed error.
    inner: Option<local::Mmap>,
}

impl Mmap {
    /// The live mapping, or the guided closed error after `close()`.
    fn inner(&self) -> napi::Result<&local::Mmap> {
        self.inner.as_ref().ok_or_else(closed_err)
    }

    /// The live mapping, mutably, or the guided closed error after `close()`.
    fn inner_mut(&mut self) -> napi::Result<&mut local::Mmap> {
        self.inner.as_mut().ok_or_else(closed_err)
    }

    /// Wraps a freshly opened core mapping (an open failure throws its guided text).
    fn from_core(inner: Result<local::Mmap, core::IoError>) -> napi::Result<Mmap> {
        Ok(Mmap {
            inner: Some(inner.map_err(to_error)?),
        })
    }
}

#[napi(namespace = "memory")]
impl Mmap {
    // ---- factories (the generic, type-inferring entries) -------------------------------

    /// Opens an **existing** file read-write — the generic entry: a **string** dispatches to
    /// the core `open_path`, a [`Uri`] (`file://…` or a plain path) to `open_uri`. Throws a
    /// guided `Error` naming the path if it is missing or inaccessible.
    #[napi(factory)]
    pub fn open(source: Either<String, &Uri>) -> napi::Result<Mmap> {
        Self::from_core(match source {
            Either::A(path) => local::Mmap::open_path(&path),
            Either::B(uri) => local::Mmap::open_uri(&uri.inner),
        })
    }

    /// Opens an **existing** file **read-only** (same string / [`Uri`] dispatch as `open`):
    /// reads work, the write primitives write nothing (count `0`), and full writes throw a
    /// guided `Error` naming the fix.
    #[napi(factory)]
    pub fn open_readonly(source: Either<String, &Uri>) -> napi::Result<Mmap> {
        Self::from_core(match source {
            Either::A(path) => local::Mmap::open_path_readonly(&path),
            Either::B(uri) => local::Mmap::open_uri_readonly(&uri.inner),
        })
    }

    /// Opens the file read-write, **creating it empty** if it does not exist — existing
    /// contents are kept, never truncated on open (same string / [`Uri`] dispatch as `open`).
    #[napi(factory)]
    pub fn create(source: Either<String, &Uri>) -> napi::Result<Mmap> {
        Self::from_core(match source {
            Either::A(path) => local::Mmap::create_path(&path),
            Either::B(uri) => local::Mmap::create_uri(&uri.inner),
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
