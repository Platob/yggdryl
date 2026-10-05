//! Named and suggested record regions discovered from worksheet source facts.

use smol_str::SmolStr;

use super::{CellRange, Workbook};
use crate::{Error, Result};

/// Whether a region is stated by an OOXML table or suggested by occupied cells.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExcelRegionKind {
    /// A registered table, with its exact workbook-wide display name.
    Table {
        /// The decoded OOXML table display name.
        name: SmolStr,
    },
    /// Bounds of one connected component, without an inferred header.
    /// The rectangle can contain holes or enclose an excluded named table;
    /// discovery does not prove that every cell belongs to one record table.
    Suggested,
}

/// One exact worksheet rectangle returned by workbook region discovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExcelRegion {
    /// The worksheet name as stored in the workbook.
    pub sheet: SmolStr,
    /// Inclusive source coordinates, including a named table's header/totals.
    pub range: CellRange,
    /// The source authority for this rectangle.
    pub kind: ExcelRegionKind,
}

/// A result keeps tab order until all rectangles can be sorted once.
pub(super) struct Found {
    tab: usize,
    pub(super) region: ExcelRegion,
    contacts: Vec<usize>,
}

const MAX_REGIONS: usize = 1024;

fn append(
    found: &mut Vec<Found>,
    tab: usize,
    sheet: &str,
    range: CellRange,
    kind: ExcelRegionKind,
    contacts: Vec<usize>,
) -> Result<()> {
    if found.len() == MAX_REGIONS {
        return Err(Error::InvalidRecord {
            path: smol_str::format_smolstr!("{sheet}!{}", range.start()),
            reason: smol_str::format_smolstr!(
                "expected at most {MAX_REGIONS} regions, got another region at {range}"
            ),
        });
    }
    found.push(Found {
        tab,
        region: ExcelRegion {
            sheet: sheet.into(),
            range,
            kind,
        },
        contacts,
    });
    Ok(())
}

#[derive(Clone, Copy)]
struct Bounds {
    first_row: u32,
    last_row: u32,
    first_column: u32,
    last_column: u32,
}

impl Bounds {
    fn row(row: u32, first_column: u32, last_column: u32) -> Self {
        Self {
            first_row: row,
            last_row: row,
            first_column,
            last_column,
        }
    }

    fn include(&mut self, other: Self) {
        self.first_row = self.first_row.min(other.first_row);
        self.last_row = self.last_row.max(other.last_row);
        self.first_column = self.first_column.min(other.first_column);
        self.last_column = self.last_column.max(other.last_column);
    }

    fn range(self) -> CellRange {
        CellRange::new(
            super::CellRef::new(self.first_row, self.first_column),
            super::CellRef::new(self.last_row, self.last_column),
        )
    }
}

struct Node {
    parent: usize,
    rank: u8,
    bounds: Bounds,
    contacts: Vec<usize>,
}

struct Run {
    first: u32,
    last: u32,
    component: usize,
}

/// Only components touching the previous row survive. Each row compacts
/// their identities, so neither the union tree nor its scratch grows with
/// worksheet height. Both run lists and both node lists are width-bounded.
#[derive(Default)]
struct Frontier {
    row: Option<u32>,
    previous: Vec<Run>,
    current: Vec<Run>,
    nodes: Vec<Node>,
    compact: Vec<Node>,
    mapping: Vec<usize>,
}

impl Frontier {
    fn root(&mut self, mut index: usize) -> usize {
        let mut root = index;
        while self.nodes[root].parent != root {
            root = self.nodes[root].parent;
        }
        while self.nodes[index].parent != index {
            let parent = self.nodes[index].parent;
            self.nodes[index].parent = root;
            index = parent;
        }
        root
    }

