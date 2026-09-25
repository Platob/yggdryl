//! The operation leaves: an order, a quote or an execution, undated or
//! dated, with the book-control facts a market-data entry carries.
//!
//! [`OperationElement<K>`] is one operation with no instant - an order, a
//! quote or an execution entry - and [`OperationEvent<K>`] the same dated,
//! which is what every market message expands to and what a book holds: `K`
//! says which kind, a crate-private holder keeps every fact, and a boxed
//! [`BookRef`] - absent on every operation
//! that is not a market-data entry, so one pointer - holds the typed facts a
//! book reads to place it: the update action, the scope, the position, and
//! the price and size the entry stated for itself. The aliases
//! [`Order`], [`Quote`], [`Execution`], [`OrderEvent`], [`QuoteEvent`] and
//! [`ExecutionEvent`] are the six leaves this crate ships; `K` is sealed, so
//! no other kind can be named. A composite trade is
//! [`TradeEvent`](super::TradeEvent), whose executions are
//! [`ExecutionEvent`].

use std::marker::PhantomData;

use smol_str::SmolStr;

use super::element::Staged;
use super::facts::{OperationEventFacts, OperationFacts};
use super::kind::MarketKind;
use super::{Element, Event, Market, Operation};
use crate::{Decimal18, Uuid};

mod sealed {
    pub trait Sealed {}
}

/// Which operation leaf a generic [`OperationElement`]/[`OperationEvent`] is:
/// sealed, so [`OrderKind`], [`QuoteKind`] and [`ExecutionKind`] are the only
/// three kinds that can ever instantiate them.
pub trait OperationKind:
    sealed::Sealed + Copy + Clone + std::fmt::Debug + Eq + PartialEq + Send + Sync + 'static
{
    /// The word this kind digests and stores under: `order`, `quote` or
    /// `execution`, whichever leaf's own row this is - dated or not.
    const KIND: MarketKind;
}

/// The [`Order`]/[`OrderEvent`] marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrderKind;
impl sealed::Sealed for OrderKind {}
impl OperationKind for OrderKind {
    const KIND: MarketKind = MarketKind::Order;
}

/// The [`Quote`]/[`QuoteEvent`] marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuoteKind;
impl sealed::Sealed for QuoteKind {}
impl OperationKind for QuoteKind {
    const KIND: MarketKind = MarketKind::Quote;
}

/// The [`Execution`]/[`ExecutionEvent`] marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionKind;
impl sealed::Sealed for ExecutionKind {}
impl OperationKind for ExecutionKind {
    const KIND: MarketKind = MarketKind::Execution;
}

/// FIX's `MDUpdateAction(279)` over a market-data entry, plus the full
/// snapshot a `W` message replaces a scope with.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum MdUpdateAction {
    /// `0`: a new entry.
    New = 0,
    /// `1`: a change to a live entry.
    Change = 1,
    /// `2`: a live entry removed.
    Delete = 2,
    /// `3`: every entry from the best through the stated position removed.
    DeleteThru = 3,
    /// `4`: every entry from the stated position onward removed.
    DeleteFrom = 4,
    /// `5`: an overlay of a live entry.
    Overlay = 5,
    /// A full snapshot: the scope is replaced by the entries beside it.
    Snapshot = 6,
}

impl MdUpdateAction {
    /// Every action, in declaration order.
    pub const ALL: [Self; 7] = [
        Self::New,
        Self::Change,
        Self::Delete,
        Self::DeleteThru,
        Self::DeleteFrom,
        Self::Overlay,
        Self::Snapshot,
    ];

    /// The stored spelling: the FIX code, or `snapshot`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::New => "0",
            Self::Change => "1",
            Self::Delete => "2",
            Self::DeleteThru => "3",
            Self::DeleteFrom => "4",
            Self::Overlay => "5",
            Self::Snapshot => "snapshot",
        }
    }

    /// The code set's name, folded: what a named body spells.
    const fn name(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Change => "change",
            Self::Delete => "delete",
            Self::DeleteThru => "deletethru",
            Self::DeleteFrom => "deletefrom",
            Self::Overlay => "overlay",
            Self::Snapshot => "snapshot",
        }
    }

    /// The action a spelling names: the code, the name folded, or the
    /// legacy `SNAPSHOT`; `None` for any other text.
    #[must_use]
    pub fn read(text: &str) -> Option<Self> {
        let text = text.trim();
        Self::ALL
            .into_iter()
            .find(|action| action.as_str() == text || action.name().eq_ignore_ascii_case(text))
    }

    /// Whether the action removes a range of positions rather than one
    /// entry: delete-through or delete-from.
    #[must_use]
    pub const fn is_range_delete(self) -> bool {
        matches!(self, Self::DeleteThru | Self::DeleteFrom)
    }

    /// Whether the action is a partial update of a live entry: a change or
    /// an overlay, which inherits what it leaves unstated.
    #[must_use]
    pub const fn is_partial(self) -> bool {
        matches!(self, Self::Change | Self::Overlay)
    }
}

