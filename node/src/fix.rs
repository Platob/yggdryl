//! Node.js view of the FIX dictionary, its message, and the process default.
//!
//! Nothing here resolves, folds, merges, shards or validates: the registry is
//! one [`Arc`] over the core [`FixRegistry`](CoreFixRegistry), and every
//! accessor coerces its key once at the boundary and redirects to the most
//! specific native method. The typed `fix:` vocabulary is not here either - it
//! lives on the protocol view class [`JsProtocolField`](crate::JsProtocolField),
//! which is what `field.fix` already answers.
//!
//! An identifier crosses as a `number` - the signed 32-bit digest of a
//! field's tag and its name that the core derives - and is read once here
//! through [`id_from_js`], so it gets no class of its own in JavaScript. A
//! bare tag or name uses the core's deterministic best match: a `number`
//! there is a tag, never an identifier, and an identifier is only ever
//! spelled through the `ById` doors. A dictionary's membership is
//! `fix:branches` on the field it contributed to, read on the protocol view;
//! nothing here resolves through it.

mod catalog;
mod plugin;

pub use catalog::{JsFixDefinitionIterator, JsMsgType, JsMsgTypeIterator};
pub use plugin::{JsPlugin, JsPlugins};

use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use napi::JsDate;
use napi::JsValue as _;
use napi::bindgen_prelude::{
    BigInt, Buffer, ClassInstance, Either, Either3, Env, FromNapiValue, Function, FunctionRef,
    Generator, JsObjectValue as _, Null, Object, Result, Status, Unknown, ValueType,
};
use napi_derive::napi;
use yggdryl::{
    DataType as CoreDataType, Error as CoreError, Field as CoreField, FixCategory,
    FixCodec as CoreFixCodec, FixId as CoreFixId, FixKey, FixLifecycle as CoreFixLifecycle,
    FixMsg as CoreFixMsg, FixRegistry as CoreFixRegistry, Scalar, TimeUnit, Timezone,
};

use crate::iobase::{LocationInput, folder_from_input, located_from_input};
use crate::iomedia::JsBatchReader;
use crate::text::codec::JsScalar;
use crate::text_line::{JsFieldPath, JsTextLine, path_from_input};
use crate::types::field::JsField;
use crate::{exact_i32, exact_i64, napi_error, napi_type_error};

/// The root a batch of FIX rows is named by, the core's own spelling.
const ROOT_NAME: &str = "fix";

/// What a mutation says when something else still holds the dictionary.
const SHARED: &str =
    "this registry is shared with a message or installed as the process default; build a new one";

/// Read one identifier, or throw naming what an identifier is.
///
/// An identifier crosses as the signed 32-bit digest `field.fix.id` answers,
/// checked exactly: a fractional or out-of-`i32` number is refused rather
/// than narrowed into a different identity. Nothing else is checked, because
/// an integer read back is whatever was written, and the registry lookup is
/// what says whether a field stands behind it.
pub(crate) fn id_from_js(value: f64) -> Result<CoreFixId> {
    exact_i32(value, "id").map(CoreFixId::from_digest)
}

