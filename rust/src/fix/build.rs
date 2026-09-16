//! One builder, from key/value pairs to a typed message.
//!
//! Every reader splits its own dialect and hands the pairs here, so there is
//! one nesting builder, one fold, one code translation and one value contract
//! under all of them. Nothing else in the FIX layer turns bytes into fields.
//!
//! # What a key may be
//!
//! | key | means |
//! | --- | --- |
//! | `54`, `"54"` | a tag, through the strict parse that refuses `+35` and `3x` |
//! | `Side`, `side`, `" Side "`, `msg_type`, `Msg Type` | a name, trimmed and folded |
//! | `Instrument.Symbol` | a path |
//! | `PartyID[0]`, `PartyID[1]` | one field, two occurrences, in order |
//! | `Parties[0].PartyID` | which group, which occurrence, which member |
//! | `NoPartyIDs[0].PartyID` | a wire counter resolving the same group |
//! | `VenueOwnThing` | an unknown name, kept |
//! | `#NoPartyIDs[0]` | a spelling a reader left marked, at the top of a row: one child under its own name, its packed value its value |
//! | `""`, `"   "` | dropped |
//!
//! # What it refuses to lose
//!
//! An unknown tag is kept as nullable `utf8` under its decimal spelling, and
//! an unknown name under its own. Every venue sends fields no dictionary has,
//! and dropping them loses data. A value that will not type is null rather
//! than a failure, with the raw text still in the entries and the refusal
//! readable through the message's anomalies: a null nobody can explain is
//! worse than the value that actually arrived. An unresolved numeric or named
//! key records tag zero; its parsed number is not a dictionary identity.

use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::entry::FixEntry;
use super::group_plan::GroupPlan;
use super::memo::{Lookup, Memo};
use super::{FixRegistry, STANDARD_HEADER_TAGS, STANDARD_TRAILER_TAGS, occurrence_name};
use crate::media::text::TextBytes;
use crate::types::{Code, State};
use crate::{DataType, Error, Field, Result, Scalar, Version};

/// What a key resolved to, before any field is built.
enum Located<'key> {
    /// One flat child, by tag or by name.
    Flat,
    /// One occurrence of a repeating group's member.
    Grouped {
        group: &'key str,
        occurrence: usize,
        member: &'key str,
    },
    /// One occurrence of a repeated flat field.
    Repeated { name: &'key str, occurrence: usize },
}

/// What the dictionary holds under one key: the field it names, with the
/// tag that field carries, and the repeating group it heads.
struct Known<'registry> {
    field: Option<(&'registry Field, i32)>,
    group: Option<&'registry Field>,
}

/// Provenance for one composable root child, bounded by root field count.
/// No value is copied: critical text remains in its one source cell until
/// the winner is typed, and invalid UTF-8 is not repaired into an identity.
struct Composed {
    source: SmolStr,
    target: Field,
    tag: i32,
    invalid_utf8: bool,
}

/// Raw consensus for one target.
struct Composition<'source> {
    source: &'source Composed,
    write: super::msg::Write,
    disagreed: bool,
    invalid_utf8: bool,
}

impl Known<'_> {
    /// A key the dictionary holds nothing under.
    const NONE: Self = Self {
        field: None,
        group: None,
    };
}

/// The level a key the dictionary does not name resolves against.
///
/// The message's own children, indexed by folded name once at registration,
/// because a wide message has three hundred and a bridge row asks for dozens
/// of keys it does not know; or one occurrence's declared members, few
/// enough to walk.
enum Scope<'level> {
    Message(Option<&'level super::MsgType>),
    Members(&'level [Field]),
}

impl<'level> Scope<'level> {
    /// The child of this level that `key` names, and the tag it carries.
    fn child(&self, key: &str) -> Option<(&'level Field, i32)> {
        match self {
            Self::Message(message) => message.and_then(|message| message.get_child_by_name(key)),
            Self::Members(fields) => in_scope(fields, key),
        }
    }
}

/// One key split into what it addresses.
struct Key<'key> {
    text: &'key str,
    located: Located<'key>,
}

impl<'key> Key<'key> {
    /// Reads the location a rendered key carries.
    ///
    /// The key states group, occurrence and member itself, so no message
    /// grammar is needed to build real nesting from it. Inferring a group
    /// from bare repetition alone is a different problem and needs one.
    fn parse(text: &'key str) -> Self {
        let located = match text.split_once('[') {
            Some((head, rest)) => match rest.split_once(']') {
                Some((index, tail)) => index
                    .parse::<usize>()
                    .ok()
                    .map(|occurrence| match tail.strip_prefix('.') {
                        Some(member) if !member.is_empty() => Located::Grouped {
                            group: head,
                            occurrence,
                            member,
                        },
                        _ => Located::Repeated {
                            name: head,
                            occurrence,
                        },
                    })
                    .unwrap_or(Located::Flat),
                None => Located::Flat,
            },
            None => Located::Flat,
        };
        Self { text, located }
    }
}

/// One child under construction, with every occurrence it has been given.
struct Slot {
    field: Field,
    tag: i32,
    /// Whether the field is the dictionary's own, and so carries the tag it
    /// resolved to.
    ///
    /// A key no dictionary explains keeps its spelling and no `fix:tag`, so
    /// the message's tag index leaves it out - which is what a reader of the
    /// finished field would find, read once here instead of once per child.
    known: bool,
    values: Vec<Scalar>,
    /// Whether this slot is a repeating group, whatever it has been given.
    ///
    /// A group whose counter arrived and whose members did not is still a
    /// group, and the empty list is what says the count was not met.
    group: bool,
    /// The group members, when this slot is a repeating group.
    ///
    /// Each member keeps the field it resolved to rather than only its name.
    /// The field is what carries the member's tag and its datatype, and
    /// without them nothing inside a group can be addressed - which is the
    /// whole of how a party is found by its role. A member that is itself a
    /// repeating group is a slot of its own, so a group nests to any depth
    /// a wire or a bridge packs it.
    occurrences: Vec<Vec<Member>>,
}

/// One member of one occurrence: a value, or a group nested inside it.
enum Member {
    Value(Field, Scalar),
    Group(Slot),
}

impl Slot {
    /// A group slot holding nothing yet.
    fn group(field: Field, tag: i32, known: bool) -> Self {
        Self {
            field,
            tag,
            known,
            values: Vec::new(),
            group: true,
            occurrences: Vec::new(),
        }
    }

    /// The nested group one occurrence holds under `field`, created on
    /// first use, with the occurrence itself created on the way.
    fn nested(&mut self, occurrence: usize, field: Field, tag: i32, known: bool) -> &mut Slot {
        while self.occurrences.len() <= occurrence {
            self.occurrences.push(Vec::new());
        }
        let members = &mut self.occurrences[occurrence];
        let at = match members.iter().position(
            |member| matches!(member, Member::Group(held) if held.field.name() == field.name()),
        ) {
            Some(at) => at,
            None => {
                members.push(Member::Group(Slot::group(field, tag, known)));
                members.len() - 1
            }
        };
        match &mut members[at] {
            Member::Group(slot) => slot,
            Member::Value(..) => unreachable!("found by its group arm"),
        }
    }
}

/// One pair the build folds in: what fills the row, and what the line said.
///
/// The two are the same pair almost everywhere, and differ where a reading
/// unpacked one: a bridge packs a whole occurrence into one value, and the
/// members read out of it fill `NOPARTYIDS[0].PARTYID` and its siblings while
/// the arrival record keeps the single pair the bridge wrote. Every range here
/// belongs to the one page the line was read into, so a pair costs two
/// reference counts and no byte.
#[derive(Clone)]
pub(super) struct FixPair {
    key: PairKey,
    value: TextBytes,
    arrival: PairArrival,
}

/// The key a pair fills its row under.
///
/// Two shapes because there are two kinds of key and only one of them is a
/// fact about a line. A pair the line wrote is keyed by the range that wrote
/// it, which costs a reference count and no byte. A pair a *reading* built is
/// keyed by a path that appears nowhere in the line - `NOPARTYIDS[0].PARTYID`
/// out of one packed value - so it is bytes somebody has to hold, and holding
/// them as bytes is one allocation where wrapping them in a page of their own
/// would be that allocation, a copy of it and a counted page nothing ever
/// counts: a rendered key is read as `&[u8]` by the build and by nothing else.
#[derive(Clone)]
enum PairKey {
    /// A range of the line this pair arrived on.
    Ranged(TextBytes),
    /// A path a reading rendered, which names no range of any line.
    Rendered(Vec<u8>),
}

/// What one pair records.
#[derive(Clone)]
pub(super) enum PairArrival {
    /// The line wrote this pair: it records itself, under the tag its key
    /// named.
    Own,
    /// The line wrote `key=value` and this pair is the first member read out
    /// of it: the packed pair is the record, and its key names no field.
    Packed(TextBytes, TextBytes),
    /// Another member of a packed pair recorded beside it: nothing to record.
    Read,
    /// The document spelled the key so, and the row fills under the
    /// dictionary's own name for it: the record keeps the spelling, under
    /// the tag the name resolved to.
    Spelled(TextBytes),
}

impl FixPair {
    /// One pair the line wrote.
    pub(super) const fn own(key: TextBytes, value: TextBytes) -> Self {
        Self {
            key: PairKey::Ranged(key),
            value,
            arrival: PairArrival::Own,
        }
    }

    /// One pair a reading built beside an arrival already recorded.
    ///
    /// The key is the path the reading rendered, owned as the bytes it is.
    pub(super) const fn read(key: Vec<u8>, value: TextBytes) -> Self {
        Self {
            key: PairKey::Rendered(key),
            value,
            arrival: PairArrival::Read,
        }
    }

    /// One pair a document wrote under its own spelling of a field the
    /// dictionary names otherwise.
    ///
    /// `name` is what the row fills under and `key` is what arrived: the
    /// entry keeps the arrival, and the child takes the dictionary's name.
    pub(super) const fn spelled(name: Vec<u8>, key: TextBytes, value: TextBytes) -> Self {
        Self {
            key: PairKey::Rendered(name),
            value,
            arrival: PairArrival::Spelled(key),
        }
    }

    /// Makes this pair the one that records the pair a line wrote and this
    /// one reads.
    pub(super) fn reads(&mut self, key: TextBytes, value: TextBytes) {
        self.arrival = PairArrival::Packed(key, value);
    }

    /// The key the row fills under.
    pub(super) fn key(&self) -> &[u8] {
        match &self.key {
            PairKey::Ranged(key) => key.as_bytes(),
            PairKey::Rendered(key) => key,
        }
    }

