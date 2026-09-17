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

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::iter::FusedIterator;
use std::sync::{Arc, OnceLock};

use smol_str::format_smolstr;

use super::{FixId, FixKey};
use crate::hashing::xxhash::Xxh64;
use crate::types::folds_equal;
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
/// A list is stepped through before anything else, including the exact walk.
/// A repeating group's occurrence is not a path segment - nobody spelling a
/// path names it - so consulting [`Field::get_field_by_path`] first would let
/// the occurrence match by its own name, which is exactly what this walk must
/// not allow now that the occurrence carries the component's name. The exact
/// walk still runs under the list, because it is the cheap answer and the
/// common one.
pub(super) fn descend<'field>(
    field: &'field Field,
    segments: &[FieldSegment],
) -> Option<&'field Field> {
    let Some((head, rest)) = segments.split_first() else {
        return Some(field);
    };
    if let crate::DataType::List(item) | crate::DataType::LargeList(item) = field.dtype() {
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
    if let crate::DataType::Map(map) = field.dtype() {
        let FieldSegment::Key(_) = head else {
            return None;
        };
        return descend(map.entries().fields().get(1)?, rest);
    }
    let child = folded_child(field, segment_name(head)?)?;
    descend(child, rest)
}

/// The name a segment states, where it states one.
///
/// A schema is addressed by name: a position says which occurrence of a group
/// a caller means, and every occurrence holds the same field, so a position
/// names no child here and is spent by the list it stands on.
fn segment_name(segment: &FieldSegment) -> Option<&str> {
    segment.as_name()
}

/// One child by folded name, reaching through a group's occurrence.
///
/// A repeating group is a List of one Struct, so a member is that struct's
/// child and not the list's. The occurrence is transparent: it is recursed
/// through without consuming a segment and it is never matched by its own
/// name, because that name is the component's - `Parties.PartyID` names
/// tag 448 and must never answer the struct that happens to share its
/// spelling. 269 of the 521 shipped groups derive a name a member of their own
/// struct already carries, so matching the occurrence would shadow every one
/// of them silently.
fn folded_child<'field>(field: &'field Field, name: &str) -> Option<&'field Field> {
    if let crate::DataType::List(item) | crate::DataType::LargeList(item) = field.dtype() {
        return folded_child(item, name);
    }
    field
        .fields()
        .iter()
        .find(|held| folds_equal(held.name(), name))
}

#[cfg(test)]
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
fn datatype_disagreement(stored: &Field, id: FixId, incoming: &Field) -> Error {
    Error::InvalidRecord {
        path: incoming.name().into(),
        reason: crate::text::expected_got(
            format_args!("the datatype {} stored for {id}", stored.dtype()),
            incoming.dtype(),
        ),
    }
}

/// The identity a field enters the registry under, beside its tag.
pub(super) fn canonical_identity(field: &Field) -> Result<(i32, FixId)> {
    let view = field.as_fix();
    let tag = view
        .tag()?
        .ok_or_else(|| Error::absent("fix:tag", field.name()))?;
    Ok((tag, FixId::of(tag, field.name())?))
}

/// The identity a field enters the registry under.
pub(super) fn canonical_id(field: &Field) -> Result<FixId> {
    canonical_identity(field).map(|(_, id)| id)
}

