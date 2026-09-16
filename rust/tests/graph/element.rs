//! An element states its identity, its cross identity and its parents; a
//! time element its instant, its state, its codes and the optional facts of
//! its lifecycle. The traits are signatures and two provided readings, so
//! what a caller can rely on is that a value implementing them answers
//! through them, including as a trait object, and that following and merging
//! fold the lifecycle the way the traits say.

use std::collections::BTreeMap;

use yggdryl::graph::{Element, MarketElement, TimeElement};
use yggdryl::types::{Bloomberg, Cfi, Currency, Cusip, Isin, Mic, Sedol, Side, State, Uuid};

/// One event as a caller would hold it: every fact the two traits name, and
/// nothing the graph owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Event {
    uuid: Uuid,
    xuuid: Option<Uuid>,
    identifiers: BTreeMap<String, String>,
    parents: Vec<Uuid>,
    unix: i128,
    hashcode: u64,
    xhashcode: u64,
    state: State,
    sequence_num: u64,
    creation_unix: Option<i128>,
    expiration_unix: Option<i128>,
    previous_unix: Option<i128>,
    previous_uuid: Option<Uuid>,
    snapshot_unix: Option<i128>,
}

impl Event {
    pub(crate) fn at(uuid: u128, unix: i128) -> Self {
        Self {
            uuid: Uuid::from_v8(uuid),
            xuuid: None,
            identifiers: BTreeMap::new(),
            parents: Vec::new(),
            unix,
            hashcode: 0,
            xhashcode: 0,
            state: State::from_spelling("New").expect("a shipped state"),
            sequence_num: 0,
            creation_unix: None,
            expiration_unix: None,
            previous_unix: None,
            previous_uuid: None,
            snapshot_unix: None,
        }
    }
}

impl Element for Event {
    fn get_current_uuid(&self) -> Uuid {
        self.uuid
    }

    fn set_current_uuid(&mut self, uuid: Uuid) {
        self.uuid = uuid;
    }

    fn get_xuuid(&self) -> Option<Uuid> {
        self.xuuid
    }

    fn set_xuuid(&mut self, xuuid: Option<Uuid>) {
        self.xuuid = xuuid;
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

    fn is_after(&self, other: &Self) -> bool {
        self.unix > other.unix
    }

    fn with_previous(self, previous: &Self) -> Option<Self> {
        self.following(previous)
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        self.merging(other)
    }
}

impl TimeElement for Event {
    fn get_unix(&self) -> i128 {
        self.unix
    }

    fn set_unix(&mut self, unix: i128) {
        self.unix = unix;
    }

    fn get_hashcode(&self) -> u64 {
        self.hashcode
    }

    fn set_hashcode(&mut self, hashcode: u64) {
        self.hashcode = hashcode;
    }

    fn get_xhashcode(&self) -> u64 {
        self.xhashcode
    }

    fn set_xhashcode(&mut self, xhashcode: u64) {
        self.xhashcode = xhashcode;
    }

    fn get_state(&self) -> &State {
        &self.state
    }

    fn set_state(&mut self, state: State) {
        self.state = state;
    }

    fn get_sequence_num(&self) -> u64 {
        self.sequence_num
    }

    fn set_sequence_num(&mut self, sequence_num: u64) {
        self.sequence_num = sequence_num;
    }

    fn get_creation_unix(&self) -> Option<i128> {
        self.creation_unix
    }

    fn set_creation_unix(&mut self, unix: Option<i128>) {
        self.creation_unix = unix;
    }

    fn get_expiration_unix(&self) -> Option<i128> {
        self.expiration_unix
    }

    fn set_expiration_unix(&mut self, unix: Option<i128>) {
        self.expiration_unix = unix;
    }

    fn get_previous_unix(&self) -> Option<i128> {
        self.previous_unix
    }

    fn set_previous_unix(&mut self, unix: Option<i128>) {
        self.previous_unix = unix;
    }

    fn get_previous_uuid(&self) -> Option<Uuid> {
        self.previous_uuid
    }

    fn set_previous_uuid(&mut self, uuid: Option<Uuid>) {
        self.previous_uuid = uuid;
    }

    fn get_snapshot_unix(&self) -> Option<i128> {
        self.snapshot_unix
    }

