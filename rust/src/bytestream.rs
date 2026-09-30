//! Lazy byte chunks over positional and cursor-based handles.

use std::io::Read;

use super::{IOBase, IOCursor};
use crate::{Error, Result};

/// A lazy, bounded stream of byte arrays.
///
/// [`IOBase::pstream_bytes`] builds this over an explicit position and
/// [`IOCursor::stream_bytes`] builds it over a cursor's current position. The
/// source is not read until the first item (or [`Read::read`]) is requested;
/// a container's stream has started its listing by then, and opens each leaf
/// when it reaches it ([`Self::from_container`]).
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
    /// Whether this is a container's stream: the leaves beneath it, each
    /// already decoded by its own name, rather than one value's bytes.
    container: bool,
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
        reader: Box<dyn crate::fs::ByteReader + 'source>,
        batch_size: usize,
    ) -> Result<Self> {
        Self::from_source(FileSystemSource { reader }, batch_size)
    }

    /// Stream one random-access filesystem reader after it has been positioned.
    pub fn from_fs_random_reader(
        reader: Box<dyn crate::fs::RandomAccessReader + 'source>,
        batch_size: usize,
    ) -> Result<Self> {
        Self::from_source(RandomFileSystemSource { reader }, batch_size)
    }

    /// Stream the leaves beneath a container, one after another.
    ///
    /// The leaves are every one [`IOBase::children_where`] answers with no
    /// filter - recursive under a folder, what a glob matches under a
    /// pattern, containers dropped and names beginning with a dot left out -
    /// in the backend's listing order, with nothing between them. Each leaf
    /// contributes its content: the codings its own media type declares are
    /// taken off - each one [`crate::Codec`] decodes, gzip, zlib and zstd,
    /// the one table every decoded read goes through - because a container's
    /// media type declares none, so `a.log.gz` contributes its text. A leaf
    /// with no bytes contributes nothing, and a handle that holds no leaves
    /// streams nothing.
    ///
    /// `position` counts content bytes across the leaves: a leaf wholly
    /// before it is passed by its size without being opened, a coded one is
    /// decoded to count, and one that sizes itself empty is read to count,
    /// so a leaf nobody could size fails where it stands rather than shifting
    /// every later byte. Nothing but the listing is started here; each
    /// leaf is opened when the stream reaches it and dropped when drained,
    /// through the leaf's own sequential read where it has one. A listing or
    /// leaf failure is yielded after what came before it, and the stream is
    /// fused after it.
    ///
    /// The stream owns what it reads, so it outlives the borrow of
    /// `container`: this is the [`IOBase::pstream_bytes`] every container role
    /// answers, and what a binding holds for the life of one iterator.
    ///
    /// # Errors
    ///
    /// Returns [`std::io::ErrorKind::InvalidInput`] when `batch_size` is
    /// zero, or the refusal to list `container` - a pattern that cannot be
    /// decomposed.
    pub fn from_container(
        container: &(impl IOBase + ?Sized),
        position: u64,
        batch_size: usize,
    ) -> Result<ByteStream<'static>> {
        if batch_size == 0 {
            return Err(zero_batch());
        }
        ByteStream::from_source(
            LeafSource {
                leaves: container.children_where(&[], false)?,
                current: None,
                skip: position,
                batch_size,
            },
            batch_size,
        )
        .map(ByteStream::with_container)
    }

    /// Whether this is a container's stream ([`Self::from_container`]): a
    /// wrapper that would decode or index one value passes it through
    /// instead, since each leaf was already read as its own name says.
    pub(crate) const fn is_container(&self) -> bool {
        self.container
    }

    /// This stream, marked as a container's: what a wrapper re-batching a
    /// container's stream answers, so the mark survives it.
    pub(crate) const fn with_container(mut self) -> Self {
        self.container = true;
        self
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
            return Err(zero_batch());
        }
        Ok(Self {
            source: Box::new(source),
            batch_size,
            pending_error: None,
            done: false,
            container: false,
        })
    }

    /// Read up to `target.len()` bytes, filling it unless the stream ends.
    pub(crate) fn read_filled(&mut self, target: &mut [u8]) -> Result<usize> {
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
                Ok(read) if read <= target.len() - filled => filled += read,
                Ok(read) => {
                    let error = Error::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "byte source reported {read} bytes for a {}-byte buffer",
                            target.len() - filled
                        ),
                    ));
                    if filled == 0 {
                        self.done = true;
                        return Err(error);
                    }
                    self.pending_error = Some(error);
                    break;
                }
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

