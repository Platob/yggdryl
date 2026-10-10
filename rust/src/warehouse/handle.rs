//! The storage handle an object resolves once and keeps beside its
//! description: what the folder, media and Iceberg implementations share.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use smol_str::{SmolStr, format_smolstr};

use super::{Properties, path_text};
use crate::fs::BoundLocation;
use crate::holder::{Holder, backend_for};
use crate::{Error, IOBase, IOMedia, Result, Uri, Url};

/// What opens a [`Site::Opened`] location: called with the object's
/// effective properties on every resolution, answering the handle and
/// sending nothing.
pub(crate) type Opener = Arc<dyn Fn(&Properties) -> Result<Holder> + Send + Sync>;

/// Where an object's storage is, as it was given: a location every backend
/// is reached by, a native handle a caller built over a store, a binding to
/// a foreign filesystem a caller supplied, or a location its owner opens.
#[derive(Clone)]
pub(crate) enum Site {
    /// A location, opened through [`Holder::from_url`] with the object's
    /// effective properties: a local one, or one named rather than handed
    /// over as a handle.
    Url(Url),
    /// A native role of a claimed backend or of HTTP a caller handed over,
    /// held as its own reopen ([`Holder::from_handle`]) and opened again
    /// from it by every resolution: the endpoint, the credentials, the
    /// session and the pool the caller built it with reach every clone,
    /// which a location would open under default options instead.
    Native { url: Url, held: Arc<Holder> },
    /// A caller's filesystem and a path on it, re-held as it was bound.
    Bound(BoundLocation),
    /// A location its owner opens - a catalog service's warehouse, reached
    /// as the catalog is - through the opener it built: the opener holds
    /// what the owner reaches the store under (the session it signs with,
    /// the region it knows the store is in, the store's own knobs it was
    /// given: where the store is, how it is addressed, a key pair stated for
    /// it) and reads the object's effective properties over it, so one
    /// stated on the object wins. What the opener holds is the site's and
    /// never the object's: nothing lists or prints it, and a bag stated on
    /// the object later leaves it in place.
    ///
    /// A catalog service's implementation builds it - the core's own under
    /// `s3tables` - and a build with no such implementation never does.
    #[cfg_attr(not(feature = "s3tables"), allow(dead_code))]
    Opened { url: Url, open: Opener },
}

impl fmt::Debug for Site {
    /// The location alone: what an opener holds is printed by nothing.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Url(url) => formatter.debug_tuple("Url").field(url).finish(),
            Self::Native { url, .. } => formatter.debug_tuple("Native").field(url).finish(),
            Self::Bound(bound) => formatter.debug_tuple("Bound").field(bound).finish(),
            Self::Opened { url, .. } => formatter
                .debug_struct("Opened")
                .field("url", url)
                .finish_non_exhaustive(),
        }
    }
}

impl Site {
    /// The site a handle was built from, when it has one to rebuild from: a
    /// binding; a native role of a claimed backend ([`backend_for`]) or of
    /// HTTP, held as its own reopen on the client it was built with - one
    /// HTTP answer, which has no client to reopen on, is held by its
    /// location alone; or a local location. An in-memory buffer spells a
    /// `mem:` identity nothing opens, so it has none. Building a site sends
    /// no request.
    pub(crate) fn of(holder: &Holder) -> Option<Self> {
        if let Some(bound) = holder.bound_location() {
            return Some(Self::Bound(bound.clone()));
        }
        let url = holder.url()?;
        if url.is_local() {
            return Some(Self::Url(url.clone()));
        }
        let scheme = url.scheme();
        if !(backend_for(scheme).is_some() || scheme.is_http()) {
            return None;
        }
        Some(match Holder::from_handle(holder) {
            Ok(held) => Self::Native {
                url: url.clone(),
                held: Arc::new(held),
            },
            Err(_) => Self::Url(url.clone()),
        })
    }

    /// The location, as a URL: a bound site's diagnostic one.
    pub(crate) fn url(&self) -> &Url {
        match self {
            Self::Url(url) | Self::Native { url, .. } | Self::Opened { url, .. } => url,
            Self::Bound(bound) => bound.diagnostic_url(),
        }
    }

    /// Open the handle this site names, touching nothing.
    pub(crate) fn resolve(&self, properties: &Properties) -> Result<Holder> {
        match self {
            Self::Url(url) => Holder::from_url(url, properties),
            Self::Native { held, .. } => {
                let properties: Vec<(String, String)> = properties
                    .iter()
                    .map(|(name, value)| (name.to_owned(), value.to_owned()))
                    .collect();
                Holder::from_handle_with(held, &properties)
            }
            Self::Bound(bound) => Ok(crate::fs::located(bound.clone())),
            Self::Opened { open, .. } => open(properties),
        }
    }
}

impl PartialEq for Site {
    /// The location: two sites opening one location under two openers are
    /// one site, as two handles on one location are.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Url(left), Self::Url(right))
            | (Self::Native { url: left, .. }, Self::Native { url: right, .. })
            | (Self::Opened { url: left, .. }, Self::Opened { url: right, .. }) => left == right,
            (Self::Bound(left), Self::Bound(right)) => left.same_location(right),
            _ => false,
        }
    }
}

impl Eq for Site {}

impl Hash for Site {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Url(url) | Self::Native { url, .. } | Self::Opened { url, .. } => url.hash(state),
            Self::Bound(bound) => {
                bound.diagnostic_url().hash(state);
                bound.path().hash(state);
            }
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
/// its storage and nothing is opened before it is needed. A successful leaf
/// record write by a `MediaTable` closes its located holder's session, releasing
/// mappings and wrapper caches while retaining the holder's media and backend
/// options; a bound handle with no site is retained as the data itself. Folder
/// and format writes and direct byte operations keep their held session. A clone starts
/// unresolved and rebuilds from the site under the same properties - a
/// native object-store or HTTP handle on the client it was built with
/// ([`Holder::from_handle`]), sending nothing to resolve; an
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

    /// Release a located writer's cached session, retaining its configuration.
    /// A bound handle without a site is the data itself.
    pub(crate) fn release_after_write(&mut self) -> Result<()> {
        if self.site.is_some() {
            self.held.get_mut().map_or(Ok(()), |held| held.close())
        } else {
            Ok(())
        }
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
