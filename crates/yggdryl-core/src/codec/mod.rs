//! Codec foundations: the [`Encoder`] / [`Decoder`] byte-array contracts and
//! their element-generic [`TypedEncoder`] / [`TypedDecoder`] extensions, plus the
//! shared [`EncodeError`] / [`DecodeError`] types.
//!
//! These traits are the Rust-only base of the codec hierarchy; concrete codecs
//! (e.g. [`Gzip`](crate::Gzip)) implement them and are what the Python and Node
//! bindings expose. See [`compression`](crate::compression) for the compression
//! specialisation.

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
