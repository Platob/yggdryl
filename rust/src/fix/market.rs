//! The one FIX boundary into typed graph market data.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::iter::FusedIterator;

use smallvec::SmallVec;
use smol_str::{SmolStr, format_smolstr};

use super::identity::{BOOK_ENTRY_TAGS, BOOK_ROOT_TAGS, TRADE_SIDE_TAGS};
use super::msg::{AccountsAt, Carried, Expanded, PartyCodes, Unmapped};
use super::{FixCodec, FixEntry, FixKey, FixMsg};
use crate::arrow::BatchReader;
use crate::graph::book::{ENTRY_ID, ENTRY_REF_ID};
use crate::graph::facts::OperationEventFacts;
use crate::graph::market::base_crosscode;
use crate::graph::{
    BookIterator, BookRef, Element, Event, ExecutionKind, Market, MarketData, MdUpdateAction,
    Operation, OperationEvent, OperationKind, OrderKind, QuoteKind, SnapshotEvent,
};
use crate::warning::warned;
use crate::{
    DataType, Decimal, Error, IdKey, IdType, Identifier, Identifiers, MarketDataKind, Result,
    Scalar, Side, State, TimeUnit,
};

const MD_ENTRIES: i32 = 268;
const TRADE_SIDES: i32 = 552;
const NANOS_PER_DAY: i64 = 86_400_000_000_000;

/// A book message becomes one leaf per `NoMDEntries(268)` occurrence, and
/// its root states the context each of them inherits.
const BOOK_EXPANSION: Expanded = Expanded {
    counter: MD_ENTRIES,
    reads: &BOOK_ENTRY_TAGS,
    inherited: &BOOK_ROOT_TAGS,
};

/// The execution a trade's parse splits off one `NoSides(552)` occurrence
/// keeps that occurrence alone, and its leaf carries the occurrence's own
/// members beside the message's.
const TRADE_EXPANSION: Expanded = Expanded {
    counter: TRADE_SIDES,
    reads: &TRADE_SIDE_TAGS,
    inherited: &[],
};

#[derive(Clone, Default)]
struct Facts {
    action: Option<SmolStr>,
    entry_type: Option<SmolStr>,
    entry_id: Option<SmolStr>,
    entry_ref_id: Option<SmolStr>,
    price: Option<SmolStr>,
    size: Option<SmolStr>,
    spotrate: Option<SmolStr>,
    forwardpoints: Option<SmolStr>,
    date: Option<SmolStr>,
    time: Option<SmolStr>,
    order_id: Option<SmolStr>,
    symbol: Option<SmolStr>,
    side: Option<SmolStr>,
    position: Option<SmolStr>,
    level: Option<SmolStr>,
    book_type: Option<SmolStr>,
    sub_book_type: Option<SmolStr>,
    feed_type: Option<SmolStr>,
    stream_id: Option<SmolStr>,
    market_id: Option<SmolStr>,
    market_segment_id: Option<SmolStr>,
    request_id: Option<SmolStr>,
    depth: Option<SmolStr>,
}

impl Facts {
    /// Records the first nonempty value of one tag an entry's facts read:
    /// exactly [`BOOK_ENTRY_TAGS`](super::identity::BOOK_ENTRY_TAGS), which
    /// is what keeps every other tag an entry states in its leaf's metadata.
    fn record(&mut self, tag: i32, value: &SmolStr) {
        let slot = match tag {
            279 => &mut self.action,
            269 => &mut self.entry_type,
            278 => &mut self.entry_id,
            280 => &mut self.entry_ref_id,
            270 => &mut self.price,
            271 => &mut self.size,
            1026 => &mut self.spotrate,
            1027 => &mut self.forwardpoints,
            272 => &mut self.date,
            273 => &mut self.time,
            37 => &mut self.order_id,
            55 => &mut self.symbol,
            54 => &mut self.side,
            290 => &mut self.position,
            1023 => &mut self.level,
            1021 => &mut self.book_type,
            1173 => &mut self.sub_book_type,
            1022 => &mut self.feed_type,
            1500 => &mut self.stream_id,
            1301 => &mut self.market_id,
            1300 => &mut self.market_segment_id,
            262 => &mut self.request_id,
            264 => &mut self.depth,
            _ => return,
        };
        if slot.is_none() && !value.is_empty() {
            *slot = Some(value.clone());
        }
    }

    fn overlay(&mut self, other: Self) {
        macro_rules! overlay {
            ($($member:ident),+ $(,)?) => {
                $(if other.$member.is_some() { self.$member = other.$member; })+
            };
        }
        overlay!(
            action,
            entry_type,
            entry_id,
            entry_ref_id,
            price,
            size,
            spotrate,
            forwardpoints,
            date,
            time,
            order_id,
            symbol,
            side,
            position,
            level,
            book_type,
            sub_book_type,
            feed_type,
            stream_id,
            market_id,
            market_segment_id,
            request_id,
            depth,
        );
    }

    /// Takes the root's context: exactly
    /// [`BOOK_ROOT_TAGS`](super::identity::BOOK_ROOT_TAGS), which is what
    /// keeps every other tag a root states in its leaves' metadata.
    fn inherit_context(&mut self, root: &Self) {
        self.symbol.clone_from(&root.symbol);
        self.book_type.clone_from(&root.book_type);
        self.sub_book_type.clone_from(&root.sub_book_type);
        self.feed_type.clone_from(&root.feed_type);
        self.stream_id.clone_from(&root.stream_id);
        self.market_id.clone_from(&root.market_id);
        self.market_segment_id.clone_from(&root.market_segment_id);
        self.request_id.clone_from(&root.request_id);
        self.depth.clone_from(&root.depth);
        self.date.clone_from(&root.date);
        self.time.clone_from(&root.time);
    }
}

struct BookEntry {
    facts: Facts,
    /// The accounts the entry's own parties name, which lead the message's
    /// on its leaf.
    accounts: Identifiers,
    position: usize,
    date_days: Option<i64>,
    time_nanos: Option<i64>,
    /// `MDEntryPx(270)`, or the value no exact decimal reads: whether that
    /// excludes the entry is its action's question.
    price: std::result::Result<Option<Decimal>, String>,
    size: Option<Decimal>,
    /// `MDEntrySpotRate(1026)` and `MDEntryForwardPoints(1027)`: the FX
    /// parts of the level's price.
    spotrate: Option<Decimal>,
    forwardpoints: Option<Decimal>,
    empty_snapshot: bool,
}

impl FixMsg {
    /// Reads this message as graph market data: one leaf per message.
    ///
    /// An order, a quote and an execution are one operation each, the leaf
    /// of their category. A trade is none: what it reports are the sided
    /// executions [its parse splits off](FixCodec::parse_line), each an
    /// execution message of its own, so a trade and its executions are
    /// never stated twice. A FIX
    /// `W` or `X` book message is expanded in nondecreasing effective time,
    /// stably retaining `NoMDEntries(268)` source order for equal instants; an
    /// empty `W` is one scoped snapshot control, and so is a `W` stating no
    /// `NoMDEntries(268)` group at all. The structured entry tree is derived
    /// at most once and then walked once.
    ///
    /// What the message states never fails the reading; it is read at the
    /// smallest part that can stand, and what is passed over is said once
    /// as a deduplicated warning naming the tag, the value and where:
    ///
    /// - A message with no market reading answers no leaf: an
    ///   administrative message, an execution report of no fill, a book
    ///   message other than `W` or `X`. A trade and a batch answer none in
    ///   silence: their leaves are the messages their parse splits off.
    /// - A book entry that cannot stand is excluded and the others stand:
    ///   an `MDEntryType(269)` absent or other than a bid, an offer or a
    ///   trade, an `X` entry with no incremental `MDUpdateAction(279)`, an
    ///   anonymous incremental update naming no entry, and a new or snapshot
    ///   entry whose `MDEntryPx(270)` is no exact decimal, which no level
    ///   could rest at.
    /// - A fact that does not decide the entry takes its default: a price
    ///   an update restates, a size or an FX part that is no exact decimal
    ///   is null, an entry clock that names no instant is the message's, an
    ///   unreadable `Side(54)` on a trade entry is [`Side::Unknown`].
    /// - A `W` or `X` stating several `NoMDEntries(268)` groups answers no
    ///   leaf, since which one is the book's cannot be chosen; one whose
    ///   typed group does not match its entries reads each entry off the
    ///   FIX tree alone.
    ///
    /// # Errors
    ///
    /// None from what the message states: the refusals above are warnings.
    ///
    /// # Metadata
    ///
    /// Every leaf carries, in its [`Market::get_metadata`], what the message
    /// states that no typed column reads: the bridge's namespaced keys, then
    /// every other field under its name as the canonical text it spells. A
    /// group or a component is one key, its name - `miscfees`,
    /// `tradereportorderdetail` - holding JSON: a group an array of objects,
    /// one per occurrence it keeps, a component one object, each member
    /// under its name and each leaf value the same text it spells at the
    /// root, so no JSON number appears and the keys are in name order. A
    /// book entry and a trade side add their own occurrence's scalar
    /// members, keyed bare and leading a message field of the same name,
    /// and a nested member of that occurrence as JSON under its bare name.
    /// The envelope a message's code leaves out stays out, so two hops of
    /// one message are one leaf. [`FixCodec::with_market_metadata`] turns it
    /// off for the codec's own doors; this door always carries it.
    ///
    /// What the leaf's identifier sets hold is no metadata: a party its
    /// `partyids` hold under its role - a book entry's own parties
    /// leading the message's, as a trade side's do - the `Account(1)` held
    /// as `account`, and a regulatory trade identifier its `identifiers`
    /// hold. A second party of one role, or a value no set takes, stays. A
    /// scalar whose key names an identifier - one the message's type
    /// declares under `FIX:identifiers` at its end, `RefOrderID(1080)`'s
    /// `reforderid`, or, for a key no dictionary field is, one of the
    /// crate's identifier names, read as [`Identifier::from_key`] reads a
    /// key: a bridge's `firm.x.ParentOrderID` is `firm.x:parentorderid` -
    /// is lifted into the set its type belongs to (`securityids`,
    /// `partyids` or `identifiers`) where that set holds its key free or
    /// with the same value, and otherwise stays; an occurrence's own member
    /// is read against what its component declares. An empty snapshot's
    /// control holds no set, and keeps all of it. Nothing is lifted where
    /// the metadata is off.
    pub fn market_data(&self) -> Result<Vec<MarketData>> {
        Ok(operations(self, self.event().clone()))
    }

