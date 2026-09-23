//! A message restated under the dictionary its registry holds.
//!
//! A capture holds what each session spoke: a FIX 4.2 report states its
//! fill as `LastShares`, its broker as `ExecBroker(76)`, its capacity as
//! `Rule80A(47)`, and a partial fill as `ExecType(150)` `1` - four things the
//! newest specification spells as `LastQty`, a `Parties` occurrence,
//! `OrderCapacity(528)` and `Trade`. Every consumer then restates them
//! independently, which is how two systems come to disagree about one
//! message. This pass restates a message once, from what the dictionary
//! itself says: the registry's field for every tag, the aliases that reach
//! it, the [`FIX:replacements`](super::replacements) a registry states on a
//! field of its own, and the [retirements](super::retired) the specification
//! states of a tag. The two are one kind of rule applied one way: the same
//! first match, the same writes, the same checks. A field's own document
//! wins whole over the specification's table, so nothing here decides
//! between the two per entry.
//!
//! # The row only
//!
//! The entries are what arrived and are carried through untouched, so a
//! restated message re-emits the received line byte for byte, exactly as an
//! [enriched](super::enrich) one does. It is idempotent for the same reason
//! enrichment is: a stated current value is never overwritten, so what one
//! pass wrote is what the next pass finds stated.
//!
//! # One level at a time
//!
//! The root is a level, and so is every occurrence of every repeating group
//! it holds, to any depth. Each level is canonicalized first - every child
//! the registry knows re-expressed under the registry's field, children that
//! reach one field merged into the most complete one - and then restated:
//! each child whose field carries replacement rules, in ascending tag order,
//! applies the first rule whose conditions its held value meets. A rule
//! applies all-or-nothing: every field it would fill is computed and
//! checked, and one target that cannot take its value blocks the whole
//! rule. A target takes a value when it is absent, null, or already equal;
//! anything else is a value the message stated, and what the message stated
//! stands.

use std::borrow::Cow;
use std::cell::RefCell;
use std::sync::Arc;

use smol_str::SmolStr;

use super::build::{stated as stated_field, typed_spelling};
use super::entry::wire_text;
use super::msg::FixMsg;
use super::registry::{FixMap, name_key};
use super::retired::{self, Fill, Part, Rule, When};
use super::schema::{item_fields, same_shape, shape_digest, tag_and_counter};
use super::{FixRegistry, occurrence_name};
use crate::expression::{Bound, Term};
use crate::{DataType, Field, Plan, Result, Scalar, StructType};

/// One level of the row: the root, or one occurrence of a repeating group.
///
/// Held unpacked while the pass works on it - a group's occurrences as
/// levels of their own rather than as the Serie value they pack into - so a
/// member written into one occurrence is an ordinary child write, and the
/// Serie is rebuilt once at the end as the union of what its occurrences
/// hold, exactly as the builder rebuilds one.
#[derive(Clone, Default)]
struct Level {
    children: Vec<Child>,
}

/// One child of a level.
#[derive(Clone)]
enum Child {
    /// A scalar child, or a nested value no repeating group declares, which
    /// is kept exactly as it is.
    Flat(Field, Scalar),
    /// A repeating group: its Serie field, and each occurrence unpacked -
    /// `None` where the row states no occurrence at that index.
    Group(Field, Vec<Option<Level>>),
}

/// What one message's root is to the pass, read without unpacking it.
enum Shape {
    /// The message the pass would answer: nothing to rename, merge, retype
    /// or replace.
    Canonical,
    /// Stated under the dictionary's own fields, with a rule at the root
    /// whose condition holds: the rule's writes are owed and nothing else.
    Ruled,
    /// Rebuilt whole.
    Rebuilt,
}

/// What canonicalization decided for one child that reaches a registry
/// field.
enum Decision {
    /// Re-expressed under the registry's field, holding this value.
    Keep(Scalar),
    /// Merged into the kept child: null, or equal to what it holds.
    Drop,
    /// Stated and disagreeing with the kept child, so left as it arrived.
    Untouched,
}

/// The value one rule is restating: what it holds and its tag.
struct Source<'rule> {
    value: &'rule Scalar,
    tag: i32,
}

/// One write a rule plans: replaced where `at` names a child, appended
/// otherwise.
struct FieldWrite {
    at: Option<usize>,
    field: Field,
    value: Scalar,
}

/// One target a rule fills, planned before anything is written so the rule
/// applies all-or-nothing.
enum Write {
    Field(FieldWrite),
    /// One occurrence of a repeating group: merged into the occurrence at
    /// `occurrence`, appended when there is none; the Serie itself created
    /// from `serie` when the level holds none. The counter child is set to
    /// the count the group then has.
    Group {
        at: Option<usize>,
        serie: Option<Field>,
        occurrence: Option<usize>,
        members: Vec<Write>,
        counter: FieldWrite,
    },
}

impl Level {
    /// The level one Struct's fields and values make, groups unpacked.
    fn unpack(fields: &[Field], values: &[Scalar]) -> Self {
        Self {
            children: fields
                .iter()
                .zip(values)
                .map(|(field, value)| Child::unpack(field.clone(), value.clone()))
                .collect(),
        }
    }

    /// The fields and values this level packs back into, every group a Serie
    /// of the union of its occurrences' members.
    fn pack(self) -> Result<(Vec<Field>, Vec<Scalar>)> {
        let mut fields = Vec::with_capacity(self.children.len());
        let mut values = Vec::with_capacity(self.children.len());
        for child in self.children {
            let (field, value) = match child {
                Child::Flat(field, value) => (field, value),
                Child::Group(serie, occurrences) => pack_group(serie, occurrences)?,
            };
            fields.push(field);
            values.push(value);
        }
        Ok((fields, values))
    }

    /// The scalar child carrying `tag`, else the one `name` spells.
    fn position_of_field(&self, tag: Option<i32>, name: &str) -> Option<usize> {
        let by_tag = tag.and_then(|tag| {
            self.children.iter().position(|child| {
                matches!(child, Child::Flat(field, _) if field.as_fix().tag().ok().flatten() == Some(tag))
            })
        });
        by_tag.or_else(|| {
            self.children
                .iter()
                .position(|child| matches!(child, Child::Flat(field, _) if field.name() == name))
        })
    }

    /// The group child headed by `counter`, else the one named `name`.
    ///
    /// A declared group stating no occurrence - a null Serie a schema
    /// declared - is that group too: a fill into it opens it rather than
    /// standing a second child of the name beside it.
    fn position_of_group(&self, counter: i32, name: &str) -> Option<usize> {
        let is_group = |child: &Child| match child {
            Child::Group(..) => true,
            Child::Flat(field, value) => value.is_null() && item_fields(field).is_some(),
        };
        let by_counter = self.children.iter().position(|child| {
            is_group(child) && child.field().as_fix().counter().ok().flatten() == Some(counter)
        });
        by_counter.or_else(|| {
            self.children
                .iter()
                .position(|child| is_group(child) && crate::folds_equal(child.field().name(), name))
        })
    }

    /// Whether some child of any shape is spelled `name`: a scalar write
    /// beside a group of one name would make two children of it.
    fn names(&self, name: &str) -> bool {
        self.children.iter().any(|child| match child {
            Child::Flat(field, _) | Child::Group(field, _) => field.name() == name,
        })
    }

    /// The value the scalar child at `at` holds.
    fn value_at(&self, at: usize) -> Option<&Scalar> {
        match self.children.get(at)? {
            Child::Flat(_, value) => Some(value),
            Child::Group(..) => None,
        }
    }

    fn write_field(&mut self, write: FieldWrite) {
        let child = Child::Flat(write.field, write.value);
        match write.at {
            Some(at) => self.children[at] = child,
            None => self.children.push(child),
        }
    }

    /// Lands one planned write, members and counter included.
    fn apply(&mut self, write: Write) {
        match write {
            Write::Field(write) => self.write_field(write),
            Write::Group {
                at,
                serie,
                occurrence,
                members,
                counter,
            } => {
                let at = match (at, serie) {
                    (Some(at), _) => at,
                    (None, Some(serie)) => {
                        self.children.push(Child::Group(serie, Vec::new()));
                        self.children.len() - 1
                    }
                    (None, None) => return,
                };
                if let Child::Flat(declared, _) = &self.children[at] {
                    // A declared group stating no occurrence opens as one.
                    let declared = declared.clone();
                    self.children[at] = Child::Group(declared, Vec::new());
                }
                let Child::Group(_, occurrences) = &mut self.children[at] else {
                    return;
                };
                let occurrence = match occurrence {
                    Some(occurrence) => occurrence,
                    None => {
                        occurrences.push(Some(Level::default()));
                        occurrences.len() - 1
                    }
                };
                let target = occurrences[occurrence].get_or_insert_with(Level::default);
                for member in members {
                    target.apply(member);
                }
                self.write_field(counter);
            }
        }
    }

