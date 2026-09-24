//! The one FIX boundary into typed graph market operations.

use std::fmt::Write as _;
use std::iter::FusedIterator;

use smol_str::{SmolStr, format_smolstr};

use super::{FixCodec, FixEntry, FixMsg};
use crate::arrow::BatchReader;
use crate::graph::book::{ENTRY_ID, ENTRY_REF_ID};
use crate::graph::{
    Book, BookControl, BookInput, BookIterator, BookRef, Element, Event, Market, MarketOperation,
    MarketOperationEventData, MdUpdateAction, Operation, OperationKind, Trade,
};
use crate::{Ccy, DataType, Decimal18, Error, Result, Scalar, Side, State, TimeUnit};

const MD_ENTRIES: i32 = 268;
const TRADE_SIDES: i32 = 552;
const NANOS_PER_DAY: i64 = 86_400_000_000_000;

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
    report_sequence: Option<SmolStr>,
    application_id: Option<SmolStr>,
    application_sequence: Option<SmolStr>,
    application_begin_sequence: Option<SmolStr>,
    application_end_sequence: Option<SmolStr>,
    application_last_sequence: Option<SmolStr>,
}

impl Facts {
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
            83 => &mut self.report_sequence,
            1180 => &mut self.application_id,
            1181 => &mut self.application_sequence,
            1182 => &mut self.application_begin_sequence,
            1183 => &mut self.application_end_sequence,
            1350 => &mut self.application_last_sequence,
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
            report_sequence,
            application_id,
            application_sequence,
            application_begin_sequence,
            application_end_sequence,
            application_last_sequence,
        );
    }

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
        self.report_sequence.clone_from(&root.report_sequence);
        self.application_id.clone_from(&root.application_id);
        self.application_sequence
            .clone_from(&root.application_sequence);
        self.application_begin_sequence
            .clone_from(&root.application_begin_sequence);
        self.application_end_sequence
            .clone_from(&root.application_end_sequence);
        self.application_last_sequence
            .clone_from(&root.application_last_sequence);
    }
}

struct BookEntry {
    facts: Facts,
    position: usize,
    date_days: Option<i64>,
    time_nanos: Option<i64>,
    price: Option<Decimal18>,
    size: Option<Decimal18>,
    /// `MDEntrySpotRate(1026)` and `MDEntryForwardPoints(1027)`: the FX
    /// parts of the level's price, read onto its lane.
    spotrate: Option<Decimal18>,
    forwardpoints: Option<Decimal18>,
    empty_snapshot: bool,
}

impl FixMsg {
    /// Reads this message as graph market operations.
    ///
    /// Order, quote, execution and initial `AE` trade reports are one
    /// operation. A FIX
    /// `W` or `X` book message is expanded in nondecreasing effective time,
    /// stably retaining `NoMDEntries(268)` source order for equal instants; an
    /// empty `W` is one scoped snapshot control. The structured entry tree is
    /// derived at most once and then walked once.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming the FIX tag and occurrence for
    /// an unsupported message, update action or market-data entry type; a
    /// missing, repeated or count-mismatched `NoSides(552)` group; a missing
    /// or non-bid/ask `Side(54)`; an invalid side decimal or currency; or a
    /// sided execution that violates the composite trade invariants.
    pub fn market_operations(&self) -> Result<Vec<BookInput>> {
        operations(self, self.event().clone())
    }

    /// Moves this message into graph market operations.
    ///
    /// A direct order, quote, execution or trade moves its
    /// [`MarketOperationEventData`] without cloning it, then finalizes the generic
    /// operation identity from those projected facts. Book messages
    /// necessarily make one owned event per `NoMDEntries(268)` occurrence, or
    /// one scoped snapshot control for an empty `W`.
    ///
    /// # Errors
    ///
    /// Returns the same typed refusals as [`Self::market_operations`].
    pub fn into_market_operations(self) -> Result<Vec<BookInput>> {
        Ok(expand_message(self)?.into_vec())
    }
}

/// A lazy, fallible projection of sorted FIX messages into graph market
/// operations. Direct messages occupy no intermediate vector; one book
/// message retains only its own expanded entries. The first source,
/// conversion, or ordering error is yielded once and fuses the iterator.
pub struct FixMarketIterator<I>
where
    I: Iterator,
    I::Item: Into<Result<FixMsg>>,
{
    source: I,
    current: Option<MessageOperations>,
    last_unix: Option<i64>,
    done: bool,
}

impl<I> FixMarketIterator<I>
where
    I: Iterator,
    I::Item: Into<Result<FixMsg>>,
{
    /// Opens a projection over messages already sorted by event time.
    #[must_use]
    pub fn new(source: I) -> Self {
        Self {
            source,
            current: None,
            last_unix: None,
            done: false,
        }
    }
}