    /// Moves this message into graph market data.
    ///
    /// A direct order, quote or execution moves the facts the
    /// message holds without cloning them, then finalizes the leaf's
    /// identity from those projected facts. Book messages
    /// necessarily make one owned event per `NoMDEntries(268)` occurrence, or
    /// one scoped snapshot control for an empty `W`.
    ///
    /// # Errors
    ///
    /// None from what the message states, as [`Self::market_data`].
    pub fn into_market_data(self) -> Result<Vec<MarketData>> {
        Ok(expand_message(self, true).into_vec())
    }
}

/// A lazy projection of sorted FIX messages into graph market operations.
/// Direct messages occupy no intermediate vector; one book message retains
/// only its own expanded entries. Each leaf takes the
/// [place](crate::graph::Event::get_seqnum) its message holds - the parse's,
/// or the lifecycle's where the messages were walked - so a chain's steps at
/// one instant keep the order the walk gave them whatever order a table
/// read them back in, and the entries one book message states share it.
///
/// Each message expands as [`FixMsg::into_market_data`] does. A message
/// its intake refused for what it states is passed over with a warning,
/// and an operation dated before one already yielded is yielded with a
/// warning, a book excluding it. Only the source's own failure
/// ([`Error::is_source_failure`]) ends the projection: it is yielded after
/// every operation before it, and the iterator is fused.
pub struct FixMarketIterator<I>
where
    I: Iterator,
    I::Item: Into<Result<FixMsg>>,
{
    source: I,
    current: Option<MessageOperations>,
    last_unix: Option<i64>,
    done: bool,
    /// Whether each leaf carries its message's unmapped fields.
    market_metadata: bool,
}

impl<I> FixMarketIterator<I>
where
    I: Iterator,
    I::Item: Into<Result<FixMsg>>,
{
    /// Opens a projection over messages already sorted by event time,
    /// each leaf carrying its message's unmapped fields as
    /// [`FixMsg::market_data`] does.
    #[must_use]
    pub fn new(source: I) -> Self {
        Self {
            source,
            current: None,
            last_unix: None,
            done: false,
            market_metadata: true,
        }
    }

    /// This projection with the codec's metadata switch.
    fn with_market_metadata(mut self, market_metadata: bool) -> Self {
        self.market_metadata = market_metadata;
        self
    }
}

impl<I> Iterator for FixMarketIterator<I>
where
    I: Iterator,
    I::Item: Into<Result<FixMsg>>,
{
    type Item = Result<MarketData>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            if let Some(operation) = self.current.as_mut().and_then(Iterator::next) {
                let unix = effective_unix(&operation);
                match self.last_unix {
                    Some(previous) if unix < previous => warned!(
                        "FIX market data operation yielded out of order: a book excludes it",
                        operation.marketdatakind().as_str(),
                        "{} is dated {unix}, before the {previous} an operation before it was",
                        operation.get_crosscode()
                    ),
                    _ => self.last_unix = Some(unix),
                }
                return Some(Ok(operation));
            }
            self.current = None;
            let Some(message) = self.source.next() else {
                self.done = true;
                return None;
            };
            match intake(message.into()) {
                Some(Ok(message)) => {
                    self.current = Some(expand_message(message, self.market_metadata));
                }
                Some(Err(failure)) => {
                    self.done = true;
                    return Some(Err(failure));
                }
                None => {}
            }
        }
    }
}

impl<I> FusedIterator for FixMarketIterator<I>
where
    I: Iterator,
    I::Item: Into<Result<FixMsg>>,
{
}

/// Whether a message is one of a book's inputs: an order, a quote stating
/// its side, an execution, a `W` or `X` book message. A trade and a quote
/// quoting sides it states no `Side(54)` for are not: what they report are
/// the messages their parse splits off, so admitting them would state each
/// fill or side twice. Nor is an execution report of no fill, which is its
/// order's report in a lifecycle and states nothing a book folds.
fn contributes_to_book(message: &FixMsg) -> bool {
    match message.msgcat() {
        MarketDataKind::Order => !message.reports_no_fill(),
        MarketDataKind::Quotation => !message.reports_no_fill() && quoted_sides(message).is_empty(),
        MarketDataKind::Execution => message.is_execution(),
        MarketDataKind::Book => matches!(message.header().msgtype(), "W" | "X"),
        _ => false,
    }
}

/// The sides a quote stating no side of its own quotes: `BUYS` where it
/// states a bid's facts, `SELL` where it states an offer's, in that order -
/// the sided quotes its parse splits it into. Nothing for a quote stating
/// its side, or stating neither side's facts.
fn quoted_sides(message: &FixMsg) -> SmallVec<[Side; 2]> {
    let mut sides = SmallVec::new();
    if message.get_side() != Side::Unknown {
        return sides;
    }
    let lifted = message.lifted();
    if message.get_bidpx().is_some()
        || message.get_bidqty().is_some()
        || lifted.bidspotrate().is_some()
        || lifted.bidforwardpoints().is_some()
    {
        sides.push(Side::Buy);
    }
    if message.get_askpx().is_some()
        || message.get_askqty().is_some()
        || lifted.offerspotrate().is_some()
        || lifted.offerforwardpoints().is_some()
    {
        sides.push(Side::Sell);
    }
    sides
}

impl FixCodec {
    /// Streams sorted FIX messages through their graph market data and
    /// the stateful book iterator into bounded Arrow batches of
    /// [`MarketData::field`] rows, each a `book_event`.
    ///
    /// Records outside orders, one-sided quotes, executions and `W`/`X`
    /// book messages are ignored: a trade and a quote stating no side reach the
    /// book as the messages their parse split off. An admitted message is
    /// read as [`FixMarketIterator`] reads it: what it states that cannot
    /// stand is passed over with a warning, and the source's own failure
    /// follows the completed book prefix and fuses the returned reader.
    ///
    /// This method does not run a lifecycle implicitly: callers that need
    /// lifecycle enrichment pass [`Self::lifecycle`] as the source. Input is
    /// pulled lazily. Each leaf carries its message's unmapped fields where
    /// [`Self::market_metadata`] says so.
    pub fn book_arrow_reader<I>(&self, messages: I, snapshot_millis: u64) -> Result<BatchReader>
    where
        I: IntoIterator,
        I::Item: Into<Result<FixMsg>>,
        I::IntoIter: Send + 'static,
    {
        let admitted = messages
            .into_iter()
            .filter_map(|message| match message.into() {
                Ok(message) => contributes_to_book(&message).then_some(Ok(message)),
                failure => Some(failure),
            });
        let operations =
            FixMarketIterator::new(admitted).with_market_metadata(self.market_metadata());
        let books = BookIterator::new(operations, snapshot_millis)?;
        MarketData::arrow_reader(
            books.map(|book| book.map(MarketData::from)),
            Some(self.batch_row_size()),
            Some(self.batch_byte_size()),
        )
    }

    /// A capture of FIX messages as the market data its book
    /// messages, orders, quotes, executions and trades are, in the order a
    /// book folds them.
    ///
    /// It admits exactly what [`Self::book_arrow_reader`] admits - orders,
    /// one-sided quotes, executions and `W` and `X` book messages - and
    /// expands each admitted message into its leaves
    /// as [`FixMsg::into_market_data`] does, each carrying its
    /// message's unmapped fields where [`Self::market_metadata`] says so.
    /// Neither [`Self::lifecycle`] nor [`Self::reads_msgtype`] runs here: a
    /// caller wanting the walk passes `self.lifecycle(messages)` as the
    /// source.
    ///
    /// The capture is collected, so it is bounded by the capture's own size,
    /// and the operations are then stably sorted by the instant a book folds
    /// them at - the snapshot instant a walk states, else the event's own -
    /// which is the key [`FixMarketIterator`] checks, so the answer never
    /// regresses: an entry clock a book message states may stand before an
    /// earlier message's, and sorting the operations rather than the
    /// messages is what places it.
    ///
    /// Nothing a message states fails the capture: a message its intake
    /// refused for what it states is passed over with a warning, and each
    /// admitted message answers what [`FixMsg::into_market_data`] answers.
    /// The source's own failure ([`Error::is_source_failure`]) ends the
    /// capture where it happens: it is yielded after the operations of every
    /// message before it, and no later message is read. The iterator is
    /// fused.
    pub fn market_data<I>(
        &self,
        messages: I,
    ) -> impl FusedIterator<Item = Result<MarketData>> + Send + 'static
    where
        I: IntoIterator,
        I::Item: Into<Result<FixMsg>>,
    {
        let mut failure = None;
        let mut operations = Vec::new();
        for message in messages {
            match intake(message.into()) {
                Some(Ok(message)) if contributes_to_book(&message) => {
                    operations.extend(expand_message(message, self.market_metadata()));
                }
                Some(Err(error)) => {
                    failure = Some(error);
                    break;
                }
                Some(Ok(_)) | None => {}
            }
        }
        operations.sort_by_key(effective_unix);
        operations
            .into_iter()
            .map(Ok)
            .chain(failure.into_iter().map(Err))
    }

