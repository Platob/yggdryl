//! The `yggdryl.memory` submodule — the in-heap byte source, its cursor/window wrappers, and
//! the seek anchor.
//!
//! Mirrors `yggdryl_core::io::memory`'s [`Heap`](yggdryl_core::io::memory::Heap) source, the
//! [`IOCursor`](yggdryl_core::io::memory::IOCursor) / [`IOSlice`](yggdryl_core::io::memory::IOSlice)
//! wrappers, and the [`Whence`](yggdryl_core::io::memory::Whence) enum. A [`Heap`] is an owned
//! byte buffer with a read/write cursor and `Vec`-like capacity — the concrete in-memory
//! implementor of the byte-access traits (positioned `pread_*` / `pwrite_*` including UTF-8
//! text and the bulk `i32`/`i64` arrays and repeated fills, the cursor stream, bounded
//! [`slice`](Heap::slice) windows, and the source metadata: an addressing `Uri`, a `Headers`
//! map, an `IOMode`, and an `IOKind`). It behaves like a `bytearray`: a mutable value that
//! compares by its stored bytes, round-trips through `serialize_bytes` / `deserialize_bytes`
//! (and pickle), and is deliberately **unhashable**. The on-disk sources live in the
//! `yggdryl.local` submodule (`LocalIO` and the raw `Mmap` — the mapping moved there with the
//! core's `io::local` family).
//!
//! `IOBase` is the **central access path**, so every source is also a node of the IO graph.
//! The in-memory types are **discovery leaves**: `ls()` streams the always-empty
//! [`NoChildren`], `children()` collects nothing, and the `rm` / `rmfile` / `rmdir` family
//! raises the core's guided no-removable-backing refusal. A [`Heap`] is still **addressable**,
//! though: [`join`](Heap::join) (and the `/` operator) composes a child address over an
//! independent buffer and [`parent`](Heap::parent) navigates back, so `name` / `parent` follow
//! the URI (`mem://heap` alone names nothing and has no parent; `mem://heap/logs/app.bin`
//! names `app.bin` and parents `mem://heap/logs`). The `Cursor` / `Slice` byte views are full
//! leaves — no path segment, `parent()` is `None`. DESIGN: the core's generic memory-tree helpers
//! (`tree_byte_size` / `blocks` / `tree_pread_byte_array` / `tree_pwrite_byte_array`) are
//! deliberately **not** mirrored as named methods — they are the internal write-once pattern
//! behind container-node byte access, which the binding reaches through the ordinary byte
//! surface on a directory node (`yggdryl.local.LocalIO`).
//!
//! Every method is one or two lines over `yggdryl_core`; a read with a hard length requirement
//! that runs off the end (a typed read, a slice past the end, a seek before the start) raises a
//! guided `ValueError` carrying the core error text unchanged.

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type
// `From`.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyIndexError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::pybacked::PyBackedBytes;
use pyo3::types::PyBytes;

use crate::datatype_id::DataTypeId;
use crate::headers::Headers;
use crate::io::kind::IOKind;
use crate::io::mode::IOMode;
use crate::mediatype::MediaType;
use crate::mimetype::MimeType;
use crate::uri::Uri;
use yggdryl_core::io::memory::{self, Aggregate, IOBase, IoError};
use yggdryl_core::io::Serializable;

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn ioerr(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// The fail-fast bounds check shared by every bulk `pread_*_array` binding: the guided
/// [`IoError::UnexpectedEof`] when `count` elements of `width` bytes each would run past the
/// `available` bytes, else `None`. Checked **before** the result list is allocated, so a
/// hostile `count` raises instead of attempting a giant allocation (mirrors the inline check
/// the hand-written `i32`/`i64` array readers use).
pub(crate) fn bulk_eof(offset: u64, available: u64, count: usize, width: usize) -> Option<IoError> {
    (count.saturating_mul(width) as u64 > available).then(|| IoError::UnexpectedEof {
        offset: offset + available,
        requested: count.saturating_mul(width),
        available: available as usize,
    })
}

/// Resolves a Python `int` index against a `len`-byte buffer — wrapping a negative index and
/// raising `IndexError` when it falls outside — then reads that single byte via `read` (the
/// source's positioned read). Shared by the byte-buffer `__getitem__` int fast path (a slice
/// key is delegated to `bytes`' own `__getitem__` for exact step/negative semantics).
fn index_one(len: u64, index: isize, read: impl FnOnce(u64, &mut [u8]) -> usize) -> PyResult<u8> {
    let len_i = len as isize;
    let real = if index < 0 { index + len_i } else { index };
    if real < 0 || real >= len_i {
        return Err(PyIndexError::new_err("index out of range"));
    }
    let mut one = [0u8; 1];
    read(real as u64, &mut one);
    Ok(one[0])
}

/// Reads a Python **file-like** object's full contents into a fresh [`memory::Heap`] — the
/// shared type-inferring reader behind [`Heap::from_io`] and [`Cursor::from_io`]. Defensive:
/// tries `getvalue()` (a `io.BytesIO` / `io.StringIO`) first, then `read()` (a `io.FileIO` or
/// any reader), accepting either `bytes` or `str` (encoded UTF-8). When the object exposes
/// `tell()`, the returned heap's cursor is set to that position, so a partially-consumed
/// `BytesIO` transfers its position.
fn heap_from_io(obj: &Bound<'_, PyAny>) -> PyResult<memory::Heap> {
    let raw = match obj.call_method0("getvalue") {
        Ok(value) => value,
        Err(_) => obj.call_method0("read").map_err(|_| {
            PyTypeError::new_err(
                "from_io expects a file-like object with getvalue() or read() (e.g. io.BytesIO, \
                 io.StringIO, io.FileIO)",
            )
        })?,
    };
    let bytes = if let Ok(b) = raw.extract::<Vec<u8>>() {
        b
    } else if let Ok(s) = raw.extract::<String>() {
        s.into_bytes()
    } else {
        return Err(PyTypeError::new_err(
            "from_io: the object's getvalue()/read() must return bytes or str",
        ));
    };
    let mut heap = memory::Heap::from_vec(bytes);
    if let Ok(pos) = obj.call_method0("tell") {
        if let Ok(position) = pos.extract::<u64>() {
            heap.set_position(position);
        }
    }
    Ok(heap)
}

/// Emits a `#[pymethods]` block of scalar positioned `pread_<t>` / `pwrite_<t>` pairs for an
/// `inner`-backed source (`Heap` / `Cursor`) — each a one-line delegation to `yggdryl_core`,
/// completing the native-width set alongside the hand-written `i32` / `i64` / byte accessors.
/// The macro emits the whole `#[pymethods] impl` block so pyo3 processes the expanded methods
/// (the binding's `multiple-pymethods` feature allows the extra block per type).
macro_rules! scalar_methods {
    ($Ty:ty $(, ($t:ty, $pread:ident, $pwrite:ident))+ $(,)?) => {
        #[pymethods]
        impl $Ty {
            $(
                #[doc = concat!("Reads a little-endian `", stringify!($t),
                    "` at `offset`, raising `ValueError` on EOF.")]
                fn $pread(&self, offset: u64) -> PyResult<$t> {
                    self.inner.$pread(offset).map_err(ioerr)
                }
                #[doc = concat!("Writes `value` as a little-endian `", stringify!($t),
                    "` at `offset`, growing as needed.")]
                fn $pwrite(&mut self, offset: u64, value: $t) -> PyResult<()> {
                    self.inner.$pwrite(offset, value).map_err(ioerr)
                }
            )+
        }
    };
}

/// Emits a `#[pymethods]` block of cursor typed `read_<t>` / `write_<t>` pairs for an
/// `inner`-backed source that carries the cursor stream (`Heap` / `Cursor`) — each reads/writes
/// the positioned value at the cursor and advances it, delegating to `yggdryl_core`.
macro_rules! cursor_typed_methods {
    ($Ty:ty $(, ($t:ty, $read:ident, $write:ident))+ $(,)?) => {
        #[pymethods]
        impl $Ty {
            $(
                #[doc = concat!("Reads a little-endian `", stringify!($t),
                    "` at the cursor, advancing it; raising `ValueError` on EOF.")]
                fn $read(&mut self) -> PyResult<$t> {
                    self.inner.$read().map_err(ioerr)
                }
                #[doc = concat!("Writes `value` as a little-endian `", stringify!($t),
                    "` at the cursor, advancing it.")]
                fn $write(&mut self, value: $t) -> PyResult<()> {
                    self.inner.$write(value).map_err(ioerr)
                }
            )+
        }
    };
}

/// Emits a `#[pymethods]` block of the bulk typed `pread_<t>_array` / `pwrite_<t>_array` /
/// `pwrite_<t>_repeat` methods for an `inner`-backed source (`Heap`) — mirroring the existing
/// `u16` array binding, with the element `$width` feeding the fail-fast bounds check before any
/// result is allocated.
macro_rules! bulk_methods {
    ($Ty:ty $(, ($t:ty, $width:literal, $pread:ident, $pwrite:ident, $repeat:ident))+ $(,)?) => {
        #[pymethods]
        impl $Ty {
            $(
                #[doc = concat!("Bulk read of `count` little-endian `", stringify!($t),
                    "`s at `offset` (fail-fast bounds check before allocating).")]
                fn $pread(&self, offset: u64, count: usize) -> PyResult<Vec<$t>> {
                    let available = self.inner.byte_size().saturating_sub(offset);
                    if let Some(e) = bulk_eof(offset, available, count, $width) {
                        return Err(ioerr(e));
                    }
                    let mut values = vec![<$t>::default(); count];
                    self.inner.$pread(offset, &mut values).map_err(ioerr)?;
                    Ok(values)
                }
                #[doc = concat!("Bulk write of little-endian `", stringify!($t),
                    "`s at `offset`, growing as needed.")]
                fn $pwrite(&mut self, offset: u64, values: Vec<$t>) -> PyResult<()> {
                    self.inner.$pwrite(offset, &values).map_err(ioerr)
                }
                #[doc = concat!("Repeated-value fill of `count` little-endian `", stringify!($t),
                    "` copies of `value` at `offset` (no full array is built).")]
                fn $repeat(&mut self, offset: u64, value: $t, count: usize) -> PyResult<()> {
                    self.inner.$repeat(offset, value, count).map_err(ioerr)
                }
            )+
        }
    };
}

