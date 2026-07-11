//! The `yggdryl.infer` namespace — runtime type inference for the interpreted API.
//!
//! A convenience layer, **binding-only by design** (`CLAUDE.md` rule 13): it has no
//! `yggdryl-core` counterpart because the Rust core reaches its typed buffers through
//! explicit generics, while the dynamically-typed JS API can read the runtime type of
//! a value and pick the matching buffer for the caller. Everything here is sugar over
//! the explicit constructors in [`crate::buffer`] — reach for those directly when a
//! value is ambiguous or out of range.
//!
//! `buffer(values)` maps a JS value to a buffer as follows — identical in intent to
//! the Python binding, adapted to JS types (there is no native integer type: an
//! integer buffer needs `bigint` elements, since a `number` is always a float):
//!
//! | JS value                        | Result buffer   |
//! |---------------------------------|-----------------|
//! | `Buffer` / `Uint8Array`         | `U8Buffer`      |
//! | array of `boolean`              | `BooleanBuffer` |
//! | array of `bigint` (i64 range)   | `I64Buffer`     |
//! | array of `number`               | `F64Buffer`     |
//!
//! An empty array, a mixed array, an out-of-`i64`-range `bigint`, or an unsupported
//! element type throws an `Error` naming the explicit constructor to use.

use napi::bindgen_prelude::{Buffer, Either, Either4};
use napi::{JsBigInt, JsUnknown, ValueType};
use napi_derive::napi;

use crate::buffer::{BooleanBuffer, F64Buffer, I64Buffer, U8Buffer};

/// Builds a thrown JS `Error` from a message.
fn to_error(message: &str) -> napi::Error {
    napi::Error::from_reason(message.to_string())
}

/// Builds the typed buffer matching the runtime JS type of `values`, inferring the
/// element type so the caller need not name a buffer class. See the module docs for
/// the mapping. Ambiguous or unsupported input throws a guided error naming the
/// explicit constructor to reach for instead.
#[napi(namespace = "infer", js_name = "buffer")]
pub fn buffer(
    values: Either<Buffer, Vec<JsUnknown>>,
) -> napi::Result<Either4<I64Buffer, F64Buffer, BooleanBuffer, U8Buffer>> {
    let items = match values {
        // A `Buffer` / `Uint8Array` is the byte buffer directly.
        Either::A(bytes) => {
            return Ok(Either4::D(U8Buffer {
                inner: yggdryl_buffer::U8Buffer::from_vec(bytes.to_vec()),
            }));
        }
        Either::B(items) => items,
    };

    let first = items.first().ok_or_else(|| {
        to_error(
            "cannot infer the element type from an empty array; call an explicit \
             constructor, e.g. new yggdryl.buffer.I64Buffer([])",
        )
    })?;

    match first.get_type()? {
        ValueType::Boolean => {
            let mut bits = Vec::with_capacity(items.len());
            for item in items {
                if item.get_type()? != ValueType::Boolean {
                    return Err(to_error(
                        "cannot infer a BooleanBuffer: every element must be a boolean; \
                         use an explicit yggdryl.buffer constructor for a mixed array",
                    ));
                }
                bits.push(item.coerce_to_bool()?.get_value()?);
            }
            Ok(Either4::C(BooleanBuffer {
                inner: yggdryl_buffer::BooleanBuffer::from_bits(&bits),
            }))
        }
        ValueType::BigInt => {
            let mut ints = Vec::with_capacity(items.len());
            for item in items {
                if item.get_type()? != ValueType::BigInt {
                    return Err(to_error(
                        "cannot infer an I64Buffer: every element must be a bigint; \
                         use an explicit yggdryl.buffer constructor for a mixed array",
                    ));
                }
                let big = unsafe { item.cast::<JsBigInt>() };
                let (value, lossless) = big.get_i64()?;
                if !lossless {
                    return Err(to_error(
                        "cannot infer an I64Buffer: a bigint is out of the signed 64-bit \
                         range; use raw bytes or an explicit constructor for wider integers",
                    ));
                }
                ints.push(value);
            }
            Ok(Either4::A(I64Buffer {
                inner: yggdryl_buffer::I64Buffer::from_vec(ints),
            }))
        }
        ValueType::Number => {
            let mut floats = Vec::with_capacity(items.len());
            for item in items {
                if item.get_type()? != ValueType::Number {
                    return Err(to_error(
                        "cannot infer an F64Buffer: every element must be a number; \
                         use an explicit yggdryl.buffer constructor for a mixed array",
                    ));
                }
                floats.push(item.coerce_to_number()?.get_double()?);
            }
            Ok(Either4::B(F64Buffer {
                inner: yggdryl_buffer::F64Buffer::from_vec(floats),
            }))
        }
        _ => Err(to_error(
            "cannot infer a buffer: supported element types are boolean, bigint, and \
             number (or pass a Buffer / Uint8Array for a U8Buffer)",
        )),
    }
}
