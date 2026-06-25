//! The in-memory [`BytesIO`] byte buffer.

use crate::io::{read_cursor, read_line_cursor, resolve, Io, IoError, IoStats, Mode, Whence};
#[allow(unused_imports)]
use crate::log_event;
use crate::Url;

/// A simple in-memory byte buffer with a cursor, modelled on Python's
/// `io.BytesIO`: a read/write [`Io`] handle, so it plugs straight into any
/// [`Codec`](crate::Codec) and exposes its bytes for zero-copy [`copy`](crate::copy).
///
/// A `BytesIO` owns a [`Vec<u8>`] and a `position` cursor; [`seek`](BytesIO::seek)
/// / [`tell`](BytesIO::tell) move and read that cursor, [`getvalue`](BytesIO::getvalue)
/// borrows the whole buffer, and writes past the end zero-fill the gap (as in
/// Python).
///
/// The [`stream`](BytesIO::stream) flag governs the **Python-style** helpers
/// [`read`](BytesIO::read) / [`read_line`](BytesIO::read_line) /
/// [`write`](BytesIO::write): when `true` (the default) they advance the cursor,
/// replicating Python's stateful streaming; when `false` the cursor stays put
/// for random access. The lower-level [`Io::read`] / [`Io::write`] primitives
/// always advance, so codecs work whatever the flag.
///
/// ```
/// use yggdryl_core::{BytesIO, Whence};
///
/// let mut io = BytesIO::from_bytes(b"hello world".to_vec());
/// assert_eq!(io.read(Some(5)), b"hello");
/// assert_eq!(io.tell(), 5);
/// io.seek(6, Whence::Start).unwrap();
/// assert_eq!(io.read(None), b"world");
/// ```
#[derive(Debug)]
pub struct BytesIO {
    buffer: Vec<u8>,
    position: usize,
    stream: bool,
    mode: Mode,
    parent: Option<Box<dyn Io>>,
}

impl Default for BytesIO {
    fn default() -> BytesIO {
        BytesIO::new()
    }
}

impl BytesIO {
    /// Creates an empty buffer with the cursor at `0` and streaming on.
    pub fn new() -> BytesIO {
        BytesIO::from_bytes(Vec::new())
    }

    /// Creates an empty buffer that can hold `capacity` bytes before
    /// reallocating — the preallocating constructor for write-heavy use.
    pub fn with_capacity(capacity: usize) -> BytesIO {
        BytesIO::from_bytes(Vec::with_capacity(capacity))
    }

