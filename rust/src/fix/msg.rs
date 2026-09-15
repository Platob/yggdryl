//! A FIX message: a value plus the registry that types it.

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use smol_str::{SmolStr, format_smolstr};

use super::anomaly::FixAnomalies;
use super::build::stated;
use super::entry::FixEntry;
use super::{FixId, FixKey, FixRegistry};
use crate::{DataType, Error, Field, FieldPath, FieldSegment, Result, Scalar, Version};

/// A FIX message value, resolved against one registry.
///
/// The schema is a core Struct [`Field`] - a non-null Struct field is the
/// only row schema - and the value is that row: a [`crate::types::nested::Record`] input
/// canonicalizes to the ordered [`crate::types::nested::Sequence`] the root declares,
/// exactly as every other row does. The registry link is an [`Arc`], cloned
/// from [`FixRegistry::global`] when the caller names none, so a message
/// carries the dictionary it was resolved against and a later lookup cannot
/// silently use a different one.
///
/// A message speaks no dialect of its own: the registry is one namespace,
/// and a bare tag or name resolves in it directly - a message transcribed
/// against a venue dictionary names its own fields by the venue's spellings
/// while still carrying `MsgType` and every other specification field, and
/// one namespace answers all of them. Which dictionaries a field belongs to
/// is the field's own `fix:branches`, a membership a reader may ask about
/// and nothing here resolves through.
///
/// A value is reached by tag, by identifier, by name, or by path, each
/// answering `Option<&Scalar>` with a failing twin; resolution goes through
/// the linked registry, never through a private copy of its rules. An unknown
/// tag is retained rather than dropped: it is looked for under its rendered
/// decimal name, which is where a transcriber keeps a tag no dictionary
/// explains.
/// A repeating-group counter remains an int32 value reached by its tag;
/// the separate collection is reached by name, such as `Parties`.
///
/// Every message stores non-null `updatedat`, `createdat`, `msghash`, `msgphash`,
/// `code`, `snapshotat` and `SendingTime(52)`. Initial intake settles clocks;
/// replay never reads now. The four direct accessors borrow their stored
/// values without lookup or allocation. `msghash` and `msgphash` are sixteen fixed
/// bytes each, never RFC identifiers: `msgphash` is the big-endian XXH3-128 of
/// the exact code bytes, and `msghash` the signed updatedat nanoseconds with
/// the sign bit flipped in bytes 0..8 beside all 64 bits of the named
/// content's XXH64 in bytes 8..16. These non-cryptographic identities are
/// separate from the immutable arrival record's [`Self::digest`].
///
/// Serialization is inherited, not written: `field.clone().into_json()`
/// renders the schema, [`into_json_scalar`](crate::into_json_scalar) the
/// value, and [`from_json_scalar_with_field`](crate::from_json_scalar_with_field)
/// reads a value back typed, ordered and canonicalized against that field.
///
/// ```
/// use std::sync::Arc;
///
/// use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, from_json_scalar_with_field, into_json_scalar};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut symbol = DataType::utf8().required_field("Symbol");
/// symbol.as_fix_mut().set_tag(55)?;
/// symbol.as_fix_mut().set_aliases(["Ticker"])?;
/// let mut qty = DataType::Int64.required_field("OrderQty");
/// qty.as_fix_mut().set_tag(38)?;
/// let registry = Arc::new(FixRegistry::from_fields([symbol.clone(), qty.clone()])?);
///
/// let root = DataType::from_fields([symbol, qty, DataType::utf8().nullable_field("9999")])?
///     .required_field("NewOrderSingle");
/// let value = Scalar::from_record([
///     ("Symbol", Scalar::from("AAPL")),
///     ("OrderQty", Scalar::from(100)),
///     ("9999", Scalar::from("custom")),
/// ])?;
/// let msg = FixMsg::with_registry(registry, root.clone(), value)?;
///
/// assert_eq!(msg.by_tag(55)?, &Scalar::from("AAPL"));
/// assert_eq!(msg.by_name("ticker")?, &Scalar::from("AAPL"));
/// assert_eq!(msg.by_tag(9999)?, &Scalar::from("custom"), "an unknown tag is kept");
/// assert_eq!(msg.updatedat(), msg.by_name("updatedat")?);
/// assert_eq!(msg.createdat(), msg.by_name("createdat")?);
/// assert_eq!(msg.msghash(), msg.by_name("msghash")?);
/// assert_eq!(msg.msgphash(), msg.by_name("msgphash")?);
///
/// // Both halves serialize through the paths every field and value share.
/// let root = msg.as_field();
/// let schema = root.clone().into_json()?;
/// let text = into_json_scalar(msg.as_value())?;
/// let read = from_json_scalar_with_field(&text, root)?;
/// assert_eq!(&read, msg.as_value());
/// assert!(schema.contains("fix:tag"));
/// # Ok(())
/// # }
/// ```
pub struct FixMsg {
    registry: Arc<FixRegistry>,
    /// What arrived, beside what it was interpreted as.
    ///
    /// Not the row restated: the row is the interpretation and this is the
    /// wire record. Neither derives from the other, which is what makes
    /// lossless re-emission possible at all. Empty for a message built from a
    /// schema and a value, in which case the emit falls back to the row and
    /// says so.
    entries: Vec<FixEntry>,
    /// Each root child's tag beside its position, in tag order.
    ///
    /// Resolved once, because reading a tag out of a child's metadata is a
    /// formatted key and a map lookup, and scanning the children per lookup
    /// pays it once per child. Every facet
    /// a lift answers is a tag lookup, so a row of twenty facets over a
    /// message of twenty children was four hundred of them.
    ///
    /// Derived from the shared field/registry column plan, replaced with the row.
    tags: Vec<(i32, usize)>,
    /// The first child of each name, built on the first tag no child
    /// declares and kept.
    ///
    /// [`Self::get_by_tag`] has two fallbacks past the index above - the
    /// child named as the dictionary names the tag, and the child named by
    /// the tag's decimal spelling - and both end in a child found by name. A
    /// fixed row asks for eighty columns a message mostly lacks, so the misses
    /// are the common case, and each costs one dictionary probe and one
    /// lookup here rather than a scan of the children. Derived from the
    /// field alone, and derived lazily, so a message nobody projects pays
    /// nothing for it.
    named: OnceLock<HashMap<SmolStr, usize>>,
    /// Group positions keyed by their `fix:counter`, separate from tag values.
    groups: Vec<(i32, usize)>,
    field: Field,
    value: Scalar,
    updatedat: Scalar,
    createdat: Scalar,
    msghash: Scalar,
    msgphash: Scalar,
}

/// One child a write lands: replaced at `at`, appended when there is none,
/// its field and value already resolved and typed.
pub(super) struct Write {
    pub(super) at: Option<usize>,
    pub(super) field: Field,
    pub(super) value: Scalar,
}

