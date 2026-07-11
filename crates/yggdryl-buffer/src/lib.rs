//! **yggdryl-buffer** — typed, immutable, cheaply-shared buffers for the native
//! fixed-width types; the top of the layer stack (buffer → field → dtype → core).
//!
//! Where [`ByteBuffer`](yggdryl_core::ByteBuffer) is untyped byte storage, a buffer here
//! is a contiguous run of one native primitive — one type per Rust primitive
//! ([`I8Buffer`] … [`F64Buffer`]) plus the bit-packed [`BooleanBuffer`]. Each shares
//! its allocation on clone, hands out an aligned typed view, round-trips through
//! little-endian bytes ([`serialize_bytes`](I64Buffer::serialize_bytes) /
//! `deserialize_bytes`, validated against the element width — [`BufferError`]),
//! compares and hashes by content, and bridges to positioned IO via
//! [`byte_cursor`](I64Buffer::byte_cursor). Each **is** the matching Arrow
//! `ScalarBuffer` (Arrow-backed), so `from_arrow` / `to_arrow` share the allocation
//! zero-copy; that Arrow interop is Rust-only (an `arrow_buffer` value does not cross
//! the FFI boundary), like [`ByteBuffer`](yggdryl_core::ByteBuffer)'s.
//!
//! A buffer also carries optional [`Headers`](yggdryl_http::Headers) and hands out the
//! matching typed [`Field`](yggdryl_field::Field) via [`field`](I64Buffer::field)
//! (`I64Buffer::field` → [`I64Field`](yggdryl_field::I64Field)).
//!
//! The numeric buffers are stamped out from one shared implementation (the
//! `primitive_buffer!` macro), mirroring the IO layer's `primitive_io!`.
//!
//! ```
//! use yggdryl_buffer::I64Buffer;
//! use yggdryl_field::Field;
//! use yggdryl_http::{Headers, HeadersBased};
//!
//! let buffer = I64Buffer::from_slice(&[1, 2, 3])
//!     .with_headers(Headers::from_pairs([(b"unit".to_vec(), b"ms".to_vec())]));
//!
//! // Hand out the matching typed field (an `I64Field`), carrying the buffer's headers.
//! let field = buffer.field("ts", true);
//! assert_eq!(field.name(), "ts");
//! assert!(field.is_nullable());
//! assert_eq!(field.get_header(b"unit"), Some(b"ms".as_slice()));
//!
//! // Headers are an annotation — they do not change the buffer's byte identity.
//! assert_eq!(buffer, I64Buffer::from_slice(&[1, 2, 3]));
//! ```

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
