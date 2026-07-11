//! Codec foundations: the [`Encoder`] / [`Decoder`] byte-array contracts, their
//! element-generic [`TypedEncoder`] / [`TypedDecoder`] extensions, the shared
//! [`EncodeError`] / [`DecodeError`] types, and the [`PrimitiveType`] tag.
//!
//! These are the Rust-only, byte-slice-based **base** of the codec hierarchy — no io
//! dependency. Concrete codecs implement them in the crates above: the compression
//! codecs (`Gzip` / `Zstd`) in `yggdryl-compression`, the representation converters (and
//! the `PrimitiveType` tag) in `yggdryl-converter`.

mod decode_error;
mod decoder;
mod encode_error;
mod encoder;
mod typed_decoder;
mod typed_encoder;

pub use decode_error::DecodeError;
pub use decoder::Decoder;
pub use encode_error::EncodeError;
pub use encoder::Encoder;
pub use typed_decoder::TypedDecoder;
pub use typed_encoder::TypedEncoder;