/// Emits a `#[pymethods]` block of the [`Aggregate`] reductions (`sum` / `min` / `max` / `mean` /
/// `std` / `first` / `last` / `count_ge`) for one numeric type over an `inner`-backed source —
/// each a one-line delegation to the core `Aggregate` blanket trait. `$acc` is the sum accumulator
/// width (`i32` / `u32` → `i64`, `i64` / `u64` → `i128`, the floats → `f64`).
macro_rules! agg_methods {
    ($Ty:ty $(, ($t:ty, $acc:ty, $sum:ident, $min:ident, $max:ident, $mean:ident, $std:ident,
        $first:ident, $last:ident, $count_ge:ident))+ $(,)?) => {
        #[pymethods]
        impl $Ty {
            $(
                #[doc = concat!("**Sum** of `count` little-endian `", stringify!($t),
                    "`s at `offset` (accumulated as `", stringify!($acc), "`).")]
                fn $sum(&self, offset: u64, count: usize) -> PyResult<$acc> {
                    self.inner.$sum(offset, count).map_err(ioerr)
                }
                #[doc = concat!("**Minimum** of `count` `", stringify!($t),
                    "`s at `offset` (a float min ignores NaN), or `None` when `count == 0`.")]
                fn $min(&self, offset: u64, count: usize) -> PyResult<Option<$t>> {
                    self.inner.$min(offset, count).map_err(ioerr)
                }
                #[doc = concat!("**Maximum** of `count` `", stringify!($t),
                    "`s at `offset` (a float max ignores NaN), or `None` when `count == 0`.")]
                fn $max(&self, offset: u64, count: usize) -> PyResult<Option<$t>> {
                    self.inner.$max(offset, count).map_err(ioerr)
                }
                #[doc = concat!("**Mean** of `count` `", stringify!($t),
                    "`s at `offset` as `float`, or `None` when `count == 0`.")]
                fn $mean(&self, offset: u64, count: usize) -> PyResult<Option<f64>> {
                    self.inner.$mean(offset, count).map_err(ioerr)
                }
                #[doc = concat!("**Population standard deviation** of `count` `", stringify!($t),
                    "`s at `offset` as `float`, or `None` when `count == 0`.")]
                fn $std(&self, offset: u64, count: usize) -> PyResult<Option<f64>> {
                    self.inner.$std(offset, count).map_err(ioerr)
                }
                #[doc = concat!("The **first** `", stringify!($t),
                    "` at `offset`, or `None` when `count == 0`.")]
                fn $first(&self, offset: u64, count: usize) -> PyResult<Option<$t>> {
                    self.inner.$first(offset, count).map_err(ioerr)
                }
                #[doc = concat!("The **last** `", stringify!($t),
                    "` of the `count` at `offset`, or `None` when `count == 0`.")]
                fn $last(&self, offset: u64, count: usize) -> PyResult<Option<$t>> {
                    self.inner.$last(offset, count).map_err(ioerr)
                }
                #[doc = concat!("**Filter count** — how many of `count` `", stringify!($t),
                    "`s at `offset` are `>= threshold`.")]
                fn $count_ge(&self, offset: u64, count: usize, threshold: $t) -> PyResult<usize> {
                    self.inner
                        .$count_ge(offset, count, threshold)
                        .map_err(ioerr)
                }
            )+
        }
    };
}

/// Where a seek offset is measured from — the POSIX `lseek` `whence`. Mirrors
/// [`yggdryl_core::io::memory::Whence`]: the **start** of the data (`SEEK_SET`), the **current**
/// cursor position (`SEEK_CUR`), or the **end** (`SEEK_END`).
#[pyclass(module = "yggdryl.memory", eq, eq_int, hash, frozen)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Whence {
    /// From the start of the data (absolute) — POSIX `SEEK_SET`.
    Start,
    /// From the current cursor position — POSIX `SEEK_CUR`.
    Current,
    /// From the end of the data — POSIX `SEEK_END`.
    End,
}

impl From<Whence> for memory::Whence {
    fn from(whence: Whence) -> Self {
        match whence {
            Whence::Start => memory::Whence::Start,
            Whence::Current => memory::Whence::Current,
            Whence::End => memory::Whence::End,
        }
    }
}

/// An in-heap byte buffer with a read/write cursor and amortized capacity — the concrete
/// in-memory implementor of the byte-access contracts. Grows like a `bytearray`; compares by
/// its stored bytes (the cursor is transient) and is intentionally **not** hashable.
#[pyclass(module = "yggdryl.memory")]
#[derive(Clone)]
pub struct Heap {
    pub(crate) inner: memory::Heap,
}

#[pymethods]
impl Heap {
    /// Builds a buffer owning a copy of `data` (bytes / bytearray), or an empty buffer if
    /// `data` is omitted. The generic, type-inferring entry point (delegates to `from_vec`).
    #[new]
    #[pyo3(signature = (data = None))]
    fn new(data: Option<Vec<u8>>) -> Self {
        match data {
            Some(bytes) => Self {
                inner: memory::Heap::from_vec(bytes),
            },
            None => Self {
                inner: memory::Heap::new(),
            },
        }
    }