/// The typed book-control facts a market-data entry carries: what a book
/// reads to place the operation, never an identifier - the entry's own and
/// referenced identifiers are the operation's `MDENTRYID` and
/// `MDENTRYREFID` alternate identifiers.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BookRef {
    /// The update action, where the entry states one.
    pub action: Option<MdUpdateAction>,
    /// The book scope the entry belongs to, where stated.
    pub scope: Option<SmolStr>,
    /// `MDEntryPositionNo(290)`: the entry's position in its level.
    pub position: Option<u32>,
    /// `MDEntryPx(270)` as the entry stated it; a partial update stating
    /// none inherits the live entry's price.
    pub entry_px: Option<Decimal18>,
    /// `MDEntrySize(271)` as the entry stated it; a partial update stating
    /// none inherits the live entry's size.
    pub entry_size: Option<Decimal18>,
}

impl BookRef {
    /// Whether any control fact is stated.
    #[must_use]
    pub fn is_stated(&self) -> bool {
        self.action.is_some()
            || self.scope.is_some()
            || self.position.is_some()
            || self.entry_px.is_some()
            || self.entry_size.is_some()
    }

    fn feed(&self, staged: &mut Staged<'_>) {
        if let Some(action) = self.action {
            staged.feed("mdupdateaction", action.as_str().as_bytes());
        }
        if let Some(scope) = &self.scope {
            staged.feed("bookscope", scope.as_bytes());
        }
        if let Some(position) = self.position {
            staged.feed("mdentrypositionno", &position.to_le_bytes());
        }
        if let Some(px) = self.entry_px {
            staged.feed("mdentrypx", &px.units().to_le_bytes());
        }
        if let Some(size) = self.entry_size {
            staged.feed("mdentrysize", &size.units().to_le_bytes());
        }
    }
}

/// One operation with no instant: an order, a quote or an execution entry,
/// as a book level holds or a walk states at an instant. `K` is
/// [`OrderKind`], [`QuoteKind`] or [`ExecutionKind`]; the aliases
/// [`Order`], [`Quote`] and [`Execution`] name the three.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationElement<K: OperationKind> {
    data: OperationFacts,
    _kind: PhantomData<K>,
}

impl<K: OperationKind> Default for OperationElement<K> {
    fn default() -> Self {
        Self {
            data: OperationFacts::default(),
            _kind: PhantomData,
        }
    }
}

impl<K: OperationKind> OperationElement<K> {
    /// An entry stating nothing, not yet finalized.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Which operation this is.
    #[must_use]
    pub const fn kind(&self) -> MarketKind {
        K::KIND
    }

    /// This entry dated at `unix`, nanoseconds since the Unix epoch, and
    /// finalized: a move.
    #[must_use]
    pub fn at(self, unix: i64) -> OperationEvent<K> {
        let mut operation = OperationEvent {
            data: self.data.at(unix),
            book: None,
            _kind: PhantomData,
        };
        operation.finalize();
        operation
    }
}

impl<K: OperationKind> Element for OperationElement<K> {
    fn get_curruuid(&self) -> Uuid {
        self.data.get_curruuid()
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.data.set_curruuid(curruuid);
    }

    fn get_crossuuid(&self) -> Uuid {
        self.data.get_crossuuid()
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.data.set_crossuuid(crossuuid);
    }

    fn get_crosscode(&self) -> &str {
        self.data.get_crosscode()
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.data.set_crosscode(crosscode);
    }

    fn get_currhashcode(&self) -> u64 {
        self.data.get_currhashcode()
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.data.set_currhashcode(hashcode);
    }

