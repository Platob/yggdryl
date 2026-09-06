//! The FIX registry: one field vector, four compact indexes, and one branch
//! table.
//!
//! Canonical and alternate identifiers are keyed directly by packed
//! [`FixId`] values. Canonical names and aliases are keyed by independent
//! seeded XXH64 digests; every hit is rechecked against the field, so a digest
//! collision is a miss on read and a typed conflict on mutation. Ordered
//! iteration is kept separately as sorted field positions. The registry is
//! built rarely and resolved constantly, so that `O(n)` insertion trade is
//! deliberate.

use std::collections::HashMap;
use std::fmt;
use std::hash::{BuildHasherDefault, Hasher};
use std::iter::FusedIterator;

use smol_str::format_smolstr;

use super::{FixBranch, FixId, FixKey};
use crate::xxhash::Xxh64;
use crate::{Error, Field, MimeType, Result};

const NAME_SEED: u64 = 0x4e41_4d45_5f46_4958;
const ALIAS_SEED: u64 = 0x414c_4941_535f_4649;
/// FIX's official `XmlData` payload tag.
const XML_DATA_TAG: i32 = 213;
/// The one routing name for FIX user-defined MsgTypes (`U` plus a suffix).
const UDF_MSGTYPE: &[u8] = b"UDF";

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
}

#[cfg(test)]
pub(super) const fn control_byte(id: FixId) -> u8 {
    (Mix::finalise(id.0 as u64) >> 57) as u8
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
        self.0 = u64::from(value);
    }

    fn write_i32(&mut self, value: i32) {
        self.0 = value as u32 as u64;
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = value;
    }

    fn write_i64(&mut self, value: i64) {
        self.0 = value as u64;
    }
}

type Index<K> = HashMap<K, usize, BuildHasherDefault<Mix>>;
type BranchTable = HashMap<u32, FixBranch, BuildHasherDefault<Mix>>;

