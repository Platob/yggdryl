//! The FIX registry: one field vector and five compact indexes over one
//! namespace.
//!
//! Identities, canonical tags and alternate tags are their own keys. Canonical
//! names and aliases are keyed by independent seeded XXH64 digests of the
//! crate's one fold; every hit is rechecked against the field, so a digest
//! collision is a miss on read and a typed conflict on mutation. Ordered
//! iteration is kept separately as field positions sorted tag-major, the
//! tag's holder first, then by identity - so a store that writes in this
//! order and loads in file order hands the bare tag back to the field that
//! held it. The registry is built rarely and resolved constantly, so that
//! `O(n)` insertion trade is deliberate.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::iter::FusedIterator;
use std::sync::{Arc, OnceLock};

use smol_str::{SmolStr, format_smolstr};

use super::source::FixSource;
use super::{FixId, FixKey};
use crate::folds_equal;
use crate::xxhash::Xxh64;
use crate::{Error, Field, FieldPath, FieldSegment, IOBase, Result};

const NAME_SEED: u64 = 0x4e41_4d45_5f46_4958;
const ALIAS_SEED: u64 = 0x414c_4941_535f_4649;

// Callers first establish the same field/definition identity.
pub(super) fn metadata_only_change(stored: &Field, incoming: &Field) -> bool {
    stored.name() == incoming.name()
        && stored.dtype() == incoming.dtype()
        && stored.is_nullable() == incoming.is_nullable()
        && stored.as_metadata() != incoming.as_metadata()
}

/// Finalize integer keys before hashbrown selects a control byte.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Mix(u64);

impl Mix {
    const fn finalise(mut value: u64) -> u64 {
        value ^= value >> 33;
        value = value.wrapping_mul(0xff51_afd7_ed55_8ccd);
        value ^= value >> 33;
        value
    }

    /// Folds one integer write into the state instead of replacing it.
    ///
    /// A key written in parts keeps every part: the rotation lands two
    /// 32-bit writes in the two halves of the state, so a key made of two
    /// integers is two keys rather than one bucket. A key written once is
    /// unchanged by the fold, the state being zero until then.
    fn fold(&mut self, value: u64) {
        self.0 = self.0.rotate_left(32) ^ value;
    }
}

/// The field a dotted path reaches under one resolved head, folding as it goes.
///
/// The generic walk matches a child's name exactly, which is right for a schema
/// a caller wrote and wrong for a dictionary: the head already folded, so
/// `Parties.PartyID` resolving its first segment and refusing its second is
/// one function disagreeing with itself.
///
/// A serie is stepped through before anything else, including the exact walk.
/// A repeating group's occurrence is not a path segment - nobody spelling a
/// path names it - so consulting [`Field::get_field_by_path`] first would let
/// the occurrence match by its own name, which is exactly what this walk must
/// not allow now that the occurrence carries the component's name. The exact
/// walk still runs under the serie, because it is the cheap answer and the
/// common one.
pub(super) fn descend<'field>(
    field: &'field Field,
    segments: &[FieldSegment],
) -> Option<&'field Field> {
    let Some((head, rest)) = segments.split_first() else {
        return Some(field);
    };
    if let crate::DataType::Serie(item) | crate::DataType::LargeSerie(item) = field.dtype() {
        // A group's occurrence is transparent in a schema: every one of them
        // has the field the item declares, so an index states which
        // occurrence a caller means without changing which field that is.
        let rest = if matches!(head, FieldSegment::Index(_)) {
            rest
        } else {
            segments
        };
        return descend(item, rest);
    }
    if let Some(entries) = field.dtype().map_entries() {
        let FieldSegment::Key(_) = head else {
            return None;
        };
        return descend(entries.fields().get(1)?, rest);
    }
    let child = folded_child(field, segment_name(head)?)?;
    descend(child, rest)
}

/// The name a segment states, where it states one.
///
/// A schema is addressed by name: a position says which occurrence of a group
/// a caller means, and every occurrence holds the same field, so a position
/// names no child here and is spent by the serie it stands on.
fn segment_name(segment: &FieldSegment) -> Option<&str> {
    segment.as_name()
}

/// One child by folded name, reaching through a group's occurrence.
///
/// A repeating group is a Serie of one Struct, so a member is that struct's
/// child and not the serie's. The occurrence is transparent: it is recursed
/// through without consuming a segment and it is never matched by its own
/// name, because that name is the component's - `Parties.PartyID` names
/// tag 448 and must never answer the struct that happens to share its
/// spelling. 269 of the 521 shipped groups derive a name a member of their own
/// struct already carries, so matching the occurrence would shadow every one
/// of them silently.
fn folded_child<'field>(field: &'field Field, name: &str) -> Option<&'field Field> {
    if let crate::DataType::Serie(item) | crate::DataType::LargeSerie(item) = field.dtype() {
        return folded_child(item, name);
    }
    field
        .fields()
        .iter()
        .find(|held| folds_equal(held.name(), name))
}

#[cfg(feature = "internals")]
pub(super) fn control_byte(id: FixId) -> u8 {
    let mut state = Mix::default();
    std::hash::Hash::hash(&id, &mut state);
    (state.finish() >> 57) as u8
}

impl Hasher for Mix {
    fn finish(&self) -> u64 {
        Self::finalise(self.0)
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut value = self.0;
        for chunk in bytes.chunks(8) {
            let mut word = [0_u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            value = value.rotate_left(17) ^ u64::from_ne_bytes(word);
        }
        self.0 = value;
    }

    fn write_u32(&mut self, value: u32) {
        self.fold(u64::from(value));
    }

    fn write_i32(&mut self, value: i32) {
        self.fold(value as u32 as u64);
    }

    fn write_u64(&mut self, value: u64) {
        self.fold(value);
    }

    fn write_i64(&mut self, value: i64) {
        self.fold(value as u64);
    }
}

type Index<K> = HashMap<K, usize, BuildHasherDefault<Mix>>;

/// A map keyed by something already spread over its bits.
///
/// The dictionary's own hasher, for every index in the FIX layer that keys
/// on a tag, a digest or an identity: those keys are integers a digest or
/// the wire already spread, and SipHash would rehash what is hashed. The
/// name-keyed maps keep the default, which is what an unspread key needs.
pub(super) type FixMap<K, V> = HashMap<K, V, BuildHasherDefault<Mix>>;

/// Fold a name directly into a seeded streaming state.
///
/// The crate's one fold: ASCII case folded, and `_`, `-` and space dropped.
/// A renderer emitting `msg_type`, `msg-type` or `Msg Type` therefore finds
/// the field `MsgType` names, which is what makes storing the folded name
/// cost no caller the spelling it was written with. No two FIX fields differ
/// only by a separator or by case, which is what lets the fold in at all.
///
/// It folds into the hash state in stack-sized chunks, so no length of name
/// allocates.
pub(super) fn name_digest(name: &str, domain: u64) -> u64 {
    let mut state = Xxh64::with_seed(domain);
    let mut folded = [0_u8; 64];
    let mut held = 0;
    for byte in name.as_bytes() {
        if matches!(byte, b'_' | b'-' | b' ') {
            continue;
        }
        folded[held] = byte.to_ascii_lowercase();
        held += 1;
        if held == folded.len() {
            state.write(&folded);
            held = 0;
        }
    }
    state.write(&folded[..held]);
    state.finish()
}

/// The key one name is indexed under.
///
/// A dictionary holds one field per name, and this is what decides which
/// two spellings are one name: the crate's fold. [`super::cfb`] settles a
/// CBlock's contended spellings against exactly this rather than against the
/// spelling, so a file the parser leaves named is a file this dictionary
/// accepts.
pub(super) fn name_key(name: &str) -> u64 {
    name_digest(name, NAME_SEED)
}

enum Held<'a> {
    Id(i32, &'a str),
    AlternateTag(i32),
    Name(&'a str),
    Alias(&'a str),
}

impl fmt::Display for Held<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(tag, name) => write!(formatter, "identity {tag} {name:?}"),
            Self::AlternateTag(tag) => write!(formatter, "alternate tag {tag}"),
            Self::Name(name) => write!(formatter, "name {name:?}"),
            Self::Alias(alias) => write!(formatter, "alias {alias:?}"),
        }
    }
}

fn conflict(held: Held<'_>, incoming: &Field, holder: &Field) -> Error {
    Error::conflict(
        "fix field",
        "fix field",
        format_smolstr!("{held} of {}, held by {}", incoming.name(), holder.name()),
    )
}

fn absent(what: impl fmt::Display) -> Error {
    Error::absent("fix field", what)
}

/// The refusal a merge raises when the incoming datatype is not the stored
/// one: merging metadata never changes a field's declared datatype.
/// Whether two declarations of one field's datatype say one thing at two
/// precisions, so a fold keeps the stored one and folds the source's field
/// under it rather than passing the field over.
///
/// Every FIX datatype derives from String - the wire is text - so a source
/// declaring a field as unbounded text has said less than the dictionary,
/// not something else; a CBlock's `string` and `char` are exactly that, and
/// they meet a stored `ccy`, `mic`, `side`, `boolean`, `datetime64` and
/// `binary` as a coarser statement of each. The numeric families are one
/// statement too: the specification derives `Qty`, `Price` and `Amt` from
/// `float` and states no width anywhere, and a CBlock's `integer` and
/// `float` name a family rather than a width, so an int32 beside an int64, a
/// float64 beside a decimal128 and an integer beside either are two
/// precisions of one number. An enum leaf stores the int32 code of its
/// member, so an integer restates it; the byte layouts are one byte string;
/// a date and a datetime are one instant, a FIX date being that day's
/// midnight in whichever zone the dictionary states, and the time-of-day
/// widths one time. Everything else - a boolean against an integer, a time
/// against a timestamp, two codes, two bounded strings of different widths -
/// is a contradiction and stays passed over.
fn datatypes_agree(stored: &crate::DataType, incoming: &crate::DataType) -> bool {
    use crate::DataTypeKind as Kind;
    if stored == incoming {
        return true;
    }
    let unbounded_text = |dtype: &crate::DataType| {
        dtype
            .string_parameters()
            .is_some_and(|leaf| leaf.bound().is_none())
    };
    if unbounded_text(stored) || unbounded_text(incoming) {
        return true;
    }
    match (stored.kind(), incoming.kind()) {
        (
            Kind::Integer | Kind::Floating | Kind::Decimal,
            Kind::Integer | Kind::Floating | Kind::Decimal,
        )
        | (Kind::Enum, Kind::Integer)
        | (Kind::Integer, Kind::Enum)
        | (Kind::Bytes, Kind::Bytes) => true,
        (Kind::Temporal, Kind::Temporal) => {
            let instant = |family: Option<&str>| matches!(family, Some("date" | "datetime"));
            let (held, declared) = (
                stored.id().temporal_family(),
                incoming.id().temporal_family(),
            );
            held == declared || (instant(held) && instant(declared))
        }
        _ => false,
    }
}

/// One `CBlock` file a fold reads: a location the caller handed over, or a
/// file a glob or a folder listed.
enum CblockFile<'location> {
    Given(&'location crate::holder::Holder),
    Listed(Box<crate::holder::Holder>),
}

impl CblockFile<'_> {
    fn as_io(&self) -> &dyn IOBase {
        match self {
            Self::Given(location) => location.as_io(),
            Self::Listed(file) => file.as_io(),
        }
    }
}

/// Whether the leaf a URL names is called `*.cfb`, the suffix in any case:
/// the member an archive URL's fragment names, else the file its path does.
fn is_cblock_name(url: &crate::Url) -> bool {
    let member = url.fragment(true).ok().flatten();
    let name = match &member {
        Some(member) => member.rsplit('/').next(),
        None => url.file_name(),
    };
    name.and_then(|name| name.rsplit_once('.'))
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("cfb"))
}

/// The `CBlock` files one location holds, appended to `files`: what a glob
/// matches, the `.cfb` files directly inside a folder, or the location
/// itself where it is a file - and nothing where nothing is there, which is
/// logged at warn level, because a location beside others that hold files
/// would otherwise add nothing without a word.
fn cblock_files<'location>(
    location: &'location crate::holder::Holder,
    files: &mut Vec<CblockFile<'location>>,
) -> Result<()> {
    let before = files.len();
    if !location.is_container() {
        if location.kind() != crate::IOKind::Unknown {
            files.push(CblockFile::Given(location));
        }
    } else {
        // A pattern is the caller's filter; a folder's is the suffix, read
        // off the entry's own name - an archive member's is in the fragment.
        let glob = location.url().is_some_and(crate::Url::is_glob);
        for entry in location.ls(false, false) {
            let entry = entry?;
            let cblock = glob || entry.url().is_some_and(is_cblock_name);
            if cblock && !entry.is_container() {
                files.push(CblockFile::Listed(Box::new(entry)));
            }
        }
    }
    if files.len() == before {
        match location.url() {
            Some(url) => log::warn!("{url}: holds no .cfb file, so it adds nothing"),
            None => log::warn!("a location holds no .cfb file, so it adds nothing"),
        }
    }
    Ok(())
}

fn datatype_disagreement(stored: &Field, tag: i32, incoming: &Field) -> Error {
    // A time of day against an instant is the one contradiction two FIX
    // spellings of one field routinely make - a CBlock has `utc-time-only`
    // and no word for `TZTimeOnly` - so the refusal says what each reading
    // accepts rather than leaving a reader to guess why two clocks disagree.
    let consequence = match (
        stored.dtype().id().temporal_family(),
        incoming.dtype().id().temporal_family(),
    ) {
        (Some("datetime"), Some("time"))
            if stored.as_fix().shape() == super::field::FixShape::TzTimeOnly =>
        {
            ": the stored instant reads a clock on the epoch day, at the offset it states or else as a wall clock in the column's zone"
        }
        (Some("datetime"), Some("time")) => {
            ": the stored instant reads a clock only with its offset, a bare time of day as null"
        }
        (Some("time"), Some("datetime")) => ": the stored time of day reads no date and no offset",
        _ => "",
    };
    Error::InvalidRecord {
        path: incoming.name().into(),
        reason: format_smolstr!(
            "{}{consequence}",
            crate::text::expected_got(
                format_args!(
                    "the datatype {} stored for {} ({tag})",
                    stored.dtype(),
                    stored.name()
                ),
                incoming.dtype(),
            )
        ),
    }
}

/// When a change to a field is re-resolved into the definitions that
/// reference it.
///
/// A named definition holds its referenced fields resolved, so a field whose
/// metadata moved leaves them stale until the catalog is resolved again.
/// One change on its own settles at once; a fold of many settles once, at
/// its end, because re-resolving the whole catalog per field makes a fold
/// of a source the size of a dictionary quadratic in it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum References {
    /// Re-resolve them now.
    Refresh,
    /// The caller re-resolves them once every change is in.
    Defer,
}

/// Whose name a fold of one field into another keeps.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Naming {
    /// The stored field's, which is every identity fold.
    Stored,
    /// The incoming field's: the stored field was named by nothing but its
    /// decimal tag and learns what the source calls it.
    Incoming,
}

/// Whether a field is named by nothing but its own decimal tag.
///
/// A source that knows a tag exists but not what anyone calls it - a CBlock
/// declaring a `vocabulary-tag` with no `alt`, or two tags under one `alt`,
/// which names neither - names the field after the tag, and that is a
/// placeholder rather than a name: the field is unnamed, the way a code named
/// after its own wire value is, and a fold is where a real name takes its
/// place.
pub(super) fn is_unnamed(field: &Field) -> bool {
    field
        .as_fix()
        .tag()
        .ok()
        .flatten()
        .is_some_and(|tag| super::field::parse_tag(field.name()) == Some(tag))
}

/// Where one incoming scalar lands under [`FixRegistry::add_field`]'s rules.
#[derive(Clone, Copy)]
enum Route {
    /// One of the crate's own fields: every dictionary's already.
    Own,
    /// The identity the field at this position holds: rule 3.
    Identity(usize),
    /// The tag the field at this position holds under no name of its own:
    /// rule 3, the holder taking the arrival's name.
    Unnamed(usize),
    /// A tag another field holds under another name, the arrival's own name
    /// no field's canonical name: rule 5, beside it.
    Beside,
    /// A name the field at this position answers to: rule 4.
    Named(usize),
    /// Nothing held answers to it: rule 5.
    New,
}

impl Route {
    /// The stored field the incoming one would fold into.
    const fn target(self) -> Option<usize> {
        match self {
            Self::Identity(position) | Self::Unnamed(position) | Self::Named(position) => {
                Some(position)
            }
            Self::Own | Self::Beside | Self::New => None,
        }
    }
}

