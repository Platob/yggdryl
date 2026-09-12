//! A handle that decodes on the way in and encodes on the way out.

use std::io::{BufReader, Read};
use std::sync::OnceLock;

use super::Charset;
use crate::bytestream::SkipReader;
use crate::holder::Holder;
use crate::{ByteStream, DEFAULT_STREAM_BATCH_SIZE, IOBase, MediaType, Result, Url};

/// A transparent charset decode over one handle.
///
/// [`Charset`] says *which* encoding a payload uses; this is that encoding
/// applied to a handle. Reading through one decodes and writing through one
/// encodes, so everything downstream - a record encoding, a structured codec,
/// another handle - sees UTF-8 and nothing below it ever learns the charset.
/// That is what keeps charset handling at one seam: a `windows-1252` CSV is
/// read by the ordinary text reader, an `iso-8859-1` JSON document by the
/// ordinary JSON parser, and neither of them gained a charset argument.
///
/// The media type reported here is the *decoded* one: wrapping a handle that
/// says `text/csv;charset=windows-1252` yields a handle that says `text/csv`,
/// because that is what its bytes now are.
///
/// ```
/// use yggdryl::charset::Transcoded;
/// use yggdryl::holder::Buffer;
/// use yggdryl::{Charset, IOBase};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut handle = Transcoded::new(Buffer::new(), Charset::Cp1252);
/// handle.write_all_bytes("symbol,désk\n".as_bytes())?;
/// handle.flush()?;
///
/// // The caller reads UTF-8 back out.
/// assert_eq!(handle.read_all_bytes()?, "symbol,désk\n".as_bytes());
/// // The bytes underneath are one per scalar.
/// assert_eq!(handle.handle().read_all_bytes()?, b"symbol,d\xe9sk\n");
/// # Ok(())
/// # }
/// ```
///
/// # Random access
///
/// A decode has no seek of its own - the byte at a decoded offset is only
/// reachable by decoding what precedes it - so this handle builds one, out of
/// the streaming decoder it already has. The first positional read past the
/// beginning makes one streamed pass and records a *resume point* every
/// [`DEFAULT_STREAM_BATCH_SIZE`] decoded bytes: the source offset and the
/// decoded offset of the same position, taken only where the decoder holds no
/// partial sequence. Every later positional read binary-searches that index,
/// asks the wrapped handle for bytes *at* the resume point, and decodes
/// forward at most one stride. So a random read costs one backing-store read
/// and a bounded decode instead of a decode of the whole prefix, and the index
/// costs sixteen bytes per stride.
///
/// Two properties make this sound, and both are contracts of this module
/// rather than assumptions: every charset here is stateless past a sequence
/// boundary, so a byte offset is the whole of what a resume needs; and
/// [`Decoder::is_pending`] is what says a boundary has been reached. A
/// shift-state encoding would need more than an offset to resume at, and
/// would have to carry that state into the index.
///
/// [`Decoder::is_pending`]: super::Decoder::is_pending
///
/// A sequential read pays none of it: reading from offset zero streams
/// straight through and never builds the index. Positional mutation and an
/// explicitly opened session materialize the decoded value until
/// [`IOBase::close`], and pending writes are published on [`IOBase::flush`]
/// rather than on every `pwrite`.
#[derive(Debug)]
pub struct Transcoded<H: IOBase> {
    handle: H,
    charset: Charset,
    /// The decoded UTF-8 value, materialized on first use.
    plain: Option<Vec<u8>>,
    /// Whether `plain` holds changes the wrapped handle has not seen.
    dirty: bool,
    /// The wrapped media type with this charset removed.
    media_type: MediaType,
    /// Where a decode may resume, built by one pass on first random access.
    resume: OnceLock<Resumes>,
}

/// How far apart the points a positional read resumes from are.
///
/// A read decodes forward from the point before it, so this bounds the work a
/// random access pays, and the index costs one point per stride.
const RESUME_STRIDE: u64 = DEFAULT_STREAM_BATCH_SIZE as u64;

/// One position, in both spellings.
#[derive(Clone, Copy, Debug)]
struct Resume {
    /// The offset in the wrapped handle's encoded bytes.
    source: u64,
    /// The offset in the decoded bytes.
    decoded: u64,
}

