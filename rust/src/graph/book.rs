//! Typed market data and a stateful, price-ordered market book.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::Hasher;
use std::iter::FusedIterator;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::arrow::{ALIVE, DELTAS};
use super::facts::{MarketEventFacts, OperationEventFacts};
use super::market::merge_market_event_into_reference;
use super::market_data::MarketData;
use super::operation::{BookRef, ExecutionEvent, MdUpdateAction, OrderKind, QuoteKind};
use super::{Element, Event, Market, Operation};
use crate::warning::warned;
use crate::xxhash::Xxh3;
use crate::{Ccy, Decimal, Error, IdType, Limit, Result, Side, State, Unit, Uuid};

/// The identifier type an entry's own `MDEntryID(278)` is held under.
pub const ENTRY_ID: IdType = IdType::MdEntryId;
/// The identifier type an entry's `MDEntryRefID(280)` is held under.
pub const ENTRY_REF_ID: IdType = IdType::MdEntryRefId;
/// The identifier type an order's `OrderID(37)` is held under.
const ORDER_ID: IdType = IdType::OrderId;

/// A full-snapshot control: the event a `W` message is, with the scope it
/// replaces, and no operation of its own.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotEvent {
    event: MarketEventFacts,
    book: BookRef,
}

impl SnapshotEvent {
    /// A snapshot control for `scope` over the event and market facts
    /// `event` states - its instant, its symbol, its identity - and none of
    /// its operation facts, finalized.
    #[must_use]
    pub fn snapshot<E: Event + Market + ?Sized>(event: &E, scope: Option<SmolStr>) -> Self {
        Self::from_facts(MarketEventFacts::from(event), scope)
    }

    /// [`Self::snapshot`] over facts already held: a move.
    pub(crate) fn from_facts(event: MarketEventFacts, scope: Option<SmolStr>) -> Self {
        Self::from_control(
            event,
            BookRef {
                scope,
                ..BookRef::default()
            },
        )
    }

    /// A snapshot control over facts already held and the control facts a
    /// row stated beside them, its action the snapshot's, finalized.
    pub(crate) fn from_control(mut event: MarketEventFacts, book: BookRef) -> Self {
        // A snapshot control is not sided: its cross code stays as given.
        event.set_marketdatakind(crate::MarketDataKind::Book);
        let mut control = Self {
            event,
            book: BookRef {
                action: Some(MdUpdateAction::Snapshot),
                ..book
            },
        };
        control.finalize();
        control
    }

    /// The event this control is.
    pub(crate) fn event(&self) -> &MarketEventFacts {
        &self.event
    }

    /// The scope this control replaces.
    #[must_use]
    pub fn book(&self) -> &BookRef {
        &self.book
    }
}

impl Element for SnapshotEvent {
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
    fn get_srcuuids(&self) -> &[Uuid] {
        self.event.get_srcuuids()
    }
    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
        self.event.set_srcuuids(sources);
    }
    fn is_after(&self, other: &Self) -> bool {
        self.event.is_after(&other.event)
    }
    fn finalize(&mut self) {
        self.event.finalize();
    }
    fn with_previous(mut self, previous: &Self) -> Option<Self> {
        self.event = self.event.clone().with_previous(&previous.event)?;
        self.finalize();
        Some(self)
    }
    fn merge_with(mut self, other: &Self) -> Option<Self> {
        self.event = self.event.clone().merge_with(&other.event)?;
        self.finalize();
        Some(self)
    }
}

delegate_market!(SnapshotEvent, event);
delegate_event!(
    SnapshotEvent,
    event,
    restating = |mut this: SnapshotEvent, live: &SnapshotEvent| {
        this.event = this.event.restating(&live.event);
        this.finalize();
        this
    },
    is_execution = |_: &SnapshotEvent| false,
    set_currunix = |this: &mut SnapshotEvent, unix: i64| this.event.set_currunix(unix)
);

/// Where a level stands on its side. The derived order is the side's: the
/// best bid or ask first, and `Unpriced` - declared last - after every
/// priced level on either side.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum BookPrice {
    Bid(Reverse<Decimal>),
    Ask(Decimal),
    /// The one level every entry stating no price rests at.
    Unpriced,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct LiveKey {
    identity: Uuid,
    symbol: Option<SmolStr>,
    scope: SmolStr,
}

/// One scope a full snapshot replaces: a symbol - none where the
/// operations state no ticker - and the book scope the entries stated. Walk
/// state: a book remembers the scopes its last snapshot replaced while the
/// walk folds it, and no row states them.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct SnapshotPartition {
    /// The symbol the snapshot is for, where the operations name one.
    symbol: Option<SmolStr>,
    /// The book scope, empty where the operations state none.
    scope: SmolStr,
}

impl SnapshotPartition {
    fn at(symbol: Option<&str>, scope: &str) -> Self {
        Self {
            symbol: symbol.map(SmolStr::new),
            scope: SmolStr::new(scope),
        }
    }

    fn of(operation: &MarketData) -> Self {
        Self::at(operation.get_ticker(), scope_of(operation))
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
    fn of(operation: &MarketData) -> Self {
        Self {
            identity: operation.get_crossuuid(),
            symbol: operation.get_ticker().map(SmolStr::new),
            scope: SmolStr::new(scope_of(operation)),
        }
    }
}

impl BookPrice {
    /// The level an entry of `side` stating `price` rests at; none for a
    /// side that is neither a bid nor an ask.
    fn of(side: Side, price: Option<Decimal>) -> Option<Self> {
        if !side.is_bid() && !side.is_ask() {
            return None;
        }
        Some(match price {
            None => Self::Unpriced,
            Some(price) if side.is_bid() => Self::Bid(Reverse(price)),
            Some(price) => Self::Ask(price),
        })
    }

