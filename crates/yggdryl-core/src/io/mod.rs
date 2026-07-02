//! Positioned byte- and bit-I/O: the [`Seekable`] cursor, the low-level
//! [`RawIOBase`] trait built on it, the typed [`IOBase`] layer, the [`Whence`]
//! reference point, and the concrete [`ByteBuffer`] / [`BitBuffer`] resources.

mod bit_buffer;
mod bits;
mod byte_buffer;
mod error;
mod seekable;
mod typed;
mod whence;

pub use bit_buffer::BitBuffer;
pub use byte_buffer::ByteBuffer;
pub use error::IOError;
pub use seekable::Seekable;
pub use typed::IOBase;
pub use whence::Whence;

/// Bytes copied per chunk by the default `pread_io` / `pwrite_io` streams, so a
/// large transfer never materializes in full.
const STREAM_CHUNK: usize = 64 * 1024;

/// Positioned reads and writes over a resource, one or many `u8` bytes or `bool`
/// bits at a time.
///
/// A resource is [`Seekable`], so [`Whence::Current`] is measured from its cursor
/// ([`tell`](Seekable::tell)); positioned access does not move the cursor, whereas
/// [`seek`](Seekable::seek) does.
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
/// accessors, bit sizes, capacities, and the [`pread_io`](RawIOBase::pread_io) /
/// [`pwrite_io`](RawIOBase::pwrite_io) streams — comes for free from default
/// implementations.
///
/// ```
/// use yggdryl_core::{ByteBuffer, RawIOBase, Seekable, Whence};
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
/// // The cursor: seek moves it, positioned access does not.
/// buf.seek(1, Whence::Start)?;
/// assert_eq!(buf.pread_byte_one(0, Whence::Current)?, 7);
///
/// // Stream into another resource, chunked — no whole-copy materialization.
/// let mut sink = ByteBuffer::new();
/// buf.pread_io(0, Whence::Start, 4, &mut sink, 0, Whence::Start)?;
/// assert_eq!(sink.as_bytes(), buf.as_bytes());
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
pub trait RawIOBase: Seekable {
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

    /// Write `values` starting at `position` (in bytes) relative to `whence`.
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

    /// Write `values` starting at `position` (in bits) relative to `whence`.
    fn pwrite_bit_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[bool],
    ) -> Result<(), IOError>;

    /// Stream `size` bytes from `self` (at `position` relative to `whence`) into
    /// `sink` (at `sink_position` relative to `sink_whence`), copying in chunks so
    /// a large transfer never materializes in full.
    ///
    /// The sink's start is resolved once through its own cursor
    /// ([`seek`](Seekable::seek), then restored), so `sink_whence` stays correct
    /// even while the sink grows during the copy.
    fn pread_io(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
        sink: &mut dyn RawIOBase,
        sink_position: usize,
        sink_whence: Whence,
    ) -> Result<(), IOError> {
        crate::log_event!(debug, "RawIOBase::pread_io size={size}");
        let saved = sink.tell();
        let sink_start = sink.seek(sink_position, sink_whence)?;
        sink.seek(saved, Whence::Start)?;
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
    /// `self`'s start is resolved once through its own cursor
    /// ([`seek`](Seekable::seek), then restored), so `whence` stays correct even
    /// while `self` grows during the copy.
    fn pwrite_io(
        &mut self,
        position: usize,
        whence: Whence,
        source: &dyn RawIOBase,
        source_position: usize,
        source_whence: Whence,
        size: usize,
    ) -> Result<(), IOError> {
        crate::log_event!(debug, "RawIOBase::pwrite_io size={size}");
        let saved = self.tell();
        let start = self.seek(position, whence)?;
        self.seek(saved, Whence::Start)?;
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