    /// [`Self::market_data`] as bounded Arrow batches of
    /// [`MarketData::field`] rows, closing as [`Self::book_arrow_reader`]
    /// closes them: every row of the capture read before a source failure,
    /// then that failure.
    ///
    /// # Errors
    ///
    /// Returns an error when the row field cannot be built.
    pub fn market_arrow_reader<I>(&self, messages: I) -> Result<BatchReader>
    where
        I: IntoIterator,
        I::Item: Into<Result<FixMsg>>,
    {
        MarketData::arrow_reader(
            self.market_data(messages),
            Some(self.batch_row_size()),
            Some(self.batch_byte_size()),
        )
    }
}

// The direct-message hot path stays inline so one order, quote, execution or
// trade does not pay a heap allocation merely to satisfy the iterator shape.
#[allow(clippy::large_enum_variant)]
enum MessageOperations {
    One(Option<MarketData>),
    Many(std::vec::IntoIter<MarketData>),
}

impl MessageOperations {
    /// Recovers the expanded allocation instead of collecting it through the
    /// type-erased iterator path.
    fn into_vec(self) -> Vec<MarketData> {
        match self {
            Self::One(operation) => operation.into_iter().collect(),
            Self::Many(operations) => operations.collect(),
        }
    }
}

impl Iterator for MessageOperations {
    type Item = MarketData;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::One(operation) => operation.take(),
            Self::Many(operations) => operations.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::One(operation) => {
                let len = usize::from(operation.is_some());
                (len, Some(len))
            }
            Self::Many(operations) => operations.size_hint(),
        }
    }
}

impl ExactSizeIterator for MessageOperations {}
impl FusedIterator for MessageOperations {}

/// What one intake item is to a market door: the message, or the source's
/// own failure, which ends the door. A refusal of what one message states
/// is said as a warning and passed over (`None`).
fn intake(item: Result<FixMsg>) -> Option<Result<FixMsg>> {
    match item {
        Err(error) if !error.is_source_failure() => {
            warned!(
                "FIX message excluded from market data: its intake refused what it states",
                "intake",
                "{error}"
            );
            None
        }
        item => Some(item),
    }
}

/// The leaves one message moves into, each carrying its message's unmapped
/// fields where `metadata` says so: the one owner of expansion.
fn expand_message(message: FixMsg, metadata: bool) -> MessageOperations {
    if let Some(kind) = direct_of(&message) {
        let carried = metadata.then(|| direct_unmapped(&message));
        let mut facts = OperationEventFacts::from(message);
        carry(&mut facts, carried, true);
        return MessageOperations::One(Some(operation(kind, facts, None)));
    }
    if !is_book_message(&message) {
        return MessageOperations::One(None);
    }
    let msgtype = SmolStr::new(message.header().msgtype());
    let entries = book_entries(&message);
    let unmapped =
        metadata.then(|| message.unmapped(Some(&BOOK_EXPANSION), holds_operations(&entries)));
    let operations = build_book_operations(
        OperationEventFacts::from(message),
        &msgtype,
        &entries,
        unmapped,
    );
    MessageOperations::Many(operations.into_iter())
}

/// Whether a message that is no direct operation is a `W` or `X` book
/// message, saying why not where it is not: a trade and a batch in
/// silence, since their leaves are the messages their parse splits off,
/// any other message with a warning.
fn is_book_message(message: &FixMsg) -> bool {
    let category = message.msgcat();
    let msgtype = message.header().msgtype();
    if category == MarketDataKind::Book && matches!(msgtype, "W" | "X") {
        return true;
    }
    if message.reports_no_fill() {
        warned!(
            "FIX message excluded from market data: an execution report of no fill states no fill",
            msgtype,
            "a {msgtype:?} report of category {} answers no leaf",
            category.as_str()
        );
    } else if category != MarketDataKind::Trade && !category.is_batch() {
        warned!(
            "FIX message excluded from market data: it is no order, quote, execution or W/X book message",
            msgtype,
            "a {msgtype:?} message of category {} answers no leaf",
            category.as_str()
        );
    }
    false
}

/// What a direct message's leaf carries: the message's unmapped fields,
/// and for the execution a trade's parse split off one side, that side's
/// own members beside them.
fn direct_unmapped(message: &FixMsg) -> Carried {
    if message.header().msgtype() == "AE" && message.msgcat() == MarketDataKind::Execution {
        return message.unmapped(Some(&TRADE_EXPANSION), true).leaf(0);
    }
    message.unmapped(None, true).into_carried()
}

/// Whether the leaves of a book message's `entries` are operations, whose
/// identifier maps hold what the message states: an empty snapshot's is a
/// control holding none, so its metadata keeps every party and account.
fn holds_operations(entries: &[BookEntry]) -> bool {
    !entries.first().is_some_and(|entry| entry.empty_snapshot)
}

/// States what a leaf carries on its facts before the leaf is finalized,
/// where there is any: its metadata, less each scalar its identifier sets
/// hold once it is lifted - into a key the set does not hold yet, or one
/// holding the same value - where `lift` asks. What they do not hold, a
/// key holding another value, stays in the metadata.
fn carry(facts: &mut OperationEventFacts, carried: Option<Carried>, lift: bool) {
    let Some(Carried {
        mut metadata,
        lifted,
    }) = carried
    else {
        return;
    };
    for (key, value, id) in lifted {
        if !(lift && lift_identifier(facts, id)) {
            metadata.entry(key).or_insert(value);
        }
    }
    facts.set_metadata(Some(metadata), true);
}

/// Lifts one identifier a key named into the set its type belongs to - a
/// security type the `securityids`, a party the `partyids`, any other the
/// `identifiers` - where that set holds its key free or with the same
/// value, saying whether it holds it now.
fn lift_identifier(facts: &mut OperationEventFacts, id: Identifier) -> bool {
    let kind = id.kind();
    let set = if kind.is_security() {
        facts.get_securityids()
    } else if kind.is_party() {
        facts.get_partyids()
    } else {
        facts.get_identifiers()
    };
    if let Some(held) = set.get_from(id.key()) {
        return held == id.value();
    }
    let inserted = if kind.is_security() {
        facts.insert_securityid(id)
    } else if kind.is_party() {
        facts.insert_partyid(id)
    } else {
        facts.insert_identifier(id)
    };
    inserted.unwrap_or(false)
}

fn effective_unix(input: &MarketData) -> i64 {
    input.as_event().map_or(0, |event| {
        event.get_snapunix().unwrap_or_else(|| event.get_currunix())
    })
}

impl FixMsg {
    /// Moves this message into the one market data leaf it is: a direct
    /// order, quote or execution, or a book message stating one entry.
    /// [`MarketData::from`] holds the message whole instead.
    ///
    /// # Errors
    ///
    /// Returns an error naming `MsgType(35)`, or `NoMDEntries(268)` for a
    /// book message, where the message is not exactly one leaf.
    pub fn into_market_leaf(self) -> Result<MarketData> {
        let message = self;
        let path = if message.msgcat() == MarketDataKind::Book {
            "$.NoMDEntries(268)"
        } else {
            "$.MsgType(35)"
        };
        let mut operations = expand_message(message, true);
        if operations.len() != 1 {
            return Err(invalid(
                path,
                format_smolstr!(
                    "expected exactly one market data element, got {}",
                    operations.len()
                ),
            ));
        }
        Ok(operations.next().expect("one operation"))
    }
}

/// [`expand_message`] over a borrowed message, every leaf carrying its
/// unmapped fields.
fn operations(message: &FixMsg, mut base: OperationEventFacts) -> Vec<MarketData> {
    if let Some(kind) = direct_of(message) {
        carry(&mut base, Some(direct_unmapped(message)), true);
        return vec![operation(kind, base, None)];
    }
    if !is_book_message(message) {
        return Vec::new();
    }
    let entries = book_entries(message);
    let unmapped = message.unmapped(Some(&BOOK_EXPANSION), holds_operations(&entries));
    build_book_operations(base, message.header().msgtype(), &entries, Some(unmapped))
}

/// Which operation leaf a message or a market-data entry becomes.
#[derive(Clone, Copy)]
enum Direct {
    Order,
    Quote,
    Execution,
}

/// The leaf a message is directly, where it is one - never an execution
/// report of no fill, its order's report in a lifecycle, which states no
/// fill a leaf holds ([`is_book_message`] says so).
fn direct_of(message: &FixMsg) -> Option<Direct> {
    if message.reports_no_fill() {
        return None;
    }
    direct_kind(message.msgcat(), message.is_execution())
}

