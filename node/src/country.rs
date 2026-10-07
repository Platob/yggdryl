//! The native ISO 3166-1 alpha-2 country code.

use napi::bindgen_prelude::Result;
use napi_derive::napi;
use yggdryl::{Country, DataType, Scalar};

use crate::napi_error;

/// A country code as the `country` datatype reads one.
fn country_of(text: &str) -> Result<Country> {
    match DataType::Country
        .scalar(Scalar::from(text))
        .map_err(napi_error)?
    {
        Scalar::Country(country) => Ok(country),
        other => Err(napi_error(format!(
            "expected a country code, got {}",
            other.kind()
        ))),
    }
}

/// One ISO 3166-1 alpha-2 country code, held by its shape: at most two
/// ASCII bytes, read as the `country` datatype reads one. Immutable.
#[napi(js_name = "Country")]
#[derive(Clone)]
pub struct JsCountry {
    inner: Country,
}

#[napi]
impl JsCountry {
    /// The code `code` spells, refused where the `country` datatype refuses
    /// it.
    #[napi(constructor)]
    pub fn new(code: String) -> Result<Self> {
        country_of(&code).map(|inner| Self { inner })
    }

    /// Whether ISO 3166-1 currently assigns this code.
    #[napi(getter)]
    pub fn is_listed(&self) -> bool {
        self.inner.is_listed()
    }

    /// The legal tender ISO 4217 list one gives this country - one currency
    /// per country, a fund code never - or `null` where it gives none: the
    /// user-assigned `XX` and `ZZ`, an agency prefix (`XS`, `EU`), a country
    /// with no universal currency.
    #[napi(getter)]
    pub fn currency(&self) -> Option<String> {
        self.inner.currency().map(|ccy| ccy.to_string())
    }

    /// Whether `other` is the same code.
    #[napi]
    pub fn equals(&self, other: &JsCountry) -> bool {
        self.inner == other.inner
    }

    /// The code.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// The code, as JSON states it.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> String {
        self.inner.to_string()
    }
}
