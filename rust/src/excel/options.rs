//! The settings a workbook read or write takes.

use smol_str::SmolStr;

use crate::media::IORecordOptions;
use crate::{Field, Filter, Level, RecordHeader, Selector};

use super::cell::CellRange;

/// Which worksheet cells a workbook record operation addresses.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ExcelSelection {
    /// A worksheet and optional range; absent sheet means the first worksheet.
    Worksheet {
        /// Sheet name, compared without ASCII case; absent selects the first.
        sheet: Option<SmolStr>,
        /// Cell bounds; absent selects the whole worksheet.
        range: Option<CellRange>,
    },
    /// One registered OOXML table, compared without ASCII case.
    Table {
        /// The table's workbook-wide display name.
        name: SmolStr,
    },
}

impl ExcelSelection {
    /// Decode one complete selection from a scalar object or null.
    ///
    /// Missing object members retain the default worksheet selection. A
    /// non-null table conflicts with a non-null sheet or range.
    ///
    /// ```
    /// use yggdryl::{from_json_scalar, excel::ExcelSelection};
    /// let selected = ExcelSelection::from_scalar(&from_json_scalar(r#"{"table":"Sales"}"#)?)?;
    /// assert_eq!(selected, ExcelSelection::Table { name: "Sales".into() });
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Refuses malformed members at `$.selection.<member>` and conflicts at
    /// `$.selection.table`, naming the other supplied selector too.
    pub fn from_scalar(value: &crate::Scalar) -> crate::Result<Self> {
        Self::read(
            value,
            &Self::Worksheet {
                sheet: None,
                range: None,
            },
        )
    }

    /// Update this selection from a scalar object or clear it with null.
    ///
    /// Missing members leave the existing value in place. All supplied
    /// members are resolved before the selection changes. Null clears a
    /// property only in its active arm; null for the whole object resets it.
    ///
    /// ```
    /// use yggdryl::{from_json_scalar, excel::ExcelSelection};
    /// let mut selected = ExcelSelection::Table { name: "Sales".into() };
    /// selected.set_from_scalar(&from_json_scalar(r#"{"sheet":"Data"}"#)?)?;
    /// assert_eq!(selected, ExcelSelection::Worksheet { sheet: Some("Data".into()), range: None });
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Refuses malformed or conflicting members without changing `self`.
    pub fn set_from_scalar(&mut self, value: &crate::Scalar) -> crate::Result<()> {
        let selection = Self::read(value, self)?;
        *self = selection;
        Ok(())
    }

    fn read(value: &crate::Scalar, current: &Self) -> crate::Result<Self> {
        use crate::Error;

        let default = || Self::Worksheet {
            sheet: None,
            range: None,
        };
        if value.is_null() {
            return Ok(default());
        }
        let entries = value.as_struct().ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$.selection"),
            reason: smol_str::format_smolstr!("expected an object or null, got {value:?}"),
        })?;
        for key in entries.keys() {
            if !matches!(key.as_str(), "sheet" | "range" | "table") {
                return Err(Error::InvalidRecord {
                    path: smol_str::format_smolstr!("$.selection.{key}"),
                    reason: smol_str::format_smolstr!(
                        "expected sheet, range or table, got {key:?}"
                    ),
                });
            }
        }

        let sheet = entries
            .get("sheet")
            .map(|value| -> crate::Result<Option<SmolStr>> {
                if value.is_null() {
                    return Ok(None);
                }
                let name = value.as_str().ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new_static("$.selection.sheet"),
                    reason: smol_str::format_smolstr!(
                        "expected a sheet name or null, got {value:?}"
                    ),
                })?;
                super::sheet::validate_sheet_name(name).map_err(|error| match error {
                    Error::InvalidRecord { reason, .. } => Error::InvalidRecord {
                        path: SmolStr::new_static("$.selection.sheet"),
                        reason,
                    },
                    error => error,
                })?;
                Ok(Some(SmolStr::new(name)))
            })
            .transpose()?;
        let range = entries
            .get("range")
            .map(|value| {
                if value.is_null() {
                    return Ok(None);
                }
                let text = value.as_str().ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new_static("$.selection.range"),
                    reason: smol_str::format_smolstr!(
                        "expected a cell range or null, got {value:?}"
                    ),
                })?;
                text.parse::<CellRange>()
                    .map(Some)
                    .map_err(|error| Error::InvalidRecord {
                        path: SmolStr::new_static("$.selection.range"),
                        reason: smol_str::format_smolstr!(
                            "expected a valid cell range, got {text:?}: {error}"
                        ),
                    })
            })
            .transpose()?;
        let table = entries
            .get("table")
            .map(|value| {
                if value.is_null() {
                    return Ok(None);
                }
                let name = value.as_str().ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new_static("$.selection.table"),
                    reason: smol_str::format_smolstr!(
                        "expected a table name or null, got {value:?}"
                    ),
                })?;
                if name.is_empty() {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::new_static("$.selection.table"),
                        reason: SmolStr::new_static("expected a nonempty table name"),
                    });
                }
                Ok(Some(SmolStr::new(name)))
            })
            .transpose()?;

        if let Some(Some(name)) = &table {
            if sheet.as_ref().is_some_and(Option::is_some)
                || range.as_ref().is_some_and(Option::is_some)
            {
                let other = match (
                    sheet.as_ref().is_some_and(Option::is_some),
                    range.as_ref().is_some_and(Option::is_some),
                ) {
                    (true, true) => "$.selection.sheet and $.selection.range",
                    (true, false) => "$.selection.sheet",
                    (false, true) => "$.selection.range",
                    (false, false) => unreachable!(),
                };
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$.selection.table"),
                    reason: smol_str::format_smolstr!(
                        "$.selection.table conflicts with {other}: expected one selection arm"
                    ),
                });
            }
            return Ok(Self::Table { name: name.clone() });
        }

        let switch_to_sheet = sheet.as_ref().is_some_and(Option::is_some)
            || range.as_ref().is_some_and(Option::is_some);
        let (mut held_sheet, mut held_range) = match current {
            Self::Worksheet { sheet, range } => (sheet.clone(), *range),
            Self::Table { .. } => (None, None),
        };
        if switch_to_sheet {
            if let Some(sheet) = sheet {
                held_sheet = sheet;
            }
            if let Some(range) = range {
                held_range = range;
            }
            return Ok(Self::Worksheet {
                sheet: held_sheet,
                range: held_range,
            });
        }
        match current {
            Self::Table { .. } if table.is_some() => Ok(default()),
            Self::Table { .. } => Ok(current.clone()),
            Self::Worksheet { .. } => {
                if let Some(sheet) = sheet {
                    held_sheet = sheet;
                }
                if let Some(range) = range {
                    held_range = range;
                }
                Ok(Self::Worksheet {
                    sheet: held_sheet,
                    range: held_range,
                })
            }
        }
    }
}