/// The dictionary a caller named, or the process default where none was.
///
/// Every entry point that resolves against a registry - a message, a reader, a
/// lifecycle, the fixed row - takes the same optional argument and falls back
/// the same way, so the fallback is spelled here once.
fn registry_or_global(
    registry: Option<ClassInstance<'_, JsFixRegistry>>,
) -> Result<Arc<CoreFixRegistry>> {
    match registry {
        Some(held) => Ok(Arc::clone(&held.inner)),
        None => CoreFixRegistry::global()
            .map(Arc::clone)
            .map_err(napi_error),
    }
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
    /// Look up a globally unique group by its scalar counter tag.
    #[napi]
    pub fn get_group_by_tag(&self, tag: f64) -> Result<Option<JsField>> {
        let tag = exact_i32(tag, "tag")?;
        Ok(self
            .inner
            .get_group_by_tag(tag)
            .cloned()
            .map(JsField::from_core))
    }

    /// Look up a globally unique group, failing when absent or ambiguous.
    #[napi]
    pub fn group_by_tag(&self, tag: f64) -> Result<JsField> {
        let tag = exact_i32(tag, "tag")?;
        self.inner
            .group_by_tag(tag)
            .cloned()
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// Look up a category definition, returning null when absent.
    #[napi]
    pub fn get_definition(&self, category: String, name: String) -> Result<Option<JsField>> {
        let category = FixCategory::from_str(&category).map_err(napi_error)?;
        Ok(self
            .inner
            .get_definition(category, &name)
            .cloned()
            .map(JsField::from_core))
    }

    /// Look up a category definition, failing when absent.
    #[napi]
    pub fn definition(&self, category: String, name: String) -> Result<JsField> {
        let category = FixCategory::from_str(&category).map_err(napi_error)?;
        self.inner
            .definition(category, &name)
            .map(|field| JsField::from_core(field.clone()))
            .map_err(napi_error)
    }

    /// Definitions in native category order, retaining the registry while active.
    #[napi]
    pub fn definitions(&self, category: String) -> Result<JsFixDefinitionIterator> {
        let category = FixCategory::from_str(&category).map_err(napi_error)?;
        Ok(JsFixDefinitionIterator {
            registry: Some(Arc::clone(&self.inner)),
            category,
            index: 0,
        })
    }

    /// Fold a named definition into the one its name reaches.
    ///
    /// The lenient counterpart of `createDefinition`, which refuses a name it
    /// holds, and of `insertDefinition`, which replaces one wholesale.
    /// Answers `true` when the definition arrived and `false` when it merged;
    /// `"fields"` redirects to `addField`.
    ///
    /// A merge keeps the stored definition's identity, name and every member
    /// it declares, in its order, and appends the members it lacks - for a
    /// group, to the occurrence inside the list, and to the component when
    /// that occurrence is a component's. It is one level deep: a member both
    /// sides declare stays the stored one, so a member whose datatype - or
    /// whose restated reference - disagrees is refused. Every message and
    /// component referencing the definition sees the appended members.
    ///
    /// One mutation: a refusal leaves the dictionary exactly as it was.
    #[napi]
    pub fn add_definition(&mut self, category: String, field: &JsField) -> Result<bool> {
        let category = FixCategory::from_str(&category).map_err(napi_error)?;
        self.inner_mut()?
            .add_definition(category, field.inner.clone())
            .map_err(napi_error)
    }

    /// Insert or replace a complete native category definition atomically.
    #[napi]
    pub fn insert_definition(
        &mut self,
        category: String,
        field: &JsField,
    ) -> Result<Option<JsField>> {
        let category = FixCategory::from_str(&category).map_err(napi_error)?;
        self.inner_mut()?
            .insert_definition(category, field.inner.clone())
            .map(|field| field.map(JsField::from_core))
            .map_err(napi_error)
    }

    /// Create a definition, refusing an existing identity.
    #[napi]
    pub fn create_definition(&mut self, category: String, field: &JsField) -> Result<()> {
        let category = FixCategory::from_str(&category).map_err(napi_error)?;
        self.inner_mut()?
            .create_definition(category, field.inner.clone())
            .map_err(napi_error)
    }

    /// Replace an existing definition atomically, preserving identity.
    #[napi]
    pub fn update_definition(&mut self, category: String, field: &JsField) -> Result<JsField> {
        let category = FixCategory::from_str(&category).map_err(napi_error)?;
        self.inner_mut()?
            .update_definition(category, field.inner.clone())
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// Remove a definition, refusing dangling references.
    #[napi]
    pub fn remove_definition(&mut self, category: String, name: String) -> Result<Option<JsField>> {
        let category = FixCategory::from_str(&category).map_err(napi_error)?;
        self.inner_mut()?
            .remove_definition(category, &name)
            .map(|field| field.map(JsField::from_core))
            .map_err(napi_error)
    }

    /// Borrow the message singleton named by wire code, canonical name, or alias.
    #[napi]
    pub fn get_msgtype(&self, spelling: String) -> Option<JsMsgType> {
        self.inner
            .get_msgtype(&spelling)
            .map(|message| JsMsgType::from_borrowed(&self.inner, message))
    }

    /// Borrow a message singleton, failing when absent.
    #[napi]
    pub fn msgtype(&self, spelling: String) -> Result<JsMsgType> {
        let message = self.inner.msgtype(&spelling).map_err(napi_error)?;
        Ok(JsMsgType::from_borrowed(&self.inner, message))
    }

    /// Iterate native message singletons in canonical order.
    #[napi]
    pub fn msgtypes(&self) -> JsMsgTypeIterator {
        JsMsgTypeIterator {
            registry: Some(Arc::clone(&self.inner)),
            index: 0,
        }
    }

    /// Add the native scalar plugin fields atomically.
    #[napi]
    pub fn with_plugin_fields(&mut self) -> Result<()> {
        let registry = self
            .inner_mut()?
            .clone()
            .with_plugin_fields()
            .map_err(napi_error)?;
        self.inner = Arc::new(registry);
        Ok(())
    }

    /// A registry holding the built-in definitions.
    ///
    /// Every registry holds the thirty-four scalar fields, the sorted
    /// `altids` Map group and the `instids` Struct that `fixCrateFields`
    /// lists, and the `pluginconfig` component. It also holds the standard
    /// `SendingTime` (52) and `TransactTime` (60) clock fields, seeded where
    /// the dictionary defines no field of its own at those tags. A dictionary
    /// loaded from a store, built from fields or left alone holds them alike;
    /// scalar lookups and `size` exclude groups and components, so a new
    /// registry's `size` is 36.
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

    /// Load the fields, components, and groups categories.
    ///
    /// `location` is an `IOBase` handle, a `Url`, or the string naming one, run
    /// through the coercion every folder-shaped entry point uses. A folder that
    /// is not there loads as a new registry - the crate's own fields and the
    /// two seeded clocks, nothing else - and is not created; a stored copy of
    /// one of the crate's own fields, in its tag block from 65000, is read
    /// past, because the crate's own definition is the one that types a row.
    /// The standard clocks are seeded after the store loads, so a stored
    /// `SendingTime` or `TransactTime` keeps its own metadata. A shard that does
    /// not parse, and a root still holding the retired `records/` layout,
    /// throw with the URL named.
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
    /// the order it spelled them. `dialect` is the membership every field,
    /// group, component and message the file produces is stamped with, on
    /// its `fix:branches` - standard tags included, since membership means
    /// the dictionary speaks it; with none named nothing is stamped. A
    /// dialect that is empty or carries a comma is refused.
    ///
    /// A file this cannot be read from throws the native sentence whole: the
    /// byte the reader stopped at, what was expected, what arrived, and the
    /// element the file spells it in.
    #[napi(ts_return_type = "[FixRegistry, Array<Field>]")]
    pub fn from_cfb_file(
        location: LocationInput<'_>,
        dialect: Option<String>,
    ) -> Result<(Self, Vec<JsField>)> {
        // A CBlock is a file, so the location is held as whichever role it
        // actually is: a container handle reads no bytes, and a reader handed
        // one answers an empty vocabulary instead of a refusal.
        let handle = located_from_input(location)?;
        let (registry, roots) = CoreFixRegistry::from_cfb_file(handle.as_io(), dialect.as_deref())
            .map_err(napi_error)?;
        Ok((
            Self::from_arc(Arc::new(registry)),
            roots.into_iter().map(JsField::from_core).collect(),
        ))
    }

    /// Register a full wire code and borrow its immutable message definition.
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
    ) -> Result<JsMsgType> {
        self.inner_mut()?
            .register_msgtype(&spelling, name.as_deref(), description.as_deref())
            .map_err(napi_error)?;
        self.msgtype(spelling)
    }

    /// Write every populated shard under `<location>/fields/<shard>.json` and
    /// every definition under `<location>/<category>/<name>.json`, removing
    /// the shards and trees no field populates any more. The crate's own
    /// definitions are written like every other - its tag block from 65000 is
    /// one shard, and its `altids`, `instids` and `pluginconfig` are three
    /// documents - so a store states the whole row; a reader takes the
    /// definition it holds from construction over the document it finds.
    #[napi]
    pub fn write_into(&self, location: LocationInput<'_>) -> Result<()> {
        let mut holder = folder_from_input(location)?;
        self.inner.write_into(&mut holder).map_err(napi_error)
    }

    /// How many scalar fields are held, the crate's own thirty-four among
    /// them, 36 for a new registry with its two seeded clocks.
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        u32::try_from(self.inner.len()).unwrap_or(u32::MAX)
    }

    /// The field one identifier names exactly, or `null`.
    ///
    /// `id` is the number `field.fix.id` answers - the digest of a tag and a
    /// name - and the lookup is exact: no tiering, no fold, and a number that
    /// is not a signed 32-bit integer is refused, never a miss.
    #[napi]
    pub fn get_field_by_id(&self, id: f64) -> Result<Option<JsField>> {
        let id = id_from_js(id)?;
        Ok(self
            .inner
            .get_field_by_id(id)
            .cloned()
            .map(JsField::from_core))
    }

    /// The field one identifier names exactly.
    #[napi]
    pub fn field_by_id(&self, id: f64) -> Result<JsField> {
        let id = id_from_js(id)?;
        self.inner
            .field_by_id(id)
            .map(|field| JsField::from_core(field.clone()))
            .map_err(napi_error)
    }

    /// The field a canonical or alternate tag names, or `null`.
    ///
    /// The canonical holder of the tag answers first, then the field holding
    /// it as an alternate.
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
    /// One namespace: the canonical fold answers first, then an alias fold.
    #[napi]
    pub fn get_field_by_name(&self, name: String) -> Option<JsField> {
        self.inner
            .get_field_by_name(&name)
            .cloned()
            .map(JsField::from_core)
    }

    /// The field a canonical name or alias names, ASCII case folded.
    #[napi]
    pub fn field_by_name(&self, name: String) -> Result<JsField> {
        self.inner
            .field_by_name(&name)
            .map(|field| JsField::from_core(field.clone()))
            .map_err(napi_error)
    }

    /// The field a dotted path reaches through a component or a group, or `null`.
    #[napi(ts_args_type = "path: string | FieldPath")]
    pub fn get_field_by_path(&self, path: Either<String, &JsFieldPath>) -> Result<Option<JsField>> {
        let path = path_from_input(path)?;
        Ok(self
            .inner
            .get_field_by_path(&path)
            .cloned()
            .map(JsField::from_core))
    }

    /// The field a path reaches through a component or a group.
    ///
    /// A position is spelled the way the one grammar spells it -
    /// `Parties[0].PartyID` - and a schema answers the item every occurrence
    /// of a group holds, so that spelling reaches the member here as well as
    /// in a message.
    #[napi(ts_args_type = "path: string | FieldPath")]
    pub fn field_by_path(&self, path: Either<String, &JsFieldPath>) -> Result<JsField> {
        let path = path_from_input(path)?;
        self.inner
            .field_by_path(&path)
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

    /// Fold one field in, adding it when absent and merging it when stored.
    ///
    /// The lenient counterpart of `insert`, which replaces, and of `update`,
    /// which refuses everything new. Answers `true` when the field arrived
    /// and `false` when it folded into a stored one: the same tag under the
    /// same folded name merges; a name folding to a stored canonical name or
    /// alias under another tag merges into that field - aliases, alternate
    /// tags and membership become the union and the incoming canonical tag
    /// joins the alternates unless another field answers it; the same tag
    /// under another name is added beside the holder, which gains the name
    /// as an alias while the bare tag keeps answering it; a nested field is
    /// redirected to `addDefinition` under the category its shape names, and
    /// one of this crate's own tags is skipped as already held.
    ///
    /// One mutation: a refusal - no `fix:tag`, a datatype disagreeing with
    /// the stored field - leaves the dictionary exactly as it was.
    #[napi]
    pub fn add_field(&mut self, field: &JsField) -> Result<bool> {
        self.inner_mut()?
            .add_field(field.inner.clone())
            .map_err(napi_error)
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

    /// Merge a definition into the stored field with the same identity: the
    /// same tag under the same folded name.
    #[napi]
    pub fn update(&mut self, field: &JsField) -> Result<()> {
        let field = field.inner.clone();
        self.inner_mut()?.update(field).map_err(napi_error)
    }

    /// Remove the field a tag or a name reaches, answering it.
    #[napi(ts_args_type = "key: number | string")]
    pub fn remove(&mut self, env: Env, key: Unknown<'_>) -> Result<Option<JsField>> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        Ok(self
            .inner_mut()?
            .remove(key.as_key())
            .map(JsField::from_core))
    }

    /// Remove the field one identifier names exactly, answering it.
    ///
    /// The generic `remove` reads a number as a tag, so this is the spelling
    /// that reaches one of two fields sharing a tag by its own identity;
    /// `id` is read exactly as every other identifier argument is.
    #[napi]
    pub fn remove_by_id(&mut self, id: f64) -> Result<Option<JsField>> {
        let id = id_from_js(id)?;
        Ok(self.inner_mut()?.remove(id).map(JsField::from_core))
    }

    /// The distinct dictionaries any field or definition names on its
    /// `fix:branches`, sorted.
    ///
    /// Membership is provenance and this is its listing; nothing resolves
    /// through it. A registry holding only the specification's own fields
    /// answers an empty array.
    #[napi]
    pub fn dialects(&self) -> Vec<String> {
        self.inner.dialects()
    }

    /// The fields in ascending identifier order, lazily.
    ///
    /// The order is the core's: tag-major, then by identifier. The iterator holds
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

    /// Whether two registries hold the same fields, in identifier order.
    #[napi]
    pub fn equals(&self, other: &JsFixRegistry) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner) || self.inner == other.inner
    }

    /// Deterministic hash bits over all categories.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
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

    /// A complete native catalog snapshot: the three categories.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> Result<serde_json::Value> {
        serde_json::from_str(&self.inner.into_json().map_err(napi_error)?).map_err(napi_error)
    }

    /// Load a complete native catalog snapshot.
    #[napi(factory)]
    pub fn from_json(input: String) -> Result<Self> {
        CoreFixRegistry::from_json(&input)
            .map(|registry| Self::from_arc(Arc::new(registry)))
            .map_err(napi_error)
    }

    /// Render a complete native catalog snapshot.
    #[napi]
    pub fn into_json(&self) -> Result<String> {
        self.inner.into_json().map_err(napi_error)
    }
}

