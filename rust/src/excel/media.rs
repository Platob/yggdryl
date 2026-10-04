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
use super::records::RowsWriteLayout;
use super::records::{FlatBinding, FlatExtent, Header, RowsLayout, RowsWindow};
use super::sheet::Sheet;
use super::styles::Splice;
use super::workbook::{NamedTable, Replaced, SheetReplacement, Workbook};
use super::writer::{SheetXml, WriteHeader, overwrite_table_body, temporal_formats};
use crate::RecordHeader;

/// Open the workbook `handle` holds, over a handle of the package's own.
///
/// The owned handle carries the media type the copy took over, so
/// [`Workbook::open`] refuses a coded name without asking `handle` again.
fn open<H: IOBase + ?Sized>(handle: &H, media_type: &crate::MediaType) -> Result<Workbook> {
    Workbook::open(crate::iobase::owned_handle(handle, media_type)?)
}

/// Discover named tables and suggested occupied-cell regions in a workbook.
///
/// None inspects all worksheets in tab order; a named sheet is compared
/// without ASCII case. Results are ordered by tab, source rectangle and table
/// name. Named table cells are excluded from suggestions. A suggestion uses
/// eight-neighbour contact, not the worksheet's declared dimension, and does
/// not select a table or infer a header. Its bounding rectangle can contain
/// gaps or overlap a named table even though that table's cells were excluded.
/// At most 1,024 regions are returned;
/// exceeding that bound is a located refusal, never truncation.
///
/// ```
/// use yggdryl::{holder::Buffer, excel::regions};
/// assert!(regions(&Buffer::new(), None)?.is_empty());
/// # Ok::<(), yggdryl::Error>(())
/// ```
///
/// # Errors
///
/// Returns malformed package/table metadata, an unknown or non-worksheet
/// selection, a located cell refusal, or the region-count bound refusal.
pub fn regions<H: IOBase + ?Sized>(
    handle: &H,
    sheet: Option<&str>,
) -> Result<Vec<super::ExcelRegion>> {
    let workbook = open(handle, handle.media_type())?;
    super::regions::read(&workbook, sheet)
}

/// One resolved record selection, shared by schema, reader and dimensions.
struct Region {
    sheet: SmolStr,
    range: super::CellRange,
    header: RecordHeader,
    explicit_range: bool,
    names: Option<Arc<[SmolStr]>>,
    row_count: Option<u32>,
    /// The metadata pass's observed cell extent and selected merges.
    geometry: Option<(super::CellRange, Vec<super::CellRange>)>,
    probed: Option<(Field, u64)>,
}