/// Every resume point of one value, and the decoded size that pass measured.
///
/// The two are answered by the same pass because they are the same walk: the
/// size is where the last point's stride ran out.
#[derive(Debug)]
struct Resumes {
    /// Ascending by both offsets, always starting at the beginning.
    points: Vec<Resume>,
    /// The decoded length in bytes.
    size: u64,
}

impl<H: IOBase> Transcoded<H> {
    /// Wrap a handle in one charset without touching it.
    pub fn new(handle: H, charset: Charset) -> Self {
        let media_type = handle.media_type().clone().without_charset();
        Self {
            handle,
            charset,
            plain: None,
            dirty: false,
            media_type,
            resume: OnceLock::new(),
        }
    }

    /// Wrap a handle in the charset its own media type declares.
    ///
    /// A handle declaring none is wrapped in [`Charset::Utf8`], which changes
    /// nothing about its bytes.
    pub fn infer(handle: H) -> Self {
        let charset = Charset::from_media_type(handle.media_type());
        Self::new(handle, charset)
    }

    /// Return the charset applied to the wrapped handle.
    pub const fn charset(&self) -> Charset {
        self.charset
    }

    /// Borrow the wrapped handle, which holds the encoded bytes.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Consume this handle, publishing any pending write first.
    ///
    /// # Errors
    ///
    /// Returns the encode or write failure.
    pub fn into_handle(mut self) -> Result<H> {
        self.publish()?;
        Ok(self.handle)
    }

    /// Materialize the decoded value, decoding the wrapped bytes once.
    fn decoded(&mut self) -> Result<&mut Vec<u8>> {
        if self.plain.is_none() {
            let mut plain = Vec::new();
            self.pstream_bytes(0, DEFAULT_STREAM_BATCH_SIZE)?
                .read_to_end(&mut plain)?;
            self.plain = Some(plain);
        }
        self.plain
            .as_mut()
            .ok_or_else(|| crate::Error::Io(std::io::Error::other("the decoded value was lost")))
    }

    /// Borrow the decoded value when this handle is holding one.
    const fn materialized(&self) -> Option<&Vec<u8>> {
        self.plain.as_ref()
    }

