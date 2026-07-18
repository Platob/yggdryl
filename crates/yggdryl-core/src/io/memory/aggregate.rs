//! [`Aggregate`] — **vectorized statistical aggregations over any [`IOBase`] source**.
//!
//! A blanket trait (`impl<T: IOBase> Aggregate for T`), so every source — a `Heap`, a mapped file,
//! a device buffer — gets the same reductions: `sum` / `mean` / `min` / `max` / `std` / `first` /
//! `last` and the threshold filter `count_ge`, for **every native numeric width** (`i8`…`u128`,
//! `f32`, `f64`). Each streams the typed data through a fixed **stack** chunk via the type's fast
//! contiguous bulk read (zero heap allocation in the reduction loop) and runs the dense,
//! LLVM-vectorized loop. Float `min`/`max` reduce with the type's own `min`/`max`, so they **ignore
//! NaN** order-independently. A GPU-backed source overrides these with device kernels.

use super::{IOBase, IoError};

/// Elements staged per stack chunk for the reductions — bounded so the widest chunk stays on the
/// stack while the loop is long enough for the compiler to auto-vectorize.
const AGG_CHUNK: usize = 1024;

/// Emits the full aggregation set (`sum` / `min` / `max` / `mean` / `std` / `first` / `last` /
/// `count_ge`) for one numeric type. Reductions stream through a stack chunk; single-element
/// `first`/`last` reuse the same bulk reader over a one-element slice.
macro_rules! agg_methods {
    ($t:ty, $read:ident, $acc:ty, $minf:path, $maxf:path,
     $sum:ident, $min:ident, $max:ident, $mean:ident, $std:ident,
     $first:ident, $last:ident, $count_ge:ident) => {
        #[doc = concat!("**Sum** of `count` `", stringify!($t), "`s at `offset` (as `",
            stringify!($acc), "`).")]
        fn $sum(&self, offset: u64, count: usize) -> Result<$acc, IoError> {
            let width = core::mem::size_of::<$t>() as u64;
            let mut chunk = [0 as $t; AGG_CHUNK];
            let mut acc: $acc = 0 as $acc;
            let mut done = 0usize;
            while done < count {
                let take = (count - done).min(AGG_CHUNK);
                self.$read(offset + done as u64 * width, &mut chunk[..take])?;
                for &value in &chunk[..take] {
                    acc += value as $acc;
                }
                done += take;
            }
            Ok(acc)
        }

        #[doc = concat!("**Minimum** of `count` `", stringify!($t),
            "`s (a float `min` ignores NaN); `None` when `count == 0`.")]
        fn $min(&self, offset: u64, count: usize) -> Result<Option<$t>, IoError> {
            let width = core::mem::size_of::<$t>() as u64;
            let mut chunk = [0 as $t; AGG_CHUNK];
            let mut best: Option<$t> = None;
            let mut done = 0usize;
            while done < count {
                let take = (count - done).min(AGG_CHUNK);
                self.$read(offset + done as u64 * width, &mut chunk[..take])?;
                for &value in &chunk[..take] {
                    best = Some(best.map_or(value, |current| $minf(current, value)));
                }
                done += take;
            }
            Ok(best)
        }

        #[doc = concat!("**Maximum** of `count` `", stringify!($t),
            "`s (a float `max` ignores NaN); `None` when `count == 0`.")]
        fn $max(&self, offset: u64, count: usize) -> Result<Option<$t>, IoError> {
            let width = core::mem::size_of::<$t>() as u64;
            let mut chunk = [0 as $t; AGG_CHUNK];
            let mut best: Option<$t> = None;
            let mut done = 0usize;
            while done < count {
                let take = (count - done).min(AGG_CHUNK);
                self.$read(offset + done as u64 * width, &mut chunk[..take])?;
                for &value in &chunk[..take] {
                    best = Some(best.map_or(value, |current| $maxf(current, value)));
                }
                done += take;
            }
            Ok(best)
        }

        #[doc = concat!("**Mean** of `count` `", stringify!($t), "`s as `f64`; `None` when empty.")]
        fn $mean(&self, offset: u64, count: usize) -> Result<Option<f64>, IoError> {
            if count == 0 {
                return Ok(None);
            }
            Ok(Some(self.$sum(offset, count)? as f64 / count as f64))
        }

        #[doc = concat!("**Population standard deviation** of `count` `", stringify!($t),
            "`s as `f64`; `None` when empty. One streamed pass (sum + sum-of-squares).")]
        fn $std(&self, offset: u64, count: usize) -> Result<Option<f64>, IoError> {
            if count == 0 {
                return Ok(None);
            }
            let width = core::mem::size_of::<$t>() as u64;
            let mut chunk = [0 as $t; AGG_CHUNK];
            let (mut sum, mut sum_sq) = (0f64, 0f64);
            let mut done = 0usize;
            while done < count {
                let take = (count - done).min(AGG_CHUNK);
                self.$read(offset + done as u64 * width, &mut chunk[..take])?;
                for &value in &chunk[..take] {
                    let x = value as f64;
                    sum += x;
                    sum_sq += x * x;
                }
                done += take;
            }
            let n = count as f64;
            let mean = sum / n;
            Ok(Some((sum_sq / n - mean * mean).max(0.0).sqrt()))
        }

        #[doc = concat!("The **first** `", stringify!($t), "` at `offset`; `None` when `count == 0`.")]
        fn $first(&self, offset: u64, count: usize) -> Result<Option<$t>, IoError> {
            if count == 0 {
                return Ok(None);
            }
            let mut one = [0 as $t; 1];
            self.$read(offset, &mut one)?;
            Ok(Some(one[0]))
        }

        #[doc = concat!("The **last** `", stringify!($t),
            "` of the `count` at `offset`; `None` when `count == 0`.")]
        fn $last(&self, offset: u64, count: usize) -> Result<Option<$t>, IoError> {
            if count == 0 {
                return Ok(None);
            }
            let width = core::mem::size_of::<$t>() as u64;
            let mut one = [0 as $t; 1];
            self.$read(offset + (count as u64 - 1) * width, &mut one)?;
            Ok(Some(one[0]))
        }

        #[doc = concat!("**Filter count** — how many of `count` `", stringify!($t),
            "`s at `offset` are `>= threshold`.")]
        fn $count_ge(&self, offset: u64, count: usize, threshold: $t) -> Result<usize, IoError> {
            let width = core::mem::size_of::<$t>() as u64;
            let mut chunk = [0 as $t; AGG_CHUNK];
            let mut matched = 0usize;
            let mut done = 0usize;
            while done < count {
                let take = (count - done).min(AGG_CHUNK);
                self.$read(offset + done as u64 * width, &mut chunk[..take])?;
                for &value in &chunk[..take] {
                    if value >= threshold {
                        matched += 1;
                    }
                }
                done += take;
            }
            Ok(matched)
        }
    };
}