/// What one fold of another source into a dictionary did.
///
/// The answer of [`FixRegistry::merge_with`] and every door over it:
/// [`FixRegistry::add_cfb_file`], [`FixRegistry::add_cfb_files`] and
/// [`FixRegistry::add_json_file`]. The counts are the scalars'; the
/// definitions fold beside them under their own rules. `restated` counts
/// the merged fields whose source declared the datatype at another
/// precision than the dictionary holds - a CBlock's `float` against a
/// stored `decimal128`, its `string` against a stored `ccy` - each folded
/// under the stored declaration. `dropped` is what a source said that the
/// dictionary already says otherwise: the fold keeps the dictionary's
/// declaration, passes the source's over, and names it here rather than
/// refusing the whole source for it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FixMerge {
    /// The sources folded: one for a dictionary or a file, every file the
    /// locations held that folded for [`FixRegistry::add_cfb_files`].
    pub sources: usize,
    /// The scalar fields that arrived.
    pub added: usize,
    /// The scalar fields folded into ones already held.
    pub merged: usize,
    /// Among the merged, the fields whose source declared another precision
    /// of the stored datatype and folded under the stored one.
    pub restated: usize,
    /// The declarations passed over, in the order the fold met them.
    pub dropped: Vec<FixDrop>,
    /// The sources left out whole, in the order the fold met them: a file
    /// [`FixRegistry::add_cfb_files`] could not read, parse or fold, which
    /// contributed nothing while every other file still folded.
    pub failed: Vec<FixFailure>,
}

impl FixMerge {
    /// Whether the fold kept every declaration its sources made.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.dropped.is_empty() && self.failed.is_empty()
    }

    /// Adds another fold's counts and drops to this one's.
    fn absorb(&mut self, other: Self) {
        self.sources += other.sources;
        self.added += other.added;
        self.merged += other.merged;
        self.restated += other.restated;
        self.dropped.extend(other.dropped);
        self.failed.extend(other.failed);
    }

    /// Names the source every drop was read from.
    pub(super) fn located(mut self, handle: &dyn IOBase) -> Self {
        if let Some(url) = handle.url() {
            let source = format_smolstr!("{url}");
            for drop in &mut self.dropped {
                drop.source = Some(source.clone());
            }
        }
        self
    }
}

/// One declaration a fold passed over, because the dictionary already
/// declares the same thing otherwise.
///
/// A scalar whose datatype contradicts the field its identity or name
/// reaches - a datatype declared at another precision of the stored one is
/// no contradiction, and folds under it - a member a definition already
/// holds in another shape, a definition whose own identity - its counter,
/// component or message code - disagrees with the one held under its name,
/// and a member reading a field the fold passed over. The dictionary's
/// declaration is what stays: merging never changes a declared datatype, and
/// a source is never the reason a dictionary forgets what it said.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixDrop {
    /// Where the declaration was read, where the fold read a file: its URL.
    pub source: Option<SmolStr>,
    /// The declaration as the source stated it: the scalar field, or the
    /// member or definition in the compact shape a store writes, a
    /// reference standing for its target.
    pub incoming: Field,
    /// The refusal the fold would otherwise have ended on, naming what the
    /// dictionary keeps.
    pub reason: SmolStr,
}

impl FixDrop {
    pub(super) fn new(incoming: Field, reason: &Error) -> Self {
        log::debug!("fix merge passed over {}: {reason}", incoming.name());
        Self {
            source: None,
            incoming,
            reason: format_smolstr!("{reason}"),
        }
    }
}

impl fmt::Display for FixDrop {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(source) = &self.source {
            write!(formatter, "{source}: ")?;
        }
        formatter.write_str(&self.reason)
    }
}

/// One source a plural fold left out whole.
///
/// [`FixRegistry::add_cfb_files`] folds each file as one mutation of its own:
/// a file that cannot be read, is not a well-formed CBlock, or whose fold
/// refuses rather than passing a declaration over is rolled back alone and
/// named here, and every other file still folds. Nothing the file said is
/// held, so the reason is the whole of what it contributed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixFailure {
    /// The file's URL, where the source is a file with one.
    pub source: Option<SmolStr>,
    /// The refusal the file was left out over.
    pub reason: SmolStr,
}

impl FixFailure {
    fn new(handle: &dyn IOBase, error: &Error) -> Self {
        log::debug!("fix merge left out {:?}: {error}", handle.url());
        Self {
            source: handle.url().map(|url| format_smolstr!("{url}")),
            reason: format_smolstr!("{error}"),
        }
    }
}

impl fmt::Display for FixFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(source) = &self.source {
            write!(formatter, "{source}: ")?;
        }
        formatter.write_str(&self.reason)
    }
}

/// The identity a field enters the registry under, beside its tag.
pub(super) fn canonical_identity(field: &Field) -> Result<(i32, FixId)> {
    let view = field.as_fix();
    let tag = view
        .tag()?
        .ok_or_else(|| Error::absent("FIX:tag", field.name()))?;
    Ok((tag, FixId::of(tag, field.name())?))
}

/// The identity a field enters the registry under.
pub(super) fn canonical_id(field: &Field) -> Result<FixId> {
    canonical_identity(field).map(|(_, id)| id)
}

/// What one registry field's metadata states that every reader of a
/// message asks for, read once when the field is indexed and answered
/// without a metadata read after.
///
/// A message's child is stated under the dictionary's own field, sharing
/// its metadata, so what the field states the child states: the tag it
/// carries, the group it counts and whether the specification retired
/// its tag. A field one of these does not read as
/// its own - a stored tag that is not a tag - states no facts, and a
/// reader asks the metadata as it always did.
#[derive(Clone, Copy, Debug)]
pub(super) struct FieldFacts {
    /// `FIX:tag`.
    pub(super) tag: Option<i32>,
    /// `FIX:counter`, on a group: the NumInGroup tag that frames it on the
    /// wire and keys it, never a field beside it.
    pub(super) counter: Option<i32>,
    /// Whether a replacement rule could restate the field: one the
    /// specification's [retirements](super::retired) state of its tag.
    pub(super) ruled: bool,
}

impl FieldFacts {
    pub(super) fn of(field: &Field) -> Option<Self> {
        let view = field.as_fix();
        let tag = view.tag().ok()?;
        Some(Self {
            tag,
            counter: view.counter().ok()?,
            ruled: tag.is_some_and(|tag| super::retired::rules_of(tag).is_some()),
        })
    }
}

/// FIX field definitions resolved by identity, tag or folded name.
pub struct FixRegistry {
    fields: Vec<Field>,
    pub(super) catalog: super::catalog::Catalog,
    /// The [code sets](super::codes) this dictionary holds, by folded name.
    ///
    /// One owner per vocabulary: a field's `FIX:codeset` names the set it
    /// draws on and the document lives here once, however many fields read
    /// by it - 103 of them for one offset-unit set in the shipped dictionary,
    /// and 2,026 fields over 735 sets in all. Held
    /// behind an `Arc` because every clone of a registry, and every staged
    /// copy a mutation makes, shares the documents rather than copying
    /// them; ordered because the store, the snapshot and the hash all read
    /// them in one order.
    pub(super) codesets: BTreeMap<SmolStr, Arc<str>>,
    /// The [sources](super::source) this dictionary was built from, by id:
    /// one entry per id a field's `FIX:sources` names, holding what is known
    /// of the source - the file it was read from - once rather than on every
    /// field. Ordered because the store, the snapshot and the hash all read
    /// them in one order; part of equality and of the hash, as the code
    /// sets are.
    pub(super) sources: BTreeMap<SmolStr, FixSource>,
    ids: Index<FixId>,
    /// A canonical tag, and the first field that held it: a bare wire tag
    /// answers that field, and a later field on the same tag under another
    /// name is reached by its name or its identity.
    tags: Index<i32>,
    alternate_tags: Index<i32>,
    names: Index<u64>,
    aliases: Index<u64>,
    positions_by_id: Vec<usize>,
    /// Each field's canonical tag and identity, by position: what the maps
    /// answer a position with, read back without the metadata reads the
    /// field's own view costs. Kept in step by [`Self::index`], which runs
    /// after every change to `fields`.
    identities: Vec<Option<(i32, FixId)>>,
    /// What each field's metadata states, by position, read when the field
    /// is indexed; and each field's position by the address of its
    /// metadata storage, which a message's child stated under the field
    /// shares. Kept in step by [`Self::index`] and [`Self::unindex`].
    facts: Vec<Option<FieldFacts>>,
    by_metadata: FixMap<usize, usize>,
    /// What this dictionary has answered about itself - a key's field, a
    /// field's null spellings, a code's translation - shared by every codec
    /// and every row read against it, and forgotten by every change to the
    /// fields.
    memo: super::memo::Memo,
    /// The currency pair each symbol names, read once per spelling and shared
    /// by every codec and message reading this registry: a fact of the
    /// symbol text alone, so no change to the fields forgets it.
    forex: super::forex::FxMemo,
    /// The names a message digests its lifted facts under, read off the
    /// fields once on the first digest and forgotten with the memo.
    lifted_names: OnceLock<LiftedNames>,
    /// Every field's `FIX:idmap`, read once on the first settle and
    /// forgotten with the memo.
    idmap_sources: OnceLock<Vec<(i32, super::FixIdSource)>>,
    /// Every field's `FIX:parents`, under the identifier type the field
    /// states, read once and forgotten with the memo.
    parents: OnceLock<Vec<(crate::IdType, Box<[crate::IdType]>)>>,
    /// Every field's `FIX:marketdatatype` pairs, read once.
    marketdatatypes: OnceLock<Vec<(i32, SmolStr, crate::MarketDataType)>>,
    /// Every wire value a field maps to a time in force, read once.
    timeinforces: OnceLock<Vec<(i32, SmolStr, crate::TimeInForce)>>,
    /// The fields a fold renamed since the catalog last resolved - a field
    /// named by nothing but its tag that learnt what a source calls it - by
    /// the identity they held, so the members reading them under it are
    /// rewritten to the identity they hold now when the catalog resolves.
    /// Empty whenever the catalog is resolved.
    pub(super) renamed: HashMap<FixId, Option<(i32, SmolStr)>>,
}

/// The names [`FixMsg`](super::FixMsg) feeds its typed facts under when it
/// digests itself: the dictionary's name for each lifted tag, the tag's
/// decimal spelling where the dictionary lacks it, and the `MsgType` name.
pub(super) struct LiftedNames {
    pub(super) msgtype: SmolStr,
    /// One per tag of [`identity::LIFTED_TAGS`](super::identity::LIFTED_TAGS), in its order.
    pub(super) lifted: Vec<SmolStr>,
}