/// FIX field definitions resolved by identity, tag or folded name.
pub struct FixRegistry {
    fields: Vec<Field>,
    pub(super) catalog: super::catalog::Catalog,
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
    /// The `fix:derivation` of every field, compiled once on the first
    /// enrichment or row fill and shared by every codec and message reading
    /// this registry - or the refusal that compile answered, kept the same
    /// way so a registry whose rules do not compile refuses every ask and
    /// compiles once; emptied by every change to the fields or the catalog,
    /// so an edited derivation is the one the next reader evaluates.
    derivations:
        OnceLock<std::result::Result<Arc<super::enrich::Derivations>, super::enrich::Refused>>,
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
            ids: Index::default(),
            tags: Index::default(),
            alternate_tags: Index::default(),
            names: Index::default(),
            aliases: Index::default(),
            positions_by_id: Vec::new(),
            identities: Vec::new(),
            derivations: OnceLock::new(),
        };
        match super::fix_crate_fields() {
            Ok(fields) => {
                for field in fields {
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
    /// canonically.
    pub fn get_field_by_name(&self, name: &str) -> Option<&Field> {
        self.position_by_name(name)
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

    /// The scalar field a canonical name or alias names, and nothing else.
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
            .filter(|group| matches!(group.dtype(), crate::DataType::Map(_)))
            .or_else(|| self.get_field_by_name(name))
            // Last, and only for a name nothing else answers: a List group is
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
    /// a list, so an indexed segment answers that item - every occurrence of
    /// a group has the field the item declares - and `Parties[0].PartyID`
    /// therefore reaches the member here as well as in the message that
    /// carries it.
    ///
    /// Nested segments continue through the schema walk, which recurses
    /// through a group's occurrence without consuming a segment.
    pub fn get_field_by_path(&self, path: &FieldPath) -> Option<&Field> {
        let (head, rest) = path.segments().split_first()?;
        let head = segment_name(head)?;
        if rest.is_empty() {
            if let Some(field) = self
                .get_message_field_by_name(head)
                // A canonical Map suppresses scalar aliases, but still
                // shares the named-root ambiguity check with components.
                .filter(|field| !matches!(field.dtype(), crate::DataType::Map(_)))
            {
                return Some(field);
            }
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

    /// Every dialect any field or named definition names as a contributor,
    /// folded, sorted, each once.
    ///
    /// Membership is provenance and this is its listing; nothing resolves
    /// through it. A registry holding only the specification's own fields
    /// answers nothing.
    pub fn dialects(&self) -> Vec<String> {
        let mut held: BTreeSet<String> = BTreeSet::new();
        for field in &self.fields {
            for dialect in field.as_fix().branches() {
                held.insert(dialect.to_owned());
            }
        }
        for field in self.catalog.all() {
            for dialect in field.field.as_fix().branches() {
                held.insert(dialect.to_owned());
            }
        }
        held.into_iter().collect()
    }

    /// Adds a field, replacing only an equal identity, under the stored
    /// spelling.
    ///
    /// A field arriving on a tag another field holds under another name is
    /// a field of its own and is added beside the holder, which gains the
    /// arrival's name as an alias: a bare wire tag keeps answering the
    /// holder, and the arrival is reached by its name or its identity.
    pub fn insert(&mut self, field: Field) -> Result<Option<Field>> {
        // A nested field is a component or a group, by its shape, and lands
        // among the named definitions.
        if let Some(category) = Self::definition_category_of(&field)? {
            return self.insert_definition(category, field);
        }
        if self.position_of_identity(&field)?.is_some() {
            let mut staged = self.clone();
            let prior = staged.insert_resolved(field)?;
            staged.validate_catalog()?;
            *self = staged;
            return Ok(prior);
        }
        self.insert_resolved(field)
    }

    fn insert_resolved(&mut self, mut field: Field) -> Result<Option<Field>> {
        self.validate_definition(crate::FixCategory::Fields, &field)?;
        let (tag, id) = canonical_identity(&field)?;
        let alternate = field.as_fix().tags()?;
        let replacing = self.position_of_identity(&field)?;
        self.check_free(&field, tag, id, &alternate, replacing)?;
        if let Some(position) = replacing {
            field.set_name(self.fields[position].name());
        }
        match replacing {
            Some(position) => self.replace_field(position, field, tag).map(Some),
            None => {
                // A tag another field already holds is not this field's to
                // answer; the holder learns the arrival's name instead.
                if let Some(holder) = self.tags.get(&tag).copied() {
                    self.lend_alias(holder, field.name())?;
                }
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

    /// Gives the field at `holder` one more spelling, the name of a field
    /// arriving on the tag it holds.
    ///
    /// Idempotent under the fold, and a spelling the holder already claims
    /// canonically is not lent to it twice.
    fn lend_alias(&mut self, holder: usize, name: &str) -> Result<()> {
        let stored = &self.fields[holder];
        if folds_equal(stored.name(), name)
            || stored
                .as_fix()
                .names()
                .any(|alias| folds_equal(alias, name))
        {
            return Ok(());
        }
        // One alias reaches one field, and `check_free` is what holds every
        // other acquisition to that. This is the one write that does not go
        // through it, so it states the rule itself: a spelling another field
        // already answers for is that field's and stays there. Noted rather
        // than refused, exactly as a contended tag is above - the arrival is
        // what the caller asked for, and the lent spelling is the courtesy
        // beside it. Taking it would repoint the one entry the index holds,
        // leaving the first holder unreachable under its own alias and
        // unable to be updated, and would delete that entry outright when
        // the borrower departed.
        if let Some(other) = self.alias_position_by_name(name) {
            if other != holder {
                log::debug!(
                    "alias {name:?} of {:?} stays with {:?}",
                    self.fields[holder].name(),
                    self.fields[other].name()
                );
                return Ok(());
            }
        }
        let stored = &self.fields[holder];
        let mut lent = stored.clone();
        let aliases: Vec<String> = lent
            .as_fix()
            .names()
            .map(str::to_owned)
            .chain(std::iter::once(name.to_owned()))
            .collect();
        lent.as_fix_mut().set_names(&aliases)?;
        self.replace_field(
            holder,
            lent,
            self.identities[holder].map_or(0, |(tag, _)| tag),
        )?;
        Ok(())
    }

    /// Merges a definition into the field with the same canonical identity.
    /// A name folding to the stored one retains the stored canonical spelling.
    pub fn update(&mut self, field: Field) -> Result<()> {
        if let Some(category) = Self::definition_category_of(&field)? {
            self.update_definition(category, field)?;
            return Ok(());
        }
        let mut staged = self.clone();
        staged.update_resolved(field)?;
        staged.validate_catalog()?;
        *self = staged;
        Ok(())
    }

    fn update_resolved(&mut self, field: Field) -> Result<()> {
        self.validate_definition(crate::FixCategory::Fields, &field)?;
        let (tag, id) = canonical_identity(&field)?;
        let Some(position) = self.position_of_identity(&field)? else {
            return Err(absent(FixKey::Id(id)));
        };
        let stored = &self.fields[position];
        if stored.dtype() != field.dtype() {
            return Err(datatype_disagreement(stored, id, &field));
        }
        // The incoming definition is what the merge folds the stored one
        // into, so the caller's ordering is the precedence: a generator
        // merging its lowest-priority source first leaves the highest as the
        // last `update`, which wins.
        //
        // Two halves, because the `fix:` view reaches only its own namespace
        // by design. The generic keys fold through the metadata merge every
        // protocol shares, and the `fix:` keys through the rule each one has.
        let mut merged = field.clone();
        merged.set_name(stored.name());
        merged.set_metadata(field.as_metadata().merge_with(stored.as_metadata())?.iter())?;
        merged.as_fix_mut().merge_with(&stored.as_fix())?;
        let alternate = merged.as_fix().tags()?;
        self.check_free(&merged, tag, id, &alternate, Some(position))?;
        self.replace_field(position, merged, tag)?;
        Ok(())
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
            return Err(datatype_disagreement(stored, id, &field));
        }
        let mut merged = field.clone();
        merged.set_name(stored.name());
        merged.set_nullable(stored.is_nullable());
        merged.set_metadata(field.as_metadata().merge_with(stored.as_metadata())?.iter())?;
        // The identity is the stored field's, and it is written before the
        // `fix:` fold, which holds both sides to one tag.
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
        self.replace_field(position, merged, tag)?;
        Ok(())
    }

    /// Puts `merged` where the field at `position` was, answering what stood
    /// there.
    ///
    /// `merged` carries the tag `tag` the position already answers to, and
    /// the caller has proven every key it holds free. A change to the
    /// metadata alone is what the catalog's references restate, so that one
    /// re-resolves them; a change to the datatype is one they refuse, and the
    /// caller's validation is what refuses it.
    fn replace_field(&mut self, position: usize, merged: Field, tag: i32) -> Result<Field> {
        let refresh = metadata_only_change(&self.fields[position], &merged);
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
    ///    `List` or `LargeList` of non-null Struct occurrences is a group, a
    ///    Struct declaring `fix:msgtype` is a message, any other Struct is a
    ///    component. Any other nested datatype is refused as a scalar field
    ///    would refuse it.
    /// 2. One of this crate's own tags is every dictionary's already, so it is
    ///    neither added nor merged: skipped, answering `false`.
    /// 3. An identity the dictionary holds - the tag and the folded name
    ///    together - merges exactly as [`Self::update`] merges: the datatype
    ///    must equal the stored one, and the incoming metadata wins a shared
    ///    key.
    /// 4. Otherwise a name that folds to a stored field's canonical name or
    ///    to one of its aliases merges *into* that field: the same field
    ///    spelled with another tag. Folding is the crate's one fold, the one
    ///    every name lookup resolves by: ASCII case, and the `_`, `-` and
    ///    space separators, so `party_id` is a spelling of `PartyID`. The
    ///    stored field keeps its identity, its name and its nullability;
    ///    aliases and alternate tags are the union, the stored ones first,
    ///    deduplicated under the same fold; the incoming name joins the
    ///    aliases unless it is the canonical one; the incoming canonical tag
    ///    joins the alternate tags unless a field already answers it, in
    ///    which case it is left out and noted through `log` at debug level;
    ///    generic metadata and the `fix:` keys fold with the precedence of
    ///    rule 3. A datatype that disagrees is refused as rule 3 refuses it.
    /// 5. Otherwise it is inserted - beside the holder of its tag where one
    ///    holds it under another name, which then gains the arrival's name
    ///    as an alias while the bare tag keeps answering the holder.
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
    /// `fix:tag`, a conflict for an alias or alternate tag another field
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
    /// use yggdryl::{DataType, FixRegistry};
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
    /// let instrument = DataType::from_fields([DataType::utf8().nullable_field("Symbol")])?
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
    /// definition of its name rather than replacing them. The dialects that
    /// contributed a field travel with it and union onto the stored one, so
    /// a merged registry says which dictionaries spoke each field.
    ///
    /// Aliases accumulate rather than replace, because reading a second
    /// source is not a statement that the first one's names were wrong, and
    /// the caller's order is the precedence here as it is everywhere else.
    ///
    /// Answers the count added and the count merged, over the fields.
    ///
    /// ```
    /// use yggdryl::{DataType, FixRegistry};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut symbol = DataType::utf8().nullable_field("Symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let mut held = FixRegistry::from_fields([symbol.clone()])?;
    /// held.insert(DataType::from_fields([symbol.clone()])?.required_field("Instrument"))?;
    ///
    /// // The other dictionary holds the same field under its own tag, with a
    /// // second name, and knows one more member of the component.
    /// let mut ticker = DataType::utf8().nullable_field("symbol");
    /// ticker.as_fix_mut().set_tag(9001)?;
    /// ticker.as_fix_mut().set_names(["Ticker"])?;
    /// let mut other = FixRegistry::from_fields([ticker])?;
    /// let venue = DataType::utf8().nullable_field("VenueSymbol");
    /// other.insert(DataType::from_fields([symbol, venue])?.required_field("Instrument"))?;
    ///
    /// // Symbol and the two standard clock seeds merge.
    /// assert_eq!(held.merge_with(&other)?, (0, 3));
    /// let stored = held.field_by_tag(9001)?;
    /// assert_eq!(stored.name(), "Symbol");
    /// assert_eq!(stored.as_fix().names().collect::<Vec<_>>(), ["Ticker"]);
    /// let instrument = held.field_by_name("Instrument")?;
    /// assert_eq!(instrument.fields()[1].name(), "VenueSymbol");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns what [`Self::add_field`] and [`Self::insert`] return.
    /// One mutation: the fields and the definitions are staged together and
    /// adopted together, so a refusal anywhere leaves this dictionary exactly
    /// as it was.
    pub fn merge_with(&mut self, other: &Self) -> Result<(usize, usize)> {
        let mut staged = self.clone();
        let counts = staged.fold_registry(other)?;
        *self = staged;
        Ok(counts)
    }

    /// Folds another dictionary in, the way [`Self::merge_with`] folds one,
    /// without the staging.
    ///
    /// The fold itself: [`Self::merge_with`] is this plus the copy that makes
    /// it one mutation, and a caller folding several sources into one staged
    /// dictionary calls this so the copy is paid once rather than once per
    /// source. That is the same relationship [`Self::add_fields`] has to
    /// [`Self::fold`], one level up.
    fn fold_registry(&mut self, other: &Self) -> Result<(usize, usize)> {
        other.validate_catalog()?;
        // In the order the other dictionary *answers*, never the order it
        // happens to be stored in. The fold's precedence is its input order,
        // and a caller merging a registry supplies no order of its own - so
        // the one a registry has is the one it publishes: tag-major, the
        // tag's holder first. Storage order is neither that nor stable: a
        // removal swaps the last field into the hole, and a store round trip
        // writes in `iter` order and loads in file order, so two dictionaries
        // that compare equal could merge to two different answers.
        // The scalars alone: the definitions fold through the catalog merge
        // below, under their own rules, and counting them here would count
        // one fold twice.
        let counts = self.fold(other.scalars().cloned())?;
        self.merge_catalog(other)?;
        Ok(counts)
    }

    /// Reads one Ullink `CBlock` into this dictionary, whole.
    ///
    /// The one call an ingest takes, and a parse in front of
    /// [`Self::merge_with`]: the file's vocabulary folds the way any source
    /// folds, and every field, group, component and message it produces
    /// carries the dialect's name in `fix:branches`, which is what the merge
    /// unions onto whatever this dictionary already held.
    ///
    /// `dialect` names the dictionary, and the file names it when the caller
    /// does not: with none supplied the handle's own stem stands in, where it
    /// reads as a name: non-empty and opening with a letter. The FIX version
    /// the file's root declares is not carried: the version a capture is read
    /// at is the row's own `beginstring` where the transport states one, else
    /// what the line implies.
    ///
    /// Answers the count added and the count merged. The message roots are
    /// dropped; take [`Self::from_cfb_file`] when they matter.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::from_cfb_file`] and [`Self::merge_with`] return,
    /// and the membership refusal when the supplied name is empty or carries
    /// a comma.
    pub fn add_cfb_file(
        &mut self,
        handle: &dyn IOBase,
        dialect: Option<&str>,
    ) -> Result<(usize, usize)> {
        let dialect = dialect.or_else(|| super::cfb::stem_dialect(handle));
        let (parsed, _) = Self::from_cfb_file(handle, dialect)?;
        self.merge_with(&parsed)
    }

    /// Reads every Ullink `CBlock` a pattern selects into this dictionary.
    ///
    /// The plural of [`Self::add_cfb_file`], over the crate's one glob walk:
    /// `pattern` is anchored at `root` exactly as [`IOBase::glob`] anchors
    /// it - a fixed prefix is descended rather than listed and filtered, `**`
    /// spans any number of levels - and a pattern that selects nothing folds
    /// nothing. Private entries are never matched: a dot-prefixed file is not
    /// a dictionary.
    ///
    /// **Files fold in ascending URL order**, whatever order the listing
    /// arrived in, because the fold's precedence is its input order and a
    /// glob's sequence is not a caller's to see: it varies with how the
    /// pattern decomposed and with the backend beneath. So where two files
    /// disagree about one tag the last-sorting file wins, and
    /// `cblocks/*.cfb`, `cblocks/**/*.cfb` and `**/venue-*.cfb` over the same
    /// files all answer the same dictionary.
    ///
    /// **`dialect` is resolved per file.** A name supplied here stamps every
    /// matched file with the one membership; `None` lets each file's own stem
    /// stand in, which is what globbing a folder of counterparty files is
    /// for - `cblocks/*.cfb` over `MSFIX44.cfb` and `BLPFIX44.cfb` stamps
    /// `msfix44` and `blpfix44` rather than one name for both.
    ///
    /// Answers the count of files folded, the count of fields added and the
    /// count merged. The file count is a fact only this call holds: an empty
    /// match and a match whose files all folded into stored fields both
    /// answer zeroes for the other two, so without it a mistyped pattern
    /// reads as a silent success.
    ///
    /// One mutation, and one copy of the dictionary for the whole call rather
    /// than one per file: nothing is adopted until every matched file has
    /// parsed and folded, so a file that is not well-formed XML, a tag whose
    /// datatype disagrees with a stored one, or a name the core will not
    /// store leaves this dictionary exactly as it was and the refusal names
    /// the file among however many matched.
    ///
    /// # Errors
    ///
    /// Returns what [`IOBase::glob`] returns for a pattern it cannot
    /// decompose or a fixed prefix it cannot resolve, what a failing listing
    /// entry carries, and what [`Self::add_cfb_file`] returns for each
    /// matched file, located at that file's URL.
    pub fn add_cfb_files(
        &mut self,
        root: &dyn IOBase,
        pattern: &str,
        dialect: Option<&str>,
    ) -> Result<(usize, usize, usize)> {
        // Collected before anything is parsed, so a listing that fails part
        // way is a refusal rather than a half-read dictionary. What is held
        // is one handle per match, never a registry per file.
        let mut files: Vec<crate::holder::Holder> =
            root.glob(pattern, false)?.collect::<Result<Vec<_>>>()?;
        // A pattern may select a directory; a dictionary is a file.
        files.retain(|file| !file.is_container());
        files.sort_by_key(|file| file.url().map(ToString::to_string));
        let mut staged = self.clone();
        let (mut added, mut merged) = (0_usize, 0_usize);
        for file in &files {
            let handle = file.as_io();
            let named = dialect.or_else(|| super::cfb::stem_dialect(handle));
            let (parsed, _) = Self::from_cfb_file(handle, named)
                .map_err(|error| super::store::located(error, handle))?;
            let (file_added, file_merged) = staged
                .fold_registry(&parsed)
                .map_err(|error| super::store::located(error, handle))?;
            added += file_added;
            merged += file_merged;
        }
        *self = staged;
        let count = files.len();
        log::debug!("added {added} and merged {merged} fix fields from {count} cblock files");
        Ok((count, added, merged))
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
        // A list of non-null scalars is one column under one name rather
        // than a definition, so it folds as a field does; the catalog's own
        // shape check is where that reading lives.
        match super::catalog::definition_category(field) {
            Some(category) => Ok(Some(category)),
            None => super::catalog::column_shape(field).map(|()| None),
        }
    }

    /// One scalar through rules 2 to 5 of [`Self::add_field`], without the
    /// staging and without the catalog validation the staged verbs run.
    fn fold_scalar(&mut self, field: Field) -> Result<Option<bool>> {
        // The crate's own fields are every dictionary's, so folding one is
        // folding a field onto itself, and never a source's to redefine.
        if field
            .as_fix()
            .tag()
            .ok()
            .flatten()
            .is_some_and(super::is_crate_tag)
        {
            return Ok(None);
        }
        let (tag, _) = canonical_identity(&field)?;
        if self.position_of_identity(&field)?.is_some() {
            self.update_resolved(field)?;
            return Ok(Some(false));
        }
        // A held tag under another name is a field of its own; a held name
        // under another tag is the same field spelled with another number.
        if self.tags.contains_key(&tag) {
            self.insert_resolved(field)?;
            return Ok(Some(true));
        }
        if let Some(position) = self.position_by_name(field.name()) {
            self.merge_named(position, field)?;
            return Ok(Some(false));
        }
        self.insert_resolved(field)?;
        Ok(Some(true))
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
        self.derivations.take();
        let removed = self.fields.swap_remove(position);
        let departed = self.identities.swap_remove(position);
        if position != last {
            self.index(position);
        }
        // The tag the departed field answered passes to the next field
        // holding it canonically, if any does - the earliest arrival among
        // them, which then moves to the front of its tag in the order.
        if let Some((tag, _)) = departed {
            if !self.tags.contains_key(&tag) {
                if let Some(next) = self
                    .identities
                    .iter()
                    .position(|held| held.is_some_and(|(held, _)| held == tag))
                {
                    self.tags.insert(tag, next);
                    self.reorder();
                }
            }
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
        let start = self.fields.as_ptr() as usize;
        let at =
            (std::ptr::from_ref(field) as usize).wrapping_sub(start) / std::mem::size_of::<Field>();
        match self.fields.get(at) {
            Some(held) if std::ptr::eq(held, field) => self.identities.get(at).copied().flatten(),
            _ => canonical_identity(field).ok(),
        }
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
            if let Some(group) = self
                .get_group_by_tag(*alternate)
                .filter(|group| matches!(group.dtype(), crate::DataType::Map(_)))
            {
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
        // Every change to the fields lands here, so what was compiled off
        // them is forgotten here too.
        self.derivations.take();
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
        self.len() == other.len() && self.iter().eq(other.iter()) && self.catalog == other.catalog
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

    /// The `fix:derivation` of every field, compiled once and shared.
    ///
    /// Built from what the registry holds on the first ask and kept until a
    /// field or a definition changes, so a stream of a million messages
    /// compiles its dictionary's derivations once and a registry edit is
    /// what the next reader evaluates. Cached here rather than on the codec
    /// because a row fill - [`FixMsg::into_row`](super::FixMsg::into_row),
    /// which has no codec in hand - evaluates the crate columns' derivations
    /// through the same compiled list, and a mutation pays a pointer reset
    /// and nothing else. A refusal is kept exactly as a compiled list is: a
    /// derivation naming a field the dictionary lacks refuses every ask,
    /// on every door, until a field changes, and is compiled once rather
    /// than once per ask.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] naming the field whose derivation
    /// reads a column no field or group of this registry answers to, does
    /// not bind against the fields it reads, or whose text a load did not
    /// validate.
    pub(super) fn derivations(&self) -> Result<Arc<super::enrich::Derivations>> {
        match self
            .derivations
            .get_or_init(|| super::enrich::Derivations::compile(self).map(Arc::new))
        {
            Ok(held) => Ok(Arc::clone(held)),
            Err(refused) => Err(refused.error()),
        }
    }

    /// Forgets the compiled derivations: what the next reader evaluates is
    /// compiled off the fields and the catalog as they stand then.
    ///
    /// Every field change reaches [`Self::index`] and forgets them there; a
    /// catalog change that lands without staging a clone calls this.
    pub(super) fn forget_derivations(&mut self) {
        self.derivations.take();
    }
}

impl Clone for FixRegistry {
    /// A clone holds the same fields and catalog and none of what was
    /// compiled off them: every mutation stages itself on a clone before
    /// replacing the registry, so a compiled derivation never outlives the
    /// dictionary it was compiled from, and a clone taken to read compiles
    /// its own once.
    fn clone(&self) -> Self {
        Self {
            fields: self.fields.clone(),
            catalog: self.catalog.clone(),
            ids: self.ids.clone(),
            tags: self.tags.clone(),
            alternate_tags: self.alternate_tags.clone(),
            names: self.names.clone(),
            aliases: self.aliases.clone(),
            positions_by_id: self.positions_by_id.clone(),
            identities: self.identities.clone(),
            derivations: OnceLock::new(),
        }
    }
}

impl Hash for FixRegistry {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.len().hash(state);
        for field in self {
            field.hash(state);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataType;

    fn tagged(name: &str, tag: i32) -> Field {
        let mut field = DataType::utf8().nullable_field(name);
        field.as_fix_mut().set_tag(tag).unwrap();
        field
    }

    #[test]
    fn a_forced_name_digest_collision_is_a_miss_then_a_conflict() {
        let held = tagged("Held", 1);
        let incoming = tagged("Incoming", 2);
        let mut registry = FixRegistry::from_fields([held.clone()]).unwrap();
        // The crate's own fields sit in front of it, so its position is
        // found rather than assumed to be the first.
        let at = registry
            .fields
            .iter()
            .position(|field| field.name() == held.name())
            .expect("the held field");
        let collided = name_digest(incoming.name(), NAME_SEED);
        registry.names.insert(collided, at);

        assert!(
            registry.get_field_by_name(incoming.name()).is_none(),
            "a digest hit is rechecked against the canonical name"
        );
        let before = registry.fields.clone();
        let error = registry.insert(incoming).unwrap_err();
        assert!(
            matches!(
                &error,
                Error::Conflict { path, .. }
                    if path.contains("Incoming") && path.contains("Held")
            ),
            "{error}"
        );
        assert_eq!(registry.fields, before);
    }

    #[test]
    fn a_forced_identity_collision_is_a_conflict_and_a_hit_is_rechecked() {
        let held = tagged("Held", 1);
        let incoming = tagged("Incoming", 2);
        let mut registry = FixRegistry::from_fields([held.clone()]).unwrap();
        let at = registry
            .fields
            .iter()
            .position(|field| field.name() == held.name())
            .expect("the held field");
        // The incoming identity is forced onto the held field's position, as
        // a 32-bit digest collision would land it.
        let collided = canonical_id(&incoming).unwrap();
        registry.ids.insert(collided, at);

        let hit = registry
            .get_field_by_id(collided)
            .expect("the position answers");
        assert_eq!(
            hit.name(),
            "Held",
            "an identity hit answers the field indexed under it"
        );
        let before = registry.fields.clone();
        let error = registry.insert(incoming).unwrap_err();
        assert!(
            matches!(
                &error,
                Error::Conflict { path, .. }
                    if path.contains("Incoming") && path.contains("Held")
            ),
            "{error}"
        );
        assert_eq!(registry.fields, before);
    }
}