    const fn price(self) -> Option<Decimal> {
        match self {
            Self::Bid(Reverse(price)) | Self::Ask(price) => Some(price),
            Self::Unpriced => None,
        }
    }
}

/// A level's quantity: the exact sum of what its entries state, an entry
/// stating none adding nothing; `None` past decimal. The one place a level
/// is summed.
/// Whether a level can trade: one of its entries does not state
/// `tradable = false` - an entry stating nothing is a live order no venue
/// halted, so only a level every entry of which states `false` cannot.
fn is_tradable(level: &[Arc<MarketData>]) -> bool {
    level
        .iter()
        .any(|operation| operation.operation_event().get_tradable() != Some(false))
}

fn level_quantity(level: &[Arc<MarketData>]) -> Option<Decimal> {
    level.iter().try_fold(Decimal::ZERO, |sum, operation| {
        sum.checked_add(operation.get_quantity().unwrap_or(Decimal::ZERO))
    })
}

/// The book scope an entry states, empty where it states none: the seam
/// over [`MarketData::book`] every comparison of a book's entries reads
/// through.
fn scope_of(operation: &MarketData) -> &str {
    operation
        .book()
        .and_then(|book| book.scope.as_deref())
        .unwrap_or("")
}

/// Whether the entry is currently typed as a dated order (versus a quote).
fn is_order(operation: &MarketData) -> bool {
    matches!(operation, MarketData::OrderEvent(_))
}

/// This entry's facts and book control, reinterpreted as a dated order -
/// the book's own quote-to-order continuation - keeping this operation's own
/// book control and taking `data` as the merged facts.
fn promote_to_order(operation: MarketData, data: OperationEventFacts) -> MarketData {
    match operation {
        MarketData::OrderEvent(event) => MarketData::OrderEvent(event.with_kind::<OrderKind>(data)),
        MarketData::QuoteEvent(event) => MarketData::OrderEvent(event.with_kind::<OrderKind>(data)),
        _ => unreachable!("a book holds alive only order or quote events"),
    }
}

/// [`promote_to_order`], to a quote.
fn promote_to_quote(operation: MarketData, data: OperationEventFacts) -> MarketData {
    match operation {
        MarketData::OrderEvent(event) => MarketData::QuoteEvent(event.with_kind::<QuoteKind>(data)),
        MarketData::QuoteEvent(event) => MarketData::QuoteEvent(event.with_kind::<QuoteKind>(data)),
        _ => unreachable!("a book holds alive only order or quote events"),
    }
}

/// One side of a book as the walk keeps it: persistent live orders and
/// quotes, price ordered, beside the deltas applied since the last emitted
/// book. Walk state rather than a value: a book answers a side as its
/// price levels through [`BookEvent::limits`] and its entries through
/// [`BookEvent::alive`] and [`BookEvent::deltas`].
///
/// A live entry is shared: a book emitted per instant is a clone of the one
/// the walk keeps, and the entries an update did not touch are the same
/// entries in both, so emitting a deep book costs a reference count per
/// entry rather than a copy of each.
#[derive(Clone, Debug)]
struct Ladder {
    /// `BUYS` for the bid side, `SELL` for the ask side.
    side: Side,
    levels: BTreeMap<BookPrice, Vec<Arc<MarketData>>>,
    /// Derived from `levels` and kept in step by every change, shared with
    /// clones until one changes: a book emitted per instant copies no index,
    /// and the side the walk keeps changes its own in place once the
    /// emitted book is gone.
    index: Arc<SideIndex>,
    deltas: Vec<MarketData>,
    /// The digest of what the side holds, alive and applied, kept in step by
    /// every refresh: a book finalized per instant reads one word per side
    /// rather than walking a side thousands deep.
    hashcode: u64,
}

/// Two sides are equal by what they hold; the index and the digest are
/// derived from it.
impl PartialEq for Ladder {
    fn eq(&self, other: &Self) -> bool {
        self.side == other.side && self.levels == other.levels && self.deltas == other.deltas
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
    fn record(&mut self, operation: &MarketData, identity: &LiveKey) {
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
    fn forget(&mut self, operation: &MarketData) {
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
    fn of(operation: &MarketData) -> Option<Self> {
        let entry = operation
            .operation_event()
            .get_identifiers()
            .get(&ENTRY_ID)?;
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
    operation: Arc<MarketData>,
}

struct SideJournal {
    hashcode: u64,
    deltas_len: usize,
    cleared_deltas: Option<Vec<MarketData>>,
    inserted: Vec<LiveKey>,
    removed: Vec<RemovedLive>,
}

impl SideJournal {
    fn new(side: &mut Ladder, clear_deltas: bool) -> Self {
        let deltas_len = side.deltas.len();
        let cleared_deltas = clear_deltas.then(|| std::mem::take(&mut side.deltas));
        Self {
            hashcode: side.hashcode,
            deltas_len,
            cleared_deltas,
            inserted: Vec::new(),
            removed: Vec::new(),
        }
    }

    fn rollback(self, side: &mut Ladder) {
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
        side.hashcode = self.hashcode;
    }
}

impl Ladder {
    /// An empty side: `BUYS` is the bid, `SELL` the ask.
    fn new(side: Side) -> Self {
        let mut ladder = Self {
            side,
            levels: BTreeMap::new(),
            index: Arc::default(),
            deltas: Vec::new(),
            hashcode: 0,
        };
        ladder.rehash();
        ladder
    }

    /// Rebuilds one canonical side from its entries - each beside its
    /// position in the book's `alive` or `deltas` list, which a refusal
    /// names - without replaying the deltas as fresh mutations.
    fn from_shared_parts(
        side: Side,
        live: Vec<(usize, Arc<MarketData>)>,
        deltas: Vec<(usize, MarketData)>,
    ) -> Result<Self> {
        let mut ladder = Self {
            side,
            levels: BTreeMap::new(),
            index: Arc::default(),
            deltas: Vec::new(),
            hashcode: 0,
        };
        let mut built = SideIndex {
            positions: HashMap::with_capacity(live.len()),
            entry_ids: HashMap::new(),
        };
        for (index, operation) in live {
            let path = format_smolstr!("$.{ALIVE}[{index}]");
            ladder.validate_component(&operation, &path, true)?;
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
            ladder.levels.entry(price).or_default().push(operation);
        }
        ladder.index = Arc::new(built);
        for level in ladder.levels.values_mut() {
            if level.iter().any(|held| position_of(held).is_some()) {
                level.sort_by_key(|held| position_of(held).unwrap_or(u64::MAX));
            }
        }
        let mut held = Vec::with_capacity(deltas.len());
        for (index, operation) in deltas {
            ladder.validate_component(
                &operation,
                &format_smolstr!("$.{DELTAS}[{index}]"),
                false,
            )?;
            held.push(operation);
        }
        ladder.deltas = held;
        ladder.refresh()?;
        Ok(ladder)
    }

    /// The live orders and quotes, best price first and every entry
    /// stating no price last.
    fn live(&self) -> impl Iterator<Item = &MarketData> {
        self.levels
            .values()
            .flat_map(|level| level.iter().map(Arc::as_ref))
    }

    fn len(&self) -> usize {
        self.index.positions.len()
    }

    /// The best tradable price on this side: the first priced level's that
    /// [`Self::limits`] answers tradable, `None` for an empty side, one
    /// holding only unpriced entries, or one no level of which can trade -
    /// never a price that cannot be traded.
    fn best_price(&self) -> Option<Decimal> {
        self.best_level().map(|(price, _)| price)
    }

    /// The aggregate quantity at the best price: the exact sum of what the
    /// entries of the first tradable priced level state, an entry stating
    /// none adding nothing; `None` where [`Self::best_price`] is.
    fn best_quantity(&self) -> Option<Decimal> {
        self.best_level().map(|(_, level)| {
            // `check_levels` refuses a level past decimal at every refresh,
            // and every change refreshes or rolls back.
            level_quantity(level)
                .expect("a refreshed side holds no level whose quantity overflows decimal")
        })
    }

    /// The first entry at the first priced level, tradable or not: the one
    /// whose currency and unit the side speaks for the book in.
    fn best_entry(&self) -> Option<&MarketData> {
        let (key, level) = self.levels.first_key_value()?;
        key.price().map(|_| level[0].as_ref())
    }

    /// The first priced level that can trade: the first key a level's
    /// entries fold tradable under [`Self::limits`]' rule, the unpriced
    /// level sorting after every priced one.
    fn best_level(&self) -> Option<(Decimal, &[Arc<MarketData>])> {
        self.levels.iter().find_map(|(key, level)| {
            let price = key.price()?;
            is_tradable(level).then_some((price, level.as_slice()))
        })
    }

    /// One [`Limit`] per level, best first and the unpriced limit last:
    /// its price, the exact sum of its entries' quantities, and their
    /// `curruuid`s in position order, ties in arrival order, and whether
    /// any of them does not state `tradable = false` - an entry stating
    /// nothing trades. One pass over the levels in the
    /// order they are held, allocating each limit's `uuids` and nothing
    /// else.
    fn limits(&self) -> impl Iterator<Item = Limit> + '_ {
        self.levels.iter().map(|(key, level)| Limit {
            price: key.price(),
            quantity: level_quantity(level)
                .expect("a refreshed side holds no level whose quantity overflows decimal"),
            uuids: level
                .iter()
                .map(|operation| operation.get_curruuid())
                .collect(),
            tradable: is_tradable(level),
        })
    }

    /// The exact sum of the first `levels` limits' quantities in
    /// [`Self::limits`] order, the unpriced limit counted where it is
    /// reached: zero for an empty side or no level, `None` only past
    /// decimal.
    fn depth(&self, levels: usize) -> Option<Decimal> {
        self.levels
            .values()
            .take(levels)
            .try_fold(Decimal::ZERO, |sum, level| {
                sum.checked_add(level_quantity(level)?)
            })
    }

    fn validate_component(&self, operation: &MarketData, path: &str, live: bool) -> Result<()> {
        if !matches!(
            operation,
            MarketData::OrderEvent(_) | MarketData::QuoteEvent(_)
        ) {
            return Err(invalid(path, "expected an order or quote on a book side"));
        }
        if live && !operation.operation_event().get_state().is_live() {
            return Err(invalid(
                format_smolstr!("{path}.state"),
                "expected every live operation to have a live state",
            ));
        }
        if operation.get_side().is_bid() != self.side.is_bid()
            || (!operation.get_side().is_bid() && !operation.get_side().is_ask())
        {
            return Err(invalid(
                format_smolstr!("{path}.side"),
                format_smolstr!(
                    "expected {:?}, got {:?}",
                    self.side.as_str(),
                    operation.get_side().as_str()
                ),
            ));
        }
        Ok(())
    }

    fn apply(&mut self, operation: MarketData) -> Result<()> {
        self.apply_inner(operation, None, None)
    }

    fn apply_inner(
        &mut self,
        mut operation: MarketData,
        other_side: Option<&MarketData>,
        mut journal: Option<&mut SideJournal>,
    ) -> Result<()> {
        if !matches!(
            operation,
            MarketData::OrderEvent(_) | MarketData::QuoteEvent(_)
        ) {
            return Err(invalid(
                "$.operation",
                "expected an order or quote on a book side",
            ));
        }
        if operation.get_side().is_bid() != self.side.is_bid() {
            return Err(invalid(
                "$.operation.side",
                format_smolstr!(
                    "expected {:?}, got {:?}",
                    self.side.as_str(),
                    operation.get_side().as_str()
                ),
            ));
        }

        let action = operation.operation_event().control_action();
        if let Some(action) = action.filter(|action| action.is_range_delete()) {
            let position = range_position(&operation)?;
            let through = action == MdUpdateAction::DeleteThru;
            self.remove_range(&operation, position, through, journal.as_deref_mut())?;
            self.deltas.push(operation);
            return Ok(());
        }

        let partial = action.is_some_and(MdUpdateAction::is_partial);
        let destination = LiveKey::of(&operation);
        // Walked only for the one statement it can refuse.
        let occupied_position = || {
            position_of(&operation).is_some_and(|position| {
                self.live().any(|held| {
                    same_partition(held, &operation) && position_of(held) == Some(position)
                })
            })
        };
        if action == Some(MdUpdateAction::New)
            && position_of(&operation).is_some()
            && !operation
                .operation_event()
                .get_identifiers()
                .contains_kind(&ENTRY_ID)
            && !operation
                .operation_event()
                .get_identifiers()
                .contains_kind(&ENTRY_REF_ID)
            && occupied_position()
        {
            return Err(invalid(
                "$.operation.book.position",
                "a new anonymous entry cannot occupy an existing position; state MDEntryID or send a snapshot",
            ));
        }
        let referenced = self.referenced_identity_of(&operation).or_else(|| {
            let wanted = operation
                .operation_event()
                .get_identifiers()
                .get(&ENTRY_REF_ID)?;
            other_side
                .filter(|previous| {
                    same_partition(previous, &operation)
                        && previous.operation_event().get_identifiers().get(&ENTRY_ID)
                            == Some(wanted)
                })
                .map(LiveKey::of)
        });
        if operation
            .operation_event()
            .get_identifiers()
            .contains_kind(&ENTRY_REF_ID)
            && referenced.is_none()
        {
            return Err(invalid(
                "$.operation.identifiers.mdentryrefid",
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
                "$.operation.identifiers.mdentryid",
                "the referenced entry and destination entry are both live",
            ));
        }
        let previous = self.get(&identity).or(other_side);
        if partial && previous.is_none() {
            if entry_px_of(&operation).is_none() {
                return Err(invalid(
                    "$.operation.book.entry_px",
                    "a partial update without a live predecessor must state its price",
                ));
            }
            if entry_size_of(&operation).is_none() {
                return Err(invalid(
                    "$.operation.book.entry_size",
                    "a partial update without a live predecessor must state its size",
                ));
            }
        }
        if let Some(previous) = previous {
            if let (Some(stated), Some(known)) = (
                operation.operation_event().get_identifiers().get(&ORDER_ID),
                previous.operation_event().get_identifiers().get(&ORDER_ID),
            ) && stated != known
            {
                return Err(invalid(
                    "$.operation.identifiers.orderid",
                    format_smolstr!("expected the live order {known:?}, got {stated:?}"),
                ));
            }
            let promote_order = match (is_order(&operation), is_order(previous)) {
                (current, previous) if current == previous => None,
                (false, true)
                    if matches!(
                        action,
                        Some(
                            MdUpdateAction::Change
                                | MdUpdateAction::Delete
                                | MdUpdateAction::Overlay
                        )
                    ) && !operation
                        .operation_event()
                        .get_identifiers()
                        .contains_kind(&ORDER_ID) =>
                {
                    Some(true)
                }
                (true, false)
                    if partial
                        && operation
                            .operation_event()
                            .get_identifiers()
                            .contains_kind(&ORDER_ID)
                        && !previous
                            .operation_event()
                            .get_identifiers()
                            .contains_kind(&ORDER_ID) =>
                {
                    Some(true)
                }
                (current, previous) => {
                    let current = if current { "order" } else { "quote" };
                    let previous = if previous { "order" } else { "quote" };
                    return Err(invalid(
                        "$.operation.kind",
                        format_smolstr!("expected a continuation of {previous}, got {current}"),
                    ));
                }
            };
            // A change or an overlay inherits what it leaves unstated, and
            // so does a delete: it names the entry it removes, and a feed
            // routinely states no price on it, so the terminal delta carries
            // the level it takes out.
            if partial || action == Some(MdUpdateAction::Delete) {
                if entry_px_of(&operation).is_none() {
                    operation.set_price(previous.get_price(), true);
                }
                if entry_size_of(&operation).is_none() {
                    operation.set_quantity(previous.get_quantity(), true);
                }
            }
            let data = operation_event_data(&operation).clone();
            let data = if data.get_curruuid() == previous.get_curruuid() {
                data.restating(operation_event_data(previous))
            } else {
                data.clone()
                    .following_operation(operation_event_data(previous))
                    .unwrap_or_else(|| {
                        let mut data = data;
                        data.finalize();
                        data
                    })
            };
            operation = match promote_order {
                Some(true) => promote_to_order(operation, data),
                Some(false) => promote_to_quote(operation, data),
                None if is_order(&operation) => promote_to_order(operation, data),
                None => promote_to_quote(operation, data),
            };
            operation.finalize();
        }
        // No price is invented for an entry stating none - not a zero, not
        // a last executed price: it rests at the unpriced level.
        let Some(price) = BookPrice::of(operation.get_side(), operation.get_price()) else {
            return Err(invalid(
                "$.operation.side",
                format_smolstr!(
                    "expected the book side {:?}, got {:?}",
                    self.side.as_str(),
                    operation.get_side().as_str()
                ),
            ));
        };
        if let Some(previous) = self.take_removed(&identity)
            && let Some(journal) = journal.as_deref_mut()
        {
            journal.removed.push(previous);
        }
        if operation.operation_event().get_state().is_live()
            && operation
                .operation_event()
                .get_exprunix()
                .is_none_or(|expiration| expiration > operation.operation_event().get_currunix())
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

    fn identity_of(&self, operation: &MarketData) -> LiveKey {
        let own = LiveKey::of(operation);
        if let Some(identity) = self.referenced_identity_of(operation) {
            return identity;
        }
        if self.index.positions.contains_key(&own) {
            return own;
        }
        operation
            .operation_event()
            .get_identifiers()
            .get(&ENTRY_ID)
            .and_then(|wanted| self.find_entry_identity(operation, wanted))
            .unwrap_or(own)
    }

    fn referenced_identity_of(&self, operation: &MarketData) -> Option<LiveKey> {
        operation
            .operation_event()
            .get_identifiers()
            .get(&ENTRY_REF_ID)
            .and_then(|wanted| self.find_entry_identity(operation, wanted))
    }

    fn find_entry_identity(&self, operation: &MarketData, wanted: &str) -> Option<LiveKey> {
        let key = EntryKey {
            partition: SnapshotPartition::of(operation),
            entry: SmolStr::new(wanted),
        };
        match self.index.entry_ids.get(&key)? {
            EntrySlot::One(identity) => Some(identity.clone()),
            EntrySlot::Many(_) => self.live().find_map(|held| {
                (same_partition(held, operation)
                    && held.operation_event().get_identifiers().get(&ENTRY_ID) == Some(wanted))
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
        operation: Arc<MarketData>,
        identity: LiveKey,
    ) {
        let index = Arc::make_mut(&mut self.index);
        index.record(&operation, &identity);
        index.positions.insert(identity, price);
        self.levels.entry(price).or_default().insert(at, operation);
    }

    fn get(&self, identity: &LiveKey) -> Option<&MarketData> {
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

    fn remove(&mut self, identity: &LiveKey) -> Option<Arc<MarketData>> {
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
        operation: &MarketData,
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
                "$.operation.book.position",
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
        snapshot: Vec<MarketData>,
        partitions: &BTreeSet<SnapshotPartition>,
    ) -> Result<()> {
        let live = self
            .levels
            .values()
            .flatten()
            .filter(|operation| !partitions.contains(&SnapshotPartition::of(operation)))
            .cloned()
            .chain(snapshot.into_iter().map(Arc::new))
            .enumerate()
            .collect();
        let deltas = self.deltas.iter().cloned().enumerate().collect();
        *self = Self::from_shared_parts(self.side, live, deltas)?;
        Ok(())
    }

    /// Refuses a level whose aggregate quantity overflows decimal: every
    /// level is summed, so a checked side holds none, which
    /// [`Self::best_quantity`] and [`Self::limits`] rely on.
    fn check_levels(&self) -> Result<()> {
        for (key, level) in &self.levels {
            if level_quantity(level).is_none() {
                return Err(invalid(
                    "$.quantity",
                    match key.price() {
                        Some(price) => format_smolstr!(
                            "expected the aggregate quantity at {price} to fit decimal, got an overflow"
                        ),
                        None => SmolStr::new_static(
                            "expected the aggregate unpriced quantity to fit decimal, got an overflow",
                        ),
                    },
                ));
            }
        }
        Ok(())
    }

    /// The side checked and its digest brought in step with what it holds.
    fn refresh(&mut self) -> Result<()> {
        self.check_levels()?;
        self.rehash();
        Ok(())
    }

    /// Digests the side: two facts per entry of a side that may be
    /// thousands deep, staged so the state reads them a chunk at a time.
    fn rehash(&mut self) {
        let mut digest = Xxh3::new();
        {
            let mut staged = super::element::Staged::new(&mut digest);
            staged.write(&(self.len() as u64).to_be_bytes());
            for operation in self.levels.values().flatten() {
                staged.write(operation.operation_event().operation_word().as_bytes());
                staged.write(&operation.get_curruuid().get().to_be_bytes());
            }
            staged.write(&(self.deltas.len() as u64).to_be_bytes());
            for operation in &self.deltas {
                staged.write(operation.operation_event().operation_word().as_bytes());
                staged.write(&operation.get_curruuid().get().to_be_bytes());
            }
        }
        self.hashcode = digest.finish();
    }

    fn clear_deltas(&mut self) {
        if self.deltas.is_empty() {
            return;
        }
        self.deltas.clear();
        self.rehash();
    }

    /// This side as it stands, deltas and all, the walk's copy keeping its
    /// entries and none of its deltas: `clone` then [`Self::clear_deltas`],
    /// the deltas handed over rather than copied.
    fn emit(&mut self) -> Self {
        let deltas = std::mem::take(&mut self.deltas);
        let hashcode = self.hashcode;
        if !deltas.is_empty() {
            self.rehash();
        }
        Self {
            side: self.side,
            levels: self.levels.clone(),
            index: Arc::clone(&self.index),
            deltas,
            hashcode,
        }
    }
}

/// The facts a book entry's data holds, whichever kind it is - never
/// mutated in place through this borrow, only read or cloned for the next
/// reconstruction.
fn operation_event_data(operation: &MarketData) -> &OperationEventFacts {
    operation.operation_event().facts()
}

/// One coherent view of a market at one exact nanosecond instant: the
/// entries alive on its two sides, the deltas applied since the book before
/// it, and the executions at its instant.
///
/// A book answers each side as the [`Limit`]s its row states under
/// `bidlimits` and `asklimits` - [`Self::limits`], best first - and the
/// readings of their first levels: [`Self::best_price`],
/// [`Self::best_quantity`], [`Self::spread`], [`Self::is_crossed`],
/// [`Self::is_locked`], [`Self::imbalance`].
#[derive(Clone, Debug)]
pub struct BookEvent {
    event: MarketEventFacts,
    bid: Ladder,
    ask: Ladder,
    executions: Vec<ExecutionEvent>,
    /// The scopes the book's last snapshot replaced: walk state, which no
    /// row states and neither the digest nor equality reads.
    snapshots: BTreeSet<SnapshotPartition>,
}

/// Two books are equal by what their rows state: the event, the entries
/// alive and applied, and the executions - never the walk's snapshot state.
impl PartialEq for BookEvent {
    fn eq(&self, other: &Self) -> bool {
        self.event == other.event
            && self.bid == other.bid
            && self.ask == other.ask
            && self.executions == other.executions
    }
}

struct BookJournal {
    event: MarketEventFacts,
    bid: SideJournal,
    ask: SideJournal,
    executions_len: usize,
    cleared_executions: Option<Vec<ExecutionEvent>>,
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
    /// The member's instant: its place bounds the book's only at the book's
    /// own instant, because a place counts the events of one instant.
    currunix: i64,
    seqnum: u64,
    creaunix: Option<i64>,
    recdunix: Option<i64>,
    execunix: Option<i64>,
}

impl EventBounds {
    /// The event bounds `event` states, with no execution: that is the
    /// market's fact, which a caller holding one sets.
    fn of<E: Event + ?Sized>(event: &E) -> Self {
        Self {
            currunix: event.get_currunix(),
            seqnum: event.get_seqnum(),
            creaunix: event.get_creaunix(),
            recdunix: event.get_recdunix(),
            execunix: None,
        }
    }

    fn of_data(operation: &MarketData) -> Self {
        Self {
            execunix: operation.get_execunix(),
            ..Self::of(operation.operation_event())
        }
    }
}

impl BookJournal {
    fn new(book: &mut BookEvent, clear_changes: bool) -> Self {
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

    fn rollback(self, book: &mut BookEvent) {
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

impl BookEvent {
    /// An empty symbol book at one nanosecond instant.
    #[must_use]
    pub fn new(unix: i64, symbol: impl Into<String>) -> Self {
        let symbol = symbol.into();
        let mut event = MarketEventFacts::at(unix);
        event.set_marketdatakind(crate::MarketDataKind::Book);
        event.set_ticker((!symbol.is_empty()).then(|| SmolStr::new(&symbol)), true);
        event.set_crosscode(symbol);
        event.set_state(State::New);
        let mut book = Self {
            event,
            bid: Ladder::new(Side::Buy),
            ask: Ladder::new(Side::Sell),
            executions: Vec::new(),
            snapshots: BTreeSet::new(),
        };
        book.finalize();
        book
    }

    /// An empty book of one category at one nanosecond instant: `key` is
    /// its crosscode - `{miccode}:{cficode}`, what
    /// [`Market::book_crosscode`] answers for an input stating no ticker -
    /// and the book states no ticker, since no input stated one.
    #[must_use]
    pub(crate) fn categorized(unix: i64, key: impl Into<String>) -> Self {
        let mut event = MarketEventFacts::at(unix);
        event.set_marketdatakind(crate::MarketDataKind::Book);
        event.set_crosscode(key.into());
        event.set_state(State::New);
        let mut book = Self {
            event,
            bid: Ladder::new(Side::Buy),
            ask: Ladder::new(Side::Sell),
            executions: Vec::new(),
            snapshots: BTreeSet::new(),
        };
        book.finalize();
        book
    }

    /// Rebuilds one canonical book from what its row states - the event,
    /// the entries alive on both sides, the deltas and the executions -
    /// each entry standing on the side it states, validating that the
    /// entries, the executions and every symbol agree with the event,
    /// without replaying anything as a fresh mutation.
    pub(crate) fn from_parts(
        event: MarketEventFacts,
        alive: Vec<MarketData>,
        deltas: Vec<MarketData>,
        executions: Vec<ExecutionEvent>,
    ) -> Result<Self> {
        let (mut bid_alive, mut ask_alive) = (Vec::new(), Vec::new());
        for (index, entry) in alive.into_iter().enumerate() {
            let held = if entry.get_side().is_bid() {
                &mut bid_alive
            } else {
                &mut ask_alive
            };
            held.push((index, Arc::new(entry)));
        }
        let (mut bid_deltas, mut ask_deltas) = (Vec::new(), Vec::new());
        for (index, entry) in deltas.into_iter().enumerate() {
            let held = if entry.get_side().is_bid() {
                &mut bid_deltas
            } else {
                &mut ask_deltas
            };
            held.push((index, entry));
        }
        Self::from_ladders(
            event,
            Ladder::from_shared_parts(Side::Buy, bid_alive, bid_deltas)?,
            Ladder::from_shared_parts(Side::Sell, ask_alive, ask_deltas)?,
            executions,
            BTreeSet::new(),
        )
    }

    /// A book over sides already built, validated and refreshed.
    fn from_ladders(
        mut event: MarketEventFacts,
        bid: Ladder,
        ask: Ladder,
        executions: Vec<ExecutionEvent>,
        snapshots: BTreeSet<SnapshotPartition>,
    ) -> Result<Self> {
        // A book is not sided: its cross code stays as given.
        event.set_marketdatakind(crate::MarketDataKind::Book);
        let mut book = Self {
            event,
            bid,
            ask,
            executions,
            snapshots,
        };
        book.validate_parts()?;
        book.refresh()?;
        Ok(book)
    }

    pub(super) fn validate_parts(&self) -> Result<()> {
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
        let book = (self.event.get_ticker(), self.event.get_crosscode());
        validate_symbols(book, ALIVE, self.alive())?;
        validate_symbols(book, DELTAS, self.deltas())?;
        for (index, execution) in self.executions.iter().enumerate() {
            validate_symbol(book, execution, || format_smolstr!("$.executions[{index}]"))?;
        }
        let unix = self.event.get_currunix();
        validate_component_times(unix, ALIVE, entries(self.alive()), true)?;
        validate_component_times(unix, DELTAS, entries(self.deltas()), false)?;
        validate_component_times(
            self.event.get_currunix(),
            "executions",
            self.executions.iter(),
            false,
        )?;
        validate_propagation_bounds(&self.event, ALIVE, entries(self.alive()))?;
        validate_propagation_bounds(&self.event, DELTAS, entries(self.deltas()))?;
        validate_propagation_bounds(&self.event, "executions", self.executions.iter())
    }

    /// Every entry alive on the book: the bid side's, best price first and
    /// every entry stating no price last, then the ask side's the same way.
    pub fn alive(&self) -> impl Iterator<Item = &MarketData> {
        self.bid.live().chain(self.ask.live())
    }

    /// The deltas applied since the book before this one: the bid side's in
    /// the order they were applied, then the ask side's.
    pub fn deltas(&self) -> impl Iterator<Item = &MarketData> {
        self.bid.deltas.iter().chain(&self.ask.deltas)
    }

    /// How many entries [`Self::alive`] answers, without walking them.
    pub(crate) fn alive_len(&self) -> usize {
        self.bid.len() + self.ask.len()
    }

    /// How many deltas [`Self::deltas`] answers, without walking them.
    pub(crate) fn deltas_len(&self) -> usize {
        self.bid.deltas.len() + self.ask.deltas.len()
    }

    /// The executions at the book's instant.
    #[must_use]
    pub fn executions(&self) -> &[ExecutionEvent] {
        &self.executions
    }

    /// The side a bid or an ask names; none for a side that is neither.
    fn ladder(&self, side: Side) -> Option<&Ladder> {
        if side.is_bid() {
            Some(&self.bid)
        } else if side.is_ask() {
            Some(&self.ask)
        } else {
            None
        }
    }

    /// The best tradable price on the side `side` takes: the first priced
    /// level's whose [`Limit::tradable`] holds, `None` for an empty side,
    /// one holding only unpriced entries, one no level of which can trade,
    /// or a side that is neither a bid nor an ask. A level that cannot trade
    /// is skipped, never answered. What the book states as `bidpx` (`BUYS`)
    /// and `askpx` (`SELL`).
    #[must_use]
    pub fn best_price(&self, side: Side) -> Option<Decimal> {
        self.ladder(side).and_then(Ladder::best_price)
    }

    /// The aggregate quantity at [`Self::best_price`]: the exact sum of
    /// what the entries of that level state, an entry stating none adding
    /// nothing.
    #[must_use]
    pub fn best_quantity(&self, side: Side) -> Option<Decimal> {
        self.ladder(side).and_then(Ladder::best_quantity)
    }

    /// One [`Limit`] per level of the side `side` takes, best first and the
    /// unpriced limit last: its price, the exact sum of its entries'
    /// quantities, their `curruuid`s in position order, ties in arrival
    /// order, and whether any of them does not state `tradable = false` - an
    /// entry stating nothing trades, and a level every entry of which states
    /// `false` cannot. Nothing for a
    /// side that is neither a bid nor an ask. What a book's row states
    /// under `bidlimits` (`BUYS`) and `asklimits` (`SELL`); the first priced
    /// limit that can trade is [`Self::best_price`] and
    /// [`Self::best_quantity`].
    ///
    /// ```
    /// use yggdryl::graph::{BookEvent, Element, Event, Market, MarketData, Operation, OrderEvent};
    /// use yggdryl::{Decimal, Side, State};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = |code: &str, price: Option<i64>, quantity: i64, tradable: Option<bool>| {
    ///     let mut order = OrderEvent::at(1);
    ///     order.set_crosscode(code.to_owned());
    ///     order.set_side(Side::Buy, true);
    ///     order.set_price(price.map(Decimal::from_int), true);
    ///     order.set_quantity(Some(Decimal::from_int(quantity)), true);
    ///     order.set_tradable(tradable, true);
    ///     order.set_state(State::New);
    ///     order.finalize();
    ///     MarketData::from(order)
    /// };
    /// let mut book = BookEvent::new(1, "ACME");
    /// book.add_operations([
    ///     order("A", Some(100), 2, None),
    ///     order("MARKET", None, 5, Some(false)),
    ///     order("B", Some(102), 3, Some(false)),
    ///     order("C", Some(101), 4, Some(true)),
    ///     order("D", Some(101), 1, Some(false)),
    /// ])?;
    ///
    /// let limits: Vec<_> = book.limits(Side::Buy).collect();
    /// assert_eq!(limits.len(), 4);
    /// // The only entry at 102 states it cannot trade.
    /// assert_eq!(limits[0].price, Some(Decimal::from_int(102)));
    /// assert!(!limits[0].tradable);
    /// // One entry at 101 can trade; 100 states nothing, so it trades too.
    /// assert_eq!(limits[1].price, Some(Decimal::from_int(101)));
    /// assert_eq!(limits[1].quantity, Decimal::from_int(5));
    /// assert_eq!(limits[1].uuids.len(), 2);
    /// assert!(limits[1].tradable && limits[2].tradable);
    /// // The market order rests after every priced level.
    /// assert_eq!(limits[3].price, None);
    /// assert_eq!(limits[3].quantity, Decimal::from_int(5));
    /// assert!(!limits[3].tradable);
    /// // The first tradable level is the best bid and the quantity there.
    /// assert_eq!(book.best_price(Side::Buy), Some(Decimal::from_int(101)));
    /// assert_eq!(book.get_bidpx(), Some(Decimal::from_int(101)));
    /// assert_eq!(book.best_quantity(Side::Buy), Some(Decimal::from_int(5)));
    /// assert_eq!(book.limits(Side::Sell).count(), 0);
    /// # Ok(())
    /// # }
    /// ```
    pub fn limits(&self, side: Side) -> impl Iterator<Item = Limit> + '_ {
        self.ladder(side)
            .into_iter()
            .flat_map(|ladder| ladder.limits())
    }

    /// The exact sum of the first `levels` limits' quantities of the side
    /// `side` takes, in [`Self::limits`] order, the unpriced limit counted
    /// where it is reached: zero for an empty side or no level, `None` past
    /// decimal or for a side that is neither a bid nor an ask.
    ///
    /// ```
    /// use yggdryl::graph::{BookEvent, Element, Event, Market, MarketData, OrderEvent};
    /// use yggdryl::{Decimal, Side, State};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = |code: &str, price: Option<i64>, quantity: i64| {
    ///     let mut order = OrderEvent::at(1);
    ///     order.set_crosscode(code.to_owned());
    ///     order.set_side(Side::Sell, true);
    ///     order.set_price(price.map(Decimal::from_int), true);
    ///     order.set_quantity(Some(Decimal::from_int(quantity)), true);
    ///     order.set_state(State::New);
    ///     order.finalize();
    ///     MarketData::from(order)
    /// };
    /// let mut book = BookEvent::new(1, "ACME");
    /// book.add_operations([
    ///     order("A", Some(102), 2),
    ///     order("B", Some(101), 3),
    ///     order("MARKET", None, 5),
    /// ])?;
    ///
    /// assert_eq!(book.depth(Side::Sell, 0), Some(Decimal::ZERO));
    /// assert_eq!(book.depth(Side::Sell, 1), Some(Decimal::from_int(3)));
    /// assert_eq!(book.depth(Side::Sell, 2), Some(Decimal::from_int(5)));
    /// assert_eq!(book.depth(Side::Sell, 3), Some(Decimal::from_int(10)));
    /// assert_eq!(book.depth(Side::Sell, 100), Some(Decimal::from_int(10)));
    /// assert_eq!(book.depth(Side::Buy, 100), Some(Decimal::ZERO));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn depth(&self, side: Side, levels: usize) -> Option<Decimal> {
        self.ladder(side)?.depth(levels)
    }

    /// Whether the best tradable bid is above the best tradable ask; a
    /// locked book - the two equal - is not crossed, and a side stating no
    /// best crosses nothing.
    #[must_use]
    pub fn is_crossed(&self) -> bool {
        matches!(
            (self.bid.best_price(), self.ask.best_price()),
            (Some(bid), Some(ask)) if bid > ask
        )
    }

    /// Whether both sides state a best tradable price and the two are equal.
    ///
    /// ```
    /// use yggdryl::graph::{BookEvent, Element, Event, Market, MarketData, Operation, OrderEvent};
    /// use yggdryl::{Decimal, Side, State};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = |code: &str, side: &str, price: i64| {
    ///     let mut order = OrderEvent::at(1);
    ///     order.set_crosscode(code.to_owned());
    ///     order.set_side(Side::read(side).unwrap(), true);
    ///     order.set_price(Some(Decimal::from_int(price)), true);
    ///     order.set_quantity(Some(Decimal::from_int(1)), true);
    ///     order.set_tradable(Some(true), true);
    ///     order.set_state(State::New);
    ///     order.finalize();
    ///     MarketData::from(order)
    /// };
    /// let mut book = BookEvent::new(1, "ACME");
    /// book.add_operations([order("B", "Buy", 100), order("A", "Sell", 100)])?;
    /// assert!(book.is_locked() && !book.is_crossed());
    /// assert_eq!(book.spread(), Some(Decimal::ZERO));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn is_locked(&self) -> bool {
        matches!(
            (self.bid.best_price(), self.ask.best_price()),
            (Some(bid), Some(ask)) if bid == ask
        )
    }

    /// The best tradable ask less the best tradable bid: negative on a
    /// crossed book, which
    /// [`Self::is_crossed`] names; `None` where a side states no best price
    /// or the difference is past decimal.
    ///
    /// ```
    /// use yggdryl::graph::{BookEvent, Element, Event, Market, MarketData, Operation, OrderEvent};
    /// use yggdryl::{Decimal, Side, State};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = |code: &str, side: &str, price: &str| {
    ///     let mut order = OrderEvent::at(1);
    ///     order.set_crosscode(code.to_owned());
    ///     order.set_side(Side::read(side).unwrap(), true);
    ///     order.set_price(Some(price.parse().unwrap()), true);
    ///     order.set_quantity(Some(Decimal::from_int(1)), true);
    ///     order.set_tradable(Some(true), true);
    ///     order.set_state(State::New);
    ///     order.finalize();
    ///     MarketData::from(order)
    /// };
    /// let mut book = BookEvent::new(1, "ACME");
    /// book.add_operations([order("B", "Buy", "100")])?;
    /// assert_eq!(book.spread(), None);
    /// book.add_operations([order("A", "Sell", "100.25")])?;
    /// assert_eq!(book.spread(), Some("0.25".parse()?));
    /// // A bid above the ask: the spread says by how much.
    /// book.add_operations([order("B-2", "Buy", "101")])?;
    /// assert!(book.is_crossed());
    /// assert_eq!(book.spread(), Some("-0.75".parse()?));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn spread(&self) -> Option<Decimal> {
        self.ask.best_price()?.checked_sub(self.bid.best_price()?)
    }

    /// The order-book imbalance over the first `levels` limits of each side:
    /// `(bid - ask) / (bid + ask)` over their [`Self::depth`]s, from `1`
    /// for a book resting on the bid alone to `-1` on the ask alone; `None`
    /// where the total is zero - both sides empty, or no level - or past
    /// decimal.
    ///
    /// ```
    /// use yggdryl::graph::{BookEvent, Element, Event, Market, MarketData, OrderEvent};
    /// use yggdryl::{Decimal, Side, State};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = |code: &str, side: &str, price: i64, quantity: i64| {
    ///     let mut order = OrderEvent::at(1);
    ///     order.set_crosscode(code.to_owned());
    ///     order.set_side(Side::read(side).unwrap(), true);
    ///     order.set_price(Some(Decimal::from_int(price)), true);
    ///     order.set_quantity(Some(Decimal::from_int(quantity)), true);
    ///     order.set_state(State::New);
    ///     order.finalize();
    ///     MarketData::from(order)
    /// };
    /// let mut book = BookEvent::new(1, "ACME");
    /// assert_eq!(book.imbalance(1), None);
    /// book.add_operations([order("B", "Buy", 100, 30)])?;
    /// assert_eq!(book.imbalance(1), Some(Decimal::ONE));
    /// book.add_operations([order("A", "Sell", 101, 10), order("A-2", "Sell", 102, 20)])?;
    /// assert_eq!(book.imbalance(1), Some("0.5".parse()?));
    /// assert_eq!(book.imbalance(2), Some(Decimal::ZERO));
    /// assert_eq!(book.imbalance(0), None);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn imbalance(&self, levels: usize) -> Option<Decimal> {
        let bid = self.bid.depth(levels)?;
        let ask = self.ask.depth(levels)?;
        bid.checked_sub(ask)?.checked_div(bid.checked_add(ask)?)
    }

    /// The arithmetic midpoint of a coherent two-sided BBO.
    #[must_use]
    pub fn bbo_midpoint(&self) -> Option<Decimal> {
        let (bid, ask) = (self.bid.best_price()?, self.ask.best_price()?);
        (bid <= ask).then(|| decimal_mean(bid, ask)).flatten()
    }

    /// The two-value median of the best bid and ask aggregate quantities.
    #[must_use]
    pub fn median_quantity(&self) -> Option<Decimal> {
        median_quantity(self.bid.best_quantity(), self.ask.best_quantity())
    }

    /// Atomically applies all operations of one timestamp. Full-snapshot depth
    /// operations and controls first replace only their declared book scope;
    /// executions never control resting membership. A book folds an
    /// [`OrderEvent`](super::OrderEvent), a
    /// [`QuoteEvent`](super::QuoteEvent), an
    /// [`ExecutionEvent`], a
    /// [`TradeEvent`](super::TradeEvent) and a [`SnapshotEvent`].
    ///
    /// An entry stating no price - a market order - is given none: it rests
    /// at the one unpriced level of its side, after every priced level, so
    /// [`Self::alive`], the book's digest and a delete-from or delete-through
    /// position all reach it last, and [`Self::limits`] answers it as the
    /// side's last limit.
    ///
    /// # Errors
    ///
    /// Returns the first item's own error, and [`Error::InvalidRecord`] for
    /// any other variant - naming its kind - an operation the book refuses,
    /// or a level whose aggregate quantity would pass decimal (at
    /// `$.quantity`); the book is unchanged on every error.
    pub fn add_operations<I>(&mut self, operations: I) -> Result<()>
    where
        I: IntoIterator,
        I::Item: Into<Result<MarketData>>,
    {
        let mut operations = operations
            .into_iter()
            .map(Into::into)
            .collect::<Result<Vec<MarketData>>>()?;
        if operations.is_empty() {
            return Ok(());
        }
        for (index, operation) in operations.iter().enumerate() {
            foldable(operation, || format_smolstr!("$.operations[{index}].kind"))?;
        }
        let unix = input_unix(&operations[0]);
        if operations
            .iter()
            .any(|operation| input_unix(operation) != unix)
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
            next.event.set_seqnum(0);
        }
        next.event.set_currunix(unix);
        next.event.set_snapunix(None);
        let partitions: BTreeSet<SnapshotPartition> = operations
            .iter()
            .filter(|input| is_full_snapshot(input) && !is_execution_input(input))
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

    fn add_one_journaled(&mut self, operation: MarketData, unix: i64) -> Result<()> {
        let advancing = self.event.get_currunix() != unix;
        let mut journal = BookJournal::new(self, advancing);
        let mut changed = journal.cleared_sides();
        let result = (|| {
            if advancing {
                self.clear_changes();
                self.event.set_seqnum(0);
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
        bid: Vec<MarketData>,
        ask: Vec<MarketData>,
        controls: &[SnapshotEvent],
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
        validate_component_times(unix, "snapshot.bid", entries(&bid), true)?;
        validate_component_times(unix, "snapshot.ask", entries(&ask), true)?;
        validate_component_times(
            unix,
            "snapshot.controls",
            controls.iter().map(|control| &control.event),
            false,
        )?;
        let mut next = self.clone();
        if next.event.get_currunix() != unix {
            next.clear_changes();
            next.event.set_seqnum(0);
        }
        next.event.set_currunix(unix);
        next.event.set_snapunix(Some(unix));
        for operation in bid.iter().chain(&ask) {
            next.fold_bounds(EventBounds::of_data(operation));
        }
        for control in controls {
            next.fold_bounds(EventBounds {
                execunix: control.event.get_execunix(),
                ..EventBounds::of(&control.event)
            });
        }
        next.bid.replace_membership(bid, partitions)?;
        next.ask.replace_membership(ask, partitions)?;
        next.record_snapshot_partitions(partitions);
        next.refresh_changed(ChangedSides::default())?;
        *self = next;
        Ok(())
    }

    fn apply(&mut self, operation: MarketData) -> Result<ChangedSides> {
        self.apply_inner(operation, None)
    }

    fn apply_journaled(
        &mut self,
        operation: MarketData,
        journal: &mut BookJournal,
    ) -> Result<ChangedSides> {
        self.apply_inner(operation, Some(journal))
    }

    fn apply_inner(
        &mut self,
        input: MarketData,
        journal: Option<&mut BookJournal>,
    ) -> Result<ChangedSides> {
        foldable(&input, || SmolStr::new_static("$.operation.kind"))?;
        if let Some(reason) = book_mismatch(self.get_ticker(), self.get_crosscode(), &input) {
            return Err(invalid("$.operation.ticker", reason));
        }
        let Some(event_view) = input.as_event() else {
            return Err(invalid(
                "$.operation",
                "expected a dated operation on a book",
            ));
        };
        validate_component_times(
            self.event.get_currunix(),
            "operation",
            std::iter::once(event_view),
            false,
        )?;
        let mut bounds = EventBounds::of(event_view);
        // When the input last executed is a market fact of the input itself.
        bounds.execunix = input.get_execunix();
        let changed = match input {
            MarketData::ExecutionEvent(execution) => {
                self.executions.push(execution);
                ChangedSides::default()
            }
            MarketData::TradeEvent(trade) => {
                self.executions.extend(trade.into_executions());
                ChangedSides::default()
            }
            MarketData::SnapshotEvent(_) => ChangedSides::default(),
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
                bounds = EventBounds::of_data(
                    self.bid
                        .deltas
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
                bounds = EventBounds::of_data(
                    self.ask
                        .deltas
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

    /// Folds a member's facts into the book's: the highest place of the
    /// members at the book's instant, a new instant having started the
    /// book's own at zero, the earliest creation and recording, and the
    /// latest execution.
    fn fold_bounds(&mut self, bounds: EventBounds) {
        if bounds.currunix == self.event.get_currunix() && bounds.seqnum > self.event.get_seqnum() {
            self.event.set_seqnum(bounds.seqnum);
        }
        self.event
            .set_creaunix(earliest(self.event.get_creaunix(), bounds.creaunix));
        self.event
            .set_recdunix(earliest(self.event.get_recdunix(), bounds.recdunix));
        self.event
            .set_execunix(latest(self.event.get_execunix(), bounds.execunix), true);
    }

    /// The event facts the book settles on, its two sides checked first: a
    /// level whose aggregate quantity passes decimal is refused.
    pub(super) fn canonical_event(&self) -> Result<MarketEventFacts> {
        self.bid.check_levels()?;
        self.ask.check_levels()?;
        Ok(self.settled_event())
    }

    /// The event facts the book settles on over sides already checked: the
    /// BBO midpoint, the median best quantity, the currency and the unit
    /// its bests agree on, and the digest.
    fn settled_event(&self) -> MarketEventFacts {
        let mut event = self.event.clone();
        let midpoint = if self.is_crossed() {
            None
        } else {
            self.bbo_midpoint()
                .or_else(|| self.bid.best_price().or_else(|| self.ask.best_price()))
        };
        event.set_price(midpoint, true);
        event.set_quantity(self.median_quantity(), true);
        // A side speaks for the book only where it states a best: a side of
        // unpriced entries alone states no currency and no unit, and leaves
        // the other side's standing alone.
        let (bid, ask) = (self.bid.best_entry(), self.ask.best_entry());
        let currency = match (bid.map(Market::get_currency), ask.map(Market::get_currency)) {
            (Some(bid), Some(ask)) if bid == ask => bid.clone(),
            (Some(currency), None) | (None, Some(currency)) => currency.clone(),
            _ => Ccy::none(),
        };
        event.set_currency(currency, true);
        let unit = match (bid.map(Market::get_unit), ask.map(Market::get_unit)) {
            (Some(bid), Some(ask)) if bid == ask => bid.clone(),
            (Some(unit), None) | (None, Some(unit)) => unit.clone(),
            _ => Unit::none(),
        };
        event.set_unit(unit, true);
        // The best bid and ask are the best tradable levels, each in the
        // book's currency where it states one: nothing where no level of
        // the side can trade.
        let stated = (event.get_currency() != &Ccy::none()).then(|| event.get_currency().clone());
        for (ladder, bid) in [(&self.bid, true), (&self.ask, false)] {
            let (px, qty) = (ladder.best_price(), ladder.best_quantity());
            let ccy = px.and(stated.clone());
            if bid {
                event.set_bidpx(px, true);
                event.set_bidqty(qty, true);
                event.set_bidccy(ccy, true);
            } else {
                event.set_askpx(px, true);
                event.set_askqty(qty, true);
                event.set_askccy(ccy, true);
            }
        }
        finalize_book_event(&mut event, &self.bid, &self.ask, &self.executions);
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
        self.event = self.settled_event();
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
        self.snapshots.clear();
    }

    /// The book as it stands, changes and all, the walk's copy kept cleared
    /// of them: `clone` then [`Self::clear_changes`], the changes an instant
    /// applied handed over rather than copied into a book that drops them.
    fn emit(&mut self) -> Self {
        Self {
            event: self.event.clone(),
            bid: self.bid.emit(),
            ask: self.ask.emit(),
            executions: std::mem::take(&mut self.executions),
            snapshots: std::mem::take(&mut self.snapshots),
        }
    }

    fn record_snapshot_partitions(&mut self, partitions: &BTreeSet<SnapshotPartition>) {
        self.snapshots.extend(partitions.iter().cloned());
    }

    fn set_book_time(&mut self, unix: i64, snapshot: bool) {
        self.event.set_currunix(unix);
        self.event.set_snapunix(snapshot.then_some(unix));
        self.finalize();
    }
}

/// One instant's inputs of one book in the order their chains place them:
/// each chain's steps by their place at the instant - a step following
/// another of that instant stands after it - each chain where its first
/// step arrived. An instant orders nothing by itself, so a source that read two
/// steps of one chain back in another order - a table sorting a first
/// step's unstated place after the second's - still folds the chain as it
/// happened; inputs already in their chains' order are left as they came.
///
/// A step is out of order where an earlier step of its chain stands at a
/// later place: where the furthest place its chain reached so far is past
/// its own. A place orders a chain only where a walk gave it - a step
/// following a step of its chain at its own instant stands after it - so a
/// step following none there reads as the first, and places a parse gave
/// runs of one instant apart in a file order nothing. A group of at most [`CHAINS_SCANNED`] inputs is checked against
/// the ones before it and allocates nothing; a larger one keeps each chain's
/// first arrival and furthest place in one table, so an instant thousands
/// deep costs one pass rather than every pair.
fn order_chains(operations: &mut Vec<MarketData>) {
    let place = |operation: &MarketData| {
        operation.as_event().map_or(0, |event| {
            if event.get_prevunix() == Some(event.get_currunix()) {
                event.get_seqnum()
            } else {
                0
            }
        })
    };
    if operations.len() <= CHAINS_SCANNED {
        let disordered = operations.iter().enumerate().any(|(at, later)| {
            operations[..at].iter().any(|earlier| {
                earlier.get_crossuuid() == later.get_crossuuid() && place(earlier) > place(later)
            })
        });
        if !disordered {
            return;
        }
    }
    // Each chain's first arrival and the furthest place it reached.
    let mut chains: HashMap<Uuid, (usize, u64)> = HashMap::with_capacity(operations.len());
    let mut disordered = false;
    let mut firsts = Vec::with_capacity(operations.len());
    for (at, operation) in operations.iter().enumerate() {
        let step = place(operation);
        let chain = chains
            .entry(operation.get_crossuuid())
            .or_insert((at, step));
        disordered |= chain.1 > step;
        chain.1 = chain.1.max(step);
        firsts.push(chain.0);
    }
    if !disordered {
        return;
    }
    let mut keyed: Vec<(usize, u64, MarketData)> = operations
        .drain(..)
        .zip(firsts)
        .map(|(operation, first)| (first, place(&operation), operation))
        .collect();
    keyed.sort_by_key(|(first, place, _)| (*first, *place));
    operations.extend(keyed.into_iter().map(|(_, _, operation)| operation));
}

/// The most inputs of one instant [`order_chains`] checks pair by pair.
const CHAINS_SCANNED: usize = 32;

fn median_quantity(bid: Option<Decimal>, ask: Option<Decimal>) -> Option<Decimal> {
    match (bid, ask) {
        (Some(bid), Some(ask)) => decimal_mean(bid, ask),
        (Some(quantity), None) | (None, Some(quantity)) => Some(quantity),
        (None, None) => None,
    }
}

fn finalize_book_event(
    event: &mut MarketEventFacts,
    bid: &Ladder,
    ask: &Ladder,
    executions: &[ExecutionEvent],
) {
    event.sync_cross();
    let mut digest = event.digest_market_event();
    digest.write(&event.get_currunix().to_be_bytes());
    for side in [bid, ask] {
        digest.write(&side.hashcode.to_be_bytes());
    }
    for execution in executions {
        digest.write(&execution.get_curruuid().get().to_be_bytes());
    }
    event.finalized(digest.finish());
}

fn canonicalize_executions(executions: &mut Vec<ExecutionEvent>) {
    executions.sort_by_key(Element::get_curruuid);
    executions.dedup_by_key(|execution| execution.get_curruuid());
}

impl Default for BookEvent {
    fn default() -> Self {
        Self::new(0, String::new())
    }
}

impl BookEvent {
    /// The book's own event facts.
    pub(crate) fn event(&self) -> &MarketEventFacts {
        &self.event
    }
}

impl Element for BookEvent {
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
        finalize_book_event(&mut self.event, &self.bid, &self.ask, &self.executions);
    }

    fn with_previous(mut self, previous: &Self) -> Option<Self> {
        if self.get_crosscode() != previous.get_crosscode()
            || self.get_currunix() < previous.get_currunix()
        {
            return None;
        }
        let event = self.event.clone().following_market(&previous.event)?;
        let authoritative = self.get_snapunix().is_some();
        let replaced = self.snapshots.clone();
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
        self.snapshots = replaced;
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
        let replaced = &reference.snapshots;
        let authoritative = reference.get_snapunix().is_some();
        let bid = merge_book_side(&reference.bid, &supplement.bid, replaced, authoritative)?;
        let ask = merge_book_side(&reference.ask, &supplement.ask, replaced, authoritative)?;
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
        merge_market_event_into_reference(&mut event, &supplement.event);
        let merged = Self::from_ladders(event, bid, ask, executions, replaced.clone()).ok()?;
        (merged != self).then_some(merged)
    }
}

delegate_market!(BookEvent, event);
delegate_event!(
    BookEvent,
    event,
    restating = |mut this: BookEvent, live: &BookEvent| {
        let event = std::mem::take(&mut this.event).restating(&live.event);
        this.event = event;
        this.finalize();
        this
    },
    is_execution = |_: &BookEvent| false,
    set_currunix = |this: &mut BookEvent, unix: i64| this.event.set_currunix(unix)
);

/// Books from a sorted operation stream, one per book crosscode and
/// effective timestamp. An input's book is [`Market::book_crosscode`]: its
/// ticker where it states one, else its category `{miccode}:{cficode}`, so
/// instruments without a ticker still split into books by market and
/// classification. The first input routed to a key decides whether its
/// book states that key as its ticker; no book adopts a ticker later. Each
/// timestamp and book is committed atomically across ordinary deltas,
/// expirations, and explicit snapshot membership.
pub struct BookIterator<I>
where
    I: Iterator,
    I::Item: Into<Result<MarketData>>,
{
    source: I,
    /// The leaves a FIX message pulled from the source split into, taken in
    /// order before the source is pulled again: a book folds what a
    /// message reports, one fill, side and entry each.
    split: VecDeque<MarketData>,
    source_head: Option<Result<MarketData>>,
    source_exhausted: bool,
    books: BTreeMap<String, BookEvent>,
    /// Per book key, whether the first input routed under it stated a
    /// ticker: what the new book states as its own. Never overwritten.
    stated_ticker: BTreeMap<String, bool>,
    pending: VecDeque<Result<BookEvent>>,
    snapshot_ns: Option<i64>,
    next_snapshot: Option<i64>,
    expirations: BTreeSet<BookExpiration>,
    last_unix: Option<i64>,
    done: bool,
}

impl<I> BookIterator<I>
where
    I: Iterator,
    I::Item: Into<Result<MarketData>>,
{
    /// Opens a book walk over operations already sorted by their event order.
    /// Owned operations and their fallible counterparts are accepted directly;
    /// `snapshot_millis == 0` disables grid snapshots.
    pub fn new(operations: I, snapshot_millis: u64) -> Result<Self> {
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
            split: VecDeque::new(),
            source_head: None,
            source_exhausted: false,
            books: BTreeMap::new(),
            stated_ticker: BTreeMap::new(),
            pending: VecDeque::new(),
            snapshot_ns: (snapshot_ns > 0).then_some(snapshot_ns),
            next_snapshot: None,
            expirations: BTreeSet::new(),
            last_unix: None,
            done: false,
        })
    }

    fn fill_source_head(&mut self) {
        if self.source_head.is_some() || self.source_exhausted {
            return;
        }
        let next = match self.split.pop_front() {
            Some(leaf) => Some(Ok(leaf)),
            None => self.source.next().map(Into::into),
        };
        let next = match next {
            Some(Ok(MarketData::Fix(message))) => match message.into_market_data() {
                Ok(leaves) => {
                    self.split.extend(leaves);
                    match self.split.pop_front() {
                        Some(leaf) => Some(Ok(leaf)),
                        // A message reporting nothing a book holds folds
                        // nothing: the next one is pulled in its place.
                        None => return self.fill_source_head(),
                    }
                }
                Err(error) => Some(Err(error)),
            },
            other => other,
        };
        match next {
            Some(operation) => {
                self.source_head = Some(operation.and_then(|operation| {
                    foldable(&operation, || SmolStr::new_static("$.operation.kind"))
                        .map(|()| operation)
                }));
            }
            None => self.source_exhausted = true,
        }
    }

    fn peek_source(&mut self) -> Option<&MarketData> {
        self.fill_source_head();
        self.source_head
            .as_ref()
            .and_then(|operation| operation.as_ref().ok())
    }

    fn take_source(&mut self) -> MarketData {
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
            self.pending.push_back(Ok(book.emit()));
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
                let Some(unix) = operation.operation_event().get_exprunix() else {
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

    fn take_expirations(&mut self, unix: i64) -> BTreeMap<String, Vec<MarketData>> {
        let mut expired = BTreeMap::<String, Vec<MarketData>>::new();
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
                || operation.operation_event().get_exprunix() != Some(unix)
                || !operation.operation_event().get_state().is_live()
            {
                continue;
            }
            let mut operation = operation.clone();
            operation.operation_event_mut().set_currunix(unix);
            operation.operation_event_mut().set_state(State::Expired);
            let mut book = operation
                .operation_event()
                .control()
                .cloned()
                .unwrap_or_default();
            book.action = Some(MdUpdateAction::Delete);
            operation.operation_event_mut().set_control(Some(book));
            // The synthetic delete addresses the current generation, not
            // the reference a prior rename used to reach its predecessor.
            let mut identifiers = operation.operation_event().get_identifiers().clone();
            if identifiers
                .remove(&crate::IdKey::base(ENTRY_REF_ID))
                .is_some()
            {
                let _ = operation
                    .operation_event_mut()
                    .set_identifiers(identifiers, true);
            }
            operation.operation_event_mut().set_execunix(None, true);
            operation.operation_event_mut().set_recdunix(None);
            operation.operation_event_mut().set_snapunix(None);
            // An event of its own deadline, placed first there, so a step of
            // its chain at that instant folds after it.
            operation.operation_event_mut().set_seqnum(0);
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
                let mut skipped = 0_usize;
                while self
                    .peek_source()
                    .is_some_and(|operation| effective_unix(operation) == unix)
                {
                    self.take_source();
                    skipped += 1;
                }
                warned!(
                    "book operations excluded: dated before the book they would fold into",
                    "$.operations",
                    "{skipped} at {unix}, after the book reached {}",
                    self.last_unix.expect("checked")
                );
                continue;
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
            let mut snapshot_bid: BTreeMap<String, Vec<MarketData>> = BTreeMap::new();
            let mut snapshot_ask: BTreeMap<String, Vec<MarketData>> = BTreeMap::new();
            let mut snapshot_controls: BTreeMap<String, Vec<SnapshotEvent>> = BTreeMap::new();
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
                    let input = self.take_source();
                    // An order or a quote rests on a book side, and one
                    // stating neither the bid nor the ask rests on none: it
                    // is left out of the book, never the book out of the
                    // walk.
                    if matches!(input, MarketData::OrderEvent(_) | MarketData::QuoteEvent(_))
                        && !input.get_side().is_bid()
                        && !input.get_side().is_ask()
                    {
                        warned!(
                            "book entry excluded: it states no bid or ask side",
                            input.kind().as_str(),
                            "{} states side {}",
                            input.get_crosscode(),
                            input.get_side().as_str()
                        );
                        continue;
                    }
                    let symbol = input.book_crosscode().into_owned();
                    self.stated_ticker
                        .entry(symbol.clone())
                        .or_insert_with(|| input.get_ticker().is_some_and(|held| !held.is_empty()));
                    touched.insert(symbol.clone());
                    if input
                        .as_event()
                        .is_none_or(|event| event.get_snapunix().is_none())
                    {
                        raw.entry(symbol).or_default().push(input);
                        continue;
                    }
                    snapshot_views.insert(symbol.clone());
                    match input {
                        MarketData::OrderEvent(_) | MarketData::QuoteEvent(_) => {
                            snapshot_partitions
                                .entry(symbol.clone())
                                .or_default()
                                .insert(SnapshotPartition::of(&input));
                            let entries = if input.get_side().is_bid() {
                                snapshot_bid.entry(symbol).or_default()
                            } else {
                                snapshot_ask.entry(symbol).or_default()
                            };
                            entries.push(input);
                        }
                        MarketData::SnapshotEvent(control) => {
                            snapshot_partitions
                                .entry(symbol.clone())
                                .or_default()
                                .insert(SnapshotPartition::at(
                                    control.event.get_ticker(),
                                    control.book.scope.as_deref().unwrap_or(""),
                                ));
                            snapshot_controls.entry(symbol).or_default().push(control);
                        }
                        mut input => {
                            let validated = match input.as_event() {
                                Some(event_view) => validate_component_times(
                                    unix,
                                    "snapshot.executions",
                                    std::iter::once(event_view),
                                    false,
                                ),
                                None => Err(invalid(
                                    "$.operation",
                                    "expected a dated operation on a book",
                                )),
                            };
                            if let Err(error) = validated {
                                source_errors.entry(symbol).or_insert(error);
                                continue;
                            }
                            match &mut input {
                                MarketData::TradeEvent(trade) => {
                                    trade.rebase_currunix(unix);
                                }
                                MarketData::ExecutionEvent(execution) => {
                                    execution.set_currunix(unix);
                                    execution.finalize();
                                }
                                _ => {}
                            }
                            raw.entry(symbol).or_default().push(input);
                        }
                    }
                }
            }

            for operations in raw.values_mut() {
                order_chains(operations);
            }
            let mut failed = HashSet::new();
            for symbol in &touched {
                // add_operations is already atomic. Only a group that also
                // replaces supplied membership needs an outer transaction.
                if !snapshot_partitions.contains_key(symbol)
                    && !source_errors.contains_key(symbol)
                    && let Some(book) = self.books.get_mut(symbol)
                {
                    if let Err(error) = raw
                        .remove(symbol)
                        .map_or(Ok(()), |operations| book.add_operations(operations))
                    {
                        excluded(&error, symbol);
                        failed.insert(symbol.clone());
                    }
                    continue;
                }
                let mut next = self.books.get(symbol).cloned().unwrap_or_else(|| {
                    if self.stated_ticker.get(symbol).copied().unwrap_or(true) {
                        BookEvent::new(unix, symbol.clone())
                    } else {
                        BookEvent::categorized(unix, symbol.clone())
                    }
                });
                let mut result = if let Some(error) = source_errors.remove(symbol) {
                    Err(error)
                } else {
                    raw.remove(symbol)
                        .map_or(Ok(()), |operations| next.add_operations(operations))
                };
                if result.is_ok()
                    && let Some(partitions) = snapshot_partitions.get(symbol)
                {
                    let bid = snapshot_bid.remove(symbol).unwrap_or_default();
                    let ask = snapshot_ask.remove(symbol).unwrap_or_default();
                    let controls = snapshot_controls.remove(symbol).unwrap_or_default();
                    result =
                        next.replace_snapshot_membership(bid, ask, &controls, partitions, unix);
                }
                match result {
                    Ok(()) => {
                        self.books.insert(symbol.clone(), next);
                    }
                    Err(error) => {
                        excluded(&error, symbol);
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
                    self.pending.push_back(Ok(book.emit()));
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
    I::Item: Into<Result<MarketData>>,
{
    type Item = Result<BookEvent>;

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
    I::Item: Into<Result<MarketData>>,
{
}

/// The instant a dated input states; every input `foldable` admits is
/// dated, so the zero is never read.
fn input_unix(input: &MarketData) -> i64 {
    input.as_event().map_or(0, Event::get_currunix)
}

fn effective_unix(input: &MarketData) -> i64 {
    match input {
        MarketData::TradeEvent(trade) => {
            trade.get_snapunix().unwrap_or_else(|| trade.get_currunix())
        }
        MarketData::SnapshotEvent(control) => control
            .event
            .get_snapunix()
            .unwrap_or_else(|| control.event.get_currunix()),
        _ => input
            .as_event()
            .and_then(Event::get_snapunix)
            .unwrap_or_else(|| input.as_event().map_or(0, Event::get_currunix)),
    }
}

/// Refuses, at `path`, every variant a book does not fold: a book takes an
/// order, a quote, an execution, a trade or a snapshot control, each dated.
fn foldable(input: &MarketData, path: impl FnOnce() -> SmolStr) -> Result<()> {
    if matches!(
        input,
        MarketData::OrderEvent(_)
            | MarketData::QuoteEvent(_)
            | MarketData::ExecutionEvent(_)
            | MarketData::TradeEvent(_)
            | MarketData::SnapshotEvent(_)
    ) {
        return Ok(());
    }
    Err(invalid(
        path(),
        format_smolstr!(
            "expected order_event, quote_event, execution_event, trade_event or snapshot_event, got {}",
            input.kind().as_str()
        ),
    ))
}

fn is_execution_input(input: &MarketData) -> bool {
    matches!(
        input,
        MarketData::ExecutionEvent(_) | MarketData::TradeEvent(_)
    )
}

fn is_full_snapshot(input: &MarketData) -> bool {
    input.book().and_then(|book| book.action) == Some(MdUpdateAction::Snapshot)
}

fn is_ordinary_update(input: &MarketData) -> bool {
    !matches!(input, MarketData::SnapshotEvent(_))
        && !is_full_snapshot(input)
        && !input
            .book()
            .and_then(|book| book.action)
            .is_some_and(MdUpdateAction::is_range_delete)
}

fn merge_book_side(
    reference: &Ladder,
    supplement: &Ladder,
    replaced: &BTreeSet<SnapshotPartition>,
    authoritative: bool,
) -> Option<Ladder> {
    let mut live = reference
        .levels
        .values()
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    let mut live_keys = live
        .iter()
        .map(|operation| LiveKey::of(operation))
        .collect::<HashSet<_>>();
    live.extend(
        supplement
            .levels
            .values()
            .flatten()
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
    Ladder::from_shared_parts(
        reference.side,
        live.into_iter().enumerate().collect(),
        deltas.into_iter().enumerate().collect(),
    )
    .ok()
}

fn position_of(operation: &MarketData) -> Option<u64> {
    operation
        .book()
        .and_then(|book| book.position)
        .map(u64::from)
}

fn entry_px_of(operation: &MarketData) -> Option<Decimal> {
    operation.book().and_then(|book| book.entry_px)
}

fn entry_size_of(operation: &MarketData) -> Option<Decimal> {
    operation.book().and_then(|book| book.entry_size)
}

fn range_position(operation: &MarketData) -> Result<usize> {
    let Some(position) = operation.book().and_then(|book| book.position) else {
        return Err(invalid(
            "$.operation.book.mdentrypositionno",
            "expected a positive position for delete-through or delete-from",
        ));
    };
    usize::try_from(position)
        .ok()
        .filter(|position| *position > 0)
        .ok_or_else(|| {
            invalid(
                "$.operation.book.mdentrypositionno",
                format_smolstr!("expected a positive position, got {position:?}"),
            )
        })
}

fn same_partition(left: &MarketData, right: &MarketData) -> bool {
    scope_of(left) == scope_of(right) && left.get_ticker() == right.get_ticker()
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

fn decimal_mean(left: Decimal, right: Decimal) -> Option<Decimal> {
    let left = left.units();
    let right = right.units();
    let units = left
        .checked_div(2)?
        .checked_add(right.checked_div(2)?)?
        .checked_add((left % 2).checked_add(right % 2)?.checked_div(2)?)?;
    Decimal::from_units(units)
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
                .get_exprunix()
                .is_some_and(|expiration| expiration <= book_unix)
        {
            return Err(invalid(
                path("exprunix"),
                format_smolstr!(
                    "expected a live expiration after {book_unix}, got {:?}",
                    operation.get_exprunix()
                ),
            ));
        }
    }
    Ok(())
}

/// The dated order or quote each book-side entry is, for the checks that
/// read an entry's clocks.
fn entries<'a>(
    entries: impl IntoIterator<Item = &'a MarketData>,
) -> impl Iterator<Item = &'a dyn super::operation::BookOperation> {
    entries.into_iter().map(MarketData::operation_event)
}

fn validate_propagation_bounds<'a, E, I>(
    event: &MarketEventFacts,
    name: &str,
    operations: I,
) -> Result<()>
where
    E: Event + Market + ?Sized + 'a,
    I: IntoIterator<Item = &'a E>,
{
    for (index, operation) in operations.into_iter().enumerate() {
        let path = |field: &str| format_smolstr!("$.{name}[{index}].{field}");
        if operation.get_currunix() == event.get_currunix()
            && event.get_seqnum() < operation.get_seqnum()
        {
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
    if let Some(component) = component
        && root.is_none_or(|root| root > component)
    {
        return Err(invalid(
            path,
            format_smolstr!("expected the book bound at or before {component}, got {root:?}"),
        ));
    }
    Ok(())
}

fn validate_latest_bound(root: Option<i64>, component: Option<i64>, path: SmolStr) -> Result<()> {
    if let Some(component) = component
        && root.is_none_or(|root| root < component)
    {
        return Err(invalid(
            path,
            format_smolstr!("expected the book bound at or after {component}, got {root:?}"),
        ));
    }
    Ok(())
}

/// A book's ticker, where it states one, and its crosscode: what an input
/// is checked against.
type BookKey<'a> = (Option<&'a str>, &'a str);

fn validate_symbols<'a, E, I>(book: BookKey<'_>, name: &str, operations: I) -> Result<()>
where
    E: Market + ?Sized + 'a,
    I: IntoIterator<Item = &'a E>,
{
    for (index, operation) in operations.into_iter().enumerate() {
        validate_symbol(book, operation, || format_smolstr!("$.{name}[{index}]"))?;
    }
    Ok(())
}

fn validate_symbol<E: Market + ?Sized>(
    (ticker, crosscode): BookKey<'_>,
    operation: &E,
    path: impl FnOnce() -> SmolStr,
) -> Result<()> {
    match book_mismatch(ticker, crosscode, operation) {
        Some(reason) => Err(invalid(path(), reason)),
        None => Ok(()),
    }
}

/// Why `operation` cannot stand in the book whose ticker is `ticker` and
/// whose crosscode is `crosscode`; `None` where it can. A ticker book takes
/// an input stating that ticker or none - a hand-built book is fed
/// ticker-less orders - and refuses another ticker; a categorized book,
/// stating no ticker, takes only an input keyed to it: a ticker-less input
/// of its market and classification, or one whose ticker spells its key.
fn book_mismatch<E: Market + ?Sized>(
    ticker: Option<&str>,
    crosscode: &str,
    operation: &E,
) -> Option<SmolStr> {
    let stated = operation.get_ticker().filter(|held| !held.is_empty());
    match ticker {
        Some(ticker) => {
            let stated = stated?;
            (stated != ticker).then(|| format_smolstr!("expected {ticker:?}, got {stated:?}"))
        }
        None => {
            let key = operation.book_crosscode();
            let crosscode = super::market::base_crosscode(crosscode);
            if key == crosscode {
                return None;
            }
            Some(match stated {
                Some(stated) => {
                    format_smolstr!("expected book crosscode {crosscode:?}, got ticker {stated:?}")
                }
                None => format_smolstr!("expected book crosscode {crosscode:?}, got {key:?}"),
            })
        }
    }
}

/// Says that the operations of one book at one instant were left out,
/// because the book refused them as `error` says: the book stands as it was
/// and the walk goes on. A book's refusal is always of what the operations
/// state; the source's own failure never reaches here.
fn excluded(error: &Error, book: &str) {
    let at = match error {
        Error::InvalidRecord { path, .. } => path.as_str(),
        _ => "$.operations",
    };
    warned!(
        "book update excluded: the book refused what its operations state",
        at,
        "{error}, on book {book}; the book stands as it was"
    );
}

fn invalid(path: impl Into<SmolStr>, reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: path.into(),
        reason: reason.into(),
    }
}