    fn join(&mut self, first: usize, second: usize) -> usize {
        let mut first = self.root(first);
        let mut second = self.root(second);
        if first == second {
            return first;
        }
        if self.nodes[first].rank < self.nodes[second].rank {
            std::mem::swap(&mut first, &mut second);
        }
        let bounds = self.nodes[second].bounds;
        self.nodes[first].bounds.include(bounds);
        for contact in std::mem::take(&mut self.nodes[second].contacts) {
            if !self.nodes[first].contacts.contains(&contact) {
                self.nodes[first].contacts.push(contact);
            }
        }
        self.nodes[second].parent = first;
        if self.nodes[first].rank == self.nodes[second].rank {
            self.nodes[first].rank += 1;
        }
        first
    }

    fn finish(&mut self, found: &mut Vec<Found>, tab: usize, sheet: &str) -> Result<()> {
        // After each row, nodes are the compact live roots only.
        for node in self.nodes.drain(..) {
            append(
                found,
                tab,
                sheet,
                node.bounds.range(),
                ExcelRegionKind::Suggested,
                node.contacts,
            )?;
        }
        self.nodes.clear();
        self.previous.clear();
        self.row = None;
        Ok(())
    }

    fn consume(
        &mut self,
        row: u32,
        columns: &[u32],
        contacts: impl Iterator<Item = (usize, CellRange)>,
        found: &mut Vec<Found>,
        tab: usize,
        sheet: &str,
    ) -> Result<()> {
        if self.row.is_some_and(|previous| previous + 1 != row) {
            self.finish(found, tab, sheet)?;
        }
        self.current.clear();
        let mut at = 0;
        let mut previous = 0;
        while at < columns.len() {
            let first = columns[at];
            let mut last = first;
            at += 1;
            while at < columns.len() && columns[at] == last + 1 {
                last = columns[at];
                at += 1;
            }
            let mut component = self.nodes.len();
            self.nodes.push(Node {
                parent: component,
                rank: 0,
                bounds: Bounds::row(row, first, last),
                contacts: Vec::new(),
            });
            while previous < self.previous.len() && self.previous[previous].last + 1 < first {
                previous += 1;
            }
            let mut linked = previous;
            while linked < self.previous.len() && self.previous[linked].first <= last + 1 {
                let other = self.previous[linked].component;
                component = self.join(component, other);
                linked += 1;
            }
            self.current.push(Run {
                first,
                last,
                component,
            });
        }
        // Merges are metadata, but contact is proved against occupied runs,
        // never against a component's possibly hollow bounding rectangle.
        for (contact, merge) in contacts {
            let first = merge.start().column().saturating_sub(1);
            let last = merge.end().column().saturating_add(1);
            let mut at = self.current.partition_point(|run| run.last < first);
            while at < self.current.len() && self.current[at].first <= last {
                let root = self.root(self.current[at].component);
                if !self.nodes[root].contacts.contains(&contact) {
                    self.nodes[root].contacts.push(contact);
                }
                at += 1;
            }
        }
        self.mapping.clear();
        self.mapping.resize(self.nodes.len(), usize::MAX);
        self.compact.clear();
        for index in 0..self.current.len() {
            let root = self.root(self.current[index].component);
            if self.mapping[root] == usize::MAX {
                self.mapping[root] = self.compact.len();
                self.compact.push(Node {
                    parent: self.compact.len(),
                    rank: 0,
                    bounds: self.nodes[root].bounds,
                    contacts: std::mem::take(&mut self.nodes[root].contacts),
                });
            }
            self.current[index].component = self.mapping[root];
        }
        for (index, node) in self.nodes.iter_mut().enumerate() {
            if node.parent == index && self.mapping[index] == usize::MAX {
                append(
                    found,
                    tab,
                    sheet,
                    node.bounds.range(),
                    ExcelRegionKind::Suggested,
                    std::mem::take(&mut node.contacts),
                )?;
            }
        }
        std::mem::swap(&mut self.previous, &mut self.current);
        std::mem::swap(&mut self.nodes, &mut self.compact);
        self.row = Some(row);
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct MergeEvent {
    row: u32,
    contact: usize,
    entering: bool,
}

/// Metadata-only row sweep. An active merge is tested against sparse occupied
/// runs by binary search, so cells outside headers add no per-merge work.
struct MergeSweep {
    merges: Vec<CellRange>,
    events: Vec<MergeEvent>,
    next: usize,
    active: std::collections::BTreeSet<usize>,
}

impl MergeSweep {
    fn new(merges: Vec<CellRange>) -> Self {
        let mut events = Vec::new();
        for (contact, merge) in merges.iter().enumerate() {
            events.push(MergeEvent {
                row: merge.start().row().saturating_sub(1),
                contact,
                entering: true,
            });
            events.push(MergeEvent {
                row: merge.end().row().saturating_add(2),
                contact,
                entering: false,
            });
        }
        events.sort_by_key(|event| (event.row, event.entering, event.contact));
        Self {
            merges,
            events,
            next: 0,
            active: std::collections::BTreeSet::new(),
        }
    }

    fn advance(&mut self, row: u32) {
        while self.next < self.events.len() && self.events[self.next].row <= row {
            let event = self.events[self.next];
            if event.entering {
                self.active.insert(event.contact);
            } else {
                self.active.remove(&event.contact);
            }
            self.next += 1;
        }
    }

    fn active(&self) -> impl Iterator<Item = (usize, CellRange)> + '_ {
        self.active
            .iter()
            .copied()
            .map(|contact| (contact, self.merges[contact]))
    }
}

#[derive(Clone, Copy)]
struct ExclusionEvent {
    row: u32,
    columns: (u32, u32),
    entering: bool,
}

/// Table extents are resolved once into row events; a row scans its active
/// column intervals once beside its cells, never every table per cell.
struct Exclusions {
    events: Vec<ExclusionEvent>,
    next: usize,
    active: std::collections::BTreeMap<(u32, u32), u32>,
}

impl Exclusions {
    fn new(ranges: impl Iterator<Item = CellRange>) -> Self {
        let mut events = Vec::new();
        for range in ranges {
            let columns = (range.start().column(), range.end().column());
            events.push(ExclusionEvent {
                row: range.start().row(),
                columns,
                entering: true,
            });
            events.push(ExclusionEvent {
                row: range.end().row() + 1,
                columns,
                entering: false,
            });
        }
        events.sort_by_key(|event| (event.row, event.entering, event.columns));
        Self {
            events,
            next: 0,
            active: std::collections::BTreeMap::new(),
        }
    }

