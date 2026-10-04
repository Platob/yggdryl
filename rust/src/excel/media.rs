//! A workbook as record media over any byte handle.
//!
//! An `.xlsx` handle holds one workbook; the medium reads and writes one of
//! its worksheets - the one the options name, else the first - as rows,
//! through the same calls as every other encoding. A read opens the package
//! over the handle and streams the sheet's part; a write produces the package
//! again with that sheet replaced and every other member carried over, and
//! hands the handle the whole once.

use std::sync::{Arc, OnceLock};

use arrow_array::RecordBatchIterator;
use smol_str::SmolStr;

use crate::arrow::{BatchReader, arrow_schema_from_field, field_from_arrow_schema};
use crate::media::{IORecordOptions, RecordOptions};
use crate::{
    ArrowCastOptions, DataType, Error, Field, IOBase, IOMedia, Result, SerieReader, StructType,
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
    let workbook = open(handle)?;
    let Some(name) = addressed(&workbook, options)? else {
        return Ok(None);
    };
    let field = inferred(&workbook, &name, options)?;
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
    let rows = SerieReader::from_arrow_reader(Some(&root), batches, ArrowCastOptions::default())?;
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

/// A byte handle retained with one workbook configuration.
///
/// Rows flow through the ordinary [`IOMedia`] methods, and the wrapper
/// retains the [`ExcelOptions`] that [`IOMedia::record_options`] answers
/// with. [`IOBase::open`] holds the workbook - its package documents, and
/// the sheets and parts read since - until [`IOBase::close`], so the schema
/// and the row count of an open handle cost the package once.
#[derive(Debug)]
pub struct Excel<H: IOBase> {
    handle: H,
    options: ExcelOptions,
    opened: bool,
    cached: OnceLock<Workbook>,
}

impl<H: IOBase> Excel<H> {
    /// Wrap a handle with default options.
    #[must_use]
    pub fn new(handle: H) -> Self {
        Self {
            handle,
            options: ExcelOptions::new(),
            opened: false,
            cached: OnceLock::new(),
        }
    }

    /// Return this media with a complete configuration.
    #[must_use]
    pub fn with_options(mut self, options: ExcelOptions) -> Self {
        self.options = options;
        self
    }

    /// Return this media with a declared canonical row field.
    #[must_use]
    pub fn with_field(mut self, field: Field) -> Self {
        self.options.set_field(field);
        self
    }

    /// Return this media addressing the sheet `sheet`.
    #[must_use]
    pub fn with_sheet(mut self, sheet: impl Into<SmolStr>) -> Self {
        self.options.sheet = Some(sheet.into());
        self
    }

    /// Borrow the retained options.
    pub const fn options(&self) -> &ExcelOptions {
        &self.options
    }

    /// Borrow the retained options mutably.
    pub fn options_mut(&mut self) -> &mut ExcelOptions {
        &mut self.options
    }

    /// Borrow the underlying byte handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Borrow the underlying byte handle mutably.
    pub fn handle_mut(&mut self) -> &mut H {
        &mut self.handle
    }

    /// Consume this media and return its byte handle.
    pub fn into_handle(self) -> H {
        self.handle
    }

    /// Whether this session already holds the leaf's workbook, which answers
    /// every dimension ask with no call - and which a container's session
    /// never holds.
    fn warm(&self) -> bool {
        self.opened && self.cached.get().is_some()
    }

    /// The workbook, held while open, opened afresh otherwise.
    fn workbook(&self) -> Result<Held<'_>> {
        if self.opened {
            if let Some(workbook) = self.cached.get() {
                return Ok(Held::Cached(workbook));
            }
            let workbook = open(&self.handle)?;
            return Ok(Held::Cached(self.cached.get_or_init(|| workbook)));
        }
        open(&self.handle).map(Held::Fresh)
    }

    fn require_options<'a>(&self, options: &'a RecordOptions) -> Result<&'a ExcelOptions> {
        match options {
            RecordOptions::Excel(options) => Ok(options),
            _ => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.encoding"),
                reason: crate::text::expected_got("Excel record options", options.mime_type()),
            }),
        }
    }

    fn invalidate(&mut self) {
        self.cached = OnceLock::new();
    }
}

