//! The `yggdryl.io` namespace — cursor-oriented byte IO.
//!
//! Exposes the [`Whence`] seek origin, [`ByteBuffer`] (storage), the byte
//! [`ByteCursor`] (the positioned reader/writer), and the element-typed cursors
//! [`I8Cursor`] … [`F64Cursor`] — one concrete class per primitive, mirroring the core
//! `TypedCursor<T>` (`tell` / `seek` count in `T` units; `byteTell` / `byteSeek` /
//! `bitTell` / `bitSeek` reach the byte and bit positions) — and the wide-integer
//! cursors `I96Cursor` / `I128Cursor` / `I256Cursor`, whose values marshal as JS
//! `BigInt`. A cursor's `byteSize` / `size` report the bytes / elements **remaining**
//! from the current position. The generic `IOBase` / `TypedIOBase` / `IOCursor`
//! traits are Rust-only. Two Node-specific omissions on the typed surface, per
//! `CLAUDE.md`: `U64Cursor` is omitted (napi has no native `u64` scalar — use
//! `I64Cursor` or raw bytes), and `F32Cursor` marshals over an `f64` JS boundary. The
//! `large*Size` accessors are returned as `BigInt`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::{BigInt, Buffer, Either3};
use napi::{JsBigInt, JsUnknown, ValueType};
use napi_derive::napi;

use yggdryl_core::{IOBase, IOCursor, IOSlice, TypedIOBase};

/// Maps a core IO error to a thrown JS `Error`.
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// The seek origin a position is measured from — `SEEK_SET` / `SEEK_CUR` /
/// `SEEK_END`.
#[napi(namespace = "io")]
pub enum Whence {
    /// From the start of the resource.
    Start,
    /// From the current cursor position.
    Current,
    /// From the end of the resource.
    End,
}

impl From<Whence> for yggdryl_core::Whence {
    fn from(whence: Whence) -> Self {
        match whence {
            Whence::Start => Self::Start,
            Whence::Current => Self::Current,
            Whence::End => Self::End,
        }
    }
}

/// Resolves an optional [`Whence`] (default `Start`) to the core origin.
fn whence_or_start(whence: Option<Whence>) -> yggdryl_core::Whence {
    whence.unwrap_or(Whence::Start).into()
}

/// An immutable byte store — pure storage; positioned IO is done via a
/// [`ByteCursor`] from [`byteCursor`](ByteBuffer::byte_cursor).
#[napi(namespace = "io")]
pub struct ByteBuffer {
    pub(crate) inner: yggdryl_core::ByteBuffer,
}

#[napi(namespace = "io")]
impl ByteBuffer {
    /// Creates a buffer, optionally holding a copy of `data`.
    #[napi(constructor)]
    pub fn new(data: Option<Buffer>) -> Self {
        let inner = match data {
            Some(bytes) => yggdryl_core::ByteBuffer::from_bytes(bytes.as_ref()),
            None => yggdryl_core::ByteBuffer::new(),
        };
        Self { inner }
    }

    /// Creates an empty buffer preallocated for `capacity` bytes.
    #[napi(factory)]
    pub fn with_byte_capacity(capacity: u32) -> Self {
        Self {
            inner: yggdryl_core::ByteBuffer::with_byte_capacity(capacity as usize),
        }
    }

    /// Creates an empty buffer preallocated for `capacity` bits.
    #[napi(factory)]
    pub fn with_bit_capacity(capacity: u32) -> Self {
        Self {
            inner: yggdryl_core::ByteBuffer::with_bit_capacity(capacity as usize),
        }
    }

    /// The number of bytes held.
    #[napi(getter)]
    pub fn length(&self) -> i64 {
        self.inner.byte_size() as i64
    }

    /// The number of bytes held.
    #[napi]
    pub fn byte_size(&self) -> i64 {
        self.inner.byte_size() as i64
    }

    /// The number of bits held.
    #[napi]
    pub fn bit_size(&self) -> i64 {
        self.inner.bit_size() as i64
    }

    /// Whether the buffer holds no bytes.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The number of bytes that can be held without reallocating.
    #[napi]
    pub fn byte_capacity(&self) -> i64 {
        self.inner.byte_capacity() as i64
    }

    /// The number of bits that can be held without reallocating.
    #[napi]
    pub fn bit_capacity(&self) -> i64 {
        self.inner.bit_capacity() as i64
    }

    /// A copy of the backing bytes.
    #[napi]
    pub fn as_bytes(&self) -> Buffer {
        self.inner.as_bytes().to_vec().into()
    }

    /// Serialises the buffer to its byte content.
    #[napi]
    pub fn serialize_bytes(&self) -> Buffer {
        self.inner.serialize_bytes().into()
    }

    /// Reconstructs a buffer from its byte content.
    #[napi(factory)]
    pub fn deserialize_bytes(bytes: Buffer) -> Self {
        Self {
            inner: yggdryl_core::ByteBuffer::deserialize_bytes(bytes.as_ref()),
        }
    }

    /// Opens a [`ByteCursor`] over this buffer (the buffer stays intact).
    #[napi]
    pub fn byte_cursor(&self) -> ByteCursor {
        ByteCursor {
            inner: self.inner.byte_cursor(),
        }
    }

    /// Opens a [`ByteSlice`] over the byte window `[offset, offset + len)` (clamped).
    #[napi]
    pub fn byte_slice(&self, offset: i64, len: u32) -> ByteSlice {
        ByteSlice {
            inner: self.inner.byte_slice(offset.max(0) as u64, len as usize),
        }
    }

    /// Content equality.
    #[napi]
    pub fn equals(&self, other: &ByteBuffer) -> bool {
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

/// A positioned, advancing cursor over a [`ByteBuffer`].
#[napi(namespace = "io")]
pub struct ByteCursor {
    pub(crate) inner: yggdryl_core::ByteCursor,
}

#[napi(namespace = "io")]
impl ByteCursor {
    /// The current position, in bytes from the start (the byte cursor's native
    /// unit; `bitTell` gives it in bits).
    #[napi]
    pub fn tell(&self) -> napi::Result<i64> {
        self.inner.byte_tell().map(|p| p as i64).map_err(to_error)
    }

    /// Moves the cursor to `offset` bytes relative to `whence`, returning the new
    /// position. A negative `offset` seeks backward.
    #[napi]
    pub fn seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
        self.inner
            .byte_seek(offset, whence_or_start(whence))
            .map(|p| p as i64)
            .map_err(to_error)
    }

    /// The current position, in bits from the start (`tell * 8`).
    #[napi]
    pub fn bit_tell(&self) -> napi::Result<i64> {
        self.inner.bit_tell().map(|p| p as i64).map_err(to_error)
    }

    /// Moves the cursor to `offset` bits relative to `whence`, returning the new bit
    /// position. The resolved bit position must be byte-aligned (a multiple of 8).
    #[napi]
    pub fn bit_seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
        self.inner
            .bit_seek(offset, whence_or_start(whence))
            .map(|p| p as i64)
            .map_err(to_error)
    }

    /// The current position (mirror of `tell`).
    #[napi]
    pub fn position(&self) -> i64 {
        self.inner.position() as i64
    }

    /// Sets the current position.
    #[napi]
    pub fn set_position(&mut self, position: i64) {
        self.inner.set_position(position.max(0) as u64);
    }

    /// Adjusts the backing allocation to hold `capacity` bytes, returning the new
    /// capacity. Growing reserves headroom; a `capacity` below the current length
    /// truncates the content (reducing the inner buffer) and clamps the cursor.
    #[napi]
    pub fn set_byte_capacity(&mut self, capacity: i64) -> i64 {
        self.inner.set_byte_capacity(capacity.max(0) as usize) as i64
    }

    /// Adjusts the backing allocation to hold `capacity` bits (rounded up to whole
    /// bytes), returning the new byte capacity.
    #[napi]
    pub fn set_bit_capacity(&mut self, capacity: i64) -> i64 {
        self.inner.set_bit_capacity(capacity.max(0) as usize) as i64
    }