    fn set_snapshot_unix(&mut self, unix: Option<i128>) {
        self.snapshot_unix = unix;
    }
}

#[test]
fn an_element_answers_the_identities_and_parents_it_was_given() {
    let mut event = Event::at(2, 0);
    assert_eq!(event.get_current_uuid(), Uuid::from_v8(2));
    assert_eq!(
        event.get_xuuid(),
        None,
        "an element has no cross identity until it states one"
    );
    assert!(
        event.get_parentuuids().is_empty(),
        "a node naming no parent is a root"
    );

    event.set_current_uuid(Uuid::from_v8(3));
    assert_eq!(event.get_current_uuid(), Uuid::from_v8(3));

    // The cross identity is what this element is elsewhere, and can be unsaid.
    event.set_xuuid(Some(Uuid::from_v8(30)));
    assert_eq!(event.get_xuuid(), Some(Uuid::from_v8(30)));
    event.set_xuuid(None);
    assert_eq!(event.get_xuuid(), None);

    // The order is the element's own and comes back as stated.
    let parents = vec![Uuid::from_v8(9), Uuid::from_v8(1)];
    event.set_parentuuids(parents.clone());
    assert_eq!(event.get_parentuuids(), parents.as_slice());

    // An empty list makes it a root again.
    event.set_parentuuids(Vec::new());
    assert!(event.get_parentuuids().is_empty());
}

fn identifiers<const N: usize>(pairs: [(&str, &str); N]) -> BTreeMap<String, String> {
    pairs
        .into_iter()
        .map(|(scheme, identifier)| (scheme.to_owned(), identifier.to_owned()))
        .collect()
}

#[test]
fn an_element_goes_by_the_names_it_was_given_each_under_its_scheme() {
    let mut event = Event::at(1, 0);
    assert!(event.get_identifiers().is_empty(), "no system named it yet");
    event.set_identifiers(identifiers([("OrderID", "O-1"), ("ClOrdID", "C-1")]));
    assert_eq!(event.get_identifiers()["ClOrdID"], "C-1");
    assert_eq!(event.get_identifiers()["OrderID"], "O-1");
    // Held as text under text, in the scheme's order, so a walk over them is
    // the same walk whatever order they were stated in.
    assert_eq!(
        event.get_identifiers().keys().collect::<Vec<_>>(),
        ["ClOrdID", "OrderID"]
    );
    // The map is replaced whole, never merged.
    event.set_identifiers(identifiers([("ExecID", "E-1")]));
    assert_eq!(event.get_identifiers().len(), 1);
    assert_eq!(event.get_identifiers().get("ClOrdID"), None);
    // And a walk over trait objects reads them the same way.
    let held: &dyn Element = &event;
    assert_eq!(held.get_identifiers()["ExecID"], "E-1");
}

#[test]
fn a_timed_element_is_after_another_by_its_instant() {
    let earlier = Event::at(1, 10);
    let later = Event::at(2, 20);
    assert!(later.is_after(&earlier));
    assert!(earlier.is_before(&later));
    assert!(!earlier.is_after(&later));
    assert!(!later.is_before(&earlier));
    // Never after itself, and an equal instant is neither after nor before.
    assert!(!earlier.is_after(&earlier) && !earlier.is_before(&earlier));
    let same = Event::at(3, 10);
    assert!(!same.is_after(&earlier) && !same.is_before(&earlier));
    // A market element orders as the event it holds does.
    assert!(trade(4, 20).is_after(&trade(5, 10)));
    assert!(trade(5, 10).is_before(&trade(4, 20)));
}

#[test]
fn a_time_element_answers_its_instant_state_code_and_is_still_an_element() {
    let mut event = Event::at(7, 0);
    // Nanoseconds since the epoch, wider than any clock the crate reads.
    let unix = i128::from(i64::MAX) * 1_000;
    event.set_unix(unix);
    event.set_hashcode(0xDEAD_BEEF_CAFE_F00D);
    assert_eq!(event.get_unix(), unix);
    assert_eq!(event.get_hashcode(), 0xDEAD_BEEF_CAFE_F00D);

    // Negative instants are before the epoch, and the type holds them.
    event.set_unix(-1);
    assert_eq!(event.get_unix(), -1);

    // A state is never absent, and moves as the lifecycle does.
    assert!(event.get_state().is_live());
    event.set_state(State::from_spelling("Filled").expect("a shipped state"));
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

    // One walk reads both traits through the subtrait object.
    event.set_parentuuids(vec![Uuid::from_v8(1)]);
    let held: &dyn TimeElement = &event;
    assert_eq!(held.get_current_uuid(), Uuid::from_v8(7));
    assert_eq!(held.get_parentuuids(), [Uuid::from_v8(1)]);
    assert_eq!(held.get_unix(), -1);
    assert_eq!(held.get_hashcode(), 0xDEAD_BEEF_CAFE_F00D);
    assert!(held.get_state().is_done());
}

#[test]
fn the_optional_lifecycle_facts_are_stated_only_where_known() {
    let mut event = Event::at(4, 40);
    assert_eq!(event.get_creation_unix(), None);
    assert_eq!(event.get_expiration_unix(), None);
    assert_eq!(event.get_previous_unix(), None);
    assert_eq!(event.get_previous_uuid(), None);
    assert_eq!(event.get_snapshot_unix(), None);

    event.set_creation_unix(Some(35));
    event.set_expiration_unix(Some(100));
    event.set_previous_unix(Some(30));
    event.set_previous_uuid(Some(Uuid::from_v8(3)));
    event.set_snapshot_unix(Some(40));
    assert_eq!(event.get_snapshot_unix(), Some(40));
    assert_eq!(event.get_creation_unix(), Some(35));
    assert_eq!(event.get_expiration_unix(), Some(100));
    assert_eq!(event.get_previous_unix(), Some(30));
    assert_eq!(event.get_previous_uuid(), Some(Uuid::from_v8(3)));

    // Each fact is unsaid on its own.
    event.set_expiration_unix(None);
    assert_eq!(event.get_expiration_unix(), None);
    assert_eq!(event.get_creation_unix(), Some(35));
}

#[test]
fn following_records_the_predecessor_and_refuses_what_cannot_follow() {
    let first = Event::at(1, 10);
    let second = Event::at(2, 20)
        .with_previous(&first)
        .expect("a later element follows an earlier one");
    assert_eq!(second.get_previous_uuid(), Some(first.get_current_uuid()));
    assert_eq!(second.get_previous_unix(), Some(10));
    // The place in the chain is the one after the predecessor's, and what
    // the element itself says stays: its parents, its instant.
    assert_eq!(second.get_sequence_num(), 1);
    assert!(second.get_parentuuids().is_empty());
    assert_eq!(second.get_unix(), 20);
    let third = Event::at(4, 30).with_previous(&second).expect("follows");
    assert_eq!(third.get_sequence_num(), 2);
    let mut deep = Event::at(5, 40);
    deep.set_sequence_num(u64::MAX);
    let capped = Event::at(6, 50).with_previous(&deep).expect("follows");
    assert_eq!(
        capped.get_sequence_num(),
        u64::MAX,
        "a place past the count saturates"
    );

    // The same instant follows: a predecessor is not later, and equal is not later.
    let same = Event::at(3, 10)
        .with_previous(&first)
        .expect("an equal instant follows");
    assert_eq!(same.get_previous_uuid(), Some(first.get_current_uuid()));

    // An element follows neither itself nor one that happened after it.
    assert!(Event::at(1, 10).with_previous(&first).is_none());
    assert!(Event::at(9, 5).with_previous(&second).is_none());

    // A later predecessor replaces the one recorded before.
    let third = second
        .clone()
        .with_previous(&Event::at(8, 15))
        .expect("a later predecessor still precedes");
    assert_eq!(third.get_previous_uuid(), Some(Uuid::from_v8(8)));
    assert_eq!(third.get_previous_unix(), Some(15));
}

pub(crate) fn filled() -> State {
    State::from_spelling("Filled").expect("a shipped state")
}

#[test]
fn following_carries_the_lifecycle_forward() {
    // The earliest creation, the latest expiration, the furthest state; and
    // each where only one side states it is that side's.
    let mut previous = Event::at(1, 10);
    previous.set_creation_unix(Some(5));
    previous.set_expiration_unix(Some(200));
    previous.set_state(filled());
    let mut next = Event::at(2, 20);
    next.set_creation_unix(Some(8));
    next.set_expiration_unix(Some(100));
    let next = next
        .with_previous(&previous)
        .expect("the later one follows");
    assert_eq!(next.get_creation_unix(), Some(5));
    assert_eq!(next.get_expiration_unix(), Some(200));
    assert!(
        next.get_state().is_done(),
        "the furthest state carries forward"
    );

    // A previous that knows less leaves what the next one knows alone.
    let mut next = Event::at(3, 30);
    next.set_creation_unix(Some(25));
    next.set_expiration_unix(Some(300));
    let bare = Event::at(4, 20);
    let next = next.with_previous(&bare).expect("follows");
    assert_eq!(next.get_creation_unix(), Some(25));
    assert_eq!(next.get_expiration_unix(), Some(300));
    assert!(
        next.get_state().is_live(),
        "a lesser state does not move the next one back"
    );

    // And one that knows more fills what the next one did not state.
    let next = Event::at(5, 40).with_previous(&previous).expect("follows");
    assert_eq!(next.get_creation_unix(), Some(5));
    assert_eq!(next.get_expiration_unix(), Some(200));

    // What the next element itself says moves nowhere.
    let mut own = Event::at(6, 50);
    own.set_hashcode(0xABC);
    own.set_xuuid(Some(Uuid::from_v8(60)));
    own.set_parentuuids(vec![Uuid::from_v8(61)]);
    let own = own.with_previous(&previous).expect("follows");
    assert_eq!(own.get_unix(), 50);
    assert_eq!(own.get_hashcode(), 0xABC);
    assert_eq!(own.get_xuuid(), Some(Uuid::from_v8(60)));
    assert_eq!(own.get_parentuuids(), [Uuid::from_v8(61)]);

    // The names the predecessor went by carry forward where the next one
    // does not state them, and its own word stays where it does.
    let mut named = Event::at(7, 10);
    named.set_identifiers(identifiers([("ClOrdID", "C-1"), ("OrderID", "O-1")]));
    let mut next = Event::at(8, 20);
    next.set_identifiers(identifiers([("OrderID", "O-2")]));
    let next = next.with_previous(&named).expect("follows");
    assert_eq!(next.get_identifiers()["ClOrdID"], "C-1");
    assert_eq!(next.get_identifiers()["OrderID"], "O-2");
    assert_eq!(next.get_identifiers().len(), 2);
}

#[test]
fn merging_folds_another_statement_of_the_same_element() {
    let mut first = Event::at(1, 10);
    first.set_hashcode(0xA);
    first.set_xhashcode(0xA0);
    first.set_parentuuids(vec![Uuid::from_v8(7)]);
    first.set_creation_unix(Some(9));
    first.set_previous_uuid(Some(Uuid::from_v8(0)));
    first.set_previous_unix(Some(1));

    let mut later = Event::at(1, 20);
    later.set_hashcode(0xB);
    later.set_xhashcode(0xB0);
    later.set_xuuid(Some(Uuid::from_v8(10)));
    later.set_parentuuids(vec![Uuid::from_v8(8), Uuid::from_v8(7)]);
    later.set_creation_unix(Some(4));
    later.set_expiration_unix(Some(99));
    later.set_state(filled());
    later.set_previous_uuid(Some(Uuid::from_v8(5)));
    later.set_previous_unix(Some(6));

    // Another element does not merge at all.
    assert!(first.clone().merge_with(&Event::at(2, 10)).is_none());

    later.set_sequence_num(3);
    let merged = first.clone().merge_with(&later).expect("the same element");
    // The later statement has the last word on the instant and the codes,
    // and the further place in the chain stands.
    assert_eq!(merged.get_unix(), 20);
    assert_eq!(merged.get_hashcode(), 0xB);
    assert_eq!(merged.get_xhashcode(), 0xB0);
    assert_eq!(merged.get_sequence_num(), 3);
    // The cross element fills what this one left out; the parents are the
    // union in this element's order, then the other's.
    assert_eq!(merged.get_xuuid(), Some(Uuid::from_v8(10)));
    assert_eq!(
        merged.get_parentuuids(),
        [Uuid::from_v8(7), Uuid::from_v8(8)]
    );
    // The lifecycle folds as following folds it.
    assert_eq!(merged.get_creation_unix(), Some(4));
    assert_eq!(merged.get_expiration_unix(), Some(99));
    assert!(merged.get_state().is_done());
    // The predecessor is this element's where it names one.
    assert_eq!(merged.get_previous_uuid(), Some(Uuid::from_v8(0)));
    assert_eq!(merged.get_previous_unix(), Some(1));

    // Merged the other way, the earlier statement's instant and codes lose,
    // and the predecessor it names is kept.
    let merged = later.merge_with(&first).expect("the same element");
    assert_eq!(merged.get_unix(), 20);
    assert_eq!(merged.get_hashcode(), 0xB);
    assert_eq!(merged.get_xuuid(), Some(Uuid::from_v8(10)));
    assert_eq!(
        merged.get_parentuuids(),
        [Uuid::from_v8(8), Uuid::from_v8(7)]
    );
    assert_eq!(merged.get_previous_uuid(), Some(Uuid::from_v8(5)));

    // A statement naming no predecessor takes the other's.
    let mut silent = Event::at(1, 30);
    silent.set_hashcode(0xC);
    let merged = silent.merge_with(&first).expect("the same element");
    assert_eq!(
        merged.get_hashcode(),
        0xC,
        "the later statement's code stands"
    );
    assert_eq!(merged.get_previous_uuid(), Some(Uuid::from_v8(0)));
    assert_eq!(merged.get_creation_unix(), Some(9));
}

#[test]
fn a_walk_over_elements_reaches_a_root_by_identity() {
    // A caller resolves parents by identity, so a graph is a map of them.
    let mut root = Event::at(1, 10);
    root.set_state(State::from_spelling("Filled").expect("a shipped state"));
    let mut child = Event::at(2, 20);
    child.set_parentuuids(vec![root.get_current_uuid()]);
    let mut leaf = Event::at(3, 30);
    leaf.set_parentuuids(vec![child.get_current_uuid()]);

    let held: Vec<Box<dyn TimeElement>> =
        vec![Box::new(root), Box::new(child), Box::new(leaf.clone())];
    let by_uuid = |uuid: Uuid| {
        held.iter()
            .find(|element| element.get_current_uuid() == uuid)
    };

    let mut at: &dyn TimeElement = &leaf;
    let mut lineage = vec![at.get_unix()];
    while let Some(parent) = at.get_parentuuids().first().copied() {
        at = by_uuid(parent)
            .expect("a parent is an element of the graph")
            .as_ref();
        lineage.push(at.get_unix());
    }
    assert_eq!(lineage, [30, 20, 10]);
    assert!(
        at.get_state().is_done(),
        "the root reached its terminal state"
    );
}

#[test]
fn the_instant_and_the_code_derive_one_time_ordered_identity() {
    let mut event = Event::at(1, 1_700_000_000_000_000_000);
    event.set_hashcode(0xCAFE);
    let held = event.txhash().expect("an instant a TxHash holds");
    assert_eq!(held.unix(), 1_700_000_000_000_000_000);
    assert_eq!(held.digest().as_u64(), Some(0xCAFE));

    // The same instant and code derive the same identity, every time.
    let identity = event.time_uuid().expect("an identity");
    assert_eq!(event.time_uuid().expect("an identity"), identity);
    assert_eq!(identity, held.into_uuid().expect("the TxHash's own UUID"));

    // A later instant sorts later whatever the code, and the same instant
    // sorts by code: the instant is in front, as a UUIDv7's is.
    let mut later = event.clone();
    later.set_unix(1_700_000_000_000_001_000);
    later.set_hashcode(0);
    assert!(later.time_uuid().expect("an identity") > identity);
    let mut sibling = event.clone();
    sibling.set_hashcode(0xCAFF);
    assert_ne!(sibling.time_uuid().expect("an identity"), identity);

    // An instant past what a TxHash holds derives nothing rather than a lie,
    // and one before the epoch has no UUIDv7 to derive.
    let mut far = event.clone();
    far.set_unix(i128::from(i64::MAX) + 1);
    assert!(far.txhash().is_err());
    assert!(far.time_uuid().is_err());
    let mut before = event.clone();
    before.set_unix(-1);
    assert!(before.txhash().is_ok());
    assert!(before.time_uuid().is_err());

    // The cross identity couples the creation with the cross code, and is
    // nothing where the element does not know when it was created.
    assert_eq!(event.time_xuuid().expect("nothing to couple"), None);
    let mut created = event.clone();
    created.set_creation_unix(Some(1_600_000_000_000_000_000));
    created.set_xhashcode(0xBEEF);
    let xuuid = created
        .time_xuuid()
        .expect("an identity")
        .expect("a creation to couple");
    let mut twin = Event::at(9, 0);
    twin.set_creation_unix(Some(1_600_000_000_000_000_000));
    twin.set_xhashcode(0xBEEF);
    assert_eq!(twin.time_xuuid().expect("an identity"), Some(xuuid));
    assert_ne!(
        Some(xuuid),
        Some(identity),
        "the cross identity is its own coupling"
    );
    created.set_creation_unix(Some(i128::from(i64::MIN) - 1));
    assert!(created.time_xuuid().is_err());
}

/// One trade as a market holds it: the timed facts, and the market's five.
#[derive(Debug, Clone, PartialEq)]
struct Trade {
    event: Event,
    px: f64,
    currency: Currency,
    qty: f64,
    unit: String,
    side: Side,
    isincode: Option<Isin>,
    cusipcode: Option<Cusip>,
    sedolcode: Option<Sedol>,
    bloombergcode: Option<Bloomberg>,
    cficode: Option<Cfi>,
    miccode: Option<Mic>,
}

impl Element for Trade {
    fn get_current_uuid(&self) -> Uuid {
        self.event.get_current_uuid()
    }

