//! `rust/src/graph/element.rs`: an element states its identity, its cross
//! identity, its codes and its sources, and an event its instant, its state
//! and the optional facts of its lifecycle. The traits are signatures and
//! provided readings, so what a caller can rely on is that a value
//! implementing them answers through them, and that following and merging
//! fold the lifecycle the way the traits say - over a foreign type that
//! implements only the signatures, and over the core's own event, a text
//! line. The market leaves' readings of the same traits are
//! `rust/market/tests/graph/element.rs`'s.

use std::hash::Hasher;
use std::sync::Arc;

use yggdryl::graph::{Element, Event};
use yggdryl::text::{TextBytes, TextLine, TextOptions};
use yggdryl::xxhash::Xxh3;
use yggdryl::{State, Uuid};

/// A line at the epoch: the core's own event, holding one byte of body.
fn line() -> TextLine {
    TextLine::from_bytes(
        0,
        TextBytes::from_bytes(b"x").expect("a page"),
        Arc::new(TextOptions::new()),
    )
    .expect("a line")
}

#[test]
fn generic_event_uuid_lists_are_sorted_unique_at_the_storage_boundary() {
    let uuids = [Uuid::from_v8(3), Uuid::from_v8(1), Uuid::from_v8(2)];
    let expected = [Uuid::from_v8(1), Uuid::from_v8(2), Uuid::from_v8(3)];

    let mut element = line();
    element.set_srcuuids(vec![uuids[2], uuids[1], uuids[2], uuids[0]]);
    assert_eq!(element.get_srcuuids(), expected);

    let mut event = line();
    event.set_srcuuids(vec![uuids[1], uuids[0], uuids[1], uuids[2]]);
    assert_eq!(event.get_srcuuids(), expected);
}

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