/// **Vectorized statistical aggregations over any source.** A blanket trait over every
/// [`IOBase`]: `sum` / `min` / `max` / `mean` / `std` / `first` / `last` and the `count_ge` filter,
/// for each native numeric width. `count` is the **element** count (not bytes); pair it with
/// [`element_count`](IOBase::element_count) to reduce a whole typed source.
///
/// ```
/// use yggdryl_core::io::memory::{Aggregate, Heap, IOBase};
///
/// let mut h = Heap::new();
/// h.pwrite_i64_array(0, &[4, 8, 15, 16, 23, 42]).unwrap();
/// assert_eq!(h.sum_i64(0, 6).unwrap(), 108);
/// assert_eq!(h.min_i64(0, 6).unwrap(), Some(4));
/// assert_eq!(h.max_i64(0, 6).unwrap(), Some(42));
/// assert_eq!(h.mean_i64(0, 6).unwrap(), Some(18.0));
/// assert_eq!(h.first_i64(0, 6).unwrap(), Some(4));
/// assert_eq!(h.last_i64(0, 6).unwrap(), Some(42));
/// assert_eq!(h.count_ge_i64(0, 6, 16).unwrap(), 3);
/// ```
pub trait Aggregate: IOBase {
    agg_methods!(
        i8,
        pread_i8_array,
        i64,
        core::cmp::min,
        core::cmp::max,
        sum_i8,
        min_i8,
        max_i8,
        mean_i8,
        std_i8,
        first_i8,
        last_i8,
        count_ge_i8
    );
    agg_methods!(
        u8,
        pread_exact,
        i64,
        core::cmp::min,
        core::cmp::max,
        sum_u8,
        min_u8,
        max_u8,
        mean_u8,
        std_u8,
        first_u8,
        last_u8,
        count_ge_u8
    );
    agg_methods!(
        i16,
        pread_i16_array,
        i64,
        core::cmp::min,
        core::cmp::max,
        sum_i16,
        min_i16,
        max_i16,
        mean_i16,
        std_i16,
        first_i16,
        last_i16,
        count_ge_i16
    );
    agg_methods!(
        u16,
        pread_u16_array,
        i64,
        core::cmp::min,
        core::cmp::max,
        sum_u16,
        min_u16,
        max_u16,
        mean_u16,
        std_u16,
        first_u16,
        last_u16,
        count_ge_u16
    );
    agg_methods!(
        i32,
        pread_i32_array,
        i64,
        core::cmp::min,
        core::cmp::max,
        sum_i32,
        min_i32,
        max_i32,
        mean_i32,
        std_i32,
        first_i32,
        last_i32,
        count_ge_i32
    );
    agg_methods!(
        u32,
        pread_u32_array,
        i64,
        core::cmp::min,
        core::cmp::max,
        sum_u32,
        min_u32,
        max_u32,
        mean_u32,
        std_u32,
        first_u32,
        last_u32,
        count_ge_u32
    );
    agg_methods!(
        i64,
        pread_i64_array,
        i128,
        core::cmp::min,
        core::cmp::max,
        sum_i64,
        min_i64,
        max_i64,
        mean_i64,
        std_i64,
        first_i64,
        last_i64,
        count_ge_i64
    );
    agg_methods!(
        u64,
        pread_u64_array,
        i128,
        core::cmp::min,
        core::cmp::max,
        sum_u64,
        min_u64,
        max_u64,
        mean_u64,
        std_u64,
        first_u64,
        last_u64,
        count_ge_u64
    );
    agg_methods!(
        i128,
        pread_i128_array,
        i128,
        core::cmp::min,
        core::cmp::max,
        sum_i128,
        min_i128,
        max_i128,
        mean_i128,
        std_i128,
        first_i128,
        last_i128,
        count_ge_i128
    );
    agg_methods!(
        u128,
        pread_u128_array,
        u128,
        core::cmp::min,
        core::cmp::max,
        sum_u128,
        min_u128,
        max_u128,
        mean_u128,
        std_u128,
        first_u128,
        last_u128,
        count_ge_u128
    );
    agg_methods!(
        f32,
        pread_f32_array,
        f64,
        f32::min,
        f32::max,
        sum_f32,
        min_f32,
        max_f32,
        mean_f32,
        std_f32,
        first_f32,
        last_f32,
        count_ge_f32
    );
    agg_methods!(
        f64,
        pread_f64_array,
        f64,
        f64::min,
        f64::max,
        sum_f64,
        min_f64,
        max_f64,
        mean_f64,
        std_f64,
        first_f64,
        last_f64,
        count_ge_f64
    );
}

impl<T: IOBase> Aggregate for T {}