    fn set_current_uuid(&mut self, uuid: Uuid) {
        self.event.set_current_uuid(uuid);
    }

    fn get_xuuid(&self) -> Option<Uuid> {
        self.event.get_xuuid()
    }

    fn set_xuuid(&mut self, xuuid: Option<Uuid>) {
        self.event.set_xuuid(xuuid);
    }

    fn get_identifiers(&self) -> &BTreeMap<String, String> {
        self.event.get_identifiers()
    }

    fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) {
        self.event.set_identifiers(identifiers);
    }

    fn get_parentuuids(&self) -> &[Uuid] {
        self.event.get_parentuuids()
    }

    fn set_parentuuids(&mut self, parents: Vec<Uuid>) {
        self.event.set_parentuuids(parents);
    }

    fn is_after(&self, other: &Self) -> bool {
        self.event.is_after(&other.event)
    }

    fn with_previous(self, previous: &Self) -> Option<Self> {
        self.following(previous)
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        self.merging_market(other)
    }
}

impl TimeElement for Trade {
    fn get_unix(&self) -> i128 {
        self.event.get_unix()
    }

    fn set_unix(&mut self, unix: i128) {
        self.event.set_unix(unix);
    }

    fn get_hashcode(&self) -> u64 {
        self.event.get_hashcode()
    }

