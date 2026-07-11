//! **yggdryl-compression** — lossless compression codecs over the core codec base.
//!
//! The [`Compression`] / [`CompressionEncoder`] / [`CompressionDecoder`] contracts (and
//! their element-generic [`TypedCompressionEncoder`] / [`TypedCompressionDecoder`]
//! extensions) specialise the [`Encoder`] / [`Decoder`] base traits (from `yggdryl-core`)
//! to lossless compression, and [`CompressIO`] streams a whole IO resource through a codec
//! (from `yggdryl-buffer`). The concrete codecs are [`Gzip`] and [`Zstd`], each behind an
//! enabled-by-default cargo feature (`gzip` / `zstd`).
//!
//! The traits are Rust-only — generics and marker traits do not cross the FFI boundary —
//! so the Python and Node bindings expose only the concrete codecs, a deliberate,
//! documented omission per the replication rule in `CLAUDE.md`.
//!
//! For convenience (and so examples read from one import path) the base traits this crate
//! builds on are re-exported here: [`Encoder`] / [`Decoder`] and the io types
//! [`ByteBuffer`] / [`IOBase`] / [`Whence`].

// One-file-per-type (see `CLAUDE.md`) puts the `Compression` trait in `compression.rs`;
// the resulting module/type name clash is the convention, not an accident.
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

// The base this crate builds on, re-exported so callers reach it through one crate.
pub use yggdryl_buffer::{ByteBuffer, ByteCursor, IOBase, IoError, Whence};
pub use yggdryl_core::{DecodeError, Decoder, EncodeError, Encoder, TypedDecoder, TypedEncoder};
