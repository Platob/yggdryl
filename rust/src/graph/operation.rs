//! The one operation type: an order, a quote or an execution as a market
//! operation event, with the book-control facts a market-data entry carries.
//!
//! [`MarketOperation`] is what every market message expands to and what a book
//! holds: its [`OperationKind`] says which, its [`OperationEventData`]
//! holds every fact, and a boxed [`BookRef`] - absent on every operation that
//! is not a market-data entry, so one pointer - holds the typed facts a book
//! reads to place it: the update action, the scope, the position, and the
//! price and size the entry stated for itself. [`MarketOperationEntry`] is the same
//! operation undated. A composite trade is [`Trade`](super::Trade), whose
//! executions are operations of kind [`OperationKind::Execution`].

use smol_str::SmolStr;

use super::element::Staged;
use super::event::{OperationData, OperationEventData};
use super::{Element, Event, Market, Operation, OperationEvent};
use crate::{Decimal18, Error, Result, Uuid};

/// Which operation a value is: the stored `operationkind` of a row.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum OperationKind {
    /// An order: a party's intent to trade at a price.
    Order = 1,
    /// A quote: one or two lanes a party is willing to trade at.
    Quote = 2,
    /// An execution: a trade that happened.
    Execution = 3,
    /// A composite trade root: [`Trade`](super::Trade), on a row.
    Trade = 4,
}

impl OperationKind {
    /// The stored spelling: `order`, `quote`, `execution` or `trade`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Order => "order",
            Self::Quote => "quote",
            Self::Execution => "execution",
            Self::Trade => "trade",
        }
    }

    /// The kind a stored spelling names, ignoring ASCII case; `None` for
    /// any other text.
    #[must_use]
    pub fn read(text: &str) -> Option<Self> {
        [Self::Order, Self::Quote, Self::Execution, Self::Trade]
            .into_iter()
            .find(|kind| kind.as_str().eq_ignore_ascii_case(text))
    }
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
    const ALL: [Self; 7] = [
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

/// One market operation: an order, a quote or an execution, dated.
#[derive(Clone, Debug, PartialEq)]
pub struct MarketOperation {
    kind: OperationKind,
    data: OperationEventData,
    book: Option<Box<BookRef>>,
}

impl MarketOperation {
    /// An operation of `kind` over `data`, not yet finalized; a
    /// [`OperationKind::Trade`] is refused, because a trade root is a
    /// [`Trade`](super::Trade).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for the trade kind.
    pub fn new(kind: OperationKind, data: OperationEventData) -> Result<Self> {
        if kind == OperationKind::Trade {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.operationkind"),
                reason: SmolStr::new_static("a trade root is a Trade, not an MarketOperation"),
            });
        }
        Ok(Self {
            kind,
            data,
            book: None,
        })
    }

    /// An order over `data`.
    #[must_use]
    pub fn order(data: OperationEventData) -> Self {
        Self {
            kind: OperationKind::Order,
            data,
            book: None,
        }
    }

    /// A quote over `data`.
    #[must_use]
    pub fn quote(data: OperationEventData) -> Self {
        Self {
            kind: OperationKind::Quote,
            data,
            book: None,
        }
    }

    /// An execution over `data`.
    #[must_use]
    pub fn execution(data: OperationEventData) -> Self {
        Self {
            kind: OperationKind::Execution,
            data,
            book: None,
        }
    }

    /// Which operation this is.
    #[must_use]
    pub const fn kind(&self) -> OperationKind {
        self.kind
    }

    /// The facts this operation holds.
    #[must_use]
    pub fn data(&self) -> &OperationEventData {
        &self.data
    }

    /// The facts, moved out.
    #[must_use]
    pub fn into_data(self) -> OperationEventData {
        self.data
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

    /// This operation without its clocks and book control: a move.
    #[must_use]
    pub fn entry(self) -> MarketOperationEntry {
        MarketOperationEntry {
            kind: self.kind,
            data: self.data.into_entry(),
        }
    }
}

