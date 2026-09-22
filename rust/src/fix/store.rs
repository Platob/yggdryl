//! Native Field documents in explicit FIX category folders, and the code sets
//! they read by beside them.
//!
//! Tagged fields are arrays sharded by `tag / 100`. Named definitions are
//! individual documents; their native Null-typed reference occurrences resolve
//! once at this boundary into shared, typed Field subtrees.
//!
//! `codesets/` is the fourth folder, and it holds vocabularies rather than
//! fields: one `<name>.json` per [code set](super::codes), stating its name
//! and its members, and a field's `FIX:codeset` names the set it draws on. It
//! is not a [`FixCategory`] for that reason - a category holds `Field`
//! documents and resolves references between them - so it is read first, and
//! written and pruned beside them.

use std::collections::{BTreeMap, HashMap};

use smol_str::{SmolStr, format_smolstr};

use super::FixRegistry;
use crate::holder::Holder;
use crate::sequence::SequenceType;
use crate::text::Formatting;
use crate::{
    DataType, DigestAlgorithm, Error, Field, FixCategory, IOBase, Result, Scalar, StructType, Url,
};

const SHARD_WIDTH: i32 = 100;
/// The folder the code sets live in, beside the three category folders.
pub(super) const CODESETS: &str = "codesets";
/// What one stored code set states: the name it is filed under, and its
/// members in the set's own order.
const CODESET_NAME: &str = "name";
const CODESET_CODES: &str = "codes";
const LOAD_ORDER: [FixCategory; 3] = [
    FixCategory::Fields,
    FixCategory::Components,
    FixCategory::Groups,
];

pub(super) const fn shard_of(tag: i32) -> i32 {
    tag / SHARD_WIDTH
}

fn shard_index(entry: &Holder) -> Option<i32> {
    if entry.is_container() {
        return None;
    }
    let url = entry.url()?;
    if url.extension() != Some("json") {
        return None;
    }
    let stem = url.stem()?;
    if stem.is_empty() || !stem.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    stem.parse().ok()
}

/// The same refusal, naming the file it was read from.
pub(super) fn located(error: Error, entry: &dyn IOBase) -> Error {
    let at = entry
        .url()
        .map_or_else(|| "FIX definition".into(), |url| format_smolstr!("{url}"));
    match error {
        Error::Parse {
            target,
            position,
            reason,
        } => Error::Parse {
            target,
            position,
            reason: format_smolstr!("{reason} in {at}"),
        },
        Error::Codec {
            format,
            position,
            reason,
        } => Error::Codec {
            format,
            position,
            reason: format_smolstr!("{reason} in {at}"),
        },
        Error::Absent { expected, path } => Error::Absent {
            expected,
            path: format_smolstr!("{path} in {at}"),
        },
        Error::Conflict {
            expected,
            actual,
            path,
        } => Error::Conflict {
            expected,
            actual,
            path: format_smolstr!("{path} in {at}"),
        },
        Error::InvalidRecord { path, reason } => Error::InvalidRecord {
            path: format_smolstr!("{path} in {at}"),
            reason,
        },
        Error::InvalidMetadataValue { key, reason } => Error::InvalidMetadataValue {
            key,
            reason: format_smolstr!("{reason} in {at}"),
        },
        other => Error::InvalidRecord {
            path: at,
            reason: format_smolstr!("{other}"),
        },
    }
}

/// The name and the members one stored code set document states.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] when the document is not an object
/// stating a text `name` and an array `codes`, which is the one shape a store
/// writes.
fn codeset_document(document: &Scalar) -> Result<(String, String)> {
    let record = document.as_struct().ok_or_else(|| Error::InvalidRecord {
        path: CODESETS.into(),
        reason: crate::text::expected_got("a JSON code set object", document.kind()),
    })?;
    let name = record
        .get(CODESET_NAME)
        .and_then(Scalar::as_str)
        .ok_or_else(|| Error::InvalidRecord {
            path: CODESETS.into(),
            reason: "expected a code set stating its name".into(),
        })?
        .to_owned();
    let codes = record
        .get(CODESET_CODES)
        .filter(|codes| codes.as_sequence().is_some())
        .ok_or_else(|| Error::InvalidRecord {
            path: name.as_str().into(),
            reason: "expected a code set stating an array of codes".into(),
        })?;
    // The array the file spells is the canonical text a set is held as, with
    // every entry's keys put back into the order the grammar declares.
    let document = super::codes::codes_text(codes)?;
    Ok((name, document))
}

