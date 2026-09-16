//! The fields a message implies but did not carry.
//!
//! A venue sends what its counterparty needs and nothing more, so a row is
//! routinely missing values the message itself already determines: an order
//! stating `OrderQty` and `CumQty` has said what `LeavesQty` is, a fill
//! stating `LastQty` and `LastPx` has said what it was worth, and a message
//! naming its instrument by an ISIN has said which country issued it. Every
//! consumer then derives those independently, which is how two systems come
//! to disagree about one message.
//!
//! The specification tabulates these rather than stating them as arithmetic:
//! FIX 4.4's Appendix D walks an order's whole life and shows what each
//! report carries at every step, FIX 4.2's Appendix O does the same for the
//! fields a foreign exchange trade settles on, and Appendix 6-D lists every
//! `SecurityType` under the ISO 10962 category its CFI code opens with. The
//! code sets say the rest: `SecurityIDSource` names the standard each code
//! stands for, and each standard closes its identifiers with a check digit;
//! the dictionary files every `SecurityType` under the `Product` group it
//! belongs to; `OrdStatus` and `ExecType` spell most of their values alike;
//! `TimeInForce` defines its own absence as a day order; and ISO 6166 opens a
//! number with the two letters ISO 3166 gives its issuing country.
//!
//! # The rules are the registry's, not this module's
//!
//! Each of those tables is one field's `fix:derivation` (decision 38): one
//! term in the crate's expression grammar over the message's fields, spelled
//! by their canonical folded names, carried by the field it fills and read
//! with [`FixField::derivation`](crate::FixField::derivation). Nothing here
//! knows what `LeavesQty` is; the field says `case when ... then orderqty -
//! cumqty end`, and this module evaluates what a field says. Adding a rule is
//! editing a field, exactly as adding a [replacement](super::latest) is, and
//! a dictionary a desk loads carries the desk's rules.
//!
//! # One compile per registry, one working row per message
//!
//! A registry compiles its derivations once - every term parsed, every name
//! it reads proven to be a field or a group, every term bound - and keeps
//! [`Derivations`] until a field changes. What the terms bind against is the
//! working schema: the ordered union of every column any derivation reads or
//! fills, each typed by the registry's field for it, a group by its group
//! definition. A term binds against that schema at compile and never against
//! a message, so there is no shape to recognize and nothing to cache per
//! shape. Per message the pass gathers exactly those columns off the message
//! by tag - a stated field as its value, a stated group laid out as the
//! registry declares its occurrence, an absent column as null - into a
//! working row over the schema, and sweeps the derivations in tag order: a
//! target the row holds non-null is skipped, else the term is evaluated and
//! a non-null answer the target's field accepts is written into the working
//! row where the next derivation reads it. Sweeps repeat until one writes
//! nothing, bounded by the number of derivations, which is what settles a
//! chain in either direction - `securityid` from `isincode` and `isincode`
//! from `securityid`, `product` after `securitytype` after `cficode` -
//! without a hand-laid order. Everything that landed then reaches the message
//! through one [`FixMsg::set_each`], one rebuild for the whole pass. Nothing
//! is kept between messages: a stream of a million messages of a thousand
//! shapes costs each message one working row and the sweeps over it.
//!
//! # Enrichment never touches the entries
//!
//! The row is the interpretation and the entries are what arrived, so a
//! derived value goes to the row alone. Re-emitting an enriched message
//! therefore reproduces the received line byte for byte, which is the whole
//! reason the two facts are held apart. It also means enrichment is
//! idempotent: a stated value is never overwritten, so a value derived once
//! is a stated value the second time and derives to itself.
//!
//! # A derivation answers only when the answer is certain
//!
//! An absent input is a null column of the working row, so a term over it
//! answers null under the grammar's three-valued rules and the target stays
//! unfilled; a condition that does not hold answers null the same way. A
//! value the target's field refuses - an identifier whose check digit does
//! not close, a spelling a code set does not read - is silence, as one
//! refused [`FixMsg::set`] is. The cost of silence is a null column; the
//! cost of a guess is a wrong number nobody can tell from a sent one.
//!
//! A dictionary whose rules do not compile has no rules to fill by, and that
//! is not silence: the compile's refusal names the field, the registry keeps
//! it beside what it would have kept of a compiled list, and every door -
//! the enrichment and the row fill alike - answers it until a field changes.