impl Default for JsFixRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// The fields of a registry, in ascending identifier order.
///
/// Answered by `keys()`. It advances with the core's own cursor - the registry
/// plus the last `FixId` it answered - so taking one field from a dictionary of
/// thousands costs one lookup, and a walk crosses every field in the one order
/// the core iterates: tag-major, then identifier. It lets the registry go the moment the walk ends, because
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
/// A tag is an `i32` and every `i32` is an exact `f64`, so the number
/// JavaScript reads is the tag rather than a rounding of it. A key the
/// dictionary did not resolve - a name or a number alike - carries tag 0
/// beside its raw key.
fn flatten_entries(entries: &[yggdryl::FixEntry], out: &mut Vec<(f64, String, String)>) {
    for entry in entries {
        out.push((
            f64::from(entry.tag()),
            entry.key().as_str().unwrap_or_default().to_owned(),
            entry.value().as_str().unwrap_or_default().to_owned(),
        ));
        flatten_entries(entry.children(), out);
    }
}

/// A FIX message: a value plus the registry that types it.
///
/// The schema is one non-null Struct `Field` - the only row schema - and the
/// value the row it declares, so a plain object crosses as the record the core
/// canonicalizes into that order exactly as every other row is. The row is
/// written through `set` and `remove`; the entries never are, because they are
/// what the wire carried. The message compares, hashes, renders and clones by
/// the schema and the value it carries, against the registry it was resolved
/// against.
///
/// Every message carries the settled replay fields, never null: `updatedat`,
/// `createdat`, `msghash`, `msgphash`, `code`, `snapshotat` and `SendingTime` (52).
/// The four the readers of the same names answer are held beside the row, so
/// reading them costs no lookup.
#[napi(js_name = "FixMsg")]
pub struct JsFixMsg {
    inner: CoreFixMsg,
}

