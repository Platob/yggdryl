//! One worksheet held whole: every cell it states, reachable by reference.
//!
//! A [`Sheet`] is the random-access model of a worksheet part - what a
//! caller opens to read `B7`, set `C3`, walk a column, window a range or
//! lay its rows out as a [`Serie`]. It holds its cells sparsely, by row then
//! column, so a cell costs what it holds and nothing else, and it is never
//! on the record path: a record read through [`Excel`](super::Excel)
//! streams the part and builds no cell.
//!
//! Every door takes a resolved [`CellRef`] or [`CellRange`], never A1 text:
//! `sheet.cell("B2".parse()?)` is the text spelling and `sheet.cell((1,
//! 1).into())` the numeric one, and a loop over cells parses nothing.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::Write;
use std::ops::Range;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::media::DEFAULT_ROOT_NAME;
use crate::{ArrowCastOptions, DataType, Error, Field, Result, Scalar, Serie, StructType};

use super::carried::{Model, WorksheetFrame};
use super::cell::{
    Cell, CellKind, CellRange, CellRef, DateSystem, ExcelError, MAX_COLUMNS, MAX_ROWS, cell_text,
};
use super::formula::{Formula, Interner};
use super::layout::{
    ColumnFormat, DEFAULT_COLUMN_WIDTH, DEFAULT_ROW_HEIGHT, Frozen, Layout, RowFormat, RowFormats,
};
use super::parser::{FormulaAttributes, FormulaKind, RawCell, SheetRows};
use super::records::{
    FlatBinding, FlatExtent, Header, HeaderProbe, RowLabel, RowsLayout, RowsWindow,
};
use super::shared_strings::{SharedStrings, SharedStringsWriter};
use super::shift::{Axis, Band};
use super::style::StyleId;
use super::styles::{NumberFormat, Splice, StyleSheet};
use crate::RecordHeader;

static CHANGE_GENERATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Clone, Copy, Debug)]
struct CellChange {
    value: u64,
    formula: Option<u64>,
}

/// Pending calculation facts for one sheet instance. Only the first change to
/// an address/flag is retained; capacity follows peak distinct pending cells,
/// not the number of edits. Structural invalidation suppresses further points
/// but keeps earlier memberships so an attempted edit can roll back its mark.
#[derive(Debug)]
pub(crate) struct ChangeSet {
    generation: u64,
    epoch: u64,
    stamp: u64,
    structural: bool,
    points: HashMap<CellRef, CellChange>,
}

impl Default for ChangeSet {
    fn default() -> Self {
        use std::sync::atomic::Ordering;
        Self {
            generation: CHANGE_GENERATIONS
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                    next.checked_add(1)
                })
                .expect("sheet change generation exhausted"),
            epoch: 0,
            stamp: 0,
            structural: true,
            points: HashMap::new(),
        }
    }
}

impl Clone for ChangeSet {
    fn clone(&self) -> Self {
        // A cloned active Sheet may replace another sheet through &mut Sheet.
        // Its identity must force a rebuild, even if revision and cells match.
        // Pending history is redundant once that new instance requires rebuild.
        Self::default()
    }
}

/// An attempt-local mark, consumed in nesting order before acknowledgment.
/// It retains no cells or map clone. Rollback scans distinct pending points;
/// normal point insertion and lookup remain amortized O(1).
#[derive(Debug)]
pub(crate) struct ChangeMark {
    generation: u64,
    epoch: u64,
    stamp: u64,
    structural: bool,
}

impl ChangeSet {
    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }
    pub(crate) const fn structural(&self) -> bool {
        self.structural
    }
    pub(crate) fn points(&self) -> impl Iterator<Item = (CellRef, bool)> + '_ {
        self.points
            .iter()
            .map(|(at, changed)| (*at, changed.formula.is_some()))
    }

    fn cell(&mut self, at: CellRef, formula: bool) {
        if self.structural {
            return;
        }
        match self.points.entry(at) {
            std::collections::hash_map::Entry::Occupied(mut held) => {
                if formula && held.get().formula.is_none() {
                    self.stamp += 1;
                    held.get_mut().formula = Some(self.stamp);
                }
            }
            std::collections::hash_map::Entry::Vacant(vacant) => {
                self.stamp += 1;
                vacant.insert(CellChange {
                    value: self.stamp,
                    formula: formula.then_some(self.stamp),
                });
            }
        }
        // At most two first memberships per grid coordinate exist in an
        // epoch. Rollback resets the stamp, so valid cells cannot overflow it.
    }

    fn mark(&self) -> ChangeMark {
        ChangeMark {
            generation: self.generation,
            epoch: self.epoch,
            stamp: self.stamp,
            structural: self.structural,
        }
    }

    fn accepts(&self, mark: &ChangeMark) -> bool {
        self.generation == mark.generation && self.epoch == mark.epoch && mark.stamp <= self.stamp
    }

    fn rollback(&mut self, mark: ChangeMark) {
        self.points.retain(|_, changed| {
            if changed.value > mark.stamp {
                return false;
            }
            if changed.formula.is_some_and(|stamp| stamp > mark.stamp) {
                changed.formula = None;
            }
            true
        });
        self.stamp = mark.stamp;
        self.structural = mark.structural;
    }

    fn acknowledge(&mut self) {
        self.epoch = self
            .epoch
            .checked_add(1)
            .expect("sheet change epoch exhausted");
        self.stamp = 0;
        self.structural = false;
        self.points.clear();
    }
}

/// Excel's own limit on a sheet name's characters.
pub const MAX_SHEET_NAME: usize = 31;

/// Whether a sheet shows in the workbook's tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SheetState {
    /// The tab is shown.
    #[default]
    Visible,
    /// The tab is hidden, and a user can unhide it.
    Hidden,
    /// The tab is hidden and only a macro can unhide it.
    VeryHidden,
}

impl SheetState {
    /// The `state` attribute as the file spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::Hidden => "hidden",
            Self::VeryHidden => "veryHidden",
        }
    }

    /// The state a `state` attribute spells; an absent one is visible.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for a value the schema does not list.
    pub fn from_attribute(value: Option<&str>) -> Result<Self> {
        Ok(match value.map(str::trim) {
            None | Some("visible") => Self::Visible,
            Some("hidden") => Self::Hidden,
            Some("veryHidden") => Self::VeryHidden,
            Some(other) => {
                return Err(Error::Parse {
                    target: "sheet state",
                    position: 0,
                    reason: format_smolstr!(
                        "expected visible, hidden or veryHidden for a sheet's state, got {other:?}"
                    ),
                });
            }
        })
    }
}

/// Refuse a sheet name Excel refuses: empty, over [`MAX_SHEET_NAME`]
/// characters, holding any of `\ / ? * [ ] :`, opening or closing with an
/// apostrophe, or the reserved `History`.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] naming the rule and the name.
pub fn validate_sheet_name(name: &str) -> Result<()> {
    let refuse = |reason: SmolStr| Error::InvalidRecord {
        path: SmolStr::new_static("$.sheet"),
        reason,
    };
    if name.is_empty() {
        return Err(refuse(SmolStr::new_static(
            "expected a sheet name, got the empty text",
        )));
    }
    let length = name.chars().count();
    if length > MAX_SHEET_NAME {
        return Err(refuse(format_smolstr!(
            "expected a sheet name of at most {MAX_SHEET_NAME} characters, got {length} in {name:?}"
        )));
    }
    if let Some(forbidden) = name
        .chars()
        .find(|character| matches!(character, '\\' | '/' | '?' | '*' | '[' | ']' | ':'))
    {
        return Err(refuse(format_smolstr!(
            "expected a sheet name without any of \\ / ? * [ ] :, got {forbidden:?} in {name:?}"
        )));
    }
    if name.starts_with('\'') || name.ends_with('\'') {
        return Err(refuse(format_smolstr!(
            "expected a sheet name that neither opens nor closes with an apostrophe, got {name:?}"
        )));
    }
    if name.eq_ignore_ascii_case("History") {
        return Err(refuse(SmolStr::new_static(
            "expected a sheet name other than the reserved `History`",
        )));
    }
    Ok(())
}

/// The cells of one row, by column.
///
/// The cells are one vector sorted by column and found by binary search: a
/// row read from a part holds it at its exact length, and a row built cell
/// by cell grows it as a vector grows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    index: u32,
    cells: Vec<Cell>,
}

impl Row {
    /// The zero-based row.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// The cells present, in column order.
    pub fn cells(&self) -> impl Iterator<Item = &Cell> + '_ {
        self.cells.iter()
    }

    /// The cell at zero-based `column`, when present.
    #[must_use]
    pub fn cell(&self, column: u32) -> Option<&Cell> {
        self.position(column).ok().map(|at| &self.cells[at])
    }

    /// How many cells the row holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether the row holds no cell.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Where the cell of `column` is, or where it would go.
    fn position(&self, column: u32) -> std::result::Result<usize, usize> {
        self.cells.binary_search_by_key(&column, Cell::column)
    }

    /// The cells from column `first` to column `last`, both included.
    fn between(&self, first: u32, last: u32) -> &[Cell] {
        let from = self.cells.partition_point(|cell| cell.column() < first);
        let to = self.cells.partition_point(|cell| cell.column() <= last);
        &self.cells[from..to.max(from)]
    }

    /// This row at zero-based `index`, every cell's reference with it.
    fn moved_to(mut self, index: u32) -> Self {
        self.index = index;
        for cell in &mut self.cells {
            cell.move_to(CellRef::new(index, cell.column()));
        }
        self
    }
}

/// A cell of a [`Sheet`] borrowed for change, from [`Sheet::cell_mut`].
///
/// It reads and writes as the [`Cell`] it borrows, and puts that cell back
/// at its reference when it is dropped: a cell replaced through it with one
/// built elsewhere - `*cell = other.clone()` - takes the borrowed cell's
/// place, never the other's, so the row stays sorted and the sheet's counts
/// hold.
pub struct CellMut<'a> {
    cell: &'a mut Cell,
    reference: CellRef,
    extras: &'a mut BTreeMap<CellRef, CellExtra>,
    checked_serial: bool,
    saved_serial: Option<(u64, Cell)>,
}

impl std::ops::Deref for CellMut<'_> {
    type Target = Cell;

    fn deref(&self) -> &Cell {
        self.cell
    }
}

impl std::ops::DerefMut for CellMut<'_> {
    fn deref_mut(&mut self) -> &mut Cell {
        if !self.checked_serial {
            self.checked_serial = true;
            if let Some(extra) = self.extras.get_mut(&self.reference) {
                if let Some(bits) = extra.original_serial_bits.take() {
                    self.saved_serial = Some((bits, self.cell.clone()));
                }
                if *extra == CellExtra::default() {
                    self.extras.remove(&self.reference);
                }
            }
        }
        self.cell
    }
}

impl Drop for CellMut<'_> {
    fn drop(&mut self) {
        self.cell.move_to(self.reference);
        if let Some((bits, original)) = self.saved_serial.take() {
            if self.cell.kind() == original.kind()
                && self.cell.value() == original.value()
                && self.cell.format() == original.format()
                && self.cell.formula() == original.formula()
                && self.cell.error() == original.error()
            {
                self.extras
                    .entry(self.reference)
                    .or_default()
                    .original_serial_bits = Some(bits);
            }
        }
    }
}

impl fmt::Debug for CellMut<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&*self.cell, formatter)
    }
}

/// What the cells of a sheet span, kept as cells come and go so neither the
/// count nor the rectangle is a walk.
#[derive(Clone, Default)]
pub(crate) struct Extent {
    /// How many cells the sheet holds.
    cells: usize,
    /// How many cells each column holds: [`MAX_COLUMNS`] counters, 64 KiB,
    /// allocated with the sheet's first cell and freed with its last.
    columns: Option<Box<[u32]>>,
    /// The first and the last column holding a cell, while one does.
    first_column: u32,
    last_column: u32,
}

impl Extent {
    /// Count a cell added at `column`.
    ///
    /// A column off the grid has no counter, so it is not counted: only a
    /// cell a caller moved behind the sheet's back can stand there, and the
    /// sheet answers around it rather than panic.
    fn add(&mut self, column: u32) {
        let columns = self
            .columns
            .get_or_insert_with(|| vec![0; MAX_COLUMNS as usize].into_boxed_slice());
        let Some(held) = columns.get_mut(column as usize) else {
            return;
        };
        *held += 1;
        if self.cells == 0 {
            (self.first_column, self.last_column) = (column, column);
        } else {
            self.first_column = self.first_column.min(column);
            self.last_column = self.last_column.max(column);
        }
        self.cells += 1;
    }

    /// Count a cell taken from `column`.
    ///
    /// An edge column that empties moves the edge to the next column still
    /// holding a cell, which is a scan up to that column and no further.
    fn remove(&mut self, column: u32) {
        let Some(held) = self
            .columns
            .as_mut()
            .and_then(|columns| columns.get_mut(column as usize))
        else {
            return;
        };
        *held = held.saturating_sub(1);
        self.cells = self.cells.saturating_sub(1);
        if self.cells == 0 {
            *self = Self::default();
            return;
        }
        if *held > 0 {
            return;
        }
        let Some(columns) = self.columns.as_deref() else {
            return;
        };
        let (first, last) = (self.first_column, self.last_column);
        if column == first {
            self.first_column = (first + 1..=last)
                .find(|column| columns[*column as usize] > 0)
                .unwrap_or(last);
        }
        if column == last {
            self.last_column = (first..last)
                .rev()
                .find(|column| columns[*column as usize] > 0)
                .unwrap_or(first);
        }
    }

    /// The first and the last column holding a cell, `None` for no cell.
    fn span(&self) -> Option<(u32, u32)> {
        (self.cells > 0).then_some((self.first_column, self.last_column))
    }
}

impl fmt::Debug for Extent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Extent")
            .field("cells", &self.cells)
            .field("columns", &self.span())
            .finish()
    }
}

/// What a few cells state beyond their value, held beside them by the
/// sheet rather than in every cell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CellExtra {
    /// The shared string item the cell was read from, where interning its
    /// text would name another: a rich or phonetic item, or a duplicate.
    shared_string: Option<u32>,
    /// The runs of a rich inline string and the text they spell, written
    /// back while the cell holds that text.
    inline_runs: Option<Arc<RichInline>>,
    /// `cm`: the cell metadata record a dynamic array's anchor names.
    cell_metadata: Option<u32>,
    /// `vm`: the value metadata record a rich value names.
    value_metadata: Option<u32>,
    /// `ph`: the cell shows its phonetic text.
    phonetic: bool,
    /// What the `<f>` states beside its text: an array's or a data table's
    /// range and inputs, a calculation flag.
    formula: Option<Box<FormulaAttributes>>,
    /// The numeric temporal cache when typed milliseconds cannot reproduce its bits.
    original_serial_bits: Option<u64>,
}

/// A rich inline string: the raw inside of its `<is>` - its runs, their
/// fonts, its phonetic text - and the text they spell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RichInline {
    text: crate::Str,
    runs: Arc<[u8]>,
}

impl CellExtra {
    /// What `raw`, read as `cell`, states beyond its value, `None` when it
    /// states nothing.
    fn of(
        raw: &RawCell,
        cell: &Cell,
        strings: &SharedStrings,
        serial: Option<u64>,
    ) -> Option<Self> {
        let shared_string = (raw.kind == CellKind::SharedString)
            .then(|| raw.content.trim().parse::<u32>().ok())
            .flatten()
            .filter(|index| !strings.is_reusable(*index as usize));
        let extra = Self {
            shared_string,
            inline_runs: raw.inline_runs.as_ref().map(|runs| {
                Arc::new(RichInline {
                    text: crate::Str::new(cell.text()),
                    runs: Arc::clone(runs),
                })
            }),
            cell_metadata: raw.cell_metadata,
            value_metadata: raw.value_metadata,
            phonetic: raw.phonetic,
            formula: raw
                .formula
                .as_ref()
                .and_then(|formula| formula.attributes.kept()),
            original_serial_bits: serial,
        };
        (extra != Self::default()).then_some(extra)
    }

    fn serial(&self) -> Option<f64> {
        self.original_serial_bits.map(f64::from_bits)
    }

    pub(crate) fn exceptional_serial(cell: &Cell, raw: f64, system: DateSystem) -> Option<u64> {
        if cell.kind() != CellKind::Number || cell.value().temporal_unit().is_none() {
            return None;
        }
        // A valid last-day serial can round to midnight in year 10000 at
        // millisecond resolution; that typed value cannot be reserialized.
        let canonical = system
            .serial_of(cell.value())
            .ok()
            .flatten()
            .map(|(serial, _)| serial);
        canonical
            .is_none_or(|serial| raw.to_bits() != serial.to_bits())
            .then_some(raw.to_bits())
    }
}

