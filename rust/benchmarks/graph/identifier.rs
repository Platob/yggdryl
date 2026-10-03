//! Identifier keys and maps: an `IdKey` spelled and read, an `Identifiers`
//! map crossing a `Scalar` and built entry by entry, and the three
//! identifier columns of a `marketdata` batch written and read.

use std::hint::black_box;

use criterion::{BatchSize, Criterion, Throughput};
use smol_str::SmolStr;
use yggdryl::arrow::batch_reader;
use yggdryl::graph::{Element, Event, Market, MarketData, Operation, OrderEvent};
use yggdryl::{IdKey, IdSource, IdType, Identifier, Identifiers, Side, State};

/// The security types a map states, each under the base key and under a
/// bridge's own source.
const SECURITIES: [(IdType, &str); 8] = [
    (IdType::Isin, "US0378331005"),
    (IdType::Cusip, "037833100"),
    (IdType::Sedol, "2046251"),
    (IdType::Figi, "BBG000B9XRY4"),
    (IdType::Ric, "AAPL.OQ"),
    (IdType::Bloomberg, "AAPL US Equity"),
    (IdType::Valor, "908440"),
    (IdType::Wkn, "865985"),
];

/// Eight securities under the bridge `oms`: inserting each also fills its
/// base key, so the map holds sixteen.
fn sourced() -> Vec<Identifier> {
    let oms: IdSource = "oms".parse().expect("a source word");
    SECURITIES
        .iter()
        .map(|(kind, value)| {
            Identifier::new(IdKey::new(oms.clone(), kind.clone()), value).expect("a bench code")
        })
        .collect()
}

/// One dated order of `code` stating three securities, two identifiers and
/// a party.
fn order(unix: i64, code: &str) -> MarketData {
    let mut event = OrderEvent::at(unix);
    event.set_crosscode(code.to_owned());
    event.set_side(Side::read("Buy").expect("a shipped side"), true);
    event.set_state(State::read("New").expect("the shipped new state"));
    for (kind, value) in &SECURITIES[..3] {
        event
            .insert_securityid(Identifier::new(IdKey::base(kind.clone()), value).expect("a code"))
            .expect("a security identifier");
    }
    for (kind, value) in [(IdType::OrderId, code), (IdType::ClOrdId, "C-1")] {
        event
            .insert_identifier(Identifier::new(IdKey::base(kind), value).expect("an id"))
            .expect("an operation identifier");
    }
    event
        .insert_partyid(Identifier::new(IdKey::base(IdType::Account), "ACC-1").expect("an account"))
        .expect("a party");
    event.finalize();
    MarketData::from(event)
}

pub fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("graph/identifier");

    let known = IdKey::new(IdSource::Derived, IdType::Cusip);
    let other: IdKey = "omsbridge:instrumentid".parse().expect("a bridge key");
    group.bench_function("idkey/spelled_known", |bencher| {
        bencher.iter(|| SmolStr::from(black_box(&known)));
    });
    group.bench_function("idkey/spelled_other", |bencher| {
        bencher.iter(|| SmolStr::from(black_box(&other)));
    });
    group.bench_function("idkey/read_known", |bencher| {
        bencher.iter(|| {
            black_box("derived:cusip")
                .parse::<IdKey>()
                .expect("a member pair")
        });
    });
    group.bench_function("idkey/read_other", |bencher| {
        bencher.iter(|| {
            black_box("omsbridge:instrumentid")
                .parse::<IdKey>()
                .expect("a bridge key")
        });
    });

    let entries = sourced();
    let map: Identifiers = entries.iter().cloned().collect();
    let scalar = map.into_scalar();
    group.bench_function("identifiers/into_scalar_16", |bencher| {
        bencher.iter(|| black_box(&map).into_scalar());
    });
    group.bench_function("identifiers/from_scalar_16", |bencher| {
        bencher.iter(|| Identifiers::from_scalar(black_box(&scalar)).expect("a map"));
    });
    group.bench_function("identifiers/insert_sourced_16", |bencher| {
        bencher.iter_batched(
            Identifiers::default,
            |mut held| {
                for id in &entries {
                    held.insert(id.clone());
                }
                held
            },
            BatchSize::SmallInput,
        );
    });
    let mut derived = map.clone();
    derived.insert(
        Identifier::new(IdKey::new(IdSource::Derived, IdType::Common), "C-1").expect("a code"),
    );
    let derivation = IdKey::new(IdSource::Derived, IdType::Common);
    group.bench_function("identifiers/remove_derived", |bencher| {
        bencher.iter_batched(
            || derived.clone(),
            |mut held| {
                black_box(held.remove(black_box(&derivation)));
                held
            },
            BatchSize::SmallInput,
        );
    });

    let rows = crate::bench_profile::corpus(4_096, 32);
    let orders: Vec<MarketData> = (0..rows)
        .map(|row| {
            order(
                1 + i64::try_from(row).expect("a bench corpus"),
                &format!("O-{row}"),
            )
        })
        .collect();
    let batches = MarketData::arrow_reader(orders.clone(), None, None)
        .expect("the bench stream")
        .collect::<Result<Vec<_>, _>>()
        .expect("every order lays out");
    let schema = batches[0].schema();
    group.throughput(Throughput::Elements(
        u64::try_from(rows).expect("a bench corpus"),
    ));
    group.bench_function(format!("marketdata/ids_write_{rows}"), |bencher| {
        bencher.iter_batched(
            || orders.clone(),
            |held| {
                MarketData::arrow_reader(held, None, None)
                    .expect("the stream")
                    .map(|batch| batch.expect("a batch").num_rows())
                    .sum::<usize>()
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function(format!("marketdata/ids_read_{rows}"), |bencher| {
        bencher.iter_batched(
            || batch_reader(schema.clone(), batches.clone()),
            |reader| {
                MarketData::from_arrow_reader(reader)
                    .expect("the columns bind")
                    .try_fold(0_usize, |read, value| value.map(|_| read + 1))
                    .expect("a row reads")
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}