/// Fold a name directly into a seeded streaming state.
fn name_digest(branch: &FixBranch, name: &str, domain: u64) -> u64 {
    let mut state = Xxh64::with_seed(domain ^ u64::from(branch.digest()));
    let mut folded = [0_u8; 64];
    for bytes in name.as_bytes().chunks(folded.len()) {
        for (target, source) in folded.iter_mut().zip(bytes) {
            *target = source.to_ascii_lowercase();
        }
        state.write(&folded[..bytes.len()]);
    }
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

/// FIX field definitions resolved by packed identity or folded name.
#[derive(Clone, Default)]
pub struct FixRegistry {
    fields: Vec<Field>,
    ids: Index<FixId>,
    alternate_ids: Index<FixId>,
    names: Index<u64>,
    aliases: Index<u64>,
    positions_by_id: Vec<usize>,
    branches: BranchTable,
    branch_order: Vec<u32>,
}

impl FixRegistry {
    /// The empty registry.
    pub fn new() -> Self {
        Self::default()
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

    /// Returns the field a canonical or alternate packed identifier names.
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
        self.get_field_by_name(head, branch)?
            .get_field_by_path(rest)
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

    /// Returns the branch for `id`.
    pub fn branch_of(&self, id: FixId) -> Option<&FixBranch> {
        self.branches.get(&id.branch_digest())
    }

    /// Returns the branch whose canonical spelling is `name`.
    pub fn branch_named(&self, name: &str) -> Option<&FixBranch> {
        let branch = FixBranch::from_str(name).ok()?;
        self.branches
            .get(&branch.digest())
            .filter(|held| held.has_identity(&branch))
    }

    /// Returns the branch declaring one exact session pair, ASCII-folded.
    pub fn branch_for_session(&self, sender: &str, target: &str) -> Option<&FixBranch> {
        self.branch_values().find(|branch| {
            !branch.sender_comp_id().is_empty()
                && branch.sender_comp_id().eq_ignore_ascii_case(sender)
                && !branch.target_comp_id().is_empty()
                && branch.target_comp_id().eq_ignore_ascii_case(target)
        })
    }

    /// Iterates the branches held by this registry.
    pub fn branches(&self) -> impl Iterator<Item = &FixBranch> {
        self.branch_values()
    }

    /// Installs or replaces one complete branch declaration.
    pub fn set_branch(&mut self, branch: FixBranch) -> Result<()> {
        if branch.is_standard()
            && (branch.version() != Default::default()
                || !branch.sender_comp_id().is_empty()
                || !branch.target_comp_id().is_empty())
        {
            return Err(Error::InvalidRecord {
                path: "".into(),
                reason: "the standard FIX branch declares no dialect or session".into(),
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

    /// Infers the FIX-shaped media type embedded in an arbitrary byte line.
    ///
    /// Numeric `tag=value` entries prove [`MimeType::FIX`], symbolic keys that
    /// this registry resolves prove [`MimeType::ULLINK`], both prove
    /// [`MimeType::FIXUL`], and an XML payload proves [`MimeType::FIXML`].
    /// Unrelated log attributes therefore do not turn an otherwise random
    /// line into Ullink. Unknown input answers
    /// [`MimeType::OCTET_STREAM`]. The scan is shallow and allocates nothing.
    pub fn infer_bytes_protocol(&self, line: &[u8]) -> MimeType {
        self.inspect_line(line).mime_type()
    }

    /// Infers the FIX-shaped media type embedded in an arbitrary text line.
    pub fn infer_text_protocol(&self, line: &str) -> MimeType {
        self.infer_bytes_protocol(line.as_bytes())
    }

    /// Extracts the raw MsgType value from a FIX-shaped frame in a log line.
    ///
    /// The same shallow scan used by protocol inference handles numeric FIX,
    /// symbolic Ullink, mixed FIXUL, and FIXML entries. A raw `MSGTYPE=` pair
    /// is checked across the whole line before numeric tag 35 and therefore
    /// wins when both are present. It neither parses nor allocates the message
    /// and returns a slice of the caller's bytes.
    pub fn infer_bytes_msgtype<'line>(&self, line: &'line [u8]) -> Option<&'line [u8]> {
        let inferred = self.inspect_line_with(line, true);
        inferred
            .name_msgtype
            .or(inferred.tag_msgtype)
            .map(route_msgtype)
    }

    /// Extracts the raw MsgType text from a FIX frame embedded in a log line.
    pub fn infer_text_msgtype<'line>(&self, line: &'line str) -> Option<&'line str> {
        let value = self.infer_bytes_msgtype(line.as_bytes())?;
        std::str::from_utf8(value).ok()
    }

    fn inspect_line<'line>(&self, line: &'line [u8]) -> LineInference<'line> {
        self.inspect_line_with(line, false)
    }

    fn inspect_line_with<'line>(
        &self,
        line: &'line [u8],
        infer_msgtype: bool,
    ) -> LineInference<'line> {
        let raw_msgtype = find_named_value(line, b"MSGTYPE");
        let mut inferred = LineInference {
            has_name: raw_msgtype.is_some(),
            name_msgtype: infer_msgtype.then_some(raw_msgtype).flatten(),
            ..LineInference::default()
        };
        let Some(frame) = locate_frame(line) else {
            return inferred;
        };
        let msgtype = infer_msgtype
            .then(|| {
                self.get_field_by_tag(35)
                    .or_else(|| self.get_field_by_name("MsgType", Some(&FixBranch::STANDARD)))
            })
            .flatten();
        let mut offset = frame.start;
        while let Some(entry) = next_entry(line, &mut offset, frame.separator) {
            let mut checksum = false;
            match entry.key {
                LineKey::Tag(tag) => {
                    inferred.has_tag = true;
                    if tag == XML_DATA_TAG {
                        if entry.value.starts_with(b"<") {
                            inferred.has_xml = true;
                        } else if memchr::memchr(b'=', entry.value).is_some() {
                            inferred.has_name = true;
                        }
                    }
                    checksum = tag == 10;
                    let resolves_to_msgtype = msgtype.is_some_and(|wanted| {
                        self.get_field_by_tag(tag)
                            .is_some_and(|field| std::ptr::eq(wanted, field))
                    });
                    if infer_msgtype
                        && (tag == 35 || resolves_to_msgtype)
                        && inferred.tag_msgtype.is_none()
                    {
                        inferred.tag_msgtype = Some(entry.value);
                    }
                }
                LineKey::Name(name) => {
                    let named_msgtype = name.eq_ignore_ascii_case(b"MSGTYPE");
                    let Ok(name) = std::str::from_utf8(name) else {
                        continue;
                    };
                    let field = self.get_field_by_name(name, Some(&FixBranch::STANDARD));
                    if frame.numeric || named_msgtype || field.is_some() {
                        inferred.has_name = true;
                    }
                    let resolves_to_msgtype = msgtype.is_some_and(|wanted| {
                        field.is_some_and(|field| std::ptr::eq(wanted, field))
                    });
                    if infer_msgtype
                        && (named_msgtype || resolves_to_msgtype)
                        && inferred.name_msgtype.is_none()
                    {
                        inferred.name_msgtype = Some(entry.value);
                    }
                }
            }
            if checksum || (!infer_msgtype && inferred.has_tag && inferred.has_name) {
                break;
            }
        }
        inferred
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
                self.unindex(position, position);
                let prior = std::mem::replace(&mut self.fields[position], field);
                self.index(position);
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
        let merged = merge(stored, &field)?;
        let alternate = alternate_ids(&merged, &branch)?;
        self.check_free(&merged, &branch, id, &alternate, Some(position))?;
        self.unindex(position, position);
        self.fields[position] = merged;
        self.index(position);
        Ok(())
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
        self.unindex(position, position);
        let removed = self.fields.swap_remove(position);
        if position != last {
            self.index(position);
        }
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

