//! The warehouse, reached from JavaScript: catalogs, namespaces and tables as
//! the core enums they are, the registry a path resolves against, and the
//! process's one registry.
//!
//! An object crosses as a napi class wrapping the core enum - `Catalog`,
//! `Namespace`, `Table` - its implementation named by `implementation`
//! rather than by a subclass, because JavaScript dispatches nothing on one
//! here, and built by a static constructor per implementation:
//! `Catalog.memory`, `Catalog.folder`, `Namespace.memory`, `Namespace.folder`,
//! `Table.media`. A view - `Namespaces`, `Tables` - holds its own clone of
//! the parent object, so it stands on its own, and every question it answers
//! is asked of the store when it is asked of the view, as the core's views
//! are. Properties cross as an ordered plain object, a path as dotted text
//! read through the plan's location grammar or as its parts, and an object is
//! held as the handle it is by `IOBase.from`, through `iobase`.

use napi::bindgen_prelude::{BigInt, ClassInstance, Either, Either3, Env, Object, Result};
use napi_derive::napi;
use yggdryl::{
    Catalog as CoreCatalog, CatalogValue as _, FolderCatalog, FolderLayout, FolderNamespace,
    IntoObjectPath, MediaTable, MemoryCatalog, MemoryNamespace, Names as CoreNames,
    Namespace as CoreNamespace, NamespaceValue, Namespaces as CoreNamespaces, Object as CoreObject,
    ObjectValue, Objects as CoreObjects, Properties, SystemWarehouse as CoreSystemWarehouse,
    Table as CoreTable, TableValue as _, Tables as CoreTables, Warehouse as CoreWarehouse,
};

use crate::datatype::{DataTypeInput, dtype_from_input};
use crate::field::JsField;
use crate::iceberg::{FieldInput, field_from_input};
use crate::iobase::{LocationInput, safe_js_len, site_from_input};
use crate::iomedia::JsBatchReader;
use crate::media::options::JsRecordOptions;
use crate::napi_error;
use crate::uri::JsUrl;

/// A path into a warehouse: dotted text, read through the plan's location
/// grammar so quoting is the grammar's, or the parts as they are.
pub type ObjectPathInput = Either<String, Vec<String>>;

/// A warehouse object of any kind: a catalog, a namespace or a table.
pub type ObjectInput<'a> = Either3<
    ClassInstance<'a, JsWarehouseCatalog>,
    ClassInstance<'a, JsWarehouseNamespace>,
    ClassInstance<'a, JsWarehouseTable>,
>;

/// A warehouse object answered: the class its kind is.
pub type ObjectOutput = Either3<JsWarehouseCatalog, JsWarehouseNamespace, JsWarehouseTable>;

/// A location: a native `Url`, or text read as one.
pub type UrlInput<'a> = Either<ClassInstance<'a, JsUrl>, String>;

/// The parts a path input names, ready for every core door that takes one.
///
/// The parts cross as the core's own path type, which this crate never names:
/// text goes through the grammar, and parts arrive as they are. The failure
/// is the core's, so a caller deciding on its kind still can.
pub(crate) fn object_path(value: ObjectPathInput) -> yggdryl::Result<impl IntoObjectPath> {
    match value {
        Either::A(text) => text.into_object_path(),
        Either::B(parts) => parts
            .iter()
            .map(String::as_str)
            .collect::<Vec<&str>>()
            .into_object_path(),
    }
}

/// The core object a JavaScript one wraps.
///
/// A clone, since the JavaScript value keeps its own: a clone of an object
/// starts with its handle unresolved and rebuilds it from its location on
/// first use, so an object bound to a handle with no location - an in-memory
/// buffer - says so by name when the clone first needs one.
pub(crate) fn object_from_input(value: ObjectInput<'_>) -> CoreObject {
    match value {
        Either3::A(catalog) => CoreObject::Catalog(catalog.inner.clone()),
        Either3::B(namespace) => CoreObject::Namespace(namespace.inner.clone()),
        Either3::C(table) => CoreObject::Table(table.inner.clone()),
    }
}

/// The class a core object crosses as.
fn object_output(object: CoreObject) -> ObjectOutput {
    match object {
        CoreObject::Catalog(catalog) => Either3::A(JsWarehouseCatalog::from_core(catalog)),
        CoreObject::Namespace(namespace) => Either3::B(JsWarehouseNamespace::from_core(namespace)),
        CoreObject::Table(table) => Either3::C(JsWarehouseTable::from_core(table)),
        _ => unreachable!("every object kind is a class above"),
    }
}

/// Read a properties bag: a plain object, its keys in order, each value text
/// or a number or boolean spelled as text; an `undefined` value is skipped,
/// the project's spelling for an argument that was not given.
pub(crate) fn properties_from_input(value: Option<Object<'_>>) -> Result<Properties> {
    let mut properties = Properties::new();
    let Some(object) = value else {
        return Ok(properties);
    };
    for name in Object::keys(&object)? {
        let value = object
            .get::<Either3<String, f64, bool>>(&name)
            .map_err(|error| {
                napi_error(format!(
                    "property {name:?} must be text, a number or a boolean: {}",
                    error.reason
                ))
            })?;
        let Some(value) = value else {
            continue;
        };
        let text = match value {
            Either3::A(text) => text,
            Either3::B(number) => number.to_string(),
            Either3::C(flag) => flag.to_string(),
        };
        properties.set(name.as_str(), text.as_str());
    }
    Ok(properties)
}

/// A properties bag as an ordered plain object.
fn properties_object<'env>(env: &'env Env, properties: &Properties) -> Result<Object<'env>> {
    let mut object = Object::new(env)?;
    for (name, value) in properties {
        object.set(name, value)?;
    }
    Ok(object)
}

/// Persist updates and removals of what an object's store keeps for it.
fn update_object_properties(
    object: &dyn ObjectValue,
    updates: Option<Object<'_>>,
    removes: Option<Vec<String>>,
) -> Result<()> {
    let updates = properties_from_input(updates)?;
    let removes = removes.unwrap_or_default();
    // The names cross as the core's own name type, inferred from the slice
    // the core takes rather than named here.
    let removes: Vec<_> = removes
        .iter()
        .map(|name| From::from(name.as_str()))
        .collect();
    object
        .update_properties(&updates, removes.as_slice())
        .map_err(napi_error)
}

