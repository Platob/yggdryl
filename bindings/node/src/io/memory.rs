//! The `yggdryl.memory` namespace — the in-heap byte source and its seek anchor.
//!
//! Mirrors `yggdryl_core::io::memory`'s concrete source [`Heap`] (an owned byte buffer with a
//! read/write cursor and `Vec`-like capacity) and the [`Whence`] seek anchor. Every method is a
//! thin one- or two-line delegation to `yggdryl_core` — the positioned primitives and typed
//! accessors of `IOBase`, the cursor stream of `IOCursor`, bounded `IOSlice` windows, and
//! `Whence`-relative seeks — with no logic in the binding.
//!
//! Numeric idioms: offsets, lengths, sizes, capacity, cursor position, and bit offsets are JS
//! `number`s typed as `u32`, so a single heap addresses up to 4 GiB in memory. A byte value is a
//! `u8`, an `i32` value an `i32`, and an `i64` value a JS `number` — accurate only up to
//! ±2^53, so keep 64-bit values below that. Byte arrays cross as `Buffer`. Every failing typed
//! read, seek, or slice surfaces as a thrown `Error` carrying the core's guided text unchanged.

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

use crate::io::uri::Uri;
use yggdryl_core::io::memory as core;
use yggdryl_core::io::memory::IOBase;

/// Maps any core error to a thrown JS `Error` (its guided text).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
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

    /// The total length in bytes.
    #[napi]
    pub fn byte_size(&self) -> u32 {
        self.inner.byte_size() as u32
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

    /// The number of bytes the storage can hold before it must reallocate — like `Vec::capacity`.
    #[napi]
    pub fn capacity(&self) -> u32 {
        self.inner.capacity() as u32
    }

    /// Reserves capacity for at least `additional` more bytes past the current `byteSize`,
    /// amortizing later writes — like `Vec::reserve`.
    #[napi]
    pub fn reserve(&mut self, additional: u32) {
        self.inner.reserve(additional as u64);
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
    /// `offset / 8`), or throws if its byte is past the end.
    #[napi]
    pub fn pread_bit(&self, offset: u32) -> napi::Result<bool> {
        self.inner.pread_bit(offset as u64).map_err(to_error)
    }

    /// Sets or clears the bit at absolute **bit** `offset` (LSB-first), read-modify-writing its
    /// byte and growing the storage (zero-filled) if the bit is past the end.
    #[napi]
    pub fn pwrite_bit(&mut self, offset: u32, value: bool) -> napi::Result<()> {
        self.inner
            .pwrite_bit(offset as u64, value)
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

    // ---- cursor: position / seek -------------------------------------------------------

    /// The current cursor position (bytes from the start). May sit past the end after a seek.
    #[napi(getter)]
    pub fn position(&self) -> u32 {
        self.inner.position() as u32
    }

    /// Moves the cursor to an absolute `position` (past the end is allowed).
    #[napi]
    pub fn set_position(&mut self, position: u32) {
        self.inner.set_position(position as u64);
    }

    /// Seeks to `whence + offset` and returns the new position. A position past the end is
    /// allowed; seeking before the start throws a guided `Error`.
    #[napi]
    pub fn seek(&mut self, whence: Whence, offset: i64) -> napi::Result<u32> {
        self.inner
            .seek(whence.into(), offset)
            .map(|position| position as u32)
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

    /// The [`Uri`] that addresses this heap (empty by default).
    #[napi(getter)]
    pub fn uri(&self) -> Uri {
        Uri {
            inner: self.inner.uri(),
        }
    }

    /// Sets the addressing `Uri` in place.
    #[napi]
    pub fn set_uri(&mut self, uri: &Uri) {
        self.inner.set_uri(uri.inner.clone());
    }

    /// Returns a copy of this heap with its addressing `Uri` set.
    #[napi]
    pub fn with_uri(&self, uri: &Uri) -> Heap {
        Heap {
            inner: self.inner.clone().with_uri(uri.inner.clone()),
        }
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

    /// An explicit copy of this heap (bytes and cursor), optionally overriding the addressing
    /// `Uri` — like `copy(uri=…)`. With no argument it is a plain clone.
    #[napi]
    pub fn copy(&self, uri: Option<&Uri>) -> Heap {
        let mut inner = self.inner.clone();
        if let Some(uri) = uri {
            inner.set_uri(uri.inner.clone());
        }
        Heap { inner }
    }

    /// Content equality — equal iff the stored bytes are equal, regardless of cursor position.
    #[napi]
    pub fn equals(&self, other: &Heap) -> bool {
        self.inner == other.inner
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

    /// The current position (bytes from the start). May sit past the end after a seek.
    #[napi(getter)]
    pub fn position(&self) -> u32 {
        self.inner.position() as u32
    }

    /// Moves the position to an absolute `position` (past the end is allowed).
    #[napi]
    pub fn set_position(&mut self, position: u32) {
        self.inner.set_position(position as u64);
    }

    /// Seeks to `whence + offset` and returns the new position; seeking before the start throws.
    #[napi]
    pub fn seek(&mut self, whence: Whence, offset: i64) -> napi::Result<u32> {
        self.inner
            .seek(whence.into(), offset)
            .map(|position| position as u32)
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

    // ---- IOBase: size + positioned typed accessors -------------------------------------

    /// The total length in bytes of the wrapped source.
    #[napi]
    pub fn byte_size(&self) -> u32 {
        self.inner.byte_size() as u32
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

    /// Reads the bit at absolute **bit** `offset` (LSB-first), or throws if its byte is past the end.
    #[napi]
    pub fn pread_bit(&self, offset: u32) -> napi::Result<bool> {
        self.inner.pread_bit(offset as u64).map_err(to_error)
    }

    /// Sets or clears the bit at absolute **bit** `offset` (LSB-first), growing the storage as needed.
    #[napi]
    pub fn pwrite_bit(&mut self, offset: u32, value: bool) -> napi::Result<()> {
        self.inner
            .pwrite_bit(offset as u64, value)
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

    // ---- source access -----------------------------------------------------------------

    /// The [`Uri`] addressing the wrapped source.
    #[napi(getter)]
    pub fn uri(&self) -> Uri {
        Uri {
            inner: self.inner.uri(),
        }
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

    /// The window length in bytes.
    #[napi]
    pub fn byte_size(&self) -> u32 {
        self.inner.byte_size() as u32
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
