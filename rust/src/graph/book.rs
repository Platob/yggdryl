//! Typed market data and a stateful, price-ordered market book.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::Hasher;
use std::iter::FusedIterator;
use std::ops::Range;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::arrow::{ALIVE, DELTAS, RowFilter};
use super::facts::{MarketEventFacts, OperationEventFacts};
use super::market::merge_market_event_into_reference;
use super::market_data::MarketData;
use super::operation::{BookRef, MdUpdateAction, OrderKind, QuoteKind};
use super::{Element, Event, Market, Operation};
use crate::expression::IntoFilter;
use crate::logging::warning::warned;
use crate::xxhash::Xxh3;
use crate::{
    Ccy, Decimal, Error, IdKey, IdType, Identifier, Isin, Limit, Result, Side, State, Unit, Uuid,
    i256,
};

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
/// operations state no ticker - and the book scope the entries stated. A
/// group replacing one is a snapshot, which states its instant and is
/// authoritative, so nothing keeps the scope past the group.
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

    /// Whether `operation` stands in this partition: [`Self::of`] compared
    /// over borrowed fields.
    fn holds(&self, operation: &MarketData) -> bool {
        operation.get_ticker() == self.symbol.as_deref() && scope_of(operation) == self.scope
    }
}

/// One live entry's deadline, scheduled once by its identity whichever
/// sides it rests on: the generation it was scheduled for, so a later
/// statement of the entry is not expired by an earlier deadline.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct BookExpiration {
    unix: i64,
    book: String,
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

    /// Whether `operation` is this identity: [`Self::of`] compared over
    /// borrowed fields, so a scan builds no key per entry it passes.
    fn matches(&self, operation: &MarketData) -> bool {
        operation.get_crossuuid() == self.identity
            && operation.get_ticker() == self.symbol.as_deref()
            && scope_of(operation) == self.scope
    }
}

impl BookPrice {
    /// The level an entry of the bid side (`bid`) or of the ask side
    /// stating `price` rests at.
    fn on(bid: bool, price: Option<Decimal>) -> Self {
        match price {
            None => Self::Unpriced,
            Some(price) if bid => Self::Bid(Reverse(price)),
            Some(price) => Self::Ask(price),
        }
    }

    const fn price(self) -> Option<Decimal> {
        match self {
            Self::Bid(Reverse(price)) | Self::Ask(price) => Some(price),
            Self::Unpriced => None,
        }
    }
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

/// One leg of a book entry: the price, the quantity and the currency it
/// rests on one side with.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Leg<'a> {
    pub(crate) px: Option<Decimal>,
    pub(crate) qty: Option<Decimal>,
    pub(crate) ccy: &'a Ccy,
}

/// The leg `entry` rests on the bid side (`bid`) or the ask side with: the
/// one rule every placement, level, reading and filter of a book reads.
/// A sided entry - an order, an execution - rests its price, quantity and
/// currency on the side it takes; any other - a quote, which holds its bid
/// and its ask and tags a side - rests each leg it states a price or a
/// quantity of, in that leg's currency, else its own. A leg stating a zero
/// quantity rests nowhere: a feed withdraws a level by sizing it zero.
pub(crate) fn leg(entry: &MarketData, bid: bool) -> Option<Leg<'_>> {
    stated_leg(entry, bid).filter(|leg| !leg.qty.is_some_and(Decimal::is_zero))
}

/// The leg `entry` states on the bid side (`bid`) or the ask side, sized
/// zero or not: [`leg`] before a zero quantity withdraws it.
fn stated_leg(entry: &MarketData, bid: bool) -> Option<Leg<'_>> {
    if entry.is_sided() {
        let side = entry.get_side();
        if !(if bid { side.is_bid() } else { side.is_ask() }) {
            return None;
        }
        return Some(Leg {
            px: entry.get_price(),
            qty: entry.get_quantity(),
            ccy: entry.get_currency(),
        });
    }
    let (px, qty, ccy) = if bid {
        (entry.get_bidpx(), entry.get_bidqty(), entry.get_bidccy())
    } else {
        (entry.get_askpx(), entry.get_askqty(), entry.get_askccy())
    };
    (px.is_some() || qty.is_some()).then(|| Leg {
        px,
        qty,
        ccy: ccy.unwrap_or_else(|| entry.get_currency()),
    })
}

/// Whether `entry` rests on the side `side` takes ([`leg`]); nothing rests
/// on a side that is neither a bid nor an ask.
pub(crate) fn rests_on(entry: &MarketData, side: Side) -> bool {
    (side.is_bid() || side.is_ask()) && leg(entry, side.is_bid()).is_some()
}

/// Whether `entry` is about the side `side` takes, resting there or not: a
/// sided entry taking it, an unsided one stating a leg there - a leg sized
/// zero, which withdraws it, included - or tagging it. What a delta that
/// takes an entry off a side still states, which the audit keeps by.
#[cfg(feature = "http")]
pub(crate) fn states_on(entry: &MarketData, side: Side) -> bool {
    (side.is_bid() || side.is_ask())
        && (stated_leg(entry, side.is_bid()).is_some()
            || (!entry.is_sided()
                && (if side.is_bid() {
                    entry.get_side().is_bid()
                } else {
                    entry.get_side().is_ask()
                })))
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

/// One side of a book as the walk keeps it: the live orders and quotes
/// resting on it in book order - best leg price first and the unpriced
/// level last, a level ordered by stated position with arrival breaking
/// ties - held as one contiguous store, a level a run of equal
/// [`BookPrice`]; a two-sided quote shares its one entry with the other
/// side. Walk state
/// rather than a value: a book answers a side as its price levels through
/// [`BookEvent::limits`] and its entries through [`BookEvent::alive_on`].
///
/// The store is shared: a book a walk emits whole holds the store the
/// walk keeps, so emitting a deep book costs a reference count, and the
/// walk copies its side once at its next change - only while a consumer
/// still holds that book. Between them the walk emits its deltas alone and
/// changes the store it alone holds in place.
///
/// A change costs a binary search over the side's levels, one scan of the
/// level it touches - where the entry an identity goes by is found - and a
/// move of the entry pointers behind the one that joins or leaves. Each
/// level keeps its quantity and how many of its entries cannot trade as
/// entries join, restate and leave, so checking a level against decimal,
/// settling the top of book and every reading of a level read no entry. A
/// delete by position range, an anonymous entry stating a position and an
/// `MDEntryID` more than one live entry goes by scan the side.
#[derive(Clone, Debug)]
struct Ladder {
    /// `BUYS` for the bid side, `SELL` for the ask side.
    side: Side,
    /// The live entries in book order, beside the level each rests at.
    store: Arc<Store>,
    /// Derived from the store and kept in step by every change, shared with
    /// clones until one changes, as the store is.
    index: Arc<SideIndex>,
}

/// Two sides are equal by what they hold; the index is derived from it.
impl PartialEq for Ladder {
    fn eq(&self, other: &Self) -> bool {
        self.side == other.side
            && (Arc::ptr_eq(&self.store, &other.store) || self.store.entries == other.store.entries)
    }
}

/// One side's live entries in book order and its levels, each a price,
/// where its run of entries ends and what that run adds up to: a level is
/// read without comparing or reading an entry, found by one binary search
/// over the levels, and how many a side holds is the length of a vector.
#[derive(Clone, Debug, Default)]
struct Store {
    /// The live entries in book order.
    entries: Vec<Arc<MarketData>>,
    /// Each level in book order.
    levels: Vec<StoreLevel>,
}

/// One level of a side's store: its price, the index past its last entry -
/// the next level's first - and what its entries add up to, kept in step as
/// entries join, restate and leave, so its quantity and whether it trades
/// are read without reading an entry.
#[derive(Clone, Copy, Debug)]
struct StoreLevel {
    price: BookPrice,
    end: usize,
    /// The exact sum of its entries' [`leg`] quantities in decimal units:
    /// wide enough that no count of entries passes it, so an entry leaving
    /// takes off exactly what it added; past decimal only where it is read
    /// ([`Level::quantity`]).
    units: i256,
    /// How many of its entries state `tradable = false`.
    halted: usize,
}

impl StoreLevel {
    /// An empty level of `price` opening at `at`.
    const fn open(price: BookPrice, at: usize) -> Self {
        Self {
            price,
            end: at,
            units: i256::ZERO,
            halted: 0,
        }
    }

    /// Counts `operation` in, on the bid side (`bid`) or the ask side.
    fn join(&mut self, operation: &MarketData, bid: bool) {
        let (units, halted) = weight(operation, bid);
        self.units = self
            .units
            .checked_add(units)
            .expect("fewer than 2^128 entries rest at a level");
        self.halted += halted;
    }

    /// Counts `operation` out, as it was counted in.
    fn leave(&mut self, operation: &MarketData, bid: bool) {
        let (units, halted) = weight(operation, bid);
        self.units = self
            .units
            .checked_sub(units)
            .expect("an entry leaves with what it joined with");
        self.halted -= halted;
    }

    /// The level over `entries`, its run of the store.
    fn level<'a>(&self, entries: &'a [Arc<MarketData>]) -> Level<'a> {
        Level {
            price: self.price.price(),
            entries,
            units: self.units,
            halted: self.halted,
        }
    }
}

/// What `operation` adds to its level on the bid side (`bid`) or the ask
/// side: its leg's quantity in decimal units, and one where it cannot
/// trade.
fn weight(operation: &MarketData, bid: bool) -> (i256, usize) {
    let units = leg(operation, bid)
        .and_then(|leg| leg.qty)
        .map_or(0, Decimal::units);
    let halted = operation.operation_event().get_tradable() == Some(false);
    (i256::from_i128(units), usize::from(halted))
}

impl Store {
    /// The store of `entries`, already in book order, on the bid side
    /// (`bid`) or the ask side.
    fn new(bid: bool, entries: Vec<Arc<MarketData>>) -> Self {
        let mut levels: Vec<StoreLevel> = Vec::new();
        for (at, operation) in entries.iter().enumerate() {
            let price = BookPrice::on(bid, leg(operation, bid).and_then(|leg| leg.px));
            if levels.last().is_none_or(|last| last.price != price) {
                levels.push(StoreLevel::open(price, at));
            }
            let level = levels.last_mut().expect("the entry's level was opened");
            level.end = at + 1;
            level.join(operation, bid);
        }
        Self { entries, levels }
    }

    /// Where the level of `price` stands among the levels, or where it
    /// would stand.
    fn rank(&self, price: BookPrice) -> std::result::Result<usize, usize> {
        self.levels
            .binary_search_by(|level| level.price.cmp(&price))
    }

    /// The index of the first entry of the level at `rank`, or of the
    /// level that would stand there.
    fn start(&self, rank: usize) -> usize {
        rank.checked_sub(1)
            .map_or(0, |before| self.levels[before].end)
    }

    /// The entries resting at `price`: empty, where it would open, when no
    /// entry does.
    fn level(&self, price: BookPrice) -> Range<usize> {
        match self.rank(price) {
            Ok(rank) => self.start(rank)..self.levels[rank].end,
            Err(rank) => self.start(rank)..self.start(rank),
        }
    }

    /// The level at `rank`, read off what it keeps.
    fn level_at(&self, rank: usize) -> Level<'_> {
        let level = &self.levels[rank];
        level.level(&self.entries[self.start(rank)..level.end])
    }

    /// Stands `operation`, resting at `price` on the bid side (`bid`) or
    /// the ask side, at `at` - within the range [`Self::level`] answers for
    /// `price` - opening its level where none stands.
    fn insert(&mut self, bid: bool, at: usize, price: BookPrice, operation: Arc<MarketData>) {
        let rank = self.rank(price).unwrap_or_else(|rank| {
            self.levels.insert(rank, StoreLevel::open(price, at));
            rank
        });
        self.levels[rank].join(&operation, bid);
        for level in &mut self.levels[rank..] {
            level.end += 1;
        }
        self.entries.insert(at, operation);
    }

    /// Takes the entry at `at` off, and its level with it where it was the
    /// level's last.
    fn remove(&mut self, bid: bool, at: usize) -> Arc<MarketData> {
        let rank = self.levels.partition_point(|level| level.end <= at);
        for level in &mut self.levels[rank..] {
            level.end -= 1;
        }
        let operation = self.entries.remove(at);
        if self.start(rank) == self.levels[rank].end {
            self.levels.remove(rank);
        } else {
            self.levels[rank].leave(&operation, bid);
        }
        operation
    }

    /// Stands `operation` at `at` in place of the entry there, at its
    /// level: the entry it replaced.
    fn replace(&mut self, bid: bool, at: usize, operation: Arc<MarketData>) -> Arc<MarketData> {
        let rank = self.levels.partition_point(|level| level.end <= at);
        self.levels[rank].join(&operation, bid);
        let held = std::mem::replace(&mut self.entries[at], operation);
        self.levels[rank].leave(&held, bid);
        held
    }

    /// Keeps the entries `keep` answers `true` for, level by level in one
    /// pass, and the levels still holding one, each counting out what it
    /// dropped.
    fn retain(&mut self, bid: bool, mut keep: impl FnMut(&Arc<MarketData>) -> bool) {
        let (mut read, mut kept, mut levels) = (0, 0, 0);
        for rank in 0..self.levels.len() {
            let mut level = self.levels[rank];
            for at in read..level.end {
                if keep(&self.entries[at]) {
                    self.entries.swap(kept, at);
                    kept += 1;
                } else {
                    level.leave(&self.entries[at], bid);
                }
            }
            read = level.end;
            if kept > self.start(levels) {
                level.end = kept;
                self.levels[levels] = level;
                levels += 1;
            }
        }
        self.entries.truncate(kept);
        self.levels.truncate(levels);
    }
}

/// One price level of a side, borrowed from the side's store: what a
/// [`Limit`] states, read without building one.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Level<'a> {
    /// The level's price; `None` on the one level every unpriced entry
    /// rests at.
    pub(crate) price: Option<Decimal>,
    /// The entries resting at the level, in position order, ties in
    /// arrival order.
    pub(crate) entries: &'a [Arc<MarketData>],
    /// What its entries' legs on its side add up to, in decimal units.
    units: i256,
    /// How many of its entries state `tradable = false`.
    halted: usize,
}

impl Level<'_> {
    /// The level's quantity: the exact sum of what its entries' legs on
    /// its side state, a leg stating none adding nothing; `None` past
    /// decimal. Kept by its store as entries join and leave, read rather
    /// than summed.
    pub(crate) fn quantity(&self) -> Option<Decimal> {
        self.units.as_i128().and_then(Decimal::from_units)
    }

    /// The level's quantity on a side already checked: every refresh
    /// refuses a level past decimal, and every change refreshes or rolls
    /// back.
    pub(crate) fn checked_quantity(&self) -> Decimal {
        self.quantity()
            .expect("a refreshed side holds no level whose quantity overflows decimal")
    }

    /// Whether the level can trade: one of its entries does not state
    /// `tradable = false` - an entry stating nothing is a live order no
    /// venue halted, so only a level every entry of which states `false`
    /// cannot.
    pub(crate) fn tradable(&self) -> bool {
        self.halted < self.entries.len()
    }

    /// The level as the [`Limit`] a book's row states: its one allocation
    /// is the `uuids` vector.
    fn into_limit(self) -> Limit {
        Limit {
            price: self.price,
            quantity: self.checked_quantity(),
            uuids: self
                .entries
                .iter()
                .map(|operation| operation.get_curruuid())
                .collect(),
            tradable: self.tradable(),
        }
    }
}

