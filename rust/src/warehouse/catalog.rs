//! A catalog: the first namespace layer, what a warehouse registers by name,
//! and the enum that says which implementation answers it.
//!
//! The core answers two catalogs itself - [`MemoryCatalog`] and
//! [`FolderCatalog`] - and holds every other as [`Catalog::Registered`],
//! through [`RegisteredCatalog`]. [`Catalog::from_url`] builds one from a
//! location through the [`CatalogFactory`] claimed for the `type` word it
//! states or for its scheme, claimed once on the register the core claims
//! its own factories on before it answers anything.

use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};

use smol_str::{SmolStr, format_smolstr};

use super::namespace::{Namespaces, Tables, container_object_io, resolve_under};
use super::object::path_text;
use super::{
    FolderCatalog, IntoObjectPath, MemoryCatalog, Namespace, NamespaceValue, Object, ObjectValue,
    Objects, Properties, Table,
};
use crate::media::RecordOptions;
use crate::plugin::{CORE, Register};
use crate::{Arn, Error, Field, IOBase, IOKind, IOMedia, Result, Scheme, Uri, Url};

/// The first namespace layer: what a warehouse registers by name.
pub trait CatalogValue: NamespaceValue {
    /// How many namespace levels may sit under it: `Some(1)` for a folder
    /// catalog read as `catalog.schema.table`, `None` where namespaces nest
    /// to any depth.
    fn namespace_levels(&self) -> Option<usize>;
}

/// A catalog implemented outside the core's memory and folder catalogs, as
/// [`Catalog::Registered`] holds it: the catalog contract, the name a
/// refusal calls the implementation by, and what a trait object owes the
/// derive-heavy enum holding it - a copy, equality, a hash and the downcast
/// [`Catalog::downcast_ref`] reads.
pub trait RegisteredCatalog: CatalogValue + Send + Sync + fmt::Debug {
    /// The implementation's own name, as a refusal names it:
    /// `IcebergCatalog`.
    fn implementation_name(&self) -> &'static str;

    /// A boxed copy.
    fn clone_box(&self) -> Box<dyn RegisteredCatalog>;

    /// Equality across the trait object: the same implementation holding an
    /// equal catalog.
    fn dyn_eq(&self, other: &dyn RegisteredCatalog) -> bool;

    /// The implementation's own hash, into any hasher.
    fn dyn_hash(&self, state: &mut dyn Hasher);

    /// The catalog as `Any`, for [`Catalog::downcast_ref`].
    fn as_any(&self) -> &dyn Any;

    /// The catalog as `Any`, mutably, for [`Catalog::downcast_mut`].
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// The catalog with stated properties, which its storage and every
    /// object under it open with.
    fn with_properties(self: Box<Self>, properties: Properties) -> Box<dyn RegisteredCatalog>;
}

impl Clone for Box<dyn RegisteredCatalog> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl PartialEq for dyn RegisteredCatalog {
    fn eq(&self, other: &Self) -> bool {
        self.dyn_eq(other)
    }
}

impl Eq for dyn RegisteredCatalog {}

impl Hash for dyn RegisteredCatalog {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.dyn_hash(state);
    }
}

/// What builds a catalog an implementation outside the core answers, from a
/// location: claimed once under the `type` word a `with (...)` clause states
/// for it (`hadoop`) and under the URL scheme its locations spell
/// (`s3tables`), whichever it names.
pub trait CatalogFactory: fmt::Debug + Send + Sync + 'static {
    /// The `type` property's word that asks for this catalog, if any:
    /// `hadoop`.
    fn type_word(&self) -> Option<&'static str>;

    /// The scheme a location with no `type` word is this catalog by, if
    /// any: `s3tables`.
    fn scheme(&self) -> Option<Scheme>;

    /// The catalog `name` over `url`, under `properties`, touching no
    /// storage; `arn` is the ARN the location was given as, when it was one.
    ///
    /// # Errors
    ///
    /// Returns the implementation's refusal of the location or of a
    /// property.
    fn catalog(
        &self,
        name: SmolStr,
        url: &Url,
        arn: Option<&Arn>,
        properties: &Properties,
    ) -> Result<Catalog>;
}

/// What a factory is claimed under: a `type` word, or a scheme.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum FactoryKey {
    /// The `type` property's word.
    Type(&'static str),
    /// A location's scheme.
    Scheme(Scheme),
}

impl fmt::Display for FactoryKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Type(word) => write!(formatter, "type `{word}`"),
            Self::Scheme(scheme) => write!(formatter, "scheme `{scheme}`"),
        }
    }
}

/// The two `type` words the core answers itself, read before any claim.
const CORE_TYPES: [&str; 2] = ["memory", "folder"];

static FACTORIES: Register<FactoryKey, &'static dyn CatalogFactory> =
    Register::new("catalog factory");