impl Default for MarketOperation {
    /// An order stating nothing, at the epoch.
    fn default() -> Self {
        Self::order(OperationEventData::default())
    }
}

impl AsRef<OperationEventData> for MarketOperation {
    fn as_ref(&self) -> &OperationEventData {
        &self.data
    }
}

impl AsMut<OperationEventData> for MarketOperation {
    fn as_mut(&mut self) -> &mut OperationEventData {
        &mut self.data
    }
}

impl Element for MarketOperation {
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
            staged.feed("operationkind", self.kind.as_str().as_bytes());
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
        if self.kind != other.kind {
            return None;
        }
        let data = std::mem::take(&mut self.data).merging_operation_event(&other.data)?;
        self.data = data;
        if self.book.is_none() && other.book.is_some() {
            self.book.clone_from(&other.book);
        }
        self.finalize();
        Some(self)
    }
}

delegate_event!(
    MarketOperation,
    data,
    restating = |mut this: MarketOperation, live: &MarketOperation| {
        let data = std::mem::take(&mut this.data).restating(&live.data);
        this.data = data;
        this.finalize();
        this
    },
    is_execution = |this: &MarketOperation| this.kind == OperationKind::Execution,
    set_currunix = |this: &mut MarketOperation, unix: i64| this.data.set_currunix(unix)
);
delegate_market!(MarketOperation, data);
delegate_operation!(MarketOperation, data);

/// One market operation undated: an order, a quote or an execution as an
/// entry a book level holds or a walk states at an instant.
#[derive(Clone, Debug, PartialEq)]
pub struct MarketOperationEntry {
    kind: OperationKind,
    data: OperationData,
}

impl MarketOperationEntry {
    /// An entry of `kind` over `data`, not yet finalized.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] for the trade kind.
    pub fn new(kind: OperationKind, data: OperationData) -> Result<Self> {
        if kind == OperationKind::Trade {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.operationkind"),
                reason: SmolStr::new_static("a trade root is a Trade, not an MarketOperationEntry"),
            });
        }
        Ok(Self { kind, data })
    }

    /// Which operation this is.
    #[must_use]
    pub const fn kind(&self) -> OperationKind {
        self.kind
    }

    /// The facts this entry holds.
    #[must_use]
    pub fn data(&self) -> &OperationData {
        &self.data
    }

    /// The facts, moved out.
    #[must_use]
    pub fn into_data(self) -> OperationData {
        self.data
    }

    /// This entry dated at `unix`, nanoseconds since the Unix epoch, and
    /// finalized: a move.
    #[must_use]
    pub fn at(self, unix: i64) -> MarketOperation {
        let mut operation = MarketOperation {
            kind: self.kind,
            data: self.data.at(unix),
            book: None,
        };
        operation.finalize();
        operation
    }
}

impl Default for MarketOperationEntry {
    /// An order entry stating nothing.
    fn default() -> Self {
        Self {
            kind: OperationKind::Order,
            data: OperationData::default(),
        }
    }
}

impl AsRef<OperationData> for MarketOperationEntry {
    fn as_ref(&self) -> &OperationData {
        &self.data
    }
}

impl AsMut<OperationData> for MarketOperationEntry {
    fn as_mut(&mut self) -> &mut OperationData {
        &mut self.data
    }
}

impl Element for MarketOperationEntry {
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

    fn finalize(&mut self) {
        self.data.fill_market();
        self.data.fill_operation();
        self.data.sync_cross();
        let mut digest = self.data.digest_operation();
        {
            let mut staged = Staged::new(&mut digest);
            staged.feed("operationkind", self.kind.as_str().as_bytes());
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
        if self.kind != other.kind {
            return None;
        }
        let data = std::mem::take(&mut self.data).merge_with(&other.data)?;
        self.data = data;
        self.finalize();
        Some(self)
    }
}

delegate_market!(MarketOperationEntry, data);
delegate_operation!(MarketOperationEntry, data);
