//! The character encodings, exposed as `decode`/`encode` over one `bytes`.
//!
//! Python already owns a codec registry, so these functions exist to be
//! measured and checked against it rather than to replace it: `decode` answers
//! what `bytes.decode(name)` answers, and `encode` what `str.encode(name)`
//! does, over the same names. What they add is the crate's own refusal - the
//! charset, the byte position, and the byte or scalar found there - and one
//! vocabulary shared with the handles, media types, and record options that
//! already name a charset.
//!
//! A whole resource is decoded by naming the charset on its media type, not by
//! calling these: [`yggdryl.enums.CHARSETS`] is the listing, and a handle
//! carrying `charset=windows-1252` reads as text without a caller passing
//! anything.

use pyo3::prelude::*;
use pyo3::types::PyBytes;

use yggdryl::Charset;

use crate::value_error;

/// Resolve a charset name or alias, or raise.
fn charset_of(name: &str) -> PyResult<Charset> {
    Charset::from_str(name).map_err(value_error)
}

/// Decode bytes in one charset, as `bytes.decode(name)` reads the same bytes.
#[pyfunction]
pub(crate) fn charset_decode(py: Python<'_>, charset: &str, data: &[u8]) -> PyResult<String> {
    let charset = charset_of(charset)?;
    py.detach(|| charset.decode(data))
        .map(std::borrow::Cow::into_owned)
        .map_err(value_error)
}

/// Decode bytes in one charset, replacing what the charset cannot read.
#[pyfunction]
pub(crate) fn charset_decode_lossy(py: Python<'_>, charset: &str, data: &[u8]) -> PyResult<String> {
    let charset = charset_of(charset)?;
    Ok(py.detach(|| charset.decode_lossy(data).into_owned()))
}

/// Encode text in one charset, as `str.encode(name)` writes the same text.
#[pyfunction]
pub(crate) fn charset_encode<'py>(
    py: Python<'py>,
    charset: &str,
    text: &str,
) -> PyResult<Bound<'py, PyBytes>> {
    let charset = charset_of(charset)?;
    let encoded = py
        .detach(|| charset.encode(text).map(std::borrow::Cow::into_owned))
        .map_err(value_error)?;
    Ok(PyBytes::new(py, &encoded))
}

/// The charset a byte-order mark names, with the mark's byte length.
///
/// Answers `None` when the payload carries no mark. The mark is not removed:
/// its length is answered so a caller decides whether `U+FEFF` is data.
#[pyfunction]
pub(crate) fn charset_from_bom(data: &[u8]) -> Option<(&'static str, usize)> {
    Charset::from_bom(data).map(|(charset, length)| (charset.as_str(), length))
}

/// The byte-order mark a charset is written with, if it has one.
#[pyfunction]
pub(crate) fn charset_bom<'py>(
    py: Python<'py>,
    charset: &str,
) -> PyResult<Option<Bound<'py, PyBytes>>> {
    Ok(charset_of(charset)?
        .bom()
        .map(|mark| PyBytes::new(py, mark)))
}

/// The canonical name a charset name or alias resolves to.
#[pyfunction]
pub(crate) fn charset_canonical_name(charset: &str) -> PyResult<&'static str> {
    charset_of(charset).map(Charset::as_str)
}
