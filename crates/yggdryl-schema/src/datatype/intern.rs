//! A process-wide **intern pool** for [`DataType`], so attaching a common type
//! (`int32`, `utf8`, a frequently-used struct, …) to many values reuses one shared
//! `Arc<DataType>` rather than re-allocating it.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use super::DataType;

/// The global pool. Keyed by the structural [`DataType`]; the value is the canonical
/// shared `Arc`. Grows monotonically — only ever holds the distinct types seen.
fn pool() -> &'static RwLock<HashMap<DataType, Arc<DataType>>> {
    static POOL: OnceLock<RwLock<HashMap<DataType, Arc<DataType>>>> = OnceLock::new();
    POOL.get_or_init(|| RwLock::new(HashMap::new()))
}

impl DataType {
    /// Returns a shared, **interned** `Arc<DataType>` for this type: the first call for a
    /// given type allocates and caches it; every later call for an equal type returns a
    /// cheap `Arc` clone of the same allocation. Attaching the type to a value or field
    /// is then a refcount bump, not a deep clone of (possibly nested) type structure.
    ///
    /// ```
    /// use yggdryl_schema::DataType;
    /// let a = DataType::int(32, true).interned();
    /// let b = DataType::int(32, true).interned();
    /// assert!(std::sync::Arc::ptr_eq(&a, &b)); // same shared allocation
    /// assert_eq!(*a, DataType::int(32, true));
    /// ```
    pub fn interned(&self) -> Arc<DataType> {
        if let Some(arc) = pool().read().expect("intern pool not poisoned").get(self) {
            return Arc::clone(arc);
        }
        let mut pool = pool().write().expect("intern pool not poisoned");
        // Re-check: another writer may have inserted between the read and the write lock.
        if let Some(arc) = pool.get(self) {
            return Arc::clone(arc);
        }
        let arc = Arc::new(self.clone());
        pool.insert(self.clone(), Arc::clone(&arc));
        arc
    }

    /// Clears the intern pool. For tests / long-lived processes that want to reclaim the
    /// cached types; correctness never depends on the pool's contents.
    pub fn clear_intern_pool() {
        pool().write().expect("intern pool not poisoned").clear();
    }
}
