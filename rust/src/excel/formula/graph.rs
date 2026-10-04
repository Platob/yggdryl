//! Sparse reverse dependencies over Excel's fixed grid.
//!
//! One registration owns its typed rectangle; canonical segment buckets
//! hold only registration IDs. Retained memberships are O(P log A), where
//! P is the peak number of live precedents and A is the indexed axis.
//! No rectangle is expanded into cells or duplicated across columns.

use std::collections::{BTreeSet, HashMap, VecDeque, btree_set};
use std::sync::atomic::{AtomicU64, Ordering};

use smol_str::format_smolstr;

use crate::excel::cell::{CellRange, CellRef};
use crate::excel::shift::Axis;
use crate::excel::workbook::SheetKey;
use crate::{Error, Result};

static INDEXES: AtomicU64 = AtomicU64::new(1);

/// A live registration's exclusive removal handle. It is deliberately not
/// Clone or Copy: successful removal returns its slot to its owner's free list.
#[derive(Debug)]
pub(crate) struct RegistrationId {
    owner: u64,
    slot: usize,
}

#[derive(Debug)]
struct Registration {
    sheet: SheetKey,
    range: CellRange,
    dependent: usize,
    active: bool,
    strict: bool,
}

#[derive(Debug)]
enum Slot {
    Live(Registration),
    Free(Option<usize>),
}

/// Reverse precedents for formula nodes. Every bucket exists only while
/// it holds IDs; slab/map capacities retain their peak allocation for reuse.
///
/// A point query visits one exact bucket, <=21 row buckets and <=15 column
/// buckets. It costs O(log A + C), where C is the IDs encountered before
/// orthogonal filtering. Each registration matches at most once; separate
/// registrations retain multiplicity, which Kahn must count consistently.
#[derive(Debug)]
pub(crate) struct DependencyIndex {
    owner: u64,
    slots: Vec<Slot>,
    free: Option<usize>,
    points: HashMap<(SheetKey, CellRef), BTreeSet<usize>>,
    segments: HashMap<(SheetKey, Axis, u32), BTreeSet<usize>>,
}

impl Default for DependencyIndex {
    fn default() -> Self {
        Self {
            // Exhaustion must refuse a new owner rather than alias a live index.
            owner: INDEXES
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                    next.checked_add(1)
                })
                .expect("dependency index identity exhausted"),
            slots: Vec::new(),
            free: None,
            points: HashMap::new(),
            segments: HashMap::new(),
        }
    }
}

impl DependencyIndex {
    /// Resolve one proven rectangle into point or segment membership.
    /// Full-height rectangles are column intervals; all others use rows.
    fn axis(range: CellRange) -> Option<Axis> {
        if range.start() == range.end() {
            None
        } else if range.start().row() == 0 && range.end().row() + 1 == Axis::Rows.limit() {
            Some(Axis::Columns)
        } else {
            Some(Axis::Rows)
        }
    }

    /// Add a resolved precedent. Both grid coordinates are checked before
    /// any slot or bucket changes; callers keep the returned removal handle.
    #[cfg(feature = "internals")]
    pub(crate) fn insert(
        &mut self,
        sheet: SheetKey,
        range: CellRange,
        dependent: usize,
    ) -> Result<RegistrationId> {
        Self::check(sheet, range)?;
        Ok(self.insert_checked(sheet, range, dependent, true))
    }

