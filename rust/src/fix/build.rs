//! One builder, from key/value pairs to a typed message.
//!
//! Every reader splits its own dialect and hands the pairs here, so there is
//! one nesting builder, one fold, one code translation and one value contract
//! under all of them. Nothing else in the FIX layer turns bytes into fields.
//!
//! # What a key may be
//!
//! | key | means |
//! | --- | --- |
//! | `54`, `"54"` | a tag, through the strict parse that refuses `+35` and `3x` |
//! | `Side`, `side`, `" Side "`, `msg_type`, `Msg Type` | a name, trimmed and folded |
//! | `Instrument.Symbol` | a path |
//! | `PartyID[0]`, `PartyID[1]` | one field, two occurrences, in order |
//! | `NoPartyIDs[0].PartyID` | which group, which occurrence, which member |
//! | `VenueOwnThing` | an unknown name, kept |
//! | `""`, `"   "` | dropped |
//!
//! # What it refuses to lose
//!
//! An unknown tag is kept as nullable `utf8` under its decimal spelling, and
//! an unknown name under its own. Every venue sends fields no dictionary has,
//! and dropping them loses data. A value that will not type is null rather
//! than a failure, with the raw text still in the entries and the refusal
//! readable through the message's anomalies: a null nobody can explain is
//! worse than the value that actually arrived.

use smol_str::{SmolStr, format_smolstr};

use super::entry::FixEntry;
use super::{FixBranch, FixRegistry, STANDARD_HEADER_TAGS, STANDARD_TRAILER_TAGS, occurrence_name};
use crate::{DataType, Field, Result, Scalar, Version};

/// What a key resolved to, before any field is built.
enum Located<'key> {
    /// One flat child, by tag or by name.
    Flat,
    /// One occurrence of a repeating group's member.
    Grouped {
        group: &'key str,
        occurrence: usize,
        member: &'key str,
    },
    /// One occurrence of a repeated flat field.
    Repeated { name: &'key str, occurrence: usize },
}

/// One key split into what it addresses.
struct Key<'key> {
    text: &'key str,
    located: Located<'key>,
}

impl<'key> Key<'key> {
    /// Reads the location a rendered key carries.
    ///
    /// The key states group, occurrence and member itself, so no message
    /// grammar is needed to build real nesting from it. Inferring a group
    /// from bare repetition alone is a different problem and needs one.
    fn parse(text: &'key str) -> Self {
        let located = match text.split_once('[') {
            Some((head, rest)) => match rest.split_once(']') {
                Some((index, tail)) => index
                    .parse::<usize>()
                    .ok()
                    .map(|occurrence| match tail.strip_prefix('.') {
                        Some(member) if !member.is_empty() => Located::Grouped {
                            group: head,
                            occurrence,
                            member,
                        },
                        _ => Located::Repeated {
                            name: head,
                            occurrence,
                        },
                    })
                    .unwrap_or(Located::Flat),
                None => Located::Flat,
            },
            None => Located::Flat,
        };
        Self { text, located }
    }
}

/// One child under construction, with every occurrence it has been given.
struct Slot {
    field: Field,
    tag: i32,
    /// Whether the field is the dictionary's own, and so carries the tag it
    /// resolved to.
    ///
    /// A key no dictionary explains keeps its spelling and no `fix:tag`, so
    /// the message's tag index leaves it out - which is what a reader of the
    /// finished field would find, read once here instead of once per child.
    known: bool,
    values: Vec<Scalar>,
    /// Whether this slot is a repeating group, whatever it has been given.
    ///
    /// A group whose counter arrived and whose members did not is still a
    /// group, and the empty list is what says the count was not met.
    group: bool,
    /// The group members, when this slot is a repeating group.
    ///
    /// Each member keeps the field it resolved to rather than only its name.
    /// The field is what carries the member's tag and its datatype, and
    /// without them nothing inside a group can be addressed - which is the
    /// whole of how a party is found by its role.
    occurrences: Vec<Vec<(Field, Scalar)>>,
}

