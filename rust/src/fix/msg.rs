//! A FIX message: its typed facts, its row, and the registry that types it.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use smol_str::{SmolStr, format_smolstr};

use super::build::stated;
use super::entry::{FixEntry, emit_bytes, emit_text, wire_text, wire_text_under};
use super::identity::{self, FixCapture, FixHeader, FixLifted, Typed};
use super::registry::FixMap;
use super::{FixId, FixKey, FixRegistry};
use crate::graph::{
    Element, Event, Lane, Market, MarketEventData, Metadata, Operation, OperationEvent,
    OperationEventData,
};
use crate::idmap::IdMap;
use crate::securityid::{SecType, SecurityId, SecurityIds};
use crate::xxhash;
use crate::{Ccy, CfiCode, Decimal18, MicCode, Side, State, StructType, TimeInForce, Unit, Uuid};
use crate::{DataType, Error, Field, FieldPath, FieldSegment, Result, Scalar, Serie};

/// The nanoseconds in one day: what a transaction time at midnight to the
/// nanosecond is a multiple of, and what a day-only `TransactTime(60)` is
/// restated as.
const NANOS_PER_DAY: i64 = 86_400 * 1_000_000_000;

/// The row stated the state the message reached: the derivation leaves it.
const ROW_STATED_STATE: u16 = 1;

/// The row stated when the message expires: the derivation leaves it.
const ROW_STATED_EXPIRY: u16 = 1 << 1;

/// The row stated this normalized market code, rather than a raw FIX pair
/// from which market derivation could replace it.
const ROW_STATED_ISIN: u16 = 1 << 2;
const ROW_STATED_BLOOMBERG: u16 = 1 << 5;
const ROW_STATED_MIC: u16 = 1 << 6;
const ROW_STATED_FIGI: u16 = 1 << 7;
const ROW_STATED_EXECUTION: u16 = 1 << 8;
const ROW_STATED_RECORDING: u16 = 1 << 9;
const ROW_STATED_MARKET_OPERATION: u16 = 1 << 10;

/// The row-owned facts whose non-null value must survive market derivation.
fn row_stated_bit(tag: i32) -> Option<u16> {
    if tag == super::STATE_TAG_NAME.0 {
        Some(ROW_STATED_STATE)
    } else if tag == super::EXPRTIME_TAG_NAME.0 {
        Some(ROW_STATED_EXPIRY)
    } else if tag == super::ISINCODE_TAG_NAME.0 {
        Some(ROW_STATED_ISIN)
    } else if tag == super::BLOOMBERGCODE_TAG_NAME.0 {
        Some(ROW_STATED_BLOOMBERG)
    } else if tag == super::MICCODE_TAG_NAME.0 {
        Some(ROW_STATED_MIC)
    } else if tag == super::FIGICODE_TAG_NAME.0 {
        Some(ROW_STATED_FIGI)
    } else if tag == super::EXECUNIX_TAG_NAME.0 {
        Some(ROW_STATED_EXECUTION)
    } else if tag == super::RECDUNIX_TAG_NAME.0 {
        Some(ROW_STATED_RECORDING)
    } else if tag == super::MSGCAT_TAG_NAME.0 {
        Some(ROW_STATED_MARKET_OPERATION)
    } else {
        None
    }
}

/// The stable graph operation identifier implied by one FIX message type.
///
/// A custom registry may assign any known business category to its own
/// message type. The category-to-integer mapping itself is crate-owned: those
/// identifiers cross the FIX boundary into generic graph rows and may not
/// change with a dictionary.
fn derived_marketoperationid(registry: &FixRegistry, msgtype: &str) -> i32 {
    let category = registry
        .get_msgtype(msgtype)
        .and_then(super::MsgType::msgcat)
        .or_else(|| super::constants::msgcat_of(msgtype))
        .unwrap_or("UNKN");
    super::constants::msgcat_code(category)
        .or_else(|| super::constants::msgcat_code("UNKN"))
        .expect("the fixed MsgCat table carries UNKN")
}

/// A FIX message: a market event with a FIX body around it.
///
/// Three typed holders and one row. The [`MarketEventData`] is the event
/// the message is - its identity, when it happened, where it stands and the
/// market's facts - and the message answers [`Element`], [`Event`] and
/// [`Market`] through it, so a walk over messages reads them as it
/// reads any event. The [`FixHeader`] is the standard header, typed: the
/// version, the type, who sent it to whom, the sequence number and the
/// sending time. The [`FixCapture`] is what the capture said about the
/// line. The row is everything else the message states - the body's
/// fields, its repeating groups, the keys no dictionary explains - as a
/// core Struct [`Field`] and its value, each child typed by the registry's
/// field for it, and none of the typed facts is in it: every fact is held
/// once. The registry link is an [`Arc`], cloned from
/// [`FixRegistry::global`] when the caller names none, so a message carries
/// the dictionary it was resolved against.
///
/// A value is reached by tag, by identifier, by name, or by path, each
/// answering the value it finds - a typed fact for a tag the holders own,
/// a row child otherwise - with a failing twin; resolution goes through the
/// linked registry, never through a private copy of its rules. An unknown
/// tag is retained rather than dropped: it is looked for under its rendered
/// decimal name, which is where a transcriber keeps a tag no dictionary
/// explains. A repeating-group counter remains an int32 value reached by its
/// tag; the separate collection is reached by name, such as `Parties`.
///
/// The message's identity is settled from what it states: the code is the
/// XXH3-64 of the event's facts, the text, the metadata, the FIX fields it
/// lifted and the named content of the row - everything but the standard
/// header and trailer, less `MsgType`, and never the chain it is in - the
/// UUIDv7 identity ordered by millisecond and sequence with a content payload
/// seeded by the cross hash, and the cross identity the UUIDv8 the cross code's
/// digest derives - the first chain
/// identifier the message spells, `OrderID` before `ClOrdID`. A bridge's
/// bracketed session and context, joined with the message's type and
/// sequence, are the session event it was delivered as,
/// [`FixCapture::msgsesseventid`]: capture provenance that never replaces
/// that chain code. Every write settles it again, so a written code or identity
/// is overwritten by the settled one. [`Self::entries`] is the row read as a tree, for a
/// consumer that walks one shape, and [`Self::into_bytes`] re-emits the
/// message from it.
///
/// Serialization is inherited, not written: `field.clone().into_json()`
/// renders the row's schema, [`into_json_scalar`](crate::into_json_scalar)
/// its value, and [`from_json_scalar_with_field`](crate::from_json_scalar_with_field)
/// reads a value back typed, ordered and canonicalized against that field;
/// [`Self::into_row`] is the whole message as one fixed row.
///
/// ```
/// use std::sync::Arc;
///
/// use yggdryl::graph::{Element, Event};
/// use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, StructType};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut symbol = DataType::utf8().required_field("Symbol");
/// symbol.as_fix_mut().set_tag(55)?;
/// symbol.as_fix_mut().set_names(["Ticker"])?;
/// let mut qty = DataType::Int64.required_field("OrderQty");
/// qty.as_fix_mut().set_tag(38)?;
/// let registry = Arc::new(FixRegistry::from_fields([symbol.clone(), qty.clone()])?);
///
/// let root = DataType::from(StructType::from_fields([symbol, qty, DataType::utf8().nullable_field("9999")])?)
///     .required_field("NewOrderSingle");
/// let value = Scalar::from_struct([
///     ("Symbol", Scalar::from("AAPL")),
///     ("OrderQty", Scalar::from(100)),
///     ("9999", Scalar::from("custom")),
/// ])?;
/// let msg = FixMsg::with_registry(registry, root.clone(), value)?;
///
/// assert_eq!(msg.by_tag(55)?, Scalar::from("AAPL"));
/// assert_eq!(msg.by_name("ticker")?, Scalar::from("AAPL"));
/// assert_eq!(msg.by_tag(9999)?, Scalar::from("custom"), "an unknown tag is kept");
/// // The identity is settled from what the message states.
/// assert_ne!(msg.get_currhashcode(), 0);
/// assert_eq!(msg.get_curruuid(), msg.time_uuid()?);
/// assert_eq!(msg.get_currunix(), msg.header().sendingtime());
///
/// // The row serializes through the paths every field and value share.
/// let root = msg.as_field();
/// let schema = root.clone().into_json()?;
/// assert!(schema.contains("FIX:tag"));
/// # Ok(())
/// # }
/// ```
pub struct FixMsg {
    registry: Arc<FixRegistry>,
    /// The event the message is: what the three graph traits answer.
    ///
    /// Boxed, because the event is forty facts and a message is moved
    /// through every stream by value.
    event: Box<OperationEventData>,
    /// The standard header, typed.
    header: Box<FixHeader>,
    /// What the line said about the capture it was written for, typed: a
    /// bridge's own row header. What the *reader* said about the line is
    /// [`Self::carried`].
    capture: Box<FixCapture>,
    /// The FIX fields the message lifted out of its row: the prices, the
    /// quantities and the identifiers, each exactly as the message stated
    /// it. What the event *derives* from them is the event's.
    lifted: Box<FixLifted>,
    /// Whether a caller or a lifecycle walk wrote a market fact through the
    /// traits, which stops the derivation from answering over it.
    ///
    /// A derived fact is recomputed from the FIX fields on every settle,
    /// because a write can change what they say; a *forced* one is somebody
    /// else's answer - the state a walk folded forward, the price it says
    /// this message moved from - and re-deriving would throw it away.
    forced: bool,
    /// The lifecycle and normalized market facts the row stated. Derivation
    /// leaves them as the row's word, so reconstruction keeps an explicit
    /// value rather than replacing it from a raw FIX field.
    row_stated: u16,
    /// `Text(58)`, where the message carries one.
    text: Option<SmolStr>,
    /// What a bridge stated under its own namespaces - `TECH.CLIENTID`,
    /// `AMON.…` - each under the key as the bridge spelled it, folded, in
    /// sorted order.
    metadata: BTreeMap<SmolStr, SmolStr>,
    /// Each root child's tag beside its position, in tag order.
    ///
    /// Resolved once, because reading a tag out of a child's metadata is a
    /// formatted key and a map lookup, and scanning the children per lookup
    /// pays it once per child.
    tags: Vec<(i32, usize)>,
    /// The first child of each name, built on the first tag no child
    /// declares and kept.
    ///
    /// [`Self::get_by_tag`] has two fallbacks past the index above - the
    /// child named as the dictionary names the tag, and the child named by
    /// the tag's decimal spelling - and both end in a child found by name.
    /// Derived from the field alone, and derived lazily, so a message nobody
    /// projects pays nothing for it.
    named: OnceLock<FixMap<SmolStr, usize>>,
    /// Group positions keyed by their `FIX:counter`, separate from tag values.
    groups: Vec<(i32, usize)>,
    field: Field,
    value: Scalar,
    /// The row read as a tree, derived on the first ask and dropped by
    /// every write. Shared, so a clone - which states the same row - keeps
    /// the tree for the cost of a reference count rather than deriving it
    /// again.
    entries: OnceLock<Arc<[FixEntry]>>,
    /// The capture's own cells: what the row this message was read from
    /// said for itself, each under the column's name. Provenance and never
    /// content - outside the code, the entries and the wire - stated back at
    /// the column of its name by [`Self::into_row`].
    carried: Vec<(SmolStr, Scalar)>,
    /// The security identifiers the message implies rather than states: the
    /// national code an ISIN carries, what a lifecycle's registry learned. The
    /// derived overlay - never on the wire, never in the arrival record - kept
    /// across settles and filling only a key the stated set leaves absent.
    derived: SecurityIds,
    /// What the message states that its reading could not take as it
    /// stands: the parse's refusals first - a value that would not type, a
    /// counter disagreeing with its group - then what a settle dropped,
    /// rebuilt by every settle behind the parse's and folded as a union when
    /// messages merge. Never a column, never a digest input.
    anomalies: Vec<super::FixAnomaly>,
    /// How many of `anomalies` the parse recorded, which every settle keeps.
    arrival_anomalies: usize,
}

/// Whether `held` is the four parts of a session event joined by `:`,
/// compared in place rather than joined again.
fn is_session_event(
    held: &str,
    msgtype: &str,
    session: &str,
    context: &str,
    sequence: u64,
) -> bool {
    let mut rest = held;
    for part in [msgtype, session, context] {
        let Some(after) = rest
            .strip_prefix(part)
            .and_then(|after| after.strip_prefix(':'))
        else {
            return false;
        };
        rest = after;
    }
    rest.parse::<u64>().is_ok_and(|held| held == sequence)
        && rest.bytes().all(|byte| byte.is_ascii_digit())
        && (rest.len() == 1 || !rest.starts_with('0'))
}

/// One child a write lands: replaced at `at`, appended when there is none,
/// its field and value already resolved and typed.
pub(super) struct Write {
    pub(super) at: Option<usize>,
    pub(super) field: Field,
    pub(super) value: Scalar,
}

/// One write a key resolved to: a typed fact the holders own, or a row
/// child.
enum Staged {
    Typed(i32, Scalar),
    Row(Write),
}

/// The refusal a write to one of the capture's own columns earns, named by
/// the column it reached.
///
/// Loud rather than silent: a caller writing `sourceurl` on a message means
/// to state where a line came from, and answering nothing would leave it
/// believing the message says so.
fn refused_capture(name: &str, tag: i32) -> Error {
    identity::refused(
        name,
        "a field a message states",
        format_smolstr!(
            "the capture's own column {name} ({tag}), which whoever read the line states on the row"
        ),
    )
}

/// Adds one staged write to the batch, the later of two writes to one child
/// standing, whether both reached it or both would append it.
fn stage(writes: &mut Vec<Write>, write: Write) {
    let pending = writes.iter().position(|held| match (held.at, write.at) {
        (Some(held), Some(at)) => held == at,
        (None, None) => held.field.name() == write.field.name(),
        _ => false,
    });
    match pending {
        Some(pending) => writes[pending] = write,
        None => writes.push(write),
    }
}

/// What one landed write does to the tag and group indexes.
pub(super) struct Indexed {
    retired_tag: Option<i32>,
    retired_counter: Option<i32>,
    tag: Option<i32>,
    counter: Option<i32>,
    index: usize,
    /// Whether the write put a differently named child in a child's place.
    pub(super) renamed: bool,
    /// Whether the write added a child rather than replacing one.
    pub(super) appended: bool,
}

/// Stage proven writes without publishing a message or inventing a second row.
pub(super) fn stage_writes(
    root: &Field,
    row: &Scalar,
    writes: Vec<Write>,
) -> Result<(Field, Vec<Scalar>, Vec<Indexed>)> {
    let mut members = Vec::with_capacity(root.fields().len() + writes.len());
    members.extend_from_slice(root.fields());
    let held = row.as_sequence().unwrap_or_default();
    let mut values = Vec::with_capacity(held.len() + writes.len());
    values.extend_from_slice(held);
    let mut indexed: Vec<Indexed> = Vec::with_capacity(writes.len());
    for Write { at, field, value } in writes {
        let (index, replaced) = match at {
            Some(at) if at < members.len() && at < values.len() => {
                let replaced = std::mem::replace(&mut members[at], field);
                values[at] = value;
                (at, Some(replaced))
            }
            _ => {
                members.push(field);
                values.push(value);
                (members.len() - 1, None)
            }
        };
        let renamed = replaced
            .as_ref()
            .is_some_and(|replaced| replaced.name() != members[index].name());
        let appended = replaced.is_none();
        let held = replaced.as_ref().map(Field::as_fix);
        let written = members[index].as_fix();
        // A tag is resolved here as the column plan resolves one, so the
        // index a write leaves is the index a construction would have built:
        // a child named after a tag the dictionary does not explain answers
        // for that tag, and a reader that walks the index rather than the
        // names finds it.
        let resolved = |field: &Field| {
            field
                .as_fix()
                .tag()
                .ok()
                .flatten()
                .or_else(|| super::field::parse_tag(field.name()))
        };
        indexed.push(Indexed {
            retired_tag: replaced.as_ref().and_then(resolved),
            retired_counter: held.as_ref().and_then(|held| held.counter().ok().flatten()),
            tag: resolved(&members[index]),
            counter: written.counter().ok().flatten(),
            index,
            renamed,
            appended,
        });
    }
    // A write replaces the child it reached or appends a field no child is
    // named as, so the members stay named once.
    let dtype = DataType::from(StructType::from_unique_fields(members));
    let field = Field::new_with_metadata(
        root.name(),
        dtype,
        root.is_nullable(),
        root.as_metadata().clone(),
    );
    Ok((field, values, indexed))
}

impl Indexed {
    pub(super) fn apply(
        &self,
        tags: &mut Vec<(i32, usize)>,
        groups: Option<&mut Vec<(i32, usize)>>,
    ) {
        retire(tags, self.retired_tag, self.index);
        admit(tags, self.tag, self.index);
        if let Some(groups) = groups {
            retire(groups, self.retired_counter, self.index);
            admit(groups, self.counter, self.index);
        }
    }
}

/// Each child's resolved tag beside its position, sorted for a binary search.
fn tag_positions(columns: &[super::schema::Column]) -> Vec<(i32, usize)> {
    let mut held = Vec::with_capacity(columns.len());
    held.extend(
        columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| Some((column.tag?, index))),
    );
    held.sort_unstable();
    held
}