    /// The value, exactly as the line holds it.
    pub(super) fn value(&self) -> &[u8] {
        self.value.as_bytes()
    }

    /// What this pair records, where it records one.
    fn arrived(&self) -> Option<Arrived> {
        match (&self.arrival, &self.key) {
            (PairArrival::Own, PairKey::Ranged(key)) => Some(Arrived {
                key: key.clone(),
                value: self.value.clone(),
                named: true,
            }),
            (PairArrival::Packed(key, value), _) => Some(Arrived {
                key: key.clone(),
                value: value.clone(),
                named: false,
            }),
            (PairArrival::Spelled(key), _) => Some(Arrived {
                key: key.clone(),
                value: self.value.clone(),
                named: true,
            }),
            // A rendered key is a reading and never an arrival, which is the
            // whole of what `read` records.
            (PairArrival::Own | PairArrival::Read, _) => None,
        }
    }
}

/// The name a row states its own version under.
///
/// Read by both readers of a row and spelled once: the batch reader looks for
/// a column of this name, the line reader for a row-header capture of it. A
/// row states a fact about its line either way, and the two doors must not
/// disagree about what it is called.
pub(super) const BEGINSTRING_COLUMN: &str = "beginstring";
/// The name a row states the direction its line moved under: tag 385's own
/// (decision 14), so a stated `msgdirection` column is read as the direction
/// and never as a fill.
pub(super) const DIRECTION_COLUMN: &str = super::MSGDIRECTION_TAG_NAME.1;

/// The version a `beginstring` states, as `BeginString` spells it.
///
/// `FIX.4.2` and `4.2` are one version; `FIXT.1.1` is none, because the
/// session layer says nothing about the application version, and neither
/// is anything else that is not a version.
pub(super) fn version_of(beginstring: &str) -> Option<Version> {
    beginstring
        .strip_prefix("FIX.")
        .unwrap_or(beginstring)
        .parse::<Version>()
        .ok()
}

/// What a row states beside its payload, applied when its message is built.
///
/// A version and fills, all the caller speaking per row. The
/// version is what the row's `beginstring` resolved to, and it outranks the
/// codec's own pin because a row is the more specific statement; it arrives
/// resolved, so the build reads under it and parses no name per line.
/// Each fill lands on the field its name reaches - a capture named `sessionId`
/// fills `sessionid`, one named `seqNum` fills `MsgSeqNum` - unless the
/// message stated that field itself, because a stated value is never
/// overridden. None of them touches the entries: the entries are what arrived
/// on the line, and these arrived on the row.
#[derive(Clone, Copy, Default)]
pub(super) struct RowExtras<'row> {
    /// The version the row is read at, where it stated one.
    pub(super) version: Option<Version>,
    /// The row's own columns, resolved to the fields they fill.
    pub(super) fills: &'row [Fill<'row>],
    /// The direction the row stated, as a code of tag 385's set: it outranks
    /// the reading of the line (decision 14).
    pub(super) direction: Option<&'row str>,
    /// The code a line stating no direction takes - the codec's pin on the
    /// batch door - and nothing on the line door, where silence is silence.
    pub(super) direction_pin: Option<&'row str>,
    /// The message type the reader supplies where the payload states none of
    /// its own: a plugin configuration is `UCFG` because the crate says so,
    /// never because the document did (decision 19). A `35=` or `MSGTYPE=`
    /// on the wire outranks it, as a stated value always does.
    pub(super) msgtype: Option<&'row (&'static str, &'static str)>,
}

/// What a row stated, owned, for the messages it answers for.
///
/// [`RowExtras`] borrows the row it was read from, and one row answers for
/// as many messages as it carries - every frame of a line, every plugin of a
/// configuration document - so what the row stated is retained once here and
/// applied to each of them rather than borrowed across an expansion the
/// caller drives. One copy per row that expands, never per line: a row of
/// one message borrows and retains nothing.
#[derive(Clone, Debug)]
pub(super) struct RowStamp {
    /// The version the row is read at, where it stated one.
    version: Option<Version>,
    /// The row's own columns, beside the field and tag each fills.
    fills: Vec<(Field, i32, Scalar)>,
    /// The direction resolved for the row, a code of tag 385's set.
    direction: Option<SmolStr>,
}

impl RowStamp {
    /// What a row stated, retained; nothing at all where it stated nothing.
    pub(super) fn retained(extras: RowExtras<'_>) -> Option<Arc<Self>> {
        if extras.version.is_none() && extras.fills.is_empty() && extras.direction.is_none() {
            return None;
        }
        Some(Arc::new(Self {
            version: extras.version,
            fills: extras
                .fills
                .iter()
                .map(|fill| (fill.field.clone(), fill.tag, fill.value.clone()))
                .collect(),
            direction: extras.direction.map(SmolStr::new),
        }))
    }

    /// The row's columns as the builder takes them, borrowed from the stamp.
    pub(super) fn fills(&self) -> Vec<Fill<'_>> {
        self.fills
            .iter()
            .map(|(field, tag, value)| Fill {
                field,
                tag: *tag,
                value,
            })
            .collect()
    }

    /// What the row stated, over the fills [`Self::fills`] answered.
    pub(super) fn extras<'row>(&'row self, fills: &'row [Fill<'row>]) -> RowExtras<'row> {
        RowExtras {
            version: self.version,
            fills,
            direction: self.direction.as_deref(),
            direction_pin: None,
            msgtype: None,
        }
    }

    /// What a stamp a row never took answers: the extras of a row that
    /// stated nothing.
    pub(super) fn held<'row>(
        stamp: Option<&'row Arc<Self>>,
        fills: &'row [Fill<'row>],
    ) -> RowExtras<'row> {
        stamp.map_or(RowExtras::NONE, |stamp| stamp.extras(fills))
    }
}

/// One of a row's own columns, resolved to the field it fills.
///
/// Resolved once per stream by the batch reader and once per record by the
/// record reader, never per row inside the builder: the dictionary probes a
/// name costs are paid where the column is decided, not where it is read.
#[derive(Clone, Copy)]
pub(super) struct Fill<'row> {
    /// The field, as a built child carries it: non-null.
    pub(super) field: &'row Field,
    /// The tag the field carries.
    pub(super) tag: i32,
    /// What the row stated.
    pub(super) value: &'row Scalar,
}

impl RowExtras<'static> {
    /// A row stating nothing beside its payload.
    pub(super) const NONE: Self = Self {
        version: None,
        fills: &[],
        direction: None,
        direction_pin: None,
        msgtype: None,
    };
}

/// The registry field one of a row's own columns fills, and its tag.
///
/// Two tiers, the second consulted only when the first missed: the
/// dictionary's one namespace - where the crate's own fields live, so a
/// column named `msgCtxId` means the capture's context - which is how every
/// key resolves; then the bridge's own spellings of standard fields, because
/// a bridge writes `seqNum` in its log where FIX says `MsgSeqNum`.
pub(super) fn fill_field<'registry>(
    registry: &'registry FixRegistry,
    key: &str,
) -> Option<(&'registry Field, i32)> {
    let key = key.trim();
    if key.is_empty() || super::field::parse_tag(key).is_some() {
        return None;
    }
    // The key arrived on a line, so the path it states is read here, at the
    // one boundary that has a string: everything below takes it resolved.
    let path = crate::FieldPath::from_str(key).ok();
    let named = path
        .as_ref()
        .and_then(|path| registry.get_field_by_path(path))
        .or_else(|| registry.get_field_by_name(key));
    let field = named?;
    let tag = field.as_fix().tag().ok().flatten()?;
    Some((field, tag))
}

/// The dictionary's one key resolver, shared by the builder and its span
/// projection. A nested descendant remains borrowed from its actual schema.
fn resolve<'registry>(
    registry: &'registry FixRegistry,
    key: &str,
) -> Option<(&'registry Field, i32)> {
    let field = if let Some(tag) = super::field::parse_tag(key) {
        registry.get_field_by_tag(tag)
    } else {
        let path = crate::FieldPath::from_str(key).ok()?;
        registry.get_field_by_path(&path)
    }?;
    Some((field, registry.identity_of(field).map_or(0, |(tag, _)| tag)))
}

/// Namespace targets and direct field identities share the existing bounded
/// key table. Dotted descendants themselves cannot be recovered by global id.
fn lookup(registry: &FixRegistry, memo: &Memo, key: &str) -> Lookup {
    memo.lookup(key, || {
        if let Some((_, last)) = key.rsplit_once('.') {
            Lookup {
                field: None,
                group: false,
                composed: registry
                    .get_field_by_name(last)
                    .and_then(|field| registry.identity_of(field)),
            }
        } else {
            Lookup {
                field: resolve(registry, key).and_then(|(field, _)| registry.identity_of(field)),
                group: registry
                    .get_definition(crate::FixCategory::Groups, key)
                    .is_some(),
                composed: None,
            }
        }
    })
}

/// Exact code bytes select the scanner's original span before value typing.
pub(super) fn preserves_value(registry: &FixRegistry, memo: &Memo, key: &[u8]) -> bool {
    let Ok(key) = std::str::from_utf8(key) else {
        return false;
    };
    let found = lookup(registry, memo, key.trim());
    found
        .field
        .or(found.composed)
        .is_some_and(|(tag, _)| tag == super::CODE_TAG_NAME.0)
}

/// The version a message is said to be read at when nothing decided one.
///
/// FIX 4.4, which is the version the standard header is ordered by here and
/// the one a bare capture that names none is most likely to be.
pub(super) fn default_version() -> Version {
    "4.4".parse().unwrap_or(Version::MIN)
}

