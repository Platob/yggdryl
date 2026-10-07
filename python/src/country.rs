//! ISO 3166-1 alpha-2 country codes: the core reading `yggdryl.enums.Country`
//! redirects to.

use pyo3::prelude::*;
use yggdryl::Country;

use crate::value_error;

/// The legal tender ISO 4217 list one gives the country `code` names - one
/// currency per country, a fund code never - or `None` where list one gives
/// it none (`XX`, `ZZ`, an agency prefix). Raises `ValueError` where `code`
/// is no country code's shape.
#[pyfunction]
pub(crate) fn country_currency(code: &str) -> PyResult<Option<String>> {
    Ok(Country::new(code)
        .map_err(value_error)?
        .currency()
        .map(|currency| currency.as_str().to_owned()))
}