/// What one named definition is keyed by: its category and its canonical
/// spelling.
pub(super) type DefinitionKey = (FixCategory, String);

pub(super) fn definition_key(category: FixCategory, field: &Field) -> DefinitionKey {
    (category, field.name().to_owned())
}

/// The definition a field restates, when it carries a reference marker.
pub(super) fn reference(field: &Field) -> Option<(FixCategory, &str)> {
    let view = field.as_fix();
    view.field_ref()
        .map(|name| (FixCategory::Fields, name))
        .or_else(|| view.group().map(|name| (FixCategory::Groups, name)))
        .or_else(|| view.component().map(|name| (FixCategory::Components, name)))
}

struct Resolver<'a> {
    fields: &'a FixRegistry,
    raw: BTreeMap<DefinitionKey, Field>,
    // Cached height proves a reused subtree fits its new occurrence depth.
    resolved: HashMap<DefinitionKey, (Field, usize)>,
    active: Vec<DefinitionKey>,
}

impl Resolver<'_> {
    fn definition(&mut self, key: &DefinitionKey, depth: usize) -> Result<(Field, usize)> {
        if let Some((field, height)) = self.resolved.get(key) {
            Self::depth(field.name(), depth + height)?;
            return Ok((field.clone(), *height));
        }
        if self.active.len() >= 64 || self.active.contains(key) {
            return Err(Error::InvalidRecord {
                path: key.1.as_str().into(),
                reason: "expected an acyclic FIX reference graph nested at most 64 levels".into(),
            });
        }
        let field = self
            .raw
            .get(key)
            .ok_or_else(|| Error::absent(key.0.as_str(), key.1.as_str()))?
            .clone();
        self.active.push(key.clone());
        let resolved = self.children(field, depth)?;
        self.active.pop();
        self.resolved.insert(key.clone(), resolved.clone());
        Ok(resolved)
    }

    fn depth(name: &str, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err(Error::InvalidRecord {
                path: name.into(),
                reason: "FIX fields are nested at most 64 levels".into(),
            });
        }
        Ok(())
    }

    fn occurrence(&mut self, field: &Field, depth: usize) -> Result<(Field, usize)> {
        Self::depth(field.name(), depth)?;
        let Some((category, name)) = reference(field) else {
            return self.children(field.clone(), depth);
        };
        let view = field.as_fix();
        if [view.field_ref(), view.group(), view.component()]
            .into_iter()
            .flatten()
            .count()
            != 1
        {
            return Err(Error::InvalidRecord {
                path: field.name().into(),
                reason: "expected exactly one FIX field, component, or group reference".into(),
            });
        }
        if field.dtype() != &DataType::Null {
            return Err(Error::InvalidRecord {
                path: field.name().into(),
                reason: "stored FIX references require the Null placeholder datatype".into(),
            });
        }
        let (mut resolved, height) = if category == FixCategory::Fields {
            // By identity where the reference carries the tag - strict, and
            // one probe - by name where it does not.
            let by_id = match view.tag()? {
                Some(tag) => super::FixId::of(tag, name)
                    .ok()
                    .and_then(|id| self.fields.get_field_by_id(id)),
                None => None,
            };
            let target = match by_id {
                Some(target) => target,
                None => self.fields.definition(category, name)?,
            };
            (target.clone(), 0)
        } else {
            let exact = (category, name.to_owned());
            let key = self
                .raw
                .get_key_value(&exact)
                .map(|(key, _)| key)
                .or_else(|| {
                    self.raw
                        .keys()
                        .find(|key| key.0 == category && crate::folds_equal(&key.1, name))
                })
                .cloned();
            match key {
                Some(key) => self.definition(&key, depth)?,
                None => {
                    let builtin = self
                        .fields
                        .get_definition(category, name)
                        .filter(|held| is_crate_field(held))
                        .ok_or_else(|| Error::absent(category.as_str(), name))?
                        .clone();
                    self.children(builtin, depth)?
                }
            }
        };
        resolved.set_name(field.name());
        resolved.set_nullable(field.is_nullable());
        let metadata = field.as_metadata().merge_with(resolved.as_metadata())?;
        // A named definition's tag is its identity in the catalog, not part of
        // what a reference to it restates - the same rule the reference check
        // in `catalog.rs` reads occurrences under. Inheriting it here would
        // make one catalog compare unequal to itself across a round trip,
        // differing only by a tag no occurrence is supposed to carry.
        let inherited = metadata
            .iter()
            .filter(|(key, _)| category == FixCategory::Fields || *key != super::field::TAG_KEY);
        resolved.set_metadata(inherited)?;
        Ok((resolved, height))
    }

    fn children(&mut self, mut field: Field, depth: usize) -> Result<(Field, usize)> {
        Self::depth(field.name(), depth)?;
        let mut height = 0;
        let dtype = match field.dtype() {
            DataType::Struct(children) => {
                let mut resolved = Vec::with_capacity(children.len());
                for child in children.iter() {
                    let (child, child_height) = self.occurrence(child, depth + 1)?;
                    height = height.max(child_height + 1);
                    resolved.push(child);
                }
                Some(DataType::from(StructType::from_fields(resolved)?))
            }
            DataType::Sequence(SequenceType::List(item))
            | DataType::Sequence(SequenceType::LargeList(item)) => {
                let (item, child_height) = self.occurrence(item, depth + 1)?;
                height = child_height + 1;
                Some(
                    if matches!(field.dtype(), DataType::Sequence(SequenceType::List(_))) {
                        DataType::list(item)
                    } else {
                        DataType::large_list(item)
                    },
                )
            }
            DataType::Mapping(map) => {
                let mut entries = map.entries().clone();
                if reference(&entries).is_some() {
                    // A persisted Map must retain its entries Struct. Only
                    // this storage envelope becomes the ordinary placeholder;
                    // supplied metadata still meets the reference checks.
                    entries.set_dtype(DataType::Null)?;
                }
                let (entries, child_height) = self.occurrence(&entries, depth + 1)?;
                height = child_height + 1;
                Some(DataType::map(entries, map.keys_sorted())?)
            }
            _ => None,
        };
        if let Some(dtype) = dtype {
            field.set_dtype(dtype)?;
        }
        field.as_fix_mut().normalize_identifiers()?;
        Ok((field, height))
    }
}

