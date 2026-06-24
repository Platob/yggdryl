//! # yggdryl-io
//!
//! Dependency-free **IO foundations** for the **yggdryl** project: an abstract
//! contract for moving typed values across a byte boundary.
//!
//! - [`ReadBytes`] / [`WriteBytes`] are the byte primitives — a source and a
//!   sink, with `&[u8]` and `Vec<u8>` as the built-in in-memory ends.
//! - [`Io<T>`] is the typed contract layered on top: an implementor turns a `T`
//!   into bytes ([`write`](Io::write)) and back ([`read`](Io::read)), and a
//!   sequence of `T`s into a [`stream`](Io::stream) that reads until the source
//!   is drained.
//! - [`Frames`] is the reference [`Io`] implementation: length-delimited byte
//!   frames, enough to round-trip and stream values out of the box.
//! - [`BytesIO`] is a simple in-memory byte buffer with a cursor, modelled on
//!   Python's `io.BytesIO`; it is both a [`ReadBytes`] and a [`WriteBytes`], so
//!   it drives any [`Io`] codec.
//!
//! ```
//! use yggdryl_io::{Frames, Io};
//!
//! // Encode two frames into a byte sink, then stream them back out.
//! let mut sink: Vec<u8> = Vec::new();
//! Frames.write(&mut sink, &b"hello".to_vec()).unwrap();
//! Frames.write(&mut sink, &b"world".to_vec()).unwrap();
//!
//! let items: Vec<Vec<u8>> = Frames.stream(&sink[..]).collect::<Result<_, _>>().unwrap();
//! assert_eq!(items, vec![b"hello".to_vec(), b"world".to_vec()]);
//! ```

use std::fmt;
use std::marker::PhantomData;

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate is dependency-free by default and pays no runtime cost).
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}

/// Error returned by every [`ReadBytes`], [`WriteBytes`] and [`Io`] operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoError {
    /// The source ended in the middle of a value (a read needed more bytes than
    /// were left).
    UnexpectedEof,
    /// The sink accepted no bytes and could make no progress (it is full or
    /// closed).
    WriteZero,
    /// The bytes were structurally malformed for the value being read or written.
    Invalid(String),
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IoError::UnexpectedEof => write!(f, "unexpected end of input"),
            IoError::WriteZero => write!(f, "sink accepted no bytes"),
            IoError::Invalid(what) => write!(f, "malformed bytes: {what}"),
        }
    }
}

impl std::error::Error for IoError {}

/// A byte **source**: pull raw bytes out of something.
///
/// Implementors only provide [`read_bytes`](ReadBytes::read_bytes), which fills
/// as much of `buf` as it can and returns the count; a count of `0` means the
/// source is drained (clean end of input). The provided
/// [`read_exact`](ReadBytes::read_exact) and [`read_to_end`](ReadBytes::read_to_end)
/// build on it. `&[u8]` is the built-in in-memory source.
pub trait ReadBytes {
    /// Reads into `buf`, returning how many bytes were written to its front.
    /// Returns `Ok(0)` only when the source is drained.
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoError>;

    /// Fills `buf` completely, or fails with [`IoError::UnexpectedEof`] if the
    /// source drains first.
    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<(), IoError> {
        while !buf.is_empty() {
            let count = self.read_bytes(buf)?;
            if count == 0 {
                return Err(IoError::UnexpectedEof);
            }
            buf = &mut buf[count..];
        }
        Ok(())
    }

    /// Drains the source, appending every remaining byte to `out` and returning
    /// how many were read.
    fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize, IoError> {
        let mut chunk = [0u8; 4096];
        let mut total = 0;
        loop {
            let count = self.read_bytes(&mut chunk)?;
            if count == 0 {
                return Ok(total);
            }
            out.extend_from_slice(&chunk[..count]);
            total += count;
        }
    }
}

/// A byte **sink**: push raw bytes into something.
///
/// Implementors only provide [`write_bytes`](WriteBytes::write_bytes), which
/// accepts as much of `bytes` as it can and returns the count; the provided
/// [`write_all`](WriteBytes::write_all) loops until everything lands. `Vec<u8>`
/// is the built-in in-memory sink.
pub trait WriteBytes {
    /// Writes the front of `bytes`, returning how many were accepted. Returns
    /// `Ok(0)` only when the sink can make no progress.
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<usize, IoError>;