static SEEDED: OnceLock<()> = OnceLock::new();
/// One claim at a time, so a factory's keys are claimed all or none.
static CLAIMING: Mutex<()> = Mutex::new(());

/// Claim the core's own factories once, before the register answers
/// anything.
fn seed() {
    SEEDED.get_or_init(|| {
        // The core's claims cannot conflict: each key is stated once in the
        // crate.
        #[cfg(feature = "iceberg")]
        claim_unseeded(&crate::iceberg::HADOOP_FACTORY, CORE)
            .expect("the core's own catalog factories claim cleanly");
        #[cfg(feature = "s3tables")]
        claim_unseeded(&crate::s3tables::S3TABLES_FACTORY, CORE)
            .expect("the core's own catalog factories claim cleanly");
    });
}

/// Claim `factory` for the crate `by`: its `type` word and its scheme, each
/// once for the life of the process.
///
/// # Errors
///
/// Returns [`Error::Conflict`] naming the first claimant where a key is
/// claimed already, and [`Error::InvalidRecord`] at `$.with.type` for a
/// claim in the core's own name, a factory naming neither a word nor a
/// scheme, or one naming a word the core answers itself.
pub fn claim_factory(factory: &'static dyn CatalogFactory, by: &'static str) -> Result<()> {
    seed();
    if by == CORE {
        return Err(invalid_claim(format_smolstr!(
            "a catalog factory is claimed by the crate that holds it, never as `{CORE}`"
        )));
    }
    claim_unseeded(factory, by)
}

fn claim_unseeded(factory: &'static dyn CatalogFactory, by: &'static str) -> Result<()> {
    let _claiming = CLAIMING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let keys: Vec<FactoryKey> = factory
        .type_word()
        .map(FactoryKey::Type)
        .into_iter()
        .chain(factory.scheme().map(FactoryKey::Scheme))
        .collect();
    if keys.is_empty() {
        return Err(invalid_claim(SmolStr::new_static(
            "a catalog factory names a type word or a scheme, got neither",
        )));
    }
    if let Some(word) = factory.type_word().filter(|word| CORE_TYPES.contains(word)) {
        return Err(invalid_claim(format_smolstr!(
            "expected a type word other than {}, got `{word}`; the core answers it",
            listed(CORE_TYPES.iter().copied())
        )));
    }
    // Every key checked before any is claimed, so a refused claim leaves the
    // register as it was.
    for key in &keys {
        if let Some(first) = FACTORIES.claimant(key) {
            return Err(Error::Conflict {
                expected: "catalog factory",
                actual: first,
                path: format_smolstr!("{key}"),
            });
        }
    }
    for key in keys {
        FACTORIES.claim(key, factory, by)?;
    }
    Ok(())
}

fn invalid_claim(reason: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$.with.type"),
        reason,
    }
}

/// Every claimed factory, once each, in the order of its first key: the
/// `type` words alphabetically, then the schemes.
#[must_use]
pub fn factories() -> Vec<&'static dyn CatalogFactory> {
    seed();
    let mut factories: Vec<&'static dyn CatalogFactory> = Vec::new();
    for factory in FACTORIES.values() {
        if !factories
            .iter()
            .any(|held| std::ptr::addr_eq(*held, factory))
        {
            factories.push(factory);
        }
    }
    factories
}

/// The factory claimed under the `type` word `word`.
fn typed_factory(word: &str) -> Option<&'static dyn CatalogFactory> {
    factories()
        .into_iter()
        .find(|factory| factory.type_word() == Some(word))
}

/// The factory claimed under `scheme`.
fn scheme_factory(scheme: &Scheme) -> Option<&'static dyn CatalogFactory> {
    seed();
    FACTORIES.get(&FactoryKey::Scheme(scheme.clone()))
}

/// Words as a refusal lists them: each quoted, joined by commas, `or` before
/// the last.
fn listed<'a>(words: impl Iterator<Item = &'a str>) -> String {
    let words: Vec<String> = words.map(|word| format!("`{word}`")).collect();
    match words.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// The refusal of a `type` word no claim answers, listing the words this
/// build answers: a word naming a catalog some build has is refused as this
/// build's lack, any other as the word it is.
fn unknown_type(word: &str) -> Error {
    let claimed = factories();
    let types = listed(
        CORE_TYPES
            .iter()
            .copied()
            .chain(claimed.iter().filter_map(|factory| factory.type_word())),
    );
    Error::InvalidRecord {
        path: SmolStr::new_static("$.with.type"),
        reason: match word {
            "hadoop" | "rest" | "xmla" => format_smolstr!(
                "expected {types}, got `{word}`; this build has no catalog of that type"
            ),
            _ => format_smolstr!("expected {types}, got {word:?}"),
        },
    }
}

/// The implementation a catalog answers through.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Catalog {
    /// Registered objects, in order, with no storage.
    Memory(MemoryCatalog),
    /// A container read as namespaces and tables.
    ///
    /// Boxed: a located catalog carries its location and its stated
    /// properties, several times the size of a memory one.
    Folder(Box<FolderCatalog>),
    /// A catalog an implementation outside the core answers - an Iceberg
    /// warehouse folder, an Amazon S3 Tables table bucket - held through
    /// the contract every such catalog answers.
    Registered(Box<dyn RegisteredCatalog>),
}

