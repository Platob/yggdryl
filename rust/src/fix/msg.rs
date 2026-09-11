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
/// let mut symbol = DataType::Utf8.required_field("Symbol");
/// symbol.as_fix_mut().set_tag(55)?;
/// symbol.as_fix_mut().set_aliases(["Ticker"])?;
/// let mut qty = DataType::Int64.required_field("OrderQty");
/// qty.as_fix_mut().set_tag(38)?;
/// let registry = Arc::new(FixRegistry::from_fields([symbol.clone(), qty.clone()])?);
///
/// let root = DataType::from_fields([symbol, qty, DataType::Utf8.nullable_field("9999")])?
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
///
/// // Both halves serialize through the paths every field and value share.
/// let schema = root.clone().into_json()?;
/// let text = into_json_scalar(msg.as_value())?;
/// let read = from_json_scalar_with_field(&text, &root)?;
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
    /// An index, not a second fact: it is derived from `field` alone and both
    /// are replaced together.
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
}

/// One child a write lands: replaced at `at`, appended when there is none,
/// its field and value already resolved and typed.
struct Write {
    at: Option<usize>,
    field: Field,
    value: Scalar,
}

/// What one landed write does to the tag and group indexes.
struct Indexed {
    retired_tag: Option<i32>,
    retired_counter: Option<i32>,
    tag: Option<i32>,
    counter: Option<i32>,
    index: usize,
}