use std::fmt;

use smol_str::{SmolStr, format_smolstr};

use crate::expression::{Bound, Term};
use crate::{DataType, Error, Field, FixCategory, Result, Scalar};

use super::msg::FixMsg;
use super::registry::FixRegistry;

/// One field's derivation, as the registry compiled it.
struct Derivation {
    /// The tag the derivation fills.
    tag: i32,
    /// The registry's field for that tag, which types every answer exactly
    /// as [`FixMsg::set`] would type it on the way in.
    field: Field,
    /// The term bound against the working schema, or `None` where it does
    /// not bind: a crate column's term over a registry lacking the standard
    /// fields it reads, which is silence for it on every message.
    bound: Option<Bound>,
    /// The working columns the term reads, ascending: what a row fill
    /// gathers for this one derivation alone.
    reads: Vec<usize>,
    /// The target's column in the working schema.
    slot: usize,
}

impl Derivation {
    /// The value this derivation answers for the working row, typed for the
    /// field it fills, or nothing: a term that does not bind, an evaluation
    /// the grammar refuses, an answer of null, and a value the field refuses
    /// are all silence.
    fn answer(&self, row: &[Scalar]) -> Option<Scalar> {
        let value = self.bound.as_ref()?.eval_values(row).ok()?;
        if value.is_null() {
            return None;
        }
        self.field.scalar(value).ok()
    }
}

/// Where a message states one column of the working schema.
enum Source {
    /// A scalar field, read by its tag.
    Field(i32),
    /// A repeating group, read by its counter's tag and laid out as the
    /// registry declares its occurrence, so the members a term reads by
    /// name stand where the group definition puts them whatever order the
    /// message stated them in.
    Group(i32),
    /// A name no field or group of the registry answers to, which no message
    /// states: a crate column's term reads the standard's fields by name,
    /// and a registry built from a handful of fields holds none of them.
    Absent,
}

/// One column of the working schema, and how a message states it.
struct Input {
    /// The field typing the column: the registry's own, or a null field for
    /// a name the registry lacks.
    field: Field,
    source: Source,
}

impl Input {
    /// The registry's column one name reaches, as the working schema holds
    /// it: a group before a scalar field, because a group's column is named
    /// by the group and its counter is a field of its own.
    fn of(registry: &FixRegistry, name: &str) -> std::result::Result<Option<Self>, SmolStr> {
        if let Some(group) = registry.get_definition(FixCategory::Groups, name) {
            let Some(counter) = group.as_fix().counter().ok().flatten() else {
                return Err(format_smolstr!(
                    "fix:derivation reads `{name}`, a group declaring no counter"
                ));
            };
            return Ok(Some(Self {
                field: group.clone(),
                source: Source::Group(counter),
            }));
        }
        let Some(field) = registry.get_field_by_name(name) else {
            return Ok(None);
        };
        // A registry field carries its tag; one that does not is a column
        // no row is indexed by, which is a column no message states.
        let source = registry
            .identity_of(field)
            .map_or(Source::Absent, |(tag, _)| Source::Field(tag));
        Ok(Some(Self {
            field: field.clone(),
            source,
        }))
    }

    /// A column the registry lacks, typed as nothing and always null.
    fn absent(name: &str) -> Self {
        Self {
            field: DataType::Null.nullable_field(name),
            source: Source::Absent,
        }
    }

    /// What `msg` states for this column, or null.
    fn read(&self, msg: &FixMsg) -> Scalar {
        match self.source {
            Source::Field(tag) => msg.indexed_by_tag(tag).cloned().unwrap_or(Scalar::Null),
            Source::Group(counter) => msg
                .index_of_group(counter)
                .and_then(|at| msg.as_value().get(at))
                .map_or(Scalar::Null, |held| {
                    msg.regrouped(counter, &self.field, held.clone())
                }),
            Source::Absent => Scalar::Null,
        }
    }
}