    /// Folds `other` into this level with this level's stated values leading.
    /// One logical child remains one child; repeating groups merge occurrence
    /// by occurrence, and an absent scalar is filled from the other statement.
    fn merge_from(&mut self, registry: &FixRegistry, other: Self) {
        // Each child's identity is resolved once, beside it, and indexed:
        // the child another statement names is found by a hash rather than
        // by comparing it against every child held.
        let mut tags: Vec<Option<i32>> = self
            .children
            .iter()
            .map(|held| child_tag(registry, held))
            .collect();
        let mut index = ChildIndex::of(&self.children, &tags);
        for child in other.children {
            let tag = child_tag(registry, &child);
            let Some(at) = index.position(&self.children, &tags, &child, tag) else {
                index.push(self.children.len(), &child, tag);
                self.children.push(child);
                tags.push(tag);
                continue;
            };
            match (&mut self.children[at], child) {
                (Child::Flat(field, value), Child::Flat(other_field, other_value)) => {
                    if value.is_null() && !other_value.is_null() {
                        // The child may now answer to another tag or name,
                        // so the index is read again from the children.
                        let moved = tags[at] != tag || field.name() != other_field.name();
                        *field = other_field;
                        *value = other_value;
                        tags[at] = tag;
                        if moved {
                            index = ChildIndex::of(&self.children, &tags);
                        }
                    }
                }
                (Child::Group(_, occurrences), Child::Group(_, other_occurrences)) => {
                    if occurrences.len() < other_occurrences.len() {
                        occurrences.resize_with(other_occurrences.len(), || None);
                    }
                    for (index, other) in other_occurrences.into_iter().enumerate() {
                        match (&mut occurrences[index], other) {
                            (Some(held), Some(other)) => held.merge_from(registry, other),
                            (slot @ None, Some(other)) => *slot = Some(other),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Collapses duplicates within one statement through the same leading
    /// merge used between statements.
    fn deduplicated(registry: &FixRegistry, level: Self) -> Self {
        let mut deduplicated = Self::default();
        deduplicated.merge_from(registry, level);
        for child in &mut deduplicated.children {
            if let Child::Group(_, occurrences) = child {
                for occurrence in occurrences.iter_mut().flatten() {
                    *occurrence = Self::deduplicated(registry, std::mem::take(occurrence));
                }
            }
        }
        deduplicated
    }

    /// Orders one merged level by its FIX identity. Tagged members lead in
    /// ascending tag order; untagged bridge fields follow by folded name.
    fn sort(&mut self, registry: &FixRegistry) {
        for child in &mut self.children {
            if let Child::Group(_, occurrences) = child {
                for occurrence in occurrences.iter_mut().flatten() {
                    occurrence.sort(registry);
                }
            }
        }
        // Keyed once per child rather than once per comparison: tagged
        // members by tag, a group after the scalar sharing its tag, and
        // untagged bridge fields after them by lowercased name. The sort is
        // stable, so members one key names keep their order.
        self.children
            .sort_by_cached_key(|child| match child_tag(registry, child) {
                Some(tag) => (false, tag, matches!(child, Child::Group(..)), String::new()),
                None => (true, 0, false, child.name().to_ascii_lowercase()),
            });
    }

    /// Brings every scalar counter in step with the merged Serie it counts.
    fn sync_group_counts(&mut self, registry: &FixRegistry) -> Result<()> {
        for child in &mut self.children {
            if let Child::Group(_, occurrences) = child {
                for occurrence in occurrences.iter_mut().flatten() {
                    occurrence.sync_group_counts(registry)?;
                }
            }
        }
        let groups: Vec<(i32, usize)> = self
            .children
            .iter()
            .filter_map(|child| match child {
                Child::Group(field, occurrences) => tag_and_counter(registry, field)
                    .1
                    .map(|counter| (counter, occurrences.len())),
                Child::Flat(..) => None,
            })
            .collect();
        for (counter, count) in groups {
            let Some(field) = registry.get_field_by_tag(counter).map(stated_field) else {
                continue;
            };
            let value = field.scalar(Scalar::from(i64::try_from(count).unwrap_or(i64::MAX)))?;
            match self.position_of_field(Some(counter), field.name()) {
                Some(at) => self.children[at] = Child::Flat(field, value),
                None => self.children.push(Child::Flat(field, value)),
            }
        }
        Ok(())
    }
}

/// The FIX identity one child states. A group is named by its counter; a
/// scalar by its tag. Shape remains part of the identity because the scalar
/// counter and the Serie it counts legitimately share one numeric tag.
fn child_tag(registry: &FixRegistry, child: &Child) -> Option<i32> {
    let field = child.field();
    let (tag, counter) = tag_and_counter(registry, field);
    match child {
        Child::Flat(..) => tag,
        Child::Group(..) => counter.or(tag),
    }
}

/// Where the first child stating each identity stands in one level, by the
/// keys [`same_child`] compares: shape and tag, and shape and folded name.
///
/// The folded name is keyed by [`name_key`], which folds ASCII exactly as
/// [`crate::folds_equal`] does; a level holding a name outside ASCII, or a
/// key two names share, is answered by the scan the index stands in for.
struct ChildIndex {
    tagged: FixMap<(bool, i32), usize>,
    /// Untagged children only: a tagged child meets a tagged one by tag.
    untagged: FixMap<(bool, u64), usize>,
    named: FixMap<(bool, u64), usize>,
    /// Whether every held name is ASCII, which is what the name keys fold.
    exact: bool,
}

impl ChildIndex {
    fn of(children: &[Child], tags: &[Option<i32>]) -> Self {
        let mut index = Self {
            tagged: FixMap::default(),
            untagged: FixMap::default(),
            named: FixMap::default(),
            exact: true,
        };
        for (at, (child, tag)) in children.iter().zip(tags).enumerate() {
            index.push(at, child, *tag);
        }
        index
    }

    /// Records the child at `at`, where no child before it holds its keys.
    fn push(&mut self, at: usize, child: &Child, tag: Option<i32>) {
        let group = matches!(child, Child::Group(..));
        let name = child.name();
        self.exact &= name.is_ascii();
        let key = (group, name_key(name));
        self.named.entry(key).or_insert(at);
        match tag {
            Some(tag) => {
                self.tagged.entry((group, tag)).or_insert(at);
            }
            None => {
                self.untagged.entry(key).or_insert(at);
            }
        }
    }

    /// The first held child [`same_child`] matches with `child`.
    fn position(
        &self,
        children: &[Child],
        tags: &[Option<i32>],
        child: &Child,
        tag: Option<i32>,
    ) -> Option<usize> {
        let scan = || {
            children
                .iter()
                .zip(tags)
                .position(|(held, held_tag)| same_child(held, *held_tag, child, tag))
        };
        let name = child.name();
        if !self.exact || !name.is_ascii() {
            return scan();
        }
        let group = matches!(child, Child::Group(..));
        let key = (group, name_key(name));
        // A name key is believed only where the names it joins fold alike;
        // two names sharing a key are left to the scan.
        let by_name = |table: &FixMap<(bool, u64), usize>| match table.get(&key) {
            Some(&at) if crate::folds_equal(children[at].name(), name) => Ok(Some(at)),
            Some(_) => Err(()),
            None => Ok(None),
        };
        let found = match tag {
            Some(tag) => by_name(&self.untagged).map(|named| {
                let tagged = self.tagged.get(&(group, tag)).copied();
                match (tagged, named) {
                    (Some(tagged), Some(named)) => Some(tagged.min(named)),
                    (tagged, named) => tagged.or(named),
                }
            }),
            None => by_name(&self.named),
        };
        found.unwrap_or_else(|()| scan())
    }
}

/// Whether two children, each beside its [`child_tag`], state one logical
/// child: the same shape, and the same tag where both carry one, else the
/// same folded name.
fn same_child(left: &Child, left_tag: Option<i32>, right: &Child, right_tag: Option<i32>) -> bool {
    if std::mem::discriminant(left) != std::mem::discriminant(right) {
        return false;
    }
    match (left_tag, right_tag) {
        (Some(left), Some(right)) => left == right,
        _ => crate::folds_equal(left.name(), right.name()),
    }
}

impl Child {
    fn field(&self) -> &Field {
        match self {
            Self::Flat(field, _) | Self::Group(field, _) => field,
        }
    }

    fn name(&self) -> &str {
        self.field().name()
    }

    /// The value a scalar child holds; a group holds none.
    fn value(&self) -> Option<&Scalar> {
        match self {
            Self::Flat(_, value) => Some(value),
            Self::Group(..) => None,
        }
    }

    /// One child, its occurrences unpacked when it is a repeating group.
    ///
    /// A group is a Serie of Structs carrying `FIX:counter`; a Serie no counter
    /// heads is a repeated flat field and stays whole, because the registry's
    /// scalar field cannot hold it.
    fn unpack(field: Field, value: Scalar) -> Self {
        // The shape first: a scalar child - nearly every child - is told
        // apart without reading its metadata for a counter.
        let (Some(members), Some(rows)) = (item_fields(&field), value.as_serie()) else {
            return Self::Flat(field, value);
        };
        if field.as_fix().counter().ok().flatten().is_none() {
            return Self::Flat(field, value);
        }
        let occurrences = rows
            .iter()
            .map(|row| {
                row.as_sequence()
                    .map(|values| Level::unpack(members, values))
            })
            .collect();
        Self::Group(field, occurrences)
    }
}

/// One group packed back into its Serie: the item is the union of every
/// occurrence's members in first-seen order, each nullable because an
/// occurrence need not state one, exactly as the builder closes a group.
///
/// A group holding no occurrence keeps the Serie it arrived as: there is
/// nothing to rebuild the item from, and the declared shape is the best
/// statement of what its occurrences would hold.
fn pack_group(serie: Field, occurrences: Vec<Option<Level>>) -> Result<(Field, Scalar)> {
    let mut finished: Vec<Option<(Vec<Field>, Vec<Scalar>)>> =
        Vec::with_capacity(occurrences.len());
    for occurrence in occurrences {
        finished.push(occurrence.map(Level::pack).transpose()?);
    }
    if finished.iter().all(Option::is_none) {
        let rows = finished.into_iter().map(|_| Scalar::Null);
        return Ok((serie, Scalar::from_sequence(rows)));
    }
    let mut members: Vec<Field> = Vec::new();
    for (fields, _) in finished.iter().flatten() {
        for field in fields {
            if members.iter().any(|held| held.name() == field.name()) {
                continue;
            }
            let mut member = field.clone();
            member.set_nullable(true);
            members.push(member);
        }
    }
    // The union took each member once by name.
    let mut item = DataType::from(StructType::from_unique_fields(members))
        .required_field(occurrence_name(&serie));
    if finished.iter().any(Option::is_none) {
        item.set_nullable(true);
    }
    // The occurrence field owns the members now, so each row lays out against
    // its own children rather than a copy of them: the copy was a whole field
    // per member of every group of every message.
    let members = item.fields();
    let rows = finished.into_iter().map(|occurrence| {
        let Some((fields, mut values)) = occurrence else {
            return Scalar::Null;
        };
        Scalar::from_sequence(members.iter().map(|member| {
            fields
                .iter()
                .position(|field| field.name() == member.name())
                .map_or(Scalar::Null, |at| {
                    std::mem::replace(&mut values[at], Scalar::Null)
                })
        }))
    });
    let rows = Scalar::from_sequence(rows);
    let dtype = match serie.dtype() {
        DataType::LargeSerie(_) => DataType::large_serie(item),
        _ => DataType::serie(item),
    };
    // The Serie's own metadata travels as the one it is rather than as a map
    // rebuilt from its entries for every group of every message.
    let rebuilt = Field::new_with_metadata(
        serie.name(),
        dtype,
        serie.is_nullable(),
        serie.as_metadata().clone(),
    );
    Ok((rebuilt, rows))
}

/// The wire text a held value spells, `None` for null and for a shape FIX
/// has no text for.
///
/// A boolean is `Y` or `N`, a number its decimal, a temporal its canonical
/// rendering; a string and a code are themselves. A `state` answers its stored
/// spelling, which [`matches`] never reaches: a state is compared through
/// [`State::from_spelling`] and never rendered back to a code.
/// The source's own value under a constant its rule writes back to it: the
/// constant where the whole value is one the rule's condition named, else the
/// held tokens with the named one replaced - `ExecInst` `G T` restated at `T`
/// is `G R`, the other instruction kept. A `state` never renders back to a
/// code, so it takes the constant whole.
///
/// `named` are the literals the rule's condition compared against, which is
/// how a rule over a `MultipleCharValue` says which of several held codes it
/// is about without a grammar of its own for one.
fn restated_tokens(held: &Scalar, named: &[SmolStr], text: &str) -> SmolStr {
    if matches!(held, Scalar::State(_)) {
        return SmolStr::new(text);
    }
    let Some(spelled) = wire_text(held) else {
        return SmolStr::new(text);
    };
    let Some(when) = named
        .iter()
        .find(|when| spelled.split(' ').any(|token| token == when.as_str()))
    else {
        return SmolStr::new(text);
    };
    if spelled == *when {
        return SmolStr::new(text);
    }
    let tokens: Vec<&str> = spelled
        .split(' ')
        .map(|token| if token == when { text } else { token })
        .collect();
    SmolStr::new(tokens.join(" "))
}

/// Every text literal a term holds: for a rule's condition, every value it
/// named.
fn named_literals(term: &Term) -> Vec<SmolStr> {
    let mut held = Vec::new();
    // Pre-order, each literal once, in the order the condition names them:
    // the first named the held value spells is the one a restatement
    // replaces.
    term.walk(&mut |node| {
        if let Term::Literal(literal) = node {
            if let Some(text) = literal.value().as_str() {
                held.push(SmolStr::new(text));
            }
        }
    });
    held
}

/// The members of the one occurrence a group projection states.
///
/// A group occurrence is spelled `[{member: term, ...}] as group`: a serie of
/// exactly one record, because a rule fills one occurrence and the Serie is
/// what the row holds a group as. The grammar reads `{member: term}` as a
/// map whose keys are paths, and a member is the one name its key spells;
/// anything else names no occurrence.
fn occurrence_members(term: &Term) -> Option<Vec<(SmolStr, &Term)>> {
    let Term::Serie(items) = term else {
        return None;
    };
    match items.as_ref() {
        [Term::Struct(members)] => Some(
            members
                .iter()
                .map(|(name, term)| (name.clone(), term))
                .collect(),
        ),
        [Term::Map(entries)] => entries
            .iter()
            .map(|(key, term)| match key {
                Term::Path(steps) => match steps.as_ref() {
                    [crate::FieldSegment::Field(name)] => Some((name.clone(), term)),
                    _ => None,
                },
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

/// A value another field typed, re-typed for `target`.
///
/// Text is a wire spelling and reads as the codec reads one, code names and
/// state names included: the generic value contract would read `D` into a
/// `state` column by the shared code table, where the field's own set says
/// which state `D` is for *this* field. Anything else goes through the value
/// contract first - an instant lands in an instant column as itself - and
/// through its wire text only where that refuses.
fn converted(registry: &FixRegistry, target: &Field, value: &Scalar) -> Scalar {
    if value.is_null() {
        return Scalar::Null;
    }
    if !matches!(value, Scalar::String(_)) {
        if let Ok(typed) = target.scalar(value.clone()) {
            if !typed.is_null() {
                return typed;
            }
        }
    }
    match wire_text(value) {
        Some(text) => typed_spelling(registry, target, &text),
        None => Scalar::Null,
    }
}

/// The rules one field restates by.
enum Entries<'registry> {
    /// The field's own `FIX:replacements`, compiled: a plan per entry, `None`
    /// where the entry's text is not a plan.
    Own(Cow<'registry, [Option<Arc<Plan>>]>),
    /// What the specification retired of the field's tag.
    Retired(&'static [Rule]),
    None,
}

/// One entry of [`Entries`].
enum Entry<'entries> {
    Plan(&'entries Arc<Plan>),
    Retired(&'static Rule),
}

impl Entries<'_> {
    /// How many entries the field states, readable or not: the bound on
    /// how many times one source is restated in turn.
    fn len(&self) -> usize {
        match self {
            Self::Own(plans) => plans.len(),
            Self::Retired(rules) => rules.len(),
            Self::None => 0,
        }
    }

    /// The readable entries, in document order.
    fn iter(&self) -> impl Iterator<Item = Entry<'_>> {
        let (own, retired) = match self {
            Self::Own(plans) => (Some(plans.iter().flatten()), None),
            Self::Retired(rules) => (None, Some(rules.iter())),
            Self::None => (None, None),
        };
        own.into_iter()
            .flatten()
            .map(Entry::Plan)
            .chain(retired.into_iter().flatten().map(Entry::Retired))
    }
}

/// One pass over one message.
///
/// Resolution goes through the message, so a tag or a name reaches the
/// message's own branch first and the standard one after, exactly as every
/// lookup on it does; nothing here re-derives the tier.
struct Restater<'msg> {
    msg: &'msg FixMsg,
    registry: &'msg FixRegistry,
    /// The root's `MsgType(35)`, which every `msgtypes` filter reads.
    msgtype: Option<&'msg str>,
}

impl<'msg> Restater<'msg> {
    /// The registry field a child reaches: by its own tag, else by its name
    /// or alias, else by the decimal tag its name spells.
    fn resolve(&self, child: &Field) -> Option<&'msg Field> {
        if let Some(tag) = tag_and_counter(self.registry, child).0 {
            if let Some(known) = self.msg.known_by_tag(tag) {
                return Some(known);
            }
        }
        if let Some(known) = self.msg.known_by_name(child.name()) {
            return Some(known);
        }
        let tag = super::field::parse_tag(child.name())?;
        self.msg.known_by_tag(tag)
    }

    /// The scalar child of `level` that `tag` reaches: the child carrying
    /// it, else the one named as the dictionary names it.
    fn position_of(&self, level: &Level, tag: i32) -> Option<usize> {
        let name = self.msg.known_by_tag(tag).map_or("", Field::name);
        level.position_of_field(Some(tag), name)
    }

    /// The stated value `tag` holds at `level`.
    fn stated_at<'level>(&self, level: &'level Level, tag: i32) -> Option<&'level Scalar> {
        level
            .value_at(self.position_of(level, tag)?)
            .filter(|held| !held.is_null())
    }

    /// The rules `field` restates by: its own document where it states one,
    /// whole - an entry of its own that is not a plan is still its own and
    /// the specification does not fill in behind it - else what the
    /// specification retired of its tag.
    fn entries(&self, field: &Field, tag: Option<i32>) -> Entries<'msg> {
        let own = plans_of(self.registry, field);
        if !own.is_empty() {
            return Entries::Own(own);
        }
        match tag.and_then(retired::rules_of) {
            Some(rules) => Entries::Retired(rules),
            None => Entries::None,
        }
    }

    /// Whether a rule could restate the registry field `known`: off the
    /// registry's facts where it indexed the field, else read the same way.
    fn carries_rules(&self, known: &Field) -> bool {
        self.registry.facts_of(known).map_or_else(
            || {
                known.as_fix().replacements().next_ok().is_some()
                    || tag_and_counter(self.registry, known)
                        .0
                        .is_some_and(|tag| retired::rules_of(tag).is_some())
            },
            |facts| facts.ruled,
        )
    }

    /// Whether one of the specification's rules holds at a level: the
    /// message type where the rule names some, the enclosing group where it
    /// applies inside one, and the held value's wire spelling - or one of
    /// its codes - where it is about one. What a plan asks of `:msgtype`,
    /// `:group` and the source column, asked of the facts directly, and
    /// answered as the plan answers it: a boolean holds no text a condition
    /// could name, so a condition on a boolean field's value does not hold,
    /// which is what the two boolean retirements - `OddLot(575)` and
    /// `PublishTrdIndicator(852)` - answer.
    fn retired_applies(&self, rule: &Rule, value: &Scalar, group: Option<&str>) -> bool {
        if !rule.msgtypes.is_empty()
            && !self
                .msgtype
                .is_some_and(|held| rule.msgtypes.contains(&held))
        {
            return false;
        }
        if rule.within.is_some_and(|within| group != Some(within)) {
            return false;
        }
        if matches!(value, Scalar::Boolean(_)) && !matches!(rule.when, When::Any) {
            return false;
        }
        match rule.when {
            When::Any => true,
            When::Equals(text) => wire_text(value).is_some_and(|held| held == text),
            When::Contains(text) => wire_text(value).is_some_and(|held| held.contains(text)),
        }
    }

    /// What one level is to the pass, read without unpacking it.
    ///
    /// A level is canonical where every scalar child the dictionary knows
    /// is the dictionary's own field as the builder states one - the
    /// canonical name, the field's datatype, its metadata shared and not
    /// nullable - holding a value, reached by no other child of the level,
    /// and carrying no replacement rule that could fire; every group
    /// occurrence is such a level in turn. Canonicalizing such a level
    /// keeps every child as it is, no rule restates one, and the group
    /// packs back to the Serie it arrived as - so the pass would rebuild
    /// the level it was handed. A child the dictionary does not know is
    /// kept as it is either way, and a nested child no group declares is
    /// too.
    ///
    /// A rule fires only where its condition holds at the root, so a root
    /// child carrying rules leaves the level canonical exactly when none of
    /// them applies: the plan is read once per rule and bound to the root
    /// as the restatement would bind it, and the first that applies makes
    /// the level ruled - canonical but for that rule's writes, which the
    /// restatement lands on the level as it stands.
    fn shape(&self, fields: &[Field], values: &[Scalar], shape: u64) -> Shape {
        let mut ruled: Vec<(&Field, &Scalar)> = Vec::new();
        if !self.canonical_level(fields, values, false, &mut ruled) {
            return Shape::Rebuilt;
        }
        if ruled.is_empty() {
            return Shape::Canonical;
        }
        let root = self.msg.as_field();
        let row = self.msg.as_value();
        let parameters = self.parameters(None);
        for (field, value) in ruled {
            let tag = tag_and_counter(self.registry, field).0;
            let applies = self.entries(field, tag).iter().any(|entry| match entry {
                Entry::Plan(plan) => Self::applies(plan, root, row, shape, &parameters),
                Entry::Retired(rule) => self.retired_applies(rule, value, None),
            });
            if applies {
                return Shape::Ruled;
            }
        }
        Shape::Canonical
    }

    /// One level of [`Self::canonical`]: `member` for a group occurrence,
    /// whose members the builder closes nullable and the group packs back
    /// nullable, so nullability says nothing there; at the root a child is
    /// nullable exactly where its value is null. A root child carrying
    /// replacement rules is collected into `ruled` for the caller to test,
    /// and one inside an occurrence is not canonical, because a rule at
    /// that level binds to the occurrence and not to the root.
    fn canonical_level<'values>(
        &self,
        fields: &[Field],
        values: &'values [Scalar],
        member: bool,
        ruled: &mut Vec<(&'msg Field, &'values Scalar)>,
    ) -> bool {
        let mut reached: Vec<*const Field> = Vec::new();
        for (field, value) in fields.iter().zip(values) {
            if tag_and_counter(self.registry, field).1.is_some() {
                if let (Some(members), Some(rows)) =
                    (super::schema::item_fields(field), value.as_serie())
                {
                    // The Serie as `pack_group` would rebuild it: a Serie and
                    // not a map, its item nullable exactly where an
                    // occurrence is null, and every member nullable.
                    let (DataType::Serie(item) | DataType::LargeSerie(item)) = field.dtype() else {
                        return false;
                    };
                    if item.is_nullable() != (rows.null_count() != 0) {
                        return false;
                    }
                    // A member level returns before it would push a ruled
                    // child, so each occurrence's list stays empty.
                    let canonical = rows.iter().all(|row| {
                        row.as_sequence().is_none_or(|values| {
                            self.canonical_level(members, values, true, &mut Vec::new())
                        })
                    });
                    if !canonical {
                        return false;
                    }
                    continue;
                }
            }
            if field.dtype().is_nested() {
                continue;
            }
            if member && !field.is_nullable() {
                return false;
            }
            let Some(known) = self.resolve(field) else {
                continue;
            };
            if reached.iter().any(|held| std::ptr::eq(*held, known)) {
                return false;
            }
            // Sized once, on the first field reached, for every child the
            // level has: a level reaching nothing allocates nothing here,
            // and one reaching a hundred grows the list once.
            if reached.capacity() == 0 {
                reached.reserve_exact(fields.len());
            }
            reached.push(known);
            let nullable = if member {
                field.is_nullable()
            } else {
                field.is_nullable() == value.is_null()
            };
            if !nullable
                || field.name() != known.name()
                || field.dtype() != known.dtype()
                || !field.as_metadata().shares_storage_with(known.as_metadata())
            {
                return false;
            }
            if !value.is_null() && self.carries_rules(known) {
                if member {
                    return false;
                }
                ruled.push((known, value));
            }
        }
        true
    }

    /// One level canonicalized and restated, its group occurrences first.
    fn level(&self, level: Level, group: Option<&str>) -> Result<Level> {
        let mut level = self.canonicalized(level)?;
        self.restated(&mut level, group, None);
        Ok(level)
    }

    /// Every child re-expressed under the registry's field, children
    /// reaching one field merged, groups recursed into.
    ///
    /// Every child is resolved before any is rewritten, because merging has
    /// to know each child that reaches one field. The kept child is the one
    /// carrying the canonical name where there is one, else the first; its
    /// value is the canonical child's when stated, else the first stated
    /// among the rest. A merged-away child whose value is null or equal is
    /// dropped, and one whose stated value disagrees is left exactly as it
    /// arrived - nothing that arrived is lost. Order is preserved: a renamed
    /// child keeps its position.
    fn canonicalized(&self, level: Level) -> Result<Level> {
        let count = level.children.len();
        let mut resolved: Vec<Option<(&'msg Field, Scalar)>> = Vec::with_capacity(count);
        for child in &level.children {
            resolved.push(match child {
                Child::Flat(field, value) if !field.dtype().is_nested() => self
                    .resolve(field)
                    .map(|known| (known, retyped(self.registry, known, field, value))),
                _ => None,
            });
        }
        let stated = |at: usize| {
            resolved[at]
                .as_ref()
                .map(|(_, value)| value)
                .filter(|value| !value.is_null())
        };
        let mut decisions: Vec<Option<Decision>> = (0..count).map(|_| None).collect();
        // One scratch for the whole level rather than one per child: two
        // children reaching one registry field is the merge case, and in the
        // ordinary one every child resolves to a field of its own, so this
        // would otherwise allocate a vector of length one per column of a row
        // eighty columns wide. Empty rather than sized, so a level with no
        // resolved child gains no allocation it did not have.
        let mut same: Vec<usize> = Vec::new();
        for index in 0..count {
            if decisions[index].is_some() {
                continue;
            }
            let Some((known, _)) = resolved[index].as_ref() else {
                continue;
            };
            same.clear();
            same.extend((index..count).filter(|at| {
                resolved[*at]
                    .as_ref()
                    .is_some_and(|(held, _)| std::ptr::eq(*held, *known))
            }));
            let canonical = same
                .iter()
                .copied()
                .find(|at| level.children[*at].name() == known.name());
            let kept = canonical.unwrap_or(index);
            let value = canonical
                .and_then(stated)
                .or_else(|| same.iter().copied().find_map(stated))
                .cloned()
                .unwrap_or(Scalar::Null);
            for at in same.iter().copied() {
                decisions[at] = Some(if at == kept {
                    Decision::Keep(value.clone())
                } else if level.children[at].value().is_none_or(Scalar::is_null)
                    || stated(at) == Some(&value)
                {
                    Decision::Drop
                } else {
                    Decision::Untouched
                });
            }
        }
        let mut out = Vec::with_capacity(count);
        for (index, child) in level.children.into_iter().enumerate() {
            match (decisions[index].take(), child) {
                (Some(Decision::Keep(value)), _) => {
                    let Some((known, _)) = resolved[index].as_ref() else {
                        continue;
                    };
                    let mut field = stated_field(known);
                    if value.is_null() {
                        field.set_nullable(true);
                    }
                    out.push(Child::Flat(field, value));
                }
                (Some(Decision::Drop), _) => {}
                (None, Child::Group(serie, occurrences)) => {
                    let mut restated = Vec::with_capacity(occurrences.len());
                    for occurrence in occurrences {
                        restated.push(
                            occurrence
                                .map(|held| self.level(held, Some(serie.name())))
                                .transpose()?,
                        );
                    }
                    out.push(Child::Group(serie, restated));
                }
                (Some(Decision::Untouched) | None, child) => out.push(child),
            }
        }
        Ok(Level { children: out })
    }

    /// Every child of `level` whose field carries replacement rules, in
    /// ascending tag order, restated by the first rule whose condition the
    /// level meets.
    ///
    /// Ascending, so `ExecTransType(20)` writes `ExecType(150)` before
    /// `ExecType`'s own value rule reads it. The first rule whose condition
    /// holds answers, applied or blocked: a later rule never fills in for one
    /// a stated value refused. A rule that rewrote the source's own value
    /// leaves a new held value, which is restated in turn - `ExecInst` `T`
    /// becomes `R`, which FIX 5.0 retired for `PegPriceType` - so one pass
    /// reaches what a second pass would otherwise find; a chain is bounded
    /// by the rules the field states, so two rules restating each other end.
    ///
    /// `pristine` is the level as its root and row already state it, where
    /// the caller hands the level over as it arrived: the first rule read
    /// against it reads those rather than a view rebuilt from the level,
    /// and a view is rebuilt only once a write has landed.
    fn restated(
        &self,
        level: &mut Level,
        group: Option<&str>,
        pristine: Option<(&Field, &Scalar, u64)>,
    ) {
        let mut sources: Vec<(i32, usize)> = level
            .children
            .iter()
            .enumerate()
            .filter_map(|(at, child)| match child {
                Child::Flat(field, value) if !value.is_null() => {
                    Some((tag_and_counter(self.registry, field).0?, at))
                }
                _ => None,
            })
            .collect();
        sources.sort_unstable();
        let parameters = self.parameters(group);
        let mut touched = false;
        // The level's view, built on the first plan that has to be read
        // against it and kept until a write lands: every later source reads
        // the level as it then stands, so a field with no rule costs no
        // clone at all, one the specification rules costs none either, and
        // the fields a write left untouched share one rebuilt view.
        let mut view: Option<(Field, Scalar, u64)> = None;
        for (tag, at) in sources {
            let mut remaining = usize::MAX;
            while remaining > 0 {
                let mut planned = None;
                // Copied only where a rule answered, because only the write
                // that rule plans has anything to compare against. A field
                // with no replacement rule - which is nearly every field of
                // nearly every message - copies nothing.
                let mut before: Option<Scalar> = None;
                {
                    let Child::Flat(field, value) = &level.children[at] else {
                        break;
                    };
                    if value.is_null() {
                        break;
                    }
                    // The tag the source was collected under, so the
                    // table is reached without resolving the field twice.
                    let entries = self.entries(field, Some(tag));
                    let count = entries.len();
                    let source = Source { value, tag };
                    for entry in entries.iter() {
                        if planned.is_some() {
                            continue;
                        }
                        let plan = match entry {
                            Entry::Plan(plan) => plan,
                            Entry::Retired(rule) => {
                                if !self.retired_applies(rule, value, group) {
                                    continue;
                                }
                                before = Some(value.clone());
                                planned =
                                    Some(self.plan_retired(level, &source, rule, group.is_none()));
                                continue;
                            }
                        };
                        if view.is_none() {
                            view = match pristine {
                                Some((root, row, shape)) if !touched => {
                                    Some((root.clone(), row.clone(), shape))
                                }
                                _ => Self::view(level),
                            };
                        }
                        let Some((root, row, shape)) = view.as_ref() else {
                            break;
                        };
                        if !Self::applies(plan, root, row, *shape, &parameters) {
                            continue;
                        }
                        before = Some(value.clone());
                        planned = Some(self.plan(
                            level,
                            root,
                            row,
                            *shape,
                            &source,
                            plan,
                            &parameters,
                            group.is_none(),
                        ));
                    }
                    remaining = remaining.min(count);
                }
                let Some(Some(writes)) = planned else {
                    break;
                };
                for write in writes {
                    level.apply(write);
                }
                touched = true;
                view = None;
                remaining -= 1;
                // A field the specification deprecated is restated and not
                // kept: what it said now lives under what replaced it, and
                // a second copy under the retired name would be a second
                // owner of one fact.
                if let Child::Flat(field, value) = &mut level.children[at] {
                    if field.as_fix().deprecated().is_some() {
                        *value = Scalar::Null;
                        break;
                    }
                }
                if level.value_at(at) == before.as_ref() {
                    break;
                }
            }
        }
    }

    /// One level's schema and row, as a term reads them.
    ///
    /// Built only where a field carries a document of its own and holds a
    /// value, so the clone it costs is paid once per firing rather than once
    /// per child, and the specification's retirements - which read the
    /// level as it is - never pay it. Rebuilt after a rule's writes land,
    /// because the next rule reads the level as it then is.
    fn view(level: &Level) -> Option<(Field, Scalar, u64)> {
        let (fields, values) = level.clone().pack().ok()?;
        // A write replaces the child it reached or appends a field no child
        // is named as, so the level's children stay named once.
        let root = DataType::from(StructType::from_unique_fields(fields)).required_field("row");
        let shape = shape_digest(&root, false);
        Some((root, Scalar::from_sequence(values), shape))
    }

    /// The parameters a rule's condition may name: `:msgtype`, the root's
    /// `MsgType(35)`, and `:group`, the repeating group the level is an
    /// occurrence of. Both are facts about the message rather than columns
    /// of the level, which is why they cross as parameters.
    fn parameters(&self, group: Option<&str>) -> [(&'static str, Scalar); 2] {
        [
            ("msgtype", self.msgtype.map_or(Scalar::Null, Scalar::from)),
            ("group", group.map_or(Scalar::Null, Scalar::from)),
        ]
    }

    /// Whether one rule's condition holds at a level.
    ///
    /// A condition the level cannot bind - a column it does not hold, a
    /// comparison with no common type - does not hold, rather than refusing
    /// the pass: a rule is data, and one that does not apply here applies
    /// nowhere.
    fn applies(
        plan: &Arc<Plan>,
        root: &Field,
        row: &Scalar,
        shape: u64,
        parameters: &[(&str, Scalar)],
    ) -> bool {
        bound_term(plan, None, root, shape, parameters)
            .is_some_and(|bound| bound.matches(row).unwrap_or(false))
    }

    /// Every write one rule makes at `level`, or nothing when one target
    /// cannot take its value.
    #[expect(
        clippy::too_many_arguments,
        reason = "one rule's whole context, threaded"
    )]
    fn plan(
        &self,
        level: &Level,
        root: &Field,
        row: &Scalar,
        shape: u64,
        source: &Source<'_>,
        plan: &Arc<Plan>,
        parameters: &[(&str, Scalar)],
        rooted: bool,
    ) -> Option<Vec<Write>> {
        let named = named_literals(plan.filter_section().term());
        let mut writes = Vec::new();
        for projection in plan.selector().projections() {
            let name = projection.name();
            let term = projection.term();
            writes.push(self.plan_projection(
                level,
                Some(source),
                &named,
                &name,
                term,
                plan,
                root,
                row,
                shape,
                parameters,
                rooted,
            )?);
        }
        Some(writes)
    }

    /// One projection planned at `writes`: an occurrence of a repeating
    /// group where the name is a group's and the term spells one, else one
    /// column. Every term is read at the rule's own level - `root` and `row`
    /// - whatever level it is written into.
    #[expect(
        clippy::too_many_arguments,
        reason = "one rule's whole context, threaded"
    )]
    fn plan_projection(
        &self,
        writes: &Level,
        source: Option<&Source<'_>>,
        named: &[SmolStr],
        name: &str,
        term: &Term,
        plan: &Arc<Plan>,
        root: &Field,
        row: &Scalar,
        shape: u64,
        parameters: &[(&str, Scalar)],
        rooted: bool,
    ) -> Option<Write> {
        if let Some(definition) = self
            .registry
            .get_definition(crate::FixCategory::Groups, name)
        {
            let members = occurrence_members(term)?;
            return self.plan_group(
                writes, definition, &members, plan, root, row, shape, parameters,
            );
        }
        let value = bound_term(plan, Some(term), root, shape, parameters)?
            .eval(row)
            .ok()?;
        self.plan_column(writes, source, named, name, term, value, rooted)
    }

    /// One column planned at `writes`: the projected value re-typed for the
    /// target, and the target found at that level. A constant written over
    /// the rule's own source replaces the token the condition named.
    #[expect(
        clippy::too_many_arguments,
        reason = "one rule's whole context, threaded"
    )]
    fn plan_column(
        &self,
        writes: &Level,
        source: Option<&Source<'_>>,
        named: &[SmolStr],
        name: &str,
        term: &Term,
        value: Scalar,
        rooted: bool,
    ) -> Option<Write> {
        let target = stated_field(self.msg.known_by_name(name)?);
        let tag = target.as_fix().tag().ok().flatten()?;
        let own = source.is_some_and(|source| source.tag == tag);
        let value = match (source, term) {
            (Some(source), Term::Literal(literal)) if own => match literal.value().as_str() {
                Some(text) => typed_spelling(
                    self.registry,
                    &target,
                    &restated_tokens(source.value, named, text),
                ),
                None => converted(self.registry, &target, &value),
            },
            _ => converted(self.registry, &target, &value),
        };
        self.column_write(writes, own, tag, target, value, rooted)
    }

