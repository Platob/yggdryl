//! A registry-owned message definition, carrying its native Struct Field.

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::group_plan::GroupPlan;
use super::registry::name_digest;
use crate::{DataType, Error, Field, Result};

/// The seed the child index folds names under.
const CHILD_DOMAIN: u64 = 0x4d53_475f_4348_4c44;

/// A named FIX message schema and its exact wire code.
///
/// Instances are owned by [`super::FixRegistry`]. Borrow one through
/// [`super::FixRegistry::msgtype`]; construction and mutation happen through
/// the registry's message-definition methods.
///
/// ```
/// use yggdryl::{DataType, FixCategory, FixRegistry};
///
/// let mut registry = FixRegistry::new();
/// let mut field = DataType::from_fields([])?.required_field("Order");
/// field.as_fix_mut().set_msgtype("D")?;
/// registry.create_definition(FixCategory::Components, field)?;
/// let message = registry.msgtype("D")?;
/// assert_eq!(message.name(), "Order");
/// assert_eq!(message.as_str(), "D");
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct MsgType {
    field: Field,
    groups: HashMap<i32, Option<GroupOccurrence>>,
    /// The direct scalar children carrying a tag, by folded name: what a
    /// key the dictionary does not name resolves against, answered by one
    /// probe rather than by a walk of a wide message's three hundred
    /// children for each of a bridge row's dozens of unknown keys.
    children: HashMap<u64, usize>,
}

#[derive(Clone, Debug)]
struct GroupOccurrence {
    path: Vec<GroupStep>,
    plan: Arc<GroupPlan>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
enum GroupStep {
    Child(usize),
    Item,
}

pub(super) fn validate_code(value: &str) -> Result<()> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(Error::InvalidMetadataValue {
            key: "fix:msgtype".into(),
            reason: "expected nonempty message-code text without control characters".into(),
        });
    }
    Ok(())
}

impl MsgType {
    /// The deterministic hash of this message's native Field definition.
    /// Uses one allocation for the shared XXH3 state, independent of schema size.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    pub(super) fn from_field(field: Field) -> Result<Self> {
        if field.is_nullable() || !matches!(field.dtype(), DataType::Struct(_)) {
            return Err(Error::InvalidRecord {
                path: field.name().into(),
                reason: crate::text::expected_got("a non-null Struct message definition", field),
            });
        }
        let code = field
            .as_fix()
            .msgtype()
            .ok_or_else(|| Error::absent("fix:msgtype", field.name()))?;
        validate_code(code)?;
        let mut groups = HashMap::new();
        Self::index_groups(&field, &mut Vec::new(), &mut groups, &mut HashMap::new())?;
        // Only a scalar child carrying a tag answers for a key: a nested
        // level is addressed by its own located key and never by a leaf,
        // and a child no tag identifies explains a key no better than the
        // key explains itself. The first of two children folding to one
        // name keeps the index, exactly as a walk would answer it first.
        let mut children = HashMap::new();
        for (index, child) in field.fields().iter().enumerate() {
            if child.dtype().is_nested() || child.as_fix().tag().ok().flatten().is_none() {
                continue;
            }
            children
                .entry(name_digest(child.name(), CHILD_DOMAIN))
                .or_insert(index);
        }
        Ok(Self {
            field,
            groups,
            children,
        })
    }

    /// The direct scalar child `key` names under the crate's name fold, and
    /// the tag it carries.
    ///
    /// One probe of the index built when the message was registered,
    /// rechecked against the child it lands on, so a collision is a miss.
    pub(super) fn get_child_by_name(&self, key: &str) -> Option<(&Field, i32)> {
        let digest = name_digest(key, CHILD_DOMAIN);
        let child = self.field.fields().get(*self.children.get(&digest)?)?;
        if !crate::types::folds_equal(child.name(), key) {
            return None;
        }
        Some((child, child.as_fix().tag().ok()??))
    }

    /// Borrows the complete native message schema without allocating.
    pub const fn as_field(&self) -> &Field {
        &self.field
    }

    /// The canonical registry name.
    pub fn name(&self) -> &str {
        self.field.name()
    }

    /// The exact wire code, preserving case and all non-control text.
    pub fn as_str(&self) -> &str {
        self.field
            .as_fix()
            .msgtype()
            .expect("a registry message has a validated wire code")
    }

    /// Borrows the unique repeating group using a counter in this message.
    /// Its path is compiled at registration; repeated contexts are ambiguous.
    pub fn get_group_by_counter(&self, tag: i32) -> Option<&Field> {
        let path = &self.groups.get(&tag)?.as_ref()?.path;
        let mut field = &self.field;
        for step in path {
            field = match step {
                GroupStep::Child(index) => field.fields().get(*index)?,
                GroupStep::Item => match field.dtype() {
                    DataType::List(item) | DataType::LargeList(item) => item,
                    _ => return None,
                },
            };
        }
        Some(field)
    }

    pub(super) fn has_group_counter(&self, tag: i32) -> bool {
        self.groups.contains_key(&tag)
    }

    pub(super) fn get_group_plan_by_counter(&self, tag: i32) -> Option<&GroupPlan> {
        Some(&self.groups.get(&tag)?.as_ref()?.plan)
    }

    fn index_groups(
        field: &Field,
        path: &mut Vec<GroupStep>,
        groups: &mut HashMap<i32, Option<GroupOccurrence>>,
        plans: &mut HashMap<Vec<GroupStep>, Arc<GroupPlan>>,
    ) -> Result<()> {
        if path.len() > 64 {
            return Err(Error::InvalidRecord {
                path: field.name().into(),
                reason: "message schemas are nested at most 64 levels".into(),
            });
        }
        if let DataType::List(item) | DataType::LargeList(item) = field.dtype() {
            if let Some(tag) = field.as_fix().counter()? {
                let plan = match plans.remove(path) {
                    Some(plan) => plan,
                    None => Arc::new(GroupPlan::from_field(field)?),
                };
                for (nested_path, nested) in plan.nested_plans() {
                    let nested_path = path
                        .iter()
                        .copied()
                        .chain(std::iter::once(GroupStep::Item))
                        .chain(nested_path.iter().copied().map(GroupStep::Child))
                        .collect::<Vec<_>>();
                    plans.insert(nested_path, Arc::clone(nested));
                }
                groups
                    .entry(tag)
                    .and_modify(|held| *held = None)
                    .or_insert_with(|| {
                        Some(GroupOccurrence {
                            path: path.clone(),
                            plan,
                        })
                    });
            }
            path.push(GroupStep::Item);
            Self::index_groups(item, path, groups, plans)?;
            path.pop();
        } else {
            for (index, child) in field.fields().iter().enumerate() {
                path.push(GroupStep::Child(index));
                Self::index_groups(child, path, groups, plans)?;
                path.pop();
            }
        }
        Ok(())
    }

    pub(super) fn into_field(self) -> Field {
        self.field
    }
}

impl PartialEq for MsgType {
    fn eq(&self, other: &Self) -> bool {
        self.field == other.field
    }
}
impl Eq for MsgType {}
impl Ord for MsgType {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.field.cmp(&other.field)
    }
}
impl PartialOrd for MsgType {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Hash for MsgType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.field.hash(state);
    }
}

impl AsRef<Field> for MsgType {
    fn as_ref(&self) -> &Field {
        self.as_field()
    }
}

impl fmt::Display for MsgType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
