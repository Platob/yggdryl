//! What a log record costs: refused before it is built, spelled into a
//! reused line, and published through a storage handle.
//!
//! The baseline row writes the same line with `std::fmt` into a reused
//! `String` - what a hand-rolled logger costs - so the formatter's rows
//! read as the price of Python's format language over it. The file rows
//! name the calls one record costs the handle, as `holder` does: a
//! publish per record is the default, a capacity turns it into one per
//! batch.

#[path = "bench_profile.rs"]
mod bench_profile;

use std::fmt::Write as _;
use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use yggdryl::holder::Buffer;
use yggdryl::holder::counted::Counted;
use yggdryl::logging::{self, FileHandler, Formatter, Handler, Level, Record, StreamHandler};

/// Records the file rows publish before each handle is swapped for a fresh
/// one, so a row measures appends and never an ever-longer buffer.
const RECORDS: usize = bench_profile::corpus(4_096, 64);

fn logging_benchmarks(criterion: &mut Criterion) {
    logging::install().expect("the tree is this process's logger");
    let mut group = criterion.benchmark_group("logging");

    let quiet = logging::get_logger("bench.quiet");
    group.bench_function("disabled/logger", |bencher| {
        bencher.iter(|| quiet.debug(black_box(format_args!("{} fills", 3))));
    });
    group.bench_function("disabled/facade", |bencher| {
        bencher.iter(|| log::debug!(target: "bench::quiet", "{} fills", black_box(3)));
    });

    let format = "%(asctime)s %(levelname)-8s %(name)s: %(message)s";
    let formatter = Formatter::from_str(format).expect("a format");
    let record = Record::new("bench.feed", Level::INFO, &"3 fills");
    let mut line = String::new();
    group.bench_function("format/std_fmt_baseline", |bencher| {
        bencher.iter(|| {
            line.clear();
            let (date, level, name) = (
                black_box("2026-10-03 14:05:09,123"),
                black_box("INFO"),
                black_box("bench.feed"),
            );
            let _ = write!(line, "{date} {level:<8} {name}: {}", black_box("3 fills"));
            black_box(line.len())
        });
    });
    group.bench_function("format/asctime_levelname_name_message", |bencher| {
        bencher.iter(|| {
            line.clear();
            formatter.format_into(black_box(&record), &mut line);
            black_box(line.len())
        });
    });

    // The default line every handler stating no format spells: plain, and as
    // a colour terminal shows it.
    let terminal = Formatter::terminal();
    let located = Record::new("bench.feed", Level::INFO, &"3 fills")
        .with_location("src/feed.rs", 42)
        .with_function("open")
        .with_thread("main");
    group.bench_function("format/terminal", |bencher| {
        bencher.iter(|| {
            line.clear();
            terminal.format_into(black_box(&located), &mut line);
            black_box(line.len())
        });
    });
    group.bench_function("format/terminal_colored", |bencher| {
        bencher.iter(|| {
            line.clear();
            terminal.format_colored_into(black_box(&located), &mut line);
            black_box(line.len())
        });
    });

    let loud = logging::get_logger("bench.loud");
    loud.set_level(Level::DEBUG);
    loud.set_propagating(false);
    let sink = StreamHandler::new(std::io::sink());
    sink.set_formatter(formatter.clone());
    loud.add_handler(Arc::new(sink));
    group.bench_function("enabled/facade_to_sink", |bencher| {
        bencher.iter(|| log::info!(target: "bench::loud", "{} fills", black_box(3)));
    });

    // A repeat on a deduplicating logger: its message hashed, counted in the
    // lock-free table and dropped before any handler.
    let repeated = logging::get_logger("bench.repeated");
    repeated.set_level(Level::DEBUG);
    repeated.set_propagating(false);
    let quiet_sink = StreamHandler::new(std::io::sink());
    quiet_sink.set_formatter(formatter.clone());
    repeated.add_handler(Arc::new(quiet_sink));
    repeated.set_deduplicating(Some(true));
    group.bench_function("repeated/deduplicated", |bencher| {
        bencher.iter(|| repeated.info(black_box(format_args!("{} fills", 3))));
    });
    let repeats = logging::Repeats::new();
    group.bench_function("repeated/stable_hash_and_count", |bencher| {
        bencher.iter(|| black_box(repeats.count_record(black_box(&record))));
    });

    for (name, capacity) in [
        ("file/append_each [append_bytes=1]", 0),
        ("file/capacity_64k [append_bytes=1 per 64 KiB]", 64 * 1024),
    ] {
        group.bench_function(name, |bencher| {
            bencher.iter_batched(
                || FileHandler::new(Counted::new(Buffer::new())).with_capacity(capacity),
                |handler| {
                    for _ in 0..RECORDS {
                        handler.handle(black_box(&record));
                    }
                    handler.flush().expect("a flush");
                    handler
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

criterion_group!(logging_group, logging_benchmarks);
criterion_main!(logging_group);
