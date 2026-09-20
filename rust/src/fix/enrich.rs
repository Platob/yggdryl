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
//! Each of those tables is one field's `FIX:derivation`: one
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

use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::iter::FusedIterator;
use std::vec;

use smol_str::{SmolStr, format_smolstr};

use crate::expression::{Bound, Term};
use crate::graph::instrument::InstrumentCodes;
use crate::graph::iterator::order;
use crate::graph::{Element, EventIterator};
use crate::{DataType, Error, Field, FixCategory, Result, Scalar, StructType};

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
    /// What the term answers over a row stating nothing it reads, probed
    /// once at compile: a term is a function of the columns it reads, so a
    /// message stating none of them answers exactly this - which for most
    /// derivations over most messages is nothing - without an evaluation.
    unread_answer: Option<Scalar>,
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
        // A decimal target takes any number the grammar answers - a float
        // column, an integer, text - as the exact decimal it restates, which
        // is what every crate price and quantity is.
        let value = if self.field.dtype() == &DataType::DECIMAL {
            Scalar::from(crate::Decimal18::from_scalar(&value)?)
        } else {
            value
        };
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
                    "FIX:derivation reads `{name}`, a group declaring no counter"
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
            Source::Field(tag) => msg.indexed_by_tag(tag).unwrap_or(Scalar::Null),
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

/// Every `FIX:derivation` a registry carries, compiled once.
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
    /// Reads every field's `FIX:derivation` and proves it against the
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
                                "FIX:derivation reads `{name}`, which names no field or group \
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
                        &"FIX:derivation fills a name the registry also gives a group",
                    ));
                }
                None => inputs.push(Input {
                    field: field.clone(),
                    source: Source::Field(tag),
                }),
            }
            carried.push((tag, field.clone(), term));
        }
        let schema = StructType::from_fields(inputs.iter().map(|input| input.field.clone()))
            .map(DataType::from)
            .map_err(|error| Refused::new("FIX:derivation", &error))?
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
            let mut derivation = Derivation {
                tag,
                field,
                bound,
                reads,
                slot,
                unread_answer: None,
            };
            // Through the one door every answer takes, so the decimal
            // restatement and the field's refusal are part of the probe.
            let unread = vec![Scalar::Null; inputs.len()];
            derivation.unread_answer = derivation.answer(&unread);
            list.push(derivation);
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
    /// The identity is not settled: the pass settles once, after
    /// everything it writes.
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
        // Which working columns the last sweep wrote. A term is a function
        // of the columns it reads, so a derivation that read no column the
        // last sweep wrote answers what it answered then: a later sweep
        // evaluates only the derivations reading a column that moved, and
        // the first evaluates every one.
        // Two buffers for every sweep of the message rather than one per
        // sweep: the last sweep's writes are read while this sweep's are
        // marked, and the two swap places at its end.
        let mut moved: Vec<bool> = vec![false; self.inputs.len()];
        let mut wrote: Vec<bool> = vec![false; self.inputs.len()];
        // A productive sweep fills at least one target and a filled target
        // is never revisited, so the derivation count bounds the sweeps; the
        // one past it is the sweep that writes nothing.
        for sweep in 0..=self.list.len() {
            wrote.fill(false);
            let mut any = false;
            for derivation in &self.list {
                // A stated value is never overwritten, which is what makes
                // this idempotent: the second pass finds the first pass's
                // answer stated.
                if !row[derivation.slot].is_null() {
                    continue;
                }
                if sweep > 0 && !derivation.reads.iter().any(|at| moved[*at]) {
                    continue;
                }
                // A term is a function of the columns it reads: over a row
                // stating none of them it answers what the probe answered,
                // and the evaluation is skipped.
                let answer = if derivation.reads.iter().all(|at| row[*at].is_null()) {
                    derivation.unread_answer.clone()
                } else {
                    derivation.answer(&row)
                };
                let Some(value) = answer else {
                    continue;
                };
                row[derivation.slot] = value.clone();
                wrote[derivation.slot] = true;
                // Sized once, on the first answer, for every derivation
                // there is: a message deriving nothing allocates nothing
                // here, and one deriving nine grows the list once.
                if landed.capacity() == 0 {
                    landed.reserve_exact(self.list.len());
                }
                landed.push((derivation.tag, value));
                any = true;
            }
            if !any {
                break;
            }
            std::mem::swap(&mut moved, &mut wrote);
        }
        if landed.is_empty() {
            return Ok(());
        }
        msg.set_each(landed)?;
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

/// Fills what `msg` implies, leaving what it stated alone.
///
/// Four steps in order, and the last step of every parse. The message is
/// restated under the dictionary the registry holds; the registry's
/// derivations fill what the message implies, to a fixpoint; the
/// component's identifier declaration fills the names the message goes by;
/// and an order's lanes fill from its price and side. Every answer lands
/// where the fact lives - a typed fact on its holder, anything else in the
/// row, typed by the dictionary's own field for the tag - so a derived
/// value is indistinguishable from a stated one, and a value the field
/// refuses, such as an identifier whose check digit does not close, is
/// silence. A declared identifier that cannot spell text is silence too:
/// it is left out rather than allowed to refuse the message.
pub(super) fn enrich(registry: &FixRegistry, msg: FixMsg) -> crate::Result<FixMsg> {
    // Restatement first, and not as a step a caller may skip: every
    // derivation reads by canonical name, and a child stored under an alias
    // is invisible until it has been canonicalized.
    let mut held = msg;
    super::latest::restate(&mut held)?;
    // The derivations, compiled and bound once per registry; a refused
    // compile is the pass's to report, since a dictionary whose rules do not
    // compile has no rules to fill by.
    registry.derivations()?.fill_all(&mut held)?;
    if held.get_identifiers().is_empty() {
        if let Some(component) = registry.get_msgtype(held.header().msgtype()) {
            let identifiers = component.identifier_mapping(&held)?;
            if !identifiers.is_null() {
                held.set_unsettled(super::IDENTIFIERS_TAG_NAME.0, identifiers)?;
            }
        }
    }
    // Settled once, at the end: a built message arrives unsettled, a
    // restatement leaves it so and the writes above land unsettled - so
    // every message is settled here, once, after everything the pass wrote.
    held.settle();
    Ok(held)
}