/// The levels of one side in book order, read off the store's levels: a
/// level costs one step, compares no price and reads no entry, and how many
/// are left - what a collection or a `take` is sized by - is a length.
#[derive(Clone, Default)]
pub(crate) struct Levels<'a> {
    /// The levels left.
    levels: &'a [StoreLevel],
    /// The side's entries.
    entries: &'a [Arc<MarketData>],
    /// The index of the next level's first entry.
    start: usize,
}

impl<'a> Iterator for Levels<'a> {
    type Item = Level<'a>;

    fn next(&mut self) -> Option<Level<'a>> {
        let (level, rest) = self.levels.split_first()?;
        let entries = &self.entries[self.start..level.end];
        self.levels = rest;
        self.start = level.end;
        Some(level.level(entries))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.levels.len(), Some(self.levels.len()))
    }
}

impl ExactSizeIterator for Levels<'_> {}

impl FusedIterator for Levels<'_> {}

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

/// One `MDEntryID` within one book partition and one entry type: FIX
/// scopes an entry's id by its `MDEntryType(269)`, the side the entry
/// takes or tags ([`entry_side`]), so a bid and an offer going by one id
/// are two entries.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct EntryKey {
    partition: SnapshotPartition,
    entry: SmolStr,
    side: Side,
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
            side: entry_side(operation),
        })
    }
}

/// The entry type an entry's `MDEntryID` is scoped by: `BUYS` for a bid,
/// `SELL` for an ask, the side an order takes or a quote tags, and
/// `UNKN` for a quote tagging none.
fn entry_side(operation: &MarketData) -> Side {
    let side = operation.get_side();
    if side.is_bid() {
        Side::Buy
    } else if side.is_ask() {
        Side::Sell
    } else {
        Side::Unknown
    }
}

/// The live entries one [`EntryKey`] names: the one, or how many where
/// several do - which the walk over the side answers in its own order.
#[derive(Clone, Debug)]
enum EntrySlot {
    One(LiveKey),
    Many(usize),
}

/// A live entry a change took off its side, and where it stood in the
/// side's store, so a rollback stands it there again.
struct RemovedLive {
    identity: LiveKey,
    price: BookPrice,
    index: usize,
    operation: Arc<MarketData>,
}

/// What one journaled group did to one side, in the order it did it: a
/// side changes by removals and insertions, so undoing them in reverse
/// restores every entry at the place it held.
#[derive(Default)]
struct SideJournal {
    changes: Vec<SideChange>,
}

/// One change a journaled group made to one side.
enum SideChange {
    Inserted(LiveKey),
    Removed(RemovedLive),
    /// The entry of an identity replaced where it stood, at `at`: the one
    /// it replaced.
    Restated {
        at: usize,
        identity: LiveKey,
        operation: Arc<MarketData>,
    },
}

impl SideJournal {
    fn rollback(self, side: &mut Ladder) {
        for change in self.changes.into_iter().rev() {
            match change {
                SideChange::Inserted(identity) => {
                    let _ = side.take_removed(&identity);
                }
                SideChange::Removed(removed) => {
                    let index = removed.index.min(side.store.entries.len());
                    side.insert_live(index, removed.operation, removed.identity, removed.price);
                }
                SideChange::Restated {
                    at,
                    identity,
                    operation,
                } => {
                    let _ = side.swap_live(at, operation, &identity);
                }
            }
        }
    }
}

impl Ladder {
    /// An empty side: `BUYS` is the bid, `SELL` the ask.
    fn new(side: Side) -> Self {
        Self {
            side,
            store: Arc::default(),
            index: Arc::default(),
        }
    }

    /// Rebuilds one canonical side from its live entries - each beside its
    /// position in the book's `alive` list, which a refusal names - in book
    /// order: by level, then stated position, then the order `order` lists
    /// their `curruuid`s in - a side's own order, which one `alive` list
    /// cannot state for both sides of a two-sided quote - then the order
    /// given.
    fn from_live(
        side: Side,
        live: Vec<(usize, Arc<MarketData>)>,
        order: Option<&[Uuid]>,
    ) -> Result<Self> {
        let mut ladder = Self::new(side);
        let mut built = SideIndex {
            positions: HashMap::with_capacity(live.len()),
            entry_ids: HashMap::new(),
        };
        let mut entries = Vec::with_capacity(live.len());
        for (index, operation) in live {
            let path = format_smolstr!("$.{ALIVE}[{index}]");
            ladder.validate_component(&operation, &path)?;
            let identity = LiveKey::of(&operation);
            if built
                .positions
                .insert(identity.clone(), ladder.price_of(&operation))
                .is_some()
            {
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
            entries.push(operation);
        }
        // Stable: entries of one level, one position and no stated rank keep
        // their order.
        match order {
            Some(order) => {
                let ranks: HashMap<Uuid, usize> = order
                    .iter()
                    .enumerate()
                    .map(|(rank, uuid)| (*uuid, rank))
                    .collect();
                entries.sort_by_cached_key(|held| {
                    let (price, position) = ladder.key_of(held);
                    let rank = ranks.get(&held.get_curruuid()).copied();
                    (price, position, rank.unwrap_or(usize::MAX))
                });
            }
            None => entries.sort_by_key(|held| ladder.key_of(held)),
        }
        ladder.store = Arc::new(Store::new(ladder.is_bid(), entries));
        ladder.index = Arc::new(built);
        ladder.check_levels()?;
        Ok(ladder)
    }

    /// Whether this is the bid side.
    fn is_bid(&self) -> bool {
        self.side.is_bid()
    }

    /// The level a live entry of this side rests at: the price of its
    /// [`leg`] on the side.
    fn price_of(&self, operation: &MarketData) -> BookPrice {
        BookPrice::on(
            self.is_bid(),
            leg(operation, self.is_bid()).and_then(|leg| leg.px),
        )
    }

    /// Where a live entry of this side stands in book order: its level,
    /// then its stated position, an unpositioned entry after every
    /// positioned one of its level.
    fn key_of(&self, operation: &MarketData) -> (BookPrice, u64) {
        (
            self.price_of(operation),
            position_of(operation).unwrap_or(u64::MAX),
        )
    }

    /// The live orders and quotes, best price first and every entry
    /// stating no price last.
    fn live(&self) -> impl ExactSizeIterator<Item = &MarketData> {
        self.store.entries.iter().map(Arc::as_ref)
    }

    fn len(&self) -> usize {
        self.store.entries.len()
    }

    /// The side's levels in book order: one run of the store each, best
    /// first and the unpriced level last.
    fn levels(&self) -> Levels<'_> {
        Levels {
            levels: &self.store.levels,
            entries: &self.store.entries,
            start: 0,
        }
    }

    /// The first entry at the first priced level, tradable or not, and its
    /// leg on this side: the one whose currency and unit the side speaks
    /// for the book in.
    fn best_entry(&self) -> Option<(&MarketData, Leg<'_>)> {
        let first = self.store.entries.first()?;
        let leg = leg(first, self.is_bid())?;
        leg.px.map(|_| (first.as_ref(), leg))
    }

    /// The first priced level that can trade, the unpriced level sorting
    /// after every priced one: the best price and the quantity there - none
    /// for an empty side, one holding only unpriced entries, or one no
    /// level of which can trade, never a price that cannot be traded.
    fn best_level(&self) -> Option<Level<'_>> {
        self.levels()
            .take_while(|level| level.price.is_some())
            .find(Level::tradable)
    }

    /// The exact sum of the first `levels` levels' quantities in book
    /// order, the unpriced level counted where it is reached: zero for an
    /// empty side or no level, `None` only past decimal.
    fn depth(&self, levels: usize) -> Option<Decimal> {
        self.levels()
            .take(levels)
            .try_fold(Decimal::ZERO, |sum, level| {
                sum.checked_add(level.quantity()?)
            })
    }

    /// Refuses, at `path`, an entry this side cannot hold alive: one that
    /// is no order or quote, states no live state, or rests on no leg of
    /// this side.
    fn validate_component(&self, operation: &MarketData, path: &str) -> Result<()> {
        validate_kind(operation, path)?;
        if !operation.operation_event().get_state().is_live() {
            return Err(invalid(
                format_smolstr!("{path}.state"),
                "expected every live operation to have a live state",
            ));
        }
        if !rests_on(operation, self.side) {
            return Err(invalid(
                format_smolstr!("{path}.side"),
                format_smolstr!(
                    "expected an entry resting on {:?}, got side {:?} stating no leg there",
                    self.side.as_str(),
                    operation.get_side().as_str()
                ),
            ));
        }
        check_leg_quantity(operation, self.is_bid(), path)
    }

    /// Whether the live entry of `identity` stands on this side.
    fn holds(&self, identity: &LiveKey) -> bool {
        self.index.positions.contains_key(identity)
    }

    /// Whether a live entry of `operation`'s partition stands on this side
    /// at the position `position` names.
    fn occupies(&self, operation: &MarketData, position: u64) -> bool {
        self.live()
            .any(|held| same_partition(held, operation) && position_of(held) == Some(position))
    }

    /// The live entry of `operation`'s partition going by the id `wanted`
    /// under the entry type `side`.
    fn find_entry_identity(
        &self,
        operation: &MarketData,
        wanted: &str,
        side: Side,
    ) -> Option<LiveKey> {
        let key = EntryKey {
            partition: SnapshotPartition::of(operation),
            entry: SmolStr::new(wanted),
            side,
        };
        match self.index.entry_ids.get(&key)? {
            EntrySlot::One(identity) => Some(identity.clone()),
            EntrySlot::Many(_) => self.live().find_map(|held| {
                (same_partition(held, operation)
                    && entry_side(held) == side
                    && held.operation_event().get_identifiers().get(&ENTRY_ID) == Some(wanted))
                .then(|| LiveKey::of(held))
            }),
        }
    }

    /// Stands `operation` live at `at` in the store, at the level of
    /// `price`, the index kept in step.
    fn insert_live(
        &mut self,
        at: usize,
        operation: Arc<MarketData>,
        identity: LiveKey,
        price: BookPrice,
    ) {
        let index = Arc::make_mut(&mut self.index);
        index.record(&operation, &identity);
        index.positions.insert(identity, price);
        let bid = self.is_bid();
        Arc::make_mut(&mut self.store).insert(bid, at, price, operation);
    }

    /// Where the live entry of `identity` stands in the store: its level
    /// found over the prices, then a scan of that level.
    fn find(&self, identity: &LiveKey) -> Option<(usize, BookPrice)> {
        let price = *self.index.positions.get(identity)?;
        let level = self.store.level(price);
        let start = level.start;
        self.store.entries[level]
            .iter()
            .position(|held| identity.matches(held))
            .map(|at| (start + at, price))
    }

    fn get(&self, identity: &LiveKey) -> Option<&MarketData> {
        self.find(identity)
            .map(|(at, _)| self.store.entries[at].as_ref())
    }

    fn take_removed(&mut self, identity: &LiveKey) -> Option<RemovedLive> {
        let (index, price) = self.find(identity)?;
        let bid = self.is_bid();
        let operation = Arc::make_mut(&mut self.store).remove(bid, index);
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

    /// Takes the live entry of `identity` off this side, journaled where a
    /// journal is kept; whether one stood here.
    fn take(&mut self, identity: &LiveKey, journal: Option<&mut SideJournal>) -> bool {
        let Some(removed) = self.take_removed(identity) else {
            return false;
        };
        if let Some(journal) = journal {
            journal.changes.push(SideChange::Removed(removed));
        }
        true
    }

    /// Stands `entry`, going by `identity`, on this side where its leg
    /// rests in book order - behind every entry of its level its position
    /// does not precede - journaled where a journal is kept, then checks the
    /// one level it joined: a side only grows a level by a placement, so a
    /// side every placement checked holds no level past decimal.
    ///
    /// # Errors
    ///
    /// Refuses, at `$.quantity`, the level whose aggregate quantity the
    /// entry takes past decimal; the entry stands, for the journal to undo.
    fn place(
        &mut self,
        entry: Arc<MarketData>,
        identity: LiveKey,
        journal: Option<&mut SideJournal>,
    ) -> Result<()> {
        let price = self.price_of(&entry);
        let position = position_of(&entry).unwrap_or(u64::MAX);
        let level = self.store.level(price);
        // Every entry of one level rests at its price: its positions alone
        // order it.
        let at = level.start
            + self.store.entries[level]
                .partition_point(|held| position_of(held).unwrap_or(u64::MAX) <= position);
        if let Some(journal) = journal {
            journal.changes.push(SideChange::Inserted(identity.clone()));
        }
        self.insert_live(at, entry, identity, price);
        self.check_level(price)
    }

    /// Replaces the live entry at `at` - going by `identity` - with
    /// `operation`, the index kept in step: the entry it replaced.
    fn swap_live(
        &mut self,
        at: usize,
        operation: Arc<MarketData>,
        identity: &LiveKey,
    ) -> Arc<MarketData> {
        let bid = self.is_bid();
        let index = Arc::make_mut(&mut self.index);
        let held = Arc::make_mut(&mut self.store).replace(bid, at, operation);
        index.forget(&held);
        index.record(&self.store.entries[at], identity);
        held
    }

    /// Restates the live entry of `identity` as `entry` where it stands,
    /// journaled where a journal is kept, when `entry` - going by the same
    /// identity - lands exactly there: at the same level and position, with
    /// no entry of that level and position behind it, which is where
    /// taking it off and placing it again would stand it. The step a
    /// restated quantity takes, moving no entry and no level; whether it
    /// applied, the side untouched where it did not.
    ///
    /// # Errors
    ///
    /// Refuses, as [`Self::place`] does, the level the entry takes past
    /// decimal; the entry stands, for the journal to undo.
    fn restate(
        &mut self,
        identity: &LiveKey,
        entry: &Arc<MarketData>,
        journal: Option<&mut SideJournal>,
    ) -> Result<bool> {
        let Some((at, price)) = self.find(identity) else {
            return Ok(false);
        };
        let position = position_of(entry).unwrap_or(u64::MAX);
        let level = self.store.level(price);
        let behind = self.store.entries[at + 1..level.end]
            .first()
            .is_some_and(|next| position_of(next).unwrap_or(u64::MAX) <= position);
        if !identity.matches(entry)
            || self.price_of(entry) != price
            || position_of(&self.store.entries[at]).unwrap_or(u64::MAX) != position
            || behind
        {
            return Ok(false);
        }
        let held = self.swap_live(at, Arc::clone(entry), identity);
        if let Some(journal) = journal {
            journal.changes.push(SideChange::Restated {
                at,
                identity: identity.clone(),
                operation: held,
            });
        }
        self.check_level(price).map(|()| true)
    }

    /// Refuses, at `$.quantity`, the level of `price` where what it keeps
    /// passes decimal - the one level a placement or a restatement changed
    /// - reading no entry.
    fn check_level(&self, price: BookPrice) -> Result<()> {
        match self.store.rank(price) {
            Ok(rank) if self.store.level_at(rank).quantity().is_none() => {
                Err(level_overflow(price.price()))
            }
            _ => Ok(()),
        }
    }

    /// Takes every live entry of `partition` off the side in one pass over
    /// the store, each kept in `cleared` under its identity; whether one
    /// was.
    fn clear_partition(
        &mut self,
        partition: &SnapshotPartition,
        cleared: &mut HashMap<LiveKey, Arc<MarketData>>,
    ) -> bool {
        if !self.live().any(|operation| partition.holds(operation)) {
            return false;
        }
        let bid = self.is_bid();
        let index = Arc::make_mut(&mut self.index);
        Arc::make_mut(&mut self.store).retain(bid, |operation| {
            if !partition.holds(operation) {
                return true;
            }
            let identity = LiveKey::of(operation);
            index.positions.remove(&identity);
            index.forget(operation);
            cleared.insert(identity, Arc::clone(operation));
            false
        });
        true
    }

    /// Replaces this side's entries of `partitions` with the `snapshot`
    /// entries resting on it, shared with the other side.
    fn replace_membership(
        &mut self,
        snapshot: &[Arc<MarketData>],
        partitions: &BTreeSet<SnapshotPartition>,
    ) -> Result<()> {
        let live = self
            .store
            .entries
            .iter()
            .filter(|operation| {
                !partitions
                    .iter()
                    .any(|partition| partition.holds(operation))
            })
            .chain(snapshot.iter().filter(|entry| rests_on(entry, self.side)))
            .cloned()
            .enumerate()
            .collect();
        *self = Self::from_live(self.side, live, None)?;
        Ok(())
    }

    /// Refuses a level whose aggregate quantity overflows decimal: every
    /// level is summed, so a checked side holds none, which
    /// [`Level::checked_quantity`] relies on. What a side built whole -
    /// read from a row, merged - is checked by; a side the book changes is
    /// checked a level at a time ([`Self::place`]).
    fn check_levels(&self) -> Result<()> {
        match self.levels().find(|level| level.quantity().is_none()) {
            Some(level) => Err(level_overflow(level.price)),
            None => Ok(()),
        }
    }

    /// The side's digest: its live count, then two facts per entry of a
    /// side that may be thousands deep, staged so the state reads them a
    /// chunk at a time. Read only where a book states a snapshot instant,
    /// so a book between snapshots never walks its sides.
    fn digest(&self) -> u64 {
        let mut digest = Xxh3::new();
        {
            let mut staged = super::element::Staged::new(&mut digest);
            staged.write(&(self.len() as u64).to_be_bytes());
            for operation in self.live() {
                staged.write(operation.operation_event().operation_word().as_bytes());
                staged.write(&operation.get_curruuid().get().to_be_bytes());
            }
        }
        digest.finish()
    }
}