/// Two instants standing no further apart than `delay`, in either
/// direction: what makes an official clock and a sending clock the one
/// event said twice. A nonpositive delay admits only equality.
fn within(unix: i64, reference: i64, delay: i64) -> bool {
    unix.abs_diff(reference) <= delay.unsigned_abs()
}

/// The rank `TransactTime(60)` takes among the official clocks: what the
/// message itself says about when its transaction happened, which no stamp
/// of another party outranks.
const TRANSACT_RANK: u8 = 0;

/// The `TrdRegTimestampType(770)` codes that stamp the event this message
/// reports - when it executed, when it entered the book, when it took its
/// priority, when it was submitted, cancelled or modified. These are the
/// venue's own answer to the question `currunix` asks.
const EVENT_TRDREG_TYPES: [i64; 9] = [
    1,  // ExecutionTime
    5,  // BrokerExecution
    8,  // TimePriority
    9,  // OrderbookEntryTime
    10, // OrderSubmissionTime
    29, // OrderCancellationTime
    30, // OrderModificationTime
    32, // TradeCancellationTime
    33, // TradeModificationTime
];

/// The `TrdRegTimestampType(770)` codes that stamp a hop the message
/// crossed on its way here rather than the event itself. Nearer the event
/// than the sending clock and further from it than the stamps above, which
/// is exactly the rank they take.
const HOP_TRDREG_TYPES: [i64; 5] = [
    2,  // TimeIn
    3,  // TimeOut
    4,  // BrokerReceipt
    6,  // DeskReceipt
    31, // OrderRoutingTime
];

/// How well one `TrdRegTimestampType(770)` answers when the event this
/// message reports happened; `None` where it answers something else.
///
/// The code set names thirty-six stamps and twenty-two of them answer
/// something else. Nineteen are the trade's afterlife rather than the trade:
/// submission to clearing, public and non-public reporting and their updates,
/// confirmation, clearing, allocation, submission to a repository,
/// continuation events, valuation, an identifier's assignment, affirmation
/// and a bare update time all happen after the event and say nothing about
/// when it happened. Three are about something other than this message: a
/// previous time priority and a previous identifier describe the state it
/// replaced, and a reference time for the BBO describes the market it was
/// measured against. A code no set names is one of these until someone says
/// otherwise - an unranked stamp is silence, never a clock - so only the two
/// lists above date a message.
fn trdregtimestamp_rank(kind: i64) -> Option<u8> {
    if EVENT_TRDREG_TYPES.contains(&kind) {
        return Some(TRANSACT_RANK + 1);
    }
    if HOP_TRDREG_TYPES.contains(&kind) {
        return Some(TRANSACT_RANK + 2);
    }
    None
}

/// One typed clock as the nanoseconds since the epoch it counts; nothing
/// where the value is not an instant, which is what a clock the dictionary
/// does not type as one, or one that would not read, leaves in the row.
fn instant_of(value: &Scalar) -> Option<i64> {
    let held @ Scalar::DateTime64(_) = value else {
        return None;
    };
    held.temporal_count_at(crate::TimeUnit::Nanosecond)
}

/// One code of a set as the number it is, however the dictionary typed the
/// tag: the integer a code set answers with, else the digits a dictionary
/// that left the tag as text carries.
fn code_of(value: &Scalar) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

/// The position of the child carrying one tag among `fields`, by the
/// dictionary's own reading of each child. A group occurrence is positional,
/// the names living on the field and never in the value, so this is found once
/// on the occurrence's field and every occurrence is read by it.
fn position_of_tag(registry: &FixRegistry, fields: &[Field], tag: i32) -> Option<usize> {
    fields
        .iter()
        .position(|child| super::schema::tag_and_counter(registry, child).0 == Some(tag))
}

fn group_positions(field: &Field, registry: &FixRegistry) -> Vec<(i32, usize)> {
    let mut held: Vec<_> = field
        .fields()
        .iter()
        .enumerate()
        .filter_map(|(index, child)| {
            Some((super::schema::tag_and_counter(registry, child).1?, index))
        })
        .collect();
    held.sort_unstable();
    held
}

