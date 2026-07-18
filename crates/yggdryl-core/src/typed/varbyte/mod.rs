//! `varbyte` — **variable-length, byte-granular** element types (UTF-8 strings, binary) —
//! *reserved*.
//!
//! A variable-length column is two buffers: an `i32`/`i64` **offsets** [`IOBase`] and a packed
//! **data** [`IOBase`] (element `i` is `data[offsets[i]..offsets[i + 1]]`), plus the same validity
//! bitmap the fixed families use. The [`Encoder`](super::Encoder) appends to the data buffer and
//! pushes the running offset; the [`Decoder`](super::Decoder) reads the `[start, end)` slice — both
//! still over the one [`IOBase`] contract, so a `Utf8` / `Binary` column memory-maps and streams
//! exactly like a fixed one.
//!
//! This module fixes the seam so the `Serie` / `Field` shape stays uniform across fixed and
//! variable types; the concrete `Utf8` / `Binary` impls (offsets + data + validity) drop in here.