/// Refuses, at `{path}.quantity`, an entry whose leg on the bid side
/// (`bid`) or the ask side states a negative quantity: a level adds only
/// what rests at it, so a side holds no negative weight and taking an entry
/// off can only shrink its level, which is what lets a removal go unchecked.
fn check_leg_quantity(entry: &MarketData, bid: bool, path: &str) -> Result<()> {
    match leg(entry, bid).and_then(|leg| leg.qty) {
        Some(quantity) if quantity < Decimal::ZERO => Err(invalid(
            format_smolstr!("{path}.quantity"),
            format_smolstr!("expected a quantity no less than zero, got {quantity}"),
        )),
        _ => Ok(()),
    }
}

/// The refusal of a level whose aggregate quantity - at `price`, or the
/// unpriced level's - passes decimal.
fn level_overflow(price: Option<Decimal>) -> Error {
    invalid(
        "$.quantity",
        match price {
            Some(price) => format_smolstr!(
                "expected the aggregate quantity at {price} to fit decimal, got an overflow"
            ),
            None => SmolStr::new_static(
                "expected the aggregate unpriced quantity to fit decimal, got an overflow",
            ),
        },
    )
}

/// The facts a book entry's data holds, whichever kind it is - never
/// mutated in place through this borrow, only read or cloned for the next
/// reconstruction.
fn operation_event_data(operation: &MarketData) -> &OperationEventFacts {
    operation.operation_event().facts()
}

/// Whether `applied` states what the live entry `held` does: the same kind,
/// the same facts ([`OperationEventFacts::same_statement`]) and the same
/// book control - a repeat, which changes nothing a book holds.
fn restates(applied: &MarketData, held: &MarketData) -> bool {
    std::mem::discriminant(applied) == std::mem::discriminant(held)
        && operation_event_data(applied).same_statement(operation_event_data(held))
        && applied.book() == held.book()
}

/// A book's two sides, each one store in book order: what a complete book
/// holds and a book stating only its deltas does not.
#[derive(Clone, Debug, PartialEq)]
struct Sides {
    bid: Ladder,
    ask: Ladder,
}

impl Sides {
    /// Two empty sides.
    fn new() -> Self {
        Self {
            bid: Ladder::new(Side::Buy),
            ask: Ladder::new(Side::Sell),
        }
    }

    /// The sides `alive` states, each entry standing as one on every side
    /// it rests on, each side in the order `orders` lists its entries'
    /// `curruuid`s - the bid's, then the ask's - where given, checked.
    ///
    /// # Errors
    ///
    /// Refuses an entry resting on no side at `$.alive[i]`, and whatever a
    /// side refuses.
    fn from_alive(alive: Vec<MarketData>, orders: [Option<&[Uuid]>; 2]) -> Result<Self> {
        let (mut bid_alive, mut ask_alive) = (Vec::new(), Vec::new());
        for (index, entry) in alive.into_iter().enumerate() {
            let (bid, ask) = (rests_on(&entry, Side::Buy), rests_on(&entry, Side::Sell));
            if !bid && !ask {
                validate_kind(&entry, &format_smolstr!("$.{ALIVE}[{index}]"))?;
                return Err(invalid(
                    format_smolstr!("$.{ALIVE}[{index}].side"),
                    format_smolstr!(
                        "expected an entry resting on the bid or the ask, got side {:?} stating no leg",
                        entry.get_side().as_str()
                    ),
                ));
            }
            let entry = Arc::new(entry);
            if bid {
                bid_alive.push((index, Arc::clone(&entry)));
            }
            if ask {
                ask_alive.push((index, entry));
            }
        }
        let [bid, ask] = orders;
        Ok(Self {
            bid: Ladder::from_live(Side::Buy, bid_alive, bid)?,
            ask: Ladder::from_live(Side::Sell, ask_alive, ask)?,
        })
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

    /// The bid side (`bid`) or the ask side.
    fn ladder_mut(&mut self, bid: bool) -> &mut Ladder {
        if bid { &mut self.bid } else { &mut self.ask }
    }

    /// Every entry alive, each once: the bid side's, then the ask side's
    /// but those resting on the bid too.
    fn alive(&self) -> impl Iterator<Item = &MarketData> {
        self.bid
            .live()
            .chain(self.ask.live().filter(|entry| !rests_on(entry, Side::Buy)))
    }

    /// Whether no entry rests on either side.
    fn is_empty(&self) -> bool {
        self.bid.len() == 0 && self.ask.len() == 0
    }

    /// How many entries [`Self::alive`] answers.
    fn alive_len(&self) -> usize {
        self.bid.len()
            + self
                .ask
                .live()
                .filter(|entry| !rests_on(entry, Side::Buy))
                .count()
    }

    /// The live entry of `identity`, on whichever side it rests.
    fn get(&self, identity: &LiveKey) -> Option<&MarketData> {
        self.bid.get(identity).or_else(|| self.ask.get(identity))
    }

    /// Refuses a level whose aggregate quantity overflows decimal, on
    /// either side.
    fn check_levels(&self) -> Result<()> {
        self.bid.check_levels()?;
        self.ask.check_levels()
    }

    /// Whether a live entry of `input`'s partition stands at the position
    /// it states, on a side it rests on.
    fn occupied(&self, input: &MarketData) -> bool {
        position_of(input).is_some_and(|position| {
            [&self.bid, &self.ask]
                .into_iter()
                .any(|ladder| rests_on(input, ladder.side) && ladder.occupies(input, position))
        })
    }

    /// The live identity `input` continues, looked up on both sides - an
    /// entry rests on every side it states a leg for, as one identity: the
    /// entry its `MDEntryRefID` names, else its own where it is live, else
    /// the entry its `MDEntryID` names; its own where none is live. An id
    /// names an entry of the input's entry type ([`entry_side`]) - else a
    /// quote tagging none, which holds both legs - so a new offer never
    /// continues a bid going by its id; only a change, an overlay or a
    /// delete finding none of its type continues the entry of the other
    /// type going by it, which it moves. An input stating no side names the
    /// one entry going by the id, of whichever type, where just one does.
    ///
    /// # Errors
    ///
    /// Refuses a reference naming no live entry, and a reference whose
    /// destination - the input's own entry, or the one its `MDEntryID`
    /// names - is another live entry.
    fn resolve(&self, input: &MarketData) -> Result<LiveKey> {
        let identifiers = input.operation_event().get_identifiers();
        let side = entry_side(input);
        let moves = input
            .operation_event()
            .control_action()
            .is_some_and(|action| {
                matches!(
                    action,
                    MdUpdateAction::Change | MdUpdateAction::Overlay | MdUpdateAction::Delete
                )
            });
        let named = |wanted: &str| -> Option<LiveKey> {
            let find = |side: Side| {
                [&self.bid, &self.ask]
                    .into_iter()
                    .find_map(|ladder| ladder.find_entry_identity(input, wanted, side))
            };
            if side != Side::Unknown {
                let other = if side == Side::Buy {
                    Side::Sell
                } else {
                    Side::Buy
                };
                return find(side)
                    .or_else(|| find(Side::Unknown))
                    .or_else(|| moves.then(|| find(other)).flatten());
            }
            let mut found = [Side::Unknown, Side::Buy, Side::Sell]
                .into_iter()
                .filter_map(find);
            let first = found.next()?;
            found.all(|other| other == first).then_some(first)
        };
        let referenced = match identifiers.get(&ENTRY_REF_ID) {
            Some(wanted) => Some(named(wanted).ok_or_else(|| {
                invalid(
                    "$.operation.identifiers.mdentryrefid",
                    "the referenced live entry does not exist in this symbol and scope",
                )
            })?),
            None => None,
        };
        let own = LiveKey::of(input);
        let destination = if self.bid.holds(&own) || self.ask.holds(&own) {
            Some(own)
        } else {
            identifiers.get(&ENTRY_ID).and_then(named)
        };
        match (referenced, destination) {
            (Some(referenced), Some(destination)) if referenced != destination => Err(invalid(
                "$.operation.identifiers.mdentryid",
                "the referenced entry and destination entry are both live",
            )),
            (Some(identity), _) | (None, Some(identity)) => Ok(identity),
            (None, None) => Ok(LiveKey::of(input)),
        }
    }

    /// Places `entry` in one step: `identity` - the live entry it continues,
    /// else its own - leaves both sides, and the entry, while live and
    /// unexpired, stands as one shared entry on every side it rests on,
    /// journaled where a journal is kept.
    ///
    /// # Errors
    ///
    /// Refuses a level the entry takes past decimal, as [`Ladder::place`]
    /// does.
    fn place(
        &mut self,
        identity: &LiveKey,
        entry: &Arc<MarketData>,
        mut journal: Option<&mut SidesJournal>,
    ) -> Result<()> {
        let event = entry.operation_event();
        let live = event.get_state().is_live()
            && event
                .get_exprunix()
                .is_none_or(|expiration| expiration > event.get_currunix());
        let own = LiveKey::of(entry);
        for bid in [true, false] {
            let rests = live && leg(entry, bid).is_some();
            if rests {
                check_leg_quantity(entry, bid, "$")?;
            }
            let ladder = self.ladder_mut(bid);
            let mut journal = journal.as_deref_mut().and_then(|journal| journal.side(bid));
            if rests && ladder.restate(identity, entry, journal.as_deref_mut())? {
                continue;
            }
            ladder.take(identity, journal.as_deref_mut());
            if rests {
                ladder.place(Arc::clone(entry), own.clone(), journal)?;
            }
        }
        Ok(())
    }

    /// Applies a delete-from or a delete-through (`through`): the positions
    /// it names of its partition on the side its entry type takes, each
    /// entry taken off every side it rests on - nothing for a range stating
    /// neither a bid nor an ask, which is warned of.
    ///
    /// # Errors
    ///
    /// Refuses a position that is not positive or past the partition's
    /// entries on that side.
    fn delete_range(
        &mut self,
        operation: &MarketData,
        through: bool,
        mut journal: Option<&mut SidesJournal>,
    ) -> Result<()> {
        let side = operation.get_side();
        if !side.is_bid() && !side.is_ask() {
            left_out(operation);
            return Ok(());
        }
        let position = range_position(operation)?;
        let ladder = if side.is_bid() { &self.bid } else { &self.ask };
        let identities: Vec<LiveKey> = ladder
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
        for (index, identity) in identities.iter().enumerate() {
            if (through && index <= at) || (!through && index >= at) {
                for bid in [true, false] {
                    let journal = journal.as_deref_mut().and_then(|journal| journal.side(bid));
                    self.ladder_mut(bid).take(identity, journal);
                }
            }
        }
        Ok(())
    }

    /// Takes every live entry of `partition` off both sides, each kept in
    /// `cleared` under its identity; whether one was.
    fn clear_partition(
        &mut self,
        partition: &SnapshotPartition,
        cleared: &mut HashMap<LiveKey, Arc<MarketData>>,
    ) -> bool {
        let bid = self.bid.clear_partition(partition, cleared);
        self.ask.clear_partition(partition, cleared) || bid
    }

    /// Replays one delta a book applied - its range, or its entry placed by
    /// the identity it continues - exactly as the book placed it: the same
    /// shared entry, with nothing followed again.
    ///
    /// # Errors
    ///
    /// Refuses what [`Self::resolve`], [`Self::place`] and
    /// [`Self::delete_range`] refuse.
    fn replay(&mut self, delta: &Arc<MarketData>) -> Result<()> {
        // An execution was recorded, never placed: replaying it places
        // nothing either.
        if matches!(delta.as_ref(), MarketData::ExecutionEvent(_)) {
            return Ok(());
        }
        match delta.operation_event().control_action() {
            Some(action) if action.is_range_delete() => {
                self.delete_range(delta, action == MdUpdateAction::DeleteThru, None)
            }
            _ => {
                let identity = self.resolve(delta)?;
                self.place(&identity, delta, None)
            }
        }
    }

    /// Settles the top-of-book facts `event` states on the sides: the best
    /// tradable level of each side as its price, quantity and currency, the
    /// book's price ([`book_price`]) and quantity ([`median_quantity`]) over
    /// them, and the currency and unit its best entries agree on.
    fn settle(&self, event: &mut MarketEventFacts) {
        let (bid, ask) = (self.bid.best_level(), self.ask.best_level());
        let (bidpx, askpx) = (
            bid.and_then(|level| level.price),
            ask.and_then(|level| level.price),
        );
        let (bidqty, askqty) = (
            bid.map(|level| level.checked_quantity()),
            ask.map(|level| level.checked_quantity()),
        );
        event.set_price(book_price(bidpx, askpx), true);
        event.set_quantity(median_quantity(bidqty, askqty), true);
        // A side speaks for the book only where it states a best: a side of
        // unpriced entries alone states no currency and no unit, and leaves
        // the other side's standing alone. The currency is the best leg's.
        let (bid, ask) = (self.bid.best_entry(), self.ask.best_entry());
        let currency = match (bid.map(|(_, leg)| leg.ccy), ask.map(|(_, leg)| leg.ccy)) {
            (Some(bid), Some(ask)) if bid == ask => bid.clone(),
            (Some(currency), None) | (None, Some(currency)) => currency.clone(),
            _ => Ccy::none(),
        };
        event.set_currency(currency, true);
        let (bid, ask) = (bid.map(|(entry, _)| entry), ask.map(|(entry, _)| entry));
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
        event.set_bidpx(bidpx, true);
        event.set_bidqty(bidqty, true);
        event.set_bidccy(bidpx.and(stated.clone()), true);
        event.set_askpx(askpx, true);
        event.set_askqty(askqty, true);
        event.set_askccy(askpx.and(stated), true);
    }
}

/// What one journaled group did to a book's sides: each side's changes in
/// the order made, or - for a group replacing membership, which clears
/// whole partitions - the sides as they stood, shared until the group
/// changes them.
enum SidesJournal {
    Each { bid: SideJournal, ask: SideJournal },
    Whole(Sides),
}

impl SidesJournal {
    /// The journal of the bid side (`bid`) or of the ask side; none where
    /// the sides are kept whole.
    fn side(&mut self, bid: bool) -> Option<&mut SideJournal> {
        match self {
            Self::Each { bid: journal, .. } if bid => Some(journal),
            Self::Each { ask: journal, .. } => Some(journal),
            Self::Whole(_) => None,
        }
    }