/// Named-table selection resolves once before any schema or row pass.
fn addressed(
    workbook: &Workbook,
    options: &ExcelOptions,
    declared: Option<&Field>,
) -> Result<Option<Region>> {
    options.require_valid()?;
    let header = options.header;
    if let Some(name) = options.table() {
        let NamedTable {
            sheet,
            table,
            columns: names,
            ..
        } = workbook.named_table(name)?;
        let first = table.range.start().row() + table.header_rows;
        let after = table.range.end().row() + 1 - table.totals_rows;
        let has_rows = first < after;
        // A no-body table keeps its legal extent solely for column coordinates;
        // row_count=0 prevents its header/totals from becoming a record.
        let range = if has_rows {
            super::CellRange::new(
                CellRef::new(first, table.range.start().column()),
                CellRef::new(after - 1, table.range.end().column()),
            )
        } else {
            table.range
        };
        return Ok(Some(Region {
            sheet,
            range,
            header: RecordHeader::None,
            explicit_range: true,
            names: matches!(header, RecordHeader::Source | RecordHeader::Infer)
                .then(|| Arc::from(names)),
            row_count: Some(after - first),
            geometry: None,
            probed: None,
        }));
    }
    let sheet = match options.sheet() {
        Some(name) => {
            if workbook.position(name).is_none() {
                return Ok(None);
            }
            workbook.sheet_part(name)?;
            SmolStr::new(name)
        }
        None => match workbook.first_worksheet() {
            Some(name) => SmolStr::new(name),
            None => return Ok(None),
        },
    };
    let mut region = Region {
        sheet,
        range: options.cells()?,
        header,
        explicit_range: options.range().is_some(),
        names: None,
        row_count: None,
        geometry: None,
        probed: None,
    };
    if header == RecordHeader::Infer {
        // Same SheetRows parser observes trailing merge metadata without
        // constructing cell rows; discovery then records actual run contacts.
        let (physical, merges) = observe_geometry(workbook, &region)?;
        let candidates = if region.explicit_range {
            None
        } else {
            Some(super::regions::read_for_infer(
                workbook,
                &region.sheet,
                &merges,
            )?)
        };
        if let Some(candidates) = &candidates
            && candidates.iter().any(|candidate| {
                matches!(
                    &candidate.region.kind,
                    super::regions::ExcelRegionKind::Table { .. }
                )
            })
        {
            super::regions::require_single(candidates, None, &merges)?;
        }
        let occupied = candidates
            .as_deref()
            .and_then(super::regions::occupied_extent);
        let (observed, depth) = infer_geometry(&region, occupied, &merges)?;
        let chosen = if !region.explicit_range {
            depth
                .map(|depth| {
                    RowsWindow::implicit_range(observed, depth, merges.iter().copied()).map(
                        |range| {
                            // Physical row occupancy owns cardinality, while actual
                            // value/formula facts own the implicit column span.
                            let last = physical.map_or(range.end().row(), |physical| {
                                range.end().row().max(physical.end().row())
                            });
                            super::CellRange::new(
                                range.start(),
                                CellRef::new(last, range.end().column()),
                            )
                        },
                    )
                })
                .transpose()?
        } else {
            None
        };
        if let Some(candidates) = &candidates {
            let header = chosen
                .map(|range| {
                    RowsWindow::header_range(range, depth.expect("chosen range has a depth"))
                })
                .transpose()?;
            super::regions::require_single(candidates, header, &merges)?;
        }
        if let Some(depth) = depth {
            region.range = if region.explicit_range {
                region.range
            } else {
                chosen.expect("inferred depth has a chosen range")
            };
            region.header = RecordHeader::Rows(depth);
            region.geometry = Some((region.range, merges));
            if declared.is_some() {
                let source = source(workbook, &region)?;
                super::reader::inferred_rows_evidence(
                    &source,
                    rows(workbook, &region.sheet)?,
                    depth,
                )?;
            }
        } else {
            let source = source(workbook, &region)?;
            let resolved = super::reader::probe_header(
                &source,
                rows(workbook, &region.sheet)?,
                options.name(),
                declared,
                options.safe(),
            )?;
            region.header = resolved.policy;
            region.probed = Some((resolved.field, resolved.record_count));
        }
    }
    Ok(Some(region))
}

/// Observe coordinates and trailing merge refs through the existing parser.
/// No RawRow or RawCell is built, including for tall physical spans.
fn observe_geometry(
    workbook: &Workbook,
    region: &Region,
) -> Result<(Option<super::CellRange>, Vec<super::CellRange>)> {
    let member = workbook.sheet_reader(&region.sheet)?;
    let input = std::io::BufReader::with_capacity(crate::DEFAULT_FETCH_BYTE_SIZE, member);
    let mut observer = if region.explicit_range {
        SheetRows::observing_header_merges(input, region.sheet.clone(), region.range)
    } else {
        SheetRows::observing_header_geometry(input, region.sheet.clone(), super::cell::MAX_ROWS)
    };
    for event in &mut observer {
        event?;
    }
    observer.into_header_geometry()
}

fn infer_geometry(
    region: &Region,
    occupied: Option<super::CellRange>,
    merges: &[super::CellRange],
) -> Result<(super::CellRange, Option<u32>)> {
    let observed = if region.explicit_range {
        region.range
    } else {
        occupied.ok_or_else(super::options::ExcelOptions::no_evidence_error)?
    };
    // Extend only the *probe* to grid bottom. The resolved range is
    // bounded to the actual extent and linked header merges below.
    let selected = if region.explicit_range {
        observed
    } else {
        super::CellRange::new(
            observed.start(),
            CellRef::new(super::cell::MAX_ROWS - 1, observed.end().column()),
        )
    };
    let depth = super::records::merge_proven_depth(&region.sheet, selected, merges)?;
    Ok((observed, depth))
}

