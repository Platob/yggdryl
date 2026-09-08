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
//! standard-tag rule all stay the core's. A bare tag or name uses the core's
//! deterministic best match, and a colon-bearing string is a name, never an
//! identifier.

use std::sync::Arc;

use napi::JsValue as _;
use napi::bindgen_prelude::{Buffer, ClassInstance, Env, Generator, Result, Unknown, ValueType};
use napi_derive::napi;
use yggdryl::{
    Field as CoreField, FixBranch as CoreFixBranch, FixCodec as CoreFixCodec, FixId as CoreFixId,
    FixKey, FixMsg as CoreFixMsg, FixRegistry as CoreFixRegistry, Scalar, Version as CoreVersion,
};

use crate::iobase::{LocationInput, folder_from_input, located_from_input};
use crate::text::codec::JsScalar;
use crate::types::field::JsField;
use crate::{exact_i32, exact_i64, napi_error, napi_type_error};

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
/// The text is `tag:branch`, and `FixId::from_str` is what parses it - the
/// standard-tag rule included, so `35:cme` is refused here exactly as it is in
/// Rust.
pub(crate) fn id_from_js(text: &str) -> Result<CoreFixId> {
    CoreFixId::from_str(text).map_err(napi_error)
}

/// Retain branch text beside the identifier for a field write.
///
/// The identifier parses first, exactly as `id_parts_from_py` does it. Reading
/// the colon first would answer a malformed identifier with a message of this
/// binding's own invention, where Python answers with the core's - and the
/// core's is the one every test and every documented example spells.
pub(crate) fn id_parts_from_js(text: &str) -> Result<(CoreFixBranch, CoreFixId)> {
    // The core parses first, so a malformed identifier is refused in the
    // grammar's own words - the same words Python's boundary answers with -
    // rather than in a sentence this file invented.
    let id = id_from_js(text)?;
    let branch = text
        .split_once(':')
        .map(|(_, branch)| branch)
        .ok_or_else(|| napi::Error::from_reason("a FIX identifier requires tag:branch"))?;
    Ok((branch_from_js(branch)?, id))
}

/// What an absent `fix:branch` means, for the `fix` namespace to freeze.
#[napi(js_name = "_fixStandardBranchNative", skip_typescript)]
pub fn fix_standard_branch_native() -> String {
    CoreFixBranch::STANDARD.name().to_owned()
}

/// Inclusive lower bound of FIX's user-defined tag range.
#[napi(js_name = "_fixUserTagMinNative", skip_typescript)]
pub fn fix_user_tag_min_native() -> i32 {
    CoreFixId::USER_TAG_MIN
}

/// Exclusive upper bound of FIX's user-defined tag range.
#[napi(js_name = "_fixUserTagMaxNative", skip_typescript)]
pub fn fix_user_tag_max_native() -> i32 {
    CoreFixId::USER_TAG_MAX
}

