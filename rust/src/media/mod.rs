//! One value naming every media implementation a handle can be read as.
//!
//! [`Media`] is to a media encoding what [`Holder`] is to [`IOBase`]: one enum
//! holding the core's three media - Arrow IPC, plain text and CSV - as
//! variants of their own and every other medium a crate claims - Parquet,
//! Avro, XML for Analysis, a workbook - as [`Media::Registered`], so a caller
//! can hold "some media over some handle" without knowing which encoding is
//! involved until the media type says. [`Media::open`] picks the medium the
//! register ([`codec_for`]) claims under the handle's media type.
//!
//! Every variant answers the same four questions - what is the schema, what
//! are the rows, what are the batches, and what are the bytes - so choosing an
//! encoding changes the construction and nothing else.
//!
//! ```
//! use std::sync::Arc;
//!
//! use arrow_array::{Int64Array, RecordBatch};
//! use yggdryl::media::Media;
//! use yggdryl::holder::Holder;
//! use yggdryl::{IOBase, IOMedia, StructType, holder::Buffer};
//! use yggdryl::{DataType, Url};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
//!     .required_field("row");
//! let arrow_schema = schema.clone().into_arrow_schema()?;
//! let batch = RecordBatch::try_new(
//!     Arc::clone(&arrow_schema),
//!     vec![Arc::new(Int64Array::from(vec![7]))],
//! )?;
//!
//! // The name decides the encoding; nothing else in the call changes.
//! let handle = Buffer::new().with_media_type(Url::from_str("file:///trades.arrows")?.media_type());
//! let mut media = Media::open(Holder::buffer(handle))?.with_field(schema.clone());
//!
//! let options = media.record_options()?;
//! media.overwrite_arrow_reader(
//!     yggdryl::arrow::batch_reader(arrow_schema, [batch]),
//!     &options,
//! )?;
//! assert_eq!(media.read_arrow_reader(&options)?.count(), 1);
//!
//! // It is also just bytes: an Arrow IPC stream starts with its continuation
//! // marker.
//! assert_eq!(&media.read_range_bytes(0, 4)?, &[0xFF, 0xFF, 0xFF, 0xFF]);
//! # Ok(())
//! # }
//! ```

pub mod cache;
pub mod codec;
pub mod format;
mod inference;
mod magic;
pub(crate) mod merge;
pub(crate) mod options;
pub mod partition;
pub(crate) mod structured;

pub use cache::{CacheTtl, Entry, MediaCache};
pub use codec::{EXTERNAL_RANK, MediaCodec, MediaWrapper, codec_for, codec_of, codecs};
pub use format::{LocatedTable, TableFormat, format_named, formats};
pub use magic::MAGIC_PROBE_LEN;
/// The root Field name a record surface uses when none is declared.
pub const DEFAULT_ROOT_NAME: &str = "row";
/// The child name a value wraps into a struct under when none is declared.
pub const DEFAULT_VALUE_NAME: &str = "value";
/// How a partition directory spells an absent value.
pub const NULL_PARTITION: &str = "null";
pub(crate) use options::{Cadence, CommitBuffer, Shaping, WriteLimitState};
pub use options::{
    DEFAULT_COMMIT_BYTE_SIZE, DEFAULT_RECORD_BATCH_ROW_SIZE, IORecordOptions, MediumOptions,
    MediumSettings, RecordOptions, RegisteredOptions,
};

use crate::IOBase;
use crate::arrow::Result;
use crate::holder::Holder;
use crate::ipc::Ipc;
use crate::{Field, MimeType};

/// A media implementation chosen by encoding.
///
/// Construct one with [`Media::open`], which reads the handle's media type
/// through the register, or name a core variant directly when the encoding
/// is already known.
#[derive(Debug)]
pub enum Media {
    /// An Arrow IPC stream.
    Ipc(Ipc<Holder>),
    /// Plain-text rows under one retained flat configuration.
    Text(crate::text::Text<Holder>),
    /// A CSV or TSV document.
    Csv(crate::csv::Csv<Holder>),
    /// A registered medium's wrapper: Parquet, Avro, XML for Analysis, a
    /// workbook, or any medium a crate claims.
    Registered(Box<dyn MediaWrapper>),
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

    /// Bind the media implementation the medium claimed under `base` opens.
    ///
    /// # Errors
    ///
    /// Returns an error when no medium is claimed under `base`, naming the
    /// media this build implements and the crate to install.
    pub fn open_as(handle: Holder, base: &MimeType) -> Result<Self> {
        let media = crate::media::codec_for(base)?.open(handle);
        // The type asked for names the dialect, whatever the handle's own
        // name would pick.
        Ok(match media {
            Self::Csv(csv) if base == &MimeType::CSV => {
                Self::Csv(csv.with_options(crate::csv::CsvOptions::new()))
            }
            Self::Csv(csv) if base == &MimeType::TSV => {
                Self::Csv(csv.with_options(crate::csv::CsvOptions::tsv()))
            }
            media => media,
        })
    }

