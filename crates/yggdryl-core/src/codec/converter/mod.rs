//! Representation converters: the [`Converter`] byte-array contract and its
//! typed [`TypedConverter<S, T>`] extension, plus the shared [`ConvertError`] and the
//! concrete converters.
//!
//! A converter maps a **source** representation to a **target** one and back:
//! [`IdentityConverter`] passes through, [`CastConverter`] casts between numeric
//! primitives, [`StringConverter`] parses/renders numbers flexibly,
//! [`BytesConverter`] moves a value to/from its little-endian bytes, and
//! [`Utf8Converter`] moves a string to/from its UTF-8 bytes. The generic
//! [`TypedConverter<S, T>`] is Rust-only (two type parameters); the bindings expose
//! the concrete converters and the byte-level [`Converter`] surface.

mod bytes_converter;
mod cast_converter;
mod convert_error;
// The base trait `Converter` lives in `converter.rs`, named for the type it holds
// (rule 1) — the same-name-as-parent is the convention, not an accident.
#[allow(clippy::module_inception)]
mod converter;
mod converter_kind;
mod identity_converter;
mod primitive_type;
mod string_converter;
mod typed_converter;
mod utf8_converter;

pub use bytes_converter::BytesConverter;
pub use cast_converter::CastConverter;
pub use convert_error::ConvertError;
pub use converter::Converter;
pub use converter_kind::ConverterKind;
pub use identity_converter::IdentityConverter;
pub use primitive_type::PrimitiveType;
pub use string_converter::StringConverter;
pub use typed_converter::TypedConverter;
pub use utf8_converter::Utf8Converter;
