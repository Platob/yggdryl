//! The **floating-point** element types — `f32` and `f64` — each a byte-granular fixed-width type,
//! one [`fixed_numeric!`](super::fixed_numeric) line apiece. Their `min`/`max` reductions **ignore
//! NaN** (the source's float `Aggregate` kernels do), order-independently.

fixed_numeric!(
    /// The 32-bit IEEE-754 float type (`f32`).
    Float32, f32, F32, f64,
    pwrite_f32_array, pread_f32_array, sum_f32, min_f32, max_f32, mean_f32
);
fixed_numeric!(
    /// The 64-bit IEEE-754 float type (`f64`).
    Float64, f64, F64, f64,
    pwrite_f64_array, pread_f64_array, sum_f64, min_f64, max_f64, mean_f64
);