/// The settings a worksheet is read and written with.
///
/// The shared settings are every record encoding's. A workbook adds which
/// sheet a read or write addresses, whether the first row names the columns,
/// and which cells the rows occupy.
///
/// ```
/// use yggdryl::{RecordHeader, excel::{ExcelOptions}};
///
/// let options = ExcelOptions::new()
///     .with_sheet("Trades")
///     .with_range("A3:F".parse()?)
///     .with_header(RecordHeader::None);
/// assert_eq!(options.sheet(), Some("Trades"));
/// assert_eq!(options.header, RecordHeader::None);
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
    /// Rows published per streamed-write commit; `None` publishes once.
    pub commit_row_size: Option<usize>,
    /// Compression level applied when the handle declares a coding.
    pub level: Level,
    /// The addressed worksheet/range or named table.
    pub selection: ExcelSelection,
    /// How columns are named; Source by default. A named table always
    /// retains the body bounds stated by its own header and totals bands.
    pub header: RecordHeader,
}

impl ExcelOptions {
    pub(super) fn no_evidence_error() -> crate::Error {
        crate::Error::InvalidRecord {
            path: SmolStr::new_static("$.header"),
            reason: SmolStr::new_static(
                "expected selected cell values or labels to infer a header, got no evidence; choose Source, None or Rows(n) explicitly",
            ),
        }
    }

    pub(super) fn write_error() -> crate::Error {
        crate::Error::InvalidRecord {
            path: SmolStr::new_static("$.header"),
            reason: SmolStr::new_static(
                "expected Source, None or Rows(n) for a write, got Infer (read-only header inference)",
            ),
        }
    }