impl Catalog {
    /// The catalog a location names, under `properties`, touching no
    /// storage.
    ///
    /// `location` is a [`Url`], or any identifier that locates one
    /// ([`Uri::locator`]): a URN, an ARN. A table bucket's ARN,
    /// `arn:<partition>:s3tables:<region>:<account>:bucket/<name>`, locates
    /// `s3tables://<name>` and is kept beside it as the bucket's ARN, so the
    /// region and the account it states are never asked for again.
    ///
    /// The explicit `type` property decides first - `memory`, `folder`, or
    /// the word a [`CatalogFactory`] is claimed under: `hadoop`, an Iceberg
    /// warehouse folder, PyIceberg's spelling, under the `iceberg` feature -
    /// and otherwise the scheme does: a scheme a factory is claimed under is
    /// that factory's catalog - an `s3tables://<bucket>` location the
    /// bucket's `S3TablesCatalog` under the `s3tables` feature - and every
    /// location a byte backend holds is a folder catalog over the container
    /// it names. The catalog is called what the `name` property says, else
    /// the location's last segment, or its bucket. A property this door does
    /// not read travels on to every handle under the catalog.
    ///
    /// # Errors
    ///
    /// Returns the locator's refusal of an identifier that names no
    /// location, [`Error::InvalidRecord`] at `$.with.type` naming a type no
    /// claim answers, [`Error::Unsupported`] for an `s3tables` location no
    /// factory answers, at `$.with.name` when no name can be read, and the
    /// factory's own refusal.
    pub fn from_url(location: impl AsRef<Uri>, properties: &Properties) -> Result<Self> {
        let location = location.as_ref();
        // An ARN is read once, for what it locates and for itself: a table
        // bucket's states the region and the account its location does not.
        let arn = (location.scheme() == &Scheme::ARN)
            .then(|| Arn::from_uri(location.clone()))
            .transpose()?;
        let located = match &arn {
            Some(arn) => arn.locator()?,
            None => location.locator()?,
        };
        let url = &located;
        let kind = properties.get("type").map(str::trim);
        let factory = match kind {
            None => scheme_factory(url.scheme()),
            Some(word) if CORE_TYPES.contains(&word) => None,
            Some(word) => Some(typed_factory(word).ok_or_else(|| unknown_type(word))?),
        };
        // A table bucket's location with no factory claiming its scheme is
        // refused here, where the location is read, rather than by the folder
        // catalog's handle on first use.
        if kind.is_none() && factory.is_none() && url.scheme().is_s3_tables() {
            return Err(Error::unsupported(
                "holding a location of this scheme; install the crate that claims it and call its \
                 `install()`",
                url.scheme().as_str(),
            ));
        }
        let name = match properties.get("name") {
            Some(name) if !name.is_empty() => SmolStr::new(name),
            _ => catalog_name(url)?,
        };
        if let Some(factory) = factory {
            return factory.catalog(name, url, arn.as_ref(), properties);
        }
        if kind == Some("memory") {
            return Ok(Self::Memory(
                MemoryCatalog::new(name).with_properties(properties.clone()),
            ));
        }
        Ok(Self::Folder(Box::new(
            FolderCatalog::new(name, url.clone())?.with_properties(properties.clone()),
        )))
    }

