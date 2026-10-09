//! Named FIX definitions beside the scalar wire-field indexes.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::group_plan::GroupPlan;
use super::registry::name_digest;
use super::store::{DefinitionKey, compact, reference};
use super::{FixDrop, FixField, FixFieldMut, FixId, FixRegistry, MsgType};
use crate::implementer::folds_equal;
use crate::{DataType, Error, Field, FixCategory, Result, StructType};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Definition {
    pub category: FixCategory,
    pub field: DefinitionField,
}

#[derive(Clone, Debug)]
pub(super) enum DefinitionField {
    Plain(Field),
    Group(Field, Arc<GroupPlan>),
    Message(MsgType),
}

impl DefinitionField {
    pub fn as_field(&self) -> &Field {
        match self {
            Self::Plain(field) | Self::Group(field, _) => field,
            Self::Message(message) => message.as_field(),
        }
    }

    fn into_field(self) -> Field {
        match self {
            Self::Plain(field) | Self::Group(field, _) => field,
            Self::Message(message) => message.into_field(),
        }
    }

    fn message(&self) -> Option<&MsgType> {
        match self {
            Self::Message(message) => Some(message),
            Self::Plain(_) | Self::Group(_, _) => None,
        }
    }

    /// A component carrying `FIX:msgtype` is a message: the
    /// marker, not the category, decides whether it compiles as one.
    fn from_field(category: FixCategory, field: Field) -> Result<Self> {
        match category {
            FixCategory::Components if FixField::new(&field).msgtype().is_some() => {
                Ok(Self::Message(MsgType::from_field(field)?))
            }
            FixCategory::Groups => {
                let plan = Arc::new(GroupPlan::from_field(&field)?);
                Ok(Self::Group(field, plan))
            }
            _ => Ok(Self::Plain(field)),
        }
    }
}

impl PartialEq for DefinitionField {
    fn eq(&self, other: &Self) -> bool {
        self.as_field() == other.as_field()
    }
}
impl Eq for DefinitionField {}

impl std::ops::Deref for DefinitionField {
    type Target = Field;
    fn deref(&self) -> &Field {
        self.as_field()
    }
}

#[derive(Clone, Debug)]
struct MessageAlias {
    spelling: SmolStr,
    code: Option<SmolStr>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Catalog {
    entries: Vec<Definition>,
    names: super::registry::FixMap<(FixCategory, u64), usize>,
    order: Vec<usize>,
    /// Each counter tag, and the one group it opens - `None` where two
    /// groups claim one counter, which names nothing.
    counters: super::registry::FixMap<i32, Option<usize>>,
    /// Each wire code, and the message a bare code answers: the one named
    /// as tag 35's code set names the code, else the first in name order.
    message_codes: HashMap<SmolStr, usize>,
    /// The components carrying `FIX:msgtype`, in name order: what
    /// `msgtypes` iterates and `msgtype_at` indexes.
    messages: Vec<usize>,
    message_aliases: super::registry::FixMap<u64, MessageAlias>,
    /// Each wire code, and the name tag 35's code set gives it.
    code_names: HashMap<SmolStr, SmolStr>,
    /// What each definition's metadata states, by the address of that
    /// metadata's storage, which a message's child stated under the
    /// definition shares: a hint a reader verifies against the entry it
    /// names, so a definition replaced or moved since costs one metadata
    /// read and never a wrong answer.
    facts: super::registry::FixMap<usize, (usize, super::registry::FieldFacts)>,
}

impl PartialEq for Catalog {
    fn eq(&self, other: &Self) -> bool {
        self.all().eq(other.all())
    }
}
impl Eq for Catalog {}

impl Catalog {
    fn key(category: FixCategory, name: &str) -> (FixCategory, u64) {
        (category, name_digest(name, 0x4341_5441_4c4f_4753))
    }

    fn position(&self, category: FixCategory, name: &str) -> Option<usize> {
        let position = *self.names.get(&Self::key(category, name))?;
        let entry = self.entries.get(position)?;
        crate::implementer::folds_equal(entry.field.name(), name).then_some(position)
    }

    pub fn iter(&self, category: FixCategory) -> impl Iterator<Item = &Field> {
        let start = self
            .order
            .partition_point(|position| self.entries[*position].category < category);
        let end = self
            .order
            .partition_point(|position| self.entries[*position].category <= category);
        self.order[start..end]
            .iter()
            .map(|position| self.entries[*position].field.as_field())
    }

    fn index_counters(&mut self) {
        self.counters.clear();
        for (position, entry) in self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.category == FixCategory::Groups)
        {
            if let Some(tag) = FixField::new(&entry.field).counter().ok().flatten() {
                self.counters
                    .entry(tag)
                    .and_modify(|held| *held = None)
                    .or_insert(Some(position));
            }
        }
    }

    /// The message a bare code answers, where two declare it under two
    /// names.
    ///
    /// The one named as tag 35's code set names the code, because that is
    /// what the specification calls the message and what a reader of `35=D`
    /// means; where no message is so named, or no code set names the code,
    /// the first in name order. Both are facts of the catalog's content and
    /// not of the order it was built in, so a dictionary folded, stored and
    /// loaded answers the same message. A second message on the code is
    /// reached by its own name.
    fn index_messages(&mut self) {
        self.messages = self
            .order
            .iter()
            .copied()
            .filter(|position| self.entries[*position].field.message().is_some())
            .collect();
        let mut codes: HashMap<SmolStr, usize> = HashMap::new();
        for (position, entry) in self.entries.iter().enumerate() {
            let Some(message) = entry.field.message() else {
                continue;
            };
            let code = message.as_str();
            let names_code = |name: &str| {
                self.code_names
                    .get(code)
                    .is_some_and(|named| crate::implementer::folds_equal(named, name))
            };
            match codes.entry(SmolStr::new(code)) {
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(position);
                }
                std::collections::hash_map::Entry::Occupied(mut slot) => {
                    let held = self.entries[*slot.get()].field.name();
                    let arriving = entry.field.name();
                    let takes = if names_code(held) {
                        false
                    } else if names_code(arriving) {
                        true
                    } else {
                        arriving < held
                    };
                    if takes {
                        slot.insert(position);
                    }
                }
            }
        }
        self.message_codes = codes;
    }

    /// The message one spelling names: an exact wire code, a folded name,
    /// or tag 35's own alias for a code.
    fn message_position(&self, spelling: &str) -> Option<usize> {
        if let Some(position) = self.message_codes.get(spelling) {
            return Some(*position);
        }
        if let Some(position) = self.position(FixCategory::Components, spelling) {
            // A definition found by its folded name answers a wire code only
            // where it does not contradict one. A message named `b` carrying
            // wire code `B` is News under a spelling MassQuoteAcknowledgement
            // answers to, and handing it back for `b` would answer one
            // message for the other - so a name that folds onto this
            // message's own code without being it names nothing here, and the
            // exact index above stays the one reading a wire code has.
            let code = self.entries[position]
                .field
                .message()
                .map(super::msgtype::MsgType::as_str);
            let contradicts = code.is_some_and(|code| {
                code != spelling && crate::implementer::folds_equal(code, spelling)
            });
            if code.is_some() && !contradicts {
                return Some(position);
            }
        }
        let alias = self
            .message_aliases
            .get(&name_digest(spelling, 0x4d53_475f_414c_4941))?;
        if !crate::implementer::folds_equal(&alias.spelling, spelling) {
            return None;
        }
        let code = alias.code.as_ref()?;
        self.message_codes.get(code).copied()
    }

    fn index_message_aliases(&mut self, codes: Option<&str>) {
        self.message_aliases.clear();
        self.code_names.clear();
        let Some(codes) = codes else {
            // The code set decides which message a bare code answers, so
            // its going re-decides them.
            self.index_messages();
            return;
        };
        for code in super::codes::FixCodes::over(Some(codes)).filter_map(Result::ok) {
            self.code_names
                .insert(SmolStr::new(code.value()), SmolStr::new(code.name()));
            for spelling in std::iter::once(code.name()).chain(code.aliases()) {
                // A spelling that is the code's own wire value is not indexed
                // here. `message_codes` answers it exactly above, which is the
                // one reading a wire value has, while this index is folded -
                // so indexing it would let `b` and `B` null each other and
                // leave two real messages reachable by neither spelling.
                if spelling == code.value() {
                    continue;
                }
                let digest = name_digest(spelling, 0x4d53_475f_414c_4941);
                self.message_aliases
                    .entry(digest)
                    .and_modify(|held| {
                        if !crate::implementer::folds_equal(&held.spelling, spelling)
                            || held.code.as_deref() != Some(code.value())
                        {
                            held.code = None;
                        }
                    })
                    .or_insert_with(|| MessageAlias {
                        spelling: spelling.into(),
                        code: Some(code.value().into()),
                    });
            }
        }
        self.index_messages();
    }

    pub fn all(&self) -> impl Iterator<Item = &Definition> {
        self.order.iter().map(|position| &self.entries[*position])
    }

    pub fn insert(&mut self, category: FixCategory, field: Field) -> Result<Option<Field>> {
        let prior = self.push(category, field)?;
        // A replacement lands in the position the name already had, so the
        // order is the one it was: only an append reorders.
        if prior.is_none() {
            self.sort_order();
        }
        if category == FixCategory::Groups {
            self.index_counters();
        }
        if category == FixCategory::Components {
            self.index_messages();
        }
        Ok(prior)
    }

    /// Adds one definition, leaving the order and the derived indexes for
    /// [`Self::settle_indexes`].
    ///
    /// The door a catalog built whole comes through. Settling per entry is
    /// O(C) work repeated C times - a full re-sort of the order and a full
    /// rebuild of the counter and message indexes, every one of them thrown
    /// away by the next entry - for a result only the last of them survives.
    ///
    /// A caller that reads the catalog between pushes must settle first:
    /// [`Self::iter`] and [`Self::at`] `partition_point` over `order`, so
    /// they need it monotone by category, and both derived indexes are
    /// stale until settled.
    pub(super) fn push(&mut self, category: FixCategory, field: Field) -> Result<Option<Field>> {
        if category == FixCategory::Groups {
            FixField::new(&field)
                .counter()?
                .ok_or_else(|| Error::absent("FIX:counter", field.name()))?;
        }
        let key = Self::key(category, field.name());
        if let Some(position) = self.names.get(&key).copied() {
            if self.position(category, field.name()) != Some(position) {
                return Err(Error::conflict(
                    "FIX definition name",
                    "FIX name digest collision",
                    field.name(),
                ));
            }
            if self.entries[position].field.name() != field.name() {
                return Err(Error::conflict(
                    "the existing canonical FIX definition spelling",
                    "a different canonical spelling",
                    crate::implementer::expected_got(
                        self.entries[position].field.name(),
                        field.name(),
                    ),
                ));
            }
            let field = DefinitionField::from_field(category, field)?;
            let prior = std::mem::replace(&mut self.entries[position].field, field).into_field();
            self.remember_facts(position);
            return Ok(Some(prior));
        }
        let position = self.entries.len();
        let field = DefinitionField::from_field(category, field)?;
        self.entries.push(Definition { category, field });
        self.names.insert(key, position);
        self.order.push(position);
        self.remember_facts(position);
        Ok(None)
    }

    /// Reads what the definition at `position` states off its metadata,
    /// once, for every reader of a message stated under it.
    fn remember_facts(&mut self, position: usize) {
        let field = self.entries[position].field.as_field();
        if let Some(facts) = super::registry::FieldFacts::of(field) {
            self.facts.insert(
                crate::implementer::metadata_storage_address(field.as_metadata()),
                (position, facts),
            );
        }
    }

    fn index_facts(&mut self) {
        self.facts.clear();
        for position in 0..self.entries.len() {
            self.remember_facts(position);
        }
    }

    /// What `field`'s metadata states, where the field is one of these
    /// definitions or shares its metadata with one.
    pub(super) fn facts_of(&self, field: &Field) -> Option<super::registry::FieldFacts> {
        let (position, facts) = *self
            .facts
            .get(&crate::implementer::metadata_storage_address(
                field.as_metadata(),
            ))?;
        let held = self.entries.get(position)?.field.as_field();
        crate::implementer::metadata_shares_storage_with(held.as_metadata(), field.as_metadata())
            .then_some(facts)
    }

    /// The iteration order: by category, then by name within it.
    fn sort_order(&mut self) {
        self.order.sort_by(|left, right| {
            let left = &self.entries[*left];
            let right = &self.entries[*right];
            (left.category, left.field.name()).cmp(&(right.category, right.field.name()))
        });
    }

    /// Settles what a run of [`Self::push`] left: the iteration order and
    /// both derived indexes, each once over the whole catalog.
    pub(super) fn settle_indexes(&mut self) {
        self.sort_order();
        self.index_counters();
        self.index_messages();
    }

    fn remove(&mut self, position: usize) -> Field {
        let removed = self.entries.remove(position).field.into_field();
        self.names.clear();
        self.order.retain(|held| *held != position);
        for held in &mut self.order {
            if *held > position {
                *held -= 1;
            }
        }
        for (position, entry) in self.entries.iter().enumerate() {
            self.names
                .insert(Self::key(entry.category, entry.field.name()), position);
        }
        self.index_facts();
        self.index_counters();
        self.index_messages();
        removed
    }
}

fn invalid(field: &Field, expected: impl std::fmt::Display) -> Error {
    Error::InvalidRecord {
        path: field.name().into(),
        reason: crate::implementer::expected_got(expected, field.dtype()),
    }
}

/// The refusal a nested datatype meets where a scalar field was expected.
pub(super) fn not_scalar(field: &Field) -> Error {
    invalid(field, "a scalar datatype")
}

/// The datatype one category's definitions have, held to once here for the
/// validation and the fold alike.
fn check_shape(category: FixCategory, field: &Field) -> Result<()> {
    match category {
        FixCategory::Fields if field.dtype().is_nested() && !is_column_serie(field) => {
            Err(not_scalar(field))
        }
        FixCategory::Components if !matches!(field.dtype(), DataType::Struct(_)) => {
            Err(invalid(field, "a Struct datatype"))
        }
        FixCategory::Groups if definition_category(field) != Some(FixCategory::Groups) => Err(
            invalid(field, "a Serie of non-null Struct occurrences or a Map"),
        ),
        _ => Ok(()),
    }
}

/// A nested field read as one column, or the refusal a nested field that is
/// no definition and no column earns.
pub(super) fn column_shape(field: &Field) -> Result<()> {
    if is_column_serie(field) {
        Ok(())
    } else {
        Err(not_scalar(field))
    }
}

