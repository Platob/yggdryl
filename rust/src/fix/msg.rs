//! A FIX message: a value plus the registry that types it.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use smol_str::SmolStr;

use super::anomaly::FixAnomalies;
use super::entry::FixEntry;
use super::{FixBranch, FixId, FixKey, FixRegistry};
use crate::{DataType, Error, Field, Result, Scalar, Version};

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
/// A message has a branch, and it is derived rather than declared: it is
/// the root field's own `fix:branch`, resolved once at construction, so
/// nothing can disagree with it. A bare tag or name then resolves in a fixed
/// two-step tier - this message's branch first, when the identifier that
/// would name is legal at all, then the standard one - because a message
/// transcribed against a venue dictionary names its own fields by the
/// venue's spellings while still carrying `MsgType` and every other
/// specification field.
///
/// A value is reached by tag, by identifier, by name, or by path, each
/// answering `Option<&Scalar>` with a failing twin; resolution goes through
/// the linked registry, never through a private copy of its rules. An unknown
/// tag is retained rather than dropped: it is looked for under its rendered
/// decimal name, which is where a transcriber keeps a tag no dictionary
/// explains.
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
#[derive(Clone)]
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
    /// The root field's branch, enriched from the linked registry when that
    /// dictionary declares a version or session defaults. Resolving it once
    /// keeps every message lookup allocation-free.
    branch: FixBranch,
    /// Each root child's tag beside its position, in tag order.
    ///
    /// Resolved once for the same reason [`Self::branch`] is: reading a tag
    /// out of a child's metadata is a formatted key and a map lookup, and
    /// scanning the children per lookup pays it once per child. Every facet
    /// a lift answers is a tag lookup, so a row of twenty facets over a
    /// message of twenty children was four hundred of them.
    ///
    /// An index, not a second fact: it is derived from `field` alone and both
    /// are replaced together.
    tags: Vec<(i32, usize)>,
    field: Field,
    value: Scalar,
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
        if entry.value().chars().any(char::is_control) {
            return Err(Error::InvalidRecord {
                path: SmolStr::new(entry.key()),
                reason: "expected a printable value, got a control byte".into(),
            });
        }
        text.push_str(entry.key());
        text.push('=');
        text.push_str(entry.value());
        text.push(separator);
        emit_text(entry.children(), separator, text)?;
    }
    Ok(())
}

impl FixMsg {
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
    /// Returns an error when the root's `fix:branch` is malformed, when
    /// the root is not a Struct field, or when the value violates it, naming
    /// the path of the first value that does not fit.
    pub fn with_registry(registry: Arc<FixRegistry>, field: Field, value: Scalar) -> Result<Self> {
        let declared = field.as_fix().branch()?;
        let branch = registry
            .branch_named(declared.name())
            .cloned()
            .unwrap_or(declared);
        let value = field.canonicalize_value(value)?;
        Ok(Self {
            registry,
            entries: Vec::new(),
            branch,
            tags: tag_positions(&field),
            field,
            value,
        })
    }

    /// Builds a message a reader already resolved, entries and all.
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

    /// Returns what arrived, in arrival order, untranslated.
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

    /// The version this message is expressed in, read back from the fields
    /// that declare it.
    ///
    /// Derived rather than stored: `BeginString(8)` and `ApplVerID(1128)` are
    /// what a message says about itself, so a converted one cannot lie.
    #[must_use]
    pub fn version(&self) -> Option<Version> {
        let begin = self.get_by_tag(8).and_then(Scalar::as_str)?;
        begin
            .strip_prefix("FIX.")
            .and_then(|rest| rest.parse().ok())
    }

    /// Returns the registry this message resolves against.
    pub const fn registry(&self) -> &Arc<FixRegistry> {
        &self.registry
    }