/// Each child's declared tag beside its position, sorted for a binary search.
fn tag_positions(field: &Field) -> Vec<(i32, usize)> {
    let Some(children) = field.dtype().as_fields() else {
        return Vec::new();
    };
    let mut held: Vec<(i32, usize)> = children
        .iter()
        .enumerate()
        .filter_map(|(index, child)| Some((child.as_fix().tag().ok().flatten()?, index)))
        .collect();
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
        crate::stable_hash_of(self)
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
        let value = field.canonicalize_value(value)?;
        let tags = tag_positions(&field);
        Ok(Self::resolved(registry, field, value, Vec::new(), tags))
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
    pub(super) fn from_built(registry: Arc<FixRegistry>, built: super::build::Built) -> Self {
        let super::build::Built {
            field,
            value,
            entries,
            tags,
        } = built;
        Self::resolved(registry, field, value, entries, tags)
    }

    /// One message from parts already proven: the group index is read off
    /// the root, and nothing else is derived yet.
    fn resolved(
        registry: Arc<FixRegistry>,
        field: Field,
        value: Scalar,
        entries: Vec<FixEntry>,
        tags: Vec<(i32, usize)>,
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
        }
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
    /// A `Null` is stored as a stated null: the child stays, nullable,
    /// holding nothing.
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
    /// let mut symbol = DataType::Utf8.nullable_field("symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let registry = Arc::new(FixRegistry::from_fields([qty, symbol])?);
    ///
    /// let root = DataType::from_fields([DataType::Utf8.required_field("symbol")])?
    ///     .required_field("D");
    /// let value = Scalar::from_record([("symbol", Scalar::from("AAPL"))])?;
    /// let mut msg = FixMsg::with_registry(registry, root, value)?;
    ///
    /// // Appended under the dictionary's field, typed by it and found by tag.
    /// msg.set(38, Scalar::from(100_i64))?;
    /// assert_eq!(msg.by_tag(38)?, &Scalar::from(100_i64));
    /// assert_eq!(msg.as_field().fields()[1].name(), "orderqty");
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
    /// assert_eq!(msg.as_field().fields().len(), 3);
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
    /// let mut symbol = DataType::Utf8.nullable_field("symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let registry = Arc::new(FixRegistry::from_fields([qty, symbol])?);
    /// let root = DataType::from_fields([DataType::Utf8.required_field("symbol")])?
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
    /// assert_eq!(msg.as_field().fields().len(), 2, "one child per field");
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
        let mut writes: Vec<Write> = Vec::new();
        for (key, value) in values {
            let key = key.into();
            let (at, mut field) = self.target(&key)?;
            let value = if value.is_null() {
                field.set_nullable(true);
                Scalar::Null
            } else {
                field.scalar(value)?
            };
            // The later of two writes to one child stands, whether both
            // reached it or both would append it.
            let pending = writes.iter().position(|held| match (held.at, at) {
                (Some(held), Some(at)) => held == at,
                (None, None) => held.field.name() == field.name(),
                _ => false,
            });
            let write = Write { at, field, value };
            match pending {
                Some(pending) => writes[pending] = write,
                None => writes.push(write),
            }
        }
        if writes.is_empty() {
            return Ok(());
        }
        self.write_all(writes)
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
    /// let mut symbol = DataType::Utf8.nullable_field("symbol");
    /// symbol.as_fix_mut().set_tag(55)?;
    /// let registry = Arc::new(FixRegistry::from_fields([symbol.clone()])?);
    /// let root = DataType::from_fields([symbol, DataType::Utf8.nullable_field("9999")])?
    ///     .required_field("D");
    /// let value = Scalar::from_record([
    ///     ("symbol", Scalar::from("AAPL")),
    ///     ("9999", Scalar::from("custom")),
    /// ])?;
    /// let mut msg = FixMsg::with_registry(registry, root, value)?;
    ///
    /// assert_eq!(msg.remove(55), Some(Scalar::from("AAPL")));
    /// assert_eq!(msg.get_by_tag(55), None);
    /// assert_eq!(msg.by_tag(9999)?, &Scalar::from("custom"), "the neighbour is still reached");
    /// assert_eq!(msg.remove("nosuchfield"), None);
    /// # Ok(())
    /// # }
    /// ```
    pub fn remove<'key>(&mut self, key: impl Into<FixKey<'key>>) -> Option<Scalar> {
        let at = self.index_of_key(&key.into())?;
        let mut members = self.field.fields().to_vec();
        let mut values = self.value.as_sequence()?.to_vec();
        if at >= members.len() || at >= values.len() {
            return None;
        }
        members.remove(at);
        let removed = values.remove(at);
        // Everything that can refuse is asked before anything is written, so
        // a refusal leaves the message as it was.
        let dtype = DataType::from_fields(members).ok()?;
        self.field = self.rerooted(dtype);
        self.value = Scalar::from_sequence(values);
        self.tags = tag_positions(&self.field);
        self.groups = group_positions(&self.field);
        self.named = OnceLock::new();
        Some(removed)
    }

    /// The child a key reaches and the field it is written under: an
    /// existing child's position where one is reached, and the field the
    /// registry resolves the key to, else the reached child's own.
    ///
    /// The field is owned because a written child is the registry's field
    /// as this message states it, and the row's own child otherwise.
    fn target(&self, key: &FixKey<'_>) -> Result<(Option<usize>, Field)> {
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
                                DataType::Utf8.nullable_field(format_smolstr!("{tag}"))
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
                self.field.index_of(known.name())
            }
            FixKey::Name(name) => self.child_index(&self.field, name),
        }
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
    /// table is derived from the children and is dropped, to be derived
    /// again on the next miss.
    fn write_all(&mut self, writes: Vec<Write>) -> Result<()> {
        let mut members = self.field.fields().to_vec();
        let mut values = self
            .value
            .as_sequence()
            .map(<[Scalar]>::to_vec)
            .unwrap_or_default();
        members.reserve(writes.len());
        values.reserve(writes.len());
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
            let held = replaced.as_ref().map(Field::as_fix);
            let written = members[index].as_fix();
            indexed.push(Indexed {
                retired_tag: held.as_ref().and_then(|held| held.tag().ok().flatten()),
                retired_counter: held.as_ref().and_then(|held| held.counter().ok().flatten()),
                tag: written.tag().ok().flatten(),
                counter: written.counter().ok().flatten(),
                index,
            });
        }
        let dtype = DataType::from_fields(members)?;
        self.field = self.rerooted(dtype);
        self.value = Scalar::from_sequence(values);
        for change in indexed {
            retire(&mut self.tags, change.retired_tag, change.index);
            retire(&mut self.groups, change.retired_counter, change.index);
            admit(&mut self.tags, change.tag, change.index);
            admit(&mut self.groups, change.counter, change.index);
        }
        self.named = OnceLock::new();
        Ok(())
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

    /// This message restated at its registry's newest version.
    ///
    /// Row only: the entries are carried through untouched, so
    /// [`Self::into_bytes`] still re-emits the received line byte for byte,
    /// and a second call answers an equal message. Every child the registry
    /// knows - by its tag, by its name or alias, or by the decimal tag its
    /// name spells - is re-expressed under the registry's field with its
    /// value re-typed, children reaching one field are merged into the most
    /// complete one, every `fix:replacements` rule the dictionary states for
    /// a held value fills the fields that stand in for it, and the crate's
    /// `version` child takes [`FixRegistry::newest`]. A child no dictionary
    /// explains, a stated value that disagrees with the one kept, and a
    /// registry with no dated field each leave things exactly as they are.
    ///
    /// ```
    /// use std::sync::Arc;
    ///
    /// use yggdryl::fix::{FixLineageEntry, FixPedigree};
    /// use yggdryl::{DataType, FixMsg, FixRegistry, Scalar, Version};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// // LastQty(32) was LastShares, an integer, until FIX 4.3.
    /// let mut qty = DataType::Float64.nullable_field("lastqty");
    /// qty.as_fix_mut().set_tag(32)?;
    /// qty.as_fix_mut().set_lineage(&[
    ///     FixLineageEntry::new(FixPedigree::new("4.0".parse::<Version>()?, None))
    ///         .with_name("lastshares")
    ///         .with_dtype("int32"),
    ///     FixLineageEntry::new(FixPedigree::new("4.3".parse::<Version>()?, None))
    ///         .with_name("lastqty")
    ///         .with_dtype("float64"),
    /// ])?;
    /// let registry = Arc::new(FixRegistry::from_fields([qty])?);
    ///
    /// let root = DataType::from_fields([
    ///     DataType::Int64.required_field("LastShares"),
    ///     DataType::Utf8.nullable_field("9999"),
    /// ])?
    /// .required_field("8");
    /// let value = Scalar::from_record([
    ///     ("LastShares", Scalar::from(100)),
    ///     ("9999", Scalar::from("custom")),
    /// ])?;
    /// let latest = FixMsg::with_registry(registry, root, value)?.into_latest()?;
    ///
    /// assert_eq!(latest.as_field().fields()[0].name(), "lastqty");
    /// assert_eq!(latest.by_tag(32)?, &Scalar::from(100.0_f64));
    /// assert_eq!(latest.by_tag(9999)?, &Scalar::from("custom"), "an unknown tag is kept");
    /// assert_eq!(latest.version(), Some("4.3".parse::<Version>()?));
    /// assert_eq!(latest.clone().into_latest()?, latest, "a second pass changes nothing");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the restated children do
    /// not make a root, or the refusal [`Self::with_registry`] raises.
    pub fn into_latest(self) -> Result<Self> {
        super::latest::restate(self)
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
        let known = self.registry.get_field_by_id(id)?;
        let index = self.field.index_of(known.name())?;
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
        self.value.get(self.reached_by_tag(tag)?)
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
        let found = self.tags.binary_search_by_key(&tag, |(held, _)| *held);
        found.ok().map(|at| self.tags[at].1)
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
        self.value.get(self.child_index(&self.field, name)?)
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
        let index = self.segment_index(&self.field, first)?;
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
        self.registry.get_field_by_tag(tag)
    }

    /// The field a bare name reaches, canonical spelling or alias.
    pub(super) fn known_by_name(&self, name: &str) -> Option<&Field> {
        self.registry.get_field_by_name(name)
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
        match segment {
            FieldSegment::Field(name) => self.child_index(parent, name),
            FieldSegment::Key(key) => self.child_index(parent, key.value().as_str()?),
            FieldSegment::Index(_) => None,
        }
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
