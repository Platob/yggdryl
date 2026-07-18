//! [`Compute`] — data **compute & transfer** ops (math aggregations, filters, copies) that
//! **auto-select GPU vs CPU**.
//!
//! Every [`GpuMemory`] buffer gets these operations for free (a blanket impl), and each one first
//! asks [`compute_backend`](Compute::compute_backend) which backend to run on: the **GPU** when
//! the buffer lives on a real device *and* the workload is big enough to amortize the host↔device
//! transfer, otherwise the **CPU** (the dense, LLVM-vectorized reduction). The dispatch is the
//! optimization seam — today both arms run the CPU kernel (correct and fast everywhere), and the
//! marked **GPU arm** is where a device reduction / filter / DMA kernel drops in behind a hardware
//! backend, so callers written against `Compute` accelerate transparently when the kernels land.
//!
//! ```
//! # #[cfg(feature = "gpu-amd")] {
//! use yggdryl_core::io::gpu::{AmdBuffer, Compute, GpuMemory};
//! use yggdryl_core::io::memory::IOBase;
//!
//! let mut buf = AmdBuffer::new();
//! buf.pwrite_i32_array(0, &[4, 8, 15, 16, 23, 42]).unwrap();
//! assert_eq!(buf.sum_i32(0, 6).unwrap(), 108);
//! assert_eq!(buf.max_i32(0, 6).unwrap(), Some(42));
//! assert_eq!(buf.count_ge_i32(0, 6, 16).unwrap(), 3); // a filter: how many >= 16
//! # }
//! ```

use super::{GpuBackend, GpuMemory};
use crate::io::memory::IoError;

/// Elements at or above this count make a GPU run worth the host↔device transfer — the
/// conservative default threshold [`Compute::compute_backend`] uses. Tunable as real kernels land.
pub const GPU_ELEMENT_THRESHOLD: usize = 1 << 16; // 65 536

/// The element count a compute op stages per stack chunk — reads the typed data through the fast
/// contiguous path in bounded pieces, zero heap allocation in the reduction loop.
const COMPUTE_CHUNK: usize = 1024;

/// The backend a compute op runs on — chosen per call by [`Compute::compute_backend`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComputeBackend {
    /// The dense, LLVM-vectorized CPU reduction (streamed through a stack chunk).
    Cpu,
    /// The device kernel (uploaded + run on the GPU) — the accelerated path.
    Gpu,
}

impl ComputeBackend {
    /// Whether this is the GPU (device-kernel) backend.
    pub fn is_gpu(&self) -> bool {
        matches!(self, ComputeBackend::Gpu)
    }
}

