//! The byte-level content codings, exposed as `loads`/`dumps` pairs.
//!
//! Each function is the core's whole-buffer codec over one `Buffer`, the same
//! three codings a handle already applies by name - and the same wire formats
//! `node:zlib` carries, so either side reads the other's output. The loader
//! groups the pairs into `gzip`, `zlib`, and `zstd` namespaces.

use napi::bindgen_prelude::{Buffer, Result};
use napi_derive::napi;

use yggdryl::Level;

use crate::napi_error;

fn level_of(level: Option<u8>) -> Level {
    level.map_or(Level::DEFAULT, Level::new)
}

/// Decode one whole gzip value.
#[napi(js_name = "_gzipLoads", skip_typescript)]
pub fn gzip_loads(data: Buffer) -> Result<Buffer> {
    yggdryl::gzip::load(data.as_ref())
        .map(Buffer::from)
        .map_err(napi_error)
}

/// Encode one whole gzip value; `level` is the shared 0-9 scale.
#[napi(js_name = "_gzipDumps", skip_typescript)]
pub fn gzip_dumps(data: Buffer, level: Option<u8>) -> Result<Buffer> {
    yggdryl::gzip::dump_with_level(data.as_ref(), level_of(level))
        .map(Buffer::from)
        .map_err(napi_error)
}

/// Decode one whole zlib value.
#[napi(js_name = "_zlibLoads", skip_typescript)]
pub fn zlib_loads(data: Buffer) -> Result<Buffer> {
    yggdryl::zlib::load(data.as_ref())
        .map(Buffer::from)
        .map_err(napi_error)
}

/// Encode one whole zlib value; `level` is the shared 0-9 scale.
#[napi(js_name = "_zlibDumps", skip_typescript)]
pub fn zlib_dumps(data: Buffer, level: Option<u8>) -> Result<Buffer> {
    yggdryl::zlib::dump_with_level(data.as_ref(), level_of(level))
        .map(Buffer::from)
        .map_err(napi_error)
}

/// Decode one whole Zstandard value.
#[napi(js_name = "_zstdLoads", skip_typescript)]
pub fn zstd_loads(data: Buffer) -> Result<Buffer> {
    yggdryl::zstd::load(data.as_ref())
        .map(Buffer::from)
        .map_err(napi_error)
}

/// Encode one whole Zstandard value; `level` is the shared 0-9 scale.
#[napi(js_name = "_zstdDumps", skip_typescript)]
pub fn zstd_dumps(data: Buffer, level: Option<u8>) -> Result<Buffer> {
    yggdryl::zstd::dump_with_level(data.as_ref(), level_of(level))
        .map(Buffer::from)
        .map_err(napi_error)
}