/// One worksheet, every cell of it in memory.
///
/// Held state: every cell the part stated, until the sheet is dropped; the
/// reason is the random access - a cell at any reference, in either
/// direction, without a second read of the part. The bound is the cells:
/// at most 80 bytes each, at most 64 bytes per row that holds one plus the
/// slack of its cell vector, one 64 KiB table of column counts while the
/// sheet holds any cell, and one map entry per cell stating a fact only a
/// few cells state (`cm`, `vm`, `ph`, a shared string item interning would
/// not name, a rich inline string's runs, an array formula's range), none
/// for any other. Beside the cells: one map entry per row stating a format
/// (Excel's `x14ac:dyDescent` makes that every row it writes, the attribute
/// text shared by the rows repeating it), one span per `<col>`, and - for a
/// sheet read from a part - what the part states outside its cells, as it
/// was written, for the part's size.
///
/// Every change through `&mut self` moves [`Self::revision`], so a holder
/// learns whether the sheet changed since it last looked without comparing
/// cells.
///
/// A cell's [`StyleId`] indexes the styles of one workbook: the one the
/// sheet was read from, or the one it was first put in. Put into another
/// workbook, the sheet keeps every value, formula and number format, and
/// its cells take that workbook's default style - an index into one
/// workbook's styles means nothing in another's - their temporal cells
/// written under the styles that workbook interns for them.
///
/// ```
/// use yggdryl::excel::{CellRange, CellRef, Sheet};
/// use yggdryl::{Field, Scalar, Serie};
///
/// let mut sheet = Sheet::new("Trades")?;
/// sheet.set_cell("A1".parse()?, "symbol")?;
/// sheet.set_cell("B1".parse()?, "price")?;
/// sheet.set_cell((1, 0).into(), "AAPL")?;
/// sheet.set_cell((1, 1).into(), 187.23)?;
///
/// assert_eq!(sheet.scalar((1, 1).into()), Scalar::from(187.23));
/// assert_eq!(sheet.scalar((5, 5).into()), Scalar::Null);
/// assert_eq!(sheet.dimension(), Some("A1:B2".parse::<CellRange>()?));
/// assert_eq!(sheet.cell_count(), 4);
///
/// // The header row names the columns; every other row is a record.
/// let rows = sheet.clone().into_serie(None, yggdryl::RecordHeader::Source, Default::default())?;
/// assert_eq!(rows.len(), 1);
/// assert_eq!(rows.child("price").and_then(|price| price.scalar(0).ok()), Some(Scalar::from(187.23)));
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct Sheet {
    name: SmolStr,
    state: SheetState,
    system: DateSystem,
    rows: BTreeMap<u32, Row>,
    extent: Extent,
    /// Coordinates addressed by record writes, including cells that state null.
    /// This is in-memory import intent, not an OOXML dimension claim.
    record_footprint: Option<CellRange>,
    /// Sparse: an entry only for a cell stating one of its facts, moved with
    /// the cell by every edit and dropped with it.
    extras: BTreeMap<CellRef, CellExtra>,
    /// Row and column formats, merges, the frozen pane and the defaults.
    layout: Layout,
    /// What the part states outside the cells, carried; `None` for a sheet
    /// built in memory.
    frame: Option<Box<WorksheetFrame>>,
    revision: u64,
    /// The workbook whose styles the cells' style ids index, `None` for a
    /// sheet in no workbook yet.
    origin: Option<u64>,
    /// Absent until explicit calculation; ordinary editing pays no journal allocation.
    changes: Option<Box<ChangeSet>>,
}

impl PartialEq for Sheet {
    /// Two sheets are equal when they hold the same cells and the same
    /// layout under the same name, state and date system; how often either
    /// changed, which workbook's styles its ids index and what it carries
    /// from the part it was read from say nothing about what it holds.
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.state == other.state
            && self.system == other.system
            && self.rows == other.rows
            && self.extras == other.extras
            && self.layout == other.layout
    }
}

impl Sheet {
    /// An empty sheet named `name`, under the 1900 date system.
    ///
    /// # Errors
    ///
    /// Returns the name's refusal ([`validate_sheet_name`]).
    pub fn new(name: impl Into<SmolStr>) -> Result<Self> {
        let name = name.into();
        validate_sheet_name(&name)?;
        Ok(Self::held(name, SheetState::Visible, DateSystem::Year1900))
    }

    /// An empty sheet of a name already proven.
    fn held(name: SmolStr, state: SheetState, system: DateSystem) -> Self {
        Self {
            name,
            state,
            system,
            rows: BTreeMap::new(),
            extent: Extent::default(),
            record_footprint: None,
            extras: BTreeMap::new(),
            layout: Layout::default(),
            frame: None,
            revision: 0,
            origin: None,
            changes: None,
        }
    }

    /// This sheet counting its serial dates from `system`.
    #[must_use]
    pub fn with_date_system(mut self, system: DateSystem) -> Self {
        if self.system != system {
            self.system = system;
            self.clear_serials();
            self.note_structure();
        }
        self
    }

    fn clear_serials(&mut self) {
        self.extras.retain(|_, extra| {
            extra.original_serial_bits = None;
            *extra != CellExtra::default()
        });
    }

    /// This sheet in `state`.
    #[must_use]
    pub const fn with_state(mut self, state: SheetState) -> Self {
        self.state = state;
        self
    }

    /// The sheet's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Rename the sheet.
    ///
    /// # Errors
    ///
    /// Returns the name's refusal ([`validate_sheet_name`]).
    pub fn set_name(&mut self, name: impl Into<SmolStr>) -> Result<()> {
        let name = name.into();
        validate_sheet_name(&name)?;
        if self.name != name {
            self.name = name;
            self.note_structure();
            self.changed();
        }
        Ok(())
    }

    /// Whether the sheet shows in the tabs.
    #[must_use]
    pub const fn state(&self) -> SheetState {
        self.state
    }

    /// Count serial dates from `system` when the sheet is written; the
    /// cells hold typed values, so nothing else changes. A workbook sets
    /// every sheet it holds when its own system is set.
    pub fn set_date_system(&mut self, system: DateSystem) {
        if self.system != system {
            self.system = system;
            self.clear_serials();
            self.note_structure();
            self.changed();
        }
    }

    /// Show or hide the sheet.
    pub fn set_state(&mut self, state: SheetState) {
        if self.state != state {
            self.state = state;
            self.changed();
        }
    }

    /// The date system the sheet's serials are read and written under.
    #[must_use]
    pub const fn date_system(&self) -> DateSystem {
        self.system
    }

    /// How many changes the sheet has taken since it was built or read:
    /// every `&mut self` call that changed what it holds moves it, and
    /// nothing else does.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Restore the stamp after an attempted edit restored its payload.
    /// Successful undo is a new edit and must not use this reset.
    pub(crate) fn reset_revision(&mut self, revision: u64) {
        self.revision = revision;
    }

    /// Activate only at the first explicit calculation. A new generation or
    /// previously inactive replacement requires a complete formula rebuild.
    pub(crate) fn track_changes(&mut self) -> &ChangeSet {
        self.changes
            .get_or_insert_with(|| Box::new(ChangeSet::default()))
    }

    pub(crate) fn changes(&self) -> Option<&ChangeSet> {
        self.changes.as_deref()
    }

    pub(crate) fn acknowledge_changes(&mut self) {
        if let Some(changes) = self.changes.as_mut() {
            changes.acknowledge();
        }
    }

    pub(crate) fn change_mark(&self) -> Option<ChangeMark> {
        self.changes.as_ref().map(|changes| changes.mark())
    }

    /// Restore only journal bookkeeping after the payload inverse succeeds.
    /// Workbook guards own mark nesting and prohibit calculation in an attempt.
    pub(crate) fn restore_change_mark(&mut self, mark: Option<ChangeMark>) -> Result<()> {
        if let Some(mark) = mark {
            if !self
                .changes
                .as_ref()
                .is_some_and(|changes| changes.accepts(&mark))
            {
                return Err(Error::Conflict {
                    expected: "a change mark from this sheet instance and calculation epoch",
                    actual: "a foreign, acknowledged or later change mark",
                    path: format_smolstr!("{}!changes", self.name),
                });
            }
            self.changes
                .as_mut()
                .expect("the mark was checked")
                .rollback(mark);
        } else {
            self.changes = None;
        }
        Ok(())
    }

    fn note_cell(&mut self, at: CellRef, formula: bool) {
        if let Some(changes) = self.changes.as_mut() {
            changes.cell(at, formula);
        }
    }

    fn note_structure(&mut self) {
        if let Some(changes) = self.changes.as_mut() {
            changes.structural = true;
        }
    }

    /// How many rows hold a cell.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether the sheet holds no cell.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// How many cells the sheet holds, answered without a walk.
    #[must_use]
    pub const fn cell_count(&self) -> usize {
        self.extent.cells
    }

    /// The rectangle the cells span, `None` for an empty sheet: the first
    /// and the last row holding a cell, and the column span the sheet keeps
    /// as cells come and go.
    #[must_use]
    pub fn dimension(&self) -> Option<CellRange> {
        let (first_column, last_column) = self.extent.span()?;
        let first_row = *self.rows.keys().next()?;
        let last_row = *self.rows.keys().next_back()?;
        Some(CellRange::new(
            CellRef::new(first_row, first_column),
            CellRef::new(last_row, last_column),
        ))
    }

    /// The addressed rectangle of a record import, including explicit nulls;
    /// ordinary geometry remains cell-only. Existing cells outside a later
    /// record write still belong to a Sheet handed to Land.
    pub(crate) fn addressed_span(&self) -> Option<CellRange> {
        match (self.record_footprint, self.dimension()) {
            (Some(written), Some(cells)) => Some(Self::enclose(written, cells)),
            (Some(written), None) => Some(written),
            (None, cells) => cells,
        }
    }

    fn enclose(first: CellRange, second: CellRange) -> CellRange {
        CellRange::new(
            CellRef::new(
                first.start().row().min(second.start().row()),
                first.start().column().min(second.start().column()),
            ),
            CellRef::new(
                first.end().row().max(second.end().row()),
                first.end().column().max(second.end().column()),
            ),
        )
    }

    fn record_intersection(first: CellRange, second: CellRange) -> Option<CellRange> {
        first.intersects(second).then(|| {
            CellRange::new(
                CellRef::new(
                    first.start().row().max(second.start().row()),
                    first.start().column().max(second.start().column()),
                ),
                CellRef::new(
                    first.end().row().min(second.end().row()),
                    first.end().column().min(second.end().column()),
                ),
            )
        })
    }

    pub(crate) const fn record_footprint(&self) -> Option<CellRange> {
        self.record_footprint
    }

    pub(crate) fn restore_record_footprint(&mut self, span: Option<CellRange>) {
        if self.record_footprint != span {
            self.record_footprint = span;
            self.changed();
        }
    }

    pub(crate) fn add_record_footprint(&mut self, range: CellRange) {
        let span = Some(
            self.record_footprint
                .map_or(range, |held| Self::enclose(held, range)),
        );
        self.restore_record_footprint(span);
    }

    /// Preserve a physical record row whose values are all null after a
    /// landing. The row has no fabricated cell or datatype.
    pub(crate) fn ensure_record_row(&mut self, row: u32) {
        if !self.rows.contains_key(&row) {
            if let std::collections::btree_map::Entry::Vacant(entry) = self.layout.rows.entry(row) {
                entry.insert(RowFormat::default());
                self.changed();
            }
        }
    }

    /// The width of zero-based `column`, in the file's character units
    /// padding included: what its `<col>` states, else
    /// [`Self::default_column_width`].
    #[must_use]
    pub fn column_width(&self, column: u32) -> f64 {
        self.layout
            .columns
            .get(column)
            .and_then(|format| format.width)
            .unwrap_or_else(|| self.default_column_width())
    }

    /// The width a column stating none takes: the sheet's
    /// `defaultColWidth`, else Excel's 64-pixel column,
    /// [`DEFAULT_COLUMN_WIDTH`].
    #[must_use]
    pub fn default_column_width(&self) -> f64 {
        self.layout
            .default_column_width
            .unwrap_or(DEFAULT_COLUMN_WIDTH)
    }

    /// The height of zero-based `row`, in points: what its `<row>` states,
    /// else [`Self::default_row_height`].
    #[must_use]
    pub fn row_height(&self, row: u32) -> f64 {
        self.layout
            .rows
            .get(&row)
            .and_then(|format| format.height)
            .unwrap_or_else(|| self.default_row_height())
    }

    /// The height a row stating none takes: the sheet's `defaultRowHeight`,
    /// else [`DEFAULT_ROW_HEIGHT`].
    #[must_use]
    pub fn default_row_height(&self) -> f64 {
        self.layout.default_row_height.unwrap_or(DEFAULT_ROW_HEIGHT)
    }

    /// Whether zero-based `row` is hidden.
    #[must_use]
    pub fn is_row_hidden(&self, row: u32) -> bool {
        self.layout
            .rows
            .get(&row)
            .is_some_and(|format| format.hidden)
    }

    /// Whether zero-based `column` is hidden.
    #[must_use]
    pub fn is_column_hidden(&self, column: u32) -> bool {
        self.layout
            .columns
            .get(column)
            .is_some_and(|format| format.hidden)
    }

    /// The style a cell of zero-based `row` that holds none shows, when the
    /// row states one it applies.
    #[must_use]
    pub fn row_style(&self, row: u32) -> Option<StyleId> {
        self.layout
            .rows
            .get(&row)
            .and_then(RowFormat::applied_style)
    }

    /// The style an empty cell of zero-based `column` shows, when its
    /// `<col>` states one.
    #[must_use]
    pub fn column_style(&self, column: u32) -> Option<StyleId> {
        self.layout
            .columns
            .get(column)
            .and_then(|format| format.style)
    }