fn source(workbook: &Workbook, region: &Region) -> Result<Source> {
    Ok(Source {
        sheet: region.sheet.clone(),
        system: workbook.stated_date_system(),
        strings: workbook.strings()?,
        styles: workbook.styles()?,
        range: region.range,
        header: region.header,
        explicit_range: region.explicit_range,
        column_names: region.names.clone(),
        row_count: region.row_count,
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
fn inferred(workbook: &Workbook, region: &Region, options: &ExcelOptions) -> Result<Field> {
    if let Some((field, _)) = &region.probed {
        return Ok(field.clone());
    }
    if let RecordHeader::Rows(levels) = region.header {
        return Ok(inferred_rows(workbook, region, options, levels)?
            .root()
            .clone());
    }
    let source = source(workbook, region)?;
    let rows = rows(workbook, &region.sheet)?;
    super::reader::infer_field(&source, rows, options.name())
}

fn inferred_rows(
    workbook: &Workbook,
    region: &Region,
    options: &ExcelOptions,
    levels: u32,
) -> Result<RowsLayout> {
    let source = source(workbook, region)?;
    let member = workbook.sheet_reader(&region.sheet)?;
    let input = std::io::BufReader::with_capacity(crate::DEFAULT_FETCH_BYTE_SIZE, member);
    let known = region
        .geometry
        .as_ref()
        .map(|(extent, merges)| (*extent, merges.clone()));
    let rows = if known.is_some() {
        SheetRows::new(input, region.sheet.clone())
    } else if source.explicit_range {
        let window = RowsWindow::header_range(source.range, levels)?;
        SheetRows::capturing_header_merges(input, region.sheet.clone(), window)
    } else {
        SheetRows::capturing_header_geometry(input, region.sheet.clone(), levels)
    };
    super::reader::infer_rows_layout(&source, rows, options.name(), levels, known)
}

/// Tables state their complete body length; worksheets count stored rows.
fn count_rows(workbook: &Workbook, region: &Region) -> Result<u64> {
    if let Some((_, count)) = &region.probed {
        return Ok(*count);
    }
    if let Some(count) = region.row_count {
        return Ok(u64::from(count));
    }
    let source = source(workbook, region)?;
    super::reader::row_count(&source, rows(workbook, &region.sheet)?)
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
    options.require_valid()?;
    if let Some(field) = options.field() {
        return Ok(field);
    }
    let workbook = open(handle, handle.media_type())?;
    match addressed(&workbook, options, None)? {
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
    let workbook = open(handle, handle.media_type())?;
    let declared = options.field();
    let Some(name) = addressed(&workbook, options, declared.as_ref())? else {
        return Ok(0);
    };
    count_rows(&workbook, &name)
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
    let workbook = open(handle, handle.media_type())?;
    let Some(name) = addressed(&workbook, options, None)? else {
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
    options.require_valid()?;
    let workbook = open(handle, handle.media_type())?;
    let declared = field.cloned().or_else(|| options.field());
    let Some(mut name) = addressed(&workbook, options, declared.as_ref())? else {
        // Per the laziness contract, a missing workbook holds no rows.
        let schema = match declared.clone() {
            Some(field) => arrow_schema_from_field(&field)?,
            None => Arc::new(arrow_schema::Schema::empty()),
        };
        return Ok(Box::new(RecordBatchIterator::new(
            std::iter::empty(),
            schema,
        )));
    };
    let mut source = source(&workbook, &name)?;
    let (root, layout, merges) = match (declared, name.header) {
        (None, RecordHeader::Rows(levels)) => {
            let layout = inferred_rows(&workbook, &name, options, levels)?;
            source.range = layout.range();
            (layout.root().clone(), Some(layout), Vec::new())
        }
        (Some(root), RecordHeader::Rows(levels)) => {
            let (range, merges) = if let Some((_, merges)) = name.geometry.take() {
                (name.range, merges)
            } else {
                let member = workbook.sheet_reader(&name.sheet)?;
                super::reader::observed_merges(&source, member, levels)?
            };
            source.range = range;
            (root, None, merges)
        }
        (Some(root), _) => (root, None, Vec::new()),
        (None, _) => (inferred(&workbook, &name, options)?, None, Vec::new()),
    };
    let member = workbook.sheet_reader(&name.sheet)?;
    super::reader::batch_reader(source, member, &root, layout, merges, options)
}

/// Replace the addressed sheet of the workbook `handle` holds with
/// `batches`, every other sheet and part carried over; an empty handle
/// becomes a workbook of that one sheet.
///
/// Worksheet rows start at the range's top-left cell, with the resolved
/// header above them. A named table keeps its header and totals; body resizing
/// checks collisions and whether its totals can move without changing meaning.
/// Text is inline, without a string table.
/// Both paths hold one input batch and stage the package before publishing.
/// A table write additionally retains the source worksheet and an eager plan
/// of changed cell XML; it releases payloads as the package encoder consumes
/// them, without building a second complete worksheet image.
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
    options.require_write()?;
    // Refused before the stream is read, an empty handle included.
    let media_type = handle.media_type();
    super::reject_outer_coding(media_type)?;
    let root = field_from_arrow_schema(options.name(), batches.schema().as_ref())?;
    let rows = SerieReader::from_arrow_reader(Some(&root), batches, ArrowCastOptions::default())?;
    let mut workbook = if handle.size() == 0 {
        Workbook::new()
    } else {
        open(handle, media_type)?
    };
    if let Some(wanted) = options.table() {
        let NamedTable {
            sheet,
            table,
            columns: names,
            part,
            bytes,
            siblings,
        } = workbook.named_table(wanted)?;
        let at = workbook
            .position(&sheet)
            .expect("registered table worksheet");
        let source = workbook.part_bytes(&workbook.sheet_part(&sheet)?)?;
        let header = Header::resolve(
            table.range,
            FlatExtent::Table(Some(&names)),
            options.header == RecordHeader::Source,
            std::iter::empty::<Result<(u32, Option<std::borrow::Cow<'static, str>>)>>(),
        )?;
        let pairing = header.pairing(
            &root,
            FlatBinding {
                sheet: &sheet,
                range: table.range,
                by_name: options.header == RecordHeader::Source,
                authoritative: true,
            },
        )?;
        let mut columns = vec![None; names.len()];
        for (input, column) in pairing.into_iter().enumerate() {
            let Some(column) = column else {
                return Err(Error::InvalidRecord {
                    path: smol_str::format_smolstr!("$.{}", root.fields()[input].name()),
                    reason: smol_str::format_smolstr!(
                        "expected a column of named table {wanted}, got an unmatched input field"
                    ),
                });
            };
            let offset = (column - table.range.start().column()) as usize;
            if columns[offset].replace(input).is_some() {
                return Err(Error::InvalidRecord {
                    path: smol_str::format_smolstr!("$.table[{wanted}]"),
                    reason: smol_str::format_smolstr!(
                        "expected one input field for table column {}, got a duplicate",
                        names[offset]
                    ),
                });
            }
        }
        let columns: Vec<usize> = columns
            .into_iter()
            .enumerate()
            .map(|(offset, input)| {
                input.ok_or_else(|| Error::InvalidRecord {
                    path: smol_str::format_smolstr!("$.table[{wanted}]"),
                    reason: smol_str::format_smolstr!(
                        "expected an input field for table column {}, got none",
                        names[offset]
                    ),
                })
            })
            .collect::<Result<_>>()?;
        let formats = temporal_formats(root.fields());
        let system = workbook.date_system();
        let stream = move |splice: &mut Splice<'_>| -> Result<SheetReplacement> {
            let written =
                overwrite_table_body(source, rows, &table, columns, sheet.clone(), system, splice)?;
            let original_body_after = table.range.end().row() + 1 - table.totals_rows;
            let changed = written.after != original_body_after;
            let range = super::cell::CellRange::new(
                table.range.start(),
                CellRef::new(
                    written.after + table.totals_rows - 1,
                    table.range.end().column(),
                ),
            );
            let parts = if changed {
                if let Some((other, _)) = siblings
                    .iter()
                    .find(|(_, other_range)| other_range.intersects(range))
                {
                    return Err(Error::InvalidRecord {
                        path: smol_str::format_smolstr!("$.table[{wanted}]"),
                        reason: smol_str::format_smolstr!(
                            "expected a resized extent outside table {other}, got {range}"
                        ),
                    });
                }
                vec![(
                    part.clone(),
                    table.resized_part(&bytes, &part, written.after)?,
                )]
            } else {
                Vec::new()
            };
            Ok(SheetReplacement {
                stream: Box::new(written.xml),
                parts,
            })
        };
        let package = workbook.write_package(Some(Replaced {
            at,
            formats,
            stream: Box::new(stream),
        }))?;
        drop(workbook);
        return handle.write_all_bytes(package.as_bytes());
    }
    let name = options
        .sheet()
        .map(SmolStr::new)
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
        .range()
        .map_or(CellRef::new(0, 0), |range| range.start());
    let workbook_ref = &workbook;
    let header = match options.header {
        RecordHeader::None => WriteHeader::None,
        RecordHeader::Source => WriteHeader::Source,
        RecordHeader::Infer => return Err(super::options::ExcelOptions::write_error()),
        RecordHeader::Rows(levels) => {
            WriteHeader::Rows(RowsWriteLayout::compile(&root, levels, anchor, None)?)
        }
    };
    let sheet = name.clone();
    let failure = Arc::new(std::sync::Mutex::new(None));
    let slot = Arc::clone(&failure);
    let formats = match &header {
        WriteHeader::Rows(layout) => {
            temporal_formats(layout.leaves().iter().map(|leaf| &leaf.field))
        }
        _ => temporal_formats(root.fields()),
    };
    let stream = move |splice: &mut Splice<'_>| -> Result<SheetReplacement> {
        let xml = SheetXml::new(
            rows,
            root,
            sheet,
            workbook_ref,
            splice.temporal_styles(),
            anchor,
            header,
        )?;
        if let Ok(mut held) = slot.lock() {
            *held = Some(xml.failure());
        }
        Ok(SheetReplacement {
            stream: Box::new(xml),
            parts: Vec::new(),
        })
    };
    let replaced = Replaced {
        at,
        formats,
        stream: Box::new(stream),
    };
    let package = match workbook.write_package(Some(replaced)) {
        Ok(package) => package,
        Err(error) => {
            // The archive saw an `io::Error`; the stream kept the refusal.
            let refusal = failure
                .lock()
                .ok()
                .and_then(|slot| slot.as_ref().and_then(|held| held.lock().ok()?.take()));
            return Err(refusal.unwrap_or(error));
        }
    };
    // Release the package mapping before a Windows file is resized.
    drop(workbook);
    handle.write_all_bytes(package.as_bytes())
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
        self.options.set_sheet(Some(sheet.into()));
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
            let workbook = open(&self.handle, self.handle.media_type())?;
            return Ok(Held::Cached(self.cached.get_or_init(|| workbook)));
        }
        open(&self.handle, self.handle.media_type()).map(Held::Fresh)
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
        let declared = self.options.field();
        let Some(name) = addressed(&workbook, &self.options, declared.as_ref())? else {
            return Ok(0);
        };
        count_rows(&workbook, &name)
    }

    fn column_size(&self) -> Result<usize> {
        self.options.require_valid()?;
        let declared = self.options.field();
        if self.options.header != RecordHeader::Infer
            && let Some(field) = &declared
        {
            return Ok(if matches!(self.options.header, RecordHeader::Rows(_)) {
                RowsLayout::leaf_width(field)?
            } else {
                field.field_len()
            });
        }
        if !self.warm() && self.handle.is_container() {
            return Ok(crate::iomedia::container_field(
                &self.handle,
                &crate::iomedia::dimension_options(self)?,
            )?
            .field_len());
        }
        let workbook = self.workbook()?;
        match addressed(&workbook, &self.options, declared.as_ref())? {
            Some(name) => {
                let field = match declared {
                    Some(field) => field,
                    None => inferred(&workbook, &name, &self.options)?,
                };
                Ok(if matches!(name.header, RecordHeader::Rows(_)) {
                    RowsLayout::leaf_width(&field)?
                } else {
                    field.field_len()
                })
            }
            None => Ok(declared.map_or(0, |field| field.field_len())),
        }
    }

    fn record_options(&self) -> Result<RecordOptions> {
        Ok(RecordOptions::Excel(self.options.clone()))
    }

    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        let options = self.require_options(options)?;
        options.require_valid()?;
        if let Some(field) = options.field() {
            return options.result_field(field);
        }
        if !self.warm() && self.handle.is_container() {
            return crate::iomedia::container_field(&self.handle, &options.clone().into());
        }
        let workbook = self.workbook()?;
        let source = match addressed(&workbook, options, None)? {
            Some(name) => inferred(&workbook, &name, options),
            None => super::reader::empty_root(options.name()),
        }?;
        options.result_field(source)
    }

    fn overwrite_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        self.require_options(options)?.require_write()?;
        self.invalidate();
        crate::iobase::overwrite_arrow_reader_default(self, batches, options)
    }

    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        self.require_options(options)?.require_write()?;
        self.invalidate();
        crate::iobase::leaf_writer(self, batches, options)
    }

    fn append_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        self.require_options(options)?.require_write()?;
        self.invalidate();
        crate::iobase::append_arrow_reader_default(self, batches, options)
    }

    fn merge_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        self.require_options(options)?.require_write()?;
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