/// Every `fix:derivation` a registry carries, compiled once (decision 38).
///
/// Built by [`FixRegistry::derivations`] and kept on the registry until a
/// field changes; every codec and every message reading that registry
/// evaluates through the same instance, the line door, the batch door and
/// the row fill alike.
pub(super) struct Derivations {
    /// In tag order, which is the order one sweep evaluates them in.
    list: Vec<Derivation>,
    /// The working schema's columns in first-seen order - every column a
    /// term reads or a derivation fills - and how a message states each.
    /// The schema every term is bound against is the Struct of their
    /// fields, held by each bound term.
    inputs: Vec<Input>,
    /// The working columns the crate columns' terms read, ascending: what
    /// a row fill gathers, once per row, for the three of them.
    crate_reads: Vec<usize>,
}

impl Derivations {
    /// Reads every field's `fix:derivation` and proves it against the
    /// registry.
    ///
    /// Every column a term reads must be a field or a group the registry
    /// names, and the term must bind against the working schema those
    /// fields make - so a name the dictionary lacks, or `orderqty - symbol`,
    /// is refused here naming the field, not once per message. The crate's
    /// own columns are the one exception: their derivations read the
    /// standard's fields by name, and a registry built from a handful of
    /// fields holds none of them, so for a crate field a name the registry
    /// lacks is an input the dictionary never states - a null column of the
    /// working schema - and a term that cannot bind over such columns is
    /// silent rather than a refusal of the registry that lacks them.
    ///
    /// # Errors
    ///
    /// Returns the refusal, naming the registry field whose derivation does
    /// not parse, reads a column no field or group of this registry answers
    /// to, or does not bind against the fields it reads.
    pub(super) fn compile(registry: &FixRegistry) -> std::result::Result<Self, Refused> {
        let mut carried: Vec<(i32, Field, Term)> = Vec::new();
        let mut inputs: Vec<Input> = Vec::new();
        for field in registry.iter() {
            let Some(term) = field
                .as_fix()
                .derivation()
                .map_err(|error| Refused::new(field.name(), &error))?
            else {
                continue;
            };
            let Some((tag, _)) = registry.identity_of(field) else {
                continue;
            };
            let crated = super::is_crate_tag(tag);
            for name in term.columns() {
                if position_of(&inputs, &name).is_some() {
                    continue;
                }
                let input = match Input::of(registry, &name)
                    .map_err(|reason| Refused::new(field.name(), &reason))?
                {
                    Some(known) => known,
                    None if crated => Input::absent(&name),
                    None => {
                        return Err(Refused::new(
                            field.name(),
                            &format_smolstr!(
                                "fix:derivation reads `{name}`, which names no field or group \
                                 of the registry"
                            ),
                        ));
                    }
                };
                inputs.push(input);
            }
            match position_of(&inputs, field.name()) {
                // A term read the target already, as a chain's two ends do.
                Some(at) if matches!(inputs[at].source, Source::Field(_)) => {}
                Some(_) => {
                    return Err(Refused::new(
                        field.name(),
                        &"fix:derivation fills a name the registry also gives a group",
                    ));
                }
                None => inputs.push(Input {
                    field: field.clone(),
                    source: Source::Field(tag),
                }),
            }
            carried.push((tag, field.clone(), term));
        }
        let schema = DataType::from_fields(inputs.iter().map(|input| input.field.clone()))
            .map_err(|error| Refused::new("fix:derivation", &error))?
            .required_field("derived");
        let mut list: Vec<Derivation> = Vec::with_capacity(carried.len());
        for (tag, field, term) in carried {
            // Bound once, here: what refuses would refuse for every message,
            // and is the field's to fix.
            let bound = match term.bind(&schema) {
                Ok(bound) => Some(bound),
                Err(_) if super::is_crate_tag(tag) => None,
                Err(error) => return Err(Refused::new(field.name(), &error)),
            };
            let reads = bound
                .as_ref()
                .map(Bound::column_indices)
                .unwrap_or_default();
            let Some(slot) = position_of(&inputs, field.name()) else {
                return Err(Refused::new(
                    field.name(),
                    &"the working schema lacks the derived column",
                ));
            };
            list.push(Derivation {
                tag,
                field,
                bound,
                reads,
                slot,
            });
        }
        // The registry iterates tag-major already; stated here so the sweep
        // order is this list's contract rather than the iteration's.
        list.sort_by_key(|derivation| derivation.tag);
        let mut crate_reads: Vec<usize> = list
            .iter()
            .filter(|derivation| super::is_crate_tag(derivation.tag))
            .flat_map(|derivation| derivation.reads.iter().copied())
            .collect();
        crate_reads.sort_unstable();
        crate_reads.dedup();
        Ok(Self {
            list,
            inputs,
            crate_reads,
        })
    }

