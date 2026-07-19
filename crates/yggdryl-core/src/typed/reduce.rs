//! [`Reduce`] — the **vectorized numeric aggregations** a numeric [`DataType`] exposes.
//!
//! It is the type-level bridge to [`Aggregate`](crate::io::memory::Aggregate): a numeric marker
//! (`Int32`, `Float64`, …) maps its `sum`/`min`/`max`/`mean`/`std`/`var`/`median`/`first`/`last`/
//! `count_ge` to the matching width-specific `Aggregate` method on the underlying source, so a
//! [`Serie`](super::Serie) reduces over its data buffer with the same allocation-free (bar the sort
//! `median` needs), LLVM-vectorized, NaN-safe kernels every `IOBase` runs — no per-column loop of
//! its own. The `bool` [`Bit`](super::fixedbit::Bit) type is **not** `Reduce` (booleans do not sum).

use super::DataType;
use crate::io::memory::IoError;

/// The numeric aggregations for a fixed-width numeric [`DataType`], routed to the source's
/// [`Aggregate`](crate::io::memory::Aggregate) kernels. `start` is an **element** index; `count` is
/// an element count.
pub trait Reduce: DataType {
    /// The accumulator type of [`sum`](Reduce::sum) — wide enough not to overflow (`i8`…`u32` → `i64`,
    /// `i64`/`u64`/`i128` → `i128`, `u128` → `u128`, floats → `f64`).
    type Sum: Copy;

    /// The **sum** of `count` elements starting at element `start`.
    fn sum<R: crate::io::memory::IOBase>(
        src: &R,
        start: u64,
        count: usize,
    ) -> Result<Self::Sum, IoError>;

    /// The **minimum** (a float min ignores NaN); `None` when `count == 0`.
    fn min<R: crate::io::memory::IOBase>(
        src: &R,
        start: u64,
        count: usize,
    ) -> Result<Option<Self::Native>, IoError>;

    /// The **maximum** (a float max ignores NaN); `None` when `count == 0`.
    fn max<R: crate::io::memory::IOBase>(
        src: &R,
        start: u64,
        count: usize,
    ) -> Result<Option<Self::Native>, IoError>;

    /// The **mean** as `f64`; `None` when `count == 0`.
    fn mean<R: crate::io::memory::IOBase>(
        src: &R,
        start: u64,
        count: usize,
    ) -> Result<Option<f64>, IoError>;

    /// The **population standard deviation** as `f64` (the `sqrt` of the variance); `None` when
    /// `count == 0`.
    fn std<R: crate::io::memory::IOBase>(
        src: &R,
        start: u64,
        count: usize,
    ) -> Result<Option<f64>, IoError>;

    /// The **population variance** as `f64` (`std²`); `None` when `count == 0`.
    fn var<R: crate::io::memory::IOBase>(
        src: &R,
        start: u64,
        count: usize,
    ) -> Result<Option<f64>, IoError>;

    /// The **median** as `f64`; `None` when `count == 0`. Materializes + sorts the `count` values
    /// (an order statistic — the one allocation is inherent).
    fn median<R: crate::io::memory::IOBase>(
        src: &R,
        start: u64,
        count: usize,
    ) -> Result<Option<f64>, IoError>;

    /// The **first** element at `start`; `None` when `count == 0`.
    fn first<R: crate::io::memory::IOBase>(
        src: &R,
        start: u64,
        count: usize,
    ) -> Result<Option<Self::Native>, IoError>;

    /// The **last** of the `count` elements; `None` when `count == 0`.
    fn last<R: crate::io::memory::IOBase>(
        src: &R,
        start: u64,
        count: usize,
    ) -> Result<Option<Self::Native>, IoError>;

    /// How many of the `count` elements are `>= threshold`.
    fn count_ge<R: crate::io::memory::IOBase>(
        src: &R,
        start: u64,
        count: usize,
        threshold: Self::Native,
    ) -> Result<usize, IoError>;
}
