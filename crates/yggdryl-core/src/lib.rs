//! # yggdryl-core
//!
//! The dependency-light foundation crate for yggdryl, on which every other crate
//! and binding builds.
//!
//! The crate is built around an **Apache Arrow-centralized** data model.
//! Alongside the minimal [`hello`] / [`version`] entry points it hosts the
//! [`codec`] foundations ([`Encoder`] / [`Decoder`] and their element-generic
//! [`TypedEncoder`] / [`TypedDecoder`] extensions, plus the [`Converter`] /
//! [`TypedConverter`] representation converters) and the [`compression`]
//! specialisation ([`Compression`] and the concrete [`Gzip`] codec). The positioned
//! byte-IO contracts ([`IOBase`] / [`TypedIOBase`]), the byte/typed buffers, and the
//! wide integers now live one layer **down** in the `yggdryl-buffer` foundation crate;
//! core builds its codecs on them and re-exports them ([`ByteBuffer`], [`TypedCursor`],
//! [`i256`], …). Add further codec/compression types here as they land, one module per
//! concern, each re-exported at the crate root, following the rules in `CLAUDE.md`.

pub mod codec;
pub mod compression;

pub use codec::{
    BytesConverter, CastConverter, ConvertError, Converter, ConverterKind, DecodeError, Decoder,
    EncodeError, Encoder, IdentityConverter, PrimitiveType, StringConverter, TypedConverter,
    TypedDecoder, TypedEncoder, Utf8Converter,
};
pub use compression::{
    CompressIO, Compression, CompressionDecoder, CompressionEncoder, TypedCompressionDecoder,
    TypedCompressionEncoder,
};

/// The io + wide-integer foundation lives one layer **down** in `yggdryl-buffer`; core
/// re-exports it so `yggdryl_core::{ByteBuffer, IOBase, TypedCursor, i256, …}` keep
/// resolving for the codecs built on top and for downstream callers.
pub use yggdryl_buffer::{
    i256, i96, ByteBuffer, ByteCursor, ByteSlice, IOBase, IOCursor, IOSlice, IoError, IoPrimitive,
    TypedCursor, TypedIOBase, TypedIOCursor, TypedIOSlice, TypedSlice, Whence,
};

#[cfg(feature = "gzip")]
pub use compression::Gzip;
#[cfg(feature = "zstd")]
pub use compression::Zstd;

/// Re-export of the exact `arrow-buffer` the core is backed by, so callers construct
/// `Buffer`s against a matching version (see
/// [`ByteBuffer::from_arrow_byte_buffer`](yggdryl_buffer::ByteBuffer::from_arrow_byte_buffer)).
pub use arrow_buffer;

/// The crate version, as declared in `Cargo.toml`.
///
/// ```
/// assert_eq!(yggdryl_core::version(), env!("CARGO_PKG_VERSION"));
/// ```
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Prints `Hello, world!` to standard output — the minimal cross-language example,
/// surfaced identically from the Python and Node bindings.
///
/// ```
/// yggdryl_core::hello();
/// ```
pub fn hello() {
    println!("Hello, world!");
}
