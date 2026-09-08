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
use std::hash::{BuildHasherDefault, Hasher};
use std::iter::FusedIterator;

use smol_str::format_smolstr;

use super::{FixBranch, FixId, FixKey, FixPedigree};
use crate::xxhash::Xxh64;
use crate::{Error, Field, IOBase, Result, Url, Version};

const NAME_SEED: u64 = 0x4e41_4d45_5f46_4958;
const ALIAS_SEED: u64 = 0x414c_4941_535f_4649;

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
/// `NoPartyIDs.PartyID` resolving its first segment and refusing its second is
/// one function disagreeing with itself.
///
/// A list is stepped through before anything else, including the exact walk.
/// A repeating group's occurrence is not a path segment - nobody spelling a
/// path names it - so consulting [`Field::get_field_by_path`] first would let
/// the occurrence match by its own name, which is exactly what this walk must
/// not allow now that the occurrence carries the component's name. The exact
/// walk still runs under the list, because it is the cheap answer and the
/// common one.
fn descend<'field>(field: &'field Field, path: &str) -> Option<&'field Field> {
    if let crate::DataType::List(item) | crate::DataType::LargeList(item) = field.dtype() {
        return descend(item, path);
    }
    if let Some(held) = field.get_field_by_path(path) {
        return Some(held);
    }
    let (head, rest) = match path.split_once('.') {
        None => (path, None),
        Some((head, rest)) => (head, Some(rest)),
    };
    let child = folded_child(field, head)?;
    match rest {
        None => Some(child),
        Some(rest) => descend(child, rest),
    }
}