impl Default for FixRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FixRegistry {
    /// The deterministic hash of fields and named definitions.
    /// Uses one allocation for the shared XXH3 state, independent of catalog size.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_of(self)
    }

    /// A registry holding the crate definitions and standard clock fields.
    ///
    /// Every registry starts here: the fields this crate defines - the
    /// digest, the clock, the partition, the bridge's session and context -
    /// are what a row is typed by, so a dictionary loaded from a store,
    /// built from fields or left empty holds them alike. Inserting them into
    /// nothing cannot collide, and a build failure of the crate's own fields
    /// is a defect [`fix_crate_fields`](super::fix_crate_fields) reports;
    /// here construction and registration failures are logged with their
    /// definition and typed error. Scalar and group definitions follow the
    /// same category rules as caller-owned definitions.
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Self::base();
        if let Err(error) = registry.seed_clocks() {
            log::warn!("registering FIX standard clocks: {error}");
        }
        registry
    }

    /// A store loads its definitions before filling absent standard clocks.
    pub(super) fn base() -> Self {
        let mut registry = Self {
            fields: Vec::new(),
            catalog: super::catalog::Catalog::default(),
            codesets: BTreeMap::new(),
            sources: BTreeMap::new(),
            ids: Index::default(),
            tags: Index::default(),
            alternate_tags: Index::default(),
            names: Index::default(),
            aliases: Index::default(),
            positions_by_id: Vec::new(),
            renamed: HashMap::new(),
            identities: Vec::new(),
            facts: Vec::new(),
            by_metadata: FixMap::default(),
            memo: super::memo::Memo::new(),
            forex: super::forex::FxMemo::new(),
            lifted_names: OnceLock::new(),
            idmap_sources: OnceLock::new(),
            parents: OnceLock::new(),
            marketdatatypes: OnceLock::new(),
            timeinforces: OnceLock::new(),
        };
        if let Some(document) = super::crated::marketdatakind_codeset() {
            registry.codesets.insert(
                SmolStr::new_static(super::crated::MARKETDATAKIND_CODESET_NAME),
                document,
            );
        }
        if let Some(document) = super::crated::state_codeset() {
            registry.codesets.insert(
                SmolStr::new_static(super::crated::STATE_CODESET_NAME),
                document,
            );
        }
        if let Some(document) = super::crated::marketdatatype_codeset() {
            registry.codesets.insert(
                SmolStr::new_static(super::crated::MARKETDATATYPE_CODESET_NAME),
                document,
            );
        }
        if let Some(document) = super::crated::msgpluginside_codeset() {
            registry.codesets.insert(
                SmolStr::new_static(super::crated::MSGPLUGINSIDE_CODESET_NAME),
                document,
            );
        }
        match super::fix_crate_fields() {
            Ok(fields) => {
                // A derived column is the fixed row's, never a field a
                // message states, so no registry files it.
                let registered = fields.iter().filter(|field| {
                    !field
                        .as_fix()
                        .tag()
                        .ok()
                        .flatten()
                        .is_some_and(super::crated::is_derived_tag)
                });
                for field in registered {
                    let category = super::catalog::definition_category(field)
                        .unwrap_or(crate::FixCategory::Fields);
                    if let Err(error) = registry.insert_definition(category, field.clone()) {
                        log::warn!("registering FIX crate definition {}: {error}", field.name());
                    }
                }
            }
            Err(error) => log::warn!("registering FIX crate definitions: {error}"),
        }
        registry
    }

    /// Standard clocks use the ordinary indexes and remain caller-owned.
    /// A loaded definition supplies its own metadata rather than colliding
    /// with a seed; duplicate stored definitions still use `create_definition`.
    pub(super) fn seed_clocks(&mut self) -> Result<()> {
        for (tag, name, display) in [
            (52, "sendingtime", "SendingTime"),
            (60, "transacttime", "TransactTime"),
        ] {
            if self.get_field_by_tag(tag).is_some() {
                continue;
            }
            let mut field = super::schema::CLOCK_DATATYPE.nullable_field(name);
            field.as_fix_mut().set_tag(tag)?;
            field.set_display(display)?;
            self.insert(field)?;
        }
        Ok(())
    }

    /// Builds a registry by inserting `fields` in order.
    pub fn from_fields<I>(fields: I) -> Result<Self>
    where
        I: IntoIterator<Item = Field>,
    {
        let mut registry = Self::new();
        for field in fields {
            registry.insert(field)?;
        }
        Ok(registry)
    }

    /// Returns the field one identity names: a scalar field by its
    /// canonical identity, else a component or a group by the identity its
    /// derived tag and name make.
    pub fn get_field_by_id(&self, id: FixId) -> Option<&Field> {
        self.canonical_position_by_id(id)
            .and_then(|position| self.fields.get(position))
            .or_else(|| {
                self.catalog
                    .all()
                    .map(|entry| entry.field.as_field())
                    .find(|field| {
                        field
                            .as_fix()
                            .id()
                            .ok()
                            .flatten()
                            .is_some_and(|held| held == id)
                    })
            })
    }

    /// Returns the field an identity names, raising absence.
    pub fn field_by_id(&self, id: FixId) -> Result<&Field> {
        self.get_field_by_id(id)
            .ok_or_else(|| absent(FixKey::Id(id)))
    }

    /// Returns the field a canonical or alternate tag names.
    ///
    /// A canonical tag before an alternate. A tag two fields hold under
    /// different names answers the first of them; the other is reached by
    /// its name or its identity.
    pub fn get_field_by_tag(&self, tag: i32) -> Option<&Field> {
        self.position_by_tag(tag)
            .and_then(|position| self.fields.get(position))
            .or_else(|| {
                // A component or a group answers to the derived tag that is
                // its identity in the catalog.
                super::FixId::is_definition_tag(tag)
                    .then(|| {
                        self.catalog
                            .all()
                            .map(|entry| entry.field.as_field())
                            .find(|field| field.as_fix().tag().ok().flatten() == Some(tag))
                    })
                    .flatten()
            })
    }

    /// Returns the repeating group the counter `tag` opens; two groups on
    /// one counter name nothing.
    ///
    /// `tag` is the counter's, not the group's own: [`Self::get_field_by_tag`]
    /// answers the counter itself off the same key, and the group it heads
    /// is a definition of its own, reached here or by its name.
    pub fn get_field_by_counter(&self, tag: i32) -> Option<&Field> {
        self.get_group_by_tag(tag)
    }

    /// Returns the repeating group the counter `tag` opens, raising absence
    /// or ambiguity.
    pub fn field_by_counter(&self, tag: i32) -> Result<&Field> {
        self.group_by_tag(tag)
    }

    /// Returns the field a tag names, raising absence.
    pub fn field_by_tag(&self, tag: i32) -> Result<&Field> {
        self.get_field_by_tag(tag)
            .ok_or_else(|| absent(FixKey::Tag(tag)))
    }

    /// Returns the field a canonical name or alias names.
    ///
    /// A canonical name before an alias, both under the crate's one fold,
    /// so an alias can never take a name away from the field that claims it
    /// canonically; only a name neither reaches is read through the word
    /// aliases - `offer`/`ask`, `size`/`qty`, `bid`/`demand`, `px`/`price`,
    /// each way, anywhere in the folded name - and one reaching two fields
    /// that way reaches none.
    ///
    /// ```
    /// use yggdryl::{DataType, FixRegistry};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let field = |name: &str, tag: i32| -> yggdryl::Result<yggdryl::Field> {
    ///     let mut field = DataType::Decimal.nullable_field(name);
    ///     field.as_fix_mut().set_tag(tag)?;
    ///     Ok(field)
    /// };
    /// let registry = FixRegistry::from_fields([
    ///     field("BidPx", 132)?,
    ///     field("OfferPx", 133)?,
    ///     field("OfferSize", 135)?,
    /// ])?;
    /// let tag = |name: &str| {
    ///     registry
    ///         .get_field_by_name(name)
    ///         .and_then(|field| field.as_fix().tag().ok().flatten())
    /// };
    /// assert_eq!(tag("OfferPx"), Some(133));
    /// assert_eq!(tag("AskPx"), Some(133));
    /// assert_eq!(tag("AskSize"), Some(135));
    /// assert_eq!(tag("BidPrice"), Some(132));
    /// assert_eq!(tag("OfferPrice"), Some(133));
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_field_by_name(&self, name: &str) -> Option<&Field> {
        self.read_position_by_name(name)
            .and_then(|position| self.fields.get(position))
            .or_else(|| self.get_definition(crate::FixCategory::Components, name))
            .or_else(|| self.get_definition(crate::FixCategory::Groups, name))
    }

    /// The scalar field a canonical or alternate tag names, and nothing
    /// else: the catalog's own reading of its scalars, which a definition
    /// check asks so a definition never answers for itself.
    pub(super) fn get_scalar_by_tag(&self, tag: i32) -> Option<&Field> {
        self.position_by_tag(tag)
            .and_then(|position| self.fields.get(position))
    }

    /// The scalar field a canonical name or alias names exactly, and
    /// nothing else: the catalog's own reading, which a reference and a
    /// definition check ask, so no word alias ever stands in for the field
    /// a definition names.
    pub(super) fn get_scalar_by_name(&self, name: &str) -> Option<&Field> {
        self.position_by_name(name)
            .and_then(|position| self.fields.get(position))
    }

    /// Returns the field a name names, raising absence.
    pub fn field_by_name(&self, name: &str) -> Result<&Field> {
        self.get_field_by_name(name)
            .ok_or_else(|| absent(FixKey::Name(name)))
    }

    /// One message column: a canonical Map name precedes a scalar alias.
    pub(super) fn get_message_field_by_name(&self, name: &str) -> Option<&Field> {
        self.get_definition(crate::FixCategory::Groups, name)
            .filter(|group| matches!(group.dtype(), crate::DataType::Map(_) | crate::DataType::SortedMap(_)))
            .or_else(|| self.get_field_by_name(name))
            // Last, and only for a name nothing else answers: a Serie group is
            // reached by its own name - `Parties`, never `NoPartyIDs`, which
            // names the count beside it - so a message can be written one
            // whole, which is what lifting a group out of the arrival record
            // needs. A scalar of that name still wins, because a field the
            // dictionary publishes is what a caller spelling it means.
            .or_else(|| self.get_definition(crate::FixCategory::Groups, name))
    }

    /// One key read as a name, and as the path it spells where it spells one.
    ///
    /// A name costs no parse, which is what nearly every key is. A key
    /// holding more than one segment is a reading a caller wrote down, and
    /// reading it is this door's job rather than a second door's.
    fn named_or_path(&self, key: &str) -> Option<&Field> {
        if let Some(field) = self.get_field_by_name(key) {
            return Some(field);
        }
        let path = FieldPath::from_str(key).ok()?;
        (path.segments().len() > 1)
            .then(|| self.get_field_by_path(&path))
            .flatten()
    }

    /// Returns the field a resolved path reaches.
    ///
    /// The path is the crate's one grammar, already parsed, and the same
    /// spelling a message is addressed by: a schema states one item type for
    /// a serie, so an indexed segment answers that item - every occurrence of
    /// a group has the field the item declares - and `Parties[0].PartyID`
    /// therefore reaches the member here as well as in the message that
    /// carries it.
    ///
    /// Nested segments continue through the schema walk, which recurses
    /// through a group's occurrence without consuming a segment.
    pub fn get_field_by_path(&self, path: &FieldPath) -> Option<&Field> {
        let (head, rest) = path.segments().split_first()?;
        let head = segment_name(head)?;
        if rest.is_empty()
            && let Some(field) = self
                .get_message_field_by_name(head)
                // A canonical Map suppresses scalar aliases, but still
                // shares the named-root ambiguity check with components.
                .filter(|field| !matches!(field.dtype(), crate::DataType::Map(_) | crate::DataType::SortedMap(_)))
        {
            return Some(field);
        }
        let mut roots = [crate::FixCategory::Components, crate::FixCategory::Groups]
            .into_iter()
            .filter_map(|category| self.get_definition(category, head));
        let root = roots.next()?;
        if roots.next().is_some() {
            return None;
        }
        if rest.is_empty() {
            return Some(root);
        }
        descend(root, rest)
    }

    /// Returns the field a resolved path reaches, raising absence.
    pub fn field_by_path(&self, path: &FieldPath) -> Result<&Field> {
        self.get_field_by_path(path)
            .ok_or_else(|| absent(format_args!("path {path}")))
    }

    /// Returns the field a tag, identifier, name, or dotted path reaches.
    pub fn get_field<'key>(&self, key: impl Into<FixKey<'key>>) -> Option<&Field> {
        match key.into() {
            FixKey::Tag(tag) => self.get_field_by_tag(tag),
            FixKey::Id(id) => self.get_field_by_id(id),
            FixKey::Name(name) => self.named_or_path(name),
        }
    }

    /// Returns the field a generic key reaches, raising absence.
    pub fn field<'key>(&self, key: impl Into<FixKey<'key>>) -> Result<&Field> {
        match key.into() {
            FixKey::Tag(tag) => self.field_by_tag(tag),
            FixKey::Id(id) => self.field_by_id(id),
            FixKey::Name(name) => {
                if let Some(field) = self.get_field_by_name(name) {
                    return Ok(field);
                }
                // Where the key spells a path, the path door is the reading
                // that was attempted, so its absence is the one to raise.
                match FieldPath::from_str(name) {
                    Ok(path) if path.segments().len() > 1 => self.field_by_path(&path),
                    _ => Err(absent(FixKey::Name(name))),
                }
            }
        }
    }

    /// Returns whether a generic key reaches a field.
    pub fn contains<'key>(&self, key: impl Into<FixKey<'key>>) -> bool {
        self.get_field(key).is_some()
    }

    /// Every source id any field or named definition names in its
    /// `FIX:sources`, folded, sorted, each once.
    ///
    /// Membership is provenance and this is its listing; nothing resolves
    /// through it. The ids a field states, not the [catalog](Self::sources)
    /// the registry holds: an entry no field names is not listed here, and
    /// an id no entry holds is. A registry holding only the specification's
    /// own fields answers nothing.
    pub fn dialects(&self) -> Vec<String> {
        let mut held: BTreeSet<String> = BTreeSet::new();
        for field in &self.fields {
            for source in field.as_fix().sources() {
                held.insert(source.to_owned());
            }
        }
        for field in self.catalog.all() {
            for source in field.field.as_fix().sources() {
                held.insert(source.to_owned());
            }
        }
        held.into_iter().collect()
    }

    /// Walks the sources catalog, in id order.
    ///
    /// One entry per source this dictionary was built from - a `.cfb` folded
    /// in, a dialect a definition was created under - holding what is known
    /// of it once: the id a field's `FIX:sources` names, the file it was
    /// read from and the role of its plugin. A store writes the catalog as
    /// `sources.json`.
    pub fn sources(&self) -> impl ExactSizeIterator<Item = &FixSource> {
        self.sources.values()
    }

    /// The source held under `id`, or nothing.
    ///
    /// One id is one entry under the crate's fold - the fold a field's list
    /// is deduplicated under and [`FixField::has_source`](crate::FixField::has_source)
    /// reads by - so the catalog and a field agree on which spellings are
    /// one source: `VENUE` and `ve_nue` both reach `venue`.
    #[must_use]
    pub fn get_source(&self, id: &str) -> Option<&FixSource> {
        // The stored key is the folded id and a field states it folded, so
        // the exact hit is the ordinary one; a caller spelling it otherwise
        // pays one scan of a catalog a few dozen entries long.
        self.sources.get(id).or_else(|| {
            self.sources
                .values()
                .find(|source| folds_equal(source.id(), id))
        })
    }

    /// Records one source in the catalog, answering whether it arrived.
    ///
    /// An id already held (under the fold [`Self::get_source`] reads by,
    /// so the catalog holds one entry per id however it is spelled) keeps
    /// its entry and takes only what it lacked: a file where it stated
    /// none, a plugin role where it stated `UKNW`; two stated roles that
    /// disagree keep the held one, logged at warn. Nothing here touches a field, since a field names its sources
    /// itself, so an entry may stand that no field names, and `yggdryl fix
    /// check` is what says so.
    pub fn add_source(&mut self, source: FixSource) -> bool {
        let held = self.get_source(&source.id).map(|held| held.id.clone());
        match held {
            Some(key) => {
                if let Some(held) = self.sources.get_mut(&key) {
                    if held.file.is_none() {
                        held.file = source.file;
                    }
                    if held.pluginside == crate::Side::Unknown {
                        held.pluginside = source.pluginside;
                    } else if source.pluginside != crate::Side::Unknown
                        && source.pluginside != held.pluginside
                    {
                        log::warn!(
                            "FIX source {key:?} states plugin side {}, keeping the held {}",
                            source.pluginside,
                            held.pluginside
                        );
                    }
                }
                false
            }
            None => {
                self.sources.insert(source.id.clone(), source);
                true
            }
        }
    }

    /// Removes the source held under `id`, folded, answering the entry, or
    /// nothing where none is held.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] naming the first field or definition
    /// that still names the id, because a field may not be left naming an
    /// entry the catalog does not hold.
    pub fn remove_source(&mut self, id: &str) -> Result<Option<FixSource>> {
        let Some(key) = self.get_source(id).map(|source| source.id.clone()) else {
            return Ok(None);
        };
        let named = self
            .fields
            .iter()
            .chain(self.catalog.all().map(|entry| entry.field.as_field()))
            .find(|field| field.as_fix().has_source(&key));
        if let Some(field) = named {
            return Err(Error::conflict(
                "FIX source no field names",
                "one a field names",
                format_smolstr!("{key:?} on {:?}", field.name()),
            ));
        }
        Ok(self.sources.remove(&key))
    }

    /// Adds a field, replacing only an equal identity, under the stored
    /// spelling.
    ///
    /// A field arriving on a tag another field holds under another name is
    /// a field of its own and is added beside the holder: a bare wire tag
    /// keeps answering the holder, and the arrival is reached by its name or
    /// its identity. Neither learns the other's name.
    pub fn insert(&mut self, field: Field) -> Result<Option<Field>> {
        // A nested field is a component or a group, by its shape, and lands
        // among the named definitions.
        if let Some(category) = Self::definition_category_of(&field)? {
            return self.insert_definition(category, field);
        }
        if self.position_of_identity(&field)?.is_some() {
            let mut staged = self.clone();
            let prior = staged.insert_resolved(field, References::Refresh)?;
            if staged.state_parents() {
                staged.refresh_references()?;
            }
            staged.validate_catalog()?;
            *self = staged;
            return Ok(prior);
        }
        let (_, id) = canonical_identity(&field)?;
        let prior = self.insert_resolved(field, References::Refresh)?;
        if let Some(at) = self.canonical_position_by_id(id)
            && self.state_parents_around(at)
        {
            self.refresh_references()?;
        }
        Ok(prior)
    }

    fn insert_resolved(
        &mut self,
        mut field: Field,
        references: References,
    ) -> Result<Option<Field>> {
        self.validate_definition(crate::FixCategory::Fields, &field)?;
        let (tag, id) = canonical_identity(&field)?;
        let alternate = field.as_fix().tags()?;
        let replacing = self.position_of_identity(&field)?;
        self.check_free(&field, tag, id, &alternate, replacing)?;
        if let Some(position) = replacing {
            field.set_name(self.fields[position].name());
        }
        match replacing {
            Some(position) => self
                .replace_field(position, field, tag, references)
                .map(Some),
            None => {
                let position = self.fields.len();
                self.fields.push(field);
                self.index(position);
                if tag == 35 {
                    self.refresh_msgtype_aliases();
                }
                Ok(None)
            }
        }
    }

    /// Merges a definition into the field with the same canonical identity.
    /// A name folding to the stored one retains the stored canonical spelling.
    pub fn update(&mut self, field: Field) -> Result<()> {
        if let Some(category) = Self::definition_category_of(&field)? {
            self.update_definition(category, field)?;
            return Ok(());
        }
        let mut staged = self.clone();
        staged.update_resolved(field, References::Refresh)?;
        if staged.state_parents() {
            staged.refresh_references()?;
        }
        staged.validate_catalog()?;
        *self = staged;
        Ok(())
    }

    fn update_resolved(&mut self, field: Field, references: References) -> Result<()> {
        let (_, id) = canonical_identity(&field)?;
        let Some(position) = self.position_of_identity(&field)? else {
            return Err(absent(FixKey::Id(id)));
        };
        self.update_at(position, field, references, Naming::Stored)
    }

    /// Folds `field` into the field at `position`, which holds its tag.
    ///
    /// The merged field is named as `naming` says: by the stored field, which
    /// is every identity fold, or by the incoming one, which is how a field
    /// named by nothing but its decimal tag learns what a source calls it.
    fn update_at(
        &mut self,
        position: usize,
        field: Field,
        references: References,
        naming: Naming,
    ) -> Result<()> {
        self.validate_definition(crate::FixCategory::Fields, &field)?;
        let stored = &self.fields[position];
        let (tag, _) = canonical_identity(stored)?;
        if stored.dtype() != field.dtype() {
            return Err(datatype_disagreement(stored, tag, &field));
        }
        // The incoming definition is what the merge folds the stored one
        // into, so the caller's ordering is the precedence: a generator
        // merging its lowest-priority source first leaves the highest as the
        // last `update`, which wins.
        //
        // Two halves, because the `FIX:` view reaches only its own namespace
        // by design. The generic keys fold through the metadata merge every
        // protocol shares, and the `FIX:` keys through the rule each one has.
        let mut merged = field.clone();
        if naming == Naming::Stored {
            merged.set_name(stored.name());
        }
        merged.set_metadata(field.as_metadata().merge_with(stored.as_metadata())?.iter())?;
        merged.as_fix_mut().set_tag(tag)?;
        merged.as_fix_mut().merge_with(&stored.as_fix())?;
        if naming == Naming::Incoming {
            // The name the field takes is nobody's alias, its own included -
            // a stored field spelled by its normalization alone aliased what
            // it is now called - and the display the digits carried was a
            // spelling another tag owns, so it goes unless the arrival states
            // one.
            let aliases: Vec<String> = merged
                .as_fix()
                .names()
                .filter(|alias| !folds_equal(alias, merged.name()))
                .map(str::to_owned)
                .collect();
            merged.as_fix_mut().set_names(&aliases)?;
            if field.display().is_none() {
                merged.remove_metadata("display");
            }
        }
        let alternate = merged.as_fix().tags()?;
        let (tag, id) = canonical_identity(&merged)?;
        self.check_free(&merged, tag, id, &alternate, Some(position))?;
        let (_, held) = canonical_identity(&self.fields[position])?;
        if held != id {
            self.renamed
                .insert(held, Some((tag, SmolStr::new(merged.name()))));
        }
        // Last before the write, so a refusal above leaves every set as it
        // was: the fold keeps the stored field's set name, and what the
        // incoming field's set declares is in that set by the time the field
        // naming it is written.
        self.unify_named_codeset(position, &field, references)?;
        self.replace_field(position, merged, tag, references)?;
        Ok(())
    }

    /// Folds the set `field` reads by into the one the stored field at
    /// `position` reads by, so the fold that keeps the stored name keeps
    /// every member too.
    fn unify_named_codeset(
        &mut self,
        position: usize,
        field: &Field,
        references: References,
    ) -> Result<()> {
        let stored = self.fields[position].as_fix().codeset().map(SmolStr::new);
        let incoming = field.as_fix().codeset().map(SmolStr::new);
        self.unify_codeset(stored.as_deref(), incoming.as_deref(), references)
    }

    /// Folds `field` into the stored field at `position`, which its name
    /// reaches.
    ///
    /// The stored field is the one being described, so it keeps its identity,
    /// its canonical spelling and its shape - the datatype has to agree, and
    /// the nullability stays the stored one, because a second spelling of a
    /// field is not a statement about whether it may be absent - and
    /// everything else folds with the precedence [`Self::update`] has: the
    /// incoming side wins a shared key. What is specific to a fold by name is
    /// the identity the incoming field carried: its name becomes an alias and
    /// its tag an alternate, because a second dictionary calling tag 9001
    /// `Symbol` is stating a second spelling of tag 55's field, not a second
    /// field. The tag is left out when another field already answers it - a
    /// tag is one field's, and this field is not the one that claimed it -
    /// which is noted through `log` at debug level rather than refused, since
    /// the name was the match.
    fn merge_named(&mut self, position: usize, field: Field) -> Result<()> {
        self.validate_definition(crate::FixCategory::Fields, &field)?;
        let (incoming, _) = canonical_identity(&field)?;
        let stored = &self.fields[position];
        let (tag, id) = canonical_identity(stored)?;
        if stored.dtype() != field.dtype() {
            return Err(datatype_disagreement(stored, tag, &field));
        }
        let mut merged = field.clone();
        merged.set_name(stored.name());
        merged.set_nullable(stored.is_nullable());
        merged.set_metadata(field.as_metadata().merge_with(stored.as_metadata())?.iter())?;
        // The identity is the stored field's, and it is written before the
        // `FIX:` fold, which holds both sides to one tag.
        merged.as_fix_mut().set_tag(tag)?;
        merged.as_fix_mut().merge_with(&stored.as_fix())?;
        // Stored order first, then what only the incoming field states. The
        // canonical tag is nobody's alternate, its own field's included.
        let mut tags = stored.as_fix().tags()?;
        let claimed = |tags: &[i32], held: i32| held == tag || tags.contains(&held);
        for held in field.as_fix().tags()? {
            if !claimed(&tags, held) {
                tags.push(held);
            }
        }
        match self.position_by_tag(incoming) {
            Some(holder) if holder != position => log::debug!(
                "tag {incoming} of {:?} stays with {:?}",
                field.name(),
                self.fields[holder].name()
            ),
            _ if claimed(&tags, incoming) => {}
            _ => tags.push(incoming),
        }
        // Spellings dedupe under the same fold the index resolves them by,
        // so no alias is kept that the stored name or an earlier alias
        // already answers for.
        let mut aliases: Vec<&str> = stored.as_fix().names().collect();
        for alias in field.as_fix().names().chain(std::iter::once(field.name())) {
            if !folds_equal(alias, stored.name())
                && !aliases.iter().any(|held| folds_equal(held, alias))
            {
                aliases.push(alias);
            }
        }
        merged.as_fix_mut().set_tags(&tags)?;
        merged.as_fix_mut().set_names(&aliases)?;
        let alternate = merged.as_fix().tags()?;
        self.check_free(&merged, tag, id, &alternate, Some(position))?;
        // As `update_resolved` does, last before the write: the fold keeps
        // the stored field's set name, so what the incoming field's set
        // declares has to be in that set by the time the field is written.
        self.unify_named_codeset(position, &field, References::Defer)?;
        // Only a fold merges by name, and a fold settles its references once.
        self.replace_field(position, merged, tag, References::Defer)?;
        Ok(())
    }

    /// Puts `merged` where the field at `position` was, answering what stood
    /// there.
    ///
    /// `merged` carries the tag `tag` the position already answers to, and
    /// the caller has proven every key it holds free. A change to the
    /// metadata alone is what the catalog's references restate, so that one
    /// re-resolves them - now, or where `references` defers it, by the
    /// caller once its last change is in; a change to the datatype is one
    /// they refuse, and the caller's validation is what refuses it.
    fn replace_field(
        &mut self,
        position: usize,
        merged: Field,
        tag: i32,
        references: References,
    ) -> Result<Field> {
        // A field that takes another name - one named by nothing but its tag
        // learning what a source calls it - is read by the catalog's members
        // under the identity it holds now, which the resolution the refresh
        // runs rewrites them to.
        let renamed = self.fields[position].name() != merged.name();
        let refresh = references == References::Refresh
            && (renamed || metadata_only_change(&self.fields[position], &merged));
        if refresh {
            self.validate_catalog()?;
        }
        self.unindex(position, position);
        let prior = std::mem::replace(&mut self.fields[position], merged);
        self.index(position);
        if refresh {
            self.refresh_references()?;
        }
        if tag == 35 {
            self.refresh_msgtype_aliases();
        }
        Ok(prior)
    }

    /// Adds one field, folding it into what the dictionary already holds.
    ///
    /// The lenient counterpart of [`Self::insert`], which replaces, and of
    /// [`Self::update`], which refuses everything new: reading a second
    /// source over a first wants a definition the dictionary lacks to arrive
    /// and one it has to keep every key only it declares. Answers `true`
    /// when a field or a named definition arrived and `false` when the
    /// incoming one folded into a stored one. The rules, in order:
    ///
    /// 1. A nested field is a named definition and goes to
    ///    [`Self::insert`] under the category its shape names: a
    ///    `Serie` or `LargeSerie` of non-null Struct occurrences is a group, a
    ///    Struct declaring `FIX:msgtype` is a message, any other Struct is a
    ///    component. Any other nested datatype is refused as a scalar field
    ///    would refuse it.
    /// 2. One of this crate's own tags is every dictionary's already, so it is
    ///    neither added nor merged: skipped, answering `false`.
    /// 3. An identity the dictionary holds - the tag and the folded name
    ///    together - merges exactly as [`Self::update`] merges: the datatype
    ///    must equal the stored one, and the incoming metadata wins a shared
    ///    key. A field named by nothing but its own decimal tag is unnamed,
    ///    the way a code named after its wire value is, so it folds the same
    ///    way: one arriving on a held tag merges into the holder whatever the
    ///    holder is called, and a holder so named takes the name of a field
    ///    arriving on its tag - where no field answers that name already -
    ///    and every member reading it under the digits reads it under the
    ///    name.
    /// 4. Otherwise a name that folds to a stored field's canonical name -
    ///    or, where no field holds the arrival's tag, to one of its aliases -
    ///    merges *into* that field: the same field spelled with another tag,
    ///    a tag a field of another name holds staying with that field.
    ///    Folding is the crate's one fold, the one every name lookup resolves
    ///    by: ASCII case, and the `_`, `-` and space separators, so
    ///    `party_id` is a spelling of `PartyID`. The
    ///    stored field keeps its identity, its name and its nullability;
    ///    aliases and alternate tags are the union, the stored ones first,
    ///    deduplicated under the same fold; the incoming name joins the
    ///    aliases unless it is the canonical one; the incoming canonical tag
    ///    joins the alternate tags unless a field already answers it, in
    ///    which case it is left out and noted through `log` at debug level;
    ///    generic metadata and the `FIX:` keys fold with the precedence of
    ///    rule 3. A datatype that disagrees is refused as rule 3 refuses it.
    /// 5. Otherwise it is inserted - beside the holder of its tag where one
    ///    holds it under another name, the bare tag answering the holder and
    ///    the arrival reached by its own name, neither learning the other's.
    ///
    /// Staged like [`Self::insert`]: a refusal leaves the dictionary exactly
    /// as it was.
    ///
    /// ```
    /// use yggdryl::{DataType, FixId, FixRegistry};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut symbol = DataType::utf8().nullable_field("Symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let mut registry = FixRegistry::from_fields([symbol])?;
    ///
    /// // A venue's file calls the same field `symbol`, under its own tag and
    /// // with a second name: that is a second spelling of tag 55's field.
    /// let mut incoming = DataType::utf8().nullable_field("symbol");
    /// incoming.as_fix_mut().set_tag(9001)?;
    /// incoming.as_fix_mut().set_names(["Ticker"])?;
    /// assert!(!registry.add_field(incoming)?, "folded into the stored field");
    ///
    /// let stored = registry.field_by_tag(9001)?;
    /// assert_eq!(stored.name(), "Symbol");
    /// assert_eq!(stored.as_fix().id()?, Some(FixId::of(55, "Symbol")?));
    /// assert_eq!(stored.as_fix().tags()?, [9001]);
    /// assert!(stored.as_fix().names().any(|alias| alias == "Ticker"));
    /// assert_eq!(registry.field_by_name("ticker")?.name(), "Symbol");
    ///
    /// // A field nothing stored answers to arrives.
    /// let mut price = DataType::Float64.nullable_field("Price");
    /// price.as_fix_mut().set_tag(44)?;
    /// assert!(registry.add_field(price)?);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns what [`Self::insert`] and [`Self::update`] return: absence
    /// for a scalar carrying no
    /// `FIX:tag`, a conflict for an alias or alternate tag another field
    /// holds, and [`Error::InvalidRecord`] for a datatype
    /// that disagrees with the stored definition. An incoming `float32` field
    /// meeting a stored `float64` field is refused: merging metadata does not
    /// change the field's declared datatype.
    pub fn add_field(&mut self, field: Field) -> Result<bool> {
        if let Some(category) = Self::definition_category_of(&field)? {
            return self.add_definition(category, field);
        }
        let mut staged = self.clone();
        let added = staged.fold_scalar(field)?.unwrap_or(false);
        staged.state_parents();
        staged.refresh_references()?;
        staged.validate_catalog()?;
        *self = staged;
        Ok(added)
    }

    /// Adds every field, the way [`Self::add_field`] adds one.
    ///
    /// The bulk form of exactly those rules: a scalar merges into the field
    /// its identity or its name reaches and is inserted otherwise, a nested
    /// field is a named definition and folds through [`Self::insert`], and
    /// one of this crate's own tags is skipped.
    /// Answers the count added and the count merged, in that order, and
    /// records the same pair through `log` at debug level; a skipped field
    /// counts as neither.
    ///
    /// The caller's order is the precedence, exactly as [`Self::update`]'s
    /// is: merge the lowest-priority source first and the highest arrives
    /// last and wins.
    ///
    /// ```
    /// use yggdryl::{DataType, FixRegistry, StructType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut symbol = DataType::utf8().nullable_field("Symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let mut registry = FixRegistry::from_fields([symbol.clone()])?;
    ///
    /// // Tag 55 is stored and merges; tag 44 is new; the Struct is a component.
    /// symbol.as_fix_mut().set_description("Ticker symbol")?;
    /// let mut price = DataType::Float64.nullable_field("Price");
    /// price.as_fix_mut().set_tag(44)?;
    /// let instrument = DataType::from(StructType::from_fields([DataType::utf8().nullable_field("Symbol")])?)
    ///     .required_field("Instrument");
    /// assert_eq!(registry.add_fields([symbol, price, instrument])?, (2, 1));
    /// assert_eq!(registry.field_by_tag(55)?.description(), Some("Ticker symbol"));
    /// assert_eq!(registry.field_by_name("Instrument")?.field_len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns what [`Self::add_field`] returns for any one of the fields.
    /// The whole fold is one mutation: it is staged and only then adopted, so
    /// a refusal on the last field of a thousand leaves the dictionary exactly
    /// as it was and what a caller fixes is the source. That costs one copy of
    /// the dictionary per call, paid once rather than per field.
    pub fn add_fields<I>(&mut self, fields: I) -> Result<(usize, usize)>
    where
        I: IntoIterator<Item = Field>,
    {
        let mut staged = self.clone();
        let counts = staged.fold(fields)?;
        staged.state_parents();
        staged.refresh_references()?;
        staged.validate_catalog()?;
        *self = staged;
        Ok(counts)
    }

    /// Folds another dictionary into this one.
    ///
    /// The one place two dictionaries combine. Every field folds exactly as
    /// [`Self::add_field`] folds one - a stored identity or a stored name
    /// merges, anything else inserts - every named definition folds as
    /// [`Self::insert`] folds one, its members appended to the stored
    /// definition of its name rather than replacing them. The sources that
    /// contributed a field travel with it and union onto the stored one, and
    /// the other dictionary's sources catalog unions onto this one's, so a
    /// merged registry says which sources spoke each field.
    ///
    /// Aliases accumulate rather than replace, because reading a second
    /// source is not a statement that the first one's names were wrong, and
    /// the caller's order is the precedence here as it is everywhere else.
    ///
    /// **What the other dictionary says at another precision is folded under
    /// the declaration held.** Every FIX datatype derives from String and the
    /// numeric families state no width, so a source typing tag 44 `float64`,
    /// `utf8` or `int32` where the dictionary holds `decimal128(38, 18)` has
    /// said less than the dictionary rather than something else: merging
    /// never changes a declared datatype, the field folds under the stored
    /// one - membership, aliases and code set included - and is counted in
    /// [`FixMerge::restated`]. An unbounded text declaration restates any
    /// datatype, the integer, floating and decimal families restate each
    /// other, an integer restates an enum leaf, the byte layouts one another,
    /// a date a datetime whatever the zone, and one time width another. The
    /// one exception is a repeating group's counter: a field the other
    /// dictionary counts a group by is NumInGroup, and one held as unbounded
    /// text, a float or another integer width is retyped int32 - said at
    /// warn level - so the group stands whichever source was folded first.
    ///
    /// **What the other dictionary says otherwise is passed over, and named;
    /// the rest of it still folds.** Counterparties contradict each other
    /// about one tag - one file types tag 43 `boolean`, the next `int32` -
    /// so the declaration already held stays and the other is a [`FixDrop`]
    /// in the answer: a scalar whose datatype contradicts the field its
    /// identity or name reaches, a member a held definition already declares
    /// in another shape, a definition whose component or message code
    /// disagrees with the one held under its name, a member reading a field
    /// passed over, and a code set that will not fold into the one held. Two
    /// references under one member name to two groups or two components on
    /// one counter are one member read two ways: the members the incoming
    /// target declares - as the source states it, whichever of the two folds
    /// first - fold into the held target under these same rules, so a group
    /// one dialect split for one message still widens the group the
    /// dictionary reads there. A member is the field it reads before the name
    /// it carries: an incoming member reading another field than the held
    /// member of its name is the held member reading that field where one
    /// does, and a member of its own beside the held ones, `{name}2`,
    /// otherwise - two tags are two tags on the wire - so a spelling two
    /// tags share passes nothing over whichever source folds first. A group
    /// held under its name on another counter is another group: it arrives
    /// named for its counter, `dealers_7101`, and a member reading it stands
    /// beside the held member under that counter's suffix. A definition whose
    /// fold refuses rather than passing a member over is passed over whole,
    /// every write it made undone.
    ///
    /// **One structure is one definition.** Once every definition of the
    /// source folded by name, each pair of components or groups of one
    /// structure the fold made - an arrival stating a held definition's
    /// structure, or a held definition the fold widened into another's -
    /// folds into a held one, the first in name order the fold widened else
    /// the first in name order: its members relaxed to the more permissive
    /// nullability, both sources listed, every reference to the other
    /// rewritten. Two definitions held as they were stay two, and so do two
    /// one source brought unless a definition held before the fold states
    /// their structure, which both fold into. A definition folded away is a name the dictionary
    /// no longer holds, so a source stating that name with fewer members than
    /// the structure it folded into - the source whose fold folded it away
    /// included - lands it again when it folds again, read by no member
    /// already folded, and a fold widening it once more files the structure
    /// under it in turn.
    ///
    /// Answers the [`FixMerge`]: the scalars added and merged, and what was
    /// passed over.
    ///
    /// ```
    /// use yggdryl::{DataType, FixRegistry, StructType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut symbol = DataType::utf8().nullable_field("Symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let mut held = FixRegistry::from_fields([symbol.clone()])?;
    /// held.insert(DataType::from(StructType::from_fields([symbol.clone()])?).required_field("Instrument"))?;
    ///
    /// // The other dictionary holds the same field under its own tag, with a
    /// // second name, and knows one more member of the component.
    /// let mut ticker = DataType::utf8().nullable_field("symbol");
    /// ticker.as_fix_mut().set_tag(9001)?;
    /// ticker.as_fix_mut().set_names(["Ticker"])?;
    /// let mut other = FixRegistry::from_fields([ticker])?;
    /// let venue = DataType::utf8().nullable_field("VenueSymbol");
    /// other.insert(DataType::from(StructType::from_fields([symbol, venue])?).required_field("Instrument"))?;
    ///
    /// // Symbol and the two standard clock seeds merge.
    /// let merge = held.merge_with(&other)?;
    /// assert_eq!((merge.added, merge.merged), (0, 3));
    /// assert!(merge.is_clean());
    /// let stored = held.field_by_tag(9001)?;
    /// assert_eq!(stored.name(), "Symbol");
    /// assert_eq!(stored.as_fix().names().collect::<Vec<_>>(), ["Ticker"]);
    /// let instrument = held.field_by_name("Instrument")?;
    /// assert_eq!(instrument.fields()[1].name(), "VenueSymbol");
    ///
    /// // A third dictionary types tag 44 at another precision - as the text
    /// // every FIX datatype derives from: the field held stays, and the
    /// // declaration folds under it, counted as restated.
    /// let mut price = DataType::Float64.nullable_field("Price");
    /// price.as_fix_mut().set_tag(44)?;
    /// held.add_field(price.clone())?;
    /// price.set_dtype(DataType::utf8())?;
    /// let merge = held.merge_with(&FixRegistry::from_fields([price.clone()])?)?;
    /// assert_eq!((merge.merged, merge.restated), (3, 1));
    /// assert!(merge.is_clean());
    /// assert_eq!(held.field_by_tag(44)?.dtype(), &DataType::Float64);
    ///
    /// // A fourth contradicts it - a price is no flag - so the declaration
    /// // is named and everything else still folds.
    /// price.set_dtype(DataType::Boolean)?;
    /// let mut side = DataType::utf8().nullable_field("Side");
    /// side.as_fix_mut().set_tag(54)?;
    /// let merge = held.merge_with(&FixRegistry::from_fields([price, side])?)?;
    /// assert_eq!(merge.added, 1);
    /// assert_eq!(merge.dropped.len(), 1);
    /// assert!(merge.dropped[0].reason.contains("float64"));
    /// assert_eq!(held.field_by_tag(44)?.dtype(), &DataType::Float64);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns what leaves nothing to keep: an incoming dictionary whose own
    /// catalog does not validate, a resolution of the folded catalog that
    /// does not, and what [`Self::add_field`] refuses beyond a disagreeing
    /// declaration. One mutation: the fields and the
    /// definitions are staged together and adopted together, so a refusal
    /// anywhere leaves this dictionary exactly as it was.
    pub fn merge_with(&mut self, other: &Self) -> Result<FixMerge> {
        let mut staged = self.clone();
        let merge = staged.fold_registry(other)?;
        *self = staged;
        Ok(merge)
    }

    /// Folds another dictionary in, the way [`Self::merge_with`] folds one,
    /// without the staging.
    ///
    /// The fold itself: [`Self::merge_with`] is this plus the copy that makes
    /// it one mutation, and a caller folding several sources into one staged
    /// dictionary calls this so the copy is paid once rather than once per
    /// source. That is the same relationship [`Self::add_fields`] has to
    /// [`Self::fold`], one level up.
    ///
    /// Every scalar folds with its references deferred, and the catalog is
    /// resolved against the folded fields once, at the end: a source of
    /// thousands of fields costs one resolution of the catalog, never one
    /// per field.
    fn fold_registry(&mut self, other: &Self) -> Result<FixMerge> {
        let mut documents = self.documents()?;
        let merge = self.fold_source(other, &mut documents)?;
        self.state_parents();
        self.settle(documents)?;
        Ok(merge)
    }

    /// One source folded into the fields and into `documents`, the catalog
    /// as the documents a store writes, which the caller resolves once when
    /// its last source is in.
    ///
    /// Nothing between the two reads the resolved catalog: the scalar rules
    /// route on the fields, and a document names a field by its identity
    /// rather than holding its metadata, so every source a caller folds can
    /// share one set of documents and one resolution.
    fn fold_source(
        &mut self,
        other: &Self,
        documents: &mut super::catalog::Documents,
    ) -> Result<FixMerge> {
        other.validate_catalog()?;
        // In the order the other dictionary *answers*, never the order it
        // happens to be stored in. The fold's precedence is its input order,
        // and a caller merging a registry supplies no order of its own - so
        // the one a registry has is the one it publishes: tag-major, the
        // tag's holder first. Storage order is neither that nor stable: a
        // removal swaps the last field into the hole, and a store round trip
        // writes in `iter` order and loads in file order, so two dictionaries
        // that compare equal could merge to two different answers.
        // The vocabularies first, and this is why: a field keeps the set it
        // already reads by, so the members the other dictionary states have
        // to be in that set by the time the field is folded. Fold them after
        // and a merge would narrow a vocabulary instead of widening one.
        let mut merge = FixMerge {
            sources: 1,
            ..FixMerge::default()
        };
        self.merge_codesets(other, &mut merge.dropped);
        // The sources catalog beside the vocabularies, for the same reason:
        // a field the fold keeps names its sources, and the entry each id
        // names has to be here by then.
        for source in other.sources.values() {
            self.add_source(source.clone());
        }
        // How a member of the other dictionary's definitions reads each field
        // the scalar fold did not keep under the identity the member names: a
        // field merged by its name alone now answers under the held identity,
        // and one passed over with nothing here answering to its identity is
        // gone, so the members reading it go with it.
        let mut remap = HashMap::new();
        // The tags the other dictionary counts a repeating group by: a field
        // on one is NumInGroup there, whatever this dictionary typed it.
        let counters: std::collections::HashSet<i32> = other
            .catalog
            .iter(crate::FixCategory::Groups)
            .filter_map(|group| group.as_fix().counter().ok().flatten())
            .collect();
        // The scalars alone: the definitions fold through the catalog merge
        // below, under their own rules, and counting them here would count
        // one fold twice.
        for declared in other.scalars() {
            let mut route = self.route(declared)?;
            let (tag, _) = canonical_identity(declared)?;
            if counters.contains(&tag) {
                // A tag this dictionary reads as another field's alternate is
                // that field spelled with another number. Where the field is
                // a count, a group counted by the tag is counted by it; where
                // it is anything else, the group contradicts the dictionary,
                // and counting it by the field's own tag, or retyping a field
                // reached only through a spelling of it, would read every
                // value that field carries as a count.
                if let Route::Identity(position) = route
                    && canonical_identity(&self.fields[position])?.0 != tag
                    && self.fields[position].dtype() != &crate::DataType::Int32
                {
                    let held = &self.fields[position];
                    let refusal = Error::InvalidRecord {
                        path: declared.name().into(),
                        reason: crate::text::expected_got(
                            format_args!("tag {tag}, counting a repeating group, to spell a count"),
                            format_args!(
                                "the alternate tag of {} ({}), held as {}",
                                held.name(),
                                canonical_identity(held)?.0,
                                held.dtype()
                            ),
                        ),
                    };
                    merge.dropped.push(FixDrop::new(declared.clone(), &refusal));
                    remap.insert(canonical_id(declared)?, None);
                    continue;
                }
                // The field a group is counted by here: the one the arrival
                // folds into - its tag's, or the one its name reaches on
                // another tag - else the tag's canonical holder, which a
                // group's counter reads.
                if declared.dtype() == &crate::DataType::Int32
                    && let Some(position) = route.target().or_else(|| self.tags.get(&tag).copied())
                    && self.retype_counter(position)?
                {
                    route = self.route(declared)?;
                }
            }
            // The field as it folds: the source's declaration, restated under
            // the datatype the dictionary holds where the two say one thing
            // at two precisions - merging never changes a declared datatype,
            // and a source that said less than the dictionary has not said
            // otherwise.
            let mut field = declared.clone();
            let disagreement = match route
                .target()
                .map(|position| &self.fields[position])
                .filter(|stored| stored.dtype() != field.dtype())
            {
                Some(stored) if datatypes_agree(stored.dtype(), field.dtype()) => {
                    log::debug!(
                        "fix merge restated {} as {} rather than {}",
                        field.name(),
                        stored.dtype(),
                        field.dtype()
                    );
                    field.set_dtype(stored.dtype().clone())?;
                    merge.restated += 1;
                    None
                }
                Some(stored) => Some(datatype_disagreement(
                    stored,
                    canonical_identity(stored)?.0,
                    &field,
                )),
                None => None,
            };
            let target = route.target();
            let refusal = match disagreement {
                Some(refusal) => refusal,
                None => match self.fold_routed(route, field.clone()) {
                    Ok(Some(true)) => {
                        merge.added += 1;
                        continue;
                    }
                    Ok(Some(false)) => {
                        merge.merged += 1;
                        if let Some(position) = target {
                            let (tag, id) = canonical_identity(&self.fields[position])?;
                            let name = SmolStr::new(self.fields[position].name());
                            // The source's members read the field under the
                            // identity that holds it now.
                            let arrived = canonical_id(&field)?;
                            if arrived != id {
                                remap.insert(arrived, Some((tag, name)));
                            }
                        }
                        continue;
                    }
                    Ok(None) => continue,
                    // A spelling or an alternate tag another field already
                    // answers to: the field cannot be held beside it, and the
                    // fold refused it before writing a thing.
                    Err(error) if error.is_conflict() => error,
                    Err(error) => return Err(error),
                },
            };
            // Named as the source stated it, the datatype it declared included.
            merge.dropped.push(FixDrop::new(declared.clone(), &refusal));
            match route {
                // The declaration is passed over, the field its tag reaches
                // is not: the dictionary keeps its own declaration of it, and
                // the source's members read that one, under the identity it
                // holds - an arrival named by nothing but its tag included,
                // whose own identity nothing holds.
                Route::Identity(position) | Route::Unnamed(position) => {
                    let (tag, id) = canonical_identity(&self.fields[position])?;
                    let arrived = canonical_id(declared)?;
                    if arrived != id {
                        let name = SmolStr::new(self.fields[position].name());
                        remap.insert(arrived, Some((tag, name)));
                    }
                }
                _ => {
                    remap.insert(canonical_id(declared)?, None);
                }
            }
        }
        // A member is read under the identity its field holds once every
        // scalar folded: a holder named by its tag alone that a later arrival
        // named is that name, whichever arrival the remap recorded first.
        for target in remap.values_mut().flatten() {
            for _ in 0..self.renamed.len() {
                match self.renamed.get(&FixId::of(target.0, &target.1)?) {
                    Some(Some(moved)) if moved != target => *target = moved.clone(),
                    _ => break,
                }
            }
        }
        // The dictionary's own documents read a field that took a source's
        // name - one named by nothing but its tag until now - under that name.
        documents.rename_references(&std::mem::take(&mut self.renamed))?;
        self.merge_catalog(documents, other, &remap, &mut merge.dropped)?;
        log::debug!(
            "added {} and merged {} fix fields, passing over {}",
            merge.added,
            merge.merged,
            merge.dropped.len()
        );
        Ok(merge)
    }

    /// Retypes the field at `position` int32 where a source counts a
    /// repeating group by it and this dictionary holds it at a coarser
    /// statement of a count: unbounded text, a float, or another integer
    /// width - answering whether it did.
    ///
    /// The one exception to a merge never changing a declared datatype, and
    /// the rule a CBlock's own parse already applies to a nested grammar's
    /// counter: a group is counted by NumInGroup, an int32, and a venue that
    /// typed the tag `float` or `string` before another counted a group by it
    /// said less than the second did, not something else. Without it the
    /// group and every member reading it would be passed over whenever the
    /// coarser declaration's file sorted first, and kept whenever it sorted
    /// second. A decimal, an enum, a code, a flag or a temporal is a
    /// contradiction rather than a coarser count, and stays as held - the
    /// group is then passed over and named. Logged at warn level, because a
    /// value another dialect sends in the tag that is not an integer reads as
    /// null from here on.
    fn retype_counter(&mut self, position: usize) -> Result<bool> {
        let held = self.fields[position].dtype();
        let coarser = held
            .string_parameters()
            .is_some_and(|leaf| leaf.bound().is_none())
            || matches!(
                held.kind(),
                crate::DataTypeKind::Integer | crate::DataTypeKind::Floating
            );
        if held == &crate::DataType::Int32 || !coarser {
            return Ok(false);
        }
        let mut retyped = self.fields[position].clone();
        retyped.set_dtype(crate::DataType::Int32)?;
        let (canonical, _) = canonical_identity(&retyped)?;
        log::warn!(
            "tag {canonical} counts a repeating group, so {} is retyped int32 from {held}",
            retyped.name()
        );
        self.replace_field(position, retyped, canonical, References::Defer)?;
        Ok(true)
    }

    /// Reads one Ullink `CBlock` into this dictionary, whole.
    ///
    /// The one call an ingest takes, and a parse in front of
    /// [`Self::merge_with`]: the file's vocabulary folds the way any source
    /// folds, and every field, group, component and message it produces
    /// carries the dialect's id in `FIX:sources`, the parse holding the
    /// dialect's entry - its id and the file's name - in its sources
    /// catalog, which is what the merge unions onto whatever this dictionary
    /// already held.
    ///
    /// `dialect` names the dictionary, and the file names it when the caller
    /// does not: with none supplied the handle's own stem stands in, where it
    /// reads as a source id - opening with an ASCII letter and holding no
    /// quote, backslash or control character - and nothing stands in where it
    /// does not. The FIX version
    /// the file's root declares is not carried: the version a capture is read
    /// at is the row's own `beginstring` where the transport states one, else
    /// what the line implies.
    ///
    /// Answers the [`FixMerge`], each drop naming the handle's URL as its
    /// source. The message roots are dropped; take [`Self::from_cfb_file`]
    /// when they matter.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::from_cfb_file`] and [`Self::merge_with`] return,
    /// and the id grammar's refusal when the supplied name is empty or holds
    /// a quote, a backslash or a control character.
    pub fn add_cfb_file(&mut self, handle: &dyn IOBase, dialect: Option<&str>) -> Result<FixMerge> {
        let stem = super::cfb::stem_dialect(handle);
        let (parsed, _) = Self::from_cfb_file(handle, dialect.or(stem.as_deref()))?;
        Ok(self.merge_with(&parsed)?.located(handle))
    }

    /// Reads every Ullink `CBlock` the locations hold into this dictionary.
    ///
    /// The plural of [`Self::add_cfb_file`], and it takes the locations alone:
    /// each is a holder, and what it is decides what it holds. A **glob** -
    /// `cblocks/*.cfb`, `cblocks/**/*.cfb`, `**/venue-*.cfb` - holds every
    /// file its pattern matches, walked exactly as [`IOBase::glob`] walks it:
    /// `*` inside one name, `**` across folders, private entries never
    /// matched. A **folder** holds the `.cfb` files directly inside it, the
    /// suffix in any case. A **file** holds itself, whatever it is named, and
    /// a location where nothing is holds nothing. A container a glob matches
    /// is passed by, since a dictionary is a file, and a file reached twice
    /// folds once. So a folder of counterparty files is
    /// `add_cfb_files(&[Holder::local("cblocks")?], None)`, and several
    /// locations are one call: one fold, one resolution.
    ///
    /// **Files fold in ascending URL order**, whatever order they were listed
    /// in, because the fold's precedence is its input order and a listing's
    /// sequence is not a caller's to see: it varies with how a pattern
    /// decomposed and with the backend beneath. So where two files disagree
    /// about one tag the first-sorting file's declaration is the one held and
    /// the later one is passed over, and `cblocks/`, `cblocks/*.cfb` and
    /// `**/*.cfb` over the same files all answer the same dictionary.
    ///
    /// **Files parse on every core, and fold on one.** A parse reads nothing
    /// but its own bytes, so the files are read in order and parsed side by
    /// side, at most one file per thread in hand; each parsed file then folds
    /// into the one staged dictionary in URL order, and the catalog is
    /// resolved once for all of them - so the answer is the sequential one,
    /// sooner, and a hundred files cost one resolution, not a hundred.
    ///
    /// **`dialect` is resolved per file.** A name supplied here stamps every
    /// file with the one membership; `None` lets each file's own stem stand
    /// in, which is what globbing a folder of counterparty files is for -
    /// `cblocks/*.cfb` over `MSFIX44.cfb` and `BLPFIX44.cfb` stamps `msfix44`
    /// and `blpfix44` rather than one name for both.
    ///
    /// Answers the [`FixMerge`] of every file together: `sources` counts the
    /// files folded, `failed` names every file left out, and each drop names
    /// the file it was read from. The file count is a fact only this call
    /// holds: an empty listing and one whose files all folded into stored
    /// fields both answer zeroes for the counts, so without it a mistyped
    /// pattern reads as a silent success.
    ///
    /// **One file is one mutation, and one bad file is one file.** Each file
    /// folds into the staged dictionary as a mutation of its own: a file that
    /// cannot be read, is not a well-formed CBlock, or whose fold refuses
    /// rather than passing a declaration over is rolled back alone - the
    /// staged dictionary is exactly what the files before it left - and named
    /// in [`FixMerge::failed`], and every other file still folds. Nothing is
    /// adopted until the last file is in and the catalog resolves, so a
    /// caller sees every file's contribution or none of the call's. The
    /// resolution is the one place two files' contributions can refuse each
    /// other; where it does, the files fold again one at a time, each resolved
    /// before the next, so the file the union cannot hold is the one named
    /// and left out, and the rest are still the dictionary.
    ///
    /// # Errors
    ///
    /// Returns a listing that fails - a location's or one entry of it, before
    /// any file is parsed, so a listing that fails part way is a refusal
    /// rather than a half-read dictionary - and what reading this
    /// dictionary's own catalog refuses.
    pub fn add_cfb_files(
        &mut self,
        locations: &[crate::holder::Holder],
        dialect: Option<&str>,
    ) -> Result<FixMerge> {
        // Collected before anything is parsed, so a listing that fails part
        // way is a refusal rather than a half-read dictionary. What is held
        // is one handle per file, never a registry per file.
        let mut listed = Vec::new();
        for location in locations {
            cblock_files(location, &mut listed)?;
        }
        let mut files: Vec<&dyn IOBase> = listed.iter().map(CblockFile::as_io).collect();
        files.sort_by_cached_key(|file| file.url().map(ToString::to_string));
        files.dedup_by_key(|file| file.url().map(ToString::to_string));
        let (staged, merge) = match self.fold_cfb_files(&files, dialect, false) {
            Ok(folded) => folded,
            Err(error) => {
                log::warn!(
                    "the {} cblock files folded do not resolve together ({error}); folding them again one at a time",
                    files.len()
                );
                self.fold_cfb_files(&files, dialect, true)?
            }
        };
        *self = staged;
        log::debug!(
            "added {} and merged {} fix fields from {} cblock files, passing over {} and leaving out {}",
            merge.added,
            merge.merged,
            merge.sources,
            merge.dropped.len(),
            merge.failed.len()
        );
        Ok(merge)
    }

    /// Every file of `files`, in the order given, parsed side by side and
    /// folded into one staged copy of this dictionary, each file as one
    /// mutation of its own: the staged copy and the [`FixMerge`] of the files
    /// that folded, every other file named in [`FixMerge::failed`].
    ///
    /// `settle_each` is the slow path a failed resolution falls back to:
    /// without it every file folds into one set of documents resolved once at
    /// the end, which is one resolution for any number of files and the
    /// refusal of the whole call where the union does not resolve; with it
    /// each file is resolved before the next folds, so a file the union
    /// cannot hold is left out by name.
    fn fold_cfb_files(
        &self,
        files: &[&dyn IOBase],
        dialect: Option<&str>,
        settle_each: bool,
    ) -> Result<(Self, FixMerge)> {
        // Read here, in order, and parsed on the workers: the bytes and the
        // name each file is stamped with cross by value, the dictionary each
        // parse answers comes back, and at most one file per thread is held.
        let threads = std::thread::available_parallelism()
            .map_or(1, usize::from)
            .min(files.len())
            .max(1);
        let reads = files.iter().copied().map(|handle| {
            let named = dialect
                .map(str::to_owned)
                .or_else(|| super::cfb::stem_dialect(handle).map(Cow::into_owned));
            (handle.read_all_bytes(), named, handle.url().cloned())
        });
        let parsed = crate::parallel::ordered(
            reads,
            threads,
            1,
            |(bytes, named, source): (Result<Vec<u8>>, Option<String>, Option<crate::Url>)| {
                bytes.and_then(|bytes| super::cfb::parse(&bytes, named.as_deref(), source.as_ref()))
            },
        )
        .with_lane_depth(1);
        let mut staged = self.clone();
        let mut documents = staged.documents()?;
        let mut merge = FixMerge::default();
        for (handle, parsed) in files.iter().copied().zip(parsed) {
            let folded = match parsed {
                Ok((parsed, _)) if settle_each => {
                    let mut trial = staged.clone();
                    let folded = (|| {
                        let mut documents = trial.documents()?;
                        let folded = trial.fold_source(&parsed, &mut documents)?;
                        trial.state_parents();
                        trial.settle(documents)?;
                        Ok(folded)
                    })();
                    if folded.is_ok() {
                        staged = trial;
                    }
                    folded
                }
                Ok((parsed, _)) => {
                    // The file's own mutation: what it folds is undone whole
                    // where its fold refuses, so the next file folds into
                    // exactly what the files before it left.
                    let held = staged.clone();
                    let mark = documents.checkpoint();
                    match staged.fold_source(&parsed, &mut documents) {
                        Ok(folded) => {
                            documents.release(mark);
                            Ok(folded)
                        }
                        Err(error) => {
                            staged = held;
                            documents.rollback(mark);
                            Err(error)
                        }
                    }
                }
                Err(error) => Err(error),
            };
            match folded {
                Ok(folded) => merge.absorb(folded.located(handle)),
                Err(error) => merge.failed.push(FixFailure::new(handle, &error)),
            }
        }
        if !settle_each {
            // One resolution for every file, whatever the count.
            staged.state_parents();
            staged.settle(documents)?;
        }
        Ok((staged, merge))
    }

    /// Adds every field, the way [`Self::fold_field`] adds one.
    ///
    /// The fold itself, without the staging: [`Self::add_fields`] is this plus
    /// the copy that makes it one mutation, and a caller already holding a
    /// staged dictionary calls this so the copy is paid once.
    fn fold<I>(&mut self, fields: I) -> Result<(usize, usize)>
    where
        I: IntoIterator<Item = Field>,
    {
        let mut added = 0_usize;
        let mut merged = 0_usize;
        for field in fields {
            match self.fold_field(field)? {
                Some(true) => added += 1,
                Some(false) => merged += 1,
                None => {}
            }
        }
        log::debug!("added {added} and merged {merged} fix field definitions");
        Ok((added, merged))
    }

    /// One field through the rules [`Self::add_field`] states, without the
    /// staging: `None` when it was this crate's own and so neither added nor
    /// merged, else whether it arrived.
    fn fold_field(&mut self, field: Field) -> Result<Option<bool>> {
        match Self::definition_category_of(&field)? {
            Some(category) => self.fold_definition(category, field).map(Some),
            None => self.fold_scalar(field),
        }
    }

    /// The category a nested field is a definition of, or `None` for a
    /// scalar; a nested datatype that is no definition is refused as the
    /// scalar rule refuses it.
    fn definition_category_of(field: &Field) -> Result<Option<crate::FixCategory>> {
        if !field.dtype().is_nested() {
            return Ok(None);
        }
        // A serie of non-null scalars is one column under one name rather
        // than a definition, so it folds as a field does; the catalog's own
        // shape check is where that reading lives.
        match super::catalog::definition_category(field) {
            Some(category) => Ok(Some(category)),
            None => super::catalog::column_shape(field).map(|()| None),
        }
    }

    /// One scalar through rules 2 to 5 of [`Self::add_field`], without the
    /// staging and without the catalog validation the staged verbs run.
    ///
    /// References are deferred: the caller resolves the catalog once when its
    /// last field is in.
    fn fold_scalar(&mut self, field: Field) -> Result<Option<bool>> {
        let route = self.route(&field)?;
        self.fold_routed(route, field)
    }

    /// Where rules 2 to 5 of [`Self::add_field`] land `field`.
    ///
    /// The one reading of those rules, so the fold that folds a field and the
    /// fold that asks first whether it would disagree cannot route it two
    /// ways.
    fn route(&self, field: &Field) -> Result<Route> {
        // The crate's own fields are every dictionary's, so folding one is
        // folding a field onto itself, and never a source's to redefine.
        if field
            .as_fix()
            .tag()
            .ok()
            .flatten()
            .is_some_and(super::is_crate_tag)
        {
            return Ok(Route::Own);
        }
        let (tag, _) = canonical_identity(field)?;
        if let Some(position) = self.position_of_identity(field)? {
            return Ok(Route::Identity(position));
        }
        // A field named by nothing but its own decimal tag carries no name:
        // it is what a source that never says what a tag is called produces.
        // Such a field on a tag a held field answers - as its own or as an
        // alternate - is that field's, whatever it is called, so a dictionary
        // holds one field per tag rather than a numbered twin beside a named
        // one, reached by the members of neither. The one exception is the
        // other half of a pair the source linked: an arrival naming the
        // holder's own tag among its alternates is the tag a source said is
        // spelled like the holder, which is two fields and not one.
        if is_unnamed(field) {
            let holder = self.tags.get(&tag).copied().or_else(|| {
                let holder = self.alternate_tags.get(&tag).copied()?;
                let (canonical, _) = canonical_identity(&self.fields[holder]).ok()?;
                let linked = field.as_fix().tags().ok()?.contains(&canonical);
                (!linked).then_some(holder)
            });
            if let Some(holder) = holder {
                return Ok(Route::Identity(holder));
            }
        }
        if let Some(holder) = self.tags.get(&tag).copied() {
            // A holder named by its tag alone takes the name of a field
            // arriving on its tag where the name is free.
            if is_unnamed(&self.fields[holder])
                && self
                    .position_by_name(field.name())
                    .is_none_or(|named| named == holder)
            {
                return Ok(Route::Unnamed(holder));
            }
            // A held tag under another name is a field of its own - unless
            // a third field is canonically named so. A canonical name
            // reaches one field, so an arrival on a tag another field holds,
            // under a name a third field holds as its own, is that field
            // spelled with another number - rule 4, read before rule 5 - and
            // never a field beside the holder of the tag, which could not be
            // named; the number stays with its holder
            // ([`Self::merge_named`]). A name another field holds only as an
            // alias is not this case: the arrival stands beside the holder
            // of its tag under its own name, and the alias stays where it is.
            return Ok(match self.canonical_position_by_name(field.name()) {
                Some(named) => Route::Named(named),
                None => Route::Beside,
            });
        }
        // A held name under another tag is the same field spelled with
        // another number.
        Ok(match self.position_by_name(field.name()) {
            Some(holder) => Route::Named(holder),
            None => Route::New,
        })
    }

    /// Folds `field` where [`Self::route`] landed it: `None` when it was this
    /// crate's own and so neither added nor merged, else whether it arrived.
    fn fold_routed(&mut self, route: Route, field: Field) -> Result<Option<bool>> {
        match route {
            Route::Own => Ok(None),
            Route::Identity(position) => {
                self.update_at(position, field, References::Defer, Naming::Stored)?;
                Ok(Some(false))
            }
            Route::Unnamed(position) => {
                self.update_at(position, field, References::Defer, Naming::Incoming)?;
                Ok(Some(false))
            }
            Route::Named(position) => {
                self.merge_named(position, field)?;
                Ok(Some(false))
            }
            Route::Beside | Route::New => {
                self.insert_resolved(field, References::Defer)?;
                Ok(Some(true))
            }
        }
    }

    /// Removes an unreferenced field a tag, identifier, name, or alias reaches.
    ///
    /// Returns no removed value for an absent or referenced field: a
    /// definition another one references stays, as its members stay.
    pub fn remove<'key>(&mut self, key: impl Into<FixKey<'key>>) -> Option<Field> {
        let key = key.into();
        let mut staged = self.clone();
        let removed = match staged.remove_resolved(key) {
            Some(removed) => removed,
            // A name no scalar answers to may name a component or a group.
            None => {
                let FixKey::Name(name) = key else {
                    return None;
                };
                return [crate::FixCategory::Components, crate::FixCategory::Groups]
                    .into_iter()
                    .find_map(|category| self.remove_definition(category, name).ok().flatten());
            }
        };
        staged.validate_catalog().ok()?;
        staged.refresh_msgtype_aliases();
        *self = staged;
        Some(removed)
    }

    pub(super) fn remove_resolved<'key>(&mut self, key: impl Into<FixKey<'key>>) -> Option<Field> {
        let position = match key.into() {
            FixKey::Tag(tag) => self.position_by_tag(tag),
            FixKey::Id(id) => self.canonical_position_by_id(id),
            FixKey::Name(name) => self.position_by_name(name),
        }?;
        let last = self.fields.len().checked_sub(1)?;
        if position != last {
            self.unindex(last, last);
        }
        self.unindex(position, position);
        self.lifted_names.take();
        self.idmap_sources.take();
        self.parents.take();
        self.marketdatatypes.take();
        self.timeinforces.take();
        // The field departing may be the last, which `index` never touches
        // again: the memo forgets what it answered for it here.
        self.memo.clear();
        let removed = self.fields.swap_remove(position);
        let departed = self.identities.swap_remove(position);
        self.facts.swap_remove(position);
        if position != last {
            self.index(position);
        }
        // The tag the departed field answered passes to the next field
        // holding it canonically, if any does - the earliest arrival among
        // them, which then moves to the front of its tag in the order.
        if let Some((tag, _)) = departed
            && !self.tags.contains_key(&tag)
            && let Some(next) = self
                .identities
                .iter()
                .position(|held| held.is_some_and(|(held, _)| held == tag))
        {
            self.tags.insert(tag, next);
            self.reorder();
        }
        Some(removed)
    }

    /// Returns the first field after `after`, in the iteration order:
    /// tag-major, the tag's holder first, then by identity.
    ///
    /// `None` for an identity this registry does not hold, which has no
    /// place in its order.
    pub fn next_field_after(&self, after: Option<FixId>) -> Option<&Field> {
        let position = match after {
            None => 0,
            Some(after) => {
                let at = self.canonical_position_by_id(after)?;
                let held = self.order_of(at)?;
                self.positions_by_id.partition_point(|position| {
                    self.order_of(*position)
                        .is_some_and(|candidate| candidate <= held)
                })
            }
        };
        self.positions_by_id
            .get(position)
            .and_then(|field| self.fields.get(*field))
    }

    /// Where the field at `position` stands in the iteration order.
    ///
    /// Tag-major, the tag's holder before every other field on the tag,
    /// then by identity. The holder is written first by a store and loaded
    /// first by its reader, which is what keeps the bare tag answering the
    /// field that held it across a round trip.
    fn order_of(&self, position: usize) -> Option<(i32, bool, FixId)> {
        let (tag, id) = self.identities.get(position).copied().flatten()?;
        Some((tag, self.tags.get(&tag) != Some(&position), id))
    }

    /// Restores the iteration order after a change to which field holds a
    /// tag, which is rare enough that a sort is cheaper than tracking it.
    fn reorder(&mut self) {
        let mut ordered = std::mem::take(&mut self.positions_by_id);
        ordered.sort_by_key(|position| self.order_of(*position));
        self.positions_by_id = ordered;
    }

    /// Iterates every field: the scalar fields in tag-major identity order,
    /// then the components and the groups in the catalog's name order.
    pub fn iter(&self) -> FixFieldIter<'_> {
        FixFieldIter {
            positions: self.positions_by_id.iter(),
            fields: &self.fields,
            rest: self
                .catalog
                .all()
                .map(|entry| entry.field.as_field())
                .collect::<Vec<_>>()
                .into_iter(),
        }
    }

    /// Iterates the scalar fields alone, in tag-major identity order.
    pub(super) fn scalars(&self) -> FixFieldIter<'_> {
        FixFieldIter {
            positions: self.positions_by_id.iter(),
            fields: &self.fields,
            rest: Vec::new().into_iter(),
        }
    }

    /// Returns the number of fields, the components and the groups among
    /// them.
    pub fn len(&self) -> usize {
        self.fields.len() + self.catalog.all().count()
    }

    /// Returns whether no field is registered.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.catalog.all().next().is_none()
    }

    pub(super) fn canonical_position_by_id(&self, id: FixId) -> Option<usize> {
        self.ids.get(&id).copied()
    }

    /// The position holding `field`'s own identity: an id hit, rechecked
    /// against the tag and the folded name before it counts.
    ///
    /// The id is thirty-two bits, so two identities can digest to one; a
    /// hit that is not this field is not this field's position, and what a
    /// caller does with it - replace, update, refuse - must not land on
    /// another field. A bare id asked from outside has nothing to recheck
    /// against, which is why this is the door every mutation takes.
    ///
    /// # Errors
    ///
    /// Returns the identity's own refusal for a field without a tag.
    fn position_of_identity(&self, field: &Field) -> Result<Option<usize>> {
        let (tag, id) = canonical_identity(field)?;
        Ok(self.ids.get(&id).copied().filter(|position| {
            self.identities
                .get(*position)
                .copied()
                .flatten()
                .is_some_and(|(held, _)| held == tag)
                && self.canonical_name_matches(*position, field.name())
        }))
    }

    pub(super) fn canonical_position_by_name(&self, name: &str) -> Option<usize> {
        let position = *self.names.get(&name_digest(name, NAME_SEED))?;
        self.canonical_name_matches(position, name)
            .then_some(position)
    }

    fn position_by_tag(&self, tag: i32) -> Option<usize> {
        self.tags
            .get(&tag)
            .or_else(|| self.alternate_tags.get(&tag))
            .copied()
    }

    fn position_by_name(&self, name: &str) -> Option<usize> {
        self.canonical_position_by_name(name)
            .or_else(|| self.alias_position_by_name(name))
    }

    /// [`Self::position_by_name`], else through the word aliases: the one
    /// lookup a spelled name is read by. A registry's own writes - a fold,
    /// an alias, a removal - address a field by its exact name alone.
    fn read_position_by_name(&self, name: &str) -> Option<usize> {
        self.position_by_name(name)
            .or_else(|| super::aliases::aliased(name, |spelled| self.position_by_name(spelled)))
    }

    fn alias_position_by_name(&self, name: &str) -> Option<usize> {
        let position = *self.aliases.get(&name_digest(name, ALIAS_SEED))?;
        self.alias_matches(position, name).then_some(position)
    }

    /// Whether the field at `position` is the one `name` names.
    ///
    /// The recheck behind a digest hit, so it tests exactly the equivalence
    /// [`name_digest`] hashes - the crate's one fold - and nothing narrower:
    /// a spelling the index keys as a stored name and the recheck then
    /// refuses would be neither found nor insertable.
    fn canonical_name_matches(&self, position: usize, name: &str) -> bool {
        self.fields
            .get(position)
            .is_some_and(|field| folds_equal(field.name(), name))
    }

    fn alias_matches(&self, position: usize, name: &str) -> bool {
        self.fields
            .get(position)
            .is_some_and(|field| field.as_fix().names().any(|alias| folds_equal(alias, name)))
    }

    /// The canonical tag and identity of one of this registry's own fields,
    /// read off the index rather than out of the field's metadata.
    ///
    /// A borrowed field the registry answered points into its own storage,
    /// and that position remembers the identity; a field it does not hold - a
    /// definition's member, a message's child - answers what its own
    /// metadata says, exactly as [`FixField::id`](crate::FixField::id) does.
    pub(super) fn identity_of(&self, field: &Field) -> Option<(i32, FixId)> {
        match self.position_of_own(field) {
            Some(at) => self.identities.get(at).copied().flatten(),
            None => canonical_identity(field).ok(),
        }
    }

    /// The position of a borrowed field that points into this registry's
    /// own storage.
    fn position_of_own(&self, field: &Field) -> Option<usize> {
        let start = self.fields.as_ptr() as usize;
        let at =
            (std::ptr::from_ref(field) as usize).wrapping_sub(start) / std::mem::size_of::<Field>();
        self.fields
            .get(at)
            .filter(|held| std::ptr::eq(*held, field))
            .map(|_| at)
    }

    /// What `field`'s metadata states, read off the index: for one of this
    /// registry's own fields, and for a field sharing its metadata with
    /// one - a message's child stated under the dictionary's field - so a
    /// reader of a million rows reads the dictionary once. Nothing for a
    /// field the registry does not know, whose metadata the reader asks
    /// as it always did.
    pub(super) fn facts_of(&self, field: &Field) -> Option<FieldFacts> {
        if let Some(at) = self.position_of_own(field) {
            return self.facts.get(at).copied().flatten();
        }
        let storage = field.as_metadata().storage_address();
        if let Some(position) = self.by_metadata.get(&storage).copied()
            && let Some(held) = self.fields.get(position)
            && held.as_metadata().shares_storage_with(field.as_metadata())
        {
            return self.facts.get(position).copied().flatten();
        }
        self.catalog.facts_of(field)
    }

    /// Every key `field` holds proven free of every field but `owner`.
    ///
    /// The identity, because two fields cannot be one; the canonical name,
    /// because a name reaches one field; each alternate tag and each alias,
    /// because a second spelling reaches one field too. The canonical tag is
    /// not among them: a tag two fields hold under different names is the
    /// one thing this namespace admits twice, answering the first holder.
    fn check_free(
        &self,
        field: &Field,
        tag: i32,
        id: FixId,
        alternate: &[i32],
        owner: Option<usize>,
    ) -> Result<()> {
        let other = |position: &usize| Some(*position) != owner;
        if let Some(holder) = self.ids.get(&id).filter(|position| other(position)) {
            return Err(conflict(
                Held::Id(tag, field.name()),
                field,
                &self.fields[*holder],
            ));
        }
        let name = name_digest(field.name(), NAME_SEED);
        if let Some(holder) = self.names.get(&name).filter(|position| other(position)) {
            return Err(conflict(
                Held::Name(field.name()),
                field,
                &self.fields[*holder],
            ));
        }
        for alternate in alternate {
            if let Some(group) = self.get_group_by_tag(*alternate).filter(|group| {
                matches!(
                    group.dtype(),
                    crate::DataType::Map(_) | crate::DataType::SortedMap(_)
                )
            }) {
                return Err(Error::conflict(
                    "a scalar alternate tag free of Map group counters",
                    "Map group",
                    format_args!(
                        "{}: alternate tag {alternate} belongs to {}",
                        field.name(),
                        group.name()
                    ),
                ));
            }
            if let Some(holder) = self
                .alternate_tags
                .get(alternate)
                .filter(|position| other(position))
            {
                return Err(conflict(
                    Held::AlternateTag(*alternate),
                    field,
                    &self.fields[*holder],
                ));
            }
        }
        for alias in field.as_fix().names() {
            let key = name_digest(alias, ALIAS_SEED);
            if let Some(holder) = self.aliases.get(&key).filter(|position| other(position)) {
                return Err(conflict(Held::Alias(alias), field, &self.fields[*holder]));
            }
        }
        Ok(())
    }

    fn index(&mut self, position: usize) {
        // Every change to the fields lands here, so what was answered off
        // them is forgotten here too.
        self.lifted_names.take();
        self.idmap_sources.take();
        self.parents.take();
        self.marketdatatypes.take();
        self.timeinforces.take();
        self.memo.clear();
        let Some(field) = self.fields.get(position) else {
            return;
        };
        let view = field.as_fix();
        // Remembered first, whatever the field answers: every position has
        // an entry, so a field that indexes nothing still identifies nothing.
        let identity = canonical_identity(field).ok();
        if position >= self.identities.len() {
            self.identities.resize(position + 1, None);
        }
        self.identities[position] = identity;
        let facts = FieldFacts::of(field);
        if position >= self.facts.len() {
            self.facts.resize(position + 1, None);
        }
        self.facts[position] = facts;
        self.by_metadata
            .insert(field.as_metadata().storage_address(), position);
        let Some((tag, id)) = identity else {
            return;
        };
        self.ids.insert(id, position);
        // The first holder of a tag keeps it.
        self.tags.entry(tag).or_insert(position);
        for alternate in view.tags().unwrap_or_default() {
            self.alternate_tags.insert(alternate, position);
        }
        self.names
            .insert(name_digest(field.name(), NAME_SEED), position);
        for alias in view.names() {
            self.aliases
                .insert(name_digest(alias, ALIAS_SEED), position);
        }
        let key = (tag, self.tags.get(&tag) != Some(&position), id);
        let ordered = self
            .positions_by_id
            .partition_point(|held| self.order_of(*held).is_some_and(|held| held < key));
        self.positions_by_id.insert(ordered, position);
    }

    fn unindex(&mut self, position: usize, pointing_at: usize) {
        let Some(field) = self.fields.get(position) else {
            return;
        };
        let storage = field.as_metadata().storage_address();
        if self.by_metadata.get(&storage) == Some(&pointing_at) {
            self.by_metadata.remove(&storage);
        }
        let view = field.as_fix();
        if let Ok((tag, id)) = canonical_identity(field) {
            if self.ids.get(&id) == Some(&pointing_at) {
                self.ids.remove(&id);
            }
            if self.tags.get(&tag) == Some(&pointing_at) {
                self.tags.remove(&tag);
            }
        }
        for tag in view.tags().unwrap_or_default() {
            if self.alternate_tags.get(&tag) == Some(&pointing_at) {
                self.alternate_tags.remove(&tag);
            }
        }
        let name = name_digest(field.name(), NAME_SEED);
        if self.names.get(&name) == Some(&pointing_at) {
            self.names.remove(&name);
        }
        for alias in view.names() {
            let key = name_digest(alias, ALIAS_SEED);
            if self.aliases.get(&key) == Some(&pointing_at) {
                self.aliases.remove(&key);
            }
        }
        if let Some(index) = self
            .positions_by_id
            .iter()
            .position(|held| *held == pointing_at)
        {
            self.positions_by_id.remove(index);
        }
    }
}

