//! [`IOBase`] — positioned (random-access) byte read/write, the base of the I/O trait family.

use super::{IOCursor, IOSlice, IoError};
use crate::compression::{codec_for_mime, compression_err, Compression};
use crate::headers::Headers;
use crate::io::{IOKind, IOMode};
use crate::mediatype::MediaType;
use crate::mimetype::MimeType;
use crate::uri::Uri;

/// The **static default URI** of an in-memory source — the stable synthetic `mem://heap`
/// (deterministic; the real allocation address is deliberately not leaked). Parsed once into
/// this process-wide static; an accessor clones it (a couple of small string clones), never
/// re-parses.
pub(crate) static DEFAULT_URI: std::sync::LazyLock<Uri> = std::sync::LazyLock::new(|| {
    Uri::parse_str("mem://heap").expect("the static mem://heap URI parses")
});

/// The shared synthetic `mem://heap` address (parsed once) — what any in-memory source
/// reports from [`uri`](IOBase::uri) unless it has been re-addressed by [`join`](IOBase::join).
pub(crate) fn default_uri() -> &'static Uri {
    &DEFAULT_URI
}

/// The element count bulk operations stage per stack chunk — sized so the widest staged chunk
/// (`i128`/`u128`: 256 × 16 = 4 KiB) stays comfortably on the stack while the per-chunk convert
/// loop is long enough for LLVM to vectorize.
const BULK_CHUNK: usize = 256;

/// The always-empty child stream of a **leaf** source — what a source with no children of
/// its own (a [`Heap`](super::Heap), a wrapper view, a raw mapped file) declares for
/// [`IOBase::Children`] / [`IOBase::Walk`] and returns from [`ls`](IOBase::ls).
pub type NoChildren<T> = std::iter::Empty<Result<T, IoError>>;

/// Validates a window `[offset, offset + len)` against `available` bytes, returning the
/// (overflow-checked) end offset — the single source of truth for the window bounds check
/// shared by [`Heap::slice`](super::Heap::slice) and [`IOSlice::new`](super::IOSlice::new),
/// so both raise the identical [`IoError::SliceOutOfBounds`].
pub(crate) fn checked_window(offset: u64, len: u64, available: u64) -> Result<u64, IoError> {
    offset
        .checked_add(len)
        .filter(|&end| end <= available)
        .ok_or(IoError::SliceOutOfBounds {
            offset,
            len,
            available,
        })
}

/// The guided error for a removal on a source with no removable backing.
fn unremovable(uri: &Uri, method: &str) -> IoError {
    IoError::FileIo {
        op: "remove",
        path: uri.to_string(),
        detail: format!(
            "{method} needs a removable backing; this source has none — address a \
             filesystem node (e.g. LocalIO) instead"
        ),
    }
}

/// Emits the three **bulk numeric** trait methods (array read, array write, repeat fill) for
/// one little-endian numeric type, each a 1-line delegation to the matching stack-staged
/// [`stage_*`] kernel — so every width (`u16`/`u32`/`u64`/`f32`/`f64`, alongside the
/// hand-written `i32`/`i64`) shares one vectorizable implementation and every source inherits
/// it. Overriding sources (`Heap`, `Mmap`) replace these with a direct contiguous conversion.
macro_rules! bulk_numeric_methods {
    ($t:ty, $read:ident, $write:ident, $repeat:ident, $sread:path, $swrite:path, $srepeat:path) => {
        #[doc = concat!("**Bulk typed read** of little-endian `", stringify!($t),
            "`s — the `", stringify!($t), "` counterpart of [`pread_i32_array`](IOBase::pread_i32_array). \
             Fills all of `dst` or errors with [`IoError::UnexpectedEof`]; stack-staged (zero heap) \
             and vectorizable.")]
        fn $read(&self, offset: u64, dst: &mut [$t]) -> Result<(), IoError> {
            $sread(self, offset, dst)
        }
        #[doc = concat!("**Bulk typed write** of little-endian `", stringify!($t),
            "`s — the `", stringify!($t), "` counterpart of [`pwrite_i32_array`](IOBase::pwrite_i32_array).")]
        fn $write(&mut self, offset: u64, src: &[$t]) -> Result<(), IoError> {
            $swrite(self, offset, src)
        }
        #[doc = concat!("**Repeated-value fill** of `count` little-endian `", stringify!($t),
            "` copies of `value` — the `", stringify!($t), "` counterpart of \
             [`pwrite_i32_repeat`](IOBase::pwrite_i32_repeat); no full array is materialized.")]
        fn $repeat(&mut self, offset: u64, value: $t, count: usize) -> Result<(), IoError> {
            $srepeat(self, offset, value, count)
        }
    };
}

/// Emits the **scalar** positioned read/write pair for one little-endian numeric type, each a
/// 2-line delegation to the byte primitives — so every native width (`i8`/`u8`/`i16`/`u16`/…/
/// `i128`/`u128`/`f32`/`f64`, alongside the hand-written `i32`/`i64`) reads and writes with the
/// same zero-allocation stack-buffer round-trip. The read fills a fixed stack array via
/// [`pread_exact`](IOBase::pread_exact) (guided `UnexpectedEof` on a short source); the write
/// streams the little-endian bytes through [`pwrite_all`](IOBase::pwrite_all), growing as needed.
macro_rules! scalar_numeric_methods {
    ($t:ty, $width:literal, $read:ident, $write:ident) => {
        #[doc = concat!("Reads a little-endian `", stringify!($t), "` (a ", stringify!($width),
            "-byte value) at `offset`, erroring with [`IoError::UnexpectedEof`] on a short source.")]
        fn $read(&self, offset: u64) -> Result<$t, IoError> {
            let mut buf = [0u8; $width];
            self.pread_exact(offset, &mut buf)?;
            Ok(<$t>::from_le_bytes(buf))
        }
        #[doc = concat!("Writes `value` as a little-endian `", stringify!($t), "` (a ",
            stringify!($width), "-byte value) at `offset`, growing the source as needed.")]
        fn $write(&mut self, offset: u64, value: $t) -> Result<(), IoError> {
            self.pwrite_all(offset, &value.to_le_bytes())
        }
    };
}

/// Emits the **bulk-op forwarding** methods for a wrapper that holds an inner [`IOBase`] in field
/// `$field` — every typed bulk array read/write and repeat fill delegates straight to
/// `self.$field`, so the wrapper inherits the backing's **fast contiguous overrides** (a `Heap`'s
/// direct-off-the-`Vec` conversion, a mapped file's direct bulk path) instead of the stack-staged
/// trait default's extra copy. Pure one-line delegations, no logic. Used by
/// [`IOCursor`](IOCursor) and the GPU host buffer.
macro_rules! forward_bulk_ops {
    ($field:ident) => {
        $crate::io::memory::forward_bulk_ops!(@a $field, pread_i32_array, pwrite_i32_array, i32);
        $crate::io::memory::forward_bulk_ops!(@a $field, pread_i64_array, pwrite_i64_array, i64);
        $crate::io::memory::forward_bulk_ops!(@a $field, pread_u16_array, pwrite_u16_array, u16);
        $crate::io::memory::forward_bulk_ops!(@a $field, pread_u32_array, pwrite_u32_array, u32);
        $crate::io::memory::forward_bulk_ops!(@a $field, pread_u64_array, pwrite_u64_array, u64);
        $crate::io::memory::forward_bulk_ops!(@a $field, pread_f32_array, pwrite_f32_array, f32);
        $crate::io::memory::forward_bulk_ops!(@a $field, pread_f64_array, pwrite_f64_array, f64);
        $crate::io::memory::forward_bulk_ops!(@a $field, pread_i8_array, pwrite_i8_array, i8);
        $crate::io::memory::forward_bulk_ops!(@a $field, pread_i16_array, pwrite_i16_array, i16);
        $crate::io::memory::forward_bulk_ops!(@a $field, pread_i128_array, pwrite_i128_array, i128);
        $crate::io::memory::forward_bulk_ops!(@a $field, pread_u128_array, pwrite_u128_array, u128);
        $crate::io::memory::forward_bulk_ops!(@r $field, pwrite_byte_repeat, u8);
        $crate::io::memory::forward_bulk_ops!(@r $field, pwrite_i32_repeat, i32);
        $crate::io::memory::forward_bulk_ops!(@r $field, pwrite_i64_repeat, i64);
        $crate::io::memory::forward_bulk_ops!(@r $field, pwrite_u16_repeat, u16);
        $crate::io::memory::forward_bulk_ops!(@r $field, pwrite_u32_repeat, u32);
        $crate::io::memory::forward_bulk_ops!(@r $field, pwrite_u64_repeat, u64);
        $crate::io::memory::forward_bulk_ops!(@r $field, pwrite_f32_repeat, f32);
        $crate::io::memory::forward_bulk_ops!(@r $field, pwrite_f64_repeat, f64);
        $crate::io::memory::forward_bulk_ops!(@r $field, pwrite_i8_repeat, i8);
        $crate::io::memory::forward_bulk_ops!(@r $field, pwrite_i16_repeat, i16);
        $crate::io::memory::forward_bulk_ops!(@r $field, pwrite_i128_repeat, i128);
        $crate::io::memory::forward_bulk_ops!(@r $field, pwrite_u128_repeat, u128);
    };
    (@a $field:ident, $pr:ident, $pw:ident, $t:ty) => {
        fn $pr(&self, offset: u64, dst: &mut [$t]) -> Result<(), $crate::io::IoError> {
            self.$field.$pr(offset, dst)
        }
        fn $pw(&mut self, offset: u64, src: &[$t]) -> Result<(), $crate::io::IoError> {
            self.$field.$pw(offset, src)
        }
    };
    (@r $field:ident, $rep:ident, $t:ty) => {
        fn $rep(&mut self, offset: u64, value: $t, count: usize) -> Result<(), $crate::io::IoError> {
            self.$field.$rep(offset, value, count)
        }
    };
}
pub(crate) use forward_bulk_ops;