    /// The working schema: the Struct every term is bound against, one
    /// column per field or group any derivation reads or fills, as the
    /// first bound term holds it; `None` where no term bound.
    #[cfg(test)]
    pub(super) fn schema(&self) -> Option<&Field> {
        self.list
            .iter()
            .find_map(|derivation| derivation.bound.as_ref())
            .map(Bound::schema)
    }

    /// Each derived tag in sweep order, beside whether its term bound.
    #[cfg(test)]
    pub(super) fn derived(&self) -> impl Iterator<Item = (i32, bool)> + '_ {
        self.list
            .iter()
            .map(|derivation| (derivation.tag, derivation.bound.is_some()))
    }

    /// Fills `msg` with everything its derivations imply, to a fixpoint.
    ///
    /// Every answer lands through one [`FixMsg::set_each`]: a row already
    /// holding the tag holds a stated null, and the answer takes that child's
    /// place; anything else is appended. The dictionary's own field typed the
    /// value before it was written into the working row, so a derived column
    /// is indistinguishable from a stated one and a value the field refuses
    /// was silence before any write was planned.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the written children do not
    /// make a root.
    pub(super) fn fill_all(&self, msg: &mut FixMsg) -> Result<()> {
        if self.list.is_empty() {
            return Ok(());
        }
        let mut row = self.working_row(msg, 0..self.inputs.len());
        let mut landed: Vec<(i32, Scalar)> = Vec::new();
        // A productive sweep fills at least one target and a filled target
        // is never revisited, so the derivation count bounds the sweeps; the
        // one past it is the sweep that writes nothing.
        for _ in 0..=self.list.len() {
            let mut wrote = false;
            for derivation in &self.list {
                // A stated value is never overwritten, which is what makes
                // this idempotent: the second pass finds the first pass's
                // answer stated.
                if !row[derivation.slot].is_null() {
                    continue;
                }
                let Some(value) = derivation.answer(&row) else {
                    continue;
                };
                row[derivation.slot] = value.clone();
                // Sized once, on the first answer, for every derivation
                // there is: a message deriving nothing allocates nothing
                // here, and one deriving nine grows the list once.
                if landed.capacity() == 0 {
                    landed.reserve_exact(self.list.len());
                }
                landed.push((derivation.tag, value));
                wrote = true;
            }
            if !wrote {
                break;
            }
        }
        if !landed.is_empty() {
            msg.set_each(landed)?;
        }
        Ok(())
    }

    /// The working row a row fill reads: the columns the crate columns'
    /// terms read, gathered off `msg` as the pass gathers them, once for
    /// every crate column the row asks for.
    pub(super) fn crate_row(&self, msg: &FixMsg) -> Vec<Scalar> {
        self.working_row(msg, self.crate_reads.iter().copied())
    }

    /// One derivation's answer over a working row, or nothing.
    ///
    /// What a row fill asks for a crate column the message does not state:
    /// the one evaluation of that column's own term over [`Self::crate_row`],
    /// typed by the column's field as the pass types it. `None` for a tag no
    /// field derives, a term that does not bind, an answer of null and a
    /// value the field refuses alike.
    pub(super) fn fill(&self, tag: i32, row: &[Scalar]) -> Option<Scalar> {
        self.list
            .iter()
            .find(|derivation| derivation.tag == tag)?
            .answer(row)
    }

    /// The working row of `msg`: every column of the working schema, the
    /// `wanted` ones read off the message and the rest null.
    fn working_row(&self, msg: &FixMsg, wanted: impl IntoIterator<Item = usize>) -> Vec<Scalar> {
        let mut row = vec![Scalar::Null; self.inputs.len()];
        for at in wanted {
            row[at] = self.inputs[at].read(msg);
        }
        row
    }
}

