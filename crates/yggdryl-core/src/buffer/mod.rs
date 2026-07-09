//! Typed, immutable, cheaply-shared buffers for the native fixed-width types.
//!
//! Where [`ByteBuffer`](crate::ByteBuffer) is untyped byte storage, a buffer here is
//! a contiguous run of one native primitive — one type per Rust primitive
//! ([`I8Buffer`] … [`F64Buffer`]) plus the bit-packed [`BooleanBuffer`]. Each shares
//! its allocation on clone, hands out an aligned typed view, round-trips through
//! little-endian bytes ([`serialize_bytes`](I64Buffer::serialize_bytes) /
//! `deserialize_bytes`, validated against the element width — [`BufferError`]),
//! compares and hashes by content, and bridges to positioned IO via
//! [`byte_cursor`](I64Buffer::byte_cursor). Each **is** the matching Arrow
//! `ScalarBuffer` (the core is Arrow-backed), so `from_arrow` / `to_arrow` share the
//! allocation zero-copy; that Arrow interop is Rust-only (an `arrow_buffer` value
//! does not cross the FFI boundary), like [`ByteBuffer`](crate::ByteBuffer)'s.
//!
//! The numeric buffers are stamped out from one shared implementation (the
//! `primitive_buffer!` macro), mirroring the IO layer's `primitive_io!`.

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
mod u8_buffer;

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
pub use u8_buffer::U8Buffer;