/// Emits the three stack-staged **bulk numeric** kernels (the trait-default source of truth)
/// for one little-endian numeric type — each stages through one fixed stack chunk (zero heap)
/// and converts in a dense, branch-free loop the compiler auto-vectorizes on stable Rust.
macro_rules! stage_numeric_kernels {
    ($t:ty, $width:literal, $read:ident, $write:ident, $repeat:ident) => {
        pub(crate) fn $read<S: IOBase>(
            src: &S,
            offset: u64,
            dst: &mut [$t],
        ) -> Result<(), IoError> {
            let mut bytes = [0u8; BULK_CHUNK * $width];
            let mut position = offset;
            for chunk in dst.chunks_mut(BULK_CHUNK) {
                let staged = &mut bytes[..chunk.len() * $width];
                src.pread_exact(position, staged)?;
                for (value, raw) in chunk.iter_mut().zip(staged.chunks_exact($width)) {
                    *value = <$t>::from_le_bytes(raw.try_into().expect("chunks_exact width"));
                }
                position += staged.len() as u64;
            }
            Ok(())
        }
        pub(crate) fn $write<S: IOBase>(
            dst: &mut S,
            offset: u64,
            src: &[$t],
        ) -> Result<(), IoError> {
            let mut bytes = [0u8; BULK_CHUNK * $width];
            let mut position = offset;
            for chunk in src.chunks(BULK_CHUNK) {
                let staged = &mut bytes[..chunk.len() * $width];
                for (raw, value) in staged.chunks_exact_mut($width).zip(chunk) {
                    raw.copy_from_slice(&value.to_le_bytes());
                }
                dst.pwrite_all(position, staged)?;
                position += staged.len() as u64;
            }
            Ok(())
        }
        pub(crate) fn $repeat<S: IOBase>(
            dst: &mut S,
            offset: u64,
            value: $t,
            count: usize,
        ) -> Result<(), IoError> {
            let mut chunk = [0u8; BULK_CHUNK * $width];
            for raw in chunk.chunks_exact_mut($width) {
                raw.copy_from_slice(&value.to_le_bytes());
            }
            let mut position = offset;
            let mut remaining = count;
            while remaining > 0 {
                let take = remaining.min(BULK_CHUNK);
                dst.pwrite_all(position, &chunk[..take * $width])?;
                position += (take * $width) as u64;
                remaining -= take;
            }
            Ok(())
        }
    };
}

/// Random-access byte storage addressed by absolute offset — no cursor. This is the base
/// every I/O **source** shares: [`IOCursor`](super::IOCursor) adds a moving position on top, and
/// [`IOSlice`](super::IOSlice) adds bounded sub-views.
///
/// # Shape
///
/// - **Size** — [`byte_size`](IOBase::byte_size) / [`bit_size`](IOBase::bit_size).
/// - **Capacity** — [`capacity`](IOBase::capacity) / [`reserve`](IOBase::reserve), the `Vec`-like
///   amortized-growth hooks (a source with no spare capacity, e.g. a memory-map, reports its size
///   and ignores `reserve`).
/// - **Byte-array primitives** — [`pread_byte_array`](IOBase::pread_byte_array) /
///   [`pwrite_byte_array`](IOBase::pwrite_byte_array): the two methods a source must implement.
/// - **Typed accessors** — `byte` / `bit` / `i32` / `i64`, positioned little-endian
///   read/write built on the primitives (`pread_byte`, `pread_bit`, `pread_i32`, `pread_i64`
///   and their `pwrite_*` twins).
///
/// DESIGN: the two **primitives** are *infallible* (`-> usize`), because the physical layer is
/// in-memory: a read past the end simply returns fewer bytes (0 at the end) and a write past the
/// end grows the storage, zero-filling any gap. The fallible surface is the **full** and **typed**
/// helpers built on them, whose contract — *fill exactly this many* — can be broken by the end of
/// the data. Signatures take `&[u8]` / `&mut [u8]` and native integers, never a foreign buffer
/// type, so the storage underneath stays an implementation detail. Bit addressing is **LSB-first**
/// (bit `i` is bit `i % 8` of byte `i / 8`, least-significant first), matching Arrow validity
/// bitmaps.
///
/// # The graph surface — every source is a node
///
/// `IOBase` is also the **central access path**: like [`uri`](IOBase::uri) addresses a
/// source, the graph methods place it in an IO graph. [`ls`](IOBase::ls) /
/// [`ls_recursive`](IOBase::ls_recursive) stream children **of the same type** (a leaf
/// source streams nothing — [`NoChildren`]), [`name`](IOBase::name) /
/// [`parent`](IOBase::parent) navigate, and [`rm`](IOBase::rm) / [`rmfile`](IOBase::rmfile)
/// / [`rmdir`](IOBase::rmdir) remove. A **container** node (a directory, an object-store
/// prefix) serves byte I/O as a **memory tree** over its children via the generic
/// [`tree_byte_size`](IOBase::tree_byte_size) /
/// [`tree_pread_byte_array`](IOBase::tree_pread_byte_array) /
/// [`tree_pwrite_byte_array`](IOBase::tree_pwrite_byte_array) — written once here so every
/// filesystem family (local today; s3 / azure later) inherits the same behavior.
pub trait IOBase: Sized {
    /// The total length in bytes.
    fn byte_size(&self) -> u64;

    /// The total length in bits — `byte_size() * 8`.
    fn bit_size(&self) -> u64 {
        self.byte_size() * 8
    }

    /// Whether the storage is empty (`byte_size() == 0`).
    fn is_empty(&self) -> bool {
        self.byte_size() == 0
    }

    /// The number of bytes the storage can hold before it must reallocate — like
    /// [`Vec::capacity`]. A source with no distinct spare capacity (a fixed map, a view) returns
    /// [`byte_size`](IOBase::byte_size); a growable one (e.g. [`Heap`](super::Heap)) overrides it.
    fn capacity(&self) -> u64 {
        self.byte_size()
    }

    /// Reserves capacity for at least `additional` more bytes past the current
    /// [`byte_size`](IOBase::byte_size), amortizing later writes — like [`Vec::reserve`]. The
    /// default is a **no-op** (a fixed source cannot grow); a growable source overrides it.
    fn reserve(&mut self, additional: u64) {
        let _ = additional;
    }

    /// The spare room already allocated — `capacity() - byte_size()`, the bytes that can be
    /// appended before the next reallocation. The planning counterpart of
    /// [`reserve`](IOBase::reserve).
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut h = Heap::with_capacity(64);
    /// h.pwrite_byte_array(0, &[0; 16]);
    /// assert!(h.spare_capacity() >= 48);
    /// ```
    fn spare_capacity(&self) -> u64 {
        self.capacity().saturating_sub(self.byte_size())
    }

    /// Reserves capacity for **exactly** `additional` more bytes — like [`Vec::reserve_exact`]:
    /// no amortized over-allocation, for a caller that knows the final size and wants no spare
    /// tail. The default is a no-op (a fixed source cannot grow).
    fn reserve_exact(&mut self, additional: u64) {
        let _ = additional;
    }

    /// **Checked** reservation of at least `additional` more bytes — like [`Vec::try_reserve`]:
    /// where [`reserve`](IOBase::reserve) would **abort the process** on overflow or allocator
    /// failure, this returns the guided [`IoError::CapacityOverflow`] instead, so a hostile or
    /// miscomputed size is a recoverable error. The default is `Ok` (a fixed source reserves
    /// nothing); a growable source overrides it with a real checked reservation.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase, IoError};
    ///
    /// let mut h = Heap::new();
    /// h.try_reserve(1024).unwrap();               // fine — and now pre-allocated
    /// assert!(h.capacity() >= 1024);
    /// let err = h.try_reserve(u64::MAX).unwrap_err(); // recoverable, never an abort
    /// assert!(matches!(err, IoError::CapacityOverflow { .. }));
    /// ```
    fn try_reserve(&mut self, additional: u64) -> Result<(), IoError> {
        let _ = additional;
        Ok(())
    }

    /// **Checked exact** reservation — [`try_reserve`](IOBase::try_reserve) without the
    /// amortized over-allocation, like [`Vec::try_reserve_exact`]. The default is `Ok`.
    fn try_reserve_exact(&mut self, additional: u64) -> Result<(), IoError> {
        let _ = additional;
        Ok(())
    }

    /// Ensures the **total** capacity is at least `total` bytes — the absolute-target form of
    /// [`reserve`](IOBase::reserve), for a pipeline that knows how much data is coming. A
    /// no-op when the capacity already suffices; never shrinks.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut h = Heap::new();
    /// h.ensure_capacity(4096);
    /// assert!(h.capacity() >= 4096);
    /// h.ensure_capacity(16); // already satisfied — no-op, never shrinks
    /// assert!(h.capacity() >= 4096);
    /// ```
    fn ensure_capacity(&mut self, total: u64) {
        if total > self.capacity() {
            self.reserve(total.saturating_sub(self.byte_size()));
        }
    }

