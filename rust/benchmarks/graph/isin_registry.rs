//! The ISIN registry: learning and filling a market element, and its table
//! streamed out, loaded back and round-tripped through an Arrow IPC holder.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, Throughput};
use smol_str::SmolStr;
use yggdryl::arrow::batch_reader;
use yggdryl::graph::{Market, OrderEvent};
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{
    Cfi, IOMedia, IOMode, IdKey, IdType, Identifier, Isin, IsinEntry, IsinRegistry, Mic, MimeType,
};

/// The ISIN numbered `number` under the two-letter `prefix`.
fn isin(prefix: &str, number: usize) -> String {
    let body = format!("{prefix}{number:09}");
    let digit = Isin::closing_digit(&body).expect("a bench ISIN body");
    format!("{body}{digit}")
}

/// `size` instruments, each with a CFI code, a market, a ticker, a common
/// code and a RIC.
fn registry(size: usize) -> IsinRegistry {
    let mut registry = IsinRegistry::new();
    for number in 0..size {
        registry
            .merge(
                IsinEntry::new(Isin::new(isin("FR", number)).expect("a bench ISIN"))
                    .with_updunix(Some(1))
                    .with_cficode(Some(Cfi::new("ESVUFR").expect("a CFI code")))
                    .with_miccode(Some(Mic::new("XPAR").expect("a market")))
                    .with_ticker(Some(SmolStr::new(format!("T{number}"))))
                    .try_with_code(IdType::Common, &format!("C-{number}"))
                    .expect("a common code")
                    .try_with_code(IdType::Ric, &format!("R{number}.PA"))
                    .expect("a RIC"),
            )
            .expect("a bench row");
    }
    registry
}

/// An order at `unix` stating each of `codes`.
fn stating(unix: i64, codes: &[(IdType, &str)]) -> OrderEvent {
    let mut event = OrderEvent::at(unix);
    for (kind, value) in codes {
        event
            .insert_securityid(Identifier::new(IdKey::base(kind.clone()), value).expect("a code"))
            .expect("a security identifier");
    }
    event
}

pub fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("graph/isin_registry");
    let size = crate::bench_profile::corpus(4_096, 32);
    let mut held = registry(size);
    let known = isin("FR", size / 2);

    let mut nothing_new = stating(
        2,
        &[
            (IdType::Isin, known.as_str()),
            (IdType::Common, &format!("C-{}", size / 2)),
        ],
    );
    nothing_new.set_miccode(Some(Mic::new("XPAR").expect("a market")), true);
    group.bench_function("learn_known", |bencher| {
        bencher.iter(|| held.learn(black_box(&nothing_new)));
    });
    let new = isin("BE", 1);
    let mut fresh = stating(
        2,
        &[(IdType::Isin, new.as_str()), (IdType::Common, "C-NEW")],
    );
    fresh.set_cficode(Some(Cfi::new("ESVUFR").expect("a CFI code")), true);
    group.bench_function("learn_new", |bencher| {
        bencher.iter_batched(
            || registry(size),
            // The table is handed back, so its drop is never timed.
            |mut table| {
                black_box(table.learn(black_box(&fresh)));
                table
            },
            BatchSize::LargeInput,
        );
    });
    let by_isin = stating(3, &[(IdType::Isin, known.as_str())]);
    group.bench_function("fill_by_isin", |bencher| {
        bencher.iter_batched(
            || by_isin.clone(),
            |mut element| {
                black_box(held.fill(&mut element));
                element
            },
            BatchSize::SmallInput,
        );
    });
    let ric = format!("R{}.PA", size / 2);
    let by_ric = stating(3, &[(IdType::Ric, ric.as_str())]);
    group.bench_function("fill_by_ric", |bencher| {
        bencher.iter_batched(
            || by_ric.clone(),
            |mut element| {
                black_box(held.fill(&mut element));
                element
            },
            BatchSize::SmallInput,
        );
    });
    let unknown = isin("DE", 1);
    let mut miss = stating(3, &[(IdType::Isin, unknown.as_str())]);
    group.bench_function("fill_miss", |bencher| {
        bencher.iter(|| held.fill(black_box(&mut miss)));
    });

    group.throughput(Throughput::Elements(
        u64::try_from(size).expect("a bench corpus"),
    ));
    group.bench_function(format!("into_arrow_reader_{size}"), |bencher| {
        bencher.iter(|| {
            held.into_arrow_reader()
                .expect("a snapshot")
                .map(|batch| batch.expect("a batch").num_rows())
                .sum::<usize>()
        });
    });
    let batches = held
        .into_arrow_reader()
        .expect("a snapshot")
        .collect::<Result<Vec<_>, _>>()
        .expect("every row lays out");
    let schema = batches[0].schema();
    group.bench_function(format!("extend_from_arrow_reader_{size}"), |bencher| {
        bencher.iter_batched(
            || batch_reader(schema.clone(), batches.clone()),
            |reader| {
                IsinRegistry::from_arrow_reader(reader)
                    .expect("the rows load")
                    .len()
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function(format!("ipc_roundtrip_{size}"), |bencher| {
        bencher.iter(|| {
            let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
            let options = handle
                .record_options()
                .expect("IPC options")
                .with_field(IsinEntry::field());
            handle
                .write_arrow_reader(
                    held.into_arrow_reader().expect("a snapshot"),
                    IOMode::Overwrite,
                    &options,
                )
                .expect("the snapshot writes");
            IsinRegistry::from_handle(&handle)
                .expect("the snapshot reads")
                .len()
        });
    });
    group.finish();
}
