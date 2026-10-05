//! Office Open XML spreadsheets (`.xlsx`): a workbook of worksheets as
//! record media, and as cells a caller reaches one by one.
//!
//! A workbook is a ZIP archive of XML parts, read through the crate's own
//! [`zip`](crate::zip) backend and [`xml`](crate::xml) codec, wired by the
//! Open Packaging Conventions in [`package`]. Its rows are the medium's
//! records, read and written through the same [`IOMedia`](crate::IOMedia)
//! calls as every other encoding - an `.xlsx` handle picks the medium as
//! `.parquet` picks Parquet - and its cells are [`Workbook`], [`Sheet`] and
//! [`Cell`], the random-access model over the same parts.
//!
//! | Layer | Owns |
//! | --- | --- |
//! | [`cell`] | `CellRef` and `CellRange` (the A1 grammar), `CellKind`, `ExcelError`, `Cell`, and `DateSystem`, the serial-date rule both ways |
//! | [`formula`] | `Formula`: what a cell's `<f>` states, one shared shape per formula - references relative to the cell holding it - in its file and its entry spelling |
//! | [`layout`] | `Frozen` and what a worksheet states about its grid beside its cells: row and column formats, merges, the frozen pane, the defaults |
//! | [`style`] | `StyleId`, the `cellXfs` index a cell's `s` states, `CellStyle` with the font, fill, border, alignment and protection it resolves to, and `StylePatch`, the change the ribbon makes to the cells of some ranges |
//! | [`styles`] | `StyleSheet`: the styles part as a model, appended to and never reordered; `NumberFormat`, which cells hold a date, a time or a duration |
//! | [`entry`] | `Entry`: the text a user types into a cell, read as en-US Excel reads it |
//! | [`format`](mod@format) | `FormatCode`: a number format code read once, what it says a number is, and `Rendered`, the text a value displays as under it |
//! | [`theme`] | `Theme`: the colours of the workbook's theme, which a style's theme colours index |
//! | [`shared_strings`] | the shared string table, read and written, with the `_xHHHH_` escape a string cell crosses under |
//! | [`package`] | the Open Packaging Conventions: content types, relationships, part names |
//! | [`sheet`] | `Sheet`, `Row` and `CellMut`: every cell of one worksheet, reachable by reference, laid out as a `Serie` or built from one, and what its part states outside its cells carried as it was written |
//! | [`workbook`] | `Workbook`, `SheetKey` and `Package`: the sheets of one package, parsed on demand, saved by writing what changed and copying every other member as it is stored; the structural verbs - rows and columns inserted and removed, sheets renamed, moved and removed - every reference in the package following, and the range verbs: clear, paste, paste text, sort |
//! | [`edit`] | `Edit`, one change of a workbook as a value, applied all or nothing by `Workbook::apply`, which answers `Applied` with the edit undoing it; `Clear`, `Paste`, `SortKey`, `Landing`, and `Restore`/`RestoreSheet`/`RestoreBand`, what an undo puts back that the opposite edit does not |
//! | [`fill`] | `FillMode` and `Workbook::fill`: AutoFill, as the series a source spells or as copies of it |
//! | [`find`] | `FindOptions`, `FindScope` and `Within`: Find Next and Replace All over one pattern of Excel's wildcards |
//! | [`journal`] | `Journal`: undo and redo, bounded in edits and in bytes |
//! | [`names`] | `DefinedName`: the workbook's defined names, carried as written until a sheet they name is renamed or removed |
//! | [`options`], [`media`] | the `.xlsx` record medium: [`ExcelOptions`] and [`Excel`] |
//!
//! ```
//! use yggdryl::holder::Buffer;
//! use yggdryl::media::IORecordOptions;
//! use yggdryl::{DataType, IOBase, IOMedia, MimeType, Scalar, StructType};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let field = DataType::from(StructType::from_fields([
//!     DataType::Int64.required_field("id"),
//!     DataType::utf8().nullable_field("symbol"),
//! ])?)
//! .required_field("row");
//! let mut handle = Buffer::new().with_media_type(MimeType::XLSX.into());
//! let options = handle.record_options()?.with_field(field.clone());
//! handle.overwrite_records(
//!     [
//!         Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]),
//!         Scalar::from_sequence([Scalar::from(2_i64), Scalar::Null]),
//!     ],
//!     &options,
//! )?;
//!
//! // The workbook opens by cell as well as by row.
//! let workbook = yggdryl::excel::Workbook::from_bytes(handle.read_all_bytes()?)?;
//! let sheet = workbook.sheet("Sheet1")?;
//! assert_eq!(sheet.scalar("A1".parse()?), Scalar::from("id"));
//! assert_eq!(sheet.scalar("B2".parse()?), Scalar::from("AAPL"));
//!
//! let mut rows = 0;
//! for batch in handle.read_arrow_reader(&handle.record_options()?)? {
//!     rows += batch?.num_rows();
//! }
//! assert_eq!(rows, 2);
//! # Ok(())
//! # }
//! ```

