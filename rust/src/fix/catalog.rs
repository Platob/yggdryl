//! Named FIX definitions beside the scalar wire-field indexes.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use super::group_plan::GroupPlan;
use super::registry::name_digest;
use super::store::{DefinitionKey, compact, reference};
use super::{FixId, FixRegistry, MsgType};
use crate::types::folds_equal;
use crate::{DataType, Error, Field, FixCategory, Result};

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

    fn from_field(category: FixCategory, field: Field) -> Result<Self> {
        match category {
            FixCategory::Messages => Ok(Self::Message(MsgType::from_field(field)?)),
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
    names: HashMap<(FixCategory, u64), usize>,
    order: Vec<usize>,
    /// Each counter tag, and the one group it opens - `None` where two
    /// groups claim one counter, which names nothing.
    counters: HashMap<i32, Option<usize>>,
    /// Each wire code, and the message a bare code answers: the one named
    /// as tag 35's code set names the code, else the first in name order.
    message_codes: HashMap<SmolStr, usize>,
    message_aliases: HashMap<u64, MessageAlias>,
    /// Each wire code, and the name tag 35's code set gives it.
    code_names: HashMap<SmolStr, SmolStr>,
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
        crate::types::folds_equal(entry.field.name(), name).then_some(position)
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

    fn at(&self, category: FixCategory, index: usize) -> Option<&Field> {
        let start = self
            .order
            .partition_point(|position| self.entries[*position].category < category);
        let position = *self.order.get(start.checked_add(index)?)?;
        let entry = &self.entries[position];
        (entry.category == category).then_some(entry.field.as_field())
    }

    fn index_counters(&mut self) {
        self.counters.clear();
        for (position, entry) in self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.category == FixCategory::Groups)
        {
            if let Some(tag) = entry.field.as_fix().counter().ok().flatten() {
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
        let mut codes: HashMap<SmolStr, usize> = HashMap::new();
        for (position, entry) in self.entries.iter().enumerate() {
            let Some(message) = entry.field.message() else {
                continue;
            };
            let code = message.as_str();
            let names_code = |name: &str| {
                self.code_names
                    .get(code)
                    .is_some_and(|named| crate::types::folds_equal(named, name))
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
        if let Some(position) = self.position(FixCategory::Messages, spelling) {
            return Some(position);
        }
        let alias = self
            .message_aliases
            .get(&name_digest(spelling, 0x4d53_475f_414c_4941))?;
        if !crate::types::folds_equal(&alias.spelling, spelling) {
            return None;
        }
        let code = alias.code.as_ref()?;
        self.message_codes.get(code).copied()
    }

    fn index_message_aliases(&mut self, field: Option<&Field>) {
        self.message_aliases.clear();
        self.code_names.clear();
        let Some(field) = field else {
            // The code set decides which message a bare code answers, so
            // its going re-decides them.
            self.index_messages();
            return;
        };
        for code in field.as_fix().codes().filter_map(Result::ok) {
            self.code_names
                .insert(SmolStr::new(code.value()), SmolStr::new(code.name()));
            for spelling in std::iter::once(code.name()).chain(code.aliases()) {
                let digest = name_digest(spelling, 0x4d53_475f_414c_4941);
                self.message_aliases
                    .entry(digest)
                    .and_modify(|held| {
                        if !crate::types::folds_equal(&held.spelling, spelling)
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
        if category == FixCategory::Groups {
            field
                .as_fix()
                .counter()?
                .ok_or_else(|| Error::absent("fix:counter", field.name()))?;
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
                    crate::text::expected_got(self.entries[position].field.name(), field.name()),
                ));
            }
            let field = DefinitionField::from_field(category, field)?;
            let prior = std::mem::replace(&mut self.entries[position].field, field).into_field();
            if category == FixCategory::Groups {
                self.index_counters();
            }
            if category == FixCategory::Messages {
                self.index_messages();
            }
            return Ok(Some(prior));
        }
        let position = self.entries.len();
        let field = DefinitionField::from_field(category, field)?;
        self.entries.push(Definition { category, field });
        self.names.insert(key, position);
        self.order.push(position);
        self.order.sort_by(|left, right| {
            let left = &self.entries[*left];
            let right = &self.entries[*right];
            (left.category, left.field.name()).cmp(&(right.category, right.field.name()))
        });
        if category == FixCategory::Groups {
            self.index_counters();
        }
        if category == FixCategory::Messages {
            self.index_messages();
        }
        Ok(None)
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
        self.index_counters();
        self.index_messages();
        removed
    }
}

fn invalid(field: &Field, expected: impl std::fmt::Display) -> Error {
    Error::InvalidRecord {
        path: field.name().into(),
        reason: crate::text::expected_got(expected, field.dtype()),
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
        FixCategory::Fields if field.dtype().is_nested() => Err(not_scalar(field)),
        FixCategory::Messages | FixCategory::Components
            if !matches!(field.dtype(), DataType::Struct(_)) =>
        {
            Err(invalid(field, "a Struct datatype"))
        }
        FixCategory::Groups if definition_category(field) != Some(FixCategory::Groups) => {
            Err(invalid(field, "a List of non-null Struct occurrences"))
        }
        _ => Ok(()),
    }
}

/// The category a nested field's shape names, for a caller handing the
/// registry a definition without saying which it is.
///
/// A repeating group is the one shape a List has in FIX - occurrences of a
/// non-null Struct - and a message is a Struct that says which wire code it
/// answers to; every other Struct is a component. A nested datatype that is
/// none of these is no definition at all, and answers nothing.
pub(super) fn definition_category(field: &Field) -> Option<FixCategory> {
    match field.dtype() {
        DataType::List(item) | DataType::LargeList(item)
            if !item.is_nullable() && matches!(item.dtype(), DataType::Struct(_)) =>
        {
            Some(FixCategory::Groups)
        }
        DataType::Struct(_) if field.as_fix().msgtype().is_some() => Some(FixCategory::Messages),
        DataType::Struct(_) => Some(FixCategory::Components),
        _ => None,
    }
}

/// The occurrence a group's list holds.
fn occurrence_of(group: &Field) -> Result<&Field> {
    match group.dtype() {
        DataType::List(item) | DataType::LargeList(item) => Ok(item),
        _ => Err(invalid(group, "a List of non-null Struct occurrences")),
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
/// through the metadata merge every protocol shares, the `fix:` keys through
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
    match stored.as_fix().tag()? {
        Some(tag) => merged.as_fix_mut().set_tag(tag)?,
        None => {
            merged.remove_metadata(super::field::TAG_KEY);
        }
    }
    merged.as_fix_mut().merge_with(&stored.as_fix())?;
    Ok(merged)
}

/// The compact documents a fold writes over, in the shape the resolver reads.
///
/// A merge is settled on documents rather than on resolved trees, because a
/// reference's expansion is the target's business, and the map is what
/// [`FixRegistry::resolve_catalog`] derives the catalog from - so the fold's
/// output is the store's input. The index beside it finds a document by the
/// folded name the catalog keys it under, and every hit is rechecked against
/// the key, exactly as the catalog rechecks its own.
struct Documents {
    raw: BTreeMap<DefinitionKey, Field>,
    keys: HashMap<(FixCategory, u64), Vec<DefinitionKey>>,
}

impl Documents {
    fn from_registry(registry: &FixRegistry) -> Result<Self> {
        let mut documents = Self {
            raw: BTreeMap::new(),
            keys: HashMap::new(),
        };
        for (key, document) in registry.compact_catalog()? {
            documents.put(key, document);
        }
        Ok(documents)
    }

    /// The document one folded name reaches, with its key.
    fn get(&self, category: FixCategory, name: &str) -> Option<(&DefinitionKey, &Field)> {
        self.keys
            .get(&Catalog::key(category, name))?
            .iter()
            .find(|key| folds_equal(&key.1, name))
            .and_then(|key| self.raw.get_key_value(key))
    }

    fn put(&mut self, key: DefinitionKey, document: Field) {
        let held = self.keys.entry(Catalog::key(key.0, &key.1)).or_default();
        if !held.contains(&key) {
            held.push(key.clone());
        }
        self.raw.insert(key, document);
    }
}

pub(super) fn validate_name(field: &Field) -> Result<()> {
    let name = field.name();
    if name.is_empty()
        || matches!(name, "." | "..")
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(Error::InvalidRecord {
            path: name.into(),
            reason: "expected a nonempty ASCII definition name containing letters, digits, underscore, hyphen, or dot".into(),
        });
    }
    Ok(())
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
fn canonical_occurrences(mut field: Field, root: bool) -> Result<Field> {
    if !root
        && field.as_fix().field_ref().is_none()
        && (field.as_fix().group().is_some() || field.as_fix().component().is_some())
    {
        field.remove_metadata(super::field::TAG_KEY);
    }
    let dtype = match field.dtype() {
        DataType::Struct(children) => Some(DataType::from_fields(
            children
                .iter()
                .cloned()
                .map(|child| canonical_occurrences(child, false))
                .collect::<Result<Vec<_>>>()?,
        )?),
        DataType::List(item) => Some(DataType::list(canonical_occurrences(
            item.as_ref().clone(),
            false,
        )?)),
        DataType::LargeList(item) => Some(DataType::large_list(canonical_occurrences(
            item.as_ref().clone(),
            false,
        )?)),
        _ => None,
    };
    if let Some(dtype) = dtype {
        field.set_dtype(dtype)?;
    }
    Ok(field)
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

    /// Iterates registered message singletons in name order.
    pub fn msgtypes(&self) -> impl Iterator<Item = &MsgType> {
        self.catalog
            .order
            .iter()
            .filter_map(|position| self.catalog.entries[*position].field.message())
    }

    /// Borrows a message singleton by its stable name iteration position.
    pub fn msgtype_at(&self, index: usize) -> Option<&MsgType> {
        let start = self.catalog.order.partition_point(|position| {
            self.catalog.entries[*position].category < FixCategory::Messages
        });
        let position = *self.catalog.order.get(start.checked_add(index)?)?;
        self.catalog.entries[position].field.message()
    }

    pub(super) fn refresh_msgtype_aliases(&mut self) {
        let field = self.get_field_by_tag(35).cloned();
        self.catalog.index_message_aliases(field.as_ref());
    }

    /// Resolves one category's folded name.
    pub fn get_definition(&self, category: FixCategory, name: &str) -> Option<&Field> {
        if category == FixCategory::Fields {
            return self.get_field_by_name(name);
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
        self.iter()
            .take(if category == FixCategory::Fields {
                self.len()
            } else {
                0
            })
            .chain(self.catalog.iter(category))
    }

    /// Borrows one definition by its category's deterministic iteration position.
    ///
    /// Positions stay stable while the registry is unchanged. Field positions
    /// follow tag order; named positions follow canonical name order.
    pub fn definition_at(&self, category: FixCategory, index: usize) -> Option<&Field> {
        if category == FixCategory::Fields {
            return self.iter().nth(index);
        }
        self.catalog.at(category, index)
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
        if category == FixCategory::Groups {
            if let Some(name) = field.as_fix().component().map(str::to_owned) {
                let component = self.definition(FixCategory::Components, &name)?;
                if let DataType::List(item) | DataType::LargeList(item) = field.dtype() {
                    // Against the canonical shape: the stored component holds
                    // no derived tag on its own occurrences, and an item a
                    // caller cloned out of the catalog still does.
                    let stated = canonical_occurrences(item.as_ref().clone(), true)?;
                    if stated.dtype() != component.dtype() {
                        return Err(invalid(
                            &field,
                            format_args!("component {name:?} datatype {}", component.dtype()),
                        ));
                    }
                    let mut item = item.as_ref().clone();
                    item.as_fix_mut().set_component(&name)?;
                    let dtype = if matches!(field.dtype(), DataType::LargeList(_)) {
                        DataType::large_list(item)
                    } else {
                        DataType::list(item)
                    };
                    field.set_dtype(dtype)?;
                }
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
            .and_then(|stored| stored.as_fix().tag().ok().flatten());
        let own = match field.as_fix().tag()? {
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
        field.as_fix_mut().set_tag(tag)?;
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
        if category == FixCategory::Fields && stored.as_fix().id()? != field.as_fix().id()? {
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
    /// the occurrence Struct inside the List, whose counter and component
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
    /// identity keys excepted, and the `fix:` keys follow
    /// [`FixFieldMut::merge_with`](crate::FixFieldMut::merge_with), which
    /// holds both sides to one message code, counter and component.
    ///
    /// The merge is settled on the documents a store writes, references
    /// folded to their markers, and the catalog is resolved from them again:
    /// every message and component referencing the definition sees the
    /// appended members without holding a copy of anything.
    ///
    /// ```
    /// use yggdryl::{DataType, FixCategory, FixRegistry};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut party_id = DataType::Utf8.nullable_field("PartyID");
    /// party_id.as_fix_mut().set_tag(448)?;
    /// let mut registry = FixRegistry::from_fields([party_id.clone()])?;
    /// party_id.as_fix_mut().set_field_ref("PartyID")?;
    /// let party = DataType::from_fields([party_id])?.required_field("Party");
    /// registry.create_definition(FixCategory::Components, party)?;
    /// // A message restates the component through a reference to it.
    /// let mut party = registry.definition(FixCategory::Components, "Party")?.clone();
    /// party.as_fix_mut().set_component("Party")?;
    /// let mut order = DataType::from_fields([party])?.required_field("Order");
    /// order.as_fix_mut().set_msgtype("D")?;
    /// registry.create_definition(FixCategory::Messages, order)?;
    ///
    /// // Extending the component is one call, and the message sees the member.
    /// let mut extended = registry.definition(FixCategory::Components, "Party")?.clone();
    /// let note = DataType::Utf8.nullable_field("PartyNote");
    /// let members = extended.fields().iter().cloned().chain([note]);
    /// extended.set_dtype(DataType::from_fields(members)?)?;
    /// assert!(!registry.add_definition(FixCategory::Components, extended)?, "merged");
    /// let member = yggdryl::FieldPath::from_str("Order.Party.PartyNote")?;
    /// assert_eq!(registry.field_by_path(&member)?.dtype(), &DataType::Utf8);
    /// assert_eq!(registry.definition(FixCategory::Components, "Party")?.field_len(), 2);
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
    pub fn add_definition(&mut self, category: FixCategory, field: Field) -> Result<bool> {
        if category == FixCategory::Fields {
            return self.add_field(field);
        }
        let mut staged = self.clone();
        let added = staged.fold_definition(category, field)?;
        *self = staged;
        Ok(added)
    }

    /// The fold of one named definition, without the staging.
    ///
    /// [`Self::add_definition`] is this plus the copy that makes it one
    /// mutation, and a fold already holding a staged dictionary calls this so
    /// the copy is paid once.
    pub(super) fn fold_definition(&mut self, category: FixCategory, field: Field) -> Result<bool> {
        validate_name(&field)?;
        check_shape(category, &field)?;
        if self.get_definition(category, field.name()).is_none() {
            self.insert_definition(category, field)?;
            return Ok(true);
        }
        let mut documents = Documents::from_registry(self)?;
        self.fold_document(&mut documents, category, &field)?;
        self.resolve_catalog(documents.raw)?;
        self.validate_catalog()?;
        Ok(false)
    }

    /// Folds one incoming definition over the documents.
    ///
    /// A document nothing stored answers to is added as the store would read
    /// it; one a stored document answers to, by folded name, is merged into
    /// that document under the stored key.
    fn fold_document(
        &self,
        documents: &mut Documents,
        category: FixCategory,
        incoming: &Field,
    ) -> Result<()> {
        let incoming = compact(incoming.clone(), true)?;
        let Some((key, stored)) = documents.get(category, incoming.name()) else {
            let key = (category, incoming.name().to_owned());
            documents.put(key, incoming);
            return Ok(());
        };
        let (key, stored) = (key.clone(), stored.clone());
        let merged = self.merge_documents(documents, category, &stored, &incoming)?;
        documents.put(key, merged);
        Ok(())
    }

    /// One level of `incoming` folded into `stored`, both compact.
    fn merge_documents(
        &self,
        documents: &mut Documents,
        category: FixCategory,
        stored: &Field,
        incoming: &Field,
    ) -> Result<Field> {
        let mut merged = merge_root(stored, incoming)?;
        let dtype = if category == FixCategory::Groups {
            let (held, item) = (occurrence_of(stored)?, occurrence_of(incoming)?);
            let occurrence = match (reference(held), reference(item)) {
                // The stored occurrence is a component's, so what the incoming
                // one adds belongs to that component: its members are folded
                // there - only its members, since the occurrence's own root
                // is the group's business and is merged above - and the
                // group, with every other reference, reads them back from
                // there once the documents resolve.
                (Some((FixCategory::Components, name)), None) => {
                    let Some((key, component)) = documents.get(FixCategory::Components, name)
                    else {
                        return Err(Error::absent(FixCategory::Components.as_str(), name));
                    };
                    let (key, mut component) = (key.clone(), component.clone());
                    let members = self.merge_children(
                        documents,
                        component.name(),
                        component.fields(),
                        item.fields(),
                    )?;
                    component.set_dtype(DataType::from_fields(members)?)?;
                    documents.put(key, component);
                    held.clone()
                }
                (None, None) => {
                    let mut occurrence = merge_root(held, item)?;
                    let members = self.merge_children(
                        documents,
                        stored.name(),
                        held.fields(),
                        item.fields(),
                    )?;
                    occurrence.set_dtype(DataType::from_fields(members)?)?;
                    occurrence
                }
                _ => {
                    self.agree(documents, stored.name(), held, item)?;
                    held.clone()
                }
            };
            if matches!(stored.dtype(), DataType::LargeList(_)) {
                DataType::large_list(occurrence)
            } else {
                DataType::list(occurrence)
            }
        } else {
            DataType::from_fields(self.merge_children(
                documents,
                stored.name(),
                stored.fields(),
                incoming.fields(),
            )?)?
        };
        merged.set_dtype(dtype)?;
        Ok(merged)
    }

    /// The stored children, then every incoming child no stored one answers
    /// to.
    ///
    /// Order is the stored definition's, because a message's members are read
    /// positionally by everything that walks it; what arrives is appended in
    /// the order it was declared.
    fn merge_children(
        &self,
        documents: &Documents,
        owner: &str,
        stored: &[Field],
        incoming: &[Field],
    ) -> Result<Vec<Field>> {
        let mut merged: Vec<Field> = stored.to_vec();
        for child in incoming {
            match merged
                .iter()
                .find(|held| folds_equal(held.name(), child.name()))
            {
                Some(held) => self.agree(documents, owner, held, child)?,
                None => merged.push(child.clone()),
            }
        }
        Ok(merged)
    }

    /// Whether an incoming child restates the stored one its name folds onto.
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
    fn agree(&self, documents: &Documents, owner: &str, held: &Field, child: &Field) -> Result<()> {
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
            return Ok(());
        }
        Err(Error::InvalidRecord {
            path: format_smolstr!("{owner}.{}", held.name()),
            reason: crate::text::expected_got(
                format_args!("{} stored for it", describe(held, held_target.as_ref())),
                describe(child, child_target.as_ref()),
            ),
        })
    }

    /// The datatype the reference `field` carries resolves to, in the compact
    /// shape the documents hold: a field's from this registry, a named
    /// definition's from the documents being folded, so a definition
    /// extended earlier in the same fold answers extended.
    fn referenced_dtype(&self, documents: &Documents, field: &Field) -> Result<DataType> {
        let (category, name) = reference(field)
            .ok_or_else(|| Error::absent("a field, component, or group reference", field.name()))?;
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

    /// The unique group one counter tag opens; two groups on one counter
    /// name nothing.
    pub fn get_group_by_counter(&self, tag: i32) -> Option<&Field> {
        let position = self.catalog.counters.get(&tag).copied().flatten()?;
        Some(self.catalog.entries[position].field.as_field())
    }

    pub(super) fn get_group_plan_by_counter(&self, tag: i32) -> Option<&GroupPlan> {
        let position = self.catalog.counters.get(&tag).copied().flatten()?;
        match &self.catalog.entries[position].field {
            DefinitionField::Group(_, plan) => Some(plan),
            _ => None,
        }
    }

    /// The unique group a counter tag opens, reporting absence or ambiguity.
    pub fn group_by_counter(&self, tag: i32) -> Result<&Field> {
        self.get_group_by_counter(tag)
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
        [
            FixCategory::Components,
            FixCategory::Groups,
            FixCategory::Messages,
        ]
        .into_iter()
        .any(|category| {
            self.catalog
                .iter(category)
                .any(|held| held.as_fix().tag().ok().flatten() == Some(tag))
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
        let span = FixId::DEFINITION_TAG_MAX - FixId::DEFINITION_TAG_MIN;
        let mut hasher = crate::xxhash::Xxh32::new();
        hasher.write_bytes(name.as_bytes());
        #[allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]
        let start = (hasher.as_u32() % (span as u32)) as i32;
        for step in 0..span {
            let tag = FixId::DEFINITION_TAG_MIN + (start + step) % span;
            if !self.definition_tag_in_use(tag) {
                return Ok(tag);
            }
        }
        Err(Error::InvalidRecord {
            path: name.into(),
            reason: crate::text::expected_got(
                "a free derived definition tag",
                format_args!("all {span} slots taken"),
            ),
        })
    }

    pub(super) fn validate_definition(&self, category: FixCategory, field: &Field) -> Result<()> {
        if category != FixCategory::Fields {
            validate_name(field)?;
            if let Some(tag) = field.as_fix().tag()? {
                if !FixId::is_definition_tag(tag) {
                    return Err(Error::InvalidRecord {
                        path: field.name().into(),
                        reason: crate::text::expected_got(
                            "a named FIX definition's derived tag",
                            format_args!(
                                "tag {tag} outside [{}, {})",
                                FixId::DEFINITION_TAG_MIN,
                                FixId::DEFINITION_TAG_MAX
                            ),
                        ),
                    });
                }
            }
        }
        check_shape(category, field)?;
        if category == FixCategory::Groups {
            let tag = field
                .as_fix()
                .counter()?
                .ok_or_else(|| Error::absent("fix:counter", field.name()))?;
            let counter = self.field_by_tag(tag)?;
            if counter.dtype() != &DataType::Int32 {
                return Err(invalid(counter, "an int32 repeating-group counter"));
            }
            if let Some(component) = field.as_fix().component() {
                if let DataType::List(item) | DataType::LargeList(item) = field.dtype() {
                    if !item
                        .as_fix()
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
            }
        }
        self.validate_references(field, 0)
    }

    pub(super) fn validate_references(&self, field: &Field, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err(invalid(field, "FIX references nested at most 64 levels"));
        }
        let view = field.as_fix();
        for code in view.codes() {
            code?;
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
                match field.dtype() {
                    DataType::List(item) | DataType::LargeList(item) => item.as_ref(),
                    _ => field,
                }
            } else {
                field
            };
            if occurrence.dtype() != target.dtype() {
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
            if category == FixCategory::Fields && view.tag()? != target.as_fix().tag()? {
                return Err(Error::conflict(
                    "the referenced FIX field's tag",
                    "a different tag",
                    field.name(),
                ));
            }
            let marker = match category {
                FixCategory::Fields => "fix:field",
                FixCategory::Groups => "fix:group",
                _ => "fix:component",
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
        match field.dtype() {
            DataType::List(item) | DataType::LargeList(item) => {
                self.validate_references(item, depth + 1)?;
            }
            _ => {
                for child in field.fields() {
                    self.validate_references(child, depth + 1)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn validate_catalog(&self) -> Result<()> {
        for field in self.iter() {
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
            let tag = entry
                .field
                .as_fix()
                .tag()?
                .ok_or_else(|| Error::absent(super::field::TAG_KEY, name))?;
            // Nested rather than a let-chain: the core builds at 1.85.
            if let Some(held) = derived.insert(tag, name) {
                if held != name {
                    return Err(Error::conflict(
                        "one FIX definition per derived tag",
                        "two definitions on one tag",
                        format_args!("{held:?} and {name:?} both hold {tag}"),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Folds every named definition of `other` into this catalog, the way
    /// [`Self::add_definition`] folds one.
    ///
    /// Settled on documents and resolved once at the end, so a definition
    /// that arrives referencing another that arrives beside it resolves
    /// whatever order the categories are walked in, and one that merges is
    /// seen extended by everything referencing it. The fields were folded
    /// before this, so a field reference resolves against the union.
    pub(super) fn merge_catalog(&mut self, other: &Self) -> Result<()> {
        let mut documents = Documents::from_registry(self)?;
        // What arrives new is put before anything merges, so a member that
        // references it on one side and states it inline on the other is
        // compared against it whatever category it belongs to.
        let mut folding = Vec::new();
        for category in [
            FixCategory::Components,
            FixCategory::Groups,
            FixCategory::Messages,
        ] {
            for field in other.catalog.iter(category) {
                if documents.get(category, field.name()).is_none() {
                    self.fold_document(&mut documents, category, field)?;
                } else {
                    folding.push((category, field));
                }
            }
        }
        for (category, field) in folding {
            self.fold_document(&mut documents, category, field)?;
        }
        self.resolve_catalog(documents.raw)?;
        self.validate_catalog()
    }
}
