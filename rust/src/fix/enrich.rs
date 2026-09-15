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
//! # One compile, one bind per shape, one rebuild per message
//!
//! A registry compiles its derivations once - every term parsed, every name
//! it reads proven to be a field or a group, every term typed against the
//! registry's own fields - and keeps [`Derivations`] until a field changes.
//! A message's root is widened, once per distinct root shape, with the
//! columns the derivations read or fill that it lacks, typed by the
//! registry's field for each, and every term is bound against that widened
//! root; a stream of a million same-shaped messages binds once. Per message
//! the pass copies the row into a working row over the widened schema, the
//! absent columns null, and sweeps the derivations in tag order: a target the
//! row holds non-null is skipped, else the term is evaluated and a non-null
//! answer the target's field accepts is written into the working row where
//! the next derivation reads it. Sweeps repeat until one writes nothing,
//! bounded by the number of derivations, which is what settles a chain in
//! either direction - `securityid` from `isincode` and `isincode` from
//! `securityid`, `product` after `securitytype` after `cficode` - without a
//! hand-laid order. Everything that landed then reaches the message through
//! one [`FixMsg::set_each`], one rebuild for the whole pass.
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
//! An absent input is a null column of the widened row, so a term over it
//! answers null under the grammar's three-valued rules and the target stays
//! unfilled; a condition that does not hold answers null the same way. A
//! value the target's field refuses - an identifier whose check digit does
//! not close, a spelling a code set does not read - is silence, as one
//! refused [`FixMsg::set`] is. The cost of silence is a null column; the
//! cost of a guess is a wrong number nobody can tell from a sent one.

use std::sync::{Arc, Mutex, PoisonError};

use smol_str::{SmolStr, format_smolstr};

use crate::expression::{Bound, Term};
use crate::types::nested::Fields;
use crate::{DataType, Error, Field, FixCategory, Result, Scalar};

use super::msg::FixMsg;
use super::registry::FixRegistry;

/// How many distinct root shapes a compiled registry keeps bound terms for.
///
/// A capture is a handful of shapes - the message types it carries, each
/// with the columns its venue states - read a million times, so a bound
/// shape is reused far more often than it is built; a shape past this many
/// evicts the oldest, and a pathological stream of a new shape per message
/// pays one bind per message and never grows.
const SHAPES: usize = 16;

/// One field's derivation, as the registry compiled it.
struct Derivation {
    /// The tag the derivation fills.
    tag: i32,
    /// The registry's field for that tag, which types every answer exactly
    /// as [`FixMsg::set`] would type it on the way in.
    field: Field,
    /// The term, as the field spells it in `fix:derivation`.
    term: Term,
}

/// The derivations bound against one root shape.
struct Shape {
    /// The root's children as they were: the key a message is matched by.
    columns: Fields,
    /// The root widened with every column a derivation reads or fills that
    /// the root lacks, each typed by the registry's field.
    schema: Field,
    /// One bound term per derivation, in list order; `None` where this shape
    /// cannot bind the term, which is silence for it on every such message.
    bound: Vec<Option<Bound>>,
    /// Each derivation's target column in `schema`.
    slots: Vec<usize>,
}

/// Every `fix:derivation` a registry carries, compiled once (decision 38).
///
/// Built by [`FixRegistry::derivations`] and kept on the registry until a
/// field changes; every codec and every message reading that registry
/// evaluates through the same instance, so the bound shapes it caches serve
/// the line door, the batch door and the row fill alike.
pub(super) struct Derivations {
    /// In tag order, which is the order one sweep evaluates them in.
    list: Vec<Derivation>,
    /// Every root column a term reads or a derivation fills, beside the
    /// registry's field for it: what a root is widened with where it lacks
    /// the column, in first-seen order.
    columns: Vec<Field>,
    /// The shapes bound so far, oldest first.
    shapes: Mutex<Vec<Arc<Shape>>>,
}