    fn get_crosshashcode(&self) -> u64 {
        self.data.get_crosshashcode()
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.data.set_crosshashcode(crosshashcode);
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        self.data.get_srcuuids()
    }

    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
        self.data.set_srcuuids(sources);
    }

    fn is_after(&self, other: &Self) -> bool {
        self.data.is_after(&other.data)
    }

    /// The entry's facts and its kind digest to the code: the same facts as
    /// an order and as a quote are two entries.
    fn finalize(&mut self) {
        self.data.fill_market();
        self.data.fill_operation();
        self.data.sync_cross();
        let mut digest = self.data.digest_operation();
        {
            let mut staged = Staged::new(&mut digest);
            staged.feed("operationkind", K::KIND.as_str().as_bytes());
        }
        let hashcode = digest.as_u64();
        self.data.set_currhashcode(hashcode);
        self.data.set_curruuid(Uuid::from_v8(u128::from(hashcode)));
        let crossuuid = self.data.cross_uuid();
        self.data.set_crossuuid(crossuuid);
    }

    fn with_previous(mut self, previous: &Self) -> Option<Self> {
        let data = std::mem::take(&mut self.data).with_previous(&previous.data)?;
        self.data = data;
        self.finalize();
        Some(self)
    }

    fn merge_with(mut self, other: &Self) -> Option<Self> {
        let data = std::mem::take(&mut self.data).merge_with(&other.data)?;
        self.data = data;
        self.finalize();
        Some(self)
    }
}

impl<K: OperationKind> Market for OperationElement<K> {
    fn get_price(&self) -> Option<Decimal18> {
        self.data.get_price()
    }
    fn set_price(&mut self, price: Option<Decimal18>) {
        self.data.set_price(price);
    }
    fn get_currency(&self) -> &crate::Ccy {
        self.data.get_currency()
    }
    fn set_currency(&mut self, currency: crate::Ccy) {
        self.data.set_currency(currency);
    }
    fn get_quantity(&self) -> Option<Decimal18> {
        self.data.get_quantity()
    }
    fn set_quantity(&mut self, quantity: Option<Decimal18>) {
        self.data.set_quantity(quantity);
    }
    fn get_unit(&self) -> &crate::Unit {
        self.data.get_unit()
    }
    fn set_unit(&mut self, unit: crate::Unit) {
        self.data.set_unit(unit);
    }
    fn get_side(&self) -> crate::Side {
        self.data.get_side()
    }
    fn set_side(&mut self, side: crate::Side) {
        self.data.set_side(side);
    }
    fn get_securityids(&self) -> &crate::securityid::SecurityIds {
        self.data.get_securityids()
    }
    fn set_securityids(&mut self, ids: crate::securityid::SecurityIds) -> crate::Result<()> {
        self.data.set_securityids(ids)
    }
    fn insert_securityid(&mut self, id: crate::securityid::SecurityId) -> crate::Result<bool> {
        self.data.insert_securityid(id)
    }
    fn remove_securityid(&mut self, key: &crate::securityid::SecType) -> crate::Result<bool> {
        self.data.remove_securityid(key)
    }
    fn derive_securityid(&mut self, id: crate::securityid::SecurityId) -> bool {
        self.data.derive_securityid(id)
    }
    fn get_cficode(&self) -> Option<&crate::CfiCode> {
        self.data.get_cficode()
    }
    fn set_cficode(&mut self, code: Option<crate::CfiCode>) {
        self.data.set_cficode(code);
    }
    fn get_miccode(&self) -> Option<&crate::MicCode> {
        self.data.get_miccode()
    }
    fn set_miccode(&mut self, code: Option<crate::MicCode>) {
        self.data.set_miccode(code);
    }
    fn get_lastpx(&self) -> Option<Decimal18> {
        self.data.get_lastpx()
    }
    fn set_lastpx(&mut self, px: Option<Decimal18>) {
        self.data.set_lastpx(px);
    }
    fn get_lastqty(&self) -> Option<Decimal18> {
        self.data.get_lastqty()
    }
    fn set_lastqty(&mut self, qty: Option<Decimal18>) {
        self.data.set_lastqty(qty);
    }
    fn get_avgpx(&self) -> Option<Decimal18> {
        self.data.get_avgpx()
    }
    fn set_avgpx(&mut self, px: Option<Decimal18>) {
        self.data.set_avgpx(px);
    }
    fn get_cumqty(&self) -> Option<Decimal18> {
        self.data.get_cumqty()
    }
    fn set_cumqty(&mut self, qty: Option<Decimal18>) {
        self.data.set_cumqty(qty);
    }
    fn get_leavesqty(&self) -> Option<Decimal18> {
        self.data.get_leavesqty()
    }
    fn set_leavesqty(&mut self, qty: Option<Decimal18>) {
        self.data.set_leavesqty(qty);
    }
    fn get_prevpx(&self) -> Option<Decimal18> {
        self.data.get_prevpx()
    }
    fn set_prevpx(&mut self, px: Option<Decimal18>) {
        self.data.set_prevpx(px);
    }
    fn get_prevqty(&self) -> Option<Decimal18> {
        self.data.get_prevqty()
    }
    fn set_prevqty(&mut self, qty: Option<Decimal18>) {
        self.data.set_prevqty(qty);
    }
    fn get_spotrate(&self) -> Option<Decimal18> {
        self.data.get_spotrate()
    }
    fn set_spotrate(&mut self, rate: Option<Decimal18>) {
        self.data.set_spotrate(rate);
    }
    fn get_forwardpoints(&self) -> Option<Decimal18> {
        self.data.get_forwardpoints()
    }
    fn set_forwardpoints(&mut self, points: Option<Decimal18>) {
        self.data.set_forwardpoints(points);
    }
    fn get_ticker(&self) -> Option<&str> {
        self.data.get_ticker()
    }
    fn set_ticker(&mut self, ticker: Option<smol_str::SmolStr>) {
        self.data.set_ticker(ticker);
    }
    fn get_metadata(&self) -> &super::market::Metadata {
        self.data.get_metadata()
    }
    fn set_metadata(&mut self, metadata: Option<super::market::Metadata>) {
        self.data.set_metadata(metadata);
    }
}

