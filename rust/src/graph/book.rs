//! Typed market operations and a stateful, price-ordered market book.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::Hasher;
use std::iter::FusedIterator;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::facts::{MarketEventFacts, MarketFacts, OperationEventFacts};
use super::market::merge_market_event_into_reference;
use super::market_data::MarketData;
use super::operation::{BookRef, ExecutionEvent, MdUpdateAction, OrderKind, QuoteKind};
use super::{Element, Event, Market, Operation};
use crate::{Ccy, Decimal, Error, Limit, Result, Side, State, Unit, Uuid};

/// The symbol of the one consolidated book emitted in global mode.
pub const GLOBAL_SYMBOL: &str = "GLOBAL";

/// The alternate-identifier key an entry's own `MDEntryID(278)` is held under.
pub const ENTRY_ID: &str = "MDENTRYID";
/// The alternate-identifier key an entry's `MDEntryRefID(280)` is held under.
pub const ENTRY_REF_ID: &str = "MDENTRYREFID";
/// The alternate-identifier key an order's `OrderID(37)` is held under.
const ORDER_ID: &str = "ORDERID";

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
    pub(crate) fn from_control(event: MarketEventFacts, book: BookRef) -> Self {
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

/// One scope a full snapshot replaces: a symbol - none in global mode -
/// and the book scope the entries stated.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SnapshotPartition {
    /// The symbol the snapshot is for, where the operations name one.
    pub symbol: Option<SmolStr>,
    /// The book scope, empty where the operations state none.
    pub scope: SmolStr,
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
fn level_quantity(level: &[Arc<MarketData>]) -> Option<Decimal> {
    level.iter().try_fold(Decimal::ZERO, |sum, operation| {
        sum.checked_add(operation.get_quantity().unwrap_or(Decimal::ZERO))
    })
}

/// The book scope an entry states, empty where it states none: the seam
/// over [`MarketData::book`] every book-side comparison reads through.
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
        _ => unreachable!("a book side holds only order or quote events"),
    }
}

