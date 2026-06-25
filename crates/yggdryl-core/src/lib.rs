//! # yggdryl-core
//!
//! The consolidated foundations of the **yggdryl** project. One crate now holds
//! what used to be five:
//!
//! - the [`Mapping`] / [`Params`] component maps, plus URL-safe percent-encoding
//!   ([`percent_encode`] / [`percent_decode`]) and the component helpers (each
//!   value type pairs its `from_str` / `from_mapping` parsers with inherent
//!   `to_str` / `to_mapping` renderers);
//! - the [`Version`] (`major.minor.patch`) value type;
//! - the [`MimeType`] enum (backed by a mutable global registry) and the
//!   [`MediaType`] extension stack;
//! - the [`Uri`] / [`Url`] value types;
//! - the byte-IO foundation — the [`Io`] handle trait, [`BytesIO`], [`LocalPath`],
//!   the [`Codec`] / [`Frames`] value coders, and the [`from_str`] /
//!   [`register_scheme`] factory;
//! - streamed byte [`Compression`] (gzip / Zstandard / Snappy) over any `Io`
//!   handle, plus the [`CompressIo`] extension trait.
//!
//! The blocking HTTP client lives in the separate `yggdryl-http` crate, which
//! depends on this one.

/// Emits a `log` event when the `log` feature is enabled, and expands to nothing
/// otherwise (so the crate stays dependency-free by default and pays no runtime
/// cost). Shared by every submodule via `pub(crate) use log_event`.
macro_rules! log_event {
    ($level:ident, $($arg:tt)+) => {{
        #[cfg(feature = "log")]
        log::$level!($($arg)+);
    }};
}
pub(crate) use log_event;

mod encoding;
mod mapping;
mod version;

mod media;
mod url;

mod compression;
mod io;

pub use encoding::{
    encode_component, percent_decode, percent_encode, validate_percent_encoding, EncodingError,
};
pub use mapping::{Mapping, Params};
pub use version::{Version, VersionError};

pub use media::{MediaError, MediaType, MimeType, Signature};

pub use url::{JoinInput, Uri, UriError, Url, UrlError};

pub use io::{
    copy, from_str, from_uri, from_url, register_scheme, BytesIO, Codec, Frames, Io, IoError,
    IoStats, Kind, LocalPath, Mode, Path, RemotePath, SchemeOpener, Stream, Whence, STREAM_CHUNK,
};

pub use compression::{CompressIo, Compression, Decoder, Encoder};