#[derive(Clone, Copy)]
enum LineKey<'line> {
    Tag(i32),
    Name(&'line [u8]),
}

#[derive(Clone, Copy)]
struct LineEntry<'line> {
    key: LineKey<'line>,
    value: &'line [u8],
}

#[derive(Clone, Copy)]
struct LineFrame {
    start: usize,
    numeric: bool,
    separator: LineSeparator,
}

#[derive(Clone, Copy)]
enum LineSeparator {
    Byte(u8),
    Marker(&'static [u8]),
    Whitespace,
}

#[derive(Default)]
struct LineInference<'line> {
    has_tag: bool,
    has_name: bool,
    has_xml: bool,
    tag_msgtype: Option<&'line [u8]>,
    name_msgtype: Option<&'line [u8]>,
}

impl LineInference<'_> {
    const fn mime_type(&self) -> MimeType {
        if self.has_xml {
            return MimeType::FIXML;
        }
        match (self.has_tag, self.has_name) {
            (false, false) => MimeType::OCTET_STREAM,
            (false, true) => MimeType::ULLINK,
            (true, false) => MimeType::FIX,
            (true, true) => MimeType::FIXUL,
        }
    }
}

/// Find a raw symbolic key/value pair anywhere in a log line.
///
/// This intentionally runs before numeric-frame location: log prefixes can
/// carry Ullink `MSGTYPE=` before an embedded `8=FIX...` frame. The returned
/// value is borrowed and bounded by the first common entry separator.
fn find_named_value<'line>(line: &'line [u8], wanted: &[u8]) -> Option<&'line [u8]> {
    for start in 0..line.len() {
        let Some((LineKey::Name(name), equals)) = pair_at(line, start) else {
            continue;
        };
        if !name.eq_ignore_ascii_case(wanted) {
            continue;
        }
        let value_start = equals + 1;
        let mut end = value_start;
        while end < line.len()
            && !matches!(
                line[end],
                0x01 | b'|'
                    | b' '
                    | b'\t'
                    | b'\r'
                    | b'\n'
                    | b','
                    | b';'
                    | b']'
                    | b')'
                    | b'}'
                    | b'^'
                    | b'<'
                    | b'{'
                    | b'\\'
            )
        {
            end += 1;
        }
        if end > value_start {
            return Some(&line[value_start..end]);
        }
    }
    None
}

