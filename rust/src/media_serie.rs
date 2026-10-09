//! Series retaining their medium and the scan clauses their own verbs
//! stated, until rows are requested. The medium holds the options - it
//! states them, or infers them from what it is and defaults the rest - and
//! a serie keeps no copy: every read asks the medium and lays its own
//! clauses over the answer.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::expression::{IntoFilter, IntoSelector};
use crate::media::{IORecordOptions, RecordOptions};
use crate::{
    Field, Filter, IOMedia, Result, Selector, Serie, SerieValue, StreamChunkedSerie, StreamSerie,
};

type Read<T> = fn(&T, &RecordOptions) -> Result<Serie>;

/// What a media serie's own verbs stated over its medium's options, and
/// nothing the medium answered: the predicate `with_filter` and `with_key`
/// conjoined, the selection `with_select` restated, the row range
/// `with_row_range` set. A clause left unstated is the medium's own.
#[derive(Clone, Debug, Default)]
pub(crate) struct Scan {
    filter: Filter,
    select: Option<Selector>,
    range: Option<(u64, Option<u64>)>,
}

impl Scan {
    /// Lay the stated clauses over the medium's options: the predicate
    /// conjoined with the medium's, the selection and the range replacing
    /// them where stated.
    fn lay(&self, options: &mut RecordOptions) {
        if !self.filter.is_always_true() {
            options.set_filter(options.filter().clone().and(self.filter.clone()));
        }
        if let Some(select) = &self.select {
            options.set_select(select.clone());
        }
        if let Some((offset, length)) = self.range {
            options.set_row_offset(Some(offset));
            options.set_max_row_size(length);
        }
    }
}

/// Shared storage, the scan clauses this serie stated and its lazily
/// retained rows. The fields are private so a scan cannot change after its
/// first pull; the options are the medium's, read through it on every read.
pub struct MediaSerieState<T: IOMedia + Send + 'static> {
    pub(crate) media: Arc<Mutex<T>>,
    pub(crate) source_field: Arc<Field>,
    pub(crate) field: Arc<Field>,
    pub(crate) scan: Scan,
    pub(crate) read: Read<T>,
    pub(crate) edited: Option<Serie>,
    pub(crate) rows: Arc<OnceLock<Serie>>,
}

impl<T: IOMedia + Send + 'static> Clone for MediaSerieState<T> {
    fn clone(&self) -> Self {
        Self {
            media: Arc::clone(&self.media),
            source_field: Arc::clone(&self.source_field),
            field: Arc::clone(&self.field),
            scan: self.scan.clone(),
            read: self.read,
            edited: self.edited.clone(),
            rows: Arc::clone(&self.rows),
        }
    }
}

impl<T: IOMedia + Send + 'static> fmt::Debug for MediaSerieState<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MediaSerieState")
            .field("field", &self.field)
            .field("scan", &self.scan)
            .field("edited", &self.edited.is_some())
            .field("rows", &self.rows.get())
            .finish()
    }
}

