//! The `primitive_buffer!` macro — the single source of the numeric buffer logic,
//! stamped out once per native type (`I8Buffer`, …, `F64Buffer`) so every buffer
//! shares one implementation, mirroring the `primitive_io!` macro in the IO layer.

/// Generates one immutable, cheaply-shared contiguous buffer type for a fixed-width
/// native primitive `$ty`, named `$name`, whose [`field`](Self::field) accessor hands
/// out a `$field` (e.g. `I64Buffer` → `I64Field`).
///
/// The backing is an `arrow_buffer::ScalarBuffer<$ty>` (Arrow-backed), so an Arrow
/// buffer is wrapped **zero-copy** and an owned `Vec` becomes one without a copy; it
/// hands out an aligned `&[$ty]` view. The buffer also carries optional
/// [`Headers`](yggdryl_http::Headers) — an annotation that does **not** affect its
/// byte-content equality/hashing.
macro_rules! primitive_buffer {
    ($name:ident, $ty:ty, $field:ident) => {
        #[doc = concat!("An immutable, cheaply-shared contiguous buffer of `", stringify!($ty), "` values.")]
        ///
        /// Cloning shares the allocation (an `Arc`/Arrow refcount bump). The buffer
        /// exposes an aligned typed view ([`as_slice`](Self::as_slice)), element
        /// access ([`get`](Self::get)), and round-trips through little-endian bytes
        /// ([`serialize_bytes`](Self::serialize_bytes) /
        /// [`deserialize_bytes`](Self::deserialize_bytes)); equality and hashing are
        /// by byte content, so two buffers are equal iff their `serialize_bytes` are
        /// (attached [`headers`](Self::headers) do not affect this). Bridge to
        /// positioned IO with [`byte_cursor`](Self::byte_cursor), and hand out the
        #[doc = concat!("matching [`", stringify!($field), "`](yggdryl_field::", stringify!($field), ") with [`field`](Self::field).")]
        ///
        #[doc = concat!("```")]
        #[doc = concat!("use yggdryl_buffer::", stringify!($name), ";")]
        #[doc = concat!("let buffer = ", stringify!($name), "::from_slice(&[1 as ", stringify!($ty), ", 2 as ", stringify!($ty), ", 3 as ", stringify!($ty), "]);")]
        #[doc = concat!("assert_eq!(buffer.len(), 3);")]
        #[doc = concat!("assert_eq!(buffer.get(1), Some(2 as ", stringify!($ty), "));")]
        #[doc = concat!("let bytes = buffer.serialize_bytes();")]
        #[doc = concat!("assert_eq!(", stringify!($name), "::deserialize_bytes(&bytes).unwrap(), buffer);")]
        #[doc = concat!("```")]
        #[derive(Clone)]
        pub struct $name {
            data: arrow_buffer::ScalarBuffer<$ty>,
            headers: Option<yggdryl_http::Headers>,
        }

        impl $name {
            /// Creates an empty buffer.
            pub fn new() -> Self {
                Self::from_vec(Vec::new())
            }

            #[doc = concat!("Creates a buffer taking ownership of `values` (no copy of the `", stringify!($ty), "` data).")]
            pub fn from_vec(values: Vec<$ty>) -> Self {
                Self {
                    data: arrow_buffer::ScalarBuffer::from(values),
                    headers: None,
                }
            }

            #[doc = concat!("Builds the matching [`", stringify!($field), "`](yggdryl_field::", stringify!($field), ") named `name` (nullable `nullable`), carrying this buffer's headers.")]
            pub fn field(&self, name: impl Into<String>, nullable: bool) -> yggdryl_field::$field {
                let field = yggdryl_field::$field::new(name, nullable);
                match &self.headers {
                    Some(headers) => {
                        yggdryl_http::HeadersBased::with_headers(field, headers.clone())
                    }
                    None => field,
                }
            }

            /// Creates a buffer holding a copy of `values`.
            pub fn from_slice(values: &[$ty]) -> Self {
                Self::from_vec(values.to_vec())
            }

            /// The number of values held.
            pub fn len(&self) -> usize {
                self.data.len()
            }

            /// Whether the buffer holds no values.
            pub fn is_empty(&self) -> bool {
                self.data.is_empty()
            }

            /// Borrows the values as an aligned slice.
            pub fn as_slice(&self) -> &[$ty] {
                &self.data[..]
            }

            /// The value at `index`, or `None` if out of bounds.
            pub fn get(&self, index: usize) -> Option<$ty> {
                self.as_slice().get(index).copied()
            }

            /// Copies the values out into an owned `Vec`.
            pub fn to_vec(&self) -> Vec<$ty> {
                self.as_slice().to_vec()
            }

            /// Borrows the backing values as their raw bytes (host-endian; equals the
            /// little-endian [`serialize_bytes`](Self::serialize_bytes) on a
            /// little-endian target). Zero-copy — no allocation.
            pub fn as_bytes(&self) -> &[u8] {
                let values = self.as_slice();
                // SAFETY: `$ty` is a fixed-width numeric primitive with no padding, and
                // `u8` has alignment 1, so reinterpreting the value slice as bytes is
                // sound; the borrow is tied to `&self`.
                unsafe {
                    core::slice::from_raw_parts(
                        values.as_ptr().cast::<u8>(),
                        core::mem::size_of_val(values),
                    )
                }
            }

            /// Serialises the values to their little-endian bytes.
            pub fn serialize_bytes(&self) -> Vec<u8> {
                #[cfg(target_endian = "little")]
                {
                    self.as_bytes().to_vec()
                }
                #[cfg(not(target_endian = "little"))]
                {
                    let values = self.as_slice();
                    let mut out = Vec::with_capacity(core::mem::size_of_val(values));
                    for value in values {
                        out.extend_from_slice(&value.to_le_bytes());
                    }
                    out
                }
            }

            #[doc = concat!("Reconstructs a buffer from little-endian `", stringify!($ty), "` bytes.")]
            ///
            /// # Errors
            /// [`BufferError::InvalidByteLength`](crate::BufferError::InvalidByteLength)
            /// if `bytes.len()` is not a multiple of the element width.
            // `W` is 1 for the byte buffers (`I8Buffer` / `U8Buffer`), where the width
            // check is a harmless no-op — every length is valid.
            #[allow(clippy::modulo_one)]
            pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, $crate::BufferError> {
                const W: usize = core::mem::size_of::<$ty>();
                if bytes.len() % W != 0 {
                    return Err($crate::BufferError::InvalidByteLength {
                        len: bytes.len(),
                        width: W,
                        ty: stringify!($ty),
                    });
                }
                let values = bytes
                    .chunks_exact(W)
                    .map(|chunk| {
                        <$ty>::from_le_bytes(chunk.try_into().expect("chunks_exact yields W bytes"))
                    })
                    .collect();
                Ok(Self::from_vec(values))
            }

            /// Freezes the values into a [`ByteBuffer`](yggdryl_core::ByteBuffer) of their
            /// little-endian bytes, for positioned IO.
            pub fn to_byte_buffer(&self) -> yggdryl_core::ByteBuffer {
                yggdryl_core::ByteBuffer::from_vec(self.serialize_bytes())
            }

            /// Opens a [`ByteCursor`](yggdryl_core::ByteCursor) over the values' bytes.
            pub fn byte_cursor(&self) -> yggdryl_core::ByteCursor {
                self.to_byte_buffer().byte_cursor()
            }

            #[doc = concat!("Opens a [`TypedCursor`](yggdryl_core::TypedCursor) over the values (native `", stringify!($ty), "` units).")]
            pub fn cursor(&self) -> yggdryl_core::TypedCursor<$ty> {
                yggdryl_core::TypedCursor::new(self.to_byte_buffer())
            }

            #[doc = concat!("Opens a [`TypedSlice`](yggdryl_core::TypedSlice) over the `offset..offset+len` window of values (in `", stringify!($ty), "` units), clamped to the buffer.")]
            pub fn slice(&self, offset: usize, len: usize) -> yggdryl_core::TypedSlice<$ty> {
                const W: usize = core::mem::size_of::<$ty>();
                yggdryl_core::TypedSlice::new(
                    self.to_byte_buffer(),
                    (offset as u64).saturating_mul(W as u64),
                    len.saturating_mul(W),
                )
            }

            #[doc = concat!("Decodes a [`ByteBuffer`](yggdryl_core::ByteBuffer) of little-endian `", stringify!($ty), "` bytes.")]
            ///
            /// # Errors
            /// As [`deserialize_bytes`](Self::deserialize_bytes).
            pub fn from_byte_buffer(buffer: &yggdryl_core::ByteBuffer) -> Result<Self, $crate::BufferError> {
                Self::deserialize_bytes(buffer.as_bytes())
            }

            /// Wraps an Arrow `ScalarBuffer` **zero-copy** — the two share the same
            /// underlying allocation (reference-counted).
            pub fn from_arrow(buffer: arrow_buffer::ScalarBuffer<$ty>) -> Self {
                Self {
                    data: buffer,
                    headers: None,
                }
            }

            /// Exports the values as an Arrow `ScalarBuffer` — **zero-copy** (an `Arc`
            /// bump).
            pub fn to_arrow(&self) -> arrow_buffer::ScalarBuffer<$ty> {
                self.data.clone()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("len", &self.len())
                    .finish()
            }
        }

        // Value semantics by byte content: equal iff `serialize_bytes` are equal, and
        // equal values hash equal (`CLAUDE.md` rule 7). Byte-based so it works for the
        // float buffers too, where the values are not `Eq`/`Hash` (bitwise identity).
        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                self.as_bytes() == other.as_bytes()
            }
        }

        impl Eq for $name {}

        impl core::hash::Hash for $name {
            fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
                self.as_bytes().hash(state);
            }
        }

        // Header get / add / update / delete + the `with_headers` builder come from this
        // one trait (shared with the field layer); the buffer only supplies the slot.
        impl yggdryl_http::HeadersBased for $name {
            fn headers(&self) -> Option<&yggdryl_http::Headers> {
                self.headers.as_ref()
            }

            fn headers_mut(&mut self) -> &mut Option<yggdryl_http::Headers> {
                &mut self.headers
            }
        }
    };
}

pub(crate) use primitive_buffer;
