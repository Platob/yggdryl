//! [`AnyIO`] and [`open`] — the scheme-dispatching entry point, the project's `open()`.
//!
//! [`open`] takes an address and hands back **one uniform handle** over whichever concrete
//! source the scheme selects — a [`LocalIO`](crate::io::local::LocalIO) for a `file://` (or
//! plain-path) URI, a [`Heap`](crate::io::memory::Heap) for a `mem://` one — so a caller reads,
//! writes, seeks, and navigates without first knowing which backing it got. It is the Rust core's
//! analogue of Python's builtin `open()`; the bindings expose the same entry point, returning the
//! concrete binding type directly (dynamic typing makes the wrapper unnecessary there).

use super::local::LocalIO;
use super::memory::{cursor_methods, Heap, IOBase, IoError, NoChildren, Whence};
use super::{IOKind, IOMode};
use crate::headers::Headers;
use crate::uri::Uri;

/// The concrete source behind an [`AnyIO`] — selected by [`open`] from the address scheme.
#[derive(Debug)]
enum Backend {
    /// A local-filesystem handle (a `file://` or plain-path address).
    Local(LocalIO),
    /// An in-heap buffer (a `mem://` address).
    Memory(Heap),
}

/// A **uniform opened handle** over any source [`open`] can address — the one type a caller holds
/// whatever the scheme resolved to. It implements the full [`IOBase`] byte + cursor contract by
/// delegating to the backing ([`LocalIO`](crate::io::local::LocalIO) / [`Heap`](crate::io::memory::Heap)),
/// and carries its **own** cursor so it streams like an open file. For scheme-specific graph
/// traversal (directory listing) reach the concrete handle with [`into_local`](AnyIO::into_local) /
/// [`as_local`](AnyIO::as_local); as a wrapper `AnyIO` is a discovery **leaf**.
///
/// ```
/// use yggdryl_core::io::memory::IOBase;
/// use yggdryl_core::io::open;
/// use yggdryl_core::uri::Uri;
///
/// // A mem:// address opens an in-heap buffer; write + read it through the uniform handle.
/// let mut io = open(&Uri::parse_str("mem://heap").unwrap()).unwrap();
/// io.pwrite_utf8(0, "opened");
/// assert_eq!(io.pread_utf8(0, 6).unwrap(), "opened");
/// assert!(io.is_memory());
/// ```
#[derive(Debug)]
pub struct AnyIO {
    backend: Backend,
    /// The uniform cursor over the backing's bytes (independent of the backing's own cursor).
    position: u64,
}

/// Dispatches an expression over whichever backend an [`AnyIO`] holds, binding it to `$x`.
macro_rules! by_backend {
    ($self:expr, $x:ident => $body:expr) => {
        match &$self.backend {
            Backend::Local($x) => $body,
            Backend::Memory($x) => $body,
        }
    };
    (mut $self:expr, $x:ident => $body:expr) => {
        match &mut $self.backend {
            Backend::Local($x) => $body,
            Backend::Memory($x) => $body,
        }
    };
}

/// Forwards every typed bulk array + repeat through [`by_backend!`] to the concrete backend, so a
/// `mem://` [`Heap`] and a mapped `file://` [`LocalIO`] both reach their **fast contiguous
/// overrides** — otherwise `open()`'s uniform handle would silently take the stack-staged default
/// (an extra copy) on the project's primary I/O entry point.
macro_rules! forward_bulk_backend {
    () => {
        forward_bulk_backend!(@a pread_i32_array, pwrite_i32_array, i32);
        forward_bulk_backend!(@a pread_i64_array, pwrite_i64_array, i64);
        forward_bulk_backend!(@a pread_u16_array, pwrite_u16_array, u16);
        forward_bulk_backend!(@a pread_u32_array, pwrite_u32_array, u32);
        forward_bulk_backend!(@a pread_u64_array, pwrite_u64_array, u64);
        forward_bulk_backend!(@a pread_f32_array, pwrite_f32_array, f32);
        forward_bulk_backend!(@a pread_f64_array, pwrite_f64_array, f64);
        forward_bulk_backend!(@a pread_i8_array, pwrite_i8_array, i8);
        forward_bulk_backend!(@a pread_i16_array, pwrite_i16_array, i16);
        forward_bulk_backend!(@a pread_i128_array, pwrite_i128_array, i128);
        forward_bulk_backend!(@a pread_u128_array, pwrite_u128_array, u128);
        forward_bulk_backend!(@r pwrite_byte_repeat, u8);
        forward_bulk_backend!(@r pwrite_i32_repeat, i32);
        forward_bulk_backend!(@r pwrite_i64_repeat, i64);
        forward_bulk_backend!(@r pwrite_u16_repeat, u16);
        forward_bulk_backend!(@r pwrite_u32_repeat, u32);
        forward_bulk_backend!(@r pwrite_u64_repeat, u64);
        forward_bulk_backend!(@r pwrite_f32_repeat, f32);
        forward_bulk_backend!(@r pwrite_f64_repeat, f64);
        forward_bulk_backend!(@r pwrite_i8_repeat, i8);
        forward_bulk_backend!(@r pwrite_i16_repeat, i16);
        forward_bulk_backend!(@r pwrite_i128_repeat, i128);
        forward_bulk_backend!(@r pwrite_u128_repeat, u128);
    };
    (@a $pr:ident, $pw:ident, $t:ty) => {
        fn $pr(&self, offset: u64, dst: &mut [$t]) -> Result<(), IoError> {
            by_backend!(self, x => x.$pr(offset, dst))
        }
        fn $pw(&mut self, offset: u64, src: &[$t]) -> Result<(), IoError> {
            by_backend!(mut self, x => x.$pw(offset, src))
        }
    };
    (@r $rep:ident, $t:ty) => {
        fn $rep(&mut self, offset: u64, value: $t, count: usize) -> Result<(), IoError> {
            by_backend!(mut self, x => x.$rep(offset, value, count))
        }
    };
}

