//! Lightweight timing benchmark for `Version` parsing/rendering.
//!
//! Run with `cargo bench -p yggdryl-version`. Uses a plain `main` (the crate
//! sets `harness = false`) so there is no benchmark-framework dependency.

use std::hint::black_box;
use std::time::Instant;

use yggdryl_version::{FromInput, ToOutput, Version};

/// Times `f` over `iters` iterations (after a short warm-up) and prints ns/iter.
fn bench(name: &str, iters: u64, mut f: impl FnMut()) {
    for _ in 0..iters / 10 {
        f();
    }
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    let per = start.elapsed().as_nanos() as f64 / iters as f64;
    println!("{name:<30} {per:>8.1} ns/iter");
}

fn main() {
    let n = 5_000_000;
    bench("Version::from_str (full)", n, || {
        black_box(Version::from_str(black_box("1.4.2")).unwrap());
    });
    bench("Version::from_str (partial)", n, || {
        black_box(Version::from_str(black_box("2")).unwrap());
    });
    let v = Version::new(1, 4, 2);
    bench("Version::to_string", n, || {
        black_box(black_box(&v).to_str(false));
    });
}