    /// An empty buffer that can hold `capacity` bytes before reallocating (like
    /// `bytearray` growth), cursor at `0`.
    #[staticmethod]
    fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: memory::Heap::with_capacity(capacity),
        }
    }

    /// A buffer owning a **copy** of a Python **file-like** object's full contents — the
    /// type-inferring constructor for a `io.BytesIO` / `io.StringIO` / `io.FileIO` (or anything
    /// with `getvalue()` / `read()`). `str` content (a `StringIO`) is encoded UTF-8; when the
    /// object exposes `tell()`, the resulting cursor is set to that position, so a
    /// partially-consumed `BytesIO` transfers its position.
    #[staticmethod]
    fn from_io(obj: &Bound<'_, PyAny>) -> PyResult<Heap> {
        Ok(Heap {
            inner: heap_from_io(obj)?,
        })
    }

    // ---- size + capacity ---------------------------------------------------------------

    /// The total length in bytes.
    fn byte_size(&self) -> u64 {
        self.inner.byte_size()
    }

    /// The total length in bytes (so `len(heap)` works).
    fn __len__(&self) -> usize {
        self.inner.byte_size() as usize
    }

    /// The total length in bits — `byte_size() * 8`.
    fn bit_size(&self) -> u64 {
        self.inner.bit_size()
    }

    /// The number of bytes the buffer can hold before it must reallocate — like
    /// `list`/`Vec` capacity.
    fn capacity(&self) -> u64 {
        self.inner.capacity()
    }

    /// Reserves capacity for at least `additional` more bytes past the current size,
    /// amortizing later writes.
    fn reserve(&mut self, additional: u64) {
        self.inner.reserve(additional);
    }

    /// The spare room already allocated — `capacity() - byte_size()`, the bytes that can be
    /// appended before the next reallocation.
    fn spare_capacity(&self) -> u64 {
        self.inner.spare_capacity()
    }

    /// Reserves capacity for **exactly** `additional` more bytes — no amortized
    /// over-allocation, for a caller that knows the final size.
    fn reserve_exact(&mut self, additional: u64) {
        self.inner.reserve_exact(additional);
    }

    /// **Checked** reservation: where `reserve` would abort the process on overflow or
    /// allocator failure, this raises a guided `ValueError` instead.
    fn try_reserve(&mut self, additional: u64) -> PyResult<()> {
        self.inner.try_reserve(additional).map_err(ioerr)
    }

    /// **Checked exact** reservation — `try_reserve` without the amortized over-allocation.
    fn try_reserve_exact(&mut self, additional: u64) -> PyResult<()> {
        self.inner.try_reserve_exact(additional).map_err(ioerr)
    }

    /// Ensures the **total** capacity is at least `total` bytes (the absolute-target form of
    /// `reserve`); a no-op when already satisfied, never shrinks.
    fn ensure_capacity(&mut self, total: u64) {
        self.inner.ensure_capacity(total);
    }

    /// **Checked** `ensure_capacity` — raises a guided `ValueError` instead of aborting.
    fn try_ensure_capacity(&mut self, total: u64) -> PyResult<()> {
        self.inner.try_ensure_capacity(total).map_err(ioerr)
    }

    /// Releases spare capacity back to the allocator, shrinking toward `byte_size()`.
    fn shrink_to_fit(&mut self) {
        self.inner.shrink_to_fit();
    }

    /// Shrinks the allocation toward `min_capacity` (never below `byte_size()`).
    fn shrink_to(&mut self, min_capacity: u64) {
        self.inner.shrink_to(min_capacity);
    }

    /// Sets the byte length to exactly `length` — shrinking (dropping the tail) or extending
    /// (zero-filling) — then syncs the size headers. The cursor is clamped to stay within the
    /// data.
    fn truncate(&mut self, length: u64) -> PyResult<()> {
        self.inner.truncate(length).map_err(ioerr)
    }

    /// The content length in bytes, **preferring the cached `Content-Length` header** when
    /// present and falling back to the live `byte_size()`.
    fn content_length(&self) -> u64 {
        self.inner.content_length()
    }

    /// Whether the buffer holds no bytes (`byte_size() == 0`).
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Truthiness — `True` when the buffer holds at least one byte (like `bytearray`).
    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    // ---- positioned byte-array ---------------------------------------------------------

    /// **Positioned read.** Returns up to `length` bytes starting at `offset` as `bytes` —
    /// short near the end, empty at or past it. Never moves the cursor. Reads **directly**
    /// into the `bytes` allocation (one copy).
    fn pread_byte_array<'py>(
        &self,
        py: Python<'py>,
        offset: u64,
        length: usize,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let n = self
            .inner
            .byte_size()
            .saturating_sub(offset)
            .min(length as u64) as usize;
        PyBytes::new_bound_with(py, n, |dst| {
            self.inner.pread_byte_array(offset, dst);
            Ok(())
        })
    }

    /// **Positioned write.** Copies `data` (bytes / bytearray) in at `offset`, growing the
    /// buffer and zero-filling any gap; returns the number of bytes written.
    fn pwrite_byte_array(&mut self, offset: u64, data: PyBackedBytes) -> usize {
        self.inner.pwrite_byte_array(offset, &data)
    }

    // ---- positioned typed accessors ----------------------------------------------------

    /// Reads the single byte at `offset`, raising `ValueError` if it is past the end.
    fn pread_byte(&self, offset: u64) -> PyResult<u8> {
        self.inner.pread_byte(offset).map_err(ioerr)
    }

    /// Writes the single byte `value` at `offset`, growing the buffer as needed.
    fn pwrite_byte(&mut self, offset: u64, value: u8) -> PyResult<()> {
        self.inner.pwrite_byte(offset, value).map_err(ioerr)
    }

    /// Reads the bit at absolute **bit** `offset` (LSB-first: bit `offset % 8` of byte
    /// `offset / 8`), raising `ValueError` if its byte is past the end.
    fn pread_bit(&self, offset: u64) -> PyResult<bool> {
        self.inner.pread_bit(offset).map_err(ioerr)
    }

    /// Sets or clears the bit at absolute **bit** `offset` (LSB-first), growing the buffer
    /// (zero-filled) if the bit is past the end.
    fn pwrite_bit(&mut self, offset: u64, value: bool) -> PyResult<()> {
        self.inner.pwrite_bit(offset, value).map_err(ioerr)
    }

    /// Reads a little-endian `i32` (4 bytes) at `offset`, raising `ValueError` on EOF.
    fn pread_i32(&self, offset: u64) -> PyResult<i32> {
        self.inner.pread_i32(offset).map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at `offset`, growing as needed.
    fn pwrite_i32(&mut self, offset: u64, value: i32) -> PyResult<()> {
        self.inner.pwrite_i32(offset, value).map_err(ioerr)
    }

    /// Reads a little-endian `i64` (8 bytes) at `offset`, raising `ValueError` on EOF.
    fn pread_i64(&self, offset: u64) -> PyResult<i64> {
        self.inner.pread_i64(offset).map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at `offset`, growing as needed.
    fn pwrite_i64(&mut self, offset: u64, value: i64) -> PyResult<()> {
        self.inner.pwrite_i64(offset, value).map_err(ioerr)
    }

    /// Reads up to `length` **bytes** at `offset` and decodes them as UTF-8 text (clamped
    /// near the end), raising a guided `ValueError` on invalid UTF-8 — including a
    /// multi-byte character cut by the range.
    fn pread_utf8(&self, offset: u64, length: usize) -> PyResult<String> {
        self.inner.pread_utf8(offset, length).map_err(ioerr)
    }

    /// Writes `text`'s UTF-8 bytes at `offset` (growing as needed); returns the number of
    /// **bytes** written.
    fn pwrite_utf8(&mut self, offset: u64, text: &str) -> usize {
        self.inner.pwrite_utf8(offset, text)
    }

    // ---- bulk typed arrays + repeated fills ----------------------------------------------

    /// **Bulk typed read.** Returns `count` little-endian `i32`s starting at `offset` as a
    /// list, raising `ValueError` if fewer bytes remain — checked **before** the result is
    /// allocated, so a hostile `count` fails fast instead of allocating.
    fn pread_i32_array(&self, offset: u64, count: usize) -> PyResult<Vec<i32>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if count.saturating_mul(4) as u64 > available {
            return Err(ioerr(IoError::UnexpectedEof {
                offset: offset + available,
                requested: count.saturating_mul(4),
                available: available as usize,
            }));
        }
        let mut values = vec![0i32; count];
        self.inner
            .pread_i32_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write.** Writes all of `values` as little-endian `i32`s at `offset`,
    /// growing as needed.
    fn pwrite_i32_array(&mut self, offset: u64, values: Vec<i32>) -> PyResult<()> {
        self.inner.pwrite_i32_array(offset, &values).map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `i64`s — the wide counterpart of
    /// [`pread_i32_array`](Heap::pread_i32_array), with the same fail-fast bounds check
    /// before the result is allocated.
    fn pread_i64_array(&self, offset: u64, count: usize) -> PyResult<Vec<i64>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if count.saturating_mul(8) as u64 > available {
            return Err(ioerr(IoError::UnexpectedEof {
                offset: offset + available,
                requested: count.saturating_mul(8),
                available: available as usize,
            }));
        }
        let mut values = vec![0i64; count];
        self.inner
            .pread_i64_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `i64`s — the wide counterpart of
    /// [`pwrite_i32_array`](Heap::pwrite_i32_array).
    fn pwrite_i64_array(&mut self, offset: u64, values: Vec<i64>) -> PyResult<()> {
        self.inner.pwrite_i64_array(offset, &values).map_err(ioerr)
    }

    /// **Repeated-value fill.** Writes `count` copies of the byte `value` at `offset`
    /// (growing as needed) without ever materializing the full array — the `memset` of the
    /// family.
    fn pwrite_byte_repeat(&mut self, offset: u64, value: u8, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_byte_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `i32` copies of `value` at `offset` —
    /// no full array is built.
    fn pwrite_i32_repeat(&mut self, offset: u64, value: i32, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_i32_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `i64` copies of `value` at `offset` —
    /// the wide counterpart of [`pwrite_i32_repeat`](Heap::pwrite_i32_repeat).
    fn pwrite_i64_repeat(&mut self, offset: u64, value: i64, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_i64_repeat(offset, value, count)
            .map_err(ioerr)
    }

    // ---- bulk unsigned + floating widths (u16/u32/u64/f32/f64) --------------------------

    /// **Bulk typed read** of `count` little-endian `u16`s — the `u16` counterpart of
    /// [`pread_i32_array`](Heap::pread_i32_array), with the same fail-fast bounds check.
    fn pread_u16_array(&self, offset: u64, count: usize) -> PyResult<Vec<u16>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if let Some(e) = bulk_eof(offset, available, count, 2) {
            return Err(ioerr(e));
        }
        let mut values = vec![0u16; count];
        self.inner
            .pread_u16_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `u16`s at `offset`, growing as needed.
    fn pwrite_u16_array(&mut self, offset: u64, values: Vec<u16>) -> PyResult<()> {
        self.inner.pwrite_u16_array(offset, &values).map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `u16` copies of `value` at `offset`.
    fn pwrite_u16_repeat(&mut self, offset: u64, value: u16, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_u16_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `u32`s (fail-fast bounds check).
    fn pread_u32_array(&self, offset: u64, count: usize) -> PyResult<Vec<u32>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if let Some(e) = bulk_eof(offset, available, count, 4) {
            return Err(ioerr(e));
        }
        let mut values = vec![0u32; count];
        self.inner
            .pread_u32_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `u32`s at `offset`, growing as needed.
    fn pwrite_u32_array(&mut self, offset: u64, values: Vec<u32>) -> PyResult<()> {
        self.inner.pwrite_u32_array(offset, &values).map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `u32` copies of `value` at `offset`.
    fn pwrite_u32_repeat(&mut self, offset: u64, value: u32, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_u32_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `u64`s (fail-fast bounds check).
    fn pread_u64_array(&self, offset: u64, count: usize) -> PyResult<Vec<u64>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if let Some(e) = bulk_eof(offset, available, count, 8) {
            return Err(ioerr(e));
        }
        let mut values = vec![0u64; count];
        self.inner
            .pread_u64_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `u64`s at `offset`, growing as needed.
    fn pwrite_u64_array(&mut self, offset: u64, values: Vec<u64>) -> PyResult<()> {
        self.inner.pwrite_u64_array(offset, &values).map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `u64` copies of `value` at `offset`.
    fn pwrite_u64_repeat(&mut self, offset: u64, value: u64, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_u64_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `f32`s (fail-fast bounds check).
    fn pread_f32_array(&self, offset: u64, count: usize) -> PyResult<Vec<f32>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if let Some(e) = bulk_eof(offset, available, count, 4) {
            return Err(ioerr(e));
        }
        let mut values = vec![0f32; count];
        self.inner
            .pread_f32_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `f32`s at `offset`, growing as needed.
    fn pwrite_f32_array(&mut self, offset: u64, values: Vec<f32>) -> PyResult<()> {
        self.inner.pwrite_f32_array(offset, &values).map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `f32` copies of `value` at `offset`.
    fn pwrite_f32_repeat(&mut self, offset: u64, value: f32, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_f32_repeat(offset, value, count)
            .map_err(ioerr)
    }

    /// **Bulk typed read** of `count` little-endian `f64`s (fail-fast bounds check).
    fn pread_f64_array(&self, offset: u64, count: usize) -> PyResult<Vec<f64>> {
        let available = self.inner.byte_size().saturating_sub(offset);
        if let Some(e) = bulk_eof(offset, available, count, 8) {
            return Err(ioerr(e));
        }
        let mut values = vec![0f64; count];
        self.inner
            .pread_f64_array(offset, &mut values)
            .map_err(ioerr)?;
        Ok(values)
    }

    /// **Bulk typed write** of little-endian `f64`s at `offset`, growing as needed.
    fn pwrite_f64_array(&mut self, offset: u64, values: Vec<f64>) -> PyResult<()> {
        self.inner.pwrite_f64_array(offset, &values).map_err(ioerr)
    }

    /// **Repeated-value fill** of `count` little-endian `f64` copies of `value` at `offset`.
    fn pwrite_f64_repeat(&mut self, offset: u64, value: f64, count: usize) -> PyResult<()> {
        self.inner
            .pwrite_f64_repeat(offset, value, count)
            .map_err(ioerr)
    }

    // ---- cross-source copy -------------------------------------------------------------

    /// Overwrites this heap with **all of `src`'s bytes** (truncating to match) — a
    /// cross-source copy. Returns the byte count.
    fn copy_from(&mut self, src: &Heap) -> PyResult<u64> {
        self.inner.copy_from(&src.inner).map_err(ioerr)
    }

    /// **Positioned cross-source write**: copies `length` bytes of `src` starting at
    /// `src_offset` into this heap at `offset`. Returns the number of bytes transferred
    /// (short at the end of `src`).
    fn pwrite_from(
        &mut self,
        offset: u64,
        src: &Heap,
        src_offset: u64,
        length: u64,
    ) -> PyResult<u64> {
        self.inner
            .pwrite_from(offset, &src.inner, src_offset, length)
            .map_err(ioerr)
    }

    // ---- element type + transforms -----------------------------------------------------

    /// The source's declared **element** [`DataTypeId`] — read from its headers
    /// (`TYPE_ID`), or [`DataTypeId.Unknown`](DataTypeId::Unknown) when none is set.
    fn dtype(&self) -> DataTypeId {
        self.inner.dtype().into()
    }

    /// Declares the source's element [`DataTypeId`] in its headers (so `dtype` /
    /// `element_count` report it). [`Unknown`](DataTypeId::Unknown) clears it.
    fn set_dtype(&mut self, dtype: DataTypeId) {
        self.inner.set_dtype(dtype.into());
    }

    /// How many whole [`dtype`](Heap::dtype) elements the source currently holds —
    /// `byte_size() / dtype.byte_size()`, or `0` when the type is unknown.
    fn element_count(&self) -> u64 {
        self.inner.element_count()
    }

    /// **Widens or shrinks the element type, returning a fresh converted [`Heap`]** — reinterprets
    /// every stored element from the current [`dtype`](Heap::dtype) to `to` at the new width (`i64`
    /// → `i32`, `i32` → `f64`, …), leaving `self` untouched. Raises a guided `ValueError` when
    /// either side has no known element type (call `set_dtype` first).
    fn resize_dtype(&self, to: DataTypeId) -> PyResult<Heap> {
        self.inner
            .resize_dtype(to.into())
            .map(|inner| Heap { inner })
            .map_err(ioerr)
    }

    /// **Widens or shrinks the element type in place** — rewrites `self`'s bytes at the new width
    /// and updates the `Elem-Type-Id` header, returning the element count. A narrowing integer
    /// target saturates, a float target rounds. Raises a guided `ValueError` when either side has
    /// no known element type.
    fn resize_dtype_in_place(&mut self, to: DataTypeId) -> PyResult<u64> {
        self.inner.resize_dtype_in_place(to.into()).map_err(ioerr)
    }

    /// **Selects elements by a bitmask, returning a fresh compacted [`Heap`]** — keeps each
    /// `dtype`-width element whose corresponding **bit is set** in `mask` (another [`Heap`] read as
    /// an LSB-first bit buffer: bit `i` selects element `i`), dropping the rest and leaving `self`
    /// untouched. Raises a guided `ValueError` when `self` has no element type or `mask` has fewer
    /// bits than elements.
    fn mask_filter(&self, mask: &Heap) -> PyResult<Heap> {
        self.inner
            .mask_filter(&mask.inner)
            .map(|inner| Heap { inner })
            .map_err(ioerr)
    }

    /// **Selects elements by a bitmask in place** — forward-compacts the kept `dtype`-width
    /// elements to the front of `self` and truncates to the kept length, returning the kept element
    /// count. Raises a guided `ValueError` when `self` has no element type or `mask` has fewer bits
    /// than elements.
    fn mask_filter_in_place(&mut self, mask: &Heap) -> PyResult<u64> {
        self.inner.mask_filter_in_place(&mask.inner).map_err(ioerr)
    }

    // ---- cursor ------------------------------------------------------------------------

    /// The current cursor position (bytes from the start). May sit past the end after a seek.
    #[getter]
    fn position(&self) -> u64 {
        self.inner.position()
    }

    /// Moves the cursor to an absolute `position` (past the end is allowed).
    fn set_position(&mut self, position: u64) {
        self.inner.set_position(position);
    }

    /// Seeks to `whence + offset` and returns the new position. A position past the end is
    /// allowed; seeking before the start raises `ValueError`.
    fn seek(&mut self, whence: Whence, offset: i64) -> PyResult<u64> {
        self.inner.seek(whence.into(), offset).map_err(ioerr)
    }

    /// Resets the cursor to the start.
    fn rewind(&mut self) {
        self.inner.rewind();
    }

    /// **Cursor read.** Returns up to `length` bytes from the current position (short near the
    /// end), advancing the cursor by the number read.
    fn read<'py>(&mut self, py: Python<'py>, length: usize) -> PyResult<Bound<'py, PyBytes>> {
        let position = self.inner.position();
        let n = self
            .inner
            .byte_size()
            .saturating_sub(position)
            .min(length as u64) as usize;
        let bytes = PyBytes::new_bound_with(py, n, |dst| {
            self.inner.pread_byte_array(position, dst);
            Ok(())
        })?;
        self.inner.set_position(position + n as u64);
        Ok(bytes)
    }

    /// **Cursor write.** Writes `data` (bytes / bytearray) at the current position, advancing
    /// the cursor by the number written (growing the buffer as needed); returns that count.
    fn write(&mut self, data: PyBackedBytes) -> usize {
        self.inner.write(&data)
    }

    /// Reads the next byte at the cursor, advancing it by 1, raising `ValueError` at the end.
    fn read_byte(&mut self) -> PyResult<u8> {
        self.inner.read_byte().map_err(ioerr)
    }

    /// Writes the byte `value` at the cursor, advancing it by 1.
    fn write_byte(&mut self, value: u8) -> PyResult<()> {
        self.inner.write_byte(value).map_err(ioerr)
    }

    /// Reads a little-endian `i32` (4 bytes) at the cursor, advancing it by 4, raising
    /// `ValueError` on EOF.
    fn read_i32(&mut self) -> PyResult<i32> {
        self.inner.read_i32().map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at the cursor, advancing it by 4.
    fn write_i32(&mut self, value: i32) -> PyResult<()> {
        self.inner.write_i32(value).map_err(ioerr)
    }

    /// Reads a little-endian `i64` (8 bytes) at the cursor, advancing it by 8, raising
    /// `ValueError` on EOF.
    fn read_i64(&mut self) -> PyResult<i64> {
        self.inner.read_i64().map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at the cursor, advancing it by 8.
    fn write_i64(&mut self, value: i64) -> PyResult<()> {
        self.inner.write_i64(value).map_err(ioerr)
    }

    /// Reads up to `length` **bytes** from the cursor and decodes them as UTF-8 text (clamped
    /// near the end), advancing the cursor by the bytes read, raising a guided `ValueError`
    /// on invalid UTF-8 (leaving the cursor put).
    fn read_utf8(&mut self, length: usize) -> PyResult<String> {
        self.inner.read_utf8(length).map_err(ioerr)
    }

    /// Writes `text`'s UTF-8 bytes at the cursor, advancing it; returns the number of
    /// **bytes** written.
    fn write_utf8(&mut self, text: &str) -> usize {
        self.inner.write_utf8(text)
    }

    /// Reads from the current position **to the end** as `bytes`, advancing the cursor to the
    /// end.
    fn read_to_end<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let position = self.inner.position();
        let n = self.inner.byte_size().saturating_sub(position) as usize;
        let bytes = PyBytes::new_bound_with(py, n, |dst| {
            self.inner.pread_byte_array(position, dst);
            Ok(())
        })?;
        self.inner.set_position(self.inner.byte_size());
        Ok(bytes)
    }

    /// **Reads one line** from the cursor — the content up to the next line terminator with the
    /// trailing `\n` / `\r\n` **stripped**, decoded as UTF-8 — advancing the cursor past the
    /// terminator. **CSV-aware**: a `\n` inside a double-quoted field does not end the line. A
    /// blank line returns `""` but **advances**; at the true end it returns `""` **without**
    /// advancing (that is how iteration stops).
    fn readline(&mut self) -> PyResult<String> {
        self.inner.readline().map_err(ioerr)
    }

    /// **Reads every remaining line** from the cursor into a list, advancing it to the end — each
    /// element has its trailing line terminator stripped (blank lines kept, quoted newlines
    /// honored).
    fn readlines(&mut self) -> PyResult<Vec<String>> {
        self.inner.readlines().map_err(ioerr)
    }

    // ---- slice -------------------------------------------------------------------------

    /// The window `[offset, offset + length)` as a fresh, independent `Heap` addressed from
    /// its own `0`. Raises `ValueError` if it runs past the end.
    fn slice(&self, offset: u64, length: u64) -> PyResult<Heap> {
        self.inner
            .slice(offset, length)
            .map(|inner| Heap { inner })
            .map_err(ioerr)
    }

    // ---- address (uri) -----------------------------------------------------------------

    /// The [`Uri`] that **addresses** this heap — the stable synthetic `mem://heap` for an
    /// anonymous buffer (which stores no address), or the composed address a heap built by
    /// [`join`](Heap::join) carries (`mem://heap/logs/app.bin`).
    #[getter]
    fn uri(&self) -> Uri {
        Uri {
            inner: self.inner.uri(),
        }
    }

    // ---- metadata (headers / mode / kind) ------------------------------------------------

    /// The [`Headers`] metadata attached to this heap — returned as an owned **copy** (the
    /// binding cannot borrow into the Rust value); mutate the copy and write it back with
    /// [`set_headers`](Heap::set_headers).
    #[getter]
    fn headers(&self) -> Headers {
        Headers {
            inner: self.inner.headers().clone(),
        }
    }

    /// Replaces the whole [`Headers`] metadata map in place.
    fn set_headers(&mut self, headers: &Headers) {
        self.inner.set_headers(headers.inner.clone());
    }

    /// Returns a copy of this heap with its [`Headers`] metadata replaced.
    fn with_headers(&self, headers: &Headers) -> Heap {
        Heap {
            inner: self.inner.clone().with_headers(headers.inner.clone()),
        }
    }

    /// How this heap may be accessed — [`IOMode.ReadWrite`](IOMode::ReadWrite) by default
    /// (it is in-memory).
    #[getter]
    fn mode(&self) -> IOMode {
        self.inner.mode().into()
    }

    /// Sets the access [`IOMode`] in place.
    fn set_mode(&mut self, mode: IOMode) {
        self.inner.set_mode(mode.into());
    }

    /// Returns a copy of this heap with its access [`IOMode`] set.
    fn with_mode(&self, mode: IOMode) -> Heap {
        Heap {
            inner: self.inner.clone().with_mode(mode.into()),
        }
    }

    /// What this source **is** — always [`IOKind.Heap`](IOKind::Heap).
    #[getter]
    fn kind(&self) -> IOKind {
        self.inner.kind().into()
    }

    // ---- predicates (is_file / is_dir / exists) ------------------------------------------

    /// Whether this source is a regular **file** — derived from [`kind`](Heap::kind); always
    /// `False` for a heap.
    fn is_file(&self) -> bool {
        self.inner.is_file()
    }

    /// Whether this source is a **directory** — derived from [`kind`](Heap::kind); always
    /// `False` for a heap.
    fn is_dir(&self) -> bool {
        self.inner.is_dir()
    }

    /// Whether the source **exists** — a live in-memory buffer always exists (`True`),
    /// although it is neither file nor directory.
    fn exists(&self) -> bool {
        self.inner.exists()
    }

    // ---- media type (declared headers, else the address, else octet-stream) --------------

    /// The **primary** [`MimeType`](crate::mimetype::MimeType) of this source: the
    /// `Content-Type` its [`headers`](Heap::headers) declare, else inferred from the
    /// [`uri`](Heap::uri)'s file name, else the `application/octet-stream` fallback — always an
    /// answer.
    fn mime_type(&self) -> MimeType {
        MimeType {
            inner: self.inner.mime_type(),
        }
    }

    /// The full [`MediaType`](crate::mediatype::MediaType) of this source: the media the
    /// `Content-Type` / `Content-Encoding` [`headers`](Heap::headers) declare, else inferred
    /// from the [`uri`](Heap::uri)'s extensions, else the single `application/octet-stream`
    /// fallback.
    fn media_type(&self) -> MediaType {
        MediaType {
            inner: self.inner.media_type(),
        }
    }

    /// Resolves the media type **and stores it** in this source's headers when `Content-Type`
    /// is not already set — memoizing the inference so later reads come straight from
    /// [`headers`](Heap::headers). Returns the effective [`MimeType`](crate::mimetype::MimeType).
    fn ensure_content_type(&mut self) -> MimeType {
        MimeType {
            inner: self.inner.ensure_content_type(),
        }
    }

    // ---- inference + compression (magic-inferred type; codec over the bytes) -------------

    /// The **primary** [`MimeType`](crate::mimetype::MimeType) inferred from this source's
    /// **magic bytes** — a positioned read of the head that **never moves the cursor**, so it
    /// works mid-stream; falls back to the declared/address [`mime_type`](Heap::mime_type) when
    /// no magic matches.
    fn infer_mime_type(&self) -> MimeType {
        MimeType {
            inner: self.inner.infer_mime_type(),
        }
    }

    /// The full [`MediaType`](crate::mediatype::MediaType) inferred by **recursive magic** — the
    /// head's type, then the type inside each compression layer it can peel (a gzipped tar reads
    /// as `[application/gzip, application/x-tar]`). The head is read positioned (no cursor seek).
    fn infer_media_type(&self) -> MediaType {
        MediaType {
            inner: self.inner.infer_media_type(),
        }
    }

    /// The `yggdryl.compression` codec for this source's media type (headers, else address),
    /// or `None` when the type is not a supported compression.
    fn compression(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        crate::compression::codec_to_object(py, self.inner.compression())
    }

    /// This source **decompressed** with the codec inferred from its **media type**, as
    /// `bytes` — raises a guided `ValueError` when the source is not a supported compression.
    fn decompress<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.decompress().map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// This source's whole content **compressed** with the explicit `codec` (a
    /// `yggdryl.compression` codec), as `bytes`.
    fn compress_with<'py>(
        &self,
        py: Python<'py>,
        codec: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let out = crate::compression::with_codec(codec, |c| self.inner.compressed_with(c))?
            .map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// This source's whole content **decompressed** with the explicit `codec`, as `bytes` —
    /// raises a guided `ValueError` on a corrupt stream.
    fn decompress_with<'py>(
        &self,
        py: Python<'py>,
        codec: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let out = crate::compression::with_codec(codec, |c| self.inner.decompressed_with(c))?
            .map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    // DESIGN: the byte-returning `decompress` / `compress_with` / `decompress_with` are the
    // ergonomic mirror of the core's compress/decompress family; the generic `compress_into` /
    // `decompress_into` (source-to-source) and `as_bytes` are deliberately not exposed.

    /// **Compresses this heap in place** — replaces its bytes with the compressed form and
    /// updates the `Content-Type` / `Content-Length` / `mtime` headers. `codec` (a
    /// `yggdryl.compression` codec) defaults to the codec of the heap's own media type, so a
    /// `.gz`-addressed heap packs itself gzip; pass an explicit one to override. Raises a
    /// guided `ValueError` when no codec applies.
    #[pyo3(signature = (codec = None))]
    fn compress_in_place(&mut self, codec: Option<&Bound<'_, PyAny>>) -> PyResult<()> {
        match codec {
            Some(codec) => {
                crate::compression::with_codec(codec, |c| self.inner.compress_in_place(Some(c)))?
                    .map_err(ioerr)
            }
            None => self.inner.compress_in_place(None).map_err(ioerr),
        }
    }

    /// **Decompresses this heap in place** — replaces its compressed bytes with the plain
    /// content (codec inferred from its media type) and updates the size/media/mtime headers.
    /// Raises a guided `ValueError` when the heap is not a supported compression.
    fn decompress_in_place(&mut self) -> PyResult<()> {
        self.inner.decompress_in_place().map_err(ioerr)
    }

    // ---- graph: navigation + discovery + CRUD (a heap is a leaf) -------------------------

    /// The node's own name — the last (percent-decoded) segment of its address's path: empty
    /// for an anonymous heap (the synthetic `mem://heap` has no path segment), the joined leaf
    /// name for an addressed one (`mem://heap/logs/app.bin` → `app.bin`).
    #[getter]
    fn name(&self) -> String {
        self.inner.name()
    }

    /// The parent node, or `None` — the inverse of [`join`](Heap::join): an addressed heap
    /// (`mem://heap/logs/app.bin`) reports its directory address (`mem://heap/logs`), a bare
    /// `mem://heap` root reports `None`. (A heap is a **leaf** for *discovery* — it streams no
    /// children — but it is still addressable, so navigation composes through the URI.)
    fn parent(&self) -> Option<Heap> {
        self.inner.parent().map(|inner| Heap { inner })
    }

    /// This node's **ancestors** as a list, nearest first — the repeated
    /// [`parent`](Heap::parent) chain up to the `mem://heap` root (empty for a bare root). The
    /// node-graph counterpart of [`Uri.parents`](crate::uri::Uri::parents).
    fn parents(&self) -> Vec<Heap> {
        self.inner.parents().map(|inner| Heap { inner }).collect()
    }

    /// The child node at `segment` — a **new, independent in-memory buffer** whose address is
    /// composed by joining `segment` onto this heap's URI (`Uri.joinpath`), so
    /// `child.parent()` addresses this node again. `segment` may be multi-segment (`"a/b/c"`),
    /// and a spaced segment percent-encodes in the address (`"my dir/f"` →
    /// `mem://heap/my%20dir/f`). Pure address algebra — the child owns no bytes yet, and
    /// writing it never touches this heap.
    fn join(&self, segment: &str) -> PyResult<Heap> {
        self.inner
            .join(segment)
            .map(|inner| Heap { inner })
            .map_err(ioerr)
    }

    /// `heap / "logs/app.bin"` — the operator spelling of [`join`](Heap::join).
    fn __truediv__(&self, segment: &str) -> PyResult<Heap> {
        self.join(segment)
    }

    /// Streams this node's children — always the empty [`NoChildren`] stream (a heap is
    /// a leaf: it streams nothing, with or without `recursive=True`), still satisfying the
    /// iterator protocol like `yggdryl.local.LocalIO.ls`.
    #[pyo3(signature = (recursive = false))]
    fn ls(&self, recursive: bool) -> PyResult<NoChildren> {
        let _ = if recursive {
            self.inner.ls_recursive()
        } else {
            self.inner.ls()
        }
        .map_err(ioerr)?;
        Ok(NoChildren {})
    }

    /// The direct children, collected — always the empty list (a leaf has none).
    fn children(&self) -> PyResult<Vec<Heap>> {
        let nodes = self.inner.children().map_err(ioerr)?;
        Ok(nodes.into_iter().map(|inner| Heap { inner }).collect())
    }

    /// A heap has no removable backing — raises the guided `ValueError` naming the fix
    /// (address a filesystem node, e.g. `yggdryl.local.LocalIO`, instead). `exist_ok` governs
    /// a **missing** node (`exist_ok=False` raises on a missing one); a heap has no backing at
    /// all, so it always raises the same guided refusal regardless.
    #[pyo3(signature = (exist_ok = true))]
    fn rm(&self, exist_ok: bool) -> PyResult<()> {
        self.inner.rm(exist_ok).map_err(ioerr)
    }

    /// A heap has no removable backing — the same guided refusal as [`rm`](Heap::rm).
    #[pyo3(signature = (exist_ok = true))]
    fn rmfile(&self, exist_ok: bool) -> PyResult<()> {
        self.inner.rmfile(exist_ok).map_err(ioerr)
    }

    /// A heap has no removable backing — the same guided refusal as [`rm`](Heap::rm).
    #[pyo3(signature = (exist_ok = true))]
    fn rmdir(&self, exist_ok: bool) -> PyResult<()> {
        self.inner.rmdir(exist_ok).map_err(ioerr)
    }

    // ---- cursor / window views ---------------------------------------------------------

    /// A [`Cursor`] over an **independent copy** of this heap (the binding clones since it
    /// cannot consume `self`), positioned at the start.
    fn cursor(&self) -> Cursor {
        Cursor {
            inner: self.inner.clone().cursor(),
        }
    }

    /// A [`Slice`] — the bounded window `[offset, offset + length)` over an **independent
    /// copy** of this heap, addressed from its own `0`. Raises `ValueError` if it runs past
    /// the end.
    fn window(&self, offset: u64, length: u64) -> PyResult<Slice> {
        self.inner
            .clone()
            .window(offset, length)
            .map(|inner| Slice { inner })
            .map_err(ioerr)
    }

    // ---- value semantics + dunders -----------------------------------------------------

    /// The stored bytes as a `bytes` copy.
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_slice())
    }

    /// The stored bytes as a `bytes` copy (so `bytes(heap)` works).
    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.as_slice())
    }

    /// Indexing like `bytes` — `heap[i]` is the byte at `i` as an `int` (negative indices wrap;
    /// out of range raises `IndexError`), and `heap[a:b:step]` is the selected `bytes`.
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<PyObject> {
        if let Ok(index) = key.extract::<isize>() {
            let byte = index_one(self.inner.byte_size(), index, |offset, buf| {
                self.inner.pread_byte_array(offset, buf)
            })?;
            Ok(byte.to_object(py))
        } else {
            // A slice (or a bad key): delegate to `bytes`' own `__getitem__` for exact semantics.
            Ok(PyBytes::new_bound(py, self.inner.as_slice())
                .as_any()
                .get_item(key)?
                .unbind())
        }
    }

    /// The heap's value form — its stored bytes (the cursor, address, headers, and mode are
    /// transient metadata and are not serialized), matching the identity `__eq__` uses.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a heap from bytes produced by [`serialize_bytes`](Heap::serialize_bytes)
    /// — the exact inverse (cursor at `0`, default address/metadata).
    #[staticmethod]
    fn deserialize_bytes(data: &[u8]) -> PyResult<Heap> {
        memory::Heap::deserialize_bytes(data)
            .map(|inner| Heap { inner })
            .map_err(ioerr)
    }

    /// Pickles through the byte codec (`deserialize_bytes(serialize_bytes())`).
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<Heap>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    /// An explicit copy of this buffer (equivalent to `copy.copy(heap)`) — bytes, cursor,
    /// headers, and mode all copied.
    fn copy(&self) -> Self {
        self.clone()
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    /// Context-manager entry — returns the heap itself, so `with Heap(data) as h:` binds it.
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Context-manager exit — a no-op for an in-memory buffer (nothing to release); returns
    /// `False` so exceptions propagate.
    fn __exit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        false
    }

    /// Line iteration — `for line in heap:` yields each line from the cursor (like a file
    /// object), via [`readline`](Heap::readline).
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// The next line from the cursor, or `StopIteration` at the true end — a line that advanced
    /// the cursor (including a blank line, yielded as `""`), stopping only when `readline`
    /// consumes nothing.
    fn __next__(&mut self) -> PyResult<Option<String>> {
        let start = self.inner.position();
        let line = self.inner.readline().map_err(ioerr)?;
        Ok((self.inner.position() != start).then_some(line))
    }

    fn __repr__(&self) -> String {
        format!("Heap(<{} bytes>)", self.inner.byte_size())
    }
}

#[pymethods]
impl Heap {
    /// **Moves** this heap's whole content into `dst` (another `Heap`) and **empties this
    /// heap** — a copy that consumes its origin (`mv` over the byte contract). Returns the
    /// number of bytes moved. A no-op when `self` and `dst` resolve to the same real address
    /// (anonymous `mem://heap` buffers still move); a bare heap has no removable backing, so it
    /// simply ends empty.
    fn move_into(&mut self, mut dst: PyRefMut<'_, Heap>) -> PyResult<u64> {
        self.inner.move_into(&mut dst.inner).map_err(ioerr)
    }
}

// The remaining native-width scalar, cursor-typed, and bulk-array accessors — completing the
// set alongside the hand-written `i32` / `i64` / byte forms in the main block above.
scalar_methods!(
    Heap,
    (i8, pread_i8, pwrite_i8),
    (u8, pread_u8, pwrite_u8),
    (i16, pread_i16, pwrite_i16),
    (u16, pread_u16, pwrite_u16),
    (u32, pread_u32, pwrite_u32),
    (u64, pread_u64, pwrite_u64),
    (i128, pread_i128, pwrite_i128),
    (u128, pread_u128, pwrite_u128),
    (f32, pread_f32, pwrite_f32),
    (f64, pread_f64, pwrite_f64),
);
cursor_typed_methods!(
    Heap,
    (i8, read_i8, write_i8),
    (u8, read_u8, write_u8),
    (i16, read_i16, write_i16),
    (u16, read_u16, write_u16),
    (u32, read_u32, write_u32),
    (u64, read_u64, write_u64),
    (i128, read_i128, write_i128),
    (u128, read_u128, write_u128),
    (f32, read_f32, write_f32),
    (f64, read_f64, write_f64),
);
bulk_methods!(
    Heap,
    (i8, 1, pread_i8_array, pwrite_i8_array, pwrite_i8_repeat),
    (i16, 2, pread_i16_array, pwrite_i16_array, pwrite_i16_repeat),
    (
        i128,
        16,
        pread_i128_array,
        pwrite_i128_array,
        pwrite_i128_repeat
    ),
    (
        u128,
        16,
        pread_u128_array,
        pwrite_u128_array,
        pwrite_u128_repeat
    ),
);
// The vectorized statistical aggregations — the core `Aggregate` blanket trait over any source —
// for a representative set of native widths. `count` is the **element** count, not bytes.
agg_methods!(
    Heap,
    (
        i32,
        i64,
        sum_i32,
        min_i32,
        max_i32,
        mean_i32,
        std_i32,
        first_i32,
        last_i32,
        count_ge_i32
    ),
    (
        i64,
        i128,
        sum_i64,
        min_i64,
        max_i64,
        mean_i64,
        std_i64,
        first_i64,
        last_i64,
        count_ge_i64
    ),
    (
        u32,
        i64,
        sum_u32,
        min_u32,
        max_u32,
        mean_u32,
        std_u32,
        first_u32,
        last_u32,
        count_ge_u32
    ),
    (
        u64,
        i128,
        sum_u64,
        min_u64,
        max_u64,
        mean_u64,
        std_u64,
        first_u64,
        last_u64,
        count_ge_u64
    ),
    (
        f32,
        f64,
        sum_f32,
        min_f32,
        max_f32,
        mean_f32,
        std_f32,
        first_f32,
        last_f32,
        count_ge_f32
    ),
    (
        f64,
        f64,
        sum_f64,
        min_f64,
        max_f64,
        mean_f64,
        std_f64,
        first_f64,
        last_f64,
        count_ge_f64
    ),
);

/// A **cursor** — a moving read/write position over an owned [`Heap`] source. Mirrors
/// `yggdryl_core::io::memory::IOCursor<Heap>`: `read` / `write` advance it, `seek` moves relative
/// to a [`Whence`] anchor, and the positioned `pread_*` / `pwrite_*` accessors reach any offset
/// without moving it. A read with a hard length requirement that runs off the end raises a
/// guided `ValueError`.
#[pyclass(module = "yggdryl.memory")]
pub struct Cursor {
    pub(crate) inner: memory::IOCursor<memory::Heap>,
}

#[pymethods]
impl Cursor {
    /// A cursor over a fresh [`Heap`] owning a copy of `data` (bytes / bytearray), or over an
    /// empty heap if `data` is omitted; positioned at the start.
    #[new]
    #[pyo3(signature = (data = None))]
    fn new(data: Option<Vec<u8>>) -> Self {
        let heap = match data {
            Some(bytes) => memory::Heap::from_vec(bytes),
            None => memory::Heap::new(),
        };
        Self {
            inner: heap.cursor(),
        }
    }

    /// A cursor over an **independent copy** of `heap` (the binding clones since it cannot
    /// consume the source), positioned at the start.
    #[staticmethod]
    fn over(heap: &Heap) -> Self {
        Self {
            inner: heap.inner.clone().cursor(),
        }
    }

    /// A cursor over a fresh [`Heap`] owning a copy of a Python **file-like** object's full
    /// contents (a `io.BytesIO` / `io.StringIO` / `io.FileIO`, or anything with `getvalue()` /
    /// `read()`) — the type-inferring constructor. `str` content is encoded UTF-8; when the
    /// object exposes `tell()`, the cursor starts at that position (a partially-consumed
    /// `BytesIO` transfers its position).
    #[staticmethod]
    fn from_io(obj: &Bound<'_, PyAny>) -> PyResult<Cursor> {
        let heap = heap_from_io(obj)?;
        let position = heap.position();
        Ok(Cursor {
            inner: memory::IOCursor::with_position(heap, position),
        })
    }

    // ---- cursor stream -----------------------------------------------------------------

    /// The current cursor position (bytes from the start). May sit past the end after a seek.
    #[getter]
    fn position(&self) -> u64 {
        self.inner.position()
    }

    /// Moves the cursor to an absolute `position` (past the end is allowed).
    fn set_position(&mut self, position: u64) {
        self.inner.set_position(position);
    }

    /// Seeks to `whence + offset` and returns the new position. A position past the end is
    /// allowed; seeking before the start raises `ValueError`.
    fn seek(&mut self, whence: Whence, offset: i64) -> PyResult<u64> {
        self.inner.seek(whence.into(), offset).map_err(ioerr)
    }

    /// Resets the cursor to the start.
    fn rewind(&mut self) {
        self.inner.rewind();
    }

    /// **Cursor read.** Returns up to `length` bytes from the current position (short near the
    /// end), advancing the cursor by the number read.
    fn read<'py>(&mut self, py: Python<'py>, length: usize) -> PyResult<Bound<'py, PyBytes>> {
        let position = self.inner.position();
        let n = self
            .inner
            .byte_size()
            .saturating_sub(position)
            .min(length as u64) as usize;
        let bytes = PyBytes::new_bound_with(py, n, |dst| {
            self.inner.pread_byte_array(position, dst);
            Ok(())
        })?;
        self.inner.set_position(position + n as u64);
        Ok(bytes)
    }

    /// **Cursor write.** Writes `data` (bytes / bytearray) at the current position, advancing
    /// the cursor by the number written (growing the source as needed); returns that count.
    fn write(&mut self, data: PyBackedBytes) -> usize {
        self.inner.write(&data)
    }

    /// Reads the next byte at the cursor, advancing it by 1, raising `ValueError` at the end.
    fn read_byte(&mut self) -> PyResult<u8> {
        self.inner.read_byte().map_err(ioerr)
    }

    /// Writes the byte `value` at the cursor, advancing it by 1.
    fn write_byte(&mut self, value: u8) -> PyResult<()> {
        self.inner.write_byte(value).map_err(ioerr)
    }

    /// Reads a little-endian `i32` (4 bytes) at the cursor, advancing it by 4, raising
    /// `ValueError` on EOF.
    fn read_i32(&mut self) -> PyResult<i32> {
        self.inner.read_i32().map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at the cursor, advancing it by 4.
    fn write_i32(&mut self, value: i32) -> PyResult<()> {
        self.inner.write_i32(value).map_err(ioerr)
    }

    /// Reads a little-endian `i64` (8 bytes) at the cursor, advancing it by 8, raising
    /// `ValueError` on EOF.
    fn read_i64(&mut self) -> PyResult<i64> {
        self.inner.read_i64().map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at the cursor, advancing it by 8.
    fn write_i64(&mut self, value: i64) -> PyResult<()> {
        self.inner.write_i64(value).map_err(ioerr)
    }

    /// Reads up to `length` **bytes** from the cursor and decodes them as UTF-8 text (clamped
    /// near the end), advancing the cursor by the bytes read, raising a guided `ValueError`
    /// on invalid UTF-8 (leaving the cursor put).
    fn read_utf8(&mut self, length: usize) -> PyResult<String> {
        self.inner.read_utf8(length).map_err(ioerr)
    }

    /// Writes `text`'s UTF-8 bytes at the cursor, advancing it; returns the number of
    /// **bytes** written.
    fn write_utf8(&mut self, text: &str) -> usize {
        self.inner.write_utf8(text)
    }

    /// Reads from the current position **to the end** as `bytes`, advancing the cursor to the
    /// end.
    fn read_to_end<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let position = self.inner.position();
        let n = self.inner.byte_size().saturating_sub(position) as usize;
        let bytes = PyBytes::new_bound_with(py, n, |dst| {
            self.inner.pread_byte_array(position, dst);
            Ok(())
        })?;
        self.inner.set_position(self.inner.byte_size());
        Ok(bytes)
    }

    /// **Reads one line** from the cursor — the content up to the next line terminator with the
    /// trailing `\n` / `\r\n` **stripped**, decoded as UTF-8 — advancing the cursor past the
    /// terminator. **CSV-aware**: a `\n` inside a double-quoted field does not end the line. A
    /// blank line returns `""` but **advances**; at the true end it returns `""` **without**
    /// advancing (that is how iteration stops).
    fn readline(&mut self) -> PyResult<String> {
        self.inner.readline().map_err(ioerr)
    }

    /// **Reads every remaining line** from the cursor into a list, advancing it to the end — each
    /// element has its trailing line terminator stripped (blank lines kept, quoted newlines
    /// honored).
    fn readlines(&mut self) -> PyResult<Vec<String>> {
        self.inner.readlines().map_err(ioerr)
    }

    // ---- positioned (delegates to the wrapped source) ----------------------------------

    /// The total length in bytes of the wrapped source.
    fn byte_size(&self) -> u64 {
        self.inner.byte_size()
    }

    /// The total length in bytes (so `len(cursor)` works).
    fn __len__(&self) -> usize {
        self.inner.byte_size() as usize
    }

    /// The total length in bits — `byte_size() * 8`.
    fn bit_size(&self) -> u64 {
        self.inner.bit_size()
    }

    /// Resizes the wrapped source to exactly `length` bytes. A cursor is a byte **view** with
    /// no resizable backing of its own, so this raises the core's guided `ValueError` (truncate
    /// a `Heap`, `Mmap`, or `LocalIO` instead).
    fn truncate(&mut self, length: u64) -> PyResult<()> {
        self.inner.truncate(length).map_err(ioerr)
    }

    /// The wrapped source's content length in bytes, preferring the cached `Content-Length`
    /// header when present and falling back to `byte_size()`.
    fn content_length(&self) -> u64 {
        self.inner.content_length()
    }

    /// Reads the single byte at `offset`, raising `ValueError` if it is past the end. Never
    /// moves the cursor.
    fn pread_byte(&self, offset: u64) -> PyResult<u8> {
        self.inner.pread_byte(offset).map_err(ioerr)
    }

    /// Reads the bit at absolute **bit** `offset` (LSB-first), raising `ValueError` if its
    /// byte is past the end.
    fn pread_bit(&self, offset: u64) -> PyResult<bool> {
        self.inner.pread_bit(offset).map_err(ioerr)
    }

    /// Reads a little-endian `i32` (4 bytes) at `offset`, raising `ValueError` on EOF.
    fn pread_i32(&self, offset: u64) -> PyResult<i32> {
        self.inner.pread_i32(offset).map_err(ioerr)
    }

    /// Reads a little-endian `i64` (8 bytes) at `offset`, raising `ValueError` on EOF.
    fn pread_i64(&self, offset: u64) -> PyResult<i64> {
        self.inner.pread_i64(offset).map_err(ioerr)
    }

    /// Writes the single byte `value` at `offset`, growing the source as needed. Never moves
    /// the cursor.
    fn pwrite_byte(&mut self, offset: u64, value: u8) -> PyResult<()> {
        self.inner.pwrite_byte(offset, value).map_err(ioerr)
    }

    /// Sets or clears the bit at absolute **bit** `offset` (LSB-first), growing the source
    /// (zero-filled) if the bit is past the end.
    fn pwrite_bit(&mut self, offset: u64, value: bool) -> PyResult<()> {
        self.inner.pwrite_bit(offset, value).map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at `offset`, growing as needed.
    fn pwrite_i32(&mut self, offset: u64, value: i32) -> PyResult<()> {
        self.inner.pwrite_i32(offset, value).map_err(ioerr)
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at `offset`, growing as needed.
    fn pwrite_i64(&mut self, offset: u64, value: i64) -> PyResult<()> {
        self.inner.pwrite_i64(offset, value).map_err(ioerr)
    }

    /// Reads up to `length` **bytes** at `offset` and decodes them as UTF-8 text (clamped
    /// near the end), raising a guided `ValueError` on invalid UTF-8. Never moves the cursor.
    fn pread_utf8(&self, offset: u64, length: usize) -> PyResult<String> {
        self.inner.pread_utf8(offset, length).map_err(ioerr)
    }

    /// Writes `text`'s UTF-8 bytes at `offset` (growing as needed); returns the number of
    /// **bytes** written. Never moves the cursor.
    fn pwrite_utf8(&mut self, offset: u64, text: &str) -> usize {
        self.inner.pwrite_utf8(offset, text)
    }

    // ---- address + source ---------------------------------------------------------------

    /// The [`Uri`] that **addresses** the wrapped source.
    #[getter]
    fn uri(&self) -> Uri {
        Uri {
            inner: self.inner.uri(),
        }
    }

    /// The wrapped source's [`Headers`] metadata — an owned **copy** (delegates to the
    /// source; edit the source and re-wrap to change it).
    #[getter]
    fn headers(&self) -> Headers {
        Headers {
            inner: self.inner.headers().clone(),
        }
    }

    /// How the wrapped source may be accessed (delegates to the source).
    #[getter]
    fn mode(&self) -> IOMode {
        self.inner.mode().into()
    }

    /// What the wrapped source **is** (delegates to the source).
    #[getter]
    fn kind(&self) -> IOKind {
        self.inner.kind().into()
    }

    /// Whether the wrapped source is a regular **file** — derived from [`kind`](Cursor::kind).
    fn is_file(&self) -> bool {
        self.inner.is_file()
    }

    /// Whether the wrapped source is a **directory** — derived from [`kind`](Cursor::kind).
    fn is_dir(&self) -> bool {
        self.inner.is_dir()
    }

    /// Whether the wrapped source **exists** — forwards the source's own notion of
    /// existence (a cursor over a live [`Heap`] exists).
    fn exists(&self) -> bool {
        self.inner.exists()
    }

    // ---- media type (delegates to the wrapped source) -----------------------------------

    /// The **primary** [`MimeType`](crate::mimetype::MimeType) of the wrapped source (headers,
    /// else address, else octet-stream).
    fn mime_type(&self) -> MimeType {
        MimeType {
            inner: self.inner.mime_type(),
        }
    }

    /// The full [`MediaType`](crate::mediatype::MediaType) of the wrapped source.
    fn media_type(&self) -> MediaType {
        MediaType {
            inner: self.inner.media_type(),
        }
    }

    /// Resolves the media type and stores it in the wrapped source's headers when
    /// `Content-Type` is unset; returns the effective [`MimeType`](crate::mimetype::MimeType).
    fn ensure_content_type(&mut self) -> MimeType {
        MimeType {
            inner: self.inner.ensure_content_type(),
        }
    }

    // ---- inference + compression (delegates to the wrapped source) -----------------------

    /// The **primary** [`MimeType`](crate::mimetype::MimeType) inferred from the wrapped
    /// source's **magic bytes** — a positioned read of the head that **never moves the
    /// cursor**; falls back to the declared/address mime when no magic matches.
    fn infer_mime_type(&self) -> MimeType {
        MimeType {
            inner: self.inner.infer_mime_type(),
        }
    }

    /// The full [`MediaType`](crate::mediatype::MediaType) inferred by **recursive magic** over
    /// the wrapped source's head (peeling each compression layer it can).
    fn infer_media_type(&self) -> MediaType {
        MediaType {
            inner: self.inner.infer_media_type(),
        }
    }

    /// The `yggdryl.compression` codec for the wrapped source's media type, or `None` when it
    /// is not a supported compression.
    fn compression(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        crate::compression::codec_to_object(py, self.inner.compression())
    }

    /// The wrapped source **decompressed** with the codec inferred from its media type, as
    /// `bytes` — raises a guided `ValueError` when it is not a supported compression.
    fn decompress<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.decompress().map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// The wrapped source's content **compressed** with the explicit `codec`, as `bytes`.
    fn compress_with<'py>(
        &self,
        py: Python<'py>,
        codec: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let out = crate::compression::with_codec(codec, |c| self.inner.compressed_with(c))?
            .map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// The wrapped source's content **decompressed** with the explicit `codec`, as `bytes`.
    fn decompress_with<'py>(
        &self,
        py: Python<'py>,
        codec: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let out = crate::compression::with_codec(codec, |c| self.inner.decompressed_with(c))?
            .map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    // ---- graph: navigation + discovery + CRUD (a cursor view is a leaf) -----------------

    /// The node's own name — the last segment of the wrapped source's address path, so
    /// empty over a heap (`mem://heap` has no path segment to name).
    #[getter]
    fn name(&self) -> String {
        self.inner.name()
    }

    /// The parent node, or `None` — a cursor view is a **leaf** of the IO graph.
    fn parent(&self) -> Option<Cursor> {
        self.inner.parent().map(|inner| Cursor { inner })
    }

    /// This node's ancestors, nearest first — empty for a leaf/root.
    fn parents(&self) -> Vec<Cursor> {
        self.inner.parents().map(|inner| Cursor { inner }).collect()
    }

    /// Streams this node's children — always the empty [`NoChildren`] stream (a cursor
    /// view is a leaf: it streams nothing, with or without `recursive=True`).
    #[pyo3(signature = (recursive = false))]
    fn ls(&self, recursive: bool) -> PyResult<NoChildren> {
        let _ = if recursive {
            self.inner.ls_recursive()
        } else {
            self.inner.ls()
        }
        .map_err(ioerr)?;
        Ok(NoChildren {})
    }

    /// The direct children, collected — always the empty list (a leaf has none).
    fn children(&self) -> PyResult<Vec<Cursor>> {
        let nodes = self.inner.children().map_err(ioerr)?;
        Ok(nodes.into_iter().map(|inner| Cursor { inner }).collect())
    }

    /// A cursor view has no removable backing — raises the guided `ValueError` naming the
    /// fix (address a filesystem node, e.g. `yggdryl.local.LocalIO`, instead). `exist_ok`
    /// governs a **missing** node (`exist_ok=False` raises on a missing one).
    #[pyo3(signature = (exist_ok = true))]
    fn rm(&self, exist_ok: bool) -> PyResult<()> {
        self.inner.rm(exist_ok).map_err(ioerr)
    }

    /// A cursor view has no removable backing — the same guided refusal as
    /// [`rm`](Cursor::rm).
    #[pyo3(signature = (exist_ok = true))]
    fn rmfile(&self, exist_ok: bool) -> PyResult<()> {
        self.inner.rmfile(exist_ok).map_err(ioerr)
    }

    /// A cursor view has no removable backing — the same guided refusal as
    /// [`rm`](Cursor::rm).
    #[pyo3(signature = (exist_ok = true))]
    fn rmdir(&self, exist_ok: bool) -> PyResult<()> {
        self.inner.rmdir(exist_ok).map_err(ioerr)
    }

    /// An independent copy of the wrapped [`Heap`] source (the cursor position is discarded).
    fn inner(&self) -> Heap {
        Heap {
            inner: self.inner.inner().clone(),
        }
    }

    /// The wrapped source's bytes as a `bytes` copy.
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.inner().as_slice())
    }

    /// The wrapped source's bytes as a `bytes` copy (so `bytes(cursor)` works).
    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, self.inner.inner().as_slice())
    }

    /// Indexing like `bytes` — `cursor[i]` is the byte at `i` as an `int` (negative indices
    /// wrap; out of range raises `IndexError`), `cursor[a:b:step]` the selected `bytes`. Never
    /// moves the cursor.
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<PyObject> {
        if let Ok(index) = key.extract::<isize>() {
            let byte = index_one(self.inner.byte_size(), index, |offset, buf| {
                self.inner.pread_byte_array(offset, buf)
            })?;
            Ok(byte.to_object(py))
        } else {
            Ok(PyBytes::new_bound(py, self.inner.inner().as_slice())
                .as_any()
                .get_item(key)?
                .unbind())
        }
    }

    /// Context-manager entry — returns the cursor itself, so `with Cursor(data) as c:` binds
    /// it.
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Context-manager exit — a no-op for an in-memory cursor; returns `False` so exceptions
    /// propagate.
    fn __exit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        false
    }

    /// Line iteration — `for line in cursor:` yields each line from the current position (like
    /// a file object), via [`readline`](Cursor::readline).
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// The next line from the cursor, or `StopIteration` at the true end — a line that advanced
    /// the cursor (including a blank line, yielded as `""`), stopping only when `readline`
    /// consumes nothing.
    fn __next__(&mut self) -> PyResult<Option<String>> {
        let start = self.inner.position();
        let line = self.inner.readline().map_err(ioerr)?;
        Ok((self.inner.position() != start).then_some(line))
    }

    fn __repr__(&self) -> String {
        format!(
            "Cursor(position={}, <{} bytes>)",
            self.inner.position(),
            self.inner.byte_size()
        )
    }
}

// The remaining native-width positioned scalars and cursor-typed read/write — completing the
// set alongside the hand-written `i32` / `i64` / byte forms in the main `Cursor` block above.
scalar_methods!(
    Cursor,
    (i8, pread_i8, pwrite_i8),
    (u8, pread_u8, pwrite_u8),
    (i16, pread_i16, pwrite_i16),
    (u16, pread_u16, pwrite_u16),
    (u32, pread_u32, pwrite_u32),
    (u64, pread_u64, pwrite_u64),
    (i128, pread_i128, pwrite_i128),
    (u128, pread_u128, pwrite_u128),
    (f32, pread_f32, pwrite_f32),
    (f64, pread_f64, pwrite_f64),
);
cursor_typed_methods!(
    Cursor,
    (i8, read_i8, write_i8),
    (u8, read_u8, write_u8),
    (i16, read_i16, write_i16),
    (u16, read_u16, write_u16),
    (u32, read_u32, write_u32),
    (u64, read_u64, write_u64),
    (i128, read_i128, write_i128),
    (u128, read_u128, write_u128),
    (f32, read_f32, write_f32),
    (f64, read_f64, write_f64),
);

/// A **bounded window** over an owned [`Heap`] source — the range `[offset, offset + length)`
/// addressed from its own `0`. Mirrors `yggdryl_core::io::memory::IOSlice<Heap>`: it is
/// **fixed-length**, so a write past its end is clamped away (it never grows the source beyond
/// the window). A typed read that runs off the window's end raises a guided `ValueError`.
#[pyclass(module = "yggdryl.memory")]
pub struct Slice {
    pub(crate) inner: memory::IOSlice<memory::Heap>,
}

#[pymethods]
impl Slice {
    /// The window `[offset, offset + length)` over an **independent copy** of `heap`, addressed
    /// from its own `0`. Raises `ValueError` if it runs past the source's end.
    #[new]
    fn new(heap: &Heap, offset: u64, length: u64) -> PyResult<Self> {
        heap.inner
            .clone()
            .window(offset, length)
            .map(|inner| Self { inner })
            .map_err(ioerr)
    }

    /// A [`Slice`] over an **independent copy** of `heap` — the same as the constructor, as a
    /// factory (the spelling shared with [`Cursor.over`](Cursor::over)). Raises `ValueError`
    /// if the window runs past the source's end.
    #[staticmethod]
    fn over(heap: &Heap, offset: u64, length: u64) -> PyResult<Self> {
        Self::new(heap, offset, length)
    }

    /// The window length in bytes.
    fn byte_size(&self) -> u64 {
        self.inner.byte_size()
    }

    /// The window length in bytes (so `len(slice)` works).
    fn __len__(&self) -> usize {
        self.inner.byte_size() as usize
    }

    /// The window's start offset within the source.
    #[getter]
    fn offset(&self) -> u64 {
        self.inner.offset()
    }

    /// Resizes the wrapped source to exactly `length` bytes. A window is a fixed-length byte
    /// **view** with no resizable backing of its own, so this raises the core's guided
    /// `ValueError` (truncate a `Heap`, `Mmap`, or `LocalIO` instead).
    fn truncate(&mut self, length: u64) -> PyResult<()> {
        self.inner.truncate(length).map_err(ioerr)
    }

    /// The window's content length in bytes, preferring the cached `Content-Length` header
    /// when present and falling back to `byte_size()`.
    fn content_length(&self) -> u64 {
        self.inner.content_length()
    }

    /// **Positioned read.** Returns up to `length` bytes starting at `offset` **within the
    /// window** as `bytes` — short near the window's end, empty at or past it. Reads
    /// **directly** into the `bytes` allocation (one copy).
    fn pread_byte_array<'py>(
        &self,
        py: Python<'py>,
        offset: u64,
        length: usize,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let n = self
            .inner
            .byte_size()
            .saturating_sub(offset)
            .min(length as u64) as usize;
        PyBytes::new_bound_with(py, n, |dst| {
            self.inner.pread_byte_array(offset, dst);
            Ok(())
        })
    }

    /// Reads the single byte at `offset` within the window, raising `ValueError` if it is past
    /// the window's end.
    fn pread_byte(&self, offset: u64) -> PyResult<u8> {
        self.inner.pread_byte(offset).map_err(ioerr)
    }

    /// Reads a little-endian `i32` (4 bytes) at `offset` within the window, raising
    /// `ValueError` on EOF.
    fn pread_i32(&self, offset: u64) -> PyResult<i32> {
        self.inner.pread_i32(offset).map_err(ioerr)
    }

    /// Reads a little-endian `i64` (8 bytes) at `offset` within the window, raising
    /// `ValueError` on EOF.
    fn pread_i64(&self, offset: u64) -> PyResult<i64> {
        self.inner.pread_i64(offset).map_err(ioerr)
    }

    /// Reads up to `length` **bytes** at `offset` **within the window** and decodes them as
    /// UTF-8 text (clamped to the window's end), raising a guided `ValueError` on invalid
    /// UTF-8 — including a multi-byte character cut by the window.
    fn pread_utf8(&self, offset: u64, length: usize) -> PyResult<String> {
        self.inner.pread_utf8(offset, length).map_err(ioerr)
    }

    /// **Positioned write**, clamped to the window. Copies `data` (bytes / bytearray) in at
    /// `offset`, writing only as far as the window's end; returns the number of bytes written.
    fn pwrite_byte_array(&mut self, offset: u64, data: PyBackedBytes) -> usize {
        self.inner.pwrite_byte_array(offset, &data)
    }

    /// The [`Uri`] that **addresses** the wrapped source.
    #[getter]
    fn uri(&self) -> Uri {
        Uri {
            inner: self.inner.uri(),
        }
    }

    /// The wrapped source's [`Headers`] metadata — an owned **copy** (delegates to the
    /// source).
    #[getter]
    fn headers(&self) -> Headers {
        Headers {
            inner: self.inner.headers().clone(),
        }
    }

    /// How the wrapped source may be accessed (delegates to the source).
    #[getter]
    fn mode(&self) -> IOMode {
        self.inner.mode().into()
    }

    /// What the wrapped source **is** (delegates to the source).
    #[getter]
    fn kind(&self) -> IOKind {
        self.inner.kind().into()
    }

    /// Whether the wrapped source is a regular **file** — derived from [`kind`](Slice::kind).
    fn is_file(&self) -> bool {
        self.inner.is_file()
    }

    /// Whether the wrapped source is a **directory** — derived from [`kind`](Slice::kind).
    fn is_dir(&self) -> bool {
        self.inner.is_dir()
    }

    /// Whether the wrapped source **exists** — forwards the source's own notion of
    /// existence (a window over a live [`Heap`] exists).
    fn exists(&self) -> bool {
        self.inner.exists()
    }

    // ---- media type (delegates to the wrapped source) -----------------------------------

    /// The **primary** [`MimeType`](crate::mimetype::MimeType) of the wrapped source (headers,
    /// else address, else octet-stream).
    fn mime_type(&self) -> MimeType {
        MimeType {
            inner: self.inner.mime_type(),
        }
    }

    /// The full [`MediaType`](crate::mediatype::MediaType) of the wrapped source.
    fn media_type(&self) -> MediaType {
        MediaType {
            inner: self.inner.media_type(),
        }
    }

    /// Resolves the media type and stores it in the wrapped source's headers when
    /// `Content-Type` is unset; returns the effective [`MimeType`](crate::mimetype::MimeType).
    fn ensure_content_type(&mut self) -> MimeType {
        MimeType {
            inner: self.inner.ensure_content_type(),
        }
    }

    // ---- inference + compression (delegates to the wrapped source) -----------------------

    /// The **primary** [`MimeType`](crate::mimetype::MimeType) inferred from the window's
    /// **magic bytes** — a positioned read of the head that **never moves the cursor**; falls
    /// back to the declared/address mime when no magic matches.
    fn infer_mime_type(&self) -> MimeType {
        MimeType {
            inner: self.inner.infer_mime_type(),
        }
    }

    /// The full [`MediaType`](crate::mediatype::MediaType) inferred by **recursive magic** over
    /// the window's head (peeling each compression layer it can).
    fn infer_media_type(&self) -> MediaType {
        MediaType {
            inner: self.inner.infer_media_type(),
        }
    }

    /// The `yggdryl.compression` codec for the window's media type, or `None` when it is not a
    /// supported compression.
    fn compression(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        crate::compression::codec_to_object(py, self.inner.compression())
    }

    /// The window **decompressed** with the codec inferred from its media type, as `bytes` —
    /// raises a guided `ValueError` when it is not a supported compression.
    fn decompress<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let out = self.inner.decompress().map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// The window's content **compressed** with the explicit `codec`, as `bytes`.
    fn compress_with<'py>(
        &self,
        py: Python<'py>,
        codec: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let out = crate::compression::with_codec(codec, |c| self.inner.compressed_with(c))?
            .map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    /// The window's content **decompressed** with the explicit `codec`, as `bytes`.
    fn decompress_with<'py>(
        &self,
        py: Python<'py>,
        codec: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let out = crate::compression::with_codec(codec, |c| self.inner.decompressed_with(c))?
            .map_err(ioerr)?;
        Ok(PyBytes::new_bound(py, &out))
    }

    // ---- graph: navigation + discovery + CRUD (a window view is a leaf) -----------------

    /// The node's own name — the last segment of the wrapped source's address path, so
    /// empty over a heap (`mem://heap` has no path segment to name).
    #[getter]
    fn name(&self) -> String {
        self.inner.name()
    }

    /// The parent node, or `None` — a window view is a **leaf** of the IO graph.
    fn parent(&self) -> Option<Slice> {
        self.inner.parent().map(|inner| Slice { inner })
    }

    /// This node's ancestors, nearest first — empty for a leaf/root.
    fn parents(&self) -> Vec<Slice> {
        self.inner.parents().map(|inner| Slice { inner }).collect()
    }

    /// Streams this node's children — always the empty [`NoChildren`] stream (a window
    /// view is a leaf: it streams nothing, with or without `recursive=True`).
    #[pyo3(signature = (recursive = false))]
    fn ls(&self, recursive: bool) -> PyResult<NoChildren> {
        let _ = if recursive {
            self.inner.ls_recursive()
        } else {
            self.inner.ls()
        }
        .map_err(ioerr)?;
        Ok(NoChildren {})
    }

    /// The direct children, collected — always the empty list (a leaf has none).
    fn children(&self) -> PyResult<Vec<Slice>> {
        let nodes = self.inner.children().map_err(ioerr)?;
        Ok(nodes.into_iter().map(|inner| Slice { inner }).collect())
    }

    /// A window view has no removable backing — raises the guided `ValueError` naming the
    /// fix (address a filesystem node, e.g. `yggdryl.local.LocalIO`, instead). `exist_ok`
    /// governs a **missing** node (`exist_ok=False` raises on a missing one).
    #[pyo3(signature = (exist_ok = true))]
    fn rm(&self, exist_ok: bool) -> PyResult<()> {
        self.inner.rm(exist_ok).map_err(ioerr)
    }

    /// A window view has no removable backing — the same guided refusal as
    /// [`rm`](Slice::rm).
    #[pyo3(signature = (exist_ok = true))]
    fn rmfile(&self, exist_ok: bool) -> PyResult<()> {
        self.inner.rmfile(exist_ok).map_err(ioerr)
    }

    /// A window view has no removable backing — the same guided refusal as
    /// [`rm`](Slice::rm).
    #[pyo3(signature = (exist_ok = true))]
    fn rmdir(&self, exist_ok: bool) -> PyResult<()> {
        self.inner.rmdir(exist_ok).map_err(ioerr)
    }

    /// An independent copy of the wrapped [`Heap`] source (the window bounds are discarded).
    fn inner(&self) -> Heap {
        Heap {
            inner: self.inner.inner().clone(),
        }
    }

    /// The window's bytes as a `bytes` copy.
    fn to_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(
            py,
            &self.inner.pread_vec(0, self.inner.byte_size() as usize),
        )
    }

    /// The window's bytes as a `bytes` copy (so `bytes(slice)` works).
    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(
            py,
            &self.inner.pread_vec(0, self.inner.byte_size() as usize),
        )
    }

    /// Indexing like `bytes` — `window[i]` is the byte at `i` **within the window** as an `int`
    /// (negative indices wrap; out of range raises `IndexError`), `window[a:b:step]` the
    /// selected `bytes`.
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<PyObject> {
        if let Ok(index) = key.extract::<isize>() {
            let byte = index_one(self.inner.byte_size(), index, |offset, buf| {
                self.inner.pread_byte_array(offset, buf)
            })?;
            Ok(byte.to_object(py))
        } else {
            Ok(PyBytes::new_bound(
                py,
                &self.inner.pread_vec(0, self.inner.byte_size() as usize),
            )
            .as_any()
            .get_item(key)?
            .unbind())
        }
    }

    /// Context-manager entry — returns the window itself, so `with Slice(h, o, n) as w:` binds
    /// it.
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Context-manager exit — a no-op for an in-memory window; returns `False` so exceptions
    /// propagate.
    fn __exit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        false
    }

    fn __repr__(&self) -> String {
        format!(
            "Slice(offset={}, <{} bytes>)",
            self.inner.offset(),
            self.inner.byte_size()
        )
    }
}

/// The **always-empty** streaming iterator every **leaf** source returns from `ls` — a
/// [`Heap`], [`Cursor`], or [`Slice`], and the raw [`yggdryl.local.Mmap`](crate::io::local::Mmap)
/// mapping, is a leaf of the IO graph (the core's `NoChildren` stream), so there is never
/// an entry to yield, but the stream still satisfies the Python iterator protocol like
/// `yggdryl.local.LocalEntries` does: `__iter__` returns the iterator itself and `__next__`
/// raises `StopIteration`.
#[pyclass(module = "yggdryl.memory")]
pub struct NoChildren {}

#[pymethods]
impl NoChildren {
    /// The iterator protocol — `iter(entries) is entries`, like every Python iterator.
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Always exhausted — a leaf streams no children, so this raises `StopIteration`.
    fn __next__(&mut self) -> Option<Py<PyAny>> {
        None
    }

    fn __repr__(&self) -> String {
        "NoChildren(<empty>)".to_string()
    }
}

/// Populates the `memory` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Heap>()?;
    module.add_class::<Whence>()?;
    module.add_class::<Cursor>()?;
    module.add_class::<Slice>()?;
    module.add_class::<NoChildren>()?;
    Ok(())
}