/// Whether a nested field is one column of this crate's own rather than a
/// definition.
///
/// A serie of non-null scalars - the identities a message descends from, each
/// a `Uuid` - is one value a row holds under one name. It is not a repeating
/// group, because a group's occurrence is a Struct of members a wire states
/// one tag at a time, and it is not a component.
///
/// Only in this crate's own tag block. A wire tag carries one value, so a
/// dialect handing the dictionary a serie under one is stating a group badly
/// and is told so; the crate's columns answer no wire tag at all, and a serie
/// is what `srcuuids` is.
fn is_column_serie(field: &Field) -> bool {
    let crate_tag = FixField::new(field)
        .tag()
        .ok()
        .flatten()
        .is_some_and(super::is_crate_tag);
    if !crate_tag {
        return false;
    }
    match field.dtype() {
        DataType::Serie(item) | DataType::LargeSerie(item) => {
            !item.is_nullable() && !item.dtype().is_nested()
        }
        _ => false,
    }
}

/// The category a nested field's shape names, for a caller handing the
/// registry a definition without saying which it is.
///
/// A repeating group holds non-null Struct occurrences, in a Serie or a Map,
/// and every Struct is a component, a message among them
/// being the component whose `FIX:msgtype` names a wire code.
/// The shape alone answers; the marker is a property of the component. A
/// nested datatype that is neither is no definition at all, and answers
/// nothing.
pub(super) fn definition_category(field: &Field) -> Option<FixCategory> {
    match field.dtype() {
        DataType::Serie(item) | DataType::LargeSerie(item)
            if !item.is_nullable() && matches!(item.dtype(), DataType::Struct(_)) =>
        {
            Some(FixCategory::Groups)
        }
        DataType::Map(_) | DataType::SortedMap(_) => Some(FixCategory::Groups),
        DataType::Struct(_) => Some(FixCategory::Components),
        _ => None,
    }
}

/// Whether an occurrence's datatype restates the definition it references.
///
/// Equality, except that two structs are their children: a struct declared
/// empty and one built empty hold the same nothing, whichever allocation
/// each has.
fn restates_datatype(occurrence: &DataType, target: &DataType) -> bool {
    match (occurrence, target) {
        (DataType::Struct(held), DataType::Struct(declared)) => {
            held.as_fields() == declared.as_fields()
        }
        (held, declared) => held == declared,
    }
}

/// The occurrence a group's serie or map holds.
pub(super) fn occurrence_of(group: &Field) -> Option<&Field> {
    match group.dtype() {
        DataType::Serie(item) | DataType::LargeSerie(item) => Some(item),
        DataType::Map(map) | DataType::SortedMap(map) => Some(map.entries()),
        _ => None,
    }
}

/// Rebuilds only the occurrence, keeping the group's storage contract.
fn group_dtype(group: &Field, occurrence: Field) -> Result<DataType> {
    match group.dtype() {
        DataType::Serie(_) => Ok(DataType::serie(occurrence)),
        DataType::LargeSerie(_) => Ok(DataType::large_serie(occurrence)),
        map_dtype @ (DataType::Map(_) | DataType::SortedMap(_)) => {
            let map = &map_dtype
                .as_mapping()
                .expect("the variant was just matched");
            DataType::map(occurrence, map.keys_sorted())
        }
        _ => Err(invalid(
            group,
            "a Serie of non-null Struct occurrences or a Map",
        )),
    }
}

/// What a child is, for a refusal that names both sides: a reference by its
/// target and the datatype that target has, an inline child by its own.
fn describe(field: &Field, resolved: Option<&DataType>) -> SmolStr {
    match (reference(field), resolved) {
        (Some((category, name)), Some(dtype)) => {
            format_smolstr!("a reference to {category} {name:?}, the datatype {dtype}")
        }
        (Some((category, name)), None) => format_smolstr!("a reference to {category} {name:?}"),
        (None, _) => format_smolstr!("the datatype {}", field.dtype()),
    }
}

/// The incoming document's metadata laid over the stored one's, on the
/// stored identity.
///
/// Name, nullability and tag are the stored definition's, and the fold has
/// the precedence a scalar [`FixRegistry::update`] has: the generic keys
/// through the metadata merge every protocol shares, the `FIX:` keys through
/// the rule each one has - which is also what holds the two sides to one
/// message code, one counter, one component. The datatype is left for the
/// caller, who merges the children.
fn merge_root(stored: &Field, incoming: &Field) -> Result<Field> {
    let mut merged = incoming.clone();
    merged.set_name(stored.name());
    merged.set_nullable(stored.is_nullable());
    merged.set_metadata(
        incoming
            .as_metadata()
            .merge_with(stored.as_metadata())?
            .iter(),
    )?;
    // A definition's tag is the registry's derivation for its name, not a
    // statement the incoming side gets to make; the occurrence inside a
    // group has none, and an incoming one carrying it is stating nothing.
    match FixField::new(stored).tag()? {
        Some(tag) => FixFieldMut::new(&mut merged).set_tag(tag)?,
        None => {
            merged.remove_metadata(super::field::TAG_KEY);
        }
    }
    FixFieldMut::new(&mut merged).merge_with(&FixField::new(stored))?;
    Ok(merged)
}

fn set_merged_dtype(field: &mut Field, dtype: DataType) -> Result<()> {
    field.set_dtype(dtype)?;
    // The incoming declaration wins, but member order is the merged
    // component's. Resolve it against that final shape once at intake.
    FixFieldMut::new(field).normalize_identifiers()
}

/// The compact documents a fold writes over, in the shape the resolver reads.
///
/// A merge is settled on documents rather than on resolved trees, because a
/// reference's expansion is the target's business, and the map is what
/// [`FixRegistry::resolve_catalog`] derives the catalog from - so the fold's
/// output is the store's input. The index beside it finds a document by the
/// folded name the catalog keys it under, and every hit is rechecked against
/// the key, exactly as the catalog rechecks its own.
pub(super) struct Documents {
    raw: BTreeMap<DefinitionKey, Field>,
    keys: HashMap<(FixCategory, u64), Vec<DefinitionKey>>,
    /// The derived tag each document holds, so a definition arriving with
    /// another dictionary's derivation is seen to collide before it lands.
    tags: HashMap<i32, DefinitionKey>,
    /// What every write replaced since the oldest open [checkpoint], oldest
    /// first: what a rollback puts back. Empty while no checkpoint is open,
    /// so a fold nothing brackets keeps nothing.
    ///
    /// [checkpoint]: Self::checkpoint
    journal: Vec<Replaced>,
    /// How many checkpoints are open.
    open: usize,
    /// What the source a fold reads states under a name the documents held,
    /// by the folded name a reference reaches it by: where an incoming
    /// member's target is read from ([`Self::incoming`]). Empty outside
    /// [`FixRegistry::merge_catalog`].
    source: HashMap<(FixCategory, u64), Vec<Field>>,
}

/// What one write to [`Documents`] replaced.
struct Replaced {
    key: DefinitionKey,
    /// The document the key held before, or nothing where the write added it.
    document: Option<Field>,
    /// The derived tag the written document holds, and the key that tag
    /// indexed before the write.
    tag: Option<(i32, Option<DefinitionKey>)>,
}

impl Documents {
    fn from_registry(registry: &FixRegistry) -> Result<Self> {
        let mut documents = Self {
            raw: BTreeMap::new(),
            keys: HashMap::new(),
            tags: HashMap::new(),
            journal: Vec::new(),
            open: 0,
            source: HashMap::new(),
        };
        for (key, document) in registry.compact_catalog()? {
            documents.put(key, document);
        }
        Ok(documents)
    }

    /// Opens a checkpoint: every write from here on can be undone by
    /// [`Self::rollback`] with the mark this answers, and is forgotten by
    /// [`Self::release`] once the outermost checkpoint closes.
    ///
    /// What lets one source, or one definition of it, be one mutation of
    /// documents shared by a whole fold: the undo costs the writes it
    /// undoes, never a copy of the catalog.
    pub(super) fn checkpoint(&mut self) -> usize {
        self.open += 1;
        self.journal.len()
    }

    /// Closes the checkpoint `mark` opened, keeping what was written since.
    pub(super) fn release(&mut self, mark: usize) {
        debug_assert!(mark <= self.journal.len(), "a mark this journal answered");
        self.open = self.open.saturating_sub(1);
        if self.open == 0 {
            self.journal.clear();
        }
    }

    /// Undoes every write since `mark` was answered, newest first, and closes
    /// that checkpoint: the documents are exactly what they were at it.
    pub(super) fn rollback(&mut self, mark: usize) {
        while self.journal.len() > mark {
            let Some(Replaced { key, document, tag }) = self.journal.pop() else {
                break;
            };
            match document {
                Some(document) => {
                    self.raw.insert(key, document);
                }
                None => {
                    self.raw.remove(&key);
                    let index = Catalog::key(key.0, &key.1);
                    if let Some(held) = self.keys.get_mut(&index) {
                        held.retain(|held| held != &key);
                        if held.is_empty() {
                            self.keys.remove(&index);
                        }
                    }
                }
            }
            if let Some((tag, previous)) = tag {
                match previous {
                    Some(previous) => {
                        self.tags.insert(tag, previous);
                    }
                    None => {
                        self.tags.remove(&tag);
                    }
                }
            }
        }
        self.release(mark);
    }

    /// Takes the document `key` holds out, answering it: undone by the
    /// rollback of a checkpoint open over it. The key stays indexed, which
    /// [`Self::get`] rechecks against the documents and [`Self::put`] adds no
    /// second of.
    fn remove(&mut self, key: &DefinitionKey) -> Option<Field> {
        let document = self.raw.remove(key)?;
        let tag = FixField::new(&document).tag().ok().flatten().map(|tag| {
            self.tags.remove(&tag);
            (tag, Some(key.clone()))
        });
        if self.open > 0 {
            self.journal.push(Replaced {
                key: key.clone(),
                document: Some(document.clone()),
                tag,
            });
        }
        Some(document)
    }

    /// The document one folded name reaches, with its key.
    fn get(&self, category: FixCategory, name: &str) -> Option<(&DefinitionKey, &Field)> {
        self.keys
            .get(&Catalog::key(category, name))?
            .iter()
            .find(|key| folds_equal(&key.1, name))
            .and_then(|key| self.raw.get_key_value(key))
    }

    /// Holds `documents` as what the source being folded states, until
    /// [`Self::unstate`].
    fn state<'a>(&mut self, documents: impl IntoIterator<Item = (FixCategory, &'a Field)>) {
        self.source.clear();
        for (category, document) in documents {
            self.source
                .entry(Catalog::key(category, document.name()))
                .or_default()
                .push(document.clone());
        }
    }

    /// Forgets what [`Self::state`] held.
    fn unstate(&mut self) {
        self.source.clear();
    }

    /// The definition an incoming member's reference reaches: what the source
    /// being folded states under that name, else the document held - an
    /// arrival of the source, or a definition no source restates.
    ///
    /// A source's member is read as the source states its target, never as
    /// the dictionary held it before the source's own definition of that name
    /// merged into it - which, definitions folding in name order, the member
    /// would otherwise meet whenever its owner sorts first. A definition
    /// written alone reads what is held, its targets being held already.
    fn incoming(&self, category: FixCategory, name: &str) -> Option<&Field> {
        self.source
            .get(&Catalog::key(category, name))
            .and_then(|stated| {
                stated
                    .iter()
                    .find(|document| folds_equal(document.name(), name))
            })
            .or_else(|| self.get(category, name).map(|(_, document)| document))
    }

    /// Rewrites every member reading a field the fold renamed, so a document
    /// held before the rename reads the field under the identity it holds
    /// now.
    ///
    /// A rename moves no member: the map answers a held identity for every
    /// key it holds, so nothing here is passed over.
    pub(super) fn rename_references(
        &mut self,
        renamed: &HashMap<FixId, Option<(i32, SmolStr)>>,
    ) -> Result<()> {
        if renamed.is_empty() {
            return Ok(());
        }
        let keys: Vec<DefinitionKey> = self.raw.keys().cloned().collect();
        for key in keys {
            let Some(document) = self.raw.get(&key) else {
                continue;
            };
            let rewritten = rename_document(document.clone(), renamed)?;
            if self.raw.get(&key) != Some(&rewritten) {
                self.put(key, rewritten);
            }
        }
        Ok(())
    }

    fn put(&mut self, key: DefinitionKey, document: Field) {
        let held = self.keys.entry(Catalog::key(key.0, &key.1)).or_default();
        if !held.contains(&key) {
            held.push(key.clone());
        }
        let tag = FixField::new(&document)
            .tag()
            .ok()
            .flatten()
            .map(|tag| (tag, self.tags.insert(tag, key.clone())));
        let previous = self.raw.insert(key.clone(), document);
        if self.open > 0 {
            self.journal.push(Replaced {
                key,
                document: previous,
                tag,
            });
        }
    }
}

pub(super) fn validate_name(field: &Field) -> Result<()> {
    validate_definition_name(field.name())
}

/// The one rule a name a store files a document under is held to.
///
/// A named definition and a named [code set](super::codes) are both written
/// as `<name>.json` under their folder and read back by that stem, so one
/// rule answers for both: what a store can file, a checkout can hold, and a
/// reader can key.
pub(super) fn validate_definition_name(name: &str) -> Result<()> {
    if !is_catalog_name(name) {
        return Err(Error::InvalidRecord {
            path: name.into(),
            reason: "expected a nonempty ASCII definition name containing letters, digits, underscore, hyphen, or dot".into(),
        });
    }
    Ok(())
}

/// Whether `name` is one a store files a document under and a reference
/// names a definition by: nonempty ASCII letters, digits, underscore, hyphen
/// and dot, and neither `.` nor `..`.
///
/// The one alphabet every catalog name is held to, a field's reference among
/// them, so what a reader may name is what a reference may point at.
pub(super) fn is_catalog_name(name: &str) -> bool {
    !name.is_empty()
        && !matches!(name, "." | "..")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

/// The catalog name a source's spelling is filed under: lower case, every
/// run of characters outside the [catalog alphabet](is_catalog_name) one
/// `_`, no `_` at either end - `OTC Trade Flags` is `otc_trade_flags`,
/// `(BloombergCustomTag05)` is `bloombergcustomtag05` - or `None` where
/// nothing of it is left.
///
/// What a reader names a field, a group or an occurrence by when a source
/// spells it freely: the spelling itself stays the definition's `display`,
/// and the fold drops only what a name lookup already folds away - case,
/// space, `_` and `-` - and punctuation, so the spelling still reaches the
/// definition wherever it carries no punctuation.
pub(super) fn catalog_name(spelling: &str) -> Option<SmolStr> {
    let mut name = String::with_capacity(spelling.len());
    let mut gap = false;
    for character in spelling.chars() {
        let character = character.to_ascii_lowercase();
        if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
            if gap && !name.is_empty() && !name.ends_with('_') && character != '_' {
                name.push('_');
            }
            gap = false;
            name.push(character);
        } else {
            gap = true;
        }
    }
    let name = name.trim_matches('_');
    is_catalog_name(name).then(|| SmolStr::new(name))
}

