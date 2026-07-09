//! # yggdryl-core
//!
//! The dependency-light foundation crate for yggdryl, on which every other crate
//! and binding builds.
//!
//! The crate is built around an **Apache Arrow-centralized** data model.
//! Alongside the minimal [`hello`] / [`version`] entry points it hosts the
//! [`codec`] foundations ([`Encoder`] / [`Decoder`] and their element-generic
//! [`TypedEncoder`] / [`TypedDecoder`] extensions, plus the [`Converter`] /
//! [`TypedConverter`] representation converters), the [`compression`]
//! specialisation ([`Compression`] and the concrete [`Gzip`] codec), the
//! [`io`] positioned byte-IO contracts ([`IOBase`] / [`TypedIOBase`]), and the
//! [`buffer`] typed native-type buffers ([`I64Buffer`] … [`BooleanBuffer`]). Add
//! further foundational types here as they land, one module per concern, each
//! re-exported at the crate root, following the rules in `CLAUDE.md`.

pub mod buffer;
pub mod codec;
pub mod compression;
pub mod int;
pub mod io;

pub use buffer::{
    BooleanBuffer, BufferError, F32Buffer, F64Buffer, I16Buffer, I32Buffer, I64Buffer, I8Buffer,
    U16Buffer, U32Buffer, U64Buffer, U8Buffer,
};
pub use codec::{
    BytesConverter, CastConverter, ConvertError, Converter, DecodeError, Decoder, EncodeError,
    Encoder, IdentityConverter, PrimitiveType, StringConverter, TypedConverter, TypedDecoder,
    TypedEncoder, Utf8Converter,
};
pub use compression::{
    CompressIO, Compression, CompressionDecoder, CompressionEncoder, TypedCompressionDecoder,
    TypedCompressionEncoder,
};
pub use int::{i256, i96};
pub use io::{
    ByteBuffer, ByteCursor, ByteSlice, IOBase, IOCursor, IOSlice, IoError, IoPrimitive,
    TypedCursor, TypedIOBase, TypedIOCursor, TypedIOSlice, TypedSlice, Whence,
};

#[cfg(feature = "gzip")]
pub use compression::Gzip;
#[cfg(feature = "zstd")]
pub use compression::Zstd;

/// Re-export of the exact `arrow-buffer` the core is backed by, so callers construct
/// `Buffer`s against a matching version (see
/// [`ByteBuffer::from_arrow_byte_buffer`](io::ByteBuffer::from_arrow_byte_buffer)).
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