    fn rollback(self, sides: &mut Sides) {
        match self {
            Self::Each { bid, ask } => {
                bid.rollback(&mut sides.bid);
                ask.rollback(&mut sides.ask);
            }
            Self::Whole(whole) => *sides = whole,
        }
    }
}

/// One coherent view of a market at one exact nanosecond instant: the
/// orders and quotes alive on its two sides, and the deltas - every event
/// of its instant since the book before it, in the order applied: the
/// orders and quotes it applied, and the executions it recorded. An entry
/// rests on every side it states a leg for: an order on the side it takes,
/// a quote on its bid and its ask, a two-sided quote one entry shared by
/// both sides. An execution rests on no side and moves none - its fill
/// moved the book through its order's or quote's own report - and stands
/// among the deltas at its instant, the book's last execution instant
/// following it; every input
/// [`MarketDataKind::is_recorded`](crate::MarketDataKind::is_recorded) does
/// not admit is pruned before the fold.
///
/// A book is **complete** - [`Self::is_complete`] - where it holds its
/// sides: one a caller builds and changes with [`Self::add_operations`],
/// one a walk emits whole, or one rebuilt by [`Element::with_previous`]. A
/// [`BookIterator`] emits a book whole only at a snapshot tick - every
/// crossed grid tick, a group that replaced membership - and every other
/// book as its **deltas** alone, beside the top-of-book facts it settled
/// on: [`Element::with_previous`] over the complete book before it rebuilds
/// it, deltas replayed in the order applied, under the same identity - over
/// the empty book every walk starts from where it names no `prevuuid`, as
/// the first book of a book code does.
///
/// Each side of a complete book is one contiguous store in book order,
/// shared with the books a walk emits until the walk changes it, and every
/// reading borrows from it: [`Self::alive`], [`Self::alive_on`],
/// [`Self::deltas`]. A complete book answers each side as the [`Limit`]s
/// its row states under `bidlimits` and `asklimits` - [`Self::limits`], best
/// first - and its depth, [`Self::depth`] and [`Self::imbalance`]; a book
/// holding only its deltas answers none of them. Every book, complete or
/// not, answers its top of book from the facts it settled on, reading no
/// side: [`Self::best_price`], [`Self::best_quantity`], [`Self::spread`],
/// [`Self::bbo_midpoint`], [`Self::median_quantity`], [`Self::is_crossed`],
/// [`Self::is_locked`].
#[derive(Clone, Debug, PartialEq)]
pub struct BookEvent {
    event: MarketEventFacts,
    /// Both sides, on a complete book; none on a book stating its deltas
    /// alone.
    sides: Option<Sides>,
    /// Every event of the book's instant since the book before this one,
    /// in the order applied across both sides: the orders and quotes
    /// applied, each the very entry a side holds where it rests, and the
    /// executions recorded, resting nowhere.
    deltas: Vec<Arc<MarketData>>,
}

/// What one group did to a book: whether it applied an order or a quote
/// as a delta, whether it replaced the membership of a partition - a
/// snapshot - and, for a snapshot, whether it removed an entry it did not
/// restate and whether the book holds an entry after it.
#[derive(Clone, Copy, Debug, Default)]
struct Applied {
    deltas: bool,
    replaced: bool,
    removed: bool,
    holds: bool,
}

impl Applied {
    /// Whether the group changed the book: it recorded a delta, or it is a
    /// snapshot of a book holding an entry or one it emptied - an empty
    /// book replaced by nothing changes nothing.
    const fn any(self) -> bool {
        self.deltas || (self.replaced && (self.removed || self.holds))
    }
}

/// What one journaled group did to a book, so a refused group - or one
/// applying nothing - leaves it as it was: the event, the sides' journal,
/// and the deltas - their count before the group, or all of them where a
/// new instant took them.
struct BookJournal {
    event: MarketEventFacts,
    sides: SidesJournal,
    deltas_len: usize,
    taken: Option<Vec<Arc<MarketData>>>,
}

impl BookJournal {
    /// The journal of a group at `unix` on `book`, its sides kept `whole`
    /// where the group replaces membership; the deltas taken where the
    /// group starts a new instant.
    fn new(book: &mut BookEvent, unix: i64, whole: bool) -> Self {
        let advancing = book.event.get_currunix() != unix;
        Self {
            event: book.event.clone(),
            sides: match &book.sides {
                Some(sides) if whole => SidesJournal::Whole(sides.clone()),
                _ => SidesJournal::Each {
                    bid: SideJournal::default(),
                    ask: SideJournal::default(),
                },
            },
            deltas_len: book.deltas.len(),
            taken: advancing.then(|| std::mem::take(&mut book.deltas)),
        }
    }

    fn rollback(self, book: &mut BookEvent) {
        if let Some(sides) = &mut book.sides {
            self.sides.rollback(sides);
        }
        match self.taken {
            Some(deltas) => book.deltas = deltas,
            None => book.deltas.truncate(self.deltas_len),
        }
        book.event = self.event;
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

/// Folds a member's facts into the book's `event`: the highest place of
/// the members at the book's instant, a new instant having started the
/// book's own at zero, the earliest creation and recording, and the latest
/// execution.
fn fold_bounds(event: &mut MarketEventFacts, bounds: EventBounds) {
    if bounds.currunix == event.get_currunix() && bounds.seqnum > event.get_seqnum() {
        event.set_seqnum(bounds.seqnum);
    }
    event.set_creaunix(earliest(event.get_creaunix(), bounds.creaunix));
    event.set_recdunix(earliest(event.get_recdunix(), bounds.recdunix));
    event.set_execunix(latest(event.get_execunix(), bounds.execunix), true);
}

/// The book `head` was, as the one a book's `event` follows: its identity,
/// its instant, and the price and quantity it settled on; whether any
/// moved. The one link every book takes from the book before it.
fn follow_book(event: &mut MarketEventFacts, head: &MarketEventFacts) -> bool {
    let link = (
        Some(head.get_curruuid()),
        Some(head.get_currunix()),
        head.get_price(),
        head.get_quantity(),
    );
    if (
        event.get_prevuuid(),
        event.get_prevunix(),
        event.get_prevpx(),
        event.get_prevqty(),
    ) == link
    {
        return false;
    }
    event.set_prevuuid(link.0);
    event.set_prevunix(link.1);
    event.set_prevpx(link.2, true);
    event.set_prevqty(link.3, true);
    true
}

/// The first top-of-book fact `settled` - what the sides settle on - and
/// the book's `stated` event disagree on, by its column name; none where
/// they agree.
fn unsettled(stated: &MarketEventFacts, settled: &MarketEventFacts) -> Option<&'static str> {
    let decimals = [
        ("price", stated.get_price(), settled.get_price()),
        ("quantity", stated.get_quantity(), settled.get_quantity()),
        ("bidpx", stated.get_bidpx(), settled.get_bidpx()),
        ("bidqty", stated.get_bidqty(), settled.get_bidqty()),
        ("askpx", stated.get_askpx(), settled.get_askpx()),
        ("askqty", stated.get_askqty(), settled.get_askqty()),
    ];
    if let Some((name, ..)) = decimals
        .iter()
        .find(|(_, stated, settled)| stated != settled)
    {
        return Some(name);
    }
    if stated.get_currency() != settled.get_currency() {
        return Some("currency");
    }
    if stated.get_unit() != settled.get_unit() {
        return Some("unit");
    }
    if stated.get_bidccy() != settled.get_bidccy() {
        return Some("bidccy");
    }
    (stated.get_askccy() != settled.get_askccy()).then_some("askccy")
}

impl BookEvent {
    /// An empty book of the ticker `symbol` at one nanosecond instant: keyed
    /// by the ticker, which it states, and taking the inputs keyed to it -
    /// stating that ticker and no ISIN - and those stating neither ISIN nor
    /// ticker. An empty `symbol` keys the book [`Isin::NONE`], the number
    /// that states none, and states no ticker.
    #[must_use]
    pub fn new(unix: i64, symbol: impl Into<String>) -> Self {
        let symbol = symbol.into();
        if symbol.is_empty() {
            return Self::keyed(unix, Isin::NONE);
        }
        let mut book = Self::keyed(unix, symbol.as_str());
        book.event.set_ticker(Some(SmolStr::new(symbol)), true);
        book.finalize();
        book
    }

    /// An empty book keyed `key` at one nanosecond instant: `key` is its
    /// crosscode, what [`Market::book_crosscode`] answers for every input
    /// it takes - an instrument's ISIN, a ticker, or [`Isin::NONE`] - and
    /// the book states neither a ticker nor an ISIN until its first input
    /// states each. The empty base every walk starts a code's books from,
    /// which a code's first book, stating its deltas alone and following
    /// no book, rebuilds over with [`Element::with_previous`]:
    ///
    /// ```
    /// use yggdryl::graph::{BookEvent, BookIterator, Element, Event, Market, MarketData, OrderEvent};
    /// use yggdryl::{Decimal, IdKey, IdType, Identifier, Side, State};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut order = OrderEvent::at(1);
    /// order.set_crosscode("B-1".to_owned());
    /// order.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "CH0012214059")?)?;
    /// order.set_ticker(Some("HOLN".into()), true);
    /// order.set_side(Side::Buy, true);
    /// order.set_price(Some(Decimal::from_int(99)), true);
    /// order.set_quantity(Some(Decimal::ONE), true);
    /// order.set_state(State::New);
    /// order.finalize();
    /// let first = BookIterator::new(vec![MarketData::from(order)].into_iter(), 0)?
    ///     .next()
    ///     .expect("a book")?;
    /// assert_eq!(first.get_crosscode(), "3:0:CH0012214059");
    /// assert_eq!(first.get_isincode(), Some("CH0012214059"));
    /// assert_eq!(first.get_ticker(), Some("HOLN"));
    /// assert!(!first.is_complete());
    /// let whole = first.with_previous(&BookEvent::keyed(1, "CH0012214059")).expect("a rebuild");
    /// assert!(whole.is_complete());
    /// assert_eq!(whole.alive().count(), 1);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn keyed(unix: i64, key: impl Into<String>) -> Self {
        let mut event = MarketEventFacts::at(unix);
        event.set_marketdatakind(crate::MarketDataKind::Book);
        event.set_crosscode(key.into());
        event.set_state(State::New);
        let mut book = Self {
            event,
            sides: Some(Sides::new()),
            deltas: Vec::new(),
        };
        book.finalize();
        book
    }

    /// Takes the facts that spell the book's key from `input` where the
    /// book states none: its ticker, and its instrument's ISIN. The first
    /// input stating each decides, and no input moves them after.
    fn adopt_instrument(&mut self, input: &dyn Market) {
        if self.event.get_ticker().is_none()
            && let Some(ticker) = input.get_ticker().filter(|ticker| !ticker.is_empty())
        {
            self.event.set_ticker(Some(SmolStr::new(ticker)), false);
        }
        if self.event.get_isincode().is_none()
            && let Some(isin) = input.get_isincode()
            && let Ok(id) = Identifier::new(IdKey::base(IdType::Isin), isin)
        {
            let _ = self.event.insert_securityid(id);
        }
    }

    /// Rebuilds one canonical book from what its row states: the event, the
    /// entries alive on both sides - none for a book stating its deltas
    /// alone - and the deltas in the order applied, each entry standing as
    /// one on every side it rests on, each side in the order `orders` lists
    /// its entries' `curruuid`s where the row states it - its price levels
    /// do - validating that the entries, the deltas and every symbol agree
    /// with the event, without replaying anything as a fresh mutation. A
    /// complete book settles its top of book on its sides; a book stating
    /// its deltas alone keeps the one it states, its price and quantity
    /// those of its best bid and ask.
    ///
    /// # Errors
    ///
    /// Refuses an entry resting on no side at `$.alive[i]`, a delta that is
    /// no order or quote at `$.deltas[i]`, and whatever a side or the book
    /// refuses.
    pub(crate) fn from_parts(
        mut event: MarketEventFacts,
        alive: Option<Vec<MarketData>>,
        deltas: Vec<MarketData>,
        orders: [Option<&[Uuid]>; 2],
    ) -> Result<Self> {
        let deltas: Vec<Arc<MarketData>> = deltas.into_iter().map(Arc::new).collect();
        for (index, delta) in deltas.iter().enumerate() {
            validate_delta_kind(delta, &format_smolstr!("$.{DELTAS}[{index}]"))?;
        }
        // A book is not sided: its cross code stays as given.
        event.set_marketdatakind(crate::MarketDataKind::Book);
        if alive.is_none()
            && let Some(snapshot) = event.get_snapunix()
        {
            return Err(invalid(
                "$.snapunix",
                format_smolstr!(
                    "expected no snapshot instant on a book holding only its deltas, got {snapshot}"
                ),
            ));
        }
        let sides = alive
            .map(|alive| Sides::from_alive(alive, orders))
            .transpose()?;
        let mut book = Self {
            event,
            sides,
            deltas,
        };
        book.validate_parts()?;
        book.event = book.canonical_event()?;
        Ok(book)
    }

    pub(super) fn validate_parts(&self) -> Result<()> {
        let key = super::market::base_crosscode(self.event.get_crosscode());
        // The book's own facts spell its key, as every input's do.
        if let Some(reason) = book_mismatch(key, &self.event) {
            return Err(invalid(
                "$.crosscode",
                format_smolstr!(
                    "expected the book's isincode or ticker to spell its key: {reason}"
                ),
            ));
        }
        validate_symbols(key, ALIVE, self.alive())?;
        validate_symbols(key, DELTAS, self.deltas())?;
        let unix = self.event.get_currunix();
        validate_component_times(unix, ALIVE, entries(self.alive()), true)?;
        validate_component_times(unix, DELTAS, entries(self.deltas()), false)?;
        validate_propagation_bounds(&self.event, ALIVE, entries(self.alive()))?;
        validate_propagation_bounds(&self.event, DELTAS, entries(self.deltas()))
    }