/// The parts of a path as JavaScript strings.
fn path_strings(object: &dyn ObjectValue) -> Vec<String> {
    object.path().iter().map(ToString::to_string).collect()
}

/// When an object last changed, as the `bigint` nanoseconds it is.
fn modified_of(object: &dyn ObjectValue) -> Option<BigInt> {
    object.modified().map(BigInt::from)
}

/// Where an object's storage is, when it has a location.
fn url_of(object: &dyn ObjectValue) -> Option<JsUrl> {
    object.url().cloned().map(JsUrl::from_core)
}

/// What a static constructor states beside the object's own arguments.
///
/// Every field is optional and each implementation reads the ones it has:
/// `description` and `properties` on every object, `objects` on a memory
/// catalog or namespace, `levels` on a folder catalog or namespace, `field`,
/// `dtype` and `layout` on a media table. A field the implementation has no
/// use for is refused by name rather than ignored.
#[napi(object)]
pub struct ObjectOptions<'env> {
    /// What the object's store says it is.
    pub description: Option<String>,
    /// What the object states, which every object under it inherits and
    /// every handle under it opens with: an ordered plain object.
    #[napi(ts_type = "Record<string, string | number | boolean>")]
    pub properties: Option<Object<'env>>,
    /// How many namespace levels sit under a folder catalog (one by
    /// default, `catalog.schema.table`) or a folder namespace (none).
    pub levels: Option<u32>,
    /// A media table's declared row schema, renamed after the table.
    pub field: Option<FieldInput<'env>>,
    /// A media table's declared row datatype, under a required field.
    pub dtype: Option<DataTypeInput<'env>>,
    /// How a media table holds its rows: `leaf`, `folder` or `format`.
    pub layout: Option<String>,
    /// The objects a memory catalog or namespace registers, in order.
    pub objects: Option<Vec<ObjectInput<'env>>>,
}

/// The options read, each field taken once.
pub(crate) struct Stated {
    pub(crate) description: Option<String>,
    pub(crate) properties: Properties,
    levels: Option<usize>,
    field: Option<yggdryl::Field>,
    dtype: Option<yggdryl::DataType>,
    layout: Option<FolderLayout>,
    objects: Vec<CoreObject>,
}

impl Stated {
    pub(crate) fn read(options: Option<ObjectOptions<'_>>) -> Result<Self> {
        let Some(options) = options else {
            return Ok(Self {
                description: None,
                properties: Properties::new(),
                levels: None,
                field: None,
                dtype: None,
                layout: None,
                objects: Vec::new(),
            });
        };
        Ok(Self {
            description: options.description,
            properties: properties_from_input(options.properties)?,
            levels: options.levels.map(|levels| levels as usize),
            field: options.field.map(field_from_input).transpose()?,
            dtype: options.dtype.map(dtype_from_input).transpose()?,
            layout: options
                .layout
                .as_deref()
                .map(str::parse::<FolderLayout>)
                .transpose()
                .map_err(napi_error)?,
            objects: options
                .objects
                .unwrap_or_default()
                .into_iter()
                .map(object_from_input)
                .collect(),
        })
    }

    /// Refuse the options an implementation has no use for, by name.
    pub(crate) fn only(
        &self,
        implementation: &str,
        levels: bool,
        table: bool,
        objects: bool,
    ) -> Result<()> {
        let unused = [
            (!levels && self.levels.is_some(), "levels"),
            (!table && self.field.is_some(), "field"),
            (!table && self.dtype.is_some(), "dtype"),
            (!table && self.layout.is_some(), "layout"),
            (!objects && !self.objects.is_empty(), "objects"),
        ];
        for (present, name) in unused {
            if present {
                return Err(napi_error(format!(
                    "expected no `{name}` option on a {implementation}, got one"
                )));
            }
        }
        Ok(())
    }
}

/// A warehouse catalog: the first namespace layer, what a warehouse registers
/// by name.
///
/// Built by [`memory`](Self::memory) over registered objects, by
/// [`folder`](Self::folder) over a container read as namespaces and tables,
/// or by [`fromUrl`](Self::from_url) under a property bag; `implementation`
/// says which. `IOBase.from(catalog)` holds it as the container of handles
/// it is.
#[napi(js_name = "Catalog")]
pub struct JsWarehouseCatalog {
    pub(crate) inner: CoreCatalog,
}