fn direct_kind(category: MarketDataKind, is_execution: bool) -> Option<Direct> {
    match category {
        MarketDataKind::Order => Some(Direct::Order),
        MarketDataKind::Quotation => Some(Direct::Quote),
        MarketDataKind::Execution if is_execution => Some(Direct::Execution),
        _ => None,
    }
}

/// The finalized leaf of `kind` over `data`, with the book control a
/// market-data entry states. An execution is one fill, complete in
/// itself: its leaf reads `FILLED` whatever its report's state, unless a
/// book message deleted it.
fn operation(kind: Direct, mut data: OperationEventFacts, book: Option<BookRef>) -> MarketData {
    if matches!(kind, Direct::Execution) && *data.get_state() != State::Canceled {
        data.set_state(State::Filled);
    }
    fn leaf<K: OperationKind>(
        data: OperationEventFacts,
        book: Option<BookRef>,
    ) -> OperationEvent<K> {
        let mut operation = OperationEvent::<K>::from_facts(data);
        operation.set_book(book);
        operation.finalize();
        operation
    }
    match kind {
        Direct::Order => MarketData::from(leaf::<OrderKind>(data, book)),
        Direct::Quote => MarketData::from(leaf::<QuoteKind>(data, book)),
        Direct::Execution => MarketData::from(leaf::<ExecutionKind>(data, book)),
    }
}

/// Where a batch message states its entries: the counter of the repeating
/// group whose occurrences are the orders, quotes or trades it states, and,
/// for a mass quote and its acknowledgement, the counter of the group
/// inside each occurrence holding the quotes. A batch type with no entry
/// group - a list's cancel or execute request, a mass order's criteria -
/// splits into nothing.
fn batch_groups(msgtype: &str) -> Option<(i32, Option<i32>)> {
    Some(match msgtype {
        // NewOrderList, ListStatus: NoOrders.
        "E" | "N" => (73, None),
        // MassOrder, MassOrderAck: NoOrderEntries.
        "DJ" | "DK" => (2428, None),
        // NewOrderCross and its replace and cancel: NoSides.
        "s" | "t" | "u" => (TRADE_SIDES, None),
        // OrderMassCancelReport, OrderMassActionReport: NoAffectedOrders.
        "r" | "BZ" => (534, None),
        // MassQuote, MassQuoteAck: NoQuoteSets, then NoQuoteEntries.
        "i" | "b" => (296, Some(295)),
        // BidRequest, BidResponse: NoBidComponents.
        "k" | "l" => (420, None),
        // ListStrikePrice: NoStrikes.
        "m" => (428, None),
        // TradeMatchReport: NoInstrmtMatchSides.
        "DC" => (1889, None),
        _ => return None,
    })
}

/// The root tag a batch entry's member is stated under where it is not its
/// own: an affected order's identifiers are the order's.
fn entry_root_tag(tag: i32) -> i32 {
    match tag {
        535 => 37,  // AffectedOrderID: OrderID
        1824 => 41, // AffectedOrigClOrdID: OrigClOrdID
        536 => 198, // AffectedSecondaryOrderID: SecondaryOrderID
        other => other,
    }
}

/// One member a batch entry states at the root of the message it becomes:
/// a field under the root tag it is written at ([`entry_root_tag`]), or a
/// nested group whole under its own name.
#[derive(Clone)]
enum Member {
    Tag(i32, Scalar),
    Group(SmolStr, Scalar),
}

impl Member {
    fn write(&self) -> (FixKey<'_>, Scalar) {
        match self {
            Self::Tag(tag, value) => (FixKey::Tag(*tag), value.clone()),
            Self::Group(name, value) => (FixKey::Name(name.as_str()), value.clone()),
        }
    }
}

/// The members a typed batch occurrence states: a component's members read
/// through, a nested group kept whole - the one counted by `skip` left out,
/// since its entries are split on their own - and a null skipped.
fn entry_members(
    message: &FixMsg,
    fields: &[crate::Field],
    values: &[Scalar],
    skip: Option<i32>,
    members: &mut Vec<Member>,
) {
    for (field, value) in fields.iter().zip(values) {
        if value.is_null() {
            continue;
        }
        match super::schema::tag_and_counter(message.registry(), field) {
            (_, Some(counter)) if Some(counter) == skip => {}
            (_, Some(_)) => members.push(Member::Group(SmolStr::new(field.name()), value.clone())),
            (Some(tag), None) => members.push(Member::Tag(entry_root_tag(tag), value.clone())),
            (None, None) if matches!(field.dtype(), DataType::Struct(_)) => {
                if let Some(nested) = value.as_sequence() {
                    entry_members(message, field.fields(), nested, skip, members);
                }
            }
            (None, None) => {}
        }
    }
}

/// The text one member states, where it states text.
fn member_text(members: &[Member], tag: i32) -> Option<&str> {
    members
        .iter()
        .rev()
        .find_map(|member| match member {
            Member::Tag(held, value) if *held == tag => value.as_str(),
            _ => None,
        })
        .filter(|text| !text.is_empty())
}

/// The chain one batch entry is walked under: the order it names - its
/// `OrderID(37)`, `ClOrdID(11)` or `OrigClOrdID(41)`, as a single order
/// message names it, so the entry joins that order's chain - else its
/// quote entry, `QuoteEntryID(299)` within its `QuoteSetID(302)`, else its
/// order entry, `OrderEntryID(2430)`, else its place in the batch.
fn entry_crosscode(batch: &FixMsg, members: &[Member], place: &str) -> String {
    if let Some(order) = [37, 11, 41]
        .into_iter()
        .find_map(|tag| member_text(members, tag))
    {
        return order.to_owned();
    }
    if let Some(entry) = member_text(members, 299) {
        return match member_text(members, 302) {
            Some(set) => format!("QuoteSetID={set}|QuoteEntryID={entry}"),
            None => format!("QuoteEntryID={entry}"),
        };
    }
    if let Some(entry) = member_text(members, 2430) {
        return format!("OrderEntryID={entry}");
    }
    let base = match batch.get_crosscode() {
        "" => batch.header().msgtype(),
        code => code,
    };
    format!("{base}|{place}")
}

/// The messages a batch splits into, one per entry of its entry group
/// ([`batch_groups`]), each split again as a message of its category is.
fn batch_entries(batch: &FixMsg) -> Vec<FixMsg> {
    let Some((counter, inner)) = batch_groups(batch.header().msgtype()) else {
        return Vec::new();
    };
    let Some(group_at) = batch.index_of_group(counter) else {
        return Vec::new();
    };
    let Some(group) = batch.as_field().fields().get(group_at) else {
        return Vec::new();
    };
    let name = SmolStr::new(group.name());
    let Some(item) = group
        .dtype()
        .as_serie_type()
        .map(|sequence| sequence.item().clone())
    else {
        return Vec::new();
    };
    let Some(rows) = batch
        .as_value()
        .as_sequence()
        .and_then(|values| values.get(group_at))
        .and_then(Scalar::sequence_rows)
    else {
        return Vec::new();
    };
    let kind = batch.msgcat().item();
    let mut answer = Vec::new();
    for (index, occurrence) in rows.iter().enumerate() {
        let Some(values) = occurrence.as_sequence() else {
            crate::warning::warned!(
                "FIX batch entry excluded: its row is not a record",
                batch.header().msgtype(),
                "{name}[{index}] holds {}",
                occurrence.kind()
            );
            continue;
        };
        let mut members = Vec::new();
        entry_members(batch, item.fields(), values, inner, &mut members);
        let place = format!("{counter}:{index}");
        let Some(inner) = inner else {
            answer.extend(batch_entry(batch, &name, kind, members, &place));
            continue;
        };
        let nested = item.fields().iter().zip(values).find(|(field, _)| {
            super::schema::tag_and_counter(batch.registry(), field).1 == Some(inner)
        });
        let Some((field, value)) = nested else {
            continue;
        };
        let (Some(sequence), Some(entries)) =
            (field.dtype().as_serie_type(), value.sequence_rows())
        else {
            continue;
        };
        for (at, entry) in entries.iter().enumerate() {
            let Some(entry) = entry.as_sequence() else {
                continue;
            };
            let mut own = members.clone();
            entry_members(batch, sequence.item().fields(), entry, None, &mut own);
            let place = format!("{place}|{inner}:{at}");
            answer.extend(batch_entry(batch, &name, kind, own, &place));
        }
    }
    answer
}

/// One batch entry as a message of `kind`, then what it splits into: the
/// batch without its entry group, `members` at the root, chained by
/// [`entry_crosscode`] and naming the batch among its sources.
fn batch_entry(
    batch: &FixMsg,
    group: &str,
    kind: MarketDataKind,
    members: Vec<Member>,
    place: &str,
) -> Vec<FixMsg> {
    let crosscode = entry_crosscode(batch, &members, place);
    let mut entry = batch.clone();
    if let Err(error) = entry.remove(FixKey::Name(group)) {
        crate::warning::warned!(
            "FIX batch entry excluded: its batch group could not be taken off",
            batch.header().msgtype(),
            "{group} at {place}: {error}"
        );
        return Vec::new();
    }
    // A member the root cannot hold is passed over, as the lenient write
    // passes it: the entry keeps what reads.
    if let Err(error) = entry.set_each(members.iter().map(Member::write)) {
        crate::warning::warned!(
            "FIX batch entry excluded: its members do not make a message",
            batch.header().msgtype(),
            "{group} at {place}: {error}"
        );
        return Vec::new();
    }
    entry.record(
        super::MARKETDATAKIND_TAG_NAME.0,
        &Scalar::MarketDataKind(kind),
    );
    entry.set_crosscode(crosscode);
    entry.set_srcuuids(provenance(batch));
    entry.settle();
    let (entry, split) = entry.split();
    std::iter::once(entry).chain(split).collect()
}

