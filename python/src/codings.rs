//! The byte-level content codings, exposed as `loads`/`dumps` pairs.
//!
//! Each function is the core's whole-buffer codec over one `bytes` value, so
//! Python holds the same codings the handles already apply by name - and can
//! measure them against `gzip`, `zlib`, and `compression.zstd` from the
//! standard library, which carry the same wire formats. Raw DEFLATE is its own
//! pair rather than a flag on the zlib one, because the two framings are two
//! wire formats and a value written as one cannot be read as the other.

use pyo3::prelude::*;
use pyo3::types::PyBytes;

use yggdryl::Level;

use crate::value_error;

fn level_of(level: Option<u8>) -> Level {
    level.map_or(Level::DEFAULT, Level::new)
}

/// Define one `loads`/`dumps` pair over one core coding entry point.
///
/// The core functions are named rather than derived from the module, because a
/// module offers more than one framing of the same algorithm: zlib carries both
/// the RFC 1950 wrapper and the bare RFC 1951 stream, and each framing is its
/// own pair here so a caller never picks one with a flag.
macro_rules! coding {
    (
        $load:ident,
        $dump:ident,
        $module:ident,
        $core_load:ident,
        $core_dump:ident,
        $decode_doc:literal,
        $encode_doc:literal
    ) => {
        #[doc = $decode_doc]
        #[pyfunction]
        pub(crate) fn $load<'py>(py: Python<'py>, data: &[u8]) -> PyResult<Bound<'py, PyBytes>> {
            let decoded = py
                .detach(|| yggdryl::$module::$core_load(data))
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
                .detach(|| yggdryl::$module::$core_dump(data, level_of(level)))
                .map_err(value_error)?;
            Ok(PyBytes::new(py, &encoded))
        }
    };
}

coding!(
    gzip_loads,
    gzip_dumps,
    gzip,
    load,
    dump_with_level,
    "Decode one gzip value, as `gzip.decompress` reads the same bytes.",
    "Encode one gzip value; `level` is the shared 0-9 scale."
);
coding!(
    zlib_loads,
    zlib_dumps,
    zlib,
    load,
    dump_with_level,
    "Decode one zlib value, as `zlib.decompress` reads the same bytes.",
    "Encode one zlib value; `level` is the shared 0-9 scale."
);
coding!(
    zlib_loads_raw,
    zlib_dumps_raw,
    zlib,
    load_raw,
    dump_raw_with_level,
    "Decode one raw DEFLATE value - no zlib header, no checksum - as \
     `zlib.decompress(data, -zlib.MAX_WBITS)` reads the same bytes.",
    "Encode one raw DEFLATE value, the framing a zip member and an Avro \
     `deflate` block carry; `level` is the shared 0-9 scale."
);
coding!(
    zstd_loads,
    zstd_dumps,
    zstd,
    load,
    dump_with_level,
    "Decode one Zstandard value.",
    "Encode one Zstandard value; `level` is the shared 0-9 scale."
);
