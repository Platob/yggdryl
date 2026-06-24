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