impl Derivations {
    /// Reads every field's `fix:derivation` and proves it against the
    /// registry.
    ///
    /// Every column a term reads must be a field or a group the registry
    /// names, and the term must type against those fields - so a name the
    /// dictionary lacks, or `orderqty - symbol`, is refused here naming the
    /// field, not once per message. The crate's own columns are the one
    /// exception: their derivations read the standard's fields by name, and
    /// a registry built from a handful of fields holds none of them, so for
    /// a crate field a name the registry lacks is an input the dictionary
    /// never states - widened as a null column the term answers null over -
    /// and a term that cannot bind over such columns is silent rather than
    /// a refusal of the registry that lacks them.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] at the registry field whose
    /// derivation does not parse, reads a column no field or group of this
    /// registry answers to, or does not type against the fields it reads.
    pub(super) fn compile(registry: &FixRegistry) -> Result<Self> {
        let mut list: Vec<Derivation> = Vec::new();
        let mut columns: Vec<Field> = Vec::new();
        for field in registry.iter() {
            let Some(term) = field
                .as_fix()
                .derivation()
                .map_err(|error| refused(field.name(), &error))?
            else {
                continue;
            };
            let Some((tag, _)) = registry.identity_of(field) else {
                continue;
            };
            let crated = super::is_crate_tag(tag);
            let mut inputs: Vec<Field> = Vec::new();
            for name in term.columns() {
                match column_of(registry, &name) {
                    Some(known) => {
                        widen(&mut inputs, known);
                        widen(&mut columns, known);
                    }
                    None if crated => {
                        let absent = DataType::Null.nullable_field(name);
                        widen(&mut inputs, &absent);
                        widen(&mut columns, &absent);
                    }
                    None => {
                        return Err(refused(
                            field.name(),
                            &format_smolstr!(
                                "fix:derivation reads `{name}`, which names no field or group \
                                 of the registry"
                            ),
                        ));
                    }
                }
            }
            // Typed once against the registry's own fields: what refuses
            // here would refuse for every message, and is the field's to fix.
            let schema = DataType::from_fields(inputs)?.required_field(field.name());
            if let Err(error) = term.bind(&schema) {
                if crated {
                    continue;
                }
                return Err(refused(field.name(), &error));
            }
            widen(&mut columns, field);
            list.push(Derivation {
                tag,
                field: field.clone(),
                term,
            });
        }
        // The registry iterates tag-major already; stated here so the sweep
        // order is this list's contract rather than the iteration's.
        list.sort_by_key(|derivation| derivation.tag);
        Ok(Self {
            list,
            columns,
            shapes: Mutex::new(Vec::new()),
        })
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
    /// Returns the widened root's refusal - two children folding to one
    /// name - or the schema grammar's refusal when the written children do
    /// not make a root.
    pub(super) fn fill_all(&self, msg: &mut FixMsg) -> Result<()> {
        if self.list.is_empty() {
            return Ok(());
        }
        let shape = self.shape_of(msg.as_field())?;
        let values = msg.as_value().as_sequence().unwrap_or_default();
        let mut row: Vec<Scalar> = Vec::with_capacity(shape.schema.field_len());
        row.extend(values.iter().cloned());
        row.resize(shape.schema.field_len(), Scalar::Null);
        let mut landed: Vec<(i32, Scalar)> = Vec::new();
        // A productive sweep fills at least one target and a filled target
        // is never revisited, so the derivation count bounds the sweeps; the
        // one past it is the sweep that writes nothing.
        for _ in 0..=self.list.len() {
            let mut wrote = false;
            for (index, derivation) in self.list.iter().enumerate() {
                let slot = shape.slots[index];
                // A stated value is never overwritten, which is what makes
                // this idempotent: the second pass finds the first pass's
                // answer stated.
                if !row[slot].is_null() {
                    continue;
                }
                let Some(bound) = &shape.bound[index] else {
                    continue;
                };
                let Some(value) = answer(bound, &derivation.field, &row) else {
                    continue;
                };
                row[slot] = value.clone();
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

    /// One derivation's answer for `msg` as it stands, or nothing.
    ///
    /// What a row fill asks for a crate column the message does not state:
    /// the one evaluation of that column's own term, over the message's row
    /// with every absent column null, typed by nothing - the column that
    /// asked types it, as it types a stated value. `None` for a tag no field
    /// derives, a shape the term does not bind against, or an answer of
    /// null.
    pub(super) fn fill(&self, tag: i32, msg: &FixMsg) -> Option<Scalar> {
        let index = self
            .list
            .iter()
            .position(|derivation| derivation.tag == tag)?;
        let shape = self.shape_of(msg.as_field()).ok()?;
        let bound = shape.bound[index].as_ref()?;
        let value = bound.eval_padded(msg.as_value().as_sequence()?).ok()?;
        (!value.is_null()).then_some(value)
    }

    /// The derivations bound against `root`'s shape, built on the first ask.
    ///
    /// Keyed as the column plan is (`column_plan_of`): the root's `Fields`
    /// by storage identity first, which a stream sharing one schema answers
    /// without a comparison, then structurally, which a root rebuilt by a
    /// write answers without a bind.
    fn shape_of(&self, root: &Field) -> Result<Arc<Shape>> {
        let DataType::Struct(columns) = root.dtype() else {
            return Err(Error::InvalidRecord {
                path: SmolStr::new(root.name()),
                reason: SmolStr::new_static("expected a Struct root to derive over"),
            });
        };
        {
            let held = self.shapes.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(shape) = held.iter().find(|shape| {
                shape.columns.shares_storage_with(columns) || shape.columns == *columns
            }) {
                return Ok(Arc::clone(shape));
            }
        }
        let shape = Arc::new(self.bind_shape(root, columns.clone())?);
        let mut held = self.shapes.lock().unwrap_or_else(PoisonError::into_inner);
        if held.len() >= SHAPES {
            held.remove(0);
        }
        held.push(Arc::clone(&shape));
        Ok(shape)
    }

    /// Widens `root` with every column it lacks and binds every term.
    fn bind_shape(&self, root: &Field, columns: Fields) -> Result<Shape> {
        let mut fields: Vec<Field> = root.fields().to_vec();
        for known in &self.columns {
            if position_of(&fields, known.name()).is_none() {
                fields.push(known.clone());
            }
        }
        let schema = Field::new(
            root.name(),
            DataType::from_fields(fields)?,
            root.is_nullable(),
        );
        let mut bound = Vec::with_capacity(self.list.len());
        let mut slots = Vec::with_capacity(self.list.len());
        for derivation in &self.list {
            // A shape the term does not bind against - a child a venue typed
            // as the term cannot read - is silence for that derivation, as a
            // value the field refuses is; the registry's own fields already
            // proved the term at compile.
            bound.push(derivation.term.bind(&schema).ok());
            slots.push(
                position_of(schema.fields(), derivation.field.name()).ok_or_else(|| {
                    Error::InvalidRecord {
                        path: SmolStr::new(derivation.field.name()),
                        reason: SmolStr::new_static("the widened root lacks the derived column"),
                    }
                })?,
            );
        }
        Ok(Shape {
            columns,
            schema,
            bound,
            slots,
        })
    }
}

/// The value one bound derivation answers for the working row, typed for
/// the field it fills, or nothing: an evaluation the grammar refuses, an
/// answer of null, and a value the field refuses are all silence.
fn answer(bound: &Bound, field: &Field, row: &[Scalar]) -> Option<Scalar> {
    let value = bound.eval_padded(row).ok()?;
    if value.is_null() {
        return None;
    }
    field.scalar(value).ok()
}

/// The registry's column one name reaches: a group before a scalar field,
/// because a group's column is named by the group and its counter is a
/// field of its own.
fn column_of<'registry>(registry: &'registry FixRegistry, name: &str) -> Option<&'registry Field> {
    registry
        .get_definition(FixCategory::Groups, name)
        .or_else(|| registry.get_field_by_name(name))
}

/// Adds `known` to the columns a root is widened with, once per name.
fn widen(columns: &mut Vec<Field>, known: &Field) {
    if position_of(columns, known.name()).is_none() {
        columns.push(known.clone());
    }
}

/// Where a column stands, under the fold the binder resolves a name by.
fn position_of(fields: &[Field], name: &str) -> Option<usize> {
    fields
        .iter()
        .position(|field| field.name().eq_ignore_ascii_case(name))
}

/// A derivation refused at compile, naming the field that carries it.
fn refused(field: &str, reason: &dyn std::fmt::Display) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new(field),
        reason: format_smolstr!("{reason}"),
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
/// [`ENTRIES_COLUMN`](super::schema::ENTRIES_COLUMN), whatever the columns
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
fn recovered(mut msg: FixMsg) -> FixMsg {
    let mut dropped: Vec<(i32, Scalar)> = Vec::new();
    for entry in msg.entries() {
        let tag = entry.tag();
        // `0` is an unresolved key - a name or number with no registry
        // identity - and a tag the message holds needs
        // nothing: the record is read for what the projection lost, never to
        // restate what survived it.
        if tag <= 0 || msg.get_by_tag(tag).is_some() {
            continue;
        }
        // A pair that headed a subtree is that subtree's. Recovering a
        // group's counter alone would state a count with no occurrences
        // under it, which is a worse answer than the silence the projection
        // left: only a leaf pair comes back here.
        if !entry.children().is_empty() || dropped.iter().any(|(held, _)| *held == tag) {
            continue;
        }
        let Some(text) = entry.value().as_str() else {
            continue;
        };
        // `Scalar::from(&str)` holds the text as the compact string it is:
        // an owned `String` here would be built only for the value to copy
        // out of it and drop it again.
        dropped.push((tag, Scalar::from(text)));
    }
    for (tag, value) in dropped {
        // The dictionary's own field types the text on the way in. Unresolved
        // keys recorded tag 0 and were skipped above, so only a positive tag
        // reaches this write; one the dictionary lacks is kept under its decimal
        // spelling, as `set` keeps any such tag.
        let _ = msg.set(tag, value);
    }
    msg
}

/// Fills what `msg` implies, leaving what it stated and what arrived alone.
///
/// Four steps in order (decision 20, decision 38): what the row's projection
/// dropped comes back off the arrival record; the message is restated at the
/// dictionary's newest version; the registry's derivations fill what the
/// message implies, to a fixpoint; and the component's identifier
/// declaration fills the sorted `altids` Map. Every answer lands in the row
/// alone, typed by the dictionary's own field for the tag, so a derived
/// column is indistinguishable from a stated one and carries the same
/// display, description and `fix:tag` a reader resolves it by - and a value
/// it refuses, such as an identifier whose check digit does not close, is
/// silence. Identifier Map construction propagates the shared text
/// conversion's typed refusal if a declared member cannot spell text.
pub(super) fn enrich(registry: &FixRegistry, msg: FixMsg) -> crate::Result<FixMsg> {
    // What the row's projection dropped comes back off the arrival record
    // first, because restatement reads what the document stated and a row
    // states only its columns (decision 20).
    let msg = recovered(msg);
    // Restatement next, and not as a step a caller may skip: every
    // derivation reads by canonical name, and a child stored under an alias
    // is invisible until it has been canonicalized (decision 20).
    let mut held = super::latest::restate(msg)?;
    // The derivations, compiled once per registry and bound once per root
    // shape; a refused compile is the pass's to report, since a dictionary
    // whose rules do not compile has no rules to fill by.
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