impl JsFixMsg {
    /// Wrap a message the core built.
    pub(crate) const fn from_core(inner: CoreFixMsg) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsFixMsg {
    /// Build a message, linking the process default when none is named.
    ///
    /// The loader widens `value`: anything `Scalar.fromJs` reads becomes the
    /// native value first, and the core alone validates and canonicalizes it
    /// against `field`. A mandatory replay field the root lacks is appended:
    /// `SendingTime` reads UTC now when the value states none, `snapshotat`
    /// is `TransactTime` (60) else `SendingTime`, `updatedat` and `createdat`
    /// default to that instant, `code` to the empty name, and `msghash` and
    /// `msgphash` are computed. A stated identity must match what is computed.
    #[napi(constructor)]
    pub fn new(
        field: &JsField,
        value: &JsScalar,
        registry: Option<ClassInstance<'_, JsFixRegistry>>,
    ) -> Result<Self> {
        let registry = registry_or_global(registry)?;
        CoreFixMsg::with_registry(registry, field.inner.clone(), value.inner.clone())
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// The message a fixed row holds: the inverse of `intoRow`.
    ///
    /// `schema` is the row's root - the one `fix.schema` or
    /// `fix.schemaCarrying` built, or a batch reader's `field` - and `row` the
    /// row under it, which the loader widens from whatever `Scalar.fromJs`
    /// reads. The columns are the message's children under the schema's
    /// names, reached by tag as a parsed message's are, and the entries are
    /// rebuilt from the `fixentries` column, so `intoBytes` re-emits the
    /// line the row was read from; a row without that column has no entries.
    /// Nothing is parsed again and no clock is read: the row must carry the
    /// seven non-null replay fields - `updatedat`, `createdat`, `msghash`,
    /// `msgphash`, `code`, `snapshotat`, `SendingTime` - and its `msghash` and
    /// `msgphash` must match what its content computes, or it throws the located
    /// refusal. The process default is the registry when none is named.
    #[napi(factory)]
    pub fn from_row(
        schema: &JsField,
        row: &JsScalar,
        registry: Option<ClassInstance<'_, JsFixRegistry>>,
    ) -> Result<Self> {
        let registry = registry_or_global(registry)?;
        CoreFixMsg::from_row(registry, &schema.inner, &row.inner)
            .map(Self::from_core)
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

    /// The value of the root child an identifier names, or `null`.
    ///
    /// An identifier is exact and does not fold: `id` is the number
    /// `field.fix.id` answers, and a field the dictionary does not hold
    /// under it simply misses.
    #[napi]
    pub fn get_by_id(&self, id: f64) -> Result<Option<JsScalar>> {
        let id = id_from_js(id)?;
        Ok(self.inner.get_by_id(id).cloned().map(JsScalar::from_core))
    }

    /// The value of the root child an identifier names.
    #[napi]
    pub fn by_id(&self, id: f64) -> Result<JsScalar> {
        let id = id_from_js(id)?;
        self.inner
            .by_id(id)
            .map(|value| JsScalar::from_core(value.clone()))
            .map_err(napi_error)
    }

    /// The value of the root child a tag names, or `null`.
    ///
    /// The tag resolves through the dictionary: the canonical holder first,
    /// then an alternate.
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
    /// The name folds through the dictionary: the canonical spelling first,
    /// then an alias.
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

    /// The value a path reaches, or `null`.
    #[napi(ts_args_type = "path: string | FieldPath")]
    pub fn get_by_path(&self, path: Either<String, &JsFieldPath>) -> Result<Option<JsScalar>> {
        let path = path_from_input(path)?;
        Ok(self
            .inner
            .get_by_path(&path)
            .cloned()
            .map(JsScalar::from_core))
    }

    /// The value a path reaches.
    ///
    /// A position is spelled the way the one grammar spells it:
    /// `Parties[0].PartyID`.
    #[napi(ts_args_type = "path: string | FieldPath")]
    pub fn by_path(&self, path: Either<String, &JsFieldPath>) -> Result<JsScalar> {
        let path = path_from_input(path)?;
        self.inner
            .by_path(&path)
            .map(|value| JsScalar::from_core(value.clone()))
            .map_err(napi_error)
    }

    /// The value a tag or a name reaches, or `null`.
    #[napi(ts_args_type = "key: number | string")]
    pub fn get(&self, env: Env, key: Unknown<'_>) -> Result<Option<JsScalar>> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        Ok(self
            .inner
            .get(key.as_key())
            .cloned()
            .map(JsScalar::from_core))
    }

    /// The value a tag or a name reaches.
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

    /// Writes one value into the row, typed by the field the key resolves to.
    ///
    /// `key` is a tag or a name, resolved as a lookup resolves one - through
    /// the dictionary, canonical before alternate or alias - and a name the
    /// dictionary does not know still reaches a child spelled that way. `value` is whatever `Scalar.fromJs` reads, widened by the
    /// loader; a known field types it through the core's value contract, and
    /// `null` is stored as a stated null. An existing child is replaced where
    /// it stands and an absent one appended; a bare tag no dictionary explains
    /// appends a text child named by its decimal. Only the row changes: the
    /// entries, the wire and the digest stay what they were, while `msghash` and
    /// `msgphash` are recomputed from the new content. No clock is read.
    ///
    /// A key reaching no field and no child, or a value the field refuses,
    /// throws the core's refusal and leaves the message as it was.
    #[napi(ts_args_type = "key: number | string, value: unknown")]
    pub fn set(&mut self, env: Env, key: Unknown<'_>, value: &JsScalar) -> Result<()> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        self.inner
            .set(key.as_key(), value.inner.clone())
            .map_err(napi_error)
    }

    /// Removes the child a key reaches, answering its value, or `null`.
    ///
    /// The key resolves as `set` resolves one, and a key reaching nothing
    /// answers `null` and changes nothing. The entries are untouched.
    ///
    /// A mandatory replay field - `updatedat`, `createdat`, `msghash`, `msgphash`,
    /// `code`, `snapshotat` or `SendingTime` - refuses removal and throws,
    /// leaving the message exactly as it was; so does any other refusal.
    #[napi(ts_args_type = "key: number | string")]
    pub fn remove(&mut self, env: Env, key: Unknown<'_>) -> Result<Option<JsScalar>> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        Ok(self
            .inner
            .remove(key.as_key())
            .map_err(napi_error)?
            .map(JsScalar::from_core))
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

    /// The settled message instant, `DateTime64(ns, UTC)`, never null.
    ///
    /// Settled once, when the message is first built: a valid stated
    /// `updatedat`, else the event instant `snapshotat` holds. A lifecycle
    /// truncates it to its snapshot grid. Mutation and enrichment carry it
    /// unless it is written explicitly; no clock is read after intake.
    #[napi]
    pub fn updatedat(&self) -> JsScalar {
        JsScalar::from_core(self.inner.updatedat().clone())
    }

    /// The settled creation instant, `DateTime64(ns, UTC)`, never null.
    ///
    /// A valid stated `createdat`, else the event instant; a lifecycle
    /// carries the first creation instant of the live chain a message joins.
    #[napi]
    pub fn createdat(&self) -> JsScalar {
        JsScalar::from_core(self.inner.createdat().clone())
    }

    /// The message's time/content identity, never null.
    ///
    /// Sixteen `fixedbinary(16)` bytes - a `Buffer` in JavaScript:
    /// `updatedat`'s signed nanoseconds with the sign bit flipped in bytes
    /// 0..8, then all 64 bits of the canonical named content's XXH64;
    /// `updatedat`, `createdat`, `msghash` itself and the arrival record are not
    /// content. A stated `msghash` must match it.
    #[napi]
    pub fn msghash(&self) -> JsScalar {
        JsScalar::from_core(self.inner.msghash().clone())
    }

    /// The event chain's identity, never null.
    ///
    /// The sixteen big-endian `fixedbinary(16)` bytes of the XXH3-128 of the
    /// exact `code` bytes alone - a `Buffer` in JavaScript - so the empty
    /// (unknown) code has one deterministic `msgphash` too. A stated `msgphash`
    /// must match it.
    #[napi]
    pub fn msgphash(&self) -> JsScalar {
        JsScalar::from_core(self.inner.msgphash().clone())
    }

    /// One lifted facet's value, or `null` where nothing carries it.
    #[napi]
    pub fn lifted(&self, facet: String) -> Option<JsScalar> {
        self.inner.lifted(&facet).cloned().map(JsScalar::from_core)
    }

    /// The tag a lifted facet was read from, or `null`.
    ///
    /// A lift source is declared by tag, and the tag is the whole of what it
    /// is; the field that tag names is the registry's to answer.
    #[napi]
    pub fn lift_source(&self, facet: String) -> Option<i32> {
        self.inner.lift_source(&facet)
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
    /// heads them, so a caller reading the array reads the wire. Each entry
    /// is `[tag, key, value]`; tag 0 marks a key the dictionary did not
    /// resolve, whether it arrived as a name or as a number, and its raw key
    /// is kept.
    #[napi(ts_return_type = "Array<[number, string, string]>")]
    pub fn arrivals(&self) -> Vec<(f64, String, String)> {
        let mut held = Vec::new();
        flatten_entries(self.inner.entries(), &mut held);
        held
    }

    /// This message as the fixed row a table holds.
    ///
    /// `schema` is the fixed root `fixSchema` builds: every column is filled by
    /// the tag its field carries - never by its spelling - so a message that
    /// carried nothing at a column answers null there rather than shifting its
    /// neighbours. A column no tag names is the capture's: it takes the child
    /// of that name where the message has one, else null. The arrival record
    /// closes the row under `fixentries`, unresolved keys at tag 0.
    ///
    /// A replayable row keeps the seven replay fields; a schema missing or
    /// mistyping one, or a cell its column cannot represent, throws the
    /// located refusal. No clock is read.
    #[napi]
    pub fn into_row(&self, schema: &JsField) -> Result<JsScalar> {
        self.inner
            .into_row(&schema.inner)
            .map(JsScalar::from_core)
            .map_err(napi_error)
    }

    /// Re-emit this message on the wire, separated by `separator`.
    #[napi]
    pub fn into_bytes(&self, separator: Option<f64>) -> Result<Buffer> {
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
        self.inner.stable_hash()
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

/// Where a JavaScript source behind a stream failed, kept for the stream.
///
/// A core stage pulls its items as values, so a failure inside the pull - an
/// item the loader could not read, an iterator that threw - cannot travel
/// through the stage as an item. It ends the pull instead and lands here, as
/// the status and reason a new error is raised with, because the error
/// itself holds handles that do not cross threads.
#[derive(Clone, Default)]
struct Failed(Arc<Mutex<Option<(Status, String)>>>);

impl Failed {
    fn set(&self, error: &napi::Error) {
        if let Ok(mut held) = self.0.lock() {
            *held = Some((error.status, error.reason.clone()));
        }
    }

    fn take(&self) -> Option<napi::Error> {
        self.0
            .lock()
            .ok()
            .and_then(|mut held| held.take())
            .map(|(status, reason)| napi::Error::new(status, reason))
    }
}

/// A JavaScript iterable pulled one item at a time into a core stage.
///
/// The loader hands over a bound pull function, `() => item | null`, so the
/// iterable's own protocol runs in JavaScript and each item arrives here
/// already read into the value the stage takes. Like the record bridge in
/// `iomedia`, it is called only on the isolate thread that supplied it and
/// only within the native call advancing the stream, which is what makes the
/// environment it is borrowed back with valid. Exhaustion ends the pull; a
/// failure ends it too and lands in `failed`, so the stage sees a shorter
/// stream and the wrapper around it throws what happened.
struct Pulled<T: FromNapiValue> {
    pull: FunctionRef<(), Option<T>>,
    environment: usize,
    thread: ThreadId,
    failed: Failed,
    done: bool,
}

impl<T: FromNapiValue> Pulled<T> {
    fn new(env: Env, pull: Function<'_, (), Option<T>>) -> Result<Self> {
        Ok(Self {
            pull: pull.create_ref()?,
            environment: env.raw().expose_provenance(),
            thread: std::thread::current().id(),
            failed: Failed::default(),
            done: false,
        })
    }

    fn pull(&self) -> Result<Option<T>> {
        if std::thread::current().id() != self.thread {
            return Err(napi_error(
                "a JavaScript message iterable can only be pulled on the isolate thread that supplied it",
            ));
        }
        let env = Env::from_raw(std::ptr::with_exposed_provenance_mut(self.environment));
        self.pull.borrow_back(&env)?.call(())
    }
}

impl<T: FromNapiValue> Iterator for Pulled<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        if self.done {
            return None;
        }
        match self.pull() {
            Ok(Some(value)) => Some(value),
            Ok(None) => {
                self.done = true;
                None
            }
            Err(error) => {
                self.done = true;
                self.failed.set(&error);
                None
            }
        }
    }
}

/// A JavaScript failure behind a stream a batch reader pulls, as the core
/// reports one.
///
/// The batch it would have landed in is being pulled by the reader rather
/// than by a JavaScript frame, so it travels as that reader's error and
/// arrives where the batch would have.
fn javascript_failure(error: napi::Error) -> CoreError {
    CoreError::Arrow(arrow_schema::ArrowError::ExternalError(Box::new(
        std::io::Error::other(error.reason.clone()),
    )))
}

/// A stream of messages, one at a time.
///
/// Every stage of the codec answers one of these - one line's messages, a
/// stream of lines parsed, records parsed, messages filled or stamped, a
/// batch read back - so a message stream has one shape at this boundary
/// whatever made it. Nothing is collected: the core iterator is the stream,
/// and a JavaScript iterable behind it is pulled one item at a time. A line
/// the reader refuses throws where it is met and the stream goes on past it;
/// a failure in the iterable behind the stream throws and ends it. The loader
/// supplies `Symbol.iterator` over `next`.
#[napi(js_name = "FixMessages")]
pub struct JsFixMessages {
    inner: Box<dyn Iterator<Item = yggdryl::Result<CoreFixMsg>>>,
    /// Where the JavaScript source behind `inner` failed, when there is one.
    failed: Option<Failed>,
}

impl JsFixMessages {
    /// A stream over a core iterator that pulls nothing from JavaScript.
    pub(super) fn over<I>(inner: I) -> Self
    where
        I: Iterator<Item = yggdryl::Result<CoreFixMsg>> + 'static,
    {
        Self {
            inner: Box::new(inner),
            failed: None,
        }
    }

    /// A stream over a core stage fed by a JavaScript iterable.
    fn pulling<I>(inner: I, failed: Failed) -> Self
    where
        I: Iterator<Item = yggdryl::Result<CoreFixMsg>> + 'static,
    {
        Self {
            inner: Box::new(inner),
            failed: Some(failed),
        }
    }
}

#[napi]
impl JsFixMessages {
    /// Advance the stream: the next message, or `null` at its end.
    ///
    /// A line the reader refused throws here and the stream continues on
    /// the next call; a failure behind the stream throws once, in place of
    /// the end.
    #[allow(clippy::should_implement_trait)] // JavaScript's iterator adapter needs a throwing next().
    #[napi(ts_return_type = "IteratorResult<FixMsg>")]
    pub fn next(&mut self) -> Result<Option<JsFixMsg>> {
        match self.inner.next() {
            Some(held) => held.map(JsFixMsg::from_core).map(Some).map_err(napi_error),
            None => match self.failed.as_ref().and_then(Failed::take) {
                Some(error) => Err(error),
                None => Ok(None),
            },
        }
    }
}

/// A JavaScript sink with `write(chunk)`, as the writer the core writes into.
///
/// Bound once, called per line, on the thread and within the call that
/// supplied it. The first failure is kept and every write after it refuses,
/// so the core stops and the failure is what the caller sees.
struct JsSink<'env> {
    sink: Object<'env>,
    write: Function<'env, Buffer, Unknown<'env>>,
    failed: Option<napi::Error>,
}

impl<'env> JsSink<'env> {
    fn new(sink: Object<'env>) -> Result<Self> {
        let write: Function<'env, Buffer, Unknown<'env>> =
            sink.get_named_property("write").map_err(|error| {
                napi_error(format!(
                    "expected a sink defining write(chunk: Uint8Array): {error}"
                ))
            })?;
        Ok(Self {
            sink,
            write,
            failed: None,
        })
    }
}

impl std::io::Write for JsSink<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.failed.is_some() {
            return Err(std::io::Error::other("the sink already failed"));
        }
        match self.write.apply(self.sink, Buffer::from(bytes.to_vec())) {
            Ok(_) => Ok(bytes.len()),
            Err(error) => {
                let reason = error.reason.clone();
                self.failed = Some(error);
                Err(std::io::Error::other(reason))
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// One dictionary, reading captured lines into messages, with the Arrow twins.
///
/// The codec is the whole parse surface: a captured line with a verb in front
/// of it, a bare frame, a numeric frame with a stated separator, a bridge's
/// name/value text, a configuration document, pairs a caller already split,
/// a record a text reader answered. Each redirects to the core method of the
/// same name, so nothing here decides a version or a separator - it only
/// carries what JavaScript said across. A stage is a call: the
/// stream methods take any iterable and answer a lazy `FixMessages`, the
/// Arrow methods take and answer a `BatchReader`, one batch at a time.
///
/// Every message it builds opens with `beginstring` - the wire's own, else
/// the version the message was read at - and carries the settled replay
/// fields: `SendingTime` is the message's valid tag 52, else the carrier's,
/// else `defaultSendingTime`, else UTC now read once for that new message;
/// `snapshotat` is `TransactTime` (60) else `SendingTime`; `updatedat` and
/// `createdat` default to that instant; `code`, `msghash` and `msgphash` follow.
/// None of them is an entry unless the line carried it, so `intoBytes`
/// re-emits the line byte for byte. Parsing undated bytes without a default
/// sending time is deliberately not deterministic.
#[napi(js_name = "FixCodec")]
pub struct JsFixCodec {
    inner: CoreFixCodec,
    registry: Arc<CoreFixRegistry>,
}

#[napi]
impl JsFixCodec {
    /// Open a codec over one dictionary, or over the process default.
    ///
    /// Every pin is the core's, spelled once here. `separator` is the byte a
    /// numeric frame splits on where
    /// the line does not say; `payloadColumn` names the batch column a line
    /// is read from; `captureNames` are what a run's row-header captures are
    /// called, in the order a line answers them, which is what lets
    /// `parseTextLine` read a capture by position rather than by name;
    /// `nullValues` are the spellings that mean nothing was
    /// sent; `direction` is the code of tag 385's set an unmarked line
    /// takes on the batch door, any spelling of one - `"S"`, `"Send"`,
    /// `"R"` - the core's `Send` code when unstated and no pin at all when
    /// empty; `batchByteSize` is the raw bytes one Arrow batch targets, the
    /// core's 128 MiB when unstated; `defaultSendingTime` is the
    /// `SendingTime` an undated message takes when neither it nor its carrier
    /// states one - a `Scalar` crosses as it is and must already be
    /// `DateTime64(ns, UTC)`, a `Date` is its UTC millisecond instant restated
    /// in nanoseconds, and `null` or absence reads UTC now per new message.
    #[napi(constructor)]
    pub fn new(
        registry: Option<ClassInstance<'_, JsFixRegistry>>,
        options: Option<FixCodecOptions<'_>>,
    ) -> Result<Self> {
        let registry = registry_or_global(registry)?;
        let options = options.unwrap_or_default();
        let mut inner = CoreFixCodec::new(Arc::clone(&registry));
        if let Some(held) = options.separator {
            inner = inner.with_separator(separator_byte(Some(held))?);
        }
        if let Some(held) = options.payload_column {
            inner = inner.with_payload_column(held);
        }
        if let Some(held) = options.capture_names {
            inner = inner.with_capture_names(held);
        }
        if let Some(held) = options.null_values {
            inner = inner.with_null_values(held);
        }
        if let Some(held) = &options.direction {
            inner = inner.try_with_direction(Some(held)).map_err(napi_error)?;
        }
        if let Some(held) = options.batch_byte_size {
            let bytes = exact_i64(held, "batchByteSize")?;
            let bytes = u64::try_from(bytes)
                .map_err(|_| napi_error("batchByteSize must not be negative"))?;
            inner = inner.with_batch_byte_size(bytes);
        }
        // `null` states "no default" as leaving the option out does.
        let sending_time = match options.default_sending_time {
            Some(Either3::A(scalar)) => Some(Either::A(scalar)),
            Some(Either3::B(date)) => Some(Either::B(date)),
            Some(Either3::C(_)) | None => None,
        };
        if let Some(held) = sending_time {
            inner = inner
                .try_with_default_sending_time(Some(sending_time_from_js(held)?))
                .map_err(napi_error)?;
        }
        Ok(Self { inner, registry })
    }

    /// The dictionary this codec resolves against, sharing it.
    #[napi(getter)]
    pub fn registry(&self) -> JsFixRegistry {
        JsFixRegistry::from_arc(Arc::clone(&self.registry))
    }

    /// The byte a numeric frame splits on, or `null` where the line decides.
    #[napi(getter)]
    pub fn separator(&self) -> Option<u32> {
        self.inner.separator().map(u32::from)
    }

    /// The record column a line is read from.
    #[napi(getter)]
    pub fn payload_column(&self) -> String {
        self.inner.payload_column().to_owned()
    }

    /// The spellings that mean nothing was sent.
    #[napi(getter)]
    pub fn null_values(&self) -> Vec<String> {
        self.inner.null_values().to_vec()
    }

    /// The code of tag 385's set an unmarked line takes on the batch door,
    /// or `null` where no pin fills silence.
    #[napi(getter)]
    pub fn direction(&self) -> Option<String> {
        self.inner.direction().map(ToOwned::to_owned)
    }

    /// The raw bytes one Arrow batch targets.
    ///
    /// A byte count is a JavaScript number, exact to 2^53, as every count
    /// at this boundary is.
    #[allow(clippy::cast_precision_loss)]
    #[napi(getter)]
    pub fn batch_byte_size(&self) -> f64 {
        self.inner.batch_byte_size() as f64
    }

    /// The `SendingTime` an undated message takes, `DateTime64(ns, UTC)`, or
    /// `null` where each new undated message reads UTC now.
    #[napi(getter)]
    pub fn default_sending_time(&self) -> Option<JsScalar> {
        self.inner
            .default_sending_time()
            .cloned()
            .map(JsScalar::from_core)
    }

    /// The complete raw message code declared by captured bytes.
    #[napi]
    pub fn infer_msgtype_bytes(body: Buffer) -> Option<Buffer> {
        CoreFixCodec::infer_msgtype_bytes(&body).map(|value| Buffer::from(value.to_vec()))
    }

    /// The complete raw message code declared by captured text.
    #[napi]
    pub fn infer_msgtype_text(body: String) -> Option<String> {
        CoreFixCodec::infer_msgtype_text(&body).map(ToOwned::to_owned)
    }

    /// One captured line, whatever it is wrapped in: its messages.
    #[napi]
    pub fn parse_line(&self, row: Buffer) -> Result<JsFixMessages> {
        self.inner
            .parse_line(&row)
            .map(JsFixMessages::over)
            .map_err(napi_error)
    }

    /// A stream of captured lines, lazily: each as `parseLine` reads it.
    ///
    /// The loader turns the iterable into the pull function this takes, so
    /// a file of ten million lines costs one line at a time. A line that is
    /// not a row at all throws where it is met and the stream continues past
    /// it; an item that is not bytes throws and ends it.
    #[napi(js_name = "_parseLinesNative", skip_typescript)]
    pub fn parse_lines_native(
        &self,
        env: Env,
        pull: Function<'_, (), Option<Buffer>>,
    ) -> Result<JsFixMessages> {
        let pulled = Pulled::new(env, pull)?;
        let failed = pulled.failed.clone();
        Ok(JsFixMessages::pulling(
            self.inner.parse_lines(pulled),
            failed,
        ))
    }

    /// One numeric frame, read by the pairs it states.
    #[napi]
    pub fn parse_fix_line(&self, body: Buffer) -> Result<JsFixMsg> {
        self.inner
            .parse_fix_line(&body)
            .map(JsFixMsg::from_core)
            .map_err(napi_error)
    }

    /// One bridge frame, whose keys are names rather than tags.
    #[napi]
    pub fn parse_ullink_line(&self, body: Buffer) -> Result<JsFixMsg> {
        self.inner
            .parse_ullink_line(&body)
            .map(JsFixMsg::from_core)
            .map_err(napi_error)
    }

    /// One FIXML row, whose fields are XML attributes.
    #[napi]
    pub fn parse_fixml_line(&self, body: Buffer) -> Result<JsFixMsg> {
        self.inner
            .parse_fixml_line(&body)
            .map(JsFixMsg::from_core)
            .map_err(napi_error)
    }

    /// One bridge configuration document, as a Jolokia answer states it:
    /// one message per `ObjectName` it names, lazily, and none where it names
    /// no configuration.
    #[napi]
    pub fn parse_plugin_line(&self, body: Buffer) -> JsFixMessages {
        JsFixMessages::over(self.inner.parse_plugin_line(&body))
    }

    /// Pairs a caller already holds, in the order they arrived.
    #[napi(ts_args_type = "pairs: Array<[string, string]>")]
    pub fn parse_pairs(&self, pairs: Vec<(String, String)>) -> Result<JsFixMsg> {
        let borrowed: Vec<(&[u8], &[u8])> = pairs
            .iter()
            .map(|(key, value)| (key.as_bytes(), value.as_bytes()))
            .collect();
        self.inner
            .parse_pairs(borrowed)
            .map(JsFixMsg::from_core)
            .map_err(napi_error)
    }

    /// One line a text reader answered: its messages.
    ///
    /// The line's body is the bytes read, and its row-header captures state
    /// the rest - the plugin that logged it, the version, and every field a
    /// capture's name reaches. The line's timestamp is capture context only:
    /// it stamps no FIX clock. `SendingTime` is the message's own, else a
    /// capture reaching that field, else the codec's `defaultSendingTime`,
    /// else UTC now.
    /// `withCaptureNames` is what decides which capture is which, once for
    /// the whole run, because a line answers its captures by position.
    ///
    /// A `pluginid` capture fills the crate's own `pluginid` field and
    /// selects nothing: the dictionary is one namespace.
    ///
    /// A `direction` capture is named so it cannot silently fill a field of
    /// that name, and is not otherwise read: only `parseTextArrowReader` has
    /// a column to put a direction in.
    #[napi(ts_args_type = "line: TextLine")]
    pub fn parse_text_line(&self, line: &JsTextLine) -> Result<JsFixMessages> {
        self.inner
            .parse_text_line(line.as_core())
            .map(JsFixMessages::over)
            .map_err(napi_error)
    }

    /// A stream of lines, lazily: each as `parseTextLine` reads it.
    #[napi(js_name = "_parseTextLinesNative", skip_typescript)]
    pub fn parse_text_lines_native(
        &self,
        env: Env,
        pull: Function<'_, (), Option<ClassInstance<'static, JsTextLine>>>,
    ) -> Result<JsFixMessages> {
        let pulled = Pulled::new(env, pull)?;
        let failed = pulled.failed.clone();
        let lines = pulled.map(|line| line.as_core().clone());
        Ok(JsFixMessages::pulling(
            self.inner.parse_text_lines(lines),
            failed,
        ))
    }

    /// A stream of Arrow batches of capture rows as batches of FIX rows.
    ///
    /// The schema is decided before the first row: the capture's own columns
    /// lead and the fixed FIX columns follow. Every row is parsed as the
    /// line door parses one, and batches close on the raw bytes of
    /// the payload column against `batchByteSize`. The source is consumed.
    #[napi]
    pub fn parse_text_arrow_reader(&self, source: &mut JsBatchReader) -> Result<JsBatchReader> {
        let parsed = self
            .inner
            .parse_text_arrow_reader(source.take()?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(parsed, ROOT_NAME))
    }

    /// Fills what one message implies but did not carry.
    ///
    /// Restatement is the pass's first step rather than a door of its own:
    /// every rule below it reads by tag, and a child stored under an alias
    /// has no tag until the registry's field has canonicalized it, so the row
    /// comes back at the dictionary's newest version.
    ///
    /// An order stating `OrderQty` and `CumQty` has said what `LeavesQty` is.
    /// Only the row is filled: the arrival record is what the wire carried and
    /// is left alone, so `intoBytes` re-emits the received line either way, and
    /// a stated value is never replaced.
    /// A known message also fills a sorted `altids` map from its direct
    /// identifiers, without flattening groups or replacing a stated map,
    /// including an empty one. Unknown types gain no map. A scalar identifier
    /// that cannot convert to UTF-8 raises the native located refusal.
    #[napi]
    pub fn enrich_message(&self, message: &JsFixMsg) -> Result<JsFixMsg> {
        self.inner
            .enrich_message(message.inner.clone())
            .map(JsFixMsg::from_core)
            .map_err(napi_error)
    }

    /// Fills a stream of messages, lazily, remembering what it passes.
    ///
    /// What the stream remembers is every `pluginconfig` it passes, by the
    /// plugin's `Name`: a later message naming that plugin takes its
    /// `SenderCompID` and `TargetCompID` where it stated none of its own. A
    /// bridge says a session's two ends once, in the configuration it printed
    /// at startup, and every line after it names only the plugin. The memory
    /// dies with the iterator, and `enrichMessage` - one message, not a
    /// stream - has none.
    #[napi(js_name = "_enrichMessagesNative", skip_typescript)]
    pub fn enrich_messages_native(
        &self,
        env: Env,
        pull: Function<'_, (), Option<ClassInstance<'static, JsFixMsg>>>,
    ) -> Result<JsFixMessages> {
        let pulled = Pulled::new(env, pull)?;
        let failed = pulled.failed.clone();
        let messages = pulled.map(|message| message.inner.clone());
        Ok(JsFixMessages::pulling(
            self.inner.enrich_messages(messages),
            failed,
        ))
    }

    /// Fills a stream of batches of FIX rows with what each message implies.
    ///
    /// `enrichMessages` over batches: each row is a message through
    /// `FixMsg.fromRow`, filled, and written back under the **same** schema,
    /// so a carried column returns to its place and the arrival record is
    /// untouched. Nothing is parsed again. The source is consumed.
    #[napi]
    pub fn enrich_messages_arrow_reader(
        &self,
        source: &mut JsBatchReader,
    ) -> Result<JsBatchReader> {
        let filled = self
            .inner
            .enrich_messages_arrow_reader(source.take()?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(filled, ROOT_NAME))
    }

    /// A stream of batches of FIX rows as the stream of messages it holds.
    ///
    /// Each row is one message through `FixMsg.fromRow` under the source's
    /// schema, lazily, one batch held at a time: a batch
    /// `parseTextArrowReader` wrote comes back as the messages that made it
    /// without a parse. One half of what the Arrow twins compose;
    /// `arrowReader` is the other. The source is consumed.
    #[napi]
    pub fn messages(&self, source: &mut JsBatchReader) -> Result<JsFixMessages> {
        Ok(JsFixMessages::over(self.inner.messages(source.take()?)))
    }

    /// A stream of messages as a stream of batches of FIX rows under `schema`.
    ///
    /// The loader turns the iterable into the pull function this takes, and
    /// the reader pulls one message at a time as its batches are read. Each
    /// message fills one row through `FixMsg.intoRow`, and batches close on
    /// the raw bytes of each message's arrival record against
    /// `batchByteSize`. An item that is not a message is the reader's error,
    /// thrown by the batch it would have landed in.
    #[napi(js_name = "_arrowReaderNative", skip_typescript)]
    pub fn arrow_reader_native(
        &self,
        env: Env,
        schema: &JsField,
        pull: Function<'_, (), Option<ClassInstance<'static, JsFixMsg>>>,
    ) -> Result<JsBatchReader> {
        let pulled = Pulled::new(env, pull)?;
        let failed = pulled.failed.clone();
        let messages = pulled
            .map(|message| Ok(message.inner.clone()))
            .chain(std::iter::from_fn(move || {
                failed.take().map(|error| Err(javascript_failure(error)))
            }));
        let reader = self
            .inner
            .arrow_reader(schema.inner.clone(), messages)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, schema.inner.name()))
    }

    /// A stream of messages as the rows one message field holds them.
    ///
    /// The third verb, and the one a consumer reads by: `parse*` turns a
    /// capture into messages, `enrich*` fills what each implies, and this
    /// answers them under whatever field a consumer reads by - a venue's own
    /// message type, `fix.schema` itself, which keeps every column a capture
    /// lands in, or any Struct root a caller built for the table it is
    /// writing.
    ///
    /// Each row is `FixMsg.intoRow` under `field`, read once here rather than
    /// per message, so a column the message did not state is derived where
    /// the crate derives it and a value the column will not hold is that
    /// column's null. A column the message does not carry at all is read off
    /// its arrival record first, which is what lets a narrow row be formatted
    /// into a wider field.
    #[napi(js_name = "_formatMessagesNative", skip_typescript)]
    pub fn format_messages_native(
        &self,
        env: Env,
        field: &JsField,
        pull: Function<'_, (), Option<ClassInstance<'static, JsFixMsg>>>,
    ) -> Result<Vec<JsScalar>> {
        let pulled = Pulled::new(env, pull)?;
        let failed = pulled.failed.clone();
        let messages = pulled.map(|message| Ok(message.inner.clone()));
        let mut rows = Vec::new();
        for row in self.inner.format_messages(messages, &field.inner) {
            rows.push(JsScalar::from_core(row.map_err(napi_error)?));
        }
        if let Some(error) = failed.take() {
            return Err(error);
        }
        Ok(rows)
    }

    /// A stream of batches of FIX rows as batches under one message field.
    ///
    /// The Arrow twin of `formatMessages`, and the last stage of the pipeline
    /// a capture runs. The schema is answered before a row is read, from the
    /// source's carried columns and `field`, and the capture's own columns
    /// still lead the row. A source carrying no arrival record is a
    /// projection already and is cast batch by batch instead of read back as
    /// messages. The source is consumed.
    #[napi]
    pub fn format_arrow_reader(
        &self,
        source: &mut JsBatchReader,
        field: &JsField,
    ) -> Result<JsBatchReader> {
        let formatted = self
            .inner
            .format_arrow_reader(source.take()?, &field.inner)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(formatted, field.inner.name()))
    }

    /// Fills a stream of messages through one lifecycle, in order, lazily.
    ///
    /// One `FixLifecycle` at `FixLifecycle.DEFAULT_INTERVAL_NS` over the whole
    /// iterable, answering every message as `FixLifecycle.fill` does: its
    /// chain `code`, `updatedat` truncated to the grid, the live chain's first
    /// `createdat`, the previous message's `prevupdatedat` and `prevmsghash`,
    /// then `msghash` and `msgphash` finalized; a terminal state closes the chain.
    /// The iterable is pulled once, in order, so the chain a message joins
    /// depends on the messages before it.
    #[napi(js_name = "_lifecycleNative", skip_typescript)]
    pub fn lifecycle_native(
        &self,
        env: Env,
        pull: Function<'_, (), Option<ClassInstance<'static, JsFixMsg>>>,
    ) -> Result<JsFixMessages> {
        let pulled = Pulled::new(env, pull)?;
        let failed = pulled.failed.clone();
        let messages = pulled.map(|message| message.inner.clone());
        Ok(JsFixMessages::pulling(
            self.inner.lifecycle(messages),
            failed,
        ))
    }

    /// Writes a stream of batches of FIX rows back to the wire, answering the
    /// count of lines.
    ///
    /// The encode direction of the same exchange: each row is the message
    /// `messages` reads out of it, written as `intoBytes` with the codec's
    /// `separator` - `SOH` when none is pinned - then a newline, into `sink`,
    /// anything with `write(chunk: Uint8Array)`. The wire is rebuilt from the
    /// arrival record, never from the columns, so a batch without the
    /// `fixentries` column is refused before a row is read. One batch is
    /// held at a time, and the source is consumed.
    #[allow(clippy::cast_precision_loss)]
    #[napi(ts_args_type = "source: BatchReader, sink: { write(chunk: Uint8Array): unknown }")]
    pub fn write_arrow_reader(&self, source: &mut JsBatchReader, sink: Object<'_>) -> Result<f64> {
        let source = source.take()?;
        let mut sink = JsSink::new(sink)?;
        let written = self.inner.write_arrow_reader(source, &mut sink);
        // The sink's own failure is the one to throw: the core reports it as
        // the write it wrapped.
        if let Some(error) = sink.failed.take() {
            return Err(error);
        }
        written.map(|count| count as f64).map_err(napi_error)
    }

    /// A cheap clone: the dictionary is shared and the pins are copied.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            registry: Arc::clone(&self.registry),
        }
    }

    /// How this codec renders: the dictionary it reads against.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        format!("FixCodec({} fields)", self.registry.len())
    }
}