impl JsWarehouseCatalog {
    pub(crate) const fn from_core(inner: CoreCatalog) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsWarehouseCatalog {
    /// A catalog called `name` of registered objects, in order, with no
    /// storage behind it: `options.objects` registers each, one level below.
    #[napi(factory)]
    pub fn memory(name: String, options: Option<ObjectOptions<'_>>) -> Result<Self> {
        let stated = Stated::read(options)?;
        stated.only("memory catalog", false, false, true)?;
        let mut catalog = MemoryCatalog::new(name).with_properties(stated.properties);
        if let Some(description) = stated.description {
            catalog = catalog.with_description(description);
        }
        for object in stated.objects {
            catalog = catalog.with_object(object).map_err(napi_error)?;
        }
        Ok(Self::from_core(CoreCatalog::Memory(catalog)))
    }

    /// A catalog called `name` over the container `location` names, read as
    /// namespaces and tables, touching nothing: a handle binds, a location
    /// or an identifier names what opens on first use. `options.levels` is
    /// how many namespace levels sit under it, one by default.
    #[napi(factory)]
    pub fn folder(
        name: String,
        location: LocationInput<'_>,
        options: Option<ObjectOptions<'_>>,
    ) -> Result<Self> {
        let stated = Stated::read(options)?;
        stated.only("folder catalog", true, false, false)?;
        let mut catalog = match site_from_input(location)? {
            Either::A(holder) => FolderCatalog::bound(name, holder),
            Either::B(uri) => FolderCatalog::new(name, uri).map_err(napi_error)?,
        }
        .with_properties(stated.properties);
        if let Some(description) = stated.description {
            catalog = catalog.with_description(description);
        }
        if let Some(levels) = stated.levels {
            catalog = catalog.with_levels(levels);
        }
        Ok(Self::from_core(CoreCatalog::Folder(Box::new(catalog))))
    }

    /// The catalog a URL names, under `properties`, touching no storage: the
    /// `type` property decides - `memory`, or `folder` - and otherwise every
    /// location is a folder catalog; the `name` property names it, else the
    /// location's last segment.
    #[napi(
        factory,
        ts_args_type = "url: Url | string, properties?: Record<string, string | number | boolean> | null"
    )]
    pub fn from_url(url: UrlInput<'_>, properties: Option<Object<'_>>) -> Result<Self> {
        let location = identifier_from_input(url)?;
        CoreCatalog::from_url(&location, &properties_from_input(properties)?)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The catalog's name, the one part of its path.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_owned()
    }

    /// Its path: its name alone.
    #[napi(getter)]
    pub fn path(&self) -> Vec<String> {
        path_strings(&self.inner)
    }

    /// `catalog`.
    #[napi(getter)]
    pub fn kind(&self) -> String {
        ObjectValue::kind(&self.inner).as_str().to_owned()
    }

    /// The implementation answering it: `MemoryCatalog` or `FolderCatalog`.
    #[napi(getter)]
    pub fn implementation(&self) -> String {
        match &self.inner {
            CoreCatalog::Memory(_) => "MemoryCatalog",
            CoreCatalog::Folder(_) => "FolderCatalog",
            CoreCatalog::Iceberg(_) => "IcebergCatalog",
            _ => "Catalog",
        }
        .to_owned()
    }

    /// What its store says it is, when it says anything.
    #[napi(getter)]
    pub fn description(&self) -> Option<String> {
        self.inner.description().map(str::to_owned)
    }

    /// Where its storage is, when it has a location.
    #[napi(getter)]
    pub fn url(&self) -> Option<JsUrl> {
        url_of(&self.inner)
    }

    /// When it last changed, in UTC nanoseconds since the epoch, when the
    /// store keeps that fact.
    #[napi(getter)]
    pub fn modified(&self) -> Option<BigInt> {
        modified_of(&self.inner)
    }

    /// Its effective properties, in order: what it states, which every
    /// object under it inherits.
    #[napi(getter, ts_return_type = "Record<string, string>")]
    pub fn properties<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        properties_object(env, &self.inner.properties().map_err(napi_error)?)
    }

    /// Persist updates and removals of what its store keeps for it; an
    /// implementation that keeps nothing refuses by name.
    #[napi(
        ts_args_type = "updates?: Record<string, string | number | boolean> | null, removes?: readonly string[] | null"
    )]
    pub fn update_properties(
        &self,
        updates: Option<Object<'_>>,
        removes: Option<Vec<String>>,
    ) -> Result<()> {
        update_object_properties(&self.inner, updates, removes)
    }

    /// How many namespace levels may sit under it: a number for a folder
    /// catalog, `null` where namespaces nest to any depth.
    #[napi(getter)]
    pub fn namespace_levels(&self) -> Option<u32> {
        self.inner
            .namespace_levels()
            .map(|levels| u32::try_from(levels).unwrap_or(u32::MAX))
    }

    /// The namespaces one level down, as a lazy map-like view.
    #[napi]
    pub fn namespaces(&self) -> JsWarehouseNamespaces {
        JsWarehouseNamespaces {
            parent: Parent::Catalog(self.inner.clone()),
        }
    }

    /// The tables one level down, as a lazy map-like view.
    #[napi]
    pub fn tables(&self) -> JsWarehouseTables {
        JsWarehouseTables {
            parent: Parent::Catalog(self.inner.clone()),
        }
    }

    /// Its children, one at a time in the store's order, each carrying its
    /// effective properties; the loader wires `Symbol.iterator`, so
    /// `for...of` walks it.
    #[napi]
    pub fn children(&self) -> JsObjectIterator {
        JsObjectIterator {
            objects: self.inner.children(),
        }
    }

    /// The child called `name`, one level down.
    #[napi]
    pub fn get(&self, name: String) -> Result<ObjectOutput> {
        self.inner.get(&name).map(object_output).map_err(napi_error)
    }

    /// The object a path below this catalog names, one `get` per part.
    #[napi]
    pub fn resolve(&self, path: ObjectPathInput) -> Result<ObjectOutput> {
        self.inner
            .resolve(object_path(path).map_err(napi_error)?)
            .map(object_output)
            .map_err(napi_error)
    }

    /// The table a path below this catalog names.
    #[napi]
    pub fn table(&self, path: ObjectPathInput) -> Result<JsWarehouseTable> {
        self.inner
            .table(object_path(path).map_err(napi_error)?)
            .map(JsWarehouseTable::from_core)
            .map_err(napi_error)
    }

    /// The namespace a path below this catalog names.
    #[napi]
    pub fn namespace(&self, path: ObjectPathInput) -> Result<JsWarehouseNamespace> {
        self.inner
            .namespace(object_path(path).map_err(napi_error)?)
            .map(JsWarehouseNamespace::from_core)
            .map_err(napi_error)
    }

    /// Create the namespace `name` under it; an implementation that creates
    /// nothing refuses by name.
    #[napi(
        ts_args_type = "name: string, properties?: Record<string, string | number | boolean> | null"
    )]
    pub fn create_namespace(
        &self,
        name: String,
        properties: Option<Object<'_>>,
    ) -> Result<JsWarehouseNamespace> {
        self.inner
            .create_namespace(&name, &properties_from_input(properties)?)
            .map(JsWarehouseNamespace::from_core)
            .map_err(napi_error)
    }

    /// Create the table `name` under it, `field` its row schema; an
    /// implementation that creates nothing refuses by name.
    #[napi(
        ts_args_type = "name: string, field: Field | string, properties?: Record<string, string | number | boolean> | null"
    )]
    pub fn create_table(
        &self,
        name: String,
        field: FieldInput<'_>,
        properties: Option<Object<'_>>,
    ) -> Result<JsWarehouseTable> {
        self.inner
            .create_table(
                &name,
                &field_from_input(field)?,
                &properties_from_input(properties)?,
            )
            .map(JsWarehouseTable::from_core)
            .map_err(napi_error)
    }

    /// Whether both describe the same catalog: the same implementation, path,
    /// location, statement and registered objects.
    #[napi]
    pub fn equals(&self, other: &JsWarehouseCatalog) -> bool {
        self.inner == other.inner
    }

    /// The dotted path, as the plan grammar spells it.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }
}

