//! Node.js view of the FIX dictionary, its message, and the process default.
//!
//! Nothing here resolves, folds, merges, shards or validates: the registry is
//! one [`Arc`] over the core [`FixRegistry`](CoreFixRegistry), and every
//! accessor coerces its key once at the boundary and redirects to the most
//! specific native method. The typed `FIX:` vocabulary is not here either - it
//! lives on the protocol view class [`JsProtocolField`](crate::JsProtocolField),
//! which is what `field.fix` already answers.
//!
//! An identifier crosses as a `number` - the signed 32-bit digest of a
//! field's tag and its name that the core derives - and is read once here
//! through [`id_from_js`], so it gets no class of its own in JavaScript. A
//! bare tag or name uses the core's deterministic best match: a `number`
//! there is a tag, never an identifier, and an identifier is only ever
//! spelled through the `ById` doors. A dictionary's membership is
//! `FIX:sources` on the field it contributed to, read on the protocol view,
//! each id an entry of the registry's sources catalog (`sources`,
//! `addSource`); nothing here resolves through it.
//!
//! A code set is the dictionary's own, not the field's: a field states only
//! the name it reads by, on `field.fix.codeset`, and the members are held
//! here once under that name. They cross as plain objects, the way
//! `FIX:directions` does - `{value, name, aliases?, doc?, group?}` per
//! member - so the codes a caller writes are the records the store writes
//! under `codesets/<name>.json`.
//!
//! A message's typed facts - its event, its header, its capture, its text
//! and its metadata - cross as plain values: a UUID as its text, a hash and
//! an instant as a `bigint`, a price as its decimal text, a code as the text
//! it is. The graph traits are what answer them, and the message's own
//! `getBy*` doors keep answering a `Scalar`, so a reader that wants the
//! native value of a typed fact asks by its tag.

mod catalog;

pub use catalog::JsMsgType;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use napi::JsDate;
use napi::JsValue as _;
use napi::bindgen_prelude::{
    BigInt, Buffer, ClassInstance, Either, Either3, Env, Function, Generator, JsObjectValue as _,
    Null, Object, Result, Unknown, ValueType,
};
use napi_derive::napi;
use yggdryl::graph::{Element, Event, Market, Metadata, Operation};
use yggdryl::{
    DataType as CoreDataType, Field as CoreField, FixCapture, FixCode as CoreFixCode,
    FixCodeSet as CoreFixCodeSet, FixCodec as CoreFixCodec, FixEntry, FixHeader,
    FixId as CoreFixId, FixKey, FixMsg as CoreFixMsg, FixRegistry as CoreFixRegistry, Scalar,
    TimeUnit, Timezone,
};

