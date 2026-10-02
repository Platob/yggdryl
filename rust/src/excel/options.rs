//! The settings a workbook read or write takes.

use smol_str::SmolStr;

use crate::media::IORecordOptions;
use crate::{Field, Filter, Level, Selector};

use super::cell::CellRange;

/// The settings a worksheet is read and written with.
///
/// The shared settings are every record encoding's. A workbook adds which
/// sheet a read or write addresses, whether the first row names the columns,
/// and which cells the rows occupy.
///
/// ```
/// use yggdryl::excel::ExcelOptions;
///
/// let options = ExcelOptions::new()
///     .with_sheet("Trades")
///     .with_range("A3:F".parse()?)
///     .with_header(false);
/// assert_eq!(options.sheet.as_deref(), Some("Trades"));
/// assert!(!options.header);
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ExcelOptions {
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
    /// is the destination's own cadence: a leaf or a folder publishes once,
    /// after the source ends; an Iceberg table each time the held batches
    /// reach its target file size, then the remainder, an overwrite's first
    /// commit replacing and every later one appending while every commit of a
    /// merge merges by its key; a write session by
    /// [`DEFAULT_COMMIT_BYTE_SIZE`](crate::media::DEFAULT_COMMIT_BYTE_SIZE).
    /// The commits completed before a later failure stay published. The rule
    /// is [`IORecordOptions::commit_batch_num`]'s.
    pub commit_batch_num: Option<usize>,
    /// Compression level applied when the handle declares a coding.
    pub level: Level,
    /// The sheet a read or write addresses, compared without case as Excel
    /// compares sheet names; `None` is the first worksheet, and a write into
    /// a workbook with none names its sheet
    /// [`DEFAULT_SHEET_NAME`](super::DEFAULT_SHEET_NAME). A read of a sheet
    /// the workbook lacks is the empty stream, as a read of a missing
    /// resource is; a write adds it.
    pub sheet: Option<SmolStr>,
    /// Whether the first row of the range names the columns; `true` by
    /// default. Without a header the columns are named by their letters.
    pub header: bool,
    /// The cells a read or write addresses; `None` is the whole sheet. A
    /// read takes the rows and columns inside it, the header being its first
    /// row; a write anchors its rows at its top-left cell.
    pub range: Option<CellRange>,
}

impl ExcelOptions {
    /// The default options: the first worksheet, a header row, the whole
    /// sheet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: SmolStr::new_static(crate::media::DEFAULT_ROOT_NAME),
            field: None,
            filter: Filter::always_true(),
            select: Selector::all(),
            merge_by: Selector::all(),
            safe: true,
            batch_byte_size: None,
            batch_row_size: None,
            max_row_size: None,
            row_offset: None,
            max_byte_size: None,
            commit_batch_num: None,
            level: Level::DEFAULT,
            sheet: None,
            header: true,
            range: None,
        }
    }

    /// Return these options addressing the sheet `sheet`.
    #[must_use]
    pub fn with_sheet(mut self, sheet: impl Into<SmolStr>) -> Self {
        self.sheet = Some(sheet.into());
        self
    }

    /// Return these options with or without a header row.
    #[must_use]
    pub const fn with_header(mut self, header: bool) -> Self {
        self.header = header;
        self
    }

    /// Return these options addressing the cells of `range`.
    #[must_use]
    pub const fn with_range(mut self, range: CellRange) -> Self {
        self.range = Some(range);
        self
    }

    /// The cells a read or write addresses: the range, else the whole grid.
    #[must_use]
    pub fn cells(&self) -> CellRange {
        self.range.unwrap_or_else(CellRange::all)
    }
}

impl Default for ExcelOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl IORecordOptions for ExcelOptions {
    crate::record_options_fields!();
}