/// Emits the five auto-dispatched compute ops (`sum` / `min` / `max` / `mean` / `count_ge`) for one
/// numeric type. Each streams the typed data through a fixed stack chunk via the type's fast bulk
/// read, runs the dense CPU reduction, and consults [`Compute::compute_backend`] for the GPU seam.
macro_rules! compute_ops {
    ($t:ty, $read:ident, $acc:ty, $sum:ident, $min:ident, $max:ident, $mean:ident, $count_ge:ident) => {
        #[doc = concat!("**Sum** of `count` `", stringify!($t), "`s at `offset` (as `",
            stringify!($acc), "`) — auto-dispatched (GPU when large + on a device, else the \
            vectorized CPU reduction).")]
        fn $sum(&self, offset: u64, count: usize) -> Result<$acc, IoError> {
            let _ = self.compute_backend(count); // GPU seam: a device sum-reduction runs here
            let width = core::mem::size_of::<$t>() as u64;
            let mut chunk = [0 as $t; COMPUTE_CHUNK];
            let mut acc: $acc = 0 as $acc;
            let mut done = 0usize;
            while done < count {
                let take = (count - done).min(COMPUTE_CHUNK);
                self.$read(offset + done as u64 * width, &mut chunk[..take])?;
                for &value in &chunk[..take] {
                    acc += value as $acc;
                }
                done += take;
            }
            Ok(acc)
        }

        #[doc = concat!("**Minimum** of `count` `", stringify!($t),
            "`s at `offset`, or `None` when `count == 0`.")]
        fn $min(&self, offset: u64, count: usize) -> Result<Option<$t>, IoError> {
            let _ = self.compute_backend(count); // GPU seam: a device min-reduction runs here
            let width = core::mem::size_of::<$t>() as u64;
            let mut chunk = [0 as $t; COMPUTE_CHUNK];
            let mut best: Option<$t> = None;
            let mut done = 0usize;
            while done < count {
                let take = (count - done).min(COMPUTE_CHUNK);
                self.$read(offset + done as u64 * width, &mut chunk[..take])?;
                for &value in &chunk[..take] {
                    best = Some(match best {
                        Some(current) if current <= value => current,
                        _ => value,
                    });
                }
                done += take;
            }
            Ok(best)
        }

        #[doc = concat!("**Maximum** of `count` `", stringify!($t),
            "`s at `offset`, or `None` when `count == 0`.")]
        fn $max(&self, offset: u64, count: usize) -> Result<Option<$t>, IoError> {
            let _ = self.compute_backend(count); // GPU seam: a device max-reduction runs here
            let width = core::mem::size_of::<$t>() as u64;
            let mut chunk = [0 as $t; COMPUTE_CHUNK];
            let mut best: Option<$t> = None;
            let mut done = 0usize;
            while done < count {
                let take = (count - done).min(COMPUTE_CHUNK);
                self.$read(offset + done as u64 * width, &mut chunk[..take])?;
                for &value in &chunk[..take] {
                    best = Some(match best {
                        Some(current) if current >= value => current,
                        _ => value,
                    });
                }
                done += take;
            }
            Ok(best)
        }

        #[doc = concat!("**Mean** of `count` `", stringify!($t),
            "`s at `offset` as `f64`, or `None` when `count == 0`.")]
        fn $mean(&self, offset: u64, count: usize) -> Result<Option<f64>, IoError> {
            if count == 0 {
                return Ok(None);
            }
            let sum = self.$sum(offset, count)?;
            Ok(Some(sum as f64 / count as f64))
        }

        #[doc = concat!("**Filter count** — how many of `count` `", stringify!($t),
            "`s at `offset` are `>= threshold`. The reduction form of a threshold filter.")]
        fn $count_ge(&self, offset: u64, count: usize, threshold: $t) -> Result<usize, IoError> {
            let _ = self.compute_backend(count); // GPU seam: a device predicate-count runs here
            let width = core::mem::size_of::<$t>() as u64;
            let mut chunk = [0 as $t; COMPUTE_CHUNK];
            let mut matched = 0usize;
            let mut done = 0usize;
            while done < count {
                let take = (count - done).min(COMPUTE_CHUNK);
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

/// **Compute & transfer operations over device memory, auto-dispatched to GPU or CPU.** A blanket
/// trait over every [`GpuMemory`] buffer ([`CpuHeap`](super::CpuHeap) / [`AmdBuffer`](super::amd::AmdBuffer)):
/// math aggregations (`sum` / `min` / `max` / `mean`), a threshold **filter** (`count_ge`), and a
/// device-aware **copy** — each picking its backend via [`compute_backend`](Compute::compute_backend).
pub trait Compute: GpuMemory {
    /// The backend the next op over `elements` values would run on: **GPU** when this buffer is on
    /// a real device *and* `elements >= `[`GPU_ELEMENT_THRESHOLD`], else **CPU**. The dispatch the
    /// aggregations / filters consult; exposed so a caller can see (or override) the decision.
    fn compute_backend(&self, elements: usize) -> ComputeBackend {
        if self.device().backend() != GpuBackend::Cpu && elements >= GPU_ELEMENT_THRESHOLD {
            ComputeBackend::Gpu
        } else {
            ComputeBackend::Cpu
        }
    }

    compute_ops!(
        i32,
        pread_i32_array,
        i64,
        sum_i32,
        min_i32,
        max_i32,
        mean_i32,
        count_ge_i32
    );
    compute_ops!(
        i64,
        pread_i64_array,
        i128,
        sum_i64,
        min_i64,
        max_i64,
        mean_i64,
        count_ge_i64
    );
    compute_ops!(
        f32,
        pread_f32_array,
        f64,
        sum_f32,
        min_f32,
        max_f32,
        mean_f32,
        count_ge_f32
    );
    compute_ops!(
        f64,
        pread_f64_array,
        f64,
        sum_f64,
        min_f64,
        max_f64,
        mean_f64,
        count_ge_f64
    );

    /// **Device-aware copy** — copies this buffer's whole content into `dst`, auto-selecting the
    /// path: a same-device GPU→GPU copy would run as a device-to-device DMA (the marked seam),
    /// otherwise the zero-copy host copy ([`copy_from`](crate::io::memory::IOBase::copy_from)).
    /// Returns the byte count. The transfer counterpart of the aggregations.
    fn compute_copy_into<D: GpuMemory>(&self, dst: &mut D) -> Result<u64, IoError> {
        let same_gpu = self.device().backend() != GpuBackend::Cpu && self.device() == dst.device();
        if same_gpu {
            // GPU seam: a device-to-device DMA runs here once the hardware queue is wired; until
            // then the host copy below is used (correct on every platform).
        }
        dst.copy_from(self)
    }
}

impl<T: GpuMemory> Compute for T {}
