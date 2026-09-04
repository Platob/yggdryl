//! The registry: one vector of fields and four indexes over it.
//!
//! Every index holds a position into the vector, so a lookup is one map
//! probe plus one slice index; the two identifier indexes are ordered maps
//! keyed by [`FixId`], and the two name indexes are keyed by a branch
//! beside ASCII-case-folded text that is folded once, at insert. A rejected
//! insert or merge touches neither the vector nor any index: every key the
//! change would claim is checked free before anything is written.

use std::borrow::{Borrow, Cow};
use std::collections::{BTreeMap, HashMap, btree_map};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::iter::FusedIterator;
use std::ops::Bound;

use smol_str::format_smolstr;

use super::{FixBranch, FixId, FixKey};
use crate::{Error, Field, Result};

/// Text compared and hashed with ASCII case folded, without being rewritten.
///
/// A name index must answer a query spelled in any case without allocating,
/// so the stored key and the probe are one type whose `Hash` and `Eq` fold
/// each byte as they read it. The stored key owns its text and the probe
/// borrows the caller's; `Cow` is what lets one type do both, and because
/// `Cow<'a, str>` is covariant in `'a`, a `&Folded<'static>` serves wherever
/// a `&Folded<'query>` is asked for - which is what [`Borrow`] on
/// [`FoldedKey`] relies on. An unsized `str` newtype would do the same with
/// one pointer cast, but the crate denies unsafe code; this is the safe
/// equivalent, and it costs the probe nothing.
#[derive(Clone, Debug)]
pub(super) struct Folded<'a>(Cow<'a, str>);

impl<'a> Folded<'a> {
    /// Probe an index with a caller's text, folding as it is read.
    pub(super) const fn probe(text: &'a str) -> Self {
        Self(Cow::Borrowed(text))
    }
}

impl Folded<'static> {
    /// Own a copy of `text`, for the key an index stores.
    fn owned(text: &str) -> Self {
        Self(Cow::Owned(text.to_owned()))
    }
}

impl PartialEq for Folded<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }
}

impl Eq for Folded<'_> {}

impl Hash for Folded<'_> {
    /// Hashes the folded bytes in bounded chunks, then a terminator.
    ///
    /// Both the stored key and the probe hash through this one impl, so the
    /// two agree by construction rather than by the hasher treating a byte
    /// at a time the same as a slice.
    fn hash<H: Hasher>(&self, state: &mut H) {
        let mut chunk = [0_u8; 32];
        for bytes in self.0.as_bytes().chunks(chunk.len()) {
            for (slot, byte) in chunk.iter_mut().zip(bytes) {
                *slot = byte.to_ascii_lowercase();
            }
            state.write(&chunk[..bytes.len()]);
        }
        state.write_u8(0xff);
    }
}

/// One name inside one dictionary: what a name index is keyed by.
///
/// A name is unique per branch rather than registry-wide, because a venue
/// dictionary reusing `Symbol` or `TradeID` is the normal case. The branch
/// is an inline [`FixBranch`] compared exactly - it is already canonical
/// lowercase - and the name keeps folding as it hashes, so the proven
/// [`Folded`] machinery is wrapped rather than rebuilt and a probe still
/// allocates nothing.
#[derive(Clone, Debug)]
pub(super) struct BranchedName<'a> {
    branch: FixBranch,
    name: Folded<'a>,
}

impl<'a> BranchedName<'a> {
    /// Probe an index with a caller's text, folding as it is read.
    pub(super) fn probe(branch: &FixBranch, name: &'a str) -> Self {
        Self {
            branch: branch.clone(),
            name: Folded::probe(name),
        }
    }
}

impl BranchedName<'static> {
    /// Own a copy of `name`, for the key an index stores.
    fn owned(branch: FixBranch, name: &str) -> Self {
        Self {
            branch,
            name: Folded::owned(name),
        }
    }
}

impl PartialEq for BranchedName<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.branch == other.branch && self.name == other.name
    }
}

impl Eq for BranchedName<'_> {}

impl Hash for BranchedName<'_> {
    /// Hashes the branch, a separator its grammar cannot hold, then the
    /// folded name, so the concatenation of the two is unambiguous.
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write(self.branch.as_str().as_bytes());
        state.write_u8(0x00);
        self.name.hash(state);
    }
}