/// One lookup key, read once at the boundary.
///
/// A `number` is a tag and a `string` is a name or dotted path, exactly as the
/// core's [`FixKey`] splits
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

    /// Read an Ullink `CBlock` into a dictionary, with what it declared.
    ///
    /// Answers the dictionary and the message roots the file spelled out, in
    /// the order it spelled them. `branch` is the dialect its user-range tags
    /// belong to; with none named they stay on the standard branch.
    ///
    /// A file this cannot be read from throws the native sentence whole: the
    /// byte the reader stopped at, what was expected, what arrived, and the
    /// element the file spells it in.
    #[napi(ts_return_type = "[FixRegistry, Array<Field>]")]
    pub fn from_cfb_file(
        location: LocationInput<'_>,
        branch: Option<String>,
    ) -> Result<(Self, Vec<JsField>)> {
        // A CBlock is a file, so the location is held as whichever role it
        // actually is: a container handle reads no bytes, and a reader handed
        // one answers an empty vocabulary instead of a refusal.
        let handle = located_from_input(location)?;
        let branch = branch.map(|held| branch_from_js(&held)).transpose()?;
        let (registry, roots) =
            CoreFixRegistry::from_cfb_file(handle.as_io(), branch.as_ref()).map_err(napi_error)?;
        Ok((
            Self::from_arc(Arc::new(registry)),
            roots.into_iter().map(JsField::from_core).collect(),
        ))
    }

    /// Register one message type, answering the value it takes.
    ///
    /// A type the code set does not have is added to it rather than rejected,
    /// and the value it takes is the core's: itself where it fits, a stable
    /// synthesized value where it does not. `name` is the symbolic name the
    /// set files it under, with the spelling kept as an alias when the two
    /// differ, and `description` is the source's own wording. Idempotent and
    /// enriching: a type already spelled answers its value, gains a spelling
    /// the set did not answer to and a description it did not have, and keeps
    /// everything it already held.
    #[napi]
    pub fn register_msgtype(
        &mut self,
        spelling: String,
        name: Option<String>,
        description: Option<String>,
    ) -> Result<String> {
        self.inner_mut()?
            .register_msgtype(&spelling, name.as_deref(), description.as_deref())
            .map(|held| held.as_str().to_owned())
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
    /// `id` is the `tag:branch` text; a malformed one throws the native parse
    /// failure, never a miss.
    #[napi]
    pub fn get_field_by_id(&self, id: String) -> Result<Option<JsField>> {
        let id = id_from_js(&id)?;
        Ok(self
            .inner
            .get_field_by_id(id)
            .cloned()
            .map(JsField::from_core))
    }

    /// The field a canonical or alternate identifier names.
    #[napi]
    pub fn field_by_id(&self, id: String) -> Result<JsField> {
        let id = id_from_js(&id)?;
        self.inner
            .field_by_id(id)
            .map(|field| JsField::from_core(field.clone()))
            .map_err(napi_error)
    }

    /// The field a canonical or alternate tag names, or `null`.
    ///
    /// The standard dictionary wins, then named dictionaries in canonical
    /// name order.
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

    /// The field a canonical name or alias names, ASCII case folded, or `null`.
    ///
    /// Supplying `branch` restricts the lookup. Otherwise the core infers the
    /// best match: canonical before alias, standard before named branches.
    #[napi]
    pub fn get_field_by_name(
        &self,
        name: String,
        branch: Option<String>,
    ) -> Result<Option<JsField>> {
        let branch = branch.as_deref().map(branch_from_js).transpose()?;
        Ok(self
            .inner
            .get_field_by_name(&name, branch.as_ref())
            .cloned()
            .map(JsField::from_core))
    }

    /// The field a canonical name or alias names, ASCII case folded.
    #[napi]
    pub fn field_by_name(&self, name: String, branch: Option<String>) -> Result<JsField> {
        let branch = branch.as_deref().map(branch_from_js).transpose()?;
        self.inner
            .field_by_name(&name, branch.as_ref())
            .map(|field| JsField::from_core(field.clone()))
            .map_err(napi_error)
    }

    /// The field a dotted path reaches through a component or a group, or `null`.
    #[napi]
    pub fn get_field_by_path(
        &self,
        path: String,
        branch: Option<String>,
    ) -> Result<Option<JsField>> {
        let branch = branch.as_deref().map(branch_from_js).transpose()?;
        Ok(self
            .inner
            .get_field_by_path(&path, branch.as_ref())
            .cloned()
            .map(JsField::from_core))
    }

    /// The field a dotted path reaches through a component or a group.
    #[napi]
    pub fn field_by_path(&self, path: String, branch: Option<String>) -> Result<JsField> {
        let branch = branch.as_deref().map(branch_from_js).transpose()?;
        self.inner
            .field_by_path(&path, branch.as_ref())
            .map(|field| JsField::from_core(field.clone()))
            .map_err(napi_error)
    }

    /// The field a tag or name reaches by deterministic best match, or `null`.
    #[napi(ts_args_type = "key: number | string")]
    pub fn get_field(&self, env: Env, key: Unknown<'_>) -> Result<Option<JsField>> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        Ok(self
            .inner
            .get_field(key.as_key())
            .cloned()
            .map(JsField::from_core))
    }

    /// The field a tag or name reaches by deterministic best match.
    #[napi(ts_args_type = "key: number | string")]
    pub fn field(&self, env: Env, key: Unknown<'_>) -> Result<JsField> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        self.inner
            .field(key.as_key())
            .map(|field| JsField::from_core(field.clone()))
            .map_err(napi_error)
    }

    /// The field a tag or name reaches by deterministic best match, or `null`:
    /// the Map-like spelling of `getField`.
    #[napi(ts_args_type = "key: number | string")]
    pub fn get(&self, env: Env, key: Unknown<'_>) -> Result<Option<JsField>> {
        self.get_field(env, key)
    }

    /// Whether a tag or name reaches a field by deterministic best match.
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
        Ok(self.inner_mut()?.remove(id).map(JsField::from_core))
    }

    /// The branch one digest names, or `null`.
    ///
    /// An arrival entry carries its dialect as the digest `branch`, so this is
    /// the table that turns a capture's column back into the branch it was
    /// read under. The digest is one way, which is why the registry publishes
    /// the resolution rather than leaving a reader to reproduce the hash. A
    /// branch crosses as its name here, as it does everywhere else in this
    /// binding.
    #[napi]
    pub fn get_branch_by_digest(&self, digest: i32) -> Option<String> {
        self.inner
            .get_branch_by_digest(digest)
            .map(|branch| branch.name().to_owned())
    }

    /// The branch one digest names.
    ///
    /// # Errors
    ///
    /// Throws naming the digest when no branch carries it.
    #[napi]
    pub fn branch_by_digest(&self, digest: i32) -> Result<String> {
        self.inner
            .branch_by_digest(digest)
            .map(|branch| branch.name().to_owned())
            .map_err(napi_error)
    }

    /// The fields in ascending canonical-identifier order, lazily.
    ///
    /// The order is the core's: tag-major, then by branch digest. The iterator holds
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
                .next_field_after(self.after)
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

