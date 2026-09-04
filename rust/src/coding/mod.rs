//! A handle that compresses on the way out and decompresses on the way in.
//!
//! [`Coding`] wraps any [`IOBase`] and presents the *decoded* bytes: reads
//! decompress, writes compress, and the wrapped handle only ever holds the
//! encoded form. The per-format aliases - [`crate::coding::gzip::Gzip`],
//! [`crate::coding::zlib::Zlib`], [`crate::coding::zstd::Zstd`] - are this type with the codec
//! already chosen.
//!
//! A coding is not seekable, so positional mutation and an explicitly opened
//! session materialize the decoded value until [`IOBase::close`]. Sequential
//! reads decode through one bounded stream and retain no earlier page. Pending
//! writes are published on [`IOBase::flush`] rather than on every `pwrite`.

mod coded;
pub mod gzip;
pub mod zlib;
pub mod zstd;

pub use coded::Coded;

use std::io::Read;

use crate::generic::Holder;
use crate::{ByteStream, DEFAULT_STREAM_BATCH_SIZE, IOBase};
use crate::{Codec, Level, MediaType, Result, Url};

/// A transparent compression buffer over one handle.
///
/// The media type reported here is the *decoded* one: wrapping a handle named
/// `trades.arrows.gz` in a gzip coding yields a handle that says
/// `application/vnd.apache.arrow.stream`, because that is what its bytes now
/// are.
#[derive(Debug)]
pub struct Coding<H: IOBase> {
    handle: H,
    codec: Codec,
    level: Level,
    /// The decoded value, materialized on first use.
    plain: Option<Vec<u8>>,
    /// Whether `plain` holds changes the wrapped handle has not seen.
    dirty: bool,
    /// The wrapped media type with this coding removed.
    media_type: MediaType,
}

impl<H: IOBase> Coding<H> {
    /// Wrap a handle in a content coding without touching it.
    pub fn new(handle: H, codec: Codec) -> Self {
        let media_type = decoded_media_type(handle.media_type(), codec);
        Self {
            handle,
            codec,
            level: Level::DEFAULT,
            plain: None,
            dirty: false,
            media_type,
        }
    }

    /// Return this handle with a different compression level.
    #[must_use]
    pub const fn with_level(mut self, level: Level) -> Self {
        self.level = level;
        self
    }

    /// Return the coding applied to the wrapped handle.
    pub const fn codec(&self) -> Codec {
        self.codec
    }

    /// Return the compression level writes use.
    pub const fn level(&self) -> Level {
        self.level
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
            let plain = {
                let mut plain = Vec::new();
                self.pstream_bytes(0, DEFAULT_STREAM_BATCH_SIZE)?
                    .read_to_end(&mut plain)?;
                plain
            };
            self.plain = Some(plain);
        }
        self.plain
            .as_mut()
            .ok_or_else(|| crate::Error::Io(std::io::Error::other("the decoded value was lost")))
    }

    /// Borrow the decoded value when this handle is holding one.
    ///
    /// Between [`IOBase::open`] and [`IOBase::close`] the decoded value is
    /// materialized, and every read-only accessor can answer straight out of
    /// it. Asking through [`Self::peek`] instead would *copy* it - which,
    /// over a value big enough to be worth compressing, is the whole payload
    /// per call.
    const fn materialized(&self) -> Option<&Vec<u8>> {
        self.plain.as_ref()
    }

    /// Count decoded bytes through one bounded window without retaining them.
    fn streamed_size(&self) -> Result<u64> {
        let mut source = self.pstream_bytes(0, DEFAULT_STREAM_BATCH_SIZE)?;
        let mut chunk = vec![0_u8; DEFAULT_STREAM_BATCH_SIZE];
        let mut size = 0_u64;
        loop {
            let read = source.read(&mut chunk)?;
            if read == 0 {
                return Ok(size);
            }
            size = size.checked_add(read as u64).ok_or_else(|| {
                crate::Error::Io(std::io::Error::other(
                    "decoded byte stream exceeds u64::MAX",
                ))
            })?;
        }
    }

    /// Write the decoded value back through the coding.
    fn publish(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let plain = self.plain.take().unwrap_or_default();
        let encoded = self.codec.dump_with_level(&plain, self.level)?;
        self.handle.write_all_bytes(&encoded)?;
        self.plain = Some(plain);
        self.dirty = false;
        Ok(())
    }

    /// Own the same presented bytes without retaining decoded pages here.
    ///
    /// A closed located view reopens the encoded resource and keeps its coding
    /// in the media type, so owning record readers decode it lazily. An
    /// unlocated source snapshots only the encoded bytes it already holds.
    /// Opened or dirty state must snapshot `plain`, because it is the stable
    /// presented value and may be newer than the resource underneath it.
    #[cfg(feature = "arrow")]
    fn owned_presented_handle(&self) -> Result<Holder> {
        if let Some(plain) = self.materialized() {
            return Ok(Holder::buffer(
                crate::io::Buffer::from_bytes(plain.clone())
                    .with_media_type(self.media_type.clone()),
            ));
        }

        let declared_codec = Codec::from_media_type(self.handle.media_type());
        // Raw DEFLATE has no distinct MIME spelling in the shared media model.
        // Keep explicit raw framing correct with one streamed decoded snapshot
        // instead of falsely labelling it as a zlib stream.
        if self.codec == Codec::Deflate && declared_codec != Codec::Deflate {
            let mut plain = Vec::new();
            self.pstream_bytes(0, DEFAULT_STREAM_BATCH_SIZE)?
                .read_to_end(&mut plain)?;
            return Ok(Holder::buffer(
                crate::io::Buffer::from_bytes(plain).with_media_type(self.media_type.clone()),
            ));
        }

        let encoded_media_type = if self.codec.is_identity() || declared_codec == self.codec {
            self.handle.media_type().clone()
        } else {
            self.handle
                .media_type()
                .clone()
                .try_with_encodings(crate::iobase::coding_mime(self.codec))?
        };
        if let Some(parent) = self.handle.parent() {
            if let Some(name) = self.handle.url().and_then(crate::Url::file_name) {
                let mut child = parent.child_by_path(name)?;
                child.set_media_type(encoded_media_type);
                return Ok(child);
            }
        }

        let mut encoded = crate::io::Buffer::new();
        self.handle.copy_into(&mut encoded)?;
        encoded.set_media_type(encoded_media_type);
        Ok(Holder::buffer(encoded))
    }

    /// Apply the generic read shaping after an owning encoding seam.
    #[cfg(feature = "arrow")]
    fn shape_owned_arrow_reader(
        reader: crate::arrow::BatchReader,
        options: &crate::generic::RecordOptions,
    ) -> Result<crate::arrow::BatchReader> {
        use crate::generic::IORecordOptions;

        let reader = match options.field() {
            Some(field) => crate::arrow::cast_reader(reader, &field, options.safe())?,
            None => reader,
        };
        let reader = crate::io::partition::filtered_reader(reader, options)?;
        options.limit_arrow_reader(crate::iobase::select_reader(reader, options)?)
    }
}