/// A warehouse namespace: a container of namespaces and tables.
///
/// Built by [`memory`](Self::memory) over registered objects or by
/// [`folder`](Self::folder) over a container, and answered by every catalog
/// or namespace listing one; `implementation` says which.
/// `IOBase.from(namespace)` holds it as the container of handles it is.
#[napi(js_name = "Namespace")]
pub struct JsWarehouseNamespace {
    pub(crate) inner: CoreNamespace,
}

impl JsWarehouseNamespace {
    pub(crate) const fn from_core(inner: CoreNamespace) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsWarehouseNamespace {
    /// A namespace at `path` - its catalog's name first - of registered
    /// objects, in order, with no storage behind it: `options.objects`
    /// registers each, one level below.
    #[napi(factory)]
    pub fn memory(path: ObjectPathInput, options: Option<ObjectOptions<'_>>) -> Result<Self> {
        let stated = Stated::read(options)?;
        stated.only("memory namespace", false, false, true)?;
        let mut namespace = MemoryNamespace::new(object_path(path).map_err(napi_error)?)
            .map_err(napi_error)?
            .with_properties(stated.properties);
        if let Some(description) = stated.description {
            namespace = namespace.with_description(description);
        }
        for object in stated.objects {
            namespace = namespace.with_object(object).map_err(napi_error)?;
        }
        Ok(Self::from_core(CoreNamespace::Memory(namespace)))
    }

    /// A namespace at `path` - its catalog's name first - over the container
    /// `location` names, read as tables and, under `options.levels` (none by
    /// default), namespaces: a handle binds, a location or an identifier
    /// names what opens on first use.
    #[napi(factory)]
    pub fn folder(
        path: ObjectPathInput,
        location: LocationInput<'_>,
        options: Option<ObjectOptions<'_>>,
    ) -> Result<Self> {
        let stated = Stated::read(options)?;
        stated.only("folder namespace", true, false, false)?;
        let path = object_path(path).map_err(napi_error)?;
        let mut namespace = match site_from_input(location)? {
            Either::A(holder) => FolderNamespace::bound(path, holder),
            Either::B(uri) => FolderNamespace::new(path, uri),
        }
        .map_err(napi_error)?
        .with_properties(stated.properties);
        if let Some(description) = stated.description {
            namespace = namespace.with_description(description);
        }
        if let Some(levels) = stated.levels {
            namespace = namespace.with_levels(levels);
        }
        Ok(Self::from_core(CoreNamespace::Folder(Box::new(namespace))))
    }

    /// The last part of its path.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_owned()
    }

    /// Its parts, from its catalog's name down to its own.
    #[napi(getter)]
    pub fn path(&self) -> Vec<String> {
        path_strings(&self.inner)
    }

    /// `namespace`.
    #[napi(getter)]
    pub fn kind(&self) -> String {
        ObjectValue::kind(&self.inner).as_str().to_owned()
    }

    /// The implementation answering it: `MemoryNamespace` or
    /// `FolderNamespace`.
    #[napi(getter)]
    pub fn implementation(&self) -> String {
        match &self.inner {
            CoreNamespace::Memory(_) => "MemoryNamespace",
            CoreNamespace::Folder(_) => "FolderNamespace",
            CoreNamespace::Iceberg(_) => "IcebergNamespace",
            _ => "Namespace",
        }
        .to_owned()
    }

    /// What its store says it is, when it says anything.
    #[napi(getter)]
    pub fn description(&self) -> Option<String> {
        self.inner.description().map(str::to_owned)
    }

    /// Where its storage is, when it has a location.
    #[napi(getter)]
    pub fn url(&self) -> Option<JsUrl> {
        url_of(&self.inner)
    }

    /// When it last changed, in UTC nanoseconds since the epoch, when the
    /// store keeps that fact.
    #[napi(getter)]
    pub fn modified(&self) -> Option<BigInt> {
        modified_of(&self.inner)
    }

    /// Its effective properties, in order: its parent's, then what it
    /// states, a later entry replacing an earlier one by name.
    #[napi(getter, ts_return_type = "Record<string, string>")]
    pub fn properties<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        properties_object(env, &self.inner.properties().map_err(napi_error)?)
    }

    /// Persist updates and removals of what its store keeps for it; an
    /// implementation that keeps nothing refuses by name.
    #[napi(
        ts_args_type = "updates?: Record<string, string | number | boolean> | null, removes?: readonly string[] | null"
    )]
    pub fn update_properties(
        &self,
        updates: Option<Object<'_>>,
        removes: Option<Vec<String>>,
    ) -> Result<()> {
        update_object_properties(&self.inner, updates, removes)
    }

    /// The namespaces one level down, as a lazy map-like view.
    #[napi]
    pub fn namespaces(&self) -> JsWarehouseNamespaces {
        JsWarehouseNamespaces {
            parent: Parent::Namespace(self.inner.clone()),
        }
    }

    /// The tables one level down, as a lazy map-like view.
    #[napi]
    pub fn tables(&self) -> JsWarehouseTables {
        JsWarehouseTables {
            parent: Parent::Namespace(self.inner.clone()),
        }
    }

    /// Its children, one at a time in the store's order, each carrying its
    /// effective properties; the loader wires `Symbol.iterator`, so
    /// `for...of` walks it.
    #[napi]
    pub fn children(&self) -> JsObjectIterator {
        JsObjectIterator {
            objects: self.inner.children(),
        }
    }

    /// The child called `name`, one level down.
    #[napi]
    pub fn get(&self, name: String) -> Result<ObjectOutput> {
        self.inner.get(&name).map(object_output).map_err(napi_error)
    }

    /// The object a path below this namespace names, one `get` per part.
    #[napi]
    pub fn resolve(&self, path: ObjectPathInput) -> Result<ObjectOutput> {
        self.inner
            .resolve(object_path(path).map_err(napi_error)?)
            .map(object_output)
            .map_err(napi_error)
    }

    /// The table a path below this namespace names.
    #[napi]
    pub fn table(&self, path: ObjectPathInput) -> Result<JsWarehouseTable> {
        self.inner
            .resolve(object_path(path).map_err(napi_error)?)
            .and_then(CoreObject::into_table)
            .map(JsWarehouseTable::from_core)
            .map_err(napi_error)
    }

    /// The namespace a path below this namespace names.
    #[napi]
    pub fn namespace(&self, path: ObjectPathInput) -> Result<JsWarehouseNamespace> {
        self.inner
            .resolve(object_path(path).map_err(napi_error)?)
            .and_then(CoreObject::into_namespace)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Create the namespace `name` under it; an implementation that creates
    /// nothing refuses by name.
    #[napi(
        ts_args_type = "name: string, properties?: Record<string, string | number | boolean> | null"
    )]
    pub fn create_namespace(
        &self,
        name: String,
        properties: Option<Object<'_>>,
    ) -> Result<JsWarehouseNamespace> {
        self.inner
            .create_namespace(&name, &properties_from_input(properties)?)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Create the table `name` under it, `field` its row schema; an
    /// implementation that creates nothing refuses by name.
    #[napi(
        ts_args_type = "name: string, field: Field | string, properties?: Record<string, string | number | boolean> | null"
    )]
    pub fn create_table(
        &self,
        name: String,
        field: FieldInput<'_>,
        properties: Option<Object<'_>>,
    ) -> Result<JsWarehouseTable> {
        self.inner
            .create_table(
                &name,
                &field_from_input(field)?,
                &properties_from_input(properties)?,
            )
            .map(JsWarehouseTable::from_core)
            .map_err(napi_error)
    }

    /// Whether both describe the same namespace.
    #[napi]
    pub fn equals(&self, other: &JsWarehouseNamespace) -> bool {
        self.inner == other.inner
    }

    /// The dotted path, as the plan grammar spells it.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }
}