    fn set_hashcode(&mut self, hashcode: u64) {
        self.event.set_hashcode(hashcode);
    }

    fn get_xhashcode(&self) -> u64 {
        self.event.get_xhashcode()
    }

    fn set_xhashcode(&mut self, xhashcode: u64) {
        self.event.set_xhashcode(xhashcode);
    }

    fn get_state(&self) -> &State {
        self.event.get_state()
    }

    fn set_state(&mut self, state: State) {
        self.event.set_state(state);
    }

    fn get_sequence_num(&self) -> u64 {
        self.event.get_sequence_num()
    }

    fn set_sequence_num(&mut self, sequence_num: u64) {
        self.event.set_sequence_num(sequence_num);
    }

    fn get_creation_unix(&self) -> Option<i128> {
        self.event.get_creation_unix()
    }

    fn set_creation_unix(&mut self, unix: Option<i128>) {
        self.event.set_creation_unix(unix);
    }

    fn get_expiration_unix(&self) -> Option<i128> {
        self.event.get_expiration_unix()
    }

    fn set_expiration_unix(&mut self, unix: Option<i128>) {
        self.event.set_expiration_unix(unix);
    }

    fn get_previous_unix(&self) -> Option<i128> {
        self.event.get_previous_unix()
    }

    fn set_previous_unix(&mut self, unix: Option<i128>) {
        self.event.set_previous_unix(unix);
    }

