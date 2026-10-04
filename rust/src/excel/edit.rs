//! Edits as values: what a caller - the workbook service, a binding, a
//! test - asks of a [`Workbook`], applied atomically and answered with the
//! edit that undoes it.
//!
//! An [`Edit`] does no I/O and names sheets by name; [`Edit::from_scalar`]
//! reads the JSON shape the service takes, sheets by key, and
//! [`Workbook::apply`] validates, mutates and answers an [`Applied`]
//! carrying the inverse. An inverse is the opposite edit where one exists
//! (rows removed for rows inserted, a rename back) plus a [`Restore`] of
//! everything that opposite would not give back: the cells a removal took,
//! a formula that became `#REF!`, a range a removal shrank, a part it
//! rewrote. Applying the inverse gives back exactly what the edit found;
//! applying the edit again gives back exactly what it made.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result, Scalar};

use super::carried::WorksheetFrame;
use super::cell::{Cell, CellRange, CellRef, MAX_COLUMNS, MAX_ROWS};
use super::fill::FillMode;
use super::find::{FindOptions, FindScope, Within};
use super::formula::Formula;
use super::layout::{ColumnFormat, Frozen, RowFormats};
use super::names::DefinedName;
use super::parser::FormulaAttributes;
use super::pivot::PivotSpec;
use super::sheet::{Sheet, SheetState};
use super::shift::{Axis, Band};
use super::style::{
    BorderPreset, BorderStyle, Borders, Color, Horizontal, StyleId, StylePatch, Underline, Vertical,
};
use super::styles::{StyleBindings, StyleSheet};
use super::workbook::{RemovedParts, SheetKey, Slot, View, Workbook};

/// What an undo puts back that the opposite edit does not: an opaque record
/// only an [`Applied::inverse`] carries, refused on the wire.
#[derive(Clone, Debug)]
pub struct Restore {
    /// The workbook whose sheet keys, package parts and original XFs these are.
    workbook: u64,
    pub(crate) steps: Vec<Step>,
    // The authored operation proves whether its inverse affects values.
    calculation_relevant: bool,
    // A calculated undo binds its authored inverse to this same receipt.
    // Public Batch composition cannot detach one from the other.
    paired: Option<Box<Edit>>,
    paired_before: bool,
    /// Meanings of the distinct appended styles retained by these steps.
    pub(crate) styles: StyleBindings,
}

impl Restore {
    /// An empty inverse bound to the workbook that captures it.
    pub(crate) fn new(workbook: &Workbook) -> Self {
        Self {
            workbook: workbook.id,
            steps: Vec::new(),
            calculation_relevant: true,
            paired: None,
            paired_before: false,
            styles: StyleBindings::default(),
        }
    }

    /// Refuse workbook-local identities before any target read or mutation.
    pub(crate) fn check_origin(&self, workbook: &Workbook) -> Result<()> {
        workbook.check_undo_origin(self.workbook)?;
        if let Some(paired) = self.paired.as_deref() {
            paired.check_origin(workbook)?;
        }
        Ok(())
    }

    /// Whether it puts nothing back.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty() && self.paired.is_none()
    }

    /// A calculated cache receipt is the one inverse that restores a prior
    /// pass instead of scheduling another one.
    fn restores_calculation(&self) -> bool {
        self.steps
            .iter()
            .any(|step| matches!(step, Step::Calculation { .. }))
    }

    /// An estimate of the bytes it holds, which the journal's bound counts.
    #[must_use]
    pub fn byte_size(&self) -> usize {
        self.steps.iter().map(Step::byte_size).sum::<usize>()
            + self.styles.byte_size()
            + self.paired.as_ref().map_or(0, |edit| edit.byte_size())
            + std::mem::size_of::<u64>()
            + std::mem::size_of::<bool>()
    }

    pub(crate) fn push(&mut self, step: Step) {
        self.steps.push(step);
    }

    /// Capture the table's meanings before an edit changes any payload.
    /// Empty styled rows and columns carry IDs independently of cells.
    pub(crate) fn capture_styles(&mut self, table: &StyleSheet) {
        if table.bindings().next().is_none() {
            return;
        }
        for step in &self.steps {
            match step {
                Step::Cells { slice, .. } => {
                    for id in slice.style_ids() {
                        self.styles.capture(table, id);
                    }
                }
                Step::Layout { rows, columns, .. } => {
                    let ids = rows
                        .iter()
                        .flat_map(|(_, rows)| rows.iter().filter_map(|(_, format)| format.style))
                        .chain(
                            columns
                                .iter()
                                .flatten()
                                .filter_map(|(_, format)| format.style),
                        );
                    for id in ids {
                        self.styles.capture(table, id);
                    }
                }
                _ => {}
            }
        }
    }

    /// Apply a preflighted map to owned inverse data, without changing any
    /// revision. The caller publishes it with the prospective style table.
    pub(crate) fn remap_styles(&mut self, moved: &HashMap<StyleId, StyleId>) {
        if moved.is_empty() {
            return;
        }
        for step in &mut self.steps {
            match step {
                Step::Cells { slice, .. } => slice.remap_styles(moved),
                Step::Layout { rows, columns, .. } => {
                    let styles = rows
                        .iter_mut()
                        .flat_map(|(_, rows)| rows.iter_mut().map(|(_, format)| &mut format.style))
                        .chain(
                            columns
                                .iter_mut()
                                .flatten()
                                .map(|(_, format)| &mut format.style),
                        );
                    for style in styles {
                        if let Some(next) = style.and_then(|id| moved.get(&id)) {
                            *style = Some(*next);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// The structural prefix of an undo: a band restored without inheriting
/// adjacent cell formats. The retained payload restores its own formats,
/// so deriving temporary styles here would grow the table during undo.
/// Only an [`Applied::inverse`] constructs this workbook-bound record.
#[derive(Clone, Debug)]
pub struct RestoreBand {
    workbook: u64,
    sheet: SheetKey,
    axis: Axis,
    band: Band,
}

/// What puts back a sheet an edit removed: the tab where it stood, its
/// part and cells, the names and views the removal changed, and the
/// references it made `#REF!`. An opaque record only an
/// [`Applied::inverse`] carries, refused on the wire.
#[derive(Clone, Debug)]
pub struct RestoreSheet {
    pub(crate) position: usize,
    pub(crate) slot: Slot,
    pub(crate) names: Vec<DefinedName>,
    pub(crate) names_touched: bool,
    pub(crate) views: Vec<View>,
    pub(crate) views_touched: bool,
    pub(crate) restore: Restore,
    pub(crate) parts: RemovedParts,
}

/// One retained package transition. Only ownership documents retain an
/// expected image. Ordinary payloads check presence; their owner changes are
/// guarded by the relationship documents retained in the same operation.
#[derive(Clone, Debug)]
pub(crate) struct PartRestore {
    pub(crate) member: SmolStr,
    pub(crate) bytes: Option<Arc<[u8]>>,
    expected: ExpectedPart,
}

#[derive(Clone, Debug)]
enum ExpectedPart {
    Absent,
    Present,
    Relationships(Arc<[u8]>),
}

impl PartRestore {
    pub(crate) fn new(
        member: SmolStr,
        bytes: Option<Arc<[u8]>>,
        expected: Option<Arc<[u8]>>,
    ) -> Self {
        let expected = match expected {
            None => ExpectedPart::Absent,
            Some(bytes) if super::package::source_of_relationships(&member).is_some() => {
                ExpectedPart::Relationships(bytes)
            }
            Some(_) => ExpectedPart::Present,
        };
        Self {
            member,
            bytes,
            expected,
        }
    }

    fn byte_size(&self) -> usize {
        self.member.len()
            + self.bytes.as_ref().map_or(0, |bytes| bytes.len())
            + match &self.expected {
                ExpectedPart::Relationships(bytes)
                    if !self
                        .bytes
                        .as_ref()
                        .is_some_and(|target| Arc::ptr_eq(target, bytes)) =>
                {
                    bytes.len()
                }
                _ => 0,
            }
            + std::mem::size_of::<ExpectedPart>()
    }

    /// Validate against the image already read for inverse capture. The
    /// adjusted target stays private until all other restore checks pass.
    fn prepare(&mut self, workbook: &Workbook, current: Option<&Arc<[u8]>>) -> Result<()> {
        let expected_present = !matches!(self.expected, ExpectedPart::Absent);
        if expected_present != current.is_some() {
            return Err(Error::Conflict {
                expected: "the retained package part's presence",
                actual: "a package identity created or removed since this inverse",
                path: self.member.clone(),
            });
        }
        if let (ExpectedPart::Relationships(expected), Some(current)) = (&self.expected, current) {
            if expected.as_ref() != current.as_ref() {
                self.bytes = workbook.restored_relationships(
                    &self.member,
                    expected,
                    current,
                    self.bytes.as_deref(),
                )?;
            }
        }
        Ok(())
    }
}

/// One thing a [`Restore`] puts back.
#[derive(Clone, Debug)]
pub(crate) enum Step {
    /// The cells of `ranges` of the sheet `key`, as `slice` holds them,
    /// with what the sheet held beside them.
    Cells {
        key: SheetKey,
        ranges: Vec<CellRange>,
        slice: Box<Sheet>,
    },
    /// The formula each cell held.
    Formulas {
        key: SheetKey,
        cells: Vec<(CellRef, Option<Formula>)>,
    },
    /// What each array formula or data table stated beside its anchor.
    FormulaAttributes {
        key: SheetKey,
        cells: Vec<(CellRef, Box<FormulaAttributes>)>,
    },
    /// The formats of the rows of `rows`, the columns' spans, the merges
    /// and the frozen pane - each where stated.
    Layout {
        key: SheetKey,
        rows: Option<RowFormats>,
        columns: Option<Vec<(Range<u32>, ColumnFormat)>>,
        merges: Option<Vec<CellRange>>,
        pane: Option<Option<Frozen>>,
    },
    /// The sheet's in-memory addressed record rectangle, including nulls.
    RecordFootprint {
        key: SheetKey,
        span: Option<CellRange>,
    },
    /// The complete worksheet envelope, or its original absence.
    Frame {
        key: SheetKey,
        frame: Option<Box<WorksheetFrame>>,
    },
    /// The defined names.
    Names(Vec<DefinedName>),
    /// The workbook views a restored sheet overwrote.
    Views(Vec<View>),
    /// The presence and bytes each part beside the sheets held: what an
    /// undo restores, whatever a save made of the member since.
    Overrides(Vec<PartRestore>),
    /// The exact clock ordinal and last published formula status. Derived
    /// graph edges are rebuilt after restoration, not retained in an undo.
    Calculation {
        pass: u64,
        status: Option<super::formula::Recalculation>,
    },
}

impl Step {
    fn byte_size(&self) -> usize {
        const CELL: usize = 96;
        match self {
            Self::Cells { slice, .. } => {
                slice.cell_count() * CELL + slice.len() * 64 + std::mem::size_of::<Sheet>()
            }
            Self::Formulas { cells, .. } => cells.len() * 32,
            Self::FormulaAttributes { cells, .. } => cells.len() * 160,
            Self::Layout {
                rows,
                columns,
                merges,
                ..
            } => {
                rows.as_ref().map_or(0, |(_, rows)| rows.len() * 80)
                    + columns.as_ref().map_or(0, |columns| columns.len() * 64)
                    + merges.as_ref().map_or(0, |merges| merges.len() * 16)
                    + 64
            }
            Self::RecordFootprint { .. } => std::mem::size_of::<Option<CellRange>>(),
            Self::Frame { frame, .. } => frame.as_ref().map_or(0, |frame| frame.byte_size()),
            Self::Names(names) => names.len() * 256,
            Self::Views(views) => views.len() * std::mem::size_of::<View>(),
            Self::Overrides(parts) => parts.iter().map(PartRestore::byte_size).sum(),
            Self::Calculation { status, .. } => {
                std::mem::size_of::<u64>()
                    + status.as_ref().map_or(0, |status| {
                        std::mem::size_of::<super::formula::Recalculation>()
                            + status.circular.len()
                                * (std::mem::size_of::<SmolStr>() + std::mem::size_of::<CellRef>())
                    })
            }
        }
    }
}

/// What [`Workbook::clear`] takes out of the cells of a range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Clear {
    /// Content and format.
    #[default]
    All,
    /// Content, keeping the format: what Delete does.
    Contents,
    /// Format, keeping the value.
    Formats,
}

impl Clear {
    /// The choice as the service spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Contents => "contents",
            Self::Formats => "formats",
        }
    }
}

/// What [`Workbook::paste`] puts of each cell it pastes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Paste {
    /// The cell whole: value or formula, and format.
    #[default]
    All,
    /// The value it shows, a formula's result included.
    Values,
    /// Its formula, or its value where it holds none.
    Formulas,
    /// Its format alone: the Format Painter.
    Formats,
}

impl Paste {
    /// The choice as the service spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Values => "values",
            Self::Formulas => "formulas",
            Self::Formats => "formats",
        }
    }
}