    /// **Checked** [`ensure_capacity`](IOBase::ensure_capacity) — errors with
    /// [`IoError::CapacityOverflow`] instead of aborting when `total` cannot be allocated.
    fn try_ensure_capacity(&mut self, total: u64) -> Result<(), IoError> {
        if total > self.capacity() {
            self.try_reserve(total.saturating_sub(self.byte_size()))?;
        }
        Ok(())
    }

    /// Releases spare capacity back to the allocator, shrinking the allocation toward
    /// [`byte_size`](IOBase::byte_size) — like [`Vec::shrink_to_fit`]. The default is a no-op
    /// (a fixed source has nothing to release).
    fn shrink_to_fit(&mut self) {}

    /// Shrinks the allocation toward `min_capacity` (never below
    /// [`byte_size`](IOBase::byte_size)) — like [`Vec::shrink_to`], keeping a known working
    /// headroom while releasing the rest. The default is a no-op.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut h = Heap::with_capacity(4096);
    /// h.pwrite_byte_array(0, &[0; 8]);
    /// h.shrink_to(64);
    /// assert!(h.capacity() >= 8 && h.capacity() <= 4096);
    /// ```
    fn shrink_to(&mut self, min_capacity: u64) {
        let _ = min_capacity;
    }

    /// Builds a source **pre-allocated** for `capacity` bytes — the fast path when the size is
    /// known up front, so the first writes never reallocate. Works on any source that is
    /// `Default` (an empty value plus one [`reserve`](IOBase::reserve)); a source with a cheaper
    /// exact allocation may override it.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let heap = <Heap as IOBase>::with_capacity(4096);
    /// assert!(heap.is_empty());
    /// assert!(heap.capacity() >= 4096);
    /// ```
    fn with_capacity(capacity: u64) -> Self
    where
        Self: Sized + Default,
    {
        let mut source = Self::default();
        source.reserve(capacity);
        source
    }

    /// The [`Uri`] that **addresses** this source — every source is locatable. The default is
    /// the stable synthetic in-memory address `mem://heap` (the `mem` scheme addresses
    /// in-memory sources; deterministic, so tests and logs can rely on it) — an anonymous
    /// in-memory source ([`Heap`](super::Heap)) stores no address and keeps this default. A
    /// source with a real address (a future file/network source) overrides it to return its
    /// own.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// // An in-memory source reports the synthetic mem:// address.
    /// assert_eq!(Heap::new().uri().to_string(), "mem://heap");
    /// assert_eq!(Heap::new().uri().scheme(), Some("mem"));
    /// ```
    fn uri(&self) -> Uri {
        DEFAULT_URI.clone()
    }

    /// The metadata attached to this source — the project-wide [`Headers`] map (HTTP headers,
    /// schema/field metadata, source annotations all live here; never a second map type).
    fn headers(&self) -> &Headers;

    /// Mutable access to the source's [`Headers`] metadata.
    fn headers_mut(&mut self) -> &mut Headers;

    /// How this source may be accessed — see [`IOMode`]. In-memory sources default to
    /// [`IOMode::ReadWrite`]; a source opened otherwise overrides it.
    fn mode(&self) -> IOMode {
        IOMode::ReadWrite
    }

    /// What this source **is** — see [`IOKind`] ([`Heap`](super::Heap) reports
    /// [`IOKind::Heap`]; a file source reports [`IOKind::File`] / [`IOKind::Directory`], or
    /// [`IOKind::Missing`] when nothing exists at its address).
    fn kind(&self) -> IOKind;

    /// Whether this source is a regular **file** — derived from [`kind`](IOBase::kind).
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// assert!(!Heap::new().is_file()); // a heap is IOKind::Heap, not a file
    /// ```
    fn is_file(&self) -> bool {
        self.kind() == IOKind::File
    }

    /// Whether this source is a **directory** — derived from [`kind`](IOBase::kind).
    fn is_dir(&self) -> bool {
        self.kind() == IOKind::Directory
    }

    /// Whether this source's kind is [`Unknown`](IOKind::Unknown) — it exists, but of a type
    /// that is not file / directory / heap.
    fn is_unknown(&self) -> bool {
        self.kind() == IOKind::Unknown
    }

    /// Whether something **exists** at this source's address — anything except
    /// [`Missing`](IOKind::Missing) (so a `File`, `Directory`, live `Heap`, or `Unknown` node
    /// all exist). Leverages [`IOKind::exists`](IOKind::exists).
    fn exists(&self) -> bool {
        self.kind().exists()
    }

    // ---------------------------------------------------------------------------------
    // Media type — one resolution: declared headers, else inferred from the address,
    // else the octet-stream fallback. Never `None` (a source always has an answer).
    // ---------------------------------------------------------------------------------

    /// The **primary [`MimeType`]** of this source: the `Content-Type`
    /// [`headers`](IOBase::headers) declare, else inferred from the
    /// [`uri`](IOBase::uri)'s file name, else the `application/octet-stream` fallback — always
    /// an answer.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    /// use yggdryl_core::headers::Headers;
    ///
    /// let mut h = Heap::new();
    /// assert!(h.mime_type().is_octet_stream()); // no headers, no address extension
    /// h.headers_mut().set_content_type("application/json");
    /// assert_eq!(h.mime_type().essence(), "application/json"); // headers win
    /// ```
    fn mime_type(&self) -> MimeType {
        self.headers()
            .mime_type()
            .unwrap_or_else(|| self.uri().mime_type())
    }

    /// The full **[`MediaType`]** of this source: the media the `Content-Type` /
    /// `Content-Encoding` [`headers`](IOBase::headers) declare, else inferred from the
    /// [`uri`](IOBase::uri)'s extensions, else the single `application/octet-stream` fallback.
    fn media_type(&self) -> MediaType {
        if let Some(media) = self.headers().media_type() {
            return media;
        }
        let from_uri = self.uri().media_type();
        if from_uri.is_empty() {
            MediaType::of(MimeType::octet_stream())
        } else {
            from_uri
        }
    }

    /// Resolves the media type **and stores it** in the source's headers when `Content-Type`
    /// is not already set — memoizing the inference so later reads come straight from
    /// [`headers`](IOBase::headers). Returns the effective [`MimeType`]. The "store optimally"
    /// entry point: it writes only when the header is absent.
    fn ensure_content_type(&mut self) -> MimeType {
        if let Some(declared) = self.headers().mime_type() {
            return declared;
        }
        let inferred = self.uri().mime_type();
        self.headers_mut().set_mime_type(&inferred);
        inferred
    }

    /// This source's content length in bytes, **preferring the cached `Content-Length`
    /// [`header`](IOBase::headers)** when present and falling back to the live
    /// [`byte_size`](IOBase::byte_size). For a source whose true size is an expensive probe (an
    /// object-store prefix summing a subtree, a network body sized by a prior `HEAD`) the header
    /// short-circuits it; for an in-memory source with no such header it is exactly `byte_size`.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut h = Heap::from_slice(b"abc");
    /// assert_eq!(h.content_length(), 3); // no header — falls back to byte_size
    /// h.headers_mut().set_content_length(999); // a cheap probe cached the size
    /// assert_eq!(h.content_length(), 999); // now served straight from the header
    /// ```
    fn content_length(&self) -> u64 {
        self.headers()
            .content_length()
            .unwrap_or_else(|| self.byte_size())
    }

    // ---------------------------------------------------------------------------------
    // Magic inference — read the head with a positioned read, never moving the cursor.
    // ---------------------------------------------------------------------------------

    /// The **primary [`MimeType`]** inferred from this source's **magic bytes** — a positioned
    /// read of the head (**never** moves the cursor), so it works mid-stream. Falls back to the
    /// declared/address [`mime_type`](IOBase::mime_type) when no magic matches.
    fn infer_mime_type(&self) -> MimeType {
        let mut head = [0u8; 32];
        let n = self.pread_byte_array(0, &mut head); // positioned — no cursor seek
        MimeType::from_magic(&head[..n]).unwrap_or_else(|| self.mime_type())
    }

    /// The full **[`MediaType`]** inferred by **recursive magic** — the head's type, then the
    /// type inside each compression layer it can peel (see
    /// [`MediaType::infer_from_head`](crate::mediatype::MediaType::infer_from_head)). A gzipped
    /// tar reads as `[application/gzip, application/x-tar]`. The head is read positioned (no
    /// cursor seek); the address is the outermost fallback.
    fn infer_media_type(&self) -> MediaType {
        let want = (64 * 1024).min(self.byte_size() as usize).max(1);
        let head = self.pread_vec(0, want);
        MediaType::infer_from_head(&head, Some(self.mime_type()))
    }

    // ---------------------------------------------------------------------------------
    // Compression — run a codec over the source's bytes, zero-copy on the read side.
    // ---------------------------------------------------------------------------------

    /// This source's bytes as one **borrowed slice**, when it has a contiguous backing (a
    /// [`Heap`](super::Heap), a mapped file) — the zero-copy read the compression helpers use
    /// to hand the codec the bytes directly. `None` for a source with no contiguous view (an
    /// ad-hoc `LocalIO` read), which then copies through [`pread_vec`](IOBase::pread_vec).
    fn as_bytes(&self) -> Option<&[u8]> {
        None
    }