    fn get_previous_uuid(&self) -> Option<Uuid> {
        self.event.get_previous_uuid()
    }

    fn set_previous_uuid(&mut self, uuid: Option<Uuid>) {
        self.event.set_previous_uuid(uuid);
    }

    fn get_snapshot_unix(&self) -> Option<i128> {
        self.event.get_snapshot_unix()
    }

    fn set_snapshot_unix(&mut self, unix: Option<i128>) {
        self.event.set_snapshot_unix(unix);
    }
}

impl MarketElement for Trade {
    fn get_px(&self) -> f64 {
        self.px
    }

    fn set_px(&mut self, px: f64) {
        self.px = px;
    }

    fn get_currency(&self) -> &Currency {
        &self.currency
    }

    fn set_currency(&mut self, currency: Currency) {
        self.currency = currency;
    }

    fn get_qty(&self) -> f64 {
        self.qty
    }

    fn set_qty(&mut self, qty: f64) {
        self.qty = qty;
    }

    fn get_unit(&self) -> &str {
        &self.unit
    }

    fn set_unit(&mut self, unit: String) {
        self.unit = unit;
    }

    fn get_side(&self) -> &Side {
        &self.side
    }

    fn set_side(&mut self, side: Side) {
        self.side = side;
    }

    fn get_isincode(&self) -> Option<&Isin> {
        self.isincode.as_ref()
    }