/// One key of a [`Workbook::sort`]: a column of the sheet, zero-based, and
/// its order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SortKey {
    /// The column of the sheet the rows are ordered by, zero-based; one of
    /// the sorted range's columns.
    pub column: u32,
    /// Whether the order runs from the largest down - blanks stay last
    /// either way.
    pub descending: bool,
}

/// Where cells read from elsewhere land: a sheet of their own, or the cells
/// of a sheet from an anchor.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Landing {
    /// A new worksheet of this name, after the last tab.
    NewSheet(SmolStr),
    /// The worksheet `sheet`, the first cell at `anchor`.
    At { sheet: SmolStr, anchor: CellRef },
}

/// One edit of a workbook, as a value: sheets named by name, ranges
/// resolved. [`Workbook::apply`] applies one atomically and answers the
/// edit that undoes it.
#[derive(Clone, Debug)]
pub enum Edit {
    /// Type each text into its cell ([`Workbook::set_entry`]).
    SetEntries {
        sheet: SmolStr,
        entries: Vec<(CellRef, SmolStr)>,
    },
    /// Type `text` into every cell of `ranges` as Ctrl+Enter does: typed at
    /// `at`, the active cell, and a formula translated from there to each.
    FillEntry {
        sheet: SmolStr,
        ranges: Vec<CellRange>,
        text: SmolStr,
        at: CellRef,
    },
    /// [`Workbook::clear`].
    Clear {
        sheet: SmolStr,
        ranges: Vec<CellRange>,
        what: Clear,
    },
    /// [`Workbook::set_style`].
    SetStyle {
        sheet: SmolStr,
        ranges: Vec<CellRange>,
        patch: StylePatch,
    },
    /// [`Workbook::insert_rows`].
    InsertRows { sheet: SmolStr, at: u32, count: u32 },
    /// [`Workbook::remove_rows`], `count` rows from `start`.
    RemoveRows {
        sheet: SmolStr,
        start: u32,
        count: u32,
    },
    /// [`Workbook::insert_columns`].
    InsertColumns { sheet: SmolStr, at: u32, count: u32 },
    /// [`Workbook::remove_columns`], `count` columns from `start`.
    RemoveColumns {
        sheet: SmolStr,
        start: u32,
        count: u32,
    },
    /// [`Sheet::set_row_height`] of `count` rows from `start`.
    RowHeight {
        sheet: SmolStr,
        start: u32,
        count: u32,
        height: Option<f64>,
    },
    /// [`Sheet::set_column_width`] of `count` columns from `start`.
    ColumnWidth {
        sheet: SmolStr,
        start: u32,
        count: u32,
        width: Option<f64>,
    },
    /// [`Sheet::set_rows_hidden`] of `count` rows from `start`.
    HideRows {
        sheet: SmolStr,
        start: u32,
        count: u32,
        hidden: bool,
    },
    /// [`Sheet::set_columns_hidden`] of `count` columns from `start`.
    HideColumns {
        sheet: SmolStr,
        start: u32,
        count: u32,
        hidden: bool,
    },
    /// [`Sheet::merge`]: the range whole, or - `across` - each of its rows;
    /// with `center`, the merged cells' content centred.
    Merge {
        sheet: SmolStr,
        range: CellRange,
        center: bool,
        across: bool,
    },
    /// [`Sheet::unmerge`].
    Unmerge { sheet: SmolStr, range: CellRange },
    /// [`Sheet::set_frozen`].
    Freeze {
        sheet: SmolStr,
        frozen: Option<Frozen>,
    },
    /// [`Workbook::fill`].
    Fill {
        sheet: SmolStr,
        source: CellRange,
        target: CellRange,
        mode: FillMode,
    },
    /// [`Workbook::sort`].
    Sort {
        sheet: SmolStr,
        range: CellRange,
        keys: Vec<SortKey>,
        header: bool,
    },
    /// [`Workbook::paste`].
    Paste {
        from: (SmolStr, CellRange),
        to: (SmolStr, CellRef),
        what: Paste,
        cut: bool,
    },
    /// [`Workbook::paste_text`].
    PasteText {
        sheet: SmolStr,
        anchor: CellRef,
        text: SmolStr,
    },
    /// [`Workbook::replace`].
    Replace {
        options: FindOptions,
        replacement: SmolStr,
    },
    /// A worksheet added - named `SheetN` for no name - at tab `at`, after
    /// the last tab for none.
    AddSheet {
        name: Option<SmolStr>,
        at: Option<usize>,
    },
    /// [`Workbook::rename_sheet`].
    RenameSheet { name: SmolStr, to: SmolStr },
    /// [`Workbook::remove_sheet`].
    RemoveSheet { name: SmolStr },
    /// [`Workbook::move_sheet`].
    MoveSheet { name: SmolStr, to: usize },
    /// Show or hide a sheet's tab.
    SheetState { name: SmolStr, state: SheetState },
    /// Cells read from elsewhere - Get Data - landed at `destination`.
    Land {
        destination: Landing,
        cells: Box<Sheet>,
    },
    /// Create a tabular pivot at `anchor` in an existing worksheet.
    PivotCreate {
        spec: PivotSpec,
        sheet: SmolStr,
        anchor: CellRef,
    },
    /// Recompute one editable pivot from its authored source.
    PivotRefresh { sheet: SmolStr, name: SmolStr },
    /// Refresh every pivot in package order as one atomic edit and inverse.
    PivotRefreshAll,
    /// Change one editable pivot specification while retaining its part identity.
    PivotUpdate {
        sheet: SmolStr,
        name: SmolStr,
        spec: PivotSpec,
    },
    /// Remove one pivot's output and its last-owner package parts.
    PivotRemove { sheet: SmolStr, name: SmolStr },
    /// Every edit in order, as one: all or none, one undo.
    Batch(Vec<Edit>),
    /// Put each cell - or take it out, for `None`: an inverse only, refused
    /// on the wire.
    SetCells {
        sheet: SmolStr,
        cells: Vec<(CellRef, Option<Cell>)>,
    },
    /// Put back what an edit changed that its opposite does not give back:
    /// an inverse only, refused on the wire.
    Restore(Box<Restore>),
    /// Put back a sheet an edit removed: an inverse only, refused on the
    /// wire.
    RestoreSheet(Box<RestoreSheet>),
    /// Restore a structural band before its retained payload: an inverse
    /// only, refused on the wire.
    RestoreBand(RestoreBand),
}

/// What [`Workbook::apply`] did: the edit undoing it, the cells it touched,
/// what else it changed, and what it answers.
#[derive(Debug)]
pub struct Applied {
    /// The edit that gives back what the edit found.
    pub inverse: Option<Edit>,
    /// The ranges the edit changed, by sheet.
    pub touched: Vec<(SmolStr, CellRange)>,
    /// Whether rows or columns opened or closed: every cell of the sheet may
    /// have moved.
    pub structural: bool,
    /// Whether the sheet list changed.
    pub sheets: bool,
    /// Whether styles were appended.
    pub styles: bool,
    /// An estimate of the bytes the inverse holds.
    pub bytes: usize,
    /// Formula work and current held/circular status after this transaction.
    /// An inverse restores prior caches without evaluating a formula again.
    pub calc: super::formula::Recalculation,
    /// A source fact from the operation that was applied, combined by Batch.
    /// Pane and tab visibility have no formula-value effect.
    calculation_relevant: bool,
    /// What the edit answers: the range pasted, the cells replaced, ...
    pub result: Scalar,
}

/// The most cells one [`Edit::FillEntry`] or [`Edit::SetEntries`] types
/// into.
pub const MAX_EDITED_CELLS: u64 = 2_097_152;

/// Rollback only consumes inverses generated during the current synchronous
/// attempt. Their targets were already proved and no save can intervene.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Applying {
    Edit,
    Rollback,
}

impl Workbook {
    /// Workbook-local opaque inverses share one origin check.
    fn check_undo_origin(&self, origin: u64) -> Result<()> {
        if origin != self.id {
            return Err(Error::Conflict {
                expected: "undo state of this workbook",
                actual: "undo state of another workbook",
                path: SmolStr::new_static("$"),
            });
        }
        Ok(())
    }