    /// Wraps existing `bytes`, with the cursor at the start, streaming on, mode
    /// [`Read`](Mode::Read) and no parent.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> BytesIO {
        BytesIO {
            buffer: bytes.into(),
            position: 0,
            stream: true,
            mode: Mode::Read,
            parent: None,
        }
    }

    /// Opens a new in-memory handle derived from this one, recording `self` as
    /// the child's [`parent`](Io::parent) and applying `mode` and `stream`.
    ///
    /// The child's initial state follows `mode`, as in Python `open`:
    /// [`Write`](Mode::Write) starts empty (truncated), [`Append`](Mode::Append)
    /// copies the bytes with the cursor at the end, and [`Read`](Mode::Read) /
    /// [`ReadWrite`](Mode::ReadWrite) copy the bytes with the cursor at the start.
    pub fn open(self, mode: Mode, stream: bool) -> BytesIO {
        log_event!(debug, "BytesIO::open mode={mode} stream={stream}");
        let bytes = self.buffer.clone();
        BytesIO::derived(bytes, mode, stream, Box::new(self))
    }

    /// Builds a derived in-memory handle over `bytes`, applying `mode`
    /// ([`Write`](Mode::Write) truncates, [`Append`](Mode::Append) seeks to the
    /// end, otherwise the cursor starts at `0`) and `stream`, with `parent`
    /// recorded. Shared by [`open`](BytesIO::open) and [`LocalPath`](crate::LocalPath)'s
    /// [`Io::open`].
    pub(crate) fn derived(
        bytes: Vec<u8>,
        mode: Mode,
        stream: bool,
        parent: Box<dyn Io>,
    ) -> BytesIO {
        let buffer = if mode == Mode::Write {
            Vec::new()
        } else {
            bytes
        };
        let position = if mode == Mode::Append {
            buffer.len()
        } else {
            0
        };
        BytesIO {
            buffer,
            position,
            stream,
            mode,
            parent: Some(parent),
        }
    }

    /// Whether the Python-style [`read`](BytesIO::read) / [`read_line`](BytesIO::read_line)
    /// / [`write`](BytesIO::write) helpers advance the cursor.
    pub fn stream(&self) -> bool {
        self.stream
    }

    /// Sets the [`stream`](BytesIO::stream) flag.
    pub fn set_stream(&mut self, stream: bool) {
        log_event!(debug, "BytesIO::set_stream {stream}");
        self.stream = stream;
    }

    /// The current cursor position.
    pub fn tell(&self) -> usize {
        self.position
    }

    /// The total number of bytes held, regardless of the cursor.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Whether the buffer holds no bytes.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// The number of bytes between the cursor and the end of the buffer.
    pub fn remaining(&self) -> usize {
        self.buffer.len().saturating_sub(self.position)
    }

    /// Borrows the whole buffer, ignoring the cursor (the inverse of
    /// [`from_bytes`](BytesIO::from_bytes)).
    pub fn getvalue(&self) -> &[u8] {
        &self.buffer
    }

    /// Reads up to `size` bytes from the cursor, or all remaining bytes when
    /// `size` is `None`. Advances the cursor when [`stream`](BytesIO::stream).
    pub fn read(&mut self, size: Option<usize>) -> Vec<u8> {
        log_event!(trace, "BytesIO::read {size:?} at {}", self.position);
        let end = match size {
            Some(n) => self.position.saturating_add(n),
            None => self.buffer.len(),
        };
        read_cursor(&self.buffer, &mut self.position, end, self.stream)
    }

    /// Reads from the cursor through the next `\n` (inclusive), or to the end of
    /// the buffer. Advances the cursor when [`stream`](BytesIO::stream).
    pub fn read_line(&mut self) -> Vec<u8> {
        read_line_cursor(&self.buffer, &mut self.position, self.stream)
    }

    /// Writes `bytes` at the cursor, overwriting any overlap and extending (zero-
    /// filling any gap) as needed. Returns the count written and advances the
    /// cursor when [`stream`](BytesIO::stream).
    ///
    /// Errors with [`IoError::Invalid`] when the write would extend the buffer past
    /// the addressable range (e.g. after seeking near [`i64::MAX`] past the end),
    /// rather than attempting a multi-exabyte allocation.
    pub fn write(&mut self, bytes: &[u8]) -> Result<usize, IoError> {
        log_event!(
            trace,
            "BytesIO::write {} bytes at {}",
            bytes.len(),
            self.position
        );
        self.put(bytes, self.stream)
    }

    /// Moves the cursor to `offset` relative to `whence`, returning the new
    /// position. Seeking past the end is allowed (a later write zero-fills the
    /// gap); seeking before the start fails with [`IoError::Invalid`].
    pub fn seek(&mut self, offset: i64, whence: Whence) -> Result<usize, IoError> {
        log_event!(trace, "BytesIO::seek {offset} from {whence:?}");
        let base = match whence {
            Whence::Start => 0,
            Whence::Current => self.position as i64,
            Whence::End => self.buffer.len() as i64,
        };
        let target = base
            .checked_add(offset)
            .ok_or_else(|| IoError::Invalid(format!("seek offset {offset} overflows")))?;
        if target < 0 {
            return Err(IoError::Invalid(format!(
                "seek to {target} is before the start"
            )));
        }
        self.position = target as usize;
        Ok(self.position)
    }

    /// Resizes the buffer to `size` bytes (the current cursor when `None`),
    /// returning the new length. Shrinks (drops the tail) or grows (zero-fills),
    /// leaving the cursor where it is, as in Python. Errors (rather than aborting on
    /// a giant allocation) when growing past the addressable range.
    pub fn truncate(&mut self, size: Option<usize>) -> Result<usize, IoError> {
        let size = size.unwrap_or(self.position);
        log_event!(debug, "BytesIO::truncate to {size}");
        self.resize(size)?;
        Ok(self.buffer.len())
    }

    /// Resizes the backing buffer to exactly `size`, growing (zero-fill) or
    /// shrinking. Shared by [`truncate`](BytesIO::truncate) and [`Io::truncate`].
    /// Caps a grow at [`isize::MAX`] (the `Vec` allocation limit), returning
    /// [`IoError::Invalid`] rather than letting `Vec::resize` abort the process.
    fn resize(&mut self, size: usize) -> Result<(), IoError> {
        if size > self.buffer.len() {
            if size > isize::MAX as usize {
                return Err(IoError::Invalid(format!(
                    "resize to {size} bytes exceeds the addressable buffer range"
                )));
            }
            self.buffer.resize(size, 0);
        } else {
            self.buffer.truncate(size);
        }
        Ok(())
    }

    /// Empties the buffer and resets the cursor to `0`.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.position = 0;
    }

    /// No-op flush, present for parity with Python's `io` API.
    pub fn flush(&mut self) {}

    /// Writes `bytes` at the cursor, zero-filling any gap and extending as needed,
    /// moving the cursor past them when `advance`. Shared by [`write`](BytesIO::write),
    /// [`Io::write`] and [`Io::pwrite`].
    ///
    /// Guards the write extent: the cursor may be seeked arbitrarily far past the
    /// end (a legal Python operation), so a write there would otherwise resize the
    /// buffer to an astronomical length and abort the process. The extent is capped
    /// at [`isize::MAX`] (the `Vec` allocation limit), returning [`IoError::Invalid`].
    ///
    /// Growth is **amortized** and never redundantly zero-filled: the bytes that
    /// extend past the end are appended with [`Vec::extend_from_slice`] (which grows
    /// the capacity geometrically, like `push`), so a sequence of appends is `O(n)`
    /// overall and the freshly-grown region is written once — not memset to zero and
    /// then overwritten. Only a genuine gap left by seeking past the end is
    /// zero-filled. An empty write is a no-op (it never grows the buffer, matching
    /// Python's `BytesIO`).
    fn put(&mut self, bytes: &[u8], advance: bool) -> Result<usize, IoError> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let start = self.position;
        let end = start
            .checked_add(bytes.len())
            .filter(|&end| end <= isize::MAX as usize)
            .ok_or_else(|| {
                IoError::Invalid(format!(
                    "write of {} bytes at offset {start} exceeds the addressable buffer range",
                    bytes.len()
                ))
            })?;
        let len = self.buffer.len();
        if start <= len {
            // Overwrite the overlap with existing bytes in place, then append the
            // remainder — growing geometrically with no zero-fill of the new region.
            let overlap = len.min(end) - start;
            self.buffer[start..start + overlap].copy_from_slice(&bytes[..overlap]);
            self.buffer.extend_from_slice(&bytes[overlap..]);
        } else {
            // The cursor sits past the end: zero-fill only the gap `[len, start)`,
            // then append the bytes (never zero-filling the region we then write).
            self.buffer.resize(start, 0);
            self.buffer.extend_from_slice(bytes);
        }
        if advance {
            self.position = end;
        }
        Ok(bytes.len())
    }
}

