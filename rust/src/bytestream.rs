//! Lazy byte chunks over positional and cursor-based handles.

use std::io::Read;

use super::{IOBase, IOCursor};
use crate::{Error, Result};

/// A lazy, bounded stream of byte arrays.
///
/// [`IOBase::pstream_bytes`] builds this over an explicit position and
/// [`IOCursor::stream_bytes`] builds it over a cursor's current position. The
/// source is not touched until the first item (or [`Read::read`]) is requested.
/// Iterator items are full `batch_size` chunks except for the final short one,
/// and a read failure is yielded once before the iterator stays fused.
///
/// The type also implements [`Read`], so a codec or parser can fill its own
/// reusable buffers directly without first allocating an iterator item.
pub struct ByteStream<'source> {
    source: Box<dyn ByteSource + 'source>,
    batch_size: usize,
    pending_error: Option<Error>,
    done: bool,
}

impl<'source> ByteStream<'source> {
    /// Stream a standard reader in bounded chunks.
    ///
    /// This constructor lets an [`IOBase`] implementation expose a native
    /// sequential reader from its [`IOBase::pstream_bytes`] override.
    /// Construction performs no read.
    ///
    /// # Errors
    ///
    /// Returns [`std::io::ErrorKind::InvalidInput`] when `batch_size` is zero.
    pub fn from_reader(reader: impl Read + 'source, batch_size: usize) -> Result<Self> {
        Self::from_source(ReaderSource(reader), batch_size)
    }

    /// Stream one filesystem reader, retaining exactly that open handle.
    pub fn from_fs_reader(
        reader: Box<dyn crate::holder::fs::ByteReader + 'source>,
        batch_size: usize,
    ) -> Result<Self> {
        Self::from_source(FileSystemSource { reader }, batch_size)
    }

    /// Stream one random-access filesystem reader after it has been positioned.
    pub fn from_fs_random_reader(
        reader: Box<dyn crate::holder::fs::RandomAccessReader + 'source>,
        batch_size: usize,
    ) -> Result<Self> {
        Self::from_source(RandomFileSystemSource { reader }, batch_size)
    }

    pub(super) fn from_handle<H: IOBase + ?Sized>(
        handle: &'source H,
        position: u64,
        batch_size: usize,
    ) -> Result<Self> {
        Self::from_source(PositionalSource { handle, position }, batch_size)
    }

    pub(super) fn from_cursor<C: IOCursor + ?Sized>(
        cursor: &'source mut C,
        batch_size: usize,
    ) -> Result<Self> {
        Self::from_source(CursorSource(cursor), batch_size)
    }

    /// Keep one native positional stream alive while advancing its cursor.
    pub(super) fn from_advancing_stream(
        stream: ByteStream<'source>,
        position: &'source mut u64,
        batch_size: usize,
    ) -> Result<Self> {
        Self::from_source(AdvancingSource { stream, position }, batch_size)
    }

    fn from_source(source: impl ByteSource + 'source, batch_size: usize) -> Result<Self> {
        if batch_size == 0 {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "byte stream batch_size must be greater than zero",
            )));
        }
        Ok(Self {
            source: Box::new(source),
            batch_size,
            pending_error: None,
            done: false,
        })
    }

    /// Read up to `target.len()` bytes, filling it unless the stream ends.
    fn read_filled(&mut self, target: &mut [u8]) -> Result<usize> {
        if target.is_empty() || self.done {
            return Ok(0);
        }
        if let Some(error) = self.pending_error.take() {
            self.done = true;
            return Err(error);
        }

        let mut filled = 0;
        while filled < target.len() {
            match self.source.read_bytes(&mut target[filled..]) {
                Ok(0) => {
                    self.done = true;
                    break;
                }
                Ok(read) => filled += read,
                Err(error) if filled == 0 => {
                    self.done = true;
                    return Err(error);
                }
                Err(error) => {
                    // Preserve the successfully read prefix. The failure is
                    // the next item/read and then the stream is fused.
                    self.pending_error = Some(error);
                    break;
                }
            }
        }
        Ok(filled)
    }
}

