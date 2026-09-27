//! The fields a message implies but did not carry.
//!
//! A venue sends what its counterparty needs and nothing more, so a row is
//! routinely missing values the message itself already determines: an order
//! stating `OrderQty` and `CumQty` has said what `LeavesQty` is, a fill
//! stating `LastQty` and `LastPx` has said what it was worth, and a message
//! naming its instrument by an ISIN has said which country issued it. Every
//! consumer then derives those independently, which is how two systems come
//! to disagree about one message.
//!
//! The specification tabulates these rather than stating them as arithmetic:
//! FIX 4.4's Appendix D walks an order's whole life and shows what each
//! report carries at every step, FIX 4.2's Appendix O does the same for the
//! fields a foreign exchange trade settles on, and Appendix 6-D lists every
//! `SecurityType` under the ISO 10962 category its CFI code opens with. The
//! code sets say the rest: `SecurityIDSource` names the standard each code
//! stands for, and each standard closes its identifiers with a check digit;
//! the dictionary files every `SecurityType` under the `Product` group it
//! belongs to; `OrdStatus` and `ExecType` spell most of their values alike;
//! `TimeInForce` defines its own absence as a day order; and ISO 6166 opens a
//! number with the two letters ISO 3166 gives its issuing country.
//!
//! # The rules are the crate's
//!
//! Each table is one hard-coded rule of
//! [`native_derivations`](super::native_derivations), keyed by the tag it
//! fills and typed by the registry's own field for that tag. A registry
//! states no rule of its own: what it holds decides how an answer is typed
//! and whether a tag has a field to land in, never which rules run, so every
//! registry fills by the same twenty-nine rules and one lacking a target's
//! field answers nothing there. Every accepted answer is exposed to later
//! rules, the sweep repeats until one writes nothing, and the pass finishes
//! through one [`FixMsg::set_each`], so chains settle in either direction
//! without rebuilding the message between rules. Nothing is kept per
//! message shape or between messages.
//!
//! # Enrichment never touches the entries
//!
//! The row is the interpretation and the entries are what arrived, so a
//! derived value goes to the row alone. Re-emitting an enriched message
//! therefore reproduces the received line byte for byte, which is the whole
//! reason the two facts are held apart. It also means enrichment is
//! idempotent: a stated value is never overwritten, so a value derived once
//! is a stated value the second time and derives to itself.
//!
//! # A derivation answers only when the answer is certain
//!
//! An absent input answers nothing, so the target stays unfilled; a
//! condition that does not hold answers nothing the same way. A value the
//! target's field refuses - an identifier whose check digit does not close,
//! a spelling a code set does not read - is silence, as one refused
//! [`FixMsg::set`] is. The cost of silence is a null column; the cost of a
//! guess is a wrong number nobody can tell from a sent one.

use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::iter::{Fuse, FusedIterator};
use std::sync::{Arc, Mutex, PoisonError};
use std::vec;

use smol_str::{SmolStr, format_smolstr};

use crate::graph::iterator::order;
use crate::graph::{Element, Event, EventIterator, Market};
use crate::securityid::SecurityIdRegistry;
use crate::{Error, Result, Scalar, Side, State, Uuid};

use super::msg::FixMsg;
use super::registry::FixRegistry;

/// Fills what `msg` implies, leaving what it stated alone.
///
/// Three steps in order, and the last step of every parse. The message is
/// restated under the dictionary the registry holds; the crate's
/// derivations fill what the message implies, to a fixpoint; and the
/// component's identifier declaration fills the names the message goes by.
/// Every answer lands
/// where the fact lives - a typed fact on its holder, anything else in the
/// row, typed by the dictionary's own field for the tag - so a derived
/// value is indistinguishable from a stated one, and a value the field
/// refuses, such as an identifier whose check digit does not close, is
/// silence. A declared identifier that cannot spell text is silence too:
/// it is left out rather than allowed to refuse the message.
pub(super) fn enrich(registry: &FixRegistry, msg: FixMsg) -> crate::Result<FixMsg> {
    // Restatement first, and not as a step a caller may skip: every
    // derivation reads a child by its tag or its canonical name, and a child
    // stored under an alias is invisible until it has been canonicalized.
    let mut held = msg;
    super::latest::restate(&mut held)?;
    enrich_restated(registry, held)
}

