//! Python wrapper for the global JSON parameters ([`yggdryl_core::JsonParams`]).

use pyo3::prelude::*;
use yggdryl_core::JsonParams as CoreJsonParams;

use crate::{hash_of, py_bool, Charset};

/// Global JSON parameters — text format (compact vs pretty) plus the `Charset` the
/// bytes are (de)coded with — shared by every `to_json` / `to_bson`.
#[pyclass(module = "yggdryl", name = "JsonParams", frozen)]
#[derive(Clone)]
pub struct JsonParams {
    pub(crate) inner: CoreJsonParams,
}

#[pymethods]
impl JsonParams {
    #[new]
    #[pyo3(signature = (pretty = false, indent = 2, charset = Charset::Utf8))]
    fn new(pretty: bool, indent: usize, charset: Charset) -> Self {
        JsonParams {
            inner: CoreJsonParams::DEFAULT
                .with_pretty(pretty)
                .with_indent(indent)
                .with_charset(charset.into()),
        }
    }

    /// Pretty-printed output with the default 2-space indent and UTF-8 charset.
    #[staticmethod]
    fn pretty() -> Self {
        JsonParams {
            inner: CoreJsonParams::pretty(),
        }
    }

    /// Compact output with the default UTF-8 charset (the default).
    #[staticmethod]
    fn compact() -> Self {
        JsonParams {
            inner: CoreJsonParams::compact(),
        }
    }

    /// Whether output is pretty-printed.
    #[getter]
    fn is_pretty(&self) -> bool {
        self.inner.is_pretty()
    }

    /// The number of spaces per indent level when pretty-printed.
    #[getter]
    fn indent(&self) -> usize {
        self.inner.indent()
    }

    /// The charset JSON bytes are (de)coded with.
    #[getter]
    fn charset(&self) -> Charset {
        self.inner.charset().into()
    }

    /// A copy with a different pretty flag.
    fn with_pretty(&self, pretty: bool) -> Self {
        JsonParams {
            inner: self.inner.with_pretty(pretty),
        }
    }

    /// A copy with a different indent width.
    fn with_indent(&self, indent: usize) -> Self {
        JsonParams {
            inner: self.inner.with_indent(indent),
        }
    }

    /// A copy with a different charset.
    fn with_charset(&self, charset: Charset) -> Self {
        JsonParams {
            inner: self.inner.with_charset(charset.into()),
        }
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .extract::<JsonParams>()
            .is_ok_and(|o| self.inner == o.inner)
    }

    fn __hash__(&self) -> u64 {
        hash_of(&self.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "JsonParams(pretty={}, indent={}, charset={:?})",
            py_bool(self.inner.is_pretty()),
            self.inner.indent(),
            self.inner.charset().name(),
        )
    }

    fn __getnewargs__(&self) -> (bool, usize, Charset) {
        (
            self.inner.is_pretty(),
            self.inner.indent(),
            self.inner.charset().into(),
        )
    }
}

/// Sets the global JSON parameters used by every `to_json` / `to_bson`.
#[pyfunction]
pub fn set_json_params(params: JsonParams) {
    yggdryl_core::set_json_params(params.inner);
}

/// The current global JSON parameters.
#[pyfunction]
pub fn json_params() -> JsonParams {
    JsonParams {
        inner: yggdryl_core::json_params(),
    }
}

/// Resets the global JSON parameters to the default (compact, UTF-8).
#[pyfunction]
pub fn reset_json_params() {
    yggdryl_core::reset_json_params();
}
