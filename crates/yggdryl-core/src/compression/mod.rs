//! Compression codecs: the [`Compression`] / [`CompressionEncoder`] /
//! [`CompressionDecoder`] contracts (and their element-generic
//! [`TypedCompressionEncoder`] / [`TypedCompressionDecoder`] extensions) that
//! specialise the [`codec`](crate::codec) traits to lossless compression, plus the
//! concrete [`Gzip`] codec.
//!
//! The traits are Rust-only — generics and marker traits do not cross the FFI
//! boundary — so the Python and Node bindings expose only the concrete codecs
//! (currently `Gzip`), a deliberate, documented omission per the replication rule
//! in `CLAUDE.md`. `Gzip` lives behind the off-by-default-in-consumers but
//! enabled-by-default `gzip` cargo feature.

// One-file-per-type (see `CLAUDE.md`) puts the `Compression` trait in
// `compression.rs` inside this `compression` module; the resulting name clash is
// the convention, not an accident.
mod compress_io;
#[allow(clippy::module_inception)]
mod compression;
mod compression_decoder;
mod compression_encoder;
mod typed_compression_decoder;
mod typed_compression_encoder;

#[cfg(feature = "gzip")]
mod gzip;
#[cfg(feature = "zstd")]
mod zstd;

pub use compress_io::CompressIO;
pub use compression::Compression;
pub use compression_decoder::CompressionDecoder;
pub use compression_encoder::CompressionEncoder;
pub use typed_compression_decoder::TypedCompressionDecoder;
pub use typed_compression_encoder::TypedCompressionEncoder;

#[cfg(feature = "gzip")]
pub use gzip::Gzip;
#[cfg(feature = "zstd")]
pub use zstd::Zstd;