pub(crate) mod carried;
pub mod cell;
pub mod edit;
pub mod entry;
pub mod fill;
pub mod find;
pub mod format;
pub mod formula;
pub mod journal;
pub mod layout;
pub mod media;
pub mod names;
pub mod options;
pub mod package;
pub(crate) mod parser;
pub mod pivot;
pub(crate) mod reader;
pub(crate) mod records;
pub(crate) mod regions;
pub mod shared_strings;
pub mod sheet;
pub(crate) mod shift;
pub mod style;
pub mod styles;
pub(crate) mod table;
pub mod theme;
pub mod workbook;
pub(crate) mod writer;

pub use cell::{
    Cell, CellKind, CellRange, CellRef, DateSystem, ExcelError, MAX_CELL_TEXT, MAX_COLUMNS,
    MAX_ROWS,
};
pub use edit::{
    Applied, Clear, Edit, Landing, MAX_EDITED_CELLS, Paste, Restore, RestoreBand, RestoreSheet,
    SortKey,
};
pub use entry::Entry;
pub use fill::FillMode;
pub use find::{FindOptions, FindScope, Within};
pub use format::{FormatCode, Rendered};
pub use formula::aggregate::Aggregate;
pub use formula::{
    Clock, Formula, FunctionDescriptor, MAX_FORMULA_LENGTH, MAX_FORMULA_NESTING, Recalculation,
};
pub use journal::{DEFAULT_JOURNAL_BYTES, DEFAULT_JOURNAL_ENTRIES, Journal};
pub use layout::{DEFAULT_COLUMN_WIDTH, DEFAULT_ROW_HEIGHT, Frozen};
pub use media::{Excel, overwrite_arrow_reader, read_batch_reader, read_field, regions};
pub(crate) use media::{row_size, stated_field};
pub use names::DefinedName;
pub use options::{ExcelOptions, ExcelSelection};
pub use pivot::{
    AxisField, ItemOrder, PivotFieldInfo, PivotOrigin, PivotSource, PivotSpec, PivotTable,
    ValueField,
};
pub use regions::{ExcelRegion, ExcelRegionKind};
pub use sheet::{CellMut, Direction, MAX_SHEET_NAME, Row, Sheet, SheetState, validate_sheet_name};
pub use style::{
    Alignment, Border, BorderPreset, BorderStyle, Borders, CellStyle, Color, Edge, Fill, Font,
    FontScheme, Horizontal, PatternType, Protection, StyleId, StylePatch, Underline, Vertical,
    VerticalRun,
};
pub use styles::{MAX_CELL_FORMATS, NumberFormat, StyleSheet};
pub use theme::Theme;
pub use workbook::{Package, SheetKey, SheetKind, Workbook};

/// The Transitional SpreadsheetML namespace used by a new workbook.
pub const NAMESPACE: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

/// The SpreadsheetML namespace retained by a "Strict Open XML" workbook.
pub const STRICT_NAMESPACE: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";

/// The namespace of the `r:` relationship attributes a part carries.
pub const RELATIONSHIPS_NAMESPACE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The strict counterpart of [`RELATIONSHIPS_NAMESPACE`].
pub const STRICT_RELATIONSHIPS_NAMESPACE: &str =
    "http://purl.oclc.org/ooxml/officeDocument/relationships";

/// The sheet a write creates when the options name none.
pub const DEFAULT_SHEET_NAME: &str = "Sheet1";

/// Refuse a handle whose media type declares a content coding.
///
/// A workbook is a ZIP package deflated inside, so `trades.xlsx.gz` names a
/// file no spreadsheet opens: every door that reads or writes the package
/// refuses the name before a byte crosses, as Parquet's do, and the holder
/// leaves the coding undecoded so the refusal is the answer a caller gets.
pub(crate) fn reject_outer_coding(media_type: &crate::MediaType) -> crate::Result<()> {
    let codec = crate::Codec::from_media_type(media_type);
    if codec.is_identity() {
        return Ok(());
    }
    Err(crate::Error::Codec {
        format: "xlsx",
        position: 0,
        reason: smol_str::format_smolstr!(
            "expected an uncompressed xlsx handle, got {codec} coding; a workbook is a ZIP \
             package deflated inside, so drop the {codec} coding from its name"
        ),
    })
}