    pub(super) fn ambiguous_error() -> crate::Error {
        crate::Error::InvalidRecord {
            path: SmolStr::new_static("$.header"),
            reason: SmolStr::new_static(
                "could not resolve one header reading from selected cells; choose Source, None or Rows(n) explicitly",
            ),
        }
    }
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
            commit_row_size: None,
            level: Level::DEFAULT,
            selection: ExcelSelection::Worksheet {
                sheet: None,
                range: None,
            },
            header: RecordHeader::Source,
        }
    }

    /// Return these options addressing the sheet `sheet`.
    #[must_use]
    pub fn with_sheet(mut self, sheet: impl Into<SmolStr>) -> Self {
        self.set_sheet(Some(sheet.into()));
        self
    }

    /// Return these options with the column naming policy `header`.
    #[must_use]
    pub fn with_header(mut self, header: impl Into<RecordHeader>) -> Self {
        self.header = header.into();
        self
    }

    /// Return these options addressing the cells of `range`.
    #[must_use]
    pub fn with_range(mut self, range: CellRange) -> Self {
        self.set_range(Some(range));
        self
    }

    /// Return these options selecting one named OOXML table.
    ///
    /// ```
    /// use yggdryl::excel::ExcelOptions;
    /// let options = ExcelOptions::new().with_table("Trades");
    /// assert_eq!(options.table(), Some("Trades"));
    /// ```
    #[must_use]
    pub fn with_table(mut self, name: impl Into<SmolStr>) -> Self {
        self.selection = ExcelSelection::Table { name: name.into() };
        self
    }

    /// The explicit worksheet name, absent for a table or first worksheet.
    #[must_use]
    pub fn sheet(&self) -> Option<&str> {
        match &self.selection {
            ExcelSelection::Worksheet { sheet, .. } => sheet.as_deref(),
            ExcelSelection::Table { .. } => None,
        }
    }

    /// The explicit worksheet range, absent for a table or whole worksheet.
    #[must_use]
    pub const fn range(&self) -> Option<CellRange> {
        match &self.selection {
            ExcelSelection::Worksheet { range, .. } => *range,
            ExcelSelection::Table { .. } => None,
        }
    }

    /// The explicit table name, absent for worksheet selection.
    #[must_use]
    pub fn table(&self) -> Option<&str> {
        match &self.selection {
            ExcelSelection::Table { name } => Some(name),
            ExcelSelection::Worksheet { .. } => None,
        }
    }

    pub(crate) fn set_sheet(&mut self, sheet: Option<SmolStr>) {
        match &mut self.selection {
            ExcelSelection::Worksheet { sheet: held, .. } => *held = sheet,
            ExcelSelection::Table { .. } => {
                if sheet.is_some() {
                    self.selection = ExcelSelection::Worksheet { sheet, range: None };
                }
            }
        }
    }

    pub(crate) fn set_range(&mut self, range: Option<CellRange>) {
        match &mut self.selection {
            ExcelSelection::Worksheet { range: held, .. } => *held = range,
            ExcelSelection::Table { .. } => {
                if range.is_some() {
                    self.selection = ExcelSelection::Worksheet { sheet: None, range };
                }
            }
        }
    }

    /// Set a complete selection only when it agrees with the header policy.
    pub(crate) fn set_selection(&mut self, selection: ExcelSelection) -> crate::Result<()> {
        let previous = std::mem::replace(&mut self.selection, selection);
        if let Err(error) = self.require_valid() {
            self.selection = previous;
            return Err(match error {
                crate::Error::InvalidRecord { path, reason } => match path.as_str() {
                    "$.sheet" => crate::Error::InvalidRecord {
                        path: SmolStr::new_static("$.selection.sheet"),
                        reason,
                    },
                    "$.range" => crate::Error::InvalidRecord {
                        path: SmolStr::new_static("$.selection.range"),
                        reason,
                    },
                    _ => crate::Error::InvalidRecord { path, reason },
                },
                error => error,
            });
        }
        Ok(())
    }

    /// Reject invalid directly constructed selection and header before cell work.
    pub(crate) fn require_valid(&self) -> crate::Result<()> {
        if let ExcelSelection::Worksheet { sheet, range } = &self.selection {
            if let Some(name) = sheet {
                super::sheet::validate_sheet_name(name)?;
            }
            if let Some(range) = range {
                for corner in [range.start(), range.end()] {
                    corner.require_in_grid().map_err(|error| match error {
                        crate::Error::InvalidRecord { reason, .. } => crate::Error::InvalidRecord {
                            path: SmolStr::new_static("$.range"),
                            reason,
                        },
                        error => error,
                    })?;
                }
            }
        }
        if matches!(&self.selection, ExcelSelection::Table { name } if name.is_empty()) {
            return Err(crate::Error::InvalidRecord {
                path: SmolStr::new_static("$.selection.table"),
                reason: SmolStr::new_static("expected a nonempty table name"),
            });
        }
        if let RecordHeader::Rows(levels) = self.header {
            if !(1..=super::cell::MAX_ROWS).contains(&levels) {
                return Err(crate::Error::InvalidRecord {
                    path: SmolStr::new_static("$.header"),
                    reason: smol_str::format_smolstr!(
                        "expected a physical header count from 1 to {}, got {levels}",
                        super::cell::MAX_ROWS
                    ),
                });
            }
            if let Some(name) = self.table() {
                return Err(crate::Error::InvalidRecord {
                    path: SmolStr::new_static("$.header"),
                    reason: smol_str::format_smolstr!(
                        "expected authoritative tableColumn names for table {name:?}, got Rows({levels})"
                    ),
                });
            }
        }
        Ok(())
    }

    /// Resolve only write policies that cannot describe output cells.
    pub(crate) fn require_write(&self) -> crate::Result<()> {
        self.require_valid()?;
        if self.header == RecordHeader::Infer {
            return Err(Self::write_error());
        }
        Ok(())
    }

    /// The addressed worksheet cells, the whole grid when no range is stated.
    ///
    /// # Errors
    ///
    /// A named table requires its workbook to resolve its extent.
    pub fn cells(&self) -> crate::Result<CellRange> {
        match &self.selection {
            ExcelSelection::Worksheet { range, .. } => Ok(range.unwrap_or_else(CellRange::all)),
            ExcelSelection::Table { name } => Err(crate::Error::InvalidRecord {
                path: SmolStr::new_static("$.table"),
                reason: smol_str::format_smolstr!("expected a workbook to resolve table {name:?}"),
            }),
        }
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
