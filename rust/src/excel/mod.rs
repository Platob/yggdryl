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
//! | [`cell`] | `CellRef` and `CellRange` (the A1 grammar), `CellKind`, `Cell`, and `DateSystem`, the serial-date rule both ways |
//! | [`styles`] | `NumberFormat`: which cells hold a date, a time or a duration, and the styles part this crate writes |
//! | [`shared_strings`] | the shared string table, read and written, with the `_xHHHH_` escape a string cell crosses under |
//! | [`package`] | the Open Packaging Conventions: content types, relationships, part names |
//! | [`sheet`] | `Sheet` and `Row`: every cell of one worksheet, reachable by reference, laid out as a `Serie` or built from one |
//! | [`workbook`] | `Workbook`: the sheets of one package, parsed on demand, written back with every other part carried over |
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

pub mod cell;
pub mod media;
pub mod options;
pub mod package;
pub(crate) mod parser;
pub(crate) mod reader;
pub mod shared_strings;
pub mod sheet;
pub mod styles;
pub mod workbook;
pub(crate) mod writer;

pub use cell::{
    Cell, CellKind, CellRange, CellRef, DateSystem, MAX_CELL_TEXT, MAX_COLUMNS, MAX_ROWS,
};
pub use media::{Excel, overwrite_arrow_reader, read_batch_reader, read_field};
pub(crate) use media::{row_size, stated_field};
pub use options::ExcelOptions;
pub use sheet::{MAX_SHEET_NAME, Row, Sheet, SheetState, validate_sheet_name};
pub use styles::NumberFormat;
pub use workbook::{SheetKind, Workbook};

/// The SpreadsheetML namespace every part is written in.
pub const NAMESPACE: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

/// The SpreadsheetML namespace of a "Strict Open XML" workbook, read alike.
pub const STRICT_NAMESPACE: &str = "http://purl.oclc.org/ooxml/spreadsheetml/main";

/// The namespace of the `r:` relationship attributes a part carries.
pub const RELATIONSHIPS_NAMESPACE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The strict counterpart of [`RELATIONSHIPS_NAMESPACE`].
pub const STRICT_RELATIONSHIPS_NAMESPACE: &str =
    "http://purl.oclc.org/ooxml/officeDocument/relationships";

/// The sheet a write creates when the options name none.
pub const DEFAULT_SHEET_NAME: &str = "Sheet1";