    /// Every write one of the specification's rules makes at `level`, or
    /// nothing when one target cannot take its value: what [`Self::plan`]
    /// plans from a plan, planned from the table.
    fn plan_retired(
        &self,
        level: &Level,
        source: &Source<'_>,
        rule: &Rule,
        rooted: bool,
    ) -> Option<Vec<Write>> {
        // The literals a plan's condition would name, in its order - the
        // message types, the group, the value - which is what a constant
        // written back over a multi-valued source replaces the token by.
        let mut named: Vec<SmolStr> = rule.msgtypes.iter().copied().map(SmolStr::new).collect();
        named.extend(rule.within.map(SmolStr::new));
        match rule.when {
            When::Any => {}
            When::Equals(text) | When::Contains(text) => named.push(SmolStr::new(text)),
        }
        let mut writes = Vec::with_capacity(rule.fills.len());
        for fill in rule.fills {
            writes.push(self.fill_write(level, level, source, &named, fill, rooted, true)?);
        }
        Some(writes)
    }

    /// One fill planned at `writes`, as [`Self::plan_projection`] plans a
    /// projection: an occurrence where the fill is one, else one column
    /// whose value the fill spells. Every value is read at `reads`, the
    /// rule's own level, whatever level it is written into - an occurrence
    /// filled from the root reads the root's fields. `owning` is whether
    /// the fill stands at the rule's own level, where a fill of the
    /// source's own tag rewrites the source; inside an occurrence nothing
    /// is the source.
    #[expect(
        clippy::too_many_arguments,
        reason = "one fill's whole context, threaded"
    )]
    fn fill_write(
        &self,
        writes: &Level,
        reads: &Level,
        source: &Source<'_>,
        named: &[SmolStr],
        fill: &Fill,
        rooted: bool,
        owning: bool,
    ) -> Option<Write> {
        let target_of = |tag: i32| self.msg.known_by_tag(tag).map(stated_field);
        match *fill {
            Fill::Constant { tag, text } => {
                let target = target_of(tag)?;
                let own = owning && source.tag == tag;
                let value = if own {
                    typed_spelling(
                        self.registry,
                        &target,
                        &restated_tokens(source.value, named, text),
                    )
                } else {
                    typed_spelling(self.registry, &target, text)
                };
                self.column_write(writes, own, tag, target, value, rooted)
            }
            Fill::Source { tag } => {
                let target = target_of(tag)?;
                let value = converted(self.registry, &target, source.value);
                self.column_write(
                    writes,
                    owning && source.tag == tag,
                    tag,
                    target,
                    value,
                    rooted,
                )
            }
            Fill::From { tag, source: other } => {
                let target = target_of(tag)?;
                let value = converted(self.registry, &target, self.stated_at(reads, other)?);
                self.column_write(writes, false, tag, target, value, rooted)
            }
            Fill::Join { tag, parts } => {
                let target = target_of(tag)?;
                let mut joined = String::new();
                for part in parts {
                    match *part {
                        Part::Text(at) => joined.push_str(self.stated_at(reads, at)?.as_str()?),
                        Part::TwoDigits(at) => {
                            let spelled = wire_text(self.stated_at(reads, at)?)?;
                            let padded = format!("0{spelled}");
                            joined.push_str(&padded[padded.len().saturating_sub(2)..]);
                        }
                    }
                }
                let value = typed_spelling(self.registry, &target, &joined);
                self.column_write(writes, false, tag, target, value, rooted)
            }
            Fill::Occurrence { group, members } => {
                self.occurrence_write(writes, reads, source, group, members)
            }
        }
    }

    /// One column write, its value already spelled for `target`: the
    /// target found at the level, checked writable, and the write planned.
    fn column_write(
        &self,
        writes: &Level,
        own: bool,
        tag: i32,
        target: Field,
        value: Scalar,
        rooted: bool,
    ) -> Option<Write> {
        if value.is_null() {
            return None;
        }
        let at = self.position_of(writes, tag);
        if at.is_none() && writes.names(target.name()) {
            return None;
        }
        // A fact the message holds typed is stated whether or not the row
        // carries a column for it: `TimeInForce(59)` and `OrderQty(38)` are
        // the event's, and a rule that would fill one has to see what the
        // message already said. Only at the root, because a group's
        // occurrence holds no typed fact.
        let typed = (rooted && at.is_none() && super::identity::is_typed_tag(tag))
            .then(|| self.msg.get_by_tag(tag))
            .flatten();
        let held = match (at.and_then(|at| writes.value_at(at)), typed.as_ref()) {
            (Some(held), _) => Some(held),
            (None, held) => held,
        };
        let writable = own || Self::writable(&target, held, &value);
        writable.then_some(Write::Field(FieldWrite {
            at,
            field: target,
            value,
        }))
    }

    /// One group occurrence planned: the occurrence whose literal members
    /// all equal the projected ones is merged into, else one is appended,
    /// and the counter child takes the count the group then has.
    ///
    /// An occurrence stating no literal matches the first occurrence there
    /// is, so a second pass finds what the first wrote rather than appending
    /// it again.
    #[expect(
        clippy::too_many_arguments,
        reason = "one rule's whole context, threaded"
    )]
    fn plan_group(
        &self,
        level: &Level,
        definition: &Field,
        members: &[(SmolStr, &Term)],
        plan: &Arc<Plan>,
        root: &Field,
        row: &Scalar,
        shape: u64,
        parameters: &[(&str, Scalar)],
    ) -> Option<Write> {
        let counter_tag = definition.as_fix().counter().ok().flatten()?;
        let counter_field = stated_field(self.msg.known_by_tag(counter_tag)?);
        let at = level.position_of_group(counter_tag, definition.name());
        let empty: Vec<Option<Level>> = Vec::new();
        let (serie, occurrences) = match at {
            Some(at) => match &level.children[at] {
                Child::Group(_, occurrences) => (None, occurrences),
                // A declared group stating no occurrence, opened on apply.
                Child::Flat(_, value) if value.is_null() => (None, &empty),
                Child::Flat(..) => return None,
            },
            None => {
                if level.names(definition.name()) {
                    return None;
                }
                let mut serie = definition.clone();
                serie.set_nullable(true);
                (Some(serie), &empty)
            }
        };
        // The literal members, typed as their fields hold them, decide which
        // occurrence this rule is about.
        let mut constants: Vec<(i32, Scalar)> = Vec::new();
        for (name, term) in members {
            let Term::Literal(literal) = term else {
                continue;
            };
            let known = self.msg.known_by_name(name)?;
            let tag = known.as_fix().tag().ok().flatten()?;
            constants.push((
                tag,
                converted(self.registry, &stated_field(known), literal.value()),
            ));
        }
        let matched = occurrences.iter().position(|occurrence| {
            occurrence.as_ref().is_some_and(|held| {
                constants.iter().all(|(tag, wanted)| {
                    self.stated_at(held, *tag)
                        .is_some_and(|stated| stated == wanted)
                })
            })
        });
        let blank = Level::default();
        let target = matched
            .and_then(|at| occurrences[at].as_ref())
            .unwrap_or(&blank);
        let mut planned = Vec::with_capacity(members.len());
        for (name, term) in members {
            planned.push(self.plan_projection(
                target,
                None,
                &[],
                name,
                term,
                plan,
                root,
                row,
                shape,
                parameters,
                // A group's occurrence, never the root: no typed fact lives
                // here.
                false,
            )?);
        }
        let count = occurrences.len() + usize::from(matched.is_none());
        let value = counter_field
            .scalar(Scalar::from(i64::try_from(count).ok()?))
            .ok()?;
        let counter = FieldWrite {
            at: self.position_of(level, counter_tag),
            field: counter_field,
            value,
        };
        Some(Write::Group {
            at,
            serie,
            occurrence: matched,
            members: planned,
            counter,
        })
    }

    /// One occurrence of `group` planned from fills, as [`Self::plan_group`]
    /// plans one from a projection: merged into the occurrence whose
    /// constant members all equal the fills' constants, else appended, the
    /// counter set to the count the group then has.
    fn occurrence_write(
        &self,
        level: &Level,
        reads: &Level,
        source: &Source<'_>,
        group: &str,
        members: &[Fill],
    ) -> Option<Write> {
        let definition = self
            .registry
            .get_definition(crate::FixCategory::Groups, group)?;
        let counter_tag = definition.as_fix().counter().ok().flatten()?;
        let counter_field = stated_field(self.msg.known_by_tag(counter_tag)?);
        let at = level.position_of_group(counter_tag, definition.name());
        let empty: Vec<Option<Level>> = Vec::new();
        let (serie, occurrences) = match at {
            Some(at) => match &level.children[at] {
                Child::Group(_, occurrences) => (None, occurrences),
                Child::Flat(_, value) if value.is_null() => (None, &empty),
                Child::Flat(..) => return None,
            },
            None => {
                if level.names(definition.name()) {
                    return None;
                }
                let mut serie = definition.clone();
                serie.set_nullable(true);
                (Some(serie), &empty)
            }
        };
        let mut constants: Vec<(i32, Scalar)> = Vec::new();
        for member in members {
            if let Fill::Constant { tag, text } = *member {
                let known = self.msg.known_by_tag(tag)?;
                constants.push((
                    tag,
                    typed_spelling(self.registry, &stated_field(known), text),
                ));
            }
        }
        let matched = occurrences.iter().position(|occurrence| {
            occurrence.as_ref().is_some_and(|held| {
                constants.iter().all(|(tag, wanted)| {
                    self.stated_at(held, *tag)
                        .is_some_and(|stated| stated == wanted)
                })
            })
        });
        let blank = Level::default();
        let target = matched
            .and_then(|at| occurrences[at].as_ref())
            .unwrap_or(&blank);
        let mut planned = Vec::with_capacity(members.len());
        for member in members {
            planned.push(self.fill_write(target, reads, source, &[], member, false, false)?);
        }
        let count = occurrences.len() + usize::from(matched.is_none());
        let value = counter_field
            .scalar(Scalar::from(i64::try_from(count).ok()?))
            .ok()?;
        let counter = FieldWrite {
            at: self.position_of(level, counter_tag),
            field: counter_field,
            value,
        };
        Some(Write::Group {
            at,
            serie,
            occurrence: matched,
            members: planned,
            counter,
        })
    }

    /// Whether a target takes `value`: absent, null, or already equal.
    ///
    /// A code set states one reading of every value it declares and retires
    /// none of them, so a stated value is a stated value: there is nothing a
    /// rule may overwrite it with. What a rule fills is what the message did
    /// not say.
    fn writable(target: &Field, held: Option<&Scalar>, value: &Scalar) -> bool {
        let _ = target;
        held.is_none_or(|held| held.is_null() || held == value)
    }
}