/// [`enrich`] past its restatement, for a message whose row is already
/// restated: a message redated keeps the row it was built with, and what
/// its new clock can move is a derivation and its identity - never a rule,
/// which reads the row and not the clock.
pub(super) fn enrich_restated(registry: &FixRegistry, msg: FixMsg) -> crate::Result<FixMsg> {
    let mut held = msg;
    // The currency pair a symbol names is detected first, so the rules read
    // the cells it fills.
    held.derive_forex(registry.forex_memo())?;
    super::native_derivations::fill_all(&mut held)?;
    // Settled once, at the end: a built message arrives unsettled, a
    // restatement leaves it so and the writes above land unsettled - so
    // every message is settled here, once, after everything the pass wrote.
    held.settle();
    Ok(held)
}

#[derive(Debug, PartialEq, Eq, Hash)]
enum DeliveryKey {
    Session {
        beginstring: SmolStr,
        sender: SmolStr,
        target: SmolStr,
        sender_sub: Option<SmolStr>,
        target_sub: Option<SmolStr>,
        sender_location: Option<SmolStr>,
        target_location: Option<SmolStr>,
        capture_session: Option<SmolStr>,
        capture_context: Option<SmolStr>,
        sequence: u64,
        original_time: i64,
        content: u64,
    },
    /// A headerless bridge row can only prove an exact repeated event. Its
    /// capture facts keep equal content observed in distinct contexts apart.
    Exact {
        uuid: crate::Uuid,
        content: u64,
        sequence: Option<u64>,
        capture_session: Option<SmolStr>,
        capture_context: Option<SmolStr>,
        direction: Option<SmolStr>,
    },
}

fn text(message: &FixMsg, tag: i32) -> Option<SmolStr> {
    message
        .get_by_tag(tag)
        .and_then(|value| value.as_str().map(SmolStr::new))
}

fn delivery_key(message: &FixMsg) -> DeliveryKey {
    let header = message.header();
    let content = message.get_currhashcode();
    let capture_session = message.capture().msgsessionid().map(SmolStr::new);
    let (Some(sender), Some(target), Some(sequence)) = (
        header.sendercompid(),
        header.targetcompid(),
        header.msgseqnum(),
    ) else {
        return DeliveryKey::Exact {
            uuid: message.get_curruuid(),
            content,
            sequence: header.msgseqnum(),
            capture_session,
            capture_context: message.capture().msgctxid().map(SmolStr::new),
            direction: header.msgdirection().map(SmolStr::new),
        };
    };
    let replay = header.possdupflag() == Some(true)
        || message.get_by_tag(97).is_some_and(|value| {
            value.as_bool() == Some(true)
                || value
                    .as_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case("Y"))
        });
    let sending_time = if header.stated_sendingtime() {
        header.sendingtime()
    } else {
        message.get_currunix()
    };
    let original_time = if replay {
        message
            .get_by_tag(122)
            .and_then(|value| value.temporal_count_at(crate::TimeUnit::Nanosecond))
            .unwrap_or(sending_time)
    } else {
        sending_time
    };
    DeliveryKey::Session {
        beginstring: SmolStr::new(header.beginstring()),
        sender: SmolStr::new(sender),
        target: SmolStr::new(target),
        sender_sub: text(message, 50),
        target_sub: text(message, 57),
        sender_location: text(message, 142),
        target_location: text(message, 143),
        capture_session,
        capture_context: message.capture().msgctxid().map(SmolStr::new),
        sequence,
        original_time,
        content,
    }
}

/// The prebuilt identity that proves two rows are observations of one FIX
/// session event. Lifecycle outputs keep their source key but are not raw
/// observations to coalesce on replay.
fn session_event_key(message: &FixMsg) -> Option<SmolStr> {
    // A lifecycle output is already placed. Expiries keep their source
    // message's capture key, and snapshots keep it while adding a view
    // clock; neither is another raw observation to coalesce on replay.
    if message.get_prevuuid().is_some() || message.get_snapunix().is_some() {
        return None;
    }
    // One delivery the parse split holds several messages - a report and
    // its execution, a trade and its sided executions, a quote and its
    // sided quotes - which are never observations of one another: the
    // category, the side and an execution's own chain keep them apart.
    let identifier = message.session_event_identifier()?;
    let chain = if message.is_execution() {
        message.get_crosscode()
    } else {
        ""
    };
    Some(format_smolstr!(
        "{identifier}\u{1f}{}\u{1f}{}\u{1f}{chain}",
        message.msgcat().code(),
        message.get_side().code()
    ))
}

/// One session event while all of its raw observations are collected.
struct SessionEventObservations {
    message: FixMsg,
    others: Vec<FixMsg>,
}