/// How a codec is pinned, where a caller pins it at all.
#[napi(object, object_to_js = false)]
#[derive(Default)]
pub struct FixCodecOptions<'env> {
    /// The byte a numeric frame splits on where the line does not say.
    pub separator: Option<f64>,
    /// The batch column a line is read from; `body` when unstated.
    pub payload_column: Option<String>,
    /// What a run's row-header captures are called, in the order a line
    /// answers them.
    pub capture_names: Option<Vec<String>>,
    /// The spellings that mean "nothing was sent".
    pub null_values: Option<Vec<String>>,
    /// The code of tag 385's set an unmarked line takes on the batch door,
    /// any spelling of one; the core's `Send` code when unstated, no pin
    /// when empty.
    pub direction: Option<String>,
    /// The raw bytes one Arrow batch targets; the core's 128 MiB when unstated.
    pub batch_byte_size: Option<f64>,
    /// The `SendingTime` an undated message takes when neither it nor its
    /// carrier states one: a `DateTime64(ns, UTC)` `Scalar`, or a `Date`
    /// restated in nanoseconds. UTC now per new message when unstated or
    /// `null`.
    #[napi(ts_type = "Scalar | Date | null")]
    pub default_sending_time: Option<Either3<ClassInstance<'env, JsScalar>, JsDate<'env>, Null>>,
}

