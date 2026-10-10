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

use super::facts::{OperationEventFacts, OperationFacts};
use super::kind::MarketKind;
use super::{Market, Operation};
use yggdryl::graph::{Element, Event};
use yggdryl::implementer::Staged;
use yggdryl::{Decimal, Uuid};

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
///
/// Walk-time facts, not row facts: only the [`Self::scope`] is a column of
/// the `marketdata` row and feeds the operation's digest; the action, the
/// position and the price and size the entry stated for itself steer the
/// book walk and are gone once it has placed the entry.
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
    pub entry_px: Option<Decimal>,
    /// `MDEntrySize(271)` as the entry stated it; a partial update stating
    /// none inherits the live entry's size.
    pub entry_size: Option<Decimal>,
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

    /// Feeds the one control fact a row states, the scope.
    fn feed(&self, staged: &mut Staged<'_>) {
        if let Some(scope) = &self.scope {
            staged.feed("bookscope", scope.as_bytes());
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
        let mut data = OperationFacts::default();
        data.set_marketdatakind(K::KIND.marketdatakind());
        Self {
            data,
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
    fn get_uuid(&self) -> Uuid {
        self.data.get_uuid()
    }

    fn set_uuid(&mut self, uuid: Uuid) {
        self.data.set_uuid(uuid);
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

    fn get_hashcode(&self) -> u64 {
        self.data.get_hashcode()
    }

    fn set_hashcode(&mut self, hashcode: u64) {
        self.data.set_hashcode(hashcode);
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
        self.data.fill_parents();
        self.data.sync_cross();
        let mut digest = self.data.digest_operation();
        {
            let mut staged = Staged::new(&mut digest);
            staged.feed(
                "marketdatakind",
                &i32::from(K::KIND.marketdatakind().code()).to_le_bytes(),
            );
        }
        let hashcode = digest.as_u64();
        self.data.set_hashcode(hashcode);
        self.data.set_uuid(Uuid::from_v8(u128::from(hashcode)));
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
    fn get_price(&self) -> Option<Decimal> {
        self.data.get_price()
    }
    fn set_price(&mut self, price: Option<Decimal>, overwrite: bool) {
        self.data.set_price(price, overwrite);
    }

    fn get_stoppx(&self) -> Option<Decimal> {
        self.data.get_stoppx()
    }

    fn set_stoppx(&mut self, value: Option<Decimal>, overwrite: bool) {
        self.data.set_stoppx(value, overwrite);
    }

    fn get_strikepx(&self) -> Option<Decimal> {
        self.data.get_strikepx()
    }

    fn set_strikepx(&mut self, value: Option<Decimal>, overwrite: bool) {
        self.data.set_strikepx(value, overwrite);
    }
    fn get_currency(&self) -> &yggdryl::Ccy {
        self.data.get_currency()
    }
    fn set_currency(&mut self, currency: yggdryl::Ccy, overwrite: bool) {
        self.data.set_currency(currency, overwrite);
    }
    fn get_origccy(&self) -> &yggdryl::Ccy {
        self.data.get_origccy()
    }
    fn set_origccy(&mut self, ccy: yggdryl::Ccy, overwrite: bool) {
        self.data.set_origccy(ccy, overwrite);
    }
    fn get_quantity(&self) -> Option<Decimal> {
        self.data.get_quantity()
    }
    fn set_quantity(&mut self, quantity: Option<Decimal>, overwrite: bool) {
        self.data.set_quantity(quantity, overwrite);
    }

    fn get_displayqty(&self) -> Option<Decimal> {
        self.data.get_displayqty()
    }

    fn set_displayqty(&mut self, value: Option<Decimal>, overwrite: bool) {
        self.data.set_displayqty(value, overwrite);
    }

    fn get_hiddenqty(&self) -> Option<Decimal> {
        self.data.get_hiddenqty()
    }

    fn set_hiddenqty(&mut self, value: Option<Decimal>, overwrite: bool) {
        self.data.set_hiddenqty(value, overwrite);
    }
    fn get_unit(&self) -> &yggdryl::Unit {
        self.data.get_unit()
    }
    fn set_unit(&mut self, unit: yggdryl::Unit, overwrite: bool) {
        self.data.set_unit(unit, overwrite);
    }
    fn get_side(&self) -> crate::Side {
        self.data.get_side()
    }
    fn set_side(&mut self, side: crate::Side, overwrite: bool) {
        self.data.set_side(side, overwrite);
    }
    fn marketdatakind(&self) -> crate::MarketDataKind {
        self.data.marketdatakind()
    }
    fn get_marketdatatype(&self) -> crate::MarketDataType {
        self.data.get_marketdatatype()
    }
    fn set_marketdatatype(&mut self, mdtype: crate::MarketDataType, overwrite: bool) {
        self.data.set_marketdatatype(mdtype, overwrite);
    }
    fn get_securityids(&self) -> &crate::Identifiers {
        self.data.get_securityids()
    }
    fn set_securityids(&mut self, ids: crate::Identifiers, overwrite: bool) -> yggdryl::Result<()> {
        self.data.set_securityids(ids, overwrite)
    }
    fn insert_securityid(&mut self, id: crate::Identifier) -> yggdryl::Result<bool> {
        self.data.insert_securityid(id)
    }
    fn remove_securityid(&mut self, key: &crate::IdKey) -> yggdryl::Result<bool> {
        self.data.remove_securityid(key)
    }
    fn derive_securityid(&mut self, kind: &crate::IdType, code: &str) -> bool {
        self.data.derive_securityid(kind, code)
    }
    fn get_cficode(&self) -> Option<&yggdryl::Cfi> {
        self.data.get_cficode()
    }
    fn set_cficode(&mut self, code: Option<yggdryl::Cfi>, overwrite: bool) {
        self.data.set_cficode(code, overwrite);
    }
    fn get_miccode(&self) -> Option<&yggdryl::Mic> {
        self.data.get_miccode()
    }
    fn set_miccode(&mut self, code: Option<yggdryl::Mic>, overwrite: bool) {
        self.data.set_miccode(code, overwrite);
    }
    fn get_execunix(&self) -> Option<i64> {
        self.data.get_execunix()
    }
    fn set_execunix(&mut self, unix: Option<i64>, overwrite: bool) {
        self.data.set_execunix(unix, overwrite);
    }
    fn get_lastpx(&self) -> Option<Decimal> {
        self.data.get_lastpx()
    }
    fn set_lastpx(&mut self, px: Option<Decimal>, overwrite: bool) {
        self.data.set_lastpx(px, overwrite);
    }
    fn get_lastqty(&self) -> Option<Decimal> {
        self.data.get_lastqty()
    }
    fn set_lastqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.data.set_lastqty(qty, overwrite);
    }
    fn get_avgpx(&self) -> Option<Decimal> {
        self.data.get_avgpx()
    }
    fn set_avgpx(&mut self, px: Option<Decimal>, overwrite: bool) {
        self.data.set_avgpx(px, overwrite);
    }
    fn get_cumqty(&self) -> Option<Decimal> {
        self.data.get_cumqty()
    }
    fn set_cumqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.data.set_cumqty(qty, overwrite);
    }
    fn get_leavesqty(&self) -> Option<Decimal> {
        self.data.get_leavesqty()
    }
    fn set_leavesqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.data.set_leavesqty(qty, overwrite);
    }

    fn get_cxlqty(&self) -> Option<Decimal> {
        self.data.get_cxlqty()
    }

    fn set_cxlqty(&mut self, value: Option<Decimal>, overwrite: bool) {
        self.data.set_cxlqty(value, overwrite);
    }
    fn get_prevpx(&self) -> Option<Decimal> {
        self.data.get_prevpx()
    }
    fn set_prevpx(&mut self, px: Option<Decimal>, overwrite: bool) {
        self.data.set_prevpx(px, overwrite);
    }
    fn get_prevqty(&self) -> Option<Decimal> {
        self.data.get_prevqty()
    }
    fn set_prevqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.data.set_prevqty(qty, overwrite);
    }
    fn get_spotrate(&self) -> Option<Decimal> {
        self.data.get_spotrate()
    }
    fn set_spotrate(&mut self, rate: Option<Decimal>, overwrite: bool) {
        self.data.set_spotrate(rate, overwrite);
    }
    fn get_forwardpoints(&self) -> Option<Decimal> {
        self.data.get_forwardpoints()
    }
    fn set_forwardpoints(&mut self, points: Option<Decimal>, overwrite: bool) {
        self.data.set_forwardpoints(points, overwrite);
    }
    fn get_ticker(&self) -> Option<&str> {
        self.data.get_ticker()
    }
    fn set_ticker(&mut self, ticker: Option<smol_str::SmolStr>, overwrite: bool) {
        self.data.set_ticker(ticker, overwrite);
    }
    fn get_instcode(&self) -> Option<&str> {
        self.data.get_instcode()
    }
    fn set_instcode(&mut self, code: Option<yggdryl::Str>, overwrite: bool) {
        self.data.set_instcode(code, overwrite);
    }
    fn get_metadata(&self) -> &super::market::Metadata {
        self.data.get_metadata()
    }
    fn set_metadata(&mut self, metadata: Option<super::market::Metadata>, overwrite: bool) {
        self.data.set_metadata(metadata, overwrite);
    }
    fn get_fxrates(&self) -> &super::market::FxRates {
        self.data.get_fxrates()
    }
    fn set_fxrates(&mut self, rates: super::market::FxRates, overwrite: bool) {
        self.data.set_fxrates(rates, overwrite);
    }
    fn get_bidpx(&self) -> Option<yggdryl::Decimal> {
        self.data.get_bidpx()
    }
    fn set_bidpx(&mut self, px: Option<yggdryl::Decimal>, overwrite: bool) {
        self.data.set_bidpx(px, overwrite);
    }
    fn get_bidqty(&self) -> Option<yggdryl::Decimal> {
        self.data.get_bidqty()
    }
    fn set_bidqty(&mut self, qty: Option<yggdryl::Decimal>, overwrite: bool) {
        self.data.set_bidqty(qty, overwrite);
    }
    fn get_bidccy(&self) -> Option<&yggdryl::Ccy> {
        self.data.get_bidccy()
    }
    fn set_bidccy(&mut self, ccy: Option<yggdryl::Ccy>, overwrite: bool) {
        self.data.set_bidccy(ccy, overwrite);
    }
    fn get_askpx(&self) -> Option<yggdryl::Decimal> {
        self.data.get_askpx()
    }
    fn set_askpx(&mut self, px: Option<yggdryl::Decimal>, overwrite: bool) {
        self.data.set_askpx(px, overwrite);
    }
    fn get_askqty(&self) -> Option<yggdryl::Decimal> {
        self.data.get_askqty()
    }
    fn set_askqty(&mut self, qty: Option<yggdryl::Decimal>, overwrite: bool) {
        self.data.set_askqty(qty, overwrite);
    }
    fn get_askccy(&self) -> Option<&yggdryl::Ccy> {
        self.data.get_askccy()
    }
    fn set_askccy(&mut self, ccy: Option<yggdryl::Ccy>, overwrite: bool) {
        self.data.set_askccy(ccy, overwrite);
    }
}

