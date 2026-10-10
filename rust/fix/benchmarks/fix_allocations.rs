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

#[path = "../../benchmarks/bench_profile.rs"]
mod bench_profile;

#[path = "../tests/support/allocations.rs"]
#[allow(dead_code)]
mod allocations;

#[path = "../../benchmarks/allocation_measurement.rs"]
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
use criterion::{Criterion, criterion_group};

use allocation_measurement::{AllocatedBytes, Allocations};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

fn requests(criterion: &mut Criterion<Allocations>) {
    ulbridge::stages(criterion, "fix/allocations", 1, Some(1));
    #[cfg(feature = "internals")]
    forex(criterion, "fix/allocations/forex");
}

fn bytes(criterion: &mut Criterion<AllocatedBytes>) {
    ulbridge::stages(criterion, "fix/allocated_bytes", 1, Some(1));
    #[cfg(feature = "internals")]
    forex(criterion, "fix/allocated_bytes/forex");
}

/// FX detection over a parsed message whose symbol the registry's memo
/// already holds: a symbol naming no pair, and a pair detected again. Both
/// allocate nothing - a hit clones inline texts, and a detected message's
/// cells already hold what detection would write. Reached through the
/// crate's internals, so the row runs where that feature is on.
#[cfg(feature = "internals")]
fn forex<M: Measurement>(criterion: &mut Criterion<M>, group: &str) {
    use std::hint::black_box;
    use std::sync::Arc;

    use yggdryl_fix::FixCodec;
    use yggdryl_fix::internals::forex::{derive, memo};

    let codec = FixCodec::new(Arc::new(seed())).with_threads(1);
    let held = memo();
    let mut group = criterion.benchmark_group(group);
    for (name, line) in [
        (
            "non_fx",
            &b"8=FIX.4.4|35=D|11=A|55=AAPL|54=1|38=100|10=0|"[..],
        ),
        (
            "fx_hit",
            b"8=FIX.4.4|35=D|11=A|55=EUR/USD|54=1|38=100|10=0|",
        ),
    ] {
        let mut message = codec
            .parse_line(line)
            .expect("a line")
            .next()
            .expect("one message")
            .expect("an order");
        derive(&mut message, &held).expect("detects");
        group.bench_function(name, |bencher| {
            bencher.iter(|| derive(black_box(&mut message), &held).expect("detects"));
        });
    }
    group.finish();
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

fn main() {
    // The kinds and names the benchmarks read are claimed before any runs.
    yggdryl_fix::install().expect("yggdryl-fix installs");
    fix_allocations();
    fix_allocated_bytes();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
