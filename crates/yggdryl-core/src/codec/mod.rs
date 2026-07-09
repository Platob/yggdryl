//! Codec foundations: the [`Encoder`] / [`Decoder`] byte-array contracts and
//! their element-generic [`TypedEncoder`] / [`TypedDecoder`] extensions, the
//! representation [`converter`] family ([`Converter`] / [`TypedConverter`] and the
//! concrete converters), plus the shared [`EncodeError`] / [`DecodeError`] /
//! [`ConvertError`] types.
//!
//! These traits are the Rust-only base of the codec hierarchy; concrete codecs
//! (e.g. [`Gzip`](crate::Gzip)) and converters implement them and are what the Python
//! and Node bindings expose. See [`compression`](crate::compression) for the
//! compression specialisation.

pub mod converter;

mod decode_error;
mod decoder;
mod encode_error;
mod encoder;
mod typed_decoder;
mod typed_encoder;

pub use converter::{
    BytesConverter, CastConverter, ConvertError, Converter, IdentityConverter, PrimitiveType,
    StringConverter, TypedConverter, Utf8Converter,
};
pub use decode_error::DecodeError;
pub use decoder::Decoder;
pub use encode_error::EncodeError;
pub use encoder::Encoder;
pub use typed_decoder::TypedDecoder;
pub use typed_encoder::TypedEncoder;