    /// Open the decoded byte stream, fetching the transport `window` at a time.
    ///
    /// The caller's batch bounds decoded output; `window` bounds what the
    /// backing store is asked for, so an eight-byte positional read fetches
    /// eight bytes' worth of transport rather than a megabyte of it.
    ///
    /// A stream from the beginning starts at the beginning and builds nothing.
    /// A stream from anywhere else resumes at the point before it, so the
    /// decode it discards is at most one stride rather than the whole prefix.
    fn decoded_stream(
        &self,
        position: u64,
        batch_size: usize,
        window: usize,
    ) -> Result<ByteStream<'_>> {
        if let Some(plain) = self.materialized() {
            let plain = usize::try_from(position)
                .ok()
                .and_then(|position| plain.get(position..))
                .unwrap_or_default();
            return ByteStream::from_reader(std::io::Cursor::new(plain), batch_size);
        }
        if self.charset.is_utf8() {
            return self.handle.pstream_bytes(position, batch_size);
        }
        let start = self.resume_before(position)?;
        let encoded = self.handle.pstream_bytes(start.source, window)?;
        ByteStream::from_reader(
            SkipReader::new(
                self.charset
                    .reader(BufReader::with_capacity(window, encoded)),
                position - start.decoded,
            ),
            batch_size,
        )
    }

    /// The resume point at or before `position`.
    ///
    /// The beginning is a resume point every value has, and answering it
    /// without consulting the index is what keeps a sequential read one pass:
    /// nothing but a genuine seek ever pays for the scan.
    fn resume_before(&self, position: u64) -> Result<Resume> {
        if position == 0 {
            return Ok(Resume {
                source: 0,
                decoded: 0,
            });
        }
        let points = &self.resumes()?.points;
        let index = points.partition_point(|point| point.decoded <= position);
        Ok(points
            .get(index.saturating_sub(1))
            .copied()
            .unwrap_or(Resume {
                source: 0,
                decoded: 0,
            }))
    }

    /// The resume index, built once and shared by every later read.
    ///
    /// This is the rule an answer only the store can give follows: one pass
    /// measures the decoded size and the resume points together, because they
    /// are the same walk, and every handle on the value reads them from here
    /// rather than walking again.
    fn resumes(&self) -> Result<&Resumes> {
        if let Some(resumes) = self.resume.get() {
            return Ok(resumes);
        }
        let scanned = self.scan_resumes()?;
        Ok(self.resume.get_or_init(|| scanned))
    }

    /// Walk the value once, recording where a decode may resume.
    ///
    /// A point is taken only where the decoder holds no partial sequence, so
    /// the source offset alone is enough to start again there.
    fn scan_resumes(&self) -> Result<Resumes> {
        let mut points = vec![Resume {
            source: 0,
            decoded: 0,
        }];
        let mut decoder = self.charset.decoder();
        let mut source = self
            .handle
            .pstream_bytes(0, crate::DEFAULT_FETCH_BYTE_SIZE)?;
        let mut chunk = vec![0_u8; DEFAULT_STREAM_BATCH_SIZE];
        let mut text = Vec::with_capacity(DEFAULT_STREAM_BATCH_SIZE);
        let mut size = 0_u64;
        let mut next = RESUME_STRIDE;
        loop {
            let read = source.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            text.clear();
            decoder.push_bytes(&chunk[..read], &mut text)?;
            size = size.checked_add(text.len() as u64).ok_or_else(|| {
                crate::Error::Io(std::io::Error::other(
                    "decoded byte stream exceeds u64::MAX",
                ))
            })?;
            if size >= next && !decoder.is_pending() {
                points.push(Resume {
                    source: decoder.consumed(),
                    decoded: size,
                });
                next = size.saturating_add(RESUME_STRIDE);
            }
        }
        decoder.finish()?;
        Ok(Resumes { points, size })
    }

    /// Drop the resume index, which a write has made a claim about old bytes.
    fn invalidate(&mut self) {
        self.resume = OnceLock::new();
    }

    /// The transport window one positional read of `length` bytes asks for.
    const fn positional_window(length: usize) -> usize {
        if length < DEFAULT_STREAM_BATCH_SIZE {
            DEFAULT_STREAM_BATCH_SIZE
        } else if length > crate::DEFAULT_FETCH_BYTE_SIZE {
            crate::DEFAULT_FETCH_BYTE_SIZE
        } else {
            length
        }
    }

    /// Write the decoded value back through the charset.
    fn publish(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let plain = self.plain.take().unwrap_or_default();
        let text = std::str::from_utf8(&plain).map_err(|error| {
            super::undecodable(
                Charset::Utf8.as_str(),
                error.valid_up_to(),
                plain.get(error.valid_up_to()).copied().unwrap_or_default(),
            )
        })?;
        let encoded = self.charset.encode(text)?;
        self.handle.write_all_bytes(&encoded)?;
        drop(encoded);
        self.plain = Some(plain);
        self.dirty = false;
        self.invalidate();
        Ok(())
    }
}

/// Copy `buffer.len()` bytes of `plain` from `offset`, returning what fit.
fn copy_range(plain: &[u8], offset: u64, buffer: &mut [u8]) -> usize {
    let Ok(offset) = usize::try_from(offset) else {
        return 0;
    };
    let Some(available) = plain.len().checked_sub(offset) else {
        return 0;
    };
    let read = available.min(buffer.len());
    buffer[..read].copy_from_slice(&plain[offset..offset + read]);
    read
}

impl<H: IOBase> crate::IOMedia for Transcoded<H> {
    crate::impl_default_iomedia!();
}

