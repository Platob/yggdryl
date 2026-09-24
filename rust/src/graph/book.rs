//! Typed market operations and a stateful, price-ordered market book.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::Hasher;
use std::iter::FusedIterator;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::{
    Element, Event, Execution, ExecutionEntry, MarketElement, MarketElementData, MarketEvent,
    MarketEventData, Order, OrderEntry, Quote, QuoteEntry, Trade,
};
use crate::{Ccy, Decimal18, Error, Result, Side, State, Uuid};

/// The symbol of the one consolidated book emitted in global mode.
pub const GLOBAL_SYMBOL: &str = "GLOBAL";

const ACTION: &str = "MDUpdateAction";
const SNAPSHOT: &str = "SNAPSHOT";
const SCOPE: &str = "BookScope";
const ENTRY_ID: &str = "MDEntryID";
const ENTRY_REF_ID: &str = "MDEntryRefID";
const ENTRY_POSITION: &str = "MDEntryPositionNo";
const ENTRY_PRICE: &str = "MDEntryPx";
const ENTRY_SIZE: &str = "MDEntrySize";
const SNAPSHOT_SCOPE_PREFIX: &str = "BookSnapshotScope:";

/// Which concrete operation or scoped book control a heterogeneous value holds.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MarketOperationKind {
    Order,
    Quote,
    Execution,
    Trade,
    Snapshot,
}

impl MarketOperationKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Order => "order",
            Self::Quote => "quote",
            Self::Execution => "execution",
            Self::Trade => "trade",
            Self::Snapshot => "snapshot",
        }
    }
}

/// One heterogeneous market event or scoped snapshot control without dynamic
/// dispatch. A composite trade owns its canonical execution list.
#[derive(Clone, Debug, PartialEq)]
pub enum MarketOperation {
    Order(Order),
    Quote(Quote),
    Execution(Execution),
    Trade(Trade),
    Snapshot(MarketEventData),
}

impl MarketOperation {
    #[must_use]
    pub const fn kind(&self) -> MarketOperationKind {
        match self {
            Self::Order(_) => MarketOperationKind::Order,
            Self::Quote(_) => MarketOperationKind::Quote,
            Self::Execution(_) => MarketOperationKind::Execution,
            Self::Trade(_) => MarketOperationKind::Trade,
            Self::Snapshot(_) => MarketOperationKind::Snapshot,
        }
    }

    /// The operation without its instants, preserving its concrete kind.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for a composite trade or snapshot
    /// control, neither of which has one undated market entry.
    pub fn into_entry(self) -> Result<MarketEntry> {
        match self {
            Self::Order(value) => Ok(MarketEntry::Order(value.into())),
            Self::Quote(value) => Ok(MarketEntry::Quote(value.into())),
            Self::Execution(value) => Ok(MarketEntry::Execution(value.into())),
            Self::Trade(_) | Self::Snapshot(_) => Err(invalid(
                "$.operation",
                "a composite trade or book snapshot control has no undated market entry",
            )),
        }
    }

    /// Whether this operation is part of a FIX full-snapshot replacement.
    #[must_use]
    pub fn is_full_snapshot(&self) -> bool {
        self.get_identifiers().get(ACTION).map(String::as_str) == Some(SNAPSHOT)
    }

    fn scope(&self) -> &str {
        self.get_identifiers().get(SCOPE).map_or("", String::as_str)
    }
}

impl Default for MarketOperation {
    fn default() -> Self {
        Self::Order(Order::from(MarketEventData::default()))
    }
}

impl AsRef<MarketEventData> for MarketOperation {
    fn as_ref(&self) -> &MarketEventData {
        match self {
            Self::Order(value) => value.as_ref(),
            Self::Quote(value) => value.as_ref(),
            Self::Execution(value) => value.as_ref(),
            Self::Trade(value) => value.as_ref(),
            Self::Snapshot(value) => value,
        }
    }
}

impl AsMut<MarketEventData> for MarketOperation {
    fn as_mut(&mut self) -> &mut MarketEventData {
        match self {
            Self::Order(value) => value.as_mut(),
            Self::Quote(value) => value.as_mut(),
            Self::Execution(value) => value.as_mut(),
            Self::Trade(value) => value.as_mut(),
            Self::Snapshot(value) => value,
        }
    }
}

impl Element for MarketOperation {
    fn get_curruuid(&self) -> Uuid {
        self.as_ref().get_curruuid()
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.as_mut().set_curruuid(curruuid);
    }

    fn get_crossuuid(&self) -> Uuid {
        self.as_ref().get_crossuuid()
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.as_mut().set_crossuuid(crossuuid);
    }

    fn get_crosscode(&self) -> &str {
        self.as_ref().get_crosscode()
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.as_mut().set_crosscode(crosscode);
    }

    fn get_currhashcode(&self) -> u64 {
        self.as_ref().get_currhashcode()
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.as_mut().set_currhashcode(hashcode);
    }

    fn get_crosshashcode(&self) -> u64 {
        self.as_ref().get_crosshashcode()
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.as_mut().set_crosshashcode(crosshashcode);
    }

    fn get_identifiers(&self) -> &BTreeMap<String, String> {
        self.as_ref().get_identifiers()
    }

    fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) {
        self.as_mut().set_identifiers(identifiers);
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        self.as_ref().get_srcuuids()
    }

    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
        self.as_mut().set_srcuuids(sources);
    }

    fn is_after(&self, other: &Self) -> bool {
        self.as_ref().is_after(other.as_ref())
    }

    fn finalize(&mut self) {
        match self {
            Self::Order(value) => value.finalize(),
            Self::Quote(value) => value.finalize(),
            Self::Execution(value) => value.finalize(),
            Self::Trade(value) => value.finalize(),
            Self::Snapshot(value) => value.finalize(),
        }
    }

    fn with_previous(self, previous: &Self) -> Option<Self> {
        match (self, previous) {
            (Self::Order(value), Self::Order(previous)) => {
                value.with_previous(previous).map(Self::Order)
            }
            (Self::Quote(value), Self::Quote(previous)) => {
                value.with_previous(previous).map(Self::Quote)
            }
            (Self::Execution(value), Self::Execution(previous)) => {
                value.with_previous(previous).map(Self::Execution)
            }
            (Self::Trade(value), Self::Trade(previous)) => {
                value.with_previous(previous).map(Self::Trade)
            }
            (Self::Snapshot(value), Self::Snapshot(previous)) => {
                value.with_previous(previous).map(Self::Snapshot)
            }
            _ => None,
        }
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        match (self, other) {
            (Self::Order(value), Self::Order(other)) => value.merge_with(other).map(Self::Order),
            (Self::Quote(value), Self::Quote(other)) => value.merge_with(other).map(Self::Quote),
            (Self::Execution(value), Self::Execution(other)) => {
                value.merge_with(other).map(Self::Execution)
            }
            (Self::Trade(value), Self::Trade(other)) => value.merge_with(other).map(Self::Trade),
            (Self::Snapshot(value), Self::Snapshot(other)) => {
                value.merge_with(other).map(Self::Snapshot)
            }
            _ => None,
        }
    }
}

delegate_market_value!(MarketOperation, MarketEventData, market_only);
delegate_market_value!(
    MarketOperation,
    MarketEventData,
    event_only,
    |this: &MarketOperation| matches!(
        this,
        MarketOperation::Execution(_) | MarketOperation::Trade(_)
    ),
    |this: &mut MarketOperation, unix: i64| match this {
        MarketOperation::Trade(trade) => trade.rebase_currunix(unix),
        _ => Event::set_currunix(
            <MarketOperation as AsMut<MarketEventData>>::as_mut(this),
            unix,
        ),
    }
);

impl From<Order> for MarketOperation {
    fn from(value: Order) -> Self {
        Self::Order(value)
    }
}

impl From<Quote> for MarketOperation {
    fn from(value: Quote) -> Self {
        Self::Quote(value)
    }
}

impl From<Execution> for MarketOperation {
    fn from(value: Execution) -> Self {
        Self::Execution(value)
    }
}

impl From<Trade> for MarketOperation {
    fn from(value: Trade) -> Self {
        Self::Trade(value)
    }
}

macro_rules! operation_try_from {
    ($type:ty, $variant:ident) => {
        impl TryFrom<MarketOperation> for $type {
            type Error = Error;

            fn try_from(value: MarketOperation) -> Result<Self> {
                match value {
                    MarketOperation::$variant(value) => Ok(value),
                    other => Err(invalid(
                        "$.operation",
                        format_smolstr!(
                            "expected {}, got {}",
                            stringify!($variant).to_ascii_lowercase(),
                            other.kind().as_str()
                        ),
                    )),
                }
            }
        }
    };
}

operation_try_from!(Order, Order);
operation_try_from!(Quote, Quote);
operation_try_from!(Execution, Execution);
operation_try_from!(Trade, Trade);

/// One heterogeneous market element without allocation or dynamic dispatch.
#[derive(Clone, Debug, PartialEq)]
pub enum MarketEntry {
    Order(OrderEntry),
    Quote(QuoteEntry),
    Execution(ExecutionEntry),
}

impl MarketEntry {
    #[must_use]
    pub const fn kind(&self) -> MarketOperationKind {
        match self {
            Self::Order(_) => MarketOperationKind::Order,
            Self::Quote(_) => MarketOperationKind::Quote,
            Self::Execution(_) => MarketOperationKind::Execution,
        }
    }

    /// This entry dated at `unix`, preserving its concrete kind.
    #[must_use]
    pub fn at(self, unix: i64) -> MarketOperation {
        let mut operation = match self {
            Self::Order(value) => MarketOperation::Order(value.into()),
            Self::Quote(value) => MarketOperation::Quote(value.into()),
            Self::Execution(value) => MarketOperation::Execution(value.into()),
        };
        operation.set_currunix(unix);
        operation.finalize();
        operation
    }
}

impl Default for MarketEntry {
    fn default() -> Self {
        Self::Order(OrderEntry::from(MarketElementData::default()))
    }
}

impl AsRef<MarketElementData> for MarketEntry {
    fn as_ref(&self) -> &MarketElementData {
        match self {
            Self::Order(value) => value.as_ref(),
            Self::Quote(value) => value.as_ref(),
            Self::Execution(value) => value.as_ref(),
        }
    }
}

impl AsMut<MarketElementData> for MarketEntry {
    fn as_mut(&mut self) -> &mut MarketElementData {
        match self {
            Self::Order(value) => value.as_mut(),
            Self::Quote(value) => value.as_mut(),
            Self::Execution(value) => value.as_mut(),
        }
    }
}

delegate_market_element!(MarketEntry, existing);

impl From<OrderEntry> for MarketEntry {
    fn from(value: OrderEntry) -> Self {
        Self::Order(value)
    }
}