/// Drops from a reference occurrence the identity only its target owns.
///
/// A named definition's derived tag is what identifies it in the catalog; a
/// reference to it restates its tree, not its identity, which is why a store
/// writes an occurrence as a bare marker and `store::Resolver::occurrence`
/// expands one without the tag. A caller building the same definition in
/// memory has no compact form to hand over - `validate_references` demands the
/// target's exact datatype, so the occurrence can only be a clone of the
/// stored definition - so the tag is dropped here instead. One shape whatever
/// built it: a hand-built catalog equals the catalog a store reads back.
pub(super) fn canonical_occurrences(mut field: Field, root: bool) -> Result<Field> {
    if !root
        && FixField::new(&field).field_ref().is_none()
        && (FixField::new(&field).group().is_some() || FixField::new(&field).component().is_some())
    {
        field.remove_metadata(super::field::TAG_KEY);
    }
    let dtype = match field.dtype() {
        DataType::Struct(children) => Some(DataType::from(StructType::from_fields(
            children
                .iter()
                .cloned()
                .map(|child| canonical_occurrences(child, false))
                .collect::<Result<Vec<_>>>()?,
        )?)),
        DataType::Serie(_)
        | DataType::LargeSerie(_)
        | DataType::Map(_)
        | DataType::SortedMap(_) => {
            let item = occurrence_of(&field).expect("a group has an occurrence");
            Some(group_dtype(
                &field,
                canonical_occurrences(item.clone(), false)?,
            )?)
        }
        _ => None,
    };
    if let Some(dtype) = dtype {
        field.set_dtype(dtype)?;
    }
    FixFieldMut::new(&mut field).normalize_identifiers()?;
    Ok(field)
}

/// The structure of a named definition, rendered as one text an index can
/// key on: what two grammars declaring one component or one group have to
/// agree on to be declaring one definition.
///
/// A record is its members in order, `(` to `)` with `,` between; a member
/// reading a wire field is `tag:name`, the field's tag and its folded name
/// (`?` for a tag it does not state); a leaf stated inline is `tag=datatype`;
/// a repeating group is its counter, its layout - `*` a serie, `**` a large
/// serie, `#` a map - and the structure of its occurrence,
/// `453*(448:partyid,447:partyidsource)`. A member referencing a component
/// or a group is that definition's structure, read through `lookup`, so a
/// group drawing on a component keys as the component's members whatever
/// the component is called, and a message holding the group keys the group
/// inline where it holds it.
///
/// Nothing else is in it: not a name, a nullability, a display, a
/// description or the sources that contributed a side - each a fact about
/// where a declaration came from or how strictly it was stated, never about
/// what is on the wire - and not the derived tag a definition is stamped
/// with, which is the catalog's identity for its name. Read on a resolved
/// definition and on the compact document a store writes for it alike,
/// since a reference is read through its target either way.
///
/// `memo` holds the structure of every definition a reference reached, by
/// category and folded name, so keying a whole catalog renders each
/// definition's tree once rather than once per definition reading it. One
/// memo per `lookup`, never shared between two: a name two lookups answer
/// with two definitions is two structures.
///
/// # Errors
///
/// Returns the absence of a definition a member references and `lookup`
/// does not answer, and the refusal of a reference graph nested past 64
/// levels, as the resolver refuses one - counted as rendered, so a tree the
/// memo answers is read at the depth it was first rendered at, the resolver
/// being what refuses the depth of a graph the fold assembles.
pub(super) fn structural_key<'a>(
    field: &Field,
    lookup: &impl Fn(FixCategory, &str) -> Option<&'a Field>,
    memo: &mut StructureMemo,
) -> Result<SmolStr> {
    let mut rendered = String::new();
    write_structure(&mut rendered, field, true, lookup, memo, 0)?;
    Ok(SmolStr::new(rendered))
}

/// The [structure](structural_key) of every definition one lookup answered a
/// reference with, by category and folded name.
pub(super) type StructureMemo = HashMap<DefinitionKey, SmolStr>;

fn write_structure<'a>(
    out: &mut String,
    field: &Field,
    root: bool,
    lookup: &impl Fn(FixCategory, &str) -> Option<&'a Field>,
    memo: &mut StructureMemo,
    depth: usize,
) -> Result<()> {
    use std::fmt::Write as _;

    if depth > 64 {
        return Err(Error::InvalidRecord {
            path: field.name().into(),
            reason: "expected an acyclic FIX reference graph nested at most 64 levels".into(),
        });
    }
    let view = FixField::new(field);
    // A definition names its component or its counter on its own root and is
    // still the definition: only a member is a reference.
    if !root && let Some((category, name)) = reference(field) {
        if category == FixCategory::Fields {
            match view.tag()? {
                Some(tag) => write!(out, "{tag}:"),
                None => write!(out, "?:"),
            }
            .expect("a String takes every write");
            out.extend(crate::implementer::folded(name));
            return Ok(());
        }
        let key = (
            category,
            crate::implementer::folded(name).collect::<String>(),
        );
        if let Some(rendered) = memo.get(&key) {
            out.push_str(rendered);
            return Ok(());
        }
        // At the member's own depth: a reference stands where its target's
        // tree stands, and the levels counted are the tree's, as the
        // resolver counts them, so what the catalog resolved keys.
        let target =
            lookup(category, name).ok_or_else(|| Error::absent(category.as_str(), name))?;
        let mut rendered = String::new();
        write_structure(&mut rendered, target, true, lookup, memo, depth)?;
        out.push_str(&rendered);
        memo.insert(key, SmolStr::new(rendered));
        return Ok(());
    }
    match field.dtype() {
        DataType::Struct(children) => {
            out.push('(');
            for (index, child) in children.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_structure(out, child, false, lookup, memo, depth + 1)?;
            }
            out.push(')');
        }
        dtype @ (DataType::Serie(_)
        | DataType::LargeSerie(_)
        | DataType::Map(_)
        | DataType::SortedMap(_)) => {
            let layout = match dtype {
                DataType::Serie(_) => "*",
                DataType::LargeSerie(_) => "**",
                _ => "#",
            };
            match view.counter()? {
                Some(counter) => write!(out, "{counter}{layout}"),
                None => write!(out, "?{layout}"),
            }
            .expect("a String takes every write");
            let item = occurrence_of(field).expect("the variant was just matched");
            write_structure(out, item, false, lookup, memo, depth + 1)?;
        }
        dtype => {
            match view.tag()? {
                Some(tag) => write!(out, "{tag}={dtype}"),
                None => write!(out, "?={dtype}"),
            }
            .expect("a String takes every write");
        }
    }
    Ok(())
}

/// Two definitions of one [structure](structural_key) as one: the held
/// definition - its name, its tag, its own nullability, its display and
/// every other fact of its own - with each member's nullability, at every
/// level, relaxed to the more permissive side and the sources of both sides
/// listed at each level.
///
/// The one fold two declarations of one structure take, whatever names they
/// were declared under: a member one grammar states as required and another
/// does not is nullable, because the dictionary serves both, and every
/// dialect that declared the structure is a source of it. The definition's
/// own nullability is the held one's, as [`merge_root`] keeps it, so a
/// dialect's group folding into one the specification declares rewrites
/// nothing of the store but what the members state.
///
/// Read on resolved definitions and on the compact documents a fold writes
/// alike, so a level one side states where the other references - a group
/// held inline against a member reading a group of that structure, which a
/// document states as the placeholder naming its target - keeps the held
/// side's statement, relaxed and sourced like every level and read no
/// further: what lies beneath a reference is its target's, folded where
/// the target is.
///
/// # Errors
///
/// Returns a refusal naming both where two records are not of one
/// structure, which a caller that keyed them never meets, and the sources
/// write's otherwise.
pub(super) fn fold_alike(held: &Field, incoming: &Field) -> Result<Field> {
    fold_alike_at(held, incoming, true)
}

fn fold_alike_at(held: &Field, incoming: &Field, root: bool) -> Result<Field> {
    let mut merged = held.clone();
    if !root {
        merged.set_nullable(held.is_nullable() || incoming.is_nullable());
    }
    let sources: Vec<&str> = FixField::new(held)
        .sources()
        .chain(FixField::new(incoming).sources())
        .collect();
    if !sources.is_empty() {
        FixFieldMut::new(&mut merged).set_sources(sources)?;
    }
    let dtype = match (held.dtype(), incoming.dtype()) {
        (DataType::Struct(ours), DataType::Struct(theirs)) => {
            if ours.len() != theirs.len() {
                return Err(invalid(
                    incoming,
                    format_args!("the structure of {} ({} members)", held.name(), ours.len()),
                ));
            }
            let members = ours
                .iter()
                .zip(theirs.iter())
                .map(|(held, incoming)| fold_alike_at(held, incoming, false))
                .collect::<Result<Vec<_>>>()?;
            Some(DataType::from(StructType::from_fields(members)?))
        }
        (
            DataType::Serie(_)
            | DataType::LargeSerie(_)
            | DataType::Map(_)
            | DataType::SortedMap(_),
            DataType::Serie(_)
            | DataType::LargeSerie(_)
            | DataType::Map(_)
            | DataType::SortedMap(_),
        ) => {
            let ours = occurrence_of(held).expect("the variant was just matched");
            let theirs = occurrence_of(incoming).expect("the variant was just matched");
            Some(group_dtype(held, fold_alike_at(ours, theirs, false)?)?)
        }
        _ => None,
    };
    if let Some(dtype) = dtype {
        merged.set_dtype(dtype)?;
    }
    Ok(merged)
}

impl FixRegistry {
    /// Borrows the message definition named by an exact wire code, a folded
    /// canonical name, or tag 35's enum alias, in that order.
    ///
    /// A code two messages declare under different names answers the one
    /// named as tag 35's code set names the code, else the first in name
    /// order; the other is reached by its name.
    pub fn get_msgtype(&self, spelling: &str) -> Option<&MsgType> {
        let position = self.catalog.message_position(spelling)?;
        self.catalog.entries[position].field.message()
    }

    /// Borrows a registry-owned message type, raising absence.
    pub fn msgtype(&self, spelling: &str) -> Result<&MsgType> {
        self.get_msgtype(spelling)
            .ok_or_else(|| Error::absent("one FIX message type", format_args!("{spelling:?}")))
    }

    pub(super) fn refresh_msgtype_aliases(&mut self) {
        // The document rather than the field: tag 35 names its set and the
        // dictionary holds it, and the index is over the set's members.
        let codes = self
            .get_field_by_tag(35)
            .and_then(|field| FixField::new(field).codeset())
            .and_then(|name| self.get_codeset(name))
            .map(|set| set.document().to_owned());
        self.catalog.index_message_aliases(codes.as_deref());
    }

    /// Resolves one category's folded name.
    pub fn get_definition(&self, category: FixCategory, name: &str) -> Option<&Field> {
        if category == FixCategory::Fields {
            return self.get_scalar_by_name(name);
        }
        let position = self.catalog.position(category, name)?;
        Some(self.catalog.entries[position].field.as_field())
    }

    /// Resolves a category name, reporting absence with its category.
    pub fn definition(&self, category: FixCategory, name: &str) -> Result<&Field> {
        self.get_definition(category, name)
            .ok_or_else(|| Error::absent(category.as_str(), name))
    }

    /// Iterates one category deterministically without collecting definitions.
    pub fn definitions(&self, category: FixCategory) -> impl Iterator<Item = &Field> {
        self.scalars()
            .take(if category == FixCategory::Fields {
                usize::MAX
            } else {
                0
            })
            .chain(self.catalog.iter(category))
    }

    /// Inserts or replaces a category definition, validating references before mutation.
    /// Case-insensitive input names retain the stored canonical spelling.
    pub fn insert_definition(
        &mut self,
        category: FixCategory,
        mut field: Field,
    ) -> Result<Option<Field>> {
        if category == FixCategory::Fields {
            return self.insert(field);
        }
        if category == FixCategory::Groups
            && let Some(name) = FixField::new(&field).component().map(str::to_owned)
        {
            let component = self.definition(FixCategory::Components, &name)?;
            if let Some(item) = occurrence_of(&field) {
                // Against the canonical shape: the stored component holds
                // no derived tag on its own occurrences, and an item a
                // caller cloned out of the catalog still does.
                let stated = canonical_occurrences(item.clone(), true)?;
                if stated.dtype() != component.dtype() {
                    return Err(invalid(
                        &field,
                        format_args!("component {name:?} datatype {}", component.dtype()),
                    ));
                }
                let mut item = item.clone();
                FixFieldMut::new(&mut item).set_component(&name)?;
                let dtype = group_dtype(&field, item)?;
                field.set_dtype(dtype)?;
            }
        }
        // After the group's item takes its component marker, and before this
        // definition's own tag is derived: an occurrence a caller cloned out of
        // the catalog arrives carrying its target's derived tag, which no
        // stored reference restates and no loaded one carries.
        field = canonical_occurrences(field, true)?;
        // An update keeps the identity the definition already has: a derived
        // tag is stable for the definition, not for the registry it was
        // derived against.
        let held = self
            .get_definition(category, field.name())
            .and_then(|stored| FixField::new(stored).tag().ok().flatten());
        let own = match FixField::new(&field).tag()? {
            // A tag outside the block is nobody's to hold. It is kept as
            // stated so `validate_definition` below refuses it by name rather
            // than this quietly deriving something else over it.
            Some(tag) if !FixId::is_definition_tag(tag) => Some(tag),
            // Its own tag, or one nothing else answers to, is kept: a stored
            // dictionary states the identity and a reader does not overrule it.
            // A tag another definition already holds is not this one's to take,
            // however it arrived - a definition cloned under a second name
            // carries the first one's - so that case derives afresh.
            Some(tag) => (held == Some(tag) || !self.definition_tag_in_use(tag)).then_some(tag),
            None => held,
        };
        let tag = match own {
            Some(tag) => tag,
            None => self.derived_definition_tag(field.name())?,
        };
        FixFieldMut::new(&mut field).set_tag(tag)?;
        self.validate_definition(category, &field)?;
        if let Some(stored) = self.get_definition(category, field.name()) {
            if stored.name().eq_ignore_ascii_case(field.name()) {
                field.set_name(stored.name());
            }
            let refresh = super::registry::metadata_only_change(stored, &field);
            if refresh {
                self.validate_catalog()?;
            }
            let mut staged = self.clone();
            let prior = staged.catalog.insert(category, field)?;
            if refresh {
                staged.refresh_references()?;
            }
            staged.validate_catalog()?;
            *self = staged;
            return Ok(prior);
        }
        // What was answered before this definition landed is forgotten
        // with it: a group widens the root the answers were read against.
        self.forget_answers();
        self.catalog.insert(category, field)
    }

    /// Creates a definition, refusing an existing name or wire identity atomically.
    pub fn create_definition(&mut self, category: FixCategory, field: Field) -> Result<()> {
        let existing = if category == FixCategory::Fields {
            // Creation reserves canonical identities only. Another field's
            // alias may name this spelling until its canonical owner arrives.
            // A tag another field holds under another name is free for this
            // one: the identity is the pair.
            self.canonical_position_by_id(super::registry::canonical_id(&field)?)
                .is_some()
                || self.canonical_position_by_name(field.name()).is_some()
        } else {
            self.get_definition(category, field.name()).is_some()
        };
        if existing {
            // Read through the template: "expected to create a free FIX
            // definition name at ..., got an existing FIX definition".
            return Err(Error::conflict(
                "free FIX definition name",
                "FIX definition",
                field.name(),
            ));
        }
        self.insert_definition(category, field)?;
        Ok(())
    }

