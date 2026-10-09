//! A record encoding another crate implements, registered under its MIME
//! type so the core's own record doors reach it.
//!
//! The core's encodings are the variants of [`RecordOptions`] and
//! [`Media`](super::Media): a `.csv` composes `Csv`, its options are
//! `RecordOptions::Csv`, and the leaf doors every record read and write run
//! through reach `csv::read_batch_reader` by that variant. An encoding that
//! lives in another crate - XML for Analysis, in `yggdryl-xmla` - can be no
//! variant, so it is registered instead: once [`register`] holds its
//! [`RegisteredEncoding`], a handle whose media type names that MIME type
//! composes the encoding's own wrapper ([`RegisteredEncoding::open`]) the
//! way a `.csv` composes `Csv`, [`RecordOptions::for_mime_type`] answers
//! [`RecordOptions::Registered`] holding the encoding's defaults, and every
//! leaf door - the schema, the row count, the rows, a write, the stored
//! shape - reaches the encoding through the trait. With nothing registered,
//! the MIME type is a record encoding this build does not implement, as
//! Parquet is without its feature.
//!
//! [`RegisteredOptions`] carries what every encoding's options carry - the
//! declared field, the plan's sections, the bounds, the cadence, the
//! compression level - beside the encoding's own settings as
//! [`Properties`]: text the encoding reads once per call through its own
//! vocabulary, which is what keeps the options plain data the core clones,
//! compares, hashes and orders without knowing the encoding.
//!
//! The registry is the process's, read under a lock taken per lookup.
//! Registering an encoding again replaces the one held under its MIME type,
//! so a crate's `register()` is idempotent and nothing is unregistered.

use std::fmt;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard};

use smol_str::SmolStr;

use crate::arrow::BatchReader;
use crate::holder::Holder;
use crate::media::IORecordOptions;
use crate::{Error, Field, Filter, IOBase, Level, MimeType, Properties, Result, Selector};

/// A record encoding implemented outside the crate: the doors the core
/// routes a leaf of its MIME type through.
///
/// Each door takes the handle as `dyn IOBase` and the options as
/// [`RegisteredOptions`], the encoding's own settings read out of their
/// properties; what the core does around them - the declared field's cast,
/// the plan's sections, the bounds, the write modes and their cadence - is
/// the same for every encoding and stays the core's.
pub trait RegisteredEncoding: fmt::Debug + Send + Sync + 'static {
    /// The MIME type a handle's media type names this encoding by.
    fn mime_type(&self) -> MimeType;

    /// The options a handle of this encoding reads and writes with when a
    /// caller states none: every setting of the encoding's own spelled in
    /// the properties, so a copy edited by name starts from the default.
    fn options(&self) -> RegisteredOptions;

    /// Read the schema the handle's bytes declare.
    ///
    /// # Errors
    ///
    /// Returns what reading or decoding the handle refuses, and the absence
    /// of a schema where the bytes declare none.
    fn read_field(&self, handle: &dyn IOBase, options: &RegisteredOptions) -> Result<Field>;

    /// Count the handle's rows, from what the encoding states about them
    /// without decoding them wherever it states a count.
    ///
    /// # Errors
    ///
    /// Returns what reading or decoding the handle refuses.
    fn row_size(&self, handle: &dyn IOBase, options: &RegisteredOptions) -> Result<u64>;

    /// Read the handle's rows as batches: under `declared`, the field a
    /// caller declared, where one was, else under the stored schema. The
    /// cast onto the declared field is the core's, after this door.
    ///
    /// # Errors
    ///
    /// Returns what reading or decoding the handle refuses.
    fn read_batch_reader(
        &self,
        handle: &dyn IOBase,
        declared: Option<&Field>,
        options: &RegisteredOptions,
    ) -> crate::arrow::Result<BatchReader>;

    /// Replace the handle's bytes with `batches`, encoded whole, through the
    /// handle's own content coding.
    ///
    /// # Errors
    ///
    /// Returns what encoding or writing refuses; nothing reaches the handle
    /// before the last batch is encoded.
    fn overwrite_arrow_reader(
        &self,
        handle: &mut dyn IOBase,
        batches: BatchReader,
        options: &RegisteredOptions,
    ) -> Result<()>;

    /// The root field the handle's own bytes declare, `None` where they
    /// declare none - an empty handle, a document written without its
    /// schema - which a write completes its rows onto before it publishes.
    ///
    /// # Errors
    ///
    /// Returns what reading or decoding the handle refuses.
    fn stated_field(
        &self,
        handle: &dyn IOBase,
        options: &RegisteredOptions,
    ) -> Result<Option<Field>>;

    /// The wrapper this encoding retains over `handle`: its
    /// [`IOMedia`](crate::IOMedia) implementation, which a handle named by
    /// the MIME type composes and [`Media::Registered`](super::Media::Registered)
    /// holds.
    fn open(&self, handle: Holder) -> Box<dyn RegisteredMedia>;
}