/// The document a store writes for a resolved definition: every referenced
/// child folded back to the Null placeholder that names its target, so the
/// target's tree is stated once, where the target is.
///
/// The root is kept whole even when it carries a marker, because a group
/// names its component on its own root and is still the definition. Applied
/// to a document already compact it changes nothing, which is what lets one
/// fold hand a document on to the resolver without asking which it was given.
pub(super) fn compact(mut field: Field, root: bool) -> Result<Field> {
    if !root {
        if let Some((category, name)) = reference(&field) {
            let name = name.to_owned();
            let mut placeholder = DataType::Null.nullable_field(field.name());
            placeholder.set_nullable(field.is_nullable());
            match category {
                FixCategory::Fields => {
                    placeholder.as_fix_mut().set_field_ref(&name)?;
                    // The tag beside the name: a reader resolves the
                    // reference by the field's identity, the pair, and
                    // never by a spelling alone.
                    if let Some(tag) = field.as_fix().tag()? {
                        placeholder.as_fix_mut().set_tag(tag)?;
                    }
                }
                FixCategory::Groups => placeholder.as_fix_mut().set_group(&name)?,
                FixCategory::Components => placeholder.as_fix_mut().set_component(&name)?,
            }
            return Ok(placeholder);
        }
    }
    let dtype = match field.dtype() {
        DataType::Struct(children) => Some(DataType::from(StructType::from_fields(
            children
                .iter()
                .cloned()
                .map(|child| compact(child, false))
                .collect::<Result<Vec<_>>>()?,
        )?)),
        DataType::Sequence(SequenceType::List(item)) => {
            Some(DataType::list(compact(item.as_ref().clone(), false)?))
        }
        DataType::Sequence(SequenceType::LargeList(item)) => {
            Some(DataType::large_list(compact(item.as_ref().clone(), false)?))
        }
        DataType::Mapping(map) => {
            // Keep the entries Struct for Map validation, but its component
            // reference inherits metadata from the same owner as any other.
            let mut entries = compact(map.entries().clone(), true)?;
            if reference(&entries).is_some() {
                let placeholder = compact(map.entries().clone(), false)?;
                entries.set_metadata(placeholder.as_metadata().iter())?;
            }
            Some(DataType::map(entries, map.keys_sorted())?)
        }
        _ => None,
    };
    if let Some(dtype) = dtype {
        field.set_dtype(dtype)?;
    }
    Ok(field)
}

/// Whether a field is one the crate defines rather than a store.
///
/// The reading half of every store asks this, to read past the copy a writer
/// left in: a dump states the whole row, and the crate's own definition is
/// still the one that types it.
fn is_crate_field(field: &Field) -> bool {
    field
        .as_fix()
        .tag()
        .ok()
        .flatten()
        .is_some_and(super::is_crate_tag)
}

