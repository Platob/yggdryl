//! Native Field documents in explicit FIX category folders.
//!
//! Tagged fields are arrays sharded by `tag / 100`. Named definitions are
//! individual documents; their native Null-typed reference occurrences resolve
//! once at this boundary into shared, typed Field subtrees.

use std::collections::{BTreeMap, HashMap};

use smol_str::format_smolstr;

use super::FixRegistry;
use crate::holder::Holder;
use crate::text::Formatting;
use crate::{DataType, Error, Field, FixCategory, IOBase, Result, Scalar, Url};

const SHARD_WIDTH: i32 = 100;
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
            (self.fields.definition(category, name)?.clone(), 0)
        } else {
            let exact = (category, name.to_owned());
            let key = self
                .raw
                .get_key_value(&exact)
                .map(|(key, _)| key)
                .or_else(|| {
                    self.raw
                        .keys()
                        .find(|key| key.0 == category && crate::types::folds_equal(&key.1, name))
                })
                .cloned();
            match key {
                Some(key) => self.definition(&key, depth)?,
                None => {
                    let builtin = self
                        .fields
                        .get_definition(category, name)
                        .filter(|held| is_crate_field(held) || is_crate_message(held))
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
                Some(DataType::from_fields(resolved)?)
            }
            DataType::List(item) | DataType::LargeList(item) => {
                let (item, child_height) = self.occurrence(item, depth + 1)?;
                height = child_height + 1;
                Some(if matches!(field.dtype(), DataType::List(_)) {
                    DataType::list(item)
                } else {
                    DataType::large_list(item)
                })
            }
            DataType::Map(map) => {
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
                FixCategory::Fields => placeholder.as_fix_mut().set_field_ref(&name)?,
                FixCategory::Groups => placeholder.as_fix_mut().set_group(&name)?,
                FixCategory::Components => placeholder.as_fix_mut().set_component(&name)?,
            }
            return Ok(placeholder);
        }
    }
    let dtype = match field.dtype() {
        DataType::Struct(children) => Some(DataType::from_fields(
            children
                .iter()
                .cloned()
                .map(|child| compact(child, false))
                .collect::<Result<Vec<_>>>()?,
        )?),
        DataType::List(item) => Some(DataType::list(compact(item.as_ref().clone(), false)?)),
        DataType::LargeList(item) => {
            Some(DataType::large_list(compact(item.as_ref().clone(), false)?))
        }
        DataType::Map(map) => {
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

/// Whether one definition is the crate's own message.
///
/// `pluginconfig` is to the components what the crate's own fields are to
/// the fields: a store writes it so a dump is the whole dictionary, and a
/// reader holding it from construction reads that copy past (decision 19).
fn is_crate_message(field: &Field) -> bool {
    field.name() == super::PLUGINCONFIG_CODE_NAME.1
}

impl FixRegistry {
    fn is_crate_definition(&self, category: FixCategory, field: &Field) -> bool {
        is_crate_field(field)
            || is_crate_message(field)
            || self
                .get_definition(category, field.name())
                .is_some_and(|held| is_crate_field(held) || is_crate_message(held))
    }

    /// Reads a complete registry snapshot from JSON.
    ///
    /// `fields`, `components`, and `groups` are arrays of native Field
    /// documents; a message is a component carrying `fix:msgtype`, and any
    /// other key - `messages` among them - is refused by name. References
    /// resolve through the same bounded graph loader as the store.
    pub fn from_json(input: &str) -> Result<Self> {
        Self::from_snapshot(&crate::from_json_scalar(input)?)
    }

    /// Renders all categories as canonical JSON.
    ///
    /// Referenced occurrences keep their native Null placeholders; loading
    /// reconstructs the same resolved catalog. No filesystem I/O is performed.
    ///
    /// ```
    /// use yggdryl::{DataType, FixCategory, FixRegistry};
    /// let mut registry = FixRegistry::new();
    /// let mut message = DataType::from_fields([])?.required_field("Order");
    /// message.as_fix_mut().set_msgtype("D")?;
    /// registry.create_definition(FixCategory::Components, message)?;
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
    /// `fix:branches` its writer meant. Naming one here would overwrite that.
    ///
    /// A snapshot restating one of this crate's own fields, or the
    /// `pluginconfig` message, is read past rather than refused: every
    /// registry holds those from construction, so the held definition stays
    /// and the counts do not move.
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
        let mut document = Vec::with_capacity(FixCategory::ALL.len());
        for category in FixCategory::ALL {
            // Every definition the dictionary holds, the crate's own among
            // them: a snapshot is the whole row as this registry types it,
            // not the half a store happened to declare. Reading one back
            // takes the crate's copy over the document's, so the two never
            // collide; the folder store writes the same way in `write_into`.
            let fields = if category == FixCategory::Fields {
                self.iter()
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
        Scalar::from_record(document)
    }

    fn from_snapshot(document: &Scalar) -> Result<Self> {
        let record = document.as_record().ok_or_else(|| Error::InvalidRecord {
            path: "fix registry".into(),
            reason: crate::text::expected_got("a JSON registry object", document.kind()),
        })?;
        for key in record.keys() {
            if !FixCategory::ALL
                .iter()
                .any(|category| category.as_str() == key)
            {
                return Err(Error::InvalidRecord {
                    path: key.clone(),
                    reason: "expected fields, components, or groups".into(),
                });
            }
        }
        let mut registry = Self::base();
        let mut raw = BTreeMap::new();
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

    /// Writes category documents and removes definitions no longer held.
    ///
    /// Each document is written through the handle's byte-write contract.
    /// Referenced children persist as native Null-typed Fields and regain
    /// their resolved datatypes when loaded.
    pub fn write_into(&self, root: &mut dyn IOBase) -> Result<()> {
        self.validate_catalog()?;
        let mut documents: BTreeMap<String, Scalar> = BTreeMap::new();
        let mut shards: BTreeMap<i32, Vec<Field>> = BTreeMap::new();
        // Every field the dictionary holds, the crate's own among them, so a
        // written store states the whole row rather than the half it declared
        // itself. The crate's block is one shard of its own, above every tag
        // a dictionary reaches, and a reader takes the held definition over
        // the document it finds there.
        for field in self {
            let (tag, _) = super::registry::canonical_identity(field)?;
            shards.entry(shard_of(tag)).or_default().push(field.clone());
        }
        for (shard, fields) in shards {
            documents.insert(
                format!("fields/{shard}.json"),
                Scalar::from_sequence(
                    fields
                        .into_iter()
                        .map(|field| super::document::dump(field.into_value()))
                        .collect::<Result<Vec<_>>>()?,
                ),
            );
        }
        for entry in self.catalog.all() {
            // And every named definition, the crate's own message among them,
            // for the reason its own fields are written above.
            let path = format!("{}/{}.json", entry.category, entry.field.name());
            documents.insert(
                path,
                super::document::dump(compact(entry.field.as_field().clone(), true)?.into_value())?,
            );
        }
        for (path, document) in &documents {
            let bytes =
                crate::text::json::into_bytes_with_formatting(document, Formatting::indented(2))?;
            root.child_by_path(path)?.write_all_bytes(&bytes)?;
        }
        for category in FixCategory::ALL {
            let prefix = format!("{category}/");
            let mut tree = root.child_by_path(category.as_str())?;
            if !documents.keys().any(|path| path.starts_with(&prefix)) {
                tree.remove(true)?;
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
                    && !documents.contains_key(&format!("{category}/{name}"))
                {
                    entry.remove(false)?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod clock_seed_tests {
    use super::*;

    #[test]
    fn stored_clocks_replace_seeds_but_duplicate_documents_refuse() {
        let registry = FixRegistry::new();
        let mut sending = registry.field_by_tag(52).unwrap().clone();
        sending
            .set_description("store owns this declaration")
            .unwrap();
        let field = sending.clone().into_json().unwrap();
        let document = format!(r#"{{"fields":[{field}],"components":[],"groups":[]}}"#);
        let loaded = FixRegistry::from_json(&document).unwrap();
        assert_eq!(loaded.field_by_tag(52).unwrap(), &sending);
        assert_eq!(
            loaded.field_by_tag(60).unwrap().dtype(),
            &super::super::schema::CLOCK_DATATYPE
        );
        assert_eq!(
            FixRegistry::from_json(&loaded.into_json().unwrap()).unwrap(),
            loaded
        );
        let duplicate = format!(r#"{{"fields":[{field},{field}],"components":[],"groups":[]}}"#);
        assert!(matches!(
            FixRegistry::from_json(&duplicate),
            Err(Error::Conflict { .. })
        ));
    }

    #[test]
    fn renamed_loaded_clock_keeps_its_canonical_identity() {
        let registry = FixRegistry::new();
        let sending = registry
            .field_by_tag(52)
            .unwrap()
            .clone()
            .with_name("VenueSendingClock");
        let field = sending.into_json().unwrap();
        let document = format!(r#"{{"fields":[{field}],"components":[],"groups":[]}}"#);
        let loaded = FixRegistry::from_json(&document).unwrap();
        assert_eq!(loaded.field_by_tag(52).unwrap().name(), "VenueSendingClock");
        assert!(loaded.get_field_by_name("SendingTime").is_none());
        assert_eq!(loaded.field_by_tag(60).unwrap().name(), "transacttime");
    }
}
