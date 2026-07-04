//! Positioned byte- and bit-I/O: the low-level [`RawIOBase`] trait, the typed
//! [`IOBase`] layer, the [`Whence`] reference point, the concrete
//! [`ByteBuffer`] / [`BitBuffer`] / [`Utf8Buffer`] resources (the last a
//! UTF-8 byte buffer with a typed `char` view), the [`Seekable`] [`RawIOCursor`] /
//! [`IOCursor`] adapters that add a moving cursor, and the [`RawIOSlice`] /
//! [`IOSlice`] adapters that bound a resource to a byte window.

mod bit_buffer;
mod bits;
mod byte_buffer;
mod byte_buffer_slice;
mod cursor;
mod error;
mod raw_cursor;
mod raw_slice;
mod seekable;
mod slice;
mod utf8_buffer;
mod typed;
mod whence;

pub use bit_buffer::BitBuffer;
pub use byte_buffer::ByteBuffer;
pub use byte_buffer_slice::ByteBufferSlice;
pub use cursor::IOCursor;
pub use error::IOError;
pub use raw_cursor::RawIOCursor;
pub use raw_slice::RawIOSlice;
pub use seekable::Seekable;
pub use slice::IOSlice;
pub use utf8_buffer::Utf8Buffer;
pub use typed::IOBase;
pub use whence::Whence;

/// Bytes copied per chunk by the default `pread_raw_io` / `pwrite_raw_io` streams, so a
/// large transfer never materializes in full.
const STREAM_CHUNK: usize = 64 * 1024;

/// Positioned reads and writes over a resource, one or many `u8` bytes or `bool`
/// bits at a time.
///
/// A bare resource keeps no cursor of its own: [`Whence::Start`] is absolute and
/// [`Whence::End`] is measured from the size, while [`Whence::Current`] — having no
/// cursor to anchor to — is measured from the start. Wrap a resource in a
/// [`RawIOCursor`] (or, for typed values, an [`IOCursor`]) for a [`Seekable`]
/// position that advances on each read and write.
///
/// Every access names a `position` and the [`Whence`] it is measured from —
/// counted in **bytes** for the `*_byte_*` methods and in **bits** (MSB-first, so
/// bit `0` of a byte is its most significant bit) for the `*_bit_*` methods.
/// Implementors provide the four array primitives
/// ([`pread_byte_array`](RawIOBase::pread_byte_array),
/// [`pwrite_byte_array`](RawIOBase::pwrite_byte_array),
/// [`pread_bit_array`](RawIOBase::pread_bit_array),
/// [`pwrite_bit_array`](RawIOBase::pwrite_bit_array)) plus
/// [`byte_size`](RawIOBase::byte_size) and
/// [`resize_bytes`](RawIOBase::resize_bytes); everything else — the `*_one`
/// accessors, bit sizes, capacities, and the [`pread_raw_io`](RawIOBase::pread_raw_io) /
/// [`pwrite_raw_io`](RawIOBase::pwrite_raw_io) streams — comes for free from default
/// implementations.
///
/// ```
/// use yggdryl_core::{ByteBuffer, RawIOBase, Whence};
///
/// let mut buf = ByteBuffer::new();
/// buf.pwrite_byte_array(0, Whence::Start, &[0b1010_0000, 7])?;
/// assert_eq!(buf.byte_size(), 2);
/// assert_eq!(buf.pread_byte_one(1, Whence::Start)?, 7);
/// assert!(buf.pread_bit_one(0, Whence::Start)?); // MSB of the first byte
///
/// // Sizes, capacities and resizing.
/// buf.resize_bytes(4)?;
/// assert_eq!((buf.byte_size(), buf.bit_size()), (4, 32));
/// assert!(buf.resize_byte_capacity(64)? >= 64);
///
/// // Stream into another resource, chunked — no whole-copy materialization.
/// let mut sink = ByteBuffer::new();
/// buf.pread_raw_io(0, Whence::Start, 4, &mut sink, 0, Whence::Start)?;
/// assert_eq!(sink.as_bytes(), buf.as_bytes());
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
pub trait RawIOBase {
    /// The resource's total size, in bytes.
    fn byte_size(&self) -> usize;

