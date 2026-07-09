//! Cursor-oriented byte IO, split `std::io::Cursor`-style into **storage** and
//! **cursor**: a [`ByteBuffer`] holds bytes (no position), and a [`ByteCursor`]
//! (from [`ByteBuffer::byte_cursor`]) holds a share of it plus a position and does
//! the reading/writing — advancing the position and copying the bytes out only on a
//! write, so the buffer stays intact.
//!
//! The contracts are [`IOBase`] (raw bytes + capacity + typed primitives) and
//! [`TypedIOBase<T>`], with the positioned markers [`IOCursor`] / [`TypedIOCursor<T>`]
//! (a growing position over a whole resource) and the bounded markers [`IOSlice`] /
//! [`TypedIOSlice<T>`] (a fixed, non-growing window). They are generic Rust contracts
//! that do not cross the FFI boundary; the concrete resources are replicated in the
//! bindings — the byte [`ByteBuffer`] / [`ByteCursor`] / [`ByteSlice`] and the
//! element-typed [`TypedCursor<T>`] / [`TypedSlice<T>`] (as one concrete class per
//! primitive) — along with the [`Whence`] seek origin. The [`IoPrimitive`] element
//! codec is the `IOBase`-layer mirror of the `buffer` layer's per-type stamping.

mod byte_buffer;
mod byte_cursor;
mod byte_slice;
mod io_base;
mod io_cursor;
mod io_error;
mod io_slice;
mod primitive;
mod typed_cursor;
mod typed_io_base;
mod typed_io_cursor;
mod typed_io_slice;
mod typed_slice;
mod whence;

pub use byte_buffer::ByteBuffer;
pub use byte_cursor::ByteCursor;
pub use byte_slice::ByteSlice;
pub use io_base::IOBase;
pub use io_cursor::IOCursor;
pub use io_error::IoError;
pub use io_slice::IOSlice;
pub use primitive::IoPrimitive;
pub use typed_cursor::TypedCursor;
pub use typed_io_base::TypedIOBase;
pub use typed_io_cursor::TypedIOCursor;
pub use typed_io_slice::TypedIOSlice;
pub use typed_slice::TypedSlice;
pub use whence::Whence;