impl FixMsg {
    /// The deterministic hash of this message's schema and row.
    /// Uses one allocation for the shared XXH3 state, independent of message size.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_of(self)
    }

    /// Builds a message against the process-wide registry.
    ///
    /// # Errors
    ///
    /// Returns the default registry's load failure, or the refusal
    /// [`Self::with_registry`] raises.
    pub fn new(field: Field, value: Scalar) -> Result<Self> {
        Self::with_registry(Arc::clone(FixRegistry::global()?), field, value)
    }

    /// Builds a message against an explicit registry.
    ///
    /// The value is validated through [`Field::validate_value`] and stored
    /// as [`Field::canonicalize_value`] rewrites it, so a record becomes the
    /// ordered sequence the root declares. A child stating a typed fact -
    /// a header tag, a crate column, the event's own tags - fills the
    /// holder that owns it and leaves the row.
    ///
    /// # Errors
    ///
    /// Returns an error when the root is not a Struct field, or when the
    /// value violates it, naming the path of the first value that does not
    /// fit.
    pub fn with_registry(registry: Arc<FixRegistry>, field: Field, value: Scalar) -> Result<Self> {
        let value = field.canonicalize_value(value)?;
        Self::assemble(
            registry,
            field,
            value,
            None,
            None,
            super::FixCodec::DEFAULT_OFFICIAL_TIME_DELAY_NS,
            true,
        )
    }

    /// Builds the message the one builder finished, checking nothing twice.
    ///
    /// Every value in the row went through the contract of the field it
    /// lands under, so the row is canonical by construction. The sending
    /// clock is what the message stated, else `fallback_sending_time` - its
    /// carrier's, else the codec's default - else now: initial intake
    /// settles clocks, and replay never reads now.
    /// `official_time_delay_ns` is how far from that clock an official
    /// transaction may stand and still date the message, which the codec
    /// states for the whole run. The identity is not settled here: the
    /// [enriching pass](super::enrich) that takes every built message
    /// restates and fills it first, and settles it once at its end, so
    /// nothing is digested that a later write of the same pass rewrites.
    pub(super) fn from_built(
        registry: Arc<FixRegistry>,
        built: super::build::Built,
        fallback_sending_time: Option<&Scalar>,
        source: Option<Uuid>,
        official_time_delay_ns: i64,
    ) -> Result<Self> {
        let super::build::Built {
            field,
            value,
            anomalies,
            ..
        } = built;
        let mut message = Self::assemble(
            registry,
            field,
            value,
            fallback_sending_time,
            source,
            official_time_delay_ns,
            false,
        )?;
        message.arrival_anomalies = anomalies.len();
        message.anomalies = anomalies;
        Ok(message)
    }

    /// Builds a message from the content reconstructed out of a semantic row.
    /// The rebuilt nested tree is canonicalized once against its newly built
    /// root. A row carrying its complete event identity keeps that recorded
    /// identity; a narrower row is settled from the facts it does carry.
    pub(super) fn from_rebuilt_row(
        registry: Arc<FixRegistry>,
        field: Field,
        value: Scalar,
        retains_identity: bool,
    ) -> Result<Self> {
        let value = field.canonicalize_value(value)?;
        let mut message = Self::assemble(
            registry,
            field,
            value,
            None,
            None,
            super::FixCodec::DEFAULT_OFFICIAL_TIME_DELAY_NS,
            false,
        )?;
        message.sync_session_event_identifier();
        if retains_identity {
            message.derive_market();
            message.rebuild_idmaps();
            message.event.fill_market();
            message.event.fill_operation();
        } else {
            message.settle();
        }
        Ok(message)
    }

    /// One message from a root and its canonical row: the typed facts are
    /// lifted out of the children that state them, the rest is the row,
    /// the clocks are settled and, where `settle` says so, the identity
    /// derived. `source` is the identity of the line the row was parsed out
    /// of, stated as the message's one source before it is settled; a row
    /// stating a `srcuuids` column of its own states those instead.
    /// `official_time_delay_ns` is how far from the sending clock an
    /// official transaction may stand and still date the message.
    fn assemble(
        registry: Arc<FixRegistry>,
        field: Field,
        value: Scalar,
        fallback_sending_time: Option<&Scalar>,
        source: Option<Uuid>,
        official_time_delay_ns: i64,
        settle: bool,
    ) -> Result<Self> {
        let plan = super::schema::column_plan_of(&field, &registry)?;
        let held = value.as_sequence().ok_or_else(|| {
            identity::refused(field.name(), "a canonical Struct row", value.kind())
        })?;
        let mut event = Box::new(OperationEventData::default());
        if let Some(source) = source {
            event.set_srcuuids(vec![source]);
        }
        let mut header = Box::new(FixHeader::unknown());
        let mut capture = Box::new(FixCapture::default());
        let mut lifted = Box::new(FixLifted::default());
        let mut text = None;
        let mut metadata = BTreeMap::new();
        let mut members = Vec::with_capacity(field.fields().len());
        let mut values = Vec::with_capacity(held.len());
        let mut kept = Vec::with_capacity(plan.len());
        let mut stated_sending = false;
        let mut stated_unix = false;
        let mut stated_creation = false;
        let mut row_stated = 0_u16;
        // A typed tag is lifted out of the row onto its holder, and a holder
        // keeps one fact per tag - so a row stating one tag twice is left
        // where it stands rather than collapsed into one slot. Two children
        // under `ClOrdID(11)`, a venue's own beside the client's, are two
        // facts and a reader addressing them by name must still find both.
        for ((child, column), value) in field.fields().iter().zip(plan.iter()).zip(held) {
            match column.tag {
                Some(tag) if identity::is_typed_tag(tag) && !column.shared => {
                    if value.is_null() {
                        continue;
                    }
                    if tag == identity::TEXT_TAG {
                        text = value.as_str().map(SmolStr::new);
                    } else if tag == super::METADATA_TAG_NAME.0 {
                        metadata = metadata_of(value);
                    } else {
                        identity::record(
                            &mut event,
                            &mut header,
                            &mut capture,
                            &mut lifted,
                            tag,
                            value,
                        );
                    }
                    stated_sending |= tag == 52;
                    stated_unix |= tag == super::CURRUNIX_TAG_NAME.0;
                    stated_creation |= tag == super::CREAUNIX_TAG_NAME.0;
                    if let Some(bit) = row_stated_bit(tag) {
                        row_stated |= bit;
                    }
                }
                // A key spelled under a namespace - `TECH.CLIENTID` - is a
                // bridge's own statement and goes to the metadata, under
                // the key as the bridge spelled it, folded.
                None if child.name().contains('.') => {
                    if let Some(held) = value.as_str().filter(|held| !held.is_empty()) {
                        metadata.insert(SmolStr::new(child.name()), SmolStr::new(held));
                    }
                }
                // The capture's own column, whoever built the root: the
                // object the line was read from, the instant it was
                // recorded. Read past, because a message states nothing
                // about the reading it arrived through - and never kept as
                // a child, which would make it content the wire re-emits.
                Some(tag) if identity::is_capture_tag(tag) => {}
                _ => {
                    members.push(child.clone());
                    values.push(value.clone());
                    kept.push(*column);
                }
            }
        }
        if !stated_sending {
            let sending = match fallback_sending_time.filter(|value| !value.is_null()) {
                Some(value) => value.clone(),
                None => identity::now()?,
            };
            identity::validate_value("sendingtime", &super::schema::CLOCK_DATATYPE, &sending)?;
            if let Some(unix) = sending.temporal_count_at(crate::TimeUnit::Nanosecond) {
                header.set_sendingtime(unix);
            }
        }
        if event.get_marketoperationid().is_none() {
            event.set_marketoperationid(Some(derived_marketoperationid(
                &registry,
                header.msgtype(),
            )));
        }
        // The members are the planned root's children less the lifted
        // ones, named once as that root named them.
        let field = Field::new_with_metadata(
            field.name(),
            DataType::from(StructType::from_unique_fields(members)),
            field.is_nullable(),
            field.as_metadata().clone(),
        );
        // The row keeps the planned root's children less the lifted ones,
        // so its plan is those children's, read once above.
        let plan: super::schema::ColumnPlan = Arc::from(kept);
        let tags = tag_positions(&plan);
        let groups = group_positions(&field, &registry);
        let mut message = Self {
            registry,
            event,
            header,
            capture,
            lifted,
            forced: false,
            row_stated,
            text,
            metadata,
            tags,
            named: OnceLock::new(),
            groups,
            field,
            value: Scalar::from_sequence(values),
            entries: OnceLock::new(),
            carried: Vec::new(),
            derived: SecurityIds::default(),
            anomalies: Vec::new(),
            arrival_anomalies: 0,
        };
        // The instant the message happened: what it states, else when the
        // transaction it reports happened, else when it was sent - the one
        // clock every message carries. A parse structures what a line said
        // and dates it against the sending clock, taking the official
        // transaction over it only where the two stand within
        // `official_time_delay_ns` of each other and are therefore the one
        // event said twice; what a resend's `OrigSendingTime(122)` says, and
        // what a `TransactTime(60)` further off than that says, is the
        // lifecycle's to read off the structured message.
        if !stated_unix {
            message
                .event
                .set_currunix(message.official_unix(official_time_delay_ns));
        }
        if !stated_creation {
            message.event.set_creaunix(Some(message.get_currunix()));
        }
        if settle {
            message.settle();
        }
        Ok(message)
    }

    /// The instant this message happened: the best official clock standing
    /// within `delay` of the sending clock, else the sending clock itself.
    ///
    /// The sending clock is the reference because every message carries one
    /// and no message carries two. An official clock is the more exact
    /// saying of when the event happened, and the distance between the two
    /// is the only evidence a parse has that they are saying the same thing:
    /// inside the delay they are one event and the official clock wins,
    /// outside it they are two and the parse keeps the clock it can trust.
    /// Rank decides between several that qualify - what the message says its
    /// transaction was before what a regulatory stamp says a hop was - and
    /// the nearer of two equal ranks decides after that, the earlier
    /// instant closing the last tie so that one row reads one way.
    fn official_unix(&self, delay: i64) -> i64 {
        let sending = self.header.sendingtime();
        self.official_clocks()
            .filter(|(_, unix)| within(*unix, sending, delay))
            .min_by_key(|(rank, unix)| (*rank, unix.abs_diff(sending), *unix))
            .map_or(sending, |(_, unix)| unix)
    }

    /// Every clock this message states about when its own event happened,
    /// each under the rank that says how well it answers that question.
    ///
    /// `TransactTime(60)` is the message's own statement and outranks
    /// everything; the `TrdRegTimestamps(768)` group is the venue's, and
    /// [`trdregtimestamp_rank`] reads each occurrence's
    /// `TrdRegTimestampType(770)` to say which of its stamps are about this
    /// event at all. Nothing here is ordered or bounded: the caller bounds
    /// them by the delay and picks one.
    fn official_clocks(&self) -> impl Iterator<Item = (u8, i64)> + use<'_> {
        self.transact_unix()
            .map(|unix| (TRANSACT_RANK, unix))
            .into_iter()
            .chain(self.trdregtimestamps())
    }

    /// The `TransactTime(60)` this message states, as an instant.
    ///
    /// By the index alone: a lookup that falls through to the name table is a
    /// read a dating has no use for. A transaction stating a day and no clock
    /// dates nothing - `60=20260814`, which the parse restates as that day's
    /// midnight - because what it says is the day, and midnight to the
    /// nanosecond is that statement and no other a venue makes.
    fn transact_unix(&self) -> Option<i64> {
        instant_of(&self.indexed_by_tag(60)?).filter(|unix| unix.rem_euclid(NANOS_PER_DAY) != 0)
    }

    /// Every `TrdRegTimestamp(769)` this message states that is about the
    /// event the message reports, under its rank.
    ///
    /// The group is read as a group and only as a group: `TrdRegTimestamp`
    /// says nothing on its own - the same tag carries the execution's
    /// instant, a desk's receipt and the moment a report reached a
    /// repository - and what tells them apart is the
    /// `TrdRegTimestampType(770)` standing beside it in the same
    /// occurrence. A dictionary that declares the group pairs them; one that
    /// does not leaves two flat children whose pairing is a guess, and a
    /// guess about which regulatory clock this is would date the message by
    /// a stamp that belongs to a different question.
    ///
    /// A group occurrence is positional - the names live on the field and
    /// never in the value - so the two members are found once on the
    /// occurrence's field and every occurrence is read by those positions.
    fn trdregtimestamps(&self) -> impl Iterator<Item = (u8, i64)> + use<'_> {
        self.trdregtimestamp_members()
            .into_iter()
            .flat_map(|(occurrences, stamp, kind)| {
                occurrences.iter().filter_map(move |occurrence| {
                    let held = occurrence.as_sequence()?;
                    let rank = trdregtimestamp_rank(code_of(held.get(kind)?)?)?;
                    Some((rank, instant_of(held.get(stamp)?)?))
                })
            })
    }

    /// The `TrdRegTimestamps(768)` occurrences beside the positions its
    /// `TrdRegTimestamp(769)` and `TrdRegTimestampType(770)` members hold in
    /// each of them; nothing where the dictionary declares no such group, or
    /// where the group it declares does not carry both members.
    fn trdregtimestamp_members(&self) -> Option<(&Serie, usize, usize)> {
        let at = self.index_of_group(768)?;
        let sequence = (self.field.fields().get(at)?.dtype()).as_serie_type()?;
        let members = sequence.item().fields();
        let stamp = position_of_tag(&self.registry, members, 769)?;
        let kind = position_of_tag(&self.registry, members, 770)?;
        let occurrences = self.value.as_sequence()?.get(at)?.as_serie()?;
        Some((occurrences, stamp, kind))
    }

    /// Replaces the row with another statement of the same content, as a
    /// restatement leaves it: the typed facts a restated child states fill
    /// their holders and the indexes are reread. The identity is not
    /// settled: the pass that restates settles once, after everything it
    /// writes.
    pub(super) fn replace_content(&mut self, field: Field, values: Vec<Scalar>) -> Result<()> {
        let plan = super::schema::column_plan_of(&field, &self.registry)?;
        let mut content = Vec::with_capacity(field.fields().len());
        let mut kept = Vec::with_capacity(values.len());
        for ((child, column), value) in field.fields().iter().zip(plan.iter()).zip(values) {
            let is_content = match column.tag {
                Some(tag) if identity::is_typed_tag(tag) => {
                    if !value.is_null() {
                        self.record(tag, &value);
                    }
                    false
                }
                // The capture's own column, which no restatement of the
                // content can make a fact of the message: read past, as
                // every other door reads it past.
                Some(tag) if identity::is_capture_tag(tag) => false,
                None if child.name().contains('.') => {
                    if let Some(held) = value.as_str().filter(|held| !held.is_empty()) {
                        self.metadata
                            .insert(SmolStr::new(child.name()), SmolStr::new(held));
                    }
                    false
                }
                _ => true,
            };
            content.push(is_content);
            if is_content {
                kept.push(value);
            }
        }
        // Where every child is content, the root handed in is the row's
        // own and its plan is the one just read.
        let plan = if kept.len() == field.fields().len() {
            self.field = field;
            plan
        } else {
            let members = field
                .fields()
                .iter()
                .zip(&content)
                .filter(|(_, is_content)| **is_content)
                .map(|(child, _)| child.clone())
                .collect();
            self.field = Field::new_with_metadata(
                field.name(),
                DataType::from(StructType::from_unique_fields(members)),
                field.is_nullable(),
                field.as_metadata().clone(),
            );
            super::schema::column_plan_of(&self.field, &self.registry)?
        };
        self.value = Scalar::from_sequence(kept);
        self.tags = tag_positions(&plan);
        self.groups = group_positions(&self.field, &self.registry);
        self.named = OnceLock::new();
        self.entries = OnceLock::new();
        Ok(())
    }

    /// Records one typed fact on the holder that owns it; a null clears it.
    /// A row-owned fact recorded is the row's word from then on, and a null
    /// recorded hands it back to derivation.
    fn record(&mut self, tag: i32, value: &Scalar) -> bool {
        if tag == identity::TEXT_TAG {
            self.text = value.as_str().map(SmolStr::new);
            return true;
        }
        if tag == super::METADATA_TAG_NAME.0 {
            self.metadata = metadata_of(value);
            return true;
        }
        let recorded = identity::record(
            &mut self.event,
            &mut self.header,
            &mut self.capture,
            &mut self.lifted,
            tag,
            value,
        );
        if recorded {
            if let Some(bit) = row_stated_bit(tag) {
                if value.is_null() {
                    self.row_stated &= !bit;
                } else {
                    self.row_stated |= bit;
                }
            }
        }
        recorded
    }

    /// What the message states under one typed tag, as the tag's own field
    /// types it.
    ///
    /// A holder keeps a number at the one width this crate keeps a number
    /// at, and a dictionary may type the tag's column narrower, so the
    /// answer is narrowed to the column that names it: one tag answers one
    /// type whether it is read here, off the row, or out of an Arrow
    /// column.
    fn typed_fact(&self, tag: i32) -> Option<Scalar> {
        if tag == identity::TEXT_TAG {
            return self.text.as_deref().map(Scalar::from);
        }
        if tag == super::METADATA_TAG_NAME.0 {
            if self.metadata.is_empty() {
                return None;
            }
            return Scalar::from_mapping(
                self.metadata
                    .iter()
                    .map(|(key, value)| (Scalar::from(key.as_str()), Scalar::from(value.as_str()))),
            )
            .ok();
        }
        let fact = Typed {
            event: &self.event,
            header: &self.header,
            capture: &self.capture,
            lifted: &self.lifted,
        }
        .fact(tag)?;
        Some(match self.registry.get_field_by_tag(tag) {
            Some(field) => super::schema::narrowed(field, fact),
            None => fact,
        })
    }

    /// What the message states that its reading could not take as it
    /// stands, in arrival order: the parse's refusals - a value that would
    /// not type, a counter disagreeing with its group - then what the last
    /// settle dropped. Read off the message beside the row: never a column,
    /// never part of the code it digests to. Two statements of one message
    /// merge them as a union, the reference's first.
    #[must_use]
    pub fn anomalies(&self) -> &[super::FixAnomaly] {
        &self.anomalies
    }

    /// Takes the parse's anomalies of another statement of this message
    /// into this one's, each once, so a merge loses no refusal either side
    /// recorded; what a settle drops is this message's own to record again.
    fn fold_anomalies(&mut self, other: &Self) {
        for anomaly in &other.anomalies[..other.arrival_anomalies] {
            if !self.anomalies[..self.arrival_anomalies].contains(anomaly) {
                self.anomalies
                    .insert(self.arrival_anomalies, anomaly.clone());
                self.arrival_anomalies += 1;
            }
        }
    }

    /// Settles the identity from what the message states: the cross code
    /// where it names none yet, the cross codes in step with it, the code
    /// the content digests to, and the identity the instant and that code
    /// derive.
    ///
    /// The cross code names the chain and defaults to the first nonempty FIX
    /// identifier in [`identity::CROSS_TAGS`], `OrderID(37)` first. The
    /// message type, capture session/context and message sequence name where
    /// a bridge observed the message instead: when all four are present,
    /// `sync_session_event_identifier` states their joined values as the
    /// capture's `msgsesseventid` without making it content.
    pub(super) fn settle(&mut self) {
        self.anomalies.truncate(self.arrival_anomalies);
        self.sync_session_event_identifier();
        if self.event.get_crosscode().is_empty() {
            let code = identity::CROSS_TAGS.iter().find_map(|tag| {
                self.get_by_tag(*tag)
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .filter(|code| !code.is_empty())
            });
            if let Some(code) = code {
                self.event.set_crosscode(code);
            }
        }
        // What the message implies about its market, read off the FIX
        // fields it stated, before the code it answers to covers either.
        self.derive_market();
        self.rebuild_idmaps();
        self.event.fill_market();
        self.event.fill_operation();
        self.event.sync_cross();
        let currhashcode = self.currhashcode();
        self.event.finalized(currhashcode);
    }

    /// Derives the complete FIX session event onto the capture: the four
    /// values joined by `:`, exactly as stated, and nothing where one is
    /// missing - a stale key never outlives the parts it was joined from.
    /// `:` is how a bridge's own row header brackets a session, its context
    /// and its sequence, so the key reads as the header does; values holding
    /// a `:` of their own can join to one key from two splits, and a bridge
    /// names none of its sessions or contexts that way.
    fn sync_session_event_identifier(&mut self) {
        let identity = match (
            self.header.msgtype(),
            self.capture.msgsessionid(),
            self.capture.msgctxid(),
            self.header.msgseqnum(),
        ) {
            (msgtype, Some(session), Some(context), Some(sequence))
                if !msgtype.is_empty() && !session.is_empty() && !context.is_empty() =>
            {
                Some((msgtype, session, context, sequence))
            }
            _ => None,
        };
        let joined = identity.map(|(msgtype, session, context, sequence)| {
            if self
                .capture
                .msgsesseventid()
                .is_some_and(|held| is_session_event(held, msgtype, session, context, sequence))
            {
                return None;
            }
            let mut joined =
                String::with_capacity(msgtype.len() + session.len() + context.len() + 3 + 20);
            write!(joined, "{msgtype}:{session}:{context}:{sequence}")
                .expect("writing into a String cannot fail");
            Some(SmolStr::from(joined))
        });
        match joined {
            // Already the key the four parts join to.
            Some(None) => {}
            Some(Some(joined)) => self.capture.set_msgsesseventid(Some(joined)),
            None => self.capture.set_msgsesseventid(None),
        }
    }

    /// The complete prebuilt session-event delivery identity, where present.
    pub(super) fn session_event_identifier(&self) -> Option<&str> {
        self.capture.msgsesseventid()
    }

    /// Whether two observations state one complete session event.
    pub(super) fn is_same_session_event(&self, other: &Self) -> bool {
        self.session_event_identifier()
            .is_some_and(|identity| other.session_event_identifier() == Some(identity))
    }

    /// Whether this is the graph's derived expiry rather than another raw FIX
    /// observation. It deliberately keeps the source session-event identity,
    /// but remains a later lifecycle event and must follow instead of merge.
    fn is_synthetic_expiry(&self) -> bool {
        self.get_state().as_str() == "95EXPIRED"
            && self.get_recdunix().is_none()
            && self.get_exprtime() == Some(self.get_currunix())
    }

    /// Whether `with_previous` must treat the two values as observations of
    /// one FIX event rather than successive lifecycle events.
    pub(super) fn should_merge_session_event(&self, other: &Self) -> bool {
        self.is_same_session_event(other) && !self.is_synthetic_expiry()
    }

    /// Fully merge another observation of this session event, retaining the
    /// latest recording as the reference row and the earliest precise facts.
    pub(super) fn merge_session_event(self, other: &Self) -> Result<Self> {
        debug_assert!(self.is_same_session_event(other));
        let other_leads = crate::graph::element::right_is_reference(
            self.get_recdunix(),
            self.get_currunix(),
            other.get_recdunix(),
            other.get_currunix(),
        );
        if other_leads {
            return other.clone().fold_session_event(&self);
        }
        self.fold_session_event(other)
    }

    /// Fully merge another observation of this session event into this one,
    /// which stays the reference whatever the two recording clocks say: for
    /// a caller that already chose it, as a run of observations sorted
    /// latest recording first does, where re-deciding at every pair would
    /// let a later one lead against the earliest recording a fold keeps.
    pub(super) fn fold_session_event(mut self, other: &Self) -> Result<Self> {
        debug_assert!(self.is_same_session_event(other));
        super::latest::merge_content(&mut self, other)?;
        crate::graph::market::merge_operation_event_into_reference(&mut self, other);
        self.fold_anomalies(other);
        Ok(self)
    }

    /// Whether an explicitly stated `ExecType(150)` reports an execution.
    /// Its raw FIX code is inspected, never the lifecycle state derived from
    /// `OrdStatus`: trade corrections, cancels and clearing transitions can
    /// carry a filled order state without being executions themselves.
    fn explicit_execution_type(&self) -> Option<bool> {
        self.get_by_tag(150)
            .map(|held| matches!(held.as_str(), Some("F" | "1" | "2")))
    }

    /// Whether this message reports an execution rather than merely carrying
    /// execution-shaped fields. A TradeCaptureReport may omit `ExecType(150)`;
    /// its initial `TradeReportTransType(487)` is then the execution signal.
    /// Requests and acknowledgements never become executions, and a cancel,
    /// replace, release or reverse report is a lifecycle action rather than a
    /// new precise execution.
    pub(super) fn reports_execution(&self) -> bool {
        let msgtype = self.header.msgtype();
        if msgtype == "AE" {
            let new_report = self.get_by_tag(487).is_none_or(|held| {
                held.is_null()
                    || held.as_i64() == Some(0)
                    || held.as_str().is_some_and(|value| {
                        matches!(value, "0" | "N") || crate::folds_equal(value, "New")
                    })
            });
            return new_report && self.explicit_execution_type() != Some(false);
        }
        if matches!(msgtype, "AD" | "AQ" | "AR") {
            return false;
        }
        self.explicit_execution_type()
            .unwrap_or_else(|| self.event.is_execution())
    }

    /// One FIX or proprietary timestamp read as the event clock the graph
    /// keeps. Dictionary timestamps are already typed; an unresolved bridge
    /// key is read once through the crate execution field's FIX spelling.
    fn execution_instant(&self, value: Option<Scalar>) -> Option<i64> {
        let value = value?;
        value
            .temporal_count_at(crate::TimeUnit::Nanosecond)
            .or_else(|| {
                let text = value.as_str()?;
                let field = self.registry.get_field_by_tag(super::EXECUNIX_TAG_NAME.0)?;
                super::build::typed_spelling(&self.registry, field, text)
                    .temporal_count_at(crate::TimeUnit::Nanosecond)
            })
    }

    /// The first regulatory timestamp explicitly classified as execution
    /// time. A timestamp of any other type says nothing about execution.
    fn trdreg_execution_instant(&self) -> Option<i64> {
        let at = self.index_of_group(768)?;
        let column = self.field.fields().get(at)?;
        let sequence = (column.dtype()).as_serie_type()?;
        let item = sequence.item();
        let (timestamp, kind) = (
            item.index_of("trdregtimestamp")?,
            item.index_of("trdregtimestamptype")?,
        );
        self.value
            .as_sequence()?
            .get(at)?
            .as_serie()?
            .iter()
            .find_map(|occurrence| {
                let held = occurrence.as_sequence()?;
                let kind = held.get(kind)?;
                let execution = kind.as_i64() == Some(1)
                    || kind.as_str().is_some_and(|kind| {
                        kind == "1" || crate::folds_equal(kind, "ExecutionTime")
                    });
                execution
                    .then(|| self.execution_instant(held.get(timestamp).cloned()))
                    .flatten()
            })
    }

    /// What the message implies about its market, read off the FIX fields
    /// it states and filled onto the event.
    ///
    /// Every market fact is FIX's own, and a message states the ones it
    /// states: `Price(44)`, `OrderQty(38)` or `Quantity(53)`, the fill and
    /// the progress, the side and the currency, the ISIN, Bloomberg and FIGI
    /// identifiers under their sources, the market, the unit, the state and
    /// the two quote lanes. CUSIP and SEDOL stay in `SecurityID` or
    /// `secaltids` unless a semantic row or setter states their normalized
    /// columns. This reads each lifted fact and hands it to the trait, which
    /// is where the ladder that decides what the message is *about* lives -
    /// [`Market::fill_market`], called straight after through the
    /// FIX-specific guard below.
    ///
    /// The execution clock is the one the message's own fields state, and a
    /// raw observation reporting an execution - [`Event::is_execution`] as
    /// this message reads it - that states none executed when it happened,
    /// so its `execunix` is its own `currunix` from intake on rather than
    /// from the first walk. A message following another that states none
    /// keeps the execution its chain reached: that is the walk's to state,
    /// and its state may be one it inherited rather than one it reported.
    ///
    /// What a lifecycle walk forced - the state it reached, what a price
    /// moved from, the instrument the chain is about - survives being
    /// settled again, and so do the state and the expiry a row stated: a
    /// chained message read back keeps what the walk folded forward rather
    /// than what its own fields say. And nothing filled here reaches the
    /// wire, the arrival record or the code the message digests to: those
    /// read what the message *stated*, and a derived fact is not a
    /// statement.
    fn derive_market(&mut self) {
        if self.forced {
            return;
        }
        let row_stated = self.row_stated;
        let marketoperationid = (row_stated & ROW_STATED_MARKET_OPERATION == 0)
            .then(|| derived_marketoperationid(&self.registry, self.header.msgtype()));
        let text = |value: Option<Scalar>| {
            value
                .and_then(|held| held.as_str().map(str::trim).map(str::to_owned))
                .filter(|held| !held.is_empty())
        };
        let by_tag = |tag: i32| self.get_by_tag(tag).filter(|held| !held.is_null());
        let number = |tag: i32| by_tag(tag).as_ref().and_then(Decimal18::from_scalar);
        let word = |tag: i32| text(by_tag(tag));

        // The numbers, each from its own lifted slot.
        let (price, orderqty, quantity) = (
            self.lifted.price(),
            self.lifted.orderqty(),
            self.lifted.quantity(),
        );
        let (lastpx, lastqty) = (self.lifted.lastpx(), self.lifted.lastqty());
        let (avgpx, cumqty, leavesqty) = (
            self.lifted.avgpx(),
            self.lifted.cumqty(),
            self.lifted.leavesqty(),
        );
        // The side, the currency and the unit the quantity is counted in.
        let side = word(54).and_then(|held| Side::read(&held).ok());
        let currency = word(15)
            .or_else(|| word(120))
            .and_then(|held| Ccy::new(&held).ok());
        let unit = word(996);
        let tif = word(identity::TIMEINFORCE_TAG);
        // The instrument: what it is classified as, what it is called, and
        // its identifiers under the sources that name them.
        // The detailed classification the chain reaches, or none: a coarse
        // stated code is not a classification the market keeps.
        let cficode = self
            .classification()
            .and_then(|held| CfiCode::new(&held).ok());
        debug_assert!(
            cficode
                .as_ref()
                .is_none_or(|held| CfiCode::is_detailed(held.as_str())),
            "the classification chain answers a detailed code or none"
        );
        let symbolticker = word(55).filter(|held| held != "[N/A]" && held != "[N/A");
        // The security identifiers FIX states, under the source that names
        // each, with a crated column's row-stated entry kept over them.
        let (mut securityids, mut dropped) = self.stated_securityids();
        // A crated column's row-stated entry ranks after the wire's own:
        // it fills a key the wire leaves absent, and a different code under
        // a filled key is dropped with an anomaly.
        for (bit, key, name) in [
            (ROW_STATED_ISIN, "ISIN", super::ISINCODE_TAG_NAME.1),
            (
                ROW_STATED_BLOOMBERG,
                "BLOOMBERG",
                super::BLOOMBERGCODE_TAG_NAME.1,
            ),
            (ROW_STATED_FIGI, "FIGI", super::FIGICODE_TAG_NAME.1),
        ] {
            if row_stated & bit != 0 {
                if let Some(id) = self.event.get_securityids().get_id(key) {
                    match securityids.get(key) {
                        Some(held) if held != id.code() => dropped.push(super::FixAnomaly::new(
                            name,
                            format!(
                                "states {key}:{} where {key}:{held} is already stated",
                                id.code()
                            ),
                        )),
                        Some(_) => {}
                        None => {
                            securityids.insert(id.clone());
                        }
                    }
                }
            }
        }
        // The market it is listed on, routed to, or last traded on.
        let miccode = word(207)
            .or_else(|| word(100))
            .or_else(|| word(30))
            .and_then(|held| MicCode::new(&held).ok());
        // Whether it could trade, from whichever status says so, in the
        // codes FIX's own enumerations state. A status that is about
        // something else - a code neither list names - says nothing either
        // way, so the next status answers instead.
        let status = |tag: i32, open: &[i64], shut: &[i64]| {
            let held = by_tag(tag)?.as_i64()?;
            if open.contains(&held) {
                Some(true)
            } else if shut.contains(&held) {
                Some(false)
            } else {
                None
            }
        };
        let tradable = status(326, &[3, 17], &[1, 2, 4, 18, 19, 21])
            .or_else(|| status(340, &[2], &[1, 3, 4, 5, 7]))
            .or_else(|| match word(965)?.as_str() {
                "1" | "3" => Some(true),
                "2" | "4" | "5" | "6" | "9" | "11" => Some(false),
                _ => None,
            });
        // The state it reached, and when it stops being good.
        let state = word(39)
            .or_else(|| word(150))
            .and_then(|held| State::read(&held).ok());
        let exprtime = [126, 62, 432, 541].into_iter().find_map(|tag| {
            by_tag(tag).and_then(|held| held.temporal_count_at(crate::TimeUnit::Nanosecond))
        });
        // Execution time is not the message time. It is stated directly by
        // the crate column, then by FIX's execution-specific timestamp,
        // regulatory execution member, a bridge's event timestamp, or the
        // transaction time of an actual trade. Corrections and cancels do
        // not make their transaction clock an execution clock.
        let execunix = self
            .execution_instant(by_tag(2749))
            .or_else(|| self.trdreg_execution_instant())
            .or_else(|| self.execution_instant(self.get_by_name("eventtimestamp")))
            .or_else(|| {
                self.reports_execution()
                    .then(|| self.execution_instant(by_tag(60)))
                    .flatten()
            });
        // What a price moved from, and the two lanes a quote states.
        let prevpx = number(140);
        let (bidpx, bidqty) = (number(132), number(134));
        let (askpx, askqty) = (number(133), number(135));
        // The FX parts of the prices, each from its own lifted slot: the
        // last price's, and each lane's.
        let (spotrate, forwardpoints) =
            (self.lifted.lastspotrate(), self.lifted.lastforwardpoints());
        let (bidspotrate, bidforwardpoints) =
            (self.lifted.bidspotrate(), self.lifted.bidforwardpoints());
        let (askspotrate, askforwardpoints) = (
            self.lifted.offerspotrate(),
            self.lifted.offerforwardpoints(),
        );

        let event = &mut *self.event;
        // Assigned rather than filled: a write can change what the FIX
        // fields say, and a fact that no longer derives must stop being
        // answered. What a walk forced is kept by the early return above.
        if let Some(marketoperationid) = marketoperationid {
            event.set_marketoperationid(Some(marketoperationid));
        }
        event.set_price(price);
        event.set_quantity(orderqty.or(quantity));
        event.set_lastpx(lastpx);
        event.set_lastqty(lastqty);
        event.set_avgpx(avgpx);
        event.set_cumqty(cumqty);
        event.set_leavesqty(leavesqty);
        event.set_prevpx(prevpx);
        event.set_spotrate(spotrate);
        event.set_forwardpoints(forwardpoints);
        // FIX states a lane's price, quantity and FX parts; its currency and
        // unit are read off the side by the fill, so the lanes are rebuilt
        // whole here with the rest of what derives.
        event.set_bid(
            Lane {
                price: bidpx,
                quantity: bidqty,
                spotrate: bidspotrate,
                forwardpoints: bidforwardpoints,
                ..Lane::default()
            }
            .stated(),
        );
        event.set_ask(
            Lane {
                price: askpx,
                quantity: askqty,
                spotrate: askspotrate,
                forwardpoints: askforwardpoints,
                ..Lane::default()
            }
            .stated(),
        );
        if row_stated & ROW_STATED_EXPIRY == 0 {
            event.set_exprtime(exprtime);
        }
        event.set_tradable(tradable);
        event.set_side(side.unwrap_or(Side::Unknown));
        event.set_currency(currency.unwrap_or_else(Ccy::none));
        event.set_unit(
            unit.and_then(|held| Unit::new(&held).ok())
                .unwrap_or_else(Unit::none),
        );
        event.set_tif(tif.and_then(|held| TimeInForce::from_spelling(&held)));
        event.set_ticker(symbolticker.map(SmolStr::from));
        event.set_cficode(cficode);
        for id in self.derived.iter() {
            securityids.insert(id.clone());
        }
        let _ = event.set_securityids(securityids);
        if row_stated & ROW_STATED_MIC == 0 {
            event.set_miccode(miccode);
        }
        // What the stated identifiers dropped, recorded once the readings
        // above no longer borrow the message.
        self.anomalies.extend(dropped);
        if row_stated & ROW_STATED_STATE == 0 {
            event.set_state(state.unwrap_or_else(State::unknown));
        }
        // A clock the fields state is the execution's. Where they state none,
        // a raw execution report executed when it happened, so intake dates
        // it rather than leaving that to a walk; a message a walk placed
        // keeps the latest execution its chain reached, which is the walk's
        // to state - a walk that carried the same instant states nothing new
        // - and its state may be one it inherited, which read as its own
        // report would date an execution it never made.
        if row_stated & ROW_STATED_EXECUTION == 0 {
            let execunix = execunix.or_else(|| {
                if self.event.get_prevuuid().is_some() {
                    self.event.get_execunix()
                } else {
                    self.reports_execution().then(|| self.event.get_currunix())
                }
            });
            self.event.set_execunix(execunix);
        }
    }

    /// The instrument's identifier under one of `sources`, read as the code
    /// it claims to be: the primary `SecurityID(48)` where
    /// `SecurityIDSource(22)` names one of them, else the `SecurityAltID`
    /// whose own source does.
    ///
    /// Each candidate is validated on its own, so a primary the standard's
    /// check digit does not close falls through to the alternate rather
    /// than answering for it - which is what a number one digit off is: not
    /// that security.
    /// The security identifiers the message states: the primary
    /// `SecurityID(48)` under its `SecurityIDSource(22)`, then each
    /// `secaltids` occurrence, each source read through [`SecType::read`],
    /// each code validated, the first stated code under a key kept.
    fn stated_securityids(&self) -> (SecurityIds, Vec<super::FixAnomaly>) {
        let mut ids = SecurityIds::default();
        let mut anomalies = Vec::new();
        // Fill only: the same code twice under one key is one entry, and a
        // later different code under a filled key is dropped with an
        // anomaly, staying on the wire as it arrived.
        let mut insert = |field: &str, key: SecType, code: &str| match SecurityId::new(key, code) {
            Ok(id) => {
                let key = id.sectype();
                match ids.get(key.as_str()) {
                    Some(held) if held != id.code() => anomalies.push(super::FixAnomaly::new(
                        field,
                        format!(
                            "states {key}:{} where {key}:{held} is already stated",
                            id.code()
                        ),
                    )),
                    Some(_) => {}
                    None => {
                        ids.insert(id);
                    }
                }
            }
            Err(error) => anomalies.push(super::FixAnomaly::new(field, error.to_string())),
        };
        let mut state = |field: &str, source: Option<SmolStr>, code: Option<SmolStr>| {
            if let (Some(source), Some(code)) = (source, code) {
                if let Ok(key) = SecType::read(&source) {
                    insert(field, key, &code);
                }
            }
        };
        state(
            "securityid",
            self.get_by_tag(22).as_ref().and_then(scalar_text),
            self.get_by_tag(48).as_ref().and_then(scalar_text),
        );
        for occurrence in self.group_members("secaltids", &["securityaltidsource", "securityaltid"])
        {
            state("secaltids", occurrence[0].clone(), occurrence[1].clone());
        }
        // A top-level field no dictionary names, whose name names an
        // identifier source - `#ISINCODE`, `cusip_code` - states an entry
        // after the wire's own: trimmed, validated, never a grouped member,
        // and left on the wire as it arrived. An empty or null-like value
        // states nothing.
        let mapped: std::collections::HashSet<usize> =
            self.tags.iter().map(|(_, at)| *at).collect();
        if let Some(cells) = self.value.as_sequence() {
            for (at, (child, cell)) in self.field.fields().iter().zip(cells).enumerate() {
                if mapped.contains(&at) || child.name().contains('.') {
                    continue;
                }
                let Some(key) = SecType::from_field_name(child.name()) else {
                    continue;
                };
                let Some(text) = scalar_text(cell)
                    .map(|held| held.trim().to_owned())
                    .filter(|held| !held.is_empty() && !is_null_like(held))
                else {
                    continue;
                };
                insert(child.name(), key, &text);
            }
        }
        (ids, anomalies)
    }

    /// The stated members of each occurrence of a root repeating group, in
    /// occurrence order; empty where the message carries no such group.
    fn group_members(&self, group: &str, members: &[&str]) -> Vec<Vec<Option<SmolStr>>> {
        let Some(at) = self.field.index_of(group) else {
            return Vec::new();
        };
        let Some(sequence) = self
            .field
            .fields()
            .get(at)
            .and_then(|column| column.dtype().as_serie_type())
        else {
            return Vec::new();
        };
        let item = sequence.item();
        let positions: Vec<Option<usize>> =
            members.iter().map(|name| item.index_of(name)).collect();
        let Some(rows) = self
            .value
            .as_sequence()
            .and_then(|values| values.get(at))
            .and_then(Scalar::as_serie)
        else {
            return Vec::new();
        };
        rows.iter()
            .filter_map(|occurrence| {
                let held = occurrence.as_sequence()?;
                Some(
                    positions
                        .iter()
                        .map(|position| position.and_then(|at| held.get(at)).and_then(scalar_text))
                        .collect(),
                )
            })
            .collect()
    }

    /// Rebuilds the account, user and alternate identifier maps from the
    /// fields that state them, at every settle.
    fn rebuild_idmaps(&mut self) {
        let mut accountids = IdMap::new();
        let mut userids = IdMap::new();
        let mut altids = IdMap::new();
        for (kind, key, tag) in IDMAP_SOURCES {
            let Some(value) = self.get_by_tag(tag).as_ref().and_then(scalar_text) else {
                continue;
            };
            let map = match kind {
                IdMapKind::Account => &mut accountids,
                IdMapKind::User => &mut userids,
                IdMapKind::Alt => &mut altids,
            };
            let _ = map.insert(key, &value);
        }
        for occurrence in self.group_members("parties", &["partyrole", "partyid"]) {
            let (Some(role), Some(id)) = (&occurrence[0], &occurrence[1]) else {
                continue;
            };
            let (map, key) = match role.as_str() {
                "24" => (&mut accountids, "CUSTOMERACCOUNT"),
                "36" => (&mut userids, "ENTERINGTRADER"),
                "12" => (&mut userids, "EXECUTINGTRADER"),
                _ => continue,
            };
            let _ = map.insert(key, id);
        }
        let _ = self.event.set_accountids(accountids);
        let _ = self.event.set_userids(userids);
        let _ = self.event.set_altids(altids);
    }

    /// Writes one security identifier where FIX states it: `SecurityID(48)`
    /// in place when `SecurityIDSource(22)` names the key, else the
    /// `secaltids` occurrence under the key's source code; `None` removes
    /// it. A key the message did not state before goes to `secaltids`,
    /// never to the primary.
    fn sync_security_id(&mut self, key: &SecType, value: Option<&str>) -> Result<()> {
        let source = key
            .fix_source()
            .map_or_else(|| key.as_str().to_owned(), |code| code.to_string());
        let primary_names = self
            .get_by_tag(22)
            .as_ref()
            .and_then(scalar_text)
            .and_then(|text| SecType::read(&text).ok())
            .is_some_and(|held| &held == key);
        if primary_names {
            match value {
                Some(value) => self.set_unsettled(48, Scalar::from(value))?,
                None => {
                    self.set_unsettled(48, Scalar::Null)?;
                    self.set_unsettled(22, Scalar::Null)?;
                }
            }
            super::latest::sync_group_occurrence(self, "secaltids", 456, &source, 455, None)
        } else {
            super::latest::sync_group_occurrence(self, "secaltids", 456, &source, 455, value)
        }
    }

    /// The source field one identifier-map key is stated by, or a located
    /// refusal where the dictionary names none.
    fn idmap_source(kind: IdMapKind, key: &str) -> Result<i32> {
        IDMAP_SOURCES
            .iter()
            .find(|(held, name, _)| *held == kind && name.eq_ignore_ascii_case(key))
            .map(|(_, _, tag)| *tag)
            .ok_or_else(|| Error::InvalidRecord {
                path: format_smolstr!("$.{}.{key}", kind.as_str()),
                reason: SmolStr::new_static("no FIX field states this key; it cannot be written"),
            })
    }

    /// The code the message digests to: the event's own facts - its parents, its state, its place in the chain and its
    /// predecessor - then every field the message states but the standard
    /// header and trailer - the text, the metadata, the FIX fields it
    /// lifted and every row child that holds a value, by name - and never
    /// the capture, because where a line was read from is a fact about the
    /// capture and not about the message.
    ///
    /// The standard header and trailer are the frame's, less the one tag in
    /// them that says what the message *is*: which session carried the
    /// message, its place in that session, when it was sent and how it was
    /// checked are the frame's, and `MsgType(35)` is not - an order and a
    /// report carrying the same tags are not one message, the exception
    /// [the wire digest](super::digest) states too. A capture logs one
    /// message at every hop it passes and each hop frames it in a session
    /// of its own, so a code over the rest of the frame would make one
    /// message as many messages as hops. Residual header or trailer entries
    /// use the wire digest's same envelope predicate, so storing an
    /// unlifted `OrigSendingTime(122)`, `PossResend(97)`, or `BodyLength(9)`
    /// cannot change this canonical code. The cross code names a chain and
    /// stays outside the content digest too. The derived `msgsesseventid`
    /// records the complete delivery provenance on the capture, and is
    /// excluded - under its former identifier name too - for the same reason
    /// as the frame and capture fields it combines.
    ///
    /// The market is not here either, except for its operation ID: other
    /// market facts derive from FIX fields the content already digests, while
    /// MsgCat is also a generic fact callers may state or mutate directly.
    /// Feeding that ID once makes the derived and serialized readings agree
    /// and makes a category mutation move the generic event identity.
    fn currhashcode(&self) -> u64 {
        let mut state = crate::xxhash::Xxh3::new();
        crate::graph::element::feed_event_facts(&mut state, &*self.event);
        let mut cells: Vec<(SmolStr, Scalar)> =
            Vec::with_capacity(self.field.fields().len() + identity::LIFTED_TAGS.len() + 2);
        if let Some(text) = self.text.as_deref() {
            cells.push((SmolStr::new_static("text"), Scalar::from(text)));
        }
        // The names are the dictionary's, read off it once per registry
        // rather than once per message.
        let names = self.registry.lifted_names();
        if !self.header.msgtype().is_empty() {
            cells.push((names.msgtype.clone(), Scalar::from(self.header.msgtype())));
        }
        for (key, value) in &self.metadata {
            cells.push((key.clone(), Scalar::from(value.as_str())));
        }
        // The fields the message lifted out of its row, under the names the
        // row's children are digested by.
        for (tag, name) in identity::LIFTED_TAGS.into_iter().zip(&names.lifted) {
            let Some(fact) = self.typed_fact(tag) else {
                continue;
            };
            cells.push((name.clone(), fact));
        }
        if let Some(marketoperationid) = self.event.get_marketoperationid() {
            cells.push((
                SmolStr::new_static("msgcat"),
                Scalar::from(marketoperationid),
            ));
        }
        cells.sort_by(|left, right| left.0.cmp(&right.0));
        xxhash::write_named_bytes(
            &mut state,
            cells.iter().map(|(name, value)| (name.as_str(), value)),
            0,
        );
        // Then the content, as the entries state it rather than as the row
        // stores it. Two readings of one message lay its children out
        // differently - a group one reading declares whole and another
        // states member by member is one group, and a child stating null
        // says nothing at all - so a code taken off the row's storage would
        // make a message read back out of a row a different message. The
        // entries are what the message says, and they are what this feeds.
        feed_entries(&mut state, self.entries());
        state.as_u64()
    }

    /// The message dated by the transaction it states, where the parse
    /// dated it by a stand-in: a message whose `SendingTime(52)` was
    /// supplied rather than stated - a carrier's, the codec's default, the
    /// intake's own clock - takes `TransactTime(60)` as its instant where it
    /// states one with a clock, and a resend's `OrigSendingTime(122)` as its
    /// creation where that is earlier, its stand-in sending clock moved to
    /// the same instant, and is enriched again around them - filled and
    /// settled - exactly as a parse dated there would have built it: the row
    /// the parse restated is the row a parse dated there restates, because a
    /// rule reads the row and never the clock. A stated sending clock stands:
    /// the parse dated the message by what it said, and the walk does not
    /// second-guess it. No delay bounds this one: a clock nobody stated is
    /// no reference to measure a distance from, which is why the parse's own
    /// [`FixCodec::official_time_delay_ms`](super::FixCodec::official_time_delay_ms)
    /// reading of the transaction ends where this one begins. What the
    /// [lifecycle](super::FixCodec::lifecycle) reads off the structured
    /// message before it walks, so a capture whose frames state no sending
    /// clock still orders, expires and folds by when its transactions
    /// happened rather than by when it was read.
    ///
    /// # Errors
    ///
    /// Returns the enriching pass's refusal, which a message this crate
    /// built never raises.
    pub fn dated_by_transaction(mut self) -> Result<Self> {
        if self.header.stated_sendingtime() {
            return Ok(self);
        }
        let Some(unix) = self.transact_unix() else {
            return Ok(self);
        };
        let current = self.get_currunix();
        let created = self.get_creaunix();
        self.event.set_currunix(unix);
        // The stand-in sending clock follows: it was never a fact of the
        // message, and a row read back states the instant as the clock.
        self.header.set_sendingtime(unix);
        if created.is_none_or(|held| held == current) {
            self.event.set_creaunix(Some(unix));
        }
        let origin = match self.indexed_by_tag(122) {
            Some(Scalar::DateTime64(origin)) => {
                Scalar::DateTime64(origin).temporal_count_at(crate::TimeUnit::Nanosecond)
            }
            _ => None,
        };
        if let Some(origin) = origin.filter(|origin| *origin < unix) {
            self.event.set_creaunix(Some(origin));
        }
        let registry = Arc::clone(&self.registry);
        super::enrich::enrich_restated(&registry, self)
    }

    /// The event this message is: every fact the three graph traits answer,
    /// held as fields.
    #[must_use]
    pub const fn event(&self) -> &OperationEventData {
        &self.event
    }

    /// The standard header and trailer, typed.
    #[must_use]
    pub const fn header(&self) -> &FixHeader {
        &self.header
    }

    /// The FIX fields the message lifted out of its row, exactly as it
    /// stated them: the prices, the quantities and the identifiers.
    ///
    /// What the message *implies* about its market is the
    /// [`Market`](crate::graph::Market) getters' to answer -
    /// `get_price` reads this ladder and the row - and a fact answered there
    /// but absent here is derived, which is why it reaches neither the wire
    /// nor the code the message digests to.
    #[must_use]
    pub const fn lifted(&self) -> &FixLifted {
        &self.lifted
    }

    /// `Text(58)`: the free text the message carries, where it carries one.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// What a bridge stated under its own namespaces - a `TECH.CLIENTID`, an
    /// `AMON.…` key - each under the key as the bridge spelled it, folded,
    /// in sorted order; empty where it stated none.
    #[must_use]
    pub const fn metadata(&self) -> &BTreeMap<SmolStr, SmolStr> {
        &self.metadata
    }

    /// What the line said about the capture it was written for, typed: the
    /// plugin a bridge logged it under, the message context and the session
    /// instance, all read off the line's own bytes, and the session event
    /// the last two join to with the message's type and sequence.
    ///
    /// Not where the line was read from and not when it was recorded: those
    /// are the reader's statements, and they are [the capture's own
    /// columns](Self::from_row) rather than facts of a message.
    #[must_use]
    pub const fn capture(&self) -> &FixCapture {
        &self.capture
    }

    /// The capture's own cells: what the row this message was read from
    /// said for itself, each under the column's name.
    ///
    /// Where the line was read from, its place in the object, the body it
    /// was cut from, when it was recorded, what a bound dropped: the columns
    /// a reader stated beside the payload, the carried ones and the one the
    /// crate tags, `sourceurl`. Provenance and never content: none of them
    /// is an entry, none reaches the wire or the code the message answers
    /// to - the same message read from a second copy of one day's log is
    /// the same message - and [`Self::into_row`] states each again at the
    /// column of its name, which is how a row read back through
    /// [`Self::from_row`] and written again keeps what it said for itself.
    /// A message parsed from bytes carries none; one parsed out of a row
    /// carries that row's, and one read back out of a row carries the
    /// row's.
    #[must_use]
    pub fn carried(&self) -> &[(SmolStr, Scalar)] {
        &self.carried
    }

    /// States the capture's own cells, replaced whole.
    pub fn set_carried(&mut self, cells: Vec<(SmolStr, Scalar)>) {
        self.carried = cells;
    }

    /// The cell carried under `name`, folded as a column is named, else
    /// null.
    pub(super) fn carried_cell(&self, name: &str) -> Scalar {
        self.carried
            .iter()
            .find(|(held, _)| crate::folds_equal(held, name))
            .map_or(Scalar::Null, |(_, value)| value.clone())
    }

    /// The row read as a tree: one entry per child it states, a group's
    /// occurrences and a component's members nested under the entry that
    /// heads them; nothing for a child stating null.
    ///
    /// Derived on the first ask and kept until a write, so a consumer
    /// walking the message twice pays once and a stream that never asks
    /// pays nothing. The typed facts are not entries: the header, the event
    /// and the capture are the holders' to answer.
    #[must_use]
    pub fn entries(&self) -> &[FixEntry] {
        self.entries.get_or_init(|| self.derive_entries().into())
    }

    /// The row read as a tree.
    ///
    /// Every child of the row is content, a key no dictionary explains
    /// included, because the row holds nothing else: the typed facts are
    /// their holders' to answer, and the capture's own columns - the body a
    /// line was read from, its place in the object, the object itself, the
    /// instant it was recorded - never reach a message at all, so there is
    /// nothing here to tell apart from what the line said.
    fn derive_entries(&self) -> Vec<FixEntry> {
        let Some(values) = self.value.as_sequence() else {
            return Vec::new();
        };
        entries_of(&self.registry, self.field.fields(), values)
    }

    /// The entries the wire carries around the row, in wire order: the
    /// standard header and then the FIX fields the message lifted in front
    /// of it, the standard trailer behind it. The row's own entries stand
    /// between the two exactly as [`Self::entries`] holds them, so a digest
    /// and a re-emission read them where they are rather than through a
    /// copy of the whole tree.
    ///
    /// The frame's own bands are the two the wire moves out of tag order,
    /// which is why the header leads and the trailer closes whatever the
    /// body's tags are. Between them stands only what the message *stated*:
    /// a fact the event derived - the price it is about, the state it
    /// reached, a lane it never quoted - is answered by the traits and
    /// emitted nowhere, because a re-emission says what was read.
    fn wire_bands(&self) -> (Vec<FixEntry>, usize) {
        let name_of = |tag: i32| {
            self.registry.get_field_by_tag(tag).map_or_else(
                || format_smolstr!("{tag}"),
                |field| SmolStr::new(field.name()),
            )
        };
        let emit = |entries: &mut Vec<FixEntry>, tag: i32| {
            if tag == 52 && !self.header.stated_sendingtime() {
                return;
            }
            let Some(fact) = self.typed_fact(tag) else {
                return;
            };
            let text = match self.registry.get_field_by_tag(tag) {
                Some(field) => wire_text_under(&self.registry, field, &fact),
                None => wire_text(&fact),
            };
            if let Some(text) = text {
                entries.push(FixEntry::new(tag, name_of(tag), Some(text)));
            }
        };
        // One vector holds both bands, the head first; the split is where
        // the row's own entries stand between them.
        let mut bands = Vec::with_capacity(
            identity::WIRE_HEADER_TAGS.len()
                + identity::LIFTED_TAGS.len()
                + identity::WIRE_TRAILER_TAGS.len(),
        );
        for tag in identity::WIRE_HEADER_TAGS
            .into_iter()
            .chain(identity::LIFTED_TAGS)
        {
            emit(&mut bands, tag);
        }
        let split = bands.len();
        for tag in identity::WIRE_TRAILER_TAGS {
            emit(&mut bands, tag);
        }
        (bands, split)
    }

    /// Writes one value into the message, typed by the field the key reaches.
    ///
    /// The key resolves as every lookup does - through the registry's one
    /// namespace. A key reaching a typed fact - a header tag, a crate
    /// column, one of the event's own tags - records it on the holder that
    /// owns it, and a `Null` clears it. Any other key lands in the row: a
    /// field the dictionary knows types the value through [`Field::scalar`]
    /// under the dictionary's own field, so a written child is
    /// indistinguishable from a stated one and carries the same `FIX:tag` a
    /// reader resolves it by; a `Null` is stored as a stated null. An
    /// existing child is replaced where it stands and an absent one is
    /// appended, so the positions every reader already holding the row
    /// addresses it by do not move. A name the dictionary does not know
    /// still reaches a child spelled that way - exactly, or under the fold
    /// every name resolves by - and keeps that child's field; a bare tag it
    /// does not know appends a nullable `utf8` child named by its decimal,
    /// which is what the builder does with an unknown tag. A name that
    /// reaches neither a field nor a child is refused, and the message is
    /// unchanged.
    ///
    /// A written value is then restated exactly as a read
    /// one is: writing `Rule80A(47)` writes the `OrderCapacity(528)` that
    /// replaced it beside it, and writing `ExecBroker(76)` makes the
    /// `Parties` occurrence it became - from the specification's own
    /// retirements, or from the rule a registry states on
    /// the field itself. What runs is the rules of the tags written, so a
    /// write of a tag no rule speaks for is the write and nothing more. The
    /// pass never overwrites a stated value and is idempotent, so writing
    /// one value twice writes its replacement once, and a caller who states
    /// the replacement keeps it. Every write settles the identity again.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use yggdryl::graph::Market;
    /// use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, StructType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut clordid = DataType::utf8().nullable_field("clordid");
    /// clordid.as_fix_mut().set_tag(11)?;
    /// let mut symbol = DataType::utf8().nullable_field("symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let registry = Arc::new(FixRegistry::from_fields([clordid, symbol])?);
    ///
    /// let root = DataType::from(StructType::from_fields([DataType::utf8().required_field("symbol")])?)
    ///     .required_field("D");
    /// let value = Scalar::from_struct([("symbol", Scalar::from("AAPL"))])?;
    /// let mut msg = FixMsg::with_registry(registry, root, value)?;
    ///
    /// // `ClOrdID(11)` is a fact the message lifts, so it lands on its
    /// // holder and the row grows by nothing.
    /// msg.set(11, Scalar::from("A1"))?;
    /// assert_eq!(msg.by_tag(11)?, Scalar::from("A1"));
    /// assert_eq!(msg.lifted().clordid(), Some("A1"));
    /// assert_eq!(msg.as_field().fields().len(), 1);
    ///
    /// // Replaced in place: the child keeps its position, the value changes.
    /// msg.set("Symbol", Scalar::from("MSFT"))?;
    /// assert_eq!(msg.as_field().fields()[0].name(), "symbol");
    /// assert_eq!(msg.by_tag(55)?, Scalar::from("MSFT"));
    ///
    /// // `Price(44)` and `OrderQty(38)` are lifted too, and what the
    /// // message is *about* is read off them.
    /// msg.set(44, Scalar::from("82.5"))?;
    /// msg.set(38, Scalar::from(100_i64))?;
    /// assert_eq!(msg.get_price().map(|px| px.to_string()).as_deref(), Some("82.5"));
    /// assert_eq!(msg.get_quantity().map(|qty| qty.to_string()).as_deref(), Some("100"));
    /// assert_eq!(msg.as_field().fields().len(), 1);
    ///
    /// // A tag no dictionary explains is kept under its decimal spelling.
    /// msg.set(9999, Scalar::from("custom"))?;
    /// assert_eq!(msg.by_tag(9999)?, Scalar::from("custom"));
    ///
    /// // A name nothing reaches is refused, and the row stands as it was.
    /// assert!(msg.set("nosuchfield", Scalar::from("x")).is_err());
    /// assert_eq!(msg.as_field().fields().len(), 2);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the key when it reaches no field and
    /// no child, the value contract's refusal when the value does not fit
    /// the field the key resolves to, or the schema grammar's refusal when
    /// the written child does not make a root. Any of them leaves the
    /// message unchanged.
    pub fn set<'key>(&mut self, key: impl Into<FixKey<'key>>, value: Scalar) -> Result<()> {
        self.set_many(std::iter::once((key, value)))
    }

    /// Writes several values into the message with one rebuild.
    ///
    /// Each key resolves and each value types exactly as [`Self::set`]
    /// resolves and types one, against the row as it stands before any of
    /// them lands; the row is then rebuilt once and the identity settled
    /// once. Two writes reaching one child, or two appending one field,
    /// land as the later one. Failure leaves the message unchanged,
    /// whichever write refused.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::set`] returns for the first write that refuses.
    pub fn set_many<'key, I, K>(&mut self, values: I) -> Result<()>
    where
        I: IntoIterator<Item = (K, Scalar)>,
        K: Into<FixKey<'key>>,
    {
        self.set_many_with(values, |_, _| Ok(()))
    }

    /// Check each resolved target before its one value canonicalization.
    /// Protocol stamps can require a native layout that coercion would erase.
    pub(super) fn set_many_with<'key, I, K>(
        &mut self,
        values: I,
        check: impl Fn(&FixKey<'key>, &Field) -> Result<()>,
    ) -> Result<()>
    where
        I: IntoIterator<Item = (K, Scalar)>,
        K: Into<FixKey<'key>>,
    {
        let mut writes: Vec<Write> = Vec::new();
        let mut typed: Vec<(i32, Scalar)> = Vec::new();
        for (key, value) in values {
            let key = key.into();
            match self.staged(&key, value, &check)? {
                Staged::Typed(tag, value) => typed.push((tag, value)),
                Staged::Row(write) => stage(&mut writes, write),
            }
        }
        self.land(typed, writes)
    }

    /// Writes every value the field it reaches can hold, with one rebuild
    /// and no settling, answering how many landed.
    ///
    /// The lenient twin of [`Self::set_many`], for a pass whose answers are
    /// best effort: a value the target refuses - an identifier whose check
    /// digit does not close, a spelling its code set does not read - is
    /// silence rather than a refusal, and every other value lands as
    /// `set_many` lands it. Nothing else is lenient: the rebuild's refusal,
    /// which no single value causes, is still returned and leaves the
    /// message unchanged. The identity is not settled: the pass that
    /// writes settles once, after everything it writes.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the written children do
    /// not make a root.
    pub(super) fn set_each<'key, I, K>(&mut self, values: I) -> Result<usize>
    where
        I: IntoIterator<Item = (K, Scalar)>,
        K: Into<FixKey<'key>>,
    {
        let mut writes: Vec<Write> = Vec::new();
        let mut typed: Vec<(i32, Scalar)> = Vec::new();
        for (key, value) in values {
            let key = key.into();
            match self.staged(&key, value, &|_, _| Ok(())) {
                Ok(Staged::Typed(tag, value)) => typed.push((tag, value)),
                Ok(Staged::Row(write)) => stage(&mut writes, write),
                Err(_) => {}
            }
        }
        let landed = typed.len() + writes.len();
        if landed > 0 {
            self.land_unsettled(typed, writes)?;
        }
        Ok(landed)
    }

    /// [`Self::set`] without settling: for the pass that writes several
    /// times and settles once after the last.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::set`] returns.
    pub(super) fn set_unsettled<'key>(
        &mut self,
        key: impl Into<FixKey<'key>>,
        value: Scalar,
    ) -> Result<()> {
        let key = key.into();
        match self.staged(&key, value, &|_, _| Ok(()))? {
            Staged::Typed(tag, value) => self.land_unsettled(vec![(tag, value)], Vec::new()),
            Staged::Row(write) => self.land_unsettled(Vec::new(), vec![write]),
        }
    }

    /// Lands staged writes: the typed facts on their holders, the row
    /// writes in one rebuild, what the written values imply restated beside
    /// them, then the identity settled once.
    ///
    /// A value a caller writes is restated exactly as a value a line states
    /// is: writing `Rule80A(47)` writes the `OrderCapacity(528)` that
    /// replaced it beside it, and a rule a registry states of its own
    /// applies here too. The pass is idempotent and never overwrites a
    /// stated value, so a message written to twice is the message, and
    /// writing what a rule would have written stands.
    fn land(&mut self, typed: Vec<(i32, Scalar)>, writes: Vec<Write>) -> Result<()> {
        if typed.is_empty() && writes.is_empty() {
            return Ok(());
        }
        // Only what a rule could be about is restated: a write of a tag no
        // rule speaks for is the write, and the pass is not run at all.
        let restates = typed
            .iter()
            .map(|(tag, _)| *tag)
            .chain(
                writes
                    .iter()
                    .filter_map(|write| write.field.as_fix().tag().ok().flatten()),
            )
            .any(|tag| super::latest::restates(&self.registry, tag));
        self.land_unsettled(typed, writes)?;
        if restates {
            super::latest::restate(self)?;
        }
        self.settle();
        Ok(())
    }

    /// [`Self::land`] without settling the identity after.
    fn land_unsettled(&mut self, typed: Vec<(i32, Scalar)>, writes: Vec<Write>) -> Result<()> {
        if !writes.is_empty() {
            self.write_all(writes)?;
        }
        for (tag, value) in typed {
            self.record(tag, &value);
        }
        Ok(())
    }

    /// One value resolved and typed for the child its key reaches, exactly
    /// as [`Self::set`] resolves and types one, against the row as it stands.
    fn staged<'key>(
        &self,
        key: &FixKey<'key>,
        value: Scalar,
        check: &impl Fn(&FixKey<'key>, &Field) -> Result<()>,
    ) -> Result<Staged> {
        let (at, mut field) = self.target(key)?;
        check(key, &field)?;
        let tag = field.as_fix().tag()?;
        // The capture's own column is nobody's to write here: a message
        // holds no fact for it, and landing one in the row would make the
        // object a line was read from a pair this message re-emits. Whoever
        // read the line states it on the row instead.
        if let Some(tag) = tag.filter(|tag| identity::is_capture_tag(*tag)) {
            return Err(refused_capture(field.name(), tag));
        }
        if let FixKey::Tag(tag) = *key {
            if identity::is_capture_tag(tag) {
                return Err(refused_capture(field.name(), tag));
            }
        }
        if let Some(tag) = tag.filter(|tag| identity::is_typed_tag(*tag)) {
            let value = if value.is_null() {
                Scalar::Null
            } else {
                field.scalar(value)?
            };
            return Ok(Staged::Typed(tag, value));
        }
        // Which tags the holders own is this crate's statement and not a
        // dictionary's, so a typed tag written under a registry that
        // declares no field for it still lands on its holder. There is no
        // field to type it through - the one above was invented for the
        // spelling - so the value crosses as it was written and the holder
        // types it.
        if tag.is_none() {
            if let FixKey::Tag(tag) = *key {
                if identity::is_typed_tag(tag) {
                    return Ok(Staged::Typed(tag, value));
                }
            }
        }
        let value = if value.is_null() {
            field.set_nullable(true);
            Scalar::Null
        } else {
            field.scalar(value)?
        };
        Ok(Staged::Row(Write { at, field, value }))
    }

    /// [`Self::set`], consuming the message.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::set`] returns.
    pub fn with_value<'key>(mut self, key: impl Into<FixKey<'key>>, value: Scalar) -> Result<Self> {
        self.set(key, value)?;
        Ok(self)
    }

    /// Removes what a key reaches, answering the value it held.
    ///
    /// A key reaching a typed fact clears it on its holder; one reaching a
    /// row child removes the child, and the children after it move up, so
    /// the tag and group indexes are reread. A key that reaches nothing
    /// answers `None` and changes nothing.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, StructType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut symbol = DataType::utf8().nullable_field("symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let registry = Arc::new(FixRegistry::from_fields([symbol.clone()])?);
    /// let root = DataType::from(StructType::from_fields([symbol, DataType::utf8().nullable_field("9999")])?)
    ///     .required_field("D");
    /// let value = Scalar::from_struct([
    ///     ("symbol", Scalar::from("AAPL")),
    ///     ("9999", Scalar::from("custom")),
    /// ])?;
    /// let mut msg = FixMsg::with_registry(registry, root, value)?;
    ///
    /// assert_eq!(msg.remove(55)?, Some(Scalar::from("AAPL")));
    /// assert_eq!(msg.get_by_tag(55), None);
    /// assert_eq!(msg.by_tag(9999)?, Scalar::from("custom"), "the neighbour is still reached");
    /// assert_eq!(msg.remove("nosuchfield")?, None);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the remaining children do
    /// not make a root, which leaves the message unchanged.
    pub fn remove<'key>(&mut self, key: impl Into<FixKey<'key>>) -> Result<Option<Scalar>> {
        let key = key.into();
        if let Some(tag) = self.typed_tag_of(&key) {
            let held = self.typed_fact(tag);
            if held.is_some() {
                self.record(tag, &Scalar::Null);
                self.settle();
            }
            return Ok(held);
        }
        let Some(at) = self.index_of_key(&key) else {
            return Ok(None);
        };
        let mut members = self.field.fields().to_vec();
        let mut values = self
            .value
            .as_sequence()
            .ok_or_else(|| {
                identity::refused(self.field.name(), "a canonical row", self.value.kind())
            })?
            .to_vec();
        if at >= members.len() || at >= values.len() {
            return Ok(None);
        }
        members.remove(at);
        let removed = values.remove(at);
        let dtype = DataType::from(StructType::from_fields(members)?);
        let field = self.rerooted(dtype);
        let plan = super::schema::column_plan_of(&field, &self.registry)?;
        self.field = field;
        self.value = Scalar::from_sequence(values);
        self.tags = tag_positions(&plan);
        self.groups = group_positions(&self.field, &self.registry);
        self.named = OnceLock::new();
        self.entries = OnceLock::new();
        self.settle();
        Ok(Some(removed))
    }

    /// The typed tag a key reaches, where it reaches one.
    fn typed_tag_of(&self, key: &FixKey<'_>) -> Option<i32> {
        let tag = match *key {
            FixKey::Tag(tag) => tag,
            FixKey::Id(id) => {
                self.registry
                    .identity_of(self.registry.get_field_by_id(id)?)?
                    .0
            }
            FixKey::Name(name) => {
                let known = self.known_by_name(name)?;
                self.registry.identity_of(known)?.0
            }
        };
        identity::is_typed_tag(tag).then_some(tag)
    }

    /// The child a key reaches and the field it is written under: an
    /// existing child's position where one is reached, and the field the
    /// registry resolves the key to, else the reached child's own.
    fn target(&self, key: &FixKey<'_>) -> Result<(Option<usize>, Field)> {
        match *key {
            FixKey::Tag(tag) => {
                let at = self.reached_by_tag(tag);
                match self.known_by_tag(tag) {
                    Some(known) => Ok((at, stated(known))),
                    None => {
                        let field = at
                            .and_then(|at| self.field.get_field_at(at))
                            .cloned()
                            .unwrap_or_else(|| {
                                DataType::utf8().nullable_field(format_smolstr!("{tag}"))
                            });
                        Ok((at, field))
                    }
                }
            }
            FixKey::Id(id) => {
                let known = self
                    .registry
                    .get_field_by_id(id)
                    .ok_or_else(|| absent(key))?;
                let at = self.index_of_name(known.name()).or_else(|| {
                    self.registry
                        .identity_of(known)
                        .and_then(|(tag, _)| self.index_of_tag(tag))
                });
                Ok((at, stated(known)))
            }
            FixKey::Name(name) => match self.known_by_name(name) {
                Some(known) => {
                    let by_tag = known
                        .as_fix()
                        .tag()
                        .ok()
                        .flatten()
                        .and_then(|tag| self.index_of_tag(tag));
                    Ok((
                        by_tag.or_else(|| self.child_index(&self.field, name)),
                        stated(known),
                    ))
                }
                None => {
                    let at = self
                        .child_index(&self.field, name)
                        .ok_or_else(|| absent(key))?;
                    let field = self
                        .field
                        .get_field_at(at)
                        .cloned()
                        .ok_or_else(|| absent(key))?;
                    Ok((Some(at), field))
                }
            },
        }
    }

    /// The position of the row child a key reaches, as a lookup reaches it.
    fn index_of_key(&self, key: &FixKey<'_>) -> Option<usize> {
        match *key {
            FixKey::Tag(tag) => self.reached_by_tag(tag),
            FixKey::Id(id) => {
                let known = self.registry.get_field_by_id(id)?;
                self.field.index_of(known.name())
            }
            FixKey::Name(name) => self.child_index(&self.field, name),
        }
    }

    /// Lands the planned row writes: each replaced at its position,
    /// appended when it has none.
    ///
    /// The whole row is rebuilt once, because a Struct's children and a
    /// row's values are both one shared allocation; everything that can
    /// refuse is asked before anything is stored, so a refusal leaves the
    /// message as it was. The tag and group indexes follow the children that
    /// changed, and the name table is carried through a write that leaves
    /// it true - an append, or a replacement under the child's own name -
    /// and dropped only where a rename makes it wrong.
    fn write_all(&mut self, writes: Vec<Write>) -> Result<()> {
        let (field, values, indexed) = stage_writes(&self.field, &self.value, writes)?;
        self.field = field;
        self.value = Scalar::from_sequence(values);
        let mut named = self.named.take();
        for change in indexed {
            if change.renamed {
                named = None;
            } else if change.appended {
                if let (Some(named), Some(child)) =
                    (named.as_mut(), self.field.fields().get(change.index))
                {
                    named
                        .entry(SmolStr::new(child.name()))
                        .or_insert(change.index);
                }
            }
            change.apply(&mut self.tags, Some(&mut self.groups));
        }
        self.named = OnceLock::new();
        if let Some(named) = named {
            let _ = self.named.set(named);
        }
        self.entries = OnceLock::new();
        Ok(())
    }

    /// The root over other children: its name, nullability and metadata,
    /// the Struct `dtype` under them.
    fn rerooted(&self, dtype: DataType) -> Field {
        Field::new_with_metadata(
            self.field.name(),
            dtype,
            self.field.is_nullable(),
            self.field.as_metadata().clone(),
        )
    }

    /// Re-emits this message on the wire, separated by `separator`.
    ///
    /// The standard header from the typed header, the event's own FIX tags,
    /// then the row in its order, each entry stating a value as one pair
    /// and an occurrence or a component as the pairs under it. What is
    /// emitted is the message as it now stands, derived values included.
    #[must_use]
    pub fn into_bytes(&self, separator: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        let (bands, split) = self.wire_bands();
        let (head, tail) = bands.split_at(split);
        emit_bytes(head, separator, &mut bytes);
        emit_bytes(self.entries(), separator, &mut bytes);
        emit_bytes(tail, separator, &mut bytes);
        bytes
    }

    /// The same, as text.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when a value holds a control byte.
    pub fn into_text(&self, separator: char) -> Result<String> {
        let mut text = String::new();
        let (bands, split) = self.wire_bands();
        let (head, tail) = bands.split_at(split);
        emit_text(head, separator, &mut text)?;
        emit_text(self.entries(), separator, &mut text)?;
        emit_text(tail, separator, &mut text)?;
        Ok(text)
    }

    /// The deterministic digest of what the wire carries: every entry of
    /// [`Self::into_bytes`], pre-order, so two messages that re-emit alike
    /// digest alike whatever separator either was read with.
    #[must_use]
    pub fn digest(&self) -> u128 {
        let (bands, split) = self.wire_bands();
        let (head, tail) = bands.split_at(split);
        super::digest::digest_of(&[head, self.entries(), tail])
    }

    /// Returns the registry this message resolves against.
    pub const fn registry(&self) -> &Arc<FixRegistry> {
        &self.registry
    }

    /// Returns the root Struct field: the row's schema, which holds every
    /// child the message states beyond its typed facts.
    pub const fn as_field(&self) -> &Field {
        &self.field
    }

    /// Returns the ordered row value.
    pub const fn as_value(&self) -> &Scalar {
        &self.value
    }

    /// Returns the value the root child an identifier names.
    ///
    /// An identifier is exact: it names one field, and the child is the one
    /// that field's name reaches.
    pub fn get_by_id(&self, id: FixId) -> Option<Scalar> {
        let known = self.registry.get_field_by_id(id)?;
        if let Some((tag, _)) = self.registry.identity_of(known) {
            if identity::is_typed_tag(tag) {
                return self.typed_fact(tag);
            }
        }
        self.value
            .get(self.field.index_of(known.name())?)
            .map(Cow::into_owned)
    }

    /// Returns the value the root child an identifier names, raising
    /// absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the identifier.
    pub fn by_id(&self, id: FixId) -> Result<Scalar> {
        self.get_by_id(id).ok_or_else(|| absent(FixKey::Id(id)))
    }

    /// Returns the value a tag names.
    ///
    /// A tag the typed holders own - a header tag, a crate column, one of
    /// the event's own tags - answers the fact the holder states, or nothing
    /// where it states none. Any other tag reaches the row: the registry
    /// resolves it to its canonical name, and that name picks the root
    /// child; a tag the dictionary does not answer is looked for under its
    /// decimal rendering, so an unknown tag a transcriber retained is still
    /// reachable.
    pub fn get_by_tag(&self, tag: i32) -> Option<Scalar> {
        if identity::is_typed_tag(tag) {
            return self.typed_fact(tag);
        }
        self.value
            .get(self.reached_by_tag(tag)?)
            .map(Cow::into_owned)
    }

    /// The value a tag names by the index alone: a typed fact, else the
    /// child declaring the tag, and never the two fallbacks of
    /// [`Self::get_by_tag`], which end in a name table built on the first
    /// miss. The [enriching pass](super::enrich) reads every native source
    /// and gathers every generic working-row column through this.
    pub(super) fn indexed_by_tag(&self, tag: i32) -> Option<Scalar> {
        if identity::is_typed_tag(tag) {
            return self.typed_fact(tag);
        }
        self.value.get(self.index_of_tag(tag)?).map(Cow::into_owned)
    }

    /// The child a tag reaches: the one carrying the tag, by one hash-free
    /// binary search over the index resolved at construction, else the one
    /// either fallback names.
    fn reached_by_tag(&self, tag: i32) -> Option<usize> {
        if let Some(index) = self.index_of_tag(tag) {
            return Some(index);
        }
        self.fallback_index(tag)
    }

    /// The child a tag reaches past the index: the one named as the
    /// dictionary names the tag, else the one named by the tag's decimal
    /// spelling.
    fn fallback_index(&self, tag: i32) -> Option<usize> {
        let named = self.named.get_or_init(|| {
            let mut named = FixMap::default();
            named.reserve(self.field.fields().len());
            for (index, child) in self.field.fields().iter().enumerate() {
                named.entry(SmolStr::new(child.name())).or_insert(index);
            }
            named
        });
        match self.known_by_tag(tag) {
            Some(known) => named.get(known.name()).copied(),
            None => named.get(format_smolstr!("{tag}").as_str()).copied(),
        }
    }

    /// The root child carrying one tag, by that child's own declaration.
    pub(super) fn index_of_tag(&self, tag: i32) -> Option<usize> {
        let at = self.tags.partition_point(|(held, _)| *held < tag);
        self.tags
            .get(at)
            .filter(|(held, _)| *held == tag)
            .map(|(_, index)| *index)
    }

    /// A tag identifies a member only when exactly one root child carries it.
    pub(super) fn unique_index_of_tag(&self, tag: i32) -> Option<usize> {
        let at = self.tags.partition_point(|(held, _)| *held < tag);
        let (held, index) = self.tags.get(at)?;
        if *held != tag || self.tags.get(at + 1).is_some_and(|(next, _)| *next == tag) {
            return None;
        }
        Some(*index)
    }

    pub(super) fn index_of_group(&self, counter: i32) -> Option<usize> {
        let index = self
            .groups
            .binary_search_by_key(&counter, |(tag, _)| *tag)
            .ok()?;
        if index > 0 && self.groups[index - 1].0 == counter
            || self
                .groups
                .get(index + 1)
                .is_some_and(|(tag, _)| *tag == counter)
        {
            return None;
        }
        Some(self.groups[index].1)
    }

    /// Returns the value a tag names, raising absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the tag.
    pub fn by_tag(&self, tag: i32) -> Result<Scalar> {
        self.get_by_tag(tag).ok_or_else(|| absent(FixKey::Tag(tag)))
    }

    /// Returns the value a name reaches.
    ///
    /// The name folds through the registry to its canonical spelling - a
    /// typed fact answers from its holder - and an exact root-child match is
    /// the fallback when the registry does not know it.
    pub fn get_by_name(&self, name: &str) -> Option<Scalar> {
        if let Some(known) = self.known_by_name(name) {
            if let Some((tag, _)) = self.registry.identity_of(known) {
                if identity::is_typed_tag(tag) {
                    return self.typed_fact(tag);
                }
            }
        }
        self.value
            .get(self.child_index(&self.field, name)?)
            .map(Cow::into_owned)
    }

    /// Returns the value a name reaches, raising absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the name.
    pub fn by_name(&self, name: &str) -> Result<Scalar> {
        self.get_by_name(name)
            .ok_or_else(|| absent(FixKey::Name(name)))
    }

    /// Returns the value a resolved path reaches.
    ///
    /// The path is the crate's one grammar, already parsed: nothing here
    /// splits a string, so a run addressing the same member a million times
    /// resolves the path once. A named segment resolves as
    /// [`Self::get_by_name`] does - the registry's canonical spelling first,
    /// then an exact match - and an indexed segment takes one occurrence of
    /// the Serie a repeating group is, which is what reaching a member
    /// needs: `Parties[0].PartyID`.
    ///
    /// A bare decimal is a name and not a position, exactly as it is one
    /// layer down where a text line's entry keyed `55` is reached by the path
    /// `55`. A path of one named segment is [`Self::get_by_name`].
    pub fn get_by_path(&self, path: &FieldPath) -> Option<Scalar> {
        let mut segments = path.segments().iter();
        let first = segments.next()?;
        let name = first.as_name()?;
        let (mut field, mut value) = match self.known_by_name(name) {
            Some(known)
                if self
                    .registry
                    .identity_of(known)
                    .is_some_and(|(tag, _)| identity::is_typed_tag(tag)) =>
            {
                let (tag, _) = self.registry.identity_of(known)?;
                (known.clone(), self.typed_fact(tag)?)
            }
            _ => {
                let index = self.child_index(&self.field, name)?;
                (
                    self.field.fields().get(index)?.clone(),
                    self.value.get(index)?.into_owned(),
                )
            }
        };
        for segment in segments {
            (field, value) = self.descend(&field, &value, segment)?;
        }
        Some(value)
    }

    /// Returns the value a resolved path reaches, raising absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the path.
    pub fn by_path(&self, path: &FieldPath) -> Result<Scalar> {
        self.get_by_path(path)
            .ok_or_else(|| absent(format_args!("path {path}")))
    }

    /// Returns the value a tag, an identifier, a name or a path reaches.
    ///
    /// Matches the key once and redirects: a tag to [`Self::get_by_tag`], an
    /// identifier to [`Self::get_by_id`], a name to [`Self::get_by_name`] -
    /// and, only where that reached nothing and the key spells more than one
    /// segment, to [`Self::get_by_path`] with the path that key states.
    pub fn get<'key>(&self, key: impl Into<FixKey<'key>>) -> Option<Scalar> {
        match key.into() {
            FixKey::Tag(tag) => self.get_by_tag(tag),
            FixKey::Id(id) => self.get_by_id(id),
            FixKey::Name(name) => self.named_or_path(name),
        }
    }

    /// Returns the value a tag, an identifier or a name reaches, raising
    /// absence.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::by_tag`], [`Self::by_id`] or
    /// [`Self::by_path`] raises, whichever the key selects.
    pub fn value<'key>(&self, key: impl Into<FixKey<'key>>) -> Result<Scalar> {
        match key.into() {
            FixKey::Tag(tag) => self.by_tag(tag),
            FixKey::Id(id) => self.by_id(id),
            FixKey::Name(name) => {
                if let Some(value) = self.get_by_name(name) {
                    return Ok(value);
                }
                match FieldPath::from_str(name) {
                    Ok(path) if path.segments().len() > 1 => self.by_path(&path),
                    _ => Err(absent(FixKey::Name(name))),
                }
            }
        }
    }

    /// The field a bare tag names: the tag's first holder in the registry.
    pub(super) fn known_by_tag(&self, tag: i32) -> Option<&Field> {
        self.registry.get_field_by_tag(tag).or_else(|| {
            self.registry
                .get_group_by_tag(tag)
                .filter(|group| matches!(group.dtype(), DataType::Map(_) | DataType::SortedMap(_)))
        })
    }

    /// The field a bare name reaches, canonical spelling or alias.
    pub(super) fn known_by_name(&self, name: &str) -> Option<&Field> {
        self.registry.get_message_field_by_name(name)
    }

    /// The position of the child `name` reaches under `parent`: the
    /// registry's canonical spelling first, then an exact match.
    fn child_index(&self, parent: &Field, name: &str) -> Option<usize> {
        self.known_by_name(name)
            .and_then(|known| parent.index_of(known.name()))
            .or_else(|| named_index(parent, name))
    }

    /// The position of the root child `name` spells, with no dictionary
    /// consulted: an exact match, else the one child the fold reaches.
    pub(super) fn index_of_name(&self, name: &str) -> Option<usize> {
        named_index(&self.field, name)
    }

    /// One key read as a name, and as the path it spells where it spells one.
    fn named_or_path(&self, key: &str) -> Option<Scalar> {
        if let Some(value) = self.get_by_name(key) {
            return Some(value);
        }
        let path = FieldPath::from_str(key).ok()?;
        (path.segments().len() > 1)
            .then(|| self.get_by_path(&path))
            .flatten()
    }

    /// The position one named segment reaches under `parent`.
    fn segment_index(&self, parent: &Field, segment: &FieldSegment) -> Option<usize> {
        self.child_index(parent, segment.as_name()?)
    }

    /// One step of a path: into a Struct child by name, or into one
    /// occupancy of the Serie a repeating group is.
    fn descend(
        &self,
        field: &Field,
        value: &Scalar,
        segment: &FieldSegment,
    ) -> Option<(Field, Scalar)> {
        match field.dtype() {
            DataType::Struct(_) => {
                let index = self.segment_index(field, segment)?;
                Some((
                    field.fields().get(index)?.clone(),
                    value.get(index)?.into_owned(),
                ))
            }
            DataType::Serie(item)
            | DataType::LargeSerie(item)
            | DataType::FixedSizeSerie(item, _)
            | DataType::SerieView(item)
            | DataType::LargeSerieView(item) => {
                let FieldSegment::Index(position) = segment else {
                    return None;
                };
                let len = value.as_serie()?.len();
                let at = if *position < 0 {
                    len.checked_sub(position.unsigned_abs() as usize)?
                } else {
                    usize::try_from(*position).ok()?
                };
                Some((item.as_ref().clone(), value.get(at)?.into_owned()))
            }
            map_dtype @ (DataType::Map(_) | DataType::SortedMap(_)) => {
                let map = &map_dtype
                    .as_mapping()
                    .expect("the variant was just matched");
                let FieldSegment::Key(key) = segment else {
                    return None;
                };
                Some((
                    map.entries().fields().get(1)?.clone(),
                    value.get_key(key.value())?.clone(),
                ))
            }
            _ => None,
        }
    }
}

