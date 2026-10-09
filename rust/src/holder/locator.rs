//! The locations a catalog service keeps, and the register
//! [`Holder::from_url`] reads them from.
//!
//! A [`Locator`] is one `static` in its crate answering whether an
//! identifier names something its service keeps and, where it does, the
//! handle that thing is - a catalog, a namespace or a table, which declares
//! no media type and takes no coding. [`Holder::from_url`] asks every
//! claimed locator before it lowers the identifier to a location, since an
//! ARN says what the location it lowers to cannot, and before any byte
//! backend: the local, ZIP, object-store and HTTP backends stay the core's.
//! With no locator claimed the question costs nothing and reads no property.

use std::fmt;
use std::sync::OnceLock;

use smol_str::{SmolStr, format_smolstr};

use crate::holder::Holder;
use crate::plugin::{CORE, Register};
use crate::{Error, Properties, Result, Scheme, Uri};

/// One catalog service's locations, stated once as a `static`.
pub trait Locator: fmt::Debug + Send + Sync + 'static {
    /// The scheme the service's locations spell, the key the locator is
    /// claimed under: `s3tables`.
    fn scheme(&self) -> Scheme;

    /// Whether `location` names something the service keeps: a location of
    /// its scheme, or an identifier of the service in another - an ARN -
    /// read as given, before it is lowered.
    fn names(&self, location: &Uri) -> bool;

    /// The handle of what `location` names, under `properties`.
    ///
    /// # Errors
    ///
    /// Returns the service's refusal of the location, of a property, or of
    /// the request that finds what it names.
    fn holder(&self, location: &Uri, properties: &Properties) -> Result<Holder>;
}

static LOCATORS: Register<Scheme, &'static dyn Locator> = Register::new("locator");
static SEEDED: OnceLock<()> = OnceLock::new();

/// Claim the core's own locators once, before the register answers
/// anything.
fn seed() {
    SEEDED.get_or_init(|| {
        // The core's claims cannot conflict: each scheme is stated once in
        // the crate.
        #[cfg(feature = "s3tables")]
        LOCATORS
            .claim(
                crate::s3tables::S3TABLES_LOCATOR.scheme(),
                &crate::s3tables::S3TABLES_LOCATOR,
                CORE,
            )
            .expect("the core's own locators claim cleanly");
    });
}

/// Claim `locator` for the crate `by`, under its scheme, once for the life
/// of the process.
///
/// # Errors
///
/// Returns [`Error::Conflict`] naming the first claimant where the scheme is
/// claimed already, and [`Error::InvalidRecord`] at `$.url` for a claim in
/// the core's own name.
pub fn claim(locator: &'static dyn Locator, by: &'static str) -> Result<()> {
    seed();
    if by == CORE {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$.url"),
            reason: format_smolstr!(
                "a locator is claimed by the crate that holds it, never as `{CORE}`"
            ),
        });
    }
    LOCATORS.claim(locator.scheme(), locator, by)
}

/// Every claimed locator, in scheme order.
#[must_use]
pub fn locators() -> Vec<&'static dyn Locator> {
    seed();
    LOCATORS.values()
}

/// The handle of what `location` names, if a claimed locator names it: the
/// one question [`Holder::from_url`] asks before it lowers an identifier.
/// `None` with nothing claimed, the properties unread.
///
/// # Errors
///
/// Returns the naming locator's refusal.
pub(crate) fn locate(location: &Uri, properties: &[(String, String)]) -> Result<Option<Holder>> {
    seed();
    for locator in LOCATORS.values() {
        if locator.names(location) {
            let properties: Properties = properties
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect();
            return locator.holder(location, &properties).map(Some);
        }
    }
    Ok(None)
}