/// Builds a message's field and value from pairs, with the entries beside it.
pub(super) struct Builder<'registry> {
    registry: &'registry FixRegistry,
    /// The message the row typed itself as, where the dictionary declares
    /// one: its own group plans answer before the registry's.
    message: Option<&'registry super::MsgType>,
    /// The `BeginString` child a message that stated none is given,
    /// resolved once per codec rather than once per line.
    beginstring: &'registry Field,
    /// What the run has already asked the dictionary, the codec's own.
    memo: &'registry Memo,
    /// The groups this line has addressed so far, by the spelling it
    /// addressed them with: a bridge names one group once per member it
    /// packs, and the group resolves once per line rather than once per
    /// member.
    groups: Vec<(SmolStr, Field, i32, bool)>,
    version: Option<Version>,
    slots: Vec<Slot>,
    /// Each slot's name digested, beside the slot.
    ///
    /// A key finds its slot by one integer compare per slot and one string
    /// compare on the hit, where comparing the names outright made a wide
    /// bridge row quadratic in its keys.
    hashes: Vec<u64>,
    entries: Vec<FixEntry>,
    /// The tag of every entry recorded so far, in arrival order.
    ///
    /// A member nests under the latest arrival of its counter, and finding
    /// that arrival walks the whole record - which is worth doing only where
    /// the counter arrived at all. One integer per entry answers that
    /// before the walk, and a document of fifty attributes under a group no
    /// counter introduced walks nothing.
    recorded: Vec<i32>,
    /// How many slots the line itself built, while a row nested inside one
    /// of its data fields is being read.
    ///
    /// A nested row is a reading of a value the line already recorded, so
    /// it records no entry and never overrides a child the line stated: a
    /// key it shares with the line is left to the line.
    outer: Option<usize>,
    /// The repeating groups a numeric frame has opened and not yet closed,
    /// outermost first.
    ///
    /// A numeric frame states no structure: a counter arrives, its members
    /// follow, and where one occurrence ends and the next begins is what
    /// the dictionary's declaration says - the first declared member opens
    /// an occurrence, a member already seen in the current one opens the
    /// next, and a tag the group does not declare closes it. Empty for a
    /// frame with no counter, which is most of them, so the fast path
    /// reads one empty vector.
    open: Vec<OpenGroup>,
    /// The type and the version the line's own frame stated, kept while a row
    /// nested inside one of its data fields is read against its own.
    framed: (Option<&'registry super::MsgType>, Option<Version>),
    /// What the pair being folded in records as the arrival, absent while the
    /// build is folding in a reading of a pair recorded beside it.
    ///
    /// Carried on the builder rather than threaded through every push, because
    /// the entry is written where the key finished resolving - five call sites
    /// down - and what it writes is the same two ranges whichever of them got
    /// there.
    arrival: Option<Arrived>,
    /// The first rejected root invariant, retained only until this build
    /// finishes. Best-effort fields never enter this bounded error slot.
    failure: Option<Error>,
    composed: Vec<Composed>,
}

/// The pair one fold records, as the line wrote it.
#[derive(Clone)]
struct Arrived {
    key: TextBytes,
    value: TextBytes,
    /// Whether the key names the field the fold is filling. A packed
    /// occurrence's key names an occurrence and no field, exactly as an
    /// indexed counter's does, so its entry carries no tag.
    named: bool,
}

/// The build-time arrival tag of a key no dictionary resolved.
///
/// The parsed number negated, or `0` where none parsed: members arriving under
/// an unresolved numeric counter still nest beneath it, while a negative tag
/// can never meet a resolved one. [`Builder::finish`] publishes every such
/// tag as `0`, the unresolved-arrival sentinel.
const fn unresolved(tag: i32) -> i32 {
    if tag > 0 { -tag } else { 0 }
}

/// One repeating group a numeric frame has opened and not yet closed.
struct OpenGroup {
    /// The group's own field name, as a located key spells it.
    name: SmolStr,
    /// The direct members the dictionary declares, the first their delimiter.
    members: Vec<i32>,
    /// The occurrence being filled, once a member arrived.
    occurrence: Option<usize>,
    /// The index the first occurrence takes: what the column already holds,
    /// so a counter the frame states twice at one level appends to the
    /// group rather than writing over what the first counter gathered.
    base: usize,
    /// The member tags the current occurrence holds.
    seen: Vec<i32>,
}

impl<'registry> Builder<'registry> {
    /// Opens a build against one dictionary and version.
    pub(super) fn new(
        registry: &'registry FixRegistry,
        message: Option<&'registry super::MsgType>,
        beginstring: &'registry Field,
        memo: &'registry Memo,
        version: Option<Version>,
        capacity: usize,
    ) -> Self {
        Self {
            registry,
            message,
            beginstring,
            memo,
            groups: Vec::new(),
            version,
            slots: Vec::with_capacity(capacity),
            hashes: Vec::with_capacity(capacity),
            entries: Vec::with_capacity(capacity),
            recorded: Vec::with_capacity(capacity),
            outer: None,
            open: Vec::new(),
            framed: (None, None),
            arrival: None,
            failure: None,
            composed: Vec::new(),
        }
    }

    /// Numeric counters open schema-scoped groups; indexed/name keys retain
    /// their explicit addressing. Only received pairs advance or allocate rows.
    pub(super) fn push_pairs(&mut self, pairs: &[FixPair], absent: impl Fn(&[u8], &[u8]) -> bool) {
        let mut cursor = 0;
        while let Some(pair) = pairs.get(cursor) {
            cursor += 1;
            let (key, value) = (pair.key(), pair.value());
            if absent(key, value) {
                continue;
            }
            let group = std::str::from_utf8(key)
                .ok()
                .and_then(super::field::parse_tag)
                .and_then(|tag| Some((tag, self.numeric_plan(tag)?)));
            self.arrival = pair.arrived();
            self.push(key, value);
            if let Some((tag, plan)) = group {
                let value = self.read_numeric_group(tag, plan, pairs, &mut cursor, &absent);
                // Not `known`: the group is addressed by the counter's tag
                // on the wire but does not carry it - the counter's own column
                // does. Indexing both under one tag makes `by_tag` answer with
                // whichever the binary search lands on.
                let slot = self.slot_for(plan.field().clone(), tag, false);
                slot.field = plan.field().clone();
                // A counter the frame states twice at one level appends to what
                // the first statement gathered rather than writing over it: a
                // dictionary that does not nest one group inside another reads
                // the inner counter twice at one level, and nothing it gathered
                // is lost for that.
                let mut rows: Vec<Scalar> = slot
                    .values
                    .pop()
                    .as_ref()
                    .and_then(Scalar::as_sequence)
                    .map_or_else(Vec::new, <[Scalar]>::to_vec);
                if let Some(read) = value.as_sequence() {
                    rows.extend(read.iter().cloned());
                }
                slot.values = vec![Scalar::from_sequence(rows)];
                slot.group = false;
                slot.occurrences.clear();
            }
        }
    }