/// One held value under the registry's field: kept where the child already
/// carries that field's datatype, re-typed otherwise.
fn retyped(registry: &FixRegistry, known: &Field, held: &Field, value: &Scalar) -> Scalar {
    if value.is_null() || held.dtype() == known.dtype() {
        return value.clone();
    }
    converted(registry, known, value)
}

/// One term of one plan bound against one shape of root under one pair of
/// parameters: what names a binding this thread may read again.
#[derive(Clone, PartialEq, Eq, Hash)]
struct BoundKey {
    /// The plan, by address: kept alive beside the binding, so the address
    /// names it for as long as the binding is remembered.
    plan: usize,
    /// The term within the plan, by address, or zero for the plan's own
    /// condition.
    term: usize,
    shape: u64,
    msgtype: Option<SmolStr>,
    group: Option<SmolStr>,
}

/// What one binding remembers: the plan it is read from, the generation of
/// the shape it was bound against, and the binding - or none, for a term
/// that does not bind against that shape.
type Remembered = (Arc<Plan>, u64, Option<Arc<Bound>>);

/// The bindings one thread has read, by what names each.
type BoundTerms = FixMap<BoundKey, Remembered>;

/// The root one shape digest currently stands for, and the generation a
/// binding read against that shape carries: a digest two shapes share
/// takes a new generation when the other shape arrives, so no binding of
/// the first is read for it.
struct ShapeRoot {
    root: Field,
    generation: u64,
}