/// Copy `buffer.len()` bytes of `plain` from `offset`, returning what fit.
///
/// Absence and the end of the value are both emptiness here, exactly as
/// [`IOBase::pread`] spells them.
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

/// Remove `codec` from a media type's encoding sequence.
fn decoded_media_type(media_type: &MediaType, codec: Codec) -> MediaType {
    if codec.is_identity() {
        return media_type.clone();
    }
    let mut encodings = media_type.encodings().to_vec();
    if encodings
        .last()
        .is_some_and(|encoding| Codec::from_mime_type(encoding) == codec)
    {
        encodings.pop();
    }
    // The sequence came from a valid media type minus one entry, so it stays
    // valid; a rejected rebuild would mean the input was already invalid.
    MediaType::from_parts(media_type.base().clone(), encodings)
        .unwrap_or_else(|_| MediaType::new(media_type.base().clone()))
}

impl<H: IOBase> crate::IOMedia for Coding<H> {
    crate::impl_default_iomedia!();

    /// Keep an owning Arrow reader on the decoded view without caching it.
    ///
    /// A closed view reopens (or snapshots) its encoded source with the coding
    /// restored in the owned handle's media type, so decoding remains lazy.
    /// Only an opened or dirty view snapshots the decoded value it already
    /// owns.
    #[cfg(feature = "arrow")]
    fn read_arrow_reader(
        &self,
        options: &crate::generic::RecordOptions,
    ) -> Result<crate::arrow::BatchReader> {
        use crate::generic::IORecordOptions;

        let owned = self.owned_presented_handle()?;
        if owned.is_container() {
            return crate::IOMedia::read_arrow_reader(&owned, options);
        }
        let reader = match options {
            crate::generic::RecordOptions::Ipc(ipc) => {
                crate::ipc::read_owned_batch_reader(owned, options.field().as_ref(), ipc)?
            }
            crate::generic::RecordOptions::Text(text) => {
                crate::text::line::arrow::read_owned_arrow_reader(owned, text)?
            }
            _ => return crate::IOMedia::read_arrow_reader(&owned, options),
        };
        Self::shape_owned_arrow_reader(reader, options)
    }
}