impl<T: IOMedia + Send + 'static> MediaSerieState<T> {
    /// Retain a medium under its own options and bind its scan without
    /// decoding result rows: one `record_options` and one `read_arrow_field`
    /// of the medium, nothing of the store a wrapper already answers for.
    ///
    /// # Errors
    /// Schema, scan and limit refusals.
    pub fn new(media: T) -> Result<Self> {
        Self::with_reader(
            media,
            |media, options| media.read_serie(Some(options)),
            |_| Ok(()),
        )
    }

    /// `require` judges the options the medium answers before the schema is
    /// read, so a leaf refuses a medium of another encoding on the one
    /// `record_options` the construction costs.
    pub(crate) fn with_reader(
        media: T,
        read: Read<T>,
        require: fn(&RecordOptions) -> Result<()>,
    ) -> Result<Self> {
        let options = media.record_options()?;
        require(&options)?;
        options.require_write_limits()?;
        let source = crate::iomedia::dimensions(options.clone());
        let source_field = Arc::new(media.read_arrow_field(&source)?);
        let field = Arc::new(scan_field(&source_field, &options)?);
        Ok(Self {
            media: Arc::new(Mutex::new(media)),
            source_field,
            field,
            scan: Scan::default(),
            read,
            edited: None,
            rows: Arc::new(OnceLock::new()),
        })
    }

    pub(crate) fn media(&self) -> MutexGuard<'_, T> {
        self.media
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The options a read runs under: the medium's own - an edited snapshot's
    /// with the clauses the snapshot already answered taken off and its field
    /// declared - with `scan` laid over them.
    fn compose(&self, media: &T, scan: &Scan) -> Result<RecordOptions> {
        let mut options = media.record_options()?;
        if self.edited.is_some() {
            options = crate::iomedia::dimensions(options);
            options.set_field((*self.source_field).clone());
        }
        scan.lay(&mut options);
        options.require_write_limits()?;
        Ok(options)
    }

    fn read(&self) -> Result<Serie> {
        let media = self.media();
        let options = self.compose(&media, &self.scan)?;
        match &self.edited {
            Some(rows) => options
                .apply_stream(rows.clone().into_stream()?)
                .map(Serie::from),
            None => (self.read)(&media, &options),
        }
    }

    pub(crate) fn rows(&self) -> &Serie {
        self.rows.get_or_init(|| {
            self.read().unwrap_or_else(|error| {
                Serie::from(StreamSerie::from_rows(
                    (*self.field).clone(),
                    std::iter::once(Err(error)),
                ))
            })
        })
    }

    pub(crate) fn into_rows(self) -> Result<Serie> {
        match self.rows.get() {
            Some(rows) => Ok(rows.clone()),
            None => self.read(),
        }
    }

    /// The same medium under `scan`, the rows to be pulled afresh and the
    /// field still the one the clauses before it bound: nothing is asked
    /// of the medium until a leaf's `from_media_state` binds the field.
    fn with_scan(&self, scan: Scan) -> Self {
        Self {
            scan,
            rows: Arc::new(OnceLock::new()),
            ..self.clone()
        }
    }

    /// Bind the field under this scan and install the leaf's reader, on
    /// one ask of the medium that `require` judges first: what every
    /// leaf's `from_media_state` does, so a re-plan - `with_scan` then
    /// this - asks the medium once and the public door refuses a medium of
    /// another encoding on that same ask. A refusal leaves nothing behind.
    pub(crate) fn bound(
        mut self,
        read: Read<T>,
        require: fn(&RecordOptions) -> Result<()>,
    ) -> Result<Self> {
        let field = {
            let media = self.media();
            let options = self.compose(&media, &self.scan)?;
            require(&options)?;
            scan_field(&self.source_field, &options)?
        };
        self.field = Arc::new(field);
        self.read = read;
        Ok(self)
    }

    pub(crate) fn splice(
        &mut self,
        range: std::ops::Range<usize>,
        values: Vec<crate::Scalar>,
    ) -> Result<()> {
        let rows = self.rows();
        rows.raise_held()?;
        let mut edited = rows.held_leaf().clone();
        edited.splice(range, values)?;
        let field = Arc::clone(edited.field_ref().expect("media rows have a field"));
        self.source_field = Arc::clone(&field);
        self.field = field;
        self.scan = Scan::default();
        self.rows = Arc::new(OnceLock::from(edited.clone()));
        self.edited = Some(edited);
        Ok(())
    }

    /// A slice of the held rows as an edited snapshot: the clauses the rows
    /// already answered are stated no more.
    pub(crate) fn sliced(&self, rows: Serie) -> Self {
        let field = Arc::clone(rows.field_ref().expect("media rows have a field"));
        Self {
            source_field: Arc::clone(&field),
            field,
            scan: Scan::default(),
            rows: Arc::new(OnceLock::from(rows.clone())),
            edited: Some(rows),
            ..self.clone()
        }
    }

    pub(crate) fn held_memory_size(&self) -> usize {
        self.rows.get().map_or(0, Serie::memory_size)
    }

    pub(crate) fn held_resident_size(&self) -> usize {
        self.rows.get().map_or(0, Serie::resident_size)
    }

    pub(crate) fn is_held(&self) -> bool {
        self.rows.get().is_some_and(Serie::is_held)
    }

    pub(crate) fn scalar(&self, index: usize) -> Result<crate::Scalar> {
        if let Some(rows) = self.rows.get() {
            return rows.scalar(index);
        }
        let seek = {
            let media = self.media();
            let options = self.compose(&media, &self.scan)?;
            // A stated Arrow-byte limit is global and cannot restart at a seek.
            if options.max_byte_size().is_some()
                || options
                    .max_row_size()
                    .is_some_and(|length| index as u64 >= length)
            {
                None
            } else {
                let offset = options
                    .row_offset()
                    .unwrap_or(0)
                    .checked_add(index as u64)
                    .ok_or_else(|| crate::Error::InvalidRecord {
                        path: self.field.name().into(),
                        reason: "the row offset exceeds uint64".into(),
                    })?;
                let mut seek = options;
                seek.set_row_offset(Some(offset));
                seek.set_max_row_size(Some(1));
                seek.require_write_limits()?;
                Some((self.read)(&media, &seek)?)
            }
        };
        let Some(rows) = seek else {
            return self.rows().scalar(index);
        };
        let mut rows = rows.into_stream()?;
        match rows.next() {
            Some(row) => row,
            None => {
                crate::serie::require_row(self.field.name(), index, self.rows().len())?;
                unreachable!("an existing result row is yielded by its native range")
            }
        }
    }
}