impl<I> Iterator for FixMarketIterator<I>
where
    I: Iterator,
    I::Item: Into<Result<FixMsg>>,
{
    type Item = Result<BookInput>;

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
                        format_smolstr!(
                            "expected nondecreasing effective timestamps, got {unix} after {}",
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
            match expand_message(message) {
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

fn contributes_to_book(message: &FixMsg) -> bool {
    match message
        .get_marketoperationid()
        .and_then(super::constants::msgcat_name)
    {
        Some("ORDR" | "QUOT") => true,
        Some("EXEC") => message.is_execution(),
        Some("BOOK") => matches!(message.header().msgtype(), "W" | "X"),
        Some("TRAD") => message.header().msgtype() == "AE",
        _ => false,
    }
}

impl FixCodec {
    /// Streams sorted FIX messages through their graph market operations and
    /// the stateful book iterator into bounded nested Arrow batches.
    ///
    /// Records outside order/quote categories, actual executions, `W`/`X`
    /// book messages and `AE` trade reports are ignored. Every `AE` reaches
    /// the strict market projection, so unsupported corrections, cancels
    /// and status reports retain their named refusals. Source errors and
    /// invalid admitted messages are never skipped. [`FixMarketIterator`]
    /// and standalone operation conversions remain strict for every input.
    ///
    /// This method does not run a lifecycle implicitly: callers that need
    /// lifecycle enrichment pass [`Self::lifecycle`] as the source. Input is
    /// pulled lazily; conversion and ordering errors follow the completed
    /// book prefix and fuse the returned reader.
    pub fn book_arrow_reader<I>(
        &self,
        messages: I,
        snapshot_millis: u64,
        global: bool,
    ) -> Result<BatchReader>
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
        let operations = FixMarketIterator::new(admitted);
        let books = BookIterator::new(operations, snapshot_millis, global)?;
        Book::arrow_reader(
            books,
            Some(self.batch_row_size()),
            Some(self.batch_byte_size()),
        )
    }
}

// The direct-message hot path stays inline so one order, quote, execution or
// trade does not pay a heap allocation merely to satisfy the iterator shape.
#[allow(clippy::large_enum_variant)]
enum MessageOperations {
    One(Option<BookInput>),
    Many(std::vec::IntoIter<BookInput>),
}

impl MessageOperations {
    /// Recovers the expanded allocation instead of collecting it through the
    /// type-erased iterator path.
    fn into_vec(self) -> Vec<BookInput> {
        match self {
            Self::One(operation) => operation.into_iter().collect(),
            Self::Many(operations) => operations.collect(),
        }
    }
}

impl Iterator for MessageOperations {
    type Item = BookInput;

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

fn expand_message(message: FixMsg) -> Result<MessageOperations> {
    let category = category(&message)?;
    if category == "TRAD" && message.header().msgtype() == "AE" && message.reports_execution() {
        let executions = trade_executions(&message, message.event())?;
        let trade = Trade::from_parts(MarketOperationEventData::from(message), executions)?;
        return Ok(MessageOperations::One(Some(trade.into())));
    }
    if let Some(kind) = direct_kind(category, message.is_execution()) {
        return Ok(MessageOperations::One(Some(
            operation(kind, MarketOperationEventData::from(message)).into(),
        )));
    }
    let msgtype = message.header().msgtype();
    if category != "BOOK" || !matches!(msgtype, "W" | "X") {
        return Err(unsupported_message(&message));
    }
    let msgtype = SmolStr::new(msgtype);
    let entries = book_entries(&message)?;
    let operations =
        build_book_operations(MarketOperationEventData::from(message), &msgtype, &entries)?;
    Ok(MessageOperations::Many(operations.into_iter()))
}

fn effective_unix(input: &BookInput) -> i64 {
    input
        .event()
        .get_snapunix()
        .unwrap_or_else(|| input.currunix())
}

impl TryFrom<FixMsg> for BookInput {
    type Error = Error;

    fn try_from(message: FixMsg) -> Result<Self> {
        let mut operations = expand_message(message)?;
        if operations.len() != 1 {
            return Err(invalid(
                "$.NoMDEntries(268)",
                format_smolstr!(
                    "expected exactly one market operation, got {}",
                    operations.len()
                ),
            ));
        }
        Ok(operations.next().expect("one operation"))
    }
}

fn operations(message: &FixMsg, base: MarketOperationEventData) -> Result<Vec<BookInput>> {
    let category = category(message)?;
    if category == "TRAD" && message.header().msgtype() == "AE" && message.reports_execution() {
        let executions = trade_executions(message, &base)?;
        return Trade::from_parts(base, executions).map(|trade| vec![trade.into()]);
    }
    if let Some(kind) = direct_kind(category, message.is_execution()) {
        return Ok(vec![operation(kind, base).into()]);
    }
    let msgtype = message.header().msgtype();
    if category != "BOOK" || !matches!(msgtype, "W" | "X") {
        return Err(unsupported_message(message));
    }
    let entries = book_entries(message)?;
    build_book_operations(base, msgtype, &entries)
}

fn category(message: &FixMsg) -> Result<&str> {
    message
        .get_marketoperationid()
        .and_then(super::constants::msgcat_name)
        .ok_or_else(|| {
            invalid(
                "$.MsgType(35)",
                format_smolstr!(
                    "expected a market message category, got {:?}",
                    message.header().msgtype()
                ),
            )
        })
}

fn direct_kind(category: &str, is_execution: bool) -> Option<OperationKind> {
    match category {
        "ORDR" => Some(OperationKind::Order),
        "QUOT" => Some(OperationKind::Quote),
        "EXEC" if is_execution => Some(OperationKind::Execution),
        _ => None,
    }
}

fn unsupported_message(message: &FixMsg) -> Error {
    invalid(
        "$.MsgType(35)",
        format_smolstr!(
            "expected ORDR, QUOT, an actual EXEC/TRAD, W or X, got {:?} ({})",
            message.header().msgtype(),
            message
                .get_marketoperationid()
                .and_then(super::constants::msgcat_name)
                .unwrap_or("UNKN")
        ),
    )
}

fn operation(kind: OperationKind, data: MarketOperationEventData) -> Operation {
    let mut operation = Operation::new(kind, data).expect("an order, a quote or an execution");
    operation.finalize();
    operation
}

fn trade_executions(message: &FixMsg, base: &MarketOperationEventData) -> Result<Vec<Operation>> {
    let mut groups = message
        .entries()
        .iter()
        .filter(|entry| entry.tag() == TRADE_SIDES);
    let group = groups.next().ok_or_else(|| {
        invalid(
            "$.NoSides(552)",
            "expected an actual trade to state its sided executions",
        )
    })?;
    if groups.next().is_some() {
        return Err(invalid(
            "$.NoSides(552)",
            "expected one repeating group, got multiple",
        ));
    }
    let stated = group
        .value()
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| invalid("$.NoSides(552)", "expected a non-negative group count"))?;
    if stated != group.entries().len() {
        return Err(invalid(
            "$.NoSides(552)",
            format_smolstr!(
                "expected {stated} occurrences, got {}",
                group.entries().len()
            ),
        ));
    }
    if stated == 0 {
        return Err(invalid(
            "$.NoSides(552)",
            "expected at least one sided execution, got none",
        ));
    }

    group
        .entries()
        .iter()
        .enumerate()
        .map(|(index, occurrence)| trade_execution(base, occurrence, index))
        .collect()
}

fn trade_execution(
    base: &MarketOperationEventData,
    occurrence: &FixEntry,
    index: usize,
) -> Result<Operation> {
    let path = |tag: i32, name: &str| format_smolstr!("$.NoSides(552)[{index}].{name}({tag})");
    let raw_side = entry_value(occurrence, 54)
        .ok_or_else(|| invalid(path(54, "Side"), "expected a bid or ask side, got no value"))?;
    let side = Side::read(raw_side).map_err(|_| {
        invalid(
            path(54, "Side"),
            format_smolstr!("expected a bid or ask side, got {raw_side:?}"),
        )
    })?;
    if !side.is_bid() && !side.is_ask() {
        return Err(invalid(
            path(54, "Side"),
            format_smolstr!("expected a bid or ask side, got {:?}", side.as_str()),
        ));
    }

    let mut event = base.clone();
    event.set_side(side);
    // A side's last executed quantity and average price are those facts
    // and nothing more: neither stands in for the price or the quantity the
    // side states.
    if let Some(value) = entry_decimal(occurrence, 1009, path(1009, "SideLastQty"))? {
        event.set_lastqty(Some(value));
    }
    if let Some(value) = entry_decimal(occurrence, 1852, path(1852, "SideAvgPx"))? {
        event.set_avgpx(Some(value));
    }
    if let Some(value) = entry_value(occurrence, 1154) {
        event.set_currency(Ccy::new(value).map_err(|_| {
            invalid(
                path(1154, "SideCurrency"),
                format_smolstr!("expected a currency code, got {value:?}"),
            )
        })?);
    }

    for (tag, key) in [
        (1427, "SIDEEXECID"),
        (1005, "SIDETRADEREPORTID"),
        (1506, "SIDETRADEID"),
        (1507, "SIDEORIGTRADEID"),
        (37, "ORDERID"),
        (198, "SECONDARYORDERID"),
        (11, "CLORDID"),
        (526, "SECONDARYCLORDID"),
        (41, "ORIGCLORDID"),
    ] {
        if let Some(value) = entry_value(occurrence, tag) {
            let _ = event.remove_altid(key);
            let _ = event.insert_altid(key, value);
        }
    }

    let stable = [1427, 1506, 1005, 37, 11]
        .into_iter()
        .find_map(|tag| entry_value(occurrence, tag).map(|value| (tag, value)))
        .map(|(tag, value)| format_smolstr!("{tag}:{}:{value}", value.len()))
        .unwrap_or_else(|| {
            let digest = super::digest::digest_of(&[std::slice::from_ref(occurrence)]);
            format_smolstr!("content:{digest:032x}")
        });
    let chain = [37, 11, 41]
        .into_iter()
        .find_map(|tag| entry_value(occurrence, tag))
        .unwrap_or_else(|| base.get_crosscode());
    event.set_crosscode(format!(
        "{}:{chain}|TradeSide={}|{stable}",
        chain.len(),
        side.as_str(),
    ));
    Ok(Operation::execution(event))
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

fn entry_decimal(entry: &FixEntry, tag: i32, path: SmolStr) -> Result<Option<Decimal18>> {
    entry_value(entry, tag)
        .map(|value| {
            value.parse::<Decimal18>().map_err(|_| {
                invalid(
                    path,
                    format_smolstr!("expected an exact decimal, got {value:?}"),
                )
            })
        })
        .transpose()
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

fn build_book_operations(
    base: MarketOperationEventData,
    msgtype: &str,
    entries: &[BookEntry],
) -> Result<Vec<BookInput>> {
    let mut answer = Vec::with_capacity(entries.len());
    let mut base = Some(base);
    for (index, entry) in entries.iter().enumerate() {
        let event = if index + 1 == entries.len() {
            base.take().expect("the final entry takes the base")
        } else {
            base.as_ref().expect("the base remains").clone()
        };
        answer.push(build_book_operation(event, msgtype, entry)?);
    }
    answer.sort_by_key(effective_unix);
    Ok(answer)
}

fn build_book_operation(
    mut event: MarketOperationEventData,
    msgtype: &str,
    entry: &BookEntry,
) -> Result<BookInput> {
    let path = |tag: i32, name: &str| {
        format_smolstr!("$.NoMDEntries(268)[{}].{name}({tag})", entry.position)
    };
    if entry.empty_snapshot {
        let scope = book_scope(&entry.facts, event.get_ticker());
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
        event.set_state(State::read("New").expect("the shipped new state"));
        let mut control = event.into_event();
        control.finalize();
        return Ok(BookInput::Snapshot(BookControl::snapshot(
            control,
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
        "0" | "1" if entry.facts.order_id.is_some() => OperationKind::Order,
        "0" | "1" => OperationKind::Quote,
        "2" => OperationKind::Execution,
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
        MdUpdateAction::Snapshot | MdUpdateAction::New => {
            State::read("New").expect("the shipped new state")
        }
        MdUpdateAction::Change | MdUpdateAction::Overlay => {
            State::read("Replaced").expect("the shipped replaced state")
        }
        MdUpdateAction::Delete | MdUpdateAction::DeleteThru | MdUpdateAction::DeleteFrom => {
            State::read("Canceled").expect("the shipped canceled state")
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
            if matches!(kind, OperationKind::Execution) {
                event.set_execunix(Some(unix));
            } else {
                event.set_creaunix(Some(unix));
            }
        } else {
            event.set_currunix(unix);
        }
        if msgtype != "W" && matches!(kind, OperationKind::Execution) {
            event.set_execunix(Some(unix));
        }
    }
    let scope = book_scope(&entry.facts, event.get_ticker());
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
    let mut operation = Operation::new(kind, event).expect("an order, a quote or an execution");
    operation.set_book(Some(book));
    operation.finalize();
    Ok(BookInput::Operation(operation))
}

fn decimal(
    typed: Option<&Scalar>,
    rendered: Option<&str>,
    path: SmolStr,
) -> Result<Option<Decimal18>> {
    let Some(value) = typed.filter(|value| !value.is_null()) else {
        return rendered
            .map(|value| {
                value.parse::<Decimal18>().map_err(|_| {
                    invalid(
                        path.clone(),
                        format_smolstr!("expected an exact decimal, got {value:?}"),
                    )
                })
            })
            .transpose();
    };
    Decimal18::from_scalar(value).map(Some).ok_or_else(|| {
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

fn book_scope(facts: &Facts, fallback_symbol: Option<&str>) -> String {
    let mut scope = String::new();
    push_scope(
        &mut scope,
        "Symbol",
        facts
            .symbol
            .as_deref()
            .or(fallback_symbol)
            .unwrap_or("GLOBAL"),
    );
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