/// Where a column stands among the inputs, under the fold the binder
/// resolves a name by.
fn position_of(inputs: &[Input], name: &str) -> Option<usize> {
    inputs
        .iter()
        .position(|input| input.field.name().eq_ignore_ascii_case(name))
}

/// A derivation refused at compile, naming the field that carries it.
///
/// What the registry keeps where it would have kept the compiled list: a
/// registry whose rules do not compile refuses every ask - the enrichment
/// and the row fill alike - with this, and compiles once until a field
/// changes rather than once per ask.
#[derive(Clone, Debug)]
pub(super) struct Refused {
    path: SmolStr,
    reason: SmolStr,
}

impl Refused {
    fn new(field: &str, reason: &dyn fmt::Display) -> Self {
        Self {
            path: SmolStr::new(field),
            reason: format_smolstr!("{reason}"),
        }
    }

    /// The refusal as the error a door answers with.
    pub(super) fn error(&self) -> Error {
        Error::InvalidRecord {
            path: self.path.clone(),
            reason: self.reason.clone(),
        }
    }
}

/// What a stream of messages remembers as it passes them.
///
/// A bridge states a plugin's comp ids once, in the configuration it printed
/// at startup, and then writes lines that name only the plugin. Those lines
/// are about a session whose two ends the reader has already read, so the
/// pass that fills what a message left unsaid can fill them too - from the
/// configuration, by name, and never over a value the message stated
/// (decision 19).
///
/// The memory is one entry per plugin, replaced when a later configuration
/// names it again, and it dies with the iterator that holds it.
#[derive(Debug, Default)]
pub(super) struct Remembered {
    /// The plugin's folded `Name`, beside the two ends its configuration
    /// named: `SenderCompID` and `TargetCompID`.
    plugins: std::collections::HashMap<String, [Option<Scalar>; 2]>,
}

/// The fields a configuration answers for on a later message.
///
/// Not `BeginString`: every built message fills tag 8 non-null from the
/// version its row was read at, so a configuration's would never find a
/// message stating none (decision 19).
const REMEMBERED_TAGS: [i32; 2] = [49, 56];

impl Remembered {
    /// Reads one message, and fills it from what an earlier one said.
    ///
    /// A `pluginconfig` is remembered under its `Name` and passed on
    /// untouched: it is the statement, not a thing to fill. Anything else
    /// naming a plugin some configuration named takes that configuration's
    /// comp ids where it stated none of its own.
    pub(super) fn fill(&mut self, msg: FixMsg) -> FixMsg {
        if msg.as_field().name() == super::PLUGINCONFIG_CODE_NAME.1 {
            self.remember(&msg);
            return msg;
        }
        let Some(named) = msg
            .get_by_tag(super::PLUGINID_TAG_NAME.0)
            .and_then(Scalar::as_str)
        else {
            return msg;
        };
        let Some(held) = self.plugins.get(&crate::types::normalized(named)) else {
            return msg;
        };
        let mut msg = msg;
        for (tag, value) in REMEMBERED_TAGS.iter().zip(held) {
            let Some(value) = value else {
                continue;
            };
            // A stated value is never overwritten, which is the rule this
            // whole pass keeps.
            if msg
                .get_by_tag(*tag)
                .is_some_and(|held| held != &Scalar::Null)
            {
                continue;
            }
            let _ = msg.set(*tag, value.clone());
        }
        msg
    }

    /// Holds what one configuration said, replacing what an earlier one did.
    fn remember(&mut self, msg: &FixMsg) {
        let Some(name) = msg.get_by_name("Name").and_then(Scalar::as_str) else {
            return;
        };
        let stated = |tag: i32| {
            msg.get_by_tag(tag)
                .filter(|value| *value != &Scalar::Null)
                .cloned()
        };
        self.plugins
            .insert(crate::types::normalized(name), [stated(49), stated(56)]);
    }
}

