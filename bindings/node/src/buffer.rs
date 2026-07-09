//! The `yggdryl.buffer` namespace — typed native-type buffers.
//!
//! Exposes one immutable buffer class per native primitive ([`I8Buffer`] …
//! [`F64Buffer`]) plus the bit-packed [`BooleanBuffer`], mirroring
//! `yggdryl_core::buffer`. Two Node-specific idioms, as on the IO cursor: `U64Buffer`
//! is **omitted** (napi has no native `u64` scalar — use `I64Buffer` or raw bytes),
//! and `F32Buffer` marshals its values over an `f64` JS boundary. `i64` values and the
//! `length`/`len` counts follow the same JS mapping as `yggdryl.io`. The Arrow
//! `from_arrow` / `to_arrow` interop is Rust-only (an `arrow_buffer` value does not
//! cross the FFI boundary).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

use crate::io::{
    ByteBuffer, ByteCursor, F32Cursor, F32Slice, F64Cursor, F64Slice, I16Cursor, I16Slice,
    I32Cursor, I32Slice, I64Cursor, I64Slice, I8Cursor, I8Slice, U16Cursor, U16Slice, U32Cursor,
    U32Slice, U8Cursor, U8Slice,
};

/// Maps a core error to a thrown JS `Error`.
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// Generates the napi wrapper class for one numeric buffer type whose element maps
/// to a native napi scalar.
macro_rules! napi_buffer {
    ($( ($name:ident, $ty:ty, $cursor:ident, $slice:ident) ),+ $(,)?) => {
        $(
            #[doc = concat!("An immutable, cheaply-shared contiguous buffer of `", stringify!($ty), "` values.")]
            #[napi(namespace = "buffer")]
            pub struct $name {
                pub(crate) inner: yggdryl_core::$name,
            }

            #[napi(namespace = "buffer")]
            impl $name {
                /// Creates a buffer, optionally holding a copy of `values`.
                #[napi(constructor)]
                pub fn new(values: Option<Vec<$ty>>) -> Self {
                    let inner = match values {
                        Some(values) => yggdryl_core::$name::from_vec(values),
                        None => yggdryl_core::$name::new(),
                    };
                    Self { inner }
                }

                /// The number of values held.
                #[napi(getter)]
                pub fn length(&self) -> i64 {
                    self.inner.len() as i64
                }

                /// The number of values held.
                #[napi]
                pub fn len(&self) -> i64 {
                    self.inner.len() as i64
                }

                /// Whether the buffer holds no values.
                #[napi]
                pub fn is_empty(&self) -> bool {
                    self.inner.is_empty()
                }

                /// The value at `index`, or `null` if out of bounds.
                #[napi]
                pub fn get(&self, index: u32) -> Option<$ty> {
                    self.inner.get(index as usize)
                }

                /// An array of the values.
                #[napi]
                pub fn to_array(&self) -> Vec<$ty> {
                    self.inner.to_vec()
                }

                /// The values' little-endian bytes.
                #[napi]
                pub fn as_bytes(&self) -> Buffer {
                    self.inner.as_bytes().to_vec().into()
                }

                /// Serialises the values to their little-endian bytes.
                #[napi]
                pub fn serialize_bytes(&self) -> Buffer {
                    self.inner.serialize_bytes().into()
                }

                #[doc = concat!("Reconstructs a buffer from little-endian `", stringify!($ty), "` bytes.")]
                #[napi(factory)]
                pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
                    yggdryl_core::$name::deserialize_bytes(bytes.as_ref())
                        .map(|inner| Self { inner })
                        .map_err(to_error)
                }

                /// Freezes the values into a `ByteBuffer` of their little-endian bytes.
                #[napi]
                pub fn to_byte_buffer(&self) -> ByteBuffer {
                    ByteBuffer {
                        inner: self.inner.to_byte_buffer(),
                    }
                }

                /// Decodes a `ByteBuffer` of little-endian bytes into a buffer.
                #[napi(factory)]
                pub fn from_byte_buffer(buffer: &ByteBuffer) -> napi::Result<Self> {
                    yggdryl_core::$name::from_byte_buffer(&buffer.inner)
                        .map(|inner| Self { inner })
                        .map_err(to_error)
                }

                /// Opens a `ByteCursor` over the values' bytes.
                #[napi]
                pub fn byte_cursor(&self) -> ByteCursor {
                    ByteCursor {
                        inner: self.inner.byte_cursor(),
                    }
                }

                #[doc = concat!("Opens a `", stringify!($cursor), "` over the values (native `", stringify!($ty), "` units).")]
                #[napi]
                pub fn cursor(&self) -> $cursor {
                    $cursor {
                        inner: self.inner.cursor(),
                    }
                }

                #[doc = concat!("Opens a `", stringify!($slice), "` over the `offset..offset+len` window of values (in `", stringify!($ty), "` units).")]
                #[napi]
                pub fn slice(&self, offset: u32, len: u32) -> $slice {
                    $slice {
                        inner: self.inner.slice(offset as usize, len as usize),
                    }
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

// `U64Buffer` is omitted (no native napi `u64` scalar); `F32Buffer` is defined below
// over an `f64` boundary.
napi_buffer!(
    (I8Buffer, i8, I8Cursor, I8Slice),
    (I16Buffer, i16, I16Cursor, I16Slice),
    (I32Buffer, i32, I32Cursor, I32Slice),
    (I64Buffer, i64, I64Cursor, I64Slice),
    (U8Buffer, u8, U8Cursor, U8Slice),
    (U16Buffer, u16, U16Cursor, U16Slice),
    (U32Buffer, u32, U32Cursor, U32Slice),
    (F64Buffer, f64, F64Cursor, F64Slice),
);

/// An immutable, cheaply-shared contiguous buffer of `f32` values (marshalled over an
/// `f64` JS boundary).
#[napi(namespace = "buffer")]
pub struct F32Buffer {
    pub(crate) inner: yggdryl_core::F32Buffer,
}

#[napi(namespace = "buffer")]
impl F32Buffer {
    /// Creates a buffer, optionally holding `values` narrowed to `f32`.
    #[napi(constructor)]
    pub fn new(values: Option<Vec<f64>>) -> Self {
        let inner = match values {
            Some(values) => {
                yggdryl_core::F32Buffer::from_vec(values.into_iter().map(|v| v as f32).collect())
            }
            None => yggdryl_core::F32Buffer::new(),
        };
        Self { inner }
    }

    /// The number of values held.
    #[napi(getter)]
    pub fn length(&self) -> i64 {
        self.inner.len() as i64
    }

    /// The number of values held.
    #[napi]
    pub fn len(&self) -> i64 {
        self.inner.len() as i64
    }

    /// Whether the buffer holds no values.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The value at `index` (widened to a JS number), or `null` if out of bounds.
    #[napi]
    pub fn get(&self, index: u32) -> Option<f64> {
        self.inner.get(index as usize).map(f64::from)
    }

    /// An array of the values (widened to JS numbers).
    #[napi]
    pub fn to_array(&self) -> Vec<f64> {
        self.inner.to_vec().into_iter().map(f64::from).collect()
    }

    /// The values' little-endian bytes.
    #[napi]
    pub fn as_bytes(&self) -> Buffer {
        self.inner.as_bytes().to_vec().into()
    }

    /// Serialises the values to their little-endian bytes.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a buffer from little-endian `f32` bytes.
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        yggdryl_core::F32Buffer::deserialize_bytes(bytes.as_ref())
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Freezes the values into a `ByteBuffer` of their little-endian bytes.
    #[napi]
    pub fn to_byte_buffer(&self) -> ByteBuffer {
        ByteBuffer {
            inner: self.inner.to_byte_buffer(),
        }
    }

    /// Decodes a `ByteBuffer` of little-endian bytes into a buffer.
    #[napi(factory)]
    pub fn from_byte_buffer(buffer: &ByteBuffer) -> napi::Result<Self> {
        yggdryl_core::F32Buffer::from_byte_buffer(&buffer.inner)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Opens a `ByteCursor` over the values' bytes.
    #[napi]
    pub fn byte_cursor(&self) -> ByteCursor {
        ByteCursor {
            inner: self.inner.byte_cursor(),
        }
    }

    /// Opens an `F32Cursor` over the values (native `f32` units, marshalled over `f64`).
    #[napi]
    pub fn cursor(&self) -> F32Cursor {
        F32Cursor {
            inner: self.inner.cursor(),
        }
    }

    /// Opens an `F32Slice` over the `offset..offset+len` window of values (in `f32` units).
    #[napi]
    pub fn slice(&self, offset: u32, len: u32) -> F32Slice {
        F32Slice {
            inner: self.inner.slice(offset as usize, len as usize),
        }
    }

    /// Content equality.
    #[napi]
    pub fn equals(&self, other: &F32Buffer) -> bool {
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

/// An immutable, bit-packed (LSB-first) buffer of `bool` values.
#[napi(namespace = "buffer")]
pub struct BooleanBuffer {
    pub(crate) inner: yggdryl_core::BooleanBuffer,
}

#[napi(namespace = "buffer")]
impl BooleanBuffer {
    /// Creates a buffer, optionally packing `values`.
    #[napi(constructor)]
    pub fn new(values: Option<Vec<bool>>) -> Self {
        let inner = match values {
            Some(values) => yggdryl_core::BooleanBuffer::from_bits(&values),
            None => yggdryl_core::BooleanBuffer::new(),
        };
        Self { inner }
    }

    /// Wraps `bytes` (LSB-first packed bits) as a buffer of `len` bits.
    #[napi(factory)]
    pub fn from_bytes(bytes: Buffer, len: u32) -> napi::Result<Self> {
        yggdryl_core::BooleanBuffer::from_bytes(bytes.as_ref(), len as usize)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// The number of bits held.
    #[napi(getter)]
    pub fn length(&self) -> i64 {
        self.inner.len() as i64
    }

    /// The number of bits held.
    #[napi]
    pub fn len(&self) -> i64 {
        self.inner.len() as i64
    }

    /// Whether the buffer holds no bits.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The boolean at `index`, or `null` if out of bounds.
    #[napi]
    pub fn get(&self, index: u32) -> Option<bool> {
        self.inner.get(index as usize)
    }

    /// An array of the boolean values.
    #[napi]
    pub fn to_array(&self) -> Vec<bool> {
        self.inner.to_vec()
    }

    /// The packed bytes (LSB-first).
    #[napi]
    pub fn as_bytes(&self) -> Buffer {
        self.inner.as_bytes().to_vec().into()
    }

    /// The number of set (`true`) bits.
    #[napi]
    pub fn count_set_bits(&self) -> i64 {
        self.inner.count_set_bits() as i64
    }

    /// Serialises to an 8-byte little-endian bit length followed by the packed bytes.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a buffer from `serializeBytes`.
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
        yggdryl_core::BooleanBuffer::deserialize_bytes(bytes.as_ref())
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Freezes the packed bytes into a `ByteBuffer` (the bit length is not carried).
    #[napi]
    pub fn to_byte_buffer(&self) -> ByteBuffer {
        ByteBuffer {
            inner: self.inner.to_byte_buffer(),
        }
    }

    /// Reads `len` packed bits from a `ByteBuffer`.
    #[napi(factory)]
    pub fn from_byte_buffer(buffer: &ByteBuffer, len: u32) -> napi::Result<Self> {
        yggdryl_core::BooleanBuffer::from_byte_buffer(&buffer.inner, len as usize)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }

    /// Content equality.
    #[napi]
    pub fn equals(&self, other: &BooleanBuffer) -> bool {
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
