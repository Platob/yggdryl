//! Node.js view of the FIX dictionary, its message, and the process default.
//!
//! Nothing here resolves, folds, merges, shards or validates: the registry is
//! one [`Arc`] over the core [`FixRegistry`](CoreFixRegistry), and every
//! accessor coerces its key once at the boundary and redirects to the most
//! specific native method. The typed `fix:` vocabulary is not here either - it
//! lives on the protocol view class [`JsProtocolField`](crate::JsProtocolField),
//! which is what `field.fix` already answers.
//!
//! A branch and an identifier cross as `string` and are parsed once here
//! through [`branch_from_js`] and [`id_from_js`], so neither gets a class of
//! its own in JavaScript and the grammar, the ASCII folding and the
//! standard-tag rule all stay the core's. A bare tag and a bare name still mean
//! the standard branch, and a colon-bearing string is a name, never an
//! identifier.

use std::sync::Arc;

use napi::JsValue as _;
use napi::bindgen_prelude::{ClassInstance, Env, Generator, Result, Unknown, ValueType};
use napi_derive::napi;
use yggdryl::{
    Field as CoreField, FixBranch as CoreFixBranch, FixId as CoreFixId, FixKey,
    FixMsg as CoreFixMsg, FixRegistry as CoreFixRegistry, Scalar,
};

use crate::iobase::{LocationInput, folder_from_input};
use crate::text::codec::JsScalar;
use crate::types::field::JsField;
use crate::{exact_i32, napi_error, napi_type_error};

/// What a mutation says when something else still holds the dictionary.
const SHARED: &str =
    "this registry is shared with a message or installed as the process default; build a new one";

/// Read one branch, or throw the native parse failure.
///
/// A branch crosses as text and becomes a [`FixBranch`](CoreFixBranch) here,
/// once, so no second class exists in JavaScript and the grammar - a leading
/// ASCII letter, no `:` or `,`, at most 23 bytes, ASCII case folded - stays the
/// core's.
pub(crate) fn branch_from_js(text: &str) -> Result<CoreFixBranch> {
    CoreFixBranch::from_str(text).map_err(napi_error)
}

/// Read one identifier, or throw the native parse failure.
///
/// The text is `branch:tag`, and `FixId::from_str` is what parses it - the
/// standard-tag rule included, so `cme:35` is refused here exactly as it is in
/// Rust.
pub(crate) fn id_from_js(text: &str) -> Result<CoreFixId> {
    CoreFixId::from_str(text).map_err(napi_error)
}

/// What an absent `fix:branch` means, for the `fix` namespace to freeze.
#[napi(js_name = "_fixStandardBranchNative", skip_typescript)]
pub fn fix_standard_branch_native() -> String {
    CoreFixBranch::STANDARD.as_str().to_owned()
}

/// Where the FIX specification's own tag range ends, for the same namespace.
#[napi(js_name = "_fixStandardTagLimitNative", skip_typescript)]
pub fn fix_standard_tag_limit_native() -> i32 {
    CoreFixId::STANDARD_TAG_LIMIT
}

/// One lookup key, read once at the boundary.
///
/// A `number` is a tag in the standard branch and a `string` is a name or a
/// dotted path in the standard branch, exactly as the core's [`FixKey`] splits
/// them; a colon-bearing string is a name, never an identifier. The owned name
/// is what lets the borrowed key be rebuilt for each call without the caller's
/// value staying alive.
enum FixKeyArg {
    Tag(i32),
    Name(String),
}

impl FixKeyArg {
    /// Read a key, or throw a `TypeError` naming what a FIX lookup accepts.
    ///
    /// A tag crosses as a JavaScript number and is checked exactly: a
    /// fractional or out-of-`i32` value is refused rather than narrowed into a
    /// different tag.
    fn from_js(env: Env, key: &Unknown<'_>, argument: &str) -> Result<Self> {
        match key.get_type()? {
            ValueType::Number => Ok(Self::Tag(exact_i32(
                key.coerce_to_number()?.get_double()?,
                argument,
            )?)),
            ValueType::String => Ok(Self::Name(
                key.coerce_to_string()?.into_utf8()?.into_owned()?,
            )),
            other => Err(napi_type_error(
                env,
                format!("{argument} must be a number tag or a string name, got {other}"),
            )),
        }
    }