impl<K: OperationKind> Operation for OperationElement<K> {
    fn get_ordqty(&self) -> Option<Decimal> {
        self.data.get_ordqty()
    }
    fn set_ordqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.data.set_ordqty(qty, overwrite);
    }
    fn get_timeinforce(&self) -> Option<&crate::TimeInForce> {
        self.data.get_timeinforce()
    }
    fn set_timeinforce(&mut self, tif: Option<crate::TimeInForce>, overwrite: bool) {
        self.data.set_timeinforce(tif, overwrite);
    }
    fn get_tradable(&self) -> Option<bool> {
        self.data.get_tradable()
    }
    fn set_tradable(&mut self, tradable: Option<bool>, overwrite: bool) {
        self.data.set_tradable(tradable, overwrite);
    }
    fn get_identifiers(&self) -> &crate::Identifiers {
        self.data.get_identifiers()
    }
    fn set_identifiers(&mut self, ids: crate::Identifiers, overwrite: bool) -> yggdryl::Result<()> {
        self.data.set_identifiers(ids, overwrite)
    }
    fn insert_identifier(&mut self, id: crate::Identifier) -> yggdryl::Result<bool> {
        self.data.insert_identifier(id)
    }
    fn remove_identifier(&mut self, key: &crate::IdKey) -> yggdryl::Result<bool> {
        self.data.remove_identifier(key)
    }
    fn get_partyids(&self) -> &crate::Identifiers {
        self.data.get_partyids()
    }
    fn set_partyids(
        &mut self,
        parties: crate::Identifiers,
        overwrite: bool,
    ) -> yggdryl::Result<()> {
        self.data.set_partyids(parties, overwrite)
    }
    fn insert_partyid(&mut self, party: crate::Identifier) -> yggdryl::Result<bool> {
        self.data.insert_partyid(party)
    }
    fn remove_partyid(&mut self, key: &crate::IdKey) -> yggdryl::Result<bool> {
        self.data.remove_partyid(key)
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
        Self::from_facts(OperationEventFacts::default())
    }
}