/// Latest recording first; the event instant breaks absent/equal recording
/// ties exactly as the graph's reference selection does.
fn reference_order(left: &FixMsg, right: &FixMsg) -> Ordering {
    let right_leads = crate::graph::element::right_is_reference(
        left.get_recdunix(),
        left.get_currunix(),
        right.get_recdunix(),
        right.get_currunix(),
    );
    if right_leads {
        return Ordering::Greater;
    }
    let left_leads = crate::graph::element::right_is_reference(
        right.get_recdunix(),
        right.get_currunix(),
        left.get_recdunix(),
        left.get_currunix(),
    );
    if left_leads {
        Ordering::Less
    } else {
        Ordering::Equal
    }
}

/// Fully merges observations carrying one complete session-event identity before
/// the lifecycle walk can mistake them for successive events. The most
/// recently recorded message is the retained FIX row; the graph fold unions
/// the other observations into it and keeps the earliest per-event clocks.
fn merge_session_events(messages: Vec<FixMsg>, failures: &mut VecDeque<Error>) -> Vec<FixMsg> {
    let mut positions = HashMap::with_capacity(messages.len().min(4_096));
    let mut merged: Vec<SessionEventObservations> = Vec::with_capacity(messages.len());

    for message in messages {
        let Some(key) = session_event_key(&message) else {
            merged.push(SessionEventObservations {
                message,
                others: Vec::new(),
            });
            continue;
        };
        match positions.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(merged.len());
                merged.push(SessionEventObservations {
                    message,
                    others: Vec::new(),
                });
            }
            Entry::Occupied(entry) => merged[*entry.get()].others.push(message),
        }
    }

    merged
        .into_iter()
        .map(|held| fold_observations(held, failures))
        .collect()
}

/// One session event out of every observation of it: the reference - the
/// latest recorded - with the others folded in, a fold that fails reported
/// and passed over.
fn fold_observations(mut held: SessionEventObservations, failures: &mut VecDeque<Error>) -> FixMsg {
    if held.others.is_empty() {
        return held.message;
    }
    held.others.push(held.message);
    held.others.sort_by(reference_order);
    let mut observations = held.others.into_iter();
    // The first is the reference, chosen once over every observation:
    // each fold keeps the earliest recording, so deciding again at
    // every pair would rank the rest against that instead.
    let mut reference = observations.next().expect("one session-event observation");
    // The distinct contents already merged: a capture logs one event
    // at every hop, mostly as the same row, and merging a content
    // again fills nothing - so a repeat folds its facts alone.
    let mut merged: Vec<FixMsg> = Vec::new();
    for other in observations {
        if merged
            .iter()
            .any(|held| super::latest::same_content(held, &other))
        {
            reference.fold_session_event_facts(&other);
            continue;
        }
        match reference.clone().fold_session_event(&other) {
            Ok(folded) => {
                reference = folded;
                merged.push(other);
            }
            Err(error) => {
                failures.push_back(error);
            }
        }
    }
    reference
}

/// A message stating no `Side(54)` restated with the side of the chain it
/// follows: the walk joined it to the one live side of its order, and its
/// cross code already carries that side, so its content states it too - a
/// row read back, a book folding it, reads the side the walk gave it. A side
/// the message states always stands, and a chain stating none lends none.
fn inherit_side(current: &mut FixMsg, previous: &FixMsg) -> Result<bool> {
    let side = previous.get_side();
    if side == Side::Unknown
        || current.get_side() != Side::Unknown
        || current.get_by_tag(54).is_some_and(|held| !held.is_null())
    {
        return Ok(false);
    }
    let Some(code) = side.fix_code() else {
        return Ok(false);
    };
    current.set_each([(54, Scalar::from(code.to_string()))])?;
    Ok(true)
}

/// One FIX message while the generic event walk selects its exact
/// predecessor. The wrapper adds the protocol's predecessor-derived order
/// spellings inside `with_previous`, so the copy retained by the walk and the
/// copy it yields are the same enriched message.
#[derive(Clone)]
struct LifecycleMessage {
    message: FixMsg,
    failure: Option<SmolStr>,
}

impl From<FixMsg> for LifecycleMessage {
    fn from(message: FixMsg) -> Self {
        Self {
            message,
            failure: None,
        }
    }
}

impl Element for LifecycleMessage {
    fn get_curruuid(&self) -> Uuid {
        self.message.get_curruuid()
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.message.set_curruuid(curruuid);
    }

    fn get_crossuuid(&self) -> Uuid {
        self.message.get_crossuuid()
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.message.set_crossuuid(crossuuid);
    }

