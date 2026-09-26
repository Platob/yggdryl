//! Allocation requests as a Criterion measurement.
//!
//! Criterion measures whatever `start` and `end` bracket. Bracketed by the
//! counting allocator the test binaries share, a case reports what one
//! iteration allocates rather than how long it takes - the requests as
//! `rust/tests/allocations.rs` counts them, or the bytes as the ULBridge
//! profile prints them. A count is deterministic, so a case's interval is
//! one number, and the change column reads exactly: `p = 0.00` where a
//! count moved and `NaN` where it did not. Criterion labels every value
//! `time` and every rate `thrpt`; here they are requests and requests per
//! message. Setup runs before `start`, so a clone `iter_batched` makes for
//! a routine is never counted against it.

use criterion::Throughput;
use criterion::measurement::{Measurement, ValueFormatter};

use crate::allocations::{self, Armed};

/// Allocation requests per iteration: every allocation and every
/// reallocation, the count the pins in `rust/tests/allocations.rs` hold.
pub(crate) struct Allocations;

/// Bytes requested per iteration: each allocation's size and each
/// reallocation's new size, never a live or peak figure.
pub(crate) struct AllocatedBytes;

impl Measurement for Allocations {
    type Intermediate = Armed;
    type Value = u64;

    fn start(&self) -> Armed {
        allocations::arm()
    }

    fn end(&self, armed: Armed) -> u64 {
        let counts = armed.finish();
        counts.allocations + counts.reallocations
    }

    fn add(&self, left: &u64, right: &u64) -> u64 {
        left + right
    }

    fn zero(&self) -> u64 {
        0
    }

    fn to_f64(&self, value: &u64) -> f64 {
        *value as f64
    }

    fn formatter(&self) -> &dyn ValueFormatter {
        &Requests
    }
}

impl Measurement for AllocatedBytes {
    type Intermediate = Armed;
    type Value = u64;

    fn start(&self) -> Armed {
        allocations::arm()
    }

    fn end(&self, armed: Armed) -> u64 {
        armed.finish().requested_bytes
    }

    fn add(&self, left: &u64, right: &u64) -> u64 {
        left + right
    }

    fn zero(&self) -> u64 {
        0
    }

    fn to_f64(&self, value: &u64) -> f64 {
        *value as f64
    }

    fn formatter(&self) -> &dyn ValueFormatter {
        &Bytes
    }
}

/// Requests, unscaled: a count is read as it is.
struct Requests;

impl ValueFormatter for Requests {
    fn scale_values(&self, _typical: f64, _values: &mut [f64]) -> &'static str {
        "allocs"
    }

    fn scale_throughputs(
        &self,
        _typical: f64,
        throughput: &Throughput,
        values: &mut [f64],
    ) -> &'static str {
        let (per, unit) = per_unit(throughput, "allocs/msg", "allocs/unit");
        for value in values {
            *value /= per;
        }
        unit
    }

    fn scale_for_machines(&self, _values: &mut [f64]) -> &'static str {
        "allocs"
    }
}

/// Bytes, scaled to the unit the typical value reads best in.
struct Bytes;

impl ValueFormatter for Bytes {
    fn scale_values(&self, typical: f64, values: &mut [f64]) -> &'static str {
        let (divisor, unit) = byte_unit(typical);
        for value in values {
            *value /= divisor;
        }
        unit
    }

    fn scale_throughputs(
        &self,
        typical: f64,
        throughput: &Throughput,
        values: &mut [f64],
    ) -> &'static str {
        let (per, _) = per_unit(throughput, "", "");
        let (divisor, unit) = byte_unit(typical / per);
        for value in values {
            *value /= per * divisor;
        }
        match throughput {
            Throughput::Elements(_) => match unit {
                "B" => "B/msg",
                "KiB" => "KiB/msg",
                _ => "MiB/msg",
            },
            Throughput::Bytes(_) | Throughput::BytesDecimal(_) | Throughput::Bits(_) => {
                match unit {
                    "B" => "B/unit",
                    "KiB" => "KiB/unit",
                    _ => "MiB/unit",
                }
            }
        }
    }

    fn scale_for_machines(&self, _values: &mut [f64]) -> &'static str {
        "B"
    }
}

/// What one iteration processed, and the unit a value divided by it reads
/// in: the messages of a stage, or the bytes or bits of a throughput stage.
fn per_unit(
    throughput: &Throughput,
    per_element: &'static str,
    per_unit: &'static str,
) -> (f64, &'static str) {
    match throughput {
        Throughput::Elements(elements) => (*elements as f64, per_element),
        Throughput::Bytes(units) | Throughput::BytesDecimal(units) | Throughput::Bits(units) => {
            (*units as f64, per_unit)
        }
    }
}

/// The binary unit `typical` bytes read best in, and its divisor.
fn byte_unit(typical: f64) -> (f64, &'static str) {
    if typical >= 1024.0 * 1024.0 {
        (1024.0 * 1024.0, "MiB")
    } else if typical >= 1024.0 {
        (1024.0, "KiB")
    } else {
        (1.0, "B")
    }
}