    fn numeric_plan(&self, counter: i32) -> Option<&'registry GroupPlan> {
        match self
            .message
            .filter(|message| message.has_group_tag(counter))
        {
            Some(message) => message.get_group_plan_by_tag(counter),
            None => self.registry.get_group_plan_by_tag(counter),
        }
    }

    fn numeric_group(&self, counter: i32) -> Option<&'registry Field> {
        match self
            .message
            .filter(|message| message.has_group_tag(counter))
        {
            Some(message) => message.get_group_by_tag(counter),
            None => self.registry.get_group_by_tag(counter),
        }
    }

    /// Reads one numeric group's occurrences, recording each member under the
    /// counter pair that heads it.
    ///
    /// `counter` is that pair's tag: the entries are the arrival record, and a
    /// repeating group's members ride under the counter that introduced them,
    /// exactly as a bridge's indexed keys state them.
    fn read_numeric_group(
        &mut self,
        counter: i32,
        plan: &'registry GroupPlan,
        pairs: &[FixPair],
        cursor: &mut usize,
        absent: &impl Fn(&[u8], &[u8]) -> bool,
    ) -> Scalar {
        let mut rows = Vec::new();
        let mut current = None;
        while let Some(pair) = pairs.get(*cursor) {
            let raw = pair.value();
            if absent(pair.key(), raw) || raw.is_empty() {
                *cursor += 1;
                continue;
            }
            let Ok(key) = std::str::from_utf8(pair.key()) else {
                break;
            };
            let Some(tag) = super::field::parse_tag(key) else {
                break;
            };
            let Some(column) = plan.tag_index(tag) else {
                break;
            };
            if plan.delimiter() == Some(tag) {
                if let Some(values) = current.take() {
                    rows.push(plan.row(values));
                }
            }
            let values = current.get_or_insert_with(|| vec![Scalar::Null; plan.columns_len()]);
            let text = String::from_utf8_lossy(raw);
            values[column] = self.typed(plan.column(column), Some(plan.column(column)), raw, &text);
            self.arrival = pair.arrived();
            self.record_under(counter, tag);
            *cursor += 1;
            if let Some((column, nested)) = plan.nested(tag) {
                values[column] = self.read_numeric_group(tag, nested, pairs, cursor, absent);
            }
        }
        if let Some(values) = current {
            rows.push(plan.row(values));
        }
        Scalar::from_sequence(rows)
    }

    /// Opens the reading of a row nested inside one of the line's data
    /// fields: what follows fills the row and records nothing.
    ///
    /// The row is a message of its own type at its own version, so it is read
    /// against both rather than against the frame's. A bridge writes a whole
    /// trade capture into a `35=UL` frame's `XmlData`, and `UL` says nothing
    /// about the groups that row nests or the spellings its dialect gave two
    /// tags; the frame's `BeginString` is the envelope's version and says
    /// nothing about which FIX the row inside it was written to, which is
    /// routinely a later one than the session speaks. The line's own type and
    /// version are restored by [`Builder::end_nested`], so what the message
    /// says it is stays what the frame said.
    pub(super) fn begin_nested(
        &mut self,
        message: Option<&'registry super::MsgType>,
        version: Option<Version>,
    ) {
        self.outer = Some(self.slots.len());
        self.framed = (self.message, self.version);
        if message.is_some() {
            self.message = message;
        }
        if version.is_some() {
            self.version = version;
        }
    }

    /// Closes the nested reading.
    pub(super) fn end_nested(&mut self) {
        self.outer = None;
        (self.message, self.version) = self.framed;
    }

    /// Whether the line itself already built a child of this name, which a
    /// nested row then leaves alone.
    fn shadowed(&self, name: &str) -> bool {
        self.outer.is_some_and(|outer| {
            self.slots[..outer]
                .iter()
                .any(|slot| slot.field.name() == name)
        })
    }

    /// Folds one arriving pair in.
    ///
    /// The value is typed from the raw bytes, never from the entry's text:
    /// the lossy decode exists only to fill the entry, and typing through it
    /// would destroy a `data` field's bytes on the way in. The ordering is
    /// the whole rule.
    pub(super) fn push(&mut self, key: &[u8], value: &[u8]) {
        let key_text = String::from_utf8_lossy(key);
        let key_text = key_text.trim();
        if key_text.is_empty() {
            return;
        }
        let value_text = String::from_utf8_lossy(value);
        // A key arriving still marked `#` is one the reader left so - kept
        // whole beside a bare twin that took the structure, or a twice-marked
        // key's bare - and is one flat child under its own name, so
        // `#NOPARTYIDS[0]` never writes over the count `#NOPARTYIDS` stated
        // beside it. A member a rendered occurrence carries marked is a
        // member like any other, and nests as its key says.
        let located = if key_text.starts_with('#') {
            Key {
                text: key_text,
                located: Located::Flat,
            }
        } else {
            Key::parse(key_text)
        };
        match located.located {
            Located::Flat => self.push_flat(located.text, &value_text, value),
            Located::Repeated { name, occurrence } => {
                self.push_repeated(name, occurrence, &value_text, value);
            }
            Located::Grouped {
                group,
                occurrence,
                member,
            } if !value.is_empty() => {
                self.push_grouped(group, occurrence, member, &value_text, value);
            }
            Located::Grouped { .. } => {}
        }
    }

    /// Fills one field from a row's own column, where the message did not
    /// state it.
    ///
    /// Row only: no entry is recorded, because the entries are what arrived
    /// on the line and this arrived on the row beside it. A value the field
    /// cannot hold fills nothing rather than a null, so a column the row
    /// carried in the wrong kind leaves the field to what the message said.
    pub(super) fn fill(&mut self, fill: &Fill<'_>) {
        if fill.value.is_null() {
            return;
        }
        // Stated by tag or by name, the message's own answer stands: a fill
        // never overrides a value, and never lands beside a same-named
        // child the line built.
        if self.recorded.contains(&fill.tag)
            || self.slots.iter().any(|slot| {
                (slot.tag == fill.tag && (slot.group || !slot.values.is_empty()))
                    || slot.field.name() == fill.field.name()
            })
        {
            return;
        }
        let critical = super::identity::required_dtype(fill.tag).is_some();
        let typed = if critical {
            typed_fill(fill.field, fill.tag, fill.value)
        } else {
            fill.field.scalar(fill.value.clone())
        };
        let typed = match typed {
            Ok(typed) => typed,
            Err(error) => {
                if critical && self.failure.is_none() {
                    self.failure = Some(error);
                }
                return;
            }
        };
        if typed.is_null() {
            return;
        }
        self.slot_for(fill.field.clone(), fill.tag, true)
            .values
            .push(typed);
    }

    /// The registry field one key names, and the tag it carries.
    ///
    /// The dictionary is one namespace, which is the tier
    /// [`FixMsg`](super::FixMsg) reads a built message by: a row transcribed
    /// against a venue's dictionary names that venue's fields by the venue's
    /// spellings while still carrying `MsgType`, `SenderCompID` and every
    /// other specification field, and one namespace answers all of them.
    fn resolve(&self, key: &str) -> Option<(&'registry Field, i32)> {
        resolve(self.registry, key)
    }

    /// The field a tag names: the one that holds the tag, first holder first.
    fn by_tag(&self, tag: i32) -> Option<&'registry Field> {
        self.registry.get_field_by_tag(tag)
    }

    /// The group a counter tag heads.
    ///
    /// Counters are scalar fields and the groups they head are catalog
    /// definitions, so this is the one lookup that crosses the two. Named
    /// apart from [`Self::by_tag`] because they answer different things off
    /// the same key: that one the scalar holding the tag, this one the group
    /// the scalar counts.
    fn group_by_tag(&self, tag: i32) -> Option<&'registry Field> {
        self.registry.get_group_by_tag(tag)
    }

    /// A group's own field, by the name a key spells it.
    fn by_group(&self, group: &str) -> Option<&'registry Field> {
        self.registry
            .get_definition(crate::FixCategory::Groups, group)
    }

    /// The field a key builds under, as the dictionary declares it.
    ///
    /// A field the dictionary knows is the registry's own, under the one name
    /// and datatype the dictionary gives it: a tag is one column whatever
    /// spelling a version gave it, and a row that renamed itself per version
    /// is a row no two captures share. A spelling an earlier version used
    /// reaches the field as one of its [names](super::FixField::names). One the
    /// dictionary does not know is kept under the key's own folded spelling
    /// as nullable text, because a venue sends fields no dictionary has.
    /// A dialect that spelled one name over two tags is why `scope` is
    /// passed: a dictionary indexes a name once, so such a spelling names
    /// neither tag there, and the message the row declares is what says
    /// which one it meant. `scope` is the children of the level
    /// the key arrived at - the message root for a flat key, the occurrence's
    /// own members for a grouped one - and it is read only where the
    /// dictionary answered nothing, so an unambiguous name costs no scan.
    ///
    /// Beside the child comes the declared field it was built from, where
    /// the dictionary or the scope declared one: what a typed read asks
    /// about the field is asked of that declaration, and a key no dictionary
    /// explained has none.
    ///
    /// `resolved` is what [`Self::known`] answered for the key: a flat key is
    /// resolved once and read twice - for the child it builds and for the
    /// group it may head - so the resolution is passed rather than repeated.
    fn field_from<'scope>(
        &self,
        key: &str,
        resolved: Option<(&'registry Field, i32)>,
        scope: Scope<'scope>,
    ) -> (Field, i32, Option<&'scope Field>)
    where
        'registry: 'scope,
    {
        if let Some((known, tag)) = resolved {
            return (stated(known), tag, Some(known));
        }
        if let Some((declared, tag)) = scope.child(key) {
            return (stated(declared), tag, Some(declared));
        }
        let name = folded_name(key);
        let tag = super::field::parse_tag(key).unwrap_or(0);
        (DataType::utf8().nullable_field(name), tag, None)
    }

    /// What the dictionary holds under one key, asked once per run.
    ///
    /// The field the key names and the group it heads are facts of the
    /// registry and the key alone, so the run remembers them; a dotted path
    /// descends into a definition the identity cannot name, and is resolved
    /// every time.
    fn known(&self, key: &str) -> Known<'registry> {
        if key.contains('.') {
            return Known {
                field: self.resolve(key),
                group: self.by_group(key),
            };
        }
        let lookup = lookup(self.registry, self.memo, key);
        Known {
            field: lookup
                .field
                .and_then(|(tag, id)| Some((self.registry.get_field_by_id(id)?, tag))),
            group: if lookup.group {
                self.by_group(key)
            } else {
                None
            },
        }
    }

    /// The sole namespace-composition resolver. The target travels with the
    /// build, so composition never splits or resolves the name a second time.
    fn composed_target(&self, name: &str) -> Option<(&'registry Field, i32)> {
        if !name.contains('.') {
            return None;
        }
        let lookup = lookup(self.registry, self.memo, name);
        let (tag, id) = lookup.composed?;
        Some((self.registry.get_field_by_id(id)?, tag))
    }

    fn retain_composed(&mut self, field: &Field, raw: &[u8]) -> bool {
        if !field.name().contains('.') {
            return false;
        }
        if let Some(held) = self
            .composed
            .iter_mut()
            .find(|held| held.source == field.name())
        {
            let critical = super::identity::required_dtype(held.tag).is_some();
            held.invalid_utf8 |= critical && std::str::from_utf8(raw).is_err();
            return critical;
        }
        let Some((target, tag)) = self.composed_target(field.name()) else {
            return false;
        };
        let critical = super::identity::required_dtype(tag).is_some();
        let invalid_utf8 = critical && std::str::from_utf8(raw).is_err();
        self.composed.push(Composed {
            source: SmolStr::new(field.name()),
            target: stated(target),
            tag,
            invalid_utf8,
        });
        critical
    }

    /// The message this row declared, which a flat key resolves against when
    /// the dictionary holds no name for it.
    ///
    /// Nothing for a row whose type resolves to no message definition, which
    /// is every row a dialect declares no grammar for.
    fn scope(&self) -> Scope<'registry> {
        Scope::Message(self.message)
    }

    /// Types one value under one field, translating its code first.
    ///
    /// A value that will not type is null rather than a failure: the raw text
    /// stays in the entries, the refusal is readable, and the message still
    /// answers `Ok`. A parse error is for input that is not a message at all.
    ///
    /// `source` is the dictionary's own declaration the field was built from,
    /// where there is one: what it states about its values - its null
    /// spellings, its code set - is read off it once per run and remembered,
    /// because a line asks it for every value and a capture asks it a
    /// million times. A field with no such declaration is read directly.
    fn typed(
        &self,
        field: &Field,
        source: Option<&'registry Field>,
        raw: &[u8],
        text: &str,
    ) -> Scalar {
        // What the row types is the text with its unknown characters gone:
        // a byte no encoding explained became U+FFFD in the decode, and a
        // control byte a bridge left in a value is not part of it. The entry
        // keeps the decode as it was, which is what the anomaly reads.
        let cleaned = cleaned(text);
        self.typed_value(field, source, raw, &cleaned)
            .unwrap_or(Scalar::Null)
    }

    /// The shared conversion before best-effort callers discard a refusal.
    fn typed_value(
        &self,
        field: &Field,
        source: Option<&'registry Field>,
        raw: &[u8],
        text: &str,
    ) -> Result<Scalar> {
        let facts = source.map(|source| self.memo.facts(source));
        // A spelling this field states as its own absence types as null while
        // the entry keeps the text: which spelling means "nothing was sent" is
        // a fact about the field, and the row is the interpretation where the
        // entries are what arrived. The capture-wide list is the other half of
        // the pair and was applied before this key was resolved at all.
        let absent = match &facts {
            Some(facts) => facts.is_null(text),
            None => field.as_fix().is_null_value(text),
        };
        if absent {
            return Ok(Scalar::Null);
        }
        // A `data` field's value is bytes, and the row is where they live:
        // the entry holds a lossy decode of them and this does not.
        if is_binary(field.dtype()) {
            return field.scalar(Scalar::from(raw.to_vec()));
        }
        // The translation is the one step of a typed read that walks a
        // document, and the one whose answer repeats across a run.
        match (&facts, source) {
            (Some(facts), Some(source)) => {
                let translated = self.memo.translation(source, facts, text);
                typed_translation(field, text, translated.as_deref())
            }
            _ => typed_spelling_checked(field, text),
        }
    }

    /// Root invariants retain their first conversion refusal and never clean
    /// invalid bytes into a different valid identity or clock spelling.
    fn typed_root(
        &mut self,
        field: &Field,
        source: Option<&'registry Field>,
        tag: i32,
        raw: &[u8],
        text: &str,
    ) -> Scalar {
        let composed = self.retain_composed(field, raw);
        if super::identity::required_dtype(tag).is_none() {
            if composed {
                // The source is still an ordinary row cell. Only a winning
                // composition checks it against the resolved target below.
                return field.scalar(Scalar::from(text)).unwrap_or(Scalar::Null);
            }
            return self.typed(field, source, raw, text);
        }
        let value = super::identity::validate_field(field, tag).and_then(|_| {
            super::identity::resolve_tag(field, self.registry)?;
            let text = std::str::from_utf8(raw).map_err(|error| invalid_value(field, error))?;
            self.typed_value(field, source, raw, text)
        });
        match value {
            Ok(value) => value,
            Err(error) => {
                if self.failure.is_none() {
                    self.failure = Some(invalid_value(field, error));
                }
                Scalar::Null
            }
        }
    }

    /// One flat child, appended in arrival order.
    fn push_flat(&mut self, key: &str, text: &str, raw: &[u8]) {
        // A flat key naming a repeating group is that group's counter: it
        // opens the group slot the members land in, and the number that
        // arrived stays in the counter's own child beside it. The group's
        // length and the stated count are two readings of one line, compared
        // on demand through `anomalies()`.
        //
        // A tag reaches the dictionary once, and which half it is in decides
        // the rest: the counter and the scalar readings are the same probe
        // filtered two ways, and a numeric key is most of every frame.
        let mut located = Known::NONE;
        let (field, tag, source) = if let Some(parsed) = super::field::parse_tag(key) {
            // A tag an open group declares is that group's member, placed in
            // the occurrence the frame's order implies; any other tag closes
            // every group still open, because a numeric frame ends a group
            // by moving on.
            if let Some(depth) = self.grouped_depth(parsed) {
                self.push_numeric_member(depth, parsed, key, text, raw);
                return;
            }
            self.open.clear();
            // The tag's first holder, the same probe `push_pairs` reads under.
            match self.by_tag(parsed) {
                Some(found) => {
                    let tag = self.registry.identity_of(found).map_or(0, |(tag, _)| tag);
                    if found.dtype().is_nested() {
                        if self.shadowed(found.name()) {
                            return;
                        }
                        self.record(tag);
                        let members = declared_members(found);
                        let name = SmolStr::new(found.name());
                        let slot = self.slot_for(stated(found), tag, true);
                        slot.group = true;
                        let base = slot.occurrences.len();
                        self.open.push(OpenGroup {
                            name,
                            members,
                            occurrence: None,
                            base,
                            seen: Vec::new(),
                        });
                        return;
                    }
                    (stated(found), tag, Some(found))
                }
                None => (
                    DataType::utf8().nullable_field(folded_name(key)),
                    parsed,
                    None,
                ),
            }
        } else {
            located = self.known(key);
            self.field_from(key, located.field, self.scope())
        };
        if self.shadowed(field.name()) {
            return;
        }
        if raw.is_empty()
            && super::identity::required_dtype(tag).is_none()
            && self
                .composed_target(field.name())
                .is_none_or(|(_, target)| super::identity::required_dtype(target).is_none())
        {
            return;
        }
        let value = self.typed_root(&field, source, tag, raw, text);
        self.record(if source.is_some() {
            tag
        } else {
            unresolved(tag)
        });
        self.slot_for(field, tag, source.is_some())
            .values
            .push(value);
        // The counter's child is built first, so the count keeps the column
        // its own field names; the group it heads is opened after it, empty
        // until a member arrives - located, indexed or numbered.
        if let Some((group, counter)) = self.counter_from(&located) {
            if !self.shadowed(group.name()) {
                // Not `known`, for the reason the numeric path states: the
                // counter holds the tag, the group it heads does not.
                self.slot_for(group, counter, false).group = true;
            }
        }
    }

    /// The repeating group a flat key names, when it names one.
    ///
    /// Counters resolve as scalar fields; the catalog supplies the group.
    fn counter_from(&self, located: &Known<'registry>) -> Option<(Field, i32)> {
        if let Some(group) = located.group {
            return Some((stated(group), group.as_fix().counter().ok()??));
        }
        let (_, tag) = located.field?;
        let group = self.numeric_group(tag)?;
        Some((stated(group), tag))
    }

    /// The depth of the open group that declares `tag`, closing every group
    /// opened inside it on the way: a member of an outer group arriving
    /// means the inner group ended.
    fn grouped_depth(&mut self, tag: i32) -> Option<usize> {
        while let Some(innermost) = self.open.last() {
            if innermost.members.contains(&tag) {
                return Some(self.open.len() - 1);
            }
            self.open.pop();
        }
        None
    }

    /// One member of a numeric frame's open group, placed in the occurrence
    /// the frame's order implies.
    ///
    /// The delimiter - the group's first declared member - opens an
    /// occurrence, and so does a member the current occurrence already
    /// holds, because a wire never states one member twice in one
    /// occurrence. The member then builds through the same path a bridge's
    /// located key builds, so a numeric frame and a bridge row stating one
    /// group land in one shape; the entry keeps the tag as it arrived.
    fn push_numeric_member(&mut self, depth: usize, tag: i32, key: &str, text: &str, raw: &[u8]) {
        {
            let group = &mut self.open[depth];
            let opens = group.occurrence.is_none()
                || group.members.first() == Some(&tag)
                || group.seen.contains(&tag);
            if opens {
                group.occurrence = Some(group.occurrence.map_or(group.base, |held| held + 1));
                group.seen.clear();
            }
            group.seen.push(tag);
        }
        // The located key a bridge would have written for this member:
        // every open group down to the member's own, each at its current
        // occurrence, then the member's tag.
        let top = &self.open[0];
        let top_name = top.name.clone();
        let top_occurrence = top.occurrence.unwrap_or(0);
        let mut member = String::new();
        for inner in &self.open[1..=depth] {
            member.push_str(inner.name.as_str());
            member.push('[');
            member.push_str(&format_smolstr!("{}", inner.occurrence.unwrap_or(0)));
            member.push_str("].");
        }
        member.push_str(key);
        self.push_grouped(top_name.as_str(), top_occurrence, &member, text, raw);
        // A member that is itself a counter opens its group inside the
        // occurrence, and what follows fills that group first.
        if let Some(nested) = self.group_by_tag(tag) {
            let members = declared_members(nested);
            self.open.push(OpenGroup {
                name: SmolStr::new(nested.name()),
                members,
                occurrence: None,
                base: 0,
                seen: Vec::new(),
            });
        }
    }

    /// One occurrence of a repeated flat field, placed by index.
    fn push_repeated(&mut self, name: &str, occurrence: usize, text: &str, raw: &[u8]) {
        // An indexed counter is the group stated per occurrence: the group
        // slot holds what each index said, and no scalar column is built
        // beside it.
        let located = self.known(name);
        if let Some((field, tag)) = self.counter_from(&located) {
            if self.shadowed(field.name()) {
                return;
            }
            self.record_under(tag, 0);
            let slot = self.slot_for(field, tag, true);
            slot.group = true;
            while slot.values.len() <= occurrence {
                slot.values.push(Scalar::Null);
            }
            slot.values[occurrence] = Scalar::from(text);
            return;
        }
        let (field, tag, source) = self.field_from(name, located.field, self.scope());
        if self.shadowed(field.name()) {
            return;
        }
        if raw.is_empty()
            && super::identity::required_dtype(tag).is_none()
            && self
                .composed_target(field.name())
                .is_none_or(|(_, target)| super::identity::required_dtype(target).is_none())
        {
            return;
        }
        let value = self.typed_root(&field, source, tag, raw, text);
        self.record(if source.is_some() {
            tag
        } else {
            unresolved(tag)
        });
        let slot = self.slot_for(field, tag, source.is_some());
        // Indices may be partial or out of order, so occurrences are built by
        // index and a gap is null.
        while slot.values.len() <= occurrence {
            slot.values.push(Scalar::Null);
        }
        slot.values[occurrence] = value;
    }

    /// The field, tag and dictionary standing of one group, addressed by
    /// its counter's tag or by its name.
    ///
    /// The same resolution the flat counter uses, so a group addressed by
    /// its tag and one addressed by its name reach one slot: `FixKey` reads
    /// every string as a name, so a numeric key resolves only tag-first,
    /// and the dictionary's own field answers for both - two spellings of
    /// one group would otherwise build two columns carrying one tag.
    fn group_field(&mut self, group: &str) -> (Field, i32, bool) {
        if let Some((_, field, tag, known)) = self.groups.iter().find(|(held, ..)| held == group) {
            return (field.clone(), *tag, *known);
        }
        let located = self.known(group);
        let answer = match self.counter_from(&located) {
            Some((field, tag)) => (field, tag, true),
            None => match located.group {
                Some(known) => {
                    let tag = known.as_fix().tag().ok().flatten().unwrap_or(0);
                    (stated(known), tag, true)
                }
                None => (
                    DataType::utf8().nullable_field(folded_name(group)),
                    super::field::parse_tag(group).unwrap_or(0),
                    false,
                ),
            },
        };
        self.groups
            .push((SmolStr::new(group), answer.0.clone(), answer.1, answer.2));
        answer
    }

    /// One member of one occurrence of a repeating group, at any depth.
    ///
    /// `member` may itself address a group nested in the occurrence -
    /// `NOPARTYSUBIDS[0].PARTYSUBID` under `NOPARTYIDS[0]` - so the path is
    /// resolved level by level against the dictionary first, and the slots
    /// walked once after, each level a group slot inside the occurrence
    /// above it. A member naming a nested group's counter opens that group
    /// in the occurrence without a value, exactly as a flat counter does at
    /// the top.
    fn push_grouped(
        &mut self,
        group: &str,
        occurrence: usize,
        member: &str,
        text: &str,
        raw: &[u8],
    ) {
        // The path, resolved: the group at the top, every nested group under
        // it beside the occurrence it addresses, and the leaf.
        let mut levels: Vec<(Field, i32, bool, usize)> = Vec::new();
        let (group_field, group_tag, known) = self.group_field(group);
        if self.shadowed(group_field.name()) {
            return;
        }
        levels.push((group_field, group_tag, known, occurrence));
        let mut leaf = member;
        loop {
            let located = Key::parse(leaf);
            match located.located {
                Located::Grouped {
                    group: sub,
                    occurrence: index,
                    member: rest,
                } => {
                    let (field, tag, known) = self.group_field(sub);
                    levels.push((field, tag, known, index));
                    leaf = rest;
                }
                Located::Repeated {
                    name,
                    occurrence: index,
                } => {
                    // An occurrence stating one bare value: the value is what
                    // the sub-occurrence holds, under the group's own name.
                    let (field, tag, known) = self.group_field(name);
                    levels.push((field, tag, known, index));
                    leaf = "";
                    break;
                }
                Located::Flat => break,
            }
        }
        // An unresolved level nests its members under the build-time tag its
        // own counter recorded, so the tree keeps what arrived under it.
        let parent_tag = levels
            .last()
            .map(|(_, tag, known, _)| if *known { *tag } else { unresolved(*tag) })
            .expect("the top group at least");
        // The leaf: a value under its field, or a nested group's counter
        // opening that group in the occurrence - resolved as the flat
        // counter resolves, through the dictionary's nested half first.
        let mut source = None;
        let mut leaf_known = false;
        let (leaf_field, leaf_tag, nested_counter) = if leaf.is_empty() {
            (
                DataType::utf8()
                    .nullable_field(occurrence_name(&levels.last().expect("a level").0)),
                0,
                false,
            )
        } else {
            let located = self.known(leaf);
            if let Some((field, tag)) = self.counter_from(&located) {
                leaf_known = true;
                (field, tag, true)
            } else {
                // The occurrence's own members are the level this leaf
                // arrived at, so a spelling the dictionary shares between
                // two tags resolves to the one this group declares. A member
                // the dictionary named is read through its declaration; one
                // only the level declares is read directly, because the
                // level is this line's own.
                source = located.field.map(|(field, _)| field);
                let scope = Scope::Members(member_fields(&levels.last().expect("a level").0));
                let (field, tag, declaration) = self.field_from(leaf, located.field, scope);
                leaf_known = declaration.is_some();
                (field, tag, false)
            }
        };
        let value = if leaf.is_empty() || nested_counter {
            Scalar::Null
        } else {
            self.typed(&leaf_field, source, raw, text)
        };
        // Recorded after the path resolves, so the entry can ride under the
        // counter pair that heads it - when that pair actually arrived.
        self.record_under(
            parent_tag,
            if leaf_known {
                leaf_tag
            } else {
                unresolved(leaf_tag)
            },
        );
        let (top_field, top_tag, top_known, _) = levels.remove(0);
        let mut slot = self.slot_for(top_field, top_tag, top_known);
        slot.group = true;
        let mut at = occurrence;
        for (field, tag, known, index) in levels {
            slot = slot.nested(at, field, tag, known);
            at = index;
        }
        if leaf.is_empty() {
            // The sub-occurrence's bare value, by index as a repeated flat
            // field keeps its own.
            while slot.values.len() <= at {
                slot.values.push(Scalar::Null);
            }
            slot.values[at] = Scalar::from(text);
            return;
        }
        while slot.occurrences.len() <= at {
            slot.occurrences.push(Vec::new());
        }
        if nested_counter {
            slot.nested(at, stated(&leaf_field), leaf_tag, true);
        } else {
            slot.occurrences[at].push(Member::Value(leaf_field, value));
        }
    }

    /// The slot one field builds into, created on first use.
    ///
    /// Found by name, because the name is what a child is: two keys the
    /// dictionary resolves to one field build one column, and two unknown
    /// keys folding to one spelling do too. The digest is compared first
    /// and the name only where it agrees, so a hit costs one string compare
    /// and a miss costs none.
    fn slot_for(&mut self, field: Field, tag: i32, known: bool) -> &mut Slot {
        let hash = crate::hashing::xxhash::xxh64(field.name().as_bytes());
        let held = self
            .hashes
            .iter()
            .enumerate()
            .find(|(index, held)| **held == hash && self.slots[*index].field.name() == field.name())
            .map(|(index, _)| index);
        match held {
            Some(index) => &mut self.slots[index],
            None => {
                // A structured root can have a composed name too. Its value
                // needs no cleaning provenance, but shares the same target.
                self.retain_composed(&field, &[]);
                self.hashes.push(hash);
                self.slots.push(Slot {
                    field,
                    tag,
                    known,
                    values: Vec::new(),
                    group: false,
                    occurrences: Vec::new(),
                });
                self.slots.last_mut().expect("just pushed")
            }
        }
    }

    /// Records what arrived under the counter that heads it, where one did.
    ///
    /// `counter_tag` is a build-time tag: an unresolved counter's is the
    /// [`unresolved`] negation, so its members nest under it exactly as under
    /// a resolved counter while no resolved tag can match it.
    ///
    /// The counter pair itself must have arrived: an entry is the arrival
    /// record, and a parent nobody sent would be an invention. A member whose
    /// counter never arrived stays flat at the top, exactly as a wire with no
    /// stated structure keeps it, and the latest arrival of the counter is
    /// the one that takes the member - which is what nests each occurrence
    /// under its own heading.
    fn record_under(&mut self, counter_tag: i32, tag: i32) {
        let Some((tag, mut entry)) = self.arrived(tag) else {
            return;
        };
        self.recorded.push(tag);
        // The walk finds a counter only where one was recorded, so a record
        // holding none is not walked for it.
        if counter_tag != 0 && self.recorded.contains(&counter_tag) {
            for root in self.entries.iter_mut().rev() {
                match root.adopt(counter_tag, entry) {
                    None => return,
                    Some(back) => entry = back,
                }
            }
        }
        self.entries.push(entry);
    }

    /// Records what arrived, whatever the row made of it.
    fn record(&mut self, tag: i32) {
        let Some((tag, entry)) = self.arrived(tag) else {
            return;
        };
        self.recorded.push(tag);
        self.entries.push(entry);
    }

    /// The entry the pair being folded in records, and the tag it carries.
    ///
    /// Absent while a row nested inside one of the line's data fields is being
    /// read - that row is a reading of a value the line already recorded - and
    /// absent for every member unpacked out of a packed occurrence after the
    /// first, which recorded the pair the bridge actually wrote. A key that
    /// named an occurrence rather than a field carries no tag, exactly as an
    /// unresolved key does.
    ///
    /// Taken rather than cloned: a pair is recorded once, where its key
    /// finished resolving, and the two ranges move into the entry rather than
    /// being counted a second time on their way there.
    fn arrived(&mut self, tag: i32) -> Option<(i32, FixEntry)> {
        if self.outer.is_some() {
            return None;
        }
        let arrived = self.arrival.take()?;
        let tag = if arrived.named { tag } else { 0 };
        Some((tag, FixEntry::new(tag, arrived.key, arrived.value)))
    }

    /// Closes the build into a root field, its value, and the entries.
    ///
    /// Order is the standard header, then the body in arrival order, then the
    /// standard trailer - flat, with no `StandardHeader` Struct, because a
    /// message is laid flat and the header is an ordering rather than a
    /// nesting.
    ///
    /// Beside the row comes the tag index a message reads it by - each
    /// dictionary child's tag and its position, tag-major - because the
    /// builder resolved every tag once already and the message would only
    /// read them back out of the fields it just wrote.
    ///
    /// Three children every message has, whatever its line carried, and none
    /// of them is an entry because none of them arrived. `BeginString` is
    /// filled from the version the message was read at where the line stated
    /// none, so a bridge row and a configuration document say which FIX they
    /// were read as exactly as a frame does. The crate's `version` states
    /// that version outright - the codec's target where the caller pinned
    /// one - because `BeginString` is what the message says about *itself*
    /// and the two differ every time a session carries a row written to a
    /// later FIX than it speaks. Mandatory clocks and identity are finalized
    /// after namespace composition, never while the payload is still built.
    pub(super) fn finish(self, name: &str) -> Result<Built> {
        let Self {
            beginstring,
            version,
            mut slots,
            mut entries,
            recorded,
            failure,
            composed,
            ..
        } = self;
        if let Some(error) = failure {
            return Err(error);
        }
        if recorded.iter().any(|tag| *tag < 0) {
            entries.iter_mut().for_each(FixEntry::settle_unresolved);
        }
        if !slots
            .iter()
            .any(|slot| slot.tag == 8 || slot.field.name() == "beginstring")
        {
            let field = beginstring.clone();
            let spelled = format_smolstr!("FIX.{}", version.unwrap_or_else(default_version));
            let value = field
                .scalar(Scalar::from(spelled.as_str()))
                .unwrap_or_else(|_| Scalar::from(spelled.as_str()));
            slots.push(Slot {
                field,
                tag: 8,
                known: true,
                values: vec![value],
                group: false,
                occurrences: Vec::new(),
            });
        }
        // The version the read used, on every message it produced: the
        // codec's target where the caller pinned one, else what the line's
        // own frame implied, else the crate's own default. `BeginString` is
        // what the message says about itself and is left exactly as it
        // arrived; this is what answered it, and the two differ every time a
        // session carries a row written to a later FIX than it speaks.
        if !slots
            .iter()
            .any(|slot| slot.tag == super::VERSION_TAG_NAME.0)
        {
            if let Some(field) = super::crated::version_field() {
                let spelled = format_smolstr!("{}", version.unwrap_or_else(default_version));
                let value = field
                    .scalar(Scalar::from(spelled.as_str()))
                    .unwrap_or_else(|_| Scalar::from(spelled.as_str()));
                slots.push(Slot {
                    field: field.clone(),
                    tag: super::VERSION_TAG_NAME.0,
                    known: true,
                    values: vec![value],
                    group: false,
                    occurrences: Vec::new(),
                });
            }
        }
        // Each slot's place is read once, as a rank, rather than once per
        // comparison inside the sort.
        let mut ordered: Vec<(usize, Slot)> = Vec::with_capacity(slots.len());
        let mut rest: Vec<Slot> = Vec::with_capacity(slots.len());
        let mut trailing: Vec<(usize, Slot)> = Vec::new();
        for slot in slots {
            if let Some(rank) = rank_in(&STANDARD_HEADER_TAGS, slot.tag) {
                ordered.push((rank, slot));
            } else if let Some(rank) = rank_in(&STANDARD_TRAILER_TAGS, slot.tag) {
                trailing.push((rank, slot));
            } else {
                rest.push(slot);
            }
        }
        ordered.sort_by_key(|(rank, _)| *rank);
        trailing.sort_by_key(|(rank, _)| *rank);
        let count = ordered.len() + rest.len() + trailing.len();
        let ordered = ordered
            .into_iter()
            .map(|(_, slot)| slot)
            .chain(rest)
            .chain(trailing.into_iter().map(|(_, slot)| slot));

        let mut fields = Vec::with_capacity(count);
        let mut values = Vec::with_capacity(count);
        let mut tags = Vec::with_capacity(count);
        for (index, slot) in ordered.enumerate() {
            if slot.known {
                tags.push((slot.tag, index));
            }
            let (field, value) = slot.into_child()?;
            fields.push(field);
            values.push(value);
        }
        tags.sort_unstable();
        let root = DataType::from_fields(fields)?.required_field(name);
        Ok(Built {
            field: root,
            value: Scalar::from_sequence(values),
            entries,
            tags,
            composed,
        })
    }
}