    fn check(sheet: SheetKey, range: CellRange) -> Result<()> {
        if !range.start().is_in_grid() || !range.end().is_in_grid() {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.sheets[{}].dependencies", sheet.as_u32()),
                reason: format_smolstr!(
                    "expected a dependency within {} rows and {} columns, got rows {}..={} columns {}..={}",
                    Axis::Rows.limit(),
                    Axis::Columns.limit(),
                    range.start().row(),
                    range.end().row(),
                    range.start().column(),
                    range.end().column()
                ),
            });
        }
        Ok(())
    }

    fn insert_checked(
        &mut self,
        sheet: SheetKey,
        range: CellRange,
        dependent: usize,
        strict: bool,
    ) -> RegistrationId {
        let registration = Registration {
            sheet,
            range,
            dependent,
            active: true,
            strict,
        };
        let id = match self.free {
            Some(id) => {
                let Slot::Free(next) = &self.slots[id] else {
                    unreachable!("the free list names only free slots")
                };
                self.free = *next;
                self.slots[id] = Slot::Live(registration);
                id
            }
            None => {
                let id = self.slots.len();
                self.slots.push(Slot::Live(registration));
                id
            }
        };
        match Self::axis(range) {
            None => {
                self.points
                    .entry((sheet, range.start()))
                    .or_default()
                    .insert(id);
            }
            Some(axis) => {
                for segment in Segments::new(axis, range) {
                    self.segments
                        .entry((sheet, axis, segment))
                        .or_default()
                        .insert(id);
                }
            }
        }
        RegistrationId {
            owner: self.owner,
            slot: id,
        }
    }

    /// Forget exactly this precedent, retaining slab capacity for the next
    /// changed formula. A foreign owner is refused before any slot is accessed.
    pub(crate) fn remove(&mut self, id: RegistrationId) -> Result<()> {
        if id.owner != self.owner {
            return Err(Error::Conflict {
                expected: "a registration from this dependency index",
                actual: "a registration from another dependency index",
                path: format_smolstr!("$.formula.dependencies[{}]", id.slot),
            });
        }
        let Slot::Live(Registration { sheet, range, .. }) =
            std::mem::replace(&mut self.slots[id.slot], Slot::Free(self.free))
        else {
            unreachable!("a registration's exclusive handle is removed once")
        };
        match Self::axis(range) {
            None => {
                let key = (sheet, range.start());
                let bucket = self
                    .points
                    .get_mut(&key)
                    .expect("the live point is indexed");
                assert!(bucket.remove(&id.slot));
                if bucket.is_empty() {
                    self.points.remove(&key);
                }
            }
            Some(axis) => {
                for segment in Segments::new(axis, range) {
                    let key = (sheet, axis, segment);
                    let bucket = self
                        .segments
                        .get_mut(&key)
                        .expect("the live segment is indexed");
                    assert!(bucket.remove(&id.slot));
                    if bucket.is_empty() {
                        self.segments.remove(&key);
                    }
                }
            }
        }
        self.free = Some(id.slot);
        Ok(())
    }

    /// A node retains its peak dynamic suffix. Disabled memberships remain
    /// owned but never seed dirty closure, contribute degrees, or form SCCs.
    fn deactivate(&mut self, id: &RegistrationId) {
        debug_assert_eq!(id.owner, self.owner);
        let Slot::Live(registration) = &mut self.slots[id.slot] else {
            unreachable!("the node owns this retained registration")
        };
        registration.active = false;
    }

    fn reactivate(
        &mut self,
        id: &RegistrationId,
        sheet: SheetKey,
        range: CellRange,
        strict: bool,
    ) -> bool {
        debug_assert_eq!(id.owner, self.owner);
        let Slot::Live(registration) = &mut self.slots[id.slot] else {
            unreachable!("the node owns this retained registration")
        };
        if registration.sheet != sheet
            || registration.range != range
            || registration.strict != strict
        {
            return false;
        }
        registration.active = true;
        true
    }

    /// Borrow a restartable-neighbor cursor. Independent cursors can coexist;
    /// querying allocates no result list, stack or per-node deduplication set.
    pub(crate) fn dependents(&self, sheet: SheetKey, at: CellRef) -> Dependents<'_> {
        self.neighbors(sheet, at, true)
    }

    fn watched_dependents(&self, sheet: SheetKey, at: CellRef) -> Dependents<'_> {
        self.neighbors(sheet, at, false)
    }

    fn neighbors(&self, sheet: SheetKey, at: CellRef, strict_only: bool) -> Dependents<'_> {
        let valid = at.is_in_grid();
        Dependents {
            index: self,
            sheet,
            at,
            strict_only,
            bucket: if valid {
                self.points.get(&(sheet, at)).map(BTreeSet::iter)
            } else {
                None
            },
            rows: if valid {
                Axis::Rows.limit() + at.row()
            } else {
                0
            },
            columns: if valid {
                Axis::Columns.limit() + at.column()
            } else {
                0
            },
            #[cfg(feature = "internals")]
            lookups: usize::from(valid),
            #[cfg(feature = "internals")]
            candidates: 0,
        }
    }
}

/// Canonical disjoint cover of an inclusive interval in a complete implicit
/// binary tree. At most two buckets per level; the complete axis uses root1.
struct Segments {
    first: u32,
    end: u32,
}

impl Segments {
    fn new(axis: Axis, range: CellRange) -> Self {
        let (first, last) = axis.span(range);
        Self {
            first: axis.limit() + first,
            end: axis.limit() + last + 1,
        }
    }
}

impl Iterator for Segments {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        while self.first < self.end {
            if self.first & 1 != 0 {
                let segment = self.first;
                self.first += 1;
                return Some(segment);
            }
            if self.end & 1 != 0 {
                self.end -= 1;
                return Some(self.end);
            }
            self.first /= 2;
            self.end /= 2;
        }
        None
    }
}

/// Neighbors borrowed from the index. IDs come directly from canonical
/// buckets, so one registration cannot appear twice on a root-to-leaf path.
#[derive(Clone)]
pub(crate) struct Dependents<'a> {
    index: &'a DependencyIndex,
    sheet: SheetKey,
    at: CellRef,
    strict_only: bool,
    bucket: Option<btree_set::Iter<'a, usize>>,
    rows: u32,
    columns: u32,
    #[cfg(feature = "internals")]
    lookups: usize,
    #[cfg(feature = "internals")]
    candidates: usize,
}

impl Iterator for Dependents<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(id) = self.bucket.as_mut().and_then(Iterator::next) {
                #[cfg(feature = "internals")]
                {
                    self.candidates += 1;
                }
                let Slot::Live(registration) = &self.index.slots[*id] else {
                    unreachable!("buckets contain only live registrations")
                };
                if registration.active
                    && (!self.strict_only || registration.strict)
                    && registration.range.contains(self.at)
                {
                    return Some(registration.dependent);
                }
                continue;
            }
            let (axis, segment) = if self.rows != 0 {
                let segment = self.rows;
                self.rows /= 2;
                (Axis::Rows, segment)
            } else if self.columns != 0 {
                let segment = self.columns;
                self.columns /= 2;
                (Axis::Columns, segment)
            } else {
                return None;
            };
            #[cfg(feature = "internals")]
            {
                self.lookups += 1;
            }
            self.bucket = self
                .index
                .segments
                .get(&(self.sheet, axis, segment))
                .map(BTreeSet::iter);
        }
    }
}

