//! Plugs `http` / `https` into the `yggdryl-io` scheme registry, so the universal
//! [`yggdryl_io::from_str`] / [`from_url`](yggdryl_io::from_url) factory can hand
//! back a streaming HTTP body for those URLs without `yggdryl-io` depending on this
//! crate.

use yggdryl_io::{Io, IoError, Uri};

use crate::{HttpRequest, HttpSession};

/// Opens an HTTP(S) URL as a streaming [`Io`] body: a `GET` whose live
/// [`HttpStream`](crate::HttpStream) body is returned (raising on a 4xx/5xx). This
/// is the [`yggdryl_io::SchemeOpener`] registered for `http` / `https`.
fn open(uri: &Uri) -> Result<Box<dyn Io>, IoError> {
    let url = uri.to_string();
    let request = HttpRequest::get(&url).map_err(|err| IoError::Invalid(err.to_string()))?;
    let response = HttpSession::new()
        .send(request, true, true, true)
        .map_err(|err| IoError::Io(err.to_string()))?;
    Ok(response.into_io())
}

/// Registers `http` / `https` with the [`yggdryl_io`] scheme registry (idempotent),
/// so [`yggdryl_io::from_str`] opens those URLs as streaming bodies. It is called
/// automatically the first time an [`HttpSession`](crate::HttpSession) is created,
/// so `yggdryl_io::from_str("https://…")` just works once this crate is linked.
pub fn register() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        yggdryl_io::register_scheme("http", open);
        yggdryl_io::register_scheme("https", open);
    });
}
