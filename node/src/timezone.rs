//! `Timezone`, exposed to JavaScript beside the zone names the runtime uses.
//!
//! JavaScript names a zone with a string and nothing else: `Intl` resolves one,
//! `Date` formats in one, and neither hands back a value that can be compared
//! or stored. The core canonicalizes on arrival, so a zone read out of `Intl`,
//! an alias written by hand, and a fixed offset all become one value that
//! compares, hashes, and round-trips through a schema.

use napi::bindgen_prelude::{ClassInstance, Either, Result};
use napi_derive::napi;

use yggdryl::Timezone;

use crate::{exact_i64, napi_error, ordering_value};

/// A native zone wrapper or an IANA name, alias, or fixed offset.
pub(crate) type TimezoneInput<'a> = Either<ClassInstance<'a, JsTimezone>, String>;

/// Read a core zone out of anything JavaScript uses to name one.
pub(crate) fn timezone_from_input(value: TimezoneInput<'_>) -> Result<Timezone> {
    match value {
        Either::A(value) => Ok(value.inner.clone()),
        Either::B(value) => Timezone::from_str(&value).map_err(napi_error),
    }
}

/// One alias and the canonical name it resolves to.
#[napi(object)]
pub struct TimezoneAlias {
    /// The name a caller may write.
    pub alias: String,
    /// The canonical name it resolves to.
    pub canonical: String,
}

/// A canonical IANA time zone, with the offset rules this build knows.
#[napi(js_name = "Timezone")]
pub struct JsTimezone {
    pub(crate) inner: Timezone,
}

impl Clone for JsTimezone {
    fn clone(&self) -> Self {
        Self::from_core(self.inner.clone())
    }
}

impl JsTimezone {
    pub(crate) fn from_core(inner: Timezone) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsTimezone {
    /// Parse a zone name, alias, or fixed offset, or clone a native zone.
    #[napi(constructor)]
    pub fn new(value: TimezoneInput<'_>) -> Result<Self> {
        timezone_from_input(value).map(Self::from_core)
    }

    /// Infer a zone from a native wrapper or a name.
    #[napi(factory, js_name = "from")]
    pub fn from_js(value: TimezoneInput<'_>) -> Result<Self> {
        timezone_from_input(value).map(Self::from_core)
    }

    /// Parse and canonicalize a zone name, alias, or fixed offset.
    #[napi(factory)]
    pub fn from_string(value: String) -> Result<Self> {
        Timezone::from_str(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Build a zone from a fixed offset east of UTC, in seconds.
    #[napi(factory)]
    pub fn from_offset(seconds: i32) -> Result<Self> {
        Timezone::from_offset(seconds)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Every zone this build knows the rules for, sorted by name.
    #[napi]
    pub fn registered() -> Vec<JsTimezone> {
        Timezone::registered().map(Self::from_core).collect()
    }

    /// Every alias and the canonical name it resolves to.
    #[napi]
    pub fn aliases() -> Vec<TimezoneAlias> {
        Timezone::aliases()
            .map(|(alias, canonical)| TimezoneAlias {
                alias: alias.to_owned(),
                canonical: canonical.to_owned(),
            })
            .collect()
    }

    /// The canonical name.
    #[napi(getter)]
    pub fn key(&self) -> String {
        // Named `key` to match what `zoneinfo.ZoneInfo` and the Python binding
        // call it, so one name reads the zone in either runtime.
        self.inner.as_str().to_owned()
    }

    /// Whether this zone is UTC itself.
    #[napi]
    pub fn is_utc(&self) -> bool {
        self.inner.is_utc()
    }

    /// Whether this build knows the offset rules for this zone.
    #[napi]
    pub fn is_known(&self) -> bool {
        self.inner.is_known()
    }

    /// Whether the name is a fixed offset rather than a place.
    #[napi]
    pub fn is_fixed(&self) -> bool {
        self.inner.is_fixed()
    }

    /// Whether this zone ever observes daylight saving.
    #[napi]
    pub fn observes_saving(&self) -> bool {
        self.inner.observes_saving()
    }

    /// The offset east of UTC, in seconds, at an instant.
    ///
    /// `epoch` is seconds since the Unix epoch. A zone this build has no rules
    /// for answers `null` rather than guessing.
    #[napi]
    pub fn offset_at(&self, epoch: f64) -> Result<Option<i32>> {
        Ok(self.inner.offset_at(exact_i64(epoch, "epoch")?))
    }

    /// The standard offset east of UTC, ignoring daylight saving.
    #[napi(getter)]
    pub fn standard_offset(&self) -> Option<i32> {
        self.inner.standard_offset()
    }

    /// Whether daylight saving is in force at an instant.
    #[napi]
    pub fn is_saving_at(&self, epoch: f64) -> Result<Option<bool>> {
        Ok(self.inner.is_saving_at(exact_i64(epoch, "epoch")?))
    }

    /// The abbreviation in use at an instant, such as `EST` or `CEST`.
    #[napi]
    pub fn abbreviation_at(&self, epoch: f64) -> Result<Option<String>> {
        Ok(self
            .inner
            .abbreviation_at(exact_i64(epoch, "epoch")?)
            .map(ToOwned::to_owned))
    }

    /// The local reading, in epoch seconds, of a UTC instant.
    #[napi]
    pub fn into_local(&self, epoch: f64) -> Result<i64> {
        self.inner
            .clone()
            .into_local(exact_i64(epoch, "epoch")?)
            .map_err(napi_error)
    }

    /// The UTC instant, in epoch seconds, a local reading names.
    #[napi]
    pub fn into_utc(&self, local: f64) -> Result<i64> {
        self.inner
            .clone()
            .into_utc(exact_i64(local, "local")?)
            .map_err(napi_error)
    }

    /// The offset in minutes west of UTC, as `Date.getTimezoneOffset` reports.
    ///
    /// This is the duck-typing half: the sign and unit are JavaScript's, not
    /// the core's, so a zone can stand in wherever a `Date` offset is read.
    #[napi]
    pub fn get_timezone_offset(&self, epoch: f64) -> Result<Option<i32>> {
        Ok(self
            .inner
            .offset_at(exact_i64(epoch, "epoch")?)
            .map(|offset| -offset / 60))
    }

    /// Exact canonical equality: two spellings of one zone are equal.
    #[napi]
    pub fn equals(&self, other: &JsTimezone) -> bool {
        self.inner == other.inner
    }

    /// Total native ordering: `-1`, `0`, or `1`.
    #[napi]
    pub fn compare(&self, other: &JsTimezone) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
    }

    /// Deterministic XXH3-64 hash of the canonical name.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// Return the canonical name, accepted losslessly by `fromString`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.as_str().to_owned()
    }

    /// Serialize as the canonical name, so a zone survives `JSON.stringify`.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> String {
        self.inner.as_str().to_owned()
    }
}
