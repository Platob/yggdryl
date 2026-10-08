//! The one claim-once register every extension point lands on: a market
//! kind, a medium, a logical name claims its key once, and a second claim is
//! refused naming the first claimant and the key, so two crates can never
//! answer one name differently. The precedent is the user-function
//! registry in `expression/user.rs`; this is that shape, keyed.

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::RwLock;

use smol_str::format_smolstr;

use crate::{Error, Result};

/// One claim: what was registered and the crate that registered it.
#[derive(Clone, Debug)]
pub struct Claim<V> {
    /// The registered value.
    pub value: V,
    /// The crate that claimed the key, named by a refusal of a second claim.
    pub by: &'static str,
}

/// A keyed claim-once register: a key is claimed exactly once for the life
/// of the process, read any number of times, and never replaced.
///
/// `what` names the register in a refusal (`market kind`, `logical name`).
/// Construction is `const`, so a register is a `static` with no `OnceLock`
/// around it.
pub struct Register<K, V> {
    what: &'static str,
    claims: RwLock<BTreeMap<K, Claim<V>>>,
}

impl<K: Ord + fmt::Display, V: Clone> Register<K, V> {
    /// An empty register of `what`.
    #[must_use]
    pub const fn new(what: &'static str) -> Self {
        Self {
            what,
            claims: RwLock::new(BTreeMap::new()),
        }
    }

    /// Claim `key` for `by`, holding `value` under it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] naming the first claimant where the key
    /// is claimed already - by another crate, or by the same crate twice.
    pub fn claim(&self, key: K, value: V, by: &'static str) -> Result<()> {
        let mut claims = self
            .claims
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(held) = claims.get(&key) {
            return Err(Error::Conflict {
                expected: self.what,
                actual: held.by,
                path: format_smolstr!("{key}"),
            });
        }
        claims.insert(key, Claim { value, by });
        Ok(())
    }

    /// The value claimed under `key`, or `None` where no claim holds it.
    pub fn get<Q>(&self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.claims
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .map(|claim| claim.value.clone())
    }

    /// The crate that claimed `key`, or `None` where no claim holds it.
    pub fn claimant<Q>(&self, key: &Q) -> Option<&'static str>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.claims
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .map(|claim| claim.by)
    }

    /// Every claimed value, in key order.
    pub fn values(&self) -> Vec<V> {
        self.claims
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .map(|claim| claim.value.clone())
            .collect()
    }

    /// How many keys are claimed.
    pub fn len(&self) -> usize {
        self.claims
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Whether no key is claimed.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<K: fmt::Debug, V: fmt::Debug> fmt::Debug for Register<K, V> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let claims = self
            .claims
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        formatter
            .debug_struct("Register")
            .field("what", &self.what)
            .field("claims", &*claims)
            .finish()
    }
}