/// Adds one staged write to the batch, the later of two writes to one child
/// standing, whether both reached it or both would append it.
fn stage(writes: &mut Vec<Write>, write: Write) {
    let pending = writes.iter().position(|held| match (held.at, write.at) {
        (Some(held), Some(at)) => held == at,
        (None, None) => held.field.name() == write.field.name(),
        _ => false,
    });
    match pending {
        Some(pending) => writes[pending] = write,
        None => writes.push(write),
    }
}

/// What one landed write does to the tag and group indexes.
pub(super) struct Indexed {
    retired_tag: Option<i32>,
    retired_counter: Option<i32>,
    tag: Option<i32>,
    counter: Option<i32>,
    index: usize,
    /// Whether the write put a differently named child in a child's place.
    ///
    /// The name table is derived from the children's names, so only this
    /// makes it wrong: a replacement under the same name leaves every entry
    /// where it was, and an append only adds one.
    pub(super) renamed: bool,
    /// Whether the write added a child rather than replacing one.
    pub(super) appended: bool,
}

/// Stage proven writes without publishing a message or inventing a second row.
pub(super) fn stage_writes(
    root: &Field,
    row: &Scalar,
    writes: Vec<Write>,
) -> Result<(Field, Vec<Scalar>, Vec<Indexed>)> {
    let mut members = Vec::with_capacity(root.fields().len() + writes.len());
    members.extend_from_slice(root.fields());
    let held = row.as_sequence().unwrap_or_default();
    let mut values = Vec::with_capacity(held.len() + writes.len());
    values.extend_from_slice(held);
    // What each write does to the indexes, applied once the row is
    // proven rather than on a copy: the tag and counter the replaced
    // child held, then the ones the written child carries, at the
    // position it landed.
    let mut indexed: Vec<Indexed> = Vec::with_capacity(writes.len());
    for Write { at, field, value } in writes {
        let (index, replaced) = match at {
            Some(at) if at < members.len() && at < values.len() => {
                let replaced = std::mem::replace(&mut members[at], field);
                values[at] = value;
                (at, Some(replaced))
            }
            _ => {
                members.push(field);
                values.push(value);
                (members.len() - 1, None)
            }
        };
        let renamed = replaced
            .as_ref()
            .is_some_and(|replaced| replaced.name() != members[index].name());
        let appended = replaced.is_none();
        let held = replaced.as_ref().map(Field::as_fix);
        let written = members[index].as_fix();
        indexed.push(Indexed {
            retired_tag: held.as_ref().and_then(|held| held.tag().ok().flatten()),
            retired_counter: held.as_ref().and_then(|held| held.counter().ok().flatten()),
            tag: written.tag().ok().flatten(),
            counter: written.counter().ok().flatten(),
            index,
            renamed,
            appended,
        });
    }
    let dtype = DataType::from_fields(members)?;
    let field = Field::new_with_metadata(
        root.name(),
        dtype,
        root.is_nullable(),
        root.metadata.clone(),
    );
    Ok((field, values, indexed))
}

impl Indexed {
    pub(super) fn apply(
        &self,
        tags: &mut Vec<(i32, usize)>,
        groups: Option<&mut Vec<(i32, usize)>>,
    ) {
        retire(tags, self.retired_tag, self.index);
        admit(tags, self.tag, self.index);
        if let Some(groups) = groups {
            retire(groups, self.retired_counter, self.index);
            admit(groups, self.counter, self.index);
        }
    }
}

/// Each child's resolved tag beside its position, sorted for a binary search.
fn tag_positions(columns: &super::schema::Columns) -> Vec<(i32, usize)> {
    let mut held = Vec::with_capacity(columns.len());
    held.extend(
        columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| Some((column.tag?, index))),
    );
    held.sort_unstable();
    held
}

fn group_positions(field: &Field) -> Vec<(i32, usize)> {
    let mut held: Vec<_> = field
        .fields()
        .iter()
        .enumerate()
        .filter_map(|(index, child)| Some((child.as_fix().counter().ok()??, index)))
        .collect();
    held.sort_unstable();
    held
}

/// Emits entries pre-order: each pair, then everything that arrived under it.
///
/// Wire order is arrival order, and a member arrived after the counter that
/// heads it - so the pre-order walk is the wire, exactly.
fn emit_bytes(entries: &[FixEntry], separator: u8, bytes: &mut Vec<u8>) {
    for entry in entries {
        bytes.extend_from_slice(entry.key().as_bytes());
        bytes.push(b'=');
        bytes.extend_from_slice(entry.value().as_bytes());
        bytes.push(separator);
        emit_bytes(entry.children(), separator, bytes);
    }
}

/// The same walk as text, refusing a control byte at every depth.
fn emit_text(entries: &[FixEntry], separator: char, text: &mut String) -> Result<()> {
    for entry in entries {
        let key = entry.key();
        let refused = |reason: &'static str| Error::InvalidRecord {
            path: SmolStr::new(key.as_str().unwrap_or_default()),
            reason: reason.into(),
        };
        // Bytes that are not text have no rendering: a `data` field carrying
        // them re-emits exactly through `into_bytes` and not at all through
        // this one.
        let (Some(key), Some(value)) = (key.as_str(), entry.value().as_str()) else {
            return Err(refused(
                "expected a printable value, got bytes that are not text",
            ));
        };
        if value.chars().any(char::is_control) {
            return Err(refused("expected a printable value, got a control byte"));
        }
        text.push_str(key);
        text.push('=');
        text.push_str(value);
        text.push(separator);
        emit_text(entry.children(), separator, text)?;
    }
    Ok(())
}

impl FixMsg {
    /// The deterministic hash of this message's schema and row.
    /// Uses one allocation for the shared XXH3 state, independent of message size.
    #[must_use]
    pub fn stable_hash(&self) -> u64 {
        crate::hashing::stable_hash_of(self)
    }

    /// Builds a message against the process-wide registry.
    ///
    /// # Errors
    ///
    /// Returns the default registry's load failure, or the refusal
    /// [`Self::with_registry`] raises.
    pub fn new(field: Field, value: Scalar) -> Result<Self> {
        Self::with_registry(Arc::clone(FixRegistry::global()?), field, value)
    }