    /// A boxed [`Compression`] codec for this source's media type, or `None` when the type is
    /// not a (supported) compression — see
    /// [`codec_for_mime`](crate::compression::codec_for_mime).
    fn compression(&self) -> Option<Box<dyn Compression>> {
        codec_for_mime(&self.mime_type())
    }

    /// This source's whole content **compressed** with `codec` — zero-copy on the read side
    /// ([`as_bytes`](IOBase::as_bytes) when available, else one `pread_vec` copy).
    fn compressed_with(&self, codec: &dyn Compression) -> Result<Vec<u8>, IoError> {
        match self.as_bytes() {
            Some(bytes) => codec.compress(bytes),
            None => codec.compress(&self.pread_vec(0, self.byte_size() as usize)),
        }
    }

    /// This source's whole content **decompressed** with `codec`, zero-copy on the read side.
    fn decompressed_with(&self, codec: &dyn Compression) -> Result<Vec<u8>, IoError> {
        match self.as_bytes() {
            Some(bytes) => codec.decompress(bytes),
            None => codec.decompress(&self.pread_vec(0, self.byte_size() as usize)),
        }
    }

    /// Compresses this source's bytes with `codec` **into** `dst` (written from offset 0),
    /// returning the compressed length. `dst` grows to fit.
    fn compress_into<D: IOBase>(
        &self,
        codec: &dyn Compression,
        dst: &mut D,
    ) -> Result<u64, IoError> {
        let out = self.compressed_with(codec)?;
        dst.pwrite_all(0, &out)?;
        Ok(out.len() as u64)
    }

    /// Decompresses this source's bytes with `codec` **into** `dst` (from offset 0), returning
    /// the decompressed length.
    fn decompress_into<D: IOBase>(
        &self,
        codec: &dyn Compression,
        dst: &mut D,
    ) -> Result<u64, IoError> {
        let out = self.decompressed_with(codec)?;
        dst.pwrite_all(0, &out)?;
        Ok(out.len() as u64)
    }

    /// Decompresses this source using the codec inferred from its **media type** (the
    /// "compression optional from the media type" path), returning the plain bytes. Errors with
    /// a guided [`IoError::Compression`] when the source is not a supported compression (or the
    /// `compression` feature is off).
    fn decompress(&self) -> Result<Vec<u8>, IoError> {
        match self.compression() {
            Some(codec) => self.decompressed_with(&*codec),
            None => Err(compression_err(
                self.mime_type().essence(),
                "decompress",
                "the source's media type is not a supported compression (enable the \
                 `compression` feature for gzip/zstd/xz/zlib)",
            )),
        }
    }

    // ---------------------------------------------------------------------------------
    // Content mutation — resize, in-place (de)compress, cross-source copy — each keeping
    // the size/media/mtime headers in sync (only the headers already declared).
    // ---------------------------------------------------------------------------------

    /// Sets this source's byte length to exactly `len` — shrinking (dropping the tail) or
    /// extending (zero-filling) — then syncs its size headers. The default reports that a
    /// fixed source (a bare wrapper/view) cannot be resized; the growable sources
    /// (`Heap` / `Mmap` / `LocalIO`) override it.
    fn truncate(&mut self, len: u64) -> Result<(), IoError> {
        let _ = len;
        Err(IoError::FileIo {
            op: "truncate",
            path: self.uri().to_string(),
            detail: "this source cannot be resized; truncate a Heap, Mmap, or LocalIO instead"
                .to_string(),
        })
    }

    /// **Releases any optimized/cached backing** — memory-mappings, open OS handles — returning
    /// the node to its lazy state so it can be removed or rebound, without discarding its value
    /// (the bytes stay on the backing). The default is a no-op (an in-memory source holds no
    /// releasable handle); [`LocalIO`](crate::io::local::LocalIO) drops its mapping here (which is
    /// why [`move_into`](IOBase::move_into) can then delete the file even on platforms that refuse
    /// to unlink a mapped file). Idempotent.
    fn close(&mut self) {}

    /// Syncs the size + timestamp headers after a content-mutating op — updates
    /// `Content-Length` to the current [`byte_size`](IOBase::byte_size) and `mtime` to now,
    /// but only the headers the source **already declares** (a bare source stays bare). The
    /// `set_*` renders are allocation-free.
    fn sync_size_headers(&mut self) {
        if self.headers().contains(Headers::CONTENT_LENGTH) {
            let size = self.byte_size();
            self.headers_mut().set_content_length(size);
        }
        if self.headers().contains(Headers::MTIME) {
            self.headers_mut().touch_mtime();
        }
    }

    /// **Compresses this source in place** — replaces its bytes with the compressed form and
    /// updates `Content-Type` to the codec, `Content-Length`, and `mtime`. `codec` defaults to
    /// the codec of the source's own [media type](IOBase::media_type) (so a `.gz`-addressed
    /// source packs itself gzip); pass an explicit one to override. Zero-copy on the read side.
    fn compress_in_place(&mut self, codec: Option<&dyn Compression>) -> Result<(), IoError> {
        let owned = codec.is_none().then(|| self.compression()).flatten();
        let codec =
            match codec.or(owned.as_deref()) {
                Some(c) => c,
                None => return Err(compression_err(
                    self.mime_type().essence(),
                    "compress",
                    "no codec: the source's media type is not a compression — pass an explicit \
                     codec, or address it as .gz/.zst/.xz/.zz",
                )),
            };
        let essence = codec.essence();
        let packed = self.compressed_with(codec)?;
        self.overwrite_with(&packed)?;
        self.headers_mut().set_content_type(essence);
        Ok(())
    }

    /// **Decompresses this source in place** — replaces its compressed bytes with the plain
    /// content (codec inferred from its media type) and updates `Content-Type` to the recovered
    /// inner type, `Content-Length`, and `mtime`. Errors when the source is not a supported
    /// compression.
    fn decompress_in_place(&mut self) -> Result<(), IoError> {
        let plain = self.decompress()?;
        let inner = MimeType::from_magic(&plain).map(|m| m.essence().to_string());
        self.overwrite_with(&plain)?;
        match inner {
            Some(essence) => self.headers_mut().set_content_type(&essence),
            None => {
                self.headers_mut().remove(Headers::CONTENT_TYPE);
            }
        }
        Ok(())
    }

    /// Replaces this source's whole content with `data` (write, then truncate to length) and
    /// syncs the size headers — the shared overwrite the in-place ops and [`copy_from`](IOBase::copy_from)
    /// use.
    fn overwrite_with(&mut self, data: &[u8]) -> Result<(), IoError> {
        self.pwrite_all(0, data)?;
        self.truncate(data.len() as u64)?; // drop any old tail past the new content
        if self.headers().contains(Headers::CONTENT_LENGTH) {
            let size = self.byte_size();
            self.headers_mut().set_content_length(size);
        }
        if self.headers().contains(Headers::MTIME) {
            self.headers_mut().touch_mtime();
        }
        Ok(())
    }

    /// Overwrites this source with **all of `src`'s bytes** (truncating to match) — a
    /// cross-source copy, zero-copy on the read side via [`as_bytes`](IOBase::as_bytes) when
    /// `src` has a contiguous view, else one buffered read. Returns the byte count.
    fn copy_from<S: IOBase>(&mut self, src: &S) -> Result<u64, IoError> {
        match src.as_bytes() {
            Some(bytes) => {
                self.overwrite_with(bytes)?;
                Ok(bytes.len() as u64)
            }
            None => {
                let bytes = src.pread_vec(0, src.byte_size() as usize);
                self.overwrite_with(&bytes)?;
                Ok(bytes.len() as u64)
            }
        }
    }

    /// **Positioned cross-source write**: copies `len` bytes of `src` starting at `src_offset`
    /// into this source at `offset`. Zero-copy when `src` exposes a contiguous slice; otherwise
    /// **streamed** through one reused buffer (no full materialization of a large transfer).
    /// Returns the number of bytes actually transferred (short at the end of `src`).
    fn pwrite_from<S: IOBase>(
        &mut self,
        offset: u64,
        src: &S,
        src_offset: u64,
        len: u64,
    ) -> Result<u64, IoError> {
        if let Some(bytes) = src.as_bytes() {
            let start = (src_offset as usize).min(bytes.len());
            let end = start.saturating_add(len as usize).min(bytes.len());
            self.pwrite_all(offset, &bytes[start..end])?;
            return Ok((end - start) as u64);
        }
        let mut buf = vec![0u8; (len as usize).clamp(1, 64 * 1024)];
        let mut done = 0u64;
        while done < len {
            let want = ((len - done) as usize).min(buf.len());
            let got = src.pread_byte_array(src_offset + done, &mut buf[..want]);
            if got == 0 {
                break; // end of src
            }
            self.pwrite_all(offset + done, &buf[..got])?;
            done += got as u64;
        }
        Ok(done)
    }

