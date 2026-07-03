//! The `yggdryl.core` namespace — thin wrappers over the `yggdryl-core` crate.
//!
//! `ByteBuffer` / `BitBuffer` expose the positioned byte- and bit-IO surface, and
//! `ByteBufferCursor` / `BitBufferCursor` (a moving cursor) and `ByteBufferSlice` /
//! `BitBufferSlice` (a bounded byte window) wrap the core `RawIOCursor` / `RawIOSlice`
//! adapters over a copy of a buffer's bytes. Two things stay Rust-only: the
//! two-resource streams (`pread_raw_io` / `pwrite_raw_io` and the typed
//! `pread_typed_io` / `pwrite_typed_io`), which borrow two resources at once — napi
//! cannot borrow-check that across the FFI boundary, so a JS caller composes the same
//! effect from `preadByteArray` + `pwriteByteArray` — and the typed `IOCursor` /
//! `IOSlice` adapters (no exposed resource implements `IOBase`).

use napi::bindgen_prelude::{BigInt, Buffer, Error, Result};
use napi_derive::napi;
use yggdryl_core::{RawIOBase, RawIOCursor, RawIOSlice, Seekable};

/// The `yggdryl-core` version string.
#[napi(namespace = "core")]
pub fn version() -> String {
    yggdryl_core::version().to_string()
}

/// Prints a greeting to standard output — the minimal cross-language example.
#[napi(namespace = "core")]
pub fn hello() {
    yggdryl_core::hello()
}

/// The reference point a position is measured from.
#[napi(namespace = "core")]
pub enum Whence {
    Start,
    Current,
    End,
}

impl From<Whence> for yggdryl_core::Whence {
    fn from(whence: Whence) -> Self {
        match whence {
            Whence::Start => yggdryl_core::Whence::Start,
            Whence::Current => yggdryl_core::Whence::Current,
            Whence::End => yggdryl_core::Whence::End,
        }
    }
}

/// A `BigInt` as an `i64`, or an actionable error when out of range.
fn bigint_to_i64(value: BigInt) -> Result<i64> {
    let (value, lossless) = value.get_i64();
    if !lossless {
        return Err(Error::from_reason(
            "expected the BigInt to be in the i64 range",
        ));
    }
    Ok(value)
}

/// A `BigInt` as a `u64`, or an actionable error when negative or out of range.
fn bigint_to_u64(value: BigInt) -> Result<u64> {
    let (sign, value, lossless) = value.get_u64();
    if sign || !lossless {
        return Err(Error::from_reason(
            "expected the BigInt to be non-negative and in the u64 range",
        ));
    }
    Ok(value)
}

fn io_error(error: yggdryl_core::IOError) -> Error {
    Error::from_reason(error.to_string())
}