    /// Builds a message against an explicit registry.
    ///
    /// The value is validated through [`Field::validate_value`] and stored
    /// as [`Field::canonicalize_value`] rewrites it, so a record becomes the
    /// ordered sequence the root declares.
    ///
    /// # Errors
    ///
    /// Returns an error when the root is not a Struct field, or when the
    /// value violates it, naming the path of the first value that does not
    /// fit.
    pub fn with_registry(registry: Arc<FixRegistry>, field: Field, value: Scalar) -> Result<Self> {
        let plan = super::schema::column_plan(&field, &registry)?;
        let mut members = field.fields().to_vec();
        let mut changed = false;
        for (child, column) in members.iter_mut().zip(plan.iter()) {
            if column.tag.is_some_and(super::identity::is_mandatory) && !child.is_nullable() {
                child.set_nullable(true);
                changed = true;
            }
        }
        let field = if changed {
            Field::new_with_metadata(
                field.name(),
                DataType::from_fields(members)?,
                field.is_nullable(),
                field.metadata.clone(),
            )
        } else {
            field
        };
        let value = field.canonicalize_value(value)?;
        let (field, value, hard) = super::identity::fresh(&registry, field, value, None, &plan)?;
        let plan = super::schema::column_plan_of(&field, &registry)?;
        let tags = tag_positions(&plan);
        Ok(Self::resolved(
            registry,
            field,
            value,
            Vec::new(),
            tags,
            hard,
        ))
    }

    /// Builds a message a reader already resolved, entries and all.
    ///
    /// The value is checked and canonicalized exactly as
    /// [`Self::with_registry`] checks one; the reader's own build takes
    /// [`Self::from_built`], because what it built needs neither.
    ///
    /// # Errors
    ///
    /// Returns the refusal [`Self::with_registry`] raises.
    #[cfg(test)]
    pub(super) fn from_parts(
        registry: Arc<FixRegistry>,
        field: Field,
        value: Scalar,
        entries: Vec<FixEntry>,
    ) -> Result<Self> {
        let mut built = Self::with_registry(registry, field, value)?;
        built.entries = entries;
        Ok(built)
    }

    /// Builds the message the one builder finished, checking nothing twice.
    ///
    /// Every value in the row went through the contract of the field it
    /// lands under, so the row is canonical by construction, and the builder
    /// resolved each child's tag on the way in - re-checking either would be
    /// a second reading of what `scalar` already answered.
    pub(super) fn from_built(
        registry: Arc<FixRegistry>,
        built: super::build::Built,
        fallback_sending_time: Option<&Scalar>,
    ) -> Result<Self> {
        let super::build::Built {
            field,
            value,
            entries,
            ..
        } = built;
        let initial = super::schema::column_plan(&field, &registry)?;
        let (field, value, hard) =
            super::identity::fresh(&registry, field, value, fallback_sending_time, &initial)?;
        let plan = super::schema::column_plan_of(&field, &registry)?;
        let tags = tag_positions(&plan);
        Ok(Self::resolved(registry, field, value, entries, tags, hard))
    }

    /// One message from parts already proven: the group index is read off
    /// the root, and nothing else is derived yet.
    fn resolved(
        registry: Arc<FixRegistry>,
        field: Field,
        value: Scalar,
        entries: Vec<FixEntry>,
        tags: Vec<(i32, usize)>,
        hard: super::identity::Hard,
    ) -> Self {
        let groups = group_positions(&field);
        Self {
            registry,
            entries,
            tags,
            named: OnceLock::new(),
            groups,
            field,
            value,
            updatedat: hard.updatedat,
            createdat: hard.createdat,
            msghash: hard.msghash,
            msgphash: hard.msgphash,
        }
    }

    /// Borrow the settled message or snapshot grid instant without a lookup.
    pub const fn updatedat(&self) -> &Scalar {
        &self.updatedat
    }

    /// Borrow the settled creation instant without a lookup.
    pub const fn createdat(&self) -> &Scalar {
        &self.createdat
    }

    /// Borrow the time/content identity, sixteen bytes, without a lookup.
    pub const fn msghash(&self) -> &Scalar {
        &self.msghash
    }

    /// Borrow the code-only chain identity, sixteen bytes, without a lookup.
    pub const fn msgphash(&self) -> &Scalar {
        &self.msgphash
    }

    /// A resolved replay or restatement never defaults a clock.
    pub(super) fn settled_parts(
        registry: Arc<FixRegistry>,
        field: Field,
        value: Scalar,
        entries: Vec<FixEntry>,
        assertions: super::identity::Assertions,
    ) -> Result<Self> {
        let plan = super::schema::column_plan_of(&field, &registry)?;
        plan.identity.require_bundle(&field)?;
        let mut values = value
            .as_sequence()
            .ok_or_else(|| {
                super::identity::refused(field.name(), "a canonical Struct row", value.kind())
            })?
            .to_vec();
        let hard = plan.identity.finalize(&field, &mut values, assertions)?;
        let tags = tag_positions(&plan);
        Ok(Self::resolved(
            registry,
            field,
            Scalar::from_sequence(values),
            entries,
            tags,
            hard,
        ))
    }

    /// Returns what arrived and was read as sent, in arrival order,
    /// untranslated.
    #[must_use]
    pub fn entries(&self) -> &[FixEntry] {
        &self.entries
    }

    /// Takes the arrival record, consuming the message.
    ///
    /// What a rebuild needs: the entries are what arrived and are carried
    /// through unchanged, so moving them costs nothing where cloning a whole
    /// capture's worth would.
    pub(super) fn into_entries(self) -> Vec<FixEntry> {
        self.entries
    }

    /// Replaces the arrival record with one a row already held.
    ///
    /// The row's own reading of what arrived, not a rebuild from the
    /// children: the entries and the row stay two facts, and this only puts
    /// back the one a projection carried alongside the other.
    pub(super) fn set_entries(&mut self, entries: Vec<FixEntry>) {
        self.entries = entries;
    }