    /// Borrow the key the core matches on.
    fn as_key(&self) -> FixKey<'_> {
        match self {
            Self::Tag(tag) => FixKey::Tag(*tag),
            Self::Name(name) => FixKey::Name(name.as_str()),
        }
    }
}

/// FIX field definitions resolved by identifier, by tag, by name, or by dotted
/// path.
///
/// The dictionary is shared rather than copied, because a `FixMsg` links the
/// very registry it was resolved against and the process default is one too: a
/// mutation therefore refuses while anything else holds it, rather than
/// changing a dictionary underneath a message that already used it.
#[napi(js_name = "FixRegistry")]
pub struct JsFixRegistry {
    pub(crate) inner: Arc<CoreFixRegistry>,
}

impl JsFixRegistry {
    /// Wrap a shared registry, sharing rather than copying it.
    const fn from_arc(inner: Arc<CoreFixRegistry>) -> Self {
        Self { inner }
    }

    /// Borrow the registry for a mutation, refusing a shared one.
    fn inner_mut(&mut self) -> Result<&mut CoreFixRegistry> {
        Arc::get_mut(&mut self.inner).ok_or_else(|| napi_error(SHARED))
    }
}

// Counts cross as JavaScript numbers; a dictionary never approaches 2^32.
#[allow(clippy::cast_possible_truncation)]
#[napi]
impl JsFixRegistry {
    /// The empty registry.
    #[napi(constructor)]
    pub fn new() -> Self {
        Self::from_arc(Arc::new(CoreFixRegistry::new()))
    }

    /// Build a registry by inserting `fields` in order.
    ///
    /// The first refusal fails the whole build.
    #[napi(factory)]
    pub fn from_fields(fields: Vec<ClassInstance<'_, JsField>>) -> Result<Self> {
        CoreFixRegistry::from_fields(fields.iter().map(|field| field.inner.clone()))
            .map(|registry| Self::from_arc(Arc::new(registry)))
            .map_err(napi_error)
    }

    /// Load every shard under `<location>/primitive` and `<location>/nested`.
    ///
    /// `location` is an `IOBase` handle, a `Url`, or the string naming one, run
    /// through the coercion every folder-shaped entry point uses. A folder that
    /// is not there loads as the empty registry and is not created; a shard
    /// that does not parse, and a root still holding the retired `records/`
    /// layout, throw with the URL named.
    #[napi(factory)]
    pub fn from_handle(location: LocationInput<'_>) -> Result<Self> {
        let holder = folder_from_input(location)?;
        CoreFixRegistry::from_handle(&holder)
            .map(|registry| Self::from_arc(Arc::new(registry)))
            .map_err(napi_error)
    }

    /// Write every populated shard under `<location>/<tree>/<branch>`, removing
    /// the shards, branch folders and trees no field populates any more.
    #[napi]
    pub fn write_into(&self, location: LocationInput<'_>) -> Result<()> {
        let mut holder = folder_from_input(location)?;
        self.inner.write_into(&mut holder).map_err(napi_error)
    }