/// Builds a message's field and value from pairs, with the entries beside it.
pub(super) struct Builder<'registry> {
    registry: &'registry FixRegistry,
    branch: FixBranch,
    version: Option<Version>,
    slots: Vec<Slot>,
    /// Each slot's name digested, beside the slot.
    ///
    /// A key finds its slot by one integer compare per slot and one string
    /// compare on the hit, where comparing the names outright made a wide
    /// bridge row quadratic in its keys.
    hashes: Vec<u64>,
    entries: Vec<FixEntry>,
    /// The tag of every entry recorded so far, in arrival order.
    ///
    /// A member nests under the latest arrival of its counter, and finding
    /// that arrival walks the whole record - which is worth doing only where
    /// the counter arrived at all. One integer per entry answers that
    /// before the walk, and a document of fifty attributes under a group no
    /// counter introduced walks nothing.
    recorded: Vec<i32>,
}

impl<'registry> Builder<'registry> {
    /// Opens a build against one dictionary, dialect and version.
    pub(super) fn new(
        registry: &'registry FixRegistry,
        branch: FixBranch,
        version: Option<Version>,
        capacity: usize,
    ) -> Self {
        Self {
            registry,
            branch,
            version,
            slots: Vec::with_capacity(capacity),
            hashes: Vec::with_capacity(capacity),
            entries: Vec::with_capacity(capacity),
            recorded: Vec::with_capacity(capacity),
        }
    }

    /// Folds one arriving pair in.
    ///
    /// The value is typed from the raw bytes, never from the entry's text:
    /// the lossy decode exists only to fill the entry, and typing through it
    /// would destroy a `data` field's bytes on the way in. The ordering is
    /// the whole rule.
    pub(super) fn push(&mut self, key: &[u8], value: &[u8]) {
        let key_text = String::from_utf8_lossy(key);
        let key_text = key_text.trim();
        if key_text.is_empty() || value.is_empty() {
            // `54=` is a malformed message, not an absent side.
            return;
        }
        let value_text = String::from_utf8_lossy(value);
        let located = Key::parse(key_text);
        match located.located {
            Located::Flat => self.push_flat(located.text, &value_text, value),
            Located::Repeated { name, occurrence } => {
                self.push_repeated(name, occurrence, located.text, &value_text, value);
            }
            Located::Grouped {
                group,
                occurrence,
                member,
            } => self.push_grouped(group, occurrence, member, located.text, &value_text, value),
        }
    }

    /// The registry field one key names, and the tag it carries.
    ///
    /// A name is looked for in this message's branch and then in the standard
    /// one, which is the tier [`FixMsg`](super::FixMsg) reads a built message
    /// by: a row transcribed against a venue's dictionary names that venue's
    /// fields by the venue's spellings while still carrying `MsgType`,
    /// `SenderCompID` and every other specification field. Building it under
    /// one branch alone would drop exactly those identities before a reader
    /// could ask for them.
    fn resolve(&self, key: &str) -> Option<(&'registry Field, i32)> {
        let field = if let Some(tag) = super::field::parse_tag(key) {
            self.registry.get_primitive_field(tag)
        } else {
            self.by_path(key, &self.branch)
                .or_else(|| {
                    (!self.branch.is_standard())
                        .then(|| self.by_path(key, &super::FixBranch::STANDARD))
                        .flatten()
                })
                .filter(|field| !super::registry::is_nested(field))
        }?;
        let tag = field.as_fix().tag().ok().flatten().unwrap_or(0);
        Some((field, tag))
    }