    fn advance(&mut self, row: u32) {
        while self.next < self.events.len() && self.events[self.next].row <= row {
            let event = self.events[self.next];
            if event.entering {
                *self.active.entry(event.columns).or_default() += 1;
            } else {
                let count = self
                    .active
                    .get_mut(&event.columns)
                    .expect("an exit follows its entry event");
                *count -= 1;
                if *count == 0 {
                    self.active.remove(&event.columns);
                }
            }
            self.next += 1;
        }
    }
}

/// The bounding box of all actual occupied candidates. Styled empty cells
/// never widen an implicit selection or turn a full-width title into data.
pub(super) fn occupied_extent(candidates: &[Found]) -> Option<CellRange> {
    let first = candidates.first()?;
    let mut bounds = Bounds {
        first_row: first.region.range.start().row(),
        last_row: first.region.range.end().row(),
        first_column: first.region.range.start().column(),
        last_column: first.region.range.end().column(),
    };
    for candidate in &candidates[1..] {
        let range = candidate.region.range;
        bounds.include(Bounds {
            first_row: range.start().row(),
            last_row: range.end().row(),
            first_column: range.start().column(),
            last_column: range.end().column(),
        });
    }
    Some(bounds.range())
}

/// Suggested components may join through an actual occupied-run contact with
/// the same selected header merge. A named table is always separate authority.
pub(super) fn require_single(
    candidates: &[Found],
    header: Option<CellRange>,
    merges: &[CellRange],
) -> Result<()> {
    if candidates.len() <= 1 {
        return Ok(());
    }
    if let Some(header) = header.filter(|_| {
        candidates
            .iter()
            .all(|candidate| matches!(&candidate.region.kind, ExcelRegionKind::Suggested))
    }) {
        let mut parent: Vec<usize> = (0..candidates.len()).collect();
        let mut first_by_merge = vec![None; merges.len()];
        let mut components = candidates.len();
        fn root(parent: &mut [usize], mut index: usize) -> usize {
            while parent[index] != index {
                let next = parent[index];
                parent[index] = parent[next];
                index = next;
            }
            index
        }
        for (index, candidate) in candidates.iter().enumerate() {
            for &contact in &candidate.contacts {
                let merge = merges[contact];
                if !header.contains(merge.start()) || !header.contains(merge.end()) {
                    continue;
                }
                if let Some(first) = first_by_merge[contact] {
                    let leader = root(&mut parent, first);
                    let other = root(&mut parent, index);
                    if leader != other {
                        parent[other] = leader;
                        components -= 1;
                        if components == 1 {
                            return Ok(());
                        }
                    }
                } else {
                    first_by_merge[contact] = Some(index);
                }
            }
        }
    }
    let names = candidates
        .iter()
        .take(3)
        .map(|candidate| match &candidate.region.kind {
            ExcelRegionKind::Table { name } => {
                format!("table {name:?} at {}", candidate.region.range)
            }
            ExcelRegionKind::Suggested => format!("region at {}", candidate.region.range),
        })
        .collect::<Vec<_>>()
        .join(", ");
    Err(Error::InvalidRecord {
        path: SmolStr::new_static("$.selection"),
        reason: smol_str::format_smolstr!(
            "ambiguous worksheet regions ({}) {names}; choose a table or range explicitly",
            candidates.len()
        ),
    })
}

/// Held and wire rows lend occupied columns to the same sparse Frontier.
pub(super) struct HeldRegions {
    sheet: SmolStr,
    found: Vec<Found>,
    frontier: Frontier,
    merges: MergeSweep,
}

impl HeldRegions {
    pub(super) fn new(sheet: impl Into<SmolStr>, merges: Vec<CellRange>) -> Self {
        Self {
            sheet: sheet.into(),
            found: Vec::new(),
            frontier: Frontier::default(),
            merges: MergeSweep::new(merges),
        }
    }