/// The facts a trade side states at the root of the execution its trade
/// splits off: each `NoSides(552)` member beside the root tag an
/// `ExecutionReport` states it under, the side's own and the fill's.
const SIDE_ROOT_TAGS: [(i32, i32); 10] = [
    (54, 54),   // Side
    (1427, 17), // SideExecID: ExecID
    (37, 37),   // OrderID
    (11, 11),   // ClOrdID
    (41, 41),   // OrigClOrdID
    (198, 198), // SecondaryOrderID
    (526, 526), // SecondaryClOrdID
    (1009, 32), // SideLastQty: LastQty
    (1852, 6),  // SideAvgPx: AvgPx
    (1154, 15), // SideCurrency: Currency
];

impl FixMsg {
    /// The messages a parsed message splits into: itself, then each
    /// message it reports beside itself. The one split, run once by every
    /// stream door of the parse - [`FixCodec::parse_line`],
    /// [`FixCodec::parse_lines`], the text-line doors and the batch reader,
    /// so a fill or a quoted side is one message wherever it is read,
    /// and the book reads each once.
    ///
    /// - A trade (`AE`) reporting an execution splits off one execution
    ///   per `NoSides(552)` occurrence: the trade's content
    ///   with that occurrence alone in its group and the side's facts at
    ///   the root, as an `ExecutionReport` states them ([`SIDE_ROOT_TAGS`]);
    ///   its chain the side's own, its stable identifier the first of
    ///   `SideExecID(1427)`, `SideTradeID(1506)`, `SideTradeReportID(1005)`,
    ///   `OrderID(37)`, `ClOrdID(11)` it states, else the occurrence's own
    ///   digest. An occurrence stating no side, or one no side reads, splits
    ///   off an execution of side `UNKN` with a warning; an unreadable
    ///   side is also kept beside the trade as an anomaly, and so is an
    ///   occurrence whose facts make no execution, which splits off none.
    /// - A quote stating no side of its own splits into one sided quote
    ///   per side whose facts it states - `BUYS` with the bid's facts,
    ///   `SELL` with the offer's, each keeping both; [`FixMsg`] reads a
    ///   sided quote's price, quantity and FX parts off its side. A quote
    ///   quoting only a bid is that bid's `BUYS` quote, never an unsided
    ///   entry no book side can hold.
    /// - An order's, a quote's or an execution report's report of an
    ///   execution splits off that execution: the report's content under
    ///   the category `EXEC`, its chain its own - `ExecID(17)` as given,
    ///   else `TradeID(1003)`, else the report's chain and code - so the
    ///   fill is stated by the execution alone. The report is then its
    ///   order's report, `ORDR`, or its quote's, `QUOT`, where it names a
    ///   `QuoteID(117)`, as an execution report of no fill is from its
    ///   parse ([`FixMsg::msgcat`]).
    ///
    /// - A batch - an order list, a mass order, a cross, a mass quote, a
    ///   bid list, a match report ([`MarketDataKind::is_batch`]) - splits
    ///   into one message of its single category per entry
    ///   ([`MarketDataKind::item`]): the batch's content without its entry
    ///   group, the entry's own members at the root, chained by the entry's
    ///   own identifier ([`batch_entries`]). Each is then split as any
    ///   message of its category is, so a mass quote's two-sided entry is a
    ///   `BUYS` and a `SELL` quote. A batch stating its entries in no group
    ///   the crate reads splits into nothing.
    ///
    /// Every message split off reads `FILLED` where it is an execution and
    /// its own state otherwise, has an identity of its own, and names its
    /// source's identity beside its source's sources as its own; the source
    /// keeps what it states, its own state included.
    pub(super) fn split(mut self) -> (Self, Vec<Self>) {
        let category = self.msgcat();
        if category.is_batch() {
            let entries = batch_entries(&self);
            return (self, entries);
        }
        if category == MarketDataKind::Trade && self.header().msgtype() == "AE" {
            if !self.reports_execution() {
                return (self, Vec::new());
            }
            let mut sides = trade_sides(&mut self);
            // By side then chain, whatever order the group stated them in:
            // the order a stream hands them over in is the place each takes
            // at the trade's instant, so it must not be the group's.
            sides.sort_by(|left, right| {
                (left.get_side(), left.get_crosscode())
                    .cmp(&(right.get_side(), right.get_crosscode()))
            });
            return (self, sides);
        }
        if category == MarketDataKind::Quotation {
            let sides = quoted_sides(&self);
            if !sides.is_empty() {
                let sided = sides
                    .into_iter()
                    .filter_map(|side| sided_quote(&self, side))
                    .collect();
                return (self, sided);
            }
        }
        if matches!(
            category,
            MarketDataKind::Order | MarketDataKind::Quotation | MarketDataKind::Execution
        ) && self.reports_execution()
        {
            // A report of a fill is its order's report once the fill is
            // split off: the execution is that fill.
            if category == MarketDataKind::Execution {
                let report = if self.lifted().quoteid().is_some() {
                    MarketDataKind::Quotation
                } else {
                    MarketDataKind::Order
                };
                self.record(
                    super::MARKETDATAKIND_TAG_NAME.0,
                    &Scalar::MarketDataKind(report),
                );
                self.settle();
            }
            let base = self
                .lifted()
                .execid()
                .map(str::to_owned)
                .or_else(|| self.lifted().tradeid().map(|id| format!("TradeID={id}")))
                .unwrap_or_else(|| {
                    format!(
                        "{}|Execution={:016x}",
                        base_crosscode(self.get_crosscode()),
                        self.get_currhashcode()
                    )
                });
            let execution = executed(&self, self.clone(), base);
            return (self, vec![execution]);
        }
        (self, Vec::new())
    }
}

/// `derived` as the execution `source` split off, chained under `base`:
/// the category `EXEC`, `FILLED`, and `source` named as its provenance.
fn executed(source: &FixMsg, mut derived: FixMsg, base: String) -> FixMsg {
    derived.record(
        super::MARKETDATAKIND_TAG_NAME.0,
        &Scalar::MarketDataKind(MarketDataKind::Execution),
    );
    derived.record(super::STATE_TAG_NAME.0, &Scalar::State(State::Filled));
    derived.set_crosscode(base);
    derived.set_srcuuids(provenance(source));
    derived.settle();
    derived
}

/// What a message split off `source` names as its sources: `source`
/// itself, then what `source` was read from.
fn provenance(source: &FixMsg) -> Vec<crate::Uuid> {
    let mut sources = Vec::with_capacity(source.get_srcuuids().len() + 1);
    sources.push(source.get_curruuid());
    sources.extend_from_slice(source.get_srcuuids());
    sources
}

/// The sided quote a quote stating no side splits off for `side`: its content
/// stating that `Side(54)`, so its price, quantity and FX parts are that
/// side's.
fn sided_quote(source: &FixMsg, side: Side) -> Option<FixMsg> {
    let mut quote = source.clone();
    let code = side.fix_code()?.to_string();
    if let Err(error) = quote.set_each([(54, Scalar::from(code))]) {
        warned!(
            "FIX quote side excluded: its Side could not be written",
            "Side",
            "the {} quote of {:?}: {error}",
            side.as_str(),
            source.get_crosscode()
        );
        return None;
    }
    quote.set_srcuuids(provenance(source));
    quote.settle();
    Some(quote)
}