    fn get_crosscode(&self) -> &str {
        self.message.get_crosscode()
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.message.set_crosscode(crosscode);
    }

    fn get_currhashcode(&self) -> u64 {
        self.message.get_currhashcode()
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.message.set_currhashcode(hashcode);
    }

    fn get_crosshashcode(&self) -> u64 {
        self.message.get_crosshashcode()
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.message.set_crosshashcode(crosshashcode);
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        self.message.get_srcuuids()
    }

    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
        self.message.set_srcuuids(sources);
    }

    fn is_after(&self, other: &Self) -> bool {
        self.message.is_after(&other.message)
    }

    fn finalize(&mut self) {
        self.message.finalize();
    }

    fn with_previous(self, previous: &Self) -> Option<Self> {
        let mut message = self.message;
        let mut failure = self.failure;
        if message.should_merge_session_event(&previous.message) {
            return match message.clone().merge_session_event(&previous.message) {
                Ok(message) => Some(Self { message, failure }),
                Err(error) => {
                    if failure.is_none() {
                        failure = Some(format_smolstr!("{error}"));
                    }
                    Some(Self { message, failure })
                }
            };
        }
        let inherited = if failure.is_none() {
            match super::latest::inherit_order_links(&mut message, &previous.message)
                .and_then(|links| Ok(inherit_side(&mut message, &previous.message)? || links))
            {
                Ok(changed) => changed,
                Err(error) => {
                    failure = Some(format_smolstr!("{error}"));
                    false
                }
            }
        } else {
            false
        };
        let fallback = inherited.then(|| message.clone());
        let message = match message.with_previous(&previous.message) {
            Some(message) => message,
            None => fallback?,
        };
        Some(Self { message, failure })
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        Some(Self {
            message: self.message.merge_with(&other.message)?,
            failure: self.failure.or_else(|| other.failure.clone()),
        })
    }
}

impl Event for LifecycleMessage {
    fn restating(self, live: &Self) -> Self {
        Self {
            message: self.message.restating(&live.message),
            failure: self.failure.or_else(|| live.failure.clone()),
        }
    }

    fn get_currunix(&self) -> i64 {
        self.message.get_currunix()
    }

    fn set_currunix(&mut self, unix: i64) {
        self.message.set_currunix(unix);
    }

    fn get_state(&self) -> &State {
        self.message.get_state()
    }

    fn set_state(&mut self, state: State) {
        self.message.set_state(state);
    }

    fn is_execution(&self) -> bool {
        self.message.is_execution()
    }

    fn get_seqnum(&self) -> u64 {
        self.message.get_seqnum()
    }

    fn set_seqnum(&mut self, seqnum: u64) {
        self.message.set_seqnum(seqnum);
    }

    fn get_creaunix(&self) -> Option<i64> {
        self.message.get_creaunix()
    }

    fn set_creaunix(&mut self, unix: Option<i64>) {
        self.message.set_creaunix(unix);
    }

    fn get_execunix(&self) -> Option<i64> {
        self.message.get_execunix()
    }

    fn set_execunix(&mut self, unix: Option<i64>) {
        self.message.set_execunix(unix);
    }

    fn get_recdunix(&self) -> Option<i64> {
        self.message.get_recdunix()
    }

    fn set_recdunix(&mut self, unix: Option<i64>) {
        self.message.set_recdunix(unix);
    }

    fn get_exprunix(&self) -> Option<i64> {
        self.message.get_exprunix()
    }

    fn set_exprunix(&mut self, unix: Option<i64>) {
        self.message.set_exprunix(unix);
    }

    fn get_prevunix(&self) -> Option<i64> {
        self.message.get_prevunix()
    }

    fn set_prevunix(&mut self, unix: Option<i64>) {
        self.message.set_prevunix(unix);
    }

    fn get_prevuuid(&self) -> Option<Uuid> {
        self.message.get_prevuuid()
    }

    fn set_prevuuid(&mut self, uuid: Option<Uuid>) {
        self.message.set_prevuuid(uuid);
    }

    fn get_snapunix(&self) -> Option<i64> {
        self.message.get_snapunix()
    }

    fn set_snapunix(&mut self, unix: Option<i64>) {
        self.message.set_snapunix(unix);
    }
}