/// A warehouse table: an object whose rows any record read and write
/// reaches.
///
/// Built by [`media`](Self::media) over any location a record medium reads,
/// and answered by every catalog or namespace listing one. Its rows are read
/// and written through `IOBase.from(table)`, which holds it as the handle its
/// implementation is: every record verb is `IOBase`'s.
#[napi(js_name = "Table")]
pub struct JsWarehouseTable {
    pub(crate) inner: CoreTable,
}

impl JsWarehouseTable {
    pub(crate) const fn from_core(inner: CoreTable) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsWarehouseTable {
    /// A table at `path` over the storage `location` names - a leaf a record
    /// medium reads, a folder read as the rows beneath it, or a folder laid
    /// out as a table format, said by `options.layout` - touching nothing: a
    /// handle binds, a location or an identifier names what opens on first
    /// use. `options.field` or `options.dtype` declares its row schema.
    #[napi(factory)]
    pub fn media(
        path: ObjectPathInput,
        location: LocationInput<'_>,
        options: Option<ObjectOptions<'_>>,
    ) -> Result<Self> {
        let stated = Stated::read(options)?;
        stated.only("media table", false, true, false)?;
        let path = object_path(path).map_err(napi_error)?;
        let mut table = match site_from_input(location)? {
            Either::A(holder) => MediaTable::bound(path, holder),
            Either::B(uri) => MediaTable::new(path, uri),
        }
        .map_err(napi_error)?
        .with_properties(stated.properties);
        if let Some(description) = stated.description {
            table = table.with_description(description);
        }
        if let Some(field) = stated.field {
            table = table.with_field(field);
        }
        if let Some(dtype) = stated.dtype {
            table = table.with_dtype(dtype).map_err(napi_error)?;
        }
        if let Some(layout) = stated.layout {
            table = table.with_layout(layout);
        }
        Ok(Self::from_core(CoreTable::Media(Box::new(table))))
    }

    /// The last part of its path.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_owned()
    }

    /// Its parts, from its catalog's name down to its own.
    #[napi(getter)]
    pub fn path(&self) -> Vec<String> {
        path_strings(&self.inner)
    }

    /// `table`.
    #[napi(getter)]
    pub fn kind(&self) -> String {
        ObjectValue::kind(&self.inner).as_str().to_owned()
    }

    /// The implementation holding it: `MediaTable`.
    #[napi(getter)]
    pub fn implementation(&self) -> String {
        match &self.inner {
            CoreTable::Media(_) => "MediaTable",
            CoreTable::Iceberg(_) => "IcebergTable",
            _ => "Table",
        }
        .to_owned()
    }

    /// What its store says it is, when it says anything.
    #[napi(getter)]
    pub fn description(&self) -> Option<String> {
        self.inner.description().map(str::to_owned)
    }

    /// Where its storage is, when it has a location.
    #[napi(getter)]
    pub fn url(&self) -> Option<JsUrl> {
        url_of(&self.inner)
    }

    /// When it last changed, in UTC nanoseconds since the epoch, when the
    /// store keeps that fact.
    #[napi(getter)]
    pub fn modified(&self) -> Option<BigInt> {
        modified_of(&self.inner)
    }

    /// Its effective properties, in order: its parent's, then what its store
    /// keeps for it, then what it states.
    #[napi(getter, ts_return_type = "Record<string, string>")]
    pub fn properties<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        properties_object(env, &self.inner.properties().map_err(napi_error)?)
    }

    /// Persist updates and removals of what its store keeps for it; an
    /// implementation that keeps nothing refuses by name.
    #[napi(
        ts_args_type = "updates?: Record<string, string | number | boolean> | null, removes?: readonly string[] | null"
    )]
    pub fn update_properties(
        &self,
        updates: Option<Object<'_>>,
        removes: Option<Vec<String>>,
    ) -> Result<()> {
        update_object_properties(&self.inner, updates, removes)
    }

    /// What holds its rows, as a listing describes it: a leaf's media type,
    /// `directory` for a folder read as the rows beneath it, `table` for a
    /// table format.
    #[napi(getter)]
    pub fn storage(&self) -> String {
        self.inner.storage()
    }

    /// How the rows are laid out: `leaf`, `folder` or `format`.
    #[napi(getter)]
    pub fn layout(&self) -> String {
        match &self.inner {
            CoreTable::Media(table) => table.layout().as_str().to_owned(),
            _ => FolderLayout::Leaf.as_str().to_owned(),
        }
    }

    /// The row schema it declares, when one was declared.
    #[napi(getter)]
    pub fn declared_field(&self) -> Option<JsField> {
        match &self.inner {
            CoreTable::Media(table) => table.declared_field().cloned().map(JsField::from_core),
            _ => None,
        }
    }

    /// Its row schema, with no row read: the declared field, else the stored
    /// one.
    #[napi]
    pub fn field(&self) -> Result<JsField> {
        self.inner
            .field()
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// Whether both describe the same table.
    #[napi]
    pub fn equals(&self, other: &JsWarehouseTable) -> bool {
        self.inner == other.inner
    }

    /// The dotted path, as the plan grammar spells it.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }
}

