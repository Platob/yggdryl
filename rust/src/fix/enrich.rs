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
//! # The registry remains the contract; the shipped plan is native
//!
//! Each table is stored on its target field as one `FIX:derivation` term in
//! the crate's expression grammar, spelled by canonical folded field names
//! and read with [`FixField::derivation`](crate::FixField::derivation). The
//! generator also emits the exact complete signature of the twenty-nine
//! shipped terms. A registry carrying that signature over the canonical
//! source, target and group shapes takes the direct native evaluator; any
//! edited, removed or added term takes the generic evaluator for the whole
//! registry. Thus adding a desk rule remains a field edit, exactly as adding
//! a [replacement](super::latest) is, without mixing two evaluators inside
//! one dependency graph.
//!
//! # One selection per registry, one native overlay or generic row per message
//!
//! A registry carrying the shipped signature validates the exact metadata and
//! field shapes, then keeps only the native-plan marker: none of its terms are
//! parsed, bound or retained. A custom registry compiles every derivation once,
//! with every term parsed, every name proven to be a field or group, and every
//! expression bound, and keeps [`Derivations`] until a field changes. Its plan
//! gathers the ordered union of every column any term reads or fills into a
//! typed working row, then evaluates the bound terms in tag order. Both paths
//! expose each accepted answer to later rules, repeat until one sweep writes
//! nothing, and finish through one [`FixMsg::set_each`], so chains settle in
//! either direction without rebuilding the message between rules. Nothing is
//! kept per message shape or between messages.
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
//! An absent input is null on either plan, so a term over it answers null
//! under the grammar's three-valued rules and the target stays unfilled; a
//! condition that does not hold answers null the same way. A
//! value the target's field refuses - an identifier whose check digit does
//! not close, a spelling a code set does not read - is silence, as one
//! refused [`FixMsg::set`] is. The cost of silence is a null column; the
//! cost of a guess is a wrong number nobody can tell from a sent one.
//!
//! A dictionary whose rules do not compile has no rules to fill by, and that
//! is not silence: the compile's refusal names the field, the registry keeps
//! it beside what it would have kept of a compiled list, and every door -
//! the enrichment and the row fill alike - answers it until a field changes.

use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::iter::FusedIterator;
use std::vec;

use smol_str::{SmolStr, format_smolstr};

use crate::expression::{Bound, Term};
use crate::graph::instrument::InstrumentCodes;
use crate::graph::iterator::order;
use crate::graph::{Element, Event, EventIterator};
use crate::{DataType, Error, Field, FixCategory, Result, Scalar, State, StructType, Uuid};

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
                    msg.regrouped(counter, &self.field, held.into_owned())
                }),
            Source::Absent => Scalar::Null,
        }
    }
}