    fn set_isincode(&mut self, isincode: Option<Isin>) {
        self.isincode = isincode;
    }

    fn get_cusipcode(&self) -> Option<&Cusip> {
        self.cusipcode.as_ref()
    }

    fn set_cusipcode(&mut self, cusipcode: Option<Cusip>) {
        self.cusipcode = cusipcode;
    }

    fn get_sedolcode(&self) -> Option<&Sedol> {
        self.sedolcode.as_ref()
    }

    fn set_sedolcode(&mut self, sedolcode: Option<Sedol>) {
        self.sedolcode = sedolcode;
    }

    fn get_bloombergcode(&self) -> Option<&Bloomberg> {
        self.bloombergcode.as_ref()
    }

    fn set_bloombergcode(&mut self, bloombergcode: Option<Bloomberg>) {
        self.bloombergcode = bloombergcode;
    }

    fn get_cficode(&self) -> Option<&Cfi> {
        self.cficode.as_ref()
    }

    fn set_cficode(&mut self, cficode: Option<Cfi>) {
        self.cficode = cficode;
    }

    fn get_miccode(&self) -> Option<&Mic> {
        self.miccode.as_ref()
    }

    fn set_miccode(&mut self, miccode: Option<Mic>) {
        self.miccode = miccode;
    }
}