/// The executions a trade splits off, one per `NoSides(552)` occurrence:
/// an occurrence stating no side, or one no side reads, is an execution of
/// side [`Side::Unknown`] - its `Side(54)` unwritten - with a warning, and
/// an unreadable side is also kept beside the trade as an anomaly.
fn trade_sides(trade: &mut FixMsg) -> Vec<FixMsg> {
    let Some(group_at) = trade.index_of_group(TRADE_SIDES) else {
        return Vec::new();
    };
    let Some(group) = trade
        .entries()
        .iter()
        .find(|entry| entry.tag() == TRADE_SIDES)
        .cloned()
    else {
        return Vec::new();
    };
    let Some(name) = trade
        .as_field()
        .fields()
        .get(group_at)
        .map(|field| SmolStr::new(field.name()))
    else {
        return Vec::new();
    };
    let Some(rows) = trade
        .as_value()
        .as_sequence()
        .and_then(|values| values.get(group_at))
        .and_then(Scalar::as_serie)
        .cloned()
    else {
        return Vec::new();
    };
    let counted = trade.get_by_tag(TRADE_SIDES).is_some();
    // A trade is not sided, so its cross code is its chain as given.
    let chain = SmolStr::new(trade.get_crosscode());
    let mut sides = Vec::with_capacity(group.entries().len());
    for (index, occurrence) in group.entries().iter().enumerate() {
        let stated = entry_value(occurrence, 54);
        let readable = stated.is_some_and(|stated| Side::from_spelling(stated).is_some());
        if !readable {
            unsided(trade, &name, index, stated);
        }
        let alone = match rows.slice(index, 1) {
            Ok(alone) => alone,
            Err(error) => {
                excluded_side(trade, &name, index, &error);
                continue;
            }
        };
        let mut execution = trade.clone();
        let mut writes: Vec<(FixKey<'_>, Scalar)> =
            vec![(FixKey::Name(name.as_str()), Scalar::from(alone))];
        for (member, root) in SIDE_ROOT_TAGS {
            if member == 54 && !readable {
                continue;
            }
            if let Some(value) = entry_value(occurrence, member) {
                writes.push((FixKey::Tag(root), side_value(root, value)));
            }
        }
        if counted {
            writes.push((FixKey::Tag(TRADE_SIDES), Scalar::from(1_i32)));
        }
        if let Err(error) = execution.set_each(writes) {
            excluded_side(trade, &name, index, &error);
            continue;
        }
        let stable = [1427, 1506, 1005, 37, 11]
            .into_iter()
            .find_map(|tag| entry_value(occurrence, tag).map(|value| (tag, value)))
            .map(|(tag, value)| format_smolstr!("{tag}:{}:{value}", value.len()))
            .unwrap_or_else(|| {
                let digest = super::digest::digest_of(&[std::slice::from_ref(occurrence)]);
                format_smolstr!("content:{digest:032x}")
            });
        let own = [37, 11, 41]
            .into_iter()
            .find_map(|tag| entry_value(occurrence, tag))
            .unwrap_or(chain.as_str());
        let base = format!("{}:{own}|{stable}", own.len());
        sides.push(executed(trade, execution, base));
    }
    sides
}

/// Says that the `index`th occurrence of a trade's `group` states no side -
/// `stated` the text no side reads, where it states one - so the execution
/// it splits off is of side `UNKN`: as a warning, and a stated text
/// beside the trade as an anomaly too.
fn unsided(trade: &mut FixMsg, group: &str, index: usize, stated: Option<&str>) {
    let Some(stated) = stated else {
        warned!(
            "FIX trade side states no Side; its execution's side defaulted to UNKN",
            "Side",
            "{group}[{index}] of trade {:?}",
            trade.get_crosscode()
        );
        return;
    };
    warned!(
        "FIX trade side's Side is unreadable; its execution's side defaulted to UNKN",
        "Side",
        "{group}[{index}] of trade {:?} states {stated:?}",
        trade.get_crosscode()
    );
    trade.note_anomaly(super::FixAnomaly::new(
        "Side",
        format!("occurrence {index} states {stated:?}, which names no side"),
    ));
}

/// Says that the `index`th occurrence of a trade's `group` splits off no
/// execution, since `error` refused taking it alone: as a warning, and
/// beside the trade as an anomaly, since the fill it states is lost.
fn excluded_side(trade: &mut FixMsg, group: &str, index: usize, error: &Error) {
    warned!(
        "FIX trade side excluded: its occurrence does not make an execution",
        group,
        "{group}[{index}] of trade {:?}: {error}",
        trade.get_crosscode()
    );
    trade.note_anomaly(super::FixAnomaly::new(
        group,
        format!("occurrence {index} splits off no execution: {error}"),
    ));
}

/// A side member's text as the root tag it is written under reads it: a
/// quantity or a price as its decimal, anything else as the text.
fn side_value(root: i32, value: &str) -> Scalar {
    match root {
        32 | 6 => value
            .parse::<Decimal>()
            .map_or_else(|_| Scalar::from(value), Scalar::from),
        _ => Scalar::from(value),
    }
}

fn entry_value(entry: &FixEntry, tag: i32) -> Option<&str> {
    if entry.tag() == tag {
        return entry.value().filter(|value| !value.is_empty());
    }
    entry
        .entries()
        .iter()
        .find_map(|child| entry_value(child, tag))
}

/// What one member of a book entry reads as: its value, `None` where it is
/// not stated, or the value as stated where it reads as nothing.
type Reading<T> = std::result::Result<Option<T>, String>;

/// Said of an entry's size, FX part, or an update's price that is no exact
/// decimal.
const DECIMAL_NULLED: &str = "FIX book entry value is no exact decimal; defaulted to null";

/// Said of an entry's or a message's date or time that names no instant.
const CLOCK_DEFAULTED: &str =
    "FIX book entry date or time names no instant; defaulted to the message's clock";

/// Where the members a book entry is read by sit in its typed occurrence.
struct MemberPaths {
    date: Option<Vec<usize>>,
    time: Option<Vec<usize>>,
    price: Option<Vec<usize>>,
    size: Option<Vec<usize>>,
    spotrate: Option<Vec<usize>>,
    forwardpoints: Option<Vec<usize>>,
}

/// An entry read off the FIX tree alone: no member has a typed path.
static UNTYPED: MemberPaths = MemberPaths {
    date: None,
    time: None,
    price: None,
    size: None,
    spotrate: None,
    forwardpoints: None,
};

/// A book message's typed `NoMDEntries(268)` group: one typed occurrence
/// per entry of the FIX tree, and where an entry's members sit in each.
struct TypedGroup<'a> {
    occurrences: Cow<'a, [Scalar]>,
    paths: MemberPaths,
    /// Where an entry states its own parties and account.
    accounts: AccountsAt,
}

/// The typed group matching the `count` entries of `message`'s FIX tree,
/// or why the row holds none that does.
fn typed_group<'a>(
    message: &'a FixMsg,
    values: Option<&'a [Scalar]>,
    count: usize,
) -> std::result::Result<TypedGroup<'a>, String> {
    let values = values.ok_or("the message row is no record")?;
    let at = message
        .index_of_group(MD_ENTRIES)
        .ok_or("no single typed column holds the group")?;
    let sequence = message
        .as_field()
        .fields()
        .get(at)
        .and_then(|field| field.dtype().as_serie_type())
        .ok_or("the group's typed column is no sequence")?;
    let occurrences = values
        .get(at)
        .and_then(Scalar::sequence_rows)
        .ok_or("the group's typed value is no sequence")?;
    if occurrences.len() != count {
        return Err(format!(
            "the typed group holds {} entries where the FIX tree holds {count}",
            occurrences.len()
        ));
    }
    let members = sequence.item().fields();
    let paths = MemberPaths {
        date: member_path(message, members, 272),
        time: member_path(message, members, 273),
        price: member_path(message, members, 270),
        size: member_path(message, members, 271),
        spotrate: member_path(message, members, 1026),
        forwardpoints: member_path(message, members, 1027),
    };
    let accounts = AccountsAt::new(message.registry(), members);
    Ok(TypedGroup {
        occurrences,
        paths,
        accounts,
    })
}

