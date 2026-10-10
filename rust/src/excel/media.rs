//! A workbook as record media over any byte handle.
//!
//! An `.xlsx` handle holds one workbook; the medium reads and writes one of
//! its worksheets - the one the options name, else the first - as rows,
//! through the same calls as every other encoding. A read opens the package
//! over the handle and streams the sheet's part; a write produces the package
//! again with that sheet replaced and every other member carried over, and
//! hands the handle the whole once.

use std::sync::Arc;

use arrow_array::RecordBatchIterator;
use smol_str::SmolStr;

use crate::arrow::{BatchReader, arrow_schema_from_field, field_from_arrow_schema};
use crate::holder::Holder;
use crate::media::{
    CacheTtl, Entry, IORecordOptions, Media, MediaCache, MediaCodec, MediaWrapper, RecordOptions,
};
use crate::{
    ArrowCastOptions, DataType, Error, Field, IOBase, IOMedia, MimeType, Result,
    StreamChunkedSerie, StructType,
};

use super::cell::CellRef;
use super::options::ExcelOptions;
use super::parser::SheetRows;
use super::reader::Source;
use super::sheet::Sheet;
use super::workbook::Workbook;
use super::writer::SheetXml;

/// Open the workbook `handle` holds, over a handle of the package's own.
///
/// The owned handle carries the media type the copy took over, so
/// [`Workbook::open`] refuses a coded name without asking `handle` again.
fn open<H: IOBase + ?Sized>(handle: &H) -> Result<Workbook> {
    Workbook::open(crate::iobase::owned_handle(handle)?)
}

/// The worksheet a read under `options` addresses: the sheet named, else
/// the first worksheet; `None` for a workbook with none, or for a sheet the
/// workbook lacks, which reads as the empty stream a missing resource is -
/// a chart or dialog sheet of that name is refused by its kind.
fn addressed(workbook: &Workbook, options: &ExcelOptions) -> Result<Option<SmolStr>> {
    match &options.sheet {
        Some(name) => {
            if workbook.position(name).is_none() {
                return Ok(None);
            }
            workbook.sheet_part(name)?;
            Ok(Some(name.clone()))
        }
        None => Ok(workbook.first_worksheet().map(SmolStr::new)),
    }
}

/// What a read of the sheet `name` streams and names.
fn source(workbook: &Workbook, name: &str, options: &ExcelOptions) -> Result<Source> {
    workbook.sheet_part(name)?;
    Ok(Source {
        sheet: SmolStr::new(name),
        system: workbook.date_system(),
        strings: workbook.strings()?,
        styles: workbook.styles()?,
        range: options.cells(),
        header: options.header,
    })
}

/// The rows of the sheet `name`, parsed off its part.
fn rows(
    workbook: &Workbook,
    name: &str,
) -> Result<SheetRows<std::io::BufReader<Box<dyn std::io::Read + Send>>>> {
    Ok(SheetRows::new(
        std::io::BufReader::with_capacity(
            crate::DEFAULT_FETCH_BYTE_SIZE,
            workbook.sheet_reader(name)?,
        ),
        name,
    ))
}

/// The field the sheet `name` states for itself: the header's names over
/// the datatypes its cells prove.
fn inferred(workbook: &Workbook, name: &str, options: &ExcelOptions) -> Result<Field> {
    let source = source(workbook, name, options)?;
    let rows = rows(workbook, name)?;
    super::reader::infer_field(&source, rows, options.name())
}

/// Read the schema of the workbook `handle` holds: the declared field, else
/// the addressed sheet's own.
///
/// A handle holding nothing, or a workbook with no worksheet, states a root
/// with no columns.
///
/// # Errors
///
/// Returns a read, package or sheet failure, a refusal naming the first
/// cell whose datatype disagrees with its column's, or a codec failure for a
/// handle whose name declares a content coding (`trades.xlsx.gz`).
pub fn read_field<H: IOBase + ?Sized>(handle: &H, options: &ExcelOptions) -> Result<Field> {
    if let Some(field) = options.field() {
        return Ok(field);
    }
    let workbook = open(handle)?;
    match addressed(&workbook, options)? {
        Some(name) => inferred(&workbook, &name, options),
        None => super::reader::empty_root(options.name()),
    }
}