    /// The resource's total size, in bits. Defaults to eight times
    /// [`byte_size`](RawIOBase::byte_size).
    fn bit_size(&self) -> usize {
        crate::log_event!(trace, "RawIOBase::bit_size");
        self.byte_size() * 8
    }

    /// The number of bytes the resource can hold without reallocating. Defaults to
    /// [`byte_size`](RawIOBase::byte_size) for resources that do not over-allocate.
    fn byte_capacity(&self) -> usize {
        self.byte_size()
    }

    /// The number of bits the resource can hold without reallocating. Defaults to
    /// eight times [`byte_capacity`](RawIOBase::byte_capacity).
    fn bit_capacity(&self) -> usize {
        self.byte_capacity() * 8
    }

    /// Request room for `capacity` bytes, returning the resulting capacity.
    ///
    /// The request is a hint: the default leaves a fixed allocation unchanged (and
    /// logs the skip); growable resources reserve or shrink towards it. It never
    /// changes [`byte_size`](RawIOBase::byte_size).
    fn resize_byte_capacity(&mut self, capacity: usize) -> Result<usize, IOError> {
        crate::log_event!(
            warn,
            "RawIOBase::resize_byte_capacity({capacity}) ignored: fixed allocation"
        );
        let _ = capacity;
        Ok(self.byte_capacity())
    }

    /// Request room for `capacity` bits, returning the resulting bit capacity.
    /// Defaults to [`resize_byte_capacity`](RawIOBase::resize_byte_capacity) on the
    /// enclosing whole bytes.
    fn resize_bit_capacity(&mut self, capacity: usize) -> Result<usize, IOError> {
        self.resize_byte_capacity(capacity.div_ceil(8))?;
        Ok(self.bit_capacity())
    }

    /// Set the resource's size to `size` bytes, truncating or zero-filling.
    fn resize_bytes(&mut self, size: usize) -> Result<(), IOError>;

    /// Set the resource's size to `size` bits. Defaults to
    /// [`resize_bytes`](RawIOBase::resize_bytes) on the enclosing whole bytes, so
    /// byte-granular resources round up; bit-granular resources override it to be
    /// exact.
    fn resize_bits(&mut self, size: usize) -> Result<(), IOError> {
        self.resize_bytes(size.div_ceil(8))
    }

    /// Read one byte at `position` (in bytes) relative to `whence`.
    fn pread_byte_one(&self, position: usize, whence: Whence) -> Result<u8, IOError> {
        crate::log_event!(
            trace,
            "RawIOBase::pread_byte_one position={position} whence={whence:?}"
        );
        self.pread_byte_array(position, whence, 1)?
            .into_iter()
            .next()
            .ok_or(IOError::UnexpectedEof {
                requested: 1,
                available: 0,
            })
    }

    /// Write one byte at `position` (in bytes) relative to `whence`.
    fn pwrite_byte_one(
        &mut self,
        position: usize,
        whence: Whence,
        value: u8,
    ) -> Result<(), IOError> {
        crate::log_event!(
            trace,
            "RawIOBase::pwrite_byte_one position={position} whence={whence:?}"
        );
        self.pwrite_byte_array(position, whence, std::slice::from_ref(&value))
    }

    /// Read `size` bytes starting at `position` (in bytes) relative to `whence`.
    fn pread_byte_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<u8>, IOError>;

