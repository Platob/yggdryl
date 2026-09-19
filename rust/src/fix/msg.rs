//! A FIX message: its typed facts, its row, and the registry that types it.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use smol_str::{SmolStr, format_smolstr};

use super::build::stated;
use super::entry::{FixEntry, emit_bytes, emit_text, wire_text, wire_text_under};
use super::identity::{self, FixCapture, FixHeader, FixLifted, Typed};
use super::{FixId, FixKey, FixRegistry};
use crate::graph::{Element, Event, MarketElement, MarketEvent, MarketEventData};
use crate::sequence::SequenceType;
use crate::xxhash;
use crate::{
    Bloomberg, Cfi, Currency, Cusip, Decimal18, Isin, Mic, Sedol, Side, State, StructureType, Uuid,
};
use crate::{DataType, Error, Field, FieldPath, FieldSegment, Result, Scalar};

/// A FIX message: a market event with a FIX body around it.
///
/// Three typed holders and one row. The [`MarketEventData`] is the event
/// the message is - its identity, when it happened, where it stands and the
/// market's facts - and the message answers [`Element`], [`Event`] and
/// [`MarketElement`] through it, so a walk over messages reads them as it
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
/// XXH3-64 of the event's facts, the header and the named content of the
/// row, the identity the UUIDv7 the instant and the code derive, and the
/// cross identity the UUIDv8 the cross code's digest derives - the first
/// chain identifier the message spells, `OrderID` before `ClOrdID`. Every
/// write settles it again, so a written code or identity is overwritten by
/// the settled one. [`Self::entries`] is the row read as a tree, for a
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
/// use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, StructureType};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut symbol = DataType::utf8().required_field("Symbol");
/// symbol.as_fix_mut().set_tag(55)?;
/// symbol.as_fix_mut().set_names(["Ticker"])?;
/// let mut qty = DataType::Int64.required_field("OrderQty");
/// qty.as_fix_mut().set_tag(38)?;
/// let registry = Arc::new(FixRegistry::from_fields([symbol.clone(), qty.clone()])?);
///
/// let root = DataType::from(StructureType::from_fields([symbol, qty, DataType::utf8().nullable_field("9999")])?)
///     .required_field("NewOrderSingle");
/// let value = Scalar::from_record([
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
    event: Box<MarketEventData>,
    /// The standard header, typed.
    header: Box<FixHeader>,
    /// What the line said about the capture it was written for, typed: a
    /// bridge's own row header. What the *reader* said about the line is
    /// held nowhere here.
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
    named: OnceLock<HashMap<SmolStr, usize>>,
    /// Group positions keyed by their `FIX:counter`, separate from tag values.
    groups: Vec<(i32, usize)>,
    field: Field,
    value: Scalar,
    /// The row read as a tree, derived on the first ask and dropped by
    /// every write.
    entries: OnceLock<Vec<FixEntry>>,
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
    let dtype = DataType::from(StructureType::from_fields(members)?);
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
fn tag_positions(columns: &super::schema::Columns) -> Vec<(i32, usize)> {
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

fn group_positions(field: &Field) -> Vec<(i32, usize)> {
    let mut held: Vec<_> = field
        .fields()
        .iter()
        .enumerate()
        .filter_map(|(index, child)| Some((child.as_fix().counter().ok()??, index)))
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
        Self::assemble(registry, field, value, None)
    }

    /// Builds the message the one builder finished, checking nothing twice.
    ///
    /// Every value in the row went through the contract of the field it
    /// lands under, so the row is canonical by construction. The sending
    /// clock is what the message stated, else `fallback_sending_time`, else
    /// now: initial intake settles clocks, and replay never reads now.
    pub(super) fn from_built(
        registry: Arc<FixRegistry>,
        built: super::build::Built,
        fallback_sending_time: Option<&Scalar>,
    ) -> Result<Self> {
        let super::build::Built { field, value, .. } = built;
        Self::assemble(registry, field, value, fallback_sending_time)
    }

    /// One message from a root and its canonical row: the typed facts are
    /// lifted out of the children that state them, the rest is the row,
    /// the clocks are settled and the identity derived.
    fn assemble(
        registry: Arc<FixRegistry>,
        field: Field,
        value: Scalar,
        fallback_sending_time: Option<&Scalar>,
    ) -> Result<Self> {
        let plan = super::schema::column_plan_of(&field, &registry)?;
        let held = value.as_sequence().ok_or_else(|| {
            identity::refused(field.name(), "a canonical Struct row", value.kind())
        })?;
        let mut event = Box::new(MarketEventData::default());
        let mut header = Box::new(FixHeader::unknown());
        let mut capture = Box::new(FixCapture::default());
        let mut lifted = Box::new(FixLifted::default());
        let mut text = None;
        let mut metadata = BTreeMap::new();
        let mut members = Vec::with_capacity(field.fields().len());
        let mut values = Vec::with_capacity(held.len());
        let mut stated_sending = false;
        let mut stated_unix = false;
        let mut stated_creation = false;
        let mut transact = None;
        let mut origin = None;
        // A typed tag is lifted out of the row onto its holder, and a holder
        // keeps one fact per tag - so a row stating one tag twice is left
        // where it stands rather than collapsed into one slot. Two children
        // under `ClOrdID(11)`, a venue's own beside the client's, are two
        // facts and a reader addressing them by name must still find both.
        let shared = |wanted: i32| {
            plan.iter()
                .filter(|column| column.tag == Some(wanted))
                .take(2)
                .count()
                > 1
        };
        for ((child, column), value) in field.fields().iter().zip(plan.iter()).zip(held) {
            match column.tag {
                Some(tag) if identity::is_typed_tag(tag) && !shared(tag) => {
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
                Some(60) => {
                    transact = value.temporal_count_at(crate::TimeUnit::Nanosecond);
                    members.push(child.clone());
                    values.push(value.clone());
                }
                Some(122) => {
                    origin = value.temporal_count_at(crate::TimeUnit::Nanosecond);
                    members.push(child.clone());
                    values.push(value.clone());
                }
                _ => {
                    members.push(child.clone());
                    values.push(value.clone());
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
        // The instant the message happened: what it states, else when the
        // transaction it reports happened, else when it was sent. A
        // `TransactTime` stating a day and no clock - which a bridge writes
        // as `20260814` - names no instant, so the sending clock stands in.
        if !stated_unix {
            const DAY: i64 = 86_400_000_000_000;
            let transact = transact.filter(|unix| unix.rem_euclid(DAY) != 0);
            event.set_currunix(transact.unwrap_or_else(|| header.sendingtime()));
        }
        // What a message says about its own creation, strongest first: a
        // stated creation, then `OrigSendingTime(122)`, then the instant it
        // happened. 122 is in the middle because of what it means: a resend
        // carries the instant the original was sent, and that original is
        // when this message came into being, so a replayed message dated
        // only by the resend would otherwise be created at the moment it
        // was replayed.
        if !stated_creation {
            event.set_creaunix(Some(origin.unwrap_or_else(|| event.get_currunix())));
        }
        let field = Field::new_with_metadata(
            field.name(),
            DataType::from(StructureType::from_fields(members)?),
            field.is_nullable(),
            field.as_metadata().clone(),
        );
        let plan = super::schema::column_plan_of(&field, &registry)?;
        let tags = tag_positions(&plan);
        let groups = group_positions(&field);
        let mut message = Self {
            registry,
            event,
            header,
            capture,
            lifted,
            forced: false,
            text,
            metadata,
            tags,
            named: OnceLock::new(),
            groups,
            field,
            value: Scalar::from_sequence(values),
            entries: OnceLock::new(),
        };
        message.settle();
        Ok(message)
    }

    /// Replaces the row with another statement of the same content, as a
    /// restatement leaves it: the typed facts a restated child states fill
    /// their holders, the indexes are reread, and the identity settled.
    pub(super) fn replace_content(&mut self, field: Field, values: Vec<Scalar>) -> Result<()> {
        let plan = super::schema::column_plan_of(&field, &self.registry)?;
        let mut members = Vec::with_capacity(field.fields().len());
        let mut kept = Vec::with_capacity(values.len());
        for ((child, column), value) in field.fields().iter().zip(plan.iter()).zip(values) {
            match column.tag {
                Some(tag) if identity::is_typed_tag(tag) => {
                    if !value.is_null() {
                        self.record(tag, &value);
                    }
                }
                // The capture's own column, which no restatement of the
                // content can make a fact of the message: read past, as
                // every other door reads it past.
                Some(tag) if identity::is_capture_tag(tag) => {}
                None if child.name().contains('.') => {
                    if let Some(held) = value.as_str().filter(|held| !held.is_empty()) {
                        self.metadata
                            .insert(SmolStr::new(child.name()), SmolStr::new(held));
                    }
                }
                _ => {
                    members.push(child.clone());
                    kept.push(value);
                }
            }
        }
        self.field = Field::new_with_metadata(
            field.name(),
            DataType::from(StructureType::from_fields(members)?),
            field.is_nullable(),
            field.as_metadata().clone(),
        );
        self.value = Scalar::from_sequence(kept);
        let plan = super::schema::column_plan_of(&self.field, &self.registry)?;
        self.tags = tag_positions(&plan);
        self.groups = group_positions(&self.field);
        self.named = OnceLock::new();
        self.entries = OnceLock::new();
        self.settle();
        Ok(())
    }

    /// Records one typed fact on the holder that owns it; a null clears it.
    fn record(&mut self, tag: i32, value: &Scalar) -> bool {
        if tag == identity::TEXT_TAG {
            self.text = value.as_str().map(SmolStr::new);
            return true;
        }
        if tag == super::METADATA_TAG_NAME.0 {
            self.metadata = metadata_of(value);
            return true;
        }
        identity::record(
            &mut self.event,
            &mut self.header,
            &mut self.capture,
            &mut self.lifted,
            tag,
            value,
        )
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

    /// Settles the identity from what the message states: the cross code
    /// where it names none yet, the cross codes in step with it, the code
    /// the content digests to, and the identity the instant and the code
    /// derive.
    pub(super) fn settle(&mut self) {
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
        self.event.fill_market();
        self.event.sync_cross();
        let currhashcode = self.currhashcode();
        self.event.finalized(currhashcode);
    }

    /// What the message implies about its market, read off the FIX fields
    /// it states and filled onto the event.
    ///
    /// Every market fact is FIX's own, and a message states the ones it
    /// states: `Price(44)`, `OrderQty(38)` or `Quantity(53)`, the fill and
    /// the progress, the side and the currency, the instrument's
    /// identifiers under their sources, the market, the unit, the state and
    /// the two quote lanes. This reads each and hands it to the trait,
    /// which is where the ladder that decides what the message is *about*
    /// lives - [`MarketElement::fill_market`], called straight after.
    ///
    /// Only an absent fact is filled, so what a lifecycle walk folded
    /// forward - the state it reached, what a price moved from, the
    /// instrument the chain is about - survives being settled again. And
    /// nothing filled here reaches the wire, the arrival record or the code
    /// the message digests to: those read what the message *stated*, and a
    /// derived fact is not a statement.
    fn derive_market(&mut self) {
        if self.forced {
            return;
        }
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
            .and_then(|held| Currency::new(&held).ok());
        let unit = word(996);
        let tif = word(identity::TIMEINFORCE_TAG);
        // The instrument: what it is classified as, what it is called, and
        // its identifiers under the sources that name them.
        let cficode = self
            .classification()
            .and_then(|held| Cfi::new(&held).ok())
            .or_else(|| word(super::cfi::CFICODE_TAG).and_then(|held| Cfi::new(&held).ok()));
        let symbolticker = word(55).filter(|held| held != "[N/A]" && held != "[N/A");
        let isincode = self.identifier(&["4"], |held| Isin::new(held).ok());
        let cusipcode = self.identifier(&["1"], |held| Cusip::new(held).ok());
        let sedolcode = self.identifier(&["2"], |held| Sedol::new(held).ok());
        let bloombergcode = self.identifier(&["A", "S"], |held| Bloomberg::new(held).ok());
        // The market it is listed on, routed to, or last traded on.
        let miccode = word(207)
            .or_else(|| word(100))
            .or_else(|| word(30))
            .and_then(|held| Mic::new(&held).ok());
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
        let expirunix = [126, 62, 432, 541].into_iter().find_map(|tag| {
            by_tag(tag).and_then(|held| held.temporal_count_at(crate::TimeUnit::Nanosecond))
        });
        // What a price moved from, and the two lanes a quote states.
        let prevpx = number(140);
        let (bidpx, bidqty) = (number(132), number(134));
        let (askpx, askqty) = (number(133), number(135));

        let event = &mut *self.event;
        // Assigned rather than filled: a write can change what the FIX
        // fields say, and a fact that no longer derives must stop being
        // answered. What a walk forced is kept by the early return above.
        event.set_px(price.unwrap_or(Decimal18::ZERO));
        event.set_qty(orderqty.or(quantity).unwrap_or(Decimal18::ZERO));
        event.set_lastpx(lastpx);
        event.set_lastqty(lastqty);
        event.set_avgpx(avgpx);
        event.set_cumqty(cumqty);
        event.set_leavesqty(leavesqty);
        event.set_prevpx(prevpx);
        event.set_bidpx(bidpx);
        event.set_bidqty(bidqty);
        event.set_askpx(askpx);
        event.set_askqty(askqty);
        event.set_expirunix(expirunix);
        event.set_tradable(tradable);
        event.set_side(side.unwrap_or_else(Side::unknown));
        event.set_currency(currency.unwrap_or_else(Currency::none));
        event.set_unit(unit.unwrap_or_default());
        event.set_tif(tif);
        event.set_symbolticker(symbolticker);
        event.set_cficode(cficode);
        event.set_isincode(isincode);
        event.set_cusipcode(cusipcode);
        event.set_sedolcode(sedolcode);
        event.set_bloombergcode(bloombergcode);
        event.set_miccode(miccode);
        event.set_state(state.unwrap_or_else(State::unknown));
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
    fn identifier<T>(&self, sources: &[&str], read: impl Fn(&str) -> Option<T>) -> Option<T> {
        let word = |value: Option<Scalar>| {
            value
                .and_then(|held| held.as_str().map(str::trim).map(SmolStr::new))
                .filter(|held| !held.is_empty())
        };
        let names = |held: &str| sources.contains(&held);
        let primary = word(self.get_by_tag(22))
            .filter(|held| names(held))
            .and_then(|_| word(self.get_by_tag(48)));
        if let Some(held) = primary.as_deref().and_then(&read) {
            return Some(held);
        }
        // A group occurrence is positional - the names live on the field
        // and never in the value - so the two members are found once and
        // every occurrence is read by those positions.
        let at = self.field.index_of("secaltidgrp")?;
        let column = self.field.fields().get(at)?;
        let DataType::Sequence(sequence) = column.dtype() else {
            return None;
        };
        let item = sequence.item();
        let (identifier, source) = (
            item.index_of("securityaltid")?,
            item.index_of("securityaltidsource")?,
        );
        let group = self.value.as_sequence()?.get(at)?;
        group
            .as_sequence()?
            .iter()
            .filter_map(|occurrence| {
                let held = occurrence.as_sequence()?;
                held.get(source)
                    .and_then(Scalar::as_str)
                    .filter(|held| names(held.trim()))?;
                word(held.get(identifier).cloned())
            })
            .find_map(|held| read(&held))
    }

    /// The code the message digests to: the event's own chain facts through
    /// [`Event::digest_event`], then the frame and the named content behind
    /// them - every header and trailer fact, every lifted fact and every
    /// row child that holds a value, by name - and never the capture,
    /// because where a line was read from is a fact about the capture and
    /// not about the message.
    ///
    /// The market is not here, and that is the point: every market fact the
    /// event answers is derived from a FIX field the content already
    /// digests, so feeding it would digest one statement twice, and a walk
    /// that folded a fact forward would move the code of a message whose
    /// line never named it.
    fn currhashcode(&self) -> u64 {
        let mut state = self.event.digest_event();
        let mut cells: Vec<(&str, Scalar)> = Vec::with_capacity(self.field.fields().len() + 8);
        if let Some(text) = self.text.as_deref() {
            cells.push(("text", Scalar::from(text)));
        }
        for (key, value) in &self.metadata {
            cells.push((key.as_str(), Scalar::from(value.as_str())));
        }
        for (tag, name) in [
            (8, "beginstring"),
            (35, "msgtype"),
            (49, "sendercompid"),
            (56, "targetcompid"),
            (34, "msgseqnum"),
            (43, "possdupflag"),
        ] {
            if let Some(fact) = self.header.fact(tag) {
                cells.push((name, fact));
            }
        }
        cells.sort_by(|left, right| left.0.cmp(right.0));
        let borrowed: Vec<(&str, &Scalar)> =
            cells.iter().map(|(name, value)| (*name, value)).collect();
        xxhash::write_named_bytes(&mut state, borrowed.into_iter(), 0);
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

    /// The event this message is: every fact the three graph traits answer,
    /// held as fields.
    #[must_use]
    pub const fn event(&self) -> &MarketEventData {
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
    /// [`MarketElement`](crate::graph::MarketElement) getters' to answer -
    /// `get_px` reads this ladder and the row - and a fact answered there
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
    /// instance, all read off the line's own bytes.
    ///
    /// Not where the line was read from and not when it was recorded: those
    /// are the reader's statements, and they are [the capture's own
    /// columns](Self::from_row) rather than facts of a message.
    #[must_use]
    pub const fn capture(&self) -> &FixCapture {
        &self.capture
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
        self.entries.get_or_init(|| self.derive_entries())
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
        entries_of(self.field.fields(), values)
    }

    /// Every entry the wire carries, in wire order: the standard header,
    /// then the FIX fields the message lifted, then the row, then the
    /// standard trailer.
    ///
    /// The frame's own bands are the two the wire moves out of tag order,
    /// which is why the header leads and the trailer closes whatever the
    /// body's tags are. Between them stands only what the message *stated*:
    /// a fact the event derived - the price it is about, the state it
    /// reached, a lane it never quoted - is answered by the traits and
    /// emitted nowhere, because a re-emission says what was read.
    fn wire_entries(&self) -> Vec<FixEntry> {
        let mut entries = Vec::with_capacity(self.field.fields().len() + 22);
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
                Some(field) => wire_text_under(field, &fact),
                None => wire_text(&fact),
            };
            if let Some(text) = text {
                entries.push(FixEntry::new(tag, name_of(tag), Some(text)));
            }
        };
        for tag in identity::WIRE_HEADER_TAGS
            .into_iter()
            .chain(identity::LIFTED_TAGS)
        {
            emit(&mut entries, tag);
        }
        entries.extend_from_slice(self.entries());
        for tag in identity::WIRE_TRAILER_TAGS {
            emit(&mut entries, tag);
        }
        entries
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
    /// unchanged. Every write settles the identity again.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use yggdryl::graph::MarketElement;
    /// use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, StructureType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut clordid = DataType::utf8().nullable_field("clordid");
    /// clordid.as_fix_mut().set_tag(11)?;
    /// let mut symbol = DataType::utf8().nullable_field("symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let registry = Arc::new(FixRegistry::from_fields([clordid, symbol])?);
    ///
    /// let root = DataType::from(StructureType::from_fields([DataType::utf8().required_field("symbol")])?)
    ///     .required_field("D");
    /// let value = Scalar::from_record([("symbol", Scalar::from("AAPL"))])?;
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
    /// assert_eq!(msg.get_px().to_string(), "82.5");
    /// assert_eq!(msg.get_qty().to_string(), "100");
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

    /// Writes every value the field it reaches can hold, with one rebuild,
    /// answering how many landed.
    ///
    /// The lenient twin of [`Self::set_many`], for a pass whose answers are
    /// best effort: a value the target refuses - an identifier whose check
    /// digit does not close, a spelling its code set does not read - is
    /// silence rather than a refusal, and every other value lands as
    /// `set_many` lands it. Nothing else is lenient: the rebuild's refusal,
    /// which no single value causes, is still returned and leaves the
    /// message unchanged.
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
            self.land(typed, writes)?;
        }
        Ok(landed)
    }

    /// Lands staged writes: the typed facts on their holders, the row
    /// writes in one rebuild, then the identity settled once.
    fn land(&mut self, typed: Vec<(i32, Scalar)>, writes: Vec<Write>) -> Result<()> {
        if typed.is_empty() && writes.is_empty() {
            return Ok(());
        }
        if !writes.is_empty() {
            self.write_all(writes)?;
        }
        for (tag, value) in typed {
            self.record(tag, &value);
        }
        self.settle();
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
    /// use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, StructureType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut symbol = DataType::utf8().nullable_field("symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let registry = Arc::new(FixRegistry::from_fields([symbol.clone()])?);
    /// let root = DataType::from(StructureType::from_fields([symbol, DataType::utf8().nullable_field("9999")])?)
    ///     .required_field("D");
    /// let value = Scalar::from_record([
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
        let dtype = DataType::from(StructureType::from_fields(members)?);
        let field = self.rerooted(dtype);
        let plan = super::schema::column_plan_of(&field, &self.registry)?;
        self.field = field;
        self.value = Scalar::from_sequence(values);
        self.tags = tag_positions(&plan);
        self.groups = group_positions(&self.field);
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
        emit_bytes(&self.wire_entries(), separator, &mut bytes);
        bytes
    }

    /// The same, as text.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when a value holds a control byte.
    pub fn into_text(&self, separator: char) -> Result<String> {
        let mut text = String::new();
        emit_text(&self.wire_entries(), separator, &mut text)?;
        Ok(text)
    }

    /// The deterministic digest of what the wire carries: every entry of
    /// [`Self::into_bytes`], pre-order, so two messages that re-emit alike
    /// digest alike whatever separator either was read with.
    #[must_use]
    pub fn digest(&self) -> u128 {
        super::digest::digest_of(&self.wire_entries())
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
        self.value.get(self.field.index_of(known.name())?).cloned()
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
        self.value.get(self.reached_by_tag(tag)?).cloned()
    }

    /// The value a tag names by the index alone: a typed fact, else the
    /// child declaring the tag, and never the two fallbacks of
    /// [`Self::get_by_tag`], which end in a name table built on the first
    /// miss. The [enriching pass](super::enrich) gathers its working row
    /// through this, a column per tag it reads.
    pub(super) fn indexed_by_tag(&self, tag: i32) -> Option<Scalar> {
        if identity::is_typed_tag(tag) {
            return self.typed_fact(tag);
        }
        self.value.get(self.index_of_tag(tag)?).cloned()
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
            let mut named = HashMap::new();
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
            .cloned()
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
    /// the List a repeating group is, which is what reaching a member
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
                    self.value.get(index)?.clone(),
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
                .filter(|group| matches!(group.dtype(), DataType::Mapping(_)))
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
    /// occupancy of the List a repeating group is.
    fn descend(
        &self,
        field: &Field,
        value: &Scalar,
        segment: &FieldSegment,
    ) -> Option<(Field, Scalar)> {
        match field.dtype() {
            DataType::Structure(_) => {
                let index = self.segment_index(field, segment)?;
                Some((
                    field.fields().get(index)?.clone(),
                    value.get(index)?.clone(),
                ))
            }
            DataType::Sequence(SequenceType::List(item))
            | DataType::Sequence(SequenceType::LargeList(item))
            | DataType::Sequence(SequenceType::FixedSizeList(item, _))
            | DataType::Sequence(SequenceType::ListView(item))
            | DataType::Sequence(SequenceType::LargeListView(item)) => {
                let FieldSegment::Index(position) = segment else {
                    return None;
                };
                let held = value.as_sequence()?;
                let at = if *position < 0 {
                    held.len().checked_sub(position.unsigned_abs() as usize)?
                } else {
                    usize::try_from(*position).ok()?
                };
                Some((item.as_ref().clone(), value.get(at)?.clone()))
            }
            DataType::Mapping(map) => {
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

/// Feeds one level of the entry tree to a digest, in the order it stands:
/// each entry's name, the value it states, and the entries under it.
///
/// Pre-order and framed by what each entry holds, so an entry heading
/// others is never the same as one stating their text: an entry that
/// states nothing feeds its name and no value, and its children follow.
fn feed_entries(state: &mut crate::xxhash::Xxh3, entries: &[FixEntry]) {
    for entry in entries {
        state.write(entry.name().as_bytes());
        if let Some(value) = entry.value() {
            state.write(value.as_bytes());
        }
        state.write_usize(entry.entries().len());
        feed_entries(state, entry.entries());
    }
}

/// One level of the row as the entries it states, in its order: every
/// child through [`entry_of`], except the counter scalar beside the group
/// it counts - at the root, in a component, in an occurrence alike - since
/// a group's count is the group entry's own value and the counter child
/// states nothing the entries do not already.
fn entries_of(fields: &[Field], values: &[Scalar]) -> Vec<FixEntry> {
    let counters: Vec<i32> = fields
        .iter()
        .filter(|child| child.dtype().is_nested())
        .filter_map(|child| child.as_fix().counter().ok().flatten())
        .collect();
    let counted = |child: &Field| {
        !child.dtype().is_nested()
            && child
                .as_fix()
                .tag()
                .ok()
                .flatten()
                .is_some_and(|tag| counters.contains(&tag))
    };
    fields
        .iter()
        .zip(values)
        .filter(|(child, _)| !counted(child))
        .filter_map(|(child, value)| entry_of(child, value))
        .collect()
}

/// One row child as the entry it is: a scalar as one stated entry, a
/// repeating group as its counter entry with an entry per occurrence and
/// the occurrence's members under each, a component as an entry heading
/// its members; nothing for a child stating null.
fn entry_of(field: &Field, value: &Scalar) -> Option<FixEntry> {
    if value.is_null() {
        return None;
    }
    let tag = field.as_fix().tag().ok().flatten().unwrap_or(0);
    let counter = field.as_fix().counter().ok().flatten();
    match field.dtype() {
        DataType::Sequence(SequenceType::List(item))
        | DataType::Sequence(SequenceType::LargeList(item)) => {
            let occurrences = value.as_sequence()?;
            let nested: Vec<FixEntry> = occurrences
                .iter()
                .filter_map(|occurrence| match item.dtype() {
                    DataType::Structure(_) => {
                        let members = entries_of(item.fields(), occurrence.as_sequence()?);
                        let own = item.as_fix().tag().ok().flatten().unwrap_or(0);
                        Some(FixEntry::new(own, item.name(), None).with_entries(members))
                    }
                    _ => entry_of(item, occurrence),
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
        DataType::Structure(_) => {
            let members = entries_of(field.fields(), value.as_sequence()?);
            Some(FixEntry::new(tag, field.name(), None).with_entries(members))
        }
        DataType::Mapping(_) => {
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
            wire_text_under(field, value),
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

impl Clone for FixMsg {
    /// The message, without its caches: the name table and the entries are
    /// derived from the row, rebuilt by the clone on its own first ask
    /// rather than copied.
    fn clone(&self) -> Self {
        Self {
            registry: Arc::clone(&self.registry),
            event: self.event.clone(),
            header: self.header.clone(),
            capture: self.capture.clone(),
            lifted: self.lifted.clone(),
            forced: self.forced,
            text: self.text.clone(),
            metadata: self.metadata.clone(),
            tags: self.tags.clone(),
            named: OnceLock::new(),
            groups: self.groups.clone(),
            field: self.field.clone(),
            value: self.value.clone(),
            entries: OnceLock::new(),
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

    /// A message's order is its instant.
    fn is_after(&self, other: &Self) -> bool {
        self.event.is_after(&other.event)
    }

    /// The identity settled again from what the message now states.
    fn finalize(&mut self) {
        self.settle();
    }

    /// The timed reading, and the predecessor adopted as a parent: a
    /// message descends from the one it follows.
    fn with_previous(self, previous: &Self) -> Option<Self> {
        let parent = previous.get_curruuid();
        let adopted = !self.get_parentuuids().contains(&parent);
        let mut this = self.following_market(previous)?;
        if adopted {
            let mut parents = this.get_parentuuids().to_vec();
            parents.push(parent);
            this.set_parentuuids(parents);
            this.finalize();
        }
        Some(this)
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        self.merging_market_event(other)
    }
}

impl Event for FixMsg {
    /// The timed restatement, and then the market's: a message logged at a
    /// second hop takes the live message's place in its chain - the
    /// predecessor, the position, the snapshot and the step before it - and
    /// what that chain is about where this reading stated none of it.
    fn restating(self, live: &Self) -> Self {
        crate::graph::element::restating_market(self, live)
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

    fn get_expirunix(&self) -> Option<i64> {
        self.event.get_expirunix()
    }

    fn set_expirunix(&mut self, unix: Option<i64>) {
        self.forced = true;
        self.event.set_expirunix(unix);
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

impl MarketElement for FixMsg {
    fn get_px(&self) -> Decimal18 {
        self.event.get_px()
    }

    fn set_px(&mut self, px: Decimal18) {
        self.forced = true;
        self.event.set_px(px);
    }

    fn get_currency(&self) -> &Currency {
        self.event.get_currency()
    }

    fn set_currency(&mut self, currency: Currency) {
        self.forced = true;
        self.event.set_currency(currency);
    }

    fn get_qty(&self) -> Decimal18 {
        self.event.get_qty()
    }

    fn set_qty(&mut self, qty: Decimal18) {
        self.forced = true;
        self.event.set_qty(qty);
    }

    fn get_unit(&self) -> &str {
        self.event.get_unit()
    }

    fn set_unit(&mut self, unit: String) {
        self.forced = true;
        self.event.set_unit(unit);
    }

    fn get_side(&self) -> &Side {
        self.event.get_side()
    }

    fn set_side(&mut self, side: Side) {
        self.forced = true;
        self.event.set_side(side);
    }

    fn get_isincode(&self) -> Option<&Isin> {
        self.event.get_isincode()
    }

    fn set_isincode(&mut self, isincode: Option<Isin>) {
        self.forced = true;
        self.event.set_isincode(isincode);
    }

    fn get_cusipcode(&self) -> Option<&Cusip> {
        self.event.get_cusipcode()
    }

    fn set_cusipcode(&mut self, cusipcode: Option<Cusip>) {
        self.forced = true;
        self.event.set_cusipcode(cusipcode);
    }

    fn get_sedolcode(&self) -> Option<&Sedol> {
        self.event.get_sedolcode()
    }

    fn set_sedolcode(&mut self, sedolcode: Option<Sedol>) {
        self.forced = true;
        self.event.set_sedolcode(sedolcode);
    }

    fn get_bloombergcode(&self) -> Option<&Bloomberg> {
        self.event.get_bloombergcode()
    }

    fn set_bloombergcode(&mut self, bloombergcode: Option<Bloomberg>) {
        self.forced = true;
        self.event.set_bloombergcode(bloombergcode);
    }

    fn get_cficode(&self) -> Option<&Cfi> {
        self.event.get_cficode()
    }

    fn set_cficode(&mut self, cficode: Option<Cfi>) {
        self.forced = true;
        self.event.set_cficode(cficode);
    }

    fn get_miccode(&self) -> Option<&Mic> {
        self.event.get_miccode()
    }

    fn set_miccode(&mut self, miccode: Option<Mic>) {
        self.forced = true;
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

    fn get_tif(&self) -> Option<&str> {
        self.event.get_tif()
    }

    fn set_tif(&mut self, tif: Option<String>) {
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

    fn get_symbolticker(&self) -> Option<&str> {
        self.event.get_symbolticker()
    }

    fn set_symbolticker(&mut self, ticker: Option<String>) {
        self.forced = true;
        self.event.set_symbolticker(ticker);
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

    fn get_bidpx(&self) -> Option<Decimal18> {
        self.event.get_bidpx()
    }

    fn set_bidpx(&mut self, px: Option<Decimal18>) {
        self.forced = true;
        self.event.set_bidpx(px);
    }

    fn get_bidcurrency(&self) -> Option<&Currency> {
        self.event.get_bidcurrency()
    }

    fn set_bidcurrency(&mut self, currency: Option<Currency>) {
        self.forced = true;
        self.event.set_bidcurrency(currency);
    }

    fn get_bidqty(&self) -> Option<Decimal18> {
        self.event.get_bidqty()
    }

    fn set_bidqty(&mut self, qty: Option<Decimal18>) {
        self.forced = true;
        self.event.set_bidqty(qty);
    }

    fn get_bidunit(&self) -> Option<&str> {
        self.event.get_bidunit()
    }

    fn set_bidunit(&mut self, unit: Option<String>) {
        self.forced = true;
        self.event.set_bidunit(unit);
    }

    fn get_askpx(&self) -> Option<Decimal18> {
        self.event.get_askpx()
    }

    fn set_askpx(&mut self, px: Option<Decimal18>) {
        self.forced = true;
        self.event.set_askpx(px);
    }

    fn get_askcurrency(&self) -> Option<&Currency> {
        self.event.get_askcurrency()
    }

    fn set_askcurrency(&mut self, currency: Option<Currency>) {
        self.forced = true;
        self.event.set_askcurrency(currency);
    }

    fn get_askqty(&self) -> Option<Decimal18> {
        self.event.get_askqty()
    }

    fn set_askqty(&mut self, qty: Option<Decimal18>) {
        self.forced = true;
        self.event.set_askqty(qty);
    }

    fn get_askunit(&self) -> Option<&str> {
        self.event.get_askunit()
    }

    fn set_askunit(&mut self, unit: Option<String>) {
        self.forced = true;
        self.event.set_askunit(unit);
    }
}