    /// The merged ranges, in the order the sheet lists them.
    pub fn merges(&self) -> impl Iterator<Item = CellRange> + '_ {
        self.layout.merges.iter().copied()
    }

    /// The frozen pane of the sheet's first view, `None` when nothing is
    /// frozen; a split pane that scrolls is carried and is no frozen pane.
    #[must_use]
    pub const fn frozen(&self) -> Option<Frozen> {
        self.layout.pane
    }

    /// Freeze `frozen` rows and columns in the sheet's first view, or
    /// unfreeze it with `None` - or a pane freezing nothing. The pane and
    /// selection written from it replace the ones the view states.
    ///
    /// ```
    /// use yggdryl::excel::{Frozen, Sheet};
    ///
    /// let mut sheet = Sheet::new("Trades")?;
    /// sheet.set_frozen(Some(Frozen { rows: 1, columns: 0 }))?;
    /// assert_eq!(sheet.frozen().map(|pane| pane.top_left().to_string()), Some("A2".into()));
    /// sheet.set_frozen(Some(Frozen { rows: 0, columns: 0 }))?;
    /// assert_eq!(sheet.frozen(), None);
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a pane freezing every row or
    /// every column, which leaves nothing to scroll.
    pub fn set_frozen(&mut self, frozen: Option<Frozen>) -> Result<()> {
        let frozen = frozen.filter(|frozen| frozen.rows > 0 || frozen.columns > 0);
        if let Some(pane) = frozen {
            if pane.rows >= MAX_ROWS || pane.columns >= MAX_COLUMNS {
                return Err(self.refused(
                    "pane",
                    format_smolstr!(
                        "expected fewer than {MAX_ROWS} rows and {MAX_COLUMNS} columns frozen, got \
                         {} rows and {} columns",
                        pane.rows,
                        pane.columns
                    ),
                ));
            }
        }
        if self.layout.pane != frozen {
            self.layout.pane = frozen;
            self.changed();
        }
        Ok(())
    }

    /// Give the columns of `columns` the width `width`, in the file's
    /// character units padding included, or - `None` - the sheet's default.
    ///
    /// ```
    /// use yggdryl::excel::Sheet;
    ///
    /// let mut sheet = Sheet::new("Trades")?;
    /// sheet.set_column_width(1..3, Some(20.0))?;
    /// assert_eq!((sheet.column_width(0), sheet.column_width(2)), (sheet.default_column_width(), 20.0));
    /// sheet.set_column_width(1..3, None)?;
    /// assert_eq!(sheet.column_width(2), sheet.default_column_width());
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for columns outside the grid or a
    /// width outside 0 to 255.
    pub fn set_column_width(&mut self, columns: Range<u32>, width: Option<f64>) -> Result<()> {
        self.require_band(Axis::Columns, &columns)?;
        if let Some(width) =
            width.filter(|width| !(width.is_finite() && (0.0..=255.0).contains(width)))
        {
            return Err(self.refused(
                "col",
                format_smolstr!("expected a column width from 0 to 255, got {width}"),
            ));
        }
        if self.layout.update_columns(columns, |format| {
            format.width = width;
            format.custom_width = width.is_some();
            if width.is_none() {
                format.best_fit = false;
            }
        }) {
            self.changed();
        }
        Ok(())
    }

    /// Give the rows of `rows` the height `height`, in points, or - `None` -
    /// the sheet's default.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for rows outside the grid or a height
    /// outside 0 to 409.
    pub fn set_row_height(&mut self, rows: Range<u32>, height: Option<f64>) -> Result<()> {
        self.require_band(Axis::Rows, &rows)?;
        if let Some(height) =
            height.filter(|height| !(height.is_finite() && (0.0..=409.0).contains(height)))
        {
            return Err(self.refused(
                "row",
                format_smolstr!("expected a row height from 0 to 409 points, got {height}"),
            ));
        }
        if self.layout.update_rows(rows, |format| {
            format.height = height;
            format.custom_height = height.is_some();
        }) {
            self.changed();
        }
        Ok(())
    }

    /// Hide the columns of `columns`, or show them.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for columns outside the grid.
    pub fn set_columns_hidden(&mut self, columns: Range<u32>, hidden: bool) -> Result<()> {
        self.require_band(Axis::Columns, &columns)?;
        if self
            .layout
            .update_columns(columns, |format| format.hidden = hidden)
        {
            self.changed();
        }
        Ok(())
    }

    /// Hide the rows of `rows`, or show them.
    ///
    /// ```
    /// use yggdryl::excel::Sheet;
    ///
    /// let mut sheet = Sheet::new("Trades")?;
    /// sheet.set_rows_hidden(4..6, true)?;
    /// assert!(sheet.is_row_hidden(5) && !sheet.is_row_hidden(6));
    /// sheet.set_rows_hidden(0..10, false)?;
    /// assert!(!sheet.is_row_hidden(5));
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for rows outside the grid.
    pub fn set_rows_hidden(&mut self, rows: Range<u32>, hidden: bool) -> Result<()> {
        self.require_band(Axis::Rows, &rows)?;
        if self
            .layout
            .update_rows(rows, |format| format.hidden = hidden)
        {
            self.note_structure();
            self.changed();
        }
        Ok(())
    }

    /// Merge the cells of `range` into one, which shows its top-left cell:
    /// every other cell of the range is taken out and answered, as Excel
    /// keeps only the upper-left value.
    ///
    /// ```
    /// use yggdryl::excel::Sheet;
    ///
    /// let mut sheet = Sheet::new("Report")?;
    /// sheet.set_cell("A1".parse()?, "Title")?;
    /// sheet.set_cell("B1".parse()?, "lost")?;
    /// let cleared = sheet.merge("A1:C1".parse()?)?;
    /// assert_eq!(cleared.len(), 1);
    /// assert_eq!(sheet.merges().map(|merge| merge.to_string()).collect::<Vec<_>>(), ["A1:C1"]);
    /// assert!(sheet.merge("C1:D2".parse()?).is_err());
    /// assert!(sheet.unmerge("B1".parse()?));
    /// assert_eq!(sheet.merges().count(), 0);
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a range of one cell, or one
    /// overlapping a merge already there, naming both.
    pub fn merge(&mut self, range: CellRange) -> Result<Vec<Cell>> {
        if range.start() == range.end() {
            return Err(self.refused(
                "mergeCell",
                format_smolstr!("expected a range of more than one cell to merge, got {range}"),
            ));
        }
        if let Some(held) = self
            .layout
            .merges
            .iter()
            .find(|held| held.intersects(range))
        {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{}!{range}", self.name),
                reason: format_smolstr!(
                    "expected a range overlapping no merge, got one overlapping {held}"
                ),
            });
        }
        let anchor = range.start();
        let cleared: Vec<CellRef> = self
            .cells_in(range)
            .map(Cell::reference)
            .filter(|at| *at != anchor)
            .collect();
        let cleared = cleared
            .into_iter()
            .filter_map(|at| self.remove_cell(at))
            .collect();
        self.layout.merges.push(range);
        self.changed();
        Ok(cleared)
    }

    /// Take apart every merge `range` meets, answering whether one was.
    pub fn unmerge(&mut self, range: CellRange) -> bool {
        let before = self.layout.merges.len();
        self.layout.merges.retain(|held| !held.intersects(range));
        let changed = self.layout.merges.len() != before;
        if changed {
            self.changed();
        }
        changed
    }

    /// Where Ctrl and an arrow go from `from`: along a run of cells holding
    /// something, to its last one; from an empty cell, or the last of a
    /// run, to the next cell holding something - or to the grid's edge when
    /// none does.
    ///
    /// ```
    /// use yggdryl::excel::{Direction, Sheet};
    ///
    /// let mut sheet = Sheet::new("Data")?;
    /// for at in ["A1", "A2", "A3", "A7"] {
    ///     sheet.set_cell(at.parse()?, 1.0)?;
    /// }
    /// let edge = |from: &str, direction| sheet.edge(from.parse().unwrap(), direction).to_string();
    /// assert_eq!(edge("A1", Direction::Down), "A3");
    /// assert_eq!(edge("A3", Direction::Down), "A7");
    /// assert_eq!(edge("A7", Direction::Down), "A1048576");
    /// assert_eq!(edge("A7", Direction::Up), "A3");
    /// assert_eq!(edge("A2", Direction::Right), "XFD2");
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    #[must_use]
    pub fn edge(&self, from: CellRef, direction: Direction) -> CellRef {
        let (axis, forward) = match direction {
            Direction::Up => (Axis::Rows, false),
            Direction::Down => (Axis::Rows, true),
            Direction::Left => (Axis::Columns, false),
            Direction::Right => (Axis::Columns, true),
        };
        let index = axis.of(from);
        let last = axis.limit() - 1;
        if (forward && index >= last) || (!forward && index == 0) {
            return from;
        }
        // The indices holding something along the line through `from`,
        // from the one past it outward.
        let line: Vec<u32> = match axis {
            Axis::Rows => {
                let column = from.column();
                let holding = |(row, cells): (&u32, &Row)| {
                    cells
                        .cell(column)
                        .filter(|cell| cell.has_content())
                        .map(|_| *row)
                };
                if forward {
                    self.rows.range(index + 1..).filter_map(holding).collect()
                } else {
                    self.rows.range(..index).rev().filter_map(holding).collect()
                }
            }
            Axis::Columns => {
                let Some(row) = self.rows.get(&from.row()) else {
                    return axis.with(from, if forward { last } else { 0 });
                };
                let holding = |cell: &Cell| cell.has_content().then_some(cell.column());
                if forward {
                    row.cells
                        .iter()
                        .filter(|cell| cell.column() > index)
                        .filter_map(holding)
                        .collect()
                } else {
                    row.cells
                        .iter()
                        .rev()
                        .filter(|cell| cell.column() < index)
                        .filter_map(holding)
                        .collect()
                }
            }
        };
        let step = |at: u32| if forward { at + 1 } else { at - 1 };
        let here = self.cell(from).is_some_and(Cell::has_content);
        let next = step(index);
        if here && line.first() == Some(&next) {
            // The run's last cell: where the next index is no longer held.
            let mut at = next;
            for held in &line[1..] {
                if *held != step(at) {
                    break;
                }
                at = *held;
            }
            return axis.with(from, at);
        }
        axis.with(
            from,
            line.first()
                .copied()
                .unwrap_or(if forward { last } else { 0 }),
        )
    }

    /// The region around `at` Ctrl+A selects: the smallest rectangle holding
    /// `at` whose every neighbouring cell - diagonals included - is empty.
    ///
    /// ```
    /// use yggdryl::excel::Sheet;
    ///
    /// let mut sheet = Sheet::new("Data")?;
    /// for at in ["B2", "C2", "C3", "D4", "F9"] {
    ///     sheet.set_cell(at.parse()?, 1.0)?;
    /// }
    /// assert_eq!(sheet.current_region("B2".parse()?).to_string(), "B2:D4");
    /// assert_eq!(sheet.current_region("F9".parse()?).to_string(), "F9");
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    #[must_use]
    pub fn current_region(&self, at: CellRef) -> CellRange {
        let (mut top, mut left, mut bottom, mut right) =
            (at.row(), at.column(), at.row(), at.column());
        let held_in = |row: u32, first: u32, last: u32| {
            self.rows
                .get(&row)
                .is_some_and(|cells| cells.between(first, last).iter().any(Cell::has_content))
        };
        loop {
            let first = left.saturating_sub(1);
            let last = (right + 1).min(MAX_COLUMNS - 1);
            let mut grew = false;
            if top > 0 && held_in(top - 1, first, last) {
                top -= 1;
                grew = true;
            }
            if bottom + 1 < MAX_ROWS && held_in(bottom + 1, first, last) {
                bottom += 1;
                grew = true;
            }
            let from = top.saturating_sub(1);
            let to = (bottom + 1).min(MAX_ROWS - 1);
            let column_held = |column: u32| {
                self.rows
                    .range(from..=to)
                    .any(|(_, cells)| cells.cell(column).is_some_and(Cell::has_content))
            };
            if left > 0 && column_held(left - 1) {
                left -= 1;
                grew = true;
            }
            if right + 1 < MAX_COLUMNS && column_held(right + 1) {
                right += 1;
                grew = true;
            }
            if !grew {
                return CellRange::new(CellRef::new(top, left), CellRef::new(bottom, right));
            }
        }
    }

    /// Refuse a band of `axis` outside the grid or holding nothing.
    fn require_band(&self, axis: Axis, band: &Range<u32>) -> Result<()> {
        if band.start >= band.end || band.end > axis.limit() {
            return Err(self.refused(
                if axis == Axis::Rows { "row" } else { "col" },
                format_smolstr!(
                    "expected {} from 1 to {}, first before last, got {} to {}",
                    axis.noun(),
                    axis.limit(),
                    band.start + 1,
                    band.end
                ),
            ));
        }
        Ok(())
    }

    /// A refusal naming the sheet and the element.
    fn refused(&self, element: &str, reason: SmolStr) -> Error {
        Error::InvalidRecord {
            path: format_smolstr!("{}!{element}", self.name),
            reason,
        }
    }

    /// Everything the sheet holds, spelled out: its cells - each formula
    /// in its file spelling where it stands - what they state beside them,
    /// its layout and what its part carries.
    #[cfg(feature = "internals")]
    pub(crate) fn describe(&self) -> String {
        use std::fmt::Write as _;

        let mut text = String::new();
        let _ = writeln!(text, "  {} {:?} {:?}", self.name, self.state, self.system);
        for cell in self.cells() {
            let _ = writeln!(
                text,
                "  cell {} {:?} kind={:?} format={:?} style={} error={:?} formula={:?}",
                cell.reference(),
                cell.value(),
                cell.kind(),
                cell.format(),
                cell.style().as_u16(),
                cell.error(),
                cell.formula()
                    .map(|formula| formula.at(cell.reference()).to_string())
            );
        }
        for (at, extra) in &self.extras {
            let _ = writeln!(text, "  extra {at} {extra:?}");
        }
        let _ = writeln!(text, "  layout {:?}", self.layout);
        if let Some(frame) = &self.frame {
            for item in &frame.items {
                let _ = writeln!(
                    text,
                    "  carried {} {}",
                    item.name,
                    String::from_utf8_lossy(&item.bytes)
                );
            }
        }
        text
    }

    /// What the part the sheet was read from states outside its cells.
    pub(crate) fn frame(&self) -> Option<&WorksheetFrame> {
        self.frame.as_deref()
    }

    /// Restore the complete envelope, including its absence. The pane
    /// recorded in the frame describes its carried views; it does not
    /// replace the independently editable pane in the model's layout.
    pub(crate) fn set_frame(&mut self, frame: Option<Box<WorksheetFrame>>) {
        if self.frame != frame {
            self.frame = frame;
            self.changed();
        }
    }

    /// Put `items` in place of the children the sheet's part carries,
    /// counting one change when they differ.
    pub(crate) fn set_frame_items(&mut self, items: Vec<super::carried::Carried>) {
        if let Some(frame) = self.frame.as_mut() {
            if frame.items != items {
                frame.items = items;
                self.changed();
            }
        } else if !items.is_empty() {
            let mut frame = WorksheetFrame::new(super::package::NamespaceFamily::Transitional);
            frame.items = items;
            self.set_frame(Some(Box::new(frame)));
        }
    }

    /// The layout: row and column formats, merges, the frozen pane.
    pub(crate) const fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Clear the content of the cell at `at` and keep its format: a cell
    /// left showing only what its row or column shows is none.
    pub(crate) fn clear_content(&mut self, at: CellRef) {
        let Some(cell) = self.cell(at) else {
            return;
        };
        let style = cell.style();
        let format = cell.format();
        let around = self
            .row_style(at.row())
            .or_else(|| self.column_style(at.column()))
            .unwrap_or_default();
        if style == around {
            self.remove_cell(at);
        } else if cell.has_content() || self.extras.contains_key(&at) {
            self.put(Cell::new(at, CellKind::Number, format, Scalar::Null).with_style(style));
        }
    }

    /// Clear the format of the cell at `at` and keep its value, read again
    /// under the default style: a date is its serial again. A blank cell
    /// left in the default style is none.
    pub(crate) fn clear_format(&mut self, at: CellRef) {
        let system = self.system;
        let Some(row) = self.rows.get_mut(&at.row()) else {
            return;
        };
        let Ok(index) = row.position(at.column()) else {
            return;
        };
        let cell = &mut row.cells[index];
        if cell.style() == StyleId::DEFAULT && cell.format() == NumberFormat::General {
            return;
        }
        let raw = self.extras.get(&at).and_then(CellExtra::serial);
        let _ = cell.restyle(StyleId::DEFAULT, NumberFormat::General, system, raw);
        if let Some(extra) = self.extras.get_mut(&at) {
            extra.original_serial_bits = None;
            if *extra == CellExtra::default() {
                self.extras.remove(&at);
            }
        }
        let empty = !cell.has_content();
        self.note_cell(at, false);
        self.changed();
        if empty && !self.extras.contains_key(&at) {
            self.remove_cell(at);
        }
    }

    /// Display the cell at `at` in `style`, whose number format reads a
    /// number as `format`: the cell's value read again under it, a blank
    /// cell put where none is unless the cell already shows `style`.
    pub(crate) fn set_cell_style(&mut self, at: CellRef, style: StyleId, format: NumberFormat) {
        if self.cell(at).is_none() && self.style_at(at) == style {
            return;
        }
        self.restyle(at, style, format);
    }

    /// Give every row of `rows` no style of its own.
    pub(crate) fn clear_row_styles(&mut self, rows: Range<u32>) {
        if self.layout.update_rows(rows, |format| {
            format.style = None;
            format.custom_format = false;
        }) {
            self.changed();
        }
    }

    /// Give every column of `columns` no style of its own.
    pub(crate) fn clear_column_styles(&mut self, columns: Range<u32>) {
        if self
            .layout
            .update_columns(columns, |format| format.style = None)
        {
            self.changed();
        }
    }

    /// Take apart every merge lying inside `range`.
    pub(crate) fn unmerge_inside(&mut self, range: CellRange) {
        let before = self.layout.merges.len();
        self.layout
            .merges
            .retain(|merge| !(range.contains(merge.start()) && range.contains(merge.end())));
        if self.layout.merges.len() != before {
            self.changed();
        }
    }

    /// Put the cells of `moved` - the cells of `block` - with `to` their new
    /// top-left cell, what they state beside them with them, each range an
    /// array formula among them states answered by `range`.
    pub(crate) fn put_moved(
        &mut self,
        moved: &Self,
        block: CellRange,
        to: CellRef,
        mut range: impl FnMut(&str) -> Option<SmolStr>,
    ) {
        let landed = |at: CellRef| {
            CellRef::new(
                to.row() + (at.row() - block.start().row()),
                to.column() + (at.column() - block.start().column()),
            )
        };
        for cell in moved.cells() {
            let at = landed(cell.reference());
            self.put(cell.clone().at(at));
        }
        if let Some(written) = moved
            .record_footprint
            .and_then(|written| Self::record_intersection(written, block))
        {
            self.add_record_footprint(CellRange::new(
                landed(written.start()),
                landed(written.end()),
            ));
            for row in moved.layout.rows.keys() {
                if *row >= block.start().row() && *row <= block.end().row() {
                    self.ensure_record_row(to.row() + (*row - block.start().row()));
                }
            }
        }
        for (at, extra) in &moved.extras {
            let mut extra = extra.clone();
            if let Some(attributes) = extra.formula.as_mut() {
                for held in [
                    &mut attributes.reference,
                    &mut attributes.first_input,
                    &mut attributes.second_input,
                ] {
                    if let Some(moved) = held.as_deref().and_then(&mut range) {
                        *held = Some(moved);
                    }
                }
            }
            self.extras.insert(landed(*at), extra);
        }
        self.changed();
    }

    /// Put the rows of `slice` - the cells of `body` - back in `order`: the
    /// row `order[i]` held lands on the `i`th row of `body`, each cell and
    /// what the sheet held beside it moving with it.
    pub(crate) fn put_sorted(&mut self, slice: &Self, body: CellRange, order: &[u32]) {
        self.restore_cells(
            &[body],
            &Self::held(self.name.clone(), self.state, self.system),
        );
        let first = body.start().row();
        for (offset, from) in order.iter().enumerate() {
            let row = first + offset as u32;
            let Some(held) = slice.rows.get(from) else {
                continue;
            };
            for cell in &held.cells {
                self.place(cell.clone().at(CellRef::new(row, cell.column())));
            }
        }
        for (at, extra) in &slice.extras {
            if let Some(offset) = order.iter().position(|from| *from == at.row()) {
                self.extras.insert(
                    CellRef::new(first + offset as u32, at.column()),
                    extra.clone(),
                );
            }
        }
        self.changed();
    }

    /// Put back what the layout stated: the formats of the rows of `rows`,
    /// the column spans, the merges and the frozen pane - each where given.
    pub(crate) fn restore_layout(
        &mut self,
        rows: Option<RowFormats>,
        columns: Option<Vec<(Range<u32>, ColumnFormat)>>,
        merges: Option<Vec<CellRange>>,
        pane: Option<Option<Frozen>>,
    ) {
        if rows.is_some() {
            self.note_structure();
        }
        if let Some((range, formats)) = rows {
            let below = self.layout.rows.split_off(&range.end);
            self.layout.rows.split_off(&range.start);
            self.layout.rows.extend(below);
            self.layout.rows.extend(formats);
        }
        if let Some(columns) = columns {
            self.layout.columns.0 = columns;
        }
        if let Some(merges) = merges {
            self.layout.merges = merges;
        }
        if let Some(pane) = pane {
            self.layout.pane = pane;
        }
        self.changed();
    }

    /// The last cell a band opening at `at` along `axis` would push off the
    /// grid, `None` when every cell stays on it.
    pub(crate) fn pushed_off(&self, axis: Axis, at: u32, count: u32) -> Option<CellRef> {
        let limit = axis.limit();
        if let Some(written) = self.record_footprint {
            let last = axis.of(written.end());
            if last >= at && last.checked_add(count).is_none_or(|moved| moved >= limit) {
                return Some(written.end());
            }
        }
        match axis {
            Axis::Rows => {
                let (last, row) = self.rows.iter().next_back()?;
                (*last >= at && last.checked_add(count).is_none_or(|moved| moved >= limit))
                    .then(|| CellRef::new(*last, row.cells.first().map_or(0, Cell::column)))
            }
            Axis::Columns => {
                let (_, last) = self.extent.span()?;
                if last < at || last.checked_add(count).is_some_and(|moved| moved < limit) {
                    return None;
                }
                self.rows
                    .values()
                    .find_map(|row| row.cells.last().filter(|cell| cell.column() == last))
                    .map(Cell::reference)
            }
        }
    }

    /// The cells a band removed along `axis` takes, as a sheet of their
    /// own: what an undo puts back.
    pub(crate) fn band_cells(&self, axis: Axis, start: u32, end: u32) -> Self {
        self.slice(axis.whole(start, end))
    }

    /// Move every cell, what the sheet holds beside it and the layout as
    /// `band` opens or closes rows or columns along `axis`: a removed cell
    /// goes, every other moves with its row or column. A cell the band
    /// would push off the grid is the caller's to refuse
    /// ([`Self::pushed_off`]).
    pub(crate) fn shift_band(&mut self, axis: Axis, band: Band) {
        self.note_structure();
        let limit = axis.limit();
        let moved = |at: CellRef| {
            band.index(axis.of(at), limit)
                .map(|index| axis.with(at, index))
        };
        let rows = std::mem::take(&mut self.rows);
        let mut extent = Extent::default();
        for (index, mut row) in rows {
            let index = match axis {
                Axis::Rows => match band.index(index, limit) {
                    Some(index) => index,
                    None => continue,
                },
                Axis::Columns => {
                    row.cells.retain_mut(|cell| match moved(cell.reference()) {
                        Some(at) => {
                            cell.move_to(at);
                            true
                        }
                        None => false,
                    });
                    if row.cells.is_empty() {
                        continue;
                    }
                    index
                }
            };
            let row = if axis == Axis::Rows {
                row.moved_to(index)
            } else {
                row
            };
            for cell in &row.cells {
                extent.add(cell.column());
            }
            self.rows.insert(index, row);
        }
        self.extent = extent;
        let extras = std::mem::take(&mut self.extras);
        self.extras = extras
            .into_iter()
            .filter_map(|(at, extra)| Some((moved(at)?, extra)))
            .collect();
        self.layout.shift(axis, band);
        self.record_footprint = self.record_footprint.and_then(|written| {
            let (first, last) = band.span(
                axis.of(written.start()),
                axis.of(written.end()),
                axis.limit(),
            )?;
            Some(CellRange::new(
                axis.with(written.start(), first),
                axis.with(written.end(), last),
            ))
        });
        self.changed();
    }

    /// Put `rewrite`'s answer - handed each formula cell's reference and
    /// formula - in place of each formula it answers one for, counting one
    /// change when any did.
    pub(crate) fn rewrite_formulas_at(
        &mut self,
        mut rewrite: impl FnMut(CellRef, &Formula) -> Option<Formula>,
    ) {
        let mut changed = false;
        for cell in self.rows.values_mut().flat_map(|row| row.cells.iter_mut()) {
            let at = cell.reference();
            if let Some(formula) = cell.formula().and_then(|formula| rewrite(at, formula)) {
                cell.set_formula(Some(formula));
                if let Some(changes) = self.changes.as_mut() {
                    changes.cell(at, true);
                }
                changed = true;
            }
        }
        if changed {
            self.changed();
        }
    }

    /// Put `rewrite`'s answer in place of each range an array formula or a
    /// data table states beside its anchor - its `ref`, `r1`, `r2` -
    /// answering what each changed one stated before.
    pub(crate) fn rewrite_formula_ranges(
        &mut self,
        mut rewrite: impl FnMut(&str) -> Option<SmolStr>,
    ) -> Vec<(CellRef, Box<FormulaAttributes>)> {
        let mut before = Vec::new();
        for (at, extra) in &mut self.extras {
            let Some(attributes) = extra.formula.as_mut() else {
                continue;
            };
            let prior = attributes.clone();
            for held in [
                &mut attributes.reference,
                &mut attributes.first_input,
                &mut attributes.second_input,
            ] {
                if let Some(moved) = held.as_deref().and_then(&mut rewrite) {
                    *held = Some(moved);
                }
            }
            if *attributes != prior {
                before.push((*at, prior));
            }
        }
        if !before.is_empty() {
            for (at, _) in &before {
                self.note_cell(*at, true);
            }
            self.changed();
        }
        before
    }

    /// The ranges each array formula, data table and dynamic array anchored
    /// on the sheet states it fills, by its anchor.
    pub(crate) fn formula_ranges(&self) -> impl Iterator<Item = (CellRef, CellRange)> + '_ {
        self.extras.iter().filter_map(|(at, extra)| {
            let attributes = extra.formula.as_deref()?;
            if !matches!(attributes.kind, FormulaKind::Array | FormulaKind::DataTable) {
                return None;
            }
            let range: CellRange = attributes.reference.as_deref()?.parse().ok()?;
            Some((*at, range))
        })
    }

    /// What the array formula or data table anchored at `at` states beside
    /// it.
    pub(crate) fn formula_attributes(&self, at: CellRef) -> Option<Box<FormulaAttributes>> {
        self.extras.get(&at)?.formula.clone()
    }

    /// Put back what an array formula or a data table stated beside its
    /// anchor at `at`.
    pub(crate) fn restore_formula_attributes(
        &mut self,
        at: CellRef,
        attributes: Box<FormulaAttributes>,
    ) {
        self.extras.entry(at).or_default().formula = Some(attributes);
        self.note_cell(at, true);
        self.changed();
    }

    /// Put the formula `formula` in the cell at `at`, when one is there.
    pub(crate) fn restore_formula(&mut self, at: CellRef, formula: Option<Formula>) {
        let Some(row) = self.rows.get_mut(&at.row()) else {
            return;
        };
        if let Ok(index) = row.position(at.column()) {
            row.cells[index].set_formula(formula);
            self.note_cell(at, true);
            self.changed();
        }
    }

    /// Put back the cells `ranges` held: each cell and what the sheet held
    /// beside it inside them taken out, and `slice`'s put in their place.
    pub(crate) fn restore_cells(&mut self, ranges: &[CellRange], slice: &Self) {
        for range in ranges {
            let taken: Vec<CellRef> = self.cells_in(*range).map(Cell::reference).collect();
            for at in taken {
                self.remove_cell(at);
            }
            let held: Vec<CellRef> = self
                .extras
                .range(
                    CellRef::new(range.start().row(), 0)
                        ..=CellRef::new(range.end().row(), u32::MAX),
                )
                .filter(|(at, _)| range.contains(**at))
                .map(|(at, _)| *at)
                .collect();
            for at in held {
                self.extras.remove(&at);
            }
        }
        for cell in slice.cells() {
            self.place(cell.clone());
        }
        for (at, extra) in &slice.extras {
            self.extras.insert(*at, extra.clone());
        }
        if let Some(written) = slice.record_footprint {
            self.add_record_footprint(written);
            for row in slice.layout.rows.keys() {
                if *row >= written.start().row() && *row <= written.end().row() {
                    self.ensure_record_row(*row);
                }
            }
        }
        self.changed();
    }

    /// The cell at `reference`, when present.
    #[must_use]
    pub fn cell(&self, reference: CellRef) -> Option<&Cell> {
        self.rows.get(&reference.row())?.cell(reference.column())
    }

    /// The cell at `reference`, mutably, when present.
    ///
    /// The borrow counts as a change of the sheet, since it can be one.
    /// The cell stays where it is: whatever the borrow puts there, the cell
    /// is back at `reference` when the borrow ends ([`CellMut`]), because
    /// its place in the row, the counts and what the sheet holds beside it
    /// all follow the reference.
    pub fn cell_mut(&mut self, reference: CellRef) -> Option<CellMut<'_>> {
        let row = self.rows.get_mut(&reference.row())?;
        let at = row.position(reference.column()).ok()?;
        // A mutable borrow may replace the formula and may be forgotten;
        // record conservatively before lending it, not only in Drop.
        if let Some(changes) = self.changes.as_mut() {
            changes.cell(reference, true);
        }
        self.revision += 1;
        Some(CellMut {
            cell: &mut row.cells[at],
            reference,
            extras: &mut self.extras,
            checked_serial: false,
            saved_serial: None,
        })
    }

    /// The value at `reference`: null where no cell is.
    #[must_use]
    pub fn scalar(&self, reference: CellRef) -> Scalar {
        self.cell(reference)
            .map_or(Scalar::Null, |cell| cell.value().clone())
    }

    /// A proven wire serial that the typed millisecond value cannot reconstruct.
    pub(crate) fn retained_serial(&self, at: CellRef) -> Option<f64> {
        self.extras.get(&at)?.serial()
    }

    /// Install already-proved exceptional bits after a cell replacement.
    pub(crate) fn attach_serial_bits(&mut self, at: CellRef, bits: u64) {
        self.extras.entry(at).or_default().original_serial_bits = Some(bits);
    }

    /// Put `value` at `reference`, answering the cell it replaced.
    ///
    /// The cell keeps the style it had, as typing into a formatted cell
    /// keeps its format - where no cell stood, the style its row or column
    /// shows - and what else the replaced cell stated, its formula and the
    /// metadata beside it, is in the cell answered, not in the one put. A
    /// temporal value is written under its style with the code of its
    /// format where that style does not read as one. To put a cell in
    /// another style, put [`Cell::from_scalar`] with [`Cell::with_style`]
    /// through [`Sheet::insert_cell`].
    ///
    /// ```
    /// use yggdryl::excel::{Cell, DateSystem, Sheet, StyleId};
    ///
    /// let mut sheet = Sheet::new("Prices")?;
    /// let at = "B2".parse()?;
    /// let styled = Cell::from_scalar(at, 1.5.into(), DateSystem::Year1900)?;
    /// sheet.insert_cell(styled.with_style(StyleId::new(2)))?;
    /// let replaced = sheet.set_cell(at, 2.0)?.expect("a cell stood there");
    /// assert_eq!(replaced.value(), &1.5.into());
    /// assert_eq!(sheet.cell(at).map(Cell::style), Some(StyleId::new(2)));
    /// sheet.insert_cell(Cell::from_scalar(at, 3.0.into(), DateSystem::Year1900)?)?;
    /// assert_eq!(sheet.cell(at).map(Cell::style), Some(StyleId::DEFAULT));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a reference outside the grid or a
    /// value the cell cannot spell ([`Cell::from_scalar`]).
    pub fn set_cell(
        &mut self,
        reference: CellRef,
        value: impl Into<Scalar>,
    ) -> Result<Option<Cell>> {
        let cell = Cell::from_scalar(reference.require_in_grid()?, value.into(), self.system)?;
        let style = self.style_at(reference);
        Ok(self.put(cell.with_style(style)))
    }

    /// Put a prebuilt cell - one carrying a formula, say - at its reference,
    /// answering the cell it replaced.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a reference outside the grid.
    pub fn insert_cell(&mut self, cell: Cell) -> Result<Option<Cell>> {
        cell.reference().require_in_grid()?;
        Ok(self.put(cell))
    }

    /// Whether this formula owns one scalar cache. Array, data-table and
    /// dynamic-array anchors own more than one result and stay held here.
    pub(crate) fn scalar_formula_at(&self, at: CellRef) -> bool {
        self.extras.get(&at).is_none_or(|extra| {
            extra.cell_metadata.is_none()
                && !extra.formula.as_deref().is_some_and(|attributes| {
                    matches!(attributes.kind, FormulaKind::Array | FormulaKind::DataTable)
                })
        })
    }

    /// Stage a typed formula cache and its raw serial as one replacement.
    pub(crate) fn plan_calculated(
        &self,
        cell: &Cell,
        result: super::formula::value::Operand,
        shown_format: NumberFormat,
    ) -> Result<Option<(Cell, Option<u64>)>> {
        use super::formula::value::Operand;
        let raw = match &result {
            Operand::Number(raw) => Some(*raw),
            Operand::Blank => Some(0.0),
            _ => None,
        };
        let changed = cell.with_calculated(result, self.system, shown_format)?;
        let candidate = changed.as_ref().unwrap_or(cell);
        let bits = raw.and_then(|raw| CellExtra::exceptional_serial(candidate, raw, self.system));
        let before = self
            .extras
            .get(&cell.reference())
            .and_then(|extra| extra.original_serial_bits);
        if changed.is_none() && before == bits {
            return Ok(None);
        }
        Ok(Some((changed.unwrap_or_else(|| cell.clone()), bits)))
    }

    /// Read one source cell through its exact retained numeric serial.
    pub(crate) fn calculation_operand_at(&self, cell: &Cell) -> super::formula::value::Outcome {
        cell.calculation_operand(self.system, self.retained_serial(cell.reference()))
    }

    /// Commit a proved formula cache and its exceptional source fact together.
    pub(crate) fn replace_calculated(&mut self, cell: Cell, bits: Option<u64>) {
        let reference = cell.reference();
        let row = self
            .rows
            .get_mut(&reference.row())
            .expect("planned formula row");
        let at = row
            .position(reference.column())
            .expect("planned formula cell");
        row.cells[at] = cell;
        if let Some(bits) = bits {
            self.extras
                .entry(reference)
                .or_default()
                .original_serial_bits = Some(bits);
        } else if let Some(extra) = self.extras.get_mut(&reference) {
            extra.original_serial_bits = None;
            if *extra == CellExtra::default() {
                self.extras.remove(&reference);
            }
        }
        self.changed();
    }

    /// Physical rows for record access, in order. Cell rows and row metadata
    /// share a key when both exist. An empty metadata row lends no cells;
    /// its stack Row keeps the existing row accessors without a new schema.
    fn record_rows(&self) -> impl Iterator<Item = std::borrow::Cow<'_, Row>> + Clone {
        use std::borrow::Cow;
        let mut cells = self.rows.values().peekable();
        let mut metadata = self.layout.rows.keys().copied().peekable();
        std::iter::from_fn(move || {
            if let Some(row) = cells.peek() {
                if metadata.peek().is_none_or(|index| row.index <= *index) {
                    if metadata.peek() == Some(&row.index) {
                        metadata.next();
                    }
                    return cells.next().map(Cow::Borrowed);
                }
            }
            metadata.next().map(|index| {
                Cow::Owned(Row {
                    index,
                    cells: Vec::new(),
                })
            })
        })
    }

    /// Cell geometry stays cell-only. Record reads additionally honour each
    /// physical row after their first header cell, including a null tail.
    fn record_end_row(&self) -> Option<u32> {
        self.rows
            .last_key_value()
            .map(|(row, _)| *row)
            .into_iter()
            .chain(self.layout.rows.last_key_value().map(|(row, _)| *row))
            .max()
    }

    /// Put `cell` in place of whatever was at its reference, what the
    /// replaced cell stated beside it going with it.
    fn put(&mut self, cell: Cell) -> Option<Cell> {
        self.extras.remove(&cell.reference());
        self.changed();
        self.place(cell)
    }

    /// Put `cell` in its row, counting it when it is new there.
    fn place(&mut self, cell: Cell) -> Option<Cell> {
        let reference = cell.reference();
        let formula = cell.formula().is_some();
        let row = self.rows.entry(reference.row()).or_insert_with(|| Row {
            index: reference.row(),
            cells: Vec::new(),
        });
        let replaced = match row.position(reference.column()) {
            Ok(at) => Some(std::mem::replace(&mut row.cells[at], cell)),
            Err(at) => {
                row.cells.insert(at, cell);
                self.extent.add(reference.column());
                None
            }
        };
        self.note_cell(
            reference,
            formula
                || replaced
                    .as_ref()
                    .is_some_and(|cell| cell.formula().is_some()),
        );
        replaced
    }

    /// Take the cell at `reference` out, when present.
    pub fn remove_cell(&mut self, reference: CellRef) -> Option<Cell> {
        let row = self.rows.get_mut(&reference.row())?;
        let at = row.position(reference.column()).ok()?;
        let removed = row.cells.remove(at);
        if row.cells.is_empty() {
            self.rows.remove(&reference.row());
        }
        self.extent.remove(reference.column());
        self.extras.remove(&reference);
        self.note_cell(reference, removed.formula().is_some());
        self.changed();
        Some(removed)
    }

    /// Count one change.
    fn changed(&mut self) {
        self.revision += 1;
    }

    /// The style the cell at `at` displays with: its own, else its row's,
    /// else its column's, else the default.
    pub(crate) fn style_at(&self, at: CellRef) -> StyleId {
        self.cell(at)
            .map(Cell::style)
            .or_else(|| self.row_style(at.row()))
            .or_else(|| self.column_style(at.column()))
            .unwrap_or_default()
    }

    /// Display the cell at `at` with `style`, whose number format reads a
    /// number as `format`: the cell's value is read again under it where
    /// that changes what it is, and a blank cell is put where none is.
    /// Answers whether anything changed.
    pub(crate) fn restyle(&mut self, at: CellRef, style: StyleId, format: NumberFormat) -> bool {
        let system = self.system;
        if let Some(row) = self.rows.get_mut(&at.row()) {
            if let Ok(index) = row.position(at.column()) {
                let cell = &mut row.cells[index];
                if cell.style() == style && cell.format() == format {
                    return false;
                }
                let raw = self.extras.get(&at).and_then(CellExtra::serial);
                let used = cell.restyle(style, format, system, raw);
                let bits = used.and_then(|raw| CellExtra::exceptional_serial(cell, raw, system));
                if let Some(bits) = bits {
                    self.extras.entry(at).or_default().original_serial_bits = Some(bits);
                } else if let Some(extra) = self.extras.get_mut(&at) {
                    extra.original_serial_bits = None;
                    if *extra == CellExtra::default() {
                        self.extras.remove(&at);
                    }
                }
                self.note_cell(at, false);
                self.changed();
                return true;
            }
        }
        self.place(Cell::new(at, CellKind::Number, format, Scalar::Null).with_style(style));
        self.changed();
        true
    }

    /// Give the row `row` the style `style` for the cells it holds none of.
    pub(crate) fn set_row_style(&mut self, row: u32, style: StyleId) {
        let format = self.layout.rows.entry(row).or_default();
        if format.style != Some(style) || !format.custom_format {
            format.style = Some(style);
            format.custom_format = true;
            self.changed();
        }
    }

    /// The columns of `columns` in runs of one style: what each `<col>`
    /// span states, and the gaps between them as `None`.
    pub(crate) fn column_runs(&self, columns: Range<u32>) -> Vec<(Range<u32>, Option<StyleId>)> {
        let mut runs = Vec::new();
        let mut at = columns.start;
        for (span, format) in &self.layout.columns.0 {
            if span.end <= at {
                continue;
            }
            if span.start >= columns.end {
                break;
            }
            if span.start > at {
                runs.push((at..span.start, None));
            }
            let start = span.start.max(at);
            let end = span.end.min(columns.end);
            runs.push((start..end, format.style));
            at = end;
        }
        if at < columns.end {
            runs.push((at..columns.end, None));
        }
        runs
    }

    /// Give each run of columns its style: a span stating one is split
    /// where a run starts or ends inside it, and a gap takes a span of the
    /// sheet's default width.
    pub(crate) fn set_column_styles(&mut self, runs: &[(Range<u32>, StyleId)]) {
        if runs.is_empty() {
            return;
        }
        let width = self.default_column_width();
        let mut spans: Vec<(Range<u32>, ColumnFormat)> = Vec::new();
        let held = std::mem::take(&mut self.layout.columns.0);
        // Every boundary a span or a run has, in order.
        let mut cuts: Vec<u32> = held
            .iter()
            .flat_map(|(span, _)| [span.start, span.end])
            .chain(runs.iter().flat_map(|(span, _)| [span.start, span.end]))
            .collect();
        cuts.sort_unstable();
        cuts.dedup();
        for pair in cuts.windows(2) {
            let piece = pair[0]..pair[1];
            let stated = held
                .iter()
                .find(|(span, _)| span.start <= piece.start && piece.end <= span.end)
                .map(|(_, format)| format.clone());
            let styled = runs
                .iter()
                .find(|(span, _)| span.start <= piece.start && piece.end <= span.end)
                .map(|(_, style)| *style);
            let format = match (stated, styled) {
                (Some(mut format), Some(style)) => {
                    format.style = Some(style);
                    format
                }
                (Some(format), None) => format,
                (None, Some(style)) => ColumnFormat {
                    width: Some(width),
                    style: Some(style),
                    ..ColumnFormat::default()
                },
                (None, None) => continue,
            };
            match spans.last_mut() {
                Some((last, previous)) if last.end == piece.start && *previous == format => {
                    last.end = piece.end;
                }
                _ => spans.push((piece, format)),
            }
        }
        if spans != held {
            self.changed();
        }
        self.layout.columns.0 = spans;
    }

    /// Every actual style reference, including empty formatted rows and columns.
    pub(crate) fn style_ids(&self) -> impl Iterator<Item = StyleId> + '_ {
        self.cells()
            .map(Cell::style)
            .chain(self.layout.rows.values().filter_map(|format| format.style))
            .chain(
                self.layout
                    .columns
                    .0
                    .iter()
                    .filter_map(|(_, format)| format.style),
            )
    }

    /// Visit the explicit cell, row and column `cellXfs` IDs of a worksheet
    /// part, changing only those `change` maps elsewhere. Main/Strict element
    /// ancestry and unqualified attributes identify the owned references;
    /// extension elements and similarly named qualified attributes are opaque.
    /// Returning `None` from every visit captures IDs without rewriting bytes.
    ///
    /// # Errors
    ///
    /// Returns a located refusal for a malformed explicit style ID, using
    /// the same sixteen-bit grammar as the worksheet reader, or invalid XML.
    pub(crate) fn rewrite_style_ids(
        bytes: &[u8],
        part: &str,
        change: impl FnMut(StyleId) -> Option<StyleId>,
    ) -> Result<Option<Vec<u8>>> {
        use super::package::{Edits, Tag};

        struct Styles<'a, F> {
            part: &'a str,
            change: F,
            // The first non-SpreadsheetML ancestor disables its whole
            // subtree. The editor's path supplies the nesting stack.
            foreign_depth: Option<usize>,
        }

        impl<F: FnMut(StyleId) -> Option<StyleId>> Edits for Styles<'_, F> {
            fn start(
                &mut self,
                path: &[SmolStr],
                attributes: &[(SmolStr, String)],
                namespace: quick_xml::name::ResolveResult<'_>,
            ) -> Result<Tag> {
                let main = matches!(
                    namespace,
                    quick_xml::name::ResolveResult::Bound(namespace)
                        if namespace.as_ref() == super::NAMESPACE.as_bytes()
                            || namespace.as_ref() == super::STRICT_NAMESPACE.as_bytes()
                );
                if !main && self.foreign_depth.is_none() {
                    self.foreign_depth = Some(path.len());
                }
                if self.foreign_depth.is_some() {
                    return Ok(Tag::Keep);
                }
                let attribute = match path {
                    [root, data, row]
                        if root == "worksheet" && data == "sheetData" && row == "row" =>
                    {
                        "s"
                    }
                    [root, data, row, cell]
                        if root == "worksheet"
                            && data == "sheetData"
                            && row == "row"
                            && cell == "c" =>
                    {
                        "s"
                    }
                    [root, columns, column]
                        if root == "worksheet" && columns == "cols" && column == "col" =>
                    {
                        "style"
                    }
                    _ => return Ok(Tag::Keep),
                };
                let Some((_, text)) = attributes.iter().find(|(key, _)| key == attribute) else {
                    return Ok(Tag::Keep);
                };
                let before =
                    StyleId::from_attribute(text).map_err(|error| Error::InvalidRecord {
                        path: format_smolstr!(
                            "{}#{}@{attribute}",
                            self.part,
                            path.iter()
                                .map(SmolStr::as_str)
                                .collect::<Vec<_>>()
                                .join("/"),
                        ),
                        reason: super::cell::wire_reason(&error),
                    })?;
                Ok(
                    match (self.change)(before).filter(|after| *after != before) {
                        Some(after) => {
                            Tag::Set(vec![(attribute.into(), Some(after.as_u16().to_string()))])
                        }
                        None => Tag::Keep,
                    },
                )
            }

            fn end(
                &mut self,
                path: &[SmolStr],
                _: usize,
                _: usize,
                _: &[(SmolStr, String)],
            ) -> Tag {
                if self.foreign_depth == Some(path.len()) {
                    self.foreign_depth = None;
                }
                Tag::Keep
            }
        }

        super::package::edit_document(
            bytes,
            &mut Styles {
                part,
                change,
                foreign_depth: None,
            },
        )
        .map_err(|error| match error {
            Error::InvalidRecord { .. } => error,
            other => Error::InvalidRecord {
                path: SmolStr::new(part),
                reason: format_smolstr!("expected a worksheet XML part, got {other}"),
            },
        })
    }

    /// Put `map`'s style in place of every style it names, cells, rows and
    /// columns alike: what a workbook adopting a package whose styles hold
    /// ones appended since does to the ids that moved. Nothing a caller
    /// sees changes, so no change is counted.
    pub(crate) fn remap_styles(&mut self, map: &HashMap<StyleId, StyleId>) {
        for cell in self.rows.values_mut().flat_map(|row| row.cells.iter_mut()) {
            if let Some(style) = map.get(&cell.style()) {
                cell.set_style(*style);
            }
        }
        let remap = |style: &mut Option<StyleId>| {
            if let Some(moved) = style.and_then(|held| map.get(&held)) {
                *style = Some(*moved);
            }
        };
        for format in self.layout.rows.values_mut() {
            remap(&mut format.style);
        }
        for (_, format) in &mut self.layout.columns.0 {
            remap(&mut format.style);
        }
    }

    /// Hold the sheet in the workbook `workbook`: a sheet whose style ids
    /// index another workbook's styles gives them up - every cell, row and
    /// column takes the default style, a conditional format its rule
    /// without its format, and what a few cells state about that workbook's
    /// metadata parts (`cm`, `vm`) goes - keeping its values, formulas and
    /// number formats.
    pub(crate) fn hold_in(&mut self, workbook: u64) {
        if self.origin.is_some_and(|origin| origin != workbook) {
            self.note_structure();
            for cell in self.rows.values_mut().flat_map(|row| row.cells.iter_mut()) {
                cell.clear_style();
            }
            self.extras.retain(|_, extra| {
                extra.cell_metadata = None;
                extra.value_metadata = None;
                extra.shared_string = None;
                *extra != CellExtra::default()
            });
            let clear = |style: &mut Option<StyleId>| *style = None;
            for format in self.layout.rows.values_mut() {
                clear(&mut format.style);
                format.custom_format = false;
            }
            for (_, format) in &mut self.layout.columns.0 {
                clear(&mut format.style);
            }
            if let Some(frame) = self.frame.as_mut() {
                frame.forget_styles();
            }
            self.changed();
        }
        // A sheet put in a new part holds none of the relationships of the
        // part it was read from.
        if let Some(frame) = self.frame.as_mut() {
            frame.detach();
        }
        self.origin = Some(workbook);
    }

    /// The rows holding a cell, in order.
    pub fn rows(&self) -> impl Iterator<Item = &Row> + '_ {
        self.rows.values()
    }

    /// The row at zero-based `index`, when it holds a cell.
    #[must_use]
    pub fn row(&self, index: u32) -> Option<&Row> {
        self.rows.get(&index)
    }

    /// Every cell, row by row.
    pub fn cells(&self) -> impl Iterator<Item = &Cell> + '_ {
        self.rows.values().flat_map(|row| row.cells.iter())
    }

    /// The cells inside `range`, row by row.
    pub fn cells_in(&self, range: CellRange) -> impl Iterator<Item = &Cell> + '_ {
        self.rows
            .range(range.start().row()..=range.end().row())
            .flat_map(move |(_, row)| {
                row.between(range.start().column(), range.end().column())
                    .iter()
            })
    }

    /// The cells of zero-based `column`, top to bottom.
    pub fn column(&self, column: u32) -> impl Iterator<Item = &Cell> + '_ {
        self.rows.values().filter_map(move |row| row.cell(column))
    }

    /// The cells inside `range`, as a sheet of their own, references kept.
    #[must_use]
    pub fn slice(&self, range: CellRange) -> Self {
        let mut sliced = Self::held(self.name.clone(), self.state, self.system);
        for cell in self.cells_in(range) {
            sliced.place(cell.clone());
        }
        sliced.record_footprint = self
            .record_footprint
            .and_then(|written| Self::record_intersection(written, range));
        if sliced.record_footprint.is_some() {
            sliced.layout.rows = self
                .layout
                .rows
                .range(range.start().row()..=range.end().row())
                .map(|(row, format)| (*row, format.clone()))
                .collect();
        }
        sliced.extras = self
            .extras
            .range(CellRef::new(range.start().row(), 0)..=CellRef::new(range.end().row(), u32::MAX))
            .filter(|(reference, _)| range.contains(**reference))
            .map(|(reference, extra)| (*reference, extra.clone()))
            .collect();
        sliced
    }

    /// The cells inside any of `ranges`, as a sheet of their own,
    /// references and what the sheet holds beside them kept.
    pub(crate) fn slice_all(&self, ranges: &[CellRange]) -> Self {
        let mut sliced = Self::held(self.name.clone(), self.state, self.system);
        for range in ranges {
            let part = self.slice(*range);
            for cell in part.cells() {
                sliced.place(cell.clone());
            }
            sliced.extras.extend(part.extras);
            sliced.layout.rows.extend(part.layout.rows);
            if let Some(written) = part.record_footprint {
                sliced.add_record_footprint(written);
            }
        }
        sliced
    }

    /// A sheet holding the rows of `serie` from `A1`: the column names in the
    /// first row with [`RecordHeader::Source`], then one row per record.
    ///
    /// A record column is laid out column by column; any other column is the
    /// one column of a record named as it is, the rule
    /// [`SerieReader::from_serie`](crate::SerieReader::from_serie) states;
    /// a run, which names no columns, is refused.
    ///
    /// # Errors
    ///
    /// Returns the name's refusal, a run, or a value no cell spells.
    pub fn from_serie(
        name: impl Into<SmolStr>,
        serie: &Serie,
        header: RecordHeader,
    ) -> Result<Self> {
        let mut sheet = Self::new(name)?;
        sheet.write_serie(CellRef::new(0, 0), serie, header)?;
        Ok(sheet)
    }

    /// Write the rows of `serie` with their top-left cell at `anchor`: the
    /// column names in the anchor's row with [`RecordHeader::Source`], then one row per
    /// record; cells already there are replaced.
    ///
    /// # Errors
    ///
    /// Returns a refusal for a run, for rows or columns that would leave
    /// the grid, or for a value no cell spells.
    pub fn write_serie(
        &mut self,
        anchor: CellRef,
        serie: &Serie,
        header: RecordHeader,
    ) -> Result<()> {
        if header == RecordHeader::Infer {
            return Err(super::options::ExcelOptions::write_error());
        }
        if let RecordHeader::Rows(levels) = header {
            return self.write_nested_serie(anchor, serie, levels);
        }
        anchor.require_in_grid()?;
        let has_header = matches!(header, RecordHeader::Source);
        let (root, columns) = record_columns(serie)?;
        let rows = serie
            .len()
            .checked_add(usize::from(has_header))
            .ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: SmolStr::new_static("the record row count overflowed the platform size"),
            })?;
        if rows > (MAX_ROWS - anchor.row()) as usize
            || columns.len() > (MAX_COLUMNS - anchor.column()) as usize
        {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: format_smolstr!(
                    "expected {rows} rows and {} columns from {anchor} to fit {MAX_ROWS} rows by \
                     {MAX_COLUMNS} columns",
                    columns.len()
                ),
            });
        }
        let mut staged = Vec::new();
        let mut row = anchor.row();
        if has_header {
            for (offset, field) in root.fields().iter().enumerate() {
                let at = CellRef::new(row, anchor.column() + offset as u32);
                staged.push((
                    at,
                    Some(Cell::from_scalar(
                        at,
                        Scalar::from(field.name()),
                        self.system,
                    )?),
                ));
            }
            row += 1;
        }
        for index in 0..serie.len() {
            for (offset, column) in columns.iter().enumerate() {
                let at = CellRef::new(row, anchor.column() + offset as u32);
                let value = column.scalar(index)?;
                let cell = if value.is_null() {
                    None
                } else {
                    Some(Cell::from_scalar(at, value, self.system)?)
                };
                staged.push((at, cell));
            }
            row += 1;
        }
        // Source consumes one physical row even when its record has zero
        // fields, so retain an empty header just as SheetXml writes it.
        self.commit_record_cells(staged, None, &[], anchor.row()..row);
        if rows != 0 && !columns.is_empty() {
            self.add_record_footprint(CellRange::new(
                anchor,
                CellRef::new(row - 1, anchor.column() + columns.len() as u32 - 1),
            ));
        }
        Ok(())
    }

    /// Commit only proven cells and merges. Header clearing is sparse, over
    /// the addressed footprint, so unrelated resident cells are untouched.
    fn commit_record_cells(
        &mut self,
        staged: Vec<(CellRef, Option<Cell>)>,
        header: Option<CellRange>,
        merges: &[CellRange],
        records: Range<u32>,
    ) {
        if let Some(header) = header {
            let stale: Vec<CellRef> = self.cells_in(header).map(Cell::reference).collect();
            for at in stale {
                self.remove_cell(at);
            }
        }
        for (at, cell) in staged {
            match cell {
                Some(cell) => {
                    self.put(cell);
                }
                None => {
                    self.remove_cell(at);
                }
            }
        }
        // Explicit null records need a physical row, not a fabricated cell.
        // Ordinary populated rows add no metadata allocation.
        for row in records {
            self.ensure_record_row(row);
        }
        for span in merges {
            if !self.layout.merges.contains(span) {
                // The Rows plan proved these spans nonoverlapping and the
                // caller proved no held merge conflicts before this commit.
                self.layout.merges.push(*span);
                self.changed();
            }
        }
    }

    /// Write a nested record under literal Rows header levels. Conversion and
    /// merge conflicts are proved before touching the held sheet.
    fn write_nested_serie(&mut self, anchor: CellRef, serie: &Serie, levels: u32) -> Result<()> {
        let (root, children) = record_columns(serie)?;
        let schema = if serie.as_struct().is_some() {
            serie.require_field()?
        } else {
            &root
        };
        let plan =
            super::records::RowsWriteLayout::compile(schema, levels, anchor, Some(serie.len()))?;
        let mut bound = plan.bind_columns(serie, &children)?;
        let last = CellRef::new(
            anchor.row() + levels - 1,
            anchor.column() + plan.leaves().len() as u32 - 1,
        );
        let header = CellRange::new(anchor, last);
        for held in &self.layout.merges {
            if held.intersects(header) && !plan.merges().contains(held) {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{}!{held}", self.name),
                    reason: format_smolstr!(
                        "expected the Rows({levels}) header merges or no merge, got existing {held}"
                    ),
                });
            }
        }
        let mut staged = Vec::new();
        for (at, text) in plan.labels() {
            staged.push((
                *at,
                Some(Cell::from_scalar(
                    *at,
                    Scalar::from(text.as_str()),
                    self.system,
                )?),
            ));
        }
        for index in 0..serie.len() {
            let row = plan.first_body_row() + index as u32;
            bound.check_row(&plan, index, row, self.name())?;
            for (offset, child) in bound.leaves().iter().enumerate() {
                let at = CellRef::new(row, anchor.column() + offset as u32);
                let value = child.scalar(index)?;
                let cell = if value.is_null() {
                    None
                } else {
                    Some(Cell::from_scalar(at, value, self.system)?)
                };
                staged.push((at, cell));
            }
        }
        self.commit_record_cells(
            staged,
            Some(header),
            plan.merges(),
            plan.first_body_row()..plan.first_body_row() + serie.len() as u32,
        );
        self.add_record_footprint(CellRange::new(
            anchor,
            CellRef::new(
                plan.first_body_row() + serie.len() as u32 - 1,
                last.column(),
            ),
        ));
        Ok(())
    }

    /// Append the rows of `serie` below the last row present, column by
    /// column from the sheet's first column, with no header.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::write_serie`] returns.
    pub fn extend_from_serie(&mut self, serie: &Serie) -> Result<()> {
        let column = self.dimension().map_or(0, |span| span.start().column());
        let anchor = CellRef::new(self.record_end_row().map_or(0, |row| row + 1), column);
        self.write_serie(anchor, serie, RecordHeader::None)
    }

    /// Lay the sheet's rows out as one record column.
    ///
    /// The first row present names columns with [`RecordHeader::Source`];
    /// [`RecordHeader::None`] uses column letters. [`RecordHeader::Rows`]
    /// lays out nested Struct header levels. Each later row is one record,
    /// null where it holds no cell. With no `field` each leaf's datatype is
    /// the one its first present value proves - `float64` for a number, the
    /// temporal a date style spells, `boolean`, `utf8` - and a later value of
    /// another datatype is refused naming the cell, so the caller declares
    /// the field. With a `field`, each leaf reads from the sheet column of
    /// the same header path (or the same position, without a header) through
    /// the field's value contract; a value it cannot hold is null under
    /// `options.safe` and refused naming the cell otherwise; a column the
    /// sheet lacks is null, or refused when the field requires it.
    ///
    /// # Errors
    ///
    /// Returns a refusal naming the sheet and the cell.
    pub fn into_serie(
        self,
        field: Option<&Field>,
        header: RecordHeader,
        options: ArrowCastOptions,
    ) -> Result<Serie> {
        if let RecordHeader::Rows(levels) = header {
            return self.rows_nested(field, levels, options);
        }
        if header == RecordHeader::Infer {
            return self.rows_infer(field, options);
        }
        let header = matches!(header, RecordHeader::Source);
        let Some(span) = self.dimension() else {
            let mut rows = self.record_rows();
            if rows.clone().next().is_none() {
                let root = match field {
                    Some(field) => field.clone().with_nullable(false),
                    None => empty_root()?,
                };
                return Serie::from_scalars(root, std::iter::empty());
            }
            if header {
                rows.next();
            }
            if let Some(field) = field {
                // No cell geometry can name columns. Reuse the same pairing
                // and located missing-value path as populated record sheets.
                let range = CellRange::all();
                let named = Header::resolve(range, FlatExtent::Sparse, header, std::iter::empty())?;
                return self.rows_under(field, &named, header, range, rows, options);
            }
            return Serie::from_scalars(
                empty_root()?,
                rows.map(|_| Scalar::from_sequence(std::iter::empty::<Scalar>())),
            );
        };
        let mut rows = self.record_rows();
        let header_row = if header { rows.next() } else { None };
        let cells = header_row
            .as_ref()
            .into_iter()
            .flat_map(|row| row.cells())
            .map(|cell| {
                let label = if !cell.has_content() || cell.kind() == CellKind::Error {
                    std::borrow::Cow::Borrowed("")
                } else {
                    cell.text()
                };
                Ok((cell.column(), Some(label)))
            });
        match field {
            Some(field) => {
                let named = Header::resolve(span, FlatExtent::Dense, header, cells)?;
                self.rows_under(field, &named, header, span, rows, options)
            }
            None => {
                let policy = if header {
                    RecordHeader::Source
                } else {
                    RecordHeader::None
                };
                let resolved = self.flat_probe(policy, span, FlatExtent::Dense, None, false)?;
                self.rows_flat_resolved(resolved, span, rows)
            }
        }
    }

    /// Rows typed by what their cells prove, the datatype of each column
    /// its first present value's.
    /// Lend held Cell facts to the same flat naming/type owner as the wire.
    fn flat_probe(
        &self,
        header: RecordHeader,
        span: CellRange,
        extent: FlatExtent<'_>,
        declared: Option<&Field>,
        safe: bool,
    ) -> Result<super::records::HeaderResolution> {
        let mut probe = HeaderProbe::new(self.name.clone(), span, header, None);
        for row in self.record_rows() {
            let first = probe.row(row.index());
            for cell in row
                .cells()
                .filter(|cell| span.contains_column(cell.column()))
            {
                let at = cell.reference();
                let label = if first && matches!(header, RecordHeader::Source | RecordHeader::Infer)
                {
                    let text = if (header == RecordHeader::Infer && !cell.kind().is_text())
                        || !cell.has_content()
                        || cell.kind() == CellKind::Error
                    {
                        SmolStr::new_static("")
                    } else {
                        SmolStr::new(cell.text())
                    };
                    Some((text, cell.kind().is_text() && !cell.is_null()))
                } else {
                    None
                };
                let dtype = if (first && header == RecordHeader::Source) || cell.is_null() {
                    None
                } else {
                    Some(
                        cell.value()
                            .dtype()
                            .map_err(|error| located(&self.name, at, error))?,
                    )
                };
                probe.cell(at, dtype, label)?;
            }
        }
        let first_row_as_data = declared.and_then(|field| {
            let pairing = probe.first_row_pairing(field)?;
            Some(self.record_rows().next().is_some_and(|row| {
                field.fields().iter().zip(&pairing).all(|(child, column)| {
                    match column.and_then(|column| row.cell(column)) {
                        Some(cell) => cell_under(cell, child, safe).is_ok(),
                        None => child.is_nullable(),
                    }
                })
            }))
        });
        probe.resolve(DEFAULT_ROOT_NAME, extent, declared, first_row_as_data)
    }

    /// The probe's Field is already resolved; landing only moves held values.
    fn rows_flat_resolved<'a>(
        &self,
        resolved: super::records::HeaderResolution,
        span: CellRange,
        body: impl Iterator<Item = std::borrow::Cow<'a, Row>> + Clone,
    ) -> Result<Serie> {
        let pairing = resolved.header.pairing(
            &resolved.field,
            FlatBinding {
                sheet: &self.name,
                range: span,
                by_name: true,
                authoritative: false,
            },
        )?;
        let rows = body.map(|row| {
            Scalar::from_sequence(pairing.iter().map(|column| {
                column
                    .and_then(|column| row.cell(column))
                    .map_or(Scalar::Null, |cell| cell.value().clone())
            }))
        });
        Serie::from_scalars(resolved.field, rows)
    }

    fn rows_infer(self, field: Option<&Field>, options: ArrowCastOptions) -> Result<Serie> {
        let merges = self.merges().collect::<Vec<_>>();
        let mut discovered = super::regions::HeldRegions::new(self.name.clone(), merges.clone());
        let mut columns = Vec::new();
        for row in self.record_rows() {
            columns.clear();
            columns.extend(
                row.cells()
                    .filter(|cell| {
                        cell.formula().is_some() || (cell.has_content() && !cell.text().is_empty())
                    })
                    .map(Cell::column),
            );
            discovered.row(row.index(), &columns)?;
        }
        let candidates = discovered.finish()?;
        let dimension = super::regions::occupied_extent(&candidates);
        let span = dimension.unwrap_or_else(CellRange::all);
        if let Some(dimension) = dimension {
            let probe = CellRange::new(
                dimension.start(),
                CellRef::new(super::cell::MAX_ROWS - 1, dimension.end().column()),
            );
            if let Some(levels) = super::records::merge_proven_depth(&self.name, probe, &merges)? {
                let chosen = RowsWindow::implicit_range(dimension, levels, merges.iter().copied())?;
                let header = RowsWindow::header_range(chosen, levels)?;
                super::regions::require_single(&candidates, Some(header), &merges)?;
                let last = RowsWindow::header_range(probe, levels)?.end().row();
                let mut body_rows = 0_u64;
                let mut typed_body = false;
                for row in self.record_rows() {
                    if row.index() <= last {
                        for cell in row
                            .cells()
                            .filter(|cell| probe.contains_column(cell.column()))
                        {
                            RowsWindow::require_inferred_label(
                                &self.name,
                                cell.reference(),
                                !cell.is_null(),
                                cell.kind().is_text(),
                            )?;
                        }
                    } else {
                        body_rows += 1;
                        for cell in row
                            .cells()
                            .filter(|cell| probe.contains_column(cell.column()) && !cell.is_null())
                        {
                            let dtype = cell
                                .value()
                                .dtype()
                                .map_err(|error| located(&self.name, cell.reference(), error))?;
                            typed_body |= dtype != DataType::utf8();
                        }
                    }
                }
                RowsWindow::require_inferred_body(body_rows, typed_body)?;
                return self.rows_nested_span(field, levels, chosen, options);
            }
        }
        super::regions::require_single(&candidates, None, &merges)?;
        let extent = if dimension.is_some() {
            FlatExtent::Dense
        } else {
            FlatExtent::Sparse
        };
        let resolved =
            self.flat_probe(RecordHeader::Infer, span, extent, field, options.is_safe())?;
        let mut rows = self.record_rows();
        if resolved.policy == RecordHeader::Source {
            rows.next();
        }
        match field {
            Some(field) => self.rows_under(
                field,
                &resolved.header,
                resolved.policy == RecordHeader::Source,
                span,
                rows,
                options,
            ),
            None => self.rows_flat_resolved(resolved, span, rows),
        }
    }
    /// Rows laid out under a declared field, each cell through the column's
    /// value contract.
    fn rows_under<'a>(
        &self,
        field: &Field,
        named: &Header,
        header: bool,
        range: CellRange,
        body: impl Iterator<Item = std::borrow::Cow<'a, Row>> + Clone,
        options: ArrowCastOptions,
    ) -> Result<Serie> {
        let root = field.clone().with_nullable(false);
        let pairing = named.pairing(
            &root,
            FlatBinding {
                sheet: &self.name,
                range,
                by_name: header,
                authoritative: false,
            },
        )?;
        let safe = options.is_safe();
        let mut rows = Vec::with_capacity(body.clone().count());
        // One buffer for every row, drained into the run each row becomes.
        let mut values = Vec::with_capacity(pairing.len());
        for row in body {
            values.clear();
            for (child, column) in root.fields().iter().zip(&pairing) {
                let cell = column.and_then(|column| row.cell(column));
                values.push(match cell {
                    Some(cell) => cell_under(cell, child, safe)
                        .map_err(|error| located(&self.name, cell.reference(), error))?,
                    None if !child.is_nullable() => {
                        return Err(Error::InvalidRecord {
                            path: format_smolstr!(
                                "{}!{}",
                                self.name,
                                CellRef::new(row.index(), column.unwrap_or(range.start().column()))
                            ),
                            reason: format_smolstr!(
                                "expected a value for the required column {}, got no cell",
                                child.name()
                            ),
                        });
                    }
                    None => Scalar::Null,
                });
            }
            rows.push(Scalar::from_sequence(values.drain(..)));
        }
        Serie::from_scalars(root, rows)
    }

    /// Write the worksheet part: what the part it was read from states
    /// outside its cells as it was read, and the model's own elements -
    /// the span, the frozen pane, the columns, every row and cell, the
    /// merges - where the schema puts them; text written against `strings`.
    ///
    /// A cell writes the style it holds. A temporal cell whose style does
    /// not read as its format - a date built in memory holds the default
    /// style - is written under its style with the format's code, which
    /// `styles` finds or interns.
    ///
    /// # Errors
    ///
    /// Returns the sink's failure, a value no cell spells, or a refusal
    /// naming a cell whose error cannot be written or whose style the
    /// workbook's styles do not hold.
    pub(crate) fn write_xml<W: Write>(
        &self,
        writer: &mut W,
        strings: &mut SharedStringsWriter<'_>,
        styles: &mut Splice<'_>,
    ) -> Result<()> {
        let fresh;
        let frame = match self.frame.as_deref() {
            Some(frame) => frame,
            None => {
                fresh = WorksheetFrame::new(super::package::NamespaceFamily::Transitional);
                &fresh
            }
        };
        let prefix = frame.prefix();
        let model = |writer: &mut W, model: Model, carried: Option<&[u8]>| {
            self.write_model(writer, model, carried, prefix, strings, styles)
        };
        frame.write(writer, model)
    }

    /// Write one of the model's elements under the element prefix `prefix`;
    /// `carried` is the `sheetViews` the part states, for that one.
    fn write_model<W: Write>(
        &self,
        writer: &mut W,
        model: Model,
        carried: Option<&[u8]>,
        prefix: &str,
        strings: &mut SharedStringsWriter<'_>,
        styles: &mut Splice<'_>,
    ) -> Result<()> {
        match model {
            Model::Dimension => match self.dimension() {
                Some(span) => write!(writer, "<{prefix}dimension ref=\"{span}\"/>")?,
                // A part that stated a span states one for an empty sheet.
                None if self.frame.is_some() => write!(writer, "<{prefix}dimension ref=\"A1\"/>")?,
                None => {}
            },
            Model::SheetViews => match carried {
                Some(views)
                    if self
                        .frame
                        .as_ref()
                        .is_some_and(|frame| frame.pane == self.layout.pane) =>
                {
                    writer.write_all(views)?;
                }
                Some(views) => {
                    writer.write_all(&super::carried::repaned(views, self.layout.pane)?)?
                }
                None => writer
                    .write_all(super::carried::sheet_views(self.layout.pane, prefix).as_bytes())?,
            },
            Model::Cols => {
                let mut text = String::new();
                self.layout.columns.write(&mut text, prefix);
                writer.write_all(text.as_bytes())?;
            }
            Model::MergeCells => {
                let mut text = String::new();
                self.layout.write_merges(&mut text, prefix);
                writer.write_all(text.as_bytes())?;
            }
            Model::SheetData => self.write_rows(writer, prefix, strings, styles)?,
        }
        Ok(())
    }

    /// Write `<sheetData>`: every row holding a cell or stating a format, in
    /// order, each with its cells.
    fn write_rows<W: Write>(
        &self,
        writer: &mut W,
        prefix: &str,
        strings: &mut SharedStringsWriter<'_>,
        styles: &mut Splice<'_>,
    ) -> Result<()> {
        write!(writer, "<{prefix}sheetData>")?;
        let mut part = PartWriter {
            writer,
            strings,
            styles,
            system: self.system,
            sheet: &self.name,
            prefix,
            reference: String::with_capacity(12),
            formula: String::new(),
            attributes: String::new(),
        };
        // The extras and the row formats are ordered as the cells are, so
        // one walk pairs them.
        let mut extras = self.extras.iter().peekable();
        let mut formats = self.layout.rows.iter().peekable();
        for row in self.rows.values() {
            while let Some((index, format)) = formats.next_if(|(index, _)| **index < row.index) {
                part.row_start(*index, Some(format), true)?;
            }
            let format = formats
                .next_if(|(index, _)| **index == row.index)
                .map(|(_, format)| format);
            part.row_start(row.index, format, false)?;
            for cell in &row.cells {
                let reference = cell.reference();
                while extras.next_if(|(at, _)| **at < reference).is_some() {}
                let extra = extras
                    .next_if(|(at, _)| **at == reference)
                    .map(|(_, extra)| extra);
                part.cell(cell, extra)?;
            }
            write!(part.writer, "</{prefix}row>")?;
        }
        for (index, format) in formats {
            part.row_start(*index, Some(format), true)?;
        }
        write!(part.writer, "</{prefix}sheetData>")?;
        Ok(())
    }

    /// Build the sheet from the rows of its part.
    ///
    /// The parser holds rows and cells to ascending order, so each row is
    /// laid out as it arrives, its cells in one vector of the exact length,
    /// and the parser's own buffer handed back for the next row. A parse that
    /// keeps the frame hands the sheet what the part states outside the
    /// cells once the rows are read.
    ///
    /// # Errors
    ///
    /// Returns a refusal naming the sheet and the cell.
    pub(crate) fn from_rows<R: std::io::BufRead>(
        name: SmolStr,
        state: SheetState,
        system: DateSystem,
        mut rows: SheetRows<R>,
        strings: &SharedStrings,
        styles: &StyleSheet,
        origin: u64,
    ) -> Result<Self> {
        let mut sheet = Self::held(name, state, system);
        sheet.origin = Some(origin);
        let mut formulas = Formulas::default();
        while let Some(row) = rows.next() {
            let row = row?;
            if let Some(format) = row.format {
                sheet.layout.rows.insert(row.index, format);
            }
            if row.cells.is_empty() {
                sheet.layout.rows.entry(row.index).or_default();
                continue;
            }
            let mut cells = Vec::with_capacity(row.cells.len());
            for raw in &row.cells {
                let reference = CellRef::new(row.index, raw.column);
                let (mut cell, serial) = sheet.read_cell(reference, raw, strings, styles)?;
                cell.set_formula(formulas.read(reference, raw));
                if let Some(extra) = CellExtra::of(raw, &cell, strings, serial) {
                    sheet.extras.insert(reference, extra);
                }
                cells.push(cell);
                sheet.extent.add(raw.column);
            }
            sheet.rows.insert(
                row.index,
                Row {
                    index: row.index,
                    cells,
                },
            );
            rows.recycle(row.cells);
        }
        formulas.resolve(&mut sheet)?;
        if let Some((frame, mut layout)) = rows.into_frame() {
            layout.rows = std::mem::take(&mut sheet.layout.rows);
            sheet.layout = layout;
            sheet.frame = Some(Box::new(frame));
        }
        Ok(sheet)
    }

    /// The cell `raw` states at `reference`.
    fn read_cell(
        &self,
        reference: CellRef,
        raw: &RawCell,
        strings: &SharedStrings,
        styles: &StyleSheet,
    ) -> Result<(Cell, Option<u64>)> {
        // A style past the part's cell formats names nothing to display a
        // cell with: the cell takes the default one, as it reads.
        let style = if usize::from(raw.style.as_u16()) < styles.len() {
            raw.style
        } else {
            StyleId::DEFAULT
        };
        let format = styles.number_format(style);
        // A shared string's content is its index, resolved to the table's
        // text; nothing is copied either way.
        let content: &str = if raw.kind == CellKind::SharedString && raw.has_content {
            let index: usize = raw
                .content
                .trim()
                .parse()
                .map_err(|_| Error::InvalidRecord {
                    path: format_smolstr!("{}!{reference}", self.name),
                    reason: format_smolstr!(
                        "expected a shared string index, got {:?}",
                        raw.content
                    ),
                })?;
            strings
                .get(index)
                .ok_or_else(|| Error::InvalidRecord {
                    path: format_smolstr!("{}!{reference}", self.name),
                    reason: format_smolstr!(
                        "expected a shared string index below {}, got {index}",
                        strings.len()
                    ),
                })?
                .as_str()
        } else {
            raw.content.as_str()
        };
        let located = |error: Error| Error::InvalidRecord {
            path: format_smolstr!("{}!{reference}", self.name),
            reason: super::cell::wire_reason(&error),
        };
        let (cell, serial) = if raw.kind == CellKind::Error {
            let literal = content.trim();
            if literal.is_empty() {
                // An error cell stating no error - `<c t="e"/>`, an empty
                // `<v>`, a formula with no cached result - holds nothing.
                (
                    Cell::new(reference, CellKind::Number, format, Scalar::Null),
                    None,
                )
            } else {
                // The literal rides the value slot, which is where an error
                // no variant names keeps it to be written back.
                (
                    Cell::new(reference, raw.kind, format, Scalar::from(literal))
                        .with_error(ExcelError::from_text(literal)),
                    None,
                )
            }
        } else {
            let (format, value, serial) = if raw.has_content || raw.kind.is_text() {
                match super::cell::wire_scalar(raw.kind, format, self.system, content)
                    .map_err(&located)?
                {
                    super::cell::WireScalar::Decoded { scalar, serial } => (format, scalar, serial),
                    // A number no day spells under a date format - Excel
                    // shows it as hashes - is the number it is.
                    super::cell::WireScalar::TemporalInvalid { serial, .. } => {
                        (NumberFormat::General, Scalar::from(serial), None)
                    }
                }
            } else {
                (format, Scalar::Null, None)
            };
            (Cell::new(reference, raw.kind, format, value), serial)
        };
        let cell = cell.with_style(style);
        let serial = serial.and_then(|raw| CellExtra::exceptional_serial(&cell, raw, self.system));
        Ok((cell, serial))
    }
}