/// The canonical entry order keeps up to 128 indexes (one KiB) on the stack.
const DIGEST_ENTRY_STACK_INDICES: usize = 128;

/// Feeds one level of the entry tree to a digest in canonical name order.
/// Equal names retain their arrival order, which keeps occurrences of one
/// repeating group distinct while making a reconstructed Struct's members
/// answer the same code as the parsed message's.
fn feed_entries(state: &mut crate::xxhash::Xxh3, entries: &[FixEntry]) {
    if entries
        .windows(2)
        .all(|pair| pair[0].name() <= pair[1].name())
    {
        for entry in entries {
            feed_entry(state, entry);
        }
        return;
    }
    let mut inline = [0_usize; DIGEST_ENTRY_STACK_INDICES];
    let mut heap = Vec::new();
    let order = if entries.len() <= inline.len() {
        &mut inline[..entries.len()]
    } else {
        heap.resize(entries.len(), 0);
        heap.as_mut_slice()
    };
    for (index, slot) in order.iter_mut().enumerate() {
        *slot = index;
    }
    order.sort_unstable_by(|left, right| {
        entries[*left]
            .name()
            .cmp(entries[*right].name())
            .then(left.cmp(right))
    });
    for index in order {
        feed_entry(state, &entries[*index]);
    }
}