/// What one build finishes as: the row's schema and value, what arrived,
/// and where each dictionary tag sits.
pub(super) struct Built {
    pub(super) field: Field,
    pub(super) value: Scalar,
    pub(super) entries: Vec<FixEntry>,
    pub(super) tags: Vec<(i32, usize)>,
    composed: Vec<Composed>,
}

impl Built {
    /// The first declared holder, else the canonical name the registry owns.
    pub(super) fn index_of_tag(&self, tag: i32, registry: &FixRegistry) -> Option<usize> {
        let at = self.tags.partition_point(|(held, _)| *held < tag);
        self.tags
            .get(at)
            .filter(|(held, _)| *held == tag)
            .map(|(_, index)| *index)
            .or_else(|| {
                registry
                    .get_field_by_tag(tag)
                    .and_then(|field| self.field.index_of(field.name()))
            })
    }

    /// A namespace's last segment may fill one absent dictionary field.
    /// Conflicting voices fill nothing. This is intake, before defaults;
    /// entries stay the payload's own and the existing row is rebuilt once.
    pub(super) fn compose(&mut self, registry: &FixRegistry) -> Result<()> {
        let mut named: Vec<Composition<'_>> = Vec::new();
        let values = self
            .value
            .as_sequence()
            .ok_or_else(|| invalid_value(&self.field, "expected a canonical Struct row"))?;
        for source in &self.composed {
            let Some(value) = self
                .field
                .index_of(&source.source)
                .and_then(|at| values.get(at))
            else {
                continue;
            };
            let tag = source.tag;
            let at = self.index_of_tag(tag, registry);
            if value.is_null()
                || at
                    .and_then(|at| values.get(at))
                    .is_some_and(|held| !held.is_null())
            {
                continue;
            }
            match named.iter_mut().find(|held| held.source.tag == tag) {
                Some(held) => {
                    held.disagreed |= held.write.value != *value;
                    held.invalid_utf8 |= source.invalid_utf8;
                }
                None => named.push(Composition {
                    source,
                    write: super::msg::Write {
                        at,
                        field: source.target.clone(),
                        value: value.clone(),
                    },
                    disagreed: false,
                    invalid_utf8: source.invalid_utf8,
                }),
            }
        }
        let mut writes = Vec::with_capacity(named.len());
        for Composition {
            source,
            mut write,
            disagreed,
            invalid_utf8,
        } in named
        {
            if disagreed {
                continue;
            }
            write.value = if super::identity::required_dtype(source.tag).is_some() {
                if invalid_utf8 {
                    return Err(invalid_value(
                        &write.field,
                        "expected valid UTF-8 in the composed value",
                    ));
                }
                typed_fill(&write.field, source.tag, &write.value)?
            } else {
                let Ok(value) = write.field.scalar(write.value) else {
                    continue;
                };
                value
            };
            if write.value.is_null() {
                write.field.set_nullable(true);
            }
            writes.push(write);
        }
        if !writes.is_empty() {
            let (field, values, indexed) =
                super::msg::stage_writes(&self.field, &self.value, writes)?;
            self.field = field;
            self.value = Scalar::from_sequence(values);
            for change in indexed {
                change.apply(&mut self.tags, None);
            }
        }
        self.composed.clear();
        Ok(())
    }
}