    /// How many fields are registered.
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        u32::try_from(self.inner.len()).unwrap_or(u32::MAX)
    }

    /// The field a canonical or alternate identifier names, or `null`.
    ///
    /// `id` is the `branch:tag` text; a malformed one throws the native parse
    /// failure, never a miss.
    #[napi]
    pub fn get_field_by_id(&self, id: String) -> Result<Option<JsField>> {
        let id = id_from_js(&id)?;
        Ok(self
            .inner
            .get_field_by_id(&id)
            .cloned()
            .map(JsField::from_core))
    }

    /// The field a canonical or alternate identifier names.
    #[napi]
    pub fn field_by_id(&self, id: String) -> Result<JsField> {
        let id = id_from_js(&id)?;
        self.inner
            .field_by_id(&id)
            .map(|field| JsField::from_core(field.clone()))
            .map_err(napi_error)
    }

    /// The field a canonical or alternate tag names, or `null`.
    ///
    /// A bare tag is the standard branch exactly, never whichever dictionary
    /// happens to be loaded.
    #[napi]
    pub fn get_field_by_tag(&self, tag: f64) -> Result<Option<JsField>> {
        let tag = exact_i32(tag, "tag")?;
        Ok(self
            .inner
            .get_field_by_tag(tag)
            .cloned()
            .map(JsField::from_core))
    }

    /// The field a canonical or alternate tag names.
    #[napi]
    pub fn field_by_tag(&self, tag: f64) -> Result<JsField> {
        let tag = exact_i32(tag, "tag")?;
        self.inner
            .field_by_tag(tag)
            .map(|field| JsField::from_core(field.clone()))
            .map_err(napi_error)
    }

    /// The field a canonical name or alias names inside one dictionary, ASCII
    /// case folded, or `null`.
    ///
    /// A name is unique per branch, not registry-wide, so the dictionary is
    /// named: `'standard'` is the specification's own.
    #[napi]
    pub fn get_field_by_name(&self, branch: String, name: String) -> Result<Option<JsField>> {
        let branch = branch_from_js(&branch)?;
        Ok(self
            .inner
            .get_field_by_name(&branch, &name)
            .cloned()
            .map(JsField::from_core))
    }

    /// The field a canonical name or alias names inside one dictionary, ASCII
    /// case folded.
    #[napi]
    pub fn field_by_name(&self, branch: String, name: String) -> Result<JsField> {
        let branch = branch_from_js(&branch)?;
        self.inner
            .field_by_name(&branch, &name)
            .map(|field| JsField::from_core(field.clone()))
            .map_err(napi_error)
    }

    /// The field a dotted path reaches through a component or a group, in one
    /// dictionary, or `null`.
    #[napi]
    pub fn get_field_by_path(&self, branch: String, path: String) -> Result<Option<JsField>> {
        let branch = branch_from_js(&branch)?;
        Ok(self
            .inner
            .get_field_by_path(&branch, &path)
            .cloned()
            .map(JsField::from_core))
    }

    /// The field a dotted path reaches through a component or a group, in one
    /// dictionary.
    #[napi]
    pub fn field_by_path(&self, branch: String, path: String) -> Result<JsField> {
        let branch = branch_from_js(&branch)?;
        self.inner
            .field_by_path(&branch, &path)
            .map(|field| JsField::from_core(field.clone()))
            .map_err(napi_error)
    }

    /// The field a tag or a name reaches in the standard branch, or `null`.
    #[napi(ts_args_type = "key: number | string")]
    pub fn get_field(&self, env: Env, key: Unknown<'_>) -> Result<Option<JsField>> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        Ok(self
            .inner
            .get_field(key.as_key())
            .cloned()
            .map(JsField::from_core))
    }

    /// The field a tag or a name reaches in the standard branch.
    #[napi(ts_args_type = "key: number | string")]
    pub fn field(&self, env: Env, key: Unknown<'_>) -> Result<JsField> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        self.inner
            .field(key.as_key())
            .map(|field| JsField::from_core(field.clone()))
            .map_err(napi_error)
    }

    /// The field a tag or a name reaches in the standard branch, or `null`:
    /// the Map-like spelling of `getField`.
    #[napi(ts_args_type = "key: number | string")]
    pub fn get(&self, env: Env, key: Unknown<'_>) -> Result<Option<JsField>> {
        self.get_field(env, key)
    }

    /// Whether a tag or a name reaches a field in the standard branch.
    #[napi(ts_args_type = "key: number | string")]
    pub fn has(&self, env: Env, key: Unknown<'_>) -> Result<bool> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        Ok(self.inner.contains(key.as_key()))
    }

    /// Add a field, answering the one it replaced.
    #[napi]
    pub fn insert(&mut self, field: &JsField) -> Result<Option<JsField>> {
        let field = field.inner.clone();
        Ok(self
            .inner_mut()?
            .insert(field)
            .map_err(napi_error)?
            .map(JsField::from_core))
    }

    /// Merge a definition into the stored field with the same canonical
    /// identifier.
    #[napi]
    pub fn update(&mut self, field: &JsField) -> Result<()> {
        let field = field.inner.clone();
        self.inner_mut()?.update(field).map_err(napi_error)
    }

    /// Remove the field a tag or a name reaches in the standard branch,
    /// answering it.
    #[napi(ts_args_type = "key: number | string")]
    pub fn remove(&mut self, env: Env, key: Unknown<'_>) -> Result<Option<JsField>> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        Ok(self
            .inner_mut()?
            .remove(key.as_key())
            .map(JsField::from_core))
    }

    /// Remove the field a canonical or alternate identifier names, answering
    /// it.
    ///
    /// The generic `remove` reads a string as a standard-branch name, so this
    /// is the spelling that reaches a vendor dictionary at all; `id` is parsed
    /// exactly as every other identifier argument is.
    #[napi]
    pub fn remove_by_id(&mut self, id: String) -> Result<Option<JsField>> {
        let id = id_from_js(&id)?;
        Ok(self.inner_mut()?.remove(&id).map(JsField::from_core))
    }

    /// The fields in ascending canonical-identifier order, lazily.
    ///
    /// The order is the core's: branch-major, then by tag. The iterator holds
    /// the registry and the identifier it stopped at, so nothing is collected
    /// crossing the boundary and the dictionary is never cloned to walk it.
    /// Holding it is therefore sharing it: a mutation refuses until the walk
    /// ends, which is what stops the fields moving under a cursor into them.
    /// The loader wires `Symbol.iterator` over this.
    #[napi(ts_return_type = "Generator<Field>")]
    pub fn keys(&self) -> JsFixFieldIterator {
        JsFixFieldIterator {
            registry: Some(Arc::clone(&self.inner)),
            after: None,
        }
    }

    /// Whether two registries hold the same fields, in canonical-identifier
    /// order.
    #[napi]
    pub fn equals(&self, other: &JsFixRegistry) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner) || self.inner == other.inner
    }

    /// Deterministic hash bits over the fields, shared with the core.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        Scalar::from_sequence(self.inner.iter().map(Scalar::from)).stable_hash()
    }

    /// A deep copy that is independently mutable.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        Self::from_arc(Arc::new((*self.inner).clone()))
    }

    /// A one-line summary: the dictionary itself is reached by iterating it.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        format!("FixRegistry({} fields)", self.inner.len())
    }

    /// The fields as their own JSON documents, in canonical-identifier order.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> Result<Vec<serde_json::Value>> {
        self.inner
            .iter()
            .map(|field| serde_json::to_value(field).map_err(napi_error))
            .collect()
    }
}