impl AnyIO {
    /// Wraps a [`LocalIO`](crate::io::local::LocalIO) as a uniform handle (cursor at the start).
    pub fn local(io: LocalIO) -> AnyIO {
        AnyIO {
            backend: Backend::Local(io),
            position: 0,
        }
    }

    /// Wraps a [`Heap`](crate::io::memory::Heap) as a uniform handle (cursor at the start).
    pub fn memory(heap: Heap) -> AnyIO {
        AnyIO {
            backend: Backend::Memory(heap),
            position: 0,
        }
    }

    /// Whether the backing is a local-filesystem handle.
    pub fn is_local(&self) -> bool {
        matches!(self.backend, Backend::Local(_))
    }

    /// Whether the backing is an in-heap buffer.
    pub fn is_memory(&self) -> bool {
        matches!(self.backend, Backend::Memory(_))
    }

    /// The backing [`LocalIO`](crate::io::local::LocalIO), or `None` when it is in-heap.
    pub fn as_local(&self) -> Option<&LocalIO> {
        match &self.backend {
            Backend::Local(io) => Some(io),
            Backend::Memory(_) => None,
        }
    }

    /// The backing [`Heap`](crate::io::memory::Heap), or `None` when it is a local handle.
    pub fn as_memory(&self) -> Option<&Heap> {
        match &self.backend {
            Backend::Memory(heap) => Some(heap),
            Backend::Local(_) => None,
        }
    }

    /// Unwraps into the backing [`LocalIO`](crate::io::local::LocalIO), or `Err(self)` when it is
    /// in-heap (so the caller can recover the handle).
    #[allow(clippy::result_large_err)] // recover-self on mismatch, like `String::from_utf8`
    pub fn into_local(self) -> Result<LocalIO, AnyIO> {
        match self.backend {
            Backend::Local(io) => Ok(io),
            Backend::Memory(_) => Err(self),
        }
    }

    /// Unwraps into the backing [`Heap`](crate::io::memory::Heap), or `Err(self)` when it is a
    /// local handle.
    #[allow(clippy::result_large_err)] // recover-self on mismatch, like `String::from_utf8`
    pub fn into_memory(self) -> Result<Heap, AnyIO> {
        match self.backend {
            Backend::Memory(heap) => Ok(heap),
            Backend::Local(_) => Err(self),
        }
    }

    cursor_methods!();
}

impl IOBase for AnyIO {
    fn byte_size(&self) -> u64 {
        by_backend!(self, x => x.byte_size())
    }

    fn capacity(&self) -> u64 {
        by_backend!(self, x => x.capacity())
    }

    fn reserve(&mut self, additional: u64) {
        by_backend!(mut self, x => x.reserve(additional));
    }

    fn try_reserve(&mut self, additional: u64) -> Result<(), IoError> {
        by_backend!(mut self, x => x.try_reserve(additional))
    }

    fn shrink_to_fit(&mut self) {
        by_backend!(mut self, x => x.shrink_to_fit());
    }

    fn pread_byte_array(&self, offset: u64, buf: &mut [u8]) -> usize {
        by_backend!(self, x => x.pread_byte_array(offset, buf))
    }

    fn pwrite_byte_array(&mut self, offset: u64, data: &[u8]) -> usize {
        by_backend!(mut self, x => x.pwrite_byte_array(offset, data))
    }

