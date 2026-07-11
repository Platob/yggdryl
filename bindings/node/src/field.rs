//! The `yggdryl.field` namespace — Arrow primitive fields.
//!
//! Exposes one class per primitive field (`I8Field` … `F64Field`, `BooleanField`),
//! mirroring `yggdryl_field`. Each carries `name`, `nullable`, its `dataType` (a
//! [`yggdryl.dtype`](super::dtype) class), the byte codec, and value semantics. The
//! Arrow `toArrow` / `fromArrow` interop is **Rust-only** (an `arrow_schema` value does
//! not cross the FFI boundary), exactly as for the dtype layer.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

use yggdryl_field::{Field, TypedField};
use yggdryl_http::HeadersBased;

use crate::buffer::{headers_from_entries, headers_to_entries, HeaderEntry};

/// Maps a [`yggdryl_field::FieldError`] to a thrown JS `Error`.
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// Generates the napi wrapper class for one primitive field. `$dtype` is the matching
/// [`yggdryl.dtype`](super::dtype) class the field's `dataType` returns.
macro_rules! napi_primitive_field {
    ($( ($field:ident, $dtype:ident, $lit:literal) ),+ $(,)?) => {
        $(
            #[doc = concat!("A named, nullable `", $lit, "` field.")]
            #[napi(namespace = "field")]
            pub struct $field {
                pub(crate) inner: yggdryl_field::$field,
            }

            #[napi(namespace = "field")]
            impl $field {
                #[napi(constructor)]
                pub fn new(name: String, nullable: Option<bool>) -> Self {
                    Self { inner: yggdryl_field::$field::new(name, nullable.unwrap_or(false)) }
                }

                /// The field's name.
                #[napi(getter)]
                pub fn name(&self) -> String {
                    self.inner.name().to_string()
                }

                /// Whether the field's values may be null.
                #[napi(getter)]
                pub fn nullable(&self) -> bool {
                    self.inner.is_nullable()
                }

                /// The field's data type (a `yggdryl.dtype` class).
                #[napi(getter)]
                pub fn data_type(&self) -> crate::dtype::$dtype {
                    crate::dtype::$dtype { inner: TypedField::data_type(&self.inner) }
                }

                /// The field's headers as an `Array<{key, value}>`, or `null`.
                #[napi(getter)]
                pub fn headers(&self) -> Option<Vec<HeaderEntry>> {
                    headers_to_entries(self.inner.headers())
                }

                /// Returns a copy of this field with `headers` attached.
                #[napi]
                pub fn with_headers(&self, headers: Vec<HeaderEntry>) -> Self {
                    Self {
                        inner: self.inner.clone().with_headers(headers_from_entries(headers)),
                    }
                }

                /// The field serialised to bytes (a nullable flag + the UTF-8 name).
                #[napi]
                pub fn serialize_bytes(&self) -> Buffer {
                    self.inner.serialize_bytes().into()
                }

                /// Reconstructs the field from its serialised bytes.
                #[napi(factory)]
                pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
                    yggdryl_field::$field::deserialize_bytes(bytes.as_ref())
                        .map(|inner| Self { inner })
                        .map_err(to_error)
                }

                /// Content equality.
                #[napi]
                pub fn equals(&self, other: &$field) -> bool {
                    self.inner == other.inner
                }

                /// Java-style `i32` content hash.
                #[napi]
                pub fn hash_code(&self) -> i32 {
                    let mut hasher = DefaultHasher::new();
                    self.inner.hash(&mut hasher);
                    let hash = hasher.finish();
                    (hash as u32 ^ (hash >> 32) as u32) as i32
                }
            }
        )+
    };
}

napi_primitive_field! {
    (I8Field, I8Type, "int8"),
    (I16Field, I16Type, "int16"),
    (I32Field, I32Type, "int32"),
    (I64Field, I64Type, "int64"),
    (U8Field, U8Type, "uint8"),
    (U16Field, U16Type, "uint16"),
    (U32Field, U32Type, "uint32"),
    (U64Field, U64Type, "uint64"),
    (F32Field, F32Type, "float32"),
    (F64Field, F64Type, "float64"),
    (BooleanField, BooleanType, "boolean"),
}
