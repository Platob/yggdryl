//! Series retaining their medium and scan clauses until rows are requested.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::expression::{IntoFilter, IntoSelector};
use crate::media::{IORecordOptions, RecordOptions};
use crate::{Field, IOMedia, Result, Serie, SerieValue, StreamChunkedSerie, StreamSerie};

type Read<T> = fn(&T, &RecordOptions) -> Result<Serie>;

/// Shared storage, one scan configuration and its lazily retained rows.
/// The fields are private so a scan cannot change after its first pull.
pub struct MediaSerieState<T: IOMedia + Send + 'static> {
    pub(crate) media: Arc<Mutex<T>>,
    pub(crate) source_field: Arc<Field>,
    pub(crate) field: Arc<Field>,
    pub(crate) options: RecordOptions,
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
            options: self.options.clone(),
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
            .field("options", &self.options)
            .field("edited", &self.edited.is_some())
            .field("rows", &self.rows.get())
            .finish()
    }
}

impl<T: IOMedia + Send + 'static> MediaSerieState<T> {
    /// Retain a medium and bind its scan without decoding result rows.
    ///
    /// # Errors
    /// Schema, scan and limit refusals.
    pub fn new(media: T, options: Option<RecordOptions>) -> Result<Self> {
        Self::with_reader(media, options, |media, options| {
            media.read_serie(Some(options))
        })
    }

    pub(crate) fn with_reader(
        media: T,
        options: Option<RecordOptions>,
        read: Read<T>,
    ) -> Result<Self> {
        let options = options.map_or_else(|| media.record_options(), Ok)?;
        options.require_write_limits()?;
        let mut source = options.clone();
        source.set_select(crate::Selector::all());
        source.set_filter(crate::Filter::always_true());
        source.set_max_row_size(None);
        source.set_max_byte_size(None);
        source.set_row_offset(None);
        let source_field = Arc::new(media.read_arrow_field(&source)?);
        let field = Arc::new(scan_field(&source_field, &options)?);
        Ok(Self {
            media: Arc::new(Mutex::new(media)),
            source_field,
            field,
            options,
            read,
            edited: None,
            rows: Arc::new(OnceLock::new()),
        })
    }

    fn media(&self) -> MutexGuard<'_, T> {
        self.media
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn read(&self) -> Result<Serie> {
        match &self.edited {
            Some(rows) => self
                .options
                .apply_stream(rows.clone().into_stream()?)
                .map(Serie::from),
            None => (self.read)(&self.media(), &self.options),
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

    fn planned(&self, options: RecordOptions) -> Result<Self> {
        options.require_write_limits()?;
        let field = Arc::new(scan_field(&self.source_field, &options)?);
        Ok(Self {
            field,
            options,
            rows: Arc::new(OnceLock::new()),
            ..self.clone()
        })
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
        let mut options = self.options.clone();
        options.set_filter(crate::Filter::always_true());
        options.set_select(crate::Selector::all());
        options.set_max_row_size(None);
        options.set_max_byte_size(None);
        options.set_row_offset(None);
        options.set_field((*field).clone());
        self.source_field = Arc::clone(&field);
        self.field = field;
        self.options = options;
        self.rows = Arc::new(OnceLock::from(edited.clone()));
        self.edited = Some(edited);
        Ok(())
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
        // A stated Arrow-byte limit is global and cannot restart at a seek.
        if self.options.max_byte_size().is_some()
            || self
                .options
                .max_row_size()
                .is_some_and(|length| index as u64 >= length)
        {
            return self.rows().scalar(index);
        }
        let mut options = self.options.clone();
        let offset = options
            .row_offset()
            .unwrap_or(0)
            .checked_add(index as u64)
            .ok_or_else(|| crate::Error::InvalidRecord {
                path: self.field.name().into(),
                reason: "the row offset exceeds uint64".into(),
            })?;
        options.set_row_offset(Some(offset));
        options.set_max_row_size(Some(1));
        options.require_write_limits()?;
        let mut scan = self.clone();
        scan.options = options;
        scan.rows = Arc::new(OnceLock::new());
        let mut rows = scan.into_rows()?.into_stream()?;
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
/// Scan clauses remain in the medium's native options: Parquet and Iceberg
/// prune metadata, Avro skips unselected fields, and row codecs read rows.
pub trait MediaSerieValue<T: IOMedia + Send + 'static>: SerieValue {
    /// The retained medium and immutable scan state.
    fn media_state(&self) -> &MediaSerieState<T>;
    /// Build this specialized series from its common scan state.
    ///
    /// # Errors
    /// A scan configuration belonging to another media encoding.
    fn from_media_state(state: MediaSerieState<T>) -> Result<Self>;

    /// Validate native options before a source can be opened.
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

    /// The scan configuration retained without pulling rows.
    fn read_options(&self) -> &RecordOptions {
        &self.media_state().options
    }

    /// Conjoin a source predicate, retaining it for native pruning.
    ///
    /// # Errors
    /// Parse and binding failures before a row is pulled.
    fn with_filter(self, filter: impl IntoFilter) -> Result<Self> {
        let mut options = self.read_options().clone();
        options.set_filter(options.filter().clone().and(filter.into_filter()?));
        self.media_state()
            .planned(options)
            .and_then(Self::from_media_state)
    }

    /// Choose source expressions before decoding their values.
    ///
    /// # Errors
    /// Parse and binding failures before a row is pulled.
    fn with_select(self, select: impl IntoSelector) -> Result<Self> {
        let mut options = self.read_options().clone();
        options.set_select(select.into_selector()?);
        self.media_state()
            .planned(options)
            .and_then(Self::from_media_state)
    }

    /// Bound result rows in the native scan, after filtering and selection.
    ///
    /// # Errors
    /// An invalid combination with a merge key.
    fn with_row_range(self, offset: u64, length: Option<u64>) -> Result<Self> {
        let mut options = self.read_options().clone();
        options.set_row_offset(Some(offset));
        options.set_max_row_size(length);
        self.media_state()
            .planned(options)
            .and_then(Self::from_media_state)
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

    /// Publish any serie through the medium's explicit write door.
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

pub(crate) fn require_kind(options: &RecordOptions, kind: &str) -> Result<()> {
    let accepted = match kind {
        "Ipc" => matches!(options, RecordOptions::Ipc(_)),
        #[cfg(feature = "parquet")]
        "Parquet" | "IcebergTable" => matches!(options, RecordOptions::Parquet(_)),
        "Avro" => matches!(options, RecordOptions::Avro(_)),
        "Csv" => matches!(options, RecordOptions::Csv(_)),
        "Text" => matches!(options, RecordOptions::Text(_)),
        "Excel" => matches!(options, RecordOptions::Excel(_)),
        "Xmla" => matches!(options, RecordOptions::Xmla(_)),
        _ => true,
    };
    if accepted {
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
    ($name:ident, $variant:ident, $access:ident, $access_mut:ident $(, $read:path)?) => {
        /// A lazy series retaining this medium's native scan configuration.
        #[derive(Clone, Debug)]
        pub struct $name {
            state: $crate::MediaSerieState<Box<dyn $crate::IOBase>>,
        }
        impl $name {
            /// Retain a media handle; discover its schema without decoding
            /// result batches. Explicit options override the handle's own.
            ///
            /// # Errors
            /// Schema, encoding and scan binding refusals.
            pub fn new<H: $crate::IOBase + 'static>(media: H, options: Option<$crate::media::RecordOptions>) -> $crate::Result<Self> {
                let media: Box<dyn $crate::IOBase> = Box::new(media);
                let options = options.map_or_else(|| $crate::IOMedia::record_options(&media), Ok)?;
                $crate::media_serie::require_kind(&options, stringify!($variant))?;
                Ok(Self { state: $crate::media_serie::MediaSerieState::with_reader(media, Some(options), <Self as $crate::MediaSerieValue<Box<dyn $crate::IOBase>>>::read_native)? })
            }
        }
        impl $crate::MediaSerieValue<Box<dyn $crate::IOBase>> for $name {
            fn media_state(&self) -> &$crate::MediaSerieState<Box<dyn $crate::IOBase>> { &self.state }
            fn from_media_state(state: $crate::MediaSerieState<Box<dyn $crate::IOBase>>) -> $crate::Result<Self> { Self::require_media_options(&state.options)?; Ok(Self { state }) }
            fn require_media_options(options: &$crate::media::RecordOptions) -> $crate::Result<()> { $crate::media_serie::require_kind(options, stringify!($variant)) }
            $(fn read_native(media: &Box<dyn $crate::IOBase>, options: &$crate::media::RecordOptions) -> $crate::Result<$crate::Serie> { $read(media.as_ref(), options) })?
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
                let sliced = rows.slice(offset, length)?;
                let mut state = self.state.clone();
                state.rows = ::std::sync::Arc::new(::std::sync::OnceLock::from(sliced.clone()));
                state.edited = Some(sliced);
                $crate::media::IORecordOptions::set_filter(&mut state.options, $crate::Filter::always_true());
                $crate::media::IORecordOptions::set_select(&mut state.options, $crate::Selector::all());
                $crate::media::IORecordOptions::set_row_offset(&mut state.options, None);
                $crate::media::IORecordOptions::set_max_row_size(&mut state.options, None);
                $crate::media::IORecordOptions::set_max_byte_size(&mut state.options, None);
                $crate::media::IORecordOptions::set_field(&mut state.options, (*state.field).clone());
                state.source_field = ::std::sync::Arc::clone(&state.field);
                Ok(Self { state })
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
            #[cfg(feature = "parquet")]
            Self::Parquet(value) => Some(value.media_state()),
            Self::Avro(value) => Some(value.media_state()),
            Self::Csv(value) => Some(value.media_state()),
            Self::Text(value) => Some(value.media_state()),
            Self::Excel(value) => Some(value.media_state()),
            Self::Xmla(value) => Some(value.media_state()),
            #[cfg(feature = "iceberg")]
            Self::IcebergTable(value) => Some(value.media_state()),
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
            #[cfg(feature = "parquet")]
            Self::Parquet(value) => Arc::make_mut(value).splice(range, rows),
            Self::Avro(value) => Arc::make_mut(value).splice(range, rows),
            Self::Csv(value) => Arc::make_mut(value).splice(range, rows),
            Self::Text(value) => Arc::make_mut(value).splice(range, rows),
            Self::Excel(value) => Arc::make_mut(value).splice(range, rows),
            Self::Xmla(value) => Arc::make_mut(value).splice(range, rows),
            #[cfg(feature = "iceberg")]
            Self::IcebergTable(value) => Arc::make_mut(value).splice(range, rows),
            Self::WarehouseTable(value) => Arc::make_mut(value).splice(range, rows),
            #[cfg(feature = "http")]
            Self::Http(value) => Arc::make_mut(value).splice(range, rows),
            Self::GenericMedia(value) => Arc::make_mut(value).splice(range, rows),
            _ => unreachable!("only specialized media have a scan state"),
        }
    }
}