/// The wrapper a registered encoding retains over a handle: a byte handle
/// answering records through the encoding, as `Csv<Holder>` does through
/// CSV.
pub trait RegisteredMedia: IOBase + Sync + fmt::Debug {
    /// The MIME type of the encoding this wrapper implements.
    fn encoding(&self) -> MimeType;

    /// Borrow the byte handle the rows are read and written through.
    fn handle(&self) -> &Holder;

    /// Consume the wrapper and answer the byte handle it read and wrote
    /// through.
    fn into_handle(self: Box<Self>) -> Holder;

    /// This wrapper with a declared canonical row field.
    fn with_field(self: Box<Self>, field: Field) -> Box<dyn RegisteredMedia>;
}

/// The settings a read or write of a registered encoding takes.
///
/// The shared settings are every record encoding's. What the encoding adds
/// is its [`properties`](Self::properties), one text value per name, which
/// the encoding reads through its own vocabulary at every door and refuses
/// by name where a value spells nothing it knows.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RegisteredOptions {
    mime_type: MimeType,
    /// Root Field name; the declared field's when one is declared.
    pub name: SmolStr,
    /// The declared root; `None` infers the shape.
    pub field: Option<Field>,
    /// The rows a read or write keeps.
    pub filter: Filter,
    /// The columns a read or write publishes.
    pub select: Selector,
    /// The columns forming an explicit merge's match key.
    pub merge_by: Selector,
    /// Whether a cast may null a value it cannot convert.
    pub safe: bool,
    /// Bytes per batch, whichever of this and `batch_row_size` binds first.
    pub batch_byte_size: Option<u64>,
    /// Rows per batch, when a reader should bound them.
    pub batch_row_size: Option<usize>,
    /// Most result rows in total - a count of rows, not a per-row byte cap.
    pub max_row_size: Option<u64>,
    /// Leading result rows skipped before `max_row_size` counts.
    pub row_offset: Option<u64>,
    /// Most Arrow in-memory bytes of result rows, never encoded bytes.
    pub max_byte_size: Option<u64>,
    /// Whole batches published per streamed-write commit, never rows; `None`
    /// is the destination's own cadence: a leaf, a folder and an Iceberg
    /// table publish once, after the source ends - the table holding every
    /// partition's rows under the process spill bound until then - an
    /// overwrite's first commit replacing and every later one appending
    /// while every commit of a merge merges by its key; a write session by
    /// [`DEFAULT_COMMIT_BYTE_SIZE`](crate::media::DEFAULT_COMMIT_BYTE_SIZE).
    /// The commits completed before a later failure stay published. The rule
    /// is [`IORecordOptions::commit_batch_num`]'s.
    pub commit_batch_num: Option<usize>,
    /// The threads a write of several parts runs on at once; `None` is the
    /// destination's own answer.
    pub num_threads: Option<usize>,
    /// Compression level applied when the handle declares a coding.
    pub level: Level,
    properties: Properties,
}