    /// Replaces an existing definition and returns its previous value.
    pub fn update_definition(&mut self, category: FixCategory, field: Field) -> Result<Field> {
        let stored = self.definition(category, field.name())?;
        if category == FixCategory::Fields
            && FixField::new(stored).id()? != FixField::new(&field).id()?
        {
            return Err(Error::conflict(
                "the existing FIX identity",
                "a different FIX identity",
                field.name(),
            ));
        }
        let prior = stored.clone();
        self.insert_definition(category, field)?;
        Ok(prior)
    }

    /// Adds a named definition, folding it into the one its name reaches.
    ///
    /// The lenient counterpart of [`Self::create_definition`], which refuses
    /// an existing name, and of [`Self::insert_definition`], which replaces
    /// one wholesale: a definition the dictionary lacks arrives as
    /// `insert_definition` would add it and answers `true`; one it holds is
    /// merged and answers `false`. [`FixCategory::Fields`] redirects to
    /// [`Self::add_field`], whose rules a scalar follows.
    ///
    /// A merge keeps the stored definition's identity, name, tag and every
    /// member it already declares, in its order. An incoming member whose
    /// folded name no stored member carries is appended - for a group, to
    /// the occurrence Struct inside the Serie, whose counter and component
    /// markers stay; when that occurrence is a component's, the members
    /// belong to the component and are appended there. A member both sides
    /// declare is the stored one, one level deep: its own children are not
    /// merged and its datatype never changes, so an incoming member that
    /// disagrees in datatype with the stored one of its name - or restates a
    /// different reference - is refused. A reference and an inline member
    /// agree when the reference's target has the inline datatype: `PartyID`
    /// stated inline as `utf8` restates a reference to the `utf8` field
    /// `PartyID`, and the stored form is what stays. Metadata folds as
    /// [`Self::add_field`] folds it: the incoming side wins a shared key, the
    /// identity keys excepted, and the `FIX:` keys follow
    /// [`FixFieldMut::merge_with`](crate::FixFieldMut::merge_with), which
    /// holds both sides to one message code, counter and component.
    ///
    /// The merge is settled on the documents a store writes, references
    /// folded to their markers, and the catalog is resolved from them again:
    /// every message and component referencing the definition sees the
    /// appended members without holding a copy of anything.
    ///
    /// ```
    /// use yggdryl::{DataType, FixFieldMut, FixRegistry, StructType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut party_id = DataType::utf8().nullable_field("PartyID");
    /// FixFieldMut::new(&mut party_id).set_tag(448)?;
    /// let mut registry = FixRegistry::from_fields([party_id.clone()])?;
    /// FixFieldMut::new(&mut party_id).set_field_ref("PartyID")?;
    /// let party = DataType::from(StructType::from_fields([party_id])?).required_field("Party");
    /// registry.insert(party)?;
    /// // A message restates the component through a reference to it.
    /// let mut party = registry.field_by_name("Party")?.clone();
    /// FixFieldMut::new(&mut party).set_component("Party")?;
    /// let mut order = DataType::from(StructType::from_fields([party])?).required_field("Order");
    /// FixFieldMut::new(&mut order).set_msgtype("D")?;
    /// registry.insert(order)?;
    ///
    /// // Extending the component is one call, and the message sees the member.
    /// let mut extended = registry.field_by_name("Party")?.clone();
    /// let note = DataType::utf8().nullable_field("PartyNote");
    /// let members = extended.fields().iter().cloned().chain([note]);
    /// extended.set_dtype(DataType::from(StructType::from_fields(members)?))?;
    /// assert!(!registry.add_field(extended)?, "merged");
    /// let member = yggdryl::FieldPath::from_str("Order.Party.PartyNote")?;
    /// assert_eq!(registry.field_by_path(&member)?.dtype(), &DataType::utf8());
    /// assert_eq!(registry.field_by_name("Party")?.field_len(), 2);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns what [`Self::insert_definition`] returns for a definition that
    /// arrives; for a merge, [`Error::InvalidRecord`] naming the member whose
    /// datatype or reference disagrees with the stored one, the conflict
    /// [`FixFieldMut::merge_with`](crate::FixFieldMut::merge_with) raises for
    /// a second message code, counter or component, and whatever resolving
    /// or validating the merged catalog raises. One staged mutation: a refusal
    /// leaves the registry untouched.
    pub(super) fn add_definition(&mut self, category: FixCategory, field: Field) -> Result<bool> {
        if category == FixCategory::Fields {
            return self.add_field(field);
        }
        let mut staged = self.clone();
        let added = staged.fold_definition(category, field)?;
        *self = staged;
        Ok(added)
    }

    /// Takes back the named definitions a walk wrote, newest first, without
    /// proving the catalog again: what it held before them was proven, and
    /// nothing else reads a definition the same walk wrote and abandoned.
    pub(super) fn forget_definitions(&mut self, written: &[(FixCategory, String)]) {
        let mut forgot = false;
        for (category, name) in written.iter().rev() {
            if let Some(position) = self.catalog.position(*category, name) {
                self.catalog.remove(position);
                forgot = true;
            }
        }
        if forgot {
            self.refresh_msgtype_aliases();
        }
    }

    /// Takes back the named definitions among `written` that nothing in the
    /// catalog reads any more, newest first, so a definition another read
    /// was folded into leaves no copy of itself behind.
    pub(super) fn forget_unread(&mut self, written: &[(FixCategory, String)]) {
        for (category, name) in written.iter().rev() {
            let read = self
                .catalog
                .all()
                .any(|entry| reads_definition(entry.field.as_field(), *category, name, 0));
            if !read && let Some(position) = self.catalog.position(*category, name) {
                self.catalog.remove(position);
            }
        }
    }

    /// The fold of one named definition, without the staging.
    ///
    /// [`Self::insert`] over a nested field is this plus the copy that makes
    /// it one mutation, and a fold already holding a staged dictionary calls
    /// this so the copy is paid once.
    pub(super) fn fold_definition(&mut self, category: FixCategory, field: Field) -> Result<bool> {
        self.fold_definition_into(category, field, None)
    }

    /// [`Self::fold_definition`], passing what disagrees with the held
    /// definition over into `drops` - the way a fold with another dictionary
    /// does - rather than refusing the definition whole, where `drops` is
    /// given.
    ///
    /// Where `drops` is given, the fold is followed by the pass a fold with
    /// another dictionary ends with ([`fold_alike_definitions`]): a held
    /// definition the fold widened into the structure of another held one is
    /// one definition, so a CBlock binding one wire type twice holds no two
    /// definitions of one structure whichever message it binds first.
    pub(super) fn fold_definition_into(
        &mut self,
        category: FixCategory,
        field: Field,
        drops: Option<&mut Vec<FixDrop>>,
    ) -> Result<bool> {
        validate_name(&field)?;
        check_shape(category, &field)?;
        if self.get_definition(category, field.name()).is_none() {
            self.insert_definition(category, field)?;
            return Ok(true);
        }
        // Resolve raw identifier spellings before compacting references,
        // whose Null placeholders no longer carry their scalar tags.
        let field = canonical_occurrences(field, true)?;
        let mut documents = Documents::from_registry(self)?;
        let reading = drops.is_some();
        let before = if reading {
            structures_of(&documents)
        } else {
            HashMap::new()
        };
        self.fold_document(&mut documents, category, &field, &mut { drops })?;
        if reading {
            fold_alike_definitions(&mut documents, &before, &[(category, field, false)])?;
        }
        self.resolve_catalog(documents.raw)?;
        self.validate_catalog()?;
        Ok(false)
    }

    /// Folds one incoming definition over the documents.
    ///
    /// A document nothing stored answers to is added as the store would read
    /// it; one a stored document answers to, by folded name, is merged into
    /// that document under the stored key.
    ///
    /// `drops` is the one difference between the two folds that call this.
    /// A definition written on its own is refused whole when it disagrees
    /// with what is held; a definition arriving with another dictionary
    /// passes the disagreeing part over, into `drops`, and folds the rest.
    fn fold_document(
        &self,
        documents: &mut Documents,
        category: FixCategory,
        incoming: &Field,
        drops: &mut Option<&mut Vec<FixDrop>>,
    ) -> Result<()> {
        let incoming = compact(incoming.clone(), true)?;
        let Some((key, stored)) = documents.get(category, incoming.name()) else {
            let key = (category, incoming.name().to_owned());
            let incoming = self.untangled(documents, &key, incoming)?;
            documents.put(key, incoming);
            return Ok(());
        };
        let (key, stored) = (key.clone(), stored.clone());
        if let Some(merged) =
            self.merge_documents(documents, category, &stored, &incoming, drops)?
        {
            documents.put(key, merged);
        }
        Ok(())
    }

    /// One level of `incoming` folded into `stored`, both compact: `None`
    /// where the incoming definition was passed over whole, its own
    /// identity disagreeing with the stored one's.
    fn merge_documents(
        &self,
        documents: &mut Documents,
        category: FixCategory,
        stored: &Field,
        incoming: &Field,
        drops: &mut Option<&mut Vec<FixDrop>>,
    ) -> Result<Option<Field>> {
        let reconciled;
        let incoming = if category == FixCategory::Groups && drops.is_some() {
            reconciled = self.reconciled_group(documents, stored, incoming, drops)?;
            &reconciled
        } else {
            incoming
        };
        let mut merged = match merge_root(stored, incoming) {
            Ok(merged) => merged,
            Err(error) => {
                passed_over(drops, incoming, error)?;
                return Ok(None);
            }
        };
        let dtype = if category == FixCategory::Groups {
            let held =
                occurrence_of(stored).ok_or_else(|| invalid(stored, "a group occurrence"))?;
            let item =
                occurrence_of(incoming).ok_or_else(|| invalid(incoming, "a group occurrence"))?;
            let occurrence = match (reference(held), reference(item)) {
                // The stored occurrence is a component's, so what the incoming
                // one adds belongs to that component: its members are folded
                // there - only its members, since the occurrence's own root
                // is the group's business and is merged above - and the
                // group, with every other reference, reads them back from
                // there once the documents resolve.
                (Some((FixCategory::Components, name)), None) => {
                    let name = name.to_owned();
                    self.fold_members(documents, FixCategory::Components, &name, item, drops)?;
                    held.clone()
                }
                (None, None) => {
                    let mut occurrence = match merge_root(held, item) {
                        Ok(occurrence) => occurrence,
                        Err(error) => {
                            passed_over(drops, incoming, error)?;
                            return Ok(None);
                        }
                    };
                    let members = self.merge_children(
                        documents,
                        stored.name(),
                        held.fields(),
                        item.fields(),
                        drops,
                    )?;
                    set_merged_dtype(
                        &mut occurrence,
                        DataType::from(StructType::from_fields(members)?),
                    )?;
                    occurrence
                }
                _ => {
                    if let Some(error) = self.disagreement(documents, stored.name(), held, item)? {
                        self.reconcile(documents, held, item, error, drops, 0)?;
                    }
                    held.clone()
                }
            };
            group_dtype(stored, occurrence)?
        } else {
            DataType::from(StructType::from_fields(self.merge_children(
                documents,
                stored.name(),
                stored.fields(),
                incoming.fields(),
                drops,
            )?)?)
        };
        set_merged_dtype(&mut merged, dtype)?;
        Ok(Some(merged))
    }

    /// A group arriving on the counter the held group of its name counts,
    /// with its occurrences drawn from another component: the one repeating
    /// group read two ways, as a dialect reads it when it split the
    /// component for one message. The incoming component's members fold into
    /// the held component, and the group arrives drawing from that one.
    /// Anything else arrives as it came, for the root merge to settle.
    fn reconciled_group(
        &self,
        documents: &mut Documents,
        stored: &Field,
        incoming: &Field,
        drops: &mut Option<&mut Vec<FixDrop>>,
    ) -> Result<Field> {
        let (Some(held), Some(other)) = (
            FixField::new(stored).component(),
            FixField::new(incoming).component(),
        ) else {
            return Ok(incoming.clone());
        };
        if folds_equal(held, other)
            || FixField::new(stored).counter()? != FixField::new(incoming).counter()?
        {
            return Ok(incoming.clone());
        }
        let held = held.to_owned();
        if !self.fold_members_at(
            documents,
            FixCategory::Components,
            &held,
            incoming,
            drops,
            0,
        )? {
            return Ok(incoming.clone());
        }
        with_component(incoming, &held)
    }

    /// The stored children, then every incoming child no stored one answers
    /// to - by name, or, in a fold with another dictionary, by the counter
    /// of the group it reads, which a stored member reading a group on that
    /// counter answers for.
    ///
    /// Order is the stored definition's, because a message's members are read
    /// positionally by everything that walks it; what arrives is appended in
    /// the order it was declared.
    fn merge_children(
        &self,
        documents: &mut Documents,
        owner: &str,
        stored: &[Field],
        incoming: &[Field],
        drops: &mut Option<&mut Vec<FixDrop>>,
    ) -> Result<Vec<Field>> {
        self.merge_children_at(documents, owner, stored, incoming, drops, 0)
    }

    fn merge_children_at(
        &self,
        documents: &mut Documents,
        owner: &str,
        stored: &[Field],
        incoming: &[Field],
        drops: &mut Option<&mut Vec<FixDrop>>,
        depth: usize,
    ) -> Result<Vec<Field>> {
        let mut merged: Vec<Field> = stored.to_vec();
        for child in incoming {
            match merged
                .iter()
                .find(|held| folds_equal(held.name(), child.name()))
                .cloned()
            {
                Some(held) => {
                    if let Some(error) = self.disagreement(documents, owner, &held, child)? {
                        // A member is the field it reads before the name it
                        // carries. An incoming member reading a field under
                        // a name a stored member reads another field by - a
                        // spelling two tags share, numbered on one side and
                        // not the other - is the stored member reading that
                        // field where one does, and a member of its own
                        // beside the stored ones otherwise, since two tags
                        // are two tags on the wire. Only a fold with another
                        // dictionary reads it so, as `beside` does: a
                        // definition written alone is refused as before.
                        if drops.is_some()
                            && let (Some(reads), Some(_)) =
                                (field_reference(child), field_reference(&held))
                        {
                            if !merged.iter().any(|standing| {
                                field_reference(standing)
                                    .is_some_and(|name| folds_equal(name, reads))
                            }) {
                                push_member(&mut merged, child.clone());
                            }
                            continue;
                        }
                        match self.beside(documents, &held, child, drops)? {
                            // One member per counter: one already reading a
                            // group on this counter - a fold placed it beside
                            // before, or the stored definition declares it -
                            // is the member this reading folds into, and only
                            // a counter no member reads takes one of its own.
                            Some((beside, counter)) => match merged
                                .iter()
                                .find(|standing| {
                                    group_counter(documents, standing) == Some(counter)
                                })
                                .cloned()
                            {
                                Some(standing) => {
                                    if let Some(error) =
                                        self.disagreement(documents, owner, &standing, &beside)?
                                    {
                                        self.reconcile(
                                            documents, &standing, &beside, error, drops, depth,
                                        )?;
                                    }
                                }
                                None => push_member(&mut merged, beside),
                            },
                            None => self.reconcile(documents, &held, child, error, drops, depth)?,
                        }
                    }
                }
                None => {
                    // One member per counter, here too: an incoming member no
                    // stored member answers to by name, reading a group on a
                    // counter a member already reads, is that member read
                    // another way - a dialect naming the group after its own
                    // `rg-name` - and folds into it rather than standing
                    // beside it as a second member of one counter. Only a
                    // fold with another dictionary reads it so, as `beside`
                    // does: a definition written alone appends what it states.
                    let standing = match (drops.is_some(), group_counter(documents, child)) {
                        (true, Some(counter)) => merged
                            .iter()
                            .find(|standing| group_counter(documents, standing) == Some(counter))
                            .cloned(),
                        _ => None,
                    };
                    match standing {
                        Some(standing) => {
                            if let Some(error) =
                                self.disagreement(documents, owner, &standing, child)?
                            {
                                self.reconcile(documents, &standing, child, error, drops, depth)?;
                            }
                        }
                        None => merged.push(child.clone()),
                    }
                }
            }
        }
        Ok(merged)
    }