    /// Whether the book holds its sides - every entry alive on it - rather
    /// than only the deltas it applied since the book before it: a book a
    /// caller builds, one a walk emits at a snapshot tick, and one rebuilt
    /// by [`Element::with_previous`] are complete.
    ///
    /// ```
    /// use yggdryl::graph::{BookEvent, BookIterator, Element, Event, Market, MarketData, OrderEvent};
    /// use yggdryl::{Decimal, Side, State};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = |code: &str, unix: i64, price: i64| {
    ///     let mut order = OrderEvent::at(unix);
    ///     order.set_crosscode(code.to_owned());
    ///     order.set_ticker(Some("ACME".into()), true);
    ///     order.set_side(Side::Buy, true);
    ///     order.set_price(Some(Decimal::from_int(price)), true);
    ///     order.set_quantity(Some(Decimal::ONE), true);
    ///     order.set_state(State::New);
    ///     order.finalize();
    ///     MarketData::from(order)
    /// };
    /// let books = BookIterator::new(vec![order("B-1", 1, 99), order("B-2", 2, 100)].into_iter(), 0)?
    ///     .collect::<yggdryl::Result<Vec<_>>>()?;
    /// // With no grid each book states its delta alone, beside the best bid
    /// // it settled on; the first follows no book.
    /// assert!(!books[0].is_complete() && !books[1].is_complete());
    /// assert_eq!(books[0].get_prevuuid(), None);
    /// assert_eq!(books[1].alive().count(), 0);
    /// assert_eq!(books[1].deltas().len(), 1);
    /// assert_eq!(books[1].best_price(Side::Buy), Some(Decimal::from_int(100)));
    /// // The first is whole over the empty book a walk starts from, and the
    /// // next over it, each under its own identity.
    /// let first = books[0].clone().with_previous(&BookEvent::new(1, "ACME")).expect("a rebuild");
    /// let rebuilt = books[1].clone().with_previous(&first).expect("a rebuild");
    /// assert!(first.is_complete() && rebuilt.is_complete());
    /// assert_eq!(rebuilt.alive().count(), 2);
    /// assert_eq!(rebuilt.get_curruuid(), books[1].get_curruuid());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.sides.is_some()
    }

    /// Every entry alive on the book, each once: the bid side's, best price
    /// first and every entry stating no price last, then the ask side's
    /// the same way but those resting on the bid too - a two-sided quote is
    /// one entry, listed with the bids. Nothing on a book stating its deltas
    /// alone.
    pub fn alive(&self) -> impl Iterator<Item = &MarketData> {
        self.sides.iter().flat_map(Sides::alive)
    }

