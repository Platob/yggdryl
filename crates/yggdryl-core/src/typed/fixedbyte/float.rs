//! The **floating-point** element types — `f16` (half), `f32`, and `f64`. `f32` / `f64` are one
//! [`fixed_numeric!`](super::fixed_numeric) line apiece (native primitives). [`Float16`] has **no
//! native Rust primitive**, so — like [`Decimal256`](super::Decimal256) over [`I256`](super::I256) —
//! its [`Encoder`] / [`Decoder`] / [`Reduce`] impls are hand-written over the [`F16`] carrier: the
//! 2-byte halves ride the source's vectorized `u16` array kernels (a `&[F16]` **is** a `&[u16]`), and
//! its reductions **widen to `f64`** (`sum` / `mean` / `std` / `var` over `to_f32()`). Every float
//! `min` / `max` **ignores NaN**, order-independently.

use crate::io::memory::{IOBase, IoError};
use crate::typed::{DataType, Decoder, Encoder, Reduce};

use super::F16;

fixed_numeric!(
    /// The 32-bit IEEE-754 float type (`f32`).
    Float32, f32, F32, f64,
    pwrite_f32_array, pread_f32_array, pwrite_f32_repeat, sum_f32, min_f32, max_f32, mean_f32,
    std_f32, var_f32, median_f32, first_f32, last_f32, count_ge_f32
);
fixed_numeric!(
    /// The 64-bit IEEE-754 float type (`f64`).
    Float64, f64, F64, f64,
    pwrite_f64_array, pread_f64_array, pwrite_f64_repeat, sum_f64, min_f64, max_f64, mean_f64,
    std_f64, var_f64, median_f64, first_f64, last_f64, count_ge_f64
);

/// The 16-bit IEEE-754 **half-precision** float type ([`F16`]). There is no typed `f16` array, but
/// [`F16`] is `#[repr(transparent)]` over `u16`, so its 2-byte halves read / write straight through
/// the source's vectorized `u16` array kernels (a `&[F16]` reinterprets to a `&[u16]` of the half
/// bits). Its aggregations **widen to `f64`** — `sum` / `mean` / `std` / `var` reduce over
/// `to_f32() as f64`, `min` / `max` order by value and ignore NaN.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Float16;

/// Elements streamed per stack chunk in a [`Float16`] reduction — bounded so the chunk stays on the
/// stack while the loop is long enough for the compiler to auto-vectorize.
const F16_CHUNK: usize = 1024;

impl DataType for Float16 {
    type Native = F16;
    const DATA_TYPE_ID: crate::datatype_id::DataTypeId = crate::datatype_id::DataTypeId::Float16;
}

impl Encoder for Float16 {
    fn encode<W: IOBase>(dst: &mut W, index: u64, value: F16) -> Result<(), IoError> {
        Self::encode_slice(dst, index, &[value])
    }
    fn encode_slice<W: IOBase>(dst: &mut W, start: u64, values: &[F16]) -> Result<(), IoError> {
        // SAFETY: `F16` is `#[repr(transparent)]` over `u16`, so `&[F16]` has the identical layout
        // of a `&[u16]` of the half bit patterns — a zero-copy reinterpret. The `u16` array kernel
        // then serializes each half little-endian (correct on every endianness — the half bits are a
        // logical `u16`, not raw memory bytes).
        let raw =
            unsafe { core::slice::from_raw_parts(values.as_ptr().cast::<u16>(), values.len()) };
        dst.pwrite_u16_array(start * 2, raw)
    }
    fn encode_repeat<W: IOBase>(
        dst: &mut W,
        start: u64,
        value: F16,
        count: usize,
    ) -> Result<(), IoError> {
        // Forward straight to the source's allocation-free repeated-value fill over the raw bits.
        dst.pwrite_u16_repeat(start * 2, value.to_bits(), count)
    }
}

impl Decoder for Float16 {
    fn decode<R: IOBase>(src: &R, index: u64) -> Result<F16, IoError> {
        let mut out = [F16::ZERO; 1];
        Self::decode_slice(src, index, &mut out)?;
        Ok(out[0])
    }
    fn decode_slice<R: IOBase>(src: &R, start: u64, out: &mut [F16]) -> Result<(), IoError> {
        // SAFETY: as in `encode_slice` — `&mut [F16]` reinterprets to a `&mut [u16]` of half bits.
        let raw =
            unsafe { core::slice::from_raw_parts_mut(out.as_mut_ptr().cast::<u16>(), out.len()) };
        src.pread_u16_array(start * 2, raw)
    }
}

impl Reduce for Float16 {
    /// The half sums **widen to `f64`** — a half has far too little range to accumulate in-place.
    type Sum = f64;