/// The object a view lists under: its own clone, so the view stands alone.
enum Parent {
    Catalog(CoreCatalog),
    Namespace(CoreNamespace),
}

impl Parent {
    /// The object a path below the parent names: text through the grammar,
    /// parts as they are - which is how a name `keys()` answered, spelled
    /// however the store spells it, reaches `get` again.
    fn resolve(&self, path: ObjectPathInput) -> yggdryl::Result<CoreObject> {
        let path = object_path(path)?;
        match self {
            Self::Catalog(catalog) => catalog.resolve(path),
            Self::Namespace(namespace) => namespace.resolve(path),
        }
    }

    /// The namespace a path below the parent names, or its absence.
    fn namespace_at(&self, path: ObjectPathInput) -> yggdryl::Result<CoreNamespace> {
        match path {
            Either::A(text) => self.namespaces().get(&text),
            Either::B(parts) => self.resolve(Either::B(parts))?.into_namespace(),
        }
    }

    /// The table a path below the parent names, or its absence.
    fn table_at(&self, path: ObjectPathInput) -> yggdryl::Result<CoreTable> {
        match path {
            Either::A(text) => self.tables().get(&text),
            Either::B(parts) => self.resolve(Either::B(parts))?.into_table(),
        }
    }

    fn namespaces(&self) -> CoreNamespaces<'_> {
        match self {
            Self::Catalog(catalog) => catalog.namespaces(),
            Self::Namespace(namespace) => namespace.namespaces(),
        }
    }

    fn tables(&self) -> CoreTables<'_> {
        match self {
            Self::Catalog(catalog) => catalog.tables(),
            Self::Namespace(namespace) => namespace.tables(),
        }
    }
}

/// The namespaces one level below a catalog or a namespace, as a lazy
/// map-like view.
///
/// JavaScript has no indexing hook a native class can answer, so the map
/// questions are spelled out: `get` and `has` for membership, `keys` and
/// `size` for the whole level - the loader wires `Symbol.iterator`, `values`
/// and `entries` over `keys` and `get` - and `create` and `openOrCreate` to
/// add one. Nothing is cached: every answer is the store's, asked when the
/// question is. A name may be dotted: `namespaces.get('sales.eu')` descends.
#[napi(js_name = "Namespaces")]
pub struct JsWarehouseNamespaces {
    parent: Parent,
}

#[napi]
impl JsWarehouseNamespaces {
    /// Open the named namespace: dotted text descending through the grammar,
    /// or the parts as they are.
    #[napi]
    pub fn get(&self, name: ObjectPathInput) -> Result<JsWarehouseNamespace> {
        self.parent
            .namespace_at(name)
            .map(JsWarehouseNamespace::from_core)
            .map_err(napi_error)
    }

    /// Whether the named namespace exists, asked of the store now; a table's
    /// name answers `false`.
    #[napi]
    pub fn has(&self, name: ObjectPathInput) -> Result<bool> {
        exists(self.parent.namespace_at(name))
    }

    /// The names one level down, lazily.
    #[napi]
    pub fn keys(&self) -> JsObjectNames {
        JsObjectNames {
            names: self.parent.namespaces().iter(),
        }
    }

    /// How many namespaces are one level down, right now; it drains the
    /// level's listing.
    #[napi(getter)]
    pub fn size(&self) -> Result<i64> {
        self.parent
            .namespaces()
            .len()
            .map(safe_js_len)
            .map_err(napi_error)
    }

    /// Create the named namespace, a dotted name descending to the parent it
    /// is created under; an implementation that creates nothing refuses by
    /// name.
    #[napi(
        ts_args_type = "name: string, properties?: Record<string, string | number | boolean> | null"
    )]
    pub fn create(
        &self,
        name: String,
        properties: Option<Object<'_>>,
    ) -> Result<JsWarehouseNamespace> {
        self.parent
            .namespaces()
            .create(&name, &properties_from_input(properties)?)
            .map(JsWarehouseNamespace::from_core)
            .map_err(napi_error)
    }

    /// Open the named namespace, creating it when absent.
    #[napi(
        ts_args_type = "name: string, properties?: Record<string, string | number | boolean> | null"
    )]
    pub fn open_or_create(
        &self,
        name: String,
        properties: Option<Object<'_>>,
    ) -> Result<JsWarehouseNamespace> {
        self.parent
            .namespaces()
            .open_or_create(&name, &properties_from_input(properties)?)
            .map(JsWarehouseNamespace::from_core)
            .map_err(napi_error)
    }
}

/// The tables one level below a catalog or a namespace, as a lazy map-like
/// view.
///
/// The same shape as [`Namespaces`](JsWarehouseNamespaces), one level down:
/// `get` opens a [`Table`](JsWarehouseTable), and the writes that take a
/// name open the table, create it from the rows' own schema where the
/// implementation creates one, and write through the table's own record
/// surface, answering the table.
#[napi(js_name = "Tables")]
pub struct JsWarehouseTables {
    parent: Parent,
}

#[napi]
impl JsWarehouseTables {
    /// Open the named table: dotted text descending through the grammar, or
    /// the parts as they are.
    #[napi]
    pub fn get(&self, name: ObjectPathInput) -> Result<JsWarehouseTable> {
        self.parent
            .table_at(name)
            .map(JsWarehouseTable::from_core)
            .map_err(napi_error)
    }

    /// Whether the named table exists, asked of the store now; a namespace's
    /// name answers `false`.
    #[napi]
    pub fn has(&self, name: ObjectPathInput) -> Result<bool> {
        exists(self.parent.table_at(name))
    }

    /// The names one level down, lazily.
    #[napi]
    pub fn keys(&self) -> JsObjectNames {
        JsObjectNames {
            names: self.parent.tables().iter(),
        }
    }