/// What one [`FixRegistry::commit`] changed under a store root.
///
/// A commit states the paths it moved and counts the ones it left, because a
/// caller reads this to see filesystem changes and a store holds thousands of
/// documents that a run normally leaves alone: naming every one of those would
/// bury the handful that moved. The operation bounds the report, so it is owned
/// rather than a lazy walk.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct FixCommit {
    /// The documents written, in the order a store lays them out.
    pub written: Vec<SmolStr>,
    /// The documents already holding what the registry states.
    pub skipped: usize,
    /// The documents removed because no definition holds them any more.
    pub removed: Vec<SmolStr>,
}

impl FixCommit {
    /// Whether the root already held everything this registry states.
    ///
    /// A second commit of an unchanged registry answers `true`, which is what
    /// makes a dump replayable: the bytes settle once and nothing after that
    /// touches the filesystem.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.written.is_empty() && self.removed.is_empty()
    }

    /// How many documents the commit considered, moved and left alike.
    #[must_use]
    pub fn len(&self) -> usize {
        self.written.len() + self.skipped
    }

    /// Whether the registry stated no document at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl FixRegistry {
    fn is_crate_definition(&self, category: FixCategory, field: &Field) -> bool {
        is_crate_field(field)
            || self
                .get_definition(category, field.name())
                .is_some_and(is_crate_field)
    }

    /// Reads a complete registry snapshot from JSON.
    ///
    /// `fields`, `components`, and `groups` are arrays of native Field
    /// documents; a message is a component carrying `FIX:msgtype`, and any
    /// other key - `messages` among them - is refused by name. `codesets` is
    /// the vocabularies the fields read by, each stating its name and its
    /// members, read before the fields that name them. References resolve
    /// through the same bounded graph loader as the store.
    pub fn from_json(input: &str) -> Result<Self> {
        Self::from_snapshot(&crate::from_json_scalar(input)?)
    }

    /// Renders all categories as canonical JSON.
    ///
    /// Referenced occurrences keep their native Null placeholders; loading
    /// reconstructs the same resolved catalog. No filesystem I/O is performed.
    ///
    /// ```
    /// use yggdryl::{DataType, FixRegistry, StructType};
    /// let mut registry = FixRegistry::new();
    /// let mut message = DataType::from(StructType::from_fields([])?).required_field("Order");
    /// message.as_fix_mut().set_msgtype("D")?;
    /// registry.insert(message)?;
    /// let restored = FixRegistry::from_json(&registry.into_json()?)?;
    /// assert_eq!(restored, registry);
    /// assert_eq!(restored.msgtype("D")?.name(), "Order");
    /// # Ok::<(), yggdryl::Error>(())
    /// ```
    pub fn into_json(&self) -> Result<String> {
        crate::into_json_scalar(&self.snapshot()?)
    }

    /// Reads one JSON registry snapshot into this dictionary, whole.
    ///
    /// The lenient door beside [`Self::from_json`], which builds a dictionary
    /// of its own: the file is read through exactly that parse - the same
    /// three categories, the same bounded reference graph, the same refusals,
    /// now naming the file they came from - and then folded in the way
    /// [`Self::merge_with`] folds any dictionary, so what only the file
    /// declares arrives and what both declare merges with the file winning a
    /// shared key.
    ///
    /// No dialect is taken, and that is the point of the pair: a `CBlock`
    /// states no membership, so [`Self::add_cfb_file`] has to be told one or
    /// guess it from the file's stem, while a snapshot is this crate's own
    /// format and every field and definition in it already carries the
    /// `FIX:branches` its writer meant. Naming one here would overwrite that.
    ///
    /// A snapshot restating one of this crate's own fields is read past
    /// rather than refused: every registry holds those from construction, so
    /// the held definition stays and the counts do not move.
    ///
    /// Answers the count added and the count merged, over the fields; the
    /// two seeded clocks every parsed snapshot carries always merge, so a
    /// file adding nothing else answers `(0, 2)`. Named definitions that
    /// arrive or merge are not counted, exactly as [`Self::merge_with`] does
    /// not count them.
    ///
    /// One mutation: a document that does not parse, a reference naming a
    /// definition nothing holds, a cycle, or a datatype disagreeing with a
    /// stored field leaves this dictionary exactly as it was.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::from_json`] returns, located at the handle's URL,
    /// and what [`Self::merge_with`] returns for the fold.
    pub fn add_json_file(&mut self, handle: &dyn IOBase) -> Result<(usize, usize)> {
        let parsed = crate::from_json_scalar(handle.read_all_bytes()?)
            .and_then(|document| Self::from_snapshot(&document))
            .map_err(|error| located(error, handle))?;
        self.merge_with(&parsed)
    }

    fn snapshot(&self) -> Result<Scalar> {
        self.validate_catalog()?;
        let mut document = Vec::with_capacity(FixCategory::ALL.len() + 1);
        // The vocabularies lead, as they do in a folder store and for the
        // same reason: a field names the set it reads by, so a reader has
        // the sets before it meets a field naming one.
        document.push((
            CODESETS,
            Scalar::from_sequence(
                self.codesets()
                    .map(|set| {
                        Scalar::from_struct([
                            (CODESET_NAME, Scalar::from(set.name())),
                            (CODESET_CODES, crate::from_json_scalar(set.document())?),
                        ])
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
        ));
        for category in FixCategory::ALL {
            // Every definition the dictionary holds, the crate's own among
            // them: a snapshot is the whole row as this registry types it,
            // not the half a store happened to declare. Reading one back
            // takes the crate's copy over the document's, so the two never
            // collide; the folder store writes the same way in `write_into`.
            let fields = if category == FixCategory::Fields {
                self.scalars()
                    .cloned()
                    .map(|field| super::document::dump(field.into_value()))
                    .collect::<Result<Vec<_>>>()?
            } else {
                self.catalog
                    .iter(category)
                    .cloned()
                    .map(|field| {
                        compact(field, true)
                            .and_then(|field| super::document::dump(field.into_value()))
                    })
                    .collect::<Result<Vec<_>>>()?
            };
            document.push((category.as_str(), Scalar::from_sequence(fields)));
        }
        Scalar::from_struct(document)
    }

    fn from_snapshot(document: &Scalar) -> Result<Self> {
        let record = document.as_struct().ok_or_else(|| Error::InvalidRecord {
            path: "fix registry".into(),
            reason: crate::text::expected_got("a JSON registry object", document.kind()),
        })?;
        for key in record.keys() {
            if key != CODESETS
                && !FixCategory::ALL
                    .iter()
                    .any(|category| category.as_str() == key)
            {
                return Err(Error::InvalidRecord {
                    path: key.clone(),
                    reason: "expected codesets, fields, components, or groups".into(),
                });
            }
        }
        let mut registry = Self::base();
        let mut raw = BTreeMap::new();
        // The vocabularies first, for the reason a folder store reads them
        // first: a field naming a set the dictionary does not hold is
        // refused. A snapshot stating none is a dictionary whose fields draw
        // on none.
        if let Some(codesets) = record.get(CODESETS) {
            let sets = codesets.as_sequence().ok_or_else(|| Error::InvalidRecord {
                path: CODESETS.into(),
                reason: "expected an array of code set documents".into(),
            })?;
            for set in sets {
                let (name, codes) = codeset_document(set)?;
                registry.create_codeset(&name, codes)?;
            }
        }
        for category in LOAD_ORDER {
            let fields = record
                .get(category.as_str())
                .and_then(Scalar::as_sequence)
                .ok_or_else(|| Error::InvalidRecord {
                    path: category.as_str().into(),
                    reason: "expected an array of native Field documents".into(),
                })?;
            for (index, value) in fields.iter().enumerate() {
                let field = Field::from_value(super::document::load(value.clone())?)?;
                if registry.is_crate_definition(category, &field) {
                    continue;
                }
                if category == FixCategory::Fields {
                    // A snapshot written before the crate held these states
                    // them; it is read past rather than allowed to replace the
                    // definition every registry already carries.
                    registry.create_definition(category, field)?;
                } else {
                    // Likewise a snapshot that states the crate's own
                    // message: read past, never over the held copy.
                    let key = definition_key(category, &field);
                    if raw.insert(key, field).is_some() {
                        return Err(Error::conflict(
                            "one FIX definition",
                            "duplicate definition",
                            format_args!("{category}[{index}]"),
                        ));
                    }
                }
            }
        }
        registry.seed_clocks()?;
        registry.load_definitions(raw, None)?;
        registry.validate_catalog()?;
        registry.refresh_msgtype_aliases();
        Ok(registry)
    }

    /// Every named definition as the document a store writes, keyed as the
    /// resolver keys it.
    ///
    /// The catalog holds resolved trees; this is the same catalog with every
    /// reference folded back to its marker, which is the shape a fold edits
    /// and [`Self::resolve_catalog`] reads. What a merge appends to one
    /// definition is therefore seen by every definition referencing it the
    /// moment the map is resolved again, because none of them holds a copy.
    pub(super) fn compact_catalog(&self) -> Result<BTreeMap<DefinitionKey, Field>> {
        self.catalog
            .all()
            .map(|entry| {
                let field = compact(entry.field.as_field().clone(), true)?;
                Ok((definition_key(entry.category, &field), field))
            })
            .collect()
    }

    /// Re-resolves every named definition against the fields now held.
    pub(super) fn refresh_references(&mut self) -> Result<()> {
        let raw = self.compact_catalog()?;
        self.resolve_catalog(raw)
    }

    /// Replaces the catalog with `raw` resolved against this registry's
    /// fields.
    ///
    /// Nothing is adopted until every document resolved and re-entered the
    /// catalog, so a document naming a definition the map lacks, or a graph
    /// that cycles, leaves the catalog as it was.
    pub(super) fn resolve_catalog(&mut self, raw: BTreeMap<DefinitionKey, Field>) -> Result<()> {
        let mut resolver = Resolver {
            fields: self,
            raw,
            resolved: HashMap::new(),
            active: Vec::new(),
        };
        let keys: Vec<_> = resolver.raw.keys().cloned().collect();
        for key in &keys {
            resolver.definition(key, 0)?;
        }
        let resolved = resolver.resolved;
        // Built whole and settled once: nothing reads this catalog until it
        // is adopted below, so the order and the two derived indexes are
        // owed once at the end rather than once per definition.
        let mut catalog = super::catalog::Catalog::default();
        for key in keys {
            let (field, _) = resolved
                .get(&key)
                .ok_or_else(|| Error::absent(key.0.as_str(), key.1.as_str()))?;
            catalog.push(key.0, field.clone())?;
        }
        catalog.settle_indexes();
        self.catalog = catalog;
        self.forget_derivations();
        self.refresh_msgtype_aliases();
        Ok(())
    }

    /// Loads fields, components, and groups.
    ///
    /// Scalar shards are arrays; named definitions are single native Field
    /// documents. References resolve once, with missing names and cycles
    /// rejected before a registry is returned. A folder inside a category
    /// is not a store's layout and is passed over.
    pub fn from_handle(handle: &dyn IOBase) -> Result<Self> {
        let mut registry = Self::base();
        let mut raw = BTreeMap::new();
        // The vocabularies first: a field names the set it reads by, and a
        // field naming one the dictionary does not hold is refused.
        registry.load_codesets(handle)?;
        for category in LOAD_ORDER {
            let root = handle.child_by_path(category.as_str())?;
            for entry in root.ls(false, false) {
                let entry = entry?;
                registry
                    .load_entry(&entry, category, &mut raw)
                    .map_err(|error| located(error, entry.as_io()))?;
            }
        }
        registry.seed_clocks()?;
        registry.load_definitions(raw, Some(handle))?;
        registry.validate_catalog()?;
        registry.refresh_msgtype_aliases();
        Ok(registry)
    }

    /// Reads every `codesets/<name>.json` the store holds.
    ///
    /// A folder inside `codesets/` is not a store's layout and is passed
    /// over, exactly as one inside a category is. A file whose stem does not
    /// equal the name it states is refused, for the reason a definition's is:
    /// the stem is how the set is addressed.
    fn load_codesets(&mut self, handle: &dyn IOBase) -> Result<()> {
        let root = handle.child_by_path(CODESETS)?;
        for entry in root.ls(false, false) {
            let entry = entry?;
            if entry.is_container() || entry.url().and_then(Url::extension) != Some("json") {
                continue;
            }
            let document = crate::from_json_scalar(entry.read_all_bytes()?)
                .and_then(|document| codeset_document(&document))
                .and_then(|(name, codes)| {
                    if entry.url().and_then(Url::stem) == Some(name.as_str()) {
                        Ok((name, codes))
                    } else {
                        Err(Error::InvalidRecord {
                            path: name.as_str().into(),
                            reason: "expected the code set name to equal its filename stem".into(),
                        })
                    }
                })
                .map_err(|error| located(error, entry.as_io()))?;
            self.create_codeset(&document.0, document.1)
                .map_err(|error| located(error, entry.as_io()))?;
        }
        Ok(())
    }

    fn load_definitions(
        &mut self,
        raw: BTreeMap<DefinitionKey, Field>,
        root: Option<&dyn IOBase>,
    ) -> Result<()> {
        let mut resolver = Resolver {
            fields: self,
            raw,
            resolved: HashMap::new(),
            active: Vec::new(),
        };
        let keys: Vec<_> = resolver.raw.keys().cloned().collect();
        for key in &keys {
            if let Err(error) = resolver.definition(key, 0) {
                let Some(root) = root else {
                    return Err(error);
                };
                let path = format!("{}/{}.json", key.0, key.1);
                return Err(located(error, root.child_by_path(&path)?.as_io()));
            }
        }
        let resolved = resolver.resolved;
        for category in LOAD_ORDER {
            for key in keys.iter().filter(|key| key.0 == category) {
                let (field, _) = resolved
                    .get(key)
                    .ok_or_else(|| Error::absent(category.as_str(), key.1.as_str()))?;
                // A document written before named definitions carried a tag
                // states none, so one is derived here exactly as
                // `insert_definition` derives it. A document that states one
                // keeps it: the dictionary owns the identity, not the reader.
                let mut field = field.clone();
                if field.as_fix().tag()?.is_none() {
                    let tag = self.derived_definition_tag(field.name())?;
                    field.as_fix_mut().set_tag(tag)?;
                }
                self.forget_derivations();
                self.catalog.insert(category, field)?;
            }
        }
        Ok(())
    }

    fn load_entry(
        &mut self,
        entry: &Holder,
        category: FixCategory,
        raw: &mut BTreeMap<DefinitionKey, Field>,
    ) -> Result<()> {
        if entry.is_container() || entry.url().and_then(Url::extension) != Some("json") {
            return Ok(());
        }
        if category == FixCategory::Fields {
            let Some(shard) = shard_index(entry) else {
                return Ok(());
            };
            let document = crate::from_json_scalar(entry.read_all_bytes()?)?;
            let fields = document.as_sequence().ok_or_else(|| Error::InvalidRecord {
                path: "fields".into(),
                reason: "expected a JSON array of field documents".into(),
            })?;
            for value in fields {
                let field = Field::from_value(super::document::load(value.clone())?)?;
                let (tag, _) = super::registry::canonical_identity(&field)?;
                // The crate's own tags are never a store's to define: every
                // registry holds the crate's definition from construction, and
                // a copy an older store wrote is read past rather than allowed
                // to replace it.
                if super::is_crate_tag(tag) {
                    continue;
                }
                if shard_of(tag) != shard {
                    return Err(Error::InvalidRecord {
                        path: field.name().into(),
                        reason: crate::text::expected_got(
                            format_args!(
                                "a tag of shard {shard}, from {} to {}",
                                shard * SHARD_WIDTH,
                                shard * SHARD_WIDTH + SHARD_WIDTH - 1
                            ),
                            format_args!("tag {tag}"),
                        ),
                    });
                }
                self.create_definition(FixCategory::Fields, field)?;
            }
        } else {
            let document = crate::from_json_scalar(entry.read_all_bytes()?)?;
            let field = Field::from_value(super::document::load(document)?)?;
            if self.is_crate_definition(category, &field) {
                return Ok(());
            }
            if entry.url().and_then(Url::stem) != Some(field.name()) {
                return Err(Error::InvalidRecord {
                    path: field.name().into(),
                    reason: "expected the definition name to equal its filename stem".into(),
                });
            }
            let key = definition_key(category, &field);
            if raw.insert(key, field).is_some() {
                return Err(Error::conflict(
                    "one FIX definition",
                    "duplicate definition",
                    category.as_str(),
                ));
            }
        }
        Ok(())
    }

    /// Writes the documents whose bytes moved and removes what is no longer held.
    ///
    /// Each document is written through the handle's byte-write contract.
    /// Referenced children persist as native Null-typed Fields and regain
    /// their resolved datatypes when loaded.
    ///
    /// The store is compared before it is written: every document is digested
    /// where it lies and left alone where it already states what this registry
    /// does. That is one read per document either way, and it is the cheaper
    /// half of the pair - a write is durable work on every backend and an
    /// object store charges for each one - so a commit that changes one field
    /// moves one document rather than all of them, and a second commit of an
    /// unchanged registry moves none. A missing document digests as empty and
    /// so never matches, which is how a document is created without asking
    /// whether it is there.
    ///
    /// # Errors
    ///
    /// Returns the backing store's read or write failure, or the catalog's own
    /// validation failure before a single byte is written.
    pub fn commit(&self, root: &mut dyn IOBase) -> Result<FixCommit> {
        self.validate_catalog()?;
        let mut report = FixCommit::default();
        let mut documents: BTreeMap<String, Scalar> = BTreeMap::new();
        let mut shards: BTreeMap<i32, Vec<Field>> = BTreeMap::new();
        // Every scalar the dictionary holds, the crate's own among them, so a
        // written store states the whole row rather than the half it declared
        // itself. The crate's block is one shard of its own, above every tag
        // a dictionary reaches, and a reader takes the held definition over
        // the document it finds there.
        for field in self.scalars() {
            let (tag, _) = super::registry::canonical_identity(field)?;
            shards.entry(shard_of(tag)).or_default().push(field.clone());
        }
        for (shard, fields) in shards {
            documents.insert(
                format!("fields/{shard:09}.json"),
                Scalar::from_sequence(
                    fields
                        .into_iter()
                        .map(|field| super::document::dump(field.into_value()))
                        .collect::<Result<Vec<_>>>()?,
                ),
            );
        }
        for entry in self.catalog.all() {
            let path = format!("{}/{}.json", entry.category, entry.field.name());
            documents.insert(
                path,
                super::document::dump(compact(entry.field.as_field().clone(), true)?.into_value())?,
            );
        }
        // And the fixed row itself, for the reason the crate's own fields are
        // written above: a consumer reads the row's shape off the store, and
        // a reader passes the document over as it passes those fields.
        let fixmsg = super::schema::fixmsg_definition(self)?;
        documents.insert(
            format!("{}/{}.json", FixCategory::Components, fixmsg.name()),
            super::document::dump(compact(fixmsg, true)?.into_value())?,
        );
        // The vocabularies beside the fields that name them, one document per
        // set, each stating the name it is filed under so the file says what
        // it is without its own path.
        for set in self.codesets() {
            documents.insert(
                format!("{CODESETS}/{}.json", set.name()),
                Scalar::from_struct([
                    (CODESET_NAME, Scalar::from(set.name())),
                    (
                        CODESET_CODES,
                        crate::from_json_scalar(set.document()).map_err(|error| {
                            Error::InvalidRecord {
                                path: set.name().into(),
                                reason: format_smolstr!("{error}"),
                            }
                        })?,
                    ),
                ])?,
            );
        }
        for (path, document) in &documents {
            // A text file ends with a newline, as the one a person's editor
            // and the generator write does, so a rewrite changes no line it
            // did not mean to.
            let mut bytes =
                crate::json::into_bytes_with_formatting(document, Formatting::indented(2))?;
            bytes.push(b'\n');
            let mut child = root.child_by_path(path)?;
            if child.read_digest(DigestAlgorithm::default())?
                == crate::xxhash::digest(&bytes, DigestAlgorithm::default())
            {
                report.skipped += 1;
                continue;
            }
            child.write_all_bytes(&bytes)?;
            report.written.push(path.as_str().into());
        }
        for folder in FixCategory::ALL
            .into_iter()
            .map(FixCategory::as_str)
            .chain(std::iter::once(CODESETS))
        {
            let prefix = format!("{folder}/");
            let mut tree = root.child_by_path(folder)?;
            if !documents.keys().any(|path| path.starts_with(&prefix)) {
                tree.remove(true)?;
                report.removed.push(prefix.as_str().into());
                continue;
            }
            for entry in tree.ls(false, false) {
                let mut entry = entry?;
                let name = entry
                    .url()
                    .and_then(Url::file_name)
                    .unwrap_or_default()
                    .to_owned();
                if entry.is_container() {
                    // A folder inside a category is not a store's layout,
                    // and a store leaves alone what it did not write.
                    continue;
                }
                if entry.url().and_then(Url::extension) == Some("json")
                    && !documents.contains_key(&format!("{folder}/{name}"))
                {
                    entry.remove(false)?;
                    report.removed.push(format_smolstr!("{folder}/{name}"));
                }
            }
        }
        Ok(report)
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/fix/mod_.rs` pins and a caller cannot reach.
    //!
    //! A shard is where a scalar definition is filed on disk; a caller reads
    //! the folder, never the arithmetic that picks the document inside it.

    /// The shard document that holds the definition of `tag`.
    #[must_use]
    pub const fn shard_of(tag: i32) -> i32 {
        super::shard_of(tag)
    }
}
