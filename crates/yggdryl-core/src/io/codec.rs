//! Typed value codecs over byte handles: the [`Codec<T>`] trait, its [`Stream`]
//! iterator, and the reference [`Frames`] length-delimited implementation.

use std::marker::PhantomData;

use crate::io::{Io, IoError};
#[allow(unused_imports)]
use crate::log_event;

/// The abstract **typed codec**: read and write values of `T` across the byte
/// primitives, in one of three shapes.
///
/// An implementor provides exactly two methods — [`read_opt`](Codec::read_opt),
/// which decodes one `T` (or `None` at a clean end of input), and
/// [`write`](Codec::write), which encodes one `T`. The rest is derived:
///
/// - single value — [`read`](Codec::read), which turns a clean end of input into
///   an [`IoError::UnexpectedEof`];
/// - many values — [`stream`](Codec::stream), an iterator that reads until the
///   source drains.
///
/// A codec composes with any [`Io`] handle: a `Codec<RecordBatch>` reads batches
/// straight out of a [`BytesIO`](crate::BytesIO), a [`LocalPath`](crate::LocalPath),
/// or a cloud path alike — they are all [`Io`], the one byte handle.
///
/// ```
/// use yggdryl_core::{BytesIO, Codec, Frames, Io, Whence};
///
/// let mut io = BytesIO::new();
/// Frames.write(&mut io, &b"payload".to_vec()).unwrap();
/// io.seek(0, Whence::Start).unwrap();
/// assert_eq!(Frames.read(&mut io).unwrap(), b"payload".to_vec());
/// ```
pub trait Codec<T> {
    /// Reads the next value, or `Ok(None)` when the source is cleanly drained at
    /// a value boundary. This is the one read primitive an implementor defines.
    fn read_opt(&self, reader: &mut dyn Io) -> Result<Option<T>, IoError>;

    /// Writes one value to the sink.
    fn write(&self, writer: &mut dyn Io, value: &T) -> Result<(), IoError>;

    /// Reads exactly one value, treating a clean end of input as an error.
    fn read(&self, reader: &mut dyn Io) -> Result<T, IoError> {
        self.read_opt(reader)?.ok_or(IoError::UnexpectedEof)
    }

    /// Returns an iterator that reads values from `reader` until it drains,
    /// yielding `Result<T, IoError>` for each.
    fn stream<R: Io>(&self, reader: R) -> Stream<'_, Self, R, T>
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

/// Iterator returned by [`Codec::stream`]: pulls one value per step from a
/// borrowed codec and an owned byte source, ending when the source is cleanly
/// drained.
pub struct Stream<'codec, C, R, T> {
    codec: &'codec C,
    reader: R,
    _marker: PhantomData<fn() -> T>,
}

impl<C, R, T> Iterator for Stream<'_, C, R, T>
where
    C: Codec<T>,
    R: Io,
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

/// The reference [`Codec`] implementation: **length-delimited byte frames**.
///
/// Each value is written as a big-endian `u32` byte length followed by that many
/// payload bytes, so frames pack back to back and a [`stream`](Codec::stream)
/// reads them out one at a time until the source drains.
///
/// ```
/// use yggdryl_core::{BytesIO, Codec, Frames};
///
/// let mut sink = BytesIO::new();
/// Frames.write(&mut sink, &b"hi".to_vec()).unwrap();
/// assert_eq!(sink.getvalue(), &[0, 0, 0, 2, b'h', b'i']);
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Frames;

impl Codec<Vec<u8>> for Frames {
    fn read_opt(&self, reader: &mut dyn Io) -> Result<Option<Vec<u8>>, IoError> {
        log_event!(trace, "Frames::read_opt");
        // Read the 4-byte length prefix. Zero bytes at the very start is a clean
        // end of the stream; a partial prefix is a truncated frame.
        let mut prefix = [0u8; 4];
        let mut filled = 0;
        while filled < prefix.len() {
            let count = reader.read(&mut prefix[filled..])?;
            if count == 0 {
                if filled == 0 {
                    log_event!(debug, "Frames::read_opt reached end of stream");
                    return Ok(None);
                }
                return Err(IoError::UnexpectedEof);
            }
            filled += count;
        }
        // Read the payload directly into the output, growing in bounded steps:
        // an honest frame takes a single allocation and a single copy, while a
        // malformed prefix (e.g. claiming 4 GiB with no body) fails fast having
        // reserved at most one step instead of gigabytes up front.
        const GROWTH_STEP: usize = 1 << 20;
        let len = u32::from_be_bytes(prefix) as usize;
        let mut payload = Vec::new();
        let mut filled = 0;
        while filled < len {
            let target = len.min(filled + GROWTH_STEP);
            payload.resize(target, 0);
            while filled < target {
                let count = reader.read(&mut payload[filled..target])?;
                if count == 0 {
                    return Err(IoError::UnexpectedEof);
                }
                filled += count;
            }
        }
        Ok(Some(payload))
    }

    fn write(&self, writer: &mut dyn Io, value: &Vec<u8>) -> Result<(), IoError> {
        log_event!(trace, "Frames::write {} bytes", value.len());
        let len = u32::try_from(value.len())
            .map_err(|_| IoError::Invalid(format!("frame of {} bytes exceeds u32", value.len())))?;
        writer.write_all(&len.to_be_bytes())?;
        writer.write_all(value)?;
        Ok(())
    }
}