    /// **Moves** this source's whole content into `dst` and **removes this source** — a copy
    /// that consumes its origin, `mv` over the byte contract. Returns the number of bytes moved.
    ///
    /// - **Same-address no-op.** When `self` and `dst` resolve to the **same** [`uri`](IOBase::uri)
    ///   the move does nothing (neither copies nor deletes) — a file never moves onto itself.
    /// - **Streamed, tail-consuming.** The bytes transfer in bounded chunks read from the
    ///   **tail**; after each chunk lands in `dst`, `self` is [`truncate`](IOBase::truncate)d to
    ///   drop it, so peak memory is **one chunk**, not the whole payload (best-effort — a source
    ///   that cannot shrink still moves correctly, just without the tail-drop). `dst` is then
    ///   truncated to the moved length and its size headers synced.
    /// - **Removal.** Afterwards the emptied source is [`rm`](IOBase::rm)'d; a source with no
    ///   removable backing (a bare [`Heap`](super::Heap)) simply ends **empty**.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut src = Heap::from_slice(b"relocate me");
    /// let mut dst = Heap::new();
    /// assert_eq!(src.move_into(&mut dst).unwrap(), 11);
    /// assert_eq!(dst.pread_vec(0, 11), b"relocate me");
    /// assert_eq!(src.byte_size(), 0); // the source is emptied
    /// ```
    fn move_into<D: IOBase>(&mut self, dst: &mut D) -> Result<u64, IoError> {
        // Moving onto the *same real address* is a no-op (a file never moves onto itself). The
        // synthetic `mem://heap` sentinel is excluded: distinct anonymous buffers share it yet are
        // genuinely different sources, so they still move.
        let src_uri = self.uri();
        if src_uri == dst.uri() && src_uri != *default_uri() {
            return Ok(self.byte_size());
        }
        let total = self.byte_size();
        if let Some(bytes) = self.as_bytes() {
            // Zero-copy fast path: a contiguous source (a `Heap`, a mapped file) hands its bytes
            // straight to the destination — no scratch allocation, no double copy — matching
            // `copy_from` / `pwrite_from`. `overwrite_with` truncates dst to `total` + syncs its
            // size headers.
            dst.overwrite_with(bytes)?;
        } else {
            // Streamed, tail-consuming: bounded chunks read from the tail; each moved chunk is
            // dropped from the source so peak memory is one chunk, not the whole payload.
            let mut buf = vec![0u8; (total as usize).clamp(1, 64 * 1024)];
            let mut remaining = total;
            while remaining > 0 {
                let take = remaining.min(buf.len() as u64);
                let start = remaining - take; // the tail block [start, start + take)
                let got = self.pread_byte_array(start, &mut buf[..take as usize]);
                dst.pwrite_all(start, &buf[..got])?;
                let _ = self.truncate(start); // best-effort: shed the moved tail to cap peak memory
                remaining = start;
            }
            dst.truncate(total)?; // drop any of dst's old content past the moved length
            if dst.headers().contains(Headers::CONTENT_LENGTH) {
                dst.headers_mut().set_content_length(total);
            }
            if dst.headers().contains(Headers::MTIME) {
                dst.headers_mut().touch_mtime();
            }
        }
        let _ = self.truncate(0); // empty the source (the streamed path already did this per chunk)
        self.close(); // release any mapping/handle first so the backing can be unlinked
        let _ = self.rm(true); // remove the emptied backing; a bare Heap has none and stays empty
        Ok(total)
    }

    // ---------------------------------------------------------------------------------
    // The graph surface — every source is a node in an IO graph
    // ---------------------------------------------------------------------------------

    /// The streamed one-level child iterator of [`ls`](IOBase::ls) — items are the **same
    /// source type**, so graphs stay homogeneous whatever node you start from. A leaf
    /// source declares [`NoChildren`].
    type Children: Iterator<Item = Result<Self, IoError>>;

    /// The streamed recursive walker of [`ls_recursive`](IOBase::ls_recursive) — same item
    /// type as [`Children`](IOBase::Children).
    type Walk: Iterator<Item = Result<Self, IoError>>;

    /// The node's own name — by default the last segment of its address's path (empty when
    /// the address has none, e.g. the synthetic `mem://heap`); a filesystem node overrides
    /// it with its real file name. The segment is **percent-decoded** so a wrapper over a
    /// spaced path reports `my file.txt`, never the encoded `my%20file.txt`.
    fn name(&self) -> String {
        let uri = self.uri();
        let segment = uri
            .path()
            .rsplit('/')
            .find(|segment| !segment.is_empty())
            .unwrap_or("");
        crate::uri::percent::decode(segment).into_owned()
    }

    /// The parent node, or `None` — the default for a leaf source or a root.
    fn parent(&self) -> Option<Self> {
        None
    }

    /// An iterator over this node's **ancestors**, nearest first — the repeated
    /// [`parent`](IOBase::parent) chain up to the root. The node-graph counterpart of
    /// [`Uri::parents`](crate::uri::Uri::parents).
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let node = Heap::new().join("a/b/c.bin").unwrap();
    /// let uris: Vec<String> = node.parents().map(|p| p.uri().to_string()).collect();
    /// assert_eq!(uris, vec!["mem://heap/a/b", "mem://heap/a", "mem://heap"]);
    /// ```
    fn parents(&self) -> impl Iterator<Item = Self> {
        std::iter::successors(self.parent(), Self::parent)
    }

    /// The child node at `segment` — a **new node of the same kind**, addressed by joining
    /// `segment` onto this node's address with [`Uri::joinpath`](crate::uri::Uri::joinpath),
    /// so navigation composes through the URI: the child's [`parent`](IOBase::parent)
    /// addresses this node again. `segment` may be multi-segment (`"a/b/c"`); an **absolute**
    /// segment (leading `/`) re-roots (the URI merge/join algebra, shared with `Uri`/`Url`).
    ///
    /// Constructing a child touches nothing — it is pure address algebra. Reading or writing
    /// the returned node is what actually creates or opens its backing (e.g. a `LocalIO`
    /// child auto-creates on first write). The default reports that this source has no
    /// navigable child space (a bare byte view or wrapper); [`Heap`](super::Heap) and the
    /// local family ([`LocalIO`](crate::io::local::LocalIO)) build a real child.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let root = Heap::new();
    /// let child = root.join("logs/app.bin").unwrap();
    /// assert_eq!(child.uri().to_string(), "mem://heap/logs/app.bin");
    /// assert_eq!(child.parent().unwrap().uri().to_string(), "mem://heap/logs");
    /// ```
    fn join(&self, segment: &str) -> Result<Self, IoError> {
        let _ = segment;
        Err(IoError::FileIo {
            op: "join",
            path: self.uri().to_string(),
            detail: "this source has no child path space to join onto; address a filesystem \
                     node (LocalIO) or an in-heap node (Heap)"
                .to_string(),
        })
    }

    /// Streams this node's **direct children**, lazily — each item is produced as the
    /// caller pulls, never a pre-collected tree. A leaf source (or a missing / file node)
    /// streams nothing; a real listing failure is a guided [`IoError::FileIo`].
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// assert_eq!(Heap::new().ls().unwrap().count(), 0); // a heap is a leaf
    /// ```
    fn ls(&self) -> Result<Self::Children, IoError>;

    /// Streams the node's **entire subtree** (depth-first), lazily — the recursive
    /// counterpart of [`ls`](IOBase::ls); the bindings expose both through one generic
    /// `ls(recursive=…)` entry point.
    fn ls_recursive(&self) -> Result<Self::Walk, IoError>;

    /// The direct children, collected — the convenience over the streamed
    /// [`ls`](IOBase::ls).
    fn children(&self) -> Result<Vec<Self>, IoError> {
        self.ls()?.collect()
    }

    /// Removes **whatever exists** at this node — a file is unlinked, a directory removed
    /// with its subtree. `exist_ok` (default `true` in the bindings) governs a **missing**
    /// node: `true` skips it (a no-op), `false` raises a guided [`IoError::FileIo`]. The
    /// default is the guided refusal of a source with no removable backing; filesystem
    /// families override it.
    fn rm(&self, exist_ok: bool) -> Result<(), IoError> {
        let _ = exist_ok;
        Err(unremovable(&self.uri(), "rm"))
    }

    /// Removes this node **as a file** — a guided error when it is a directory (use
    /// [`rmdir`](IOBase::rmdir)); a missing node is skipped when `exist_ok`, else raises.
    /// Default: the guided refusal.
    fn rmfile(&self, exist_ok: bool) -> Result<(), IoError> {
        let _ = exist_ok;
        Err(unremovable(&self.uri(), "rmfile"))
    }

    /// Removes this node **as a directory**, recursively — a guided error when it is a file
    /// (use [`rmfile`](IOBase::rmfile)); a missing node is skipped when `exist_ok`, else
    /// raises. Default: the guided refusal.
    fn rmdir(&self, exist_ok: bool) -> Result<(), IoError> {
        let _ = exist_ok;
        Err(unremovable(&self.uri(), "rmdir"))
    }

    // ---------------------------------------------------------------------------------
    // The memory tree — a container node served as one contiguous byte region
    // ---------------------------------------------------------------------------------

    /// The **memory-tree size** of a container node: the lazy, streamed sum of every child
    /// block's [`byte_size`](IOBase::byte_size) (a child container recurses through its own
    /// `byte_size`). Nothing is collected and nothing is cached — the size is recomputed
    /// per call from the live tree. Written once here so every filesystem family serves
    /// container sizes identically.
    ///
    /// DESIGN: an **erroring** child (one whose listing yields `Err`) is skipped, so a tree
    /// with an unreadable entry reports the size of the readable remainder rather than
    /// failing. Cycle safety (a directory symlink pointing at an ancestor) is the family's
    /// concern: it belongs to [`blocks`](IOBase::blocks), which a family overrides to keep
    /// the layout acyclic (the local family drops symlinked directories there).
    fn tree_byte_size(&self) -> u64 {
        match self.ls() {
            Ok(children) => children
                .filter_map(Result::ok)
                .map(|child| child.byte_size())
                .sum(),
            Err(_) => 0,
        }
    }

