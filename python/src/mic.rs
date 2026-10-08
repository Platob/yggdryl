//! ISO 10383 market identifier codes: the core readings `yggdryl.enums.Mic`
//! redirects to.

use pyo3::prelude::*;
use yggdryl::Mic;

use crate::value_error;

/// The code `code` names, through the core's shape rule.
fn mic(code: &str) -> PyResult<Mic> {
    Mic::new(code).map_err(value_error)
}

/// The operating MIC `code` trades under in ISO 10383: itself for an
/// operating MIC, its market's for a segment, `None` for a code the registry
/// never assigned.
#[pyfunction]
pub(crate) fn mic_operating(code: &str) -> PyResult<Option<String>> {
    Ok(mic(code)?
        .operating()
        .map(|operating| operating.as_str().to_owned()))
}

/// Whether ISO 10383 lists `code` as a segment of another market.
#[pyfunction]
pub(crate) fn mic_is_segment(code: &str) -> PyResult<bool> {
    Ok(mic(code)?.is_segment())
}

/// The country ISO 10383 places `code` in, or `None`: a code it never
/// assigned, or one it places in no single country (`XOFF`, `XXXX`).
#[pyfunction]
pub(crate) fn mic_country(code: &str) -> PyResult<Option<String>> {
    Ok(mic(code)?
        .country()
        .map(|country| country.as_str().to_owned()))
}