impl<K: OperationKind> Operation for OperationElement<K> {
    fn get_marketoperationid(&self) -> Option<i32> {
        self.data.get_marketoperationid()
    }
    fn set_marketoperationid(&mut self, marketoperationid: Option<i32>) {
        self.data.set_marketoperationid(marketoperationid);
    }
    fn get_tif(&self) -> Option<&crate::TimeInForce> {
        self.data.get_tif()
    }
    fn set_tif(&mut self, tif: Option<crate::TimeInForce>) {
        self.data.set_tif(tif);
    }
    fn get_tradable(&self) -> Option<bool> {
        self.data.get_tradable()
    }
    fn set_tradable(&mut self, tradable: Option<bool>) {
        self.data.set_tradable(tradable);
    }
    fn get_accountids(&self) -> &crate::idmap::IdMap {
        self.data.get_accountids()
    }
    fn set_accountids(&mut self, ids: crate::idmap::IdMap) -> crate::Result<()> {
        self.data.set_accountids(ids)
    }
    fn insert_accountid(&mut self, key: &str, value: &str) -> crate::Result<bool> {
        self.data.insert_accountid(key, value)
    }
    fn remove_accountid(&mut self, key: &str) -> crate::Result<bool> {
        self.data.remove_accountid(key)
    }
    fn get_userids(&self) -> &crate::idmap::IdMap {
        self.data.get_userids()
    }
    fn set_userids(&mut self, ids: crate::idmap::IdMap) -> crate::Result<()> {
        self.data.set_userids(ids)
    }
    fn insert_userid(&mut self, key: &str, value: &str) -> crate::Result<bool> {
        self.data.insert_userid(key, value)
    }
    fn remove_userid(&mut self, key: &str) -> crate::Result<bool> {
        self.data.remove_userid(key)
    }
    fn get_altids(&self) -> &crate::idmap::IdMap {
        self.data.get_altids()
    }
    fn set_altids(&mut self, ids: crate::idmap::IdMap) -> crate::Result<()> {
        self.data.set_altids(ids)
    }
    fn insert_altid(&mut self, key: &str, value: &str) -> crate::Result<bool> {
        self.data.insert_altid(key, value)
    }
    fn remove_altid(&mut self, key: &str) -> crate::Result<bool> {
        self.data.remove_altid(key)
    }
    fn get_bid(&self) -> Option<&super::market::Lane> {
        self.data.get_bid()
    }
    fn set_bid(&mut self, lane: Option<super::market::Lane>) {
        self.data.set_bid(lane);
    }
    fn get_ask(&self) -> Option<&super::market::Lane> {
        self.data.get_ask()
    }
    fn set_ask(&mut self, lane: Option<super::market::Lane>) {
        self.data.set_ask(lane);
    }
}

/// One dated operation: an order, a quote or an execution. `K` is
/// [`OrderKind`], [`QuoteKind`] or [`ExecutionKind`]; the aliases
/// [`OrderEvent`], [`QuoteEvent`] and [`ExecutionEvent`] name the three.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationEvent<K: OperationKind> {
    data: OperationEventFacts,
    book: Option<Box<BookRef>>,
    _kind: PhantomData<K>,
}

impl<K: OperationKind> Default for OperationEvent<K> {
    fn default() -> Self {
        Self {
            data: OperationEventFacts::default(),
            book: None,
            _kind: PhantomData,
        }
    }
}