/// The formulas of one part as it is read: each normal formula interned
/// by shape, each shared group's master shape by its `si`, and the
/// dependents read before their master, resolved once the part is read.
#[derive(Default)]
struct Formulas {
    interned: Interner,
    shared: HashMap<u32, Formula>,
    pending: Vec<(CellRef, u32)>,
}

impl Formulas {
    /// The formula `raw` states at `reference`, `None` for none or for a
    /// dependent whose master is not read yet.
    fn read(&mut self, reference: CellRef, raw: &RawCell) -> Option<Formula> {
        let formula = raw.formula.as_ref()?;
        let group = Some(&formula.attributes)
            .filter(|attributes| attributes.kind == FormulaKind::Shared)
            .and_then(|attributes| attributes.shared_index);
        match group {
            // A dependent states no text: it holds its master's shape.
            Some(group) if formula.text.is_empty() => {
                if let Some(master) = self.shared.get(&group) {
                    return Some(master.clone());
                }
                self.pending.push((reference, group));
                None
            }
            Some(group) => {
                let master = self
                    .interned
                    .intern(Formula::from_file(&formula.text, reference));
                self.shared.insert(group, master.clone());
                Some(master)
            }
            None if formula.text.is_empty() => None,
            None => Some(
                self.interned
                    .intern(Formula::from_file(&formula.text, reference)),
            ),
        }
    }

