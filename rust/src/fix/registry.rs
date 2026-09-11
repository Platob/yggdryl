//! The FIX registry: one field vector, four compact indexes, and one branch
//! table.
//!
//! Canonical and alternate identifiers are keyed directly by [`FixId`]
//! values, both halves reaching the hasher. Canonical names and aliases are
//! keyed by independent seeded XXH64 digests; every hit is rechecked against the field, so a digest
//! collision is a miss on read and a typed conflict on mutation. Ordered
//! iteration is kept separately as sorted field positions. The registry is
//! built rarely and resolved constantly, so that `O(n)` insertion trade is
//! deliberate.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::iter::FusedIterator;

use smol_str::format_smolstr;

use super::{FixBranch, FixId, FixKey, FixPedigree};
use crate::types::folds_equal;
use crate::xxhash::Xxh64;
use crate::{Error, Field, FieldPath, FieldSegment, IOBase, Result, Url, Version};

const NAME_SEED: u64 = 0x4e41_4d45_5f46_4958;
const ALIAS_SEED: u64 = 0x414c_4941_535f_4649;

// Callers first establish the same canonical branch and field/definition identity.
pub(super) fn metadata_only_change(stored: &Field, incoming: &Field) -> bool {
    stored.name() == incoming.name()
        && stored.dtype() == incoming.dtype()
        && stored.is_nullable() == incoming.is_nullable()
        && stored.as_metadata() != incoming.as_metadata()
}

/// Finalize integer keys before hashbrown selects a control byte.
#[derive(Clone, Copy, Debug, Default)]
struct Mix(u64);

impl Mix {
    const fn finalise(mut value: u64) -> u64 {
        value ^= value >> 33;
        value = value.wrapping_mul(0xff51_afd7_ed55_8ccd);
        value ^= value >> 33;
        value
    }

    /// Folds one integer write into the state instead of replacing it.
    ///
    /// A key written in parts keeps every part: a [`FixId`] hashes its tag
    /// and then its branch, and the rotation lands them in the two halves of
    /// the state, so two tags in one dictionary are two keys rather than one
    /// bucket. A key written once is unchanged by the fold, the state being
    /// zero until then. Two 32-bit halves is exactly what it separates: a
    /// third such write folds back over the first.
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
    let child = folded_child(field, segment_name(head)?)?;
    descend(child, rest)
}

