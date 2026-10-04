//! The workbook: the one model of a package's parts and sheets.
//!
//! A [`Workbook`] opened over a handle mounts the archive and reads the
//! package documents once - `_rels/.rels` to the office document, the
//! workbook part for its sheets and date system, its relationships for each
//! sheet's part and for the shared strings and styles. Sheets are then read
//! on demand: the first access to a sheet parses its part into a
//! [`Sheet`] held until the workbook is dropped; the shared strings and the
//! styles are read once, on the first sheet that needs them. A workbook
//! built with [`Workbook::new`] starts from a template of the four package
//! documents and holds only the sheets it is given.
//!
//! Writing produces the package again, and it writes exactly what changed:
//! a sheet is written from its cells when it is new or changed since it was
//! read or last saved; every other member - an untouched sheet, parsed or
//! not, its drawings, the theme, the document properties - is copied as it
//! is stored, its compressed bytes never inflated. The styles and the
//! shared strings are written again only when a save added to them, and
//! every index a cell states keeps naming what it named: the styles are
//! appended to, their counts patched and every other byte kept; the shared
//! strings are written again from their text - a rich or phonetic item
//! kept at its index as its plain text, `count` counting the references of
//! the sheets the save wrote. A workbook with no visible worksheet is
//! refused, as Excel refuses it.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use smallvec::SmallVec;
use smol_str::{SmolStr, format_smolstr};

use crate::holder::{Buffer, Holder};
use crate::zip::ZipArchive;
use crate::{Codec, Error, IOBase, Result, Scalar};

use super::cell::{Cell, CellKind, CellRange, CellRef, DateSystem, MAX_COLUMNS, MAX_ROWS};
use super::edit::{
    Clear, MAX_EDITED_CELLS, PartRestore, Paste, Restore, RestoreSheet, SortKey, Step,
};
use super::entry::Entry;
use super::format::{FormatCode, Rendered};
use super::formula::Formula;
use super::formula::value::NameId;
use super::names::DefinedName;
use super::package::{self, Insertion, NamespaceFamily, RelationshipKind, Relationships};
use super::parser::SheetRows;
use super::pivot::{AxisField, ItemOrder, PivotFieldInfo, PivotIdentity, PivotSource, PivotSpec, PivotTable, ValueField};
use super::formula::aggregate::Aggregate;
use super::shared_strings::{SharedStrings, SharedStringsWriter};
use super::sheet::{ChangeMark, Sheet, SheetState, validate_sheet_name};
use super::shift::{self, Axis, Band, Host, Rewriter, Shift, adjust_range, range_text};
use super::style::{CellStyle, Outline, StyleId, StylePatch};
use super::styles::{Checkpoint as StyleCheckpoint, NumberFormat, Splice, StyleSheet};
use super::table::Table;
use super::theme::Theme;

/// What a `<sheet>` entry names, by the relationship its `r:id` resolves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SheetKind {
    /// A grid of cells: the one kind with rows to read.
    Worksheet,
    /// A chart on a tab of its own; it holds no cells.
    Chartsheet,
    /// A dialog on a tab of its own; it holds no cells.
    Dialogsheet,
}

impl SheetKind {
    /// The OPC content type for a live tab of this resolved kind.
    const fn content_type(self) -> &'static str {
        match self {
            Self::Worksheet => package::WORKSHEET_CONTENT_TYPE,
            Self::Chartsheet => {
                "application/vnd.openxmlformats-officedocument.spreadsheetml.chartsheet+xml"
            }
            Self::Dialogsheet => {
                "application/vnd.openxmlformats-officedocument.spreadsheetml.dialogsheet+xml"
            }
        }
    }

    /// The kind as the refusals name it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Worksheet => "worksheet",
            Self::Chartsheet => "chartsheet",
            Self::Dialogsheet => "dialogsheet",
        }
    }
}

/// A sheet's identity in one workbook: stable while the workbook lives,
/// whatever the sheet is renamed to or wherever its tab moves, and never
/// given to another sheet - a sheet put in place of another takes a key of
/// its own.
///
/// ```
/// use yggdryl::excel::Workbook;
///
/// let mut workbook = Workbook::new();
/// workbook.add_sheet("Trades")?;
/// let key = workbook.sheet_key("trades").expect("a key");
/// workbook.rename_sheet("Trades", "Fills")?;
/// assert_eq!(workbook.sheet_by_key(key), Some("Fills"));
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SheetKey(u32);

impl SheetKey {
    /// The key as a number.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

/// Why a typed workbook reference cannot yet supply a computed value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReferenceHeld {
    Shape(super::formula::shape::Held),
    NameAnchor,
    NameScope,
    NonWorksheet,
}

/// A context borrowed from one resolver's workbook. Named expressions keep
/// their definition identity without rendering or copying the shared arena.
#[derive(Clone, Copy)]
pub(crate) struct ReferenceHost<'w> {
    book: &'w Workbook,
    tab: usize,
    at: CellRef,
    name: Option<NameId>,
}

/// Geometry resolved for one formula's immutable workbook context. The arena
/// keeps this small value rather than lending a workbook slice to the evaluator.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReferenceArea {
    first_tab: usize,
    end_tab: usize,
    range: CellRange,
}

impl ReferenceArea {
    /// Identity/dimension consumers accept exactly one worksheet tab.
    fn geometry(self) -> Option<CellRange> {
        (self.end_tab == self.first_tab + 1).then_some(self.range)
    }

    /// One selection owner for graph edges and scalar reference reads.
    /// ISREF geometry has no value area, so it creates no cell-value edge.
    /// Non-cm formulas use legacy implicit intersection, whether the file
    /// retained an explicit @ marker or Excel saved an unmarked reference.
    fn selected(
        self,
        usage: super::formula::parser::ReferenceUse,
        host: CellRef,
    ) -> std::result::Result<Option<Self>, super::cell::ExcelError> {
        use super::formula::parser::ReferenceUse;
        if matches!(usage, ReferenceUse::Range | ReferenceUse::SingleRange) {
            return Ok(Some(self));
        }
        if usage == ReferenceUse::Geometry {
            return Ok(None);
        }
        if self.end_tab != self.first_tab + 1 {
            return Ok(None);
        }
        let start = self.range.start();
        let end = self.range.end();
        if start == end {
            return Ok(Some(self));
        }
        let at = if start.row() == end.row() {
            if !(start.column()..=end.column()).contains(&host.column()) {
                return Err(super::cell::ExcelError::Value);
            }
            CellRef::new(start.row(), host.column())
        } else if start.column() == end.column() {
            if !(start.row()..=end.row()).contains(&host.row()) {
                return Err(super::cell::ExcelError::Value);
            }
            CellRef::new(host.row(), start.column())
        } else {
            if !self.range.contains(host) {
                return Err(super::cell::ExcelError::Value);
            }
            host
        };
        Ok(Some(Self {
            range: CellRange::new(at, at),
            ..self
        }))
    }

    /// Reborrow only against the workbook that resolved this descriptor. Its
    /// formula-scoped owner cannot mutate tabs or retain IDs across evaluations.
    fn view(self, book: &Workbook) -> ReferenceRange<'_> {
        ReferenceRange { book, area: self }
    }
}

/// One proven rectangle over a contiguous worksheet-only tab slice.
#[derive(Clone, Copy)]
pub(crate) struct ReferenceRange<'w> {
    book: &'w Workbook,
    area: ReferenceArea,
}

impl<'w> ReferenceRange<'w> {
    /// Copy geometry into the formula-scoped descriptor arena without a borrow.
    pub(crate) const fn descriptor(self) -> ReferenceArea {
        self.area
    }

    fn slots(self) -> &'w [Slot] {
        &self.book.slots[self.area.first_tab..self.area.end_tab]
    }

    /// Geometry shared by dependency intake and evaluator reads. Blank cells
    /// remain inside the rectangle even though the sparse cursor omits them.
    pub(crate) fn areas(self) -> impl Iterator<Item = (SheetKey, CellRange)> + 'w {
        self.slots()
            .iter()
            .map(move |slot| (slot.key, self.area.range))
    }

    /// A borrowed sheet view; scalar reads use cell and aggregates cells_in.
    #[cfg(feature = "internals")]
    pub(crate) fn sheets(self) -> impl Iterator<Item = (SheetKey, &'w Sheet, CellRange)> + 'w {
        self.slots().iter().map(move |slot| {
            (
                slot.key,
                slot.parsed
                    .get()
                    .expect("the resolver parsed each worksheet"),
                self.area.range,
            )
        })
    }

    /// Stored cells only, in tab then row/column order. No blank grid is built.
    #[cfg(feature = "internals")]
    pub(crate) fn cells(self) -> impl Iterator<Item = (SheetKey, &'w Cell)> + 'w {
        self.sheets()
            .flat_map(|(key, sheet, area)| sheet.cells_in(area).map(move |cell| (key, cell)))
    }

    /// A 3D logical size can exceed the per-sheet u64 count. Consumers that
    /// narrow it must check their own result representation.
    #[cfg(feature = "internals")]
    pub(crate) fn logical_len(self) -> u128 {
        u128::from(self.area.range.cell_count()) * (self.area.end_tab - self.area.first_tab) as u128
    }
}

#[derive(Clone, Copy)]
pub(crate) struct NameBinding<'w> {
    id: NameId,
    host: ReferenceHost<'w>,
}

impl<'w> NameBinding<'w> {
    pub(crate) fn definition(self) -> &'w DefinedName {
        &self.host.book.stated.names[self.id.0]
    }

    pub(crate) fn expression(
        self,
    ) -> std::result::Result<&'w super::formula::parser::Expr, super::formula::shape::Held> {
        self.definition().formula().expression()
    }

    #[cfg(feature = "internals")]
    pub(crate) fn host(self) -> ReferenceHost<'w> {
        self.host
    }
}

pub(crate) enum ResolvedReference<'w> {
    Range(ReferenceRange<'w>),
    Name(NameBinding<'w>),
    Error(super::cell::ExcelError),
    Held(ReferenceHeld),
}

/// Pass-local lookup over the workbook's existing names and tabs. The only
/// retained allocation is a sorted index of names, never names or expressions.
pub(crate) struct ReferenceResolver<'w> {
    book: &'w Workbook,
    names: Vec<usize>,
}

impl<'w> ReferenceResolver<'w> {
    /// Explicit recalculation intake; opening or reading records stays lazy.
    #[cfg(feature = "internals")]
    pub(crate) fn new(book: &'w Workbook) -> Result<Self> {
        book.parse_all()?;
        Ok(Self::indexed(book, Vec::new(), true))
    }

    fn indexed(book: &'w Workbook, mut names: Vec<usize>, refresh: bool) -> Self {
        if refresh {
            names.clear();
            names.extend(0..book.stated.names.len());
            names.sort_unstable_by(|&left, &right| {
                let left = &book.stated.names[left];
                let right = &book.stated.names[right];
                left.scope()
                    .cmp(&right.scope())
                    .then_with(|| Self::compare(left.name(), right.name()))
            });
        }
        Self { book, names }
    }

    /// Bind a scheduled cell once; scalar/range reads never repeat this lookup.
    #[cfg(feature = "internals")]
    pub(crate) fn host(&self, sheet: SheetKey, at: CellRef) -> Option<ReferenceHost<'w>> {
        let tab = self.book.slots.iter().position(|slot| slot.key == sheet)?;
        self.host_at(tab, at)
    }

    fn host_at(&self, tab: usize, at: CellRef) -> Option<ReferenceHost<'w>> {
        if at.row() >= MAX_ROWS || at.column() >= MAX_COLUMNS || tab >= self.book.slots.len() {
            return None;
        }
        Some(ReferenceHost {
            book: self.book,
            tab,
            at,
            name: None,
        })
    }

    fn compare(left: &str, right: &str) -> std::cmp::Ordering {
        left.bytes()
            .map(|byte| byte.to_ascii_lowercase())
            .cmp(right.bytes().map(|byte| byte.to_ascii_lowercase()))
    }

    fn named(&self, scope: Option<SheetKey>, name: &str) -> Result<Option<NameId>> {
        let compare = |index: usize| {
            let held = &self.book.stated.names[index];
            held.scope()
                .cmp(&scope)
                .then_with(|| Self::compare(held.name(), name))
        };
        let first = self.names.partition_point(|&index| compare(index).is_lt());
        let count = self.names[first..]
            .iter()
            .take_while(|&&index| compare(index).is_eq())
            .count();
        match count {
            0 => Ok(None),
            1 => Ok(Some(NameId(self.names[first]))),
            _ => Err(Error::InvalidRecord {
                path: format_smolstr!("{}#definedName[{name}]", self.book.workbook_part),
                reason: format_smolstr!(
                    "expected one defined name in scope {scope:?}, got {count}"
                ),
            }),
        }
    }

    fn binding(&self, id: NameId, host: ReferenceHost<'w>) -> ResolvedReference<'w> {
        let name = &self.book.stated.names[id.0];
        if name.scope().is_some_and(|key| !self.book.slots.iter().any(|slot| slot.key == key)) {
            return ResolvedReference::Error(super::cell::ExcelError::Ref);
        }
        ResolvedReference::Name(NameBinding {
            id,
            // Wire relative names are anchored to the consuming cell, not a
            // saved active selection. Their own scope selects only the name.
            host: ReferenceHost { name: Some(id), ..host },
        })
    }

    fn name_expression(&self, name: NameId) -> std::result::Result<&'w super::formula::parser::Expr, super::formula::shape::Held> {
        self.book.stated.names[name.0].formula().expression()
    }

    pub(crate) fn resolve(
        &self,
        host: ReferenceHost<'w>,
        reference: &super::formula::reference::Reference,
    ) -> Result<ResolvedReference<'w>> {
        use super::cell::ExcelError;
        use super::formula::reference::{SheetSpec, Target};
        use super::formula::shape::Held;

        if !std::ptr::eq(self.book, host.book) {
            return Ok(ResolvedReference::Error(ExcelError::Ref));
        }
        if matches!(reference.target, Target::Invalid) {
            return Ok(ResolvedReference::Error(ExcelError::Ref));
        }
        let (first, last, qualified) = match &reference.sheet {
            SheetSpec::Own => (host.tab, host.tab, false),
            SheetSpec::Named { name, .. } => match self.book.resolve(name) {
                Some(at) => (at, at, true),
                None => return Ok(ResolvedReference::Error(ExcelError::Ref)),
            },
            SheetSpec::Span { first, last, .. } => {
                let (Some(first), Some(last)) = (self.book.resolve(first), self.book.resolve(last))
                else {
                    return Ok(ResolvedReference::Error(ExcelError::Ref));
                };
                (first.min(last), first.max(last), true)
            }
            SheetSpec::External(_) => {
                return Ok(ResolvedReference::Held(ReferenceHeld::Shape(
                    Held::External,
                )));
            }
            SheetSpec::Invalid => return Ok(ResolvedReference::Error(ExcelError::Ref)),
        };
        let slots = &self.book.slots[first..=last];
        if slots.iter().any(|slot| slot.kind != SheetKind::Worksheet) {
            return Ok(ResolvedReference::Held(ReferenceHeld::NonWorksheet));
        }
        if let Target::Name(name) = &reference.target {
            if first != last || matches!(reference.sheet, SheetSpec::Span { .. }) {
                return Ok(ResolvedReference::Held(ReferenceHeld::NameScope));
            }
            // Bare tokens inside a defined expression bind globally, even
            // when that definition is local. Explicit sheet names keep scope.
            if (host.name.is_none() || qualified)
                && let Some(defined) = self.named(Some(slots[0].key), name)?
            {
                return Ok(self.binding(defined, host));
            }
            if let Some(defined) = self.named(None, name)? {
                return Ok(if qualified {
                    ResolvedReference::Held(ReferenceHeld::NameScope)
                } else {
                    self.binding(defined, host)
                });
            }
            return Ok(ResolvedReference::Error(ExcelError::Name));
        }
        if host.name.is_some() && matches!(reference.sheet, SheetSpec::Own) {
            return Ok(ResolvedReference::Held(ReferenceHeld::NameAnchor));
        }
        Ok(match reference.target.range(host.at) {
            Some(range) => ResolvedReference::Range(
                ReferenceArea {
                    first_tab: first,
                    end_tab: last + 1,
                    range,
                }
                .view(self.book),
            ),
            None => ResolvedReference::Error(ExcelError::Ref),
        })
    }
}

#[derive(Debug)]
enum DependencyFrame {
    Node { name: Option<NameId>, id: usize, usage: super::formula::parser::ReferenceUse },
    LeaveName { name: NameId, usage: super::formula::parser::ReferenceUse },
}

#[derive(Clone, Copy, Debug)]
enum ReadShape {
    Area(ReferenceArea),
    Union(super::formula::value::ReferenceId, super::formula::value::ReferenceId),
}

#[derive(Debug)]
struct ReadArea {
    shape: ReadShape,
    scalar: bool,
    range: bool,
    subtotal: [bool; 2],
}

impl ReadArea {
    fn area(area: ReferenceArea) -> Self {
        Self { shape: ReadShape::Area(area), scalar: false, range: false,
            subtotal: [false; 2] }
    }

    fn union(left: super::formula::value::ReferenceId,
        right: super::formula::value::ReferenceId) -> Self {
        Self { shape: ReadShape::Union(left, right), scalar: false, range: false,
            subtotal: [false; 2] }
    }
}

/// Same evaluator, held only while a selected dependency is unfinished.
/// The pool retains peak simultaneous formula/name scratch, never AST copies.
#[derive(Debug, Default)]
struct SuspendedCalculation {
    evaluator: super::formula::eval::Evaluator,
    areas: Vec<ReadArea>,
    random: Option<u64>,
}

impl SuspendedCalculation {
    fn clear(&mut self) {
        self.evaluator.clear();
        self.areas.clear();
        self.random = None;
    }
}

/// Lazy calculation state. No cell or expression arena is copied here. Scratch
/// retains peak node/precedent/changed-result capacities; graph edges remain
/// sparse rectangles rather than a materialized adjacency list.
#[derive(Debug, Default)]
struct Calculation {
    graph: super::formula::graph::Graph,
    schedule: super::formula::graph::Schedule,
    evaluator: super::formula::eval::Evaluator,
    pass: u64,
    last_status: Option<super::formula::Recalculation>,
    valid: bool,
    documents: u64,
    identities: Vec<(SheetKey, u64)>,
    tabs: HashMap<SheetKey, usize>,
    names: Vec<usize>,
    stack: Vec<DependencyFrame>,
    active_names: HashSet<NameId>,
    completed_names: HashSet<(NameId, super::formula::parser::ReferenceUse)>,
    #[cfg(feature = "internals")]
    dependency_nodes: usize,
    precedents: Vec<(SheetKey, CellRange)>,
    areas: Vec<ReadArea>,
    area_stack: Vec<super::formula::value::ReferenceId>,
    area_leaves: Vec<super::formula::value::ReferenceId>,
    suspended: HashMap<usize, SuspendedCalculation>,
    spare: Vec<SuspendedCalculation>,
    outcomes: Vec<Option<super::formula::value::Outcome>>,
    receipts: Vec<bool>,
    replacements: Vec<(usize, Cell, Option<u64>)>,
}

impl Calculation {
    fn rebuild_required(&self, book: &Workbook) -> bool {
        !self.valid
            || self.documents != book.documents
            || self.identities.len() != book.slots.len()
            || book
                .slots
                .iter()
                .zip(&self.identities)
                .any(|(slot, &(key, generation))| {
                    slot.key != key
                        || (slot.kind == SheetKind::Worksheet
                            && slot
                                .parsed
                                .get()
                                .and_then(Sheet::changes)
                                .is_none_or(|changes| {
                                    changes.generation() != generation || changes.structural()
                                }))
                })
    }

    fn register(
        &mut self,
        resolver: &ReferenceResolver<'_>,
        tab: usize,
        cell: &Cell,
    ) -> Result<()> {
        use super::formula::parser::{EvaluationPolicy, Node, ReferenceUse};
        self.precedents.clear();
        self.stack.clear();
        self.active_names.clear();
        self.completed_names.clear();
        #[cfg(feature = "internals")]
        { self.dependency_nodes = 0; }
        let mut volatile = false;
        let mut selective = false;
        let slot = &resolver.book.slots[tab];
        let host = resolver
            .host_at(tab, cell.reference())
            .expect("a formula has a proven host");
        let sheet = slot.parsed.get().expect("calculation parsed worksheets");
        if sheet.scalar_formula_at(cell.reference()) {
            if let Ok(expression) = cell
                .formula()
                .expect("register receives formulas")
                .expression()
            {
                self.stack.push(DependencyFrame::Node { name: None, id: expression.root, usage: ReferenceUse::Scalar });
                while let Some(frame) = self.stack.pop() {
                    let (name, id, usage) = match frame {
                        DependencyFrame::Node { name, id, usage } => (name, id, usage),
                        DependencyFrame::LeaveName { name, usage } => {
                            self.active_names.remove(&name);
                            self.completed_names.insert((name, usage));
                            continue;
                        }
                    };
                    let arena = match name {
                        Some(name) => resolver.name_expression(name).expect("entry proved the immutable name arena"),
                        None => expression,
                    };
                    #[cfg(feature = "internals")]
                    { self.dependency_nodes += 1; }
                    let node = &arena.nodes[id];
                    if let Node::Call {
                        function: Some(function),
                        ..
                    } = node
                    {
                        volatile |= function.info().volatile;
                    }
                    match node.evaluation_children() {
                        EvaluationPolicy::Leaf => {
                            if let Node::Reference(reference) = node {
                                match resolver.resolve(ReferenceHost { name, ..host }, reference)? {
                                    ResolvedReference::Range(range) => {
                                        if let Ok(Some(area)) = range.descriptor().selected(usage, host.at) {
                                            self.precedents.extend(area.view(resolver.book).areas());
                                        }
                                    }
                                    ResolvedReference::Name(binding) => {
                                        if !self.completed_names.contains(&(binding.id, usage))
                                            && let Ok(named) = binding.expression()
                                            && self.active_names.insert(binding.id)
                                        {
                                            self.stack.push(DependencyFrame::LeaveName { name: binding.id, usage });
                                            self.stack.push(DependencyFrame::Node { name: Some(binding.id), id: named.root, usage });
                                        }
                                    }
                                    ResolvedReference::Error(_) | ResolvedReference::Held(_) => {}
                                }
                            }
                        }
                        EvaluationPolicy::Strict(children) => {
                            selective |= matches!(&children, super::formula::parser::Children::MixedCall { dynamic: true, .. }
                                | super::formula::parser::Children::GeometryPair(_, _));
                            children.visit_references(usage, |child, usage| {
                                self.stack.push(DependencyFrame::Node { name, id: child, usage })
                            });
                        }
                        EvaluationPolicy::Select(selection) => {
                            selective = true;
                            selection.visit_inputs_reverse(|input, usage| {
                                self.stack.push(DependencyFrame::Node { name, id: input, usage });
                            });
                        }
                        EvaluationPolicy::Held => {}
                    }
                }
            }
        }
        self.graph
            .set((slot.key, cell.reference()), &self.precedents, volatile, selective)
    }

    fn synchronize(&mut self, resolver: &ReferenceResolver<'_>, rebuild: bool) -> Result<()> {
        if rebuild {
            // This index is derived, so a failed rebuild invalidates it instead
            // of retaining an O(graph) rollback copy. Pass buffers stay owned.
            self.graph = super::formula::graph::Graph::default();
            self.tabs.clear();
            self.tabs.reserve(resolver.book.slots.len());
            for (tab, slot) in resolver.book.slots.iter().enumerate() {
                self.tabs.insert(slot.key, tab);
                if slot.kind != SheetKind::Worksheet {
                    continue;
                }
                for cell in slot.parsed.get().expect("parsed above").cells() {
                    if cell.formula().is_some() {
                        self.register(resolver, tab, cell)?;
                    }
                }
            }
        } else {
            for (tab, slot) in resolver.book.slots.iter().enumerate() {
                if slot.kind != SheetKind::Worksheet {
                    continue;
                }
                let sheet = slot.parsed.get().expect("parsed above");
                for (at, formula_changed) in sheet
                    .changes()
                    .expect("valid graph tracks its sheets")
                    .points()
                {
                    if formula_changed {
                        if let Some(cell) = sheet.cell(at).filter(|cell| cell.formula().is_some()) {
                            self.register(resolver, tab, cell)?;
                        } else {
                            self.graph.remove((slot.key, at));
                            self.graph.changed((slot.key, at))?;
                        }
                    } else {
                        self.graph.changed((slot.key, at))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn clear_evaluations(&mut self) {
        self.evaluator.clear();
        self.areas.clear();
        self.area_stack.clear();
        self.area_leaves.clear();
        for (_, mut held) in self.suspended.drain() {
            held.clear();
            self.spare.push(held);
        }
    }

    fn prepare(
        &mut self,
        resolver: &ReferenceResolver<'_>,
        styles: &StyleSheet,
        pass: super::formula::graph::PassKind,
        rebuild: bool,
    ) -> Result<super::formula::Recalculation> {
        use super::formula::eval::Evaluation;
        use super::formula::value::{Outcome, Unevaluated};
        self.clear_evaluations();
        self.synchronize(resolver, rebuild)?;
        self.graph.begin(pass, &mut self.schedule)?;
        let mut clock = resolver.book.stated.clock.sample(self.pass)?;
        self.outcomes.clear();
        self.outcomes.resize_with(self.schedule.len(), || None);
        self.receipts.clear();
        self.receipts.reserve(self.schedule.len());
        self.replacements.clear();
        let mut report = super::formula::Recalculation::default();
        while let Some((key, at)) = self.graph.next(&mut self.schedule) {
            let index = self.graph.scheduled_index(&self.schedule, (key, at))
                .expect("the next root belongs to this pass");
            let tab = self.tabs[&key];
            let slot = &resolver.book.slots[tab];
            let sheet = slot.parsed.get().expect("parsed above");
            let cell = sheet
                .cell(at)
                .expect("the synchronized graph names a formula cell");
            let formula = cell
                .formula()
                .expect("the synchronized node is still a formula");
            let expression = if sheet.scalar_formula_at(at) { formula.expression().ok() } else { None };
            let resumed = self.suspended.remove(&index);
            let random = if let Some(mut held) = resumed {
                std::mem::swap(&mut self.evaluator, &mut held.evaluator);
                std::mem::swap(&mut self.areas, &mut held.areas);
                let random = held.random.take();
                held.clear();
                self.spare.push(held);
                random
            } else {
                self.areas.clear();
                if let Some(expression) = expression { self.evaluator.begin(expression); }
                None
            };
            let mut context = CalculationContext {
                resolver,
                root: expression,
                host: resolver.host_at(tab, at).expect("a synchronized host is in grid"),
                selective: self.graph.selective((key, at)),
                graph: &mut self.graph,
                schedule: &mut self.schedule,
                outcomes: &self.outcomes,
                areas: &mut self.areas,
                area_stack: &mut self.area_stack,
                area_leaves: &mut self.area_leaves,
                clock: &mut clock,
                random,
                random_sheet: key.0,
            };
            let result = if !sheet.scalar_formula_at(at) {
                Ok(Evaluation::Complete(Outcome::Uncomputed(Unevaluated::Array)))
            } else {
                match formula.expression() {
                    Ok(expression) => self.evaluator.resume(expression, &mut context),
                    Err(reason) => Ok(Evaluation::Complete(Outcome::Uncomputed(Unevaluated::Held(reason)))),
                }
            };
            let random = context.random;
            let mut outcome = match result? {
                Evaluation::Complete(outcome) => outcome,
                Evaluation::Paused => {
                    let mut held = self.spare.pop().unwrap_or_default();
                    std::mem::swap(&mut self.evaluator, &mut held.evaluator);
                    std::mem::swap(&mut self.areas, &mut held.areas);
                    held.random = random;
                    self.suspended.insert(index, held);
                    continue;
                }
            };
            let volatile = expression.is_some() && self.evaluator.volatile();
            self.evaluator.clear();
            self.areas.clear();
            self.receipts.push(matches!(outcome, Outcome::Computed(_)));
            if let Outcome::Computed(ref value) = outcome {
                let format = styles.shown_number_format(cell.style(), cell.format());
                let raw = match value {
                    super::formula::value::Operand::Number(raw) => Some(*raw),
                    super::formula::value::Operand::Blank => Some(0.0),
                    _ => None,
                };
                let changed = sheet.plan_calculated(cell, value.clone(), format)?;
                let candidate = changed.as_ref().map_or(cell, |(candidate, _)| candidate);
                // The current pass observes the exact calculated serial, even
                // where its typed temporal cache has millisecond resolution.
                outcome = candidate.calculation_operand(resolver.book.system, raw);
                if let Some((changed, bits)) = changed {
                    self.replacements.push((tab, changed, bits));
                }
                report.evaluated += 1;
            }
            self.outcomes[index] = Some(outcome);
            self.graph.complete(&mut self.schedule, (key, at), volatile);
        }
        self.graph.finish(&mut self.schedule);
        self.clear_evaluations();
        // Acknowledgment validates the entire receipt before changing status.
        // Workbook cache publication below is infallible after this point.
        self.graph.acknowledge(&self.schedule, &self.receipts)?;
        (report.uncomputed, report.circular_count) = self.graph.status_counts();
        if report.circular_count != 0 {
            let mut first: SmallVec<[(SheetKey, CellRef); 256]> = SmallVec::new();
            for address in self.graph.circular_cells() {
                let at = first.partition_point(|held| *held < address);
                if at < 256 {
                    if first.len() == 256 {
                        first.pop();
                    }
                    first.insert(at, address);
                }
            }
            report.circular.extend(
                first
                    .into_iter()
                    .map(|(key, at)| (resolver.book.slots[self.tabs[&key]].name.clone(), at)),
            );
        }
        Ok(report)
    }
}

/// Reference handles and pass outcomes live only during one evaluation. The
/// Context lends cells from the same workbook that proved each descriptor.
struct CalculationContext<'a, 'w> {
    resolver: &'a ReferenceResolver<'w>,
    root: Option<&'w super::formula::parser::Expr>,
    host: ReferenceHost<'w>,
    selective: bool,
    graph: &'a mut super::formula::graph::Graph,
    schedule: &'a mut super::formula::graph::Schedule,
    outcomes: &'a [Option<super::formula::value::Outcome>],
    areas: &'a mut Vec<ReadArea>,
    area_stack: &'a mut Vec<super::formula::value::ReferenceId>,
    area_leaves: &'a mut Vec<super::formula::value::ReferenceId>,
    clock: &'a mut super::formula::PassClock,
    random: Option<u64>,
    random_sheet: u32,
}

impl CalculationContext<'_, '_> {
    /// Flatten an arena DAG into ordered rectangle identities. Duplicate and
    /// overlapping leaves remain separate, as Excel's union fold requires.
    fn collect_areas(&mut self, root: super::formula::value::ReferenceId)
        -> std::result::Result<(), super::formula::value::Unevaluated> {
        self.area_stack.clear();
        self.area_stack.push(root);
        while let Some(id) = self.area_stack.pop() {
            match self.areas[id.0].shape {
                ReadShape::Area(_) => {
                    if self.area_leaves.len() == super::formula::MAX_FORMULA_LENGTH {
                        self.area_stack.clear();
                        return Err(super::formula::value::Unevaluated::ReferenceComplexity);
                    }
                    self.area_leaves.push(id);
                }
                ReadShape::Union(left, right) => {
                    self.area_stack.push(right);
                    self.area_stack.push(left);
                }
            }
        }
        Ok(())
    }

    fn visit_one_area(
        &mut self,
        area: ReferenceArea,
        read: super::formula::eval::RangeRead,
        from: u64,
        mut visit: impl FnMut(super::formula::value::Outcome, u64) -> std::ops::ControlFlow<()>,
    ) -> Result<super::formula::eval::RangeProgress> {
        use super::formula::eval::{RangeProgress, RangeRead};
        use super::formula::value::{Operand, Outcome};
        let dynamic = read == RangeRead::Lookup;
        let dense = matches!(read, RangeRead::ValuesWithBlanks | RangeRead::BlankPresence
            | RangeRead::NumericDense | RangeRead::Lookup);
        let start = area.range.start();
        let end = area.range.end();
        let width = u64::from(end.column() - start.column()) + 1;
        let length = area.range.cell_count();
        debug_assert!(from <= length);
        if from == length { return Ok(RangeProgress::Complete); }
        debug_assert!(!dynamic || (area.geometry().is_some()
            && (area.range.row_size() == 1 || area.range.column_size() == 1)));
        let first = if from == 0 { start }
            else if area.range.column_size() == 1 { CellRef::new(start.row() + from as u32, start.column()) }
            else if area.range.row_size() == 1 { CellRef::new(start.row(), start.column() + from as u32) }
            else { start };
        // A row-major 2D suffix is one partial row followed by full rows.
        // One bounding rectangle would omit earlier columns on later rows.
        let (remaining, tail) = if from > 0 && area.range.column_size() > 1
            && area.range.row_size() > 1 {
            let row = start.row() + (from / width) as u32;
            let column = start.column() + (from % width) as u32;
            (CellRange::new(CellRef::new(row, column), CellRef::new(row, end.column())),
             (row < end.row()).then(|| CellRange::new(
                 CellRef::new(row + 1, start.column()), end)))
        } else {
            (CellRange::new(first, end), None)
        };
        let dependent = (self.resolver.book.slots[self.host.tab].key, self.host.at);
        for slot in &self.resolver.book.slots[area.first_tab..area.end_tab] {
            let mut next = from;
            let sheet = slot.parsed.get().expect("the resolver parsed worksheets");
            for cell in sheet.cells_in(remaining)
                .chain(tail.into_iter().flat_map(|range| sheet.cells_in(range))) {
                let at = cell.reference();
                let position = u64::from(at.row() - start.row()) * width
                    + u64::from(at.column() - start.column());
                if dense && position > next {
                    let blank = position - next;
                    next = position;
                    if visit(Outcome::Computed(Operand::Blank), blank).is_break() {
                        if dynamic { Self::admit_lookup_prefix(&mut *self.graph, &mut *self.schedule, dependent, slot.key, area, next)?; }
                        return Ok(RangeProgress::Complete);
                    }
                }
                if dynamic {
                    if cell.formula().is_some() && self.graph.pending(self.schedule, (slot.key, at)) {
                        let point = CellRange::new(at, at);
                        self.graph.admit(self.schedule, dependent, slot.key, point, std::iter::once(at))?;
                        return Ok(RangeProgress::Paused(position));
                    }
                }
                next = position + 1;
                if let RangeRead::Subtotal { exclude_hidden } = read {
                    if !Self::subtotal_value_source(slot, at, cell, exclude_hidden) {
                        continue;
                    }
                }
                if let Some(value) = self.read(slot, at, Some(cell), read) {
                    if visit(value, 1).is_break() {
                        if dynamic { Self::admit_lookup_prefix(&mut *self.graph, &mut *self.schedule, dependent, slot.key, area, next)?; }
                        return Ok(RangeProgress::Complete);
                    }
                }
            }
            if dense && next < length {
                let blank = length - next;
                next = length;
                if visit(Outcome::Computed(Operand::Blank), blank).is_break() {
                    if dynamic { Self::admit_lookup_prefix(&mut *self.graph, &mut *self.schedule, dependent, slot.key, area, next)?; }
                    return Ok(RangeProgress::Complete);
                }
            }
        }
        if dynamic { Self::admit_lookup_prefix(&mut *self.graph, &mut *self.schedule, dependent,
            self.resolver.book.slots[area.first_tab].key, area, length)?; }
        Ok(RangeProgress::Complete)
    }
    fn subtotal_value_source(slot: &Slot, at: CellRef, cell: &Cell, exclude_hidden: bool) -> bool {
        let sheet = slot.parsed.get().expect("the resolver parsed worksheets");
        !(exclude_hidden && sheet.is_row_hidden(at.row())
            || cell.formula().is_some_and(|formula| formula.expression().ok()
                .is_some_and(|expr| expr.has_subtotal)))
    }

    fn read(
        &self,
        slot: &Slot,
        at: CellRef,
        known: Option<&Cell>,
        mode: super::formula::eval::RangeRead,
    ) -> Option<super::formula::value::Outcome> {
        use super::formula::eval::RangeRead;
        use super::formula::graph::Status;
        use super::formula::value::{Operand, Outcome, Unevaluated};
        let address = (slot.key, at);
        // Outcome/status precedes type filtering: a held formula's old text,
        // Boolean or blank cache never makes that dependency disappear.
        if let Some(index) = self.graph.scheduled_index(self.schedule, address) {
            return match &self.outcomes[index] {
                Some(Outcome::Computed(Operand::Blank)) if mode == RangeRead::AggregateA => None,
                Some(Outcome::Computed(Operand::Text(_))) if mode == RangeRead::AggregateA =>
                    Some(Outcome::Computed(Operand::Number(0.0))),
                Some(Outcome::Computed(Operand::Boolean(value))) if mode == RangeRead::AggregateA =>
                    Some(Outcome::Computed(Operand::Number(f64::from(*value as u8)))),
                Some(Outcome::Computed(Operand::Blank | Operand::Text(_)
                    | Operand::Boolean(_))) if mode == RangeRead::NumericDense =>
                    Some(Outcome::Computed(Operand::Blank)),
                Some(Outcome::Computed(operand)) if mode == RangeRead::BlankPresence => {
                    let blank = matches!(operand, Operand::Blank)
                        || matches!(operand, Operand::Text(text) if text.as_str().is_empty());
                    Some(Outcome::Computed(if blank { Operand::Blank }
                        else { Operand::Boolean(true) }))
                }
                Some(Outcome::Computed(Operand::Blank)) if mode == RangeRead::Presence => None,
                Some(Outcome::Computed(_)) if mode == RangeRead::Presence => {
                    Some(Outcome::Computed(Operand::Boolean(true)))
                }
                Some(Outcome::Computed(Operand::Blank | Operand::Text(_)))
                    if !mode.includes_values() => None,
                Some(Outcome::Computed(Operand::Boolean(_)))
                    if mode == RangeRead::Numbers => None,
                Some(outcome) => Some(outcome.clone()),
                None => Some(Outcome::Uncomputed(Unevaluated::Reference)),
            };
        }
        if self
            .graph
            .status(address)
            .is_some_and(|status| status != Status::Computed)
        {
            return Some(Outcome::Uncomputed(Unevaluated::Reference));
        }
        // Range iteration already lends the cell; scalar references look it
        // up once, only after scheduled outcomes and held status were checked.
        let sheet = slot.parsed.get().expect("the resolver parsed worksheets");
        let cell = known.or_else(|| sheet.cell(at));
        if mode == RangeRead::NumericDense {
            return Some(match cell {
                None => Outcome::Computed(Operand::Blank),
                Some(cell) if cell.error().is_none()
                    && (cell.value().is_null() || cell.kind().is_text()
                        || cell.value().as_bool().is_some()) =>
                    Outcome::Computed(Operand::Blank),
                Some(cell) => sheet.calculation_operand_at(cell),
            });
        }
        if mode == RangeRead::BlankPresence {
            if let Some(cell) = cell {
                if cell.error() == Some(super::cell::ExcelError::Unrecognized) {
                    return Some(sheet.calculation_operand_at(cell));
                }
            }
            let blank = cell.is_none_or(|cell| cell.error().is_none()
                && (cell.value().is_null()
                    || cell.value().as_str().is_some_and(str::is_empty)));
            return Some(Outcome::Computed(if blank { Operand::Blank }
                else { Operand::Boolean(true) }));
        }
        if mode == RangeRead::Presence {
            return cell.filter(|cell| cell.error().is_some() || !cell.value().is_null())
                .map(|_| Outcome::Computed(Operand::Boolean(true)));
        }
        if mode == RangeRead::AggregateA {
            let cell = cell?;
            if cell.error().is_none() {
                if cell.value().is_null() { return None; }
                if cell.kind().is_text() {
                    return Some(Outcome::Computed(Operand::Number(0.0)));
                }
                if let Some(value) = cell.value().as_bool() {
                    return Some(Outcome::Computed(Operand::Number(f64::from(value as u8))));
                }
            }
        }
        if !mode.includes_values()
            && cell.is_none_or(|cell| {
                cell.error().is_none()
                    && (cell.kind().is_text()
                        || (mode == RangeRead::Numbers
                            && cell.kind() == super::cell::CellKind::Boolean)
                        || cell.value().is_null())
            })
        {
            return None;
        }
        Some(cell.map_or(Outcome::Computed(Operand::Blank), |cell| {
            sheet.calculation_operand_at(cell)
        }))
    }
    /// Publish only the visited prefix after every source value in it proved
    /// ready. The empty iterator avoids a second cell scan; pending sources
    /// were admitted individually before this point and have completed.
    fn admit_lookup_prefix(
        graph: &mut super::formula::graph::Graph,
        schedule: &mut super::formula::graph::Schedule,
        dependent: (SheetKey, CellRef), source: SheetKey,
        area: ReferenceArea, count: u64,
    ) -> Result<()> {
        if count == 0 { return Ok(()); }
        let start = area.range.start();
        let last = if area.range.column_size() == 1 {
            CellRef::new(start.row() + count as u32 - 1, start.column())
        } else {
            CellRef::new(start.row(), start.column() + count as u32 - 1)
        };
        let prefix = CellRange::new(start, last);
        graph.admit(schedule, dependent, source, prefix, std::iter::empty())
    }
}

impl<'w> super::formula::eval::Context<'w> for CalculationContext<'_, 'w> {
    fn system(&self) -> DateSystem {
        self.resolver.book.system
    }

    fn text_compatibility(&self) -> super::formula::text::Compatibility {
        self.resolver.book.stated.text_compatibility
    }

    fn clock_serial(&mut self, today: bool) -> Result<f64> {
        self.clock.serial(self.resolver.book.system, today)
    }

    fn random_u64(&mut self) -> u64 {
        self.clock.draw(&mut self.random, self.random_sheet, self.host.at)
    }

    fn reference(
        &mut self,
        name: Option<NameId>,
        reference: &super::formula::reference::Reference,
    ) -> Result<super::formula::eval::ReferenceResult> {
        use super::formula::eval::ReferenceResult;
        use super::formula::value::{Operand, Outcome, ReferenceId, Unevaluated};
        Ok(match self.resolver.resolve(ReferenceHost { name, ..self.host }, reference)? {
            ResolvedReference::Range(range) => {
                let id = ReferenceId(self.areas.len());
                if self.areas.len() == super::formula::MAX_FORMULA_LENGTH {
                    return Ok(ReferenceResult::Value(Outcome::Uncomputed(
                        Unevaluated::ReferenceComplexity)));
                }
                self.areas.push(ReadArea::area(range.descriptor()));
                ReferenceResult::Value(Outcome::Computed(Operand::Reference(id)))
            }
            ResolvedReference::Error(error) => ReferenceResult::Value(Outcome::Computed(Operand::Error(error))),
            ResolvedReference::Held(reason) => {
                let reason = match reason {
                    ReferenceHeld::Shape(reason) => Unevaluated::Held(reason),
                    ReferenceHeld::NameAnchor => Unevaluated::ReferenceAnchor,
                    ReferenceHeld::NameScope => Unevaluated::ReferenceScope,
                    ReferenceHeld::NonWorksheet => Unevaluated::ReferenceNonWorksheet,
                };
                ReferenceResult::Value(Outcome::Uncomputed(reason))
            },
            ResolvedReference::Name(binding) => ReferenceResult::Name(binding.id),
        })
    }

    fn expression(&self, name: Option<NameId>) -> std::result::Result<&'w super::formula::parser::Expr, super::formula::shape::Held> {
        match name {
            Some(name) => self.resolver.name_expression(name),
            None => Ok(self.root.expect("an active evaluator has a parsed root")),
        }
    }

    fn host(&self) -> CellRef { self.host.at }

    fn reference_combine(
        &mut self,
        op: super::formula::parser::BinaryOp,
        left: super::formula::value::ReferenceId,
        right: super::formula::value::ReferenceId,
    ) -> super::formula::value::Outcome {
        use super::cell::ExcelError;
        use super::formula::parser::BinaryOp;
        use super::formula::value::{Operand, Outcome, ReferenceId, Unevaluated};

        self.area_leaves.clear();
        if let Err(reason) = self.collect_areas(left) {
            return Outcome::Uncomputed(reason);
        }
        let split = self.area_leaves.len();
        if let Err(reason) = self.collect_areas(right) {
            return Outcome::Uncomputed(reason);
        }
        let mut tab = None;
        for &id in self.area_leaves.iter() {
            let ReadShape::Area(area) = self.areas[id.0].shape else {
                unreachable!("the flattened arena contains rectangles")
            };
            if area.end_tab != area.first_tab + 1 {
                return Outcome::Uncomputed(Unevaluated::Reference);
            }
            match tab {
                Some(previous) if previous != area.first_tab =>
                    return Outcome::Computed(Operand::Error(ExcelError::Value)),
                None => tab = Some(area.first_tab),
                _ => {}
            }
        }
        let result = match op {
            BinaryOp::Union => {
                if self.areas.len() == super::formula::MAX_FORMULA_LENGTH {
                    return Outcome::Uncomputed(Unevaluated::ReferenceComplexity);
                }
                let id = ReferenceId(self.areas.len());
                self.areas.push(ReadArea::union(left, right));
                id
            }
            BinaryOp::Range => {
                if split != 1 || self.area_leaves.len() != 2 {
                    return Outcome::Uncomputed(Unevaluated::Binary(op));
                }
                if self.areas.len() == super::formula::MAX_FORMULA_LENGTH {
                    return Outcome::Uncomputed(Unevaluated::ReferenceComplexity);
                }
                let ReadShape::Area(first) = self.areas[self.area_leaves[0].0].shape else { unreachable!() };
                let ReadShape::Area(second) = self.areas[self.area_leaves[1].0].shape else { unreachable!() };
                // Native colon probes include two rectangular endpoints and
                // an endpoint contained by the other rectangle. The result
                // is the smallest rectangle enclosing both operands.
                let start = CellRef::new(
                    first.range.start().row().min(second.range.start().row()),
                    first.range.start().column().min(second.range.start().column()));
                let end = CellRef::new(
                    first.range.end().row().max(second.range.end().row()),
                    first.range.end().column().max(second.range.end().column()));
                let range = CellRange::new(start, end);
                let id = ReferenceId(self.areas.len());
                self.areas.push(ReadArea::area(ReferenceArea { range, ..first }));
                id
            }
            BinaryOp::Intersection => {
                // A pair may append one rectangle and one union link. Bind
                // the entire worst-case expansion to the same 8,192-character
                // formula budget that bounds its authored reference tokens.
                let available = super::formula::MAX_FORMULA_LENGTH.saturating_sub(self.areas.len());
                let Some(pairs) = split.checked_mul(self.area_leaves.len() - split) else {
                    return Outcome::Uncomputed(Unevaluated::ReferenceComplexity);
                };
                if pairs.checked_mul(2).is_none_or(|needed| needed > available) {
                    return Outcome::Uncomputed(Unevaluated::ReferenceComplexity);
                }
                let mut joined = None;
                for first_index in 0..split {
                    let ReadShape::Area(first) = self.areas[self.area_leaves[first_index].0].shape else { unreachable!() };
                    for second_index in split..self.area_leaves.len() {
                        let ReadShape::Area(second) = self.areas[self.area_leaves[second_index].0].shape else { unreachable!() };
                        if !first.range.intersects(second.range) { continue; }
                        let start = CellRef::new(
                            first.range.start().row().max(second.range.start().row()),
                            first.range.start().column().max(second.range.start().column()));
                        let end = CellRef::new(
                            first.range.end().row().min(second.range.end().row()),
                            first.range.end().column().min(second.range.end().column()));
                        let id = ReferenceId(self.areas.len());
                        self.areas.push(ReadArea::area(ReferenceArea {
                            range: CellRange::new(start, end), ..first
                        }));
                        joined = Some(if let Some(previous) = joined {
                            let union = ReferenceId(self.areas.len());
                            self.areas.push(ReadArea::union(previous, id));
                            union
                        } else { id });
                    }
                }
                let Some(id) = joined else {
                    return Outcome::Computed(Operand::Error(ExcelError::Null));
                };
                id
            }
            _ => unreachable!("only reference operators combine reference areas"),
        };
        Outcome::Computed(Operand::Reference(result))
    }

    fn reference_is_union(&self, id: super::formula::value::ReferenceId) -> bool {
        matches!(self.areas[id.0].shape, ReadShape::Union(..))
    }

    fn reference_nth(&mut self, id: super::formula::value::ReferenceId, index: usize)
        -> super::formula::value::Outcome {
        use super::formula::value::{Operand, Outcome};
        self.area_leaves.clear();
        if let Err(reason) = self.collect_areas(id) {
            return Outcome::Uncomputed(reason);
        }
        self.area_leaves.get(index).copied().map_or(
            Outcome::Computed(Operand::Error(super::cell::ExcelError::Ref)),
            |area| Outcome::Computed(Operand::Reference(area)))
    }

    fn reference_geometry(&self, id: super::formula::value::ReferenceId) -> Option<CellRange> {
        match self.areas[id.0].shape {
            ReadShape::Area(area) => area.geometry(),
            ReadShape::Union(..) => None,
        }
    }

    fn intersection(&mut self, id: super::formula::value::ReferenceId) -> super::formula::value::Outcome {
        use super::formula::value::{Operand, Outcome, ReferenceId, Unevaluated};
        let ReadShape::Area(area) = self.areas[id.0].shape else {
            return Outcome::Computed(Operand::Error(super::cell::ExcelError::Value));
        };
        match area.selected(super::formula::parser::ReferenceUse::Scalar, self.host.at) {
            Ok(Some(area)) => {
                if self.areas.len() == super::formula::MAX_FORMULA_LENGTH {
                    return Outcome::Uncomputed(Unevaluated::ReferenceComplexity);
                }
                let ready = self.areas[id.0].scalar;
                let selected = ReferenceId(self.areas.len());
                self.areas.push(ReadArea { shape: ReadShape::Area(area), scalar: ready,
                    range: ready, subtotal: [false; 2] });
                Outcome::Computed(Operand::Reference(selected))
            }
            Ok(None) => Outcome::Uncomputed(Unevaluated::Reference),
            Err(error) => Outcome::Computed(Operand::Error(error)),
        }
    }

    fn reference_range(&mut self, id: super::formula::value::ReferenceId, range: CellRange) -> super::formula::value::Outcome {
        use super::formula::value::{Operand, Outcome, ReferenceId};
        let ReadShape::Area(area) = self.areas[id.0].shape else {
            return Outcome::Computed(Operand::Error(super::cell::ExcelError::Value));
        };
        if self.areas.len() == super::formula::MAX_FORMULA_LENGTH {
            return Outcome::Uncomputed(super::formula::value::Unevaluated::ReferenceComplexity);
        }
        let selected = ReferenceId(self.areas.len());
        self.areas.push(ReadArea::area(ReferenceArea { range, ..area }));
        Outcome::Computed(Operand::Reference(selected))
    }

    fn ready(&mut self, id: super::formula::value::ReferenceId,
        usage: super::formula::parser::ReferenceUse) -> Result<bool> {
        use super::formula::parser::ReferenceUse;
        // Geometry and rejected union consumers must never register value
        // dependencies, including cycles through an otherwise unused area.
        if !self.selective || usage == ReferenceUse::Geometry
            || (usage != ReferenceUse::Range && self.reference_is_union(id)) {
            return Ok(true);
        }
        let address = (self.resolver.book.slots[self.host.tab].key, self.host.at);
        let prior = match usage {
            ReferenceUse::Scalar => self.areas[id.0].scalar,
            ReferenceUse::Range | ReferenceUse::SingleRange => self.areas[id.0].range,
            ReferenceUse::Geometry => unreachable!("returned above"),
        };
        if !prior {
            self.area_leaves.clear();
            if self.collect_areas(id).is_err() { return Ok(true); }
            for &leaf in self.area_leaves.iter() {
                let ReadShape::Area(area) = self.areas[leaf.0].shape else { unreachable!() };
                if let Ok(Some(area)) = area.selected(usage, self.host.at) {
                    for slot in &self.resolver.book.slots[area.first_tab..area.end_tab] {
                        self.graph.admit(
                            self.schedule, address, slot.key, area.range,
                            slot.parsed.get().expect("parsed above").cells_in(area.range)
                                .map(Cell::reference),
                        )?;
                    }
                }
            }
            match usage {
                ReferenceUse::Scalar => self.areas[id.0].scalar = true,
                ReferenceUse::Range | ReferenceUse::SingleRange => self.areas[id.0].range = true,
                ReferenceUse::Geometry => unreachable!("returned above"),
            }
        }
        Ok(self.graph.ready(self.schedule, address))
    }

    fn ready_subtotal(&mut self, id: super::formula::value::ReferenceId,
        exclude_hidden: bool) -> Result<bool> {
        let index = usize::from(exclude_hidden);
        let address = (self.resolver.book.slots[self.host.tab].key, self.host.at);
        if !self.areas[id.0].subtotal[index] {
            self.area_leaves.clear();
            if self.collect_areas(id).is_err() { return Ok(true); }
            for &leaf in self.area_leaves.iter() {
                let ReadShape::Area(area) = self.areas[leaf.0].shape else { unreachable!() };
                for slot in &self.resolver.book.slots[area.first_tab..area.end_tab] {
                    // The watch retains future source changes; a nested
                    // SUBTOTAL remains a graph edge even when its value is
                    // excluded from this fold.
                    self.graph.watch(self.schedule, address, slot.key, area.range)?;
                    let sheet = slot.parsed.get().expect("the resolver parsed worksheets");
                    for cell in sheet.cells_in(area.range) {
                        let at = cell.reference();
                        if cell.formula().is_some()
                            && !(exclude_hidden && sheet.is_row_hidden(at.row()))
                            && self.graph.pending(self.schedule, (slot.key, at))
                        {
                            self.graph.admit(self.schedule, address, slot.key,
                                CellRange::new(at, at), std::iter::once(at))?;
                        }
                    }
                }
            }
            self.areas[id.0].subtotal[index] = true;
        }
        Ok(self.graph.ready(self.schedule, address))
    }

    fn scalar(
        &mut self,
        value: super::formula::value::Operand,
    ) -> Result<super::formula::value::Outcome> {
        use super::formula::value::{Operand, Outcome, Unevaluated};
        let Operand::Reference(id) = value else {
            return Ok(Outcome::Computed(value));
        };
        let ReadShape::Area(source) = self.areas[id.0].shape else {
            return Ok(Outcome::Computed(Operand::Error(super::cell::ExcelError::Value)));
        };
        let area = match source.selected(super::formula::parser::ReferenceUse::Scalar, self.host.at) {
            Ok(Some(area)) if area.range.start() == area.range.end() => area,
            Ok(_) => return Ok(Outcome::Uncomputed(Unevaluated::Reference)),
            Err(error) => return Ok(Outcome::Computed(Operand::Error(error))),
        };
        let at = area.range.start();
        Ok(self
            .read(
                &self.resolver.book.slots[area.first_tab],
                at,
                None,
                super::formula::eval::RangeRead::Values,
            )
            .expect("Values includes blank and text cells"))
    }

    fn visit_range(
        &mut self,
        id: super::formula::value::ReferenceId,
        read: super::formula::eval::RangeRead,
        from: u64,
        mut visit: impl FnMut(super::formula::value::Outcome, u64) -> std::ops::ControlFlow<()>,
    ) -> Result<super::formula::eval::RangeProgress> {
        use super::formula::eval::{RangeProgress, RangeRead};
        use super::formula::value::{Operand, Outcome};
        if let ReadShape::Area(area) = self.areas[id.0].shape {
            return self.visit_one_area(area, read, from, visit);
        }
        self.area_leaves.clear();
        if let Err(reason) = self.collect_areas(id) {
            let _ = visit(Outcome::Uncomputed(reason), 1);
            return Ok(RangeProgress::Complete);
        }
        if read == RangeRead::Lookup && self.area_leaves.len() != 1 {
            let _ = visit(Outcome::Computed(Operand::Error(super::cell::ExcelError::Value)), 1);
            return Ok(RangeProgress::Complete);
        }
        let mut offset = 0_u64;
        for position in 0..self.area_leaves.len() {
            let leaf = self.area_leaves[position];
            let ReadShape::Area(area) = self.areas[leaf.0].shape else { unreachable!() };
            let length = area.range.cell_count();
            if from >= offset + length {
                offset += length;
                continue;
            }
            let mut stopped = false;
            let result = self.visit_one_area(area, read, from.saturating_sub(offset), |value, count| {
                let answer = visit(value, count);
                stopped |= answer.is_break();
                answer
            })?;
            if result != RangeProgress::Complete { return Ok(result); }
            if stopped { return Ok(RangeProgress::Complete); }
            offset += length;
        }
        Ok(RangeProgress::Complete)
    }

}

/// One `<sheet>` of the workbook, its part, and the [`Sheet`] once parsed.
#[derive(Clone, Debug)]
pub(crate) struct Slot {
    name: SmolStr,
    key: SheetKey,
    /// `sheetId`: the tab's number in the file, kept through every save.
    sheet_id: u32,
    kind: SheetKind,
    /// The state the package's workbook part states for the tab.
    state: SheetState,
    /// The sheet's package identity, reserved before insertion and kept
    /// through reordering, saves and undo.
    part: SmolStr,
    /// This key has a serialized image in the source or a retained override.
    /// A different key's image at a reused path never backs this slot.
    backed: bool,
    parsed: OnceLock<Sheet>,
    /// The revision of the parsed sheet the member holds.
    saved: u64,
}

impl Slot {
    /// The tab's name.
    pub(crate) const fn name(&self) -> &SmolStr {
        &self.name
    }

    /// An estimate of the bytes the slot holds, its parsed sheet's cells
    /// counted.
    pub(crate) fn byte_size(&self) -> usize {
        self.parsed
            .get()
            .map_or(0, |sheet| sheet.cell_count() * 96 + sheet.len() * 64)
            + 256
    }

    /// The sheet's state: the parsed sheet's once it is held, else what the
    /// workbook part stated.
    fn state(&self) -> SheetState {
        self.parsed.get().map_or(self.state, Sheet::state)
    }

    /// Whether a save writes the sheet from its cells: no member holds it,
    /// or it changed since the member was written. A sheet never parsed, or
    /// only read, is the member as it stands.
    fn is_dirty(&self) -> bool {
        !self.backed
            || self
                .parsed
                .get()
                .is_some_and(|sheet| sheet.revision() != self.saved)
    }
}

/// Package bytes owned by a removed sheet, retained only by its undo. Shared
/// descendants belong to the removal that makes their last owner disappear.
#[derive(Clone, Debug, Default)]
pub(crate) struct RemovedParts {
    /// Sorted by member name when captured from the orphan delta.
    parts: Vec<(SmolStr, Arc<[u8]>)>,
    types: Vec<Registration>,
    relationships: Vec<Registration>,
    caches: Vec<Registration>,
}

impl RemovedParts {
    /// Membership in the one retained, sorted orphan delta.
    fn contains(&self, member: &str) -> bool {
        self.parts
            .binary_search_by(|(name, _)| name.as_str().cmp(member))
            .is_ok()
    }

    pub(crate) fn byte_size(&self) -> usize {
        self.parts
            .iter()
            .map(|(name, bytes)| name.len() + bytes.len())
            .sum::<usize>()
            + self
                .types
                .iter()
                .chain(&self.relationships)
                .chain(&self.caches)
                .map(|entry| {
                    entry.key.len()
                        + entry.xml.len()
                        + entry.inherited_markup.byte_size()
                        + entry.markup.byte_size()
                        + entry
                            .namespaces
                            .iter()
                            .map(|(key, value)| key.len() + value.len())
                            .sum::<usize>()
                })
                .sum::<usize>()
    }
}

/// Transfer semantics are ECMA-376 Part 3, fifth edition (2015), §§7.2,
/// 7.3 and 9.2: namespace/name pairs accumulate, not their prefix spellings.
const MARKUP_NAMESPACE: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct MarkupContext {
    ignorable: BTreeSet<SmolStr>,
    process: BTreeSet<(SmolStr, SmolStr)>,
    /// Other editions' directives have no inferred inheritance semantics.
    /// Keep authored bytes; moving away from this inherited context refuses.
    opaque: Vec<(SmolStr, SmolStr, BTreeMap<SmolStr, SmolStr>)>,
}

impl MarkupContext {
    fn empty() -> Arc<Self> {
        static EMPTY: std::sync::LazyLock<Arc<MarkupContext>> =
            std::sync::LazyLock::new(|| Arc::new(MarkupContext::default()));
        Arc::clone(&EMPTY)
    }

    fn byte_size(&self) -> usize {
        self.ignorable.iter().map(SmolStr::len).sum::<usize>()
            + self
                .process
                .iter()
                .map(|(namespace, name)| namespace.len() + name.len())
                .sum::<usize>()
            + self
                .opaque
                .iter()
                .map(|(name, value, bindings)| {
                    name.len()
                        + value.len()
                        + bindings
                            .iter()
                            .map(|(name, value)| name.len() + value.len())
                            .sum::<usize>()
                })
                .sum::<usize>()
    }

    fn at(
        parent: &Arc<Self>,
        reader: &quick_xml::NsReader<&[u8]>,
        start: &quick_xml::events::BytesStart<'_>,
        position: usize,
    ) -> Result<Arc<Self>> {
        let mut changed = None;
        let name = |value: &str| {
            let mut characters = value.chars();
            characters
                .next()
                .is_some_and(|first| crate::xml::is_name_start(first) && first != ':')
                && characters
                    .all(|character| crate::xml::is_name_char(character) && character != ':')
        };
        for attribute in start.attributes() {
            let attribute =
                attribute.map_err(|error| package::codec_error(position, error.to_string()))?;
            let (namespace, attribute_name) = reader.resolver().resolve_attribute(attribute.key);
            if !Registration::in_namespace(namespace, &[MARKUP_NAMESPACE])? {
                continue;
            }
            // MustUnderstand is examined where authored (§9.4); it is not
            // inherited by the transferred subtree. Its own bytes stay intact.
            if attribute_name.as_ref() == b"MustUnderstand" {
                continue;
            }
            let value = attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|error| package::codec_error(position, error.to_string()))?;
            let context = changed.get_or_insert_with(|| Self::clone(parent));
            let namespace = |prefix: &str| -> Result<SmolStr> {
                if !name(prefix) {
                    return Err(package::codec_error(
                        position,
                        format!("expected an MCE namespace prefix, got `{prefix}`"),
                    ));
                }
                let qualified = format!("{prefix}:_");
                let resolved = reader
                    .resolver()
                    .resolve_prefix(quick_xml::name::QName(qualified.as_bytes()).prefix(), false);
                let quick_xml::name::ResolveResult::Bound(namespace) = resolved else {
                    return Err(package::codec_error(
                        position,
                        format!("expected a bound MCE prefix, got `{prefix}`"),
                    ));
                };
                let uri = std::str::from_utf8(namespace.as_ref())
                    .map_err(|error| package::codec_error(position, error.to_string()))?;
                let uri = quick_xml::escape::unescape(uri)
                    .map_err(|error| package::codec_error(position, error.to_string()))?;
                if uri == MARKUP_NAMESPACE {
                    return Err(package::codec_error(
                        position,
                        "Ignorable and ProcessContent cannot name the MCE namespace",
                    ));
                }
                Ok(SmolStr::new(uri))
            };
            match attribute_name.as_ref() {
                b"Ignorable" => {
                    for prefix in value
                        .split([' ', '\t', '\r', '\n'])
                        .filter(|token| !token.is_empty())
                    {
                        context.ignorable.insert(namespace(prefix)?);
                    }
                }
                b"ProcessContent" => {
                    for token in value
                        .split([' ', '\t', '\r', '\n'])
                        .filter(|token| !token.is_empty())
                    {
                        let (prefix, local) = token
                            .split_once(':')
                            .filter(|(prefix, local)| {
                                name(prefix) && (*local == "*" || name(local))
                            })
                            .ok_or_else(|| {
                                package::codec_error(
                                    position,
                                    format!("expected an MCE ProcessContent QName, got `{token}`"),
                                )
                            })?;
                        context.process.insert((namespace(prefix)?, local.into()));
                    }
                }
                _ => context.opaque.push((
                    std::str::from_utf8(attribute_name.as_ref())
                        .map_err(|error| package::codec_error(position, error.to_string()))?
                        .into(),
                    SmolStr::new(value),
                    Registration::namespaces(reader)?,
                )),
            }
        }
        if let Some(context) = &changed {
            for (namespace, _) in &context.process {
                if !context.ignorable.contains(namespace) {
                    return Err(package::codec_error(
                        position,
                        format!(
                            "expected an Ignorable ProcessContent namespace, got `{namespace}`"
                        ),
                    ));
                }
            }
        }
        Ok(changed.map_or_else(|| Arc::clone(parent), Arc::new))
    }

    fn processes(&self, namespace: &str, local: &str) -> bool {
        self.process
            .iter()
            .any(|(uri, name)| uri == namespace && (name == local || name == "*"))
    }

    /// Additional destination policies cannot be subtracted in 2015 MCE.
    /// Prove invariance for this fragment without guessing a consumer's
    /// understood namespaces or selecting AlternateContent branches. A
    /// relevant extra policy whose interpretation cannot be preserved is a
    /// named refusal; unrelated directives do not prevent a transfer.
    fn check_destination(entry: &Registration, destination: &Arc<Self>) -> Result<()> {
        if entry.inherited_markup.opaque != destination.opaque {
            return Err(Error::Unsupported {
                operation: "transfer inherited MCE directives outside ECMA-376 Part 3 (2015)",
                filesystem: format_smolstr!("registration[{}]#mc", entry.key),
            });
        }
        let mut combined = Self::clone(&entry.inherited_markup);
        combined
            .ignorable
            .extend(destination.ignorable.iter().cloned());
        combined.process.extend(destination.process.iter().cloned());
        if combined == *entry.inherited_markup {
            return Ok(());
        }
        let bases = [Arc::clone(&entry.inherited_markup), Arc::new(combined)];
        let mut scopes: Vec<[Arc<Self>; 2]> = Vec::new();
        let xml = entry.fragment_namespaces(&BTreeMap::new())?;
        Registration::select_in(
            xml.as_bytes(),
            Arc::clone(&entry.inherited_markup),
            |reader, start, depth, position| {
                scopes.truncate(depth);
                let parent = scopes.last().unwrap_or(&bases);
                let context = [
                    Self::at(&parent[0], reader, start, position)?,
                    Self::at(&parent[1], reader, start, position)?,
                ];
                let changed = |namespace: quick_xml::name::ResolveResult<'_>,
                               local: &[u8],
                               element: bool|
                 -> Result<bool> {
                    let quick_xml::name::ResolveResult::Bound(namespace) = namespace else {
                        return Ok(false);
                    };
                    let uri = std::str::from_utf8(namespace.as_ref())
                        .map_err(|error| package::codec_error(position, error.to_string()))?;
                    let uri = quick_xml::escape::unescape(uri)
                        .map_err(|error| package::codec_error(position, error.to_string()))?;
                    if uri == MARKUP_NAMESPACE {
                        return Ok(false);
                    }
                    let local = std::str::from_utf8(local)
                        .map_err(|error| package::codec_error(position, error.to_string()))?;
                    let ignored = [
                        context[0].ignorable.contains(uri.as_ref()),
                        context[1].ignorable.contains(uri.as_ref()),
                    ];
                    Ok(ignored[0] != ignored[1]
                        || (element
                            && ignored[0]
                            && context[0].processes(&uri, local)
                                != context[1].processes(&uri, local)))
                };
                let (namespace, name) = reader.resolver().resolve_element(start.name());
                let mut differs = changed(namespace, name.as_ref(), true)?;
                for attribute in start.attributes() {
                    let attribute = attribute
                        .map_err(|error| package::codec_error(position, error.to_string()))?;
                    let (namespace, name) = reader.resolver().resolve_attribute(attribute.key);
                    differs |= changed(namespace, name.as_ref(), false)?;
                }
                if differs {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("registration[{}]#MCE@{position}", entry.key),
                        reason: SmolStr::new_static(
                            "cannot preserve inherited MCE interpretation under the destination Ignorable or ProcessContent directives",
                        ),
                    });
                }
                scopes.push(context);
                Ok(None)
            },
        )?;
        Ok(())
    }
}
/// One existing formula-bearing XML field. Its complete scoped fragment is
/// retained so changing a formula never rebuilds surrounding rule metadata.
struct CarriedFormulaField {
    registration: Registration,
    attribute: bool,
}

impl CarriedFormulaField {
    fn text(&self, part: &str) -> Result<String> {
        if self.attribute {
            self.registration
                .root_attribute(b"val")?
                .ok_or_else(|| Error::InvalidRecord {
                    path: format_smolstr!("{part}#cfvo"),
                    reason: "expected the formula threshold's val attribute".into(),
                })
        } else {
            self.registration.plain_text(part)
        }
    }

    fn with_formula(&self, formula: &Formula, host: CellRef) -> Result<Registration> {
        let text = formula.at(host).to_string();
        if self.attribute {
            self.registration
                .with_attributes(vec![("val".into(), Some(text))])
        } else {
            self.registration.with_text(text)
        }
    }
}

impl Registration {
    fn direct_child_count(&self) -> Result<usize> {
        let (bytes, _) = self.contextual();
        Ok(Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |_, _, depth, position| Ok((depth == 2).then(|| format_smolstr!("{position}"))),
        )?.len())
    }

    fn has_children(&self) -> Result<bool> {
        let (bytes, _) = self.contextual();
        Ok(!Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |_, _, depth, position| Ok((depth == 2).then(|| format_smolstr!("{position}"))),
        )?
        .is_empty())
    }

    fn replaced_children(&self, changes: &BTreeMap<SmolStr, Vec<Self>>) -> Result<Self> {
        self.patched_children(changes.len(), |key, _| changes.get(key).map(Vec::as_slice))
    }

    /// Cut a complete formula-bearing worksheet child without joining
    /// unrelated top-level siblings into one Registration.
    fn formula_partition(
        &self,
        name: &str,
        owner: (&str, Option<&str>),
        shift: &Shift<'_>,
        part: &str,
    ) -> Result<Vec<Self>> {
        use super::carried::{ShiftedExtension, X14_NAMESPACE};
        if name == "conditionalFormatting" {
            return self.formula_pieces(
                shift::CarriedFormulaKind::Conditional,
                false,
                owner,
                shift,
                part,
            );
        }
        let main = &[super::NAMESPACE, super::STRICT_NAMESPACE];
        if name == "dataValidations" {
            let children = self.children_named(main, "dataValidation")?;
            if children.is_empty() {
                return Ok(if owner.1.is_none() {
                    vec![self.clone()]
                } else {
                    Vec::new()
                });
            }
            let mut replacements = BTreeMap::new();
            let mut count = 0usize;
            for child in children {
                let pieces = child.formula_pieces(
                    shift::CarriedFormulaKind::Validation,
                    false,
                    owner,
                    shift,
                    part,
                )?;
                count = count
                    .checked_add(pieces.len())
                    .ok_or_else(|| Error::Unsupported {
                        operation: "counting carried-formula registrations",
                        filesystem: SmolStr::new(part),
                    })?;
                if pieces.len() != 1 || pieces[0].xml != child.xml {
                    replacements.insert(child.key.clone(), pieces);
                }
            }
            if replacements.is_empty() {
                return Ok(vec![self.clone()]);
            }
            let rewritten = self.replaced_children(&replacements)?;
            if count == 0 && (owner.1.is_some() || !rewritten.has_children()?) {
                return Ok(Vec::new());
            }
            return Ok(vec![rewritten.with_attributes(vec![(
                "count".into(),
                Some(count.to_string()),
            )])?]);
        }
        debug_assert_eq!(name, "extLst");
        let extensions = self.children_named(main, "ext")?;
        let mut changes = BTreeMap::new();
        for extension in &extensions {
            let kind = extension
                .root_attribute(b"uri")?
                .as_deref()
                .and_then(ShiftedExtension::from_uri);
            let form = match kind {
                Some(ShiftedExtension::ConditionalFormatting) => {
                    Some(shift::CarriedFormulaKind::Conditional)
                }
                Some(ShiftedExtension::DataValidation) => {
                    Some(shift::CarriedFormulaKind::Validation)
                }
                _ => None,
            };
            if let Some(form) = form {
                let (container, child_name) =
                    kind.expect("a formula extension has a kind").container();
                let list = extension.one_child(&[X14_NAMESPACE], container, part)?;
                let mut hosts = BTreeMap::new();
                let mut count = 0usize;
                for host in
                    list.selected_children(&[X14_NAMESPACE], child_name, owner.1.map(|_| part))?
                {
                    let pieces = host.formula_pieces(form, true, owner, shift, part)?;
                    count = count
                        .checked_add(pieces.len())
                        .ok_or_else(|| Error::Unsupported {
                            operation: "counting carried-formula registrations",
                            filesystem: SmolStr::new(part),
                        })?;
                    if pieces.len() != 1 || pieces[0].xml != host.xml {
                        hosts.insert(host.key.clone(), pieces);
                    }
                }
                if hosts.is_empty() && !(count == 0 && owner.1.is_some()) {
                    continue;
                }
                let mut list = list.replaced_children(&hosts)?;
                if count == 0 && (owner.1.is_some() || !list.has_children()?) {
                    changes.insert(extension.key.clone(), None);
                    continue;
                }
                if form == shift::CarriedFormulaKind::Validation {
                    list = list.with_attributes(vec![("count".into(), Some(count.to_string()))])?;
                }
                let mut children = BTreeMap::new();
                children.insert(list.key.clone(), Some(list));
                changes.insert(
                    extension.key.clone(),
                    Some(extension.changed_children(&children)?),
                );
            } else if owner.1.is_none() || kind == Some(ShiftedExtension::Sparkline) {
                // Keep the existing sparkline/unknown-extension adjuster. Its
                // complete extLst wrapper supplies the same path and scopes.
                let remove: BTreeMap<_, _> = extensions
                    .iter()
                    .filter(|other| other.key != extension.key)
                    .map(|other| (other.key.clone(), None))
                    .collect();
                let isolated = self.changed_children(&remove)?;
                if let Some(bytes) = shift::SheetEdits::apply(
                    isolated.xml.as_bytes(),
                    owner.0,
                    shift,
                    part,
                    owner.1,
                )? {
                    if bytes.is_empty() {
                        changes.insert(extension.key.clone(), None);
                    } else {
                        let mut updated = isolated;
                        updated.xml = std::str::from_utf8(&bytes)
                            .map_err(|error| package::codec_error(0, error.to_string()))?
                            .into();
                        let child = updated.children_named(main, "ext")?.pop();
                        changes.insert(extension.key.clone(), child);
                    }
                }
            } else {
                changes.insert(extension.key.clone(), None);
            }
        }
        if changes.is_empty() {
            return Ok(vec![self.clone()]);
        }
        let rewritten = self.changed_children(&changes)?;
        if rewritten.children_named(main, "ext")?.is_empty() {
            Ok(Vec::new())
        } else {
            Ok(vec![rewritten])
        }
    }

    /// Replace disjoint captured descendants by lexical keys. The selector
    /// stops below a selected fragment, so replacements never overlap. These
    /// are text/attribute edits of the same captured fields: their namespace
    /// placement and declarations remain unchanged.
    fn changed_entries(&self, changes: &BTreeMap<SmolStr, Option<Self>>) -> Result<Self> {
        if changes.is_empty() {
            return Ok(self.clone());
        }
        let (bytes, offset) = self.contextual();
        let mut starts = Vec::new();
        let entries = Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |_, _, depth, position| {
                if depth <= 1 {
                    return Ok(None);
                }
                let key = format_smolstr!("{position}");
                if !changes.contains_key(&key) {
                    return Ok(None);
                }
                starts.push(position - offset);
                Ok(Some(key))
            },
        )?;
        if entries.len() != changes.len() {
            return Err(package::codec_error(
                0,
                "expected every selected formula field to remain in its registration",
            ));
        }
        let mut xml = String::with_capacity(self.xml.len());
        let mut end = 0;
        for (entry, start) in entries.iter().zip(starts) {
            xml.push_str(&self.xml[end..start]);
            if let Some(after) = &changes[&entry.key] {
                xml.push_str(&after.xml);
            }
            end = start + entry.xml.len();
        }
        xml.push_str(&self.xml[end..]);
        let mut result = self.clone();
        result.xml = xml.into();
        Ok(result)
    }

    /// Resolve formula fields once at the existing namespace-aware intake.
    /// Standard thresholds state `val`; standard formulas state text; x14
    /// formula wrappers and thresholds put their text in a verified xm:f.
    fn formula_fields(&self, extended: bool, part: &str) -> Result<Vec<CarriedFormulaField>> {
        use super::carried::{X14_NAMESPACE, XM_NAMESPACE};
        let (bytes, _) = self.contextual();
        let mut attributes = Vec::new();
        let mut path: Vec<(SmolStr, bool)> = Vec::new();
        let entries = Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |reader, start, depth, position| {
                if depth == 0 {
                    return Ok(None);
                }
                path.truncate(depth - 1);
                let (namespace, local) = reader.resolver().resolve_element(start.name());
                let name = SmolStr::new(String::from_utf8_lossy(local.as_ref()));
                let main =
                    Self::in_namespace(namespace, &[super::NAMESPACE, super::STRICT_NAMESPACE])?;
                let x14 = Self::in_namespace(
                    reader.resolver().resolve_element(start.name()).0,
                    &[X14_NAMESPACE],
                )?;
                let xm = Self::in_namespace(
                    reader.resolver().resolve_element(start.name()).0,
                    &[XM_NAMESPACE],
                )?;
                // A leaf is formula-bearing only on its complete schema path.
                // Known names below an opaque extension do not acquire meaning
                // merely by rebinding the leaf to a standard namespace.
                let standard_path = path.iter().all(|(_, recognized)| *recognized);
                let root = path.first().map(|(name, _)| name.as_str());
                let threshold = root == Some("cfRule")
                    && path.len() >= 2
                    && matches!(path[1].0.as_str(), "colorScale" | "dataBar" | "iconSet");
                let formula = standard_path
                    && if extended {
                        xm && name == "f"
                            && ((path.len() == 1 && root == Some("cfRule"))
                                || (path.len() == 2
                                    && root == Some("dataValidation")
                                    && matches!(path[1].0.as_str(), "formula1" | "formula2"))
                                || (path.len() == 3 && threshold && path[2].0 == "cfvo"))
                    } else {
                        main && path.len() == 1
                            && ((root == Some("cfRule") && name == "formula")
                                || (root == Some("dataValidation")
                                    && matches!(name.as_str(), "formula1" | "formula2")))
                    };
                let attribute = !extended
                    && standard_path
                    && threshold
                    && path.len() == 2
                    && main
                    && name == "cfvo"
                    && Self::exact_attribute(start, b"type", position)?.as_deref()
                        == Some("formula");
                path.push((name, if extended { x14 } else { main }));
                if !formula && !attribute {
                    return Ok(None);
                }
                if attribute && Self::exact_attribute(start, b"val", position)?.is_none() {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{part}#cfvo"),
                        reason: "expected the formula threshold's val attribute".into(),
                    });
                }
                attributes.push(attribute);
                Ok(Some(format_smolstr!("{position}")))
            },
        )?;
        Ok(entries
            .into_iter()
            .zip(attributes)
            .map(|(registration, attribute)| CarriedFormulaField {
                registration,
                attribute,
            })
            .collect())
    }

    /// Split one complete standard/x14 CF host or validation. Each resulting
    /// fragment remains a complete Registration with its inherited scope.
    fn formula_pieces(
        &self,
        kind: shift::CarriedFormulaKind,
        extended: bool,
        owner: (&str, Option<&str>),
        shift: &Shift<'_>,
        part: &str,
    ) -> Result<Vec<Self>> {
        use super::carried::{X14_NAMESPACE, XM_NAMESPACE};
        let main = &[super::NAMESPACE, super::STRICT_NAMESPACE];
        let sqref = if extended {
            Some(self.one_child(&[XM_NAMESPACE], "sqref", part)?)
        } else {
            None
        };
        let text = if let Some(sqref) = &sqref {
            sqref.plain_text(part)?
        } else {
            self.root_attribute(b"sqref")?
                .ok_or_else(|| Error::InvalidRecord {
                    path: format_smolstr!("{part}#sqref"),
                    reason: "expected a carried formula's ranges".into(),
                })?
        };
        let ranges: Vec<CellRange> = text
            .split_whitespace()
            .map(|piece| {
                piece.parse().map_err(|_| Error::InvalidRecord {
                    path: format_smolstr!("{part}#sqref"),
                    reason: format_smolstr!("expected worksheet ranges, got {text:?}"),
                })
            })
            .collect::<Result<_>>()?;
        let Some(first) = ranges.first() else {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{part}#sqref"),
                reason: "expected at least one carried formula range".into(),
            });
        };
        let host = CellRef::new(
            ranges.iter().fold(first.start().row(), |min, range| {
                min.min(range.start().row())
            }),
            ranges.iter().fold(first.start().column(), |min, range| {
                min.min(range.start().column())
            }),
        );
        let conditional = kind == shift::CarriedFormulaKind::Conditional;
        let rules = if conditional {
            self.children_named(if extended { &[X14_NAMESPACE] } else { main }, "cfRule")?
        } else {
            vec![self.clone()]
        };
        let mut planned = Vec::with_capacity(rules.len());
        let mut unchanged = true;
        for rule in &rules {
            let fields = rule.formula_fields(extended, part)?;
            let formulas: Vec<_> = fields
                .iter()
                .map(|field| field.text(part).map(|text| Formula::from_file(&text, host)))
                .collect::<Result<_>>()?;
            let regions = shift.formula_regions(kind, owner, &ranges, &formulas, part)?;
            unchanged &= regions.len() == 1
                && regions[0].ranges == ranges
                && regions[0].formulas.as_slice() == formulas.as_slice();
            planned.push((rule, fields, regions));
        }
        // Resolve every rule before emitting XML: the ordinary unchanged case
        // retains its original complete registration without discarded splices.
        if unchanged {
            return Ok(vec![self.clone()]);
        }
        // Multiple rules share one parent template while preserving each
        // rule's lexical insertion point and the opaque children between them.
        // A single rule replaces its child directly in the original parent.
        let shared = conditional && rules.len() > 1;
        let mut positions: BTreeMap<_, _> = if shared {
            rules
                .iter()
                .map(|rule| (rule.key.clone(), 0usize))
                .collect()
        } else {
            BTreeMap::new()
        };
        let template = if shared {
            Some(self.patched_children(rules.len(), |key, at| {
                positions.get_mut(key).map(|position| {
                    *position = at;
                    &[][..]
                })
            })?)
        } else {
            None
        };
        let mut result = Vec::new();
        for (rule, fields, regions) in planned {
            for region in regions {
                let at = CellRef::new(
                    region
                        .ranges
                        .iter()
                        .map(|range| range.start().row())
                        .min()
                        .expect("a formula group has hosts"),
                    region
                        .ranges
                        .iter()
                        .map(|range| range.start().column())
                        .min()
                        .expect("a formula group has hosts"),
                );
                let mut changes = BTreeMap::new();
                for (field, formula) in fields.iter().zip(&region.formulas) {
                    changes.insert(
                        field.registration.key.clone(),
                        Some(field.with_formula(formula, at)?),
                    );
                }
                let rewritten_rule = rule.changed_entries(&changes)?;
                let mut rewritten = if let Some(template) = &template {
                    template
                        .inserted_at(std::slice::from_ref(&rewritten_rule), positions[&rule.key])?
                } else if conditional {
                    let mut changed = BTreeMap::new();
                    changed.insert(rule.key.clone(), Some(rewritten_rule));
                    self.changed_children(&changed)?
                } else {
                    rewritten_rule
                };
                let identical_ranges = region.ranges == ranges;
                let mut after = String::with_capacity(text.len());
                for range in region.ranges {
                    if !after.is_empty() {
                        after.push(' ');
                    }
                    range.write_a1(&mut after);
                }
                if identical_ranges {
                    after = text.clone();
                }
                if let Some(sqref) = &sqref {
                    // Rule edits can change lexical offsets before xm:sqref.
                    // Resolve that one direct child again in the new image.
                    let current = rewritten.one_child(&[XM_NAMESPACE], "sqref", part)?;
                    let mut changed = BTreeMap::new();
                    changed.insert(current.key.clone(), Some(sqref.with_text(after)?));
                    rewritten = rewritten.changed_children(&changed)?;
                } else {
                    rewritten = rewritten.with_attributes(vec![("sqref".into(), Some(after))])?;
                }
                result.push(rewritten);
            }
        }
        Ok(result)
    }
}

#[derive(Clone, Debug)]
struct Registration {
    key: SmolStr,
    xml: SmolStr,
    /// Inherited bindings travel with a registration when its container is removed.
    namespaces: BTreeMap<SmolStr, SmolStr>,
    inherited_markup: Arc<MarkupContext>,
    markup: Arc<MarkupContext>,
}

#[derive(Default)]
struct X14Transfer {
    priority_pairs: Vec<(u32, u32)>,
    selected_guid_owners: BTreeMap<crate::Uuid, SmolStr>,
    forks: BTreeMap<crate::Uuid, crate::Uuid>,
}

impl Registration {
    /// Direct children keep a lexical identity within this container. Selection
    /// and splicing share Registration's scoped parser; foreign children stay raw.
    fn children_named(&self, namespaces: &[&str], local: &str) -> Result<Vec<Self>> {
        self.selected_children(namespaces, local, None)
    }

    fn selected_children(
        &self,
        namespaces: &[&str],
        local: &str,
        strict_part: Option<&str>,
    ) -> Result<Vec<Self>> {
        let (bytes, _) = self.contextual();
        Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |reader, start, depth, position| {
                let (namespace, name) = reader.resolver().resolve_element(start.name());
                if depth != 2 {
                    return Ok(None);
                }
                let accepted =
                    name.as_ref() == local.as_bytes() && Self::in_namespace(namespace, namespaces)?;
                if !accepted {
                    if let Some(part) = strict_part {
                        return Err(Error::Unsupported {
                            operation: "cross-sheet transfer of an unproved x14 child",
                            filesystem: format_smolstr!(
                                "{part}#{}",
                                String::from_utf8_lossy(name.as_ref())
                            ),
                        });
                    }
                }
                Ok(accepted.then(|| format_smolstr!("{position}")))
            },
        )
    }

    /// Retain shared-item order and reject foreign children before using indexes.
    fn children_with_names(&self, namespaces: &[&str]) -> Result<Option<Vec<(SmolStr, Self)>>> {
        let (bytes, _) = self.contextual();
        let mut names = Vec::new();
        let mut foreign = false;
        let children = Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |reader, start, depth, position| {
                if depth != 2 {
                    return Ok(None);
                }
                let (namespace, local) = reader.resolver().resolve_element(start.name());
                if !Self::in_namespace(namespace, namespaces)? {
                    foreign = true;
                    return Ok(None);
                }
                let name = std::str::from_utf8(local.as_ref())
                    .map_err(|error| package::codec_error(position, error.to_string()))?;
                names.push(SmolStr::new(name));
                Ok(Some(format_smolstr!("{position}")))
            },
        )?;
        if foreign {
            return Ok(None);
        }
        if names.len() != children.len() {
            return Err(Error::InvalidRecord {
                path: self.key.clone(),
                reason: "shared item names and registrations disagree".into(),
            });
        }
        Ok(Some(names.into_iter().zip(children).collect()))
    }

    /// Resolve the registration's one unqualified worksheet reference.
    fn cell_range(&self, name: &str, part: &str) -> Result<CellRange> {
        let text = self
            .root_attribute(b"ref")?
            .ok_or_else(|| Error::InvalidRecord {
                path: format_smolstr!("{part}#{name}"),
                reason: "expected a worksheet range".into(),
            })?;
        text.parse().map_err(|_| Error::InvalidRecord {
            path: format_smolstr!("{part}#{name}"),
            reason: format_smolstr!("expected a worksheet range, got {text:?}"),
        })
    }

    /// Cut a worksheet filter/sort through its captured namespace context.
    /// Foreign same-local-name children and unchanged scopes stay byte-exact.
    fn cut_worksheet_range(
        &self,
        name: &str,
        range: CellRange,
        sheet: &str,
        shift: &Shift<'_>,
        part: &str,
    ) -> Result<Option<Vec<u8>>> {
        if name == "sortState" {
            shift.check_sort_cut(range, sheet, part)?;
            return Ok(None);
        }
        let moved = match shift.filter_move(range, sheet, part)? {
            shift::FilterMove::Keep => return Ok(None),
            shift::FilterMove::Drop => return Ok(Some(Vec::new())),
            shift::FilterMove::Move(moved) => moved,
        };
        let namespaces = &[super::NAMESPACE, super::STRICT_NAMESPACE];
        let mut changes = BTreeMap::new();
        for column in self.children_named(namespaces, "filterColumn")? {
            changes.insert(column.key, None);
        }
        let mut sorts = self.children_named(namespaces, "sortState")?;
        if sorts.len() > 1 {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{part}#autoFilter/sortState"),
                reason: "expected at most one filter sortState".into(),
            });
        }
        if let Some(sort) = sorts.pop() {
            let shifted_range = |entry: &Registration, child: &str| -> Result<CellRange> {
                let text = entry
                    .root_attribute(b"ref")?
                    .ok_or_else(|| Error::InvalidRecord {
                        path: format_smolstr!("{part}#{child}"),
                        reason: "expected a sort range".into(),
                    })?;
                let parsed: CellRange = text.parse().map_err(|_| Error::InvalidRecord {
                    path: format_smolstr!("{part}#{child}"),
                    reason: format_smolstr!("expected a sort range, got {text:?}"),
                })?;
                if !range.encloses(parsed) {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{part}#{child}"),
                        reason: format_smolstr!(
                            "expected a sort range inside {range}, got {parsed}"
                        ),
                    });
                }
                shift::adjust_range(parsed, sheet, shift).ok_or_else(|| Error::InvalidRecord {
                    path: format_smolstr!("{part}#{child}"),
                    reason: "expected the moved filter to retain its sort range".into(),
                })
            };
            let sort_range = shifted_range(&sort, "sortState")?;
            let mut conditions = BTreeMap::new();
            for condition in sort.children_named(namespaces, "sortCondition")? {
                let after = shifted_range(&condition, "sortCondition")?;
                conditions.insert(
                    condition.key.clone(),
                    Some(
                        condition.with_attributes(vec![(
                            "ref".into(),
                            Some(shift::range_text(after)),
                        )])?,
                    ),
                );
            }
            let changed = sort
                .changed_children(&conditions)?
                .with_attributes(vec![("ref".into(), Some(shift::range_text(sort_range)))])?;
            changes.insert(sort.key, Some(changed));
        }
        let changed = self
            .changed_children(&changes)?
            .with_attributes(vec![("ref".into(), Some(shift::range_text(moved)))])?;
        Ok(Some(changed.xml.as_bytes().to_vec()))
    }

    fn one_child(&self, namespaces: &[&str], local: &str, part: &str) -> Result<Self> {
        let mut children = self.children_named(namespaces, local)?;
        if children.len() != 1 {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{part}#{local}"),
                reason: format_smolstr!("expected one {local}, got {}", children.len()),
            });
        }
        Ok(children.remove(0))
    }

    fn is_element(&self, namespaces: &[&str], local: &str) -> Result<bool> {
        let (bytes, _) = self.contextual();
        Ok(!Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |reader, start, depth, _| {
                let (namespace, name) = reader.resolver().resolve_element(start.name());
                Ok((depth == 1
                    && name.as_ref() == local.as_bytes()
                    && Self::in_namespace(namespace, namespaces)?)
                .then(SmolStr::default))
            },
        )?
        .is_empty())
    }

    /// Replace selected direct children in place, preserving gaps and siblings.
    fn changed_children(&self, changes: &BTreeMap<SmolStr, Option<Self>>) -> Result<Self> {
        self.patched_children(changes.len(), |key, _| {
            changes.get(key).map(Option::as_slice)
        })
    }

    fn patched_children<'a>(
        &self,
        count: usize,
        mut replacement: impl FnMut(&SmolStr, usize) -> Option<&'a [Self]>,
    ) -> Result<Self> {
        if count == 0 {
            return Ok(self.clone());
        }
        let (bytes, offset) = self.contextual();
        let mut starts = Vec::new();
        let children = Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |_, _, depth, position| {
                if depth != 2 {
                    return Ok(None);
                }
                starts.push(position - offset);
                Ok(Some(format_smolstr!("{position}")))
            },
        )?;
        let mut seen = 0;
        let mut end = 0;
        let mut xml = String::with_capacity(self.xml.len());
        for (child, start) in children.iter().zip(starts) {
            // The output position includes the untouched gap before this child.
            let Some(after) = replacement(&child.key, xml.len() + start - end) else {
                continue;
            };
            seen += 1;
            xml.push_str(&self.xml[end..start]);
            for after in after {
                xml.push_str(&after.fragment(&self.namespaces, &self.markup)?);
            }
            end = start + child.xml.len();
        }
        if seen != count {
            return Err(package::codec_error(
                0,
                "expected every selected child to remain in its container",
            ));
        }
        xml.push_str(&self.xml[end..]);
        let mut result = self.clone();
        result.xml = xml.into();
        Ok(result)
    }

    fn plain_text(&self, part: &str) -> Result<String> {
        struct Text(String);
        impl package::Edits for Text {
            fn start(
                &mut self,
                path: &[SmolStr],
                _: &[(SmolStr, String)],
                _: quick_xml::name::ResolveResult<'_>,
            ) -> Result<package::Tag> {
                if path.len() != 1 {
                    return Err(package::codec_error(
                        0,
                        "expected a text-only metadata element",
                    ));
                }
                Ok(package::Tag::Keep)
            }
            fn text(&mut self, _: &[SmolStr], text: &str) -> Result<Option<String>> {
                self.0.push_str(text);
                Ok(None)
            }
        }
        let mut text = Text(String::new());
        package::edit_document(self.xml.as_bytes(), &mut text).map_err(|error| {
            Error::InvalidRecord {
                path: part.into(),
                reason: format_smolstr!("{error}"),
            }
        })?;
        Ok(text.0)
    }

    fn with_text(&self, text: String) -> Result<Self> {
        struct Text(Option<String>);
        impl package::Edits for Text {
            fn text(&mut self, path: &[SmolStr], _: &str) -> Result<Option<String>> {
                Ok((path.len() == 1).then(|| self.0.take()).flatten())
            }
        }
        let mut result = self.clone();
        if let Some(bytes) = package::edit_document(self.xml.as_bytes(), &mut Text(Some(text)))? {
            result.xml = std::str::from_utf8(&bytes)
                .map_err(|error| package::codec_error(0, error.to_string()))?
                .into();
        }
        Ok(result)
    }

    /// The relationship attribute's QName and value, resolved in captured scope.
    fn relationship_key(&self, part: &str) -> Result<(SmolStr, SmolStr)> {
        let (bytes, _) = self.contextual();
        let mut keys = Vec::new();
        Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |reader, start, depth, position| {
                if depth != 1 {
                    return Ok(None);
                }
                for attribute in start.attributes() {
                    let attribute = attribute
                        .map_err(|error| package::codec_error(position, error.to_string()))?;
                    let (namespace, name) = reader.resolver().resolve_attribute(attribute.key);
                    if name.as_ref() == b"id"
                        && Self::in_namespace(
                            namespace,
                            &[
                                super::RELATIONSHIPS_NAMESPACE,
                                super::STRICT_RELATIONSHIPS_NAMESPACE,
                            ],
                        )?
                    {
                        keys.push((
                            SmolStr::new(std::str::from_utf8(attribute.key.as_ref()).map_err(
                                |error| package::codec_error(position, error.to_string()),
                            )?),
                            SmolStr::new(
                                attribute
                                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                    .map_err(|error| {
                                        package::codec_error(position, error.to_string())
                                    })?,
                            ),
                        ));
                    }
                }
                Ok(Some(SmolStr::default()))
            },
        )?;
        if keys.len() != 1 || keys[0].1.is_empty() {
            return Err(Error::InvalidRecord {
                path: part.into(),
                reason: "expected one nonempty relationship id".into(),
            });
        }
        Ok(keys.remove(0))
    }
    /// One document root, captured with the same scope rules as its entries.
    /// Require the selected package part to have exactly this main-namespace
    /// root; Registration remains the sole XML capture/parser owner.
    fn root_named(bytes: &[u8], local: &str, part: &str) -> Result<Self> {
        let mut roots = Self::select(bytes, |reader, start, depth, position| {
            if depth != 0 {
                return Ok(None);
            }
            let (namespace, name) = reader.resolver().resolve_element(start.name());
            let main = name.as_ref() == local.as_bytes()
                && Self::in_namespace(
                    namespace,
                    &[super::NAMESPACE, super::STRICT_NAMESPACE],
                )?;
            if !main {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new(part),
                    reason: format_smolstr!("expected a {local} root in the main worksheet namespace"),
                });
            }
            Ok(Some(format_smolstr!("{position}")))
        })?;
        if roots.len() != 1 {
            return Err(Error::InvalidRecord {
                path: SmolStr::new(part),
                reason: format_smolstr!("expected exactly one {local} root, got {}", roots.len()),
            });
        }
        Ok(roots.remove(0))
    }

    fn root(bytes: &[u8]) -> Result<Self> {
        Self::root_at(bytes).map(|(_, root)| root)
    }

    /// Keep a root and its source offset paired; a second root is invalid
    /// input, never an offset for the first root's bytes.
    fn root_at(bytes: &[u8]) -> Result<(usize, Self)> {
        let mut at = None;
        let mut entries = Self::select(bytes, |_, _, depth, position| {
            if depth != 0 {
                return Ok(None);
            }
            if at.replace(position).is_some() {
                return Err(package::codec_error(position, "expected one metadata root"));
            }
            Ok(Some(SmolStr::default()))
        })?;
        match (at, entries.pop()) {
            (Some(at), Some(root)) => Ok((at, root)),
            _ => Err(package::codec_error(0, "expected a metadata root")),
        }
    }

    fn replace_root(bytes: &[u8], replacement: &str) -> Result<Vec<u8>> {
        let (at, root) = Self::root_at(bytes)?;
        let mut result = Vec::with_capacity(bytes.len() + replacement.len());
        result.extend_from_slice(&bytes[..at]);
        result.extend_from_slice(replacement.as_bytes());
        result.extend_from_slice(&bytes[at + root.xml.len()..]);
        Ok(result)
    }

    /// The original fragment in its captured scope; offsets into its bytes
    /// differ from those of the wrapper by the returned opening-tag size.
    fn contextual(&self) -> (Vec<u8>, usize) {
        let mut opening = String::from("<context");
        for (name, value) in &self.namespaces {
            opening.push_str(&format!(" {name}=\"{}\"", package::escape_attribute(value)));
        }
        opening.push('>');
        let offset = opening.len();
        let mut bytes = opening.into_bytes();
        bytes.extend_from_slice(self.xml.as_bytes());
        bytes.extend_from_slice(b"</context>");
        (bytes, offset)
    }

    /// An actual table or OPC relationship key, with its exact QName.
    fn member_keys(
        reader: &quick_xml::NsReader<&[u8]>,
        start: &quick_xml::events::BytesStart<'_>,
        table: bool,
        position: usize,
    ) -> Result<Vec<(SmolStr, SmolStr)>> {
        let (namespace, name) = reader.resolver().resolve_element(start.name());
        let recognized = if table {
            name.as_ref() == b"tablePart"
                && Self::in_namespace(namespace, &[super::NAMESPACE, super::STRICT_NAMESPACE])?
        } else {
            name.as_ref() == b"Relationship"
                && Self::in_namespace(namespace, &[package::PACKAGE_RELATIONSHIPS_NAMESPACE])?
        };
        if !recognized {
            return Ok(Vec::new());
        }
        let mut found = Vec::new();
        for attribute in start.attributes() {
            let attribute =
                attribute.map_err(|error| package::codec_error(position, error.to_string()))?;
            let matches = if table {
                let (namespace, name) = reader.resolver().resolve_attribute(attribute.key);
                name.as_ref() == b"id"
                    && Self::in_namespace(
                        namespace,
                        &[
                            super::RELATIONSHIPS_NAMESPACE,
                            super::STRICT_RELATIONSHIPS_NAMESPACE,
                        ],
                    )?
            } else {
                attribute.key.as_ref() == b"Id"
            };
            if matches {
                let key = std::str::from_utf8(attribute.key.as_ref())
                    .map_err(|error| package::codec_error(position, error.to_string()))?;
                let value = attribute
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .map_err(|error| package::codec_error(position, error.to_string()))?;
                found.push((key.into(), SmolStr::new(value)));
            }
        }
        Ok(found)
    }

    fn member_key(
        reader: &quick_xml::NsReader<&[u8]>,
        start: &quick_xml::events::BytesStart<'_>,
        table: bool,
        position: usize,
    ) -> Result<Option<(SmolStr, SmolStr)>> {
        let mut keys = Self::member_keys(reader, start, table, position)?;
        if keys.len() > 1 {
            return Err(package::codec_error(position, "expected one metadata key"));
        }
        Ok(keys.pop())
    }

    /// Direct registrations only; the shared parser captures their original
    /// bytes and inherited bindings, including alternate relationship prefixes.
    fn members(&self, table: bool) -> Result<Vec<Self>> {
        let (bytes, _) = self.contextual();
        Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |reader, start, depth, position| {
                if depth != 2 {
                    return Ok(None);
                }
                Ok(Self::member_key(reader, start, table, position)?.map(|(_, key)| key))
            },
        )
    }

    /// Delete selected direct registrations without rewriting their siblings,
    /// the container's envelope, or any unrelated same-local-name extension.
    fn without(&self, table: bool, keys: &BTreeSet<SmolStr>) -> Result<Self> {
        let (bytes, offset) = self.contextual();
        let mut starts = Vec::new();
        let removed = Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |reader, start, depth, position| {
                let key = if depth == 2 {
                    Self::member_key(reader, start, table, position)?.map(|(_, key)| key)
                } else {
                    None
                };
                let key = key.filter(|key| keys.contains(key));
                if key.is_some() {
                    starts.push(position - offset);
                }
                Ok(key)
            },
        )?;
        let mut xml = String::with_capacity(self.xml.len());
        let mut after = 0;
        for (entry, start) in removed.iter().zip(starts) {
            xml.push_str(&self.xml[after..start]);
            after = start + entry.xml.len();
        }
        xml.push_str(&self.xml[after..]);
        let mut result = self.clone();
        result.xml = xml.into();
        Ok(result)
    }

    /// Change only root attributes by exact QName; a vendor's `x:id` or
    /// nested `Id` must not be rewritten with the registration's own key.
    fn with_attributes(&self, attributes: Vec<(SmolStr, Option<String>)>) -> Result<Self> {
        struct Attributes(Vec<(SmolStr, Option<String>)>);
        impl package::Edits for Attributes {
            fn start(
                &mut self,
                path: &[SmolStr],
                _: &[(SmolStr, String)],
                _: quick_xml::name::ResolveResult<'_>,
            ) -> Result<package::Tag> {
                Ok(if path.len() == 1 {
                    package::Tag::Set(std::mem::take(&mut self.0))
                } else {
                    package::Tag::Keep
                })
            }
        }
        let mut result = self.clone();
        if let Some(bytes) =
            package::edit_document(self.xml.as_bytes(), &mut Attributes(attributes))?
        {
            result.xml = std::str::from_utf8(&bytes)
                .map_err(|error| package::codec_error(0, error.to_string()))?
                .into();
        }
        Ok(result)
    }

    fn with_table_key(&self, key: &str) -> Result<Self> {
        let (bytes, _) = self.contextual();
        let mut name = None;
        Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |reader, start, depth, position| {
                if depth == 1 {
                    name = Self::member_key(reader, start, true, position)?.map(|(name, _)| name);
                    return Ok(Some(SmolStr::default()));
                }
                Ok(None)
            },
        )?;
        let name =
            name.ok_or_else(|| package::codec_error(0, "expected a table relationship key"))?;
        let mut result = self.with_attributes(vec![(name, Some(key.to_owned()))])?;
        result.key = key.into();
        Ok(result)
    }

    /// Append fragments without prefix rewriting: each brings precisely the
    /// scope the destination does not already provide.
    fn appended(&self, entries: &[Self]) -> Result<Self> {
        use quick_xml::events::Event;

        if entries.is_empty() {
            return Ok(self.clone());
        }
        let mut added = String::new();
        for entry in entries {
            added.push_str(&entry.fragment(&self.namespaces, &self.markup)?);
        }
        let mut reader = super::styles::reader(self.xml.as_bytes());
        let event = reader
            .read_event()
            .map_err(|error| package::codec_error(0, error.to_string()))?;
        let mut xml = self.xml.to_string();
        match event {
            Event::Empty(start) => {
                let end = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
                let name = std::str::from_utf8(start.name().as_ref())
                    .map_err(|error| package::codec_error(0, error.to_string()))?
                    .to_owned();
                xml.replace_range(end - 2..end, &format!(">{added}</{name}>"));
            }
            Event::Start(_) => {
                let at = xml
                    .rfind("</")
                    .ok_or_else(|| package::codec_error(0, "expected the container end"))?;
                xml.insert_str(at, &added);
            }
            _ => return Err(package::codec_error(0, "expected a metadata container")),
        }
        let mut result = self.clone();
        result.xml = xml.into();
        Ok(result)
    }

    /// Insert complete fragments before one selected direct child, retaining
    /// its bytes and position. Containers with a trailing extLst use this to
    /// preserve schema order rather than moving or rebuilding the extension.
    fn inserted_before(&self, entries: &[Self], before: &str) -> Result<Self> {
        if entries.is_empty() {
            return Ok(self.clone());
        }
        let (bytes, offset) = self.contextual();
        let mut position = None;
        Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |_, _, depth, at| {
                if depth == 2 && format_smolstr!("{at}") == before {
                    position = Some(at - offset);
                }
                Ok(None)
            },
        )?;
        let position = position
            .ok_or_else(|| package::codec_error(0, "expected the selected direct child"))?;
        self.inserted_at(entries, position)
    }

    /// Insert at a lexical boundary proven by this registration's child splice
    /// pass. The retained template and its offsets must travel together.
    fn inserted_at(&self, entries: &[Self], position: usize) -> Result<Self> {
        let mut added = String::new();
        for entry in entries {
            added.push_str(&entry.fragment(&self.namespaces, &self.markup)?);
        }
        let mut result = self.clone();
        let mut xml = self.xml.to_string();
        xml.insert_str(position, &added);
        result.xml = xml.into();
        Ok(result)
    }

    /// Capture carried registrations in their root scope without copying
    /// unrelated worksheet children.
    fn carried_entries<'a>(
        frame: &super::carried::WorksheetFrame,
        wanted: &str,
        items: impl IntoIterator<Item = &'a super::carried::Carried>,
    ) -> Result<Vec<Self>> {
        let mut bytes = frame.root.to_vec();
        for item in items {
            bytes.extend_from_slice(&item.bytes);
        }
        bytes.extend_from_slice(format!("</{}>", frame.root_name).as_bytes());
        Self::select(&bytes, |reader, start, depth, _| {
            let (namespace, name) = reader.resolver().resolve_element(start.name());
            Ok((depth == 1
                && name.as_ref() == wanted.as_bytes()
                && Self::in_namespace(namespace, &[super::NAMESPACE, super::STRICT_NAMESPACE])?)
            .then(SmolStr::default))
        })
    }

    /// The selected carried child, with the frame's inherited scope.
    fn carried_child(frame: &super::carried::WorksheetFrame, wanted: &str) -> Result<Option<Self>> {
        let mut entries = Self::carried_entries(
            frame,
            wanted,
            frame.items.iter().filter(|item| item.name == wanted),
        )?;
        if entries.len() > 1 {
            return Err(package::codec_error(
                0,
                "expected at most one selected carried child",
            ));
        }
        Ok(entries.pop())
    }
    /// Capture one carried item without cloning the sheet's other children.
    fn carried_item(
        frame: &super::carried::WorksheetFrame,
        item: &super::carried::Carried,
    ) -> Result<Option<Self>> {
        let mut entries = Self::carried_entries(frame, &item.name, std::iter::once(item))?;
        Ok(entries.pop())
    }

    fn frame_root(frame: &super::carried::WorksheetFrame) -> Result<Self> {
        let mut bytes = frame.root.to_vec();
        bytes.extend_from_slice(format!("</{}>", frame.root_name).as_bytes());
        Self::root(&bytes)
    }

    /// Prove the complete payload only when it will cross worksheets. The
    /// direct owner-list intake separately prevents foreign hosts being lost
    /// while retained opaque descendants remain in their original worksheet.
    fn x14_transfer_only(&self, kind: super::carried::ShiftedExtension, part: &str) -> Result<()> {
        use super::carried::{ShiftedExtension, X14_NAMESPACE, XM_NAMESPACE};
        let (bytes, _) = self.contextual();
        let (container, child) = kind.container();
        let host = if kind == ShiftedExtension::Sparkline {
            "sparkline"
        } else {
            child
        };
        let mut path: Vec<(SmolStr, bool)> = Vec::new();
        Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |reader, start, depth, _| {
                if depth > 0 {
                    let (namespace, name) = reader.resolver().resolve_element(start.name());
                    if !Self::in_namespace(
                        namespace,
                        &[
                            super::NAMESPACE,
                            super::STRICT_NAMESPACE,
                            X14_NAMESPACE,
                            XM_NAMESPACE,
                        ],
                    )? {
                        return Err(Error::Unsupported {
                            operation: "cross-sheet transfer of foreign x14 markup",
                            filesystem: format_smolstr!(
                                "{part}#{}",
                                String::from_utf8_lossy(name.as_ref())
                            ),
                        });
                    }
                    let x14 = Self::in_namespace(
                        reader.resolver().resolve_element(start.name()).0,
                        &[X14_NAMESPACE],
                    )?;
                    let xm = Self::in_namespace(
                        reader.resolver().resolve_element(start.name()).0,
                        &[XM_NAMESPACE],
                    )?;
                    let local = SmolStr::new(String::from_utf8_lossy(name.as_ref()));
                    path.truncate(depth - 1);
                    let parent = path.last();
                    let accepted = if depth == 2 {
                        x14 && local == container
                    } else if depth == 3 {
                        x14 && local == child
                    } else if local == "sqref" {
                        xm && parent.is_some_and(|(name, x14)| *x14 && name == host)
                    } else if local == "f" {
                        xm && path.iter().any(|(name, x14)| *x14 && name == host)
                    } else if local == "sparkline" {
                        kind == ShiftedExtension::Sparkline
                            && x14
                            && parent.is_some_and(|(name, x14)| *x14 && name == "sparklines")
                            && path.iter().any(|(name, x14)| *x14 && name == child)
                    } else if local == "cfRule" {
                        kind == ShiftedExtension::ConditionalFormatting
                            && x14
                            && parent
                                .is_some_and(|(name, x14)| *x14 && name == "conditionalFormatting")
                    } else if matches!(local.as_str(), "conditionalFormatting" | "dataValidation") {
                        false // Another nested host has no proved worksheet owner.
                    } else {
                        true
                    };
                    if !accepted {
                        return Err(Error::Unsupported {
                            operation: "cross-sheet transfer of an unproved x14 child",
                            filesystem: format_smolstr!("{part}#{local}"),
                        });
                    }
                    path.push((local, x14));
                }
                Ok(None)
            },
        )?;
        Ok(())
    }

    /// A transfer edits known SpreadsheetML only. Unknown descendants must
    /// not be mistaken for local-name matches by SheetEdits.
    fn standard_transfer_only(&self, part: &str, linked_cf: bool) -> Result<()> {
        use super::carried::X14_NAMESPACE;
        let (bytes, _) = self.contextual();
        let mut path = Vec::<SmolStr>::new();
        let mut link_ext = false;
        Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |reader, start, depth, position| {
                if depth >= 1 {
                    path.truncate(depth - 1);
                    let (namespace, name) = reader.resolver().resolve_element(start.name());
                    let local = SmolStr::new(String::from_utf8_lossy(name.as_ref()));
                    let main = Self::in_namespace(
                        namespace,
                        &[super::NAMESPACE, super::STRICT_NAMESPACE],
                    )?;
                    if depth == 4 {
                        link_ext = false;
                        if linked_cf
                            && main
                            && local == "ext"
                            && path.iter().map(SmolStr::as_str).eq([
                                "conditionalFormatting",
                                "cfRule",
                                "extLst",
                            ])
                        {
                            link_ext = Self::exact_attribute(start, b"uri", position)?.is_some_and(
                                |uri| {
                                    uri.eq_ignore_ascii_case(
                                        "{B025F937-C7B1-47D3-B67F-A62EFF666E3E}",
                                    )
                                },
                            );
                        }
                    }
                    let linked_id = linked_cf
                        && depth == 5
                        && link_ext
                        && local == "id"
                        && path.iter().map(SmolStr::as_str).eq([
                            "conditionalFormatting",
                            "cfRule",
                            "extLst",
                            "ext",
                        ])
                        && Self::in_namespace(
                            reader.resolver().resolve_element(start.name()).0,
                            &[X14_NAMESPACE],
                        )?;
                    if !main && !linked_id {
                        return Err(Error::Unsupported {
                            operation: "cross-sheet transfer of foreign carried markup",
                            filesystem: format_smolstr!("{part}#{local}"),
                        });
                    }
                    path.push(local);
                }
                Ok(None)
            },
        )?;
        Ok(())
    }

    fn root_attribute(&self, name: &[u8]) -> Result<Option<String>> {
        self.attribute(name)
    }

    /// Container settings apply to every child. A merge is representable
    /// only when source and destination settings agree; count is recomputed.
    fn container_semantics(&self) -> Result<BTreeMap<(SmolStr, SmolStr), SmolStr>> {
        let mut reader = quick_xml::NsReader::from_reader(self.xml.as_bytes());
        let event = reader
            .read_event()
            .map_err(|error| package::codec_error(0, error.to_string()))?;
        let start = match event {
            quick_xml::events::Event::Start(start) | quick_xml::events::Event::Empty(start) => {
                start
            }
            _ => return Err(package::codec_error(0, "expected a metadata container")),
        };
        let counted = start.local_name().as_ref() == b"dataValidations";
        let mut settings = BTreeMap::new();
        for attribute in start.attributes() {
            let attribute =
                attribute.map_err(|error| package::codec_error(0, error.to_string()))?;
            let qualified = std::str::from_utf8(attribute.key.as_ref())
                .map_err(|error| package::codec_error(0, error.to_string()))?;
            if qualified == "xmlns" || qualified.starts_with("xmlns:") {
                continue;
            }
            let (namespace, local) = reader.resolver().resolve_attribute(attribute.key);
            let name = std::str::from_utf8(local.as_ref())
                .map_err(|error| package::codec_error(0, error.to_string()))?;
            let namespace = match namespace {
                quick_xml::name::ResolveResult::Unbound => SmolStr::default(),
                quick_xml::name::ResolveResult::Bound(namespace) => {
                    let value = std::str::from_utf8(namespace.as_ref())
                        .map_err(|error| package::codec_error(0, error.to_string()))?;
                    SmolStr::new(
                        quick_xml::escape::unescape(value)
                            .map_err(|error| package::codec_error(0, error.to_string()))?,
                    )
                }
                quick_xml::name::ResolveResult::Unknown(prefix) => {
                    // The fragment can inherit its binding from the captured
                    // worksheet scope; it need not repeat an xmlns attribute.
                    let prefix = std::str::from_utf8(&prefix)
                        .map_err(|error| package::codec_error(0, error.to_string()))?;
                    self.namespaces
                        .get(&format_smolstr!("xmlns:{prefix}"))
                        .filter(|value| !value.is_empty())
                        .cloned()
                        .ok_or_else(|| {
                            package::codec_error(
                                0,
                                format!("expected a namespace for container attribute {qualified}"),
                            )
                        })?
                }
            };
            if (counted && namespace.is_empty() && name == "count")
                || (namespace == MARKUP_NAMESPACE
                    && matches!(
                        name,
                        "Ignorable" | "ProcessContent" | "PreserveElements" | "PreserveAttributes"
                    ))
            {
                continue;
            }
            let value = attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|error| package::codec_error(0, error.to_string()))?;
            if settings
                .insert((namespace, SmolStr::new(name)), SmolStr::new(value))
                .is_some()
            {
                return Err(package::codec_error(
                    0,
                    format!("expected distinct expanded container attributes, got {qualified}"),
                ));
            }
        }
        Ok(settings)
    }

    /// Unlike table memberships, an ordinary hyperlink can carry only a
    /// location and have no relationship id.
    fn optional_relationship_key(&self, part: &str) -> Result<Option<(SmolStr, SmolStr)>> {
        let (bytes, _) = self.contextual();
        let mut keys = Vec::new();
        Self::select_in(
            &bytes,
            Arc::clone(&self.inherited_markup),
            |reader, start, depth, position| {
                if depth == 1 {
                    for attribute in start.attributes() {
                        let attribute = attribute
                            .map_err(|error| package::codec_error(position, error.to_string()))?;
                        let (namespace, name) = reader.resolver().resolve_attribute(attribute.key);
                        if name.as_ref() == b"id"
                            && Self::in_namespace(
                                namespace,
                                &[
                                    super::RELATIONSHIPS_NAMESPACE,
                                    super::STRICT_RELATIONSHIPS_NAMESPACE,
                                ],
                            )?
                        {
                            let qname =
                                std::str::from_utf8(attribute.key.as_ref()).map_err(|error| {
                                    package::codec_error(position, error.to_string())
                                })?;
                            let value = attribute
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .map_err(|error| {
                                    package::codec_error(position, error.to_string())
                                })?;
                            keys.push((SmolStr::new(qname), SmolStr::new(value)));
                        }
                    }
                }
                Ok(None)
            },
        )?;
        if keys.len() > 1 || keys.first().is_some_and(|(_, value)| value.is_empty()) {
            return Err(Error::InvalidRecord {
                path: part.into(),
                reason: "expected at most one nonempty relationship id".into(),
            });
        }
        Ok(keys.pop())
    }
    fn read(bytes: &[u8], element: &[u8], key: &[u8]) -> Result<Vec<Self>> {
        Self::select(bytes, |_, start, _, position| {
            if package::local_name(start.name().as_ref()) != element {
                return Ok(None);
            }
            package::attribute(start, key, position)?
                .map(|key| Some(SmolStr::new(key)))
                .ok_or_else(|| {
                    package::codec_error(position, "expected a metadata registration key")
                })
        })
    }

    /// Capture selected subtrees and their inherited namespace bindings once.
    fn select(
        bytes: &[u8],
        select: impl FnMut(
            &quick_xml::NsReader<&[u8]>,
            &quick_xml::events::BytesStart<'_>,
            usize,
            usize,
        ) -> Result<Option<SmolStr>>,
    ) -> Result<Vec<Self>> {
        Self::select_in(bytes, MarkupContext::empty(), select)
    }

    fn select_in(
        bytes: &[u8],
        base: Arc<MarkupContext>,
        mut select: impl FnMut(
            &quick_xml::NsReader<&[u8]>,
            &quick_xml::events::BytesStart<'_>,
            usize,
            usize,
        ) -> Result<Option<SmolStr>>,
    ) -> Result<Vec<Self>> {
        use quick_xml::events::Event;

        let mut reader = quick_xml::NsReader::from_reader(bytes);
        let mut buffer = Vec::new();
        let mut entries = Vec::new();
        let mut depth = 0_usize;
        let mut scopes: Vec<(usize, Arc<MarkupContext>)> = Vec::new();
        loop {
            let position = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
            let event = reader
                .read_event_into(&mut buffer)
                .map_err(|error| package::codec_error(position, error.to_string()))?;
            match event {
                Event::Start(ref start) | Event::Empty(ref start) => {
                    while scopes.last().is_some_and(|(at, _)| *at >= depth) {
                        scopes.pop();
                    }
                    let inherited_markup =
                        Arc::clone(scopes.last().map_or(&base, |(_, scope)| scope));
                    let markup = MarkupContext::at(&inherited_markup, &reader, start, position)?;
                    let Some(key) = select(&reader, start, depth, position)? else {
                        if matches!(event, Event::Start(_)) {
                            if !Arc::ptr_eq(&inherited_markup, &markup) {
                                scopes.push((depth, markup));
                            }
                            depth += 1;
                        }
                        buffer.clear();
                        continue;
                    };
                    let namespaces = Self::namespaces(&reader)?;
                    if matches!(event, Event::Start(_)) {
                        let opening =
                            SmolStr::new(std::str::from_utf8(start.name().as_ref()).map_err(
                                |error| package::codec_error(position, error.to_string()),
                            )?);
                        let mut depth = 1_usize;
                        while depth != 0 {
                            buffer.clear();
                            match reader.read_event_into(&mut buffer).map_err(|error| {
                                package::codec_error(position, error.to_string())
                            })? {
                                Event::Start(_) => depth += 1,
                                Event::End(_) => depth -= 1,
                                Event::Eof => {
                                    return Err(package::codec_error(
                                        usize::try_from(reader.buffer_position())
                                            .unwrap_or(usize::MAX),
                                        format!(
                                            "expected the end tag of <{opening}>, got the end of the part"
                                        ),
                                    ));
                                }
                                _ => {}
                            }
                        }
                    }
                    let end = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
                    let xml = bytes
                        .get(position..end)
                        .and_then(|bytes| std::str::from_utf8(bytes).ok())
                        .ok_or_else(|| {
                            package::codec_error(position, "expected a UTF-8 metadata registration")
                        })?;
                    entries.push(Self {
                        key,
                        xml: xml.into(),
                        namespaces,
                        inherited_markup,
                        markup,
                    });
                }
                Event::End(_) => {
                    depth = depth.saturating_sub(1);
                    while scopes.last().is_some_and(|(at, _)| *at >= depth) {
                        scopes.pop();
                    }
                }
                Event::Eof => break,
                _ => {}
            }
            buffer.clear();
        }
        Ok(entries)
    }

    /// Actual table memberships, read in the worksheet's inherited scope.
    fn table_parts(frame: &super::carried::WorksheetFrame, part: &str) -> Result<Vec<Self>> {
        let items = frame.items.iter().filter(|item| item.name == "tableParts");
        let size: usize = items.clone().map(|item| item.bytes.len()).sum();
        if size == 0 {
            return Ok(Vec::new());
        }
        // Only the root and memberships are needed; sheetData is never copied.
        let mut bytes = Vec::with_capacity(frame.root.len() + size + frame.root_name.len() + 3);
        bytes.extend_from_slice(&frame.root);
        for item in items {
            bytes.extend_from_slice(&item.bytes);
        }
        bytes.extend_from_slice(b"</");
        bytes.extend_from_slice(frame.root_name.as_bytes());
        bytes.push(b'>');
        let mut worksheet = false;
        let mut container = false;
        let mut ordinal = 0;
        Self::select(&bytes, |reader, start, depth, position| {
            let (namespace, name) = reader.resolver().resolve_element(start.name());
            let main = Self::in_namespace(namespace, &[super::NAMESPACE, super::STRICT_NAMESPACE])?;
            match depth {
                0 => worksheet = main && name.as_ref() == b"worksheet",
                1 => container = worksheet && main && name.as_ref() == b"tableParts",
                2 if container && main && name.as_ref() == b"tablePart" => {
                    ordinal += 1;
                    let refusal = |reason| Error::InvalidRecord {
                        path: format_smolstr!("{part}#tablePart[{ordinal}]"),
                        reason,
                    };
                    let mut keys = Self::member_keys(reader, start, true, position)?;
                    if keys.len() > 1 {
                        return Err(refusal(SmolStr::new_static(
                            "expected one relationship id, got duplicate qualified ids",
                        )));
                    }
                    let key = keys.pop().map(|(_, key)| key);
                    return key.filter(|key| !key.is_empty()).map(Some).ok_or_else(|| {
                        refusal(SmolStr::new_static(
                            "expected a relationship id, got a missing r:id",
                        ))
                    });
                }
                _ => {}
            }
            Ok(None)
        })
    }

    /// quick-xml exposes namespace declaration bytes before entity decoding.
    fn in_namespace(
        namespace: quick_xml::name::ResolveResult<'_>,
        expected: &[&str],
    ) -> Result<bool> {
        let quick_xml::name::ResolveResult::Bound(namespace) = namespace else {
            return Ok(false);
        };
        let value = std::str::from_utf8(namespace.as_ref())
            .map_err(|error| package::codec_error(0, error.to_string()))?;
        let value = quick_xml::escape::unescape(value)
            .map_err(|error| package::codec_error(0, error.to_string()))?;
        Ok(expected.contains(&value.as_ref()))
    }

    fn namespaces(reader: &quick_xml::NsReader<&[u8]>) -> Result<BTreeMap<SmolStr, SmolStr>> {
        use quick_xml::name::PrefixDeclaration;

        let mut namespaces = BTreeMap::new();
        // Absence of a default namespace is also meaningful when moving an element.
        namespaces.insert(SmolStr::new_static("xmlns"), SmolStr::default());
        for (prefix, namespace) in reader.resolver().bindings() {
            let name = match prefix {
                PrefixDeclaration::Default => SmolStr::new_static("xmlns"),
                PrefixDeclaration::Named(prefix) => format_smolstr!(
                    "xmlns:{}",
                    std::str::from_utf8(prefix)
                        .map_err(|error| package::codec_error(0, error.to_string()))?
                ),
            };
            // Decode the original delimiter-independent value once; insertion
            // escapes it for its own quotes without doubling existing entities.
            let value = std::str::from_utf8(namespace.as_ref())
                .map_err(|error| package::codec_error(0, error.to_string()))?;
            let value = quick_xml::escape::unescape(value)
                .map_err(|error| package::codec_error(0, error.to_string()))?;
            namespaces.insert(name, value.as_ref().into());
        }
        Ok(namespaces)
    }

    /// The existing container's scope, or the root's when a container is recreated.
    fn scope(
        bytes: &[u8],
        parent: &[u8],
    ) -> Result<(BTreeMap<SmolStr, SmolStr>, Arc<MarkupContext>)> {
        let entry = Self::select(bytes, |_, start, _, _| {
            Ok((start.local_name().as_ref() == parent).then(SmolStr::default))
        })?
        .into_iter()
        .next();
        let entry = match entry {
            Some(entry) => entry,
            None => Self::root(bytes)?,
        };
        Ok((entry.namespaces, entry.markup))
    }

    /// Close inherited 2015 MCE context with namespace-resolved directives.
    /// Authored non-2015 attributes remain byte-exact; inherited unknown
    /// semantics are refused, never flattened by joining prefix strings.
    fn fragment(
        &self,
        scope: &BTreeMap<SmolStr, SmolStr>,
        markup: &Arc<MarkupContext>,
    ) -> Result<String> {
        MarkupContext::check_destination(self, markup)?;
        let xml = self.fragment_namespaces(scope)?;
        let needs_ignorable = !self.inherited_markup.ignorable.is_subset(&markup.ignorable);
        let needs_process = !self.inherited_markup.process.is_subset(&markup.process);
        if !needs_ignorable && !needs_process {
            return Ok(xml);
        }
        let mut bindings = self.namespaces.clone();
        // Source bindings shadow destination bindings on this fragment.
        for (name, value) in scope {
            bindings
                .entry(name.clone())
                .or_insert_with(|| value.clone());
        }
        let mut attributes = Vec::new();
        let mut prefix = |uri: &str| -> String {
            if let Some((name, _)) = bindings
                .iter()
                .find(|(name, value)| name.starts_with("xmlns:") && value.as_str() == uri)
            {
                return name[6..].to_owned();
            }
            let mut index = 1_u32;
            loop {
                let name = format_smolstr!("xmlns:mce{index}");
                if !bindings.contains_key(&name) {
                    bindings.insert(name.clone(), uri.into());
                    attributes.push((name, Some(uri.to_owned())));
                    return format!("mce{index}");
                }
                index += 1;
            }
        };
        let mc = prefix(MARKUP_NAMESPACE);
        let ignorable = needs_ignorable.then(|| {
            self.markup
                .ignorable
                .iter()
                .map(|namespace| prefix(namespace))
                .collect::<Vec<_>>()
                .join(" ")
        });
        let process = needs_process.then(|| {
            self.markup
                .process
                .iter()
                .map(|(namespace, local)| format!("{}:{local}", prefix(namespace)))
                .collect::<Vec<_>>()
                .join(" ")
        });
        // Retain the QName of any authored directive so no duplicate expanded
        // attribute is introduced through an alternative MC namespace prefix.
        let (contextual, _) = self.contextual();
        let mut names = BTreeMap::new();
        Self::select_in(
            &contextual,
            Arc::clone(&self.inherited_markup),
            |reader, start, depth, position| {
                if depth == 1 {
                    for attribute in start.attributes() {
                        let attribute = attribute
                            .map_err(|error| package::codec_error(position, error.to_string()))?;
                        let (namespace, local) = reader.resolver().resolve_attribute(attribute.key);
                        if Self::in_namespace(namespace, &[MARKUP_NAMESPACE])? {
                            let local = std::str::from_utf8(local.as_ref()).map_err(|error| {
                                package::codec_error(position, error.to_string())
                            })?;
                            let name =
                                std::str::from_utf8(attribute.key.as_ref()).map_err(|error| {
                                    package::codec_error(position, error.to_string())
                                })?;
                            names.insert(local.to_owned(), SmolStr::new(name));
                        }
                    }
                    return Ok(Some(SmolStr::default()));
                }
                Ok(None)
            },
        )?;
        for (local, value) in [("Ignorable", ignorable), ("ProcessContent", process)] {
            if let Some(value) = value {
                attributes.push((
                    names
                        .remove(local)
                        .unwrap_or_else(|| format_smolstr!("{mc}:{local}")),
                    Some(value),
                ));
            }
        }
        let mut closed = self.clone();
        closed.xml = xml.into();
        Ok(closed.with_attributes(attributes)?.xml.to_string())
    }
    /// Keep original bytes, adding only bindings the destination no longer supplies.
    fn fragment_namespaces(&self, scope: &BTreeMap<SmolStr, SmolStr>) -> Result<String> {
        use quick_xml::events::Event;

        let mut reader = super::styles::reader(self.xml.as_bytes());
        let mut buffer = Vec::new();
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| package::codec_error(0, error.to_string()))?;
        let empty = matches!(event, Event::Empty(_));
        let start = match event {
            Event::Start(start) | Event::Empty(start) => start,
            _ => return Err(package::codec_error(0, "expected a metadata registration")),
        };
        let stated = start
            .attributes()
            .map(|attribute| {
                attribute
                    .map(|attribute| attribute.key.as_ref().to_vec())
                    .map_err(|error| package::codec_error(0, error.to_string()))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut declarations = String::new();
        for (name, value) in &self.namespaces {
            if scope.get(name) != Some(value)
                && !stated.iter().any(|key| key.as_slice() == name.as_bytes())
            {
                declarations.push_str(&format!(" {name}=\"{}\"", package::escape_attribute(value)));
            }
        }
        let end = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
        let at = end
            .checked_sub(if empty { 2 } else { 1 })
            .filter(|at| self.xml.is_char_boundary(*at))
            .ok_or_else(|| package::codec_error(0, "expected a metadata start tag"))?;
        let mut xml = self.xml.to_string();
        xml.insert_str(at, &declarations);
        Ok(xml)
    }

    fn exact_attribute(
        start: &quick_xml::events::BytesStart<'_>,
        name: &[u8],
        position: usize,
    ) -> Result<Option<String>> {
        package::exact_attribute(start, name, position)
            .map(|value| value.map(|text| text.into_owned()))
    }

    fn attribute(&self, name: &[u8]) -> Result<Option<String>> {
        use quick_xml::events::Event;

        let mut reader = super::styles::reader(self.xml.as_bytes());
        let mut buffer = Vec::new();
        match reader
            .read_event_into(&mut buffer)
            .map_err(|error| package::codec_error(0, error.to_string()))?
        {
            Event::Start(start) | Event::Empty(start) => Self::exact_attribute(&start, name, 0),
            _ => Err(package::codec_error(0, "expected a metadata registration")),
        }
    }

    /// Append only absent registrations; a reused key is an atomic refusal.
    fn merge(
        bytes: &[u8],
        part: &str,
        element: &[u8],
        key: &[u8],
        parent: &[u8],
        entries: &[Self],
    ) -> Result<Option<Arc<[u8]>>> {
        if entries.is_empty() {
            return Ok(None);
        }
        let current = Self::read(bytes, element, key)?;
        let scope = Self::scope(bytes, parent)?;
        let mut added = String::new();
        for entry in entries {
            if let Some(held) = current.iter().find(|held| held.key == entry.key) {
                if held.xml != entry.xml {
                    return Err(Error::Conflict {
                        expected: "the removed sheet's package registration",
                        actual: "a different registration with the same key",
                        path: format_smolstr!("{part} ({})", entry.key),
                    });
                }
            } else {
                added.push_str(&entry.fragment(&scope.0, &scope.1)?);
            }
        }
        if added.is_empty() {
            return Ok(None);
        }
        let has_parent = package::has_element(bytes, parent)?;
        let insert = if !has_parent && parent == b"pivotCaches" {
            vec![Insertion {
                fragment: format!("<pivotCaches>{added}</pivotCaches>"),
                parent: b"workbook",
                before_first_of: &AFTER_CALCULATION_PROPERTIES[3..],
            }]
        } else {
            Vec::new()
        };
        if !has_parent && insert.is_empty() {
            return Err(package::codec_error(
                0,
                format_smolstr!("expected the metadata container in {part}"),
            ));
        }
        package::rewrite(
            bytes,
            &package::Rewrite {
                before_end: &|name| (has_parent && name == parent).then(|| added.clone()),
                insert: &insert,
                ..package::Rewrite::default()
            },
        )
        .map(|bytes| Some(Arc::from(bytes)))
    }
}

struct NoteBundle {
    rels: SmolStr,
    bytes: Option<Arc<[u8]>>,
    document: Registration,
    entries: Vec<Registration>,
    comments: Option<(SmolStr, LegacyNotes)>,
    threads: Option<(SmolStr, ThreadedNotes)>,
    drawing: Option<(SmolStr, SmolStr)>,
    legacy: Option<Registration>,
}

/// A legacy comment part resolved once for one cut plan, retaining raw payloads.
struct LegacyNotes {
    member: SmolStr,
    bytes: Arc<[u8]>,
    root: Registration,
    namespace: &'static str,
    authors: Registration,
    author_entries: Vec<Registration>,
    author_names: Vec<String>,
    list: Registration,
    notes: Vec<LegacyNote>,
}

struct LegacyNote {
    at: CellRef,
    author: usize,
    entry: Registration,
}

impl LegacyNotes {
    fn read(member: SmolStr, bytes: Arc<[u8]>) -> Result<Self> {
        let root = Registration::root(&bytes)?;
        let namespace = if root.is_element(&[super::NAMESPACE], "comments")? {
            super::NAMESPACE
        } else if root.is_element(&[super::STRICT_NAMESPACE], "comments")? {
            super::STRICT_NAMESPACE
        } else {
            return Err(Error::InvalidRecord {
                path: member,
                reason: "expected a SpreadsheetML comments root".into(),
            });
        };
        let authors = root.one_child(&[namespace], "authors", &member)?;
        let author_entries = authors.children_named(&[namespace], "author")?;
        let author_names = author_entries
            .iter()
            .map(|entry| entry.plain_text(&member))
            .collect::<Result<Vec<_>>>()?;
        let list = root.one_child(&[namespace], "commentList", &member)?;
        let mut notes = Vec::new();
        let mut seen = BTreeSet::new();
        for entry in list.children_named(&[namespace], "comment")? {
            let reference = entry
                .attribute(b"ref")?
                .ok_or_else(|| Error::InvalidRecord {
                    path: member.clone(),
                    reason: "expected a comment ref".into(),
                })?;
            let at: CellRef =
                reference
                    .parse::<CellRef>()
                    .map_err(|error| Error::InvalidRecord {
                        path: format_smolstr!("{member}#{reference}"),
                        reason: format_smolstr!("{error}"),
                    })?;
            if !at.is_in_grid() || !seen.insert(at) {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{member}#{reference}"),
                    reason: "expected one comment per grid cell".into(),
                });
            }
            let author = entry
                .attribute(b"authorId")?
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|id| *id < author_names.len())
                .ok_or_else(|| Error::InvalidRecord {
                    path: format_smolstr!("{member}#{reference}"),
                    reason: "expected a valid comment author index".into(),
                })?;
            notes.push(LegacyNote { at, author, entry });
        }
        Ok(Self {
            member,
            bytes,
            root,
            namespace,
            authors,
            author_entries,
            author_names,
            list,
            notes,
        })
    }

    fn rewritten(
        &self,
        changes: &BTreeMap<SmolStr, Option<Registration>>,
        added: &[Registration],
        authors: &[Registration],
    ) -> Result<Arc<[u8]>> {
        let list = self.list.changed_children(changes)?.appended(added)?;
        let mut root_changes = BTreeMap::from([(self.list.key.clone(), Some(list))]);
        if !authors.is_empty() {
            root_changes.insert(
                self.authors.key.clone(),
                Some(self.authors.appended(authors)?),
            );
        }
        let root = self.root.changed_children(&root_changes)?;
        Ok(Registration::replace_root(&self.bytes, &root.xml)?.into())
    }
}

const THREADED_NAMESPACE: &str =
    "http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments";
const THREADED_RELATIONSHIP: &str =
    "http://schemas.microsoft.com/office/2017/10/relationships/threadedComment";
const PERSON_RELATIONSHIP: &str =
    "http://schemas.microsoft.com/office/2017/10/relationships/person";

/// One part's messages, resolved through parent links to their legacy placeholders.
/// Raw registrations retain optional attributes, text, mentions and MCE context.
struct ThreadedNotes {
    member: SmolStr,
    bytes: Arc<[u8]>,
    root: Registration,
    messages: Vec<ThreadedMessage>,
    people: BTreeMap<crate::Uuid, bool>,
}

struct ThreadedMessage {
    id: crate::Uuid,
    at: CellRef,
    has_ref: bool,
    entry: Registration,
}

impl ThreadedMessage {
    fn moved(&self, at: CellRef) -> Result<Registration> {
        // CT_ThreadedComment.ref is optional. The moved placeholder and parent
        // chain establish location; absence must not become an authored ref.
        if self.has_ref {
            self.entry
                .with_attributes(vec![("ref".into(), Some(at.to_string()))])
        } else {
            Ok(self.entry.clone())
        }
    }
}

impl ThreadedNotes {
    fn invalid(member: &str, reason: impl Into<SmolStr>) -> Error {
        Error::InvalidRecord {
            path: member.into(),
            reason: reason.into(),
        }
    }

    fn guid(value: &str, member: &str, name: &str) -> Result<crate::Uuid> {
        let bare = value
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
            .unwrap_or(value);
        // Uuid also accepts 16 arbitrary binary bytes; XML GUID intake must not
        // mistake sixteen malformed text characters for that binary spelling.
        if !matches!(bare.len(), 32 | 36) {
            return Err(Self::invalid(
                member,
                format_smolstr!("expected a GUID for {name}, got {value}"),
            ));
        }
        crate::Uuid::from_bytes(bare.as_bytes()).map_err(|_| {
            Self::invalid(
                member,
                format_smolstr!("expected a GUID for {name}, got {value}"),
            )
        })
    }

    fn required_guid(entry: &Registration, member: &str, name: &str) -> Result<crate::Uuid> {
        let value = entry
            .attribute(name.as_bytes())?
            .ok_or_else(|| Self::invalid(member, format_smolstr!("expected {name}")))?;
        Self::guid(&value, member, name)
    }

    fn read(member: SmolStr, bytes: Arc<[u8]>, legacy: Option<&LegacyNotes>) -> Result<Self> {
        let root = Registration::root(&bytes)?;
        if !root.is_element(&[THREADED_NAMESPACE], "ThreadedComments")? {
            return Err(Self::invalid(&member, "expected a ThreadedComments root"));
        }
        let entries = root.children_named(&[THREADED_NAMESPACE], "threadedComment")?;
        let mut ids = BTreeMap::new();
        let mut parents = Vec::with_capacity(entries.len());
        let mut identities = Vec::with_capacity(entries.len());
        let mut has_refs = Vec::with_capacity(entries.len());
        let mut people = BTreeMap::new();
        for (index, entry) in entries.iter().enumerate() {
            let id = Self::required_guid(entry, &member, "id")?;
            if ids.insert(id, index).is_some() {
                return Err(Self::invalid(
                    &member,
                    "expected unique threaded message IDs",
                ));
            }
            identities.push(id);
            parents.push(
                entry
                    .attribute(b"parentId")?
                    .map(|value| Self::guid(&value, &member, "parentId"))
                    .transpose()?,
            );
            let person = Self::required_guid(entry, &member, "personId")?;
            people.entry(person).or_insert(false);
            let reference = entry.attribute(b"ref")?;
            if let Some(reference) = &reference {
                if !reference.parse::<CellRef>().is_ok_and(|at| at.is_in_grid()) {
                    return Err(Self::invalid(
                        &member,
                        format_smolstr!("expected a grid ref, got {reference}"),
                    ));
                }
            }
            has_refs.push(reference.is_some());
            for mentions in entry.children_named(&[THREADED_NAMESPACE], "mentions")? {
                for mention in mentions.children_named(&[THREADED_NAMESPACE], "mention")? {
                    let person = Self::required_guid(&mention, &member, "mentionpersonId")?;
                    Self::required_guid(&mention, &member, "mentionId")?;
                    people.insert(person, true);
                }
            }
        }
        // Resolve every chain once without recursion: workbook data cannot
        // turn a long reply chain into call-stack growth or repeated walks.
        let mut roots = vec![None; entries.len()];
        let mut visiting = vec![false; entries.len()];
        let mut trail = Vec::new();
        for start in 0..entries.len() {
            if roots[start].is_some() {
                continue;
            }
            let mut index = start;
            let root = loop {
                if let Some(root) = roots[index] {
                    break root;
                }
                if visiting[index] {
                    return Err(Self::invalid(
                        &member,
                        "expected acyclic threaded parent links",
                    ));
                }
                visiting[index] = true;
                trail.push(index);
                let Some(parent) = parents[index] else {
                    break index;
                };
                index = *ids.get(&parent).ok_or_else(|| {
                    Self::invalid(&member, "expected every threaded parent ID to exist")
                })?;
            };
            for index in trail.drain(..) {
                visiting[index] = false;
                roots[index] = Some(root);
            }
        }
        let mut locations = BTreeMap::new();
        if let Some(legacy) = legacy {
            for note in &legacy.notes {
                let author = &legacy.author_names[note.author];
                let mut identity = None;
                for token in author.split("tc={").skip(1) {
                    let Some((guid, _)) = token.split_once('}') else {
                        return Err(Self::invalid(
                            &member,
                            "expected a closed tc={GUID} author link",
                        ));
                    };
                    let id = Self::guid(guid, &member, "legacy placeholder author")?;
                    if identity.is_some_and(|previous| previous != id) {
                        return Err(Self::invalid(
                            &member,
                            "expected an unambiguous threaded placeholder author",
                        ));
                    }
                    identity = Some(id);
                }
                let Some(id) = identity else {
                    continue;
                };
                if let Some(index) = ids.get(&id) {
                    if parents[*index].is_some() || locations.insert(*index, note.at).is_some() {
                        return Err(Self::invalid(
                            &member,
                            "expected one legacy placeholder per top-level threaded comment",
                        ));
                    }
                }
            }
        }
        let mut messages = Vec::with_capacity(entries.len());
        for (index, entry) in entries.into_iter().enumerate() {
            let root = roots[index].expect("every finite chain was resolved");
            let at = *locations.get(&root).ok_or_else(|| {
                Self::invalid(
                    &member,
                    "expected a legacy placeholder for the threaded root",
                )
            })?;
            messages.push(ThreadedMessage {
                id: identities[index],
                at,
                has_ref: has_refs[index],
                entry,
            });
        }
        Ok(Self {
            member,
            bytes,
            root,
            messages,
            people,
        })
    }

    fn rewritten(
        &self,
        changes: &BTreeMap<SmolStr, Option<Registration>>,
        added: &[Registration],
    ) -> Result<Arc<[u8]>> {
        if changes.is_empty() && added.is_empty() {
            return Ok(Arc::clone(&self.bytes));
        }
        let root = self.root.changed_children(changes)?;
        let tails = root.children_named(&[THREADED_NAMESPACE], "extLst")?;
        let root = match tails.first() {
            Some(tail) => root.inserted_before(added, &tail.key)?,
            None => root.appended(added)?,
        };
        Ok(Registration::replace_root(&self.bytes, &root.xml)?.into())
    }
}

pub(super) const VML_NAMESPACE: &str = "urn:schemas-microsoft-com:vml";
pub(super) const VML_EXCEL_NAMESPACE: &str = "urn:schemas-microsoft-com:office:excel";
const VML_OFFICE_NAMESPACE: &str = "urn:schemas-microsoft-com:office:office";

struct NoteShape {
    at: CellRef,
    entry: Registration,
    data: Registration,
    row: Registration,
    column: Registration,
    anchor: Registration,
    corners: [i64; 8],
    id: SmolStr,
    kind: SmolStr,
}

impl NoteShape {
    /// Change the VML shape's own identity and template link. Office's
    /// qualified spid, when present, is the same standard numeric identity;
    /// an unrelated or custom identity is never inferred from its spelling.
    fn rebound(
        &self,
        entry: &Registration,
        id: &str,
        kind: &str,
        part: &str,
    ) -> Result<Registration> {
        let mut attributes = Vec::new();
        if id != self.id.as_str() {
            attributes.push(("id".into(), Some(id.to_owned())));
            let (bytes, _) = entry.contextual();
            Registration::select_in(
                &bytes,
                Arc::clone(&entry.inherited_markup),
                |reader, start, depth, position| {
                    if depth != 1 {
                        return Ok(None);
                    }
                    for attribute in start.attributes() {
                        let attribute = attribute
                            .map_err(|error| package::codec_error(position, error.to_string()))?;
                        let (namespace, name) = reader.resolver().resolve_attribute(attribute.key);
                        if name.as_ref() == b"spid"
                            && Registration::in_namespace(namespace, &[VML_OFFICE_NAMESPACE])?
                        {
                            let value = attribute
                                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .map_err(|error| {
                                    package::codec_error(position, error.to_string())
                                })?;
                            if value.as_ref() != self.id.as_str() {
                                return Err(Error::unsupported(
                                    "remapping disagreeing VML shape identities",
                                    part,
                                ));
                            }
                            let key =
                                std::str::from_utf8(attribute.key.as_ref()).map_err(|error| {
                                    package::codec_error(position, error.to_string())
                                })?;
                            attributes.push((key.into(), Some(id.to_owned())));
                        }
                    }
                    Ok(Some(SmolStr::default()))
                },
            )?;
        }
        if kind != self.kind.as_str() {
            attributes.push(("type".into(), Some(format!("#{kind}"))));
        }
        if attributes.is_empty() {
            Ok(entry.clone())
        } else {
            entry.with_attributes(attributes)
        }
    }

    fn moved(&self, at: CellRef, part: &str) -> Result<Registration> {
        // Images/controls can carry part-local relationships and identities;
        // copying such a selected payload needs the subsequent graph slice.
        let (bytes, _) = self.entry.contextual();
        let mut dependent = false;
        Registration::select_in(
            &bytes,
            Arc::clone(&self.entry.inherited_markup),
            |reader, start, _, _| {
                let (namespace, name) = reader.resolver().resolve_element(start.name());
                if name.as_ref() == b"imagedata"
                    && Registration::in_namespace(namespace, &[VML_NAMESPACE])?
                {
                    dependent = true;
                }
                for attribute in start.attributes() {
                    let attribute =
                        attribute.map_err(|error| package::codec_error(0, error.to_string()))?;
                    let (namespace, name) = reader.resolver().resolve_attribute(attribute.key);
                    if Registration::in_namespace(
                        namespace,
                        &[
                            super::RELATIONSHIPS_NAMESPACE,
                            super::STRICT_RELATIONSHIPS_NAMESPACE,
                        ],
                    )? || (name.as_ref() == b"relid"
                        && Registration::in_namespace(
                            reader.resolver().resolve_attribute(attribute.key).0,
                            &[VML_OFFICE_NAMESPACE],
                        )?)
                    {
                        dependent = true;
                    }
                }
                Ok(None)
            },
        )?;
        if dependent {
            return Err(Error::unsupported(
                "moving a note shape with image or relationship payloads",
                part,
            ));
        }
        let mut corners = self.corners;
        let rows = i64::from(at.row()) - i64::from(self.at.row());
        let columns = i64::from(at.column()) - i64::from(self.at.column());
        for index in [0, 4] {
            corners[index] = corners[index].checked_add(columns).ok_or_else(|| {
                Error::unsupported("moving a VML anchor past its integer bounds", part)
            })?;
        }
        for index in [2, 6] {
            corners[index] = corners[index].checked_add(rows).ok_or_else(|| {
                Error::unsupported("moving a VML anchor past its integer bounds", part)
            })?;
        }
        if [0, 4]
            .iter()
            .any(|index| !(0..i64::from(MAX_COLUMNS)).contains(&corners[*index]))
            || [2, 6]
                .iter()
                .any(|index| !(0..i64::from(MAX_ROWS)).contains(&corners[*index]))
        {
            return Err(Error::unsupported(
                "clipping a moved VML note box at the grid edge",
                part,
            ));
        }
        let data = self.data.changed_children(&BTreeMap::from([
            (
                self.row.key.clone(),
                Some(self.row.with_text(at.row().to_string())?),
            ),
            (
                self.column.key.clone(),
                Some(self.column.with_text(at.column().to_string())?),
            ),
            (
                self.anchor.key.clone(),
                Some(
                    self.anchor.with_text(
                        corners
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", "),
                    )?,
                ),
            ),
        ]))?;
        self.entry
            .changed_children(&BTreeMap::from([(self.data.key.clone(), Some(data))]))
    }
}

struct NoteDrawing {
    member: SmolStr,
    bytes: Arc<[u8]>,
    root: Registration,
    notes: Vec<NoteShape>,
    /// The same intake owns each direct shape identity and its lexical child key.
    shape_keys: BTreeMap<SmolStr, SmolStr>,
    grouped: bool,
    /// Root payload outside direct shapes/type/layout metadata may still render.
    /// It remains owned even after the final modeled note disappears.
    other_payload: bool,
    types: BTreeMap<SmolStr, Registration>,
    layout: Option<Registration>,
    idmap: Option<Registration>,
}

impl NoteDrawing {
    fn read(member: SmolStr, bytes: Arc<[u8]>) -> Result<Self> {
        let root = Registration::root(&bytes)?;
        // Capture every direct child once with its namespace and inherited MCE
        // context. The same owned fragments feed classification and note intake.
        enum Child {
            Type,
            Shape,
            Layout,
            Other,
        }
        let mut kinds = Vec::new();
        let mut grouped = false;
        let mut other_payload = false;
        let (contextual, _) = root.contextual();
        let children = Registration::select_in(
            &contextual,
            Arc::clone(&root.inherited_markup),
            |reader, start, depth, position| {
                if depth != 2 {
                    return Ok(None);
                }
                let (namespace, name) = reader.resolver().resolve_element(start.name());
                let kind = match name.as_ref() {
                    b"shape" => {
                        if Registration::in_namespace(namespace, &[VML_NAMESPACE])? {
                            Child::Shape
                        } else {
                            Child::Other
                        }
                    }
                    b"shapetype" => {
                        if Registration::in_namespace(namespace, &[VML_NAMESPACE])? {
                            Child::Type
                        } else {
                            Child::Other
                        }
                    }
                    b"shapelayout" => {
                        if Registration::in_namespace(namespace, &[VML_OFFICE_NAMESPACE])? {
                            Child::Layout
                        } else {
                            Child::Other
                        }
                    }
                    b"group" => {
                        grouped |= Registration::in_namespace(namespace, &[VML_NAMESPACE])?;
                        Child::Other
                    }
                    _ => Child::Other,
                };
                other_payload |= matches!(kind, Child::Other);
                kinds.push(kind);
                Ok(Some(format_smolstr!("{position}")))
            },
        )?;
        let mut types = BTreeMap::new();
        let mut shapes = Vec::new();
        let mut layout = None;
        for (entry, kind) in children.into_iter().zip(kinds) {
            match kind {
                Child::Type => {
                    let id: SmolStr = entry
                        .attribute(b"id")?
                        .filter(|id| !id.is_empty())
                        .ok_or_else(|| Error::InvalidRecord {
                            path: member.clone(),
                            reason: "expected a VML type id".into(),
                        })?
                        .into();
                    if types.insert(id, entry).is_some() {
                        return Err(Error::InvalidRecord {
                            path: member,
                            reason: "expected unique VML type ids".into(),
                        });
                    }
                }
                Child::Shape => shapes.push(entry),
                Child::Layout => {
                    if layout.replace(entry).is_some() {
                        return Err(Error::InvalidRecord {
                            path: member,
                            reason: "expected at most one VML shape layout".into(),
                        });
                    }
                }
                Child::Other => {}
            }
        }
        let mut shape_keys = BTreeMap::new();
        let mut owners = BTreeSet::new();
        let mut notes = Vec::new();
        for entry in shapes {
            let id: SmolStr = entry
                .attribute(b"id")?
                .filter(|id| !id.is_empty())
                .ok_or_else(|| Error::InvalidRecord {
                    path: member.clone(),
                    reason: "expected a VML shape id".into(),
                })?
                .into();
            if shape_keys.insert(id.clone(), entry.key.clone()).is_some() {
                return Err(Error::InvalidRecord {
                    path: member,
                    reason: "expected unique VML shape ids".into(),
                });
            }
            let data = entry.children_named(&[VML_EXCEL_NAMESPACE], "ClientData")?;
            let mut note = false;
            for data in &data {
                note |= data.attribute(b"ObjectType")?.as_deref() == Some("Note");
            }
            if !note {
                continue;
            }
            if data.len() != 1 {
                return Err(Error::InvalidRecord {
                    path: member,
                    reason: "expected one ClientData for a VML note".into(),
                });
            }
            let data = data.into_iter().next().expect("one ClientData");
            let kind: SmolStr = entry
                .attribute(b"type")?
                .and_then(|value| value.strip_prefix('#').map(SmolStr::new))
                .filter(|kind| types.contains_key(kind))
                .ok_or_else(|| Error::InvalidRecord {
                    path: member.clone(),
                    reason: "expected a VML note's type to resolve".into(),
                })?;
            let row = data.one_child(&[VML_EXCEL_NAMESPACE], "Row", &member)?;
            let column = data.one_child(&[VML_EXCEL_NAMESPACE], "Column", &member)?;
            let anchor = data.one_child(&[VML_EXCEL_NAMESPACE], "Anchor", &member)?;
            let number = |entry: &Registration, limit: u32| -> Result<u32> {
                entry
                    .plain_text(&member)?
                    .trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|value| *value < limit)
                    .ok_or_else(|| Error::InvalidRecord {
                        path: member.clone(),
                        reason: "expected a VML note owner within the grid".into(),
                    })
            };
            let at = CellRef::new(number(&row, MAX_ROWS)?, number(&column, MAX_COLUMNS)?);
            if !owners.insert(at) {
                return Err(Error::InvalidRecord {
                    path: member,
                    reason: "expected one VML shape per note cell".into(),
                });
            }
            let values: Vec<i64> = anchor
                .plain_text(&member)?
                .split(',')
                .map(|value| value.trim().parse::<i64>())
                .collect::<std::result::Result<_, _>>()
                .map_err(|error| Error::InvalidRecord {
                    path: member.clone(),
                    reason: format_smolstr!("expected eight VML anchor integers: {error}"),
                })?;
            let corners: [i64; 8] = values.try_into().map_err(|_| Error::InvalidRecord {
                path: member.clone(),
                reason: "expected eight VML anchor integers".into(),
            })?;
            notes.push(NoteShape {
                at,
                entry,
                data,
                row,
                column,
                anchor,
                corners,
                id,
                kind,
            });
        }
        let idmap = match &layout {
            Some(layout) => {
                let mut maps = layout.children_named(&[VML_OFFICE_NAMESPACE], "idmap")?;
                if maps.len() > 1 {
                    return Err(Error::InvalidRecord {
                        path: member,
                        reason: "expected at most one VML shape id map".into(),
                    });
                }
                maps.pop()
            }
            None => None,
        };
        Ok(Self {
            member,
            bytes,
            root,
            notes,
            shape_keys,
            grouped,
            other_payload,
            types,
            layout,
            idmap,
        })
    }

    fn rewritten(
        &self,
        changes: &BTreeMap<SmolStr, Option<Registration>>,
        added: &[Registration],
    ) -> Result<Arc<[u8]>> {
        let root = self.root.changed_children(changes)?.appended(added)?;
        Ok(Registration::replace_root(&self.bytes, &root.xml)?.into())
    }

    /// Preserve everything outside the selected note shapes. An empty simple
    /// note drawing has no remaining worksheet-visible payload to own it.
    fn remaining(&self, removed: &BTreeSet<CellRef>) -> Result<Option<Arc<[u8]>>> {
        let changes: BTreeMap<_, _> = self
            .notes
            .iter()
            .filter(|shape| removed.contains(&shape.at))
            .map(|shape| (shape.entry.key.clone(), None))
            .collect();
        if changes.len() == self.shape_keys.len() && !self.other_payload {
            return Ok(None);
        }
        self.rewritten(&changes, &[]).map(Some)
    }

    /// Split a simple note drawing while retaining its original type and
    /// layout declarations. Shapes outside the partition never travel with it.
    fn partition(&self, moved: &[(&NoteShape, Registration)]) -> Result<Arc<[u8]>> {
        if self.grouped {
            return Err(Error::unsupported(
                "splitting grouped VML note shapes",
                &self.member,
            ));
        }
        if self.other_payload {
            return Err(Error::unsupported(
                "splitting a VML root with an unmodeled child",
                &self.member,
            ));
        }
        let mut changes = BTreeMap::new();
        for key in self.shape_keys.values() {
            let after = moved
                .iter()
                .find(|(old, _)| old.entry.key == *key)
                .map(|(_, moved)| moved.clone());
            changes.insert(key.clone(), after);
        }
        self.rewritten(&changes, &[])
    }
    /// A numeric Office identity can be allocated without interpreting any
    /// custom identifier. Existing noncolliding identities remain verbatim.
    fn reserve_id(
        &self,
        used: &mut BTreeSet<SmolStr>,
        wanted: &SmolStr,
        prefix: &str,
    ) -> Result<(SmolStr, Option<u32>)> {
        if used.insert(wanted.clone()) {
            return Ok((wanted.clone(), None));
        }
        if wanted
            .strip_prefix(prefix)
            .and_then(|digits| digits.parse::<u32>().ok())
            .is_none()
        {
            return Err(Error::unsupported(
                "merging colliding custom VML identities",
                format!("{}#{wanted}", self.member),
            ));
        }
        let highest = used
            .iter()
            .filter_map(|id| {
                id.strip_prefix(prefix)
                    .and_then(|digits| digits.parse::<u32>().ok())
            })
            .max();
        let number = highest
            .map_or(Some(1), |number| number.checked_add(1))
            .or_else(|| {
                (1..=u32::MAX).find(|number| !used.contains(&format_smolstr!("{prefix}{number}")))
            })
            .ok_or_else(|| {
                Error::unsupported("allocating exhausted numeric VML identities", &self.member)
            })?;
        let id = format_smolstr!("{prefix}{number}");
        used.insert(id.clone());
        Ok((id, Some(number)))
    }

    fn merged(
        &self,
        source: &Self,
        moved: &[(&NoteShape, Registration)],
        removed: &BTreeSet<CellRef>,
    ) -> Result<Arc<[u8]>> {
        if self.other_payload || source.other_payload {
            return Err(Error::unsupported(
                "merging note identities with unmodeled VML root payload",
                &self.member,
            ));
        }
        let removed_ids: BTreeSet<_> = self
            .notes
            .iter()
            .filter(|shape| removed.contains(&shape.at))
            .map(|shape| shape.id.clone())
            .collect();
        let mut shape_ids: BTreeSet<_> = self
            .shape_keys
            .keys()
            .filter(|id| !removed_ids.contains(*id))
            .cloned()
            .collect();
        let mut type_ids: BTreeSet<_> = self.types.keys().cloned().collect();
        let mut bound_types: BTreeMap<SmolStr, SmolStr> = BTreeMap::new();
        let mut new_types: BTreeMap<SmolStr, usize> = BTreeMap::new();
        let mut added: Vec<Registration> = Vec::new();
        let mut shapes = Vec::new();
        let mut blocks = BTreeSet::new();
        let mut changes: BTreeMap<_, _> = self
            .notes
            .iter()
            .filter(|shape| removed.contains(&shape.at))
            .map(|shape| (shape.entry.key.clone(), None))
            .collect();
        for (shape, entry) in moved {
            let kind = if let Some(kind) = bound_types.get(&shape.kind) {
                kind.clone()
            } else {
                let source_type = &source.types[&shape.kind];
                let held = self
                    .types
                    .get(&shape.kind)
                    .or_else(|| new_types.get(&shape.kind).map(|index| &added[*index]));
                let identical = held.is_some_and(|held| {
                    held.xml == source_type.xml
                        && held.namespaces == source_type.namespaces
                        && held.inherited_markup == source_type.inherited_markup
                });
                let kind = if identical {
                    shape.kind.clone()
                } else {
                    let (kind, _) = self.reserve_id(&mut type_ids, &shape.kind, "_x0000_t")?;
                    let copied = if kind == shape.kind {
                        source_type.clone()
                    } else {
                        source_type.with_attributes(vec![("id".into(), Some(kind.to_string()))])?
                    };
                    new_types.insert(kind.clone(), added.len());
                    added.push(copied);
                    kind
                };
                bound_types.insert(shape.kind.clone(), kind.clone());
                kind
            };
            let (id, number) = self.reserve_id(&mut shape_ids, &shape.id, "_x0000_s")?;
            if let Some(number) = number {
                // MS-OE376 2.1.1731: block zero covers 0..=1024, then
                // successive blocks cover 1024 positive shape identifiers.
                blocks.insert(number.saturating_sub(1) / 1024);
            }
            shapes.push(shape.rebound(entry, &id, &kind, &self.member)?);
        }
        added.extend(shapes);
        match (&self.idmap, &source.idmap) {
            (Some(current), Some(source_map)) => {
                let indices = |entry: &Registration| -> Result<Vec<u32>> {
                    entry
                        .attribute(b"data")?
                        .unwrap_or_default()
                        .split(',')
                        .filter(|value| !value.trim().is_empty())
                        .map(|value| {
                            value.trim().parse::<u32>().map_err(|_| {
                                Error::unsupported(
                                    "merging a custom VML shape id map",
                                    &self.member,
                                )
                            })
                        })
                        .collect()
                };
                let mut combined = indices(current)?;
                for index in indices(source_map)?.into_iter().chain(blocks) {
                    if !combined.contains(&index) {
                        combined.push(index);
                    }
                }
                // Office documents these as reserved shape-ID blocks; it ignores
                // them on read and regenerates them on save (MS-OE376 2.1.1731).
                let idmap = current.with_attributes(vec![(
                    "data".into(),
                    Some(
                        combined
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(","),
                    ),
                )])?;
                let layout = self.layout.as_ref().expect("the id map has a layout");
                changes.insert(
                    layout.key.clone(),
                    Some(
                        layout.changed_children(&BTreeMap::from([(
                            current.key.clone(),
                            Some(idmap),
                        )]))?,
                    ),
                );
            }
            (None, None) => {}
            _ => {
                return Err(Error::unsupported(
                    "merging differing VML shape id map layouts",
                    &self.member,
                ));
            }
        }
        self.rewritten(&changes, &added)
    }
}

/// Inbound package edges, excluding the workbook's non-owning cache list.
struct PartGraph {
    sources: HashMap<SmolStr, Vec<SmolStr>>,
}

impl PartGraph {
    /// Replace one prospective owner's edges in the graph already read for
    /// this mutation; no second inventory or relationship parser is needed.
    fn replace_source(&mut self, source: &str, relationships: &Relationships, workbook: &str) {
        for owners in self.sources.values_mut() {
            owners.retain(|owner| owner != source);
        }
        for relationship in relationships.entries() {
            if source == workbook && relationship.kind == RelationshipKind::PivotCacheDefinition {
                continue;
            }
            if let Some(target) = &relationship.target {
                self.sources
                    .entry(target.clone())
                    .or_default()
                    .push(source.into());
            }
        }
    }
    fn dropped(&self, removed: &[SmolStr]) -> BTreeSet<SmolStr> {
        let mut dropped: BTreeSet<SmolStr> = removed.iter().cloned().collect();
        loop {
            let mut grew = false;
            for (target, from) in &self.sources {
                if !dropped.contains(target)
                    && !from.is_empty()
                    && from.iter().all(|source| dropped.contains(source))
                {
                    dropped.insert(target.clone());
                    grew = true;
                }
            }
            if !grew {
                return dropped;
            }
        }
    }
}

/// What a workbook's package documents and untouched members come from.
#[derive(Debug)]
enum Source {
    /// The package it was opened over, or last saved as.
    Archive(Arc<ZipArchive>),
    /// A workbook built from nothing: the four template documents
    /// ([`package::TEMPLATE`]) and no other member.
    Template,
}

impl Source {
    /// Whether the indexed source has a member, without inflating it.
    fn contains(&self, name: &str) -> Result<bool> {
        match self {
            Self::Archive(archive) => Ok(archive.get_entry(name)?.is_some()),
            Self::Template => Ok(package::TEMPLATE.iter().any(|(member, _)| *member == name)),
        }
    }

    /// The members, directories left out, in the order the package lists
    /// them.
    fn names(&self) -> Result<Vec<SmolStr>> {
        match self {
            Self::Archive(archive) => Ok(archive
                .entries()?
                .iter()
                .filter(|entry| !entry.is_directory())
                .map(|entry| SmolStr::new(entry.name()))
                .collect()),
            Self::Template => Ok(package::TEMPLATE
                .iter()
                .map(|(name, _)| SmolStr::new_static(name))
                .collect()),
        }
    }

    /// The template document `name`.
    fn template(name: &str) -> Result<&'static str> {
        package::TEMPLATE
            .iter()
            .find(|(held, _)| *held == name)
            .map(|(_, text)| *text)
            .ok_or_else(|| Error::absent("workbook part", name))
    }

    /// The member `name`, whole.
    fn read(&self, name: &str) -> Result<Vec<u8>> {
        match self {
            Self::Archive(archive) => archive.read_member(name),
            Self::Template => Ok(Self::template(name)?.as_bytes().to_vec()),
        }
    }

    /// The member `name`, streamed.
    fn reader(&self, name: &str) -> Result<Box<dyn Read + Send>> {
        match self {
            Self::Archive(archive) => archive
                .member_reader(name)?
                .ok_or_else(|| Error::absent("workbook part", name)),
            Self::Template => Ok(Box::new(std::io::Cursor::new(
                Self::template(name)?.as_bytes(),
            ))),
        }
    }

    /// Put the member `name` in `target` as it is stored: its compressed
    /// bytes copied, never inflated.
    fn copy_into(&self, target: &ZipArchive, name: &str) -> Result<()> {
        match self {
            Self::Archive(archive) => target.copy_member_from(archive, name).map(|_| ()),
            Self::Template => target
                .write_member_with(name, Self::template(name)?.as_bytes(), Codec::Deflate)
                .map(|_| ()),
        }
    }

    fn handle_reads(&self) -> u64 {
        match self {
            Self::Archive(archive) => archive.handle_reads(),
            Self::Template => 0,
        }
    }
}

/// Where each workbook takes its identity from.
static WORKBOOKS: AtomicU64 = AtomicU64::new(1);

/// Where each package takes its place in the order packages are built.
static PACKAGES: AtomicU64 = AtomicU64::new(1);

/// A saved package: its bytes, and which state of which workbook they hold.
///
/// [`Workbook::into_package`] builds one; [`Workbook::rebase`] tells the
/// workbook a caller wrote it, and [`Workbook::write_into`] does both.
pub struct Package {
    bytes: Vec<u8>,
    snapshot: Snapshot,
}

impl Package {
    /// The package's bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The package's bytes, owned.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl std::fmt::Debug for Package {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Package")
            .field("bytes", &self.bytes.len())
            .field("sheets", &self.snapshot.slots.len())
            .finish_non_exhaustive()
    }
}

/// Which state of a workbook a package holds.
#[derive(Debug)]
struct Snapshot {
    workbook: u64,
    /// Where the package stands among every package built: one built later
    /// holds a later state.
    sequence: u64,
    slots: Vec<Written>,
    strings_part: Option<SmolStr>,
    styles_part: Option<SmolStr>,
    /// The package's style meanings, including entries generated by its writer.
    /// A clean save still retains a loaded table: another in-flight snapshot
    /// can rearrange its mutable suffix before this one is adopted.
    styles: Option<Arc<StyleSheet>>,
    documents: u64,
    system: DateSystem,
    /// The exact overlay insertions this save handles, including parts
    /// intentionally dropped because their last owner was removed.
    overrides: Vec<(SmolStr, u128)>,
}

/// One sheet as a package holds it.
#[derive(Debug)]
struct Written {
    key: SheetKey,
    part: SmolStr,
    /// The revision of the parsed sheet the part was written from, `None`
    /// for a sheet never parsed or written from a stream.
    revision: Option<u64>,
    state: SheetState,
}

/// A workbook: its sheets, reachable by name, over the package they came
/// from or built from nothing.
///
/// Held state: the sheet list and the resolved part names from open, one
/// parsed [`Sheet`] per sheet asked for, and the shared strings and styles
/// once a sheet needed them - all until drop, or until a save adopts the
/// package it wrote, which drops the strings and styles read to be read
/// again from it.
///
/// ```
/// use yggdryl::excel::Workbook;
/// use yggdryl::holder::Buffer;
/// use yggdryl::Scalar;
///
/// let mut workbook = Workbook::new();
/// let trades = workbook.add_sheet("Trades")?;
/// trades.set_cell("A1".parse()?, "symbol")?;
/// trades.set_cell("A2".parse()?, "AAPL")?;
/// let bytes = workbook.into_bytes()?;
///
/// // The package opens again over any handle, and only the sheet asked
/// // for is parsed.
/// let opened = Workbook::open(Buffer::from_bytes(bytes))?;
/// assert_eq!(opened.sheet_names(), ["Trades"]);
/// assert_eq!(opened.sheet("trades")?.scalar("A2".parse()?), Scalar::from("AAPL"));
/// assert!(!opened.is_dirty());
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Debug)]
pub struct Workbook {
    /// Which workbook this is: the one whose styles a sheet's style ids
    /// index, and whose packages it can rebase onto.
    pub(super) id: u64,
    source: Source,
    system: DateSystem,
    /// The date system the source's workbook part states.
    stated_system: DateSystem,
    slots: Vec<Slot>,
    next_key: u32,
    /// The highest `sheetId` a tab of this workbook has had.
    last_sheet_id: u32,
    /// The workbook part, `xl/workbook.xml` unless the package says otherwise.
    workbook_part: SmolStr,
    strings_part: Option<SmolStr>,
    styles_part: Option<SmolStr>,
    strings: OnceLock<Arc<SharedStrings>>,
    styles: OnceLock<Arc<StyleSheet>>,
    /// Moved by every change of the sheet list or the date system; the
    /// source's documents state the model while it equals the saved one.
    documents: u64,
    documents_saved: u64,
    /// The [`Snapshot::sequence`] of the package last adopted, `0` for none:
    /// a package built before it holds an older state.
    adopted: u64,
    /// What the workbook part states beside its sheets and its date
    /// system, held apart so a workbook stays small to move.
    stated: Box<Stated>,
}

/// What the workbook part states that the model reads beside the sheet
/// list: the defined names, the views and the pivot caches.
#[derive(Debug, Default)]
struct Stated {
    /// Resolved once from the workbook root; every generated part uses it.
    family: NamespaceFamily,
    /// Authored text semantics; the original workbook XML remains its writer.
    text_compatibility: super::formula::text::Compatibility,
    /// The defined names, in the order the workbook part lists them.
    names: Vec<DefinedName>,
    /// Whether a rename, a removal or a move touched a name, so the list is
    /// written again from the model.
    names_touched: bool,
    /// The workbook part's views, and whether a removal or a move moved
    /// their tabs.
    views: Vec<View>,
    views_touched: bool,
    /// The theme part the workbook's relationships name, and the theme read
    /// from it on first use.
    theme_part: Option<SmolStr>,
    theme: OnceLock<Theme>,
    /// The parts beside the sheets an edit rewrote or removed. `None`
    /// masks a source member until a successful save records its absence.
    overrides: BTreeMap<SmolStr, PartOverride>,
    override_revision: u128,
    /// The package's reference-bearing parts, plus shared table-owner documents,
    /// read on the first edit that asks ([`Workbook::referring`]).
    referring: OnceLock<Arc<[Referring]>>,
    pivots: OnceLock<PivotInventory>,
    /// The slicer and timeline caches the workbook's relationships name.
    caches: Vec<SmolStr>,
    /// Only a running Batch allocates a ledger; ordinary edits keep none.
    attempt: Option<Box<Attempt>>,
    /// Created only by explicit calculation; record/media reads keep none.
    calculation: Option<Box<Calculation>>,
    /// Runtime formula context; no package metadata or eager sheet parse.
    clock: super::formula::Clock,
}

/// All workbook cache identities, including a cache no visible pivot uses,
/// are resolved by the existing lazy pivot intake exactly once.
#[derive(Debug, Default)]
struct PivotInventory {
    tables: Vec<PivotTable>,
    max_cache_id: Option<u32>,
}

/// One insertion into the effective package. An undo may reintroduce the
/// same shared bytes after a snapshot, so payload identity is insufficient.
#[derive(Debug)]
struct PartOverride {
    revision: u128,
    bytes: Option<Arc<[u8]>>,
}

/// What a guarded cell/layout edit can change beside its retained payload.
/// One-sheet edits keep their stamp inline; no style table is cloned here.
pub(crate) struct GuardCheckpoint {
    styles: StyleCheckpoint,
    revisions: SmallVec<[(SheetKey, u64, Option<ChangeMark>); 1]>,
}

/// Only a running Batch owns these before-images. Payloads stay in the
/// ordinary generated inverses; this ledger holds stamps and overlay identity.
#[derive(Debug, Default)]
struct Attempt {
    before: Vec<Before>,
    marks: SmallVec<[usize; 2]>,
    rollback: bool,
}

#[derive(Debug)]
enum Before {
    Part(SmolStr, Option<PartOverride>),
    Sheet(SheetStamp),
}

#[derive(Debug)]
struct SheetStamp {
    key: SheetKey,
    name: SmolStr,
    state: SheetState,
    saved: u64,
    parsed: Option<(SmolStr, SheetState, u64, Option<ChangeMark>)>,
}

/// One nested Batch's boundary in the shared ledger.
pub(crate) struct BatchMark {
    start: usize,
    documents: u64,
    documents_saved: u64,
    next_key: u32,
    last_sheet_id: u32,
    styles: Option<StyleCheckpoint>,
    names_touched: bool,
    views_touched: bool,
    referring: Option<Arc<[Referring]>>,
    calculation_valid: Option<bool>,
}

impl BatchMark {
    /// The shared ledger boundary this nested Batch must close.
    pub(crate) fn start(&self) -> usize {
        self.start
    }
}

impl Attempt {
    fn sheet_stamp(&self, slot: &Slot) -> Option<SheetStamp> {
        let start = *self.marks.last().expect("an attempt has an active Batch");
        if self.rollback
            || self.before[start..]
                .iter()
                .any(|before| matches!(before, Before::Sheet(stamp) if stamp.key == slot.key))
        {
            return None;
        }
        Some(SheetStamp {
            key: slot.key,
            name: slot.name.clone(),
            state: slot.state,
            saved: slot.saved,
            parsed: slot.parsed.get().map(|sheet| {
                let name = if sheet.name() == slot.name {
                    slot.name.clone()
                } else {
                    SmolStr::new(sheet.name())
                };
                (name, sheet.state(), sheet.revision(), sheet.change_mark())
            }),
        })
    }
}

impl Stated {
    /// The sole writer of overrides gives each insertion a distinct stamp.
    fn set_part(&mut self, name: SmolStr, bytes: Option<Arc<[u8]>>) {
        self.pivots.take();
        // No input sets this counter: exhausting it takes 2^128 actual part
        // writes, beyond a process lifetime. Never wrap and alias a snapshot.
        self.override_revision = self
            .override_revision
            .checked_add(1)
            .expect("a process cannot perform 2^128 package part writes");
        // An inverse may publish an unchanged part retained only because the
        // opposite shift would move it. Its first write still needs the exact
        // prior overlay, even when that write happens during rollback.
        let record = self.attempt.as_ref().is_some_and(|attempt| {
            let start = *attempt
                .marks
                .last()
                .expect("an attempt has an active Batch");
            !attempt.before[start..]
                .iter()
                .any(|before| matches!(before, Before::Part(held, _) if *held == name))
        });
        let next = PartOverride {
            revision: self.override_revision,
            bytes,
        };
        if record {
            let before = self.overrides.insert(name.clone(), next);
            self.attempt
                .as_mut()
                .expect("recording this Batch")
                .before
                .push(Before::Part(name, before));
        } else {
            self.overrides.insert(name, next);
        }
    }

    fn remember_slot(&mut self, slot: &Slot) {
        if let Some(attempt) = &mut self.attempt {
            if let Some(stamp) = attempt.sheet_stamp(slot) {
                attempt.before.push(Before::Sheet(stamp));
            }
        }
    }
}

impl Default for Workbook {
    fn default() -> Self {
        Self::new()
    }
}

impl Workbook {
    /// An empty workbook under the 1900 date system, with no sheet yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: WORKBOOKS.fetch_add(1, Ordering::Relaxed),
            source: Source::Template,
            system: DateSystem::Year1900,
            stated_system: DateSystem::Year1900,
            slots: Vec::new(),
            next_key: 0,
            last_sheet_id: 0,
            workbook_part: SmolStr::new_static(package::WORKBOOK_PART),
            strings_part: None,
            styles_part: None,
            strings: OnceLock::new(),
            styles: OnceLock::new(),
            documents: 0,
            documents_saved: 0,
            adopted: 0,
            stated: Box::default(),
        }
    }

    /// Open the package `handle` holds: the archive is mounted and the
    /// workbook documents read; no sheet is parsed yet.
    ///
    /// A handle holding nothing opens as an empty workbook, per the laziness
    /// contract every read follows.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for a BIFF (`.xls`) or encrypted
    /// workbook, [`Error::Codec`] for bytes that are not a ZIP package or a
    /// part that is not well-formed, and [`Error::InvalidRecord`] naming the
    /// part for a package with no workbook.
    pub fn open(handle: impl Into<Holder>) -> Result<Self> {
        let handle = handle.into();
        if handle.size() == 0 {
            return Ok(Self::new());
        }
        let head = handle.read_range_bytes(0, 4)?;
        if head == [0xD0, 0xCF, 0x11, 0xE0] {
            return Err(Error::unsupported(
                "reading a BIFF or encrypted workbook",
                handle.url().map_or_else(
                    || SmolStr::new_static("<buffer>"),
                    |url| format_smolstr!("{url}"),
                ),
            ));
        }
        let archive = Arc::new(ZipArchive::new(handle));
        let mut workbook = Self::new();
        workbook.load(archive)?;
        Ok(workbook)
    }

    /// Open the package `bytes` hold.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::open`] returns.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        Self::open(Holder::buffer(Buffer::from_bytes(bytes)))
    }

    /// Read the package documents off `archive`.
    fn load(&mut self, archive: Arc<ZipArchive>) -> Result<()> {
        let members = archive.entries().map_err(|error| match error {
            Error::Codec { position, reason, .. } => Error::Codec {
                format: "xlsx",
                position,
                reason: format_smolstr!(
                    "expected a ZIP package (application/vnd.openxmlformats-officedocument.spreadsheetml.sheet), got: {reason}"
                ),
            },
            other => other,
        })?;
        let held = |name: &str| members.iter().any(|entry| entry.name() == name);
        let listing = || {
            members
                .iter()
                .map(|entry| entry.name().to_owned())
                .collect::<Vec<_>>()
                .join(", ")
        };
        // The office document is where the package's own relationships say,
        // and at the conventional part where they say nothing.
        let workbook_part = if held(package::ROOT_RELATIONSHIPS_PART) {
            let root = archive.read_member(package::ROOT_RELATIONSHIPS_PART)?;
            Relationships::from_xml(&root, "")?
                .first_of(RelationshipKind::OfficeDocument)
                .and_then(|relationship| relationship.target.clone())
                .unwrap_or_else(|| SmolStr::new_static(package::WORKBOOK_PART))
        } else {
            SmolStr::new_static(package::WORKBOOK_PART)
        };
        if !held(&workbook_part) {
            return Err(Error::InvalidRecord {
                path: workbook_part,
                reason: format_smolstr!(
                    "expected the workbook part in the package, got the members [{}]",
                    listing()
                ),
            });
        }
        let relationships_part = package::relationships_part_of(&workbook_part);
        let relationships = if held(&relationships_part) {
            Relationships::from_xml(&archive.read_member(&relationships_part)?, &workbook_part)?
        } else {
            Relationships::default()
        };
        let document = archive.read_member(&workbook_part)?;
        let WorkbookDocument {
            sheets: entries,
            family,
            text_compatibility,
            system,
            names,
            views,
            ..
        } = read_workbook(&document, &workbook_part)?;
        let mut slots = Vec::with_capacity(entries.len());
        let mut numbered: Vec<u32> = entries.iter().filter_map(|entry| entry.sheet_id).collect();
        let mut last_sheet_id = numbered.iter().copied().max().unwrap_or(0);
        for entry in entries {
            // A sheet is its part: an `r:id` no relationship names, or one
            // naming an external target, is a package no reader can honour.
            let part = relationships
                .by_id(&entry.rid)
                .and_then(|relationship| relationship.target.clone())
                .ok_or_else(|| Error::InvalidRecord {
                    path: workbook_part.clone(),
                    reason: format_smolstr!(
                        "expected the relationship {} of sheet {:?} to name a part in {}, got [{}]",
                        entry.rid,
                        entry.name,
                        relationships_part,
                        relationships
                            .entries()
                            .iter()
                            .map(|relationship| relationship.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                })?;
            let kind = match relationships
                .by_id(&entry.rid)
                .map(|relationship| relationship.kind)
            {
                Some(RelationshipKind::Chartsheet) => SheetKind::Chartsheet,
                Some(RelationshipKind::Dialogsheet) => SheetKind::Dialogsheet,
                _ => SheetKind::Worksheet,
            };
            // A tab stating no number takes the next one.
            let sheet_id = entry.sheet_id.unwrap_or_else(|| {
                let id = next_sheet_id(last_sheet_id, |id| numbered.contains(&id));
                numbered.push(id);
                last_sheet_id = last_sheet_id.max(id);
                id
            });
            let key = self.take_key();
            slots.push(Slot {
                name: entry.name,
                key,
                sheet_id,
                kind,
                state: entry.state,
                part,
                backed: true,
                parsed: OnceLock::new(),
                saved: 0,
            });
        }
        // A name's scope is the tab its `localSheetId` counts to; one past
        // the tabs is a name of no sheet this workbook has, kept as read.
        self.stated.names = names
            .into_iter()
            .map(|entry| {
                let scope = entry
                    .local_sheet_id
                    .and_then(|tab| slots.get(tab).map(|slot: &Slot| (tab, slot.key)));
                DefinedName::read(
                    Arc::from(entry.raw),
                    entry.name,
                    &entry.text,
                    scope,
                    entry.hidden,
                    entry.comment,
                )
            })
            .collect();
        self.stated.views = views;
        self.stated.family = family;
        self.stated.text_compatibility = text_compatibility;
        self.source = Source::Archive(archive);
        self.system = system;
        self.stated_system = system;
        self.slots = slots;
        self.last_sheet_id = last_sheet_id;
        self.workbook_part = workbook_part;
        self.strings_part = relationships
            .first_of(RelationshipKind::SharedStrings)
            .and_then(|relationship| relationship.target.clone())
            .filter(|part| held(part));
        self.styles_part = relationships
            .first_of(RelationshipKind::Styles)
            .and_then(|relationship| relationship.target.clone())
            .filter(|part| held(part));
        self.stated.theme_part = relationships
            .first_of(RelationshipKind::Theme)
            .and_then(|relationship| relationship.target.clone())
            .filter(|part| held(part));
        self.stated.caches = relationships
            .entries()
            .iter()
            .filter(|relationship| {
                matches!(
                    relationship.kind,
                    RelationshipKind::SlicerCache | RelationshipKind::TimelineCache
                )
            })
            .filter_map(|relationship| relationship.target.clone())
            .filter(|part| held(part))
            .collect();
        Ok(())
    }

    /// A key no sheet of this workbook has had.
    fn take_key(&mut self) -> SheetKey {
        let key = SheetKey(self.next_key);
        self.next_key += 1;
        key
    }

    /// A slot for `sheet`, which no member holds yet.
    fn slot_of(&mut self, mut sheet: Sheet, sheet_id: u32, part: SmolStr) -> Slot {
        // The implicit envelope of a frameless standalone Sheet is
        // Transitional. Materialize Strict before publishing a fresh slot;
        // authored/imported frames and retained removals stay byte-exact.
        if self.stated.family == NamespaceFamily::Strict && sheet.frame().is_none() {
            sheet.set_frame(Some(Box::new(super::carried::WorksheetFrame::new(
                self.stated.family,
            ))));
        }
        let slot = Slot {
            name: SmolStr::new(sheet.name()),
            key: self.take_key(),
            sheet_id,
            kind: SheetKind::Worksheet,
            state: sheet.state(),
            part,
            backed: false,
            parsed: OnceLock::new(),
            saved: 0,
        };
        let _ = slot.parsed.set(sheet);
        slot
    }

    /// Choose past effective members, relationship owners, live slots and
    /// prospective part names; the highest number falls back to the lowest gap.
    fn next_part<'a>(
        &self,
        prefix: &str,
        suffix: &str,
        pending: impl Iterator<Item = &'a SmolStr>,
    ) -> Result<SmolStr> {
        let members = self.members()?;
        let number = |name: &SmolStr| {
            let owner = package::source_of_relationships(name);
            let name = owner.as_ref().unwrap_or(name);
            name.strip_prefix(prefix)
                .and_then(|rest| rest.strip_suffix(suffix))
                .and_then(|number| number.parse::<usize>().ok())
        };
        let mut numbers: BTreeSet<usize> = members
            .iter()
            .chain(self.slots.iter().map(|slot| &slot.part))
            .filter_map(&number)
            .collect();
        numbers.extend(pending.filter_map(number));
        let number = numbers
            .last()
            .map_or(Some(1), |last| last.checked_add(1))
            .or_else(|| (1..=usize::MAX).find(|number| !numbers.contains(number)))
            .ok_or_else(|| Error::InvalidRecord {
                path: self.workbook_part.clone(),
                reason: SmolStr::new_static(
                    "expected a package part number no member has, got none left",
                ),
            })?;
        Ok(format_smolstr!("{prefix}{number}{suffix}"))
    }

    /// The date system the workbook's serials count from.
    #[must_use]
    pub const fn date_system(&self) -> DateSystem {
        self.system
    }

    /// Use `clock` for subsequent explicit formula passes.
    #[must_use]
    pub fn with_clock(mut self, clock: super::formula::Clock) -> Self {
        self.set_clock(clock);
        self
    }

    /// Replace the runtime clock without changing worksheet or package bytes.
    pub fn set_clock(&mut self, clock: super::formula::Clock) {
        self.stated.clock = clock;
        if let Some(calculation) = self.stated.calculation.as_mut() {
            calculation.pass = 0;
        }
    }

    /// Count serial dates from `system` in every sheet written, the ones
    /// already parsed included.
    ///
    /// A date keeps its day: every worksheet is written again on the next
    /// save, its serials counted from the new epoch, and a sheet not yet
    /// parsed reads its part under the system the package states before
    /// taking this one.
    pub fn set_date_system(&mut self, system: DateSystem) {
        if self.system == system {
            return;
        }
        self.system = system;
        self.documents += 1;
        for sheet in self
            .slots
            .iter_mut()
            .filter_map(|slot| slot.parsed.get_mut())
        {
            sheet.set_date_system(system);
        }
    }

    /// Whether the workbook holds anything its package does not: a sheet
    /// added, removed, renamed or changed, or the date system - since it
    /// was opened or last saved.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.documents != self.documents_saved
            || !self.stated.overrides.is_empty()
            || self
                .slots
                .iter()
                .any(|slot| slot.is_dirty() || slot.state() != slot.state)
    }

    /// Lazily inspect pivot identities without parsing unrelated worksheets.
    /// A foreign pivot remains read-only until its selected XML is completely
    /// representable by the typed writer.
    /// Inspect source headers and items without publishing a pivot.
    pub fn pivot_fields(&self, source: &PivotSource) -> Result<Vec<PivotFieldInfo>> {
        super::pivot::compute::field_info(source, self.sheet(&source.sheet)?)
    }

    pub fn pivots(&self) -> Result<&[PivotTable]> {
        if self.stated.pivots.get().is_none() {
            let pivots = self.read_pivots()?;
            let _ = self.stated.pivots.set(pivots);
        }
        Ok(&self.stated.pivots.get().expect("pivot inventory was installed").tables)
    }

    fn read_pivots(&self) -> Result<PivotInventory> {
        let bytes = self.part_bytes(&self.workbook_part)?;
        let document = read_workbook(&bytes, &self.workbook_part)?;
        let relationships_part = package::relationships_part_of(&self.workbook_part);
        let relationships = self
            .part_bytes_if_present(&relationships_part)?
            .map(|bytes| Relationships::from_xml(&bytes, &self.workbook_part))
            .transpose()?
            .unwrap_or_default();
        let prefix = self.stated.family.relationships_namespace();
        let cache_type = format_smolstr!("{prefix}/pivotCacheDefinition");
        let table_type = format_smolstr!("{prefix}/pivotTable");
        let mut caches = HashMap::<u32, SmolStr>::new();
        for entry in document.pivot_caches {
            let id = entry.id.parse::<u32>().map_err(|_| Error::InvalidRecord {
                path: format_smolstr!("{}#pivotCaches/pivotCache", self.workbook_part),
                reason: format_smolstr!("expected a numeric cacheId, got {:?}", entry.id),
            })?;
            let relationship = relationships.by_id(&entry.rid).ok_or_else(|| Error::InvalidRecord {
                path: relationships_part.clone(),
                reason: format_smolstr!("expected relationship {} of pivot cache {id}", entry.rid),
            })?;
            if relationship.type_uri != cache_type {
                return Err(Error::InvalidRecord {
                    path: relationships_part.clone(),
                    reason: format_smolstr!("expected {cache_type} for {}, got {}", entry.rid, relationship.type_uri),
                });
            }
            let target = relationship.target.clone().ok_or_else(|| Error::InvalidRecord {
                path: relationships_part.clone(),
                reason: format_smolstr!("expected internal target for {}", entry.rid),
            })?;
            if caches.insert(id, target).is_some() {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{}#pivotCaches", self.workbook_part),
                    reason: format_smolstr!("expected unique cacheId, got {id}"),
                });
            }
        }
        let max_cache_id = caches.keys().copied().max();
        let mut pivots = Vec::new();
        let mut names = HashSet::<SmolStr>::new();
        let mut tables = HashSet::<SmolStr>::new();
        for slot in self.slots.iter().filter(|slot| slot.kind == SheetKind::Worksheet) {
            let host_rels = package::relationships_part_of(&slot.part);
            let Some(bytes) = self.part_bytes_if_present(&host_rels)? else {
                continue;
            };
            let relationships = Relationships::from_xml(&bytes, &slot.part)?;
            for relationship in relationships.entries().iter().filter(|item| item.type_uri == table_type) {
                let table_part = relationship.target.clone().ok_or_else(|| Error::InvalidRecord {
                    path: host_rels.clone(),
                    reason: format_smolstr!("expected internal pivot target for {}", relationship.id),
                })?;
                if !tables.insert(table_part.clone()) {
                    return Err(Error::InvalidRecord {
                        path: host_rels.clone(),
                        reason: format_smolstr!("expected one host for pivot part {table_part}"),
                    });
                }
                let table = Registration::root_named(
                    &self.part_bytes(&table_part)?,
                    "pivotTableDefinition",
                    &table_part,
                )?;
                let name = table.root_attribute(b"name")?.filter(|name| !name.is_empty())
                    .ok_or_else(|| Error::InvalidRecord {
                        path: table_part.clone(),
                        reason: "expected a pivot name".into(),
                    })?;
                let name = SmolStr::new(super::shared_strings::decode(&name));
                if !names.insert(SmolStr::new(name.to_ascii_lowercase())) {
                    return Err(Error::InvalidRecord {
                        path: table_part.clone(),
                        reason: format_smolstr!("expected a unique pivot name, got {name}"),
                    });
                }
                let id = table.root_attribute(b"cacheId")?
                    .and_then(|id| id.parse::<u32>().ok())
                    .ok_or_else(|| Error::InvalidRecord {
                        path: table_part.clone(),
                        reason: "expected a numeric pivot cacheId".into(),
                    })?;
                let cache_part = caches.get(&id).cloned().ok_or_else(|| Error::InvalidRecord {
                    path: table_part.clone(),
                    reason: format_smolstr!("expected a workbook pivot cache for cacheId {id}"),
                })?;
                let table_rels = package::relationships_part_of(&table_part);
                let links = Relationships::from_xml(&self.part_bytes(&table_rels)?, &table_part)?;
                let mut linked = links.entries().iter().filter(|link| link.type_uri == cache_type);
                let actual = linked.next().and_then(|link| link.target.as_deref());
                if actual != Some(cache_part.as_str()) || linked.next().is_some() {
                    return Err(Error::InvalidRecord {
                        path: table_rels,
                        reason: format_smolstr!(
                            "expected one pivot cache relationship to {cache_part} for cacheId {id}, got {actual:?}"
                        ),
                    });
                }
                let cache = Registration::root_named(
                    &self.part_bytes(&cache_part)?,
                    "pivotCacheDefinition",
                    &cache_part,
                )?;
                let mut locations = table.children_named(
                    &[super::NAMESPACE, super::STRICT_NAMESPACE], "location",
                )?;
                if locations.len() != 1 {
                    return Err(Error::InvalidRecord {
                        path: table_part.clone(),
                        reason: format_smolstr!("expected one pivot location, got {}", locations.len()),
                    });
                }
                let range = locations.remove(0).root_attribute(b"ref")?
                    .and_then(|text| text.parse::<CellRange>().ok())
                    .ok_or_else(|| Error::InvalidRecord {
                        path: format_smolstr!("{table_part}#location"),
                        reason: "expected a worksheet rectangle".into(),
                    })?;
                let mut unrepresented = None;
                let spec = self.vertical_pivot_spec(&name, &table, &cache, &table_part, &cache_part, &mut unrepresented)?;
                let reason = unrepresented.or_else(|| spec.is_none().then(||
                    SmolStr::new_static("foreign pivot fields are not representable by the current typed writer")));
                pivots.push(PivotTable::read(
                    PivotIdentity {
                        name,
                        sheet: slot.name.clone(),
                        location: range,
                        cache_id: id,
                        table_part,
                        cache_part,
                    },
                    spec,
                    reason,
                ));
            }
        }
        Ok(PivotInventory { tables: pivots, max_cache_id })
    }

    /// The sheets, in tab order, worksheets and chart sheets alike.
    #[must_use]
    pub fn sheet_names(&self) -> Vec<&str> {
        self.slots.iter().map(|slot| slot.name.as_str()).collect()
    }

    /// What the sheet `name` is, `None` for a name no sheet has.
    #[must_use]
    pub fn sheet_kind(&self, name: &str) -> Option<SheetKind> {
        self.resolve(name).map(|at| self.slots[at].kind)
    }

    /// The key of the sheet `name`, compared without case; `None` for a
    /// name no sheet has.
    #[must_use]
    pub fn sheet_key(&self, name: &str) -> Option<SheetKey> {
        self.resolve(name).map(|at| self.slots[at].key)
    }

    /// The name of the sheet `key` names, `None` once the sheet is gone.
    #[must_use]
    pub fn sheet_by_key(&self, key: SheetKey) -> Option<&str> {
        self.slots
            .iter()
            .find(|slot| slot.key == key)
            .map(|slot| slot.name.as_str())
    }

    /// How many sheets the workbook lists.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the workbook lists no sheet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The position of the sheet `name`, compared without case as Excel
    /// compares names.
    fn resolve(&self, name: &str) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| slot.name.eq_ignore_ascii_case(name) || slot.name == name)
    }

    /// The worksheet `name`, parsed on first access.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] listing the sheets when none has the name,
    /// [`Error::InvalidRecord`] naming the kind for a chart or dialog sheet,
    /// or the part's refusal.
    pub fn sheet(&self, name: &str) -> Result<&Sheet> {
        let at = self.resolve(name).ok_or_else(|| self.absent(name))?;
        self.parsed(at)
    }

    /// The worksheet `name`, when the workbook has it.
    ///
    /// # Errors
    ///
    /// Returns a chart or dialog sheet's refusal, or the part's.
    pub fn get_sheet(&self, name: &str) -> Result<Option<&Sheet>> {
        match self.resolve(name) {
            Some(at) => self.parsed(at).map(Some),
            None => Ok(None),
        }
    }

    /// The sheet at zero-based `index` in tab order, when there is one.
    ///
    /// # Errors
    ///
    /// Returns a chart or dialog sheet's refusal, or the part's.
    pub fn sheet_at(&self, index: usize) -> Result<Option<&Sheet>> {
        if index >= self.slots.len() {
            return Ok(None);
        }
        self.parsed(index).map(Some)
    }

    /// The worksheet `name`, mutably, parsed on first access.
    ///
    /// The tab's name is the workbook's fact: [`Self::rename_sheet`] changes
    /// it, and a name set on the sheet itself through this borrow is not
    /// what the package is written under.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::sheet`] returns.
    pub fn sheet_mut(&mut self, name: &str) -> Result<&mut Sheet> {
        let at = self.resolve(name).ok_or_else(|| self.absent(name))?;
        self.parsed(at)?;
        self.stated.remember_slot(&self.slots[at]);
        Ok(self.slots[at]
            .parsed
            .get_mut()
            .expect("the sheet was parsed by the call above"))
    }

    /// The style the cell at `at` of the sheet `sheet` displays with: its
    /// own, else its row's, else its column's, else the default - an empty
    /// cell included. A date, time or duration whose style does not read as
    /// one - a cell built in memory holds the default style - shows, and
    /// is saved, with its style's other facts and the crate's code for it
    /// (`yyyy-mm-dd`, `h:mm:ss`, ...).
    ///
    /// ```
    /// use yggdryl::excel::{StylePatch, Workbook};
    ///
    /// let mut workbook = Workbook::new();
    /// workbook.add_sheet("Sheet1")?;
    /// assert_eq!(workbook.cell_style("Sheet1", "C3".parse()?)?.font.name, "Calibri");
    /// let italic = StylePatch { italic: Some(true), ..StylePatch::default() };
    /// workbook.set_style("Sheet1", &["3:3".parse()?], &italic)?;
    /// assert!(workbook.cell_style("Sheet1", "C3".parse()?)?.font.italic);
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns what [`Self::sheet`] and [`Self::style_sheet`] return.
    pub fn cell_style(&self, sheet: &str, at: CellRef) -> Result<CellStyle> {
        let sheet = self.sheet(sheet)?;
        let held = sheet.cell(at).map_or(NumberFormat::General, Cell::format);
        Ok(self
            .style_sheet()?
            .shown_style(sheet.style_at(at), held)
            .into_owned())
    }

    /// Apply `patch` to every cell of `ranges` in the sheet `sheet`, as the
    /// ribbon does.
    ///
    /// Each distinct style the cells display with is patched once and the
    /// result found in the styles or appended to them, so a range of any
    /// size costs its cells and the distinct styles among them. A range of
    /// whole columns (`B:D`) or whole rows (`3:5`) sets the columns' or the
    /// rows' own style and patches the cells they hold, putting no cell
    /// where none is; any other range puts a blank cell in the style where
    /// no cell is. A border preset is drawn on each range: `Outside` on its
    /// outline only; a cell several ranges hold is patched once, with every
    /// edge each of them draws on it. A cell whose number format changes
    /// what its number is reads its value again: `45292` under `m/d/yyyy`
    /// is a date, and a date under `0.00` its serial; a date whose style
    /// does not read as one keeps being one ([`Self::cell_style`]).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a patch no cell can take (see
    /// [`StylePatch`]), for ranges putting more than 2,097,152 blank cells,
    /// or when the styles would hold more than
    /// [`MAX_CELL_FORMATS`](super::MAX_CELL_FORMATS); what [`Self::sheet`]
    /// returns otherwise. A refused patch changes nothing.
    pub fn set_style(
        &mut self,
        sheet: &str,
        ranges: &[CellRange],
        patch: &StylePatch,
    ) -> Result<()> {
        let stated = patch.check()?;
        let at = self.resolve(sheet).ok_or_else(|| self.absent(sheet))?;
        // The blank cells the bounded ranges put: each one's area but the
        // cells it holds, a cell two ranges share counted twice.
        let held = self.parsed(at)?;
        let blanks: u64 = ranges
            .iter()
            .filter(|range| !range.is_row_open() && !range.is_column_open())
            .map(|range| {
                (u64::from(range.row_size()) * u64::from(range.column_size()))
                    .saturating_sub(held.cells_in(*range).count() as u64)
            })
            .sum();
        if blanks > MAX_EDITED_CELLS {
            return Err(Error::InvalidRecord {
                path: format_smolstr!(
                    "{sheet}!{}",
                    ranges
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                reason: format_smolstr!(
                    "expected ranges putting at most {MAX_EDITED_CELLS} blank cells, or whole \
                     rows or columns, got {blanks}"
                ),
            });
        }
        self.styles()?;
        let Self { slots, styles, .. } = self;
        let sheet = slots[at]
            .parsed
            .get_mut()
            .expect("the sheet was parsed above");
        let table = Arc::make_mut(styles.get_mut().expect("the styles were read above"));
        let checkpoint = table.checkpoint();
        let mut patcher = Patcher {
            patch,
            stated: stated.as_ref(),
            derived: HashMap::new(),
            plan: StylePlan::default(),
        };
        let plan = match patcher.plan(sheet, table, ranges) {
            Ok(()) => patcher.plan,
            Err(error) => {
                table.rollback(&checkpoint);
                return Err(error);
            }
        };
        plan.apply(sheet, table);
        Ok(())
    }

    /// Type `text` into the cell at `at` of the sheet `sheet`, as en-US
    /// Excel reads it ([`Entry::from_text`]), answering the cell it
    /// replaced.
    ///
    /// The cell keeps its style. A format the text suggests - `0%` for
    /// `12%`, `m/d/yyyy` for a date - is taken only while the cell's format
    /// is General, and the value is what the cell's format then reads it
    /// as: a date typed into a cell formatted `0.00` is its serial, a
    /// number typed into a date cell the day it counts to. In a cell
    /// formatted as text (`@`) whatever is typed stays text as typed, a
    /// formula included. A `'` before the text keeps it text and states
    /// `quotePrefix`; any other entry drops that. A formula is held
    /// uncomputed, and nothing typed clears the content and keeps the
    /// style.
    ///
    /// ```
    /// use yggdryl::excel::Workbook;
    /// use yggdryl::Scalar;
    ///
    /// let mut workbook = Workbook::new();
    /// workbook.add_sheet("Sheet1")?;
    /// let at = "B2".parse()?;
    /// workbook.set_entry("Sheet1", at, "1/2/2024")?;
    /// assert_eq!(workbook.sheet("Sheet1")?.scalar(at), Scalar::date32(19_724));
    /// assert_eq!(workbook.cell_style("Sheet1", at)?.number_format, "m/d/yyyy");
    /// assert_eq!(workbook.entry_text("Sheet1", at)?.as_deref(), Some("1/2/2024"));
    ///
    /// // The cell's format stays: 45 is the 45th day of 1900.
    /// workbook.set_entry("Sheet1", at, "45")?;
    /// assert_eq!(workbook.display_text("Sheet1", at)?.map(|shown| shown.text), Some("2/14/1900".into()));
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for a formula that does not read, at its
    /// byte; [`Error::InvalidRecord`] for a reference outside the grid, a
    /// text past [`MAX_CELL_TEXT`](super::MAX_CELL_TEXT) characters, or a
    /// style the styles cannot take past
    /// [`MAX_CELL_FORMATS`](super::MAX_CELL_FORMATS); what [`Self::sheet`]
    /// returns otherwise. A refused entry changes nothing.
    pub fn set_entry(&mut self, sheet: &str, at: CellRef, text: &str) -> Result<Option<Cell>> {
        self.enter_text(sheet, at, text, None)
    }

    /// [`Self::set_entry`], a formula the text reads as holding `shape`
    /// instead: what Ctrl+Enter puts in every cell of a selection, the
    /// formula typed at the active cell translated to each.
    pub(crate) fn enter_text(
        &mut self,
        sheet: &str,
        at: CellRef,
        text: &str,
        shape: Option<&super::formula::Formula>,
    ) -> Result<Option<Cell>> {
        at.require_in_grid()?;
        let index = self.resolve(sheet).ok_or_else(|| self.absent(sheet))?;
        self.parsed(index)?;
        self.styles()?;
        let system = self.system;
        let Self { slots, styles, .. } = self;
        let sheet = slots[index]
            .parsed
            .get_mut()
            .expect("the sheet was parsed above");
        let table = Arc::make_mut(styles.get_mut().expect("the styles were read above"));
        let source = sheet.style_at(at);
        let held = sheet.cell(at).map_or(NumberFormat::General, Cell::format);
        let format = table
            .shown_format(source, held)
            .cloned()
            .unwrap_or_default();
        let mut entry = Entry::from_text_in(text, at, system, &format)?;
        if let (Entry::Formula(formula), Some(shape)) = (&mut entry, shape) {
            *formula = shape.clone();
        }
        let mut wanted = table.shown_style(source, held).into_owned();
        wanted.quote_prefix = matches!(entry, Entry::Quoted(_));
        if let Entry::Value {
            format: Some(code), ..
        } = &entry
        {
            if format.is_general() {
                wanted.number_format = code.clone();
            }
        }
        let checkpoint = table.checkpoint();
        let entered = enter(sheet, table, at, entry, source, &wanted, system);
        if entered.is_err() {
            table.rollback(&checkpoint);
        }
        entered
    }

    /// What the cell at `at` of the sheet `sheet` would be typed as, `None`
    /// where no cell is: `=` and the formula as it is typed, a date as
    /// `m/d/yyyy`, a clock as `h:mm:ss AM/PM`, an elapsed time as its hours
    /// (`36:00:00`), a percentage as `12%`, a number in at most fifteen
    /// significant digits, an error or a boolean as it is spelled, text as
    /// it is - after a `'` where its style states `quotePrefix` or where it
    /// would read as anything else (`'TRUE`, `'007`, `'=A1`). Typing it
    /// again ([`Self::set_entry`]) leaves the cell's value and formula as
    /// they are, text read back after its `'` stating `quotePrefix`; a
    /// number or a boolean in a cell formatted as text reads back as text,
    /// as in Excel.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::sheet`] and [`Self::style_sheet`] return.
    pub fn entry_text(&self, sheet: &str, at: CellRef) -> Result<Option<String>> {
        let sheet = self.sheet(sheet)?;
        let Some(cell) = sheet.cell(at) else {
            return Ok(None);
        };
        if let Some(formula) = cell.formula() {
            return Ok(Some(format!("={}", formula.entry_at(at))));
        }
        if cell.error().is_some() {
            return Ok(Some(cell.error_text().to_owned()));
        }
        let styles = self.style_sheet()?;
        let quoted = styles
            .style(cell.style())
            .is_some_and(|style| style.quote_prefix);
        let general;
        let format = match styles.shown_format(cell.style(), cell.format()) {
            Some(format) => format,
            None => {
                general = FormatCode::general();
                &general
            }
        };
        let raw = sheet.retained_serial(at).map(Scalar::from);
        Ok(Some(Entry::spell(
            raw.as_ref().unwrap_or(cell.value()),
            format,
            quoted,
            at,
            self.system,
        )))
    }

    /// The text the cell at `at` of the sheet `sheet` displays, through
    /// its number format ([`FormatCode::render`]), `None` where no cell is;
    /// an error cell displays its error, and a section's `[ColorN]` is the
    /// colour the workbook's palette gives it.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::sheet`] and [`Self::style_sheet`] return.
    pub fn display_text(&self, sheet: &str, at: CellRef) -> Result<Option<Rendered>> {
        let sheet = self.sheet(sheet)?;
        let Some(cell) = sheet.cell(at) else {
            return Ok(None);
        };
        if cell.error().is_some() {
            return Ok(Some(Rendered {
                text: SmolStr::new(cell.error_text()),
                ..Rendered::default()
            }));
        }
        let styles = self.style_sheet()?;
        let general;
        let format = match styles.shown_format(cell.style(), cell.format()) {
            Some(format) => format,
            None => {
                general = FormatCode::general();
                &general
            }
        };
        let raw = sheet.retained_serial(at).map(Scalar::from);
        Ok(Some(format.render_in(
            raw.as_ref().unwrap_or(cell.value()),
            self.system,
            styles.indexed_colors(),
        )))
    }

    /// Parse every worksheet the workbook has not parsed yet, so none is
    /// read again; a chart or dialog sheet holds no cells and is passed
    /// over.
    ///
    /// # Errors
    ///
    /// Returns the first part's refusal.
    pub fn parse_all(&self) -> Result<()> {
        for (at, slot) in self.slots.iter().enumerate() {
            if slot.kind == SheetKind::Worksheet {
                self.parsed(at)?;
            }
        }
        Ok(())
    }

    /// Recompute every computable formula in this workbook. An uncomputed
    /// expression retains its prior cached result, and an unchanged result
    /// does not move the sheet's revision.
    ///
    /// # Errors
    ///
    /// Returns the first source or result-conversion refusal before any
    /// cached result is changed.
    pub fn calculate_all(&mut self) -> Result<super::formula::Recalculation> {
        self.calculate(super::formula::graph::PassKind::Full)
    }

    /// Recompute changed formulas and their dependent closure, plus volatile
    /// candidates. A second unchanged pass evaluates no nonvolatile formulas.
    /// Existing uncomputed and circular workbook status is still reported.
    ///
    /// # Errors
    ///
    /// Returns the first dependency/source/cache-conversion refusal without
    /// changing cached values, sheet revisions or pending calculation changes.
    ///
    /// ```
    /// use yggdryl::excel::Workbook;
    /// let mut book = Workbook::new();
    /// book.add_sheet("Data")?.set_cell("A1".parse()?, 2.0)?;
    /// book.set_entry("Data", "B1".parse()?, "=A1+1")?;
    /// assert_eq!(book.recalculate()?.evaluated, 1);
    /// assert_eq!(book.recalculate()?.evaluated, 0);
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    pub fn recalculate(&mut self) -> Result<super::formula::Recalculation> {
        self.calculate(super::formula::graph::PassKind::Incremental)
    }

    /// Metadata edits need no style part; lazy parsing adds no new formats.
    pub(crate) fn style_count(&self) -> usize {
        self.styles.get().map_or(0, |styles| styles.appended_len())
    }

    pub(crate) fn invalidate_calculation(&mut self) {
        if let Some(calculation) = self.stated.calculation.as_mut() {
            calculation.valid = false;
        }
    }

    fn calculate(
        &mut self,
        pass: super::formula::graph::PassKind,
    ) -> Result<super::formula::Recalculation> {
        if self.stated.attempt.is_some() {
            return Err(Error::Conflict {
                expected: "a completed workbook edit before calculation",
                actual: "an active compound edit",
                path: "$.formula.calculation".into(),
            });
        }
        let report = self.prepare_calculation(pass)?;
        self.commit_prepared_calculation(&report);
        Ok(report)
    }

    /// Plan all formula outcomes while the authored edit still has an undo
    /// attempt. No result cell or change receipt is published here.
    pub(crate) fn prepare_calculation(
        &mut self,
        pass: super::formula::graph::PassKind,
    ) -> Result<super::formula::Recalculation> {
        self.parse_all()?;
        let styles = self.styles()?;
        let mut calculation = self.stated.calculation.take().unwrap_or_default();
        let rebuild = calculation.rebuild_required(self);
        let mut resolver =
            ReferenceResolver::indexed(self, std::mem::take(&mut calculation.names), rebuild);
        let result = calculation.prepare(&resolver, &styles, pass, rebuild);
        calculation.names = std::mem::take(&mut resolver.names);
        drop(resolver);
        if result.is_err() {
            calculation.valid = false;
            calculation.replacements.clear();
            calculation.outcomes.clear();
            calculation.clear_evaluations();
        }
        self.stated.calculation = Some(calculation);
        result
    }

    /// Snapshot only formula cells that the prepared pass will replace.
    pub(crate) fn capture_prepared_calculation(
        &self,
    ) -> Result<(Restore, Vec<(SmolStr, CellRange)>)> {
        let mut restore = Restore::new(self);
        let mut touched = Vec::new();
        if let Some(calculation) = &self.stated.calculation {
            let mut by_tab: BTreeMap<usize, Vec<CellRange>> = BTreeMap::new();
            for (tab, cell, _) in &calculation.replacements {
                let at = cell.reference();
                by_tab.entry(*tab).or_default().push(CellRange::new(at, at));
            }
            for (tab, ranges) in by_tab {
                let name = &self.slots[tab].name;
                restore.push(self.cells_step(name, &ranges)?);
                touched.extend(ranges.into_iter().map(|range| (name.clone(), range)));
            }
        }
        let (pass, status) = self.calculation_receipt();
        restore.push(Step::Calculation { pass, status });
        restore.capture_styles(self.styles()?.as_ref());
        Ok((restore, touched))
    }

    /// Publish a completely prepared pass after its enclosing edit is known
    /// to be successful. Conversion and dependency refusal occurred above.
    pub(crate) fn commit_prepared_calculation(
        &mut self,
        report: &super::formula::Recalculation,
    ) {
        let mut calculation = self.stated.calculation.take().expect("prepared above");
        for (tab, cell, bits) in calculation.replacements.drain(..) {
            self.slots[tab]
                .parsed
                .get_mut()
                .expect("parsed above")
                .replace_calculated(cell, bits);
        }
        calculation.identities.clear();
        for slot in &mut self.slots {
            let generation = if slot.kind == SheetKind::Worksheet {
                let sheet = slot.parsed.get_mut().expect("parsed above");
                let generation = sheet.track_changes().generation();
                sheet.acknowledge_changes();
                generation
            } else {
                0
            };
            calculation.identities.push((slot.key, generation));
        }
        calculation.documents = self.documents;
        calculation.valid = true;
        calculation.pass = calculation.pass.wrapping_add(1);
        calculation.last_status = Some(report.clone());
        calculation.outcomes.clear();
        calculation.clear_evaluations();
        self.stated.calculation = Some(calculation);
    }

    pub(crate) fn discard_prepared_calculation(&mut self) {
        if let Some(calculation) = self.stated.calculation.as_mut() {
            calculation.replacements.clear();
            calculation.outcomes.clear();
            calculation.clear_evaluations();
            calculation.valid = false;
        }
    }

    pub(crate) fn calculation_receipt(&self) -> (u64, Option<super::formula::Recalculation>) {
        self.stated.calculation.as_ref().map_or((0, None), |calculation| {
            (calculation.pass, calculation.last_status.clone())
        })
    }

    pub(crate) fn restore_calculation(
        &mut self,
        pass: u64,
        status: Option<super::formula::Recalculation>,
    ) {
        let calculation = self.stated.calculation.get_or_insert_with(Default::default);
        calculation.pass = pass;
        calculation.last_status = status;
        calculation.valid = false;
        calculation.replacements.clear();
        calculation.outcomes.clear();
        calculation.clear_evaluations();
    }

    pub(crate) fn restored_calculation_status(&self) -> super::formula::Recalculation {
        let mut status = self.stated.calculation.as_ref()
            .and_then(|calculation| calculation.last_status.clone())
            .unwrap_or_default();
        status.evaluated = 0;
        status
    }

    /// Rename the sheet `name` to `new_name`, keeping its place and part.
    ///
    /// Every formula of every worksheet and every defined name that names
    /// the sheet names it by its new name, and so does every reference the
    /// package states beside them: the conditional formats, validations,
    /// hyperlinks and sparklines a worksheet carries, and the charts, pivot
    /// caches, tables' calculated and totals formulas, and shapes' cell links
    /// of the package. Each worksheet is
    /// parsed, and one holding such a reference is written again on the
    /// next save; one naming it nowhere is still the member it was. A slicer
    /// or timeline cache names a pivot table by its tab's number, which a
    /// rename keeps.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] listing the sheets when none has the name,
    /// the new name's refusal ([`validate_sheet_name`]), [`Error::Conflict`]
    /// when another sheet already has it, compared without case - a sheet
    /// renamed to its own name in another case takes it - the refusal of a
    /// worksheet part that had to be parsed, or [`Error::Unsupported`]
    /// naming the part and the element where an element this crate does not
    /// model (`dataConsolidate`, `oleObjects`, ...) names the sheet, before
    /// anything changes.
    pub fn rename_sheet(&mut self, name: &str, new_name: impl Into<SmolStr>) -> Result<()> {
        self.rename(name, new_name.into()).map(|_| ())
    }

    /// [`Self::rename_sheet`], answering what renaming the sheet back would
    /// not give back.
    pub(crate) fn rename(&mut self, name: &str, new_name: SmolStr) -> Result<Restore> {
        let at = self.resolve(name).ok_or_else(|| self.absent(name))?;
        validate_sheet_name(&new_name)?;
        if self.resolve(&new_name).is_some_and(|other| other != at) {
            return Err(self.taken(&new_name));
        }
        if self.slots[at].name == new_name {
            return Ok(Restore::new(self));
        }
        self.parse_all()?;
        let old_name = self.slots[at].name.clone();
        let shift = Shift::RenameSheet {
            from: &old_name,
            to: &new_name,
        };
        let plan = self.plan(&shift, &[], None, PartBytes::new())?;
        let mut restore = Restore::new(self);
        self.stated.remember_slot(&self.slots[at]);
        self.follow(&shift, plan, None, &mut restore);
        self.commit_name(at, new_name);
        Ok(restore)
    }

    fn commit_name(&mut self, at: usize, new_name: SmolStr) {
        let slot = &mut self.slots[at];
        // A name is the workbook part's fact, never the sheet part's: a
        // sheet its member holds stays held by it.
        let clean = !slot.is_dirty();
        if let Some(sheet) = slot.parsed.get_mut() {
            sheet
                .set_name(new_name.clone())
                .expect("the name was already validated");
            if clean {
                slot.saved = sheet.revision();
            }
        }
        slot.name = new_name;
        self.documents += 1;
    }

    /// Add an empty worksheet named `name` after the last tab.
    ///
    /// # Errors
    ///
    /// Returns the name's refusal ([`validate_sheet_name`]), or
    /// [`Error::Conflict`] when a sheet already has the name, compared
    /// without case.
    pub fn add_sheet(&mut self, name: impl Into<SmolStr>) -> Result<&mut Sheet> {
        let name = name.into();
        let sheet = Sheet::new(name)?.with_date_system(self.system);
        if self.resolve(sheet.name()).is_some() {
            return Err(self.taken(sheet.name()));
        }
        self.insert_sheet(sheet)?;
        let at = self.slots.len() - 1;
        Ok(self.slots[at]
            .parsed
            .get_mut()
            .expect("the sheet was just inserted"))
    }

    /// Put `sheet` in the workbook: in place of the sheet of the same name,
    /// answering it, or after the last tab.
    ///
    /// A sheet read from another workbook keeps its values, formulas and
    /// number formats, and takes this workbook's default style: its style
    /// ids index the other workbook's styles.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming the kind when the name is a
    /// chart or dialog sheet's, which holds no cells to replace, or the
    /// package's refusal while reserving an unused worksheet part.
    pub fn insert_sheet(&mut self, sheet: Sheet) -> Result<Option<Sheet>> {
        let mut sheet = sheet.with_date_system(self.system);
        match self.resolve(sheet.name()) {
            Some(at) => {
                if self.slots[at].kind != SheetKind::Worksheet {
                    return Err(self.not_a_worksheet(at));
                }
                let part = self.next_part("xl/worksheets/sheet", ".xml", std::iter::empty())?;
                let previous = self.parsed(at).ok().cloned();
                sheet.hold_in(self.id);
                // The tab keeps its place and number; the sheet in it is a
                // new one, which no member holds, and the names defined on
                // the tab are defined on it.
                let sheet_id = self.slots[at].sheet_id;
                let replaced = self.slots[at].key;
                self.slots[at] = self.slot_of(sheet, sheet_id, part);
                self.stated.referring.take();
                let key = self.slots[at].key;
                for defined in &mut self.stated.names {
                    if defined.scope() == Some(replaced) {
                        defined.set_scope(key);
                    }
                }
                self.documents += 1;
                Ok(previous)
            }
            None => {
                let part = self.next_part("xl/worksheets/sheet", ".xml", std::iter::empty())?;
                sheet.hold_in(self.id);
                let sheet_id = next_sheet_id(self.last_sheet_id, |id| {
                    self.slots.iter().any(|slot| slot.sheet_id == id)
                });
                self.last_sheet_id = self.last_sheet_id.max(sheet_id);
                let slot = self.slot_of(sheet, sheet_id, part);
                self.slots.push(slot);
                self.documents += 1;
                Ok(None)
            }
        }
    }

    /// Take the sheet `name` out of the workbook, answering it parsed: every
    /// reference naming it is `#REF!` - in each formula, defined name and
    /// carried child of the other worksheets, and in the charts, tables'
    /// calculated and totals formulas, and shapes' cell links of the package.
    /// A name no sheet has removes nothing and
    /// answers `None`.
    ///
    /// # Errors
    ///
    /// Returns, before anything changes, the part's refusal when the sheet
    /// had to be parsed to be answered; [`Error::Unsupported`] naming the
    /// cache part when a slicer or timeline cache filters a pivot table or a
    /// table the sheet hosts, and naming the part and the element where an
    /// element this crate does not model names the sheet. A chart or dialog
    /// sheet is removed and answers `None`.
    pub fn remove_sheet(&mut self, name: &str) -> Result<Option<Sheet>> {
        Ok(self
            .remove(name, false)?
            .and_then(|mut restore| restore.slot.parsed.take()))
    }

    /// [`Self::remove_sheet`], answering what puts the sheet back.
    pub(crate) fn remove(&mut self, name: &str, retain: bool) -> Result<Option<RestoreSheet>> {
        let Some(at) = self.resolve(name) else {
            return Ok(None);
        };
        self.parse_all()?;
        self.check_caches(at)?;
        // What named the sheet names nothing: its references are `#REF!`,
        // a span of sheets ending on it drawn in, a name defined on it gone.
        let removed = self.slots[at].name.clone();
        let order: Vec<SmolStr> = self.slots.iter().map(|slot| slot.name.clone()).collect();
        let order: Vec<&str> = order.iter().map(SmolStr::as_str).collect();
        let shift = Shift::RemoveSheet {
            name: &removed,
            order: &order,
        };
        let mut plan = self.plan(&shift, &[], Some(at), PartBytes::new())?;
        let parts = if retain {
            self.removed_parts(at)?
        } else {
            RemovedParts::default()
        };
        // Exclusively owned parts leave with this sheet. RemovedParts alone
        // retains their original bytes; a second reference inverse would
        // revive a discarded intermediate image before a later sheet undo.
        // Shared descendants remain in the plan and follow the removed name.
        plan.overrides
            .retain(|entry| !parts.contains(&entry.member));
        plan.kept.retain(|(name, _)| !parts.contains(name));
        let names = self.stated.names.clone();
        let names_touched = self.stated.names_touched;
        let views = self.stated.views.clone();
        let views_touched = self.stated.views_touched;
        let mut restore = Restore::new(self);
        if retain {
            let table = self.styles()?;
            // Original XFs never move, so no retained reference can need a
            // descriptor. In particular, do not rescan clean worksheet XML.
            if table.bindings().next().is_some() {
                if let Some(sheet) = self.slots[at].parsed.get() {
                    for id in sheet.style_ids() {
                        restore.styles.capture(&table, id);
                    }
                }
                // A clean temporal cell can keep DEFAULT in the parsed model
                // while its saved XML names a generated date XF. Both references
                // must retain their original meanings across pending saves.
                for (part, bytes) in &parts.parts {
                    if *part == self.slots[at].part {
                        Sheet::rewrite_style_ids(bytes, part, |id| {
                            restore.styles.capture(&table, id);
                            None
                        })?;
                    }
                }
            }
        }
        self.follow(&shift, plan, Some(at), &mut restore);
        let key = self.slots[at].key;
        let named = self.stated.names.len();
        self.stated
            .names
            .retain(|defined| defined.scope() != Some(key));
        // A tab index past the removed one counts one fewer.
        self.stated.names_touched |= self.stated.names.len() != named
            || self
                .stated
                .names
                .iter()
                .any(|defined| defined.scope().is_some());
        let slot = self.slots.remove(at);
        // Retired tables must not enter a later edit's inverse through the
        // old ownership index, even when this removal changed no references.
        self.stated.referring.take();
        self.remap_views(at);
        self.documents += 1;
        Ok(Some(RestoreSheet {
            position: at,
            slot,
            names,
            names_touched,
            views,
            views_touched,
            restore,
            parts,
        }))
    }

    /// Put back a sheet [`Self::remove`] took out, where it stood, with the
    /// names, views and references it took with it.
    pub(crate) fn restore_sheet(&mut self, restored: RestoreSheet) -> Result<Restore> {
        restored.restore.check_origin(self)?;
        let RestoreSheet {
            position,
            mut slot,
            names,
            names_touched,
            views,
            views_touched,
            mut restore,
            parts,
        } = restored;
        if self.resolve(&slot.name).is_some() {
            return Err(self.taken(&slot.name));
        }
        if self.slots.iter().any(|held| held.part == slot.part) {
            return Err(Error::Conflict {
                expected: "the removed sheet's unused package part",
                actual: "a part now reserved by another live sheet",
                path: format_smolstr!("{} ({})", slot.name, slot.part),
            });
        }
        if self
            .slots
            .iter()
            .any(|held| held.key == slot.key || held.sheet_id == slot.sheet_id)
        {
            return Err(Error::Conflict {
                expected: "the removed sheet's unused identity",
                actual: "a live sheet with the same key or sheetId",
                path: slot.name.clone(),
            });
        }
        // Every fallible read and collision check precedes the first mutation.
        let mut overrides = self.restored_parts(&slot, &parts)?;
        self.check_restore(&restore, Some(slot.key))?;
        let mut inverse = self.restore_inverse(&mut restore)?;
        self.sheet_metadata_inverse(&mut inverse);
        let (styles, moved) = self.styles()?.rebind(restore.styles.iter())?;
        if !moved.is_empty() {
            for (part, bytes) in &mut overrides {
                if *part == slot.part {
                    if let Some(patched) =
                        Sheet::rewrite_style_ids(bytes, part, |id| moved.get(&id).copied())?
                    {
                        *bytes = Arc::from(patched);
                    }
                }
            }
        }
        restore.remap_styles(&moved);
        if let Some(sheet) = slot.parsed.get_mut() {
            sheet.remap_styles(&moved);
        }
        restore.styles = Default::default();
        let position = position.min(self.slots.len());
        for (name, bytes) in overrides {
            self.stated.set_part(name, Some(bytes));
        }
        self.stated.referring.take();
        self.styles = OnceLock::from(styles);
        self.slots.insert(position, slot);
        self.stated.names = names;
        self.stated.names_touched = names_touched || self.stated.names_touched;
        self.stated.views = views;
        self.stated.views_touched = views_touched || self.stated.views_touched;
        self.documents += 1;
        self.commit_restore(restore);
        Ok(inverse)
    }

    /// Move the sheet `name` to tab position `to`, the tabs between it and
    /// there each moving one place toward where it stood.
    ///
    /// Nothing a sheet's formulas state changes: a reference names a sheet
    /// by name, and a span of sheets (`Jan:Mar!B2`) spans whatever tabs now
    /// lie between its ends, as it does in Excel. A name defined on a sheet
    /// stays defined on it, and each workbook view keeps showing the sheet
    /// it showed.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] listing the sheets when none has the name,
    /// or [`Error::InvalidRecord`] for a position past the last tab, before
    /// anything changes.
    pub fn move_sheet(&mut self, name: &str, to: usize) -> Result<()> {
        let at = self.resolve(name).ok_or_else(|| self.absent(name))?;
        if to >= self.slots.len() {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{}", self.slots[at].name),
                reason: format_smolstr!(
                    "expected a tab position below {}, got {to}",
                    self.slots.len()
                ),
            });
        }
        if to == at {
            return Ok(());
        }
        let slot = self.slots.remove(at);
        self.slots.insert(to, slot);
        // Each tab a view names follows the sheet it named.
        let follow = |tab: usize| {
            if tab == at {
                to
            } else if at < tab && tab <= to {
                tab - 1
            } else if to <= tab && tab < at {
                tab + 1
            } else {
                tab
            }
        };
        for view in &mut self.stated.views {
            view.active_tab = view.active_tab.map(follow);
        }
        self.stated.views_touched |= !self.stated.views.is_empty();
        // A name's `localSheetId` counts tabs, which moved.
        self.stated.names_touched |= self
            .stated
            .names
            .iter()
            .any(|defined| defined.scope().is_some());
        self.documents += 1;
        Ok(())
    }

    /// Open `count` empty rows at zero-based `at` of the sheet `sheet`,
    /// moving every row from `at` down, and every reference the workbook
    /// states with what it names: each formula of every sheet, each defined
    /// name, the ranges and formulas the sheet's part carries - conditional
    /// formats, validations, hyperlinks, filters, sparklines - its tables,
    /// drawings, comments and their notes, and every chart and pivot cache
    /// naming it. A range the rows open inside grows, a merge with it, and
    /// the frozen split moves when the rows open above it. The change is
    /// all or nothing.
    ///
    /// ```
    /// use yggdryl::excel::{Cell, DateSystem, Formula, Workbook};
    ///
    /// let mut workbook = Workbook::new();
    /// let sheet = workbook.add_sheet("Data")?;
    /// sheet.set_cell("A1".parse()?, 1.0)?;
    /// sheet.set_cell("A2".parse()?, 2.0)?;
    /// let total = "A3".parse()?;
    /// sheet.insert_cell(
    ///     Cell::from_scalar(total, 3.0.into(), DateSystem::Year1900)?
    ///         .with_formula(Formula::from_file("SUM(A1:A2)", total)),
    /// )?;
    /// workbook.insert_rows("Data", 1, 2)?;
    /// let moved = workbook.sheet("Data")?.cell("A5".parse()?).expect("the total moved");
    /// assert_eq!(moved.formula().map(|f| f.at(moved.reference()).to_string()).as_deref(), Some("SUM(A1:A4)"));
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Adjacent preceding formats are inherited. Exterior border edges
    /// survive only where both neighbors agree; the diagonal and both
    /// direction flags must agree together. Inserting at the first row uses
    /// defaults. Default blank cells remain absent.
    ///
    /// Returns, before anything changes: [`Error::InvalidRecord`] for rows
    /// outside the grid, a cell the rows would push off it, rows opening
    /// inside an array formula, a data table or a pivot table, or a derived
    /// style exceeding [`super::MAX_CELL_FORMATS`]; and
    /// [`Error::Unsupported`] naming the part for a sheet carrying an
    /// element no edit here can carry through (`oleObjects`, `controls`,
    /// `scenarios`, an extension this crate does not model, ...), such an
    /// element of another sheet naming this one, or a table filled by a
    /// query, or inner/non-outline borders whose insertion is not modelled.
    pub fn insert_rows(&mut self, sheet: &str, at: u32, count: u32) -> Result<()> {
        self.shift_band(sheet, Axis::Rows, Band::Insert { at, count }, true)
            .map(|_| ())
    }

    /// Remove the rows `rows` of the sheet `sheet`, moving every row below
    /// up and every reference with it ([`Self::insert_rows`]): a reference
    /// to a removed cell is `#REF!`, a range the rows cut shrinks, one they
    /// take whole is `#REF!`; a merge inside them goes, a hyperlink, a
    /// comment or a conditional format on them goes, and a drawing anchored
    /// inside them moves to their first row.
    ///
    /// # Errors
    ///
    /// What [`Self::insert_rows`] returns, and [`Error::InvalidRecord`] for
    /// rows taking part of an array formula, a data table or a pivot table,
    /// or a table's header row, totals row or every data row.
    pub fn remove_rows(&mut self, sheet: &str, rows: Range<u32>) -> Result<()> {
        self.shift_band(
            sheet,
            Axis::Rows,
            Band::Remove {
                start: rows.start,
                end: rows.end,
            },
            true,
        )
        .map(|_| ())
    }

    /// Open `count` empty columns at zero-based `at` of the sheet `sheet`,
    /// as [`Self::insert_rows`] opens rows.
    ///
    /// # Errors
    ///
    /// What [`Self::insert_rows`] returns, and [`Error::InvalidRecord`] for
    /// columns opening inside a table.
    pub fn insert_columns(&mut self, sheet: &str, at: u32, count: u32) -> Result<()> {
        self.shift_band(sheet, Axis::Columns, Band::Insert { at, count }, true)
            .map(|_| ())
    }

    /// Remove the columns `columns` of the sheet `sheet`, as
    /// [`Self::remove_rows`] removes rows.
    ///
    /// # Errors
    ///
    /// What [`Self::remove_rows`] returns, and [`Error::InvalidRecord`] for
    /// columns meeting a table.
    pub fn remove_columns(&mut self, sheet: &str, columns: Range<u32>) -> Result<()> {
        self.shift_band(
            sheet,
            Axis::Columns,
            Band::Remove {
                start: columns.start,
                end: columns.end,
            },
            true,
        )
        .map(|_| ())
    }

    /// Open or close `band` along `axis` of the sheet `sheet`, every
    /// reference following, answering what the opposite band would not give
    /// back.
    pub(crate) fn shift_band(
        &mut self,
        sheet: &str,
        axis: Axis,
        band: Band,
        inherit: bool,
    ) -> Result<Restore> {
        let at = self.resolve(sheet).ok_or_else(|| self.absent(sheet))?;
        self.parsed(at)?;
        let limit = axis.limit();
        let refused = |reason: SmolStr| Error::InvalidRecord {
            path: format_smolstr!("{sheet}!{}", axis.noun()),
            reason,
        };
        match band {
            Band::Insert { count: 0, .. } => return Ok(Restore::new(self)),
            Band::Remove { start, end } if start >= end => return Ok(Restore::new(self)),
            Band::Insert { at: index, count } if index >= limit || count >= limit => {
                return Err(refused(format_smolstr!(
                    "expected {} opening within the {limit} of the grid, got {count} at {}",
                    axis.noun(),
                    index + 1
                )));
            }
            Band::Remove { end, .. } if end > limit => {
                return Err(refused(format_smolstr!(
                    "expected {} within the {limit} of the grid, got up to {end}",
                    axis.noun()
                )));
            }
            _ => {}
        }
        self.parse_all()?;
        let mut fresh = PartBytes::new();
        let related = self.check_band(at, axis, band, &mut fresh)?;
        let name = self.slots[at].name.clone();
        let shift = Shift::Band {
            sheet: &name,
            axis,
            band,
        };
        let plan = self.plan(&shift, &related, None, fresh)?;
        let key = self.slots[at].key;
        let mut restore = Restore::new(self);
        {
            let held = self.slots[at]
                .parsed
                .get()
                .expect("every sheet was parsed above");
            if let Band::Remove { start, end } = band {
                restore.push(Step::Cells {
                    key,
                    ranges: vec![axis.whole(start, end)],
                    slice: Box::new(held.band_cells(axis, start, end)),
                });
            }
            let layout = held.layout();
            let rows = match (axis, band) {
                (Axis::Rows, Band::Remove { start, end }) => Some(start..end),
                (Axis::Rows, Band::Insert { count, .. }) => {
                    Some(limit.saturating_sub(count)..limit)
                }
                (Axis::Columns, _) => None,
            };
            restore.push(Step::Layout {
                key,
                rows: rows.map(|rows| {
                    let formats = layout
                        .rows
                        .range(rows.clone())
                        .map(|(row, format)| (*row, format.clone()))
                        .collect();
                    (rows, formats)
                }),
                columns: (axis == Axis::Columns).then(|| layout.columns.0.clone()),
                merges: Some(layout.merges.clone()),
                pane: Some(layout.pane),
            });
        }
        let mut styles = self.styles()?;
        restore.capture_styles(styles.as_ref());
        let inheritance = if inherit {
            StylePlan::insertion(self.parsed(at)?, &mut styles, axis, band)?
        } else {
            StylePlan::default()
        };
        // All source, grid and style-capacity refusals precede publication.
        self.stated.remember_slot(&self.slots[at]);
        self.styles = OnceLock::from(styles);
        self.commit_band(at, &name, axis, band, plan, &mut restore);
        inheritance.apply(
            self.slots[at]
                .parsed
                .get_mut()
                .expect("the sheet was parsed above"),
            self.styles.get().expect("the styles were read above"),
        );
        Ok(restore)
    }

    fn commit_band(
        &mut self,
        at: usize,
        name: &str,
        axis: Axis,
        band: Band,
        plan: Plan,
        restore: &mut Restore,
    ) {
        let key = self.slots[at].key;
        let shift = Shift::Band {
            sheet: name,
            axis,
            band,
        };
        self.follow(&shift, plan, None, restore);
        let held = self.slots[at]
            .parsed
            .get_mut()
            .expect("every sheet was parsed above");
        let prior = held.rewrite_formula_ranges(|text| {
            let range: CellRange = text.parse().ok()?;
            let moved = adjust_range(range, name, &shift)?;
            (moved != range).then(|| SmolStr::new(range_text(moved)))
        });
        if !prior.is_empty() {
            restore.push(Step::FormulaAttributes { key, cells: prior });
        }
        held.shift_band(axis, band);
    }

    /// Refuse `band` along `axis` of the worksheet at `at` before anything
    /// changes: a cell pushed off the grid, an element no edit here carries
    /// through, an array formula, a data table, a pivot table or a table the
    /// band cuts, a table a query fills. Answers the parts beside the sheet
    /// the band moves references in - its tables, comments, drawings, notes
    /// and pivot tables - each read once, for [`Self::plan`] to rewrite.
    fn check_band(
        &self,
        at: usize,
        axis: Axis,
        band: Band,
        fresh: &mut PartBytes,
    ) -> Result<Vec<Related>> {
        let slot = &self.slots[at];
        let sheet = self.parsed(at)?;
        let part = slot.part.clone();
        let located = |range: CellRange, reason: SmolStr| Error::InvalidRecord {
            path: format_smolstr!("{}!{range}", slot.name),
            reason,
        };
        if let Band::Insert { at: index, count } = band {
            if let Some(cell) = sheet.pushed_off(axis, index, count) {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{}!{cell}", slot.name),
                    reason: format_smolstr!(
                        "expected the moved {} to stay within {} {}, got {cell} moving by {count}",
                        axis.noun(),
                        axis.limit(),
                        axis.noun()
                    ),
                });
            }
        }
        if let Some(frame) = sheet.frame() {
            if let Some(item) = frame
                .items
                .iter()
                .find(|item| item.class == super::carried::Class::Blocking)
            {
                let (operation, named) = blocked(axis, band, &item.name);
                return Err(Error::Unsupported {
                    operation,
                    filesystem: if named {
                        part
                    } else {
                        format_smolstr!("{part}#{}", item.name)
                    },
                });
            }
        }
        let cuts = |range: CellRange| {
            let (first, last) = axis.span(range);
            band.splits(first, last)
        };
        for (anchor, range) in sheet.formula_ranges() {
            if cuts(range) {
                return Err(located(
                    range,
                    format_smolstr!(
                        "expected {} leaving the array formula or data table anchored at {anchor} \
                         whole, got some of it",
                        axis.noun()
                    ),
                ));
            }
        }
        let mut related = Vec::new();
        for (kind, member) in self.relations_read(at, Some(fresh), None)? {
            if !matches!(
                kind,
                RelationshipKind::Table
                    | RelationshipKind::Comments
                    | RelationshipKind::ThreadedComment
                    | RelationshipKind::PivotTable
                    | RelationshipKind::Drawing
                    | RelationshipKind::VmlDrawing
            ) {
                continue;
            }
            let bytes = self.part_bytes(&member)?;
            match kind {
                RelationshipKind::Table => {
                    let table = Table::read(&bytes, &member)?;
                    if self
                        .relations_of(&member)?
                        .iter()
                        .any(|(kind, _)| *kind == RelationshipKind::QueryTable)
                    {
                        return Err(Error::unsupported(
                            "moving rows or columns through a table a query fills",
                            member,
                        ));
                    }
                    if let Some(reason) = table.refusal(axis, band) {
                        return Err(located(table.range, reason));
                    }
                }
                RelationshipKind::PivotTable => {
                    let location = shift::pivot_location(&bytes, &member)?;
                    let (first, last) = axis.span(location);
                    let refused = match band {
                        Band::Insert { .. } => band.splits(first, last),
                        Band::Remove { .. } => band.meets(first, last),
                    };
                    if refused {
                        return Err(located(
                            location,
                            format_smolstr!(
                                "expected {} leaving the pivot table of {member} where it is, got \
                                 some of it",
                                axis.noun()
                            ),
                        ));
                    }
                }
                _ => {}
            }
            related.push((kind, member, bytes));
        }
        Ok(related)
    }

    /// What `shift` rewrites beside the cells, worked out before anything
    /// changes: the carried children of every worksheet part but the one at
    /// `except`, the parts `related` to the sheet a band opens or closes -
    /// its tables, drawings, comments, notes and pivot tables - and every
    /// chart, pivot cache and drawing of the package naming a sheet the
    /// shift is about.
    ///
    /// # Errors
    ///
    /// Returns, before anything changes, [`Error::Unsupported`] naming the
    /// part and the element where a child this crate does not model might
    /// hold a reference the shift moves - on a sheet whose cells it moves,
    /// or naming a sheet it is about - and the refusal of a part the
    /// rewrite cannot read.
    fn plan(
        &self,
        shift: &Shift<'_>,
        related: &[Related],
        except: Option<usize>,
        mut fresh: PartBytes,
    ) -> Result<Plan> {
        let mut plan = Plan::default();
        let mut filter_body = false;
        for (index, slot) in self.slots.iter().enumerate() {
            if Some(index) == except {
                continue;
            }
            let Some(frame) = slot.parsed.get().and_then(Sheet::frame) else {
                continue;
            };
            let part = slot.part.clone();
            let own = shift.moves_cells_of(&slot.name);
            if let Some(item) = frame.items.iter().find(|item| {
                item.class == super::carried::Class::Blocking
                    && (own || shift.refers_in(&item.bytes))
            }) {
                return Err(Error::Unsupported {
                    operation: named_by(shift, own),
                    filesystem: format_smolstr!("{part}#{}", item.name),
                });
            }
            let mut validated_cf = false;
            let mut items = Vec::with_capacity(frame.items.len());
            let mut changed = false;
            for item in &frame.items {
                if item.class != super::carried::Class::Shifted
                    || (!own && !shift.may_name_in(&item.bytes))
                {
                    items.push(item.clone());
                    continue;
                }
                if matches!(shift, Shift::Move { .. })
                    && matches!(
                        item.name.as_str(),
                        "conditionalFormatting" | "dataValidations" | "extLst"
                    )
                {
                    if let Some(raw) = Registration::carried_item(frame, item)? {
                        let rewritten =
                            raw.formula_partition(&item.name, (&slot.name, None), shift, &part)?;
                        if rewritten.len() == 1 && rewritten[0].xml == raw.xml {
                            items.push(item.clone());
                        } else {
                            if !validated_cf {
                                let mut changed_cf = item.name == "conditionalFormatting";
                                if item.name == "extLst" {
                                    let cf_extensions =
                                        |registration: &Registration| -> Result<Vec<Registration>> {
                                            let mut found = Vec::new();
                                            for extension in registration.children_named(
                                                &[super::NAMESPACE, super::STRICT_NAMESPACE],
                                                "ext",
                                            )? {
                                                let Some(uri) = extension.root_attribute(b"uri")?
                                                else {
                                                    continue;
                                                };
                                                if super::carried::ShiftedExtension::from_uri(&uri)
                                                == Some(super::carried::ShiftedExtension::ConditionalFormatting) {
                                                found.push(extension);
                                            }
                                            }
                                            Ok(found)
                                        };
                                    let before = cf_extensions(&raw)?;
                                    let after = if let Some(registration) = rewritten.first() {
                                        cf_extensions(registration)?
                                    } else {
                                        Vec::new()
                                    };
                                    changed_cf = before.len() != after.len()
                                        || before
                                            .iter()
                                            .zip(&after)
                                            .any(|(before, after)| before.xml != after.xml);
                                }
                                if changed_cf {
                                    Self::carried_cf_max(frame, &part)?;
                                    validated_cf = true;
                                }
                            }
                            let root = Registration::frame_root(frame)?;
                            for entry in rewritten {
                                let mut child = item.clone();
                                child.bytes = entry
                                    .fragment(&root.namespaces, &root.markup)?
                                    .into_bytes()
                                    .into();
                                items.push(child);
                            }
                            changed = true;
                        }
                        continue;
                    }
                }
                let rewritten = if matches!(shift, Shift::Move { .. })
                    && matches!(item.name.as_str(), "autoFilter" | "sortState")
                {
                    let entry = Registration::carried_item(frame, item)?.ok_or_else(|| {
                        Error::InvalidRecord {
                            path: format_smolstr!("{part}#{}", item.name),
                            reason: "expected a worksheet range registration".into(),
                        }
                    })?;
                    let range = entry.cell_range(&item.name, &part)?;
                    filter_body |=
                        item.name == "autoFilter" && shift.cuts_filter_body(range, &slot.name);
                    entry.cut_worksheet_range(&item.name, range, &slot.name, shift, &part)?
                } else {
                    shift::SheetEdits::apply(&item.bytes, &slot.name, shift, &part, None)?
                };
                let mut item = item.clone();
                if let Some(bytes) = rewritten {
                    item.bytes = bytes.into();
                    changed = true;
                }
                items.push(item);
            }
            if changed {
                if validated_cf {
                    let mut rewritten = frame.clone();
                    rewritten.items = items;
                    Self::settle_carried_forks(&mut rewritten, &part)?;
                    items = rewritten.items;
                }
                plan.frames.push((index, items));
            }
        }
        if let Shift::Band { sheet, axis, band } = shift {
            for (kind, member, bytes) in related {
                let edit = |shift: &Shift<'_>| match kind {
                    RelationshipKind::Drawing => {
                        package::edit_document(bytes, &mut shift::DrawingEdits::new(shift, sheet))
                    }
                    RelationshipKind::VmlDrawing => {
                        shift::VmlEdits::apply(bytes, *axis, *band, member)
                    }
                    _ => shift::SheetEdits::apply(bytes, sheet, shift, member, None),
                };
                match edit(shift)? {
                    Some(edited) => plan.overrides.push(Rewritten {
                        member: member.clone(),
                        before: Some(Arc::clone(bytes)),
                        after: Some(Arc::from(edited)),
                    }),
                    // An anchor a removal leaves at its first row or column
                    // is one the opposite insertion moves: the undo puts the
                    // part back as it is.
                    None if matches!(band, Band::Remove { .. })
                        && matches!(
                            kind,
                            RelationshipKind::Drawing | RelationshipKind::VmlDrawing
                        ) =>
                    {
                        let opposite = Shift::Band {
                            sheet,
                            axis: *axis,
                            band: band.inverse(),
                        };
                        let moved = match kind {
                            RelationshipKind::Drawing => package::edit_document(
                                bytes,
                                &mut shift::DrawingEdits::new(&opposite, sheet),
                            )?,
                            _ => shift::VmlEdits::apply(bytes, *axis, band.inverse(), member)?,
                        };
                        if moved.is_some() {
                            plan.keep_part(member.clone(), Arc::clone(bytes));
                        }
                    }
                    None => {}
                }
            }
            if related.iter().any(|(_, member, _)| plan.contains(member)) {
                let index = self.resolve(sheet).expect("the band's owner was resolved");
                let member = package::relationships_part_of(&self.slots[index].part);
                let bytes = fresh
                    .get(&member)
                    .expect("the band check read its ownership document");
                plan.keep_part(member, Arc::clone(bytes));
            }
        }
        // The drawings of the sheets a cut moves cells of hold links naming
        // no sheet, which the cut moves too.
        let mut owned: Vec<(SmolStr, SmolStr)> = Vec::new();
        let mut sheet_relations = Vec::new();
        let mut has_notes = false;
        if matches!(shift, Shift::Move { .. }) {
            for (index, slot) in self.slots.iter().enumerate() {
                if shift.moves_cells_of(&slot.name) {
                    let relations =
                        self.relations_read(index, Some(&mut fresh), Some(&mut has_notes))?;
                    owned.extend(
                        relations
                            .iter()
                            .filter(|(kind, _)| *kind == RelationshipKind::Drawing)
                            .map(|(_, member)| (member.clone(), slot.name.clone())),
                    );
                    sheet_relations.push((slot.key, relations));
                }
            }
        }
        let known_sheet = match shift {
            Shift::Band { sheet, .. } => Some(*sheet),
            _ => None,
        };
        let referring = self.referring(related, known_sheet, &sheet_relations, &mut fresh)?;
        let moved_tables = self.check_table_move(shift, &referring, &mut fresh)?;
        for entry in referring.iter() {
            if related.iter().any(|(_, member, _)| *member == entry.member)
                || (entry.kind == Referrer::Cache && matches!(shift, Shift::RemoveSheet { .. }))
            {
                continue;
            }
            let owner = match entry.kind {
                Referrer::Table(key) => {
                    let Some((index, slot)) = self
                        .slots
                        .iter()
                        .enumerate()
                        .find(|(_, slot)| slot.key == key)
                    else {
                        continue;
                    };
                    if Some(index) == except {
                        continue;
                    }
                    Some(slot.name.as_str())
                }
                _ => owned
                    .iter()
                    .find(|(member, _)| *member == entry.member)
                    .map(|(_, sheet)| sheet.as_str()),
            };
            let own = owner.is_some_and(|sheet| shift.moves_cells_of(sheet));
            let held = self
                .stated
                .overrides
                .get(&entry.member)
                .and_then(|held| held.bytes.as_ref());
            let names = match (own, held, &entry.references) {
                (true, _, _) => true,
                (false, Some(bytes), _) => shift.may_name_in(bytes),
                (false, None, Some(references)) => shift.may_name(references),
                (false, None, None) => true,
            };
            if !names {
                continue;
            }
            let bytes = match fresh.get(&entry.member) {
                Some(bytes) => Arc::clone(bytes),
                None => self.part_bytes(&entry.member)?,
            };
            if !own && held.is_none() && entry.references.is_none() && !shift.may_name_in(&bytes) {
                continue;
            }
            let edited = match entry.kind {
                Referrer::Chart => {
                    package::edit_document(&bytes, &mut shift::ChartEdits { shift })?
                }
                Referrer::Cache => package::edit_document(
                    &bytes,
                    &mut shift::CacheEdits {
                        shift,
                        part: &entry.member,
                    },
                )?,
                // A drawing's links name their sheet or the sheet the
                // drawing is on: a band of that sheet moves them as a
                // related part, a cut here as one it owns.
                Referrer::Drawing => package::edit_document(
                    &bytes,
                    &mut shift::DrawingEdits::new(shift, owner.unwrap_or("")),
                )?,
                Referrer::Table(_) => shift::SheetEdits::apply(
                    &bytes,
                    owner.expect("a table's sheet was resolved above"),
                    shift,
                    &entry.member,
                    None,
                )?,
            };
            if let Some(edited) = edited {
                if let Some((member, bytes)) = entry.ownership(self)? {
                    plan.keep_part(member, bytes);
                }
                plan.overrides.push(Rewritten {
                    member: entry.member.clone(),
                    before: Some(bytes),
                    after: Some(Arc::from(edited)),
                });
            }
        }
        if filter_body {
            for (index, defined) in self
                .stated
                .names
                .iter()
                .enumerate()
                .filter(|(_, name)| name.name().eq_ignore_ascii_case("_xlnm._FilterDatabase"))
            {
                let Some(scope) = defined.scope().and_then(|key| self.sheet_by_key(key)) else {
                    continue;
                };
                if let Some(formula) =
                    shift.filter_database(defined.formula(), scope, &self.workbook_part)?
                {
                    plan.filter_names.push((index, formula));
                }
            }
        }
        self.transfer_carried(shift, &fresh, &mut plan)?;
        self.transfer_tables(shift, &moved_tables, &fresh, &mut plan)?;
        if has_notes {
            self.transfer_notes(shift, &mut fresh, &mut plan)?;
        }
        Ok(plan)
    }

    /// Refuse a whole table landing over another table before any part is
    /// rewritten. Read each involved member once for both this check and
    /// the subsequent rewrite; tables moving together keep their offsets.
    fn check_table_move(
        &self,
        shift: &Shift<'_>,
        referring: &[Referring],
        fresh: &mut PartBytes,
    ) -> Result<Vec<SmolStr>> {
        let Shift::Move {
            from, block, to, ..
        } = *shift
        else {
            return Ok(Vec::new());
        };
        let source = self.sheet_key(from).expect("a cut's source was resolved");
        let target = self.sheet_key(to).expect("a cut's target was resolved");
        let mut read = |entry: &Referring| -> Result<Table> {
            let bytes = match fresh.entry(entry.member.clone()) {
                std::collections::hash_map::Entry::Occupied(held) => held.into_mut(),
                std::collections::hash_map::Entry::Vacant(vacant) => {
                    vacant.insert(self.part_bytes(&entry.member)?)
                }
            };
            Table::read(bytes, &entry.member)
        };
        let mut moved = Vec::new();
        for entry in referring
            .iter()
            .filter(|entry| entry.kind == Referrer::Table(source))
        {
            let table = read(entry)?;
            if block.encloses(table.range) {
                let (_, start) = shift
                    .place(from, table.range.start())
                    .expect("a whole table is inside the checked cut");
                let range = table.range.moved_to(start);
                moved.push((&entry.member, table, range));
            }
        }
        if moved.is_empty() {
            return Ok(Vec::new());
        }
        for entry in referring
            .iter()
            .filter(|entry| entry.kind == Referrer::Table(target))
        {
            if moved.iter().any(|(member, _, _)| **member == entry.member) {
                continue;
            }
            let stationary = read(entry)?;
            for (_, table, range) in &moved {
                if range.intersects(stationary.range) {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{to}!{range}"),
                        reason: format_smolstr!(
                            "expected the moved table {} to avoid other tables, got {} at {}",
                            table.name,
                            stationary.name,
                            stationary.range
                        ),
                    });
                }
            }
        }
        Ok(moved
            .into_iter()
            .map(|(member, _, _)| member.clone())
            .collect())
    }

    /// Transfer the package ownership of whole tables after every payload
    /// and collision check succeeds, still before any workbook mutation.
    fn frame_relationship_ids(frame: &super::carried::WorksheetFrame) -> Result<BTreeSet<SmolStr>> {
        let mut bytes = frame.root.to_vec();
        for item in &frame.items {
            bytes.extend_from_slice(&item.bytes);
        }
        bytes.extend_from_slice(format!("</{}>", frame.root_name).as_bytes());
        let mut ids = BTreeSet::new();
        Registration::select(&bytes, |reader, start, _, position| {
            for attribute in start.attributes() {
                let attribute =
                    attribute.map_err(|error| package::codec_error(position, error.to_string()))?;
                let (namespace, name) = reader.resolver().resolve_attribute(attribute.key);
                if name.as_ref() == b"id"
                    && Registration::in_namespace(
                        namespace,
                        &[
                            super::RELATIONSHIPS_NAMESPACE,
                            super::STRICT_RELATIONSHIPS_NAMESPACE,
                        ],
                    )?
                {
                    let value = attribute
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|error| package::codec_error(position, error.to_string()))?;
                    ids.insert(SmolStr::new(value));
                }
            }
            Ok(None)
        })?;
        Ok(ids)
    }
    fn same_cf_owner(left: &str, right: &str, part: &str) -> Result<bool> {
        let parse = |text: &str| -> Result<BTreeSet<CellRange>> {
            let mut ranges = BTreeSet::new();
            for token in text.split_whitespace() {
                let range = token.parse().map_err(|_| Error::InvalidRecord {
                    path: format_smolstr!("{part}#sqref"),
                    reason: format_smolstr!("expected worksheet ranges, got {text:?}"),
                })?;
                ranges.insert(range);
            }
            if ranges.is_empty() {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{part}#sqref"),
                    reason: "expected at least one worksheet range".into(),
                });
            }
            Ok(ranges)
        };
        Ok(parse(left)? == parse(right)?)
    }

    fn x14_cf_guids(
        extension: &Registration,
        part: &str,
    ) -> Result<BTreeMap<crate::Uuid, SmolStr>> {
        use super::carried::{X14_NAMESPACE, XM_NAMESPACE};
        let mut guids = BTreeMap::new();
        let list = extension.one_child(&[X14_NAMESPACE], "conditionalFormattings", part)?;
        for host in list.children_named(&[X14_NAMESPACE], "conditionalFormatting")? {
            let mut owner = None;
            for rule in host.children_named(&[X14_NAMESPACE], "cfRule")? {
                // MS-XLSX ignores id when the x14 rule owns a priority.
                if rule.root_attribute(b"priority")?.is_some() {
                    continue;
                }
                let Some(id) = rule.root_attribute(b"id")? else {
                    continue;
                };
                let guid = ThreadedNotes::guid(&id, part, "x14 cfRule id")?;
                if owner.is_none() {
                    owner = Some(
                        host.one_child(&[XM_NAMESPACE], "sqref", part)?
                            .plain_text(part)?,
                    );
                }
                if guids
                    .insert(guid, SmolStr::new(owner.as_deref().unwrap()))
                    .is_some()
                {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{part}#x14:cfRule"),
                        reason: format_smolstr!("expected a unique x14 cfRule GUID, got {id}"),
                    });
                }
            }
        }
        Ok(guids)
    }

    fn legacy_cf_guids(held: &Registration, part: &str) -> Result<BTreeMap<crate::Uuid, SmolStr>> {
        let main = &[super::NAMESPACE, super::STRICT_NAMESPACE];
        let mut guids = BTreeMap::new();
        for rule in held.children_named(main, "cfRule")? {
            for list in rule.children_named(main, "extLst")? {
                for extension in list.children_named(main, "ext")? {
                    if !extension.root_attribute(b"uri")?.is_some_and(|uri| {
                        uri.eq_ignore_ascii_case("{B025F937-C7B1-47D3-B67F-A62EFF666E3E}")
                    }) {
                        continue;
                    }
                    let id = extension
                        .one_child(&[super::carried::X14_NAMESPACE], "id", part)?
                        .plain_text(part)?;
                    let guid = ThreadedNotes::guid(&id, part, "x14:id")?;
                    let owner =
                        held.root_attribute(b"sqref")?
                            .ok_or_else(|| Error::InvalidRecord {
                                path: format_smolstr!("{part}#conditionalFormatting"),
                                reason: "expected a conditional-format sqref".into(),
                            })?;
                    if guids.insert(guid, SmolStr::new(&owner)).is_some() {
                        return Err(Error::InvalidRecord {
                            path: format_smolstr!("{part}#cfRule/x14:id"),
                            reason: format_smolstr!("expected a unique linked GUID, got {id}"),
                        });
                    }
                }
            }
        }
        Ok(guids)
    }

    /// IDs on priorityless x14 rules link them to legacy conditional formats.
    /// A partial cross-sheet move keeps the source pair and forks one new pair
    /// for the destination; the two XML representations receive the same ID.
    fn frame_cf_guid_sets(
        frame: &super::carried::WorksheetFrame,
        part: &str,
    ) -> Result<(BTreeSet<crate::Uuid>, BTreeSet<crate::Uuid>)> {
        use super::carried::ShiftedExtension;
        let main = &[super::NAMESPACE, super::STRICT_NAMESPACE];
        let mut legacy = BTreeSet::new();
        let mut x14 = BTreeSet::new();
        for item in &frame.items {
            if !matches!(item.name.as_str(), "conditionalFormatting" | "extLst") {
                continue;
            }
            let Some(held) = Registration::carried_item(frame, item)? else {
                continue;
            };
            if item.name == "conditionalFormatting" {
                for guid in Self::legacy_cf_guids(&held, part)?.into_keys() {
                    if !legacy.insert(guid) {
                        return Err(Error::InvalidRecord {
                            path: format_smolstr!("{part}#cfRule/x14:id"),
                            reason: format_smolstr!("expected a unique linked GUID, got {guid}"),
                        });
                    }
                }
            } else if item.name == "extLst" {
                for extension in held.children_named(main, "ext")? {
                    let Some(uri) = extension.root_attribute(b"uri")? else {
                        continue;
                    };
                    if ShiftedExtension::from_uri(&uri)
                        != Some(ShiftedExtension::ConditionalFormatting)
                    {
                        continue;
                    }
                    // x14_cf_guids excludes prioritized rules by the MS-XLSX rule.
                    for guid in Self::x14_cf_guids(&extension, part)?.into_keys() {
                        if !x14.insert(guid) {
                            return Err(Error::InvalidRecord {
                                path: format_smolstr!("{part}#x14:cfRule"),
                                reason: format_smolstr!("expected a unique x14 GUID, got {guid}"),
                            });
                        }
                    }
                }
            }
        }
        Ok((legacy, x14))
    }

    fn fork_x14_guids(
        extension: Registration,
        forks: &BTreeMap<crate::Uuid, crate::Uuid>,
        part: &str,
    ) -> Result<Registration> {
        use super::carried::X14_NAMESPACE;
        if forks.is_empty() {
            return Ok(extension);
        }
        let list = extension.one_child(&[X14_NAMESPACE], "conditionalFormattings", part)?;
        let mut hosts = BTreeMap::new();
        for host in list.children_named(&[X14_NAMESPACE], "conditionalFormatting")? {
            let mut rules = BTreeMap::new();
            for rule in host.children_named(&[X14_NAMESPACE], "cfRule")? {
                if rule.root_attribute(b"priority")?.is_some() {
                    continue;
                }
                let Some(id) = rule.root_attribute(b"id")? else {
                    continue;
                };
                let old = ThreadedNotes::guid(&id, part, "x14 cfRule id")?;
                let Some(&new) = forks.get(&old) else {
                    continue;
                };
                rules.insert(
                    rule.key.clone(),
                    Some(rule.with_attributes(vec![("id".into(), Some(format!("{{{new}}}")))])?),
                );
            }
            if !rules.is_empty() {
                hosts.insert(host.key.clone(), Some(host.changed_children(&rules)?));
            }
        }
        if hosts.is_empty() {
            return Ok(extension);
        }
        let list = list.changed_children(&hosts)?;
        let mut changes = BTreeMap::new();
        changes.insert(list.key.clone(), Some(list));
        extension.changed_children(&changes)
    }

    fn fork_legacy_guids(
        held: Registration,
        forks: &BTreeMap<crate::Uuid, crate::Uuid>,
        part: &str,
    ) -> Result<Registration> {
        if forks.is_empty() {
            return Ok(held);
        }
        let main = &[super::NAMESPACE, super::STRICT_NAMESPACE];
        let mut rules = BTreeMap::new();
        for rule in held.children_named(main, "cfRule")? {
            let mut lists = BTreeMap::new();
            for list in rule.children_named(main, "extLst")? {
                let mut extensions = BTreeMap::new();
                for extension in list.children_named(main, "ext")? {
                    if !extension.root_attribute(b"uri")?.is_some_and(|uri| {
                        uri.eq_ignore_ascii_case("{B025F937-C7B1-47D3-B67F-A62EFF666E3E}")
                    }) {
                        continue;
                    }
                    let id = extension.one_child(&[super::carried::X14_NAMESPACE], "id", part)?;
                    let old = ThreadedNotes::guid(&id.plain_text(part)?, part, "x14:id")?;
                    let Some(&new) = forks.get(&old) else {
                        continue;
                    };
                    let id = id.with_text(format!("{{{new}}}"))?;
                    let mut changed = BTreeMap::new();
                    changed.insert(id.key.clone(), Some(id));
                    extensions.insert(
                        extension.key.clone(),
                        Some(extension.changed_children(&changed)?),
                    );
                }
                if !extensions.is_empty() {
                    lists.insert(list.key.clone(), Some(list.changed_children(&extensions)?));
                }
            }
            if !lists.is_empty() {
                rules.insert(rule.key.clone(), Some(rule.changed_children(&lists)?));
            }
        }
        held.changed_children(&rules)
    }

    /// The worksheet-wide priority domain includes legacy and x14 rules.
    fn carried_cf_max(frame: &super::carried::WorksheetFrame, part: &str) -> Result<u32> {
        use super::carried::{ShiftedExtension, X14_NAMESPACE};
        let main = &[super::NAMESPACE, super::STRICT_NAMESPACE];
        let mut maximum = 0_u32;
        let mut seen = BTreeSet::new();
        let mut priority = |rule: &Registration, required: bool| -> Result<()> {
            let value = rule.root_attribute(b"priority")?;
            let Some(value) = value else {
                if required {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{part}#cfRule"),
                        reason: "expected a conditional-format priority".into(),
                    });
                }
                return Ok(());
            };
            let value: u32 = value.parse().map_err(|_| Error::InvalidRecord {
                path: format_smolstr!("{part}#cfRule"),
                reason: "expected an integer conditional-format priority".into(),
            })?;
            if value == 0 || !seen.insert(value) {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{part}#cfRule"),
                    reason: format_smolstr!(
                        "expected a unique positive conditional-format priority, got {value}"
                    ),
                });
            }
            maximum = maximum.max(value);
            Ok(())
        };
        for item in &frame.items {
            if item.name == "conditionalFormatting" {
                let Some(held) = Registration::carried_item(frame, item)? else {
                    continue;
                };
                for rule in held.children_named(main, "cfRule")? {
                    priority(&rule, true)?;
                }
            } else if item.name == "extLst" {
                let Some(held) = Registration::carried_item(frame, item)? else {
                    continue;
                };
                for extension in held.children_named(main, "ext")? {
                    let Some(uri) = extension.root_attribute(b"uri")? else {
                        continue;
                    };
                    if ShiftedExtension::from_uri(&uri)
                        != Some(ShiftedExtension::ConditionalFormatting)
                    {
                        continue;
                    }
                    let list =
                        extension.one_child(&[X14_NAMESPACE], "conditionalFormattings", part)?;
                    for host in list.children_named(&[X14_NAMESPACE], "conditionalFormatting")? {
                        for rule in host.children_named(&[X14_NAMESPACE], "cfRule")? {
                            priority(&rule, false)?;
                        }
                    }
                }
            }
        }
        Ok(maximum)
    }

    /// One XML rewrite for the worksheet's legacy and x14 priority domain.
    fn rewrite_cf_priorities(
        frame: &mut super::carried::WorksheetFrame,
        part: &str,
        mut reprioritize: impl FnMut(Registration) -> Result<Registration>,
    ) -> Result<()> {
        use super::carried::{ShiftedExtension, X14_NAMESPACE};
        let main = &[super::NAMESPACE, super::STRICT_NAMESPACE];
        let originals = frame.items.clone();
        for item in originals {
            if item.name == "conditionalFormatting" {
                let Some(held) = Registration::carried_item(frame, &item)? else {
                    continue;
                };
                let mut changes = BTreeMap::new();
                for rule in held.children_named(main, "cfRule")? {
                    changes.insert(rule.key.clone(), Some(reprioritize(rule)?));
                }
                let updated = held.changed_children(&changes)?;
                let root = Registration::frame_root(frame)?;
                let xml = updated.fragment(&root.namespaces, &root.markup)?;
                frame.set_child(
                    "conditionalFormatting",
                    Some(&item.bytes),
                    Some(xml.into_bytes().into()),
                );
            } else if item.name == "extLst" {
                let Some(held) = Registration::carried_item(frame, &item)? else {
                    continue;
                };
                let mut extensions = BTreeMap::new();
                for extension in held.children_named(main, "ext")? {
                    let Some(uri) = extension.root_attribute(b"uri")? else {
                        continue;
                    };
                    if ShiftedExtension::from_uri(&uri)
                        != Some(ShiftedExtension::ConditionalFormatting)
                    {
                        continue;
                    }
                    let list =
                        extension.one_child(&[X14_NAMESPACE], "conditionalFormattings", part)?;
                    let mut hosts = BTreeMap::new();
                    for host in list.children_named(&[X14_NAMESPACE], "conditionalFormatting")? {
                        let mut rules = BTreeMap::new();
                        for rule in host.children_named(&[X14_NAMESPACE], "cfRule")? {
                            rules.insert(rule.key.clone(), Some(reprioritize(rule)?));
                        }
                        hosts.insert(host.key.clone(), Some(host.changed_children(&rules)?));
                    }
                    let list = list.changed_children(&hosts)?;
                    let mut change = BTreeMap::new();
                    change.insert(list.key.clone(), Some(list));
                    extensions.insert(
                        extension.key.clone(),
                        Some(extension.changed_children(&change)?),
                    );
                }
                if !extensions.is_empty() {
                    let updated = held.changed_children(&extensions)?;
                    let root = Registration::frame_root(frame)?;
                    let xml = updated.fragment(&root.namespaces, &root.markup)?;
                    frame.set_child("extLst", Some(&item.bytes), Some(xml.into_bytes().into()));
                }
            }
        }
        Ok(())
    }

    /// A split retained rule keeps its first priority and inserts later
    /// fragments immediately after it. Later original gaps remain gaps.
    /// The original frame has already passed `carried_cf_max` before split.
    fn settle_carried_forks(frame: &mut super::carried::WorksheetFrame, part: &str) -> Result<()> {
        use super::carried::{ShiftedExtension, X14_NAMESPACE};
        let main = &[super::NAMESPACE, super::STRICT_NAMESPACE];
        let mut counts = BTreeMap::<u32, u32>::new();
        let mut record = |rule: Registration| -> Result<()> {
            let Some(value) = rule.root_attribute(b"priority")? else {
                return Ok(()); // a linked x14 companion uses its legacy priority
            };
            let priority: u32 = value.parse().map_err(|_| Error::InvalidRecord {
                path: format_smolstr!("{part}#cfRule"),
                reason: "expected an integer conditional-format priority".into(),
            })?;
            if priority == 0 {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{part}#cfRule"),
                    reason: "expected a positive conditional-format priority, got 0".into(),
                });
            }
            let count = counts.entry(priority).or_default();
            *count = count.checked_add(1).ok_or_else(|| Error::Unsupported {
                operation: "counting retained conditional-format forks",
                filesystem: format_smolstr!("{part}#cfRule"),
            })?;
            Ok(())
        };
        for item in &frame.items {
            if item.name == "conditionalFormatting" {
                let Some(held) = Registration::carried_item(frame, item)? else {
                    continue;
                };
                for rule in held.children_named(main, "cfRule")? {
                    record(rule)?;
                }
            } else if item.name == "extLst" {
                let Some(held) = Registration::carried_item(frame, item)? else {
                    continue;
                };
                for extension in held.children_named(main, "ext")? {
                    let Some(uri) = extension.root_attribute(b"uri")? else {
                        continue;
                    };
                    if ShiftedExtension::from_uri(&uri)
                        != Some(ShiftedExtension::ConditionalFormatting)
                    {
                        continue;
                    }
                    let list =
                        extension.one_child(&[X14_NAMESPACE], "conditionalFormattings", part)?;
                    for host in list.children_named(&[X14_NAMESPACE], "conditionalFormatting")? {
                        for rule in host.children_named(&[X14_NAMESPACE], "cfRule")? {
                            record(rule)?;
                        }
                    }
                }
            }
        }
        if !counts.values().any(|count| *count > 1) {
            return Ok(());
        }
        let mut bases = BTreeMap::new();
        let mut extra = 0_u32;
        for (&priority, &count) in &counts {
            let base = priority
                .checked_add(extra)
                .ok_or_else(|| Error::Unsupported {
                    operation: "allocating retained conditional-format fork priorities",
                    filesystem: format_smolstr!("{part}#cfRule"),
                })?;
            base.checked_add(count - 1)
                .ok_or_else(|| Error::Unsupported {
                    operation: "allocating retained conditional-format fork priorities",
                    filesystem: format_smolstr!("{part}#cfRule"),
                })?;
            bases.insert(priority, base);
            extra = extra
                .checked_add(count - 1)
                .ok_or_else(|| Error::Unsupported {
                    operation: "allocating retained conditional-format fork priorities",
                    filesystem: format_smolstr!("{part}#cfRule"),
                })?;
        }
        let mut seen = BTreeMap::<u32, u32>::new();
        Self::rewrite_cf_priorities(frame, part, |rule| {
            let Some(value) = rule.root_attribute(b"priority")? else {
                return Ok(rule);
            };
            let priority: u32 = value.parse().map_err(|_| Error::InvalidRecord {
                path: format_smolstr!("{part}#cfRule"),
                reason: "expected an integer conditional-format priority".into(),
            })?;
            let ordinal = seen.entry(priority).or_default();
            let updated = bases[&priority] + *ordinal;
            *ordinal += 1;
            if updated == priority {
                return Ok(rule);
            }
            rule.with_attributes(vec![("priority".into(), Some(updated.to_string()))])
        })
    }

    /// One priority domain for legacy and x14 rules. Incoming rules keep their
    /// source order at the front; destination priorities retain their gaps.
    fn settle_cf_priorities(
        frame: &mut super::carried::WorksheetFrame,
        pairs: &[(u32, u32)],
        part: &str,
    ) -> Result<()> {
        if pairs.is_empty() {
            return Ok(());
        }
        let count = u32::try_from(pairs.len()).map_err(|_| Error::Unsupported {
            operation: "allocating moved conditional-format priorities",
            filesystem: format_smolstr!("{part}#cfRule"),
        })?;
        let mut order = pairs.to_vec();
        order.sort_by_key(|(source, _)| *source);
        let first = pairs
            .iter()
            .map(|(_, temporary)| *temporary)
            .min()
            .expect("a nonempty priority set has a first temporary priority");
        let base = first.checked_sub(1).ok_or_else(|| Error::InvalidRecord {
            path: format_smolstr!("{part}#cfRule"),
            reason: "expected positive moved priority".into(),
        })?;
        let mut ranks = BTreeMap::new();
        for (rank, (_, temporary)) in order.iter().enumerate() {
            let rank = u32::try_from(rank + 1).expect("rank is bounded by count");
            if ranks.insert(*temporary, rank).is_some() {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{part}#cfRule"),
                    reason: "expected distinct temporary priorities".into(),
                });
            }
        }
        let reprioritize = |rule: Registration| -> Result<Registration> {
            let Some(priority) = rule.root_attribute(b"priority")? else {
                return Ok(rule); // linked x14 dataBar rules have a legacy priority owner
            };
            let priority: u32 = priority.parse().map_err(|_| Error::InvalidRecord {
                path: format_smolstr!("{part}#cfRule"),
                reason: "expected an integer conditional-format priority".into(),
            })?;
            let updated = if let Some(rank) = ranks.get(&priority) {
                *rank
            } else if priority <= base {
                priority
                    .checked_add(count)
                    .ok_or_else(|| Error::Unsupported {
                        operation: "shifting destination conditional-format priorities",
                        filesystem: format_smolstr!("{part}#cfRule"),
                    })?
            } else {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{part}#cfRule"),
                    reason: format_smolstr!("expected a recorded moved priority, got {priority}"),
                });
            };
            rule.with_attributes(vec![("priority".into(), Some(updated.to_string()))])
        };
        Self::rewrite_cf_priorities(frame, part, reprioritize)
    }

    /// Preserve source x14 rule precedence above all destination rules.
    fn repriority_x14(
        extension: Registration,
        maximum: &mut u32,
        pairs: &mut Vec<(u32, u32)>,
        part: &str,
    ) -> Result<Registration> {
        use super::carried::X14_NAMESPACE;
        let list = extension.one_child(&[X14_NAMESPACE], "conditionalFormattings", part)?;
        let hosts = list.children_named(&[X14_NAMESPACE], "conditionalFormatting")?;
        let mut ordered = Vec::new();
        for host in &hosts {
            for rule in host.children_named(&[X14_NAMESPACE], "cfRule")? {
                let Some(value) = rule.root_attribute(b"priority")? else {
                    continue;
                };
                let value: u32 = value.parse().map_err(|_| Error::InvalidRecord {
                    path: format_smolstr!("{part}#cfRule"),
                    reason: "expected an integer conditional-format priority".into(),
                })?;
                if value == 0 {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{part}#cfRule"),
                        reason: format_smolstr!("expected a positive x14 priority, got {value}"),
                    });
                }
                ordered.push((value, host.key.clone(), rule));
            }
        }
        if ordered.is_empty() {
            return Ok(extension);
        }
        ordered.sort_by_key(|(priority, _, _)| *priority);
        let mut rules: BTreeMap<SmolStr, BTreeMap<SmolStr, Option<Registration>>> = BTreeMap::new();
        for (source_priority, host, rule) in ordered {
            *maximum = maximum.checked_add(1).ok_or_else(|| Error::Unsupported {
                operation: "allocating a moved conditional-format priority",
                filesystem: format_smolstr!("{part}#cfRule"),
            })?;
            pairs.push((source_priority, *maximum));
            rules.entry(host).or_default().insert(
                rule.key.clone(),
                Some(rule.with_attributes(vec![("priority".into(), Some(maximum.to_string()))])?),
            );
        }
        let mut changed_hosts = BTreeMap::new();
        for host in hosts {
            if let Some(changes) = rules.remove(&host.key) {
                changed_hosts.insert(host.key.clone(), Some(host.changed_children(&changes)?));
            }
        }
        let list = list.changed_children(&changed_hosts)?;
        let mut changes = BTreeMap::new();
        changes.insert(list.key.clone(), Some(list));
        extension.changed_children(&changes)
    }

    /// Move only selected x14 hosts. Each known ext is isolated under its
    /// original extLst before the existing SheetEdits partition pass, so an
    /// unrelated extension can never hitchhike into the destination.
    fn transfer_x14(
        &self,
        shift: &Shift<'_>,
        plan: &mut Plan,
        original: &super::carried::WorksheetFrame,
        source: usize,
        destination: usize,
    ) -> Result<X14Transfer> {
        use super::carried::{ShiftedExtension, X14_NAMESPACE};
        let Shift::Move { from, to, .. } = shift else {
            return Ok(X14Transfer::default());
        };
        let source_part = &self.slots[source].part;
        let destination_part = &self.slots[destination].part;
        let main = &[super::NAMESPACE, super::STRICT_NAMESPACE];
        let mut moved = Vec::new();
        let mut selected_guid_owners = BTreeMap::new();
        for item in original.items.iter().filter(|item| item.name == "extLst") {
            let Some(parent) = Registration::carried_item(original, item)? else {
                continue;
            };
            let extensions = parent.children_named(main, "ext")?;
            for extension in &extensions {
                let Some(uri) = extension.root_attribute(b"uri")? else {
                    continue;
                };
                let Some(kind) = ShiftedExtension::from_uri(&uri) else {
                    continue;
                };
                if kind == ShiftedExtension::Sparkline {
                    extension.x14_transfer_only(kind, source_part)?;
                }
                let mut remove = BTreeMap::new();
                for other in &extensions {
                    if other.key != extension.key {
                        remove.insert(other.key.clone(), None);
                    }
                }
                let isolated = parent.changed_children(&remove)?;
                let edited = if matches!(
                    kind,
                    ShiftedExtension::ConditionalFormatting | ShiftedExtension::DataValidation
                ) {
                    let selected = isolated.formula_partition(
                        "extLst",
                        (from, Some(to)),
                        shift,
                        source_part,
                    )?;
                    Some(
                        selected
                            .first()
                            .map_or_else(Vec::new, |entry| entry.xml.as_bytes().to_vec()),
                    )
                } else {
                    shift::SheetEdits::apply(
                        isolated.xml.as_bytes(),
                        from,
                        shift,
                        source_part,
                        Some(to),
                    )?
                };
                if edited.as_ref().is_some_and(Vec::is_empty) {
                    continue;
                }
                // `None` means selected ownership moved without changing a
                // coordinate or byte; the same fact as for legacy CF/DV.
                let xml = edited.unwrap_or_else(|| isolated.xml.as_bytes().to_vec());
                let mut item = item.clone();
                item.bytes = xml.into();
                let parent = Registration::carried_item(original, &item)?
                    .ok_or_else(|| package::codec_error(0, "expected moved x14 extension"))?;
                let mut selected = parent.children_named(main, "ext")?;
                if selected.len() != 1 {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{source_part}#extLst"),
                        reason: "expected one moved x14 extension".into(),
                    });
                }
                let selected = selected.remove(0);
                if kind != ShiftedExtension::Sparkline {
                    selected.x14_transfer_only(kind, source_part)?;
                }
                if kind == ShiftedExtension::ConditionalFormatting {
                    let source_guids = Self::x14_cf_guids(extension, source_part)?;
                    for (guid, _) in Self::x14_cf_guids(&selected, source_part)? {
                        let owner =
                            source_guids
                                .get(&guid)
                                .ok_or_else(|| Error::InvalidRecord {
                                    path: format_smolstr!("{source_part}#x14:cfRule"),
                                    reason: "expected selected GUID in source extension".into(),
                                })?;
                        if selected_guid_owners.insert(guid, owner.clone()).is_some() {
                            return Err(Error::InvalidRecord {
                                path: format_smolstr!("{source_part}#x14:cfRule"),
                                reason: "expected unique moved x14 GUID".into(),
                            });
                        }
                    }
                }
                moved.push((kind, parent, selected));
            }
        }
        if moved.is_empty() {
            return Ok(X14Transfer::default());
        }
        let mut frame = plan.frame(self, destination);
        let mut forks = BTreeMap::new();
        if !selected_guid_owners.is_empty() {
            let source_frame = plan.frame(self, source);
            let (source_legacy, source_x14) = Self::frame_cf_guid_sets(&source_frame, source_part)?;
            let (destination_legacy, destination_x14) =
                Self::frame_cf_guid_sets(&frame, destination_part)?;
            for &guid in selected_guid_owners.keys() {
                if source_x14.contains(&guid) != source_legacy.contains(&guid) {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{source_part}#cfRule/x14:id"),
                        reason: format_smolstr!(
                            "expected linked legacy and x14 GUID {guid} to retain the same source ownership"
                        ),
                    });
                }
            }
            let mut occupied = source_legacy;
            occupied.extend(source_x14.iter().copied());
            occupied.extend(destination_legacy.iter().copied());
            occupied.extend(destination_x14.iter().copied());
            occupied.extend(selected_guid_owners.keys().copied());
            for &guid in selected_guid_owners.keys() {
                if !source_x14.contains(&guid) {
                    continue;
                }
                let mut seed = guid.get();
                let replacement = loop {
                    seed = seed.wrapping_add(1);
                    let candidate = crate::Uuid::from_v8(seed);
                    if occupied.insert(candidate) {
                        break candidate;
                    }
                };
                forks.insert(guid, replacement);
            }
            for item in &frame.items {
                if item.name == "conditionalFormatting" {
                    if let Some(held) = Registration::carried_item(&frame, item)? {
                        for guid in Self::legacy_cf_guids(&held, destination_part)?.keys() {
                            if selected_guid_owners
                                .keys()
                                .any(|old| forks.get(old).unwrap_or(old) == guid)
                            {
                                return Err(Error::InvalidRecord {
                                    path: format_smolstr!("{destination_part}#cfRule/x14:id"),
                                    reason: "expected a GUID absent from the destination".into(),
                                });
                            }
                        }
                    }
                } else if item.name == "extLst" {
                    if let Some(held) = Registration::carried_item(&frame, item)? {
                        for extension in held.children_named(main, "ext")? {
                            if extension
                                .root_attribute(b"uri")?
                                .as_deref()
                                .and_then(ShiftedExtension::from_uri)
                                != Some(ShiftedExtension::ConditionalFormatting)
                            {
                                continue;
                            }
                            for guid in Self::x14_cf_guids(&extension, destination_part)?.keys() {
                                if selected_guid_owners
                                    .keys()
                                    .any(|old| forks.get(old).unwrap_or(old) == guid)
                                {
                                    return Err(Error::InvalidRecord {
                                        path: format_smolstr!("{destination_part}#x14:cfRule"),
                                        reason: "expected a GUID absent from the destination"
                                            .into(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        let family = |frame: &super::carried::WorksheetFrame| -> Result<SmolStr> {
            let root = Registration::frame_root(frame)?;
            let key = frame
                .root_name
                .split_once(':')
                .map(|(prefix, _)| format_smolstr!("xmlns:{prefix}"))
                .unwrap_or_else(|| SmolStr::new_static("xmlns"));
            root.namespaces
                .get(&key)
                .cloned()
                .ok_or_else(|| Error::InvalidRecord {
                    path: frame.root_name.clone(),
                    reason: "expected a worksheet root namespace".into(),
                })
        };
        if family(original)? != family(&frame)? {
            return Err(Error::Unsupported {
                operation: "cross-sheet transfer of x14 across worksheet namespace families",
                filesystem: source_part.clone(),
            });
        }
        let mut destination_ext = Registration::carried_child(&frame, "extLst")?;
        let mut maximum_priority = Self::carried_cf_max(&frame, destination_part)?;
        let mut priority_pairs = Vec::new();
        for (kind, mut moved_parent, mut extension) in moved {
            if kind == ShiftedExtension::ConditionalFormatting {
                extension = Self::repriority_x14(
                    extension,
                    &mut maximum_priority,
                    &mut priority_pairs,
                    source_part,
                )?;
                extension = Self::fork_x14_guids(extension, &forks, source_part)?;
                let mut changed = BTreeMap::new();
                changed.insert(extension.key.clone(), Some(extension.clone()));
                moved_parent = moved_parent.changed_children(&changed)?;
            }
            let uri = extension
                .root_attribute(b"uri")?
                .ok_or_else(|| Error::InvalidRecord {
                    path: format_smolstr!("{source_part}#ext"),
                    reason: "expected a known x14 extension URI".into(),
                })?;
            let next = if let Some(parent) = destination_ext.as_ref() {
                let extensions = parent.children_named(main, "ext")?;
                let mut same = Vec::new();
                for held in extensions {
                    if held.root_attribute(b"uri")?.as_deref() == Some(uri.as_str()) {
                        same.push(held);
                    }
                }
                if same.len() > 1 {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{destination_part}#extLst"),
                        reason: format_smolstr!("expected one extension with URI {uri}"),
                    });
                }
                if let Some(existing) = same.pop() {
                    let (container, child) = kind.container();
                    if existing.container_semantics()? != extension.container_semantics()? {
                        return Err(Error::Unsupported {
                            operation: "merging x14 extension settings",
                            filesystem: format_smolstr!("{destination_part}#ext[{uri}]"),
                        });
                    }
                    let source_list =
                        extension.one_child(&[X14_NAMESPACE], container, source_part)?;
                    let added = source_list.children_named(&[X14_NAMESPACE], child)?;
                    let target_list =
                        existing.one_child(&[X14_NAMESPACE], container, destination_part)?;
                    if source_list.container_semantics()? != target_list.container_semantics()? {
                        return Err(Error::Unsupported {
                            operation: "merging x14 container settings",
                            filesystem: format_smolstr!("{destination_part}#{container}"),
                        });
                    }
                    let tails = target_list.children_named(&[X14_NAMESPACE], "extLst")?;
                    let mut joined = if let Some(tail) = tails.first() {
                        target_list.inserted_before(&added, &tail.key)?
                    } else {
                        target_list.appended(&added)?
                    };
                    if kind == ShiftedExtension::DataValidation {
                        let count = joined.children_named(&[X14_NAMESPACE], child)?.len();
                        joined = joined
                            .with_attributes(vec![("count".into(), Some(count.to_string()))])?;
                    }
                    let mut change = BTreeMap::new();
                    change.insert(target_list.key, Some(joined));
                    let existing = existing.changed_children(&change)?;
                    change.clear();
                    change.insert(existing.key.clone(), Some(existing));
                    parent.changed_children(&change)?
                } else {
                    parent.appended(&[extension])?
                }
            } else {
                moved_parent
            };
            let root = Registration::frame_root(&frame)?;
            let xml = next.fragment(&root.namespaces, &root.markup)?;
            frame.set_child(
                "extLst",
                destination_ext.as_ref().map(|old| old.xml.as_bytes()),
                Some(xml.into_bytes().into()),
            );
            destination_ext = Registration::carried_child(&frame, "extLst")?;
        }
        plan.set_frame(destination, frame.items);
        Ok(X14Transfer {
            priority_pairs,
            selected_guid_owners,
            forks,
        })
    }

    /// Transfer worksheet-owned registrations using the same SheetEdits pass
    /// that removed them from the source. The source frame is read before the
    /// plan's rewritten frame so a dropped child is still available here.
    fn transfer_carried(
        &self,
        shift: &Shift<'_>,
        cached: &PartBytes,
        plan: &mut Plan,
    ) -> Result<()> {
        let Shift::Move { from, to, .. } = shift else {
            return Ok(());
        };
        if super::formula::reference::same_sheet(from, to) {
            return Ok(());
        }
        let source = self.resolve(from).expect("the cut source is resolved");
        let destination = self.resolve(to).expect("the cut destination is resolved");
        let Some(original) = self.slots[source].parsed.get().and_then(Sheet::frame) else {
            return Ok(());
        };
        let source_part = &self.slots[source].part;
        let X14Transfer {
            mut priority_pairs,
            selected_guid_owners,
            forks,
        } = self.transfer_x14(shift, plan, original, source, destination)?;
        // No destination frame or relationship document is read unless a
        // supported source registration actually has a moved partition.
        let mut moved_items = BTreeMap::new();
        let mut selected_legacy_guids = BTreeSet::new();
        for (index, item) in original.items.iter().enumerate() {
            if !matches!(
                item.name.as_str(),
                "conditionalFormatting"
                    | "dataValidations"
                    | "hyperlinks"
                    | "protectedRanges"
                    | "ignoredErrors"
                    | "cellWatches"
            ) {
                continue;
            }
            let Some(raw) = Registration::carried_item(original, item)? else {
                continue;
            };
            let moved: SmallVec<[Registration; 1]> = if matches!(
                item.name.as_str(),
                "conditionalFormatting" | "dataValidations"
            ) {
                SmallVec::from_vec(raw.formula_partition(
                    &item.name,
                    (from, Some(to)),
                    shift,
                    source_part,
                )?)
            } else {
                match shift::SheetEdits::apply(&item.bytes, from, shift, source_part, Some(to))? {
                    Some(bytes) if bytes.is_empty() => SmallVec::new(),
                    Some(bytes) => {
                        let mut rendered = item.clone();
                        rendered.bytes = bytes.into();
                        Registration::carried_item(original, &rendered)?
                            .into_iter()
                            .collect()
                    }
                    None => std::iter::once(raw.clone()).collect(),
                }
            };
            if moved.is_empty() {
                continue;
            }
            let linked = if item.name == "conditionalFormatting" {
                Self::legacy_cf_guids(&raw, source_part)?
            } else {
                BTreeMap::new()
            };
            for (guid, owner) in &linked {
                let Some(x14_owner) = selected_guid_owners.get(guid) else {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{source_part}#cfRule/x14:id"),
                        reason: "expected a selected x14 rule with the same GUID".into(),
                    });
                };
                if !Self::same_cf_owner(owner, x14_owner, source_part)? {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{source_part}#cfRule/x14:id"),
                        reason: "expected linked legacy and x14 rules to own the same sqref".into(),
                    });
                }
            }
            raw.standard_transfer_only(source_part, !linked.is_empty())?;
            selected_legacy_guids.extend(linked.keys().copied());
            for (fragment, moved) in moved.into_iter().enumerate() {
                let moved = if item.name == "conditionalFormatting" {
                    Self::fork_legacy_guids(moved, &forks, source_part)?
                } else {
                    moved
                };
                moved_items.insert((index, fragment), moved);
            }
        }
        if !selected_guid_owners.is_empty() {
            for item in original
                .items
                .iter()
                .filter(|item| item.name == "conditionalFormatting")
            {
                if let Some(held) = Registration::carried_item(original, item)? {
                    for (guid, owner) in Self::legacy_cf_guids(&held, source_part)? {
                        if let Some(x14_owner) = selected_guid_owners.get(&guid) {
                            if !selected_legacy_guids.contains(&guid)
                                || !Self::same_cf_owner(&owner, x14_owner, source_part)?
                            {
                                return Err(Error::InvalidRecord {
                                    path: format_smolstr!("{source_part}#cfRule/x14:id"),
                                    reason: "expected linked legacy and x14 rules to move together"
                                        .into(),
                                });
                            }
                        }
                    }
                }
            }
        }
        if moved_items.is_empty() {
            if !priority_pairs.is_empty() {
                let destination_part = &self.slots[destination].part;
                let mut frame = plan.frame(self, destination);
                Self::settle_cf_priorities(&mut frame, &priority_pairs, destination_part)?;
                plan.set_frame(destination, frame.items);
            }
            return Ok(());
        }
        let destination_part = &self.slots[destination].part;
        let mut target_frame = plan.frame(self, destination);
        let mut target_changed = false;
        let mut maximum_priority = if moved_items
            .keys()
            .any(|index| original.items[index.0].name == "conditionalFormatting")
        {
            Self::carried_cf_max(&target_frame, destination_part)?
        } else {
            0
        };

        // Priorities are global to the worksheet, even when its rules are
        // spread across several conditionalFormatting registrations.
        let mut ordered_rules = Vec::new();
        for (index, moved) in &moved_items {
            if original.items[index.0].name != "conditionalFormatting" {
                continue;
            }
            for rule in
                moved.children_named(&[super::NAMESPACE, super::STRICT_NAMESPACE], "cfRule")?
            {
                let priority =
                    rule.root_attribute(b"priority")?
                        .ok_or_else(|| Error::InvalidRecord {
                            path: format_smolstr!("{source_part}#cfRule"),
                            reason: "expected a conditional-format priority".into(),
                        })?;
                let priority: u32 = priority.parse().map_err(|_| Error::InvalidRecord {
                    path: format_smolstr!("{source_part}#cfRule"),
                    reason: "expected an integer conditional-format priority".into(),
                })?;
                if priority == 0 {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{source_part}#cfRule"),
                        reason: format_smolstr!(
                            "expected a positive conditional-format priority, got {priority}"
                        ),
                    });
                }
                ordered_rules.push((priority, *index, rule));
            }
        }
        ordered_rules.sort_unstable_by_key(|(priority, index, _)| (*priority, *index));
        let mut priority_changes: BTreeMap<
            (usize, usize),
            BTreeMap<SmolStr, Option<Registration>>,
        > = BTreeMap::new();
        for (source_priority, index, rule) in ordered_rules {
            maximum_priority =
                maximum_priority
                    .checked_add(1)
                    .ok_or_else(|| Error::Unsupported {
                        operation: "allocating a moved conditional-format priority",
                        filesystem: format_smolstr!("{destination_part}#cfRule"),
                    })?;
            priority_pairs.push((source_priority, maximum_priority));
            priority_changes.entry(index).or_default().insert(
                rule.key.clone(),
                Some(rule.with_attributes(vec![(
                    "priority".into(),
                    Some(maximum_priority.to_string()),
                )])?),
            );
        }
        for (index, changes) in priority_changes {
            let held = moved_items
                .get_mut(&index)
                .expect("a moved rule belongs to a format");
            *held = held.changed_children(&changes)?;
        }

        // Only hyperlink registrations can add a relationship. These are
        // loaded lazily, once, and the pending Plan image wins over disk.
        let source_rels = package::relationships_part_of(source_part);
        let destination_rels = package::relationships_part_of(destination_part);
        let mut source_rel_bytes: Option<Arc<[u8]>> = None;
        let mut destination_rel_bytes: Option<Arc<[u8]>> = None;
        let mut source_rel_document: Option<Registration> = None;
        let mut destination_rel_document: Option<Registration> = None;
        let mut used_ids = BTreeSet::new();
        let mut removed_ids = BTreeSet::new();
        let mut added_relations = Vec::new();
        let mut link_ids: BTreeMap<SmolStr, SmolStr> = BTreeMap::new();

        for (index, mut moved) in moved_items {
            let name = original.items[index.0].name.as_str();
            if name == "conditionalFormatting" {
                let root = Registration::frame_root(&target_frame)?;
                let xml = moved.fragment(&root.namespaces, &root.markup)?;
                target_frame.set_child(
                    "conditionalFormatting",
                    None,
                    Some(xml.into_bytes().into()),
                );
                target_changed = true;
                continue;
            }

            let (container_name, child_name) = match name {
                "hyperlinks" => ("hyperlinks", "hyperlink"),
                "dataValidations" => ("dataValidations", "dataValidation"),
                "protectedRanges" => ("protectedRanges", "protectedRange"),
                "ignoredErrors" => ("ignoredErrors", "ignoredError"),
                "cellWatches" => ("cellWatches", "cellWatch"),
                _ => unreachable!("the carried transfer selected a supported container"),
            };
            let mut children =
                moved.children_named(&[super::NAMESPACE, super::STRICT_NAMESPACE], child_name)?;
            if children.is_empty() {
                continue;
            }
            // A source container extension has no cell extent proving
            // which partition owns it. Do not duplicate or discard it.
            if name == "ignoredErrors"
                && !moved
                    .children_named(&[super::NAMESPACE, super::STRICT_NAMESPACE], "extLst")?
                    .is_empty()
            {
                return Err(Error::Unsupported {
                    operation: "moving ignored-error container extensions",
                    filesystem: format_smolstr!("{source_part}#ignoredErrors/extLst"),
                });
            }
            if name == "hyperlinks" {
                let has_link = children.iter().try_fold(false, |any, child| {
                    Ok::<bool, Error>(
                        any || child.optional_relationship_key(source_part)?.is_some(),
                    )
                })?;
                if has_link && source_rel_document.is_none() {
                    source_rel_bytes = plan.part(self, &source_rels, cached)?;
                    destination_rel_bytes = plan.part(self, &destination_rels, cached)?;
                    if let Some(bytes) = source_rel_bytes.as_deref() {
                        source_rel_document = Some(Registration::root(bytes)?);
                    }
                    destination_rel_document = Some(Registration::root(
                        destination_rel_bytes
                            .as_deref()
                            .unwrap_or(package::TEMPLATE_WORKBOOK_RELATIONSHIPS.as_bytes()),
                    )?);
                    let destination_relations = Relationships::from_xml(
                        destination_rel_bytes
                            .as_deref()
                            .unwrap_or(package::TEMPLATE_WORKBOOK_RELATIONSHIPS.as_bytes()),
                        destination_part,
                    )?;
                    used_ids.extend(
                        destination_relations
                            .entries()
                            .iter()
                            .map(|entry| entry.id.clone()),
                    );
                }
                let Some(source_bytes) = source_rel_bytes
                    .as_deref()
                    .or_else(|| (!has_link).then_some(&b""[..]))
                else {
                    return Err(Error::InvalidRecord {
                        path: source_rels.clone(),
                        reason: "expected the hyperlink relationship part".into(),
                    });
                };
                let relations = if has_link {
                    Some(Relationships::from_xml(source_bytes, source_part)?)
                } else {
                    None
                };
                let entries = if has_link {
                    source_rel_document
                        .as_ref()
                        .expect("loaded above")
                        .members(false)?
                } else {
                    Vec::new()
                };
                let mut changes = BTreeMap::new();
                for child in &children {
                    let Some((qname, old_id)) = child.optional_relationship_key(source_part)?
                    else {
                        continue;
                    };
                    let new_id = match link_ids.get(&old_id) {
                        Some(held) => held.clone(),
                        None => {
                            let relationship = relations
                                .as_ref()
                                .expect("a linked child loaded relationships")
                                .by_id(&old_id)
                                .ok_or_else(|| Error::InvalidRecord {
                                    path: format_smolstr!("{source_part}#hyperlink"),
                                    reason: format_smolstr!("expected relationship {old_id}"),
                                })?;
                            let entry = entries
                                .iter()
                                .find(|entry| entry.key == old_id)
                                .ok_or_else(|| Error::InvalidRecord {
                                    path: source_rels.clone(),
                                    reason: format_smolstr!(
                                        "expected relationship registration {old_id}"
                                    ),
                                })?;
                            let kind = entry.root_attribute(b"Type")?.unwrap_or_default();
                            if kind != format!("{}/hyperlink", super::RELATIONSHIPS_NAMESPACE)
                                && kind
                                    != format!(
                                        "{}/hyperlink",
                                        super::STRICT_RELATIONSHIPS_NAMESPACE
                                    )
                            {
                                return Err(Error::InvalidRecord {
                                    path: format_smolstr!("{source_rels}#{old_id}"),
                                    reason: "expected a hyperlink relationship".into(),
                                });
                            }
                            let mut id = old_id.clone();
                            let mut next = 1_u32;
                            while !used_ids.insert(id.clone()) {
                                id = format_smolstr!("rIdHyperlink{next}");
                                next = next.checked_add(1).ok_or_else(|| Error::Unsupported {
                                    operation: "allocating a hyperlink relationship id",
                                    filesystem: destination_rels.clone(),
                                })?;
                            }
                            let mut attributes = vec![("Id".into(), Some(id.to_string()))];
                            if let Some(target) = &relationship.target {
                                attributes.push((
                                    "Target".into(),
                                    Some(package::relative_to(
                                        package::folder_of(destination_part),
                                        target,
                                    )),
                                ));
                            }
                            let mut copied = entry.with_attributes(attributes)?;
                            copied.key = id.clone();
                            added_relations.push(copied);
                            removed_ids.insert(old_id.clone());
                            link_ids.insert(old_id.clone(), id.clone());
                            id
                        }
                    };
                    if new_id != old_id {
                        changes.insert(
                            child.key.clone(),
                            Some(child.with_attributes(vec![(qname, Some(new_id.to_string()))])?),
                        );
                    }
                }
                moved = moved.changed_children(&changes)?;
                children = moved
                    .children_named(&[super::NAMESPACE, super::STRICT_NAMESPACE], child_name)?;
            }

            let held = Registration::carried_child(&target_frame, name)?;
            let root = Registration::frame_root(&target_frame)?;
            let after = match held.as_ref() {
                None => moved.fragment(&root.namespaces, &root.markup)?,
                Some(existing) => {
                    if matches!(
                        name,
                        "dataValidations" | "protectedRanges" | "ignoredErrors" | "cellWatches"
                    ) && moved.container_semantics()? != existing.container_semantics()?
                    {
                        return Err(Error::Unsupported {
                            operation: "merging carried container attributes",
                            filesystem: format_smolstr!("{destination_part}#{name}"),
                        });
                    }
                    let mut combined = if name == "ignoredErrors" {
                        let tails = existing.children_named(
                            &[super::NAMESPACE, super::STRICT_NAMESPACE],
                            "extLst",
                        )?;
                        match tails.as_slice() {
                            [] => existing.appended(&children)?,
                            [tail] => existing.inserted_before(&children, &tail.key)?,
                            _ => {
                                return Err(Error::InvalidRecord {
                                    path: format_smolstr!("{destination_part}#ignoredErrors"),
                                    reason: "expected at most one extension list".into(),
                                });
                            }
                        }
                    } else {
                        existing.appended(&children)?
                    };
                    if name == "dataValidations" {
                        let count = combined
                            .children_named(
                                &[super::NAMESPACE, super::STRICT_NAMESPACE],
                                child_name,
                            )?
                            .len();
                        combined = combined
                            .with_attributes(vec![("count".into(), Some(count.to_string()))])?;
                    }
                    combined.xml.to_string()
                }
            };
            target_frame.set_child(
                container_name,
                held.as_ref().map(|entry| entry.xml.as_bytes()),
                Some(after.into_bytes().into()),
            );
            target_changed = true;
        }
        if !priority_pairs.is_empty() {
            Self::settle_cf_priorities(&mut target_frame, &priority_pairs, destination_part)?;
            target_changed = true;
        }
        if target_changed {
            plan.set_frame(destination, target_frame.items);
        }

        if !added_relations.is_empty() {
            // A relationship still used by another source element stays; an
            // unused one leaves with its hyperlink. Either image is retained
            // as an inverse witness.
            let source_frame = plan.frame(self, source);
            let source_uses = Self::frame_relationship_ids(&source_frame)?;
            removed_ids.retain(|id| !source_uses.contains(id));
            let source_before = source_rel_bytes.expect("a moved relationship loaded its source");
            if removed_ids.is_empty() {
                plan.keep_part(source_rels.clone(), Arc::clone(&source_before));
            } else {
                let source_after = source_rel_document
                    .expect("loaded above")
                    .without(false, &removed_ids)?;
                plan.set_part(
                    source_rels.clone(),
                    Some(Arc::clone(&source_before)),
                    Some(Registration::replace_root(&source_before, &source_after.xml)?.into()),
                );
            }
            let destination_before = destination_rel_bytes;
            let destination_document = destination_rel_document.expect("loaded above");
            let destination_after = destination_document.appended(&added_relations)?;
            let seed = destination_before
                .as_deref()
                .unwrap_or(package::TEMPLATE_WORKBOOK_RELATIONSHIPS.as_bytes());
            let bytes = Registration::replace_root(seed, &destination_after.xml)?;
            if destination_before.is_none() {
                plan.ensure_content_type(
                    self,
                    &destination_rels,
                    package::RELATIONSHIPS_CONTENT_TYPE,
                    cached,
                )?;
            }
            plan.set_part(destination_rels, destination_before, Some(bytes.into()));
        }
        Ok(())
    }
    fn transfer_tables(
        &self,
        shift: &Shift<'_>,
        moved: &[SmolStr],
        cached: &PartBytes,
        plan: &mut Plan,
    ) -> Result<()> {
        let Shift::Move { from, to, .. } = *shift else {
            return Ok(());
        };
        if moved.is_empty() || super::formula::reference::same_sheet(from, to) {
            return Ok(());
        }
        let source = self.resolve(from).expect("a cut's source was resolved");
        let destination = self.resolve(to).expect("a cut's destination was resolved");
        let source_part = &self.slots[source].part;
        let destination_part = &self.slots[destination].part;
        let source_rels = package::relationships_part_of(source_part);
        let destination_rels = package::relationships_part_of(destination_part);
        let source_bytes = plan.part(self, &source_rels, cached)?.ok_or_else(|| {
            package::codec_error(0, "expected the transferred table's relationships part")
        })?;
        let destination_before = plan.part(self, &destination_rels, cached)?;
        let source_document = Registration::root(&source_bytes)?;
        let destination_document = Registration::root(
            destination_before
                .as_deref()
                .unwrap_or(package::TEMPLATE_WORKBOOK_RELATIONSHIPS.as_bytes()),
        )?;
        let relations = Relationships::from_xml(&source_bytes, source_part)?;
        let mut used: BTreeSet<_> = Relationships::from_xml(
            destination_before
                .as_deref()
                .unwrap_or(package::TEMPLATE_WORKBOOK_RELATIONSHIPS.as_bytes()),
            destination_part,
        )?
        .entries()
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
        let source_entries = source_document.members(false)?;
        let mut source_frame = plan.frame(self, source);
        let mut destination_frame = plan.frame(self, destination);
        let source_container = Registration::carried_child(&source_frame, "tableParts")?
            .ok_or_else(|| {
                package::codec_error(0, "expected the transferred table's membership")
            })?;
        let held_destination = Registration::carried_child(&destination_frame, "tableParts")?;
        let destination_container = match &held_destination {
            Some(container) => container.clone(),
            None => {
                // Parse the new child in the existing root's exact scope,
                // including Strict/prefixed roots, through the same parser.
                let mut temporary = destination_frame.clone();
                temporary.set_child(
                    "tableParts",
                    None,
                    Some(
                        format!("<{}tableParts count=\"0\"/>", temporary.prefix())
                            .into_bytes()
                            .into(),
                    ),
                );
                Registration::carried_child(&temporary, "tableParts")?
                    .expect("the model wrote tableParts")
            }
        };
        let memberships = source_container.members(true)?;
        let mut removed = BTreeSet::new();
        let mut added_memberships = Vec::new();
        let mut added_relations = Vec::new();
        for membership in &memberships {
            let relation = relations
                .by_id(&membership.key)
                .expect("plan validated every active membership");
            let member = relation
                .target
                .as_ref()
                .expect("an active table has an internal target");
            if !moved.contains(member) {
                continue;
            }
            let mut id = membership.key.clone();
            let mut next = 1_u32;
            while !used.insert(id.clone()) {
                id = format_smolstr!("rIdTable{next}");
                next = next.checked_add(1).ok_or_else(|| Error::InvalidRecord {
                    path: destination_rels.clone(),
                    reason: SmolStr::new_static(
                        "expected a free relationship id, got an exhausted sequence",
                    ),
                })?;
            }
            let relation = source_entries
                .iter()
                .find(|entry| entry.key == membership.key)
                .ok_or_else(|| package::codec_error(0, "expected the table's OPC registration"))?;
            let mut relation = relation.with_attributes(vec![
                ("Id".into(), Some(id.to_string())),
                (
                    "Target".into(),
                    Some(package::relative_to(
                        package::folder_of(destination_part),
                        member,
                    )),
                ),
            ])?;
            relation.key = id.clone();
            added_relations.push(relation);
            added_memberships.push(membership.with_table_key(&id)?);
            removed.insert(membership.key.clone());
        }
        if removed.len() != moved.len() {
            return Err(Error::InvalidRecord {
                path: source_part.clone(),
                reason: SmolStr::new_static(
                    "expected each moved table to have one active membership",
                ),
            });
        }
        let remaining = memberships.len() - removed.len();
        source_frame.set_child(
            "tableParts",
            Some(source_container.xml.as_bytes()),
            if remaining == 0 {
                None
            } else {
                Some(
                    source_container
                        .without(true, &removed)?
                        .with_attributes(vec![("count".into(), Some(remaining.to_string()))])?
                        .xml
                        .as_bytes()
                        .into(),
                )
            },
        );
        let count = destination_container.members(true)?.len() + added_memberships.len();
        destination_frame.set_child(
            "tableParts",
            held_destination.as_ref().map(|entry| entry.xml.as_bytes()),
            Some(
                destination_container
                    .appended(&added_memberships)?
                    .with_attributes(vec![("count".into(), Some(count.to_string()))])?
                    .xml
                    .as_bytes()
                    .into(),
            ),
        );
        plan.set_frame(source, source_frame.items);
        plan.set_frame(destination, destination_frame.items);
        let source_after = source_document.without(false, &removed)?;
        let destination_after = destination_document.appended(&added_relations)?;
        // Root extraction deliberately excludes declarations and epilogues.
        // Splice only that root into each full .rels envelope.
        plan.set_part(
            source_rels,
            Some(Arc::clone(&source_bytes)),
            Some(Registration::replace_root(&source_bytes, &source_after.xml)?.into()),
        );
        let destination_bytes = destination_before
            .as_deref()
            .unwrap_or(package::TEMPLATE_WORKBOOK_RELATIONSHIPS.as_bytes());
        let after = Registration::replace_root(destination_bytes, &destination_after.xml)?;
        if destination_before.is_none() {
            plan.ensure_content_type(
                self,
                &destination_rels,
                package::RELATIONSHIPS_CONTENT_TYPE,
                cached,
            )?;
        }
        plan.set_part(destination_rels, destination_before, Some(after.into()));
        Ok(())
    }
    /// Read only the two worksheets' current ownership documents. Comment
    /// relationships are implicit; VML is selected by the actual legacyDrawing.
    fn note_bundle(&self, index: usize, cached: &mut PartBytes, plan: &Plan) -> Result<NoteBundle> {
        let part = &self.slots[index].part;
        let rels = package::relationships_part_of(part);
        let bytes = plan.part(self, &rels, cached)?;
        let text = bytes
            .as_deref()
            .unwrap_or(package::TEMPLATE_WORKBOOK_RELATIONSHIPS.as_bytes());
        let document = Registration::root(text)?;
        let entries = document.members(false)?;
        let mut keys = BTreeSet::new();
        for entry in &entries {
            if entry.key.is_empty() || !keys.insert(entry.key.clone()) {
                return Err(Error::InvalidRecord {
                    path: rels.clone(),
                    reason: "expected unique nonempty relationship ids".into(),
                });
            }
        }
        let relationships = Relationships::from_xml(text, part)?;
        let frame = plan.frame(self, index);
        let legacy = Registration::carried_child(&frame, "legacyDrawing")?;
        let legacy_id = legacy
            .as_ref()
            .map(|entry| entry.relationship_key(part))
            .transpose()?
            .map(|(_, id)| id);
        let mut comments = None;
        let mut threads = None;
        let mut drawing = None;
        for entry in &entries {
            let kind = entry.attribute(b"Type")?.unwrap_or_default();
            let comment = [
                super::RELATIONSHIPS_NAMESPACE,
                super::STRICT_RELATIONSHIPS_NAMESPACE,
            ]
            .iter()
            .any(|ns| kind == format!("{ns}/comments"));
            let vml = legacy_id.as_ref() == Some(&entry.key);
            let threaded = kind == THREADED_RELATIONSHIP;
            if !threaded
                && relationships
                    .by_id(&entry.key)
                    .is_some_and(|relation| relation.kind == RelationshipKind::ThreadedComment)
            {
                return Err(Error::unsupported(
                    "cutting cells with an unrecognized threaded comment relationship family",
                    format_smolstr!("{rels}#{}", entry.key),
                ));
            }
            if !comment && !vml && !threaded {
                continue;
            }
            if vml
                && ![
                    super::RELATIONSHIPS_NAMESPACE,
                    super::STRICT_RELATIONSHIPS_NAMESPACE,
                ]
                .iter()
                .any(|namespace| kind == format!("{namespace}/vmlDrawing"))
            {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{rels}#{}", entry.key),
                    reason: "expected the VML drawing relationship type".into(),
                });
            }
            let found: Vec<_> = relationships
                .entries()
                .iter()
                .filter(|relation| relation.id == entry.key)
                .collect();
            if found.len() != 1
                || found[0].target.is_none()
                || (vml && found[0].kind != RelationshipKind::VmlDrawing)
            {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{rels}#{}", entry.key),
                    reason: "expected one internal relationship of the selected note kind".into(),
                });
            }
            let member = found[0].target.as_ref().expect("internal target").clone();
            if comment {
                let payload = plan
                    .part(self, &member, cached)?
                    .ok_or_else(|| Error::absent("note part", &member))?;
                if comments.is_some() {
                    return Err(Error::InvalidRecord {
                        path: rels.clone(),
                        reason: "expected at most one legacy comments relationship".into(),
                    });
                }
                cached
                    .entry(member.clone())
                    .or_insert_with(|| Arc::clone(&payload));
                comments = Some((entry.key.clone(), LegacyNotes::read(member, payload)?));
            } else if threaded {
                if threads.replace((entry.key.clone(), member)).is_some() {
                    return Err(ThreadedNotes::invalid(
                        &rels,
                        "expected at most one threaded comments relationship",
                    ));
                }
            } else {
                drawing = Some((entry.key.clone(), member));
            }
        }
        if legacy.is_some() && drawing.is_none() {
            return Err(Error::InvalidRecord {
                path: part.clone(),
                reason: "expected legacyDrawing to resolve to a VML part".into(),
            });
        }
        let threads = threads
            .map(|(id, member)| {
                let payload = plan
                    .part(self, &member, cached)?
                    .ok_or_else(|| Error::absent("threaded comment part", &member))?;
                cached
                    .entry(member.clone())
                    .or_insert_with(|| Arc::clone(&payload));
                ThreadedNotes::read(member, payload, comments.as_ref().map(|(_, notes)| notes))
                    .map(|notes| (id, notes))
            })
            .transpose()?;
        Ok(NoteBundle {
            rels,
            bytes,
            document,
            entries,
            comments,
            threads,
            drawing,
            legacy,
        })
    }

    /// The shared registry stays workbook-owned; only the two affected thread
    /// parts' resolved person identities are checked, with one payload read.
    fn check_thread_people(
        &self,
        bundles: &[Option<&NoteBundle>],
        book_bytes: &[u8],
        relationships: &Relationships,
        cached: &mut PartBytes,
        plan: &Plan,
    ) -> Result<()> {
        if bundles
            .iter()
            .flatten()
            .all(|bundle| bundle.threads.is_none())
        {
            return Ok(());
        }
        let mut member = None;
        for entry in Registration::root(book_bytes)?.members(false)? {
            if entry.attribute(b"Type")?.as_deref() != Some(PERSON_RELATIONSHIP) {
                continue;
            }
            let mut matching = relationships
                .entries()
                .iter()
                .filter(|relation| relation.id == entry.key);
            let target = matching
                .next()
                .and_then(|relation| relation.target.as_ref())
                .ok_or_else(|| {
                    ThreadedNotes::invalid(
                        &self.workbook_part,
                        "expected an internal persons relationship",
                    )
                })?;
            if matching.next().is_some() || member.replace(target.clone()).is_some() {
                return Err(ThreadedNotes::invalid(
                    &self.workbook_part,
                    "expected one workbook persons relationship",
                ));
            }
        }
        let member = member.ok_or_else(|| {
            ThreadedNotes::invalid(
                &self.workbook_part,
                "expected a workbook persons relationship",
            )
        })?;
        let bytes = plan
            .part(self, &member, cached)?
            .ok_or_else(|| Error::absent("threaded comment persons", &member))?;
        cached
            .entry(member.clone())
            .or_insert_with(|| Arc::clone(&bytes));
        let root = Registration::root(&bytes)?;
        if !root.is_element(&[THREADED_NAMESPACE], "personList")? {
            return Err(ThreadedNotes::invalid(
                &member,
                "expected a personList root",
            ));
        }
        let mut people = BTreeMap::new();
        for person in root.children_named(&[THREADED_NAMESPACE], "person")? {
            let id = ThreadedNotes::required_guid(&person, &member, "id")?;
            let picker = person.attribute(b"providerId")?.as_deref() == Some("PeoplePicker");
            if people.insert(id, picker).is_some() {
                return Err(ThreadedNotes::invalid(
                    &member,
                    "expected unique person IDs",
                ));
            }
        }
        for notes in bundles
            .iter()
            .flatten()
            .filter_map(|bundle| bundle.threads.as_ref())
        {
            for (id, mentioned) in &notes.1.people {
                if !people.get(id).is_some_and(|picker| !mentioned || *picker) {
                    return Err(ThreadedNotes::invalid(
                        &notes.1.member,
                        "expected every person link to resolve, with PeoplePicker for mentions",
                    ));
                }
            }
        }
        Ok(())
    }

    fn transfer_notes(
        &self,
        shift: &Shift<'_>,
        cached: &mut PartBytes,
        plan: &mut Plan,
    ) -> Result<()> {
        let Shift::Move {
            from,
            block,
            to,
            target,
            ..
        } = *shift
        else {
            return Ok(());
        };
        let source_at = self.resolve(from).expect("the cut source is resolved");
        let destination_at = self.resolve(to).expect("the cut destination is resolved");
        let same = source_at == destination_at;
        if same && block.start() == target {
            return Ok(());
        }
        let source = self.note_bundle(source_at, cached, plan)?;
        let destination = if same {
            None
        } else {
            Some(self.note_bundle(destination_at, cached, plan)?)
        };
        let landing = block.moved_to(target);
        let selected: Vec<_> = source
            .comments
            .as_ref()
            .map(|(_, notes)| {
                notes
                    .notes
                    .iter()
                    .filter(|note| block.contains(note.at))
                    .collect()
            })
            .unwrap_or_default();
        let selected_cells: BTreeSet<_> = selected.iter().map(|note| note.at).collect();
        let target_bundle = destination.as_ref().unwrap_or(&source);
        let covered: BTreeSet<_> = target_bundle
            .comments
            .as_ref()
            .map(|(_, notes)| {
                notes
                    .notes
                    .iter()
                    .filter(|note| {
                        landing.contains(note.at) && !(same && selected_cells.contains(&note.at))
                    })
                    .map(|note| note.at)
                    .collect()
            })
            .unwrap_or_default();
        if selected.is_empty() && covered.is_empty() {
            return Ok(());
        }
        let mut load_drawing = |bundle: &NoteBundle| -> Result<Option<NoteDrawing>> {
            bundle
                .drawing
                .as_ref()
                .map(|(_, member)| {
                    let payload = plan
                        .part(self, member, cached)?
                        .ok_or_else(|| Error::absent("note drawing", member))?;
                    cached
                        .entry(member.clone())
                        .or_insert_with(|| Arc::clone(&payload));
                    NoteDrawing::read(member.clone(), payload)
                })
                .transpose()
        };
        let source_drawing = if same || !selected.is_empty() {
            load_drawing(&source)?
        } else {
            None
        };
        let destination_drawing = destination
            .as_ref()
            .map(load_drawing)
            .transpose()?
            .flatten();
        let check_drawing = |bundle: &NoteBundle,
                             drawing: Option<&NoteDrawing>,
                             affected: &BTreeSet<CellRef>|
         -> Result<()> {
            let Some(drawing) = drawing else {
                return Ok(());
            };
            let cells: BTreeSet<_> = bundle
                .comments
                .as_ref()
                .map(|(_, notes)| notes.notes.iter().map(|note| note.at).collect())
                .unwrap_or_default();
            if drawing.notes.iter().any(|shape| !cells.contains(&shape.at))
                || affected
                    .iter()
                    .any(|at| !drawing.notes.iter().any(|shape| shape.at == *at))
            {
                return Err(Error::InvalidRecord {
                    path: drawing.member.clone(),
                    reason:
                        "expected each affected legacy note to have its corresponding VML shape"
                            .into(),
                });
            }
            Ok(())
        };
        let mut source_affected = selected_cells.clone();
        if same {
            source_affected.extend(&covered);
        }
        check_drawing(&source, source_drawing.as_ref(), &source_affected)?;
        if let Some(destination) = &destination {
            check_drawing(destination, destination_drawing.as_ref(), &covered)?;
        }
        // One graph proves exclusive mutation ownership and later prunes the
        // newly orphaned closure after prospective relationship updates.
        let members = self.members()?;
        let book_rels = package::relationships_part_of(&self.workbook_part);
        let book_bytes = plan
            .part(self, &book_rels, cached)?
            .ok_or_else(|| Error::absent("workbook relationships", &book_rels))?;
        let relationships = Relationships::from_xml(&book_bytes, &self.workbook_part)?;
        self.check_thread_people(
            &[Some(&source), destination.as_ref()],
            &book_bytes,
            &relationships,
            cached,
            plan,
        )?;
        let mut graph =
            self.part_graph(&members, (&book_rels, &relationships), Some(&mut *cached))?;
        // The graph has already cached every present .rels part. MS-XLSX
        // 2.1.17 forbids outgoing edges to its protocol-defined parts; the
        // two part kinds modeled here are threaded comments and persons.
        // Unknown custom edges are not classified as protocol violations.
        let mut source_thread_dependencies = None;
        for (index, bundle) in [Some(&source), destination.as_ref()]
            .into_iter()
            .flatten()
            .enumerate()
        {
            let Some((_, notes)) = &bundle.threads else {
                continue;
            };
            let rels = package::relationships_part_of(&notes.member);
            let Some(bytes) = plan.part(self, &rels, cached)? else {
                continue;
            };
            let entries = Registration::root(&bytes)?.members(false)?;
            for entry in &entries {
                if matches!(
                    entry.attribute(b"Type")?.as_deref(),
                    Some(PERSON_RELATIONSHIP | THREADED_RELATIONSHIP)
                ) {
                    return Err(ThreadedNotes::invalid(
                        &rels,
                        "threaded comments cannot relate to a persons or threaded comments part",
                    ));
                }
            }
            if index == 0 && !entries.is_empty() {
                source_thread_dependencies = Some(rels);
            }
        }
        let removed_sheets = self.removed_roots(&relationships);
        let already_dropped = graph.dropped(&removed_sheets);
        let mut affected = Vec::new();
        if same || !selected.is_empty() {
            affected.push((source_at, &source));
        }
        if let Some(destination) = &destination {
            affected.push((destination_at, destination));
        }
        for (index, bundle) in &affected {
            for member in bundle
                .comments
                .iter()
                .map(|(_, notes)| &notes.member)
                .chain(bundle.drawing.iter().map(|(_, member)| member))
                .chain(bundle.threads.iter().map(|(_, notes)| &notes.member))
            {
                if graph
                    .sources
                    .get(member)
                    .into_iter()
                    .flatten()
                    .filter(|owner| !already_dropped.contains(*owner))
                    .any(|owner| *owner != self.slots[*index].part)
                {
                    return Err(Error::Conflict {
                        expected: "a note part owned only by its worksheet",
                        actual: "a note part shared by another package owner",
                        path: member.clone(),
                    });
                }
            }
        }
        let moved_at = |at: CellRef| {
            CellRef::new(
                target.row() + at.row() - block.start().row(),
                target.column() + at.column() - block.start().column(),
            )
        };
        type Image = Option<(SmolStr, Arc<[u8]>)>;
        let source_threads = source.threads.as_ref().map(|(_, notes)| notes);
        let destination_threads = destination
            .as_ref()
            .and_then(|bundle| bundle.threads.as_ref().map(|(_, notes)| notes));
        if let (Some(rels), Some(notes)) = (&source_thread_dependencies, source_threads) {
            let moved = notes
                .messages
                .iter()
                .filter(|message| selected_cells.contains(&message.at))
                .count();
            // Opaque message extensions may name any of their part's edges.
            // Keep that part and its relative targets intact, or refuse before
            // splitting/merging messages into a different relationship owner.
            if !same
                && moved != 0
                && (moved != notes.messages.len() || destination_threads.is_some())
            {
                return Err(Error::unsupported(
                    "moving threaded comments into another part with custom dependencies",
                    rels,
                ));
            }
        }
        if let (Some(source), Some(destination)) = (source_threads, destination_threads) {
            let ids: BTreeSet<_> = source.messages.iter().map(|message| message.id).collect();
            if destination
                .messages
                .iter()
                .any(|message| ids.contains(&message.id))
            {
                return Err(ThreadedNotes::invalid(
                    &destination.member,
                    "expected threaded message IDs to be unique across worksheets",
                ));
            }
        }
        let remaining_threads = source_threads
            .map(|notes| -> Result<Image> {
                let mut changes = BTreeMap::new();
                for message in &notes.messages {
                    if selected_cells.contains(&message.at) {
                        changes.insert(
                            message.entry.key.clone(),
                            if same {
                                Some(message.moved(moved_at(message.at))?)
                            } else {
                                None
                            },
                        );
                    } else if same && covered.contains(&message.at) {
                        changes.insert(message.entry.key.clone(), None);
                    }
                }
                let removed = changes.values().filter(|value| value.is_none()).count();
                if removed == notes.messages.len() {
                    return Ok(None);
                }
                Ok(Some((
                    notes.member.clone(),
                    notes.rewritten(&changes, &[])?,
                )))
            })
            .transpose()?
            .flatten();
        let arriving_threads = if same {
            None
        } else {
            let added = source_threads
                .map(|notes| {
                    notes
                        .messages
                        .iter()
                        .filter(|message| selected_cells.contains(&message.at))
                        .map(|message| message.moved(moved_at(message.at)))
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?
                .unwrap_or_default();
            if let Some(notes) = destination_threads {
                let changes: BTreeMap<_, _> = notes
                    .messages
                    .iter()
                    .filter(|message| covered.contains(&message.at))
                    .map(|message| (message.entry.key.clone(), None))
                    .collect();
                if notes.messages.len() - changes.len() + added.len() == 0 {
                    None
                } else {
                    Some((notes.member.clone(), notes.rewritten(&changes, &added)?))
                }
            } else if !added.is_empty() {
                let notes = source_threads.expect("moved messages have a source part");
                let member = if remaining_threads.is_none() {
                    notes.member.clone()
                } else {
                    plan.note_member(self, &notes.member)?
                };
                let mut changes: BTreeMap<_, _> = notes
                    .messages
                    .iter()
                    .map(|message| (message.entry.key.clone(), None))
                    .collect();
                for (message, after) in notes
                    .messages
                    .iter()
                    .filter(|message| selected_cells.contains(&message.at))
                    .zip(added)
                {
                    changes.insert(message.entry.key.clone(), Some(after));
                }
                Some((member, notes.rewritten(&changes, &[])?))
            } else {
                None
            }
        };

        let mut outputs: Vec<(usize, &NoteBundle, Image, Image, Image)> = Vec::new();
        if same {
            let (_, notes) = source.comments.as_ref().expect("affected notes exist");
            let mut changes = BTreeMap::new();
            for note in &notes.notes {
                if selected_cells.contains(&note.at) {
                    changes.insert(
                        note.entry.key.clone(),
                        Some(note.entry.with_attributes(vec![(
                            "ref".into(),
                            Some(moved_at(note.at).to_string()),
                        )])?),
                    );
                } else if covered.contains(&note.at) {
                    changes.insert(note.entry.key.clone(), None);
                }
            }
            let comments = (notes.notes.len() > covered.len())
                .then(|| {
                    notes
                        .rewritten(&changes, &[], &[])
                        .map(|bytes| (notes.member.clone(), bytes))
                })
                .transpose()?;
            let drawing = source_drawing
                .as_ref()
                .map(|drawing| -> Result<Image> {
                    let mut changes = BTreeMap::new();
                    for shape in &drawing.notes {
                        if selected_cells.contains(&shape.at) {
                            changes.insert(
                                shape.entry.key.clone(),
                                Some(shape.moved(moved_at(shape.at), &drawing.member)?),
                            );
                        } else if covered.contains(&shape.at) {
                            changes.insert(shape.entry.key.clone(), None);
                        }
                    }
                    let gone = changes.values().filter(|after| after.is_none()).count();
                    if gone == drawing.shape_keys.len() && !drawing.other_payload {
                        return Ok(None);
                    }
                    Ok(Some((
                        drawing.member.clone(),
                        drawing.rewritten(&changes, &[])?,
                    )))
                })
                .transpose()?
                .flatten();
            outputs.push((source_at, &source, comments, drawing, remaining_threads));
        } else {
            let destination = destination.as_ref().expect("cross-sheet destination");
            let source_notes = source.comments.as_ref().map(|(_, notes)| notes);
            let destination_notes = destination.comments.as_ref().map(|(_, notes)| notes);
            if let (Some(source), Some(destination)) = (source_notes, destination_notes) {
                if !selected.is_empty() && source.namespace != destination.namespace {
                    return Err(Error::unsupported(
                        "merging legacy comment parts from different SpreadsheetML namespaces",
                        &source.member,
                    ));
                }
            }
            let mut names = destination_notes
                .map(|notes| notes.author_names.clone())
                .or_else(|| source_notes.map(|notes| notes.author_names.clone()))
                .unwrap_or_default();
            let mut added_authors = Vec::new();
            let mut added = Vec::new();
            for note in &selected {
                let notes = source_notes.expect("selected note part exists");
                let author = if destination_notes.is_some() {
                    match names
                        .iter()
                        .position(|name| *name == notes.author_names[note.author])
                    {
                        Some(index) => index,
                        None => {
                            let index = names.len();
                            names.push(notes.author_names[note.author].clone());
                            added_authors.push(notes.author_entries[note.author].clone());
                            index
                        }
                    }
                } else {
                    note.author
                };
                added.push(note.entry.with_attributes(vec![
                    ("ref".into(), Some(moved_at(note.at).to_string())),
                    ("authorId".into(), Some(author.to_string())),
                ])?);
            }
            let source_comments = source_notes
                .map(|notes| -> Result<Image> {
                    if notes.notes.len() == selected.len() {
                        return Ok(None);
                    }
                    let changes = selected
                        .iter()
                        .map(|note| (note.entry.key.clone(), None))
                        .collect();
                    Ok(Some((
                        notes.member.clone(),
                        notes.rewritten(&changes, &[], &[])?,
                    )))
                })
                .transpose()?
                .flatten();
            let target_comments = if let Some(notes) = destination_notes {
                if notes.notes.len() - covered.len() + added.len() == 0 {
                    None
                } else {
                    let changes = notes
                        .notes
                        .iter()
                        .filter(|note| covered.contains(&note.at))
                        .map(|note| (note.entry.key.clone(), None))
                        .collect();
                    Some((
                        notes.member.clone(),
                        notes.rewritten(&changes, &added, &added_authors)?,
                    ))
                }
            } else if !added.is_empty() {
                let notes = source_notes.expect("selected note part exists");
                let mut changes: BTreeMap<_, _> = notes
                    .notes
                    .iter()
                    .map(|note| (note.entry.key.clone(), None))
                    .collect();
                for (old, new) in selected.iter().zip(&added) {
                    changes.insert(old.entry.key.clone(), Some(new.clone()));
                }
                let member = if source_comments.is_none() {
                    notes.member.clone()
                } else {
                    plan.note_member(self, &notes.member)?
                };
                Some((member, notes.rewritten(&changes, &[], &[])?))
            } else {
                None
            };

            let moved_shapes = source_drawing
                .as_ref()
                .map(|drawing| {
                    drawing
                        .notes
                        .iter()
                        .filter(|shape| selected_cells.contains(&shape.at))
                        .map(|shape| Ok((shape, shape.moved(moved_at(shape.at), &drawing.member)?)))
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?
                .unwrap_or_default();
            let source_shapes = source_drawing
                .as_ref()
                .map(|drawing| {
                    drawing
                        .remaining(&selected_cells)
                        .map(|bytes| bytes.map(|bytes| (drawing.member.clone(), bytes)))
                })
                .transpose()?
                .flatten();
            let target_shapes = if let Some(drawing) = &destination_drawing {
                if moved_shapes.is_empty() {
                    drawing
                        .remaining(&covered)?
                        .map(|bytes| (drawing.member.clone(), bytes))
                } else {
                    Some((
                        drawing.member.clone(),
                        drawing.merged(
                            source_drawing.as_ref().expect("moved shapes exist"),
                            &moved_shapes,
                            &covered,
                        )?,
                    ))
                }
            } else if !moved_shapes.is_empty() {
                let drawing = source_drawing.as_ref().expect("moved shapes exist");
                let member = if source_shapes.is_none() {
                    drawing.member.clone()
                } else {
                    plan.note_member(self, &drawing.member)?
                };
                let bytes = if source_shapes.is_none() {
                    let changes = moved_shapes
                        .iter()
                        .map(|(shape, entry)| (shape.entry.key.clone(), Some(entry.clone())))
                        .collect();
                    drawing.rewritten(&changes, &[])?
                } else {
                    drawing.partition(&moved_shapes)?
                };
                Some((member, bytes))
            } else {
                None
            };
            if !selected.is_empty() {
                outputs.push((
                    source_at,
                    &source,
                    source_comments,
                    source_shapes,
                    remaining_threads,
                ));
            }
            outputs.push((
                destination_at,
                destination,
                target_comments,
                target_shapes,
                arriving_threads,
            ));
        }
        let mut retired = Vec::new();
        for (index, bundle, comments, drawing, threads) in &outputs {
            let desired_threads = threads.as_ref().map(|(member, _)| member.as_str());
            if let Some((_, old)) = &bundle.threads {
                if Some(old.member.as_str()) != desired_threads {
                    retired.push(old.member.clone());
                }
            }
            let desired_comments = comments.as_ref().map(|(member, _)| member.as_str());
            let desired_drawing = drawing.as_ref().map(|(member, _)| member.as_str());
            if let Some((_, old)) = &bundle.comments {
                if Some(old.member.as_str()) != desired_comments {
                    retired.push(old.member.clone());
                }
            }
            if let Some((_, old)) = &bundle.drawing {
                if Some(old.as_str()) != desired_drawing {
                    retired.push(old.clone());
                }
            }
            for (image, mime) in [
                (
                    comments,
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml",
                ),
                (
                    drawing,
                    "application/vnd.openxmlformats-officedocument.vmlDrawing",
                ),
                (threads, "application/vnd.ms-excel.threadedcomments+xml"),
            ] {
                if let Some((member, bytes)) = image {
                    plan.note_payload(self, member, Arc::clone(bytes), cached)?;
                    if !members.contains(member) {
                        plan.ensure_content_type(self, member, mime, cached)?;
                    }
                }
            }
            self.plan_note_ownership(
                *index,
                bundle,
                (desired_comments, desired_drawing, desired_threads),
                &source,
                cached,
                plan,
            )?;
        }
        for (index, bundle, _, _, _) in &outputs {
            let bytes = plan.part(self, &bundle.rels, cached)?;
            let relationships = match bytes {
                Some(bytes) => Relationships::from_xml(&bytes, &self.slots[*index].part)?,
                None => Relationships::default(),
            };
            graph.replace_source(
                &self.slots[*index].part,
                &relationships,
                &self.workbook_part,
            );
        }
        let mut roots = removed_sheets;
        for member in retired {
            if graph
                .sources
                .get(&member)
                .is_none_or(|owners| owners.iter().all(|owner| already_dropped.contains(owner)))
            {
                roots.push(member);
            }
        }
        let removed = graph
            .dropped(&roots)
            .difference(&already_dropped)
            .cloned()
            .collect();
        plan.prune_note_parts(self, removed, cached)
    }

    /// Commit only the ownership documents to this private plan. Payloads are
    /// planned separately, so a whole part can move without acquiring a new name.
    fn plan_note_ownership(
        &self,
        index: usize,
        bundle: &NoteBundle,
        targets: (Option<&str>, Option<&str>, Option<&str>),
        template: &NoteBundle,
        cached: &PartBytes,
        plan: &mut Plan,
    ) -> Result<()> {
        let (comments, drawing, threads) = targets;
        let mut used: BTreeSet<_> = bundle
            .entries
            .iter()
            .map(|entry| entry.key.clone())
            .collect();
        let mut removed = BTreeSet::new();
        let mut added = Vec::new();
        let mut drawing_id = None;
        let current_comments = bundle
            .comments
            .as_ref()
            .map(|(id, notes)| (id, notes.member.as_str()));
        let current_drawing = bundle
            .drawing
            .as_ref()
            .map(|(id, member)| (id, member.as_str()));
        let current_threads = bundle
            .threads
            .as_ref()
            .map(|(id, notes)| (id, notes.member.as_str()));
        for (current, target, original, vml) in [
            (
                current_comments,
                comments,
                template.comments.as_ref().map(|(id, _)| id),
                false,
            ),
            (
                current_drawing,
                drawing,
                template.drawing.as_ref().map(|(id, _)| id),
                true,
            ),
            (
                current_threads,
                threads,
                template.threads.as_ref().map(|(id, _)| id),
                false,
            ),
        ] {
            if current.is_some_and(|(_, member)| Some(member) == target) {
                continue;
            }
            if let Some((id, _)) = current {
                removed.insert(id.clone());
            }
            let Some(target) = target else {
                continue;
            };
            let original = original.ok_or_else(|| Error::InvalidRecord {
                path: bundle.rels.clone(),
                reason: "expected a source note registration".into(),
            })?;
            let entry = template
                .entries
                .iter()
                .find(|entry| entry.key == *original)
                .expect("the resolved note registration is direct");
            let mut id = original.clone();
            let mut next = 1_u32;
            while !used.insert(id.clone()) {
                id = format_smolstr!("rIdNote{next}");
                next = next.checked_add(1).ok_or_else(|| {
                    Error::unsupported(
                        "allocating an exhausted note relationship id sequence",
                        &bundle.rels,
                    )
                })?;
            }
            let mut moved = entry.with_attributes(vec![
                ("Id".into(), Some(id.to_string())),
                (
                    "Target".into(),
                    Some(package::relative_to(
                        package::folder_of(&self.slots[index].part),
                        target,
                    )),
                ),
            ])?;
            moved.key = id.clone();
            added.push(moved);
            if vml {
                drawing_id = Some(id);
            }
        }
        if !removed.is_empty() || !added.is_empty() {
            let after = bundle.document.without(false, &removed)?.appended(&added)?;
            let before = bundle
                .bytes
                .as_deref()
                .unwrap_or(package::TEMPLATE_WORKBOOK_RELATIONSHIPS.as_bytes());
            let after = Registration::replace_root(before, &after.xml)?;
            if bundle.bytes.is_none() {
                plan.ensure_content_type(
                    self,
                    &bundle.rels,
                    package::RELATIONSHIPS_CONTENT_TYPE,
                    cached,
                )?;
            }
            plan.set_part(
                bundle.rels.clone(),
                bundle.bytes.clone(),
                Some(after.into()),
            );
        }
        if let Some(bytes) = &bundle.bytes {
            // An unchanged relationship still proves who owns retained note
            // payloads. A planned rewrite already retains its own precondition.
            plan.keep_part(bundle.rels.clone(), Arc::clone(bytes));
        }
        if current_drawing.map(|(_, member)| member) != drawing {
            let mut frame = plan.frame(self, index);
            let previous = bundle.legacy.as_ref().map(|entry| entry.xml.as_bytes());
            let after = match drawing_id {
                Some(id) => {
                    let legacy = template
                        .legacy
                        .as_ref()
                        .expect("the source owns a legacy drawing");
                    let (key, _) = legacy.relationship_key(&self.slots[index].part)?;
                    let moved = legacy.with_attributes(vec![(key, Some(id.to_string()))])?;
                    let mut root = frame.root.to_vec();
                    root.extend_from_slice(format!("</{}>", frame.root_name).as_bytes());
                    let root = Registration::root(&root)?;
                    Some(
                        moved
                            .fragment(&root.namespaces, &root.markup)?
                            .into_bytes()
                            .into(),
                    )
                }
                None => None,
            };
            frame.set_child("legacyDrawing", previous, after);
            plan.set_frame(index, frame.items);
        }
        Ok(())
    }

    /// The parts beside the sheets that state references by sheet name -
    /// every chart, pivot cache definition, drawing and worksheet table - each
    /// with the references it states, read once per package. New reads join
    /// `fresh` for the same plan to reuse; a member in `known` comes from there.
    ///
    /// Held state: one [`Referring`] per such member and one shared relationship
    /// document per table-owning worksheet, until adoption or membership changes.
    /// Every structural edit, rename and removal asks the same index; retaining
    /// the owner image makes later inverse witnesses require no source reread.
    fn referring(
        &self,
        known: &[Related],
        known_sheet: Option<&str>,
        sheet_relations: &[(SheetKey, Vec<(RelationshipKind, SmolStr)>)],
        fresh: &mut PartBytes,
    ) -> Result<Arc<[Referring]>> {
        if let Some(held) = self.stated.referring.get() {
            return Ok(Arc::clone(held));
        }
        let mut entries = Vec::new();
        // A table's unqualified references belong to the worksheet that
        // relates it. A stable key keeps that ownership across renames.
        // The band check already read its own sheet's relationships.
        let mut tables = HashMap::new();
        let mut table_owner = |member: SmolStr, key: SheetKey, bytes: Arc<[u8]>| -> Result<()> {
            if let Some((previous, _)) = tables.insert(member.clone(), (key, bytes)) {
                if previous != key {
                    let name = |key| {
                        self.slots
                            .iter()
                            .find(|slot| slot.key == key)
                            .map_or("", |slot| slot.name.as_str())
                    };
                    return Err(Error::InvalidRecord {
                        path: member,
                        reason: format_smolstr!(
                            "expected one worksheet owning the table part, got {} and {}",
                            name(previous),
                            name(key)
                        ),
                    });
                }
            }
            Ok(())
        };
        for (index, slot) in self.slots.iter().enumerate() {
            if known_sheet == Some(slot.name.as_str()) {
                for (_, member, _) in known
                    .iter()
                    .filter(|(kind, _, _)| *kind == RelationshipKind::Table)
                {
                    let part = package::relationships_part_of(&slot.part);
                    let bytes = fresh
                        .get(&part)
                        .expect("the owner discovery retained its relationship bytes");
                    table_owner(member.clone(), slot.key, Arc::clone(bytes))?;
                }
            } else if slot
                .parsed
                .get()
                .and_then(Sheet::frame)
                .is_some_and(|frame| frame.items.iter().any(|item| item.name == "tableParts"))
            {
                // A cut already read these relationships to find owned
                // drawings. Reuse that same discovery for table ownership.
                let read;
                let relations = match sheet_relations.iter().find(|(key, _)| *key == slot.key) {
                    Some((_, relations)) => relations,
                    None => {
                        read = self.relations_read(index, Some(&mut *fresh), None)?;
                        &read
                    }
                };
                for (_, member) in relations
                    .iter()
                    .filter(|(kind, _)| *kind == RelationshipKind::Table)
                {
                    let part = package::relationships_part_of(&slot.part);
                    let bytes = fresh
                        .get(&part)
                        .expect("the owner discovery retained its relationship bytes");
                    table_owner(member.clone(), slot.key, Arc::clone(bytes))?;
                }
            }
        }
        for member in self.members()? {
            let xml = member.ends_with(".xml");
            let (kind, owner_document) = if let Some((key, bytes)) = tables.remove(&member) {
                (Referrer::Table(key), Some(bytes))
            } else if xml && member.starts_with("xl/charts/chart") {
                (Referrer::Chart, None)
            } else if xml && member.starts_with("xl/pivotCache/pivotCacheDefinition") {
                (Referrer::Cache, None)
            } else if xml && member.starts_with("xl/drawings/drawing") {
                (Referrer::Drawing, None)
            } else {
                continue;
            };
            // A member an edit rewrote is read from what it wrote until the
            // workbook adopts a package, which reads this again.
            let references = if self.stated.overrides.contains_key(&member) {
                None
            } else {
                let bytes = match known.iter().find(|(_, held, _)| *held == member) {
                    Some((_, _, bytes)) => Arc::clone(bytes),
                    None => Arc::from(self.source.read(&member)?),
                };
                let mut gather = shift::References::new();
                // A part that does not read is looked at whole, as before.
                let references = package::edit_document(&bytes, &mut gather)
                    .ok()
                    .map(|_| gather.into_text());
                fresh.insert(member.clone(), bytes);
                references
            };
            entries.push(Referring {
                member,
                kind,
                references,
                owner_document,
            });
        }
        let entries: Arc<[Referring]> = entries.into();
        let _ = self.stated.referring.set(Arc::clone(&entries));
        Ok(entries)
    }

    /// Refuse removing the worksheet at `at` when a slicer or timeline cache
    /// the workbook lists names a pivot table on it, by its tab's `sheetId`,
    /// or a table it hosts, by the table's id: the cache would name what no
    /// sheet holds.
    fn check_caches(&self, at: usize) -> Result<()> {
        if self.stated.caches.is_empty() {
            return Ok(());
        }
        let sheet_id = self.slots[at].sheet_id;
        let mut tables: Option<Vec<u32>> = None;
        for member in &self.stated.caches {
            let bytes = self.part_bytes(member)?;
            let named = shift::CacheSources::read(&bytes, member)?;
            let mut hosted = named.tabs.contains(&sheet_id);
            if !hosted && !named.tables.is_empty() {
                if tables.is_none() {
                    let mut ids = Vec::new();
                    for (kind, table) in self.relations(at)? {
                        if kind == RelationshipKind::Table {
                            ids.extend(Table::read(&self.part_bytes(&table)?, &table)?.id);
                        }
                    }
                    tables = Some(ids);
                }
                hosted = tables
                    .as_ref()
                    .is_some_and(|ids| named.tables.iter().any(|id| ids.contains(id)));
            }
            if hosted {
                return Err(Error::unsupported(
                    "removing a sheet hosting a pivot table or table a slicer or timeline \
                     cache names",
                    member,
                ));
            }
        }
        Ok(())
    }

    /// Follow `shift` everywhere a reference is held: every formula of every
    /// parsed worksheet but the one at `except` - one rewrite per shape and
    /// host class - every defined name, and what `plan` worked out; each
    /// change the opposite shift would not give back recorded in
    /// `restore`.
    fn follow(
        &mut self,
        shift: &Shift<'_>,
        plan: Plan,
        except: Option<usize>,
        restore: &mut Restore,
    ) {
        let mut rewriter = Rewriter::new(*shift);
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if Some(index) == except {
                continue;
            }
            let stamp = self
                .stated
                .attempt
                .as_ref()
                .and_then(|attempt| attempt.sheet_stamp(slot));
            let Some(sheet) = slot.parsed.get_mut() else {
                continue;
            };
            let revision = sheet.revision();
            let name = slot.name.clone();
            let mut lost = Vec::new();
            sheet.rewrite_formulas_at(|at, formula| {
                let (to_sheet, to) = shift.place(&name, at)?;
                let host = Host {
                    sheet: &name,
                    at,
                    to_sheet,
                    to,
                };
                let (rewritten, reversible) = rewriter.rewrite(formula, &host);
                if !reversible {
                    lost.push((at, Some(formula.clone())));
                }
                rewritten
            });
            if sheet.revision() != revision {
                if let Some(stamp) = stamp {
                    self.stated
                        .attempt
                        .as_mut()
                        .expect("the stamp came from this Batch")
                        .before
                        .push(Before::Sheet(stamp));
                }
            }
            if !lost.is_empty() {
                restore.push(Step::Formulas {
                    key: slot.key,
                    cells: lost,
                });
            }
        }
        let names = self.stated.names.clone();
        let mut touched = false;
        let mut filter_names = plan.filter_names.into_iter().peekable();
        for (index, defined) in self.stated.names.iter_mut().enumerate() {
            let scope = defined
                .scope()
                .and_then(|key| self.slots.iter().find(|slot| slot.key == key))
                .map_or("", |slot| slot.name.as_str());
            let host = Host::fixed(scope, CellRef::new(0, 0));
            let prepared = if filter_names.peek().is_some_and(|(at, _)| *at == index) {
                filter_names.next().map(|(_, formula)| formula)
            } else {
                None
            };
            if let Some(formula) =
                prepared.or_else(|| shift::shifted(defined.formula(), &host, shift))
            {
                defined.set_formula(formula);
                touched = true;
            }
        }
        if touched {
            self.invalidate_calculation();
            self.stated.names_touched = true;
            restore.push(Step::Names(names));
        }
        for (index, items) in plan.frames {
            let slot = &mut self.slots[index];
            self.stated.remember_slot(slot);
            if let Some(sheet) = slot.parsed.get_mut() {
                // This edit keeps the root's namespace context. Only a
                // changed tableParts list can change table ownership;
                // ordinary CF/DV rewrites must retain the referring index.
                let table_parts = |item: &&super::carried::Carried| item.name == "tableParts";
                let tables_changed = sheet
                    .frame()
                    .into_iter()
                    .flat_map(|frame| frame.items.iter())
                    .filter(table_parts)
                    .ne(items.iter().filter(table_parts));
                restore.push(Step::Frame {
                    key: slot.key,
                    frame: sheet.frame().cloned().map(Box::new),
                });
                sheet.set_frame_items(items);
                if tables_changed {
                    self.stated.referring = OnceLock::new();
                }
            }
        }
        // Each part is put back as the bytes it held, whatever a save
        // makes of the member in between.
        if !plan.overrides.is_empty() || !plan.kept.is_empty() {
            let mut prior: Vec<_> = plan
                .kept
                .into_iter()
                .map(|(name, bytes)| PartRestore::new(name, Some(Arc::clone(&bytes)), Some(bytes)))
                .collect();
            for Rewritten {
                member,
                before,
                after,
            } in plan.overrides
            {
                prior.push(PartRestore::new(member.clone(), before, after.clone()));
                self.stated.set_part(member, after);
            }
            restore.push(Step::Overrides(prior));
        }
    }

    /// Clear `what` of every cell of `ranges` in the sheet `sheet`: its
    /// content and its format ([`Clear::All`]), its content, keeping its
    /// format ([`Clear::Contents`]) - as Delete does - or its format,
    /// keeping its value ([`Clear::Formats`]), which reads a date as its
    /// serial again. Clearing formats of whole rows or columns clears
    /// their own style too, and a merge inside the ranges is taken apart.
    ///
    /// ```
    /// use yggdryl::excel::{Clear, StylePatch, Workbook};
    ///
    /// let mut workbook = Workbook::new();
    /// workbook.add_sheet("Sheet1")?.set_cell("A1".parse()?, 1.5)?;
    /// let bold = StylePatch { bold: Some(true), ..StylePatch::default() };
    /// workbook.set_style("Sheet1", &["A1".parse()?], &bold)?;
    /// workbook.clear("Sheet1", &["A1".parse()?], Clear::Contents)?;
    /// assert!(workbook.cell_style("Sheet1", "A1".parse()?)?.font.bold);
    /// assert_eq!(workbook.display_text("Sheet1", "A1".parse()?)?.map(|shown| shown.text), Some("".into()));
    /// workbook.clear("Sheet1", &["A1".parse()?], Clear::All)?;
    /// assert!(workbook.sheet("Sheet1")?.cell("A1".parse()?).is_none());
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns what [`Self::sheet`] returns.
    pub fn clear(&mut self, sheet: &str, ranges: &[CellRange], what: Clear) -> Result<()> {
        let at = self.resolve(sheet).ok_or_else(|| self.absent(sheet))?;
        self.parsed(at)?;
        let held = self.slots[at].parsed.get_mut().expect("parsed above");
        for range in ranges {
            let cells: Vec<CellRef> = held.cells_in(*range).map(Cell::reference).collect();
            let whole_rows = range.is_column_open() && range.start().column() == 0;
            let whole_columns = range.is_row_open() && range.start().row() == 0;
            if what != Clear::Contents {
                if whole_rows {
                    held.clear_row_styles(range.start().row()..range.end().row() + 1);
                }
                if whole_columns {
                    held.clear_column_styles(range.start().column()..range.end().column() + 1);
                }
                held.unmerge_inside(*range);
            }
            for cell in cells {
                match what {
                    Clear::All => {
                        held.remove_cell(cell);
                    }
                    Clear::Contents => held.clear_content(cell),
                    Clear::Formats => held.clear_format(cell),
                }
            }
        }
        Ok(())
    }

    /// Paste the cells of `from` - a sheet and a range of it - with their
    /// top-left cell at `to`, answering the range pasted over.
    ///
    /// [`Paste::All`] puts each cell whole, its formula translated to where
    /// it lands; [`Paste::Values`] puts each value, a formula's result
    /// included, [`Paste::Formulas`] each formula or constant, both keeping
    /// the format of the cell pasted over; [`Paste::Formats`] puts each
    /// cell's style and keeps the value pasted over - the Format Painter.
    /// A reference a translated formula carries off the grid is `#REF!`.
    /// With `cut`, the cells move, as Excel moves cut cells: each moved
    /// formula keeps naming the cells it named, every reference into the
    /// moved cells anywhere in the workbook follows them, one into the cells
    /// pasted over is `#REF!`, and the cells cut are empty after.
    /// A cross-sheet cut qualifies bare names owned by the source sheet so
    /// their binding survives the move.
    ///
    /// ```
    /// use yggdryl::excel::{Cell, DateSystem, Formula, Paste, Workbook};
    ///
    /// let mut workbook = Workbook::new();
    /// let sheet = workbook.add_sheet("Sheet1")?;
    /// sheet.set_cell("A1".parse()?, 2.0)?;
    /// let host = "B1".parse()?;
    /// sheet.insert_cell(
    ///     Cell::from_scalar(host, 4.0.into(), DateSystem::Year1900)?
    ///         .with_formula(Formula::from_file("A1*2", host)),
    /// )?;
    /// let pasted = workbook.paste(("Sheet1", "B1".parse()?), ("Sheet1", "B3".parse()?), Paste::All, false)?;
    /// assert_eq!(pasted.to_string(), "B3");
    /// let copied = workbook.sheet("Sheet1")?.cell("B3".parse()?).unwrap();
    /// assert_eq!(copied.formula().unwrap().at(copied.reference()).to_string(), "A3*2");
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns, before anything changes, [`Error::InvalidRecord`] for a
    /// paste landing off the grid or over a merge, one taking or landing on
    /// part of an array formula, a cut pasting less than everything, or
    /// formats pasted over more than [`MAX_EDITED_CELLS`] cells; what
    /// [`Self::sheet`] returns otherwise. A cross-sheet cut returns
    /// [`Error::Unsupported`] if a destination-local name would shadow a
    /// source binding, or affected names occur in lexical bindings or as
    /// callable names whose scope cannot yet be preserved.
    pub fn paste(
        &mut self,
        from: (&str, CellRange),
        to: (&str, CellRef),
        what: Paste,
        cut: bool,
    ) -> Result<CellRange> {
        self.paste_checks(from, to, what, cut)?;
        let source_at = self.resolve(from.0).ok_or_else(|| self.absent(from.0))?;
        let target_at = self.resolve(to.0).ok_or_else(|| self.absent(to.0))?;
        let block = from.1;
        let landing = block.moved_to(to.1);
        if cut {
            self.cut_paste(source_at, block, target_at, to.1)?;
            return Ok(landing);
        }
        let system = self.system;
        let table = self.styles()?;
        // A landing cell's origin in the block, and a block cell's landing:
        // a paste costs the cells the two ranges hold, never the grid.
        let origin = |at: CellRef| {
            CellRef::new(
                block.start().row() + (at.row() - landing.start().row()),
                block.start().column() + (at.column() - landing.start().column()),
            )
        };
        let landed = |at: CellRef| {
            CellRef::new(
                landing.start().row() + (at.row() - block.start().row()),
                landing.start().column() + (at.column() - block.start().column()),
            )
        };
        if what == Paste::Formats {
            // Each landing cell shows what its origin shows - its own style,
            // else its row's or its column's - so this one walks the
            // landing, which the checks bound.
            let held = self.parsed(source_at)?;
            let styles: Vec<(CellRef, StyleId)> = landing
                .cells()
                .map(|at| (at, held.style_at(origin(at))))
                .collect();
            let target = self.slots[target_at]
                .parsed
                .get_mut()
                .expect("parsed above");
            for (at, style) in styles {
                target.set_cell_style(at, style, table.number_format(style));
            }
            return Ok(landing);
        }
        let source = self.parsed(source_at)?.slice(block);
        let target = self.slots[target_at]
            .parsed
            .get_mut()
            .expect("parsed above");
        let translated = |cell: &Cell| {
            let at = landed(cell.reference());
            let mut cell = cell.clone().at(at);
            if let Some(formula) = cell
                .formula()
                .and_then(|formula| shift::materialized(formula, at))
            {
                cell.set_formula(Some(formula));
            }
            cell
        };
        if what == Paste::All {
            // What the block holds no cell for is empty after.
            let gone: Vec<CellRef> = target
                .cells_in(landing)
                .map(Cell::reference)
                .filter(|at| source.cell(origin(*at)).is_none())
                .collect();
            for at in gone {
                target.remove_cell(at);
            }
            for cell in source.cells() {
                let raw = source.retained_serial(cell.reference());
                let landed = translated(cell);
                let at = landed.reference();
                let bits = raw.and_then(|raw| {
                    super::sheet::CellExtra::exceptional_serial(&landed, raw, system)
                });
                target.insert_cell(landed)?;
                if let Some(bits) = bits {
                    target.attach_serial_bits(at, bits);
                }
            }
            return Ok(landing);
        }
        // Values and formulas keep the format of the cell pasted over, and
        // what the block holds no content for is cleared of content.
        let cleared: Vec<CellRef> = target
            .cells_in(landing)
            .map(Cell::reference)
            .filter(|at| !source.cell(origin(*at)).is_some_and(Cell::has_content))
            .collect();
        for at in cleared {
            target.clear_content(at);
        }
        for held in source.cells().filter(|cell| cell.has_content()) {
            let raw = source.retained_serial(held.reference());
            let mut cell = translated(held);
            if what == Paste::Values || held.formula().is_none() {
                cell.set_formula(None);
            }
            let style = target.style_at(cell.reference());
            let used = cell.restyle(style, table.number_format(style), system, raw);
            let at = cell.reference();
            let bits = used
                .and_then(|raw| super::sheet::CellExtra::exceptional_serial(&cell, raw, system));
            target.insert_cell(cell)?;
            if let Some(bits) = bits {
                target.attach_serial_bits(at, bits);
            }
        }
        Ok(landing)
    }

    /// Refuse a paste [`Self::paste`] refuses, before anything changes.
    pub(crate) fn paste_checks(
        &self,
        from: (&str, CellRange),
        to: (&str, CellRef),
        what: Paste,
        cut: bool,
    ) -> Result<()> {
        let source_at = self.resolve(from.0).ok_or_else(|| self.absent(from.0))?;
        let target_at = self.resolve(to.0).ok_or_else(|| self.absent(to.0))?;
        self.parsed(source_at)?;
        self.parsed(target_at)?;
        let block = from.1;
        let refused = |at: CellRange, reason: SmolStr| Error::InvalidRecord {
            path: format_smolstr!("{}!{at}", to.0),
            reason,
        };
        let last_row = u64::from(to.1.row()) + u64::from(block.row_size());
        let last_column = u64::from(to.1.column()) + u64::from(block.column_size());
        if last_row > u64::from(MAX_ROWS) || last_column > u64::from(MAX_COLUMNS) {
            return Err(refused(
                block,
                format_smolstr!(
                    "expected the pasted cells to land on the grid, got some past it from {}",
                    to.1
                ),
            ));
        }
        let landing = block.moved_to(to.1);
        if cut && what != Paste::All {
            return Err(refused(
                landing,
                SmolStr::new_static("expected a cut to paste everything, got part of it"),
            ));
        }
        if what == Paste::Formats && landing.cell_count() > MAX_EDITED_CELLS {
            return Err(refused(
                landing,
                format_smolstr!(
                    "expected at most {MAX_EDITED_CELLS} cells formatted at once, got {}",
                    landing.cell_count()
                ),
            ));
        }
        {
            let target = self.parsed(target_at)?;
            // Content under a merge shows only at its first cell, so a
            // paste lands on none.
            if let Some(merge) = target.merges().find(|merge| merge.intersects(landing)) {
                return Err(refused(
                    landing,
                    format_smolstr!("expected a paste over no merge, got {merge}"),
                ));
            }
            for (sheet, range) in [(source_at, block), (target_at, landing)] {
                let held = self.parsed(sheet)?;
                if let Some((anchor, array)) = held
                    .formula_ranges()
                    .find(|(_, array)| array.intersects(range) && !range.encloses(*array))
                {
                    return Err(refused(
                        range,
                        format_smolstr!(
                            "expected a paste taking whole array formulas, got part of the one \
                             anchored at {anchor} over {array}"
                        ),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Move the cells of `block` of the worksheet at `source` to where `to`
    /// is their top-left cell on the worksheet at `target`, every reference
    /// into them following, answering what puts them back.
    pub(crate) fn cut_paste(
        &mut self,
        source: usize,
        block: CellRange,
        target: usize,
        to: CellRef,
    ) -> Result<Restore> {
        self.parse_all()?;
        let from_name = self.slots[source].name.clone();
        let to_name = self.slots[target].name.clone();
        let landing = block.moved_to(to);
        let names = if source == target {
            shift::MoveNames::default()
        } else {
            let source_key = self.slots[source].key;
            let target_key = self.slots[target].key;
            shift::MoveNames::new(self.stated.names.iter().filter_map(|name| {
                let action = match name.scope() {
                    Some(key) if key == source_key => shift::NameMove::Qualify,
                    Some(key) if key == target_key => shift::NameMove::Refuse,
                    _ => return None,
                };
                Some((name.name(), action))
            }))
        };
        let shift = Shift::Move {
            from: &from_name,
            block,
            to: &to_name,
            target: to,
            names: &names,
        };
        if !names.is_empty() {
            for cell in self.parsed(source)?.cells_in(block) {
                if let Some(formula) = cell.formula() {
                    let at = cell.reference();
                    let (to_sheet, to) = shift.place(&from_name, at).expect("source cell moves");
                    let host = Host {
                        sheet: &from_name,
                        at,
                        to_sheet,
                        to,
                    };
                    shift.check_names(formula, &host, || format_smolstr!("{from_name}!{at}"))?;
                }
            }
        }
        let plan = self.plan(&shift, &[], None, PartBytes::new())?;
        let mut restore = Restore::new(self);
        let (from_key, to_key) = (self.slots[source].key, self.slots[target].key);
        restore.push(Step::Cells {
            key: to_key,
            ranges: vec![landing],
            slice: Box::new(self.parsed(target)?.slice(landing)),
        });
        restore.push(Step::Cells {
            key: from_key,
            ranges: vec![block],
            slice: Box::new(self.parsed(source)?.slice(block)),
        });
        restore.capture_styles(self.styles()?.as_ref());
        self.stated.remember_slot(&self.slots[source]);
        self.stated.remember_slot(&self.slots[target]);
        self.follow(&shift, plan, None, &mut restore);
        let moved = {
            let held = self.slots[source].parsed.get_mut().expect("parsed above");
            let taken = held.slice(block);
            let cells: Vec<CellRef> = taken.cells().map(Cell::reference).collect();
            for at in cells {
                held.remove_cell(at);
            }
            taken
        };
        let held = self.slots[target].parsed.get_mut().expect("parsed above");
        let covered: Vec<CellRef> = held.cells_in(landing).map(Cell::reference).collect();
        for at in covered {
            held.remove_cell(at);
        }
        // An array's range, which lies in the block, moves with it - onto
        // the other sheet too; an input cell outside it follows the move.
        held.put_moved(&moved, block, to, |text| {
            let range: CellRange = text.parse().ok()?;
            let moved = if block.encloses(range) {
                range.moved_to(CellRef::new(
                    to.row() + (range.start().row() - block.start().row()),
                    to.column() + (range.start().column() - block.start().column()),
                ))
            } else {
                adjust_range(range, &from_name, &shift)?
            };
            Some(SmolStr::new(range_text(moved)))
        });
        Ok(restore)
    }

    /// Enter the text `tsv` - rows by line, cells by tab, a field in double
    /// quotes holding tabs, line breaks and doubled quotes, as Excel puts
    /// text on the clipboard - with its first cell at `anchor` of the sheet
    /// `sheet`, each cell typed as [`Self::set_entry`] types it; an empty
    /// field clears its cell's content. Answers the range entered. A line
    /// ends at `\n`, `\r\n` or a lone `\r`.
    ///
    /// ```
    /// use yggdryl::excel::Workbook;
    /// use yggdryl::Scalar;
    ///
    /// let mut workbook = Workbook::new();
    /// workbook.add_sheet("Sheet1")?;
    /// let range = workbook.paste_text("Sheet1", "B2".parse()?, "id\tname\n1\t\"Smith, \"\"J\"\"\"\n")?;
    /// assert_eq!(range.to_string(), "B2:C3");
    /// assert_eq!(workbook.sheet("Sheet1")?.scalar("C3".parse()?), Scalar::from("Smith, \"J\""));
    /// assert_eq!(workbook.sheet("Sheet1")?.scalar("B3".parse()?), Scalar::from(1.0));
    ///
    /// // A field that does not enter refuses the paste whole.
    /// workbook.paste_text("Sheet1", "E1".parse()?, "ok\t=SUM(").unwrap_err();
    /// assert_eq!(workbook.sheet("Sheet1")?.scalar("E1".parse()?), Scalar::Null);
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns, before any cell changes, [`Error::InvalidRecord`] for text
    /// landing off the grid or holding more than
    /// [`MAX_EDITED_CELLS`] fields, and the first
    /// refusal [`Self::set_entry`] answers.
    pub fn paste_text(&mut self, sheet: &str, anchor: CellRef, tsv: &str) -> Result<CellRange> {
        let (rows, span) = pasted(sheet, anchor, tsv)?;
        let step = self.cells_step(sheet, &[span])?;
        self.guarded(vec![step], |workbook| {
            workbook.enter_rows(sheet, anchor, &rows)
        })?;
        Ok(span)
    }

    /// Type each field of `rows` into its cell from `anchor` of the sheet
    /// `sheet`, stopping at the first refusal: [`Self::paste_text`] without
    /// its snapshot, for an edit that took one.
    pub(crate) fn enter_rows(
        &mut self,
        sheet: &str,
        anchor: CellRef,
        rows: &[Vec<String>],
    ) -> Result<()> {
        for (row, fields) in rows.iter().enumerate() {
            for (column, field) in fields.iter().enumerate() {
                let at = CellRef::new(anchor.row() + row as u32, anchor.column() + column as u32);
                self.set_entry(sheet, at, field)?;
            }
        }
        Ok(())
    }

    /// Sort the rows of `range` of the sheet `sheet` by `keys` - each a
    /// column of the sheet and its order - the first row left in place when
    /// `header`. The sort is stable and orders as Excel does: numbers, then
    /// text compared without case, then `FALSE` and `TRUE`, then errors,
    /// blanks last in either order. Each row's cells move whole, their
    /// formulas translated to the row they land on - as Excel moves them -
    /// and a reference from outside the range is left as it is.
    ///
    /// ```
    /// use yggdryl::excel::{SortKey, Workbook};
    /// use yggdryl::Scalar;
    ///
    /// let mut workbook = Workbook::new();
    /// let sheet = workbook.add_sheet("Sheet1")?;
    /// for (at, value) in [("A1", "name"), ("A2", "pear"), ("A3", "Apple"), ("A4", "fig")] {
    ///     sheet.set_cell(at.parse()?, value)?;
    /// }
    /// workbook.sort("Sheet1", "A1:A4".parse()?, &[SortKey { column: 0, descending: false }], true)?;
    /// let sheet = workbook.sheet("Sheet1")?;
    /// assert_eq!(sheet.scalar("A2".parse()?), Scalar::from("Apple"));
    /// assert_eq!(sheet.scalar("A4".parse()?), Scalar::from("pear"));
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a key outside the range's
    /// columns, a range meeting a merge or an array formula, or one of no
    /// row; what [`Self::sheet`] returns otherwise.
    pub fn sort(
        &mut self,
        sheet: &str,
        range: CellRange,
        keys: &[SortKey],
        header: bool,
    ) -> Result<()> {
        let at = self.resolve(sheet).ok_or_else(|| self.absent(sheet))?;
        let held = self.parsed(at)?;
        let refused = |reason: SmolStr| Error::InvalidRecord {
            path: format_smolstr!("{sheet}!{range}"),
            reason,
        };
        if let Some(key) = keys.iter().find(|key| !range.contains_column(key.column)) {
            return Err(refused(format_smolstr!(
                "expected sort keys inside the range's columns, got column {}",
                CellRef::column_name(key.column)
            )));
        }
        if let Some(merge) = held.merges().find(|merge| merge.intersects(range)) {
            return Err(refused(format_smolstr!(
                "expected a range meeting no merge, got {merge}"
            )));
        }
        if let Some((anchor, _)) = held
            .formula_ranges()
            .find(|(_, array)| array.intersects(range))
        {
            return Err(refused(format_smolstr!(
                "expected a range meeting no array formula, got the one anchored at {anchor}"
            )));
        }
        // An open range sorts the rows holding cells.
        let last = match held.dimension() {
            Some(span) if range.is_row_open() => span.end().row().min(range.end().row()),
            _ => range.end().row(),
        };
        let first = range.start().row() + u32::from(header);
        if first > last {
            return Ok(());
        }
        let body = CellRange::new(
            CellRef::new(first, range.start().column()),
            CellRef::new(last, range.end().column()),
        );
        let slice = held.slice(body);
        // Each row's keys, read once: the comparisons read them many times.
        let mut order: Vec<(u32, Vec<SortOperand>)> = (first..=last)
            .map(|row| {
                let values = keys
                    .iter()
                    .map(|key| sort_key(slice.cell(CellRef::new(row, key.column))))
                    .collect();
                (row, values)
            })
            .collect();
        order.sort_by(|(_, a), (_, b)| {
            for ((first, second), key) in a.iter().zip(b).zip(keys) {
                let ordering = compare_keys(first, second, key.descending);
                if ordering != std::cmp::Ordering::Equal {
                    return ordering;
                }
            }
            std::cmp::Ordering::Equal
        });
        let order: Vec<u32> = order.into_iter().map(|(row, _)| row).collect();
        if order.iter().copied().eq(first..=last) {
            return Ok(());
        }
        let held = self.slots[at].parsed.get_mut().expect("parsed above");
        held.put_sorted(&slice, body, &order);
        Ok(())
    }

    /// Start a nested Batch without retaining another style-table Arc.
    pub(crate) fn begin_batch(&mut self) -> BatchMark {
        self.stated.pivots.take();
        let attempt = self.stated.attempt.get_or_insert_with(Box::default);
        let start = attempt.before.len();
        attempt.marks.push(start);
        BatchMark {
            start,
            documents: self.documents,
            documents_saved: self.documents_saved,
            next_key: self.next_key,
            last_sheet_id: self.last_sheet_id,
            styles: self.styles.get().map(|table| table.checkpoint()),
            names_touched: self.stated.names_touched,
            views_touched: self.stated.views_touched,
            referring: self.stated.referring.get().cloned(),
            calculation_valid: self
                .stated
                .calculation
                .as_ref()
                .map(|calculation| calculation.valid),
        }
    }

    /// Keep a successful inner Batch's before-images for its parent.
    pub(crate) fn finish_batch(&mut self, start: usize) {
        let attempt = self.stated.attempt.as_mut().expect("Batch started above");
        assert_eq!(attempt.marks.pop(), Some(start));
        if attempt.marks.is_empty() {
            self.stated.attempt = None;
        }
    }

    /// Suppress new sheet stamps while generated inverses put payloads back.
    /// Package before-images still cover parts first published by an inverse.
    pub(crate) fn begin_rollback(&mut self) {
        self.stated
            .attempt
            .as_mut()
            .expect("Batch started above")
            .rollback = true;
    }

    /// Payload inverses ran first; restore the exact prior bookkeeping now.
    pub(crate) fn rollback_batch(&mut self, mark: BatchMark) {
        let Stated {
            attempt, overrides, ..
        } = self.stated.as_mut();
        let attempt = attempt.as_mut().expect("Batch started above");
        for before in attempt.before.drain(mark.start..).rev() {
            match before {
                Before::Part(name, Some(held)) => {
                    overrides.insert(name, held);
                }
                Before::Part(name, None) => {
                    overrides.remove(&name);
                }
                Before::Sheet(stamp) => {
                    // A sheet created and removed inside the failed Batch
                    // has no surviving slot whose stamp needs resetting.
                    let Some(slot) = self.slots.iter_mut().find(|slot| slot.key == stamp.key)
                    else {
                        continue;
                    };
                    slot.name = stamp.name;
                    slot.state = stamp.state;
                    slot.saved = stamp.saved;
                    if stamp.parsed.is_none() {
                        // Parsing during an aborted edit is derived state. The
                        // retained archive and restored overlays own its input.
                        slot.parsed = OnceLock::new();
                    } else if let (Some(sheet), Some((name, state, revision, changes))) =
                        (slot.parsed.get_mut(), stamp.parsed)
                    {
                        sheet.set_name(name).expect("a retained name was valid");
                        sheet.set_state(state);
                        // Before-images drain newest first; calculation cannot
                        // acknowledge a Sheet while its edit is in progress.
                        sheet
                            .restore_change_mark(changes)
                            .expect("the attempted sheet retains its calculation epoch");
                        sheet.reset_revision(revision);
                    }
                }
            }
        }
        attempt.rollback = false;
        match mark.styles {
            Some(checkpoint) => {
                let table = self.styles.get_mut().expect("the Batch began with styles");
                if table.checkpoint() != checkpoint {
                    Arc::make_mut(table).rollback(&checkpoint);
                }
            }
            None => self.styles = OnceLock::new(),
        }
        self.documents = mark.documents;
        self.documents_saved = mark.documents_saved;
        self.next_key = mark.next_key;
        self.last_sheet_id = mark.last_sheet_id;
        self.stated.names_touched = mark.names_touched;
        self.stated.views_touched = mark.views_touched;
        self.stated.referring = mark.referring.map_or_else(OnceLock::new, OnceLock::from);
        if let (Some(calculation), Some(valid)) =
            (&mut self.stated.calculation, mark.calculation_valid)
        {
            calculation.valid = valid;
        }
        // Keep override_revision monotonic. The old map entries retain their
        // exact old stamps; no aborted stamp is recycled into a later save.
        self.finish_batch(mark.start);
    }

    /// An inverse generated in this attempt needs no package planning.
    pub(crate) fn rollback_band(&mut self, name: &str, axis: Axis, band: Band) {
        // A zero-width forward band retained no changed slot stamp.
        match band {
            Band::Insert { count: 0, .. } => return,
            Band::Remove { start, end } if start >= end => return,
            _ => {}
        }
        let at = self
            .resolve(name)
            .expect("the opposite band retains its sheet");
        let name = self.slots[at].name.clone();
        let mut ignored = Restore::new(self);
        self.commit_band(at, &name, axis, band, Plan::default(), &mut ignored);
    }

    /// Reverse an attempted rename using already parsed formulas and names.
    /// The ledger restores package parts; no relationship planning is needed.
    pub(crate) fn rollback_rename(&mut self, name: &str, new_name: SmolStr) {
        let at = self
            .resolve(name)
            .expect("the opposite rename retains its sheet");
        let old_name = self.slots[at].name.clone();
        let shift = Shift::RenameSheet {
            from: &old_name,
            to: &new_name,
        };
        let mut ignored = Restore::new(self);
        self.follow(&shift, Plan::default(), None, &mut ignored);
        self.commit_name(at, new_name);
    }

    /// Only generated inverses of an insertion reach this path. Their next
    /// Restore puts overwritten references and workbook metadata back.
    pub(crate) fn rollback_remove(&mut self, name: &str) {
        let at = self
            .resolve(name)
            .expect("the inserted sheet is still present");
        self.slots.remove(at);
        self.remap_views(at);
    }

    /// No save occurs during an attempt, so original source parts still
    /// exist; the ledger restores overrides without a package graph read.
    pub(crate) fn rollback_sheet(&mut self, restored: RestoreSheet) {
        let RestoreSheet {
            position,
            slot,
            names,
            names_touched,
            views,
            views_touched,
            restore,
            ..
        } = restored;
        self.slots.insert(position, slot);
        self.stated.names = names;
        self.stated.names_touched = names_touched;
        self.stated.views = views;
        self.stated.views_touched = views_touched;
        self.commit_restore(restore);
    }

    /// Restore prior visibility without the last-visible-sheet refusal,
    /// which an intermediate inverse can cross.
    pub(crate) fn rollback_state(&mut self, name: &str, state: SheetState) {
        let at = self
            .resolve(name)
            .expect("the state inverse retains its sheet");
        // The unchanged forward state leaves a loaded worksheet unparsed.
        if self.slots[at].state() != state {
            self.commit_sheet_state(at, state);
        }
    }

    /// Restore a pane already validated before the attempted edit.
    pub(crate) fn rollback_frozen(&mut self, name: &str, frozen: Option<super::layout::Frozen>) {
        let at = self
            .resolve(name)
            .expect("the pane inverse retains its sheet");
        self.slots[at]
            .parsed
            .get_mut()
            .expect("the forward edit parsed its sheet")
            .restore_layout(None, None, None, Some(frozen));
    }

    /// Capture each workbook fact a historical RestoreSheet overwrites
    /// once, reusing a names step when its reference inverse already has it.
    pub(crate) fn sheet_metadata_inverse(&self, restore: &mut Restore) {
        if !restore
            .steps
            .iter()
            .any(|step| matches!(step, Step::Names(_)))
        {
            restore.push(Step::Names(self.stated.names.clone()));
        }
        if !restore
            .steps
            .iter()
            .any(|step| matches!(step, Step::Views(_)))
        {
            restore.push(Step::Views(self.stated.views.clone()));
        }
    }

    /// Capture the views a retained sheet restoration will overwrite.
    pub(crate) fn views_inverse(&self) -> Step {
        Step::Views(self.stated.views.clone())
    }

    /// Checkpoint a guarded edit's already retained cell/layout payload.
    /// These edits keep sheet membership, names and package parts unchanged.
    pub(crate) fn guard_checkpoint(&mut self, steps: &[Step]) -> Result<GuardCheckpoint> {
        let styles = self.style_sheet()?.checkpoint();
        let mut revisions = SmallVec::<[(SheetKey, u64, Option<ChangeMark>); 1]>::new();
        for step in steps {
            let key = match step {
                Step::Cells { key, .. }
                | Step::Layout { key, .. }
                | Step::RecordFootprint { key, .. } => *key,
                _ => unreachable!("guarded edits retain only cells and layout"),
            };
            if revisions.iter().any(|(held, _, _)| *held == key) {
                continue;
            }
            let at = self
                .slots
                .iter()
                .position(|slot| slot.key == key)
                .ok_or_else(|| Error::absent("worksheet", key.as_u32()))?;
            let sheet = self.parsed(at)?;
            revisions.push((key, sheet.revision(), sheet.change_mark()));
            self.stated.remember_slot(&self.slots[at]);
        }
        Ok(GuardCheckpoint { styles, revisions })
    }

    /// Put back the payload before dropping appended styles and edit stamps.
    /// No save, membership change or style-ID relocation occurs in a guard.
    pub(crate) fn rollback_guard(&mut self, restore: Restore, checkpoint: GuardCheckpoint) {
        self.commit_restore(restore);
        let table = self.styles.get_mut().expect("the guard read its styles");
        // A refusal before any style append must not clone a table shared
        // with an in-flight package just to truncate it to its current size.
        if table.checkpoint() != checkpoint.styles {
            Arc::make_mut(table).rollback(&checkpoint.styles);
        }
        for (key, revision, changes) in checkpoint.revisions {
            let sheet = self.sheet_of_key(key);
            sheet
                .restore_change_mark(changes)
                .expect("a guarded edit retains its calculation epoch");
            sheet.reset_revision(revision);
        }
    }

    /// Validate every step before publishing any payload or rebound style.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] for another workbook's inverse,
    /// [`Error::Absent`] for a step naming a sheet the workbook no
    /// longer holds, or [`Error::InvalidRecord`] before any step changes
    /// the workbook when a step removes required package metadata.
    fn check_restore(&self, restore: &Restore, restored: Option<SheetKey>) -> Result<()> {
        restore.check_origin(self)?;
        // A later part step must not refuse after an earlier cell, name or
        // part changed. Required documents cannot be restored as absent.
        for step in &restore.steps {
            let key = match step {
                Step::Cells { key, .. }
                | Step::Formulas { key, .. }
                | Step::FormulaAttributes { key, .. }
                | Step::Layout { key, .. }
                | Step::RecordFootprint { key, .. }
                | Step::Frame { key, .. } => Some(*key),
                Step::Names(_) | Step::Views(_) | Step::Overrides(_)
                | Step::Calculation { .. } => None,
            };
            if let Some(key) = key.filter(|key| Some(*key) != restored) {
                let at = self
                    .slots
                    .iter()
                    .position(|slot| slot.key == key)
                    .ok_or_else(|| Error::absent("worksheet", key.as_u32()))?;
                self.parsed(at)?;
            }
            if let Step::Overrides(parts) = step {
                for PartRestore { member, bytes, .. } in parts {
                    if bytes.is_none()
                        && (member == package::CONTENT_TYPES_PART
                            || member == package::ROOT_RELATIONSHIPS_PART
                            || member == &self.workbook_part
                            || *member == package::relationships_part_of(&self.workbook_part))
                    {
                        return Err(Error::InvalidRecord {
                            path: member.clone(),
                            reason: SmolStr::new_static(
                                "expected required workbook metadata to remain present",
                            ),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Reconcile an ownership document against its retained post-edit image.
    /// Only added workbook styles/shared strings can survive without changing
    /// a retained worksheet frame; other owners or opaque context refuse.
    /// Every byte here was already read for the inverse, never from a new handle.
    pub(crate) fn restored_relationships(
        &self,
        member: &str,
        expected: &[u8],
        current: &[u8],
        target: Option<&[u8]>,
    ) -> Result<Option<Arc<[u8]>>> {
        let source = package::source_of_relationships(member)
            .expect("only relationship documents retain an expected image");
        let expected_edges = Relationships::from_xml(expected, &source)?;
        let current_edges = Relationships::from_xml(current, &source)?;
        let (expected_at, expected_root) = Registration::root_at(expected)?;
        let (current_at, current_root) = Registration::root_at(current)?;
        let expected_entries = expected_root.members(false)?;
        let current_entries = current_root.members(false)?;
        let conflict = |id: &str, target: Option<&str>| Error::Conflict {
            expected: "the retained package relationship ownership",
            actual: "a relationship identity or opaque context changed since this inverse",
            path: match target {
                Some(target) => format_smolstr!("{member}#{id} ({target})"),
                None => format_smolstr!("{member}#{id}"),
            },
        };
        let unique = |entries: &[Registration]| {
            let keys: BTreeSet<_> = entries.iter().map(|entry| &entry.key).collect();
            keys.len() == entries.len() && !keys.iter().any(|key| key.is_empty())
        };
        if !unique(&expected_entries) || !unique(&current_entries) {
            return Err(conflict("Relationship", None));
        }
        // Compare resolved edges, including an internal/external transition,
        // before raw declarations: equal bytes under different scope are not
        // proof that the target still belongs to this owner.
        for edge in expected_edges.entries() {
            if current_edges.by_id(&edge.id) != Some(edge) {
                return Err(conflict(&edge.id, edge.target.as_deref()));
            }
        }
        for entry in &expected_entries {
            if current_entries
                .iter()
                .find(|held| held.key == entry.key)
                .is_none_or(|held| held.xml != entry.xml)
            {
                return Err(conflict(
                    &entry.key,
                    expected_edges
                        .by_id(&entry.key)
                        .and_then(|edge| edge.target.as_deref()),
                ));
            }
        }
        // The full historical document would overwrite root attributes,
        // extension children and surrounding text. Refuse those differences
        // rather than silently discarding an unrelated edit.
        let expected_ids = expected_entries
            .iter()
            .map(|entry| entry.key.clone())
            .collect();
        let current_ids = current_entries
            .iter()
            .map(|entry| entry.key.clone())
            .collect();
        if expected_root.without(false, &expected_ids)?.xml
            != current_root.without(false, &current_ids)?.xml
        {
            return Err(conflict("Relationships", None));
        }
        if expected[..expected_at] != current[..current_at]
            || expected[expected_at + expected_root.xml.len()..]
                != current[current_at + current_root.xml.len()..]
        {
            return Err(conflict("Relationships", None));
        }
        let added: Vec<_> = current_entries
            .into_iter()
            .filter(|entry| !expected_ids.contains(&entry.key))
            .collect();
        if added.is_empty() {
            return Err(conflict("Relationships", None));
        }
        let Some(target) = target else {
            // Removing the entire document would also erase these later IDs.
            return Err(conflict(&added[0].key, None));
        };
        let target_root = Registration::root(target)?;
        let target_entries = target_root.members(false)?;
        if !unique(&target_entries) {
            return Err(conflict("Relationship", None));
        }
        for entry in &added {
            let edge = current_edges.by_id(&entry.key);
            let safe = source == self.workbook_part
                && edge.is_some_and(|edge| {
                    let owned = match edge.kind {
                        RelationshipKind::Styles => self.styles_part.as_ref(),
                        RelationshipKind::SharedStrings => self.strings_part.as_ref(),
                        _ => None,
                    };
                    owned.is_some()
                        && edge.target.as_ref() == owned
                        && expected_edges.first_of(edge.kind).is_none()
                        && current_edges
                            .entries()
                            .iter()
                            .filter(|held| held.kind == edge.kind)
                            .count()
                            == 1
                });
            // Even byte-equal IDs newly occupied since the expected image
            // cannot be claimed by a historical relationship. Other added
            // owners could be detached when an old worksheet Frame returns.
            if !safe || target_entries.iter().any(|held| held.key == entry.key) {
                return Err(conflict(
                    &entry.key,
                    edge.and_then(|edge| edge.target.as_deref()),
                ));
            }
        }
        Registration::merge(
            target,
            member,
            b"Relationship",
            b"Id",
            b"Relationships",
            &added,
        )
        .map(|bytes| Some(bytes.unwrap_or_else(|| Arc::from(target))))
    }

    /// Put back the preflighted payload under today's style meanings.
    pub(crate) fn restore(&mut self, mut restore: Restore) -> Result<Restore> {
        self.check_restore(&restore, None)?;
        let inverse = self.restore_inverse(&mut restore)?;
        let (styles, moved) = self.styles()?.rebind(restore.styles.iter())?;
        restore.remap_styles(&moved);
        self.styles = OnceLock::from(styles);
        self.commit_restore(restore);
        Ok(inverse)
    }

    /// Commit owned payload whose keys and style IDs were already proved.
    /// The preflight/commit pair cannot change membership between calls.
    pub(crate) fn commit_restore(&mut self, restore: Restore) {
        for step in restore.steps {
            match step {
                Step::Names(names) => {
                    self.invalidate_calculation();
                    self.stated.names = names;
                    self.stated.names_touched = true;
                }
                Step::Views(views) => {
                    self.stated.views = views;
                    self.stated.views_touched = true;
                }
                Step::Overrides(parts) => {
                    for PartRestore { member, bytes, .. } in parts {
                        self.stated.set_part(member, bytes);
                    }
                    self.stated.referring = OnceLock::new();
                }
                Step::Calculation { pass, status } => {
                    self.restore_calculation(pass, status);
                }
                Step::Cells { key, ranges, slice } => {
                    self.sheet_of_key(key).restore_cells(&ranges, &slice);
                }
                Step::RecordFootprint { key, span } => {
                    self.sheet_of_key(key).restore_record_footprint(span);
                }
                Step::Formulas { key, cells } => {
                    let sheet = self.sheet_of_key(key);
                    for (at, formula) in cells {
                        sheet.restore_formula(at, formula);
                    }
                }
                Step::FormulaAttributes { key, cells } => {
                    let sheet = self.sheet_of_key(key);
                    for (at, attributes) in cells {
                        sheet.restore_formula_attributes(at, attributes);
                    }
                }
                Step::Layout {
                    key,
                    rows,
                    columns,
                    merges,
                    pane,
                } => {
                    self.sheet_of_key(key)
                        .restore_layout(rows, columns, merges, pane);
                }
                Step::Frame { key, frame } => {
                    self.sheet_of_key(key).set_frame(frame);
                    // A full restore can also change the inherited
                    // namespace bindings that resolve tablePart IDs.
                    self.stated.referring = OnceLock::new();
                }
            }
        }
    }

    /// What a removal of a sheet named `name` would rewrite, as a restore
    /// putting it back: each formula naming it, the defined names, the
    /// carried children and the charts and pivot caches naming it.
    pub(crate) fn naming(&self, name: &str) -> Result<Restore> {
        self.parse_all()?;
        let mut restore = Restore::new(self);
        for slot in &self.slots {
            let Some(sheet) = slot.parsed.get() else {
                continue;
            };
            let cells: Vec<(CellRef, Option<super::formula::Formula>)> = sheet
                .cells()
                .filter(|cell| cell.formula().is_some_and(|formula| formula.names(name)))
                .map(|cell| (cell.reference(), cell.formula().cloned()))
                .collect();
            if !cells.is_empty() {
                restore.push(Step::Formulas {
                    key: slot.key,
                    cells,
                });
            }
            if let Some(frame) = sheet.frame() {
                if frame
                    .items
                    .iter()
                    .any(|item| shift::names_sheet_in(&item.bytes, name))
                {
                    restore.push(Step::Frame {
                        key: slot.key,
                        frame: Some(Box::new(frame.clone())),
                    });
                }
            }
        }
        if self
            .stated
            .names
            .iter()
            .any(|defined| defined.formula().names(name))
        {
            restore.push(Step::Names(self.stated.names.clone()));
        }
        // What a removal rewrites beside the sheets: the charts and the
        // drawings naming it.
        let mut fresh = PartBytes::new();
        let referring = self.referring(&[], None, &[], &mut fresh)?;
        let mut parts = Vec::new();
        for entry in referring
            .iter()
            .filter(|entry| entry.kind != Referrer::Cache)
        {
            let names = match (
                self.stated
                    .overrides
                    .get(&entry.member)
                    .and_then(|held| held.bytes.as_ref()),
                &entry.references,
            ) {
                (Some(bytes), _) => shift::names_sheet_in(bytes, name),
                (None, Some(references)) => shift::names_sheet_in(references.as_bytes(), name),
                (None, None) => true,
            };
            if names {
                let bytes = match fresh.get(&entry.member) {
                    Some(bytes) => Arc::clone(bytes),
                    None => self.part_bytes(&entry.member)?,
                };
                parts.push(PartRestore::new(
                    entry.member.clone(),
                    Some(Arc::clone(&bytes)),
                    Some(bytes),
                ));
                if let Some((member, bytes)) = entry.ownership(self)? {
                    if !parts.iter().any(|held| held.member == member) {
                        parts.push(PartRestore::new(
                            member,
                            Some(Arc::clone(&bytes)),
                            Some(bytes),
                        ));
                    }
                }
            }
        }
        if !parts.is_empty() {
            restore.push(Step::Overrides(parts));
        }
        Ok(restore)
    }

    /// Whether the sheet `name` shows in the tabs, `None` for a name no
    /// sheet has.
    #[must_use]
    pub fn sheet_state(&self, name: &str) -> Option<SheetState> {
        self.resolve(name).map(|at| self.slots[at].state())
    }

    /// Show or hide the sheet `name` - a chart or dialog sheet as well as a
    /// worksheet.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] for a name no sheet has, and
    /// [`Error::InvalidRecord`] for hiding the last visible worksheet, which
    /// Excel refuses.
    pub fn set_sheet_state(&mut self, name: &str, state: SheetState) -> Result<()> {
        let at = self.resolve(name).ok_or_else(|| self.absent(name))?;
        if self.slots[at].state() == state {
            return Ok(());
        }
        if state != SheetState::Visible
            && !self.slots.iter().enumerate().any(|(index, slot)| {
                index != at
                    && slot.kind == SheetKind::Worksheet
                    && slot.state() == SheetState::Visible
            })
        {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{}", self.slots[at].name),
                reason: SmolStr::new_static(
                    "expected another visible worksheet to be left, got none",
                ),
            });
        }
        self.commit_sheet_state(at, state);
        Ok(())
    }

    fn commit_sheet_state(&mut self, at: usize, state: SheetState) {
        self.stated.remember_slot(&self.slots[at]);
        // Visibility belongs to workbook metadata. A cold worksheet inherits
        // the slot's state if it is read later; changing a tab needs no cells.
        if let Some(sheet) = self.slots[at].parsed.get_mut() {
            sheet.set_state(state);
        } else {
            self.slots[at].state = state;
            self.documents += 1;
        }
    }

    /// Everything the workbook holds that an edit may change, spelled out -
    /// each tab, each parsed sheet's cells, what they state beside them, its
    /// layout and what its part carries, the names, the views and the parts
    /// an edit rewrote - and nothing about how often any changed: what an
    /// undo is pinned to give back.
    #[cfg(feature = "internals")]
    pub(crate) fn describe(&self) -> String {
        use std::fmt::Write as _;

        let mut text = String::new();
        for slot in &self.slots {
            let _ = writeln!(
                text,
                "sheet {} key={} id={} kind={:?} state={:?} part={:?}",
                slot.name,
                slot.key.as_u32(),
                slot.sheet_id,
                slot.kind,
                slot.state(),
                slot.part
            );
            if let Some(sheet) = slot.parsed.get() {
                text.push_str(&sheet.describe());
            }
        }
        for defined in &self.stated.names {
            let _ = writeln!(
                text,
                "name {} scope={:?} text={} hidden={} comment={:?}",
                defined.name(),
                defined.scope().map(SheetKey::as_u32),
                defined.text(),
                defined.is_hidden(),
                defined.comment()
            );
        }
        let _ = writeln!(text, "views {:?}", self.stated.views);
        // Each part an edit may rewrite, as the workbook holds it - an
        // edit's rewrite or the member - so a part put back reads alike
        // whichever holds it.
        let rewritable = |member: &str| {
            [
                "xl/tables/",
                "xl/drawings/",
                "xl/comments",
                "xl/threadedComments/",
                "xl/pivotTables/",
                "xl/charts/chart",
                "xl/pivotCache/pivotCacheDefinition",
            ]
            .iter()
            .any(|prefix| member.starts_with(prefix))
                && !member.contains("/_rels/")
        };
        let members: BTreeSet<SmolStr> = self
            .members()
            .unwrap_or_default()
            .into_iter()
            .filter(|member| rewritable(member))
            .collect();
        for member in members {
            let bytes = self.part_bytes(&member).unwrap_or_else(|_| Arc::from([]));
            let _ = writeln!(text, "part {member} {}", String::from_utf8_lossy(&bytes));
        }
        text
    }

    /// A restore target proved present and parsed before its commit.
    fn sheet_of_key(&mut self, key: SheetKey) -> &mut Sheet {
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.key == key)
            .expect("the restore target was checked before mutation");
        self.stated.remember_slot(slot);
        slot.parsed
            .get_mut()
            .expect("the restore target was parsed before mutation")
    }

    /// The bytes of the member `name` as the workbook holds it: as an edit
    /// rewrote it, else as it is stored.
    pub(crate) fn part_bytes(&self, name: &str) -> Result<Arc<[u8]>> {
        if let Some(held) = self.stated.overrides.get(name) {
            return held
                .bytes
                .clone()
                .ok_or_else(|| Error::absent("workbook part", name));
        }
        Ok(Arc::from(self.source.read(name)?))
    }

    /// Stream a serialized image from the same effective package as part reads.
    fn part_reader(&self, name: &str) -> Result<Box<dyn Read + Send>> {
        match self.stated.overrides.get(name).map(|held| &held.bytes) {
            Some(Some(bytes)) => Ok(Box::new(std::io::Cursor::new(Arc::clone(bytes)))),
            Some(None) => Err(Error::absent("workbook part", name)),
            None => self.source.reader(name),
        }
    }

    /// The effective presence of a part, retained by an inverse before
    /// additions or removals. An explicit removal never reads the source.
    pub(crate) fn part_bytes_if_present(&self, name: &str) -> Result<Option<Arc<[u8]>>> {
        if let Some(held) = self.stated.overrides.get(name) {
            return Ok(held.bytes.clone());
        }
        if self.source.contains(name)? {
            self.source.read(name).map(|bytes| Some(Arc::from(bytes)))
        } else {
            Ok(None)
        }
    }

    /// The current package inventory, including parts an undo restored since save.
    fn members(&self) -> Result<Vec<SmolStr>> {
        let mut members = self.source.names()?;
        members.retain(|name| {
            !self
                .stated
                .overrides
                .get(name)
                .is_some_and(|held| held.bytes.is_none())
        });
        for (name, held) in &self.stated.overrides {
            if held.bytes.is_some() && !members.contains(name) {
                members.push(name.clone());
            }
        }
        Ok(members)
    }

    fn copy_part_into(&self, target: &ZipArchive, name: &str) -> Result<()> {
        match self.stated.overrides.get(name).map(|held| &held.bytes) {
            Some(Some(bytes)) => target
                .write_member_with(name, bytes, Codec::Deflate)
                .map(|_| ()),
            Some(None) => Err(Error::absent("workbook part", name)),
            None => self.source.copy_into(target, name),
        }
    }

    fn removed_roots(&self, relationships: &Relationships) -> Vec<SmolStr> {
        relationships
            .entries()
            .iter()
            .filter(|relationship| relationship.kind.is_sheet())
            .filter_map(|relationship| relationship.target.clone())
            .filter(|part| !self.slots.iter().any(|slot| slot.part == *part))
            .collect()
    }

    /// Read the prospective orphan delta before references or parts change.
    fn removed_parts(&self, at: usize) -> Result<RemovedParts> {
        let slot = &self.slots[at];
        let part = &slot.part;
        let members = self.members()?;
        if !slot.backed && !members.contains(&package::relationships_part_of(part)) {
            return Ok(RemovedParts::default());
        }
        let relationships_part = package::relationships_part_of(&self.workbook_part);
        let relationships_bytes = self.part_bytes(&relationships_part)?;
        let relationships = Relationships::from_xml(&relationships_bytes, &self.workbook_part)?;
        let graph = self.part_graph(&members, (&relationships_part, &relationships), None)?;
        let mut removed = self.removed_roots(&relationships);
        let before = graph.dropped(&removed);
        removed.push(part.clone());
        let after = graph.dropped(&removed);
        let mut retained: BTreeSet<SmolStr> = after.difference(&before).cloned().collect();
        for part in retained.clone() {
            let related = package::relationships_part_of(&part);
            if members.contains(&related) {
                retained.insert(related);
            }
        }
        let mut held = RemovedParts::default();
        for name in &retained {
            if members.contains(name) {
                held.parts.push((name.clone(), self.part_bytes(name)?));
            }
        }
        if members
            .iter()
            .any(|member| member == package::CONTENT_TYPES_PART)
        {
            held.types = Registration::read(
                &self.part_bytes(package::CONTENT_TYPES_PART)?,
                b"Override",
                b"PartName",
            )?
            .into_iter()
            .filter(|entry| retained.contains(entry.key.trim_start_matches('/')))
            .collect();
        }
        let restored_ids: BTreeSet<_> = relationships
            .entries()
            .iter()
            .filter(|relationship| {
                relationship
                    .target
                    .as_ref()
                    .is_some_and(|target| retained.contains(target))
            })
            .map(|relationship| relationship.id.clone())
            .collect();
        held.relationships = Registration::read(&relationships_bytes, b"Relationship", b"Id")?
            .into_iter()
            .filter(|entry| restored_ids.contains(&entry.key))
            .collect();
        for entry in Registration::read(
            &self.part_bytes(&self.workbook_part)?,
            b"pivotCache",
            b"cacheId",
        )? {
            if restored_ids.contains(&entry.relationship_key(&self.workbook_part)?.1) {
                held.caches.push(entry);
            }
        }
        Ok(held)
    }

    /// Validate identities and merge declarations into today's metadata, never
    /// replace a document with the version held before other edits and saves.
    fn restored_parts(
        &self,
        slot: &Slot,
        held: &RemovedParts,
    ) -> Result<Vec<(SmolStr, Arc<[u8]>)>> {
        if held.parts.is_empty() {
            return Ok(Vec::new());
        }
        let members = self.members()?;
        let relationships_part = package::relationships_part_of(&self.workbook_part);
        let relationships_bytes = self.part_bytes(&relationships_part)?;
        let relationships = Relationships::from_xml(&relationships_bytes, &self.workbook_part)?;
        let removed = self.removed_roots(&relationships);
        let disposable = self
            .part_graph(&members, (&relationships_part, &relationships), None)?
            .dropped(&removed);
        for (name, bytes) in &held.parts {
            if let Some(source) = package::source_of_relationships(name) {
                for relationship in Relationships::from_xml(bytes, &source)?.entries() {
                    if let Some(target) = &relationship.target {
                        // An orphan still physically present can become live again.
                        // An out-of-order inverse cannot recreate bytes another inverse owns.
                        if !members.contains(target) && !held.contains(target) {
                            return Err(Error::Conflict {
                                expected: "the removed sheet's package dependency",
                                actual: "a dependency absent from the current package and this inverse",
                                path: format_smolstr!("{} ({name} -> {target})", slot.name),
                            });
                        }
                    }
                }
            }
            let owned = self.slots.iter().any(|held| held.part == *name);
            let orphan = disposable.contains(name)
                || package::source_of_relationships(name)
                    .is_some_and(|source| disposable.contains(&source));
            if owned
                || (members.contains(name)
                    && !orphan
                    && self.part_bytes(name)?.as_ref() != bytes.as_ref())
            {
                return Err(Error::Conflict {
                    expected: "the removed sheet's unused package part",
                    actual: "a part now held by another live value",
                    path: format_smolstr!("{} ({name})", slot.name),
                });
            }
        }
        let mut restored = held.parts.clone();
        for (part, element, key, parent, entries) in [
            (
                package::CONTENT_TYPES_PART,
                b"Override".as_slice(),
                b"PartName".as_slice(),
                b"Types".as_slice(),
                held.types.as_slice(),
            ),
            (
                relationships_part.as_str(),
                b"Relationship".as_slice(),
                b"Id".as_slice(),
                b"Relationships".as_slice(),
                held.relationships.as_slice(),
            ),
            (
                self.workbook_part.as_str(),
                b"pivotCache".as_slice(),
                b"cacheId".as_slice(),
                b"pivotCaches".as_slice(),
                held.caches.as_slice(),
            ),
        ] {
            if !entries.is_empty() {
                if let Some(bytes) = Registration::merge(
                    &self.part_bytes(part)?,
                    part,
                    element,
                    key,
                    parent,
                    entries,
                )? {
                    restored.push((part.into(), bytes));
                }
            }
        }
        Ok(restored)
    }

    /// What the part of the worksheet at `at` names through its
    /// relationships: each member and its kind.
    fn relations(&self, at: usize) -> Result<Vec<(RelationshipKind, SmolStr)>> {
        self.relations_read(at, None, None)
    }

    /// Retain bytes already read while discovering ownership for the same
    /// plan's subsequent .rels rewrite, without a second handle read.
    fn relations_read(
        &self,
        at: usize,
        cached: Option<&mut PartBytes>,
        has_notes: Option<&mut bool>,
    ) -> Result<Vec<(RelationshipKind, SmolStr)>> {
        let slot = &self.slots[at];
        let members = self.members()?;
        let part = package::relationships_part_of(&slot.part);
        let relationships = if members.contains(&part) {
            let bytes = match cached.as_ref().and_then(|cache| cache.get(&part)) {
                Some(bytes) => Arc::clone(bytes),
                None => self.part_bytes(&part)?,
            };
            let relationships = Relationships::from_xml(&bytes, &slot.part)?;
            if let Some(cache) = cached {
                cache.insert(part, bytes);
            }
            relationships
        } else {
            Relationships::default()
        };
        // Derive cut eligibility from the relationships already resolved here,
        // before target filtering: malformed note targets must still be checked.
        if let Some(has_notes) = has_notes {
            *has_notes |= relationships.entries().iter().any(|relationship| {
                matches!(
                    relationship.kind,
                    RelationshipKind::Comments | RelationshipKind::ThreadedComment
                )
            });
        }
        let registrations = match slot.parsed.get().and_then(Sheet::frame) {
            Some(frame) => Registration::table_parts(frame, &slot.part)?,
            None => Vec::new(),
        };
        let mut result: Vec<_> = relationships
            .entries()
            .iter()
            .filter(|relationship| relationship.kind != RelationshipKind::Table)
            .filter_map(|relationship| Some((relationship.kind, relationship.target.clone()?)))
            .filter(|(_, target)| members.contains(target))
            .collect();
        Self::table_targets(
            &relationships,
            registrations,
            &members,
            &slot.part,
            &mut result,
        )?;
        Ok(result)
    }

    /// Resolve actual table memberships through one relationship validator.
    fn table_targets(
        relationships: &Relationships,
        registrations: Vec<Registration>,
        members: &[SmolStr],
        part: &str,
        result: &mut Vec<(RelationshipKind, SmolStr)>,
    ) -> Result<()> {
        let mut seen = BTreeSet::new();
        for registration in registrations {
            let refusal = |reason| Error::InvalidRecord {
                path: format_smolstr!("{}#tablePart[{}]", part, registration.key),
                reason,
            };
            let mut matching = relationships
                .entries()
                .iter()
                .filter(|relationship| relationship.id == registration.key);
            let relationship = matching.next().ok_or_else(|| {
                refusal(SmolStr::new_static(
                    "expected a table relationship, got a missing relationship",
                ))
            })?;
            if matching.next().is_some() {
                return Err(refusal(SmolStr::new_static(
                    "expected one table relationship, got a duplicate relationship id",
                )));
            }
            if relationship.kind != RelationshipKind::Table {
                return Err(refusal(format_smolstr!(
                    "expected a table relationship, got {:?}",
                    relationship.kind,
                )));
            }
            let target = relationship.target.as_ref().ok_or_else(|| {
                refusal(SmolStr::new_static(
                    "expected an internal table part, got an external or absent target",
                ))
            })?;
            if !members.contains(target) {
                return Err(refusal(format_smolstr!(
                    "expected a table part, got missing {target}"
                )));
            }
            if !seen.insert(target.clone()) {
                return Err(refusal(format_smolstr!(
                    "expected one membership per table, got duplicate relationship to {target}",
                )));
            }
            result.push((RelationshipKind::Table, target.clone()));
        }
        Ok(())
    }

    /// Visit each actual named table once, giving its columns directly to
    /// the consumer. The returned name registry already owns duplicate-name
    /// validation and supplies diagnostics without building another registry.
    pub(crate) fn visit_tables(
        &self,
        sheet: Option<&str>,
        mut visit: impl FnMut(usize, &str, Table, Vec<SmolStr>, &str, Arc<[u8]>) -> Result<()>,
    ) -> Result<BTreeMap<String, (SmolStr, SmolStr, SmolStr)>> {
        let members = self.members()?;
        let mut names = BTreeMap::new();
        let mut owned = BTreeSet::new();
        for (tab, slot) in self.slots.iter().enumerate() {
            if slot.kind != SheetKind::Worksheet
                || sheet.is_some_and(|name| !slot.name.eq_ignore_ascii_case(name))
            {
                continue;
            }
            let registrations = if let Some(frame) = slot.parsed.get().and_then(Sheet::frame) {
                Registration::table_parts(frame, &slot.part)?
            } else if slot.backed {
                let mut rows = SheetRows::observing_tables(
                    std::io::BufReader::with_capacity(
                        crate::DEFAULT_FETCH_BYTE_SIZE,
                        self.part_reader(&slot.part)?,
                    ),
                    slot.name.clone(),
                );
                while let Some(row) = rows.next() {
                    rows.recycle(row?.cells);
                }
                match rows.into_frame() {
                    Some((frame, _)) => Registration::table_parts(&frame, &slot.part)?,
                    None => Vec::new(),
                }
            } else {
                Vec::new()
            };
            if registrations.is_empty() {
                continue;
            }
            let rels_part = package::relationships_part_of(&slot.part);
            let relationships = if members.contains(&rels_part) {
                Relationships::from_xml(&self.part_bytes(&rels_part)?, &slot.part)?
            } else {
                Relationships::default()
            };
            let mut targets = Vec::new();
            Self::table_targets(
                &relationships,
                registrations,
                &members,
                &slot.part,
                &mut targets,
            )?;
            for (_, part) in targets {
                if !owned.insert(part.clone()) {
                    return Err(Error::InvalidRecord {
                        path: part,
                        reason: SmolStr::new_static(
                            "expected a table owned by one worksheet, got multiple owners",
                        ),
                    });
                }
                let bytes = self.part_bytes(&part)?;
                let (table, columns) = Table::read_named(&bytes, &part)?;
                if let Some((_, _, previous)) = names.insert(
                    table.name.to_ascii_lowercase(),
                    (slot.name.clone(), table.name.clone(), part.clone()),
                ) {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::new_static("$.table"),
                        reason: format_smolstr!(
                            "expected a unique table name, got {:?} in {previous} and {part}",
                            table.name
                        ),
                    });
                }
                visit(tab, &slot.name, table, columns, &part, bytes)?;
            }
        }
        Ok(names)
    }

    /// Resolve a named table without parsing any worksheet into a Sheet.
    /// The metadata observer retains only the root scope and tableParts;
    /// tableColumns are read from each registered table exactly once.
    pub(crate) fn named_table(&self, name: &str) -> Result<NamedTable> {
        let mut selected = None;
        let mut ranges = SmallVec::<[(usize, SmolStr, CellRange); 4]>::new();
        let names = self.visit_tables(None, |tab, sheet, table, columns, part, bytes| {
            ranges.push((tab, table.name.clone(), table.range));
            if table.name.eq_ignore_ascii_case(name) {
                selected = Some((
                    tab,
                    NamedTable {
                        sheet: SmolStr::new(sheet),
                        table,
                        columns,
                        part: SmolStr::new(part),
                        bytes,
                        siblings: SmallVec::new(),
                    },
                ));
            }
            Ok(())
        })?;
        let (tab, mut selected) = selected.ok_or_else(|| {
            let available: Vec<_> = names
                .values()
                .map(|(sheet, table, part)| format_smolstr!("{sheet}!{table} ({part})"))
                .collect();
            Error::InvalidRecord {
                path: SmolStr::new_static("$.table"),
                reason: format_smolstr!(
                    "expected a named table, got {name:?}; available [{}]",
                    available.join(", ")
                ),
            }
        })?;
        for (owner, other, range) in ranges {
            if owner == tab && !other.eq_ignore_ascii_case(&selected.table.name) {
                selected.siblings.push((other, range));
            }
        }
        Ok(selected)
    }
    /// What the member `part` names through its relationships: each member
    /// the package holds, and its kind.
    fn relations_of(&self, part: &str) -> Result<Vec<(RelationshipKind, SmolStr)>> {
        let members = self.members()?;
        let relationships = package::relationships_part_of(part);
        if !members.contains(&relationships) {
            return Ok(Vec::new());
        }
        Ok(
            Relationships::from_xml(&self.part_bytes(&relationships)?, part)?
                .entries()
                .iter()
                .filter_map(|relationship| Some((relationship.kind, relationship.target.clone()?)))
                .filter(|(_, target)| members.contains(target))
                .collect(),
        )
    }

    /// Move each view's tabs past the removed tab `at` down by one; a view
    /// showing the removed tab shows the nearest visible one after it, else
    /// before it.
    fn remap_views(&mut self, at: usize) {
        if self.stated.views.is_empty() {
            return;
        }
        let visible = |slot: &Slot| slot.state() == SheetState::Visible;
        let start = at.min(self.slots.len());
        let nearest = self.slots[start..]
            .iter()
            .position(visible)
            .map(|offset| start + offset)
            .or_else(|| self.slots[..start].iter().rposition(visible))
            .unwrap_or(0);
        let last = self.slots.len().saturating_sub(1);
        for view in &mut self.stated.views {
            view.active_tab = view.active_tab.map(|tab| match tab.cmp(&at) {
                std::cmp::Ordering::Less => tab,
                std::cmp::Ordering::Equal => nearest,
                std::cmp::Ordering::Greater => tab - 1,
            });
            view.first_sheet = view
                .first_sheet
                .map(|tab| if tab > at { tab - 1 } else { tab.min(last) });
        }
        self.stated.views_touched = true;
    }

    /// The tab the workbook opens on: the first view's `activeTab`, the
    /// first tab when the workbook states none.
    #[must_use]
    pub fn active_tab(&self) -> usize {
        self.stated
            .views
            .first()
            .and_then(|view| view.active_tab)
            .filter(|tab| *tab < self.slots.len())
            .unwrap_or(0)
    }

    /// The workbook's defined names, in the order its workbook part lists
    /// them.
    pub fn defined_names(&self) -> impl Iterator<Item = &DefinedName> + '_ {
        self.stated.names.iter()
    }

    /// The position of the sheet `name` in tab order, compared without case.
    pub(crate) fn position(&self, name: &str) -> Option<usize> {
        self.resolve(name)
    }

    /// The first worksheet in tab order, which a read addresses when its
    /// options name no sheet.
    pub(crate) fn first_worksheet(&self) -> Option<&str> {
        self.slots
            .iter()
            .find(|slot| slot.kind == SheetKind::Worksheet)
            .map(|slot| slot.name.as_str())
    }

    /// The parsed sheet at `at`, parsing its part on first access.
    fn parsed(&self, at: usize) -> Result<&Sheet> {
        let slot = &self.slots[at];
        if slot.kind != SheetKind::Worksheet {
            return Err(self.not_a_worksheet(at));
        }
        if let Some(sheet) = slot.parsed.get() {
            return Ok(sheet);
        }
        let sheet = self.parse(slot)?;
        Ok(slot.parsed.get_or_init(|| sheet))
    }

    /// Parse the part of `slot` into a sheet.
    fn parse(&self, slot: &Slot) -> Result<Sheet> {
        if !slot.backed {
            return Ok(Sheet::new(slot.name.clone())?
                .with_date_system(self.system)
                .with_state(slot.state));
        }
        let member = self.part_reader(&slot.part)?;
        let rows = SheetRows::capturing(std::io::BufReader::new(member), slot.name.clone());
        let strings = self.strings()?;
        let styles = self.styles()?;
        // The part counts its serials from the epoch the package states; a
        // system set since keeps each instant, and the sheet is written again
        // under it.
        let mut sheet = Sheet::from_rows(
            slot.name.clone(),
            slot.state,
            self.stated_system,
            rows,
            &strings,
            &styles,
            self.id,
        )?;
        sheet.set_date_system(self.system);
        Ok(sheet)
    }

    /// The family resolved at workbook intake, shared by every fresh part
    /// emitted by the object and record writers.
    pub(crate) fn namespace_family(&self) -> NamespaceFamily {
        self.stated.family
    }

    /// The date system the parts of the package the workbook reads from
    /// count their serials from: what its workbook part states, which a
    /// system set since leaves until the next save.
    pub(crate) const fn stated_date_system(&self) -> DateSystem {
        self.stated_system
    }

    /// Stream the part of the worksheet `name`, for the record path.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::sheet`] returns, and an empty part for a sheet
    /// added in memory and never written.
    pub(crate) fn sheet_reader(&self, name: &str) -> Result<Box<dyn Read + Send>> {
        let at = self.resolve(name).ok_or_else(|| self.absent(name))?;
        let slot = &self.slots[at];
        if slot.kind != SheetKind::Worksheet {
            return Err(self.not_a_worksheet(at));
        }
        if slot.backed {
            self.part_reader(&slot.part)
        } else {
            Ok(Box::new(std::io::Cursor::new(EMPTY_SHEET.as_bytes())))
        }
    }

    /// The part name of the worksheet `name`, for the refusals a read names.
    pub(crate) fn sheet_part(&self, name: &str) -> Result<SmolStr> {
        let at = self.resolve(name).ok_or_else(|| self.absent(name))?;
        Ok(self.slots[at].part.clone())
    }

    /// The shared strings, read on first use; none for a package without
    /// the part.
    pub(crate) fn strings(&self) -> Result<Arc<SharedStrings>> {
        if let Some(strings) = self.strings.get() {
            return Ok(Arc::clone(strings));
        }
        let strings = match &self.strings_part {
            Some(part) => SharedStrings::from_xml(&self.source.read(part)?)?,
            None => SharedStrings::default(),
        };
        let strings = Arc::new(strings);
        let _ = self.strings.set(Arc::clone(&strings));
        Ok(strings)
    }

    /// The styles, read on first use; the styles a package without the
    /// part is written with otherwise.
    pub(crate) fn styles(&self) -> Result<Arc<StyleSheet>> {
        if let Some(styles) = self.styles.get() {
            return Ok(Arc::clone(styles));
        }
        let styles = match &self.styles_part {
            Some(part) => StyleSheet::from_xml(self.source.read(part)?)?,
            None => StyleSheet::new(self.stated.family),
        };
        let styles = Arc::new(styles);
        let _ = self.styles.set(Arc::clone(&styles));
        Ok(styles)
    }

    /// The workbook's styles: every cell format a cell's [`StyleId`] may
    /// name, read on first use.
    ///
    /// # Errors
    ///
    /// Returns the styles part's refusal.
    pub fn style_sheet(&self) -> Result<&StyleSheet> {
        if self.styles.get().is_none() {
            self.styles()?;
        }
        Ok(self
            .styles
            .get()
            .expect("the styles were read by the call above"))
    }

    /// The workbook's theme, whose colours a style's theme colours index:
    /// the part its theme relationship names, read on first use, or the
    /// Office default ([`Theme::OFFICE`]) for a workbook without one.
    ///
    /// # Errors
    ///
    /// Returns the theme part's refusal: bytes that are not well-formed
    /// XML, or a colour that is not six hex digits.
    pub fn theme(&self) -> Result<&Theme> {
        if let Some(theme) = self.stated.theme.get() {
            return Ok(theme);
        }
        let theme = match &self.stated.theme_part {
            Some(part) => Theme::from_xml(&self.source.read(part)?)?,
            None => Theme::OFFICE,
        };
        Ok(self.stated.theme.get_or_init(|| theme))
    }

    /// Calls the package's handle has answered so far: what opening the
    /// workbook and reading its sheets asked of the bytes beneath it, in
    /// [`IOBase`] calls, which the cost pins assert. A workbook that saved
    /// counts from the package it saved.
    #[must_use]
    pub fn handle_reads(&self) -> u64 {
        self.source.handle_reads()
    }

    /// The refusal of a name another sheet already has.
    fn taken(&self, name: &str) -> Error {
        Error::Conflict {
            expected: "sheet name no other sheet has",
            actual: "sheet of that name, compared without case",
            path: SmolStr::new(name),
        }
    }

    fn absent(&self, name: &str) -> Error {
        Error::Absent {
            expected: "worksheet",
            path: format_smolstr!(
                "{name} (the workbook holds [{}])",
                self.sheet_names().join(", ")
            ),
        }
    }

    fn not_a_worksheet(&self, at: usize) -> Error {
        let slot = &self.slots[at];
        Error::InvalidRecord {
            path: format_smolstr!("$.{}", slot.name),
            reason: format_smolstr!(
                "expected a worksheet, got the {} `{}`, which holds no cells",
                slot.kind.as_str(),
                slot.name
            ),
        }
    }

    /// The package, as bytes: every sheet written from its cells that
    /// changed, every other member carried over as it is stored. The
    /// workbook is left as it is: it still counts what it wrote as unsaved.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::into_package`] returns.
    pub fn into_bytes(&self) -> Result<Vec<u8>> {
        Ok(self.into_package()?.into_bytes())
    }

    /// The package this workbook is, with the state of each sheet it holds.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a workbook with no visible
    /// worksheet, which Excel refuses to open, or a cell no part spells.
    pub fn into_package(&self) -> Result<Package> {
        self.write_package(None)
    }

    /// Save the workbook into `target`: the package built, adopted as what
    /// the workbook reads from - so no member is read again from wherever it
    /// was opened, `target` included - written with one
    /// [`IOBase::write_all_bytes`], and only then counted as saved.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::into_package`] returns, or the write's failure,
    /// which leaves the workbook holding what it held and counting it
    /// unsaved.
    pub fn write_into(&mut self, target: &mut (impl IOBase + ?Sized)) -> Result<()> {
        let package = self.into_package()?;
        self.adopt(&package)?;
        target.write_all_bytes(package.as_bytes())?;
        self.mark(&package.snapshot);
        Ok(())
    }

    /// Adopt `package` - one [`Self::into_package`] built and the caller
    /// wrote - as what the workbook reads from, and count what it holds as
    /// saved; a change made since it was built still counts as unsaved.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] for another workbook's package, or for
    /// one built before the package the workbook last adopted - two saves
    /// finishing out of order - whose sheets the workbook no longer holds
    /// the parts of.
    pub fn rebase(&mut self, package: Package) -> Result<()> {
        if package.snapshot.workbook != self.id {
            return Err(Error::Conflict {
                expected: "saved state of this workbook",
                actual: "package of another workbook",
                path: SmolStr::new_static("$"),
            });
        }
        if package.snapshot.sequence <= self.adopted {
            return Err(Error::Conflict {
                expected: "saved state newer than the one last adopted",
                actual: "package built before it",
                path: SmolStr::new_static("$"),
            });
        }
        self.adopt(&package)?;
        self.mark(&package.snapshot);
        Ok(())
    }

    /// Read from `package` from now on: each sheet it holds at the part it
    /// was written under, the strings and styles read again from it. No
    /// sheet counts as saved yet.
    ///
    /// # Errors
    ///
    /// Returns the styles' refusal of a style appended since the package
    /// was built ([`Self::adopted_styles`]), before anything is adopted.
    fn adopt(&mut self, package: &Package) -> Result<()> {
        let snapshot = &package.snapshot;
        let styles = snapshot
            .styles
            .as_ref()
            .map(|written| self.adopted_styles(written))
            .transpose()?;
        // Retained worksheet overlays were serialized under today's table,
        // not the incoming snapshot's. Translate their IDs before adopting
        // either the source or styles; unchanged XML bytes remain shared.
        let mut overlays = Vec::new();
        if let Some((_, moved)) = &styles {
            if !moved.is_empty() {
                for slot in self
                    .slots
                    .iter()
                    .filter(|slot| slot.kind == SheetKind::Worksheet)
                {
                    if let Some(bytes) = self
                        .stated
                        .overrides
                        .get(&slot.part)
                        .and_then(|held| held.bytes.as_ref())
                    {
                        if let Some(patched) = Sheet::rewrite_style_ids(bytes, &slot.part, |id| {
                            moved.get(&id).copied()
                        })? {
                            overlays.push((slot.part.clone(), Arc::from(patched)));
                        }
                    }
                }
            }
        }
        self.source = Source::Archive(Arc::new(ZipArchive::new(Holder::buffer(
            Buffer::from_bytes(package.bytes.clone()),
        ))));
        for slot in &mut self.slots {
            if let Some(written) = snapshot
                .slots
                .iter()
                .find(|written| written.key == slot.key)
            {
                debug_assert_eq!(slot.part, written.part);
                slot.backed = true;
                slot.state = written.state;
            } else {
                // A removed key's old image may occupy a newer key's path.
                // Only this slot's retained override can back an absent key.
                slot.backed = self
                    .stated
                    .overrides
                    .get(&slot.part)
                    .is_some_and(|held| held.bytes.is_some());
            }
        }
        for (part, bytes) in overlays {
            // This changes representation, not the logical overlay insertion:
            // mark still clears it if this snapshot actually saved that edit.
            self.stated
                .overrides
                .get_mut(&part)
                .expect("preflighted overlay")
                .bytes = Some(bytes);
        }
        self.stated_system = snapshot.system;
        self.adopted = snapshot.sequence;
        self.strings_part.clone_from(&snapshot.strings_part);
        self.styles_part.clone_from(&snapshot.styles_part);
        self.strings = OnceLock::new();
        self.stated.referring = OnceLock::new();
        if let Some((table, moved)) = styles {
            if !moved.is_empty() {
                for sheet in self
                    .slots
                    .iter_mut()
                    .filter_map(|slot| slot.parsed.get_mut())
                {
                    sheet.remap_styles(&moved);
                }
            }
            self.styles = OnceLock::new();
            let _ = self.styles.set(table);
        }
        Ok(())
    }

    /// Reconcile the entire appended suffix with this package's table.
    /// All snapshots share the original opaque XF prefix, but no later index
    /// is stable when writers and edits append formats concurrently.
    fn adopted_styles(
        &self,
        written: &Arc<StyleSheet>,
    ) -> Result<(Arc<StyleSheet>, HashMap<StyleId, StyleId>)> {
        match self.styles.get() {
            Some(current) => written.rebind(current.bindings()),
            None => Ok((Arc::clone(written), HashMap::new())),
        }
    }

    /// Count the state `snapshot` names as saved.
    fn mark(&mut self, snapshot: &Snapshot) {
        // Adoption precedes IO. Keep edits dirty until that IO succeeds;
        // an older package must not clear a newer presence or deletion.
        for (name, revision) in &snapshot.overrides {
            if self
                .stated
                .overrides
                .get(name)
                .is_some_and(|held| held.revision == *revision)
            {
                self.stated.overrides.remove(name);
            }
        }
        for written in &snapshot.slots {
            let Some(revision) = written.revision else {
                continue;
            };
            if let Some(slot) = self.slots.iter_mut().find(|slot| slot.key == written.key) {
                slot.saved = revision;
            }
        }
        self.documents_saved = snapshot.documents;
    }

    /// Write the package, the worksheet `replaced` names taking the place of
    /// the one held, written from the stream it carries.
    ///
    /// The archive is built in memory and answered whole: one write of the
    /// handle a caller hands the bytes to.
    pub(crate) fn write_package(&self, replaced: Option<Replaced<'_>>) -> Result<Package> {
        if !self
            .slots
            .iter()
            .any(|slot| slot.kind == SheetKind::Worksheet && slot.state() == SheetState::Visible)
        {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: SmolStr::new_static(
                    "expected at least one visible worksheet, which Excel requires of a workbook",
                ),
            });
        }
        let target = ZipArchive::new(Holder::buffer(Buffer::new())).with_restart_stride(0);
        let snapshot = self.write_preserving(&target, replaced)?;
        target.flush()?;
        let bytes = match target.into_handle()? {
            Holder::Buffer(buffer) => buffer.into_bytes(),
            other => other.read_all_bytes()?,
        };
        Ok(Package { bytes, snapshot })
    }

    /// The source again, as `target`: the sheets that changed written from
    /// their cells, the documents rewritten where what they state changed,
    /// every other member copied as it is stored.
    fn write_preserving(
        &self,
        target: &ZipArchive,
        replaced: Option<Replaced<'_>>,
    ) -> Result<Snapshot> {
        let members = self.members()?;
        let held = |name: &str| members.iter().any(|member| member == name);
        let relationships_part = package::relationships_part_of(&self.workbook_part);
        let replaced_at = replaced.as_ref().map(|replaced| replaced.at);
        // A date system set since the package was read moves every serial,
        // so every worksheet is written again under it.
        let system_changed = self.system != self.stated_system;
        let dirty: Vec<bool> = self
            .slots
            .iter()
            .enumerate()
            .map(|(at, slot)| {
                slot.is_dirty()
                    || replaced_at == Some(at)
                    || (system_changed && slot.kind == SheetKind::Worksheet)
            })
            .collect();
        let parts: Vec<SmolStr> = self.slots.iter().map(|slot| slot.part.clone()).collect();
        let list_changed = self.documents != self.documents_saved
            || self
                .slots
                .iter()
                .any(|slot| !slot.backed || slot.state() != slot.state);
        let any_dirty = dirty.iter().any(|dirty| *dirty);
        // Excel recalculates a workbook whose cells or sheets this save
        // wrote, and a calculation chain naming them would be stale.
        let rewrote = any_dirty || list_changed;
        // The relationships are read only by a save that may change them; a
        // save that wrote no sheet copies every document as it is stored.
        // They are read once: what they state here, and the bytes a
        // relationships document rewritten below starts from.
        let relationships_bytes = if rewrote && held(&relationships_part) {
            Some(self.part_bytes(&relationships_part)?)
        } else {
            None
        };
        let relationships = match &relationships_bytes {
            Some(bytes) => Relationships::from_xml(bytes, &self.workbook_part)?,
            None => Relationships::default(),
        };

        // The sheets, from their cells or their streams.
        let held_strings = if any_dirty {
            self.strings()?
        } else {
            Arc::new(SharedStrings::default())
        };
        let mut strings = SharedStringsWriter::new(&held_strings);
        // The styles a change appended to are written whether or not a
        // sheet is.
        let held_styles = if any_dirty || self.styles.get().is_some_and(|held| held.is_appended()) {
            Some(self.styles()?)
        } else {
            None
        };
        let mut splice = held_styles.as_deref().map(Splice::new);
        let mut replaced = replaced;
        let mut sidecars: Vec<(SmolStr, Vec<u8>)> = Vec::new();
        for (at, _) in dirty.iter().enumerate().filter(|(_, dirty)| **dirty) {
            let (part, splice) = (&parts[at], splice.as_mut().expect("read for a dirty sheet"));
            if replaced_at == Some(at) {
                let replaced = replaced.take().expect("the replaced sheet is written once");
                for format in &replaced.formats {
                    splice.temporal(*format)?;
                }
                let replacement = (replaced.stream)(splice)?;
                for (position, (name, _)) in replacement.parts.iter().enumerate() {
                    if !members.contains(name)
                        || parts.contains(name)
                        || sidecars.iter().any(|(held, _)| held == name)
                        || replacement.parts[..position]
                            .iter()
                            .any(|(held, _)| held == name)
                    {
                        return Err(Error::InvalidRecord {
                            path: name.clone(),
                            reason: SmolStr::new_static(
                                "expected one existing non-worksheet replacement part",
                            ),
                        });
                    }
                }
                sidecars.extend(replacement.parts);
                target.write_member_from(part, replacement.stream, Codec::Deflate)?;
            } else {
                // A sheet written again for the date system alone is parsed
                // here, under the system its part states.
                let sheet = self.parsed(at)?;
                let mut content = Vec::new();
                sheet.write_xml(&mut content, &mut strings, splice)?;
                target.write_member_with(part, &content, Codec::Deflate)?;
            }
        }
        let appended = strings.has_appended();
        let grown = splice.and_then(Splice::into_grown);
        let styles_created = self.styles_part.is_none() && held_styles.is_some();
        // The generated table, or a held table whose appended entries need writing.
        let written_styles: Option<StyleSheet> = match (grown, &held_styles) {
            (Some(grown), _) => Some(grown),
            (None, Some(held)) if styles_created || held.is_appended() => {
                Some(StyleSheet::clone(held))
            }
            _ => None,
        };
        let styles_bytes = match &written_styles {
            Some(table) => Some(table.to_part()?),
            None => None,
        };
        let strings_part = self
            .strings_part
            .clone()
            .or_else(|| appended.then(|| SmolStr::new_static(package::SHARED_STRINGS_PART)));
        let styles_part = self
            .styles_part
            .clone()
            .or_else(|| styles_created.then(|| SmolStr::new_static(package::STYLES_PART)));
        let strings_created = self.strings_part.is_none() && appended;

        // What the sheet list no longer names goes, with its own
        // relationships; the calculation chain goes on a save that wrote a
        // sheet.
        let orphaned: Vec<SmolStr> = relationships
            .entries()
            .iter()
            .filter(|relationship| relationship.kind.is_sheet())
            .filter_map(|relationship| relationship.target.clone())
            .filter(|part| !parts.contains(part))
            .collect();
        let calc_chain = relationships
            .first_of(RelationshipKind::CalcChain)
            .and_then(|relationship| relationship.target.clone())
            .or_else(|| {
                held(package::CALC_CHAIN_PART)
                    .then(|| SmolStr::new_static(package::CALC_CHAIN_PART))
            })
            .filter(|_| rewrote);
        // What only the removed sheets reached goes with them: drawings and
        // their charts, tables, comments, pivot tables and the caches their
        // last table went with.
        let reached = if orphaned.is_empty() {
            Vec::new()
        } else {
            self.orphans(&members, &orphaned, (&relationships_part, &relationships))?
        };
        let dropped_caches: Vec<SmolStr> = relationships
            .entries()
            .iter()
            .filter(|relationship| {
                relationship.kind == RelationshipKind::PivotCacheDefinition
                    && relationship
                        .target
                        .as_ref()
                        .is_some_and(|target| reached.contains(target))
            })
            .map(|relationship| relationship.id.clone())
            .collect();
        let mut dropped: Vec<SmolStr> = orphaned.clone();
        dropped.extend(reached);
        dropped.extend(calc_chain.iter().cloned());
        // Each tab keeps the relationship naming its part; a part no
        // relationship names takes an id past every one stated.
        let mut next_id = relationships.next_id();
        let mut fresh = Vec::new();
        let mut ids: Vec<SmolStr> = Vec::with_capacity(parts.len());
        for (at, part) in parts.iter().enumerate() {
            match relationships
                .to_part(part)
                .filter(|relationship| relationship.kind.is_sheet())
            {
                Some(relationship) => ids.push(relationship.id.clone()),
                None => {
                    fresh.push(at);
                    ids.push(take_id(&mut next_id, &relationships_part)?);
                }
            }
        }

        let documents = Documents {
            members: &members,
            relationships: &relationships,
            relationships_bytes: relationships_bytes.as_deref(),
            parts: &parts,
            ids: &ids,
            fresh: &fresh,
            next_id,
            list_changed,
            rewrote,
            dropped: &dropped,
            dropped_caches: &dropped_caches,
            strings_part: strings_part.as_deref().filter(|_| strings_created),
            styles_part: styles_part.as_deref().filter(|_| styles_created),
        };
        let workbook = self.workbook_document(&documents)?;
        let workbook_relationships = self.relationships_document(&documents)?;
        let content_types = self.content_types_document(&documents)?;

        // Every other member, as it is stored.
        let written: Vec<&str> = parts
            .iter()
            .zip(&dirty)
            .filter(|(_, dirty)| **dirty)
            .map(|(part, _)| part.as_str())
            .collect();
        let deferred = [
            Some(self.workbook_part.as_str()),
            Some(relationships_part.as_str()),
            Some(package::CONTENT_TYPES_PART),
            strings_part.as_deref().filter(|_| appended),
            styles_part.as_deref().filter(|_| styles_bytes.is_some()),
        ];
        for name in &members {
            let name = name.as_str();
            if written.contains(&name)
                || sidecars.iter().any(|(part, _)| part.as_str() == name)
                || deferred.contains(&Some(name))
                || dropped
                    .iter()
                    .any(|part| part == name || package::relationships_part_of(part) == name)
            {
                continue;
            }
            self.copy_part_into(target, name)?;
        }
        for (name, bytes) in &sidecars {
            target.write_member_with(name, bytes, Codec::Deflate)?;
        }
        for (name, document) in [
            (package::CONTENT_TYPES_PART, content_types),
            (self.workbook_part.as_str(), workbook),
            (relationships_part.as_str(), workbook_relationships),
        ] {
            match document {
                Some(bytes) => {
                    target.write_member_with(name, &bytes, Codec::Deflate)?;
                }
                None if held(name) => self.copy_part_into(target, name)?,
                None => {}
            }
        }
        if let Some(part) = strings_part.as_deref().filter(|_| appended) {
            // The table is extended as the package stores it, every item it
            // held kept as it stands.
            let original = match &self.strings_part {
                Some(held) => Some(self.source.read(held)?),
                None => None,
            };
            let content = strings.into_part(original.as_deref(), self.stated.family)?;
            target.write_member_with(part, &content, Codec::Deflate)?;
        }
        if let (Some(part), Some(bytes)) = (styles_part.as_deref(), &styles_bytes) {
            target.write_member_with(part, bytes, Codec::Deflate)?;
        }
        Ok(Snapshot {
            workbook: self.id,
            sequence: PACKAGES.fetch_add(1, Ordering::Relaxed),
            slots: self
                .slots
                .iter()
                .zip(&parts)
                .enumerate()
                .map(|(at, (slot, part))| Written {
                    key: slot.key,
                    part: part.clone(),
                    revision: slot
                        .parsed
                        .get()
                        .map(Sheet::revision)
                        .filter(|_| replaced_at != Some(at)),
                    state: slot.state(),
                })
                .collect(),
            strings_part,
            styles_part,
            styles: written_styles
                .zip(styles_bytes)
                .map(|(table, bytes)| Arc::new(table.written(bytes)))
                .or_else(|| self.styles.get().cloned()),
            documents: self.documents,
            system: self.system,
            overrides: self
                .stated
                .overrides
                .iter()
                .map(|(name, held)| (name.clone(), held.revision))
                .collect(),
        })
    }

    /// The workbook part again, `None` when what it states did not change:
    /// the sheet list regenerated when it changed, the defined names when a
    /// rename or a removal touched one, the views' tabs moved past a removed
    /// one, the pivot caches whose last table went left out, the date
    /// system stated when it changed, and Excel asked to recalculate on
    /// load when a sheet was written.
    fn workbook_document(&self, documents: &Documents<'_>) -> Result<Option<Vec<u8>>> {
        use std::cell::Cell;

        let system_changed = self.system != self.stated_system;
        if !documents.rewrote && !system_changed {
            return Ok(None);
        }
        let bytes = self.part_bytes(&self.workbook_part)?;
        let mut sheets = String::new();
        if documents.list_changed {
            // A `<sheet>` names its part in the relationships namespace: by
            // the prefix the workbook part binds to it where the sheets are,
            // else by one it declares itself.
            let prefix = package::prefix_of(
                &bytes,
                b"sheets",
                &[self.stated.family.relationships_namespace()],
            )?;
            let (prefix, declaration) = match prefix {
                Some(prefix) => (prefix, String::new()),
                None => (
                    String::from("r"),
                    format!(
                        " xmlns:r=\"{}\"",
                        self.stated.family.relationships_namespace()
                    ),
                ),
            };
            for (slot, id) in self.slots.iter().zip(documents.ids) {
                sheets.push_str(&format!(
                    "<sheet name=\"{}\" sheetId=\"{}\"{declaration} {prefix}:id=\"{id}\"{}/>",
                    package::escape_attribute(&slot.name),
                    slot.sheet_id,
                    match slot.state() {
                        SheetState::Visible => String::new(),
                        other => format!(" state=\"{}\"", other.as_str()),
                    }
                ));
            }
        }
        // The names, each as it was written unless what it states changed,
        // its scope counted in today's tabs.
        let names = if self.stated.names_touched {
            let mut names = Vec::new();
            for defined in &self.stated.names {
                let tab = defined
                    .scope()
                    .and_then(|key| self.slots.iter().position(|slot| slot.key == key));
                names.extend(defined.element(tab)?);
            }
            Some(String::from_utf8(names).map_err(|error| {
                package::codec_error(0, format_smolstr!("a defined name is not UTF-8: {error}"))
            })?)
        } else {
            None
        };
        let no_names = names.as_ref().is_some_and(String::is_empty);
        // The list goes when the last cache it holds went - counted in the
        // part as stored, which a save that dropped a cache rewrote.
        let no_caches = !documents.dropped_caches.is_empty()
            && read_workbook(&bytes, &self.workbook_part)?
                .pivot_caches
                .iter()
                .all(|cache| documents.dropped_caches.contains(&cache.rid));
        let has_properties = package::has_element(&bytes, b"workbookPr")?;
        let has_calculation = package::has_element(&bytes, b"calcPr")?;
        let mut insert = Vec::new();
        if system_changed && !has_properties && self.system == DateSystem::Year1904 {
            insert.push(Insertion {
                fragment: "<workbookPr date1904=\"1\"/>".to_owned(),
                parent: b"workbook",
                before_first_of: AFTER_WORKBOOK_PROPERTIES,
            });
        }
        if documents.rewrote && !has_calculation {
            insert.push(Insertion {
                fragment: "<calcPr fullCalcOnLoad=\"1\"/>".to_owned(),
                parent: b"workbook",
                before_first_of: AFTER_CALCULATION_PROPERTIES,
            });
        }
        let date1904 = (self.system == DateSystem::Year1904).then(|| "1".to_owned());
        // Each view in turn, as the attribute hook meets them.
        let view = Cell::new(0_usize);
        package::rewrite(
            &bytes,
            &package::Rewrite {
                skip: &|start| {
                    let qualified = start.name();
                    match package::local_name(qualified.as_ref()) {
                        b"sheet" => documents.list_changed,
                        b"definedName" => names.is_some(),
                        b"definedNames" => no_names,
                        b"pivotCaches" => no_caches,
                        b"pivotCache" => {
                            start
                                .attributes()
                                .with_checks(false)
                                .flatten()
                                .any(|attribute| {
                                    package::local_name(attribute.key.as_ref()) == b"id"
                                        && documents
                                            .dropped_caches
                                            .iter()
                                            .any(|id| id.as_bytes() == attribute.value.as_ref())
                                })
                        }
                        _ => false,
                    }
                },
                before_end: &|name| match name {
                    b"sheets" if documents.list_changed => Some(sheets.clone()),
                    b"definedNames" => names.clone(),
                    _ => None,
                },
                attributes: &|name| match name {
                    b"workbookPr" if system_changed => vec![("date1904", date1904.clone())],
                    b"calcPr" if documents.rewrote => {
                        vec![("fullCalcOnLoad", Some("1".to_owned()))]
                    }
                    b"workbookView" if self.stated.views_touched => {
                        let at = view.get();
                        view.set(at + 1);
                        let Some(stated) = self.stated.views.get(at) else {
                            return Vec::new();
                        };
                        let mut edits = Vec::new();
                        if let Some(tab) = stated.active_tab {
                            edits.push(("activeTab", Some(tab.to_string())));
                        }
                        if let Some(tab) = stated.first_sheet {
                            edits.push(("firstSheet", Some(tab.to_string())));
                        }
                        edits
                    }
                    _ => Vec::new(),
                },
                insert: &insert,
                ..package::Rewrite::default()
            },
        )
        .map(Some)
    }

    /// The parts a save drops with the removed sheet parts `removed`: every
    /// part that only they reach, through every relationship the package
    /// states - a drawing, the charts it holds and their styles, a table,
    /// comments and their drawing, printer settings, a pivot table - and a
    /// pivot cache the workbook lists once its last table is gone.
    ///
    /// Each relationships part is read once, the workbook's - `workbook`,
    /// its member and what the save read of it - not again.
    fn orphans(
        &self,
        members: &[SmolStr],
        removed: &[SmolStr],
        workbook: (&str, &Relationships),
    ) -> Result<Vec<SmolStr>> {
        Ok(self
            .part_graph(members, workbook, None)?
            .dropped(removed)
            .into_iter()
            .filter(|part| !removed.contains(part))
            .collect())
    }

    /// Read one graph for both sides of an undo's prospective orphan delta.
    fn part_graph(
        &self,
        members: &[SmolStr],
        workbook: (&str, &Relationships),
        mut cached: Option<&mut PartBytes>,
    ) -> Result<PartGraph> {
        let mut sources: HashMap<SmolStr, Vec<SmolStr>> = HashMap::new();
        for member in members {
            let Some(source) = package::source_of_relationships(member) else {
                continue;
            };
            let read;
            let relationships = if member == workbook.0 {
                workbook.1
            } else {
                let bytes = match cached.as_ref().and_then(|cache| cache.get(member)) {
                    Some(bytes) => Arc::clone(bytes),
                    None => self.part_bytes(member)?,
                };
                if let Some(cache) = cached.as_mut() {
                    cache
                        .entry(member.clone())
                        .or_insert_with(|| Arc::clone(&bytes));
                }
                read = Relationships::from_xml(&bytes, &source)?;
                &read
            };
            for relationship in relationships.entries() {
                if relationship.kind == RelationshipKind::PivotCacheDefinition
                    && source == self.workbook_part
                {
                    continue;
                }
                if let Some(target) = &relationship.target {
                    sources
                        .entry(target.clone())
                        .or_default()
                        .push(source.clone());
                }
            }
        }
        Ok(PartGraph { sources })
    }

    /// The workbook's relationships again, `None` when none changed: the
    /// relationships of the tabs gone and of the calculation chain dropped,
    /// one added for each sheet part and shared part this save created.
    fn relationships_document(&self, documents: &Documents<'_>) -> Result<Option<Vec<u8>>> {
        if !documents.rewrote {
            return Ok(None);
        }
        let base = package::folder_of(&self.workbook_part);
        let relationships_part = package::relationships_part_of(&self.workbook_part);
        let mut next = documents.next_id;
        let mut added = String::new();
        for at in documents.fresh {
            added.push_str(&package::relationship_element(
                &documents.ids[*at],
                self.stated.family,
                self.slots[*at].kind.as_str(),
                &package::relative_to(base, &documents.parts[*at]),
            ));
        }
        for (part, kind) in [
            (documents.strings_part, "sharedStrings"),
            (documents.styles_part, "styles"),
        ] {
            if let Some(shared) = part {
                added.push_str(&package::relationship_element(
                    &take_id(&mut next, &relationships_part)?,
                    self.stated.family,
                    kind,
                    &package::relative_to(base, shared),
                ));
            }
        }
        let skipped: Vec<&str> = documents
            .relationships
            .entries()
            .iter()
            .filter(|relationship| {
                relationship
                    .target
                    .as_ref()
                    .is_some_and(|target| documents.dropped.contains(target))
            })
            .map(|relationship| relationship.id.as_str())
            .collect();
        if added.is_empty() && skipped.is_empty() {
            return Ok(None);
        }
        // A package holding no relationships for its workbook gets them.
        let Some(bytes) = documents.relationships_bytes else {
            return Ok(Some(
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
                     <Relationships xmlns=\"{}\">{added}</Relationships>",
                    package::PACKAGE_RELATIONSHIPS_NAMESPACE
                )
                .into_bytes(),
            ));
        };
        package::rewrite(
            bytes,
            &package::Rewrite {
                skip: &|start| {
                    package::local_name(start.name().as_ref()) == b"Relationship"
                        && start.attributes().flatten().any(|attribute| {
                            package::local_name(attribute.key.as_ref()) == b"Id"
                                && skipped
                                    .iter()
                                    .any(|id| id.as_bytes() == attribute.value.as_ref())
                        })
                },
                before_end: &|name| (name == b"Relationships").then(|| added.clone()),
                ..package::Rewrite::default()
            },
        )
        .map(Some)
    }

    /// `[Content_Types].xml` again, `None` when no part came or went: an
    /// `Override` for each worksheet part and shared part this save
    /// created, none for a part it dropped.
    fn content_types_document(&self, documents: &Documents<'_>) -> Result<Option<Vec<u8>>> {
        let created = self.slots.iter().any(|slot| !slot.backed)
            || documents.strings_part.is_some()
            || documents.styles_part.is_some();
        // A retained inverse can restore metadata from before later saves
        // backed a fresh sheet or shared part. Their live identities still
        // require declarations even though no payload is fresh this save.
        let historical = self
            .stated
            .overrides
            .contains_key(package::CONTENT_TYPES_PART);
        if !created && documents.dropped.is_empty() && !historical {
            return Ok(None);
        }
        let existing = if documents
            .members
            .iter()
            .any(|name| name == package::CONTENT_TYPES_PART)
        {
            Some(self.part_bytes(package::CONTENT_TYPES_PART)?)
        } else {
            None
        };
        let mut declared = BTreeSet::new();
        if let Some(existing) = &existing {
            for entry in Registration::root(existing)?
                .children_named(&[package::CONTENT_TYPES_NAMESPACE], "Override")?
            {
                let part = entry
                    .attribute(b"PartName")?
                    .ok_or_else(|| Error::InvalidRecord {
                        path: package::CONTENT_TYPES_PART.into(),
                        reason: "expected an Override PartName".into(),
                    })?;
                declared.insert(part.trim_start_matches('/').to_owned());
            }
        }
        let stated = |part: &str| declared.contains(part);
        let mut overrides = String::new();
        for (slot, part) in self.slots.iter().zip(documents.parts) {
            if (!slot.backed || historical) && !stated(part) {
                overrides.push_str(&package::override_element(part, slot.kind.content_type()));
            }
        }
        for (part, content_type) in [
            (
                documents
                    .strings_part
                    .or(self.strings_part.as_deref().filter(|_| historical)),
                package::SHARED_STRINGS_CONTENT_TYPE,
            ),
            (
                documents
                    .styles_part
                    .or(self.styles_part.as_deref().filter(|_| historical)),
                package::STYLES_CONTENT_TYPE,
            ),
        ] {
            if let Some(part) = part.filter(|part| !stated(part)) {
                overrides.push_str(&package::override_element(part, content_type));
            }
        }
        // A dropped part's relationships go with it, and so does the
        // override a writer stated for them.
        let dropped: Vec<String> = documents
            .dropped
            .iter()
            .flat_map(|part| {
                [
                    format!("/{part}"),
                    format!("/{}", package::relationships_part_of(part)),
                ]
            })
            .filter(|part| declared.contains(part.trim_start_matches('/')))
            .collect();
        let Some(existing) = existing else {
            // A package with no content types part gets one naming the
            // parts it holds, wherever they are.
            let mut content = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
                 <Types xmlns=\"{}\"><Default Extension=\"rels\" ContentType=\"{}\"/>\
                 <Default Extension=\"xml\" ContentType=\"application/xml\"/>",
                package::CONTENT_TYPES_NAMESPACE,
                package::RELATIONSHIPS_CONTENT_TYPE
            );
            content.push_str(&package::override_element(
                &self.workbook_part,
                package::WORKBOOK_CONTENT_TYPE,
            ));
            content.push_str(&overrides);
            content.push_str("</Types>");
            return Ok(Some(content.into_bytes()));
        };
        if overrides.is_empty() && dropped.is_empty() {
            return Ok(None);
        }
        package::rewrite(
            &existing,
            &package::Rewrite {
                skip: &|start| {
                    package::local_name(start.name().as_ref()) == b"Override"
                        && start.attributes().flatten().any(|attribute| {
                            package::local_name(attribute.key.as_ref()) == b"PartName"
                                && dropped
                                    .iter()
                                    .any(|part| part.as_bytes() == attribute.value.as_ref())
                        })
                },
                before_end: &|name| (name == b"Types").then(|| overrides.clone()),
                ..package::Rewrite::default()
            },
        )
        .map(Some)
    }
}

/// The rows and fields of the tab-separated text `tsv` pasted from
/// `anchor` of the sheet `sheet`, and the range they span.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] for text landing off the grid, or
/// holding more than [`MAX_EDITED_CELLS`] fields.
pub(crate) fn pasted(
    sheet: &str,
    anchor: CellRef,
    tsv: &str,
) -> Result<(Vec<Vec<String>>, CellRange)> {
    let rows = tab_separated(tsv);
    let height = rows.len().max(1) as u64;
    let width = rows.iter().map(Vec::len).max().unwrap_or(1).max(1) as u64;
    let fields: u64 = rows.iter().map(|row| row.len() as u64).sum();
    if fields > MAX_EDITED_CELLS {
        return Err(Error::InvalidRecord {
            path: format_smolstr!("{sheet}!{anchor}"),
            reason: format_smolstr!(
                "expected at most {MAX_EDITED_CELLS} fields pasted at once, got {fields}"
            ),
        });
    }
    if u64::from(anchor.row()) + height > u64::from(MAX_ROWS)
        || u64::from(anchor.column()) + width > u64::from(MAX_COLUMNS)
    {
        return Err(Error::InvalidRecord {
            path: format_smolstr!("{sheet}!{anchor}"),
            reason: format_smolstr!(
                "expected {height} rows and {width} columns from {anchor} to fit the grid"
            ),
        });
    }
    let span = CellRange::new(
        anchor,
        CellRef::new(
            anchor.row() + height as u32 - 1,
            anchor.column() + width as u32 - 1,
        ),
    );
    Ok((rows, span))
}

/// The rows and fields of tab-separated text as Excel puts it on the
/// clipboard: a field opening with `"` runs to the `"` closing it, `""`
/// inside it one `"`, tabs and line breaks inside it kept; a line break
/// ending the text ends the last row.
fn tab_separated(text: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut fresh = true;
    while let Some(character) = chars.next() {
        match character {
            '"' if fresh => {
                // A quoted field, to its closing quote.
                loop {
                    match chars.next() {
                        Some('"') if chars.peek() == Some(&'"') => {
                            chars.next();
                            field.push('"');
                        }
                        Some('"') | None => break,
                        Some(other) => field.push(other),
                    }
                }
                fresh = false;
            }
            '\t' => {
                row.push(std::mem::take(&mut field));
                fresh = true;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                fresh = true;
            }
            other => {
                field.push(other);
                fresh = false;
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// What a cell sorts as: a number, text folded for comparison, a boolean,
/// an error, or nothing.
#[derive(Clone, Debug, PartialEq)]
enum SortOperand {
    Number(f64),
    Text(String),
    Boolean(bool),
    Error,
    Blank,
}

/// What the cell `cell` sorts as.
fn sort_key(cell: Option<&Cell>) -> SortOperand {
    let Some(cell) = cell else {
        return SortOperand::Blank;
    };
    if cell.error().is_some() {
        return SortOperand::Error;
    }
    let value = cell.value();
    if value.is_null() {
        return SortOperand::Blank;
    }
    if let Some(flag) = value.as_bool() {
        return SortOperand::Boolean(flag);
    }
    if let Some(number) = super::cell::number_of(value) {
        return SortOperand::Number(number);
    }
    if let Ok(Some((serial, _))) = DateSystem::Year1900.serial_of(value) {
        return SortOperand::Number(serial);
    }
    SortOperand::Text(cell.text().to_lowercase())
}

/// Order two sort values as Excel sorts: numbers, text, `FALSE` and
/// `TRUE`, errors - reversed when `descending` - and blanks last either
/// way.
fn compare_keys(first: &SortOperand, second: &SortOperand, descending: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let rank = |value: &SortOperand| match value {
        SortOperand::Number(_) => 0,
        SortOperand::Text(_) => 1,
        SortOperand::Boolean(_) => 2,
        SortOperand::Error => 3,
        SortOperand::Blank => 4,
    };
    match (first, second) {
        (SortOperand::Blank, SortOperand::Blank) => return Ordering::Equal,
        (SortOperand::Blank, _) => return Ordering::Greater,
        (_, SortOperand::Blank) => return Ordering::Less,
        _ => {}
    }
    let ordering = match (first, second) {
        (SortOperand::Number(a), SortOperand::Number(b)) => a.total_cmp(b),
        (SortOperand::Text(a), SortOperand::Text(b)) => a.cmp(b),
        (SortOperand::Boolean(a), SortOperand::Boolean(b)) => a.cmp(b),
        _ => rank(first).cmp(&rank(second)),
    };
    if descending {
        ordering.reverse()
    } else {
        ordering
    }
}

/// A part a worksheet relates that a band of its rows or columns moves
/// references in: its kind, its member name and its bytes, read once.
type Related = (RelationshipKind, SmolStr, Arc<[u8]>);

/// What a shift rewrites beside the cells, worked out before anything
/// changes.
#[derive(Default)]
struct Plan {
    /// Special built-in filter-name contractions, prepared before mutation.
    filter_names: Vec<(usize, super::formula::Formula)>,
    /// Each worksheet whose carried children change, by slot, and the
    /// children it carries after.
    frames: Vec<(usize, Vec<super::carried::Carried>)>,
    /// Each part beside the sheets that changes.
    overrides: Vec<Rewritten>,
    /// Each unchanged part the inverse must retain: an opposite shift's
    /// lossy payload or the relationship document proving its ownership.
    kept: Vec<(SmolStr, Arc<[u8]>)>,
}

impl Plan {
    fn contains(&self, member: &str) -> bool {
        self.overrides.iter().any(|entry| entry.member == member)
            || self.kept.iter().any(|(name, _)| name == member)
    }

    /// Retain one unchanged witness without rewriting it in the forward edit.
    fn keep_part(&mut self, member: SmolStr, bytes: Arc<[u8]>) {
        if !self.contains(&member) {
            self.kept.push((member, bytes));
        }
    }

    /// Reserve absence in the existing planned part image, then let
    /// note_payload fill that image. Later allocations see the same owner;
    /// no separate name registry can diverge from the published parts.
    fn note_member(&mut self, workbook: &Workbook, source: &str) -> Result<SmolStr> {
        let (stem, extension) = source.rsplit_once('.').ok_or_else(|| {
            Error::unsupported("splitting a note part without a filename extension", source)
        })?;
        let stem = stem.trim_end_matches(|ch: char| ch.is_ascii_digit());
        let suffix = format!(".{extension}");
        let name = workbook.next_part(
            stem,
            &suffix,
            self.overrides.iter().map(|entry| &entry.member),
        )?;
        self.set_part(name.clone(), None, None);
        Ok(name)
    }

    fn note_payload(
        &mut self,
        workbook: &Workbook,
        name: &str,
        bytes: Arc<[u8]>,
        cached: &PartBytes,
    ) -> Result<()> {
        let before = self.part(workbook, name, cached)?;
        if before.as_deref() != Some(bytes.as_ref()) {
            self.set_part(name.into(), before, Some(bytes));
        }
        Ok(())
    }

    /// Drop a proved orphan closure and only its explicit type registrations.
    /// Each absent byte is retained by the ordinary part-overlay inverse.
    fn prune_note_parts(
        &mut self,
        workbook: &Workbook,
        mut removed: BTreeSet<SmolStr>,
        cached: &PartBytes,
    ) -> Result<()> {
        if removed.is_empty() {
            return Ok(());
        }
        for member in removed.clone() {
            let rels = package::relationships_part_of(&member);
            if self.part(workbook, &rels, cached)?.is_some() {
                removed.insert(rels);
            }
        }
        for member in &removed {
            let before = self.part(workbook, member, cached)?;
            if before.is_some() {
                self.set_part(member.clone(), before, None);
            }
        }
        let part = package::CONTENT_TYPES_PART;
        let bytes = self
            .part(workbook, part, cached)?
            .ok_or_else(|| Error::absent("workbook part", part))?;
        let root = Registration::root(&bytes)?;
        let mut changes = BTreeMap::new();
        for entry in root.children_named(&[package::CONTENT_TYPES_NAMESPACE], "Override")? {
            if entry
                .attribute(b"PartName")?
                .is_some_and(|name| removed.contains(name.trim_start_matches('/')))
            {
                changes.insert(entry.key, None);
            }
        }
        if !changes.is_empty() {
            let root = root.changed_children(&changes)?;
            let after = Registration::replace_root(&bytes, &root.xml)?;
            self.set_part(part.into(), Some(bytes), Some(after.into()));
        }
        Ok(())
    }
    fn ensure_content_type(
        &mut self,
        workbook: &Workbook,
        member: &str,
        expected: &str,
        cached: &PartBytes,
    ) -> Result<()> {
        let part = package::CONTENT_TYPES_PART;
        let types = self
            .part(workbook, part, cached)?
            .ok_or_else(|| Error::absent("workbook part", part))?;
        let defaults = Registration::read(&types, b"Default", b"Extension")?;
        let explicit = Registration::read(&types, b"Override", b"PartName")?;
        let extension = member
            .rsplit_once('.')
            .map_or("", |(_, extension)| extension);
        if let Some(stated) = explicit
            .iter()
            .find(|entry| entry.key == format!("/{member}"))
        {
            if stated.attribute(b"ContentType")?.as_deref() != Some(expected) {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{part} ({member})"),
                    reason: format_smolstr!("expected content type {expected}"),
                });
            }
            return Ok(());
        }
        if let Some(default) = defaults.iter().find(|entry| entry.key == extension) {
            if default.attribute(b"ContentType")?.as_deref() == Some(expected) {
                return Ok(());
            }
        }
        let entry = Registration::root(
            format!(
                "<Override xmlns=\"{}\" PartName=\"/{}\" ContentType=\"{}\"/>",
                package::CONTENT_TYPES_NAMESPACE,
                package::escape_attribute(member),
                package::escape_attribute(expected),
            )
            .as_bytes(),
        )?;
        let updated = Registration::root(&types)?.appended(&[entry])?;
        let after = Registration::replace_root(&types, &updated.xml)?;
        self.set_part(part.into(), Some(types), Some(after.into()));
        Ok(())
    }
    /// Read the latest pending image; the inverse retains the first image.
    fn part(
        &self,
        workbook: &Workbook,
        name: &str,
        cached: &PartBytes,
    ) -> Result<Option<Arc<[u8]>>> {
        match self.overrides.iter().find(|entry| entry.member == name) {
            Some(entry) => Ok(entry.after.clone()),
            None => match cached.get(name) {
                Some(bytes) => Ok(Some(Arc::clone(bytes))),
                None => workbook.part_bytes_if_present(name),
            },
        }
    }

    fn set_part(&mut self, member: SmolStr, before: Option<Arc<[u8]>>, after: Option<Arc<[u8]>>) {
        // A later table/note ownership rewrite supersedes an earlier unchanged
        // witness; one member must have one expected post-edit image.
        self.kept.retain(|(held, _)| *held != member);
        match self
            .overrides
            .iter_mut()
            .find(|entry| entry.member == member)
        {
            Some(entry) => entry.after = after,
            None => self.overrides.push(Rewritten {
                member,
                before,
                after,
            }),
        }
    }

    fn frame(&self, workbook: &Workbook, index: usize) -> super::carried::WorksheetFrame {
        let mut frame = workbook.slots[index]
            .parsed
            .get()
            .and_then(Sheet::frame)
            .cloned()
            .unwrap_or_else(|| super::carried::WorksheetFrame::new(workbook.stated.family));
        if let Some((_, items)) = self.frames.iter().find(|(at, _)| *at == index) {
            frame.items.clone_from(items);
        }
        frame
    }

    fn set_frame(&mut self, index: usize, items: Vec<super::carried::Carried>) {
        match self.frames.iter_mut().find(|(at, _)| *at == index) {
            Some((_, held)) => *held = items,
            None => self.frames.push((index, items)),
        }
    }
}

/// A part beside the sheets a shift rewrites: its member, and its bytes
/// before and after.
struct Rewritten {
    member: SmolStr,
    before: Option<Arc<[u8]>>,
    after: Option<Arc<[u8]>>,
}

/// The bytes of parts beside the sheets, by member.
type PartBytes = HashMap<SmolStr, Arc<[u8]>>;

/// A part beside the sheets stating references by sheet name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Referrer {
    /// A chart: its series formulas.
    Chart,
    /// A pivot cache: the sheet its source is on.
    Cache,
    /// A drawing: its shapes' cell links.
    Drawing,
    /// A table: its column formulas, relative to its owning worksheet.
    Table(SheetKey),
}

/// One part of [`Workbook::referring`]'s index: the member, what it is,
/// and the text of the references it states - `None` for a member read
/// from an edit's rewrite, or one that does not read, which is looked at
/// whole.
#[derive(Debug)]
pub(crate) struct Referring {
    member: SmolStr,
    kind: Referrer,
    references: Option<String>,
    /// Only tables retain ownership here. Tables on the same worksheet share
    /// its one already-read Arc until adoption; SheetKey remains the identity.
    owner_document: Option<Arc<[u8]>>,
}

impl Referring {
    fn ownership(&self, workbook: &Workbook) -> Result<Option<(SmolStr, Arc<[u8]>)>> {
        let Referrer::Table(key) = self.kind else {
            return Ok(None);
        };
        let Some(slot) = workbook.slots.iter().find(|slot| slot.key == key) else {
            return Ok(None);
        };
        let member = package::relationships_part_of(&slot.part);
        // Notes can change a table owner's relationship document without
        // changing its table membership. The effective overlay wins until
        // adoption rebuilds the index; no extra source read is required.
        let bytes = match workbook.stated.overrides.get(&member) {
            Some(held) => held
                .bytes
                .clone()
                .ok_or_else(|| Error::absent("worksheet relationships", &member))?,
            None => Arc::clone(
                self.owner_document
                    .as_ref()
                    .expect("the table index retains each resolved owner document"),
            ),
        };
        Ok(Some((member, bytes)))
    }
}

/// The refusal of `shift` where an element this crate does not model might
/// hold a reference it moves: on a sheet whose cells it moves (`own`), or
/// naming a sheet it is about.
fn named_by(shift: &Shift<'_>, own: bool) -> &'static str {
    match (*shift, own) {
        (
            Shift::Band {
                axis: Axis::Rows,
                band: Band::Insert { .. },
                ..
            },
            _,
        ) => "inserting rows in a sheet an element this crate does not model names",
        (
            Shift::Band {
                axis: Axis::Rows,
                band: Band::Remove { .. },
                ..
            },
            _,
        ) => "removing rows in a sheet an element this crate does not model names",
        (
            Shift::Band {
                axis: Axis::Columns,
                band: Band::Insert { .. },
                ..
            },
            _,
        ) => "inserting columns in a sheet an element this crate does not model names",
        (
            Shift::Band {
                axis: Axis::Columns,
                band: Band::Remove { .. },
                ..
            },
            _,
        ) => "removing columns in a sheet an element this crate does not model names",
        (Shift::Move { .. }, true) => {
            "moving cells through a sheet carrying an element this crate does not model"
        }
        (Shift::Move { .. }, false) => "moving cells an element this crate does not model names",
        (Shift::RenameSheet { .. }, _) => {
            "renaming a sheet an element this crate does not model names"
        }
        (Shift::RemoveSheet { .. }, _) => {
            "removing a sheet an element this crate does not model names"
        }
    }
}

/// The refusal of `band` along `axis` through a sheet carrying the element
/// `element`, and whether it names the element itself.
fn blocked(axis: Axis, band: Band, element: &str) -> (&'static str, bool) {
    macro_rules! through {
        ($what:literal) => {
            match (axis, band) {
                (Axis::Rows, Band::Insert { .. }) => {
                    concat!("inserting rows through a sheet carrying ", $what)
                }
                (Axis::Rows, Band::Remove { .. }) => {
                    concat!("removing rows through a sheet carrying ", $what)
                }
                (Axis::Columns, Band::Insert { .. }) => {
                    concat!("inserting columns through a sheet carrying ", $what)
                }
                (Axis::Columns, Band::Remove { .. }) => {
                    concat!("removing columns through a sheet carrying ", $what)
                }
            }
        };
    }
    match element {
        "oleObjects" => (through!("oleObjects"), true),
        "controls" => (through!("controls"), true),
        "scenarios" => (through!("scenarios"), true),
        "dataConsolidate" => (through!("dataConsolidate"), true),
        "customSheetViews" => (through!("customSheetViews"), true),
        "smartTags" => (through!("smartTags"), true),
        "extLst" => (through!("an extension this crate does not model"), true),
        _ => (through!("an element this crate does not model"), false),
    }
}

/// What one [`Workbook::set_style`] changes, planned before anything is.
#[derive(Default)]
struct StylePlan {
    /// Cells and the style each takes, blank ones put where none is.
    cells: Vec<(CellRef, StyleId)>,
    /// Rows whose own style changes.
    rows: Vec<(u32, StyleId)>,
    /// Runs of columns whose own style changes.
    columns: Vec<(std::ops::Range<u32>, StyleId)>,
}

/// Style inheritance for an opened band, resolved before any workbook mutation.
/// The sparse overrides are only cells that differ from the inserted parent
/// row/column style. A default band therefore retains no cell or style entry.
struct Inheritor {
    derived: HashMap<(StyleId, NumberFormat, StyleId), StyleId>,
    plan: StylePlan,
}

impl Inheritor {
    fn derive(
        &mut self,
        table: &mut Arc<StyleSheet>,
        source: StyleId,
        format: NumberFormat,
        following: StyleId,
    ) -> Result<StyleId> {
        let format = if table.overrides(source, format) {
            format
        } else {
            NumberFormat::General
        };
        let border = table
            .style(source)
            .map_or_else(Default::default, |style| style.border);
        let following_border = table
            .style(following)
            .map_or_else(Default::default, |style| style.border);
        let inherited = border
            .inherited(following_border)
            .ok_or_else(|| Error::Unsupported {
                operation: "inserting through inner or non-outline cell borders",
                filesystem: format_smolstr!("$.styles[{},{}]", source.as_u16(), following.as_u16()),
            })?;
        if inherited == border && format == NumberFormat::General {
            return Ok(source);
        }
        let key = (source, format, following);
        if let Some(&derived) = self.derived.get(&key) {
            return Ok(derived);
        }
        let mut style = table.shown_style(source, format).into_owned();
        style.border = inherited;
        let derived = match table.find(&style) {
            Some(id) => id,
            None => {
                table.check_append()?;
                Arc::make_mut(table).intern(&style)?
            }
        };
        self.derived.insert(key, derived);
        Ok(derived)
    }

    fn cell(
        &mut self,
        sheet: &Sheet,
        table: &mut Arc<StyleSheet>,
        axis: Axis,
        band: Range<u32>,
        cross: u32,
        baseline: StyleId,
    ) -> Result<()> {
        let (at, end) = (band.start, band.end);
        let source = match axis {
            Axis::Rows => CellRef::new(at - 1, cross),
            Axis::Columns => CellRef::new(cross, at - 1),
        };
        let following = axis.with(source, at);
        let format = sheet
            .cell(source)
            .map_or(NumberFormat::General, Cell::format);
        let style = self.derive(
            table,
            sheet.style_at(source),
            format,
            sheet.style_at(following),
        )?;
        if style == baseline {
            return Ok(());
        }
        let count = u64::from(end - at);
        if self.plan.cells.len() as u64 + count > MAX_EDITED_CELLS {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{}!{}", sheet.name(), axis.whole(at, end)),
                reason: format_smolstr!(
                    "expected insertion putting at most {MAX_EDITED_CELLS} styled blank cells, got more"
                ),
            });
        }
        self.plan
            .cells
            .extend((at..end).map(|index| (axis.with(source, index), style)));
        Ok(())
    }

    fn rows(
        &mut self,
        sheet: &Sheet,
        table: &mut Arc<StyleSheet>,
        at: u32,
        end: u32,
    ) -> Result<()> {
        let previous = sheet.row_style(at - 1);
        let following = sheet.row_style(at);
        let inherited = previous
            .map(|source| {
                self.derive(
                    table,
                    source,
                    NumberFormat::General,
                    following.unwrap_or_default(),
                )
            })
            .transpose()?;
        if let Some(style) = inherited {
            self.plan.rows.extend((at..end).map(|row| (row, style)));
        }
        // Only a row-style/column-style precedence boundary can change an
        // otherwise empty cell. Visit those declared column spans, never gaps.
        if previous.is_some() != following.is_some() {
            for (columns, format) in &sheet.layout().columns.0 {
                let Some(column) = format.style else { continue };
                let baseline = inherited.unwrap_or(column);
                let ordinary = self.derive(
                    table,
                    previous.unwrap_or(column),
                    NumberFormat::General,
                    following.unwrap_or(column),
                )?;
                if ordinary != baseline {
                    for column in columns.clone() {
                        self.cell(sheet, table, Axis::Rows, at..end, column, baseline)?;
                    }
                }
            }
        }
        let mut before = sheet
            .row(at - 1)
            .into_iter()
            .flat_map(|row| row.cells())
            .peekable();
        let mut after = sheet
            .row(at)
            .into_iter()
            .flat_map(|row| row.cells())
            .peekable();
        loop {
            let column = match (before.peek(), after.peek()) {
                (Some(a), Some(b)) => a.column().min(b.column()),
                (Some(cell), None) | (None, Some(cell)) => cell.column(),
                (None, None) => break,
            };
            if before.peek().is_some_and(|cell| cell.column() == column) {
                before.next();
            }
            if after.peek().is_some_and(|cell| cell.column() == column) {
                after.next();
            }
            let column_style = sheet.column_style(column).unwrap_or_default();
            let baseline = inherited.unwrap_or(column_style);
            let ordinary = self.derive(
                table,
                previous.unwrap_or(column_style),
                NumberFormat::General,
                following.unwrap_or(column_style),
            )?;
            // The column span above already examined explicit exceptions as
            // well as empty cells, so each target cell is planned once.
            if ordinary == baseline {
                self.cell(sheet, table, Axis::Rows, at..end, column, baseline)?;
            }
        }
        Ok(())
    }

    fn columns(
        &mut self,
        sheet: &Sheet,
        table: &mut Arc<StyleSheet>,
        at: u32,
        end: u32,
    ) -> Result<()> {
        let previous = sheet.column_style(at - 1);
        let following = sheet.column_style(at);
        let inherited = previous
            .map(|source| {
                self.derive(
                    table,
                    source,
                    NumberFormat::General,
                    following.unwrap_or_default(),
                )
            })
            .transpose()?;
        if let Some(style) = inherited {
            self.plan.columns.push((at..end, style));
        }
        // Row formats override both old columns and the inserted column, so
        // only a stored cell can introduce a row exception here.
        for row in sheet.rows() {
            if row.cell(at - 1).is_none() && row.cell(at).is_none() {
                continue;
            }
            let baseline = sheet
                .row_style(row.index())
                .or(inherited)
                .unwrap_or_default();
            self.cell(sheet, table, Axis::Columns, at..end, row.index(), baseline)?;
        }
        Ok(())
    }
}

impl StylePlan {
    fn insertion(
        sheet: &Sheet,
        table: &mut Arc<StyleSheet>,
        axis: Axis,
        band: Band,
    ) -> Result<Self> {
        let Band::Insert { at, count } = band else {
            return Ok(Self::default());
        };
        if at == 0 || count == 0 {
            return Ok(Self::default());
        }
        let end = at.saturating_add(count).min(axis.limit());
        let mut inheritor = Inheritor {
            derived: HashMap::new(),
            plan: Self::default(),
        };
        match axis {
            Axis::Rows => inheritor.rows(sheet, table, at, end)?,
            Axis::Columns => inheritor.columns(sheet, table, at, end)?,
        }
        Ok(inheritor.plan)
    }

    fn apply(self, sheet: &mut Sheet, table: &StyleSheet) {
        for (cell, style) in self.cells {
            sheet.restyle(cell, style, table.number_format(style));
        }
        for (row, style) in self.rows {
            sheet.set_row_style(row, style);
        }
        sheet.set_column_styles(&self.columns);
    }
}

/// A patch applied to one sheet: each style it derives, by the style it
/// derives from, the temporal format a cell's value restates it with and
/// the border edges drawn; and what it changes.
struct Patcher<'a> {
    patch: &'a StylePatch,
    stated: Option<&'a FormatCode>,
    derived: HashMap<(StyleId, NumberFormat, u8), StyleId>,
    plan: StylePlan,
}

impl Patcher<'_> {
    /// The style `source` becomes with `edges` drawn, for a cell holding a
    /// value of `format` - which restates the style's number format where
    /// it does not read as it ([`StyleSheet::shown_style`]): derived once,
    /// found in the table or appended to it; `source` itself when the patch
    /// changes nothing the cell shows.
    fn derive(
        &mut self,
        table: &mut StyleSheet,
        source: StyleId,
        format: NumberFormat,
        edges: u8,
    ) -> Result<StyleId> {
        let format = if table.overrides(source, format) {
            format
        } else {
            NumberFormat::General
        };
        if let Some(derived) = self.derived.get(&(source, format, edges)) {
            return Ok(*derived);
        }
        let style = table.shown_style(source, format).into_owned();
        let shown = table.shown_format(source, format).cloned();
        let patched = self
            .patch
            .applied(&style, shown.as_ref(), self.stated, edges);
        let derived = if patched == style {
            source
        } else {
            table.intern(&patched)?
        };
        self.derived.insert((source, format, edges), derived);
        Ok(derived)
    }

    /// The border edges the patch draws on a cell at `at` of `range`.
    fn edges(&self, range: CellRange, at: CellRef) -> u8 {
        self.patch.borders.map_or(0, |borders| {
            borders.edges(Outline {
                top: at.row() == range.start().row(),
                bottom: at.row() == range.end().row(),
                left: at.column() == range.start().column(),
                right: at.column() == range.end().column(),
            })
        })
    }

    /// Plan the patch on the cell at `at`, `edges` drawn on it.
    fn cell(
        &mut self,
        sheet: &Sheet,
        table: &mut StyleSheet,
        at: CellRef,
        edges: u8,
    ) -> Result<()> {
        let held = sheet.cell(at);
        let source = held.map_or_else(|| sheet.style_at(at), Cell::style);
        let format = held.map_or(NumberFormat::General, Cell::format);
        let derived = self.derive(table, source, format, edges)?;
        if derived != source {
            self.plan.cells.push((at, derived));
        }
        Ok(())
    }

    /// Plan the patch on the row `row`'s own style, `edges` drawn on it.
    fn row(&mut self, sheet: &Sheet, table: &mut StyleSheet, row: u32, edges: u8) -> Result<()> {
        let source = sheet.row_style(row).unwrap_or_default();
        let derived = self.derive(table, source, NumberFormat::General, edges)?;
        if derived != source {
            self.plan.rows.push((row, derived));
        }
        Ok(())
    }

    /// Plan the patch over `ranges`, appending to `table` every style it
    /// derives.
    ///
    /// Every cell of a bounded range is patched, and the cells an open one
    /// holds; the rows of a range of whole rows and the columns of one of
    /// whole columns take the patch as their own style, drawing the edges
    /// every cell of them shares. Each cell, row and column is patched
    /// once, with the edges of every range holding it: in the order of the
    /// one range, and for several in the order of the sheet.
    fn plan(&mut self, sheet: &Sheet, table: &mut StyleSheet, ranges: &[CellRange]) -> Result<()> {
        let single = ranges.len() == 1;
        let mut cells: BTreeMap<CellRef, u8> = BTreeMap::new();
        let mut rows: BTreeMap<u32, u8> = BTreeMap::new();
        let mut columns: BTreeMap<u32, u8> = BTreeMap::new();
        for range in ranges {
            let (start, end) = (range.start(), range.end());
            let whole_columns = start.row() == 0 && range.is_row_open();
            let whole_rows = !whole_columns && start.column() == 0 && range.is_column_open();
            if range.is_row_open() || range.is_column_open() {
                for cell in sheet.cells_in(*range) {
                    let at = cell.reference();
                    let edges = self.edges(*range, at);
                    if single {
                        self.cell(sheet, table, at, edges)?;
                    } else {
                        *cells.entry(at).or_default() |= edges;
                    }
                }
            } else {
                for at in range.cells() {
                    let edges = self.edges(*range, at);
                    if single {
                        self.cell(sheet, table, at, edges)?;
                    } else {
                        *cells.entry(at).or_default() |= edges;
                    }
                }
            }
            if whole_rows {
                for row in start.row()..=end.row() {
                    // A row's own style draws what every cell of it shares:
                    // its top and bottom edges, no left or right.
                    let edges = self.edges(*range, CellRef::new(row, 1))
                        & self.edges(*range, CellRef::new(row, MAX_COLUMNS - 2));
                    if single {
                        self.row(sheet, table, row, edges)?;
                    } else {
                        *rows.entry(row).or_default() |= edges;
                    }
                }
            }
            if whole_columns {
                // A column's own style draws its left and right edges, no
                // top or bottom.
                for column in start.column()..=end.column() {
                    let edges = self.edges(*range, CellRef::new(1, column))
                        & self.edges(*range, CellRef::new(MAX_ROWS - 2, column));
                    *columns.entry(column).or_default() |= edges;
                }
            }
        }
        for (at, edges) in cells {
            self.cell(sheet, table, at, edges)?;
        }
        for (row, edges) in rows {
            self.row(sheet, table, row, edges)?;
        }
        let (Some(&first), Some(&last)) = (columns.keys().next(), columns.keys().next_back())
        else {
            return Ok(());
        };
        // Consecutive columns of one style patched alike are one run.
        for (run, style) in sheet.column_runs(first..last + 1) {
            let source = style.unwrap_or_default();
            for column in run {
                let Some(&edges) = columns.get(&column) else {
                    continue;
                };
                let derived = self.derive(table, source, NumberFormat::General, edges)?;
                if derived == source {
                    continue;
                }
                match self.plan.columns.last_mut() {
                    Some((span, held)) if *held == derived && span.end == column => {
                        span.end += 1;
                    }
                    _ => self.plan.columns.push((column..column + 1, derived)),
                }
            }
        }
        Ok(())
    }
}

/// Put `entry` in the cell at `at` of `sheet` in the style `wanted` -
/// `source`, the style the cell shows, where that is its own; else found
/// in `table` or appended to it - answering the cell it replaced. A refusal
/// leaves the sheet as it was; what it appended to `table` is the caller's
/// to take back.
fn enter(
    sheet: &mut Sheet,
    table: &mut StyleSheet,
    at: CellRef,
    entry: Entry,
    source: StyleId,
    wanted: &CellStyle,
    system: DateSystem,
) -> Result<Option<Cell>> {
    let own = table
        .style(source)
        .map_or_else(|| *wanted == CellStyle::default(), |own| own == wanted);
    let target = if own { source } else { table.intern(wanted)? };
    let kind = table.number_format(target);
    let mut serial_bits = None;
    let cell = match entry {
        Entry::Blank => {
            // A blank cell showing what its row or column shows is no cell
            // at all.
            let around = sheet
                .row_style(at.row())
                .or_else(|| sheet.column_style(at.column()))
                .unwrap_or_default();
            if target == around {
                return Ok(sheet.remove_cell(at));
            }
            Cell::new(at, CellKind::Number, kind, Scalar::Null)
        }
        Entry::Formula(formula) => {
            Cell::new(at, CellKind::Number, kind, Scalar::Null).with_formula(formula)
        }
        Entry::Text(text) | Entry::Quoted(text) => {
            Cell::from_scalar(at, Scalar::from(text), system)?
        }
        Entry::Error(error) => Cell::new(at, CellKind::Error, kind, Scalar::Null).with_error(error),
        Entry::Value { value, .. } => {
            let mut cell = Cell::from_scalar(at, value, system)?;
            let used = cell.restyle(target, kind, system, None);
            serial_bits = used
                .and_then(|raw| super::sheet::CellExtra::exceptional_serial(&cell, raw, system));
            cell
        }
    };
    let replaced = sheet.insert_cell(cell.with_style(target))?;
    if let Some(bits) = serial_bits {
        sheet.attach_serial_bits(at, bits);
    }
    Ok(replaced)
}

/// What one save decided about the package documents.
struct Documents<'a> {
    /// The source's members.
    members: &'a [SmolStr],
    relationships: &'a Relationships,
    /// The workbook's relationships part as the source stores it, when the
    /// save read it.
    relationships_bytes: Option<&'a [u8]>,
    /// Each slot's part, in tab order.
    parts: &'a [SmolStr],
    /// Each slot's relationship id, in tab order.
    ids: &'a [SmolStr],
    /// The slots whose relationship this save adds.
    fresh: &'a [usize],
    /// The number the next relationship this save adds takes, `None` once
    /// none is left.
    next_id: Option<usize>,
    list_changed: bool,
    rewrote: bool,
    /// The parts this save leaves out: tabs gone and what only they
    /// reached, the calculation chain.
    dropped: &'a [SmolStr],
    /// The workbook relationships of the pivot caches this save leaves out.
    dropped_caches: &'a [SmolStr],
    /// The shared parts this save creates.
    strings_part: Option<&'a str>,
    styles_part: Option<&'a str>,
}

/// The elements of a workbook part a `workbookPr` goes before.
const AFTER_WORKBOOK_PROPERTIES: &[&[u8]] = &[
    b"AlternateContent",
    b"revisionPtr",
    b"workbookProtection",
    b"bookViews",
    b"sheets",
];

/// The elements of a workbook part a `calcPr` goes before.
const AFTER_CALCULATION_PROPERTIES: &[&[u8]] = &[
    b"oleSize",
    b"customWorkbookViews",
    b"pivotCaches",
    b"smartTagPr",
    b"smartTagTypes",
    b"webPublishing",
    b"fileRecoveryPr",
    b"webPublishObjects",
    b"extLst",
];

/// The `sheetId` a new tab takes: one past `last`, else - a tab numbered
/// the highest there is - the lowest number `taken` says no tab has.
fn next_sheet_id(last: u32, taken: impl Fn(u32) -> bool) -> u32 {
    last.checked_add(1)
        .or_else(|| (1..=u32::MAX).find(|id| !taken(*id)))
        .unwrap_or(u32::MAX)
}

/// The relationship id `next` names, moving it past; a refusal naming
/// `part` once no number is left.
fn take_id(next: &mut Option<usize>, part: &str) -> Result<SmolStr> {
    let number = next.ok_or_else(|| Error::InvalidRecord {
        path: SmolStr::new(part),
        reason: format_smolstr!(
            "expected a relationship id past every one the part states, got none left past \
             rId{}",
            usize::MAX
        ),
    })?;
    *next = number.checked_add(1);
    Ok(format_smolstr!("rId{number}"))
}

/// A worksheet written from a stream in place of the one held: the slot it
/// takes, the temporal formats its cells are written under, and the part's
/// stream once the styles of those formats are interned.
pub(crate) struct Replaced<'a> {
    pub(crate) at: usize,
    pub(crate) formats: Vec<NumberFormat>,
    pub(crate) stream: SheetStream<'a>,
}

/// The stream of a worksheet part, opened once the temporal styles its cells
/// are written under are interned.
pub(crate) struct NamedTable {
    pub(crate) sheet: SmolStr,
    pub(crate) table: Table,
    pub(crate) columns: Vec<SmolStr>,
    pub(crate) part: SmolStr,
    pub(crate) bytes: Arc<[u8]>,
    pub(crate) siblings: SmallVec<[(SmolStr, CellRange); 4]>,
}

pub(crate) struct SheetReplacement {
    pub(crate) stream: Box<dyn Read + Send>,
    pub(crate) parts: Vec<(SmolStr, Vec<u8>)>,
}

pub(crate) type SheetStream<'a> =
    Box<dyn for<'s> FnOnce(&mut Splice<'s>) -> Result<SheetReplacement> + 'a>;

/// A worksheet part with no cell.
const EMPTY_SHEET: &str = "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData/></worksheet>";

/// One `<sheet>` entry of the workbook part.
struct SheetEntry {
    name: SmolStr,
    rid: SmolStr,
    sheet_id: Option<u32>,
    state: SheetState,
}

/// One `<definedName>` of the workbook part.
struct NameEntry {
    raw: Vec<u8>,
    name: SmolStr,
    text: String,
    local_sheet_id: Option<usize>,
    hidden: bool,
    comment: Option<SmolStr>,
}

/// One `<workbookView>`: the tab it shows and the first tab its strip
/// shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct View {
    active_tab: Option<usize>,
    first_sheet: Option<usize>,
}

/// One authored cache identity. Validation is deferred until pivot inventory;
/// ordinary workbook opening may carry an opaque foreign cache unchanged.
struct PivotCacheEntry {
    id: SmolStr,
    rid: SmolStr,
}

/// What the workbook part states that the model reads.
struct WorkbookDocument {
    family: NamespaceFamily,
    text_compatibility: super::formula::text::Compatibility,
    sheets: Vec<SheetEntry>,
    system: DateSystem,
    names: Vec<NameEntry>,
    views: Vec<View>,
    /// Authored cache identities, including unvalidated foreign spellings.
    pivot_caches: Vec<PivotCacheEntry>,
}

/// Read the workbook part: its sheets, its date system, its defined names,
/// its views and its pivot caches.
fn read_workbook(bytes: &[u8], part: &str) -> Result<WorkbookDocument> {
    use quick_xml::events::Event;

    let mut reader = super::styles::reader(bytes);
    let mut buffer = Vec::new();
    let mut document = WorkbookDocument {
        family: NamespaceFamily::default(),
        text_compatibility: super::formula::text::Compatibility::default(),
        sheets: Vec::new(),
        system: DateSystem::Year1900,
        names: Vec::new(),
        views: Vec::new(),
        pivot_caches: Vec::new(),
    };
    let mut root_seen = false;
    let mut namespaces = quick_xml::name::NamespaceResolver::default();
    let mut compatibility_list = false;
    let mut compatibility_extension = false;
    let mut compatibility_extensions = 0_u8;
    let mut compatibility_seen = false;
    let compatibility_refusal = |reason: SmolStr| Error::InvalidRecord {
        path: format_smolstr!("{part}#extLst/ext/version"), reason,
    };
    let root_refusal = |actual: &str| Error::InvalidRecord {
        path: format_smolstr!("{part}#workbook"),
        reason: format_smolstr!(
            "expected a workbook root in the Transitional or Strict namespace family, got {actual}"
        ),
    };
    // The `<definedName>` being read: its element so far, and its text.
    let mut name: Option<(NameEntry, usize)> = None;
    let refused = |error: Error| Error::InvalidRecord {
        path: SmolStr::new(part),
        reason: format_smolstr!("{error}"),
    };
    loop {
        let position = usize::try_from(reader.buffer_position()).unwrap_or(usize::MAX);
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| package::codec_error(position, error.to_string()))?;
        // The same event pass resolves the extension's complete ancestor
        // path. Declarations are decoded before NamespaceResolver receives
        // them, including character references in a namespace URI.
        match &event {
            Event::Start(start) | Event::Empty(start) => {
                let level = namespaces.level().checked_add(1).ok_or_else(|| {
                    package::codec_error(position, "expected XML nesting within the namespace resolver's limit")
                })?;
                namespaces.set_level(level);
                let mut declarations = 0;
                for attribute in start.attributes().with_checks(level == 1) {
                    let attribute = attribute.map_err(|error| package::codec_error(position, error.to_string()))?;
                    let Some(prefix) = attribute.key.as_namespace_binding() else { continue };
                    declarations += 1;
                    if declarations > namespaces.max_declarations_per_element() {
                        return Err(package::codec_error(position, "too many namespace declarations on one element"));
                    }
                    let value = attribute.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|error| package::codec_error(position, error.to_string()))?;
                    namespaces.add(prefix, quick_xml::name::Namespace(value.as_bytes()))
                        .map_err(|error| package::codec_error(position, error.to_string()))?;
                }
                let (namespace, local) = namespaces.resolve_element(start.name());
                if level == 1 {
                    if root_seen || local.as_ref() != b"workbook" {
                        return Err(root_refusal(&format!("root {:?}", String::from_utf8_lossy(start.name().as_ref()))));
                    }
                    let quick_xml::name::ResolveResult::Bound(uri) = &namespace else {
                        return Err(root_refusal("an unbound root namespace"));
                    };
                    document.family = std::str::from_utf8(uri.as_ref()).ok()
                        .and_then(NamespaceFamily::from_namespace)
                        .ok_or_else(|| root_refusal(&format!("namespace {uri:?}")))?;
                    root_seen = true;
                }
                let main = matches!(&namespace, quick_xml::name::ResolveResult::Bound(uri)
                    if uri.as_ref() == document.family.namespace().as_bytes());
                if level == 2 {
                    compatibility_list = main && local.as_ref() == b"extLst";
                }
                if level == 3 {
                    compatibility_extension = compatibility_list && main && local.as_ref() == b"ext"
                        && package::exact_attribute(start, b"uri", position)?.as_deref()
                            == Some(super::formula::text::Compatibility::EXTENSION);
                    if compatibility_extension {
                        compatibility_extensions += 1;
                        if compatibility_extensions > 1 {
                            return Err(compatibility_refusal("expected one compatibility extension, got multiple extensions".into()));
                        }
                    }
                }
                if level == 4 && compatibility_extension && local.as_ref() == b"version"
                    && matches!(&namespace, quick_xml::name::ResolveResult::Bound(uri)
                        if uri.as_ref() == super::formula::text::Compatibility::NAMESPACE.as_bytes())
                {
                    if compatibility_seen {
                        return Err(compatibility_refusal("expected one compatibility version, got multiple versions".into()));
                    }
                    compatibility_seen = true;
                    let (mut version, mut warning) = (None, None);
                    for attribute in start.attributes() {
                        let attribute = attribute.map_err(|error| compatibility_refusal(format_smolstr!("expected unique compatibility attributes, got {error}")))?;
                        let target = match attribute.key.as_ref() {
                            b"setVersion" => &mut version,
                            b"warnBelowVersion" => &mut warning,
                            _ => continue,
                        };
                        let value = attribute.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .map_err(|error| compatibility_refusal(format_smolstr!("expected a decoded compatibility attribute, got {error}")))?;
                        *target = Some(value.trim().parse::<u32>().map_err(|_| compatibility_refusal(format_smolstr!("expected an unsigned 32-bit compatibility version, got {value:?}")))?);
                    }
                    if warning == Some(0) || matches!((version, warning), (Some(version), Some(warning)) if version < warning) {
                        return Err(compatibility_refusal(format_smolstr!("expected a positive warnBelowVersion no greater than setVersion, got {warning:?} and {version:?}")));
                    }
                    document.text_compatibility = super::formula::text::Compatibility::from_version(version);
                }
                if matches!(event, Event::Empty(_)) { namespaces.pop(); }
            }
            Event::End(_) => { namespaces.pop(); }
            _ => {}
        }
        if let Some((entry, depth)) = name.as_mut() {
            super::carried::write_raw(&event, &mut entry.raw);
            match &event {
                Event::Start(_) => *depth += 1,
                Event::End(_) => {
                    *depth -= 1;
                    if *depth == 0 {
                        let (entry, _) = name.take().expect("a name is being read");
                        document.names.push(entry);
                    }
                }
                Event::Text(text) if *depth == 1 => {
                    entry
                        .text
                        .push_str(&package::text_piece(text.xml10_content(), position)?);
                }
                Event::CData(text) if *depth == 1 => {
                    entry
                        .text
                        .push_str(&package::text_piece(text.xml10_content(), position)?);
                }
                Event::GeneralRef(_) if *depth == 1 => {
                    entry
                        .text
                        .push_str(&package::reference_text(&event, position)?);
                }
                _ => {}
            }
            buffer.clear();
            continue;
        }
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                match package::local_name(start.name().as_ref()) {
                    b"sheet" => {
                        let name =
                            package::attribute(start, b"name", position)?.unwrap_or_default();
                        let rid = package::attribute(start, b"id", position)?.unwrap_or_default();
                        let sheet_id = package::attribute(start, b"sheetId", position)?
                            .and_then(|id| id.trim().parse::<u32>().ok());
                        let state = SheetState::from_attribute(
                            package::attribute(start, b"state", position)?.as_deref(),
                        )
                        .map_err(refused)?;
                        document.sheets.push(SheetEntry {
                            name: SmolStr::new(name),
                            rid: SmolStr::new(rid),
                            sheet_id,
                            state,
                        });
                    }
                    b"workbookPr" => {
                        if let Some(value) = package::attribute(start, b"date1904", position)? {
                            document.system = match value.trim() {
                                "1" | "true" => DateSystem::Year1904,
                                "0" | "false" => DateSystem::Year1900,
                                other => {
                                    return Err(Error::InvalidRecord {
                                        path: format_smolstr!("{part}#workbookPr/@date1904"),
                                        reason: format_smolstr!(
                                            "expected 1, true, 0 or false for date1904, got {other:?}"
                                        ),
                                    });
                                }
                            };
                        }
                    }
                    b"workbookView" => {
                        let index = |value: Option<std::borrow::Cow<'_, str>>| {
                            value.and_then(|value| value.trim().parse::<usize>().ok())
                        };
                        document.views.push(View {
                            active_tab: index(package::attribute(start, b"activeTab", position)?),
                            first_sheet: index(package::attribute(start, b"firstSheet", position)?),
                        });
                    }
                    b"pivotCache" => {
                        let id = package::attribute(start, b"cacheId", position)?
                            .unwrap_or_default();
                        let rid = package::attribute(start, b"id", position)?
                            .unwrap_or_default();
                        document.pivot_caches.push(PivotCacheEntry {
                            id: SmolStr::new(id),
                            rid: SmolStr::new(rid),
                        });
                    }
                    b"definedName" => {
                        let flag = |value: Option<std::borrow::Cow<'_, str>>| {
                            value.is_some_and(|value| matches!(value.trim(), "1" | "true"))
                        };
                        let mut entry = NameEntry {
                            raw: Vec::new(),
                            name: SmolStr::new(
                                package::attribute(start, b"name", position)?.unwrap_or_default(),
                            ),
                            text: String::new(),
                            local_sheet_id: package::attribute(start, b"localSheetId", position)?
                                .and_then(|id| id.trim().parse::<usize>().ok()),
                            hidden: flag(package::attribute(start, b"hidden", position)?),
                            comment: package::attribute(start, b"comment", position)?
                                .map(SmolStr::new),
                        };
                        super::carried::write_raw(&event, &mut entry.raw);
                        if matches!(event, Event::Start(_)) {
                            name = Some((entry, 1));
                        } else {
                            document.names.push(entry);
                        }
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if !root_seen {
        return Err(root_refusal("no root element"));
    }
    if compatibility_extensions != 0 && !compatibility_seen {
        return Err(compatibility_refusal("expected one genuine compatibility version child, got none".into()));
    }
    Ok(document)
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! Borrowed reference bindings pinned by the mirrored workbook tests.
    /// Actual typed-node visits for the last registered/evaluated cell, plus
    /// its retained input registration and scheduled-cell counts.
    /// No timing or expanded-grid proxy.
    #[must_use]
    pub fn calculation_work(book: &super::Workbook) -> (usize, usize, usize, usize) {
        let calculation = book.stated.calculation.as_ref().expect("calculation was requested");
        (calculation.dependency_nodes, calculation.evaluator.visited(), calculation.precedents.len(), calculation.schedule.ordered().len())
    }

    /// Actual cache-miss element operations for the last evaluated formula.
    /// The counter is absent from production builds.
    #[must_use]
    pub fn array_work(book: &super::Workbook) -> usize {
        book.stated.calculation.as_ref().expect("calculation was requested").evaluator.array_steps()
    }

    use super::{Cell, CellRange, CellRef, DefinedName, SheetKey, Workbook};
    use crate::excel::cell::ExcelError;
    use crate::excel::formula::{Formula, parser::Node};
    use crate::{Error, Result};

    /// One explicit, parsed workbook reference context.
    pub struct References<'w>(super::ReferenceResolver<'w>);
    /// The private result, forwarding its borrowed data without a second parser.
    pub enum Resolved<'w> {
        /// A proven worksheet rectangle or worksheet-only 3D span.
        Range(Range<'w>),
        /// A borrowed defined-name expression and its evaluation scope.
        Name(Name<'w>),
        /// A computed missing-name or missing-reference Excel error.
        Error(ExcelError),
        /// A named semantic boundary whose cached value must stay held.
        Held(&'static str),
    }
    /// A borrowed range view.
    pub struct Range<'w> {
        book: &'w Workbook,
        area: super::ReferenceArea,
    }
    /// An existing defined-name expression and its context.
    pub struct Name<'w>(super::NameBinding<'w>);

    impl<'w> References<'w> {
        /// Parse the workbook's worksheets once and index its existing names.
        ///
        /// # Errors
        ///
        /// Returns a worksheet or source refusal from Workbook::parse_all.
        pub fn new(book: &'w Workbook) -> Result<Self> {
            super::ReferenceResolver::new(book).map(Self)
        }

        /// Bind a single reference expression at the requested cell.
        ///
        /// # Errors
        ///
        /// Returns a located ambiguous-name refusal or a non-reference test
        /// input refusal. Missing sheets/names are Resolved::Error values.
        pub fn resolve(&self, sheet: &str, formula: &Formula, at: CellRef) -> Result<Resolved<'w>> {
            let Some(key) = self.0.book.sheet_key(sheet) else {
                return Ok(Resolved::Error(ExcelError::Ref));
            };
            let Some(host) = self.0.host(key, at) else {
                return Ok(Resolved::Error(ExcelError::Ref));
            };
            self.root(host, formula)
        }

        /// Bind a name whose already-parsed expression has one reference root.
        ///
        /// # Errors
        ///
        /// Returns the same ambiguous-name and test-input refusals as resolve.
        pub fn resolve_name_reference(&self, binding: &Name<'w>) -> Result<Resolved<'w>> {
            self.root(binding.0.host(), binding.0.definition().formula())
        }

        fn root(&self, host: super::ReferenceHost<'w>, formula: &Formula) -> Result<Resolved<'w>> {
            let expression = match formula.expression() {
                Ok(expression) => expression,
                Err(reason) => return Ok(Resolved::Held(reason.as_str())),
            };
            let Node::Reference(reference) = &expression.nodes[expression.root] else {
                return Err(Error::InvalidRecord {
                    path: smol_str::SmolStr::new_static("reference-test"),
                    reason: smol_str::SmolStr::new_static(
                        "expected one reference expression root, got another node",
                    ),
                });
            };
            self.0
                .resolve(host, reference)
                .map(|resolved| match resolved {
                    super::ResolvedReference::Range(view) => Resolved::Range(Range {
                        book: view.book,
                        area: view.descriptor(),
                    }),
                    super::ResolvedReference::Name(binding) => Resolved::Name(Name(binding)),
                    super::ResolvedReference::Error(error) => Resolved::Error(error),
                    super::ResolvedReference::Held(reason) => Resolved::Held(match reason {
                        super::ReferenceHeld::Shape(reason) => reason.as_str(),
                        super::ReferenceHeld::NameAnchor => "defined-name anchor",
                        super::ReferenceHeld::NameScope => "qualified workbook name scope",
                        super::ReferenceHeld::NonWorksheet => "non-worksheet reference span",
                    }),
                })
        }
    }

    impl<'w> Range<'w> {
        /// Lend the exact dependency rectangles without reading cells.
        pub fn areas(&self) -> impl Iterator<Item = (SheetKey, CellRange)> + 'w {
            self.area.view(self.book).areas()
        }
        /// Lend physical cells in tab and coordinate order, without blanks.
        pub fn cells(&self) -> impl Iterator<Item = (SheetKey, &'w Cell)> + 'w {
            self.area.view(self.book).cells()
        }
        /// Count the logical grid positions, including physically absent cells.
        #[must_use]
        pub fn logical_len(&self) -> u128 {
            self.area.view(self.book).logical_len()
        }
    }

    impl<'w> Name<'w> {
        /// Borrow the original definition, which also identifies expansion cycles.
        #[must_use]
        pub fn definition(&self) -> &'w DefinedName {
            self.0.definition()
        }
    }
}

// Dependently replace the first vertical intake after matrix/subtotal writers.
// All XML selection remains Registration-owned; foreign layouts return None.
impl Workbook {
    fn vertical_pivot_spec(
        &self,
        name: &str,
        table: &Registration,
        cache: &Registration,
        table_part: &str,
        cache_part: &str,
        unrepresented: &mut Option<SmolStr>,
    ) -> Result<Option<PivotSpec>> {
        let main = &[super::NAMESPACE, super::STRICT_NAMESPACE];
        if !table.children_named(main, "pageFields")?.is_empty()
            || !table.children_named(main, "filters")?.is_empty()
            || !Self::native_vertical_extensions(table, cache)?
            || !matches!(cache.root_attribute(b"saveData")?.as_deref(), None | Some("0"))
            || !matches!(cache.root_attribute(b"refreshOnLoad")?.as_deref(), None | Some("1"))
        {
            return Ok(None);
        }
        let source = cache.one_child(main, "cacheSource", cache_part)?;
        if source.root_attribute(b"type")?.as_deref() != Some("worksheet") {
            return Ok(None);
        }
        let worksheet = source.one_child(main, "worksheetSource", cache_part)?;
        let (Some(source_sheet), Some(source_range)) = (
            worksheet.root_attribute(b"sheet")?,
            worksheet
                .root_attribute(b"ref")?
                .and_then(|text| text.parse::<CellRange>().ok()),
        ) else {
            return Ok(None);
        };
        let fields = cache
            .one_child(main, "cacheFields", cache_part)?
            .children_named(main, "cacheField")?;
        if fields.len() != source_range.column_size() as usize {
            return Ok(None);
        }
        let mut headers = Vec::with_capacity(fields.len());
        for field in &fields {
            let Some(label) = field.root_attribute(b"name")? else {
                return Ok(None);
            };
            headers.push(match super::shared_strings::decode(&label) {
                std::borrow::Cow::Borrowed(_) => label,
                std::borrow::Cow::Owned(decoded) => decoded,
            });
        }
        let pivot_fields = table
            .one_child(main, "pivotFields", table_part)?
            .children_named(main, "pivotField")?;
        if pivot_fields.len() != headers.len() {
            return Ok(None);
        }
        let mut read_axis = |index: usize, kind: &str| -> Result<Option<AxisField>> {
            let Some(field) = pivot_fields.get(index) else {
                return Ok(None);
            };
            if field.root_attribute(b"axis")?.as_deref() != Some(kind) {
                return Ok(None);
            }
            // Compare item names against the existing typed caption owners;
            // custom names and hidden members have no PivotSpec slot.
            let shared = fields[index].children_named(main, "sharedItems")?;
            if shared.len() > 1 {
                return Ok(None);
            }
            let cache_items = if let Some(shared) = shared.first() {
                let Some(items) = shared.children_with_names(main)? else {
                    *unrepresented = Some(format_smolstr!(
                        "{cache_part}#cacheFields[{index}]/sharedItems: foreign item namespace"
                    ));
                    return Ok(None);
                };
                items
            } else {
                Vec::new()
            };
            for items in field.children_named(main, "items")? {
                for item in items.children_named(main, "item")? {
                    if !matches!(item.root_attribute(b"h")?.as_deref(), None | Some("0") | Some("false")) {
                        *unrepresented = Some(format_smolstr!(
                            "{table_part}#pivotFields[{index}]/items: hidden item is not represented"
                        ));
                        return Ok(None);
                    }
                    let item_index = item.root_attribute(b"x")?
                        .and_then(|value| value.parse::<usize>().ok());
                    let item_kind = item.root_attribute(b"t")?;
                    if item_kind.as_deref() == Some("default") {
                        if item_index.is_some() || item.root_attribute(b"n")?.is_some() {
                            *unrepresented = Some(format_smolstr!(
                                "{table_part}#pivotFields[{index}]/items: malformed default item"
                            ));
                            return Ok(None);
                        }
                        continue;
                    }
                    if !matches!(item_kind.as_deref(), None | Some("data"))
                        || item_index.is_none_or(|at| at >= cache_items.len()) {
                        *unrepresented = Some(format_smolstr!(
                            "{table_part}#pivotFields[{index}]/items: item index or kind is not represented"
                        ));
                        return Ok(None);
                    }
                    if let Some(label) = item.root_attribute(b"n")? {
                        let expected = if let Some((kind, value)) =
                            item_index.and_then(|at| cache_items.get(at)) {
                            match kind.as_str() {
                                "m" => Some(SmolStr::new_static(super::pivot::BLANK_CAPTION)),
                                "b" => {
                                    let raw = value.root_attribute(b"v")?;
                                    match raw.as_deref() {
                                        Some("1") | Some("0") | Some("true") | Some("false") => {
                                            let truth = matches!(raw.as_deref(), Some("1") | Some("true"));
                                            Some(super::pivot::part::item_caption(
                                                &super::pivot::compute::PivotItem::Boolean(truth),
                                                "$.pivot.itemCaption",
                                            )?)
                                        }
                                        _ => None,
                                    }
                                },
                                "e" => value.root_attribute(b"v")?
                                    .and_then(|text| super::pivot::compute::PivotItem::Error(
                                        super::cell::ExcelError::from_text(&text)
                                    ).authored_error_label().map(SmolStr::new_static)),
                                "s" => value.root_attribute(b"v")?
                                    .map(|text| SmolStr::new(super::shared_strings::decode(&text))),
                                _ => None,
                            }
                        } else {
                            None
                        };
                        if expected.as_deref()
                            != Some(super::shared_strings::decode(&label).as_ref()) {
                            *unrepresented = Some(format_smolstr!(
                                "{table_part}#pivotFields[{index}]/items: authored item caption or index is not represented"
                            ));
                            return Ok(None);
                        }
                    }
                }
            }
            let order = match field.root_attribute(b"sortType")?.as_deref() {
                Some("ascending") => ItemOrder::Ascending,
                Some("descending") => ItemOrder::Descending,
                _ => return Ok(None),
            };
            Ok(Some(AxisField {
                field: headers[index].clone().into(),
                order,
            }))
        };
        let row_fields = table
            .one_child(main, "rowFields", table_part)?
            .children_named(main, "field")?;
        if row_fields.is_empty() {
            return Ok(None);
        }
        let mut rows = Vec::with_capacity(row_fields.len());
        let mut used = BTreeSet::new();
        for field in row_fields {
            let Some(index) = field
                .root_attribute(b"x")?
                .and_then(|text| text.parse::<usize>().ok())
            else {
                return Ok(None);
            };
            let Some(axis) = read_axis(index, "axisRow")? else {
                return Ok(None);
            };
            if !used.insert(index) {
                return Ok(None);
            }
            rows.push(axis);
        }
        let mut columns = Vec::new();
        let mut value_axis = false;
        let column_fields = table.children_named(main, "colFields")?;
        if column_fields.len() > 1 {
            return Ok(None);
        }
        if let Some(fields) = column_fields.first() {
            for field in fields.children_named(main, "field")? {
                let Some(index) = field.root_attribute(b"x")? else {
                    return Ok(None);
                };
                if index == "-2" {
                    if value_axis {
                        return Ok(None);
                    }
                    value_axis = true;
                } else {
                    let Some(index) = index.parse::<usize>().ok() else {
                        return Ok(None);
                    };
                    let Some(axis) = read_axis(index, "axisCol")? else {
                        return Ok(None);
                    };
                    if !used.insert(index) {
                        return Ok(None);
                    }
                    columns.push(axis);
                }
            }
        }
        let data_fields = table
            .one_child(main, "dataFields", table_part)?
            .children_named(main, "dataField")?;
        if data_fields.is_empty() || value_axis != (data_fields.len() > 1) {
            return Ok(None);
        }
        let mut values = Vec::with_capacity(data_fields.len());
        for value in data_fields {
            let Some(index) = value
                .root_attribute(b"fld")?
                .and_then(|text| text.parse::<usize>().ok())
            else {
                return Ok(None);
            };
            let Some(source) = headers.get(index) else {
                return Ok(None);
            };
            if used.contains(&index) {
                return Ok(None);
            }
            let number_format = match value.root_attribute(b"numFmtId")? {
                Some(text) => {
                    let Ok(id) = text.parse::<u32>() else { return Ok(None); };
                    Some(self.styles()?.code_of(id))
                }
                None => None,
            };
            let Some(aggregate) = Aggregate::from_subtotal(
                value.root_attribute(b"subtotal")?.as_deref().unwrap_or("sum"),
            ) else { return Ok(None); };
            values.push(ValueField {
                field: source.clone().into(),
                aggregate,
                caption: value.root_attribute(b"name")?
                    .map(|text| SmolStr::new(super::shared_strings::decode(&text))),
                number_format,
            });
        }
        let axis_subtotals = |name: &str| -> Result<bool> {
            let mut found = false;
            for item in table.one_child(main, name, table_part)?.children_named(main, "i")? {
                found |= item.root_attribute(b"t")?.as_deref() == Some("default");
            }
            Ok(found)
        };
        let row_subtotals = axis_subtotals("rowItems")?;
        let column_subtotals = axis_subtotals("colItems")?;
        let subtotals = row_subtotals || column_subtotals;
        // PivotSpec has one subtotal switch for both axes. A foreign pivot
        // with asymmetric parent levels cannot be regenerated faithfully.
        if row_subtotals != (subtotals && rows.len() > 1)
            || column_subtotals != (subtotals && columns.len() > 1)
        {
            return Ok(None);
        }
        let flag = |name: &[u8]| -> Result<Option<bool>> {
            Ok(match table.root_attribute(name)?.as_deref() {
                Some("1") => Some(true),
                Some("0") => Some(false),
                _ => None,
            })
        };
        let row_grand_totals = flag(b"rowGrandTotals")?.unwrap_or(true);
        let column_grand_totals = flag(b"colGrandTotals")?.unwrap_or(true);
        Ok(Some(PivotSpec {
            name: name.into(),
            source: PivotSource {
                sheet: SmolStr::new(super::shared_strings::decode(&source_sheet)),
                range: source_range,
            },
            rows,
            columns,
            values,
            subtotals,
            row_grand_totals,
            column_grand_totals,
        }))
    }
}


// Scratch continuation inside workbook.rs: one vertical pivot publisher.
// Requires the staged model, compute, layout, part, identity and inventory.

struct PivotPublication {
    host: usize,
    geometry: super::pivot::layout::Geometry,
    previous: Option<CellRange>,
    cells: Vec<Cell>,
    serials: Vec<(CellRef, u64)>,
    styles: Option<Arc<StyleSheet>>,
    plan: Plan,
}

impl PivotPublication {
    /// Nothing in this method can refuse: all cells and XML were prepared,
    /// grid/ownership checked, and the destination sheet parsed above.
    fn commit(self, workbook: &mut Workbook) -> (CellRange, Restore) {
        let sheet = &workbook.slots[self.host];
        let host = sheet.name.clone();
        let mut restore = Restore::new(workbook);
        let mut ranges = vec![self.geometry.range];
        if let Some(previous) = self.previous {
            if previous != self.geometry.range {
                ranges.push(previous);
            }
        }
        restore.push(
            workbook
                .cells_step(&host, &ranges)
                .expect("preflighted worksheet"),
        );
        let mut parts = Vec::with_capacity(self.plan.overrides.len());
        for changed in &self.plan.overrides {
            parts.push(PartRestore::new(
                changed.member.clone(),
                changed.before.clone(),
                changed.after.clone(),
            ));
        }
        if !parts.is_empty() {
            restore.push(Step::Overrides(parts));
        }
        if let Some(table) = workbook.styles.get() {
            restore.capture_styles(table);
        }
        let mark = workbook.begin_batch();
        workbook.stated.remember_slot(&workbook.slots[self.host]);
        if let Some(styles) = self.styles {
            workbook.styles = OnceLock::from(styles);
        }
        let sheet = workbook.slots[self.host]
            .parsed
            .get_mut()
            .expect("parsed in preflight");
        if let Some(previous) = self.previous {
            let previous_cells: Vec<_> = sheet.cells_in(previous).map(Cell::reference).collect();
            for at in previous_cells {
                sheet.remove_cell(at);
            }
        }
        for cell in self.cells {
            sheet.insert_cell(cell).expect("checked grid cell");
        }
        for (at, bits) in self.serials {
            sheet.attach_serial_bits(at, bits);
        }
        for changed in self.plan.overrides {
            workbook.stated.set_part(changed.member, changed.after);
        }
        workbook.invalidate_calculation();
        workbook.finish_batch(mark.start());
        (self.geometry.range, restore)
    }
}

impl Plan {
    /// Add one relationship through the package's existing scoped capture.
    fn pivot_link(
        &mut self,
        workbook: &Workbook,
        owner: &str,
        kind: &str,
        target: &str,
        cached: &PartBytes,
    ) -> Result<SmolStr> {
        let rels_part = package::relationships_part_of(owner);
        let before = self.part(workbook, &rels_part, cached)?;
        let base: Arc<[u8]> = before.clone().unwrap_or_else(|| {
            Arc::from(
                format!(
                    "<Relationships xmlns=\"{}\"/>",
                    package::PACKAGE_RELATIONSHIPS_NAMESPACE
                )
                .into_bytes(),
            )
        });
        let parsed = Relationships::from_xml(&base, owner)?;
        let number = parsed.next_id().ok_or_else(|| Error::InvalidRecord {
            path: rels_part.clone(),
            reason: "expected a free relationship ID".into(),
        })?;
        let id = format_smolstr!("rId{number}");
        let relative = package::relative_to(package::folder_of(owner), target);
        // A fragment parsed outside its OPC namespace would carry xmlns=""
        // into the destination. Capture the relationship in its real scope.
        let document = format!(
            "<Relationships xmlns=\"{}\">{}</Relationships>",
            package::PACKAGE_RELATIONSHIPS_NAMESPACE,
            package::relationship_element(&id, workbook.stated.family, kind, &relative),
        );
        let entries = Registration::root(document.as_bytes())?.members(false)?;
        let root = Registration::root(&base)?.appended(&entries)?;
        let after = Registration::replace_root(&base, &root.xml)?;
        self.set_part(rels_part, before, Some(Arc::from(after)));
        Ok(id)
    }

    /// Drop one selected relationship, keeping every sibling and its raw XML.
    fn pivot_unlink(
        &mut self,
        workbook: &Workbook,
        owner: &str,
        kind: &str,
        target: &str,
        cached: &PartBytes,
    ) -> Result<()> {
        let rels_part = package::relationships_part_of(owner);
        let before = self
            .part(workbook, &rels_part, cached)?
            .ok_or_else(|| Error::absent("pivot relationships", &rels_part))?;
        let links = Relationships::from_xml(&before, owner)?;
        let expected_type = format_smolstr!("{}/{kind}", workbook.stated.family.relationships_namespace());
        let id = links.entries().iter()
            .find(|link| link.type_uri == expected_type && link.target.as_deref() == Some(target))
            .ok_or_else(|| Error::InvalidRecord {
                path: rels_part.clone(),
                reason: format_smolstr!("expected a relationship to {target}"),
            })?
            .id
            .clone();
        if links.entries().iter().filter(|link| link.target.as_deref() == Some(target)).count() > 1 {
            return Err(Error::Unsupported {
                operation: "removing a pivot part referenced by another relationship",
                filesystem: format_smolstr!("{owner} -> {target}"),
            });
        }
        let root = Registration::root(&before)?;
        let after_root = root.without(false, &BTreeSet::from([id]))?;
        let after = Registration::replace_root(&before, &after_root.xml)?;
        self.set_part(rels_part, Some(before), Some(Arc::from(after)));
        Ok(())
    }

    fn pivot_content_type_removed(
        &mut self,
        workbook: &Workbook,
        removed: &BTreeSet<SmolStr>,
        cached: &PartBytes,
    ) -> Result<()> {
        let part = package::CONTENT_TYPES_PART;
        let before = self
            .part(workbook, part, cached)?
            .ok_or_else(|| Error::absent("content types", part))?;
        let root = Registration::root(&before)?;
        let mut changes = BTreeMap::new();
        for entry in root.children_named(&[package::CONTENT_TYPES_NAMESPACE], "Override")? {
            if entry
                .attribute(b"PartName")?
                .is_some_and(|name| removed.contains(name.trim_start_matches('/')))
            {
                changes.insert(entry.key, None);
            }
        }
        let after = Registration::replace_root(&before, &root.changed_children(&changes)?.xml)?;
        self.set_part(part.into(), Some(before), Some(Arc::from(after)));
        Ok(())
    }
}

impl Workbook {
    /// Create one real tabular pivot, using the same publication path as an
    /// edit's forward action. The initial writer admits a vertical SUM pivot.
    pub fn add_pivot(
        &mut self,
        spec: PivotSpec,
        sheet: &str,
        anchor: CellRef,
    ) -> Result<CellRange> {
        self.add_pivot_owned(spec, sheet, anchor).map(|(range, _)| range)
    }

    pub(crate) fn add_pivot_owned(
        &mut self,
        spec: PivotSpec,
        sheet: &str,
        anchor: CellRef,
    ) -> Result<(CellRange, Restore)> {
        let planned = self.plan_vertical_pivot(&spec, sheet, anchor, None)?;
        Ok(planned.commit(self))
    }

    /// Recompute the pivot from its current source, preserving part identities.
    pub fn refresh_pivot(&mut self, sheet: &str, name: &str) -> Result<CellRange> {
        self.refresh_pivot_owned(sheet, name).map(|(range, _)| range)
    }

    pub(crate) fn refresh_pivot_owned(&mut self, sheet: &str, name: &str) -> Result<(CellRange, Restore)> {
        let host = self.resolve(sheet).ok_or_else(|| self.absent(sheet))?;
        let sheet = self.slots[host].name.as_str();
        let old = self
            .pivots()?
            .iter()
            .find(|pivot| pivot.host_sheet() == sheet && pivot.name() == name)
            .cloned()
            .ok_or_else(|| Error::absent("pivot", name))?;
        let spec = old
            .spec()
            .ok_or_else(|| Error::Unsupported {
                operation: "refreshing an unrepresentable pivot",
                filesystem: format_smolstr!("{sheet}#{name}"),
            })?
            .clone();
        let planned = self.plan_vertical_pivot(&spec, sheet, old.location().start(), Some(&old))?;
        Ok(planned.commit(self))
    }

    /// Change one editable pivot specification while preserving its package identity.
    pub fn update_pivot(&mut self, sheet: &str, name: &str, spec: PivotSpec) -> Result<CellRange> {
        self.update_pivot_owned(sheet, name, spec).map(|(range, _)| range)
    }

    pub(crate) fn update_pivot_owned(
        &mut self, sheet: &str, name: &str, spec: PivotSpec,
    ) -> Result<(CellRange, Restore)> {
        let host = self.resolve(sheet).ok_or_else(|| self.absent(sheet))?;
        let sheet = self.slots[host].name.as_str();
        let old = self.pivots()?.iter()
            .find(|pivot| pivot.host_sheet() == sheet && pivot.name() == name)
            .cloned()
            .ok_or_else(|| Error::absent("pivot", name))?;
        if !old.editable() {
            return Err(Error::Unsupported {
                operation: "updating an unrepresentable pivot",
                filesystem: format_smolstr!("{sheet}#{name}"),
            });
        }
        let planned = self.plan_vertical_pivot(&spec, sheet, old.location().start(), Some(&old))?;
        Ok(planned.commit(self))
    }

    fn plan_vertical_pivot(
        &self,
        spec: &PivotSpec,
        sheet: &str,
        anchor: CellRef,
        old: Option<&PivotTable>,
    ) -> Result<PivotPublication> {
        let host = self.resolve(sheet).ok_or_else(|| self.absent(sheet))?;
        let sheet = self.slots[host].name.as_str();
        if self.slots[host].kind != SheetKind::Worksheet {
            return Err(self.not_a_worksheet(host));
        }
        if self.pivots()?.iter().any(|pivot| {
            pivot.name().eq_ignore_ascii_case(&spec.name)
                && old.is_none_or(|old| old.table_part != pivot.table_part)
        }) {
            return Err(Error::Conflict {
                expected: "a distinct pivot name",
                actual: "an existing pivot of that name",
                path: spec.name.clone(),
            });
        }
        let source = self.sheet(&spec.source.sheet)?;
        // Resolve an input alias once. Only that uncommon spelling needs an
        // owned request; every calculation and the serialized source agree.
        let resolved;
        let spec = if spec.source.sheet.as_str() == source.name() {
            spec
        } else {
            resolved = PivotSpec {
                source: PivotSource { sheet: source.name().into(), range: spec.source.range },
                ..spec.clone()
            };
            &resolved
        };
        let bound = super::pivot::compute::BoundSource::bind(spec, source)?;
        // One leaf on each axis is a lower bound on every rendered pivot.
        // Refuse an impossible landing before walking any source record.
        super::pivot::layout::geometry(spec, anchor, 1, 1)?;
        let computed = super::pivot::compute::PivotComputed::build(spec, &bound, source)?;
        let display = super::pivot::layout::PivotDisplay::new(spec, &computed)?;
        let rows = u32::try_from(display.row_events.len())
            .map_err(|_| Error::InvalidRecord {
                path: "$.pivot.rows".into(),
                reason: "expected a bounded item count".into(),
            })?
;
        let columns = u32::try_from(display.column_events.len()).map_err(|_| Error::InvalidRecord {
            path: "$.pivot.columns".into(),
            reason: "expected a bounded item count".into(),
        })?;
        let geometry = super::pivot::layout::geometry(spec, anchor, rows, columns)?;
        if spec.source.sheet == sheet && spec.source.range.intersects(geometry.range) {
            return Err(Error::Conflict {
                expected: "a pivot output outside its source rectangle",
                actual: "overlapping source and output",
                path: format_smolstr!("{sheet}!{}", geometry.range),
            });
        }
        if let Some(old) = old {
            if self.pivots()?.iter().any(|pivot| {
                pivot.table_part != old.table_part && pivot.cache_id() == old.cache_id()
            }) {
                return Err(Error::Unsupported {
                    operation: "refreshing a pivot whose cache is shared in this writer phase",
                    filesystem: old.cache_part.clone(),
                });
            }
        }
        let captions = if let Some(old) = old {
            let table = Registration::root_named(
                &self.part_bytes(&old.table_part)?, "pivotTableDefinition", &old.table_part,
            )?;
            super::pivot::part::PivotCaptions::new(
                table.root_attribute(b"dataCaption")?,
                table.root_attribute(b"grandTotalCaption")?,
            )?
        } else {
            super::pivot::part::PivotCaptions::new(None, None)?
        };
        let destination = self.sheet(sheet)?;
        let mut cells = super::pivot::part::display_cells(
            spec, &bound, &computed, &display, &captions, geometry, self.system,
        )?;
        // Keep imported styles without losing exceptional date serials.
        if old.is_some_and(|old| matches!(old.origin(), super::pivot::PivotOrigin::Read { editable: true, .. })) {
            for cell in &mut cells.cells {
                if let Some(before) = destination.cell(cell.reference()) {
                    cell.set_style(before.style());
                }
            }
        }
        let previous = old.map(PivotTable::location);
        for cell in destination.cells_in(geometry.range) {
            if previous.is_none_or(|range| !range.contains(cell.reference())) {
                return Err(Error::Conflict {
                    expected: "empty pivot output cells",
                    actual: "an occupied output cell",
                    path: format_smolstr!("{sheet}!{}", cell.reference()),
                });
            }
        }
        if destination
            .layout()
            .merges
            .iter()
            .any(|range| range.intersects(geometry.range))
            || self.pivots()?.iter().any(|pivot| {
                old.is_none_or(|old| old.table_part != pivot.table_part)
                    && pivot.host_sheet() == sheet
                    && pivot.location().intersects(geometry.range)
            })
        {
            return Err(Error::Conflict {
                expected: "unowned pivot output rectangle",
                actual: "a merged or other pivot rectangle",
                path: format_smolstr!("{sheet}!{}", geometry.range),
            });
        }
        // The source/display loops never parse or intern a format. Resolve
        // each distinct requested code once, cloning styles only on append.
        let mut styles = if spec.values.iter().any(|value| value.number_format.is_some()) {
            Some(self.styles()?)
        } else {
            None
        };
        let mut value_styles: SmallVec<[Option<StyleId>; 2]> = SmallVec::new();
        let mut formats: SmallVec<[Option<u32>; 2]> = SmallVec::new();
        for (index, value) in spec.values.iter().enumerate() {
            let Some(code) = &value.number_format else {
                value_styles.push(None);
                formats.push(None);
                continue;
            };
            if let Some(previous) = spec.values[..index].iter()
                .position(|value| value.number_format.as_ref() == Some(code))
            {
                value_styles.push(value_styles[previous]);
                formats.push(formats[previous]);
                continue;
            }
            let path = format_smolstr!("$.values[{index}].numberFormat");
            let parsed = FormatCode::from_code(code).map_err(|error| Error::InvalidRecord {
                path: path.clone(),
                reason: format_smolstr!("expected an en-US Excel number format, got {code:?}: {error}"),
            })?;
            if parsed.is_localized() {
                return Err(Error::Unsupported {
                    operation: "publishing a localized pivot number format",
                    filesystem: path,
                });
            }
            let table = styles.as_mut().expect("requested format owns a style table");
            let wanted = CellStyle {
                number_format: if parsed.is_general() { SmolStr::new_static("General") } else { code.clone() },
                ..CellStyle::default()
            };
            let id = match table.find(&wanted) {
                Some(id) => id,
                None => {
                    table.check_append()?;
                    Arc::make_mut(table).intern_format(parsed)?
                }
            };
            value_styles.push(Some(id));
            formats.push(table.format_id(id));
        }
        if let Some(table) = &styles {
            let data_row = geometry.range.start().row() + geometry.header_rows;
            let data_col = geometry.range.start().column() + spec.rows.len() as u32;
            for cell in &mut cells.cells {
                let at = cell.reference();
                if at.row() >= data_row && at.column() >= data_col {
                    let value = (at.column() - data_col) as usize % spec.values.len();
                    if let Some(style) = value_styles[value] {
                        let raw = cell.restyle(style, table.number_format(style), self.system, None);
                        if let Some(bits) = raw.and_then(|raw| super::sheet::CellExtra::exceptional_serial(cell, raw, self.system)) {
                            cells.serials.push((at, bits));
                        }
                    }
                }
            }
        }
        let mut plan = Plan::default();
        let cached = PartBytes::new();
        let table_part = match old {
            Some(old) => old.table_part.clone(),
            None => self.next_part("xl/pivotTables/pivotTable", ".xml", std::iter::empty())?,
        };
        let cache_part = match old {
            Some(old) => old.cache_part.clone(),
            None => self.next_part(
                "xl/pivotCache/pivotCacheDefinition",
                ".xml",
                std::iter::empty(),
            )?,
        };
        let records_part = match old {
            Some(_) => {
                let rels = package::relationships_part_of(&cache_part);
                let bytes = self.part_bytes(&rels)?;
                Relationships::from_xml(&bytes, &cache_part)?
                    .first_of(RelationshipKind::PivotCacheRecords)
                    .and_then(|link| link.target.clone())
                    .ok_or_else(|| Error::InvalidRecord {
                        path: rels,
                        reason: "expected owned cache records".into(),
                    })?
            }
            None => self.next_part(
                "xl/pivotCache/pivotCacheRecords",
                ".xml",
                std::iter::empty(),
            )?,
        };
        let cache_id = match old {
            Some(old) => old.cache_id(),
            None => {
                let _ = self.pivots()?;
                self.stated.pivots.get().and_then(|inventory| inventory.max_cache_id)
                    .unwrap_or(0).checked_add(1)
            }
                .ok_or_else(|| Error::InvalidRecord {
                    path: "$.pivot.cacheId".into(),
                    reason: "expected a free cache ID".into(),
                })?,
        };
        let mut rendered = super::pivot::part::render(spec, &bound, &computed, &display, geometry, cache_id, &captions, &formats)?;
        if let Some(old) = old {
            if matches!(old.origin(), super::pivot::PivotOrigin::Read { editable: true, .. }) {
                rendered.table = self.preserve_imported_pivot_part(
                    &table_part,
                    rendered.table,
                    "pivotTableDefinition",
                    &["location", "pivotFields", "rowFields", "rowItems", "colItems", "dataFields"],
                    &["name", "rowGrandTotals", "colGrandTotals"],
                )?;
                rendered.cache = self.preserve_imported_pivot_part(
                    &cache_part,
                    rendered.cache,
                    "pivotCacheDefinition",
                    &["cacheSource", "cacheFields"],
                    &["saveData", "refreshOnLoad", "recordCount"],
                )?;
            }
        }
        for (member, bytes) in [
            (&table_part, rendered.table),
            (&cache_part, rendered.cache),
            (&records_part, rendered.records),
        ] {
            let before = plan.part(self, member, &cached)?;
            plan.set_part(member.clone(), before, Some(Arc::from(bytes)));
        }
        if old.is_none() {
            plan.pivot_link(
                self,
                &self.slots[host].part,
                "pivotTable",
                &table_part,
                &cached,
            )?;
            plan.pivot_link(
                self,
                &table_part,
                "pivotCacheDefinition",
                &cache_part,
                &cached,
            )?;
            let cache_record_id = plan.pivot_link(
                self,
                &cache_part,
                "pivotCacheRecords",
                &records_part,
                &cached,
            )?;
            if cache_record_id != "rId1" {
                return Err(Error::InvalidRecord {
                    path: cache_part.clone(),
                    reason: "expected the new cache records relationship rId1".into(),
                });
            }
            let workbook_rid = plan.pivot_link(
                self,
                &self.workbook_part,
                "pivotCacheDefinition",
                &cache_part,
                &cached,
            )?;
            let before = plan
                .part(self, &self.workbook_part, &cached)?
                .ok_or_else(|| Error::absent("workbook part", &self.workbook_part))?;
            let item = Registration::root(format!("<pivotCache xmlns=\"{}\" xmlns:r=\"{}\" cacheId=\"{cache_id}\" r:id=\"{workbook_rid}\"/>", self.stated.family.namespace(), self.stated.family.relationships_namespace()).as_bytes())?;
            let after = Registration::merge(
                &before,
                &self.workbook_part,
                b"pivotCache",
                b"cacheId",
                b"pivotCaches",
                &[item],
            )?
            .ok_or_else(|| Error::InvalidRecord {
                path: self.workbook_part.clone(),
                reason: "expected a newly inserted pivot cache".into(),
            })?;
            plan.set_part(self.workbook_part.clone(), Some(before), Some(after));
            for (member, kind) in [
                (
                    &table_part,
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml",
                ),
                (
                    &cache_part,
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml",
                ),
                (
                    &records_part,
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml",
                ),
            ] {
                plan.ensure_content_type(self, member, kind, &cached)?;
            }
        }
        Ok(PivotPublication {
            host,
            geometry,
            previous,
            cells: cells.cells,
            serials: cells.serials,
            styles,
            plan,
        })
    }

    /// Remove one representable pivot and its last-owner cache in one
    /// package/cell transaction. Shared or opaque caches are a named refusal
    /// in this first vertical slice.
    pub fn remove_pivot(&mut self, sheet: &str, name: &str) -> Result<()> {
        self.remove_pivot_owned(sheet, name).map(|_| ())
    }

    pub(crate) fn remove_pivot_owned(
        &mut self,
        sheet: &str,
        name: &str,
    ) -> Result<(CellRange, Restore)> {
        let host = self.resolve(sheet).ok_or_else(|| self.absent(sheet))?;
        let sheet = self.slots[host].name.as_str();
        let pivots = self.pivots()?;
        let selected = pivots
            .iter()
            .find(|pivot| pivot.host_sheet() == sheet && pivot.name() == name)
            .cloned()
            .ok_or_else(|| Error::absent("pivot", name))?;
        if !selected.editable() {
            return Err(Error::Unsupported {
                operation: "removing an unrepresentable pivot",
                filesystem: format_smolstr!("{sheet}#{name}"),
            });
        }
        if pivots.iter().any(|pivot| {
            pivot.table_part != selected.table_part && pivot.cache_id() == selected.cache_id()
        }) {
            return Err(Error::Unsupported {
                operation: "removing a pivot whose cache is shared in this writer phase",
                filesystem: selected.cache_part.clone(),
            });
        }
        self.check_caches(host)?;
        let cached = PartBytes::new();
        let mut plan = Plan::default();
        let table_rels = package::relationships_part_of(&selected.table_part);
        let cache_rels = package::relationships_part_of(&selected.cache_part);
        let cache_rels_bytes = self.part_bytes(&cache_rels)?;
        let records = Relationships::from_xml(&cache_rels_bytes, &selected.cache_part)?
            .first_of(RelationshipKind::PivotCacheRecords)
            .and_then(|link| link.target.clone())
            .ok_or_else(|| Error::InvalidRecord {
                path: cache_rels.clone(),
                reason: "expected one owned cache records part".into(),
            })?;
        for other in pivots.iter().filter(|other| other.cache_part != selected.cache_part) {
            let relationships = package::relationships_part_of(&other.cache_part);
            let bytes = self.part_bytes(&relationships)?;
            let owner = Relationships::from_xml(&bytes, &other.cache_part)?;
            if owner
                .first_of(RelationshipKind::PivotCacheRecords)
                .and_then(|link| link.target.as_deref())
                == Some(records.as_str())
            {
                return Err(Error::Unsupported {
                    operation: "removing pivot records shared by another cache",
                    filesystem: records.clone(),
                });
            }
        }
        plan.pivot_unlink(self, &self.slots[host].part, "pivotTable", &selected.table_part, &cached)?;
        plan.pivot_unlink(self, &self.workbook_part, "pivotCacheDefinition", &selected.cache_part, &cached)?;
        let before = plan
            .part(self, &self.workbook_part, &cached)?
            .ok_or_else(|| Error::absent("workbook part", &self.workbook_part))?;
        let root = Registration::root(&before)?;
        let caches = root.one_child(
            &[super::NAMESPACE, super::STRICT_NAMESPACE],
            "pivotCaches",
            &self.workbook_part,
        )?;
        let mut changes = BTreeMap::new();
        for child in caches.children_named(
            &[super::NAMESPACE, super::STRICT_NAMESPACE],
            "pivotCache",
        )? {
            if child.root_attribute(b"cacheId")?.as_deref()
                == Some(&selected.cache_id().to_string())
            {
                changes.insert(child.key, None);
            }
        }
        if changes.len() != 1 {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("{}#pivotCaches", self.workbook_part),
                reason: "expected one selected cache registration".into(),
            });
        }
        let remaining = caches.changed_children(&changes)?;
        let mut root_changes = BTreeMap::new();
        root_changes.insert(
            caches.key,
            if remaining.has_children()? {
                Some(remaining)
            } else {
                None
            },
        );
        let after = Registration::replace_root(&before, &root.changed_children(&root_changes)?.xml)?;
        plan.set_part(self.workbook_part.clone(), Some(before), Some(Arc::from(after)));
        let removed = BTreeSet::from([
            selected.table_part.clone(),
            table_rels,
            selected.cache_part.clone(),
            cache_rels,
            records,
        ]);
        plan.pivot_content_type_removed(self, &removed, &cached)?;
        for member in &removed {
            let before = plan.part(self, member, &cached)?;
            if before.is_some() {
                plan.set_part(member.clone(), before, None);
            }
        }

        let mut restore = Restore::new(self);
        restore.push(self.cells_step(sheet, &[selected.location()])?);
        restore.capture_styles(self.styles()?.as_ref());
        let mut parts = Vec::with_capacity(plan.overrides.len());
        for changed in &plan.overrides {
            parts.push(PartRestore::new(
                changed.member.clone(),
                changed.before.clone(),
                changed.after.clone(),
            ));
        }
        restore.push(Step::Overrides(parts));
        // Every fallible package and cell read ended above. This is the same
        // Attempt and the same inverse owner as creation/refresh.
        let mark = self.begin_batch();
        self.stated.remember_slot(&self.slots[host]);
        let target = self.slots[host].parsed.get_mut().expect("preflighted worksheet");
        let cells: Vec<_> = target.cells_in(selected.location()).map(Cell::reference).collect();
        for at in cells {
            target.remove_cell(at);
        }
        for changed in plan.overrides {
            self.stated.set_part(changed.member, changed.after);
        }
        self.invalidate_calculation();
        self.finish_batch(mark.start());
        Ok((selected.location(), restore))
    }
}

// One narrow Excel-saved extension set whose selected bytes can be carried
// unchanged when the calculated children are replaced. Unknown extension
// semantics remain read-only, even if their markup is syntactically valid.
impl Workbook {
    fn native_vertical_extensions(
        table: &Registration,
        cache: &Registration,
    ) -> Result<bool> {
        const XPDL: &str = "http://schemas.microsoft.com/office/spreadsheetml/2016/pivotdefaultlayout";
        const TABLE_XPDL: &str = "{747A6164-185A-40DC-8AA5-F01512510D54}";
        const CACHE_X14: &str = "{725AE2AE-9491-48be-B2B4-4EB974FC3084}";
        let main = &[super::NAMESPACE, super::STRICT_NAMESPACE];
        let tables = table.children_named(main, "extLst")?;
        let caches = cache.children_named(main, "extLst")?;
        // colFields is present for a real column axis or the values axis.
        // Its contents and uniqueness are checked by the typed spec intake;
        // foreign same-local-name children still fail this exact count.
        let table_children = 7 + table.children_named(main, "colFields")?.len();
        if tables.is_empty() && caches.is_empty() {
            return Ok(table.direct_child_count()? == table_children && cache.direct_child_count()? == 2);
        }
        let [table_ext] = tables.as_slice() else { return Ok(false); };
        let cache_ext = match caches.as_slice() {
            [] => None,
            [entry] => Some(entry),
            _ => return Ok(false),
        };
        if table.direct_child_count()? != table_children + 1
            || cache.direct_child_count()? != if cache_ext.is_some() { 3 } else { 2 }
            || !table_ext.container_semantics()?.is_empty()
        {
            return Ok(false);
        }
        let table_entries = table_ext.children_named(main, "ext")?;
        let count = if cache_ext.is_some() { 2 } else { 1 };
        if table_ext.direct_child_count()? != count || table_entries.len() != count {
            return Ok(false);
        }
        let mut x14 = false;
        let mut xpdl = false;
        for entry in table_entries {
            let uri = entry.root_attribute(b"uri")?;
            if entry.container_semantics()?.len() != 1 {
                return Ok(false);
            }
            if uri.as_deref() == Some(super::pivot::part::HIDE_VALUES_URI) {
                let child = entry.children_named(&[super::pivot::part::HIDE_VALUES_NAMESPACE], "pivotTableDefinition")?;
                if child.len() != 1 || entry.direct_child_count()? != 1
                    || child[0].has_children()?
                    || child[0].root_attribute(b"hideValuesRow")?.as_deref() != Some("1")
                    || child[0].container_semantics()?.len() != 1
                {
                    return Ok(false);
                }
                x14 = true;
            } else if uri.as_deref() == Some(TABLE_XPDL) {
                let child = entry.children_named(&[XPDL], "pivotTableDefinition16")?;
                if child.len() != 1 || entry.direct_child_count()? != 1 || child[0].has_children()?
                    || !child[0].container_semantics()?.is_empty() {
                    return Ok(false);
                }
                xpdl = true;
            } else {
                return Ok(false);
            }
        }
        // Our no-column writer owns exactly this single x14 display hint.
        if cache_ext.is_none() {
            return Ok(x14 && !xpdl
                && table.root_attribute(b"gridDropZones")?.as_deref() == Some("0"));
        }
        let cache_ext = cache_ext.unwrap();
        if !cache_ext.container_semantics()?.is_empty()
            || cache_ext.direct_child_count()? != 1 { return Ok(false); }
        let cache_entries = cache_ext.children_named(main, "ext")?;
        if cache_entries.len() != 1 { return Ok(false); }
        let cache_entry = &cache_entries[0];
        let cache_child = cache_entry.children_named(&[super::pivot::part::HIDE_VALUES_NAMESPACE], "pivotCacheDefinition")?;
        Ok(x14 && xpdl
            && cache_entry.root_attribute(b"uri")?.as_deref() == Some(CACHE_X14)
            && cache_entry.container_semantics()?.len() == 1
            && cache_entry.direct_child_count()? == 1
            && cache_child.len() == 1
            && !cache_child[0].has_children()?
            && cache_child[0].container_semantics()?.is_empty())
    }
}

// Keep the imported root's passive Excel metadata and its proved extensions.
// Registration performs all scoped XML selection and child replacement.
impl Workbook {
    fn preserve_imported_pivot_part(
        &self,
        member: &str,
        rendered: Vec<u8>,
        root: &str,
        children: &[&str],
        attributes: &[&str],
    ) -> Result<Vec<u8>> {
        let main = &[super::NAMESPACE, super::STRICT_NAMESPACE];
        let mut old = Registration::root_named(&self.part_bytes(member)?, root, member)?;
        let new = Registration::root_named(&rendered, root, member)?;
        if root == "pivotTableDefinition" {
            // A column/values axis may be added or removed by update. Keep
            // the optional child before colItems using the shared XML owner.
            let mut before = old.children_named(main, "colFields")?;
            let mut after = new.children_named(main, "colFields")?;
            if before.len() > 1 || after.len() > 1 {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("{member}#colFields"),
                    reason: format_smolstr!("expected at most one colFields, got {}/{}", before.len(), after.len()),
                });
            }
            if let Some(before) = before.pop() {
                old = old.changed_children(&BTreeMap::from([(before.key, after.pop())]))?;
            } else if let Some(after) = after.pop() {
                let next = old.one_child(main, "colItems", member)?;
                old = old.inserted_before(&[after], &next.key)?;
            }
        }
        let mut changes = BTreeMap::new();
        for name in children {
            let before = old.one_child(main, name, member)?;
            let after = new.one_child(main, name, member)?;
            changes.insert(before.key.clone(), Some(after));
        }
        let attributes = attributes
            .iter()
            .map(|name| {
                Ok((
                    SmolStr::new(*name),
                    new.root_attribute(name.as_bytes())?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(old.changed_children(&changes)?.with_attributes(attributes)?.xml.as_bytes().to_vec())
    }
}
