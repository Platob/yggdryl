//! [`SystemWarehouse`]: the process's one [`Warehouse`].

use std::sync::{LazyLock, PoisonError, RwLock};

use super::{
    Catalog, FolderNamespace, IntoObjectPath, MemoryCatalog, Namespace, Object, Properties, Table,
    Warehouse,
};
use crate::holder::Holder;
use crate::local::LocalFolder;
use crate::{Result, Url};

/// The process's one warehouse, which a plan's `from catalog.namespace.table`
/// resolves against when it is given no other.
///
/// It starts with the memory catalog `local`, holding the folder namespaces
/// `temporary`, `home` and `config` over the platform's temporary directory,
/// the user's home and its `.config`; a root that cannot be resolved - no
/// `HOME` - is left out. Every door takes the lock for its own call and
/// nothing re-enters it; a poisoned lock is recovered, never a panic.
///
/// ```
/// use yggdryl::{IOKind, ObjectValue, SystemWarehouse};
///
/// # fn main() -> yggdryl::Result<()> {
/// let local = SystemWarehouse::catalog("local")?;
/// assert_eq!(local.kind(), IOKind::Catalog);
/// assert_eq!(SystemWarehouse::get("local.temporary")?.kind(), IOKind::Namespace);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct SystemWarehouse;

static SYSTEM: LazyLock<RwLock<Warehouse>> = LazyLock::new(|| RwLock::new(initial()));

/// The warehouse the process starts with.
fn initial() -> Warehouse {
    let mut local = MemoryCatalog::new("local");
    for (name, root) in [
        ("temporary", LocalFolder::temporary()),
        ("home", LocalFolder::home()),
        ("config", LocalFolder::config()),
    ] {
        let Ok(root) = root else {
            continue;
        };
        if let Ok(namespace) = FolderNamespace::bound(["local", name], Holder::LocalFolder(root))
            && let Ok(extended) = local
                .clone()
                .with_object(Namespace::Folder(Box::new(namespace)))
        {
            local = extended;
        }
    }
    let mut warehouse = Warehouse::new();
    // The only catalog registered so far carries a name nothing else has.
    let _ = warehouse.register(Catalog::Memory(local));
    warehouse
}

impl SystemWarehouse {
    /// Read the warehouse under its lock.
    pub fn with<R>(read: impl FnOnce(&Warehouse) -> R) -> R {
        let guard = SYSTEM.read().unwrap_or_else(PoisonError::into_inner);
        read(&guard)
    }

    /// Change the warehouse under its lock.
    pub fn with_mut<R>(change: impl FnOnce(&mut Warehouse) -> R) -> R {
        let mut guard = SYSTEM.write().unwrap_or_else(PoisonError::into_inner);
        change(&mut guard)
    }

    /// [`Warehouse::register`] on the system warehouse.
    ///
    /// # Errors
    ///
    /// Returns what [`Warehouse::register`] returns.
    pub fn register(object: impl Into<Object>) -> Result<()> {
        let object = object.into();
        Self::with_mut(|warehouse| warehouse.register(object))
    }

    /// [`Warehouse::replace`] on the system warehouse.
    ///
    /// # Errors
    ///
    /// Returns what [`Warehouse::replace`] returns.
    pub fn replace(object: impl Into<Object>) -> Result<Option<Object>> {
        let object = object.into();
        Self::with_mut(|warehouse| warehouse.replace(object))
    }

    /// [`Warehouse::unregister`] on the system warehouse.
    ///
    /// # Errors
    ///
    /// Returns what [`Warehouse::unregister`] returns.
    pub fn unregister(path: impl IntoObjectPath) -> Result<Object> {
        let path = path.into_object_path()?;
        Self::with_mut(|warehouse| warehouse.unregister(path))
    }

    /// The registered catalogs, in order.
    #[must_use]
    pub fn catalogs() -> Vec<Catalog> {
        Self::with(|warehouse| warehouse.catalogs().to_vec())
    }

    /// [`Warehouse::catalog`] on the system warehouse.
    ///
    /// # Errors
    ///
    /// Returns what [`Warehouse::catalog`] returns.
    pub fn catalog(name: &str) -> Result<Catalog> {
        Self::with(|warehouse| warehouse.catalog(name).cloned())
    }

    /// [`Warehouse::get`] on the system warehouse.
    ///
    /// # Errors
    ///
    /// Returns what [`Warehouse::get`] returns.
    pub fn get(path: impl IntoObjectPath) -> Result<Object> {
        let path = path.into_object_path()?;
        Self::with(|warehouse| warehouse.get(path))
    }

    /// [`Warehouse::table`] on the system warehouse.
    ///
    /// # Errors
    ///
    /// Returns what [`Warehouse::table`] returns.
    pub fn table(path: impl IntoObjectPath) -> Result<Table> {
        let path = path.into_object_path()?;
        Self::with(|warehouse| warehouse.table(path))
    }

    /// [`Warehouse::namespace`] on the system warehouse.
    ///
    /// # Errors
    ///
    /// Returns what [`Warehouse::namespace`] returns.
    pub fn namespace(path: impl IntoObjectPath) -> Result<Namespace> {
        let path = path.into_object_path()?;
        Self::with(|warehouse| warehouse.namespace(path))
    }

    /// [`Warehouse::properties_for`] on the system warehouse.
    #[must_use]
    pub fn properties_for(url: &Url) -> Properties {
        Self::with(|warehouse| warehouse.properties_for(url))
    }
}
