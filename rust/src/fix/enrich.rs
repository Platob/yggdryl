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
//! target's field refuses - an identifier of the wrong width, a spelling a
//! code set does not read - is dropped with a deduplicated warning naming
//! the field. The cost of a dropped value is a null column;
//! the cost of a guess is a wrong number nobody can tell from a sent one.
//!
//! # The pass never refuses the message
//!
//! Every step is best effort over a message that already stands: a
//! restatement, an FX detection or a landing of derived values the rebuild
//! refuses leaves the message as it was stated, beside a warning naming the
//! message type and why, and the pass goes on to the next step.
//!
//! # A parse reads the instrument table its door fixed
//!
//! Beside the dictionary, a parse depends on one more piece of reference
//! data: the [`IsinTable`] the door fixed once, on the thread that opened
//! it, from the registry the codec shares. Every message of one reading
//! fills from that one table, no worker reaches the registry's lock, and a
//! learn while the reading runs reaches no message of it. What a parse
//! takes from the table is derived security identifiers only - the ISIN a
//! ticker names on its market, every equivalent, the pair - which reach no
//! field, no wire and no digest: a message's identity is the same with and
//! without a table. Learning, and the market facts a row fills - the
//! ticker, the CFI code, the currency - are the lifecycle's ([`Codes`]).

use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::iter::FusedIterator;
use std::sync::{Arc, Mutex, PoisonError};
use std::vec;

use smol_str::{SmolStr, format_smolstr};

use crate::graph::iterator::order;
use crate::graph::{Element, Event, EventIterator, Market};
use crate::implementer::warned;
use crate::isin_registry::{EconomicMemo, IsinTable, warn_full};
use crate::{Error, IsinRegistry, Result, Scalar, Side, State, Uuid};

use super::msg::{FixMsg, Viewed};
use super::registry::FixRegistry;

/// Fills what `msg` implies, leaving what it stated alone.
///
/// Three steps in order, and the last step of every parse. The message is
/// restated under the dictionary the registry holds; the crate's derivations
/// fill what the message implies, to a fixpoint; and the component's identifier
/// declaration fills the names the message goes by. Every answer lands where
/// the fact lives - a typed fact on its holder, anything else in the row, typed
/// by the dictionary's own field for the tag - so a derived value is
/// indistinguishable from a stated one, and a value the field refuses, such as
/// an identifier of the wrong width, is dropped with a warning. A declared
/// identifier that cannot spell text is left out rather than allowed to refuse
/// the message. A step the rebuild refuses leaves the message as the step found
/// it, beside a warning. The views of the security identifiers the line stated,
/// `viewed`, are resolved once all of that is filled, before the one settle
/// ([`FixMsg::resolve_views`]); the security identifiers `instruments` holds
/// for the message's instrument are derived after it, before the identity is
/// stamped ([`FixMsg::fill_instrument_ids`]).
pub(super) fn enrich(
    registry: &FixRegistry,
    instruments: Option<&IsinTable>,
    msg: FixMsg,
    viewed: Viewed,
) -> FixMsg {
    // Restatement first, and not as a step a caller may skip: every
    // derivation reads a child by its tag or its canonical name, and a child
    // stored under an alias is invisible until it has been canonicalized.
    let mut held = msg;
    if let Err(error) = super::latest::restate(&mut held) {
        warned!(
            "FIX message kept as stated: restating it under the dictionary was refused",
            held.header().msgtype(),
            "{error}"
        );
    }
    enrich_restated(registry, instruments, held, viewed)
}

/// [`enrich`] past its restatement, for a message whose row is already
/// restated.
pub(super) fn enrich_restated(
    registry: &FixRegistry,
    instruments: Option<&IsinTable>,
    msg: FixMsg,
    viewed: Viewed,
) -> FixMsg {
    let mut held = msg;
    // The currency pair a symbol names is detected first, so the rules read
    // the cells it fills.
    detect_forex(registry, &mut held);
    enrich_detected(registry, instruments, held, viewed)
}