    #[inline]
    fn as_bytes(&self) -> Option<&[u8]> {
        by_backend!(self, x => x.as_bytes())
    }

    // Forward every typed bulk array + repeat to the concrete backend, so `open()`'s uniform
    // handle reaches a `Heap`'s / mapped `LocalIO`'s fast contiguous overrides on all widths.
    forward_bulk_backend!();

    fn truncate(&mut self, len: u64) -> Result<(), IoError> {
        by_backend!(mut self, x => x.truncate(len))
    }

    fn close(&mut self) {
        by_backend!(mut self, x => x.close());
    }

    fn uri(&self) -> Uri {
        by_backend!(self, x => x.uri())
    }

    fn headers(&self) -> &Headers {
        by_backend!(self, x => x.headers())
    }

    fn headers_mut(&mut self) -> &mut Headers {
        by_backend!(mut self, x => x.headers_mut())
    }

    fn mode(&self) -> IOMode {
        by_backend!(self, x => x.mode())
    }

    fn kind(&self) -> IOKind {
        by_backend!(self, x => x.kind())
    }

    fn exists(&self) -> bool {
        by_backend!(self, x => x.exists())
    }

    fn rm(&self, exist_ok: bool) -> Result<(), IoError> {
        by_backend!(self, x => x.rm(exist_ok))
    }

    fn rmfile(&self, exist_ok: bool) -> Result<(), IoError> {
        by_backend!(self, x => x.rmfile(exist_ok))
    }

    fn rmdir(&self, exist_ok: bool) -> Result<(), IoError> {
        by_backend!(self, x => x.rmdir(exist_ok))
    }

    // A uniform wrapper is a discovery **leaf**: reach the concrete handle (`as_local`) to walk a
    // directory tree. Byte + navigation defaults ride the delegated primitives + `uri`.
    type Children = NoChildren<Self>;
    type Walk = NoChildren<Self>;

    fn ls(&self) -> Result<Self::Children, IoError> {
        Ok(std::iter::empty())
    }

    fn ls_recursive(&self) -> Result<Self::Walk, IoError> {
        Ok(std::iter::empty())
    }
}

/// **Opens** `uri` into a uniform [`AnyIO`] handle, dispatching on the scheme — the project's
/// `open()`:
///
/// - `file://…` or a **plain path** (no scheme) → a lazy [`LocalIO`](crate::io::local::LocalIO)
///   (reads before any write are ad hoc, the first write auto-creates + maps).
/// - `mem://…` → an in-heap [`Heap`](crate::io::memory::Heap) addressed by the URI.
///
/// Any other scheme is a guided [`IoError::FileIo`] naming the supported ones. The returned handle
/// reads/writes/seeks like an open file; wrap in an [`IOCursor`](crate::io::memory::IOCursor) or
/// [`IOSlice`](crate::io::memory::IOSlice) for an independent view.
///
/// ```
/// use yggdryl_core::io::memory::IOBase;
/// use yggdryl_core::io::open;
/// use yggdryl_core::uri::Uri;
///
/// let io = open(&Uri::from_file_path(&format!(
///     "{}/yggdryl_open_doc.bin",
///     std::env::temp_dir().to_string_lossy()
/// )))
/// .unwrap();
/// assert!(io.is_local());
/// ```
pub fn open(uri: &Uri) -> Result<AnyIO, IoError> {
    // Dispatch on the raw scheme: a bare path parses as `file` (or stays scheme-less when built
    // via `from_path`), and both route to the local family — hence `None | Some("file")`.
    match uri.scheme_opt() {
        None | Some("file") => Ok(AnyIO::local(LocalIO::from_uri(uri)?)),
        Some("mem") => Ok(AnyIO::memory(Heap::at_uri(uri.clone()))),
        Some(other) => Err(IoError::FileIo {
            op: "open",
            path: uri.to_string(),
            detail: format!(
                "cannot open the `{other}://` scheme; open a `file://` (or plain path) as a \
                 LocalIO or a `mem://` as a Heap"
            ),
        }),
    }
}

/// [`open`] from a path or URI **string** — a plain path or a `file://` / `mem://` URI, parsed
/// with [`Uri::parse_str`]. The string form of the project's `open()`.
///
/// # Errors
/// A [`crate::uri::UriError`]-derived [`IoError::FileIo`] when `target` is not a valid address, or
/// any [`open`] error.
pub fn open_str(target: &str) -> Result<AnyIO, IoError> {
    let uri = Uri::parse_str(target).map_err(|error| IoError::FileIo {
        op: "open",
        path: target.to_string(),
        detail: format!("not a valid path or URI: {error}"),
    })?;
    open(&uri)
}