impl<K: OperationKind> OperationEvent<K> {
    /// An operation that happened at `unix`, nanoseconds since the Unix
    /// epoch, stating nothing else yet.
    #[must_use]
    pub fn at(unix: i64) -> Self {
        Self::from_facts(OperationEventFacts::at(unix))
    }

    /// An operation over facts already held, not yet finalized: the move a
    /// FIX message and a decoded row make into their leaf. The facts are
    /// stamped with this kind, so a cross code stated before the side is
    /// stored under it now.
    pub(crate) fn from_facts(mut data: OperationEventFacts) -> Self {
        data.set_marketdatakind(K::KIND.marketdatakind());
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

    /// Which operation this is.
    #[must_use]
    pub const fn kind(&self) -> MarketKind {
        K::KIND
    }

    /// This operation without its clocks: a move.
    #[must_use]
    pub fn into_element(self) -> OperationElement<K> {
        // The facts carry this kind's stamp across the move.
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
            book: self.book,
            ..OperationEvent::<K2>::from_facts(data)
        }
    }
}

impl<K: OperationKind> Element for OperationEvent<K> {
    fn get_uuid(&self) -> Uuid {
        self.data.get_uuid()
    }

    fn set_uuid(&mut self, uuid: Uuid) {
        self.data.set_uuid(uuid);
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

    fn get_hashcode(&self) -> u64 {
        self.data.get_hashcode()
    }

    fn set_hashcode(&mut self, hashcode: u64) {
        self.data.set_hashcode(hashcode);
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

    /// The operation's facts, its kind and its book scope digest to the
    /// code: the same entry as an order and as a quote are two operations.
    /// The rest of the book control is walk-time and feeds nothing.
    fn finalize(&mut self) {
        self.data.fill_market();
        self.data.fill_parents();
        self.data.sync_cross();
        let mut digest = self.data.digest_operation_event();
        {
            let mut staged = Staged::new(&mut digest);
            staged.feed(
                "marketdatakind",
                &i32::from(K::KIND.marketdatakind().code()).to_le_bytes(),
            );
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
    fn get_transunix(&self) -> i64 {
        self.data.get_transunix()
    }
    fn set_transunix(&mut self, unix: i64) {
        self.data.set_transunix(unix);
    }
    fn get_state(&self) -> &yggdryl::State {
        self.data.get_state()
    }
    fn set_state(&mut self, state: yggdryl::State) {
        self.data.set_state(state);
    }
    /// An execution reports one whatever its state, and an order or a
    /// quote where its state does - a partial fill, a fill - as the
    /// lifecycle reads it: a native leaf has no report kind of its own
    /// beside its state, so the state speaks.
    fn is_execution(&self) -> bool {
        matches!(K::KIND, MarketKind::Execution) || self.get_state().is_execution()
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
    fn get_sendunix(&self) -> Option<i64> {
        self.data.get_sendunix()
    }
    fn set_sendunix(&mut self, unix: Option<i64>) {
        self.data.set_sendunix(unix);
    }
    fn get_exprunix(&self) -> Option<i64> {
        self.data.get_exprunix()
    }
    fn set_exprunix(&mut self, unix: Option<i64>) {
        self.data.set_exprunix(unix);
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
    fn get_price(&self) -> Option<Decimal> {
        self.data.get_price()
    }
    fn set_price(&mut self, price: Option<Decimal>, overwrite: bool) {
        self.data.set_price(price, overwrite);
    }

    fn get_stoppx(&self) -> Option<Decimal> {
        self.data.get_stoppx()
    }

    fn set_stoppx(&mut self, value: Option<Decimal>, overwrite: bool) {
        self.data.set_stoppx(value, overwrite);
    }

    fn get_strikepx(&self) -> Option<Decimal> {
        self.data.get_strikepx()
    }

    fn set_strikepx(&mut self, value: Option<Decimal>, overwrite: bool) {
        self.data.set_strikepx(value, overwrite);
    }
    fn get_currency(&self) -> &yggdryl::Ccy {
        self.data.get_currency()
    }
    fn set_currency(&mut self, currency: yggdryl::Ccy, overwrite: bool) {
        self.data.set_currency(currency, overwrite);
    }
    fn get_origccy(&self) -> &yggdryl::Ccy {
        self.data.get_origccy()
    }
    fn set_origccy(&mut self, ccy: yggdryl::Ccy, overwrite: bool) {
        self.data.set_origccy(ccy, overwrite);
    }
    fn get_quantity(&self) -> Option<Decimal> {
        self.data.get_quantity()
    }
    fn set_quantity(&mut self, quantity: Option<Decimal>, overwrite: bool) {
        self.data.set_quantity(quantity, overwrite);
    }

    fn get_displayqty(&self) -> Option<Decimal> {
        self.data.get_displayqty()
    }

    fn set_displayqty(&mut self, value: Option<Decimal>, overwrite: bool) {
        self.data.set_displayqty(value, overwrite);
    }

    fn get_hiddenqty(&self) -> Option<Decimal> {
        self.data.get_hiddenqty()
    }

    fn set_hiddenqty(&mut self, value: Option<Decimal>, overwrite: bool) {
        self.data.set_hiddenqty(value, overwrite);
    }
    fn get_unit(&self) -> &yggdryl::Unit {
        self.data.get_unit()
    }
    fn set_unit(&mut self, unit: yggdryl::Unit, overwrite: bool) {
        self.data.set_unit(unit, overwrite);
    }
    fn get_side(&self) -> crate::Side {
        self.data.get_side()
    }
    fn set_side(&mut self, side: crate::Side, overwrite: bool) {
        self.data.set_side(side, overwrite);
    }
    fn marketdatakind(&self) -> crate::MarketDataKind {
        self.data.marketdatakind()
    }
    fn get_marketdatatype(&self) -> crate::MarketDataType {
        self.data.get_marketdatatype()
    }
    fn set_marketdatatype(&mut self, mdtype: crate::MarketDataType, overwrite: bool) {
        self.data.set_marketdatatype(mdtype, overwrite);
    }
    fn get_securityids(&self) -> &crate::Identifiers {
        self.data.get_securityids()
    }
    fn set_securityids(&mut self, ids: crate::Identifiers, overwrite: bool) -> yggdryl::Result<()> {
        self.data.set_securityids(ids, overwrite)
    }
    fn insert_securityid(&mut self, id: crate::Identifier) -> yggdryl::Result<bool> {
        self.data.insert_securityid(id)
    }
    fn remove_securityid(&mut self, key: &crate::IdKey) -> yggdryl::Result<bool> {
        self.data.remove_securityid(key)
    }
    fn derive_securityid(&mut self, kind: &crate::IdType, code: &str) -> bool {
        self.data.derive_securityid(kind, code)
    }
    fn get_cficode(&self) -> Option<&yggdryl::Cfi> {
        self.data.get_cficode()
    }
    fn set_cficode(&mut self, code: Option<yggdryl::Cfi>, overwrite: bool) {
        self.data.set_cficode(code, overwrite);
    }
    fn get_miccode(&self) -> Option<&yggdryl::Mic> {
        self.data.get_miccode()
    }
    fn set_miccode(&mut self, code: Option<yggdryl::Mic>, overwrite: bool) {
        self.data.set_miccode(code, overwrite);
    }
    fn get_execunix(&self) -> Option<i64> {
        self.data.get_execunix()
    }
    fn set_execunix(&mut self, unix: Option<i64>, overwrite: bool) {
        self.data.set_execunix(unix, overwrite);
    }
    fn get_lastpx(&self) -> Option<Decimal> {
        self.data.get_lastpx()
    }
    fn set_lastpx(&mut self, px: Option<Decimal>, overwrite: bool) {
        self.data.set_lastpx(px, overwrite);
    }
    fn get_lastqty(&self) -> Option<Decimal> {
        self.data.get_lastqty()
    }
    fn set_lastqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.data.set_lastqty(qty, overwrite);
    }
    fn get_avgpx(&self) -> Option<Decimal> {
        self.data.get_avgpx()
    }
    fn set_avgpx(&mut self, px: Option<Decimal>, overwrite: bool) {
        self.data.set_avgpx(px, overwrite);
    }
    fn get_cumqty(&self) -> Option<Decimal> {
        self.data.get_cumqty()
    }
    fn set_cumqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.data.set_cumqty(qty, overwrite);
    }
    fn get_leavesqty(&self) -> Option<Decimal> {
        self.data.get_leavesqty()
    }
    fn set_leavesqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.data.set_leavesqty(qty, overwrite);
    }

    fn get_cxlqty(&self) -> Option<Decimal> {
        self.data.get_cxlqty()
    }

    fn set_cxlqty(&mut self, value: Option<Decimal>, overwrite: bool) {
        self.data.set_cxlqty(value, overwrite);
    }
    fn get_prevpx(&self) -> Option<Decimal> {
        self.data.get_prevpx()
    }
    fn set_prevpx(&mut self, px: Option<Decimal>, overwrite: bool) {
        self.data.set_prevpx(px, overwrite);
    }
    fn get_prevqty(&self) -> Option<Decimal> {
        self.data.get_prevqty()
    }
    fn set_prevqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.data.set_prevqty(qty, overwrite);
    }
    fn get_spotrate(&self) -> Option<Decimal> {
        self.data.get_spotrate()
    }
    fn set_spotrate(&mut self, rate: Option<Decimal>, overwrite: bool) {
        self.data.set_spotrate(rate, overwrite);
    }
    fn get_forwardpoints(&self) -> Option<Decimal> {
        self.data.get_forwardpoints()
    }
    fn set_forwardpoints(&mut self, points: Option<Decimal>, overwrite: bool) {
        self.data.set_forwardpoints(points, overwrite);
    }
    fn get_ticker(&self) -> Option<&str> {
        self.data.get_ticker()
    }
    fn set_ticker(&mut self, ticker: Option<smol_str::SmolStr>, overwrite: bool) {
        self.data.set_ticker(ticker, overwrite);
    }
    fn get_instcode(&self) -> Option<&str> {
        self.data.get_instcode()
    }
    fn set_instcode(&mut self, code: Option<yggdryl::Str>, overwrite: bool) {
        self.data.set_instcode(code, overwrite);
    }
    fn get_metadata(&self) -> &super::market::Metadata {
        self.data.get_metadata()
    }
    fn set_metadata(&mut self, metadata: Option<super::market::Metadata>, overwrite: bool) {
        self.data.set_metadata(metadata, overwrite);
    }
    fn get_fxrates(&self) -> &super::market::FxRates {
        self.data.get_fxrates()
    }
    fn set_fxrates(&mut self, rates: super::market::FxRates, overwrite: bool) {
        self.data.set_fxrates(rates, overwrite);
    }
    fn get_bidpx(&self) -> Option<yggdryl::Decimal> {
        self.data.get_bidpx()
    }
    fn set_bidpx(&mut self, px: Option<yggdryl::Decimal>, overwrite: bool) {
        self.data.set_bidpx(px, overwrite);
    }
    fn get_bidqty(&self) -> Option<yggdryl::Decimal> {
        self.data.get_bidqty()
    }
    fn set_bidqty(&mut self, qty: Option<yggdryl::Decimal>, overwrite: bool) {
        self.data.set_bidqty(qty, overwrite);
    }
    fn get_bidccy(&self) -> Option<&yggdryl::Ccy> {
        self.data.get_bidccy()
    }
    fn set_bidccy(&mut self, ccy: Option<yggdryl::Ccy>, overwrite: bool) {
        self.data.set_bidccy(ccy, overwrite);
    }
    fn get_askpx(&self) -> Option<yggdryl::Decimal> {
        self.data.get_askpx()
    }
    fn set_askpx(&mut self, px: Option<yggdryl::Decimal>, overwrite: bool) {
        self.data.set_askpx(px, overwrite);
    }
    fn get_askqty(&self) -> Option<yggdryl::Decimal> {
        self.data.get_askqty()
    }
    fn set_askqty(&mut self, qty: Option<yggdryl::Decimal>, overwrite: bool) {
        self.data.set_askqty(qty, overwrite);
    }
    fn get_askccy(&self) -> Option<&yggdryl::Ccy> {
        self.data.get_askccy()
    }
    fn set_askccy(&mut self, ccy: Option<yggdryl::Ccy>, overwrite: bool) {
        self.data.set_askccy(ccy, overwrite);
    }
}

impl<K: OperationKind> Operation for OperationEvent<K> {
    fn get_ordqty(&self) -> Option<Decimal> {
        self.data.get_ordqty()
    }
    fn set_ordqty(&mut self, qty: Option<Decimal>, overwrite: bool) {
        self.data.set_ordqty(qty, overwrite);
    }
    fn get_timeinforce(&self) -> Option<&crate::TimeInForce> {
        self.data.get_timeinforce()
    }
    fn set_timeinforce(&mut self, tif: Option<crate::TimeInForce>, overwrite: bool) {
        self.data.set_timeinforce(tif, overwrite);
    }
    fn get_tradable(&self) -> Option<bool> {
        self.data.get_tradable()
    }
    fn set_tradable(&mut self, tradable: Option<bool>, overwrite: bool) {
        self.data.set_tradable(tradable, overwrite);
    }
    fn get_identifiers(&self) -> &crate::Identifiers {
        self.data.get_identifiers()
    }
    fn set_identifiers(&mut self, ids: crate::Identifiers, overwrite: bool) -> yggdryl::Result<()> {
        self.data.set_identifiers(ids, overwrite)
    }
    fn insert_identifier(&mut self, id: crate::Identifier) -> yggdryl::Result<bool> {
        self.data.insert_identifier(id)
    }
    fn remove_identifier(&mut self, key: &crate::IdKey) -> yggdryl::Result<bool> {
        self.data.remove_identifier(key)
    }
    fn get_partyids(&self) -> &crate::Identifiers {
        self.data.get_partyids()
    }
    fn set_partyids(
        &mut self,
        parties: crate::Identifiers,
        overwrite: bool,
    ) -> yggdryl::Result<()> {
        self.data.set_partyids(parties, overwrite)
    }
    fn insert_partyid(&mut self, party: crate::Identifier) -> yggdryl::Result<bool> {
        self.data.insert_partyid(party)
    }
    fn remove_partyid(&mut self, key: &crate::IdKey) -> yggdryl::Result<bool> {
        self.data.remove_partyid(key)
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