impl Iterator for ByteStream<'_> {
    type Item = Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let mut bytes = Vec::new();
        if let Err(source) = bytes.try_reserve_exact(self.batch_size) {
            self.done = true;
            return Some(Err(Error::Io(std::io::Error::other(format!(
                "cannot allocate a {}-byte stream batch: {source}",
                self.batch_size
            )))));
        }
        bytes.resize(self.batch_size, 0);
        match self.read_filled(&mut bytes) {
            Ok(0) => None,
            Ok(read) => {
                bytes.truncate(read);
                Some(Ok(bytes))
            }
            Err(error) => Some(Err(error)),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }
}

impl std::iter::FusedIterator for ByteStream<'_> {}

impl Read for ByteStream<'_> {
    fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
        // A caller-provided buffer is already bounded. Capping each source
        // request keeps `batch_size` authoritative even for a very large one.
        let length = target.len().min(self.batch_size);
        self.read_filled(&mut target[..length])
            .map_err(into_io_error)
    }
}

impl std::fmt::Debug for ByteStream<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ByteStream")
            .field("batch_size", &self.batch_size)
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

trait ByteSource {
    fn read_bytes(&mut self, target: &mut [u8]) -> Result<usize>;
}

struct PositionalSource<'source, H: IOBase + ?Sized> {
    handle: &'source H,
    position: u64,
}

impl<H: IOBase + ?Sized> ByteSource for PositionalSource<'_, H> {
    fn read_bytes(&mut self, target: &mut [u8]) -> Result<usize> {
        let read = self.handle.pread(self.position, target)?;
        self.position = self.position.checked_add(read as u64).ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "byte stream position exceeds u64::MAX",
            ))
        })?;
        Ok(read)
    }
}

struct CursorSource<'source, C: IOCursor + ?Sized>(&'source mut C);

impl<C: IOCursor + ?Sized> ByteSource for CursorSource<'_, C> {
    fn read_bytes(&mut self, target: &mut [u8]) -> Result<usize> {
        self.0.read_next(target)
    }
}

/// One positional stream tied to the disjoint position field of its cursor.
///
/// This is what keeps a `Cursor<Buffered<_>>` off the page cache and a
/// `Cursor<Coding<_>>` on one decoder rather than rebuilding either path for
/// each output chunk.
struct AdvancingSource<'source> {
    stream: ByteStream<'source>,
    position: &'source mut u64,
}

impl ByteSource for AdvancingSource<'_> {
    fn read_bytes(&mut self, target: &mut [u8]) -> Result<usize> {
        let read = self.stream.read_filled(target)?;
        *self.position = (*self.position).checked_add(read as u64).ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "byte stream position exceeds u64::MAX",
            ))
        })?;
        Ok(read)
    }
}

struct ReaderSource<R>(R);

impl<R: Read> ByteSource for ReaderSource<R> {
    fn read_bytes(&mut self, target: &mut [u8]) -> Result<usize> {
        loop {
            match self.0.read(target) {
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                result => return result.map_err(Error::Io),
            }
        }
    }
}

struct FileSystemSource<'source> {
    reader: Box<dyn crate::holder::fs::ByteReader + 'source>,
}

struct RandomFileSystemSource<'source> {
    reader: Box<dyn crate::holder::fs::RandomAccessReader + 'source>,
}

impl ByteSource for RandomFileSystemSource<'_> {
    fn read_bytes(&mut self, target: &mut [u8]) -> Result<usize> {
        let read = self.reader.read(target)?;
        if read == 0 && !self.reader.closed() {
            self.reader.close()?;
        }
        Ok(read)
    }
}

impl Drop for RandomFileSystemSource<'_> {
    fn drop(&mut self) {
        if !self.reader.closed() {
            let _ = self.reader.close();
        }
    }
}

