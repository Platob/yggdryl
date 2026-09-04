//! The process-wide default registry, resolved once on first use.

use std::sync::{Arc, OnceLock};

use smol_str::format_smolstr;

use super::FixRegistry;
use crate::holder::local::Folder;
use crate::{Error, Result, Url};

/// The environment variable naming the folder the default loads from.
const REGISTRY_LOCATION: &str = "YGGDRYL_FIX_REGISTRY";

/// The folder under the configuration directory that is the production
/// default: `~/.config/fix`.
const CONFIG_FOLDER: &str = "fix";

/// The default, once resolved. Unset until a load succeeds or a registry is
/// installed, so a failed load is retried by the next call.
static GLOBAL: OnceLock<Arc<FixRegistry>> = OnceLock::new();

impl FixRegistry {
    /// Returns the process-wide registry, loading it on the first call.
    ///
    /// Nothing loads at module init and no thread is spawned: the first call
    /// resolves the default on the calling thread, reading the environment
    /// exactly once, and every later call answers the same `Arc`. The
    /// resolution order is fixed, first match wins:
    ///
    /// 1. a registry installed by [`Self::install_global`];
    /// 2. the folder `YGGDRYL_FIX_REGISTRY` names - a URL, or a bare path -
    ///    opened through the local backend;
    /// 3. `~/.config/fix`, reached through [`Folder::config`], when that
    ///    folder exists; skipped when the machine has no home variable;
    /// 4. the empty registry.
    ///
    /// Step 3 is the one place absence is not a failure: a machine with no
    /// dictionary installed is an ordinary first-run state. A
    /// `YGGDRYL_FIX_REGISTRY` that is set but names no directory, a scheme
    /// this crate has no backend for, or a malformed shard under either
    /// folder is an error, never the empty registry - a default that quietly
    /// loaded nothing would turn every later lookup into a wrong answer.
    ///
    /// # Errors
    ///
    /// Returns the load failure. The default stays unresolved, so the next
    /// call retries the load rather than answering a registry that was
    /// never there.
    pub fn global() -> Result<&'static Arc<Self>> {
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
        let config = match Folder::config() {
            Ok(config) => Some(config),
            Err(error) if error.is_absent() => None,
            Err(error) => return Err(error),
        };
        let registry = autoload(location.as_deref(), config)?;
        Ok(GLOBAL.get_or_init(|| Arc::new(registry)))
    }

    /// Installs the process-wide registry before anything resolves it.
    ///
    /// # Errors
    ///
    /// Returns a typed conflict when the default has already been resolved
    /// or installed, so the value every caller saw cannot change underneath
    /// them.
    pub fn install_global(registry: Self) -> Result<()> {
        GLOBAL.set(Arc::new(registry)).map_err(|_| {
            Error::conflict(
                "fix registry",
                "fix registry",
                "the process default, already resolved",
            )
        })
    }
}

/// Resolve the default from its two inputs, in the documented order.
///
/// Pure in both: `registry_location` is what `YGGDRYL_FIX_REGISTRY` held and
/// `config` what [`Folder::config`] answered, so the rule is tested with
/// explicit inputs and never through the process-wide environment.
pub(super) fn autoload(
    registry_location: Option<&str>,
    config: Option<Folder>,
) -> Result<FixRegistry> {
    if let Some(location) = registry_location {
        return from_location(location);
    }
    if let Some(config) = config {
        // A lazy handle: an absent `~/.config/fix` lists nothing and loads
        // as the empty registry, while a present malformed one fails.
        let folder = Folder::from_url(config.url().joinpath(CONFIG_FOLDER)?)?;
        return FixRegistry::from_handle(&folder);
    }
    Ok(FixRegistry::new())
}

/// Open the folder an explicit location names, loudly.
fn from_location(location: &str) -> Result<FixRegistry> {
    let url = Url::from_str(location).or_else(|_| Url::from_path(location))?;
    if !url.is_local() {
        return Err(Error::absent(
            "storage backend",
            format_args!("scheme {}", url.scheme()),
        ));
    }
    let folder = Folder::from_url(url)?;
    if !folder.exists() {
        // The caller named this folder, so nothing there is a misconfiguration
        // rather than a first run.
        return Err(Error::absent("fix registry folder", folder.url()));
    }
    FixRegistry::from_handle(&folder)
}
