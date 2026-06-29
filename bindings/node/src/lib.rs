//! Node.js extension for **yggdryl**.
//!
//! Thin napi-rs wrappers over the Arrow-centric `yggdryl_core` types; each type
//! lives in its own module mirroring the Rust crate. All logic lives in the shared
//! core so the Node and Python bindings behave identically.

mod binary;
mod binary_type;
mod field;
mod string;
mod whence;

use napi::Either;
use yggdryl_core::{AnyType, DataType};

pub(crate) use binary_type::BinaryType;
pub(crate) use string::Utf8;
pub(crate) use whence::Whence;

/// Maps any core error to a JavaScript `Error`.
pub(crate) fn to_napi_err<E: std::fmt::Display>(err: E) -> napi::Error {
    napi::Error::from_reason(err.to_string())
}

/// Wraps a core [`AnyType`] in the matching JS data-type object.
pub(crate) fn anytype_to_either(ty: &AnyType) -> Either<BinaryType, Utf8> {
    match ty {
        AnyType::Binary(inner) => Either::A(BinaryType { inner: *inner }),
        AnyType::Utf8(inner) => Either::B(Utf8 { inner: *inner }),
    }
}

/// Extracts a core [`AnyType`] from a JS data-type object (`BinaryType`/`Utf8`).
pub(crate) fn anytype_from_either(data_type: Either<&BinaryType, &Utf8>) -> AnyType {
    match data_type {
        Either::A(binary) => binary.inner.to_any(),
        Either::B(utf8) => utf8.inner.to_any(),
    }
}