fn trade(uuid: u128, unix: i128) -> Trade {
    Trade {
        event: Event::at(uuid, unix),
        px: 82.5,
        currency: Currency::new("USD").expect("a currency"),
        qty: 1_000.0,
        unit: "bbl".to_owned(),
        side: Side::read("Buy").expect("a side"),
        isincode: None,
        cusipcode: None,
        sedolcode: None,
        bloombergcode: None,
        cficode: None,
        miccode: None,
    }
}

#[test]
fn a_market_element_names_its_instrument_the_way_the_market_does() {
    let mut held = trade(1, 10);
    for absent in [
        held.get_isincode().is_none(),
        held.get_cusipcode().is_none(),
        held.get_sedolcode().is_none(),
        held.get_bloombergcode().is_none(),
        held.get_cficode().is_none(),
    ] {
        assert!(
            absent,
            "an instrument is named only where the market names it"
        );
    }
    held.set_isincode(Some(Isin::new("US0378331005").expect("an ISIN")));
    held.set_cusipcode(Some(Cusip::new("037833100").expect("a CUSIP")));
    held.set_sedolcode(Some(Sedol::new("B0YBKJ7").expect("a SEDOL")));
    held.set_bloombergcode(Some(
        Bloomberg::new("AAPL US EQUITY").expect("a Bloomberg identifier"),
    ));
    held.set_cficode(Some(Cfi::new("ESVUFR").expect("a CFI")));
    held.set_miccode(Some(Mic::new("XPAR").expect("a MIC")));
    assert_eq!(held.get_isincode().map(Isin::as_str), Some("US0378331005"));
    assert_eq!(held.get_cusipcode().map(Cusip::as_str), Some("037833100"));
    assert_eq!(held.get_sedolcode().map(Sedol::as_str), Some("B0YBKJ7"));
    assert_eq!(
        held.get_bloombergcode().map(Bloomberg::as_str),
        Some("AAPL US EQUITY")
    );
    assert_eq!(held.get_cficode().map(Cfi::as_str), Some("ESVUFR"));
    assert_eq!(held.get_miccode().map(Mic::as_str), Some("XPAR"));
    // Each is unsaid on its own.
    held.set_cusipcode(None);
    assert!(held.get_cusipcode().is_none());
    assert!(held.get_isincode().is_some());
    // The codes are the crate's own: a spelling that is no identifier never
    // reaches the element.
    assert!(
        Isin::new("US0378331006").is_err(),
        "a wrong check digit is no ISIN"
    );
}

