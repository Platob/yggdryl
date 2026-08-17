//! The byte-level content codings, exposed as `loads`/`dumps` pairs.
//!
//! Each function is the core's whole-buffer codec over one `Buffer`, the same
//! three codings a handle already applies by name - and the same wire formats
//! `node:zlib` carries, so either side reads the other's output.

use napi::bindgen_prelude::{Buffer, Result};
use napi_derive::napi;

use yggdryl::Level;

use crate::napi_error;

fn level_of(level: Option<u8>) -> Level {
    level.map_or(Level::DEFAULT, Level::new)
}

macro_rules! coding {
    ($load:ident, $load_name:literal, $dump:ident, $dump_name:literal, $module:ident) => {
        /// Decode one whole value of this coding.
        #[napi(js_name = $load_name)]
        pub fn $load(data: Buffer) -> Result<Buffer> {
            yggdryl::$module::load(data.as_ref())
                .map(Buffer::from)
                .map_err(napi_error)
        }

        /// Encode one whole value; `level` is the shared 0-9 scale.
        #[napi(js_name = $dump_name)]
        pub fn $dump(data: Buffer, level: Option<u8>) -> Result<Buffer> {
            yggdryl::$module::dump_with_level(data.as_ref(), level_of(level))
                .map(Buffer::from)
                .map_err(napi_error)
        }
    };
}

coding!(gzip_loads, "_gzipLoads", gzip_dumps, "_gzipDumps", gzip);
coding!(zlib_loads, "_zlibLoads", zlib_dumps, "_zlibDumps", zlib);
coding!(zstd_loads, "_zstdLoads", zstd_dumps, "_zstdDumps", zstd);