    /// Give each dependent read before its master the master's shape.
    ///
    /// # Errors
    ///
    /// Returns a refusal naming a dependent whose group no master states.
    fn resolve(self, sheet: &mut Sheet) -> Result<()> {
        for (reference, group) in self.pending {
            let Some(master) = self.shared.get(&group) else {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{}!{reference}", sheet.name),
                    reason: format_smolstr!(
                        "expected the master of shared formula {group} in the part, got a \
                         dependent naming a group no cell states the text of"
                    ),
                });
            };
            if let Some(cell) = sheet.rows.get_mut(&reference.row()).and_then(|row| {
                let at = row.position(reference.column()).ok()?;
                row.cells.get_mut(at)
            }) {
                cell.set_formula(Some(master.clone()));
            }
        }
        Ok(())
    }
}

/// A cell's value under the column `field` declares.
///
/// The value crosses the field's own contract; where that refuses a value
/// the cell's text spelling is tried through the text door every document
/// leaf crosses, so `7` written as a number lands in an `int64` column and
/// `2024-01-02` written as text in a `date32` one. What both refuse is null
/// under `safe` on a nullable column, else the refusal.
fn cell_under(cell: &Cell, field: &Field, safe: bool) -> Result<Scalar> {
    if cell.error().is_some() {
        // The error as the file spelled it, an unrecognized one included.
        let refused = Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!(
                "expected a {} value, got the error {}",
                field.dtype(),
                cell.error_text()
            ),
        };
        return if safe && field.is_nullable() {
            Ok(Scalar::Null)
        } else {
            Err(refused)
        };
    }
    let value = cell.value();
    if value.is_null() {
        return field.scalar(Scalar::Null);
    }
    let direct = if field.dtype().string_parameters().is_some() {
        field.scalar(Scalar::from(cell_text(value).into_owned()))
    } else {
        field.scalar(value.clone())
    };
    let read = direct
        .or_else(|_| crate::text::prepare_text(Scalar::from(cell_text(value).into_owned()), field));
    match read {
        Ok(value) => Ok(value),
        Err(_) if safe && field.is_nullable() => Ok(Scalar::Null),
        Err(error) => Err(error),
    }
}

