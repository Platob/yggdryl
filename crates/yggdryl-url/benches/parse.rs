//! Lightweight timing benchmarks for the hot parsing/rendering paths.
//!
//! Run with `cargo bench -p yggdryl-url`. Uses a plain `main` (the crate sets
//! `harness = false`) so there is no benchmark-framework dependency; it reports
//! nanoseconds per iteration using a fixed iteration count.

use std::hint::black_box;
use std::time::Instant;

use yggdryl_url::{FromInput, MediaType, MimeType, Uri, Url};

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
    println!("{name:<34} {per:>8.1} ns/iter");
}

fn main() {
    let n = 2_000_000;

    println!("== parsing ==");
    bench("Uri::from_str (https)", n, || {
        black_box(Uri::from_str(black_box("https://example.com/docs?page=2#intro")).unwrap());
    });
    bench("Url::from_str (full)", n, || {
        black_box(
            Url::from_str(black_box(
                "https://user:pw@example.com:8443/api?a=1&a=2#top",
            ))
            .unwrap(),
        );
    });
    bench("Uri::from_str (windows path)", n, || {
        black_box(Uri::from_str(black_box("C:\\Users\\me\\report.csv")).unwrap());
    });
    bench("Uri::from_str (no-backslash path)", n, || {
        black_box(Uri::from_str(black_box("file:/var/log/syslog")).unwrap());
    });

    println!("== media inference ==");
    bench("MimeType::from_extension", n, || {
        black_box(MimeType::from_extension(black_box("parquet")));
    });
    bench("MimeType::from_magic", n, || {
        black_box(MimeType::from_magic(black_box(b"PK\x03\x04\x14\x00\x00")));
    });
    bench("MediaType::from_path (csv.gz)", n, || {
        black_box(MediaType::from_path(black_box("/data/sales/report.csv.gz")));
    });

    println!("== rendering / query ==");
    let url = Url::from_str("https://user:pw@example.com:8443/api?a=1&a=2#top").unwrap();
    bench("Url::to_str(true)", n, || {
        black_box(black_box(&url).to_str(true));
    });
    let q = Uri::from_str("https://h/p?a=1&a=2&b=hello%20world").unwrap();
    bench("Uri::params(true)", n, || {
        black_box(black_box(&q).params(true));
    });
}