    /// The number of bytes the resource holds.
    #[napi]
    pub fn byte_size(&self) -> napi::Result<i64> {
        self.inner.byte_size().map(|n| n as i64).map_err(to_error)
    }

    /// The number of bits the resource holds.
    #[napi]
    pub fn bit_size(&self) -> napi::Result<i64> {
        self.inner.bit_size().map(|n| n as i64).map_err(to_error)
    }

    /// The number of bytes as a JS `BigInt`.
    #[napi]
    pub fn large_byte_size(&self) -> napi::Result<BigInt> {
        self.inner
            .large_byte_size()
            .map(BigInt::from)
            .map_err(to_error)
    }

    /// The number of bits as a JS `BigInt`.
    #[napi]
    pub fn large_bit_size(&self) -> napi::Result<BigInt> {
        self.inner
            .large_bit_size()
            .map(BigInt::from)
            .map_err(to_error)
    }

    /// The number of bytes that can be held without reallocating.
    #[napi]
    pub fn byte_capacity(&self) -> napi::Result<i64> {
        self.inner
            .byte_capacity()
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// The number of bits that can be held without reallocating.
    #[napi]
    pub fn bit_capacity(&self) -> napi::Result<i64> {
        self.inner
            .bit_capacity()
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// The number of `u8` values held (equals `byteSize`).
    #[napi]
    pub fn size(&self) -> napi::Result<i64> {
        TypedIOBase::<u8>::size(&self.inner)
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// The `u8` capacity (equals `byteCapacity`).
    #[napi]
    pub fn capacity(&self) -> napi::Result<i64> {
        TypedIOBase::<u8>::capacity(&self.inner)
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// The default `u8` value used to fill a gap opened past the end on a grow (`0`).
    #[napi]
    pub fn default_value(&self) -> u8 {
        TypedIOBase::<u8>::default_value(&self.inner)
    }

    /// The little-endian bytes of `count` default values — the gap-fill pattern
    /// (all-zero for the byte cursor).
    #[napi]
    pub fn default_byte_array(&self, count: u32) -> Buffer {
        TypedIOBase::<u8>::default_byte_array(&self.inner, count as usize).into()
    }

    /// Reads up to `size` bytes at `whence`, advancing the cursor.
    #[napi]
    pub fn pread_byte_array(&mut self, size: u32, whence: Option<Whence>) -> napi::Result<Buffer> {
        let out = self
            .inner
            .pread_byte_array(size as usize, whence_or_start(whence))
            .map_err(to_error)?;
        Ok(out.into())
    }

    /// Writes `data` at `whence`, advancing the cursor.
    #[napi]
    pub fn pwrite_byte_array(&mut self, data: Buffer, whence: Option<Whence>) -> napi::Result<i64> {
        let n = self
            .inner
            .pwrite_byte_array(data.as_ref(), whence_or_start(whence))
            .map_err(to_error)?;
        Ok(n as i64)
    }

    /// Writes `data`, **inferring** its type and redirecting to the optimal write:
    /// a `Buffer` writes raw bytes, a `string` writes its UTF-8, an array of `bigint`
    /// writes little-endian `i64`, and an array of `number` writes little-endian
    /// `f64`. Returns the number of **bytes** written. For a specific width use the
    /// explicit `pwrite*` methods.
    #[napi]
    pub fn write(
        &mut self,
        data: Either3<Buffer, String, Vec<JsUnknown>>,
        whence: Option<Whence>,
    ) -> napi::Result<i64> {
        let w = whence_or_start(whence);
        match data {
            Either3::A(buffer) => Ok(self
                .inner
                .pwrite_byte_array(buffer.as_ref(), w)
                .map_err(to_error)? as i64),
            Either3::B(text) => Ok(self
                .inner
                .pwrite_byte_array(text.as_bytes(), w)
                .map_err(to_error)? as i64),
            Either3::C(items) => {
                if items.is_empty() {
                    return Ok(0);
                }
                match items[0].get_type()? {
                    ValueType::BigInt => {
                        let mut values = Vec::with_capacity(items.len());
                        for item in items {
                            if item.get_type()? != ValueType::BigInt {
                                return Err(napi::Error::from_reason(
                                    "cannot write a mixed array; every element must be a bigint",
                                ));
                            }
                            values.push(unsafe { item.cast::<JsBigInt>() }.get_i64()?.0);
                        }
                        self.inner.pwrite_i64_array(&values, w).map_err(to_error)?;
                        Ok((values.len() * 8) as i64)
                    }
                    ValueType::Number => {
                        let mut values = Vec::with_capacity(items.len());
                        for item in items {
                            if item.get_type()? != ValueType::Number {
                                return Err(napi::Error::from_reason(
                                    "cannot write a mixed array; every element must be a number",
                                ));
                            }
                            values.push(item.coerce_to_number()?.get_double()?);
                        }
                        self.inner.pwrite_f64_array(&values, w).map_err(to_error)?;
                        Ok((values.len() * 8) as i64)
                    }
                    _ => Err(napi::Error::from_reason(
                        "cannot infer a write; supported: Buffer, string, bigint[], number[]",
                    )),
                }
            }
        }
    }

    /// Reads up to `buf.length` bytes at `whence` **into** the provided `buf`,
    /// advancing the cursor, and returns the number read. Fills the JS `Buffer` in
    /// place (no new Buffer allocated), so reuse `buf` for zero-allocation reads.
    #[napi]
    pub fn pread_into(&mut self, mut buf: Buffer, whence: Option<Whence>) -> napi::Result<i64> {
        let n = self
            .inner
            .pread_into(buf.as_mut(), whence_or_start(whence))
            .map_err(to_error)?;
        Ok(n as i64)
    }

    /// Reads a single byte at `whence`, advancing the cursor.
    #[napi]
    pub fn pread_one(&mut self, whence: Option<Whence>) -> napi::Result<u8> {
        TypedIOBase::<u8>::pread_one(&mut self.inner, whence_or_start(whence)).map_err(to_error)
    }

    /// Writes a single byte at `whence`, advancing the cursor.
    #[napi]
    pub fn pwrite_one(&mut self, value: u8, whence: Option<Whence>) -> napi::Result<i64> {
        TypedIOBase::<u8>::pwrite_one(&mut self.inner, value, whence_or_start(whence))
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// Reads a little-endian `f32` at `whence`, widened to a JS number.
    #[napi]
    pub fn pread_f32(&mut self, whence: Option<Whence>) -> napi::Result<f64> {
        self.inner
            .pread_f32(whence_or_start(whence))
            .map(f64::from)
            .map_err(to_error)
    }

    /// Writes `value` (narrowed from a JS number) as a little-endian `f32`.
    #[napi]
    pub fn pwrite_f32(&mut self, value: f64, whence: Option<Whence>) -> napi::Result<i64> {
        self.inner
            .pwrite_f32(value as f32, whence_or_start(whence))
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// Copies up to `size` bytes from this cursor into `sink`, advancing both.
    #[napi]
    pub fn pread_io(
        &mut self,
        sink: &mut ByteCursor,
        size: u32,
        whence: Option<Whence>,
    ) -> napi::Result<i64> {
        self.inner
            .pread_io(&mut sink.inner, size as usize, whence_or_start(whence))
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// Copies up to `size` bytes from `source` into this cursor, advancing both.
    #[napi]
    pub fn pwrite_io(
        &mut self,
        source: &mut ByteCursor,
        size: u32,
        whence: Option<Whence>,
    ) -> napi::Result<i64> {
        self.inner
            .pwrite_io(&mut source.inner, size as usize, whence_or_start(whence))
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// The cursor's current bytes, including any writes.
    #[napi]
    pub fn as_bytes(&self) -> Buffer {
        self.inner.as_bytes().to_vec().into()
    }

    /// Freezes the cursor's bytes into a new [`ByteBuffer`].
    #[napi]
    pub fn to_byte_buffer(&self) -> ByteBuffer {
        ByteBuffer {
            inner: self.inner.to_byte_buffer(),
        }
    }
}

/// Generates the napi wrappers for one primitive's typed cursor accessors into
/// their own `#[napi] impl` block. (`i64` marshals as a JS `BigInt`.)
macro_rules! napi_primitive_io {
    ($( ($ty:ty, $read_one:ident, $write_one:ident, $read_arr:ident, $write_arr:ident) ),+ $(,)?) => {
        #[napi(namespace = "io")]
        impl ByteCursor {
            $(
                #[napi]
                pub fn $read_one(&mut self, whence: Option<Whence>) -> napi::Result<$ty> {
                    self.inner.$read_one(whence_or_start(whence)).map_err(to_error)
                }

                #[napi]
                pub fn $write_one(&mut self, value: $ty, whence: Option<Whence>) -> napi::Result<i64> {
                    self.inner.$write_one(value, whence_or_start(whence)).map(|n| n as i64).map_err(to_error)
                }

                #[napi]
                pub fn $read_arr(&mut self, count: u32, whence: Option<Whence>) -> napi::Result<Vec<$ty>> {
                    self.inner.$read_arr(count as usize, whence_or_start(whence)).map_err(to_error)
                }

                #[napi]
                pub fn $write_arr(&mut self, data: Vec<$ty>, whence: Option<Whence>) -> napi::Result<i64> {
                    self.inner.$write_arr(&data, whence_or_start(whence)).map(|n| n as i64).map_err(to_error)
                }
            )+
        }
    };
}

// `u64` is omitted (no native napi scalar); `f32` is handled above via `f64`.
napi_primitive_io!(
    (i8, pread_i8, pwrite_i8, pread_i8_array, pwrite_i8_array),
    (u8, pread_u8, pwrite_u8, pread_u8_array, pwrite_u8_array),
    (
        i16,
        pread_i16,
        pwrite_i16,
        pread_i16_array,
        pwrite_i16_array
    ),
    (
        u16,
        pread_u16,
        pwrite_u16,
        pread_u16_array,
        pwrite_u16_array
    ),
    (
        i32,
        pread_i32,
        pwrite_i32,
        pread_i32_array,
        pwrite_i32_array
    ),
    (
        u32,
        pread_u32,
        pwrite_u32,
        pread_u32_array,
        pwrite_u32_array
    ),
    (
        i64,
        pread_i64,
        pwrite_i64,
        pread_i64_array,
        pwrite_i64_array
    ),
    (
        f64,
        pread_f64,
        pwrite_f64,
        pread_f64_array,
        pwrite_f64_array
    ),
);

/// Generates one element-typed cursor class (`yggdryl_core::TypedCursor<$ty>`) whose
/// native unit is `$ty` — `tell` / `seek` count in `$ty` values, while `byte*` /
/// `bit*` reach the underlying byte and bit positions. Mirrors the core
/// `TypedCursor<T>`, one concrete class per primitive whose element maps to a native
/// napi scalar (`u64` is omitted; `f32` is `F32Cursor` below over an `f64` boundary;
/// the `u8` case is also `ByteCursor`).
macro_rules! napi_typed_cursor {
    ($( ($name:ident, $ty:ty) ),+ $(,)?) => {
        $(
            #[doc = concat!("A positioned, advancing cursor whose native unit is a `", stringify!($ty), "` value.")]
            #[napi(namespace = "io")]
            pub struct $name {
                pub(crate) inner: yggdryl_core::TypedCursor<$ty>,
            }

            #[napi(namespace = "io")]
            impl $name {
                #[doc = concat!("Creates an empty cursor preallocated for `capacity` `", stringify!($ty), "` values.")]
                #[napi(factory)]
                pub fn with_capacity(capacity: u32) -> Self {
                    Self {
                        inner: <yggdryl_core::TypedCursor<$ty> as TypedIOBase<$ty>>::with_capacity(capacity as usize),
                    }
                }

                #[doc = concat!("The current position, in `", stringify!($ty), "` values from the start.")]
                #[napi]
                pub fn tell(&self) -> napi::Result<i64> {
                    TypedIOBase::<$ty>::tell(&self.inner).map(|p| p as i64).map_err(to_error)
                }

                #[doc = concat!("Moves the cursor to `offset` `", stringify!($ty), "` values relative to `whence`. A negative `offset` seeks backward.")]
                #[napi]
                pub fn seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
                    TypedIOBase::<$ty>::seek(&mut self.inner, offset, whence_or_start(whence)).map(|p| p as i64).map_err(to_error)
                }

                /// The current position, in bytes from the start.
                #[napi]
                pub fn byte_tell(&self) -> napi::Result<i64> {
                    self.inner.byte_tell().map(|p| p as i64).map_err(to_error)
                }

                /// Moves the cursor to `offset` bytes relative to `whence`.
                #[napi]
                pub fn byte_seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
                    self.inner.byte_seek(offset, whence_or_start(whence)).map(|p| p as i64).map_err(to_error)
                }

                /// The current position, in bits from the start (`byteTell * 8`).
                #[napi]
                pub fn bit_tell(&self) -> napi::Result<i64> {
                    self.inner.bit_tell().map(|p| p as i64).map_err(to_error)
                }

                /// Moves the cursor to `offset` bits relative to `whence`; the resolved
                /// bit position must be byte-aligned (a multiple of 8).
                #[napi]
                pub fn bit_seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
                    self.inner.bit_seek(offset, whence_or_start(whence)).map(|p| p as i64).map_err(to_error)
                }

                /// The current position in bytes (mirror of `byteTell`).
                #[napi]
                pub fn position(&self) -> i64 {
                    self.inner.position() as i64
                }

                /// Sets the current byte position.
                #[napi]
                pub fn set_position(&mut self, position: i64) {
                    self.inner.set_position(position.max(0) as u64);
                }

                #[doc = concat!("The number of `", stringify!($ty), "` values held.")]
                #[napi]
                pub fn size(&self) -> napi::Result<i64> {
                    TypedIOBase::<$ty>::size(&self.inner).map(|n| n as i64).map_err(to_error)
                }

                #[doc = concat!("The `", stringify!($ty), "` capacity without reallocating.")]
                #[napi]
                pub fn capacity(&self) -> napi::Result<i64> {
                    TypedIOBase::<$ty>::capacity(&self.inner).map(|n| n as i64).map_err(to_error)
                }

                /// The number of bytes the resource holds.
                #[napi]
                pub fn byte_size(&self) -> napi::Result<i64> {
                    self.inner.byte_size().map(|n| n as i64).map_err(to_error)
                }

                /// The number of bits the resource holds.
                #[napi]
                pub fn bit_size(&self) -> napi::Result<i64> {
                    self.inner.bit_size().map(|n| n as i64).map_err(to_error)
                }

                /// The number of bytes that can be held without reallocating.
                #[napi]
                pub fn byte_capacity(&self) -> napi::Result<i64> {
                    self.inner.byte_capacity().map(|n| n as i64).map_err(to_error)
                }

                /// The number of bits that can be held without reallocating.
                #[napi]
                pub fn bit_capacity(&self) -> napi::Result<i64> {
                    self.inner.bit_capacity().map(|n| n as i64).map_err(to_error)
                }

                #[doc = concat!("Reads a single `", stringify!($ty), "` at `whence`, advancing the cursor.")]
                #[napi]
                pub fn pread_one(&mut self, whence: Option<Whence>) -> napi::Result<$ty> {
                    TypedIOBase::<$ty>::pread_one(&mut self.inner, whence_or_start(whence)).map_err(to_error)
                }

                #[doc = concat!("Writes a single `", stringify!($ty), "` at `whence`, advancing the cursor.")]
                #[napi]
                pub fn pwrite_one(&mut self, value: $ty, whence: Option<Whence>) -> napi::Result<i64> {
                    TypedIOBase::<$ty>::pwrite_one(&mut self.inner, value, whence_or_start(whence)).map(|n| n as i64).map_err(to_error)
                }

                #[doc = concat!("Reads up to `count` `", stringify!($ty), "` values at `whence`, advancing the cursor.")]
                #[napi]
                pub fn pread_array(&mut self, count: u32, whence: Option<Whence>) -> napi::Result<Vec<$ty>> {
                    TypedIOBase::<$ty>::pread_array(&mut self.inner, count as usize, whence_or_start(whence)).map_err(to_error)
                }

                #[doc = concat!("Writes the `", stringify!($ty), "` values in `data` at `whence`, advancing the cursor.")]
                #[napi]
                pub fn pwrite_array(&mut self, data: Vec<$ty>, whence: Option<Whence>) -> napi::Result<i64> {
                    TypedIOBase::<$ty>::pwrite_array(&mut self.inner, &data, whence_or_start(whence)).map(|n| n as i64).map_err(to_error)
                }

                #[doc = concat!("The default `", stringify!($ty), "` value used to fill a gap opened past the end on a grow (`0`).")]
                #[napi]
                pub fn default_value(&self) -> $ty {
                    TypedIOBase::<$ty>::default_value(&self.inner)
                }

                /// The little-endian bytes of `count` default values — the gap-fill
                /// pattern (all-zero for every native primitive).
                #[napi]
                pub fn default_byte_array(&self, count: u32) -> Buffer {
                    TypedIOBase::<$ty>::default_byte_array(&self.inner, count as usize).into()
                }

                /// Reads up to `size` raw bytes at `whence`, advancing the cursor.
                #[napi]
                pub fn pread_byte_array(&mut self, size: u32, whence: Option<Whence>) -> napi::Result<Buffer> {
                    self.inner.pread_byte_array(size as usize, whence_or_start(whence)).map(Into::into).map_err(to_error)
                }

                /// Writes raw `data` bytes at `whence`, advancing the cursor.
                #[napi]
                pub fn pwrite_byte_array(&mut self, data: Buffer, whence: Option<Whence>) -> napi::Result<i64> {
                    self.inner.pwrite_byte_array(data.as_ref(), whence_or_start(whence)).map(|n| n as i64).map_err(to_error)
                }

                /// The cursor's current bytes, including any writes.
                #[napi]
                pub fn as_bytes(&self) -> Buffer {
                    self.inner.as_bytes().to_vec().into()
                }

                /// Freezes the cursor's bytes into a new `ByteBuffer`.
                #[napi]
                pub fn to_byte_buffer(&self) -> ByteBuffer {
                    ByteBuffer {
                        inner: self.inner.to_byte_buffer(),
                    }
                }
            }
        )+
    };
}

// `U64Cursor` is omitted (no native napi `u64` scalar); `F32Cursor` is defined below
// over an `f64` boundary; the `u8` case is also `ByteCursor`.
napi_typed_cursor!(
    (I8Cursor, i8),
    (U8Cursor, u8),
    (I16Cursor, i16),
    (U16Cursor, u16),
    (I32Cursor, i32),
    (U32Cursor, u32),
    (I64Cursor, i64),
    (F64Cursor, f64),
);

/// A positioned, advancing cursor whose native unit is an `f32` value (marshalled over
/// an `f64` JS boundary).
#[napi(namespace = "io")]
pub struct F32Cursor {
    pub(crate) inner: yggdryl_core::TypedCursor<f32>,
}

#[napi(namespace = "io")]
impl F32Cursor {
    /// Creates an empty cursor preallocated for `capacity` `f32` values.
    #[napi(factory)]
    pub fn with_capacity(capacity: u32) -> Self {
        Self {
            inner: <yggdryl_core::TypedCursor<f32> as TypedIOBase<f32>>::with_capacity(
                capacity as usize,
            ),
        }
    }

    /// The current position, in `f32` values from the start.
    #[napi]
    pub fn tell(&self) -> napi::Result<i64> {
        TypedIOBase::<f32>::tell(&self.inner)
            .map(|p| p as i64)
            .map_err(to_error)
    }

    /// Moves the cursor to `offset` `f32` values relative to `whence`.
    #[napi]
    pub fn seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
        TypedIOBase::<f32>::seek(&mut self.inner, offset, whence_or_start(whence))
            .map(|p| p as i64)
            .map_err(to_error)
    }

    /// The current position, in bytes from the start.
    #[napi]
    pub fn byte_tell(&self) -> napi::Result<i64> {
        self.inner.byte_tell().map(|p| p as i64).map_err(to_error)
    }

    /// Moves the cursor to `offset` bytes relative to `whence`.
    #[napi]
    pub fn byte_seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
        self.inner
            .byte_seek(offset, whence_or_start(whence))
            .map(|p| p as i64)
            .map_err(to_error)
    }

    /// The current position, in bits from the start (`byteTell * 8`).
    #[napi]
    pub fn bit_tell(&self) -> napi::Result<i64> {
        self.inner.bit_tell().map(|p| p as i64).map_err(to_error)
    }

    /// Moves the cursor to `offset` bits relative to `whence`; the resolved bit
    /// position must be byte-aligned (a multiple of 8).
    #[napi]
    pub fn bit_seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
        self.inner
            .bit_seek(offset, whence_or_start(whence))
            .map(|p| p as i64)
            .map_err(to_error)
    }

    /// The current position in bytes (mirror of `byteTell`).
    #[napi]
    pub fn position(&self) -> i64 {
        self.inner.position() as i64
    }

    /// Sets the current byte position.
    #[napi]
    pub fn set_position(&mut self, position: i64) {
        self.inner.set_position(position.max(0) as u64);
    }

    /// The number of `f32` values held.
    #[napi]
    pub fn size(&self) -> napi::Result<i64> {
        TypedIOBase::<f32>::size(&self.inner)
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// The `f32` capacity without reallocating.
    #[napi]
    pub fn capacity(&self) -> napi::Result<i64> {
        TypedIOBase::<f32>::capacity(&self.inner)
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// The number of bytes the resource holds.
    #[napi]
    pub fn byte_size(&self) -> napi::Result<i64> {
        self.inner.byte_size().map(|n| n as i64).map_err(to_error)
    }

    /// The number of bits the resource holds.
    #[napi]
    pub fn bit_size(&self) -> napi::Result<i64> {
        self.inner.bit_size().map(|n| n as i64).map_err(to_error)
    }

    /// The number of bytes that can be held without reallocating.
    #[napi]
    pub fn byte_capacity(&self) -> napi::Result<i64> {
        self.inner
            .byte_capacity()
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// The number of bits that can be held without reallocating.
    #[napi]
    pub fn bit_capacity(&self) -> napi::Result<i64> {
        self.inner
            .bit_capacity()
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// Reads a single `f32` at `whence` (widened to a JS number), advancing the cursor.
    #[napi]
    pub fn pread_one(&mut self, whence: Option<Whence>) -> napi::Result<f64> {
        TypedIOBase::<f32>::pread_one(&mut self.inner, whence_or_start(whence))
            .map(f64::from)
            .map_err(to_error)
    }

    /// Writes a single `f32` (narrowed from a JS number) at `whence`, advancing.
    #[napi]
    pub fn pwrite_one(&mut self, value: f64, whence: Option<Whence>) -> napi::Result<i64> {
        TypedIOBase::<f32>::pwrite_one(&mut self.inner, value as f32, whence_or_start(whence))
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// Reads up to `count` `f32` values at `whence` (widened to JS numbers).
    #[napi]
    pub fn pread_array(&mut self, count: u32, whence: Option<Whence>) -> napi::Result<Vec<f64>> {
        TypedIOBase::<f32>::pread_array(&mut self.inner, count as usize, whence_or_start(whence))
            .map(|values| values.into_iter().map(f64::from).collect())
            .map_err(to_error)
    }

    /// Writes the `f32` values in `data` (narrowed from JS numbers) at `whence`.
    #[napi]
    pub fn pwrite_array(&mut self, data: Vec<f64>, whence: Option<Whence>) -> napi::Result<i64> {
        let narrowed: Vec<f32> = data.into_iter().map(|v| v as f32).collect();
        TypedIOBase::<f32>::pwrite_array(&mut self.inner, &narrowed, whence_or_start(whence))
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// The default `f32` value used to fill a gap opened past the end on a grow (`0`).
    #[napi]
    pub fn default_value(&self) -> f64 {
        f64::from(TypedIOBase::<f32>::default_value(&self.inner))
    }

    /// The little-endian bytes of `count` default values — the gap-fill pattern
    /// (all-zero).
    #[napi]
    pub fn default_byte_array(&self, count: u32) -> Buffer {
        TypedIOBase::<f32>::default_byte_array(&self.inner, count as usize).into()
    }

    /// Reads up to `size` raw bytes at `whence`, advancing the cursor.
    #[napi]
    pub fn pread_byte_array(&mut self, size: u32, whence: Option<Whence>) -> napi::Result<Buffer> {
        self.inner
            .pread_byte_array(size as usize, whence_or_start(whence))
            .map(Into::into)
            .map_err(to_error)
    }

    /// Writes raw `data` bytes at `whence`, advancing the cursor.
    #[napi]
    pub fn pwrite_byte_array(&mut self, data: Buffer, whence: Option<Whence>) -> napi::Result<i64> {
        self.inner
            .pwrite_byte_array(data.as_ref(), whence_or_start(whence))
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// The cursor's current bytes, including any writes.
    #[napi]
    pub fn as_bytes(&self) -> Buffer {
        self.inner.as_bytes().to_vec().into()
    }

    /// Freezes the cursor's bytes into a new `ByteBuffer`.
    #[napi]
    pub fn to_byte_buffer(&self) -> ByteBuffer {
        ByteBuffer {
            inner: self.inner.to_byte_buffer(),
        }
    }
}

// --- Wide-integer cursors (values marshalled as JS `BigInt`) ---
//
// Marshalling goes through the raw little-endian two's-complement bytes, uniformly
// for every width, rather than napi's `get_i128` (which flags `i128::MIN` as
// not-lossless and cannot carry `i256`).

use yggdryl_core::{i256, i96};

/// Two's-complement-negates a little-endian byte buffer in place.
fn negate_le(bytes: &mut [u8]) {
    for b in bytes.iter_mut() {
        *b = !*b;
    }
    let mut carry = 1u16;
    for b in bytes.iter_mut() {
        let sum = u16::from(*b) + carry;
        *b = sum as u8;
        carry = sum >> 8;
    }
}

/// Converts a wide integer's little-endian two's-complement bytes to a JS `BigInt`
/// (sign bit + little-endian `u64` magnitude words). `le` may be any length; it is
/// sign-extended to a whole number of 64-bit words.
fn le_bytes_to_bigint(le: &[u8]) -> BigInt {
    let negative = le.last().is_some_and(|b| b & 0x80 != 0);
    let words = le.len().div_ceil(8).max(1);
    let mut mag = vec![if negative { 0xFF } else { 0x00 }; words * 8];
    mag[..le.len()].copy_from_slice(le);
    if negative {
        negate_le(&mut mag);
    }
    let words = mag
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().expect("8 bytes")))
        .collect();
    BigInt {
        sign_bit: negative,
        words,
    }
}

/// Converts a JS `BigInt` to `width` little-endian two's-complement bytes, erroring if
/// it does not fit the signed `width`-byte range.
fn bigint_to_le_bytes(value: &BigInt, width: usize) -> napi::Result<Vec<u8>> {
    let range_err = || napi::Error::from_reason(format!("BigInt out of range for i{}", width * 8));
    // Magnitude bytes from the words (little-endian, unsigned).
    let mut mag = vec![0u8; value.words.len().max(1) * 8];
    for (i, word) in value.words.iter().enumerate() {
        mag[i * 8..i * 8 + 8].copy_from_slice(&word.to_le_bytes());
    }
    // Any magnitude bit at or above the width is out of range.
    if mag.len() > width && mag[width..].iter().any(|&b| b != 0) {
        return Err(range_err());
    }
    mag.resize(width, 0);
    // The top bit of the magnitude (value == 2^(8*width-1)) is only valid as the
    // negative extreme (`MIN`); anything else with it set overflows.
    let top_set = mag[width - 1] & 0x80 != 0;
    let is_min = top_set && mag[width - 1] == 0x80 && mag[..width - 1].iter().all(|&b| b == 0);
    if top_set && !(value.sign_bit && is_min) {
        return Err(range_err());
    }
    if value.sign_bit {
        negate_le(&mut mag);
    }
    Ok(mag)
}

fn i96_to_bigint(value: i96) -> BigInt {
    le_bytes_to_bigint(&value.to_le_bytes())
}

fn bigint_to_i96(value: BigInt) -> napi::Result<i96> {
    let bytes = bigint_to_le_bytes(&value, 12)?;
    Ok(i96::from_le_bytes(bytes.try_into().expect("12 bytes")))
}

fn i128_to_bigint(value: i128) -> BigInt {
    le_bytes_to_bigint(&value.to_le_bytes())
}

fn bigint_to_i128(value: BigInt) -> napi::Result<i128> {
    let bytes = bigint_to_le_bytes(&value, 16)?;
    Ok(i128::from_le_bytes(bytes.try_into().expect("16 bytes")))
}

fn i256_to_bigint(value: i256) -> BigInt {
    le_bytes_to_bigint(&value.to_le_bytes())
}

fn bigint_to_i256(value: BigInt) -> napi::Result<i256> {
    let bytes = bigint_to_le_bytes(&value, 32)?;
    Ok(i256::from_le_bytes(bytes.try_into().expect("32 bytes")))
}

/// Generates one wide-integer cursor class whose values marshal to/from JS `BigInt`.
macro_rules! napi_wide_cursor {
    ($( ($name:ident, $ty:ty, $to_big:path, $from_big:path) ),+ $(,)?) => {
        $(
            #[doc = concat!("A positioned, advancing cursor over `", stringify!($ty), "` values (marshalled as JS `BigInt`).")]
            #[napi(namespace = "io")]
            pub struct $name {
                pub(crate) inner: yggdryl_core::TypedCursor<$ty>,
            }

            #[napi(namespace = "io")]
            impl $name {
                #[doc = concat!("Creates an empty cursor preallocated for `capacity` `", stringify!($ty), "` values.")]
                #[napi(factory)]
                pub fn with_capacity(capacity: u32) -> Self {
                    Self {
                        inner: <yggdryl_core::TypedCursor<$ty> as TypedIOBase<$ty>>::with_capacity(capacity as usize),
                    }
                }

                /// Opens a cursor over a copy of `data` (little-endian bytes).
                #[napi(factory)]
                pub fn from_bytes(data: Buffer) -> Self {
                    Self {
                        inner: yggdryl_core::TypedCursor::new(yggdryl_core::ByteBuffer::from_bytes(data.as_ref())),
                    }
                }

                /// The current position, in element units from the start.
                #[napi]
                pub fn tell(&self) -> napi::Result<i64> {
                    TypedIOBase::<$ty>::tell(&self.inner).map(|p| p as i64).map_err(to_error)
                }

                /// Moves the cursor to `offset` element units relative to `whence`.
                #[napi]
                pub fn seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
                    TypedIOBase::<$ty>::seek(&mut self.inner, offset, whence_or_start(whence)).map(|p| p as i64).map_err(to_error)
                }

                /// The current position, in bytes from the start.
                #[napi]
                pub fn byte_tell(&self) -> napi::Result<i64> {
                    self.inner.byte_tell().map(|p| p as i64).map_err(to_error)
                }

                /// Moves the cursor to `offset` bytes relative to `whence`.
                #[napi]
                pub fn byte_seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
                    self.inner.byte_seek(offset, whence_or_start(whence)).map(|p| p as i64).map_err(to_error)
                }

                /// The current position, in bits from the start.
                #[napi]
                pub fn bit_tell(&self) -> napi::Result<i64> {
                    self.inner.bit_tell().map(|p| p as i64).map_err(to_error)
                }

                /// Moves the cursor to `offset` bits relative to `whence` (byte-aligned).
                #[napi]
                pub fn bit_seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
                    self.inner.bit_seek(offset, whence_or_start(whence)).map(|p| p as i64).map_err(to_error)
                }

                /// The current byte position (mirror of `byteTell`).
                #[napi]
                pub fn position(&self) -> i64 {
                    self.inner.position() as i64
                }

                /// Sets the current byte position.
                #[napi]
                pub fn set_position(&mut self, position: i64) {
                    self.inner.set_position(position.max(0) as u64);
                }

                /// The number of element values **remaining** from the current position.
                #[napi]
                pub fn size(&self) -> napi::Result<i64> {
                    TypedIOBase::<$ty>::size(&self.inner).map(|n| n as i64).map_err(to_error)
                }

                /// The element capacity without reallocating.
                #[napi]
                pub fn capacity(&self) -> napi::Result<i64> {
                    TypedIOBase::<$ty>::capacity(&self.inner).map(|n| n as i64).map_err(to_error)
                }

                /// The number of bytes remaining.
                #[napi]
                pub fn byte_size(&self) -> napi::Result<i64> {
                    self.inner.byte_size().map(|n| n as i64).map_err(to_error)
                }

                /// The number of bits remaining.
                #[napi]
                pub fn bit_size(&self) -> napi::Result<i64> {
                    self.inner.bit_size().map(|n| n as i64).map_err(to_error)
                }

                /// The byte capacity without reallocating.
                #[napi]
                pub fn byte_capacity(&self) -> napi::Result<i64> {
                    self.inner.byte_capacity().map(|n| n as i64).map_err(to_error)
                }

                /// The bit capacity without reallocating.
                #[napi]
                pub fn bit_capacity(&self) -> napi::Result<i64> {
                    self.inner.bit_capacity().map(|n| n as i64).map_err(to_error)
                }

                /// Reads a single value at `whence` (as a JS `BigInt`), advancing.
                #[napi]
                pub fn pread_one(&mut self, whence: Option<Whence>) -> napi::Result<BigInt> {
                    TypedIOBase::<$ty>::pread_one(&mut self.inner, whence_or_start(whence)).map($to_big).map_err(to_error)
                }

                /// Writes a single value (a JS `BigInt`) at `whence`, advancing.
                #[napi]
                pub fn pwrite_one(&mut self, value: BigInt, whence: Option<Whence>) -> napi::Result<i64> {
                    let v = $from_big(value)?;
                    TypedIOBase::<$ty>::pwrite_one(&mut self.inner, v, whence_or_start(whence)).map(|n| n as i64).map_err(to_error)
                }

                /// Reads up to `count` values at `whence` as JS `BigInt`s.
                #[napi]
                pub fn pread_array(&mut self, count: u32, whence: Option<Whence>) -> napi::Result<Vec<BigInt>> {
                    TypedIOBase::<$ty>::pread_array(&mut self.inner, count as usize, whence_or_start(whence))
                        .map(|values| values.into_iter().map($to_big).collect())
                        .map_err(to_error)
                }

                /// Writes the values in `data` (JS `BigInt`s) at `whence`.
                #[napi]
                pub fn pwrite_array(&mut self, data: Vec<BigInt>, whence: Option<Whence>) -> napi::Result<i64> {
                    let values: Vec<$ty> = data.into_iter().map($from_big).collect::<napi::Result<_>>()?;
                    TypedIOBase::<$ty>::pwrite_array(&mut self.inner, &values, whence_or_start(whence)).map(|n| n as i64).map_err(to_error)
                }

                /// The default value (`0`) used to fill a gap on a grow, as a JS `BigInt`.
                #[napi]
                pub fn default_value(&self) -> BigInt {
                    $to_big(TypedIOBase::<$ty>::default_value(&self.inner))
                }

                /// The little-endian bytes of `count` default values (all-zero).
                #[napi]
                pub fn default_byte_array(&self, count: u32) -> Buffer {
                    TypedIOBase::<$ty>::default_byte_array(&self.inner, count as usize).into()
                }

                /// The cursor's current bytes, including any writes.
                #[napi]
                pub fn as_bytes(&self) -> Buffer {
                    self.inner.as_bytes().to_vec().into()
                }

                /// Freezes the cursor's bytes into a new `ByteBuffer`.
                #[napi]
                pub fn to_byte_buffer(&self) -> ByteBuffer {
                    ByteBuffer { inner: self.inner.to_byte_buffer() }
                }
            }
        )+
    };
}

napi_wide_cursor!(
    (I96Cursor, i96, i96_to_bigint, bigint_to_i96),
    (I128Cursor, i128, i128_to_bigint, bigint_to_i128),
    (I256Cursor, i256, i256_to_bigint, bigint_to_i256),
);

// --- Bounded windows (the slice siblings of the cursors) ---

/// A bounded, non-growing byte **window** `[offset, offset + len)` over a `ByteBuffer`.
#[napi(namespace = "io")]
pub struct ByteSlice {
    pub(crate) inner: yggdryl_core::ByteSlice,
}

#[napi(namespace = "io")]
impl ByteSlice {
    /// Opens a window `[offset, offset + len)` over a copy of `data` (clamped).
    #[napi(factory)]
    pub fn from_bytes(data: Buffer, offset: i64, len: u32) -> Self {
        Self {
            inner: yggdryl_core::ByteSlice::new(
                yggdryl_core::ByteBuffer::from_bytes(data.as_ref()),
                offset.max(0) as u64,
                len as usize,
            ),
        }
    }

    /// The window's start offset within the origin resource, in bytes.
    #[napi]
    pub fn slice_offset(&self) -> i64 {
        self.inner.slice_offset() as i64
    }

    /// The window's length in bytes (its fixed extent).
    #[napi]
    pub fn slice_len(&self) -> i64 {
        self.inner.slice_len() as i64
    }

    /// The current position, in bytes from the window start.
    #[napi]
    pub fn tell(&self) -> napi::Result<i64> {
        self.inner.byte_tell().map(|p| p as i64).map_err(to_error)
    }

    /// Moves to `offset` bytes relative to `whence` (within the window).
    #[napi]
    pub fn seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
        self.inner
            .byte_seek(offset, whence_or_start(whence))
            .map(|p| p as i64)
            .map_err(to_error)
    }

    /// The current position, in bits from the window start (`tell * 8`).
    #[napi]
    pub fn bit_tell(&self) -> napi::Result<i64> {
        self.inner.bit_tell().map(|p| p as i64).map_err(to_error)
    }

    /// Moves to `offset` bits relative to `whence` (byte-aligned).
    #[napi]
    pub fn bit_seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
        self.inner
            .bit_seek(offset, whence_or_start(whence))
            .map(|p| p as i64)
            .map_err(to_error)
    }

    /// The current position (mirror of `tell`).
    #[napi]
    pub fn position(&self) -> i64 {
        self.inner.position() as i64
    }

    /// Sets the current position (within the window).
    #[napi]
    pub fn set_position(&mut self, position: i64) {
        self.inner.set_position(position.max(0) as u64);
    }

    /// The number of bytes **remaining** in the window from the current position.
    #[napi]
    pub fn byte_size(&self) -> napi::Result<i64> {
        self.inner.byte_size().map(|n| n as i64).map_err(to_error)
    }

    /// The number of bits remaining in the window.
    #[napi]
    pub fn bit_size(&self) -> napi::Result<i64> {
        self.inner.bit_size().map(|n| n as i64).map_err(to_error)
    }

    /// The window's byte capacity (its fixed length).
    #[napi]
    pub fn byte_capacity(&self) -> napi::Result<i64> {
        self.inner
            .byte_capacity()
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// Reads up to `size` bytes at `whence`, clamped to the window.
    #[napi]
    pub fn pread_byte_array(&mut self, size: u32, whence: Option<Whence>) -> napi::Result<Buffer> {
        self.inner
            .pread_byte_array(size as usize, whence_or_start(whence))
            .map(Into::into)
            .map_err(to_error)
    }

    /// Writes `data` at `whence`, clamped to the window (never grows).
    #[napi]
    pub fn pwrite_byte_array(&mut self, data: Buffer, whence: Option<Whence>) -> napi::Result<i64> {
        self.inner
            .pwrite_byte_array(data.as_ref(), whence_or_start(whence))
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// The window's current bytes, including any writes.
    #[napi]
    pub fn as_bytes(&self) -> Buffer {
        self.inner.as_bytes().to_vec().into()
    }

    /// Freezes the window's bytes into a new `ByteBuffer`.
    #[napi]
    pub fn to_byte_buffer(&self) -> ByteBuffer {
        ByteBuffer {
            inner: self.inner.to_byte_buffer(),
        }
    }
}

/// Generates one element-typed slice class (`yggdryl_core::TypedSlice<$ty>`) — the
/// bounded, non-growing sibling of the typed cursors.
macro_rules! napi_typed_slice {
    ($( ($name:ident, $ty:ty) ),+ $(,)?) => {
        $(
            #[doc = concat!("A bounded window over `", stringify!($ty), "` values (native units).")]
            #[napi(namespace = "io")]
            pub struct $name {
                pub(crate) inner: yggdryl_core::TypedSlice<$ty>,
            }

            #[napi(namespace = "io")]
            impl $name {
                /// Opens a window over a copy of `data` spanning the byte range `[offset, offset+len)`.
                #[napi(factory)]
                pub fn from_bytes(data: Buffer, offset: i64, len: u32) -> Self {
                    Self { inner: yggdryl_core::TypedSlice::new(yggdryl_core::ByteBuffer::from_bytes(data.as_ref()), offset.max(0) as u64, len as usize) }
                }
                /// The window's start offset within the origin resource, in bytes.
                #[napi]
                pub fn slice_offset(&self) -> i64 { self.inner.slice_offset() as i64 }
                /// The window's length in bytes.
                #[napi]
                pub fn slice_len(&self) -> i64 { self.inner.slice_len() as i64 }
                #[doc = concat!("The current position, in `", stringify!($ty), "` values from the window start.")]
                #[napi]
                pub fn tell(&self) -> napi::Result<i64> { TypedIOBase::<$ty>::tell(&self.inner).map(|p| p as i64).map_err(to_error) }
                #[doc = concat!("Moves to `offset` `", stringify!($ty), "` values relative to `whence`.")]
                #[napi]
                pub fn seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
                    TypedIOBase::<$ty>::seek(&mut self.inner, offset, whence_or_start(whence)).map(|p| p as i64).map_err(to_error)
                }
                /// The current position, in bytes from the window start.
                #[napi]
                pub fn byte_tell(&self) -> napi::Result<i64> { self.inner.byte_tell().map(|p| p as i64).map_err(to_error) }
                /// Moves to `offset` bytes relative to `whence`.
                #[napi]
                pub fn byte_seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
                    self.inner.byte_seek(offset, whence_or_start(whence)).map(|p| p as i64).map_err(to_error)
                }
                /// The current byte position (mirror of `byteTell`).
                #[napi]
                pub fn position(&self) -> i64 { self.inner.position() as i64 }
                /// Sets the current byte position (within the window).
                #[napi]
                pub fn set_position(&mut self, position: i64) { self.inner.set_position(position.max(0) as u64); }
                #[doc = concat!("The number of `", stringify!($ty), "` values remaining from the current position.")]
                #[napi]
                pub fn size(&self) -> napi::Result<i64> { TypedIOBase::<$ty>::size(&self.inner).map(|n| n as i64).map_err(to_error) }
                #[doc = concat!("The `", stringify!($ty), "` capacity (the window's fixed length).")]
                #[napi]
                pub fn capacity(&self) -> napi::Result<i64> { TypedIOBase::<$ty>::capacity(&self.inner).map(|n| n as i64).map_err(to_error) }
                /// The number of bytes remaining in the window.
                #[napi]
                pub fn byte_size(&self) -> napi::Result<i64> { self.inner.byte_size().map(|n| n as i64).map_err(to_error) }
                #[doc = concat!("Reads a single `", stringify!($ty), "` at `whence`, advancing.")]
                #[napi]
                pub fn pread_one(&mut self, whence: Option<Whence>) -> napi::Result<$ty> {
                    TypedIOBase::<$ty>::pread_one(&mut self.inner, whence_or_start(whence)).map_err(to_error)
                }
                #[doc = concat!("Writes a single `", stringify!($ty), "` at `whence` (clamped to the window).")]
                #[napi]
                pub fn pwrite_one(&mut self, value: $ty, whence: Option<Whence>) -> napi::Result<i64> {
                    TypedIOBase::<$ty>::pwrite_one(&mut self.inner, value, whence_or_start(whence)).map(|n| n as i64).map_err(to_error)
                }
                #[doc = concat!("Reads up to `count` `", stringify!($ty), "` values at `whence`, clamped.")]
                #[napi]
                pub fn pread_array(&mut self, count: u32, whence: Option<Whence>) -> napi::Result<Vec<$ty>> {
                    TypedIOBase::<$ty>::pread_array(&mut self.inner, count as usize, whence_or_start(whence)).map_err(to_error)
                }
                #[doc = concat!("Writes the `", stringify!($ty), "` values in `data` at `whence` (only whole values that fit).")]
                #[napi]
                pub fn pwrite_array(&mut self, data: Vec<$ty>, whence: Option<Whence>) -> napi::Result<i64> {
                    TypedIOBase::<$ty>::pwrite_array(&mut self.inner, &data, whence_or_start(whence)).map(|n| n as i64).map_err(to_error)
                }
                /// The window's current bytes.
                #[napi]
                pub fn as_bytes(&self) -> Buffer { self.inner.as_bytes().to_vec().into() }
                /// Freezes the window's bytes into a new `ByteBuffer`.
                #[napi]
                pub fn to_byte_buffer(&self) -> ByteBuffer { ByteBuffer { inner: self.inner.to_byte_buffer() } }
            }
        )+
    };
}

napi_typed_slice!(
    (I8Slice, i8),
    (U8Slice, u8),
    (I16Slice, i16),
    (U16Slice, u16),
    (I32Slice, i32),
    (U32Slice, u32),
    (I64Slice, i64),
    (F64Slice, f64),
);

/// Generates one wide-integer slice class whose values marshal to/from JS `BigInt`.
macro_rules! napi_wide_slice {
    ($( ($name:ident, $ty:ty, $to_big:path, $from_big:path) ),+ $(,)?) => {
        $(
            #[doc = concat!("A bounded window over `", stringify!($ty), "` values (marshalled as JS `BigInt`).")]
            #[napi(namespace = "io")]
            pub struct $name {
                pub(crate) inner: yggdryl_core::TypedSlice<$ty>,
            }

            #[napi(namespace = "io")]
            impl $name {
                /// Opens a window over a copy of `data` spanning the byte range `[offset, offset+len)`.
                #[napi(factory)]
                pub fn from_bytes(data: Buffer, offset: i64, len: u32) -> Self {
                    Self { inner: yggdryl_core::TypedSlice::new(yggdryl_core::ByteBuffer::from_bytes(data.as_ref()), offset.max(0) as u64, len as usize) }
                }
                /// The window's start offset within the origin resource, in bytes.
                #[napi]
                pub fn slice_offset(&self) -> i64 { self.inner.slice_offset() as i64 }
                /// The window's length in bytes.
                #[napi]
                pub fn slice_len(&self) -> i64 { self.inner.slice_len() as i64 }
                /// The current position, in element units from the window start.
                #[napi]
                pub fn tell(&self) -> napi::Result<i64> { TypedIOBase::<$ty>::tell(&self.inner).map(|p| p as i64).map_err(to_error) }
                /// Moves to `offset` element units relative to `whence`.
                #[napi]
                pub fn seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
                    TypedIOBase::<$ty>::seek(&mut self.inner, offset, whence_or_start(whence)).map(|p| p as i64).map_err(to_error)
                }
                /// The current position, in bytes from the window start.
                #[napi]
                pub fn byte_tell(&self) -> napi::Result<i64> { self.inner.byte_tell().map(|p| p as i64).map_err(to_error) }
                /// Moves to `offset` bytes relative to `whence`.
                #[napi]
                pub fn byte_seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
                    self.inner.byte_seek(offset, whence_or_start(whence)).map(|p| p as i64).map_err(to_error)
                }
                /// The current byte position (mirror of `byteTell`).
                #[napi]
                pub fn position(&self) -> i64 { self.inner.position() as i64 }
                /// Sets the current byte position (within the window).
                #[napi]
                pub fn set_position(&mut self, position: i64) { self.inner.set_position(position.max(0) as u64); }
                /// The number of element values remaining from the current position.
                #[napi]
                pub fn size(&self) -> napi::Result<i64> { TypedIOBase::<$ty>::size(&self.inner).map(|n| n as i64).map_err(to_error) }
                /// The element capacity (the window's fixed length).
                #[napi]
                pub fn capacity(&self) -> napi::Result<i64> { TypedIOBase::<$ty>::capacity(&self.inner).map(|n| n as i64).map_err(to_error) }
                /// The number of bytes remaining in the window.
                #[napi]
                pub fn byte_size(&self) -> napi::Result<i64> { self.inner.byte_size().map(|n| n as i64).map_err(to_error) }
                /// Reads a single value at `whence` (as a JS `BigInt`), advancing.
                #[napi]
                pub fn pread_one(&mut self, whence: Option<Whence>) -> napi::Result<BigInt> {
                    TypedIOBase::<$ty>::pread_one(&mut self.inner, whence_or_start(whence)).map($to_big).map_err(to_error)
                }
                /// Writes a single value (a JS `BigInt`) at `whence` (clamped to the window).
                #[napi]
                pub fn pwrite_one(&mut self, value: BigInt, whence: Option<Whence>) -> napi::Result<i64> {
                    let v = $from_big(value)?;
                    TypedIOBase::<$ty>::pwrite_one(&mut self.inner, v, whence_or_start(whence)).map(|n| n as i64).map_err(to_error)
                }
                /// Reads up to `count` values at `whence` as JS `BigInt`s.
                #[napi]
                pub fn pread_array(&mut self, count: u32, whence: Option<Whence>) -> napi::Result<Vec<BigInt>> {
                    TypedIOBase::<$ty>::pread_array(&mut self.inner, count as usize, whence_or_start(whence))
                        .map(|values| values.into_iter().map($to_big).collect()).map_err(to_error)
                }
                /// Writes the values in `data` (JS `BigInt`s) at `whence` (only whole values that fit).
                #[napi]
                pub fn pwrite_array(&mut self, data: Vec<BigInt>, whence: Option<Whence>) -> napi::Result<i64> {
                    let values: Vec<$ty> = data.into_iter().map($from_big).collect::<napi::Result<_>>()?;
                    TypedIOBase::<$ty>::pwrite_array(&mut self.inner, &values, whence_or_start(whence)).map(|n| n as i64).map_err(to_error)
                }
                /// The window's current bytes.
                #[napi]
                pub fn as_bytes(&self) -> Buffer { self.inner.as_bytes().to_vec().into() }
                /// Freezes the window's bytes into a new `ByteBuffer`.
                #[napi]
                pub fn to_byte_buffer(&self) -> ByteBuffer { ByteBuffer { inner: self.inner.to_byte_buffer() } }
            }
        )+
    };
}