    /// Writes one value into the row, typed by the field the key reaches.
    ///
    /// Row only: the entries are what arrived and are not changed by what
    /// the row now says about it, so [`Self::into_bytes`] still re-emits the
    /// received line byte for byte. The key resolves as every lookup does -
    /// through the registry's one namespace - and a field the
    /// dictionary knows types the value through [`Field::scalar`] under the
    /// dictionary's own field, so a written child is indistinguishable from
    /// a stated one and carries the same `fix:tag` a reader resolves it by.
    /// A `Null` is stored as a stated null except for mandatory holders,
    /// which refuse it. Content writes recompute identities before publication;
    /// explicitly written identities must match the complete candidate state.
    ///
    /// An existing child is replaced where it stands, its field updated to
    /// the resolved one, and an absent one is appended rather than inserted
    /// in tag order: the positions every reader already holding the row
    /// addresses it by do not move, and a written field is found by tag.
    /// The tag and group indexes follow the one child that changed rather
    /// than being reread. A name the dictionary does not know still reaches
    /// a child spelled that way - exactly, or under the fold every name
    /// resolves by - and keeps that child's field; a bare tag it does not
    /// know appends a nullable `utf8` child named by its decimal, which is
    /// what the builder does with an unknown tag. A name that reaches
    /// neither a field nor a child is refused, and the message is unchanged.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use yggdryl::{DataType, FixMsg, FixRegistry, Scalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut qty = DataType::Int64.nullable_field("orderqty");
    /// qty.as_fix_mut().set_tag(38)?;
    /// let mut symbol = DataType::utf8().nullable_field("symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let registry = Arc::new(FixRegistry::from_fields([qty, symbol])?);
    ///
    /// let root = DataType::from_fields([DataType::utf8().required_field("symbol")])?
    ///     .required_field("D");
    /// let value = Scalar::from_record([("symbol", Scalar::from("AAPL"))])?;
    /// let mut msg = FixMsg::with_registry(registry, root, value)?;
    ///
    /// // Appended under the dictionary's field, typed by it and found by tag.
    /// msg.set(38, Scalar::from(100_i64))?;
    /// assert_eq!(msg.by_tag(38)?, &Scalar::from(100_i64));
    /// assert_eq!(msg.as_field().fields().last().unwrap().name(), "orderqty");
    ///
    /// // Replaced in place: the child keeps its position, the value changes.
    /// msg.set("Symbol", Scalar::from("MSFT"))?;
    /// assert_eq!(msg.as_field().fields()[0].name(), "symbol");
    /// assert_eq!(msg.by_tag(55)?, &Scalar::from("MSFT"));
    ///
    /// // A tag no dictionary explains is kept under its decimal spelling.
    /// msg.set(9999, Scalar::from("custom"))?;
    /// assert_eq!(msg.by_tag(9999)?, &Scalar::from("custom"));
    ///
    /// // A name nothing reaches is refused, and the row stands as it was.
    /// assert!(msg.set("nosuchfield", Scalar::from("x")).is_err());
    /// assert_eq!(msg.as_field().fields().len(), 10);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the key when it reaches no field and
    /// no child, the value contract's refusal when the value does not fit
    /// the field the key resolves to, or the schema grammar's refusal when
    /// the written child does not make a root. Any of them leaves the
    /// message unchanged.
    pub fn set<'key>(&mut self, key: impl Into<FixKey<'key>>, value: Scalar) -> Result<()> {
        self.set_many(std::iter::once((key, value)))
    }

    /// Writes several values into the row with one rebuild.
    ///
    /// Each key resolves and each value types exactly as [`Self::set`]
    /// resolves and types one, against the row as it stands before any of
    /// them lands; the row is then rebuilt once, where a write per value
    /// rebuilds it per value. A stream stamping three identities on every
    /// message is what this is for. Two writes reaching one child, or two
    /// appending one field, land as the later one, so the result is what the
    /// same writes made one at a time. Failure leaves the message unchanged,
    /// whichever write refused.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use yggdryl::{DataType, FixMsg, FixRegistry, Scalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut qty = DataType::Int64.nullable_field("orderqty");
    /// qty.as_fix_mut().set_tag(38)?;
    /// let mut symbol = DataType::utf8().nullable_field("symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let registry = Arc::new(FixRegistry::from_fields([qty, symbol])?);
    /// let root = DataType::from_fields([DataType::utf8().required_field("symbol")])?
    ///     .required_field("D");
    /// let value = Scalar::from_record([("symbol", Scalar::from("AAPL"))])?;
    /// let mut msg = FixMsg::with_registry(registry, root, value)?;
    ///
    /// msg.set_many([
    ///     (55, Scalar::from("MSFT")),
    ///     (38, Scalar::from(100_i64)),
    ///     (38, Scalar::from(200_i64)),
    /// ])?;
    /// assert_eq!(msg.by_tag(55)?, &Scalar::from("MSFT"));
    /// assert_eq!(msg.by_tag(38)?, &Scalar::from(200_i64), "the later write lands");
    /// assert_eq!(msg.as_field().fields().len(), 9, "two business fields and the seven-field replay bundle");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns what [`Self::set`] returns for the first write that refuses.
    pub fn set_many<'key, I, K>(&mut self, values: I) -> Result<()>
    where
        I: IntoIterator<Item = (K, Scalar)>,
        K: Into<FixKey<'key>>,
    {
        self.set_many_with(values, |_, _| Ok(()))
    }

    /// Check each resolved target before its one value canonicalization.
    /// Protocol stamps can require a native layout that coercion would erase.
    pub(super) fn set_many_with<'key, I, K>(
        &mut self,
        values: I,
        check: impl Fn(&FixKey<'key>, &Field) -> Result<()>,
    ) -> Result<()>
    where
        I: IntoIterator<Item = (K, Scalar)>,
        K: Into<FixKey<'key>>,
    {
        let mut writes: Vec<Write> = Vec::new();
        for (key, value) in values {
            let key = key.into();
            stage(&mut writes, self.staged(&key, value, &check)?);
        }
        if writes.is_empty() {
            return Ok(());
        }
        self.write_all(writes)
    }

    /// Writes every value the field it reaches can hold, with one rebuild,
    /// answering how many landed.
    ///
    /// The lenient twin of [`Self::set_many`], for a pass whose answers are
    /// best effort: a value the target refuses - an identifier whose check
    /// digit does not close, a spelling its code set does not read - is
    /// silence rather than a refusal, exactly as one refused `set` is to the
    /// [enriching pass](super::FixCodec::enrich_message), and every other
    /// value lands as `set_many` lands it. Nothing else is lenient: the
    /// rebuild's refusal, which no single value causes, is still returned
    /// and leaves the message unchanged.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the written children do
    /// not make a root.
    pub(super) fn set_each<'key, I, K>(&mut self, values: I) -> Result<usize>
    where
        I: IntoIterator<Item = (K, Scalar)>,
        K: Into<FixKey<'key>>,
    {
        let mut writes: Vec<Write> = Vec::new();
        for (key, value) in values {
            let key = key.into();
            if let Ok(write) = self.staged(&key, value, &|_, _| Ok(())) {
                stage(&mut writes, write);
            }
        }
        let landed = writes.len();
        if landed > 0 {
            self.write_all(writes)?;
        }
        Ok(landed)
    }

    /// One value resolved and typed for the child its key reaches, exactly
    /// as [`Self::set`] resolves and types one, against the row as it stands.
    fn staged<'key>(
        &self,
        key: &FixKey<'key>,
        value: Scalar,
        check: &impl Fn(&FixKey<'key>, &Field) -> Result<()>,
    ) -> Result<Write> {
        let (at, mut field) = self.target(key)?;
        check(key, &field)?;
        if field
            .as_fix()
            .tag()?
            .is_some_and(super::identity::is_mandatory)
            && value.is_null()
        {
            return Err(super::identity::refused(
                field.name(),
                "a non-null mandatory value",
                "null",
            ));
        }
        let value = if value.is_null() {
            field.set_nullable(true);
            Scalar::Null
        } else {
            field.scalar(value)?
        };
        Ok(Write { at, field, value })
    }

    /// [`Self::set`], consuming the message.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::set`] returns.
    pub fn with_value<'key>(mut self, key: impl Into<FixKey<'key>>, value: Scalar) -> Result<Self> {
        self.set(key, value)?;
        Ok(self)
    }

    /// Removes the child a key reaches, answering the value it held.
    ///
    /// Row only, exactly as [`Self::set`] is: the entries are untouched.
    /// The key resolves as a lookup does - a tag reaches the child carrying
    /// it, else the one named as the dictionary names it or by the tag's
    /// decimal; a name reaches the child its canonical spelling, an exact
    /// match or the fold picks - and one that reaches nothing answers `None`
    /// and changes nothing. The children after the removed one move up, so
    /// the tag and group indexes are reread.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use yggdryl::{DataType, FixMsg, FixRegistry, Scalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut symbol = DataType::utf8().nullable_field("symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let registry = Arc::new(FixRegistry::from_fields([symbol.clone()])?);
    /// let root = DataType::from_fields([symbol, DataType::utf8().nullable_field("9999")])?
    ///     .required_field("D");
    /// let value = Scalar::from_record([
    ///     ("symbol", Scalar::from("AAPL")),
    ///     ("9999", Scalar::from("custom")),
    /// ])?;
    /// let mut msg = FixMsg::with_registry(registry, root, value)?;
    ///
    /// assert_eq!(msg.remove(55)?, Some(Scalar::from("AAPL")));
    /// assert_eq!(msg.get_by_tag(55), None);
    /// assert_eq!(msg.by_tag(9999)?, &Scalar::from("custom"), "the neighbour is still reached");
    /// assert_eq!(msg.remove("nosuchfield")?, None);
    /// # Ok(())
    /// # }
    /// ```
    /// # Errors
    /// Refuses removal of a mandatory field; any refusal leaves the message unchanged.
    pub fn remove<'key>(&mut self, key: impl Into<FixKey<'key>>) -> Result<Option<Scalar>> {
        let Some(at) = self.index_of_key(&key.into()) else {
            return Ok(None);
        };
        if super::identity::resolve_tag(&self.field.fields()[at], &self.registry)?
            .is_some_and(super::identity::is_mandatory)
        {
            return Err(super::identity::refused(
                self.field.fields()[at].name(),
                "a retained mandatory field",
                "removal",
            ));
        }
        let mut members = self.field.fields().to_vec();
        let mut values = self
            .value
            .as_sequence()
            .ok_or_else(|| {
                super::identity::refused(self.field.name(), "a canonical row", self.value.kind())
            })?
            .to_vec();
        if at >= members.len() || at >= values.len() {
            return Ok(None);
        }
        members.remove(at);
        let removed = values.remove(at);
        // Everything that can refuse is asked before anything is written, so
        // a refusal leaves the message as it was.
        let dtype = DataType::from_fields(members)?;
        let field = self.rerooted(dtype);
        let plan = super::schema::column_plan_of(&field, &self.registry)?;
        let hard =
            plan.identity
                .finalize(&field, &mut values, super::identity::Assertions::default())?;
        self.field = field;
        self.value = Scalar::from_sequence(values);
        self.tags = tag_positions(&plan);
        self.groups = group_positions(&self.field);
        self.named = OnceLock::new();
        self.publish_hard(hard);
        Ok(Some(removed))
    }

    /// The child a key reaches and the field it is written under: an
    /// existing child's position where one is reached, and the field the
    /// registry resolves the key to, else the reached child's own.
    ///
    /// The field is owned because a written child is the registry's field
    /// as this message states it, and the row's own child otherwise.
    fn target(&self, key: &FixKey<'_>) -> Result<(Option<usize>, Field)> {
        if let Some(at) = self.index_of_key(key) {
            let field = &self.field.fields()[at];
            if let Some(tag) = super::identity::resolve_tag(field, &self.registry)?
                .filter(|tag| super::identity::is_mandatory(*tag))
            {
                let mut field = field.clone();
                field.as_fix_mut().set_tag(tag)?;
                return Ok((Some(at), field));
            }
        }
        match *key {
            FixKey::Tag(tag) => {
                let at = self.reached_by_tag(tag);
                match self.known_by_tag(tag) {
                    Some(known) => Ok((at, stated(known))),
                    None => {
                        let field = at
                            .and_then(|at| self.field.get_field_at(at))
                            .cloned()
                            .unwrap_or_else(|| {
                                DataType::utf8().nullable_field(format_smolstr!("{tag}"))
                            });
                        Ok((at, field))
                    }
                }
            }
            FixKey::Id(id) => {
                let known = self
                    .registry
                    .get_field_by_id(id)
                    .ok_or_else(|| absent(key))?;
                let at = self.index_of_name(known.name()).or_else(|| {
                    self.registry
                        .identity_of(known)
                        .and_then(|(tag, _)| self.index_of_tag(tag))
                });
                Ok((at, stated(known)))
            }
            FixKey::Name(name) => match self.known_by_name(name) {
                Some(known) => {
                    let by_tag = known
                        .as_fix()
                        .tag()
                        .ok()
                        .flatten()
                        .and_then(|tag| self.index_of_tag(tag));
                    Ok((
                        by_tag.or_else(|| self.child_index(&self.field, name)),
                        stated(known),
                    ))
                }
                None => {
                    let at = self
                        .child_index(&self.field, name)
                        .ok_or_else(|| absent(key))?;
                    let field = self
                        .field
                        .get_field_at(at)
                        .cloned()
                        .ok_or_else(|| absent(key))?;
                    Ok((Some(at), field))
                }
            },
        }
    }

    /// The position of the child a key reaches, as a lookup reaches it.
    fn index_of_key(&self, key: &FixKey<'_>) -> Option<usize> {
        match *key {
            FixKey::Tag(tag) => self.reached_by_tag(tag),
            FixKey::Id(id) => {
                let known = self.registry.get_field_by_id(id)?;
                self.field
                    .index_of(known.name())
                    .or_else(|| self.mandatory_index(known))
            }
            FixKey::Name(name) => self.child_index(&self.field, name).or_else(|| {
                self.known_by_name(name)
                    .and_then(|known| self.mandatory_index(known))
            }),
        }
    }

    fn mandatory_index(&self, known: &Field) -> Option<usize> {
        let (tag, _) = self.registry.identity_of(known)?;
        super::identity::is_mandatory(tag)
            .then(|| self.index_of_tag(tag))
            .flatten()
    }

    /// Lands the planned writes: each replaced at its position, appended
    /// when it has none.
    ///
    /// The whole row is rebuilt once, because a Struct's children and a
    /// row's values are both one shared allocation; everything that can
    /// refuse is asked before anything is stored, so a refusal leaves the
    /// message as it was. The tag and group indexes follow the children that
    /// changed: the entry a replaced child held is retired and the one the
    /// written child carries admitted, and nothing else is reread. The name
    /// table is derived from the children, so it is carried through a write
    /// that leaves it true - an append, or a replacement under the child's
    /// own name - and dropped only where a rename makes it wrong.
    fn write_all(&mut self, writes: Vec<Write>) -> Result<()> {
        let mut assertions = super::identity::Assertions::default();
        for write in &writes {
            let tag = write.field.as_fix().tag()?;
            assertions.msghash |= tag == Some(super::MSGHASH_TAG_NAME.0);
            assertions.persistent |= tag == Some(super::MSGPHASH_TAG_NAME.0);
        }
        let (field, mut values, indexed) = stage_writes(&self.field, &self.value, writes)?;
        let plan = super::schema::column_plan_of(&field, &self.registry)?;
        plan.identity.require_bundle(&field)?;
        let hard = plan.identity.finalize(&field, &mut values, assertions)?;
        self.field = field;
        self.value = Scalar::from_sequence(values);
        // The name table is derived from the children, and most writes leave
        // it true: a replacement under the child's own name moves nothing,
        // and an append only adds an entry. Enrich is the case that matters -
        // it alternates a miss, which builds this table over every column of
        // a wide row, with a write that would have thrown it away - and every
        // one of its writes is an append or a same-name replacement.
        let mut named = self.named.take();
        for change in indexed {
            if change.renamed {
                named = None;
            } else if change.appended {
                if let (Some(named), Some(child)) =
                    (named.as_mut(), self.field.fields().get(change.index))
                {
                    named
                        .entry(SmolStr::new(child.name()))
                        .or_insert(change.index);
                }
            }
            change.apply(&mut self.tags, Some(&mut self.groups));
        }
        self.named = OnceLock::new();
        if let Some(named) = named {
            let _ = self.named.set(named);
        }
        self.publish_hard(hard);
        Ok(())
    }

    fn publish_hard(&mut self, hard: super::identity::Hard) {
        self.updatedat = hard.updatedat;
        self.createdat = hard.createdat;
        self.msghash = hard.msghash;
        self.msgphash = hard.msgphash;
    }

    /// The root over other children: its name, nullability and metadata,
    /// the Struct `dtype` under them.
    ///
    /// `DataType::from_fields` already refused a duplicate name and checked
    /// every child, and that is the whole of what the root's setter would
    /// check again before comparing the old children with the new one by
    /// one - so the root is rebuilt around the proven datatype instead, which
    /// is a per-message cost on every stream that stamps or fills a row.
    fn rerooted(&self, dtype: DataType) -> Field {
        Field::new_with_metadata(
            self.field.name(),
            dtype,
            self.field.is_nullable(),
            self.field.metadata.clone(),
        )
    }

    /// Returns what this message says about itself that does not add up.
    ///
    /// Derived by comparing the row against the entries, never stored, so
    /// there is nothing to keep in step and a caller who never asks pays
    /// nothing. A message built from a schema and a value has no entries and
    /// so reports nothing: there is no arrival record to disagree with.
    #[must_use]
    pub fn anomalies(&self) -> FixAnomalies<'_> {
        FixAnomalies::new(self)
    }

    /// Re-emits this message on the wire, separated by `separator`.
    ///
    /// The bytes come from the entries rather than from the row, because the
    /// row is a lossy interpretation by construction: a translated `4` cannot
    /// say whether the wire carried `4` or its symbolic spelling. A message
    /// built from a schema and a value carries no entries and emits nothing.
    #[must_use]
    pub fn into_bytes(&self, separator: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        emit_bytes(&self.entries, separator, &mut bytes);
        bytes
    }

    /// The same, as text.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when a value is not printable, which
    /// is why the byte emit exists: a message carrying a `data` field cannot
    /// round-trip through text.
    pub fn into_text(&self, separator: char) -> Result<String> {
        let mut text = String::new();
        emit_text(&self.entries, separator, &mut text)?;
        Ok(text)
    }

    /// The version this message was read at.
    ///
    /// The crate's own `version` child where a read stamped one - the codec's
    /// target, which is what every field and every code in the row resolved
    /// against - and `BeginString(8)` otherwise, which is what a message
    /// converted from a schema and a value says about itself.
    ///
    /// Derived rather than stored either way, so a converted one cannot lie.
    /// The two answers differ exactly where a session mislabels itself or
    /// carries a row written to a later FIX than it speaks, which is what the
    /// crate keeps a column for.
    #[must_use]
    pub fn version(&self) -> Option<Version> {
        if let Some(held) = self
            .get_by_tag(super::VERSION_TAG_NAME.0)
            .and_then(Scalar::as_str)
            .and_then(|held| held.parse().ok())
        {
            return Some(held);
        }
        let begin = self.get_by_tag(8).and_then(Scalar::as_str)?;
        begin
            .strip_prefix("FIX.")
            .and_then(|rest| rest.parse().ok())
    }

    /// Returns the registry this message resolves against.
    pub const fn registry(&self) -> &Arc<FixRegistry> {
        &self.registry
    }

    /// Returns the root Struct field: the message's resolved schema.
    pub const fn as_field(&self) -> &Field {
        &self.field
    }

    /// Returns the ordered row value.
    pub const fn as_value(&self) -> &Scalar {
        &self.value
    }

    /// Returns the value of the root child an identifier names.
    ///
    /// An identifier is exact: it names one field, and the child is the one
    /// that field's name reaches.
    pub fn get_by_id(&self, id: FixId) -> Option<&Scalar> {
        let index = self.index_of_key(&FixKey::Id(id))?;
        self.value.get(index)
    }

    /// Returns the value of the root child an identifier names, raising
    /// absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the identifier.
    pub fn by_id(&self, id: FixId) -> Result<&Scalar> {
        self.get_by_id(id).ok_or_else(|| absent(FixKey::Id(id)))
    }

    /// Returns the value of the root child a tag names.
    ///
    /// The registry resolves the tag to its canonical name, and that name
    /// picks the root child, so a venue field and `MsgType` are both
    /// reachable from a venue message. A tag the dictionary does not answer
    /// is looked for under its decimal rendering, so an unknown tag a
    /// transcriber retained is still reachable.
    pub fn get_by_tag(&self, tag: i32) -> Option<&Scalar> {
        self.clock_by_tag(tag)
            .or_else(|| self.value.get(self.reached_by_tag(tag)?))
    }

    /// The value a tag names by the index alone: one of the four values the
    /// message computes about itself, else the child declaring the tag, and
    /// never the two fallbacks of [`Self::get_by_tag`], which end in a name
    /// table built on the first miss. The [enriching pass](super::enrich)
    /// gathers its working row through this, a column per tag it reads, so
    /// the columns a message lacks - most of them - cost a binary search
    /// each and never the table.
    pub(super) fn indexed_by_tag(&self, tag: i32) -> Option<&Scalar> {
        self.clock_by_tag(tag)
            .or_else(|| self.value.get(self.index_of_tag(tag)?))
    }

    /// The four values the message holds beside its row rather than in it,
    /// where `tag` names one of them.
    fn clock_by_tag(&self, tag: i32) -> Option<&Scalar> {
        if tag == super::UPDATEDAT_TAG_NAME.0 {
            Some(self.updatedat())
        } else if tag == super::CREATEDAT_TAG_NAME.0 {
            Some(self.createdat())
        } else if tag == super::MSGHASH_TAG_NAME.0 {
            Some(self.msghash())
        } else if tag == super::MSGPHASH_TAG_NAME.0 {
            Some(self.msgphash())
        } else {
            None
        }
    }

    /// The child a tag reaches: the one carrying the tag, by one hash-free
    /// binary search over the index resolved at construction, else the one
    /// either fallback names. A field a dictionary never explained carries
    /// no tag and falls through to the fallbacks, where the dictionary is
    /// one probe.
    fn reached_by_tag(&self, tag: i32) -> Option<usize> {
        if let Some(index) = self.index_of_tag(tag) {
            return Some(index);
        }
        self.fallback_index(tag)
    }

    /// The child a tag reaches past the index: the one named as the
    /// dictionary names the tag, else the one named by the tag's decimal
    /// spelling.
    fn fallback_index(&self, tag: i32) -> Option<usize> {
        let named = self.named.get_or_init(|| {
            let mut named = HashMap::new();
            for (index, child) in self.field.fields().iter().enumerate() {
                named.entry(SmolStr::new(child.name())).or_insert(index);
            }
            named
        });
        match self.known_by_tag(tag) {
            Some(known) => named.get(known.name()).copied(),
            // Eleven bytes hold every `i32`, sign included, so the rendering
            // stays inline and a miss allocates nothing.
            None => named.get(format_smolstr!("{tag}").as_str()).copied(),
        }
    }

    /// The root child carrying one tag, by that child's own declaration.
    pub(super) fn index_of_tag(&self, tag: i32) -> Option<usize> {
        // The first child carrying the tag, in the row's own order: a row
        // may hold two children on one tag where the dictionary holds two
        // fields on it, and the index is sorted by tag then position, so the
        // partition point is the earliest.
        let at = self.tags.partition_point(|(held, _)| *held < tag);
        self.tags
            .get(at)
            .filter(|(held, _)| *held == tag)
            .map(|(_, index)| *index)
    }

    /// A tag identifies a member only when exactly one root child carries it.
    pub(super) fn unique_index_of_tag(&self, tag: i32) -> Option<usize> {
        let at = self.tags.partition_point(|(held, _)| *held < tag);
        let (held, index) = self.tags.get(at)?;
        if *held != tag || self.tags.get(at + 1).is_some_and(|(next, _)| *next == tag) {
            return None;
        }
        Some(*index)
    }

    pub(super) fn index_of_group(&self, counter: i32) -> Option<usize> {
        let index = self
            .groups
            .binary_search_by_key(&counter, |(tag, _)| *tag)
            .ok()?;
        if index > 0 && self.groups[index - 1].0 == counter
            || self
                .groups
                .get(index + 1)
                .is_some_and(|(tag, _)| *tag == counter)
        {
            return None;
        }
        Some(self.groups[index].1)
    }

    /// Returns the value of the root child a tag names, raising absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the tag.
    pub fn by_tag(&self, tag: i32) -> Result<&Scalar> {
        self.get_by_tag(tag).ok_or_else(|| absent(FixKey::Tag(tag)))
    }

    /// Returns the value of the root child a name reaches.
    ///
    /// The name folds through the registry to its canonical spelling, and
    /// an exact root-child match is the fallback when the registry does not
    /// know it.
    pub fn get_by_name(&self, name: &str) -> Option<&Scalar> {
        self.value.get(self.index_of_key(&FixKey::Name(name))?)
    }

    /// Returns the value of the root child a name reaches, raising absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the name.
    pub fn by_name(&self, name: &str) -> Result<&Scalar> {
        self.get_by_name(name)
            .ok_or_else(|| absent(FixKey::Name(name)))
    }

    /// Returns the value a resolved path reaches.
    ///
    /// The path is the crate's one grammar, already parsed: nothing here
    /// splits a string, so a run addressing the same member a million times
    /// resolves the path once. A named segment resolves as
    /// [`Self::get_by_name`] does - the registry's canonical spelling first,
    /// then an exact match - and an indexed segment takes one occurrence of
    /// the List a
    /// repeating group is, which is what reaching a member needs:
    /// `Parties[0].PartyID`.
    ///
    /// A bare decimal is a name and not a position, exactly as it is one
    /// layer down where a text line's entry keyed `55` is reached by the path
    /// `55`. A path of one named segment is [`Self::get_by_name`].
    pub fn get_by_path(&self, path: &FieldPath) -> Option<&Scalar> {
        let mut segments = path.segments().iter();
        let first = segments.next()?;
        let name = first.as_name()?;
        let index = self.index_of_key(&FixKey::Name(name))?;
        let mut field = self.field.fields().get(index)?;
        let mut value = self.value.get(index)?;
        for segment in segments {
            (field, value) = self.descend(field, value, segment)?;
        }
        Some(value)
    }

    /// Returns the value a resolved path reaches, raising absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the path.
    pub fn by_path(&self, path: &FieldPath) -> Result<&Scalar> {
        self.get_by_path(path)
            .ok_or_else(|| absent(format_args!("path {path}")))
    }

    /// Returns the value a tag, an identifier, a name or a path reaches.
    ///
    /// Matches the key once and redirects: a tag to [`Self::get_by_tag`], an
    /// identifier to [`Self::get_by_id`], a name to [`Self::get_by_name`] -
    /// and, only where that reached nothing and the key spells more than one
    /// segment, to [`Self::get_by_path`] with the path that key states. The
    /// reading a caller writes down is resolved here, which is why the door
    /// that takes one already resolved is the one a loop should use.
    pub fn get<'key>(&self, key: impl Into<FixKey<'key>>) -> Option<&Scalar> {
        match key.into() {
            FixKey::Tag(tag) => self.get_by_tag(tag),
            FixKey::Id(id) => self.get_by_id(id),
            FixKey::Name(name) => self.named_or_path(name),
        }
    }

    /// Returns the value a tag, an identifier or a name reaches, raising
    /// absence.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::by_tag`], [`Self::by_id`] or
    /// [`Self::by_path`] raises, whichever the key selects.
    pub fn value<'key>(&self, key: impl Into<FixKey<'key>>) -> Result<&Scalar> {
        match key.into() {
            FixKey::Tag(tag) => self.by_tag(tag),
            FixKey::Id(id) => self.by_id(id),
            FixKey::Name(name) => {
                if let Some(value) = self.get_by_name(name) {
                    return Ok(value);
                }
                // Where the key spells a path, the path door is the reading
                // that was attempted, so its absence is the one to raise.
                match FieldPath::from_str(name) {
                    Ok(path) if path.segments().len() > 1 => self.by_path(&path),
                    _ => Err(absent(FixKey::Name(name))),
                }
            }
        }
    }

    /// The field a bare tag names: the tag's first holder in the registry.
    pub(super) fn known_by_tag(&self, tag: i32) -> Option<&Field> {
        self.registry.get_field_by_tag(tag).or_else(|| {
            self.registry
                .get_group_by_tag(tag)
                .filter(|group| matches!(group.dtype(), DataType::Map(_)))
        })
    }

    /// The field a bare name reaches, canonical spelling or alias.
    pub(super) fn known_by_name(&self, name: &str) -> Option<&Field> {
        self.registry.get_message_field_by_name(name)
    }

    /// The position of the child `name` reaches under `parent`: the
    /// registry's canonical spelling first, then an exact match.
    fn child_index(&self, parent: &Field, name: &str) -> Option<usize> {
        self.known_by_name(name)
            .and_then(|known| parent.index_of(known.name()))
            .or_else(|| named_index(parent, name))
    }

    /// The position of the root child `name` spells, with no dictionary
    /// consulted: an exact match, else the one child the fold reaches.
    ///
    /// Where a column that carries no tag is matched to a child: a capture's
    /// own column is named by nobody's dictionary, so the spelling alone
    /// decides.
    pub(super) fn index_of_name(&self, name: &str) -> Option<usize> {
        named_index(&self.field, name)
    }

    /// One key read as a name, and as the path it spells where it spells one.
    ///
    /// A name costs no parse, which is what nearly every key is. A key
    /// holding more than one segment is a reading a caller wrote down, and
    /// reading it is this door's job rather than a second door's.
    fn named_or_path(&self, key: &str) -> Option<&Scalar> {
        if let Some(value) = self.get_by_name(key) {
            return Some(value);
        }
        let path = FieldPath::from_str(key).ok()?;
        (path.segments().len() > 1)
            .then(|| self.get_by_path(&path))
            .flatten()
    }

    /// The position one named segment reaches under `parent`.
    ///
    /// Where a segment's name is resolved, and the only place FIX's own
    /// naming enters a path: the grammar states which child is wanted and
    /// this states which child that is. An indexed segment names no child, so
    /// it reaches nothing here - a position is answered by [`Self::descend`],
    /// which knows whether it is standing on a list.
    fn segment_index(&self, parent: &Field, segment: &FieldSegment) -> Option<usize> {
        self.child_index(parent, segment.as_name()?)
    }

    /// One step of a path: into a Struct child by name, or into one
    /// occupancy of the List a repeating group is.
    fn descend<'value>(
        &self,
        field: &'value Field,
        value: &'value Scalar,
        segment: &FieldSegment,
    ) -> Option<(&'value Field, &'value Scalar)> {
        match field.dtype() {
            DataType::Struct(_) => {
                let index = self.segment_index(field, segment)?;
                Some((field.fields().get(index)?, value.get(index)?))
            }
            DataType::List(item)
            | DataType::LargeList(item)
            | DataType::FixedSizeList(item, _)
            | DataType::ListView(item)
            | DataType::LargeListView(item) => {
                let FieldSegment::Index(position) = segment else {
                    return None;
                };
                // A negative index counts back from the end, as the grammar
                // states it: the last occurrence is `[-1]` whatever a message
                // happened to carry.
                let held = value.as_sequence()?;
                let at = if *position < 0 {
                    held.len().checked_sub(position.unsigned_abs() as usize)?
                } else {
                    usize::try_from(*position).ok()?
                };
                Some((item.as_ref(), value.get(at)?))
            }
            DataType::Map(map) => {
                let FieldSegment::Key(key) = segment else {
                    return None;
                };
                Some((map.entries().fields().get(1)?, value.get_key(key.value())?))
            }
            _ => None,
        }
    }
}

