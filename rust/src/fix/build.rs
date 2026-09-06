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
use super::{FixBranch, FixRegistry, STANDARD_HEADER_TAGS, STANDARD_TRAILER_TAGS};
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
    values: Vec<Scalar>,
    /// The group members, when this slot is a repeating group.
    occurrences: Vec<Vec<(SmolStr, Scalar)>>,
}

/// Builds a message's field and value from pairs, with the entries beside it.
pub(super) struct Builder<'registry> {
    registry: &'registry FixRegistry,
    branch: FixBranch,
    version: Option<Version>,
    slots: Vec<Slot>,
    entries: Vec<FixEntry>,
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
            entries: Vec::with_capacity(capacity),
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
    fn resolve(&self, key: &str) -> Option<(&'registry Field, i32)> {
        let field = if let Some(tag) = super::field::parse_tag(key) {
            self.registry.get_primitive_field(tag)
        } else {
            self.registry
                .get_field_by_path(key, Some(&self.branch))
                .filter(|field| !super::registry::is_nested(field))
        }?;
        let tag = field.as_fix().tag().ok().flatten().unwrap_or(0);
        Some((field, tag))
    }

    /// The field a key builds under, cloned and projected to the version.
    ///
    /// A field the dictionary knows is the registry's, renamed and retyped to
    /// what the message's own version called it; one it does not is kept
    /// under the key's own folded spelling as nullable text, because a venue
    /// sends fields no dictionary has.
    fn field_for(&self, key: &str) -> (Field, i32) {
        match self.resolve(key) {
            Some((known, tag)) => (self.project(known), tag),
            None => {
                let name = folded_name(key);
                let tag = super::field::parse_tag(key).unwrap_or(0);
                (DataType::Utf8.nullable_field(name), tag)
            }
        }
    }

    /// One registry field as the message's own version spells and types it.
    fn project(&self, known: &Field) -> Field {
        let mut field = known.clone();
        if let Some(at) = self.version {
            let view = known.as_fix();
            if let Some(name) = view.name_at(at) {
                if name != known.name() {
                    field.set_name(name);
                }
            }
            if let Ok(Some(dtype)) = view.dtype_at(at) {
                if dtype != *known.dtype() {
                    field = Field::new(field.name(), dtype, field.is_nullable());
                    let _ = field.set_metadata(known.as_metadata().iter());
                }
            }
        }
        // The value is there, so this message's schema says so.
        field.set_nullable(false);
        field
    }

    /// Types one value under one field, translating its code first.
    ///
    /// A value that will not type is null rather than a failure: the raw text
    /// stays in the entries, the refusal is readable, and the message still
    /// answers `Ok`. A parse error is for input that is not a message at all.
    fn typed(&self, field: &Field, raw: &[u8], text: &str) -> Scalar {
        let view = field.as_fix();
        let translated = match self.version {
            Some(at) => view.code_value_at(at, text),
            None => view.code_value(text),
        };
        let spelling = translated.unwrap_or(text);
        // A `data` field's value is bytes, and the row is where they live:
        // the entry holds a lossy decode of them and this does not.
        if matches!(field.dtype(), DataType::Binary | DataType::LargeBinary) {
            return field
                .scalar(Scalar::from(raw.to_vec()))
                .unwrap_or(Scalar::Null);
        }
        // Every wire value is text, and the generic value contract does not
        // read text as a number, an instant or a flag. Two of those it can
        // learn from the field alone, which is the crate's own coercion; the
        // third is a FIX *spelling* and stays here, because the generic
        // contract must not learn one.
        let value = match wire_spelling(field.dtype(), spelling) {
            Some(value) => value,
            None => crate::text::prepare_text(Scalar::from(spelling), field)
                .unwrap_or_else(|_| Scalar::from(spelling)),
        };
        field.scalar(value).unwrap_or(Scalar::Null)
    }

    /// One flat child, appended in arrival order.
    fn push_flat(&mut self, key: &str, text: &str, raw: &[u8]) {
        let (field, tag) = self.field_for(key);
        let value = self.typed(&field, raw, text);
        self.record(key, text, tag);
        self.slot_for(field, tag).values.push(value);
    }

    /// One occurrence of a repeated flat field, placed by index.
    fn push_repeated(&mut self, name: &str, occurrence: usize, key: &str, text: &str, raw: &[u8]) {
        let (field, tag) = self.field_for(name);
        let value = self.typed(&field, raw, text);
        self.record(key, text, tag);
        let slot = self.slot_for(field, tag);
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
        let (member_field, member_tag) = self.field_for(member);
        let value = self.typed(&member_field, raw, text);
        self.record(key, text, member_tag);

        let counter = self
            .registry
            .get_nested_field(group)
            .or_else(|| self.registry.get_field_by_name(group, Some(&self.branch)));
        let (group_field, group_tag) = match counter {
            Some(known) => {
                let tag = known.as_fix().tag().ok().flatten().unwrap_or(0);
                (known.clone(), tag)
            }
            None => (
                DataType::Utf8.nullable_field(folded_name(group)),
                super::field::parse_tag(group).unwrap_or(0),
            ),
        };
        let slot = self.slot_for(group_field, group_tag);
        while slot.occurrences.len() <= occurrence {
            slot.occurrences.push(Vec::new());
        }
        slot.occurrences[occurrence].push((SmolStr::new(member_field.name()), value));
    }

    /// The slot one field builds into, created on first use.
    fn slot_for(&mut self, field: Field, tag: i32) -> &mut Slot {
        let held = self
            .slots
            .iter()
            .position(|slot| slot.field.name() == field.name());
        match held {
            Some(index) => &mut self.slots[index],
            None => {
                self.slots.push(Slot {
                    field,
                    tag,
                    values: Vec::new(),
                    occurrences: Vec::new(),
                });
                self.slots.last_mut().expect("just pushed")
            }
        }
    }

    /// Records what arrived, whatever the row made of it.
    fn record(&mut self, key: &str, value: &str, tag: i32) {
        let entry = FixEntry::new(tag, key, value);
        let entry = if tag == 0 {
            entry
        } else {
            entry.with_branch(&self.branch)
        };
        self.entries.push(entry);
    }

    /// Closes the build into a root field, its value, and the entries.
    ///
    /// Order is the standard header, then the body in arrival order, then the
    /// standard trailer - flat, with no `StandardHeader` Struct, because a
    /// message is laid flat and the header is an ordering rather than a
    /// nesting.
    pub(super) fn finish(self, name: &str) -> Result<(Field, Scalar, Vec<FixEntry>)> {
        let Self { slots, entries, .. } = self;
        let mut ordered: Vec<Slot> = Vec::with_capacity(slots.len());
        let mut rest: Vec<Slot> = Vec::with_capacity(slots.len());
        let mut trailing: Vec<Slot> = Vec::new();
        for slot in slots {
            if STANDARD_HEADER_TAGS.contains(&slot.tag) {
                ordered.push(slot);
            } else if STANDARD_TRAILER_TAGS.contains(&slot.tag) {
                trailing.push(slot);
            } else {
                rest.push(slot);
            }
        }
        ordered.sort_by_key(|slot| {
            STANDARD_HEADER_TAGS
                .iter()
                .position(|tag| *tag == slot.tag)
                .unwrap_or(usize::MAX)
        });
        trailing.sort_by_key(|slot| {
            STANDARD_TRAILER_TAGS
                .iter()
                .position(|tag| *tag == slot.tag)
                .unwrap_or(usize::MAX)
        });
        ordered.extend(rest);
        ordered.extend(trailing);

        let mut fields = Vec::with_capacity(ordered.len());
        let mut values = Vec::with_capacity(ordered.len());
        for slot in ordered {
            let (field, value) = slot.into_child()?;
            fields.push(field);
            values.push(value);
        }
        let root = DataType::from_fields(fields)?.required_field(name);
        Ok((root, Scalar::from_sequence(values), entries))
    }
}

