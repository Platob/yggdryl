//! Scoped chain lookup and epoch-grid snapshots through the same transition.

use std::collections::HashSet;
use std::hint::black_box;
use std::sync::Arc;

use criterion::{BatchSize, Criterion, Throughput};
use yggdryl::types::Uuid;
use yggdryl::{
    ALTIDS_TAG_NAME, FixCodec, FixLifecycle, INSTUUID_TAG_NAME, PREVTIMESTAMP_TAG_NAME,
    PREVUUID_TAG_NAME, PUUID_TAG_NAME, Scalar, TimeUnit, Timezone, UPDATEDAT_TAG_NAME,
};

use super::seed;

const SCOPES: usize = crate::bench_profile::corpus(128, 4);

pub fn benchmarks(criterion: &mut Criterion) {
    let registry = Arc::new(seed());
    let codec = FixCodec::new(Arc::clone(&registry));
    let message = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=ORDER-REUSED|60=20260102-10:15:30.000|10=0|")
        .expect("one order frame");
    assert_eq!(
        registry
            .msgtype("D")
            .unwrap()
            .identifier_values(&message)
            .count(),
        1,
        "the registered component selects the order identifier",
    );
    let mut derived = Vec::with_capacity(SCOPES * 2);
    for scope in 0..SCOPES {
        let mut scoped = message.clone();
        scoped
            .set(
                INSTUUID_TAG_NAME.0,
                Scalar::Uuid(Uuid::from_v8(scope as u128)),
            )
            .expect("a native instrument UUID");
        derived.extend([scoped.clone(), scoped]);
    }
    let stated = codec
        .enrich_messages(derived.clone())
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("the shared selector builds the maps");
    let mut empty = derived.clone();
    for message in &mut empty {
        message
            .set(ALTIDS_TAG_NAME.0, Scalar::from_mapping([]).unwrap())
            .expect("an authoritative empty map");
    }

    let mut group = criterion.benchmark_group("fix/lifecycle");
    for (name, rows, expected_chains) in [
        ("scoped_identifiers", derived, SCOPES),
        ("scoped_altids", stated, SCOPES),
        ("empty_altids", empty, 0),
    ] {
        // Outside measurement: identical text and clocks must still open
        // separate scopes, while an empty map suppresses the scalar ID.
        let mut life = FixLifecycle::new(Arc::clone(&registry));
        let mut identities = HashSet::new();
        let mut previous = None;
        for (index, message) in rows.iter().enumerate() {
            let stamped = life.fill(message.clone()).expect("a scoped message");
            if expected_chains != 0 && index % 2 == 1 {
                let (timestamp, uuid) = previous.as_ref().expect("the scope's first message");
                assert_eq!(stamped.by_tag(PREVTIMESTAMP_TAG_NAME.0).unwrap(), timestamp);
                assert_eq!(stamped.by_tag(PREVUUID_TAG_NAME.0).unwrap(), uuid);
            } else {
                for tag in [PREVTIMESTAMP_TAG_NAME.0, PREVUUID_TAG_NAME.0] {
                    assert_eq!(stamped.by_tag(tag).unwrap(), &Scalar::Null, "{name}");
                }
            }
            previous = Some((stamped.updatedat().clone(), stamped.uuid().clone()));
            if expected_chains != 0 {
                let Scalar::Uuid(value) = stamped.by_tag(PUUID_TAG_NAME.0).unwrap() else {
                    panic!("a native code identity");
                };
                identities.insert(*value);
            }
        }
        assert_eq!(life.alive(), expected_chains, "{name}");
        assert_eq!(identities.len(), expected_chains, "{name}");
        group.throughput(Throughput::Elements(rows.len() as u64));
        group.bench_function(name, |bencher| {
            bencher.iter_batched(
                || rows.clone(),
                |messages| {
                    let mut life = FixLifecycle::new(Arc::clone(&registry));
                    for message in messages {
                        black_box(life.fill(message).expect("a scoped message"));
                    }
                    life
                },
                BatchSize::LargeInput,
            );
        });
        let off_grid: Vec<_> = rows
            .iter()
            .enumerate()
            .map(|(index, message)| {
                let mut message = message.clone();
                let nanos = message.updatedat().as_datetime64().unwrap().0;
                message
                    .set(
                        UPDATEDAT_TAG_NAME.0,
                        Scalar::datetime64(
                            nanos + 1 + (index % 2) as i64,
                            TimeUnit::Nanosecond,
                            Timezone::UTC,
                        )
                        .unwrap(),
                    )
                    .expect("two arrivals inside the same second");
                message
            })
            .collect();
        let snapshots = FixLifecycle::new(Arc::clone(&registry))
            .snapshots(off_grid.clone().into_iter().map(Ok))
            .collect::<yggdryl::Result<Vec<_>>>()
            .expect("the same transition filters only repeated buckets");
        assert_eq!(snapshots.len(), expected_chains, "{name}");
        for snapshot in &snapshots {
            assert_eq!(snapshot.updatedat(), message.updatedat());
            assert!(snapshot.by_tag(PREVUUID_TAG_NAME.0).unwrap().is_null());
        }
        group.bench_function(format!("{name}_snapshots"), |bencher| {
            bencher.iter_batched(
                || off_grid.clone(),
                |messages| {
                    for snapshot in FixLifecycle::new(Arc::clone(&registry))
                        .snapshots(messages.into_iter().map(Ok))
                    {
                        black_box(snapshot.expect("a scoped snapshot"));
                    }
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}
