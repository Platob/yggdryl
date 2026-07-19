//! yggdryl core — a dependency-free byte / memory-access foundation.
//!
//! Two root modules: the [`io`] layer — the abstract byte-access contract
//! ([`io::memory::IOBase`]) with its sources and the cross-cutting value types at the [`io`]
//! root — and the [`uri`] family that **addresses** those sources. New features are added here
//! first, in the Rust core, then mirrored thinly in the Python and Node extensions.

/// The io layer: byte / memory access and the shared value types.
pub mod io;

/// The primitive element data types ([`datatype_id::DataTypeId`]) a byte region is interpreted as.
pub mod datatype_id;

/// The typed data serialization layer over the io byte contract — `DataType` / `Encoder` /
/// `Decoder` / `Scalar` / `Serie` / `Field`, with the fixed/variable × bit/byte type families.
pub mod typed;

/// The Apache Arrow interop bridge (feature `arrow`) — every type ↔ its closest Arrow equivalent.
#[cfg(feature = "arrow")]
pub mod arrow;

/// The project's one metadata map (ordered, case-insensitive, multi-value byte headers).
pub mod headers;

/// One media type (`type/subtype`) with its extensions + magic bytes, and the registry that
/// resolves it from a mime string, file name, extension, or magic bytes.
pub mod mimetype;

/// An ordered list of [`mimetype::MimeType`]s — the layered type description of a resource.
pub mod mediatype;

/// The compression codec contract ([`compression::Compression`]) — byte + stream compress /
/// decompress; the concrete Gzip/Zlib/Zstd/Lzma codecs are behind the `compression` feature.
pub mod compression;

/// The URI / URL family (RFC 3986) that addresses io sources, parsed from scratch.
pub mod uri;

/// The crate version string (from `Cargo.toml`), e.g. `"0.1.1"`.
///
/// This is the minimal end-to-end example: the same value is exposed by the Python and
/// Node extensions.
///
/// ```
/// assert_eq!(yggdryl_core::version(), env!("CARGO_PKG_VERSION"));
/// ```
pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_the_crate_version() {
        assert_eq!(super::version(), env!("CARGO_PKG_VERSION"));
        assert!(!super::version().is_empty());
    }
}