/// Count the records of the addressed sheet: every row inside the range,
/// less the header.
///
/// # Errors
///
/// Returns a read, package or sheet failure.
pub(crate) fn row_size<H: IOBase + ?Sized>(handle: &H, options: &ExcelOptions) -> Result<u64> {
    let workbook = open(handle)?;
    let Some(name) = addressed(&workbook, options)? else {
        return Ok(0);
    };
    let source = source(&workbook, &name, options)?;
    let rows = rows(&workbook, &name)?;
    super::reader::row_count(&source, rows)
}

/// The field the stored sheet states, `None` where it states none: an empty
/// handle, a workbook with no worksheet, a sheet with no rows.
///
/// # Errors
///
/// Returns a read, package or sheet failure.
pub(crate) fn stated_field<H: IOBase + ?Sized>(
    handle: &H,
    options: &ExcelOptions,
) -> Result<Option<Field>> {
    if handle.size() == 0 {
        return Ok(None);
    }
    stated(&open(handle)?, options)
}

/// The field the sheet `options` address states in `workbook`, every column
/// nullable; `None` where it states none: a workbook with no worksheet, a
/// sheet with no rows.
fn stated(workbook: &Workbook, options: &ExcelOptions) -> Result<Option<Field>> {
    let Some(name) = addressed(workbook, options)? else {
        return Ok(None);
    };
    let field = inferred(workbook, &name, options)?;
    if field.fields().is_empty() {
        return Ok(None);
    }
    // A worksheet records no nullability - any cell may be empty - so what
    // it states constrains the datatypes and never the absence of a value.
    let children = field
        .fields()
        .iter()
        .map(|child| child.clone().with_nullable(true));
    Ok(Some(Field::new(
        field.name(),
        DataType::from(StructType::from_fields(children)?),
        false,
    )))
}

/// Read the addressed sheet's rows as batches.
///
/// `field` types the rows: each of its columns is read from the sheet
/// column of the same header name (by position, without a header) through
/// the field's value contract. Without one, the sheet's own field is learned
/// first by one pass over the part, and the rows stream under it.
///
/// # Errors
///
/// Returns a read, package, sheet or pairing failure, or a codec failure
/// for a handle whose name declares a content coding.
pub fn read_batch_reader<H: IOBase + ?Sized>(
    handle: &H,
    field: Option<&Field>,
    options: &ExcelOptions,
) -> crate::arrow::Result<BatchReader> {
    let workbook = open(handle)?;
    let Some(name) = addressed(&workbook, options)? else {
        // Per the laziness contract, a missing workbook holds no rows.
        let schema = match field.cloned().or_else(|| options.field()) {
            Some(field) => arrow_schema_from_field(&field)?,
            None => Arc::new(arrow_schema::Schema::empty()),
        };
        return Ok(Box::new(RecordBatchIterator::new(
            std::iter::empty(),
            schema,
        )));
    };
    let root = match field.cloned().or_else(|| options.field()) {
        Some(field) => field,
        None => inferred(&workbook, &name, options)?,
    };
    let source = source(&workbook, &name, options)?;
    let member = workbook.sheet_reader(&name)?;
    super::reader::batch_reader(
        source,
        member,
        &root,
        options.safe(),
        options.batch_row_size(),
        options.batch_byte_size(),
    )
}