    /// The node's direct children as **name-sorted blocks** — the deterministic order the
    /// memory-tree byte layout uses (listing order is OS-dependent; names are not). One
    /// level is collected and sorted per call, never the whole tree.
    fn blocks(&self) -> Vec<Self> {
        let mut blocks: Vec<Self> = match self.ls() {
            Ok(children) => children.filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        };
        blocks.sort_by_key(|block| block.name());
        blocks
    }

    /// **Memory-tree read**: serves [`pread_byte_array`](IOBase::pread_byte_array) for a
    /// container node by reading across its name-sorted child [`blocks`](IOBase::blocks)
    /// as one contiguous byte region — a child container recurses through its own
    /// `pread_byte_array`, so the whole subtree reads as one lazily-computed buffer.
    /// Blocks before `offset` are skipped by size alone; nothing is materialized.
    fn tree_pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize {
        let mut done = 0usize;
        let mut block_start = 0u64;
        for block in self.blocks() {
            if done == buf.len() {
                break;
            }
            let block_end = block_start + block.byte_size();
            let read_at = offset + done as u64;
            // Only read where the target lands inside this block. `read_at < block_start`
            // means an earlier block short-read and left a hole — the region is no longer
            // contiguous, so stop (never underflow `read_at - block_start`).
            if read_at < block_start {
                break;
            }
            if read_at < block_end {
                done += block.pread_byte_array(read_at - block_start, &mut buf[done..]);
            }
            block_start = block_end;
        }
        done
    }

    /// **Memory-tree write**: routes [`pwrite_byte_array`](IOBase::pwrite_byte_array) for a
    /// container node across its name-sorted child [`blocks`](IOBase::blocks). A write
    /// inside a block stays **capped at that block's end** (a middle block never grows —
    /// the layout would shift); bytes past the last block grow the **last** block. A
    /// container with no blocks writes nothing (the full writes report the guided fix).
    fn tree_pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize {
        let mut blocks = self.blocks();
        let Some(last) = blocks.len().checked_sub(1) else {
            return 0;
        };
        let mut done = 0usize;
        let mut block_start = 0u64;
        for (i, block) in blocks.iter_mut().enumerate() {
            if done == data.len() {
                break;
            }
            let block_end = block_start + block.byte_size();
            let write_at = offset + done as u64;
            // A hole from an earlier short write breaks contiguity — stop before underflow.
            if write_at < block_start {
                break;
            }
            if write_at < block_end || i == last {
                let chunk_end = if i == last {
                    data.len()
                } else {
                    // `write_at <= block_end` here (write_at < block_end), so the cap is safe.
                    done + ((block_end - write_at) as usize).min(data.len() - done)
                };
                let written =
                    block.pwrite_byte_array(write_at - block_start, &data[done..chunk_end]);
                done += written;
                if done < chunk_end {
                    break; // the block refused (e.g. read-only) — stop, report the shortfall
                }
            }
            block_start = block_end;
        }
        done
    }

    /// **Positioned read** (primitive). Copies up to `buf.len()` bytes starting at `offset` into
    /// `buf`, returning the number copied — `0` at or past the end, a short count near it. Never
    /// moves a cursor.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(b"hello world");
    /// let mut buf = [0u8; 5];
    /// assert_eq!(data.pread_byte_array(6, &mut buf), 5);
    /// assert_eq!(&buf, b"world");
    /// assert_eq!(data.pread_byte_array(11, &mut buf), 0); // at the end -> nothing
    /// ```
    fn pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize;

    /// **Positioned write** (primitive). Copies `data` in at `offset`, growing the storage (and
    /// zero-filling any gap between the old end and `offset`) as needed. Returns the number of
    /// bytes written — always `data.len()`.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut data = Heap::from_slice(b"abc");
    /// assert_eq!(data.pwrite_byte_array(5, b"Z"), 1); // writes past the end, zero-filling the gap
    /// assert_eq!(data.as_slice(), b"abc\0\0Z");
    /// ```
    fn pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize;

    /// **Full positioned read.** Fills *all* of `buf` starting at `offset`, or errors with
    /// [`IoError::UnexpectedEof`] naming the shortfall if fewer bytes remain.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(b"hello");
    /// let mut buf = [0u8; 3];
    /// data.pread_exact(1, &mut buf).unwrap();
    /// assert_eq!(&buf, b"ell");
    /// assert!(data.pread_exact(3, &mut [0u8; 5]).is_err()); // only 2 remain
    /// ```
    fn pread_exact(&self, offset: u64, buf: &mut [u8]) -> Result<(), IoError> {
        let read = self.pread_byte_array(offset, buf);
        if read == buf.len() {
            Ok(())
        } else {
            Err(IoError::UnexpectedEof {
                offset: offset + read as u64,
                requested: buf.len(),
                available: read,
            })
        }
    }

    /// **Full positioned write** of *all* of `data` at `offset` — the counterpart of
    /// [`pread_exact`](IOBase::pread_exact). Errors with [`IoError::UnexpectedEof`] (naming the
    /// shortfall) when the sink could not take every byte — a bounded window
    /// ([`IOSlice`](super::IOSlice)) clamps at its edge, and even a growable source refuses an
    /// offset so large the write would overflow the address space. For an ordinary in-heap write
    /// this always succeeds (the storage grows to fit).
    fn pwrite_all(&mut self, offset: u64, data: &[u8]) -> Result<(), IoError> {
        let written = self.pwrite_byte_array(offset, data);
        if written == data.len() {
            Ok(())
        } else {
            Err(IoError::UnexpectedEof {
                offset: offset + written as u64,
                requested: data.len(),
                available: written,
            })
        }
    }

    /// Reads up to `len` bytes at `offset` into a fresh `Vec` (short near the end) — the owning
    /// read for callers that do not bring their own buffer. One allocation, **pre-sized to what
    /// is actually available** (never to the raw request), so a hostile or corrupt `len` cannot
    /// trigger a runaway allocation.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(b"hello world");
    /// assert_eq!(data.pread_vec(6, 100), b"world"); // clamped to what remains
    /// assert_eq!(data.pread_vec(6, usize::MAX), b"world"); // no giant up-front allocation
    /// ```
    fn pread_vec(&self, offset: u64, len: usize) -> Vec<u8> {
        let available = self.byte_size().saturating_sub(offset).min(len as u64) as usize;
        let mut buf = vec![0u8; available];
        let read = self.pread_byte_array(offset, &mut buf);
        buf.truncate(read);
        buf
    }

    /// Reads up to `len` bytes at `offset` into `dst`, **reusing `dst`'s existing allocation** —
    /// the allocation-free bulk read for a transfer loop that reads chunk after chunk into one
    /// scratch buffer. `dst` is cleared, grown once to fit only if its capacity is too small
    /// (its spare capacity is reused otherwise), and filled; returns the number of bytes read
    /// (short near the end). Unlike [`pread_vec`](IOBase::pread_vec) — a fresh `Vec` every call —
    /// this keeps a caller's buffer hot across a whole transfer.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let src = Heap::from_slice(b"hello world");
    /// let mut scratch = Vec::new();
    /// assert_eq!(src.pread_into(0, 5, &mut scratch), 5);
    /// assert_eq!(&scratch, b"hello");
    /// let cap = scratch.capacity();
    /// assert_eq!(src.pread_into(6, 5, &mut scratch), 5); // reuses the allocation
    /// assert_eq!(&scratch, b"world");
    /// assert_eq!(scratch.capacity(), cap);
    /// ```
    fn pread_into(&self, offset: u64, len: usize, dst: &mut Vec<u8>) -> usize {
        // Size to what is actually available (never the raw request), so a hostile `len`
        // cannot force a runaway grow; reuses `dst`'s capacity when it already fits.
        let available = self.byte_size().saturating_sub(offset).min(len as u64) as usize;
        dst.clear();
        dst.resize(available, 0);
        let read = self.pread_byte_array(offset, dst);
        dst.truncate(read);
        read
    }

    /// Reads the single byte at `offset`, or errors with [`IoError::UnexpectedEof`] if it is past
    /// the end.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(b"abc");
    /// assert_eq!(data.pread_byte(1).unwrap(), b'b');
    /// assert!(data.pread_byte(3).is_err());
    /// ```
    fn pread_byte(&self, offset: u64) -> Result<u8, IoError> {
        let mut buf = [0u8; 1];
        self.pread_exact(offset, &mut buf)?;
        Ok(buf[0])
    }

    /// Writes the single byte `value` at `offset`, growing the storage as needed.
    fn pwrite_byte(&mut self, offset: u64, value: u8) -> Result<(), IoError> {
        self.pwrite_all(offset, &[value])
    }

    /// Reads the bit at absolute **bit** `offset` (LSB-first: bit `offset % 8` of byte
    /// `offset / 8`), or errors with [`IoError::UnexpectedEof`] if its byte is past the end.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(&[0b0000_0101]);
    /// assert!(data.pread_bit(0).unwrap());  // bit 0 set
    /// assert!(!data.pread_bit(1).unwrap()); // bit 1 clear
    /// assert!(data.pread_bit(2).unwrap());  // bit 2 set
    /// ```
    fn pread_bit(&self, offset: u64) -> Result<bool, IoError> {
        let byte = self.pread_byte(offset / 8)?;
        Ok((byte >> (offset % 8)) & 1 != 0)
    }