impl ByteSource for FileSystemSource<'_> {
    fn read_bytes(&mut self, target: &mut [u8]) -> Result<usize> {
        let read = self.reader.read(target)?;
        if read == 0 && !self.reader.closed() {
            self.reader.close()?;
        }
        Ok(read)
    }
}

impl Drop for FileSystemSource<'_> {
    fn drop(&mut self) {
        if !self.reader.closed() {
            let _ = self.reader.close();
        }
    }
}

fn into_io_error(error: Error) -> std::io::Error {
    match error {
        Error::Io(error) => error,
        error => std::io::Error::other(error),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::holder::Buffer;

    struct CountedReader {
        bytes: std::io::Cursor<Vec<u8>>,
        reads: Arc<AtomicUsize>,
    }

    impl Read for CountedReader {
        fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.bytes.read(target)
        }
    }

    #[test]
    fn positional_streams_start_lazily_and_chunk_exactly() {
        let handle = Buffer::from_bytes(b"0123456789".to_vec());
        let chunks = handle
            .pstream_bytes(2, 3)
            .unwrap()
            .collect::<Result<Vec<_>>>()
            .unwrap();
        assert_eq!(chunks, [b"234".to_vec(), b"567".to_vec(), b"89".to_vec()]);

        let reads = Arc::new(AtomicUsize::new(0));
        let mut stream = ByteStream::from_reader(
            CountedReader {
                bytes: std::io::Cursor::new(b"abc".to_vec()),
                reads: Arc::clone(&reads),
            },
            2,
        )
        .unwrap();
        assert_eq!(reads.load(Ordering::Relaxed), 0);
        assert_eq!(stream.next().unwrap().unwrap(), b"ab");
        assert!(reads.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn zero_batch_size_is_refused_before_reading() {
        let error = Buffer::new().pstream_bytes(0, 0).unwrap_err();
        assert!(
            matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::InvalidInput)
        );
    }

    #[test]
    fn a_byte_stream_is_a_bounded_standard_reader() {
        let handle = Buffer::from_bytes(b"abcdefgh".to_vec());
        let mut stream = handle.pstream_bytes(1, 3).unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"bcdefgh");
    }

    #[test]
    fn positional_streaming_remains_object_safe() {
        let handle: Box<dyn IOBase> = Box::new(Buffer::from_bytes(b"abcdef".to_vec()));
        let chunks = handle
            .pstream_bytes(1, 2)
            .unwrap()
            .collect::<Result<Vec<_>>>()
            .unwrap();
        assert_eq!(chunks, [b"bc".to_vec(), b"de".to_vec(), b"f".to_vec()]);
    }

    #[test]
    fn a_cursor_stream_advances_only_as_it_is_consumed() {
        let mut cursor = Buffer::from_bytes(b"01234567".to_vec()).cursor_at(2);
        {
            let mut stream = cursor.stream_bytes(3).unwrap();
            assert_eq!(stream.next().unwrap().unwrap(), b"234");
        }
        assert_eq!(cursor.tell(), 5);
        assert_eq!(cursor.read_next(&mut [0_u8; 0]).unwrap(), 0);
    }

    struct FailsAfterPrefix(bool);

    impl Read for FailsAfterPrefix {
        fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
            if self.0 {
                return Err(std::io::Error::other("later failure"));
            }
            self.0 = true;
            target[..2].copy_from_slice(b"ok");
            Ok(2)
        }
    }

    #[test]
    fn a_failure_follows_its_prefix_once_then_fuses() {
        let mut stream = ByteStream::from_reader(FailsAfterPrefix(false), 4).unwrap();
        assert_eq!(stream.next().unwrap().unwrap(), b"ok");
        assert!(stream.next().is_some_and(|item| item.is_err()));
        assert!(stream.next().is_none());
        assert!(stream.next().is_none());
    }
}
