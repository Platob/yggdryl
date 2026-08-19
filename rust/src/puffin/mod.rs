//! Puffin blob containers: statistics and deletion vectors beside a table.
//!
//! Puffin is the container format Iceberg keeps per-file information in -
//! Theta sketches, deletion vectors - as arbitrary "blobs" framed by a magic,
//! a JSON footer, and per-blob metadata. It is implemented here as a container
//! encoding beside [`crate::ipc`], [`crate::avro`], and [`crate::parquet`],
//! not as an Iceberg detail: the format needs nothing but bytes, the JSON
//! codec in [`crate::json`], the Zstandard codec in [`crate::zstd`], and the
//! CRC-32 the pinned `flate2` already exposes, so the module compiles
//! unconditionally like the Avro value codec and adds no dependency.
//!
//! [`Puffin`] is a wrapping handle first: it mirrors its handle's bytes, so
//! the file can be copied, uploaded, or handed to a foreign reader without
//! unwrapping. It is **not** a record media and does not implement the three
//! record methods - a blob container is not a row encoding. Its surface is the
//! blob one instead: [`Puffin::footer`] reads the footer lazily on first ask
//! (cached only between [`IOBase::open`] and [`IOBase::close`]),
//! [`Puffin::read_blob`] returns one blob's bytes decompressed per its codec,
//! and [`Puffin::append_blob`] stages blobs whose footer
//! [`Puffin::finish`] - or `close` - publishes. Existing blobs, Theta
//! sketches included, are preserved losslessly across an append: their bytes
//! are never rewritten and their metadata is carried into the new footer.
//! This implementation never produces a sketch of its own.
//!
//! The `deletion-vector-v1` blob content - the framing Iceberg v3 row deletes
//! use - is the pair [`write_deletion_vector`] and [`read_deletion_vector`]:
//! sorted positions to framed bytes and back, with the length, magic, CRC-32,
//! and portable 64-bit Roaring rules validated in both directions.
//!
//! ```
//! use yggdryl::io::{Buffer, IOBase};
//! use yggdryl::puffin::Puffin;
//!
//! # fn main() -> yggdryl::Result<()> {
//! let mut file = Puffin::new(Buffer::new());
//! let blob = file.append_deletion_vector("data/trades-001.parquet", &[4, 7, 1024])?;
//! file.finish()?;
//!
//! let reread = Puffin::new(file.into_handle());
//! assert_eq!(reread.footer()?.blobs.len(), 1);
//! assert_eq!(reread.read_deletion_vector(&blob)?, vec![4, 7, 1024]);
//! # Ok(())
//! # }
//! ```

mod bitmap;
mod blob;
mod format;

use std::io::Read;

use smol_str::{SmolStr, format_smolstr};

use crate::io::IOBase;
use crate::{Limits, Result};

pub use bitmap::{read_deletion_vector, read_deletion_vector_with_limits, write_deletion_vector};
pub use blob::{
    APACHE_DATASKETCHES_THETA_V1, BlobMetadata, CARDINALITY_PROPERTY, DELETION_VECTOR_V1,
    REFERENCED_DATA_FILE_PROPERTY, UNASSIGNED_SNAPSHOT,
};
pub use format::FileMetadata;

use bitmap::invalid;

/// The blob compression codec this build can write and read back.
const ZSTD_CODEC: &str = "zstd";

/// A Puffin blob container bound to one [`IOBase`] handle.
///
/// See the [module documentation](self) for the shape of the surface; the
/// handle mirrors its inner bytes and is not a record media.
#[derive(Debug)]
pub struct Puffin<H: IOBase> {
    handle: H,
    /// Footer cached by `open`, discarded by `close`, `clear`, and `remove`.
    cached_footer: Option<FileMetadata>,
    /// Blobs appended and properties set since the last published footer.
    ///
    /// Holding every blob's *metadata* is bounded by the footer document the
    /// file must carry anyway; the blobs' bytes are written as they arrive.
    pending: Option<Pending>,
}

/// The staged state between a first mutation and the footer that publishes it.
#[derive(Debug)]
struct Pending {
    metadata: FileMetadata,
    /// Where the next appended blob's bytes land: the end of the blob region.
    end: u64,
}

