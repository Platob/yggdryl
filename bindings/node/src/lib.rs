//! Node.js extension for **yggdryl**.
//!
//! Thin napi-rs wrappers over the Arrow-centric `yggdryl_core` types; each type
//! lives in its own module mirroring the Rust crate. All logic lives in the shared
//! core so the Node and Python bindings behave identically.

mod binary;
mod binary_scalar;
mod field;
mod string;
mod string_scalar;

use napi::Either;
use yggdryl_core::{AnyType, DataType};

pub(crate) use binary::Binary;
pub(crate) use string::Utf8;

/// Maps any core error to a JavaScript `Error`.
pub(crate) fn to_napi_err<E: std::fmt::Display>(err: E) -> napi::Error {
    napi::Error::from_reason(err.to_string())
}

/// Wraps a core [`AnyType`] in the matching JS data-type object (`Binary`/`Utf8`).
pub(crate) fn anytype_to_either(ty: &AnyType) -> Either<Binary, Utf8> {
    match ty {
        AnyType::Binary(inner) => Either::A(Binary { inner: *inner }),
        AnyType::Utf8(inner) => Either::B(Utf8 { inner: *inner }),
    }
}

/// Extracts a core [`AnyType`] from a JS data-type object (`Binary`/`Utf8`).
pub(crate) fn anytype_from_either(data_type: Either<&Binary, &Utf8>) -> AnyType {
    match data_type {
        Either::A(binary) => binary.inner.to_any(),
        Either::B(utf8) => utf8.inner.to_any(),
    }
}