/// Generates the shared `RawIOBase` surface for an adapter wrapper whose `inner`
/// field implements `RawIOBase` over a buffer reachable with `get_ref`, so the
/// delegating surface is written once. Per-type extras (a factory, the cursor's
/// `tell`/`seek`, the slice's `start`/`end`) live in a second `#[napi]` impl block.
macro_rules! raw_io_adapter_node {
    ($ty:ident) => {
        #[napi(namespace = "core")]
        impl $ty {
            /// The wrapped resource's bytes.
            #[napi]
            pub fn to_bytes(&self) -> Buffer {
                Buffer::from(self.inner.get_ref().as_bytes().to_vec())
            }

            /// The size, in bytes.
            #[napi]
            pub fn byte_size(&self) -> u32 {
                self.inner.byte_size() as u32
            }

            /// The size, in bits.
            #[napi]
            pub fn bit_size(&self) -> u32 {
                self.inner.bit_size() as u32
            }

            /// The number of bytes the resource can hold without reallocating.
            #[napi]
            pub fn byte_capacity(&self) -> u32 {
                self.inner.byte_capacity() as u32
            }

            /// The number of bits the resource can hold without reallocating.
            #[napi]
            pub fn bit_capacity(&self) -> u32 {
                self.inner.bit_capacity() as u32
            }

            /// Request room for `capacity` bytes, returning the resulting capacity.
            #[napi]
            pub fn resize_byte_capacity(&mut self, capacity: u32) -> Result<u32> {
                self.inner
                    .resize_byte_capacity(capacity as usize)
                    .map(|capacity| capacity as u32)
                    .map_err(io_error)
            }

            /// Request room for `capacity` bits, returning the resulting bit capacity.
            #[napi]
            pub fn resize_bit_capacity(&mut self, capacity: u32) -> Result<u32> {
                self.inner
                    .resize_bit_capacity(capacity as usize)
                    .map(|capacity| capacity as u32)
                    .map_err(io_error)
            }

            /// Set the size to `size` bytes, truncating or zero-filling.
            #[napi]
            pub fn resize_bytes(&mut self, size: u32) -> Result<()> {
                self.inner.resize_bytes(size as usize).map_err(io_error)
            }

            /// Set the size to `size` bits.
            #[napi]
            pub fn resize_bits(&mut self, size: u32) -> Result<()> {
                self.inner.resize_bits(size as usize).map_err(io_error)
            }

            /// Read one byte.
            #[napi]
            pub fn pread_byte_one(&self, position: u32, whence: Whence) -> Result<u8> {
                self.inner
                    .pread_byte_one(position as usize, whence.into())
                    .map_err(io_error)
            }

            /// Read one little-endian `i8` at `position` (in bytes) relative to `whence`.
            #[napi]
            pub fn pread_i8(&self, position: u32, whence: Whence) -> Result<i32> {
                self.inner
                    .pread_i8(position as usize, whence.into())
                    .map(i32::from)
                    .map_err(io_error)
            }

            /// Write one `i8` as its little-endian bytes at `position` (in bytes)
            /// relative to `whence`.
            #[napi]
            pub fn pwrite_i8(&mut self, position: u32, whence: Whence, value: i32) -> Result<()> {
                let value = i8::try_from(value).map_err(|_| {
                    Error::from_reason(format!("expected {value} to be in the i8 range"))
                })?;
                self.inner
                    .pwrite_i8(position as usize, whence.into(), value)
                    .map_err(io_error)
            }

            /// Read one little-endian `i16` at `position` (in bytes) relative to `whence`.
            #[napi]
            pub fn pread_i16(&self, position: u32, whence: Whence) -> Result<i32> {
                self.inner
                    .pread_i16(position as usize, whence.into())
                    .map(i32::from)
                    .map_err(io_error)
            }

            /// Write one `i16` as its little-endian bytes at `position` (in bytes)
            /// relative to `whence`.
            #[napi]
            pub fn pwrite_i16(&mut self, position: u32, whence: Whence, value: i32) -> Result<()> {
                let value = i16::try_from(value).map_err(|_| {
                    Error::from_reason(format!("expected {value} to be in the i16 range"))
                })?;
                self.inner
                    .pwrite_i16(position as usize, whence.into(), value)
                    .map_err(io_error)
            }

            /// Read one little-endian `i32` at `position` (in bytes) relative to `whence`.
            #[napi]
            pub fn pread_i32(&self, position: u32, whence: Whence) -> Result<i32> {
                self.inner
                    .pread_i32(position as usize, whence.into())
                    .map_err(io_error)
            }

            /// Write one `i32` as its little-endian bytes at `position` (in bytes)
            /// relative to `whence`.
            #[napi]
            pub fn pwrite_i32(&mut self, position: u32, whence: Whence, value: i32) -> Result<()> {
                self.inner
                    .pwrite_i32(position as usize, whence.into(), value)
                    .map_err(io_error)
            }

            /// Read one little-endian `i64` at `position` (in bytes) relative to `whence`.
            #[napi]
            pub fn pread_i64(&self, position: u32, whence: Whence) -> Result<BigInt> {
                self.inner
                    .pread_i64(position as usize, whence.into())
                    .map(BigInt::from)
                    .map_err(io_error)
            }

            /// Write one `i64` as its little-endian bytes at `position` (in bytes)
            /// relative to `whence`.
            #[napi]
            pub fn pwrite_i64(
                &mut self,
                position: u32,
                whence: Whence,
                value: BigInt,
            ) -> Result<()> {
                let value = bigint_to_i64(value)?;
                self.inner
                    .pwrite_i64(position as usize, whence.into(), value)
                    .map_err(io_error)
            }

            /// Read one little-endian `u8` at `position` (in bytes) relative to `whence`.
            #[napi]
            pub fn pread_u8(&self, position: u32, whence: Whence) -> Result<u32> {
                self.inner
                    .pread_u8(position as usize, whence.into())
                    .map(u32::from)
                    .map_err(io_error)
            }

            /// Write one `u8` as its little-endian bytes at `position` (in bytes)
            /// relative to `whence`.
            #[napi]
            pub fn pwrite_u8(&mut self, position: u32, whence: Whence, value: u32) -> Result<()> {
                let value = u8::try_from(value).map_err(|_| {
                    Error::from_reason(format!("expected {value} to be in the u8 range"))
                })?;
                self.inner
                    .pwrite_u8(position as usize, whence.into(), value)
                    .map_err(io_error)
            }

            /// Read one little-endian `u16` at `position` (in bytes) relative to `whence`.
            #[napi]
            pub fn pread_u16(&self, position: u32, whence: Whence) -> Result<u32> {
                self.inner
                    .pread_u16(position as usize, whence.into())
                    .map(u32::from)
                    .map_err(io_error)
            }

            /// Write one `u16` as its little-endian bytes at `position` (in bytes)
            /// relative to `whence`.
            #[napi]
            pub fn pwrite_u16(&mut self, position: u32, whence: Whence, value: u32) -> Result<()> {
                let value = u16::try_from(value).map_err(|_| {
                    Error::from_reason(format!("expected {value} to be in the u16 range"))
                })?;
                self.inner
                    .pwrite_u16(position as usize, whence.into(), value)
                    .map_err(io_error)
            }

            /// Read one little-endian `u32` at `position` (in bytes) relative to `whence`.
            #[napi]
            pub fn pread_u32(&self, position: u32, whence: Whence) -> Result<u32> {
                self.inner
                    .pread_u32(position as usize, whence.into())
                    .map_err(io_error)
            }

            /// Write one `u32` as its little-endian bytes at `position` (in bytes)
            /// relative to `whence`.
            #[napi]
            pub fn pwrite_u32(&mut self, position: u32, whence: Whence, value: u32) -> Result<()> {
                self.inner
                    .pwrite_u32(position as usize, whence.into(), value)
                    .map_err(io_error)
            }

            /// Read one little-endian `u64` at `position` (in bytes) relative to `whence`.
            #[napi]
            pub fn pread_u64(&self, position: u32, whence: Whence) -> Result<BigInt> {
                self.inner
                    .pread_u64(position as usize, whence.into())
                    .map(BigInt::from)
                    .map_err(io_error)
            }

            /// Write one `u64` as its little-endian bytes at `position` (in bytes)
            /// relative to `whence`.
            #[napi]
            pub fn pwrite_u64(
                &mut self,
                position: u32,
                whence: Whence,
                value: BigInt,
            ) -> Result<()> {
                let value = bigint_to_u64(value)?;
                self.inner
                    .pwrite_u64(position as usize, whence.into(), value)
                    .map_err(io_error)
            }

            /// Read one little-endian `f32` at `position` (in bytes) relative to `whence`.
            #[napi]
            pub fn pread_f32(&self, position: u32, whence: Whence) -> Result<f64> {
                self.inner
                    .pread_f32(position as usize, whence.into())
                    .map(f64::from)
                    .map_err(io_error)
            }

            /// Write one `f32` as its little-endian bytes at `position` (in bytes)
            /// relative to `whence`.
            #[napi]
            pub fn pwrite_f32(&mut self, position: u32, whence: Whence, value: f64) -> Result<()> {
                let value = value as f32;
                self.inner
                    .pwrite_f32(position as usize, whence.into(), value)
                    .map_err(io_error)
            }

            /// Read one little-endian `f64` at `position` (in bytes) relative to `whence`.
            #[napi]
            pub fn pread_f64(&self, position: u32, whence: Whence) -> Result<f64> {
                self.inner
                    .pread_f64(position as usize, whence.into())
                    .map_err(io_error)
            }

            /// Write one `f64` as its little-endian bytes at `position` (in bytes)
            /// relative to `whence`.
            #[napi]
            pub fn pwrite_f64(&mut self, position: u32, whence: Whence, value: f64) -> Result<()> {
                self.inner
                    .pwrite_f64(position as usize, whence.into(), value)
                    .map_err(io_error)
            }

            /// Write one byte.
            #[napi]
            pub fn pwrite_byte_one(
                &mut self,
                position: u32,
                whence: Whence,
                value: u8,
            ) -> Result<()> {
                self.inner
                    .pwrite_byte_one(position as usize, whence.into(), value)
                    .map_err(io_error)
            }

            /// Read `size` bytes.
            #[napi]
            pub fn pread_byte_array(
                &self,
                position: u32,
                whence: Whence,
                size: u32,
            ) -> Result<Buffer> {
                self.inner
                    .pread_byte_array(position as usize, whence.into(), size as usize)
                    .map(Buffer::from)
                    .map_err(io_error)
            }

            /// Write bytes (an empty array is a no-op).
            #[napi]
            pub fn pwrite_byte_array(
                &mut self,
                position: u32,
                whence: Whence,
                values: Buffer,
            ) -> Result<()> {
                self.inner
                    .pwrite_byte_array(position as usize, whence.into(), &values)
                    .map_err(io_error)
            }

            /// Read one bit (MSB-first).
            #[napi]
            pub fn pread_bit_one(&self, position: u32, whence: Whence) -> Result<bool> {
                self.inner
                    .pread_bit_one(position as usize, whence.into())
                    .map_err(io_error)
            }

            /// Write one bit (MSB-first).
            #[napi]
            pub fn pwrite_bit_one(
                &mut self,
                position: u32,
                whence: Whence,
                value: bool,
            ) -> Result<()> {
                self.inner
                    .pwrite_bit_one(position as usize, whence.into(), value)
                    .map_err(io_error)
            }

            /// Read `size` bits (MSB-first).
            #[napi]
            pub fn pread_bit_array(
                &self,
                position: u32,
                whence: Whence,
                size: u32,
            ) -> Result<Vec<bool>> {
                self.inner
                    .pread_bit_array(position as usize, whence.into(), size as usize)
                    .map_err(io_error)
            }

            /// Write bits (MSB-first; an empty array is a no-op).
            #[napi]
            pub fn pwrite_bit_array(
                &mut self,
                position: u32,
                whence: Whence,
                values: Vec<bool>,
            ) -> Result<()> {
                self.inner
                    .pwrite_bit_array(position as usize, whence.into(), &values)
                    .map_err(io_error)
            }
        }
    };
}

