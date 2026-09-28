//! The one FIX boundary into typed graph market data.

use std::fmt::Write as _;
use std::iter::FusedIterator;

use smallvec::SmallVec;
use smol_str::{SmolStr, format_smolstr};

use super::identity::{BOOK_ENTRY_TAGS, BOOK_ROOT_TAGS, TRADE_SIDE_TAGS};
use super::msg::{Expanded, Unmapped};
use super::{FixCodec, FixEntry, FixKey, FixMsg};
use crate::arrow::BatchReader;
use crate::graph::book::{ENTRY_ID, ENTRY_REF_ID};
use crate::graph::facts::OperationEventFacts;
use crate::graph::market::unsided_crosscode;
use crate::graph::{
    BookIterator, BookRef, Element, Event, ExecutionKind, Market, MarketData, MdUpdateAction,
    Metadata, Operation, OperationEvent, OperationKind, OrderKind, QuoteKind, SnapshotEvent,
};
use crate::{DataType, Decimal, Error, MarketDataKind, Result, Scalar, Side, State, TimeUnit};

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
    position: usize,
    date_days: Option<i64>,
    time_nanos: Option<i64>,
    price: Option<Decimal>,
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
    /// empty `W` is one scoped snapshot control. The structured entry tree is
    /// derived at most once and then walked once.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming the FIX tag and occurrence for
    /// an unsupported message - a trade among them - update action or
    /// market-data entry type.
    ///
    /// # Metadata
    ///
    /// Every leaf carries, in its [`Market::get_metadata`], what the message
    /// states that no typed column reads: the bridge's namespaced keys, then
    /// every other field under its name as the canonical text it spells. A
    /// group or a component is one key, its name - `parties`,
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
    pub fn market_data(&self) -> Result<Vec<MarketData>> {
        operations(self, self.event().clone())
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
    /// Returns the same typed refusals as [`Self::market_data`].
    pub fn into_market_data(self) -> Result<Vec<MarketData>> {
        Ok(expand_message(self, true)?.into_vec())
    }
}