impl From<QuoteEntry> for MarketEntry {
    fn from(value: QuoteEntry) -> Self {
        Self::Quote(value)
    }
}

impl From<ExecutionEntry> for MarketEntry {
    fn from(value: ExecutionEntry) -> Self {
        Self::Execution(value)
    }
}

impl From<MarketEntry> for MarketElementData {
    fn from(value: MarketEntry) -> Self {
        match value {
            MarketEntry::Order(value) => value.into(),
            MarketEntry::Quote(value) => value.into(),
            MarketEntry::Execution(value) => value.into(),
        }
    }
}

impl From<MarketEntry> for MarketOperation {
    fn from(value: MarketEntry) -> Self {
        value.at(0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum BookPrice {
    Bid(Reverse<Decimal18>),
    Ask(Decimal18),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct LiveKey {
    identity: Uuid,
    symbol: Option<SmolStr>,
    scope: SmolStr,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct SnapshotPartition {
    symbol: Option<SmolStr>,
    scope: SmolStr,
}

impl SnapshotPartition {
    fn of(operation: &MarketOperation) -> Self {
        Self {
            symbol: operation.get_symbolticker().map(SmolStr::new),
            scope: SmolStr::new(operation.scope()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum BookSideKind {
    Bid,
    Ask,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct BookExpiration {
    unix: i64,
    book: String,
    side: BookSideKind,
    identity: LiveKey,
    generation: Uuid,
}

impl LiveKey {
    fn of(operation: &MarketOperation) -> Self {
        Self {
            identity: operation.get_crossuuid(),
            symbol: operation.get_symbolticker().map(SmolStr::new),
            scope: SmolStr::new(operation.scope()),
        }
    }
}

impl BookPrice {
    fn of(side: &Side, price: Decimal18) -> Option<Self> {
        if side.is_bid() {
            Some(Self::Bid(Reverse(price)))
        } else if side.is_ask() {
            Some(Self::Ask(price))
        } else {
            None
        }
    }

    const fn price(self) -> Decimal18 {
        match self {
            Self::Bid(Reverse(price)) | Self::Ask(price) => price,
        }
    }
}

/// One side of a book: persistent live orders and quotes, price ordered,
/// beside the deltas applied since the last emitted book.
///
/// A live entry is shared: a book emitted per instant is a clone of the one
/// the walk keeps, and the entries an update did not touch are the same
/// entries in both, so emitting a deep book costs a reference count per
/// entry rather than a copy of each.
#[derive(Clone, Debug)]
pub struct BookSide {
    element: MarketElementData,
    levels: BTreeMap<BookPrice, Vec<Arc<MarketOperation>>>,
    /// Derived from `levels` and kept in step by every change, shared with
    /// clones until one changes: a book emitted per instant copies no index,
    /// and the side the walk keeps changes its own in place once the
    /// emitted book is gone.
    index: Arc<SideIndex>,
    deltas: Vec<MarketOperation>,
}

/// Two sides are equal by what they hold; the index is derived from it.
impl PartialEq for BookSide {
    fn eq(&self, other: &Self) -> bool {
        self.element == other.element && self.levels == other.levels && self.deltas == other.deltas
    }
}

/// Where each live identity of one side stands, and the live entry each
/// stated `MDEntryID` names within its partition: an update addressing an
/// entry is answered without walking a side thousands deep.
#[derive(Clone, Debug, Default)]
struct SideIndex {
    positions: HashMap<LiveKey, BookPrice>,
    entry_ids: HashMap<EntryKey, EntrySlot>,
}

impl SideIndex {
    /// Records a live entry under the id it states.
    fn record(&mut self, operation: &MarketOperation, identity: &LiveKey) {
        let Some(key) = EntryKey::of(operation) else {
            return;
        };
        let slot = self.entry_ids.entry(key).or_insert(EntrySlot::Many(0));
        *slot = match slot {
            EntrySlot::Many(0) => EntrySlot::One(identity.clone()),
            EntrySlot::One(_) => EntrySlot::Many(2),
            EntrySlot::Many(count) => EntrySlot::Many(*count + 1),
        };
    }

    /// Forgets a live entry leaving the side under the id it states.
    fn forget(&mut self, operation: &MarketOperation) {
        let Some(key) = EntryKey::of(operation) else {
            return;
        };
        let std::collections::hash_map::Entry::Occupied(mut slot) = self.entry_ids.entry(key)
        else {
            return;
        };
        match slot.get_mut() {
            EntrySlot::Many(count) if *count > 1 => *count -= 1,
            _ => {
                slot.remove();
            }
        }
    }
}

/// One `MDEntryID` within one book partition.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct EntryKey {
    partition: SnapshotPartition,
    entry: SmolStr,
}

impl EntryKey {
    fn of(operation: &MarketOperation) -> Option<Self> {
        let entry = operation.get_identifiers().get(ENTRY_ID)?;
        Some(Self {
            partition: SnapshotPartition::of(operation),
            entry: SmolStr::new(entry),
        })
    }
}

/// The live entries one [`EntryKey`] names: the one, or how many where
/// several do - which the walk over the side answers in its own order.
#[derive(Clone, Debug)]
enum EntrySlot {
    One(LiveKey),
    Many(usize),
}

struct RemovedLive {
    identity: LiveKey,
    price: BookPrice,
    index: usize,
    operation: Arc<MarketOperation>,
}

struct SideJournal {
    element: MarketElementData,
    deltas_len: usize,
    cleared_deltas: Option<Vec<MarketOperation>>,
    inserted: Vec<LiveKey>,
    removed: Vec<RemovedLive>,
}

impl SideJournal {
    fn new(side: &mut BookSide, clear_deltas: bool) -> Self {
        let deltas_len = side.deltas.len();
        let cleared_deltas = clear_deltas.then(|| std::mem::take(&mut side.deltas));
        Self {
            element: side.element.clone(),
            deltas_len,
            cleared_deltas,
            inserted: Vec::new(),
            removed: Vec::new(),
        }
    }

    fn rollback(self, side: &mut BookSide) {
        for identity in self.inserted.into_iter().rev() {
            let _ = side.take_removed(&identity);
        }
        for removed in self.removed.into_iter().rev() {
            let level = side.levels.entry(removed.price).or_default();
            let index = removed.index.min(level.len());
            side.insert_live(removed.price, index, removed.operation, removed.identity);
        }
        if let Some(deltas) = self.cleared_deltas {
            side.deltas = deltas;
        } else {
            side.deltas.truncate(self.deltas_len);
        }
        side.element = self.element;
    }
}

impl BookSide {
    /// An empty bid or ask side.
    pub fn new(side: Side) -> Result<Self> {
        if !side.is_bid() && !side.is_ask() {
            return Err(invalid(
                "$.side",
                format_smolstr!("expected a bid or ask side, got {:?}", side.as_str()),
            ));
        }
        let mut element = MarketElementData::default();
        element.set_side(side);
        let mut side = Self {
            element,
            levels: BTreeMap::new(),
            index: Arc::default(),
            deltas: Vec::new(),
        };
        side.finalize();
        Ok(side)
    }

    /// Rebuilds one canonical side from its serialized parts without replaying
    /// the serialized deltas as fresh mutations.
    pub(crate) fn from_parts(
        element: MarketElementData,
        live: Vec<MarketOperation>,
        deltas: Vec<MarketOperation>,
    ) -> Result<Self> {
        Self::from_shared_parts(element, live.into_iter().map(Arc::new).collect(), deltas)
    }

    /// [`Self::from_parts`] over live entries already shared, which a side
    /// rebuilt around its own entries keeps rather than copies.
    fn from_shared_parts(
        element: MarketElementData,
        live: Vec<Arc<MarketOperation>>,
        deltas: Vec<MarketOperation>,
    ) -> Result<Self> {
        if !element.get_side().is_bid() && !element.get_side().is_ask() {
            return Err(invalid(
                "$.side",
                format_smolstr!(
                    "expected a bid or ask side, got {:?}",
                    element.get_side().as_str()
                ),
            ));
        }
        let mut side = Self {
            element,
            levels: BTreeMap::new(),
            index: Arc::default(),
            deltas: Vec::new(),
        };
        let mut built = SideIndex {
            positions: HashMap::with_capacity(live.len()),
            entry_ids: HashMap::new(),
        };
        for (index, operation) in live.into_iter().enumerate() {
            let path = format_smolstr!("$.live[{index}]");
            side.validate_component(&operation, &path, true)?;
            let price = BookPrice::of(operation.get_side(), operation.get_price())
                .expect("a validated side has a book price");
            let identity = LiveKey::of(&operation);
            if built.positions.insert(identity.clone(), price).is_some() {
                return Err(invalid(
                    path,
                    format_smolstr!(
                        "duplicate live market identity {} in scope {:?}",
                        identity.identity,
                        identity.scope
                    ),
                ));
            }
            built.record(&operation, &identity);
            side.levels.entry(price).or_default().push(operation);
        }
        side.index = Arc::new(built);
        for level in side.levels.values_mut() {
            if level.iter().any(|held| position_of(held).is_some()) {
                level.sort_by_key(|held| position_of(held).unwrap_or(u64::MAX));
            }
        }
        for (index, operation) in deltas.iter().enumerate() {
            side.validate_component(operation, &format_smolstr!("$.deltas[{index}]"), false)?;
        }
        side.deltas = deltas;
        side.refresh()?;
        Ok(side)
    }

    /// The live orders and quotes, best price first.
    pub fn live(&self) -> impl Iterator<Item = &MarketOperation> {
        self.levels
            .values()
            .flat_map(|level| level.iter().map(Arc::as_ref))
    }

    /// Deltas applied since this side was last cleared.
    #[must_use]
    pub fn deltas(&self) -> &[MarketOperation] {
        &self.deltas
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.index.positions.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.positions.is_empty()
    }

    /// The best live price on this side.
    #[must_use]
    pub fn best_price(&self) -> Option<Decimal18> {
        self.levels
            .first_key_value()
            .map(|(price, _)| price.price())
    }

    /// Aggregate quantity at the best exact price.
    #[must_use]
    pub fn best_quantity(&self) -> Option<Decimal18> {
        Some(
            self.levels
                .first_key_value()?
                .1
                .iter()
                .fold(Decimal18::ZERO, |quantity, operation| {
                    quantity + operation.get_quantity()
                }),
        )
    }

    /// Atomically applies one order or quote delta.
    pub fn add_operation(&mut self, operation: MarketOperation) -> Result<()> {
        let mut journal = SideJournal::new(self, false);
        let result = self
            .apply_inner(operation, None, Some(&mut journal))
            .and_then(|()| self.refresh());
        if let Err(error) = result {
            journal.rollback(self);
            return Err(error);
        }
        Ok(())
    }

    fn validate_component(
        &self,
        operation: &MarketOperation,
        path: &str,
        live: bool,
    ) -> Result<()> {
        if !matches!(
            operation.kind(),
            MarketOperationKind::Order | MarketOperationKind::Quote
        ) {
            return Err(invalid(path, "expected an order or quote on a book side"));
        }
        if live && !operation.get_state().is_live() {
            return Err(invalid(
                format_smolstr!("{path}.state"),
                "expected every live operation to have a live state",
            ));
        }
        if operation.get_side().is_bid() != self.get_side().is_bid()
            || (!operation.get_side().is_bid() && !operation.get_side().is_ask())
        {
            return Err(invalid(
                format_smolstr!("{path}.side"),
                format_smolstr!(
                    "expected {:?}, got {:?}",
                    self.get_side().as_str(),
                    operation.get_side().as_str()
                ),
            ));
        }
        Ok(())
    }

    fn apply(&mut self, operation: MarketOperation) -> Result<()> {
        self.apply_inner(operation, None, None)
    }

    fn apply_inner(
        &mut self,
        mut operation: MarketOperation,
        other_side: Option<&MarketOperation>,
        mut journal: Option<&mut SideJournal>,
    ) -> Result<()> {
        if !matches!(
            operation.kind(),
            MarketOperationKind::Order | MarketOperationKind::Quote
        ) {
            return Err(invalid(
                "$.operation",
                "expected an order or quote on a book side",
            ));
        }
        if operation.get_side().is_bid() != self.get_side().is_bid() {
            return Err(invalid(
                "$.operation.side",
                format_smolstr!(
                    "expected {:?}, got {:?}",
                    self.get_side().as_str(),
                    operation.get_side().as_str()
                ),
            ));
        }

        match operation.get_identifiers().get(ACTION).map(String::as_str) {
            Some("3") => {
                let position = range_position(&operation)?;
                self.remove_range(&operation, position, true, journal.as_deref_mut())?;
                self.deltas.push(operation);
                return Ok(());
            }
            Some("4") => {
                let position = range_position(&operation)?;
                self.remove_range(&operation, position, false, journal.as_deref_mut())?;
                self.deltas.push(operation);
                return Ok(());
            }
            _ => {}
        }

        let action = operation.get_identifiers().get(ACTION).map(String::as_str);
        let partial = matches!(action, Some("1" | "5"));
        let destination = LiveKey::of(&operation);
        // Walked only for the one statement it can refuse.
        let occupied_position = || {
            position_of(&operation).is_some_and(|position| {
                self.live().any(|held| {
                    same_partition(held, &operation) && position_of(held) == Some(position)
                })
            })
        };
        if action == Some("0")
            && operation.get_identifiers().contains_key(ENTRY_POSITION)
            && !operation.get_identifiers().contains_key(ENTRY_ID)
            && !operation.get_identifiers().contains_key(ENTRY_REF_ID)
            && occupied_position()
        {
            return Err(invalid(
                "$.operation.identifiers.MDEntryPositionNo",
                "a new anonymous entry cannot occupy an existing position; state MDEntryID or send a snapshot",
            ));
        }
        let referenced = self.referenced_identity_of(&operation).or_else(|| {
            let wanted = operation.get_identifiers().get(ENTRY_REF_ID)?;
            other_side
                .filter(|previous| {
                    same_partition(previous, &operation)
                        && previous.get_identifiers().get(ENTRY_ID) == Some(wanted)
                })
                .map(LiveKey::of)
        });
        if operation.get_identifiers().contains_key(ENTRY_REF_ID) && referenced.is_none() {
            return Err(invalid(
                "$.operation.identifiers.MDEntryRefID",
                "the referenced live entry does not exist in this symbol and scope",
            ));
        }
        let identity = referenced.unwrap_or_else(|| {
            other_side.map_or_else(|| self.identity_of(&operation), LiveKey::of)
        });
        if (identity != destination && self.index.positions.contains_key(&destination))
            || other_side.is_some_and(|previous| LiveKey::of(previous) != identity)
        {
            return Err(invalid(
                "$.operation.identifiers.MDEntryID",
                "the referenced entry and destination entry are both live",
            ));
        }
        let previous = self.get(&identity).or(other_side);
        if partial && previous.is_none() {
            if !operation.get_identifiers().contains_key(ENTRY_PRICE) {
                return Err(invalid(
                    "$.operation.identifiers.MDEntryPx",
                    "a partial update without a live predecessor must state its price",
                ));
            }
            if !operation.get_identifiers().contains_key(ENTRY_SIZE) {
                return Err(invalid(
                    "$.operation.identifiers.MDEntrySize",
                    "a partial update without a live predecessor must state its size",
                ));
            }
        }
        if let Some(previous) = previous {
            if let (Some(stated), Some(known)) = (
                operation.get_identifiers().get("OrderID"),
                previous.get_identifiers().get("OrderID"),
            ) {
                if stated != known {
                    return Err(invalid(
                        "$.operation.identifiers.OrderID",
                        format_smolstr!("expected the live order {known:?}, got {stated:?}"),
                    ));
                }
            }
            let kind = match (operation.kind(), previous.kind()) {
                (current, previous) if current == previous => current,
                (MarketOperationKind::Quote, MarketOperationKind::Order)
                    if matches!(action, Some("1" | "2" | "5"))
                        && !operation.get_identifiers().contains_key("OrderID") =>
                {
                    MarketOperationKind::Order
                }
                (MarketOperationKind::Order, MarketOperationKind::Quote)
                    if partial
                        && operation.get_identifiers().contains_key("OrderID")
                        && !previous.get_identifiers().contains_key("OrderID") =>
                {
                    MarketOperationKind::Order
                }
                (current, previous) => {
                    return Err(invalid(
                        "$.operation.kind",
                        format_smolstr!(
                            "expected a continuation of {}, got {}",
                            previous.as_str(),
                            current.as_str()
                        ),
                    ));
                }
            };
            if partial {
                if !operation.get_identifiers().contains_key(ENTRY_PRICE) {
                    operation.set_price(previous.get_price());
                }
                if !operation.get_identifiers().contains_key(ENTRY_SIZE) {
                    operation.set_quantity(previous.get_quantity());
                }
            }
            let mut event = match operation {
                MarketOperation::Order(order) => MarketEventData::from(order),
                MarketOperation::Quote(quote) => MarketEventData::from(quote),
                _ => unreachable!("a book side accepts only orders and quotes"),
            };
            let event = if event.get_curruuid() == previous.get_curruuid() {
                event.restating(previous.as_ref())
            } else {
                event
                    .clone()
                    .following_market(previous.as_ref())
                    .unwrap_or_else(|| {
                        event.finalize();
                        event
                    })
            };
            operation = match kind {
                MarketOperationKind::Order => Order::from(event).into(),
                MarketOperationKind::Quote => Quote::from(event).into(),
                _ => unreachable!("a live predecessor is an order or quote"),
            };
        }
        let Some(price) = BookPrice::of(operation.get_side(), operation.get_price()) else {
            return Err(invalid(
                "$.operation.side",
                format_smolstr!(
                    "expected the book side {:?}, got {:?}",
                    self.get_side().as_str(),
                    operation.get_side().as_str()
                ),
            ));
        };
        if let Some(previous) = self.take_removed(&identity) {
            if let Some(journal) = journal.as_deref_mut() {
                journal.removed.push(previous);
            }
        }
        if operation.get_state().is_live()
            && operation
                .get_exprtime()
                .is_none_or(|expiration| expiration > operation.get_currunix())
        {
            let identity = LiveKey::of(&operation);
            // A level is kept ordered by stated position, unpositioned
            // entries last, each in arrival order: an entry lands behind
            // every one its position does not precede.
            let level = self.levels.entry(price).or_default();
            let key = position_of(&operation).unwrap_or(u64::MAX);
            let at = level.partition_point(|held| position_of(held).unwrap_or(u64::MAX) <= key);
            if let Some(journal) = journal {
                journal.inserted.push(identity.clone());
            }
            self.insert_live(price, at, Arc::new(operation.clone()), identity);
        }
        self.deltas.push(operation);
        Ok(())
    }

    fn identity_of(&self, operation: &MarketOperation) -> LiveKey {
        let own = LiveKey::of(operation);
        if let Some(identity) = self.referenced_identity_of(operation) {
            return identity;
        }
        if self.index.positions.contains_key(&own) {
            return own;
        }
        operation
            .get_identifiers()
            .get(ENTRY_ID)
            .and_then(|wanted| self.find_entry_identity(operation, wanted))
            .unwrap_or(own)
    }

    fn referenced_identity_of(&self, operation: &MarketOperation) -> Option<LiveKey> {
        operation
            .get_identifiers()
            .get(ENTRY_REF_ID)
            .and_then(|wanted| self.find_entry_identity(operation, wanted))
    }

    fn find_entry_identity(&self, operation: &MarketOperation, wanted: &String) -> Option<LiveKey> {
        let key = EntryKey {
            partition: SnapshotPartition::of(operation),
            entry: SmolStr::new(wanted),
        };
        match self.index.entry_ids.get(&key)? {
            EntrySlot::One(identity) => Some(identity.clone()),
            EntrySlot::Many(_) => self.live().find_map(|held| {
                (same_partition(held, operation)
                    && held.get_identifiers().get(ENTRY_ID) == Some(wanted))
                .then(|| LiveKey::of(held))
            }),
        }
    }

    /// Stands `operation` live at `at` in the level of `price`, the index
    /// kept in step.
    fn insert_live(
        &mut self,
        price: BookPrice,
        at: usize,
        operation: Arc<MarketOperation>,
        identity: LiveKey,
    ) {
        let index = Arc::make_mut(&mut self.index);
        index.record(&operation, &identity);
        index.positions.insert(identity, price);
        self.levels.entry(price).or_default().insert(at, operation);
    }

    fn get(&self, identity: &LiveKey) -> Option<&MarketOperation> {
        let price = self.index.positions.get(identity)?;
        self.levels
            .get(price)?
            .iter()
            .find(|operation| LiveKey::of(operation) == *identity)
            .map(Arc::as_ref)
    }

    fn take_removed(&mut self, identity: &LiveKey) -> Option<RemovedLive> {
        let price = *self.index.positions.get(identity)?;
        let level = self.levels.get_mut(&price)?;
        let index = level
            .iter()
            .position(|operation| LiveKey::of(operation) == *identity)?;
        let operation = level.remove(index);
        if level.is_empty() {
            self.levels.remove(&price);
        }
        let held = Arc::make_mut(&mut self.index);
        held.positions.remove(identity);
        held.forget(&operation);
        Some(RemovedLive {
            identity: identity.clone(),
            price,
            index,
            operation,
        })
    }

    fn remove(&mut self, identity: &LiveKey) -> Option<Arc<MarketOperation>> {
        self.take_removed(identity).map(|removed| removed.operation)
    }

    fn remove_journaled(&mut self, identity: &LiveKey, journal: &mut SideJournal) -> bool {
        if let Some(removed) = self.take_removed(identity) {
            journal.removed.push(removed);
            true
        } else {
            false
        }
    }

    fn remove_range(
        &mut self,
        operation: &MarketOperation,
        position: usize,
        through: bool,
        mut journal: Option<&mut SideJournal>,
    ) -> Result<()> {
        let identities: Vec<LiveKey> = self
            .live()
            .filter(|held| same_partition(held, operation))
            .map(LiveKey::of)
            .collect();
        if position > identities.len() {
            return Err(invalid(
                "$.operation.identifiers.MDEntryPositionNo",
                format_smolstr!(
                    "expected a position from 1 through {}, got {position}",
                    identities.len()
                ),
            ));
        }
        let at = position - 1;
        for (index, identity) in identities.into_iter().enumerate() {
            if (through && index <= at) || (!through && index >= at) {
                if let Some(journal) = journal.as_deref_mut() {
                    let _ = self.remove_journaled(&identity, journal);
                } else {
                    self.remove(&identity);
                }
            }
        }
        Ok(())
    }

    fn clear_partition(&mut self, partition: &SnapshotPartition) -> bool {
        let identities: Vec<LiveKey> = self
            .live()
            .filter(|operation| SnapshotPartition::of(operation) == *partition)
            .map(LiveKey::of)
            .collect();
        let changed = !identities.is_empty();
        for identity in identities {
            self.remove(&identity);
        }
        changed
    }

    fn replace_membership(
        &mut self,
        snapshot: Vec<MarketOperation>,
        partitions: &BTreeSet<SnapshotPartition>,
    ) -> Result<()> {
        let mut live = self
            .levels
            .values()
            .flatten()
            .filter(|operation| !partitions.contains(&SnapshotPartition::of(operation)))
            .cloned()
            .collect::<Vec<_>>();
        live.extend(snapshot.into_iter().map(Arc::new));
        *self = Self::from_shared_parts(self.element.clone(), live, self.deltas.clone())?;
        Ok(())
    }

    pub(super) fn canonical_element(&self) -> Result<MarketElementData> {
        let mut element = self.element.clone();
        element.set_price(Decimal18::ZERO);
        element.set_quantity(Decimal18::ZERO);
        element.set_currency(Ccy::none());
        element.set_unit(String::new());
        element.set_bidpx(None);
        element.set_bidqty(None);
        element.set_bidcurrency(None);
        element.set_bidunit(None);
        element.set_askpx(None);
        element.set_askqty(None);
        element.set_askcurrency(None);
        element.set_askunit(None);
        if let Some((price, level)) = self.levels.first_key_value() {
            let quantity = level.iter().try_fold(Decimal18::ZERO, |sum, operation| {
                sum.checked_add(operation.get_quantity()).ok_or_else(|| {
                    invalid(
                        "$.quantity",
                        "aggregate best-level quantity exceeds decimal18",
                    )
                })
            })?;
            let first = &level[0];
            element.set_price(price.price());
            element.set_quantity(quantity);
            element.set_currency(first.get_currency().clone());
            element.set_unit(first.get_unit().to_owned());
            if element.get_side().is_bid() {
                element.set_bidpx(Some(price.price()));
                element.set_bidqty(Some(quantity));
                element.set_bidcurrency(Some(first.get_currency().clone()));
                element.set_bidunit(Some(first.get_unit().to_owned()));
            } else {
                element.set_askpx(Some(price.price()));
                element.set_askqty(Some(quantity));
                element.set_askcurrency(Some(first.get_currency().clone()));
                element.set_askunit(Some(first.get_unit().to_owned()));
            }
        }
        finalize_side_element(&mut element, &self.levels, self.len(), &self.deltas);
        Ok(element)
    }

    fn refresh(&mut self) -> Result<()> {
        self.element = self.canonical_element()?;
        Ok(())
    }

    fn clear_deltas(&mut self) {
        if self.deltas.is_empty() {
            return;
        }
        self.deltas.clear();
        self.finalize();
    }
}

fn finalize_side_element(
    element: &mut MarketElementData,
    levels: &BTreeMap<BookPrice, Vec<Arc<MarketOperation>>>,
    live_len: usize,
    deltas: &[MarketOperation],
) {
    element.fill_market();
    element.sync_cross();
    let mut digest = element.digest_market();
    // Two facts per entry of a side that may be thousands deep, staged so
    // the state reads them a chunk at a time.
    {
        let mut staged = super::element::Staged::new(&mut digest);
        staged.write(&(live_len as u64).to_be_bytes());
        for operation in levels.values().flatten() {
            staged.write(operation.kind().as_str().as_bytes());
            staged.write(&operation.get_curruuid().get().to_be_bytes());
        }
        staged.write(&(deltas.len() as u64).to_be_bytes());
        for operation in deltas {
            staged.write(operation.kind().as_str().as_bytes());
            staged.write(&operation.get_curruuid().get().to_be_bytes());
        }
    }
    let hashcode = digest.finish();
    element.set_currhashcode(hashcode);
    element.set_curruuid(Uuid::from_v8(u128::from(hashcode)));
    element.set_crossuuid(element.cross_uuid());
}

impl Default for BookSide {
    fn default() -> Self {
        Self::new(Side::read("Buy").expect("the shipped Buy side")).expect("Buy is a bid side")
    }
}

impl AsRef<MarketElementData> for BookSide {
    fn as_ref(&self) -> &MarketElementData {
        &self.element
    }
}

impl AsMut<MarketElementData> for BookSide {
    fn as_mut(&mut self) -> &mut MarketElementData {
        &mut self.element
    }
}

impl Element for BookSide {
    fn get_curruuid(&self) -> Uuid {
        self.element.get_curruuid()
    }

    fn set_curruuid(&mut self, uuid: Uuid) {
        self.element.set_curruuid(uuid);
    }

    fn get_crossuuid(&self) -> Uuid {
        self.element.get_crossuuid()
    }

    fn set_crossuuid(&mut self, uuid: Uuid) {
        self.element.set_crossuuid(uuid);
    }

    fn get_crosscode(&self) -> &str {
        self.element.get_crosscode()
    }

    fn set_crosscode(&mut self, code: String) {
        self.element.set_crosscode(code);
    }

    fn get_currhashcode(&self) -> u64 {
        self.element.get_currhashcode()
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.element.set_currhashcode(hashcode);
    }

    fn get_crosshashcode(&self) -> u64 {
        self.element.get_crosshashcode()
    }

    fn set_crosshashcode(&mut self, hashcode: u64) {
        self.element.set_crosshashcode(hashcode);
    }

    fn get_identifiers(&self) -> &BTreeMap<String, String> {
        self.element.get_identifiers()
    }

    fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) {
        self.element.set_identifiers(identifiers);
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        self.element.get_srcuuids()
    }

    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
        self.element.set_srcuuids(sources);
    }

    fn is_after(&self, other: &Self) -> bool {
        self.element.is_after(&other.element)
    }

    fn finalize(&mut self) {
        finalize_side_element(
            &mut self.element,
            &self.levels,
            self.index.positions.len(),
            &self.deltas,
        );
    }

    fn with_previous(mut self, previous: &Self) -> Option<Self> {
        if self.get_side() != previous.get_side() {
            return None;
        }
        self.element = self.element.with_previous(&previous.element)?;
        self.finalize();
        Some(self)
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        if self.get_side() != other.get_side() {
            return None;
        }
        let mut live = self.live().cloned().collect::<Vec<_>>();
        let mut live_keys = live.iter().map(LiveKey::of).collect::<HashSet<_>>();
        live.extend(
            other
                .live()
                .filter(|operation| live_keys.insert(LiveKey::of(operation)))
                .cloned(),
        );
        let mut deltas = self.deltas.clone();
        let mut delta_keys = deltas
            .iter()
            .map(|operation| (operation.kind(), operation.get_curruuid()))
            .collect::<HashSet<_>>();
        deltas.extend(
            other
                .deltas
                .iter()
                .filter(|operation| delta_keys.insert((operation.kind(), operation.get_curruuid())))
                .cloned(),
        );
        let element = self
            .element
            .clone()
            .merge_with(&other.element)
            .unwrap_or_else(|| self.element.clone());
        let merged = Self::from_parts(element, live, deltas).ok()?;
        (merged != self).then_some(merged)
    }
}

delegate_market_element!(BookSide, element, market_only);

/// One coherent view of a market at one exact nanosecond instant.
#[derive(Clone, Debug, PartialEq)]
pub struct Book {
    event: MarketEventData,
    bid: BookSide,
    ask: BookSide,
    executions: Vec<Execution>,
}

struct BookJournal {
    event: MarketEventData,
    bid: SideJournal,
    ask: SideJournal,
    executions_len: usize,
    cleared_executions: Option<Vec<Execution>>,
}

#[derive(Clone, Copy, Default)]
struct ChangedSides {
    bid: bool,
    ask: bool,
}

impl ChangedSides {
    fn include(&mut self, other: Self) {
        self.bid |= other.bid;
        self.ask |= other.ask;
    }
}

#[derive(Clone, Copy)]
struct EventBounds {
    seqnum: u64,
    creaunix: Option<i64>,
    recdunix: Option<i64>,
    execunix: Option<i64>,
}

impl EventBounds {
    fn of<E: Event + ?Sized>(event: &E) -> Self {
        Self {
            seqnum: event.get_seqnum(),
            creaunix: event.get_creaunix(),
            recdunix: event.get_recdunix(),
            execunix: event.get_execunix(),
        }
    }
}

impl BookJournal {
    fn new(book: &mut Book, clear_changes: bool) -> Self {
        let event = book.event.clone();
        let executions_len = book.executions.len();
        let cleared_executions = clear_changes.then(|| std::mem::take(&mut book.executions));
        let bid = SideJournal::new(&mut book.bid, clear_changes);
        let ask = SideJournal::new(&mut book.ask, clear_changes);
        Self {
            event,
            bid,
            ask,
            executions_len,
            cleared_executions,
        }
    }

    fn cleared_sides(&self) -> ChangedSides {
        ChangedSides {
            bid: self
                .bid
                .cleared_deltas
                .as_ref()
                .is_some_and(|deltas| !deltas.is_empty()),
            ask: self
                .ask
                .cleared_deltas
                .as_ref()
                .is_some_and(|deltas| !deltas.is_empty()),
        }
    }

    fn rollback(self, book: &mut Book) {
        self.bid.rollback(&mut book.bid);
        self.ask.rollback(&mut book.ask);
        if let Some(executions) = self.cleared_executions {
            book.executions = executions;
        } else {
            book.executions.truncate(self.executions_len);
        }
        book.event = self.event;
    }
}

impl Book {
    /// An empty symbol book at one nanosecond instant.
    #[must_use]
    pub fn new(unix: i64, symbol: impl Into<String>) -> Self {
        let symbol = symbol.into();
        let mut event = MarketEventData::at(unix);
        event.set_crosscode(symbol.clone());
        event.set_symbolticker((!symbol.is_empty()).then_some(symbol));
        event.set_state(State::from_spelling("New").expect("the shipped New state"));
        let mut book = Self {
            event,
            bid: BookSide::new(Side::read("Buy").expect("the shipped Buy side"))
                .expect("Buy is a bid"),
            ask: BookSide::new(Side::read("Sell").expect("the shipped Sell side"))
                .expect("Sell is an ask"),
            executions: Vec::new(),
        };
        book.finalize();
        book
    }

    /// Rebuilds one canonical book from serialized parts.
    pub(crate) fn from_parts(
        event: MarketEventData,
        bid: BookSide,
        ask: BookSide,
        executions: Vec<Execution>,
    ) -> Result<Self> {
        let mut book = Self {
            event,
            bid,
            ask,
            executions,
        };
        book.validate_parts()?;
        book.refresh()?;
        Ok(book)
    }

    pub(super) fn validate_parts(&self) -> Result<()> {
        if !self.bid.get_side().is_bid() {
            return Err(invalid("$.bid.side", "expected a bid side"));
        }
        if !self.ask.get_side().is_ask() {
            return Err(invalid("$.ask.side", "expected an ask side"));
        }
        for (index, execution) in self.executions.iter().enumerate() {
            if execution.get_currunix() != self.event.get_currunix() {
                return Err(invalid(
                    format_smolstr!("$.executions[{index}].currunix"),
                    format_smolstr!(
                        "expected {}, got {}",
                        self.event.get_currunix(),
                        execution.get_currunix()
                    ),
                ));
            }
        }
        let symbol = self.event.get_symbolticker();
        validate_symbols(symbol, "bid.live", self.bid.live())?;
        validate_symbols(symbol, "bid.deltas", self.bid.deltas().iter())?;
        validate_symbols(symbol, "ask.live", self.ask.live())?;
        validate_symbols(symbol, "ask.deltas", self.ask.deltas().iter())?;
        for (index, execution) in self.executions.iter().enumerate() {
            validate_symbol(symbol, execution, || {
                format_smolstr!("$.executions[{index}]")
            })?;
        }
        validate_component_times(self.event.get_currunix(), "bid.live", self.bid.live(), true)?;
        validate_component_times(
            self.event.get_currunix(),
            "bid.deltas",
            self.bid.deltas().iter(),
            false,
        )?;
        validate_component_times(self.event.get_currunix(), "ask.live", self.ask.live(), true)?;
        validate_component_times(
            self.event.get_currunix(),
            "ask.deltas",
            self.ask.deltas().iter(),
            false,
        )?;
        validate_component_times(
            self.event.get_currunix(),
            "executions",
            self.executions.iter(),
            false,
        )?;
        validate_propagation_bounds(&self.event, "bid.live", self.bid.live())?;
        validate_propagation_bounds(&self.event, "bid.deltas", self.bid.deltas().iter())?;
        validate_propagation_bounds(&self.event, "ask.live", self.ask.live())?;
        validate_propagation_bounds(&self.event, "ask.deltas", self.ask.deltas().iter())?;
        validate_propagation_bounds(&self.event, "executions", self.executions.iter())
    }

    #[must_use]
    pub const fn bid(&self) -> &BookSide {
        &self.bid
    }

    #[must_use]
    pub const fn ask(&self) -> &BookSide {
        &self.ask
    }

    #[must_use]
    pub fn executions(&self) -> &[Execution] {
        &self.executions
    }

    /// Whether the best bid is above the best ask.
    #[must_use]
    pub fn is_crossed(&self) -> bool {
        matches!(
            (self.bid.best_price(), self.ask.best_price()),
            (Some(bid), Some(ask)) if bid > ask
        )
    }

    /// The arithmetic midpoint of a coherent two-sided BBO.
    #[must_use]
    pub fn bbo_midpoint(&self) -> Option<Decimal18> {
        let (bid, ask) = (self.bid.best_price()?, self.ask.best_price()?);
        (bid <= ask).then(|| decimal_mean(bid, ask)).flatten()
    }

    /// The two-value median of the best bid and ask aggregate quantities.
    #[must_use]
    pub fn median_quantity(&self) -> Option<Decimal18> {
        median_quantity(self.bid.best_quantity(), self.ask.best_quantity())
    }

    fn cached_median_quantity(
        &self,
        bid: &MarketElementData,
        ask: &MarketElementData,
    ) -> Option<Decimal18> {
        median_quantity(
            (!self.bid.is_empty()).then(|| bid.get_quantity()),
            (!self.ask.is_empty()).then(|| ask.get_quantity()),
        )
    }

    /// Atomically applies all operations of one timestamp. Full-snapshot depth
    /// operations and controls first replace only their declared book scope;
    /// executions never control resting membership.
    pub fn add_operations<I>(&mut self, operations: I) -> Result<()>
    where
        I: IntoIterator<Item = MarketOperation>,
    {
        let mut operations: Vec<_> = operations.into_iter().collect();
        if operations.is_empty() {
            return Ok(());
        }
        let unix = operations[0].get_currunix();
        if operations
            .iter()
            .any(|operation| operation.get_currunix() != unix)
        {
            return Err(invalid(
                "$.operations",
                "expected every operation in one atomic group to have the same currunix",
            ));
        }
        if unix < self.event.get_currunix() {
            return Err(invalid(
                "$.operations",
                format_smolstr!(
                    "expected a timestamp at or after {}, got {unix}",
                    self.event.get_currunix()
                ),
            ));
        }
        if operations.len() == 1 && is_ordinary_update(&operations[0]) {
            return self.add_one_journaled(operations.pop().expect("one ordinary operation"), unix);
        }
        let mut next = self.clone();
        if next.event.get_currunix() != unix {
            next.clear_changes();
        }
        next.event.set_currunix(unix);
        next.event.set_snapunix(None);
        let partitions: BTreeSet<SnapshotPartition> = operations
            .iter()
            .filter(|operation| {
                operation.is_full_snapshot()
                    && !matches!(
                        operation.kind(),
                        MarketOperationKind::Execution | MarketOperationKind::Trade
                    )
            })
            .map(SnapshotPartition::of)
            .collect();
        let mut changed = ChangedSides::default();
        for partition in &partitions {
            changed.bid |= next.bid.clear_partition(partition);
            changed.ask |= next.ask.clear_partition(partition);
        }
        for operation in operations {
            changed.include(next.apply(operation)?);
        }
        next.record_snapshot_partitions(&partitions);
        next.refresh_changed(changed)?;
        *self = next;
        Ok(())
    }

    fn add_one_journaled(&mut self, operation: MarketOperation, unix: i64) -> Result<()> {
        let advancing = self.event.get_currunix() != unix;
        let mut journal = BookJournal::new(self, advancing);
        let mut changed = journal.cleared_sides();
        let result = (|| {
            if advancing {
                self.clear_changes();
            }
            self.event.set_currunix(unix);
            self.event.set_snapunix(None);
            changed.include(self.apply_journaled(operation, &mut journal)?);
            self.refresh_changed(changed)
        })();
        if let Err(error) = result {
            journal.rollback(self);
            return Err(error);
        }
        Ok(())
    }

    fn replace_snapshot_membership(
        &mut self,
        bid: Vec<MarketOperation>,
        ask: Vec<MarketOperation>,
        controls: &[MarketOperation],
        partitions: &BTreeSet<SnapshotPartition>,
        unix: i64,
    ) -> Result<()> {
        if unix < self.event.get_currunix() {
            return Err(invalid(
                "$.snapshot",
                format_smolstr!(
                    "expected a timestamp at or after {}, got {unix}",
                    self.event.get_currunix()
                ),
            ));
        }
        validate_component_times(unix, "snapshot.bid", bid.iter(), true)?;
        validate_component_times(unix, "snapshot.ask", ask.iter(), true)?;
        validate_component_times(unix, "snapshot.controls", controls.iter(), false)?;
        let mut next = self.clone();
        if next.event.get_currunix() != unix {
            next.clear_changes();
        }
        next.event.set_currunix(unix);
        next.event.set_snapunix(Some(unix));
        for operation in bid.iter().chain(&ask).chain(controls) {
            next.fold_bounds(EventBounds::of(operation));
        }
        next.bid.replace_membership(bid, partitions)?;
        next.ask.replace_membership(ask, partitions)?;
        next.record_snapshot_partitions(partitions);
        next.refresh_changed(ChangedSides::default())?;
        *self = next;
        Ok(())
    }

    fn apply(&mut self, operation: MarketOperation) -> Result<ChangedSides> {
        self.apply_inner(operation, None)
    }

    fn apply_journaled(
        &mut self,
        operation: MarketOperation,
        journal: &mut BookJournal,
    ) -> Result<ChangedSides> {
        self.apply_inner(operation, Some(journal))
    }

    fn apply_inner(
        &mut self,
        operation: MarketOperation,
        journal: Option<&mut BookJournal>,
    ) -> Result<ChangedSides> {
        let symbol = operation.get_symbolticker();
        if self.get_symbolticker() != Some(GLOBAL_SYMBOL)
            && symbol.is_some()
            && symbol != self.get_symbolticker()
        {
            return Err(invalid(
                "$.operation.symbolticker",
                format_smolstr!("expected {:?}, got {:?}", self.get_symbolticker(), symbol),
            ));
        }
        validate_component_times(
            self.event.get_currunix(),
            "operation",
            std::iter::once(&operation),
            false,
        )?;
        let mut bounds = EventBounds::of(&operation);
        let changed = match operation {
            MarketOperation::Execution(execution) => {
                self.executions.push(execution);
                ChangedSides::default()
            }
            MarketOperation::Trade(trade) => {
                self.executions.extend(trade.into_executions());
                ChangedSides::default()
            }
            MarketOperation::Snapshot(_) => ChangedSides::default(),
            operation if operation.get_side().is_bid() => {
                let opposite = self.ask.identity_of(&operation);
                let previous = self.ask.get(&opposite);
                let ask = if let Some(journal) = journal {
                    self.bid
                        .apply_inner(operation, previous, Some(&mut journal.bid))?;
                    self.ask.remove_journaled(&opposite, &mut journal.ask)
                } else {
                    self.bid.apply_inner(operation, previous, None)?;
                    self.ask.remove(&opposite).is_some()
                };
                bounds = EventBounds::of(
                    self.bid
                        .deltas()
                        .last()
                        .expect("an applied bid operation is recorded as a delta"),
                );
                ChangedSides { bid: true, ask }
            }
            operation if operation.get_side().is_ask() => {
                let opposite = self.bid.identity_of(&operation);
                let previous = self.bid.get(&opposite);
                let bid = if let Some(journal) = journal {
                    self.ask
                        .apply_inner(operation, previous, Some(&mut journal.ask))?;
                    self.bid.remove_journaled(&opposite, &mut journal.bid)
                } else {
                    self.ask.apply_inner(operation, previous, None)?;
                    self.bid.remove(&opposite).is_some()
                };
                bounds = EventBounds::of(
                    self.ask
                        .deltas()
                        .last()
                        .expect("an applied ask operation is recorded as a delta"),
                );
                ChangedSides { bid, ask: true }
            }
            operation => {
                return Err(invalid(
                    "$.operation.side",
                    format_smolstr!(
                        "expected a bid or ask operation, got {:?}",
                        operation.get_side().as_str()
                    ),
                ));
            }
        };
        self.fold_bounds(bounds);
        Ok(changed)
    }

    fn fold_bounds(&mut self, bounds: EventBounds) {
        self.event
            .set_seqnum(self.event.get_seqnum().max(bounds.seqnum));
        self.event
            .set_creaunix(earliest(self.event.get_creaunix(), bounds.creaunix));
        self.event
            .set_recdunix(earliest(self.event.get_recdunix(), bounds.recdunix));
        self.event
            .set_execunix(latest(self.event.get_execunix(), bounds.execunix));
    }

    pub(super) fn canonical_event(
        &self,
        bid: &MarketElementData,
        ask: &MarketElementData,
    ) -> MarketEventData {
        let mut event = self.event.clone();
        event.set_bidpx((!self.bid.is_empty()).then(|| bid.get_price()));
        event.set_bidqty((!self.bid.is_empty()).then(|| bid.get_quantity()));
        event.set_bidcurrency((!self.bid.is_empty()).then(|| bid.get_currency().clone()));
        event.set_bidunit((!self.bid.is_empty()).then(|| bid.get_unit().to_owned()));
        event.set_askpx((!self.ask.is_empty()).then(|| ask.get_price()));
        event.set_askqty((!self.ask.is_empty()).then(|| ask.get_quantity()));
        event.set_askcurrency((!self.ask.is_empty()).then(|| ask.get_currency().clone()));
        event.set_askunit((!self.ask.is_empty()).then(|| ask.get_unit().to_owned()));
        let midpoint = if self.is_crossed() {
            None
        } else {
            self.bbo_midpoint()
                .or_else(|| self.bid.best_price().or_else(|| self.ask.best_price()))
        };
        event.set_price(midpoint.unwrap_or(Decimal18::ZERO));
        event.set_quantity(
            self.cached_median_quantity(bid, ask)
                .unwrap_or(Decimal18::ZERO),
        );
        let currency = match (
            (!self.bid.is_empty()).then(|| bid.get_currency()),
            (!self.ask.is_empty()).then(|| ask.get_currency()),
        ) {
            (Some(bid), Some(ask)) if bid == ask => bid.clone(),
            (Some(currency), None) | (None, Some(currency)) => currency.clone(),
            _ => Ccy::none(),
        };
        event.set_currency(currency);
        let unit = match (
            (!self.bid.is_empty()).then(|| bid.get_unit()),
            (!self.ask.is_empty()).then(|| ask.get_unit()),
        ) {
            (Some(bid), Some(ask)) if bid == ask => bid.to_owned(),
            (Some(unit), None) | (None, Some(unit)) => unit.to_owned(),
            _ => String::new(),
        };
        event.set_unit(unit);
        finalize_book_event(&mut event, bid, ask, &self.executions);
        event
    }

    fn refresh_changed(&mut self, changed: ChangedSides) -> Result<()> {
        if changed.bid {
            self.bid.refresh()?;
        }
        if changed.ask {
            self.ask.refresh()?;
        }
        canonicalize_executions(&mut self.executions);
        self.event = self.canonical_event(self.bid.as_ref(), self.ask.as_ref());
        Ok(())
    }

    fn refresh(&mut self) -> Result<()> {
        self.refresh_changed(ChangedSides {
            bid: true,
            ask: true,
        })
    }

    fn clear_changes(&mut self) {
        self.bid.clear_deltas();
        self.ask.clear_deltas();
        self.executions.clear();
        self.clear_snapshot_partitions();
    }

    fn clear_snapshot_partitions(&mut self) {
        if !self
            .event
            .get_identifiers()
            .keys()
            .any(|name| name.starts_with(SNAPSHOT_SCOPE_PREFIX))
        {
            return;
        }
        let mut identifiers = self.event.get_identifiers().clone();
        identifiers.retain(|name, _| !name.starts_with(SNAPSHOT_SCOPE_PREFIX));
        self.event.set_identifiers(identifiers);
    }

    fn record_snapshot_partitions(&mut self, partitions: &BTreeSet<SnapshotPartition>) {
        if partitions.is_empty() {
            return;
        }
        let mut identifiers = self.event.get_identifiers().clone();
        for partition in partitions {
            let symbol = partition.symbol.as_deref();
            let encoded_symbol = symbol.map_or_else(
                || "N:".to_owned(),
                |symbol| format!("S{}:{symbol}", symbol.len()),
            );
            identifiers.insert(
                format!(
                    "{SNAPSHOT_SCOPE_PREFIX}{encoded_symbol}{}:{}",
                    partition.scope.len(),
                    partition.scope
                ),
                String::new(),
            );
        }
        self.event.set_identifiers(identifiers);
    }

    fn snapshot_partitions(&self) -> BTreeSet<SnapshotPartition> {
        self.event
            .get_identifiers()
            .keys()
            .filter_map(|name| decode_snapshot_partition(name))
            .collect()
    }

    fn set_book_time(&mut self, unix: i64, snapshot: bool) {
        self.event.set_currunix(unix);
        self.event.set_snapunix(snapshot.then_some(unix));
        self.finalize();
    }
}

fn median_quantity(bid: Option<Decimal18>, ask: Option<Decimal18>) -> Option<Decimal18> {
    match (bid, ask) {
        (Some(bid), Some(ask)) => decimal_mean(bid, ask),
        (Some(quantity), None) | (None, Some(quantity)) => Some(quantity),
        (None, None) => None,
    }
}

fn finalize_book_event(
    event: &mut MarketEventData,
    bid: &MarketElementData,
    ask: &MarketElementData,
    executions: &[Execution],
) {
    event.sync_cross();
    let mut digest = event.digest_market_event();
    digest.write(&event.get_currunix().to_be_bytes());
    for side in [bid, ask] {
        digest.write(&side.get_curruuid().get().to_be_bytes());
    }
    for execution in executions {
        digest.write(&execution.get_curruuid().get().to_be_bytes());
    }
    event.finalized(digest.finish());
}

fn canonicalize_executions(executions: &mut Vec<Execution>) {
    executions.sort_by_key(Element::get_curruuid);
    executions.dedup_by_key(|execution| execution.get_curruuid());
}

impl Default for Book {
    fn default() -> Self {
        Self::new(0, String::new())
    }
}

impl AsRef<MarketEventData> for Book {
    fn as_ref(&self) -> &MarketEventData {
        &self.event
    }
}

impl AsMut<MarketEventData> for Book {
    fn as_mut(&mut self) -> &mut MarketEventData {
        &mut self.event
    }
}

impl Element for Book {
    fn get_curruuid(&self) -> Uuid {
        self.event.get_curruuid()
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.event.set_curruuid(curruuid);
    }

    fn get_crossuuid(&self) -> Uuid {
        self.event.get_crossuuid()
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.event.set_crossuuid(crossuuid);
    }

    fn get_crosscode(&self) -> &str {
        self.event.get_crosscode()
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.event.set_crosscode(crosscode);
    }

    fn get_currhashcode(&self) -> u64 {
        self.event.get_currhashcode()
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.event.set_currhashcode(hashcode);
    }

    fn get_crosshashcode(&self) -> u64 {
        self.event.get_crosshashcode()
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.event.set_crosshashcode(crosshashcode);
    }

    fn get_identifiers(&self) -> &BTreeMap<String, String> {
        self.event.get_identifiers()
    }

    fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) {
        self.event.set_identifiers(identifiers);
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        self.event.get_srcuuids()
    }

    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
        self.event.set_srcuuids(sources);
    }

    fn is_after(&self, other: &Self) -> bool {
        self.get_currunix() > other.get_currunix()
            || self.get_currunix() == other.get_currunix()
                && self.get_crosscode() > other.get_crosscode()
    }

    fn finalize(&mut self) {
        finalize_book_event(
            &mut self.event,
            self.bid.as_ref(),
            self.ask.as_ref(),
            &self.executions,
        );
    }

    fn with_previous(mut self, previous: &Self) -> Option<Self> {
        if self.get_crosscode() != previous.get_crosscode()
            || self.get_currunix() < previous.get_currunix()
        {
            return None;
        }
        let event = self.event.clone().following_market(&previous.event)?;
        let authoritative = self.get_snapunix().is_some();
        let replaced = self.snapshot_partitions();
        if !authoritative {
            let bid = self.bid.deltas.clone();
            let ask = self.ask.deltas.clone();
            self.bid = previous.bid.clone();
            self.ask = previous.ask.clone();
            self.bid.clear_deltas();
            self.ask.clear_deltas();
            for partition in &replaced {
                let _ = self.bid.clear_partition(partition);
                let _ = self.ask.clear_partition(partition);
            }
            for operation in bid {
                self.bid.apply(operation).ok()?;
            }
            for operation in ask {
                self.ask.apply(operation).ok()?;
            }
            self.bid.refresh().ok()?;
            self.ask.refresh().ok()?;
        }
        self.event = event;
        self.clear_snapshot_partitions();
        self.record_snapshot_partitions(&replaced);
        self.refresh().ok()?;
        Some(self)
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        if self.get_crosscode() != other.get_crosscode()
            || self.get_currunix() != other.get_currunix()
        {
            return None;
        }
        let right = reference_clock(other) > reference_clock(&self);
        let (reference, supplement) = if right {
            (other, &self)
        } else {
            (&self, other)
        };
        let replaced = reference.snapshot_partitions();
        let authoritative = reference.get_snapunix().is_some();
        let bid = merge_book_side(&reference.bid, &supplement.bid, &replaced, authoritative)?;
        let ask = merge_book_side(&reference.ask, &supplement.ask, &replaced, authoritative)?;
        let mut executions = reference.executions.clone();
        if !authoritative {
            let mut execution_ids = executions
                .iter()
                .map(Element::get_curruuid)
                .collect::<HashSet<_>>();
            executions.extend(
                supplement
                    .executions
                    .iter()
                    .filter(|execution| execution_ids.insert(execution.get_curruuid()))
                    .cloned(),
            );
        }
        let mut event = reference.event.clone();
        super::element::merge_market_event_into_reference(&mut event, &supplement.event);
        let merged = Self::from_parts(event, bid, ask, executions).ok()?;
        (merged != self).then_some(merged)
    }
}

delegate_market_event!(Book, event, market_only);
delegate_market_event!(Book, event, event_only, false);

/// Books from a sorted operation stream, one per symbol and effective
/// timestamp, or one consolidated `GLOBAL` book. Each timestamp and symbol is
/// committed atomically across ordinary deltas, expirations, and explicit
/// snapshot membership.
pub struct BookIterator<I>
where
    I: Iterator,
    I::Item: Into<Result<MarketOperation>>,
{
    source: I,
    source_head: Option<Result<MarketOperation>>,
    source_exhausted: bool,
    books: BTreeMap<String, Book>,
    pending: VecDeque<Result<Book>>,
    global: bool,
    snapshot_ns: Option<i64>,
    next_snapshot: Option<i64>,
    expirations: BTreeSet<BookExpiration>,
    last_unix: Option<i64>,
    done: bool,
}

impl<I> BookIterator<I>
where
    I: Iterator,
    I::Item: Into<Result<MarketOperation>>,
{
    /// Opens a book walk over operations already sorted by their event order.
    /// Owned operations and their fallible counterparts are accepted directly;
    /// `snapshot_millis == 0` disables grid snapshots.
    pub fn new(operations: I, snapshot_millis: u64, global: bool) -> Result<Self> {
        let snapshot_ns = snapshot_millis
            .checked_mul(1_000_000)
            .and_then(|value| i64::try_from(value).ok())
            .ok_or_else(|| {
                invalid(
                    "$.snapshot_millis",
                    format_smolstr!(
                        "{snapshot_millis} milliseconds exceeds an i64 nanosecond grid"
                    ),
                )
            })?;
        Ok(Self {
            source: operations,
            source_head: None,
            source_exhausted: false,
            books: BTreeMap::new(),
            pending: VecDeque::new(),
            global,
            snapshot_ns: (snapshot_ns > 0).then_some(snapshot_ns),
            next_snapshot: None,
            expirations: BTreeSet::new(),
            last_unix: None,
            done: false,
        })
    }

    #[must_use]
    pub const fn global(&self) -> bool {
        self.global
    }

    fn fill_source_head(&mut self) {
        if self.source_head.is_some() || self.source_exhausted {
            return;
        }
        match self.source.next() {
            Some(operation) => self.source_head = Some(operation.into()),
            None => self.source_exhausted = true,
        }
    }

    fn peek_source(&mut self) -> Option<&MarketOperation> {
        self.fill_source_head();
        self.source_head
            .as_ref()
            .and_then(|operation| operation.as_ref().ok())
    }

    fn take_source(&mut self) -> MarketOperation {
        match self.source_head.take() {
            Some(Ok(operation)) => operation,
            Some(Err(_)) => unreachable!("a source error is never taken as an operation"),
            None => unreachable!("a source operation is taken only after it is peeked"),
        }
    }

    fn ensure_snapshot(&mut self, unix: i64) {
        if self.next_snapshot.is_none() {
            self.next_snapshot = self
                .snapshot_ns
                .and_then(|step| grid_at_or_after(unix, step));
        }
    }

    fn emit_snapshot(&mut self, snapshot: i64, failed: &HashSet<String>) {
        for (symbol, book) in &mut self.books {
            if failed.contains(symbol) {
                continue;
            }
            book.set_book_time(snapshot, true);
            self.pending.push_back(Ok(book.clone()));
            book.clear_changes();
        }
        self.next_snapshot = self.snapshot_ns.and_then(|step| snapshot.checked_add(step));
    }

    fn synchronize_expirations(&mut self, symbol: &str) {
        self.expirations
            .retain(|expiration| expiration.book != symbol);
        let Some(book) = self.books.get(symbol) else {
            return;
        };
        let mut scheduled = Vec::new();
        for (side, operations) in [
            (BookSideKind::Bid, book.bid.live()),
            (BookSideKind::Ask, book.ask.live()),
        ] {
            for operation in operations {
                let Some(unix) = operation.get_exprtime() else {
                    continue;
                };
                if unix <= book.get_currunix() {
                    continue;
                }
                scheduled.push(BookExpiration {
                    unix,
                    book: symbol.to_owned(),
                    side,
                    identity: LiveKey::of(operation),
                    generation: operation.get_curruuid(),
                });
            }
        }
        self.expirations.extend(scheduled);
    }

    fn take_expirations(&mut self, unix: i64) -> BTreeMap<String, Vec<MarketOperation>> {
        let mut expired = BTreeMap::<String, Vec<MarketOperation>>::new();
        while self
            .expirations
            .first()
            .is_some_and(|expiration| expiration.unix == unix)
        {
            let expiration = self
                .expirations
                .pop_first()
                .expect("the first expiration was present");
            let Some(book) = self.books.get(&expiration.book) else {
                continue;
            };
            let side = match expiration.side {
                BookSideKind::Bid => &book.bid,
                BookSideKind::Ask => &book.ask,
            };
            let Some(operation) = side.get(&expiration.identity) else {
                continue;
            };
            if operation.get_curruuid() != expiration.generation
                || operation.get_exprtime() != Some(unix)
                || !operation.get_state().is_live()
            {
                continue;
            }
            let mut operation = operation.clone();
            operation.set_currunix(unix);
            operation.set_state(State::read("expired").expect("the shipped expired state"));
            let mut identifiers = operation.get_identifiers().clone();
            identifiers.insert(ACTION.to_owned(), "2".to_owned());
            // The synthetic delete addresses the current generation, not
            // the reference a prior rename used to reach its predecessor.
            identifiers.remove(ENTRY_REF_ID);
            operation.set_identifiers(identifiers);
            operation.set_execunix(None);
            operation.set_recdunix(None);
            operation.set_snapunix(None);
            operation.finalize();
            expired.entry(expiration.book).or_default().push(operation);
        }
        expired
    }

    fn fill_pending(&mut self) -> bool {
        loop {
            if self.done {
                return false;
            }
            self.fill_source_head();
            if self
                .source_head
                .as_ref()
                .is_some_and(|operation| operation.is_err())
            {
                let error = match self.source_head.take() {
                    Some(Err(error)) => error,
                    _ => unreachable!("the source head was checked as an error"),
                };
                self.done = true;
                self.pending.push_back(Err(error));
                return true;
            }
            let source_unix = self.peek_source().map(effective_unix);
            let expiration_unix = self.expirations.first().map(|expiration| expiration.unix);
            let unix = match (source_unix, expiration_unix) {
                (Some(source), Some(expiration)) => source.min(expiration),
                (Some(unix), None) | (None, Some(unix)) => unix,
                (None, None) => {
                    self.done = true;
                    return false;
                }
            };

            if source_unix == Some(unix) && self.last_unix.is_some_and(|previous| unix < previous) {
                while self
                    .peek_source()
                    .is_some_and(|operation| effective_unix(operation) == unix)
                {
                    self.take_source();
                }
                self.pending.push_back(Err(invalid(
                    "$.operations",
                    format_smolstr!(
                        "expected a sorted operation timestamp at or after {}, got {unix}",
                        self.last_unix.expect("checked")
                    ),
                )));
                return true;
            }

            self.ensure_snapshot(unix);
            if let Some(snapshot) = self.next_snapshot.filter(|snapshot| *snapshot < unix) {
                self.emit_snapshot(snapshot, &HashSet::new());
                if !self.pending.is_empty() {
                    return true;
                }
                continue;
            }
            let at_grid = self.next_snapshot == Some(unix);

            let mut raw = self.take_expirations(unix);
            let mut touched = raw.keys().cloned().collect::<BTreeSet<_>>();
            let mut snapshot_bid: BTreeMap<String, Vec<MarketOperation>> = BTreeMap::new();
            let mut snapshot_ask: BTreeMap<String, Vec<MarketOperation>> = BTreeMap::new();
            let mut snapshot_controls: BTreeMap<String, Vec<MarketOperation>> = BTreeMap::new();
            let mut snapshot_partitions: BTreeMap<String, BTreeSet<SnapshotPartition>> =
                BTreeMap::new();
            let mut snapshot_views = HashSet::new();
            let mut source_errors = BTreeMap::new();

            if source_unix == Some(unix) {
                self.last_unix = Some(unix);
                while self
                    .peek_source()
                    .is_some_and(|operation| effective_unix(operation) == unix)
                {
                    let mut operation = self.take_source();
                    let symbol = if self.global {
                        GLOBAL_SYMBOL.to_owned()
                    } else if let Some(symbol) = operation.get_symbolticker() {
                        symbol.to_owned()
                    } else {
                        self.pending.push_back(Err(invalid(
                            "$.operation.symbolticker",
                            "expected a symbol outside global mode",
                        )));
                        continue;
                    };
                    touched.insert(symbol.clone());
                    if operation.get_snapunix().is_some() {
                        snapshot_views.insert(symbol.clone());
                        match operation.kind() {
                            MarketOperationKind::Order | MarketOperationKind::Quote => {
                                snapshot_partitions
                                    .entry(symbol.clone())
                                    .or_default()
                                    .insert(SnapshotPartition::of(&operation));
                                let entries = if operation.get_side().is_bid() {
                                    snapshot_bid.entry(symbol).or_default()
                                } else {
                                    snapshot_ask.entry(symbol).or_default()
                                };
                                entries.push(operation);
                            }
                            MarketOperationKind::Snapshot => {
                                snapshot_partitions
                                    .entry(symbol.clone())
                                    .or_default()
                                    .insert(SnapshotPartition::of(&operation));
                                snapshot_controls.entry(symbol).or_default().push(operation);
                            }
                            MarketOperationKind::Execution | MarketOperationKind::Trade => {
                                if let Err(error) = validate_component_times(
                                    unix,
                                    "snapshot.executions",
                                    std::iter::once(&operation),
                                    false,
                                ) {
                                    source_errors.entry(symbol).or_insert(error);
                                    continue;
                                }
                                match &mut operation {
                                    MarketOperation::Trade(trade) => {
                                        trade.rebase_currunix(unix);
                                    }
                                    _ => {
                                        operation.set_currunix(unix);
                                        operation.finalize();
                                    }
                                }
                                raw.entry(symbol).or_default().push(operation);
                            }
                        }
                    } else {
                        raw.entry(symbol).or_default().push(operation);
                    }
                }
            }

            let mut failed = HashSet::new();
            for symbol in &touched {
                // add_operations is already atomic. Only a group that also
                // replaces supplied membership needs an outer transaction.
                if !snapshot_partitions.contains_key(symbol) && !source_errors.contains_key(symbol)
                {
                    if let Some(book) = self.books.get_mut(symbol) {
                        if let Err(error) = raw
                            .remove(symbol)
                            .map_or(Ok(()), |operations| book.add_operations(operations))
                        {
                            self.pending.push_back(Err(error));
                            failed.insert(symbol.clone());
                        }
                        continue;
                    }
                }
                let mut next = self
                    .books
                    .get(symbol)
                    .cloned()
                    .unwrap_or_else(|| Book::new(unix, symbol.clone()));
                let mut result = if let Some(error) = source_errors.remove(symbol) {
                    Err(error)
                } else {
                    raw.remove(symbol)
                        .map_or(Ok(()), |operations| next.add_operations(operations))
                };
                if result.is_ok() {
                    if let Some(partitions) = snapshot_partitions.get(symbol) {
                        let bid = snapshot_bid.remove(symbol).unwrap_or_default();
                        let ask = snapshot_ask.remove(symbol).unwrap_or_default();
                        let controls = snapshot_controls.remove(symbol).unwrap_or_default();
                        result =
                            next.replace_snapshot_membership(bid, ask, &controls, partitions, unix);
                    }
                }
                match result {
                    Ok(()) => {
                        self.books.insert(symbol.clone(), next);
                    }
                    Err(error) => {
                        self.pending.push_back(Err(error));
                        failed.insert(symbol.clone());
                    }
                }
            }

            if at_grid {
                self.emit_snapshot(unix, &failed);
            } else {
                for symbol in &touched {
                    if failed.contains(symbol) {
                        continue;
                    }
                    let Some(book) = self.books.get_mut(symbol) else {
                        continue;
                    };
                    book.set_book_time(unix, snapshot_views.contains(symbol));
                    self.pending.push_back(Ok(book.clone()));
                    book.clear_changes();
                }
            }
            for symbol in &touched {
                self.synchronize_expirations(symbol);
            }
            if !self.pending.is_empty() {
                return true;
            }
        }
    }
}

impl<I> Iterator for BookIterator<I>
where
    I: Iterator,
    I::Item: Into<Result<MarketOperation>>,
{
    type Item = Result<Book>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(book) = self.pending.pop_front() {
                return Some(book);
            }
            if !self.fill_pending() {
                return None;
            }
        }
    }
}

impl<I> FusedIterator for BookIterator<I>
where
    I: FusedIterator,
    I::Item: Into<Result<MarketOperation>>,
{
}

fn effective_unix(operation: &MarketOperation) -> i64 {
    operation
        .get_snapunix()
        .unwrap_or_else(|| operation.get_currunix())
}

fn is_ordinary_update(operation: &MarketOperation) -> bool {
    operation.kind() != MarketOperationKind::Snapshot
        && !operation.is_full_snapshot()
        && !matches!(
            operation.get_identifiers().get(ACTION).map(String::as_str),
            Some("3" | "4")
        )
}

fn merge_book_side(
    reference: &BookSide,
    supplement: &BookSide,
    replaced: &BTreeSet<SnapshotPartition>,
    authoritative: bool,
) -> Option<BookSide> {
    let mut live = reference.live().cloned().collect::<Vec<_>>();
    let mut live_keys = live.iter().map(LiveKey::of).collect::<HashSet<_>>();
    live.extend(
        supplement
            .live()
            .filter(|operation| {
                !authoritative
                    && !replaced.contains(&SnapshotPartition::of(operation))
                    && live_keys.insert(LiveKey::of(operation))
            })
            .cloned(),
    );
    let mut deltas = reference.deltas.clone();
    let mut delta_keys = deltas
        .iter()
        .map(|operation| (operation.kind(), operation.get_curruuid()))
        .collect::<HashSet<_>>();
    deltas.extend(
        supplement
            .deltas
            .iter()
            .filter(|operation| {
                !authoritative
                    && !replaced.contains(&SnapshotPartition::of(operation))
                    && delta_keys.insert((operation.kind(), operation.get_curruuid()))
            })
            .cloned(),
    );
    let element = reference
        .element
        .clone()
        .merge_with(&supplement.element)
        .unwrap_or_else(|| reference.element.clone());
    BookSide::from_parts(element, live, deltas).ok()
}

fn decode_snapshot_partition(name: &str) -> Option<SnapshotPartition> {
    let encoded = name.strip_prefix(SNAPSHOT_SCOPE_PREFIX)?;
    let (symbol_header, rest) = encoded.split_once(':')?;
    let (symbol, rest) = if symbol_header == "N" {
        (None, rest)
    } else {
        let length = symbol_header.strip_prefix('S')?.parse::<usize>().ok()?;
        let symbol = rest.get(..length)?;
        let rest = rest.get(length..)?;
        (Some(SmolStr::new(symbol)), rest)
    };
    let (scope_length, scope) = rest.split_once(':')?;
    (scope_length.parse::<usize>().ok()? == scope.len()).then(|| SnapshotPartition {
        symbol,
        scope: SmolStr::new(scope),
    })
}

fn position_of(operation: &MarketOperation) -> Option<u64> {
    operation
        .get_identifiers()
        .get(ENTRY_POSITION)
        .and_then(|value| value.parse().ok())
}

fn range_position(operation: &MarketOperation) -> Result<usize> {
    let Some(value) = operation.get_identifiers().get(ENTRY_POSITION) else {
        return Err(invalid(
            "$.operation.identifiers.MDEntryPositionNo",
            "expected a positive position for delete-through or delete-from",
        ));
    };
    value
        .parse::<usize>()
        .ok()
        .filter(|position| *position > 0)
        .ok_or_else(|| {
            invalid(
                "$.operation.identifiers.MDEntryPositionNo",
                format_smolstr!("expected a positive position, got {value:?}"),
            )
        })
}

fn same_partition(left: &MarketOperation, right: &MarketOperation) -> bool {
    left.scope() == right.scope() && left.get_symbolticker() == right.get_symbolticker()
}

fn grid_at_or_after(unix: i64, step: i64) -> Option<i64> {
    let remainder = unix.rem_euclid(step);
    if remainder == 0 {
        Some(unix)
    } else {
        unix.checked_add(step - remainder)
    }
}

fn reference_clock<E: Event>(event: &E) -> (Option<i64>, i64) {
    (event.get_recdunix(), event.get_currunix())
}

fn earliest(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn latest(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn decimal_mean(left: Decimal18, right: Decimal18) -> Option<Decimal18> {
    let left = left.units();
    let right = right.units();
    let units = left
        .checked_div(2)?
        .checked_add(right.checked_div(2)?)?
        .checked_add((left % 2).checked_add(right % 2)?.checked_div(2)?)?;
    Decimal18::from_units(units)
}

fn validate_component_times<'a, E, I>(
    book_unix: i64,
    name: &str,
    operations: I,
    live: bool,
) -> Result<()>
where
    E: Event + ?Sized + 'a,
    I: IntoIterator<Item = &'a E>,
{
    for (index, operation) in operations.into_iter().enumerate() {
        let path = |field: &str| format_smolstr!("$.{name}[{index}].{field}");
        if operation.get_currunix() > book_unix {
            return Err(invalid(
                path("currunix"),
                format_smolstr!(
                    "expected a component timestamp at or before {book_unix}, got {}",
                    operation.get_currunix()
                ),
            ));
        }
        if operation
            .get_snapunix()
            .is_some_and(|snapshot| snapshot > book_unix)
        {
            return Err(invalid(
                path("snapunix"),
                format_smolstr!(
                    "expected an effective timestamp at or before {book_unix}, got {:?}",
                    operation.get_snapunix()
                ),
            ));
        }
        if live
            && operation
                .get_exprtime()
                .is_some_and(|expiration| expiration <= book_unix)
        {
            return Err(invalid(
                path("exprtime"),
                format_smolstr!(
                    "expected a live expiration after {book_unix}, got {:?}",
                    operation.get_exprtime()
                ),
            ));
        }
    }
    Ok(())
}

fn validate_propagation_bounds<'a, E, I>(
    event: &MarketEventData,
    name: &str,
    operations: I,
) -> Result<()>
where
    E: Event + ?Sized + 'a,
    I: IntoIterator<Item = &'a E>,
{
    for (index, operation) in operations.into_iter().enumerate() {
        let path = |field: &str| format_smolstr!("$.{name}[{index}].{field}");
        if event.get_seqnum() < operation.get_seqnum() {
            return Err(invalid(
                path("seqnum"),
                format_smolstr!(
                    "expected at least {}, got {}",
                    operation.get_seqnum(),
                    event.get_seqnum()
                ),
            ));
        }
        validate_earliest_bound(
            event.get_creaunix(),
            operation.get_creaunix(),
            path("creaunix"),
        )?;
        validate_earliest_bound(
            event.get_recdunix(),
            operation.get_recdunix(),
            path("recdunix"),
        )?;
        validate_latest_bound(
            event.get_execunix(),
            operation.get_execunix(),
            path("execunix"),
        )?;
    }
    Ok(())
}

fn validate_earliest_bound(root: Option<i64>, component: Option<i64>, path: SmolStr) -> Result<()> {
    if let Some(component) = component {
        if root.is_none_or(|root| root > component) {
            return Err(invalid(
                path,
                format_smolstr!("expected the book bound at or before {component}, got {root:?}"),
            ));
        }
    }
    Ok(())
}

fn validate_latest_bound(root: Option<i64>, component: Option<i64>, path: SmolStr) -> Result<()> {
    if let Some(component) = component {
        if root.is_none_or(|root| root < component) {
            return Err(invalid(
                path,
                format_smolstr!("expected the book bound at or after {component}, got {root:?}"),
            ));
        }
    }
    Ok(())
}

fn validate_symbols<'a, E, I>(book_symbol: Option<&str>, name: &str, operations: I) -> Result<()>
where
    E: MarketElement + ?Sized + 'a,
    I: IntoIterator<Item = &'a E>,
{
    for (index, operation) in operations.into_iter().enumerate() {
        validate_symbol(book_symbol, operation, || {
            format_smolstr!("$.{name}[{index}]")
        })?;
    }
    Ok(())
}

fn validate_symbol<E: MarketElement + ?Sized>(
    book_symbol: Option<&str>,
    operation: &E,
    path: impl FnOnce() -> SmolStr,
) -> Result<()> {
    let operation_symbol = operation.get_symbolticker();
    if book_symbol != Some(GLOBAL_SYMBOL)
        && operation_symbol.is_some()
        && operation_symbol != book_symbol
    {
        return Err(invalid(
            path(),
            format_smolstr!("expected symbol {book_symbol:?}, got {operation_symbol:?}"),
        ));
    }
    Ok(())
}

fn invalid(path: impl Into<SmolStr>, reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: path.into(),
        reason: reason.into(),
    }
}