impl<H: IOBase> Puffin<H> {
    /// Bind a Puffin container to a handle.
    pub const fn new(handle: H) -> Self {
        Self {
            handle,
            cached_footer: None,
            pending: None,
        }
    }

    /// Borrow the underlying handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Borrow the underlying handle mutably.
    pub const fn handle_mut(&mut self) -> &mut H {
        &mut self.handle
    }

    /// Consume the container and return its handle.
    pub fn into_handle(self) -> H {
        self.handle
    }

    /// Return the file's metadata: its blobs and its properties.
    ///
    /// Uncommitted appends are included, because they are what the next
    /// [`Self::finish`] publishes. An open container answers from the cache
    /// [`IOBase::open`] filled; a closed one reads the footer fresh every
    /// time, because a cache nobody asked for is how a handle serves a stale
    /// footer after the resource changes underneath it. An empty resource
    /// reads as an empty footer - absence reads as empty, and the first
    /// append creates the file.
    ///
    /// # Errors
    ///
    /// Returns a read or footer-framing failure.
    pub fn footer(&self) -> Result<FileMetadata> {
        if let Some(pending) = &self.pending {
            return Ok(pending.metadata.clone());
        }
        if let Some(cached) = &self.cached_footer {
            return Ok(cached.clone());
        }
        if self.handle.is_empty() {
            return Ok(FileMetadata::default());
        }
        format::read_footer(&self.handle).map(|(metadata, _)| metadata)
    }

    /// Read one blob's bytes, decompressed per its `compression-codec`.
    ///
    /// An uncompressed blob is returned as stored; a `zstd` blob is decoded
    /// through [`crate::zstd`] under the default [`Limits`] byte ceiling; an
    /// `lz4` blob is refused naming the codec, because this build takes no
    /// LZ4 dependency and a skipped blob would be silent data loss.
    ///
    /// # Errors
    ///
    /// Returns a read failure, a decoding failure, or the codec refusal.
    pub fn read_blob(&self, blob: &BlobMetadata) -> Result<Vec<u8>> {
        let length = usize::try_from(blob.length()).map_err(|_| {
            invalid(format_smolstr!(
                "expected a blob length fitting this platform, got {}",
                blob.length()
            ))
        })?;
        let mut bytes = vec![0_u8; length];
        self.handle.pread_exact(blob.offset(), &mut bytes)?;
        match blob.compression_codec() {
            None => Ok(bytes),
            Some(ZSTD_CODEC) => {
                // Bounded like every other decompression: a small stored blob
                // must not be able to decode the process to death.
                let bound = Limits::default().max_input_bytes();
                let mut decoded = Vec::new();
                let mut reader = crate::zstd::reader(bytes.as_slice()).take(bound as u64 + 1);
                reader.read_to_end(&mut decoded).map_err(|error| {
                    invalid(format_smolstr!("expected a valid zstd blob, got {error}"))
                })?;
                if decoded.len() > bound {
                    return Err(invalid(format_smolstr!(
                        "expected a blob of at most {bound} decoded bytes, got more"
                    )));
                }
                Ok(decoded)
            }
            Some(other) => Err(refuse_codec(other)),
        }
    }

    /// Read one `deletion-vector-v1` blob as sorted row positions.
    ///
    /// The blob's metadata is validated first -
    /// [`BlobMetadata::validate_deletion_vector`] checks the type, the
    /// required properties, the ban on compression, and the `-1` snapshot
    /// sentinels - then the content framing is decoded, and the declared
    /// `cardinality` property must equal the number of decoded positions.
    ///
    /// # Errors
    ///
    /// Returns an error naming the first broken metadata or framing rule.
    pub fn read_deletion_vector(&self, blob: &BlobMetadata) -> Result<Vec<u64>> {
        blob.validate_deletion_vector()?;
        let cardinality = blob.cardinality()?;
        let length = usize::try_from(blob.length()).map_err(|_| {
            invalid(format_smolstr!(
                "expected a blob length fitting this platform, got {}",
                blob.length()
            ))
        })?;
        let mut bytes = vec![0_u8; length];
        self.handle.pread_exact(blob.offset(), &mut bytes)?;
        let positions = read_deletion_vector(&bytes)?;
        if positions.len() as u64 != cardinality {
            return Err(invalid(format_smolstr!(
                "expected the declared cardinality {cardinality}, got {} positions",
                positions.len()
            )));
        }
        Ok(positions)
    }

