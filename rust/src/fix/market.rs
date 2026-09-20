//! The FIX reading of the market's products: one door per product over a
//! stream of messages, one over an Arrow reader of message rows, the one
//! stream of every statement a capture makes, and the message a product
//! states where one does.
//!
//! A [message](FixMsg) is what a venue said; a [product](crate::market)
//! is what the market did. Every door here reads products out of the
//! [chained](FixCodec::lifecycle) stream, so a product reads messages that
//! already know their order and their chain, states every message it read
//! as one of its sources, folds the statements of one product that are
//! one - a message logged at two hops - by the product's own merge, and
//! walks the statements as the lifecycle walks messages, so each carries
//! its predecessor, its place and its lineage. Nothing here is a second
//! model of FIX: what a message states is read through the facts it
//! already answers, and what a product is lives under
//! [`crate::market`], where a second protocol could feed it.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::num::NonZeroU32;
use std::sync::Arc;

use smol_str::SmolStr;

use crate::arrow::BatchReader;
use crate::graph::{Element, Event, MarketElement};
use crate::market::{
    Book, BookData, BookIterator, Execution, ExecutionData, Folded, Order, OrderData, Party,
    Product, Quote, QuoteData, Statement, Trade, TradeData, Walked, arrow_reader,
};
use crate::{DataType, Decimal18, Error, Result, Scalar, State, TimeUnit, Timezone, Uuid};

use super::{FixCodec, FixMsg};

/// The nanoseconds in one day: what a day-only clock counts by.
const NANOS_PER_DAY: i64 = 86_400 * 1_000_000_000;

/// The wire codes of the messages that state an order: a placement, a
/// replace, a cancel, a report against it and a cancel reject.
const ORDER_TYPES: [&str; 5] = ["D", "G", "F", "8", "9"];

/// The wire codes of the messages that report an execution.
const EXECUTION_TYPES: [&str; 1] = ["8"];

/// The wire codes of the messages that report a trade: an execution
/// report and a trade capture report.
const TRADE_TYPES: [&str; 2] = ["8", "AE"];

/// The wire codes of the messages that state a quote: the quote, its
/// status and its cancel.
const QUOTE_TYPES: [&str; 3] = ["S", "AI", "Z"];

/// The names an order goes by, among a message's identifiers.
const ORDER_SCHEMES: [&str; 5] = [
    "clordid",
    "origclordid",
    "orderid",
    "secondaryorderid",
    "secondaryclordid",
];

/// The names an execution goes by: its own, and the order's, which is the
/// chain it stands in; never the match's, which two orders' fills share.
const EXECUTION_SCHEMES: [&str; 6] = [
    "execid",
    "execrefid",
    "secondaryexecid",
    "orderid",
    "clordid",
    "origclordid",
];

/// The names a trade goes by: the match's, the report's and the
/// execution's; never the order's, which every trade of one order shares
/// and which would chain them as one.
const TRADE_SCHEMES: [&str; 5] = [
    "tradereportid",
    "tradeid",
    "secondarytradeid",
    "execid",
    "secondaryexecid",
];

/// The names a quote goes by.
const QUOTE_SCHEMES: [&str; 3] = ["quoteid", "quotereqid", "secondaryquoteid"];

/// The identifiers an order's chain is named by, strongest first: the
/// venue's, else the client identifier a request is about - the original
/// one, so a replace or a cancel names the order it amends and the walk
/// chains it there - else the client's own.
const ORDER_CHAIN: [&str; 3] = ["orderid", "origclordid", "clordid"];

/// The identifiers a trade's chain is named by, strongest first.
const TRADE_CHAIN: [&str; 4] = ["trdmatchid", "tradeid", "tradereportid", "execid"];

/// The identifiers a quote's chain is named by, strongest first.
const QUOTE_CHAIN: [&str; 2] = ["quoteid", "quotereqid"];

/// FIX's own tags for the facts a product reads beside the lifted ones.
const STOPPX_TAG: i32 = 99;
const ORDTYPE_TAG: i32 = 40;
const ORDSTATUS_TAG: i32 = 39;
const EXECTYPE_TAG: i32 = 150;
const TRADEDATE_TAG: i32 = 75;
const SETTLDATE_TAG: i32 = 64;
const TRDMATCHID_TAG: i32 = 880;
const QUOTEMSGID_TAG: i32 = 1166;
const QUOTEENTRYID_TAG: i32 = 299;

/// The wire code a message's type resolves to under the codec's
/// dictionary, or the spelling itself where the dictionary names none.
fn wire_code(codec: &FixCodec, message: &FixMsg) -> SmolStr {
    let spelling = message.header().msgtype();
    codec.registry().get_msgtype(spelling).map_or_else(
        || SmolStr::new(spelling),
        |held| SmolStr::new(held.as_str()),
    )
}

/// The trimmed text one tag states, or nothing for an empty or absent one.
fn word(message: &FixMsg, tag: i32) -> Option<String> {
    message
        .get_by_tag(tag)
        .and_then(|held| held.as_str().map(str::trim).map(str::to_owned))
        .filter(|held| !held.is_empty())
}

/// The exact number one tag states, or nothing for an absent one.
fn number(message: &FixMsg, tag: i32) -> Option<Decimal18> {
    message
        .get_by_tag(tag)
        .as_ref()
        .and_then(Decimal18::from_scalar)
}