    /// Apply `edit`, answering what it did and the edit undoing it.
    ///
    /// An edit is all or nothing: a refusal leaves the workbook as it was.
    /// Formula values affected by the edit are recalculated once after the
    /// outermost batch. Metadata-only edits leave cold formula sheets unread.
    /// Undo and redo restore cached values and pass status without another
    /// volatile draw.
    ///
    /// ```
    /// use yggdryl::excel::{Edit, Workbook};
    ///
    /// let mut workbook = Workbook::new();
    /// workbook.add_sheet("Sheet1")?;
    /// let applied = workbook.apply(Edit::SetEntries {
    ///     sheet: "Sheet1".into(),
    ///     entries: vec![("A1".parse()?, "12%".into())],
    /// })?;
    /// assert_eq!(workbook.display_text("Sheet1", "A1".parse()?)?.map(|shown| shown.text), Some("12%".into()));
    /// workbook.apply(applied.inverse.expect("an undo"))?;
    /// assert!(workbook.sheet("Sheet1")?.cell("A1".parse()?).is_none());
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the refusal of the method the edit names, before anything
    /// changes, [`Error::Conflict`] for an opaque inverse belonging to
    /// another workbook (including inside a batch), and [`Error::InvalidRecord`] for more than
    /// [`MAX_EDITED_CELLS`] cells typed into at once, or a formula calculation
    /// refusal after the authored edit, rolled back with its cache changes.
    pub fn apply(&mut self, edit: Edit) -> Result<Applied> {
        edit.check_origin(self)?;
        let restores_calculation = edit.restores_calculation();
        let styles = self.style_count();
        let mark = self.begin_batch();
        let mut applied = match self.apply_edit(edit, Applying::Edit) {
            Ok(applied) => applied,
            Err(error) => {
                self.rollback_batch(mark);
                return Err(error);
            }
        };
        if restores_calculation || !applied.calculation_relevant {
            self.finish_batch(mark.start());
            applied.calc = self.restored_calculation_status();
        } else {
            let prepared = self
                .prepare_calculation(super::formula::graph::PassKind::Incremental)
                .and_then(|report| {
                    self.capture_prepared_calculation()
                        .map(|(restore, touched)| (report, restore, touched))
                });
            let (report, restore, calculated_touched) = match prepared {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.discard_prepared_calculation();
                    self.begin_rollback();
                    if let Some(inverse) = applied.inverse.take() {
                        self.apply_edit(inverse, Applying::Rollback)
                            .expect("a generated inverse has an infallible rollback path");
                    }
                    self.rollback_batch(mark);
                    self.invalidate_calculation();
                    return Err(error);
                }
            };
            self.finish_batch(mark.start());
            self.commit_prepared_calculation(&report);
            applied.touched.extend(calculated_touched);
            if let Some(inverse) = applied.inverse.take() {
                let mut restore = restore;
                restore.paired = Some(Box::new(inverse));
                applied.inverse = Some(Edit::Restore(Box::new(restore)));
            }
            applied.calc = report;
        }
        applied.styles |= self.style_count() != styles;
        applied.bytes = applied.inverse.as_ref().map_or(0, Edit::byte_size);
        Ok(applied)
    }

    fn apply_edit(&mut self, edit: Edit, applying: Applying) -> Result<Applied> {
        match edit {
            Edit::SetEntries { sheet, entries } => {
                bound(&sheet, entries.len() as u64)?;
                let ranges: Vec<CellRange> = entries
                    .iter()
                    .map(|(at, _)| CellRange::new(*at, *at))
                    .collect();
                let step = self.cells_step(&sheet, &ranges)?;
                self.guarded(vec![step], |workbook| {
                    for (at, text) in &entries {
                        workbook.set_entry(&sheet, *at, text)?;
                    }
                    Ok(())
                })
                .map(|inverse| touched(&sheet, ranges, inverse))
            }
            Edit::FillEntry {
                sheet,
                ranges,
                text,
                at,
            } => {
                let cells: u64 = ranges
                    .iter()
                    .map(|range| u64::from(range.row_size()) * u64::from(range.column_size()))
                    .sum();
                bound(&sheet, cells)?;
                let system = self.date_system();
                let shape = match super::entry::Entry::from_text(&text, at, system)? {
                    super::entry::Entry::Formula(formula) => Some(formula),
                    _ => None,
                };
                let step = self.cells_step(&sheet, &ranges)?;
                self.guarded(vec![step], |workbook| {
                    for range in &ranges {
                        for cell in range.cells() {
                            workbook.enter_text(&sheet, cell, &text, shape.as_ref())?;
                        }
                    }
                    Ok(())
                })
                .map(|inverse| touched(&sheet, ranges, inverse))
            }
            Edit::Clear {
                sheet,
                ranges,
                what,
            } => {
                let steps = self.format_steps(&sheet, &ranges)?;
                self.guarded(steps, |workbook| workbook.clear(&sheet, &ranges, what))
                    .map(|inverse| touched(&sheet, ranges, inverse))
            }
            Edit::SetStyle {
                sheet,
                ranges,
                patch,
            } => {
                // Only the number format can change the typed temporal value
                // observed by formulas. Appearance edits keep cold sheets cold.
                let calculation_relevant =
                    patch.number_format.is_some() || patch.decimals.is_some();
                let steps = self.format_steps(&sheet, &ranges)?;
                self.guarded(steps, |workbook| {
                    workbook.set_style(&sheet, &ranges, &patch)
                })
                .map(|inverse| {
                    let mut applied = touched(&sheet, ranges, inverse);
                    if let Some(Edit::Restore(restore)) = &mut applied.inverse {
                        restore.calculation_relevant = calculation_relevant;
                    }
                    applied.calculation_relevant = calculation_relevant;
                    applied
                })
            }
            Edit::InsertRows { sheet, at, count } => self.structural(
                &sheet,
                Axis::Rows,
                Band::Insert { at, count },
                applying,
                true,
            ),
            Edit::RemoveRows {
                sheet,
                start,
                count,
            } => self.structural(
                &sheet,
                Axis::Rows,
                Band::Remove {
                    start,
                    end: start.saturating_add(count),
                },
                applying,
                true,
            ),
            Edit::InsertColumns { sheet, at, count } => self.structural(
                &sheet,
                Axis::Columns,
                Band::Insert { at, count },
                applying,
                true,
            ),
            Edit::RemoveColumns {
                sheet,
                start,
                count,
            } => self.structural(
                &sheet,
                Axis::Columns,
                Band::Remove {
                    start,
                    end: start.saturating_add(count),
                },
                applying,
                true,
            ),
            Edit::RowHeight {
                sheet,
                start,
                count,
                height,
            } => {
                let rows = start..start.saturating_add(count);
                let step = self.layout_step(&sheet, Some(rows.clone()), false)?;
                self.guarded(vec![step], |workbook| {
                    workbook
                        .sheet_mut(&sheet)?
                        .set_row_height(rows.clone(), height)
                })
                .map(|inverse| {
                    touched(
                        &sheet,
                        vec![Axis::Rows.whole(start, start.saturating_add(count).max(start + 1))],
                        inverse,
                    )
                })
            }
            Edit::ColumnWidth {
                sheet,
                start,
                count,
                width,
            } => {
                let columns = start..start.saturating_add(count);
                let step = self.layout_step(&sheet, None, true)?;
                self.guarded(vec![step], |workbook| {
                    workbook
                        .sheet_mut(&sheet)?
                        .set_column_width(columns.clone(), width)
                })
                .map(|inverse| {
                    touched(
                        &sheet,
                        vec![
                            Axis::Columns.whole(start, start.saturating_add(count).max(start + 1)),
                        ],
                        inverse,
                    )
                })
            }
            Edit::HideRows {
                sheet,
                start,
                count,
                hidden,
            } => {
                let rows = start..start.saturating_add(count);
                let step = self.layout_step(&sheet, Some(rows.clone()), false)?;
                self.guarded(vec![step], |workbook| {
                    workbook
                        .sheet_mut(&sheet)?
                        .set_rows_hidden(rows.clone(), hidden)
                })
                .map(|inverse| {
                    touched(
                        &sheet,
                        vec![Axis::Rows.whole(start, start.saturating_add(count).max(start + 1))],
                        inverse,
                    )
                })
            }
            Edit::HideColumns {
                sheet,
                start,
                count,
                hidden,
            } => {
                let columns = start..start.saturating_add(count);
                let step = self.layout_step(&sheet, None, true)?;
                self.guarded(vec![step], |workbook| {
                    workbook
                        .sheet_mut(&sheet)?
                        .set_columns_hidden(columns.clone(), hidden)
                })
                .map(|inverse| {
                    touched(
                        &sheet,
                        vec![
                            Axis::Columns.whole(start, start.saturating_add(count).max(start + 1)),
                        ],
                        inverse,
                    )
                })
            }
            Edit::Merge {
                sheet,
                range,
                center,
                across,
            } => {
                let mut steps = self.format_steps(&sheet, &[range])?;
                steps.push(self.merges_step(&sheet)?);
                self.guarded(steps, |workbook| {
                    let rows: Vec<CellRange> = if across {
                        (range.start().row()..=range.end().row())
                            .map(|row| {
                                CellRange::new(
                                    CellRef::new(row, range.start().column()),
                                    CellRef::new(row, range.end().column()),
                                )
                            })
                            .filter(|row| row.start() != row.end())
                            .collect()
                    } else {
                        vec![range]
                    };
                    for merged in &rows {
                        workbook.sheet_mut(&sheet)?.merge(*merged)?;
                    }
                    if center {
                        let patch = StylePatch {
                            horizontal: Some(super::style::Horizontal::Center),
                            ..StylePatch::default()
                        };
                        workbook.set_style(&sheet, &[range], &patch)?;
                    }
                    Ok(())
                })
                .map(|inverse| touched(&sheet, vec![range], inverse))
            }
            Edit::Unmerge { sheet, range } => {
                let step = self.merges_step(&sheet)?;
                self.guarded(vec![step], |workbook| {
                    workbook.sheet_mut(&sheet)?.unmerge(range);
                    Ok(())
                })
                .map(|inverse| touched(&sheet, vec![range], inverse))
            }
            Edit::Freeze { sheet, frozen } => {
                if applying == Applying::Rollback {
                    self.rollback_frozen(&sheet, frozen);
                    return Ok(Applied::of(None, Vec::new()));
                }
                let held = self.sheet(&sheet)?.frozen();
                self.sheet_mut(&sheet)?.set_frozen(frozen)?;
                let mut applied = Applied::of(
                    Some(Edit::Freeze {
                        sheet,
                        frozen: held,
                    }),
                    Vec::new(),
                );
                applied.calculation_relevant = false;
                Ok(applied)
            }
            Edit::Fill {
                sheet,
                source,
                target,
                mode,
            } => {
                let step = self.cells_step(&sheet, &[target])?;
                self.guarded(vec![step], |workbook| {
                    workbook.fill(&sheet, source, target, mode)
                })
                .map(|inverse| {
                    let mut applied = touched(&sheet, vec![target], inverse);
                    applied.result = range_result(target);
                    applied
                })
            }
            Edit::Sort {
                sheet,
                range,
                keys,
                header,
            } => {
                let step = self.cells_step(&sheet, &[range])?;
                self.guarded(vec![step], |workbook| {
                    workbook.sort(&sheet, range, &keys, header)
                })
                .map(|inverse| touched(&sheet, vec![range], inverse))
            }
            Edit::Paste {
                from,
                to,
                what,
                cut,
            } => {
                if cut {
                    let source = self
                        .position(&from.0)
                        .ok_or_else(|| Error::absent("worksheet", from.0.as_str()))?;
                    let target = self
                        .position(&to.0)
                        .ok_or_else(|| Error::absent("worksheet", to.0.as_str()))?;
                    // The checks a paste makes, before the move.
                    let landing = from.1.moved_to(to.1);
                    self.paste_checks((&from.0, from.1), (&to.0, to.1), what, cut)?;
                    let restore = self.cut_paste(source, from.1, target, to.1)?;
                    let mut applied = Applied::of(
                        Some(Edit::Restore(Box::new(restore))),
                        vec![(from.0.clone(), from.1), (to.0.clone(), landing)],
                    );
                    applied.result = range_result(landing);
                    return Ok(applied);
                }
                let landing = from.1.moved_to(to.1);
                let step = self.cells_step(&to.0, &[landing])?;
                self.guarded(vec![step], |workbook| {
                    workbook
                        .paste((&from.0, from.1), (&to.0, to.1), what, cut)
                        .map(drop)
                })
                .map(|inverse| {
                    let mut applied = touched(&to.0, vec![landing], inverse);
                    applied.result = range_result(landing);
                    applied
                })
            }
            Edit::PasteText {
                sheet,
                anchor,
                text,
            } => {
                // The snapshot spans what the paste enters, read by the one
                // reading the paste makes.
                let (rows, span) = super::workbook::pasted(&sheet, anchor, &text)?;
                let step = self.cells_step(&sheet, &[span])?;
                self.guarded(vec![step], |workbook| {
                    workbook.enter_rows(&sheet, anchor, &rows)
                })
                .map(|inverse| {
                    let mut applied = touched(&sheet, vec![span], inverse);
                    applied.result = range_result(span);
                    applied
                })
            }
            Edit::Replace {
                options,
                replacement,
            } => {
                let planned = self.plan_replace(&options, &replacement)?;
                let mut steps = Vec::new();
                let mut changed = Vec::new();
                for (name, cells) in super::find::by_sheet(&planned) {
                    steps.push(self.cells_step(&name, &cells)?);
                    changed.extend(cells.into_iter().map(|range| (name.clone(), range)));
                }
                let inverse = self.guarded(steps, |workbook| workbook.enter_replaced(&planned))?;
                let mut applied = Applied::of(inverse, changed);
                applied.result =
                    Scalar::from_struct([("replaced", Scalar::from(planned.len() as i64))])?;
                Ok(applied)
            }
            Edit::AddSheet { name, at } => {
                let name = match name {
                    Some(name) => name,
                    None => self.next_sheet_name(),
                };
                if let Some(position) = at.filter(|position| *position > self.len()) {
                    return Err(Error::InvalidRecord {
                        path: name,
                        reason: format_smolstr!(
                            "expected a tab position below {}, got {position}",
                            self.len() + 1
                        ),
                    });
                }
                let snapshot = self.naming(&name)?;
                self.add_sheet(name.clone())?;
                if let Some(position) = at {
                    // Membership and the target position were proved before
                    // insertion; moving the new tab cannot refuse.
                    self.move_sheet(&name, position)?;
                }
                let key = self
                    .sheet_key(&name)
                    .map_or(0, super::workbook::SheetKey::as_u32);
                let mut applied = Applied::of(
                    Some(Edit::Batch(vec![
                        Edit::RemoveSheet { name: name.clone() },
                        Edit::Restore(Box::new(snapshot)),
                    ])),
                    Vec::new(),
                );
                applied.sheets = true;
                applied.result = Scalar::from_struct([
                    ("name", Scalar::from(name.as_str())),
                    ("key", Scalar::from(i64::from(key))),
                ])?;
                Ok(applied)
            }
            Edit::RenameSheet { name, to } => {
                if applying == Applying::Rollback {
                    self.rollback_rename(&name, to);
                    return Ok(Applied::of(None, Vec::new()));
                }
                let from = self
                    .sheet_names()
                    .into_iter()
                    .find(|held| held.eq_ignore_ascii_case(&name))
                    .map(SmolStr::new)
                    .ok_or_else(|| Error::absent("worksheet", name.as_str()))?;
                let restore = self.rename(&name, to.clone())?;
                let mut applied = Applied::of(
                    Some(Edit::Batch(vec![
                        Edit::RenameSheet { name: to, to: from },
                        Edit::Restore(Box::new(restore)),
                    ])),
                    Vec::new(),
                );
                applied.sheets = true;
                Ok(applied)
            }
            Edit::RemoveSheet { name } => {
                if applying == Applying::Rollback {
                    self.rollback_remove(&name);
                    return Ok(Applied::of(None, Vec::new()));
                }
                if self.position(&name).is_none() {
                    return Err(Error::absent("worksheet", name.as_str()));
                }
                let visible = self
                    .sheet_names()
                    .into_iter()
                    .filter(|held| {
                        !held.eq_ignore_ascii_case(&name)
                            && self.sheet_kind(held) == Some(super::workbook::SheetKind::Worksheet)
                            && self.sheet_state(held) == Some(SheetState::Visible)
                    })
                    .count();
                if visible == 0 {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::new(name.as_str()),
                        reason: SmolStr::new_static(
                            "expected another visible worksheet to be left, got none",
                        ),
                    });
                }
                let restored = self
                    .remove(&name, true)?
                    .expect("the sheet was found above");
                let mut applied =
                    Applied::of(Some(Edit::RestoreSheet(Box::new(restored))), Vec::new());
                applied.sheets = true;
                Ok(applied)
            }
            Edit::MoveSheet { name, to } => {
                if applying == Applying::Rollback {
                    self.move_sheet(&name, to)
                        .expect("the inverse retains its tab and position");
                    return Ok(Applied::of(None, Vec::new()));
                }
                let at = self
                    .position(&name)
                    .ok_or_else(|| Error::absent("worksheet", name.as_str()))?;
                self.move_sheet(&name, to)?;
                let mut applied = Applied::of(Some(Edit::MoveSheet { name, to: at }), Vec::new());
                applied.sheets = true;
                Ok(applied)
            }
            Edit::SheetState { name, state } => {
                if applying == Applying::Rollback {
                    self.rollback_state(&name, state);
                    return Ok(Applied::of(None, Vec::new()));
                }
                let held = self
                    .sheet_state(&name)
                    .ok_or_else(|| Error::absent("worksheet", name.as_str()))?;
                self.set_sheet_state(&name, state)?;
                let mut applied =
                    Applied::of(Some(Edit::SheetState { name, state: held }), Vec::new());
                applied.sheets = true;
                applied.calculation_relevant = false;
                Ok(applied)
            }
            Edit::Land { destination, cells } => self.land(destination, *cells),
            Edit::PivotCreate {
                spec,
                sheet,
                anchor,
            } => {
                let (range, restore) = self.add_pivot_owned(spec, &sheet, anchor)?;
                let mut result =
                    Applied::of(Some(Edit::Restore(Box::new(restore))), vec![(sheet, range)]);
                result.result = range_result(range);
                Ok(result)
            }
            Edit::PivotRefresh { sheet, name } => {
                let (range, restore) = self.refresh_pivot_owned(&sheet, &name)?;
                let mut result =
                    Applied::of(Some(Edit::Restore(Box::new(restore))), vec![(sheet, range)]);
                result.result = range_result(range);
                Ok(result)
            }
            Edit::PivotRefreshAll => {
                let edits = self
                    .pivots()?
                    .iter()
                    .map(|pivot| Edit::PivotRefresh {
                        sheet: SmolStr::new(pivot.host_sheet()),
                        name: SmolStr::new(pivot.name()),
                    })
                    .collect();
                self.apply_edit(Edit::Batch(edits), applying)
            }
            Edit::PivotUpdate { sheet, name, spec } => {
                let (range, restore) = self.update_pivot_owned(&sheet, &name, spec)?;
                let mut result =
                    Applied::of(Some(Edit::Restore(Box::new(restore))), vec![(sheet, range)]);
                result.result = range_result(range);
                Ok(result)
            }
            Edit::PivotRemove { sheet, name } => {
                let (range, restore) = self.remove_pivot_owned(&sheet, &name)?;
                let mut result =
                    Applied::of(Some(Edit::Restore(Box::new(restore))), vec![(sheet, range)]);
                result.result = range_result(range);
                Ok(result)
            }
            Edit::Batch(edits) => {
                if applying == Applying::Rollback {
                    for edit in edits {
                        self.apply_edit(edit, Applying::Rollback)
                            .expect("generated rollback uses only infallible commit paths");
                    }
                    return Ok(Applied::of(None, Vec::new()));
                }
                let mark = self.begin_batch();
                let mut inverses: Vec<Edit> = Vec::with_capacity(edits.len());
                let mut all = Applied::of(None, Vec::new());
                all.calculation_relevant = false;
                let mut undoable = true;
                for edit in edits {
                    match self.apply_edit(edit, Applying::Edit) {
                        Ok(applied) => {
                            all.touched.extend(applied.touched);
                            all.structural |= applied.structural;
                            all.sheets |= applied.sheets;
                            all.styles |= applied.styles;
                            all.calculation_relevant |= applied.calculation_relevant;
                            all.result = applied.result;
                            match applied.inverse {
                                Some(inverse) => inverses.push(inverse),
                                None => undoable = false,
                            }
                        }
                        Err(error) => {
                            self.begin_rollback();
                            // No source reads, validation or new user inverses
                            // occur while taking this attempt back.
                            for inverse in inverses.into_iter().rev() {
                                self.apply_edit(inverse, Applying::Rollback)
                                    .expect("generated rollback uses only infallible commit paths");
                            }
                            self.rollback_batch(mark);
                            return Err(error);
                        }
                    }
                }
                self.finish_batch(mark.start());
                inverses.reverse();
                all.inverse = undoable.then_some(Edit::Batch(inverses));
                Ok(all)
            }
            Edit::SetCells { sheet, cells } => {
                let ranges: Vec<CellRange> = cells
                    .iter()
                    .map(|(at, _)| CellRange::new(*at, *at))
                    .collect();
                let step = self.cells_step(&sheet, &ranges)?;
                self.guarded(vec![step], |workbook| {
                    let held = workbook.sheet_mut(&sheet)?;
                    for (at, cell) in &cells {
                        match cell {
                            Some(cell) => {
                                held.insert_cell(cell.clone().at(*at))?;
                            }
                            None => {
                                held.remove_cell(*at);
                            }
                        }
                    }
                    Ok(())
                })
                .map(|inverse| touched(&sheet, ranges, inverse))
            }
            Edit::RestoreBand(restored) => {
                self.check_undo_origin(restored.workbook)?;
                let sheet = self
                    .sheet_by_key(restored.sheet)
                    .map(SmolStr::new)
                    .ok_or_else(|| Error::Absent {
                        expected: "worksheet retained by undo",
                        path: format_smolstr!("$.undo.sheet[{}]", restored.sheet.as_u32()),
                    })?;
                self.structural(&sheet, restored.axis, restored.band, applying, false)
            }
            Edit::Restore(mut restore) => {
                if let Some(paired) = restore.paired.take() {
                    let paired_before = restore.paired_before;
                    let receipt = Edit::Restore(restore);
                    let pair = if paired_before {
                        vec![*paired, receipt]
                    } else {
                        vec![receipt, *paired]
                    };
                    let mut applied = self.apply_edit(Edit::Batch(pair), applying)?;
                    if let Some(Edit::Batch(inverses)) = applied.inverse.take() {
                        let mut inverses = inverses.into_iter();
                        let first = inverses.next().expect("two generated inverses");
                        let second = inverses.next().expect("two generated inverses");
                        let (receipt, paired) = if paired_before {
                            (first, second)
                        } else {
                            (second, first)
                        };
                        let Edit::Restore(mut receipt) = receipt else {
                            unreachable!("the receipt's inverse is a receipt");
                        };
                        receipt.paired = Some(Box::new(paired));
                        receipt.paired_before = !paired_before;
                        applied.inverse = Some(Edit::Restore(receipt));
                    }
                    // This opaque pair restores caches and its authored inverse
                    // together. Only fresh edits beside it need a new pass.
                    applied.calculation_relevant = false;
                    return Ok(applied);
                }
                if applying == Applying::Rollback {
                    self.commit_restore(*restore);
                    return Ok(Applied::of(None, Vec::new()));
                }
                let touched = if restore.restores_calculation() {
                    restore
                        .steps
                        .iter()
                        .filter_map(|step| match step {
                            Step::Cells { key, ranges, .. } => {
                                self.sheet_by_key(*key).map(|name| {
                                    ranges
                                        .iter()
                                        .copied()
                                        .map(move |range| (SmolStr::new(name), range))
                                })
                            }
                            _ => None,
                        })
                        .flatten()
                        .collect()
                } else {
                    Vec::new()
                };
                let calculation_relevant = restore.calculation_relevant;
                let inverse = self.restore(*restore)?;
                let mut applied = Applied::of(Some(Edit::Restore(Box::new(inverse))), touched);
                applied.calculation_relevant = calculation_relevant;
                Ok(applied)
            }
            Edit::RestoreSheet(restored) => {
                if applying == Applying::Rollback {
                    self.rollback_sheet(*restored);
                    return Ok(Applied::of(None, Vec::new()));
                }
                // A retained removal can be restored after intervening
                // edits. Removing it again does not recover what its old
                // payload overwrites today, so retain those current facts.
                let name = restored.slot_name();
                let inverse = self.restore_sheet(*restored)?;
                let mut applied = Applied::of(
                    Some(Edit::Batch(vec![
                        Edit::RemoveSheet { name },
                        Edit::Restore(Box::new(inverse)),
                    ])),
                    Vec::new(),
                );
                applied.sheets = true;
                Ok(applied)
            }
        }
    }

    /// Open or close `band` along `axis` of the sheet `sheet`, answering
    /// the opposite band and what it would not give back.
    fn structural(
        &mut self,
        sheet: &str,
        axis: Axis,
        band: Band,
        applying: Applying,
        inherit: bool,
    ) -> Result<Applied> {
        if applying == Applying::Rollback {
            self.rollback_band(sheet, axis, band);
            return Ok(Applied::of(None, Vec::new()));
        }
        let restore = self.shift_band(sheet, axis, band, inherit)?;
        let (at, count) = match band {
            Band::Insert { at, count } => (at, count),
            Band::Remove { start, end } => (start, end - start),
        };
        let name = SmolStr::new(sheet);
        let opposite = match (axis, band) {
            (Axis::Rows, Band::Insert { .. }) => Edit::RemoveRows {
                sheet: name,
                start: at,
                count,
            },
            (Axis::Rows, Band::Remove { .. }) => Edit::RestoreBand(RestoreBand {
                workbook: self.id,
                sheet: self
                    .sheet_key(sheet)
                    .expect("the structural target was resolved"),
                axis,
                band: Band::Insert { at, count },
            }),
            (Axis::Columns, Band::Insert { .. }) => Edit::RemoveColumns {
                sheet: name,
                start: at,
                count,
            },
            (Axis::Columns, Band::Remove { .. }) => Edit::RestoreBand(RestoreBand {
                workbook: self.id,
                sheet: self
                    .sheet_key(sheet)
                    .expect("the structural target was resolved"),
                axis,
                band: Band::Insert { at, count },
            }),
        };
        let mut applied = Applied::of(
            Some(Edit::Batch(vec![
                opposite,
                Edit::Restore(Box::new(restore)),
            ])),
            Vec::new(),
        );
        applied.structural = count > 0;
        Ok(applied)
    }

    /// Land `cells` - built from `A1` - at `destination`.
    fn land(&mut self, destination: Landing, cells: Sheet) -> Result<Applied> {
        match destination {
            Landing::NewSheet(name) => {
                if self.position(&name).is_some() {
                    return Err(Error::Conflict {
                        expected: "sheet name no other sheet has",
                        actual: "sheet of that name, compared without case",
                        path: name,
                    });
                }
                let snapshot = self.naming(&name)?;
                let mut sheet = cells;
                sheet.set_name(name.clone())?;
                let span = sheet.addressed_span();
                self.insert_sheet(sheet)?;
                let mut applied = Applied::of(
                    Some(Edit::Batch(vec![
                        Edit::RemoveSheet { name: name.clone() },
                        Edit::Restore(Box::new(snapshot)),
                    ])),
                    Vec::new(),
                );
                applied.sheets = true;
                if let Some(span) = span {
                    applied.result = range_result(span);
                    applied.touched.push((name, span));
                }
                Ok(applied)
            }
            Landing::At { sheet, anchor } => {
                let Some(span) = cells.addressed_span() else {
                    return Ok(Applied::of(Some(Edit::Batch(Vec::new())), Vec::new()));
                };
                if !anchor.is_in_grid() {
                    return Err(Error::InvalidRecord {
                        path: sheet.clone(),
                        reason: format_smolstr!(
                            "expected a landing anchor in the grid, got row {} column {}",
                            u64::from(anchor.row()) + 1,
                            u64::from(anchor.column()) + 1
                        ),
                    });
                }
                let last_row = u64::from(span.end().row()) + u64::from(anchor.row());
                let last_column = u64::from(span.end().column()) + u64::from(anchor.column());
                if last_row >= u64::from(MAX_ROWS) || last_column >= u64::from(MAX_COLUMNS) {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{sheet}!{anchor}"),
                        reason: format_smolstr!(
                            "expected the cells to land on the grid, got {span} from {anchor}"
                        ),
                    });
                }
                let landing = CellRange::new(
                    CellRef::new(
                        span.start().row() + anchor.row(),
                        span.start().column() + anchor.column(),
                    ),
                    CellRef::new(last_row as u32, last_column as u32),
                );
                let key = self
                    .sheet_key(&sheet)
                    .ok_or_else(|| Error::absent("worksheet", &sheet))?;
                let steps = vec![
                    self.cells_step(&sheet, &[landing])?,
                    self.layout_step(
                        &sheet,
                        Some(landing.start().row()..landing.end().row() + 1),
                        false,
                    )?,
                    Step::RecordFootprint {
                        key,
                        span: self.sheet(&sheet)?.record_footprint(),
                    },
                ];
                self.guarded(steps, |workbook| {
                    let held = workbook.sheet_mut(&sheet)?;
                    let previous: Vec<_> = held.cells_in(landing).map(Cell::reference).collect();
                    for at in previous {
                        held.remove_cell(at);
                    }
                    for cell in cells.cells() {
                        let at = CellRef::new(
                            cell.row() + anchor.row(),
                            cell.column() + anchor.column(),
                        );
                        held.insert_cell(cell.clone().at(at))?;
                    }
                    for row in cells.layout().rows.keys() {
                        if *row >= span.start().row() && *row <= span.end().row() {
                            held.ensure_record_row(row + anchor.row());
                        }
                    }
                    if let Some(written) = cells.record_footprint() {
                        held.add_record_footprint(CellRange::new(
                            CellRef::new(
                                written.start().row() + anchor.row(),
                                written.start().column() + anchor.column(),
                            ),
                            CellRef::new(
                                written.end().row() + anchor.row(),
                                written.end().column() + anchor.column(),
                            ),
                        ));
                    }
                    Ok(())
                })
                .map(|inverse| {
                    let mut applied = touched(&sheet, vec![landing], inverse);
                    applied.result = range_result(landing);
                    applied
                })
            }
        }
    }

    /// Run `change`, answering the [`Restore`] of `steps` as its inverse;
    /// a refusal puts back what `steps` recorded and answers it.
    pub(crate) fn guarded(
        &mut self,
        steps: Vec<Step>,
        change: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<Option<Edit>> {
        let mut restore = Restore {
            steps,
            ..Restore::new(self)
        };
        let checkpoint = self.guard_checkpoint(&restore.steps)?;
        restore.capture_styles(self.style_sheet()?);
        match change(self) {
            Ok(()) => Ok(Some(Edit::Restore(Box::new(restore)))),
            Err(error) => {
                self.rollback_guard(restore, checkpoint);
                Err(error)
            }
        }
    }

    /// The cells of `ranges` of the sheet `sheet`, as a step putting them
    /// back.
    pub(crate) fn cells_step(&self, sheet: &str, ranges: &[CellRange]) -> Result<Step> {
        let held = self.sheet(sheet)?;
        let key = self
            .sheet_key(sheet)
            .ok_or_else(|| Error::absent("worksheet", sheet))?;
        Ok(Step::Cells {
            key,
            ranges: ranges.to_vec(),
            slice: Box::new(held.slice_all(ranges)),
        })
    }

    /// What a format edit of `ranges` changes, as steps putting it back:
    /// their cells, and the formats of the whole rows and columns among
    /// them.
    fn format_steps(&self, sheet: &str, ranges: &[CellRange]) -> Result<Vec<Step>> {
        let mut steps = vec![self.cells_step(sheet, ranges)?];
        let rows = ranges
            .iter()
            .filter(|range| range.is_column_open() && range.start().column() == 0)
            .map(|range| range.start().row()..range.end().row() + 1)
            .reduce(|first, second| first.start.min(second.start)..first.end.max(second.end));
        let columns = ranges
            .iter()
            .any(|range| range.is_row_open() && range.start().row() == 0);
        if rows.is_some() || columns {
            steps.push(self.layout_step(sheet, rows, columns)?);
        }
        steps.push(self.merges_step(sheet)?);
        Ok(steps)
    }

    /// The formats of the rows of `rows` and - with `columns` - the column
    /// spans of the sheet `sheet`, as a step putting them back.
    fn layout_step(&self, sheet: &str, rows: Option<Range<u32>>, columns: bool) -> Result<Step> {
        let held = self.sheet(sheet)?;
        let key = self
            .sheet_key(sheet)
            .ok_or_else(|| Error::absent("worksheet", sheet))?;
        let layout = held.layout();
        Ok(Step::Layout {
            key,
            rows: rows.map(|rows| {
                let formats = layout
                    .rows
                    .range(rows.clone())
                    .map(|(row, format)| (*row, format.clone()))
                    .collect();
                (rows, formats)
            }),
            columns: columns.then(|| layout.columns.0.clone()),
            merges: None,
            pane: None,
        })
    }

    /// The merges of the sheet `sheet`, as a step putting them back.
    fn merges_step(&self, sheet: &str) -> Result<Step> {
        let held = self.sheet(sheet)?;
        let key = self
            .sheet_key(sheet)
            .ok_or_else(|| Error::absent("worksheet", sheet))?;
        Ok(Step::Layout {
            key,
            rows: None,
            columns: None,
            merges: Some(held.merges().collect()),
            pane: None,
        })
    }

    /// What `restore` would overwrite, as a restore putting it back: the
    /// inverse of a [`Restore`] applied on its own.
    pub(crate) fn restore_inverse(&self, restore: &mut Restore) -> Result<Restore> {
        restore.check_origin(self)?;
        let mut inverse = Restore {
            calculation_relevant: restore.calculation_relevant,
            ..Restore::new(self)
        };
        for step in &mut restore.steps {
            let step = match step {
                Step::Cells { key, ranges, .. } => {
                    let name = self
                        .sheet_by_key(*key)
                        .ok_or_else(|| Error::absent("worksheet", key.as_u32()))?;
                    self.cells_step(name, ranges)?
                }
                Step::Formulas { key, cells } => {
                    let name = self
                        .sheet_by_key(*key)
                        .ok_or_else(|| Error::absent("worksheet", key.as_u32()))?;
                    let held = self.sheet(name)?;
                    Step::Formulas {
                        key: *key,
                        cells: cells
                            .iter()
                            .map(|(at, _)| (*at, held.cell(*at).and_then(Cell::formula).cloned()))
                            .collect(),
                    }
                }
                Step::FormulaAttributes { key, cells } => {
                    let name = self
                        .sheet_by_key(*key)
                        .ok_or_else(|| Error::absent("worksheet", key.as_u32()))?;
                    let held = self.sheet(name)?;
                    Step::FormulaAttributes {
                        key: *key,
                        cells: cells
                            .iter()
                            .filter_map(|(at, _)| Some((*at, held.formula_attributes(*at)?)))
                            .collect(),
                    }
                }
                Step::Layout {
                    key,
                    rows,
                    columns,
                    merges,
                    pane,
                } => {
                    let name = self
                        .sheet_by_key(*key)
                        .ok_or_else(|| Error::absent("worksheet", key.as_u32()))?;
                    let layout = self.sheet(name)?.layout();
                    Step::Layout {
                        key: *key,
                        rows: rows.as_ref().map(|(range, _)| {
                            (
                                range.clone(),
                                layout
                                    .rows
                                    .range(range.clone())
                                    .map(|(row, format)| (*row, format.clone()))
                                    .collect(),
                            )
                        }),
                        columns: columns.as_ref().map(|_| layout.columns.0.clone()),
                        merges: merges.as_ref().map(|_| layout.merges.clone()),
                        pane: pane.map(|_| layout.pane),
                    }
                }
                Step::RecordFootprint { key, .. } => {
                    let name = self
                        .sheet_by_key(*key)
                        .ok_or_else(|| Error::absent("worksheet", key.as_u32()))?;
                    Step::RecordFootprint {
                        key: *key,
                        span: self.sheet(name)?.record_footprint(),
                    }
                }
                Step::Frame { key, .. } => {
                    let name = self
                        .sheet_by_key(*key)
                        .ok_or_else(|| Error::absent("worksheet", key.as_u32()))?;
                    Step::Frame {
                        key: *key,
                        frame: self.sheet(name)?.frame().cloned().map(Box::new),
                    }
                }
                Step::Names(_) => Step::Names(self.defined_names().cloned().collect()),
                Step::Views(_) => self.views_inverse(),
                Step::Overrides(parts) => Step::Overrides(
                    parts
                        .iter_mut()
                        .map(|part| {
                            let current = self.part_bytes_if_present(&part.member)?;
                            part.prepare(self, current.as_ref())?;
                            Ok(PartRestore::new(
                                part.member.clone(),
                                current,
                                part.bytes.clone(),
                            ))
                        })
                        .collect::<Result<_>>()?,
                ),
                Step::Calculation { .. } => {
                    let (pass, status) = self.calculation_receipt();
                    Step::Calculation { pass, status }
                }
            };
            inverse.steps.push(step);
        }
        // Put back in the opposite order: a later step's cells over an
        // earlier one's.
        inverse.steps.reverse();
        inverse.capture_styles(self.styles()?.as_ref());
        Ok(inverse)
    }

    /// The first `SheetN` no sheet has, `N` counting from the tabs.
    fn next_sheet_name(&self) -> SmolStr {
        (self.len() + 1..)
            .map(|number| format_smolstr!("Sheet{number}"))
            .find(|name| self.position(name).is_none())
            .unwrap_or_else(|| SmolStr::new_static("Sheet"))
    }
}