    /// Append one blob's bytes and stage its metadata for the next footer.
    ///
    /// The content is compressed per the metadata's `compression-codec` -
    /// only `zstd` is writable, `lz4` is refused by name - and written at the
    /// end of the blob region immediately; `offset` and `length` are stamped
    /// with where the bytes landed, and the stamped metadata is returned. The
    /// footer itself is published by [`Self::finish`] or [`IOBase::close`].
    /// A `deletion-vector-v1` blob is validated against the spec's metadata
    /// rules before anything is written.
    ///
    /// # Errors
    ///
    /// Returns a validation, compression, or write failure; a failure before
    /// the write stages nothing.
    pub fn append_blob(&mut self, blob: BlobMetadata, content: &[u8]) -> Result<BlobMetadata> {
        let stored: Vec<u8>;
        let stored = match blob.compression_codec() {
            None => content,
            Some(ZSTD_CODEC) => {
                stored = crate::zstd::dump(content)?;
                &stored
            }
            Some(other) => return Err(refuse_codec(other)),
        };
        if blob.blob_type() == DELETION_VECTOR_V1 {
            blob.validate_deletion_vector()?;
        }
        let offset = self.stage()?;
        self.handle.pwrite_all(offset, stored)?;
        let mut blob = blob;
        blob.set_location(offset, stored.len() as u64);
        let pending = self.staged_mut()?;
        pending.end = offset + stored.len() as u64;
        pending.metadata.blobs.push(blob.clone());
        Ok(blob)
    }

    /// Serialize sorted positions and append them as a `deletion-vector-v1` blob.
    ///
    /// The content framing comes from [`write_deletion_vector`]; the metadata
    /// carries the required `referenced-data-file` and `cardinality`
    /// properties, the `-1` snapshot sentinels, no compression, and an empty
    /// field list. The stamped metadata is returned for the manifest entry
    /// that must record the blob's offset and length.
    ///
    /// # Errors
    ///
    /// Returns an error when the positions are unordered or out of range, or
    /// when the write fails.
    pub fn append_deletion_vector(
        &mut self,
        referenced_data_file: impl Into<SmolStr>,
        positions: &[u64],
    ) -> Result<BlobMetadata> {
        let content = write_deletion_vector(positions)?;
        let blob = BlobMetadata::deletion_vector(referenced_data_file, positions.len() as u64);
        self.append_blob(blob, &content)
    }

    /// Set one file property in the footer the next [`Self::finish`] publishes.
    ///
    /// # Errors
    ///
    /// Returns a read or footer-framing failure from staging the current
    /// footer.
    pub fn set_file_property(
        &mut self,
        key: impl Into<SmolStr>,
        value: impl Into<SmolStr>,
    ) -> Result<()> {
        self.stage()?;
        let pending = self.staged_mut()?;
        let key = key.into();
        let value = value.into();
        if let Some(entry) = pending
            .metadata
            .properties
            .iter_mut()
            .find(|(name, _)| *name == key)
        {
            entry.1 = value;
        } else {
            pending.metadata.properties.push((key, value));
        }
        Ok(())
    }

    /// Publish the staged blobs and properties as the file's footer.
    ///
    /// The footer is written after the last blob, the file is truncated to
    /// exactly that end, and the handle is flushed - a finish *is* an
    /// operation, so a second handle on the same location reads the published
    /// file. Without staged changes this is a no-op. Existing blob bytes are
    /// never moved: an append preserves every prior blob, sketches included,
    /// losslessly.
    ///
    /// # Errors
    ///
    /// Returns an encoding or write failure.
    pub fn finish(&mut self) -> Result<()> {
        let Some(pending) = self.pending.take() else {
            return Ok(());
        };
        let footer = format::footer_bytes(&pending.metadata)?;
        self.handle.pwrite_all(pending.end, &footer)?;
        self.handle.truncate(pending.end + footer.len() as u64)?;
        self.handle.flush()?;
        // An open handle keeps serving from its cache, so the cache is
        // refreshed to what was just published rather than left stale.
        if self.cached_footer.is_some() {
            self.cached_footer = Some(pending.metadata);
        }
        Ok(())
    }