    /// Hold an Arrow IPC stream over a handle.
    pub fn ipc(handle: Holder) -> Self {
        Self::Ipc(Ipc::new(handle))
    }

    /// Hold plain-text record media over a handle.
    pub fn text(handle: Holder) -> Self {
        Self::Text(crate::text::Text::new(handle))
    }

    /// Hold a CSV or TSV document over a handle.
    pub fn csv(handle: Holder) -> Self {
        Self::Csv(crate::csv::Csv::new(handle))
    }

    /// The medium this media encodes: a core variant's own codec, a
    /// registered wrapper's the one it names. `medium`, not `codec`, because
    /// `codec` is the content coding every [`IOBase`] answers.
    #[must_use]
    pub fn medium(&self) -> &'static dyn MediaCodec {
        match self {
            Self::Ipc(_) => &crate::ipc::IPC_CODEC,
            Self::Text(_) => &crate::text::TEXT_CODEC,
            Self::Csv(_) => &crate::csv::CSV_CODEC,
            Self::Registered(wrapper) => wrapper.medium(),
        }
    }

    /// Return this media with an explicit canonical schema.
    #[must_use]
    pub fn with_field(self, field: Field) -> Self {
        match self {
            Self::Ipc(ipc) => Self::Ipc(ipc.with_field(field)),
            Self::Text(text) => Self::Text(text.with_field(field)),
            Self::Csv(csv) => Self::Csv(csv.with_field(field)),
            Self::Registered(wrapper) => Self::Registered(wrapper.with_field(field)),
        }
    }

    /// Borrow the byte handle this media reads and writes through.
    ///
    /// The companion of [`crate::coding::Coded::handle`] and
    /// [`crate::text::Text::handle`]: one accessor that answers what a
    /// record encoding is layered over, whichever encoding it is.
    pub fn handle(&self) -> &Holder {
        match self {
            Self::Ipc(inner) => inner.handle(),
            Self::Text(inner) => inner.handle(),
            Self::Csv(inner) => inner.handle(),
            Self::Registered(inner) => inner.handle(),
        }
    }

    /// Consume this media and return the byte handle it read and wrote
    /// through.
    ///
    /// The companion of [`crate::text::Text::into_handle`], so a caller
    /// can descend one composed layer whichever encoding is on top.
    #[must_use]
    pub fn into_handle(self) -> Holder {
        match self {
            Self::Ipc(inner) => inner.into_handle(),
            Self::Text(inner) => inner.into_handle(),
            Self::Csv(inner) => inner.into_handle(),
            Self::Registered(inner) => inner.into_handle(),
        }
    }

    /// Borrow the held implementation as a byte handle.
    pub fn as_io(&self) -> &dyn IOBase {
        match self {
            Self::Ipc(ipc) => ipc,
            Self::Text(text) => text,
            Self::Csv(csv) => csv,
            Self::Registered(wrapper) => &**wrapper,
        }
    }

    /// Borrow the held implementation as a mutable byte handle.
    pub fn as_io_mut(&mut self) -> &mut dyn IOBase {
        match self {
            Self::Ipc(ipc) => ipc,
            Self::Text(text) => text,
            Self::Csv(csv) => csv,
            Self::Registered(wrapper) => &mut **wrapper,
        }
    }

    /// Borrow the selected implementation through its media contract.
    ///
    /// This match is intentionally separate from [`Self::as_io`]. `IOBase`
    /// has `IOMedia` as a supertrait, but dispatching through the exact media
    /// trait object makes it explicit that a variant's optimized schema,
    /// dimension, projection, and write overrides are retained.
    fn as_media(&self) -> &dyn crate::IOMedia {
        match self {
            Self::Ipc(ipc) => ipc,
            Self::Text(text) => text,
            Self::Csv(csv) => csv,
            Self::Registered(wrapper) => &**wrapper,
        }
    }

    /// Mutably borrow the selected implementation through its media contract.
    fn as_media_mut(&mut self) -> &mut dyn crate::IOMedia {
        match self {
            Self::Ipc(ipc) => ipc,
            Self::Text(text) => text,
            Self::Csv(csv) => csv,
            Self::Registered(wrapper) => &mut **wrapper,
        }
    }
}

impl crate::IOMedia for Media {
    fn as_io_base(&self) -> &dyn IOBase {
        self.as_io()
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self.as_io_mut()
    }

    fn row_size(&self) -> crate::Result<u64> {
        crate::IOMedia::row_size(self.as_media())
    }

    fn column_size(&self) -> crate::Result<usize> {
        crate::IOMedia::column_size(self.as_media())
    }

    fn record_options(&self) -> crate::Result<crate::media::RecordOptions> {
        crate::IOMedia::record_options(self.as_media())
    }

    fn merge_by(&self) -> crate::Result<crate::Selector> {
        crate::IOMedia::merge_by(self.as_media())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        self.as_media().as_any()
    }

