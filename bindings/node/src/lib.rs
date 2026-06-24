//! Node.js extension for **yggdryl**.
//!
//! Thin napi-rs wrappers around [`yggdryl_url::Uri`]/[`yggdryl_url::Url`],
//! [`yggdryl_version::Version`], [`yggdryl_media::MimeType`] and
//! [`yggdryl_media::MediaType`]; each type lives in its own module, mirroring the
//! Rust crates. All logic lives in the shared core so the Node and Python
//! bindings stay in lockstep.

mod bytesio;
mod media;
mod mime;
mod uri;
mod url;
mod version;

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_url::{percent_decode, percent_encode, Mapping};

/// Converts a JS object (`HashMap`) into the core ordered [`Mapping`].
pub(crate) fn to_mapping(fields: HashMap<String, String>) -> Mapping {
    fields.into_iter().collect()
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
