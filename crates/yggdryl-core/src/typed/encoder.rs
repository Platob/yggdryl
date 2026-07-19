//! [`Encoder`] — writes a **native Rust value** as an element into any [`IOBase`] source.
//!
//! The encoder is stateless: it maps an **element index** (not a byte offset) to the physical
//! position and writes there, so the caller thinks in elements and the type owns its bit/byte
//! stride. The bulk [`encode_slice`](Encoder::encode_slice) forwards to the source's **vectorized**
//! typed array write (`pwrite_i32_array`, …), so a whole column encodes in one dense pass.

use super::{DataType, FlexibleFromStr};
use crate::io::memory::{IOBase, IoError};

/// Encodes [`Native`](DataType::Native) values as elements of this type into an [`IOBase`].
pub trait Encoder: DataType {
    /// Writes `value` as the element at `index` (an **element** index; the impl maps it to the
    /// physical bit/byte position).
    fn encode<W: IOBase>(dst: &mut W, index: u64, value: Self::Native) -> Result<(), IoError>;

    /// Writes `values` as the contiguous elements starting at element `start` — the **bulk** path,
    /// forwarding to the source's vectorized typed array write.
    fn encode_slice<W: IOBase>(
        dst: &mut W,
        start: u64,
        values: &[Self::Native],
    ) -> Result<(), IoError>;

    /// Parses `s` with the tolerant [`parse_flexible`](FlexibleFromStr::parse_flexible) (thousands
    /// separators, `0x`/`0b`/`0o` radices, `1e3` scientific, `+`/whitespace) and writes the value as
    /// the element at `index`. A value the type cannot represent surfaces the guided
    /// [`IoError::ParseError`].
    ///
    /// ```
    /// use yggdryl_core::io::memory::Heap;
    /// use yggdryl_core::typed::{Decoder, Encoder};
    /// use yggdryl_core::typed::fixedbyte::Int64;
    ///
    /// let mut h = Heap::new();
    /// Int64::encode_str(&mut h, 0, "1,000").unwrap(); // tolerant: strips the separator
    /// assert_eq!(Int64::decode(&h, 0).unwrap(), 1000);
    /// ```
    fn encode_str<W: IOBase>(dst: &mut W, index: u64, s: &str) -> Result<(), IoError>
    where
        Self::Native: FlexibleFromStr,
    {
        let value = <Self::Native as FlexibleFromStr>::parse_flexible(s)?;
        Self::encode(dst, index, value)
    }

    /// The **bulk** twin of [`encode_str`](Encoder::encode_str): parses every string once into a
    /// pre-sized `Vec`, then writes them in a single vectorized [`encode_slice`](Encoder::encode_slice)
    /// (never element-by-element into the buffer).
    fn encode_str_slice<W: IOBase>(dst: &mut W, start: u64, values: &[&str]) -> Result<(), IoError>
    where
        Self::Native: FlexibleFromStr,
    {
        let mut parsed = Vec::with_capacity(values.len());
        for s in values {
            parsed.push(<Self::Native as FlexibleFromStr>::parse_flexible(s)?);
        }
        Self::encode_slice(dst, start, &parsed)
    }

    /// The **strict** twin of [`encode_str`](Encoder::encode_str): parses with
    /// [`parse_exact`](FlexibleFromStr::parse_exact) (`str::parse`, no coercion).
    fn encode_str_exact<W: IOBase>(dst: &mut W, index: u64, s: &str) -> Result<(), IoError>
    where
        Self::Native: FlexibleFromStr,
    {
        let value = <Self::Native as FlexibleFromStr>::parse_exact(s)?;
        Self::encode(dst, index, value)
    }

    /// The strict, **bulk** twin: [`parse_exact`](FlexibleFromStr::parse_exact) every string into a
    /// pre-sized `Vec`, then a single vectorized [`encode_slice`](Encoder::encode_slice).
    fn encode_str_exact_slice<W: IOBase>(
        dst: &mut W,
        start: u64,
        values: &[&str],
    ) -> Result<(), IoError>
    where
        Self::Native: FlexibleFromStr,
    {
        let mut parsed = Vec::with_capacity(values.len());
        for s in values {
            parsed.push(<Self::Native as FlexibleFromStr>::parse_exact(s)?);
        }
        Self::encode_slice(dst, start, &parsed)
    }
}