/// The `FIX:derivation` plan a registry carries, selected once.
///
/// Built by `FixRegistry::derivations`, which is private, and kept on the
/// registry until a
/// field changes; every codec and every message reading that registry
/// evaluates through the same instance, the line door, the batch door and
/// the row fill alike.
pub struct Derivations {
    /// Whether this registry carries the complete shipped rule set over the
    /// canonical field and group shapes the native evaluator implements.
    /// Any customization selects the generic list for the whole registry.
    native: bool,
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
    /// Selects the exact shipped native plan, or compiles every custom
    /// `FIX:derivation` and proves it against the registry.
    ///
    /// The shipped signature is checked directly against canonical metadata
    /// and its expected field shapes, so that path parses, binds and retains
    /// no generic terms or working schema. On the custom path, every column a
    /// term reads must be a field or a group the registry
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
        if native_registry(registry) {
            return Ok(Self {
                native: true,
                list: Vec::new(),
                inputs: Vec::new(),
                crate_reads: Vec::new(),
            });
        }
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
        carried.sort_by_key(|(tag, _, _)| *tag);
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
            native: false,
            list,
            inputs,
            crate_reads,
        })
    }

    /// The working schema: the Struct every term is bound against, one
    /// column per field or group any derivation reads or fills, as the
    /// first bound term holds it; `None` where no term bound.
    #[cfg(feature = "internals")]
    pub fn schema(&self) -> Option<&Field> {
        self.list
            .iter()
            .find_map(|derivation| derivation.bound.as_ref())
            .map(Bound::schema)
    }

    /// Each derived tag in sweep order, beside whether its term bound.
    #[cfg(feature = "internals")]
    pub fn derived(&self) -> impl Iterator<Item = (i32, bool)> + '_ {
        self.list
            .iter()
            .map(|derivation| (derivation.tag, derivation.bound.is_some()))
    }

    /// Whether the complete shipped rule set takes the direct evaluator.
    #[cfg(feature = "internals")]
    pub const fn is_native(&self) -> bool {
        self.native
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
        if self.native {
            return super::native_derivations::fill_all(msg);
        }
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

/// Whether the registry is exactly the generated shipped plan. This reads raw
/// canonical metadata only: parsing and binding it would rebuild the generic
/// plan whose startup and retained state the native path exists to remove.
fn native_registry(registry: &FixRegistry) -> bool {
    let mut expected = super::constants::SHIPPED_DERIVATIONS.iter();
    for field in registry.iter() {
        let Some(term) = field.get_metadata("FIX:derivation") else {
            continue;
        };
        let Some(&(expected_tag, expected_term)) = expected.next() else {
            return false;
        };
        let Some((tag, _)) = registry.identity_of(field) else {
            return false;
        };
        if tag != expected_tag || term != expected_term {
            return false;
        }
    }
    expected.next().is_none() && super::native_derivations::supports(registry)
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
    enrich_restated(registry, held)
}

/// [`enrich`] past its restatement, for a message whose row is already
/// restated: a message redated keeps the row it was built with, and what
/// its new clock can move is a derivation and its identity - never a rule,
/// which reads the row and not the clock.
pub(super) fn enrich_restated(registry: &FixRegistry, msg: FixMsg) -> crate::Result<FixMsg> {
    let mut held = msg;
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
        capture_context: Option<SmolStr>,
        sequence: u64,
        original_time: i64,
        content: u64,
    },
    /// A headerless bridge row can only prove an exact repeated event. Its
    /// capture facts keep equal content observed in distinct contexts apart.
    Exact {
        uuid: crate::Uuid,
        content: u64,
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
    let content = message.get_currhashcode();
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
    let sending_time = if header.stated_sendingtime() {
        header.sendingtime()
    } else {
        message.get_currunix()
    };
    let original_time = if replay {
        message
            .get_by_tag(122)
            .and_then(|value| value.temporal_count_at(crate::TimeUnit::Nanosecond))
            .unwrap_or(sending_time)
    } else {
        sending_time
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
        capture_context: message.capture().msgctxid().map(SmolStr::new),
        sequence,
        original_time,
        content,
    }
}

/// The prebuilt identity that proves two rows are observations of one FIX
/// session event. Lifecycle outputs keep their source key but are not raw
/// observations to coalesce on replay.
fn session_event_key(message: &FixMsg) -> Option<SmolStr> {
    // A lifecycle output is already placed. Expiries keep their source
    // message's capture key, and snapshots keep it while adding a view
    // clock; neither is another raw observation to coalesce on replay.
    if message.get_prevuuid().is_some() || message.get_snapunix().is_some() {
        return None;
    }
    message.session_event_identifier().map(SmolStr::new)
}

/// One session event while all of its raw observations are collected.
struct SessionEventObservations {
    message: FixMsg,
    others: Vec<FixMsg>,
}

/// Latest recording first; the event instant breaks absent/equal recording
/// ties exactly as the graph's reference selection does.
fn reference_order(left: &FixMsg, right: &FixMsg) -> Ordering {
    let right_leads = crate::graph::element::right_is_reference(
        crate::graph::element::reference_recdunix(left),
        left.get_currunix(),
        crate::graph::element::reference_recdunix(right),
        right.get_currunix(),
    );
    if right_leads {
        return Ordering::Greater;
    }
    let left_leads = crate::graph::element::right_is_reference(
        crate::graph::element::reference_recdunix(right),
        right.get_currunix(),
        crate::graph::element::reference_recdunix(left),
        left.get_currunix(),
    );
    if left_leads {
        Ordering::Less
    } else {
        Ordering::Equal
    }
}

/// Fully merges observations carrying one complete session-event identity before
/// the lifecycle walk can mistake them for successive events. The most
/// recently recorded message is the retained FIX row; the graph fold unions
/// the other observations into it and keeps the earliest per-event clocks.
fn merge_session_events(messages: Vec<FixMsg>, failures: &mut VecDeque<Error>) -> Vec<FixMsg> {
    let mut positions = HashMap::with_capacity(messages.len().min(4_096));
    let mut merged: Vec<SessionEventObservations> = Vec::with_capacity(messages.len());

    for message in messages {
        let Some(key) = session_event_key(&message) else {
            merged.push(SessionEventObservations {
                message,
                others: Vec::new(),
            });
            continue;
        };
        match positions.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(merged.len());
                merged.push(SessionEventObservations {
                    message,
                    others: Vec::new(),
                });
            }
            Entry::Occupied(entry) => merged[*entry.get()].others.push(message),
        }
    }

    merged
        .into_iter()
        .map(|mut held| {
            if held.others.is_empty() {
                return held.message;
            }
            held.others.push(held.message);
            held.others.sort_by(reference_order);
            let mut observations = held.others.into_iter();
            let mut reference = observations.next().expect("one session-event observation");
            for other in observations {
                match reference.clone().merge_session_event(&other) {
                    Ok(merged) => reference = merged,
                    Err(error) => {
                        failures.push_back(error);
                    }
                }
            }
            reference
        })
        .collect()
}