impl Default for JsFixRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// The fields of a registry, in ascending canonical-identifier order.
///
/// Answered by `keys()`. It advances with the core's own cursor - the registry
/// plus the last `FixId` it answered - so taking one field from a dictionary of
/// thousands costs one lookup, and a walk crosses every branch in the one order
/// the core iterates. It lets the registry go the moment the walk ends, because
/// JavaScript collects at its own pace and a mutation must not wait for a
/// drained iterator to be swept.
#[napi(iterator, js_name = "FixFieldIterator")]
pub struct JsFixFieldIterator {
    registry: Option<Arc<CoreFixRegistry>>,
    after: Option<CoreFixId>,
}

impl Generator for JsFixFieldIterator {
    type Yield = JsField;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<Self::Next>) -> Option<Self::Yield> {
        let found = self.registry.as_ref().and_then(|registry| {
            registry
                .next_field_after(self.after.as_ref())
                .map(|field| (field.clone(), field.as_fix().id().ok().flatten()))
        });
        match found {
            // The cursor is the canonical identifier every registered field
            // carries; a field without one cannot be advanced past, so the
            // walk ends there rather than answering it forever.
            Some((field, Some(id))) => {
                self.after = Some(id);
                Some(JsField::from_core(field))
            }
            Some((field, None)) => {
                self.registry = None;
                Some(JsField::from_core(field))
            }
            None => {
                self.registry = None;
                None
            }
        }
    }

    fn complete(&mut self, _value: Option<Self::Return>) -> Option<Self::Yield> {
        // `for...of` with a `break` calls this, so an abandoned walk stops
        // sharing the registry as promptly as a drained one.
        self.registry = None;
        None
    }
}

/// A FIX message: a value plus the registry that types it.
///
/// The schema is one non-null Struct `Field` - the only row schema - and the
/// value the row it declares, so a plain object crosses as the record the core
/// canonicalizes into that order exactly as every other row is. The message is
/// immutable: it compares, hashes, renders and clones by the schema and the
/// value it carries, against the registry it was resolved against.
#[napi(js_name = "FixMsg")]
pub struct JsFixMsg {
    inner: CoreFixMsg,
}

impl JsFixMsg {
    /// The value both the hash and the JSON document read.
    fn identity_value(&self) -> Scalar {
        Scalar::from_sequence([
            Scalar::from(self.inner.as_field()),
            self.inner.as_value().clone(),
        ])
    }
}