/// A formula's identity uses the workbook's stable sheet key.
pub(crate) type Address = (SheetKey, CellRef);

/// A clean formula's retained result status. Computed Excel error values are
/// Computed; Held and Circular must never be read as their prior cached value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Computed,
    Held,
    Circular,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum PassKind {
    Full,
    Incremental,
}

#[derive(Debug)]
struct Node {
    address: Address,
    precedents: Vec<RegistrationId>,
    fixed: usize,
    used: usize,
    selective: bool,
    status: Status,
}

#[derive(Debug)]
enum NodeSlot {
    Live(Node),
    Free(Option<usize>),
}

/// Resolved formula dependencies only: no formula text, AST, value, name
/// resolver or duplicate evaluator. Source rectangles are owned by index
/// registrations; nodes retain only their exclusive removal handles.
/// Dynamic suffixes retain peak reached-read capacity, including disabled
/// memberships. Retained memory is O(P_peak log axis); query candidate bounds
/// include those disabled memberships, but semantic results never do.
///
/// Updates seed node IDs immediately, including dependents of a removed
/// formula's old address. Pending IDs and volatile IDs are bounded by live
/// nodes. Slab/map capacity retains its high-water mark for reuse.
#[derive(Debug, Default)]
pub(crate) struct Graph {
    index: DependencyIndex,
    nodes: Vec<NodeSlot>,
    free: Option<usize>,
    addresses: HashMap<Address, usize>,
    pending: BTreeSet<usize>,
    volatile: BTreeSet<usize>,
    revision: u64,
    uncomputed: u64,
    circular_count: u64,
}

/// A prepared pass owns O(D) addresses, with no O(E) adjacency. The ready
/// queue retains at most one live entry per dirty node across suspensions. It does not
/// change retained status or consume pending changes until acknowledgment.
/// Ordered nodes are candidates for one evaluator visit, not a promise that
/// they are computable. Their reference reads use the pass overlay first and
/// retained status only for clean predecessors.
#[derive(Debug, Default)]
pub(crate) struct Schedule {
    owner: u64,
    revision: u64,
    work: Work,
    ready: VecDeque<usize>,
    ordered: Vec<Address>,
    circular: Vec<Address>,
    blocked: Vec<Address>,
}

impl Schedule {
    #[cfg(feature = "internals")]
    pub(crate) fn ordered(&self) -> &[Address] {
        &self.ordered
    }
    #[cfg(feature = "internals")]
    pub(crate) fn circular(&self) -> &[Address] {
        &self.circular
    }
    #[cfg(feature = "internals")]
    pub(crate) fn blocked(&self) -> &[Address] {
        &self.blocked
    }
    /// Dirty-node count for the caller's outcome vector. Positions include
    /// held/circular nodes as well as once-only evaluation candidates.
    pub(crate) fn len(&self) -> usize {
        self.work.nodes.len()
    }
}

impl Graph {
    fn node(&self, id: usize) -> &Node {
        let NodeSlot::Live(node) = &self.nodes[id] else {
            unreachable!("node indexes contain only live slots")
        };
        node
    }

    fn node_mut(&mut self, id: usize) -> &mut Node {
        let NodeSlot::Live(node) = &mut self.nodes[id] else {
            unreachable!("node indexes contain only live slots")
        };
        node
    }

    fn advance(&mut self) {
        self.revision = self
            .revision
            .checked_add(1)
            .expect("dependency graph revision exhausted");
    }

    fn seed(&mut self, address: Address) {
        if let Some(id) = self.addresses.get(&address) {
            self.pending.insert(*id);
        }
        self.pending
            .extend(self.index.watched_dependents(address.0, address.1));
    }

    /// Replace a formula's already-resolved, active precedents atomically.
    /// The caller must not register speculative references from unresolved
    /// lazy/dynamic expressions as though those references were active.
    pub(crate) fn set(
        &mut self,
        address: Address,
        precedents: &[(SheetKey, CellRange)],
        volatile: bool,
        selective: bool,
    ) -> Result<()> {
        DependencyIndex::check(address.0, CellRange::new(address.1, address.1))?;
        for &(sheet, range) in precedents {
            DependencyIndex::check(sheet, range)?;
        }
        let (id, fresh) = if let Some(&id) = self.addresses.get(&address) {
            (id, false)
        } else if let Some(id) = self.free {
            let NodeSlot::Free(next) = &self.nodes[id] else {
                unreachable!("the node free list names only free slots")
            };
            self.free = *next;
            self.nodes[id] = NodeSlot::Live(Node {
                address,
                precedents: Vec::new(),
                fixed: 0,
                used: 0,
                selective: false,
                status: Status::Held,
            });
            self.addresses.insert(address, id);
            (id, true)
        } else {
            let id = self.nodes.len();
            self.nodes.push(NodeSlot::Live(Node {
                address,
                precedents: Vec::new(),
                fixed: 0,
                used: 0,
                selective: false,
                status: Status::Held,
            }));
            self.addresses.insert(address, id);
            (id, true)
        };
        // Validation above is the only fallible part. Old handles are owned
        // by this index, and every new rectangle has already crossed intake.
        self.seed(address);
        let mut registrations = std::mem::take(&mut self.node_mut(id).precedents);
        for handle in registrations.drain(..) {
            self.index
                .remove(handle)
                .expect("node registrations belong to this graph");
        }
        // Reuse the common unchanged-arity buffer without retaining each
        // node's unrelated historical peak after its dependency count changes.
        if registrations.capacity() != precedents.len() {
            registrations = Vec::with_capacity(precedents.len());
        }
        registrations.extend(
            precedents
                .iter()
                .map(|&(sheet, range)| self.index.insert_checked(sheet, range, id, true)),
        );
        self.node_mut(id).precedents = registrations;
        self.node_mut(id).fixed = precedents.len();
        self.node_mut(id).used = precedents.len();
        self.node_mut(id).selective = selective;
        if fresh {
            self.uncomputed += 1;
        }
        self.set_status(id, Status::Held);
        if volatile {
            self.volatile.insert(id);
        } else {
            self.volatile.remove(&id);
        }
        self.advance();
        Ok(())
    }