/// Report that nothing in the message is reached by `what`.
fn absent(what: impl fmt::Display) -> Error {
    Error::absent("fix value", what)
}

/// The position of the child `name` spells under `parent`: an exact match,
/// else the one child the fold reaches - two children one fold reaches name
/// neither.
///
/// One pass answers both readings: a write that appends misses every child
/// under both, and a message stamped per row pays that pass once per stamp.
fn named_index(parent: &Field, name: &str) -> Option<usize> {
    let mut folded = None;
    let mut ambiguous = false;
    for (index, field) in parent.fields().iter().enumerate() {
        let held = field.name();
        if held == name {
            return Some(index);
        }
        if crate::types::folds_equal(held, name) {
            ambiguous |= folded.is_some();
            folded = Some(index);
        }
    }
    if ambiguous { None } else { folded }
}

/// Forgets the entry a child at `at` held in a sorted position index.
fn retire(index: &mut Vec<(i32, usize)>, key: Option<i32>, at: usize) {
    if let Some(key) = key {
        if let Ok(position) = index.binary_search(&(key, at)) {
            index.remove(position);
        }
    }
}

/// Records the entry the child at `at` carries in a sorted position index.
fn admit(index: &mut Vec<(i32, usize)>, key: Option<i32>, at: usize) {
    if let Some(key) = key {
        if let Err(position) = index.binary_search(&(key, at)) {
            index.insert(position, (key, at));
        }
    }
}

