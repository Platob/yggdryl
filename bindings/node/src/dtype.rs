//! The `yggdryl.dtype` namespace — Arrow primitive data types.
//!
//! Exposes one class per primitive data type (`I8Type` … `F64Type`, plus the
//! bit-packed `BooleanType` and the sui-generis `NullType`), mirroring `yggdryl_dtype`.
//! Each carries the type-identity
//! surface — `name`, `byteWidth`, `primitiveTag`, the byte codec
//! (`serializeBytes` / `deserializeBytes`), and value semantics (`equals` /
//! `hashCode`). The Arrow `toArrow` / `fromArrow` interop is **Rust-only** (an
//! `arrow_schema` value does not cross the FFI boundary), exactly as for the buffers'
//! Arrow interop.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

use yggdryl_dtype::DataType;

/// Maps a [`yggdryl_dtype::DTypeError`] to a thrown JS `Error`.
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// Generates the napi wrapper class for one primitive data type. `$tag` is the core
/// [`PrimitiveType`](yggdryl_converter::PrimitiveType) tag name (e.g. `Some("i64")`), or
/// `None` for `Boolean`.
macro_rules! napi_primitive_dtype {
    ($( ($name:ident, $lit:literal, $tag:expr) ),+ $(,)?) => {
        $(
            #[doc = concat!("The `", $lit, "` primitive data type.")]
            #[napi(namespace = "dtype")]
            pub struct $name {
                pub(crate) inner: yggdryl_dtype::$name,
            }

            #[napi(namespace = "dtype")]
            impl $name {
                #[napi(constructor)]
                #[allow(clippy::new_without_default)]
                pub fn new() -> Self {
                    Self { inner: yggdryl_dtype::$name::new() }
                }

                /// The canonical lower-snake type name, e.g. `"int64"`.
                #[napi(getter)]
                pub fn name(&self) -> String {
                    self.inner.name().to_string()
                }

                /// The fixed value width in bytes, or `null` for `boolean` (bit-packed).
                #[napi(getter)]
                pub fn byte_width(&self) -> Option<u32> {
                    self.inner.byte_width().map(|w| w as u32)
                }

                /// The core `PrimitiveType` tag name (e.g. `"i64"`), or `null` for
                /// `boolean`.
                #[napi(getter)]
                pub fn primitive_tag(&self) -> Option<String> {
                    $tag.map(str::to_string)
                }

                /// The type's (empty) serialised payload.
                #[napi]
                pub fn serialize_bytes(&self) -> Buffer {
                    self.inner.serialize_bytes().into()
                }

                /// Reconstructs the type from its serialised payload (must be empty).
                #[napi(factory)]
                pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
                    yggdryl_dtype::$name::deserialize_bytes(bytes.as_ref())
                        .map(|inner| Self { inner })
                        .map_err(to_error)
                }

                /// Content equality.
                #[napi]
                pub fn equals(&self, other: &$name) -> bool {
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

napi_primitive_dtype! {
    (I8Type, "int8", Some("i8")),
    (I16Type, "int16", Some("i16")),
    (I32Type, "int32", Some("i32")),
    (I64Type, "int64", Some("i64")),
    (U8Type, "uint8", Some("u8")),
    (U16Type, "uint16", Some("u16")),
    (U32Type, "uint32", Some("u32")),
    (U64Type, "uint64", Some("u64")),
    (F32Type, "float32", Some("f32")),
    (F64Type, "float64", Some("f64")),
    (BooleanType, "boolean", None),
    (NullType, "null", None),
}
