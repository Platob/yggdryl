//! Plugs `http` / `https` into the `yggdryl-io` scheme registry, so the universal
//! [`yggdryl_core::from_str`] / [`from_url`](yggdryl_core::from_url) factory can hand
//! back a streaming HTTP body for those URLs without `yggdryl-io` depending on this
//! crate.

use yggdryl_core::{Io, IoError, Uri};

use crate::{HttpRequest, HttpSession};

/// Opens an HTTP(S) URL as an [`Io`] handle: a `GET` sent through the shared
/// per-host [`HttpSession`](crate::HttpSession::shared_for), whose live
/// [`HttpResponse`](crate::HttpResponse) — itself an [`Io`] over the body — is
/// returned (raising on a 4xx/5xx). This is the [`yggdryl_core::SchemeOpener`]
/// registered for `http` / `https`, so `yggdryl_core::from_str("https://…")` hands
/// back a sent response ready to read.
fn open(uri: &Uri) -> Result<Box<dyn Io>, IoError> {
    let request =
        HttpRequest::get(&uri.to_string()).map_err(|err| IoError::Invalid(err.to_string()))?;
    let response = HttpSession::shared_for(request.url().host())
        .send(request, true)
        .map_err(|err| IoError::Io(err.to_string()))?;
    Ok(Box::new(response))
}

/// Registers `http` / `https` with the [`yggdryl_core`] scheme registry (idempotent),
/// so [`yggdryl_core::from_str`] opens those URLs as streaming bodies. It is called
/// automatically the first time an [`HttpSession`](crate::HttpSession) is created,
/// so `yggdryl_core::from_str("https://…")` just works once this crate is linked.
pub fn register() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        yggdryl_core::register_scheme("http", open);
        yggdryl_core::register_scheme("https", open);
    });
}