    /// Writes every byte of `bytes`, or fails with [`IoError::WriteZero`] if the
    /// sink stalls before they all land.
    fn write_all(&mut self, mut bytes: &[u8]) -> Result<(), IoError> {
        while !bytes.is_empty() {
            let count = self.write_bytes(bytes)?;
            if count == 0 {
                return Err(IoError::WriteZero);
            }
            bytes = &bytes[count..];
        }
        Ok(())
    }

    /// Flushes any buffered bytes to their destination. The default is a no-op,
    /// which suits unbuffered sinks like [`Vec<u8>`].
    fn flush(&mut self) -> Result<(), IoError> {
        Ok(())
    }
}

/// In-memory source: reading advances the slice past the bytes consumed, so a
/// `&[u8]` can be read to exhaustion.
impl ReadBytes for &[u8] {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        let count = buf.len().min(self.len());
        let (head, tail) = self.split_at(count);
        buf[..count].copy_from_slice(head);
        *self = tail;
        Ok(count)
    }
}

/// In-memory sink: writing appends to the vector, which never stalls.
impl WriteBytes for Vec<u8> {
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<usize, IoError> {
        self.extend_from_slice(bytes);
        Ok(bytes.len())
    }
}

/// Where a [`BytesIO::seek`] offset is measured from, mirroring the `whence`
/// values of Python's `io` module (`SEEK_SET` / `SEEK_CUR` / `SEEK_END`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Whence {
    /// From the start of the buffer (`0`).
    #[default]
    Start,
    /// From the current cursor position (`1`).
    Current,
    /// From the end of the buffer (`2`).
    End,
}

/// A simple in-memory byte buffer with a cursor, modelled on Python's
/// `io.BytesIO`: it is both a [`ReadBytes`] source and a [`WriteBytes`] sink, so
/// it plugs straight into any [`Io`] codec.
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
/// for random access. The lower-level [`ReadBytes`] / [`WriteBytes`] primitives
/// always advance, so codecs work whatever the flag.
///
/// ```
/// use yggdryl_io::{BytesIO, Whence};
///
/// let mut io = BytesIO::from_bytes(b"hello world".to_vec());
/// assert_eq!(io.read(Some(5)), b"hello");
/// assert_eq!(io.tell(), 5);
/// io.seek(6, Whence::Start).unwrap();
/// assert_eq!(io.read(None), b"world");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BytesIO {
    buffer: Vec<u8>,
    position: usize,
    stream: bool,
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

    /// Wraps existing `bytes`, with the cursor at the start and streaming on.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> BytesIO {
        BytesIO {
            buffer: bytes.into(),
            position: 0,
            stream: true,
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
        self.take(end, self.stream)
    }

    /// Reads from the cursor through the next `\n` (inclusive), or to the end of
    /// the buffer. Advances the cursor when [`stream`](BytesIO::stream).
    pub fn read_line(&mut self) -> Vec<u8> {
        let start = self.position.min(self.buffer.len());
        let end = self.buffer[start..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(self.buffer.len(), |offset| start + offset + 1);
        self.take(end, self.stream)
    }

    /// Writes `bytes` at the cursor, overwriting any overlap and extending (zero-
    /// filling any gap) as needed. Returns the count written and advances the
    /// cursor when [`stream`](BytesIO::stream).
    pub fn write(&mut self, bytes: &[u8]) -> usize {
        log_event!(
            trace,
            "BytesIO::write {} bytes at {}",
            bytes.len(),
            self.position
        );
        self.put(bytes, self.stream)
    }

    /// Reads `[cursor..end]` (clamped to the buffer) as an owned vector, advancing
    /// the cursor by the count actually read when `advance` (so a cursor seeked
    /// past the end stays put, as in Python). Shared by the read helpers.
    fn take(&mut self, end: usize, advance: bool) -> Vec<u8> {
        let start = self.position.min(self.buffer.len());
        let end = end.clamp(start, self.buffer.len());
        if advance {
            self.position += end - start;
        }
        self.buffer[start..end].to_vec()
    }

    /// Writes `bytes` at the cursor, zero-filling any gap and extending as needed,
    /// moving the cursor past them when `advance`. Shared by [`write`](BytesIO::write)
    /// and the [`WriteBytes`] primitive.
    fn put(&mut self, bytes: &[u8], advance: bool) -> usize {
        let start = self.position;
        let end = start + bytes.len();
        if self.buffer.len() < end {
            self.buffer.resize(end, 0);
        }
        self.buffer[start..end].copy_from_slice(bytes);
        if advance {
            self.position = end;
        }
        bytes.len()
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
        let target = base + offset;
        if target < 0 {
            return Err(IoError::Invalid(format!(
                "seek to {target} is before the start"
            )));
        }
        self.position = target as usize;
        Ok(self.position)
    }

    /// Truncates the buffer to `size` bytes (the current cursor when `None`),
    /// returning the new length. The cursor is left where it is, as in Python.
    pub fn truncate(&mut self, size: Option<usize>) -> usize {
        let size = size.unwrap_or(self.position);
        log_event!(debug, "BytesIO::truncate to {size}");
        self.buffer.truncate(size);
        self.buffer.len()
    }

    /// Empties the buffer and resets the cursor to `0`.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.position = 0;
    }

    /// No-op flush, present for parity with Python's `io` API.
    pub fn flush(&mut self) {}
}