/// The fields the arrival record names that the message no longer holds.
///
/// A row is a projection. [`fix_schema`](super::fix_schema) names a column
/// for the tags a book, a blotter, a quote feed and a monitor read, and a
/// field outside that list reaches a message rebuilt from a row only through
/// the arrival record - which the row carries whole, under
/// [`FIXENTRIES_COLUMN`](super::schema::FIXENTRIES_COLUMN), whatever the columns
/// made of it. `ExecBroker(76)` and `ClientID(109)` are two such fields, and
/// the replacements that restate them write the `parties` group and its
/// `NoPartyIDs(453)` counter, which do have columns. A pass reading only the
/// columns would therefore answer two parties on the line door and none on
/// the batch door for one message, which is the one thing the two doors may
/// never do.
///
/// So the pass opens on the record rather than on the columns, and it costs
/// nothing where nothing was dropped: a message parsed from a line already
/// holds a child for every tag its record names, so the walk writes nothing
/// and allocates nothing. Only a tag the message holds no child for at all is
/// taken - a stated null is a child, and a message that said "nothing sent"
/// said it.
/// What a row's projection dropped, lifted back out of the arrival record.
///
/// The one door a [format](super::FixCodec::format_messages) opens before it
/// fills a message field. A row is a projection: a column the row it came
/// from did not carry is in the arrival record and nowhere else, so
/// formatting a narrower row into a wider field reads the record for what
/// the narrower one lost.
///
/// It fills and never overwrites, so it is idempotent and free where nothing
/// was dropped: a message parsed from a line already holds a child for every
/// tag its record names, and the walk writes nothing.
///
/// What it cannot answer is what the record does not say. A bridge's packed
/// occurrence is one pair the codec unpacked into members, a composed key is
/// one pair the codec resolved onto another field, and a row-header capture
/// never arrived on the wire at all - the record keeps each as the bridge
/// wrote it, under tag zero where nothing resolved it, because an arrival is
/// what arrived (decisions 8 and 20). Those readings are the codec's, so a
/// row that drops their columns has dropped them.
pub(super) fn lifted(registry: &FixRegistry, msg: FixMsg) -> FixMsg {
    recovered(registry, msg)
}

fn recovered(registry: &FixRegistry, mut msg: FixMsg) -> FixMsg {
    let mut dropped: Vec<(i32, Scalar)> = Vec::new();
    let mut groups: Vec<(SmolStr, Scalar)> = Vec::new();
    for entry in msg.entries() {
        let tag = entry.tag();
        // `0` is an unresolved key - a name or number with no registry
        // identity - and a tag the message holds needs
        // nothing: the record is read for what the projection lost, never to
        // restate what survived it.
        if tag <= 0 || msg.get_by_tag(tag).is_some() {
            continue;
        }
        if dropped.iter().any(|(held, _)| *held == tag) {
            continue;
        }
        // A pair that headed a subtree is a group's counter, and the
        // occurrences it heads come back as the group the dictionary
        // declares - never as a count with nothing under it. The group is
        // written under its own name, because a counter's tag names the
        // count and the group is the thing beside it.
        if !entry.children().is_empty() {
            if let Some(group) = registry.get_group_by_tag(tag) {
                if let Some(value) = occurrences_of(group, entry.children()) {
                    let count = value.as_sequence().map_or(0, <[Scalar]>::len);
                    groups.push((SmolStr::new(group.name()), value));
                    dropped.push((tag, Scalar::from(i32::try_from(count).unwrap_or(i32::MAX))));
                }
            }
            continue;
        }
        let Some(text) = entry.value().as_str() else {
            continue;
        };
        // The dictionary's own field reads the spelling, which is what the
        // builder read on the way in: a FIX timestamp, a code's name and a
        // decimal are spellings the generic value contract does not know, and
        // a value lifted back out of the record has to come back as what went
        // in. A spelling the field refuses is the null the row would hold
        // anyway. `Scalar::from(&str)` holds the text as the compact string
        // it is, for a tag no dictionary explains.
        let value = registry.get_field_by_tag(tag).map_or_else(
            || Scalar::from(text),
            |field| super::build::typed_spelling(field, text),
        );
        if !value.is_null() {
            dropped.push((tag, value));
        }
    }
    for (tag, value) in dropped {
        // Unresolved keys recorded tag 0 and were skipped above, so only a
        // positive tag reaches this write; one the dictionary lacks is kept
        // under its decimal spelling, as `set` keeps any such tag.
        let _ = msg.set(tag, value);
    }
    for (name, value) in groups {
        let _ = msg.set(name.as_str(), value);
    }
    msg
}