thread_local! {
    /// Every rule term this thread has bound, by the shape of the root it
    /// was bound against: a term binds by column position and type, so a
    /// binding read against one root answers every root of that shape, and
    /// a capture states a few shapes a hundred thousand times each.
    static BOUND_TERMS: RefCell<BoundTerms> = RefCell::new(BoundTerms::default());
    /// The root each shape digest stands for. A root is compared against it
    /// once and then stands for the shape itself, so the rules read against
    /// one message's root verify its shape once between them.
    static SHAPE_ROOTS: RefCell<FixMap<u64, ShapeRoot>> = RefCell::new(FixMap::default());
    static SHAPE_GENERATIONS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many bindings one thread remembers; past it a term is still bound
/// and simply not kept.
const REMEMBERED_BINDINGS: usize = 4_096;

/// How many shapes one thread remembers a root for; past it a shape takes a
/// fresh generation on every read and its terms are bound every time.
const REMEMBERED_SHAPES: usize = 4_096;

/// The generation of the shape `root` is, whose digest is `shape`.
fn shape_generation(root: &Field, shape: u64) -> u64 {
    SHAPE_ROOTS.with(|held| {
        let mut held = held.borrow_mut();
        if let Some(known) = held.get_mut(&shape) {
            if std::ptr::eq(known.root.fields(), root.fields()) {
                return known.generation;
            }
            if same_shape(&known.root, root) {
                known.root = root.clone();
                return known.generation;
            }
        }
        let generation = SHAPE_GENERATIONS.with(|next| {
            let generation = next.get();
            next.set(generation + 1);
            generation
        });
        if held.len() < REMEMBERED_SHAPES || held.contains_key(&shape) {
            held.insert(
                shape,
                ShapeRoot {
                    root: root.clone(),
                    generation,
                },
            );
        }
        generation
    })
}

/// The text one parameter supplies, where it supplies one.
fn parameter_text(parameters: &[(&str, Scalar)], name: &str) -> Option<SmolStr> {
    parameters
        .iter()
        .find(|(held, _)| *held == name)
        .and_then(|(_, value)| value.as_str().map(SmolStr::new))
}

/// `term` of `plan` - the plan's own condition where `None` - bound against
/// `root`, whose shape digests to `shape`, under `parameters`, read once
/// per shape of root per thread; nothing where it does not bind, exactly
/// as a bind's refusal reads.
///
/// The shape digest names a bucket; the root it currently stands for says
/// whether this root is that shape, compared once per root, so a digest two
/// shapes share costs a bind and never a wrong answer.
fn bound_term(
    plan: &Arc<Plan>,
    term: Option<&Term>,
    root: &Field,
    shape: u64,
    parameters: &[(&str, Scalar)],
) -> Option<Arc<Bound>> {
    let key = BoundKey {
        plan: Arc::as_ptr(plan) as usize,
        term: term.map_or(0, |term| std::ptr::from_ref(term) as usize),
        shape,
        msgtype: parameter_text(parameters, "msgtype"),
        group: parameter_text(parameters, "group"),
    };
    let generation = shape_generation(root, shape);
    let known = BOUND_TERMS.with(|held| {
        held.borrow()
            .get(&key)
            .filter(|(_, held, _)| *held == generation)
            .map(|(_, _, bound)| bound.clone())
    });
    if let Some(bound) = known {
        return bound;
    }
    let bound = match term {
        Some(term) => term.bind_with(root, parameters),
        None => plan.filter_section().bind_with(root, parameters),
    }
    .ok()
    .map(Arc::new);
    BOUND_TERMS.with(|held| {
        let mut held = held.borrow_mut();
        if held.len() < REMEMBERED_BINDINGS || held.contains_key(&key) {
            held.insert(key, (Arc::clone(plan), generation, bound.clone()));
        }
    });
    bound
}

/// The rules `field` carries, compiled: read off the registry, which
/// compiled them once when it indexed the field, or off the field's own
/// document for a field the registry does not hold. `None` stands for a
/// rule whose text is not a plan, kept in its place so the count and the
/// order of the rules are the document's.
fn plans_of<'registry>(
    registry: &'registry FixRegistry,
    field: &Field,
) -> Cow<'registry, [Option<Arc<Plan>>]> {
    match registry.plans_of(field) {
        Some(plans) => Cow::Borrowed(plans),
        None => Cow::Owned(compiled_plans(field)),
    }
}

