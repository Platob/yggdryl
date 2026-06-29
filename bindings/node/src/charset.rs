//! Node wrapper for [`yggdryl_core::Charset`].

use napi_derive::napi;
use yggdryl_core::Charset as CoreCharset;

/// A text encoding for converting JSON between text and bytes (`Utf8` is the
/// default; `Ascii` and `Latin1` replace unrepresentable characters with `?`).
#[napi]
pub enum Charset {
    Utf8,
    Ascii,
    Latin1,
}

impl From<Charset> for CoreCharset {
    fn from(charset: Charset) -> Self {
        match charset {
            Charset::Utf8 => CoreCharset::Utf8,
            Charset::Ascii => CoreCharset::Ascii,
            Charset::Latin1 => CoreCharset::Latin1,
        }
    }
}

impl From<CoreCharset> for Charset {
    fn from(charset: CoreCharset) -> Self {
        match charset {
            CoreCharset::Utf8 => Charset::Utf8,
            CoreCharset::Ascii => Charset::Ascii,
            CoreCharset::Latin1 => Charset::Latin1,
        }
    }
}