/// The default sending time one codec option names, as the core takes it.
///
/// A `Scalar` crosses as it is, so a layout other than `DateTime64(ns, UTC)`
/// is the core's refusal to state. A `Date` is its UTC millisecond instant,
/// restated once through the clock datatype's own value contract.
fn sending_time_from_js(value: Either<ClassInstance<'_, JsScalar>, JsDate<'_>>) -> Result<Scalar> {
    match value {
        Either::A(scalar) => Ok(scalar.inner.clone()),
        Either::B(date) => {
            let instant = crate::text::codec::scalar_from_date_millis(date.value_of()?)?;
            CoreDataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)
                .and_then(|clock| clock.scalar(instant))
                .map_err(napi_error)
        }
    }
}

/// The state a stream of messages has reached, one chain per live event.
///
/// Built once per stream, over one dictionary or the process default, and fed
/// every message in order. A nonempty `code` names its chain globally;
/// otherwise the first identifier - stated `altids`, else the message type's
/// declared identifiers - reaching a live chain under the instrument the
/// message names supplies its code, and a new chain is named
/// `<scope hex or ->/<first identifier>`; the instrument is a digest of what
/// the message says it is and no column carries it. Occupied identifiers are never
/// stolen, and an empty code opens no chain. `msgphash` hashes the settled code.
///
/// Every accepted message has `updatedat` truncated to its epoch grid bucket
/// of `intervalNs`, while `snapshotat` keeps the real instant. A live chain
/// carries its first message's `createdat` and hands each later message the
/// previous message's `prevupdatedat` and `prevmsghash`. A terminal state closes
/// the chain; what is held is the live chains, their code, first creation
/// instant, last clock and identity, and highest consumed bucket - never pending
/// messages. `FixCodec.lifecycle` runs one at the default cadence over an
/// iterable.
#[napi(js_name = "FixLifecycle")]
pub struct JsFixLifecycle {
    /// The lifecycle, until `snapshots` takes it for the stream it answers.
    inner: Option<CoreFixLifecycle>,
}

