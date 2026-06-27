//! Node.js extension for **yggdryl**.
//!
//! Thin napi-rs wrappers around [`yggdryl_core::Uri`]/[`yggdryl_core::Url`],
//! [`yggdryl_core::Version`], [`yggdryl_core::MimeType`] and
//! [`yggdryl_core::MediaType`]; each type lives in its own module, mirroring the
//! Rust crates. All logic lives in the shared core so the Node and Python
//! bindings stay in lockstep.

mod bytesio;
mod compression;
mod datatype;
mod date;
mod datetime;
mod duration;
mod field;
mod http;
mod iostats;
mod localpath;
mod media;
mod mime;
mod serie;
mod time;
mod timezone;
mod uri;
mod url;
mod version;

// Re-export the module-level HTTP verbs (backed by the shared `HttpSession`
// singleton) so they are part of the crate's public surface — napi exports them
// to JS regardless, this just keeps plain `cargo` from flagging them unused.
pub use http::{http_get, http_head, http_patch, http_post, http_put, http_request, set_base_url};

use std::collections::BTreeMap;
use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_core::Whence;
use yggdryl_core::{percent_decode, percent_encode};

/// Converts a JS object (`HashMap`) into the core ordered `BTreeMap`.
pub(crate) fn to_mapping(fields: HashMap<String, String>) -> BTreeMap<String, String> {
    fields.into_iter().collect()
}

/// Converts any displayable error (core / schema / time) into a thrown JS `Error`.
pub(crate) fn err<E: std::fmt::Display>(error: E) -> Error {
    Error::from_reason(error.to_string())
}

/// Converts a JS `BigInt` to an `i128`, throwing if the value does not fit (rather
/// than silently truncating, the way `get_i128().0` would). Shared by the
/// nanosecond-valued constructors.
pub(crate) fn bigint_i128(value: BigInt) -> Result<i128> {
    let (signed, lossless) = value.get_i128();
    if lossless {
        Ok(signed)
    } else {
        Err(Error::from_reason(
            "BigInt value does not fit in a signed 128-bit integer",
        ))
    }
}

/// Maps a `whence` integer (`0` start, `1` current, `2` end) to the core
/// [`Whence`], throwing on any other value. Shared by the seekable IO types.
pub(crate) fn whence_from(whence: u8) -> Result<Whence> {
    match whence {
        0 => Ok(Whence::Start),
        1 => Ok(Whence::Current),
        2 => Ok(Whence::End),
        other => Err(Error::from_reason(format!(
            "invalid whence ({other}), expected 0, 1 or 2"
        ))),
    }
}

/// URL-safe percent-encode `input` (e.g. a space becomes `%20`).
#[napi(js_name = "percentEncode")]
pub fn percent_encode_js(input: String) -> String {
    percent_encode(&input)
}

/// Percent-decode `input`, throwing on a malformed escape.
#[napi(js_name = "percentDecode")]
pub fn percent_decode_js(input: String) -> Result<String> {
    percent_decode(&input)
        .map(|decoded| decoded.into_owned())
        .map_err(|e| Error::from_reason(e.to_string()))
}

/// Open a byte-IO handle for `location`, dispatching on its URL scheme (the core
/// `Io` factory): a bare path or `file://` URL opens a `LocalPath`. Remote schemes
/// (`http` / `https`) are served by `HttpSession`; any other scheme throws.
#[napi]
pub fn open(location: String) -> Result<crate::localpath::LocalPath> {
    let uri =
        yggdryl_core::Uri::from_str(&location).map_err(|e| Error::from_reason(e.to_string()))?;
    match uri.scheme() {
        "file" | "" => Ok(crate::localpath::LocalPath {
            inner: yggdryl_core::LocalPath::from_uri(&uri),
        }),
        other => Err(Error::from_reason(format!(
            "no local Io handle for scheme {other:?}; use HttpSession for http/https"
        ))),
    }
}