/// [`enrich_restated`] past FX detection: the rules to their fixpoint, the
/// facts settled, the instrument's identifiers derived off the table, then
/// the one stamp. A `Symbol(55)` a rule fills is detected as a stated one
/// is: landed alone and detected before any other rule reads the cells its
/// detection fills, the rules then run over what detection filled.
fn enrich_detected(
    registry: &FixRegistry,
    instruments: Option<&IsinTable>,
    msg: FixMsg,
    viewed: Viewed,
) -> FixMsg {
    let mut held = msg;
    let landed = super::native_derivations::derive_all(&held);
    if let Some(symbol) = landed.iter().find(|(tag, _)| *tag == 55).cloned()
        && held.set_each([symbol]).is_ok()
    {
        detect_forex(registry, &mut held);
        super::native_derivations::fill_all(&mut held);
    } else {
        super::native_derivations::land(&mut held, landed);
    }
    held.resolve_views(viewed, false);
    // Settled once, at the end: a built message arrives unsettled, a
    // restatement leaves it so and the writes above land unsettled - so
    // every message's facts are settled here, once, after everything the
    // pass wrote; the table then derives what it holds of the instrument,
    // which no settle restates and no digest reads, and the identity is
    // stamped once over it all.
    held.settle_facts();
    if let Some(table) = instruments {
        held.fill_instrument_ids(table);
    }
    held.stamp_identity();
    held
}

/// Detects the currency pair `msg`'s symbol names, answering whether
/// detection moved anything. A detection the rebuild refuses is warned about
/// and answers that it moved, so the message is settled whole after it.
fn detect_forex(registry: &FixRegistry, msg: &mut FixMsg) -> bool {
    msg.derive_forex(registry.forex_memo())
        .unwrap_or_else(|error| {
            warned!(
                "FIX currency pair not detected: writing what Symbol(55) names was refused",
                msg.header().msgtype(),
                "{error}"
            );
            true
        })
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
    /// A headerless delivery, known by its millisecond and its chain beside
    /// its content - the identity its content derives there, whatever place
    /// a run of that instant gave it.
    Exact {
        millisecond: i64,
        cross: u64,
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
            millisecond: message.get_currunix().div_euclid(1_000_000),
            cross: message.get_crosshashcode(),
            content,
            sequence: header.msgseqnum(),
            capture_session,
            capture_context: message.capture().msgctxid().map(SmolStr::new),
            direction: header.msgdirection().map(SmolStr::new),
        };
    };
    let replay = header.possdupflag() == Some(true)
        || message
            .get_by_tag(97)
            .is_some_and(|value| crate::implementer::bool_of(&value) == Some(true));
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
    // its execution, a trade and its sided executions - which are never
    // observations of one another: the category, the side a sided kind is
    // keyed by and an execution's own chain keep them apart. An unsided
    // message's side is a tag, which keys nothing.
    let identifier = message.session_event_identifier()?;
    let chain = if message.is_execution() {
        message.get_crosscode()
    } else {
        ""
    };
    let kind = message.marketdatakind();
    Some(format_smolstr!(
        "{identifier}\u{1f}{}\u{1f}{}\u{1f}{chain}",
        kind.code(),
        kind.stored_side(message.get_side()).code()
    ))
}

/// One session event while all of its raw observations are collected.
struct SessionEventObservations {
    message: FixMsg,
    others: Vec<FixMsg>,
    /// The first observation's [`session_event_key`], built once where the
    /// observations wait to be walked.
    key: Option<SmolStr>,
}