    /// One name looked for in exactly one dictionary.
    fn by_path(&self, key: &str, branch: &FixBranch) -> Option<&'registry Field> {
        self.registry.get_field_by_path(key, Some(branch))
    }

    /// A group's own field, under the same tier a member resolves by.
    fn by_group(&self, group: &str) -> Option<&'registry Field> {
        self.registry
            .get_field_by_name(group, Some(&self.branch))
            .or_else(|| {
                (!self.branch.is_standard())
                    .then(|| {
                        self.registry
                            .get_field_by_name(group, Some(&FixBranch::STANDARD))
                    })
                    .flatten()
            })
    }

    /// The field a key builds under, as the dictionary declares it.
    ///
    /// A field the dictionary knows is the registry's own, under the name and
    /// the datatype it holds at every version: a tag is one column whatever
    /// spelling a version gave it, and a row that renamed itself per version
    /// is a row no two captures share. What each version called it stays
    /// readable through the field's [lineage](super::lineage). One the
    /// dictionary does not know is kept under the key's own folded spelling
    /// as nullable text, because a venue sends fields no dictionary has.
    fn field_for(&self, key: &str) -> (Field, i32, bool) {
        match self.resolve(key) {
            Some((known, tag)) => (stated(known), tag, true),
            None => {
                let name = folded_name(key);
                let tag = super::field::parse_tag(key).unwrap_or(0);
                (DataType::Utf8.nullable_field(name), tag, false)
            }
        }
    }

    /// Types one value under one field, translating its code first.
    ///
    /// A value that will not type is null rather than a failure: the raw text
    /// stays in the entries, the refusal is readable, and the message still
    /// answers `Ok`. A parse error is for input that is not a message at all.
    fn typed(&self, field: &Field, raw: &[u8], text: &str) -> Scalar {
        let view = field.as_fix();
        // A spelling this field states as its own absence types as null while
        // the entry keeps the text: which spelling means "nothing was sent" is
        // a fact about the field, and the row is the interpretation where the
        // entries are what arrived. The capture-wide list is the other half of
        // the pair and was applied before this key was resolved at all.
        if view.is_null_value(text) {
            return Scalar::Null;
        }
        let translated = match self.version {
            Some(at) => view.code_value_at(at, text),
            None => view.code_value(text),
        };
        let spelling = translated.unwrap_or(text);
        // A `data` field's value is bytes, and the row is where they live:
        // the entry holds a lossy decode of them and this does not.
        if is_binary(field.dtype()) {
            return field
                .scalar(Scalar::from(raw.to_vec()))
                .unwrap_or(Scalar::Null);
        }
        // Every wire value is text, and the generic value contract does not
        // read text as a number, an instant or a flag. Two of those it can
        // learn from the field alone, which is the crate's own coercion; the
        // third is a FIX *spelling* and stays here, because the generic
        // contract must not learn one.
        // A FIX spelling is rewritten into the one the crate's coercion reads,
        // and then coerced like any other text: `20240102-10:15:30` is a
        // timestamp only after both steps, and the value contract reads
        // neither a separator-free instant nor a bare `Y`.
        let candidate =
            wire_spelling(field.dtype(), spelling).unwrap_or_else(|| Scalar::from(spelling));
        // The text contract reads the spelling and hands the value through
        // the field's own contract, so what it answers is already the stored
        // form and is not checked a second time. A spelling it refuses is
        // offered to the field as it stands, which is where a raw payload
        // that is not a spelling of anything still lands.
        crate::text::prepare_text(candidate, field)
            .or_else(|_| field.scalar(Scalar::from(spelling)))
            .unwrap_or(Scalar::Null)
    }

    /// One flat child, appended in arrival order.
    fn push_flat(&mut self, key: &str, text: &str, raw: &[u8]) {
        // A flat key naming a repeating group is that group's counter. The
        // row holds the group as a List and its length *is* the count, so the
        // number that arrived stays in the entries and the two are compared
        // on demand through `anomalies()`. Writing it into the row as well
        // would put two facts about one thing at one tag.
        //
        // A tag reaches the dictionary once, and which half it is in decides
        // the rest: the counter and the scalar readings are the same probe
        // filtered two ways, and a numeric key is most of every frame.
        let (field, tag, known) = if let Some(parsed) = super::field::parse_tag(key) {
            match self.registry.get_field_by_tag(parsed) {
                Some(found) => {
                    let tag = found.as_fix().tag().ok().flatten().unwrap_or(0);
                    if super::registry::is_nested(found) {
                        self.record(key, text, tag);
                        let slot = self.slot_for(stated(found), tag, true);
                        slot.group = true;
                        return;
                    }
                    (stated(found), tag, true)
                }
                None => (
                    DataType::Utf8.nullable_field(folded_name(key)),
                    parsed,
                    false,
                ),
            }
        } else {
            if let Some((field, tag)) = self.counter(key) {
                self.record(key, text, tag);
                let slot = self.slot_for(field, tag, true);
                slot.group = true;
                return;
            }
            self.field_for(key)
        };
        let value = self.typed(&field, raw, text);
        self.record(key, text, tag);
        self.slot_for(field, tag, known).values.push(value);
    }

    /// The repeating group a flat key names, when it names one.
    ///
    /// Only the nested half is probed, which is what makes this a counter
    /// rather than a second reading of an ordinary key: a scalar reaches its
    /// field through `get_primitive_field` and neither half can answer for
    /// the other.
    fn counter(&self, key: &str) -> Option<(Field, i32)> {
        let field = match super::field::parse_tag(key) {
            Some(tag) => self.registry.get_nested_field(tag),
            None => self.registry.get_nested_field(key),
        }?;
        let tag = field.as_fix().tag().ok().flatten().unwrap_or(0);
        Some((stated(field), tag))
    }

    /// One occurrence of a repeated flat field, placed by index.
    fn push_repeated(&mut self, name: &str, occurrence: usize, key: &str, text: &str, raw: &[u8]) {
        let (field, tag, known) = self.field_for(name);
        let value = self.typed(&field, raw, text);
        self.record(key, text, tag);
        let slot = self.slot_for(field, tag, known);
        // Indices may be partial or out of order, so occurrences are built by
        // index and a gap is null.
        while slot.values.len() <= occurrence {
            slot.values.push(Scalar::Null);
        }
        slot.values[occurrence] = value;
    }

    /// One member of one occurrence of a repeating group.
    fn push_grouped(
        &mut self,
        group: &str,
        occurrence: usize,
        member: &str,
        key: &str,
        text: &str,
        raw: &[u8],
    ) {
        let (member_field, member_tag, _) = self.field_for(member);
        let value = self.typed(&member_field, raw, text);

        // The same resolution the flat counter uses, so a group addressed by
        // its tag and one addressed by its name reach one slot: `FixKey` reads
        // every string as a name, so a numeric key resolves only tag-first,
        // and the dictionary's own field answers for both - two spellings of
        // one group would otherwise build two columns carrying one tag.
        let (group_field, group_tag, known) = match self.counter(group) {
            Some((field, tag)) => (field, tag, true),
            None => match self.by_group(group) {
                Some(known) => {
                    let tag = known.as_fix().tag().ok().flatten().unwrap_or(0);
                    (stated(known), tag, true)
                }
                None => (
                    DataType::Utf8.nullable_field(folded_name(group)),
                    super::field::parse_tag(group).unwrap_or(0),
                    false,
                ),
            },
        };
        // Recorded after the group resolves, so the entry can ride under the
        // counter pair that heads it - when that pair actually arrived.
        self.record_under(group_tag, key, text, member_tag);
        let slot = self.slot_for(group_field, group_tag, known);
        slot.group = true;
        while slot.occurrences.len() <= occurrence {
            slot.occurrences.push(Vec::new());
        }
        slot.occurrences[occurrence].push((member_field, value));
    }

    /// The slot one field builds into, created on first use.
    ///
    /// Found by name, because the name is what a child is: two keys the
    /// dictionary resolves to one field build one column, and two unknown
    /// keys folding to one spelling do too. The digest is compared first
    /// and the name only where it agrees, so a hit costs one string compare
    /// and a miss costs none.
    fn slot_for(&mut self, field: Field, tag: i32, known: bool) -> &mut Slot {
        let hash = crate::xxhash::xxh64(field.name().as_bytes());
        let held = self
            .hashes
            .iter()
            .enumerate()
            .find(|(index, held)| **held == hash && self.slots[*index].field.name() == field.name())
            .map(|(index, _)| index);
        match held {
            Some(index) => &mut self.slots[index],
            None => {
                self.hashes.push(hash);
                self.slots.push(Slot {
                    field,
                    tag,
                    known,
                    values: Vec::new(),
                    group: false,
                    occurrences: Vec::new(),
                });
                self.slots.last_mut().expect("just pushed")
            }
        }
    }

    /// Records what arrived under the counter that heads it, where one did.
    ///
    /// The counter pair itself must have arrived: an entry is the arrival
    /// record, and a parent nobody sent would be an invention. A member whose
    /// counter never arrived stays flat at the top, exactly as a wire with no
    /// stated structure keeps it, and the latest arrival of the counter is
    /// the one that takes the member - which is what nests each occurrence
    /// under its own heading.
    fn record_under(&mut self, counter_tag: i32, key: &str, value: &str, tag: i32) {
        let entry = FixEntry::new(tag, key, value);
        let mut entry = if tag == 0 {
            entry
        } else {
            entry.with_branch(&self.branch)
        };
        self.recorded.push(tag);
        // The walk finds a counter only where one was recorded, so a record
        // holding none is not walked for it.
        if counter_tag != 0 && self.recorded.contains(&counter_tag) {
            for root in self.entries.iter_mut().rev() {
                match root.adopt(counter_tag, entry) {
                    None => return,
                    Some(back) => entry = back,
                }
            }
        }
        self.entries.push(entry);
    }

    /// Records what arrived, whatever the row made of it.
    fn record(&mut self, key: &str, value: &str, tag: i32) {
        // A key that named no field resolved in no dialect, so it keeps the
        // standard digest the constructor set; anything the dictionary
        // answered carries the branch that answered it.
        let entry = FixEntry::new(tag, key, value);
        let entry = if tag == 0 {
            entry
        } else {
            entry.with_branch(&self.branch)
        };
        self.recorded.push(tag);
        self.entries.push(entry);
    }

    /// Closes the build into a root field, its value, and the entries.
    ///
    /// Order is the standard header, then the body in arrival order, then the
    /// standard trailer - flat, with no `StandardHeader` Struct, because a
    /// message is laid flat and the header is an ordering rather than a
    /// nesting.
    ///
    /// Beside the row comes the tag index a message reads it by - each
    /// dictionary child's tag and its position, tag-major - because the
    /// builder resolved every tag once already and the message would only
    /// read them back out of the fields it just wrote.
    pub(super) fn finish(self, name: &str) -> Result<Built> {
        let Self { slots, entries, .. } = self;
        // Each slot's place is read once, as a rank, rather than once per
        // comparison inside the sort.
        let mut ordered: Vec<(usize, Slot)> = Vec::with_capacity(slots.len());
        let mut rest: Vec<Slot> = Vec::with_capacity(slots.len());
        let mut trailing: Vec<(usize, Slot)> = Vec::new();
        for slot in slots {
            if let Some(rank) = rank_in(&STANDARD_HEADER_TAGS, slot.tag) {
                ordered.push((rank, slot));
            } else if let Some(rank) = rank_in(&STANDARD_TRAILER_TAGS, slot.tag) {
                trailing.push((rank, slot));
            } else {
                rest.push(slot);
            }
        }
        ordered.sort_by_key(|(rank, _)| *rank);
        trailing.sort_by_key(|(rank, _)| *rank);
        let count = ordered.len() + rest.len() + trailing.len();
        let ordered = ordered
            .into_iter()
            .map(|(_, slot)| slot)
            .chain(rest)
            .chain(trailing.into_iter().map(|(_, slot)| slot));

        let mut fields = Vec::with_capacity(count);
        let mut values = Vec::with_capacity(count);
        let mut tags = Vec::with_capacity(count);
        for (index, slot) in ordered.enumerate() {
            if slot.known {
                tags.push((slot.tag, index));
            }
            let (field, value) = slot.into_child()?;
            fields.push(field);
            values.push(value);
        }
        tags.sort_unstable();
        let root = DataType::from_fields(fields)?.required_field(name);
        Ok(Built {
            field: root,
            value: Scalar::from_sequence(values),
            entries,
            tags,
        })
    }
}