    /// Write `values` starting at `position` (in bytes) relative to `whence`. An
    /// empty `values` is a no-op and never grows the resource.
    fn pwrite_byte_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[u8],
    ) -> Result<(), IOError>;

    /// Read one bit at `position` (in bits) relative to `whence`.
    fn pread_bit_one(&self, position: usize, whence: Whence) -> Result<bool, IOError> {
        crate::log_event!(
            trace,
            "RawIOBase::pread_bit_one position={position} whence={whence:?}"
        );
        self.pread_bit_array(position, whence, 1)?
            .into_iter()
            .next()
            .ok_or(IOError::UnexpectedEof {
                requested: 1,
                available: 0,
            })
    }

    /// Write one bit at `position` (in bits) relative to `whence`.
    fn pwrite_bit_one(
        &mut self,
        position: usize,
        whence: Whence,
        value: bool,
    ) -> Result<(), IOError> {
        crate::log_event!(
            trace,
            "RawIOBase::pwrite_bit_one position={position} whence={whence:?}"
        );
        self.pwrite_bit_array(position, whence, std::slice::from_ref(&value))
    }

    /// Read `size` bits starting at `position` (in bits) relative to `whence`.
    fn pread_bit_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<bool>, IOError>;

    /// Write `values` starting at `position` (in bits) relative to `whence`. An
    /// empty `values` is a no-op and never grows the resource.
    fn pwrite_bit_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[bool],
    ) -> Result<(), IOError>;

    /// Read one little-endian `i8` at `position` (in bytes) relative to `whence`.
    ///
    /// The `pread_*` / `pwrite_*` primitive helpers share one shape: every Rust
    /// numeric primitive reads and writes as its fixed little-endian
    /// bytes through the byte-array primitives, so any resource gets the whole
    /// numeric surface for free.
    fn pread_i8(&self, position: usize, whence: Whence) -> Result<i8, IOError> {
        let bytes = self.pread_byte_array(position, whence, 1)?;
        Ok(i8::from_le_bytes(bytes.try_into().expect(
            "pread_byte_array returns exactly the requested number of bytes",
        )))
    }

    /// Write one `i8` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`. See [`pread_i8`](RawIOBase::pread_i8) for the shared
    /// shape.
    fn pwrite_i8(&mut self, position: usize, whence: Whence, value: i8) -> Result<(), IOError> {
        self.pwrite_byte_array(position, whence, &value.to_le_bytes())
    }

    /// Read one little-endian `i16` at `position` (in bytes) relative to `whence`.
    /// See [`pread_i8`](RawIOBase::pread_i8) for the shared shape.
    fn pread_i16(&self, position: usize, whence: Whence) -> Result<i16, IOError> {
        let bytes = self.pread_byte_array(position, whence, 2)?;
        Ok(i16::from_le_bytes(bytes.try_into().expect(
            "pread_byte_array returns exactly the requested number of bytes",
        )))
    }

    /// Write one `i16` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`. See [`pread_i8`](RawIOBase::pread_i8) for the shared
    /// shape.
    fn pwrite_i16(&mut self, position: usize, whence: Whence, value: i16) -> Result<(), IOError> {
        self.pwrite_byte_array(position, whence, &value.to_le_bytes())
    }

    /// Read one little-endian `i32` at `position` (in bytes) relative to `whence`.
    /// See [`pread_i8`](RawIOBase::pread_i8) for the shared shape.
    fn pread_i32(&self, position: usize, whence: Whence) -> Result<i32, IOError> {
        let bytes = self.pread_byte_array(position, whence, 4)?;
        Ok(i32::from_le_bytes(bytes.try_into().expect(
            "pread_byte_array returns exactly the requested number of bytes",
        )))
    }

    /// Write one `i32` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`. See [`pread_i8`](RawIOBase::pread_i8) for the shared
    /// shape.
    fn pwrite_i32(&mut self, position: usize, whence: Whence, value: i32) -> Result<(), IOError> {
        self.pwrite_byte_array(position, whence, &value.to_le_bytes())
    }

    /// Read one little-endian `i64` at `position` (in bytes) relative to `whence`.
    /// See [`pread_i8`](RawIOBase::pread_i8) for the shared shape.
    fn pread_i64(&self, position: usize, whence: Whence) -> Result<i64, IOError> {
        let bytes = self.pread_byte_array(position, whence, 8)?;
        Ok(i64::from_le_bytes(bytes.try_into().expect(
            "pread_byte_array returns exactly the requested number of bytes",
        )))
    }

    /// Write one `i64` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`. See [`pread_i8`](RawIOBase::pread_i8) for the shared
    /// shape.
    fn pwrite_i64(&mut self, position: usize, whence: Whence, value: i64) -> Result<(), IOError> {
        self.pwrite_byte_array(position, whence, &value.to_le_bytes())
    }

    /// Read one little-endian `u8` at `position` (in bytes) relative to `whence`.
    /// See [`pread_i8`](RawIOBase::pread_i8) for the shared shape.
    fn pread_u8(&self, position: usize, whence: Whence) -> Result<u8, IOError> {
        let bytes = self.pread_byte_array(position, whence, 1)?;
        Ok(u8::from_le_bytes(bytes.try_into().expect(
            "pread_byte_array returns exactly the requested number of bytes",
        )))
    }

    /// Write one `u8` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`. See [`pread_i8`](RawIOBase::pread_i8) for the shared
    /// shape.
    fn pwrite_u8(&mut self, position: usize, whence: Whence, value: u8) -> Result<(), IOError> {
        self.pwrite_byte_array(position, whence, &value.to_le_bytes())
    }

    /// Read one little-endian `u16` at `position` (in bytes) relative to `whence`.
    /// See [`pread_i8`](RawIOBase::pread_i8) for the shared shape.
    fn pread_u16(&self, position: usize, whence: Whence) -> Result<u16, IOError> {
        let bytes = self.pread_byte_array(position, whence, 2)?;
        Ok(u16::from_le_bytes(bytes.try_into().expect(
            "pread_byte_array returns exactly the requested number of bytes",
        )))
    }

    /// Write one `u16` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`. See [`pread_i8`](RawIOBase::pread_i8) for the shared
    /// shape.
    fn pwrite_u16(&mut self, position: usize, whence: Whence, value: u16) -> Result<(), IOError> {
        self.pwrite_byte_array(position, whence, &value.to_le_bytes())
    }

    /// Read one little-endian `u32` at `position` (in bytes) relative to `whence`.
    /// See [`pread_i8`](RawIOBase::pread_i8) for the shared shape.
    fn pread_u32(&self, position: usize, whence: Whence) -> Result<u32, IOError> {
        let bytes = self.pread_byte_array(position, whence, 4)?;
        Ok(u32::from_le_bytes(bytes.try_into().expect(
            "pread_byte_array returns exactly the requested number of bytes",
        )))
    }

    /// Write one `u32` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`. See [`pread_i8`](RawIOBase::pread_i8) for the shared
    /// shape.
    fn pwrite_u32(&mut self, position: usize, whence: Whence, value: u32) -> Result<(), IOError> {
        self.pwrite_byte_array(position, whence, &value.to_le_bytes())
    }

    /// Read one little-endian `u64` at `position` (in bytes) relative to `whence`.
    /// See [`pread_i8`](RawIOBase::pread_i8) for the shared shape.
    fn pread_u64(&self, position: usize, whence: Whence) -> Result<u64, IOError> {
        let bytes = self.pread_byte_array(position, whence, 8)?;
        Ok(u64::from_le_bytes(bytes.try_into().expect(
            "pread_byte_array returns exactly the requested number of bytes",
        )))
    }

    /// Write one `u64` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`. See [`pread_i8`](RawIOBase::pread_i8) for the shared
    /// shape.
    fn pwrite_u64(&mut self, position: usize, whence: Whence, value: u64) -> Result<(), IOError> {
        self.pwrite_byte_array(position, whence, &value.to_le_bytes())
    }

    /// Read one little-endian `f32` at `position` (in bytes) relative to `whence`.
    /// See [`pread_i8`](RawIOBase::pread_i8) for the shared shape.
    fn pread_f32(&self, position: usize, whence: Whence) -> Result<f32, IOError> {
        let bytes = self.pread_byte_array(position, whence, 4)?;
        Ok(f32::from_le_bytes(bytes.try_into().expect(
            "pread_byte_array returns exactly the requested number of bytes",
        )))
    }

    /// Write one `f32` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`. See [`pread_i8`](RawIOBase::pread_i8) for the shared
    /// shape.
    fn pwrite_f32(&mut self, position: usize, whence: Whence, value: f32) -> Result<(), IOError> {
        self.pwrite_byte_array(position, whence, &value.to_le_bytes())
    }

    /// Read one little-endian `f64` at `position` (in bytes) relative to `whence`.
    /// See [`pread_i8`](RawIOBase::pread_i8) for the shared shape.
    fn pread_f64(&self, position: usize, whence: Whence) -> Result<f64, IOError> {
        let bytes = self.pread_byte_array(position, whence, 8)?;
        Ok(f64::from_le_bytes(bytes.try_into().expect(
            "pread_byte_array returns exactly the requested number of bytes",
        )))
    }

    /// Write one `f64` as its little-endian bytes at `position` (in bytes)
    /// relative to `whence`. See [`pread_i8`](RawIOBase::pread_i8) for the shared
    /// shape.
    fn pwrite_f64(&mut self, position: usize, whence: Whence, value: f64) -> Result<(), IOError> {
        self.pwrite_byte_array(position, whence, &value.to_le_bytes())
    }

    /// Consume this resource into a [`RawIOCursor`], a moving cursor over it that
    /// advances on every read and write.
    fn cursor(self) -> RawIOCursor<Self>
    where
        Self: Sized,
    {
        RawIOCursor::new(self)
    }

    /// Consume this resource into a [`RawIOSlice`], a view bounded to the byte
    /// window `[start, end)`.
    fn slice(self, start: usize, end: usize) -> RawIOSlice<Self>
    where
        Self: Sized,
    {
        RawIOSlice::new(self, start, end)
    }

    /// Stream `size` bytes from `self` (at `position` relative to `whence`) into
    /// `sink` (at `sink_position` relative to `sink_whence`), copying in chunks so
    /// a large transfer never materializes in full.
    ///
    /// The sink's start is resolved once against its current
    /// [`byte_size`](RawIOBase::byte_size), so `sink_whence` — notably
    /// [`Whence::End`] — stays anchored even while the sink grows during the copy.
    fn pread_raw_io(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
        sink: &mut dyn RawIOBase,
        sink_position: usize,
        sink_whence: Whence,
    ) -> Result<(), IOError> {
        crate::log_event!(debug, "RawIOBase::pread_raw_io size={size}");
        let sink_start = resolve_byte_start(sink.byte_size(), sink_position, sink_whence)?;
        let mut copied = 0;
        while copied < size {
            let chunk = STREAM_CHUNK.min(size - copied);
            let bytes = self.pread_byte_array(position + copied, whence, chunk)?;
            sink.pwrite_byte_array(sink_start + copied, Whence::Start, &bytes)?;
            copied += chunk;
        }
        Ok(())
    }

    /// Stream `size` bytes from `source` (at `source_position` relative to
    /// `source_whence`) into `self` (at `position` relative to `whence`), copying
    /// in chunks so a large transfer never materializes in full.
    ///
    /// `self`'s start is resolved once against its current
    /// [`byte_size`](RawIOBase::byte_size), so `whence` — notably [`Whence::End`] —
    /// stays anchored even while `self` grows during the copy.
    fn pwrite_raw_io(
        &mut self,
        position: usize,
        whence: Whence,
        source: &dyn RawIOBase,
        source_position: usize,
        source_whence: Whence,
        size: usize,
    ) -> Result<(), IOError> {
        crate::log_event!(debug, "RawIOBase::pwrite_raw_io size={size}");
        let start = resolve_byte_start(self.byte_size(), position, whence)?;
        let mut copied = 0;
        while copied < size {
            let chunk = STREAM_CHUNK.min(size - copied);
            let bytes = source.pread_byte_array(source_position + copied, source_whence, chunk)?;
            self.pwrite_byte_array(start + copied, Whence::Start, &bytes)?;
            copied += chunk;
        }
        Ok(())
    }
}

/// Resolve a `(position, whence)` pair to an absolute byte offset against a
/// cursorless resource of `size` bytes: [`Whence::End`] is measured from the end
/// (`size`), while [`Whence::Start`] and (having no cursor) [`Whence::Current`] are
/// measured from the start. Guards the addition against overflow.
fn resolve_byte_start(size: usize, position: usize, whence: Whence) -> Result<usize, IOError> {
    let base = match whence {
        Whence::End => size,
        _ => 0,
    };
    bits::offset(base, position)
}