    /// An incoming member reading a group on another counter than the group
    /// the stored member of its name reads, as a member of its own beside it,
    /// named for its counter - `dealers_7101` beside `dealers` - or `None`
    /// for any other disagreement.
    ///
    /// Two counters are two tags on the wire, so the one message carries both
    /// groups and a venue's occurrences land in the member its counter opens;
    /// a member read two ways under one name is still the only member the
    /// stored definition has for it. Only a fold with another dictionary
    /// places one beside: a definition written alone is refused as before.
    fn beside(
        &self,
        documents: &Documents,
        held: &Field,
        child: &Field,
        drops: &Option<&mut Vec<FixDrop>>,
    ) -> Result<Option<(Field, i32)>> {
        if drops.is_none() {
            return Ok(None);
        }
        let (Some(stored), Some(incoming)) = (
            group_counter(documents, held),
            group_counter(documents, child),
        ) else {
            return Ok(None);
        };
        if stored == incoming {
            return Ok(None);
        }
        let mut beside = child.clone();
        beside.set_name(format!("{}_{incoming}", child.name()));
        Ok(Some((beside, incoming)))
    }

    /// Settles one member the stored and the incoming definition declare
    /// differently, the stored member staying where it is.
    ///
    /// Written alone, a definition is refused. Folded with another
    /// dictionary, two references to two groups or two components are one
    /// member read two ways - a dialect splits a group it declares
    /// differently for one message, and names the split after that message -
    /// so what the incoming target declares folds into the target the stored
    /// member already reads, under these same rules. Anything else is passed
    /// over into `drops`: a reference against an inline child, a group
    /// against a component, two groups on two counters - a field against a
    /// field of its name is read by the field before this is asked, and
    /// stands beside.
    fn reconcile(
        &self,
        documents: &mut Documents,
        held: &Field,
        child: &Field,
        error: Error,
        drops: &mut Option<&mut Vec<FixDrop>>,
        depth: usize,
    ) -> Result<()> {
        if drops.is_none() {
            return Err(error);
        }
        if !self.fold_referenced(documents, held, child, drops, depth)? {
            passed_over(drops, child, error)?;
        }
        Ok(())
    }

    /// Folds the definition `child` references into the one `held` does, both
    /// of one nested category: whether it could. `child`'s target is read as
    /// the source states it ([`Documents::incoming`]), `held`'s as held.
    ///
    /// Only the members fold: the target's own identity - its name, its tag,
    /// its counter - is the stored one's, and an incoming group on another
    /// counter is another group, which is not folded. Bounded by the depth a
    /// reference graph may reach, so a fold that would chase its own tail is
    /// passed over rather than followed.
    fn fold_referenced(
        &self,
        documents: &mut Documents,
        held: &Field,
        child: &Field,
        drops: &mut Option<&mut Vec<FixDrop>>,
        depth: usize,
    ) -> Result<bool> {
        let (Some((category, stored)), Some((other, incoming))) =
            (reference(held), reference(child))
        else {
            return Ok(false);
        };
        if category != other || category == FixCategory::Fields || depth >= 64 {
            return Ok(false);
        }
        let Some(target) = documents.incoming(category, incoming) else {
            return Ok(false);
        };
        let target = target.clone();
        let stored = stored.to_owned();
        if category == FixCategory::Components {
            return self.fold_members_at(documents, category, &stored, &target, drops, depth + 1);
        }
        let Some((_, group)) = documents.get(category, &stored) else {
            return Ok(false);
        };
        let group = group.clone();
        if FixField::new(&group).counter()? != FixField::new(&target).counter()? {
            return Ok(false);
        }
        let (Some(held), Some(item)) = (occurrence_of(&group), occurrence_of(&target)) else {
            return Ok(false);
        };
        match reference(held) {
            Some((FixCategory::Components, _)) if reference(item).is_some() => {
                let (held, item) = (held.clone(), item.clone());
                self.fold_referenced(documents, &held, &item, drops, depth + 1)
            }
            Some((FixCategory::Components, name)) => {
                let name = name.to_owned();
                let item = item.clone();
                self.fold_members_at(
                    documents,
                    FixCategory::Components,
                    &name,
                    &item,
                    drops,
                    depth + 1,
                )
            }
            Some(_) => Ok(false),
            None => self.fold_members_at(documents, category, &stored, &target, drops, depth + 1),
        }
    }

    /// Folds the members `incoming` declares - its own children, or the
    /// children of the component it references, as the source states it
    /// ([`Documents::incoming`]) - into the stored definition `name`: a
    /// component's own children, or an inline group occurrence's.
    fn fold_members(
        &self,
        documents: &mut Documents,
        category: FixCategory,
        name: &str,
        incoming: &Field,
        drops: &mut Option<&mut Vec<FixDrop>>,
    ) -> Result<()> {
        if self.fold_members_at(documents, category, name, incoming, drops, 0)? {
            return Ok(());
        }
        Err(Error::absent(category.as_str(), name))
    }

    fn fold_members_at(
        &self,
        documents: &mut Documents,
        category: FixCategory,
        name: &str,
        incoming: &Field,
        drops: &mut Option<&mut Vec<FixDrop>>,
        depth: usize,
    ) -> Result<bool> {
        let Some((key, stored)) = documents.get(category, name) else {
            return Ok(false);
        };
        let (key, mut stored) = (key.clone(), stored.clone());
        // What the incoming side declares: an inline struct's children, a
        // group's inline occurrence's, or the component a reference names.
        let member = occurrence_of(incoming).unwrap_or(incoming);
        let members = match reference(member) {
            Some((FixCategory::Components, component)) => {
                match documents.incoming(FixCategory::Components, component) {
                    Some(target) => target.fields().to_vec(),
                    None => return Ok(false),
                }
            }
            Some(_) => return Ok(false),
            None => member.fields().to_vec(),
        };
        if category == FixCategory::Groups {
            let Some(held) = occurrence_of(&stored).cloned() else {
                return Ok(false);
            };
            let mut occurrence = held.clone();
            let merged = self.merge_children_at(
                documents,
                stored.name(),
                held.fields(),
                &members,
                drops,
                depth,
            )?;
            set_merged_dtype(
                &mut occurrence,
                DataType::from(StructType::from_fields(merged)?),
            )?;
            let dtype = group_dtype(&stored, occurrence)?;
            set_merged_dtype(&mut stored, dtype)?;
        } else {
            let merged = self.merge_children_at(
                documents,
                stored.name(),
                stored.fields(),
                &members,
                drops,
                depth,
            )?;
            set_merged_dtype(
                &mut stored,
                DataType::from(StructType::from_fields(merged)?),
            )?;
        }
        documents.put(key, stored);
        Ok(true)
    }

    /// Whether an incoming child restates the stored one its name folds onto:
    /// `None` where it does, else the refusal naming both sides.
    ///
    /// Two references agree on the target they name, never on the expansion
    /// either side happens to hold, because two registries expand one
    /// component differently exactly when one of them extended it. Two
    /// inline children agree on their datatype. A reference on one side and
    /// an inline child on the other agree when the target's datatype is the
    /// inline one - a dictionary built in memory states `PartyID` inline
    /// where a loaded one references it, and both describe tag 448 - and the
    /// stored side is what is kept, whichever form it has. One level and
    /// nothing else: a child both sides declare is the stored one, so what
    /// its own children say is not this merge's to read.
    fn disagreement(
        &self,
        documents: &Documents,
        owner: &str,
        held: &Field,
        child: &Field,
    ) -> Result<Option<Error>> {
        // Resolved only where one side is a reference and the other is not:
        // two references agree or disagree on what they name, and resolving
        // them would ask for a target that may be arriving in this very fold.
        let (mut held_target, mut child_target) = (None, None);
        let same = match (reference(held), reference(child)) {
            (Some((category, name)), Some((other, spelling))) => {
                category == other && folds_equal(name, spelling)
            }
            (None, None) => held.dtype() == child.dtype(),
            (Some(_), None) => {
                held_target = Some(self.referenced_dtype(documents, held)?);
                held_target.as_ref() == Some(child.dtype())
            }
            (None, Some(_)) => {
                child_target = Some(self.referenced_dtype(documents, child)?);
                child_target.as_ref() == Some(held.dtype())
            }
        };
        if same {
            return Ok(None);
        }
        Ok(Some(Error::InvalidRecord {
            path: format_smolstr!("{owner}.{}", held.name()),
            reason: crate::implementer::expected_got(
                format_args!("{} stored for it", describe(held, held_target.as_ref())),
                describe(child, child_target.as_ref()),
            ),
        }))
    }

    /// The datatype the reference `field` carries resolves to, in the compact
    /// shape the documents hold: a field's from this registry, a named
    /// definition's from the documents being folded, so a definition
    /// extended earlier in the same fold answers extended.
    fn referenced_dtype(&self, documents: &Documents, field: &Field) -> Result<DataType> {
        let (category, name) = reference(field)
            .ok_or_else(|| Error::absent("field, component, or group reference", field.name()))?;
        if category == FixCategory::Fields {
            return Ok(self.definition(category, name)?.dtype().clone());
        }
        documents
            .get(category, name)
            .map(|(_, document)| document.dtype().clone())
            .ok_or_else(|| Error::absent(category.as_str(), name))
    }

    /// Removes a definition only when every remaining reference stays valid.
    pub fn remove_definition(
        &mut self,
        category: FixCategory,
        name: &str,
    ) -> Result<Option<Field>> {
        let Some(field) = self.get_definition(category, name) else {
            return Ok(None);
        };
        let name = field.name().to_owned();
        let mut staged = self.clone();
        let removed = if category == FixCategory::Fields {
            let id = super::registry::canonical_id(field)?;
            staged.remove_resolved(id)
        } else {
            let position = staged
                .catalog
                .position(category, &name)
                .ok_or_else(|| Error::absent(category.as_str(), &name))?;
            Some(staged.catalog.remove(position))
        };
        staged.validate_catalog()?;
        staged.refresh_msgtype_aliases();
        *self = staged;
        Ok(removed)
    }

    /// The unique group the field `tag` names opens; two groups on one
    /// counter name nothing.
    ///
    /// `tag` is the counter's, not the group's own: a group is a catalog
    /// definition and the field counting it is a scalar, so this is the one
    /// lookup that crosses the two. [`Self::get_field_by_tag`] answers the
    /// counter itself off the same key.
    pub(super) fn get_group_by_tag(&self, tag: i32) -> Option<&Field> {
        let position = self.catalog.counters.get(&tag).copied().flatten()?;
        Some(self.catalog.entries[position].field.as_field())
    }

    /// Whether `tag` counts a repeating group of the dictionary - one group
    /// or several, so a counter two groups share answers too.
    pub(super) fn is_counter_tag(&self, tag: i32) -> bool {
        self.catalog.counters.contains_key(&tag)
    }

    pub(super) fn get_group_plan_by_tag(&self, tag: i32) -> Option<&GroupPlan> {
        let position = self.catalog.counters.get(&tag).copied().flatten()?;
        match &self.catalog.entries[position].field {
            DefinitionField::Group(field, plan)
                if !matches!(field.dtype(), DataType::Map(_) | DataType::SortedMap(_)) =>
            {
                Some(plan)
            }
            _ => None,
        }
    }

    /// The unique group the counter `tag` names opens, reporting absence or
    /// ambiguity.
    pub(super) fn group_by_tag(&self, tag: i32) -> Result<&Field> {
        self.get_group_by_tag(tag)
            .ok_or_else(|| Error::absent("one unambiguous FIX group", tag))
    }

    /// Whether anything in this registry already answers to `tag`.
    ///
    /// A published tag never reaches the derived block - the crate's own
    /// stop at [`crate::CRATE_TAG_MAX`] - so the only thing that can occupy a
    /// slot is another derived definition. The scalar index is asked anyway,
    /// because a dictionary read from a store is whatever that store held.
    fn definition_tag_in_use(&self, tag: i32) -> bool {
        if self.get_field_by_tag(tag).is_some() {
            return true;
        }
        [FixCategory::Components, FixCategory::Groups]
            .into_iter()
            .any(|category| {
                self.catalog
                    .iter(category)
                    .any(|held| FixField::new(held).tag().ok().flatten() == Some(tag))
            })
    }

    /// The tag a named definition takes, derived from its name.
    ///
    /// XXH32 of the name places it in the derived block; a slot already taken
    /// is stepped past, wrapping, until a free one is found. Probing rather
    /// than refusing means a name is always registrable, at the cost of a tag
    /// that depends on what was registered before it - which is why the tag
    /// is stored on the definition rather than recomputed on every read.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when every slot in the block is
    /// taken, which needs a million definitions in one registry.
    pub(super) fn derived_definition_tag(&self, name: &str) -> Result<i32> {
        derived_tag(name, |tag| self.definition_tag_in_use(tag))
    }

    /// `incoming` under a derived tag nothing else in the fold holds.
    ///
    /// A definition arriving with another dictionary carries the tag that
    /// dictionary derived for it, probed against what *it* held; here that
    /// slot can be another definition's, and two definitions on one tag is a
    /// catalog defect. So a colliding slot is derived again, the same probe
    /// against what this fold holds - its fields, and every document stored
    /// or already arrived. Nothing restates a definition's tag: a reference
    /// names its target, so the move is seen by nothing but the target.
    fn untangled(
        &self,
        documents: &Documents,
        key: &DefinitionKey,
        mut incoming: Field,
    ) -> Result<Field> {
        let taken = |tag: i32| {
            documents.tags.get(&tag).is_some_and(|held| held != key)
                || self.get_field_by_tag(tag).is_some()
        };
        let Some(tag) = FixField::new(&incoming).tag()? else {
            return Ok(incoming);
        };
        if !FixId::is_definition_tag(tag) || !taken(tag) {
            return Ok(incoming);
        }
        let tag = derived_tag(incoming.name(), taken)?;
        FixFieldMut::new(&mut incoming).set_tag(tag)?;
        Ok(incoming)
    }