fn scan_field(source: &Field, options: &RecordOptions) -> Result<Field> {
    let selected = options.select().apply_field(source)?;
    let (early, late) = crate::expression::filter_phases(
        options.filter(),
        options.select(),
        source.fields().iter().map(|field| field.name()),
    );
    if !early.is_always_true() {
        early.bind(source)?;
    }
    if !late.is_always_true() {
        late.bind(&selected)?;
    }
    StreamChunkedSerie::root_of(&selected)
}

/// Defaults for a series whose rows belong to an [`IOMedia`].
/// Scan clauses reach the medium's native options at every read: Parquet
/// and Iceberg prune metadata, Avro skips unselected fields, and row codecs
/// read rows.
pub trait MediaSerieValue<T: IOMedia + Send + 'static>: SerieValue {
    /// The retained medium and immutable scan state.
    fn media_state(&self) -> &MediaSerieState<T>;
    /// Build this specialized series from its common scan state: one ask
    /// of the medium, judging its encoding and binding the scan's field
    /// under this leaf's rule. Every re-plan crosses it, so a re-plan
    /// asks the medium once.
    ///
    /// # Errors
    /// A medium of another encoding, or a clause its field refuses.
    fn from_media_state(state: MediaSerieState<T>) -> Result<Self>;

    /// Validate the medium's options before a source can be opened.
    ///
    /// # Errors
    /// A configuration belonging to another encoding.
    fn require_media_options(_options: &RecordOptions) -> Result<()> {
        Ok(())
    }

    /// Read in the medium's native representation, without applying bounds
    /// belonging to a later conversion. Row media override this door.
    ///
    /// # Errors
    /// The medium's read and scan refusals.
    fn read_native(media: &T, options: &RecordOptions) -> Result<Serie> {
        media.read_serie(Some(options))
    }

    /// Conjoin a source predicate, retaining it for native pruning.
    ///
    /// # Errors
    /// Parse and binding failures before a row is pulled.
    fn with_filter(self, filter: impl IntoFilter) -> Result<Self> {
        let mut scan = self.media_state().scan.clone();
        scan.filter = scan.filter.and(filter.into_filter()?);
        Self::from_media_state(self.media_state().with_scan(scan))
    }

    /// Choose source expressions before decoding their values.
    ///
    /// # Errors
    /// Parse and binding failures before a row is pulled.
    fn with_select(self, select: impl IntoSelector) -> Result<Self> {
        let mut scan = self.media_state().scan.clone();
        scan.select = Some(select.into_selector()?);
        Self::from_media_state(self.media_state().with_scan(scan))
    }

    /// Bound result rows in the native scan, after filtering and selection.
    ///
    /// # Errors
    /// An invalid combination with a merge key.
    fn with_row_range(self, offset: u64, length: Option<u64>) -> Result<Self> {
        let mut scan = self.media_state().scan.clone();
        scan.range = Some((offset, length));
        Self::from_media_state(self.media_state().with_scan(scan))
    }

    /// Retain an exact key predicate in the native scan, including null cells
    /// and casts. Aliases name the key record; source terms drive pruning.
    ///
    /// # Errors
    /// Invalid key, key value or source predicate before pulling rows.
    fn with_key(self, by: impl IntoSelector, key: crate::Scalar) -> Result<Self> {
        let by = by.into_selector()?;
        let root = &self.media_state().source_field;
        let bound = by.bind_key(root, root.name(), "prune by")?;
        let key = bound.output().scalar(key)?;
        let cells = key.sequence_rows().expect("a key value is a record");
        let predicates =
            by.expanded(root)
                .into_iter()
                .zip(cells.iter())
                .map(|(projection, value)| {
                    let term = match projection.dtype() {
                        Some(dtype) => projection.term().clone().cast(dtype.clone()),
                        None => projection.term().clone(),
                    };
                    crate::Filter::new(if value.is_null() {
                        term.is_null()
                    } else {
                        term.eq(crate::expression::Term::literal(value.clone()))
                    })
                });
        let filter = crate::Filter::all(predicates);
        self.with_filter(filter)
    }

    /// Stream only the selected key cells, leaving unused payload columns
    /// to the medium's projection plan.
    ///
    /// # Errors
    /// The selector's and read's refusals.
    fn key_values(self, by: impl IntoSelector) -> Result<StreamSerie> {
        self.with_select(by)?.into_stream()
    }

    /// Read native rows lazily. Row codecs allocate no intermediate batch.
    ///
    /// # Errors
    /// The medium's read refusals, followed by source errors while pulling.
    fn into_stream(self) -> Result<StreamSerie> {
        self.media_state().clone().into_rows()?.into_stream()
    }

    /// Convert under the same bounds every serie kind uses.
    ///
    /// # Errors
    /// The medium's read refusals and the conversion's bounds.
    fn into_chunked_stream(
        self,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> crate::arrow::Result<StreamChunkedSerie> {
        self.media_state()
            .clone()
            .into_rows()?
            .into_chunked_stream(row_size, byte_size)
    }

    /// Cluster native chunks as lazy adjacent windows.
    ///
    /// # Errors
    /// Key binding or read failure; later failures are yielded once.
    fn window_by(
        self,
        by: impl crate::IntoKeyBy,
        sorted: bool,
    ) -> crate::arrow::Result<crate::StreamKeySerie> {
        let rows = self.media_state().clone().into_rows()?;
        match rows {
            Serie::Stream(stream) => {
                crate::SharedStream::into_stream(stream)?.window_by(by, sorted)
            }
            chunks => StreamChunkedSerie::from_serie(chunks)?.window_by(by, sorted),
        }
    }

    /// Cluster native chunks with bounded open partitions.
    ///
    /// # Errors
    /// Key binding or read failure; later failures are yielded once.
    fn partition_by(
        self,
        by: impl crate::IntoKeyBy,
        options: crate::PartitionOptions,
    ) -> crate::arrow::Result<crate::StreamKeySerie> {
        let rows = self.media_state().clone().into_rows()?;
        match rows {
            Serie::Stream(stream) => {
                crate::SharedStream::into_stream(stream)?.partition_by(by, options)
            }
            chunks => StreamChunkedSerie::from_serie(chunks)?.partition_by(by, options),
        }
    }

    /// Publish any serie through the medium's explicit write door, under the
    /// medium's own options where none are given.
    /// A row mutation edits this series' held snapshot until this is called.
    ///
    /// # Errors
    /// The medium's encoding, write and commit refusals.
    fn write_serie(
        &self,
        value: Serie,
        mode: crate::IOMode,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        let resolved = {
            let media = self.media_state().media();
            let document =
                crate::text::Format::from_media_type(media.as_io_base().media_type()).is_ok();
            if document {
                crate::iomedia::require_document_mode(mode)?;
                options.cloned()
            } else {
                crate::iomedia::require_serie_write_mode(mode)?;
                let options = crate::iomedia::own_options(&*media, options)?.into_owned();
                Self::require_media_options(&options)?;
                options.require_write_mode(mode)?;
                options.require_commit_batch_num()?;
                options.require_num_threads()?;
                options.require_write_limits()?;
                Some(options)
            }
        };
        // Native readers own their source after opening. The target guard is
        // acquired afterwards, so writing this same medium cannot lock itself.
        let value = Serie::from(StreamChunkedSerie::from_serie(value)?);
        self.media_state()
            .media()
            .write_serie(value, mode, resolved.as_ref())
    }
}