/// What one build finishes as: the row's schema and value, what arrived,
/// and where each dictionary tag sits.
pub(super) struct Built {
    pub(super) field: Field,
    pub(super) value: Scalar,
    pub(super) entries: Vec<FixEntry>,
    pub(super) tags: Vec<(i32, usize)>,
}

/// Where one tag sits in a component's declared order, if it is in it.
fn rank_in(component: &[i32], tag: i32) -> Option<usize> {
    component.iter().position(|held| *held == tag)
}

impl Slot {
    /// This slot as one child field and its value.
    fn into_child(self) -> Result<(Field, Scalar)> {
        // A group whose counter arrived and whose members did not is a group
        // holding nothing, not a scalar: the empty list is what lets the
        // count it stated be compared with what the row actually holds.
        if self.group && self.occurrences.is_empty() {
            if self.values.is_empty() {
                let value = Scalar::from_sequence(Vec::new());
                return Ok((self.field, value));
            }
            // An occurrence that named no member is still one: a bridge writes
            // `NOPARTYIDS[0]=ONE` where it has nothing to name, and dropping
            // those would lose what arrived to say the group held nothing.
            // They have no field of their own, so they are what `field_for`
            // typed them as - text.
            let absent = self.values.iter().any(Scalar::is_null);
            // One group has one occurrence name whatever the occurrence's
            // type, so the stand-in is named exactly as a member-bearing one.
            let item = DataType::Utf8.named_field(occurrence_name(&self.field), absent);
            let values = Scalar::from_sequence(self.values);
            let mut list = DataType::list(item).required_field(self.field.name());
            let _ = list.set_metadata(self.field.as_metadata().iter());
            return Ok((list, values));
        }
        if self.occurrences.is_empty() {
            // A value that would not type is null, and a field a null lands
            // in is nullable: the message's schema says the value is there
            // only where it actually is.
            let absent = self.values.iter().any(Scalar::is_null);
            if self.values.len() <= 1 {
                let value = self.values.into_iter().next().unwrap_or(Scalar::Null);
                let mut field = self.field;
                if absent || value.is_null() {
                    field.set_nullable(true);
                }
                return Ok((field, value));
            }
            // A tag appearing twice stays two occurrences in input order: a
            // map keyed by tag would lose a repeating group. Indices may be
            // gapped, and a gap is null, so the item takes that nullability.
            // Not a group: no counter arrived, so there is no component to
            // name. The occurrence takes the repeated field's own name, which
            // is what keeps this shape distinguishable from a repeating group.
            let mut item = self.field.clone().with_name(self.field.name().to_owned());
            item.set_nullable(absent);
            let values = Scalar::from_sequence(self.values);
            let mut list = DataType::list(item).required_field(self.field.name());
            let _ = list.set_metadata(self.field.as_metadata().iter());
            return Ok((list, values));
        }

        // A group is a List of a non-null `item` Struct, and the occurrences
        // are built by index, so a gap is an empty one rather than a shift.
        // In first-seen order across every occurrence, so a member only the
        // second occurrence carries is still a column and still in its
        // arrival place. Nullable, because an occurrence need not state one.
        let mut member_fields: Vec<Field> = Vec::new();
        for occurrence in &self.occurrences {
            for (field, _) in occurrence {
                if member_fields.iter().any(|held| held.name() == field.name()) {
                    continue;
                }
                let mut member = field.clone();
                member.set_nullable(true);
                member_fields.push(member);
            }
        }
        let mut item = DataType::from_fields(member_fields.clone())?
            .required_field(occurrence_name(&self.field));
        // A gapped index leaves an occurrence nobody stated, which is null.
        if self.occurrences.iter().any(Vec::is_empty) {
            item.set_nullable(true);
        }
        let mut rows = Vec::with_capacity(self.occurrences.len());
        for occurrence in self.occurrences {
            if occurrence.is_empty() {
                rows.push(Scalar::Null);
                continue;
            }
            let mut row = Vec::with_capacity(member_fields.len());
            for field in &member_fields {
                let held = occurrence
                    .iter()
                    .find(|(held, _)| held.name() == field.name())
                    .map(|(_, value)| value.clone());
                row.push(held.unwrap_or(Scalar::Null));
            }
            rows.push(Scalar::from_sequence(row));
        }
        let mut list = DataType::list(item).required_field(self.field.name());
        let _ = list.set_metadata(self.field.as_metadata().iter());
        Ok((list, Scalar::from_sequence(rows)))
    }
}