/// A lazy, fallible projection of sorted FIX messages into graph market
/// operations. Direct messages occupy no intermediate vector; one book
/// message retains only its own expanded entries. Each leaf takes the
/// [place](crate::graph::Event::get_seqnum) its message holds - the parse's,
/// or the lifecycle's where the messages were walked - so a chain's steps at
/// one instant keep the order the walk gave them whatever order a table
/// read them back in, and the entries one book message states share it. The
/// first source, conversion, or ordering error is yielded once and fuses
/// the iterator.
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
                if self.last_unix.is_some_and(|previous| unix < previous) {
                    self.done = true;
                    self.current = None;
                    return Some(Err(invalid(
                        "$.operations",
                        // The same words the book walk refuses an unsorted
                        // stream with, so one mistake reads one way.
                        format_smolstr!(
                            "expected a sorted operation timestamp at or after {}, got {unix}",
                            self.last_unix.expect("the prior timestamp was checked")
                        ),
                    )));
                }
                self.last_unix = Some(unix);
                return Some(Ok(operation));
            }
            self.current = None;
            let message = match self.source.next() {
                Some(message) => match message.into() {
                    Ok(message) => message,
                    Err(error) => {
                        self.done = true;
                        return Some(Err(error));
                    }
                },
                None => {
                    self.done = true;
                    return None;
                }
            };
            match expand_message(message, self.market_metadata) {
                Ok(operations) => self.current = Some(operations),
                Err(error) => {
                    self.done = true;
                    return Some(Err(error));
                }
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
/// fill or side twice.
fn contributes_to_book(message: &FixMsg) -> bool {
    match message.msgcat() {
        MarketDataKind::Order => true,
        MarketDataKind::Quotation => quoted_sides(message).is_empty(),
        MarketDataKind::Execution => message.is_execution(),
        MarketDataKind::Book => matches!(message.header().msgtype(), "W" | "X"),
        _ => false,
    }
}

/// The sides a quote stating no side of its own quotes: `BUY` where it
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
    /// book as the messages their parse split off. Source errors and
    /// invalid admitted messages are never skipped. [`FixMarketIterator`]
    /// and standalone operation conversions remain strict for every input.
    ///
    /// This method does not run a lifecycle implicitly: callers that need
    /// lifecycle enrichment pass [`Self::lifecycle`] as the source. Input is
    /// pulled lazily; conversion and ordering errors follow the completed
    /// book prefix and fuse the returned reader. Each leaf carries its
    /// message's unmapped fields where [`Self::market_metadata`] says so.
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
                Err(error) => Some(Err(error)),
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
    /// messages is what places it. A source error and the refusal of an
    /// admitted message's expansion are kept in source order and yielded
    /// first, before every operation, and a refused message drops only its
    /// own leaves; the iterator is fused.
    pub fn market_data<I>(
        &self,
        messages: I,
    ) -> impl FusedIterator<Item = Result<MarketData>> + Send + 'static
    where
        I: IntoIterator,
        I::Item: Into<Result<FixMsg>>,
    {
        let mut failures = Vec::new();
        let mut operations = Vec::new();
        for message in messages {
            match message.into() {
                Ok(message) if contributes_to_book(&message) => {
                    match expand_message(message, self.market_metadata()) {
                        Ok(expanded) => operations.extend(expanded),
                        Err(error) => failures.push(error),
                    }
                }
                Ok(_) => {}
                Err(error) => failures.push(error),
            }
        }
        operations.sort_by_key(effective_unix);
        failures
            .into_iter()
            .map(Err)
            .chain(operations.into_iter().map(Ok))
    }

    /// [`Self::market_data`] as bounded Arrow batches of
    /// [`MarketData::field`] rows, closing as [`Self::book_arrow_reader`]
    /// closes them.
    ///
    /// The writer yields a failure where it meets it, and every failure is
    /// met before the first operation: one intake error is the reader's
    /// only item, and no row follows it.
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

/// The leaves one message moves into, each carrying its message's unmapped
/// fields where `metadata` says so: the one owner of expansion.
fn expand_message(message: FixMsg, metadata: bool) -> Result<MessageOperations> {
    let category = message.msgcat();
    if let Some(kind) = direct_kind(category, message.is_execution()) {
        let unmapped = metadata.then(|| direct_unmapped(&message));
        let mut facts = OperationEventFacts::from(message);
        carry(&mut facts, unmapped);
        return Ok(MessageOperations::One(Some(operation(kind, facts, None))));
    }
    let msgtype = message.header().msgtype();
    if category != MarketDataKind::Book || !matches!(msgtype, "W" | "X") {
        return Err(unsupported_message(&message));
    }
    let msgtype = SmolStr::new(msgtype);
    let entries = book_entries(&message)?;
    let unmapped = metadata.then(|| message.unmapped(Some(&BOOK_EXPANSION)));
    let operations = build_book_operations(
        OperationEventFacts::from(message),
        &msgtype,
        &entries,
        unmapped,
    )?;
    Ok(MessageOperations::Many(operations.into_iter()))
}

/// What a direct message's leaf carries: the message's unmapped fields,
/// and for the execution a trade's parse split off one side, that side's
/// own members beside them.
fn direct_unmapped(message: &FixMsg) -> Metadata {
    if message.header().msgtype() == "AE" && message.msgcat() == MarketDataKind::Execution {
        return message.unmapped(Some(&TRADE_EXPANSION)).leaf(0);
    }
    message.unmapped(None).message
}

/// States `metadata` on a leaf's facts before the leaf is finalized, where
/// there is any to state.
fn carry(facts: &mut OperationEventFacts, metadata: Option<Metadata>) {
    if let Some(metadata) = metadata {
        facts.set_metadata(Some(metadata));
    }
}

fn effective_unix(input: &MarketData) -> i64 {
    input.as_event().map_or(0, |event| {
        event.get_snapunix().unwrap_or_else(|| event.get_currunix())
    })
}

impl TryFrom<FixMsg> for MarketData {
    type Error = Error;

    fn try_from(message: FixMsg) -> Result<Self> {
        let mut operations = expand_message(message, true)?;
        if operations.len() != 1 {
            return Err(invalid(
                "$.NoMDEntries(268)",
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
fn operations(message: &FixMsg, mut base: OperationEventFacts) -> Result<Vec<MarketData>> {
    let category = message.msgcat();
    if let Some(kind) = direct_kind(category, message.is_execution()) {
        carry(&mut base, Some(direct_unmapped(message)));
        return Ok(vec![operation(kind, base, None)]);
    }
    let msgtype = message.header().msgtype();
    if category != MarketDataKind::Book || !matches!(msgtype, "W" | "X") {
        return Err(unsupported_message(message));
    }
    let entries = book_entries(message)?;
    let unmapped = message.unmapped(Some(&BOOK_EXPANSION));
    build_book_operations(base, msgtype, &entries, Some(unmapped))
}

/// Which operation leaf a message or a market-data entry becomes.
#[derive(Clone, Copy)]
enum Direct {
    Order,
    Quote,
    Execution,
}

fn direct_kind(category: MarketDataKind, is_execution: bool) -> Option<Direct> {
    match category {
        MarketDataKind::Order => Some(Direct::Order),
        MarketDataKind::Quotation => Some(Direct::Quote),
        MarketDataKind::Execution if is_execution => Some(Direct::Execution),
        _ => None,
    }
}

fn unsupported_message(message: &FixMsg) -> Error {
    invalid(
        "$.MsgType(35)",
        format_smolstr!(
            "expected ORDR, QUOT, an actual EXEC, W or X - a trade states its fills as the \
             executions its parse splits off - got {:?} ({})",
            message.header().msgtype(),
            message.msgcat().as_str()
        ),
    )
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
    ///   per `NoSides(552)` occurrence stating a side: the trade's content
    ///   with that occurrence alone in its group and the side's facts at
    ///   the root, as an `ExecutionReport` states them ([`SIDE_ROOT_TAGS`]);
    ///   its chain the side's own, its stable identifier the first of
    ///   `SideExecID(1427)`, `SideTradeID(1506)`, `SideTradeReportID(1005)`,
    ///   `OrderID(37)`, `ClOrdID(11)` it states, else the occurrence's own
    ///   digest. An occurrence stating no side splits nothing and is kept
    ///   beside the trade as an anomaly.
    /// - A quote stating no side of its own splits into one sided quote
    ///   per side whose facts it states - `BUY` with the bid's facts,
    ///   `SELL` with the offer's, each keeping both; [`FixMsg`] reads a
    ///   sided quote's price, quantity and FX parts off its side. A quote
    ///   quoting only a bid is that bid's `BUY` quote, never an unsided
    ///   entry no book side can hold.
    /// - An order's, a quote's or an execution report's report of an
    ///   execution splits off that execution: the report's content under
    ///   the category `EXEC`, its chain its own - `ExecID(17)`, else
    ///   `TradeID(1003)`, else the report's chain and code. An execution
    ///   report is then its order's report, `ORDR` - `QUOT` where it names
    ///   a `QuoteID(117)` - so the fill is stated by the execution alone.
    ///
    /// Every message split off reads `FILLED` where it is an execution and
    /// its own state otherwise, has an identity of its own, and names its
    /// source's identity beside its source's sources as its own; the source
    /// keeps what it states, its own state included.
    pub(super) fn split(mut self) -> (Self, Vec<Self>) {
        let category = self.msgcat();
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
            if category == MarketDataKind::Execution {
                let report = if self.lifted().quoteid().is_some() {
                    MarketDataKind::Quotation
                } else {
                    MarketDataKind::Order
                };
                self.record(super::MSGCAT_TAG_NAME.0, &Scalar::MarketDataKind(report));
                self.settle();
            }
            let base = self
                .lifted()
                .execid()
                .map(|id| format!("ExecID={id}"))
                .or_else(|| self.lifted().tradeid().map(|id| format!("TradeID={id}")))
                .unwrap_or_else(|| {
                    format!(
                        "{}|Execution={:016x}",
                        unsided_crosscode(self.get_crosscode()),
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
        super::MSGCAT_TAG_NAME.0,
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
    quote.set_each([(54, Scalar::from(code))]).ok()?;
    quote.set_srcuuids(provenance(source));
    quote.settle();
    Some(quote)
}

/// The executions a trade splits off, one per `NoSides(552)` occurrence
/// stating a side; an occurrence stating none is kept as an anomaly.
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
    let chain = SmolStr::new(unsided_crosscode(trade.get_crosscode()));
    let mut sides = Vec::with_capacity(group.entries().len());
    for (index, occurrence) in group.entries().iter().enumerate() {
        let side = entry_value(occurrence, 54).and_then(|value| Side::read(value).ok());
        if side.is_none_or(|side| side == Side::Unknown) {
            trade.note_anomaly(super::FixAnomaly::new(
                "NoSides",
                format!("occurrence {index} states no side, so it splits off no execution"),
            ));
            continue;
        }
        let Ok(alone) = rows.slice(index, 1) else {
            continue;
        };
        let mut execution = trade.clone();
        let mut writes: Vec<(FixKey<'_>, Scalar)> =
            vec![(FixKey::Name(name.as_str()), Scalar::from(alone))];
        for (member, root) in SIDE_ROOT_TAGS {
            if let Some(value) = entry_value(occurrence, member) {
                writes.push((FixKey::Tag(root), side_value(root, value)));
            }
        }
        if counted {
            writes.push((FixKey::Tag(TRADE_SIDES), Scalar::from(1_i32)));
        }
        if execution.set_each(writes).is_err() {
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

fn book_entries(message: &FixMsg) -> Result<Vec<BookEntry>> {
    let entries = message.entries();
    let mut root = Facts {
        request_id: message.lifted().mdreqid().map(SmolStr::new),
        ..Facts::default()
    };
    let mut group = None;
    for entry in entries {
        if entry.tag() == MD_ENTRIES {
            if group.replace(entry).is_some() {
                return Err(invalid(
                    "$.NoMDEntries(268)",
                    "expected one repeating group, got multiple",
                ));
            }
        } else {
            gather(entry, &mut root);
        }
    }
    let group = group
        .ok_or_else(|| invalid("$.NoMDEntries(268)", "expected a repeating group, got none"))?;

    let values = message
        .as_value()
        .as_sequence()
        .ok_or_else(|| invalid("$", "expected the FIX message row to be a sequence"))?;
    let group_at = message.index_of_group(MD_ENTRIES).ok_or_else(|| {
        invalid(
            "$.NoMDEntries(268)",
            "expected one typed repeating group, got none or multiple",
        )
    })?;
    let Some(sequence) = message
        .as_field()
        .fields()
        .get(group_at)
        .ok_or_else(|| invalid("$.NoMDEntries(268)", "typed group field is absent"))?
        .dtype()
        .as_serie_type()
    else {
        return Err(invalid(
            "$.NoMDEntries(268)",
            "expected the typed group field to be a sequence",
        ));
    };
    let members = sequence.item().fields();
    let date_at = member_path(message, members, 272);
    let time_at = member_path(message, members, 273);
    let price_at = member_path(message, members, 270);
    let size_at = member_path(message, members, 271);
    let spotrate_at = member_path(message, members, 1026);
    let forwardpoints_at = member_path(message, members, 1027);
    let typed_entries = values
        .get(group_at)
        .and_then(Scalar::sequence_rows)
        .ok_or_else(|| {
            invalid(
                "$.NoMDEntries(268)",
                "expected the typed group value to be a sequence",
            )
        })?;
    if typed_entries.len() != group.entries().len() {
        return Err(invalid(
            "$.NoMDEntries(268)",
            format_smolstr!(
                "typed group has {} entries but the FIX tree has {}",
                typed_entries.len(),
                group.entries().len()
            ),
        ));
    }

    let root_date = root_temporal_count(message, values, 272, TimeUnit::Day)?;
    let root_time = root_temporal_count(message, values, 273, TimeUnit::Nanosecond)?;

    let mut answer = Vec::with_capacity(group.entries().len());
    if group.entries().is_empty() && message.header().msgtype() == "W" {
        answer.push(BookEntry {
            facts: root,
            position: 0,
            date_days: root_date,
            time_nanos: root_time,
            price: None,
            size: None,
            spotrate: None,
            forwardpoints: None,
            empty_snapshot: true,
        });
        return Ok(answer);
    }
    for (position, occurrence) in group.entries().iter().enumerate() {
        let mut facts = Facts::default();
        facts.inherit_context(&root);
        let mut own = Facts::default();
        gather(occurrence, &mut own);
        facts.overlay(own);
        let typed = typed_entries[position].as_sequence().ok_or_else(|| {
            invalid(
                format_smolstr!("$.NoMDEntries(268)[{position}]"),
                "expected the typed occurrence to be a sequence",
            )
        })?;
        let date_days = temporal_count(
            member_value(typed, date_at.as_deref()),
            TimeUnit::Day,
            format_smolstr!("$.NoMDEntries(268)[{position}].MDEntryDate(272)"),
        )?
        .or(root_date);
        let time_nanos = temporal_count(
            member_value(typed, time_at.as_deref()),
            TimeUnit::Nanosecond,
            format_smolstr!("$.NoMDEntries(268)[{position}].MDEntryTime(273)"),
        )?
        .or(root_time);
        let price = decimal(
            member_value(typed, price_at.as_deref()),
            facts.price.as_deref(),
            format_smolstr!("$.NoMDEntries(268)[{position}].MDEntryPx(270)"),
        )?;
        let size = decimal(
            member_value(typed, size_at.as_deref()),
            facts.size.as_deref(),
            format_smolstr!("$.NoMDEntries(268)[{position}].MDEntrySize(271)"),
        )?;
        let spotrate = decimal(
            member_value(typed, spotrate_at.as_deref()),
            facts.spotrate.as_deref(),
            format_smolstr!("$.NoMDEntries(268)[{position}].MDEntrySpotRate(1026)"),
        )?;
        let forwardpoints = decimal(
            member_value(typed, forwardpoints_at.as_deref()),
            facts.forwardpoints.as_deref(),
            format_smolstr!("$.NoMDEntries(268)[{position}].MDEntryForwardPoints(1027)"),
        )?;
        answer.push(BookEntry {
            facts,
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
    Ok(answer)
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
        if matches!(field.dtype(), DataType::Struct(_)) {
            if let Some(mut nested) = member_path(message, field.fields(), tag) {
                nested.insert(0, index);
                return Some(nested);
            }
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

fn root_temporal_count(
    message: &FixMsg,
    values: &[Scalar],
    tag: i32,
    unit: TimeUnit,
) -> Result<Option<i64>> {
    temporal_count(
        message
            .unique_index_of_tag(tag)
            .and_then(|at| values.get(at)),
        unit,
        format_smolstr!(
            "$.{}({tag})",
            if tag == 272 {
                "MDEntryDate"
            } else {
                "MDEntryTime"
            }
        ),
    )
}

fn temporal_count(value: Option<&Scalar>, unit: TimeUnit, path: SmolStr) -> Result<Option<i64>> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    value.temporal_count_at(unit).map(Some).ok_or_else(|| {
        invalid(
            path,
            format_smolstr!(
                "expected a temporal value exactly representable as {unit}, got {value:?}"
            ),
        )
    })
}

fn gather(entry: &FixEntry, facts: &mut Facts) {
    if let Some(value) = entry.held_value() {
        facts.record(entry.tag(), value);
    }
    for child in entry.entries() {
        gather(child, facts);
    }
}

/// One leaf per entry, each carrying the message's unmapped fields and its
/// own occurrence's where `unmapped` states them.
fn build_book_operations(
    base: OperationEventFacts,
    msgtype: &str,
    entries: &[BookEntry],
    mut unmapped: Option<Unmapped>,
) -> Result<Vec<MarketData>> {
    let mut answer = Vec::with_capacity(entries.len());
    let mut base = Some(base);
    for (index, entry) in entries.iter().enumerate() {
        let mut event = if index + 1 == entries.len() {
            base.take().expect("the final entry takes the base")
        } else {
            base.as_ref().expect("the base remains").clone()
        };
        carry(
            &mut event,
            unmapped
                .as_mut()
                .map(|unmapped| unmapped.leaf(entry.position)),
        );
        answer.push(build_book_operation(event, msgtype, entry)?);
    }
    answer.sort_by_key(effective_unix);
    Ok(answer)
}

fn build_book_operation(
    mut event: OperationEventFacts,
    msgtype: &str,
    entry: &BookEntry,
) -> Result<MarketData> {
    let path = |tag: i32, name: &str| {
        format_smolstr!("$.NoMDEntries(268)[{}].{name}({tag})", entry.position)
    };
    if entry.empty_snapshot {
        let scope = book_scope(&entry.facts, &event);
        let ticker = entry
            .facts
            .symbol
            .as_deref()
            .map(SmolStr::new)
            .or_else(|| event.get_ticker().map(SmolStr::new));
        event.set_ticker(ticker);
        let mut crosscode = scope.clone();
        push_scope(&mut crosscode, "BookSnapshot", "empty");
        event.set_crosscode(crosscode);
        event.set_state(State::New);
        // A snapshot control states no operation of its own: the operation
        // facts a FIX event states drop.
        return Ok(MarketData::from(SnapshotEvent::from_facts(
            event.into_event(),
            Some(SmolStr::new(scope)),
        )));
    }
    let entry_type = entry.facts.entry_type.as_deref().ok_or_else(|| {
        invalid(
            path(269, "MDEntryType"),
            "expected 0 (bid), 1 (offer) or 2 (trade), got no value",
        )
    })?;
    let kind = match entry_type {
        "0" | "1" if entry.facts.order_id.is_some() => Direct::Order,
        "0" | "1" => Direct::Quote,
        "2" => Direct::Execution,
        other => {
            return Err(invalid(
                path(269, "MDEntryType"),
                format_smolstr!("expected 0 (bid), 1 (offer) or 2 (trade), got {other:?}"),
            ));
        }
    };
    let action = if msgtype == "W" {
        MdUpdateAction::Snapshot
    } else {
        let stated = entry.facts.action.as_deref().ok_or_else(|| {
            invalid(
                path(279, "MDUpdateAction"),
                "expected an incremental action from 0 through 5, got no value",
            )
        })?;
        MdUpdateAction::read(stated)
            .filter(|action| *action != MdUpdateAction::Snapshot)
            .ok_or_else(|| {
                invalid(
                    path(279, "MDUpdateAction"),
                    format_smolstr!(
                        "expected an incremental action from 0 through 5, got {stated:?}"
                    ),
                )
            })?
    };
    let state = match action {
        MdUpdateAction::Snapshot | MdUpdateAction::New => State::New,
        MdUpdateAction::Change | MdUpdateAction::Overlay => State::Replaced,
        MdUpdateAction::Delete | MdUpdateAction::DeleteThru | MdUpdateAction::DeleteFrom => {
            State::Canceled
        }
    };

    let side = match entry_type {
        "0" => Side::read("Buy").expect("the shipped buy side"),
        "1" => Side::read("Sell").expect("the shipped sell side"),
        _ => entry
            .facts
            .side
            .as_deref()
            .and_then(Side::from_spelling)
            .unwrap_or(Side::Unknown),
    };
    if let Some(unix) = entry_unix(entry, event.get_currunix(), &path)? {
        if msgtype == "W" {
            if matches!(kind, Direct::Execution) {
                event.set_execunix(Some(unix));
            } else {
                event.set_creaunix(Some(unix));
            }
        } else {
            event.set_currunix(unix);
        }
        if msgtype != "W" && matches!(kind, Direct::Execution) {
            event.set_execunix(Some(unix));
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
    } else {
        fallback_crosscode(&scope, entry_type, action.as_str(), &entry.facts, &path)?
    };

    // The entry's MDEntryPx(270), MDEntrySpotRate(1026) and
    // MDEntryForwardPoints(1027), each as stated.
    event.set_price(entry.price);
    event.set_quantity(entry.size);
    event.set_spotrate(entry.spotrate);
    event.set_forwardpoints(entry.forwardpoints);
    event.set_side(side);
    event.set_state(state);
    let ticker = entry
        .facts
        .symbol
        .as_deref()
        .map(SmolStr::new)
        .or_else(|| event.get_ticker().map(SmolStr::new));
    event.set_ticker(ticker);
    event.set_crosscode(crosscode);

    // The entry's own and referenced identifiers and the order it names are
    // the operation's alternate identifiers; the book-control facts ride
    // beside the operation, typed.
    for (key, value) in [
        (ENTRY_ID, entry.facts.entry_id.as_deref()),
        (ENTRY_REF_ID, entry.facts.entry_ref_id.as_deref()),
        ("ORDERID", entry.facts.order_id.as_deref()),
    ] {
        if let Some(value) = value {
            let _ = event.remove_altid(key);
            let _ = event.insert_altid(key, value);
        }
    }
    let book = BookRef {
        action: Some(action),
        scope: Some(SmolStr::new(scope)),
        position: entry
            .facts
            .position
            .as_deref()
            .and_then(|position| position.parse().ok()),
        entry_px: entry.facts.price.as_ref().and(entry.price),
        entry_size: entry.facts.size.as_ref().and(entry.size),
    };
    Ok(operation(kind, event, Some(book)))
}

fn decimal(
    typed: Option<&Scalar>,
    rendered: Option<&str>,
    path: SmolStr,
) -> Result<Option<Decimal>> {
    let Some(value) = typed.filter(|value| !value.is_null()) else {
        return rendered
            .map(|value| {
                value.parse::<Decimal>().map_err(|_| {
                    invalid(
                        path.clone(),
                        format_smolstr!("expected an exact decimal, got {value:?}"),
                    )
                })
            })
            .transpose();
    };
    Decimal::from_scalar(value).map(Some).ok_or_else(|| {
        invalid(
            path,
            format_smolstr!("expected an exact decimal, got {value:?}"),
        )
    })
}

fn entry_unix(
    entry: &BookEntry,
    fallback_unix: i64,
    path: &impl Fn(i32, &str) -> SmolStr,
) -> Result<Option<i64>> {
    let Some(clock) = entry.time_nanos else {
        return Ok(None);
    };
    let day = entry
        .date_days
        .unwrap_or_else(|| fallback_unix.div_euclid(NANOS_PER_DAY));
    let unix = i128::from(day)
        .checked_mul(i128::from(NANOS_PER_DAY))
        .and_then(|date| date.checked_add(i128::from(clock)))
        .and_then(|unix| i64::try_from(unix).ok())
        .ok_or_else(|| {
            invalid(
                path(273, "MDEntryTime"),
                "market-data entry instant exceeds nanosecond range",
            )
        })?;
    Ok(Some(unix))
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
        .or_else(|| ids.get("ISIN"))
        .or_else(|| ids.get("FOREX"))
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

fn fallback_crosscode(
    scope: &str,
    entry_type: &str,
    action: &str,
    facts: &Facts,
    path: &impl Fn(i32, &str) -> SmolStr,
) -> Result<String> {
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
        if !matches!(action, "snapshot" | "0") {
            return Err(invalid(
                path(278, "MDEntryID"),
                "expected MDEntryID, MDEntryRefID, MDEntryPositionNo or MDPriceLevel for an anonymous incremental update",
            ));
        }
        push_scope(
            &mut crosscode,
            "MDEntryPx",
            facts.price.as_deref().unwrap_or(""),
        );
    }
    Ok(crosscode)
}

fn invalid(path: impl Into<SmolStr>, reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: path.into(),
        reason: reason.into(),
    }
}
