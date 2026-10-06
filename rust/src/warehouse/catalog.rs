//! A catalog: the first namespace layer, what a warehouse registers by name,
//! and the enum that says which implementation answers it.

use std::fmt;

use smol_str::{SmolStr, format_smolstr};

use super::namespace::{Namespaces, Tables, container_object_io, resolve_under};
use super::object::path_text;
use super::{
    FolderCatalog, IntoObjectPath, MemoryCatalog, Namespace, NamespaceValue, Object, ObjectValue,
    Objects, Properties, Table,
};
use crate::media::RecordOptions;
use crate::{Arn, Error, Field, IOBase, IOKind, IOMedia, Result, Scheme, Uri, Url};

/// The first namespace layer: what a warehouse registers by name.
pub trait CatalogValue: NamespaceValue {
    /// How many namespace levels may sit under it: `Some(1)` for a folder
    /// catalog read as `catalog.schema.table`, `None` where namespaces nest
    /// to any depth.
    fn namespace_levels(&self) -> Option<usize>;
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
    /// An Iceberg warehouse folder, read as namespaces nested to any depth
    /// and tables laid out as the format lays them out.
    #[cfg(feature = "iceberg")]
    Iceberg(Box<crate::iceberg::IcebergCatalog>),
    /// An Amazon S3 Tables table bucket: namespaces one level below it,
    /// Iceberg tables below them, every commit published through the
    /// control plane.
    #[cfg(feature = "s3tables")]
    S3Tables(Box<crate::s3tables::S3TablesCatalog>),
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
    /// `hadoop`, an Iceberg warehouse folder, PyIceberg's spelling - and
    /// otherwise the scheme does: an `s3tables://<bucket>` location is that
    /// bucket's `S3TablesCatalog` under the `s3tables` feature, and every
    /// location a byte backend holds is a folder catalog over the container
    /// it names. The catalog is called what the `name` property says, else
    /// the location's last segment, or its bucket. A property this door does
    /// not read travels on to every handle under the catalog.
    ///
    /// # Errors
    ///
    /// Returns the locator's refusal of an identifier that names no
    /// location, [`Error::InvalidRecord`] at `$.with.type` naming a type
    /// this build does not answer, and at `$.with.name` when no name can be
    /// read.
    pub fn from_url(location: impl AsRef<Uri>, properties: &Properties) -> Result<Self> {
        /// The types this build answers, as a refusal lists them.
        #[cfg(feature = "iceberg")]
        const TYPES: &str = "`memory`, `folder` or `hadoop`";
        #[cfg(not(feature = "iceberg"))]
        const TYPES: &str = "`memory` or `folder`";
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
        let not_built = |other: &str| Error::InvalidRecord {
            path: SmolStr::new_static("$.with.type"),
            reason: format_smolstr!(
                "expected {TYPES}, got `{other}`; this build has no catalog of that type"
            ),
        };
        match kind {
            None | Some("folder" | "memory") => {}
            #[cfg(feature = "iceberg")]
            Some("hadoop") => {}
            #[cfg(feature = "iceberg")]
            Some(other @ ("rest" | "xmla")) => return Err(not_built(other)),
            #[cfg(not(feature = "iceberg"))]
            Some(other @ ("hadoop" | "rest" | "xmla")) => return Err(not_built(other)),
            Some(other) => {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$.with.type"),
                    reason: format_smolstr!("expected {TYPES}, got {other:?}"),
                });
            }
        }
        #[cfg(not(feature = "s3tables"))]
        if kind.is_none() && url.scheme().is_s3_tables() {
            return Err(Error::unsupported(
                "holding an S3 Tables catalog in this build",
                url.scheme().as_str(),
            ));
        }
        let name = match properties.get("name") {
            Some(name) if !name.is_empty() => SmolStr::new(name),
            _ => catalog_name(url)?,
        };
        #[cfg(feature = "s3tables")]
        if kind.is_none() && url.scheme().is_s3_tables() {
            return Ok(Self::S3Tables(Box::new(
                crate::s3tables::S3TablesCatalog::from_location(name, url, arn, properties)?,
            )));
        }
        if kind == Some("memory") {
            return Ok(Self::Memory(
                MemoryCatalog::new(name).with_properties(properties.clone()),
            ));
        }
        #[cfg(feature = "iceberg")]
        if kind == Some("hadoop") {
            return Ok(Self::Iceberg(Box::new(
                crate::iceberg::IcebergCatalog::new(name, url.clone())?
                    .with_properties(properties.clone()),
            )));
        }
        Ok(Self::Folder(Box::new(
            FolderCatalog::new(name, url.clone())?.with_properties(properties.clone()),
        )))
    }

    /// The implementation's own name: `MemoryCatalog`, `FolderCatalog`,
    /// `IcebergCatalog`, `S3TablesCatalog`.
    pub(crate) const fn implementation_name(&self) -> &'static str {
        match self {
            Self::Memory(_) => "MemoryCatalog",
            Self::Folder(_) => "FolderCatalog",
            #[cfg(feature = "iceberg")]
            Self::Iceberg(_) => "IcebergCatalog",
            #[cfg(feature = "s3tables")]
            Self::S3Tables(_) => "S3TablesCatalog",
        }
    }

    /// Borrow the implementation through the contract every catalog answers.
    pub fn as_catalog(&self) -> &dyn CatalogValue {
        match self {
            Self::Memory(catalog) => catalog,
            Self::Folder(catalog) => catalog.as_ref(),
            #[cfg(feature = "iceberg")]
            Self::Iceberg(catalog) => catalog.as_ref(),
            #[cfg(feature = "s3tables")]
            Self::S3Tables(catalog) => catalog.as_ref(),
        }
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

#[cfg(feature = "iceberg")]
impl From<crate::iceberg::IcebergCatalog> for Catalog {
    fn from(catalog: crate::iceberg::IcebergCatalog) -> Self {
        Self::Iceberg(Box::new(catalog))
    }
}

#[cfg(feature = "s3tables")]
impl From<crate::s3tables::S3TablesCatalog> for Catalog {
    fn from(catalog: crate::s3tables::S3TablesCatalog) -> Self {
        Self::S3Tables(Box::new(catalog))
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