/// The text a value is typed from, with its unknown characters gone.
///
/// A lossy decode marks a byte no encoding explained with U+FFFD, and a
/// bridge sometimes leaves a control byte inside a value; neither is part of
/// what the value says. The common case - clean text - borrows.
fn cleaned(text: &str) -> std::borrow::Cow<'_, str> {
    if text
        .chars()
        .all(|held| held != '\u{FFFD}' && (!held.is_control() || held == '\t'))
    {
        return std::borrow::Cow::Borrowed(text);
    }
    std::borrow::Cow::Owned(
        text.chars()
            .filter(|held| *held != '\u{FFFD}' && (!held.is_control() || *held == '\t'))
            .collect(),
    )
}

/// Where one tag sits in a component's declared order, if it is in it.
fn rank_in(component: &[i32], tag: i32) -> Option<usize> {
    component.iter().position(|held| *held == tag)
}

impl Slot {
    /// This slot as one child field and its value.
    fn into_child(self) -> Result<(Field, Scalar)> {
        // A group whose counter arrived and whose members did not is a group
        // holding nothing, not a scalar: the empty list is what lets the
        // count it stated be compared with what the row actually holds.
        if self.group && self.occurrences.is_empty() {
            if self.values.is_empty() {
                let value = Scalar::from_sequence(Vec::new());
                return Ok((self.field, value));
            }
            // An unlabeled value cannot type as a component. Preserve its
            // raw entry and occurrence position; the typed occurrence is null.
            let mut item = match self.field.dtype() {
                DataType::List(item) | DataType::LargeList(item) => item.as_ref().clone(),
                _ => DataType::from_fields([])?.required_field(occurrence_name(&self.field)),
            };
            item.set_nullable(true);
            let values = Scalar::from_sequence(self.values.into_iter().map(|_| Scalar::Null));
            let mut list = DataType::list(item).required_field(self.field.name());
            let _ = list.set_metadata(self.field.as_metadata().iter());
            return Ok((list, values));
        }
        if self.occurrences.is_empty() {
            // A value that would not type is null, and a field a null lands
            // in is nullable: the message's schema says the value is there
            // only where it actually is.
            let absent = self.values.iter().any(Scalar::is_null);
            if self.values.len() <= 1 {
                let value = self.values.into_iter().next().unwrap_or(Scalar::Null);
                let mut field = self.field;
                if absent || value.is_null() {
                    field.set_nullable(true);
                }
                return Ok((field, value));
            }
            // A tag appearing twice stays two occurrences in input order: a
            // map keyed by tag would lose a repeating group. Indices may be
            // gapped, and a gap is null, so the item takes that nullability.
            // Not a group: no counter arrived, so there is no component to
            // name. The occurrence takes the repeated field's own name, which
            // is what keeps this shape distinguishable from a repeating group.
            let mut item = self.field.clone().with_name(self.field.name().to_owned());
            item.set_nullable(absent);
            let values = Scalar::from_sequence(self.values);
            let mut list = DataType::list(item).required_field(self.field.name());
            let _ = list.set_metadata(self.field.as_metadata().iter());
            return Ok((list, values));
        }

        // A group is a List of its named component. Occurrence indices retain
        // gaps as null and never shift subsequent entries.
        // In first-seen order across every occurrence, so a member only the
        // second occurrence carries is still a column and still in its
        // arrival place. Nullable, because an occurrence need not state one.
        // Every occurrence becomes its members' fields and values first -
        // a nested group closing into its own list field on the way - so
        // the item's fields are read off finished children.
        let mut finished: Vec<Option<Vec<(Field, Scalar)>>> =
            Vec::with_capacity(self.occurrences.len());
        for occurrence in self.occurrences {
            if occurrence.is_empty() {
                finished.push(None);
                continue;
            }
            let mut members = Vec::with_capacity(occurrence.len());
            for member in occurrence {
                members.push(match member {
                    Member::Value(field, value) => (field, value),
                    Member::Group(slot) => slot.into_child()?,
                });
            }
            finished.push(Some(members));
        }
        let mut member_fields: Vec<Field> = Vec::new();
        for occurrence in finished.iter().flatten() {
            for (field, _) in occurrence {
                if member_fields.iter().any(|held| held.name() == field.name()) {
                    continue;
                }
                let mut member = field.clone();
                member.set_nullable(true);
                member_fields.push(member);
            }
        }
        let mut item = DataType::from_fields(member_fields.clone())?
            .required_field(occurrence_name(&self.field));
        // A gapped index leaves an occurrence nobody stated, which is null.
        if finished.iter().any(Option::is_none) {
            item.set_nullable(true);
        }
        let mut rows = Vec::with_capacity(finished.len());
        for occurrence in finished {
            let Some(occurrence) = occurrence else {
                rows.push(Scalar::Null);
                continue;
            };
            let mut row = Vec::with_capacity(member_fields.len());
            for field in &member_fields {
                let held = occurrence
                    .iter()
                    .find(|(held, _)| held.name() == field.name())
                    .map(|(_, value)| value.clone());
                row.push(held.unwrap_or(Scalar::Null));
            }
            rows.push(Scalar::from_sequence(row));
        }
        let mut list = DataType::list(item).required_field(self.field.name());
        let _ = list.set_metadata(self.field.as_metadata().iter());
        Ok((list, Scalar::from_sequence(rows)))
    }
}