    /// Sets or clears the bit at absolute **bit** `offset` (LSB-first), read-modify-writing its
    /// byte and growing the storage (zero-filled) if the bit is past the end.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut data = Heap::new();
    /// data.pwrite_bit(10, true).unwrap();      // grows to 2 bytes, sets bit 2 of byte 1
    /// assert_eq!(data.as_slice(), &[0b0000_0000, 0b0000_0100]);
    /// assert!(data.pread_bit(10).unwrap());
    /// ```
    fn pwrite_bit(&mut self, offset: u64, value: bool) -> Result<(), IoError> {
        let byte_offset = offset / 8;
        let mask = 1u8 << (offset % 8);
        let mut buf = [0u8; 1];
        self.pread_byte_array(byte_offset, &mut buf); // 0 if past the end (about to grow)
        if value {
            buf[0] |= mask;
        } else {
            buf[0] &= !mask;
        }
        self.pwrite_all(byte_offset, &buf)
    }

    /// Reads a little-endian `i32` (4 bytes) at `offset`, or errors with
    /// [`IoError::UnexpectedEof`].
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(&(-42i32).to_le_bytes());
    /// assert_eq!(data.pread_i32(0).unwrap(), -42);
    /// ```
    fn pread_i32(&self, offset: u64) -> Result<i32, IoError> {
        let mut buf = [0u8; 4];
        self.pread_exact(offset, &mut buf)?;
        Ok(i32::from_le_bytes(buf))
    }

    /// Writes `value` as a little-endian `i32` (4 bytes) at `offset`, growing as needed.
    fn pwrite_i32(&mut self, offset: u64, value: i32) -> Result<(), IoError> {
        self.pwrite_all(offset, &value.to_le_bytes())
    }

    /// Reads a little-endian `i64` (8 bytes) at `offset`, or errors with
    /// [`IoError::UnexpectedEof`].
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let data = Heap::from_slice(&(1234567890123i64).to_le_bytes());
    /// assert_eq!(data.pread_i64(0).unwrap(), 1234567890123);
    /// ```
    fn pread_i64(&self, offset: u64) -> Result<i64, IoError> {
        let mut buf = [0u8; 8];
        self.pread_exact(offset, &mut buf)?;
        Ok(i64::from_le_bytes(buf))
    }

    /// Writes `value` as a little-endian `i64` (8 bytes) at `offset`, growing as needed.
    fn pwrite_i64(&mut self, offset: u64, value: i64) -> Result<(), IoError> {
        self.pwrite_all(offset, &value.to_le_bytes())
    }

    /// Reads up to `len` **bytes** at `offset` and decodes them as UTF-8 text (clamped near the
    /// end, like [`pread_vec`](IOBase::pread_vec)), or errors with [`IoError::InvalidUtf8`]
    /// naming the offending byte — including a multi-byte character cut by the range. Built on
    /// the byte primitives, so every source inherits it.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut data = Heap::new();
    /// data.pwrite_utf8(0, "héllo");
    /// assert_eq!(data.pread_utf8(0, 6).unwrap(), "héllo"); // é is 2 bytes
    /// assert!(data.pread_utf8(0, 2).is_err());             // cuts é in half
    /// ```
    fn pread_utf8(&self, offset: u64, len: usize) -> Result<String, IoError> {
        String::from_utf8(self.pread_vec(offset, len)).map_err(|error| IoError::InvalidUtf8 {
            position: error.utf8_error().valid_up_to(),
        })
    }

    /// Writes `text`'s UTF-8 bytes at `offset` (growing as needed); returns the number of
    /// **bytes** written. The exact writing counterpart of [`pread_utf8`](IOBase::pread_utf8).
    fn pwrite_utf8(&mut self, offset: u64, text: &str) -> usize {
        self.pwrite_byte_array(offset, text.as_bytes())
    }

    /// **Bulk typed read.** Fills *all* of `dst` with little-endian `i32`s starting at `offset`,
    /// or errors with [`IoError::UnexpectedEof`]. Stages through a fixed stack chunk — zero heap
    /// allocation — and converts each chunk in a dense, branch-free loop the compiler
    /// vectorizes.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut data = Heap::new();
    /// data.pwrite_i32_array(0, &[1, -2, 3]).unwrap();
    /// let mut values = [0i32; 3];
    /// data.pread_i32_array(0, &mut values).unwrap();
    /// assert_eq!(values, [1, -2, 3]);
    /// ```
    fn pread_i32_array(&self, offset: u64, dst: &mut [i32]) -> Result<(), IoError> {
        stage_pread_i32_array(self, offset, dst)
    }

    /// **Bulk typed write.** Writes all of `src` as little-endian `i32`s at `offset` (growing
    /// as needed). Stages through a fixed stack chunk — zero heap allocation, vectorizable.
    fn pwrite_i32_array(&mut self, offset: u64, src: &[i32]) -> Result<(), IoError> {
        stage_pwrite_i32_array(self, offset, src)
    }

    /// **Bulk typed read** of little-endian `i64`s — the wide counterpart of
    /// [`pread_i32_array`](IOBase::pread_i32_array).
    fn pread_i64_array(&self, offset: u64, dst: &mut [i64]) -> Result<(), IoError> {
        stage_pread_i64_array(self, offset, dst)
    }

    /// **Bulk typed write** of little-endian `i64`s — the wide counterpart of
    /// [`pwrite_i32_array`](IOBase::pwrite_i32_array).
    fn pwrite_i64_array(&mut self, offset: u64, src: &[i64]) -> Result<(), IoError> {
        stage_pwrite_i64_array(self, offset, src)
    }

    /// **Repeated-value fill.** Writes `count` copies of the byte `value` starting at `offset`
    /// (growing as needed) — without ever materializing the full array: a fixed stack chunk is
    /// filled once and written repeatedly. The byte-level `memset` of the family.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut data = Heap::new();
    /// data.pwrite_byte_repeat(0, 0xAB, 5).unwrap();
    /// assert_eq!(data.as_slice(), &[0xAB; 5]);
    /// ```
    fn pwrite_byte_repeat(&mut self, offset: u64, value: u8, count: usize) -> Result<(), IoError> {
        stage_pwrite_byte_repeat(self, offset, value, count)
    }

    /// **Repeated-value fill** of `count` little-endian `i32` copies of `value` at `offset` —
    /// no full array is built (one stack chunk, filled once, written repeatedly).
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut data = Heap::new();
    /// data.pwrite_i32_repeat(0, -1, 3).unwrap();
    /// let mut values = [0i32; 3];
    /// data.pread_i32_array(0, &mut values).unwrap();
    /// assert_eq!(values, [-1, -1, -1]);
    /// ```
    fn pwrite_i32_repeat(&mut self, offset: u64, value: i32, count: usize) -> Result<(), IoError> {
        stage_pwrite_i32_repeat(self, offset, value, count)
    }

    /// **Repeated-value fill** of `count` little-endian `i64` copies of `value` at `offset` —
    /// the wide counterpart of [`pwrite_i32_repeat`](IOBase::pwrite_i32_repeat).
    fn pwrite_i64_repeat(&mut self, offset: u64, value: i64, count: usize) -> Result<(), IoError> {
        stage_pwrite_i64_repeat(self, offset, value, count)
    }

    // The unsigned + floating widths — same stack-staged, auto-vectorized kernels as the signed
    // pair above, so every numeric bulk op is one dense loop and every source inherits all of them.
    bulk_numeric_methods!(
        u16,
        pread_u16_array,
        pwrite_u16_array,
        pwrite_u16_repeat,
        stage_pread_u16_array,
        stage_pwrite_u16_array,
        stage_pwrite_u16_repeat
    );
    bulk_numeric_methods!(
        u32,
        pread_u32_array,
        pwrite_u32_array,
        pwrite_u32_repeat,
        stage_pread_u32_array,
        stage_pwrite_u32_array,
        stage_pwrite_u32_repeat
    );
    bulk_numeric_methods!(
        u64,
        pread_u64_array,
        pwrite_u64_array,
        pwrite_u64_repeat,
        stage_pread_u64_array,
        stage_pwrite_u64_array,
        stage_pwrite_u64_repeat
    );
    bulk_numeric_methods!(
        f32,
        pread_f32_array,
        pwrite_f32_array,
        pwrite_f32_repeat,
        stage_pread_f32_array,
        stage_pwrite_f32_array,
        stage_pwrite_f32_repeat
    );
    bulk_numeric_methods!(
        f64,
        pread_f64_array,
        pwrite_f64_array,
        pwrite_f64_repeat,
        stage_pread_f64_array,
        stage_pwrite_f64_array,
        stage_pwrite_f64_repeat
    );
    // The remaining native integer widths as bulk arrays + repeats (i8/i16/i128/u128), so every
    // fixed-width native type has the full positioned-array surface (u8 is `pread_byte_array`).
    bulk_numeric_methods!(
        i8,
        pread_i8_array,
        pwrite_i8_array,
        pwrite_i8_repeat,
        stage_pread_i8_array,
        stage_pwrite_i8_array,
        stage_pwrite_i8_repeat
    );
    bulk_numeric_methods!(
        i16,
        pread_i16_array,
        pwrite_i16_array,
        pwrite_i16_repeat,
        stage_pread_i16_array,
        stage_pwrite_i16_array,
        stage_pwrite_i16_repeat
    );
    bulk_numeric_methods!(
        i128,
        pread_i128_array,
        pwrite_i128_array,
        pwrite_i128_repeat,
        stage_pread_i128_array,
        stage_pwrite_i128_array,
        stage_pwrite_i128_repeat
    );
    bulk_numeric_methods!(
        u128,
        pread_u128_array,
        pwrite_u128_array,
        pwrite_u128_repeat,
        stage_pread_u128_array,
        stage_pwrite_u128_array,
        stage_pwrite_u128_repeat
    );

