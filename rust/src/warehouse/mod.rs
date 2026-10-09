//! One abstraction for every place that answers "which tables are there, and
//! how do I read one".
//!
//! Four traits say what every object a path reaches answers:
//! [`ObjectValue`], [`NamespaceValue`], [`CatalogValue`] and [`TableValue`].
//! Four enums say which implementation answers it - [`Object`], [`Catalog`],
//! [`Namespace`] and [`Table`] - the way [`Holder`](crate::holder::Holder)
//! does for storage and [`Media`](crate::media::Media) for encodings. A
//! catalog is the first namespace layer; its name is what a [`Warehouse`]
//! registers, and [`SystemWarehouse`] is the process's one registry, which a
//! plan's `from catalog.namespace.table` resolves against.
//!
//! An object is a description - its path, what it states, and where its
//! storage is - and never a view borrowed from its parent, so any object sits
//! in an enum, is registered, and crosses a binding boundary. Its storage
//! handle is resolved once, on first use, and a clone starts unresolved.
//!
//! Every object carries [`Properties`]: its parent's effective properties,
//! then what its store keeps for it, then what was stated for it, a later
//! entry replacing an earlier one by name. They propagate - every child an
//! object answers carries them, and every handle under it opens with them -
//! so credentials stated once on a catalog reach every table's storage.
//!
//! The generic implementations live here: [`MemoryCatalog`] and
//! [`MemoryNamespace`] keep registered objects in order, [`FolderCatalog`]
//! and [`FolderNamespace`] read a container as namespaces and tables, and
//! [`MediaTable`] is a table over any location a record medium reads. Every
//! other implementation lives in the root folder of its own name and is
//! held as the `Registered` variant of its enum, through
//! [`RegisteredCatalog`], [`RegisteredNamespace`] or [`RegisteredTable`];
//! [`Catalog::from_url`] reaches one through the [`CatalogFactory`] claimed
//! for its `type` word or its scheme ([`claim_factory`], [`factories`]).
//!
//! ```
//! use yggdryl::holder::Holder;
//! use yggdryl::{FolderCatalog, IOMedia, MediaTable, ObjectValue, TableValue, Warehouse};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let root = std::env::temp_dir().join(format!("yggdryl-warehouse-doc-{}", std::process::id()));
//! std::fs::create_dir_all(root.join("eu"))?;
//! std::fs::write(root.join("eu/trades.csv"), "symbol,price\nAAPL,187.5\n")?;
//!
//! // A folder is a catalog: its folders are namespaces, its files tables.
//! let mut warehouse = Warehouse::new();
//! warehouse.register(FolderCatalog::bound("market", Holder::folder(&root)?))?;
//! let trades = warehouse.table("market.eu.trades")?;
//! assert_eq!(trades.storage(), "text/csv");
//! assert_eq!(trades.field()?.field_len(), 2);
//!
//! // A table at any location registers at a path of its own.
//! let csv = yggdryl::Url::from_path(root.join("eu/trades.csv"))?;
//! warehouse.register(MediaTable::new("lake.raw.trades", csv)?)?;
//! assert_eq!(warehouse.get("lake.raw")?.kind(), yggdryl::IOKind::Namespace);
//! assert_eq!(warehouse.table("lake.raw.trades")?.row_size()?, 1);
//! # std::fs::remove_dir_all(&root)?;
//! # Ok(())
//! # }
//! ```

mod catalog;
mod folder;
mod handle;
mod media;
mod memory;
mod namespace;
mod object;
mod properties;
mod system;
mod table;

pub use catalog::{
    Catalog, CatalogFactory, CatalogValue, RegisteredCatalog, claim_factory, factories,
};
pub use folder::{FolderCatalog, FolderLayout, FolderNamespace};
pub use handle::Handle;
pub use media::MediaTable;
pub use memory::{MemoryCatalog, MemoryNamespace};
pub use namespace::{Names, Namespace, NamespaceValue, Namespaces, RegisteredNamespace, Tables};
pub use object::{IntoObjectPath, Object, ObjectValue, Objects};
pub use properties::Properties;
pub use system::SystemWarehouse;
pub use table::{RegisteredTable, Table, TableValue};

pub(crate) use catalog::no_catalog;
#[cfg(feature = "iceberg")]
pub(crate) use folder::{entry_name, table_layout};
#[cfg(feature = "iceberg")]
pub(crate) use handle::Site;
pub(crate) use namespace::no_table;
pub(crate) use object::path_text;
#[cfg(feature = "iceberg")]
pub(crate) use object::{extended, implementation_name};

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result, Url};

/// The registry of catalogs a path resolves against.
///
/// Catalogs register by name, in order; a namespace or a table registers at
/// its path, the memory catalogs and namespaces along it created as needed.
/// Registration only extends memory catalogs and namespaces: a folder
/// catalog lists its own store, and registering under one is refused by
/// name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Warehouse {
    catalogs: Vec<Catalog>,
}