    pub(super) fn validate_definition(&self, category: FixCategory, field: &Field) -> Result<()> {
        // Even a declaration whose category does not consume the counter must
        // not publish malformed protocol metadata. Groups reuse this reading.
        // The alternate names are read infallibly everywhere else, so this is
        // where a text the read would walk as nothing is refused.
        FixField::new(field).validate_names()?;
        FixField::new(field).validate_sources()?;
        FixField::new(field).validate_parents()?;
        // So is an identifier-map document; a role names a `Parties`
        // occurrence, whose `PartyID(448)` is the one member it reads.
        for source in FixField::new(field).idmap() {
            if source?.role().is_some()
                && FixField::new(field).tag()? != Some(super::identity::PARTYID_TAG)
            {
                return Err(Error::InvalidMetadataValue {
                    key: "FIX:idmap".into(),
                    reason: format_smolstr!(
                        "expected a role only on PartyID(448), {} states one",
                        field.name()
                    ),
                });
            }
        }
        let counter = FixField::new(field).counter()?;
        let map_group = category == FixCategory::Groups
            && matches!(field.dtype(), DataType::Map(_) | DataType::SortedMap(_));
        if map_group {
            let tag = FixField::new(field)
                .tag()?
                .ok_or_else(|| Error::absent("FIX:tag", field.name()))?;
            let counter = counter.ok_or_else(|| Error::absent("FIX:counter", field.name()))?;
            if !super::is_crate_tag(tag) || counter != tag {
                return Err(Error::InvalidRecord {
                    path: field.name().into(),
                    reason: crate::implementer::expected_got(
                        "equal fix:tag and fix:counter in the crate's reserved range",
                        format_args!("FIX:tag={tag}, fix:counter={counter}"),
                    ),
                });
            }
            if let Some(scalar) = self
                .get_scalar_by_name(field.name())
                .filter(|scalar| folds_equal(scalar.name(), field.name()))
            {
                return Err(Error::conflict(
                    "a Map group name free of canonical scalar names",
                    "scalar field",
                    format_args!(
                        "{}: canonical name belongs to {}",
                        field.name(),
                        scalar.name()
                    ),
                ));
            }
            if let Some(scalar) = self.get_scalar_by_tag(tag) {
                return Err(Error::conflict(
                    "a Map group counter free of scalar fields",
                    "scalar field",
                    format_args!("{}: tag {tag} belongs to {}", field.name(), scalar.name()),
                ));
            }
            if let Some(group) = self.catalog.iter(FixCategory::Groups).find(|group| {
                FixField::new(group).counter().ok().flatten() == Some(tag)
                    && !folds_equal(group.name(), field.name())
            }) {
                return Err(Error::conflict(
                    "one Map group per counter",
                    "another group",
                    format_args!(
                        "{}: counter {tag} belongs to {}",
                        field.name(),
                        group.name()
                    ),
                ));
            }
        } else if category == FixCategory::Fields {
            if let Some(group) = self
                .get_definition(FixCategory::Groups, field.name())
                .filter(|group| matches!(group.dtype(), DataType::Map(_) | DataType::SortedMap(_)))
            {
                return Err(Error::conflict(
                    "a scalar field name free of canonical Map group names",
                    "Map group",
                    format_args!(
                        "{}: canonical name belongs to {}",
                        field.name(),
                        group.name()
                    ),
                ));
            }
            if let Some(group) = FixField::new(field)
                .tag()?
                .and_then(|tag| self.get_group_by_tag(tag))
                .filter(|group| matches!(group.dtype(), DataType::Map(_) | DataType::SortedMap(_)))
            {
                return Err(Error::conflict(
                    "a scalar field tag free of Map group counters",
                    "Map group",
                    format_args!("{}: tag belongs to {}", field.name(), group.name()),
                ));
            }
        }
        if category != FixCategory::Fields {
            validate_name(field)?;
            if let Some(tag) = FixField::new(field).tag()? {
                // A definition nobody published a tag for is identified by a
                // derived one, which is what the block above `CRATE_TAG_MAX`
                // is for. This crate's own definitions are the exception: a
                // crate tag is reserved, unique and already the identity the
                // fixed row reaches the column by, so `metadata` answers
                // to its crate tag rather than to a second identity nothing
                // else spells.
                if !map_group && !FixId::is_definition_tag(tag) && !super::is_crate_tag(tag) {
                    return Err(Error::InvalidRecord {
                        path: field.name().into(),
                        reason: crate::implementer::expected_got(
                            "a named FIX definition's derived tag, or one of this crate's own",
                            format_args!(
                                "tag {tag} outside [{}, {}) and [{}, {}]",
                                FixId::DEFINITION_TAG_MIN,
                                FixId::DEFINITION_TAG_MAX,
                                super::CRATE_TAG_MIN,
                                super::CRATE_TAG_MAX,
                            ),
                        ),
                    });
                }
            }
        }
        check_shape(category, field)?;
        if category == FixCategory::Groups {
            let tag = counter.ok_or_else(|| Error::absent("FIX:counter", field.name()))?;
            if !map_group {
                let counter = self.field_by_tag(tag)?;
                if counter.dtype() != &DataType::Int32 {
                    return Err(invalid(counter, "an int32 repeating-group counter"));
                }
            }
            if let Some(component) = FixField::new(field).component()
                && let Some(item) = occurrence_of(field)
                && !FixField::new(item)
                    .component()
                    .is_some_and(|name| folds_equal(name, component))
            {
                return Err(Error::conflict(
                    "the group's component reference on its item",
                    "a different or absent item reference",
                    field.name(),
                ));
            }
        }
        self.validate_references(field, 0)
    }

    pub(super) fn validate_references(&self, field: &Field, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err(invalid(field, "FIX references nested at most 64 levels"));
        }
        let view = FixField::new(field);
        view.compiled_identifier_positions()?;
        // A code set is a reference like any other: the name is one a store
        // can file, and the dictionary has to hold the set it names, or the
        // field reads by a vocabulary nothing states.
        if let Some(name) = view.codeset() {
            validate_definition_name(name)?;
            if self.get_codeset(name).is_none() {
                return Err(Error::absent("codesets", name));
            }
        }
        if view.field_ref().is_some() && (view.component().is_some() || view.group().is_some()) {
            return Err(invalid(
                field,
                "one FIX field reference without a component or group reference",
            ));
        }
        let reference = view
            .field_ref()
            .map(|name| (FixCategory::Fields, name))
            .or_else(|| view.group().map(|name| (FixCategory::Groups, name)))
            .or_else(|| view.component().map(|name| (FixCategory::Components, name)));
        if let Some((category, name)) = reference {
            let target = self.definition(category, name)?;
            let occurrence = if category == FixCategory::Components {
                occurrence_of(field).unwrap_or(field)
            } else {
                field
            };
            if !restates_datatype(occurrence.dtype(), target.dtype()) {
                return Err(invalid(
                    field,
                    format_args!(
                        "resolved {category} reference {name:?} datatype {}",
                        target.dtype()
                    ),
                ));
            }
            // An occurrence is named by the message that holds it - a
            // duplicate constraint or a contended spelling renames it - so
            // its own identity is the message's business; the tag it carries
            // is the target's, and that is what a reference restates.
            if category == FixCategory::Fields && view.tag()? != FixField::new(target).tag()? {
                return Err(Error::conflict(
                    "the referenced FIX field's tag",
                    "a different tag",
                    field.name(),
                ));
            }
            let marker = match category {
                FixCategory::Fields => "FIX:field",
                FixCategory::Groups => "FIX:group",
                _ => "FIX:component",
            };
            // A named definition carries a derived tag of its own, which is
            // its identity in the catalog rather than anything a reference
            // restates - so an occurrence never carries it and it is not part
            // of what "unchanged" means here. A field reference does carry its
            // target's tag, and the identity check above already proved it.
            let carried = |(key, _): &(&str, &str)| {
                *key != marker && (category == FixCategory::Fields || *key != super::field::TAG_KEY)
            };
            if !occurrence
                .as_metadata()
                .iter()
                .filter(carried)
                .eq(target.as_metadata().iter().filter(carried))
            {
                return Err(Error::conflict(
                    "referenced metadata unchanged apart from its reference marker",
                    "an occurrence metadata override or stale target metadata",
                    field.name(),
                ));
            }
            // The exact datatype comparison proved the complete target tree.
            // Each catalog target is validated separately; visiting its expanded
            // occurrence again would turn a shared graph into repeated walks.
            return Ok(());
        }
        if let Some(item) = occurrence_of(field) {
            self.validate_references(item, depth + 1)?;
        } else {
            // A group is its list alone, its length the count: the NumInGroup
            // field framing it on the wire is its `FIX:counter`, never a
            // member standing beside it.
            let mut counters = Vec::new();
            for child in field.fields() {
                let counter = match FixField::new(child).group() {
                    Some(name) => {
                        FixField::new(self.definition(FixCategory::Groups, name)?).counter()?
                    }
                    None => FixField::new(child).counter()?,
                };
                counters.extend(counter);
            }
            for child in field.fields() {
                if !child.dtype().is_nested()
                    && let Some(tag) = FixField::new(child).tag()?
                    && counters.contains(&tag)
                {
                    return Err(Error::InvalidRecord {
                        path: format_smolstr!("{}.{}", field.name(), child.name()),
                        reason: crate::implementer::expected_got(
                            "no NumInGroup counter beside the group it counts, whose length is its count",
                            format_args!("{} ({tag})", child.name()),
                        ),
                    });
                }
                self.validate_references(child, depth + 1)?;
            }
        }
        Ok(())
    }

    pub(super) fn validate_catalog(&self) -> Result<()> {
        for field in self.scalars() {
            self.validate_definition(FixCategory::Fields, field)?;
        }
        // Per-definition first, then the property no single definition can
        // state: a derived tag identifies one definition in the whole catalog,
        // so two definitions holding one tag is a catalog defect even though
        // each is well formed on its own.
        let mut derived: HashMap<i32, &str> = HashMap::new();
        for entry in self.catalog.all() {
            self.validate_definition(entry.category, &entry.field)?;
            let name = entry.field.name();
            let tag = FixField::new(&entry.field)
                .tag()?
                .ok_or_else(|| Error::absent(super::field::TAG_KEY, name))?;
            if let Some(held) = derived.insert(tag, name)
                && held != name
            {
                return Err(Error::conflict(
                    "one FIX definition per derived tag",
                    "two definitions on one tag",
                    format_args!("{held:?} and {name:?} both hold {tag}"),
                ));
            }
        }
        Ok(())
    }

    /// The catalog as the documents a store writes, for a fold to write over.
    pub(super) fn documents(&self) -> Result<Documents> {
        Documents::from_registry(self)
    }

    /// Replaces the definition `field` names - its own name, which the
    /// catalog holds - with `field`, and resolves every reference to it
    /// again, so a group drawing on a component relaxed here holds the
    /// relaxed occurrence, as every message holding that group does.
    ///
    /// The document path a change to a definition's shape takes, where
    /// [`Self::insert_definition`] replaces the one definition and proves the
    /// occurrences other definitions hold of it against the replacement.
    /// Staged, so a refusal leaves the catalog as it was.
    pub(super) fn restate_definition(&mut self, category: FixCategory, field: Field) -> Result<()> {
        let mut raw = self.compact_catalog()?;
        raw.insert((category, field.name().to_owned()), compact(field, true)?);
        let mut staged = self.clone();
        staged.resolve_catalog(raw)?;
        staged.validate_catalog()?;
        staged.forget_answers();
        *self = staged;
        Ok(())
    }

    /// Resolves the documents a fold wrote into this catalog, and proves it.
    pub(super) fn settle(&mut self, documents: Documents) -> Result<()> {
        // The documents were rewritten for every rename as the fold made it.
        self.renamed.clear();
        self.resolve_catalog(documents.raw)?;
        self.validate_catalog()
    }

    /// Folds every named definition of `other` into `documents`, the way
    /// [`Self::insert`] folds one.
    ///
    /// Settled on documents and resolved once, by [`Self::settle`], so a
    /// definition that arrives referencing another that arrives beside it
    /// resolves whatever order the categories are walked in, and one that
    /// merges is seen extended by everything referencing it. The fields were
    /// folded before this, so a field reference resolves against the union.
    ///
    /// What disagrees with a held definition is passed over into `dropped`
    /// rather than refusing the fold, as [`Self::merge_with`] states. Before
    /// anything folds, each incoming definition is made to read this
    /// dictionary: a member reads a field as the scalar fold left it -
    /// `remap` rewrites the identity of a field merged by name and passes over
    /// the member of one passed over - a group whose counter is no int32
    /// field here, or whose own identity disagrees with the group held under
    /// its name, is passed over with every member reading it, and a message
    /// whose name another wire code holds is named for its own code.
    ///
    /// **One definition is one mutation.** A definition this fold cannot make
    /// read the dictionary, or whose fold refuses rather than passing a member
    /// over, is passed over whole into `dropped`: every write its fold made to
    /// `documents` is undone and every drop it recorded is replaced by its
    /// own, and every other definition of the source still folds. A
    /// definition that arrives new and goes is passed over with every member
    /// of the source reading it, as a group refused its counter is.
    ///
    /// **One structure is one definition.** Every definition folds by name
    /// first, as it always has; once every one of the source folded, each
    /// pair of definitions of one [structure](structural_key) the fold made
    /// folds into the definition held before the fold: an arrival stating
    /// the structure of a held definition, or a held definition the fold
    /// widened until it states another's. The kept one is relaxed to the
    /// more permissive side and lists both sources, and every document
    /// reading the other reads it ([`fold_alike_definitions`]). Two
    /// definitions held as they were, or two arrivals of one source, stay
    /// two: a dictionary stating two definitions of one structure states two.
    pub(super) fn merge_catalog(
        &self,
        documents: &mut Documents,
        other: &Self,
        remap: &HashMap<FixId, Option<(i32, SmolStr)>>,
        dropped: &mut Vec<FixDrop>,
    ) -> Result<()> {
        // The definitions that will not land, by category and name: a member
        // of the source reading one goes with it, where nothing held answers
        // to the name instead.
        let mut lost: Vec<(FixCategory, String)> = Vec::new();
        let mut losing = |documents: &Documents, category: FixCategory, name: &str| {
            if category == FixCategory::Groups || documents.get(category, name).is_none() {
                lost.push((category, name.to_owned()));
            }
        };
        let mut incoming = Vec::new();
        for category in [FixCategory::Components, FixCategory::Groups] {
            for field in other.catalog.iter(category) {
                let mark = dropped.len();
                let document = compact(field.clone(), true).and_then(|mut document| {
                    if remap.is_empty() {
                        return Ok(Ok(document));
                    }
                    // A group counts by a field as its members read one: the
                    // counter is read under the identity the scalar fold left.
                    if category == FixCategory::Groups
                        && let Some(error) = counter_remapped(other, &mut document, remap)?
                    {
                        return Ok(Err(error));
                    }
                    members_read(document, dropped, &mut |owner, member| {
                        remapped(owner, member, remap)
                    })
                    .map(Ok)
                });
                match document {
                    Ok(Ok(document)) => incoming.push((category, document)),
                    Ok(Err(error)) | Err(error) => {
                        dropped.truncate(mark);
                        losing(documents, category, field.name());
                        dropped.push(FixDrop::new(field.clone(), &error));
                    }
                }
            }
        }
        let mut kept = Vec::with_capacity(incoming.len());
        // The groups `standing` named for their own counter, under the name the
        // source's members read them by, so those members follow.
        let mut renamed: Vec<(String, String)> = Vec::new();
        for (category, document) in incoming {
            let name = document.name().to_owned();
            let (document, error) = match self.standing(documents, category, document.clone()) {
                Ok(Ok(document)) => {
                    if category == FixCategory::Groups && document.name() != name {
                        renamed.push((name, document.name().to_owned()));
                    }
                    kept.push((category, document));
                    continue;
                }
                Ok(Err(refused)) => refused,
                Err(error) => (document, error),
            };
            // Under the name the source's members read it by, whatever name
            // `standing` tried for it.
            losing(documents, category, &name);
            dropped.push(FixDrop::new(document, &error));
        }
        if !renamed.is_empty() {
            let mut reading = Vec::with_capacity(kept.len());
            for (category, document) in kept {
                let mark = dropped.len();
                match members_read(document.clone(), dropped, &mut |_, member| {
                    let group = FixField::new(member).group().and_then(|group| {
                        renamed.iter().find(|(held, _)| folds_equal(held, group))
                    });
                    if let Some((_, named)) = group {
                        FixFieldMut::new(member).set_group(named)?;
                    }
                    Ok(None)
                }) {
                    Ok(read) => reading.push((category, read)),
                    Err(error) => {
                        dropped.truncate(mark);
                        dropped.push(FixDrop::new(document, &error));
                    }
                }
            }
            kept = reading;
        }
        if !lost.is_empty() {
            let mut reading = Vec::with_capacity(kept.len());
            for (category, document) in kept {
                let mark = dropped.len();
                match members_read(document.clone(), dropped, &mut |owner, member| {
                    Ok(reference(member)
                        .filter(|(category, name)| {
                            lost.iter()
                                .any(|(held, lost)| held == category && folds_equal(lost, name))
                        })
                        .map(|(category, name)| Error::InvalidRecord {
                            path: format_smolstr!("{owner}.{}", member.name()),
                            reason: format_smolstr!(
                                "expected a {} this dictionary holds, got {name:?}, which the merge passed over",
                                if category == FixCategory::Groups {
                                    "group"
                                } else {
                                    "component"
                                }
                            ),
                        }))
                }) {
                    Ok(read) => reading.push((category, read)),
                    Err(error) => {
                        dropped.truncate(mark);
                        dropped.push(FixDrop::new(document, &error));
                    }
                }
            }
            kept = reading;
        }
        // What the documents state before this source folds, so the pass
        // after it folds only the pairs the fold made.
        let before = if kept.iter().any(|(category, document)| {
            matches!(category, FixCategory::Components | FixCategory::Groups)
                && FixField::new(document).msgtype().is_none()
        }) {
            structures_of(documents)
        } else {
            HashMap::new()
        };
        // What arrives new is put before anything merges, so a member that
        // references it on one side and states it inline on the other is
        // compared against it whatever category it belongs to.
        let mut folding = Vec::new();
        let mut arriving = Vec::new();
        for (category, document) in kept {
            if documents.get(category, document.name()).is_none() {
                arriving.push((category, document));
            } else {
                folding.push((category, document));
            }
        }
        let mut folded = Vec::with_capacity(arriving.len() + folding.len());
        // A member of the source reads its target as the source states it,
        // whichever of the two folds first: an arrival lands before anything
        // merges, so only a definition of a held name has to be held apart.
        documents.state(
            folding
                .iter()
                .map(|(category, document)| (*category, document)),
        );
        let landing = arriving
            .into_iter()
            .map(|(category, document)| (category, document, true))
            .chain(
                folding
                    .into_iter()
                    .map(|(category, document)| (category, document, false)),
            );
        for (category, document, landed) in landing {
            let mark = documents.checkpoint();
            let passed = dropped.len();
            match self.fold_document(documents, category, &document, &mut Some(&mut *dropped)) {
                Ok(()) => {
                    documents.release(mark);
                    folded.push((category, document, landed));
                }
                Err(error) => {
                    documents.rollback(mark);
                    dropped.truncate(passed);
                    dropped.push(FixDrop::new(document, &error));
                }
            }
        }
        documents.unstate();
        fold_alike_definitions(documents, &before, &folded)
    }

    /// Whether one incoming definition can stand beside what the fold holds:
    /// the definition, maybe renamed, or the refusal it is passed over with.
    ///
    /// A group counts its occurrences by an int32 field, so one whose counter
    /// is no such field here - a counter the scalar fold passed over, or one
    /// this dictionary types otherwise - cannot stand. A definition whose own
    /// identity - its counter, its component, its message code - disagrees
    /// with the one held under its name is another definition: a message is
    /// then named for its own wire code, as a CBlock names one no spelling
    /// names, and anything else is passed over.
    #[allow(clippy::type_complexity)]
    fn standing(
        &self,
        documents: &Documents,
        category: FixCategory,
        mut document: Field,
    ) -> Result<std::result::Result<Field, (Field, Error)>> {
        if category == FixCategory::Groups
            && matches!(
                document.dtype(),
                DataType::Serie(_) | DataType::LargeSerie(_)
            )
            && let Some(counter) = FixField::new(&document).counter()?
        {
            let held = self.get_field_by_tag(counter);
            if held.is_none_or(|field| field.dtype() != &DataType::Int32) {
                let error = Error::InvalidRecord {
                    path: document.name().into(),
                    reason: crate::implementer::expected_got(
                        "an int32 repeating-group counter this dictionary holds",
                        format_args!(
                            "tag {counter} as {}",
                            held.map_or_else(
                                || SmolStr::new("nothing"),
                                |field| format_smolstr!("{}", field.dtype())
                            )
                        ),
                    ),
                };
                return Ok(Err((document, error)));
            }
        }
        let Some((_, stored)) = documents.get(category, document.name()) else {
            return Ok(Ok(document));
        };
        // One group on one counter drawn from two components is one group:
        // the fold reconciles the component, so only the rest has to agree.
        let probe = match FixField::new(stored).component() {
            Some(component)
                if category == FixCategory::Groups
                    && FixField::new(stored).counter()?
                        == FixField::new(&document).counter()? =>
            {
                with_component(&document, component)?
            }
            _ => document.clone(),
        };
        let Err(error) = merge_root(stored, &probe) else {
            return Ok(Ok(document));
        };
        // Another identity under a held name is another definition: a message
        // is named for its own wire code, as a CBlock names one no spelling
        // names, and a group for its own counter, as a CBlock splits one for
        // its message - so a venue's `dealers` on its own counter stands
        // beside the held `dealers` rather than being passed over with every
        // member reading it.
        let (stored_counter, counter) = (
            FixField::new(stored).counter()?,
            FixField::new(&document).counter()?,
        );
        let renamed = match category {
            FixCategory::Groups => counter
                .filter(|counter| stored_counter.is_some_and(|held| held != *counter))
                .map(|counter| format!("{}_{counter}", document.name())),
            _ => FixField::new(&document)
                .msgtype()
                .filter(|wire| FixField::new(stored).msgtype() != Some(*wire))
                .map(super::msgtype::derived_name)
                .filter(|name| !folds_equal(name, document.name())),
        };
        let Some(name) = renamed else {
            let error = Error::InvalidRecord {
                path: document.name().into(),
                reason: format_smolstr!(
                    "expected the {} held under this name, got another: {error}",
                    if category == FixCategory::Groups {
                        "group"
                    } else {
                        "component"
                    }
                ),
            };
            return Ok(Err((document, error)));
        };
        log::debug!(
            "{:?} takes {name:?}: the {} held under its name is another",
            document.name(),
            if category == FixCategory::Groups {
                "group"
            } else {
                "message"
            }
        );
        document.set_name(name);
        let Some((_, held)) = documents.get(category, document.name()) else {
            return Ok(Ok(document));
        };
        // The name taken may hold this group from another source already,
        // drawn from another component: one group on one counter, whose
        // component the fold reconciles, as under the first name.
        let probe = match FixField::new(held).component() {
            Some(component)
                if category == FixCategory::Groups
                    && FixField::new(held).counter()? == FixField::new(&document).counter()? =>
            {
                with_component(&document, component)?
            }
            _ => document.clone(),
        };
        match merge_root(held, &probe) {
            Ok(_) => Ok(Ok(document)),
            Err(error) => Ok(Err((document, error))),
        }
    }
}

