//! `rust/src/graph/element.rs` over the market leaves, and
//! `rust/market/src/graph/market.rs`: what the element and event readings
//! answer over the crate's own leaves - the cross code a leaf stores under
//! its category and side, the execution clock, the digest a market element
//! and an operation continue - and what a market states: its price,
//! quantity, side and instrument, and the names a market operation goes by,
//! carried along a chain. An [`OrderEvent`] stands for any dated operation,
//! an [`Order`] for an undated market element, and a foreign type that
//! implements only the signatures for any other holder. What the traits do
//! over that foreign type and a text line alone is
//! `rust/tests/graph/element.rs`'s.

use std::collections::BTreeMap;
use std::hash::Hasher;

use smol_str::SmolStr;
use yggdryl::graph::{Element, Event};
use yggdryl::xxhash::Xxh3;
use yggdryl::{Ccy, Cfi, Decimal, Isin, Mic, State, Unit, Uuid};
use yggdryl_market::IdKey;
use yggdryl_market::graph::{Market, Operation, Order, OrderEvent};
use yggdryl_market::{IdSource, IdType, Identifier, Identifiers, Side, TimeInForce};

/// One report as a foreign caller would hold it: every fact the two traits
/// name, an identity assigned rather than derived, and nothing the graph
/// owns.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Report {
    uuid: Uuid,
    crossuuid: Uuid,
    crosscode: String,
    hashcode: u64,
    crosshashcode: u64,
    sources: Vec<Uuid>,
    unix: i64,
    state: State,
    seqnum: u64,
    creaunix: Option<i64>,
    sendunix: Option<i64>,
    exprunix: Option<i64>,
    prevunix: Option<i64>,
    prevuuid: Option<Uuid>,
    snapunix: Option<i64>,
    /// Whether the identity derives from the instant and the content code,
    /// which is what finalizing resets it to; an assigned identity stays
    /// assigned.
    derived: bool,
}

impl Report {
    fn at(uuid: u128, unix: i64) -> Self {
        Self {
            uuid: Uuid::from_v8(uuid),
            crossuuid: Uuid::from_v8(uuid),
            crosscode: String::new(),
            hashcode: 0,
            crosshashcode: 0,
            sources: Vec::new(),
            unix,
            state: State::from_spelling("New").expect("a shipped state"),
            seqnum: 0,
            creaunix: None,
            sendunix: None,
            exprunix: None,
            prevunix: None,
            prevuuid: None,
            snapunix: None,
            derived: false,
        }
    }
}

impl Element for Report {
    fn get_uuid(&self) -> Uuid {
        self.uuid
    }

    fn set_uuid(&mut self, uuid: Uuid) {
        self.uuid = uuid;
    }

    fn get_crossuuid(&self) -> Uuid {
        self.crossuuid
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.crossuuid = crossuuid;
    }

    fn get_crosscode(&self) -> &str {
        &self.crosscode
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.crosscode = crosscode;
    }

    fn get_hashcode(&self) -> u64 {
        self.hashcode
    }

    fn set_hashcode(&mut self, hashcode: u64) {
        self.hashcode = hashcode;
    }

    fn get_crosshashcode(&self) -> u64 {
        self.crosshashcode
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.crosshashcode = crosshashcode;
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        &self.sources
    }

    fn set_srcuuids(&mut self, mut sources: Vec<Uuid>) {
        sources.sort_unstable();
        sources.dedup();
        self.sources = sources;
    }

    fn is_after(&self, other: &Self) -> bool {
        self.unix > other.unix
    }

    fn finalize(&mut self) {
        // The cross codes always follow the cross code; an assigned identity
        // keeps its code, one derived from the instant and the content is
        // reset to what they now derive.
        self.sync_cross();
        if self.derived {
            let hashcode = self.digest_event().as_u64();
            self.finalized(hashcode);
        }
    }

    fn with_previous(self, previous: &Self) -> Option<Self> {
        self.following(previous)
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        self.merging(other)
    }
}

impl Event for Report {
    fn get_transunix(&self) -> i64 {
        self.unix
    }

    fn set_transunix(&mut self, unix: i64) {
        self.unix = unix;
    }

    fn get_state(&self) -> &State {
        &self.state
    }

    fn set_state(&mut self, state: State) {
        self.state = state;
    }

    fn get_seqnum(&self) -> u64 {
        self.seqnum
    }

    fn set_seqnum(&mut self, seqnum: u64) {
        self.seqnum = seqnum;
    }

    fn get_creaunix(&self) -> Option<i64> {
        self.creaunix
    }

    fn set_creaunix(&mut self, unix: Option<i64>) {
        self.creaunix = unix;
    }

    fn get_sendunix(&self) -> Option<i64> {
        self.sendunix
    }

    fn set_sendunix(&mut self, unix: Option<i64>) {
        self.sendunix = unix;
    }

    fn get_exprunix(&self) -> Option<i64> {
        self.exprunix
    }

    fn set_exprunix(&mut self, unix: Option<i64>) {
        self.exprunix = unix;
    }

    fn get_prevunix(&self) -> Option<i64> {
        self.prevunix
    }

    fn set_prevunix(&mut self, unix: Option<i64>) {
        self.prevunix = unix;
    }

    fn get_prevuuid(&self) -> Option<Uuid> {
        self.prevuuid
    }

    fn set_prevuuid(&mut self, uuid: Option<Uuid>) {
        self.prevuuid = uuid;
    }

    fn get_snapunix(&self) -> Option<i64> {
        self.snapunix
    }

    fn set_snapunix(&mut self, unix: Option<i64>) {
        self.snapunix = unix;
    }
}

/// One nanosecond count per millisecond: a derived identity opens with the
/// microsecond its instant falls in, and the instants below are spaced a
/// whole millisecond apart, so no two of them ever share one.
const MS: i64 = 1_000_000;

/// An instant a derived identity holds: `ms` milliseconds after one
/// evening in November 2023, UTC.
fn at(ms: i64) -> i64 {
    1_700_000_000_000_000_000 + ms * MS
}

pub(crate) fn filled() -> State {
    State::from_spelling("Filled").expect("a shipped state")
}

/// The millisecond, saturated sequence lane and 62-bit payload one generic
/// event identity carries, read back out of its UUIDv7 bits.
fn decoded(uuid: Uuid) -> (i64, u16, u64) {
    let packed = uuid.get();
    (
        i64::try_from(packed >> 80).expect("a millisecond count an i64 holds"),
        u16::try_from((packed >> 64) & 0xfff).expect("twelve sequence bits"),
        u64::try_from(packed & ((1 << 62) - 1)).expect("sixty-two payload bits"),
    )
}

/// The XXH3-64 of one cross code, as the trait derives it.
pub(crate) fn crosshash(crosscode: &str) -> u64 {
    let mut state = Xxh3::new();
    state.write(crosscode.as_bytes());
    state.as_u64()
}

fn decimal(text: &str) -> Decimal {
    Decimal::parse(text).expect("a decimal")
}

fn currency(code: &str) -> Ccy {
    Ccy::new(code).expect("a currency")
}

fn unit(text: &str) -> Unit {
    Unit::new(text).expect("a unit")
}

/// One security identifier of `kind` - `isin`, `cusip`, a FIX source code -
/// stated from `base` and validated by its type.
fn securityid(kind: &str, code: &str) -> Identifier {
    Identifier::new(
        IdKey::base(IdType::from_security_source(kind).expect("a security type")),
        code,
    )
    .expect("an identifier")
}

/// One identifier of a plain holder: a value of `kind` from `fix`.
fn identifier(kind: &str, value: &str) -> Identifier {
    Identifier::new(IdKey::base(kind.parse().expect("a type")), value).expect("an identifier")
}

/// Finalizes `this` the way a market event stating no operation of its own
/// does: `fill_market`, `sync_cross`, and the identity
/// [`Market::digest_market_event`] derives, composed from the same public
/// pieces the trait always exposed. It is fixture infrastructure for the
/// trait-level tests below, which need an event in "states no operation"
/// shape; a leaf's own finalize digests what that leaf states.
fn finalize_as_market_event<E: Event + Market + Sized>(this: &mut E) {
    this.fill_market();
    this.sync_cross();
    let hashcode = this.digest_market_event().as_u64();
    this.finalized(hashcode);
}