/// Latest recording first; the event instant breaks absent/equal recording
/// ties exactly as the graph's reference selection does.
fn reference_order(left: &FixMsg, right: &FixMsg) -> Ordering {
    let right_leads = crate::implementer::right_is_reference(
        left.get_recdunix(),
        left.get_currunix(),
        right.get_recdunix(),
        right.get_currunix(),
    );
    if right_leads {
        return Ordering::Greater;
    }
    let left_leads = crate::implementer::right_is_reference(
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
fn merge_session_events(messages: Vec<FixMsg>) -> Vec<FixMsg> {
    let mut positions = HashMap::with_capacity(messages.len().min(4_096));
    let mut merged: Vec<SessionEventObservations> = Vec::with_capacity(messages.len());

    for message in messages {
        let Some(key) = session_event_key(&message) else {
            merged.push(SessionEventObservations {
                message,
                others: Vec::new(),
                key: None,
            });
            continue;
        };
        match positions.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(merged.len());
                merged.push(SessionEventObservations {
                    message,
                    others: Vec::new(),
                    key: None,
                });
            }
            Entry::Occupied(entry) => merged[*entry.get()].others.push(message),
        }
    }

    merged.into_iter().map(fold_observations).collect()
}

/// One session event out of every observation of it: the reference - the
/// latest recorded - with the others folded in. An observation whose
/// content the rebuild refuses to merge folds its clocks, its anomalies and
/// its provenance alone, beside a warning.
///
/// Each fold settles the facts it moved and none stamps the identity: no
/// fold reads the reference's code or identity, which are a function of
/// its facts, so the reference is stamped once, after the last fold that
/// moved anything - the identity stamping at every fold would have left.
fn fold_observations(mut held: SessionEventObservations) -> FixMsg {
    if held.others.is_empty() {
        return held.message;
    }
    held.others.push(held.message);
    held.others.sort_by(reference_order);
    // Every observation's sources, named once: a fold unions them, and a
    // source reaches neither the code nor the identity, so the reference
    // takes the whole union before the first fold and no fold below finds
    // one to add - or settles again for one.
    let sources: Vec<crate::Uuid> = held
        .others
        .iter()
        .flat_map(|observation| observation.get_srcuuids().iter().copied())
        .collect();
    let mut observations = held.others.into_iter();
    // The first is the reference, chosen once over every observation:
    // each fold keeps the earliest recording, so deciding again at
    // every pair would rank the rest against that instead.
    let mut reference = observations.next().expect("one session-event observation");
    reference.set_srcuuids(sources);
    // The distinct contents already merged: a capture logs one event
    // at every hop, mostly as the same row, and merging a content
    // again fills nothing - so a repeat folds its facts alone.
    let mut merged: Vec<FixMsg> = Vec::new();
    let mut unstamped = false;
    for other in observations {
        if merged
            .iter()
            .any(|held| super::latest::same_content(held, &other))
        {
            unstamped |= reference.fold_session_event_facts_unstamped(&other);
            continue;
        }
        match reference.clone().fold_session_event_unstamped(&other) {
            Ok(folded) => {
                reference = folded;
                merged.push(other);
                unstamped = true;
            }
            Err(error) => {
                warned!(
                    "FIX observation's content not merged: the rebuild refused it; its clocks and provenance are folded",
                    other.header().msgtype(),
                    "{error}"
                );
                unstamped |= reference.fold_session_event_facts_unstamped(&other);
            }
        }
    }
    if unstamped {
        reference.stamp_identity();
    }
    reference
}

/// A message stating no `Side(54)` restated with the side of the chain it
/// follows, answering whether it was: the walk joined it to the one live
/// side of its order - a sided kind's, the side its cross code is stored
/// under - so its content states it too, and a row read back, a book
/// folding it, reads the side the walk gave it. A side the message states
/// always stands, a chain stating none lends none, and an unsided chain - a
/// quote's, whose side is each statement's own tag - lends none either. A
/// write the rebuild refuses leaves the side the message stated, `UKNW`,
/// beside a warning. Unsettled: following settles the message after, and
/// so does the walk re-keying one onto its chain
/// ([`Operation::follow_identity`](crate::graph::Operation::follow_identity)).
pub(super) fn inherit_side(current: &mut FixMsg, previous: &FixMsg) -> bool {
    let side = previous.get_side();
    if !previous.is_sided()
        || side == Side::Unknown
        || current.get_side() != Side::Unknown
        || current.get_by_tag(54).is_some_and(|held| !held.is_null())
    {
        return false;
    }
    let Some(code) = side.fix_code() else {
        return false;
    };
    match current.set_each([(54, Scalar::from(code.to_string()))]) {
        Ok(_) => true,
        Err(error) => {
            warned!(
                "FIX side left UKNW: writing the side its chain states was refused",
                current.header().msgtype(),
                "{error}, inheriting {code} on {}",
                current.get_crosscode()
            );
            false
        }
    }
}

