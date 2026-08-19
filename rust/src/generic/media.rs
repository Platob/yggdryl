//! One value naming every media implementation in the core.
//!
//! [`Media`] is to a media encoding what [`Holder`] is to [`IOBase`]: a concrete
//! enum over the implementations the core ships, so a caller can hold "some
//! media over some handle" without a trait object and without knowing which
//! encoding is involved until the media type says.
//!
//! Every variant answers the same four questions - what is the schema, what
//! are the rows, what are the batches, and what are the bytes - so choosing an
//! encoding changes the construction and nothing else.
//!
//! ```
//! use std::sync::Arc;
//!
//! use arrow_array::{Int64Array, RecordBatch};
//! use yggdryl::generic::{Holder, Media};
//! use yggdryl::io::{Buffer, IOBase};
//! use yggdryl::{DataType, Url};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
//!     .required_field("row");
//! let arrow_schema = schema.to_arrow_schema()?;
//! let batch = RecordBatch::try_new(
//!     Arc::clone(&arrow_schema),
//!     vec![Arc::new(Int64Array::from(vec![7]))],
//! )?;
//!
//! // The name decides the encoding; nothing else in the call changes.
//! let handle = Buffer::new().with_media_type(Url::from_str("file:///trades.arrows")?.media_type());
//! let mut media = Media::open(Holder::buffer(handle))?.with_schema(schema.clone());
//!
//! media.write_batch_reader(yggdryl::arrow::batch_reader(arrow_schema, [batch]))?;
//! assert_eq!(media.read_batch_reader(None)?.count(), 1);
//!
//! // It is also just bytes: an Arrow IPC stream starts with its continuation
//! // marker.
//! assert_eq!(&media.read_range(0, 4)?, &[0xFF, 0xFF, 0xFF, 0xFF]);
//! # Ok(())
//! # }
//! ```

use super::Holder;
use crate::arrow::{Error, Result};
use crate::io::IOBase;
use crate::ipc::Ipc;
use crate::{Field, MimeType};

/// A media implementation chosen by encoding.
///
/// Construct one with [`Media::open`], which reads the handle's media type, or
/// name a variant directly when the encoding is already known.
#[derive(Debug)]
pub enum Media {
    /// An Arrow IPC stream.
    Ipc(Ipc<Holder>),
    /// An Apache Parquet file.
    #[cfg(feature = "parquet")]
    Parquet(crate::parquet::Parquet<Holder>),
    /// An Apache Avro object container.
    Avro(crate::avro::Avro<Holder>),
}

impl Media {
    /// Bind the media implementation the handle's media type names.
    ///
    /// Nothing is read: the decision comes from the declared media type, which
    /// a handle derives from its name or its content. An encoding with no
    /// implementation in this build is reported rather than guessed at.
    ///
    /// # Errors
    ///
    /// Returns an error when no media implementation covers the handle's media
    /// type, naming the type that was found.
    pub fn open(handle: Holder) -> Result<Self> {
        let base = handle.media_type().base().clone();
        Self::open_as(handle, &base)
    }

    /// Bind the media implementation for an explicit MIME type.
    ///
    /// # Errors
    ///
    /// Returns an error when no media implementation covers `base`.
    pub fn open_as(handle: Holder, base: &MimeType) -> Result<Self> {
        if base == &MimeType::ARROW_STREAM || base == &MimeType::ARROW_FILE {
            return Ok(Self::Ipc(Ipc::new(handle)));
        }
        #[cfg(feature = "parquet")]
        if base == &MimeType::PARQUET {
            return Ok(Self::Parquet(crate::parquet::Parquet::new(handle)));
        }
        if base == &MimeType::AVRO {
            return Ok(Self::Avro(crate::avro::Avro::new(handle)));
        }
        Err(Error::IncompatibleSchema(format!(
            "expected a media type with an implementation in this build \
             (application/vnd.apache.arrow.stream{}, application/avro), got {base}",
            if cfg!(feature = "parquet") {
                ", application/vnd.apache.parquet"
            } else {
                "; the `parquet` feature is not enabled"
            }
        )))
    }

    /// Hold an Arrow IPC stream over a handle.
    pub fn ipc(handle: Holder) -> Self {
        Self::Ipc(Ipc::new(handle))
    }

    /// Hold a Parquet file over a handle.
    #[cfg(feature = "parquet")]
    pub fn parquet(handle: Holder) -> Self {
        Self::Parquet(crate::parquet::Parquet::new(handle))
    }

    /// Hold an Avro object container over a handle.
    pub fn avro(handle: Holder) -> Self {
        Self::Avro(crate::avro::Avro::new(handle))
    }

    /// Return this media with an explicit canonical schema.
    #[must_use]
    pub fn with_schema(self, schema: Field) -> Self {
        match self {
            Self::Ipc(ipc) => Self::Ipc(ipc.with_schema(schema)),
            #[cfg(feature = "parquet")]
            Self::Parquet(parquet) => Self::Parquet(parquet.with_schema(schema)),
            Self::Avro(avro) => Self::Avro(avro.with_schema(schema)),
        }
    }