/// The day one tag states, as days since the Unix epoch: a day as it
/// stands, a clock by the day it falls in.
fn days(message: &FixMsg, tag: i32) -> Option<i32> {
    let value = message.get_by_tag(tag)?;
    if let Scalar::Date32(held) = &value {
        return Some(held.count());
    }
    let nanos = value.temporal_count_at(TimeUnit::Nanosecond)?;
    i32::try_from(nanos.div_euclid(NANOS_PER_DAY)).ok()
}

/// The state a message states for itself, read off the tags it spells it
/// in - `tags` in the order they are asked - and never off the state the
/// lifecycle folded onto it, which is its conversation's word and not the
/// report's: a product's chain folds its own states forward.
fn stated_state(message: &FixMsg, tags: &[i32]) -> State {
    tags.iter()
        .find_map(|tag| word(message, *tag))
        .and_then(|held| State::read(&held).ok())
        .unwrap_or_else(State::unknown)
}

/// The names a product goes by, taken from the message's own identifiers
/// under the schemes the product's are, then from the tags beside them
/// where the message states one.
fn names(message: &FixMsg, schemes: &[&str], tags: &[(i32, &str)]) -> BTreeMap<String, String> {
    let mut held: BTreeMap<String, String> = message
        .get_identifiers()
        .iter()
        .filter(|(scheme, _)| schemes.contains(&scheme.as_str()))
        .map(|(scheme, name)| (scheme.clone(), name.clone()))
        .collect();
    for (tag, scheme) in tags {
        if let Some(text) = word(message, *tag) {
            held.entry((*scheme).to_owned()).or_insert(text);
        }
    }
    held
}

/// The code the chain is named by: the first stated of `order`, empty
/// where none is.
fn chain(names: &BTreeMap<String, String>, order: &[&str]) -> String {
    order
        .iter()
        .find_map(|scheme| names.get(*scheme).cloned())
        .unwrap_or_default()
}

/// The instrument the message says the product is about: the ticker and
/// the codes the market named it by, each as the message's own reading
/// answers it.
fn instrument<P: MarketElement>(message: &FixMsg, product: &mut P) {
    product.set_symbolticker(message.get_symbolticker().map(str::to_owned));
    product.set_isincode(message.get_isincode().cloned());
    product.set_cusipcode(message.get_cusipcode().cloned());
    product.set_sedolcode(message.get_sedolcode().cloned());
    product.set_bloombergcode(message.get_bloombergcode().cloned());
    product.set_cficode(message.get_cficode().cloned());
    product.set_miccode(message.get_miccode().cloned());
}

/// What the message says the product is priced and counted in.
fn denominated<P: MarketElement>(message: &FixMsg, product: &mut P) {
    product.set_currency(message.get_currency().clone());
    product.set_unit(message.get_unit().to_owned());
}

