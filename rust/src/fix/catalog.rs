//! Named FIX definitions beside the scalar wire-field indexes.

use std::collections::HashMap;
use std::sync::Arc;

use smol_str::SmolStr;

use super::group_plan::GroupPlan;
use super::registry::name_digest;
use super::{FixBranch, FixId, FixRegistry, MsgType};
use crate::{DataType, Error, Field, FixCategory, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Definition {
    pub category: FixCategory,
    pub branch: FixBranch,
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
    counters: HashMap<FixId, Option<usize>>,
    message_codes: HashMap<u32, HashMap<SmolStr, Option<usize>>>,
    message_aliases: HashMap<u64, MessageAlias>,
}

impl PartialEq for Catalog {
    fn eq(&self, other: &Self) -> bool {
        self.all().eq(other.all())
    }
}
impl Eq for Catalog {}

impl Catalog {
    fn key(category: FixCategory, branch: &FixBranch, name: &str) -> (FixCategory, u64) {
        (category, name_digest(branch, name, 0x4341_5441_4c4f_4753))
    }

    fn position(&self, category: FixCategory, branch: &FixBranch, name: &str) -> Option<usize> {
        let position = *self.names.get(&Self::key(category, branch, name))?;
        let entry = self.entries.get(position)?;
        (entry.branch.has_identity(branch) && crate::types::folds_equal(entry.field.name(), name))
            .then_some(position)
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
            if let Some(id) = entry
                .field
                .as_fix()
                .counter()
                .ok()
                .flatten()
                .and_then(|tag| FixId::from_parts(&entry.branch, tag).ok())
            {
                self.counters
                    .entry(id)
                    .and_modify(|held| *held = None)
                    .or_insert(Some(position));
            }
        }
    }

    fn index_messages(&mut self) {
        self.message_codes.clear();
        for (position, entry) in self.entries.iter().enumerate() {
            if let Some(message) = entry.field.message() {
                self.message_codes
                    .entry(entry.branch.digest())
                    .or_default()
                    .entry(SmolStr::new(message.as_str()))
                    .and_modify(|held| *held = None)
                    .or_insert(Some(position));
            }
        }
    }

    fn message_position(&self, branch: &FixBranch, spelling: &str) -> Option<Option<usize>> {
        let codes = self.message_codes.get(&branch.digest())?;
        if let Some(position) = codes.get(spelling) {
            return Some(*position);
        }
        if let Some(position) = self.position(FixCategory::Messages, branch, spelling) {
            return Some(Some(position));
        }
        let alias = self.message_aliases.get(&name_digest(
            &FixBranch::STANDARD,
            spelling,
            0x4d53_475f_414c_4941,
        ))?;
        if !crate::types::folds_equal(&alias.spelling, spelling) {
            return None;
        }
        let code = alias.code.as_ref()?;
        codes.get(code).copied()
    }

    fn index_message_aliases(&mut self, field: Option<&Field>) {
        self.message_aliases.clear();
        let Some(field) = field else {
            return;
        };
        for code in field.as_fix().codes().filter_map(Result::ok) {
            for spelling in std::iter::once(code.name()).chain(code.aliases()) {
                let digest = name_digest(&FixBranch::STANDARD, spelling, 0x4d53_475f_414c_4941);
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
    }

    pub fn all(&self) -> impl Iterator<Item = &Definition> {
        self.order.iter().map(|position| &self.entries[*position])
    }

    pub fn insert(
        &mut self,
        category: FixCategory,
        branch: FixBranch,
        field: Field,
    ) -> Result<Option<Field>> {
        if category == FixCategory::Groups {
            let tag = field
                .as_fix()
                .counter()?
                .ok_or_else(|| Error::absent("fix:counter", field.name()))?;
            FixId::from_parts(&branch, tag)?;
        }
        let key = Self::key(category, &branch, field.name());
        if let Some(position) = self.names.get(&key).copied() {
            if self.position(category, &branch, field.name()) != Some(position) {
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
        self.entries.push(Definition {
            category,
            branch,
            field,
        });
        self.names.insert(key, position);
        self.order.push(position);
        self.order.sort_by(|left, right| {
            let left = &self.entries[*left];
            let right = &self.entries[*right];
            (left.category, left.branch.name(), left.field.name()).cmp(&(
                right.category,
                right.branch.name(),
                right.field.name(),
            ))
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
            self.names.insert(
                Self::key(entry.category, &entry.branch, entry.field.name()),
                position,
            );
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
    /// Borrows the unique message definition named by an exact wire code,
    /// folded canonical name, or tag 35's enum alias, in that order.
    ///
    /// An omitted branch tries the standard namespace first, then named
    /// branches in canonical order. Ambiguous wire codes return no match.
    pub fn get_msgtype(&self, spelling: &str, branch: Option<&FixBranch>) -> Option<&MsgType> {
        let position = if let Some(branch) = branch {
            self.get_branch_by_digest(branch.digest_signed())
                .filter(|held| held.has_identity(branch))?;
            self.catalog.message_position(branch, spelling)?
        } else {
            let mut found = None;
            for branch in self.branch_values() {
                if let Some(position) = self.catalog.message_position(branch, spelling) {
                    found = Some(position);
                    break;
                }
            }
            found?
        }?;
        self.catalog.entries[position].field.message()
    }

    /// Borrows a unique registry-owned message type, raising absence or ambiguity.
    pub fn msgtype(&self, spelling: &str, branch: Option<&FixBranch>) -> Result<&MsgType> {
        self.get_msgtype(spelling, branch).ok_or_else(|| {
            Error::absent(
                "one unambiguous FIX message type",
                format_args!("{spelling:?} in branch {:?}", branch.map(FixBranch::name)),
            )
        })
    }

    /// Iterates registered message singletons in branch/name order.
    pub fn msgtypes(&self) -> impl Iterator<Item = &MsgType> {
        self.catalog
            .order
            .iter()
            .filter_map(|position| self.catalog.entries[*position].field.message())
    }

    /// Borrows a message singleton by its stable branch/name iteration position.
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

    /// Resolves one category's folded name within its branch namespace.
    pub fn get_definition(
        &self,
        category: FixCategory,
        name: &str,
        branch: Option<&FixBranch>,
    ) -> Option<&Field> {
        if category == FixCategory::Fields {
            return self.get_field_by_name(name, branch);
        }
        let position = if let Some(branch) = branch {
            self.catalog.position(category, branch, name)
        } else {
            self.catalog
                .position(category, &FixBranch::STANDARD, name)
                .or_else(|| {
                    self.branch_values()
                        .find_map(|branch| self.catalog.position(category, branch, name))
                })
        }?;
        Some(self.catalog.entries[position].field.as_field())
    }

    /// Resolves a category name, reporting absence with its category and branch.
    pub fn definition(
        &self,
        category: FixCategory,
        name: &str,
        branch: Option<&FixBranch>,
    ) -> Result<&Field> {
        self.get_definition(category, name, branch).ok_or_else(|| {
            Error::absent(
                category.as_str(),
                format_args!("{name:?} in branch {:?}", branch.map(FixBranch::name)),
            )
        })
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
    /// follow tag order; named positions follow branch and canonical name order.
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
                let branch = field.as_fix().branch()?;
                let component = self.definition(FixCategory::Components, &name, Some(&branch))?;
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
        let branch = field.as_fix().branch()?;
        // An update keeps the identity the definition already has: a derived
        // tag is stable for the definition, not for the registry it was
        // derived against.
        let held = self
            .get_definition(category, field.name(), Some(&branch))
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
        self.check_branch(&branch)?;
        if let Some(stored) = self.get_definition(category, field.name(), Some(&branch)) {
            if stored.name().eq_ignore_ascii_case(field.name()) {
                field.set_name(stored.name());
            }
            let refresh = super::registry::metadata_only_change(stored, &field);
            if refresh {
                self.validate_catalog()?;
            }
            let mut staged = self.clone();
            let prior = staged.catalog.insert(category, branch, field)?;
            if refresh {
                staged.refresh_references()?;
            }
            staged.validate_catalog()?;
            *self = staged;
            return Ok(prior);
        }
        let prior = self.catalog.insert(category, branch.clone(), field)?;
        self.ensure_branch(branch);
        Ok(prior)
    }

    /// Creates a definition, refusing an existing name or wire identity atomically.
    pub fn create_definition(&mut self, category: FixCategory, field: Field) -> Result<()> {
        let branch = field.as_fix().branch()?;
        let existing = if category == FixCategory::Fields {
            // Creation reserves canonical identities only. Another field's
            // alias may name this spelling until its canonical owner arrives.
            self.canonical_position_by_id(super::registry::canonical_id(&field)?)
                .is_some()
                || self
                    .canonical_position_by_name(&branch, field.name())
                    .is_some()
        } else {
            self.get_definition(category, field.name(), Some(&branch))
                .is_some()
        };
        if existing {
            return Err(Error::conflict(
                "absent FIX definition",
                "existing FIX definition",
                field.name(),
            ));
        }
        self.insert_definition(category, field)?;
        Ok(())
    }

    /// Replaces an existing definition and returns its previous value.
    pub fn update_definition(&mut self, category: FixCategory, field: Field) -> Result<Field> {
        let branch = field.as_fix().branch()?;
        let stored = self.definition(category, field.name(), Some(&branch))?;
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

    /// Removes a definition only when every remaining reference stays valid.
    pub fn remove_definition(
        &mut self,
        category: FixCategory,
        name: &str,
        branch: Option<&FixBranch>,
    ) -> Result<Option<Field>> {
        let Some(field) = self.get_definition(category, name, branch) else {
            return Ok(None);
        };
        let branch = field.as_fix().branch()?;
        let name = field.name().to_owned();
        let mut staged = self.clone();
        let removed = if category == FixCategory::Fields {
            let id = super::registry::canonical_id(field)?;
            staged.remove_resolved(id)
        } else {
            let position = staged
                .catalog
                .position(category, &branch, &name)
                .ok_or_else(|| Error::absent(category.as_str(), &name))?;
            Some(staged.catalog.remove(position))
        };
        staged.validate_catalog()?;
        staged.retain_populated_branches();
        staged.refresh_msgtype_aliases();
        *self = staged;
        Ok(removed)
    }

    /// The unique group using one counter; ambiguous contexts return no match.
    pub fn get_group_by_counter(&self, id: FixId) -> Option<&Field> {
        let position = self.catalog.counters.get(&id).copied().flatten()?;
        Some(self.catalog.entries[position].field.as_field())
    }

    pub(super) fn get_group_plan_by_counter(&self, id: FixId) -> Option<&GroupPlan> {
        let position = self.catalog.counters.get(&id).copied().flatten()?;
        match &self.catalog.entries[position].field {
            DefinitionField::Group(_, plan) => Some(plan),
            _ => None,
        }
    }

    /// The unique group using a counter, reporting absence or ambiguity.
    pub fn group_by_counter(&self, id: FixId) -> Result<&Field> {
        self.get_group_by_counter(id)
            .ok_or_else(|| Error::absent("one unambiguous FIX group", id))
    }

    /// Whether anything in this registry already answers to `tag`.
    ///
    /// A published tag can never reach the derived block - a dialect's own
    /// tags stop at [`FixId::USER_TAG_MAX`] and this crate's at
    /// [`crate::CRATE_TAG_MAX`] - so the only thing that can occupy a slot is
    /// another derived definition. The scalar index is asked anyway, because
    /// a dictionary read from a store is whatever that store held.
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
        match category {
            FixCategory::Fields if field.dtype().is_nested() => {
                return Err(invalid(field, "a scalar datatype"));
            }
            FixCategory::Messages | FixCategory::Components
                if !matches!(field.dtype(), DataType::Struct(_)) =>
            {
                return Err(invalid(field, "a Struct datatype"));
            }
            FixCategory::Groups => {
                if !matches!(field.dtype(), DataType::List(item) | DataType::LargeList(item) if !item.is_nullable() && matches!(item.dtype(), DataType::Struct(_)))
                {
                    return Err(invalid(field, "a List of non-null Struct occurrences"));
                }
                let tag = field
                    .as_fix()
                    .counter()?
                    .ok_or_else(|| Error::absent("fix:counter", field.name()))?;
                let branch = field.as_fix().branch()?;
                let id = FixId::from_parts(&branch, tag)?;
                let counter = self.field_by_id(id)?;
                if counter.dtype() != &DataType::Int32 {
                    return Err(invalid(counter, "an int32 repeating-group counter"));
                }
                if let Some(component) = field.as_fix().component() {
                    if let DataType::List(item) | DataType::LargeList(item) = field.dtype() {
                        if !item
                            .as_fix()
                            .component()
                            .is_some_and(|name| crate::types::folds_equal(name, component))
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
            _ => {}
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
            let branch = view.branch()?;
            let target = self.definition(category, name, Some(&branch))?;
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
            if category == FixCategory::Fields && view.id()? != target.as_fix().id()? {
                return Err(Error::conflict(
                    "the referenced FIX field identity",
                    "a different field identity",
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
            if let Some(held) = derived.insert(tag, name).filter(|held| *held != name) {
                return Err(Error::conflict(
                    "one FIX definition per derived tag",
                    "two definitions on one tag",
                    format_args!("{held:?} and {name:?} both hold {tag}"),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn merge_catalog(&mut self, other: &Self) -> Result<()> {
        // The source has been validated against its own fields. Resolve its
        // references against the merged fields before combining the catalogs;
        // retained target references must still reject structural replacements.
        let stored = std::mem::replace(&mut self.catalog, other.catalog.clone());
        let refreshed = self.refresh_references();
        let incoming = std::mem::replace(&mut self.catalog, stored);
        refreshed?;
        for category in [
            FixCategory::Components,
            FixCategory::Groups,
            FixCategory::Messages,
        ] {
            for field in incoming.iter(category) {
                let branch = field.as_fix().branch()?;
                self.check_branch(&branch)?;
                let mut field = field.clone();
                if let Some(stored) = self.get_definition(category, field.name(), Some(&branch)) {
                    if stored.name().eq_ignore_ascii_case(field.name()) {
                        field.set_name(stored.name());
                    }
                }
                self.catalog.insert(category, branch.clone(), field)?;
                self.ensure_branch(branch);
            }
        }
        self.validate_catalog()?;
        self.refresh_msgtype_aliases();
        Ok(())
    }

    pub(super) fn has_branch_definitions(&self, branch: &FixBranch) -> bool {
        self.iter().any(|field| {
            field
                .as_fix()
                .branch()
                .is_ok_and(|held| held.has_identity(branch))
        }) || self
            .catalog
            .all()
            .any(|entry| entry.branch.has_identity(branch))
    }
}