    /// Returns the dictionary this message is spelled in.
    pub const fn branch(&self) -> &FixBranch {
        &self.branch
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
    /// An identifier is exact and does not tier: it names one dictionary, and
    /// a dictionary this message does not speak simply misses.
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
    /// picks the root child. The tag is looked for in this message's own
    /// branch first and then in the standard one, so a venue field and
    /// `MsgType` are both reachable from a venue message. A tag neither
    /// answers is looked for under its decimal rendering, so an unknown tag a
    /// transcriber retained is still reachable.
    pub fn get_by_tag(&self, tag: i32) -> Option<&Scalar> {
        // The message's own children answer first, by the tag they carry -
        // one hash-free binary search over the index resolved at construction,
        // where the dictionary tiers below are a probe per branch. A field a
        // dictionary never explained carries no tag and falls through to them.
        if let Some(index) = self.index_of_tag(tag) {
            return self.value.get(index);
        }
        let index = match self.known_by_tag(tag) {
            Some(known) => self.field.index_of(known.name()),
            None => {
                let mut rendered = Decimal::default();
                rendered.render(tag)?;
                self.field.index_of(rendered.as_str())
            }
        }?;
        self.value.get(index)
    }

    /// The root child carrying one tag, by that child's own declaration.
    pub(super) fn index_of_tag(&self, tag: i32) -> Option<usize> {
        let found = self.tags.binary_search_by_key(&tag, |(held, _)| *held);
        found.ok().map(|at| self.tags[at].1)
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
    /// The name folds through the registry to its canonical spelling in this
    /// message's branch first and then in the standard one, and an exact
    /// root-child match is the fallback when neither knows it.
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

    /// Returns the value a dotted path reaches.
    ///
    /// The whole string is tried as a name first. Otherwise the first segment
    /// resolves as [`Self::get_by_name`] does and each further segment
    /// descends: into a Struct child by name - the registry's canonical
    /// spelling first, then an exact match - or, when the value at hand is
    /// the sequence a List field holds, a decimal segment indexes one entry.
    /// A repeating group is that List of Structs, so reaching one member
    /// needs the entry's index: `NoPartyIDs.0.PartyID`.
    pub fn get_by_path(&self, path: &str) -> Option<&Scalar> {
        if let Some(value) = self.get_by_name(path) {
            return Some(value);
        }
        let mut segments = path.split('.');
        let index = self.child_index(&self.field, segments.next()?)?;
        let mut field = self.field.fields().get(index)?;
        let mut value = self.value.get(index)?;
        for segment in segments {
            (field, value) = self.descend(field, value, segment)?;
        }
        Some(value)
    }

    /// Returns the value a dotted path reaches, raising absence.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the path.
    pub fn by_path(&self, path: &str) -> Result<&Scalar> {
        self.get_by_path(path)
            .ok_or_else(|| absent(format_args!("path {path:?}")))
    }

    /// Returns the value a tag, an identifier or a name reaches.
    ///
    /// Matches the key once and redirects: a tag to [`Self::get_by_tag`], an
    /// identifier to [`Self::get_by_id`], a name to [`Self::get_by_path`].
    pub fn get<'key>(&self, key: impl Into<FixKey<'key>>) -> Option<&Scalar> {
        match key.into() {
            FixKey::Tag(tag) => self.get_by_tag(tag),
            FixKey::Id(id) => self.get_by_id(id),
            FixKey::Name(name) => self.get_by_path(name),
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
            FixKey::Name(name) => self.by_path(name),
        }
    }

    /// The field a bare tag names: this message's branch, then the
    /// standard one.
    ///
    /// Step one is skipped when this message is already standard, because the
    /// two probes would be the same one, and when the identifier it would
    /// build is inadmissible - a specification tag belongs to the standard
    /// branch and to no other.
    fn known_by_tag(&self, tag: i32) -> Option<&Field> {
        let own = if self.branch.is_standard() || !FixId::is_admissible(&self.branch, tag) {
            None
        } else {
            FixId::from_parts(&self.branch, tag)
                .ok()
                .and_then(|id| self.registry.get_field_by_id(id))
        };
        own.or_else(|| self.registry.get_field_by_id(FixId::standard(tag)))
    }

    /// The field a bare name reaches: this message's branch, then the
    /// standard one.
    fn known_by_name(&self, name: &str) -> Option<&Field> {
        if !self.branch.is_standard() {
            if let Some(field) = self.registry.get_field_by_name(name, Some(&self.branch)) {
                return Some(field);
            }
        }
        self.registry
            .get_field_by_name(name, Some(&FixBranch::STANDARD))
    }

    /// The position of the child `name` reaches under `parent`: the
    /// registry's canonical spelling first, then an exact match.
    fn child_index(&self, parent: &Field, name: &str) -> Option<usize> {
        self.known_by_name(name)
            .and_then(|known| parent.index_of(known.name()))
            .or_else(|| parent.index_of(name))
    }

    /// One step of a path: into a Struct child by name, or into a List entry
    /// by index.
    fn descend<'value>(
        &self,
        field: &'value Field,
        value: &'value Scalar,
        segment: &str,
    ) -> Option<(&'value Field, &'value Scalar)> {
        match field.dtype() {
            DataType::Struct(_) => {
                let index = self.child_index(field, segment)?;
                Some((field.fields().get(index)?, value.get(index)?))
            }
            DataType::List(item)
            | DataType::LargeList(item)
            | DataType::FixedSizeList(item, _)
            | DataType::ListView(item)
            | DataType::LargeListView(item) => {
                if segment.is_empty() || !segment.bytes().all(|byte| byte.is_ascii_digit()) {
                    return None;
                }
                Some((item.as_ref(), value.get(segment.parse().ok()?)?))
            }
            _ => None,
        }
    }
}

/// Report that nothing in the message is reached by `what`.
fn absent(what: impl fmt::Display) -> Error {
    Error::absent("fix value", what)
}

/// A tag rendered in decimal on the stack, for the unknown-tag lookup.
///
/// Eleven bytes hold every `i32`, sign included, so rendering never
/// allocates and a miss costs nothing.
#[derive(Default)]
struct Decimal {
    bytes: [u8; 11],
    len: usize,
}

impl Decimal {
    /// Render `tag`, answering `None` only if it could not fit - which no
    /// `i32` fails.
    fn render(&mut self, tag: i32) -> Option<()> {
        use std::fmt::Write as _;
        write!(self, "{tag}").ok()
    }

    /// The rendered text.
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or_default()
    }
}

impl fmt::Write for Decimal {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.len + text.len();
        if end > self.bytes.len() {
            return Err(fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
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