/// Refuse more than [`MAX_EDITED_CELLS`] cells typed into.
fn bound(sheet: &str, cells: u64) -> Result<()> {
    if cells > MAX_EDITED_CELLS {
        return Err(Error::InvalidRecord {
            path: SmolStr::new(sheet),
            reason: format_smolstr!(
                "expected at most {MAX_EDITED_CELLS} cells typed into at once, got {cells}"
            ),
        });
    }
    Ok(())
}

/// The answer of an edit of `ranges` of the sheet `sheet`.
fn touched(sheet: &str, ranges: Vec<CellRange>, inverse: Option<Edit>) -> Applied {
    Applied::of(
        inverse,
        ranges
            .into_iter()
            .map(|range| (SmolStr::new(sheet), range))
            .collect(),
    )
}

/// `{"range": "A1:B2"}`.
fn range_result(range: CellRange) -> Scalar {
    Scalar::from_struct([("range", Scalar::from(range.to_string()))]).unwrap_or(Scalar::Null)
}

impl Applied {
    fn of(inverse: Option<Edit>, touched: Vec<(SmolStr, CellRange)>) -> Self {
        Self {
            inverse,
            touched,
            structural: false,
            sheets: false,
            styles: false,
            bytes: 0,
            calc: super::formula::Recalculation::default(),
            calculation_relevant: true,
            result: Scalar::Null,
        }
    }
}