/// A growable, byte-granular in-memory buffer.
#[napi(namespace = "core")]
#[derive(Default)]
pub struct ByteBuffer {
    inner: yggdryl_core::ByteBuffer,
}

impl ByteBuffer {
    // Wraps an existing core buffer — crate-internal, so sibling modules (the
    // data layer's `Binary::toIo`) can convert.
    pub(crate) fn from_inner(inner: yggdryl_core::ByteBuffer) -> Self {
        Self { inner }
    }
}

#[napi(namespace = "core")]
impl ByteBuffer {
    /// An empty buffer.
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: yggdryl_core::ByteBuffer::new(),
        }
    }

    /// A buffer over `data`.
    #[napi(factory)]
    pub fn from_bytes(data: Buffer) -> Self {
        Self {
            inner: yggdryl_core::ByteBuffer::from_bytes(data.to_vec()),
        }
    }

    /// The buffer's bytes.
    #[napi]
    pub fn to_bytes(&self) -> Buffer {
        Buffer::from(self.inner.as_bytes().to_vec())
    }

    /// The buffer's size, in bytes.
    #[napi]
    pub fn byte_size(&self) -> u32 {
        self.inner.byte_size() as u32
    }

    /// The buffer's size, in bits (eight times the byte size).
    #[napi]
    pub fn bit_size(&self) -> u32 {
        self.inner.bit_size() as u32
    }

    /// The number of bytes the buffer can hold without reallocating.
    #[napi]
    pub fn byte_capacity(&self) -> u32 {
        self.inner.byte_capacity() as u32
    }

    /// The number of bits the buffer can hold without reallocating.
    #[napi]
    pub fn bit_capacity(&self) -> u32 {
        self.inner.bit_capacity() as u32
    }

    /// Request room for `capacity` bytes, returning the resulting capacity.
    #[napi]
    pub fn resize_byte_capacity(&mut self, capacity: u32) -> Result<u32> {
        self.inner
            .resize_byte_capacity(capacity as usize)
            .map(|capacity| capacity as u32)
            .map_err(io_error)
    }

    /// Request room for `capacity` bits, returning the resulting bit capacity.
    #[napi]
    pub fn resize_bit_capacity(&mut self, capacity: u32) -> Result<u32> {
        self.inner
            .resize_bit_capacity(capacity as usize)
            .map(|capacity| capacity as u32)
            .map_err(io_error)
    }

    /// Set the buffer's size to `size` bytes, truncating or zero-filling.
    #[napi]
    pub fn resize_bytes(&mut self, size: u32) -> Result<()> {
        self.inner.resize_bytes(size as usize).map_err(io_error)
    }

    /// Set the buffer's size to `size` bits, rounded up to whole bytes.
    #[napi]
    pub fn resize_bits(&mut self, size: u32) -> Result<()> {
        self.inner.resize_bits(size as usize).map_err(io_error)
    }

    /// Read one byte.
    #[napi]
    pub fn pread_byte_one(&self, position: u32, whence: Whence) -> Result<u8> {
        self.inner
            .pread_byte_one(position as usize, whence.into())
            .map_err(io_error)
    }

    /// Read one little-endian `i8` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_i8(&self, position: u32, whence: Whence) -> Result<i32> {
        self.inner
            .pread_i8(position as usize, whence.into())
            .map(i32::from)
            .map_err(io_error)
    }

    /// Write one `i8` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_i8(&mut self, position: u32, whence: Whence, value: i32) -> Result<()> {
        let value = i8::try_from(value)
            .map_err(|_| Error::from_reason(format!("expected {value} to be in the i8 range")))?;
        self.inner
            .pwrite_i8(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `i16` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_i16(&self, position: u32, whence: Whence) -> Result<i32> {
        self.inner
            .pread_i16(position as usize, whence.into())
            .map(i32::from)
            .map_err(io_error)
    }

    /// Write one `i16` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_i16(&mut self, position: u32, whence: Whence, value: i32) -> Result<()> {
        let value = i16::try_from(value)
            .map_err(|_| Error::from_reason(format!("expected {value} to be in the i16 range")))?;
        self.inner
            .pwrite_i16(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `i32` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_i32(&self, position: u32, whence: Whence) -> Result<i32> {
        self.inner
            .pread_i32(position as usize, whence.into())
            .map_err(io_error)
    }

    /// Write one `i32` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_i32(&mut self, position: u32, whence: Whence, value: i32) -> Result<()> {
        self.inner
            .pwrite_i32(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `i64` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_i64(&self, position: u32, whence: Whence) -> Result<BigInt> {
        self.inner
            .pread_i64(position as usize, whence.into())
            .map(BigInt::from)
            .map_err(io_error)
    }

    /// Write one `i64` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_i64(&mut self, position: u32, whence: Whence, value: BigInt) -> Result<()> {
        let value = bigint_to_i64(value)?;
        self.inner
            .pwrite_i64(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `u8` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_u8(&self, position: u32, whence: Whence) -> Result<u32> {
        self.inner
            .pread_u8(position as usize, whence.into())
            .map(u32::from)
            .map_err(io_error)
    }

    /// Write one `u8` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_u8(&mut self, position: u32, whence: Whence, value: u32) -> Result<()> {
        let value = u8::try_from(value)
            .map_err(|_| Error::from_reason(format!("expected {value} to be in the u8 range")))?;
        self.inner
            .pwrite_u8(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `u16` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_u16(&self, position: u32, whence: Whence) -> Result<u32> {
        self.inner
            .pread_u16(position as usize, whence.into())
            .map(u32::from)
            .map_err(io_error)
    }

    /// Write one `u16` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_u16(&mut self, position: u32, whence: Whence, value: u32) -> Result<()> {
        let value = u16::try_from(value)
            .map_err(|_| Error::from_reason(format!("expected {value} to be in the u16 range")))?;
        self.inner
            .pwrite_u16(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `u32` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_u32(&self, position: u32, whence: Whence) -> Result<u32> {
        self.inner
            .pread_u32(position as usize, whence.into())
            .map_err(io_error)
    }

    /// Write one `u32` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_u32(&mut self, position: u32, whence: Whence, value: u32) -> Result<()> {
        self.inner
            .pwrite_u32(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `u64` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_u64(&self, position: u32, whence: Whence) -> Result<BigInt> {
        self.inner
            .pread_u64(position as usize, whence.into())
            .map(BigInt::from)
            .map_err(io_error)
    }

    /// Write one `u64` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_u64(&mut self, position: u32, whence: Whence, value: BigInt) -> Result<()> {
        let value = bigint_to_u64(value)?;
        self.inner
            .pwrite_u64(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `f32` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_f32(&self, position: u32, whence: Whence) -> Result<f64> {
        self.inner
            .pread_f32(position as usize, whence.into())
            .map(f64::from)
            .map_err(io_error)
    }

    /// Write one `f32` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_f32(&mut self, position: u32, whence: Whence, value: f64) -> Result<()> {
        let value = value as f32;
        self.inner
            .pwrite_f32(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `f64` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_f64(&self, position: u32, whence: Whence) -> Result<f64> {
        self.inner
            .pread_f64(position as usize, whence.into())
            .map_err(io_error)
    }

    /// Write one `f64` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_f64(&mut self, position: u32, whence: Whence, value: f64) -> Result<()> {
        self.inner
            .pwrite_f64(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Write one byte.
    #[napi]
    pub fn pwrite_byte_one(&mut self, position: u32, whence: Whence, value: u8) -> Result<()> {
        self.inner
            .pwrite_byte_one(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read `size` bytes.
    #[napi]
    pub fn pread_byte_array(&self, position: u32, whence: Whence, size: u32) -> Result<Buffer> {
        self.inner
            .pread_byte_array(position as usize, whence.into(), size as usize)
            .map(Buffer::from)
            .map_err(io_error)
    }

    /// Write bytes (an empty array is a no-op).
    #[napi]
    pub fn pwrite_byte_array(
        &mut self,
        position: u32,
        whence: Whence,
        values: Buffer,
    ) -> Result<()> {
        self.inner
            .pwrite_byte_array(position as usize, whence.into(), &values)
            .map_err(io_error)
    }

    /// Read one bit (MSB-first).
    #[napi]
    pub fn pread_bit_one(&self, position: u32, whence: Whence) -> Result<bool> {
        self.inner
            .pread_bit_one(position as usize, whence.into())
            .map_err(io_error)
    }

    /// Write one bit (MSB-first).
    #[napi]
    pub fn pwrite_bit_one(&mut self, position: u32, whence: Whence, value: bool) -> Result<()> {
        self.inner
            .pwrite_bit_one(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read `size` bits (MSB-first).
    #[napi]
    pub fn pread_bit_array(&self, position: u32, whence: Whence, size: u32) -> Result<Vec<bool>> {
        self.inner
            .pread_bit_array(position as usize, whence.into(), size as usize)
            .map_err(io_error)
    }

    /// Write bits (MSB-first; an empty array is a no-op).
    #[napi]
    pub fn pwrite_bit_array(
        &mut self,
        position: u32,
        whence: Whence,
        values: Vec<bool>,
    ) -> Result<()> {
        self.inner
            .pwrite_bit_array(position as usize, whence.into(), &values)
            .map_err(io_error)
    }

    /// A moving cursor over a copy of this buffer's bytes.
    #[napi]
    pub fn cursor(&self) -> ByteBufferCursor {
        ByteBufferCursor {
            inner: self.inner.clone().cursor(),
        }
    }

    /// A view bounded to the byte window `[start, end)` over a copy of this buffer.
    #[napi]
    pub fn slice(&self, start: u32, end: u32) -> ByteBufferSlice {
        ByteBufferSlice {
            inner: self.inner.clone().slice(start as usize, end as usize),
        }
    }
}

/// A growable, bit-granular in-memory buffer (its bit size need not be a multiple of
/// eight).
#[napi(namespace = "core")]
#[derive(Default)]
pub struct BitBuffer {
    inner: yggdryl_core::BitBuffer,
}

#[napi(namespace = "core")]
impl BitBuffer {
    /// An empty buffer.
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: yggdryl_core::BitBuffer::new(),
        }
    }

    /// A buffer over `data` (a whole number of bytes).
    #[napi(factory)]
    pub fn from_bytes(data: Buffer) -> Self {
        Self {
            inner: yggdryl_core::BitBuffer::from_bytes(data.to_vec()),
        }
    }

    /// The buffer's backing bytes (trailing padding bits are always zero).
    #[napi]
    pub fn to_bytes(&self) -> Buffer {
        Buffer::from(self.inner.as_bytes().to_vec())
    }

    /// The buffer's size, in bytes (rounded up).
    #[napi]
    pub fn byte_size(&self) -> u32 {
        self.inner.byte_size() as u32
    }

    /// The buffer's exact size, in bits.
    #[napi]
    pub fn bit_size(&self) -> u32 {
        self.inner.bit_size() as u32
    }

    /// The number of bytes the buffer can hold without reallocating.
    #[napi]
    pub fn byte_capacity(&self) -> u32 {
        self.inner.byte_capacity() as u32
    }

    /// The number of bits the buffer can hold without reallocating.
    #[napi]
    pub fn bit_capacity(&self) -> u32 {
        self.inner.bit_capacity() as u32
    }

    /// Request room for `capacity` bytes, returning the resulting capacity.
    #[napi]
    pub fn resize_byte_capacity(&mut self, capacity: u32) -> Result<u32> {
        self.inner
            .resize_byte_capacity(capacity as usize)
            .map(|capacity| capacity as u32)
            .map_err(io_error)
    }

    /// Request room for `capacity` bits, returning the resulting bit capacity.
    #[napi]
    pub fn resize_bit_capacity(&mut self, capacity: u32) -> Result<u32> {
        self.inner
            .resize_bit_capacity(capacity as usize)
            .map(|capacity| capacity as u32)
            .map_err(io_error)
    }

    /// Set the buffer's size to `size` bytes, truncating or zero-filling.
    #[napi]
    pub fn resize_bytes(&mut self, size: u32) -> Result<()> {
        self.inner.resize_bytes(size as usize).map_err(io_error)
    }

    /// Set the buffer's size to an exact `size` bits.
    #[napi]
    pub fn resize_bits(&mut self, size: u32) -> Result<()> {
        self.inner.resize_bits(size as usize).map_err(io_error)
    }

    /// Read one byte.
    #[napi]
    pub fn pread_byte_one(&self, position: u32, whence: Whence) -> Result<u8> {
        self.inner
            .pread_byte_one(position as usize, whence.into())
            .map_err(io_error)
    }

    /// Read one little-endian `i8` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_i8(&self, position: u32, whence: Whence) -> Result<i32> {
        self.inner
            .pread_i8(position as usize, whence.into())
            .map(i32::from)
            .map_err(io_error)
    }

    /// Write one `i8` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_i8(&mut self, position: u32, whence: Whence, value: i32) -> Result<()> {
        let value = i8::try_from(value)
            .map_err(|_| Error::from_reason(format!("expected {value} to be in the i8 range")))?;
        self.inner
            .pwrite_i8(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `i16` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_i16(&self, position: u32, whence: Whence) -> Result<i32> {
        self.inner
            .pread_i16(position as usize, whence.into())
            .map(i32::from)
            .map_err(io_error)
    }

    /// Write one `i16` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_i16(&mut self, position: u32, whence: Whence, value: i32) -> Result<()> {
        let value = i16::try_from(value)
            .map_err(|_| Error::from_reason(format!("expected {value} to be in the i16 range")))?;
        self.inner
            .pwrite_i16(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `i32` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_i32(&self, position: u32, whence: Whence) -> Result<i32> {
        self.inner
            .pread_i32(position as usize, whence.into())
            .map_err(io_error)
    }

    /// Write one `i32` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_i32(&mut self, position: u32, whence: Whence, value: i32) -> Result<()> {
        self.inner
            .pwrite_i32(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `i64` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_i64(&self, position: u32, whence: Whence) -> Result<BigInt> {
        self.inner
            .pread_i64(position as usize, whence.into())
            .map(BigInt::from)
            .map_err(io_error)
    }

    /// Write one `i64` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_i64(&mut self, position: u32, whence: Whence, value: BigInt) -> Result<()> {
        let value = bigint_to_i64(value)?;
        self.inner
            .pwrite_i64(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `u8` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_u8(&self, position: u32, whence: Whence) -> Result<u32> {
        self.inner
            .pread_u8(position as usize, whence.into())
            .map(u32::from)
            .map_err(io_error)
    }

    /// Write one `u8` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_u8(&mut self, position: u32, whence: Whence, value: u32) -> Result<()> {
        let value = u8::try_from(value)
            .map_err(|_| Error::from_reason(format!("expected {value} to be in the u8 range")))?;
        self.inner
            .pwrite_u8(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `u16` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_u16(&self, position: u32, whence: Whence) -> Result<u32> {
        self.inner
            .pread_u16(position as usize, whence.into())
            .map(u32::from)
            .map_err(io_error)
    }

    /// Write one `u16` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_u16(&mut self, position: u32, whence: Whence, value: u32) -> Result<()> {
        let value = u16::try_from(value)
            .map_err(|_| Error::from_reason(format!("expected {value} to be in the u16 range")))?;
        self.inner
            .pwrite_u16(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `u32` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_u32(&self, position: u32, whence: Whence) -> Result<u32> {
        self.inner
            .pread_u32(position as usize, whence.into())
            .map_err(io_error)
    }

    /// Write one `u32` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_u32(&mut self, position: u32, whence: Whence, value: u32) -> Result<()> {
        self.inner
            .pwrite_u32(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `u64` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_u64(&self, position: u32, whence: Whence) -> Result<BigInt> {
        self.inner
            .pread_u64(position as usize, whence.into())
            .map(BigInt::from)
            .map_err(io_error)
    }

    /// Write one `u64` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_u64(&mut self, position: u32, whence: Whence, value: BigInt) -> Result<()> {
        let value = bigint_to_u64(value)?;
        self.inner
            .pwrite_u64(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `f32` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_f32(&self, position: u32, whence: Whence) -> Result<f64> {
        self.inner
            .pread_f32(position as usize, whence.into())
            .map(f64::from)
            .map_err(io_error)
    }

    /// Write one `f32` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_f32(&mut self, position: u32, whence: Whence, value: f64) -> Result<()> {
        let value = value as f32;
        self.inner
            .pwrite_f32(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read one little-endian `f64` at `position` (in bytes) relative to `whence`.
    #[napi]
    pub fn pread_f64(&self, position: u32, whence: Whence) -> Result<f64> {
        self.inner
            .pread_f64(position as usize, whence.into())
            .map_err(io_error)
    }

    /// Write one `f64` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`.
    #[napi]
    pub fn pwrite_f64(&mut self, position: u32, whence: Whence, value: f64) -> Result<()> {
        self.inner
            .pwrite_f64(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Write one byte.
    #[napi]
    pub fn pwrite_byte_one(&mut self, position: u32, whence: Whence, value: u8) -> Result<()> {
        self.inner
            .pwrite_byte_one(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read `size` bytes.
    #[napi]
    pub fn pread_byte_array(&self, position: u32, whence: Whence, size: u32) -> Result<Buffer> {
        self.inner
            .pread_byte_array(position as usize, whence.into(), size as usize)
            .map(Buffer::from)
            .map_err(io_error)
    }

    /// Write bytes (an empty array is a no-op).
    #[napi]
    pub fn pwrite_byte_array(
        &mut self,
        position: u32,
        whence: Whence,
        values: Buffer,
    ) -> Result<()> {
        self.inner
            .pwrite_byte_array(position as usize, whence.into(), &values)
            .map_err(io_error)
    }

    /// Read one bit (MSB-first).
    #[napi]
    pub fn pread_bit_one(&self, position: u32, whence: Whence) -> Result<bool> {
        self.inner
            .pread_bit_one(position as usize, whence.into())
            .map_err(io_error)
    }

    /// Write one bit (MSB-first).
    #[napi]
    pub fn pwrite_bit_one(&mut self, position: u32, whence: Whence, value: bool) -> Result<()> {
        self.inner
            .pwrite_bit_one(position as usize, whence.into(), value)
            .map_err(io_error)
    }

    /// Read `size` bits (MSB-first).
    #[napi]
    pub fn pread_bit_array(&self, position: u32, whence: Whence, size: u32) -> Result<Vec<bool>> {
        self.inner
            .pread_bit_array(position as usize, whence.into(), size as usize)
            .map_err(io_error)
    }

    /// Write bits (MSB-first; an empty array is a no-op).
    #[napi]
    pub fn pwrite_bit_array(
        &mut self,
        position: u32,
        whence: Whence,
        values: Vec<bool>,
    ) -> Result<()> {
        self.inner
            .pwrite_bit_array(position as usize, whence.into(), &values)
            .map_err(io_error)
    }

    /// A moving cursor over a copy of this buffer's bytes.
    #[napi]
    pub fn cursor(&self) -> BitBufferCursor {
        BitBufferCursor {
            inner: self.inner.clone().cursor(),
        }
    }

    /// A view bounded to the byte window `[start, end)` over a copy of this buffer.
    #[napi]
    pub fn slice(&self, start: u32, end: u32) -> BitBufferSlice {
        BitBufferSlice {
            inner: self.inner.clone().slice(start as usize, end as usize),
        }
    }
}

/// A moving cursor over a copy of a [`ByteBuffer`]'s bytes (a `RawIOCursor`): every
/// read and write advances its position, measured from `Whence.Current`.
#[napi(namespace = "core")]
pub struct ByteBufferCursor {
    inner: RawIOCursor<yggdryl_core::ByteBuffer>,
}

raw_io_adapter_node!(ByteBufferCursor);

#[napi(namespace = "core")]
impl ByteBufferCursor {
    /// A cursor over `data`.
    #[napi(factory)]
    pub fn from_bytes(data: Buffer) -> Self {
        Self {
            inner: RawIOCursor::new(yggdryl_core::ByteBuffer::from_bytes(data.to_vec())),
        }
    }

    /// The cursor position, in bytes.
    #[napi]
    pub fn tell(&self) -> u32 {
        self.inner.tell() as u32
    }

    /// Move the cursor, returning the new position.
    #[napi]
    pub fn seek(&mut self, position: u32, whence: Whence) -> Result<u32> {
        self.inner
            .seek(position as usize, whence.into())
            .map(|position| position as u32)
            .map_err(io_error)
    }
}

/// A moving cursor over a copy of a [`BitBuffer`]'s bytes (a `RawIOCursor`).
#[napi(namespace = "core")]
pub struct BitBufferCursor {
    inner: RawIOCursor<yggdryl_core::BitBuffer>,
}

raw_io_adapter_node!(BitBufferCursor);

#[napi(namespace = "core")]
impl BitBufferCursor {
    /// A cursor over `data`.
    #[napi(factory)]
    pub fn from_bytes(data: Buffer) -> Self {
        Self {
            inner: RawIOCursor::new(yggdryl_core::BitBuffer::from_bytes(data.to_vec())),
        }
    }

    /// The cursor position, in bytes.
    #[napi]
    pub fn tell(&self) -> u32 {
        self.inner.tell() as u32
    }

    /// Move the cursor, returning the new position.
    #[napi]
    pub fn seek(&mut self, position: u32, whence: Whence) -> Result<u32> {
        self.inner
            .seek(position as usize, whence.into())
            .map(|position| position as u32)
            .map_err(io_error)
    }
}

/// A view of a copy of a [`ByteBuffer`] bounded to the byte window `[start, end)` (a
/// `RawIOSlice`): access outside the window fails.
#[napi(namespace = "core")]
pub struct ByteBufferSlice {
    inner: RawIOSlice<yggdryl_core::ByteBuffer>,
}

impl ByteBufferSlice {
    // Wraps an existing core slice — crate-internal, so sibling modules (the
    // data layer's `Binary::toIoSlice`) can convert.
    pub(crate) fn from_inner(inner: yggdryl_core::ByteBufferSlice) -> Self {
        Self { inner }
    }
}

raw_io_adapter_node!(ByteBufferSlice);

#[napi(namespace = "core")]
impl ByteBufferSlice {
    /// A window `[start, end)` over `data`.
    #[napi(factory)]
    pub fn from_bytes(data: Buffer, start: u32, end: u32) -> Self {
        Self {
            inner: RawIOSlice::new(
                yggdryl_core::ByteBuffer::from_bytes(data.to_vec()),
                start as usize,
                end as usize,
            ),
        }
    }

    /// The window's start byte offset.
    #[napi]
    pub fn start(&self) -> u32 {
        self.inner.start() as u32
    }

    /// The window's end byte offset (exclusive).
    #[napi]
    pub fn end(&self) -> u32 {
        self.inner.end() as u32
    }
}

/// A view of a copy of a [`BitBuffer`] bounded to the byte window `[start, end)` (a
/// `RawIOSlice`).
#[napi(namespace = "core")]
pub struct BitBufferSlice {
    inner: RawIOSlice<yggdryl_core::BitBuffer>,
}

raw_io_adapter_node!(BitBufferSlice);

#[napi(namespace = "core")]
impl BitBufferSlice {
    /// A window `[start, end)` over `data`.
    #[napi(factory)]
    pub fn from_bytes(data: Buffer, start: u32, end: u32) -> Self {
        Self {
            inner: RawIOSlice::new(
                yggdryl_core::BitBuffer::from_bytes(data.to_vec()),
                start as usize,
                end as usize,
            ),
        }
    }

    /// The window's start byte offset.
    #[napi]
    pub fn start(&self) -> u32 {
        self.inner.start() as u32
    }

    /// The window's end byte offset (exclusive).
    #[napi]
    pub fn end(&self) -> u32 {
        self.inner.end() as u32
    }
}
