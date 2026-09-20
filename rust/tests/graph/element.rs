//! An element states its identity, its cross identity, its codes, its names
//! and its parents; an event its instant, its state and the optional facts
//! of its lifecycle; a market element its price, quantity and side. The
//! traits are signatures and provided readings, so what a caller can rely
//! on is that a value implementing them answers through them, including as
//! a trait object, and that following, merging and syncing fold the
//! lifecycle the way the traits say - over the crate's own holders,
//! [`MarketElementData`] and [`MarketEventData`], and over a foreign type
//! that implements only the signatures.

use std::collections::BTreeMap;
use std::hash::Hasher;

use yggdryl::graph::{
    Element, Event, MarketElement, MarketElementData, MarketEvent, MarketEventData,
};
use yggdryl::xxhash::Xxh3;
use yggdryl::{
    BloombergCode, CfiCode, Currency, CusipCode, Decimal18, IsinCode, MicCode, SedolCode, Side,
    State, Uuid,
};

/// One report as a foreign caller would hold it: every fact the two traits
/// name, an identity assigned rather than derived, and nothing the graph
/// owns.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Report {
    curruuid: Uuid,
    crossuuid: Uuid,
    crosscode: String,
    hashcode: u64,
    crosshashcode: u64,
    identifiers: BTreeMap<String, String>,
    parents: Vec<Uuid>,
    sources: Vec<Uuid>,
    unix: i64,
    state: State,
    seqnum: u64,
    creaunix: Option<i64>,
    exprtime: Option<i64>,
    prevunix: Option<i64>,
    prevuuid: Option<Uuid>,
    snapunix: Option<i64>,
    /// Whether the identity derives from the instant and the content, which
    /// is what finalizing resets it to; an assigned identity keeps its code.
    derived: bool,
}

impl Report {
    fn at(uuid: u128, unix: i64) -> Self {
        Self {
            curruuid: Uuid::from_v8(uuid),
            crossuuid: Uuid::from_v8(uuid),
            crosscode: String::new(),
            hashcode: 0,
            crosshashcode: 0,
            identifiers: BTreeMap::new(),
            parents: Vec::new(),
            sources: Vec::new(),
            unix,
            state: State::from_spelling("New").expect("a shipped state"),
            seqnum: 0,
            creaunix: None,
            exprtime: None,
            prevunix: None,
            prevuuid: None,
            snapunix: None,
            derived: false,
        }
    }
}

impl Element for Report {
    fn get_curruuid(&self) -> Uuid {
        self.curruuid
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.curruuid = curruuid;
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

    fn get_currhashcode(&self) -> u64 {
        self.hashcode
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.hashcode = hashcode;
    }

    fn get_crosshashcode(&self) -> u64 {
        self.crosshashcode
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.crosshashcode = crosshashcode;
    }

    fn get_identifiers(&self) -> &BTreeMap<String, String> {
        &self.identifiers
    }

    fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) {
        self.identifiers = identifiers;
    }

    fn get_parentuuids(&self) -> &[Uuid] {
        &self.parents
    }

    fn set_parentuuids(&mut self, parents: Vec<Uuid>) {
        self.parents = parents;
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        &self.sources
    }

    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
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
    fn get_currunix(&self) -> i64 {
        self.unix
    }

