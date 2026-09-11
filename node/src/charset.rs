//! The character encodings, exposed as `decode`/`encode` over one `Buffer`.
//!
//! `TextDecoder` and `TextEncoder` already exist in the runtime, so these
//! functions exist to be checked against them rather than to replace them:
//! `decode` answers what `new TextDecoder(name).decode(bytes)` answers over
//! the same names, and `encode` is the direction the runtime does not have at
//! all - `TextEncoder` writes UTF-8 and nothing else. What both add is the
//! crate's own refusal, naming the charset, the byte position, and the byte or
//! scalar found there, and one vocabulary shared with the media types, handles
//! and record options that already name a charset.
//!
//! The loader groups them into the `charset` namespace.

use napi::bindgen_prelude::{Buffer, Result};
use napi_derive::napi;

use yggdryl::Charset;

use crate::napi_error;

/// Resolve a charset name or alias, or throw.
fn charset_of(name: &str) -> Result<Charset> {
    Charset::from_str(name).map_err(napi_error)
}

/// Decode bytes in one charset, refusing what the charset cannot read.
#[napi(js_name = "_charsetDecode", skip_typescript)]
pub fn charset_decode(charset: String, data: Buffer) -> Result<String> {
    charset_of(&charset)?
        .decode(data.as_ref())
        .map(std::borrow::Cow::into_owned)
        .map_err(napi_error)
}

/// Decode bytes in one charset, replacing what the charset cannot read.
#[napi(js_name = "_charsetDecodeLossy", skip_typescript)]
pub fn charset_decode_lossy(charset: String, data: Buffer) -> Result<String> {
    Ok(charset_of(&charset)?
        .decode_lossy(data.as_ref())
        .into_owned())
}

/// Encode text in one charset, refusing a scalar it has no byte for.
#[napi(js_name = "_charsetEncode", skip_typescript)]
pub fn charset_encode(charset: String, text: String) -> Result<Buffer> {
    charset_of(&charset)?
        .encode(&text)
        .map(|encoded| Buffer::from(encoded.into_owned()))
        .map_err(napi_error)
}

/// A byte-order mark: the charset it declares, and how long the mark is.
#[napi(object, js_name = "CharsetMark")]
pub struct JsCharsetMark {
    /// The canonical charset name the mark declares.
    pub charset: String,
    /// How many bytes the mark itself occupies.
    pub length: u32,
}

/// The charset a byte-order mark names, and the mark's byte length.
///
/// Answers `null` when the payload carries no mark. The mark is not removed:
/// its length is answered so a caller decides whether `U+FEFF` is data.
#[napi(js_name = "_charsetFromBom", skip_typescript)]
pub fn charset_from_bom(data: Buffer) -> Option<JsCharsetMark> {
    Charset::from_bom(data.as_ref()).map(|(charset, length)| JsCharsetMark {
        charset: charset.as_str().to_owned(),
        length: u32::try_from(length).unwrap_or(u32::MAX),
    })
}

/// The byte-order mark a charset is written with, if it has one.
#[napi(js_name = "_charsetBom", skip_typescript)]
pub fn charset_bom(charset: String) -> Result<Option<Buffer>> {
    Ok(charset_of(&charset)?
        .bom()
        .map(|mark| Buffer::from(mark.to_vec())))
}

/// The canonical name a charset name or alias resolves to.
#[napi(js_name = "_charsetCanonicalName", skip_typescript)]
pub fn charset_canonical_name(charset: String) -> Result<String> {
    charset_of(&charset).map(|charset| charset.as_str().to_owned())
}