impl<K: OperationKind> OperationEvent<K> {
    /// An operation that happened at `unix`, nanoseconds since the Unix
    /// epoch, stating nothing else yet.
    #[must_use]
    pub fn at(unix: i64) -> Self {
        Self {
            data: OperationEventFacts::at(unix),
            book: None,
            _kind: PhantomData,
        }
    }

    /// An operation over facts already held, not yet finalized: the move a
    /// FIX message and a decoded row make into their leaf.
    pub(crate) fn from_facts(data: OperationEventFacts) -> Self {
        Self {
            data,
            book: None,
            _kind: PhantomData,
        }
    }

    /// The facts this operation holds.
    pub(crate) fn facts(&self) -> &OperationEventFacts {
        &self.data
    }

    /// This event as the one after an operation event of any kind, through
    /// the facts both hold: an execution follows the order it fills, keeping
    /// its own kind and control. `None` where it cannot follow it.
    pub(crate) fn following_facts(mut self, previous: &OperationEventFacts) -> Option<Self> {
        let data = std::mem::take(&mut self.data).following_operation(previous)?;
        self.data = data;
        self.finalize();
        Some(self)
    }

    /// This event as another statement of an operation event of any kind,
    /// through the facts both hold; its own kind and control stay.
    pub(crate) fn restating_facts(mut self, live: &OperationEventFacts) -> Self {
        let data = std::mem::take(&mut self.data).restating(live);
        self.data = data;
        self.finalize();
        self
    }

    /// Which operation this is.
    #[must_use]
    pub const fn kind(&self) -> MarketKind {
        K::KIND
    }

    /// This operation without its clocks: a move.
    #[must_use]
    pub fn into_element(self) -> OperationElement<K> {
        OperationElement {
            data: self.data.into_entry(),
            _kind: PhantomData,
        }
    }

    /// The book-control facts, where the operation is a market-data entry.
    #[must_use]
    pub fn book(&self) -> Option<&BookRef> {
        self.book.as_deref()
    }

    /// Sets [`Self::book`]; a control stating nothing is `None`. The
    /// operation is not refinalized: the caller finalizes once its facts
    /// are in.
    pub fn set_book(&mut self, book: Option<BookRef>) {
        self.book = book.filter(BookRef::is_stated).map(Box::new);
    }

    /// This operation with the given book-control facts.
    #[must_use]
    pub fn with_book(mut self, book: BookRef) -> Self {
        self.set_book(Some(book));
        self
    }

    /// The update action the entry states, where it states one.
    #[must_use]
    pub fn action(&self) -> Option<MdUpdateAction> {
        self.book.as_ref().and_then(|book| book.action)
    }

    /// Whether this operation is part of a FIX full-snapshot replacement.
    #[must_use]
    pub fn is_full_snapshot(&self) -> bool {
        self.action() == Some(MdUpdateAction::Snapshot)
    }

    /// The book scope the entry states, empty where it states none.
    #[must_use]
    pub fn scope(&self) -> &str {
        self.book
            .as_ref()
            .and_then(|book| book.scope.as_deref())
            .unwrap_or("")
    }

    /// This operation's facts and book control, reinterpreted as a
    /// different kind - the book's own quote-to-order continuation, which
    /// takes fresh facts for the promoted operation and keeps this
    /// operation's book control unchanged.
    pub(crate) fn with_kind<K2: OperationKind>(
        self,
        data: OperationEventFacts,
    ) -> OperationEvent<K2> {
        OperationEvent {
            data,
            book: self.book,
            _kind: PhantomData,
        }
    }
}

impl<K: OperationKind> Element for OperationEvent<K> {
    fn get_curruuid(&self) -> Uuid {
        self.data.get_curruuid()
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.data.set_curruuid(curruuid);
    }

    fn get_crossuuid(&self) -> Uuid {
        self.data.get_crossuuid()
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.data.set_crossuuid(crossuuid);
    }

    fn get_crosscode(&self) -> &str {
        self.data.get_crosscode()
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.data.set_crosscode(crosscode);
    }

    fn get_currhashcode(&self) -> u64 {
        self.data.get_currhashcode()
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.data.set_currhashcode(hashcode);
    }