fn filled() -> State {
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

/// The event UUID payload: content and the whole sequence under the cross
/// hash seed, narrowed only where UUIDv7's `rand_b` stores it.
fn uuid_payload(hashcode: u64, crosshashcode: u64, seqnum: u64) -> u64 {
    let mut payload = Xxh3::with_seed(crosshashcode);
    payload.write_bytes(&hashcode.to_le_bytes());
    payload.write_bytes(&seqnum.to_le_bytes());
    payload.as_u64() & ((1 << 62) - 1)
}

/// The XXH3-64 of one cross code, as the trait derives it.
fn crosshash(crosscode: &str) -> u64 {
    let mut state = Xxh3::new();
    state.write(crosscode.as_bytes());
    state.as_u64()
}

#[test]
fn following_records_the_predecessor_and_refuses_what_cannot_follow() {
    let first = Report::at(1, 10);
    let second = Report::at(2, 20)
        .with_previous(&first)
        .expect("a later element follows an earlier one");
    assert_eq!(second.get_prevuuid(), Some(first.get_uuid()));
    assert_eq!(second.get_prevunix(), Some(10));
    // A later instant keeps the place it already had.
    assert_eq!(second.get_seqnum(), 0);
    assert_eq!(second.get_transunix(), 20);
    // A step at a later instant keeps its own place, however many steps its
    // chain has taken; following what it already follows changes nothing.
    let third = Report::at(4, 30).with_previous(&second).expect("follows");
    assert_eq!(third.get_seqnum(), 0);
    assert_eq!(third.get_prevuuid(), Some(second.get_uuid()));
    assert!(third.clone().with_previous(&second).is_none());
    // An equal instant is not later: it takes the place after its
    // predecessor's, saturating past the widest one a predecessor can hold.
    let mut deep = Report::at(5, 40);
    deep.set_seqnum(u64::MAX);
    let capped = Report::at(6, 40).with_previous(&deep).expect("follows");
    assert_eq!(
        capped.get_seqnum(),
        u64::MAX,
        "a place past the count saturates"
    );

    // The same instant follows: a predecessor is not later, and equal is not later.
    let same = Report::at(3, 10)
        .with_previous(&first)
        .expect("an equal instant follows");
    assert_eq!(same.get_prevuuid(), Some(first.get_uuid()));
    // An equal instant stands after its predecessor's place.
    assert_eq!(same.get_seqnum(), 1);

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
fn following_never_carries_the_wire_clock() {
    let mut previous = Report::at(1, 10);
    previous.set_state(filled());
    previous.set_sendunix(Some(9));
    let next = Report::at(2, 20)
        .with_previous(&previous)
        .expect("the later event follows");
    assert_eq!(
        next.get_sendunix(),
        None,
        "no predecessor wire clock carries"
    );
}

#[test]
fn merging_folds_another_statement_of_the_same_element() {
    let mut first = Report::at(1, 10);
    first.set_hashcode(0xA);
    first.set_srcuuids(vec![Uuid::from_v8(70)]);
    first.set_creaunix(Some(9));
    first.set_prevuuid(Some(Uuid::from_v8(0)));
    first.set_prevunix(Some(1));

    let mut later = Report::at(1, 20);
    later.set_hashcode(0xB);
    later.set_crosscode("O-10".to_owned());
    later.set_srcuuids(vec![Uuid::from_v8(71), Uuid::from_v8(70)]);
    later.set_creaunix(Some(4));
    later.set_exprunix(Some(99));
    later.set_state(filled());
    later.set_prevuuid(Some(Uuid::from_v8(5)));
    later.set_prevunix(Some(6));
    later.finalize();

    // Another element does not merge at all.
    assert!(first.clone().merge_with(&Report::at(2, 10)).is_none());

    later.set_seqnum(3);
    let merged = first.clone().merge_with(&later).expect("the same element");
    // The later statement has the last word on the instant and the code,
    // and the higher place stands.
    assert_eq!(merged.get_transunix(), 20);
    assert_eq!(merged.get_hashcode(), 0xB);
    assert_eq!(merged.get_seqnum(), 3);
    // With no wire clocks, the later event is the reference: its cross
    // code leads, and the sources are a sorted unique union, once each,
    // because the merged statement was read from both lines.
    assert_eq!(merged.get_crosscode(), "O-10");
    assert_eq!(merged.get_crosshashcode(), crosshash("O-10"));
    assert_eq!(merged.get_crossuuid(), later.get_crossuuid());
    assert_eq!(
        merged.get_srcuuids(),
        [Uuid::from_v8(70), Uuid::from_v8(71)]
    );
    // The lifecycle folds as following folds it.
    assert_eq!(merged.get_creaunix(), Some(4));
    assert_eq!(merged.get_exprunix(), Some(99));
    assert!(merged.get_state().is_done());
    // The predecessor is the reference's where it names one.
    assert_eq!(merged.get_prevuuid(), Some(Uuid::from_v8(5)));
    assert_eq!(merged.get_prevunix(), Some(6));

    // Merged the other way, the earlier statement adds nothing the later
    // one lacks. Same-event folding never infers an execution clock from the
    // lifecycle state; intake and ordinary following own normalization.
    assert!(later.clone().merge_with(&first).is_none());
    assert_eq!(later.get_transunix(), 20);
    assert_eq!(later.get_hashcode(), 0xB);
    assert_eq!(later.get_crosscode(), "O-10");
    assert_eq!(later.get_srcuuids(), [Uuid::from_v8(70), Uuid::from_v8(71)]);
    assert_eq!(later.get_prevuuid(), Some(Uuid::from_v8(5)));

    // A statement naming no predecessor takes the other's.
    let mut silent = Report::at(1, 30);
    silent.set_hashcode(0xC);
    let merged = silent.merge_with(&first).expect("the same element");
    assert_eq!(
        merged.get_hashcode(),
        0xC,
        "the later statement's code stands"
    );
    assert_eq!(merged.get_prevuuid(), Some(Uuid::from_v8(0)));
    assert_eq!(merged.get_creaunix(), Some(9));
}

#[test]
fn merging_uses_the_statement_sent_last_as_the_reference_but_keeps_earliest_clocks() {
    let mut event_time_later = Report::at(1, 30);
    event_time_later.set_hashcode(0xA);
    event_time_later.set_sendunix(Some(100));
    event_time_later.set_crosscode("OLD".to_owned());
    event_time_later.set_srcuuids(vec![Uuid::from_v8(70)]);
    event_time_later.set_prevuuid(Some(Uuid::from_v8(2)));
    event_time_later.set_prevunix(Some(20));
    event_time_later.set_snapunix(Some(31));

    let mut recorded_later = Report::at(1, 20);
    recorded_later.set_hashcode(0xB);
    recorded_later.set_sendunix(Some(200));
    recorded_later.set_crosscode("REFERENCE".to_owned());
    recorded_later.set_srcuuids(vec![Uuid::from_v8(71)]);
    recorded_later.set_prevuuid(Some(Uuid::from_v8(3)));
    recorded_later.set_prevunix(Some(19));
    recorded_later.set_snapunix(Some(21));

    for merged in [
        event_time_later
            .clone()
            .merge_with(&recorded_later)
            .expect("the recording-selected reference moves the event"),
        recorded_later
            .clone()
            .merge_with(&event_time_later)
            .expect("the other statement contributes facts"),
    ] {
        assert_eq!(merged.get_transunix(), 20);
        assert_eq!(merged.get_hashcode(), 0xB);
        assert_eq!(merged.get_crosscode(), "REFERENCE");
        assert_eq!(
            merged.get_srcuuids(),
            [Uuid::from_v8(70), Uuid::from_v8(71)]
        );
        assert_eq!(merged.get_prevuuid(), Some(Uuid::from_v8(3)));
        assert_eq!(merged.get_prevunix(), Some(19));
        assert_eq!(merged.get_snapunix(), Some(21));
        // The reference selects conflicts; the wire clock still folds
        // to the earliest, and the reference's own 200 is kept nowhere.
        assert_eq!(merged.get_sendunix(), Some(100));
    }

    let mut unstated = Report::at(2, 40);
    unstated.set_hashcode(0xC);
    let mut stated = Report::at(2, 10);
    stated.set_hashcode(0xD);
    stated.set_sendunix(Some(50));
    let merged = unstated
        .merge_with(&stated)
        .expect("a stated wire clock selects the reference");
    assert_eq!((merged.get_transunix(), merged.get_hashcode()), (10, 0xD));
    assert_eq!(
        merged.get_sendunix(),
        Some(50),
        "the one stated recording is the earliest either knows"
    );

    // Equal wire clocks, or none on either side, fall back to the later
    // event instant, whichever statement merges into which: merged into the
    // earlier, the later one leads; merged into the later, the earlier one
    // moves nothing.
    for sendunix in [Some(50), None] {
        let mut earlier = Report::at(3, 10);
        earlier.set_hashcode(0xE);
        earlier.set_sendunix(sendunix);
        let mut later = Report::at(3, 20);
        later.set_hashcode(0xF);
        later.set_sendunix(sendunix);
        let merged = earlier
            .clone()
            .merge_with(&later)
            .expect("the later instant leads");
        assert_eq!(
            (
                merged.get_transunix(),
                merged.get_hashcode(),
                merged.get_sendunix()
            ),
            (20, 0xF, sendunix)
        );
        assert!(later.merge_with(&earlier).is_none(), "{sendunix:?}");
    }

    // An exact tie - the same recording and the same instant - keeps this
    // statement as the reference: the other only fills what it leaves
    // unstated, and adds nothing at all where it states nothing more.
    let mut this = Report::at(4, 10);
    this.set_hashcode(0xA);
    this.set_sendunix(Some(50));
    this.set_crosscode("THIS".to_owned());
    this.finalize();
    let mut that = Report::at(4, 10);
    that.set_hashcode(0xB);
    that.set_sendunix(Some(50));
    that.set_crosscode("THAT".to_owned());
    that.finalize();
    assert!(this.clone().merge_with(&that).is_none());
    assert!(that.clone().merge_with(&this).is_none());
    that.set_srcuuids(vec![Uuid::from_v8(77)]);
    let merged = this.merge_with(&that).expect("the other fills a source");
    assert_eq!(merged.get_hashcode(), 0xA);
    assert_eq!(merged.get_crosscode(), "THIS");
    assert_eq!(merged.get_srcuuids(), [Uuid::from_v8(77)]);
}

#[test]
fn a_folded_statement_ranks_by_its_earliest_sendunix_so_three_way_folds_depend_on_order() {
    // A fold keeps the earliest `sendunix` its statements know and no
    // separate clock of the reference it chose, so against a third
    // statement it ranks by that earliest `sendunix`. The reference of three
    // statements is therefore the later recorded of the pair folded last -
    // merging is not associative in its choice of reference.
    let observation = |unix, sendunix, hashcode, crosscode: &str| {
        let mut event = Report::at(1, unix);
        event.set_sendunix(Some(sendunix));
        event.set_hashcode(hashcode);
        event.set_crosscode(crosscode.to_owned());
        event
    };
    let oldest = observation(30, 100, 0xA, "OLD");
    let latest = observation(20, 200, 0xB, "LATEST");
    let middle = observation(40, 150, 0xC, "MIDDLE");
    let observations = [&oldest, &latest, &middle];

    // Folded last, `middle` (150) meets the pair of `oldest` and `latest`,
    // which held `latest` (200) as its reference but ranks at its earliest
    // recording (100), and so leads it. Folded last into the pair of
    // `latest` and `middle`, which ranks at 150, `oldest` (100) does not
    // lead. Folded last, `latest` (200) leads either pair.
    for (order, reference) in [
        ([0, 1, 2], (40, 0xC, "MIDDLE")),
        ([1, 0, 2], (40, 0xC, "MIDDLE")),
        ([0, 2, 1], (20, 0xB, "LATEST")),
        ([2, 0, 1], (20, 0xB, "LATEST")),
        ([1, 2, 0], (20, 0xB, "LATEST")),
        ([2, 1, 0], (20, 0xB, "LATEST")),
    ] {
        let mut merged = observations[order[0]].clone();
        for index in &order[1..] {
            if let Some(next) = merged.clone().merge_with(observations[*index]) {
                merged = next;
            }
        }
        assert_eq!(
            (
                merged.get_transunix(),
                merged.get_hashcode(),
                merged.get_crosscode()
            ),
            reference,
            "order {order:?}"
        );
        assert_eq!(
            merged.get_sendunix(),
            Some(100),
            "every order keeps the earliest sendunix, order {order:?}"
        );
    }
}

#[test]
fn a_walk_over_events_reaches_the_first_of_a_chain_by_identity() {
    // A caller resolves predecessors by identity, so a chain is a map of
    // them, walked one step back at a time.
    let mut first = Report::at(1, 10);
    first.set_state(filled());
    let mut second = Report::at(2, 20);
    second.set_prevuuid(Some(first.get_uuid()));
    let mut third = Report::at(3, 30);
    third.set_prevuuid(Some(second.get_uuid()));

    let held: Vec<Box<dyn Event>> =
        vec![Box::new(first), Box::new(second), Box::new(third.clone())];
    let by_uuid = |uuid: Uuid| held.iter().find(|event| event.get_uuid() == uuid);

    let mut at: &dyn Event = &third;
    let mut walked = vec![at.get_transunix()];
    while let Some(previous) = at.get_prevuuid() {
        at = by_uuid(previous)
            .expect("a predecessor is an event of the chain")
            .as_ref();
        walked.push(at.get_transunix());
    }
    assert_eq!(walked, [30, 20, 10]);
    assert!(
        at.get_state().is_done(),
        "the first event reached its terminal state"
    );
}

#[test]
fn the_millisecond_sequence_and_seeded_content_derive_one_time_ordered_identity() {
    let mut event = Report::at(1, at(0));
    event.set_hashcode(0xCAFE);
    let held = event.txhash().expect("an instant a TxHash holds");
    assert_eq!(held.unix(), at(0));
    assert_eq!(held.digest().as_u64(), Some(0xCAFE));

    // The same inputs derive the same UUIDv7. TxHash keeps its own exact
    // microsecond/64-bit projection; generic events use the sequenced layout.
    let identity = event.time_uuid().expect("an identity");
    assert_eq!(event.time_uuid().expect("an identity"), identity);
    assert_ne!(identity, held.into_uuid().expect("the TxHash's own UUID"));
    assert_eq!(identity.version(), 7);
    assert_eq!(
        decoded(identity),
        (at(0) / MS, 0, uuid_payload(0xCAFE, 0, 0))
    );

    // Sub-millisecond precision no longer occupies the order lane.
    let mut within = event.clone();
    within.set_transunix(at(0) + MS - 1);
    assert_eq!(within.time_uuid().expect("an identity"), identity);

    // The millisecond leads every other field. Inside one millisecond the
    // sequence leads the hashed payload, so its order is deterministic even
    // though content hashes need not order.
    let mut later = event.clone();
    later.set_transunix(at(0) + MS);
    later.set_hashcode(u64::MAX);
    assert!(later.time_uuid().expect("an identity") > identity);
    let mut sequenced = event.clone();
    sequenced.set_seqnum(1);
    let sequenced_uuid = sequenced.time_uuid().expect("an identity");
    assert!(sequenced_uuid > identity);
    assert_eq!(
        decoded(sequenced_uuid),
        (at(0) / MS, 1, uuid_payload(0xCAFE, 0, 1))
    );

    // The content is hashed into rand_b and the cross hash is its seed.
    let mut recoded = event.clone();
    recoded.set_hashcode(0xCAFF);
    assert_ne!(recoded.time_uuid().unwrap(), identity);
    let mut crossed = event.clone();
    crossed.set_crosshashcode(0xBEEF);
    let crossed_uuid = crossed.time_uuid().unwrap();
    assert_ne!(crossed_uuid, identity);
    assert_eq!(
        decoded(crossed_uuid),
        (at(0) / MS, 0, uuid_payload(0xCAFE, 0xBEEF, 0))
    );

    // rand_a saturates, but the full sequence remains in the seeded payload:
    // values beyond twelve bits share the terminal order band, not an UUID.
    let mut last_exact = event.clone();
    last_exact.set_seqnum(4_094);
    let mut terminal = event.clone();
    terminal.set_seqnum(4_095);
    let mut overflow = event.clone();
    overflow.set_seqnum(4_096);
    let terminal_uuid = terminal.time_uuid().unwrap();
    let overflow_uuid = overflow.time_uuid().unwrap();
    assert!(last_exact.time_uuid().unwrap() < terminal_uuid);
    assert_eq!(decoded(terminal_uuid).1, 4_095);
    assert_eq!(decoded(overflow_uuid).1, 4_095);
    assert_eq!(decoded(overflow_uuid).2, uuid_payload(0xCAFE, 0, 4_096));
    assert_ne!(terminal_uuid, overflow_uuid);

    // Every instant the count holds - the last one is in 2262 - a UUIDv7
    // holds too; one before the epoch has no UUIDv7 to derive.
    let mut far = event.clone();
    far.set_transunix(i64::MAX);
    assert!(far.txhash().is_ok());
    assert!(far.time_uuid().expect("an identity") > identity);
    let mut before = event.clone();
    before.set_transunix(-1);
    assert!(before.txhash().is_ok());
    assert!(before.time_uuid().is_err());

    // Finalized with a code, an event's identity is the one its instant and
    // its code derive. Its cross element is that identity where it states no
    // cross code, and the cross code's own identity otherwise.
    let mut finalized = event.clone();
    finalized.finalized(0xCAFE);
    assert_eq!(finalized.get_hashcode(), 0xCAFE);
    assert_eq!(finalized.get_uuid(), identity);
    assert_eq!(finalized.get_crossuuid(), identity);
    finalized.set_crosscode("O-1".to_owned());
    finalized.sync_cross();
    finalized.finalized(0xCAFE);
    // The cross code moves both its cross element and its `uuid`, because
    // the derived cross hash seeds the payload.
    assert_ne!(finalized.get_uuid(), identity);
    assert_eq!(finalized.get_uuid(), finalized.time_uuid().unwrap());
    assert_eq!(
        finalized.get_crossuuid(),
        Uuid::from_v8(u128::from(crosshash("O-1")))
    );
    // An instant a UUIDv7 cannot hold leaves the identity as it was.
    before.finalized(0xCAFE);
    assert_eq!(before.get_uuid(), Uuid::from_v8(1));
}

#[test]
fn merging_two_incarnations_of_one_identity_unions_their_sources_once() {
    // Two incarnations of one identity, each walked behind a chain of its
    // own and read from lines that overlap: with no wire clocks the
    // later event is the reference, so its predecessor stands, the sources
    // are the sorted unique union, and merged again it moves nothing.
    let first = Report::at(1, 10);
    let branch = Report::at(2, 15).with_previous(&first).expect("follows");
    let mut left = Report::at(5, 20).with_previous(&branch).expect("follows");
    left.set_uuid(Uuid::from_v8(5));
    left.set_srcuuids(vec![Uuid::from_v8(71), Uuid::from_v8(70)]);
    let other = Report::at(3, 12).with_previous(&first).expect("follows");
    let mut right = Report::at(5, 30).with_previous(&other).expect("follows");
    right.set_uuid(Uuid::from_v8(5));
    right.set_srcuuids(vec![Uuid::from_v8(72), Uuid::from_v8(71)]);
    let merged = left.clone().merge_with(&right).expect("the same element");
    assert_eq!(
        merged.get_srcuuids(),
        [Uuid::from_v8(70), Uuid::from_v8(71), Uuid::from_v8(72)]
    );
    assert_eq!(merged.get_prevuuid(), Some(other.get_uuid()));
    // Every step of both chains stood at a later instant than the one
    // before it, so each kept its own place; merging keeps the higher.
    assert_eq!(merged.get_seqnum(), 0);
    // Each identity once: the same statement folded again changes nothing,
    // and a fold that changes nothing is no fold.
    assert!(
        merged.clone().merge_with(&right).is_none(),
        "a second merge changes nothing"
    );
}

/// A line holding `body`, dated `unix`: the core's own event, its instant
/// stated rather than captured.
fn line_at(unix: i64, body: &[u8]) -> TextLine {
    let mut line = TextLine::from_bytes(
        0,
        TextBytes::from_bytes(body).expect("a page"),
        Arc::new(TextOptions::new()),
    )
    .expect("a line");
    line.set_transunix(unix);
    line
}

#[test]
fn an_element_answers_the_identities_codes_and_sources_it_was_given() {
    let mut element = line();
    // A line derives its identity until one is stated, and states no cross
    // code until something addresses it or a caller states one.
    assert_eq!(
        element.get_uuid(),
        element.time_uuid().expect("an identity")
    );
    assert_eq!(
        element.get_crosscode(),
        "",
        "no cross code until it states one"
    );
    assert_eq!(element.get_crosshashcode(), 0);

    element.set_uuid(Uuid::from_v8(3));
    assert_eq!(element.get_uuid(), Uuid::from_v8(3));
    element.set_crossuuid(Uuid::from_v8(30));
    assert_eq!(element.get_crossuuid(), Uuid::from_v8(30));
    element.set_hashcode(0xDEAD_BEEF_CAFE_F00D);
    element.set_crosshashcode(0xBEEF);
    assert_eq!(element.get_hashcode(), 0xDEAD_BEEF_CAFE_F00D);
    assert_eq!(element.get_crosshashcode(), 0xBEEF);

    // The cross code is text, and can be unsaid.
    element.set_crosscode("O-100".to_owned());
    assert_eq!(element.get_crosscode(), "O-100", "stated as given");
    element.set_crosscode(String::new());
    assert_eq!(element.get_crosscode(), "");

    // The sources are the element's provenance: a new element was read from
    // nothing, and an empty list states none again.
    assert!(element.get_srcuuids().is_empty());
    let sources = vec![Uuid::from_v8(70), Uuid::from_v8(71)];
    element.set_srcuuids(sources.clone());
    assert_eq!(element.get_srcuuids(), sources.as_slice());
    element.set_srcuuids(Vec::new());
    assert!(element.get_srcuuids().is_empty());
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
    // type holds them: a line derives its cross facts from the code it
    // states, so it is in step already.
    let mut element = line();
    element.set_crosscode("O-100".to_owned());
    assert!(!element.sync_cross(), "derived from the code, so in step");
    assert_eq!(element.get_crosshashcode(), crosshash("O-100"));
    let mut stated = Report::at(2, 10);
    stated.set_crosscode(element.get_crosscode().to_owned());
    stated.sync_cross();
    assert_eq!(element.get_crossuuid(), stated.get_crossuuid());
    assert_eq!(element.get_crosshashcode(), stated.get_crosshashcode());

    // No cross code: the digest is zero and the cross element is the
    // element's own identity, which it follows when that moves.
    report.set_crosscode(String::new());
    assert!(report.sync_cross());
    assert_eq!(report.get_crosshashcode(), 0);
    assert_eq!(report.get_crossuuid(), Uuid::from_v8(1));
    report.set_uuid(Uuid::from_v8(2));
    assert_eq!(report.cross_uuid(), Uuid::from_v8(2));
    assert!(report.sync_cross());
    assert_eq!(report.get_crossuuid(), Uuid::from_v8(2));
    // A stale cross hash code alone is enough to move.
    report.set_crosshashcode(7);
    assert!(report.sync_cross());
    assert_eq!(report.get_crosshashcode(), 0);
}

#[test]
fn an_event_is_after_another_by_its_instant() {
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
    // A line orders by its instant too, then by its place in the object:
    // two lines the header did not date stand in the order they were
    // written.
    let first = line_at(at(10), b"x");
    assert!(line_at(at(20), b"x").is_after(&first));
    assert!(first.is_before(&line_at(at(20), b"x")));
    assert!(!first.is_after(&first) && !first.is_before(&first));
    let mut next = line_at(at(10), b"x");
    next.set_index(1);
    assert!(next.is_after(&first) && first.is_before(&next));
}

#[test]
fn an_event_answers_its_instant_state_and_place_and_is_still_an_element() {
    let mut event = line();
    // Nanoseconds since the epoch, the count every clock the crate reads.
    event.set_transunix(i64::MAX);
    assert_eq!(event.get_transunix(), i64::MAX);
    // Negative instants are before the epoch, and the type holds them.
    event.set_transunix(-1);
    assert_eq!(event.get_transunix(), -1);
    event.set_hashcode(0xDEAD_BEEF_CAFE_F00D);
    assert_eq!(event.get_hashcode(), 0xDEAD_BEEF_CAFE_F00D);

    // A state is never absent: a new event reached none, says so with the
    // code that means exactly that, and moves as the lifecycle does.
    assert_eq!(event.get_state().as_str(), "UNKNOWN");
    assert!(!event.is_execution());
    assert!(event.get_state().is_live(), "not ended, so still live");
    event.set_state(filled());
    assert!(
        event.is_execution(),
        "an event whose state reports a fill reports an execution"
    );
    let mut report = Report::at(1, 10);
    assert!(!report.is_execution());
    report.set_state(filled());
    assert!(
        report.is_execution(),
        "the default reads the lifecycle state"
    );
    assert!(event.get_state().is_done());
    assert_eq!(*event.get_state(), State::Filled);
    assert_eq!(
        event.get_state().rank(),
        80,
        "a filled state sits on the done rank"
    );
    assert_eq!(event.get_seqnum(), 0, "first at its instant");
    event.set_seqnum(4);
    assert_eq!(event.get_seqnum(), 4);

    // One walk reads both traits through the subtrait object.
    event.set_uuid(Uuid::from_v8(7));
    event.set_srcuuids(vec![Uuid::from_v8(1)]);
    let held: &dyn Event = &event;
    assert_eq!(held.get_uuid(), Uuid::from_v8(7));
    assert_eq!(held.get_srcuuids(), [Uuid::from_v8(1)]);
    assert_eq!(held.get_transunix(), -1);
    assert_eq!(held.get_hashcode(), 0xDEAD_BEEF_CAFE_F00D);
    assert_eq!(held.get_seqnum(), 4);
    assert!(held.get_state().is_done());
}

#[test]
fn the_optional_lifecycle_facts_are_stated_only_where_known() {
    let mut event = line_at(40, b"x");
    assert_eq!(event.get_creaunix(), None);
    assert_eq!(event.get_sendunix(), None);
    assert_eq!(event.get_exprunix(), None);
    assert_eq!(event.get_prevunix(), None);
    assert_eq!(event.get_prevuuid(), None);
    assert_eq!(event.get_snapunix(), None);

    event.set_creaunix(Some(35));
    event.set_sendunix(Some(39));
    event.set_exprunix(Some(100));
    event.set_prevunix(Some(30));
    event.set_prevuuid(Some(Uuid::from_v8(3)));
    event.set_snapunix(Some(40));
    assert_eq!(event.get_snapunix(), Some(40));
    assert_eq!(event.get_creaunix(), Some(35));
    assert_eq!(event.get_sendunix(), Some(39));
    assert_eq!(event.get_exprunix(), Some(100));
    assert_eq!(event.get_prevunix(), Some(30));
    assert_eq!(event.get_prevuuid(), Some(Uuid::from_v8(3)));

    // Each fact is unsaid on its own.
    event.set_exprunix(None);
    assert_eq!(event.get_exprunix(), None);
    assert_eq!(event.get_creaunix(), Some(35));
}

#[test]
fn following_carries_the_lifecycle_forward() {
    // Earliest creation and furthest state carry forward; a newer stated
    // expiry replaces the prior deadline, including when it shortens it.
    let mut previous = Report::at(1, 10);
    previous.set_creaunix(Some(5));
    previous.set_exprunix(Some(200));
    previous.set_state(filled());
    let mut next = Report::at(2, 20);
    next.set_creaunix(Some(8));
    next.set_exprunix(Some(100));
    let next = next
        .with_previous(&previous)
        .expect("the later one follows");
    assert_eq!(next.get_creaunix(), Some(5));
    assert_eq!(next.get_exprunix(), Some(100));
    assert!(
        next.get_state().is_done(),
        "the furthest state carries forward"
    );

    // A previous that knows less leaves what the next one knows alone.
    let mut next = Report::at(3, 30);
    next.set_creaunix(Some(25));
    next.set_exprunix(Some(300));
    let bare = Report::at(4, 20);
    let next = next.with_previous(&bare).expect("follows");
    assert_eq!(next.get_creaunix(), Some(25));
    assert_eq!(next.get_exprunix(), Some(300));
    assert!(
        next.get_state().is_live(),
        "a lesser state does not move the next one back"
    );

    // And one that knows more fills what the next one did not state.
    let next = Report::at(5, 40).with_previous(&previous).expect("follows");
    assert_eq!(next.get_creaunix(), Some(5));
    assert_eq!(next.get_exprunix(), Some(200));

    // What the next element itself says moves nowhere: its instant, its
    // code, its sources, and the cross code it states where the
    // predecessor states none - with the cross element in step with it.
    // The predecessor's sources reach it not at all, because provenance
    // travels along no chain.
    let mut own = Report::at(6, 50);
    own.set_hashcode(0xABC);
    own.set_crosscode("Q-1".to_owned());
    own.set_srcuuids(vec![Uuid::from_v8(70)]);
    let mut previous = previous.clone();
    previous.set_srcuuids(vec![Uuid::from_v8(60)]);
    let own = own.with_previous(&previous).expect("follows");
    assert_eq!(own.get_transunix(), 50);
    assert_eq!(own.get_hashcode(), 0xABC);
    assert_eq!(own.get_crosscode(), "Q-1");
    assert_eq!(own.get_crosshashcode(), crosshash("Q-1"));
    assert_eq!(own.get_crossuuid(), own.cross_uuid());
    assert_eq!(own.get_srcuuids(), [Uuid::from_v8(70)]);
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
    let report = report.with_previous(&order).expect("follows");
    assert_eq!(report.get_crosscode(), "O-100");
    assert_eq!(report.get_crosshashcode(), crosshash("O-100"));
    assert_eq!(report.get_crossuuid(), order.get_crossuuid());
    derived(&report, "the report");
    // A follower stating none takes it the same way; one already sharing
    // it moves nothing on that account.
    let nameless = Report::at(3, 30).with_previous(&order).expect("follows");
    assert_eq!(nameless.get_crosscode(), "O-100");
    assert_eq!(nameless.get_crossuuid(), order.get_crossuuid());
    derived(&nameless, "a follower stating none");
    let mut shared = Report::at(4, 40);
    shared.set_crosscode("O-100".to_owned());
    shared.finalize();
    let shared = shared.with_previous(&order).expect("follows");
    // A later instant keeps its own place.
    assert_eq!(shared.get_seqnum(), 0);
    assert_eq!(shared.get_crossuuid(), order.get_crossuuid());
    derived(&shared, "a follower sharing it");

    // The core's own event does the same, and re-derives its identity.
    let mut placed = line_at(at(10), b"x");
    placed.set_crosscode("O-100".to_owned());
    placed.finalize();
    let mut fill = line_at(at(20), b"y");
    fill.set_crosscode("O-999".to_owned());
    fill.finalize();
    let before = fill.get_uuid();
    let fill = fill.with_previous(&placed).expect("follows");
    assert_eq!(fill.get_crosscode(), "O-100");
    assert_eq!(fill.get_crossuuid(), placed.get_crossuuid());
    derived(&fill, "a line");
    assert_eq!(fill.get_prevuuid(), Some(placed.get_uuid()));
    assert_ne!(fill.get_uuid(), before, "followed, so finalized");
    assert_eq!(fill.get_uuid(), fill.time_uuid().expect("an identity"));
    assert!(fill.clone().with_previous(&fill).is_none(), "never itself");
    assert!(
        fill.clone().with_previous(&placed).is_none(),
        "following again changes nothing"
    );
}

#[test]
fn restating_takes_the_live_elements_place_in_its_chain() {
    // The live element: second in its chain, filled.
    let first = Report::at(1, 10);
    let mut live = Report::at(2, 20);
    live.set_crosscode("O-100".to_owned());
    live.set_creaunix(Some(5));
    live.set_state(filled());
    let mut live = live.with_previous(&first).expect("follows");
    live.set_snapunix(Some(20));
    // Its twin, as another hop logged it: the same instant, a cross code
    // of its own, a line of its own and a lifecycle it knows less of.
    live.set_srcuuids(vec![Uuid::from_v8(70)]);
    let mut twin = Report::at(2, 20);
    twin.set_crosscode("O-999".to_owned());
    twin.set_srcuuids(vec![Uuid::from_v8(71)]);
    twin.set_creaunix(Some(8));
    twin.set_exprunix(Some(99));
    let twin = twin.restating(&live);
    // The place the live one holds is the twin's: the chain grows by nothing.
    assert_eq!(twin.get_prevuuid(), Some(Uuid::from_v8(1)));
    assert_eq!(twin.get_prevunix(), Some(10));
    assert_eq!(twin.get_seqnum(), 0);
    assert_eq!(twin.get_snapunix(), Some(20));
    // The chain's cross code is forced, and its codes brought in step.
    assert_eq!(twin.get_crosscode(), "O-100");
    assert_eq!(twin.get_crosshashcode(), crosshash("O-100"));
    assert_eq!(twin.get_crossuuid(), live.get_crossuuid());
    // The sources stay the twin's own: the line it was read from, never the
    // live one's, because provenance travels along no chain.
    assert_eq!(twin.get_srcuuids(), [Uuid::from_v8(71)]);
    // The lifecycle folds: the earliest creation, the latest expiration,
    // the furthest state. What it says of itself - its instant - is its own.
    assert_eq!(twin.get_creaunix(), Some(5));
    assert_eq!(twin.get_exprunix(), Some(99));
    assert!(twin.get_state().is_done());
    assert_eq!(twin.get_transunix(), 20);

    // Two statements of one line finalize to one identity: the twin of a
    // followed line - its chain's cross code adopted, so its identity
    // seeded anew - restates it and is it.
    let mut placed = line_at(at(10), b"x");
    placed.set_crosscode("O-100".to_owned());
    placed.finalize();
    let fill = line_at(at(20), b"y");
    let twin = fill.clone();
    let fill = fill.with_previous(&placed).expect("follows");
    assert_ne!(twin.get_uuid(), fill.get_uuid(), "followed, so moved");
    let twin = twin.restating(&fill);
    assert_eq!(twin.get_uuid(), fill.get_uuid());
    assert_eq!(twin.get_uuid(), twin.time_uuid().expect("an identity"));
}

#[test]
fn restating_and_merging_keep_the_earliest_per_event_instants() {
    let mut one = Report::at(1, 20);
    one.set_state(filled());
    one.set_sendunix(Some(30));
    let mut other = Report::at(1, 20);
    other.set_sendunix(Some(25));

    // Only the earliest `sendunix` survives either fold: `one` is the
    // reference (sent at 30, after 25), but no separate reference clock
    // keeps its 30, so the folded statement ranks at 25 from here on.
    let restated = one.clone().restating(&other);
    assert_eq!(restated.get_sendunix(), Some(25));

    let merged = one.merge_with(&other).expect("the instants moved");
    assert_eq!(merged.get_sendunix(), Some(25));
    assert!(
        merged.clone().merge_with(&other).is_none(),
        "the fold is idempotent"
    );
}

#[test]
fn mutating_a_concrete_events_identity_inputs_reprojects_eagerly() {
    let mut event = line_at(at(0), b"x");
    event.set_hashcode(0xCAFE);
    let uncrossed = event.get_uuid();
    assert_eq!(uncrossed, event.time_uuid().unwrap());
    assert_eq!(event.get_crossuuid(), uncrossed);

    event.set_transunix(at(1));
    assert_eq!(event.get_uuid(), event.time_uuid().unwrap());
    assert_ne!(event.get_uuid(), uncrossed);
    event.set_transunix(at(0));
    assert_eq!(event.get_uuid(), uncrossed);

    // The place at the instant owns rand_a and is also fed whole into
    // rand_b, and it is where the event stands, never what it says: the
    // digest is the same wherever a stream placed it.
    let placed = event.digest_event().as_u64();
    event.set_seqnum(1);
    assert_eq!(event.get_uuid(), event.time_uuid().unwrap());
    assert_ne!(event.get_uuid(), uncrossed);
    assert_eq!(decoded(event.get_uuid()).1, 1);
    assert_eq!(
        event.digest_event().as_u64(),
        placed,
        "the place is outside the digest"
    );
    event.set_seqnum(0);
    assert_eq!(event.get_uuid(), uncrossed);

    // The cross hash both names the cross element and seeds the current
    // identity's payload.
    event.set_crosshashcode(0xBEEF);
    assert_eq!(event.get_uuid(), event.time_uuid().unwrap());
    assert_ne!(event.get_uuid(), uncrossed);
    assert_eq!(decoded(event.get_uuid()).2, uuid_payload(0xCAFE, 0xBEEF, 0));
    assert_eq!(event.get_crossuuid(), Uuid::from_v8(0xBEEF));
    event.set_crosshashcode(0);
    assert_eq!(event.get_uuid(), uncrossed);
    assert_eq!(event.get_crossuuid(), uncrossed);

    event.set_hashcode(0xCAFF);
    assert_eq!(event.get_uuid(), event.time_uuid().unwrap());
    assert_ne!(event.get_uuid(), uncrossed);
    event.set_hashcode(0xCAFE);
    assert_eq!(event.get_uuid(), uncrossed);

    event.finalized(0xCAFF);
    assert_eq!(event.get_hashcode(), 0xCAFF);
    assert_eq!(event.get_uuid(), event.time_uuid().unwrap());
    assert_ne!(event.get_uuid(), uncrossed);
    assert_eq!(event.get_crossuuid(), event.get_uuid());
    event.finalized(0xCAFE);
    assert_eq!(event.get_uuid(), uncrossed);

    // A line states the code as given, and the cross hash is that code's.
    event.set_crosscode("O-100".to_owned());
    assert_eq!(event.get_crosscode(), "O-100");
    assert_eq!(event.get_uuid(), event.time_uuid().unwrap());
    assert_ne!(event.get_uuid(), uncrossed, "the cross seed moved");
    assert_eq!(event.get_crosshashcode(), crosshash("O-100"));
    assert_eq!(
        event.get_crossuuid(),
        Uuid::from_v8(u128::from(crosshash("O-100")))
    );
    event.set_crosscode(String::new());
    assert_eq!(event.get_crosshashcode(), 0);
    assert_eq!(event.get_uuid(), uncrossed);
    assert_eq!(event.get_crossuuid(), uncrossed);
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
    let mut sourced = Report::at(2, 20);
    sourced.set_srcuuids(vec![Uuid::from_v8(70)]);
    let merged = second
        .clone()
        .merge_with(&sourced)
        .expect("one source taken");
    assert_eq!(merged.get_srcuuids(), [Uuid::from_v8(70)]);

    // An identity assigned stays through a change; one derived from the
    // instant and the content code is reset from those inputs.
    assert_eq!(second.get_uuid(), Uuid::from_v8(2));
    let mut derived = Report::at(3, at(30));
    derived.derived = true;
    derived.finalize();
    let before = derived.get_uuid();
    assert_ne!(before, Uuid::from_v8(3));
    let followed = derived.with_previous(&second).expect("follows");
    assert_ne!(followed.get_uuid(), before, "its place moved its code");
    assert_eq!(
        followed.get_uuid(),
        followed.time_uuid().expect("an identity")
    );
    assert_eq!(followed.get_hashcode(), followed.digest_event().as_u64());
    // The core's own event finalizes the same way.
    let line = line_at(at(40), b"x");
    let mut later = line.clone();
    later.set_transunix(at(50));
    later.set_uuid(line.get_uuid());
    let merged = line
        .clone()
        .merge_with(&later)
        .expect("the same line, later");
    assert_eq!(merged.get_transunix(), at(50));
    assert_eq!(merged.get_uuid(), merged.time_uuid().expect("an identity"));
    let same = line.clone();
    assert!(line.merge_with(&same).is_none(), "nothing moved");
}

#[test]
fn the_digest_starts_from_what_an_element_states_and_never_from_when() {
    let event = Report::at(1, 10);
    let same = event.clone();
    let code = |event: &Report| event.digest_event().as_u64();
    // Two elements stating the same things digest alike, whatever the
    // instant, the identity, the codes or the clocks around them.
    let mut moved = same.clone();
    moved.set_transunix(99);
    moved.set_uuid(Uuid::from_v8(7));
    moved.set_hashcode(0xAB);
    moved.set_crosshashcode(0xCD);
    moved.set_crossuuid(Uuid::from_v8(77));
    moved.set_srcuuids(vec![Uuid::from_v8(70)]);
    moved.set_creaunix(Some(1));
    moved.set_sendunix(Some(3));
    moved.set_exprunix(Some(200));
    moved.set_snapunix(Some(10));
    assert_eq!(code(&event), code(&moved));
    // What an element states moves the code: a cross code, the state, the
    // predecessor - never the place, which is where an event stands rather
    // than what it says.
    let mut crossed = same.clone();
    crossed.set_crosscode("O-1".to_owned());
    assert_ne!(code(&event), code(&crossed));
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
}

#[test]
fn the_content_code_ignores_derived_cross_facts_but_the_identity_uses_the_cross_seed() {
    // The content code excludes derived cross facts and provenance. The
    // `uuid` deliberately uses the cross hash as its payload seed;
    // the cross UUID and source identities remain outside it.
    let report = Report::at(1, 10);
    let mut crossed = report.clone();
    crossed.set_crosshashcode(0xCD);
    crossed.set_crossuuid(Uuid::from_v8(77));
    crossed.set_srcuuids(vec![Uuid::from_v8(70)]);
    assert_eq!(
        crossed.digest_event().as_u64(),
        report.digest_event().as_u64()
    );
    assert_ne!(
        crossed.time_uuid().expect("an identity"),
        report.time_uuid().expect("an identity")
    );

    // A line's code is its body's digest: the cross facts and its sources
    // move its identity and never its code, and finalized, a line stating
    // no cross code is back where it was, its sources kept and never fed.
    let stated = line_at(at(10), b"x");
    let mut crossed = stated.clone();
    crossed.set_crosshashcode(0xCD);
    crossed.set_crossuuid(Uuid::from_v8(77));
    crossed.set_srcuuids(vec![Uuid::from_v8(70)]);
    assert_eq!(crossed.get_hashcode(), stated.get_hashcode());
    assert_ne!(
        crossed.time_uuid().expect("an identity"),
        stated.time_uuid().expect("an identity")
    );
    let mut finalized = crossed.clone();
    finalized.finalize();
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
fn following_keeps_its_own_place_unless_the_predecessor_shares_or_passes_its_instant() {
    let mut first = line_at(at(0), b"x");
    first.set_crosscode("O-1".to_owned());
    first.finalize();
    // A later instant is a place of its own: following keeps it.
    let later = line_at(at(1), b"x")
        .with_previous(&first)
        .expect("a later event follows");
    assert_eq!(later.get_seqnum(), 0);
    assert_eq!(later.get_prevuuid(), Some(first.get_uuid()));
    let mut placed = line_at(at(1), b"x");
    placed.set_seqnum(3);
    let placed = placed.with_previous(&first).expect("a later event follows");
    assert_eq!(
        placed.get_seqnum(),
        3,
        "a later event keeps the place its instant gave it"
    );
    // A step at its predecessor's instant stands after it, and keeps a
    // higher place its instant already gave it.
    let mut same = line_at(at(0), b"x");
    same.set_state(State::Filled);
    let same = same
        .with_previous(&first)
        .expect("the same instant follows");
    assert_eq!(same.get_seqnum(), 1);
    let mut ahead = line_at(at(0), b"x");
    ahead.set_state(State::Filled);
    ahead.set_seqnum(5);
    let ahead = ahead
        .with_previous(&first)
        .expect("the same instant follows");
    assert_eq!(ahead.get_seqnum(), 5);
    // At the same instant, the later place's identity sorts after its
    // predecessor's.
    assert!(same.get_uuid() > first.get_uuid());
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::graph_element::instant_places;

    #[test]
    fn a_place_counts_the_contents_of_one_instant_and_restarts_at_the_next() {
        const A: u64 = 0xA;
        const B: u64 = 0xB;
        const C: u64 = 0xC;
        let stream = [
            (10, A),
            (10, B),
            (10, A),
            (10, C),
            (20, A),
            (20, A),
            (10, B),
        ];
        // By order, a run counts its events from zero and the next instant
        // restarts - even an instant the stream stood at before.
        assert_eq!(instant_places(&stream, false), [0, 1, 2, 3, 0, 1, 0]);
        // By content, a content already placed in the run takes its place
        // again.
        assert_eq!(instant_places(&stream, true), [0, 1, 0, 2, 0, 0, 0]);
        // Past the inline places a run keeps counting and still answers a
        // repeated content its place.
        let run: Vec<(i64, u64)> = (0..9)
            .map(|code| (7, code))
            .chain([(7, 5), (7, 8)])
            .collect();
        assert_eq!(
            instant_places(&run, true),
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 5, 8]
        );
        assert!(instant_places(&[], true).is_empty());
    }
}