use crate::field::JsField;
use crate::graph::{JsMarketData, JsMarketDataRowIterator};
use crate::iobase::{LocationInput, folder_from_input, located_from_input};
use crate::iomedia::JsBatchReader;
use crate::isin_registry::JsIsinRegistry;
use crate::text::codec::JsScalar;
use crate::text::line::{JsFieldPath, JsTextLine, path_from_input};
use crate::{
    Failed, Pulled, exact_f64, exact_i32, exact_i64, javascript_failure, napi_error,
    napi_type_error, or_null,
};

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
fn registry_or_env(
    registry: Option<ClassInstance<'_, JsFixRegistry>>,
) -> Result<Arc<CoreFixRegistry>> {
    match registry {
        Some(held) => Ok(Arc::clone(&held.inner)),
        None => CoreFixRegistry::from_env()
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

/// What one [`JsFixRegistry::commit`] moved under a store root.
///
/// A commit names the documents it wrote and removed and counts the ones it
/// left, because a store holds thousands a run normally leaves alone: naming
/// each of those would bury the handful that moved.
#[napi(object)]
pub struct FixCommitReport {
    /// The documents written, in the order a store lays them out.
    pub written: Vec<String>,
    /// How many documents already stated what the registry does.
    pub skipped: u32,
    /// The documents removed because no definition holds them any more.
    pub removed: Vec<String>,
}

/// One member of a FIX code set, as the plain object JavaScript reads and
/// writes.
///
/// The record a store writes under `codesets/<name>.json`: the wire value
/// and the symbolic name every member states, and the spellings, the wording
/// and the group a specification adds where it has them. A key a member does
/// not state is absent rather than empty, so a bare code is the two facts it
/// is.
/// One thing a message states that its reading could not take as it
/// stands: the field it was stated under, and why.
#[napi(object)]
pub struct FixAnomalyView {
    /// The dictionary's name for the field, else the key as it arrived.
    pub field: String,
    /// Why the reading could not take the value as it stands.
    pub reason: String,
}

#[napi(object)]
pub struct FixCode {
    /// The wire value this code stands for.
    pub value: String,
    /// The symbolic name.
    pub name: String,
    /// The venue and per-version spellings that also reach this code.
    pub aliases: Option<Vec<String>>,
    /// The specification's own wording, decoded.
    pub doc: Option<String>,
    /// The group the specification files this code under, decoded.
    pub group: Option<String>,
}

impl FixCode {
    /// One owned core code, as the object JavaScript reads.
    fn from_core(code: CoreFixCode) -> Self {
        Self {
            value: code.value().to_owned(),
            name: code.name().to_owned(),
            aliases: (!code.aliases().is_empty())
                .then(|| code.aliases().iter().map(ToString::to_string).collect()),
            doc: code.description().map(ToOwned::to_owned),
            group: code.group().map(ToOwned::to_owned),
        }
    }

    /// The core code this object states.
    fn into_core(self) -> CoreFixCode {
        let mut code = CoreFixCode::new(self.name, self.value);
        if let Some(aliases) = self.aliases {
            code = code.with_aliases(aliases);
        }
        if let Some(doc) = self.doc {
            code = code.with_description(doc);
        }
        if let Some(group) = self.group {
            code = code.with_group(group);
        }
        code
    }
}

/// One named FIX code set, as the plain object JavaScript reads.
///
/// The dictionary owns the members under the name and a field states only
/// the name, so this is the pair read together: one vocabulary, however many
/// fields draw on it.
#[napi(object, object_from_js = false)]
pub struct FixCodeSetView {
    /// The name the dictionary files this set under.
    pub name: String,
    /// The members, ordered by wire value.
    pub codes: Vec<FixCode>,
}

/// One entry of a dictionary's sources catalog, as the plain object
/// JavaScript reads: what a `FIX:sources` id names, recorded once.
///
/// The keys are the entry a store writes in `sources.json` - `file` left out
/// where none is known, `pluginside` always stated.
#[napi(object, object_from_js = false)]
pub struct FixSourceView {
    /// The id, folded to ASCII lowercase: what a field's `FIX:sources`
    /// states.
    pub id: String,
    /// The file the source was read from, as it was named - `venue.cfb` -
    /// where one is known.
    pub file: Option<String>,
    /// The role of the source's plugin, as the `pluginside` member's stored
    /// name: `BUYS`, `SELL`, or `UKNW` where the source states none.
    pub pluginside: String,
}

impl FixSourceView {
    /// One borrowed catalog entry, as the object JavaScript reads.
    fn from_core(source: &yggdryl::FixSource) -> Self {
        Self {
            id: source.id().to_owned(),
            file: source.file().map(ToOwned::to_owned),
            pluginside: source.pluginside().as_str().to_owned(),
        }
    }
}

/// What `addSource` records beside the id: each left out states nothing.
#[napi(object)]
pub struct FixSourceOptions {
    /// The file the source was read from, as it was named.
    pub file: Option<String>,
    /// The role of the source's plugin: a `PluginSide` member's stored name
    /// in any case, the role's own name - `BuySide`, `sell-side` - or its
    /// code, what `PluginSide.BUYS` holds.
    #[napi(ts_type = "string | number")]
    pub pluginside: Option<Either<String, f64>>,
}

/// One borrowed code set, as the object JavaScript reads.
///
/// The members are owned on the way across - a JavaScript value outlives the
/// dictionary it was read from - and the stored escapes are decoded there,
/// which is what `FixCode::from` does.
/// The parents one identifier type is given: a base and its parent types,
/// nearest first.
#[napi(object, object_from_js = false)]
pub struct FixParentSource {
    /// The identifier type the parents belong to: the field's `FIX:idmap`
    /// key, else its folded name.
    pub base: String,
    /// The parent types, nearest first.
    pub parents: Vec<String>,
}

/// Where one identifier type stands among the parents of a base.
#[napi(object, object_from_js = false)]
pub struct FixParentPlace {
    /// The identifier type this one is a parent of.
    pub base: String,
    /// Its place among the base's parents, `0` the nearest.
    pub at: u32,
}

/// One field that names a message by an identifier, and the key it states.
#[napi(object, object_from_js = false)]
pub struct FixIdMapSource {
    /// The field's tag.
    pub tag: i32,
    /// The map it lands in: `identifiers`, the one identifier map.
    pub map: String,
    /// The lower-case identifier type it lands under.
    pub key: String,
    /// Whether an operation that follows another carries it.
    pub follow: bool,
    /// On `PartyID(448)`, the `PartyRole(452)` code of the occurrence stating it.
    pub role: Option<String>,
}

fn codeset_view(set: CoreFixCodeSet<'_>) -> Result<FixCodeSetView> {
    Ok(FixCodeSetView {
        name: set.name().to_owned(),
        codes: set
            .codes()
            .map(|code| {
                Ok(FixCode::from_core(CoreFixCode::from(
                    code.map_err(napi_error)?,
                )))
            })
            .collect::<Result<Vec<_>>>()?,
    })
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
    /// The repeating group a counter tag opens, or `null`.
    ///
    /// `tag` is the counter's, never the group's own: `getFieldByTag` answers
    /// the counter itself off the same key, and the group it heads is a
    /// definition of its own, reached here or by its name. Two groups on one
    /// counter name nothing.
    #[napi]
    pub fn get_field_by_counter(&self, tag: f64) -> Result<Option<JsField>> {
        let tag = exact_i32(tag, "tag")?;
        Ok(self
            .inner
            .get_field_by_counter(tag)
            .cloned()
            .map(JsField::from_core))
    }

    /// The repeating group a counter tag opens, failing when absent or
    /// ambiguous.
    #[napi]
    pub fn field_by_counter(&self, tag: f64) -> Result<JsField> {
        let tag = exact_i32(tag, "tag")?;
        self.inner
            .field_by_counter(tag)
            .cloned()
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// The message definition a wire code, a canonical name or tag 35's
    /// alias names, or `null`: a copy of the registry's own, so a later
    /// mutation of the dictionary leaves it as it was answered.
    #[napi]
    pub fn get_msgtype(&self, spelling: String) -> Option<JsMsgType> {
        self.inner
            .get_msgtype(&spelling)
            .map(|message| JsMsgType::from_core(message.clone()))
    }

    /// The message definition a spelling names, failing when absent.
    #[napi]
    pub fn msgtype(&self, spelling: String) -> Result<JsMsgType> {
        self.inner
            .msgtype(&spelling)
            .map(|message| JsMsgType::from_core(message.clone()))
            .map_err(napi_error)
    }

    /// A registry holding the built-in definitions.
    ///
    /// Every registry holds the crate's own definitions - the scalar columns
    /// `fixCrateFields` lists, the `metadata` Map group and the `fixmsg`
    /// component that is the fixed row - and the standard
    /// `SendingTime` (52) and `TransactTime` (60) clock fields, seeded where
    /// the dictionary defines no field of its own at those tags. A dictionary
    /// loaded from a store, built from fields or left alone holds them alike,
    /// and `size` counts every one of them beside the dictionary's own.
    #[napi(constructor)]
    pub fn new() -> Self {
        Self::from_arc(Arc::new(CoreFixRegistry::new()))
    }

    /// The registry the process environment names, loaded on the first call.
    ///
    /// The order is the core's: a registry installed by `installEnv`, then the
    /// folder `YGGDRYL_FIX_REGISTRY` names, then `~/.config/fix` when it
    /// exists, then a new registry holding the crate's own definitions and the
    /// two seeded clocks alone. Only the third step treats absence as that
    /// default; every other failure throws with the native message and the
    /// default stays unresolved, so the next call retries.
    #[napi(factory)]
    pub fn from_env() -> Result<Self> {
        CoreFixRegistry::from_env()
            .map(|registry| Self::from_arc(Arc::clone(registry)))
            .map_err(napi_error)
    }

    /// Install `registry` as the one every later `fromEnv` answers, before
    /// anything resolves one.
    ///
    /// Throws once the default has resolved or been installed, so the value
    /// every caller saw cannot change underneath them.
    #[napi]
    pub fn install_env(registry: &JsFixRegistry) -> Result<()> {
        CoreFixRegistry::install_env((*registry.inner).clone()).map_err(napi_error)
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
    /// the order it spelled them. `dialect` is the source id every field,
    /// group, component and message the file produces is stamped with, on
    /// its `FIX:sources` - standard tags included, since membership means
    /// the dictionary speaks it - and the registry holds its catalog entry:
    /// the file's name and the plugin side the root's `type` states. With
    /// none named nothing is stamped and no entry is made. A dialect that is
    /// empty or holds a quote, a backslash or a control character is
    /// refused.
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
        // actually is: a container handle reads as its leaves end to end,
        // which is no one vocabulary.
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

    /// Commit the store and answer nothing.
    ///
    /// The same work as [`Self::commit`] for a caller that does not read what
    /// moved.
    #[napi]
    pub fn write_into(&self, location: LocationInput<'_>) -> Result<()> {
        let mut holder = folder_from_input(location)?;
        self.inner.write_into(&mut holder).map_err(napi_error)
    }

    /// Write every populated shard under `<location>/fields/<shard>.json` and
    /// every definition under `<location>/<category>/<name>.json`, removing
    /// the shards and trees no field populates any more. A shard is named by
    /// its tag block, nine digits with leading zeros: tag 55 lands in
    /// `fields/000000000.json`, tag 5001 in `fields/000000050.json`. The
    /// crate's own definitions are written like every other - its tag block
    /// from 65000 is `fields/000000650.json`, its `metadata` Map group one
    /// document under `groups/`, and the fixed row is
    /// `components/fixmsg.json` - so a store states the whole row; a
    /// reader takes the definition it holds from construction over the
    /// document it finds.
    ///
    /// Each document is digested where it lies and left alone where it
    /// already states this registry, so a commit writes what moved and a
    /// second commit of one registry writes nothing. The report is a plain
    /// object: `written` and `removed` name the documents, in the order a
    /// store lays them out, and `skipped` counts the ones a run left.
    #[napi]
    pub fn commit(&self, location: LocationInput<'_>) -> Result<FixCommitReport> {
        let mut holder = folder_from_input(location)?;
        let report = self.inner.commit(&mut holder).map_err(napi_error)?;
        Ok(FixCommitReport {
            written: report.written.iter().map(ToString::to_string).collect(),
            skipped: u32::try_from(report.skipped).unwrap_or(u32::MAX),
            removed: report.removed.iter().map(ToString::to_string).collect(),
        })
    }

    /// How many fields are held: the scalar fields, then the components and
    /// the groups, the crate's own and the two seeded clocks among them.
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
    /// One mutation: a refusal - no `FIX:tag`, a datatype disagreeing with
    /// the stored field - leaves the dictionary exactly as it was.
    #[napi]
    pub fn add_field(&mut self, field: &JsField) -> Result<bool> {
        self.inner_mut()?
            .add_field(field.inner.clone())
            .map_err(napi_error)
    }

    /// Add a field, answering the one it replaced.
    ///
    /// A definition is filed by the shape it has: a Struct inserts as a
    /// component - a message when it carries `FIX:msgtype` - a Serie of
    /// Structs or a Map as a group, and anything else as a scalar field.
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
    /// same tag under the same folded name, or the component or group of
    /// the same folded name.
    #[napi]
    pub fn update(&mut self, field: &JsField) -> Result<()> {
        let field = field.inner.clone();
        self.inner_mut()?.update(field).map_err(napi_error)
    }

    /// Remove the field a tag or a name reaches, answering it.
    ///
    /// A name no scalar answers to reaches a component or a group, so a
    /// definition leaves through the same door; one another definition still
    /// references stays, and `null` says so.
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

    /// The code set held under `name`, or `null`.
    ///
    /// The lenient door beside `codeset`, which throws absence: a caller
    /// asking whether a vocabulary is held asks this. The name is folded,
    /// so whichever spelling a field states reaches it.
    #[napi]
    pub fn get_codeset(&self, name: String) -> Result<Option<FixCodeSetView>> {
        self.inner.get_codeset(&name).map(codeset_view).transpose()
    }

    /// The code set held under `name`, failing when absent.
    #[napi]
    pub fn codeset(&self, name: String) -> Result<FixCodeSetView> {
        self.inner
            .codeset(&name)
            .map_err(napi_error)
            .and_then(codeset_view)
    }

    /// The code set `field` reads its values by, or `null`.
    ///
    /// The field states the name and the dictionary holds the members, so
    /// this is the one door between them. A field naming no set answers
    /// `null`; a held field never names one this dictionary lacks, because
    /// every door a field arrives through refuses that.
    #[napi]
    pub fn codeset_of(&self, field: &JsField) -> Result<Option<FixCodeSetView>> {
        self.inner
            .codeset_of(&field.inner)
            .map(codeset_view)
            .transpose()
    }

    /// The names of every code set held, in name order.
    ///
    /// The listing, the way `dialects` lists membership: a set is read by
    /// name through `codeset`, so nothing parses here.
    #[napi]
    pub fn codeset_names(&self) -> Vec<String> {
        self.inner
            .codesets()
            .map(|set| set.name().to_owned())
            .collect()
    }

    /// Every field that names a message by an identifier, one entry per key
    /// its `FIX:idmap` states, in tag order. A message rebuilds its
    /// `identifiers` from these, and an operation that follows another carries
    /// the keys whose entry follows.
    #[napi]
    pub fn idmap_sources(&self) -> Vec<FixIdMapSource> {
        self.inner
            .idmap_sources()
            .iter()
            .map(|(tag, source)| FixIdMapSource {
                tag: *tag,
                map: source.map().as_str().to_owned(),
                key: source.key().to_string(),
                follow: source.follows(),
                role: source.role().map(ToOwned::to_owned),
            })
            .collect()
    }

    /// Every parents list this dictionary's fields state under
    /// `FIX:parents`, one `{ base, parents }` per field, in field order: the
    /// base is the field's `FIX:idmap` key, else its folded name.
    #[napi]
    pub fn parent_sources(&self) -> Vec<FixParentSource> {
        self.inner
            .parent_sources()
            .iter()
            .map(|(base, parents)| FixParentSource {
                base: base.as_str().to_owned(),
                parents: parents
                    .iter()
                    .map(|kind| kind.as_str().to_owned())
                    .collect(),
            })
            .collect()
    }

    /// The parent types of the identifier type `base`, nearest first: the
    /// list a field stating it names under `FIX:parents`, else the ones its
    /// name has - `orderid` is `["parentorderid", "origorderid"]`, `clordid`
    /// `["origclordid"]`; a parent type has none.
    #[napi]
    pub fn parents_of(&self, base: String) -> Result<Vec<String>> {
        let base: yggdryl::IdType = base.parse().map_err(napi_error)?;
        Ok(self
            .inner
            .parents_of(&base)
            .iter()
            .map(|kind| kind.as_str().to_owned())
            .collect())
    }

    /// The base the identifier type `kind` is a parent of and its place
    /// among the base's parents - `{ base: "clordid", at: 0 }` for
    /// `origclordid`, `{ base: "orderid", at: 1 }` for `origorderid` - or
    /// `null` for a type that is no one's parent.
    #[napi]
    pub fn parent_of(&self, kind: String) -> Result<Option<FixParentPlace>> {
        let kind: yggdryl::IdType = kind.parse().map_err(napi_error)?;
        Ok(self
            .inner
            .parent_of(&kind)
            .map(|(base, at)| FixParentPlace {
                base: base.as_str().to_owned(),
                at: u32::try_from(at).unwrap_or(u32::MAX),
            }))
    }

    /// The market data type one wire value of the FIX field `tag` types an
    /// element as, as the member's stored name: this dictionary's own
    /// `FIX:marketdatatype` first, then the crate's table; `null` for a
    /// field that types nothing.
    #[napi]
    pub fn marketdatatype_of(&self, tag: i32, wire: String) -> Option<String> {
        self.inner
            .marketdatatype_of(tag, &wire)
            .map(|member| member.as_str().to_owned())
    }

    /// The time in force one wire value of the FIX field `tag` stands for,
    /// as the member's stored name: this dictionary's own `FIX:timeinforce`
    /// first, then `TimeInForce(59)`'s own values, a value neither names
    /// being `OTHER`; `null` for a field that states no time in force.
    #[napi]
    pub fn timeinforce_of(&self, tag: i32, wire: String) -> Option<String> {
        self.inner
            .timeinforce_of(tag, &wire)
            .map(|member| member.as_str().to_owned())
    }

    /// The symbolic name one wire value stands for in the set `name`.
    ///
    /// What a field's own `codeName` answered before a set had a name of its
    /// own; the set is where the vocabulary lives now, so this is keyed by
    /// it and throws when the dictionary holds none.
    #[napi]
    pub fn code_name(&self, name: String, value: String) -> Result<Option<String>> {
        Ok(self
            .inner
            .codeset(&name)
            .map_err(napi_error)?
            .code_name(&value)
            .map(ToOwned::to_owned))
    }

    /// The wire value any spelling of a code stands for in the set `name`:
    /// the value itself, a symbolic name, or an alias, folded.
    ///
    /// A spelling the set does not answer to is `null` rather than a
    /// refusal, because a venue sends codes no dictionary lists.
    #[napi]
    pub fn code_value(&self, name: String, text: String) -> Result<Option<String>> {
        Ok(self
            .inner
            .codeset(&name)
            .map_err(napi_error)?
            .code_value(&text)
            .map(ToOwned::to_owned))
    }

    /// State the members of the code set `name`, replacing what it held.
    ///
    /// The set is filed under the folded name, which is the stem a store
    /// writes it as. An empty array removes the set, and one a held field
    /// still reads by is refused: a field may not be left naming a
    /// vocabulary nothing states. `marketdatakindcodeset` is intrinsic: its stable
    /// integer market operation IDs cannot be replaced or removed.
    #[napi]
    pub fn set_codeset(&mut self, name: String, codes: Vec<FixCode>) -> Result<()> {
        let codes: Vec<CoreFixCode> = codes.into_iter().map(FixCode::into_core).collect();
        self.inner_mut()?
            .set_codeset(&name, &codes)
            .map_err(napi_error)
    }

    /// Fold `codes` into the code set `name`, keeping what it already held.
    ///
    /// Keyed by wire value: a placeholder name yields to a real one, every
    /// surviving spelling is kept as an alias, and a set the dictionary did
    /// not hold arrives whole. So a venue's statement of a vocabulary
    /// enriches the one held rather than replacing it. `marketdatakindcodeset` is
    /// intrinsic and refuses any merge that would change its stable integer
    /// IDs.
    #[napi]
    pub fn merge_codeset(&mut self, name: String, codes: Vec<FixCode>) -> Result<()> {
        let codes: Vec<CoreFixCode> = codes.into_iter().map(FixCode::into_core).collect();
        self.inner_mut()?
            .merge_codeset(&name, &codes)
            .map_err(napi_error)
    }

    /// Remove the code set `name`, answering the members it held.
    ///
    /// A set no field reads by leaves; one a held field still names is
    /// refused, naming the field. A name nothing is filed under answers
    /// `null`. `marketdatakindcodeset` is intrinsic and cannot be removed.
    #[napi]
    pub fn remove_codeset(&mut self, name: String) -> Result<Option<Vec<FixCode>>> {
        Ok(self
            .inner_mut()?
            .remove_codeset(&name)
            .map_err(napi_error)?
            .map(|codes| codes.into_iter().map(FixCode::from_core).collect()))
    }

    /// The distinct source ids any field or definition names on its
    /// `FIX:sources`, folded, sorted.
    ///
    /// Membership is provenance and this is its listing; nothing resolves
    /// through it. The ids fields state, not the catalog `sources()`
    /// answers: an entry no field names is not listed here, and an id no
    /// entry holds is. A registry holding only the specification's own
    /// fields answers an empty array.
    #[napi]
    pub fn dialects(&self) -> Vec<String> {
        self.inner.dialects()
    }

    /// The sources catalog, in id order: one plain object per source this
    /// dictionary was built from - a `CBlock` folded in, a dialect a
    /// definition was created under - holding what is known of it once,
    /// `{ id, file?, pluginside }`. A store writes it as `sources.json`.
    #[napi]
    pub fn sources(&self) -> Vec<FixSourceView> {
        self.inner.sources().map(FixSourceView::from_core).collect()
    }

    /// Record one source in the catalog, answering whether it arrived.
    ///
    /// `id` is held to the id grammar - a non-empty word holding no quote,
    /// backslash or control character - and folded to ASCII lowercase. An id
    /// already held, under the same fold, keeps its entry and takes only
    /// what it lacked: a file where it stated none, a plugin side where it
    /// stated `UKNW`; two stated sides that disagree keep the held one.
    /// Nothing here touches a field: a field names its sources itself, on
    /// `field.fix.sources`.
    #[napi]
    pub fn add_source(&mut self, id: String, options: Option<FixSourceOptions>) -> Result<bool> {
        let mut source = yggdryl::FixSource::new(&id).map_err(napi_error)?;
        if let Some(options) = options {
            if let Some(file) = options.file {
                source = source.with_file(file);
            }
            if let Some(side) = options.pluginside {
                source = source.with_pluginside(crate::pluginside::plugin_side_of(side)?);
            }
        }
        Ok(self.inner_mut()?.add_source(source))
    }

    /// Every field, lazily: the scalar fields in ascending identifier order,
    /// then the components and the groups in the catalog's name order.
    ///
    /// The order is the core's: tag-major, then by identifier, the definitions
    /// behind. The iterator holds the registry and the identifier it stopped
    /// at, so nothing is collected crossing the boundary and the dictionary is
    /// never cloned to walk it. Holding it is therefore sharing it: a mutation
    /// refuses until the walk ends, which is what stops the fields moving
    /// under a cursor into them. The loader wires `Symbol.iterator` over this.
    #[napi(ts_return_type = "Generator<Field>")]
    pub fn keys(&self) -> JsFixFieldIterator {
        JsFixFieldIterator {
            registry: Some(Arc::clone(&self.inner)),
            after: None,
            scalars: 0,
            definition: None,
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

    /// A complete native catalog snapshot: the code sets, the sources
    /// catalog where it holds an entry, the fields, the components and the
    /// groups.
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

/// The fields of a registry: the scalars in ascending identifier order, then
/// the components and the groups.
///
/// Answered by `keys()`. The scalars advance with the core's own cursor - the
/// registry plus the last `FixId` it answered - so taking one field from a
/// dictionary of thousands costs one lookup, and a walk crosses every field in
/// the one order the core iterates: tag-major, then identifier. The
/// definitions follow, each reached by its position from the end of the
/// core's own walk, because the catalog keeps them behind the scalars and
/// answers no cursor into them. It lets the registry go the moment the walk
/// ends, because JavaScript collects at its own pace and a mutation must not
/// wait for a drained iterator to be swept.
#[napi(iterator, js_name = "FixFieldIterator")]
pub struct JsFixFieldIterator {
    registry: Option<Arc<CoreFixRegistry>>,
    after: Option<CoreFixId>,
    /// How many scalars the cursor answered: where the definitions start in
    /// the core's walk once the scalars are exhausted.
    scalars: usize,
    /// The next definition to answer, once the scalars are exhausted.
    definition: Option<usize>,
}

impl JsFixFieldIterator {
    /// The definition at `index` behind the scalars, reached from the back
    /// of the core's walk so the scalars in front are never stepped over.
    fn definition_at(
        registry: &CoreFixRegistry,
        scalars: usize,
        index: usize,
    ) -> Option<CoreField> {
        let definitions = registry.iter().len().checked_sub(scalars)?;
        let from_back = definitions.checked_sub(index + 1)?;
        registry.iter().nth_back(from_back).cloned()
    }
}

impl Generator for JsFixFieldIterator {
    type Yield = JsField;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<Self::Next>) -> Option<Self::Yield> {
        let registry = self.registry.as_ref()?;
        if let Some(index) = self.definition {
            let Some(field) = Self::definition_at(registry, self.scalars, index) else {
                self.registry = None;
                return None;
            };
            self.definition = Some(index + 1);
            return Some(JsField::from_core(field));
        }
        let found = registry
            .next_field_after(self.after)
            .map(|field| (field.clone(), field.as_fix().id().ok().flatten()));
        match found {
            // The cursor is the canonical identifier every registered field
            // carries; a field without one cannot be advanced past, so the
            // scalars end there and the definitions follow.
            Some((field, Some(id))) => {
                self.after = Some(id);
                self.scalars += 1;
                Some(JsField::from_core(field))
            }
            Some((field, None)) => {
                self.scalars += 1;
                self.definition = Some(0);
                Some(JsField::from_core(field))
            }
            None => {
                self.definition = Some(0);
                self.next(None)
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

/// One entry of a message, as the plain object JavaScript reads.
///
/// The row read as a tree: a resolved field carries its canonical positive
/// tag and name, a key no dictionary explains carries `0` and its own
/// spelling, and an entry that heads others - a group filed under its
/// counter's tag, an occurrence, a component - nests them under `entries`. A
/// group entry's value is its length, the count no field states beside it;
/// an occurrence and a component state no value of their own.
#[napi(object, object_from_js = false)]
pub struct FixEntryView {
    /// The resolved canonical tag, or `0` for a key no dictionary explains.
    pub tag: i32,
    /// The dictionary's canonical name, else the key as it arrived.
    pub name: String,
    /// The value as the wire spells it, or `null` for an entry that only
    /// heads others.
    #[napi(ts_type = "string | null")]
    pub value: Either<String, Null>,
    /// The entries nested under this one, in their order.
    pub entries: Vec<FixEntryView>,
}

/// One instant as JavaScript reads it: nanoseconds since the epoch.
fn instant(unix: i64) -> BigInt {
    BigInt::from(unix)
}

/// One core entry and everything under it, as the object JavaScript reads.
fn entry_view(entry: &FixEntry) -> FixEntryView {
    FixEntryView {
        tag: entry.tag(),
        name: entry.name().to_owned(),
        value: or_null(entry.value().map(ToOwned::to_owned)),
        entries: entry.entries().iter().map(entry_view).collect(),
    }
}

/// The metadata a market element carries, key to value, sorted.
fn metadata_view(metadata: &Metadata) -> BTreeMap<String, String> {
    metadata
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// The sources an element states, each as its text.
fn sources_view<E: Element + ?Sized>(element: &E) -> Vec<String> {
    element
        .get_srcuuids()
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// The standard header a message holds typed, as plain values.
///
/// What FIX puts in front of every message: the version it says it speaks,
/// the type it is, who sent it to whom, its place in the session and when it
/// was sent, plus which way it moved where the line or the caller said.
#[napi(object, object_from_js = false)]
pub struct FixHeaderView {
    /// `BeginString(8)`: `FIX.4.4`; empty where the message states none.
    pub beginstring: String,
    /// `MsgType(35)`: the wire code, `D`; empty where the message states
    /// none.
    pub msgtype: String,
    /// `SenderCompID(49)`, where stated.
    #[napi(ts_type = "string | null")]
    pub sendercompid: Either<String, Null>,
    /// `TargetCompID(56)`, where stated.
    #[napi(ts_type = "string | null")]
    pub targetcompid: Either<String, Null>,
    /// `MsgSeqNum(34)`, where stated.
    #[napi(ts_type = "number | null")]
    pub msgseqnum: Either<f64, Null>,
    /// `SendingTime(52)` as nanoseconds since the Unix epoch, UTC: what the
    /// message stated, else the clock the intake settled.
    pub sendingtime: BigInt,
    /// `PossDupFlag(43)`, where stated.
    #[napi(ts_type = "boolean | null")]
    pub possdupflag: Either<bool, Null>,
    /// `MsgDirection(385)` as the code the dictionary's set spells it, where
    /// the line or the caller stated which way the message moved.
    #[napi(ts_type = "string | null")]
    pub msgdirection: Either<String, Null>,
    /// `SignatureLength(93)`: how many bytes the signature runs to, where
    /// the frame carried one.
    #[napi(ts_type = "number | null")]
    pub signaturelength: Either<f64, Null>,
    /// `Signature(89)`: the bytes the frame was signed with, where it was.
    #[napi(ts_type = "Buffer | null")]
    pub signature: Either<Buffer, Null>,
    /// `CheckSum(10)` as the wire spelled it. Always a frame's last pair,
    /// and the last one `intoBytes` emits.
    #[napi(ts_type = "string | null")]
    pub checksum: Either<String, Null>,
}

fn header_view(header: &FixHeader) -> Result<FixHeaderView> {
    Ok(FixHeaderView {
        beginstring: header.beginstring().to_owned(),
        msgtype: header.msgtype().to_owned(),
        sendercompid: or_null(header.sendercompid().map(ToOwned::to_owned)),
        targetcompid: or_null(header.targetcompid().map(ToOwned::to_owned)),
        msgseqnum: or_null(
            header
                .msgseqnum()
                .map(|held| exact_f64(held, "msgseqnum"))
                .transpose()?,
        ),
        sendingtime: instant(header.sendingtime()),
        possdupflag: or_null(header.possdupflag()),
        msgdirection: or_null(header.msgdirection().map(ToOwned::to_owned)),
        // Every `i32` is a JavaScript number exactly, so a length needs no
        // width check the way a sequence number does.
        signaturelength: or_null(header.signaturelength().map(f64::from)),
        signature: or_null(header.signature().map(Buffer::from)),
        checksum: or_null(header.checksum().map(ToOwned::to_owned)),
    })
}

/// What the capture stated about the line a message was read from, as
/// plain values.
///
/// What the line itself said about the capture it was written for: a
/// bridge's own row header. None of it is FIX and none is content, so none
/// of it is an entry or on the wire, and none of it reaches the code the
/// content digests to. Where the message type, the session instance, the
/// message context and `MsgSeqNum` are all stated, their values joined by
/// `:` are `msgsesseventid`, the session event the message was delivered
/// as: derived whenever the message settles, delivery provenance rather
/// than the message's content identity or its chain code, and the key two
/// observations of one delivery merge on.
///
/// What the *reader* said about the line - the object it came out of, when
/// it was recorded - is not here: those are the capture's own columns,
/// which the message carries under their names - `carried` - and `intoRow`
/// states again.
#[napi(object, object_from_js = false)]
pub struct FixCaptureView {
    /// The plugin that logged the line inside a bridge, as the bridge names
    /// it.
    #[napi(ts_type = "string | null")]
    pub msgpluginid: Either<String, Null>,
    /// The role of the FIX plugin whose session produced the message, as
    /// the `pluginside` member's stored name: `BUYS` for a Buy-Side plugin,
    /// `SELL` for a Sell-Side one, `UKNW` where the codec read under no
    /// source or one stating no role - never `null`. The codec stamps it from
    /// the source it reads under (`FixCodec`'s `source`), and a row-header
    /// capture or a row cell named `msgpluginside` is the row's word over
    /// it; also `byTag(65043)`.
    pub msgpluginside: String,
    /// The message context a bridge handled the line in.
    #[napi(ts_type = "string | null")]
    pub msgctxid: Either<String, Null>,
    /// The session instance a bridge handled the line on.
    #[napi(ts_type = "string | null")]
    pub msgsessionid: Either<String, Null>,
    /// The session event the message was delivered as - `MsgType`,
    /// `msgsessionid`, `msgctxid` and `MsgSeqNum` joined by `:`, as
    /// `8:e7256476:9effef3e6a:1094` - where all four are stated; also
    /// `byTag(65047)`.
    #[napi(ts_type = "string | null")]
    pub msgsesseventid: Either<String, Null>,
    /// The plugin the message came into a bridge through, as the bridge's
    /// log line names it - `OMS_X1_OrderOut` in `Message received: ... from
    /// (OMS_X1_OrderOut as XM8NNITE382)`; also `byTag(65044)`.
    #[napi(ts_type = "string | null")]
    pub msgoriginator: Either<String, Null>,
    /// The conversation a bridge filed the message under - a
    /// `CONVERSATIONID` the message stated, else the `{conversationId: ..}`
    /// of its log line; also `byTag(65048)`.
    #[napi(ts_type = "string | null")]
    pub conversationid: Either<String, Null>,
}

fn capture_view(capture: &FixCapture) -> FixCaptureView {
    FixCaptureView {
        msgpluginid: or_null(capture.msgpluginid().map(ToOwned::to_owned)),
        msgpluginside: capture.msgpluginside().as_str().to_owned(),
        msgctxid: or_null(capture.msgctxid().map(ToOwned::to_owned)),
        msgsessionid: or_null(capture.msgsessionid().map(ToOwned::to_owned)),
        msgsesseventid: or_null(capture.msgsesseventid().map(ToOwned::to_owned)),
        msgoriginator: or_null(capture.msgoriginator().map(ToOwned::to_owned)),
        conversationid: or_null(capture.conversationid().map(ToOwned::to_owned)),
    }
}

/// A FIX message: its typed facts, and the content row the registry types.
///
/// The typed facts live beside the row: the event the message is - the
/// graph traits' facts - the standard header, what the line said about the
/// capture it was written for, the `Text(58)` and the metadata a bridge
/// spelled under its own namespaces. The row holds everything else the message states: the
/// dictionary fields, groups as series - each its list alone, its length the
/// count - components as structs. The schema is one non-null Struct `Field` - the only row schema -
/// and a plain object crosses as the record the core canonicalizes into that
/// order exactly as every other row is; a child stating a typed fact fills
/// the holder that owns it and leaves the row. The entries are the row read
/// as a tree, derived on the first ask; the wire is the header, the event's
/// own tags and the entries. The message compares, hashes, renders and clones
/// by its facts and its row, against the registry it was resolved against.
///
/// Every message carries its identity settled: the cross code, the first
/// stated of tags 37, 11, 41, 117, 131 and 262 stored as
/// `{kind}:{side}:{base}`, the `crosshashcode` over that stored code, the
/// `currhashcode` over everything the message says but the
/// standard header and trailer, the `curruuid` ordered by millisecond and
/// sequence with a content payload seeded by the cross hash, and the
/// `crossuuid` over the cross hash - or
/// the `curruuid` itself when no cross code names a chain. Every write settles
/// it again.
#[napi(js_name = "FixMsg")]
pub struct JsFixMsg {
    inner: CoreFixMsg,
}

impl JsFixMsg {
    /// Wrap a message the core built.
    pub(crate) const fn from_core(inner: CoreFixMsg) -> Self {
        Self { inner }
    }

    /// Borrow the message the core built.
    pub(crate) const fn as_core(&self) -> &CoreFixMsg {
        &self.inner
    }

    /// Borrow the message the core built to write it.
    pub(crate) const fn as_core_mut(&mut self) -> &mut CoreFixMsg {
        &mut self.inner
    }
}

#[napi]
impl JsFixMsg {
    /// Build a message, linking the process default when none is named.
    ///
    /// The loader widens `value`: anything `Scalar.fromJs` reads becomes the
    /// native value first, and the core alone validates and canonicalizes it
    /// against `field`. A child stating a typed fact - a header or trailer
    /// tag, a crate column, one of the FIX fields a message lifts,
    /// `Text(58)` - fills the holder that owns it and leaves the row. `SendingTime` reads UTC now
    /// when the value states none; the event's instant is the stated one,
    /// else the official transaction clock standing within the core's default
    /// one-second delay of that sending time - a `TransactTime(60)`, else a
    /// ranked `TrdRegTimestamp(769)` - else the sending time itself, the
    /// creation the stated one, else the instant, and the execution of a
    /// report stating no execution clock that instant too. What
    /// `OrigSendingTime(122)` says is the lifecycle's to read. The identity
    /// is then settled.
    #[napi(constructor)]
    pub fn new(
        field: &JsField,
        value: &JsScalar,
        registry: Option<ClassInstance<'_, JsFixRegistry>>,
    ) -> Result<Self> {
        let registry = registry_or_env(registry)?;
        CoreFixMsg::with_registry(registry, field.inner.clone(), value.inner.clone())
            .map(|inner| Self { inner })
            .map_err(napi_error)
    }

    /// The message a fixed row holds: the inverse of `intoRow`.
    ///
    /// `schema` is the row's root - the one `fix.schema` or
    /// `fix.schemaCarrying` built, or a batch reader's `field` - and `row` the
    /// row under it, which the loader widens from whatever `Scalar.fromJs`
    /// reads. The typed facts are read off the columns that hold them, and
    /// the content is rebuilt from the `fixentries` column, each entry typed
    /// through the dictionary exactly as the builder types a pair, so
    /// `intoBytes` re-emits the line the row was read from; a row without
    /// that column has the typed facts and no content. Every capture column
    /// is carried - the one the crate tags, `sourceurl`, and every column
    /// no tag and no counter names - each under its name, as `carried`
    /// answers, and held as no fact; a column whose name holds a `.` is a
    /// bridge's own statement and lands in the metadata. `intoRow` states
    /// each carried cell again at its column. Nothing is parsed again and
    /// no clock is read. The
    /// process default is the registry when none is named.
    #[napi(factory)]
    pub fn from_row(
        schema: &JsField,
        row: &JsScalar,
        registry: Option<ClassInstance<'_, JsFixRegistry>>,
    ) -> Result<Self> {
        let registry = registry_or_env(registry)?;
        CoreFixMsg::from_row(registry, &schema.inner, &row.inner)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The registry this message resolves against, sharing it.
    #[napi(getter)]
    pub fn registry(&self) -> JsFixRegistry {
        JsFixRegistry::from_arc(Arc::clone(self.inner.registry()))
    }

    /// The root Struct field: the content row's schema, holding every child
    /// the message states beyond its typed facts.
    #[napi(getter)]
    pub fn field(&self) -> JsField {
        JsField::from_core(self.inner.as_field().clone())
    }

    /// The ordered content row.
    #[napi(getter)]
    pub fn value(&self) -> JsScalar {
        JsScalar::from_core(self.inner.as_value().clone())
    }

    /// How many children the content row declares.
    ///
    /// Counts are JavaScript numbers, exact to 2^53, as everywhere else at
    /// this boundary.
    #[allow(clippy::cast_precision_loss)]
    #[napi(getter)]
    pub fn size(&self) -> f64 {
        self.inner.as_field().fields().len() as f64
    }

    /// The graph market data this message expands to: an order, a quote,
    /// an execution or a trade report is one leaf; a book `W` or `X` one per
    /// `NoMDEntries(268)` occurrence, or one scoped snapshot control for an
    /// empty `W` - each a `MarketData` carrying, in its `metadata`, what the
    /// message states that no typed column reads and no identifier map of the
    /// leaf holds, the identifiers among it lifted into the set their type
    /// belongs to - `securityids`, `partyids` or `identifiers` - where that
    /// set holds the key free or with the same value.
    #[napi]
    pub fn market_data(&self) -> Result<Vec<JsMarketData>> {
        self.inner
            .market_data()
            .map(|operations| {
                operations
                    .into_iter()
                    .map(JsMarketData::from_core)
                    .collect()
            })
            .map_err(napi_error)
    }

    /// The standard header, typed, as one plain object read once.
    #[napi]
    pub fn header(&self) -> Result<FixHeaderView> {
        header_view(self.inner.header())
    }

    /// What the line said about the capture it was written for, as one plain
    /// object read once: the plugin, the message context and the session
    /// instance. Not where the line was read from and not when it was
    /// recorded - those are the reader's, held nowhere on a message.
    #[napi]
    pub fn capture(&self) -> FixCaptureView {
        capture_view(self.inner.capture())
    }

    /// `Text(58)`: the free text the message carries, or `null`.
    #[napi(getter)]
    pub fn text(&self) -> Option<String> {
        self.inner.text().map(ToOwned::to_owned)
    }

    /// What a bridge stated under its own namespaces - a `TECH.CLIENTID`,
    /// a `firm.*` key - each under the key as the bridge spelled it, folded,
    /// in sorted order; empty where it stated none.
    #[napi(getter, ts_return_type = "Record<string, string>")]
    pub fn metadata(&self) -> BTreeMap<String, String> {
        metadata_view(self.inner.metadata())
    }

    /// The business category the message's type files under, as the
    /// `marketdatakind` member's stored name - `ORDR`, `QUOT`, `EXEC`,
    /// `TRAD`, `BOOK` - and `UKNW` where it files none.
    #[napi(getter)]
    pub fn marketdatakind(&self) -> &'static str {
        self.inner.marketdatakind().as_str()
    }

    /// The role of the plugin whose session produced this message.
    #[napi(getter)]
    pub fn msgpluginside(&self) -> &'static str {
        self.inner.msgpluginside().as_str()
    }

    /// The strike price of the option the message identifies - its market
    /// fact `strikepx`, derived from `StrikePrice(202)` - as decimal text, or
    /// `null`.
    #[napi(getter)]
    pub fn strikepx(&self) -> Option<String> {
        self.inner.get_strikepx().map(|held| held.to_string())
    }

    /// This message's own `UUIDv7` identity, ordered by millisecond and
    /// sequence with a content payload seeded by its cross hash, as
    /// hyphenated text.
    #[napi(getter)]
    pub fn curruuid(&self) -> String {
        self.inner.get_curruuid().to_string()
    }

    /// The identity of the chain this message belongs to, as its hyphenated
    /// text: `curruuid` when no cross code names a chain.
    #[napi(getter)]
    pub fn crossuuid(&self) -> String {
        self.inner.get_crossuuid().to_string()
    }

    /// The code the chain is named by, stored as `{kind}:{side}:{base}` - the
    /// `MarketDataKind` code, the `Side` code of a sided kind (`0` for any
    /// other) and the identifier the message names, so a buy order `O-1` is
    /// `10:1:O-1` - or empty where it names none.
    #[napi(getter)]
    pub fn crosscode(&self) -> String {
        self.inner.get_crosscode().to_owned()
    }

    /// The XXH3-64 over everything this message says.
    #[napi(getter)]
    pub fn currhashcode(&self) -> BigInt {
        BigInt::from(self.inner.get_currhashcode())
    }

    /// The XXH3-64 of the stored cross code, `0n` where there is none.
    #[napi(getter)]
    pub fn crosshashcode(&self) -> BigInt {
        BigInt::from(self.inner.get_crosshashcode())
    }

    /// When the event happened, nanoseconds since the Unix epoch, UTC.
    #[napi(getter)]
    pub fn currunix(&self) -> BigInt {
        instant(self.inner.get_currunix())
    }

    /// The order state the message reached, ranked: `UNKNOWN` where it
    /// states none.
    #[napi(getter)]
    pub fn state(&self) -> String {
        self.inner.get_state().as_str().to_owned()
    }

    /// The message's place among the messages of its instant: zero for the
    /// first of each run the parse hands over at that instant, with no other
    /// instant between, one more for each next; once walked, the place its
    /// content took there.
    #[napi(getter)]
    pub fn seqnum(&self) -> Result<f64> {
        exact_f64(self.inner.get_seqnum(), "seqnum")
    }

    /// The identity of the message this one follows, or `null`.
    #[napi(getter)]
    pub fn prevuuid(&self) -> Option<String> {
        self.inner.get_prevuuid().map(|uuid| uuid.to_string())
    }

    /// When the order this message belongs to was created, where known.
    #[napi(getter)]
    pub fn creaunix(&self) -> Option<BigInt> {
        self.inner.get_creaunix().map(instant)
    }

    /// The latest execution instant the lifecycle reached, where known.
    #[napi(getter)]
    pub fn execunix(&self) -> Option<BigInt> {
        self.inner.get_execunix().map(instant)
    }

    /// When the message was recorded, where stated.
    #[napi(getter)]
    pub fn recdunix(&self) -> Option<BigInt> {
        self.inner.get_recdunix().map(instant)
    }

    /// When the order expires, where it has an expiry.
    #[napi(getter)]
    pub fn exprunix(&self) -> Option<BigInt> {
        self.inner.get_exprunix().map(instant)
    }

    /// When the message this one follows happened, where it follows one.
    #[napi(getter)]
    pub fn prevunix(&self) -> Option<BigInt> {
        self.inner.get_prevunix().map(instant)
    }

    /// The grid step a walk read this message as the snapshot of, where one
    /// did.
    #[napi(getter)]
    pub fn snapunix(&self) -> Option<BigInt> {
        self.inner.get_snapunix().map(instant)
    }

    /// The sorted unique identities of the elements this one was read from:
    /// the text line it was parsed out of, and for a message the parse split
    /// off another that message's identity beside its sources; none for one
    /// parsed from raw bytes. Provenance, never its chain: no walk moves it.
    #[napi(getter)]
    pub fn srcuuids(&self) -> Vec<String> {
        sources_view(&self.inner)
    }

    /// The capture's own cells the message carries, each under the column
    /// it was read from: where the line was read from, its place in the
    /// object, the body it was cut from, when it was recorded. Provenance
    /// and never content - none is an entry, none reaches the wire or the
    /// hash code - and `intoRow` states each again at its column. Empty
    /// for a message parsed from bytes.
    #[napi(getter, ts_return_type = "Record<string, Scalar>")]
    pub fn carried(&self) -> HashMap<String, JsScalar> {
        self.inner
            .carried()
            .iter()
            .map(|(name, value)| (name.to_string(), JsScalar::from_core(value.clone())))
            .collect()
    }

    /// The security identifiers the instrument goes by, each a code under a
    /// key - `isin`, `sedol`, `figi` and any other type `SecurityIDSource(22)`
    /// or the `SecurityAltID` group names under its base key, an unmapped
    /// entry whose key names a security type under the source it names, and
    /// the codes an ISIN embeds under `derived` (`derived:cusip`) - a map
    /// keyed `src:type`, the type alone for the base source, in key order;
    /// empty where the message states none.
    #[napi(getter, ts_return_type = "Identifiers")]
    pub fn securityids(&self) -> crate::identifier::JsIdentifiers {
        crate::identifier::JsIdentifiers::from_core(self.inner.get_securityids())
    }

    /// The instrument's ISIN, borrowed from `securityids`, or `null`.
    #[napi(getter)]
    pub fn isincode(&self) -> Option<String> {
        self.inner.get_isincode().map(ToOwned::to_owned)
    }

    /// The rates an amount in `currency` is divided by to state it in each
    /// target currency, keyed by the target's `ccy` code, each rate as
    /// decimal text; empty where none - and nothing fills it yet.
    #[napi(getter, ts_return_type = "Record<string, string>")]
    pub fn fxrates(&self) -> BTreeMap<String, String> {
        self.inner
            .get_fxrates()
            .iter()
            .map(|(target, rate)| (target.as_str().to_owned(), rate.to_string()))
            .collect()
    }

    /// The bid price the message states, `BidPx(132)`, as decimal text, or
    /// `null`.
    #[napi(getter)]
    pub fn bidpx(&self) -> Option<String> {
        self.inner.get_bidpx().map(|held| held.to_string())
    }

    /// The bid size the message states, `BidSize(134)`, as decimal text, or
    /// `null`.
    #[napi(getter)]
    pub fn bidqty(&self) -> Option<String> {
        self.inner.get_bidqty().map(|held| held.to_string())
    }

    /// The currency the bid is stated in: a stated `BidCurrency`, else the
    /// message's currency where it states a bid; `null` otherwise.
    #[napi(getter)]
    pub fn bidccy(&self) -> Option<String> {
        self.inner.get_bidccy().map(|held| held.as_str().to_owned())
    }

    /// The offer price the message states, `OfferPx(133)`, as decimal text, or
    /// `null`.
    #[napi(getter)]
    pub fn askpx(&self) -> Option<String> {
        self.inner.get_askpx().map(|held| held.to_string())
    }

    /// The offer size the message states, `OfferSize(135)`, as decimal text,
    /// or `null`.
    #[napi(getter)]
    pub fn askqty(&self) -> Option<String> {
        self.inner.get_askqty().map(|held| held.to_string())
    }

    /// The currency the offer is stated in: a stated `AskCurrency` or
    /// `OfferCurrency`, else the message's currency where it states an offer;
    /// `null` otherwise.
    #[napi(getter)]
    pub fn askccy(&self) -> Option<String> {
        self.inner.get_askccy().map(|held| held.as_str().to_owned())
    }

    /// The names the operation goes by, each typed by the field that
    /// stated it - `orderid`, `clordid`, `execid`, `quoteid`, `tradeid` and
    /// the rest the message states - under the base key of its type where a
    /// FIX field stated it, or under the source an unmapped entry's key
    /// names (`OMS_ClOrdID` is `oms:clordid`, which fills `clordid` where it
    /// is empty), with the parents a chain gave them (`origclordid`,
    /// `parentorderid`, `origorderid`); a map keyed `src:type`, the type
    /// alone for the base source, in key order.
    #[napi(getter, ts_return_type = "Identifiers")]
    pub fn identifiers(&self) -> crate::identifier::JsIdentifiers {
        crate::identifier::JsIdentifiers::from_core(self.inner.get_identifiers())
    }

    /// The parties the message names - each `Parties` occurrence's
    /// `PartyID` typed by its `PartyRole`'s name, such as `executingtrader`,
    /// from its `PartyIDSource`'s (the base source where it states none),
    /// and its `Account(1)` typed `account` - a map keyed `src:type`, the
    /// type alone for the base source, each named source filling its type's
    /// base key, in key order.
    #[napi(getter, ts_return_type = "Identifiers")]
    pub fn partyids(&self) -> crate::identifier::JsIdentifiers {
        crate::identifier::JsIdentifiers::from_core(self.inner.get_partyids())
    }

    /// The price stated, as decimal text, or `null` where none is. Never a
    /// last executed price, which `lastpx` answers.
    #[napi(getter)]
    pub fn price(&self) -> Option<String> {
        self.inner.get_price().map(|held| held.to_string())
    }

    /// The quantity stated, as decimal text, or `null` where none is. Never
    /// a last executed quantity, which `lastqty` answers.
    #[napi(getter)]
    pub fn quantity(&self) -> Option<String> {
        self.inner.get_quantity().map(|held| held.to_string())
    }

    /// The stop price the message states, `StopPx(99)`, as decimal text, or `null` where none is.
    #[napi(getter)]
    pub fn stoppx(&self) -> Option<String> {
        self.inner.get_stoppx().map(|held| held.to_string())
    }

    /// The part of the quantity shown, `DisplayQty(1138)` else `MaxFloor(111)`, as decimal text, or `null` where none is.
    #[napi(getter)]
    pub fn displayqty(&self) -> Option<String> {
        self.inner.get_displayqty().map(|held| held.to_string())
    }

    /// The part of the quantity kept back: the quantity past the part shown, as decimal text, or `null` where none is.
    #[napi(getter)]
    pub fn hiddenqty(&self) -> Option<String> {
        self.inner.get_hiddenqty().map(|held| held.to_string())
    }

    /// How much was canceled, `CxlQty(84)`, as decimal text, or `null` where none is.
    #[napi(getter)]
    pub fn cxlqty(&self) -> Option<String> {
        self.inner.get_cxlqty().map(|held| held.to_string())
    }

    /// The unit the quantity is counted in, `UnitOfMeasure(996)`; empty
    /// where none is stated.
    #[napi(getter)]
    pub fn unit(&self) -> String {
        self.inner.get_unit().as_str().to_owned()
    }

    /// The type of its kind the message is, as the `marketdatatype` member's
    /// stored name: `UKNW` where none.
    #[napi(getter)]
    pub fn marketdatatype(&self) -> String {
        self.inner.get_marketdatatype().as_str().to_owned()
    }

    /// The side, as the `side` member's four-letter code: the one stated, else
    /// `UKNW` - never `null`.
    #[napi(getter)]
    pub fn side(&self) -> String {
        self.inner.get_side().as_str().to_owned()
    }

    /// The currency; `XXX` where none is stated.
    #[napi(getter)]
    pub fn currency(&self) -> String {
        self.inner.get_currency().as_str().to_owned()
    }

    /// The currency the instrument originates in, as the `ccy` code it is:
    /// the crate field `origccy` the message states, or what a registry
    /// filled; `null` where neither did, never defaulted.
    #[napi(getter)]
    pub fn origccy(&self) -> Option<String> {
        let held = self.inner.get_origccy();
        (!held.is_none()).then(|| held.as_str().to_owned())
    }

    /// The origin currency read with its default: `origccy` where held,
    /// else `currency` - `XXX` only where neither is stated.
    #[napi(getter)]
    pub fn origin_currency(&self) -> String {
        self.inner.origin_currency().as_str().to_owned()
    }

    /// The price the message last traded at, as decimal text, or `null`.
    /// FIX's own `LastPx(31)`.
    #[napi(getter)]
    pub fn lastpx(&self) -> Option<String> {
        self.inner.get_lastpx().map(|held| held.to_string())
    }

    /// What the message states that its reading could not take as it
    /// stands, in arrival order: a value that would not type, an alias
    /// stating another value than the field it lost to, what the last settle
    /// dropped.
    #[napi(getter)]
    pub fn anomalies(&self) -> Vec<FixAnomalyView> {
        self.inner
            .anomalies()
            .iter()
            .map(|held| FixAnomalyView {
                field: held.field().to_owned(),
                reason: held.reason().to_owned(),
            })
            .collect()
    }

    /// The quantity it last traded, `LastQty(32)`, or `null`.
    #[napi(getter)]
    pub fn lastqty(&self) -> Option<String> {
        self.inner.get_lastqty().map(|held| held.to_string())
    }

    /// FIX's own `LastSpotRate(194)`, the spot rate of the last price, as decimal text, or `null`.
    #[napi(getter)]
    pub fn lastspotrate(&self) -> Option<String> {
        self.inner
            .lifted()
            .lastspotrate()
            .map(|held| held.to_string())
    }

    /// FIX's own `LastForwardPoints(195)`, the forward points of the last price, as decimal text, or `null`.
    #[napi(getter)]
    pub fn lastforwardpoints(&self) -> Option<String> {
        self.inner
            .lifted()
            .lastforwardpoints()
            .map(|held| held.to_string())
    }

    /// FIX's own `BidSpotRate(188)`, the bid lane's spot rate, as decimal text, or `null`.
    #[napi(getter)]
    pub fn bidspotrate(&self) -> Option<String> {
        self.inner
            .lifted()
            .bidspotrate()
            .map(|held| held.to_string())
    }

    /// FIX's own `BidForwardPoints(189)`, the bid lane's forward points, as decimal text, or `null`.
    #[napi(getter)]
    pub fn bidforwardpoints(&self) -> Option<String> {
        self.inner
            .lifted()
            .bidforwardpoints()
            .map(|held| held.to_string())
    }

    /// FIX's own `OfferSpotRate(190)`, the ask lane's spot rate, as decimal text, or `null`.
    #[napi(getter)]
    pub fn offerspotrate(&self) -> Option<String> {
        self.inner
            .lifted()
            .offerspotrate()
            .map(|held| held.to_string())
    }

    /// FIX's own `OfferForwardPoints(191)`, the ask lane's forward points, as decimal text, or `null`.
    #[napi(getter)]
    pub fn offerforwardpoints(&self) -> Option<String> {
        self.inner
            .lifted()
            .offerforwardpoints()
            .map(|held| held.to_string())
    }

    /// The price it averaged, `AvgPx(6)`, or `null`.
    #[napi(getter)]
    pub fn avgpx(&self) -> Option<String> {
        self.inner.get_avgpx().map(|held| held.to_string())
    }

    /// How much of its quantity is done, `CumQty(14)`, or `null`.
    #[napi(getter)]
    pub fn cumqty(&self) -> Option<String> {
        self.inner.get_cumqty().map(|held| held.to_string())
    }

    /// How much of it is still open, `LeavesQty(151)`, or `null`.
    #[napi(getter)]
    pub fn leavesqty(&self) -> Option<String> {
        self.inner.get_leavesqty().map(|held| held.to_string())
    }

    /// The price stated before this message - its own closing price, else
    /// what the statement it follows settled on, which a walk fills.
    #[napi(getter)]
    pub fn prevpx(&self) -> Option<String> {
        self.inner.get_prevpx().map(|held| held.to_string())
    }

    /// The quantity that statement settled on, or `null`.
    #[napi(getter)]
    pub fn prevqty(&self) -> Option<String> {
        self.inner.get_prevqty().map(|held| held.to_string())
    }

    /// The spot part of an FX forward price, as decimal text, or `null`.
    #[napi(getter)]
    pub fn spotrate(&self) -> Option<String> {
        self.inner.get_spotrate().map(|held| held.to_string())
    }

    /// The forward points of an FX forward price, as decimal text, or
    /// `null`.
    #[napi(getter)]
    pub fn forwardpoints(&self) -> Option<String> {
        self.inner.get_forwardpoints().map(|held| held.to_string())
    }

    /// How long the message stands, `TimeInForce(59)`, as the `timeinforce`
    /// member's stored name - `DAY`, `GTC`, a venue's own value `OTHER` -
    /// or `null` where none is stated.
    #[napi(getter)]
    pub fn timeinforce(&self) -> Option<String> {
        self.inner
            .get_timeinforce()
            .map(|held| held.as_str().to_owned())
    }

    /// The quantity the order asked for, as decimal text, or `null` where
    /// none is stated.
    #[napi(getter)]
    pub fn ordqty(&self) -> Option<String> {
        self.inner.get_ordqty().map(|held| held.to_string())
    }

    /// Whether the instrument could be traded when the message was sent, or
    /// `null` where the market said nothing either way - which is not the
    /// same as `false`.
    #[napi(getter)]
    pub fn tradable(&self) -> Option<bool> {
        self.inner.get_tradable()
    }

    /// The ticker the instrument is known by, `Symbol(55)`, or `null` where
    /// it has none and the security identifiers are what name it.
    #[napi(getter)]
    pub fn ticker(&self) -> Option<String> {
        self.inner.get_ticker().map(ToOwned::to_owned)
    }

    /// The instrument's classification, read off `CFICode(461)` and what the
    /// message says about the security, or `null` where nothing does.
    #[napi(getter)]
    pub fn cficode(&self) -> Option<String> {
        self.inner
            .get_cficode()
            .map(|held| held.as_str().to_owned())
    }

    /// The market, read off `SecurityExchange(207)`, `ExDestination(100)` or
    /// `LastMkt(30)`, the first that names an ISO 10383 MIC.
    #[napi(getter)]
    pub fn miccode(&self) -> Option<String> {
        self.inner
            .get_miccode()
            .map(|held| held.as_str().to_owned())
    }

    /// The value the root child an identifier names, or `null`.
    ///
    /// An identifier is exact and does not fold: `id` is the number
    /// `field.fix.id` answers, and a field the dictionary does not hold
    /// under it simply misses. A typed fact answers from its holder.
    #[napi]
    pub fn get_by_id(&self, id: f64) -> Result<Option<JsScalar>> {
        let id = id_from_js(id)?;
        Ok(self.inner.get_by_id(id).map(JsScalar::from_core))
    }

    /// The value the root child an identifier names.
    #[napi]
    pub fn by_id(&self, id: f64) -> Result<JsScalar> {
        let id = id_from_js(id)?;
        self.inner
            .by_id(id)
            .map(JsScalar::from_core)
            .map_err(napi_error)
    }

    /// The value a tag names, or `null`.
    ///
    /// A tag the typed holders own - a header or trailer tag, a crate
    /// column, one of the FIX fields a message lifts, `Text(58)` - answers
    /// the fact the holder states as the `Scalar` its column types:
    /// `byTag(35)` is the type's text, `byTag(52)` the sending clock as
    /// `datetime64(ns, UTC)`, `byTag(44)` an exact price, a crate tag its
    /// column's own type - a
    /// `uuid`, a `uint64`, a `decimal128(38, 18)`. Any other tag reaches
    /// the row through the dictionary: the canonical holder first, then an
    /// alternate, then a child spelled by the tag's decimal.
    #[napi]
    pub fn get_by_tag(&self, tag: f64) -> Result<Option<JsScalar>> {
        let tag = exact_i32(tag, "tag")?;
        Ok(self.inner.get_by_tag(tag).map(JsScalar::from_core))
    }

    /// The value a tag names.
    #[napi]
    pub fn by_tag(&self, tag: f64) -> Result<JsScalar> {
        let tag = exact_i32(tag, "tag")?;
        self.inner
            .by_tag(tag)
            .map(JsScalar::from_core)
            .map_err(napi_error)
    }

    /// The value a name reaches, or `null`.
    ///
    /// The name folds through the dictionary: the canonical spelling first,
    /// then an alias; a typed fact answers from its holder.
    #[napi]
    pub fn get_by_name(&self, name: String) -> Option<JsScalar> {
        self.inner.get_by_name(&name).map(JsScalar::from_core)
    }

    /// The value a name reaches.
    #[napi]
    pub fn by_name(&self, name: String) -> Result<JsScalar> {
        self.inner
            .by_name(&name)
            .map(JsScalar::from_core)
            .map_err(napi_error)
    }

    /// The value a path reaches, or `null`.
    #[napi(ts_args_type = "path: string | FieldPath")]
    pub fn get_by_path(&self, path: Either<String, &JsFieldPath>) -> Result<Option<JsScalar>> {
        let path = path_from_input(path)?;
        Ok(self.inner.get_by_path(&path).map(JsScalar::from_core))
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
            .map(JsScalar::from_core)
            .map_err(napi_error)
    }

    /// The value a tag or a name reaches, or `null`.
    #[napi(ts_args_type = "key: number | string")]
    pub fn get(&self, env: Env, key: Unknown<'_>) -> Result<Option<JsScalar>> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        Ok(self.inner.get(key.as_key()).map(JsScalar::from_core))
    }

    /// The value a tag or a name reaches.
    ///
    /// The failing half of `get` is spelled `at` rather than the core's
    /// `value`, because `value` is this class's property for the whole
    /// content row.
    #[napi(ts_args_type = "key: number | string")]
    pub fn at(&self, env: Env, key: Unknown<'_>) -> Result<JsScalar> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        self.inner
            .value(key.as_key())
            .map(JsScalar::from_core)
            .map_err(napi_error)
    }

    /// Writes one value into the message, typed by the field the key
    /// resolves to.
    ///
    /// `key` is a tag or a name, resolved as a lookup resolves one - through
    /// the dictionary, canonical before alternate or alias - and a name the
    /// dictionary does not know still reaches a child spelled that way.
    /// `value` is whatever `Scalar.fromJs` reads, widened by the loader. A
    /// key reaching a typed fact records it on the holder that owns it, and
    /// `null` clears it; any other key lands in the row: a known field types
    /// the value through the core's value contract, `null` is stored as a
    /// stated null, an existing child is replaced where it stands and an
    /// absent one appended, and a bare tag no dictionary explains appends a
    /// text child named by its decimal. The entries and the wire follow the
    /// row, and the identity is settled again. No clock is read.
    ///
    /// A key reaching no field and no child, or a value the field refuses,
    /// throws the core's refusal and leaves the message as it was. So does a
    /// key reaching the capture's own column - `sourceurl` (65052), by tag
    /// or by name: a message holds no fact for it, and a row child would put
    /// it on the wire.
    #[napi(ts_args_type = "key: number | string, value: unknown")]
    pub fn set(&mut self, env: Env, key: Unknown<'_>, value: &JsScalar) -> Result<()> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        self.inner
            .set(key.as_key(), value.inner.clone())
            .map_err(napi_error)
    }

    /// Removes what a key reaches, answering the value it held, or `null`.
    ///
    /// The key resolves as `set` resolves one: a typed fact is cleared on
    /// its holder, a row child leaves the row, and a key reaching nothing
    /// answers `null` and changes nothing. The identity is settled again.
    #[napi(ts_args_type = "key: number | string")]
    pub fn remove(&mut self, env: Env, key: Unknown<'_>) -> Result<Option<JsScalar>> {
        let key = FixKeyArg::from_js(env, &key, "key")?;
        Ok(self
            .inner
            .remove(key.as_key())
            .map_err(napi_error)?
            .map(JsScalar::from_core))
    }

    /// The content row read as a tree: one entry per child it states, a
    /// group's occurrences and a component's members nested under the entry
    /// that heads them, nothing for a child stating null.
    ///
    /// Derived on the first ask and kept until a write. The typed facts are
    /// not entries: the header, the event and the capture are the holders'
    /// to answer, and the wire `intoBytes` emits puts the header and the
    /// lifted FIX fields in front of these and the trailer behind them. The
    /// loader wires
    /// `Symbol.iterator` over this.
    #[napi]
    pub fn entries(&self) -> Vec<FixEntryView> {
        self.inner.entries().iter().map(entry_view).collect()
    }

    /// The digest of what this message emits on the wire, as sixteen bytes.
    ///
    /// Over every entry of `intoBytes`, pre-order, so two messages that
    /// re-emit alike digest alike whatever separator either was read with.
    #[napi]
    pub fn digest(&self) -> Buffer {
        Buffer::from(self.inner.digest().to_be_bytes().to_vec())
    }

    /// This message as the fixed row a table holds.
    ///
    /// `schema` is the fixed root `fixSchema` builds: every column is filled by
    /// the tag its field carries - never by its spelling - so a message that
    /// carried nothing at a column answers null there rather than shifting its
    /// neighbours. A typed fact fills its column from its holder, a group's
    /// column from the message's own occurrences, and the capture's own
    /// columns answer null - the one the crate tags, `sourceurl`, and every
    /// column no tag and no counter names - because a message holds no fact
    /// for any of them; the capture readers state them on the row instead.
    /// The arrival record closes the row under `fixentries`, unresolved keys
    /// at tag 0.
    ///
    /// A value a column will not hold is that column's null; a column that
    /// cannot be null keeps the refusal, and throws it located. No clock is
    /// read.
    #[napi]
    pub fn into_row(&self, schema: &JsField) -> Result<JsScalar> {
        self.inner
            .into_row(&schema.inner)
            .map(JsScalar::from_core)
            .map_err(napi_error)
    }

    /// Re-emit this message on the wire, separated by `separator`: the
    /// standard header - `SendingTime` only when the message stated it - the
    /// event's own FIX tags, then the content entries, derived values
    /// included, coded facts as their wire code.
    #[napi]
    pub fn into_bytes(&self, separator: Option<f64>) -> Result<Buffer> {
        let separator = separator_byte(separator)?;
        Ok(Buffer::from(self.inner.into_bytes(separator)))
    }

    /// The same as `intoBytes`, as text, separated by one character - the
    /// FIX `SOH` when unstated.
    ///
    /// A value holding a control byte throws the core's refusal.
    #[napi]
    pub fn into_text(&self, separator: Option<String>) -> Result<String> {
        let separator = separator_char(separator)?;
        self.inner.into_text(separator).map_err(napi_error)
    }

    /// Whether two messages carry the same facts, schema, row and
    /// dictionary.
    #[napi]
    pub fn equals(&self, other: &JsFixMsg) -> bool {
        self.inner == other.inner
    }

    /// Deterministic hash bits over the facts, the schema and the row.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// A cheap clone: the schema and row are shared, the registry link kept.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    /// A one-line summary naming the root and how many values the row holds.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        format!(
            "FixMsg({:?}, {} values)",
            self.inner.as_field().name(),
            self.inner.as_field().fields().len()
        )
    }

    /// The content row's schema document and value document.
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

/// The lifecycle's deduplication window a caller states: a whole number of
/// milliseconds, `null` remembering none as a nonpositive window does.
fn dedup_window_of(window: Either<f64, Null>) -> Result<i64> {
    match window {
        Either::A(held) => exact_i64(held, "dedupWindowMs"),
        Either::B(Null) => Ok(0),
    }
}

/// One separator character, or the FIX default where none was named.
fn separator_char(separator: Option<String>) -> Result<char> {
    let Some(held) = separator else {
        return Ok(char::from(SOH));
    };
    let mut characters = held.chars();
    match (characters.next(), characters.next()) {
        (Some(character), None) => Ok(character),
        _ => Err(napi_error("a separator is one character")),
    }
}

/// The separator FIX itself uses.
const SOH: u8 = 0x01;

/// A stream of messages, one at a time.
///
/// Every stage of the codec answers one of these - one line's messages, a
/// stream of lines parsed, records parsed, messages filled or stamped, a
/// batch read back - so a message stream has one shape at this boundary
/// whatever made it. Nothing is collected: the core iterator is the stream,
/// and a JavaScript iterable behind it is pulled one item at a time. What a
/// line or a message states that cannot stand is passed over with a warning
/// on standard error, never thrown; a failure in the iterable behind the
/// stream throws and ends it. The loader supplies `Symbol.iterator` over
/// `next`.
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
    /// A line the reader passed over is no item; a failure behind the stream
    /// throws once, in place of the end.
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
/// Every message it builds is settled as it is parsed: the typed facts are
/// lifted off the line, a nested `XmlData` is exploded into the message,
/// deprecated fields are restated to their latest aliases, the dictionary's
/// native derivations run, the identifier maps and the security identifiers
/// fill, and
/// the identity is derived. `SendingTime` is the message's valid tag 52,
/// else a row cell reaching that tag, else the `mtime` of the `TextLine` it
/// was read out of - on `parseTextArrowReader`, the row's `currunix` cell -
/// else `defaultSendingTime`, else UTC now read once for that new message,
/// and it goes back on the wire only when the message stated it: a clock
/// the parse supplied is never the message's own, so the row's `sendingtime`
/// column states none either. The raw-byte doors read no line, so parsing
/// undated bytes there without a default sending time is deliberately not
/// deterministic. A message reporting an execution that states no execution
/// clock executed at its instant: its `execunix` is its `currunix`.
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
    /// empty; `batchByteSize` and `batchRowSize` are the raw bytes and the
    /// rows one Arrow batch targets, the core's 128 MiB and 32,768 rows when
    /// unstated, whichever the batch reaches first; `threads` is how many
    /// workers parsing and row conversion use, the available CPUs when
    /// unstated. Arrow capture parsing keeps at most one input batch per
    /// worker and yields in order; line doors use bounded row chunks.
    /// One thread reads lazily without a pool; zero reads as one;
    /// `includeMsgtypes` and
    /// `excludeMsgtypes` are the message types a parse keeps and refuses,
    /// each read before a frame is built and spelled as a code or a name -
    /// `"0"`, `"Heartbeat"`, `"unknown"` for a line stating no type - the
    /// core refusing `Heartbeat`, `TestRequest` and the untyped line when
    /// unstated and an empty `excludeMsgtypes` keeping every type;
    /// `defaultSendingTime` is the
    /// `SendingTime` an undated message takes when nothing it was read with
    /// dates it either, neither a capture reaching tag 52 nor the `mtime` of
    /// the line it was read out of - a `Scalar` crosses as it is and must
    /// already be `DateTime64(ns, UTC)`, a `Date` is its UTC millisecond
    /// instant restated in nanoseconds, and `null` or absence reads UTC now
    /// per new message.
    /// `snapshotNs` is an epoch-aligned lifecycle snapshot width in exact
    /// nanoseconds; `null`, zero and a negative width disable snapshots;
    /// `sortedLifecycle` states that the messages `lifecycle` is handed
    /// arrive in instant order, so the walk reads them as they come one
    /// epoch hour at a time rather than collecting and sorting the whole
    /// capture, off when unstated;
    /// `officialTimeDelayMs` is how far from `SendingTime(52)` an official
    /// transaction clock may stand and still date the message, the core's
    /// one second when unstated; `dedupWindowMs` is how long, in
    /// milliseconds of event time, `lifecycle` remembers an identity it
    /// yielded so it yields that identity once - the core's one minute when
    /// unstated, and `null`, zero or a negative window remembering none;
    /// `marketMetadata` is whether a market operation carries its message's
    /// unmapped fields - its parties, `Account(1)` and regulatory trade
    /// identifiers stay its `partyids` and `identifiers` - and lifts the
    /// identifiers among them into the set their type belongs to, on when
    /// unstated; `isinRegistry` is the `IsinRegistry` every `lifecycle`
    /// learns into and fills from, shared so a walk run after another starts
    /// from what the first learned, each walk learning into its own when
    /// unstated; `source` is the id of the dictionary's source the codec
    /// reads under, whose catalog entry's plugin side every message it
    /// builds states as its `msgpluginside` - a row-header capture or a row
    /// cell named `msgpluginside` being the row's word over it - resolved
    /// once and refused when the catalog holds no such id, `UKNW` on every
    /// message when unstated.
    #[napi(constructor)]
    pub fn new(
        registry: Option<ClassInstance<'_, JsFixRegistry>>,
        options: Option<FixCodecOptions<'_>>,
    ) -> Result<Self> {
        Self::open(registry, options)
    }

    /// A codec over the registry the process environment names,
    /// `FixRegistry.fromEnv()`, sharing the instrument registry it names
    /// too, `IsinRegistry.fromEnv()` - unless `isinRegistry` names another
    /// - pinned by the options the constructor takes. The one constructor
    /// that attaches the process's own; `new FixCodec(...)` attaches none,
    /// and a commit of what the walks learned is always the caller's.
    #[napi(factory)]
    pub fn from_env(options: Option<FixCodecOptions<'_>>) -> Result<Self> {
        let attach = options
            .as_ref()
            .is_none_or(|options| options.isin_registry.is_none());
        let mut codec = Self::open(None, options)?;
        if attach {
            let instruments = yggdryl::IsinRegistry::from_env().map_err(napi_error)?;
            codec.inner = codec.inner.with_isin_registry(Arc::clone(instruments));
        }
        Ok(codec)
    }
}

impl JsFixCodec {
    fn open(
        registry: Option<ClassInstance<'_, JsFixRegistry>>,
        options: Option<FixCodecOptions<'_>>,
    ) -> Result<Self> {
        let registry = registry_or_env(registry)?;
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
        if let Some(held) = options.batch_row_size {
            let rows = exact_i64(held, "batchRowSize")?;
            let rows = usize::try_from(rows)
                .map_err(|_| napi_error("batchRowSize must not be negative"))?;
            inner = inner.with_batch_row_size(rows);
        }
        if let Some(held) = options.threads {
            let threads = exact_i64(held, "threads")?;
            let threads =
                usize::try_from(threads).map_err(|_| napi_error("threads must not be negative"))?;
            inner = inner.with_threads(threads);
        }
        if let Some(Either::A(held)) = options.snapshot_ns {
            let snapshot_ns = crate::exact_i128(&held, "snapshotNs")?;
            let snapshot_ns = i64::try_from(snapshot_ns)
                .map_err(|_| napi_error("snapshotNs must be a signed 64-bit integer"))?;
            inner = inner.with_snapshot_ns(snapshot_ns);
        }
        if let Some(held) = options.sorted_lifecycle {
            inner = inner.with_sorted_lifecycle(held);
        }
        if let Some(held) = options.official_time_delay_ms {
            let delay = exact_i64(held, "officialTimeDelayMs")?;
            inner = inner.with_official_time_delay_ms(delay);
        }
        if let Some(held) = options.dedup_window_ms {
            inner = inner.with_dedup_window_ms(dedup_window_of(held)?);
        }
        if let Some(held) = options.include_msgtypes {
            inner = inner.with_include_msgtypes(held);
        }
        if let Some(held) = options.exclude_msgtypes {
            inner = inner.with_exclude_msgtypes(held);
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
        if let Some(held) = &options.isin_registry {
            inner = inner.with_isin_registry(Arc::clone(&held.inner));
        }
        if let Some(held) = options.market_metadata {
            inner = inner.with_market_metadata(held);
        }
        if let Some(held) = &options.source {
            inner = inner.with_source(held).map_err(napi_error)?;
        }
        Ok(Self { inner, registry })
    }
}

#[napi]
impl JsFixCodec {
    /// The dictionary this codec resolves against, sharing it.
    #[napi(getter)]
    pub fn registry(&self) -> JsFixRegistry {
        JsFixRegistry::from_arc(Arc::clone(&self.registry))
    }

    /// The `IsinRegistry` every `lifecycle` this codec runs shares - the
    /// same table the caller holds - or `null` where each walk learns into
    /// its own.
    #[napi(getter)]
    pub fn isin_registry(&self) -> Option<JsIsinRegistry> {
        self.inner.isin_registry().map(JsIsinRegistry::from_shared)
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

    /// The source this codec reads under, folded: the dictionary's
    /// catalog entry whose plugin side every message it builds states as
    /// its `msgpluginside`; `null` where none was named.
    #[napi(getter)]
    pub fn source(&self) -> Option<String> {
        self.inner.source().map(ToOwned::to_owned)
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

    /// The rows one Arrow batch targets.
    #[allow(clippy::cast_precision_loss)]
    #[napi(getter)]
    pub fn batch_row_size(&self) -> f64 {
        self.inner.batch_row_size() as f64
    }

    /// The workers parsing and row conversion use; available CPUs by default.
    #[allow(clippy::cast_precision_loss)]
    #[napi(getter)]
    pub fn threads(&self) -> f64 {
        self.inner.threads() as f64
    }

    /// The epoch-aligned lifecycle snapshot width in nanoseconds, or `null`
    /// where snapshots are disabled.
    #[napi(getter)]
    pub fn snapshot_ns(&self) -> Option<BigInt> {
        self.inner.snapshot_ns().map(BigInt::from)
    }

    /// This codec with its lifecycle snapshot grid set to `snapshotNs`, an
    /// epoch-aligned width in exact nanoseconds: `null`, zero and a negative
    /// width disable snapshots. Every other setting, the dictionary included,
    /// is this codec's, so a caller's codec gains a grid without its options
    /// being listed again.
    #[napi]
    pub fn with_snapshot_ns(&self, snapshot_ns: Either<BigInt, Null>) -> Result<Self> {
        let width = match snapshot_ns {
            Either::A(held) => {
                let held = crate::exact_i128(&held, "snapshotNs")?;
                i64::try_from(held)
                    .map_err(|_| napi_error("snapshotNs must be a signed 64-bit integer"))?
            }
            Either::B(_) => 0,
        };
        Ok(Self {
            inner: self.inner.clone().with_snapshot_ns(width),
            registry: Arc::clone(&self.registry),
        })
    }

    /// Whether `lifecycle` reads its messages as they come, in instant
    /// order, one epoch hour at a time.
    #[napi(getter)]
    pub fn sorted_lifecycle(&self) -> bool {
        self.inner.sorted_lifecycle()
    }

    /// This codec with its lifecycle pinned to messages arriving in instant
    /// order - a table read hour partition by hour partition, sorted by
    /// `currunix` - when `sorted`: the walk then holds one epoch hour at a
    /// time, sorts within it exactly as a whole capture is sorted, and walks
    /// an hour once a message two hours past it is read; a message dated
    /// before an hour already walked is walked where it arrives. `false`
    /// collects and sorts the whole capture. Every other setting is this
    /// codec's.
    #[napi]
    pub fn with_sorted_lifecycle(&self, sorted: bool) -> Self {
        Self {
            inner: self.inner.clone().with_sorted_lifecycle(sorted),
            registry: Arc::clone(&self.registry),
        }
    }

    /// How far from `SendingTime(52)` an official transaction clock may
    /// stand and still date the message, in milliseconds.
    ///
    /// A millisecond count is a JavaScript number, exact to 2^53, as every
    /// count at this boundary is.
    #[allow(clippy::cast_precision_loss)]
    #[napi(getter)]
    pub fn official_time_delay_ms(&self) -> f64 {
        self.inner.official_time_delay_ms() as f64
    }

    /// How long, in milliseconds of event time, `lifecycle` remembers an
    /// identity it yielded so it yields that identity once, or `null` where
    /// it remembers none.
    #[allow(clippy::cast_precision_loss)]
    #[napi(getter)]
    pub fn dedup_window_ms(&self) -> Option<f64> {
        self.inner.dedup_window_ms().map(|window| window as f64)
    }

    /// This codec with its lifecycle remembering the identities it yielded
    /// for `dedupWindowMs` milliseconds of event time: `null`, zero and a
    /// negative window remember none. Every other setting, the dictionary
    /// included, is this codec's.
    #[napi]
    pub fn with_dedup_window_ms(&self, dedup_window_ms: Either<f64, Null>) -> Result<Self> {
        Ok(Self {
            inner: self
                .inner
                .clone()
                .with_dedup_window_ms(dedup_window_of(dedup_window_ms)?),
            registry: Arc::clone(&self.registry),
        })
    }

    /// The message types a parse keeps, empty where it keeps every type the
    /// refusals leave.
    #[napi(getter)]
    pub fn include_msgtypes(&self) -> Vec<String> {
        self.inner
            .include_msgtypes()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// The message types a parse refuses before it builds a frame.
    #[napi(getter)]
    pub fn exclude_msgtypes(&self) -> Vec<String> {
        self.inner
            .exclude_msgtypes()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Whether a market operation this codec builds carries, in its
    /// metadata, what its message states that no typed column reads and no
    /// identifier map of the leaf holds, and lifts the identifiers among
    /// them into the set their type belongs to.
    #[napi(getter)]
    pub fn market_metadata(&self) -> bool {
        self.inner.market_metadata()
    }

    /// The `SendingTime` an undated message takes - one neither its row nor
    /// its line dates - `DateTime64(ns, UTC)`, or `null` where each new
    /// undated message reads UTC now.
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
    /// not a row at all is passed over with a warning and the stream reads
    /// on; an item that is not bytes throws and ends it.
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
    /// capture's name reaches. A `timestamp` capture is context and stamps
    /// nothing; the line's own clock does. Its `mtime` - an `mtime` capture,
    /// else its handle's modification time - is the message's `recdunix`,
    /// and the sending clock of a message stating none: `SendingTime` is the
    /// message's own, else a capture reaching that field, else the line's
    /// `mtime`, else the codec's `defaultSendingTime`, else UTC now, and the
    /// instant `currunix` is read against it - the stated one, else the
    /// official clock standing within `officialTimeDelayMs` of it, else it.
    /// A clock the parse supplied is never the message's own: neither the
    /// wire nor the row's `sendingtime` column states it.
    /// `withCaptureNames` is what decides which capture is which, once for
    /// the whole run, because a line answers its captures by position.
    ///
    /// A `msgpluginid` capture fills the crate's own `msgpluginid` field and
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
    /// line door parses one - a row's `currunix` cell is its line's clock,
    /// so it is the messages' `recdunix` and the sending clock of one
    /// stating none - and batches close on the bytes each row lands as
    /// against `batchByteSize`. The source is consumed.
    ///
    /// The capture's own columns fill nothing: the carried ones, and the one
    /// the crate tags - a `sourceurl` column - are read off the source row
    /// and written straight into the row this answers. This is the one door
    /// that can state them, and it is why they survive a parse without a
    /// message holding one.
    #[napi]
    pub fn parse_text_arrow_reader(&self, source: &mut JsBatchReader) -> Result<JsBatchReader> {
        let parsed = self
            .inner
            .parse_text_arrow_reader(source.take()?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(parsed, ROOT_NAME))
    }

    /// A stream of batches of FIX rows walked through one lifecycle.
    ///
    /// `lifecycle` over batches: each row is a message through
    /// `FixMsg.fromRow`, stated as the one after the live message it follows,
    /// and written back under the **same** schema, so a carried column
    /// returns to its place. Nothing is parsed again. The source is consumed.
    ///
    /// A carried column returns to its place because the message carries
    /// it: each row's own cells are read into the message it made, under
    /// their names, and stated again where that message lands. The pairing
    /// is by message and never by position - a walk answers messages
    /// in their own order, which a capture's lines are routinely not in.
    #[napi]
    pub fn lifecycle_arrow_reader(&self, source: &mut JsBatchReader) -> Result<JsBatchReader> {
        let walked = self
            .inner
            .lifecycle_arrow_reader(source.take()?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(walked, ROOT_NAME))
    }

    /// A stream of batches of FIX rows as the stream of messages it holds.
    ///
    /// Each row is one message through `FixMsg.fromRow` under the source's
    /// schema, lazily, one batch held at a time: a batch
    /// `parseTextArrowReader` wrote comes back as the messages that made it
    /// without a parse. One half of what the Arrow twins compose;
    /// `arrowReader` is the other. The source is consumed.
    ///
    /// The capture's own columns are carried: each row's own cells are read
    /// into the message it makes, so `arrowReader(schema, messages(reader))`
    /// states them again exactly as `lifecycleArrowReader(reader)` does.
    #[napi]
    pub fn messages(&self, source: &mut JsBatchReader) -> Result<JsFixMessages> {
        Ok(JsFixMessages::over(self.inner.messages(source.take()?)))
    }

    /// A stream of messages as a stream of batches of FIX rows under `schema`.
    ///
    /// The loader turns the iterable into the pull function this takes, and
    /// the reader pulls one message at a time as its batches are read. Each
    /// message fills one row through `FixMsg.intoRow`, and batches close on
    /// the bytes each row lands as against `batchByteSize`. An item that is not a message is the reader's error,
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

    /// Streams sorted FIX messages through native market operations and the
    /// stateful book iterator into Arrow batches of lifted `marketdata` rows,
    /// one `book_event` row per book.
    ///
    /// Folds orders, quotes and `W`/`X` book messages into each book's
    /// sides - the orders and quotes its instant applied its `delta` - and
    /// records every execution among its `events`, moving no side, since a
    /// fill moves a book through its order's or quote's report; a trade and
    /// every other record are ignored before they are expanded. Source
    /// errors still fail.
    ///
    /// The loader supplies the iterable pull. `snapshotMillis` enables an
    /// epoch-aligned snapshot grid. `filter` - a `Filter`, a `Term` or the
    /// text of a predicate over the `marketdata` row - narrows what the
    /// books fold, bound once here; it never admits a pruned kind. Not
    /// given, every recorded leaf is kept. Lifecycle enrichment remains an
    /// explicit composition.
    #[napi(js_name = "_bookArrowReaderNative", skip_typescript)]
    pub fn book_arrow_reader_native(
        &self,
        env: Env,
        pull: Function<'_, (), Option<ClassInstance<'static, JsFixMsg>>>,
        snapshot_millis: f64,
        filter: Option<
            Either3<
                ClassInstance<'_, crate::expression::JsFilter>,
                ClassInstance<'_, crate::expression::JsTerm>,
                String,
            >,
        >,
    ) -> Result<JsBatchReader> {
        let snapshot_millis = exact_i64(snapshot_millis, "snapshotMillis")?;
        let snapshot_millis = u64::try_from(snapshot_millis)
            .map_err(|_| napi_error("snapshotMillis must not be negative"))?;
        let filter = filter
            .map(crate::expression::filter_from_input)
            .transpose()?;
        let pulled = Pulled::new(env, pull)?;
        let failed = pulled.failed.clone();
        let messages = pulled
            .map(|message| Ok(message.inner.clone()))
            .chain(std::iter::from_fn(move || {
                failed.take().map(|error| Err(javascript_failure(error)))
            }));
        let reader = self
            .inner
            .book_arrow_reader(messages, snapshot_millis, filter.as_ref())
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, "marketdata"))
    }

    /// A capture of messages as the market data a book folds, in the order
    /// it folds them.
    ///
    /// Admits what `bookArrowReader` admits and expands each admitted
    /// message as `FixMsg.marketData` does, each leaf carrying its
    /// message's unmapped fields where `marketMetadata` says so. The capture
    /// is collected when this is called - it is bounded by its own size -
    /// and the operations are stably sorted by `snapunix`, else `currunix`:
    /// the instant a book folds them at. A source error and the refusal of
    /// an admitted message's expansion are yielded first, in source order,
    /// each thrown by its own `next`; neither the lifecycle nor the msgtype
    /// filter runs here. The loader supplies the iterable pull, and a
    /// failure of the iterable itself throws once, in place of the end.
    #[napi(js_name = "_marketDataNative", skip_typescript)]
    pub fn market_data_native(
        &self,
        env: Env,
        pull: Function<'_, (), Option<ClassInstance<'static, JsFixMsg>>>,
    ) -> Result<JsMarketDataRowIterator> {
        let pulled = Pulled::new(env, pull)?;
        let failed = pulled.failed.clone();
        let messages = pulled
            .map(|message| Ok(message.inner.clone()))
            .chain(std::iter::from_fn(move || {
                failed.take().map(|error| Err(javascript_failure(error)))
            }));
        Ok(JsMarketDataRowIterator::over(Box::new(
            self.inner.market_data(messages),
        )))
    }

    /// `marketData` as bounded Arrow batches of lifted `marketdata`
    /// rows, closing as `bookArrowReader` closes them. Every failure is met
    /// before the first operation, so one intake failure - a failure of the
    /// iterable included - is the reader's only item.
    #[napi(js_name = "_marketArrowReaderNative", skip_typescript)]
    pub fn market_arrow_reader_native(
        &self,
        env: Env,
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
            .market_arrow_reader(messages)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, "marketdata"))
    }

    /// A stream of batches of FIX rows as batches of lifted `marketdata`
    /// rows: `messages` into `marketArrowReader`, each row read as its own
    /// message. Over rows no walk wrote it answers what `marketArrowReader`
    /// answers for their messages. A walked capture reaches the sorted door
    /// as messages - `marketArrowReader(codec.lifecycle(messages))` - never
    /// as the rows `lifecycleArrowReader` writes: what a walk settles from a
    /// message's predecessors (its `prevpx` and `prevqty`, a side or a ticker
    /// carried forward, an execution instant) is no cell of the row. A
    /// schema making no FIX root is refused before a row is read. The source
    /// is consumed.
    #[napi]
    pub fn market_data_arrow_reader(&self, source: &mut JsBatchReader) -> Result<JsBatchReader> {
        let reader = self
            .inner
            .market_data_arrow_reader(source.take()?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, "marketdata"))
    }

    /// A stream of messages as the rows one message field holds them.
    ///
    /// The third verb, and the one a consumer reads by: `parse*` turns a
    /// capture into messages, `lifecycle` states what each follows, and this
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
    /// still lead the row - stated from the cells each message carries. A
    /// source carrying no arrival record is a
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

    /// Walks a stream of messages through one lifecycle, lazily.
    ///
    /// The one walk over events: the messages are collected, sorted by
    /// instant, and each is stated as the one after the live message it
    /// follows - the last message of its chain, under the cross identity its
    /// cross code derives, still alive, and of its own market data kind (an
    /// order and an execution under one cross code are two chains) - so a
    /// chained message carries its
    /// predecessor's `prevuuid` and `prevunix`, its place at `seqnum` - its
    /// own unless the predecessor happened at the same instant or later,
    /// where it takes the higher of its own and one past the predecessor's -
    /// the chain's `creaunix` and every `metadata` key of the chain it does not
    /// state, and is settled again around them. A message no live one precedes is
    /// answered as it came. The loader turns the iterable into the pull
    /// function this takes; a failure of the iterable throws and ends the
    /// stream. The walk reads the structured message first: one whose
    /// sending clock the parse supplied rather than read is dated by the
    /// `TransactTime(60)` it states, so a capture whose frames state no
    /// `SendingTime(52)` still orders, expires and folds by when its
    /// transactions happened.
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
    /// The rows one Arrow batch targets; the core's 32,768 when unstated.
    /// A batch closes on whichever bound it reaches first.
    pub batch_row_size: Option<f64>,
    /// The workers parsing and row conversion use; available CPUs when unstated.
    /// Arrow capture parsing holds at most one input batch per worker and
    /// yields in order; line doors use bounded row chunks. One reads lazily
    /// without a pool, and zero reads as one.
    pub threads: Option<f64>,
    /// The epoch-aligned lifecycle snapshot width in exact nanoseconds.
    /// `null`, zero and a negative width disable snapshots.
    #[napi(ts_type = "bigint | null")]
    pub snapshot_ns: Option<Either<BigInt, Null>>,
    /// Whether the messages `lifecycle` is handed arrive in instant order,
    /// so the walk holds one epoch hour at a time rather than collecting and
    /// sorting the whole capture; `false` when unstated.
    pub sorted_lifecycle: Option<bool>,
    /// How far from `SendingTime(52)` an official transaction clock may
    /// stand and still date the message, in milliseconds; the core's one
    /// second when unstated, and a nonpositive delay admits only a
    /// transaction clock equal to the sending clock.
    pub official_time_delay_ms: Option<f64>,
    /// How long, in milliseconds of event time, `lifecycle` remembers an
    /// identity it yielded so it yields that identity once; the core's one
    /// minute when unstated, and `null`, zero or a negative window
    /// remembering none.
    #[napi(ts_type = "number | null")]
    pub dedup_window_ms: Option<Either<f64, Null>>,
    /// The message types a parse keeps, spelled as codes or as names -
    /// `"0"`, `"Heartbeat"`, `"unknown"` for a line stating no type. Empty
    /// or unstated keeps every type the refusals leave.
    pub include_msgtypes: Option<Vec<String>>,
    /// The message types a parse refuses before it builds a frame; the
    /// core's `Heartbeat`, `TestRequest` and untyped line when unstated, and
    /// an empty list keeps every type.
    pub exclude_msgtypes: Option<Vec<String>>,
    /// The `SendingTime` an undated message takes when neither it, its row
    /// nor its line dates it: a `DateTime64(ns, UTC)` `Scalar`, or a `Date`
    /// restated in nanoseconds. UTC now per new message when unstated or
    /// `null`.
    #[napi(ts_type = "Scalar | Date | null")]
    pub default_sending_time: Option<Either3<ClassInstance<'env, JsScalar>, JsDate<'env>, Null>>,
    /// Whether a market operation this codec builds carries, in its
    /// metadata, what its message states that no typed column reads and no
    /// identifier map of the leaf holds, lifting the identifiers among them
    /// into the set their type belongs to - part of the leaf's identity; the
    /// core's `true` when unstated.
    pub market_metadata: Option<bool>,
    /// The `IsinRegistry` every `lifecycle` learns into and fills from,
    /// shared so a walk run after another starts from what the first
    /// learned; each walk learns into its own, starting empty, when unstated.
    #[napi(ts_type = "IsinRegistry")]
    pub isin_registry: Option<ClassInstance<'env, JsIsinRegistry>>,
    /// The id of the dictionary's source this codec reads under, folded:
    /// the catalog entry (`FixRegistry.sources()`) whose plugin side every
    /// message the codec builds states as its `msgpluginside`, resolved
    /// once here and refused when the catalog holds no such id; `UKNW` on
    /// every message when unstated.
    pub source: Option<String>,
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

/// The fixed root every message answers as, built from one dictionary.
///
/// The crate's own columns lead - its clocks, then its identities, then the
/// rest it knows - then the header, the fields a consumer reads, the groups
/// worth persisting whole, the trailer, `MsgDirection` (385), and the one
/// serie that closes every row: `fixentries`, the whole content record,
/// unresolved keys at tag 0. Columns are spelled by the dictionary's folded
/// canonical names - `msgtype`, never `35` - so a row reads the way a
/// message reads; the tag stays each column's identity, on its `FIX:tag`,
/// and is what fills it. `beginstring` and the settled identity - `currunix`,
/// `creaunix`, `currhashcode`, `crosshashcode`, `curruuid`, `crossuuid` - are
/// required; every other column is nullable, because a message that carried
/// nothing there must answer null rather than shift its neighbours.
#[napi(js_name = "fixSchema")]
pub fn fix_schema(
    registry: Option<ClassInstance<'_, JsFixRegistry>>,
    name: Option<String>,
) -> Result<JsField> {
    let registry = registry_or_env(registry)?;
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
/// `bridgesessionid`, `msgctxid`, `msgseqnum`, `msgpluginid` - for that reason,
/// so each value reaches its column rather than leading the row, and never
/// over a reading the message stated itself.
///
/// A carried column is nullable whatever the capture declared it: a capture's
/// own column is the *reading's* statement and no message holds one, so a
/// pass that has no source row in hand writes null there rather than
/// refusing per row. The one-pass readers state every one of them.
#[napi(js_name = "fixSchemaCarrying")]
pub fn fix_schema_carrying(carrier: &JsField, read: &JsField) -> Result<JsField> {
    yggdryl::fix_schema_carrying(&carrier.inner, &read.inner)
        .map(JsField::from_core)
        .map_err(napi_error)
}

/// The row header a `ULBridge` log writes in front of every line, as the
/// crate spells it: `fix.ULBRIDGE_ROWHEADER` is where a caller reads it,
/// and this is the half that carries the text across.
#[napi(js_name = "_fixUlbridgeRowheaderNative", skip_typescript)]
pub fn fix_ulbridge_rowheader_native() -> &'static str {
    yggdryl::ULBRIDGE_ROWHEADER
}

/// One row's columns, in order, as tags: the crate's leading columns, then
/// the message, the instrument, the order, the values, how it went, the
/// groups - each by the counter tag naming it - and the frame.
#[allow(clippy::cast_lossless)]
#[napi(js_name = "fixSchemaTags")]
pub fn fix_schema_tags() -> Vec<f64> {
    yggdryl::fix_schema_tags()
        .into_iter()
        .map(f64::from)
        .collect()
}

/// The definitions this crate owns, in tag order, above every tag FIX or a
/// venue publishes.
///
/// The event's instant `currunix` and the chain's `creaunix`, `execunix`,
/// `recdunix`, `prevunix`, `snapunix` and `exprunix`; the identities
/// `currhashcode`, `crosshashcode`, `curruuid`, `crossuuid` and `prevuuid`;
/// the `srcuuids` list of the lines it was read from; the `crosscode`, the
/// `seqnum` and the `state` reached; the `metadata` Map group; what a
/// bridge's capture states - `msgctxid`, `msgpluginid`,
/// `msgsessionid` - and the `msgsesseventid` the session and the context
/// join to with the message type and sequence; the capture's own column,
/// `sourceurl`, which whoever read the line states on the row and no message
/// holds; the `marketdatakind` the message type files under; the normalized
/// instrument codes (`isincode`, `bloombergcode`, `figicode`, `forexcode`,
/// `miccode`) and the market and operation facts a message names - each a
/// fact no FIX dictionary publishes, at the datatype its graph column names,
/// numbered contiguously from `65001` through `fixmsg` (`65053`). The strike
/// price is the derived market fact `strikepx` over `StrikePrice(202)`, and a
/// bridge's own identifier keys are no crate field either: they arrive as
/// unmapped entries and are read for the identifier name they end with.
///
/// `currunix`, `creaunix`, `currhashcode`, `crosshashcode`, `curruuid` and
/// `crossuuid` are non-null; `state` is written on every row a message
/// writes and stays nullable, a state having no neutral member. Every
/// registry already holds them, so this is the listing a schema or a
/// document walks rather than something a caller registers.
#[napi(js_name = "fixCrateFields")]
pub fn fix_crate_fields() -> Result<Vec<JsField>> {
    yggdryl::fix_crate_fields()
        .map(|held| held.iter().cloned().map(JsField::from_core).collect())
        .map_err(napi_error)
}