/// One child by folded name, reaching through a group's occurrence.
///
/// A repeating group is a List of one Struct, so a member is that struct's
/// child and not the list's. The occurrence is transparent: it is recursed
/// through without consuming a segment and it is never matched by its own
/// name, because that name is the component's - `NoPartyIDs.PartyID` names
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
        .find(|held| crate::types::folds_equal(held.name(), name))
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
fn name_digest(branch: &FixBranch, name: &str, domain: u64) -> u64 {
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

/// Whether a field carries a nested subtree rather than one scalar value.
pub(super) fn is_nested(field: &Field) -> bool {
    field.dtype().is_nested()
}

/// FIX field definitions resolved by identity or folded name.
#[derive(Clone)]
pub struct FixRegistry {
    fields: Vec<Field>,
    ids: Index<FixId>,
    alternate_ids: Index<FixId>,
    names: Index<u64>,
    aliases: Index<u64>,
    positions_by_id: Vec<usize>,
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
            ids: Index::default(),
            alternate_ids: Index::default(),
            names: Index::default(),
            aliases: Index::default(),
            positions_by_id: Vec::new(),
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

    /// Returns the field a dotted path reaches.
    ///
    /// The optional branch applies only to the registry root; nested segments
    /// continue through [`Field::get_field_by_path`].
    pub fn get_field_by_path(&self, path: &str, branch: Option<&FixBranch>) -> Option<&Field> {
        if let Some(field) = self.get_field_by_name(path, branch) {
            return Some(field);
        }
        let (head, rest) = path.split_once('.')?;
        descend(self.get_field_by_name(head, branch)?, rest)
    }

    /// Returns the field a dotted path reaches, raising absence.
    pub fn field_by_path(&self, path: &str, branch: Option<&FixBranch>) -> Result<&Field> {
        self.get_field_by_path(path, branch)
            .ok_or_else(|| absent(format_args!("path {path:?}")))
    }

    /// Returns the field a tag, identifier, name, or dotted path reaches.
    pub fn get_field<'key>(&self, key: impl Into<FixKey<'key>>) -> Option<&Field> {
        match key.into() {
            FixKey::Tag(tag) => self.get_field_by_tag(tag),
            FixKey::Id(id) => self.get_field_by_id(id),
            FixKey::Name(name) => self.get_field_by_path(name, None),
        }
    }

    /// Returns the field a generic key reaches, raising absence.
    pub fn field<'key>(&self, key: impl Into<FixKey<'key>>) -> Result<&Field> {
        match key.into() {
            FixKey::Tag(tag) => self.field_by_tag(tag),
            FixKey::Id(id) => self.field_by_id(id),
            FixKey::Name(name) => self.field_by_path(name, None),
        }
    }

    /// Returns whether a generic key reaches a field.
    pub fn contains<'key>(&self, key: impl Into<FixKey<'key>>) -> bool {
        self.get_field(key).is_some()
    }

    /// Returns the field a key reaches, when that field holds one scalar.
    ///
    /// A transcriber resolving a wire tag wants a value, not a subtree, and
    /// this is what says so: the same tiers, filtered to the half a scalar
    /// can be in. A counter tag reaches its group through
    /// [`Self::get_nested_field`] instead, so neither half can answer for the
    /// other and [`Self::get_field`] answers exactly what it always did.
    pub fn get_primitive_field<'key>(&self, key: impl Into<FixKey<'key>>) -> Option<&Field> {
        self.get_field(key).filter(|field| !is_nested(field))
    }

    /// Returns the scalar field a key reaches, raising absence.
    ///
    /// # Errors
    ///
    /// Returns the absence [`Self::field`] raises when no field reaches the
    /// key, and when the one that does carries a subtree.
    pub fn primitive_field<'key>(&self, key: impl Into<FixKey<'key>>) -> Result<&Field> {
        let key = key.into();
        self.get_primitive_field(key).ok_or_else(|| absent(key))
    }

    /// Returns the field a key reaches, when that field carries a subtree.
    pub fn get_nested_field<'key>(&self, key: impl Into<FixKey<'key>>) -> Option<&Field> {
        self.get_field(key).filter(|field| is_nested(field))
    }

    /// Returns the nested field a key reaches, raising absence.
    ///
    /// # Errors
    ///
    /// Returns the absence [`Self::field`] raises when no field reaches the
    /// key, and when the one that does holds a single scalar.
    pub fn nested_field<'key>(&self, key: impl Into<FixKey<'key>>) -> Result<&Field> {
        let key = key.into();
        self.get_nested_field(key).ok_or_else(|| absent(key))
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
    /// The reverse of the digest an entry stores: a capture's `branch` column
    /// joins to a whole dialect declaration through this, which is what makes
    /// the capture self-describing rather than merely legible. The argument is
    /// the entry's own signed reading of the XXH32, so a digest above
    /// `i32::MAX` arrives negative and resolves exactly as it stored.
    pub fn get_branch_by_digest(&self, digest: i32) -> Option<&FixBranch> {
        self.branches.get(&super::entry::unsigned(digest))
    }

    /// Returns the branch one digest names, raising absence.
    ///
    /// # Errors
    ///
    /// Returns absence naming the digest when no branch carries it.
    pub fn branch_by_digest(&self, digest: i32) -> Result<&FixBranch> {
        self.get_branch_by_digest(digest).ok_or_else(|| {
            absent(format_args!(
                "branch #{:08x}",
                super::entry::unsigned(digest)
            ))
        })
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

    /// Adds a field, replacing only an equal canonical identity and name.
    pub fn insert(&mut self, field: Field) -> Result<Option<Field>> {
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

        self.ensure_branch(branch);
        match replacing {
            Some(position) => {
                self.departing(position);
                self.unindex(position, position);
                let prior = std::mem::replace(&mut self.fields[position], field);
                self.index(position);
                self.settle();
                Ok(Some(prior))
            }
            None => {
                let position = self.fields.len();
                self.fields.push(field);
                self.index(position);
                Ok(None)
            }
        }
    }

    /// Merges a definition into the field with the same canonical identity.
    pub fn update(&mut self, field: Field) -> Result<()> {
        let branch = field.as_fix().branch()?;
        self.check_branch(&branch)?;
        let id = canonical_id(&field)?;
        let Some(position) = self.canonical_position_by_id(id) else {
            return Err(absent(FixKey::Id(id)));
        };
        let stored = &self.fields[position];
        if !stored.name().eq_ignore_ascii_case(field.name()) {
            return Err(Error::InvalidRecord {
                path: field.name().into(),
                reason: crate::text::expected_got(
                    format_args!("the name {:?} stored for {id}", stored.name()),
                    format_args!("{:?}", field.name()),
                ),
            });
        }
        if stored.dtype() != field.dtype() {
            return Err(Error::InvalidRecord {
                path: field.name().into(),
                reason: crate::text::expected_got(
                    format_args!("the datatype {} stored for {id}", stored.dtype()),
                    field.dtype(),
                ),
            });
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
        merged.set_metadata(field.as_metadata().merge_with(stored.as_metadata())?.iter())?;
        merged.as_fix_mut().merge_with(&stored.as_fix())?;
        let alternate = alternate_ids(&merged, &branch)?;
        self.check_free(&merged, &branch, id, &alternate, Some(position))?;
        self.departing(position);
        self.unindex(position, position);
        self.fields[position] = merged;
        self.index(position);
        self.settle();
        Ok(())
    }

    /// Adds every field, folding into one already stored under its identity.
    ///
    /// Add or update, in bulk: a field whose canonical identity the dictionary
    /// does not hold is [inserted](Self::insert), and one it holds is
    /// [merged](Self::update). That is what reading a second source over a
    /// first wants - a definition the dictionary lacks arrives, and one it has
    /// keeps every key only it declares - where a bare `insert` would replace
    /// wholesale and a bare `update` would refuse everything new.
    ///
    /// The caller's order is the precedence, exactly as [`Self::update`]'s is:
    /// merge the lowest-priority source first and the highest arrives last and
    /// wins. Answers the count added and the count merged, in that order, and
    /// records the same pair through `log` at debug level.
    ///
    /// The identity is the whole probe. A tag alone is not: the same tag in
    /// two branches is two fields, and a merge keyed on the tag would fold a
    /// venue's definition into the specification's.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::insert`] and [`Self::update`] return - absence for
    /// a field carrying no `fix:tag`, a conflict for a key another field holds
    /// in the same branch, and a typed refusal for a name or a datatype that
    /// disagrees with the stored definition. A CBlock's generic `float` or
    /// `string` meeting the committed dictionary's `float64` or `msgtype` is
    /// that last one, and is the expected shape of a refusal here rather than
    /// a defect: a CBlock says nothing about which tag is money.
    ///
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
        *self = staged;
        Ok(counts)
    }

    /// Folds another dictionary into this one.
    ///
    /// The one place two dictionaries combine. Every field folds exactly as
    /// [`Self::add_fields`] folds one - absent identity inserts, stored
    /// identity merges - and every dialect folds beside them: one this
    /// dictionary does not hold arrives whole, and one it holds takes the
    /// incoming record while keeping every spelling it already answered to.
    ///
    /// Aliases accumulate rather than replace, because reading a second
    /// source is not a statement that the first one's names were wrong. The
    /// rest of a branch record is the incoming declaration's, whole, so the
    /// caller's order is the precedence here as it is everywhere else.
    ///
    /// Answers the count added and the count merged, over the fields.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::set_branch`], [`Self::insert`] and
    /// [`Self::update`] return. One mutation: the branches and the fields are
    /// staged together and adopted together, so a refusal anywhere leaves
    /// this dictionary exactly as it was.
    pub fn merge_with(&mut self, other: &Self) -> Result<(usize, usize)> {
        let mut staged = self.clone();
        staged.absorb_branches(other.branch_values(), None)?;
        let counts = staged.fold(other.fields.iter().cloned())?;
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
        let counts = staged.fold(parsed.fields)?;
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

    /// Adds every field, folding one already stored under its identity.
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
            // The crate's own fields are every dictionary's, so folding
            // them is folding a field onto itself: neither added nor merged,
            // and never a source's to redefine.
            if field
                .as_fix()
                .tag()
                .ok()
                .flatten()
                .is_some_and(super::is_crate_tag)
            {
                continue;
            }
            if self
                .canonical_position_by_id(canonical_id(&field)?)
                .is_some()
            {
                self.update(field)?;
                merged += 1;
            } else {
                self.insert(field)?;
                added += 1;
            }
        }
        log::debug!("added {added} and merged {merged} fix field definitions");
        Ok((added, merged))
    }

    /// Removes the field a tag, identifier, canonical name, or alias reaches.
    pub fn remove<'key>(&mut self, key: impl Into<FixKey<'key>>) -> Option<Field> {
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
        if position != last {
            self.index(position);
        }
        self.settle();
        if let Ok(Some(id)) = removed.as_fix().id() {
            if !self.positions_by_id.iter().any(|held| {
                self.fields
                    .get(*held)
                    .and_then(|field| field.as_fix().id().ok().flatten())
                    .is_some_and(|other| other.branch_digest() == id.branch_digest())
            }) {
                self.branches.remove(&id.branch_digest());
                self.branch_order
                    .retain(|digest| *digest != id.branch_digest());
            }
        }
        Some(removed)
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
        self.fields.is_empty()
    }

    pub(super) fn branch_values(&self) -> impl Iterator<Item = &FixBranch> {
        self.branch_order
            .iter()
            .filter_map(|digest| self.branches.get(digest))
    }

    fn check_branch(&self, branch: &FixBranch) -> Result<()> {
        if let Some(stored) = self.branches.get(&branch.digest()) {
            if !stored.has_identity(branch) {
                return Err(branch_collision(stored, branch));
            }
        }
        Ok(())
    }

    fn ensure_branch(&mut self, branch: FixBranch) {
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

    fn canonical_position_by_id(&self, id: FixId) -> Option<usize> {
        self.ids.get(&id).copied()
    }

    fn canonical_position_by_name(&self, branch: &FixBranch, name: &str) -> Option<usize> {
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

    fn canonical_name_matches(&self, position: usize, branch: &FixBranch, name: &str) -> bool {
        self.fields.get(position).is_some_and(|field| {
            field.name().eq_ignore_ascii_case(name)
                && field
                    .as_fix()
                    .id()
                    .ok()
                    .flatten()
                    .is_some_and(|id| id.branch_digest() == branch.digest())
        })
    }

    fn alias_matches(&self, position: usize, branch: &FixBranch, name: &str) -> bool {
        self.fields.get(position).is_some_and(|field| {
            field
                .as_fix()
                .aliases()
                .any(|alias| alias.eq_ignore_ascii_case(name))
                && field
                    .as_fix()
                    .id()
                    .ok()
                    .flatten()
                    .is_some_and(|id| id.branch_digest() == branch.digest())
        })
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
        let Ok(branch) = view.branch() else {
            return;
        };
        let Ok(Some(id)) = view.id() else {
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
            self.fields
                .get(*held)
                .and_then(|field| field.as_fix().id().ok().flatten())
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
        self.len() == other.len() && self.iter().eq(other.iter()) && self.branches == other.branches
    }
}

impl Eq for FixRegistry {}

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