/// The workbook a media call reads: the one the open scope holds, or one
/// opened for this call and dropped with it.
enum Held<'a> {
    Cached(&'a Workbook),
    Fresh(Workbook),
}

impl std::ops::Deref for Held<'_> {
    type Target = Workbook;

    fn deref(&self) -> &Workbook {
        match self {
            Self::Cached(workbook) => workbook,
            Self::Fresh(workbook) => workbook,
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
        if !self.warm() && self.handle.is_container() {
            return crate::iomedia::container_row_size(
                &self.handle,
                &crate::iomedia::dimension_options(self)?,
            );
        }
        let workbook = self.workbook()?;
        let Some(name) = addressed(&workbook, &self.options)? else {
            return Ok(0);
        };
        let source = source(&workbook, &name, &self.options)?;
        let rows = rows(&workbook, &name)?;
        super::reader::row_count(&source, rows)
    }

    fn column_size(&self) -> Result<usize> {
        if let Some(field) = self.options.field() {
            return Ok(field.field_len());
        }
        if !self.warm() && self.handle.is_container() {
            return Ok(crate::iomedia::container_field(
                &self.handle,
                &crate::iomedia::dimension_options(self)?,
            )?
            .field_len());
        }
        let workbook = self.workbook()?;
        match addressed(&workbook, &self.options)? {
            Some(name) => Ok(inferred(&workbook, &name, &self.options)?.field_len()),
            None => Ok(0),
        }
    }

    fn record_options(&self) -> Result<RecordOptions> {
        Ok(RecordOptions::Excel(self.options.clone()))
    }

    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        let options = self.require_options(options)?;
        if let Some(field) = options.field() {
            return Ok(field);
        }
        if !self.warm() && self.handle.is_container() {
            return crate::iomedia::container_field(&self.handle, &options.clone().into());
        }
        let workbook = self.workbook()?;
        match addressed(&workbook, options)? {
            Some(name) => inferred(&workbook, &name, options),
            None => super::reader::empty_root(options.name()),
        }
    }

    fn overwrite_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        self.require_options(options)?;
        self.invalidate();
        crate::iobase::overwrite_arrow_reader_default(self, batches, options)
    }

    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_options(options)?;
        self.invalidate();
        crate::iobase::leaf_writer(self, batches, options)
    }

    fn append_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        self.require_options(options)?;
        self.invalidate();
        crate::iobase::append_arrow_reader_default(self, batches, options)
    }

    fn merge_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        self.require_options(options)?;
        self.invalidate();
        crate::iobase::merge_arrow_reader_default(self, batches, options)
    }
}

impl<H: IOBase> IOBase for Excel<H> {
    crate::delegate_iobase!(handle: pread, read_all_bytes, read_range_bytes, pstream_bytes,
        size, capacity, reserve, uri, url, bound_location, mtime, media_type, applied_codec, flush, parent,
        child_by_path, ls, kind, is_container);

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.invalidate();
        self.handle.pwrite(offset, bytes)
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        self.invalidate();
        self.handle.truncate(size)
    }

    fn set_media_type(&mut self, media_type: crate::MediaType) {
        self.invalidate();
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

    fn open(&mut self) -> Result<()> {
        if self.opened {
            return Ok(());
        }
        self.handle.open()?;
        self.invalidate();
        self.opened = true;
        Ok(())
    }

    fn opened(&self) -> bool {
        self.opened
    }

    fn close(&mut self) -> Result<()> {
        self.invalidate();
        self.opened = false;
        self.handle.close()
    }

    fn clear(&mut self) -> Result<()> {
        self.invalidate();
        self.handle.clear()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.invalidate();
        self.opened = false;
        self.handle.remove(recursive)
    }
}