impl RestoreSheet {
    /// The name of the sheet it puts back.
    pub(crate) fn slot_name(&self) -> SmolStr {
        self.slot.name().clone()
    }
}

impl Edit {
    fn restores_calculation(&self) -> bool {
        match self {
            Self::Restore(restore) => restore.restores_calculation(),
            Self::Batch(edits) => !edits.is_empty() && edits.iter().all(Self::restores_calculation),
            _ => false,
        }
    }

    /// Opaque inverses retain workbook-local identities. Check a complete
    /// batch before its first child can change values, styles or revisions.
    fn check_origin(&self, workbook: &Workbook) -> Result<()> {
        match self {
            Self::Restore(restore) => restore.check_origin(workbook),
            Self::RestoreSheet(restored) => restored.restore.check_origin(workbook),
            Self::RestoreBand(restored) => workbook.check_undo_origin(restored.workbook),
            Self::Batch(edits) => {
                for edit in edits {
                    edit.check_origin(workbook)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// An estimate of the bytes the edit holds, which a journal's bound
    /// counts.
    #[must_use]
    pub fn byte_size(&self) -> usize {
        match self {
            Self::Restore(restore) => restore.byte_size(),
            Self::RestoreSheet(restored) => {
                restored.restore.byte_size()
                    + restored.names.len() * 256
                    + restored.slot.byte_size()
                    + restored.parts.byte_size()
            }
            Self::Batch(edits) => edits.iter().map(Self::byte_size).sum(),
            Self::SetEntries { entries, .. } => {
                entries.iter().map(|(_, text)| text.len() + 16).sum()
            }
            Self::SetCells { cells, .. } => cells.len() * 96,
            Self::Land { cells, .. } => cells.cell_count() * 96,
            Self::PasteText { text, .. } | Self::FillEntry { text, .. } => text.len() + 64,
            _ => 64,
        }
    }

    /// What Excel's Undo and Redo menus call the edit: `Typing 'x' in B4`,
    /// `Insert Rows`, `Paste`.
    #[must_use]
    pub fn label(&self) -> SmolStr {
        let text = |text: &str| {
            let short: String = text.chars().take(24).collect();
            if short.len() < text.len() {
                format!("{short}...")
            } else {
                short
            }
        };
        match self {
            Self::SetEntries { entries, .. } => match entries.as_slice() {
                [(at, typed)] => format_smolstr!("Typing '{}' in {at}", text(typed)),
                _ => SmolStr::new_static("Typing"),
            },
            Self::FillEntry {
                text: typed, at, ..
            } => format_smolstr!("Typing '{}' in {at}", text(typed)),
            Self::Clear { what, .. } => SmolStr::new_static(match what {
                Clear::All => "Clear All",
                Clear::Contents => "Clear Contents",
                Clear::Formats => "Clear Formats",
            }),
            Self::SetStyle { .. } => SmolStr::new_static("Format Cells"),
            Self::InsertRows { .. } => SmolStr::new_static("Insert Rows"),
            Self::RemoveRows { .. } => SmolStr::new_static("Delete Rows"),
            Self::InsertColumns { .. } => SmolStr::new_static("Insert Columns"),
            Self::RemoveColumns { .. } => SmolStr::new_static("Delete Columns"),
            Self::RowHeight { .. } => SmolStr::new_static("Row Height"),
            Self::ColumnWidth { .. } => SmolStr::new_static("Column Width"),
            Self::HideRows { hidden, .. } => {
                SmolStr::new_static(if *hidden { "Hide Rows" } else { "Unhide Rows" })
            }
            Self::HideColumns { hidden, .. } => SmolStr::new_static(if *hidden {
                "Hide Columns"
            } else {
                "Unhide Columns"
            }),
            Self::Merge { center: true, .. } => SmolStr::new_static("Merge & Center"),
            Self::Merge { .. } => SmolStr::new_static("Merge Cells"),
            Self::Unmerge { .. } => SmolStr::new_static("Unmerge Cells"),
            Self::Freeze {
                frozen: Some(_), ..
            } => SmolStr::new_static("Freeze Panes"),
            Self::Freeze { .. } => SmolStr::new_static("Unfreeze Panes"),
            Self::Fill { .. } => SmolStr::new_static("Auto Fill"),
            Self::Sort { .. } => SmolStr::new_static("Sort"),
            Self::Paste { cut: true, .. } => SmolStr::new_static("Cut"),
            Self::Paste {
                what: Paste::Formats,
                ..
            } => SmolStr::new_static("Paste Formats"),
            Self::Paste { .. } | Self::PasteText { .. } => SmolStr::new_static("Paste"),
            Self::Replace { .. } => SmolStr::new_static("Replace"),
            Self::AddSheet { .. } => SmolStr::new_static("Insert Sheet"),
            Self::RenameSheet { .. } => SmolStr::new_static("Rename Sheet"),
            Self::RemoveSheet { .. } => SmolStr::new_static("Delete Sheet"),
            Self::MoveSheet { .. } => SmolStr::new_static("Move Sheet"),
            Self::SheetState {
                state: SheetState::Visible,
                ..
            } => SmolStr::new_static("Unhide Sheet"),
            Self::SheetState { .. } => SmolStr::new_static("Hide Sheet"),
            Self::Land { .. } => SmolStr::new_static("Get Data"),
            Self::PivotCreate { .. } => SmolStr::new_static("Create Pivot Table"),
            Self::PivotRefresh { .. } => SmolStr::new_static("Refresh Pivot Table"),
            Self::PivotRefreshAll => SmolStr::new_static("Refresh All Pivot Tables"),
            Self::PivotUpdate { .. } => SmolStr::new_static("Update Pivot Table"),
            Self::PivotRemove { .. } => SmolStr::new_static("Remove Pivot Table"),
            Self::Batch(edits) => edits
                .first()
                .map_or_else(|| SmolStr::new_static("Edit"), Self::label),
            Self::SetCells { .. }
            | Self::Restore(_)
            | Self::RestoreSheet(_)
            | Self::RestoreBand(_) => SmolStr::new_static("Undo"),
        }
    }
}

/// One value of an edit's JSON, where it stands in the edit.
struct Field<'a> {
    value: &'a Scalar,
    path: SmolStr,
}

impl<'a> Field<'a> {
    fn refused(&self, reason: impl Into<SmolStr>) -> Error {
        Error::InvalidRecord {
            path: self.path.clone(),
            reason: reason.into(),
        }
    }

    /// The member `name`, `None` when absent.
    fn get(&self, name: &str) -> Option<Field<'a>> {
        self.value.get_key_str(name).map(|value| Field {
            value,
            path: format_smolstr!("{}.{name}", self.path),
        })
    }

    /// The member `name`, refused when absent.
    fn need(&self, name: &str) -> Result<Field<'a>> {
        self.get(name).ok_or_else(|| Error::InvalidRecord {
            path: format_smolstr!("{}.{name}", self.path),
            reason: SmolStr::new_static("expected a value, got none"),
        })
    }

    /// The member `name` unless absent or null.
    fn given(&self, name: &str) -> Option<Field<'a>> {
        self.get(name).filter(|field| !field.value.is_null())
    }

