//! A FIX message: a value plus the registry that types it.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::{FixId, FixKey, FixNamespace, FixRegistry};
use crate::{DataType, Error, Field, Result, Scalar};

/// A FIX message value, resolved against one registry.
///
/// The schema is a core Struct [`Field`] - a non-null Struct field is the
/// only row schema - and the value is that row: a [`Scalar::Record`] input
/// canonicalizes to the ordered [`Scalar::Sequence`] the root declares,
/// exactly as every other row does. The registry link is an [`Arc`], cloned
/// from [`FixRegistry::global`] when the caller names none, so a message
/// carries the dictionary it was resolved against and a later lookup cannot
/// silently use a different one.
///
/// A message has a namespace, and it is derived rather than declared: it is
/// the root field's own `fix:namespace`, resolved once at construction, so
/// nothing can disagree with it. A bare tag or name then resolves in a fixed
/// two-step tier - this message's namespace first, when the identifier that
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
///     ("OrderQty", Scalar::I64(100)),
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
    /// The root field's own namespace, resolved once so a lookup stays total
    /// and allocation-free and corruption is reported at construction. It is
    /// derived from `field`, which is what equality, hashing and
    /// serialization already carry, so it is not state of its own.
    namespace: FixNamespace,
    field: Field,
    value: Scalar,
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
    /// Returns an error when the root's `fix:namespace` is malformed, when
    /// the root is not a Struct field, or when the value violates it, naming
    /// the path of the first value that does not fit.
    pub fn with_registry(registry: Arc<FixRegistry>, field: Field, value: Scalar) -> Result<Self> {
        let namespace = field.as_fix().namespace()?;
        let value = field.canonicalize_value(value)?;
        Ok(Self {
            registry,
            namespace,
            field,
            value,
        })
    }

    /// Returns the registry this message resolves against.
    pub const fn registry(&self) -> &Arc<FixRegistry> {
        &self.registry
    }

    /// Returns the dictionary this message is spelled in.
    pub const fn namespace(&self) -> &FixNamespace {
        &self.namespace
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
    pub fn get_by_id(&self, id: &FixId) -> Option<&Scalar> {
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
    pub fn by_id(&self, id: &FixId) -> Result<&Scalar> {
        self.get_by_id(id).ok_or_else(|| absent(FixKey::Id(id)))
    }

    /// Returns the value of the root child a tag names.
    ///
    /// The registry resolves the tag to its canonical name, and that name
    /// picks the root child. The tag is looked for in this message's own
    /// namespace first and then in the standard one, so a venue field and
    /// `MsgType` are both reachable from a venue message. A tag neither
    /// answers is looked for under its decimal rendering, so an unknown tag a
    /// transcriber retained is still reachable.
    pub fn get_by_tag(&self, tag: i32) -> Option<&Scalar> {
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
    /// message's namespace first and then in the standard one, and an exact
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

    /// The field a bare tag names: this message's namespace, then the
    /// standard one.
    ///
    /// Step one is skipped when this message is already standard, because the
    /// two probes would be the same one, and when the identifier it would
    /// build is inadmissible - a specification tag belongs to the standard
    /// namespace and to no other.
    fn known_by_tag(&self, tag: i32) -> Option<&Field> {
        let own = if self.namespace.is_standard() || !FixId::is_admissible(&self.namespace, tag) {
            None
        } else {
            FixId::from_parts(self.namespace.clone(), tag)
                .ok()
                .and_then(|id| self.registry.get_field_by_id(&id))
        };
        own.or_else(|| self.registry.get_field_by_id(&FixId::standard(tag)))
    }

    /// The field a bare name reaches: this message's namespace, then the
    /// standard one.
    fn known_by_name(&self, name: &str) -> Option<&Field> {
        if !self.namespace.is_standard() {
            if let Some(field) = self.registry.get_field_by_name(&self.namespace, name) {
                return Some(field);
            }
        }
        self.registry
            .get_field_by_name(&FixNamespace::STANDARD, name)
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