#[napi]
impl JsFixMsg {
    /// Build a message, linking the process default when none is named.
    ///
    /// The loader widens `value`: anything `Scalar.fromJs` reads becomes the
    /// native value first, and the core alone validates and canonicalizes it
    /// against `field`.
    #[napi(constructor)]
    pub fn new(
        field: &JsField,
        value: &JsScalar,
        registry: Option<ClassInstance<'_, JsFixRegistry>>,
    ) -> Result<Self> {
        let registry = match registry {
            Some(registry) => Arc::clone(&registry.inner),
            None => Arc::clone(CoreFixRegistry::global().map_err(napi_error)?),
        };
        CoreFixMsg::with_registry(registry, field.inner.clone(), value.inner.clone())
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// The registry this message resolves against, sharing it.
    #[napi(getter)]
    pub fn registry(&self) -> JsFixRegistry {
        JsFixRegistry::from_arc(Arc::clone(self.inner.registry()))
    }

    /// The root Struct field: the message's resolved schema.
    #[napi(getter)]
    pub fn field(&self) -> JsField {
        JsField::from_core(self.inner.as_field().clone())
    }

    /// The ordered row value.
    #[napi(getter)]
    pub fn value(&self) -> JsScalar {
        JsScalar::from_core(self.inner.as_value().clone())
    }

    /// How many values the root declares, which is what `entries` yields.
    ///
    /// Counts are JavaScript numbers, exact to 2^53, as everywhere else at
    /// this boundary; Python spells the same answer `len(message)`.
    #[allow(clippy::cast_precision_loss)]
    #[napi(getter)]
    pub fn size(&self) -> f64 {
        self.inner.as_field().fields().len() as f64
    }

    /// The dictionary this message is spelled in.
    ///
    /// Derived from the root field's own `fix:branch` at construction, never
    /// declared, so nothing can disagree with it; `'standard'` when the root
    /// states none.
    #[napi(getter)]
    pub fn branch(&self) -> String {
        self.inner.branch().as_str().to_owned()
    }

    /// The value of the root child an identifier names, or `null`.
    ///
    /// An identifier is exact and does not tier: a dictionary this message does
    /// not speak simply misses.
    #[napi]
    pub fn get_by_id(&self, id: String) -> Result<Option<JsScalar>> {
        let id = id_from_js(&id)?;
        Ok(self.inner.get_by_id(&id).cloned().map(JsScalar::from_core))
    }

    /// The value of the root child an identifier names.
    #[napi]
    pub fn by_id(&self, id: String) -> Result<JsScalar> {
        let id = id_from_js(&id)?;
        self.inner
            .by_id(&id)
            .map(|value| JsScalar::from_core(value.clone()))
            .map_err(napi_error)
    }

    /// The value of the root child a tag names, or `null`.
    ///
    /// The tag resolves in this message's own branch first, then in the
    /// standard one.
    #[napi]
    pub fn get_by_tag(&self, tag: f64) -> Result<Option<JsScalar>> {
        let tag = exact_i32(tag, "tag")?;
        Ok(self.inner.get_by_tag(tag).cloned().map(JsScalar::from_core))
    }

    /// The value of the root child a tag names.
    #[napi]
    pub fn by_tag(&self, tag: f64) -> Result<JsScalar> {
        let tag = exact_i32(tag, "tag")?;
        self.inner
            .by_tag(tag)
            .map(|value| JsScalar::from_core(value.clone()))
            .map_err(napi_error)
    }

    /// The value of the root child a name reaches, or `null`.
    ///
    /// The name folds through this message's own branch first, then the
    /// standard one.
    #[napi]
    pub fn get_by_name(&self, name: String) -> Option<JsScalar> {
        self.inner
            .get_by_name(&name)
            .cloned()
            .map(JsScalar::from_core)
    }

    /// The value of the root child a name reaches.
    #[napi]
    pub fn by_name(&self, name: String) -> Result<JsScalar> {
        self.inner
            .by_name(&name)
            .map(|value| JsScalar::from_core(value.clone()))
            .map_err(napi_error)
    }

    /// The value a dotted path reaches, or `null`.
    #[napi]
    pub fn get_by_path(&self, path: String) -> Option<JsScalar> {
        self.inner
            .get_by_path(&path)
            .cloned()
            .map(JsScalar::from_core)
    }

    /// The value a dotted path reaches.
    #[napi]
    pub fn by_path(&self, path: String) -> Result<JsScalar> {
        self.inner
            .by_path(&path)
            .map(|value| JsScalar::from_core(value.clone()))
            .map_err(napi_error)
    }

    /// The value a tag or a name reaches in the standard branch tier, or
    /// `null`.
    #[napi(ts_args_type = "key: number | string")]
    pub fn get(&self, env: Env, key: Unknown<'_>) -> Result<Option<JsScalar>> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        Ok(self
            .inner
            .get(key.as_key())
            .cloned()
            .map(JsScalar::from_core))
    }

