//! Python wrapper for the global JSON format ([`yggdryl_core::JsonFormat`]).

use pyo3::prelude::*;
use yggdryl_core::JsonFormat as CoreJsonFormat;

use crate::{hash_of, py_bool};

/// Global JSON formatting parameters shared by every `to_json`.
#[pyclass(module = "yggdryl", name = "JsonFormat", frozen)]
#[derive(Clone)]
pub struct JsonFormat {
    pub(crate) inner: CoreJsonFormat,
}

#[pymethods]
impl JsonFormat {
    #[new]
    #[pyo3(signature = (pretty = false, indent = 2))]
    fn new(pretty: bool, indent: usize) -> Self {
        JsonFormat {
            inner: CoreJsonFormat::DEFAULT
                .with_pretty(pretty)
                .with_indent(indent),
        }
    }

    /// Pretty-printed output with the default 2-space indent.
    #[staticmethod]
    fn pretty() -> Self {
        JsonFormat {
            inner: CoreJsonFormat::pretty(),
        }
    }

    /// Compact output (the default).
    #[staticmethod]
    fn compact() -> Self {
        JsonFormat {
            inner: CoreJsonFormat::compact(),
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

    /// A copy with a different pretty flag.
    fn with_pretty(&self, pretty: bool) -> Self {
        JsonFormat {
            inner: self.inner.with_pretty(pretty),
        }
    }

    /// A copy with a different indent width.
    fn with_indent(&self, indent: usize) -> Self {
        JsonFormat {
            inner: self.inner.with_indent(indent),
        }
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .extract::<JsonFormat>()
            .is_ok_and(|o| self.inner == o.inner)
    }

    fn __hash__(&self) -> u64 {
        hash_of(&self.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "JsonFormat(pretty={}, indent={})",
            py_bool(self.inner.is_pretty()),
            self.inner.indent()
        )
    }

    fn __getnewargs__(&self) -> (bool, usize) {
        (self.inner.is_pretty(), self.inner.indent())
    }
}

/// Sets the global JSON format used by every `to_json`.
#[pyfunction]
pub fn set_json_format(format: JsonFormat) {
    yggdryl_core::set_json_format(format.inner);
}

/// The current global JSON format.
#[pyfunction]
pub fn json_format() -> JsonFormat {
    JsonFormat {
        inner: yggdryl_core::json_format(),
    }
}

/// Resets the global JSON format to the default (compact).
#[pyfunction]
pub fn reset_json_format() {
    yggdryl_core::reset_json_format();
}