/// Feeds one entry with boundaries around each variable-length member.
fn feed_entry(state: &mut crate::xxhash::Xxh3, entry: &FixEntry) {
    if super::digest::is_envelope(entry.tag()) {
        return;
    }
    state.write_usize(entry.name().len());
    state.write(entry.name().as_bytes());
    if let Some(value) = entry.value() {
        state.write_u8(1);
        state.write_usize(value.len());
        state.write(value.as_bytes());
    } else {
        state.write_u8(0);
    }
    state.write_usize(
        entry
            .entries()
            .iter()
            .filter(|child| !super::digest::is_envelope(child.tag()))
            .count(),
    );
    feed_entries(state, entry.entries());
}

/// One level of the row as the entries it states, in its order: every
/// child through [`entry_of`], except the counter scalar beside the group
/// it counts - at the root, in a component, in an occurrence alike - since
/// a group's count is the group entry's own value and the counter child
/// states nothing the entries do not already.
fn entries_of(registry: &FixRegistry, fields: &[Field], values: &[Scalar]) -> Vec<FixEntry> {
    let counters: Vec<i32> = fields
        .iter()
        .filter(|child| child.dtype().is_nested())
        .filter_map(|child| super::schema::tag_and_counter(registry, child).1)
        .collect();
    let counted = |child: &Field| {
        !counters.is_empty()
            && !child.dtype().is_nested()
            && super::schema::tag_and_counter(registry, child)
                .0
                .is_some_and(|tag| counters.contains(&tag))
    };
    let mut entries = Vec::new();
    for entry in fields
        .iter()
        .zip(values)
        .filter(|(child, _)| !counted(child))
        .filter_map(|(child, value)| entry_of(registry, child, value))
    {
        // Sized once, on the first entry, for every child there is: a level
        // stating nothing allocates nothing here, and one stating a hundred
        // grows the list once.
        if entries.capacity() == 0 {
            entries.reserve_exact(fields.len());
        }
        entries.push(entry);
    }
    entries
}