/// [`promote_to_order`], to a quote.
fn promote_to_quote(operation: MarketData, data: OperationEventFacts) -> MarketData {
    match operation {
        MarketData::OrderEvent(event) => MarketData::QuoteEvent(event.with_kind::<QuoteKind>(data)),
        MarketData::QuoteEvent(event) => MarketData::QuoteEvent(event.with_kind::<QuoteKind>(data)),
        _ => unreachable!("a book side holds only order or quote events"),
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
    element: MarketFacts,
    levels: BTreeMap<BookPrice, Vec<Arc<MarketData>>>,
    /// Derived from `levels` and kept in step by every change, shared with
    /// clones until one changes: a book emitted per instant copies no index,
    /// and the side the walk keeps changes its own in place once the
    /// emitted book is gone.
    index: Arc<SideIndex>,
    deltas: Vec<MarketData>,
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
        let entry = operation.operation_event().get_altids().get(ENTRY_ID)?;
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
    element: MarketFacts,
    deltas_len: usize,
    cleared_deltas: Option<Vec<MarketData>>,
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
        let mut element = MarketFacts::default();
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

    /// Rebuilds one canonical side from its serialized parts without
    /// replaying the serialized deltas as fresh mutations.
    pub(crate) fn from_parts(
        element: MarketFacts,
        live: Vec<MarketData>,
        deltas: Vec<MarketData>,
    ) -> Result<Self> {
        Self::from_shared_parts(element, live.into_iter().map(Arc::new).collect(), deltas)
    }

    /// [`Self::from_parts`] over live entries already shared, which a side
    /// rebuilt around its own entries keeps rather than copies.
    fn from_shared_parts(
        element: MarketFacts,
        live: Vec<Arc<MarketData>>,
        deltas: Vec<MarketData>,
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

    /// The live orders and quotes, best price first and every entry
    /// stating no price last.
    pub fn live(&self) -> impl Iterator<Item = &MarketData> {
        self.levels
            .values()
            .flat_map(|level| level.iter().map(Arc::as_ref))
    }

    /// Deltas applied since this side was last cleared.
    #[must_use]
    pub fn deltas(&self) -> &[MarketData] {
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

    /// The best live price on this side: the first priced level's, `None`
    /// for an empty side or one holding only unpriced entries.
    #[must_use]
    pub fn best_price(&self) -> Option<Decimal> {
        self.best_level().map(|(price, _)| price)
    }

    /// The aggregate quantity at the best price: the exact sum of what the
    /// entries of the first priced level state, an entry stating none adding
    /// nothing; `None` where [`Self::best_price`] is.
    #[must_use]
    pub fn best_quantity(&self) -> Option<Decimal> {
        self.best_level().map(|(_, level)| {
            // `canonical_element` refuses a level past decimal at every
            // refresh, and every change refreshes or rolls back.
            level_quantity(level)
                .expect("a refreshed side holds no level whose quantity overflows decimal")
        })
    }

    /// The first priced level: the first key, since the unpriced level
    /// sorts after every priced one.
    fn best_level(&self) -> Option<(Decimal, &[Arc<MarketData>])> {
        let (key, level) = self.levels.first_key_value()?;
        Some((key.price()?, level))
    }

    /// One [`Limit`] per level, best first and the unpriced limit last:
    /// its price, the exact sum of its entries' quantities, and their
    /// `curruuid`s in position order, ties in arrival order.
    ///
    /// One pass over the levels in the order they are held, allocating each
    /// limit's `uuids` and nothing else.
    ///
    /// ```
    /// use yggdryl::graph::{BookSide, Element, Event, Market, MarketData, OrderEvent};
    /// use yggdryl::{Decimal, Side, State};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = |code: &str, price: Option<i64>, quantity: i64| {
    ///     let mut order = OrderEvent::at(1);
    ///     order.set_crosscode(code.to_owned());
    ///     order.set_side(Side::read("Buy").unwrap());
    ///     order.set_price(price.map(Decimal::from_int));
    ///     order.set_quantity(Some(Decimal::from_int(quantity)));
    ///     order.set_state(State::read("New").unwrap());
    ///     order.finalize();
    ///     MarketData::from(order)
    /// };
    /// let mut side = BookSide::new(Side::read("Buy")?)?;
    /// side.add_operation(order("A", Some(100), 2))?;
    /// side.add_operation(order("MARKET", None, 5))?;
    /// side.add_operation(order("B", Some(101), 3))?;
    /// side.add_operation(order("C", Some(101), 4))?;
    ///
    /// let limits: Vec<_> = side.limits().collect();
    /// assert_eq!(limits.len(), 3);
    /// assert_eq!(limits[0].price, Some(Decimal::from_int(101)));
    /// assert_eq!(limits[0].quantity, Decimal::from_int(7));
    /// assert_eq!(limits[0].uuids.len(), 2);
    /// assert_eq!(limits[1].price, Some(Decimal::from_int(100)));
    /// // The market order rests after every priced level.
    /// assert_eq!(limits[2].price, None);
    /// assert_eq!(limits[2].quantity, Decimal::from_int(5));
    /// assert_eq!(side.best_price(), Some(Decimal::from_int(101)));
    /// # Ok(())
    /// # }
    /// ```
    pub fn limits(&self) -> impl Iterator<Item = Limit> {
        self.levels.iter().map(|(key, level)| Limit {
            price: key.price(),
            quantity: level_quantity(level)
                .expect("a refreshed side holds no level whose quantity overflows decimal"),
            uuids: level
                .iter()
                .map(|operation| operation.get_curruuid())
                .collect(),
        })
    }

    /// The exact sum of the first `levels` limits' quantities in
    /// [`Self::limits`] order, the unpriced limit counted where it is
    /// reached: zero for an empty side or no level, `None` only past
    /// decimal.
    ///
    /// ```
    /// use yggdryl::graph::{BookSide, Element, Event, Market, MarketData, OrderEvent};
    /// use yggdryl::{Decimal, Side, State};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = |code: &str, price: Option<i64>, quantity: i64| {
    ///     let mut order = OrderEvent::at(1);
    ///     order.set_crosscode(code.to_owned());
    ///     order.set_side(Side::read("Sell").unwrap());
    ///     order.set_price(price.map(Decimal::from_int));
    ///     order.set_quantity(Some(Decimal::from_int(quantity)));
    ///     order.set_state(State::read("New").unwrap());
    ///     order.finalize();
    ///     MarketData::from(order)
    /// };
    /// let mut side = BookSide::new(Side::read("Sell")?)?;
    /// side.add_operation(order("A", Some(102), 2))?;
    /// side.add_operation(order("B", Some(101), 3))?;
    /// side.add_operation(order("MARKET", None, 5))?;
    ///
    /// assert_eq!(side.depth(0), Some(Decimal::ZERO));
    /// assert_eq!(side.depth(1), Some(Decimal::from_int(3)));
    /// assert_eq!(side.depth(2), Some(Decimal::from_int(5)));
    /// assert_eq!(side.depth(3), Some(Decimal::from_int(10)));
    /// assert_eq!(side.depth(100), Some(Decimal::from_int(10)));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn depth(&self, levels: usize) -> Option<Decimal> {
        self.levels
            .values()
            .take(levels)
            .try_fold(Decimal::ZERO, |sum, level| {
                sum.checked_add(level_quantity(level)?)
            })
    }

    /// Atomically applies one order or quote delta.
    ///
    /// An entry stating no price - a market order - is given none: it rests
    /// at the one unpriced level, after every priced level, so [`Self::live`],
    /// the side's digest and a delete-from or delete-through position all
    /// reach it last, and [`Self::limits`] answers it as the last limit.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidRecord`] for any variant but an order or a quote, an
    /// entry of the other side, an update the side cannot place, or a level
    /// whose aggregate quantity would pass decimal (at `$.quantity`); the
    /// side is unchanged on every error.
    pub fn add_operation(&mut self, operation: MarketData) -> Result<()> {
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
                .get_altids()
                .contains_key(ENTRY_ID)
            && !operation
                .operation_event()
                .get_altids()
                .contains_key(ENTRY_REF_ID)
            && occupied_position()
        {
            return Err(invalid(
                "$.operation.book.mdentrypositionno",
                "a new anonymous entry cannot occupy an existing position; state MDEntryID or send a snapshot",
            ));
        }
        let referenced = self.referenced_identity_of(&operation).or_else(|| {
            let wanted = operation.operation_event().get_altids().get(ENTRY_REF_ID)?;
            other_side
                .filter(|previous| {
                    same_partition(previous, &operation)
                        && previous.operation_event().get_altids().get(ENTRY_ID) == Some(wanted)
                })
                .map(LiveKey::of)
        });
        if operation
            .operation_event()
            .get_altids()
            .contains_key(ENTRY_REF_ID)
            && referenced.is_none()
        {
            return Err(invalid(
                "$.operation.altids.MDENTRYREFID",
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
                "$.operation.altids.MDENTRYID",
                "the referenced entry and destination entry are both live",
            ));
        }
        let previous = self.get(&identity).or(other_side);
        if partial && previous.is_none() {
            if entry_px_of(&operation).is_none() {
                return Err(invalid(
                    "$.operation.book.mdentrypx",
                    "a partial update without a live predecessor must state its price",
                ));
            }
            if entry_size_of(&operation).is_none() {
                return Err(invalid(
                    "$.operation.book.mdentrysize",
                    "a partial update without a live predecessor must state its size",
                ));
            }
        }
        if let Some(previous) = previous {
            if let (Some(stated), Some(known)) = (
                operation.operation_event().get_altids().get(ORDER_ID),
                previous.operation_event().get_altids().get(ORDER_ID),
            ) {
                if stated != known {
                    return Err(invalid(
                        "$.operation.altids.ORDERID",
                        format_smolstr!("expected the live order {known:?}, got {stated:?}"),
                    ));
                }
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
                        .get_altids()
                        .contains_key(ORDER_ID) =>
                {
                    Some(true)
                }
                (true, false)
                    if partial
                        && operation
                            .operation_event()
                            .get_altids()
                            .contains_key(ORDER_ID)
                        && !previous
                            .operation_event()
                            .get_altids()
                            .contains_key(ORDER_ID) =>
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
                    operation.set_price(previous.get_price());
                }
                if entry_size_of(&operation).is_none() {
                    operation.set_quantity(previous.get_quantity());
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
        if operation.operation_event().get_state().is_live()
            && operation
                .operation_event()
                .get_exprtime()
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
            .get_altids()
            .get(ENTRY_ID)
            .and_then(|wanted| self.find_entry_identity(operation, wanted))
            .unwrap_or(own)
    }

    fn referenced_identity_of(&self, operation: &MarketData) -> Option<LiveKey> {
        operation
            .operation_event()
            .get_altids()
            .get(ENTRY_REF_ID)
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
                    && held.operation_event().get_altids().get(ENTRY_ID) == Some(wanted))
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
                "$.operation.book.mdentrypositionno",
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

    pub(super) fn canonical_element(&self) -> Result<MarketFacts> {
        let mut element = self.element.clone();
        element.set_price(None);
        element.set_quantity(None);
        element.set_currency(Ccy::none());
        element.set_unit(Unit::none());
        // Every level is summed, so a refreshed side holds none whose
        // quantity overflows: `best_quantity` and `limits` rely on it. The
        // digest below walks every entry anyway, so this stays linear.
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
        // The summary is the first priced level's; with none, no price and
        // no quantity are stated - never a zero.
        if let Some((price, level)) = self.best_level() {
            let first = &level[0];
            element.set_price(Some(price));
            element.set_quantity(level_quantity(level));
            element.set_currency(first.get_currency().clone());
            element.set_unit(first.get_unit().clone());
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

/// The facts a book-side entry's data holds, whichever kind it is - never
/// mutated in place through this borrow, only read or cloned for the next
/// reconstruction.
fn operation_event_data(operation: &MarketData) -> &OperationEventFacts {
    operation.operation_event().facts()
}

fn finalize_side_element(
    element: &mut MarketFacts,
    levels: &BTreeMap<BookPrice, Vec<Arc<MarketData>>>,
    live_len: usize,
    deltas: &[MarketData],
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
            staged.write(operation.operation_event().operation_word().as_bytes());
            staged.write(&operation.get_curruuid().get().to_be_bytes());
        }
        staged.write(&(deltas.len() as u64).to_be_bytes());
        for operation in deltas {
            staged.write(operation.operation_event().operation_word().as_bytes());
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

impl BookSide {
    /// The side's own facts.
    pub(crate) fn element(&self) -> &MarketFacts {
        &self.element
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

    /// A side's identity is content-derived, never dated: stores the code
    /// with no `curruuid` reprojection.
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

    fn get_srcuuids(&self) -> &[Uuid] {
        self.element.get_srcuuids()
    }

    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
        self.element.set_srcuuids(sources);
    }

    /// A side has no instant and no predecessor: it states no order.
    fn is_after(&self, _other: &Self) -> bool {
        false
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

delegate_market!(BookSide, element);

/// One coherent view of a market at one exact nanosecond instant.
#[derive(Clone, Debug, PartialEq)]
pub struct BookEvent {
    event: MarketEventFacts,
    bid: BookSide,
    ask: BookSide,
    executions: Vec<ExecutionEvent>,
    snapshots: BTreeSet<SnapshotPartition>,
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

    fn of_data(operation: &MarketData) -> Self {
        Self::of(operation.operation_event())
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
        event.set_ticker((!symbol.is_empty()).then(|| SmolStr::new(&symbol)));
        event.set_crosscode(symbol);
        event.set_state(State::from_spelling("New").expect("the shipped New state"));
        let mut book = Self {
            event,
            bid: BookSide::new(Side::read("Buy").expect("the shipped Buy side"))
                .expect("Buy is a bid"),
            ask: BookSide::new(Side::read("Sell").expect("the shipped Sell side"))
                .expect("Sell is an ask"),
            executions: Vec::new(),
            snapshots: BTreeSet::new(),
        };
        book.finalize();
        book
    }

    /// Rebuilds one canonical book from its serialized parts - the event,
    /// the two sides, the executions and the snapshot partitions replaced -
    /// validating that the sides, the executions and every symbol agree
    /// with the event, without replaying anything as a fresh mutation.
    pub(crate) fn from_parts(
        event: MarketEventFacts,
        bid: BookSide,
        ask: BookSide,
        executions: Vec<ExecutionEvent>,
        snapshots: BTreeSet<SnapshotPartition>,
    ) -> Result<Self> {
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
        let symbol = self.event.get_ticker();
        validate_symbols(symbol, "bid.live", self.bid.live())?;
        validate_symbols(symbol, "bid.deltas", self.bid.deltas().iter())?;
        validate_symbols(symbol, "ask.live", self.ask.live())?;
        validate_symbols(symbol, "ask.deltas", self.ask.deltas().iter())?;
        for (index, execution) in self.executions.iter().enumerate() {
            validate_symbol(symbol, execution, || {
                format_smolstr!("$.executions[{index}]")
            })?;
        }
        let unix = self.event.get_currunix();
        validate_component_times(unix, "bid.live", entries(self.bid.live()), true)?;
        validate_component_times(unix, "bid.deltas", entries(self.bid.deltas()), false)?;
        validate_component_times(unix, "ask.live", entries(self.ask.live()), true)?;
        validate_component_times(unix, "ask.deltas", entries(self.ask.deltas()), false)?;
        validate_component_times(
            self.event.get_currunix(),
            "executions",
            self.executions.iter(),
            false,
        )?;
        validate_propagation_bounds(&self.event, "bid.live", entries(self.bid.live()))?;
        validate_propagation_bounds(&self.event, "bid.deltas", entries(self.bid.deltas()))?;
        validate_propagation_bounds(&self.event, "ask.live", entries(self.ask.live()))?;
        validate_propagation_bounds(&self.event, "ask.deltas", entries(self.ask.deltas()))?;
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
    pub fn executions(&self) -> &[ExecutionEvent] {
        &self.executions
    }

    /// Whether the best bid is above the best ask; a locked book - the two
    /// equal - is not crossed, and a side stating no best crosses nothing.
    #[must_use]
    pub fn is_crossed(&self) -> bool {
        matches!(
            (self.bid.best_price(), self.ask.best_price()),
            (Some(bid), Some(ask)) if bid > ask
        )
    }

    /// Whether both sides state a best price and the two are equal.
    ///
    /// ```
    /// use yggdryl::graph::{BookEvent, Element, Event, Market, MarketData, OrderEvent};
    /// use yggdryl::{Decimal, Side, State};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = |code: &str, side: &str, price: i64| {
    ///     let mut order = OrderEvent::at(1);
    ///     order.set_crosscode(code.to_owned());
    ///     order.set_side(Side::read(side).unwrap());
    ///     order.set_price(Some(Decimal::from_int(price)));
    ///     order.set_quantity(Some(Decimal::from_int(1)));
    ///     order.set_state(State::read("New").unwrap());
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

    /// The best ask less the best bid: negative on a crossed book, which
    /// [`Self::is_crossed`] names; `None` where a side states no best price
    /// or the difference is past decimal.
    ///
    /// ```
    /// use yggdryl::graph::{BookEvent, Element, Event, Market, MarketData, OrderEvent};
    /// use yggdryl::{Decimal, Side, State};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = |code: &str, side: &str, price: &str| {
    ///     let mut order = OrderEvent::at(1);
    ///     order.set_crosscode(code.to_owned());
    ///     order.set_side(Side::read(side).unwrap());
    ///     order.set_price(Some(price.parse().unwrap()));
    ///     order.set_quantity(Some(Decimal::from_int(1)));
    ///     order.set_state(State::read("New").unwrap());
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
    /// `(bid - ask) / (bid + ask)` over their [`BookSide::depth`]s, from `1`
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
    ///     order.set_side(Side::read(side).unwrap());
    ///     order.set_price(Some(Decimal::from_int(price)));
    ///     order.set_quantity(Some(Decimal::from_int(quantity)));
    ///     order.set_state(State::read("New").unwrap());
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

    fn cached_median_quantity(&self, bid: &MarketFacts, ask: &MarketFacts) -> Option<Decimal> {
        median_quantity(
            (!self.bid.is_empty()).then(|| bid.get_quantity()).flatten(),
            (!self.ask.is_empty()).then(|| ask.get_quantity()).flatten(),
        )
    }

    /// Atomically applies all operations of one timestamp. Full-snapshot depth
    /// operations and controls first replace only their declared book scope;
    /// executions never control resting membership. A book folds an
    /// [`OrderEvent`](super::OrderEvent), a
    /// [`QuoteEvent`](super::QuoteEvent), an
    /// [`ExecutionEvent`], a
    /// [`TradeEvent`](super::TradeEvent) and a [`SnapshotEvent`].
    ///
    /// # Errors
    ///
    /// Returns the first item's own error, and [`Error::InvalidRecord`] for
    /// any other variant - naming its kind - or an operation the book
    /// refuses; the book is unchanged on every error.
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
        }
        next.event.set_currunix(unix);
        next.event.set_snapunix(Some(unix));
        for operation in bid.iter().chain(&ask) {
            next.fold_bounds(EventBounds::of_data(operation));
        }
        for control in controls {
            next.fold_bounds(EventBounds::of(&control.event));
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
        let symbol = input.get_ticker();
        if self.get_ticker() != Some(GLOBAL_SYMBOL)
            && symbol.is_some()
            && symbol != self.get_ticker()
        {
            return Err(invalid(
                "$.operation.ticker",
                format_smolstr!("expected {:?}, got {:?}", self.get_ticker(), symbol),
            ));
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
                bounds = EventBounds::of_data(
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

    pub(super) fn canonical_event(&self, bid: &MarketFacts, ask: &MarketFacts) -> MarketEventFacts {
        let mut event = self.event.clone();
        let midpoint = if self.is_crossed() {
            None
        } else {
            self.bbo_midpoint()
                .or_else(|| self.bid.best_price().or_else(|| self.ask.best_price()))
        };
        event.set_price(midpoint);
        event.set_quantity(self.cached_median_quantity(bid, ask));
        // A side speaks for the book only where it states a best: a side of
        // unpriced entries alone states no currency and no unit, and leaves
        // the other side's standing alone.
        let (bid_best, ask_best) = (bid.get_price().is_some(), ask.get_price().is_some());
        let currency = match (
            bid_best.then(|| bid.get_currency()),
            ask_best.then(|| ask.get_currency()),
        ) {
            (Some(bid), Some(ask)) if bid == ask => bid.clone(),
            (Some(currency), None) | (None, Some(currency)) => currency.clone(),
            _ => Ccy::none(),
        };
        event.set_currency(currency);
        let unit = match (
            bid_best.then(|| bid.get_unit()),
            ask_best.then(|| ask.get_unit()),
        ) {
            (Some(bid), Some(ask)) if bid == ask => bid.clone(),
            (Some(unit), None) | (None, Some(unit)) => unit.clone(),
            _ => Unit::none(),
        };
        event.set_unit(unit);
        finalize_book_event(&mut event, bid, ask, &self.executions, &self.snapshots);
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
        self.event = self.canonical_event(self.bid.element(), self.ask.element());
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
        self.snapshots.clear();
    }

    fn record_snapshot_partitions(&mut self, partitions: &BTreeSet<SnapshotPartition>) {
        self.snapshots.extend(partitions.iter().cloned());
    }

    /// The scopes the book's last snapshot replaced, empty where the last
    /// change was an ordinary update.
    #[must_use]
    pub fn snapshot_partitions(&self) -> &BTreeSet<SnapshotPartition> {
        &self.snapshots
    }

    fn set_book_time(&mut self, unix: i64, snapshot: bool) {
        self.event.set_currunix(unix);
        self.event.set_snapunix(snapshot.then_some(unix));
        self.finalize();
    }
}

fn median_quantity(bid: Option<Decimal>, ask: Option<Decimal>) -> Option<Decimal> {
    match (bid, ask) {
        (Some(bid), Some(ask)) => decimal_mean(bid, ask),
        (Some(quantity), None) | (None, Some(quantity)) => Some(quantity),
        (None, None) => None,
    }
}

fn finalize_book_event(
    event: &mut MarketEventFacts,
    bid: &MarketFacts,
    ask: &MarketFacts,
    executions: &[ExecutionEvent],
    snapshots: &BTreeSet<SnapshotPartition>,
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
    // The scopes the last snapshot replaced are part of what the book
    // states: a book after a replacement is not the book before it.
    for partition in snapshots {
        digest.write(partition.symbol.as_deref().unwrap_or("").as_bytes());
        digest.write(&[0]);
        digest.write(partition.scope.as_bytes());
        digest.write(&[0]);
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
        finalize_book_event(
            &mut self.event,
            self.bid.element(),
            self.ask.element(),
            &self.executions,
            &self.snapshots,
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
        let replaced = self.snapshot_partitions().clone();
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
        let merged = Self::from_parts(event, bid, ask, executions, replaced.clone()).ok()?;
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

/// Books from a sorted operation stream, one per symbol and effective
/// timestamp, or one consolidated `GLOBAL` book. Each timestamp and symbol is
/// committed atomically across ordinary deltas, expirations, and explicit
/// snapshot membership.
pub struct BookIterator<I>
where
    I: Iterator,
    I::Item: Into<Result<MarketData>>,
{
    source: I,
    source_head: Option<Result<MarketData>>,
    source_exhausted: bool,
    books: BTreeMap<String, BookEvent>,
    pending: VecDeque<Result<BookEvent>>,
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
    I::Item: Into<Result<MarketData>>,
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
            Some(operation) => {
                self.source_head = Some(operation.into().and_then(|operation| {
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
                let Some(unix) = operation.operation_event().get_exprtime() else {
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
                || operation.operation_event().get_exprtime() != Some(unix)
                || !operation.operation_event().get_state().is_live()
            {
                continue;
            }
            let mut operation = operation.clone();
            operation.operation_event_mut().set_currunix(unix);
            operation
                .operation_event_mut()
                .set_state(State::read("expired").expect("the shipped expired state"));
            let mut book = operation
                .operation_event()
                .control()
                .cloned()
                .unwrap_or_default();
            book.action = Some(MdUpdateAction::Delete);
            operation.operation_event_mut().set_control(Some(book));
            // The synthetic delete addresses the current generation, not
            // the reference a prior rename used to reach its predecessor.
            let _ = operation.operation_event_mut().remove_altid(ENTRY_REF_ID);
            operation.operation_event_mut().set_execunix(None);
            operation.operation_event_mut().set_recdunix(None);
            operation.operation_event_mut().set_snapunix(None);
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
                    let symbol = if self.global {
                        GLOBAL_SYMBOL.to_owned()
                    } else if let Some(symbol) = input.get_ticker() {
                        symbol.to_owned()
                    } else {
                        self.pending.push_back(Err(invalid(
                            "$.operation.ticker",
                            "expected a symbol outside global mode",
                        )));
                        continue;
                    };
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
                    .unwrap_or_else(|| BookEvent::new(unix, symbol.clone()));
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
    E: Market + ?Sized + 'a,
    I: IntoIterator<Item = &'a E>,
{
    for (index, operation) in operations.into_iter().enumerate() {
        validate_symbol(book_symbol, operation, || {
            format_smolstr!("$.{name}[{index}]")
        })?;
    }
    Ok(())
}

fn validate_symbol<E: Market + ?Sized>(
    book_symbol: Option<&str>,
    operation: &E,
    path: impl FnOnce() -> SmolStr,
) -> Result<()> {
    let operation_symbol = operation.get_ticker();
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