impl fmt::Debug for FixRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut map = formatter.debug_map();
        for position in &self.positions_by_id {
            let Some(field) = self.fields.get(*position) else {
                continue;
            };
            let Some((tag, _)) = self.identities.get(*position).copied().flatten() else {
                continue;
            };
            map.entry(&format_args!("{tag} {}", field.name()), field);
        }
        map.finish()
    }
}

impl PartialEq for FixRegistry {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self.iter().eq(other.iter())
            && self.catalog == other.catalog
            && self.codesets == other.codesets
            && self.sources == other.sources
    }
}

impl Eq for FixRegistry {}

impl FixRegistry {
    /// The registry's reading of tag 385: its code set and which code the
    /// prose in front of a payload names.
    ///
    /// Built from what the registry holds when asked, so a code set edited
    /// on tag 385 is the set the next reading answers; a codec compiles it
    /// once when it takes its registry.
    #[must_use]
    pub fn msgdirection(&self) -> super::MsgDirection {
        super::MsgDirection::from_registry(self)
    }

    /// The currency pair each symbol names, as FX detection reads it.
    pub(super) const fn forex_memo(&self) -> &super::forex::FxMemo {
        &self.forex
    }

    /// Forgets what was answered off the fields and the catalog: what the
    /// next reader asks is answered off them as they stand then.
    ///
    /// Every field change reaches [`Self::index`] and forgets them there; a
    /// catalog change that lands without staging a clone calls this.
    pub(super) fn forget_answers(&mut self) {
        self.lifted_names.take();
        self.idmap_sources.take();
        self.parents.take();
        self.marketdatatypes.take();
        self.timeinforces.take();
        self.memo.clear();
    }