/// Refuse options of a medium the `kind` leaf does not read: `accepts` the
/// MIME types it reads, `None` every type.
pub(crate) fn require_kind(
    options: &RecordOptions,
    kind: &str,
    accepts: Option<&[crate::MimeType]>,
) -> Result<()> {
    if accepts.is_none_or(|types| types.contains(&options.mime_type())) {
        return Ok(());
    }
    Err(crate::Error::InvalidRecord {
        path: "$.encoding".into(),
        reason: smol_str::format_smolstr!(
            "expected {kind} serie options, got {}",
            options.mime_type()
        ),
    })
}

/// Define the media's leaf in its own module; defaults have one owner here.
macro_rules! media_serie {
    ($name:ident, $variant:ident, $access:ident, $access_mut:ident, accepts = $accepts:expr) => {
        /// A lazy series over this medium, reading under the medium's own
        /// options and the clauses its verbs state.
        #[derive(Clone, Debug)]
        pub struct $name {
            state: $crate::MediaSerieState<Box<dyn $crate::IOBase>>,
        }
        impl $name {
            /// Retain a media handle under its own options; discover its
            /// schema without decoding result batches. A medium of another
            /// encoding is refused before its schema is read.
            ///
            /// # Errors
            /// Schema, encoding and scan binding refusals.
            pub fn new<H: $crate::IOBase + 'static>(media: H) -> $crate::Result<Self> {
                let media: Box<dyn $crate::IOBase> = Box::new(media);
                Ok(Self { state: $crate::media_serie::MediaSerieState::with_reader(media, <Self as $crate::MediaSerieValue<Box<dyn $crate::IOBase>>>::read_native, <Self as $crate::MediaSerieValue<Box<dyn $crate::IOBase>>>::require_media_options)? })
            }
        }
        impl $crate::MediaSerieValue<Box<dyn $crate::IOBase>> for $name {
            fn media_state(&self) -> &$crate::MediaSerieState<Box<dyn $crate::IOBase>> { &self.state }
            fn from_media_state(state: $crate::MediaSerieState<Box<dyn $crate::IOBase>>) -> $crate::Result<Self> {
                state.bound(<Self as $crate::MediaSerieValue<Box<dyn $crate::IOBase>>>::read_native, <Self as $crate::MediaSerieValue<Box<dyn $crate::IOBase>>>::require_media_options).map(|state| Self { state })
            }
            fn require_media_options(options: &$crate::media::RecordOptions) -> $crate::Result<()> { $crate::media_serie::require_kind(options, stringify!($variant), $accepts) }
        }
        $crate::serie::serie_leaf!($name);
        impl From<$name> for $crate::Serie {
            fn from(value: $name) -> Self { $crate::SerieValue::into_serie(value) }
        }
        impl $crate::Serie {
            /// Borrow the specialized media scan, without opening it.
            pub fn $access(&self) -> Option<&$name> { match self { Self::$variant(value) => Some(value), _ => None } }
            /// Edit the specialized scan's snapshot through copy on write.
            pub fn $access_mut(&mut self) -> Option<&mut $name> { match self { Self::$variant(value) => Some(::std::sync::Arc::make_mut(value)), _ => None } }
        }
        impl $crate::SerieValue for $name {
            fn field(&self) -> &$crate::Field { &self.state.field }
            fn field_ref(&self) -> &::std::sync::Arc<$crate::Field> { &self.state.field }
            fn len(&self) -> usize { self.state.rows().len() }
            fn null_count(&self) -> usize { self.state.rows().null_count() }
            fn is_null(&self, index: usize) -> $crate::Result<bool> { self.state.scalar(index).map(|row| row.is_null()) }
            fn scalar(&self, index: usize) -> $crate::Result<$crate::Scalar> { self.state.scalar(index) }
            fn slice(&self, offset: usize, length: usize) -> $crate::Result<Self> {
                let rows = self.state.rows();
                rows.raise_held_rows()?;
                Ok(Self { state: self.state.sliced(rows.slice(offset, length)?) })
            }
            fn splice(&mut self, range: ::std::ops::Range<usize>, rows: Vec<$crate::Scalar>) -> $crate::Result<()> { self.state.splice(range, rows) }
            fn into_arrow_array(&self) -> ::arrow_array::ArrayRef { self.state.rows().into_arrow_array().expect("media rows have a field") }
            fn memory_size(&self) -> usize { self.state.held_memory_size() }
            fn resident_size(&self) -> usize { self.state.held_resident_size() }
            fn into_serie(self) -> $crate::Serie { $crate::Serie::$variant(::std::sync::Arc::new(self)) }
            fn from_serie(value: &$crate::Serie) -> Option<&Self> {
                match value { $crate::Serie::$variant(value) => Some(value), _ => None }
            }
        }
    };
}
pub(crate) use media_serie;