    /// How many tables are one level down, right now; it drains the level's
    /// listing.
    #[napi(getter)]
    pub fn size(&self) -> Result<i64> {
        self.parent
            .tables()
            .len()
            .map(safe_js_len)
            .map_err(napi_error)
    }

    /// Create the named table with `field` as its row schema, a dotted name
    /// descending to the namespace it is created under; an implementation
    /// that creates nothing refuses by name.
    #[napi(
        ts_args_type = "name: string, field: Field | string, properties?: Record<string, string | number | boolean> | null"
    )]
    pub fn create(
        &self,
        name: String,
        field: FieldInput<'_>,
        properties: Option<Object<'_>>,
    ) -> Result<JsWarehouseTable> {
        self.parent
            .tables()
            .create(
                &name,
                &field_from_input(field)?,
                &properties_from_input(properties)?,
            )
            .map(JsWarehouseTable::from_core)
            .map_err(napi_error)
    }

    /// Open the named table if it exists, creating it otherwise; an existing
    /// table is opened as it is, `field` describing only the table this call
    /// would create.
    #[napi(
        ts_args_type = "name: string, field: Field | string, properties?: Record<string, string | number | boolean> | null"
    )]
    pub fn open_or_create(
        &self,
        name: String,
        field: FieldInput<'_>,
        properties: Option<Object<'_>>,
    ) -> Result<JsWarehouseTable> {
        self.parent
            .tables()
            .open_or_create(
                &name,
                &field_from_input(field)?,
                &properties_from_input(properties)?,
            )
            .map(JsWarehouseTable::from_core)
            .map_err(napi_error)
    }

    /// Append `data` to the named table, creating it from the reader's own
    /// schema where the implementation creates one, under `options` or the
    /// table's own record options, and answer the table. The loader widens
    /// `data` to anything `BatchReader.from` reads and a property bag onto a
    /// copy of the options.
    #[napi]
    pub fn append(
        &self,
        name: String,
        data: &mut JsBatchReader,
        options: Option<&JsRecordOptions>,
    ) -> Result<JsWarehouseTable> {
        let batches = data.take()?;
        let tables = self.parent.tables();
        match options {
            Some(options) => {
                tables.append_arrow_reader_with_options(&name, batches, &options.inner)
            }
            None => tables.append_arrow_reader(&name, batches),
        }
        .map(JsWarehouseTable::from_core)
        .map_err(napi_error)
    }

    /// Replace the named table's rows with `data`, creating it from the
    /// reader's own schema where the implementation creates one, under
    /// `options` or the table's own record options, and answer the table.
    #[napi]
    pub fn overwrite(
        &self,
        name: String,
        data: &mut JsBatchReader,
        options: Option<&JsRecordOptions>,
    ) -> Result<JsWarehouseTable> {
        let batches = data.take()?;
        let tables = self.parent.tables();
        match options {
            Some(options) => {
                tables.overwrite_arrow_reader_with_options(&name, batches, &options.inner)
            }
            None => tables.overwrite_arrow_reader(&name, batches),
        }
        .map(JsWarehouseTable::from_core)
        .map_err(napi_error)
    }
}

/// Whether an object was there: its absence is `false`, every other failure
/// its own.
fn exists<T>(answer: yggdryl::Result<T>) -> Result<bool> {
    match answer {
        Ok(_) => Ok(true),
        Err(error) if error.is_absent() => Ok(false),
        Err(error) => Err(napi_error(error)),
    }
}

/// The names of one collection level, one at a time.
///
/// Built by `keys()` on `Namespaces` and `Tables`. It wraps the core names
/// iterator directly, so nothing is collected on the way across the
/// boundary; `next()` is the native half of the iteration protocol and the
/// loader wraps it so `for...of` yields strings. A failure throws at the
/// entry it happened on, after which the iterator is exhausted.
#[napi(js_name = "ObjectNames")]
pub struct JsObjectNames {
    names: CoreNames,
}

#[napi]
impl JsObjectNames {
    /// The next name, or `null` when the level is exhausted.
    #[napi]
    pub fn next(&mut self) -> Result<Option<String>> {
        self.names
            .next()
            .transpose()
            .map(|name| name.map(|name| name.to_string()))
            .map_err(napi_error)
    }
}

/// The children of a catalog or a namespace, one at a time.
///
/// Built by `children()`. It wraps the core walk directly, so a caller that
/// takes three children of a hundred thousand pays for three; `next()` is
/// the native half of the iteration protocol and the loader wraps it so
/// `for...of` yields objects. A failure throws at the child it happened on,
/// after which the iterator is exhausted.
#[napi(js_name = "ObjectIterator")]
pub struct JsObjectIterator {
    objects: CoreObjects,
}

#[napi]
impl JsObjectIterator {
    /// The next child, or `null` when the listing is exhausted.
    #[napi]
    pub fn next(&mut self) -> Result<Option<ObjectOutput>> {
        self.objects
            .next()
            .transpose()
            .map(|object| object.map(object_output))
            .map_err(napi_error)
    }
}

/// The URL an input names.
fn url_from_input(value: UrlInput<'_>) -> Result<yggdryl::Url> {
    match value {
        Either::A(url) => Ok(url.inner.clone()),
        Either::B(text) => yggdryl::Url::from_location(&text).map_err(napi_error),
    }
}

/// The identifier an input names, as named: a table bucket's ARN states the
/// region and the account its location does not, so the core locates it.
fn identifier_from_input(value: UrlInput<'_>) -> Result<yggdryl::Uri> {
    match value {
        Either::A(url) => Ok(url.inner.clone().into_uri()),
        Either::B(text) => yggdryl::Uri::from_str(&text).map_err(napi_error),
    }
}

/// The registry of catalogs a path resolves against.
///
/// Catalogs register by name, in order; a namespace or a table registers at
/// its path, the memory catalogs and namespaces along it created as needed.
/// Registration only extends memory catalogs and namespaces: a folder
/// catalog lists its own store, and registering under one is refused by
/// name.
#[napi(js_name = "Warehouse")]
pub struct JsWarehouse {
    inner: CoreWarehouse,
}

impl Default for JsWarehouse {
    fn default() -> Self {
        Self::new()
    }
}

impl JsWarehouse {
    /// The core registry, what a plan resolves against.
    pub(crate) const fn core(&self) -> &CoreWarehouse {
        &self.inner
    }
}