napi_wide_slice!(
    (I96Slice, i96, i96_to_bigint, bigint_to_i96),
    (I128Slice, i128, i128_to_bigint, bigint_to_i128),
    (I256Slice, i256, i256_to_bigint, bigint_to_i256),
);

/// A bounded window over `f32` values (marshalled over an `f64` JS boundary).
#[napi(namespace = "io")]
pub struct F32Slice {
    pub(crate) inner: yggdryl_core::TypedSlice<f32>,
}

#[napi(namespace = "io")]
impl F32Slice {
    /// Opens a window over a copy of `data` spanning the byte range `[offset, offset+len)`.
    #[napi(factory)]
    pub fn from_bytes(data: Buffer, offset: i64, len: u32) -> Self {
        Self {
            inner: yggdryl_core::TypedSlice::new(
                yggdryl_core::ByteBuffer::from_bytes(data.as_ref()),
                offset.max(0) as u64,
                len as usize,
            ),
        }
    }

    /// The window's start offset within the origin resource, in bytes.
    #[napi]
    pub fn slice_offset(&self) -> i64 {
        self.inner.slice_offset() as i64
    }

    /// The window's length in bytes.
    #[napi]
    pub fn slice_len(&self) -> i64 {
        self.inner.slice_len() as i64
    }