/// In-memory source: reads from the cursor and advances it, so a `BytesIO` drains
/// like any other [`ReadBytes`] when driving an [`Io`] codec.
impl ReadBytes for BytesIO {
    fn read_bytes(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
        let start = self.position.min(self.buffer.len());
        let count = buf.len().min(self.buffer.len() - start);
        buf[..count].copy_from_slice(&self.buffer[start..start + count]);
        self.position += count;
        Ok(count)
    }
}

/// In-memory sink: writes at the cursor and advances it, never stalling.
impl WriteBytes for BytesIO {
    fn write_bytes(&mut self, bytes: &[u8]) -> Result<usize, IoError> {
        Ok(self.put(bytes, true))
    }
}

/// The abstract **typed IO contract**: read and write values of `T` across the
/// byte primitives, in one of three shapes.
///
/// An implementor provides exactly two methods — [`read_opt`](Io::read_opt),
/// which decodes one `T` (or `None` at a clean end of input), and
/// [`write`](Io::write), which encodes one `T`. The rest is derived:
///
/// - single value — [`read`](Io::read), which turns a clean end of input into an
///   [`IoError::UnexpectedEof`];
/// - many values — [`stream`](Io::stream), an iterator that reads until the
///   source drains.
///
/// In-memory round-trips need no extra methods: `&[u8]` is a [`ReadBytes`] and
/// `Vec<u8>` is a [`WriteBytes`], so a slice reads and a vector writes directly.
///
/// ```
/// use yggdryl_io::{Frames, Io};
///
/// let mut bytes: Vec<u8> = Vec::new();
/// Frames.write(&mut bytes, &b"payload".to_vec()).unwrap();
/// assert_eq!(Frames.read(&mut &bytes[..]).unwrap(), b"payload".to_vec());
/// ```
pub trait Io<T> {
    /// Reads the next value, or `Ok(None)` when the source is cleanly drained at
    /// a value boundary. This is the one read primitive an implementor defines.
    fn read_opt(&self, reader: &mut impl ReadBytes) -> Result<Option<T>, IoError>;

    /// Writes one value to the sink.
    fn write(&self, writer: &mut impl WriteBytes, value: &T) -> Result<(), IoError>;

    /// Reads exactly one value, treating a clean end of input as an error.
    fn read(&self, reader: &mut impl ReadBytes) -> Result<T, IoError> {
        self.read_opt(reader)?.ok_or(IoError::UnexpectedEof)
    }

    /// Returns an iterator that reads values from `reader` until it drains,
    /// yielding `Result<T, IoError>` for each.
    fn stream<R: ReadBytes>(&self, reader: R) -> Stream<'_, Self, R, T>
    where
        Self: Sized,
    {
        Stream {
            codec: self,
            reader,
            _marker: PhantomData,
        }
    }
}

/// Iterator returned by [`Io::stream`]: pulls one value per step from a borrowed
/// codec and an owned byte source, ending when the source is cleanly drained.
pub struct Stream<'io, C, R, T> {
    codec: &'io C,
    reader: R,
    _marker: PhantomData<fn() -> T>,
}

impl<C, R, T> Iterator for Stream<'_, C, R, T>
where
    C: Io<T>,
    R: ReadBytes,
{
    type Item = Result<T, IoError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.codec.read_opt(&mut self.reader) {
            Ok(Some(value)) => Some(Ok(value)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        }
    }
}

/// The reference [`Io`] implementation: **length-delimited byte frames**.
///
/// Each value is written as a big-endian `u32` byte length followed by that many
/// payload bytes, so frames pack back to back and a [`stream`](Io::stream) reads
/// them out one at a time until the source drains.
///
/// ```
/// use yggdryl_io::{Frames, Io};
///
/// let mut sink: Vec<u8> = Vec::new();
/// Frames.write(&mut sink, &b"hi".to_vec()).unwrap();
/// assert_eq!(sink, vec![0, 0, 0, 2, b'h', b'i']);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Frames;