    /// What this dictionary has answered about itself.
    pub(super) fn memo(&self) -> &super::memo::Memo {
        &self.memo
    }

    /// The identifier-map sources the dictionary states, field by field in
    /// iteration order: each the tag of the field whose value names a
    /// message and the [`FIX:idmap`](crate::FixField::idmap) entry saying in
    /// which map and under which key. A message rebuilds its `identifiers`
    /// from these at every settle, and an operation that follows another
    /// carries the keys whose entry follows. Compiled once and forgotten by every change to the
    /// fields.
    ///
    /// ```
    /// use yggdryl::fix::FixIdMapKind;
    /// use yggdryl::{FixRegistry, IdType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let registry = FixRegistry::from_handle(&yggdryl::local::LocalFolder::new(
    ///     std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix"),
    /// )?)?;
    /// let (tag, order) = registry
    ///     .idmap_sources()
    ///     .iter()
    ///     .find(|(_, source)| source.key() == &IdType::OrderId)
    ///     .expect("OrderID names the order");
    /// assert_eq!((*tag, order.map(), order.follows()), (37, FixIdMapKind::Identifiers, true));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn idmap_sources(&self) -> &[(i32, super::FixIdSource)] {
        self.idmap_sources.get_or_init(|| {
            self.iter()
                .filter_map(|field| Some((field.as_fix().tag().ok()??, field)))
                .flat_map(|(tag, field)| {
                    // Every document was read whole when the field was taken.
                    field
                        .as_fix()
                        .idmap()
                        .filter_map(Result::ok)
                        .map(move |source| (tag, source))
                })
                .collect()
        })
    }

    /// Every parents list this dictionary's fields state under
    /// `FIX:parents`, each under the identifier type its field states - the
    /// field's identifier-map key, else its own name - read once and
    /// forgotten by every change to the fields.
    ///
    /// A dictionary states what its own field names imply: wherever fields
    /// arrive - a store loading, a field inserted, updated or added, a
    /// dictionary or a `CBlock` merged - a field named as another's parent
    /// (`parent` or `orig` before that field's name, as
    /// [`IdType::parent_of`](crate::IdType::parent_of) reads it, which names
    /// a parent of a chain identity alone) is listed among that field's
    /// `FIX:parents`, a `parent` type before an `orig` one, beside what the
    /// field already states. Each parent field answers to its own name
    /// alone: `ParentOrderID` beside `OrigOrderID` is a second parent, never
    /// the first spelled another way. So `OrigClOrdID(41)` makes
    /// `ClOrdID(11)`'s parents `origclordid`, `OrigTradeID(1126)`
    /// `TradeID(1003)`'s `origtradeid`, and an identifier no field is a
    /// parent of by name keeps the parents its type has
    /// ([`IdType::parents`](crate::IdType::parents)) -
    /// `TradeReportID(571)`'s `tradereportrefid`, which no field states.
    ///
    /// ```
    /// use yggdryl::{FixRegistry, IdType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let registry = FixRegistry::from_handle(&yggdryl::local::LocalFolder::new(
    ///     std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix"),
    /// )?)?;
    /// let clordid = registry.parent_sources().iter().find(|(base, _)| *base == IdType::ClOrdId);
    /// assert_eq!(clordid.map(|(_, parents)| parents.as_ref()), Some([IdType::OrigClOrdId].as_slice()));
    /// let parents = |base: IdType| -> Vec<String> {
    ///     registry.parents_of(&base).iter().map(ToString::to_string).collect()
    /// };
    /// assert_eq!(parents(IdType::TradeId), ["origtradeid"], "OrigTradeID(1126)");
    /// assert_eq!(parents(IdType::OrderId), ["parentorderid", "origorderid"], "no field: the name's");
    ///
    /// // Fields arriving name their parents to the fields already held.
    /// let field = |name: &str, tag: i32| -> yggdryl::Result<yggdryl::Field> {
    ///     let mut field = yggdryl::DataType::utf8().nullable_field(name);
    ///     field.as_fix_mut().set_tag(tag)?;
    ///     Ok(field)
    /// };
    /// let mut venue = FixRegistry::from_fields([field("OrderID", 37)?, field("OrigOrderID", 9001)?])?;
    /// let stated = |venue: &FixRegistry| -> yggdryl::Result<Vec<String>> {
    ///     Ok(venue.field_by_tag(37)?.as_fix().parents().map(str::to_owned).collect())
    /// };
    /// let tag_of = |venue: &FixRegistry, name: &str| -> yggdryl::Result<Option<i32>> {
    ///     venue.field_by_name(name)?.as_fix().tag()
    /// };
    /// assert_eq!(stated(&venue)?, ["origorderid"]);
    /// // The one parent answers to its own name alone...
    /// assert!(venue.field_by_name("ParentOrderID").is_err());
    /// assert!(venue.field_by_tag(9001)?.as_fix().names().next().is_none());
    /// // ...so a field of the other spelling is a second parent.
    /// venue.add_field(field("ParentOrderID", 9002)?)?;
    /// assert_eq!(stated(&venue)?, ["parentorderid", "origorderid"]);
    /// assert_eq!(tag_of(&venue, "ParentOrderID")?, Some(9002));
    /// assert_eq!(tag_of(&venue, "OrigOrderID")?, Some(9001));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn parent_sources(&self) -> &[(crate::IdType, Box<[crate::IdType]>)] {
        self.parents.get_or_init(|| {
            self.iter()
                .filter_map(|field| {
                    let parents: Box<[crate::IdType]> = field
                        .as_fix()
                        .parents()
                        .filter_map(|word| word.parse().ok())
                        .collect();
                    if parents.is_empty() {
                        return None;
                    }
                    Some((identifier_type(field)?, parents))
                })
                .collect()
        })
    }

    /// The parent types of `base`, nearest first: the list a field stating
    /// that identifier type states under `FIX:parents`, else the ones its
    /// name has ([`IdType::parents`](crate::IdType::parents)).
    #[must_use]
    pub fn parents_of(&self, base: &crate::IdType) -> Cow<'_, [crate::IdType]> {
        match self.parent_sources().iter().find(|(held, _)| held == base) {
            Some((_, parents)) => Cow::Borrowed(parents),
            None => base.parents(),
        }
    }

    /// The base `kind` is a parent of and its place among the base's
    /// [`Self::parents_of`]: a list this dictionary states first, then the
    /// name's reading ([`IdType::parent_of`](crate::IdType::parent_of)) for
    /// a base that states none.
    #[must_use]
    pub fn parent_of(&self, kind: &crate::IdType) -> Option<(crate::IdType, usize)> {
        let sources = self.parent_sources();
        sources
            .iter()
            .find_map(|(base, parents)| {
                let at = parents.iter().position(|held| held == kind)?;
                Some((base.clone(), at))
            })
            .or_else(|| {
                kind.parent_of()
                    .filter(|(base, _)| !sources.iter().any(|(held, _)| held == base))
            })
    }

    /// States, on every field this dictionary holds a parent of by name,
    /// that parent among its `FIX:parents` ([`Self::parent_sources`]): what
    /// a dictionary that took fields in from elsewhere runs once before its
    /// definitions resolve. Whether any field moved.
    pub(super) fn state_parents(&mut self) -> bool {
        let mut bases: Vec<usize> = (0..self.fields.len())
            .filter_map(|at| self.parent_link(at).map(|(base, ..)| base))
            .collect();
        bases.sort_unstable();
        bases.dedup();
        let mut moved = false;
        for base in bases {
            moved |= self.reconcile_parents(base);
        }
        if moved {
            self.forget_answers();
        }
        moved
    }

    /// [`Self::state_parents`] around the one field at `position`: the base
    /// it is a parent of, and the field itself as a base, so inserting
    /// fields one at a time costs a few lookups each rather than a walk of
    /// the dictionary.
    fn state_parents_around(&mut self, position: usize) -> bool {
        let mut moved = false;
        if let Some((base, ..)) = self.parent_link(position) {
            moved |= self.reconcile_parents(base);
        }
        moved |= self.reconcile_parents(position);
        if moved {
            self.forget_answers();
        }
        moved
    }

    /// Brings the field at `base_at` in step with its parent fields: each
    /// field named as a parent its type has
    /// ([`IdType::parents`](crate::IdType::parents)) is listed among its
    /// `FIX:parents`. Whether anything moved.
    fn reconcile_parents(&mut self, base_at: usize) -> bool {
        let Some(base) = self.fields.get(base_at).and_then(identifier_type) else {
            return false;
        };
        let held: Vec<(usize, crate::IdType)> = base
            .parents()
            .iter()
            .filter_map(|parent| {
                let (linked, rank, kind) = self.parent_link(self.position_of_type(parent)?)?;
                (linked == base_at).then_some((rank, kind))
            })
            .collect();
        let mut moved = false;
        for (rank, kind) in held {
            moved |= self.state_parent(base_at, rank, kind);
        }
        moved
    }

    /// The field one field is a parent of by name, the place the parent
    /// takes among that field's parents, and the parent's type:
    /// `OrigClOrdID` is `ClOrdID`'s first. A field whose name opens as no
    /// parent's is none, though its type has a base - `TradeReportRefID` is
    /// `TradeReportID`'s parent by the vocabulary, which no field states.
    fn parent_link(&self, position: usize) -> Option<(usize, usize, crate::IdType)> {
        let field = self.fields.get(position)?;
        if !is_parent_named(field) {
            return None;
        }
        let kind = identifier_type(field)?;
        let (base, rank) = kind.parent_of()?;
        let at = self.position_of_type(&base)?;
        (at != position).then_some((at, rank, kind))
    }

    /// The field named as `kind` is spelled that states it as its
    /// identifier type.
    fn position_of_type(&self, kind: &crate::IdType) -> Option<usize> {
        self.position_by_name(kind.as_str())
            .filter(|at| identifier_type(&self.fields[*at]).as_ref() == Some(kind))
    }

    /// Lists `kind` among the parents of the field at `at`, where the
    /// field does not list it yet, in the place its `rank` gives it among
    /// the ones the names say - a `parent` type before an `orig` one;
    /// whether it moved.
    fn state_parent(&mut self, at: usize, rank: usize, kind: crate::IdType) -> bool {
        let mut parents: Vec<crate::IdType> = self.fields[at]
            .as_fix()
            .parents()
            .filter_map(|word| word.parse().ok())
            .collect();
        if parents.contains(&kind) {
            return false;
        }
        let slot = parents
            .iter()
            .position(|held| held.parent_of().map_or(usize::MAX, |(_, held)| held) > rank)
            .unwrap_or(parents.len());
        parents.insert(slot, kind);
        self.fields[at]
            .as_fix_mut()
            .set_parents(parents.iter().map(crate::IdType::as_str))
            .is_ok()
    }

    /// Every wire value a field of this dictionary maps to a market data
    /// type through its `FIX:marketdatatype`, as the field's tag, the value
    /// and the member, read once.
    #[must_use]
    pub fn marketdatatype_sources(&self) -> &[(i32, SmolStr, crate::MarketDataType)] {
        self.marketdatatypes.get_or_init(|| {
            self.iter()
                .filter_map(|field| Some((field.as_fix().tag().ok()??, field)))
                .flat_map(|(tag, field)| {
                    field
                        .as_fix()
                        .marketdatatypes()
                        .map(move |(wire, member)| (tag, SmolStr::new(wire), member))
                })
                .collect()
        })
    }

    /// The market data type one wire value of the field under `tag` types an
    /// element as: the member this dictionary's `FIX:marketdatatype` maps it
    /// to, else the crate's own reading of `OrdType(40)`, `QuoteType(537)`,
    /// `TrdType(828)` and `MDEntryType(269)`
    /// ([`MarketDataType::from_fix`](crate::MarketDataType::from_fix));
    /// `None` for a field that types nothing.
    ///
    /// ```
    /// use yggdryl::{DataType, FixRegistry, MarketDataType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut ordtype = DataType::utf8().nullable_field("ordtype");
    /// ordtype.as_fix_mut().set_tag(40)?;
    /// ordtype
    ///     .as_fix_mut()
    ///     .set_marketdatatypes(&[("Z", MarketDataType::OrdPegged)])?;
    /// let registry = FixRegistry::from_fields([ordtype])?;
    /// assert_eq!(registry.marketdatatype_of(40, "Z"), Some(MarketDataType::OrdPegged));
    /// assert_eq!(registry.marketdatatype_of(40, "2"), Some(MarketDataType::OrdLimit));
    /// assert_eq!(registry.marketdatatype_of(54, "1"), None);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn marketdatatype_of(&self, tag: i32, wire: &str) -> Option<crate::MarketDataType> {
        self.marketdatatype_sources()
            .iter()
            .find(|(held, value, _)| *held == tag && value == wire)
            .map(|(_, _, member)| *member)
            .or_else(|| crate::MarketDataType::from_fix(tag, wire))
    }

    /// Every wire value a field of this dictionary maps to a time in force
    /// through its `FIX:timeinforce`, as the field's tag, the value and the
    /// member, read once.
    #[must_use]
    pub fn timeinforce_sources(&self) -> &[(i32, SmolStr, crate::TimeInForce)] {
        self.timeinforces.get_or_init(|| {
            self.iter()
                .filter_map(|field| Some((field.as_fix().tag().ok()??, field)))
                .flat_map(|(tag, field)| {
                    field
                        .as_fix()
                        .timeinforces()
                        .map(move |(wire, member)| (tag, SmolStr::new(wire), member))
                })
                .collect()
        })
    }

    /// The time in force one wire value of the field under `tag` stands
    /// for: the member this dictionary's `FIX:timeinforce` maps it to, else
    /// the crate's own reading of `TimeInForce(59)`
    /// ([`TimeInForce::from_fix`](crate::TimeInForce::from_fix)), a value
    /// it names no member for being `OTHER`; `None` for a field that states
    /// none.
    ///
    /// ```
    /// use yggdryl::{DataType, FixRegistry, TimeInForce};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut tif = DataType::utf8().nullable_field("timeinforce");
    /// tif.as_fix_mut().set_tag(59)?;
    /// tif.as_fix_mut()
    ///     .set_timeinforces(&[("G", TimeInForce::GoodTillCancel)])?;
    /// let registry = FixRegistry::from_fields([tif])?;
    /// assert_eq!(registry.timeinforce_of(59, "G"), Some(TimeInForce::GoodTillCancel));
    /// assert_eq!(registry.timeinforce_of(59, "3"), Some(TimeInForce::ImmediateOrCancel));
    /// assert_eq!(registry.timeinforce_of(59, "Q"), Some(TimeInForce::Other));
    /// assert_eq!(registry.timeinforce_of(54, "1"), None);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn timeinforce_of(&self, tag: i32, wire: &str) -> Option<crate::TimeInForce> {
        self.timeinforce_sources()
            .iter()
            .find(|(held, value, _)| *held == tag && value == wire)
            .map(|(_, _, member)| *member)
            .or_else(|| (tag == 59).then(|| crate::TimeInForce::from_fix(wire)))
    }

    /// The names a message digests its lifted facts under, read once.
    pub(super) fn lifted_names(&self) -> &LiftedNames {
        self.lifted_names.get_or_init(|| LiftedNames {
            msgtype: self
                .get_field_by_tag(super::MSGTYPE_TAG_NAME.0)
                .map_or_else(
                    || SmolStr::new_static(super::MSGTYPE_TAG_NAME.1),
                    |field| SmolStr::new(field.name()),
                ),
            lifted: super::identity::LIFTED_TAGS
                .into_iter()
                .map(|tag| {
                    self.get_field_by_tag(tag).map_or_else(
                        || format_smolstr!("{tag}"),
                        |field| SmolStr::new(field.name()),
                    )
                })
                .collect(),
        })
    }
}