    /// The current position, in `f32` values from the window start.
    #[napi]
    pub fn tell(&self) -> napi::Result<i64> {
        TypedIOBase::<f32>::tell(&self.inner)
            .map(|p| p as i64)
            .map_err(to_error)
    }

    /// Moves to `offset` `f32` values relative to `whence`.
    #[napi]
    pub fn seek(&mut self, offset: i64, whence: Option<Whence>) -> napi::Result<i64> {
        TypedIOBase::<f32>::seek(&mut self.inner, offset, whence_or_start(whence))
            .map(|p| p as i64)
            .map_err(to_error)
    }

    /// The number of `f32` values remaining from the current position.
    #[napi]
    pub fn size(&self) -> napi::Result<i64> {
        TypedIOBase::<f32>::size(&self.inner)
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// Reads a single `f32` at `whence` (widened to a JS number), advancing.
    #[napi]
    pub fn pread_one(&mut self, whence: Option<Whence>) -> napi::Result<f64> {
        TypedIOBase::<f32>::pread_one(&mut self.inner, whence_or_start(whence))
            .map(f64::from)
            .map_err(to_error)
    }

    /// Writes a single `f32` (narrowed from a JS number) at `whence` (clamped).
    #[napi]
    pub fn pwrite_one(&mut self, value: f64, whence: Option<Whence>) -> napi::Result<i64> {
        TypedIOBase::<f32>::pwrite_one(&mut self.inner, value as f32, whence_or_start(whence))
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// Reads up to `count` `f32` values at `whence` (widened to JS numbers).
    #[napi]
    pub fn pread_array(&mut self, count: u32, whence: Option<Whence>) -> napi::Result<Vec<f64>> {
        TypedIOBase::<f32>::pread_array(&mut self.inner, count as usize, whence_or_start(whence))
            .map(|values| values.into_iter().map(f64::from).collect())
            .map_err(to_error)
    }

    /// Writes the `f32` values in `data` (narrowed from JS numbers) at `whence`.
    #[napi]
    pub fn pwrite_array(&mut self, data: Vec<f64>, whence: Option<Whence>) -> napi::Result<i64> {
        let narrowed: Vec<f32> = data.into_iter().map(|v| v as f32).collect();
        TypedIOBase::<f32>::pwrite_array(&mut self.inner, &narrowed, whence_or_start(whence))
            .map(|n| n as i64)
            .map_err(to_error)
    }

    /// The window's current bytes.
    #[napi]
    pub fn as_bytes(&self) -> Buffer {
        self.inner.as_bytes().to_vec().into()
    }

    /// Freezes the window's bytes into a new `ByteBuffer`.
    #[napi]
    pub fn to_byte_buffer(&self) -> ByteBuffer {
        ByteBuffer {
            inner: self.inner.to_byte_buffer(),
        }
    }
}