/// One row child as the entry it is: a scalar as one stated entry, a
/// repeating group as its counter entry with an entry per occurrence and
/// the occurrence's members under each, a component as an entry heading
/// its members; nothing for a child stating null.
fn entry_of(registry: &FixRegistry, field: &Field, value: &Scalar) -> Option<FixEntry> {
    if value.is_null() {
        return None;
    }
    let (tag, counter) = super::schema::tag_and_counter(registry, field);
    let tag = tag.unwrap_or(0);
    match field.dtype() {
        DataType::Serie(item) | DataType::LargeSerie(item) => {
            let occurrences = value.as_serie()?;
            // The item is one field for every occurrence, so its facts are
            // read once for all of them.
            let item_facts = super::schema::tag_and_counter(registry, item);
            let nested: Vec<FixEntry> = occurrences
                .iter()
                .filter_map(|occurrence| match item.dtype() {
                    DataType::Struct(_) => {
                        let members =
                            entries_of(registry, item.fields(), occurrence.as_sequence()?);
                        let own = item_facts.0.unwrap_or(0);
                        Some(FixEntry::new(own, item.name(), None).with_entries(members))
                    }
                    _ => entry_of(registry, item, &occurrence),
                })
                .collect();
            match counter {
                Some(counter) => Some(
                    FixEntry::new(
                        counter,
                        field.name(),
                        Some(format_smolstr!("{}", occurrences.len())),
                    )
                    .with_entries(nested),
                ),
                None => Some(FixEntry::new(tag, field.name(), None).with_entries(nested)),
            }
        }
        DataType::Struct(_) => {
            let members = entries_of(registry, field.fields(), value.as_sequence()?);
            Some(FixEntry::new(tag, field.name(), None).with_entries(members))
        }
        DataType::Map(_) | DataType::SortedMap(_) => {
            let members = value
                .as_mapping()?
                .iter()
                .filter_map(|(key, value)| Some(FixEntry::new(0, key.as_str()?, wire_text(value))))
                .collect();
            Some(FixEntry::new(tag, field.name(), None).with_entries(members))
        }
        _ => Some(FixEntry::new(
            tag,
            field.name(),
            wire_text_under(registry, field, value),
        )),
    }
}