    fn get_crosshashcode(&self) -> u64 {
        self.data.get_crosshashcode()
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.data.set_crosshashcode(crosshashcode);
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        self.data.get_srcuuids()
    }

    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
        self.data.set_srcuuids(sources);
    }

    fn is_after(&self, other: &Self) -> bool {
        self.data.is_after(&other.data)
    }

    /// The operation's facts, its kind and its book control digest to the
    /// code: the same entry as an order and as a quote are two operations.
    fn finalize(&mut self) {
        self.data.fill_market();
        self.data.fill_operation();
        self.data.sync_cross();
        let mut digest = self.data.digest_operation_event();
        {
            let mut staged = Staged::new(&mut digest);
            staged.feed("operationkind", K::KIND.as_str().as_bytes());
            if let Some(book) = &self.book {
                book.feed(&mut staged);
            }
        }
        let hashcode = digest.as_u64();
        self.data.finalized(hashcode);
    }

    fn with_previous(mut self, previous: &Self) -> Option<Self> {
        let data = std::mem::take(&mut self.data).following_operation(&previous.data)?;
        self.data = data;
        self.finalize();
        Some(self)
    }

    fn merge_with(mut self, other: &Self) -> Option<Self> {
        let data = std::mem::take(&mut self.data).merging_operation_event(&other.data)?;
        self.data = data;
        if self.book.is_none() && other.book.is_some() {
            self.book.clone_from(&other.book);
        }
        self.finalize();
        Some(self)
    }
}

impl<K: OperationKind> Event for OperationEvent<K> {
    fn get_currunix(&self) -> i64 {
        self.data.get_currunix()
    }
    fn set_currunix(&mut self, unix: i64) {
        self.data.set_currunix(unix);
    }
    fn get_state(&self) -> &crate::State {
        self.data.get_state()
    }
    fn set_state(&mut self, state: crate::State) {
        self.data.set_state(state);
    }
    fn is_execution(&self) -> bool {
        matches!(K::KIND, MarketKind::Execution)
    }
    fn get_seqnum(&self) -> u64 {
        self.data.get_seqnum()
    }
    fn set_seqnum(&mut self, seqnum: u64) {
        self.data.set_seqnum(seqnum);
    }
    fn get_creaunix(&self) -> Option<i64> {
        self.data.get_creaunix()
    }
    fn set_creaunix(&mut self, unix: Option<i64>) {
        self.data.set_creaunix(unix);
    }
    fn get_execunix(&self) -> Option<i64> {
        self.data.get_execunix()
    }
    fn set_execunix(&mut self, unix: Option<i64>) {
        self.data.set_execunix(unix);
    }
    fn get_recdunix(&self) -> Option<i64> {
        self.data.get_recdunix()
    }
    fn set_recdunix(&mut self, unix: Option<i64>) {
        self.data.set_recdunix(unix);
    }
    fn get_exprtime(&self) -> Option<i64> {
        self.data.get_exprtime()
    }
    fn set_exprtime(&mut self, unix: Option<i64>) {
        self.data.set_exprtime(unix);
    }
    fn get_prevunix(&self) -> Option<i64> {
        self.data.get_prevunix()
    }
    fn set_prevunix(&mut self, unix: Option<i64>) {
        self.data.set_prevunix(unix);
    }
    fn get_prevuuid(&self) -> Option<Uuid> {
        self.data.get_prevuuid()
    }
    fn set_prevuuid(&mut self, uuid: Option<Uuid>) {
        self.data.set_prevuuid(uuid);
    }
    fn get_snapunix(&self) -> Option<i64> {
        self.data.get_snapunix()
    }
    fn set_snapunix(&mut self, unix: Option<i64>) {
        self.data.set_snapunix(unix);
    }
    fn restating(mut self, live: &Self) -> Self {
        let data = std::mem::take(&mut self.data).restating(&live.data);
        self.data = data;
        self.finalize();
        self
    }
    fn finalized(&mut self, hashcode: u64) {
        self.data.finalized(hashcode);
    }
}

