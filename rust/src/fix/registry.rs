//! The registry: one vector of fields and four indexes over it.
//!
//! Every index holds a position into the vector, so a lookup is one map
//! probe plus one slice index, and the two name indexes are keyed by
//! ASCII-case-folded text that is folded once, at insert. A rejected insert
//! or merge touches neither the vector nor any index: every key the change
//! would claim is checked free before anything is written.

use std::borrow::{Borrow, Cow};
use std::collections::{BTreeMap, HashMap, btree_map};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::iter::FusedIterator;

use smol_str::format_smolstr;

use super::FixKey;
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

/// The owned key a name index stores, probed with a borrowed [`Folded`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FoldedKey(Folded<'static>);

impl<'query> Borrow<Folded<'query>> for FoldedKey {
    fn borrow(&self) -> &Folded<'query> {
        &self.0
    }
}

/// One key a field can hold, rendered the way a conflict names it.
enum Held<'a> {
    Tag(i32),
    AlternateTag(i32),
    Name(&'a str),
    Alias(&'a str),
}

impl fmt::Display for Held<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tag(tag) => write!(formatter, "tag {tag}"),
            Self::AlternateTag(tag) => write!(formatter, "alternate tag {tag}"),
            Self::Name(name) => write!(formatter, "name {name:?}"),
            Self::Alias(alias) => write!(formatter, "alias {alias:?}"),
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

/// The identity a field enters the registry under: its canonical tag.
///
/// # Errors
///
/// Returns a typed absence naming the field when it carries no `fix:tag`,
/// or the parse failure when the stored tag is not one.
pub(super) fn canonical_tag(field: &Field) -> Result<i32> {
    field
        .as_fix()
        .tag()?
        .ok_or_else(|| Error::absent("fix:tag", field.name()))
}

/// FIX field definitions resolved by tag or by name.
///
/// Fields live in one vector at positions the four indexes point at:
/// canonical tags and alternate tags in two ordered maps, canonical names and
/// aliases in two folded-key maps. A lookup probes the canonical tier first
/// and the alternate tier only on a miss, so an alias can never take a name
/// away from a field that claims it canonically, and either answers the
/// canonical field itself - never the spelling the query used.
///
/// Identity is the pair of canonical tag and folded canonical name. Two
/// fields may share neither, nor an alternate tag, nor an alias; overlap
/// *across* tiers is allowed and resolved by tier order. [`Self::insert`]
/// never replaces a different field and [`Self::update`] merges into the one
/// with the same tag; both leave the registry untouched when they refuse.
///
/// ```
/// use yggdryl::{DataType, FixKey, FixRegistry};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut registry = FixRegistry::new();
///
/// let mut symbol = DataType::Utf8.required_field("Symbol");
/// symbol.as_fix_mut().set_tag(55)?;
/// symbol.as_fix_mut().set_aliases(["Ticker"])?;
/// assert_eq!(registry.insert(symbol)?, None);
///
/// // Any spelling finds the field; the answer is the canonical one.
/// assert_eq!(registry.field_by_name("TICKER")?.name(), "Symbol");
/// assert_eq!(registry.get_field(55), registry.get_field("symbol"));
/// assert!(registry.contains(FixKey::Tag(55)));
/// assert!(!registry.contains("55"));
/// assert_eq!(registry.len(), 1);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Default)]
pub struct FixRegistry {
    /// Every field, at the position the indexes point at.
    fields: Vec<Field>,
    /// Canonical tag to position.
    tags: BTreeMap<i32, usize>,
    /// Alternate tag to position.
    alternate_tags: BTreeMap<i32, usize>,
    /// Folded canonical name to position.
    names: HashMap<FoldedKey, usize>,
    /// Folded alias to position.
    aliases: HashMap<FoldedKey, usize>,
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

    /// Returns the field a canonical or alternate tag names.
    pub fn get_field_by_tag(&self, tag: i32) -> Option<&Field> {
        self.position_by_tag(tag)
            .map(|position| &self.fields[position])
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

    /// Returns the field a canonical name or alias names, ASCII case folded.
    pub fn get_field_by_name(&self, name: &str) -> Option<&Field> {
        self.position_by_name(name)
            .map(|position| &self.fields[position])
    }

    /// Returns the field a canonical name or alias names, raising absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the name.
    pub fn field_by_name(&self, name: &str) -> Result<&Field> {
        self.get_field_by_name(name)
            .ok_or_else(|| absent(FixKey::Name(name)))
    }

    /// Returns the field a dotted path reaches through a component or a
    /// repeating group.
    ///
    /// The whole string is tried as a name first, so a path with one segment
    /// is a name lookup. Otherwise the first segment resolves here, folded,
    /// and the remainder is [`Field::get_field_by_path`]'s to resolve, so
    /// nesting has one path resolver: `NoPartyIDs.PartyID` reaches the group
    /// member and `NoPartyIDs` the group itself.
    pub fn get_field_by_path(&self, path: &str) -> Option<&Field> {
        if let Some(field) = self.get_field_by_name(path) {
            return Some(field);
        }
        let (head, rest) = path.split_once('.')?;
        self.get_field_by_name(head)?.get_field_by_path(rest)
    }

    /// Returns the field a dotted path reaches, raising absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the path.
    pub fn field_by_path(&self, path: &str) -> Result<&Field> {
        self.get_field_by_path(path)
            .ok_or_else(|| absent(format_args!("path {path:?}")))
    }

    /// Returns the field a tag or a name reaches.
    ///
    /// The generic form matches the key once and redirects: a tag goes to
    /// [`Self::get_field_by_tag`] and a name to [`Self::get_field_by_path`],
    /// which is the name lookup plus the dotted descent.
    pub fn get_field<'key>(&self, key: impl Into<FixKey<'key>>) -> Option<&Field> {
        match key.into() {
            FixKey::Tag(tag) => self.get_field_by_tag(tag),
            FixKey::Name(name) => self.get_field_by_path(name),
        }
    }

    /// Returns the field a tag or a name reaches, raising absence.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::field_by_tag`] or [`Self::field_by_path`]
    /// raises, whichever the key selects.
    pub fn field<'key>(&self, key: impl Into<FixKey<'key>>) -> Result<&Field> {
        match key.into() {
            FixKey::Tag(tag) => self.field_by_tag(tag),
            FixKey::Name(name) => self.field_by_path(name),
        }
    }

    /// Returns whether a tag or a name reaches a field.
    pub fn contains<'key>(&self, key: impl Into<FixKey<'key>>) -> bool {
        self.get_field(key).is_some()
    }

    /// Adds a field, answering the one it replaced.
    ///
    /// A field enters only with a `fix:tag`. A fresh insert answers `None`.
    /// Re-inserting the same identity - canonical tag *and* folded canonical
    /// name both equal to one stored field's - replaces that field wholesale
    /// and answers the prior one. Anything in between is refused: a tag or a
    /// folded name another field holds canonically, an alternate tag another
    /// field lists, or an alias another field declares. Overlap across tiers
    /// is not a conflict; the tier order decides it.
    ///
    /// # Errors
    ///
    /// Returns a typed absence when the field carries no `fix:tag`, the
    /// parse failure when a `fix:` property is malformed, or a typed conflict
    /// naming both fields and the key. Failure leaves the registry unchanged.
    pub fn insert(&mut self, field: Field) -> Result<Option<Field>> {
        let tag = canonical_tag(&field)?;
        let alternate = field.as_fix().tags()?;
        let replacing = match (
            self.tags.get(&tag),
            self.names.get(&Folded::probe(field.name())),
        ) {
            (Some(by_tag), Some(by_name)) if by_tag == by_name => Some(*by_tag),
            _ => None,
        };
        self.check_free(&field, tag, &alternate, replacing)?;
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

    /// Merges a definition into the stored field with the same canonical tag.
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
    /// Returns a typed absence when no field holds the tag, a typed refusal
    /// naming both spellings when the folded names disagree or both datatypes
    /// when they do - a disagreement is never widened silently - and a typed
    /// conflict when a merged alternate tag or alias is another field's.
    pub fn update(&mut self, field: Field) -> Result<()> {
        let tag = canonical_tag(&field)?;
        let Some(&position) = self.tags.get(&tag) else {
            return Err(absent(FixKey::Tag(tag)));
        };
        let stored = &self.fields[position];
        if !stored.name().eq_ignore_ascii_case(field.name()) {
            return Err(Error::InvalidRecord {
                path: field.name().into(),
                reason: crate::text::expected_got(
                    format_args!("the name {:?} stored for tag {tag}", stored.name()),
                    format_args!("{:?}", field.name()),
                ),
            });
        }
        if stored.dtype() != field.dtype() {
            return Err(Error::InvalidRecord {
                path: field.name().into(),
                reason: crate::text::expected_got(
                    format_args!("the datatype {} stored for tag {tag}", stored.dtype()),
                    field.dtype(),
                ),
            });
        }
        let merged = merge(stored, &field)?;
        let alternate = merged.as_fix().tags()?;
        self.check_free(&merged, tag, &alternate, Some(position))?;
        self.unindex(position, position);
        self.fields[position] = merged;
        self.index(position);
        Ok(())
    }

    /// Removes the field a tag or a name reaches, answering it.
    ///
    /// A name here is a canonical name or an alias, never a dotted path: a
    /// member of a component is not a registry entry. The last field moves
    /// into the gap and every index entry follows it, so positions stay
    /// consistent without tombstones.
    pub fn remove<'key>(&mut self, key: impl Into<FixKey<'key>>) -> Option<Field> {
        let position = match key.into() {
            FixKey::Tag(tag) => self.position_by_tag(tag),
            FixKey::Name(name) => self.position_by_name(name),
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

    /// Iterates the fields in ascending canonical-tag order.
    pub fn iter(&self) -> FixFieldIter<'_> {
        FixFieldIter {
            positions: self.tags.values(),
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
    fn position_by_tag(&self, tag: i32) -> Option<usize> {
        self.tags
            .get(&tag)
            .or_else(|| self.alternate_tags.get(&tag))
            .copied()
    }

    /// The canonical tier first, the alternate tier only on a miss.
    fn position_by_name(&self, name: &str) -> Option<usize> {
        let probe = Folded::probe(name);
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
        tag: i32,
        alternate: &[i32],
        owner: Option<usize>,
    ) -> Result<()> {
        let other = |position: &usize| Some(*position) != owner;
        if let Some(holder) = self.tags.get(&tag).filter(|position| other(position)) {
            return Err(conflict(Held::Tag(tag), field, &self.fields[*holder]));
        }
        if let Some(holder) = self
            .names
            .get(&Folded::probe(field.name()))
            .filter(|position| other(position))
        {
            return Err(conflict(
                Held::Name(field.name()),
                field,
                &self.fields[*holder],
            ));
        }
        for tag in alternate {
            if let Some(holder) = self
                .alternate_tags
                .get(tag)
                .filter(|position| other(position))
            {
                return Err(conflict(
                    Held::AlternateTag(*tag),
                    field,
                    &self.fields[*holder],
                ));
            }
        }
        for alias in field.as_fix().aliases() {
            if let Some(holder) = self
                .aliases
                .get(&Folded::probe(alias))
                .filter(|position| other(position))
            {
                return Err(conflict(Held::Alias(alias), field, &self.fields[*holder]));
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
        if let Ok(Some(tag)) = view.tag() {
            self.tags.insert(tag, position);
        }
        for tag in view.tags().unwrap_or_default() {
            self.alternate_tags.insert(tag, position);
        }
        self.names
            .insert(FoldedKey(Folded::owned(field.name())), position);
        for alias in view.aliases() {
            self.aliases
                .insert(FoldedKey(Folded::owned(alias)), position);
        }
    }

    /// Drop every entry of the field at `position` that still points at
    /// `pointing_at`, leaving another field's entries under the same key alone.
    fn unindex(&mut self, position: usize, pointing_at: usize) {
        let field = &self.fields[position];
        let view = field.as_fix();
        if let Ok(Some(tag)) = view.tag() {
            if self.tags.get(&tag) == Some(&pointing_at) {
                self.tags.remove(&tag);
            }
        }
        for tag in view.tags().unwrap_or_default() {
            if self.alternate_tags.get(&tag) == Some(&pointing_at) {
                self.alternate_tags.remove(&tag);
            }
        }
        let name = Folded::probe(field.name());
        if self.names.get(&name) == Some(&pointing_at) {
            self.names.remove(&name);
        }
        for alias in view.aliases() {
            let alias = Folded::probe(alias);
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
    /// Renders every field under its canonical tag, in tag order.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_map()
            .entries(
                self.tags
                    .iter()
                    .map(|(tag, position)| (tag, &self.fields[*position])),
            )
            .finish()
    }
}

impl PartialEq for FixRegistry {
    /// Compares the fields alone, in canonical-tag order, so two registries
    /// that received the same fields in different orders are equal.
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

/// The fields of a registry in ascending canonical-tag order.
///
/// Answered by [`FixRegistry::iter`]. It walks the canonical-tag index, so
/// the order is deterministic whatever order the fields entered in.
#[derive(Clone, Debug)]
pub struct FixFieldIter<'registry> {
    positions: btree_map::Values<'registry, i32, usize>,
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
