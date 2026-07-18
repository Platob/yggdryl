//! `varbit` — **variable-length, bit-granular** element types (bit-lists / bitsets) — *reserved*.
//!
//! The bit-granular counterpart of [`varbyte`](super::varbyte): an offsets [`IOBase`] measured in
//! **bits** over a packed bit [`IOBase`], plus validity — element `i` is the bit range
//! `[offsets[i], offsets[i + 1])`. It reuses the same `Encoder` / `Decoder` / `Serie` / `Field`
//! shape as the other three families; the concrete impls drop in here.