/// Replace the addressed sheet of the workbook `handle` holds with
/// `batches`, every other sheet and part carried over; an empty handle
/// becomes a workbook of that one sheet.
///
/// The rows start at the range's top-left cell, the column names in its
/// first row when the options say `header`; text is written inline, so the
/// write holds one batch and the package image, never a string table.
///
/// # Errors
///
/// Returns a schema, value, package or write failure, a refusal naming the
/// row that would leave the grid, or a codec failure for a handle whose name
/// declares a content coding - before a byte is written.
pub fn overwrite_arrow_reader<H: IOBase + ?Sized>(
    handle: &mut H,
    batches: BatchReader,
    options: &ExcelOptions,
) -> Result<()> {
    // Refused before the stream is read, an empty handle included.
    super::reject_outer_coding(handle)?;
    let root = field_from_arrow_schema(options.name(), batches.schema().as_ref())?;
    let rows =
        StreamChunkedSerie::from_arrow_reader(Some(&root), batches, ArrowCastOptions::default())?;
    let mut workbook = if handle.size() == 0 {
        Workbook::new()
    } else {
        open(handle)?
    };
    let name = options
        .sheet
        .clone()
        .or_else(|| workbook.first_worksheet().map(SmolStr::new))
        .unwrap_or_else(|| SmolStr::new_static(super::DEFAULT_SHEET_NAME));
    let at = match workbook.position(&name) {
        Some(at) => at,
        None => {
            workbook.insert_sheet(Sheet::new(name.clone())?)?;
            workbook.len() - 1
        }
    };
    let anchor = options
        .range
        .map_or(CellRef::new(0, 0), |range| range.start());
    let system = workbook.date_system();
    let header = options.header;
    let sheet = name.clone();
    let failure = Arc::new(std::sync::Mutex::new(None));
    let slot = Arc::clone(&failure);
    let stream = move |offset: u32| -> Result<Box<dyn std::io::Read + Send>> {
        let xml = SheetXml::new(rows, root, sheet, system, offset, anchor, header)?;
        if let Ok(mut held) = slot.lock() {
            *held = Some(xml.failure());
        }
        Ok(Box::new(xml))
    };
    let bytes = match workbook.write_package(Some((at, Box::new(stream)))) {
        Ok(bytes) => bytes,
        Err(error) => {
            // The archive saw an `io::Error`; the stream kept the refusal.
            let refusal = failure
                .lock()
                .ok()
                .and_then(|slot| slot.as_ref().and_then(|held| held.lock().ok()?.take()));
            return Err(refusal.unwrap_or(error));
        }
    };
    // The package was read through a handle of its own onto the same file,
    // and Windows refuses to resize a file while a view of it is mapped:
    // that handle goes before this one rewrites the file.
    drop(workbook);
    handle.write_all_bytes(&bytes)
}

/// The MIME type a workbook answers.
static EXCEL_TYPES: [MimeType; 1] = [MimeType::XLSX];

/// An Office Open XML workbook as a record medium: [`read_batch_reader`],
/// [`read_field`] and [`overwrite_arrow_reader`] behind the one contract
/// every medium answers.
#[derive(Debug)]
pub struct ExcelCodec;

/// The workbook medium, claimed under its MIME type.
pub static EXCEL_CODEC: ExcelCodec = ExcelCodec;

impl MediaCodec for ExcelCodec {
    fn name(&self) -> &'static str {
        "excel"
    }

    fn title(&self) -> &'static str {
        "Excel"
    }

    fn rank(&self) -> u8 {
        6
    }

    fn mime_types(&self) -> &'static [MimeType] {
        &EXCEL_TYPES
    }

    fn default_options(&self, _base: &MimeType) -> RecordOptions {
        RecordOptions::registered(ExcelOptions::new())
    }

    /// A workbook is a ZIP package deflated inside, so an outer coding names
    /// a file no spreadsheet opens.
    fn compresses_internally(&self) -> bool {
        true
    }

    fn read_batch_reader(
        &self,
        handle: &dyn IOBase,
        declared: Option<&Field>,
        options: &RecordOptions,
    ) -> Result<BatchReader> {
        let excel = options.require_settings::<ExcelOptions>()?;
        Ok(read_batch_reader(handle, declared, excel)?)
    }

    fn row_size(&self, handle: &dyn IOBase, options: &RecordOptions) -> Result<u64> {
        row_size(handle, options.require_settings::<ExcelOptions>()?)
    }

    fn read_field(&self, handle: &dyn IOBase, options: &RecordOptions) -> Result<Field> {
        read_field(handle, options.require_settings::<ExcelOptions>()?)
    }

    /// A sheet may hold no rows, which is a resource with no shape yet,
    /// never one that cannot be read.
    fn stated_field(&self, handle: &dyn IOBase, options: &RecordOptions) -> Result<Option<Field>> {
        stated_field(handle, options.require_settings::<ExcelOptions>()?)
    }

    fn overwrite_arrow_reader(
        &self,
        handle: &mut dyn IOBase,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        let excel = options.require_settings::<ExcelOptions>()?;
        overwrite_arrow_reader(handle, batches, excel)
    }

    fn open(&self, handle: Holder) -> Media {
        Media::Registered(Box::new(Excel::new(handle)))
    }
}