/// The owned key a name index stores, probed with a borrowed
/// [`BranchedName`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct BranchedKey(BranchedName<'static>);

impl<'query> Borrow<BranchedName<'query>> for BranchedKey {
    fn borrow(&self) -> &BranchedName<'query> {
        &self.0
    }
}

/// One key a field can hold, rendered the way a conflict names it.
///
/// Every rendering names the branch, because a key is only ever unique
/// inside one.
enum Held<'a> {
    Id(FixId),
    AlternateId(FixId),
    Name(&'a FixBranch, &'a str),
    Alias(&'a FixBranch, &'a str),
}

impl fmt::Display for Held<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(id) => write!(formatter, "identifier {id}"),
            Self::AlternateId(id) => write!(formatter, "alternate identifier {id}"),
            Self::Name(branch, name) => {
                write!(formatter, "name {name:?} in branch {:?}", branch.as_str())
            }
            Self::Alias(branch, alias) => {
                write!(formatter, "alias {alias:?} in branch {:?}", branch.as_str())
            }
        }
    }
}

/// Report that `incoming` claims a key `holder` already holds.
fn conflict(held: Held<'_>, incoming: &Field, holder: &Field) -> Error {
    Error::conflict(
        "fix field",
        "fix field",
        format_smolstr!("{held} of {}, held by {}", incoming.name(), holder.name()),
    )
}

/// Report that nothing is registered under `what`.
fn absent(what: impl fmt::Display) -> Error {
    Error::absent("fix field", what)
}

/// The identity a field enters the registry under.
///
/// # Errors
///
/// Returns a typed absence naming the field when it carries no `fix:tag`,
/// the parse failure when a stored `fix:` property is malformed, or
/// [`FixId::from_parts`]'s refusal when the field claims a specification tag
/// for another dictionary.
pub(super) fn canonical_id(field: &Field) -> Result<FixId> {
    field
        .as_fix()
        .id()?
        .ok_or_else(|| Error::absent("fix:tag", field.name()))
}

/// The alternate identities a field lists, in stored priority order.
fn alternate_ids(field: &Field, branch: &FixBranch) -> Result<Vec<FixId>> {
    field
        .as_fix()
        .tags()?
        .into_iter()
        .map(|tag| FixId::from_parts(branch.clone(), tag))
        .collect()
}

/// FIX field definitions resolved by identifier or by name.
///
/// Fields live in one vector at positions the four indexes point at:
/// canonical and alternate identifiers in two ordered maps, canonical names
/// and aliases in two maps keyed by branch plus folded text. A lookup
/// probes the canonical tier first and the alternate tier only on a miss, so
/// an alias can never take a name away from a field that claims it
/// canonically, and either answers the canonical field itself - never the
/// spelling the query used.
///
/// Identity is the [`FixId`] and, separately, the pair of branch and
/// folded canonical name. Two fields may share neither, nor an alternate
/// identifier, nor an alias in one branch; two *branches* may define the
/// same name and the same tag, and a conflict is only ever within one.
/// Overlap *across* tiers is allowed and resolved by tier order.
/// [`Self::insert`] never replaces a different field and [`Self::update`]
/// merges into the one with the same identifier; both leave the registry
/// untouched when they refuse.
///
/// ```
/// use yggdryl::{DataType, FixId, FixKey, FixBranch, FixRegistry};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut registry = FixRegistry::new();
/// let cme = FixBranch::from_str("cme")?;
///
/// let mut symbol = DataType::Utf8.required_field("Symbol");
/// symbol.as_fix_mut().set_tag(55)?;
/// symbol.as_fix_mut().set_aliases(["Ticker"])?;
/// assert_eq!(registry.insert(symbol)?, None);
///
/// // The same name in another dictionary is a different field.
/// let mut venue = DataType::Utf8.required_field("Symbol");
/// venue.as_fix_mut().set_id(&FixId::from_parts(cme.clone(), 5055)?)?;
/// assert_eq!(registry.insert(venue)?, None);
///
/// // Any spelling finds the field; the answer is the canonical one.
/// assert_eq!(registry.field_by_name(&FixBranch::STANDARD, "TICKER")?.name(), "Symbol");
/// assert_eq!(registry.get_field(55), registry.get_field("symbol"));
/// assert!(registry.contains(FixKey::Tag(55)));
/// assert!(!registry.contains("55"));
/// assert_eq!(registry.field_by_name(&cme, "symbol")?.as_fix().tag()?, Some(5055));
/// assert_eq!(registry.len(), 2);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Default)]
pub struct FixRegistry {
    /// Every field, at the position the indexes point at.
    fields: Vec<Field>,
    /// Canonical identifier to position.
    ids: BTreeMap<FixId, usize>,
    /// Alternate identifier to position.
    alternate_ids: BTreeMap<FixId, usize>,
    /// Branch and folded canonical name to position.
    names: HashMap<BranchedKey, usize>,
    /// Branch and folded alias to position.
    aliases: HashMap<BranchedKey, usize>,
}