    /// Remove a formula while notifying consumers that its old address now
    /// resolves through the ordinary cell reader. No removed-address log is kept.
    pub(crate) fn remove(&mut self, address: Address) -> bool {
        let Some(&id) = self.addresses.get(&address) else {
            return false;
        };
        self.seed(address);
        let NodeSlot::Live(node) =
            std::mem::replace(&mut self.nodes[id], NodeSlot::Free(self.free))
        else {
            unreachable!("the address index names only live nodes")
        };
        self.free = Some(id);
        self.addresses.remove(&address);
        self.pending.remove(&id);
        self.volatile.remove(&id);
        self.uncomputed -= u64::from(node.status != Status::Computed);
        self.circular_count -= u64::from(node.status == Status::Circular);
        for handle in node.precedents {
            self.index
                .remove(handle)
                .expect("node registrations belong to this graph");
        }
        self.advance();
        true
    }

    /// A changed scalar/cache/formula address seeds itself and direct
    /// consumers; the next pass takes the transitive closure without cells.
    pub(crate) fn changed(&mut self, address: Address) -> Result<()> {
        DependencyIndex::check(address.0, CellRange::new(address.1, address.1))?;
        self.seed(address);
        self.advance();
        Ok(())
    }

    pub(crate) fn status(&self, address: Address) -> Option<Status> {
        self.addresses.get(&address).map(|&id| self.node(id).status)
    }

    /// Current workbook-wide retained status, independent of this pass's dirty set.
    pub(crate) const fn status_counts(&self) -> (u64, u64) {
        (self.uncomputed, self.circular_count)
    }