impl Slot {
    /// This slot as one child field and its value.
    fn into_child(self) -> Result<(Field, Scalar)> {
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
            let mut item = self.field.clone().with_name("item");
            item.set_nullable(absent);
            let values = Scalar::from_sequence(self.values);
            let mut list = DataType::list(item).required_field(self.field.name());
            let _ = list.set_metadata(self.field.as_metadata().iter());
            return Ok((list, values));
        }

        // A group is a List of a non-null `item` Struct, and the occurrences
        // are built by index, so a gap is an empty one rather than a shift.
        let mut member_fields: Vec<Field> = Vec::new();
        for occurrence in &self.occurrences {
            for (name, value) in occurrence {
                if member_fields.iter().any(|field| field.name() == name) {
                    continue;
                }
                let dtype = value.dtype().unwrap_or(DataType::Utf8);
                member_fields.push(dtype.nullable_field(name.as_str()));
            }
        }
        let mut item = DataType::from_fields(member_fields.clone())?.required_field("item");
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
                    .find(|(name, _)| name == field.name())
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

/// The value one FIX wire spelling states, where FIX spells it its own way.
///
/// A boolean is `Y` or `N`, and a temporal is a run of digits with no
/// separators a general parser would recognize: `20260821-10:30:00.123456`,
/// `20260821`, `10:30:00.000000`. These are facts about FIX rather than about
/// the datatype, so they are read here and the generic value contract learns
/// none of them.
fn wire_spelling(dtype: &DataType, text: &str) -> Option<Scalar> {
    match dtype {
        DataType::Boolean => match text.as_bytes() {
            [b'Y' | b'y'] => Some(Scalar::from(true)),
            [b'N' | b'n'] => Some(Scalar::from(false)),
            _ => None,
        },
        DataType::DateTime64 { .. } => {
            let (day, time) = text.split_once('-')?;
            let rendered = format_smolstr!(
                "{}-{}-{}T{time}Z",
                &day.get(..4)?,
                &day.get(4..6)?,
                &day.get(6..8)?
            );
            Some(Scalar::from(rendered.as_str()))
        }
        DataType::Date32 | DataType::Date64 if text.len() == 8 => {
            let rendered = format_smolstr!("{}-{}-{}", &text[..4], &text[4..6], &text[6..8]);
            Some(Scalar::from(rendered.as_str()))
        }
        _ => None,
    }
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
