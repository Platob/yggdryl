//! A handle that compresses on the way out and decompresses on the way in.
//!
//! [`Coded`] wraps any [`IOBase`] and presents the *decoded* bytes: reads
//! decompress, writes compress, and the wrapped handle only ever holds the
//! encoded form. The per-format aliases - [`crate::gzip::Gzip`],
//! [`crate::zlib::Zlib`], [`crate::zstd::Zstd`] - are this type with the codec
//! already chosen.
//!
//! A coding is not seekable, so the decoded value is materialized once and kept
//! until [`IOBase::close`]. That is what makes positional reads and writes work
//! at all over a compressed payload, and it is why the write is published on
//! [`IOBase::flush`] rather than on every `pwrite`.

use crate::io::{Holder, IOBase};
use crate::{Codec, Level, MediaType, Result, Url};

/// A transparent compression buffer over one handle.
///
/// The media type reported here is the *decoded* one: wrapping a handle named
/// `trades.arrows.gz` in a gzip coding yields a handle that says
/// `application/vnd.apache.arrow.stream`, because that is what its bytes now
/// are.
#[derive(Debug)]
pub struct Coded<H: IOBase> {
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

impl<H: IOBase> Coded<H> {
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
            let encoded = self.handle.read_all()?;
            // Per the laziness contract, a resource that does not exist yet
            // decodes to nothing rather than failing.
            let plain = if encoded.is_empty() {
                Vec::new()
            } else {
                self.codec.load(&encoded)?
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

    /// Decode without caching, for the read-only accessors.
    ///
    /// This is the path for a handle that is *not* open: nothing may be
    /// cached as a side effect of an ordinary read, so the value is decoded
    /// for this call and dropped after it.
    fn peek(&self) -> Result<Vec<u8>> {
        if let Some(plain) = &self.plain {
            return Ok(plain.clone());
        }
        let encoded = self.handle.read_all()?;
        if encoded.is_empty() {
            return Ok(Vec::new());
        }
        self.codec.load(&encoded)
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

impl<H: IOBase> IOBase for Coded<H> {
    /// Lines stream off the *encoded* handle rather than the materialized
    /// value: the coding is peeled as a streaming decoder, then whatever
    /// codings the decoded media type still carries, so a compressed resource
    /// pays one buffer instead of its decompressed size. A pending write has
    /// the decoded value in memory already, so it takes the default path.
    fn read_lines(&self) -> Result<super::Lines<Box<dyn std::io::Read + '_>>>
    where
        Self: Sized,
    {
        if self.dirty {
            let mut stream: Box<dyn std::io::Read + '_> = Box::new(self.reader_at(0));
            for coding in self.media_type().encodings().iter().rev() {
                stream = crate::Codec::from_mime_type(coding).reader(stream);
            }
            return Ok(super::Lines::over(stream));
        }
        let mut stream: Box<dyn std::io::Read + '_> = self.codec.reader(self.handle.reader_at(0));
        for coding in self.media_type().encodings().iter().rev() {
            stream = crate::Codec::from_mime_type(coding).reader(stream);
        }
        Ok(super::Lines::over(stream))
    }

    /// The borrowed projection reads a snapshot of the *decoded* value.
    ///
    /// The default reopens the handle's location, which for this decoding
    /// view holds the encoded form - not the bytes these reads present - so
    /// the projection snapshots the presented value instead, exactly what
    /// this handle materializes to serve any read.
    #[cfg(feature = "arrow")]
    fn read_arrow_lines(
        &self,
        options: &super::LineRecordOptions,
    ) -> Result<crate::arrow::BatchReader>
    where
        Self: Sized,
    {
        super::lines::snapshot_arrow_lines(self, options)
    }

    /// Read the range out of the decoded value.
    ///
    /// An open handle answers from the value it already holds; a closed one
    /// decodes for this call. Either way only the requested range is copied
    /// into the caller's buffer - a positional read over a coded handle costs
    /// the decode, never a second copy of the whole payload.
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        if let Some(plain) = self.materialized() {
            return Ok(copy_range(plain, offset, buffer));
        }
        Ok(copy_range(&self.peek()?, offset, buffer))
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let offset = usize::try_from(offset).map_err(|_| {
            crate::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("expected an offset within a decoded buffer, got {offset}"),
            ))
        })?;
        let plain = self.decoded()?;
        let end = offset + bytes.len();
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
        self.peek().map_or(0, |plain| plain.len() as u64)
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

    fn is_open(&self) -> bool {
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

    fn child_by(&self, name: &str) -> Result<Holder> {
        self.handle.child_by(name)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Result<Vec<Holder>> {
        self.handle.ls(recursive, include_private)
    }
}

#[cfg(test)]
mod tests;