    fn sum<R: IOBase>(src: &R, start: u64, count: usize) -> Result<f64, IoError> {
        let mut chunk = [F16::ZERO; F16_CHUNK];
        let mut acc = 0f64;
        let mut done = 0usize;
        while done < count {
            let take = (count - done).min(F16_CHUNK);
            Self::decode_slice(src, start + done as u64, &mut chunk[..take])?;
            for value in &chunk[..take] {
                acc += value.to_f32() as f64;
            }
            done += take;
        }
        Ok(acc)
    }

    fn min<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<F16>, IoError> {
        reduce_extreme(src, start, count, true)
    }

    fn max<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<F16>, IoError> {
        reduce_extreme(src, start, count, false)
    }

    fn mean<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<f64>, IoError> {
        if count == 0 {
            return Ok(None);
        }
        Ok(Some(Self::sum(src, start, count)? / count as f64))
    }

    fn var<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<f64>, IoError> {
        if count == 0 {
            return Ok(None);
        }
        let mut chunk = [F16::ZERO; F16_CHUNK];
        let (mut sum, mut sum_sq) = (0f64, 0f64);
        let mut done = 0usize;
        while done < count {
            let take = (count - done).min(F16_CHUNK);
            Self::decode_slice(src, start + done as u64, &mut chunk[..take])?;
            for value in &chunk[..take] {
                let x = value.to_f32() as f64;
                sum += x;
                sum_sq += x * x;
            }
            done += take;
        }
        let n = count as f64;
        let mean = sum / n;
        Ok(Some((sum_sq / n - mean * mean).max(0.0)))
    }

    fn std<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<f64>, IoError> {
        Ok(Self::var(src, start, count)?.map(f64::sqrt))
    }

    fn median<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<f64>, IoError> {
        if count == 0 {
            return Ok(None);
        }
        let mut values: Vec<f64> = Vec::with_capacity(count);
        let mut chunk = [F16::ZERO; F16_CHUNK];
        let mut done = 0usize;
        while done < count {
            let take = (count - done).min(F16_CHUNK);
            Self::decode_slice(src, start + done as u64, &mut chunk[..take])?;
            values.extend(chunk[..take].iter().map(|v| v.to_f32() as f64));
            done += take;
        }
        values.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
        let mid = count / 2;
        let median = if count.is_multiple_of(2) {
            (values[mid - 1] + values[mid]) / 2.0
        } else {
            values[mid]
        };
        Ok(Some(median))
    }

    fn first<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<F16>, IoError> {
        if count == 0 {
            return Ok(None);
        }
        Ok(Some(Self::decode(src, start)?))
    }

    fn last<R: IOBase>(src: &R, start: u64, count: usize) -> Result<Option<F16>, IoError> {
        if count == 0 {
            return Ok(None);
        }
        Ok(Some(Self::decode(src, start + count as u64 - 1)?))
    }

    fn count_ge<R: IOBase>(
        src: &R,
        start: u64,
        count: usize,
        threshold: F16,
    ) -> Result<usize, IoError> {
        let limit = threshold.to_f32();
        let mut chunk = [F16::ZERO; F16_CHUNK];
        let mut matched = 0usize;
        let mut done = 0usize;
        while done < count {
            let take = (count - done).min(F16_CHUNK);
            Self::decode_slice(src, start + done as u64, &mut chunk[..take])?;
            for value in &chunk[..take] {
                if value.to_f32() >= limit {
                    matched += 1;
                }
            }
            done += take;
        }
        Ok(matched)
    }
}

/// The shared NaN-ignoring `min` (`want_min`) / `max` extreme fold for [`Float16`], comparing by
/// value through `f32` and streaming through one stack chunk (zero heap). NaN elements are skipped,
/// so the result is order-independent — matching the source's float `Aggregate` kernels.
fn reduce_extreme<R: IOBase>(
    src: &R,
    start: u64,
    count: usize,
    want_min: bool,
) -> Result<Option<F16>, IoError> {
    let mut chunk = [F16::ZERO; F16_CHUNK];
    let mut best: Option<(F16, f32)> = None;
    let mut done = 0usize;
    while done < count {
        let take = (count - done).min(F16_CHUNK);
        <Float16 as Decoder>::decode_slice(src, start + done as u64, &mut chunk[..take])?;
        for &value in &chunk[..take] {
            let x = value.to_f32();
            if x.is_nan() {
                continue;
            }
            best = Some(match best {
                Some((cur, cx)) if (want_min && cx <= x) || (!want_min && cx >= x) => (cur, cx),
                _ => (value, x),
            });
        }
        done += take;
    }
    Ok(best.map(|(value, _)| value))
}
