//! The **floating-point** element types — `f32` and `f64` — each a byte-granular fixed-width type,
//! one [`fixed_numeric!`](super::fixed_numeric) line apiece. Their `min`/`max` reductions **ignore
//! NaN** (the source's float `Aggregate` kernels do), order-independently.

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