/// One FIX message while the generic event walk selects its exact
/// predecessor. The wrapper adds the protocol's predecessor-derived order
/// spellings inside `with_previous`, so the copy retained by the walk and the
/// copy it yields are the same enriched message.
#[derive(Clone)]
struct LifecycleMessage {
    message: FixMsg,
    failure: Option<SmolStr>,
}

impl From<FixMsg> for LifecycleMessage {
    fn from(message: FixMsg) -> Self {
        Self {
            message,
            failure: None,
        }
    }
}

impl Element for LifecycleMessage {
    fn get_curruuid(&self) -> Uuid {
        self.message.get_curruuid()
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.message.set_curruuid(curruuid);
    }

    fn get_crossuuid(&self) -> Uuid {
        self.message.get_crossuuid()
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.message.set_crossuuid(crossuuid);
    }

    fn get_crosscode(&self) -> &str {
        self.message.get_crosscode()
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.message.set_crosscode(crosscode);
    }

    fn get_currhashcode(&self) -> u64 {
        self.message.get_currhashcode()
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.message.set_currhashcode(hashcode);
    }

    fn get_crosshashcode(&self) -> u64 {
        self.message.get_crosshashcode()
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.message.set_crosshashcode(crosshashcode);
    }

    fn get_identifiers(&self) -> &BTreeMap<String, String> {
        self.message.get_identifiers()
    }

    fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) {
        self.message.set_identifiers(identifiers);
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        self.message.get_srcuuids()
    }

    fn set_srcuuids(&mut self, sources: Vec<Uuid>) {
        self.message.set_srcuuids(sources);
    }

    fn is_after(&self, other: &Self) -> bool {
        self.message.is_after(&other.message)
    }

    fn finalize(&mut self) {
        self.message.finalize();
    }

    fn with_previous(self, previous: &Self) -> Option<Self> {
        let mut message = self.message;
        let mut failure = self.failure;
        if message.should_merge_session_event(&previous.message) {
            return match message.clone().merge_session_event(&previous.message) {
                Ok(message) => Some(Self { message, failure }),
                Err(error) => {
                    if failure.is_none() {
                        failure = Some(format_smolstr!("{error}"));
                    }
                    Some(Self { message, failure })
                }
            };
        }
        let inherited = if failure.is_none() {
            match super::latest::inherit_order_links(&mut message, &previous.message) {
                Ok(changed) => changed,
                Err(error) => {
                    failure = Some(format_smolstr!("{error}"));
                    false
                }
            }
        } else {
            false
        };
        let fallback = inherited.then(|| message.clone());
        let message = match message.with_previous(&previous.message) {
            Some(message) => message,
            None => fallback?,
        };
        Some(Self { message, failure })
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        Some(Self {
            message: self.message.merge_with(&other.message)?,
            failure: self.failure.or_else(|| other.failure.clone()),
        })
    }
}