/// The order links `current` takes from its exact predecessor, answering
/// whether it took any. A write the rebuild refuses leaves the links the
/// message stated, settled again, beside a warning.
fn inherit_order_links(current: &mut FixMsg, previous: &FixMsg) -> bool {
    super::latest::inherit_order_links(current, previous).unwrap_or_else(|error| {
        warned!(
            "FIX order links not inherited: writing its predecessor's was refused",
            current.header().msgtype(),
            "{error}, following {}",
            previous.get_curruuid()
        );
        // A refusal after one link landed leaves the message unsettled.
        current.settle();
        false
    })
}

/// One FIX message while the generic event walk selects its exact
/// predecessor. The wrapper adds the protocol's predecessor-derived order
/// spellings inside `with_previous`, so the copy retained by the walk and the
/// copy it yields are the same enriched message.
#[derive(Clone)]
struct LifecycleMessage {
    message: FixMsg,
}

impl From<FixMsg> for LifecycleMessage {
    fn from(message: FixMsg) -> Self {
        Self { message }
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
        if message.should_merge_session_event(&previous.message) {
            // An observation the rebuild refuses to merge is walked as it
            // was stated: the chain still reads it, and so does the caller.
            return Some(Self {
                message: message
                    .clone()
                    .merge_session_event(&previous.message)
                    .unwrap_or_else(|error| {
                        warned!(
                            "FIX observation walked unmerged: merging it into the one it repeats was refused",
                            message.header().msgtype(),
                            "{error}, repeating {}",
                            previous.message.get_curruuid()
                        );
                        message
                    }),
            });
        }
        let links = inherit_order_links(&mut message, &previous.message);
        let inherited = inherit_side(&mut message, &previous.message) || links;
        let fallback = inherited.then(|| message.clone());
        let message = match message.with_previous(&previous.message) {
            Some(message) => message,
            None => fallback?,
        };
        Some(Self { message })
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        Some(Self {
            message: self.message.merge_with(&other.message)?,
        })
    }
}