/// The day a FIX temporal that states no date is read on.
///
/// `TZTimeOnly` is a time of day and an offset, so the instant it names needs
/// a day and the specification supplies none. The epoch day is the one choice
/// that costs nothing: the count is the time of day itself, in the unit the
/// column declares, and two readings still subtract.
const EPOCH_DAY: &str = "1970-01-01";

/// The value one FIX wire spelling states, where FIX spells it its own way.
///
/// A boolean is `Y` or `N`, and a temporal is a run of digits with no
/// separators a general parser would recognize: `20260821-10:30:00.123456`,
/// `20260821`, `10:30:00.000000`. These are facts about FIX rather than about
/// the datatype, so they are read here and the generic value contract learns
/// none of them.
///
/// Three FIX datatypes land on `DateTime64` and this reads all three, because
/// only their spelling differs: `UTCTimestamp` states a date and no zone,
/// `TZTimestamp` states both, and `TZTimeOnly` states a zone and no date. So
/// the date is taken where there is one and the epoch day stands in where
/// there is not, the seconds a `TZTimeOnly` may omit are filled, and the zone
/// is kept where the value states one rather than `Z` written over it - which
/// is what a `TZTimestamp` carrying `-05:00` needs, since appending `Z` to it
/// spells a zone twice and reads as nothing at all.
///
/// Neither half may be missing at once. A value stating a date is read
/// whatever zone it states or omits, and one stating only a clock is read
/// only where it states an offset - so a dated spelling that arrives without
/// its date still answers nothing, exactly as it did when only the dated
/// shape was read.
pub(super) fn wire_spelling(dtype: &DataType, text: &str) -> Option<Scalar> {
    match dtype {
        DataType::Boolean => match text.as_bytes() {
            [b'Y' | b'y'] => Some(Scalar::from(true)),
            [b'N' | b'n'] => Some(Scalar::from(false)),
            _ => None,
        },
        DataType::DateTime64 { timezone, .. } => {
            // The zone a value states outranks the column's, and a column
            // stating none takes no zone rather than Z: a `LocalMktDate` and
            // a `LocalMktDatetime` are local market values, and rendering
            // them as instants would make the reading claim a zone the wire
            // never sent.
            let implied = if timezone.is_naive() { "" } else { "Z" };
            // A date states no clock, so it reads as that day at midnight.
            // This is what makes `UTCDateOnly` and `LocalMktDate` instants
            // rather than a second temporal type to cast through.
            if text.len() == 8 && text.bytes().all(|byte| byte.is_ascii_digit()) {
                let rendered = format_smolstr!(
                    "{}-{}-{}T00:00:00{implied}",
                    &text[..4],
                    &text[4..6],
                    &text[6..8],
                );
                return Some(Scalar::from(rendered.as_str()));
            }
            let dated = fix_date(text);
            let (clock, zone) = zoned(dated.as_ref().map_or(text, |(_, rest)| *rest));
            let date = match dated.as_ref() {
                Some((date, _)) => date.as_str(),
                // A value stating no date is a `TZTimeOnly`, and readable
                // only where it states its offset: FIX means *local* time by
                // omitting one, which an instant cannot hold, and a dated
                // spelling that lost its date is not a reading either.
                None if zone.is_some() => EPOCH_DAY,
                None => return None,
            };
            // `HH:MM` is the one width a FIX clock may stop at, and only a
            // `TZTimeOnly` does; anything else is left to fail the read.
            let seconds = if clock.len() == 5 { ":00" } else { "" };
            let zone = zone.unwrap_or(implied);
            let rendered = format_smolstr!("{date}T{clock}{seconds}{zone}");
            Some(Scalar::from(rendered.as_str()))
        }
        DataType::Date32 | DataType::Date64 if text.len() == 8 => {
            let rendered = format_smolstr!("{}-{}-{}", &text[..4], &text[4..6], &text[6..8]);
            Some(Scalar::from(rendered.as_str()))
        }
        _ => None,
    }
}