impl Clone for FixRegistry {
    /// A clone holds the same fields and catalog and none of what was
    /// answered off them: every mutation stages itself on a clone before
    /// replacing the registry, so a remembered answer never outlives the
    /// dictionary it was read from, and a clone taken to read remembers its
    /// own.
    fn clone(&self) -> Self {
        Self {
            fields: self.fields.clone(),
            catalog: self.catalog.clone(),
            codesets: self.codesets.clone(),
            sources: self.sources.clone(),
            ids: self.ids.clone(),
            tags: self.tags.clone(),
            alternate_tags: self.alternate_tags.clone(),
            names: self.names.clone(),
            aliases: self.aliases.clone(),
            positions_by_id: self.positions_by_id.clone(),
            identities: self.identities.clone(),
            facts: self.facts.clone(),
            by_metadata: self.by_metadata.clone(),
            memo: super::memo::Memo::new(),
            forex: super::forex::FxMemo::new(),
            lifted_names: OnceLock::new(),
            idmap_sources: OnceLock::new(),
            parents: OnceLock::new(),
            marketdatatypes: OnceLock::new(),
            timeinforces: OnceLock::new(),
            renamed: self.renamed.clone(),
        }
    }
}

impl Hash for FixRegistry {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.len().hash(state);
        for field in self {
            field.hash(state);
        }
        // The vocabularies the fields read by: two dictionaries whose fields
        // agree but whose sets do not are two dictionaries, and a name a
        // field states means whatever the set under it says.
        self.codesets.len().hash(state);
        for (name, document) in &self.codesets {
            name.hash(state);
            document.hash(state);
        }
        // The sources the fields name: an entry states what the id on a
        // field does not, so two dictionaries whose entries differ are two.
        self.sources.len().hash(state);
        for source in self.sources.values() {
            source.hash(state);
        }
        for category in [crate::FixCategory::Components, crate::FixCategory::Groups] {
            category.hash(state);
            self.catalog.iter(category).count().hash(state);
            for field in self.catalog.iter(category) {
                field.hash(state);
            }
        }
    }
}