    fn read_origin_field(&self) -> crate::Result<Option<crate::Field>> {
        crate::IOMedia::read_origin_field(self.as_media())
    }

    fn read_arrow_field(
        &self,
        options: &crate::media::RecordOptions,
    ) -> crate::Result<crate::Field> {
        crate::IOMedia::read_arrow_field(self.as_media(), options)
    }

    fn read_serie(
        &self,
        options: Option<&crate::media::RecordOptions>,
    ) -> crate::Result<crate::Serie> {
        crate::IOMedia::read_serie(self.as_media(), options)
    }

    fn overwrite_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&crate::media::RecordOptions>,
    ) -> crate::Result<crate::IOResult> {
        crate::IOMedia::overwrite_serie(self.as_media_mut(), value, options)
    }

    fn overwrite_prepared_serie(
        &mut self,
        value: crate::StreamChunkedSerie,
        options: &crate::media::RecordOptions,
    ) -> crate::Result<()> {
        crate::IOMedia::overwrite_prepared_serie(self.as_media_mut(), value, options)
    }

    fn append_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&crate::media::RecordOptions>,
    ) -> crate::Result<crate::IOResult> {
        crate::IOMedia::append_serie(self.as_media_mut(), value, options)
    }

    fn merge_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&crate::media::RecordOptions>,
    ) -> crate::Result<crate::IOResult> {
        crate::IOMedia::merge_serie(self.as_media_mut(), value, options)
    }
}

/// A `Media` is the bytes it encodes, so every byte operation reaches straight
/// through to the handle underneath - through the medium, which keeps for
/// itself what its own state shapes: an upload through its invalidating
/// write, a discard and the two roles as the defaults answer them.
impl IOBase for Media {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> crate::Result<usize> {
        self.as_io().pread(offset, buffer)
    }

    fn pstream_bytes(
        &self,
        position: u64,
        batch_size: usize,
    ) -> crate::Result<crate::ByteStream<'_>> {
        self.as_io().pstream_bytes(position, batch_size)
    }

    fn read_all_bytes(&self) -> crate::Result<Vec<u8>> {
        self.as_io().read_all_bytes()
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> crate::Result<Vec<u8>> {
        self.as_io().read_range_bytes(offset, length)
    }

    fn read_tail_bytes(&self, length: usize) -> crate::Result<(Vec<u8>, u64)> {
        self.as_io().read_tail_bytes(length)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
        self.as_io_mut().pwrite(offset, bytes)
    }

    fn create_bytes(&mut self, bytes: &[u8]) -> crate::Result<()> {
        self.as_io_mut().create_bytes(bytes)
    }

    fn upload_from(&mut self, source: &mut dyn std::io::Read, length: u64) -> crate::Result<()> {
        self.as_io_mut().upload_from(source, length)
    }

    fn size(&self) -> u64 {
        self.as_io().size()
    }

    fn set_known_size(&mut self, size: u64) {
        self.as_io_mut().set_known_size(size);
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

    fn uri(&self) -> Option<&crate::Uri> {
        self.as_io().uri()
    }

    fn url(&self) -> Option<&crate::Url> {
        self.as_io().url()
    }

    fn bound_location(&self) -> Option<&crate::fs::BoundLocation> {
        self.as_io().bound_location()
    }

    fn mtime(&self) -> Option<i64> {
        self.as_io().mtime()
    }

    fn media_type(&self) -> &crate::MediaType {
        self.as_io().media_type()
    }

    fn applied_codec(&self) -> crate::Codec {
        self.as_io().applied_codec()
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

    fn opened(&self) -> bool {
        self.as_io().opened()
    }

    fn close(&mut self) -> crate::Result<()> {
        self.as_io_mut().close()
    }

    fn clear(&mut self) -> crate::Result<()> {
        self.as_io_mut().clear()
    }

    fn remove(&mut self, recursive: bool) -> crate::Result<()> {
        self.as_io_mut().remove(recursive)
    }

    fn discard(&self) -> crate::Result<bool> {
        self.as_io().discard()
    }

    fn parent(&self) -> Option<Holder> {
        self.as_io().parent()
    }

    fn child_by_path(&self, name: &str) -> crate::Result<Holder> {
        self.as_io().child_by_path(name)
    }

    fn as_leaf(&self) -> crate::Result<Option<Holder>> {
        self.as_io().as_leaf()
    }

    fn as_container(&self) -> crate::Result<Option<Holder>> {
        self.as_io().as_container()
    }

    fn ls(&self, recursive: bool, include_private: bool) -> crate::Listing {
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

impl From<crate::text::Text<Holder>> for Media {
    fn from(value: crate::text::Text<Holder>) -> Self {
        Self::Text(value)
    }
}

impl From<crate::csv::Csv<Holder>> for Media {
    fn from(value: crate::csv::Csv<Holder>) -> Self {
        Self::Csv(value)
    }
}

crate::media_serie::media_serie!(
    GenericMediaSerie,
    GenericMedia,
    as_generic_media,
    get_generic_media_mut,
    accepts = None
);
