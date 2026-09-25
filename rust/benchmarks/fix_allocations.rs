//! The bridge's capture through the FIX pipeline, counted rather than
//! timed: what each stage of `fix/ulbridge` allocates per message, and
//! how many bytes it asks for.
//!
//! Its own target rather than two more groups of the `fix` binary, because
//! a counting allocator reads a thread-local on every allocation whether
//! or not a section is armed, and the timings `fix` reports are measured
//! under the allocator a caller runs with. One copy of the capture on one
//! thread, since the counter observes the thread it is armed on; a count is
//! exact, so ten samples of one second say all there is to say.

#[path = "bench_profile.rs"]
mod bench_profile;

#[path = "../tests/support/allocations.rs"]
#[allow(dead_code)]
mod allocations;

#[path = "allocation_measurement.rs"]
mod allocation_measurement;

#[path = "fix/common.rs"]
#[allow(dead_code)]
mod common;

#[path = "fix/ulbridge.rs"]
#[allow(dead_code)]
mod ulbridge;

pub(crate) use common::seed;

use std::time::Duration;

use criterion::measurement::Measurement;
use criterion::{Criterion, criterion_group, criterion_main};

use allocation_measurement::{AllocatedBytes, Allocations};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

fn requests(criterion: &mut Criterion<Allocations>) {
    ulbridge::stages(criterion, "fix/allocations", 1, Some(1));
}

fn bytes(criterion: &mut Criterion<AllocatedBytes>) {
    ulbridge::stages(criterion, "fix/allocated_bytes", 1, Some(1));
}

/// A count needs no warm-up and no long sample: the smallest configuration
/// Criterion accepts.
fn counting<M: Measurement>(measurement: M) -> Criterion<M> {
    Criterion::default()
        .with_measurement(measurement)
        .sample_size(10)
        .warm_up_time(Duration::from_millis(100))
        .measurement_time(Duration::from_secs(1))
}

criterion_group! {
    name = fix_allocations;
    config = counting(Allocations);
    targets = requests
}

criterion_group! {
    name = fix_allocated_bytes;
    config = counting(AllocatedBytes);
    targets = bytes
}

criterion_main!(fix_allocations, fix_allocated_bytes);