/// A direction Ctrl and an arrow move in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Toward row 1.
    Up,
    /// Toward the last row.
    Down,
    /// Toward column A.
    Left,
    /// Toward the last column.
    Right,
}

/// Name the sheet and cell an error is about.
fn located(sheet: &str, reference: CellRef, error: Error) -> Error {
    match error {
        Error::InvalidRecord { reason, .. } => Error::InvalidRecord {
            path: format_smolstr!("{sheet}!{reference}"),
            reason,
        },
        other => other,
    }
}

/// The record root and the columns `serie` lays out: a record column's own
/// children, or any other column as the one child of a record named as it is.
fn record_columns(serie: &Serie) -> Result<(Field, Vec<Serie>)> {
    let field = serie.require_field()?;
    if let Some(record) = serie.as_struct() {
        return Ok((
            field.clone().with_nullable(false),
            record.children().to_vec(),
        ));
    }
    let root = Field::new(
        DEFAULT_ROOT_NAME,
        DataType::from(StructType::from_fields([field.clone()])?),
        false,
    );
    Ok((root, vec![serie.clone()]))
}

/// The record root of a sheet with no cell: no columns.
fn empty_root() -> Result<Field> {
    Ok(Field::new(
        DEFAULT_ROOT_NAME,
        DataType::from(StructType::from_fields(std::iter::empty::<Field>())?),
        false,
    ))
}