/// Whether `field` reads the definition `category` `name` anywhere beneath
/// its own root, through a member's reference marker.
fn reads_definition(field: &Field, category: FixCategory, name: &str, depth: usize) -> bool {
    if depth > 64 {
        return false;
    }
    let children: &[Field] = match occurrence_of(field) {
        Some(item) => std::slice::from_ref(item),
        None => field.fields(),
    };
    children.iter().any(|child| {
        reference(child).is_some_and(|(held, target)| held == category && folds_equal(target, name))
            || reads_definition(child, category, name, depth + 1)
    })
}

/// The field a member reads by reference, where it reads one.
fn field_reference(member: &Field) -> Option<&str> {
    match reference(member) {
        Some((FixCategory::Fields, name)) => Some(name),
        _ => None,
    }
}

/// The counter of the group a member reads by reference, where it reads one
/// the documents hold.
fn group_counter(documents: &Documents, member: &Field) -> Option<i32> {
    let (FixCategory::Groups, name) = reference(member)? else {
        return None;
    };
    let (_, group) = documents.get(FixCategory::Groups, name)?;
    FixField::new(group).counter().ok().flatten()
}

/// `group` drawing its occurrences from the component `name`.
fn with_component(group: &Field, name: &str) -> Result<Field> {
    let mut group = group.clone();
    FixFieldMut::new(&mut group).set_component(name)?;
    if let Some(item) = occurrence_of(&group) {
        let mut item = item.clone();
        FixFieldMut::new(&mut item).set_component(name)?;
        let dtype = group_dtype(&group, item)?;
        group.set_dtype(dtype)?;
    }
    Ok(group)
}

/// The first free slot of the derived block for `name`: XXH32 of the name
/// places it, and a slot `taken` answers for is stepped past, wrapping.
fn derived_tag(name: &str, taken: impl Fn(i32) -> bool) -> Result<i32> {
    let span = FixId::DEFINITION_TAG_MAX - FixId::DEFINITION_TAG_MIN;
    let hash = crate::xxhash::xxh32(name.as_bytes());
    #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
    let start = (hash % (span as u32)) as i32;
    for step in 0..span {
        let tag = FixId::DEFINITION_TAG_MIN + (start + step) % span;
        if !taken(tag) {
            return Ok(tag);
        }
    }
    Err(Error::InvalidRecord {
        path: name.into(),
        reason: crate::implementer::expected_got(
            "a free derived definition tag",
            format_args!("all {span} slots taken"),
        ),
    })
}

/// Rewrites every member of `raw` reading a field a fold renamed, so a
/// document written before the rename reads the field under the identity it
/// holds now.
///
/// A rename moves no member: the map answers a held identity for every key
/// it holds, so nothing here is passed over.
pub(super) fn rename_raw_references(
    raw: &mut BTreeMap<DefinitionKey, Field>,
    renamed: &HashMap<FixId, Option<(i32, SmolStr)>>,
) -> Result<()> {
    if renamed.is_empty() {
        return Ok(());
    }
    let keys: Vec<DefinitionKey> = raw.keys().cloned().collect();
    for key in keys {
        let Some(document) = raw.remove(&key) else {
            continue;
        };
        raw.insert(key, rename_document(document, renamed)?);
    }
    Ok(())
}

/// One document with every member reading a renamed field reading it under
/// the identity it holds now.
fn rename_document(
    document: Field,
    renamed: &HashMap<FixId, Option<(i32, SmolStr)>>,
) -> Result<Field> {
    let mut drops = Vec::new();
    let rewritten = members_read(document, &mut drops, &mut |owner, member| {
        remapped(owner, member, renamed)
    })?;
    debug_assert!(drops.is_empty(), "a rename passes no member over");
    Ok(rewritten)
}

/// The [structure](structural_key) of every component and group the
/// documents hold, by category and name - a message is its wire type's and
/// never another's, so none is keyed, and a definition stating no member
/// keys none, since an empty record says nothing two declarations could
/// share. A definition keying no structure - one reading a definition the
/// documents do not hold - is logged and left out.
fn structures_of(documents: &Documents) -> HashMap<DefinitionKey, SmolStr> {
    let mut structures = HashMap::new();
    let mut memo = StructureMemo::new();
    let lookup = |category, name: &str| documents.get(category, name).map(|(_, document)| document);
    for (key, document) in &documents.raw {
        if !matches!(key.0, FixCategory::Components | FixCategory::Groups)
            || FixField::new(document).msgtype().is_some()
        {
            continue;
        }
        match structural_key(document, &lookup, &mut memo) {
            Ok(structure) if structure != EMPTY_STRUCTURE => {
                structures.insert(key.clone(), structure);
            }
            Ok(_) => {}
            Err(error) => log::debug!("{:?} keys no structure: {error}", key.1),
        }
    }
    structures
}

/// The structure of a record stating no member.
pub(super) const EMPTY_STRUCTURE: &str = "()";