/// One source of bytes a [`ByteStream`] pulls its chunks from.
///
/// The crate root declares `bytestream` privately, so this reaches nobody
/// outside the crate except through `internals`.
pub trait ByteSource {
    /// Fill as much of `target` as the source can give, answering how many
    /// bytes were written.
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
    reader: Box<dyn crate::fs::ByteReader + 'source>,
}

struct RandomFileSystemSource<'source> {
    reader: Box<dyn crate::fs::RandomAccessReader + 'source>,
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

/// The refusal of a stream that could never yield a byte.
fn zero_batch() -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "byte stream batch_size must be greater than zero",
    ))
}

/// The leaves of one container read end to end: the listing still to walk,
/// the one leaf being read, and the content bytes still owed to `position`.
///
/// What it holds is bounded by one leaf: the listing's own frontier, and one
/// open read with at most one decoder over it.
struct LeafSource {
    leaves: crate::Listing,
    current: Option<LeafRead>,
    skip: u64,
    batch_size: usize,
}

/// One leaf being read: its stored stream as it is, or that stream under the
/// decoders its codings name - a failure the stored stream raised through
/// them still arriving as the typed error it was.
enum LeafRead {
    Stored(ByteStream<'static>),
    Decoded(Box<dyn Read>),
}

impl ByteSource for LeafSource {
    fn read_bytes(&mut self, target: &mut [u8]) -> Result<usize> {
        loop {
            if let Some(reader) = self.current.as_mut() {
                let read = match reader {
                    LeafRead::Stored(stream) => stream.read_filled(target)?,
                    LeafRead::Decoded(reader) => read_retrying(reader.as_mut(), target)?,
                };
                if read > 0 {
                    return Ok(read);
                }
                // Drained: an empty read ends the whole stream, so the next
                // leaf is opened rather than answering one here.
                self.current = None;
            }
            let Some(leaf) = self.leaves.next() else {
                return Ok(0);
            };
            self.current = self.open(leaf?)?;
        }
    }
}

impl LeafSource {
    /// Open one leaf at the content bytes still owed, or pass it by when
    /// they cover it whole.
    fn open(&mut self, leaf: crate::holder::Holder) -> Result<Option<LeafRead>> {
        let codings = leaf.media_type().encodings().to_vec();
        if codings.is_empty() {
            let size = if self.skip > 0 { leaf.size() } else { 0 };
            // A leaf that sizes itself empty is read rather than believed:
            // `size` answers zero for one it could not ask about, and passing
            // that by would shift every later byte instead of failing.
            if size > 0 && self.skip >= size {
                self.skip -= size;
                return Ok(None);
            }
            let offset = if size > 0 {
                std::mem::take(&mut self.skip)
            } else {
                0
            };
            let mut stream = leaf.into_byte_stream(offset, self.batch_size)?;
            let open = self.discard(|target| stream.read_filled(target))?;
            return Ok(open.then_some(LeafRead::Stored(stream)));
        }
        // A decoder pulls its own small windows, so the stored bytes are
        // fetched a transport window at a time - whatever batch the caller
        // asked the decoded stream for.
        let window = crate::DEFAULT_FETCH_BYTE_SIZE;
        let mut stored =
            std::io::BufReader::with_capacity(window, leaf.into_byte_stream(0, window)?);
        // A leaf holding nothing - absent by the time it is read - holds no
        // coded stream either, and a decoder would refuse its missing header.
        if std::io::BufRead::fill_buf(&mut stored)
            .map_err(from_io_error)?
            .is_empty()
        {
            return Ok(None);
        }
        let mut reader = crate::Codec::decoding(&codings, stored);
        // A coded leaf has no seek: the content before `position` is only
        // reached by decoding it.
        let open = self.discard(|target| read_retrying(reader.as_mut(), target))?;
        Ok(open.then_some(LeafRead::Decoded(reader)))
    }