impl Event for LifecycleMessage {
    fn restating(self, live: &Self) -> Self {
        Self {
            message: self.message.restating(&live.message),
            failure: self.failure.or_else(|| live.failure.clone()),
        }
    }

    fn get_currunix(&self) -> i64 {
        self.message.get_currunix()
    }

    fn set_currunix(&mut self, unix: i64) {
        self.message.set_currunix(unix);
    }

    fn get_state(&self) -> &State {
        self.message.get_state()
    }

    fn set_state(&mut self, state: State) {
        self.message.set_state(state);
    }

    fn is_execution(&self) -> bool {
        self.message.is_execution()
    }

    fn get_seqnum(&self) -> u64 {
        self.message.get_seqnum()
    }

    fn set_seqnum(&mut self, seqnum: u64) {
        self.message.set_seqnum(seqnum);
    }

    fn get_creaunix(&self) -> Option<i64> {
        self.message.get_creaunix()
    }

    fn set_creaunix(&mut self, unix: Option<i64>) {
        self.message.set_creaunix(unix);
    }

    fn get_execunix(&self) -> Option<i64> {
        self.message.get_execunix()
    }

    fn set_execunix(&mut self, unix: Option<i64>) {
        self.message.set_execunix(unix);
    }

    fn get_recdunix(&self) -> Option<i64> {
        self.message.get_recdunix()
    }

    fn set_recdunix(&mut self, unix: Option<i64>) {
        self.message.set_recdunix(unix);
    }

    fn get_refrecdunix(&self) -> Option<i64> {
        self.message.get_refrecdunix()
    }

    fn set_refrecdunix(&mut self, unix: Option<i64>) {
        self.message.set_refrecdunix(unix);
    }

    fn get_exprtime(&self) -> Option<i64> {
        self.message.get_exprtime()
    }

    fn set_exprtime(&mut self, unix: Option<i64>) {
        self.message.set_exprtime(unix);
    }

    fn get_prevunix(&self) -> Option<i64> {
        self.message.get_prevunix()
    }

    fn set_prevunix(&mut self, unix: Option<i64>) {
        self.message.set_prevunix(unix);
    }

    fn get_prevuuid(&self) -> Option<Uuid> {
        self.message.get_prevuuid()
    }

    fn set_prevuuid(&mut self, uuid: Option<Uuid>) {
        self.message.set_prevuuid(uuid);
    }

    fn get_snapunix(&self) -> Option<i64> {
        self.message.get_snapunix()
    }

    fn set_snapunix(&mut self, unix: Option<i64>) {
        self.message.set_snapunix(unix);
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
    type Item = LifecycleMessage;

    fn next(&mut self) -> Option<LifecycleMessage> {
        loop {
            let mut message = self.source.next()?;
            if !self.seen.insert(delivery_key(&message)) {
                continue;
            }
            self.codes.enrich(&mut message);
            return Some(message.into());
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
    walk: EventIterator<LifecycleMessage, Prepared>,
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
        let mut messages = merge_session_events(messages, &mut failures);
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
        self.walk.next().map(|held| match held.failure {
            Some(reason) => Err(Error::InvalidRecord {
                path: "fix.lifecycle".into(),
                reason,
            }),
            None => Ok(held.message),
        })
    }
}

impl FusedIterator for Walked {}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/fix/mod_.rs` pins and a caller cannot reach.
    //!
    //! `fix::enrich` is a private module of a published one, so `Derivations`
    //! being `pub` reaches nobody: this door is the only path to it, and it
    //! exists under the `internals` feature alone. A registry's compiled
    //! derivations are reached through
    //! [`fix_registry::derivations`](crate::internals::fix_registry::derivations);
    //! the type is named here so that signature is public.
    pub use super::Derivations;
}
