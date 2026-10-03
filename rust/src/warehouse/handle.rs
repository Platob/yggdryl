//! The storage handle an object resolves once and keeps beside its
//! description: what the folder, media and Iceberg implementations share.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use smol_str::{SmolStr, format_smolstr};

use super::{Properties, path_text};
use crate::fs::BoundLocation;
use crate::holder::Holder;
use crate::{Error, IOBase, IOMedia, Result, Uri, Url};

/// Where an object's storage is, as it was given: a location every backend
/// is reached by, or a binding to a foreign filesystem a caller supplied.
#[derive(Clone, Debug)]
pub(crate) enum Site {
    /// A location, opened through [`Holder::from_url`] with the object's
    /// effective properties.
    Url(Url),
    /// A caller's filesystem and a path on it, re-held as it was bound.
    Bound(BoundLocation),
    /// An object-store location opened under the session its owner signs
    /// with - a catalog service's warehouse, reached as the catalog is - in
    /// the region the owner knows it is in, the object's effective
    /// properties read over both.
    #[cfg(feature = "s3")]
    Store {
        url: Url,
        session: crate::aws::Session,
        region: String,
    },
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
            #[cfg(feature = "s3")]
            Self::Store { url, .. } => url,
        }
    }

    /// Open the handle this site names, touching nothing.
    pub(crate) fn resolve(&self, properties: &Properties) -> Result<Holder> {
        match self {
            Self::Url(url) => Holder::from_url(url, properties),
            Self::Bound(bound) => Ok(crate::fs::located(bound.clone())),
            #[cfg(feature = "s3")]
            Self::Store {
                url,
                session,
                region,
            } => {
                // A session that consults nothing outside itself seals the
                // store's own options too.
                let options = crate::s3::S3Options::default()
                    .with_environment(session.reads_environment())
                    .with_session(session.clone())
                    .with_region(region.clone())
                    .with_properties(properties.iter())?;
                crate::s3::located_with(&url.to_string(), options)
            }
        }
    }
}

impl PartialEq for Site {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Url(left), Self::Url(right)) => left == right,
            (Self::Bound(left), Self::Bound(right)) => left.same_location(right),
            #[cfg(feature = "s3")]
            (Self::Store { url: left, .. }, Self::Store { url: right, .. }) => left == right,
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
            #[cfg(feature = "s3")]
            Self::Store { url, .. } => url.hash(state),
        }
    }
}

/// The storage of a warehouse object: where it is and what it opens with,
/// resolved to the [`Holder`] it names on the first verb that needs one and
/// kept for the object's life.
///
/// Every object holds one beside its description, and an
/// `IcebergTable` held by a [`Table`](crate::Table)
/// is rooted on one - which is why the type is public: a holder in hand is
/// not clonable, and an object is. It answers every [`IOBase`] and
/// [`IOMedia`] verb as the handle it resolves to - a verb that returns a
/// `Result` carrying the resolution's failure, an accessor that cannot
/// answering the empty value - so a property stated on the object reaches
/// its storage and nothing is opened before it is needed. A clone starts
/// unresolved and rebuilds from the site under the same properties; an
/// object bound to a handle with no site cannot be rebuilt after a clone,
/// and says so by name when its handle is next needed. Equality and the
/// hash read the site, never what was resolved. The warehouse builds its
/// own; a caller with a holder in hand roots an Iceberg table on it through
/// `Handle::from(holder)`, and [`get`](Self::get) is the holder it resolves
/// to.
pub struct Handle {
    site: Option<Site>,
    /// Whether the resolved handle composes what its name declares - the
    /// content coding and the record implementation - as a table's does.
    declared: bool,
    /// The object the handle belongs to, as a refusal names it.
    what: SmolStr,
    /// What an unresolved handle opens with: the object's effective
    /// properties, as last stated.
    properties: Properties,
    held: OnceLock<Box<Holder>>,
}