/// The name a segment states, where it states one.
///
/// A schema is addressed by name: a position says which occurrence of a group
/// a caller means, and every occurrence holds the same field, so a position
/// names no child here and is spent by the list it stands on.
fn segment_name(segment: &FieldSegment) -> Option<&str> {
    match segment {
        FieldSegment::Field(name) => Some(name.as_str()),
        FieldSegment::Key(key) => key.value().as_str(),
        FieldSegment::Index(_) => None,
    }
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
type BranchTable = HashMap<u32, FixBranch, BuildHasherDefault<Mix>>;

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
pub(super) fn name_digest(branch: &FixBranch, name: &str, domain: u64) -> u64 {
    let mut state = Xxh64::with_seed(domain ^ u64::from(branch.digest()));
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

/// The key one name is indexed under in one branch.
///
/// A dictionary holds one field per name per branch, and this is what decides
/// which two spellings are one name: the crate's fold, seeded by the branch.
/// [`super::cfb`] settles a CBlock's contended spellings against exactly this
/// rather than against the spelling, so a file the parser leaves named is a
/// file this dictionary accepts.
pub(super) fn name_key(branch: &FixBranch, name: &str) -> u64 {
    name_digest(branch, name, NAME_SEED)
}

enum Held<'a> {
    Id(&'a FixBranch, i32),
    AlternateId(&'a FixBranch, i32),
    Name(&'a FixBranch, &'a str),
    Alias(&'a FixBranch, &'a str),
}

impl fmt::Display for Held<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(branch, tag) => {
                write!(formatter, "identifier {tag}:{}", branch.name())
            }
            Self::AlternateId(branch, tag) => {
                write!(formatter, "alternate identifier {tag}:{}", branch.name())
            }
            Self::Name(branch, name) => {
                write!(formatter, "name {name:?} in branch {:?}", branch.name())
            }
            Self::Alias(branch, alias) => {
                write!(formatter, "alias {alias:?} in branch {:?}", branch.name())
            }
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

fn branch_collision(stored: &FixBranch, incoming: &FixBranch) -> Error {
    Error::conflict(
        "FIX branch",
        "FIX branch",
        format_smolstr!(
            "branches {:?} and {:?} have digest #{:08x}",
            stored.name(),
            incoming.name(),
            incoming.digest()
        ),
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

/// The identity a field enters the registry under.
pub(super) fn canonical_id(field: &Field) -> Result<FixId> {
    field
        .as_fix()
        .id()?
        .ok_or_else(|| Error::absent("fix:tag", field.name()))
}

fn alternate_ids(field: &Field, branch: &FixBranch) -> Result<Vec<FixId>> {
    field
        .as_fix()
        .tags()?
        .into_iter()
        .map(|tag| FixId::from_parts(branch, tag))
        .collect()
}

/// FIX field definitions resolved by identity or folded name.
#[derive(Clone)]
pub struct FixRegistry {
    fields: Vec<Field>,
    pub(super) catalog: super::catalog::Catalog,
    ids: Index<FixId>,
    alternate_ids: Index<FixId>,
    names: Index<u64>,
    aliases: Index<u64>,
    positions_by_id: Vec<usize>,
    /// Each field's canonical identity, by position: what the maps answer a
    /// position with, read back without the two metadata reads and the
    /// branch parse the field's own view costs. Kept in step by
    /// [`Self::index`], which runs after every change to `fields`.
    identities: Vec<Option<FixId>>,
    branches: BranchTable,
    branch_order: Vec<u32>,
    newest: Option<FixPedigree>,
    resettle_newest: bool,
}

impl Default for FixRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FixRegistry {
    /// The deterministic hash of fields, named definitions and branch declarations.
    /// Uses one allocation for the shared XXH3 state, independent of catalog size.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    /// A registry holding nothing but this crate's own fields.
    ///
    /// Every registry starts here: the fields this crate defines - the
    /// digest, the clock, the partition, the bridge's session and context -
    /// are what a row is typed by, so a dictionary loaded from a store,
    /// built from fields or left empty holds them alike. Inserting them into
    /// nothing cannot collide, and a build failure of the crate's own fields
    /// is a defect [`fix_crate_fields`](super::fix_crate_fields) reports;
    /// here it leaves the registry without them rather than unable to exist.
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Self {
            fields: Vec::new(),
            catalog: super::catalog::Catalog::default(),
            ids: Index::default(),
            alternate_ids: Index::default(),
            names: Index::default(),
            aliases: Index::default(),
            positions_by_id: Vec::new(),
            identities: Vec::new(),
            branches: BranchTable::default(),
            branch_order: Vec::new(),
            newest: None,
            resettle_newest: false,
        };
        for field in super::fix_crate_fields().unwrap_or_default() {
            let _ = registry.insert(field.clone());
        }
        registry
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

    /// Returns the field a canonical or alternate identifier names.
    pub fn get_field_by_id(&self, id: FixId) -> Option<&Field> {
        self.position_by_id(id)
            .and_then(|position| self.fields.get(position))
    }

    /// Returns the field an identifier names, raising absence.
    pub fn field_by_id(&self, id: FixId) -> Result<&Field> {
        self.get_field_by_id(id)
            .ok_or_else(|| absent(FixKey::Id(id)))
    }

    /// Returns the best field a canonical or alternate tag names.
    ///
    /// The absent standard branch wins when it declares the tag. A tag in the
    /// user-defined range then tries named branches in canonical spelling
    /// order, with canonical identifiers before alternates.
    pub fn get_field_by_tag(&self, tag: i32) -> Option<&Field> {
        self.best_position_by_tag(tag)
            .and_then(|position| self.fields.get(position))
    }

    /// Returns the best field a tag names, raising absence.
    pub fn field_by_tag(&self, tag: i32) -> Result<&Field> {
        self.get_field_by_tag(tag)
            .ok_or_else(|| absent(FixKey::Tag(tag)))
    }

    /// Returns the field a canonical name or alias names.
    ///
    /// `branch` restricts the lookup when supplied. Without it, canonical
    /// names are tried before aliases, the standard branch wins within each
    /// tier, and remaining branches are tried in canonical spelling order.
    pub fn get_field_by_name(&self, name: &str, branch: Option<&FixBranch>) -> Option<&Field> {
        branch
            .map_or_else(
                || self.best_position_by_name(name),
                |branch| self.position_by_name(branch, name),
            )
            .and_then(|position| self.fields.get(position))
    }

    /// Returns the field a name names, raising absence.
    pub fn field_by_name(&self, name: &str, branch: Option<&FixBranch>) -> Result<&Field> {
        self.get_field_by_name(name, branch)
            .ok_or_else(|| absent(FixKey::Name(name)))
    }

    /// One key read as a name, and as the path it spells where it spells one.
    ///
    /// A name costs no parse, which is what nearly every key is. A key
    /// holding more than one segment is a reading a caller wrote down, and
    /// reading it is this door's job rather than a second door's.
    fn named_or_path(&self, key: &str) -> Option<&Field> {
        if let Some(field) = self.get_field_by_name(key, None) {
            return Some(field);
        }
        let path = FieldPath::from_str(key).ok()?;
        (path.segments().len() > 1)
            .then(|| self.get_field_by_path(&path, None))
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
    /// The optional branch applies only to the registry root; nested segments
    /// continue through the schema walk, which recurses through a group's
    /// occurrence without consuming a segment.
    pub fn get_field_by_path(
        &self,
        path: &FieldPath,
        branch: Option<&FixBranch>,
    ) -> Option<&Field> {
        let (head, rest) = path.segments().split_first()?;
        let head = segment_name(head)?;
        if rest.is_empty() {
            if let Some(field) = self.get_field_by_name(head, branch) {
                return Some(field);
            }
        }
        let mut roots = [
            crate::FixCategory::Messages,
            crate::FixCategory::Components,
            crate::FixCategory::Groups,
        ]
        .into_iter()
        .filter_map(|category| self.get_definition(category, head, branch));
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
    pub fn field_by_path(&self, path: &FieldPath, branch: Option<&FixBranch>) -> Result<&Field> {
        self.get_field_by_path(path, branch)
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
                if let Some(field) = self.get_field_by_name(name, None) {
                    return Ok(field);
                }
                // Where the key spells a path, the path door is the reading
                // that was attempted, so its absence is the one to raise.
                match FieldPath::from_str(name) {
                    Ok(path) if path.segments().len() > 1 => self.field_by_path(&path, None),
                    _ => Err(absent(FixKey::Name(name))),
                }
            }
        }
    }

    /// Returns whether a generic key reaches a field.
    pub fn contains<'key>(&self, key: impl Into<FixKey<'key>>) -> bool {
        self.get_field(key).is_some()
    }

    /// Returns the field a key reaches, filtered to one FIX version.
    ///
    /// The registry itself stays version-agnostic: it holds every tag ever
    /// defined, and a version is a filter on the read, which is what "defined
    /// in one version, available in the others" means. There is no
    /// registry-wide default version; a caller who wants one holds a
    /// [`Version`] beside the registry.
    ///
    /// A field with no lineage answers exactly as [`Self::get_field`] does,
    /// so an undated dictionary behaves as it always has. A field the lineage
    /// says did not exist at `at` answers nothing, including one a later
    /// version removed.
    pub fn get_field_at<'key>(&self, at: Version, key: impl Into<FixKey<'key>>) -> Option<&Field> {
        self.get_field(key)
            .filter(|field| field.as_fix().defined_at(at))
    }

    /// Returns the field a key reaches at one FIX version, raising absence.
    ///
    /// # Errors
    ///
    /// Returns the same absence [`Self::field`] raises, naming the version,
    /// when no field reaches the key or the one that does is not defined at
    /// `at`.
    pub fn field_at<'key>(&self, at: Version, key: impl Into<FixKey<'key>>) -> Result<&Field> {
        let key = key.into();
        self.get_field_at(at, key)
            .ok_or_else(|| absent(format_args!("{key} at FIX {at}")))
    }

    /// Returns every FIX version some field in this dictionary is dated at,
    /// ascending.
    ///
    /// Derived rather than stored, so a dictionary cannot claim a version no
    /// field is dated in.
    pub fn versions(&self) -> Vec<Version> {
        let mut versions: BTreeSet<Version> = BTreeSet::new();
        for field in &self.fields {
            let mut walk = field.as_fix().lineage();
            while let Some(entry) = walk.next_ok() {
                versions.insert(entry.since());
            }
        }
        versions.into_iter().collect()
    }

    /// Returns the newest pedigree this dictionary holds.
    ///
    /// This is the whole of what "FIX Latest" means for a given dictionary:
    /// the greatest version-and-extension-pack pair any lineage carries.
    /// "Latest" is a moving label for the newest published version plus the
    /// extension packs since, so it is resolved here rather than stored as a
    /// version, and never as [`Version::MAX`] - a sentinel compares wrongly
    /// against a field genuinely dated at the newest version and goes stale
    /// the moment an extension pack lands.
    /// It is held rather than searched. Every reader that infers a version
    /// asks for it once per row, and folding six thousand lineage documents
    /// to answer took two milliseconds - three orders of magnitude more than
    /// reading the row it was asked about. A maximum is a monotone fold, so
    /// it is maintained where every other index is: one document walk as a
    /// field lands, and a rescan only when the field holding the maximum
    /// leaves, which is the one removal that can lower it.
    #[must_use]
    pub const fn newest(&self) -> Option<FixPedigree> {
        self.newest
    }

    /// The greatest pedigree any lineage in `fields` carries.
    fn scan_newest(fields: &[Field]) -> Option<FixPedigree> {
        let mut newest: Option<FixPedigree> = None;
        for field in fields {
            newest = newest.max(Self::field_newest(field));
        }
        newest
    }

    /// The greatest pedigree one field's lineage carries.
    fn field_newest(field: &Field) -> Option<FixPedigree> {
        let mut newest: Option<FixPedigree> = None;
        let mut walk = field.as_fix().lineage();
        while let Some(entry) = walk.next_ok() {
            newest = newest.max(Some(entry.pedigree()));
        }
        newest
    }

    /// Returns the branch for `id`.
    pub fn branch_of(&self, id: FixId) -> Option<&FixBranch> {
        self.branches.get(&id.branch_digest())
    }

    /// Returns the branch one digest names, or `None`.
    ///
    /// The reverse of the derivation a branch's digest is: the store's own
    /// branch manifest publishes that digest beside the whole declaration, and
    /// this is how a reader holding one turns it back into the dialect. The
    /// derivation is a one-way XXH32 over the folded name, so publishing the
    /// resolution is the only way a reader can join at all. The argument is
    /// the signed reading the manifest writes, so a digest above `i32::MAX`
    /// arrives negative and resolves exactly as it was stored.
    pub fn get_branch_by_digest(&self, digest: i32) -> Option<&FixBranch> {
        self.branches.get(&super::unsigned(digest))
    }

    /// Returns the branch one digest names, raising absence.
    ///
    /// # Errors
    ///
    /// Returns absence naming the digest when no branch carries it.
    pub fn branch_by_digest(&self, digest: i32) -> Result<&FixBranch> {
        self.get_branch_by_digest(digest)
            .ok_or_else(|| absent(format_args!("branch #{:08x}", super::unsigned(digest))))
    }

    /// Returns the branch `name` reaches, canonically or by an alias.
    ///
    /// A canonical name is answered by the digest it hashes to, which costs
    /// nothing; an alias is answered by a scan, which a dictionary can afford
    /// because it holds a handful of branches where it holds thousands of
    /// fields. A canonical name never loses to an alias: the exact spelling
    /// is tried first and answered whole, so a dialect cannot be shadowed by
    /// another dialect's second spelling of it.
    pub fn branch_named(&self, name: &str) -> Option<&FixBranch> {
        let branch = FixBranch::from_str(name).ok()?;
        if let Some(held) = self
            .branches
            .get(&branch.digest())
            .filter(|held| held.has_identity(&branch))
        {
            return Some(held);
        }
        self.branch_values()
            .find(|held| held.has_alias(branch.name()))
    }

    /// Iterates the branches held by this registry.
    pub fn branches(&self) -> impl Iterator<Item = &FixBranch> {
        self.branch_values()
    }

    /// Installs or replaces one complete branch declaration.
    pub fn set_branch(&mut self, branch: FixBranch) -> Result<()> {
        if branch.is_standard() && branch.version() != Default::default() {
            return Err(Error::InvalidRecord {
                path: "".into(),
                reason: "the standard FIX branch declares no dialect".into(),
            });
        }
        let digest = branch.digest();
        if let Some(stored) = self.branches.get(&digest) {
            if !stored.has_identity(&branch) {
                return Err(branch_collision(stored, &branch));
            }
        }
        let is_new = self.branches.insert(digest, branch).is_none();
        if is_new {
            self.insert_branch_order(digest);
        }
        Ok(())
    }

    /// Adds a field, replacing only an equal canonical identity and folded
    /// name, under the stored spelling.
    pub fn insert(&mut self, field: Field) -> Result<Option<Field>> {
        if self.get_field_by_id(canonical_id(&field)?).is_some() {
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
        let branch = field.as_fix().branch()?;
        self.check_branch(&branch)?;
        let id = canonical_id(&field)?;
        let alternate = alternate_ids(&field, &branch)?;
        let replacing = match (
            self.canonical_position_by_id(id),
            self.canonical_position_by_name(&branch, field.name()),
        ) {
            (Some(by_id), Some(by_name)) if by_id == by_name => Some(by_id),
            _ => None,
        };
        self.check_free(&field, &branch, id, &alternate, replacing)?;
        if let Some(position) = replacing {
            field.set_name(self.fields[position].name());
        }
        self.ensure_branch(branch);
        match replacing {
            Some(position) => self.replace_field(position, field, id).map(Some),
            None => {
                let position = self.fields.len();
                self.fields.push(field);
                self.index(position);
                if id.tag() == 35 {
                    self.refresh_msgtype_aliases();
                }
                Ok(None)
            }
        }
    }

    /// Merges a definition into the field with the same canonical identity.
    /// A name folding to the stored one retains the stored canonical spelling.
    pub fn update(&mut self, field: Field) -> Result<()> {
        let mut staged = self.clone();
        staged.update_resolved(field)?;
        staged.validate_catalog()?;
        *self = staged;
        Ok(())
    }

    fn update_resolved(&mut self, field: Field) -> Result<()> {
        self.validate_definition(crate::FixCategory::Fields, &field)?;
        let branch = field.as_fix().branch()?;
        self.check_branch(&branch)?;
        let id = canonical_id(&field)?;
        let Some(position) = self.canonical_position_by_id(id) else {
            return Err(absent(FixKey::Id(id)));
        };
        let stored = &self.fields[position];
        if !folds_equal(stored.name(), field.name()) {
            return Err(Error::InvalidRecord {
                path: field.name().into(),
                reason: crate::text::expected_got(
                    format_args!("the name {:?} stored for {id}", stored.name()),
                    format_args!("{:?}", field.name()),
                ),
            });
        }
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
        let alternate = alternate_ids(&merged, &branch)?;
        self.check_free(&merged, &branch, id, &alternate, Some(position))?;
        self.replace_field(position, merged, id)?;
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
    /// field. The tag is left out when the branch already answers it - a tag
    /// is one field's, and this field is not the one that claimed it - which
    /// is noted through `log` at debug level rather than refused, since the
    /// name was the match.
    fn merge_named(&mut self, position: usize, field: Field) -> Result<()> {
        self.validate_definition(crate::FixCategory::Fields, &field)?;
        let branch = field.as_fix().branch()?;
        self.check_branch(&branch)?;
        let incoming = canonical_id(&field)?;
        let stored = &self.fields[position];
        let id = canonical_id(stored)?;
        if stored.dtype() != field.dtype() {
            return Err(datatype_disagreement(stored, id, &field));
        }
        let mut merged = field.clone();
        merged.set_name(stored.name());
        merged.set_nullable(stored.is_nullable());
        merged.set_metadata(field.as_metadata().merge_with(stored.as_metadata())?.iter())?;
        // The identity is the stored field's, and it is written before the
        // `fix:` fold, which holds both sides to one tag.
        merged.as_fix_mut().set_tag(id.tag())?;
        merged.as_fix_mut().merge_with(&stored.as_fix())?;
        // Stored order first, then what only the incoming field states. The
        // canonical tag is nobody's alternate, its own field's included.
        let mut tags = stored.as_fix().tags()?;
        let claimed = |tags: &[i32], tag: i32| tag == id.tag() || tags.contains(&tag);
        for tag in field.as_fix().tags()? {
            if !claimed(&tags, tag) {
                tags.push(tag);
            }
        }
        match self.position_by_id(incoming) {
            Some(holder) if holder != position => log::debug!(
                "tag {} of {:?} stays with {:?}",
                incoming.tag(),
                field.name(),
                self.fields[holder].name()
            ),
            _ if claimed(&tags, incoming.tag()) => {}
            _ => tags.push(incoming.tag()),
        }
        // Spellings dedupe under the same fold the index resolves them by,
        // so no alias is kept that the stored name or an earlier alias
        // already answers for.
        let mut aliases: Vec<&str> = stored.as_fix().aliases().collect();
        for alias in field
            .as_fix()
            .aliases()
            .chain(std::iter::once(field.name()))
        {
            if !folds_equal(alias, stored.name())
                && !aliases.iter().any(|held| folds_equal(held, alias))
            {
                aliases.push(alias);
            }
        }
        merged.as_fix_mut().set_tags(&tags)?;
        merged.as_fix_mut().set_aliases(&aliases)?;
        let alternate = alternate_ids(&merged, &branch)?;
        self.check_free(&merged, &branch, id, &alternate, Some(position))?;
        self.replace_field(position, merged, id)?;
        Ok(())
    }

    /// Puts `merged` where the field at `position` was, answering what stood
    /// there.
    ///
    /// `merged` carries the identity `id` the position already answers to,
    /// and the caller has proven every key it holds free. A change to the
    /// metadata alone is what the catalog's references restate, so that one
    /// re-resolves them; a change to the datatype is one they refuse, and the
    /// caller's validation is what refuses it.
    fn replace_field(&mut self, position: usize, merged: Field, id: FixId) -> Result<Field> {
        let refresh = metadata_only_change(&self.fields[position], &merged);
        if refresh {
            self.validate_catalog()?;
        }
        self.departing(position);
        self.unindex(position, position);
        let prior = std::mem::replace(&mut self.fields[position], merged);
        self.index(position);
        self.settle();
        if refresh {
            self.refresh_references()?;
        }
        if id.tag() == 35 {
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
    ///    [`Self::add_definition`] under the category its shape names: a
    ///    `List` or `LargeList` of non-null Struct occurrences is a group, a
    ///    Struct declaring `fix:msgtype` is a message, any other Struct is a
    ///    component. Any other nested datatype is refused as a scalar field
    ///    would refuse it.
    /// 2. One of this crate's own tags is every dictionary's already, so it is
    ///    neither added nor merged: skipped, answering `false`.
    /// 3. A canonical identity the dictionary holds - the branch and the tag
    ///    together, because the same tag in two branches is two fields -
    ///    merges exactly as [`Self::update`] merges: the name must fold to
    ///    the stored one, the datatype must equal it, and the incoming
    ///    metadata wins a shared key.
    /// 4. Otherwise a name that folds to a stored field's canonical name or
    ///    to one of its aliases, in the same branch, merges *into* that
    ///    field. Folding is the crate's one fold, the one every name lookup
    ///    resolves by: ASCII case, and the `_`, `-` and space separators, so
    ///    `party_id` is a spelling of `PartyID`. The stored field keeps its
    ///    identity, its name and its nullability; aliases and alternate tags
    ///    are the union, the stored ones first, deduplicated under the same
    ///    fold; the incoming name joins the aliases unless it is the
    ///    canonical one; the incoming canonical tag joins the alternate tags
    ///    unless a field in the branch already answers it, in which case it
    ///    is left out and noted through `log` at debug level; generic
    ///    metadata and the `fix:` keys fold with the precedence of rule 3. A
    ///    datatype that disagrees is refused as rule 3 refuses it.
    /// 5. Otherwise it is inserted.
    ///
    /// Staged like [`Self::insert`]: a refusal leaves the dictionary exactly
    /// as it was.
    ///
    /// ```
    /// use yggdryl::{DataType, FixId, FixRegistry};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut symbol = DataType::Utf8.nullable_field("Symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let mut registry = FixRegistry::from_fields([symbol])?;
    ///
    /// // A venue's file calls the same field `symbol`, under its own tag and
    /// // with a second name: that is a second spelling of tag 55's field.
    /// let mut incoming = DataType::Utf8.nullable_field("symbol");
    /// incoming.as_fix_mut().set_tag(9001)?;
    /// incoming.as_fix_mut().set_aliases(["Ticker"])?;
    /// assert!(!registry.add_field(incoming)?, "folded into the stored field");
    ///
    /// let stored = registry.field_by_tag(9001)?;
    /// assert_eq!(stored.name(), "Symbol");
    /// assert_eq!(stored.as_fix().id()?, Some(FixId::standard(55)));
    /// assert_eq!(stored.as_fix().tags()?, [9001]);
    /// assert!(stored.as_fix().aliases().any(|alias| alias == "Ticker"));
    /// assert_eq!(registry.field_by_name("ticker", None)?.name(), "Symbol");
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
    /// Returns what [`Self::insert`], [`Self::update`] and
    /// [`Self::add_definition`] return: absence for a scalar carrying no
    /// `fix:tag`, a conflict for an alias or alternate tag another field
    /// holds in the same branch, and [`Error::InvalidRecord`] for a datatype
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
    /// field is a named definition and folds through
    /// [`Self::add_definition`], and one of this crate's own tags is skipped.
    /// Answers the count added and the count merged, in that order, and
    /// records the same pair through `log` at debug level; a skipped field
    /// counts as neither.
    ///
    /// The caller's order is the precedence, exactly as [`Self::update`]'s
    /// is: merge the lowest-priority source first and the highest arrives
    /// last and wins.
    ///
    /// ```
    /// use yggdryl::{DataType, FixCategory, FixRegistry};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut symbol = DataType::Utf8.nullable_field("Symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let mut registry = FixRegistry::from_fields([symbol.clone()])?;
    ///
    /// // Tag 55 is stored and merges; tag 44 is new; the Struct is a component.
    /// symbol.as_fix_mut().set_description("Ticker symbol")?;
    /// let mut price = DataType::Float64.nullable_field("Price");
    /// price.as_fix_mut().set_tag(44)?;
    /// let instrument = DataType::from_fields([DataType::Utf8.nullable_field("Symbol")])?
    ///     .required_field("Instrument");
    /// assert_eq!(registry.add_fields([symbol, price, instrument])?, (2, 1));
    /// assert_eq!(registry.field_by_tag(55)?.description(), Some("Ticker symbol"));
    /// assert_eq!(registry.definitions(FixCategory::Components).count(), 1);
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
    /// [`Self::add_definition`] folds one, its members appended to the stored
    /// definition of its name rather than replacing them, and every dialect
    /// folds beside them: one this dictionary does not hold arrives whole,
    /// and one it holds takes the incoming record while keeping every
    /// spelling it already answered to.
    ///
    /// Aliases accumulate rather than replace, because reading a second
    /// source is not a statement that the first one's names were wrong. The
    /// rest of a branch record is the incoming declaration's, whole, so the
    /// caller's order is the precedence here as it is everywhere else.
    ///
    /// Answers the count added and the count merged, over the fields.
    ///
    /// ```
    /// use yggdryl::{DataType, FixCategory, FixRegistry};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut symbol = DataType::Utf8.nullable_field("Symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let mut held = FixRegistry::from_fields([symbol.clone()])?;
    /// held.create_definition(
    ///     FixCategory::Components,
    ///     DataType::from_fields([symbol.clone()])?.required_field("Instrument"),
    /// )?;
    ///
    /// // The other dictionary holds the same field under its own tag, with a
    /// // second name, and knows one more member of the component.
    /// let mut ticker = DataType::Utf8.nullable_field("symbol");
    /// ticker.as_fix_mut().set_tag(9001)?;
    /// ticker.as_fix_mut().set_aliases(["Ticker"])?;
    /// let mut other = FixRegistry::from_fields([ticker])?;
    /// let venue = DataType::Utf8.nullable_field("VenueSymbol");
    /// other.create_definition(
    ///     FixCategory::Components,
    ///     DataType::from_fields([symbol, venue])?.required_field("Instrument"),
    /// )?;
    ///
    /// assert_eq!(held.merge_with(&other)?, (0, 1));
    /// let stored = held.field_by_tag(9001)?;
    /// assert_eq!(stored.name(), "Symbol");
    /// assert_eq!(stored.as_fix().aliases().collect::<Vec<_>>(), ["Ticker"]);
    /// let instrument = held.definition(FixCategory::Components, "Instrument", None)?;
    /// assert_eq!(instrument.fields()[1].name(), "VenueSymbol");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns what [`Self::set_branch`], [`Self::add_field`] and
    /// [`Self::add_definition`] return. One mutation: the branches, the
    /// fields and the definitions are staged together and adopted together,
    /// so a refusal anywhere leaves this dictionary exactly as it was.
    pub fn merge_with(&mut self, other: &Self) -> Result<(usize, usize)> {
        other.validate_catalog()?;
        let mut staged = self.clone();
        staged.absorb_branches(other.branch_values(), None)?;
        let counts = staged.fold(other.fields.iter().cloned())?;
        staged.merge_catalog(other)?;
        *self = staged;
        Ok(counts)
    }

    /// Reads one Ullink `CBlock` into this dictionary, whole.
    ///
    /// The one call an ingest takes, and a parse in front of
    /// [`Self::merge_with`]: the file's vocabulary folds the way any source
    /// folds, and the dialect the root element declares - its FIX version and
    /// its session `CompID` pair - is recorded beside it, which reading the
    /// fields alone would lose.
    ///
    /// `branch` names the dialect, and the file names it when the caller does
    /// not: with none supplied the handle's own stem stands in. **The stem
    /// also becomes an alias whenever it is not already the name**, so a
    /// dictionary read from `MSFIX44.cfb` under the branch `morgan` still
    /// answers to `msfix44` - the file a definition arrived as is a spelling
    /// people use for it. `aliases` names any others.
    ///
    /// Answers the count added and the count merged. The message roots are
    /// dropped; take [`Self::from_cfb_file`] when they matter.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::from_cfb_file`] and [`Self::merge_with`] return,
    /// and [`Error::Parse`] when the supplied name, the stem standing in for
    /// it, or an alias is not a branch.
    pub fn add_cfb_file(
        &mut self,
        handle: &dyn IOBase,
        branch: Option<&str>,
        aliases: Option<&[&str]>,
    ) -> Result<(usize, usize)> {
        let stem = handle.url().and_then(Url::stem);
        let dialect = branch
            .or(stem)
            .map(FixBranch::from_str)
            .transpose()?
            .filter(|held| !held.is_standard());
        let (parsed, _) = Self::from_cfb_file(handle, dialect.as_ref())?;

        let mut staged = self.clone();
        // The names this file answers to are the caller's, then the file's
        // own; what the dictionary already answered to is added by the fold.
        let named: Vec<&str> = aliases
            .unwrap_or_default()
            .iter()
            .copied()
            .chain(stem)
            .collect();
        staged.absorb_branches(parsed.branch_values(), Some(&named))?;
        let counts = staged.merge_with(&parsed)?;
        *self = staged;
        Ok(counts)
    }

    /// Declares every incoming branch, keeping the spellings already answered.
    fn absorb_branches<'branch>(
        &mut self,
        incoming: impl IntoIterator<Item = &'branch FixBranch>,
        extra: Option<&[&str]>,
    ) -> Result<()> {
        for branch in incoming {
            if branch.is_standard() {
                continue;
            }
            let spellings = self.spellings_for(branch, extra.unwrap_or_default())?;
            self.set_branch(branch.clone().with_aliases(spellings)?)?;
        }
        Ok(())
    }

    /// Every spelling `incoming` should answer to once it is folded in.
    ///
    /// What this dictionary already answered to, then what the incoming
    /// declaration names, then anything else the caller added - each kept only
    /// where it says something the canonical name and the spellings before it
    /// do not.
    fn spellings_for(&self, incoming: &FixBranch, extra: &[&str]) -> Result<Vec<String>> {
        let stored = self.branches.get(&incoming.digest());
        let mut held: Vec<String> = Vec::new();
        for candidate in stored
            .into_iter()
            .flat_map(FixBranch::aliases)
            .chain(incoming.aliases())
            .map(smol_str::SmolStr::as_str)
            .chain(extra.iter().copied())
        {
            // Folded through the branch grammar rather than compared raw, so
            // one rule decides here and inside `with_aliases` alike.
            let folded = FixBranch::from_str(candidate)?;
            if folded.digest() == incoming.digest() || held.iter().any(|seen| seen == folded.name())
            {
                continue;
            }
            held.push(folded.name().to_owned());
        }
        Ok(held)
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
        super::catalog::definition_category(field)
            .map(Some)
            .ok_or_else(|| super::catalog::not_scalar(field))
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
        let id = canonical_id(&field)?;
        if self.canonical_position_by_id(id).is_some() {
            self.update_resolved(field)?;
            return Ok(Some(false));
        }
        let branch = field.as_fix().branch()?;
        if let Some(position) = self.position_by_name(&branch, field.name()) {
            self.merge_named(position, field)?;
            return Ok(Some(false));
        }
        self.insert_resolved(field)?;
        Ok(Some(true))
    }

    /// Removes an unreferenced field a tag, identifier, name, or alias reaches.
    ///
    /// Returns no removed value for an absent or referenced field. Use
    /// [`Self::remove_definition`] for a typed reference refusal.
    pub fn remove<'key>(&mut self, key: impl Into<FixKey<'key>>) -> Option<Field> {
        let mut staged = self.clone();
        let removed = staged.remove_resolved(key)?;
        staged.validate_catalog().ok()?;
        staged.refresh_msgtype_aliases();
        *self = staged;
        Some(removed)
    }

    pub(super) fn remove_resolved<'key>(&mut self, key: impl Into<FixKey<'key>>) -> Option<Field> {
        let position = match key.into() {
            FixKey::Tag(tag) => self.position_by_id(FixId::standard(tag)),
            FixKey::Id(id) => self.position_by_id(id),
            FixKey::Name(name) => self.position_by_name(&FixBranch::STANDARD, name),
        }?;
        let last = self.fields.len().checked_sub(1)?;
        if position != last {
            self.unindex(last, last);
        }
        self.departing(position);
        self.unindex(position, position);
        let removed = self.fields.swap_remove(position);
        self.identities.swap_remove(position);
        if position != last {
            self.index(position);
        }
        self.settle();
        self.retain_populated_branches();
        Some(removed)
    }

    pub(super) fn retain_populated_branches(&mut self) {
        let empty: Vec<u32> = self
            .branch_values()
            .filter(|branch| !self.has_branch_definitions(branch))
            .map(FixBranch::digest)
            .collect();
        for digest in empty {
            self.branches.remove(&digest);
            self.branch_order.retain(|held| *held != digest);
        }
    }

    /// Returns the first field after `after`, in tag-major identifier order.
    pub fn next_field_after(&self, after: Option<FixId>) -> Option<&Field> {
        let position = match after {
            None => 0,
            Some(after) => self.positions_by_id.partition_point(|position| {
                self.fields
                    .get(*position)
                    .and_then(|field| field.as_fix().id().ok().flatten())
                    .is_some_and(|id| id <= after)
            }),
        };
        self.positions_by_id
            .get(position)
            .and_then(|field| self.fields.get(*field))
    }

    /// Iterates fields in tag-major canonical-identifier order.
    pub fn iter(&self) -> FixFieldIter<'_> {
        FixFieldIter {
            positions: self.positions_by_id.iter(),
            fields: &self.fields,
        }
    }

    /// Returns the number of registered fields.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Returns whether no field is registered.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.catalog.all().next().is_none()
    }

    pub(super) fn branch_values(&self) -> impl Iterator<Item = &FixBranch> {
        self.branch_order
            .iter()
            .filter_map(|digest| self.branches.get(digest))
    }

    pub(super) fn check_branch(&self, branch: &FixBranch) -> Result<()> {
        if let Some(stored) = self.branches.get(&branch.digest()) {
            if !stored.has_identity(branch) {
                return Err(branch_collision(stored, branch));
            }
        }
        Ok(())
    }

    pub(super) fn ensure_branch(&mut self, branch: FixBranch) {
        let digest = branch.digest();
        if self.branches.contains_key(&digest) {
            return;
        }
        self.branches.insert(digest, branch);
        self.insert_branch_order(digest);
    }

    fn insert_branch_order(&mut self, digest: u32) {
        let Some(branch) = self.branches.get(&digest).cloned() else {
            return;
        };
        let position = self.branch_order.partition_point(|held| {
            self.branches
                .get(held)
                .is_some_and(|held| held.name() < branch.name())
        });
        self.branch_order.insert(position, digest);
    }

    pub(super) fn canonical_position_by_id(&self, id: FixId) -> Option<usize> {
        self.ids.get(&id).copied()
    }

    pub(super) fn canonical_position_by_name(
        &self,
        branch: &FixBranch,
        name: &str,
    ) -> Option<usize> {
        if !self.branch_matches(branch) {
            return None;
        }
        let position = *self.names.get(&name_digest(branch, name, NAME_SEED))?;
        self.canonical_name_matches(position, branch, name)
            .then_some(position)
    }

    fn position_by_id(&self, id: FixId) -> Option<usize> {
        self.ids
            .get(&id)
            .or_else(|| self.alternate_ids.get(&id))
            .copied()
    }

    fn best_position_by_tag(&self, tag: i32) -> Option<usize> {
        let standard = FixId::standard(tag);
        self.ids
            .get(&standard)
            .copied()
            .or_else(|| {
                self.branch_values()
                    .filter(|branch| !branch.is_standard() && FixId::is_admissible(branch, tag))
                    .find_map(|branch| {
                        FixId::from_parts(branch, tag)
                            .ok()
                            .and_then(|id| self.ids.get(&id).copied())
                    })
            })
            .or_else(|| self.alternate_ids.get(&standard).copied())
            .or_else(|| {
                self.branch_values()
                    .filter(|branch| !branch.is_standard() && FixId::is_admissible(branch, tag))
                    .find_map(|branch| {
                        FixId::from_parts(branch, tag)
                            .ok()
                            .and_then(|id| self.alternate_ids.get(&id).copied())
                    })
            })
    }

    fn position_by_name(&self, branch: &FixBranch, name: &str) -> Option<usize> {
        if !self.branch_matches(branch) {
            return None;
        }
        if let Some(position) = self.names.get(&name_digest(branch, name, NAME_SEED)) {
            if self.canonical_name_matches(*position, branch, name) {
                return Some(*position);
            }
        }
        self.alias_position_by_name(branch, name)
    }

    fn alias_position_by_name(&self, branch: &FixBranch, name: &str) -> Option<usize> {
        if !self.branch_matches(branch) {
            return None;
        }
        let position = *self.aliases.get(&name_digest(branch, name, ALIAS_SEED))?;
        self.alias_matches(position, branch, name)
            .then_some(position)
    }

    fn best_position_by_name(&self, name: &str) -> Option<usize> {
        self.canonical_position_by_name(&FixBranch::STANDARD, name)
            .or_else(|| {
                self.branch_values()
                    .filter(|branch| !branch.is_standard())
                    .find_map(|branch| self.canonical_position_by_name(branch, name))
            })
            .or_else(|| self.alias_position_by_name(&FixBranch::STANDARD, name))
            .or_else(|| {
                self.branch_values()
                    .filter(|branch| !branch.is_standard())
                    .find_map(|branch| self.alias_position_by_name(branch, name))
            })
    }

    fn branch_matches(&self, branch: &FixBranch) -> bool {
        self.branches
            .get(&branch.digest())
            .is_some_and(|held| held.has_identity(branch))
    }

    /// Whether the field at `position` is the one `name` names in `branch`.
    ///
    /// The recheck behind a digest hit, so it tests exactly the equivalence
    /// [`name_digest`] hashes - the crate's one fold - and nothing narrower:
    /// a spelling the index keys as a stored name and the recheck then
    /// refuses would be neither found nor insertable.
    fn canonical_name_matches(&self, position: usize, branch: &FixBranch, name: &str) -> bool {
        self.fields
            .get(position)
            .is_some_and(|field| folds_equal(field.name(), name))
            && self.identity_at(position, branch)
    }

    fn alias_matches(&self, position: usize, branch: &FixBranch, name: &str) -> bool {
        self.fields.get(position).is_some_and(|field| {
            field
                .as_fix()
                .aliases()
                .any(|alias| folds_equal(alias, name))
        }) && self.identity_at(position, branch)
    }

    /// Whether the field at `position` belongs to `branch`, by the identity
    /// the index remembers for it.
    fn identity_at(&self, position: usize, branch: &FixBranch) -> bool {
        self.identities
            .get(position)
            .copied()
            .flatten()
            .is_some_and(|id| id.branch_digest() == branch.digest())
    }

    /// The canonical identity of one of this registry's own fields, read off
    /// the index rather than out of the field's metadata.
    ///
    /// A borrowed field the registry answered points into its own storage,
    /// and that position remembers the identity; a field it does not hold - a
    /// definition's member, a message's child - answers what its own
    /// metadata says, exactly as [`FixField::id`](crate::FixField::id) does.
    pub(super) fn identity_of(&self, field: &Field) -> Option<FixId> {
        let start = self.fields.as_ptr() as usize;
        let at =
            (std::ptr::from_ref(field) as usize).wrapping_sub(start) / std::mem::size_of::<Field>();
        match self.fields.get(at) {
            Some(held) if std::ptr::eq(held, field) => self.identities.get(at).copied().flatten(),
            _ => field.as_fix().id().ok().flatten(),
        }
    }

    fn check_free(
        &self,
        field: &Field,
        branch: &FixBranch,
        id: FixId,
        alternate: &[FixId],
        owner: Option<usize>,
    ) -> Result<()> {
        let other = |position: &usize| Some(*position) != owner;
        if let Some(holder) = self.ids.get(&id).filter(|position| other(position)) {
            return Err(conflict(
                Held::Id(branch, id.tag()),
                field,
                &self.fields[*holder],
            ));
        }
        let name = name_digest(branch, field.name(), NAME_SEED);
        if let Some(holder) = self.names.get(&name).filter(|position| other(position)) {
            return Err(conflict(
                Held::Name(branch, field.name()),
                field,
                &self.fields[*holder],
            ));
        }
        for alternate in alternate {
            if let Some(holder) = self
                .alternate_ids
                .get(alternate)
                .filter(|position| other(position))
            {
                return Err(conflict(
                    Held::AlternateId(branch, alternate.tag()),
                    field,
                    &self.fields[*holder],
                ));
            }
        }
        for alias in field.as_fix().aliases() {
            let key = name_digest(branch, alias, ALIAS_SEED);
            if let Some(holder) = self.aliases.get(&key).filter(|position| other(position)) {
                return Err(conflict(
                    Held::Alias(branch, alias),
                    field,
                    &self.fields[*holder],
                ));
            }
        }
        Ok(())
    }

    fn index(&mut self, position: usize) {
        let Some(field) = self.fields.get(position) else {
            return;
        };
        let view = field.as_fix();
        // Remembered first, whatever the field answers: every position has
        // an entry, so a field that indexes nothing still identifies nothing.
        let identity = view.id().ok().flatten();
        if position >= self.identities.len() {
            self.identities.resize(position + 1, None);
        }
        self.identities[position] = identity;
        let Ok(branch) = view.branch() else {
            return;
        };
        let Some(id) = identity else {
            return;
        };
        self.ids.insert(id, position);
        for tag in view.tags().unwrap_or_default() {
            if let Ok(alternate) = FixId::from_parts(&branch, tag) {
                self.alternate_ids.insert(alternate, position);
            }
        }
        self.newest = self.newest.max(Self::field_newest(field));
        self.names
            .insert(name_digest(&branch, field.name(), NAME_SEED), position);
        for alias in view.aliases() {
            self.aliases
                .insert(name_digest(&branch, alias, ALIAS_SEED), position);
        }
        let ordered = self.positions_by_id.partition_point(|held| {
            self.identities
                .get(*held)
                .copied()
                .flatten()
                .is_some_and(|held| held < id)
        });
        self.positions_by_id.insert(ordered, position);
    }

    /// Notes that the field at `position` is leaving or being overwritten.
    ///
    /// Separate from [`Self::unindex`] because that also re-points a field
    /// being *moved*, which is not a departure and must not cost a rescan.
    fn departing(&mut self, position: usize) {
        if self.newest.is_none() {
            return;
        }
        if let Some(field) = self.fields.get(position) {
            // A maximum cannot be un-maxed, so the field carrying the newest
            // pedigree is the one departure that has to be rescanned for.
            if Self::field_newest(field) == self.newest {
                self.resettle_newest = true;
            }
        }
    }

    /// Restores the held maximum after a departure that could have lowered it.
    ///
    /// Called once a mutation is complete, so the rescan sees what the
    /// dictionary now holds rather than what it held mid-edit.
    fn settle(&mut self) {
        if std::mem::take(&mut self.resettle_newest) {
            self.newest = Self::scan_newest(&self.fields);
        }
    }

    fn unindex(&mut self, position: usize, pointing_at: usize) {
        let Some(field) = self.fields.get(position) else {
            return;
        };
        let view = field.as_fix();
        let Ok(branch) = view.branch() else {
            return;
        };
        if let Ok(Some(id)) = view.id() {
            if self.ids.get(&id) == Some(&pointing_at) {
                self.ids.remove(&id);
            }
        }
        for tag in view.tags().unwrap_or_default() {
            let Ok(id) = FixId::from_parts(&branch, tag) else {
                continue;
            };
            if self.alternate_ids.get(&id) == Some(&pointing_at) {
                self.alternate_ids.remove(&id);
            }
        }
        let name = name_digest(&branch, field.name(), NAME_SEED);
        if self.names.get(&name) == Some(&pointing_at) {
            self.names.remove(&name);
        }
        for alias in view.aliases() {
            let key = name_digest(&branch, alias, ALIAS_SEED);
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
            let Ok(Some(id)) = field.as_fix().id() else {
                continue;
            };
            let key = self
                .branch_of(id)
                .map_or_else(|| id.to_string(), |branch| format!("{}:{branch}", id.tag()));
            map.entry(&key, field);
        }
        map.finish()
    }
}

impl PartialEq for FixRegistry {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self.iter().eq(other.iter())
            && self.branches == other.branches
            && self.catalog == other.catalog
    }
}