/// Sorted messages prepared in lifecycle order: retransmissions removed and
/// missing instrument codes learned only from messages already observed.
struct Prepared<I> {
    source: Intake<I>,
    codes: SecurityIdRegistry,
    /// At most one key per distinct delivery in this already collected finite
    /// capture. A late retransmission must remain a repeat after any number of
    /// intervening deliveries; retaining only a recent window loses that fact.
    seen: HashSet<DeliveryKey>,
}

impl<I> Prepared<I> {
    fn new(source: Intake<I>) -> Self {
        // Reserve a small capture once, without reserving a giant repeated
        // capture's upper bound. Growth beyond this hint follows unique keys.
        let capacity = source.len_hint().min(4_096);
        // A bridge's instrument names state a listing - `dbi;ISIN_MIC_CCY` -
        // and are no association of the ISIN alone.
        let listings = super::crated::instrument_sources()
            .filter_map(|(_, _, source)| crate::SecType::read(source).ok());
        Self {
            source,
            codes: SecurityIdRegistry::with_listings(listings),
            seen: HashSet::with_capacity(capacity),
        }
    }
}

impl<I: Iterator<Item = Result<FixMsg>>> Iterator for Prepared<I> {
    type Item = LifecycleMessage;

    fn next(&mut self) -> Option<LifecycleMessage> {
        loop {
            let mut message = self.source.next()?;
            if !self.seen.insert(delivery_key(&message)) {
                continue;
            }
            crate::securityid::enrich(&mut self.codes, &mut message);
            return Some(message.into());
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.source {
            Intake::Whole(source) => (0, source.size_hint().1),
            Intake::Hourly(_) => (0, None),
        }
    }
}

impl<I: Iterator<Item = Result<FixMsg>>> FusedIterator for Prepared<I> {}

/// The span a sorted walk holds its messages in: one epoch hour.
const HOUR_NS: i64 = 3_600_000_000_000;

/// Where a walk's messages come from: a finite capture collected and sorted
/// whole, or a source already in instant order, held one hour at a time.
enum Intake<I> {
    Whole(vec::IntoIter<FixMsg>),
    Hourly(Box<Hourly<I>>),
}

impl<I> Intake<I> {
    /// The messages already in hand: the whole capture's count, none for
    /// a source read as it comes.
    fn len_hint(&self) -> usize {
        match self {
            Self::Whole(source) => source.len(),
            Self::Hourly(_) => 0,
        }
    }
}

impl<I: Iterator<Item = Result<FixMsg>>> Iterator for Intake<I> {
    type Item = FixMsg;

    fn next(&mut self) -> Option<FixMsg> {
        match self {
            Self::Whole(source) => source.next(),
            Self::Hourly(source) => source.next(),
        }
    }
}

/// A source its caller states is in instant order, walked one epoch hour at
/// a time: each message waits in the bucket of the hour its instant falls
/// in - an observation of a session event already held, with the others of
/// it, whatever its own hour - and a bucket is walked - its session events
/// folded and its messages sorted, as a whole capture is - once the stream
/// has read a message two hours past it. The hour of grace is what a
/// message the walk dates by its transaction rather than by where it was
/// stored needs to still land in its own hour, and what the hops of one
/// delivery logged across an hour's end need to meet; a message dated before
/// a bucket already walked is walked where it arrives, as any late element
/// is. What is held is the messages of the hours not yet walked - two of
/// them for a source in order - never the stream.
struct Hourly<I> {
    source: Fuse<I>,
    /// The session events not yet walked, every observation of each, by the
    /// epoch hour of its first observation.
    buckets: BTreeMap<i64, Vec<SessionEventObservations>>,
    /// Where each held session event's observations wait: its hour and slot.
    held: HashMap<SmolStr, (i64, usize)>,
    /// The hour of the message read last: how far the stream has come.
    position: Option<i64>,
    /// The bucket being walked, merged and sorted.
    ready: vec::IntoIter<FixMsg>,
    /// What the source could not read, reported by the walk as it goes.
    failures: Arc<Mutex<VecDeque<Error>>>,
}

impl<I: Iterator<Item = Result<FixMsg>>> Hourly<I> {
    /// The first bucket the stream has read past by more than the hour of
    /// grace, or any bucket once the stream is done.
    fn walkable(&self, done: bool) -> Option<i64> {
        let (&hour, _) = self.buckets.first_key_value()?;
        (done
            || self
                .position
                .is_some_and(|position| hour < position.saturating_sub(1)))
        .then_some(hour)
    }
}

impl<I: Iterator<Item = Result<FixMsg>>> Iterator for Hourly<I> {
    type Item = FixMsg;