impl<K: OperationKind> Market for OperationEvent<K> {
    fn get_price(&self) -> Option<Decimal18> {
        self.data.get_price()
    }
    fn set_price(&mut self, price: Option<Decimal18>) {
        self.data.set_price(price);
    }
    fn get_currency(&self) -> &crate::Ccy {
        self.data.get_currency()
    }
    fn set_currency(&mut self, currency: crate::Ccy) {
        self.data.set_currency(currency);
    }
    fn get_quantity(&self) -> Option<Decimal18> {
        self.data.get_quantity()
    }
    fn set_quantity(&mut self, quantity: Option<Decimal18>) {
        self.data.set_quantity(quantity);
    }
    fn get_unit(&self) -> &crate::Unit {
        self.data.get_unit()
    }
    fn set_unit(&mut self, unit: crate::Unit) {
        self.data.set_unit(unit);
    }
    fn get_side(&self) -> crate::Side {
        self.data.get_side()
    }
    fn set_side(&mut self, side: crate::Side) {
        self.data.set_side(side);
    }
    fn get_securityids(&self) -> &crate::securityid::SecurityIds {
        self.data.get_securityids()
    }
    fn set_securityids(&mut self, ids: crate::securityid::SecurityIds) -> crate::Result<()> {
        self.data.set_securityids(ids)
    }
    fn insert_securityid(&mut self, id: crate::securityid::SecurityId) -> crate::Result<bool> {
        self.data.insert_securityid(id)
    }
    fn remove_securityid(&mut self, key: &crate::securityid::SecType) -> crate::Result<bool> {
        self.data.remove_securityid(key)
    }
    fn derive_securityid(&mut self, id: crate::securityid::SecurityId) -> bool {
        self.data.derive_securityid(id)
    }
    fn get_cficode(&self) -> Option<&crate::CfiCode> {
        self.data.get_cficode()
    }
    fn set_cficode(&mut self, code: Option<crate::CfiCode>) {
        self.data.set_cficode(code);
    }
    fn get_miccode(&self) -> Option<&crate::MicCode> {
        self.data.get_miccode()
    }
    fn set_miccode(&mut self, code: Option<crate::MicCode>) {
        self.data.set_miccode(code);
    }
    fn get_lastpx(&self) -> Option<Decimal18> {
        self.data.get_lastpx()
    }
    fn set_lastpx(&mut self, px: Option<Decimal18>) {
        self.data.set_lastpx(px);
    }
    fn get_lastqty(&self) -> Option<Decimal18> {
        self.data.get_lastqty()
    }
    fn set_lastqty(&mut self, qty: Option<Decimal18>) {
        self.data.set_lastqty(qty);
    }
    fn get_avgpx(&self) -> Option<Decimal18> {
        self.data.get_avgpx()
    }
    fn set_avgpx(&mut self, px: Option<Decimal18>) {
        self.data.set_avgpx(px);
    }
    fn get_cumqty(&self) -> Option<Decimal18> {
        self.data.get_cumqty()
    }
    fn set_cumqty(&mut self, qty: Option<Decimal18>) {
        self.data.set_cumqty(qty);
    }
    fn get_leavesqty(&self) -> Option<Decimal18> {
        self.data.get_leavesqty()
    }
    fn set_leavesqty(&mut self, qty: Option<Decimal18>) {
        self.data.set_leavesqty(qty);
    }
    fn get_prevpx(&self) -> Option<Decimal18> {
        self.data.get_prevpx()
    }
    fn set_prevpx(&mut self, px: Option<Decimal18>) {
        self.data.set_prevpx(px);
    }
    fn get_prevqty(&self) -> Option<Decimal18> {
        self.data.get_prevqty()
    }
    fn set_prevqty(&mut self, qty: Option<Decimal18>) {
        self.data.set_prevqty(qty);
    }
    fn get_spotrate(&self) -> Option<Decimal18> {
        self.data.get_spotrate()
    }
    fn set_spotrate(&mut self, rate: Option<Decimal18>) {
        self.data.set_spotrate(rate);
    }
    fn get_forwardpoints(&self) -> Option<Decimal18> {
        self.data.get_forwardpoints()
    }
    fn set_forwardpoints(&mut self, points: Option<Decimal18>) {
        self.data.set_forwardpoints(points);
    }
    fn get_ticker(&self) -> Option<&str> {
        self.data.get_ticker()
    }
    fn set_ticker(&mut self, ticker: Option<smol_str::SmolStr>) {
        self.data.set_ticker(ticker);
    }
    fn get_metadata(&self) -> &super::market::Metadata {
        self.data.get_metadata()
    }
    fn set_metadata(&mut self, metadata: Option<super::market::Metadata>) {
        self.data.set_metadata(metadata);
    }
}