    /// The entries alive on the side `side` takes, best price first and
    /// every entry stating no price last, borrowed from the side's store -
    /// a two-sided quote on both sides, at its leg's price on each; nothing
    /// for a side that is neither a bid nor an ask, or on a book stating its
    /// deltas alone.
    ///
    /// ```
    /// use yggdryl::graph::{BookEvent, Element, Event, Market, MarketData, OrderEvent};
    /// use yggdryl::{Decimal, Side, State};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = |code: &str, side: Side, price: i64| {
    ///     let mut order = OrderEvent::at(1);
    ///     order.set_crosscode(code.to_owned());
    ///     order.set_side(side, true);
    ///     order.set_price(Some(Decimal::from_int(price)), true);
    ///     order.set_quantity(Some(Decimal::ONE), true);
    ///     order.set_state(State::New);
    ///     order.finalize();
    ///     MarketData::from(order)
    /// };
    /// let mut book = BookEvent::new(1, "ACME");
    /// book.add_operations([
    ///     order("B-1", Side::Buy, 99),
    ///     order("A-1", Side::Sell, 101),
    ///     order("B-2", Side::Buy, 100),
    /// ])?;
    /// let bids: Vec<_> = book.alive_on(Side::Buy).map(Market::get_price).collect();
    /// assert_eq!(bids, [Some(Decimal::from_int(100)), Some(Decimal::from_int(99))]);
    /// assert_eq!(book.alive_on(Side::Sell).len(), 1);
    /// assert_eq!(book.alive_on(Side::Unknown).len(), 0);
    /// // The deltas are the three orders, in the order applied.
    /// let applied: Vec<_> = book.deltas().map(Element::get_crosscode).collect();
    /// assert_eq!(applied, ["10:1:B-1", "10:2:A-1", "10:1:B-2"]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn alive_on(&self, side: Side) -> impl ExactSizeIterator<Item = &MarketData> {
        self.ladder(side)
            .map_or(&[][..], |ladder| ladder.store.entries.as_slice())
            .iter()
            .map(Arc::as_ref)
    }

    /// Every event of the book's instant since the book before this one,
    /// in the order applied across both sides - the orders and quotes
    /// applied, and the executions recorded, which rest on no side: what a
    /// book stating its deltas alone states, and what
    /// [`Element::with_previous`] replays over the book before it, an
    /// execution placing nothing on the way.
    pub fn deltas(&self) -> impl ExactSizeIterator<Item = &MarketData> {
        self.deltas.iter().map(Arc::as_ref)
    }

    /// Whether the book holds no entry alive - a book stating its deltas
    /// alone states none.
    fn is_empty(&self) -> bool {
        self.sides.as_ref().is_none_or(Sides::is_empty)
    }

    /// How many entries [`Self::alive`] answers: every bid, and every ask
    /// that does not rest on the bid too, read without walking the bids.
    pub(crate) fn alive_len(&self) -> usize {
        self.sides.as_ref().map_or(0, Sides::alive_len)
    }

    /// The levels of the side `side` takes, borrowed from its store, best
    /// first and the unpriced level last: what [`Self::limits`] answers,
    /// read without building a [`Limit`]; nothing for a side that is
    /// neither a bid nor an ask, or on a book stating its deltas alone.
    pub(crate) fn levels(&self, side: Side) -> Levels<'_> {
        self.ladder(side)
            .map_or_else(Levels::default, Ladder::levels)
    }

    /// The side a bid or an ask names; none for a side that is neither, or
    /// on a book stating its deltas alone.
    fn ladder(&self, side: Side) -> Option<&Ladder> {
        self.sides.as_ref()?.ladder(side)
    }

    /// The live entry of `identity`, on whichever side it rests.
    fn live_entry(&self, identity: &LiveKey) -> Option<&MarketData> {
        self.sides.as_ref()?.get(identity)
    }

    /// The best tradable price on the side `side` takes: the first priced
    /// level's whose [`Limit::tradable`] holds, `None` for an empty side,
    /// one holding only unpriced entries, one no level of which can trade,
    /// or a side that is neither a bid nor an ask. A level that cannot trade
    /// is skipped, never answered. What the book states as `bidpx` (`BUYS`)
    /// and `askpx` (`SELL`), and read from it, so a book stating its deltas
    /// alone answers it too.
    #[must_use]
    pub fn best_price(&self, side: Side) -> Option<Decimal> {
        if side.is_bid() {
            self.event.get_bidpx()
        } else if side.is_ask() {
            self.event.get_askpx()
        } else {
            None
        }
    }

    /// The aggregate quantity at [`Self::best_price`]: the exact sum of
    /// what the entries of that level state on the side - an order its
    /// quantity, a quote that leg's - one stating none adding nothing. What
    /// the book states as `bidqty` and `askqty`, and read from it.
    #[must_use]
    pub fn best_quantity(&self, side: Side) -> Option<Decimal> {
        if side.is_bid() {
            self.event.get_bidqty()
        } else if side.is_ask() {
            self.event.get_askqty()
        } else {
            None
        }
    }

    /// One [`Limit`] per level of the side `side` takes, best first and the
    /// unpriced limit last: its price, the exact sum of its entries'
    /// quantities, their `curruuid`s in position order, ties in arrival
    /// order, and whether any of them does not state `tradable = false` - an
    /// entry stating nothing trades, and a level every entry of which states
    /// `false` cannot. Nothing for a side that is neither a bid nor an ask,
    /// or on a book stating its deltas alone. What a complete book's row
    /// states under `bidlimits` (`BUYS`) and `asklimits` (`SELL`); the first
    /// priced limit that can trade is [`Self::best_price`] and
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
        self.levels(side).map(Level::into_limit)
    }

    /// The exact sum of the first `levels` limits' quantities of the side
    /// `side` takes, in [`Self::limits`] order, the unpriced limit counted
    /// where it is reached: zero for an empty side or no level, `None` past
    /// decimal, for a side that is neither a bid nor an ask, or on a book
    /// stating its deltas alone.
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
            (self.event.get_bidpx(), self.event.get_askpx()),
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
            (self.event.get_bidpx(), self.event.get_askpx()),
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
        self.event.get_askpx()?.checked_sub(self.event.get_bidpx()?)
    }

    /// The order-book imbalance over the first `levels` limits of each side:
    /// `(bid - ask) / (bid + ask)` over their [`Self::depth`]s, from `1`
    /// for a book resting on the bid alone to `-1` on the ask alone; `None`
    /// where the total is zero - both sides empty, or no level - past
    /// decimal, or on a book stating its deltas alone.
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
        let sides = self.sides.as_ref()?;
        let bid = sides.bid.depth(levels)?;
        let ask = sides.ask.depth(levels)?;
        bid.checked_sub(ask)?.checked_div(bid.checked_add(ask)?)
    }

    /// The arithmetic midpoint of a coherent two-sided BBO.
    #[must_use]
    pub fn bbo_midpoint(&self) -> Option<Decimal> {
        let (bid, ask) = (self.event.get_bidpx()?, self.event.get_askpx()?);
        (bid <= ask).then(|| decimal_mean(bid, ask)).flatten()
    }

    /// The two-value median of the best bid and ask aggregate quantities.
    #[must_use]
    pub fn median_quantity(&self) -> Option<Decimal> {
        median_quantity(self.event.get_bidqty(), self.event.get_askqty())
    }

    /// Atomically applies all operations of one timestamp. Full-snapshot depth
    /// operations and controls first replace only their declared book scope.
    /// A book folds an [`OrderEvent`](super::OrderEvent), a
    /// [`QuoteEvent`](super::QuoteEvent) and a [`SnapshotEvent`], each
    /// applied order or quote recorded as a delta in the order applied.
    /// Every input of a kind
    /// [`MarketDataKind::is_booked`](crate::MarketDataKind::is_booked) does
    /// not admit - an [`ExecutionEvent`](super::ExecutionEvent), a
    /// [`TradeEvent`](super::TradeEvent) - is pruned first, so a group of
    /// nothing else changes nothing: the instant does not advance and the
    /// deltas stand.
    ///
    /// An order or a quote rests on every side it states a leg for, as one
    /// entry: an order on the side it takes, a quote - which holds a bid and
    /// an ask and tags a side - on each leg it states a price or a quantity
    /// of, so a two-sided quote stands on both sides and is listed once by
    /// [`Self::alive`]; a leg sized zero rests nowhere. Every order and quote
    /// is a delta wherever it rests: one resting on no side and continuing
    /// no live entry - warned of where it is live - one first seen ended and
    /// one ending an entry the book no longer holds place nothing, never
    /// refused, and still advance the book as its deltas.
    ///
    /// A statement repeating the live entry it continues - every fact the
    /// same but its identity, its digests and where it stands in its chain,
    /// and the same book control - is no change: it records no delta and
    /// leaves the entry as it stood, and a group of repeats changes nothing.
    /// A full snapshot restating an entry its scope held keeps that entry,
    /// identity and all, and one replacing an empty book by nothing changes
    /// nothing either.
    ///
    /// A group at a later instant advances the book: it follows the book it
    /// was, naming it as its `prevuuid` and `prevunix` and its price and
    /// quantity as its `prevpx` and `prevqty`, and its deltas start again.
    ///
    /// An entry stating no price - a market order - is given none: it rests
    /// at the one unpriced level of its side, after every priced level, so
    /// [`Self::alive`], the book's digest and a delete-from or delete-through
    /// position all reach it last, and [`Self::limits`] answers it as the
    /// side's last limit.
    ///
    /// # Errors
    ///
    /// Refuses, at `$.alive`, a book stating its deltas alone - rebuild it
    /// with [`Element::with_previous`] first. Returns the first item's own
    /// error, and [`Error::InvalidRecord`] for any other variant - naming its
    /// kind - an operation the book refuses, or a level whose aggregate
    /// quantity would pass decimal (at `$.quantity`); the book is unchanged
    /// on every error.
    pub fn add_operations<I>(&mut self, operations: I) -> Result<()>
    where
        I: IntoIterator,
        I::Item: Into<Result<MarketData>>,
    {
        let operations = operations
            .into_iter()
            .map(Into::into)
            .collect::<Result<Vec<MarketData>>>()?;
        if self.fold_group(operations)?.any() {
            self.finalize();
        }
        Ok(())
    }

    /// [`Self::add_operations`] over one instant's inputs in hand, every
    /// group journaled in place: what it applied, its top of book settled
    /// and its identity left for the caller to derive - a group of pruned
    /// and repeated inputs alone, or an empty book replaced by nothing,
    /// leaves the book as it was.
    fn fold_group(&mut self, mut operations: Vec<MarketData>) -> Result<Applied> {
        if self.sides.is_none() {
            return Err(invalid(
                format_smolstr!("$.{ALIVE}"),
                "a book holding only its deltas takes no operations: rebuild it with with_previous first",
            ));
        }
        operations.retain(recorded);
        if operations.is_empty() {
            return Ok(Applied::default());
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
        let partitions: BTreeSet<SnapshotPartition> = operations
            .iter()
            .filter(|input| is_full_snapshot(input))
            .map(SnapshotPartition::of)
            .collect();
        let mut journal = BookJournal::new(self, unix, !partitions.is_empty());
        let result = self.fold_journaled(operations, &partitions, unix, &mut journal.sides);
        match result {
            Ok(applied) if applied.any() => {
                self.settle();
                Ok(applied)
            }
            refused => {
                journal.rollback(self);
                refused
            }
        }
    }

    /// The body of [`Self::fold_group`] once its journal is open: the book
    /// advanced to `unix`, `partitions` cleared - what each held kept - then
    /// every input applied in order.
    fn fold_journaled(
        &mut self,
        operations: Vec<MarketData>,
        partitions: &BTreeSet<SnapshotPartition>,
        unix: i64,
        journal: &mut SidesJournal,
    ) -> Result<Applied> {
        self.advance(unix);
        self.event.set_snapunix(None);
        let mut cleared = HashMap::new();
        let sides = self.sides.as_mut().expect("a complete book folds");
        for partition in partitions {
            sides.clear_partition(partition, &mut cleared);
        }
        let mut applied = Applied {
            replaced: !partitions.is_empty(),
            ..Applied::default()
        };
        for operation in operations {
            applied.deltas |= self.apply_inner(operation, Some(&mut *journal), &cleared)?;
        }
        // A group replacing membership is a snapshot: the book states its
        // instant, and is authoritative where it is merged.
        if applied.replaced {
            self.event.set_snapunix(Some(unix));
            let sides = self.sides.as_ref().expect("a complete book folds");
            applied.removed = cleared.keys().any(|identity| sides.get(identity).is_none());
            applied.holds = !sides.is_empty();
        }
        Ok(applied)
    }

    /// Replaces the membership of `partitions` with the snapshot's
    /// `members`, each standing as one on every side it rests on - one
    /// resting on none, which no membership holds, warned of - its
    /// `controls` folded.
    fn replace_snapshot_membership(
        &mut self,
        members: Vec<MarketData>,
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
        validate_component_times(unix, "snapshot.members", entries(&members), true)?;
        validate_component_times(
            unix,
            "snapshot.controls",
            controls.iter().map(|control| &control.event),
            false,
        )?;
        let mut next = self.clone();
        next.advance(unix);
        next.event.set_snapunix(Some(unix));
        let mut snapshot = Vec::with_capacity(members.len());
        for entry in members {
            if !rests_on(&entry, Side::Buy) && !rests_on(&entry, Side::Sell) {
                left_out(&entry);
                continue;
            }
            next.adopt_instrument(&entry);
            fold_bounds(&mut next.event, EventBounds::of_data(&entry));
            snapshot.push(Arc::new(entry));
        }
        for control in controls {
            next.adopt_instrument(&control.event);
            fold_bounds(
                &mut next.event,
                EventBounds {
                    execunix: control.event.get_execunix(),
                    ..EventBounds::of(&control.event)
                },
            );
        }
        let sides = next.sides.as_mut().expect("a walk's book is complete");
        sides.bid.replace_membership(&snapshot, partitions)?;
        sides.ask.replace_membership(&snapshot, partitions)?;
        next.settle();
        *self = next;
        Ok(())
    }

    /// Applies one input: a snapshot control folds its bounds, a range
    /// delete takes positions off its side, and an order or a quote
    /// continues the live entry it resolves to and is placed on every side
    /// it rests on, one delta recording it. Every order and quote but a
    /// repeat is a delta, in the order applied, whether or not it rests
    /// anywhere: one ending an entry the book does not hold, one first seen
    /// ended, and one resting on no side - a live one warned of - place
    /// nothing, and a range stating neither side takes nothing off. Whether
    /// it recorded a delta: not for a control, nor for a repeat of the live
    /// entry it continues, or of the one a full snapshot cleared from its
    /// scope, which stands again as it was.
    fn apply_inner(
        &mut self,
        input: MarketData,
        mut journal: Option<&mut SidesJournal>,
        cleared: &HashMap<LiveKey, Arc<MarketData>>,
    ) -> Result<bool> {
        foldable(&input, || SmolStr::new_static("$.operation.kind"))?;
        let key = super::market::base_crosscode(self.get_crosscode());
        if let Some(reason) = book_mismatch(key, &input) {
            let at = if input.get_isincode().is_some() {
                "$.operation.isincode"
            } else {
                "$.operation.ticker"
            };
            return Err(invalid(at, reason));
        }
        self.adopt_instrument(&input);
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
        let Self {
            event,
            sides,
            deltas,
        } = self;
        // An execution moves no side - its fill moved the book through its
        // order's or quote's own report - and is recorded among the deltas
        // at its instant, the bounds it states following: the book's last
        // execution instant is its own.
        if matches!(input, MarketData::ExecutionEvent(_)) {
            fold_bounds(event, EventBounds::of_data(&input));
            deltas.push(Arc::new(input));
            return Ok(true);
        }
        let sides = sides.as_mut().expect("a complete book folds");
        if matches!(input, MarketData::SnapshotEvent(_)) {
            // When the input last executed is a market fact of the input itself.
            fold_bounds(
                event,
                EventBounds {
                    execunix: input.get_execunix(),
                    ..EventBounds::of(event_view)
                },
            );
            return Ok(false);
        }
        let action = input.operation_event().control_action();
        if let Some(action) = action.filter(|action| action.is_range_delete()) {
            sides.delete_range(&input, action == MdUpdateAction::DeleteThru, journal)?;
            fold_bounds(event, EventBounds::of_data(&input));
            deltas.push(Arc::new(input));
            return Ok(true);
        }
        if action == Some(MdUpdateAction::New)
            && !input
                .operation_event()
                .get_identifiers()
                .contains_kind(&ENTRY_ID)
            && !input
                .operation_event()
                .get_identifiers()
                .contains_kind(&ENTRY_REF_ID)
            && sides.occupied(&input)
        {
            return Err(invalid(
                "$.operation.book.position",
                "a new anonymous entry cannot occupy an existing position; state MDEntryID or send a snapshot",
            ));
        }
        let identity = sides.resolve(&input)?;
        let previous = sides.get(&identity);
        let continued = previous.is_some();
        let applied = continue_entry(input, previous, action)?;
        if previous.is_some_and(|held| restates(&applied, held)) {
            return Ok(false);
        }
        if !continued
            && let Some(held) = cleared.get(&identity)
            && restates(&applied, held)
        {
            sides.place(&identity, held, journal.as_deref_mut())?;
            return Ok(false);
        }
        if !continued
            && applied.operation_event().get_state().is_live()
            && !rests_on(&applied, Side::Buy)
            && !rests_on(&applied, Side::Sell)
        {
            left_out(&applied);
        }
        let entry = Arc::new(applied);
        sides.place(&identity, &entry, journal)?;
        fold_bounds(event, EventBounds::of_data(&entry));
        deltas.push(entry);
        Ok(true)
    }

    /// Settles the top-of-book facts on the sides of a complete book; a
    /// book stating its deltas alone keeps the ones it states.
    fn settle(&mut self) {
        if let Some(sides) = &self.sides {
            sides.settle(&mut self.event);
        }
    }

    /// The event facts the book settles on: a complete book's on its sides,
    /// checked first - a level whose aggregate quantity passes decimal is
    /// refused - and a book stating its deltas alone its own, priced at its
    /// best bid and ask ([`book_price`], [`median_quantity`]); finalized.
    pub(super) fn canonical_event(&self) -> Result<MarketEventFacts> {
        let mut event = self.event.clone();
        match &self.sides {
            Some(sides) => {
                sides.check_levels()?;
                sides.settle(&mut event);
            }
            None => {
                event.set_price(book_price(event.get_bidpx(), event.get_askpx()), true);
                event.set_quantity(
                    median_quantity(event.get_bidqty(), event.get_askqty()),
                    true,
                );
            }
        }
        finalize_book_event(&mut event, self.sides.as_ref(), &self.deltas);
        Ok(event)
    }

    /// Moves the book to `unix` where it stands at another instant: it
    /// follows the book it was ([`follow_book`]), its place and deltas
    /// start again.
    fn advance(&mut self, unix: i64) {
        if self.event.get_currunix() == unix {
            return;
        }
        let head = self.event.clone();
        follow_book(&mut self.event, &head);
        self.deltas.clear();
        self.event.set_seqnum(0);
        self.event.set_currunix(unix);
    }

    /// The book a walk emits at its instant - whole at a snapshot `tick`,
    /// which states its instant as its `snapunix`, else its deltas alone -
    /// finalized, the deltas handed over and the walk's copy keeping the
    /// identity it emitted: the sides shared, never copied.
    fn emit(&mut self, tick: bool) -> Self {
        let unix = self.event.get_currunix();
        self.event.set_snapunix(tick.then_some(unix));
        self.finalize();
        Self {
            event: self.event.clone(),
            sides: if tick { self.sides.clone() } else { None },
            deltas: std::mem::take(&mut self.deltas),
        }
    }

    /// Refuses `previous` as the book this one follows: another book code,
    /// a later instant, this book itself, or another book than the one this
    /// one names as its `prevuuid`.
    fn check_follows(&self, previous: &Self) -> Result<()> {
        if self.get_crosscode() != previous.get_crosscode() {
            return Err(invalid(
                "$.crosscode",
                format_smolstr!(
                    "expected the book {:?} follows, got {:?}",
                    self.get_crosscode(),
                    previous.get_crosscode()
                ),
            ));
        }
        if self.get_currunix() < previous.get_currunix() {
            return Err(invalid(
                "$.prevunix",
                format_smolstr!(
                    "expected a book at or before {}, got {}",
                    self.get_currunix(),
                    previous.get_currunix()
                ),
            ));
        }
        if self.get_curruuid() == previous.get_curruuid() {
            return Err(invalid("$.prevuuid", "a book does not follow itself"));
        }
        match self.event.get_prevuuid() {
            Some(named) if named != previous.get_curruuid() => Err(invalid(
                "$.prevuuid",
                format_smolstr!(
                    "expected the book {named} this one follows, got {}",
                    previous.get_curruuid()
                ),
            )),
            _ => Ok(()),
        }
    }

    /// This book over `previous`, which it follows, moved rather than
    /// borrowed: a book stating its deltas alone takes `previous`' sides -
    /// shared with nothing else where `previous` was the last holder, so
    /// nothing is copied - and replays its deltas over them; a complete book
    /// follows it alone. A book stating its deltas alone and no `prevuuid`
    /// follows no book - it is the first of its code a walk emitted, or the
    /// first after it started again - so it rebuilds over the empty book
    /// every walk starts from, whatever `previous` holds
    /// ([`Self::rebuilt_from_empty`]).
    ///
    /// # Errors
    ///
    /// Refuses what [`Element::with_previous`] answers `None` for, naming
    /// why: another book, a `previous` holding no sides, a delta that does
    /// not replay, or a rebuild whose top of book is not the one this book
    /// states.
    pub(crate) fn rebuilt(mut self, previous: Self) -> Result<Self> {
        self.check_follows(&previous)?;
        if self.sides.is_some() {
            follow_book(&mut self.event, &previous.event);
            self.finalize();
            return Ok(self);
        }
        if self.event.get_prevuuid().is_none() {
            return self.rebuilt_from_empty();
        }
        let Self { event, sides, .. } = previous;
        let Some(sides) = sides else {
            return Err(invalid(
                format_smolstr!("$.{ALIVE}"),
                "expected a complete book to rebuild over, got one holding only its deltas",
            ));
        };
        self.rebuild(sides, Some(&event))
    }

    /// This book stating its deltas alone and no `prevuuid`, rebuilt over
    /// the empty book it follows - [`Self::keyed`] under its own key, the
    /// one every walk starts a code's books from - whole and under its own
    /// identity: the first book a walk emits for its code, unless a
    /// snapshot tick made it whole.
    ///
    /// # Errors
    ///
    /// Refuses, at `$.prevuuid`, a book following another, and what a
    /// rebuild refuses: a delta that does not replay, or a rebuild whose top
    /// of book is not the one this book states.
    pub(crate) fn rebuilt_from_empty(self) -> Result<Self> {
        if let Some(previous) = self.event.get_prevuuid() {
            return Err(invalid(
                "$.prevuuid",
                format_smolstr!("expected a book following no book, got one following {previous}"),
            ));
        }
        if self.sides.is_some() {
            return Ok(self);
        }
        let empty = Self::keyed(
            self.event.get_currunix(),
            super::market::base_crosscode(self.event.get_crosscode()),
        );
        let sides = empty.sides.expect("an empty book is complete");
        self.rebuild(sides, None)
    }

    /// This book stating its deltas alone rebuilt over `sides`, those of the
    /// book `previous` it follows - the empty book where it follows none:
    /// each delta replayed in the order applied, the link to `previous`
    /// taken, and the top of book the rebuild settles on checked against
    /// the one this book states; under its own identity.
    fn rebuild(mut self, mut sides: Sides, previous: Option<&MarketEventFacts>) -> Result<Self> {
        for (index, delta) in self.deltas.iter().enumerate() {
            sides.replay(delta).map_err(|error| match error {
                Error::InvalidRecord { reason, .. } => {
                    invalid(format_smolstr!("$.{DELTAS}[{index}]"), reason)
                }
                other => other,
            })?;
        }
        if let Some(previous) = previous {
            follow_book(&mut self.event, previous);
        }
        let mut settled = self.event.clone();
        sides.settle(&mut settled);
        if let Some(name) = unsettled(&self.event, &settled) {
            return Err(invalid(
                format_smolstr!("$.{name}"),
                "expected the book its deltas rebuild over the book before it, got another top of book",
            ));
        }
        self.sides = Some(sides);
        self.finalize();
        Ok(self)
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

/// `entry`, alive on a book, as the delta that takes it off the book at
/// `unix` in `state` - an expiration at its deadline, a removal where its
/// chain moved to another book: a delete of the current generation, the
/// reference a prior rename used to reach its predecessor dropped,
/// reporting no fill ([`super::iterator::clear_fill`]), no execution, no
/// recording and no snapshot instant. Finalized by its caller, once it
/// has placed it among its instant's inputs.
fn withdrawn(entry: &MarketData, unix: i64, state: State) -> MarketData {
    let mut operation = entry.clone();
    let event = operation.operation_event_mut();
    event.set_currunix(unix);
    event.set_state(state);
    let mut book = event.control().cloned().unwrap_or_default();
    book.action = Some(MdUpdateAction::Delete);
    event.set_control(Some(book));
    let mut identifiers = event.get_identifiers().clone();
    if identifiers
        .remove(&crate::IdKey::base(ENTRY_REF_ID))
        .is_some()
    {
        let _ = event.set_identifiers(identifiers, true);
    }
    super::iterator::clear_fill(event);
    event.set_execunix(None, true);
    event.set_recdunix(None);
    event.set_snapunix(None);
    operation
}

/// The price a book states over its best bid and ask: their midpoint, the
/// one stated where the other is not, and none where they cross - the one
/// rule a book's price is settled and checked by.
fn book_price(bid: Option<Decimal>, ask: Option<Decimal>) -> Option<Decimal> {
    match (bid, ask) {
        (Some(bid), Some(ask)) if bid > ask => None,
        (Some(bid), Some(ask)) => decimal_mean(bid, ask).or(Some(bid)),
        (Some(price), None) | (None, Some(price)) => Some(price),
        (None, None) => None,
    }
}

/// The quantity a book states over its best bid and ask quantities: their
/// two-value median, the one stated where the other is not - the one rule
/// a book's quantity is settled and checked by.
fn median_quantity(bid: Option<Decimal>, ask: Option<Decimal>) -> Option<Decimal> {
    match (bid, ask) {
        (Some(bid), Some(ask)) => decimal_mean(bid, ask),
        (Some(quantity), None) | (None, Some(quantity)) => Some(quantity),
        (None, None) => None,
    }
}

/// Digests a book in chain form: its market event - its state, the book it
/// follows and every market fact, its top of book included - its instant,
/// the digest of each side's live entries only where it states a snapshot
/// instant, then its deltas - their count and each one's operation word and
/// `curruuid` - in the order applied. A book between snapshots is pinned by
/// the book it follows, its deltas and the facts they settled on, so a book
/// stating its deltas alone and the book rebuilt from it share one
/// identity, and a book walks its sides only at a snapshot.
fn finalize_book_event(
    event: &mut MarketEventFacts,
    sides: Option<&Sides>,
    deltas: &[Arc<MarketData>],
) {
    event.sync_cross();
    let mut digest = event.digest_market_event();
    digest.write(&event.get_currunix().to_be_bytes());
    if event.get_snapunix().is_some()
        && let Some(sides) = sides
    {
        for side in [&sides.bid, &sides.ask] {
            digest.write(&side.digest().to_be_bytes());
        }
    }
    digest.write(&(deltas.len() as u64).to_be_bytes());
    for delta in deltas {
        digest.write(delta.operation_event().operation_word().as_bytes());
        digest.write(&delta.get_curruuid().get().to_be_bytes());
    }
    event.finalized(digest.finish());
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
        finalize_book_event(&mut self.event, self.sides.as_ref(), &self.deltas);
    }

    /// This book as the one after `previous`: a book stating its deltas
    /// alone rebuilt over `previous`' sides, its deltas replayed in the
    /// order applied, whole and under its own identity - over the empty
    /// book where it names no `prevuuid`, being the first of its code a walk
    /// emitted - and a complete book - authoritative - linked to `previous`
    /// alone. `None` where it cannot follow it - another book code, an
    /// earlier instant than `previous`', another book than the one it names
    /// as its `prevuuid`, a book holding only its deltas over one holding no
    /// sides either, a delta that does not replay, or a rebuild settling on
    /// another top of book than the one it states - and, for a complete
    /// book, where nothing moves.
    fn with_previous(mut self, previous: &Self) -> Option<Self> {
        if self.sides.is_none() {
            if self.event.get_prevuuid().is_none() {
                self.check_follows(previous).ok()?;
                return self.rebuilt_from_empty().ok();
            }
            // The sides `previous` shares are copied once, where the first
            // delta changes them.
            return self.rebuilt(previous.clone()).ok();
        }
        self.check_follows(previous).ok()?;
        if !follow_book(&mut self.event, &previous.event) {
            return None;
        }
        self.finalize();
        Some(self)
    }

    /// This book merged with another statement of it at its instant, the
    /// reference chosen by its recording clock: two complete books' sides
    /// joined - the reference's entries, then each of the supplement's it
    /// does not hold, unless the reference is a snapshot, which is
    /// authoritative - and their deltas; a complete book over one stating
    /// its deltas alone, which it is authoritative over; two books stating
    /// their deltas alone, the reference's facts over the deltas of both,
    /// the reference's first. `None` where they are not one book at one
    /// instant or nothing moves.
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
        let (mut merged, other) = match (&reference.sides, &supplement.sides) {
            (Some(held), Some(supplied)) => {
                let authoritative = reference.get_snapunix().is_some();
                let sides = merge_book_sides(held, supplied, authoritative)?;
                let deltas = if authoritative {
                    reference.deltas.clone()
                } else {
                    union_deltas(&reference.deltas, &supplement.deltas)
                };
                let merged = Self {
                    event: reference.event.clone(),
                    sides: Some(sides),
                    deltas,
                };
                (merged, supplement)
            }
            // A complete book is authoritative over one stating its deltas
            // alone, whichever records later.
            (Some(_), None) => (reference.clone(), supplement),
            (None, Some(_)) => (supplement.clone(), reference),
            (None, None) => {
                let merged = Self {
                    event: reference.event.clone(),
                    sides: None,
                    deltas: union_deltas(&reference.deltas, &supplement.deltas),
                };
                (merged, supplement)
            }
        };
        merge_market_event_into_reference(&mut merged.event, &other.event);
        merged.validate_parts().ok()?;
        merged.event = merged.canonical_event().ok()?;
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
/// instrument's ISIN where it holds one, else its ticker, else
/// [`Isin::NONE`] - one book per instrument wherever an ISIN is known, and
/// a ticker-only input joining it once the lifecycle's registry learned the
/// pair. A book opens keyed ([`BookEvent::keyed`]) and takes its ticker and
/// its ISIN from the first input stating each; no input moves them after.
/// An identity restated under another key - a chain stated by its ticker
/// alone and then, its instrument learned, under its ISIN - leaves the book
/// it stood in by a delta there - a snapshot's member as any other
/// statement - removing it in state `REMOVED` and reporting no fill, and
/// opens in its own at the same instant, so an entry rests in one book at a
/// time.
/// Each timestamp and book is committed atomically across ordinary deltas,
/// expirations, and explicit snapshot membership.
///
/// A book is emitted whole - [`BookEvent::is_complete`], its alive entries,
/// its limits and its `snapunix` the instant - only at a snapshot tick:
/// every grid tick `snapshot_millis` crosses for every code - the ticks the
/// walk catches up on and the quiet buckets included - and a group that
/// replaced membership: a full refresh, an empty snapshot control, or
/// inputs stating a `snapunix`. Every other book states the deltas its
/// instant applied alone, beside the top of book they settled on, and names
/// the book it follows as its `prevuuid` - none for the first book of a
/// book code, which follows the empty book a walk starts from:
/// [`Element::with_previous`] over the complete book before it rebuilds it
/// whole. With no grid, a book is whole only at a full refresh, so
/// rebuilding a book reads back to its first appearance or its last full
/// refresh.
///
/// A book is emitted where it holds a delta - every order and quote folded
/// into it but a repeat of the live entry it continues, resting anywhere
/// or not - or at a snapshot tick where it holds an entry; a snapshot
/// emptying a book is emitted too, empty and whole, for the books after it
/// to rebuild over. An instant whose inputs only repeat what the book
/// holds emits no book for it, and neither does a grid tick finding a book
/// empty that nothing changed there: the next book follows the last one
/// emitted.
///
/// The walk folds what
/// [`MarketDataKind::is_booked`](crate::MarketDataKind::is_booked) admits -
/// orders, quotes and snapshot controls - records an execution among the
/// deltas of its instant, moving no side, and prunes every other input
/// [`MarketDataKind::is_recorded`](crate::MarketDataKind::is_recorded)
/// does not admit where it is pulled, a FIX message's leaves once it is
/// split: a pruned input touches no book, no instant and no grid, so an
/// instant only a trade reached emits no book. [`Self::with_filter`]
/// narrows the walk further.
pub struct BookIterator<I>
where
    I: Iterator,
    I::Item: Into<Result<MarketData>>,
{
    source: I,
    /// The booked leaves pulled from the source and not yet taken, in
    /// order: a FIX message's leaves once it is split - a book folds what
    /// a message reports, one side and entry each - or, under a filter, the
    /// ones a batch of pulled leaves kept.
    split: VecDeque<MarketData>,
    /// What [`Self::with_filter`] installed; none keeps every booked leaf.
    filter: Option<RowFilter>,
    /// The source's failure, yielded once the leaves pulled before it are
    /// taken.
    failed: Option<Error>,
    /// Whether the source answered its end.
    drained: bool,
    source_head: Option<Result<MarketData>>,
    source_exhausted: bool,
    books: BTreeMap<String, BookEvent>,
    pending: VecDeque<Result<BookEvent>>,
    snapshot_ns: Option<i64>,
    next_snapshot: Option<i64>,
    /// Every deadline scheduled, earliest first.
    expirations: BTreeSet<BookExpiration>,
    /// Per book, the one deadline each identity is scheduled for: a later
    /// statement withdraws its predecessor's, so the schedule holds at most
    /// one deadline per identity a delta stated - never one per amendment -
    /// and a full refresh drops its book's alone.
    scheduled: HashMap<String, HashMap<LiveKey, BookExpiration>>,
    /// Per identity the walk folded alive, the key of the book it stands
    /// in: an input of the identity keyed elsewhere - a chain stated by
    /// its ticker alone and then under its instrument's ISIN - withdraws
    /// it from that book before it opens it in its own. One slot per order
    /// or quote identity whose last statement stood alive; one its
    /// statement ended, its deadline expired or a snapshot replacing its
    /// book's membership dropped leaves it.
    booked: HashMap<LiveKey, String>,
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
    /// `snapshot_millis == 0` disables grid snapshots, so a book is emitted
    /// whole only at a full refresh.
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
            filter: None,
            failed: None,
            drained: false,
            source_head: None,
            source_exhausted: false,
            books: BTreeMap::new(),
            pending: VecDeque::new(),
            snapshot_ns: (snapshot_ns > 0).then_some(snapshot_ns),
            next_snapshot: None,
            expirations: BTreeSet::new(),
            scheduled: HashMap::new(),
            booked: HashMap::new(),
            last_unix: None,
            done: false,
        })
    }

    /// This walk folding only the inputs `filter` keeps: an expression over
    /// the [`MarketData::field`] row, bound once here and answered by the
    /// expression engine over one batch per 1,024 booked inputs the walk
    /// pulls ahead. The kind rule prunes first, so a filter narrows what a
    /// book folds and records and never admits what
    /// [`MarketDataKind::is_recorded`](crate::MarketDataKind::is_recorded)
    /// does not. A filter that keeps every row installs nothing, and the walk
    /// pulls one input at a time again.
    ///
    /// ```
    /// use yggdryl::graph::{BookIterator, Element, Event, Market, MarketData, OrderEvent};
    /// use yggdryl::{Decimal, Side, State};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = |code: &str, side: Side, unix: i64| {
    ///     let mut order = OrderEvent::at(unix);
    ///     order.set_crosscode(code.to_owned());
    ///     order.set_ticker(Some("ACME".into()), true);
    ///     order.set_side(side, true);
    ///     order.set_price(Some(Decimal::from_int(100)), true);
    ///     order.set_quantity(Some(Decimal::ONE), true);
    ///     order.set_state(State::New);
    ///     order.finalize();
    ///     MarketData::from(order)
    /// };
    /// let inputs = vec![order("B-1", Side::Buy, 1), order("A-1", Side::Sell, 2)];
    /// let books = BookIterator::new(inputs.into_iter(), 0)?
    ///     .with_filter("side = 'BUYS'")?
    ///     .collect::<yggdryl::Result<Vec<_>>>()?;
    /// // The ask never reached a book, so its instant emitted none.
    /// assert_eq!(books.len(), 1);
    /// assert_eq!(books[0].deltas().len(), 1);
    /// // A column the row does not carry is refused where the filter is bound.
    /// let walk = BookIterator::new(std::iter::empty::<MarketData>(), 0)?;
    /// assert!(walk.with_filter("nope = 1").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the filter's own parse error, and an error when it names a
    /// column the row does not carry or answers anything but a boolean.
    pub fn with_filter(mut self, filter: impl IntoFilter) -> Result<Self> {
        self.filter = RowFilter::new(&filter.into_filter()?)?;
        Ok(self)
    }

    fn fill_source_head(&mut self) {
        if self.source_head.is_some() || self.source_exhausted {
            return;
        }
        // A filter may keep nothing of one pull: the next is pulled.
        while self.split.is_empty() && self.failed.is_none() && !self.drained {
            self.pull();
        }
        self.source_head = match self.split.pop_front() {
            Some(operation) => Some(
                foldable(&operation, || SmolStr::new_static("$.operation.kind"))
                    .map(|()| operation),
            ),
            None => self.failed.take().map(Err),
        };
        self.source_exhausted = self.source_head.is_none();
    }

    /// Pulls the source into [`Self::split`] until it holds a recorded leaf -
    /// or, under a filter, [`FILTERED_ROWS`] of them, which the filter then
    /// narrows as one batch - stopping at the source's end or its failure,
    /// kept for after the leaves before it. Every input
    /// [`MarketDataKind::is_recorded`](crate::MarketDataKind::is_recorded)
    /// does not admit is dropped here, a FIX message's leaves once it is
    /// split.
    fn pull(&mut self) {
        let wanted = if self.filter.is_some() {
            FILTERED_ROWS
        } else {
            1
        };
        while self.split.len() < wanted {
            let Some(item) = self.source.next() else {
                self.drained = true;
                break;
            };
            match item.into() {
                Ok(MarketData::Fix(message)) => match message.into_market_data() {
                    Ok(leaves) => self.split.extend(leaves.into_iter().filter(recorded)),
                    Err(error) => {
                        self.failed = Some(error);
                        break;
                    }
                },
                Ok(leaf) => {
                    if recorded(&leaf) {
                        self.split.push_back(leaf);
                    }
                }
                Err(error) => {
                    self.failed = Some(error);
                    break;
                }
            }
        }
        let Some(filter) = &self.filter else {
            return;
        };
        let mut pulled = Vec::from(std::mem::take(&mut self.split));
        // A filter that cannot answer ends the walk as the source's
        // failure does, the leaves it could not judge left out.
        match filter.retain(&mut pulled) {
            Ok(()) => self.split = VecDeque::from(pulled),
            Err(error) => self.failed = Some(error),
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

    /// Emits every book whole at the grid tick `snapshot`, each a book the
    /// walk's last book of its code follows - but those whose group at that
    /// tick `failed`, and an empty book no group changed at the tick, which
    /// holds neither an entry nor a delta: skipped before it advances, so
    /// the next book follows the last one emitted.
    fn emit_snapshot(&mut self, snapshot: i64, failed: &HashSet<String>) {
        for (symbol, book) in &mut self.books {
            if failed.contains(symbol)
                || (book.is_empty() && book.deltas.is_empty() && book.get_currunix() != snapshot)
            {
                continue;
            }
            book.advance(snapshot);
            self.pending.push_back(Ok(book.emit(true)));
        }
        self.next_snapshot = self.snapshot_ns.and_then(|step| snapshot.checked_add(step));
    }

    /// Schedules the deadline of each entry `symbol`'s last group applied
    /// alive, withdrawing the one its identity was scheduled for before -
    /// a delta ending the entry or stating no deadline withdraws it alone -
    /// or, where the group replaced its membership (`resync`), of every
    /// entry alive, the book's earlier schedules withdrawn: each entry once,
    /// whichever sides it rests on. What a delta reaches only through
    /// another identity - a range, a rename - stays scheduled until it falls
    /// due, where [`Self::due`] passes it over.
    fn schedule_expirations(&mut self, symbol: &str, resync: bool) {
        let Self {
            books,
            expirations,
            scheduled,
            ..
        } = self;
        let Some(book) = books.get(symbol) else {
            return;
        };
        let schedules = match scheduled.get_mut(symbol) {
            Some(schedules) => schedules,
            None => scheduled.entry(symbol.to_owned()).or_default(),
        };
        if resync {
            for (_, held) in schedules.drain() {
                expirations.remove(&held);
            }
        }
        let unix = book.get_currunix();
        let stated = if resync {
            Box::new(book.alive()) as Box<dyn Iterator<Item = &MarketData>>
        } else {
            Box::new(book.deltas())
        };
        for operation in stated {
            let identity = LiveKey::of(operation);
            if let Some(held) = schedules.remove(&identity) {
                expirations.remove(&held);
            }
            let event = operation.operation_event();
            let Some(expiration) = event.get_exprunix() else {
                continue;
            };
            if expiration <= unix || !event.get_state().is_live() {
                continue;
            }
            let schedule = BookExpiration {
                unix: expiration,
                book: symbol.to_owned(),
                identity: identity.clone(),
                generation: operation.get_curruuid(),
            };
            expirations.insert(schedule.clone());
            schedules.insert(identity, schedule);
        }
    }

    /// Forgets `expiration`, taken off the schedule's front, as the deadline
    /// its identity is scheduled for, where it still is.
    fn unschedule(&mut self, expiration: &BookExpiration) {
        let Some(schedules) = self.scheduled.get_mut(&expiration.book) else {
            return;
        };
        if schedules.get(&expiration.identity) == Some(expiration) {
            schedules.remove(&expiration.identity);
        }
    }

    /// The live entry `expiration` is the deadline of, where it still is:
    /// the generation scheduled, expiring then, and live; none where a later
    /// statement of the entry, or its end, outdated the schedule.
    fn due(&self, expiration: &BookExpiration) -> Option<&MarketData> {
        let operation = self
            .books
            .get(&expiration.book)?
            .live_entry(&expiration.identity)?;
        let event = operation.operation_event();
        (operation.get_curruuid() == expiration.generation
            && event.get_exprunix() == Some(expiration.unix)
            && event.get_state().is_live())
        .then_some(operation)
    }

    /// The instant of the next deadline still due, every schedule its
    /// entry no longer answers - one a range or a rename reached - dropped
    /// on the way, so an outdated one never moves the walk to its instant,
    /// nor a grid tick past the last input.
    fn next_expiration(&mut self) -> Option<i64> {
        loop {
            let first = self.expirations.first()?;
            if self.due(first).is_some() {
                return Some(first.unix);
            }
            let outdated = self.expirations.pop_first().expect("the first was present");
            self.unschedule(&outdated);
        }
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
            self.unschedule(&expiration);
            let Some(operation) = self.due(&expiration) else {
                continue;
            };
            let mut operation = withdrawn(operation, unix, State::Expired);
            // An event of its own deadline, placed first there, so a step of
            // its chain at that instant folds after it.
            operation.operation_event_mut().set_seqnum(0);
            operation.finalize();
            self.booked.remove(&expiration.identity);
            expired.entry(expiration.book).or_default().push(operation);
        }
        expired
    }

    /// The delta that withdraws `input`'s identity from the book it stands
    /// in where that is not `key`, `input`'s own, with that book's key:
    /// the entry as it stands there - alive, or pending in `raw` at this
    /// instant - taken off it ([`withdrawn`]) at `unix`, removed; `None`
    /// where the identity stands in no other book. Records `key` as where
    /// a live `input` stands next, and forgets an identity `input` ends.
    fn withdrawal(
        &mut self,
        input: &MarketData,
        key: &str,
        unix: i64,
        raw: &BTreeMap<String, Vec<MarketData>>,
        members: &mut BTreeMap<String, Vec<MarketData>>,
    ) -> Option<(String, MarketData)> {
        if !matches!(input, MarketData::OrderEvent(_) | MarketData::QuoteEvent(_)) {
            return None;
        }
        let identity = LiveKey::of(input);
        // A chain restated under the key it stands in moves no slot, so the
        // map allocates once per chain and once per move.
        let held = if input.operation_event().get_state().is_live() {
            match self.booked.get_mut(&identity) {
                Some(held) if held.as_str() == key => return None,
                Some(held) => Some(std::mem::replace(held, key.to_owned())),
                None => {
                    self.booked.insert(identity.clone(), key.to_owned());
                    return None;
                }
            }
        } else {
            self.booked.remove(&identity)
        }
        .filter(|held| held != key)?;
        // A member of the held book's snapshot pending at this instant
        // leaves that snapshot, so the membership it replaces no longer
        // states the entry the restatement moves.
        if let Some(group) = members.get_mut(&held) {
            group.retain(|member| !identity.matches(member));
        }
        let standing = self
            .books
            .get(&held)
            .and_then(|book| book.live_entry(&identity))
            .or_else(|| {
                raw.get(&held)
                    .and_then(|group| group.iter().rev().find(|held| identity.matches(held)))
            })?;
        let mut operation = withdrawn(standing, unix, State::Removed);
        operation.finalize();
        Some((held, operation))
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
            let expiration_unix = self.next_expiration();
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
            let mut snapshot_members: BTreeMap<String, Vec<MarketData>> = BTreeMap::new();
            let mut snapshot_controls: BTreeMap<String, Vec<SnapshotEvent>> = BTreeMap::new();
            let mut snapshot_partitions: BTreeMap<String, BTreeSet<SnapshotPartition>> =
                BTreeMap::new();

            if source_unix == Some(unix) {
                self.last_unix = Some(unix);
                while self
                    .peek_source()
                    .is_some_and(|operation| effective_unix(operation) == unix)
                {
                    let input = self.take_source();
                    let symbol = input.book_crosscode().to_owned();
                    touched.insert(symbol.clone());
                    // An identity restated under another key leaves the book
                    // it stood in, a snapshot's member as any other
                    // statement.
                    if let Some((held, withdrawn)) =
                        self.withdrawal(&input, &symbol, unix, &raw, &mut snapshot_members)
                    {
                        touched.insert(held.clone());
                        raw.entry(held).or_default().push(withdrawn);
                    }
                    // An execution is recorded and never a snapshot's member,
                    // whatever snapshot instant it states: it joins the
                    // group of its instant as any delta does.
                    if matches!(input, MarketData::ExecutionEvent(_))
                        || input
                            .as_event()
                            .is_none_or(|event| event.get_snapunix().is_none())
                    {
                        raw.entry(symbol).or_default().push(input);
                        continue;
                    }
                    // A snapshotted input is a control or, as `foldable`
                    // admitted, an order or a quote.
                    match input {
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
                        input => {
                            snapshot_partitions
                                .entry(symbol.clone())
                                .or_default()
                                .insert(SnapshotPartition::of(&input));
                            snapshot_members.entry(symbol).or_default().push(input);
                        }
                    }
                }
            }

            for operations in raw.values_mut() {
                order_chains(operations);
            }
            let mut failed = HashSet::new();
            // The books a group applied nothing to: left as they were, and
            // emitting nothing.
            let mut unchanged = HashSet::new();
            // The books emitted whole at this instant beside every one at a
            // grid tick: one a group replaced the membership of.
            let mut keyframes = HashSet::new();
            for symbol in &touched {
                // fold_group is already atomic. Only a group that also
                // replaces supplied membership needs an outer transaction.
                if !snapshot_partitions.contains_key(symbol)
                    && let Some(book) = self.books.get_mut(symbol)
                {
                    match raw
                        .remove(symbol)
                        .map_or(Ok(Applied::default()), |operations| {
                            book.fold_group(operations)
                        }) {
                        Ok(applied) if applied.any() => {
                            if applied.replaced {
                                keyframes.insert(symbol.clone());
                            }
                        }
                        Ok(_) => {
                            unchanged.insert(symbol.clone());
                        }
                        Err(error) => {
                            excluded(&error, symbol);
                            failed.insert(symbol.clone());
                        }
                    }
                    continue;
                }
                let mut next = self
                    .books
                    .get(symbol)
                    .cloned()
                    .unwrap_or_else(|| BookEvent::keyed(unix, symbol.clone()));
                let mut result = raw
                    .remove(symbol)
                    .map_or(Ok(Applied::default()), |operations| {
                        next.fold_group(operations)
                    });
                if let Ok(applied) = &mut result
                    && let Some(partitions) = snapshot_partitions.get(symbol)
                {
                    let members = snapshot_members.remove(symbol).unwrap_or_default();
                    let controls = snapshot_controls.remove(symbol).unwrap_or_default();
                    let held = !next.is_empty();
                    match next.replace_snapshot_membership(members, &controls, partitions, unix) {
                        Ok(()) => {
                            // Emptied, it removed what it held: replacing
                            // leaves every other partition as it stood.
                            applied.replaced = true;
                            applied.holds = !next.is_empty();
                            applied.removed |= held && !applied.holds;
                        }
                        Err(error) => result = Err(error),
                    }
                }
                match result {
                    Ok(applied) if applied.any() => {
                        if applied.replaced {
                            keyframes.insert(symbol.clone());
                        }
                        self.books.insert(symbol.clone(), next);
                    }
                    Ok(_) => {
                        unchanged.insert(symbol.clone());
                    }
                    Err(error) => {
                        excluded(&error, symbol);
                        failed.insert(symbol.clone());
                    }
                }
            }
            for symbol in &touched {
                if !failed.contains(symbol) && !unchanged.contains(symbol) {
                    self.schedule_expirations(symbol, keyframes.contains(symbol));
                }
            }
            // A book whose membership a snapshot replaced holds only what
            // the snapshot stated: the slots of the identities it dropped
            // go with them.
            for symbol in &keyframes {
                if let Some(book) = self.books.get(symbol) {
                    self.booked.retain(|identity, held| {
                        held != symbol || book.live_entry(identity).is_some()
                    });
                }
            }

            if at_grid {
                self.emit_snapshot(unix, &failed);
            } else {
                // A book emits only what a group changed: its deltas, or
                // itself whole at a keyframe.
                for symbol in &touched {
                    if failed.contains(symbol) || unchanged.contains(symbol) {
                        continue;
                    }
                    let Some(book) = self.books.get_mut(symbol) else {
                        continue;
                    };
                    self.pending
                        .push_back(Ok(book.emit(keyframes.contains(symbol))));
                }
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

/// The instant an input takes effect at: the snapshot it belongs to, else
/// its own.
fn effective_unix(input: &MarketData) -> i64 {
    input.as_event().map_or(0, |event| {
        event.get_snapunix().unwrap_or_else(|| event.get_currunix())
    })
}

/// How many booked inputs a filtered walk pulls ahead and lays out as one
/// batch for its filter.
const FILTERED_ROWS: usize = 1024;

/// Whether a book states an input of `input`'s kind - folded into a side,
/// or recorded among its deltas:
/// [`MarketDataKind::is_recorded`](crate::MarketDataKind::is_recorded), the
/// one rule every pruning site reads.
fn recorded(input: &MarketData) -> bool {
    input.marketdatakind().is_recorded()
}

/// Refuses, at `path`, every variant a book does not state: a book takes an
/// order or a quote, an execution it records, or a snapshot control, each
/// dated.
fn foldable(input: &MarketData, path: impl FnOnce() -> SmolStr) -> Result<()> {
    if matches!(
        input,
        MarketData::OrderEvent(_)
            | MarketData::QuoteEvent(_)
            | MarketData::ExecutionEvent(_)
            | MarketData::SnapshotEvent(_)
    ) {
        return Ok(());
    }
    Err(invalid(
        path(),
        format_smolstr!(
            "expected order_event, quote_event, execution_event or snapshot_event, got {}",
            input.kind().as_str()
        ),
    ))
}

/// Refuses, at `path`, anything but an order or a quote: what a book side
/// holds.
fn validate_kind(operation: &MarketData, path: &str) -> Result<()> {
    if matches!(
        operation,
        MarketData::OrderEvent(_) | MarketData::QuoteEvent(_)
    ) {
        return Ok(());
    }
    Err(invalid(path, "expected an order or quote on a book side"))
}

/// Refuses, at `path`, anything but an order, a quote or an execution: what
/// a book's delta is - an entry it placed, or an execution it recorded.
fn validate_delta_kind(operation: &MarketData, path: &str) -> Result<()> {
    if matches!(
        operation,
        MarketData::OrderEvent(_) | MarketData::QuoteEvent(_) | MarketData::ExecutionEvent(_)
    ) {
        return Ok(());
    }
    Err(invalid(
        path,
        "expected an order, a quote or an execution among a book's deltas",
    ))
}

/// Says that `operation` rests on no side of its book: live, it takes
/// neither the bid nor the ask - a sided entry taking neither, a quote
/// stating no leg, a leg sized zero, a range stating neither side - and
/// continues no live entry, so it places nothing.
fn left_out(operation: &MarketData) {
    warned!(
        "book entry placed nowhere: it rests on neither the bid nor the ask and continues no live entry",
        operation.kind().as_str(),
        "{} states side {}",
        operation.get_crosscode(),
        operation.get_side().as_str()
    );
}

/// `operation` as it continues `previous`, the live entry it resolved to,
/// under `action`: a change or an overlay inherits the price and the size
/// it leaves unstated, and so does a delete - it names the entry it
/// removes, and a feed routinely states no price on it, so the terminal
/// delta carries the level it takes out - then it follows the entry,
/// carrying what its chain carries (a quote's legs it states nothing of),
/// typed as an order where it continues one or learns its `OrderID`.
///
/// # Errors
///
/// Refuses a partial update continuing nothing that states no price or no
/// size, an `OrderID` other than the live entry's, and a quote continuing
/// an order, or an order a quote, that no rule promotes.
fn continue_entry(
    mut operation: MarketData,
    previous: Option<&MarketData>,
    action: Option<MdUpdateAction>,
) -> Result<MarketData> {
    let partial = action.is_some_and(MdUpdateAction::is_partial);
    let Some(previous) = previous else {
        if partial && entry_px_of(&operation).is_none() {
            return Err(invalid(
                "$.operation.book.entry_px",
                "a partial update without a live predecessor must state its price",
            ));
        }
        if partial && entry_size_of(&operation).is_none() {
            return Err(invalid(
                "$.operation.book.entry_size",
                "a partial update without a live predecessor must state its size",
            ));
        }
        return Ok(operation);
    };
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
                Some(MdUpdateAction::Change | MdUpdateAction::Delete | MdUpdateAction::Overlay)
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
    let mut operation = match promote_order {
        Some(true) => promote_to_order(operation, data),
        Some(false) => promote_to_quote(operation, data),
        None if is_order(&operation) => promote_to_order(operation, data),
        None => promote_to_quote(operation, data),
    };
    operation.finalize();
    Ok(operation)
}

fn is_full_snapshot(input: &MarketData) -> bool {
    // An execution is recorded and replaces no membership, whatever control
    // the parse left on it - a snapshot's trade entry states one.
    !matches!(input, MarketData::ExecutionEvent(_))
        && input.book().and_then(|book| book.action) == Some(MdUpdateAction::Snapshot)
}

/// The two sides of a merged book, each the reference's live entries, then
/// each of the supplement's whose identity the reference holds on neither
/// side - an entry stands on the sides one statement of it rests on -
/// unless the reference is an authoritative snapshot.
fn merge_book_sides(reference: &Sides, supplement: &Sides, authoritative: bool) -> Option<Sides> {
    let held: HashSet<LiveKey> = if authoritative {
        HashSet::new()
    } else {
        reference.alive().map(LiveKey::of).collect()
    };
    let side = |reference: &Ladder, supplement: &Ladder| {
        let mut live = reference.store.entries.to_vec();
        if !authoritative {
            live.extend(
                supplement
                    .store
                    .entries
                    .iter()
                    .filter(|operation| !held.contains(&LiveKey::of(operation)))
                    .cloned(),
            );
        }
        Ladder::from_live(reference.side, live.into_iter().enumerate().collect(), None).ok()
    };
    Some(Sides {
        bid: side(&reference.bid, &supplement.bid)?,
        ask: side(&reference.ask, &supplement.ask)?,
    })
}

/// The deltas of two statements of one book: the reference's in their
/// order, then each of the supplement's it lacks, by kind and `curruuid`.
fn union_deltas(
    reference: &[Arc<MarketData>],
    supplement: &[Arc<MarketData>],
) -> Vec<Arc<MarketData>> {
    let mut keys = reference
        .iter()
        .map(|operation| (operation.kind(), operation.get_curruuid()))
        .collect::<HashSet<_>>();
    let mut deltas = reference.to_vec();
    deltas.extend(
        supplement
            .iter()
            .filter(|operation| keys.insert((operation.kind(), operation.get_curruuid())))
            .cloned(),
    );
    deltas
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

fn validate_symbols<'a, E, I>(key: &str, name: &str, operations: I) -> Result<()>
where
    E: Market + ?Sized + 'a,
    I: IntoIterator<Item = &'a E>,
{
    for (index, operation) in operations.into_iter().enumerate() {
        if let Some(reason) = book_mismatch(key, operation) {
            return Err(invalid(format_smolstr!("$.{name}[{index}]"), reason));
        }
    }
    Ok(())
}

/// Why `operation` cannot stand in the book keyed `key`; `None` where it
/// can. One rule: the input's own key ([`Market::book_crosscode`]) - its
/// ISIN, else its ticker - spells the book's, or the input states neither
/// an ISIN nor a ticker, which stands in any book - a hand-built book is
/// fed ticker-less orders. An ISIN-keyed book therefore takes two
/// listings' tickers, which stay apart inside it by their own partition.
fn book_mismatch<E: Market + ?Sized>(key: &str, operation: &E) -> Option<SmolStr> {
    let stated = operation.book_crosscode();
    (stated != key && stated != Isin::NONE)
        .then(|| format_smolstr!("expected book crosscode {key:?}, got {stated:?}"))
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

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/graph/book.rs` pins and a caller cannot reach.
    //!
    //! A book a walk emits shares each side's store with the walk rather
    //! than copying it, and the walk changes a store it alone holds in
    //! place: nothing a book answers shows either, only where a side's
    //! store lives and how many hold it.
    use std::sync::Arc;

    use super::{BookEvent, BookIterator, MarketData};
    use crate::{Result, Side};

    /// Where the store of `book`'s `side` lives, and how many books and
    /// walks hold it; `None` for a side that is neither a bid nor an ask.
    pub fn side_store(book: &BookEvent, side: Side) -> Option<(usize, usize)> {
        let store = &book.ladder(side)?.store;
        Some((Arc::as_ptr(store).addr(), Arc::strong_count(store)))
    }

    /// How many deadlines `walk` holds scheduled: at most one per identity
    /// a delta stated, however often it was amended.
    pub fn scheduled_expirations<I>(walk: &BookIterator<I>) -> usize
    where
        I: Iterator,
        I::Item: Into<Result<MarketData>>,
    {
        walk.expirations.len()
    }
}