impl Io for BytesIO {
    /// `mem://<buffer-address>` — the `mem` scheme with the hex address of the
    /// backing bytes, so every in-memory handle still has a stable-shape URL.
    fn url(&self) -> Url {
        Url::new("mem", format!("{:x}", self.buffer.as_ptr() as usize))
    }

    fn stats(&self) -> Result<IoStats, IoError> {
        Ok(IoStats::new(self.buffer.len() as u64))
    }

    /// Delegates to the Python-style [`seek`](BytesIO::seek), widening to `u64`.
    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        BytesIO::seek(self, offset, whence).map(|position| position as u64)
    }

    fn stream_position(&self) -> u64 {
        self.position as u64
    }

    fn stream_len(&self) -> Option<u64> {
        Some(self.buffer.len() as u64)
    }

    fn mode(&self) -> Mode {
        self.mode
    }

    fn stream(&self) -> bool {
        self.stream
    }

    fn set_stream(&mut self, stream: bool) {
        self.stream = stream;
    }

    fn parent(&self) -> Option<&dyn Io> {
        self.parent.as_deref()
    }

    /// Opens a derived in-memory handle (see [`BytesIO::open`]).
    fn open(self: Box<Self>, mode: Mode, stream: bool) -> Result<Box<dyn Io>, IoError> {
        Ok(Box::new((*self).open(mode, stream)))
    }

    fn as_slice(&self) -> Option<&[u8]> {
        Some(&self.buffer)
    }

    /// Writes `bytes` at the cursor, advancing it — the in-memory streamed write.
    fn write(&mut self, bytes: &[u8]) -> Result<usize, IoError> {
        self.put(bytes, true)
    }

    /// Positional write into the buffer, overwriting and zero-filling as needed.
    /// [`Whence::Current`] advances the cursor; otherwise it is left put.
    fn pwrite(&mut self, bytes: &[u8], offset: i64, whence: Whence) -> Result<usize, IoError> {
        let resolved = resolve(
            self.position as u64,
            Some(self.buffer.len() as u64),
            offset,
            whence,
        )?;
        // Narrow the u64 position to an index; on a 32-bit target an offset beyond
        // the address space is an error, not a silent wraparound.
        let start = usize::try_from(resolved).map_err(|_| {
            IoError::Invalid(format!("offset {resolved} exceeds the addressable range"))
        })?;
        let saved = self.position;
        self.position = start;
        let count = self.put(bytes, true);
        if !matches!(whence, Whence::Current) {
            self.position = saved;
        }
        count
    }

    /// The reserved capacity of the backing [`Vec<u8>`].
    fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    /// Reserves room for `additional` more bytes in the backing buffer. Errors
    /// (rather than aborting) when the requested capacity exceeds the addressable
    /// range.
    fn reserve_capacity(&mut self, additional: usize) -> Result<(), IoError> {
        if self
            .buffer
            .len()
            .checked_add(additional)
            .is_none_or(|needed| needed > isize::MAX as usize)
        {
            return Err(IoError::Invalid(format!(
                "reserve of {additional} bytes exceeds the addressable buffer range"
            )));
        }
        self.buffer.reserve(additional);
        Ok(())
    }

    /// Resizes the buffer to `size` bytes (grow zero-fills, shrink drops the
    /// tail); the cursor is left where it is. Errors when growing past the
    /// addressable range rather than aborting on the allocation.
    fn truncate(&mut self, size: u64) -> Result<(), IoError> {
        let size = usize::try_from(size).map_err(|_| {
            IoError::Invalid(format!("truncate to {size} exceeds the addressable range"))
        })?;
        self.resize(size)
    }
}