    /// The implementation's own name: `MemoryCatalog`, `FolderCatalog`, or
    /// what a registered catalog calls itself.
    pub(crate) fn implementation_name(&self) -> &'static str {
        match self {
            Self::Memory(_) => "MemoryCatalog",
            Self::Folder(_) => "FolderCatalog",
            Self::Registered(catalog) => catalog.implementation_name(),
        }
    }

    /// Borrow the implementation through the contract every catalog answers.
    pub fn as_catalog(&self) -> &dyn CatalogValue {
        match self {
            Self::Memory(catalog) => catalog,
            Self::Folder(catalog) => &**catalog,
            Self::Registered(catalog) => &**catalog,
        }
    }

    /// The implementation as the type it is, when it is a `T`: a
    /// [`MemoryCatalog`], a [`FolderCatalog`], or a registered catalog's own
    /// type.
    #[must_use]
    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        let any: &dyn Any = match self {
            Self::Memory(catalog) => catalog,
            Self::Folder(catalog) => &**catalog,
            Self::Registered(catalog) => catalog.as_any(),
        };
        any.downcast_ref()
    }

    /// The implementation as the type it is, mutably, when it is a `T`.
    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        let any: &mut dyn Any = match self {
            Self::Memory(catalog) => catalog,
            Self::Folder(catalog) => &mut **catalog,
            Self::Registered(catalog) => catalog.as_any_mut(),
        };
        any.downcast_mut()
    }

    /// The object a path of parts below this catalog names, descending
    /// through [`NamespaceValue::get`] one part at a time.
    ///
    /// # Errors
    ///
    /// Returns what the level that fails answers.
    pub fn resolve(&self, path: impl IntoObjectPath) -> Result<Object> {
        resolve_under(self, path.into_object_path()?)
    }

    /// The table a path of parts below this catalog names.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] naming the path when nothing - or a
    /// namespace - is there, and what the descent answers otherwise.
    pub fn table(&self, path: impl IntoObjectPath) -> Result<Table> {
        self.resolve(path)?.into_table()
    }

    /// The namespace a path of parts below this catalog names.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] naming the path when nothing - or a table -
    /// is there, and what the descent answers otherwise.
    pub fn namespace(&self, path: impl IntoObjectPath) -> Result<Namespace> {
        self.resolve(path)?.into_namespace()
    }

    /// The namespaces one level down, as a lazy view.
    #[must_use]
    pub fn namespaces(&self) -> Namespaces<'_> {
        Namespaces::of(self)
    }

    /// The tables one level down, as a lazy view.
    #[must_use]
    pub fn tables(&self) -> Tables<'_> {
        Tables::of(self)
    }
}

impl ObjectValue for Catalog {
    fn name(&self) -> &str {
        self.as_catalog().name()
    }

    fn path(&self) -> &[SmolStr] {
        self.as_catalog().path()
    }

    fn kind(&self) -> IOKind {
        self.as_catalog().kind()
    }

    fn description(&self) -> Option<&str> {
        self.as_catalog().description()
    }

    fn url(&self) -> Option<&Url> {
        self.as_catalog().url()
    }

    fn modified(&self) -> Option<i64> {
        self.as_catalog().modified()
    }

    fn properties(&self) -> Result<Properties> {
        self.as_catalog().properties()
    }

    fn update_properties(&self, updates: &Properties, removes: &[SmolStr]) -> Result<()> {
        self.as_catalog().update_properties(updates, removes)
    }
}

impl NamespaceValue for Catalog {
    fn children(&self) -> Objects {
        self.as_catalog().children()
    }

    fn get(&self, name: &str) -> Result<Object> {
        self.as_catalog().get(name)
    }

    fn descend(&self, name: &str) -> Result<Object> {
        self.as_catalog().descend(name)
    }

    fn create_namespace(&self, name: &str, properties: &Properties) -> Result<Namespace> {
        self.as_catalog().create_namespace(name, properties)
    }

    fn create_table(&self, name: &str, field: &Field, properties: &Properties) -> Result<Table> {
        self.as_catalog().create_table(name, field, properties)
    }
}

impl CatalogValue for Catalog {
    fn namespace_levels(&self) -> Option<usize> {
        self.as_catalog().namespace_levels()
    }
}

impl fmt::Display for Catalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::object::write_path(formatter, self.path())
    }
}

impl From<MemoryCatalog> for Catalog {
    fn from(catalog: MemoryCatalog) -> Self {
        Self::Memory(catalog)
    }
}

impl From<FolderCatalog> for Catalog {
    fn from(catalog: FolderCatalog) -> Self {
        Self::Folder(Box::new(catalog))
    }
}

container_object_io!(Catalog);

/// What a location names a catalog after: its last non-empty path segment,
/// its escapes decoded, else its host or its bucket.
fn catalog_name(url: &Url) -> Result<SmolStr> {
    let segment = url
        .path_segments()
        .rev()
        .find(|segment| !segment.is_empty())
        .map(
            |segment| match crate::uri::percent_decode(segment, "a catalog's name") {
                Ok(decoded) => SmolStr::new(decoded),
                Err(_) => SmolStr::new(segment),
            },
        )
        .or_else(|| url.hostname().or_else(|| url.bucket()).map(SmolStr::new));
    segment.ok_or_else(|| Error::InvalidRecord {
        path: SmolStr::new_static("$.with.name"),
        reason: format_smolstr!(
            "expected a `name` property, or a URL whose last segment names the catalog, got {url}"
        ),
    })
}

/// The absence of a catalog called `name`.
pub(crate) fn no_catalog(name: &str) -> Error {
    Error::absent("catalog", path_text(&[SmolStr::new(name)]))
}