/// One trade as the crate holds it: a buy of a thousand barrels at 82.5
/// dollars, stated and not yet finalized.
fn stated(ms: i64) -> OrderEvent {
    let mut trade = OrderEvent::at(at(ms));
    trade.set_price(Some(decimal("82.5")), true);
    trade.set_currency(currency("USD"), true);
    trade.set_quantity(Some(Decimal::from_int(1_000)), true);
    trade.set_unit(unit("bbl"), true);
    trade.set_side(Side::read("Buy").expect("a side"), true);
    trade
}

/// The same trade, finalized as a market event stating no operation of its
/// own: its identity is what it states and when.
fn trade(ms: i64) -> OrderEvent {
    let mut trade = stated(ms);
    finalize_as_market_event(&mut trade);
    trade
}

/// The same operation, finalized.
fn operation(ms: i64) -> OrderEvent {
    let mut operation = stated(ms);
    operation.finalize();
    operation
}

#[test]
fn an_operation_goes_by_the_names_it_was_given_each_under_its_scheme() {
    crate::install::installed();
    let mut operation = OrderEvent::default();
    assert!(
        operation.get_identifiers().is_empty(),
        "no system named it yet"
    );
    assert!(
        operation
            .insert_identifier(identifier("OrderID", "O-1"))
            .expect("a plain holder takes every key")
    );
    assert!(
        operation
            .insert_identifier(identifier("ClOrdID", "C-1"))
            .expect("a plain holder takes every key")
    );
    // Held under the type folded to lower case, in byte order, so a walk over
    // them is the same walk whatever order they were stated in.
    assert_eq!(
        operation.get_identifiers().get(&IdType::ClOrdId),
        Some("C-1")
    );
    assert_eq!(
        operation.get_identifiers().get(&IdType::OrderId),
        Some("O-1")
    );
    assert_eq!(
        operation
            .get_identifiers()
            .iter()
            .map(|id| id.kind().as_str())
            .collect::<Vec<_>>(),
        ["clordid", "orderid"]
    );
    // A name stated once is stated: inserting its type and source again
    // fills nothing.
    assert!(
        !operation
            .insert_identifier(identifier("ORDERID", "O-2"))
            .expect("a plain holder")
    );
    assert_eq!(
        operation.get_identifiers().get(&IdType::OrderId),
        Some("O-1")
    );
    assert!(
        operation
            .remove_identifier(&IdKey::base(IdType::OrderId))
            .expect("a plain holder")
    );
    assert!(
        !operation
            .remove_identifier(&IdKey::base(IdType::OrderId))
            .expect("nothing left to remove")
    );
    // The set is replaced whole, never merged.
    let mut ids = Identifiers::new();
    ids.insert(identifier("EXECID", "E-1"));
    operation
        .set_identifiers(ids, true)
        .expect("a plain holder");
    assert_eq!(operation.get_identifiers().len(), 1);
    assert_eq!(operation.get_identifiers().get(&IdType::ClOrdId), None);
    // And a walk written against the signatures alone reads them the same
    // way, whatever holder stands behind them.
    fn execid<E: Operation + ?Sized>(held: &E) -> Option<&str> {
        held.get_identifiers().get(&IdType::ExecId)
    }
    assert_eq!(execid(&operation), Some("E-1"));
    let mut redated = operation.clone();
    redated.set_transunix(0);
    assert_eq!(execid(&redated), Some("E-1"));
}

/// A leaf stores the cross code it is given under its category and side,
/// can unsay it, and shares the cross identity with any holder stating the
/// stored code. The trait's syncing over a foreign holder and a text line is
/// `rust/tests/graph/element.rs`'s.
#[test]
fn a_leaf_stores_its_cross_code_under_its_category_and_side() {
    crate::install::installed();
    let mut element = OrderEvent::default();
    element.set_crosscode("O-100".to_owned());
    element.sync_cross();
    assert_eq!(element.get_crosscode(), "10:0:O-100");
    assert_eq!(element.get_crosshashcode(), crosshash("10:0:O-100"));
    let mut stored = Report::at(2, 10);
    stored.set_crosscode(element.get_crosscode().to_owned());
    stored.sync_cross();
    assert_eq!(element.get_crossuuid(), stored.get_crossuuid());
    assert_eq!(element.get_crosshashcode(), stored.get_crosshashcode());

    element.set_crosscode(String::new());
    assert_eq!(element.get_crosscode(), "", "and it can be unsaid");
}

#[test]
fn a_market_event_orders_by_its_instant_and_a_market_element_states_no_order() {
    crate::install::installed();
    // A market event orders by its instant.
    assert!(trade(20).is_after(&trade(10)));
    assert!(trade(10).is_before(&trade(20)));

    // An undated order has no instant and records no predecessor, so it
    // states no order: one that followed another is neither after nor
    // before it, and neither is one unrelated to it.
    let mut first = Order::new();
    first.set_crosscode("FIRST".to_owned());
    first.finalize();
    let next = Order::new()
        .with_previous(&first)
        .expect("an undated order follows another");
    assert!(!next.is_after(&first) && !next.is_before(&first));
    assert!(!first.is_after(&next) && !first.is_before(&next));
    let stranger = Order::new();
    assert!(!stranger.is_after(&first) && !stranger.is_before(&first));
}

#[test]
fn an_undated_version_inherits_only_an_absent_ticker() {
    crate::install::installed();
    // An undated element that follows another takes the predecessor's
    // ticker only where it states none of its own - a name issued at
    // creation holds for the whole lifecycle, but its own word is never
    // overridden.
    let mut previous = Order::new();
    // A crosscode neither states of its own forces `changed`, so
    // following answers `Some` whatever the ticker does.
    previous.set_crosscode("SCOPE-1".to_owned());
    previous.set_ticker(Some(SmolStr::new("AAPL")), true);
    previous.finalize();

    let inherited = Order::new()
        .with_previous(&previous)
        .expect("an undated order follows another");
    assert_eq!(inherited.get_ticker(), Some("AAPL"));

    let mut stated = Order::new();
    stated.set_ticker(Some(SmolStr::new("MSFT")), true);
    let stated = stated
        .with_previous(&previous)
        .expect("an undated order follows another");
    assert_eq!(
        stated.get_ticker(),
        Some("MSFT"),
        "a stated ticker is this element's own word"
    );
}

#[test]
fn following_carries_the_chains_identifiers_and_metadata_forward() {
    crate::install::installed();
    // Every identifier of the chain carries forward where the next
    // operation does not state it, and its own word stays where it does -
    // but a book entry's reference to its predecessor, which names one
    // step. The metadata alike: the next one's own values stand, and every
    // key of the chain's it does not state is beside them.
    let mut named = OrderEvent::at(at(10));
    named.set_crosscode("O-100".to_owned());
    named
        .insert_identifier(identifier("ORDERID", "O-1"))
        .expect("a plain holder");
    named
        .insert_identifier(identifier("CLORDID", "C-1"))
        .expect("a plain holder");
    named
        .insert_identifier(identifier("MDENTRYREFID", "R-1"))
        .expect("a plain holder");
    named.set_metadata(
        Some(
            [("desk", "EQ"), ("venue", "XPAR")]
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        ),
        true,
    );
    named.finalize();
    let mut next = OrderEvent::at(at(20));
    next.set_crosscode("O-100".to_owned());
    next.insert_identifier(identifier("EXECID", "E-2"))
        .expect("a plain holder");
    next.set_metadata(
        Some(
            [("desk", "FX")]
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        ),
        true,
    );
    next.finalize();
    let unfollowed = next.get_uuid();
    let next = next.with_previous(&named).expect("follows");
    assert_eq!(next.get_identifiers().get(&IdType::OrderId), Some("O-1"));
    assert_eq!(next.get_identifiers().get(&IdType::ExecId), Some("E-2"));
    assert_eq!(next.get_identifiers().get(&IdType::ClOrdId), Some("C-1"));
    assert_eq!(next.get_identifiers().get(&IdType::MdEntryRefId), None);
    assert_eq!(next.get_identifiers().len(), 3);
    let metadata: Vec<_> = next
        .get_metadata()
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    assert_eq!(metadata, [("desk", "FX"), ("venue", "XPAR")]);
    assert_ne!(
        next.get_uuid(),
        unfollowed,
        "what it takes is what it states"
    );
    let mut own = OrderEvent::at(at(20));
    own.set_crosscode("O-100".to_owned());
    own.insert_identifier(identifier("ORDERID", "O-2"))
        .expect("a plain holder");
    own.finalize();
    let own = own.with_previous(&named).expect("follows");
    assert_eq!(own.get_identifiers().get(&IdType::OrderId), Some("O-2"));
}

