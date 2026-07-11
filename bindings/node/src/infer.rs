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
//! The element type is inferred from the **first non-null** element, and a `null` /
//! `undefined` element becomes that type's
//! [`default_value`](yggdryl_dtype::TypedDataType::default_value) (`0n` / `0` / `false`) —
//! so a nullable column materialises into a non-nullable buffer. An empty array, an
//! all-null array, a mixed array, an out-of-`i64`-range `bigint`, or an unsupported element
//! type throws an `Error` naming the explicit constructor to use.

use napi::bindgen_prelude::{Buffer, Either, Either4};
use napi::{JsBigInt, JsUnknown, ValueType};
use napi_derive::napi;

use yggdryl_dtype::{BooleanType, F64Type, I64Type, TypedDataType};

use crate::buffer::{BooleanBuffer, F64Buffer, I64Buffer};
use crate::io::ByteBuffer;

/// Builds a thrown JS `Error` from a message.
fn to_error(message: &str) -> napi::Error {
    napi::Error::from_reason(message.to_string())
}

/// Whether a JS value is `null` or `undefined` (a "null" element).
fn is_nullish(ty: ValueType) -> bool {
    matches!(ty, ValueType::Null | ValueType::Undefined)
}

/// Builds the typed buffer matching the runtime JS type of `values`, inferring the
/// element type so the caller need not name a buffer class. See the module docs for
/// the mapping. Ambiguous or unsupported input throws a guided error naming the
/// explicit constructor to reach for instead.
#[napi(namespace = "infer", js_name = "buffer")]
pub fn buffer(
    values: Either<Buffer, Vec<JsUnknown>>,
) -> napi::Result<Either4<I64Buffer, F64Buffer, BooleanBuffer, ByteBuffer>> {
    let items = match values {
        // A `Buffer` / `Uint8Array` is the byte buffer directly (the merged `U8Buffer`).
        Either::A(bytes) => {
            return Ok(Either4::D(ByteBuffer {
                inner: yggdryl_buffer::U8Buffer::from_vec(bytes.to_vec()),
            }));
        }
        Either::B(items) => items,
    };

    if items.is_empty() {
        return Err(to_error(
            "cannot infer the element type from an empty array; call an explicit \
             constructor, e.g. new yggdryl.buffer.I64Buffer([])",
        ));
    }

    // Infer the element type from the first non-null element; `null` / `undefined` become
    // the type's default value, so a nullable column materialises into a non-nullable buffer.
    let mut kind = None;
    for item in &items {
        let ty = item.get_type()?;
        if !is_nullish(ty) {
            kind = Some(ty);
            break;
        }
    }
    let kind = kind.ok_or_else(|| {
        to_error(
            "cannot infer the element type: every value is null; call an explicit \
             constructor, e.g. new yggdryl.buffer.I64Buffer([...])",
        )
    })?;

    match kind {
        ValueType::Boolean => {
            let default = BooleanType::new().default_value();
            let mut bits = Vec::with_capacity(items.len());
            for item in items {
                let ty = item.get_type()?;
                if is_nullish(ty) {
                    bits.push(default);
                } else if ty == ValueType::Boolean {
                    bits.push(item.coerce_to_bool()?.get_value()?);
                } else {
                    return Err(to_error(
                        "cannot infer a BooleanBuffer: every non-null element must be a \
                         boolean; use an explicit yggdryl.buffer constructor for a mixed array",
                    ));
                }
            }
            Ok(Either4::C(BooleanBuffer {
                inner: yggdryl_buffer::BooleanBuffer::from_bits(&bits),
            }))
        }
        ValueType::BigInt => {
            let default = I64Type::new().default_value();
            let mut ints = Vec::with_capacity(items.len());
            for item in items {
                let ty = item.get_type()?;
                if is_nullish(ty) {
                    ints.push(default);
                } else if ty == ValueType::BigInt {
                    let big = unsafe { item.cast::<JsBigInt>() };
                    let (value, lossless) = big.get_i64()?;
                    if !lossless {
                        return Err(to_error(
                            "cannot infer an I64Buffer: a bigint is out of the signed 64-bit \
                             range; use raw bytes or an explicit constructor for wider integers",
                        ));
                    }
                    ints.push(value);
                } else {
                    return Err(to_error(
                        "cannot infer an I64Buffer: every non-null element must be a bigint; \
                         use an explicit yggdryl.buffer constructor for a mixed array",
                    ));
                }
            }
            Ok(Either4::A(I64Buffer {
                inner: yggdryl_buffer::I64Buffer::from_vec(ints),
            }))
        }
        ValueType::Number => {
            let default = F64Type::new().default_value();
            let mut floats = Vec::with_capacity(items.len());
            for item in items {
                let ty = item.get_type()?;
                if is_nullish(ty) {
                    floats.push(default);
                } else if ty == ValueType::Number {
                    floats.push(item.coerce_to_number()?.get_double()?);
                } else {
                    return Err(to_error(
                        "cannot infer an F64Buffer: every non-null element must be a number; \
                         use an explicit yggdryl.buffer constructor for a mixed array",
                    ));
                }
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