/// The metadata a Map value states: every text key under its text value.
fn metadata_of(value: &Scalar) -> BTreeMap<SmolStr, SmolStr> {
    value
        .as_mapping()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(key, value)| {
                    Some((SmolStr::new(key.as_str()?), SmolStr::new(value.as_str()?)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Report that nothing in the message is reached by `what`.
fn absent(what: impl fmt::Display) -> Error {
    Error::absent("fix value", what)
}

/// The position of the child `name` spells under `parent`: an exact match,
/// else the one child the fold reaches - two children one fold reaches name
/// neither.
fn named_index(parent: &Field, name: &str) -> Option<usize> {
    let mut folded = None;
    let mut ambiguous = false;
    for (index, field) in parent.fields().iter().enumerate() {
        let held = field.name();
        if held == name {
            return Some(index);
        }
        if crate::folds_equal(held, name) {
            ambiguous |= folded.is_some();
            folded = Some(index);
        }
    }
    if ambiguous { None } else { folded }
}

/// Forgets the entry a child at `at` held in a sorted position index.
fn retire(index: &mut Vec<(i32, usize)>, key: Option<i32>, at: usize) {
    if let Some(key) = key {
        if let Ok(position) = index.binary_search(&(key, at)) {
            index.remove(position);
        }
    }
}

/// Records the entry the child at `at` carries in a sorted position index.
fn admit(index: &mut Vec<(i32, usize)>, key: Option<i32>, at: usize) {
    if let Some(key) = key {
        if let Err(position) = index.binary_search(&(key, at)) {
            index.insert(position, (key, at));
        }
    }
}

impl From<FixMsg> for Result<FixMsg> {
    fn from(message: FixMsg) -> Self {
        Ok(message)
    }
}

impl From<FixMsg> for OperationEventData {
    /// Moves the message's market operation out without re-reading or
    /// cloning any FIX content.
    fn from(message: FixMsg) -> Self {
        *message.event
    }
}

impl From<FixMsg> for MarketEventData {
    /// Moves the message's market event out, its operation facts dropped.
    fn from(message: FixMsg) -> Self {
        message.event.into_event()
    }
}

impl Clone for FixMsg {
    /// The message, without its name table, which the clone rebuilds on its
    /// own first ask; the entries, derived from the same row, are shared.
    fn clone(&self) -> Self {
        Self {
            registry: Arc::clone(&self.registry),
            event: self.event.clone(),
            header: self.header.clone(),
            capture: self.capture.clone(),
            lifted: self.lifted.clone(),
            forced: self.forced,
            row_stated: self.row_stated,
            text: self.text.clone(),
            metadata: self.metadata.clone(),
            tags: self.tags.clone(),
            named: OnceLock::new(),
            groups: self.groups.clone(),
            field: self.field.clone(),
            value: self.value.clone(),
            entries: self.entries.clone(),
            carried: self.carried.clone(),
            derived: self.derived.clone(),
            anomalies: self.anomalies.clone(),
            arrival_anomalies: self.arrival_anomalies,
        }
    }
}

impl fmt::Debug for FixMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FixMsg")
            .field("header", &self.header)
            .field("event", &self.event)
            .field("capture", &self.capture)
            .field("field", &self.field)
            .field("value", &self.value)
            .finish_non_exhaustive()
    }
}

impl PartialEq for FixMsg {
    /// Two messages are equal when they state the same facts and the same
    /// row against the same registry - the same `Arc`, or registries that
    /// hold the same fields.
    fn eq(&self, other: &Self) -> bool {
        self.event == other.event
            && self.header == other.header
            && self.capture == other.capture
            && self.text == other.text
            && self.metadata == other.metadata
            && self.field == other.field
            && self.value == other.value
            && self.carried == other.carried
            && (Arc::ptr_eq(&self.registry, &other.registry) || self.registry == other.registry)
    }
}

impl Eq for FixMsg {}

impl Hash for FixMsg {
    /// Hashes the settled code, the row's schema and its value; the
    /// registry is part of equality but not of the hash, which keeps equal
    /// messages hashing alike.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.event.get_currhashcode().hash(state);
        self.field.hash(state);
        self.value.hash(state);
    }
}

impl Element for FixMsg {
    fn get_curruuid(&self) -> Uuid {
        self.event.get_curruuid()
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.event.set_curruuid(curruuid);
    }

    fn get_crossuuid(&self) -> Uuid {
        self.event.get_crossuuid()
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.event.set_crossuuid(crossuuid);
    }

    fn get_crosscode(&self) -> &str {
        self.event.get_crosscode()
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.event.set_crosscode(crosscode);
    }

    fn get_currhashcode(&self) -> u64 {
        self.event.get_currhashcode()
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.event.set_currhashcode(hashcode);
    }

    fn get_crosshashcode(&self) -> u64 {
        self.event.get_crosshashcode()
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.event.set_crosshashcode(crosshashcode);
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        self.event.get_srcuuids()
    }

    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
        self.event.set_srcuuids(sources);
    }

    /// A message's order is its instant.
    fn is_after(&self, other: &Self) -> bool {
        self.event.is_after(&other.event)
    }

    /// The identity settled again from what the message now states.
    fn finalize(&mut self) {
        self.settle();
    }

    /// Another raw observation carrying this complete session-event identity
    /// is the same event and fully merges before predecessor logic. Otherwise
    /// the timed market reading descends from the whole lineage of the one it
    /// follows, the predecessor last. A graph-derived expiry deliberately
    /// follows even though it retains the source delivery identity.
    fn with_previous(self, previous: &Self) -> Option<Self> {
        if self.should_merge_session_event(previous) {
            return self.merge_session_event(previous).ok();
        }
        self.following_operation(previous)
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        let mut merged = self.merging_operation_event(other)?;
        merged.fold_anomalies(other);
        Some(merged)
    }
}