/// A byte handle retained with one workbook configuration.
///
/// Rows flow through the ordinary [`IOMedia`] methods, and the wrapper
/// retains the [`ExcelOptions`] that [`IOMedia::record_options`] answers
/// with. The workbook - its package documents, and the sheets and parts read
/// since - is held in a [`MediaCache`] beside what the addressed sheet
/// states, from [`IOBase::open`] until [`IOBase::close`], or for the
/// options' `cache_ttl` on a closed handle, so the schema and the row count
/// of an open handle cost the package once.
#[derive(Debug)]
pub struct Excel<H: IOBase> {
    handle: H,
    options: ExcelOptions,
    /// The workbook as the entry's state, and what the sheet the retained
    /// options address states; never held for a container.
    cache: MediaCache,
}

/// The workbook an entry holds as its state.
fn book_of(entry: &Entry) -> Option<Arc<Workbook>> {
    Arc::clone(entry.state.as_ref()?)
        .downcast::<Workbook>()
        .ok()
}

impl<H: IOBase> Excel<H> {
    /// Wrap a handle with default options.
    #[must_use]
    pub fn new(handle: H) -> Self {
        Self {
            handle,
            options: ExcelOptions::new(),
            cache: MediaCache::new(),
        }
    }

    /// Return this media with a complete configuration, dropping what the
    /// cache holds of the sheet the previous one addressed.
    #[must_use]
    pub fn with_options(mut self, options: ExcelOptions) -> Self {
        self.options = options;
        self.cache.invalidate();
        self
    }

    /// Return this media with a declared canonical row field.
    #[must_use]
    pub fn with_field(mut self, field: Field) -> Self {
        self.options.set_field(field);
        self
    }

    /// Return this media addressing the sheet `sheet`, dropping what the
    /// cache holds of the sheet addressed before.
    #[must_use]
    pub fn with_sheet(mut self, sheet: impl Into<SmolStr>) -> Self {
        self.options.sheet = Some(sheet.into());
        self.cache.invalidate();
        self
    }

    /// Borrow the retained options.
    pub const fn options(&self) -> &ExcelOptions {
        &self.options
    }

    /// Borrow the retained options mutably, dropping what the cache holds:
    /// the sheet, the header and the range they name decide what is stated.
    pub fn options_mut(&mut self) -> &mut ExcelOptions {
        self.cache.invalidate();
        &mut self.options
    }

    /// Borrow the underlying byte handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Borrow the underlying byte handle mutably, dropping the held workbook
    /// before any byte mutation can occur.
    pub fn handle_mut(&mut self) -> &mut H {
        self.cache.invalidate();
        &mut self.handle
    }

    /// Consume this media and return its byte handle.
    pub fn into_handle(self) -> H {
        self.handle
    }

    fn require_options<'a>(&self, options: &'a RecordOptions) -> Result<&'a ExcelOptions> {
        options
            .settings::<ExcelOptions>()
            .ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$.encoding"),
                reason: crate::text::expected_got("Excel record options", options.mime_type()),
            })
    }

    /// The package opened as an entry: the workbook as its state, and the
    /// field the sheet the retained options address states.
    fn read_entry(&self) -> Result<Entry> {
        let workbook = open(&self.handle)?;
        let origin = stated(&workbook, &self.options)?;
        let state: Arc<dyn std::any::Any + Send + Sync> = Arc::new(workbook);
        Ok(Entry {
            columns: Some(origin.as_ref().map_or(0, Field::field_len)),
            origin,
            rows: None,
            state: Some(state),
        })
    }

    /// The leaf's entry under `ttl`, `served` where the cache had one: the
    /// package opened and kept where the cache keeps, else opened for this
    /// call alone.
    fn entry(&self, ttl: CacheTtl, served: Option<Entry>) -> Result<Entry> {
        if let Some(entry) = served {
            return Ok(entry);
        }
        if self.cache.keeps(ttl) {
            self.cache
                .get_or_fill(ttl, crate::media::cache::now(), || self.read_entry())
        } else {
            self.read_entry()
        }
    }

    /// The workbook `entry` holds, else - an emptied handle's entry holds
    /// none - the package opened for this call.
    fn workbook_of(&self, entry: &Entry) -> Result<Arc<Workbook>> {
        match book_of(entry) {
            Some(workbook) => Ok(workbook),
            None => Ok(Arc::new(open(&self.handle)?)),
        }
    }
}