impl From<FixMsg> for Result<FixMsg> {
    fn from(message: FixMsg) -> Self {
        Ok(message)
    }
}

impl Clone for FixMsg {
    /// The message, without the name table: a cache derived from the
    /// children, rebuilt by the clone on its own first miss rather than
    /// copied - a stream clones a message far more often than it projects
    /// one.
    fn clone(&self) -> Self {
        Self {
            registry: Arc::clone(&self.registry),
            entries: self.entries.clone(),
            tags: self.tags.clone(),
            named: OnceLock::new(),
            groups: self.groups.clone(),
            field: self.field.clone(),
            value: self.value.clone(),
            updatedat: self.updatedat.clone(),
            createdat: self.createdat.clone(),
            msghash: self.msghash.clone(),
            msgphash: self.msgphash.clone(),
        }
    }
}

impl fmt::Debug for FixMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FixMsg")
            .field("field", &self.field)
            .field("value", &self.value)
            .finish_non_exhaustive()
    }
}

impl PartialEq for FixMsg {
    /// Two messages are equal when they carry the same schema and value
    /// against the same registry - the same `Arc`, or registries that hold
    /// the same fields.
    fn eq(&self, other: &Self) -> bool {
        self.field == other.field
            && self.value == other.value
            && (Arc::ptr_eq(&self.registry, &other.registry) || self.registry == other.registry)
    }
}

impl Eq for FixMsg {}

impl Hash for FixMsg {
    /// Hashes the schema and the value; the registry is part of equality but
    /// not of the hash, which keeps equal messages hashing alike.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.field.hash(state);
        self.value.hash(state);
    }
}