    // Scalar positioned read/write for every remaining native width — `i32`/`i64`/byte are the
    // hand-written references above; these complete the set so any native value round-trips
    // through one positioned call.
    scalar_numeric_methods!(i8, 1, pread_i8, pwrite_i8);
    scalar_numeric_methods!(u8, 1, pread_u8, pwrite_u8);
    scalar_numeric_methods!(i16, 2, pread_i16, pwrite_i16);
    scalar_numeric_methods!(u16, 2, pread_u16, pwrite_u16);
    scalar_numeric_methods!(u32, 4, pread_u32, pwrite_u32);
    scalar_numeric_methods!(u64, 8, pread_u64, pwrite_u64);
    scalar_numeric_methods!(i128, 16, pread_i128, pwrite_i128);
    scalar_numeric_methods!(u128, 16, pread_u128, pwrite_u128);
    scalar_numeric_methods!(f32, 4, pread_f32, pwrite_f32);
    scalar_numeric_methods!(f64, 8, pread_f64, pwrite_f64);

    /// Wraps this source in an [`IOCursor`] positioned at the start — the standard way to add a
    /// moving read/write position to any source. Consumes the source (zero-copy); wrap a clone to
    /// keep the original.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let mut cur = Heap::from_slice(b"hi").cursor();
    /// assert_eq!(cur.read_byte().unwrap(), b'h');
    /// ```
    fn cursor(self) -> IOCursor<Self>
    where
        Self: Sized,
    {
        IOCursor::new(self)
    }

    /// Wraps this source in an [`IOSlice`] — the bounded window `[offset, offset + len)` addressed
    /// from its own `0`. Errors with [`IoError::SliceOutOfBounds`] if it runs past the end.
    /// Consumes the source (zero-copy); wrap a clone to keep the original.
    ///
    /// ```
    /// use yggdryl_core::io::memory::{Heap, IOBase};
    ///
    /// let win = Heap::from_slice(b"hello world").window(6, 5).unwrap();
    /// assert_eq!(win.pread_vec(0, 5), b"world");
    /// ```
    fn window(self, offset: u64, len: u64) -> Result<IOSlice<Self>, IoError>
    where
        Self: Sized,
    {
        IOSlice::new(self, offset, len)
    }
}

// -------------------------------------------------------------------------------------
// Stack-staged bulk kernels, as free functions — the single source of truth for the trait
// defaults. A source with a faster **contiguous** backing (`Heap` over its `Vec`, `Mmap`
// over its mapping) overrides the trait methods with a direct conversion; a source that
// **composes** another (`LocalIO`'s non-mapped branch, over its ad-hoc / memory-tree byte
// methods) reuses these staged kernels verbatim rather than duplicating them. Each stages
// through one fixed stack chunk (zero heap allocation) and converts in a dense, branch-free
// loop the compiler vectorizes.
// -------------------------------------------------------------------------------------

pub(crate) fn stage_pread_i32_array<S: IOBase>(
    src: &S,
    offset: u64,
    dst: &mut [i32],
) -> Result<(), IoError> {
    let mut bytes = [0u8; BULK_CHUNK * 4];
    let mut position = offset;
    for chunk in dst.chunks_mut(BULK_CHUNK) {
        let staged = &mut bytes[..chunk.len() * 4];
        src.pread_exact(position, staged)?;
        for (value, raw) in chunk.iter_mut().zip(staged.chunks_exact(4)) {
            *value = i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        }
        position += staged.len() as u64;
    }
    Ok(())
}

pub(crate) fn stage_pwrite_i32_array<S: IOBase>(
    dst: &mut S,
    offset: u64,
    src: &[i32],
) -> Result<(), IoError> {
    let mut bytes = [0u8; BULK_CHUNK * 4];
    let mut position = offset;
    for chunk in src.chunks(BULK_CHUNK) {
        let staged = &mut bytes[..chunk.len() * 4];
        for (raw, value) in staged.chunks_exact_mut(4).zip(chunk) {
            raw.copy_from_slice(&value.to_le_bytes());
        }
        dst.pwrite_all(position, staged)?;
        position += staged.len() as u64;
    }
    Ok(())
}

pub(crate) fn stage_pread_i64_array<S: IOBase>(
    src: &S,
    offset: u64,
    dst: &mut [i64],
) -> Result<(), IoError> {
    let mut bytes = [0u8; BULK_CHUNK * 8];
    let mut position = offset;
    for chunk in dst.chunks_mut(BULK_CHUNK) {
        let staged = &mut bytes[..chunk.len() * 8];
        src.pread_exact(position, staged)?;
        for (value, raw) in chunk.iter_mut().zip(staged.chunks_exact(8)) {
            *value = i64::from_le_bytes(raw.try_into().expect("chunks_exact yields 8"));
        }
        position += staged.len() as u64;
    }
    Ok(())
}

pub(crate) fn stage_pwrite_i64_array<S: IOBase>(
    dst: &mut S,
    offset: u64,
    src: &[i64],
) -> Result<(), IoError> {
    let mut bytes = [0u8; BULK_CHUNK * 8];
    let mut position = offset;
    for chunk in src.chunks(BULK_CHUNK) {
        let staged = &mut bytes[..chunk.len() * 8];
        for (raw, value) in staged.chunks_exact_mut(8).zip(chunk) {
            raw.copy_from_slice(&value.to_le_bytes());
        }
        dst.pwrite_all(position, staged)?;
        position += staged.len() as u64;
    }
    Ok(())
}

pub(crate) fn stage_pwrite_byte_repeat<S: IOBase>(
    dst: &mut S,
    offset: u64,
    value: u8,
    count: usize,
) -> Result<(), IoError> {
    let chunk = [value; BULK_CHUNK * 4];
    let mut position = offset;
    let mut remaining = count;
    while remaining > 0 {
        let take = remaining.min(chunk.len());
        dst.pwrite_all(position, &chunk[..take])?;
        position += take as u64;
        remaining -= take;
    }
    Ok(())
}

pub(crate) fn stage_pwrite_i32_repeat<S: IOBase>(
    dst: &mut S,
    offset: u64,
    value: i32,
    count: usize,
) -> Result<(), IoError> {
    let mut chunk = [0u8; BULK_CHUNK * 4];
    for raw in chunk.chunks_exact_mut(4) {
        raw.copy_from_slice(&value.to_le_bytes());
    }
    let mut position = offset;
    let mut remaining = count;
    while remaining > 0 {
        let take = remaining.min(BULK_CHUNK);
        dst.pwrite_all(position, &chunk[..take * 4])?;
        position += (take * 4) as u64;
        remaining -= take;
    }
    Ok(())
}

pub(crate) fn stage_pwrite_i64_repeat<S: IOBase>(
    dst: &mut S,
    offset: u64,
    value: i64,
    count: usize,
) -> Result<(), IoError> {
    let mut chunk = [0u8; BULK_CHUNK * 8];
    for raw in chunk.chunks_exact_mut(8) {
        raw.copy_from_slice(&value.to_le_bytes());
    }
    let mut position = offset;
    let mut remaining = count;
    while remaining > 0 {
        let take = remaining.min(BULK_CHUNK);
        dst.pwrite_all(position, &chunk[..take * 8])?;
        position += (take * 8) as u64;
        remaining -= take;
    }
    Ok(())
}

// The remaining little-endian widths (u16/u32/u64/f32/f64) share one generated kernel each —
// i32/i64 stay hand-written above as the readable reference the macro mirrors.
stage_numeric_kernels!(
    u16,
    2,
    stage_pread_u16_array,
    stage_pwrite_u16_array,
    stage_pwrite_u16_repeat
);
stage_numeric_kernels!(
    u32,
    4,
    stage_pread_u32_array,
    stage_pwrite_u32_array,
    stage_pwrite_u32_repeat
);
stage_numeric_kernels!(
    u64,
    8,
    stage_pread_u64_array,
    stage_pwrite_u64_array,
    stage_pwrite_u64_repeat
);
stage_numeric_kernels!(
    f32,
    4,
    stage_pread_f32_array,
    stage_pwrite_f32_array,
    stage_pwrite_f32_repeat
);
stage_numeric_kernels!(
    f64,
    8,
    stage_pread_f64_array,
    stage_pwrite_f64_array,
    stage_pwrite_f64_repeat
);
stage_numeric_kernels!(
    i8,
    1,
    stage_pread_i8_array,
    stage_pwrite_i8_array,
    stage_pwrite_i8_repeat
);
stage_numeric_kernels!(
    i16,
    2,
    stage_pread_i16_array,
    stage_pwrite_i16_array,
    stage_pwrite_i16_repeat
);
stage_numeric_kernels!(
    i128,
    16,
    stage_pread_i128_array,
    stage_pwrite_i128_array,
    stage_pwrite_i128_repeat
);
stage_numeric_kernels!(
    u128,
    16,
    stage_pread_u128_array,
    stage_pwrite_u128_array,
    stage_pwrite_u128_repeat
);
