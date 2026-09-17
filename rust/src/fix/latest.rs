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
//! it, and the [`fix:replacements`](super::replacements) each retired field
//! or value carries. Nothing here holds a table of rules.
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

use smol_str::SmolStr;

use super::build::{stated as stated_field, typed_spelling};
use super::entry::wire_text;
use super::msg::FixMsg;
use super::schema::item_fields;
use super::{FixRegistry, occurrence_name};
use crate::expression::Term;
use crate::types::Code;
use crate::{DataType, Field, Plan, Result, Scalar};

/// One level of the row: the root, or one occurrence of a repeating group.
///
/// Held unpacked while the pass works on it - a group's occurrences as
/// levels of their own rather than as the List value they pack into - so a
/// member written into one occurrence is an ordinary child write, and the
/// List is rebuilt once at the end as the union of what its occurrences
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
    /// A repeating group: its List field, and each occurrence unpacked -
    /// `None` where the row states no occurrence at that index.
    Group(Field, Vec<Option<Level>>),
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
    /// `occurrence`, appended when there is none; the List itself created
    /// from `list` when the level holds none. The counter child is set to
    /// the count the group then has.
    Group {
        at: Option<usize>,
        list: Option<Field>,
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

    /// The fields and values this level packs back into, every group a List
    /// of the union of its occurrences' members.
    fn pack(self) -> Result<(Vec<Field>, Vec<Scalar>)> {
        let mut fields = Vec::with_capacity(self.children.len());
        let mut values = Vec::with_capacity(self.children.len());
        for child in self.children {
            let (field, value) = match child {
                Child::Flat(field, value) => (field, value),
                Child::Group(list, occurrences) => pack_group(list, occurrences)?,
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
    /// A declared group stating no occurrence - a null List a schema
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
            self.children.iter().position(|child| {
                is_group(child) && crate::types::folds_equal(child.field().name(), name)
            })
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
                list,
                occurrence,
                members,
                counter,
            } => {
                let at = match (at, list) {
                    (Some(at), _) => at,
                    (None, Some(list)) => {
                        self.children.push(Child::Group(list, Vec::new()));
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
    /// A group is a List of Structs carrying `fix:counter`; a List no counter
    /// heads is a repeated flat field and stays whole, because the registry's
    /// scalar field cannot hold it.
    fn unpack(field: Field, value: Scalar) -> Self {
        if field.as_fix().counter().ok().flatten().is_none() {
            return Self::Flat(field, value);
        }
        let Some(members) = item_fields(&field) else {
            return Self::Flat(field, value);
        };
        let Some(rows) = value.as_sequence() else {
            return Self::Flat(field, value);
        };
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

/// One group packed back into its List: the item is the union of every
/// occurrence's members in first-seen order, each nullable because an
/// occurrence need not state one, exactly as the builder closes a group.
///
/// A group holding no occurrence keeps the List it arrived as: there is
/// nothing to rebuild the item from, and the declared shape is the best
/// statement of what its occurrences would hold.
fn pack_group(list: Field, occurrences: Vec<Option<Level>>) -> Result<(Field, Scalar)> {
    let mut finished: Vec<Option<(Vec<Field>, Vec<Scalar>)>> =
        Vec::with_capacity(occurrences.len());
    for occurrence in occurrences {
        finished.push(occurrence.map(Level::pack).transpose()?);
    }
    if finished.iter().all(Option::is_none) {
        let rows = finished.into_iter().map(|_| Scalar::Null);
        return Ok((list, Scalar::from_sequence(rows)));
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
    let mut item = DataType::from_fields(members)?.required_field(occurrence_name(&list));
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
    let dtype = match list.dtype() {
        DataType::LargeList(_) => DataType::large_list(item),
        _ => DataType::list(item),
    };
    let mut rebuilt = dtype.required_field(list.name());
    rebuilt.set_nullable(list.is_nullable());
    rebuilt.set_metadata(list.as_metadata().iter())?;
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
    if matches!(held, Scalar::Code(Code::State(_))) {
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
    // The walk is the point; the term it rebuilds is dropped.
    let _ = term.map(&mut |node| {
        if let Term::Literal(literal) = node {
            if let Some(text) = literal.value().as_str() {
                held.push(SmolStr::new(text));
            }
        }
        Ok(None)
    });
    held
}

/// The members of the one occurrence a group projection states.
///
/// A group occurrence is spelled `[{member: term, ...}] as group`: a list of
/// exactly one record, because a rule fills one occurrence and the List is
/// what the row holds a group as. The grammar reads `{member: term}` as a
/// map whose keys are paths, and a member is the one name its key spells;
/// anything else names no occurrence.
fn occurrence_members(term: &Term) -> Option<Vec<(SmolStr, &Term)>> {
    let Term::List(items) = term else {
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
fn converted(target: &Field, value: &Scalar) -> Scalar {
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
        Some(text) => typed_spelling(target, &text),
        None => Scalar::Null,
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
        if let Ok(Some(tag)) = child.as_fix().tag() {
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

    /// One level canonicalized and restated, its group occurrences first.
    fn level(&self, level: Level, group: Option<&str>) -> Result<Level> {
        let mut level = self.canonicalized(level)?;
        self.restated(&mut level, group);
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
                    .map(|known| (known, retyped(known, field, value))),
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
                (None, Child::Group(list, occurrences)) => {
                    let mut restated = Vec::with_capacity(occurrences.len());
                    for occurrence in occurrences {
                        restated.push(
                            occurrence
                                .map(|held| self.level(held, Some(list.name())))
                                .transpose()?,
                        );
                    }
                    out.push(Child::Group(list, restated));
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
    fn restated(&self, level: &mut Level, group: Option<&str>) {
        let mut sources: Vec<(i32, usize)> = level
            .children
            .iter()
            .enumerate()
            .filter_map(|(at, child)| match child {
                Child::Flat(field, value) if !value.is_null() => {
                    Some((field.as_fix().tag().ok().flatten()?, at))
                }
                _ => None,
            })
            .collect();
        sources.sort_unstable();
        let parameters = self.parameters(group);
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
                    let mut rules = field.as_fix().replacements();
                    let mut count = 0;
                    // The level's view is built on the first rule that has
                    // to be read against it and never before, so a field
                    // with no rule costs no clone at all.
                    let mut view = None;
                    while let Some(rule) = rules.next_ok() {
                        count += 1;
                        if planned.is_some() {
                            continue;
                        }
                        let Ok(plan) = rule.parse_plan() else {
                            continue;
                        };
                        if view.is_none() {
                            view = Self::view(level);
                        }
                        let Some((root, row)) = view.as_ref() else {
                            break;
                        };
                        if !Self::applies(&plan, root, row, &parameters) {
                            continue;
                        }
                        let source = Source { value, tag };
                        before = Some(value.clone());
                        planned = Some(self.plan(
                            level,
                            root,
                            row,
                            &source,
                            &plan,
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
    /// Built only where a field carries a rule and holds a value - which is
    /// thirty-seven fields of six thousand - so the clone it costs is paid
    /// once per firing rather than once per child. Rebuilt after a rule's
    /// writes land, because the next rule reads the level as it then is.
    fn view(level: &Level) -> Option<(Field, Scalar)> {
        let (fields, values) = level.clone().pack().ok()?;
        let root = DataType::from_fields(fields).ok()?.required_field("row");
        Some((root, Scalar::from_sequence(values)))
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
    fn applies(plan: &Plan, root: &Field, row: &Scalar, parameters: &[(&str, Scalar)]) -> bool {
        plan.filter_section()
            .bind_with(root, parameters)
            .and_then(|bound| bound.matches(row))
            .unwrap_or(false)
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
        source: &Source<'_>,
        plan: &Plan,
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
                root,
                row,
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
        root: &Field,
        row: &Scalar,
        parameters: &[(&str, Scalar)],
        rooted: bool,
    ) -> Option<Write> {
        if let Some(definition) = self
            .registry
            .get_definition(crate::FixCategory::Groups, name)
        {
            let members = occurrence_members(term)?;
            return self.plan_group(writes, definition, &members, root, row, parameters);
        }
        let value = term.bind_with(root, parameters).ok()?.eval(row).ok()?;
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
                Some(text) => typed_spelling(&target, &restated_tokens(source.value, named, text)),
                None => converted(&target, &value),
            },
            _ => converted(&target, &value),
        };
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
    fn plan_group(
        &self,
        level: &Level,
        definition: &Field,
        members: &[(SmolStr, &Term)],
        root: &Field,
        row: &Scalar,
        parameters: &[(&str, Scalar)],
    ) -> Option<Write> {
        let counter_tag = definition.as_fix().counter().ok().flatten()?;
        let counter_field = stated_field(self.msg.known_by_tag(counter_tag)?);
        let at = level.position_of_group(counter_tag, definition.name());
        let empty: Vec<Option<Level>> = Vec::new();
        let (list, occurrences) = match at {
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
                let mut list = definition.clone();
                list.set_nullable(true);
                (Some(list), &empty)
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
            constants.push((tag, converted(&stated_field(known), literal.value())));
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
                root,
                row,
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
            list,
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
fn retyped(known: &Field, held: &Field, value: &Scalar) -> Scalar {
    if value.is_null() || held.dtype() == known.dtype() {
        return value.clone();
    }
    converted(known, value)
}

/// The message restated under the dictionary its registry holds.
///
/// The levels are rebuilt whole, because canonicalizing merges and drops
/// children and a group's item is the union of what its occurrences hold.
/// The `version` a row carries is left as it is: it is what the line said,
/// and no pass here pins one.
///
/// # Errors
///
/// Returns the schema grammar's refusal when the restated children do not
/// make a root, or the refusal [`FixMsg::with_registry`] raises.
pub(super) fn restate(mut msg: FixMsg) -> Result<FixMsg> {
    let root = msg.as_field();
    let Some(children) = root.dtype().as_fields() else {
        return Ok(msg);
    };
    let values = msg.as_value().as_sequence().unwrap_or_default();
    let (fields, values) = {
        let restater = Restater {
            msg: &msg,
            registry: msg.registry(),
            msgtype: Some(msg.header().msgtype()).filter(|held| !held.is_empty()),
        };
        restater
            .level(Level::unpack(children, values), None)?
            .pack()?
    };
    // The children were each checked by `from_fields`, which is what the
    // root's setter would check a second time before comparing every child.
    let root = Field::new_with_metadata(
        root.name(),
        DataType::from_fields(fields)?,
        root.is_nullable(),
        root.metadata.clone(),
    );
    msg.replace_content(root, values)?;
    Ok(msg)
}