/// The worksheet part being written: the sink, the shared strings text is
/// written against, the styles part beside it, and the buffers every cell
/// reuses.
struct PartWriter<'a, 'w, 's, W: Write> {
    writer: &'a mut W,
    strings: &'a mut SharedStringsWriter<'w>,
    styles: &'a mut Splice<'s>,
    system: DateSystem,
    /// The sheet a refusal names.
    sheet: &'a str,
    /// The prefix every element is written under, colon included.
    prefix: &'a str,
    /// The cell's A1 reference, rendered once per cell into one buffer.
    reference: String,
    /// The cell's formula, rendered once per cell into one buffer.
    formula: String,
    /// A row's or a formula's attributes, rendered into one buffer.
    attributes: String,
}

impl<W: Write> PartWriter<'_, '_, '_, W> {
    /// Write the start tag of the row at zero-based `index` with what
    /// `format` states; a row holding no cell is closed at once.
    fn row_start(&mut self, index: u32, format: Option<&RowFormat>, empty: bool) -> Result<()> {
        self.attributes.clear();
        if let Some(format) = format {
            format.write(&mut self.attributes);
        }
        write!(
            self.writer,
            "<{}row r=\"{}\"{}{}",
            self.prefix,
            index + 1,
            self.attributes,
            if empty { "/>" } else { ">" }
        )?;
        Ok(())
    }

    /// Write one `<c>` for `cell`, with what the sheet holds beside it.
    ///
    /// Text is written against the shared strings, an inline string as it
    /// was read; a cell carrying a formula writes it first, with what its
    /// `<f>` stated beside the text, and types its cached text `str`, never
    /// `s`; a temporal writes its serial under its format's style; a
    /// boolean `b`; an error `e`, as it was read.
    fn cell(&mut self, cell: &Cell, extra: Option<&CellExtra>) -> Result<()> {
        use std::fmt::Write as _;

        let prefix = self.prefix;
        self.reference.clear();
        cell.reference().write_a1(&mut self.reference);
        // A formula whose text is empty is none unless its `<f>` states an
        // array or a data table: Excel repairs a part stating `<f></f>`.
        self.formula.clear();
        if let Some(formula) = cell.formula() {
            let _ = write!(self.formula, "{}", formula.at(cell.reference()));
        }
        let stated = extra.and_then(|extra| extra.formula.as_deref());
        // An array's `<f>` states the formula it anchors: an anchor holding
        // none since - a value put over it - is no array, nor the dynamic
        // array its `cm` names.
        let lost_array = cell.formula().is_none()
            && stated.is_some_and(|attributes| attributes.kind == FormulaKind::Array);
        let attributes = stated.filter(|_| !lost_array);
        let volatile = cell.formula().is_some_and(Formula::is_volatile);
        let has_formula = !self.formula.is_empty()
            || attributes.is_some_and(|attributes| {
                matches!(attributes.kind, FormulaKind::Array | FormulaKind::DataTable)
            });
        let refused = |reason: &'static str| Error::InvalidRecord {
            path: format_smolstr!("{}!{}", self.sheet, self.reference),
            reason: SmolStr::new_static(reason),
        };
        let error = match (cell.error(), cell.kind()) {
            (None, CellKind::Error) => {
                return Err(refused(
                    "expected the error an error cell holds, got a cell of kind e holding none",
                ));
            }
            (Some(_), _) => Some(cell.written_error().ok_or_else(|| {
                refused("expected the literal an unrecognized error was read with, got none")
            })?),
            (None, _) => None,
        };
        let own = cell.style();
        let table = self.styles.held();
        // The default names the entry a cell stating no `s` displays with,
        // which every table has whether or not its part lists one.
        if own != StyleId::DEFAULT && usize::from(own.as_u16()) >= table.len() {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{}!{}", self.sheet, self.reference),
                reason: format_smolstr!(
                    "expected a style the workbook's {} cell formats hold, got index {}",
                    table.len(),
                    own.as_u16()
                ),
            });
        }
        let style = self.styles.written(own, cell.format())?;
        let inline = cell.kind() == CellKind::InlineString && !has_formula;
        let writer = &mut *self.writer;
        write!(writer, "<{prefix}c r=\"{}\"", self.reference)?;
        if style != StyleId::DEFAULT {
            write!(writer, " s=\"{}\"", style.as_u16())?;
        }
        let kind = match (error, cell.kind()) {
            (Some(_), _) => Some("e"),
            (None, CellKind::Boolean) => Some("b"),
            (None, _) if inline => Some("inlineStr"),
            (None, kind) if kind.is_text() => Some(if has_formula { "str" } else { "s" }),
            (None, _) => None,
        };
        if let Some(kind) = kind {
            write!(writer, " t=\"{kind}\"")?;
        }
        if let Some(extra) = extra {
            // A `cm` names the dynamic array a formula anchors, which only a
            // formula written with its array attributes states.
            let anchors = !lost_array
                && (!has_formula
                    || attributes.is_some_and(|attributes| attributes.kind == FormulaKind::Array));
            if let Some(index) = extra.cell_metadata.filter(|_| anchors) {
                write!(writer, " cm=\"{index}\"")?;
            }
            if let Some(index) = extra.value_metadata {
                write!(writer, " vm=\"{index}\"")?;
            }
            if extra.phonetic {
                write!(writer, " ph=\"1\"")?;
            }
        }
        write!(writer, ">")?;
        if has_formula {
            self.attributes.clear();
            write_formula_attributes(&mut self.attributes, attributes, volatile);
            write!(writer, "<{prefix}f{}", self.attributes)?;
            if self.formula.is_empty() {
                write!(writer, "/>")?;
            } else {
                write!(writer, ">")?;
                crate::xml::write_element_text(writer, &self.formula)?;
                write!(writer, "</{prefix}f>")?;
            }
        }
        let value = cell.value();
        if let Some(text) = error {
            write!(writer, "<{prefix}v>")?;
            crate::xml::write_element_text(writer, text)?;
            write!(writer, "</{prefix}v></{prefix}c>")?;
            return Ok(());
        }
        match cell.kind() {
            CellKind::Boolean => {
                write!(
                    writer,
                    "<{prefix}v>{}</{prefix}v>",
                    u8::from(value.as_bool().unwrap_or(false))
                )?;
            }
            CellKind::Number | CellKind::Date => {
                if !value.is_null() {
                    write!(writer, "<{prefix}v>")?;
                    let serial = match extra.and_then(CellExtra::serial) {
                        Some(serial) => Some((serial, cell.format())),
                        None => self.system.serial_of(value)?,
                    };
                    match serial {
                        Some((serial, _)) => {
                            write!(writer, "{}", super::cell::serial_text(serial))?;
                        }
                        None => super::cell::write_cell_text(writer, value)?,
                    }
                    write!(writer, "</{prefix}v>")?;
                }
            }
            _ if inline => {
                write!(writer, "<{prefix}is>")?;
                let text = cell.text();
                // The runs spell the text the cell was read with; a cell
                // holding other text since writes its own.
                match extra
                    .and_then(|extra| extra.inline_runs.as_deref())
                    .filter(|rich| rich.text.as_str() == text)
                {
                    Some(rich) => writer.write_all(&rich.runs)?,
                    None => {
                        write!(writer, "<{prefix}t")?;
                        if text.starts_with(char::is_whitespace)
                            || text.ends_with(char::is_whitespace)
                        {
                            write!(writer, " xml:space=\"preserve\"")?;
                        }
                        write!(writer, ">")?;
                        crate::xml::write_element_text(
                            writer,
                            &super::shared_strings::encode(&text),
                        )?;
                        write!(writer, "</{prefix}t>")?;
                    }
                }
                write!(writer, "</{prefix}is>")?;
            }
            CellKind::SharedString | CellKind::InlineString | CellKind::FormulaString => {
                let text = cell.text();
                if has_formula {
                    write!(writer, "<{prefix}v>")?;
                    crate::xml::write_element_text(writer, &super::shared_strings::encode(&text))?;
                    write!(writer, "</{prefix}v>")?;
                } else {
                    let kept = extra.and_then(|extra| extra.shared_string);
                    let index = self.strings.index(&text, kept);
                    write!(writer, "<{prefix}v>{index}</{prefix}v>")?;
                }
            }
            CellKind::Error => {}
        }
        write!(writer, "</{prefix}c>")?;
        Ok(())
    }
}