/// Whether writing `tag` could restate anything at all: what the
/// specification retired of it, or the rule the registry's field for it
/// states of its own.
///
/// A write of a tag no rule is about restates nothing, so a door that
/// writes pays for the rules its writes could fire and never for the pass
/// itself - which is what lets every write run it.
pub(super) fn restates(registry: &FixRegistry, tag: i32) -> bool {
    retired::rules_of(tag).is_some()
        || registry.get_field_by_tag(tag).is_some_and(|field| {
            registry.facts_of(field).map_or_else(
                || field.as_fix().replacements().next_ok().is_some(),
                |facts| facts.ruled,
            )
        })
}

/// Synchronizes one keyed occurrence of a root repeating group.
///
/// The same unpacker and packer the restating pass uses preserve every
/// unrelated occurrence and optional member. `value` replaces the one
/// occurrence whose `selector_tag` equals `selector`, removing duplicates;
/// `None` removes every matching occurrence. The group's counter is written
/// from the resulting length.
pub(super) fn sync_group_occurrence(
    msg: &mut FixMsg,
    group: &str,
    selector_tag: i32,
    selector: &str,
    value_tag: i32,
    value: Option<&str>,
) -> Result<()> {
    let registry = Arc::clone(msg.registry());
    let Some(definition) = registry
        .get_definition(crate::FixCategory::Groups, group)
        .cloned()
    else {
        return Ok(());
    };
    let Some(counter_tag) = definition.as_fix().counter()? else {
        return Ok(());
    };
    let Some(selector_field) = registry.get_field_by_tag(selector_tag).map(stated_field) else {
        return Ok(());
    };
    let Some(value_field) = registry.get_field_by_tag(value_tag).map(stated_field) else {
        return Ok(());
    };
    let Some(counter_field) = registry.get_field_by_tag(counter_tag).map(stated_field) else {
        return Ok(());
    };
    let selector = typed_spelling(&registry, &selector_field, selector);
    if selector.is_null() {
        return Ok(());
    }
    let value = value.map(|held| typed_spelling(&registry, &value_field, held));
    if value.as_ref().is_some_and(Scalar::is_null) {
        return Ok(());
    }

    let root = msg.as_field().clone();
    let Some(children) = root.dtype().as_fields() else {
        return Ok(());
    };
    let Some(values) = msg.as_value().as_sequence() else {
        return Ok(());
    };
    let mut level = Level::unpack(children, values);
    let at = match level.position_of_group(counter_tag, definition.name()) {
        Some(at) => at,
        None if value.is_none() => return Ok(()),
        None => {
            level
                .children
                .push(Child::Group(stated_field(&definition), Vec::new()));
            level.children.len() - 1
        }
    };
    if let Child::Flat(declared, held) = &level.children[at] {
        if !held.is_null() {
            return Ok(());
        }
        level.children[at] = Child::Group(declared.clone(), Vec::new());
    }
    let Child::Group(_, occurrences) = &mut level.children[at] else {
        return Ok(());
    };

    let mut matched = false;
    occurrences.retain_mut(|occurrence| {
        let is_match = occurrence.as_ref().is_some_and(|held| {
            held.position_of_field(Some(selector_tag), selector_field.name())
                .and_then(|at| held.value_at(at))
                == Some(&selector)
        });
        if !is_match {
            return true;
        }
        let Some(value) = value.as_ref() else {
            return false;
        };
        if matched {
            return false;
        }
        matched = true;
        let target = occurrence.get_or_insert_with(Level::default);
        let at = target.position_of_field(Some(value_tag), value_field.name());
        target.write_field(FieldWrite {
            at,
            field: value_field.clone(),
            value: value.clone(),
        });
        true
    });
    if let Some(value) = value.filter(|_| !matched) {
        let mut occurrence = Level::default();
        occurrence.write_field(FieldWrite {
            at: None,
            field: value_field,
            value,
        });
        occurrence.write_field(FieldWrite {
            at: None,
            field: selector_field,
            value: selector,
        });
        occurrences.push(Some(occurrence));
    }
    let Ok(count) = i64::try_from(occurrences.len()) else {
        return Ok(());
    };
    let counter = counter_field.scalar(Scalar::from(count))?;
    let counter_at = level.position_of_field(Some(counter_tag), counter_field.name());
    level.write_field(FieldWrite {
        at: counter_at,
        field: counter_field,
        value: counter,
    });

    let (fields, values) = level.pack()?;
    let root = Field::new_with_metadata(
        root.name(),
        DataType::from(StructType::from_checked_fields(fields)?),
        root.is_nullable(),
        root.as_metadata().clone(),
    );
    msg.replace_content(root, values)
}