/// The parties a message states, as its `Parties` group states them: the
/// role, the identifier and its source of every occurrence, each as the
/// text it arrived as or the code the dictionary read it to.
fn parties(message: &FixMsg) -> Vec<Party> {
    let field = message.as_field();
    let Some(at) = field.index_of("parties") else {
        return Vec::new();
    };
    let Some(column) = field.fields().get(at) else {
        return Vec::new();
    };
    let DataType::Sequence(sequence) = column.dtype() else {
        return Vec::new();
    };
    let item = sequence.item();
    let (role, id, source) = (
        item.index_of("partyrole"),
        item.index_of("partyid"),
        item.index_of("partyidsource"),
    );
    let text = |cells: &[Scalar], at: Option<usize>| {
        let held = at.and_then(|at| cells.get(at))?;
        held.as_str()
            .map(str::trim)
            .map(str::to_owned)
            .or_else(|| held.as_i64().map(|code| code.to_string()))
    };
    message
        .as_value()
        .as_sequence()
        .and_then(|row| row.get(at))
        .and_then(Scalar::as_sequence)
        .map(|occurrences| {
            occurrences
                .iter()
                .filter_map(|occurrence| {
                    let cells = occurrence.as_sequence()?;
                    Some(Party {
                        role: text(cells, role).unwrap_or_default(),
                        id: text(cells, id)?,
                        source: text(cells, source).unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The order one message states, where it states one: a placement, a
/// replace, a cancel, a report against the order or a cancel reject.
fn order_of(codec: &FixCodec, message: &FixMsg) -> Option<OrderData> {
    if !ORDER_TYPES.contains(&wire_code(codec, message).as_str()) {
        return None;
    }
    let lifted = message.lifted();
    let mut order = OrderData::at(message.get_currunix());
    order.set_px(lifted.price().unwrap_or(Decimal18::ZERO));
    order.set_stoppx(number(message, STOPPX_TAG));
    order.set_ordtype(word(message, ORDTYPE_TAG));
    order.set_avgpx(lifted.avgpx());
    order.set_qty(
        lifted
            .orderqty()
            .or_else(|| lifted.quantity())
            .unwrap_or(Decimal18::ZERO),
    );
    order.set_cumqty(lifted.cumqty());
    order.set_leavesqty(lifted.leavesqty());
    order.set_side(message.get_side().clone());
    order.set_tif(message.get_tif().map(str::to_owned));
    order.set_tradable(message.get_tradable());
    instrument(message, &mut order);
    denominated(message, &mut order);
    order.set_state(stated_state(message, &[ORDSTATUS_TAG, EXECTYPE_TAG]));
    order.set_creaunix(message.get_creaunix());
    order.set_expirunix(message.get_expirunix());
    let names = names(message, &ORDER_SCHEMES, &[]);
    order.set_crosscode(chain(&names, &ORDER_CHAIN));
    order.set_identifiers(names);
    order.set_srcuuids(vec![message.get_curruuid()]);
    order.finalize();
    Some(order)
}

/// The fill one message reports, where it reports one: an execution report
/// stating a quantity that traded.
fn execution_of(codec: &FixCodec, message: &FixMsg) -> Option<ExecutionData> {
    if !EXECUTION_TYPES.contains(&wire_code(codec, message).as_str()) {
        return None;
    }
    let lifted = message.lifted();
    let qty = lifted.lastqty().filter(|held| *held > Decimal18::ZERO)?;
    let mut fill = ExecutionData::at(message.get_currunix());
    fill.set_px(lifted.lastpx().unwrap_or(Decimal18::ZERO));
    fill.set_qty(qty);
    fill.set_side(message.get_side().clone());
    instrument(message, &mut fill);
    denominated(message, &mut fill);
    fill.set_state(stated_state(message, &[ORDSTATUS_TAG, EXECTYPE_TAG]));
    fill.set_creaunix(message.get_creaunix());
    let names = names(message, &EXECUTION_SCHEMES, &[]);
    fill.set_crosscode(chain(&names, &ORDER_CHAIN));
    fill.set_identifiers(names);
    fill.set_srcuuids(vec![message.get_curruuid()]);
    fill.finalize();
    Some(fill)
}

/// The trade one message reports, where it reports one: an execution
/// report or a trade capture report stating a quantity that traded.
fn trade_of(codec: &FixCodec, message: &FixMsg) -> Option<TradeData> {
    if !TRADE_TYPES.contains(&wire_code(codec, message).as_str()) {
        return None;
    }
    let lifted = message.lifted();
    let qty = lifted.lastqty().filter(|held| *held > Decimal18::ZERO)?;
    let mut trade = TradeData::at(message.get_currunix());
    trade.set_px(lifted.lastpx().unwrap_or(Decimal18::ZERO));
    trade.set_qty(qty);
    trade.set_side(message.get_side().clone());
    instrument(message, &mut trade);
    denominated(message, &mut trade);
    trade.set_tradedate(days(message, TRADEDATE_TAG));
    trade.set_settldate(days(message, SETTLDATE_TAG));
    trade.set_parties(parties(message));
    // A trade stands where the execution's own type says - traded,
    // corrected, cancelled - never where the order it filled stands: the
    // order's status is the order's fact, and a filled order would end the
    // trade's chain before the other side's report reached it.
    trade.set_state(stated_state(message, &[EXECTYPE_TAG]));
    trade.set_creaunix(message.get_creaunix());
    let names = names(message, &TRADE_SCHEMES, &[(TRDMATCHID_TAG, "trdmatchid")]);
    trade.set_crosscode(chain(&names, &TRADE_CHAIN));
    trade.set_identifiers(names);
    trade.set_srcuuids(vec![message.get_curruuid()]);
    trade.finalize();
    Some(trade)
}

/// The quote one message states, where it states one: the quote, its
/// status report or its cancel.
fn quote_of(codec: &FixCodec, message: &FixMsg) -> Option<QuoteData> {
    let code = wire_code(codec, message);
    if !QUOTE_TYPES.contains(&code.as_str()) {
        return None;
    }
    let mut quote = QuoteData::at(message.get_currunix());
    quote.set_bidpx(message.get_bidpx());
    quote.set_bidqty(message.get_bidqty());
    quote.set_askpx(message.get_askpx());
    quote.set_askqty(message.get_askqty());
    // A lane priced in no currency of its own is priced in the quote's,
    // and counted in its unit.
    let currency =
        Some(message.get_currency().clone()).filter(|held| *held != crate::Currency::none());
    let unit = Some(message.get_unit().to_owned()).filter(|held| !held.is_empty());
    quote.set_bidcurrency(
        message
            .get_bidcurrency()
            .cloned()
            .or_else(|| currency.clone().filter(|_| quote.get_bidpx().is_some())),
    );
    quote.set_askcurrency(
        message
            .get_askcurrency()
            .cloned()
            .or_else(|| currency.filter(|_| quote.get_askpx().is_some())),
    );
    quote.set_bidunit(
        message
            .get_bidunit()
            .map(str::to_owned)
            .or_else(|| unit.clone().filter(|_| quote.get_bidpx().is_some())),
    );
    quote.set_askunit(
        message
            .get_askunit()
            .map(str::to_owned)
            .or_else(|| unit.filter(|_| quote.get_askpx().is_some())),
    );
    quote.set_side(message.get_side().clone());
    instrument(message, &mut quote);
    // A cancel ends the quote; anything else stands where the message
    // says it does.
    quote.set_state(if code == "Z" {
        State::from_spelling("canceled").unwrap_or_else(State::unknown)
    } else {
        stated_state(message, &[ORDSTATUS_TAG, EXECTYPE_TAG])
    });
    quote.set_creaunix(message.get_creaunix());
    quote.set_expirunix(message.get_expirunix());
    let names = names(
        message,
        &QUOTE_SCHEMES,
        &[
            (QUOTEMSGID_TAG, "quotemsgid"),
            (QUOTEENTRYID_TAG, "quoteentryid"),
        ],
    );
    quote.set_crosscode(chain(&names, &QUOTE_CHAIN));
    quote.set_identifiers(names);
    quote.set_srcuuids(vec![message.get_curruuid()]);
    quote.finalize();
    Some(quote)
}

/// The prints a stream of messages reports, by the identity of the
/// message each was read from, waiting for the order statement the same
/// message makes to be walked.
type Prints = HashMap<Uuid, ExecutionData>;

/// Every statement a stream of chained messages makes, in instant order:
/// the orders' walk and the quotes' walk merged by instant, the orders of
/// one instant first, and each print yielded right after the order
/// statement its report makes, in that order's chain.
///
/// A print is the fill an execution report states beside the order it
/// restates, and its chain is that order's: the order statement is walked
/// first, so it carries the chain's cross code whichever identifier the
/// report spelled, and the print takes it - which is what joins the
/// execution table to the order table on `crossuuid`. Prints cannot share
/// the orders' walk, because a print arriving under a live order's
/// identity would take the order's place in it. Two prints of one fill -
/// one report logged at two hops - fold into one naming both.
struct Statements<O, Q> {
    orders: O,
    quotes: Q,
    /// The next statement of each walk, pulled and held until the other's
    /// is known to be later.
    order: Option<Result<OrderData>>,
    quote: Option<Result<QuoteData>>,
    prints: Prints,
    /// The prints claimed by the last order statement, owed after it.
    owed: VecDeque<ExecutionData>,
}

impl<O, Q> Statements<O, Q>
where
    O: Iterator<Item = Result<OrderData>>,
    Q: Iterator<Item = Result<QuoteData>>,
{
    fn new(orders: O, quotes: Q, prints: Prints) -> Self {
        Self {
            orders,
            quotes,
            order: None,
            quote: None,
            prints,
            owed: VecDeque::new(),
        }
    }

    /// Claims the prints the messages an order statement was read from
    /// report, each stated in the order's chain, twins folded.
    fn claim(&mut self, order: &OrderData) {
        let mut claimed: Vec<ExecutionData> = Vec::new();
        for source in order.get_srcuuids() {
            let Some(mut print) = self.prints.remove(source) else {
                continue;
            };
            print.set_crosscode(order.get_crosscode().to_owned());
            print.finalize();
            let identity = print.get_curruuid();
            match claimed
                .iter()
                .position(|held| held.get_curruuid() == identity)
            {
                Some(at) => {
                    if let Some(merged) = claimed[at].clone().merge_with(&print) {
                        claimed[at] = merged;
                    }
                }
                None => claimed.push(print),
            }
        }
        self.owed.extend(claimed);
    }
}

impl<O, Q> Iterator for Statements<O, Q>
where
    O: Iterator<Item = Result<OrderData>>,
    Q: Iterator<Item = Result<QuoteData>>,
{
    type Item = Result<Statement>;

    fn next(&mut self) -> Option<Result<Statement>> {
        if let Some(print) = self.owed.pop_front() {
            return Some(Ok(Statement::Execution(print)));
        }
        if self.order.is_none() {
            self.order = self.orders.next();
        }
        if self.quote.is_none() {
            self.quote = self.quotes.next();
        }
        // An error is yielded where it is met, before any statement.
        if matches!(self.order, Some(Err(_))) {
            return self.order.take().map(|held| held.map(Statement::Order));
        }
        if matches!(self.quote, Some(Err(_))) {
            return self.quote.take().map(|held| held.map(Statement::Quote));
        }
        let order_first = match (&self.order, &self.quote) {
            (None, None) => {
                // The streams are done: a print no order statement claimed
                // stands in its own chain.
                let unclaimed = self.prints.keys().min().copied()?;
                let print = self.prints.remove(&unclaimed)?;
                return Some(Ok(Statement::Execution(print)));
            }
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (Some(Ok(order)), Some(Ok(quote))) => order.get_currunix() <= quote.get_currunix(),
            _ => true,
        };
        if order_first {
            let order = self.order.take()?.ok()?;
            self.claim(&order);
            Some(Ok(Statement::Order(order)))
        } else {
            let quote = self.quote.take()?.ok()?;
            Some(Ok(Statement::Quote(quote)))
        }
    }
}

impl FixCodec {
    /// The statements of one product a stream of messages makes, chained.
    ///
    /// The [lifecycle](Self::lifecycle) first, so every statement is read
    /// off a message that knows its chain; `read` answers the statement a
    /// message makes, where it makes one; the statements of one instant
    /// that are one product fold into one, [`Folded`]; and the one walk
    /// chains the rest, [`Walked`], in the order the lifecycle answers.
    fn products<P, I>(
        &self,
        messages: I,
        read: fn(&Self, &FixMsg) -> Option<P>,
    ) -> impl Iterator<Item = Result<P>> + use<P, I>
    where
        P: Product,
        I: IntoIterator,
        I::Item: Into<Result<FixMsg>>,
    {
        let codec = self.clone();
        let statements = self.lifecycle(messages).filter_map(move |held| match held {
            Ok(message) => read(&codec, &message).map(Ok),
            Err(error) => Some(Err(error)),
        });
        Walked::new(Folded::new(statements), true)
    }

    /// The orders a stream of messages states, chained: one statement per
    /// message that states an order - a placement, a replace, a cancel, a
    /// report against it, a cancel reject - each naming the message it was
    /// read from as its source, the statements of one order in one chain
    /// under the identifier the venue gave it.
    ///
    /// The lifecycle first: the messages are chained before a product is
    /// read, so a statement carries what its message's chain folded
    /// forward. A message logged at two hops states its order twice, and
    /// the two statements fold into one by [`Element::merge_with`],
    /// naming both messages among its sources. The walk then chains the
    /// statements of one order - its predecessor, its place, its parents
    /// and the lifecycle carried forward - exactly as [`Self::lifecycle`]
    /// chains messages; [`Self::orders_arrow_reader`] is the same door
    /// over batches of message rows. Errors move through in the order the
    /// source had them; exhaustion is fused.
    pub fn orders<I>(&self, messages: I) -> impl Iterator<Item = Result<OrderData>> + use<I>
    where
        I: IntoIterator,
        I::Item: Into<Result<FixMsg>>,
    {
        self.products(messages, order_of)
    }

    /// The fills a stream of messages reports, chained: one per execution
    /// report stating a quantity that traded, the narrow door - a fill is
    /// one report - each in the chain of the order it fills.
    ///
    /// Read out of [`Self::statements`], where each print takes its chain
    /// from the order statement the same report makes once that is
    /// walked, so a fill's `crossuuid` is its order's whichever identifier
    /// the report spelled; the fills are then walked as every product is,
    /// each stated as the one after the last fill of its order.
    /// [`Self::executions_arrow_reader`] is the same door over batches of
    /// message rows.
    pub fn executions<I>(&self, messages: I) -> impl Iterator<Item = Result<ExecutionData>> + use<I>
    where
        I: IntoIterator,
        I::Item: Into<Result<FixMsg>>,
    {
        let prints = self.statements(messages).filter_map(|held| match held {
            Ok(Statement::Execution(print)) => Some(Ok(print)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        });
        Walked::new(prints, true)
    }

    /// The trades a stream of messages reports, chained: one per execution
    /// report or trade capture report stating a quantity that traded, the
    /// two sides' reports of one match in one chain under the identifier
    /// the venue matched them by.
    ///
    /// [`Self::orders`] says how the stream is read: the lifecycle first,
    /// twins folded, the walk after. [`Self::trades_arrow_reader`] is the
    /// same door over batches of message rows.
    pub fn trades<I>(&self, messages: I) -> impl Iterator<Item = Result<TradeData>> + use<I>
    where
        I: IntoIterator,
        I::Item: Into<Result<FixMsg>>,
    {
        self.products(messages, trade_of)
    }

    /// The quotes a stream of messages states, chained: one statement per
    /// quote, quote status report or quote cancel, the statements of one
    /// quote in one chain under its identifier, a cancel ending it.
    ///
    /// [`Self::orders`] says how the stream is read: the lifecycle first,
    /// twins folded, the walk after. [`Self::quotes_arrow_reader`] is the
    /// same door over batches of message rows.
    pub fn quotes<I>(&self, messages: I) -> impl Iterator<Item = Result<QuoteData>> + use<I>
    where
        I: IntoIterator,
        I::Item: Into<Result<FixMsg>>,
    {
        self.products(messages, quote_of)
    }

    /// Every statement a stream of messages makes, in instant order: the
    /// orders and the quotes, walked, and each fill in its order's chain.
    ///
    /// The lifecycle first, then two walks over the chained messages - the
    /// orders' and the quotes', which cannot share one, since an order and
    /// a quote spelling one identifier would take each other's place in
    /// it - merged by instant, the orders of one instant first; and each
    /// fill an execution report states yielded right after the order
    /// statement the same report makes, in that order's chain, because the
    /// report restates the order and the walk has resolved its chain by
    /// then. Trades are not among them: a fill is the print, and the trade
    /// of the same report would print it again. The chained messages are
    /// held once, so both walks can read them; a message the lifecycle
    /// refuses is yielded first, in the order met, since it belongs to
    /// neither walk. This is the stream [`Self::books`] reads and the one
    /// [`Self::executions`] takes its fills from.
    pub fn statements<I>(&self, messages: I) -> impl Iterator<Item = Result<Statement>> + use<I>
    where
        I: IntoIterator,
        I::Item: Into<Result<FixMsg>>,
    {
        let mut errors: Vec<Error> = Vec::new();
        let mut chained: Vec<FixMsg> = Vec::new();
        for held in self.lifecycle(messages) {
            match held {
                Ok(message) => chained.push(message),
                Err(error) => errors.push(error),
            }
        }
        let chained: Arc<[FixMsg]> = Arc::from(chained);
        let codec = self.clone();
        let prints: Prints = chained
            .iter()
            .filter_map(|message| {
                execution_of(&codec, message).map(|fill| (message.get_curruuid(), fill))
            })
            .collect();
        let orders = {
            let (codec, chained) = (self.clone(), Arc::clone(&chained));
            Walked::new(
                Folded::new(
                    (0..chained.len()).filter_map(move |at| order_of(&codec, &chained[at]).map(Ok)),
                ),
                true,
            )
        };
        let quotes = {
            let (codec, chained) = (self.clone(), Arc::clone(&chained));
            Walked::new(
                Folded::new(
                    (0..chained.len()).filter_map(move |at| quote_of(&codec, &chained[at]).map(Ok)),
                ),
                true,
            )
        };
        errors
            .into_iter()
            .map(Err)
            .chain(Statements::new(orders, quotes, prints))
    }

    /// The books a stream of messages makes: one per symbol per instant
    /// the symbol was touched at, or per grid step with a grid, chained
    /// per symbol.
    ///
    /// [`BookIterator`] over [`Self::statements`]: the orders and the
    /// quotes rest, the fills print, and the book of every symbol an
    /// instant touched is read once the stream moves past it, to `depth`
    /// levels per side. `snapshot_ns` of zero reads one book per instant;
    /// a positive step is a grid, the book of a step its closing state,
    /// dated at the last instant that moved the symbol and stamped with
    /// the step. Depth is a declaration and so is the grid: a depth of
    /// nothing is no ladder and is refused, and a negative step is no
    /// grid and is refused, rather than read as either.
    /// [`Self::books_arrow_reader`] is the same door over batches of
    /// message rows.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming `depth` for a depth of zero
    /// and `snapshot_ns` for a negative step.
    pub fn books<I>(
        &self,
        messages: I,
        depth: u32,
        snapshot_ns: i64,
    ) -> Result<impl Iterator<Item = Result<BookData>> + use<I>>
    where
        I: IntoIterator,
        I::Item: Into<Result<FixMsg>>,
    {
        let depth = book_depth(depth)?;
        if snapshot_ns < 0 {
            return Err(Error::InvalidRecord {
                path: "snapshot_ns".into(),
                reason: crate::text::expected_got(
                    "a grid step in nanoseconds, or zero for one book per instant",
                    snapshot_ns,
                ),
            });
        }
        Ok(BookIterator::new(self.statements(messages), depth, true).with_snapshot_ns(snapshot_ns))
    }

    /// [`Self::orders`] over a stream of batches of message rows: the rows
    /// read as messages by [`Self::messages`], the orders read out of them
    /// and written as batches of order rows under [`OrderData::field`].
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the order row does not
    /// make an Arrow schema; a row that is not a FIX row and the source
    /// reader's own failure are error batches.
    pub fn orders_arrow_reader(&self, source: BatchReader) -> Result<BatchReader> {
        arrow_reader(
            &OrderData::field()?,
            self.orders(self.messages(source)),
            self.batch_row_size(),
            self.batch_byte_size(),
        )
    }

    /// [`Self::executions`] over a stream of batches of message rows,
    /// written as batches of execution rows under [`ExecutionData::field`].
    ///
    /// # Errors
    ///
    /// Returns what [`Self::orders_arrow_reader`] returns.
    pub fn executions_arrow_reader(&self, source: BatchReader) -> Result<BatchReader> {
        arrow_reader(
            &ExecutionData::field()?,
            self.executions(self.messages(source)),
            self.batch_row_size(),
            self.batch_byte_size(),
        )
    }

    /// [`Self::trades`] over a stream of batches of message rows, written
    /// as batches of trade rows under [`TradeData::field`].
    ///
    /// # Errors
    ///
    /// Returns what [`Self::orders_arrow_reader`] returns.
    pub fn trades_arrow_reader(&self, source: BatchReader) -> Result<BatchReader> {
        arrow_reader(
            &TradeData::field()?,
            self.trades(self.messages(source)),
            self.batch_row_size(),
            self.batch_byte_size(),
        )
    }

    /// [`Self::quotes`] over a stream of batches of message rows, written
    /// as batches of quote rows under [`QuoteData::field`].
    ///
    /// # Errors
    ///
    /// Returns what [`Self::orders_arrow_reader`] returns.
    pub fn quotes_arrow_reader(&self, source: BatchReader) -> Result<BatchReader> {
        arrow_reader(
            &QuoteData::field()?,
            self.quotes(self.messages(source)),
            self.batch_row_size(),
            self.batch_byte_size(),
        )
    }

    /// [`Self::books`] over a stream of batches of message rows, written
    /// as batches of book rows under [`BookData::field`] at `depth`.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::books`] refuses, and what
    /// [`Self::orders_arrow_reader`] returns.
    pub fn books_arrow_reader(
        &self,
        source: BatchReader,
        depth: u32,
        snapshot_ns: i64,
    ) -> Result<BatchReader> {
        arrow_reader(
            &BookData::field(book_depth(depth)?)?,
            self.books(self.messages(source), depth, snapshot_ns)?,
            self.batch_row_size(),
            self.batch_byte_size(),
        )
    }
}

/// The depth a door was asked for, as the non-zero one a book is read to.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] naming `depth` for zero.
fn book_depth(depth: u32) -> Result<NonZeroU32> {
    NonZeroU32::new(depth).ok_or_else(|| Error::InvalidRecord {
        path: "depth".into(),
        reason: "expected a depth of at least one level, got zero: a book is a ladder to a \
                 declared depth, and no ladder is no book"
            .into(),
    })
}

/// One instant as FIX spells a UTC timestamp on the wire.
fn stamp(unix: i64) -> Result<SmolStr> {
    let clock = Scalar::datetime64(unix, TimeUnit::Nanosecond, Timezone::UTC)?;
    super::entry::wire_text(&clock).ok_or_else(|| Error::InvalidRecord {
        path: "currunix".into(),
        reason: crate::text::expected_got("an instant a FIX timestamp spells", unix),
    })
}

/// The pairs a product's instrument states: its ticker, its primary
/// identifier under the source that names it, and its market.
fn instrument_pairs<P: MarketElement>(product: &P, pairs: &mut Vec<(&'static str, SmolStr)>) {
    if let Some(ticker) = product.get_symbolticker() {
        pairs.push(("55", SmolStr::new(ticker)));
    }
    let identifier = product
        .get_isincode()
        .map(|held| ("4", held.as_str()))
        .or_else(|| product.get_cusipcode().map(|held| ("1", held.as_str())))
        .or_else(|| product.get_sedolcode().map(|held| ("2", held.as_str())))
        .or_else(|| product.get_bloombergcode().map(|held| ("A", held.as_str())));
    if let Some((source, held)) = identifier {
        pairs.push(("48", SmolStr::new(held)));
        pairs.push(("22", SmolStr::new_static(source)));
    }
    if let Some(cfi) = product.get_cficode() {
        pairs.push(("461", SmolStr::new(cfi.as_str())));
    }
    if product.get_side().as_str() != "UNKNOWN" {
        pairs.push(("54", SmolStr::new(product.get_side().as_str())));
    }
    if *product.get_currency() != crate::Currency::none() {
        pairs.push(("15", SmolStr::new(product.get_currency().as_str())));
    }
    if !product.get_unit().is_empty() {
        pairs.push(("996", SmolStr::new(product.get_unit())));
    }
}

/// The refusal a product with no message that states it earns: the one
/// sentence on why the crate does not guess.
fn unstated(product: &'static str, reason: &'static str) -> Error {
    Error::InvalidRecord {
        path: product.into(),
        reason: reason.into(),
    }
}

impl FixMsg {
    /// The message a product's pairs state, read under `codec` and naming
    /// the product as its one source.
    fn stated(codec: &FixCodec, pairs: &[(&'static str, SmolStr)], source: Uuid) -> Result<Self> {
        let mut message = codec.parse_pairs(
            pairs
                .iter()
                .map(|(key, value)| (key.as_bytes(), value.as_bytes())),
        )?;
        message.set_srcuuids(vec![source]);
        message.finalize();
        Ok(message)
    }

    /// The new order single an order states: `35=D`, exactly.
    ///
    /// An order is one message where it is a placement, and a placement is
    /// what this states: the order's client identifier as `ClOrdID(11)` -
    /// the name it goes by under that scheme, else the code its chain is
    /// named by - the venue's `OrderID(37)` where known, the instrument's
    /// ticker and primary identifier under its source, its market, its
    /// side, its quantity, its type - the one it states, else what its
    /// price ladder implies, [`Order::pricing`] - its prices, its time in force
    /// and expiry, its currency and unit, and its instant as both
    /// `TransactTime(60)` and `SendingTime(52)`, so nothing reads a clock.
    /// The message names the order as its one source, and is read under
    /// `codec` exactly as a line spelling those pairs would be.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use yggdryl::graph::{Element, MarketElement};
    /// use yggdryl::local::Folder;
    /// use yggdryl::market::OrderData;
    /// use yggdryl::{Decimal18, FixCodec, FixMsg, FixRegistry, Side};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    /// let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    /// let codec = FixCodec::new(registry);
    /// let mut order = OrderData::at(1_700_000_000_000_000_000);
    /// order.set_crosscode("C-1".to_owned());
    /// order.set_symbolticker(Some("AAPL".to_owned()));
    /// order.set_side(Side::read("1")?);
    /// order.set_qty(Decimal18::from_int(100));
    /// order.set_px("82.5".parse()?);
    /// order.finalize();
    /// let message = FixMsg::from_order(&codec, &order)?;
    /// assert_eq!(message.header().msgtype(), "D");
    /// assert_eq!(message.by_tag(11)?.as_str(), Some("C-1"));
    /// assert_eq!(message.by_tag(40)?.as_str(), Some("2"), "a limit order");
    /// assert_eq!(message.get_px(), order.get_px());
    /// assert_eq!(message.get_srcuuids(), [order.get_curruuid()]);
    /// // What the order stated is what the message reads back as.
    /// let again = codec.orders([message]).next().expect("the placement")?;
    /// assert_eq!((again.get_qty(), again.get_side().as_str()), (order.get_qty(), "BUY"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming `clordid` for an order that
    /// goes by no client identifier and names no chain, and the codec's
    /// refusal of the pairs otherwise.
    pub fn from_order(codec: &FixCodec, order: &impl Order) -> Result<Self> {
        let names = order.get_identifiers();
        let clordid = names
            .get("clordid")
            .map(String::as_str)
            .filter(|held| !held.is_empty())
            .or_else(|| Some(order.get_crosscode()).filter(|held| !held.is_empty()))
            .ok_or_else(|| Error::InvalidRecord {
                path: "clordid".into(),
                reason: "expected an order going by a ClOrdID or naming its chain, got neither, \
                         and a new order single states a ClOrdID"
                    .into(),
            })?;
        let mut pairs: Vec<(&'static str, SmolStr)> = vec![
            ("35", SmolStr::new_static("D")),
            ("11", SmolStr::new(clordid)),
        ];
        for (tag, scheme) in [("37", "orderid"), ("41", "origclordid")] {
            if let Some(held) = names.get(scheme).filter(|held| !held.is_empty()) {
                pairs.push((tag, SmolStr::new(held)));
            }
        }
        instrument_pairs(order, &mut pairs);
        if let Some(mic) = order.get_miccode() {
            pairs.push(("207", SmolStr::new(mic.as_str())));
        }
        let (px, stoppx) = (
            Some(order.get_px()).filter(|held| *held != Decimal18::ZERO),
            order.get_stoppx(),
        );
        // The type the order states, else what its prices imply.
        let ordtype = order.get_ordtype().map(SmolStr::new).or_else(|| {
            order
                .pricing()
                .map(|pricing| SmolStr::new_static(pricing.code()))
        });
        if let Some(ordtype) = ordtype {
            pairs.push(("40", ordtype));
        }
        if order.get_qty() != Decimal18::ZERO {
            pairs.push(("38", smol_str::format_smolstr!("{}", order.get_qty())));
        }
        if let Some(px) = px {
            pairs.push(("44", smol_str::format_smolstr!("{px}")));
        }
        if let Some(stoppx) = stoppx {
            pairs.push(("99", smol_str::format_smolstr!("{stoppx}")));
        }
        if let Some(tif) = order.get_tif() {
            pairs.push(("59", SmolStr::new(tif)));
        }
        if let Some(expiry) = order.get_expirunix() {
            pairs.push(("126", stamp(expiry)?));
        }
        let instant = stamp(order.get_currunix())?;
        pairs.push(("60", instant.clone()));
        pairs.push(("52", instant));
        Self::stated(codec, &pairs, order.get_curruuid())
    }

    /// The execution report an execution states: `35=8`, `150=F`, exactly.
    ///
    /// A fill is one report, and this is the report: the execution's own
    /// `ExecID(17)`, the order it fills as `OrderID(37)` and `ClOrdID(11)`
    /// where it goes by them, the venue's match where it names one, the
    /// order's status as far as a fill can say it - open where the
    /// execution's state is live, filled where done, cancelled or rejected
    /// where its state ended so - the instrument, its market as
    /// `LastMkt(30)`, the side, the price and quantity that traded as
    /// `LastPx(31)` and `LastQty(32)`, the currency and unit, and the
    /// instant as `TransactTime(60)` and `SendingTime(52)`. The message
    /// names the execution as its one source.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming `execid` for an execution
    /// that goes by no identifier of its own, and the codec's refusal of
    /// the pairs otherwise.
    pub fn from_execution(codec: &FixCodec, execution: &impl Execution) -> Result<Self> {
        let names = execution.get_identifiers();
        let execid = names
            .get("execid")
            .filter(|held| !held.is_empty())
            .ok_or_else(|| Error::InvalidRecord {
                path: "execid".into(),
                reason: "expected an execution going by an ExecID, got none, and an execution \
                         report states one"
                    .into(),
            })?;
        let mut pairs: Vec<(&'static str, SmolStr)> = vec![
            ("35", SmolStr::new_static("8")),
            ("17", SmolStr::new(execid)),
        ];
        let orderid = names
            .get("orderid")
            .map(String::as_str)
            .filter(|held| !held.is_empty())
            .or_else(|| Some(execution.get_crosscode()).filter(|held| !held.is_empty()));
        if let Some(orderid) = orderid {
            pairs.push(("37", SmolStr::new(orderid)));
        }
        for (tag, scheme) in [("11", "clordid"), ("880", "trdmatchid")] {
            if let Some(held) = names.get(scheme).filter(|held| !held.is_empty()) {
                pairs.push((tag, SmolStr::new(held)));
            }
        }
        pairs.push(("150", SmolStr::new_static("F")));
        let state = execution.get_state();
        let ordstatus = if state.is_done() {
            "2"
        } else if state.is_cancelled() {
            "4"
        } else if state.is_failed() {
            "8"
        } else {
            "1"
        };
        pairs.push(("39", SmolStr::new_static(ordstatus)));
        instrument_pairs(execution, &mut pairs);
        if let Some(mic) = execution.get_miccode() {
            pairs.push(("30", SmolStr::new(mic.as_str())));
        }
        if execution.get_px() != Decimal18::ZERO {
            pairs.push(("31", smol_str::format_smolstr!("{}", execution.get_px())));
        }
        if execution.get_qty() != Decimal18::ZERO {
            pairs.push(("32", smol_str::format_smolstr!("{}", execution.get_qty())));
        }
        let instant = stamp(execution.get_currunix())?;
        pairs.push(("60", instant.clone()));
        pairs.push(("52", instant));
        Self::stated(codec, &pairs, execution.get_curruuid())
    }

    /// Refused: no one message states a quote.
    ///
    /// A quote is a reading of the quote message and its updates, and no
    /// one message states which of them a reading came from, so the crate
    /// does not guess. [`Self::from_order`] and [`Self::from_execution`]
    /// are the two products one message states exactly.
    ///
    /// # Errors
    ///
    /// Always [`Error::InvalidRecord`] naming `quote`, with that sentence.
    pub fn from_quote(_codec: &FixCodec, _quote: &impl Quote) -> Result<Self> {
        Err(unstated(
            "quote",
            "a quote is a reading of the quote message and its updates, and no one message \
             states which of them a reading came from, so the crate does not guess",
        ))
    }

    /// Refused: no one message states a trade.
    ///
    /// A trade is a reading of the executions that matched, and no one
    /// message states a match, so the crate does not guess.
    ///
    /// # Errors
    ///
    /// Always [`Error::InvalidRecord`] naming `trade`, with that sentence.
    pub fn from_trade(_codec: &FixCodec, _trade: &impl Trade) -> Result<Self> {
        Err(unstated(
            "trade",
            "a trade is a reading of the executions that matched, and no one message states a \
             match, so the crate does not guess",
        ))
    }

    /// Refused: no one message states a book.
    ///
    /// A book is a reading of every order and quote resting at an instant,
    /// and no one message states a ladder, so the crate does not guess.
    ///
    /// # Errors
    ///
    /// Always [`Error::InvalidRecord`] naming `book`, with that sentence.
    pub fn from_book(_codec: &FixCodec, _book: &impl Book) -> Result<Self> {
        Err(unstated(
            "book",
            "a book is a reading of every order and quote resting at an instant, and no one \
             message states a ladder, so the crate does not guess",
        ))
    }
}