/// Route the standard's `U*` user-defined range through one dictionary root.
fn route_msgtype(value: &[u8]) -> &[u8] {
    if value.len() > 1 && value[0] == b'U' && value[1..].iter().all(u8::is_ascii_alphanumeric) {
        UDF_MSGTYPE
    } else {
        value
    }
}

fn locate_frame(line: &[u8]) -> Option<LineFrame> {
    let mut first = None;
    let mut msgtype = None;
    for start in 0..line.len() {
        let Some((key, _)) = pair_at(line, start) else {
            continue;
        };
        let candidate = (start, matches!(key, LineKey::Tag(_)));
        first.get_or_insert(candidate);
        match key {
            LineKey::Tag(8) => return Some(frame(line, candidate)),
            LineKey::Tag(35) if msgtype.is_none() => msgtype = Some(candidate),
            LineKey::Tag(_) | LineKey::Name(_) => {}
        }
    }
    msgtype.or(first).map(|candidate| frame(line, candidate))
}

fn frame(line: &[u8], (start, numeric): (usize, bool)) -> LineFrame {
    LineFrame {
        start,
        numeric,
        separator: LineSeparator::for_line(line, start, numeric),
    }
}

impl LineSeparator {
    fn for_line(line: &[u8], start: usize, numeric: bool) -> Self {
        let tail = &line[start..];
        if !numeric {
            return if memchr::memchr(b'|', tail).is_some() {
                Self::Byte(b'|')
            } else {
                Self::Whitespace
            };
        }

        let mut found: Option<(usize, Self)> = None;
        for separator in [
            Self::Byte(0x01),
            Self::Byte(b'|'),
            Self::Marker(b"^A"),
            Self::Marker(b"\\x01"),
            Self::Marker(b"<SOH>"),
            Self::Marker(b"{SOH}"),
        ] {
            let position = match separator {
                Self::Byte(byte) => memchr::memchr(byte, tail),
                Self::Marker(marker) => memchr::memmem::find(tail, marker),
                Self::Whitespace => None,
            };
            if let Some(position) = position {
                if found.is_none_or(|(held, _)| position < held) {
                    found = Some((position, separator));
                }
            }
        }
        found.map_or(Self::Whitespace, |(_, separator)| separator)
    }

    fn segment(self, line: &[u8], start: usize) -> (usize, usize) {
        match self {
            Self::Byte(byte) => match memchr::memchr(byte, &line[start..]) {
                Some(relative) => (start + relative, start + relative + 1),
                None => (line.len(), line.len()),
            },
            Self::Marker(marker) => match memchr::memmem::find(&line[start..], marker) {
                Some(relative) => (start + relative, start + relative + marker.len()),
                None => (line.len(), line.len()),
            },
            Self::Whitespace => {
                let mut end = start;
                while end < line.len() && !line[end].is_ascii_whitespace() {
                    end += 1;
                }
                let mut next = end;
                while next < line.len() && line[next].is_ascii_whitespace() {
                    next += 1;
                }
                (end, next)
            }
        }
    }
}

