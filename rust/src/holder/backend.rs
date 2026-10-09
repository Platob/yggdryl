//! The byte backends a crate claims, and the register [`Holder::from_url`]
//! reads them from.
//!
//! A [`StorageBackend`] is one `static` in its crate naming the schemes its
//! locations spell and opening the handle one names; it is claimed on
//! [`Register`] under every scheme it names, all or none. [`Holder::from_url`]
//! asks it after the identifier is lowered to a location and after the local
//! and ZIP arms, before HTTP, and describes what it answers - `media_type`,
//! `codec` - as it describes every backend's handle. The handle is a
//! [`RegisteredHandle`], held as [`Holder::Registered`]: every [`IOBase`] verb
//! its own, so a backend outside the core costs exactly the requests it
//! states. A scheme a core arm holds is never a backend's to claim. The core
//! claims the object stores' backend itself under the `s3` feature, until
//! `yggdryl-s3`'s `install()` does; a scheme no claim answers is refused
//! naming the crate to install.

use std::any::Any;
use std::fmt;
use std::sync::{Mutex, OnceLock};

use smol_str::{SmolStr, format_smolstr};

use crate::holder::Holder;
use crate::plugin::{CORE, Register};
use crate::{Error, IOBase, Result, Scheme, Url};

/// One byte backend, stated once as a `static` in its own crate.
pub trait StorageBackend: fmt::Debug + Send + Sync + 'static {
    /// The crate that holds the backend, as a refusal names it:
    /// `yggdryl-s3`.
    fn name(&self) -> &'static str;

    /// Every scheme the backend's locations spell, the keys it is claimed
    /// under, all or none: `s3`, `gs`, `az` and their aliases.
    fn schemes(&self) -> &'static [Scheme];

    /// Whether `name` is a property the backend reads for itself - who signs,
    /// where the store is, how it is addressed - rather than one it leaves to
    /// the object it holds.
    ///
    /// A location's query states the backend's properties: a parameter this
    /// answers `false` for is refused by name before the backend is asked,
    /// and an Iceberg table opened by its location states its properties
    /// less these.
    fn is_property(&self, name: &str) -> bool;

    /// The handle of the location `url` names, under `properties`: the
    /// query's first, then the caller's, so a property stated twice is the
    /// caller's. `url` arrives with its query taken off, since a query says
    /// how the resource is reached and not which resource it is. Building
    /// the handle sends nothing.
    ///
    /// # Errors
    ///
    /// Returns the backend's refusal of the location or of a property.
    fn holder(&self, url: &Url, properties: &[(String, String)]) -> Result<Holder>;
}

/// A handle a [`StorageBackend`] answers, held as [`Holder::Registered`]: a
/// byte handle answering every [`IOBase`] verb itself - the capability verbs
/// a backend specializes ([`IOBase::upload_from`], [`IOBase::discard`],
/// [`IOBase::as_leaf`], [`IOBase::as_container`], [`IOBase::set_known_size`],
/// [`IOBase::owned_stream_bytes`]) included - and the few questions
/// [`Holder`] asks of the handle it holds.
pub trait RegisteredHandle: IOBase + Sync + fmt::Debug {
    /// The implementation's own name, as a refusal names it: `S3File`.
    fn implementation_name(&self) -> &'static str;

    /// Whether anything is at the handle's location now, its role's own
    /// answer: a container whether it is there, a leaf whether it is, a
    /// location whether either is - what [`Holder::exists`] answers.
    fn exists(&self) -> bool;

    /// A second handle on the resource this one addresses, over the same
    /// client and its role kept, sending nothing: what
    /// [`Holder::from_handle`] answers for it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for a handle with no resource to
    /// address again, else the backend's refusal to build the handle.
    fn reopen(&self) -> Result<Holder>;

    /// The handle as `Any`, for [`Holder::downcast_ref`].
    fn as_any(&self) -> &dyn Any;

    /// The handle as `Any`, mutably, for [`Holder::downcast_mut`].
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// What `scheme` is held by in the core, where a core arm holds it: never a
/// backend's to claim, since [`Holder::from_url`] answers it before any
/// backend - or, for an identifier, lowers it before one is asked.
fn core_arm(scheme: &Scheme) -> Option<&'static str> {
    Some(match scheme.as_str() {
        "file" => "the local and ZIP arm holds it",
        "http" | "https" => "the HTTP arm holds it",
        "mem" => "an in-memory buffer's identity spells it",
        "urn" | "arn" => "an identifier spelling it is lowered before any backend is asked",
        _ => return None,
    })
}

static BACKENDS: Register<Scheme, &'static dyn StorageBackend> = Register::new("storage backend");
static SEEDED: OnceLock<()> = OnceLock::new();
/// One claim at a time, so a backend's schemes are claimed all or none.
static CLAIMING: Mutex<()> = Mutex::new(());