    pub(super) fn row(&mut self, index: u32, columns: &[u32]) -> Result<()> {
        self.merges.advance(index);
        self.frontier.consume(
            index,
            columns,
            self.merges.active(),
            &mut self.found,
            0,
            &self.sheet,
        )
    }

    pub(super) fn finish(mut self) -> Result<Vec<Found>> {
        self.frontier.finish(&mut self.found, 0, &self.sheet)?;
        Ok(self.found)
    }
}

fn occupied(
    workbook: &Workbook,
    strings: &mut Option<std::sync::Arc<super::shared_strings::SharedStrings>>,
    sheet: &str,
    row: u32,
    cell: &super::parser::RawCell,
) -> Result<bool> {
    // Formula ownership exists independently of its optional cached value.
    if cell.formula.is_some() {
        return Ok(true);
    }
    if !cell.has_content {
        return Ok(false);
    }
    if cell.kind == super::CellKind::SharedString {
        if strings.is_none() {
            *strings = Some(workbook.strings()?);
        }
        let content = cell.resolved_content(
            strings.as_ref().expect("loaded above"),
            sheet,
            super::CellRef::new(row, cell.column),
        )?;
        return Ok(!content.is_empty());
    }
    Ok(if cell.kind.is_text() {
        !cell.content.is_empty()
    } else {
        !cell.content.trim().is_empty()
    })
}

/// Consume one opened source workbook, never a live model API.
pub(crate) fn read(workbook: &Workbook, sheet: Option<&str>) -> Result<Vec<ExcelRegion>> {
    Ok(read_found(workbook, sheet, &[])?
        .into_iter()
        .map(|found| found.region)
        .collect())
}

pub(super) fn read_for_infer(
    workbook: &Workbook,
    sheet: &str,
    merges: &[CellRange],
) -> Result<Vec<Found>> {
    read_found(workbook, Some(sheet), merges)
}

fn read_found(
    workbook: &Workbook,
    sheet: Option<&str>,
    merges: &[CellRange],
) -> Result<Vec<Found>> {
    let names = workbook.sheet_names();
    let selected: Vec<_> = match sheet {
        Some(requested) => {
            let (tab, name) = names
                .iter()
                .enumerate()
                .find(|(_, name)| name.eq_ignore_ascii_case(requested))
                .filter(|(_, name)| workbook.sheet_kind(name) == Some(super::SheetKind::Worksheet))
                .ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new_static("$.sheet"),
                    reason: smol_str::format_smolstr!("expected a worksheet, got {requested:?}"),
                })?;
            vec![(tab, *name)]
        }
        None => names
            .iter()
            .enumerate()
            .filter_map(|(tab, name)| {
                (workbook.sheet_kind(name) == Some(super::SheetKind::Worksheet))
                    .then_some((tab, *name))
            })
            .collect(),
    };
    let mut found = Vec::new();
    workbook.visit_tables(sheet, |tab, name, table, _columns, _part, _bytes| {
        append(
            &mut found,
            tab,
            name,
            table.range,
            ExcelRegionKind::Table { name: table.name },
            Vec::new(),
        )
    })?;
    let mut strings = None;
    let mut columns = Vec::new();
    for (tab, name) in selected {
        let mut exclusions = Exclusions::new(found.iter().filter_map(|held| {
            (held.tab == tab && matches!(&held.region.kind, ExcelRegionKind::Table { .. }))
                .then_some(held.region.range)
        }));
        let mut frontier = Frontier::default();
        let mut contacts = MergeSweep::new(merges.to_vec());
        let mut rows = super::parser::SheetRows::new(
            std::io::BufReader::with_capacity(
                crate::DEFAULT_FETCH_BYTE_SIZE,
                workbook.sheet_reader(name)?,
            ),
            name,
        );
        while let Some(row) = rows.next() {
            let row = row?;
            columns.clear();
            exclusions.advance(row.index);
            let mut intervals = exclusions.active.keys().peekable();
            let mut covered: Option<u32> = None;
            for cell in &row.cells {
                while intervals
                    .peek()
                    .is_some_and(|interval| interval.0 <= cell.column)
                {
                    let interval = intervals.next().expect("peeked interval");
                    covered = Some(covered.map_or(interval.1, |end| end.max(interval.1)));
                }
                if covered.is_some_and(|end| cell.column <= end) {
                    continue;
                }
                if occupied(workbook, &mut strings, name, row.index, cell)? {
                    columns.push(cell.column);
                }
            }
            contacts.advance(row.index);
            frontier.consume(
                row.index,
                &columns,
                contacts.active(),
                &mut found,
                tab,
                name,
            )?;
            rows.recycle(row.cells);
        }
        frontier.finish(&mut found, tab, name)?;
    }
    // Equal keys expose identical regions; inference contacts travel with
    // each result and its connectivity calculation is order-independent.
    found.sort_unstable_by(|first, second| {
        let first_name = match &first.region.kind {
            ExcelRegionKind::Table { name } => Some(name.as_str()),
            ExcelRegionKind::Suggested => None,
        };
        let second_name = match &second.region.kind {
            ExcelRegionKind::Table { name } => Some(name.as_str()),
            ExcelRegionKind::Suggested => None,
        };
        first
            .tab
            .cmp(&second.tab)
            .then_with(|| first.region.range.cmp(&second.region.range))
            .then_with(|| first_name.cmp(&second_name))
    });
    Ok(found)
}