impl Event for FixMsg {
    /// The timed restatement, and then the market's: a message logged at a
    /// second hop takes the live message's place in its chain - the
    /// predecessor, the position, the snapshot and the step before it - and
    /// what that chain is about where this reading stated none of it.
    fn restating(self, live: &Self) -> Self {
        crate::graph::market::restating_operation(self, live)
    }

    fn get_currunix(&self) -> i64 {
        self.event.get_currunix()
    }

    fn set_currunix(&mut self, unix: i64) {
        self.event.set_currunix(unix);
    }

    fn get_state(&self) -> &State {
        self.event.get_state()
    }

    fn set_state(&mut self, state: State) {
        self.forced = true;
        self.event.set_state(state);
    }

    fn is_execution(&self) -> bool {
        self.reports_execution()
    }

    fn get_seqnum(&self) -> u64 {
        self.event.get_seqnum()
    }

    fn set_seqnum(&mut self, seqnum: u64) {
        self.event.set_seqnum(seqnum);
    }

    fn get_creaunix(&self) -> Option<i64> {
        self.event.get_creaunix()
    }

    fn set_creaunix(&mut self, unix: Option<i64>) {
        self.event.set_creaunix(unix);
    }

    fn get_execunix(&self) -> Option<i64> {
        self.event.get_execunix()
    }

    fn set_execunix(&mut self, unix: Option<i64>) {
        if unix.is_some() {
            self.row_stated |= ROW_STATED_EXECUTION;
        } else {
            self.row_stated &= !ROW_STATED_EXECUTION;
        }
        self.event.set_execunix(unix);
    }

    fn get_recdunix(&self) -> Option<i64> {
        self.event.get_recdunix()
    }

    fn set_recdunix(&mut self, unix: Option<i64>) {
        if unix.is_some() {
            self.row_stated |= ROW_STATED_RECORDING;
        } else {
            self.row_stated &= !ROW_STATED_RECORDING;
        }
        self.event.set_recdunix(unix);
    }

    fn get_exprtime(&self) -> Option<i64> {
        self.event.get_exprtime()
    }

    fn set_exprtime(&mut self, unix: Option<i64>) {
        self.forced = true;
        self.event.set_exprtime(unix);
    }

    fn get_prevunix(&self) -> Option<i64> {
        self.event.get_prevunix()
    }

    fn set_prevunix(&mut self, unix: Option<i64>) {
        self.event.set_prevunix(unix);
    }

    fn get_prevuuid(&self) -> Option<Uuid> {
        self.event.get_prevuuid()
    }

    fn set_prevuuid(&mut self, uuid: Option<Uuid>) {
        self.event.set_prevuuid(uuid);
    }

    fn get_snapunix(&self) -> Option<i64> {
        self.event.get_snapunix()
    }

    fn set_snapunix(&mut self, unix: Option<i64>) {
        self.event.set_snapunix(unix);
    }
}

impl Market for FixMsg {
    fn get_price(&self) -> Option<Decimal18> {
        self.event.get_price()
    }

    fn set_price(&mut self, px: Option<Decimal18>) {
        self.forced = true;
        self.event.set_price(px);
    }

    fn get_currency(&self) -> &Ccy {
        self.event.get_currency()
    }

    fn set_currency(&mut self, currency: Ccy) {
        self.forced = true;
        self.event.set_currency(currency);
    }

    fn get_quantity(&self) -> Option<Decimal18> {
        self.event.get_quantity()
    }

    fn set_quantity(&mut self, qty: Option<Decimal18>) {
        self.forced = true;
        self.event.set_quantity(qty);
    }

    fn get_unit(&self) -> &Unit {
        self.event.get_unit()
    }

    fn set_unit(&mut self, unit: Unit) {
        self.forced = true;
        self.event.set_unit(unit);
    }

    fn get_side(&self) -> Side {
        self.event.get_side()
    }

    fn set_side(&mut self, side: Side) {
        self.forced = true;
        self.event.set_side(side);
    }

    fn get_securityids(&self) -> &SecurityIds {
        self.event.get_securityids()
    }

    fn set_securityids(&mut self, ids: SecurityIds) -> Result<()> {
        self.derived = SecurityIds::default();
        let held: Vec<SecurityId> = self.event.get_securityids().iter().cloned().collect();
        for id in &held {
            if ids.get(id.sectype().as_str()) != Some(id.code()) {
                self.sync_security_id(&id.sectype(), None)?;
            }
        }
        for id in ids.iter() {
            if self.event.get_securityids().get(id.sectype().as_str()) != Some(id.code()) {
                self.sync_security_id(&id.sectype(), Some(id.code()))?;
            }
        }
        self.forced = true;
        self.row_stated_securityids(&ids);
        self.event.set_securityids(ids)
    }

    fn insert_securityid(&mut self, id: SecurityId) -> Result<bool> {
        let key = id.sectype();
        // A stated identifier replaces a derived one under its key; a stated
        // one already held is not replaced.
        let derived = self.derived.remove(&key).is_some();
        if !derived && self.event.get_securityids().contains_key(key.as_str()) {
            return Ok(false);
        }
        self.sync_security_id(&key, Some(id.code()))?;
        self.forced = true;
        if let Some(bit) = row_stated_securityid_bit(key.as_str()) {
            self.row_stated |= bit;
        }
        if derived {
            let mut ids = self.event.get_securityids().clone();
            ids.set(id);
            self.event.set_securityids(ids).map(|()| true)
        } else {
            self.event.insert_securityid(id)
        }
    }

    fn remove_securityid(&mut self, key: &SecType) -> Result<bool> {
        self.derived.remove(key);
        if !self.event.get_securityids().contains_key(key.as_str()) {
            return Ok(false);
        }
        self.sync_security_id(key, None)?;
        self.forced = true;
        if let Some(bit) = row_stated_securityid_bit(key.as_str()) {
            self.row_stated &= !bit;
        }
        self.event.remove_securityid(key)
    }

    fn derive_securityid(&mut self, id: SecurityId) -> bool {
        let added = self.event.derive_securityid(id.clone());
        if added {
            self.derived.insert(id);
        }
        added
    }

    fn get_cficode(&self) -> Option<&CfiCode> {
        self.event.get_cficode()
    }

    fn set_cficode(&mut self, cficode: Option<CfiCode>) {
        self.forced = true;
        self.event.set_cficode(cficode);
    }

    fn get_miccode(&self) -> Option<&MicCode> {
        self.event.get_miccode()
    }

    fn set_miccode(&mut self, miccode: Option<MicCode>) {
        self.forced = true;
        if miccode.is_some() {
            self.row_stated |= ROW_STATED_MIC;
        } else {
            self.row_stated &= !ROW_STATED_MIC;
        }
        self.event.set_miccode(miccode);
    }

    fn get_lastpx(&self) -> Option<Decimal18> {
        self.event.get_lastpx()
    }

    fn set_lastpx(&mut self, px: Option<Decimal18>) {
        self.forced = true;
        self.event.set_lastpx(px);
    }

    fn get_lastqty(&self) -> Option<Decimal18> {
        self.event.get_lastqty()
    }

    fn set_lastqty(&mut self, qty: Option<Decimal18>) {
        self.forced = true;
        self.event.set_lastqty(qty);
    }

    fn get_avgpx(&self) -> Option<Decimal18> {
        self.event.get_avgpx()
    }

    fn set_avgpx(&mut self, px: Option<Decimal18>) {
        self.forced = true;
        self.event.set_avgpx(px);
    }

    fn get_cumqty(&self) -> Option<Decimal18> {
        self.event.get_cumqty()
    }

    fn set_cumqty(&mut self, qty: Option<Decimal18>) {
        self.forced = true;
        self.event.set_cumqty(qty);
    }

    fn get_leavesqty(&self) -> Option<Decimal18> {
        self.event.get_leavesqty()
    }

    fn set_leavesqty(&mut self, qty: Option<Decimal18>) {
        self.forced = true;
        self.event.set_leavesqty(qty);
    }

    fn get_prevpx(&self) -> Option<Decimal18> {
        self.event.get_prevpx()
    }

    fn set_prevpx(&mut self, px: Option<Decimal18>) {
        self.forced = true;
        self.event.set_prevpx(px);
    }

    fn get_prevqty(&self) -> Option<Decimal18> {
        self.event.get_prevqty()
    }

    fn set_prevqty(&mut self, qty: Option<Decimal18>) {
        self.forced = true;
        self.event.set_prevqty(qty);
    }

    fn get_spotrate(&self) -> Option<Decimal18> {
        self.event.get_spotrate()
    }

    fn set_spotrate(&mut self, rate: Option<Decimal18>) {
        self.forced = true;
        self.event.set_spotrate(rate);
    }

    fn get_forwardpoints(&self) -> Option<Decimal18> {
        self.event.get_forwardpoints()
    }

    fn set_forwardpoints(&mut self, points: Option<Decimal18>) {
        self.forced = true;
        self.event.set_forwardpoints(points);
    }

    fn get_ticker(&self) -> Option<&str> {
        self.event.get_ticker()
    }

    fn set_ticker(&mut self, ticker: Option<SmolStr>) {
        self.forced = true;
        self.event.set_ticker(ticker);
    }

    fn get_metadata(&self) -> &Metadata {
        &self.metadata
    }

    fn set_metadata(&mut self, metadata: Option<Metadata>) {
        self.metadata = metadata.unwrap_or_default();
    }
}

impl Operation for FixMsg {
    fn get_marketoperationid(&self) -> Option<i32> {
        self.event.get_marketoperationid()
    }

    fn set_marketoperationid(&mut self, marketoperationid: Option<i32>) {
        self.forced = true;
        self.event.set_marketoperationid(marketoperationid);
    }

    fn get_tif(&self) -> Option<&TimeInForce> {
        self.event.get_tif()
    }

    fn set_tif(&mut self, tif: Option<TimeInForce>) {
        self.forced = true;
        self.event.set_tif(tif);
    }

    fn get_tradable(&self) -> Option<bool> {
        self.event.get_tradable()
    }

    fn set_tradable(&mut self, tradable: Option<bool>) {
        self.forced = true;
        self.event.set_tradable(tradable);
    }

    fn get_accountids(&self) -> &IdMap {
        self.event.get_accountids()
    }

    fn set_accountids(&mut self, ids: IdMap) -> Result<()> {
        self.write_idmap(IdMapKind::Account, &ids)?;
        self.event.set_accountids(ids)
    }

    fn insert_accountid(&mut self, key: &str, value: &str) -> Result<bool> {
        if self.event.get_accountids().contains_key(key) {
            return Ok(false);
        }
        let tag = Self::idmap_source(IdMapKind::Account, key)?;
        self.set_unsettled(tag, Scalar::from(value))?;
        self.event.insert_accountid(key, value)
    }

    fn remove_accountid(&mut self, key: &str) -> Result<bool> {
        if !self.event.get_accountids().contains_key(key) {
            return Ok(false);
        }
        let tag = Self::idmap_source(IdMapKind::Account, key)?;
        self.set_unsettled(tag, Scalar::Null)?;
        self.event.remove_accountid(key)
    }

    fn get_userids(&self) -> &IdMap {
        self.event.get_userids()
    }

    fn set_userids(&mut self, ids: IdMap) -> Result<()> {
        self.write_idmap(IdMapKind::User, &ids)?;
        self.event.set_userids(ids)
    }

    fn insert_userid(&mut self, key: &str, value: &str) -> Result<bool> {
        if self.event.get_userids().contains_key(key) {
            return Ok(false);
        }
        let tag = Self::idmap_source(IdMapKind::User, key)?;
        self.set_unsettled(tag, Scalar::from(value))?;
        self.event.insert_userid(key, value)
    }

    fn remove_userid(&mut self, key: &str) -> Result<bool> {
        if !self.event.get_userids().contains_key(key) {
            return Ok(false);
        }
        let tag = Self::idmap_source(IdMapKind::User, key)?;
        self.set_unsettled(tag, Scalar::Null)?;
        self.event.remove_userid(key)
    }

    fn get_altids(&self) -> &IdMap {
        self.event.get_altids()
    }

    fn set_altids(&mut self, ids: IdMap) -> Result<()> {
        self.write_idmap(IdMapKind::Alt, &ids)?;
        self.event.set_altids(ids)
    }

    fn insert_altid(&mut self, key: &str, value: &str) -> Result<bool> {
        if self.event.get_altids().contains_key(key) {
            return Ok(false);
        }
        let tag = Self::idmap_source(IdMapKind::Alt, key)?;
        self.set_unsettled(tag, Scalar::from(value))?;
        self.event.insert_altid(key, value)
    }

    fn remove_altid(&mut self, key: &str) -> Result<bool> {
        if !self.event.get_altids().contains_key(key) {
            return Ok(false);
        }
        let tag = Self::idmap_source(IdMapKind::Alt, key)?;
        self.set_unsettled(tag, Scalar::Null)?;
        self.event.remove_altid(key)
    }

    fn get_bid(&self) -> Option<&Lane> {
        self.event.get_bid()
    }

    fn set_bid(&mut self, lane: Option<Lane>) {
        self.forced = true;
        self.event.set_bid(lane);
    }

    fn get_ask(&self) -> Option<&Lane> {
        self.event.get_ask()
    }

    fn set_ask(&mut self, lane: Option<Lane>) {
        self.forced = true;
        self.event.set_ask(lane);
    }
}

impl FixMsg {
    /// Writes every entry of `ids` to its source field and removes the
    /// entries the message holds that `ids` does not.
    fn write_idmap(&mut self, kind: IdMapKind, ids: &IdMap) -> Result<()> {
        let held = match kind {
            IdMapKind::Account => self.event.get_accountids().clone(),
            IdMapKind::User => self.event.get_userids().clone(),
            IdMapKind::Alt => self.event.get_altids().clone(),
        };
        // A key no top-level field states - a party role's - is rebuilt from
        // its group at the next settle and has nothing to null here.
        for (key, _) in held.iter() {
            if !ids.contains_key(key) {
                if let Ok(tag) = Self::idmap_source(kind, key) {
                    self.set_unsettled(tag, Scalar::Null)?;
                }
            }
        }
        for (key, value) in ids.iter() {
            if held.get(key) != Some(value) {
                let tag = Self::idmap_source(kind, key)?;
                self.set_unsettled(tag, Scalar::from(value))?;
            }
        }
        Ok(())
    }

    /// Sets the row-stated bit of every crated identifier column `ids`
    /// states and clears the bit of every one it does not.
    fn row_stated_securityids(&mut self, ids: &SecurityIds) {
        for (key, bit) in [
            ("ISIN", ROW_STATED_ISIN),
            ("BLOOMBERG", ROW_STATED_BLOOMBERG),
            ("FIGI", ROW_STATED_FIGI),
        ] {
            if ids.contains_key(key) {
                self.row_stated |= bit;
            } else {
                self.row_stated &= !bit;
            }
        }
    }
}

/// Which identifier map a source field fills.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IdMapKind {
    Account,
    User,
    Alt,
}

impl IdMapKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Account => "accountids",
            Self::User => "userids",
            Self::Alt => "altids",
        }
    }
}

/// The top-level fields that state an identifier-map entry, each under the
/// key it fills: the dictionary's `FIX:idmap` sources, held here until the
/// bridge fields join them. Party roles are read from the `parties` group
/// beside these.
const IDMAP_SOURCES: [(IdMapKind, &str, i32); 13] = [
    (IdMapKind::Account, "ACCOUNT", 1),
    (IdMapKind::User, "SENDERSUBID", 50),
    (IdMapKind::User, "ONBEHALFOFSUBID", 116),
    (IdMapKind::Alt, "ORDERID", 37),
    (IdMapKind::Alt, "SECONDARYORDERID", 198),
    (IdMapKind::Alt, "CLORDID", 11),
    (IdMapKind::Alt, "ORIGCLORDID", 41),
    (IdMapKind::Alt, "EXECID", 17),
    (IdMapKind::Alt, "TRDMATCHID", 880),
    (IdMapKind::Alt, "QUOTEID", 117),
    (IdMapKind::Alt, "QUOTEREQID", 131),
    (IdMapKind::Alt, "MDREQID", 262),
    (IdMapKind::Alt, "TRADEID", 1003),
];

/// Whether `text` is one of the spellings a wire uses for no value at all.
fn is_null_like(text: &str) -> bool {
    matches!(
        text.to_ascii_uppercase().as_str(),
        "NULL" | "NONE" | "N/A" | "[N/A]" | "NA" | "-"
    )
}

/// The row-stated bit of the crated column a security-identifier key
/// stands for, where one does.
const fn row_stated_securityid_bit(key: &str) -> Option<u16> {
    match key.as_bytes() {
        b"ISIN" => Some(ROW_STATED_ISIN),
        b"BLOOMBERG" => Some(ROW_STATED_BLOOMBERG),
        b"FIGI" => Some(ROW_STATED_FIGI),
        _ => None,
    }
}

/// A cell's text: a string as it is, a code as its spelling, an integer as
/// its digits; trimmed, and none where empty or of another shape.
fn scalar_text(value: &Scalar) -> Option<SmolStr> {
    if let Some(text) = value.as_str() {
        let text = text.trim();
        return (!text.is_empty()).then(|| SmolStr::new(text));
    }
    value.as_i64().map(|held| format_smolstr!("{held}"))
}