    fn set_currunix(&mut self, unix: i64) {
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

    fn get_exprtime(&self) -> Option<i64> {
        self.exprtime
    }

    fn set_exprtime(&mut self, unix: Option<i64>) {
        self.exprtime = unix;
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

/// One nanosecond count per millisecond: the instants below are spaced so
/// two of them never share the microsecond a derived identity opens with.
const MS: i64 = 1_000_000;

/// An instant a derived identity holds: `ms` milliseconds after one
/// evening in November 2023, UTC.
fn at(ms: i64) -> i64 {
    1_700_000_000_000_000_000 + ms * MS
}

pub(crate) fn filled() -> State {
    State::from_spelling("Filled").expect("a shipped state")
}

fn identifiers<const N: usize>(pairs: [(&str, &str); N]) -> BTreeMap<String, String> {
    pairs
        .into_iter()
        .map(|(scheme, identifier)| (scheme.to_owned(), identifier.to_owned()))
        .collect()
}

/// The XXH3-64 of one cross code, as the trait derives it.
fn crosshash(crosscode: &str) -> u64 {
    let mut state = Xxh3::new();
    state.write(crosscode.as_bytes());
    state.as_u64()
}

/// One trade as the crate holds it: a buy of a thousand barrels at 82.5
/// dollars, stated and not yet finalized, so the lanes it implies are still
/// empty and a case about filling them starts from nothing.
fn stated(ms: i64) -> MarketEventData {
    let mut trade = MarketEventData::at(at(ms));
    trade.set_px(Decimal18::parse("82.5").expect("a decimal"));
    trade.set_currency(Currency::new("USD").expect("a currency"));
    trade.set_qty(Decimal18::from_int(1_000));
    trade.set_unit("bbl".to_owned());
    trade.set_side(Side::read("Buy").expect("a side"));
    trade
}

/// The same trade, finalized: its identity is what it states and when, and
/// the lane its side implies is filled, because finalizing fills.
fn trade(ms: i64) -> MarketEventData {
    let mut trade = stated(ms);
    trade.finalize();
    trade
}

#[test]
fn an_element_answers_the_identities_codes_and_parents_it_was_given() {
    let mut element = MarketElementData::default();
    assert_eq!(element.get_curruuid(), Uuid::default(), "no identity yet");
    assert_eq!(element.get_crossuuid(), Uuid::default());
    assert_eq!(
        element.get_crosscode(),
        "",
        "no cross code until it states one"
    );
    assert_eq!(
        (element.get_currhashcode(), element.get_crosshashcode()),
        (0, 0)
    );
    assert!(
        element.get_parentuuids().is_empty(),
        "a node naming no parent is a root"
    );

    element.set_curruuid(Uuid::from_v8(3));
    assert_eq!(element.get_curruuid(), Uuid::from_v8(3));
    element.set_crossuuid(Uuid::from_v8(30));
    assert_eq!(element.get_crossuuid(), Uuid::from_v8(30));
    element.set_currhashcode(0xDEAD_BEEF_CAFE_F00D);
    element.set_crosshashcode(0xBEEF);
    assert_eq!(element.get_currhashcode(), 0xDEAD_BEEF_CAFE_F00D);
    assert_eq!(element.get_crosshashcode(), 0xBEEF);

    // The cross code is text, and can be unsaid.
    element.set_crosscode("O-100".to_owned());
    assert_eq!(element.get_crosscode(), "O-100");
    element.set_crosscode(String::new());
    assert_eq!(element.get_crosscode(), "");

    // The order is the element's own and comes back as stated.
    let parents = vec![Uuid::from_v8(9), Uuid::from_v8(1)];
    element.set_parentuuids(parents.clone());
    assert_eq!(element.get_parentuuids(), parents.as_slice());

    // An empty list makes it a root again.
    element.set_parentuuids(Vec::new());
    assert!(element.get_parentuuids().is_empty());

    // The sources are the element's provenance, stated in the same way and
    // kept apart from its lineage: a new element was read from nothing.
    assert!(element.get_srcuuids().is_empty());
    let sources = vec![Uuid::from_v8(70), Uuid::from_v8(71)];
    element.set_srcuuids(sources.clone());
    assert_eq!(element.get_srcuuids(), sources.as_slice());
    assert!(
        element.get_parentuuids().is_empty(),
        "a source is no parent"
    );
    element.set_srcuuids(Vec::new());
    assert!(element.get_srcuuids().is_empty());

    // The lineage a follower takes: the parents, oldest first, then the
    // element itself, each once.
    element.set_curruuid(Uuid::from_v8(3));
    element.set_parentuuids(vec![Uuid::from_v8(9), Uuid::from_v8(3), Uuid::from_v8(1)]);
    assert_eq!(
        element.lineage(),
        [Uuid::from_v8(9), Uuid::from_v8(3), Uuid::from_v8(1)]
    );
    element.set_parentuuids(Vec::new());
    assert_eq!(element.lineage(), [Uuid::from_v8(3)]);
}

#[test]
fn an_element_goes_by_the_names_it_was_given_each_under_its_scheme() {
    let mut element = MarketElementData::default();
    assert!(
        element.get_identifiers().is_empty(),
        "no system named it yet"
    );
    element.set_identifiers(identifiers([("OrderID", "O-1"), ("ClOrdID", "C-1")]));
    assert_eq!(element.get_identifiers()["ClOrdID"], "C-1");
    assert_eq!(element.get_identifiers()["OrderID"], "O-1");
    // Held as text under text, in the scheme's order, so a walk over them is
    // the same walk whatever order they were stated in.
    assert_eq!(
        element.get_identifiers().keys().collect::<Vec<_>>(),
        ["ClOrdID", "OrderID"]
    );
    // The map is replaced whole, never merged.
    element.set_identifiers(identifiers([("ExecID", "E-1")]));
    assert_eq!(element.get_identifiers().len(), 1);
    assert_eq!(element.get_identifiers().get("ClOrdID"), None);
    // And a walk over trait objects reads them the same way.
    let held: &dyn Element = &element;
    assert_eq!(held.get_identifiers()["ExecID"], "E-1");
}

#[test]
fn sync_cross_forces_the_cross_codes_from_the_cross_code() {
    // An element stating a cross code: whatever the codes held, syncing
    // brings the cross hash code to the code's digest and the cross
    // element to the identity that digest derives.
    let mut report = Report::at(1, 10);
    report.set_crosscode("O-100".to_owned());
    report.set_crosshashcode(0xBEEF);
    report.set_crossuuid(Uuid::from_v8(99));
    assert!(report.sync_cross(), "both codes moved");
    assert_eq!(report.get_crosshashcode(), crosshash("O-100"));
    assert_ne!(report.get_crosshashcode(), 0);
    assert_eq!(
        report.get_crossuuid(),
        Uuid::from_v8(u128::from(crosshash("O-100")))
    );
    assert_eq!(report.get_crossuuid(), report.cross_uuid());
    assert!(!report.sync_cross(), "in step already: nothing moves");

    // Two elements sharing a cross code share the cross identity, whichever
    // type holds them.
    let mut element = MarketElementData::default();
    element.set_crosscode("O-100".to_owned());
    element.sync_cross();
    assert_eq!(element.get_crossuuid(), report.get_crossuuid());
    assert_eq!(element.get_crosshashcode(), report.get_crosshashcode());

    // No cross code: the digest is zero and the cross element is the
    // element's own identity, which it follows when that moves.
    report.set_crosscode(String::new());
    assert!(report.sync_cross());
    assert_eq!(report.get_crosshashcode(), 0);
    assert_eq!(report.get_crossuuid(), Uuid::from_v8(1));
    report.set_curruuid(Uuid::from_v8(2));
    assert_eq!(report.cross_uuid(), Uuid::from_v8(2));
    assert!(report.sync_cross());
    assert_eq!(report.get_crossuuid(), Uuid::from_v8(2));
    // A stale cross hash code alone is enough to move.
    report.set_crosshashcode(7);
    assert!(report.sync_cross());
    assert_eq!(report.get_crosshashcode(), 0);
}

#[test]
fn an_event_is_after_another_by_its_instant_and_an_element_by_its_lineage() {
    let earlier = Report::at(1, 10);
    let later = Report::at(2, 20);
    assert!(later.is_after(&earlier));
    assert!(earlier.is_before(&later));
    assert!(!earlier.is_after(&later));
    assert!(!later.is_before(&earlier));
    // Never after itself, and an equal instant is neither after nor before.
    assert!(!earlier.is_after(&earlier) && !earlier.is_before(&earlier));
    let same = Report::at(3, 10);
    assert!(!same.is_after(&earlier) && !same.is_before(&earlier));
    // A market event orders by its instant too.
    assert!(trade(20).is_after(&trade(10)));
    assert!(trade(10).is_before(&trade(20)));

    // A market element with no instant orders by lineage: it is after the
    // elements it descends from, and unrelated to everything else.
    let mut root = MarketElementData::default();
    root.set_crosscode("ROOT".to_owned());
    root.finalize();
    let mut child = MarketElementData::default();
    child.set_parentuuids(vec![root.get_curruuid()]);
    child.finalize();
    assert!(child.is_after(&root) && root.is_before(&child));
    assert!(!root.is_after(&child) && !child.is_before(&root));
    let stranger = MarketElementData::default();
    assert!(!stranger.is_after(&root) && !stranger.is_before(&root));
}

#[test]
fn an_event_answers_its_instant_state_and_place_and_is_still_an_element() {
    let mut event = MarketEventData::at(0);
    // Nanoseconds since the epoch, the count every clock the crate reads.
    event.set_currunix(i64::MAX);
    assert_eq!(event.get_currunix(), i64::MAX);
    // Negative instants are before the epoch, and the type holds them.
    event.set_currunix(-1);
    assert_eq!(event.get_currunix(), -1);
    event.set_currhashcode(0xDEAD_BEEF_CAFE_F00D);
    assert_eq!(event.get_currhashcode(), 0xDEAD_BEEF_CAFE_F00D);

    // A state is never absent: a new event reached none, says so with the
    // code that means exactly that, and moves as the lifecycle does.
    assert_eq!(event.get_state().as_str(), "00UNKNOWN");
    assert!(event.get_state().is_live(), "not ended, so still live");
    event.set_state(filled());
    assert!(event.get_state().is_done());
    assert!(
        event.get_state().as_str().ends_with("FILLED"),
        "{}",
        event.get_state().as_str()
    );
    assert!(
        event.get_state().rank().is_some(),
        "a shipped state sits on a rank"
    );
    assert_eq!(event.get_seqnum(), 0, "no place in a chain yet");
    event.set_seqnum(4);
    assert_eq!(event.get_seqnum(), 4);

    // One walk reads both traits through the subtrait object.
    event.set_curruuid(Uuid::from_v8(7));
    event.set_parentuuids(vec![Uuid::from_v8(1)]);
    let held: &dyn Event = &event;
    assert_eq!(held.get_curruuid(), Uuid::from_v8(7));
    assert_eq!(held.get_parentuuids(), [Uuid::from_v8(1)]);
    assert_eq!(held.get_currunix(), -1);
    assert_eq!(held.get_currhashcode(), 0xDEAD_BEEF_CAFE_F00D);
    assert_eq!(held.get_seqnum(), 4);
    assert!(held.get_state().is_done());
}

#[test]
fn the_optional_lifecycle_facts_are_stated_only_where_known() {
    let mut event = MarketEventData::at(40);
    assert_eq!(event.get_creaunix(), None);
    assert_eq!(event.get_exprtime(), None);
    assert_eq!(event.get_prevunix(), None);
    assert_eq!(event.get_prevuuid(), None);
    assert_eq!(event.get_snapunix(), None);

    event.set_creaunix(Some(35));
    event.set_exprtime(Some(100));
    event.set_prevunix(Some(30));
    event.set_prevuuid(Some(Uuid::from_v8(3)));
    event.set_snapunix(Some(40));
    assert_eq!(event.get_snapunix(), Some(40));
    assert_eq!(event.get_creaunix(), Some(35));
    assert_eq!(event.get_exprtime(), Some(100));
    assert_eq!(event.get_prevunix(), Some(30));
    assert_eq!(event.get_prevuuid(), Some(Uuid::from_v8(3)));

    // Each fact is unsaid on its own.
    event.set_exprtime(None);
    assert_eq!(event.get_exprtime(), None);
    assert_eq!(event.get_creaunix(), Some(35));
}

#[test]
fn following_records_the_predecessor_and_refuses_what_cannot_follow() {
    let first = Report::at(1, 10);
    let second = Report::at(2, 20)
        .with_previous(&first)
        .expect("a later element follows an earlier one");
    assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
    assert_eq!(second.get_prevunix(), Some(10));
    // The place in the chain is the one after the predecessor's, the
    // parents are the predecessor's lineage, and what the element itself
    // says stays: its instant.
    assert_eq!(second.get_seqnum(), 1);
    assert_eq!(second.get_parentuuids(), [first.get_curruuid()]);
    assert_eq!(second.get_currunix(), 20);
    // A chain of three ends with two parents, oldest first, the immediate
    // predecessor last; following what it already follows changes nothing.
    let third = Report::at(4, 30).with_previous(&second).expect("follows");
    assert_eq!(third.get_seqnum(), 2);
    assert_eq!(
        third.get_parentuuids(),
        [first.get_curruuid(), second.get_curruuid()]
    );
    assert_eq!(third.get_parentuuids(), second.lineage());
    assert!(third.clone().with_previous(&second).is_none());
    let mut deep = Report::at(5, 40);
    deep.set_seqnum(u64::MAX);
    let capped = Report::at(6, 50).with_previous(&deep).expect("follows");
    assert_eq!(
        capped.get_seqnum(),
        u64::MAX,
        "a place past the count saturates"
    );

    // The same instant follows: a predecessor is not later, and equal is not later.
    let same = Report::at(3, 10)
        .with_previous(&first)
        .expect("an equal instant follows");
    assert_eq!(same.get_prevuuid(), Some(first.get_curruuid()));

    // An element follows neither itself nor one that happened after it.
    assert!(Report::at(1, 10).with_previous(&first).is_none());
    assert!(Report::at(9, 5).with_previous(&second).is_none());

    // A later predecessor replaces the one recorded before.
    let third = second
        .clone()
        .with_previous(&Report::at(8, 15))
        .expect("a later predecessor still precedes");
    assert_eq!(third.get_prevuuid(), Some(Uuid::from_v8(8)));
    assert_eq!(third.get_prevunix(), Some(15));
}

#[test]
fn following_carries_the_lifecycle_forward() {
    // Earliest creation and furthest state carry forward; a newer stated
    // expiry replaces the prior deadline, including when it shortens it.
    let mut previous = Report::at(1, 10);
    previous.set_creaunix(Some(5));
    previous.set_exprtime(Some(200));
    previous.set_state(filled());
    let mut next = Report::at(2, 20);
    next.set_creaunix(Some(8));
    next.set_exprtime(Some(100));
    let next = next
        .with_previous(&previous)
        .expect("the later one follows");
    assert_eq!(next.get_creaunix(), Some(5));
    assert_eq!(next.get_exprtime(), Some(100));
    assert!(
        next.get_state().is_done(),
        "the furthest state carries forward"
    );

    // A previous that knows less leaves what the next one knows alone.
    let mut next = Report::at(3, 30);
    next.set_creaunix(Some(25));
    next.set_exprtime(Some(300));
    let bare = Report::at(4, 20);
    let next = next.with_previous(&bare).expect("follows");
    assert_eq!(next.get_creaunix(), Some(25));
    assert_eq!(next.get_exprtime(), Some(300));
    assert!(
        next.get_state().is_live(),
        "a lesser state does not move the next one back"
    );

    // And one that knows more fills what the next one did not state.
    let next = Report::at(5, 40).with_previous(&previous).expect("follows");
    assert_eq!(next.get_creaunix(), Some(5));
    assert_eq!(next.get_exprtime(), Some(200));

    // What the next element itself says moves nowhere: its instant, its
    // code, its sources, and the cross code it states where the
    // predecessor states none - with the cross element in step with it.
    // Its parents are the chain's: the predecessor's lineage replaces the
    // parent it named on its own. The predecessor's sources reach it not
    // at all, because provenance travels along no chain.
    let mut own = Report::at(6, 50);
    own.set_currhashcode(0xABC);
    own.set_crosscode("Q-1".to_owned());
    own.set_parentuuids(vec![Uuid::from_v8(61)]);
    own.set_srcuuids(vec![Uuid::from_v8(70)]);
    let mut previous = previous.clone();
    previous.set_srcuuids(vec![Uuid::from_v8(60)]);
    let own = own.with_previous(&previous).expect("follows");
    assert_eq!(own.get_currunix(), 50);
    assert_eq!(own.get_currhashcode(), 0xABC);
    assert_eq!(own.get_crosscode(), "Q-1");
    assert_eq!(own.get_crosshashcode(), crosshash("Q-1"));
    assert_eq!(own.get_crossuuid(), own.cross_uuid());
    assert_eq!(own.get_parentuuids(), [previous.get_curruuid()]);
    assert_eq!(own.get_srcuuids(), [Uuid::from_v8(70)]);

    // The names the predecessor went by carry forward where the next one
    // does not state them, and its own word stays where it does.
    let mut named = Report::at(7, 10);
    named.set_identifiers(identifiers([("ClOrdID", "C-1"), ("OrderID", "O-1")]));
    let mut next = Report::at(8, 20);
    next.set_identifiers(identifiers([("OrderID", "O-2")]));
    let next = next.with_previous(&named).expect("follows");
    assert_eq!(next.get_identifiers()["ClOrdID"], "C-1");
    assert_eq!(next.get_identifiers()["OrderID"], "O-2");
    assert_eq!(next.get_identifiers().len(), 2);
}

#[test]
fn following_adopts_the_predecessors_cross_code() {
    // Two events of one chain share the cross code, so the predecessor's
    // is forced onto the follower where its own differs - and the cross
    // hash code and the cross element follow it.
    let mut order = Report::at(1, 10);
    order.set_crosscode("O-100".to_owned());
    order.finalize();
    let mut report = Report::at(2, 20);
    report.set_crosscode("O-999".to_owned());
    report.finalize();
    assert_ne!(report.get_crossuuid(), order.get_crossuuid());
    let report = report.with_previous(&order).expect("follows");
    assert_eq!(report.get_crosscode(), "O-100");
    assert_eq!(report.get_crosshashcode(), crosshash("O-100"));
    assert_eq!(report.get_crossuuid(), order.get_crossuuid());
    // A follower stating none takes it the same way; one already sharing
    // it moves nothing on that account.
    let nameless = Report::at(3, 30).with_previous(&order).expect("follows");
    assert_eq!(nameless.get_crosscode(), "O-100");
    assert_eq!(nameless.get_crossuuid(), order.get_crossuuid());
    let mut shared = Report::at(4, 40);
    shared.set_crosscode("O-100".to_owned());
    shared.finalize();
    let shared = shared.with_previous(&order).expect("follows by its place");
    assert_eq!(shared.get_seqnum(), 1);
    assert_eq!(shared.get_crossuuid(), order.get_crossuuid());

    // The crate's own event does the same, and re-derives its identity.
    let mut placed = trade(10);
    placed.set_crosscode("O-100".to_owned());
    placed.finalize();
    let mut fill = trade(20);
    fill.set_crosscode("O-999".to_owned());
    fill.finalize();
    let before = fill.get_curruuid();
    let fill = fill.with_previous(&placed).expect("follows");
    assert_eq!(fill.get_crosscode(), "O-100");
    assert_eq!(fill.get_crossuuid(), placed.get_crossuuid());
    assert_eq!(fill.get_prevuuid(), Some(placed.get_curruuid()));
    assert_ne!(fill.get_curruuid(), before, "followed, so finalized");
    assert_eq!(fill.get_curruuid(), fill.time_uuid().expect("an identity"));

    // A market element follows by descending, and adopts the code too.
    let mut root = MarketElementData::default();
    root.set_crosscode("O-100".to_owned());
    root.finalize();
    let mut leaf = MarketElementData::default();
    leaf.set_crosscode("O-999".to_owned());
    leaf.finalize();
    let leaf = leaf.with_previous(&root).expect("descends");
    assert_eq!(leaf.get_parentuuids(), [root.get_curruuid()]);
    assert_eq!(leaf.get_crosscode(), "O-100");
    assert_eq!(leaf.get_crossuuid(), root.get_crossuuid());
    assert!(leaf.is_after(&root));
    assert!(leaf.clone().with_previous(&leaf).is_none(), "never itself");
    assert!(
        leaf.clone().with_previous(&root).is_none(),
        "descending again changes nothing"
    );
}

#[test]
fn restating_takes_the_live_elements_place_in_its_chain() {
    // The live element: second in its chain, named, rooted, filled.
    let first = Report::at(1, 10);
    let mut live = Report::at(2, 20);
    live.set_crosscode("O-100".to_owned());
    live.set_identifiers(identifiers([("ClOrdID", "C-1")]));
    live.set_parentuuids(vec![Uuid::from_v8(9)]);
    live.set_creaunix(Some(5));
    live.set_state(filled());
    let mut live = live.with_previous(&first).expect("follows");
    live.set_snapunix(Some(20));
    // Its twin, as another hop logged it: the same instant, its own
    // names, its own parents, a cross code of its own, a line of its own
    // and a lifecycle it knows less of.
    live.set_srcuuids(vec![Uuid::from_v8(70)]);
    let mut twin = Report::at(2, 20);
    twin.set_crosscode("O-999".to_owned());
    twin.set_identifiers(identifiers([("ExecID", "E-2"), ("ClOrdID", "C-9")]));
    twin.set_parentuuids(vec![Uuid::from_v8(8), Uuid::from_v8(9)]);
    twin.set_srcuuids(vec![Uuid::from_v8(71)]);
    twin.set_creaunix(Some(8));
    twin.set_exprtime(Some(99));
    let twin = twin.restating(&live);
    // The place the live one holds is the twin's: the chain grows by nothing.
    assert_eq!(twin.get_prevuuid(), Some(Uuid::from_v8(1)));
    assert_eq!(twin.get_prevunix(), Some(10));
    assert_eq!(twin.get_seqnum(), 1);
    assert_eq!(twin.get_snapunix(), Some(20));
    // The chain's cross code is forced, and its codes brought in step.
    assert_eq!(twin.get_crosscode(), "O-100");
    assert_eq!(twin.get_crosshashcode(), crosshash("O-100"));
    assert_eq!(twin.get_crossuuid(), live.get_crossuuid());
    // The names it lacks are taken, its own word kept; the parents are the
    // union in its order, then the live one's - which following gave the
    // chain's lineage. The sources stay the twin's own: the line it was
    // read from, never the live one's, because provenance travels along no
    // chain.
    assert_eq!(twin.get_identifiers()["ClOrdID"], "C-9");
    assert_eq!(twin.get_identifiers()["ExecID"], "E-2");
    assert_eq!(
        twin.get_parentuuids(),
        [Uuid::from_v8(8), Uuid::from_v8(9), first.get_curruuid()]
    );
    assert_eq!(twin.get_srcuuids(), [Uuid::from_v8(71)]);
    // The lifecycle folds: the earliest creation, the latest expiration,
    // the furthest state. What it says of itself - its instant - is its own.
    assert_eq!(twin.get_creaunix(), Some(5));
    assert_eq!(twin.get_exprtime(), Some(99));
    assert!(twin.get_state().is_done());
    assert_eq!(twin.get_currunix(), 20);

    // Two statements of one market event finalize to one identity: the
    // twin of a followed event restates it and is it.
    let placed = trade(10);
    let fill = trade(20);
    let twin = fill.clone();
    let fill = fill.with_previous(&placed).expect("follows");
    assert_ne!(
        twin.get_curruuid(),
        fill.get_curruuid(),
        "followed, so moved"
    );
    let twin = twin.restating(&fill);
    assert_eq!(twin, fill);
    assert_eq!(twin.get_curruuid(), fill.get_curruuid());
    assert_eq!(twin.get_curruuid(), twin.time_uuid().expect("an identity"));
}

#[test]
fn merging_folds_another_statement_of_the_same_element() {
    let mut first = Report::at(1, 10);
    first.set_currhashcode(0xA);
    first.set_parentuuids(vec![Uuid::from_v8(7)]);
    first.set_srcuuids(vec![Uuid::from_v8(70)]);
    first.set_creaunix(Some(9));
    first.set_prevuuid(Some(Uuid::from_v8(0)));
    first.set_prevunix(Some(1));

    let mut later = Report::at(1, 20);
    later.set_currhashcode(0xB);
    later.set_crosscode("O-10".to_owned());
    later.set_parentuuids(vec![Uuid::from_v8(8), Uuid::from_v8(7)]);
    later.set_srcuuids(vec![Uuid::from_v8(71), Uuid::from_v8(70)]);
    later.set_creaunix(Some(4));
    later.set_exprtime(Some(99));
    later.set_state(filled());
    later.set_prevuuid(Some(Uuid::from_v8(5)));
    later.set_prevunix(Some(6));
    later.finalize();

    // Another element does not merge at all.
    assert!(first.clone().merge_with(&Report::at(2, 10)).is_none());

    later.set_seqnum(3);
    let merged = first.clone().merge_with(&later).expect("the same element");
    // The later statement has the last word on the instant and the code,
    // and the further place in the chain stands.
    assert_eq!(merged.get_currunix(), 20);
    assert_eq!(merged.get_currhashcode(), 0xB);
    assert_eq!(merged.get_seqnum(), 3);
    // The cross code fills what this one left out, its digest and the
    // cross element in step; the parents are the union in this element's
    // order, then the other's.
    assert_eq!(merged.get_crosscode(), "O-10");
    assert_eq!(merged.get_crosshashcode(), crosshash("O-10"));
    assert_eq!(merged.get_crossuuid(), later.get_crossuuid());
    assert_eq!(
        merged.get_parentuuids(),
        [Uuid::from_v8(7), Uuid::from_v8(8)]
    );
    // The sources are the same union, once each: the merged statement was
    // read from both lines.
    assert_eq!(
        merged.get_srcuuids(),
        [Uuid::from_v8(70), Uuid::from_v8(71)]
    );
    // The lifecycle folds as following folds it.
    assert_eq!(merged.get_creaunix(), Some(4));
    assert_eq!(merged.get_exprtime(), Some(99));
    assert!(merged.get_state().is_done());
    // The predecessor is this element's where it names one.
    assert_eq!(merged.get_prevuuid(), Some(Uuid::from_v8(0)));
    assert_eq!(merged.get_prevunix(), Some(1));

    // Merged the other way, the earlier statement adds nothing the later
    // one lacks - its instant and code lose, the predecessor the later one
    // names is kept - so the fold answers nothing, and the later statement
    // stands as it was.
    assert!(later.clone().merge_with(&first).is_none());
    assert_eq!(later.get_currunix(), 20);
    assert_eq!(later.get_currhashcode(), 0xB);
    assert_eq!(later.get_crosscode(), "O-10");
    assert_eq!(
        later.get_parentuuids(),
        [Uuid::from_v8(8), Uuid::from_v8(7)]
    );
    assert_eq!(later.get_prevuuid(), Some(Uuid::from_v8(5)));

    // A statement naming no predecessor takes the other's.
    let mut silent = Report::at(1, 30);
    silent.set_currhashcode(0xC);
    let merged = silent.merge_with(&first).expect("the same element");
    assert_eq!(
        merged.get_currhashcode(),
        0xC,
        "the later statement's code stands"
    );
    assert_eq!(merged.get_prevuuid(), Some(Uuid::from_v8(0)));
    assert_eq!(merged.get_creaunix(), Some(9));
}

#[test]
fn a_walk_over_elements_reaches_a_root_by_identity() {
    // A caller resolves parents by identity, so a graph is a map of them.
    let mut root = Report::at(1, 10);
    root.set_state(filled());
    let mut child = Report::at(2, 20);
    child.set_parentuuids(vec![root.get_curruuid()]);
    let mut leaf = Report::at(3, 30);
    leaf.set_parentuuids(vec![child.get_curruuid()]);

    let held: Vec<Box<dyn Event>> = vec![Box::new(root), Box::new(child), Box::new(leaf.clone())];
    let by_uuid = |uuid: Uuid| held.iter().find(|element| element.get_curruuid() == uuid);

    let mut at: &dyn Event = &leaf;
    let mut lineage = vec![at.get_currunix()];
    while let Some(parent) = at.get_parentuuids().first().copied() {
        at = by_uuid(parent)
            .expect("a parent is an element of the graph")
            .as_ref();
        lineage.push(at.get_currunix());
    }
    assert_eq!(lineage, [30, 20, 10]);
    assert!(
        at.get_state().is_done(),
        "the root reached its terminal state"
    );
}

#[test]
fn the_instant_and_the_code_derive_one_time_ordered_identity() {
    let mut event = Report::at(1, at(0));
    event.set_currhashcode(0xCAFE);
    let held = event.txhash().expect("an instant a TxHash holds");
    assert_eq!(held.unix(), at(0));
    assert_eq!(held.digest().as_u64(), Some(0xCAFE));

    // The same instant and code derive the same identity, every time.
    let identity = event.time_uuid().expect("an identity");
    assert_eq!(event.time_uuid().expect("an identity"), identity);
    assert_eq!(identity, held.into_uuid().expect("the TxHash's own UUID"));

    // A later instant sorts later whatever the code, and the same instant
    // sorts by code: the instant is in front, as a UUIDv7's is.
    let mut later = event.clone();
    later.set_currunix(at(0) + 1_000);
    later.set_currhashcode(0);
    assert!(later.time_uuid().expect("an identity") > identity);
    let mut sibling = event.clone();
    sibling.set_currhashcode(0xCAFF);
    assert_ne!(sibling.time_uuid().expect("an identity"), identity);

    // Every instant the count holds - the last one is in 2262 - a UUIDv7
    // holds too; one before the epoch has no UUIDv7 to derive.
    let mut far = event.clone();
    far.set_currunix(i64::MAX);
    assert!(far.txhash().is_ok());
    assert!(far.time_uuid().expect("an identity") > identity);
    let mut before = event.clone();
    before.set_currunix(-1);
    assert!(before.txhash().is_ok());
    assert!(before.time_uuid().is_err());

    // Finalized with a code, an event's identity is the one they derive
    // and its cross element the identity itself where it states no cross
    // code; the cross code's own identity where it does.
    let mut finalized = event.clone();
    finalized.finalized(0xCAFE);
    assert_eq!(finalized.get_currhashcode(), 0xCAFE);
    assert_eq!(finalized.get_curruuid(), identity);
    assert_eq!(finalized.get_crossuuid(), identity);
    finalized.set_crosscode("O-1".to_owned());
    finalized.sync_cross();
    finalized.finalized(0xCAFE);
    assert_eq!(finalized.get_curruuid(), identity);
    assert_eq!(
        finalized.get_crossuuid(),
        Uuid::from_v8(u128::from(crosshash("O-1")))
    );
    // An instant a UUIDv7 cannot hold leaves the identity as it was.
    before.finalized(0xCAFE);
    assert_eq!(before.get_curruuid(), Uuid::from_v8(1));
}

#[test]
fn a_market_element_names_its_instrument_the_way_the_market_does() {
    let mut held = trade(10);
    for absent in [
        held.get_isincode().is_none(),
        held.get_cusipcode().is_none(),
        held.get_sedolcode().is_none(),
        held.get_bloombergcode().is_none(),
        held.get_cficode().is_none(),
        held.get_miccode().is_none(),
    ] {
        assert!(
            absent,
            "an instrument is named only where the market names it"
        );
    }
    held.set_isincode(Some(IsinCode::new("US0378331005").expect("an ISIN")));
    held.set_cusipcode(Some(CusipCode::new("037833100").expect("a CUSIP")));
    held.set_sedolcode(Some(SedolCode::new("B0YBKJ7").expect("a SEDOL")));
    held.set_bloombergcode(Some(
        BloombergCode::new("AAPL US EQUITY").expect("a BloombergCode identifier"),
    ));
    held.set_cficode(Some(CfiCode::new("ESVUFR").expect("a CFI")));
    held.set_miccode(Some(MicCode::new("XPAR").expect("a MIC")));
    assert_eq!(
        held.get_isincode().map(IsinCode::as_str),
        Some("US0378331005")
    );
    assert_eq!(
        held.get_cusipcode().map(CusipCode::as_str),
        Some("037833100")
    );
    assert_eq!(held.get_sedolcode().map(SedolCode::as_str), Some("B0YBKJ7"));
    assert_eq!(
        held.get_bloombergcode().map(BloombergCode::as_str),
        Some("AAPL US EQUITY")
    );
    assert_eq!(held.get_cficode().map(CfiCode::as_str), Some("ESVUFR"));
    assert_eq!(held.get_miccode().map(MicCode::as_str), Some("XPAR"));
    // Each is unsaid on its own.
    held.set_cusipcode(None);
    assert!(held.get_cusipcode().is_none());
    assert!(held.get_isincode().is_some());
    // The codes are the crate's own: a spelling that is no identifier never
    // reaches the element.
    assert!(
        IsinCode::new("US0378331006").is_err(),
        "a wrong check digit is no ISIN"
    );
}

#[test]
fn a_market_element_answers_its_five_facts_and_is_still_an_event() {
    let mut held = trade(10);
    assert_eq!(held.get_px().to_string(), "82.5");
    assert_eq!(held.get_currency().as_str(), "USD");
    assert_eq!(held.get_qty(), Decimal18::from_int(1_000));
    assert_eq!(held.get_unit(), "bbl");
    assert_eq!(held.get_side().as_str(), "BUY");

    held.set_px(Decimal18::from_int(83));
    held.set_currency(Currency::new("EUR").expect("a currency"));
    held.set_qty(Decimal18::ZERO);
    held.set_unit("MWh".to_owned());
    held.set_side(Side::read("2").expect("a side"));
    assert_eq!(held.get_px(), Decimal18::from_int(83));
    assert_eq!(held.get_currency().as_str(), "EUR");
    assert_eq!(held.get_qty(), Decimal18::ZERO);
    assert_eq!(held.get_unit(), "MWh");
    assert_eq!(held.get_side().as_str(), "SELL");

    // A new element states nothing: no price, no currency, no unit, no side.
    let bare = MarketElementData::default();
    assert_eq!(
        (bare.get_px(), bare.get_qty()),
        (Decimal18::ZERO, Decimal18::ZERO)
    );
    assert_eq!(bare.get_currency(), &Currency::none());
    assert_eq!(bare.get_unit(), "");
    assert_eq!(bare.get_side(), &Side::unknown());

    // One walk reads all three traits through the market event object, and
    // the timed readings are the market event's too.
    let next = trade(20)
        .with_previous(&held)
        .expect("a later trade follows");
    let object: &dyn MarketEvent = &next;
    assert_eq!(
        object.get_curruuid(),
        next.time_uuid().expect("an identity")
    );
    assert_eq!(object.get_currunix(), at(20));
    assert_eq!(object.get_seqnum(), 1);
    assert_eq!(object.get_prevuuid(), Some(held.get_curruuid()));
    assert_eq!(
        object.get_px().to_string(),
        "82.5",
        "what the trade itself says moves nowhere"
    );
}

#[test]
fn merging_a_market_event_takes_the_later_statement_and_the_better_codes() {
    let mut first = trade(10);
    first.set_currency(Currency::none());
    first.set_side(Side::unknown());
    first.set_cficode(Some(CfiCode::new("ESXXXR").expect("a CFI")));
    first.set_isincode(Some(IsinCode::new("US0378331005").expect("an ISIN")));
    first.finalize();
    // A later statement of the same trade: the identity kept, the instant
    // and the facts restated.
    let mut later = first.clone();
    later.set_currunix(at(20));
    later.set_px(Decimal18::from_int(83));
    later.set_qty(Decimal18::from_int(5));
    later.set_unit("MWh".to_owned());
    later.set_currency(Currency::new("EUR").expect("a currency"));
    later.set_side(Side::read("2").expect("a side"));
    later.set_cficode(Some(CfiCode::new("ESVUFX").expect("a CFI")));
    later.set_miccode(Some(MicCode::new("XPAR").expect("a MIC")));

    // The later statement has the last word on the market's facts, and each
    // code is the better of the two: the earlier fills what the later left
    // unknown, and a code only one statement names is that one's.
    let merged = first.clone().merge_with(&later).expect("the same trade");
    assert_eq!(merged.get_currunix(), at(20));
    assert_eq!(
        (merged.get_px(), merged.get_qty(), merged.get_unit()),
        (Decimal18::from_int(83), Decimal18::from_int(5), "MWh")
    );
    assert_eq!(merged.get_currency().as_str(), "EUR");
    assert_eq!(merged.get_side().as_str(), "SELL");
    assert_eq!(merged.get_cficode().map(CfiCode::as_str), Some("ESVUFR"));
    assert_eq!(
        merged.get_isincode().map(IsinCode::as_str),
        Some("US0378331005")
    );
    assert_eq!(merged.get_miccode().map(MicCode::as_str), Some("XPAR"));
    // Merged, the event is finalized: its identity is what it now says.
    assert_eq!(
        merged.get_curruuid(),
        merged.time_uuid().expect("an identity")
    );
    assert_ne!(merged.get_curruuid(), first.get_curruuid());

    // Merged the other way round the later statement still leads, so the
    // reading does not depend on which statement a caller held.
    let merged = later.clone().merge_with(&first).expect("the same trade");
    assert_eq!(
        (merged.get_px(), merged.get_currency().as_str()),
        (Decimal18::from_int(83), "EUR")
    );
    assert_eq!(merged.get_cficode().map(CfiCode::as_str), Some("ESVUFR"));
    assert_eq!(
        merged.get_isincode().map(IsinCode::as_str),
        Some("US0378331005")
    );

    // A statement that knows a code the later one states as none keeps
    // its own: the later statement leads, and unknown takes the other.
    let mut bare = later.clone();
    bare.set_currunix(at(30));
    bare.set_currency(Currency::none());
    let merged = later.clone().merge_with(&bare).expect("the same trade");
    assert_eq!(merged.get_currency().as_str(), "EUR");
    assert_eq!(merged.get_currunix(), at(30));

    // Another trade does not merge at all.
    assert!(first.merge_with(&trade(30)).is_none());
}

#[test]
fn merging_a_market_element_lets_this_statement_lead() {
    // With no instant to say which statement is later, this one leads:
    // its price, quantity and unit stand, and each code is the better of
    // the two with this one first.
    let mut this = MarketElementData::default();
    this.set_crosscode("T-1".to_owned());
    this.set_px(Decimal18::parse("82.5").expect("a decimal"));
    this.set_qty(Decimal18::from_int(1_000));
    this.set_cficode(Some(CfiCode::new("ESXXXR").expect("a CFI")));
    this.finalize();
    let mut other = this.clone();
    other.set_px(Decimal18::from_int(83));
    other.set_unit("bbl".to_owned());
    other.set_currency(Currency::new("USD").expect("a currency"));
    other.set_side(Side::read("1").expect("a side"));
    other.set_cficode(Some(CfiCode::new("ESVUFR").expect("a CFI")));
    other.set_identifiers(identifiers([("ClOrdID", "C-1")]));
    let merged = this.clone().merge_with(&other).expect("the same element");
    assert_eq!(merged.get_px().to_string(), "82.5");
    assert_eq!(
        merged.get_unit(),
        "",
        "this element's unit stands, stated or not"
    );
    assert_eq!(
        merged.get_currency().as_str(),
        "USD",
        "unknown takes the other"
    );
    assert_eq!(merged.get_side().as_str(), "BUY");
    assert_eq!(merged.get_cficode().map(CfiCode::as_str), Some("ESVUFR"));
    assert_eq!(merged.get_identifiers()["ClOrdID"], "C-1");
    // Finalized: the identity is what the merged element states.
    assert_eq!(
        merged.get_curruuid(),
        Uuid::from_v8(u128::from(merged.get_currhashcode()))
    );
    assert_ne!(merged.get_curruuid(), this.get_curruuid());
    // Another element does not merge, and nothing new answers nothing.
    let mut stranger = MarketElementData::default();
    stranger.set_crosscode("T-2".to_owned());
    stranger.finalize();
    assert!(this.clone().merge_with(&stranger).is_none());
    assert!(this.clone().merge_with(&this).is_none());
}

#[test]
fn a_reading_that_changes_nothing_answers_nothing_and_a_changed_element_is_finalized() {
    // Following what the element already follows changes nothing, so the
    // reading answers nothing and a caller skips what it holds.
    let first = Report::at(1, 10);
    let second = Report::at(2, 20).with_previous(&first).expect("follows");
    assert!(second.clone().with_previous(&first).is_none());
    // Merging a statement that adds nothing answers nothing too.
    assert!(second.clone().merge_with(&second).is_none());
    let mut bare = Report::at(2, 20);
    bare.set_prevuuid(second.get_prevuuid());
    assert!(second.clone().merge_with(&bare).is_none());
    // And a statement that adds one thing answers the element with it.
    let mut named = Report::at(2, 20);
    named.set_identifiers(identifiers([("ClOrdID", "C-1")]));
    let merged = second.clone().merge_with(&named).expect("one name taken");
    assert_eq!(merged.get_identifiers()["ClOrdID"], "C-1");

    // An identity assigned stays through a change; one derived from the
    // instant and the content is reset to what they now derive.
    assert_eq!(second.get_curruuid(), Uuid::from_v8(2));
    let mut derived = Report::at(3, at(30));
    derived.derived = true;
    derived.finalize();
    let before = derived.get_curruuid();
    assert_ne!(before, Uuid::from_v8(3));
    let followed = derived.with_previous(&second).expect("follows");
    assert_ne!(followed.get_curruuid(), before, "its place moved its code");
    assert_eq!(
        followed.get_curruuid(),
        followed.time_uuid().expect("an identity")
    );
    assert_eq!(
        followed.get_currhashcode(),
        followed.digest_event().as_u64()
    );
    // The market reading finalizes the same way.
    let trade = trade(40);
    let mut later = trade.clone();
    later.set_currunix(at(50));
    let merged = trade
        .clone()
        .merge_with(&later)
        .expect("the same trade, later");
    assert_eq!(merged.get_currunix(), at(50));
    assert_eq!(
        merged.get_curruuid(),
        merged.time_uuid().expect("an identity")
    );
    let same = trade.clone();
    assert!(trade.merge_with(&same).is_none(), "nothing moved");
}

#[test]
fn the_lane_the_side_implies_fills_from_the_elements_own_facts() {
    // A buy is a bid: the bid lane takes the price, the currency, the
    // quantity and the unit, and the ask lane stays empty.
    let mut buy = trade(10);
    buy.fill_lanes();
    assert_eq!(
        buy.get_bidpx(),
        Some(Decimal18::parse("82.5").expect("a decimal"))
    );
    assert_eq!(buy.get_bidcurrency().map(Currency::as_str), Some("USD"));
    assert_eq!(buy.get_bidqty(), Some(Decimal18::from_int(1_000)));
    assert_eq!(buy.get_bidunit(), Some("bbl"));
    assert_eq!((buy.get_askpx(), buy.get_askqty()), (None, None));
    assert!(buy.get_askcurrency().is_none() && buy.get_askunit().is_none());

    // A sell short is an ask, and what the lane already states stands.
    let mut sell = stated(20);
    sell.set_side(Side::read("SellShort").expect("a side"));
    sell.set_askpx(Some(Decimal18::from_int(90)));
    sell.fill_lanes();
    assert_eq!(sell.get_askpx(), Some(Decimal18::from_int(90)));
    assert_eq!(sell.get_askqty(), Some(Decimal18::from_int(1_000)));
    assert_eq!(sell.get_askunit(), Some("bbl"));
    assert_eq!(sell.get_bidpx(), None);

    // A cross takes neither lane, and a side stated as none neither.
    for spelling in ["Cross", "Opposite", "UNKNOWN"] {
        let mut neither = stated(30);
        neither.set_side(Side::read(spelling).expect("a side"));
        neither.fill_lanes();
        assert_eq!(
            (neither.get_bidpx(), neither.get_askpx()),
            (None, None),
            "{spelling}"
        );
    }

    // Only a stated fact fills a lane: a price or a quantity of nothing, no
    // currency and no unit are nothing to state on the lane either.
    let mut unstated = MarketElementData::default();
    unstated.set_side(Side::read("Buy").expect("a side"));
    unstated.fill_lanes();
    assert_eq!((unstated.get_bidpx(), unstated.get_bidqty()), (None, None));
    assert!(unstated.get_bidcurrency().is_none() && unstated.get_bidunit().is_none());
    unstated.set_qty(Decimal18::from_int(5));
    unstated.fill_lanes();
    assert_eq!(unstated.get_bidqty(), Some(Decimal18::from_int(5)));
    assert_eq!(unstated.get_bidpx(), None, "still no price to state");

    // Lanes merge as the market's facts do: the later statement's where it
    // states one, else this one's.
    let mut quoted = trade(40);
    quoted.set_bidpx(Some(Decimal18::from_int(80)));
    let mut later = quoted.clone();
    later.set_currunix(at(50));
    later.set_bidpx(None);
    later.set_askpx(Some(Decimal18::from_int(85)));
    let merged = quoted.merge_with(&later).expect("the same quote");
    assert_eq!(merged.get_bidpx(), Some(Decimal18::from_int(80)));
    assert_eq!(merged.get_askpx(), Some(Decimal18::from_int(85)));
}

#[test]
fn the_digest_starts_from_what_an_element_states_and_never_from_when() {
    let mut event = Report::at(1, 10);
    event.set_identifiers(identifiers([("ClOrdID", "C-1")]));
    let same = event.clone();
    let code = |event: &Report| event.digest_event().as_u64();
    // Two elements stating the same things digest alike, whatever the
    // instant, the identity, the codes or the clocks around them.
    let mut moved = same.clone();
    moved.set_currunix(99);
    moved.set_curruuid(Uuid::from_v8(7));
    moved.set_currhashcode(0xAB);
    moved.set_crosshashcode(0xCD);
    moved.set_crossuuid(Uuid::from_v8(77));
    moved.set_srcuuids(vec![Uuid::from_v8(70)]);
    moved.set_creaunix(Some(1));
    moved.set_exprtime(Some(200));
    moved.set_snapunix(Some(10));
    assert_eq!(code(&event), code(&moved));
    // What an element states moves the code: a cross code, a name, a
    // parent, the state, the place in its chain, the predecessor.
    let mut crossed = same.clone();
    crossed.set_crosscode("O-1".to_owned());
    assert_ne!(code(&event), code(&crossed));
    let mut named = same.clone();
    named.set_identifiers(identifiers([("ClOrdID", "C-2")]));
    assert_ne!(code(&event), code(&named));
    let mut rooted = same.clone();
    rooted.set_parentuuids(vec![Uuid::from_v8(9)]);
    assert_ne!(code(&event), code(&rooted));
    let mut done = same.clone();
    done.set_state(filled());
    assert_ne!(code(&event), code(&done));
    let followed = same
        .clone()
        .with_previous(&Report::at(3, 5))
        .expect("follows");
    assert_ne!(code(&event), code(&followed));
    // An implementor feeds its own content behind and reads the code; the
    // element-level digest is where an event's starts.
    let mut own = event.digest_event();
    own.write(b"body");
    assert_ne!(own.as_u64(), code(&event));
    assert_ne!(event.digest().as_u64(), code(&event));
    // A market element continues with its facts: a price moves the code,
    // and the event's digest continues the element's with its own.
    let trade = trade(10);
    let mut repriced = trade.clone();
    repriced.set_px(Decimal18::from_int(90));
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
    let element = MarketElementData::from(&trade);
    assert_eq!(
        element.digest_market().as_u64(),
        trade.digest_market().as_u64()
    );
}

#[test]
fn the_identity_never_reads_the_cross_hash_the_cross_element_or_a_source() {
    // The code and the identity are what an element states and when: the
    // cross hash code and the cross element are derived from the cross
    // code and a source is where the element was read, so none of the
    // three is an input to either, on the crate's holder or on an
    // implementor of the signatures alone.
    let stated = trade(10);
    let mut crossed = stated.clone();
    crossed.set_crosshashcode(0xCD);
    crossed.set_crossuuid(Uuid::from_v8(77));
    crossed.set_srcuuids(vec![Uuid::from_v8(70)]);
    assert_eq!(
        crossed.digest_market_event().as_u64(),
        stated.digest_market_event().as_u64()
    );
    assert_eq!(
        crossed.time_uuid().expect("an identity"),
        stated.time_uuid().expect("an identity")
    );
    // Finalized, the identity is the same and the cross facts are back in
    // step with the cross code, which states none.
    let mut finalized = crossed.clone();
    finalized.finalize();
    assert_eq!(finalized.get_curruuid(), stated.get_curruuid());
    assert_eq!(finalized.get_currhashcode(), stated.get_currhashcode());
    assert_eq!(finalized.get_crosshashcode(), 0);
    assert_eq!(finalized.get_crossuuid(), finalized.get_curruuid());
    assert_eq!(
        finalized.get_srcuuids(),
        [Uuid::from_v8(70)],
        "kept, not fed"
    );

    let report = Report::at(1, 10);
    let mut crossed = report.clone();
    crossed.set_crosshashcode(0xCD);
    crossed.set_crossuuid(Uuid::from_v8(77));
    crossed.set_srcuuids(vec![Uuid::from_v8(70)]);
    assert_eq!(
        crossed.digest_event().as_u64(),
        report.digest_event().as_u64()
    );
    assert_eq!(
        crossed.time_uuid().expect("an identity"),
        report.time_uuid().expect("an identity")
    );
}

#[test]
fn merging_two_incarnations_of_one_identity_unions_their_lineages_once() {
    // Two incarnations of one identity, each walked behind a chain of its
    // own: merged, the lineage is the union in this one's order, then the
    // other's, each identity once, and merged again it moves nothing.
    let root = Report::at(1, 10);
    let branch = Report::at(2, 15).with_previous(&root).expect("follows");
    let mut left = Report::at(5, 20).with_previous(&branch).expect("follows");
    left.set_curruuid(Uuid::from_v8(5));
    let other = Report::at(3, 12).with_previous(&root).expect("follows");
    let mut right = Report::at(5, 30).with_previous(&other).expect("follows");
    right.set_curruuid(Uuid::from_v8(5));
    assert_eq!(
        left.get_parentuuids(),
        [root.get_curruuid(), branch.get_curruuid()]
    );
    assert_eq!(
        right.get_parentuuids(),
        [root.get_curruuid(), other.get_curruuid()]
    );
    let merged = left.clone().merge_with(&right).expect("the same element");
    assert_eq!(
        merged.get_parentuuids(),
        [
            root.get_curruuid(),
            branch.get_curruuid(),
            other.get_curruuid()
        ]
    );
    // Each identity once: the same statement folded again changes nothing,
    // and a fold that changes nothing is no fold.
    assert!(
        merged.clone().merge_with(&right).is_none(),
        "a second merge changes nothing"
    );
}

#[test]
fn the_crates_own_holders_derive_their_identity_from_what_they_state() {
    // A market element's identity is its content: RFC 9562 UUIDv8 over the
    // code, so two elements stating the same things are one identity.
    let mut element = MarketElementData::default();
    element.set_crosscode("O-100".to_owned());
    element.set_px(Decimal18::parse("82.5").expect("a decimal"));
    element.set_side(Side::read("Buy").expect("a side"));
    element.finalize();
    assert_ne!(element.get_currhashcode(), 0);
    assert_eq!(element.get_currhashcode(), element.digest_market().as_u64());
    assert_eq!(
        element.get_curruuid(),
        Uuid::from_v8(u128::from(element.get_currhashcode()))
    );
    assert_eq!(element.get_crosshashcode(), crosshash("O-100"));
    assert_eq!(element.get_crossuuid(), element.cross_uuid());
    let mut same = element.clone();
    same.set_curruuid(Uuid::from_v8(1));
    same.set_currhashcode(0);
    same.finalize();
    assert_eq!(same, element);
    // Without a cross code the cross element is the identity itself.
    let mut alone = MarketElementData::default();
    alone.finalize();
    assert_eq!(alone.get_crossuuid(), alone.get_curruuid());
    assert_ne!(alone.get_curruuid(), Uuid::default());

    // A market event's identity is its instant and its content: UUIDv7,
    // so the same facts at another instant are another identity.
    let event = trade(10);
    assert_eq!(
        event.get_curruuid(),
        event.time_uuid().expect("an identity")
    );
    assert_eq!(
        event.get_currhashcode(),
        event.digest_market_event().as_u64()
    );
    assert_eq!(event.get_crossuuid(), event.get_curruuid(), "no cross code");
    assert_eq!(trade(10), event);
    assert_ne!(trade(11).get_curruuid(), event.get_curruuid());
    assert_eq!(trade(11).get_currhashcode(), event.get_currhashcode());
}

#[test]
fn the_market_element_and_the_market_event_convert_into_each_other() {
    let mut event = trade(10);
    event.set_crosscode("O-100".to_owned());
    event.set_identifiers(identifiers([("ClOrdID", "C-1")]));
    event.set_parentuuids(vec![Uuid::from_v8(9)]);
    event.set_srcuuids(vec![Uuid::from_v8(70)]);
    event.set_state(filled());
    event.set_seqnum(3);
    event.set_creaunix(Some(at(5)));
    event.set_exprtime(Some(at(99)));
    event.set_prevuuid(Some(Uuid::from_v8(8)));
    event.set_prevunix(Some(at(8)));
    event.set_snapunix(Some(at(10)));
    event.set_isincode(Some(IsinCode::new("US0378331005").expect("an ISIN")));
    event.fill_lanes();
    event.finalize();

    // Through the signatures the two share: every element and market fact
    // copied, the event's instants left behind.
    let element = MarketElementData::from(&event);
    assert_eq!(element.get_curruuid(), event.get_curruuid());
    assert_eq!(element.get_crossuuid(), event.get_crossuuid());
    assert_eq!(element.get_crosscode(), "O-100");
    assert_eq!(element.get_currhashcode(), event.get_currhashcode());
    assert_eq!(element.get_crosshashcode(), event.get_crosshashcode());
    assert_eq!(element.get_identifiers(), event.get_identifiers());
    assert_eq!(element.get_parentuuids(), event.get_parentuuids());
    assert_eq!(element.get_srcuuids(), [Uuid::from_v8(70)]);
    assert_eq!(element.get_px(), event.get_px());
    assert_eq!(element.get_currency(), event.get_currency());
    assert_eq!(element.get_qty(), event.get_qty());
    assert_eq!(element.get_unit(), event.get_unit());
    assert_eq!(element.get_side(), event.get_side());
    assert_eq!(element.get_isincode(), event.get_isincode());
    assert_eq!(element.get_bidpx(), event.get_bidpx());
    assert_eq!(element.get_bidunit(), event.get_bidunit());
    // Moved, the same element results.
    assert_eq!(MarketElementData::from(event.clone()), element);
    // And an event from an event is the event.
    assert_eq!(MarketEventData::from(&event), event);

    // Back: the element at the epoch, stating what it states and no
    // instant, no state, no place, no lifecycle.
    let back = MarketEventData::from(element.clone());
    assert_eq!(back.get_currunix(), 0);
    assert_eq!(back.get_state(), &State::unknown());
    assert_eq!(back.get_seqnum(), 0);
    assert_eq!((back.get_creaunix(), back.get_exprtime()), (None, None));
    assert_eq!((back.get_prevuuid(), back.get_prevunix()), (None, None));
    assert_eq!(back.get_snapunix(), None);
    assert_eq!(back.get_curruuid(), element.get_curruuid());
    assert_eq!(back.get_crosscode(), "O-100");
    assert_eq!(back.get_srcuuids(), [Uuid::from_v8(70)]);
    assert_eq!(back.get_px(), event.get_px());
    assert_eq!(back.get_isincode(), event.get_isincode());
    assert_eq!(MarketElementData::from(&back), element);
    // Dated and finalized, its identity is what the instant and the shared
    // facts derive - and back again, the element's own.
    let mut dated = back;
    dated.set_currunix(at(10));
    dated.finalize();
    assert_eq!(
        dated.get_curruuid(),
        dated.time_uuid().expect("an identity")
    );
    let mut again = MarketElementData::from(&dated);
    again.finalize();
    let mut expected = element.clone();
    expected.finalize();
    assert_eq!(again.get_curruuid(), expected.get_curruuid());
}

#[test]
fn filling_settles_the_price_and_the_quantity_down_one_ladder_each() {
    // What a report says about a trade and nothing about an order: the
    // price it is about is the price it traded at, and the quantity the
    // quantity it traded.
    let mut fill = MarketEventData::at(at(10));
    fill.set_side(Side::read("Buy").expect("a side"));
    fill.set_lastpx(Some(Decimal18::parse("82.5").expect("a decimal")));
    fill.set_lastqty(Some(Decimal18::from_int(300)));
    fill.set_avgpx(Some(Decimal18::parse("82.25").expect("a decimal")));
    fill.fill_market();
    assert_eq!(fill.get_px(), Decimal18::parse("82.5").expect("a decimal"));
    assert_eq!(fill.get_qty(), Decimal18::from_int(300));
    // And what it settled on reaches the lane its side implies.
    assert_eq!(fill.get_bidpx(), Some(Decimal18::parse("82.5").unwrap()));
    assert_eq!(fill.get_bidqty(), Some(Decimal18::from_int(300)));

    // The average stands in where nothing traded under this message.
    let mut averaged = MarketEventData::at(at(20));
    averaged.set_avgpx(Some(Decimal18::from_int(99)));
    averaged.fill_market();
    assert_eq!(averaged.get_px(), Decimal18::from_int(99));

    // How much is done and how much is left are not on the quantity's
    // ladder: together they are the quantity ordered, which is a rule the
    // dictionary states and this never restates.
    let mut working = MarketEventData::at(at(30));
    working.set_cumqty(Some(Decimal18::from_int(40)));
    working.set_leavesqty(Some(Decimal18::from_int(60)));
    working.fill_market();
    assert_eq!(working.get_qty(), Decimal18::ZERO);

    // A quote states only its lanes, and the side says which one it is
    // about. Nothing is invented for a side that takes neither.
    let mut quote = MarketEventData::at(at(40));
    quote.set_side(Side::read("Sell").expect("a side"));
    quote.set_askpx(Some(Decimal18::from_int(85)));
    quote.set_askqty(Some(Decimal18::from_int(7)));
    quote.set_askcurrency(Some(Currency::new("EUR").expect("a currency")));
    quote.set_askunit(Some("mt".to_owned()));
    quote.fill_market();
    assert_eq!(quote.get_px(), Decimal18::from_int(85));
    assert_eq!(quote.get_qty(), Decimal18::from_int(7));
    assert_eq!(quote.get_currency().as_str(), "EUR");
    assert_eq!(quote.get_unit(), "mt");

    // Filling twice changes nothing the first run did not, and a fact the
    // element stated is never overwritten.
    let mut once = trade(50);
    once.set_lastpx(Some(Decimal18::from_int(1)));
    once.fill_market();
    let twice = {
        let mut held = once.clone();
        held.fill_market();
        held
    };
    assert_eq!(once.get_px(), Decimal18::parse("82.5").expect("a decimal"));
    assert_eq!(once, twice);
}

#[test]
fn a_market_event_carries_what_its_chain_is_about_forward_and_folds_the_rest() {
    // An order naming its instrument, how long it stands and what the
    // market says about trading it.
    let mut order = trade(10);
    order.set_crosscode("O-100".to_owned());
    order.set_tif(Some("GoodTillCancel".to_owned()));
    order.set_tradable(Some(true));
    order.set_symbolticker(Some("BRN".to_owned()));
    order.set_isincode(Some(IsinCode::new("US0378331005").expect("an ISIN")));
    order.set_miccode(Some(MicCode::new("XLON").expect("a MIC")));
    order.finalize();

    // The report that answers it names none of that, and states a price and
    // a quantity of its own.
    let mut report = MarketEventData::at(at(20));
    report.set_crosscode("O-100".to_owned());
    report.set_px(Decimal18::from_int(83));
    report.set_qty(Decimal18::from_int(400));
    report.finalize();

    let followed = report.with_previous(&order).expect("the step after");
    // The step before, as the step before: a price beside the price it
    // moved from.
    assert_eq!(
        followed.get_prevpx(),
        Some(Decimal18::parse("82.5").expect("a decimal"))
    );
    assert_eq!(followed.get_prevqty(), Some(Decimal18::from_int(1_000)));
    // And what the chain is about, where this report said nothing.
    assert_eq!(followed.get_tif(), Some("GoodTillCancel"));
    assert_eq!(followed.get_tradable(), Some(true));
    assert_eq!(followed.get_symbolticker(), Some("BRN"));
    assert_eq!(followed.get_currency().as_str(), "USD");
    assert_eq!(followed.get_unit(), "bbl");
    assert_eq!(followed.get_side().as_str(), "BUY");
    assert_eq!(
        followed.get_isincode().map(IsinCode::as_str),
        Some("US0378331005")
    );
    assert_eq!(followed.get_miccode().map(MicCode::as_str), Some("XLON"));
    // What this report does say is its own: the price it states is not the
    // one it followed.
    assert_eq!(followed.get_px(), Decimal18::from_int(83));
    assert_eq!(followed.get_qty(), Decimal18::from_int(400));
    assert_eq!(followed.get_seqnum(), 1);

    // A statement of its own never gives way to the chain's.
    let mut own = MarketEventData::at(at(20));
    own.set_crosscode("O-100".to_owned());
    own.set_tif(Some("ImmediateOrCancel".to_owned()));
    own.set_tradable(Some(false));
    own.set_symbolticker(Some("WTI".to_owned()));
    own.set_prevpx(Some(Decimal18::from_int(1)));
    own.finalize();
    let followed = own.with_previous(&order).expect("the step after");
    assert_eq!(followed.get_tif(), Some("ImmediateOrCancel"));
    assert_eq!(followed.get_tradable(), Some(false));
    assert_eq!(followed.get_symbolticker(), Some("WTI"));
    assert_eq!(followed.get_prevpx(), Some(Decimal18::from_int(1)));

    // Merging folds the same facts the other way: two statements of one
    // event, the later leading where both state one and the other filling
    // what it leaves out.
    let mut first = trade(30);
    first.set_lastpx(Some(Decimal18::from_int(80)));
    first.set_cumqty(Some(Decimal18::from_int(100)));
    first.set_tif(Some("Day".to_owned()));
    first.finalize();
    let mut later = first.clone();
    later.set_currunix(at(40));
    later.set_lastpx(Some(Decimal18::from_int(81)));
    later.set_cumqty(None);
    later.set_leavesqty(Some(Decimal18::from_int(900)));
    later.set_avgpx(Some(Decimal18::parse("80.5").expect("a decimal")));
    later.set_tif(None);
    later.set_tradable(Some(true));
    later.set_symbolticker(Some("BRN".to_owned()));
    let merged = first.merge_with(&later).expect("the same event");
    assert_eq!(merged.get_lastpx(), Some(Decimal18::from_int(81)));
    assert_eq!(merged.get_cumqty(), Some(Decimal18::from_int(100)));
    assert_eq!(merged.get_leavesqty(), Some(Decimal18::from_int(900)));
    assert_eq!(
        merged.get_avgpx(),
        Some(Decimal18::parse("80.5").expect("a decimal"))
    );
    assert_eq!(merged.get_tif(), Some("Day"));
    assert_eq!(merged.get_tradable(), Some(true));
    assert_eq!(merged.get_symbolticker(), Some("BRN"));
    assert_eq!(merged.get_currunix(), at(40));

    // Every one of them is part of what the event is, so two events that
    // differ only there answer to different codes.
    let mut halted = trade(50);
    halted.set_tradable(Some(false));
    halted.finalize();
    let mut trading = trade(50);
    trading.set_tradable(Some(true));
    trading.finalize();
    assert_ne!(halted.get_currhashcode(), trading.get_currhashcode());
    let mut named = trade(50);
    named.set_symbolticker(Some("BRN".to_owned()));
    named.finalize();
    assert_ne!(named.get_currhashcode(), trade(50).get_currhashcode());
}
#[test]
fn isin_setters_fill_only_deterministic_missing_identifiers() {
    use yggdryl::graph::{MarketElement, MarketElementData, MarketEventData};
    use yggdryl::{CusipCode, IsinCode};

    let mut element = MarketElementData::default();
    element.set_isincode(Some(IsinCode::new("US0378331005").unwrap()));
    assert_eq!(
        element.get_cusipcode().map(CusipCode::as_str),
        Some("037833100")
    );
    assert!(element.get_cficode().is_none());
    assert!(element.get_bloombergcode().is_none());
    let stated = CusipCode::new("594918104").unwrap();
    element.set_cusipcode(Some(stated.clone()));
    element.set_isincode(Some(IsinCode::new("US0378331005").unwrap()));
    assert_eq!(element.get_cusipcode(), Some(&stated));

    let mut event = MarketEventData::at(0);
    event.set_isincode(Some(IsinCode::new("US0378331005").unwrap()));
    assert_eq!(
        event.get_cusipcode().map(CusipCode::as_str),
        Some("037833100")
    );
    let mut unknown = MarketElementData::default();
    unknown.set_isincode(Some(IsinCode::default()));
    assert!(unknown.get_cusipcode().is_none());
}