impl Eq for FixRegistry {}

impl Hash for FixRegistry {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.len().hash(state);
        for field in self {
            field.hash(state);
        }
        self.branch_order.len().hash(state);
        for branch in self.branch_values() {
            branch.hash(state);
        }
        for category in [
            crate::FixCategory::Messages,
            crate::FixCategory::Components,
            crate::FixCategory::Groups,
        ] {
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
}

impl<'registry> Iterator for FixFieldIter<'registry> {
    type Item = &'registry Field;

    fn next(&mut self) -> Option<Self::Item> {
        self.positions
            .next()
            .and_then(|position| self.fields.get(*position))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.positions.size_hint()
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.positions
            .nth(n)
            .and_then(|position| self.fields.get(*position))
    }
}

impl DoubleEndedIterator for FixFieldIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.positions
            .next_back()
            .and_then(|position| self.fields.get(*position))
    }
}

impl ExactSizeIterator for FixFieldIter<'_> {}
impl FusedIterator for FixFieldIter<'_> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DataType;

    fn tagged(name: &str, tag: i32) -> Field {
        let mut field = DataType::Utf8.nullable_field(name);
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
        let collided = name_digest(&FixBranch::STANDARD, incoming.name(), NAME_SEED);
        registry.names.insert(collided, at);

        assert!(
            registry
                .get_field_by_name(incoming.name(), Some(&FixBranch::STANDARD))
                .is_none(),
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
}
