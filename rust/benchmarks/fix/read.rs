use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, Throughput};
use yggdryl::{FixReader, Version};

use super::seed;

/// The four dialects a capture line arrives in, one row each.
///
/// Each is a real shape rather than a synthetic one: a framed tag stream with
/// prose either side, a bare tag stream, a bridge line keyed by name, and a
/// wide order with a repeating group.
const TAGGED: &str =
    "sending >> 8=FIX.4.4|9=176|35=D|11=ORDER-1|55=AAPL|54=1|38=100|40=2|10=203| << queued";
const BARE: &str = "8=FIX.4.4|9=176|35=D|11=ORDER-1|55=AAPL|54=1|38=100|40=2|10=203|";
const NAMED: &str =
    "ACCOUNT=A1|MSGTYPE=D|CLORDID=ORDER-1|SYMBOL=AAPL|SIDE=1|ORDERQTY=100|ORDTYPE=2";
const GROUPED: &str = "MSGTYPE=D|#NOPARTYIDS=2|#NOPARTYIDS[0]=PARTYID=SYNTH-01\u{4}\u{3}PARTYIDSOURCE=D\u{4}\u{3}PARTYROLE=1|#NOPARTYIDS[1]=PARTYID=SYNTH-02\u{4}\u{3}PARTYIDSOURCE=D\u{4}\u{3}PARTYROLE=3";

pub fn benchmarks(criterion: &mut Criterion) {
    let reader = FixReader::new(Arc::new(seed()));
    let mut group = criterion.benchmark_group("fix/read");

    for (label, row) in [
        ("tagged", TAGGED),
        ("bare", BARE),
        ("named", NAMED),
        ("grouped", GROUPED),
    ] {
        group.throughput(Throughput::Bytes(row.len() as u64));
        group.bench_function(label, |bencher| {
            bencher.iter(|| {
                black_box(&reader)
                    .text(black_box(row))
                    .expect("a readable row")
            });
        });
    }

    // A row read as an older version pays the lineage projection per field,
    // which is the reason a version is resolved once and cached per field.
    let dated = reader
        .clone()
        .source_version("4.2".parse::<Version>().expect("a version"));
    group.bench_function("tagged_at_version", |bencher| {
        bencher.iter(|| {
            black_box(&dated)
                .text(black_box(BARE))
                .expect("a readable row")
        });
    });

    // The emit that closes the round trip, from the entries rather than the row.
    let message = reader.text(BARE).expect("a readable row");
    group.bench_function("emit", |bencher| {
        bencher.iter(|| black_box(&message).into_bytes(black_box(b'|')));
    });
    group.finish();
}