    fn text(&self) -> Result<&'a str> {
        self.value.as_str().ok_or_else(|| {
            self.refused(format_smolstr!("expected text, got {}", self.value.kind()))
        })
    }

    fn flag(&self) -> Result<bool> {
        self.value.as_bool().ok_or_else(|| {
            self.refused(format_smolstr!(
                "expected true or false, got {}",
                self.value.kind()
            ))
        })
    }

    fn number(&self) -> Result<f64> {
        self.value
            .as_f64()
            .or_else(|| self.value.as_i128().map(|number| number as f64))
            .or_else(|| self.value.as_u128().map(|number| number as f64))
            .filter(|number| number.is_finite())
            .ok_or_else(|| {
                self.refused(format_smolstr!(
                    "expected a number, got {}",
                    self.value.kind()
                ))
            })
    }

    /// A size, `None` for `null`: the default.
    fn size(&self) -> Result<Option<f64>> {
        if self.value.is_null() {
            return Ok(None);
        }
        self.number().map(Some)
    }

    fn index(&self) -> Result<u32> {
        let number = self.number()?;
        if number.fract() != 0.0 || !(0.0..=f64::from(u32::MAX)).contains(&number) {
            return Err(self.refused(format_smolstr!(
                "expected a whole number of at least 0, got {number}"
            )));
        }
        Ok(number as u32)
    }

    fn items(&self) -> Result<Vec<Field<'a>>> {
        let Some(serie) = self.value.as_serie() else {
            return Err(self.refused(format_smolstr!(
                "expected a list, got {}",
                self.value.kind()
            )));
        };
        let Some(run) = serie.as_run() else {
            return Err(self.refused("expected a list of values, got a column"));
        };
        Ok(run
            .as_slice()
            .iter()
            .enumerate()
            .map(|(at, value)| Field {
                value,
                path: format_smolstr!("{}[{at}]", self.path),
            })
            .collect())
    }

    fn range(&self) -> Result<CellRange> {
        let text = self.text()?;
        text.parse().map_err(|_| {
            self.refused(format_smolstr!(
                "expected a range such as A1:C3, got {text:?}"
            ))
        })
    }

    fn cell(&self) -> Result<CellRef> {
        let text = self.text()?;
        text.parse::<CellRef>()
            .ok()
            .filter(|at| at.is_in_grid())
            .ok_or_else(|| {
                self.refused(format_smolstr!("expected a cell such as B2, got {text:?}"))
            })
    }

    fn ranges(&self) -> Result<Vec<CellRange>> {
        self.items()?.iter().map(Field::range).collect()
    }

    /// The name of the sheet a key names.
    fn sheet(&self, workbook: &Workbook) -> Result<SmolStr> {
        let key = self.index()?;
        workbook
            .sheet_names()
            .into_iter()
            .find(|name| {
                workbook
                    .sheet_key(name)
                    .is_some_and(|held| held.as_u32() == key)
            })
            .map(SmolStr::new)
            .ok_or_else(|| Error::Absent {
                expected: "worksheet",
                path: format_smolstr!("{} (no sheet has the key {key})", self.path),
            })
    }

    /// One of `choices`, by the text spelling it.
    fn choice<T: Copy>(&self, choices: &[(&str, T)]) -> Result<T> {
        let text = self.text()?;
        choices
            .iter()
            .find(|(name, _)| *name == text)
            .map(|(_, value)| *value)
            .ok_or_else(|| {
                self.refused(format_smolstr!(
                    "expected one of {}, got {text:?}",
                    choices
                        .iter()
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })
    }

    /// A colour spelled `#RRGGBB`, `None` for null.
    fn color(&self) -> Result<Option<Color>> {
        if self.value.is_null() {
            return Ok(None);
        }
        let text = self.text()?;
        let digits = text
            .strip_prefix('#')
            .filter(|digits| digits.len() == 6)
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
            .ok_or_else(|| {
                self.refused(format_smolstr!(
                    "expected a colour such as #FF0000, got {text:?}"
                ))
            })?;
        Ok(Some(Color::Rgb(0xFF00_0000 | digits)))
    }
}

impl Edit {
    /// The edit the JSON value `value` states, as the workbook service takes
    /// one: `{"op": "insertRows", "sheet": 1, "at": 4, "count": 2}` - sheets
    /// by key, which `workbook` names; ranges and cells as A1 text; indices
    /// zero-based.
    ///
    /// | `op` | Members |
    /// | --- | --- |
    /// | `setEntries` | `sheet`, `entries: [{ref, text}]` |
    /// | `fillEntry` | `sheet`, `ranges`, `text`, `ref` (the active cell) |
    /// | `clear` | `sheet`, `ranges`, `what`: `all`, `contents`, `formats` |
    /// | `setStyle` | `sheet`, `ranges`, `patch` - an absent member skipped, `null` clearing |
    /// | `insertRows`, `insertColumns` | `sheet`, `at`, `count` |
    /// | `removeRows`, `removeColumns` | `sheet`, `start`, `count` |
    /// | `rowHeight`, `columnWidth` | `sheet`, `start`, `count`, `size` (`null` for the default, never absent) |
    /// | `hideRows`, `hideColumns` | `sheet`, `start`, `count`, `hidden` |
    /// | `merge` | `sheet`, `range`, `center`, `across` |
    /// | `unmerge` | `sheet`, `range` |
    /// | `freeze` | `sheet`, `rows`, `columns` (both `0` unfreezes) |
    /// | `fill` | `sheet`, `source`, `target`, `mode`: `series`, `copy` |
    /// | `sort` | `sheet`, `range`, `header`, `keys: [{column: "B", descending}]` |
    /// | `paste` | `from: {sheet, range}`, `to: {sheet, ref}`, `what`, `cut` |
    /// | `pasteText` | `sheet`, `ref`, `text` |
    /// | `replace` | `scope`, `sheet`, `text`, `replacement`, `in`, `matchCase`, `entireCell` |
    /// | `addSheet` | `name`, `at` |
    /// | `renameSheet` | `sheet`, `name` |
    /// | `removeSheet` | `sheet` |
    /// | `moveSheet` | `sheet`, `to` |
    /// | `sheetState` | `sheet`, `state`: `visible`, `hidden`, `veryHidden` |
    /// | `batch` | `edits` |
    ///
    /// ```
    /// use yggdryl::excel::{Edit, Workbook};
    /// use yggdryl::from_json_scalar;
    ///
    /// let mut workbook = Workbook::new();
    /// workbook.add_sheet("Data")?;
    /// let key = workbook.sheet_key("Data").unwrap().as_u32();
    /// let json = format!(r#"{{"op":"setEntries","sheet":{key},"entries":[{{"ref":"B2","text":"=1+1"}}]}}"#);
    /// let edit = Edit::from_scalar(&from_json_scalar(json)?, &workbook)?;
    /// workbook.apply(edit)?;
    /// assert_eq!(workbook.entry_text("Data", "B2".parse()?)?.as_deref(), Some("=1+1"));
    ///
    /// let bad = format!(r#"{{"op":"setEntries","sheet":{key},"entries":[{{"ref":"B0","text":"x"}}]}}"#);
    /// assert_eq!(
    ///     Edit::from_scalar(&from_json_scalar(bad)?, &workbook).unwrap_err().to_string(),
    ///     "invalid record value at $.edit.entries[0].ref: expected a cell such as B2, got \"B0\"",
    /// );
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming the member - its path from
    /// `$.edit` - that is missing, of the wrong kind or out of range, an
    /// `op` no edit has, or one only an inverse carries; [`Error::Absent`]
    /// for a sheet key no sheet has.
    pub fn from_scalar(value: &Scalar, workbook: &Workbook) -> Result<Self> {
        Self::read(
            &Field {
                value,
                path: SmolStr::new_static("$.edit"),
            },
            workbook,
        )
    }

    fn read(field: &Field<'_>, workbook: &Workbook) -> Result<Self> {
        if field.value.as_struct().is_none() {
            return Err(field.refused(format_smolstr!(
                "expected an edit object, got {}",
                field.value.kind()
            )));
        }
        let op = field.need("op")?;
        let sheet = || field.need("sheet")?.sheet(workbook);
        let count = || field.need("count")?.index();
        let flag = |name: &str| field.given(name).map_or(Ok(false), |value| value.flag());
        Ok(match op.text()? {
            "setEntries" => Self::SetEntries {
                sheet: sheet()?,
                entries: field
                    .need("entries")?
                    .items()?
                    .iter()
                    .map(|entry| {
                        Ok((
                            entry.need("ref")?.cell()?,
                            SmolStr::new(entry.need("text")?.text()?),
                        ))
                    })
                    .collect::<Result<_>>()?,
            },
            "fillEntry" => Self::FillEntry {
                sheet: sheet()?,
                ranges: field.need("ranges")?.ranges()?,
                text: SmolStr::new(field.need("text")?.text()?),
                at: field.need("ref")?.cell()?,
            },
            "clear" => Self::Clear {
                sheet: sheet()?,
                ranges: field.need("ranges")?.ranges()?,
                what: field.need("what")?.choice(&[
                    ("all", Clear::All),
                    ("contents", Clear::Contents),
                    ("formats", Clear::Formats),
                ])?,
            },
            "setStyle" => Self::SetStyle {
                sheet: sheet()?,
                ranges: field.need("ranges")?.ranges()?,
                patch: patch(&field.need("patch")?)?,
            },
            "insertRows" => Self::InsertRows {
                sheet: sheet()?,
                at: field.need("at")?.index()?,
                count: count()?,
            },
            "insertColumns" => Self::InsertColumns {
                sheet: sheet()?,
                at: field.need("at")?.index()?,
                count: count()?,
            },
            "removeRows" => Self::RemoveRows {
                sheet: sheet()?,
                start: field.need("start")?.index()?,
                count: count()?,
            },
            "removeColumns" => Self::RemoveColumns {
                sheet: sheet()?,
                start: field.need("start")?.index()?,
                count: count()?,
            },
            "rowHeight" => Self::RowHeight {
                sheet: sheet()?,
                start: field.need("start")?.index()?,
                count: count()?,
                height: field.need("size")?.size()?,
            },
            "columnWidth" => Self::ColumnWidth {
                sheet: sheet()?,
                start: field.need("start")?.index()?,
                count: count()?,
                width: field.need("size")?.size()?,
            },
            "hideRows" => Self::HideRows {
                sheet: sheet()?,
                start: field.need("start")?.index()?,
                count: count()?,
                hidden: field.need("hidden")?.flag()?,
            },
            "hideColumns" => Self::HideColumns {
                sheet: sheet()?,
                start: field.need("start")?.index()?,
                count: count()?,
                hidden: field.need("hidden")?.flag()?,
            },
            "merge" => Self::Merge {
                sheet: sheet()?,
                range: field.need("range")?.range()?,
                center: flag("center")?,
                across: flag("across")?,
            },
            "unmerge" => Self::Unmerge {
                sheet: sheet()?,
                range: field.need("range")?.range()?,
            },
            "freeze" => {
                let rows = field.need("rows")?.index()?;
                let columns = field.need("columns")?.index()?;
                Self::Freeze {
                    sheet: sheet()?,
                    frozen: (rows > 0 || columns > 0).then_some(Frozen { rows, columns }),
                }
            }
            "fill" => Self::Fill {
                sheet: sheet()?,
                source: field.need("source")?.range()?,
                target: field.need("target")?.range()?,
                mode: field
                    .need("mode")?
                    .choice(&[("series", FillMode::Series), ("copy", FillMode::Copy)])?,
            },
            "sort" => Self::Sort {
                sheet: sheet()?,
                range: field.need("range")?.range()?,
                header: flag("header")?,
                keys: field
                    .need("keys")?
                    .items()?
                    .iter()
                    .map(|key| {
                        let column = key.need("column")?;
                        let letters = column.text()?;
                        Ok(SortKey {
                            column: CellRef::column_index(letters).ok_or_else(|| {
                                column.refused(format_smolstr!(
                                    "expected column letters such as B, got {letters:?}"
                                ))
                            })?,
                            descending: key
                                .given("descending")
                                .map_or(Ok(false), |value| value.flag())?,
                        })
                    })
                    .collect::<Result<_>>()?,
            },
            "paste" => {
                let from = field.need("from")?;
                let to = field.need("to")?;
                Self::Paste {
                    from: (
                        from.need("sheet")?.sheet(workbook)?,
                        from.need("range")?.range()?,
                    ),
                    to: (to.need("sheet")?.sheet(workbook)?, to.need("ref")?.cell()?),
                    what: field.given("what").map_or(Ok(Paste::All), |what| {
                        what.choice(&[
                            ("all", Paste::All),
                            ("values", Paste::Values),
                            ("formulas", Paste::Formulas),
                            ("formats", Paste::Formats),
                        ])
                    })?,
                    cut: flag("cut")?,
                }
            }
            "pasteText" => Self::PasteText {
                sheet: sheet()?,
                anchor: field.need("ref")?.cell()?,
                text: SmolStr::new(field.need("text")?.text()?),
            },
            "replace" => Self::Replace {
                options: find_options(field, workbook)?,
                replacement: SmolStr::new(field.need("replacement")?.text()?),
            },
            "pivotCreate" => Self::PivotCreate {
                spec: PivotSpec::from_scalar(field.need("spec")?.value)?,
                sheet: sheet()?,
                anchor: field.need("anchor")?.cell()?,
            },
            "pivotRefresh" => Self::PivotRefresh {
                sheet: sheet()?,
                name: SmolStr::new(field.need("name")?.text()?),
            },
            "pivotRefreshAll" => Self::PivotRefreshAll,
            "pivotUpdate" => Self::PivotUpdate {
                sheet: sheet()?,
                name: SmolStr::new(field.need("name")?.text()?),
                spec: PivotSpec::from_scalar(field.need("spec")?.value)?,
            },
            "pivotRemove" => Self::PivotRemove {
                sheet: sheet()?,
                name: SmolStr::new(field.need("name")?.text()?),
            },
            "addSheet" => Self::AddSheet {
                name: field
                    .given("name")
                    .map(|name| name.text().map(SmolStr::new))
                    .transpose()?,
                at: field
                    .given("at")
                    .map(|at| at.index().map(|at| at as usize))
                    .transpose()?,
            },
            "renameSheet" => Self::RenameSheet {
                name: sheet()?,
                to: SmolStr::new(field.need("name")?.text()?),
            },
            "removeSheet" => Self::RemoveSheet { name: sheet()? },
            "moveSheet" => Self::MoveSheet {
                name: sheet()?,
                to: field.need("to")?.index()? as usize,
            },
            "sheetState" => Self::SheetState {
                name: sheet()?,
                state: field.need("state")?.choice(&[
                    ("visible", SheetState::Visible),
                    ("hidden", SheetState::Hidden),
                    ("veryHidden", SheetState::VeryHidden),
                ])?,
            },
            "batch" => Self::Batch(
                field
                    .need("edits")?
                    .items()?
                    .iter()
                    .map(|edit| Self::read(edit, workbook))
                    .collect::<Result<_>>()?,
            ),
            other => {
                return Err(op.refused(format_smolstr!(
                    "expected an edit the service takes (setEntries, fillEntry, clear, setStyle, \
                     insertRows, removeRows, insertColumns, removeColumns, rowHeight, \
                     columnWidth, hideRows, hideColumns, merge, unmerge, freeze, fill, sort, \
                     paste, pasteText, replace, addSheet, renameSheet, removeSheet, moveSheet, \
                     sheetState, pivotCreate, pivotRefresh, pivotRefreshAll, pivotUpdate, pivotRemove, batch), got {other:?}"
                )));
            }
        })
    }
}

/// The find options the members of `field` state: the `replace` edit's and
/// the service's find.
fn find_options(field: &Field<'_>, workbook: &Workbook) -> Result<FindOptions> {
    let flag = |name: &str| field.given(name).map_or(Ok(false), |value| value.flag());
    let text = field.need("text")?;
    if text.text()?.is_empty() {
        return Err(text.refused("expected text to find, got the empty text"));
    }
    Ok(FindOptions {
        text: SmolStr::new(text.text()?),
        scope: field.given("scope").map_or(Ok(FindScope::Sheet), |scope| {
            scope.choice(&[
                ("sheet", FindScope::Sheet),
                ("workbook", FindScope::Workbook),
            ])
        })?,
        sheet: field.need("sheet")?.sheet(workbook)?,
        within: field.given("in").map_or(Ok(Within::Values), |within| {
            within.choice(&[("values", Within::Values), ("formulas", Within::Formulas)])
        })?,
        match_case: flag("matchCase")?,
        entire_cell: flag("entireCell")?,
    })
}

/// The style patch the members of `field` state: an absent member
/// skipped, `null` clearing what can be cleared.
fn patch(field: &Field<'_>) -> Result<StylePatch> {
    if field.value.as_struct().is_none() {
        return Err(field.refused(format_smolstr!(
            "expected a patch object, got {}",
            field.value.kind()
        )));
    }
    let mut patch = StylePatch::default();
    // A member stated `null` clears; one absent is skipped.
    let cleared = |name: &str| field.get(name).is_some_and(|value| value.value.is_null());
    if let Some(name) = field.given("fontName") {
        patch.font_name = Some(SmolStr::new(name.text()?));
    }
    if let Some(size) = field.given("fontSize") {
        patch.font_size = Some(size.number()?);
    }
    for (name, slot) in [
        ("bold", &mut patch.bold),
        ("italic", &mut patch.italic),
        ("strike", &mut patch.strike),
        ("wrap", &mut patch.wrap),
    ] {
        if cleared(name) {
            *slot = Some(false);
        } else if let Some(value) = field.given(name) {
            *slot = Some(value.flag()?);
        }
    }
    if cleared("underline") {
        patch.underline = Some(Underline::None);
    } else if let Some(underline) = field.given("underline") {
        patch.underline = Some(Underline::from_attribute(underline.text()?).map_err(|_| {
            underline.refused("expected none, single, double, singleAccounting or doubleAccounting")
        })?);
    }
    if let Some(color) = field.get("fontColor") {
        patch.font_color = Some(color.color()?);
    }
    if let Some(fill) = field.get("fill") {
        patch.fill = Some(fill.color()?);
    }
    if cleared("borders") {
        patch.borders = Some(Borders {
            preset: BorderPreset::None,
            ..Borders::default()
        });
    } else if let Some(borders) = field.given("borders") {
        let preset = borders.need("preset")?;
        patch.borders = Some(Borders {
            preset: BorderPreset::from_attribute(preset.text()?).map_err(|_| {
                preset.refused(
                    "expected bottom, top, left, right, all, outside, thickOutside or none",
                )
            })?,
            style: match borders.given("style") {
                Some(style) => BorderStyle::from_attribute(style.text()?).map_err(|_| {
                    style.refused(format_smolstr!(
                        "expected a border style, got {:?}",
                        style.value
                    ))
                })?,
                None => BorderStyle::Thin,
            },
            color: match borders.get("color") {
                Some(color) => color.color()?,
                None => None,
            },
        });
    }
    if cleared("horizontal") {
        patch.horizontal = Some(Horizontal::General);
    } else if let Some(horizontal) = field.given("horizontal") {
        patch.horizontal =
            Some(Horizontal::from_attribute(horizontal.text()?).map_err(|_| {
                horizontal.refused("expected a horizontal alignment such as center")
            })?);
    }
    if cleared("vertical") {
        patch.vertical = Some(Vertical::Bottom);
    } else if let Some(vertical) = field.given("vertical") {
        patch.vertical = Some(
            Vertical::from_attribute(vertical.text()?)
                .map_err(|_| vertical.refused("expected a vertical alignment such as center"))?,
        );
    }
    if cleared("indent") {
        patch.indent = Some(0);
    } else if let Some(indent) = field.given("indent") {
        let steps = indent.index()?;
        patch.indent = Some(u8::try_from(steps).map_err(|_| {
            indent.refused(format_smolstr!(
                "expected an indent of at most 250, got {steps}"
            ))
        })?);
    }
    if cleared("numberFormat") {
        patch.number_format = Some(SmolStr::new_static("General"));
    } else if let Some(code) = field.given("numberFormat") {
        patch.number_format = Some(SmolStr::new(code.text()?));
    }
    if let Some(decimals) = field.given("decimals") {
        let delta = decimals.number()?;
        if delta.fract() != 0.0 || !(-15.0..=15.0).contains(&delta) {
            return Err(decimals.refused(format_smolstr!(
                "expected a whole number from -15 to 15, got {delta}"
            )));
        }
        patch.decimals = Some(delta as i8);
    }
    Ok(patch)
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/excel/edit.rs` pins and a caller cannot reach:
    //! everything a workbook holds that an edit may change, spelled out,
    //! which an edit's inverse gives back exactly.

    use crate::excel::{Sheet, SheetKey, Workbook};

    /// Whether the worksheet frame is present, including an empty frame.
    #[must_use]
    pub fn has_frame(sheet: &Sheet) -> bool {
        sheet.frame().is_some()
    }

    /// Exercise a worksheet-frame restore through the production step.
    #[must_use]
    pub fn frame_edit(workbook: &Workbook, key: SheetKey, source: &Sheet) -> super::Edit {
        let mut restore = super::Restore::new(workbook);
        restore.push(super::Step::Frame {
            key,
            frame: source.frame().cloned().map(Box::new),
        });
        super::Edit::Restore(Box::new(restore))
    }

    /// Exercise the authored payload's core restore preflight without the
    /// edit dispatcher. Automatic-calculation receipts wrap that payload;
    /// the helper validates provenance, then discards only those wrappers.
    pub fn restore_direct(workbook: &mut Workbook, mut edit: super::Edit) -> crate::Result<()> {
        edit.check_origin(workbook)?;
        loop {
            match edit {
                super::Edit::Restore(mut restore) => {
                    if restore.restores_calculation()
                        && let Some(paired) = restore.paired.take()
                    {
                        edit = *paired;
                        continue;
                    }
                    return workbook.restore(*restore).map(|_| ());
                }
                other => return Err(crate::Error::absent("restore edit", other.label())),
            }
        }
    }

    /// Every tab, every parsed sheet's cells and layout and what its part
    /// carries, the names, the views and the parts an edit rewrote.
    #[must_use]
    pub fn state(workbook: &Workbook) -> String {
        workbook.describe()
    }

    /// Package part steps, including ones whose inverse must restore absence.
    #[must_use]
    pub fn parts_edit(workbook: &Workbook, parts: &[(&str, Option<&[u8]>)]) -> super::Edit {
        let mut restore = super::Restore::new(workbook);
        for (member, bytes) in parts {
            let expected = workbook
                .part_bytes_if_present(member)
                .expect("test package part can be read");
            restore.push(super::Step::Overrides(vec![super::PartRestore::new(
                (*member).into(),
                bytes.map(Into::into),
                expected,
            )]));
        }
        super::Edit::Restore(Box::new(restore))
    }
}