#[test]
fn a_market_element_answers_its_five_facts_and_is_still_a_timed_element() {
    let mut held = trade(1, 10);
    assert_eq!(held.get_px(), 82.5);
    assert_eq!(held.get_currency().as_str(), "USD");
    assert_eq!(held.get_qty(), 1_000.0);
    assert_eq!(held.get_unit(), "bbl");
    assert_eq!(held.get_side().as_str(), "BUY");

    held.set_px(83.0);
    held.set_currency(Currency::new("EUR").expect("a currency"));
    held.set_qty(0.0);
    held.set_unit("MWh".to_owned());
    held.set_side(Side::read("2").expect("a side"));
    assert_eq!(held.get_px(), 83.0);
    assert_eq!(held.get_currency().as_str(), "EUR");
    assert_eq!(held.get_qty(), 0.0);
    assert_eq!(held.get_unit(), "MWh");
    assert_eq!(held.get_side().as_str(), "SELL");

    // One walk reads all three traits through the market object, and the
    // timed readings are the market element's too.
    let next = trade(2, 20)
        .with_previous(&held)
        .expect("a later trade follows");
    let object: &dyn MarketElement = &next;
    assert_eq!(object.get_current_uuid(), Uuid::from_v8(2));
    assert_eq!(object.get_unix(), 20);
    assert_eq!(object.get_sequence_num(), 1);
    assert_eq!(object.get_previous_uuid(), Some(Uuid::from_v8(1)));
    assert_eq!(
        object.get_px(),
        82.5,
        "what the trade itself says moves nowhere"
    );
}

#[test]
fn merging_a_market_element_takes_the_later_statement_and_the_better_codes() {
    let mut first = trade(1, 10);
    first.set_currency(Currency::new("XXX").expect("no currency"));
    first.set_side(Side::read("UNKNOWN").expect("a side"));
    first.set_cficode(Some(Cfi::new("ESXXXR").expect("a CFI")));
    first.set_isincode(Some(Isin::new("US0378331005").expect("an ISIN")));
    let mut later = trade(1, 20);
    later.set_px(83.0);
    later.set_qty(5.0);
    later.set_unit("MWh".to_owned());
    later.set_currency(Currency::new("EUR").expect("a currency"));
    later.set_side(Side::read("2").expect("a side"));
    later.set_cficode(Some(Cfi::new("ESVUFX").expect("a CFI")));
    later.set_miccode(Some(Mic::new("XPAR").expect("a MIC")));

    // The later statement has the last word on the market's facts, and each
    // code is the better of the two: the earlier fills what the later left
    // unknown, and a code only one statement names is that one's.
    let merged = first.clone().merge_with(&later).expect("the same trade");
    assert_eq!(merged.get_unix(), 20);
    assert_eq!(
        (merged.get_px(), merged.get_qty(), merged.get_unit()),
        (83.0, 5.0, "MWh")
    );
    assert_eq!(merged.get_currency().as_str(), "EUR");
    assert_eq!(merged.get_side().as_str(), "SELL");
    assert_eq!(merged.get_cficode().map(Cfi::as_str), Some("ESVUFR"));
    assert_eq!(
        merged.get_isincode().map(Isin::as_str),
        Some("US0378331005")
    );
    assert_eq!(merged.get_miccode().map(Mic::as_str), Some("XPAR"));

    // Merged the other way round the later statement still leads, so the
    // reading does not depend on which statement a caller held.
    let merged = later.clone().merge_with(&first).expect("the same trade");
    assert_eq!(
        (merged.get_px(), merged.get_currency().as_str()),
        (83.0, "EUR")
    );
    assert_eq!(merged.get_cficode().map(Cfi::as_str), Some("ESVUFR"));
    assert_eq!(
        merged.get_isincode().map(Isin::as_str),
        Some("US0378331005")
    );

    // A statement that knows a code the later one states as none keeps
    // its own: the later statement leads, and unknown takes the other.
    let mut bare = trade(1, 30);
    bare.set_currency(Currency::new("XXX").expect("no currency"));
    let merged = later.clone().merge_with(&bare).expect("the same trade");
    assert_eq!(merged.get_currency().as_str(), "EUR");
    assert_eq!(merged.get_unix(), 30);

    // Another trade does not merge at all.
    assert!(first.merge_with(&trade(2, 30)).is_none());
}