    /// Return the canonical non-null Struct root Field of this media.
    ///
    /// # Errors
    ///
    /// Returns a read, decoding, or schema-projection failure.
    pub fn schema(&mut self) -> Result<Field> {
        match self {
            Self::Ipc(ipc) => ipc.schema(),
            #[cfg(feature = "parquet")]
            Self::Parquet(parquet) => parquet.schema(),
            Self::Avro(avro) => avro.schema(),
        }
    }

    /// Read the media, keeping only the columns `field` names.
    ///
    /// A `field` naming a subset of the stored columns is pushed into whichever
    /// encoding is held, so choosing the encoding still changes nothing but the
    /// construction.
    ///
    /// # Errors
    ///
    /// Returns a read or decoding failure.
    pub fn read_batch_reader(&self, field: Option<&Field>) -> Result<crate::arrow::BatchReader> {
        match self {
            Self::Ipc(ipc) => ipc.read_batch_reader(field),
            #[cfg(feature = "parquet")]
            Self::Parquet(parquet) => parquet.read_batch_reader(field),
            Self::Avro(avro) => avro.read_batch_reader(field),
        }
    }

    /// Replace the media's contents with every batch `batches` yields.
    ///
    /// # Errors
    ///
    /// Returns a schema, encoding, or write failure.
    pub fn write_batch_reader(&mut self, batches: crate::arrow::BatchReader) -> Result<()> {
        match self {
            Self::Ipc(ipc) => ipc.write_batch_reader(batches),
            #[cfg(feature = "parquet")]
            Self::Parquet(parquet) => parquet.write_batch_reader(batches),
            Self::Avro(avro) => avro.write_batch_reader(batches),
        }
    }

    /// Borrow the held implementation as a byte handle.
    pub fn as_io(&self) -> &dyn IOBase {
        match self {
            Self::Ipc(ipc) => ipc,
            #[cfg(feature = "parquet")]
            Self::Parquet(parquet) => parquet,
            Self::Avro(avro) => avro,
        }
    }

    /// Borrow the held implementation as a mutable byte handle.
    pub fn as_io_mut(&mut self) -> &mut dyn IOBase {
        match self {
            Self::Ipc(ipc) => ipc,
            #[cfg(feature = "parquet")]
            Self::Parquet(parquet) => parquet,
            Self::Avro(avro) => avro,
        }
    }
}

/// A `Media` is the bytes it encodes, so every byte operation reaches straight
/// through to the handle underneath.
impl IOBase for Media {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> crate::Result<usize> {
        self.as_io().pread(offset, buffer)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
        self.as_io_mut().pwrite(offset, bytes)
    }

    fn size(&self) -> u64 {
        self.as_io().size()
    }

    fn capacity(&self) -> u64 {
        self.as_io().capacity()
    }

    fn reserve(&mut self, capacity: u64) -> crate::Result<()> {
        self.as_io_mut().reserve(capacity)
    }

    fn truncate(&mut self, size: u64) -> crate::Result<()> {
        self.as_io_mut().truncate(size)
    }

    fn url(&self) -> Option<&crate::Url> {
        self.as_io().url()
    }

    fn media_type(&self) -> &crate::MediaType {
        self.as_io().media_type()
    }

    fn set_media_type(&mut self, media_type: crate::MediaType) {
        self.as_io_mut().set_media_type(media_type);
    }

    fn flush(&mut self) -> crate::Result<()> {
        self.as_io_mut().flush()
    }

    fn open(&mut self) -> crate::Result<()> {
        self.as_io_mut().open()
    }

    fn is_open(&self) -> bool {
        self.as_io().is_open()
    }

    fn close(&mut self) -> crate::Result<()> {
        self.as_io_mut().close()
    }

    fn parent(&self) -> Option<Holder> {
        self.as_io().parent()
    }

    fn child_by(&self, name: &str) -> crate::Result<Holder> {
        self.as_io().child_by(name)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> crate::Result<Vec<Holder>> {
        self.as_io().ls(recursive, include_private)
    }

    fn kind(&self) -> crate::IOKind {
        self.as_io().kind()
    }

    fn is_atomic(&self) -> bool {
        self.as_io().is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.as_io().is_tabular()
    }
}

impl From<Ipc<Holder>> for Media {
    fn from(value: Ipc<Holder>) -> Self {
        Self::Ipc(value)
    }
}

#[cfg(feature = "parquet")]
impl From<crate::parquet::Parquet<Holder>> for Media {
    fn from(value: crate::parquet::Parquet<Holder>) -> Self {
        Self::Parquet(value)
    }
}

impl From<crate::avro::Avro<Holder>> for Media {
    fn from(value: crate::avro::Avro<Holder>) -> Self {
        Self::Avro(value)
    }
}

#[cfg(test)]
mod tests;