impl<K: OperationKind> Operation for OperationEvent<K> {
    fn get_marketoperationid(&self) -> Option<i32> {
        self.data.get_marketoperationid()
    }
    fn set_marketoperationid(&mut self, marketoperationid: Option<i32>) {
        self.data.set_marketoperationid(marketoperationid);
    }
    fn get_tif(&self) -> Option<&crate::TimeInForce> {
        self.data.get_tif()
    }
    fn set_tif(&mut self, tif: Option<crate::TimeInForce>) {
        self.data.set_tif(tif);
    }
    fn get_tradable(&self) -> Option<bool> {
        self.data.get_tradable()
    }
    fn set_tradable(&mut self, tradable: Option<bool>) {
        self.data.set_tradable(tradable);
    }
    fn get_accountids(&self) -> &crate::idmap::IdMap {
        self.data.get_accountids()
    }
    fn set_accountids(&mut self, ids: crate::idmap::IdMap) -> crate::Result<()> {
        self.data.set_accountids(ids)
    }
    fn insert_accountid(&mut self, key: &str, value: &str) -> crate::Result<bool> {
        self.data.insert_accountid(key, value)
    }
    fn remove_accountid(&mut self, key: &str) -> crate::Result<bool> {
        self.data.remove_accountid(key)
    }
    fn get_userids(&self) -> &crate::idmap::IdMap {
        self.data.get_userids()
    }
    fn set_userids(&mut self, ids: crate::idmap::IdMap) -> crate::Result<()> {
        self.data.set_userids(ids)
    }
    fn insert_userid(&mut self, key: &str, value: &str) -> crate::Result<bool> {
        self.data.insert_userid(key, value)
    }
    fn remove_userid(&mut self, key: &str) -> crate::Result<bool> {
        self.data.remove_userid(key)
    }
    fn get_altids(&self) -> &crate::idmap::IdMap {
        self.data.get_altids()
    }
    fn set_altids(&mut self, ids: crate::idmap::IdMap) -> crate::Result<()> {
        self.data.set_altids(ids)
    }
    fn insert_altid(&mut self, key: &str, value: &str) -> crate::Result<bool> {
        self.data.insert_altid(key, value)
    }
    fn remove_altid(&mut self, key: &str) -> crate::Result<bool> {
        self.data.remove_altid(key)
    }
    fn get_bid(&self) -> Option<&super::market::Lane> {
        self.data.get_bid()
    }
    fn set_bid(&mut self, lane: Option<super::market::Lane>) {
        self.data.set_bid(lane);
    }
    fn get_ask(&self) -> Option<&super::market::Lane> {
        self.data.get_ask()
    }
    fn set_ask(&mut self, lane: Option<super::market::Lane>) {
        self.data.set_ask(lane);
    }
}

impl<K: OperationKind, E: Event + Operation + ?Sized> From<&E> for OperationEvent<K> {
    /// Copies every fact `event` states - the identities as stated - onto an
    /// operation of this kind with no book control, not refinalized.
    fn from(event: &E) -> Self {
        Self::from_facts(OperationEventFacts::from(event))
    }
}

/// What every dated order or quote answers about itself and its book
/// control, whichever kind it is: the seam
/// [`MarketData`](super::MarketData)'s book-side internals read through
/// instead of matching [`OrderEvent`]/[`QuoteEvent`] by hand. Never
/// implemented for anything a book side does not hold.
pub(crate) trait BookOperation: Event + Operation {
    /// [`OperationEvent::book`].
    fn control(&self) -> Option<&BookRef>;
    /// [`OperationEvent::set_book`].
    fn set_control(&mut self, book: Option<BookRef>);
    /// [`OperationEvent::action`].
    fn control_action(&self) -> Option<MdUpdateAction>;
    /// The word this operation's kind digests under: `order`, `quote` or
    /// `execution` - never the finer `MarketData` spelling.
    fn operation_word(&self) -> &'static str;
    /// The facts this operation holds, kind-agnostic: the book's own
    /// quote-to-order promotion reads and merges through this, since the
    /// facts type answers `Event`/`Operation` on its own regardless of the
    /// kind that will end up wrapping the merged result.
    fn facts(&self) -> &OperationEventFacts;
}

impl<K: OperationKind> BookOperation for OperationEvent<K> {
    fn control(&self) -> Option<&BookRef> {
        self.book()
    }

    fn set_control(&mut self, book: Option<BookRef>) {
        self.set_book(book);
    }

    fn control_action(&self) -> Option<MdUpdateAction> {
        self.action()
    }

    fn operation_word(&self) -> &'static str {
        K::KIND.as_str()
    }

    fn facts(&self) -> &OperationEventFacts {
        &self.data
    }
}

/// An undated order.
pub type Order = OperationElement<OrderKind>;
/// An undated quote.
pub type Quote = OperationElement<QuoteKind>;
/// An undated execution.
pub type Execution = OperationElement<ExecutionKind>;
/// A dated order.
pub type OrderEvent = OperationEvent<OrderKind>;
/// A dated quote.
pub type QuoteEvent = OperationEvent<QuoteKind>;
/// A dated execution.
pub type ExecutionEvent = OperationEvent<ExecutionKind>;