/// Claim the core's own backends once, before the register answers
/// anything.
fn seed() {
    SEEDED.get_or_init(|| {
        // The core's claims cannot conflict: each scheme is stated once in
        // the crate.
        #[cfg(feature = "s3")]
        claim_unseeded(&crate::s3::S3_BACKEND, CORE)
            .expect("the core's own storage backends claim cleanly");
    });
}

/// Claim `backend` for the crate `by`: every scheme it names, once for the
/// life of the process, all or none.
///
/// ```
/// use yggdryl::holder::{Holder, StorageBackend, claim_backend};
/// use yggdryl::{Result, Scheme, Url};
///
/// #[derive(Debug)]
/// struct Shadow;
///
/// static SHADOWED: [Scheme; 1] = [Scheme::FILE];
///
/// impl StorageBackend for Shadow {
///     fn name(&self) -> &'static str {
///         "shadow"
///     }
///     fn schemes(&self) -> &'static [Scheme] {
///         &SHADOWED
///     }
///     fn is_property(&self, _name: &str) -> bool {
///         false
///     }
///     fn holder(&self, url: &Url, _properties: &[(String, String)]) -> Result<Holder> {
///         Ok(Holder::from(url.clone()))
///     }
/// }
///
/// static SHADOW: Shadow = Shadow;
///
/// // `file:` is the core's own arm, so no crate claims it.
/// let refused = claim_backend(&SHADOW, "shadow").unwrap_err().to_string();
/// assert!(refused.contains("`file`"), "{refused}");
/// ```
///
/// # Errors
///
/// Returns [`Error::Conflict`] naming the first claimant where a scheme is
/// claimed already, and [`Error::InvalidRecord`] at `$.url` for a claim in
/// the core's own name, a backend naming no scheme or one scheme twice, or
/// one naming a scheme a core arm holds - `file`, `http`, `https`, `mem`,
/// `urn` or `arn`.
pub fn claim(backend: &'static dyn StorageBackend, by: &'static str) -> Result<()> {
    seed();
    if by == CORE {
        return Err(invalid(format_smolstr!(
            "a storage backend is claimed by the crate that holds it, never as `{CORE}`"
        )));
    }
    claim_unseeded(backend, by)
}

fn claim_unseeded(backend: &'static dyn StorageBackend, by: &'static str) -> Result<()> {
    let _claiming = CLAIMING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let schemes = backend.schemes();
    if schemes.is_empty() {
        return Err(invalid(format_smolstr!(
            "a storage backend names at least one scheme, `{}` names none",
            backend.name()
        )));
    }
    // Every scheme checked before any is claimed, so a refused claim leaves
    // the register as it was.
    for (position, scheme) in schemes.iter().enumerate() {
        if let Some(arm) = core_arm(scheme) {
            return Err(invalid(format_smolstr!(
                "`{scheme}` is not a storage backend's to claim: {arm}"
            )));
        }
        if schemes[..position].contains(scheme) {
            return Err(invalid(format_smolstr!(
                "`{}` names the scheme `{scheme}` twice",
                backend.name()
            )));
        }
        if let Some(first) = BACKENDS.claimant(scheme) {
            return Err(Error::Conflict {
                expected: "storage backend",
                actual: first,
                path: SmolStr::new(scheme.as_str()),
            });
        }
    }
    for scheme in schemes {
        BACKENDS.claim(scheme.clone(), backend, by)?;
    }
    Ok(())
}

fn invalid(reason: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$.url"),
        reason,
    }
}

/// The backend claimed under `scheme`, if any: the one question
/// [`Holder::from_url`] asks of a lowered location no core arm holds.
///
/// ```
/// use yggdryl::Scheme;
/// use yggdryl::holder::backend_for;
///
/// // The core holds `file:` itself, so no backend is claimed under it.
/// assert!(backend_for(&Scheme::FILE).is_none());
/// assert!(backend_for(&Scheme::from_str("ftp")?).is_none());
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[must_use]
pub fn backend_for(scheme: &Scheme) -> Option<&'static dyn StorageBackend> {
    seed();
    BACKENDS.get(scheme)
}

/// Every claimed backend, once each, in the order of its first scheme.
#[must_use]
pub fn backends() -> Vec<&'static dyn StorageBackend> {
    seed();
    let mut backends: Vec<&'static dyn StorageBackend> = Vec::new();
    for backend in BACKENDS.values() {
        if !backends
            .iter()
            .any(|held| std::ptr::addr_eq(*held, backend))
        {
            backends.push(backend);
        }
    }
    backends
}

/// The refusal of a location whose scheme no claim answers: no core arm, no
/// claimed backend and no locator holds it as a handle, and no catalog
/// factory reads it as a catalog.
pub(crate) fn unregistered(scheme: &Scheme) -> Error {
    Error::unsupported(
        "holding a location of this scheme; install the crate that claims it and call its \
         `install()`",
        scheme.as_str(),
    )
}