    /// Cycle addresses without a result allocation. The caller limits/sorts
    /// its public report; the graph never truncates the actual status count.
    pub(crate) fn circular_cells(&self) -> impl Iterator<Item = Address> + '_ {
        self.nodes.iter().filter_map(|slot| match slot {
            NodeSlot::Live(node) if node.status == Status::Circular => Some(node.address),
            _ => None,
        })
    }

    fn set_status(&mut self, id: usize, status: Status) {
        let prior = self.node(id).status;
        if prior == status {
            return;
        }
        self.uncomputed -= u64::from(prior != Status::Computed);
        self.uncomputed += u64::from(status != Status::Computed);
        self.circular_count -= u64::from(prior == Status::Circular);
        self.circular_count += u64::from(status == Status::Circular);
        self.node_mut(id).status = status;
    }

    /// Schedule the dirty induced graph using reverse queries only. The
    /// caller retains scratch capacity between passes; each dirty node opens
    /// three cursors: closure, degree and Kahn OR SCC. No O(E) edge list is
    /// stored. Acyclic warm passes allocate no work buffers; borrowed Tarjan
    /// frames allocate once per cyclic pass at that pass's residue bound.
    fn collect(&self, kind: PassKind, pass: &mut Schedule) {
        // A refused partial preparation must not be acknowledged as a pass.
        pass.owner = 0;
        pass.revision = 0;
        let Schedule {
            work,
            ready,
            ordered,
            circular,
            blocked,
            ..
        } = pass;
        work.clear();
        ready.clear();
        ordered.clear();
        circular.clear();
        blocked.clear();
        match kind {
            PassKind::Full => {
                work.reserve(self.addresses.len());
                for &id in self.addresses.values() {
                    work.add(id);
                }
            }
            PassKind::Incremental => {
                for &id in self.pending.iter().chain(&self.volatile) {
                    work.add(id);
                }
            }
        }
        let mut cursor = 0;
        while cursor < work.nodes.len() {
            let address = self.node(work.nodes[cursor].id).address;
            for next in self.index.watched_dependents(address.0, address.1) {
                work.add(next);
            }
            cursor += 1;
        }
    }

    fn degrees(&self, pass: &mut Schedule) -> Result<()> {
        let Schedule {
            work,
            ready,
            ordered,
            ..
        } = pass;
        for at in 0..work.nodes.len() {
            let address = self.node(work.nodes[at].id).address;
            for next in self.index.dependents(address.0, address.1) {
                let following = work.positions[&next];
                let degree = &mut work.nodes[following].remaining;
                *degree = degree.checked_add(1).ok_or_else(|| Error::InvalidRecord {
                    path: format_smolstr!(
                        "$.sheets[{}].formulas[{}]",
                        self.node(next).address.0.as_u32(),
                        self.node(next).address.1
                    ),
                    reason:
                        "expected an incoming dependency count representable as usize, got overflow"
                            .into(),
                })?;
            }
        }
        ready.reserve(work.nodes.len());
        ready.extend(
            work.nodes
                .iter()
                .enumerate()
                .filter_map(|(at, node)| (node.remaining == 0).then_some(at)),
        );
        ready
            .make_contiguous()
            .sort_unstable_by_key(|&at| self.node(work.nodes[at].id).address);
        ordered.reserve(work.nodes.len());
        pass.owner = self.index.owner;
        pass.revision = self.revision;
        Ok(())
    }

    fn classify(&self, pass: &mut Schedule) {
        let Schedule {
            work,
            circular,
            blocked,
            ..
        } = pass;
        work.classify(self);
        for node in &work.nodes {
            if node.remaining != 0 {
                let address = self.node(node.id).address;
                if node.circular {
                    circular.push(address);
                } else {
                    blocked.push(address);
                }
            }
        }
        circular.sort_unstable();
        blocked.sort_unstable();
    }

    #[cfg(feature = "internals")]
    pub(crate) fn prepare(&self, kind: PassKind, pass: &mut Schedule) -> Result<()> {
        self.collect(kind, pass);
        self.degrees(pass)?;
        while let Some(address) = self.next(pass) {
            self.complete_edges(pass, address);
        }
        self.classify(pass);
        Ok(())
    }

    /// Dirty closure uses the previous selected edges. Only afterward may
    /// a dirty selector retire its dynamic suffix and choose new branches.
    pub(crate) fn begin(&mut self, kind: PassKind, pass: &mut Schedule) -> Result<()> {
        self.collect(kind, pass);
        for visit in &pass.work.nodes {
            let id = visit.id;
            let NodeSlot::Live(node) = &mut self.nodes[id] else {
                unreachable!("dirty work names live nodes")
            };
            for handle in &node.precedents[node.fixed..] {
                self.index.deactivate(handle);
            }
            node.used = node.fixed;
        }
        self.degrees(pass)
    }

    /// Next runnable root, including a previously suspended continuation.
    pub(crate) fn next(&self, pass: &mut Schedule) -> Option<Address> {
        let at = pass.ready.pop_front()?;
        debug_assert_eq!(pass.work.nodes[at].remaining, 0);
        Some(self.node(pass.work.nodes[at].id).address)
    }

    pub(crate) fn selective(&self, address: Address) -> bool {
        self.node(self.addresses[&address]).selective
    }

    pub(crate) fn ready(&self, pass: &Schedule, address: Address) -> bool {
        self.scheduled_index(pass, address)
            .is_some_and(|at| pass.work.nodes[at].remaining == 0)
    }

    /// Whether a source formula still needs this pass. Ordered consumers
    /// admit only this point before suspending, then publish one visited
    /// prefix rectangle after the scan completes.
    pub(crate) fn pending(&self, pass: &Schedule, source: Address) -> bool {
        self.scheduled_index(pass, source)
            .is_some_and(|at| !pass.work.nodes[at].finished)
    }

    /// Admit one reached source rectangle and count unfinished dirty sources
    /// with the caller's sparse cell cursor. Each descriptor/use is admitted
    /// once; separate rectangles preserve reverse-query multiplicity.
    pub(crate) fn admit(
        &mut self,
        pass: &mut Schedule,
        address: Address,
        sheet: SheetKey,
        range: CellRange,
        cells: impl Iterator<Item = CellRef>,
    ) -> Result<()> {
        self.admit_mode(pass, address, sheet, range, cells, true)
    }

    pub(crate) fn watch(
        &mut self,
        pass: &mut Schedule,
        address: Address,
        sheet: SheetKey,
        range: CellRange,
    ) -> Result<()> {
        self.admit_mode(pass, address, sheet, range, std::iter::empty(), false)
    }

    fn admit_mode(
        &mut self,
        pass: &mut Schedule,
        address: Address,
        sheet: SheetKey,
        range: CellRange,
        cells: impl Iterator<Item = CellRef>,
        strict: bool,
    ) -> Result<()> {
        self.check_pass(pass)?;
        DependencyIndex::check(sheet, range)?;
        let at = self
            .scheduled_index(pass, address)
            .expect("only scheduled roots admit reads");
        let mut added = 0usize;
        for cell in cells {
            if let Some(source) = self.scheduled_index(pass, (sheet, cell))
                && !pass.work.nodes[source].finished
            {
                added = added.checked_add(1).ok_or_else(|| Error::InvalidRecord {
                    path: "$.formula.calculation.dependencies".into(),
                    reason: "expected a usize dependency count, got overflow".into(),
                })?;
            }
        }
        let remaining = pass.work.nodes[at]
            .remaining
            .checked_add(added)
            .ok_or_else(|| Error::InvalidRecord {
                path: "$.formula.calculation.dependencies".into(),
                reason: "expected a usize dependency count, got overflow".into(),
            })?;
        let id = pass.work.nodes[at].id;
        // Ordinal reuse is O(1), without a second rectangle lookup index.
        // A changed branch replaces only the mismatching slot; an unused
        // suffix stays disabled and bounds storage by peak reached reads.
        let used = self.node(id).used;
        let mut handles = std::mem::take(&mut self.node_mut(id).precedents);
        if let Some(handle) = handles.get_mut(used) {
            if !self.index.reactivate(handle, sheet, range, strict) {
                let replacement = self.index.insert_checked(sheet, range, id, strict);
                let old = std::mem::replace(handle, replacement);
                self.index.remove(old).expect("exclusive node registration");
            }
        } else {
            debug_assert_eq!(used, handles.len());
            handles.push(self.index.insert_checked(sheet, range, id, strict));
        }
        self.node_mut(id).precedents = handles;
        self.node_mut(id).used += 1;
        pass.work.nodes[at].remaining = remaining;
        Ok(())
    }

    fn complete_edges(&self, pass: &mut Schedule, address: Address) {
        let at = self
            .scheduled_index(pass, address)
            .expect("a runnable root belongs to its pass");
        debug_assert!(!pass.work.nodes[at].finished);
        pass.work.nodes[at].finished = true;
        pass.ordered.push(address);
        for next in self.index.dependents(address.0, address.1) {
            let following = pass.work.positions[&next];
            if pass.work.nodes[following].finished {
                continue;
            }
            let degree = &mut pass.work.nodes[following].remaining;
            debug_assert!(*degree != 0);
            *degree -= 1;
            if *degree == 0 {
                pass.ready.push_back(following);
            }
        }
    }

    pub(crate) fn complete(&mut self, pass: &mut Schedule, address: Address, volatile: bool) {
        self.complete_edges(pass, address);
        let id = self.addresses[&address];
        if volatile {
            self.volatile.insert(id);
        } else {
            self.volatile.remove(&id);
        }
    }

    /// Classify only the final active residue, never the old branch graph.
    pub(crate) fn finish(&self, pass: &mut Schedule) {
        debug_assert!(pass.ready.is_empty());
        self.classify(pass);
    }

    fn check_pass(&self, pass: &Schedule) -> Result<()> {
        if pass.owner != self.index.owner || pass.revision != self.revision {
            return Err(Error::Conflict {
                expected: "a schedule for the current dependency graph revision",
                actual: "a schedule from another graph or an earlier revision",
                path: "$.formula.calculation".into(),
            });
        }
        Ok(())
    }

    /// Reuse the scheduling map for the evaluator's dirty outcome overlay.
    /// A stale/foreign schedule supplies no readable pass-local value.
    pub(crate) fn scheduled_index(&self, pass: &Schedule, address: Address) -> Option<usize> {
        if pass.owner != self.index.owner || pass.revision != self.revision {
            return None;
        }
        pass.work
            .positions
            .get(self.addresses.get(&address)?)
            .copied()
    }

    /// Publish statuses only after workbook value publication is ready. Each
    /// ordered node supplies exactly one Computed/Held receipt, including
    /// consumers of clean Held/Circular predecessors. Cyclic residue is
    /// graph-owned. Failure leaves statuses and pending seeds unchanged.
    pub(crate) fn acknowledge(&mut self, pass: &Schedule, computed: &[bool]) -> Result<()> {
        self.check_pass(pass)?;
        if computed.len() != pass.ordered.len() {
            return Err(Error::InvalidRecord {
                path: "$.formula.calculation.outcomes".into(),
                reason: format_smolstr!(
                    "expected {} ordered outcomes, got {}",
                    pass.ordered.len(),
                    computed.len()
                ),
            });
        }
        for (&address, &computed) in pass.ordered.iter().zip(computed) {
            let id = self.addresses[&address];
            self.set_status(
                id,
                if computed {
                    Status::Computed
                } else {
                    Status::Held
                },
            );
        }
        for address in &pass.circular {
            let id = self.addresses[address];
            self.set_status(id, Status::Circular);
        }
        for address in &pass.blocked {
            let id = self.addresses[address];
            self.set_status(id, Status::Held);
        }
        self.pending.clear();
        self.advance();
        Ok(())
    }
}