/// The entries a book message states, each read at the smallest part that
/// can stand: a fact no reading takes is null with a warning, and a
/// message whose group cannot be told apart answers none.
fn book_entries(message: &FixMsg) -> Vec<BookEntry> {
    let msgtype = message.header().msgtype();
    let unix = message.get_currunix();
    let mut root = Facts {
        request_id: message.lifted().mdreqid().map(SmolStr::new),
        ..Facts::default()
    };
    let mut group = None;
    let mut groups = 0_usize;
    for entry in message.entries() {
        if entry.tag() == MD_ENTRIES {
            group = Some(entry);
            groups += 1;
        } else {
            gather(entry, &mut root);
        }
    }
    if groups > 1 {
        warned!(
            "FIX book message excluded: it states several NoMDEntries groups",
            msgtype,
            "the {msgtype:?} message at {unix} states {groups}, and which one is the book's \
             cannot be chosen"
        );
        return Vec::new();
    }

    let values = message.as_value().as_sequence();
    let root_at = |tag: i32, name: &str| {
        format!("{name}({tag}) at the root of the {msgtype:?} message at {unix}")
    };
    let root_date = values.and_then(|values| {
        let value = message
            .unique_index_of_tag(272)
            .and_then(|at| values.get(at));
        or_null(
            temporal_count(value, TimeUnit::Day),
            CLOCK_DEFAULTED,
            272,
            "MDEntryDate",
            &root_at,
        )
    });
    let root_time = values.and_then(|values| {
        let value = message
            .unique_index_of_tag(273)
            .and_then(|at| values.get(at));
        or_null(
            temporal_count(value, TimeUnit::Nanosecond),
            CLOCK_DEFAULTED,
            273,
            "MDEntryTime",
            &root_at,
        )
    });
    let empty_snapshot = || BookEntry {
        facts: root.clone(),
        accounts: Identifiers::new(),
        position: 0,
        date_days: root_date,
        time_nanos: root_time,
        price: Ok(None),
        size: None,
        spotrate: None,
        forwardpoints: None,
        empty_snapshot: true,
    };
    let occurrences = match group {
        Some(group) if group.entries().is_empty() => {
            return if msgtype == "W" {
                vec![empty_snapshot()]
            } else {
                Vec::new()
            };
        }
        Some(group) => group.entries(),
        None if msgtype == "W" => {
            warned!(
                "FIX full refresh states no NoMDEntries group; defaulted to an empty snapshot",
                "NoMDEntries",
                "the {msgtype:?} message at {unix} clears its scope"
            );
            return vec![empty_snapshot()];
        }
        None => {
            warned!(
                "FIX incremental refresh excluded: it states no NoMDEntries group",
                "NoMDEntries",
                "the {msgtype:?} message at {unix} answers no leaf"
            );
            return Vec::new();
        }
    };

    let roles = PartyCodes::new(message.registry());
    let typed = typed_group(message, values, occurrences.len())
        .map_err(|reason| {
            warned!(
                "FIX book entries read off the FIX tree alone: their typed group does not match them",
                "NoMDEntries",
                "the {msgtype:?} message at {unix}: {reason}"
            );
        })
        .ok();
    let mut answer = Vec::with_capacity(occurrences.len());
    for (position, occurrence) in occurrences.iter().enumerate() {
        let place =
            || format!("$.NoMDEntries(268)[{position}] of the {msgtype:?} message at {unix}");
        let at = |tag: i32, name: &str| format!("{name}({tag}) at {}", place());
        let mut facts = Facts::default();
        facts.inherit_context(&root);
        let mut own = Facts::default();
        gather(occurrence, &mut own);
        facts.overlay(own);
        let (members, paths) = typed
            .as_ref()
            .and_then(|typed| {
                let occurrence = &typed.occurrences[position];
                let members = occurrence.as_sequence();
                if members.is_none() {
                    warned!(
                        "FIX book entry read off the FIX tree alone: its typed occurrence is no record",
                        "NoMDEntries",
                        "{} holds {}",
                        place(),
                        occurrence.kind()
                    );
                }
                members.map(|members| (members, &typed.paths))
            })
            .unwrap_or((&[], &UNTYPED));
        let date_days = or_null(
            temporal_count(member_value(members, paths.date.as_deref()), TimeUnit::Day),
            CLOCK_DEFAULTED,
            272,
            "MDEntryDate",
            &at,
        )
        .or(root_date);
        let time_nanos = or_null(
            temporal_count(
                member_value(members, paths.time.as_deref()),
                TimeUnit::Nanosecond,
            ),
            CLOCK_DEFAULTED,
            273,
            "MDEntryTime",
            &at,
        )
        .or(root_time);
        let price = decimal(
            member_value(members, paths.price.as_deref()),
            facts.price.as_deref(),
        );
        let size = or_null(
            decimal(
                member_value(members, paths.size.as_deref()),
                facts.size.as_deref(),
            ),
            DECIMAL_NULLED,
            271,
            "MDEntrySize",
            &at,
        );
        let spotrate = or_null(
            decimal(
                member_value(members, paths.spotrate.as_deref()),
                facts.spotrate.as_deref(),
            ),
            DECIMAL_NULLED,
            1026,
            "MDEntrySpotRate",
            &at,
        );
        let forwardpoints = or_null(
            decimal(
                member_value(members, paths.forwardpoints.as_deref()),
                facts.forwardpoints.as_deref(),
            ),
            DECIMAL_NULLED,
            1027,
            "MDEntryForwardPoints",
            &at,
        );
        let mut accounts = Identifiers::new();
        if let Some(typed) = typed.as_ref() {
            typed.accounts.read(roles, members, &mut accounts);
        }
        answer.push(BookEntry {
            facts,
            accounts,
            position,
            date_days,
            time_nanos,
            price,
            size,
            spotrate,
            forwardpoints,
            empty_snapshot: false,
        });
    }
    answer
}

/// What `read` answers, null where the stated value reads as nothing -
/// with the warning `what` about `name`, naming the value where `at`
/// places the member.
fn or_null<T>(
    read: Reading<T>,
    what: &'static str,
    tag: i32,
    name: &'static str,
    at: &impl Fn(i32, &str) -> String,
) -> Option<T> {
    read.unwrap_or_else(|value| {
        warned!(what, name, "{} states {value}", at(tag, name));
        None
    })
}

fn member_path(message: &FixMsg, fields: &[crate::Field], tag: i32) -> Option<Vec<usize>> {
    for (index, field) in fields.iter().enumerate() {
        if message
            .registry()
            .identity_of(field)
            .is_some_and(|(held, _)| held == tag)
        {
            return Some(vec![index]);
        }
        if matches!(field.dtype(), DataType::Struct(_))
            && let Some(mut nested) = member_path(message, field.fields(), tag)
        {
            nested.insert(0, index);
            return Some(nested);
        }
    }
    None
}

fn member_value<'a>(values: &'a [Scalar], path: Option<&[usize]>) -> Option<&'a Scalar> {
    let (first, nested) = path?.split_first()?;
    let mut value = values.get(*first)?;
    for index in nested {
        value = value.as_sequence()?.get(*index)?;
    }
    Some(value)
}

/// A typed date or time as a count of `unit`.
fn temporal_count(value: Option<&Scalar>, unit: TimeUnit) -> Reading<i64> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    value
        .temporal_count_at(unit)
        .map(Some)
        .ok_or_else(|| format!("{value:?}, which is no exact count of {unit}"))
}

fn gather(entry: &FixEntry, facts: &mut Facts) {
    if let Some(value) = entry.held_value() {
        facts.record(entry.tag(), value);
    }
    for child in entry.entries() {
        gather(child, facts);
    }
}

/// One leaf per entry that can stand, each carrying the message's unmapped
/// fields and its own occurrence's where `unmapped` states them.
fn build_book_operations(
    base: OperationEventFacts,
    msgtype: &str,
    entries: &[BookEntry],
    mut unmapped: Option<Unmapped>,
) -> Vec<MarketData> {
    let mut answer = Vec::with_capacity(entries.len());
    let mut base = Some(base);
    for (index, entry) in entries.iter().enumerate() {
        let event = if index + 1 == entries.len() {
            base.take().expect("the final entry takes the base")
        } else {
            base.as_ref().expect("the base remains").clone()
        };
        let carried = unmapped
            .as_mut()
            .map(|unmapped| unmapped.leaf(entry.position));
        answer.extend(build_book_operation(event, msgtype, entry, carried));
    }
    answer.sort_by_key(effective_unix);
    answer
}