impl Io<Vec<u8>> for Frames {
    fn read_opt(&self, reader: &mut impl ReadBytes) -> Result<Option<Vec<u8>>, IoError> {
        log_event!(trace, "Frames::read_opt");
        // Read the 4-byte length prefix. Zero bytes at the very start is a clean
        // end of the stream; a partial prefix is a truncated frame.
        let mut prefix = [0u8; 4];
        let mut filled = 0;
        while filled < prefix.len() {
            let count = reader.read_bytes(&mut prefix[filled..])?;
            if count == 0 {
                if filled == 0 {
                    log_event!(debug, "Frames::read_opt reached end of stream");
                    return Ok(None);
                }
                return Err(IoError::UnexpectedEof);
            }
            filled += count;
        }
        let len = u32::from_be_bytes(prefix) as usize;
        let mut payload = vec![0u8; len];
        reader.read_exact(&mut payload)?;
        Ok(Some(payload))
    }

    fn write(&self, writer: &mut impl WriteBytes, value: &Vec<u8>) -> Result<(), IoError> {
        log_event!(trace, "Frames::write {} bytes", value.len());
        let len = u32::try_from(value.len())
            .map_err(|_| IoError::Invalid(format!("frame of {} bytes exceeds u32", value.len())))?;
        writer.write_all(&len.to_be_bytes())?;
        writer.write_all(value)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytesio_reads_and_advances_the_cursor() {
        let mut io = BytesIO::from_bytes(b"hello world".to_vec());
        assert_eq!(io.read(Some(5)), b"hello");
        assert_eq!(io.tell(), 5);
        assert_eq!(io.remaining(), 6);
        assert_eq!(io.read(Some(1)), b" ");
        assert_eq!(io.read(None), b"world");
        // Reading at the end yields nothing and the cursor stays put.
        assert_eq!(io.read(None), b"");
        assert_eq!(io.tell(), 11);
    }

    #[test]
    fn bytesio_without_stream_keeps_the_cursor_fixed() {
        let mut io = BytesIO::from_bytes(b"abcdef".to_vec());
        io.set_stream(false);
        // Each read starts from the same fixed cursor.
        assert_eq!(io.read(Some(3)), b"abc");
        assert_eq!(io.read(Some(3)), b"abc");
        assert_eq!(io.tell(), 0);
        // The cursor only moves on an explicit seek.
        io.seek(3, Whence::Start).unwrap();
        assert_eq!(io.read(Some(3)), b"def");
    }

    #[test]
    fn bytesio_seek_whences_and_errors() {
        let mut io = BytesIO::from_bytes(b"0123456789".to_vec());
        assert_eq!(io.seek(4, Whence::Start).unwrap(), 4);
        assert_eq!(io.seek(2, Whence::Current).unwrap(), 6);
        assert_eq!(io.seek(-1, Whence::End).unwrap(), 9);
        assert_eq!(io.read(None), b"9");
        // Seeking before the start fails; seeking past the end is allowed.
        assert!(matches!(
            io.seek(-1, Whence::Start),
            Err(IoError::Invalid(_))
        ));
        assert_eq!(io.seek(3, Whence::End).unwrap(), 13);
        assert_eq!(io.read(None), b"");
        // A read past the end leaves the cursor where it was (as in Python).
        assert_eq!(io.tell(), 13);
    }

    #[test]
    fn bytesio_write_overwrites_and_zero_fills() {
        let mut io = BytesIO::from_bytes(b"abc".to_vec());
        io.seek(1, Whence::Start).unwrap();
        assert_eq!(io.write(b"XY"), 2);
        assert_eq!(io.getvalue(), b"aXY");
        // A write past the end zero-fills the gap.
        io.seek(5, Whence::Start).unwrap();
        io.write(b"Z");
        assert_eq!(io.getvalue(), b"aXY\0\0Z");
    }

    #[test]
    fn bytesio_read_line_walks_lines() {
        let mut io = BytesIO::from_bytes(b"one\ntwo\nthree".to_vec());
        assert_eq!(io.read_line(), b"one\n");
        assert_eq!(io.read_line(), b"two\n");
        // The final line has no trailing newline.
        assert_eq!(io.read_line(), b"three");
        assert_eq!(io.read_line(), b"");
    }

    #[test]
    fn bytesio_truncate_and_clear() {
        let mut io = BytesIO::from_bytes(b"abcdef".to_vec());
        io.seek(3, Whence::Start).unwrap();
        // truncate() defaults to the cursor.
        assert_eq!(io.truncate(None), 3);
        assert_eq!(io.getvalue(), b"abc");
        io.clear();
        assert!(io.is_empty());
        assert_eq!(io.tell(), 0);
    }

    #[test]
    fn bytesio_drives_a_frames_codec() {
        // As a ReadBytes/WriteBytes pair it round-trips through any Io codec.
        let mut io = BytesIO::new();
        Frames.write(&mut io, &b"one".to_vec()).unwrap();
        Frames.write(&mut io, &b"two".to_vec()).unwrap();
        io.seek(0, Whence::Start).unwrap();
        let items: Vec<Vec<u8>> = Frames.stream(io).collect::<Result<_, _>>().unwrap();
        assert_eq!(items, vec![b"one".to_vec(), b"two".to_vec()]);
    }

    #[test]
    fn read_exact_and_to_end_drain_a_slice() {
        let data = [1u8, 2, 3, 4, 5];
        let mut reader: &[u8] = &data;
        let mut head = [0u8; 2];
        reader.read_exact(&mut head).unwrap();
        assert_eq!(head, [1, 2]);
        let mut rest = Vec::new();
        assert_eq!(reader.read_to_end(&mut rest).unwrap(), 3);
        assert_eq!(rest, vec![3, 4, 5]);
        // The source is drained: a further read yields zero, exact reads error.
        assert_eq!(reader.read_bytes(&mut head).unwrap(), 0);
        assert_eq!(reader.read_exact(&mut head), Err(IoError::UnexpectedEof));
    }

    #[test]
    fn write_all_appends_to_a_vec() {
        let mut sink: Vec<u8> = Vec::new();
        sink.write_all(b"ab").unwrap();
        sink.write_all(b"cd").unwrap();
        sink.flush().unwrap();
        assert_eq!(sink, b"abcd");
    }

    #[test]
    fn frames_round_trip_one_value() {
        let value = b"payload".to_vec();
        let mut bytes: Vec<u8> = Vec::new();
        Frames.write(&mut bytes, &value).unwrap();
        assert_eq!(bytes, [&[0, 0, 0, 7][..], b"payload"].concat());
        assert_eq!(Frames.read(&mut &bytes[..]).unwrap(), value);
    }

    #[test]
    fn frames_round_trip_empty_value() {
        let mut bytes: Vec<u8> = Vec::new();
        Frames.write(&mut bytes, &Vec::new()).unwrap();
        assert_eq!(bytes, vec![0, 0, 0, 0]);
        assert_eq!(Frames.read(&mut &bytes[..]).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn stream_yields_every_frame_then_ends() {
        let mut sink: Vec<u8> = Vec::new();
        for value in [&b"one"[..], b"", b"three"] {
            Frames.write(&mut sink, &value.to_vec()).unwrap();
        }
        let items: Vec<Vec<u8>> = Frames.stream(&sink[..]).collect::<Result<_, _>>().unwrap();
        assert_eq!(items, vec![b"one".to_vec(), Vec::new(), b"three".to_vec()]);
    }

    #[test]
    fn read_on_empty_source_is_eof_but_stream_is_just_empty() {
        let empty: &[u8] = &[];
        // A single read past a clean boundary is an error...
        assert_eq!(Frames.read(&mut &empty[..]), Err(IoError::UnexpectedEof));
        // ...but streaming a drained source simply yields nothing.
        let mut stream = Frames.stream(empty);
        assert!(stream.next().is_none());
    }

    #[test]
    fn truncated_frame_is_unexpected_eof() {
        // A length prefix promising 5 bytes with only 2 present is truncated.
        let bytes = [0u8, 0, 0, 5, b'h', b'i'];
        assert_eq!(Frames.read(&mut &bytes[..]), Err(IoError::UnexpectedEof));
        // A partial length prefix is likewise truncated, not a clean end.
        assert_eq!(Frames.read(&mut &[0u8, 0][..]), Err(IoError::UnexpectedEof));
    }

    #[test]
    fn io_error_displays() {
        assert_eq!(
            IoError::UnexpectedEof.to_string(),
            "unexpected end of input"
        );
        assert_eq!(IoError::WriteZero.to_string(), "sink accepted no bytes");
        assert_eq!(
            IoError::Invalid("too big".to_string()).to_string(),
            "malformed bytes: too big"
        );
    }
}