/// The ISO date a FIX temporal opens with, and what follows it.
///
/// The eight digits are the test rather than the `-`, because a `TZTimeOnly`
/// carrying a western offset has one too and `07:39:12-08` would otherwise
/// read `07:39:12` as a date.
///
/// A value stating no date answers `None`, which is what separates the two
/// kinds of caller: a column declared `TZTimeOnly` takes the epoch day and
/// reads the instant, while one deriving a capture's clock wants a moment the
/// capture happened at and has to refuse a dateless one.
pub(super) fn fix_date(text: &str) -> Option<(SmolStr, &str)> {
    let (day, rest) = text.split_once('-')?;
    if day.len() != 8 || !day.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some((
        format_smolstr!("{}-{}-{}", &day[..4], &day[4..6], &day[6..8]),
        rest,
    ))
}

/// One FIX clock split from the zone it states, or `None` where it states
/// none.
///
/// A clock states no sign and no `Z`, so the first of either opens the zone.
/// A dated value stating none is UTC, which is what `UTCTimestamp` means by
/// saying nothing; a dateless one stating none is not read at all.
fn zoned(text: &str) -> (&str, Option<&str>) {
    if let Some(clock) = text.strip_suffix(['Z', 'z']) {
        return (clock, Some("Z"));
    }
    match text.find(['+', '-']) {
        Some(at) => {
            let (clock, zone) = text.split_at(at);
            (clock, Some(zone))
        }
        None => (text, None),
    }
}

