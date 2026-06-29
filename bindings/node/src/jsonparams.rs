//! Node wrapper for the global JSON parameters ([`yggdryl_core::JsonParams`]).

use napi_derive::napi;
use yggdryl_core::JsonParams as CoreJsonParams;

use crate::Charset;

/// Global JSON parameters — text format (compact vs pretty) plus the `Charset` the
/// bytes are (de)coded with — shared by every `toJsonString` / `toBson`.
#[napi]
pub struct JsonParams {
    pub(crate) inner: CoreJsonParams,
}

#[napi]
impl JsonParams {
    #[napi(constructor)]
    pub fn new(pretty: Option<bool>, indent: Option<u32>, charset: Option<Charset>) -> Self {
        JsonParams {
            inner: CoreJsonParams::DEFAULT
                .with_pretty(pretty.unwrap_or(false))
                .with_indent(indent.unwrap_or(2) as usize)
                .with_charset(charset.unwrap_or(Charset::Utf8).into()),
        }
    }

    /// Pretty-printed output with the default 2-space indent and UTF-8 charset.
    #[napi(factory)]
    pub fn pretty() -> Self {
        JsonParams {
            inner: CoreJsonParams::pretty(),
        }
    }

    /// Compact output with the default UTF-8 charset (the default).
    #[napi(factory)]
    pub fn compact() -> Self {
        JsonParams {
            inner: CoreJsonParams::compact(),
        }
    }

    /// Whether output is pretty-printed.
    #[napi(getter)]
    pub fn is_pretty(&self) -> bool {
        self.inner.is_pretty()
    }

    /// The number of spaces per indent level when pretty-printed.
    #[napi(getter)]
    pub fn indent(&self) -> u32 {
        self.inner.indent() as u32
    }

    /// The charset JSON bytes are (de)coded with.
    #[napi(getter)]
    pub fn charset(&self) -> Charset {
        self.inner.charset().into()
    }

    /// A copy with a different pretty flag.
    #[napi]
    pub fn with_pretty(&self, pretty: bool) -> JsonParams {
        JsonParams {
            inner: self.inner.with_pretty(pretty),
        }
    }

    /// A copy with a different indent width.
    #[napi]
    pub fn with_indent(&self, indent: u32) -> JsonParams {
        JsonParams {
            inner: self.inner.with_indent(indent as usize),
        }
    }

    /// A copy with a different charset.
    #[napi]
    pub fn with_charset(&self, charset: Charset) -> JsonParams {
        JsonParams {
            inner: self.inner.with_charset(charset.into()),
        }
    }

    /// Structural equality with another `JsonParams`.
    #[napi]
    pub fn equals(&self, other: &JsonParams) -> bool {
        self.inner == other.inner
    }
}

/// Sets the global JSON parameters used by every `toJsonString` / `toBson`.
#[napi]
pub fn set_json_params(params: &JsonParams) {
    yggdryl_core::set_json_params(params.inner);
}

/// The current global JSON parameters.
#[napi]
pub fn json_params() -> JsonParams {
    JsonParams {
        inner: yggdryl_core::json_params(),
    }
}

/// Resets the global JSON parameters to the default (compact, UTF-8).
#[napi]
pub fn reset_json_params() {
    yggdryl_core::reset_json_params();
}
