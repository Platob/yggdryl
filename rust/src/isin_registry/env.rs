//! The process-wide default registry, resolved once on first use.
//!
//! The default starts from the seed ([`IsinRegistry::seeded`]) - the
//! common instruments `config/isin/instruments.json` states, embedded at
//! build time - and lays what the environment names over it: the store's
//! rows fold over the seed's, so a value the store states wins and a seed
//! row the store has no row of stands, and the registry is clean after the
//! load. The seed reaches a store through the first commit that moves
//! anything, as part of its snapshot.

use std::sync::{Arc, Mutex, OnceLock};

use smol_str::format_smolstr;

use super::IsinRegistry;
use crate::local::LocalFolder;
use crate::{Error, Result, Url};

/// The environment variable naming the store the default loads from.
const REGISTRY_LOCATION: &str = "YGGDRYL_ISIN_REGISTRY_URI";

/// The folder under the home directory that is the production default:
/// `~/.config/yggdryl/isin/`, a folder of Arrow IPC parts.
const CONFIG_PATH: &str = ".config/yggdryl/isin/";

/// The default, once resolved. Unset until a load succeeds or a registry is
/// installed, so a failed load is retried by the next call.
static GLOBAL: OnceLock<Arc<Mutex<IsinRegistry>>> = OnceLock::new();

impl IsinRegistry {
    /// The registry the process environment names, loaded on the first
    /// call and shared behind one lock: what
    /// [`FixCodec::from_env`](crate::FixCodec::from_env) attaches, and what
    /// the caller commits.
    ///
    /// Nothing loads at module init and no thread is spawned: the first call
    /// resolves the default on the calling thread, reading the environment
    /// once per attempt, and every later call answers the same `Arc` once
    /// one succeeded. The resolution order is fixed, first match wins:
    ///
    /// 1. a registry installed by [`Self::install_env`], as it was given;
    /// 2. the location `YGGDRYL_ISIN_REGISTRY_URI` names, trimmed of
    ///    blanks: a URL of any scheme this build holds, or a bare path, a
    ///    leading `~/` the home directory, which itself is refused; bound
    ///    as [`Self::seeded_from_url`] binds, a store holding nothing yet a
    ///    first run; an empty value reads as unset;
    /// 3. `~/.config/yggdryl/isin/`, a folder of Arrow IPC parts the first
    ///    commit lays out, reached through [`LocalFolder::home`];
    /// 4. with no home directory, the seed bound to no store.
    ///
    /// A store is laid over the seed ([`Self::seeded_from_url`]): its rows
    /// fold over the seed's by the update rule, so a value the store states
    /// wins, a seed row it has no row of stands, and the registry is clean
    /// after the load. Absence is a first run at every step: a store that holds
    /// nothing loads as the seed bound to it. A location that names a
    /// scheme this build has no backend for, a store that cannot be read or
    /// a row the registry refuses is an error, never the seed alone.
    ///
    /// # Errors
    ///
    /// Returns the load failure. The default stays unresolved, so the next
    /// call retries the load rather than answering a registry that was
    /// never there.
    pub fn from_env() -> Result<&'static Arc<Mutex<Self>>> {
        if let Some(registry) = GLOBAL.get() {
            return Ok(registry);
        }
        let location = match std::env::var_os(REGISTRY_LOCATION) {
            Some(value) => Some(value.into_string().map_err(|value| Error::Codec {
                format: "text",
                position: 0,
                reason: format_smolstr!("expected UTF-8 in {REGISTRY_LOCATION}, got {value:?}"),
            })?),
            None => None,
        };
        let home = match LocalFolder::home() {
            Ok(home) => Some(home),
            Err(error) if error.is_absent() => None,
            Err(error) => return Err(error),
        };
        let registry = autoload(location.as_deref(), home)?;
        Ok(GLOBAL.get_or_init(|| Arc::new(Mutex::new(registry))))
    }

    /// Installs the registry every later [`Self::from_env`] answers, before
    /// anything resolves one.
    ///
    /// # Errors
    ///
    /// Returns a typed conflict when the default has already been resolved
    /// or installed, so the value every caller saw cannot change underneath
    /// them.
    pub fn install_env(registry: Self) -> Result<()> {
        Self::install_env_shared(Arc::new(Mutex::new(registry)))
    }

    /// [`Self::install_env`] of a registry already shared behind its lock:
    /// the very `Arc` every later [`Self::from_env`] answers, so a caller
    /// holding it - a binding's object, a codec's - and the process default
    /// are one table.
    ///
    /// # Errors
    ///
    /// What [`Self::install_env`] refuses.
    pub fn install_env_shared(registry: Arc<Mutex<Self>>) -> Result<()> {
        GLOBAL.set(registry).map_err(|_| {
            Error::conflict(
                "isin registry",
                "isin registry",
                "the process default, already resolved",
            )
        })
    }
}