impl FixRegistry {
    /// The empty registry.
    ///
    /// Not `const`: a hash map's random state cannot be built in one.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a registry by inserting `fields` in order.
    ///
    /// # Errors
    ///
    /// Returns the first insert's refusal, which fails the whole build.
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
    ///
    /// This carries the implementation the tag pair redirects to: an
    /// identifier is the exact address of one field in one dictionary, and a
    /// tag is that address in the standard branch.
    pub fn get_field_by_id(&self, id: &FixId) -> Option<&Field> {
        self.position_by_id(id)
            .map(|position| &self.fields[position])
    }

    /// Returns the field a canonical or alternate identifier names, raising
    /// absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the identifier.
    pub fn field_by_id(&self, id: &FixId) -> Result<&Field> {
        self.get_field_by_id(id)
            .ok_or_else(|| absent(FixKey::Id(id)))
    }

    /// Returns the field a canonical or alternate tag names in the standard
    /// branch.
    ///
    /// A bare tag is always `standard:<tag>`, never whichever dictionary
    /// happens to be loaded: below [`FixId::STANDARD_TAG_LIMIT`] no other
    /// branch may hold it, and above it a vendor field is addressed by its
    /// [`FixId`].
    pub fn get_field_by_tag(&self, tag: i32) -> Option<&Field> {
        self.get_field_by_id(&FixId::standard(tag))
    }

    /// Returns the field a canonical or alternate tag names, raising absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the tag.
    pub fn field_by_tag(&self, tag: i32) -> Result<&Field> {
        self.get_field_by_tag(tag)
            .ok_or_else(|| absent(FixKey::Tag(tag)))
    }

    /// Returns the field a canonical name or alias names inside one
    /// dictionary, ASCII case folded.
    pub fn get_field_by_name(&self, branch: &FixBranch, name: &str) -> Option<&Field> {
        self.position_by_name(branch, name)
            .map(|position| &self.fields[position])
    }

    /// Returns the field a canonical name or alias names, raising absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the name.
    pub fn field_by_name(&self, branch: &FixBranch, name: &str) -> Result<&Field> {
        self.get_field_by_name(branch, name)
            .ok_or_else(|| absent(FixKey::Name(name)))
    }

    /// Returns the field a dotted path reaches through a component or a
    /// repeating group.
    ///
    /// The whole string is tried as a name first, so a path with one segment
    /// is a name lookup. Otherwise the first segment resolves here, folded,
    /// and the remainder is [`Field::get_field_by_path`]'s to resolve, so
    /// nesting has one path resolver: `NoPartyIDs.PartyID` reaches the group
    /// member and `NoPartyIDs` the group itself. The whole path resolves in
    /// one branch.
    pub fn get_field_by_path(&self, branch: &FixBranch, path: &str) -> Option<&Field> {
        if let Some(field) = self.get_field_by_name(branch, path) {
            return Some(field);
        }
        let (head, rest) = path.split_once('.')?;
        self.get_field_by_name(branch, head)?
            .get_field_by_path(rest)
    }

    /// Returns the field a dotted path reaches, raising absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the path.
    pub fn field_by_path(&self, branch: &FixBranch, path: &str) -> Result<&Field> {
        self.get_field_by_path(branch, path)
            .ok_or_else(|| absent(format_args!("path {path:?}")))
    }