impl<H: IOBase> IOMedia for Excel<H> {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    fn row_size(&self) -> Result<u64> {
        let ttl = self.options.cache_ttl;
        let now = crate::media::cache::now();
        let served = self.cache.entry(ttl, now);
        if let Some(rows) = served.as_ref().and_then(|entry| entry.rows) {
            return Ok(rows);
        }
        // Past the cache, which only a leaf ever fills.
        if served.is_none() && self.handle.is_container() {
            return crate::iomedia::container_row_size(
                &self.handle,
                &crate::iomedia::dimension_options(self)?,
            );
        }
        let held = served.is_some();
        let entry = self.entry(ttl, served)?;
        let workbook = self.workbook_of(&entry)?;
        let count = match addressed(&workbook, &self.options)? {
            Some(name) => {
                let source = source(&workbook, &name, &self.options)?;
                super::reader::row_count(&source, rows(&workbook, &name)?)?
            }
            None => 0,
        };
        // Added to the entry served - the one held, or the one this ask just
        // kept - its stamp kept: its TTL counts from the one reading it was.
        if held || self.cache.keeps(ttl) {
            self.cache.add(ttl, now, |entry| entry.rows = Some(count));
        }
        Ok(count)
    }

    fn column_size(&self) -> Result<usize> {
        if let Some(field) = self.options.field() {
            return Ok(field.field_len());
        }
        let ttl = self.options.cache_ttl;
        let served = self.cache.entry(ttl, crate::media::cache::now());
        if served.is_none() && self.handle.is_container() {
            return Ok(crate::iomedia::container_origin(
                &self.handle,
                crate::iomedia::dimension_options(self)?,
            )?
            .map_or(0, |field| field.field_len()));
        }
        Ok(self.entry(ttl, served)?.columns.unwrap_or_default())
    }

    fn record_options(&self) -> Result<RecordOptions> {
        Ok(self.options.clone().into())
    }

    /// The field the sheet the retained options address states - every
    /// column nullable, since a worksheet records none - named as the
    /// options name it; `None` for an empty handle, a workbook with no
    /// worksheet, a sheet with no rows.
    fn read_origin_field(&self) -> Result<Option<Field>> {
        let ttl = self.options.cache_ttl;
        let served = self.cache.entry(ttl, crate::media::cache::now());
        if served.is_none() && self.handle.is_container() {
            return crate::iomedia::container_origin(
                &self.handle,
                crate::iomedia::dimension_options(self)?,
            );
        }
        Ok(self
            .entry(ttl, served)?
            .origin
            .map(|origin| origin.with_name(self.options.name())))
    }

