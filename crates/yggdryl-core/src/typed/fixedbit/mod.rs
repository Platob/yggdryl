//! `fixedbit` — **fixed-length, bit-granular** element types: the boolean [`Bit`].
//!
//! Unlike the byte-granular [`fixedbyte`](super::fixedbyte) types, a bit packs at a **bit** stride,
//! so its [`Encoder`](super::Encoder) / [`Decoder`](super::Decoder) route through the source's LSB-first
//! bit primitives (`pwrite_bit` / `pread_bit`). A boolean does not sum, so `Bit` is not
//! [`Reduce`](super::Reduce).

mod bit;

pub use bit::Bit;