impl<H: IOBase> IOBase for Transcoded<H> {
    /// Read the range out of the decoded value.
    ///
    /// An open handle answers from the value it already holds; a closed one
    /// decodes only far enough to fill this call and retains nothing.
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        if let Some(plain) = self.materialized() {
            return Ok(copy_range(plain, offset, buffer));
        }
        if buffer.is_empty() {
            return Ok(0);
        }
        Ok(self
            .decoded_stream(offset, buffer.len(), Self::positional_window(buffer.len()))?
            .read(buffer)?)
    }

    /// Stream decoded bytes without materializing the decoded value.
    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<ByteStream<'_>> {
        // A stream is read to its end, so it fetches whole windows.
        self.decoded_stream(position, batch_size, crate::DEFAULT_FETCH_BYTE_SIZE)
    }

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        if let Some(plain) = self.materialized() {
            return Ok(plain.clone());
        }
        let mut bytes = Vec::new();
        self.pstream_bytes(0, DEFAULT_STREAM_BATCH_SIZE)?
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        if length == 0 {
            return Ok(Vec::new());
        }
        if let Some(plain) = self.materialized() {
            let Some(offset) = usize::try_from(offset)
                .ok()
                .filter(|offset| *offset < plain.len())
            else {
                return Ok(Vec::new());
            };
            return Ok(plain[offset..plain.len().min(offset.saturating_add(length))].to_vec());
        }
        let mut bytes = Vec::with_capacity(length.min(DEFAULT_STREAM_BATCH_SIZE));
        Read::take(
            self.decoded_stream(
                offset,
                length.clamp(1, DEFAULT_STREAM_BATCH_SIZE),
                Self::positional_window(length),
            )?,
            length as u64,
        )
        .read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let offset = usize::try_from(offset).map_err(|_| {
            crate::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("expected an offset within a decoded buffer, got {offset}"),
            ))
        })?;
        let plain = self.decoded()?;
        let end = offset.checked_add(bytes.len()).ok_or_else(|| {
            crate::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "decoded write range exceeds addressable memory",
            ))
        })?;
        if plain.len() < end {
            // Writing past the end grows the value and zero-fills any gap.
            plain.resize(end, 0);
        }
        plain[offset..end].copy_from_slice(bytes);
        self.dirty = true;
        self.invalidate();
        Ok(bytes.len())
    }

    /// The decoded length, measured by the same pass that indexes the value.
    ///
    /// A charset changes how many bytes a scalar takes, so this is a read
    /// rather than a forward of the wrapped size - but it is the read the
    /// resume index already needed, so asking twice costs one pass.
    fn size(&self) -> u64 {
        if let Some(plain) = self.materialized() {
            return plain.len() as u64;
        }
        if self.charset.is_utf8() {
            return self.handle.size();
        }
        self.resumes().map_or(0, |resumes| resumes.size)
    }

    fn capacity(&self) -> u64 {
        self.plain
            .as_ref()
            .map_or_else(|| self.size(), |plain| plain.capacity() as u64)
    }

    fn reserve(&mut self, capacity: u64) -> Result<()> {
        let capacity = usize::try_from(capacity).unwrap_or(usize::MAX);
        let plain = self.decoded()?;
        if capacity > plain.len() {
            plain.reserve(capacity - plain.len());
        }
        Ok(())
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        let size = usize::try_from(size).unwrap_or(usize::MAX);
        let plain = self.decoded()?;
        if size < plain.len() {
            plain.truncate(size);
        } else {
            plain.resize(size, 0);
        }
        self.dirty = true;
        self.invalidate();
        Ok(())
    }

    fn url(&self) -> Option<&Url> {
        self.handle.url()
    }

    fn bound_location(&self) -> Option<&crate::holder::fs::BoundLocation> {
        self.handle.bound_location()
    }

    /// The storage role is the wrapped handle's; a charset changes the bytes,
    /// not where they live.
    fn kind(&self) -> crate::IOKind {
        self.handle.kind()
    }

    fn media_type(&self) -> &MediaType {
        &self.media_type
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.media_type = media_type;
    }

    fn flush(&mut self) -> Result<()> {
        self.publish()?;
        self.handle.flush()
    }

    fn open(&mut self) -> Result<()> {
        self.handle.open()?;
        self.decoded()?;
        Ok(())
    }

    fn opened(&self) -> bool {
        self.plain.is_some()
    }

    fn close(&mut self) -> Result<()> {
        self.publish()?;
        self.plain = None;
        self.handle.close()
    }

    fn parent(&self) -> Option<Holder> {
        self.handle.parent()
    }

    fn child_by_path(&self, name: &str) -> Result<Holder> {
        self.handle.child_by_path(name)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> crate::Listing {
        self.handle.ls(recursive, include_private)
    }

    /// Empty the encoded resource, dropping the decoded value with it.
    fn clear(&mut self) -> Result<()> {
        self.plain = None;
        self.dirty = false;
        self.invalidate();
        self.handle.clear()
    }

    /// Delete the *encoded* resource, not merely this view of it.
    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.plain = None;
        self.dirty = false;
        self.invalidate();
        self.handle.remove(recursive)
    }
}