    /// The one schema answer, the declared root else the field the addressed
    /// sheet's cells prove, as the `where` and `select` leave it - kept here
    /// because the sheet, the header and the range `options` state may not
    /// be the retained ones, and are read off the held workbook, and because
    /// options of another encoding are refused by name.
    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        let excel = self.require_options(options)?;
        let root = match excel.field() {
            Some(field) => field,
            None => {
                let ttl = excel.cache_ttl;
                let served = self.cache.entry(ttl, crate::media::cache::now());
                // Past the cache, which only a leaf ever fills.
                if served.is_none() && self.handle.is_container() {
                    return crate::iomedia::container_field(&self.handle, options);
                }
                let workbook = self.workbook_of(&self.entry(ttl, served)?)?;
                match addressed(&workbook, excel)? {
                    Some(name) => inferred(&workbook, &name, excel)?,
                    None => super::reader::empty_root(excel.name())?,
                }
            }
        };
        crate::iomedia::field_under(options, &root)
    }

    fn read_serie(&self, options: Option<&RecordOptions>) -> Result<crate::Serie> {
        let options = crate::iomedia::own_options(self, options)?;
        self.require_options(&options)?;
        crate::iomedia::read_record_serie(self, Some(&options))
    }

    fn overwrite_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        let options = crate::iomedia::own_options(self, options)?;
        let options = options.as_ref();
        let batches = crate::StreamChunkedSerie::from_serie(value)?.into_arrow_reader();
        self.require_options(options)?;
        let result =
            crate::iobase::overwrite_arrow_reader_default_with_field(self, batches, options)
                .map(|(_, result)| result);
        // A sheet's field is what its cells prove when read, which the field
        // written does not decide, and the writer renders the sheet's XML as
        // the batches arrive without a parsed workbook to keep: the next ask
        // opens the package afresh.
        self.cache.invalidate();
        result
    }

    fn overwrite_prepared_serie(
        &mut self,
        value: crate::StreamChunkedSerie,
        options: &RecordOptions,
    ) -> Result<()> {
        let batches = value.into_arrow_reader();
        self.require_options(options)?;
        let result = crate::iobase::leaf_writer(self, batches, options);
        self.cache.invalidate();
        result
    }

    fn append_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        let options = crate::iomedia::own_options(self, options)?;
        let options = options.as_ref();
        let batches = crate::StreamChunkedSerie::from_serie(value)?.into_arrow_reader();
        self.require_options(options)?;
        let result = crate::iobase::append_arrow_reader_default(self, batches, options);
        self.cache.invalidate();
        result
    }

    fn merge_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        let options = crate::iomedia::own_options(self, options)?;
        let options = options.as_ref();
        let batches = crate::StreamChunkedSerie::from_serie(value)?.into_arrow_reader();
        self.require_options(options)?;
        let result = crate::iobase::merge_arrow_reader_default(self, batches, options);
        self.cache.invalidate();
        result
    }
}

impl<H: IOBase> IOBase for Excel<H> {
    crate::delegate_iobase!(handle: pread, read_all_bytes, read_range_bytes, read_tail_bytes,
        pstream_bytes,
        size, set_known_size, capacity, reserve, uri, url, bound_location, mtime, media_type, applied_codec, flush, parent,
        child_by_path, ls, kind, is_container);

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.cache.invalidate();
        self.handle.pwrite(offset, bytes)
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        self.cache.invalidate();
        self.handle.truncate(size)
    }

    fn create_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.cache.invalidate();
        self.handle.create_bytes(bytes)
    }

    fn set_media_type(&mut self, media_type: crate::MediaType) {
        self.cache.invalidate();
        self.handle.set_media_type(media_type);
    }

    /// A workbook is a record encoding, whatever media type the bytes
    /// underneath carry.
    fn is_tabular(&self) -> bool {
        true
    }

    /// A record encoding is never read as one whole byte value.
    fn is_atomic(&self) -> bool {
        false
    }

    /// Materialize the handle and hold the workbook, opened as it is first
    /// asked for, until [`close`](IOBase::close).
    fn open(&mut self) -> Result<()> {
        if self.cache.is_open() {
            return Ok(());
        }
        self.handle.open()?;
        self.cache.invalidate();
        self.cache.open();
        Ok(())
    }

    fn opened(&self) -> bool {
        self.cache.is_open()
    }

    fn close(&mut self) -> Result<()> {
        self.cache.close();
        self.handle.close()
    }

    /// Empty the workbook; the cache then holds what an empty handle states
    /// - no sheet, no row, no column - where it keeps.
    fn clear(&mut self) -> Result<()> {
        self.cache.invalidate();
        self.handle.clear()?;
        // A container caches nothing: its leaves answer for it on every ask.
        if self.cache.keeps(self.options.cache_ttl) && !self.handle.is_container() {
            self.cache.update(crate::media::cache::now(), |entry| {
                entry.origin = None;
                entry.rows = Some(0);
                entry.columns = Some(0);
                entry.state = None;
            });
        }
        Ok(())
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.cache.close();
        self.handle.remove(recursive)
    }
}

impl MediaWrapper for Excel<Holder> {
    fn medium(&self) -> &'static dyn MediaCodec {
        &EXCEL_CODEC
    }

    fn handle(&self) -> &Holder {
        &self.handle
    }

    fn into_handle(self: Box<Self>) -> Holder {
        self.handle
    }

    fn with_field(self: Box<Self>, field: Field) -> Box<dyn MediaWrapper> {
        Box::new(Excel::with_field(*self, field))
    }
}
