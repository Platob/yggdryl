//! Node.js extension for **yggdryl**.
//!
//! Thin napi-rs wrappers over the Arrow-centric `yggdryl_core` types; each type
//! lives in its own module mirroring the Rust crate. All logic lives in the shared
//! core so the Node and Python bindings behave identically.

mod binary;
mod binary_type;
mod charset;
mod field;
mod jsonparams;
mod utf8;
mod utf8_type;
mod whence;

use napi::Either;
use yggdryl_dtype::{AnyType, DataType};
use yggdryl_scalar::AnyScalar;

pub(crate) use binary::Binary;
pub(crate) use binary_type::BinaryType;
pub(crate) use charset::Charset;
pub(crate) use utf8::Utf8;
pub(crate) use utf8_type::Utf8Type;
pub(crate) use whence::Whence;

// Re-export the module-level JSON-params functions so plain `cargo`/`clippy` does
// not flag them unused; napi exports them to JS regardless.
pub use jsonparams::{json_params, reset_json_params, set_json_params};

/// Maps any core error to a JavaScript `Error`.
pub(crate) fn to_napi_err<E: std::fmt::Display>(err: E) -> napi::Error {
    napi::Error::from_reason(err.to_string())
}

/// Wraps a core [`AnyType`] in the matching JS data-type object.
pub(crate) fn anytype_to_either(ty: &AnyType) -> Either<BinaryType, Utf8Type> {
    match ty {
        AnyType::Binary(inner) => Either::A(BinaryType { inner: *inner }),
        AnyType::Utf8(inner) => Either::B(Utf8Type { inner: *inner }),
    }
}

/// Extracts a core [`AnyType`] from a JS data-type object.
pub(crate) fn anytype_from_either(data_type: Either<&BinaryType, &Utf8Type>) -> AnyType {
    match data_type {
        Either::A(binary) => binary.inner.to_any(),
        Either::B(utf8) => utf8.inner.to_any(),
    }
}

/// Wraps a core [`AnyScalar`] in the matching JS scalar value object.
pub(crate) fn anyscalar_to_either(scalar: AnyScalar) -> Either<Binary, Utf8> {
    match scalar {
        AnyScalar::Binary(inner) => Either::A(Binary { inner }),
        AnyScalar::Utf8(inner) => Either::B(Utf8 { inner }),
    }
}
