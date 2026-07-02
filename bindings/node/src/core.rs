//! The `yggdryl.core` namespace — thin wrappers over the `yggdryl-core` crate.

use napi::bindgen_prelude::{Buffer, Error, Result};
use napi_derive::napi;
use yggdryl_core::{RawIOBase, Seekable};

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

fn io_error(error: yggdryl_core::IOError) -> Error {
    Error::from_reason(error.to_string())
}

/// A growable, byte-granular in-memory buffer.
#[napi(namespace = "core")]
#[derive(Default)]
pub struct ByteBuffer {
    inner: yggdryl_core::ByteBuffer,
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

    /// The buffer's size, in bits.
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

    /// The current cursor position, in bytes.
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

    /// Read one byte.
    #[napi]
    pub fn pread_byte_one(&self, position: u32, whence: Whence) -> Result<u8> {
        self.inner
            .pread_byte_one(position as usize, whence.into())
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

    /// Write bytes.
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

    /// Write bits (MSB-first).
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

    /// Stream `size` bytes from this buffer into `sink`, copying in chunks.
    #[napi]
    pub fn pread_io(
        &self,
        position: u32,
        whence: Whence,
        size: u32,
        sink: &mut ByteBuffer,
        sink_position: u32,
        sink_whence: Whence,
    ) -> Result<()> {
        self.inner
            .pread_io(
                position as usize,
                whence.into(),
                size as usize,
                &mut sink.inner,
                sink_position as usize,
                sink_whence.into(),
            )
            .map_err(io_error)
    }

    /// Stream `size` bytes from `source` into this buffer, copying in chunks.
    #[napi]
    pub fn pwrite_io(
        &mut self,
        position: u32,
        whence: Whence,
        source: &ByteBuffer,
        source_position: u32,
        source_whence: Whence,
        size: u32,
    ) -> Result<()> {
        self.inner
            .pwrite_io(
                position as usize,
                whence.into(),
                &source.inner,
                source_position as usize,
                source_whence.into(),
                size as usize,
            )
            .map_err(io_error)
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

    /// The buffer's backing bytes.
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

    /// The current cursor position, in bytes.
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

    /// Read one byte.
    #[napi]
    pub fn pread_byte_one(&self, position: u32, whence: Whence) -> Result<u8> {
        self.inner
            .pread_byte_one(position as usize, whence.into())
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

    /// Write bytes.
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

    /// Write bits (MSB-first).
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

    /// Stream `size` bytes from this buffer into `sink`, copying in chunks.
    #[napi]
    pub fn pread_io(
        &self,
        position: u32,
        whence: Whence,
        size: u32,
        sink: &mut BitBuffer,
        sink_position: u32,
        sink_whence: Whence,
    ) -> Result<()> {
        self.inner
            .pread_io(
                position as usize,
                whence.into(),
                size as usize,
                &mut sink.inner,
                sink_position as usize,
                sink_whence.into(),
            )
            .map_err(io_error)
    }

    /// Stream `size` bytes from `source` into this buffer, copying in chunks.
    #[napi]
    pub fn pwrite_io(
        &mut self,
        position: u32,
        whence: Whence,
        source: &BitBuffer,
        source_position: u32,
        source_whence: Whence,
        size: u32,
    ) -> Result<()> {
        self.inner
            .pwrite_io(
                position as usize,
                whence.into(),
                &source.inner,
                source_position as usize,
                source_whence.into(),
                size as usize,
            )
            .map_err(io_error)
    }
}