impl Warehouse {
    /// An empty warehouse.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            catalogs: Vec::new(),
        }
    }

    /// Register an object: a catalog by its name; a namespace or a table at
    /// its path, creating memory catalogs and namespaces along the path.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] when the name is taken at that level, and
    /// [`Error::Unsupported`] when the path runs under a catalog or a
    /// namespace that lists its own store.
    pub fn register(&mut self, object: impl Into<Object>) -> Result<()> {
        let object = object.into();
        match object {
            Object::Catalog(catalog) => {
                if let Some(existing) = self
                    .catalogs
                    .iter()
                    .find(|held| held.name() == catalog.name())
                {
                    return Err(Error::conflict(
                        "catalog",
                        existing.kind().as_str(),
                        path_text(catalog.path()),
                    ));
                }
                self.catalogs.push(catalog);
                Ok(())
            }
            object => match self.parent_of(object.path(), true)? {
                Parent::Catalog(catalog) => catalog.register(object),
                Parent::Namespace(namespace) => namespace.register(object),
            },
        }
    }

    /// Register an object, replacing the one of its name at that level and
    /// answering what was replaced.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] when the path runs under a catalog or
    /// a namespace that lists its own store.
    pub fn replace(&mut self, object: impl Into<Object>) -> Result<Option<Object>> {
        let object = object.into();
        match object {
            Object::Catalog(catalog) => Ok(self.replace_catalog(catalog).map(Object::Catalog)),
            object => match self.parent_of(object.path(), true)? {
                Parent::Catalog(catalog) => catalog.replace(object),
                Parent::Namespace(namespace) => namespace.replace(object),
            },
        }
    }

    /// Register a catalog, replacing the one of its name and answering it:
    /// [`Self::replace`] for a catalog, which registers by its name alone
    /// and so is never refused.
    pub fn replace_catalog(&mut self, catalog: Catalog) -> Option<Catalog> {
        match self
            .catalogs
            .iter()
            .position(|held| held.name() == catalog.name())
        {
            Some(at) => Some(std::mem::replace(&mut self.catalogs[at], catalog)),
            None => {
                self.catalogs.push(catalog);
                None
            }
        }
    }

    /// Remove the object registered at `path`, answering it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] when nothing is registered there, and
    /// [`Error::Unsupported`] when the path runs under a catalog or a
    /// namespace that lists its own store.
    pub fn unregister(&mut self, path: impl IntoObjectPath) -> Result<Object> {
        let path = path.into_object_path()?;
        let Some((name, parents)) = path.split_last() else {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.path"),
                reason: SmolStr::new_static(
                    "expected the path of a registered object, got the empty path",
                ),
            });
        };
        if parents.is_empty() {
            return match self.catalogs.iter().position(|held| held.name() == name) {
                Some(at) => Ok(Object::Catalog(self.catalogs.remove(at))),
                None => Err(no_catalog(name)),
            };
        }
        match self.parent_of(&path, false)? {
            Parent::Catalog(catalog) => catalog.unregister(name),
            Parent::Namespace(namespace) => namespace.unregister(name),
        }
    }

    /// The memory catalog or namespace an object at `path` registers under,
    /// built along the way when `create`.
    fn parent_of(&mut self, path: &[SmolStr], create: bool) -> Result<Parent<'_>> {
        let Some((_, parents)) = path.split_last() else {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.path"),
                reason: SmolStr::new_static("expected a path of at least one part, got none"),
            });
        };
        let Some((catalog_name, namespaces)) = parents.split_first() else {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.{}", path_text(path)),
                reason: SmolStr::new_static(
                    "expected a namespace or a table path under a catalog name, got one part",
                ),
            });
        };
        let at = match self
            .catalogs
            .iter()
            .position(|held| held.name() == catalog_name.as_str())
        {
            Some(at) => at,
            None if create => {
                self.catalogs
                    .push(Catalog::Memory(MemoryCatalog::new(catalog_name.clone())));
                self.catalogs.len() - 1
            }
            None => return Err(no_catalog(catalog_name)),
        };
        let catalog = match &mut self.catalogs[at] {
            Catalog::Memory(catalog) => catalog,
            other => return Err(not_registrable(other.implementation_name(), other.path())),
        };
        if namespaces.is_empty() {
            return Ok(Parent::Catalog(catalog));
        }
        let mut current: &mut MemoryNamespace = {
            let name = &namespaces[0];
            if catalog.child_mut(name).is_none() {
                if !create {
                    return Err(Error::absent("namespace", path_text(&path[..=1])));
                }
                catalog.register(Object::Namespace(Namespace::Memory(MemoryNamespace::new(
                    path[..=1].to_vec(),
                )?)))?;
            }
            match catalog.child_mut(name) {
                Some(Object::Namespace(Namespace::Memory(namespace))) => namespace,
                Some(other) => {
                    return Err(not_registrable(other.implementation_name(), other.path()));
                }
                None => unreachable!("the namespace was registered just above"),
            }
        };
        for (depth, name) in (2..).zip(&namespaces[1..]) {
            if current.child_mut(name).is_none() {
                if !create {
                    return Err(Error::absent("namespace", path_text(&path[..=depth])));
                }
                current.register(Object::Namespace(Namespace::Memory(MemoryNamespace::new(
                    path[..=depth].to_vec(),
                )?)))?;
            }
            current = match current.child_mut(name) {
                Some(Object::Namespace(Namespace::Memory(namespace))) => namespace,
                Some(other) => {
                    return Err(not_registrable(other.implementation_name(), other.path()));
                }
                None => unreachable!("the namespace was registered just above"),
            };
        }
        Ok(Parent::Namespace(current))
    }

    /// The catalogs, in registration order.
    #[must_use]
    pub fn catalogs(&self) -> &[Catalog] {
        &self.catalogs
    }

    /// The catalog called `name`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] naming it when none is registered.
    pub fn catalog(&self, name: &str) -> Result<&Catalog> {
        self.catalogs
            .iter()
            .find(|catalog| catalog.name() == name)
            .ok_or_else(|| no_catalog(name))
    }

    /// The object a path names: a catalog by its one part, else the object
    /// the catalog's descent reaches.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] naming the catalog or the path, and what the
    /// descent answers otherwise.
    pub fn get(&self, path: impl IntoObjectPath) -> Result<Object> {
        let path = path.into_object_path()?;
        let Some((name, rest)) = path.split_first() else {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.path"),
                reason: SmolStr::new_static(
                    "expected a path naming a catalog first, got the empty path",
                ),
            });
        };
        let catalog = self.catalog(name)?;
        if rest.is_empty() {
            return Ok(Object::Catalog(catalog.clone()));
        }
        catalog.resolve(rest)
    }

    /// The table a path names.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] naming the path when nothing - or a
    /// namespace or a catalog - is there.
    pub fn table(&self, path: impl IntoObjectPath) -> Result<Table> {
        self.get(path)?.into_table()
    }

    /// The namespace a path names.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] naming the path when nothing - or a table
    /// or a catalog - is there.
    pub fn namespace(&self, path: impl IntoObjectPath) -> Result<Namespace> {
        self.get(path)?.into_namespace()
    }

    /// The effective properties of the deepest registered object whose URL
    /// holds `url`, on a path boundary: the rest of `url` after the object's
    /// is empty or starts with `/`, `?` or `#`, so `file:///a/lake` holds
    /// `file:///a/lake/t.parquet` and not `file:///a/lakehouse`.
    ///
    /// An object no registered URL holds has none, which is the empty bag.
    #[must_use]
    pub fn properties_for(&self, url: &Url) -> Properties {
        let text = url.to_string();
        let mut deepest: Option<(usize, Properties)> = None;
        for catalog in &self.catalogs {
            let mut stack: Vec<(&dyn ObjectValue, &[Object])> = vec![(
                catalog,
                match catalog {
                    Catalog::Memory(memory) => memory.registered(),
                    _ => &[],
                },
            )];
            while let Some((object, registered)) = stack.pop() {
                if let Some(held) = object.url()
                    && let Some(depth) = holds(&held.to_string(), &text)
                    && deepest.as_ref().is_none_or(|(best, _)| depth > *best)
                    && let Ok(properties) = object.properties()
                {
                    deepest = Some((depth, properties));
                }
                for child in registered {
                    stack.push((
                        child,
                        match child {
                            Object::Namespace(Namespace::Memory(memory)) => memory.registered(),
                            _ => &[],
                        },
                    ));
                }
            }
        }
        deepest
            .map(|(_, properties)| properties)
            .unwrap_or_default()
    }
}

/// Whether `prefix` holds `url` on a path boundary, and how long the match
/// is: the one containment rule every location check reads.
pub(crate) fn holds(prefix: &str, url: &str) -> Option<usize> {
    let prefix = prefix.trim_end_matches('/');
    let rest = url.strip_prefix(prefix)?;
    (rest.is_empty() || rest.starts_with(['/', '?', '#'])).then_some(prefix.len())
}

/// The refusal of registering under an object that lists its own store.
fn not_registrable(implementation: &'static str, path: &[SmolStr]) -> Error {
    Error::unsupported(
        "registering under an object that lists its own store",
        format_args!("{implementation} `{}`", path_text(path)),
    )
}

/// The memory object a registration lands in.
enum Parent<'a> {
    Catalog(&'a mut MemoryCatalog),
    Namespace(&'a mut MemoryNamespace),
}

pub use table::WarehouseTableSerie;