/// The day a FIX temporal that states no date is read on.
///
/// `TZTimeOnly` is a time of day and an offset, so the instant it names needs
/// a day and the specification supplies none. The epoch day is the one choice
/// that costs nothing: the count is the time of day itself, in the unit the
/// column declares, and two readings still subtract.
const EPOCH_DAY: &str = "1970-01-01";

/// The value one FIX wire spelling states, where FIX spells it its own way.
///
/// A boolean is `Y` or `N`. A temporal is a run of digits, and the crate's own
/// ISO reader takes the shape of one as it stands - `20260821-10:30:00.123456`
/// and the bare `20260821` are both readings there - so what is supplied here
/// is only what FIX leaves out of the reading: the zone a UTC column carries,
/// the date a `TZTimeOnly` omits, the seconds an `HH:MM` stops before, and the
/// decimal point a bridge writing one long digit run never writes. Those are
/// facts about FIX rather than about the datatype, so they are stated here and
/// the generic value contract learns none of them.
///
/// Three FIX datatypes land on `DateTime64` and this reads all three, because
/// only their spelling differs: `UTCTimestamp` states a date and no zone,
/// `TZTimestamp` states both, and `TZTimeOnly` states a zone and no date. So
/// the date is taken where there is one and the epoch day stands in where
/// there is not, the seconds a `TZTimeOnly` may omit are filled, and the zone
/// is kept where the value states one rather than `Z` written over it - which
/// is what a `TZTimestamp` carrying `-05:00` needs, since appending `Z` to it
/// spells a zone twice and reads as nothing at all.
///
/// Neither half may be missing at once. A value stating a date is read
/// whatever zone it states or omits, and one stating only a clock is read
/// only where it states an offset - so a dated spelling that arrives without
/// its date still answers nothing, exactly as it did when only the dated
/// shape was read.
pub(super) fn wire_spelling(dtype: &DataType, text: &str) -> Option<Scalar> {
    match dtype {
        DataType::Boolean => match text.as_bytes() {
            [b'Y' | b'y'] => Some(Scalar::from(true)),
            [b'N' | b'n'] => Some(Scalar::from(false)),
            _ => None,
        },
        DataType::DateTime64 { timezone, .. } => {
            // The zone a value states outranks the column's, and a column
            // stating none takes no zone rather than Z: a `LocalMktDate` and
            // a `LocalMktDatetime` are local market values, and rendering
            // them as instants would make the reading claim a zone the wire
            // never sent.
            let implied = if timezone.is_naive() { "" } else { "Z" };
            // A date states no clock, so it reads as that day at midnight -
            // which is what makes `UTCDateOnly` and `LocalMktDate` instants
            // rather than a second temporal type to cast through. The reader
            // takes the compact date as it stands, so the only thing left to
            // write is the zone FIX leaves implied, and a naive column is
            // owed none: its wire text is already the reading.
            if text.len() == 8 && text.bytes().all(|byte| byte.is_ascii_digit()) {
                return (!implied.is_empty())
                    .then(|| Scalar::from(format_smolstr!("{text}{implied}").as_str()));
            }
            // A bridge writes a clock as one digit run - the date, the
            // time, and three, six or nine digits of fraction - and it is
            // the same instant `20240102-10:15:30.123` spells with its
            // separators.
            if text.len() >= 14 && text.bytes().all(|byte| byte.is_ascii_digit()) {
                let fraction = &text[14..];
                if matches!(fraction.len(), 0 | 3 | 6 | 9) {
                    let point = if fraction.is_empty() { "" } else { "." };
                    let rendered = format_smolstr!(
                        "{}-{}-{}T{}:{}:{}{point}{fraction}{implied}",
                        &text[..4],
                        &text[4..6],
                        &text[6..8],
                        &text[8..10],
                        &text[10..12],
                        &text[12..14],
                    );
                    return Some(Scalar::from(rendered.as_str()));
                }
            }
            let dated = fix_date(text);
            let (clock, zone) = zoned(dated.as_ref().map_or(text, |(_, rest)| *rest));
            let date = match dated.as_ref() {
                Some((date, _)) => date.as_str(),
                // A value stating no date is a `TZTimeOnly`, and readable
                // only where it states its offset: FIX means *local* time by
                // omitting one, which an instant cannot hold, and a dated
                // spelling that lost its date is not a reading either.
                None if zone.is_some() => EPOCH_DAY,
                None => return None,
            };
            // `HH:MM` is the one width a FIX clock may stop at, and only a
            // `TZTimeOnly` does; anything else is left to fail the read.
            let seconds = if clock.len() == 5 { ":00" } else { "" };
            let zone = zone.unwrap_or(implied);
            let rendered = format_smolstr!("{date}T{clock}{seconds}{zone}");
            Some(Scalar::from(rendered.as_str()))
        }
        _ => None,
    }
}