/// What a lifecycle throws once `snapshots` owns it.
///
/// The core's snapshot stream consumes its lifecycle, so the stream is the
/// one owner of that state: a later call gets this rather than a fresh
/// lifecycle that looks like one with no chain alive.
fn owned_by_snapshots() -> napi::Error {
    napi_error("this FixLifecycle is owned by the stream snapshots() answered; build a new one")
}

/// Read one interval exactly: a `bigint` as is, a number below 2^53.
fn interval_from_js(value: Either<BigInt, f64>) -> Result<i64> {
    match value {
        Either::A(count) => {
            let (count, lossless) = count.get_i64();
            if !lossless {
                return Err(napi_error("intervalNs must fit a signed 64-bit integer"));
            }
            Ok(count)
        }
        Either::B(count) => exact_i64(count, "intervalNs"),
    }
}

impl JsFixLifecycle {
    /// Borrow the lifecycle, refusing one a snapshot stream owns.
    fn live(&self) -> Result<&CoreFixLifecycle> {
        self.inner.as_ref().ok_or_else(owned_by_snapshots)
    }

    /// Borrow the lifecycle for a transition, refusing one a snapshot stream
    /// owns.
    fn live_mut(&mut self) -> Result<&mut CoreFixLifecycle> {
        self.inner.as_mut().ok_or_else(owned_by_snapshots)
    }
}

#[napi]
impl JsFixLifecycle {
    /// A stream with no event alive yet.
    ///
    /// `options.intervalNs` is the positive grid interval in nanoseconds,
    /// `FixLifecycle.DEFAULT_INTERVAL_NS` (one second) when unstated; a
    /// nonpositive one throws the core's located refusal. The stamped columns
    /// are the registry's own crate fields, which every registry holds.
    #[napi(constructor)]
    pub fn new(
        registry: Option<ClassInstance<'_, JsFixRegistry>>,
        options: Option<FixLifecycleOptions>,
    ) -> Result<Self> {
        let mut inner = CoreFixLifecycle::new(registry_or_global(registry)?);
        if let Some(held) = options.and_then(|options| options.interval_ns) {
            inner = inner
                .try_with_interval_ns(interval_from_js(held)?)
                .map_err(napi_error)?;
        }
        Ok(Self { inner: Some(inner) })
    }

    /// The epoch-grid interval in nanoseconds.
    #[napi(getter)]
    pub fn interval_ns(&self) -> Result<BigInt> {
        Ok(BigInt::from(self.live()?.interval_ns()))
    }

    /// Selects a positive grid interval before any chain is live.
    ///
    /// Repeating the current interval is a no-op even while chains are live;
    /// a nonpositive interval, or a change while a chain is live, throws and
    /// changes nothing.
    #[napi]
    pub fn set_interval_ns(&mut self, interval_ns: Either<BigInt, f64>) -> Result<()> {
        let interval_ns = interval_from_js(interval_ns)?;
        self.live_mut()?
            .set_interval_ns(interval_ns)
            .map_err(napi_error)
    }