impl RegisteredOptions {
    /// The default shared settings for the encoding `mime_type` names, with
    /// no property of the encoding's own stated.
    #[must_use]
    pub fn new(mime_type: MimeType) -> Self {
        Self {
            mime_type,
            name: SmolStr::new_static(crate::media::DEFAULT_ROOT_NAME),
            field: None,
            filter: Filter::always_true(),
            select: Selector::all(),
            merge_by: Selector::all(),
            safe: false,
            batch_byte_size: None,
            batch_row_size: None,
            max_row_size: None,
            row_offset: None,
            max_byte_size: None,
            commit_batch_num: None,
            num_threads: None,
            level: Level::DEFAULT,
            properties: Properties::new(),
        }
    }

    /// The MIME type of the encoding these options describe.
    pub const fn mime_type(&self) -> &MimeType {
        &self.mime_type
    }

    /// Borrow the encoding's own settings, one text value per name.
    pub const fn properties(&self) -> &Properties {
        &self.properties
    }

    /// Borrow the encoding's own settings mutably.
    pub const fn properties_mut(&mut self) -> &mut Properties {
        &mut self.properties
    }

    /// Return these options with one of the encoding's own settings stated,
    /// a value already held under `name` replaced.
    #[must_use]
    pub fn with_property(mut self, name: impl Into<SmolStr>, value: impl Into<SmolStr>) -> Self {
        self.properties.set(name, value);
        self
    }

    /// The encoding registered under these options' MIME type.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] at `$.encoding` naming the MIME type
    /// when nothing in this process is registered under it.
    pub fn encoding(&self) -> Result<Arc<dyn RegisteredEncoding>> {
        registered(&self.mime_type).ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$.encoding"),
            reason: crate::text::expected_got(
                format_args!(
                    "a record encoding registered in this process for {}",
                    self.mime_type
                ),
                "none",
            ),
        })
    }
}

impl IORecordOptions for RegisteredOptions {
    crate::record_options_fields!();
}

/// The encodings registered in this process, one per MIME type.
static REGISTRY: RwLock<Vec<Arc<dyn RegisteredEncoding>>> = RwLock::new(Vec::new());

/// The registry for reading; a poisoned lock holds a list a panic left
/// whole, since nothing writes it but a push and a retain.
fn registry() -> RwLockReadGuard<'static, Vec<Arc<dyn RegisteredEncoding>>> {
    REGISTRY.read().unwrap_or_else(PoisonError::into_inner)
}

/// Register `encoding` under its MIME type, replacing the encoding held
/// under it, if any. From here on a handle whose media type names that
/// MIME type composes the encoding's wrapper and reads and writes records
/// through its doors.
pub fn register(encoding: Arc<dyn RegisteredEncoding>) {
    let mime_type = encoding.mime_type();
    let mut held = REGISTRY.write().unwrap_or_else(PoisonError::into_inner);
    held.retain(|held| held.mime_type() != mime_type);
    held.push(encoding);
}

/// The encoding registered under `mime_type`, if any.
pub fn registered(mime_type: &MimeType) -> Option<Arc<dyn RegisteredEncoding>> {
    registry()
        .iter()
        .find(|held| held.mime_type() == *mime_type)
        .cloned()
}

/// The MIME types an encoding is registered under, in registration order.
pub fn registered_mime_types() -> Vec<MimeType> {
    registry().iter().map(|held| held.mime_type()).collect()
}

/// The encodings a record door names when it refuses a MIME type: the
/// core's own under this build's features, then the registered ones.
pub(crate) fn implemented_encodings() -> String {
    let mut text = String::from("application/vnd.apache.arrow.stream");
    if cfg!(feature = "parquet") {
        text.push_str(", application/vnd.apache.parquet");
    }
    text.push_str(
        ", application/avro, text/plain, text/csv, text/tab-separated-values, \
         application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    );
    for mime_type in registered_mime_types() {
        text.push_str(", ");
        text.push_str(mime_type.as_str());
    }
    if !cfg!(feature = "parquet") {
        text.push_str("; the `parquet` feature is not enabled");
    }
    text
}