    /// Read past the content bytes still owed, answering whether the leaf
    /// holds any beyond them.
    fn discard(&mut self, mut read: impl FnMut(&mut [u8]) -> Result<usize>) -> Result<bool> {
        let mut discarded = [0_u8; 8 * 1024];
        while self.skip > 0 {
            let length = usize::try_from(self.skip)
                .unwrap_or(usize::MAX)
                .min(discarded.len());
            let count = read(&mut discarded[..length])?;
            if count == 0 {
                return Ok(false);
            }
            self.skip -= count as u64;
        }
        Ok(true)
    }
}

/// One read, retried while the source is only interrupted.
fn read_retrying(reader: &mut dyn Read, target: &mut [u8]) -> Result<usize> {
    loop {
        match reader.read(target) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => return result.map_err(from_io_error),
        }
    }
}

/// The typed error a stream's [`Read`] carried through a decoder, or the I/O
/// failure itself - the inverse of [`into_io_error`].
fn from_io_error(error: std::io::Error) -> Error {
    match error.downcast::<Error>() {
        Ok(error) => error,
        Err(error) => Error::Io(error),
    }
}

fn into_io_error(error: Error) -> std::io::Error {
    match error {
        Error::Io(error) => error,
        error => std::io::Error::other(error),
    }
}

/// Lazily discard a prefix of a reader before serving the requested position.
///
/// A transformed stream has no seek: the bytes at a decoded or decompressed
/// offset are only reachable by producing everything before them. Both
/// transforming handles - [`crate::coding::Coding`] and
/// [`crate::charset::Transcoded`] - reach a position this way, through one
/// bounded scratch buffer that retains nothing.
pub(crate) struct SkipReader<R> {
    reader: R,
    remaining: u64,
}

impl<R> SkipReader<R> {
    /// Serve `reader` from `remaining` bytes in.
    pub(crate) const fn new(reader: R, remaining: u64) -> Self {
        Self { reader, remaining }
    }
}

impl<R: Read> Read for SkipReader<R> {
    fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
        if target.is_empty() {
            return Ok(0);
        }
        let mut discarded = [0_u8; 8 * 1024];
        while self.remaining > 0 {
            let length = usize::try_from(self.remaining)
                .unwrap_or(usize::MAX)
                .min(discarded.len());
            let read = self.reader.read(&mut discarded[..length])?;
            if read == 0 {
                self.remaining = 0;
                return Ok(0);
            }
            self.remaining -= read as u64;
        }
        self.reader.read(target)
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/bytestream.rs` pins and a caller cannot reach.
    //!
    //! [`ByteSource`](super::ByteSource) is the one thing a chunked stream
    //! pulls through, and a source that over-reports what it wrote is only
    //! reachable by writing one, so the trait itself has to be nameable. The
    //! crate root declares `bytestream` privately, so it reaches nobody
    //! without the feature; the constructor stays behind a forwarder.

    use super::{ByteStream, Result};

    pub use super::ByteSource;

    /// Chunk `source` into batches of `batch_size` bytes.
    pub fn from_source<'source>(
        source: impl ByteSource + 'source,
        batch_size: usize,
    ) -> Result<ByteStream<'source>> {
        ByteStream::from_source(source, batch_size)
    }
}
