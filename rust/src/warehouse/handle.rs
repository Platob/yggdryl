//! The storage handle an object resolves once and keeps beside its
//! description: what the folder, media and remote implementations share.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use smol_str::format_smolstr;

use super::Properties;
use crate::fs::BoundLocation;
use crate::holder::Holder;
use crate::{Error, IOBase, Result, Url};

/// Where an object's storage is, as it was given: a location every backend
/// is reached by, or a binding to a foreign filesystem a caller supplied.
#[derive(Clone, Debug)]
pub(crate) enum Site {
    /// A location, opened through [`Holder::from_url`] with the object's
    /// effective properties.
    Url(Url),
    /// A caller's filesystem and a path on it, re-held as it was bound.
    Bound(BoundLocation),
}

impl Site {
    /// The site a handle was built from, when it has one to rebuild from: a
    /// binding, or a location of a scheme [`Holder::from_url`] holds - an
    /// in-memory buffer spells a `mem:` identity nothing opens, so it has
    /// none.
    pub(crate) fn of(holder: &Holder) -> Option<Self> {
        if let Some(bound) = holder.bound_location() {
            return Some(Self::Bound(bound.clone()));
        }
        let url = holder.url()?;
        let scheme = url.scheme();
        (url.is_local() || scheme.is_object_store() || scheme.is_http())
            .then(|| Self::Url(url.clone()))
    }

    /// The location, as a URL: a bound site's diagnostic one.
    pub(crate) fn url(&self) -> &Url {
        match self {
            Self::Url(url) => url,
            Self::Bound(bound) => bound.diagnostic_url(),
        }
    }

    /// Open the handle this site names, touching nothing.
    pub(crate) fn resolve(&self, properties: &Properties) -> Result<Holder> {
        match self {
            Self::Url(url) => Holder::from_url(url, properties),
            Self::Bound(bound) => Ok(crate::fs::located(bound.clone())),
        }
    }
}

impl PartialEq for Site {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Url(left), Self::Url(right)) => left == right,
            (Self::Bound(left), Self::Bound(right)) => left.same_location(right),
            _ => false,
        }
    }
}

impl Eq for Site {}

impl Hash for Site {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Url(url) => url.hash(state),
            Self::Bound(bound) => {
                bound.diagnostic_url().hash(state);
                bound.path().hash(state);
            }
        }
    }
}

/// A handle resolved on first use from its site, kept for the object's life.
///
/// A clone starts unresolved and rebuilds from the site; an object bound to
/// a handle with no site cannot be rebuilt after a clone, and says so by name
/// when its handle is next needed.
pub(crate) struct Handle {
    site: Option<Site>,
    /// Whether the resolved handle composes what its name declares - the
    /// content coding and the record implementation - as a table's does.
    declared: bool,
    held: OnceLock<Box<Holder>>,
}

impl Handle {
    /// A handle resolved from `site` when first needed.
    pub(crate) const fn at(site: Site, declared: bool) -> Self {
        Self {
            site: Some(site),
            declared,
            held: OnceLock::new(),
        }
    }

    /// A handle already in hand, its site read off it for a clone.
    pub(crate) fn bound(holder: Holder, declared: bool) -> Self {
        let site = Site::of(&holder);
        let holder = if declared {
            holder.into_declared_media()
        } else {
            holder
        };
        Self {
            site,
            declared,
            held: OnceLock::from(Box::new(holder)),
        }
    }

    /// The location, when the site is one.
    pub(crate) fn url(&self) -> Option<&Url> {
        self.site.as_ref().map(Site::url)
    }

    /// The handle already resolved, without resolving one.
    pub(crate) fn opened(&self) -> Option<&Holder> {
        self.held.get().map(Box::as_ref)
    }

    /// The handle, resolved on the first call with `properties`.
    pub(crate) fn get(&self, properties: &Properties, what: &str) -> Result<&Holder> {
        if let Some(held) = self.held.get() {
            return Ok(held.as_ref());
        }
        let resolved = self.resolve(properties, what)?;
        Ok(self.held.get_or_init(|| Box::new(resolved)).as_ref())
    }

    /// The handle, mutably, resolved on the first call with `properties`.
    pub(crate) fn get_mut(&mut self, properties: &Properties, what: &str) -> Result<&mut Holder> {
        if self.held.get().is_none() {
            self.held = OnceLock::from(Box::new(self.resolve(properties, what)?));
        }
        match self.held.get_mut() {
            Some(held) => Ok(held.as_mut()),
            None => unreachable!("a handle was resolved just above"),
        }
    }

    fn resolve(&self, properties: &Properties, what: &str) -> Result<Holder> {
        let Some(site) = &self.site else {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.{what}"),
                reason: format_smolstr!(
                    "expected a located handle to rebuild `{what}` from, got one with no URL; \
                     a clone of an object bound to an unlocated handle has nothing to open"
                ),
            });
        };
        let holder = site.resolve(properties)?;
        Ok(if self.declared {
            holder.into_declared_media()
        } else {
            holder
        })
    }
}

impl Clone for Handle {
    fn clone(&self) -> Self {
        Self {
            site: self.site.clone(),
            declared: self.declared,
            held: OnceLock::new(),
        }
    }
}

impl fmt::Debug for Handle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Handle")
            .field("site", &self.site)
            .field("resolved", &self.held.get().is_some())
            .finish()
    }
}

impl PartialEq for Handle {
    fn eq(&self, other: &Self) -> bool {
        self.site == other.site && self.declared == other.declared
    }
}

impl Eq for Handle {}

impl Hash for Handle {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.site.hash(state);
        self.declared.hash(state);
    }
}