/// Folds an older observation's FIX content into the selected reference.
///
/// The reference is the conflict base. Missing lifted and scalar facts are
/// filled, while repeating groups merge the occurrences at equal indexes and
/// their members recursively. The rebuilt levels have one logical key each
/// and deterministic FIX tag/name order.
pub(super) fn merge_content(reference: &mut FixMsg, other: &FixMsg) -> Result<()> {
    let root = reference.as_field().clone();
    let Some(reference_values) = reference.as_value().as_sequence() else {
        return Ok(());
    };
    let Some(other_values) = other.as_value().as_sequence() else {
        return Ok(());
    };
    let registry = Arc::clone(reference.registry());
    let mut merged = Level::deduplicated(&registry, Level::unpack(root.fields(), reference_values));
    let other_level = Level::deduplicated(
        &registry,
        Level::unpack(other.as_field().fields(), other_values),
    );
    merged.merge_from(&registry, other_level);
    merged.sync_group_counts(&registry)?;
    merged.sort(&registry);
    let (fields, values) = merged.pack()?;
    let root = Field::new_with_metadata(
        root.name(),
        DataType::from(StructType::from_checked_fields(fields)?),
        root.is_nullable(),
        root.as_metadata().clone(),
    );
    reference.replace_content(root, values)?;

    // These are the body facts stored beside the row. Frame fields remain
    // the selected observation's exact envelope.
    for tag in super::identity::LIFTED_TAGS
        .into_iter()
        .chain(std::iter::once(super::identity::TEXT_TAG))
    {
        let missing = reference
            .get_by_tag(tag)
            .as_ref()
            .is_none_or(Scalar::is_null);
        if missing {
            if let Some(value) = other.get_by_tag(tag).filter(|value| !value.is_null()) {
                reference.set_unsettled(tag, value)?;
            }
        }
    }
    reference.settle();
    Ok(())
}