#[napi]
impl JsWarehouse {
    /// An empty warehouse.
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: CoreWarehouse::new(),
        }
    }

    /// Register an object: a catalog by its name, a namespace or a table at
    /// its path; a name taken at that level is a conflict.
    #[napi]
    pub fn register(&mut self, object: ObjectInput<'_>) -> Result<()> {
        self.inner
            .register(object_from_input(object))
            .map_err(napi_error)
    }

    /// Register an object, replacing the one of its name at that level and
    /// answering what was replaced, or `null`.
    #[napi]
    pub fn replace(&mut self, object: ObjectInput<'_>) -> Result<Option<ObjectOutput>> {
        self.inner
            .replace(object_from_input(object))
            .map(|replaced| replaced.map(object_output))
            .map_err(napi_error)
    }

    /// Remove the object registered at `path`, answering it.
    #[napi]
    pub fn unregister(&mut self, path: ObjectPathInput) -> Result<ObjectOutput> {
        self.inner
            .unregister(object_path(path).map_err(napi_error)?)
            .map(object_output)
            .map_err(napi_error)
    }

    /// The catalogs, in registration order.
    #[napi(getter)]
    pub fn catalogs(&self) -> Vec<JsWarehouseCatalog> {
        self.inner
            .catalogs()
            .iter()
            .cloned()
            .map(JsWarehouseCatalog::from_core)
            .collect()
    }

    /// The catalog called `name`.
    #[napi]
    pub fn catalog(&self, name: String) -> Result<JsWarehouseCatalog> {
        self.inner
            .catalog(&name)
            .cloned()
            .map(JsWarehouseCatalog::from_core)
            .map_err(napi_error)
    }

    /// The object a path names: a catalog by its one part, else the object
    /// the catalog's descent reaches.
    #[napi]
    pub fn get(&self, path: ObjectPathInput) -> Result<ObjectOutput> {
        self.inner
            .get(object_path(path).map_err(napi_error)?)
            .map(object_output)
            .map_err(napi_error)
    }

    /// The table a path names.
    #[napi]
    pub fn table(&self, path: ObjectPathInput) -> Result<JsWarehouseTable> {
        self.inner
            .table(object_path(path).map_err(napi_error)?)
            .map(JsWarehouseTable::from_core)
            .map_err(napi_error)
    }

    /// The namespace a path names.
    #[napi]
    pub fn namespace(&self, path: ObjectPathInput) -> Result<JsWarehouseNamespace> {
        self.inner
            .namespace(object_path(path).map_err(napi_error)?)
            .map(JsWarehouseNamespace::from_core)
            .map_err(napi_error)
    }

    /// The effective properties of the deepest registered object whose URL
    /// holds `url` on a path boundary; an object no registered URL holds has
    /// none, which is the empty bag.
    #[napi(ts_return_type = "Record<string, string>")]
    pub fn properties_for<'env>(&self, env: &'env Env, url: UrlInput<'_>) -> Result<Object<'env>> {
        properties_object(env, &self.inner.properties_for(&url_from_input(url)?))
    }
}

/// The process's one warehouse, which a plan's `from catalog.namespace.table`
/// resolves against when it is given no other: the same verbs as
/// [`Warehouse`](JsWarehouse), as static methods.
///
/// It starts with the memory catalog `local`, holding the folder namespaces
/// `temporary`, `home` and `config` over the platform's temporary directory,
/// the user's home and its `.config`.
#[napi(js_name = "SystemWarehouse")]
pub struct JsSystemWarehouse {}

#[napi]
impl JsSystemWarehouse {
    /// Register an object on the system warehouse.
    #[napi]
    pub fn register(object: ObjectInput<'_>) -> Result<()> {
        CoreSystemWarehouse::register(object_from_input(object)).map_err(napi_error)
    }

    /// Register an object on the system warehouse, replacing the one of its
    /// name at that level and answering what was replaced, or `null`.
    #[napi]
    pub fn replace(object: ObjectInput<'_>) -> Result<Option<ObjectOutput>> {
        CoreSystemWarehouse::replace(object_from_input(object))
            .map(|replaced| replaced.map(object_output))
            .map_err(napi_error)
    }

    /// Remove the object registered at `path` from the system warehouse,
    /// answering it.
    #[napi]
    pub fn unregister(path: ObjectPathInput) -> Result<ObjectOutput> {
        CoreSystemWarehouse::unregister(object_path(path).map_err(napi_error)?)
            .map(object_output)
            .map_err(napi_error)
    }

    /// The registered catalogs, in order.
    #[napi]
    pub fn catalogs() -> Vec<JsWarehouseCatalog> {
        CoreSystemWarehouse::catalogs()
            .into_iter()
            .map(JsWarehouseCatalog::from_core)
            .collect()
    }

    /// The catalog called `name`.
    #[napi]
    pub fn catalog(name: String) -> Result<JsWarehouseCatalog> {
        CoreSystemWarehouse::catalog(&name)
            .map(JsWarehouseCatalog::from_core)
            .map_err(napi_error)
    }

    /// The object a path names.
    #[napi]
    pub fn get(path: ObjectPathInput) -> Result<ObjectOutput> {
        CoreSystemWarehouse::get(object_path(path).map_err(napi_error)?)
            .map(object_output)
            .map_err(napi_error)
    }

    /// The table a path names.
    #[napi]
    pub fn table(path: ObjectPathInput) -> Result<JsWarehouseTable> {
        CoreSystemWarehouse::table(object_path(path).map_err(napi_error)?)
            .map(JsWarehouseTable::from_core)
            .map_err(napi_error)
    }

    /// The namespace a path names.
    #[napi]
    pub fn namespace(path: ObjectPathInput) -> Result<JsWarehouseNamespace> {
        CoreSystemWarehouse::namespace(object_path(path).map_err(napi_error)?)
            .map(JsWarehouseNamespace::from_core)
            .map_err(napi_error)
    }

    /// The effective properties of the deepest registered object whose URL
    /// holds `url`, or the empty bag.
    #[napi(ts_return_type = "Record<string, string>")]
    pub fn properties_for<'env>(env: &'env Env, url: UrlInput<'_>) -> Result<Object<'env>> {
        properties_object(
            env,
            &CoreSystemWarehouse::properties_for(&url_from_input(url)?),
        )
    }
}
