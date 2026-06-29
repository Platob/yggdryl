//! Node wrapper for the global JSON format ([`yggdryl_core::JsonFormat`]).

use napi_derive::napi;
use yggdryl_core::JsonFormat as CoreJsonFormat;

/// Global JSON formatting parameters shared by every `toJsonString`.
#[napi]
pub struct JsonFormat {
    pub(crate) inner: CoreJsonFormat,
}

#[napi]
impl JsonFormat {
    #[napi(constructor)]
    pub fn new(pretty: Option<bool>, indent: Option<u32>) -> Self {
        JsonFormat {
            inner: CoreJsonFormat::DEFAULT
                .with_pretty(pretty.unwrap_or(false))
                .with_indent(indent.unwrap_or(2) as usize),
        }
    }

    /// Pretty-printed output with the default 2-space indent.
    #[napi(factory)]
    pub fn pretty() -> Self {
        JsonFormat {
            inner: CoreJsonFormat::pretty(),
        }
    }

    /// Compact output (the default).
    #[napi(factory)]
    pub fn compact() -> Self {
        JsonFormat {
            inner: CoreJsonFormat::compact(),
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

    /// A copy with a different pretty flag.
    #[napi]
    pub fn with_pretty(&self, pretty: bool) -> JsonFormat {
        JsonFormat {
            inner: self.inner.with_pretty(pretty),
        }
    }

    /// A copy with a different indent width.
    #[napi]
    pub fn with_indent(&self, indent: u32) -> JsonFormat {
        JsonFormat {
            inner: self.inner.with_indent(indent as usize),
        }
    }

    /// Structural equality with another `JsonFormat`.
    #[napi]
    pub fn equals(&self, other: &JsonFormat) -> bool {
        self.inner == other.inner
    }
}

/// Sets the global JSON format used by every `toJsonString`.
#[napi]
pub fn set_json_format(format: &JsonFormat) {
    yggdryl_core::set_json_format(format.inner);
}

/// The current global JSON format.
#[napi]
pub fn json_format() -> JsonFormat {
    JsonFormat {
        inner: yggdryl_core::json_format(),
    }
}

/// Resets the global JSON format to the default (compact).
#[napi]
pub fn reset_json_format() {
    yggdryl_core::reset_json_format();
}