fn next_entry<'line>(
    line: &'line [u8],
    offset: &mut usize,
    separator: LineSeparator,
) -> Option<LineEntry<'line>> {
    while *offset < line.len() {
        while *offset < line.len() && line[*offset].is_ascii_whitespace() {
            *offset += 1;
        }
        let start = *offset;
        let (end, next) = separator.segment(line, start);
        *offset = next;
        let Some((key, equals)) = pair_at(line, start) else {
            if next == line.len() {
                return None;
            }
            continue;
        };
        if equals >= end {
            continue;
        }
        let mut value_end = end;
        while value_end > equals + 1
            && matches!(
                line[value_end - 1],
                b' ' | b'\t' | b'\r' | b'\n' | b']' | b')' | b'}' | b',' | b';'
            )
        {
            value_end -= 1;
        }
        if value_end > equals + 1 {
            return Some(LineEntry {
                key,
                value: &line[equals + 1..value_end],
            });
        }
    }
    None
}

fn pair_at(line: &[u8], start: usize) -> Option<(LineKey<'_>, usize)> {
    if !is_field_start(line, start) {
        return None;
    }
    let mut key_start = start;
    if line.get(key_start) == Some(&b'#') {
        key_start += 1;
    }
    let first = *line.get(key_start)?;
    let (key, equals) = if first.is_ascii_digit() {
        let mut position = key_start;
        let mut tag = Some(0_i32);
        while position < line.len() && line[position].is_ascii_digit() {
            tag = tag.and_then(|tag| {
                tag.checked_mul(10)
                    .and_then(|tag| tag.checked_add(i32::from(line[position] - b'0')))
            });
            position += 1;
        }
        (LineKey::Tag(tag?), position)
    } else if is_name_start(first) {
        let mut position = key_start + 1;
        while position < line.len() && is_name_continue(line[position]) {
            position += 1;
        }
        (LineKey::Name(&line[key_start..position]), position)
    } else {
        return None;
    };
    if line.get(equals) != Some(&b'=') {
        return None;
    }
    let first_value = *line.get(equals + 1)?;
    if matches!(first_value, b'\'' | b'"') || is_field_end(line, equals + 1) {
        return None;
    }
    Some((key, equals))
}

const fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

const fn is_name_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
}

fn is_field_start(line: &[u8], position: usize) -> bool {
    position == 0
        || matches!(
            line[position - 1],
            0x01 | b'|' | b' ' | b'\t' | b'[' | b'(' | b'{' | b'>'
        )
        || (position >= 2 && &line[position - 2..position] == b"^A")
        || (position >= 4 && &line[position - 4..position] == b"\\x01")
        || (position >= 5 && &line[position - 5..position] == b"<SOH>")
        || (position >= 5 && &line[position - 5..position] == b"{SOH}")
}

fn is_field_end(line: &[u8], position: usize) -> bool {
    matches!(
        line[position],
        0x01 | b'|' | b' ' | b'\t' | b'\r' | b'\n' | b']' | b')' | b'}' | b',' | b';'
    ) || line[position..].starts_with(b"^A")
        || line[position..].starts_with(b"\\x01")
        || line[position..].starts_with(b"<SOH>")
        || line[position..].starts_with(b"{SOH}")
}

fn merge(stored: &Field, incoming: &Field) -> Result<Field> {
    let mut merged = incoming.clone();
    merged.set_metadata(
        incoming
            .as_metadata()
            .merge_with(stored.as_metadata())?
            .iter(),
    )?;
    let mut tags = incoming.as_fix().tags()?;
    for tag in stored.as_fix().tags()? {
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    merged.as_fix_mut().set_tags(&tags)?;
    let mut aliases: Vec<&str> = incoming.as_fix().aliases().collect();
    for alias in stored.as_fix().aliases() {
        if !aliases.iter().any(|held| held.eq_ignore_ascii_case(alias)) {
            aliases.push(alias);
        }
    }
    merged.as_fix_mut().set_aliases(&aliases)?;
    Ok(merged)
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
        let collided = name_digest(&FixBranch::STANDARD, incoming.name(), NAME_SEED);
        registry.names.insert(collided, 0);

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