/// Folds every pair of definitions one source's fold left as one
/// [structure](structural_key) into one, once every definition of the
/// source folded: the components first, then the groups, whose structure is
/// their counter over the component they draw on.
///
/// `before` is the structure of every definition the documents held before
/// this source's fold. A definition the fold *made* - one that arrived, or a
/// held one whose structure the fold changed by widening it - is the only
/// kind that can make a new pair, so a class of one structure folds only
/// where it holds one, and only into a definition held before the fold:
/// - the survivor is the first definition, in name order, that the fold
///   reshaped, else the first one it left as it was - so a name a source's
///   merge by name widened keeps answering for its structure, and the next
///   fold of that source finds it there;
/// - every definition of the class that arrived, or that the fold reshaped,
///   folds into the survivor, and so does every one held as it was where
///   the survivor is reshaped, since that pair is new; two definitions held
///   as they were stay two where the survivor is one of them, because a
///   dictionary stating two definitions of one structure states two;
/// - a class of arrivals alone stays as it arrived, for the same reason: a
///   source stating two definitions of one structure states two, and it
///   folds into an empty dictionary as itself.
///
/// A fold is [`fold_alike`]: the survivor's identity and its own
/// nullability kept, each member's relaxed to the more permissive side, the
/// sources of both listed; the other's document taken out, and every
/// document reading it reading the survivor - a member by its marker, a
/// group by the component it draws on, wherever the reference stands.
///
/// A definition that merged into the held one of its own name and states
/// its structure relaxes it the same way, which the merge by name, keeping
/// the held side's members as stated, does not.
///
/// The survivor is a name the dictionary held, so the name a structure is
/// filed under, and the order of its members, are the first source's, as
/// every name a fold keeps is.
fn fold_alike_definitions(
    documents: &mut Documents,
    before: &HashMap<DefinitionKey, SmolStr>,
    folded: &[(FixCategory, Field, bool)],
) -> Result<()> {
    // Nothing was held, so everything arrived and stays as it arrived.
    if before.is_empty() {
        return Ok(());
    }
    // Merged by name: the held side's strictness, relaxed to the incoming
    // side's where the two are of one structure.
    {
        let mut memo = StructureMemo::new();
        let mut relaxed = Vec::new();
        {
            let lookup =
                |category, name: &str| documents.get(category, name).map(|(_, document)| document);
            for (category, incoming, landed) in folded {
                if *landed
                    || !matches!(category, FixCategory::Components | FixCategory::Groups)
                    || FixField::new(incoming).msgtype().is_some()
                {
                    continue;
                }
                let Some((key, document)) = documents.get(*category, incoming.name()) else {
                    continue;
                };
                let (Ok(held), Ok(stated)) = (
                    structural_key(document, &lookup, &mut memo),
                    structural_key(incoming, &lookup, &mut memo),
                ) else {
                    continue;
                };
                if held == stated && held != EMPTY_STRUCTURE {
                    let merged = fold_alike(document, incoming)?;
                    if &merged != document {
                        relaxed.push((key.clone(), merged));
                    }
                }
            }
        }
        for (key, merged) in relaxed {
            documents.put(key, merged);
        }
    }
    for category in [FixCategory::Components, FixCategory::Groups] {
        let after: Vec<(DefinitionKey, SmolStr)> = {
            let mut after: Vec<_> = structures_of(documents)
                .into_iter()
                .filter(|(key, _)| key.0 == category)
                .collect();
            after.sort();
            after
        };
        let reshaped =
            |key: &DefinitionKey, structure: &SmolStr| before.get(key) != Some(structure);
        // One class per structure, its members in name order.
        let mut classes: Vec<(SmolStr, Vec<DefinitionKey>)> = Vec::new();
        let mut at: HashMap<SmolStr, usize> = HashMap::new();
        for (key, structure) in after {
            match at.get(&structure) {
                Some(&position) => classes[position].1.push(key),
                None => {
                    at.insert(structure.clone(), classes.len());
                    classes.push((structure, vec![key]));
                }
            }
        }
        let mut renamed: Vec<(String, String)> = Vec::new();
        for (structure, class) in classes {
            if class.len() < 2 || !class.iter().any(|key| reshaped(key, &structure)) {
                continue;
            }
            let held: Vec<&DefinitionKey> = class
                .iter()
                .filter(|key| before.contains_key(*key))
                .collect();
            let Some(survivor) = held
                .iter()
                .find(|key| reshaped(key, &structure))
                .or_else(|| held.first())
                .map(|key| (*key).clone())
            else {
                continue;
            };
            let survivor_reshaped = reshaped(&survivor, &structure);
            for other in &class {
                if *other == survivor
                    || (!survivor_reshaped
                        && before.contains_key(other)
                        && !reshaped(other, &structure))
                {
                    continue;
                }
                log::debug!(
                    "{:?} folds into the {} {:?} it states alike",
                    other.1,
                    category.as_str(),
                    survivor.1
                );
                let alike = fold_alike(&documents.raw[&survivor], &documents.raw[other])?;
                documents.put(survivor.clone(), alike);
                documents.remove(other);
                renamed.push((other.1.clone(), survivor.1.clone()));
            }
        }
        if !renamed.is_empty() {
            let renames: Vec<(&str, &str)> = renamed
                .iter()
                .map(|(from, to)| (from.as_str(), to.as_str()))
                .collect();
            rename_definition_references(documents, category, &renames)?;
        }
    }
    Ok(())
}

/// Every document reading a definition `renamed` names reading the one it
/// folded into, wherever the reference stands ([`rename_references_in`]). A
/// rename moves no member, so nothing here is passed over.
fn rename_definition_references(
    documents: &mut Documents,
    category: FixCategory,
    renamed: &[(&str, &str)],
) -> Result<()> {
    let keys: Vec<DefinitionKey> = documents.raw.keys().cloned().collect();
    for key in keys {
        let Some(document) = documents.raw.get(&key) else {
            continue;
        };
        let mut rewritten = document.clone();
        if rename_references_in(&mut rewritten, category, renamed, true)? {
            documents.put(key, rewritten);
        }
    }
    Ok(())
}

/// Every reference `field` holds to a definition of `category` that
/// `renamed` names, rewritten to the name it folded into: a member by its
/// marker, a group's own root by the component it draws on, and the
/// occurrence of a group a document states inline, a map's entries and
/// every level beneath them alike - wherever a reference stands, because a
/// document names its target at any depth and one left naming a removed
/// document refuses the whole fold when it resolves. Answers whether
/// anything was rewritten.
fn rename_references_in(
    field: &mut Field,
    category: FixCategory,
    renamed: &[(&str, &str)],
    root: bool,
) -> Result<bool> {
    let target = |name: &str| {
        renamed
            .iter()
            .find(|(from, _)| folds_equal(from, name))
            .map(|(_, to)| SmolStr::new(to))
    };
    let mut changed = false;
    // A definition names its component on its own root and is still the
    // definition: a group draws on the renamed component there, and only a
    // member is a reference.
    let to = if root {
        (category == FixCategory::Components)
            .then(|| FixField::new(field).component().and_then(target))
            .flatten()
    } else {
        match reference(field) {
            Some((held, name)) if held == category => target(name),
            _ => None,
        }
    };
    if let Some(to) = to {
        match category {
            FixCategory::Groups => FixFieldMut::new(field).set_group(&to)?,
            _ => FixFieldMut::new(field).set_component(&to)?,
        }
        changed = true;
    }
    let dtype = match field.dtype() {
        DataType::Struct(children) => {
            let mut children: Vec<Field> = children.iter().cloned().collect();
            let mut moved = false;
            for child in &mut children {
                moved |= rename_references_in(child, category, renamed, false)?;
            }
            moved
                .then(|| StructType::from_fields(children).map(DataType::from))
                .transpose()?
        }
        DataType::Serie(_)
        | DataType::LargeSerie(_)
        | DataType::Map(_)
        | DataType::SortedMap(_) => {
            let mut item = occurrence_of(field)
                .expect("the variant was just matched")
                .clone();
            if rename_references_in(&mut item, category, renamed, false)? {
                Some(group_dtype(field, item)?)
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(dtype) = dtype {
        field.set_dtype(dtype)?;
        changed = true;
    }
    Ok(changed)
}

/// A member reading a field the scalar fold did not keep under the identity
/// it names: rewritten to the identity that holds it now, or the refusal it
/// is passed over with.
fn remapped(
    owner: &str,
    member: &mut Field,
    remap: &HashMap<FixId, Option<(i32, SmolStr)>>,
) -> Result<Option<Error>> {
    let view = FixField::new(member);
    let (Some(name), Some(tag)) = (view.field_ref(), view.tag()?) else {
        return Ok(None);
    };
    let Ok(id) = FixId::of(tag, name) else {
        return Ok(None);
    };
    match remap.get(&id) {
        None => Ok(None),
        Some(Some((held, named))) => {
            // A member named after the field it read - a constraint takes
            // its field's name, and an unnamed field's is its digits - is
            // named after it still.
            if folds_equal(member.name(), name) {
                member.set_name(named.as_str());
            }
            FixFieldMut::new(member).set_field_ref(named)?;
            FixFieldMut::new(member).set_tag(*held)?;
            Ok(None)
        }
        Some(None) => Ok(Some(Error::InvalidRecord {
            path: format_smolstr!("{owner}.{}", member.name()),
            reason: format_smolstr!(
                "expected a field this dictionary holds, got {name} ({tag}), which the merge passed over"
            ),
        })),
    }
}

/// Keeps two members of one struct in wire order under names no two of
/// which fold to one: the first keeps its name, a later one the first free
/// `{name}2`, `{name}3`... Their `FIX:tag` still names the wire field.
///
/// The one rule a duplicate member is named by, whether a grammar binds one
/// tag twice, a fold lands two of a source's fields on one held field, or a
/// fold places a member reading another tag beside the held one of its name.
pub(super) fn push_member(children: &mut Vec<Field>, mut field: Field) {
    let taken = |children: &[Field], candidate: &str| {
        children
            .iter()
            .any(|held| folds_equal(held.name(), candidate))
    };
    if !taken(children, field.name()) {
        children.push(field);
        return;
    }
    let base = field.name().to_owned();
    for suffix in 2..u32::MAX {
        let candidate = format!("{base}{suffix}");
        if !taken(children, &candidate) {
            field.set_name(candidate);
            children.push(field);
            return;
        }
    }
}

/// A group of the source whose counter the scalar fold did not keep under
/// the identity the group names, read the way [`remapped`] reads a member:
/// counted by the field that holds the counter now, or the refusal the group
/// is passed over with where the counter itself was passed over.
fn counter_remapped(
    other: &FixRegistry,
    group: &mut Field,
    remap: &HashMap<FixId, Option<(i32, SmolStr)>>,
) -> Result<Option<Error>> {
    let Some(tag) = FixField::new(group).counter()? else {
        return Ok(None);
    };
    let Some(counter) = other.get_field_by_tag(tag) else {
        return Ok(None);
    };
    match remap.get(&super::registry::canonical_id(counter)?) {
        None => Ok(None),
        Some(Some((held, _))) => {
            if *held != tag {
                FixFieldMut::new(group).set_counter(*held)?;
            }
            Ok(None)
        }
        Some(None) => Ok(Some(Error::InvalidRecord {
            path: group.name().into(),
            reason: format_smolstr!(
                "expected a counter this dictionary holds, got {} ({tag}), which the merge passed over",
                counter.name()
            ),
        })),
    }
}

/// Passes one declaration over into `drops`, or refuses it where there are
/// none to pass it into.
fn passed_over(
    drops: &mut Option<&mut Vec<FixDrop>>,
    incoming: &Field,
    error: Error,
) -> Result<()> {
    match drops.as_deref_mut() {
        Some(drops) => {
            drops.push(FixDrop::new(incoming.clone(), &error));
            Ok(())
        }
        None => Err(error),
    }
}

/// `field` with every member it states read by `member`: rewritten in place,
/// or, where `member` answers a refusal, passed over into `drops`.
///
/// Walks what the document states inline and stops at a reference, which is
/// its target's to answer for.
fn members_read(
    mut field: Field,
    drops: &mut Vec<FixDrop>,
    member: &mut impl FnMut(&str, &mut Field) -> Result<Option<Error>>,
) -> Result<Field> {
    let dtype = match field.dtype() {
        DataType::Struct(children) => {
            let mut kept = Vec::with_capacity(children.len());
            for child in children.iter() {
                let mut child = child.clone();
                if let Some(error) = member(field.name(), &mut child)? {
                    drops.push(FixDrop::new(child, &error));
                    continue;
                }
                // A member the read renamed onto a sibling's name - two fields
                // of the source the scalar fold landed on one held field, each
                // member named after the field it read - is the same field
                // twice, as a duplicate constraint is, and takes the name a
                // duplicate constraint takes rather than refusing the struct.
                push_member(
                    &mut kept,
                    if reference(&child).is_some() {
                        child
                    } else {
                        members_read(child, drops, member)?
                    },
                );
            }
            DataType::from(StructType::from_fields(kept)?)
        }
        DataType::Serie(item) if reference(item).is_none() => {
            DataType::serie(members_read(item.as_ref().clone(), drops, member)?)
        }
        DataType::LargeSerie(item) if reference(item).is_none() => {
            DataType::large_serie(members_read(item.as_ref().clone(), drops, member)?)
        }
        _ => return Ok(field),
    };
    field.set_dtype(dtype)?;
    Ok(field)
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/fix/catalog.rs` and `rust/tests/fix/group_plan.rs`
    //! pin and a caller cannot reach.
    //!
    //! [`FixRegistry`] is published and its definition doors with it; the
    //! compiled group plan a definition carries is not, because a plan is the
    //! layout a row is laid out by rather than anything a caller states, and
    //! neither is the structure two declarations of one definition agree on,
    //! which a caller meets as one definition where it declared two.
    use super::super::group_plan::GroupPlan;
    use crate::{Field, FixCategory, FixRegistry, Result};

    /// The structure of a definition, read through the registry's own
    /// definitions: what two grammars have to agree on to be declaring one.
    ///
    /// # Errors
    ///
    /// Returns the absence of a definition a member references and the
    /// registry does not hold.
    pub fn structural_key(registry: &FixRegistry, field: &Field) -> Result<String> {
        super::structural_key(
            field,
            &|category, name| registry.get_definition(category, name),
            &mut super::StructureMemo::new(),
        )
        .map(|key| key.to_string())
    }

    /// The group definition a counter tag names.
    #[must_use]
    pub fn get_group_by_tag(registry: &FixRegistry, tag: i32) -> Option<&Field> {
        registry.get_group_by_tag(tag)
    }

    /// The compiled plan a counter tag's group definition carries, where its
    /// layout is one a wire states a tag at a time.
    #[must_use]
    pub fn get_group_plan_by_tag(registry: &FixRegistry, tag: i32) -> Option<&GroupPlan> {
        registry.get_group_plan_by_tag(tag)
    }

    /// The unique group a counter tag opens, reporting absence or ambiguity.
    ///
    /// # Errors
    ///
    /// Returns a typed absence where no group, or more than one, answers the
    /// tag.
    pub fn group_by_tag(registry: &FixRegistry, tag: i32) -> Result<&Field> {
        registry.group_by_tag(tag)
    }

    /// Fold one definition into the catalog, answering whether it was new.
    ///
    /// The published door replaces or inserts whole; this is the fold a merge
    /// of two dictionaries performs, which a caller never reaches on its own.
    ///
    /// # Errors
    ///
    /// Returns whatever folding, resolving and validating the merged catalog
    /// raises; a refusal leaves the registry untouched.
    pub fn add_definition(
        registry: &mut FixRegistry,
        category: FixCategory,
        field: Field,
    ) -> Result<bool> {
        registry.add_definition(category, field)
    }

    /// The tag a named definition derives, out of the block reserved for them.
    ///
    /// # Errors
    ///
    /// Returns a typed failure where every slot in the block is taken.
    pub fn derived_definition_tag(registry: &FixRegistry, name: &str) -> Result<i32> {
        registry.derived_definition_tag(name)
    }
}
