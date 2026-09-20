//! A registry-owned message definition, carrying its native Struct Field.

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::catalog::occurrence_of;
use super::group_plan::GroupPlan;
use super::registry::name_digest;
use crate::{DataType, Error, Field, FixMsg, Result, Scalar};

/// The seed the child index folds names under.
const CHILD_DOMAIN: u64 = 0x4d53_475f_4348_4c44;

/// A named FIX message schema and its exact wire code.
///
/// Instances are owned by [`super::FixRegistry`]. Borrow one through
/// [`super::FixRegistry::msgtype`]; construction and mutation happen through
/// the registry's message-definition methods.
///
/// ```
/// use yggdryl::{DataType, FixRegistry, StructType};
///
/// let mut registry = FixRegistry::new();
/// let mut field = DataType::from(StructType::from_fields([])?).required_field("Order");
/// field.as_fix_mut().set_msgtype("D")?;
/// registry.insert(field)?;
/// let message = registry.msgtype("D")?;
/// assert_eq!(message.name(), "Order");
/// assert_eq!(message.as_str(), "D");
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct MsgType {
    field: Field,
    groups: super::registry::FixMap<i32, Option<GroupOccurrence>>,
    /// The direct scalar children carrying a tag, by folded name: what a
    /// key the dictionary does not name resolves against, answered by one
    /// probe rather than by a walk of a wide message's three hundred
    /// children for each of a bridge row's dozens of unknown keys.
    children: HashMap<u64, usize>,
    /// At most one entry per direct scalar member. Positions address our
    /// definition; tags address a message's independently ordered row.
    identifiers: Vec<(usize, Option<i32>)>,
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

/// The wire value one declared message type spelling carries.
///
/// A message type is alphanumeric and never holds a space, so a spelling that
/// holds one is a wire value and a qualifier: `AR Inbound` and `AR Outbound`
/// are tag 35 `AR` in the two directions, `J Report` is `J` used as a report,
/// `c SDR` and `c SLR` are `c` used two ways. The value is the first word and
/// the qualifier is what the dialect calls that use of it, so the qualified
/// spelling reaches the type as a spelling of it and never declares a second
/// one.
///
/// The one owner of that split: every intake taking a spelling a person or a
/// configuration wrote resolves it here, so `6 Inbound` is tag 35 `6`
/// wherever it arrives.
///
/// The full first word is retained without a datatype width limit.
pub(super) fn wire_value(spelling: &str) -> &str {
    spelling.split_whitespace().next().unwrap_or(spelling)
}

pub(super) fn validate_code(value: &str) -> Result<()> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(Error::InvalidMetadataValue {
            key: "FIX:msgtype".into(),
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
        crate::hashing::stable_hash_of(self)
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
            .ok_or_else(|| Error::absent("FIX:msgtype", field.name()))?;
        validate_code(code)?;
        let tags = field
            .fields()
            .iter()
            .map(|child| {
                if child.dtype().is_nested() {
                    Ok(None)
                } else {
                    child.as_fix().tag()
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let identifiers = field
            .as_fix()
            .compiled_identifier_positions()?
            .into_iter()
            .map(|index| {
                let tag = tags[index]
                    .filter(|tag| tags.iter().filter(|held| **held == Some(*tag)).count() == 1);
                (index, tag)
            })
            .collect();
        let mut groups = super::registry::FixMap::default();
        Self::index_groups(&field, &mut Vec::new(), &mut groups, &mut HashMap::new())?;
        // Only a scalar child carrying a tag answers for a key: a nested
        // level is addressed by its own located key and never by a leaf,
        // and a child no tag identifies explains a key no better than the
        // key explains itself. The first of two children folding to one
        // name keeps the index, exactly as a walk would answer it first.
        let mut children = HashMap::new();
        for (index, child) in field.fields().iter().enumerate() {
            if tags[index].is_none() {
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
            identifiers,
        })
    }

    /// Borrows this component's non-null identifiers at the message's own level.
    ///
    /// Selection is compiled at registration. Iteration allocates nothing,
    /// parses no metadata and does not descend into repeating groups. The
    /// field is the declaration's member, beside the message's actual value.
    /// An exact member name wins; a renamed child is selected by tag only
    /// when that tag is unique in both the definition and the message.
    /// Ambiguous unnamed tags contribute no identifier.
    ///
    /// ```
    /// use std::sync::Arc;
    /// use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, StructType};
    /// let mut id = DataType::utf8().nullable_field("clordid");
    /// id.as_fix_mut().set_tag(11)?;
    /// let mut field = DataType::from(StructType::from_fields([id])?).required_field("order");
    /// field.as_fix_mut().set_msgtype("D")?;
    /// field.as_fix_mut().set_identifiers(["11"])?;
    /// let mut registry = FixRegistry::new();
    /// registry.insert(field.clone())?;
    /// let registry = Arc::new(registry);
    /// let message = FixMsg::with_registry(
    ///     Arc::clone(&registry), field, Scalar::from_sequence([Scalar::from("O-1")]),
    /// )?;
    /// let values: Vec<_> = registry.msgtype("D")?.identifier_values(&message)
    ///     .map(|(field, value)| (field.name().to_owned(), value.as_str().map(str::to_owned)))
    ///     .collect();
    /// assert_eq!(values, [("clordid".to_owned(), Some("O-1".to_owned()))]);
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    pub fn identifier_values<'a>(
        &'a self,
        message: &'a FixMsg,
    ) -> impl Iterator<Item = (&'a Field, Scalar)> {
        self.identifiers.iter().filter_map(move |(position, tag)| {
            let field = &self.field.fields()[*position];
            // The row is read first, because several fields may share a tag
            // and the member's own name is what tells them apart; an
            // identifier the message lifted out of its row - `ClOrdID(11)`,
            // `OrderID(37)` and the rest are held typed - answers under its
            // tag instead, which is why the value is the message's to hand
            // over rather than the row's to lend.
            let value = message
                .as_field()
                .index_of(field.name())
                .or_else(|| tag.and_then(|tag| message.unique_index_of_tag(tag)))
                .and_then(|index| message.as_value().get(index).cloned())
                .or_else(|| tag.and_then(|tag| message.get_by_tag(tag)))
                .filter(|value| !value.is_null())?;
            Some((field, value))
        })
    }

    /// Canonical identifier text in ascending member-name order.
    ///
    /// Enrichment stores this Map; lifecycle uses the same answer when the
    /// message states none. An identifier whose value will not spell text is
    /// left out, so one unreadable member costs that member and never the
    /// message.
    pub(super) fn identifier_mapping(&self, message: &FixMsg) -> Result<Scalar> {
        let mut entries: Vec<(Scalar, Scalar)> = self
            .identifier_values(message)
            // An identifier whose value will not spell text names nothing, so
            // it is left out rather than taken as the empty name or allowed
            // to refuse the message around it: the arrival record still
            // carries the bytes, and every identifier that does spell text
            // still reaches the map.
            .filter_map(|(field, value)| {
                let held = DataType::utf8().scalar(value).ok()?;
                Some((Scalar::from(field.name()), held))
            })
            .collect();
        // `schema::fitted` trusts a matching Map datatype ID. The
        // producer must therefore establish sortedness before storage.
        entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        Scalar::from_mapping(entries)
    }

    /// The direct scalar child `key` names under the crate's name fold, and
    /// the tag it carries.
    ///
    /// One probe of the index built when the message was registered,
    /// rechecked against the child it lands on, so a collision is a miss.
    pub(super) fn get_child_by_name(&self, key: &str) -> Option<(&Field, i32)> {
        let digest = name_digest(key, CHILD_DOMAIN);
        let child = self.field.fields().get(*self.children.get(&digest)?)?;
        if !crate::folds_equal(child.name(), key) {
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

    /// Borrows the unique repeating group the counter `tag` opens in this
    /// message.
    ///
    /// The tag is the counter's, as it is on
    /// [`FixRegistry::get_field_by_counter`](crate::FixRegistry::get_field_by_counter).
    /// Its path is compiled at registration; repeated contexts are ambiguous.
    pub fn get_group_by_tag(&self, tag: i32) -> Option<&Field> {
        let path = &self.groups.get(&tag)?.as_ref()?.path;
        let mut field = &self.field;
        for step in path {
            field = match step {
                GroupStep::Child(index) => field.fields().get(*index)?,
                GroupStep::Item => occurrence_of(field)?,
            };
        }
        Some(field)
    }

    pub(super) fn has_group_tag(&self, tag: i32) -> bool {
        self.groups.contains_key(&tag)
    }

    pub(super) fn get_group_plan_by_tag(&self, tag: i32) -> Option<&GroupPlan> {
        let plan = &self.groups.get(&tag)?.as_ref()?.plan;
        (!matches!(plan.field().dtype(), DataType::Mapping(_))).then_some(plan)
    }

    fn index_groups(
        field: &Field,
        path: &mut Vec<GroupStep>,
        groups: &mut super::registry::FixMap<i32, Option<GroupOccurrence>>,
        plans: &mut HashMap<Vec<GroupStep>, Arc<GroupPlan>>,
    ) -> Result<()> {
        if path.len() > 64 {
            return Err(Error::InvalidRecord {
                path: field.name().into(),
                reason: "message schemas are nested at most 64 levels".into(),
            });
        }
        if let Some(item) = occurrence_of(field) {
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