impl Event for LifecycleMessage {
    /// A twin is the live message's arrival again, so what following wrote
    /// into the live one's content - the order links, the side - is
    /// written into the twin's too, before it restates: the two digest
    /// alike.
    fn restating(self, live: &Self) -> Self {
        let mut message = self.message;
        inherit_order_links(&mut message, &live.message);
        inherit_side(&mut message, &live.message);
        Self {
            message: message.restating(&live.message),
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

/// The instrument registry a walk learns into and fills from: its own,
/// starting empty - a second walk learns nothing the first did - or one a
/// codec shares across the walks it runs one after another, locked once per
/// message.
enum Codes {
    Walk(IsinRegistry),
    Shared(Arc<Mutex<IsinRegistry>>),
}

impl Codes {
    /// Learns what `message` states about its instrument - its origin
    /// currency ([`FixMsg::stated_origccy`]), its country of
    /// issue beside it, where it states one its ISIN does not already say
    /// ([`FixMsg::stated_country`]), the instrument it is written on
    /// ([`FixMsg::stated_underlying_isin`]) and the EUSIPA product category
    /// a bridge's key states ([`FixMsg::stated_eusipa`]), lifted nowhere -
    /// then fills
    /// what it left unstated
    /// ([`FixMsg::fill_instrument`]): the identifiers, the ticker, the CFI
    /// code and the currency, settled no further than the market facts
    /// they imply, since a parsed message is already clean and nothing a
    /// fill writes reaches its identity - an economic match only where the
    /// registry states it ([`IsinRegistry::is_economic_match`]), answered
    /// once per short name and currency the walk meets while the
    /// instruments stand still (`memo`). The one lock is held across the
    /// learn and the fill, and the warning a full registry owes is raised
    /// once it is let go of: the host a warning reaches may be waiting on
    /// that very lock.
    fn learn_and_fill(&mut self, message: &mut FixMsg, memo: &mut EconomicMemo) {
        let country = message.stated_country();
        let underlying = message.stated_underlying_isin();
        let product = message.stated_eusipa();
        let origccy = message.stated_origccy();
        let full = match self {
            Self::Walk(registry) => {
                let learned = registry.learn_stating(
                    message,
                    origccy.as_ref(),
                    country.as_ref(),
                    underlying.as_ref(),
                    product,
                );
                message.fill_instrument(registry.as_table(), memo);
                learned.full
            }
            Self::Shared(registry) => {
                let mut registry = registry.lock().unwrap_or_else(PoisonError::into_inner);
                let learned = registry.learn_stating(
                    message,
                    origccy.as_ref(),
                    country.as_ref(),
                    underlying.as_ref(),
                    product,
                );
                message.fill_instrument(registry.as_table(), memo);
                learned.full
            }
        };
        if let Some(max) = full {
            warn_full(max);
        }
    }
}

/// Sorted messages prepared in lifecycle order: retransmissions removed and
/// missing instrument codes learned only from messages already observed.
struct Prepared<I> {
    source: Intake<I>,
    codes: Codes,
    /// The economic matches this walk answered.
    memo: EconomicMemo,
    /// At most one key per distinct delivery in this already collected finite
    /// capture. A late retransmission must remain a repeat after any number of
    /// intervening deliveries; retaining only a recent window loses that fact.
    seen: HashSet<DeliveryKey>,
}

impl<I> Prepared<I> {
    fn new(source: Intake<I>, registry: Option<Arc<Mutex<IsinRegistry>>>) -> Self {
        // Reserve a small capture once, without reserving a giant repeated
        // capture's upper bound. Growth beyond this hint follows unique keys.
        let capacity = source.len_hint().min(4_096);
        Self {
            source,
            codes: registry.map_or_else(|| Codes::Walk(IsinRegistry::new()), Codes::Shared),
            memo: EconomicMemo::default(),
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
            self.codes.learn_and_fill(&mut message, &mut self.memo);
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
///
/// An item the source refused is excluded with a warning; a source failure
/// ends the reading, every bucket still held is walked, and the failure
/// waits for the walk to yield them.
struct Hourly<I> {
    /// The source still read: none once it ended or failed.
    source: Option<I>,
    /// The session events not yet walked, every observation of each, by the
    /// epoch hour of its first observation.
    buckets: BTreeMap<i64, Vec<SessionEventObservations>>,
    /// Where each held session event's observations wait: its hour and slot.
    held: HashMap<SmolStr, (i64, usize)>,
    /// The hour of the message read last: how far the stream has come.
    position: Option<i64>,
    /// The bucket being walked, merged and sorted.
    ready: vec::IntoIter<FixMsg>,
    /// The source failure that ended the reading, shared with the walk,
    /// which yields it after the messages read before it.
    ended: Arc<Mutex<Option<Error>>>,
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
            match self.source.as_mut().and_then(Iterator::next) {
                Some(Ok(message)) => {
                    let hour = message.get_currunix().div_euclid(HOUR_NS);
                    self.position = Some(hour);
                    let key = session_event_key(&message);
                    if let Some(&(held, slot)) = key.as_ref().and_then(|key| self.held.get(key))
                        && let Some(observations) = self
                            .buckets
                            .get_mut(&held)
                            .and_then(|bucket| bucket.get_mut(slot))
                    {
                        observations.others.push(message);
                        continue;
                    }
                    let bucket = self.buckets.entry(hour).or_default();
                    if let Some(key) = &key {
                        self.held.insert(key.clone(), (hour, bucket.len()));
                    }
                    bucket.push(SessionEventObservations {
                        message,
                        others: Vec::new(),
                        key,
                    });
                }
                Some(Err(error)) => {
                    if let Some(error) = super::messages::source_failure(error, UNREAD) {
                        *self.ended.lock().unwrap_or_else(PoisonError::into_inner) = Some(error);
                        self.source = None;
                    }
                }
                None => self.source = None,
            }
            let done = self.source.is_none();
            let Some(hour) = self.walkable(done) else {
                if done {
                    return None;
                }
                continue;
            };
            let bucket = self.buckets.remove(&hour).unwrap_or_default();
            let mut messages = Vec::with_capacity(bucket.len());
            for observations in bucket {
                if let Some(key) = &observations.key {
                    self.held.remove(key);
                }
                messages.push(fold_observations(observations));
            }
            messages.sort_by(order);
            self.ready = messages.into_iter();
        }
    }
}

/// The identities a walk yielded within its deduplication window, so it
/// yields each once.
///
/// An identity is remembered at the `currunix` it was yielded under and
/// forgotten once the walk has yielded a message more than the window after
/// it: the bound is the event time a window spans, never the capture's
/// length. Every identity is keyed once, beside the instant it was yielded
/// at; a sweep drops what fell out of the window whenever the table has
/// doubled since the last one, so remembering costs amortized constant time
/// and the table holds at most twice what the window does.
struct Window {
    /// The window in nanoseconds of event time; nonpositive remembers none.
    span: i64,
    /// Each identity yielded, at the instant it was yielded under.
    held: HashMap<Uuid, i64>,
    /// The latest instant yielded: how far the walk has come.
    watermark: i64,
    /// The size past which the table is swept again.
    sweep_at: usize,
}

impl Window {
    /// The fewest identities a table holds before it is swept at all.
    const SWEEP_FLOOR: usize = 1_024;

    fn new(span: i64) -> Self {
        Self {
            span,
            held: HashMap::new(),
            watermark: i64::MIN,
            sweep_at: Self::SWEEP_FLOOR,
        }
    }

    /// Whether the walk already yielded `message`'s identity within the
    /// window, remembering it where it did not. A grid view is exempt and
    /// never remembered: a view is the live message as of a tick, one per
    /// tick and chain, and the one at the live message's own instant derives
    /// the live message's identity. A nil identity is no identity at all.
    fn repeats(&mut self, message: &FixMsg) -> bool {
        if self.span <= 0 || message.get_snapunix().is_some() {
            return false;
        }
        let uuid = message.get_curruuid();
        if uuid.is_nil() {
            return false;
        }
        let unix = message.get_currunix();
        self.watermark = self.watermark.max(unix);
        let horizon = self.watermark.saturating_sub(self.span);
        match self.held.entry(uuid) {
            // Within the window, inclusive of its edge.
            Entry::Occupied(held) if *held.get() >= horizon => return true,
            Entry::Occupied(mut held) => {
                held.insert(unix);
            }
            Entry::Vacant(held) => {
                held.insert(unix);
            }
        }
        if self.held.len() >= self.sweep_at {
            self.held.retain(|_, at| *at >= horizon);
            self.sweep_at = (self.held.len() * 2).max(Self::SWEEP_FLOOR);
        }
        false
    }
}

/// What the walk warns about an item its source could not read, excluding
/// it.
const UNREAD: &str = "FIX message excluded from the lifecycle: its stream could not read it";

/// A capture walked in event-time order.
///
/// An item the source refused is excluded with a warning and never queued.
/// A source failure ends the intake: the messages read before it are
/// walked and yielded, then the failure, once - collected whole, the
/// capture is read up to it before the walk; read sorted, one hour at a
/// time, the hours still held are walked once it is met.
pub(super) struct Walked<I> {
    walk: EventIterator<LifecycleMessage, Prepared<I>>,
    /// What the walk already yielded within the codec's window.
    window: Window,
    /// The source failure that ended a whole capture's intake.
    failure: Option<Error>,
    /// The source failure that ended an hourly intake, found as the walk
    /// reads on: the one slot shared with it, and allocated only for that
    /// intake.
    reading: Option<Arc<Mutex<Option<Error>>>>,
}

impl<I: Iterator<Item = Result<FixMsg>>> Walked<I> {
    /// The walk of `source`: collected and sorted whole, or, where `sorted`
    /// says the source is already in instant order, read as it comes and
    /// held one hour at a time ([`Hourly`]); a positive `window_ns` yields
    /// each identity once within that span of event time ([`Window`]). The
    /// instruments are learned into `registry` where one is shared, else
    /// into the walk's own.
    pub(super) fn new(
        source: I,
        snapshot_ns: i64,
        sorted: bool,
        window_ns: i64,
        registry: Option<Arc<Mutex<IsinRegistry>>>,
    ) -> Self {
        let mut failure = None;
        let mut reading = None;
        let intake = if sorted {
            let shared = Arc::new(Mutex::new(None));
            reading = Some(Arc::clone(&shared));
            Intake::Hourly(Box::new(Hourly {
                source: Some(source),
                buckets: BTreeMap::new(),
                held: HashMap::new(),
                position: None,
                ready: Vec::new().into_iter(),
                ended: shared,
            }))
        } else {
            let mut messages = Vec::new();
            for held in source {
                match held {
                    Ok(message) => messages.push(message),
                    Err(error) => {
                        failure = super::messages::source_failure(error, UNREAD);
                        if failure.is_some() {
                            break;
                        }
                    }
                }
            }
            let mut messages = merge_session_events(messages);
            messages.sort_by(order);
            Intake::Whole(messages.into_iter())
        };
        Self {
            walk: EventIterator::new(Prepared::new(intake, registry), true)
                .with_snapshot_ns(snapshot_ns)
                .with_placing(true),
            window: Window::new(window_ns),
            failure,
            reading,
        }
    }

    /// The source failure that ended the intake, taken once.
    fn failure(&mut self) -> Option<Error> {
        self.failure.take().or_else(|| {
            self.reading
                .as_ref()?
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
        })
    }
}

impl<I: Iterator<Item = Result<FixMsg>>> Iterator for Walked<I> {
    type Item = Result<FixMsg>;

    fn next(&mut self) -> Option<Result<FixMsg>> {
        loop {
            let Some(held) = self.walk.next() else {
                // Every message read before the source failed is out.
                return self.failure().map(Err);
            };
            // The walk has already moved on it; only its copy goes.
            if self.window.repeats(&held.message) {
                continue;
            }
            return Some(Ok(held.message));
        }
    }
}

impl<I: Iterator<Item = Result<FixMsg>>> FusedIterator for Walked<I> {}

crate::graph::delegate_market!(LifecycleMessage, message);
crate::graph::delegate_operation!(LifecycleMessage, message;
    /// The message's own: the side the chain lends written as `Side(54)`,
    /// [`inherit_side`] as following writes it, so the row and the digest
    /// state it, then the chain's stored cross code, the cross codes in
    /// step - unsettled, for the walk to settle once.
    fn follow_identity(&mut self, live: &Self) -> bool {
        crate::graph::Operation::follow_identity(&mut self.message, &live.message)
    }
);

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/fix/enrich.rs` pins and a caller cannot reach.
    //!
    //! A redated message settles its clock alone; the whole pass over a
    //! parsed message is what that stands for, so it is forwarded here for
    //! the test that holds the two to one answer.
    use std::sync::Arc;

    use super::FixMsg;

    /// [`FixMsg::dated_by_transaction`] through the whole enriching pass,
    /// with no instrument table.
    pub fn dated_by_transaction_whole(mut msg: FixMsg) -> FixMsg {
        if !msg.redate_by_transaction() {
            return msg;
        }
        let registry = Arc::clone(msg.registry());
        super::enrich_restated(&registry, None, msg, Default::default())
    }
}