/// Every arrival entry, pre-order, as the tuple JavaScript reads.
///
/// The native record nests a group's members under the counter that heads
/// them; a binding is a view, so it flattens rather than inventing a second
/// shape. Order is the wire's.
///
/// A branch digest is an XXH32 held signed, and every `i32` is an exact
/// `f64`, so the number JavaScript reads is the digest rather than a rounding
/// of it.
fn flatten_entries(entries: &[yggdryl::FixEntry], out: &mut Vec<(f64, f64, String, String)>) {
    for entry in entries {
        out.push((
            f64::from(entry.tag()),
            f64::from(entry.branch()),
            entry.key().to_owned(),
            entry.value().to_owned(),
        ));
        flatten_entries(entry.children(), out);
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
    /// Wrap a message the core built.
    pub(crate) const fn from_core(inner: CoreFixMsg) -> Self {
        Self { inner }
    }

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
    /// declared, so nothing can disagree with it; empty when the root states
    /// none.
    #[napi(getter)]
    pub fn branch(&self) -> String {
        self.inner.branch().name().to_owned()
    }

    /// The value of the root child an identifier names, or `null`.
    ///
    /// An identifier is exact and does not tier: a dictionary this message does
    /// not speak simply misses.
    #[napi]
    pub fn get_by_id(&self, id: String) -> Result<Option<JsScalar>> {
        let id = id_from_js(&id)?;
        Ok(self.inner.get_by_id(id).cloned().map(JsScalar::from_core))
    }

    /// The value of the root child an identifier names.
    #[napi]
    pub fn by_id(&self, id: String) -> Result<JsScalar> {
        let id = id_from_js(&id)?;
        self.inner
            .by_id(id)
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

    /// The digest of what this message said, as sixteen bytes.
    ///
    /// Over the arrival record with the envelope tags left out, so two
    /// republications of one message digest alike however their sequence
    /// numbers and sending times differ.
    #[napi]
    pub fn digest(&self) -> Buffer {
        Buffer::from(self.inner.digest().to_be_bytes().to_vec())
    }

    /// One instrument symbol that is the same across venues, or `null`.
    #[napi]
    pub fn symbol_ticker(&self) -> Option<JsScalar> {
        answered(&self.inner.symbol_ticker())
    }

    /// The timestamp a capture is ordered by, or `null`.
    #[napi]
    pub fn market_timestamp(&self) -> Option<JsScalar> {
        answered(&self.inner.market_timestamp())
    }

    /// The partition that timestamp falls in, in whole seconds.
    #[napi]
    pub fn unix_partition(&self, seconds: f64) -> Result<Option<JsScalar>> {
        let seconds = exact_i64(seconds, "seconds")?;
        Ok(answered(&self.inner.unix_partition(seconds)))
    }

    /// One lifted facet's value, or `null` where nothing carries it.
    #[napi]
    pub fn lifted(&self, facet: String) -> Option<JsScalar> {
        self.inner.lifted(&facet).cloned().map(JsScalar::from_core)
    }

    /// Which field a lifted facet came from, or `null`.
    #[napi]
    pub fn lift_source(&self, facet: String) -> Option<String> {
        self.inner.lift_source(&facet).map(|id| id.to_string())
    }

    /// Every facet this message lifts, in the table's own order.
    #[napi(ts_return_type = "Array<[string, Scalar]>")]
    pub fn lift(&self) -> Vec<(String, JsScalar)> {
        self.inner
            .lift()
            .map(|(facet, value)| (facet.to_owned(), JsScalar::from_core(value.clone())))
            .collect()
    }

    /// One party by its role: identifier, source, role, qualifier.
    #[napi(ts_return_type = "Array<Scalar | null> | null")]
    pub fn party(&self, role: String) -> Option<Vec<Option<JsScalar>>> {
        let held = self.inner.party(&role)?;
        Some(vec![
            held.id().cloned().map(JsScalar::from_core),
            held.source().cloned().map(JsScalar::from_core),
            held.role().cloned().map(JsScalar::from_core),
            held.qualifier().cloned().map(JsScalar::from_core),
        ])
    }

    /// One regulatory timestamp by its type, or `null`.
    #[napi]
    pub fn trd_reg_timestamp(&self, kind: String) -> Option<JsScalar> {
        self.inner
            .trd_reg_timestamp(&kind)
            .cloned()
            .map(JsScalar::from_core)
    }

    /// What this message says about itself that does not add up.
    ///
    /// Derived by comparing the row against the arrival record, so a caller
    /// who never asks pays nothing.
    #[napi]
    pub fn anomalies(&self) -> Vec<String> {
        self.inner
            .anomalies()
            .map(|held| held.to_string())
            .collect()
    }

    /// What arrived, in arrival order, untranslated.
    ///
    /// Flattened pre-order: a group's members follow the counter pair that
    /// heads them, so a caller reading the array reads the wire. The dialect
    /// crosses as its digest, which `FixRegistry.branchByDigest` resolves.
    #[napi(ts_return_type = "Array<[number, number, string, string]>")]
    pub fn arrivals(&self) -> Vec<(f64, f64, String, String)> {
        let mut held = Vec::new();
        flatten_entries(self.inner.entries(), &mut held);
        held
    }

    /// This message as the fixed row a table holds.
    ///
    /// `schema` is the fixed root `fixSchema` builds: every column is filled by
    /// the tag its name spells, and a column no tag names answers null because
    /// it is the capture's rather than the message's.
    #[napi]
    pub fn to_row(&self, schema: &JsField) -> Result<JsScalar> {
        self.inner
            .to_row(&schema.inner)
            .map(JsScalar::from_core)
            .map_err(napi_error)
    }

    /// Re-emit this message on the wire, separated by `separator`.
    #[napi]
    pub fn to_bytes(&self, separator: Option<f64>) -> Result<Buffer> {
        let separator = separator_byte(separator)?;
        Ok(Buffer::from(self.inner.into_bytes(separator)))
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

/// One separator byte, or the FIX default where none was named.
///
/// A separator is one byte and a JavaScript number is not, so the narrowing is
/// stated here once rather than repeated at each entry point that takes one.
fn separator_byte(separator: Option<f64>) -> Result<u8> {
    let Some(held) = separator else {
        return Ok(SOH);
    };
    u8::try_from(exact_i64(held, "separator")?).map_err(|_| napi_error("a separator is one byte"))
}

/// The separator FIX itself uses.
const SOH: u8 = 0x01;

/// One value answered, or nothing where it is null.
///
/// A derived fact that no message carries is absent rather than a null value:
/// JavaScript already spells absence, and a `Scalar` holding null would make a
/// caller ask the same question twice.
fn answered(value: &Scalar) -> Option<JsScalar> {
    (!value.is_null()).then(|| JsScalar::from_core(value.clone()))
}

/// One dictionary, reading captured lines into messages.
///
/// The reader is the whole parse surface: a captured line with a verb in front
/// of it, a bare frame, a numeric frame with a stated separator, a bridge's
/// name/value text, or pairs a caller already has. Each redirects to the core
/// method of the same name, so nothing here decides a dialect, a version or a
/// separator - it only carries what JavaScript said across.
#[napi(js_name = "FixCodec")]
pub struct JsFixCodec {
    inner: CoreFixCodec,
    registry: Arc<CoreFixRegistry>,
}

#[napi]
impl JsFixCodec {
    /// Open a reader over one dictionary, or over the process default.
    #[napi(constructor)]
    pub fn new(
        registry: Option<ClassInstance<'_, JsFixRegistry>>,
        options: Option<FixCodecOptions>,
    ) -> Result<Self> {
        let registry = match registry {
            Some(held) => Arc::clone(&held.inner),
            None => Arc::clone(CoreFixRegistry::global().map_err(napi_error)?),
        };
        let options = options.unwrap_or_default();
        let mut inner = CoreFixCodec::new(Arc::clone(&registry));
        if let Some(held) = &options.branch {
            inner = inner.with_branch(&branch_from_js(held)?);
        }
        if let Some(held) = &options.version {
            inner = inner.with_version(version_from_js(held)?);
        }
        if let Some(held) = options.null_values {
            inner = inner.with_null_values(held);
        }
        Ok(Self { inner, registry })
    }

    /// The dictionary this reader resolves against, sharing it.
    #[napi(getter)]
    pub fn registry(&self) -> JsFixRegistry {
        JsFixRegistry::from_arc(Arc::clone(&self.registry))
    }

    /// One captured line, whatever it is wrapped in.
    #[napi]
    pub fn transform_line(&self, row: Buffer, enrich: Option<bool>) -> Result<JsFixMsg> {
        self.inner
            .transform_line(&row, enrich.unwrap_or(false))
            .map(JsFixMsg::from_core)
            .map_err(napi_error)
    }

    /// One numeric frame, split on the separator stated or inferred.
    #[napi]
    pub fn transform_fix_line(
        &self,
        body: Buffer,
        separator: Option<f64>,
        enrich: Option<bool>,
    ) -> Result<JsFixMsg> {
        let codec = match separator {
            Some(held) => self
                .inner
                .clone()
                .with_separator(separator_byte(Some(held))?),
            None => self.inner.clone(),
        };
        codec
            .transform_fix_line(&body, enrich.unwrap_or(false))
            .map(JsFixMsg::from_core)
            .map_err(napi_error)
    }

    /// One bridge frame, whose keys are names rather than tags.
    #[napi]
    pub fn transform_ullink_line(&self, body: Buffer, enrich: Option<bool>) -> Result<JsFixMsg> {
        self.inner
            .transform_ullink_line(&body, enrich.unwrap_or(false))
            .map(JsFixMsg::from_core)
            .map_err(napi_error)
    }

    /// One FIXML row, whose fields are XML attributes.
    #[napi]
    pub fn transform_fixml_line(&self, body: Buffer, enrich: Option<bool>) -> Result<JsFixMsg> {
        self.inner
            .transform_fixml_line(&body, enrich.unwrap_or(false))
            .map(JsFixMsg::from_core)
            .map_err(napi_error)
    }

    /// Pairs a caller already holds, in the order they arrived.
    #[napi(ts_args_type = "pairs: Array<[string, string]>")]
    pub fn transform_pairs(
        &self,
        pairs: Vec<(String, String)>,
        enrich: Option<bool>,
    ) -> Result<JsFixMsg> {
        let borrowed: Vec<(&[u8], &[u8])> = pairs
            .iter()
            .map(|(key, value)| (key.as_bytes(), value.as_bytes()))
            .collect();
        self.inner
            .transform_pairs(borrowed, enrich.unwrap_or(false))
            .map(JsFixMsg::from_core)
            .map_err(napi_error)
    }

    /// Fills what one message implies but did not carry.
    ///
    /// An order stating `OrderQty` and `CumQty` has said what `LeavesQty` is.
    /// Only the row is filled: the arrival record is what the wire carried and
    /// is left alone, so `toBytes` re-emits the received line either way, and
    /// a stated value is never replaced.
    #[napi]
    pub fn enrich_fixmsg(&self, message: &JsFixMsg) -> Result<JsFixMsg> {
        self.inner
            .enrich_fixmsg(message.inner.clone())
            .map(JsFixMsg::from_core)
            .map_err(napi_error)
    }

    /// A cheap clone: the dictionary is shared and the pins are copied.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            registry: Arc::clone(&self.registry),
        }
    }

    /// How this reader renders: the dictionary it reads against.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        format!("FixCodec({} fields)", self.registry.len())
    }
}

/// How a reader is pinned, where a caller pins it at all.
#[napi(object)]
#[derive(Default)]
pub struct FixCodecOptions {
    /// The dialect every row is read in, rather than the one each row implies.
    pub branch: Option<String>,
    /// The version built messages are expressed in.
    pub version: Option<String>,
    /// The spellings that mean "nothing was sent".
    pub null_values: Option<Vec<String>>,
}

/// Read one FIX version, or report the native parse failure.
fn version_from_js(text: &str) -> Result<CoreVersion> {
    let spelling = text.strip_prefix("FIX.").unwrap_or(text);
    spelling.parse::<CoreVersion>().map_err(napi_error)
}

/// The fixed root every message answers as, built from one dictionary.
///
/// Header, the fields a consumer reads, the groups worth persisting whole, the
/// trailer, this crate's own derived facts, and the two lists that close every
/// row. Columns are named by tag, because a tag is the one name a field has in
/// every version and every dialect.
#[napi(js_name = "fixSchema")]
pub fn fix_schema(
    registry: Option<ClassInstance<'_, JsFixRegistry>>,
    name: Option<String>,
) -> Result<JsField> {
    let registry = match registry {
        Some(held) => Arc::clone(&held.inner),
        None => Arc::clone(CoreFixRegistry::global().map_err(napi_error)?),
    };
    let name = name.unwrap_or_else(|| "fix".to_owned());
    yggdryl::fix_schema(&registry, name)
        .map(JsField::from_core)
        .map_err(napi_error)
}

/// The fixed root behind a capture's own columns.
///
/// `carrier` is a capture's own root - where a line was read from, which line
/// it was, what stamped it - and its columns lead the row, because that is what
/// a monitor orders and joins on. A carried column whose name a FIX column
/// already takes is dropped rather than renamed: the FIX column is the one a
/// reader spelling it means.
#[napi(js_name = "fixSchemaCarrying")]
pub fn fix_schema_carrying(carrier: &JsField, read: &JsField) -> Result<JsField> {
    yggdryl::fix_schema_carrying(&carrier.inner, &read.inner)
        .map(JsField::from_core)
        .map_err(napi_error)
}

/// One row's columns, in order, as tags.
#[allow(clippy::cast_lossless)]
#[napi(js_name = "fixSchemaTags")]
pub fn fix_schema_tags() -> Vec<f64> {
    yggdryl::fix_schema_tags()
        .into_iter()
        .map(f64::from)
        .collect()
}

/// The fields this crate defines on its own branch, in tag order.
#[napi(js_name = "fixCrateFields")]
pub fn fix_crate_fields() -> Result<Vec<JsField>> {
    yggdryl::fix_crate_fields()
        .map(|held| held.iter().cloned().map(JsField::from_core).collect())
        .map_err(napi_error)
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