impl Handle {
    /// A handle resolved from `site` when first needed, under `properties`,
    /// belonging to the object at `what`.
    pub(crate) fn at(site: Site, declared: bool, what: &[SmolStr], properties: Properties) -> Self {
        Self {
            site: Some(site),
            declared,
            what: SmolStr::from(path_text(what)),
            properties,
            held: OnceLock::new(),
        }
    }

    /// A handle already in hand, its site read off it for a clone, which
    /// opens under `properties`.
    pub(crate) fn bound(
        holder: Holder,
        declared: bool,
        what: &[SmolStr],
        properties: Properties,
    ) -> Self {
        let site = Site::of(&holder);
        let holder = if declared {
            holder.into_declared_media()
        } else {
            holder
        };
        Self {
            site,
            declared,
            what: SmolStr::from(path_text(what)),
            properties,
            held: OnceLock::from(Box::new(holder)),
        }
    }

    /// State what the handle opens with from now on; a handle already in
    /// hand was opened by whoever handed it over and is kept.
    pub(crate) fn set_properties(&mut self, properties: Properties) {
        self.properties = properties;
    }

    /// The location, when the site is one.
    pub(crate) fn url(&self) -> Option<&Url> {
        self.site.as_ref().map(Site::url)
    }

    /// The handle already resolved, without resolving one.
    pub(crate) fn held(&self) -> Option<&Holder> {
        self.held.get().map(Box::as_ref)
    }

    /// The holder the handle resolves to, resolved on the first call and
    /// kept.
    ///
    /// # Errors
    ///
    /// Returns the refusal of a handle with no site to rebuild from, and the
    /// backend's own failure to open the location.
    pub fn get(&self) -> Result<&Holder> {
        if let Some(held) = self.held.get() {
            return Ok(held.as_ref());
        }
        let resolved = self.resolve()?;
        Ok(self.held.get_or_init(|| Box::new(resolved)).as_ref())
    }

    /// The holder the handle resolves to, mutably, resolved on the first
    /// call and kept.
    ///
    /// # Errors
    ///
    /// As [`get`](Self::get).
    pub fn get_mut(&mut self) -> Result<&mut Holder> {
        if self.held.get().is_none() {
            self.held = OnceLock::from(Box::new(self.resolve()?));
        }
        match self.held.get_mut() {
            Some(held) => Ok(held.as_mut()),
            None => unreachable!("a handle was resolved just above"),
        }
    }

    /// Whether anything is at the location now.
    pub(crate) fn exists(&self) -> bool {
        self.get().is_ok_and(Holder::exists)
    }

    fn resolve(&self) -> Result<Holder> {
        let Some(site) = &self.site else {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.{}", self.what),
                reason: format_smolstr!(
                    "expected a located handle to rebuild `{}` from, got one with no URL; \
                     a clone of an object bound to an unlocated handle has nothing to open",
                    self.what
                ),
            });
        };
        let holder = site.resolve(&self.properties)?;
        Ok(if self.declared {
            holder.into_declared_media()
        } else {
            holder
        })
    }
}

impl From<Holder> for Handle {
    /// A holder in hand, its site read off it for a clone, which opens under
    /// no properties: how a caller roots an
    /// `IcebergTable` on a handle it built.
    fn from(holder: Holder) -> Self {
        let what = holder
            .url()
            .and_then(Url::file_name)
            .filter(|name| !name.is_empty())
            .map_or_else(|| SmolStr::new_static("handle"), SmolStr::new);
        Self::bound(holder, false, &[what], Properties::new())
    }
}

impl Clone for Handle {
    fn clone(&self) -> Self {
        Self {
            site: self.site.clone(),
            declared: self.declared,
            what: self.what.clone(),
            properties: self.properties.clone(),
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

impl IOBase for Handle {
    crate::__delegate_resolved_iobase!(get, get_mut, held);

    fn uri(&self) -> Option<&Uri> {
        self.get().ok()?.uri()
    }

    /// The site's location, resolving nothing.
    fn url(&self) -> Option<&Url> {
        Handle::url(self)
    }
}

impl IOMedia for Handle {
    crate::__delegate_resolved_iomedia!(get, get_mut);
}