/// Write what an `<f>` states beside its text, a space before each: the
/// `attributes` it was read with, `ca` also where the formula is volatile.
fn write_formula_attributes(
    target: &mut String,
    attributes: Option<&FormulaAttributes>,
    volatile: bool,
) {
    use std::fmt::Write as _;

    let Some(attributes) = attributes else {
        if volatile {
            target.push_str(" ca=\"1\"");
        }
        return;
    };
    match attributes.kind {
        FormulaKind::Array => target.push_str(" t=\"array\""),
        FormulaKind::DataTable => target.push_str(" t=\"dataTable\""),
        FormulaKind::Normal | FormulaKind::Shared => {}
    }
    let flag = |target: &mut String, stated: bool, name: &str| {
        if stated {
            let _ = write!(target, " {name}=\"1\"");
        }
    };
    flag(target, attributes.array_always_calculate, "aca");
    if let Some(reference) = &attributes.reference {
        let _ = write!(
            target,
            " ref=\"{}\"",
            super::package::escape_attribute(reference)
        );
    }
    flag(target, attributes.two_dimensional, "dt2D");
    flag(target, attributes.row_input, "dtr");
    flag(target, attributes.first_deleted, "del1");
    flag(target, attributes.second_deleted, "del2");
    for (input, name) in [
        (&attributes.first_input, "r1"),
        (&attributes.second_input, "r2"),
    ] {
        if let Some(input) = input {
            let _ = write!(
                target,
                " {name}=\"{}\"",
                super::package::escape_attribute(input)
            );
        }
    }
    flag(target, attributes.always_calculate || volatile, "ca");
    flag(target, attributes.assigns, "bx");
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/excel/sheet.rs` pins and a caller cannot reach:
    //! carried-child replacement and the distinction between no frame
    //! and a frame with no children.

    use super::Sheet;

    /// Whether calculation has activated this sheet's lazy change owner.
    #[must_use]
    pub fn changes_active(sheet: &Sheet) -> bool {
        sheet.changes().is_some()
    }

    /// Activate the private owner and return its replacement-detection identity.
    pub fn track_changes(sheet: &mut Sheet) -> u64 {
        sheet.track_changes().generation()
    }

    /// The semantic pending facts in coordinate order, for the mirrored tests.
    #[must_use]
    pub fn pending_changes(sheet: &Sheet) -> Option<(u64, bool, Vec<(super::CellRef, bool)>)> {
        let changes = sheet.changes()?;
        let mut points: Vec<_> = changes.points().collect();
        points.sort_unstable();
        Some((changes.generation(), changes.structural(), points))
    }

    /// A successful calculation acknowledges facts without changing identity.
    pub fn acknowledge_changes(sheet: &mut Sheet) {
        sheet.acknowledge_changes();
    }

    /// A guard's opaque journal checkpoint; cell payload is owned elsewhere.
    pub struct ChangeCheckpoint(Option<super::ChangeMark>);

    /// Capture a bounded checkpoint without cloning pending coordinates.
    pub fn change_mark(sheet: &Sheet) -> ChangeCheckpoint {
        ChangeCheckpoint(sheet.change_mark())
    }

    /// Restore journal facts after the corresponding payload inverse.
    ///
    /// # Errors
    ///
    /// Returns Conflict for an active mark from another instance/epoch.
    pub fn restore_change_mark(sheet: &mut Sheet, mark: ChangeCheckpoint) -> crate::Result<()> {
        sheet.restore_change_mark(mark.0)
    }

    /// Exercise the existing cache-publication owner without a graph mutation.
    ///
    /// # Errors
    ///
    /// Returns a missing-cell or result-conversion refusal.
    pub fn replace_cache(sheet: &mut Sheet, at: super::CellRef, value: f64) -> crate::Result<()> {
        let cell = sheet
            .cell(at)
            .ok_or_else(|| crate::Error::absent("cell", at))?;
        let replacement = sheet.plan_calculated(
            cell,
            crate::excel::formula::value::Operand::Number(value),
            cell.format(),
        )?;
        if let Some((cell, bits)) = replacement {
            sheet.replace_calculated(cell, bits);
        }
        Ok(())
    }

    /// Visit or rewrite only the cell, row and column style IDs owned by
    /// main/Strict worksheet elements, retaining every other XML byte.
    ///
    /// # Errors
    ///
    /// Returns a refusal located at `part` for invalid XML or style IDs.
    pub fn rewrite_style_ids(
        bytes: &[u8],
        part: &str,
        change: impl FnMut(super::StyleId) -> Option<super::StyleId>,
    ) -> crate::Result<Option<Vec<u8>>> {
        Sheet::rewrite_style_ids(bytes, part, change)
    }

    /// Whether the sheet retains a worksheet frame, even an empty one.
    #[must_use]
    pub fn has_frame(sheet: &Sheet) -> bool {
        sheet.frame().is_some()
    }

    /// Replace only the carried children with those of `source`; `None`
    /// supplies no children. The source's root and cells are not copied.
    pub fn set_frame_items_from(sheet: &mut Sheet, source: Option<&Sheet>) {
        let items = source
            .and_then(Sheet::frame)
            .map_or_else(Vec::new, |frame| frame.items.clone());
        sheet.set_frame_items(items);
    }
}

impl Sheet {
    fn rows_window(&self, span: CellRange, levels: u32) -> Result<(RowsWindow, usize)> {
        let header = RowsWindow::header_range(span, levels)?;
        let mut labels = Vec::new();
        let mut columns = std::collections::BTreeSet::new();
        for (_, row) in self.rows.range(..=header.end().row()) {
            for cell in row.cells() {
                let text = if !cell.has_content() || cell.kind() == CellKind::Error {
                    SmolStr::new_static("")
                } else {
                    SmolStr::new(cell.text())
                };
                labels.push(RowLabel {
                    at: cell.reference(),
                    text,
                });
            }
        }
        let mut body_len = 0;
        for row in self
            .record_rows()
            .filter(|row| row.index() > header.end().row())
        {
            columns.extend(row.cells().filter(|cell| !cell.is_null()).map(Cell::column));
            body_len += 1;
        }
        // Sheet stores its own merges. One metadata owner validates intersection;
        // no workbook/package pass is needed for held cells.
        let merges = self
            .merges()
            .filter(|merge| merge.intersects(header))
            .collect();
        Ok((
            RowsWindow {
                sheet: self.name.clone(),
                range: span,
                levels,
                columns: columns.into_iter().collect(),
                labels,
                merges,
            },
            body_len,
        ))
    }

    fn rows_nested(
        self,
        field: Option<&Field>,
        levels: u32,
        options: ArrowCastOptions,
    ) -> Result<Serie> {
        let Some(span) = self.dimension() else {
            // An explicit Rows request has no physical header on an empty Sheet.
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.header"),
                reason: SmolStr::new_static("expected selected header cells, got an empty sheet"),
            });
        };
        let span = CellRange::new(
            span.start(),
            CellRef::new(
                self.record_end_row().unwrap_or(span.end().row()),
                span.end().column(),
            ),
        );
        let span = RowsWindow::implicit_range(span, levels, self.merges())?;
        self.rows_nested_span(field, levels, span, options)
    }

    fn rows_nested_span(
        self,
        field: Option<&Field>,
        levels: u32,
        span: CellRange,
        options: ArrowCastOptions,
    ) -> Result<Serie> {
        let (window, body_len) = self.rows_window(span, levels)?;
        let body_start = RowsWindow::header_range(span, levels)?.end().row() + 1;
        let layout = match field {
            Some(field) => RowsLayout::declared(window, field)?,
            None => {
                let mut first = BTreeMap::<u32, (DataType, u64)>::new();
                for row in self.record_rows().filter(|row| row.index() >= body_start) {
                    for cell in row.cells().filter(|cell| !cell.is_null()) {
                        let dtype = cell
                            .value()
                            .dtype()
                            .map_err(|error| located(&self.name, cell.reference(), error))?;
                        match first.entry(cell.column()) {
                            std::collections::btree_map::Entry::Vacant(slot) => {
                                slot.insert((dtype, 1));
                            }
                            std::collections::btree_map::Entry::Occupied(mut slot) => {
                                let (known, seen) = slot.get_mut();
                                if *known != dtype {
                                    return Err(Error::InvalidRecord {
                                        path: format_smolstr!("{}!{}", self.name, cell.reference()),
                                        reason: format_smolstr!(
                                            "expected {known} like the column's first value, got {dtype}; declare a field to read the column as one datatype"
                                        ),
                                    });
                                }
                                *seen += 1;
                            }
                        }
                    }
                }
                let stats = first
                    .iter()
                    .map(|(column, (dtype, seen))| {
                        (*column, (dtype.clone(), *seen != body_len as u64))
                    })
                    .collect::<BTreeMap<_, _>>();
                RowsLayout::inferred(window, DEFAULT_ROOT_NAME, &stats)?
            }
        };
        let root = layout.root().clone();
        let safe = options.is_safe();
        let mut values = Vec::with_capacity(layout.leaves().len());
        let mut scalars = Vec::with_capacity(body_len);
        for row in self.record_rows().filter(|row| row.index() >= body_start) {
            values.clear();
            for leaf in layout.leaves() {
                let cell = leaf.column.and_then(|column| row.cell(column));
                let value = match cell {
                    Some(cell) if !cell.is_null() => cell_under(cell, &leaf.field, safe)
                        .map_err(|error| located(&self.name, cell.reference(), error))?,
                    _ if !leaf.field.is_nullable() => {
                        return Err(Error::InvalidRecord {
                            path: format_smolstr!(
                                "{}!{}",
                                self.name,
                                CellRef::new(row.index(), leaf.column.unwrap_or(0))
                            ),
                            reason: format_smolstr!(
                                "expected a value for the required column {}, got no cell",
                                leaf.field.name()
                            ),
                        });
                    }
                    _ => Scalar::Null,
                };
                values.push(value);
            }
            scalars.push(layout.assemble(&mut values, &self.name, row.index())?);
        }
        Serie::from_scalars(root, scalars)
    }
}