/// The ISO date a FIX temporal opens with, and what follows it.
///
/// The eight digits are the test rather than the `-`, because a `TZTimeOnly`
/// carrying a western offset has one too and `07:39:12-08` would otherwise
/// read `07:39:12` as a date.
///
/// A value stating no date answers `None`, which is what separates the two
/// kinds of caller: a column declared `TZTimeOnly` takes the epoch day and
/// reads the instant, while one deriving a capture's clock wants a moment the
/// capture happened at and has to refuse a dateless one.
pub(super) fn fix_date(text: &str) -> Option<(SmolStr, &str)> {
    let (day, rest) = text.split_once('-')?;
    if day.len() != 8 || !day.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((
        format_smolstr!("{}-{}-{}", &day[..4], &day[4..6], &day[6..8]),
        rest,
    ))
}

/// One FIX clock split from the zone it states, or `None` where it states
/// none.
///
/// A clock states no sign and no `Z`, so the first of either opens the zone.
/// A dated value stating none is UTC, which is what `UTCTimestamp` means by
/// saying nothing; a dateless one stating none is not read at all.
fn zoned(text: &str) -> (&str, Option<&str>) {
    if let Some(clock) = text.strip_suffix(['Z', 'z']) {
        return (clock, Some("Z"));
    }
    match text.find(['+', '-']) {
        Some(at) => {
            let (clock, zone) = text.split_at(at);
            (clock, Some(zone))
        }
        None => (text, None),
    }
}

/// Types one wire spelling under one field, translating its code first.
///
/// The half of a typed read that needs no bytes: what a value's text says
/// under the field it lands in, at the version the read is pinned to - or
/// at every version, where `at` is `None`, which is how a value is re-typed
/// for a field it did not arrive under. A spelling that will not type is
/// null rather than a failure, for the reason the builder's read is.
pub(super) fn typed_spelling(field: &Field, text: &str) -> Scalar {
    typed_spelling_checked(field, text).unwrap_or(Scalar::Null)
}

/// The same conversion with its typed refusal preserved for root invariants.
pub(super) fn typed_spelling_checked(field: &Field, text: &str) -> Result<Scalar> {
    let translated = field.as_fix().code_value(text);
    typed_translation(field, text, translated)
}

/// [`typed_spelling`], the translation already made: `translated` is the wire
/// value the field's code set gives `text` at `at`, or nothing where the set
/// gives none, exactly as the builder's own table answers it.
fn typed_translation(field: &Field, text: &str, translated: Option<&str>) -> Result<Scalar> {
    let view = field.as_fix();
    let spelling = translated.unwrap_or(text);
    // A state is read through the name the field gives its code before
    // the code itself, because two fields share a letter and not a
    // meaning: `D` is Restated as an `ExecType` and AcceptedForBidding as
    // an `OrdStatus`. The code answers where the dictionary names none.
    if matches!(field.dtype(), DataType::State) {
        if let Some(state) = view.code_name(spelling).and_then(State::from_spelling) {
            return Ok(Scalar::Code(Code::State(state)));
        }
    }
    // Every wire value is text, and the generic value contract does not
    // read text as a number, an instant or a flag. Two of those it can
    // learn from the field alone, which is the crate's own coercion; the
    // third is a FIX *spelling* and stays here, because the generic
    // contract must not learn one.
    // A FIX spelling is rewritten into the one the crate's coercion reads,
    // and then coerced like any other text: `20240102-10:15:30` is a
    // timestamp only after both steps, and the value contract reads
    // neither a separator-free instant nor a bare `Y`.
    let candidate =
        wire_spelling(field.dtype(), spelling).unwrap_or_else(|| Scalar::from(spelling));
    // The text contract reads the spelling and hands the value through
    // the field's own contract, so what it answers is already the stored
    // form and is not checked a second time. A spelling it refuses is
    // offered to the field as it stands, which is where a raw payload
    // that is not a spelling of anything still lands.
    crate::text::prepare_text(candidate, field).or_else(|_| field.scalar(Scalar::from(spelling)))
}

fn invalid_value(field: &Field, error: impl std::fmt::Display) -> Error {
    Error::InvalidRecord {
        path: crate::path::Path::root()
            .field(field.name())
            .render()
            .into(),
        reason: format_smolstr!("{}", crate::text::elide_display(&error)),
    }
}

/// Carrier/composed text shares the wire converter; native values must
/// already have the declared critical layout rather than coerce into it.
pub(super) fn typed_fill(field: &Field, tag: i32, value: &Scalar) -> Result<Scalar> {
    super::identity::validate_field(field, tag)?;
    if value.is_null() {
        return Ok(Scalar::Null);
    }
    if let Some(text) = value.as_str() {
        if field.as_fix().is_null_value(text) {
            return Ok(Scalar::Null);
        }
        return typed_spelling_checked(field, text).map_err(|error| invalid_value(field, error));
    }
    super::identity::validate_value(field.name(), field.dtype(), value)?;
    Ok(value.clone())
}

/// One registry field as this message carried it.
///
/// The dictionary's field says what a tag is; only the nullability is this
/// message's own, and it is false because the value is there.
pub(super) fn stated(known: &Field) -> Field {
    let mut field = known.clone();
    field.set_nullable(false);
    field
}

/// The child of one declared level that a key names, when it names one, and
/// the tag that child carries.
///
/// Folded exactly as the dictionary folds a name, so a key resolves the same
/// way whichever of the two answered it. Only a scalar child carrying a tag
/// answers: a nested level is addressed by its own located key and never by a
/// leaf, and a child no tag identifies explains a key no better than the key
/// explains itself.
fn in_scope<'held>(scope: &'held [Field], key: &str) -> Option<(&'held Field, i32)> {
    scope.iter().find_map(|held| {
        if held.dtype().is_nested() || !crate::types::folds_equal(held.name(), key) {
            return None;
        }
        Some((held, held.as_fix().tag().ok()??))
    })
}

/// The members one repeating group declares, as a level a key resolves in.
fn member_fields(group: &Field) -> &[Field] {
    super::schema::item_fields(group).unwrap_or_default()
}

/// The tags one repeating group declares as its direct members, the first
/// being the delimiter that opens an occurrence.
fn declared_members(group: &Field) -> Vec<i32> {
    member_fields(group)
        .iter()
        .filter_map(|member| member.as_fix().tag().ok().flatten())
        .collect()
}

/// The `BeginString` child a built message carries when its line stated
/// none: the dictionary's own field, non-null, or a text field carrying the
/// tag where the dictionary has none.
pub(super) fn beginstring_field(registry: &FixRegistry) -> Field {
    registry.get_field_by_tag(8).map_or_else(
        || {
            let mut field = DataType::utf8().required_field("beginstring");
            let _ = field.as_fix_mut().set_tag(8);
            field
        },
        stated,
    )
}

/// Whether a datatype is one the FIX layer reads raw wire bytes into.
///
/// A `data` field's value is bytes and the row is where they live, so the
/// typed read hands them over untouched instead of reading a spelling.
const fn is_binary(dtype: &DataType) -> bool {
    matches!(dtype, DataType::Bytes(_))
}

/// One unknown key's own spelling, folded the way every built name is.
///
/// The entry keeps the arrival casing, because the entries are the wire
/// record and the row is the interpretation; this is the interpretation's
/// side of that split. The fold is the crate's one name fold; a key the fold
/// empties - separators alone - keeps its own spelling, because a child has
/// to be called something.
fn folded_name(key: &str) -> String {
    let name = crate::types::normalized(key);
    if name.is_empty() {
        key.to_owned()
    } else {
        name
    }
}

/// The message type a built row is named by.
///
/// A message is typed by `35=` in a FIX frame or `MSGTYPE=` in a bridge one.
/// Where a row carries neither it is built anyway and named `unknown`: every
/// pair that parsed becomes a field, the entries record the whole row, and
/// nothing is dropped. This fallback names the row only; it does not register
/// a message type or supply a value for the missing tag.
pub(super) const UNKNOWN_MSGTYPE: &str = "unknown";

/// The root name one message type gives a row.
pub(super) fn root_name(msgtype: Option<&str>) -> SmolStr {
    msgtype.map_or_else(
        || SmolStr::new_static(UNKNOWN_MSGTYPE),
        |value| format_smolstr!("{value}"),
    )
}