    /// Fill the pending state from the current footer on the first mutation,
    /// returning the end of the blob region - where the next blob lands.
    ///
    /// An empty resource gets its head magic here - creation is a consequence
    /// of the write, never a step a caller performs first.
    fn stage(&mut self) -> Result<u64> {
        if let Some(pending) = &self.pending {
            return Ok(pending.end);
        }
        let pending = if self.handle.is_empty() {
            self.handle.pwrite_all(0, &format::MAGIC)?;
            Pending {
                metadata: FileMetadata::default(),
                end: format::MAGIC.len() as u64,
            }
        } else {
            let (metadata, footer_offset) = format::read_footer(&self.handle)?;
            Pending {
                metadata,
                end: footer_offset,
            }
        };
        let end = pending.end;
        self.pending = Some(pending);
        Ok(end)
    }

    /// Borrow the staged state a preceding [`Self::stage`] guarantees.
    fn staged_mut(&mut self) -> Result<&mut Pending> {
        self.pending.as_mut().ok_or_else(|| {
            invalid(SmolStr::new_static(
                "expected staged writer state, got none",
            ))
        })
    }
}

/// Refuse a blob compression codec this build cannot decode, by name.
fn refuse_codec(codec: &str) -> crate::Error {
    invalid(format_smolstr!(
        "expected a Puffin compression codec this build implements ({ZSTD_CODEC}), got {codec:?}"
    ))
}

/// A `Puffin` mirrors the bytes of the handle it owns, so a caller can reach
/// the raw container - to copy it, upload it, or hand it to a foreign reader -
/// without unwrapping.
///
/// [`IOBase::open`] additionally caches the parsed footer and
/// [`IOBase::close`] publishes staged blobs and releases the cache.
impl<H: IOBase> IOBase for Puffin<H> {
    crate::delegate_iobase!(handle: pread, pwrite, size, capacity, reserve,
        truncate, url, media_type, set_media_type, parent, child_by_path, ls, kind);

    /// A Puffin file is one whole byte value - its blob surface is inherent,
    /// not the record surface, because a blob container is not a row encoding.
    fn is_atomic(&self) -> bool {
        true
    }

    /// A blob container holds no rows, whatever the bytes underneath carry.
    fn is_tabular(&self) -> bool {
        false
    }

    /// Materialize the handle and cache the container's footer.
    fn open(&mut self) -> Result<()> {
        self.handle.open()?;
        if self.cached_footer.is_none() && !self.handle.is_empty() {
            self.cached_footer = Some(format::read_footer(&self.handle)?.0);
        }
        Ok(())
    }

    /// Return whether a footer is currently cached.
    fn opened(&self) -> bool {
        self.cached_footer.is_some()
    }

    /// Publish staged blobs, flush the handle.
    fn flush(&mut self) -> Result<()> {
        self.finish()?;
        self.handle.flush()
    }

    /// Publish staged blobs, drop the cached footer, close the handle.
    fn close(&mut self) -> Result<()> {
        self.finish()?;
        self.cached_footer = None;
        self.handle.close()
    }

    /// Empty the resource, and the staged blobs and cached footer with it.
    ///
    /// Pending state goes as part of the call, not on the next `open`: a
    /// later [`Self::finish`] must not resurrect what was emptied.
    fn clear(&mut self) -> Result<()> {
        self.pending = None;
        self.cached_footer = None;
        self.handle.clear()
    }

    /// Delete the resource this handle wraps, and every staged write and
    /// cache it holds, so a later flush cannot resurrect it.
    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.pending = None;
        self.cached_footer = None;
        self.handle.remove(recursive)
    }
}

#[cfg(test)]
mod tests;