    /// The value a tag or a name reaches in the standard branch tier.
    ///
    /// The failing half of `get` is spelled `at` rather than the core's
    /// `value`, because `value` is this class's property for the whole message
    /// value.
    #[napi(ts_args_type = "key: number | string")]
    pub fn at(&self, env: Env, key: Unknown<'_>) -> Result<JsScalar> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        self.inner
            .value(key.as_key())
            .map(|value| JsScalar::from_core(value.clone()))
            .map_err(napi_error)
    }

    /// The `[name, value]` pairs of the root, in the order it declares.
    ///
    /// The loader wires `Symbol.iterator` over this.
    #[napi(ts_return_type = "Generator<[string, Scalar]>")]
    pub fn entries(&self) -> JsFixMsgEntries {
        JsFixMsgEntries {
            field: self.inner.as_field().clone(),
            value: self.inner.as_value().clone(),
            index: 0,
        }
    }

    /// Whether two messages carry the same schema, value and dictionary.
    #[napi]
    pub fn equals(&self, other: &JsFixMsg) -> bool {
        self.inner == other.inner
    }

    /// Deterministic hash bits over the schema and the value.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.identity_value().stable_hash()
    }

    /// A cheap clone: the schema and value are shared, the registry link kept.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    /// A one-line summary naming the root and how many values it holds.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        format!(
            "FixMsg({:?}, {} values)",
            self.inner.as_field().name(),
            self.inner.as_field().fields().len()
        )
    }

    /// The schema document and the value document, the two halves a message is.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> Result<serde_json::Value> {
        let field = serde_json::to_value(self.inner.as_field()).map_err(napi_error)?;
        let value = serde_json::from_str(
            &yggdryl::into_json_scalar(self.inner.as_value()).map_err(napi_error)?,
        )
        .map_err(napi_error)?;
        let mut document = serde_json::Map::with_capacity(2);
        document.insert("field".to_owned(), field);
        document.insert("value".to_owned(), value);
        Ok(serde_json::Value::Object(document))
    }
}

/// The `[name, value]` pairs of one message's root, in declared order.
#[napi(iterator, js_name = "FixMsgEntries")]
pub struct JsFixMsgEntries {
    field: CoreField,
    value: Scalar,
    index: usize,
}

impl Generator for JsFixMsgEntries {
    type Yield = (String, JsScalar);
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<Self::Next>) -> Option<Self::Yield> {
        let child = self.field.fields().get(self.index)?;
        let value = self.value.get(self.index)?.clone();
        let name = child.name().to_owned();
        self.index += 1;
        Some((name, JsScalar::from_core(value)))
    }
}

/// The process-wide registry, loading it on the first call.
///
/// The order is the core's: a registry installed by
/// [`fix_install_global_registry`], then the folder `YGGDRYL_FIX_REGISTRY`
/// names, then `~/.config/fix` when it exists, then the empty registry. Only
/// the third step treats absence as empty; every other failure throws with the
/// native message and the default stays unresolved, so the next call retries.
#[napi(js_name = "fixGlobalRegistryNative", skip_typescript)]
pub fn fix_global_registry() -> Result<JsFixRegistry> {
    CoreFixRegistry::global()
        .map(|registry| JsFixRegistry::from_arc(Arc::clone(registry)))
        .map_err(napi_error)
}

/// Install the process-wide registry before anything resolves it.
///
/// Throws once the default has resolved or been installed, so the value every
/// caller saw cannot change underneath them.
#[napi(js_name = "fixInstallGlobalRegistryNative", skip_typescript)]
pub fn fix_install_global_registry(registry: &JsFixRegistry) -> Result<()> {
    CoreFixRegistry::install_global((*registry.inner).clone()).map_err(napi_error)
}
