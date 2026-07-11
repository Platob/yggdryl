//! **yggdryl-buffer** — the dependency-light **foundation** crate: Arrow-backed byte and
//! typed buffers, positioned byte/element IO, and the wide integers, on which every other
//! yggdryl crate and binding builds (its only dependency is `arrow-buffer`).
//!
//! Three things live here, one concern per module:
//!
//! - **Buffers.** [`ByteBuffer`] is untyped byte storage; a typed buffer is a contiguous
//!   run of one native primitive ([`I8Buffer`] … [`F64Buffer`]) plus the bit-packed
//!   [`BooleanBuffer`]. The `u8` buffer *is* [`ByteBuffer`] — [`U8Buffer`] is an alias, so
//!   the byte store and the `u8` typed buffer are one type. Each shares its allocation on
//!   clone, hands out an aligned typed view, round-trips through little-endian bytes
//!   ([`serialize_bytes`](I64Buffer::serialize_bytes) / `deserialize_bytes`, validated
//!   against the element width — [`BufferError`]), compares/hashes by content, and **is**
//!   the matching Arrow buffer (`from_arrow` / `to_arrow` share the allocation zero-copy;
//!   that Arrow interop is Rust-only — an `arrow_buffer` value does not cross the FFI
//!   boundary).
//! - **IO** ([`io`]). Every buffer bridges to positioned IO: a [`ByteBuffer::byte_cursor`]
//!   or the element-typed [`cursor`](I64Buffer::cursor) / [`slice`](I64Buffer::slice) view.
//!   The contracts are [`IOBase`] / [`TypedIOBase<T>`] with the positioned markers
//!   [`IOCursor`] / [`IOSlice`] (and their typed twins); the concrete resources are the
//!   byte [`ByteCursor`] / [`ByteSlice`] and the element-typed [`TypedCursor<T>`] /
//!   [`TypedSlice<T>`]. The [`IoPrimitive`] element codec stamps each native type.
//! - **Wide integers** ([`int`]). [`i96`] and [`i256`] flank native `i128`, each an
//!   [`IoPrimitive`] a [`TypedCursor<T>`] reads and writes little-endian.
//!
//! A buffer here carries no schema: naming, nullability, and [`Headers`] annotations are
//! applied **from above** (the `yggdryl-field` layer turns a buffer into a `Field`).
//!
//! ```
//! use yggdryl_buffer::{I64Buffer, TypedIOBase, Whence};
//!
//! let buffer = I64Buffer::from_slice(&[1, 2, 3]);
//! assert_eq!(buffer.get(1), Some(2));
//!
//! // Round-trips through little-endian bytes, equal iff the bytes are equal.
//! let bytes = buffer.serialize_bytes();
//! assert_eq!(I64Buffer::deserialize_bytes(&bytes).unwrap(), buffer);
//!
//! // Bridges to positioned IO through a typed cursor.
//! let mut cursor = buffer.cursor();
//! assert_eq!(cursor.pread_array(2, Whence::Start).unwrap(), vec![1, 2]);
//! ```

pub mod int;
pub mod io;

mod primitive;

mod boolean_buffer;
mod buffer_error;
mod f32_buffer;
mod f64_buffer;
mod i16_buffer;
mod i32_buffer;
mod i64_buffer;
mod i8_buffer;
mod u16_buffer;
mod u32_buffer;
mod u64_buffer;

pub use boolean_buffer::BooleanBuffer;
pub use buffer_error::BufferError;
pub use f32_buffer::F32Buffer;
pub use f64_buffer::F64Buffer;
pub use i16_buffer::I16Buffer;
pub use i32_buffer::I32Buffer;
pub use i64_buffer::I64Buffer;
pub use i8_buffer::I8Buffer;
pub use u16_buffer::U16Buffer;
pub use u32_buffer::U32Buffer;
pub use u64_buffer::U64Buffer;

pub use int::{i256, i96};
pub use io::{
    ByteBuffer, ByteCursor, ByteSlice, IOBase, IOCursor, IOSlice, IoError, IoPrimitive,
    TypedCursor, TypedIOBase, TypedIOCursor, TypedIOSlice, TypedSlice, Whence,
};

/// The `u8` typed buffer **is** the byte store: [`ByteBuffer`] backs both, so the two names
/// refer to one type (`CLAUDE.md` rule 1 — one concern, one type). Use `U8Buffer` when
/// reading it as the `u8` member of the typed-buffer family (`I8Buffer` … `U8Buffer` …),
/// and `ByteBuffer` when reading it as untyped byte storage for IO.
pub type U8Buffer = ByteBuffer;

/// Re-export of the exact `arrow-buffer` these buffers are backed by, so callers construct
/// Arrow buffers against a matching version (see
/// [`ByteBuffer::from_arrow_byte_buffer`](io::ByteBuffer::from_arrow_byte_buffer)).
pub use arrow_buffer;