/// The leaf one entry is, or `None` - with a warning - where it cannot
/// stand: no kind, no action, no identity, or no price to rest at.
fn build_book_operation(
    mut event: OperationEventFacts,
    msgtype: &str,
    entry: &BookEntry,
    carried: Option<Carried>,
) -> Option<MarketData> {
    let unix = event.get_currunix();
    let place = || {
        format!(
            "$.NoMDEntries(268)[{}] of the {msgtype:?} message at {unix}",
            entry.position
        )
    };
    let at = |tag: i32, name: &str| format!("{name}({tag}) at {}", place());
    if entry.empty_snapshot {
        let scope = book_scope(&entry.facts, &event);
        let ticker = entry
            .facts
            .symbol
            .as_deref()
            .map(SmolStr::new)
            .or_else(|| event.get_ticker().map(SmolStr::new));
        event.set_ticker(ticker, true);
        let mut crosscode = scope.clone();
        push_scope(&mut crosscode, "BookSnapshot", "empty");
        event.set_crosscode(crosscode);
        event.set_state(State::New);
        carry(&mut event, carried, false);
        // A snapshot control states no operation of its own: the operation
        // facts a FIX event states drop.
        return Some(MarketData::from(SnapshotEvent::from_facts(
            event.into_event(),
            Some(SmolStr::new(scope)),
        )));
    }
    let Some(entry_type) = entry.facts.entry_type.as_deref() else {
        warned!(
            "FIX book entry excluded: it states no MDEntryType",
            "MDEntryType",
            "{} is absent",
            at(269, "MDEntryType")
        );
        return None;
    };
    let kind = match entry_type {
        "0" | "1" if entry.facts.order_id.is_some() => Direct::Order,
        "0" | "1" => Direct::Quote,
        "2" => Direct::Execution,
        other => {
            warned!(
                "FIX book entry excluded: MDEntryType is not a bid, an offer or a trade",
                "MDEntryType",
                "{} states {other:?}; a book reads 0 (bid), 1 (offer) and 2 (trade)",
                at(269, "MDEntryType")
            );
            return None;
        }
    };
    let action = if msgtype == "W" {
        MdUpdateAction::Snapshot
    } else {
        let Some(stated) = entry.facts.action.as_deref() else {
            warned!(
                "FIX book entry excluded: it states no MDUpdateAction",
                "MDUpdateAction",
                "{} is absent",
                at(279, "MDUpdateAction")
            );
            return None;
        };
        let Some(action) =
            MdUpdateAction::read(stated).filter(|action| *action != MdUpdateAction::Snapshot)
        else {
            warned!(
                "FIX book entry excluded: MDUpdateAction is not an incremental action",
                "MDUpdateAction",
                "{} states {stated:?}; an incremental action is 0 through 5",
                at(279, "MDUpdateAction")
            );
            return None;
        };
        action
    };
    let rests = matches!(action, MdUpdateAction::Snapshot | MdUpdateAction::New);
    let price = match &entry.price {
        Ok(price) => *price,
        Err(value) if rests => {
            warned!(
                "FIX book entry excluded: its MDEntryPx is no exact decimal, and a new entry cannot rest unpriced",
                "MDEntryPx",
                "{} states {value}",
                at(270, "MDEntryPx")
            );
            return None;
        }
        // An update restating no price keeps the live entry's.
        Err(value) => {
            warned!(
                DECIMAL_NULLED,
                "MDEntryPx",
                "{} states {value}",
                at(270, "MDEntryPx")
            );
            None
        }
    };
    let state = match action {
        MdUpdateAction::Snapshot | MdUpdateAction::New => State::New,
        MdUpdateAction::Change | MdUpdateAction::Overlay => State::Replaced,
        MdUpdateAction::Delete | MdUpdateAction::DeleteThru | MdUpdateAction::DeleteFrom => {
            State::Canceled
        }
    };

    let side = match (entry_type, entry.facts.side.as_deref()) {
        ("0", _) => Side::Buy,
        ("1", _) => Side::Sell,
        (_, None) => Side::Unknown,
        (_, Some(stated)) => Side::from_spelling(stated).unwrap_or_else(|| {
            warned!(
                "FIX book entry Side is unreadable; defaulted to UNKN",
                "Side",
                "{} states {stated:?}",
                at(54, "Side")
            );
            Side::Unknown
        }),
    };
    if let Some(instant) = entry_unix(entry, unix, &at) {
        if msgtype == "W" {
            if matches!(kind, Direct::Execution) {
                event.set_execunix(Some(instant), true);
            } else {
                event.set_creaunix(Some(instant));
            }
        } else {
            event.set_currunix(instant);
        }
        if msgtype != "W" && matches!(kind, Direct::Execution) {
            event.set_execunix(Some(instant), true);
        }
    }
    let scope = book_scope(&entry.facts, &event);
    let crosscode = if let Some(identifier) = entry
        .facts
        .entry_id
        .as_deref()
        .or(entry.facts.entry_ref_id.as_deref())
    {
        let mut crosscode = scope.clone();
        push_scope(&mut crosscode, "MDEntryID", identifier);
        crosscode
    } else if let Some(crosscode) = fallback_crosscode(&scope, entry_type, action, &entry.facts) {
        crosscode
    } else {
        warned!(
            "FIX book entry excluded: an anonymous incremental update names no entry",
            "MDEntryID",
            "{} is absent, and so are MDEntryRefID, MDEntryPositionNo and MDPriceLevel for \
             action {}",
            at(278, "MDEntryID"),
            action.as_str()
        );
        return None;
    };

    // The entry's MDEntryPx(270), MDEntrySpotRate(1026) and
    // MDEntryForwardPoints(1027), each as stated.
    event.set_price(price, true);
    event.set_quantity(entry.size, true);
    event.set_spotrate(entry.spotrate, true);
    event.set_forwardpoints(entry.forwardpoints, true);
    event.set_side(side, true);
    event.set_state(state);
    let ticker = entry
        .facts
        .symbol
        .as_deref()
        .map(SmolStr::new)
        .or_else(|| event.get_ticker().map(SmolStr::new));
    event.set_ticker(ticker, true);
    event.set_crosscode(crosscode);

    // The entry's own and referenced identifiers and the order it names are
    // the operation's alternate identifiers; the book-control facts ride
    // beside the operation, typed.
    for (key, value) in [
        (ENTRY_ID, entry.facts.entry_id.as_deref()),
        (ENTRY_REF_ID, entry.facts.entry_ref_id.as_deref()),
        (IdType::OrderId, entry.facts.order_id.as_deref()),
    ] {
        if let Some(value) = value {
            // The type's base key, so the entry's own value replaces the
            // type: removing a base key removes every key of its type.
            let _ = event.remove_identifier(&IdKey::base(key.clone()));
            if let Err(error) = Identifier::new(IdKey::base(key.clone()), value)
                .and_then(|id| event.insert_identifier(id))
            {
                warned!(
                    "FIX book entry identifier not kept: the operation's identifiers refuse it",
                    key.as_str(),
                    "{} states {key} {value:?}: {error}",
                    place()
                );
            }
        }
    }
    let position = entry.facts.position.as_deref().and_then(|position| {
        let parsed = position.parse().ok();
        if parsed.is_none() {
            warned!(
                "FIX book entry position is no count; defaulted to null",
                "MDEntryPositionNo",
                "{} states {position:?}",
                at(290, "MDEntryPositionNo")
            );
        }
        parsed
    });
    let book = BookRef {
        action: Some(action),
        scope: Some(SmolStr::new(scope)),
        position,
        entry_px: entry.facts.price.as_ref().and(price),
        entry_size: entry.facts.size.as_ref().and(entry.size),
    };
    // The entry's own parties lead the message's, as a trade side's do.
    if !entry.accounts.is_empty() {
        let mut accounts = entry.accounts.clone();
        accounts.merge(event.get_partyids(), false);
        let _ = event.set_partyids(accounts, true);
    }
    carry(&mut event, carried, true);
    Some(operation(kind, event, Some(book)))
}

/// An entry's decimal: the typed value where the row holds one, else the
/// text the entry states.
fn decimal(typed: Option<&Scalar>, rendered: Option<&str>) -> Reading<Decimal> {
    match typed.filter(|value| !value.is_null()) {
        Some(value) => Decimal::from_scalar(value)
            .map(Some)
            .ok_or_else(|| format!("{value:?}")),
        None => rendered.map_or(Ok(None), |text| {
            text.parse().map(Some).map_err(|_| format!("{text:?}"))
        }),
    }
}

/// The instant an entry's clock names on the day it states - else the
/// message's day - or `None`, with a warning, where no nanosecond count
/// holds it.
fn entry_unix(
    entry: &BookEntry,
    fallback_unix: i64,
    at: &impl Fn(i32, &str) -> String,
) -> Option<i64> {
    let clock = entry.time_nanos?;
    let day = entry
        .date_days
        .unwrap_or_else(|| fallback_unix.div_euclid(NANOS_PER_DAY));
    let unix = i128::from(day)
        .checked_mul(i128::from(NANOS_PER_DAY))
        .and_then(|date| date.checked_add(i128::from(clock)))
        .and_then(|unix| i64::try_from(unix).ok());
    if unix.is_none() {
        warned!(
            CLOCK_DEFAULTED,
            "MDEntryTime",
            "{} states {clock} ns on day {day}, past the nanosecond range",
            at(273, "MDEntryTime")
        );
    }
    unix
}

/// The scope a market-data entry stands in. Its symbol is the entry's own,
/// else the message's ticker, else the instrument's first stated identifier
/// (its ISIN, else its currency pair), else the book the message keys to,
/// [`Market::book_crosscode`]: two instruments stating neither ticker nor
/// identifier under one market and classification share a scope.
fn book_scope<E: Market + ?Sized>(facts: &Facts, event: &E) -> String {
    let ids = event.get_securityids();
    let key;
    let symbol = match facts
        .symbol
        .as_deref()
        .or_else(|| event.get_ticker())
        .or_else(|| ids.get(&IdType::Isin))
        .or_else(|| ids.get(&IdType::Forex))
    {
        Some(symbol) => symbol,
        None => {
            key = event.book_crosscode();
            key.as_ref()
        }
    };
    let mut scope = String::new();
    push_scope(&mut scope, "Symbol", symbol);
    for (name, value) in [
        ("MDBookType", facts.book_type.as_deref()),
        ("MDSubBookType", facts.sub_book_type.as_deref()),
        ("MDFeedType", facts.feed_type.as_deref()),
        ("MDStreamID", facts.stream_id.as_deref()),
        ("MarketID", facts.market_id.as_deref()),
        ("MarketSegmentID", facts.market_segment_id.as_deref()),
        ("MDReqID", facts.request_id.as_deref()),
        ("MarketDepth", facts.depth.as_deref()),
    ] {
        if let Some(value) = value {
            push_scope(&mut scope, name, value);
        }
    }
    scope
}

fn push_scope(scope: &mut String, name: &str, value: &str) {
    if !scope.is_empty() {
        scope.push('|');
    }
    write!(scope, "{name}=").expect("writing into a String cannot fail");
    for character in value.chars() {
        match character {
            '%' => scope.push_str("%25"),
            '|' => scope.push_str("%7C"),
            '=' => scope.push_str("%3D"),
            character => scope.push(character),
        }
    }
}

/// The identity an entry naming no `MDEntryID(278)` or
/// `MDEntryRefID(280)` stands under: its scope, its type and the position
/// or level it states, else - for a new or a snapshot entry, which rests at
/// its price - its price. `None` for an anonymous update stating neither,
/// which names no entry.
fn fallback_crosscode(
    scope: &str,
    entry_type: &str,
    action: MdUpdateAction,
    facts: &Facts,
) -> Option<String> {
    let mut crosscode = String::with_capacity(scope.len() + 64);
    crosscode.push_str(scope);
    push_scope(&mut crosscode, "MDEntryType", entry_type);
    if let Some(position) = facts.position.as_deref() {
        push_scope(&mut crosscode, "MDEntryPositionNo", position);
    }
    if let Some(level) = facts.level.as_deref() {
        push_scope(&mut crosscode, "MDPriceLevel", level);
    }
    if facts.position.is_none() && facts.level.is_none() {
        if !matches!(action, MdUpdateAction::Snapshot | MdUpdateAction::New) {
            return None;
        }
        push_scope(
            &mut crosscode,
            "MDEntryPx",
            facts.price.as_deref().unwrap_or(""),
        );
    }
    Some(crosscode)
}

fn invalid(path: impl Into<SmolStr>, reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: path.into(),
        reason: reason.into(),
    }
}