/// Resolve the default from its two inputs, in the documented order: the
/// store they name laid over the seed, or the seed alone.
///
/// Pure in both: `registry_location` is what `YGGDRYL_ISIN_REGISTRY_URI`
/// held and `home` what [`LocalFolder::home`] answered, so the rule is
/// tested with explicit inputs and never through the process-wide
/// environment.
pub(super) fn autoload(
    registry_location: Option<&str>,
    home: Option<LocalFolder>,
) -> Result<IsinRegistry> {
    let url = match registry_location
        .map(str::trim)
        .filter(|location| !location.is_empty())
    {
        Some(location) => located(location, home.as_ref())?,
        None => match home {
            Some(home) => under_home(&home, CONFIG_PATH)?,
            None => return Ok(IsinRegistry::seeded()),
        },
    };
    let none: [(&str, &str); 0] = [];
    IsinRegistry::seeded_from_url(&url, none)
}

/// The URL `location` names: a URL as spelled, a bare path rooted on the
/// working directory, and a path opening with `~` and a separator rooted
/// on `home` - the home directory itself refused, because a store owns the
/// record leaves beneath it.
fn located(location: &str, home: Option<&LocalFolder>) -> Result<Url> {
    if let Some(rest) = location.strip_prefix('~')
        && (rest.is_empty() || rest.starts_with(['/', '\\']))
    {
        let Some(home) = home else {
            return Err(Error::absent("home directory", location));
        };
        if rest.trim_matches(['/', '\\']).is_empty() {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static(REGISTRY_LOCATION),
                reason: format_smolstr!(
                    "expected a folder or a leaf under the home directory, got the home directory itself: {location:?}"
                ),
            });
        }
        return under_home(home, rest);
    }
    Url::from_location(location)
}

/// The URL of the path `rest` spells under `home`, read as a platform path
/// so a blank, a non-ASCII character or a backslash in it is the path's
/// own; a trailing separator keeps naming a folder.
fn under_home(home: &LocalFolder, rest: &str) -> Result<Url> {
    let mut path = home.path()?.into_os_string();
    path.push(std::path::MAIN_SEPARATOR_STR);
    path.push(rest.trim_start_matches(['/', '\\']));
    let text = path.to_str().ok_or_else(|| Error::InvalidRecord {
        path: smol_str::SmolStr::new_static("home"),
        reason: format_smolstr!("expected UTF-8 in the home directory path, got {path:?}"),
    })?;
    Url::from_location(text)
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/isin_registry/env.rs` pins and a caller cannot
    //! reach.
    //!
    //! [`IsinRegistry::from_env`](crate::IsinRegistry::from_env) resolves
    //! once per process and reads the environment, so the order it resolves
    //! in is pinned through the pure step underneath it instead.
    use super::IsinRegistry;
    use crate::Result;
    use crate::local::LocalFolder;

    /// Resolve the default from its two inputs, in the documented order.
    ///
    /// # Errors
    ///
    /// Returns a typed failure where a stated location names a scheme this
    /// crate has no backend for, a store that cannot be read or a row the
    /// registry refuses.
    pub fn autoload(
        registry_location: Option<&str>,
        home: Option<LocalFolder>,
    ) -> Result<IsinRegistry> {
        super::autoload(registry_location, home)
    }
}