impl<H: IOBase> IOBase for Coding<H> {
    /// Read the range out of the decoded value.
    ///
    /// An open handle answers from the value it already holds; a closed one
    /// decodes only far enough to fill this call. No closed positional read
    /// retains or allocates the whole decoded payload.
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        if let Some(plain) = self.materialized() {
            return Ok(copy_range(plain, offset, buffer));
        }
        if buffer.is_empty() {
            return Ok(0);
        }
        Ok(self.pstream_bytes(offset, buffer.len())?.read(buffer)?)
    }

    /// Stream decoded bytes without materializing the decoded value.
    ///
    /// Compression has no decoded seek, so a closed stream starts at the
    /// encoded beginning and discards decoded bytes up to `position` through
    /// one bounded scratch buffer. An opened or dirty handle already owns its
    /// decoded snapshot and streams a borrowed suffix of that value instead.
    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<ByteStream<'_>> {
        if let Some(plain) = self.materialized() {
            let plain = usize::try_from(position)
                .ok()
                .and_then(|position| plain.get(position..))
                .unwrap_or_default();
            return ByteStream::from_reader(std::io::Cursor::new(plain), batch_size);
        }
        if self.codec.is_identity() {
            return self.handle.pstream_bytes(position, batch_size);
        }
        // The caller's batch bounds decoded output. The encoded transport has
        // its own fixed window: coupling it to a one-byte output request would
        // turn one decode into one positional backend call per encoded byte.
        let encoded = self.handle.pstream_bytes(0, DEFAULT_STREAM_BATCH_SIZE)?;
        ByteStream::from_reader(
            SkipReader::new(LazyDecoder::new(self.codec, encoded), position),
            batch_size,
        )
    }

    /// Materialize one streamed decode, without first materializing the
    /// encoded value or decoding once merely to discover the decoded size.
    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        if let Some(plain) = self.materialized() {
            return Ok(plain.clone());
        }
        let mut bytes = Vec::new();
        self.pstream_bytes(0, DEFAULT_STREAM_BATCH_SIZE)?
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Read one decoded range through the streaming decoder.
    ///
    /// Only the requested result is retained. A closed compressed stream must
    /// still decode from its beginning to reach `offset`, but no earlier
    /// decoded page remains cached afterwards.
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
        std::io::Read::take(
            self.pstream_bytes(offset, length.clamp(1, DEFAULT_STREAM_BATCH_SIZE))?,
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
        Ok(bytes.len())
    }

    fn size(&self) -> u64 {
        if let Some(plain) = self.materialized() {
            return plain.len() as u64;
        }
        self.streamed_size().unwrap_or(0)
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
        Ok(())
    }

    fn url(&self) -> Option<&Url> {
        self.handle.url()
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
    ///
    /// The decoded buffer is discarded rather than published: a pending write
    /// must never survive an emptying to reappear on a later flush.
    fn clear(&mut self) -> Result<()> {
        self.plain = None;
        self.dirty = false;
        self.handle.clear()
    }

    /// Delete the *encoded* resource, not merely this view of it.
    ///
    /// A coding handle wraps bytes that live somewhere; complete removal is
    /// deleting those bytes, plus the decoded value held in `plain` and any
    /// unflushed `dirty` write, so a later flush cannot resurrect what was
    /// removed.
    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.plain = None;
        self.dirty = false;
        self.handle.remove(recursive)
    }
}

/// Construct a decoder only after the encoded source yields its first byte.
///
/// An absent resource is an empty decoded value, while an empty byte slice is
/// not a complete gzip, zlib, or Zstandard frame. Delaying construction lets
/// the handle contract win without probing its size and keeps construction
/// itself lazy.
struct LazyDecoder<'source> {
    codec: Codec,
    source: Option<ByteStream<'source>>,
    decoder: Option<Box<dyn Read + 'source>>,
}

impl<'source> LazyDecoder<'source> {
    const fn new(codec: Codec, source: ByteStream<'source>) -> Self {
        Self {
            codec,
            source: Some(source),
            decoder: None,
        }
    }
}

impl Read for LazyDecoder<'_> {
    fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
        if target.is_empty() {
            return Ok(0);
        }
        if self.decoder.is_none() {
            let Some(mut source) = self.source.take() else {
                return Ok(0);
            };
            // Probe with enough bytes for every supported framing header. A
            // one-byte probe costs a separate remote range request before the
            // decoder's first useful read; replaying this owned prefix keeps
            // absence lazy while making the first request useful.
            let mut prefix = [0_u8; 64];
            let read = source.read(&mut prefix)?;
            if read == 0 {
                return Ok(0);
            }
            self.decoder = Some(
                self.codec
                    .reader(std::io::Cursor::new(prefix).take(read as u64).chain(source)),
            );
        }
        self.decoder
            .as_mut()
            .map_or(Ok(0), |decoder| decoder.read(target))
    }
}

/// Lazily discard a decoded prefix before serving the requested position.
struct SkipReader<R> {
    reader: R,
    remaining: u64,
}

impl<R> SkipReader<R> {
    const fn new(reader: R, remaining: u64) -> Self {
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

#[cfg(test)]
mod tests;
