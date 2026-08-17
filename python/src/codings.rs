//! The byte-level content codings, exposed as `loads`/`dumps` pairs.
//!
//! Each function is the core's whole-buffer codec over one `bytes` value, so
//! Python holds the same three codings the handles already apply by name -
//! and can measure them against `gzip`, `zlib`, and `compression.zstd` from
//! the standard library, which carry the same wire formats.

use pyo3::prelude::*;
use pyo3::types::PyBytes;

use yggdryl::Level;

use crate::value_error;

fn level_of(level: Option<u8>) -> Level {
    level.map_or(Level::DEFAULT, Level::new)
}

macro_rules! coding {
    ($load:ident, $dump:ident, $module:ident, $decode_doc:literal, $encode_doc:literal) => {
        #[doc = $decode_doc]
        #[pyfunction]
        pub(crate) fn $load<'py>(py: Python<'py>, data: &[u8]) -> PyResult<Bound<'py, PyBytes>> {
            let decoded = py
                .detach(|| yggdryl::$module::load(data))
                .map_err(value_error)?;
            Ok(PyBytes::new(py, &decoded))
        }

        #[doc = $encode_doc]
        #[pyfunction]
        #[pyo3(signature = (data, level = None))]
        pub(crate) fn $dump<'py>(
            py: Python<'py>,
            data: &[u8],
            level: Option<u8>,
        ) -> PyResult<Bound<'py, PyBytes>> {
            let encoded = py
                .detach(|| yggdryl::$module::dump_with_level(data, level_of(level)))
                .map_err(value_error)?;
            Ok(PyBytes::new(py, &encoded))
        }
    };
}

coding!(
    gzip_loads,
    gzip_dumps,
    gzip,
    "Decode one gzip value, as `gzip.decompress` reads the same bytes.",
    "Encode one gzip value; `level` is the shared 0-9 scale."
);
coding!(
    zlib_loads,
    zlib_dumps,
    zlib,
    "Decode one zlib value, as `zlib.decompress` reads the same bytes.",
    "Encode one zlib value; `level` is the shared 0-9 scale."
);
coding!(
    zstd_loads,
    zstd_dumps,
    zstd,
    "Decode one Zstandard value.",
    "Encode one Zstandard value; `level` is the shared 0-9 scale."
);