/// Carries order-link spellings from the exact lifecycle predecessor without
/// overwriting anything the current message stated.
pub(super) fn inherit_order_links(current: &mut FixMsg, previous: &FixMsg) -> Result<bool> {
    let previous_order = previous.lifted().orderid().map(str::to_owned);
    let current_order = current.lifted().orderid().map(str::to_owned);
    let previous_clord = previous.lifted().clordid().map(str::to_owned);
    let current_clord = current.lifted().clordid().map(str::to_owned);
    let parent_missing = current
        .get_by_name("parentorderid")
        .as_ref()
        .is_none_or(|value| value.is_null() || value.as_str() == Some(""));
    let inherit_parent = parent_missing
        && matches!(
            (previous_order.as_deref(), current_order.as_deref()),
            (Some(previous), Some(current)) if previous != current
        );
    let inherit_clord = current_clord.is_none()
        && previous_clord
            .as_deref()
            .is_some_and(|previous| current_clord.as_deref() != Some(previous));
    if !inherit_parent && !inherit_clord {
        return Ok(false);
    }

    if inherit_parent {
        let root = current.as_field().clone();
        let Some(values) = current.as_value().as_sequence() else {
            return Ok(false);
        };
        let mut level = Level::unpack(root.fields(), values);
        let value = Scalar::from(previous_order.as_deref().expect("checked above"));
        match level
            .children
            .iter()
            .position(|child| crate::folds_equal(child.name(), "parentorderid"))
        {
            Some(at) => {
                if let Child::Flat(field, held) = &mut level.children[at] {
                    if held.is_null() || held.as_str() == Some("") {
                        *held = field.scalar(value)?;
                    }
                }
            }
            None => level.children.push(Child::Flat(
                DataType::utf8().nullable_field("parentorderid"),
                value,
            )),
        }
        let (fields, values) = level.pack()?;
        let root = Field::new_with_metadata(
            root.name(),
            DataType::from(StructType::from_checked_fields(fields)?),
            root.is_nullable(),
            root.as_metadata().clone(),
        );
        current.replace_content(root, values)?;
    }
    if inherit_clord {
        current.set_unsettled(
            11,
            Scalar::from(previous_clord.as_deref().expect("checked above")),
        )?;
    }
    current.settle();
    Ok(true)
}

/// Every rule `field` carries, in the document's order, each the plan it
/// is or `None` where its text is not a plan; the walk stops where the
/// document itself refuses an entry, exactly as reading the rules does.
pub(super) fn compiled_plans(field: &Field) -> Vec<Option<Arc<Plan>>> {
    let mut rules = field.as_fix().replacements();
    let mut plans = Vec::new();
    while let Some(rule) = rules.next_ok() {
        plans.push(rule.parse_plan().ok().map(Arc::new));
    }
    plans
}

/// The message restated under the dictionary its registry holds.
///
/// The levels are rebuilt whole, because canonicalizing merges and drops
/// children and a group's item is the union of what its occurrences hold.
/// The `version` a row carries is left as it is: it is what the line said,
/// and no pass here pins one.
///
/// Idempotent, so a message restated again is the message: a stated value
/// is never overwritten, and what one pass wrote the next finds stated.
/// That is what lets every door that writes a value run it - the
/// [parse](super::enrich::enrich) once per message, and
/// [`FixMsg::set`](FixMsg::set) for the tags one write states, which
/// [`restates`] answers before the pass is entered at all.
///
/// # Errors
///
/// Returns the schema grammar's refusal when the restated children do not
/// make a root, or the refusal [`FixMsg::with_registry`] raises.
pub(super) fn restate(msg: &mut FixMsg) -> Result<()> {
    let root = msg.as_field();
    let Some(children) = root.dtype().as_fields() else {
        return Ok(());
    };
    let values = msg.as_value().as_sequence().unwrap_or_default();
    let (fields, values) = {
        let restater = Restater {
            msg,
            registry: msg.registry(),
            msgtype: Some(msg.header().msgtype()).filter(|held| !held.is_empty()),
        };
        // A message the builder stated under the dictionary's own fields is
        // the message this pass would answer: nothing to rename, merge,
        // retype or replace, so nothing is rebuilt. One a rule applies to
        // is that message but for the rule's writes, which land on the
        // level as it arrived.
        let shape = shape_digest(root, false);
        match restater.shape(children, values, shape) {
            Shape::Canonical => return Ok(()),
            Shape::Ruled => {
                let mut level = Level::unpack(children, values);
                restater.restated(&mut level, None, Some((root, msg.as_value(), shape)));
                level.pack()?
            }
            Shape::Rebuilt => restater
                .level(Level::unpack(children, values), None)?
                .pack()?,
        }
    };
    // The children were each checked by `from_fields`, which is what the
    // root's setter would check a second time before comparing every child.
    let root = Field::new_with_metadata(
        root.name(),
        DataType::from(StructType::from_checked_fields(fields)?),
        root.is_nullable(),
        root.as_metadata().clone(),
    );
    msg.replace_content(root, values)?;
    Ok(())
}