/// One registry field as this message carried it.
///
/// The dictionary's field says what a tag is; only the nullability is this
/// message's own, and it is false because the value is there.
fn stated(known: &Field) -> Field {
    let mut field = known.clone();
    field.set_nullable(false);
    field
}

/// Whether a datatype is one the FIX layer reads raw wire bytes into.
///
/// A `data` field's value is bytes and the row is where they live, so the
/// typed read hands them over untouched instead of reading a spelling.
const fn is_binary(dtype: &DataType) -> bool {
    matches!(dtype, DataType::Binary | DataType::LargeBinary)
}

/// One unknown key's own spelling, folded the way every built name is.
///
/// The entry keeps the arrival casing, because the entries are the wire
/// record and the row is the interpretation; this is the interpretation's
/// side of that split.
fn folded_name(key: &str) -> String {
    let mut name = String::with_capacity(key.len());
    for character in key.chars() {
        if matches!(character, '_' | '-' | ' ') {
            continue;
        }
        name.extend(character.to_lowercase());
    }
    if name.is_empty() {
        key.to_owned()
    } else {
        name
    }
}

/// The message type a built row is named by.
///
/// A message is typed by `35=` in a FIX frame or `MSGTYPE=` in a bridge one.
/// Where a row carries neither it is built anyway and named `unknown`: every
/// pair that parsed becomes a field, the entries record the whole row, and
/// nothing is dropped. `unknown` is safe for the same reason tag `0` is -
/// every `MsgType` the code set declares is one or two characters, so none
/// can collide with it.
pub(super) const UNKNOWN_MSGTYPE: &str = "unknown";

/// The root name one message type gives a row.
pub(super) fn root_name(msgtype: Option<&str>) -> SmolStr {
    msgtype.map_or_else(
        || SmolStr::new_static(UNKNOWN_MSGTYPE),
        |value| format_smolstr!("{value}"),
    )
}