    fn next(&mut self) -> Option<FixMsg> {
        loop {
            if let Some(message) = self.ready.next() {
                return Some(message);
            }
            let read = self.source.next();
            let done = read.is_none();
            match read {
                Some(Ok(message)) => {
                    let hour = message.get_currunix().div_euclid(HOUR_NS);
                    self.position = Some(hour);
                    let key = session_event_key(&message);
                    if let Some(&(held, slot)) = key.as_ref().and_then(|key| self.held.get(key)) {
                        if let Some(observations) = self
                            .buckets
                            .get_mut(&held)
                            .and_then(|bucket| bucket.get_mut(slot))
                        {
                            observations.others.push(message);
                            continue;
                        }
                    }
                    let bucket = self.buckets.entry(hour).or_default();
                    if let Some(key) = key {
                        self.held.insert(key, (hour, bucket.len()));
                    }
                    bucket.push(SessionEventObservations {
                        message,
                        others: Vec::new(),
                    });
                }
                Some(Err(error)) => self
                    .failures
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push_back(error),
                None => {}
            }
            let Some(hour) = self.walkable(done) else {
                if done {
                    return None;
                }
                continue;
            };
            let bucket = self.buckets.remove(&hour).unwrap_or_default();
            let mut failures = self.failures.lock().unwrap_or_else(PoisonError::into_inner);
            let mut messages = Vec::with_capacity(bucket.len());
            for observations in bucket {
                if let Some(key) = session_event_key(&observations.message) {
                    self.held.remove(&key);
                }
                messages.push(fold_observations(observations, &mut failures));
            }
            drop(failures);
            messages.sort_by(order);
            self.ready = messages.into_iter();
        }
    }
}

/// A capture walked in event-time order. Collected whole, its intake
/// failures are reported before its messages, because sorting consumes the
/// capture first; read sorted, one hour at a time, each is reported as the
/// walk reaches it.
pub(super) struct Walked<I> {
    walk: EventIterator<LifecycleMessage, Prepared<I>>,
    /// What the whole capture's intake could not read, in its order.
    failures: VecDeque<Error>,
    /// What an hourly intake could not read, found as the walk reads on:
    /// the one queue shared with it, and allocated only for that intake.
    reading: Option<Arc<Mutex<VecDeque<Error>>>>,
}

impl<I: Iterator<Item = Result<FixMsg>>> Walked<I> {
    /// The walk of `source`: collected and sorted whole, or, where `sorted`
    /// says the source is already in instant order, read as it comes and
    /// held one hour at a time ([`Hourly`]).
    pub(super) fn new(source: I, snapshot_ns: i64, sorted: bool) -> Self {
        let mut failures = VecDeque::new();
        let mut reading = None;
        let intake = if sorted {
            let shared = Arc::new(Mutex::new(VecDeque::new()));
            reading = Some(Arc::clone(&shared));
            Intake::Hourly(Box::new(Hourly {
                source: source.fuse(),
                buckets: BTreeMap::new(),
                held: HashMap::new(),
                position: None,
                ready: Vec::new().into_iter(),
                failures: shared,
            }))
        } else {
            let mut messages = Vec::new();
            for held in source {
                match held {
                    Ok(message) => messages.push(message),
                    Err(error) => failures.push_back(error),
                }
            }
            let mut messages = merge_session_events(messages, &mut failures);
            messages.sort_by(order);
            Intake::Whole(messages.into_iter())
        };
        Self {
            walk: EventIterator::new(Prepared::new(intake), true).with_snapshot_ns(snapshot_ns),
            failures,
            reading,
        }
    }

    fn failure(&mut self) -> Option<Error> {
        self.failures.pop_front().or_else(|| {
            self.reading
                .as_ref()?
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
        })
    }
}

impl<I: Iterator<Item = Result<FixMsg>>> Iterator for Walked<I> {
    type Item = Result<FixMsg>;

    fn next(&mut self) -> Option<Result<FixMsg>> {
        if let Some(error) = self.failure() {
            return Some(Err(error));
        }
        match self.walk.next() {
            Some(held) => Some(match held.failure {
                Some(reason) => Err(Error::InvalidRecord {
                    path: "fix.lifecycle".into(),
                    reason,
                }),
                None => Ok(held.message),
            }),
            // Reading the source's end can find a last failure.
            None => self.failure().map(Err),
        }
    }
}

impl<I: Iterator<Item = Result<FixMsg>>> FusedIterator for Walked<I> {}

crate::graph::delegate_market!(LifecycleMessage, message);
crate::graph::delegate_operation!(LifecycleMessage, message);