    /// Answers one message normalized to its grid and moves its chain along.
    ///
    /// The chain is selected by `code`, else by identifier; the message's
    /// `updatedat` becomes its grid instant, it takes the live chain's first
    /// `createdat`, each absent previous stamp comes from the chain's last
    /// message while a stated one is kept, and `msghash` is finalized after
    /// every stamp. A fresh or cleared lifecycle replaying the same stream
    /// answers the same messages. Only the row is stamped: the arrival record
    /// is what the wire carried, so `intoBytes` re-emits the received line.
    /// A refusal throws and changes no chain, history, bucket or identifier.
    #[napi]
    pub fn fill(&mut self, message: &JsFixMsg) -> Result<JsFixMsg> {
        self.live_mut()?
            .fill(message.inner.clone())
            .map(JsFixMsg::from_core)
            .map_err(napi_error)
    }

    /// Processes one message as `fill` does, answering it only as a new
    /// snapshot, else `null`.
    ///
    /// A message emits only when it arrived off-grid in a bucket above its
    /// live chain's highest consumed one. An already-aligned arrival consumes
    /// its bucket without emitting; equal or older buckets and messages with
    /// no chain name answer `null`. Suppressed messages still advance history
    /// and a terminal one still closes its chain.
    #[napi]
    pub fn snapshot(&mut self, message: &JsFixMsg) -> Result<Option<JsFixMsg>> {
        self.live_mut()?
            .snapshot(message.inner.clone())
            .map(|held| held.map(JsFixMsg::from_core))
            .map_err(napi_error)
    }

    /// The snapshots a stream of messages emits, lazily; the loader publishes
    /// it as `snapshots`.
    ///
    /// The stream owns this lifecycle, configured interval and live chains
    /// alike, so every later call on this object throws. Only suppressed
    /// messages disappear: a refused message throws where it is met without
    /// advancing state and the stream continues, and a failure of the
    /// iterable throws and ends it.
    #[napi(js_name = "_snapshotsNative", skip_typescript)]
    pub fn snapshots_native(
        &mut self,
        env: Env,
        pull: Function<'_, (), Option<ClassInstance<'static, JsFixMsg>>>,
    ) -> Result<JsFixMessages> {
        let pulled = Pulled::new(env, pull)?;
        let life = self.inner.take().ok_or_else(owned_by_snapshots)?;
        let failed = pulled.failed.clone();
        let messages = pulled.map(|message| Ok::<CoreFixMsg, CoreError>(message.inner.clone()));
        Ok(JsFixMessages::pulling(life.snapshots(messages), failed))
    }

    /// How many events are alive: opened by a message and not yet closed by
    /// a terminal state.
    #[napi(getter)]
    pub fn alive(&self) -> Result<u32> {
        Ok(u32::try_from(self.live()?.alive()).unwrap_or(u32::MAX))
    }

    /// Forgets every chain's creation, history and bucket, as a new session
    /// or a new day would, keeping the interval.
    #[napi]
    pub fn clear(&mut self) -> Result<()> {
        self.live_mut()?.clear();
        Ok(())
    }

    /// How this stream renders: the events alive in it.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.as_ref().map_or_else(
            || "FixLifecycle(owned by its snapshot stream)".to_owned(),
            |inner| format!("FixLifecycle({} alive)", inner.alive()),
        )
    }
}

/// How a lifecycle is configured, where a caller configures it at all.
#[napi(object, object_to_js = false)]
pub struct FixLifecycleOptions {
    /// The positive epoch-grid interval in nanoseconds;
    /// `FixLifecycle.DEFAULT_INTERVAL_NS` when unstated.
    pub interval_ns: Option<Either<BigInt, f64>>,
}

/// The lifecycle's default grid interval in nanoseconds: one second.
///
/// The loader publishes it as `FixLifecycle.DEFAULT_INTERVAL_NS`.
// Discovered through NAPI's generated registration inventory rather than an
// ordinary Rust call site, like the private natives in `hashing`.
#[allow(dead_code)]
#[napi(js_name = "_fixLifecycleDefaultIntervalNsNative", skip_typescript)]
pub fn fix_lifecycle_default_interval_ns_native() -> BigInt {
    BigInt::from(CoreFixLifecycle::DEFAULT_INTERVAL_NS)
}

/// The fixed root every message answers as, built from one dictionary.
///
/// Header, the fields a consumer reads, the groups worth persisting whole, the
/// trailer, this crate's own derived facts through tag 65025, `MsgDirection`
/// (385), and the one list that closes every row: `fixentries`, the whole
/// arrival record, unresolved keys at tag 0. Columns are spelled by the
/// dictionary's folded canonical names - `msgtype`, never `35` - so a row
/// reads the way a message reads; the tag stays each column's identity, on
/// its `fix:tag`, and is what fills it. `beginstring` and
/// the replay fields - `sendingtime`, `updatedat`, `createdat`, `msghash`,
/// `msgphash`, `code` - are required; `snapshotat` is the one the bundle
/// holds without requiring, because only a snapshot stamps it.
#[napi(js_name = "fixSchema")]
pub fn fix_schema(
    registry: Option<ClassInstance<'_, JsFixRegistry>>,
    name: Option<String>,
) -> Result<JsField> {
    let registry = registry_or_global(registry)?;
    let name = name.unwrap_or_else(|| "fix".to_owned());
    yggdryl::fix_schema(&registry, name)
        .map(JsField::from_core)
        .map_err(napi_error)
}

/// The fixed root behind a capture's own columns.
///
/// `carrier` is a capture's own root - where a line was read from, which line
/// it was, what stamped it - and its columns lead the row, because that is what
/// a monitor orders and joins on. A carried column whose folded name a FIX
/// column already takes - `MsgCtxId` and `msgctxid` are one name - is dropped
/// rather than renamed: the FIX column is the one a reader spelling it means.
/// A bridge's own row header names every capture for the field it fills -
/// `bridgesessionid`, `msgctxid`, `msgseqnum`, `pluginid` - for that reason,
/// so each value reaches its column rather than leading the row, and never
/// over a reading the message stated itself.
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

/// The thirty-six definitions this crate owns, in tag order: thirty-four
/// scalar fields at 65001 to 65003, 65005 to 65015, 65017 to 65019, 65021 to
/// 65035 and 65037 to 65038, the sorted `altids` Map group at 65020 and the
/// `instids` Struct at 65036. Tags 65000, 65004 and 65016 are retired and
/// not reused.
///
/// The version read at, the ticker, `updatedat` and its partition, the
/// parent identifiers, the sessions the message states, the bridge's message
/// context, the plugin that logged the line and the one it came through
/// before that, the two session names the line spells, the ISIN, MIC and
/// order state a row derives, the `msghash` and `msgphash` identities,
/// the direct identifiers enrichment records in `altids`, the previous
/// message's `prevupdatedat` and `prevmsghash`, `createdat`, `code` and
/// `snapshotat`, the `sourceurl` a line was read from, the `nofixentries`
/// that counts its arrival record, `recordedat` and `expiredat`, the two
/// lane currencies, the bridge's own session instance, the instrument's
/// Bloomberg, CUSIP and SEDOL codes with the `instids` Struct that joins
/// them, and the two session message identifiers. `updatedat`, `msghash`, `msgphash`, `createdat`, `code` and
/// `snapshotat` are non-null. Every registry already holds them in their
/// category, so this is the listing a schema or a document walks rather than
/// something a caller registers.
#[napi(js_name = "fixCrateFields")]
pub fn fix_crate_fields() -> Result<Vec<JsField>> {
    yggdryl::fix_crate_fields()
        .map(|held| held.iter().cloned().map(JsField::from_core).collect())
        .map_err(napi_error)
}

/// The scalar fields the plugin dictionary owns.
#[napi(js_name = "fixPluginFields")]
pub fn fix_plugin_fields() -> Result<Vec<JsField>> {
    yggdryl::fix_plugin_fields()
        .map(|held| held.iter().cloned().map(JsField::from_core).collect())
        .map_err(napi_error)
}

/// The message a plugin configuration is: the `pluginconfig` component.
///
/// FIX's own `MsgType` beside every plugin attribute and the `BeginString`,
/// `SenderCompID` and `TargetCompID` a configuration also states. Its name is
/// the type a configuration reads as and `field.fix.msgtype` the wire code it
/// answers on tag 35. Registering it is nobody's choice: every registry holds
/// it as it holds the crate's own fields, so a configuration reads as itself
/// whatever dictionary met it.
#[napi(js_name = "fixPluginMessage")]
pub fn fix_plugin_message() -> Result<JsField> {
    yggdryl::fix_plugin_message()
        .map(|held| JsField::from_core(held.clone()))
        .map_err(napi_error)
}

/// The process-wide registry, loading it on the first call.
///
/// The order is the core's: a registry installed by
/// [`fix_install_global_registry`], then the folder `YGGDRYL_FIX_REGISTRY`
/// names, then `~/.config/fix` when it exists, then a new registry holding the
/// crate's own definitions and the two seeded clocks alone. Only the third
/// step treats absence as that
/// default; every other failure throws with the native message and the default
/// stays unresolved, so the next call retries.
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
