use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, Throughput};
use yggdryl::media::IORecordOptions;
use yggdryl::{FixBatchReader, FixCodec, FixDedup, FixOptions};

use super::seed;

/// How many capture lines one measured run reads.
const ROWS: usize = crate::bench_profile::corpus(2_000, 200);

/// A capture of ordinary orders, one line each.
fn capture() -> Vec<yggdryl::Result<Vec<u8>>> {
    (0..ROWS)
        .map(|index| {
            Ok(format!(
                "sending >> 8=FIX.4.4|9=176|35=D|49=SENDER|56=TARGET|34={index}|11=ORDER-{index:06}|55=AAPL|54=1|38=100|44=12.5|10=203|"
            )
            .into_bytes())
        })
        .collect()
}

pub fn benchmarks(criterion: &mut Criterion) {
    let registry = Arc::new(seed());
    let bytes: u64 = capture()
        .iter()
        .map(|row| row.as_ref().map_or(0, |held| held.len() as u64))
        .sum();

    let mut group = criterion.benchmark_group("fix/batch");
    group.throughput(Throughput::Bytes(bytes));

    // A capture in, columns out: the whole phase, per row.
    group.bench_function("rows", |bencher| {
        bencher.iter(|| {
            let reader =
                FixBatchReader::from_rows(Arc::clone(&registry), capture(), FixOptions::new())
                    .expect("a reader");
            reader
                .map(|batch| batch.expect("a batch").num_rows())
                .sum::<usize>()
        });
    });

    // The same, closing batches on bytes rather than on rows.
    group.bench_function("rows_byte_bounded", |bencher| {
        bencher.iter(|| {
            let options = FixOptions::new().with_batch_byte_size(1 << 16);
            let reader = FixBatchReader::from_rows(Arc::clone(&registry), capture(), options)
                .expect("a reader");
            reader
                .map(|batch| batch.expect("a batch").num_rows())
                .sum::<usize>()
        });
    });

    // What the opt-in filter costs on a capture with nothing to drop.
    group.bench_function("rows_dedup", |bencher| {
        bencher.iter(|| {
            let options = FixOptions::new().with_dedup(true);
            let reader = FixBatchReader::from_rows(Arc::clone(&registry), capture(), options)
                .expect("a reader");
            reader
                .map(|batch| batch.expect("a batch").num_rows())
                .sum::<usize>()
        });
    });
    group.finish();

    // The digest alone, which the dedup and the column both pay.
    let reader = FixCodec::new(Arc::clone(&registry));
    let message = reader
        .read_line(b"8=FIX.4.4|9=176|35=D|49=SENDER|56=TARGET|34=7|11=ORDER-1|55=AAPL|54=1|38=100|44=12.5|10=203|")
        .expect("a readable row");
    let mut group = criterion.benchmark_group("fix/digest");
    group.bench_function("message", |bencher| {
        bencher.iter(|| black_box(&message).digest());
    });
    group.bench_function("dedup", |bencher| {
        bencher.iter(|| {
            let held = std::iter::repeat_n(message.clone(), 16);
            FixDedup::new(held).count()
        });
    });
    group.finish();
}