#[derive(Debug, PartialEq, Eq, Hash)]
enum DeliveryKey {
    Session {
        beginstring: SmolStr,
        sender: SmolStr,
        target: SmolStr,
        sender_sub: Option<SmolStr>,
        target_sub: Option<SmolStr>,
        sender_location: Option<SmolStr>,
        target_location: Option<SmolStr>,
        capture_session: Option<SmolStr>,
        sequence: u64,
        original_time: i64,
        content: u128,
    },
    /// A headerless bridge row can only prove an exact repeated event. Its
    /// capture facts keep equal content observed in distinct contexts apart.
    Exact {
        uuid: crate::Uuid,
        content: u128,
        sequence: Option<u64>,
        capture_session: Option<SmolStr>,
        capture_context: Option<SmolStr>,
        direction: Option<SmolStr>,
    },
}

fn text(message: &FixMsg, tag: i32) -> Option<SmolStr> {
    message
        .get_by_tag(tag)
        .and_then(|value| value.as_str().map(SmolStr::new))
}

fn delivery_key(message: &FixMsg) -> DeliveryKey {
    let header = message.header();
    let content = message.digest();
    let capture_session = message.capture().msgsessionid().map(SmolStr::new);
    let (Some(sender), Some(target), Some(sequence)) = (
        header.sendercompid(),
        header.targetcompid(),
        header.msgseqnum(),
    ) else {
        return DeliveryKey::Exact {
            uuid: message.get_curruuid(),
            content,
            sequence: header.msgseqnum(),
            capture_session,
            capture_context: message.capture().msgctxid().map(SmolStr::new),
            direction: header.msgdirection().map(SmolStr::new),
        };
    };
    let replay = header.possdupflag() == Some(true)
        || message.get_by_tag(97).is_some_and(|value| {
            value.as_bool() == Some(true)
                || value
                    .as_str()
                    .is_some_and(|value| value.eq_ignore_ascii_case("Y"))
        });
    let original_time = if replay {
        message
            .get_by_tag(122)
            .and_then(|value| value.temporal_count_at(crate::TimeUnit::Nanosecond))
            .unwrap_or_else(|| header.sendingtime())
    } else {
        header.sendingtime()
    };
    DeliveryKey::Session {
        beginstring: SmolStr::new(header.beginstring()),
        sender: SmolStr::new(sender),
        target: SmolStr::new(target),
        sender_sub: text(message, 50),
        target_sub: text(message, 57),
        sender_location: text(message, 142),
        target_location: text(message, 143),
        capture_session,
        sequence,
        original_time,
        content,
    }
}

/// Sorted messages prepared in lifecycle order: retransmissions removed and
/// missing instrument codes learned only from messages already observed.
struct Prepared {
    source: vec::IntoIter<FixMsg>,
    codes: InstrumentCodes,
    /// At most one key per distinct delivery in this already collected finite
    /// capture. A late retransmission must remain a repeat after any number of
    /// intervening deliveries; retaining only a recent window loses that fact.
    seen: HashSet<DeliveryKey>,
}

impl Prepared {
    fn new(source: Vec<FixMsg>) -> Self {
        // Reserve a small capture once, without reserving a giant repeated
        // capture's upper bound. Growth beyond this hint follows unique keys.
        let capacity = source.len().min(4_096);
        Self {
            source: source.into_iter(),
            codes: InstrumentCodes::default(),
            seen: HashSet::with_capacity(capacity),
        }
    }
}

impl Iterator for Prepared {
    type Item = FixMsg;

    fn next(&mut self) -> Option<FixMsg> {
        loop {
            let mut message = self.source.next()?;
            if !self.seen.insert(delivery_key(&message)) {
                continue;
            }
            self.codes.enrich(&mut message);
            return Some(message);
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, self.source.size_hint().1)
    }
}

impl FusedIterator for Prepared {}

/// A finite capture walked in event-time order. Intake failures are reported
/// before messages because sorting necessarily consumes the capture first.
pub(super) struct Walked {
    walk: EventIterator<FixMsg, Prepared>,
    failures: VecDeque<Error>,
}

impl Walked {
    pub(super) fn new<I>(source: I, snapshot_ns: i64) -> Self
    where
        I: Iterator<Item = Result<FixMsg>>,
    {
        let mut messages = Vec::new();
        let mut failures = VecDeque::new();
        for held in source {
            match held {
                Ok(message) => messages.push(message),
                Err(error) => failures.push_back(error),
            }
        }
        messages.sort_by(order);
        Self {
            walk: EventIterator::new(Prepared::new(messages), true).with_snapshot_ns(snapshot_ns),
            failures,
        }
    }
}

impl Iterator for Walked {
    type Item = Result<FixMsg>;

    fn next(&mut self) -> Option<Result<FixMsg>> {
        if let Some(error) = self.failures.pop_front() {
            return Some(Err(error));
        }
        self.walk.next().map(Ok)
    }
}

impl FusedIterator for Walked {}
