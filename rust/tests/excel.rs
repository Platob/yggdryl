//! One test file per file under `rust/src/excel/`, under `tests/excel/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the Excel medium -
//! the workbook, its sheets and cells, and the record path over a worksheet.
//! A test reaches the crate through `yggdryl::` and nothing else; a file
//! pinning what a caller cannot name reaches `yggdryl::internals` in its own
//! `internal` module, which exists only under the `internals` feature.

#[path = "support/excel_package.rs"]
mod excel_package;

#[path = "excel/carried.rs"]
mod carried;
#[path = "excel/cell.rs"]
mod cell;
#[path = "excel/edit.rs"]
mod edit;
#[path = "excel/entry.rs"]
mod entry;
#[path = "excel/fill.rs"]
mod fill;
#[path = "excel/find.rs"]
mod find;
#[path = "excel/format.rs"]
mod format;
#[path = "excel/formula.rs"]
mod formula;
#[path = "excel/journal.rs"]
mod journal;
#[path = "excel/layout.rs"]
mod layout;
#[path = "excel/media.rs"]
mod media;
#[path = "excel/mod_.rs"]
mod mod_;
#[path = "excel/names.rs"]
mod names;
#[path = "excel/options.rs"]
mod options;
#[path = "excel/package.rs"]
mod package;
#[path = "excel/parser.rs"]
mod parser;
#[path = "excel/reader.rs"]
mod reader;
#[path = "excel/records.rs"]
mod records;
#[path = "excel/regions.rs"]
mod regions;
#[path = "excel/shared_strings.rs"]
mod shared_strings;
#[path = "excel/sheet.rs"]
mod sheet;
#[path = "excel/shift.rs"]
mod shift;
#[path = "excel/style.rs"]
mod style;
#[path = "excel/styles.rs"]
mod styles;
#[path = "excel/table.rs"]
mod table;
#[path = "excel/theme.rs"]
mod theme;
#[path = "excel/workbook.rs"]
mod workbook;
#[path = "excel/writer.rs"]
mod writer;