#[derive(Debug, Default)]
struct Work {
    positions: HashMap<usize, usize>,
    nodes: Vec<Visit>,
    stack: Vec<usize>,
}

#[derive(Debug)]
struct Visit {
    id: usize,
    remaining: usize,
    finished: bool,
    number: usize,
    low: usize,
    active: bool,
    self_loop: bool,
    circular: bool,
}

struct Frame<'a> {
    at: usize,
    neighbors: Dependents<'a>,
}

impl Work {
    fn clear(&mut self) {
        self.positions.clear();
        self.nodes.clear();
        self.stack.clear();
    }

    fn reserve(&mut self, nodes: usize) {
        self.positions.reserve(nodes);
        self.nodes.reserve(nodes);
    }

    fn add(&mut self, id: usize) {
        let next = self.nodes.len();
        if let std::collections::hash_map::Entry::Vacant(entry) = self.positions.entry(id) {
            entry.insert(next);
            self.nodes.push(Visit {
                id,
                remaining: 0,
                finished: false,
                number: usize::MAX,
                low: 0,
                active: false,
                self_loop: false,
                circular: false,
            });
        }
    }

    fn enter<'a>(
        &mut self,
        at: usize,
        graph: &'a Graph,
        number: &mut usize,
        stack: &mut Vec<usize>,
        frames: &mut Vec<Frame<'a>>,
    ) {
        let node = &mut self.nodes[at];
        node.number = *number;
        node.low = *number;
        node.active = true;
        *number += 1;
        stack.push(at);
        let address = graph.node(node.id).address;
        frames.push(Frame {
            at,
            neighbors: graph.index.dependents(address.0, address.1),
        });
    }

    /// Tarjan on Kahn's residue, with borrowed neighbor cursors instead of
    /// recursion or a materialized edge list. Duplicate edges are harmless.
    fn classify(&mut self, graph: &Graph) {
        let residue = self.nodes.iter().filter(|node| node.remaining != 0).count();
        if residue == 0 {
            return;
        }
        let mut stack = std::mem::take(&mut self.stack);
        stack.reserve(residue);
        // Cursors borrow graph for this pass; keep no self-referential state.
        let mut frames = Vec::with_capacity(residue);
        let mut number = 0;
        for root in 0..self.nodes.len() {
            if self.nodes[root].remaining == 0 || self.nodes[root].number != usize::MAX {
                continue;
            }
            self.enter(root, graph, &mut number, &mut stack, &mut frames);
            while let Some(frame) = frames.last_mut() {
                let at = frame.at;
                if let Some(next) = frame.neighbors.next() {
                    let following = self.positions[&next];
                    if self.nodes[following].remaining == 0 {
                        continue;
                    }
                    if following == at {
                        self.nodes[at].self_loop = true;
                    }
                    if self.nodes[following].number == usize::MAX {
                        self.enter(following, graph, &mut number, &mut stack, &mut frames);
                    } else if self.nodes[following].active {
                        self.nodes[at].low = self.nodes[at].low.min(self.nodes[following].number);
                    }
                    continue;
                }
                frames.pop();
                if let Some(parent) = frames.last() {
                    self.nodes[parent.at].low = self.nodes[parent.at].low.min(self.nodes[at].low);
                }
                if self.nodes[at].low == self.nodes[at].number {
                    let mut members = 0;
                    loop {
                        let member = stack.pop().expect("an SCC root is on the active stack");
                        self.nodes[member].active = false;
                        self.nodes[member].circular = true;
                        members += 1;
                        if member == at {
                            break;
                        }
                    }
                    if members == 1 && !self.nodes[at].self_loop {
                        self.nodes[at].circular = false;
                    }
                }
            }
        }
        self.stack = stack;
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! Private index behavior and retained membership counts pinned by
    //! rust/tests/excel/formula/graph.rs; no workbook calculation API.

    use crate::excel::{CellRange, CellRef, SheetKey};

    /// A private-index fixture without extra retained test state.
    #[derive(Default)]
    pub struct Index(super::DependencyIndex);

    /// Exclusive registration handle used by the fixture.
    pub struct Registration(super::RegistrationId);

    /// Resolved graph fixture, without Workbook or evaluator integration.
    #[derive(Default)]
    pub struct Scheduler(super::Graph);

    /// Prepared scheduling result; no value or AST copies are retained.
    pub struct Pass(super::Schedule);

    /// Reusable schedule fixture; production owns the same schedule buffers.
    #[derive(Default)]
    pub struct Workspace(Option<super::Schedule>);

    impl Workspace {
        /// Once-only evaluator candidates from the most recently prepared pass.
        pub fn ordered(&self) -> &[(SheetKey, CellRef)] {
            self.0.as_ref().map_or(&[], super::Schedule::ordered)
        }
        /// True cycle members from the most recently prepared pass.
        pub fn circular(&self) -> &[(SheetKey, CellRef)] {
            self.0.as_ref().map_or(&[], super::Schedule::circular)
        }
        /// Noncyclic nodes blocked behind a cycle.
        pub fn blocked(&self) -> &[(SheetKey, CellRef)] {
            self.0.as_ref().map_or(&[], super::Schedule::blocked)
        }
    }

    impl Pass {
        /// Once-only topological evaluator candidates.
        pub fn ordered(&self) -> &[(SheetKey, CellRef)] {
            self.0.ordered()
        }
        /// True cycle members in stable address order.
        pub fn circular(&self) -> &[(SheetKey, CellRef)] {
            self.0.circular()
        }
        /// Noncyclic Kahn residue blocked behind cycles.
        pub fn blocked(&self) -> &[(SheetKey, CellRef)] {
            self.0.blocked()
        }
    }

    impl Scheduler {
        /// Start an incremental scheduling pass that admits selected reads.
        pub fn begin(&mut self, full: bool) -> crate::Result<Pass> {
            let mut pass = super::Schedule::default();
            self.0.begin(
                if full {
                    super::PassKind::Full
                } else {
                    super::PassKind::Incremental
                },
                &mut pass,
            )?;
            Ok(Pass(pass))
        }
        /// Take one ready formula or suspended continuation.
        pub fn next(&self, pass: &mut Pass) -> Option<(SheetKey, CellRef)> {
            self.0.next(&mut pass.0)
        }
        /// Admit a selected rectangle using its sparse source-cell coordinates.
        pub fn admit(
            &mut self,
            pass: &mut Pass,
            address: (SheetKey, CellRef),
            sheet: SheetKey,
            range: CellRange,
            cells: &[CellRef],
        ) -> crate::Result<()> {
            self.0
                .admit(&mut pass.0, address, sheet, range, cells.iter().copied())
        }
        /// Finish the root once after all of its reached reads are ready.
        pub fn complete(&mut self, pass: &mut Pass, address: (SheetKey, CellRef)) {
            self.0.complete(&mut pass.0, address, false);
        }
        /// Classify the final active residue without guessing old branches.
        pub fn finish(&self, pass: &mut Pass) {
            self.0.finish(&mut pass.0);
        }
        /// Replace one node using already-resolved rectangles.
        pub fn set(
            &mut self,
            sheet: SheetKey,
            at: CellRef,
            precedents: &[(SheetKey, CellRange)],
            volatile: bool,
        ) -> crate::Result<()> {
            self.0.set((sheet, at), precedents, volatile, false)
        }
        /// Remove a formula and seed consumers at its former address.
        pub fn remove(&mut self, sheet: SheetKey, at: CellRef) -> bool {
            self.0.remove((sheet, at))
        }
        /// Mark one changed address.
        pub fn changed(&mut self, sheet: SheetKey, at: CellRef) -> crate::Result<()> {
            self.0.changed((sheet, at))
        }
        /// Prepare a full or incremental pass without acknowledging it.
        pub fn prepare(&self, full: bool) -> crate::Result<Pass> {
            let mut pass = super::Schedule::default();
            self.0.prepare(
                if full {
                    super::PassKind::Full
                } else {
                    super::PassKind::Incremental
                },
                &mut pass,
            )?;
            Ok(Pass(pass))
        }
        /// Publish one boolean Computed/Held receipt per ordered node.
        pub fn acknowledge(&mut self, pass: Pass, computed: &[bool]) -> crate::Result<()> {
            self.0.acknowledge(&pass.0, computed)
        }
        /// Prepare into a caller-owned workspace.
        ///
        /// # Errors
        ///
        /// Returns the scheduling refusal without publishing values.
        pub fn prepare_reusing(&self, full: bool, workspace: &mut Workspace) -> crate::Result<()> {
            self.0.prepare(
                if full {
                    super::PassKind::Full
                } else {
                    super::PassKind::Incremental
                },
                workspace.0.get_or_insert_with(super::Schedule::default),
            )
        }
        /// Acknowledge the workspace's current pass.
        ///
        /// # Errors
        ///
        /// Returns a missing, stale, foreign or incomplete pass refusal.
        pub fn acknowledge_reusing(
            &mut self,
            workspace: &mut Workspace,
            computed: &[bool],
        ) -> crate::Result<()> {
            let pass = workspace.0.as_ref().ok_or_else(|| {
                crate::Error::absent("prepared calculation pass", "$.formula.calculation")
            })?;
            self.0.acknowledge(pass, computed)
        }
        /// Workbook-wide uncomputed/circular counts, including clean nodes.
        pub fn status_counts(&self) -> (u64, u64) {
            self.0.status_counts()
        }
        /// Current cycle members, independent of the last dirty closure.
        pub fn circular_cells(&self) -> impl Iterator<Item = (SheetKey, CellRef)> + '_ {
            self.0.circular_cells()
        }
        /// Retained status used only when the evaluator's pass overlay has
        /// no newer result for this formula address.
        pub fn status(&self, sheet: SheetKey, at: CellRef) -> Option<&'static str> {
            self.0.status((sheet, at)).map(|status| match status {
                super::Status::Computed => "computed",
                super::Status::Held => "held",
                super::Status::Circular => "circular",
            })
        }
    }

    impl Index {
        /// Insert a typed predecessor for one dependent node.
        pub fn insert(
            &mut self,
            sheet: SheetKey,
            range: CellRange,
            dependent: usize,
        ) -> crate::Result<Registration> {
            self.0.insert(sheet, range, dependent).map(Registration)
        }

        /// Remove exactly the registration whose handle was retained.
        pub fn remove(&mut self, id: Registration) -> crate::Result<()> {
            self.0.remove(id.0)
        }

        /// Enumerate matching registrations without allocating query state.
        pub fn dependents(
            &self,
            sheet: SheetKey,
            at: CellRef,
        ) -> impl Iterator<Item = usize> + Clone + '_ {
            self.0.dependents(sheet, at)
        }

        /// Actual bucket lookups, candidate IDs visited and matching IDs
        /// while exhausting the ordinary borrowed cursor.
        pub fn query_counts(&self, sheet: SheetKey, at: CellRef) -> (usize, usize, usize) {
            let mut cursor = self.0.dependents(sheet, at);
            let matches = cursor.by_ref().count();
            (cursor.lookups, cursor.candidates, matches)
        }

        /// Live source records, retained slab length, nonempty buckets and
        /// ID memberships. Map/slab capacity may retain a prior high-water mark.
        pub fn counts(&self) -> (usize, usize, usize, usize) {
            let live = self
                .0
                .slots
                .iter()
                .filter(|slot| matches!(slot, super::Slot::Live(_)))
                .count();
            let buckets = self.0.points.len() + self.0.segments.len();
            let memberships = self
                .0
                .points
                .values()
                .chain(self.0.segments.values())
                .map(|ids| ids.len())
                .sum();
            (live, self.0.slots.len(), buckets, memberships)
        }
    }
}