impl Serie {
    /// The shared scan state of a specialized medium, without pulling rows.
    pub(crate) fn media_state(&self) -> Option<&MediaSerieState<Box<dyn crate::IOBase>>> {
        match self {
            Self::Ipc(value) => Some(value.media_state()),
            Self::Csv(value) => Some(value.media_state()),
            Self::Text(value) => Some(value.media_state()),
            Self::WarehouseTable(value) => Some(value.media_state()),
            #[cfg(feature = "http")]
            Self::Http(value) => Some(value.media_state()),
            Self::GenericMedia(value) => Some(value.media_state()),
            _ => None,
        }
    }

    pub(crate) fn splice_media(
        &mut self,
        range: std::ops::Range<usize>,
        rows: Vec<crate::Scalar>,
    ) -> Result<()> {
        match self {
            Self::Ipc(value) => Arc::make_mut(value).splice(range, rows),
            Self::Csv(value) => Arc::make_mut(value).splice(range, rows),
            Self::Text(value) => Arc::make_mut(value).splice(range, rows),
            Self::WarehouseTable(value) => Arc::make_mut(value).splice(range, rows),
            #[cfg(feature = "http")]
            Self::Http(value) => Arc::make_mut(value).splice(range, rows),
            Self::GenericMedia(value) => Arc::make_mut(value).splice(range, rows),
            _ => unreachable!("only specialized media have a scan state"),
        }
    }
}