/// One group's occurrences, rebuilt from the entries that arrived under its
/// counter.
///
/// An occurrence opens on the group's declared delimiter - its first member,
/// which is what FIX says opens one - and on any member the occurrence being
/// filled has already stated, which is what a venue writing its members in
/// its own order still says. A member the dictionary does not declare for
/// this group is not this group's, so it is left where the record has it; a
/// nested counter's own subtree comes back through the same walk.
///
/// `None` where nothing was rebuilt, so a group that says nothing writes
/// nothing rather than an empty list the message never stated.
fn occurrences_of(group: &Field, entries: &[super::FixEntry]) -> Option<Scalar> {
    let members = super::schema::item_fields(group)?;
    let tags: Vec<Option<i32>> = members
        .iter()
        .map(|member| member.as_fix().tag().ok().flatten())
        .collect();
    let mut rows: Vec<Vec<Scalar>> = Vec::new();
    let mut stated: Vec<bool> = Vec::new();
    for entry in entries {
        let Some(at) = tags.iter().position(|held| *held == Some(entry.tag())) else {
            continue;
        };
        // The first declared member opens an occurrence, and so does a
        // member the one being filled already stated.
        if rows.is_empty() || at == 0 || stated[at] {
            rows.push(vec![Scalar::Null; members.len()]);
            stated = vec![false; members.len()];
        }
        let member = &members[at];
        let value = if entry.children().is_empty() {
            entry
                .value()
                .as_str()
                .map(|text| super::build::typed_spelling(member, text))
                .filter(|held| !held.is_null())
        } else {
            occurrences_of(member, entry.children()).and_then(|held| member.scalar(held).ok())
        };
        if let Some(value) = value {
            let last = rows.len() - 1;
            rows[last][at] = value;
            stated[at] = true;
        }
    }
    if rows.is_empty() {
        return None;
    }
    Some(Scalar::from_sequence(
        rows.into_iter().map(Scalar::from_sequence),
    ))
}

/// Fills what `msg` implies, leaving what it stated and what arrived alone.
///
/// Four steps in order (decision 20, decision 38): what the row's projection
/// dropped comes back off the arrival record; the message is restated under
/// the dictionary the registry holds; the registry's derivations fill what the
/// message implies, to a fixpoint; and the component's identifier
/// declaration fills the sorted `altids` Map. Every answer lands in the row
/// alone, typed by the dictionary's own field for the tag, so a derived
/// column is indistinguishable from a stated one and carries the same
/// display, description and `fix:tag` a reader resolves it by - and a value
/// it refuses, such as an identifier whose check digit does not close, is
/// silence. A declared identifier that cannot spell text is silence too: it
/// is left out of the Map rather than allowed to refuse the message.
pub(super) fn enrich(registry: &FixRegistry, msg: FixMsg) -> crate::Result<FixMsg> {
    // What the row's projection dropped comes back off the arrival record
    // first, because restatement reads what the document stated and a row
    // states only its columns (decision 20).
    let msg = recovered(registry, msg);
    // Restatement next, and not as a step a caller may skip: every
    // derivation reads by canonical name, and a child stored under an alias
    // is invisible until it has been canonicalized (decision 20).
    let mut held = super::latest::restate(msg)?;
    // The derivations, compiled and bound once per registry; a refused
    // compile is the pass's to report, since a dictionary whose rules do not
    // compile has no rules to fill by.
    registry.derivations()?.fill_all(&mut held)?;
    if held
        .get_by_tag(super::ALTIDS_TAG_NAME.0)
        .is_none_or(Scalar::is_null)
    {
        // Held compactly rather than as a `String`: a message type is one to
        // three characters, which stays inside the value.
        let msgtype = SmolStr::new(
            held.get_by_tag(35)
                .and_then(Scalar::as_str)
                .unwrap_or_default(),
        );
        if let Some(component) = registry.get_msgtype(&msgtype) {
            held.set(
                super::ALTIDS_TAG_NAME.0,
                component.identifier_mapping(&held)?,
            )?;
        }
    }
    Ok(held)
}