    /// Returns the field a tag, an identifier or a name reaches.
    ///
    /// The generic form matches the key once and redirects: a tag goes to
    /// [`Self::get_field_by_tag`], an identifier to [`Self::get_field_by_id`],
    /// and a name to [`Self::get_field_by_path`] in the standard branch,
    /// which is the name lookup plus the dotted descent.
    pub fn get_field<'key>(&self, key: impl Into<FixKey<'key>>) -> Option<&Field> {
        match key.into() {
            FixKey::Tag(tag) => self.get_field_by_tag(tag),
            FixKey::Id(id) => self.get_field_by_id(id),
            FixKey::Name(name) => self.get_field_by_path(&FixBranch::STANDARD, name),
        }
    }

    /// Returns the field a tag, an identifier or a name reaches, raising
    /// absence.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::field_by_tag`], [`Self::field_by_id`] or
    /// [`Self::field_by_path`] raises, whichever the key selects.
    pub fn field<'key>(&self, key: impl Into<FixKey<'key>>) -> Result<&Field> {
        match key.into() {
            FixKey::Tag(tag) => self.field_by_tag(tag),
            FixKey::Id(id) => self.field_by_id(id),
            FixKey::Name(name) => self.field_by_path(&FixBranch::STANDARD, name),
        }
    }

    /// Returns whether a tag, an identifier or a name reaches a field.
    pub fn contains<'key>(&self, key: impl Into<FixKey<'key>>) -> bool {
        self.get_field(key).is_some()
    }

    /// Adds a field, answering the one it replaced.
    ///
    /// A field enters only with a `fix:tag`. A fresh insert answers `None`.
    /// Re-inserting the same identity - canonical identifier *and* branched
    /// folded canonical name both equal to one stored field's - replaces that
    /// field wholesale and answers the prior one. Anything in between is
    /// refused: an identifier or a branched folded name another field holds
    /// canonically, an alternate identifier another field lists, or an alias
    /// another field declares in the same branch. Overlap across tiers, and
    /// any overlap across branches, is not a conflict; the tier order
    /// decides the first and nothing crosses the second.
    ///
    /// # Errors
    ///
    /// Returns a typed absence when the field carries no `fix:tag`, the
    /// parse failure when a `fix:` property is malformed, the standard-tag
    /// refusal when it claims a specification tag for another dictionary, or
    /// a typed conflict naming both fields and the key. Failure leaves the
    /// registry unchanged.
    pub fn insert(&mut self, field: Field) -> Result<Option<Field>> {
        let id = canonical_id(&field)?;
        let alternate = alternate_ids(&field, id.branch())?;
        let replacing = match (
            self.ids.get(&id),
            self.names
                .get(&BranchedName::probe(id.branch(), field.name())),
        ) {
            (Some(by_id), Some(by_name)) if by_id == by_name => Some(*by_id),
            _ => None,
        };
        self.check_free(&field, &id, &alternate, replacing)?;
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

    /// Merges a definition into the stored field with the same canonical
    /// identifier.
    ///
    /// The incoming field wins the name spelling, nullability, and every
    /// metadata key both declare; the stored field keeps the keys only it
    /// declares; `fix:tags` and `fix:aliases` concatenate, incoming first,
    /// deduplicated with aliases folded, order kept. The merged field is
    /// built first and every key it would newly claim checked free, so a
    /// refusal leaves the vector and all four indexes as they were.
    ///
    /// # Errors
    ///
    /// Returns a typed absence when no field holds the identifier - a
    /// branch disagreement is exactly that, because the branch is half
    /// of it - a typed refusal naming both spellings when the folded names
    /// disagree or both datatypes when they do - a disagreement is never
    /// widened silently - and a typed conflict when a merged alternate
    /// identifier or alias is another field's.
    pub fn update(&mut self, field: Field) -> Result<()> {
        let id = canonical_id(&field)?;
        let Some(&position) = self.ids.get(&id) else {
            return Err(absent(FixKey::Id(&id)));
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
        let alternate = alternate_ids(&merged, id.branch())?;
        self.check_free(&merged, &id, &alternate, Some(position))?;
        self.unindex(position, position);
        self.fields[position] = merged;
        self.index(position);
        Ok(())
    }

    /// Removes the field a tag, an identifier or a name reaches, answering it.
    ///
    /// A name here is a canonical name or an alias in the standard branch,
    /// never a dotted path: a member of a component is not a registry entry.
    /// The last field moves into the gap and every index entry follows it, so
    /// positions stay consistent without tombstones.
    pub fn remove<'key>(&mut self, key: impl Into<FixKey<'key>>) -> Option<Field> {
        let position = match key.into() {
            FixKey::Tag(tag) => self.position_by_id(&FixId::standard(tag)),
            FixKey::Id(id) => self.position_by_id(id),
            FixKey::Name(name) => self.position_by_name(&FixBranch::STANDARD, name),
        }?;
        self.unindex(position, position);
        let removed = self.fields.swap_remove(position);
        if position < self.fields.len() {
            // The former last field now lives at `position`, and its entries
            // still point at the slot it left.
            let former = self.fields.len();
            self.unindex(position, former);
            self.index(position);
        }
        Some(removed)
    }

    /// Returns the first field after `after`, or the first for `None`.
    ///
    /// This is the cursor form an owning FFI iterator advances with, the way
    /// [`ProtocolField::next_entry`](crate::ProtocolField::next_entry) is: a
    /// binding holds the registry and one [`FixId`], so lazy iteration crosses
    /// the boundary without cloning the dictionary or borrowing across it. The
    /// order is [`Self::iter`]'s, ascending canonical identifier.
    ///
    /// ```
    /// use yggdryl::{DataType, FixId, FixRegistry};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut price = DataType::Float64.required_field("Price");
    /// price.as_fix_mut().set_tag(44)?;
    /// let mut symbol = DataType::Utf8.required_field("Symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let registry = FixRegistry::from_fields([price, symbol])?;
    ///
    /// let first = registry.next_field_after(None).expect("a first field");
    /// assert_eq!(first.name(), "Price");
    /// let second = registry.next_field_after(first.as_fix().id()?.as_ref());
    /// assert_eq!(second.map(yggdryl::Field::name), Some("Symbol"));
    /// assert!(registry.next_field_after(Some(&FixId::standard(55))).is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn next_field_after(&self, after: Option<&FixId>) -> Option<&Field> {
        let entry = match after {
            Some(id) => self
                .ids
                .range((Bound::Excluded(id), Bound::Unbounded))
                .next(),
            None => self.ids.iter().next(),
        };
        self.fields.get(*entry?.1)
    }

    /// Iterates the fields in ascending canonical-identifier order, which is
    /// branch-major and then by tag.
    pub fn iter(&self) -> FixFieldIter<'_> {
        FixFieldIter {
            positions: self.ids.values(),
            fields: &self.fields,
        }
    }

    /// Returns how many fields are registered.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Returns whether no field is registered.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// The canonical tier first, the alternate tier only on a miss.
    fn position_by_id(&self, id: &FixId) -> Option<usize> {
        self.ids
            .get(id)
            .or_else(|| self.alternate_ids.get(id))
            .copied()
    }

    /// The canonical tier first, the alternate tier only on a miss.
    fn position_by_name(&self, branch: &FixBranch, name: &str) -> Option<usize> {
        let probe = BranchedName::probe(branch, name);
        self.names
            .get(&probe)
            .or_else(|| self.aliases.get(&probe))
            .copied()
    }

    /// Refuse `field` when any key it claims is held by a position other than
    /// `owner`.
    fn check_free(
        &self,
        field: &Field,
        id: &FixId,
        alternate: &[FixId],
        owner: Option<usize>,
    ) -> Result<()> {
        let branch = id.branch();
        let other = |position: &usize| Some(*position) != owner;
        if let Some(holder) = self.ids.get(id).filter(|position| other(position)) {
            return Err(conflict(Held::Id(id.clone()), field, &self.fields[*holder]));
        }
        if let Some(holder) = self
            .names
            .get(&BranchedName::probe(branch, field.name()))
            .filter(|position| other(position))
        {
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
                    Held::AlternateId(alternate.clone()),
                    field,
                    &self.fields[*holder],
                ));
            }
        }
        for alias in field.as_fix().aliases() {
            if let Some(holder) = self
                .aliases
                .get(&BranchedName::probe(branch, alias))
                .filter(|position| other(position))
            {
                return Err(conflict(
                    Held::Alias(branch, alias),
                    field,
                    &self.fields[*holder],
                ));
            }
        }
        Ok(())
    }

    /// Point every key of the field at `position` at that position.
    ///
    /// The keys were parsed and validated when the field entered, and a
    /// stored field is handed out by shared reference only, so the parses
    /// here cannot fail; a failure is treated as the key being absent.
    fn index(&mut self, position: usize) {
        let field = &self.fields[position];
        let view = field.as_fix();
        let branch = view.branch().unwrap_or_default();
        if let Ok(Some(id)) = view.id() {
            self.ids.insert(id, position);
        }
        for tag in view.tags().unwrap_or_default() {
            if let Ok(id) = FixId::from_parts(branch.clone(), tag) {
                self.alternate_ids.insert(id, position);
            }
        }
        self.names.insert(
            BranchedKey(BranchedName::owned(branch.clone(), field.name())),
            position,
        );
        for alias in view.aliases() {
            self.aliases.insert(
                BranchedKey(BranchedName::owned(branch.clone(), alias)),
                position,
            );
        }
    }

    /// Drop every entry of the field at `position` that still points at
    /// `pointing_at`, leaving another field's entries under the same key alone.
    fn unindex(&mut self, position: usize, pointing_at: usize) {
        let field = &self.fields[position];
        let view = field.as_fix();
        let branch = view.branch().unwrap_or_default();
        if let Ok(Some(id)) = view.id() {
            if self.ids.get(&id) == Some(&pointing_at) {
                self.ids.remove(&id);
            }
        }
        for tag in view.tags().unwrap_or_default() {
            let Ok(id) = FixId::from_parts(branch.clone(), tag) else {
                continue;
            };
            if self.alternate_ids.get(&id) == Some(&pointing_at) {
                self.alternate_ids.remove(&id);
            }
        }
        let name = BranchedName::probe(&branch, field.name());
        if self.names.get(&name) == Some(&pointing_at) {
            self.names.remove(&name);
        }
        for alias in view.aliases() {
            let alias = BranchedName::probe(&branch, alias);
            if self.aliases.get(&alias) == Some(&pointing_at) {
                self.aliases.remove(&alias);
            }
        }
    }
}