/// Parentage is a relation between types, never a part of a value: a
/// first-seen element states the identifiers it states and no parent of
/// them, whatever set holds them, once it finalizes.
#[test]
fn a_first_seen_identifier_states_no_parent() {
    crate::install::installed();
    let mut event = OrderEvent::at(at(10));
    event.set_crosscode("O-100".to_owned());
    event
        .insert_identifier(identifier("ORDERID", "O-1"))
        .expect("a plain holder");
    event
        .insert_identifier(identifier("CLORDID", "C-1"))
        .expect("a plain holder");
    event
        .insert_partyid(identifier("CUSTOMERACCOUNT", "ACC-1"))
        .expect("a plain holder");
    event
        .insert_securityid(securityid("ISIN", "US0378331005"))
        .expect("a plain holder");
    event.finalize();
    let keys = |ids: &Identifiers| {
        ids.iter()
            .map(|id| id.key().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(event.get_identifiers()), ["clordid", "orderid"]);
    assert_eq!(keys(event.get_partyids()), ["customeraccount"]);
    // The ISIN implies the national number it carries, derived and stated by
    // no source; no security identifier has a parent.
    assert_eq!(
        keys(event.get_securityids()),
        ["cusip", "derived:cusip", "isin"]
    );

    let mut element = Order::new();
    element
        .insert_securityid(securityid("ISIN", "US0378331005"))
        .expect("a plain holder");
    element.finalize();
    assert_eq!(
        keys(element.get_securityids()),
        ["cusip", "derived:cusip", "isin"]
    );
}

/// Along a chain a follower takes the parents of each base it states from
/// its predecessor: an `orderid` that changes names the value it replaced as
/// its `parentorderid` and the chain's first as its `origorderid`, and a
/// restatement of the same value moves neither.
#[test]
fn an_order_identifier_chain_ends_with_its_parent_and_origin() {
    crate::install::installed();
    let named = |ms: i64, value: &str| {
        let mut event = OrderEvent::at(at(ms));
        event.set_crosscode("O-100".to_owned());
        event
            .insert_identifier(identifier("ORDERID", value))
            .expect("a plain holder");
        event.finalize();
        event
    };
    let parentage = |event: &OrderEvent| {
        let ids = event.get_identifiers();
        let held = |kind: &str| {
            ids.get_from(&IdKey::base(kind.parse().expect("a type")))
                .map(str::to_owned)
        };
        (held("orderid"), held("parentorderid"), held("origorderid"))
    };
    let state = |order: &str, parent: Option<&str>, orig: Option<&str>| {
        (
            Some(order.to_owned()),
            parent.map(str::to_owned),
            orig.map(str::to_owned),
        )
    };
    let first = named(10, "A");
    assert_eq!(parentage(&first), state("A", None, None));
    let second = named(20, "B").with_previous(&first).expect("follows");
    assert_eq!(parentage(&second), state("B", Some("A"), Some("A")));
    let third = named(30, "C").with_previous(&second).expect("follows");
    assert_eq!(parentage(&third), state("C", Some("B"), Some("A")));
    let fourth = named(40, "D").with_previous(&third).expect("follows");
    assert_eq!(parentage(&fourth), state("D", Some("C"), Some("A")));
    let restated = named(50, "D").with_previous(&fourth).expect("follows");
    assert_eq!(parentage(&restated), state("D", Some("C"), Some("A")));
    assert_eq!(restated.get_identifiers().len(), 3);
}

/// A client order identifier has one parent, FIX's `OrigClOrdID(41)`: the
/// previous `clordid`, so a replace chain A, B, C ends with `origclordid` B.
#[test]
fn a_client_order_identifier_chain_names_its_previous_value() {
    crate::install::installed();
    let named = |ms: i64, value: &str| {
        let mut event = OrderEvent::at(at(ms));
        event.set_crosscode("O-100".to_owned());
        event
            .insert_identifier(identifier("CLORDID", value))
            .expect("a plain holder");
        event.finalize();
        event
    };
    let orig = |event: &OrderEvent| {
        event
            .get_identifiers()
            .get_from(&IdKey::base("origclordid".parse().expect("a type")))
            .map(str::to_owned)
    };
    let first = named(10, "A");
    assert_eq!(orig(&first), None);
    let second = named(20, "B").with_previous(&first).expect("follows");
    assert_eq!(orig(&second).as_deref(), Some("A"));
    let third = named(30, "C").with_previous(&second).expect("follows");
    assert_eq!(orig(&third).as_deref(), Some("B"));
    assert_eq!(
        third.get_identifiers().len(),
        2,
        "the client order identifier has no further parent"
    );
}

/// A follower states its own parents: the chain's statement only fills the
/// parents it does not state, and a parent under another source is never
/// taken for this source's.
#[test]
fn a_follower_keeps_the_parents_it_states_and_follows_each_source_alone() {
    crate::install::installed();
    let mut previous = OrderEvent::at(at(10));
    previous.set_crosscode("O-100".to_owned());
    previous
        .insert_identifier(identifier("ORDERID", "A"))
        .expect("a plain holder");
    previous.finalize();

    let mut next = OrderEvent::at(at(20));
    next.set_crosscode("O-100".to_owned());
    next.insert_identifier(identifier("ORDERID", "B"))
        .expect("a plain holder");
    next.insert_identifier(identifier("PARENTORDERID", "STATED"))
        .expect("a plain holder");
    next.finalize();
    let next = next.with_previous(&previous).expect("follows");
    let held = |event: &OrderEvent, kind: &str| {
        event
            .get_identifiers()
            .get_from(&IdKey::base(kind.parse().expect("a type")))
            .map(str::to_owned)
    };
    assert_eq!(held(&next, "parentorderid").as_deref(), Some("STATED"));
    assert_eq!(held(&next, "origorderid").as_deref(), Some("A"));

    let venue: IdSource = "venue".parse().expect("a source");
    let mut other = OrderEvent::at(at(30));
    other.set_crosscode("O-100".to_owned());
    other
        .insert_identifier(
            Identifier::new(IdKey::new(venue.clone(), IdType::OrderId), "Z")
                .expect("an identifier"),
        )
        .expect("a plain holder");
    other.finalize();
    let other = other.with_previous(&next).expect("follows");
    assert_eq!(
        other
            .get_identifiers()
            .get_from(&IdKey::new(venue, "parentorderid".parse().expect("a type"))),
        None,
        "the venue's order identifier has no predecessor under the venue"
    );
}

/// An element stating a parent but not its base is what it came from: it
/// takes the base from its nearest stated parent once it finalizes.
#[test]
fn an_element_stating_only_a_parent_takes_the_base_from_it() {
    crate::install::installed();
    let mut event = OrderEvent::at(at(10));
    event.set_crosscode("O-100".to_owned());
    event
        .insert_identifier(identifier("ORIGORDERID", "A"))
        .expect("a plain holder");
    event
        .insert_identifier(identifier("PARENTORDERID", "C"))
        .expect("a plain holder");
    event.finalize();
    assert_eq!(
        event.get_identifiers().get(&IdType::OrderId),
        Some("C"),
        "the nearest parent: parentorderid before origorderid"
    );

    let mut origin = OrderEvent::at(at(10));
    origin.set_crosscode("O-100".to_owned());
    origin
        .insert_identifier(identifier("ORIGORDERID", "A"))
        .expect("a plain holder");
    origin.finalize();
    assert_eq!(origin.get_identifiers().get(&IdType::OrderId), Some("A"));

    let mut stated = OrderEvent::at(at(10));
    stated.set_crosscode("O-100".to_owned());
    stated
        .insert_identifier(identifier("ORDERID", "B"))
        .expect("a plain holder");
    stated
        .insert_identifier(identifier("PARENTORDERID", "C"))
        .expect("a plain holder");
    stated.finalize();
    assert_eq!(
        stated.get_identifiers().get(&IdType::OrderId),
        Some("B"),
        "a stated base is never replaced"
    );

    let mut replaced = OrderEvent::at(at(10));
    replaced.set_crosscode("O-100".to_owned());
    replaced
        .insert_identifier(identifier("ORIGCLORDID", "C-1"))
        .expect("a plain holder");
    replaced.finalize();
    assert_eq!(
        replaced.get_identifiers().get(&IdType::ClOrdId),
        Some("C-1"),
        "origclordid is the nearest, and only, parent a clordid has"
    );
}

#[test]
fn following_a_market_event_carries_its_latest_execution() {
    crate::install::installed();
    let mut previous = stated(10);
    previous.set_state(filled());
    previous.set_execunix(Some(at(7)), true);
    previous.set_sendunix(Some(at(9)));
    previous.finalize();

    let next = operation(20)
        .with_previous(&previous)
        .expect("the later event follows");
    assert_eq!(
        next.get_execunix(),
        Some(at(7)),
        "the latest lifecycle execution carries"
    );
    assert_eq!(
        next.get_sendunix(),
        None,
        "no predecessor wire clock carries"
    );

    // The successor inherited the predecessor's furthest state and its
    // execution clock. Another statement of it keeps that clock rather than
    // dating the inherited state from its own instant.
    let mut other = next.clone();
    other.set_execunix(None, true);
    other.set_srcuuids(vec![Uuid::from_v8(99)]);
    let restated = other.clone().restating(&next);
    assert_eq!(restated.get_execunix(), Some(at(7)));
    let merged = next
        .clone()
        .merge_with(&other)
        .expect("the other statement added a source");
    assert_eq!(
        merged.get_execunix(),
        Some(at(7)),
        "the carried clock survives a full merge"
    );
    assert!(
        next.clone().with_previous(&previous).is_none(),
        "replaying the same lifecycle edge changes nothing"
    );
    let inserted = operation(15)
        .with_previous(&previous)
        .expect("the inserted event follows");
    let relinked = next
        .clone()
        .with_previous(&inserted)
        .expect("the stamped successor takes the inserted predecessor");
    assert_eq!(relinked.get_execunix(), Some(at(7)));
    assert_eq!(relinked.get_prevuuid(), Some(inserted.get_uuid()));

    // An execution stating no clock dates itself from its own instant.
    let mut execution = stated(30);
    execution.set_state(filled());
    execution.set_sendunix(Some(at(31)));
    execution.finalize();
    let execution = execution
        .with_previous(&operation(25))
        .expect("the execution follows");
    assert_eq!(execution.get_execunix(), Some(at(30)));
    assert_eq!(execution.get_sendunix(), Some(at(31)));

    let later = operation(40)
        .with_previous(&execution)
        .expect("the non-execution successor follows");
    assert_eq!(
        later.get_execunix(),
        Some(at(30)),
        "a later non-execution carries the last execution clock"
    );

    let mut later_execution = stated(50);
    later_execution.set_state(State::read("PartiallyFilled").unwrap());
    later_execution.finalize();
    let later_execution = later_execution
        .with_previous(&later)
        .expect("the later execution follows");
    assert_eq!(
        later_execution.get_execunix(),
        Some(at(50)),
        "a later execution replaces the carried clock"
    );

    let mut explicit = stated(41);
    explicit.set_state(filled());
    explicit.set_execunix(Some(at(35)), true);
    explicit.finalize();
    let explicit = explicit
        .with_previous(&operation(39))
        .expect("the stated execution follows");
    assert_eq!(
        explicit.get_execunix(),
        Some(at(35)),
        "an explicit instant wins"
    );

    let mut stale = stated(60);
    stale.set_state(filled());
    stale.set_execunix(Some(at(25)), true);
    stale.finalize();
    let stale = stale
        .with_previous(&later_execution)
        .expect("the delayed execution report follows");
    assert_eq!(
        stale.get_execunix(),
        Some(at(50)),
        "a delayed report cannot regress the lifecycle's latest execution"
    );
}

/// A leaf following another adopts the chain's code under its own category
/// and side. The adoption over a foreign holder and a text line is
/// `rust/tests/graph/element.rs`'s.
#[test]
fn a_leaf_adopts_its_predecessors_code_under_its_own_category_and_side() {
    crate::install::installed();
    // Every follower's cross identity derives from the code it stores.
    let derived = |element: &dyn Element, what: &str| {
        assert_eq!(
            element.get_crosshashcode(),
            crosshash(element.get_crosscode()),
            "{what}"
        );
        assert_eq!(
            element.get_crossuuid(),
            Uuid::from_v8(u128::from(element.get_crosshashcode())),
            "{what}"
        );
    };
    // A market event adopts the code and re-derives its identity.
    let mut placed = trade(10);
    placed.set_crosscode("O-100".to_owned());
    placed.finalize();
    let mut fill = trade(20);
    fill.set_crosscode("O-999".to_owned());
    fill.finalize();
    let before = fill.get_uuid();
    let fill = fill.with_previous(&placed).expect("follows");
    // The code is stored under the side the event takes.
    assert_eq!(fill.get_crosscode(), "10:1:O-100");
    assert_eq!(fill.get_crossuuid(), placed.get_crossuuid());
    derived(&fill, "the crate's own event");
    assert_eq!(fill.get_prevuuid(), Some(placed.get_uuid()));
    assert_ne!(fill.get_uuid(), before, "followed, so finalized");
    assert_eq!(fill.get_uuid(), fill.time_uuid().expect("an identity"));

    // A market element follows too, and adopts the code the same way.
    let mut first = OrderEvent::default();
    first.set_crosscode("O-100".to_owned());
    first.finalize();
    let mut next = OrderEvent::default();
    next.set_crosscode("O-999".to_owned());
    next.finalize();
    let next = next.with_previous(&first).expect("follows");
    assert_eq!(next.get_crosscode(), "10:0:O-100");
    assert_eq!(next.get_crossuuid(), first.get_crossuuid());
    derived(&next, "a market element");
    assert!(next.clone().with_previous(&next).is_none(), "never itself");
    assert!(
        next.clone().with_previous(&first).is_none(),
        "following again changes nothing"
    );
}

#[test]
fn two_statements_of_one_market_event_finalize_to_one_identity() {
    crate::install::installed();
    // Two statements of one market event finalize to one identity: the
    // twin of a followed event restates it and is it.
    let placed = trade(10);
    let fill = trade(20);
    let twin = fill.clone();
    let fill = fill.with_previous(&placed).expect("follows");
    assert_ne!(twin.get_uuid(), fill.get_uuid(), "followed, so moved");
    let twin = twin.restating(&fill);
    assert_eq!(twin, fill);
    assert_eq!(twin.get_uuid(), fill.get_uuid());
    assert_eq!(twin.get_uuid(), twin.time_uuid().expect("an identity"));
}

#[test]
fn restating_and_merging_keep_the_earliest_execution_clock() {
    crate::install::installed();
    // A market event states no execution clock until one is stated or its
    // state reports an execution.
    let mut event = OrderEvent::at(40);
    assert_eq!(event.get_execunix(), None);
    event.set_execunix(Some(37), true);
    assert_eq!(event.get_execunix(), Some(37));

    let mut unstamped = trade(40);
    unstamped.set_state(filled());
    let observed = trade(40);
    let merged = unstamped
        .merge_with(&observed)
        .expect("the execution clock was filled");
    assert_eq!(
        merged.get_execunix(),
        Some(at(40)),
        "a raw execution observation dates itself before the full merge"
    );

    // Market events fold their execution clock, a market fact, the way
    // every event folds its recording: the earliest either statement knows.
    let mut market = trade(20);
    market.set_state(filled());
    market.set_sendunix(Some(at(30)));
    market.finalize();
    let mut market_other = market.clone();
    market_other.set_execunix(Some(at(18)), true);
    market_other.set_sendunix(Some(at(25)));
    let restated = market_other.clone().restating(&market);
    assert_eq!(restated.get_execunix(), Some(at(18)));
    assert_eq!(restated.get_sendunix(), Some(at(25)));
    let merged = market
        .merge_with(&market_other)
        .expect("the market event instants moved");
    assert_eq!(merged.get_execunix(), Some(at(18)));
    assert_eq!(merged.get_sendunix(), Some(at(25)));
}

/// A leaf's own inputs reproject its identity eagerly: the place it stands
/// at, outside the code it digests, and the cross code it stores under its
/// category and side. The reprojection over a text line is
/// `rust/tests/graph/element.rs`'s.
#[test]
fn a_market_events_place_and_stored_cross_code_reproject_its_identity() {
    crate::install::installed();
    let mut event = OrderEvent::at(at(0));
    event.set_hashcode(0xCAFE);
    let uncrossed = event.get_uuid();
    assert_eq!(uncrossed, event.time_uuid().unwrap());
    assert_eq!(event.get_crossuuid(), uncrossed);

    // The place at the instant owns rand_a and is also fed whole into
    // rand_b, and it is where the event stands, never what it says: the
    // code is the same wherever a stream placed it.
    let placed = event.digest_market_event().as_u64();
    event.set_seqnum(1);
    assert_eq!(event.get_uuid(), event.time_uuid().unwrap());
    assert_ne!(event.get_uuid(), uncrossed);
    assert_eq!(decoded(event.get_uuid()).1, 1);
    assert_eq!(
        event.digest_market_event().as_u64(),
        placed,
        "the place is outside the code"
    );
    event.set_seqnum(0);
    assert_eq!(event.get_uuid(), uncrossed);

    // The leaf stores the code under its category and side, and the cross
    // hash is the stored code's.
    event.set_crosscode("O-100".to_owned());
    assert_eq!(event.get_crosscode(), "10:0:O-100");
    assert_eq!(event.get_uuid(), event.time_uuid().unwrap());
    assert_ne!(event.get_uuid(), uncrossed, "the cross seed moved");
    assert_eq!(event.get_crosshashcode(), crosshash("10:0:O-100"));
    assert_eq!(
        event.get_crossuuid(),
        Uuid::from_v8(u128::from(crosshash("10:0:O-100")))
    );
    event.set_crosscode(String::new());
    assert_eq!(event.get_crosshashcode(), 0);
    assert_eq!(event.get_uuid(), uncrossed);
    assert_eq!(event.get_crossuuid(), uncrossed);
}

#[test]
fn a_market_element_names_its_instrument_the_way_the_market_does() {
    crate::install::installed();
    let mut held = trade(10);
    assert!(
        held.get_securityids().is_empty(),
        "an instrument is named only where the market names it"
    );
    assert!(held.get_cficode().is_none());
    assert!(held.get_miccode().is_none());
    // One identifier per source, stated through the fallible verbs a view
    // of another store may refuse and a plain holder never does.
    assert!(
        held.insert_securityid(securityid("ISIN", "US0378331005"))
            .expect("a plain holder")
    );
    assert!(
        held.insert_securityid(securityid("CUSIP", "037833100"))
            .expect("a plain holder")
    );
    assert!(
        held.insert_securityid(securityid("2", "B0YBKJ7"))
            .expect("the SEDOL source code")
    );
    assert!(
        held.insert_securityid(securityid("BLOOMBERG", "AAPL US EQUITY"))
            .expect("a plain holder")
    );
    held.set_cficode(Some(Cfi::new("ESVUFR").expect("a CFI")), true);
    held.set_miccode(Some(Mic::new("XPAR").expect("a MIC")), true);
    // Read by the type, which a spelling folds into; a FIX source code names
    // the type where the identifier is built, never on a lookup.
    assert_eq!(
        held.get_securityids().get(&IdType::Isin),
        Some("US0378331005")
    );
    assert_eq!(
        held.get_securityids().get(&"ISIN_Number".parse().unwrap()),
        Some("US0378331005")
    );
    assert_eq!(held.get_securityids().get(&"4".parse().unwrap()), None);
    assert_eq!(securityid("4", "US0378331005").kind(), "isin");
    assert_eq!(
        held.get_securityids().get(&IdType::Cusip),
        Some("037833100")
    );
    assert_eq!(held.get_securityids().get(&IdType::Sedol), Some("B0YBKJ7"));
    assert_eq!(
        held.get_securityids().get(&IdType::Bloomberg),
        Some("AAPL US EQUITY")
    );
    assert_eq!(held.get_securityids().len(), 4);
    assert_eq!(
        held.get_securityids()
            .iter()
            .map(|id| id.kind().as_str())
            .collect::<Vec<_>>(),
        ["bloomberg", "cusip", "isin", "sedol"],
        "held in type order"
    );
    assert_eq!(held.get_cficode().map(Cfi::as_str), Some("ESVUFR"));
    assert_eq!(held.get_miccode().map(Mic::as_str), Some("XPAR"));
    // Each is unsaid on its own; a stated identifier is never overwritten
    // by inserting, and a second statement under a held source fills nothing.
    assert!(
        held.remove_securityid(&IdKey::base(IdType::Cusip))
            .expect("a plain holder")
    );
    assert!(held.get_securityids().get(&IdType::Cusip).is_none());
    assert!(held.get_securityids().get(&IdType::Isin).is_some());
    assert!(
        !held
            .insert_securityid(securityid("ISIN", "US5949181045"))
            .expect("a plain holder")
    );
    assert_eq!(
        held.get_securityids().get(&IdType::Isin),
        Some("US0378331005")
    );
    // The codes are the crate's own: a spelling that is no identifier never
    // reaches the element.
    assert!(
        Isin::new("US037833100").is_err(),
        "eleven characters are no ISIN"
    );
    assert!(
        Identifier::new(IdKey::base(IdType::Isin), "US037833100").is_err(),
        "and no security identifier of the ISIN type either"
    );
    assert_eq!(
        IdType::Isin.rank("US0378331006"),
        1,
        "a wrong check digit is a rank, not a refusal"
    );
    assert!(
        IdType::from_security_source("ticker").is_err(),
        "a ticker is a name, never a security identifier source"
    );
}

#[test]
fn a_market_element_answers_its_five_facts_and_is_still_an_event() {
    crate::install::installed();
    let mut held = trade(10);
    assert_eq!(held.get_price(), Some(decimal("82.5")));
    assert_eq!(held.get_currency().as_str(), "USD");
    assert_eq!(held.get_quantity(), Some(Decimal::from_int(1_000)));
    assert_eq!(held.get_unit().as_str(), "bbl");
    assert_eq!(held.get_side().as_str(), "BUYS");

    held.set_price(Some(Decimal::from_int(83)), true);
    held.set_currency(currency("EUR"), true);
    held.set_quantity(None, true);
    held.set_unit(unit("MWh"), true);
    held.set_side(Side::read("2").expect("a side"), true);
    assert_eq!(held.get_price(), Some(Decimal::from_int(83)));
    assert_eq!(held.get_currency().as_str(), "EUR");
    assert_eq!(
        held.get_quantity(),
        None,
        "a quantity taken away is none stated, never a zero"
    );
    assert_eq!(held.get_unit().as_str(), "MWh");
    assert_eq!(held.get_side().as_str(), "SELL");
    assert_eq!(held.get_side(), Side::Sell, "a side is a value, copied out");

    // A new element states nothing: no price, no quantity, no currency, no
    // unit, no side - and no price is `None`, never a zero standing in.
    let bare = OrderEvent::default();
    assert_eq!((bare.get_price(), bare.get_quantity()), (None, None));
    assert_eq!(bare.get_currency(), &Ccy::none());
    assert_eq!(bare.get_unit(), &Unit::none());
    assert!(bare.get_unit().is_none());
    assert_eq!(bare.get_side(), Side::Unknown);
    assert_eq!(bare.get_ticker(), None);
    assert!(bare.get_metadata().is_empty());

    // One walk written against the market event signatures reads all three
    // traits, and the timed readings are the market event's too.
    fn readings<E: Event + Market + ?Sized>(
        object: &E,
    ) -> (Uuid, i64, u64, Option<Uuid>, Option<Decimal>) {
        (
            object.get_uuid(),
            object.get_transunix(),
            object.get_seqnum(),
            object.get_prevuuid(),
            object.get_price(),
        )
    }
    let next = trade(20)
        .with_previous(&held)
        .expect("a later trade follows");
    assert_eq!(
        readings(&next),
        (
            next.time_uuid().expect("an identity"),
            at(20),
            // A later instant keeps its own place.
            0,
            Some(held.get_uuid()),
            // What the trade itself says moves nowhere.
            Some(decimal("82.5")),
        )
    );
}

#[test]
fn merging_a_market_event_takes_the_later_statement_and_the_better_codes() {
    crate::install::installed();
    let mut first = trade(10);
    first.set_currency(Ccy::none(), true);
    first.set_side(Side::Unknown, true);
    first.set_cficode(Some(Cfi::new("ESXXXR").expect("a CFI")), true);
    first
        .insert_securityid(securityid("ISIN", "US0378331005"))
        .expect("a plain holder");
    first.finalize();
    // A later statement of the same trade: the identity kept, the instant
    // and the facts restated.
    let mut later = first.clone();
    later.set_transunix(at(20));
    later.set_price(Some(Decimal::from_int(83)), true);
    later.set_quantity(Some(Decimal::from_int(5)), true);
    later.set_unit(unit("MWh"), true);
    later.set_currency(currency("EUR"), true);
    later.set_side(Side::read("2").expect("a side"), true);
    later.set_cficode(Some(Cfi::new("ESVUFX").expect("a CFI")), true);
    later.set_miccode(Some(Mic::new("XPAR").expect("a MIC")), true);
    // The capture protocol already proved these are two observations of one
    // event. Identity-input setters keep a standalone event coherent, so
    // state that shared capture identity explicitly before the generic fold.
    later.set_uuid(first.get_uuid());

    // The later statement has the last word on the market's facts, and each
    // code is the better of the two: the earlier fills what the later left
    // unknown, and a code only one statement names is that one's.
    let merged = first.clone().merge_with(&later).expect("the same trade");
    assert_eq!(merged.get_transunix(), at(20));
    assert_eq!(
        (
            merged.get_price(),
            merged.get_quantity(),
            merged.get_unit().as_str()
        ),
        (
            Some(Decimal::from_int(83)),
            Some(Decimal::from_int(5)),
            "MWh"
        )
    );
    assert_eq!(merged.get_currency().as_str(), "EUR");
    assert_eq!(merged.get_side().as_str(), "SELL");
    assert_eq!(merged.get_cficode().map(Cfi::as_str), Some("ESVUFR"));
    assert_eq!(
        merged.get_securityids().get(&IdType::Isin),
        Some("US0378331005")
    );
    assert_eq!(
        merged.get_securityids().get(&IdType::Cusip),
        Some("037833100"),
        "the CUSIP the ISIN carries was derived when the first statement finalized"
    );
    assert_eq!(merged.get_miccode().map(Mic::as_str), Some("XPAR"));
    // Merged, the event is finalized: its identity is what it now says.
    assert_eq!(merged.get_uuid(), merged.time_uuid().expect("an identity"));
    assert_ne!(merged.get_uuid(), first.get_uuid());

    // Merged the other way round the later statement still leads, so the
    // reading does not depend on which statement a caller held.
    let merged = later.clone().merge_with(&first).expect("the same trade");
    assert_eq!(
        (merged.get_price(), merged.get_currency().as_str()),
        (Some(Decimal::from_int(83)), "EUR")
    );
    assert_eq!(merged.get_cficode().map(Cfi::as_str), Some("ESVUFR"));
    assert_eq!(
        merged.get_securityids().get(&IdType::Isin),
        Some("US0378331005")
    );

    // A statement that knows a code the later one states as none keeps
    // its own: the later statement leads, and unknown takes the other.
    let mut bare = later.clone();
    bare.set_transunix(at(30));
    bare.set_currency(Ccy::none(), true);
    bare.set_uuid(later.get_uuid());
    let merged = later.clone().merge_with(&bare).expect("the same trade");
    assert_eq!(merged.get_currency().as_str(), "EUR");
    assert_eq!(merged.get_transunix(), at(30));

    // Another trade does not merge at all.
    assert!(first.merge_with(&trade(30)).is_none());
}

#[test]
fn merging_a_market_event_lets_the_statement_sent_last_lead_event_time() {
    crate::install::installed();
    let first = trade(10);
    let mut event_time_later = first.clone();
    event_time_later.set_transunix(at(30));
    event_time_later.set_sendunix(Some(at(100)));
    event_time_later.set_execunix(Some(at(12)), true);
    event_time_later.set_price(Some(Decimal::from_int(83)), true);
    event_time_later.set_unit(unit("old"), true);
    event_time_later.set_uuid(first.get_uuid());

    let mut recorded_later = first.clone();
    recorded_later.set_transunix(at(20));
    recorded_later.set_sendunix(Some(at(200)));
    recorded_later.set_execunix(Some(at(15)), true);
    recorded_later.set_price(Some(Decimal::from_int(84)), true);
    recorded_later.set_unit(unit("reference"), true);
    recorded_later.set_uuid(first.get_uuid());

    for merged in [
        event_time_later
            .clone()
            .merge_with(&recorded_later)
            .expect("the reference moves the event"),
        recorded_later
            .clone()
            .merge_with(&event_time_later)
            .expect("the other statement contributes its earlier clocks"),
    ] {
        assert_eq!(merged.get_transunix(), at(20));
        assert_eq!(merged.get_price(), Some(Decimal::from_int(84)));
        assert_eq!(merged.get_unit().as_str(), "reference");
        assert_eq!(merged.get_execunix(), Some(at(12)));
        assert_eq!(merged.get_sendunix(), Some(at(100)));
    }
}

#[test]
fn a_market_event_a_merge_moved_is_finalized() {
    crate::install::installed();
    // A merge that moves a market event finalizes it; one that moves nothing
    // answers nothing.
    let trade = trade(40);
    let mut later = trade.clone();
    later.set_transunix(at(50));
    later.set_uuid(trade.get_uuid());
    let merged = trade
        .clone()
        .merge_with(&later)
        .expect("the same trade, later");
    assert_eq!(merged.get_transunix(), at(50));
    assert_eq!(merged.get_uuid(), merged.time_uuid().expect("an identity"));
    let same = trade.clone();
    assert!(trade.merge_with(&same).is_none(), "nothing moved");
}

/// A market element's and an operation's digests continue the event's with
/// what they state. Where an event's digest starts is
/// `rust/tests/graph/element.rs`'s.
#[test]
fn a_market_element_and_an_operation_digest_what_they_state() {
    crate::install::installed();
    // A market element continues with its facts: a price moves the code,
    // and the event's digest continues the element's with its own.
    let trade = trade(10);
    let mut repriced = trade.clone();
    repriced.set_price(Some(Decimal::from_int(90)), true);
    assert_ne!(
        trade.digest_market_event().as_u64(),
        repriced.digest_market_event().as_u64()
    );
    assert_eq!(
        trade.digest_market_event().as_u64(),
        trade.clone().digest_market_event().as_u64()
    );
    assert_ne!(
        trade.digest_market().as_u64(),
        trade.digest_market_event().as_u64()
    );
    let element = trade.clone().into_element();
    assert_eq!(
        element.digest_market().as_u64(),
        trade.digest_market().as_u64()
    );
    // An operation continues with its own facts: a name it goes by moves
    // the code.
    let operation = operation(10);
    let mut named = operation.clone();
    named
        .insert_identifier(identifier("ORDERID", "O-1"))
        .expect("a plain holder");
    assert_ne!(
        operation.digest_operation_event().as_u64(),
        named.digest_operation_event().as_u64()
    );
    // An operation stating none of its own facts digests as the market
    // event it is; one it states continues that code.
    assert_eq!(
        operation.digest_operation_event().as_u64(),
        operation.digest_market_event().as_u64()
    );
    assert_ne!(
        named.digest_operation_event().as_u64(),
        named.digest_market_event().as_u64()
    );
    let entry = operation.clone().into_element();
    assert_eq!(
        entry.digest_operation().as_u64(),
        operation.digest_operation().as_u64()
    );
    // Metadata is part of what a market states, in key order.
    let mut annotated = trade.clone();
    annotated.set_metadata(
        Some(BTreeMap::from([(
            SmolStr::new("Feed"),
            SmolStr::new("PRIMARY"),
        )])),
        true,
    );
    assert_ne!(
        trade.digest_market().as_u64(),
        annotated.digest_market().as_u64()
    );
}

/// A market event's code excludes the derived cross facts and provenance,
/// while its identity takes the cross hash as its payload seed. The same
/// over a foreign holder and a text line is `rust/tests/graph/element.rs`'s.
#[test]
fn a_market_events_code_ignores_derived_cross_facts() {
    crate::install::installed();
    let stated = trade(10);
    let mut crossed = stated.clone();
    crossed.set_crosshashcode(0xCD);
    crossed.set_crossuuid(Uuid::from_v8(77));
    crossed.set_srcuuids(vec![Uuid::from_v8(70)]);
    assert_eq!(
        crossed.digest_market_event().as_u64(),
        stated.digest_market_event().as_u64()
    );
    assert_ne!(
        crossed.time_uuid().expect("an identity"),
        stated.time_uuid().expect("an identity")
    );
    // Finalized as a market event stating no operation of its own, the
    // identity is the same and the cross facts are back in step with the
    // cross code, which states none.
    let mut finalized = crossed.clone();
    finalize_as_market_event(&mut finalized);
    assert_eq!(finalized.get_uuid(), stated.get_uuid());
    assert_eq!(finalized.get_hashcode(), stated.get_hashcode());
    assert_eq!(finalized.get_crosshashcode(), 0);
    assert_eq!(finalized.get_crossuuid(), finalized.get_uuid());
    assert_eq!(
        finalized.get_srcuuids(),
        [Uuid::from_v8(70)],
        "kept, not fed"
    );
}

#[test]
fn the_crates_own_holders_derive_their_identity_from_what_they_state() {
    crate::install::installed();
    // A market element's identity is its content: RFC 9562 UUIDv8 over the
    // code, so two elements stating the same things are one identity. A
    // market element states no operation of its own, so it finalizes as
    // one - a book side's own role, pinned in `rust/market/tests/graph/book.rs`.

    // A market event's identity is UUIDv7 over its instant and the code its
    // content digests to, so the same facts at another instant differ.
    let event = trade(10);
    assert_eq!(event.get_uuid(), event.time_uuid().expect("an identity"));
    assert_eq!(event.get_hashcode(), event.digest_market_event().as_u64());
    assert_eq!(event.get_crossuuid(), event.get_uuid(), "no cross code");
    assert_eq!(trade(10), event);
    assert_ne!(trade(11).get_uuid(), event.get_uuid());
    assert_eq!(trade(11).get_hashcode(), event.get_hashcode());

    // And an operation event's over its instant, its operation facts and
    // the kind it is: an order's code continues the operation's with the
    // word `order`, so the same facts as a quote are another operation.
    let operation = operation(10);
    assert_eq!(
        operation.get_uuid(),
        operation.time_uuid().expect("an identity")
    );
    assert_ne!(
        operation.get_hashcode(),
        operation.digest_operation_event().as_u64()
    );
    let mut quoted = yggdryl_market::graph::QuoteEvent::from(&operation);
    quoted.finalize();
    assert_ne!(quoted.get_hashcode(), operation.get_hashcode());
    let mut again = operation.clone();
    again.set_hashcode(0);
    again.finalize();
    assert_eq!(again, operation, "the code is what the order states");
    assert_ne!(
        operation.get_hashcode(),
        event.get_hashcode(),
        "the kind it is is part of what the operation states"
    );
}

#[test]
fn filling_never_invents_a_price_or_a_quantity_the_element_did_not_state() {
    crate::install::installed();
    // What a report says about a trade and nothing about an order: its
    // last executed price and quantity are those facts and nothing more.
    // The price and the quantity it states stay none.
    let mut fill = OrderEvent::at(at(10));
    fill.set_side(Side::read("Buy").expect("a side"), true);
    fill.set_lastpx(Some(decimal("82.5")), true);
    fill.set_lastqty(Some(Decimal::from_int(300)), true);
    fill.set_avgpx(Some(decimal("82.25")), true);
    fill.fill_market();
    assert_eq!(
        fill.get_price(),
        None,
        "a last executed price is not the price stated"
    );
    assert_eq!(
        fill.get_quantity(),
        None,
        "nor a last executed quantity the quantity stated"
    );
    assert_eq!(fill.get_lastpx(), Some(decimal("82.5")));
    assert_eq!(fill.get_lastqty(), Some(Decimal::from_int(300)));
    assert_eq!(fill.get_avgpx(), Some(decimal("82.25")));

    // An average is an average: it stands in for no price either.
    let mut averaged = OrderEvent::at(at(20));
    averaged.set_avgpx(Some(Decimal::from_int(99)), true);
    averaged.fill_market();
    assert_eq!(averaged.get_price(), None);
    assert_eq!(averaged.get_avgpx(), Some(Decimal::from_int(99)));

    // How much is done and how much is left, on an order still working,
    // are together what it ordered - FIX's `LeavesQty = OrderQty - CumQty` -
    // and what is left open is the quantity it is about, never what it
    // ordered.
    let mut working = OrderEvent::at(at(30));
    working.set_state(State::PartiallyFilled);
    working.set_cumqty(Some(Decimal::from_int(40)), true);
    working.set_leavesqty(Some(Decimal::from_int(60)), true);
    working.fill_market();
    assert_eq!(working.get_ordqty(), Some(Decimal::from_int(100)));
    assert_eq!(working.get_quantity(), Some(Decimal::from_int(60)));

    // The ISIN's national number fills the source it names, as a derived
    // identifier, only where the element states none under it.
    let mut listed = OrderEvent::at(at(45));
    listed
        .insert_securityid(securityid("ISIN", "US0378331005"))
        .expect("a plain holder");
    listed.fill_market();
    assert_eq!(
        listed.get_securityids().get(&IdType::Cusip),
        Some("037833100")
    );
    let mut stated = OrderEvent::at(at(46));
    stated
        .insert_securityid(securityid("CUSIP", "594918104"))
        .expect("a plain holder");
    stated
        .insert_securityid(securityid("ISIN", "US0378331005"))
        .expect("a plain holder");
    stated.fill_market();
    assert_eq!(
        stated.get_securityids().get(&IdType::Cusip),
        Some("594918104")
    );

    // Filling twice changes nothing the first run did not, and a fact the
    // element stated is never overwritten: a trade stating its price keeps
    // it beside a last executed price of its own.
    let mut once = trade(50);
    once.set_lastpx(Some(Decimal::from_int(1)), true);
    once.fill_market();
    let twice = {
        let mut held = once.clone();
        held.fill_market();
        held
    };
    assert_eq!(once.get_price(), Some(decimal("82.5")));
    assert_eq!(once.get_lastpx(), Some(Decimal::from_int(1)));
    assert_eq!(once, twice);
}

/// An operation following another keeps the side it states, and one
/// stating none is about the side its chain took.
#[test]
fn an_operation_following_another_keeps_its_own_side_or_takes_the_chains() {
    crate::install::installed();
    let order = |ms: i64, side: Option<&str>| {
        let mut held = OrderEvent::at(at(ms));
        held.set_crosscode("O1".to_owned());
        if let Some(side) = side {
            held.set_side(Side::read(side).expect("a side"), true);
        }
        held.finalize();
        held
    };
    let first = order(10, Some("Buy"));
    let own = order(20, Some("Sell"))
        .with_previous(&first)
        .expect("the next order");
    assert_eq!(own.get_side().as_str(), "SELL", "its own side stands");
    let silent = order(30, None)
        .with_previous(&first)
        .expect("the next order");
    assert_eq!(silent.get_side().as_str(), "BUYS", "the chain's side");
}

#[test]
fn a_linked_market_event_still_inherits_a_missing_ticker() {
    crate::install::installed();
    let mut previous = OrderEvent::at(at(10));
    previous.set_ticker(Some(SmolStr::new("AAPL")), true);
    previous.finalize();
    let mut linked = OrderEvent::at(at(20))
        .with_previous(&previous)
        .expect("the next version");
    linked.set_ticker(None, true);
    linked.finalize();
    let before = linked.get_uuid();

    let inherited = linked
        .with_previous(&previous)
        .expect("market facts can change when the timed link is unchanged");
    assert_eq!(inherited.get_ticker(), Some("AAPL"));
    assert_eq!(inherited.get_prevuuid(), Some(previous.get_uuid()));
    // A later instant keeps its own place.
    assert_eq!(inherited.get_seqnum(), 0);
    assert_ne!(inherited.get_uuid(), before);
    assert!(inherited.with_previous(&previous).is_none());
}

#[test]
fn a_market_event_carries_what_its_chain_is_about_forward_and_folds_the_rest() {
    crate::install::installed();
    // An order naming its instrument, how long it stands and what the
    // market says about trading it.
    let mut order = operation(10);
    order.set_crosscode("O-100".to_owned());
    order.set_timeinforce(TimeInForce::from_spelling("GoodTillCancel"), true);
    order.set_tradable(Some(true), true);
    order.set_ticker(Some(SmolStr::new("BRN")), true);
    order
        .insert_securityid(securityid("ISIN", "US0378331005"))
        .expect("a plain holder");
    order.set_miccode(Some(Mic::new("XLON").expect("a MIC")), true);
    order.finalize();

    // The report that answers it names none of that, and states a price and
    // a quantity of its own.
    let mut report = OrderEvent::at(at(20));
    report.set_crosscode("O-100".to_owned());
    report.set_price(Some(Decimal::from_int(83)), true);
    report.set_quantity(Some(Decimal::from_int(400)), true);
    report.finalize();

    let followed = report.with_previous(&order).expect("the step after");
    // The step before, as the step before: a price beside the price it
    // moved from.
    assert_eq!(followed.get_prevpx(), Some(decimal("82.5")));
    assert_eq!(followed.get_prevqty(), Some(Decimal::from_int(1_000)));
    // And what the chain is about, where this report said nothing.
    assert_eq!(
        followed.get_timeinforce().map(|held| held.as_str()),
        Some("GTC")
    );
    assert_eq!(followed.get_tradable(), Some(true));
    assert_eq!(followed.get_ticker(), Some("BRN"));
    assert_eq!(followed.get_currency().as_str(), "USD");
    assert_eq!(followed.get_unit().as_str(), "bbl");
    assert_eq!(followed.get_side().as_str(), "BUYS");
    assert_eq!(
        followed.get_securityids().get(&IdType::Isin),
        Some("US0378331005")
    );
    assert_eq!(followed.get_miccode().map(Mic::as_str), Some("XLON"));
    // What this report does say is its own: the price it states is not the
    // one it followed.
    assert_eq!(followed.get_price(), Some(Decimal::from_int(83)));
    assert_eq!(followed.get_quantity(), Some(Decimal::from_int(400)));
    // A later instant keeps its own place.
    assert_eq!(followed.get_seqnum(), 0);

    // A statement of its own never gives way to the chain's.
    let mut own = OrderEvent::at(at(20));
    own.set_crosscode("O-100".to_owned());
    own.set_timeinforce(TimeInForce::from_spelling("ImmediateOrCancel"), true);
    own.set_tradable(Some(false), true);
    own.set_ticker(Some(SmolStr::new("WTI")), true);
    own.set_prevpx(Some(Decimal::from_int(1)), true);
    own.finalize();
    let followed = own.with_previous(&order).expect("the step after");
    assert_eq!(
        followed.get_timeinforce().map(|held| held.as_str()),
        Some("IOC")
    );
    assert_eq!(followed.get_tradable(), Some(false));
    assert_eq!(followed.get_ticker(), Some("WTI"));
    assert_eq!(followed.get_prevpx(), Some(Decimal::from_int(1)));

    // Merging folds the same facts the other way: two statements of one
    // event, the later leading where both state one and the other filling
    // what it leaves out.
    let mut first = operation(30);
    first.set_lastpx(Some(Decimal::from_int(80)), true);
    first.set_cumqty(Some(Decimal::from_int(100)), true);
    first.set_timeinforce(TimeInForce::from_spelling("Day"), true);
    first.finalize();
    let mut later = first.clone();
    later.set_transunix(at(40));
    later.set_lastpx(Some(Decimal::from_int(81)), true);
    later.set_cumqty(None, true);
    later.set_leavesqty(Some(Decimal::from_int(900)), true);
    later.set_avgpx(Some(decimal("80.5")), true);
    later.set_timeinforce(None, true);
    later.set_tradable(Some(true), true);
    later.set_ticker(Some(SmolStr::new("BRN")), true);
    later.set_uuid(first.get_uuid());
    let merged = first.merge_with(&later).expect("the same event");
    assert_eq!(merged.get_lastpx(), Some(Decimal::from_int(81)));
    assert_eq!(merged.get_cumqty(), Some(Decimal::from_int(100)));
    assert_eq!(merged.get_leavesqty(), Some(Decimal::from_int(900)));
    assert_eq!(merged.get_avgpx(), Some(decimal("80.5")));
    assert_eq!(
        merged.get_timeinforce().map(|held| held.as_str()),
        Some("DAY")
    );
    assert_eq!(merged.get_tradable(), Some(true));
    assert_eq!(merged.get_ticker(), Some("BRN"));
    assert_eq!(merged.get_transunix(), at(40));

    // Every one of them is part of what the event is, so two events that
    // differ only there answer to different codes.
    let mut halted = operation(50);
    halted.set_tradable(Some(false), true);
    halted.finalize();
    let mut trading = operation(50);
    trading.set_tradable(Some(true), true);
    trading.finalize();
    assert_ne!(halted.get_hashcode(), trading.get_hashcode());
    let mut named = trade(50);
    named.set_ticker(Some(SmolStr::new("BRN")), true);
    named.finalize();
    assert_ne!(named.get_hashcode(), trade(50).get_hashcode());
}

#[test]
fn an_isin_derives_only_the_deterministic_missing_identifiers() {
    crate::install::installed();
    let mut element = OrderEvent::default();
    element
        .insert_securityid(securityid("ISIN", "US0378331005"))
        .expect("a plain holder");
    element.finalize();
    assert_eq!(
        element.get_securityids().get(&IdType::Cusip),
        Some("037833100")
    );
    assert!(element.get_cficode().is_none());
    assert!(element.get_securityids().get(&IdType::Bloomberg).is_none());
    // A stated identifier stands: deriving fills only a missing key.
    let mut stated = OrderEvent::default();
    stated
        .insert_securityid(securityid("CUSIP", "594918104"))
        .expect("a plain holder");
    stated
        .insert_securityid(securityid("ISIN", "US0378331005"))
        .expect("a plain holder");
    stated.finalize();
    assert_eq!(
        stated.get_securityids().get(&IdType::Cusip),
        Some("594918104")
    );
    // Deriving is a fill that never writes through: the verb says whether
    // it filled, and answers nothing where the key is held.
    let mut derived = OrderEvent::default();
    assert!(derived.derive_securityid(&IdType::Cusip, "037833100"));
    assert!(!derived.derive_securityid(&IdType::Cusip, "594918104"));
    assert_eq!(
        derived.get_securityids().get(&IdType::Cusip),
        Some("037833100")
    );

    let mut event = OrderEvent::at(0);
    event
        .insert_securityid(securityid("ISIN", "US0378331005"))
        .expect("a plain holder");
    event.finalize();
    assert_eq!(
        event.get_securityids().get(&IdType::Cusip),
        Some("037833100")
    );
    // An ISIN of a country whose number the crate does not know carries
    // nothing it can derive.
    let mut foreign = OrderEvent::default();
    foreign
        .insert_securityid(securityid("ISIN", "FR0000131104"))
        .expect("a plain holder");
    foreign.finalize();
    assert_eq!(foreign.get_securityids().len(), 1);
}

/// An event dates its own execution from its instant where its state
/// reports one and it states no clock - whatever leaf holds it: an order's
/// report that filled is as much an execution report as an execution, so a
/// walk reading it alone dates it as following does.
#[test]
fn an_order_report_whose_state_reports_a_fill_dates_its_execution() {
    crate::install::installed();
    use yggdryl_market::graph::EventIterator;

    for state in [State::PartiallyFilled, filled()] {
        let mut report = stated(30);
        report.set_crosscode("ORD-1".to_owned());
        report.set_state(state);
        report.finalize();
        let walked: Vec<OrderEvent> = EventIterator::new([report], true).collect();
        assert_eq!(walked[0].get_execunix(), Some(at(30)), "{state:?}");
    }
    // A state reporting none dates nothing.
    let mut working = stated(30);
    working.set_crosscode("ORD-1".to_owned());
    working.set_state(State::New);
    working.finalize();
    let walked: Vec<OrderEvent> = EventIterator::new([working], true).collect();
    assert_eq!(walked[0].get_execunix(), None);
}