impl<'registry> IntoIterator for &'registry FixRegistry {
    type Item = &'registry Field;
    type IntoIter = FixFieldIter<'registry>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// The fields of a registry in ascending tag-major identifier order.
#[derive(Clone, Debug)]
pub struct FixFieldIter<'registry> {
    positions: std::slice::Iter<'registry, usize>,
    fields: &'registry [Field],
    /// The components and the groups, behind the scalars.
    rest: std::vec::IntoIter<&'registry Field>,
}

impl<'registry> Iterator for FixFieldIter<'registry> {
    type Item = &'registry Field;

    fn next(&mut self) -> Option<Self::Item> {
        self.positions
            .next()
            .and_then(|position| self.fields.get(*position))
            .or_else(|| self.rest.next())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let held = self.positions.len() + self.rest.len();
        (held, Some(held))
    }
}

impl DoubleEndedIterator for FixFieldIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.rest.next_back().or_else(|| {
            self.positions
                .next_back()
                .and_then(|position| self.fields.get(*position))
        })
    }
}

impl ExactSizeIterator for FixFieldIter<'_> {}
impl FusedIterator for FixFieldIter<'_> {}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/fix/registry.rs` and `rust/tests/fix/mod_.rs` pin and
    //! a caller cannot reach.
    //!
    //! A registry answers a tag, an identity and a name, and that is the whole
    //! of what a caller sees. What it is *made of* - the seeded digests, the
    //! five position indexes and the lifted-name cache - is reached here,
    //! because a collision and a warm cache cannot be staged from outside.
    use super::FixRegistry;
    use crate::{Field, FixId, Result};

    /// The seed canonical names are digested under.
    pub const NAME_SEED: u64 = super::NAME_SEED;

    /// The seed aliases are digested under.
    pub const ALIAS_SEED: u64 = super::ALIAS_SEED;

    /// The seeded XXH64 of a name under the crate's one fold.
    #[must_use]
    pub fn name_digest(name: &str, domain: u64) -> u64 {
        super::name_digest(name, domain)
    }

    /// The identity a field's canonical tag and name make.
    ///
    /// # Errors
    ///
    /// Returns a typed failure naming `FIX:tag` where the field states none
    /// or states a nonpositive one.
    pub fn canonical_id(field: &Field) -> Result<FixId> {
        super::canonical_id(field)
    }

    /// The byte an identity spreads over the shards of a store.
    #[must_use]
    pub fn control_byte(id: FixId) -> u8 {
        super::control_byte(id)
    }

    /// The fields a registry holds, in insertion order.
    #[must_use]
    pub fn fields(registry: &FixRegistry) -> &[Field] {
        &registry.fields
    }

    /// File `at` under `digest` in the canonical-name index, as a 64-bit
    /// digest collision would land it.
    pub fn force_name_index(registry: &mut FixRegistry, digest: u64, at: usize) {
        registry.names.insert(digest, at);
    }

    /// File `at` under `digest` in the alias index, as a 64-bit digest
    /// collision would land it.
    pub fn force_alias_index(registry: &mut FixRegistry, digest: u64, at: usize) {
        registry.aliases.insert(digest, at);
    }

    /// File `at` under `id` in the identity index, as a 32-bit digest
    /// collision would land it.
    pub fn force_id_index(registry: &mut FixRegistry, id: FixId, at: usize) {
        registry.ids.insert(id, at);
    }
}

/// The identifier type a field states: its identifier-map key, else its own
/// name folded - what its `FIX:parents` lists the parents of, and what a
/// parent field's name is read against.
fn identifier_type(field: &Field) -> Option<crate::IdType> {
    field
        .as_fix()
        .idmap()
        .filter_map(Result::ok)
        .find(|source| source.role().is_none())
        .map(|source| source.key().clone())
        .or_else(|| field.name().parse().ok())
}

/// Whether a field's name opens as a parent's does - `parent` or `orig`,
/// whatever the case - which is all a walk of the dictionary reads before
/// it folds the name.
fn is_parent_named(field: &Field) -> bool {
    let name = field.name().as_bytes();
    ["parent", "orig"].iter().any(|prefix| {
        name.get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
    })
}