/// Build the field `update` stores: the incoming definition over the stored
/// one, with the two list properties concatenated.
fn merge(stored: &Field, incoming: &Field) -> Result<Field> {
    let mut merged = incoming.clone();
    // The union of both, the incoming field winning any key they share.
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
    /// Renders every field under its canonical identifier, in identifier
    /// order.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_map()
            .entries(
                self.ids
                    .iter()
                    .map(|(id, position)| (id.to_string(), &self.fields[*position])),
            )
            .finish()
    }
}

impl PartialEq for FixRegistry {
    /// Compares the fields alone, in canonical-identifier order, so two
    /// registries that received the same fields in different orders are equal.
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
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

/// The fields of a registry in ascending canonical-identifier order.
///
/// Answered by [`FixRegistry::iter`]. It walks the canonical-identifier
/// index, which is branch-major and then by tag, so the order is
/// deterministic whatever order the fields entered in.
#[derive(Clone, Debug)]
pub struct FixFieldIter<'registry> {
    positions: btree_map::Values<'registry, FixId, usize>,
    fields: &'registry [Field],
}

impl<'registry> Iterator for FixFieldIter<'registry> {
    type Item = &'registry Field;

    fn next(&mut self) -> Option<Self::Item> {
        self.positions
            .next()
            .map(|position| &self.fields[*position])
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.positions.size_hint()
    }
}

impl DoubleEndedIterator for FixFieldIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.positions
            .next_back()
            .map(|position| &self.fields[*position])
    }
}

impl ExactSizeIterator for FixFieldIter<'_> {}

impl FusedIterator for FixFieldIter<'_> {}
