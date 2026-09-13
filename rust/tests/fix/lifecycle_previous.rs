//! Decision 24: each live chain remembers only its last successful clock and UUID.

use std::sync::Arc;

use super::SoleMessage;
use super::lifecycle_chains::row;
use yggdryl::types::Uuid;
use yggdryl::{
    ALTIDS_TAG_NAME, Error, FixCodec, FixLifecycle, FixMsg, FixRegistry, INSTUUID_TAG_NAME,
    PREVTIMESTAMP_TAG_NAME, PREVUUID_TAG_NAME, PUUID_TAG_NAME, Scalar, TIMESTAMP_TAG_NAME,
    TimeUnit, Timezone, UUID_TAG_NAME, arrow, fix_schema,
};

fn clock(nanos: i64) -> Scalar {
    Scalar::datetime64(nanos, TimeUnit::Nanosecond, Timezone::UTC).unwrap()
}

fn identifiers(entries: &[(&str, &str)]) -> (i32, Scalar) {
    (
        ALTIDS_TAG_NAME.0,
        Scalar::from_mapping(
            entries
                .iter()
                .map(|(key, value)| (Scalar::from(*key), Scalar::from(*value))),
        )
        .unwrap(),
    )
}

fn event(
    registry: &Arc<FixRegistry>,
    nanos: i64,
    current: u128,
    persistent: Option<u128>,
    scope: Option<u128>,
    entries: &[(&str, &str)],
) -> FixMsg {
    let mut cells = vec![
        (TIMESTAMP_TAG_NAME.0, clock(nanos)),
        (UUID_TAG_NAME.0, Scalar::Uuid(Uuid::new(current))),
        identifiers(entries),
    ];
    cells.extend(persistent.map(|value| (PUUID_TAG_NAME.0, Scalar::Uuid(Uuid::new(value)))));
    cells.extend(scope.map(|value| (INSTUUID_TAG_NAME.0, Scalar::Uuid(Uuid::new(value)))));
    row(registry, cells)
}

fn uuid(message: &FixMsg, tag: i32) -> Uuid {
    let Scalar::Uuid(value) = message.by_tag(tag).unwrap() else {
        panic!("tag {tag} must retain a native UUID");
    };
    *value
}

fn assert_pair(message: &FixMsg, expected: Option<(i64, u128)>) {
    assert_eq!(
        message.by_tag(PREVTIMESTAMP_TAG_NAME.0).unwrap(),
        &expected.map_or(Scalar::Null, |(time, _)| clock(time)),
    );
    assert_eq!(
        message.by_tag(PREVUUID_TAG_NAME.0).unwrap(),
        &expected.map_or(Scalar::Null, |(_, id)| Scalar::Uuid(Uuid::new(id))),
    );
}

fn located(error: Error, expected: &str) {
    let Error::InvalidRecord { path, reason } = error else {
        panic!("expected a located previous-value refusal, got {error}");
    };
    assert_eq!(path, expected);
    assert!(reason.len() < 256, "{reason}");
}

#[test]
fn three_messages_follow_arrival_order_without_sorting_their_clocks() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let mut previous = None;
    for (time, id) in [(30, 101), (10, 102), (20, 103)] {
        let message = life
            .fill(event(&registry, time, id, Some(7), None, &[("id", "A")]))
            .unwrap();
        assert_pair(&message, previous);
        assert_eq!(message.by_tag(TIMESTAMP_TAG_NAME.0).unwrap(), &clock(time));
        assert_eq!(uuid(&message, UUID_TAG_NAME.0), Uuid::new(id));
        assert_eq!(uuid(&message, PUUID_TAG_NAME.0), Uuid::new(7));
        previous = Some((time, id));
    }
    assert_eq!(life.alive(), 1);
}

#[test]
fn previous_fields_are_independent_statements_not_the_stored_current_pair() {
    let registry = Arc::new(FixRegistry::new());
    for (stated_time, stated_uuid) in [
        (None, None),
        (Some(900), None),
        (None, Some(901)),
        (Some(900), Some(901)),
    ] {
        let mut life = FixLifecycle::new(Arc::clone(&registry));
        let mut first = event(&registry, 10, 101, Some(7), None, &[]);
        first
            .set_many([
                (PREVTIMESTAMP_TAG_NAME.0, clock(800)),
                (PREVUUID_TAG_NAME.0, Scalar::Uuid(Uuid::new(801))),
            ])
            .unwrap();
        let first = life.fill(first).unwrap();
        assert_pair(&first, Some((800, 801)));

        let mut second = event(&registry, 20, 102, Some(7), None, &[]);
        second
            .set_many([
                (
                    PREVTIMESTAMP_TAG_NAME.0,
                    stated_time.map_or(Scalar::Null, clock),
                ),
                (
                    PREVUUID_TAG_NAME.0,
                    stated_uuid.map_or(Scalar::Null, |id| Scalar::Uuid(Uuid::new(id))),
                ),
            ])
            .unwrap();
        let second = life.fill(second).unwrap();
        assert_pair(
            &second,
            Some((stated_time.unwrap_or(10), stated_uuid.unwrap_or(101))),
        );
        let third = life
            .fill(event(&registry, 30, 103, Some(7), None, &[]))
            .unwrap();
        assert_pair(&third, Some((20, 102)));
    }
}

#[test]
fn absent_nil_and_distinct_scopes_keep_separate_previous_pairs() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    for (scope, persistent, time, id) in [
        (None, 11, 10, 101),
        (Some(0), 12, 20, 102),
        (Some(1), 13, 30, 103),
        (Some(2), 14, 40, 104),
    ] {
        let first = life
            .fill(event(
                &registry,
                time,
                id,
                Some(persistent),
                scope,
                &[("id", "SAME")],
            ))
            .unwrap();
        assert_pair(&first, None);
    }
    for (scope, persistent, time, id) in [
        (None, 11, 10, 101),
        (Some(0), 12, 20, 102),
        (Some(1), 13, 30, 103),
        (Some(2), 14, 40, 104),
    ] {
        let corrected = life
            .fill(event(
                &registry,
                time + 100,
                id + 100,
                Some(999),
                scope,
                &[("id", "SAME")],
            ))
            .unwrap();
        assert_eq!(uuid(&corrected, PUUID_TAG_NAME.0), Uuid::new(persistent));
        assert_pair(&corrected, Some((time, id)));
    }
    assert_eq!(life.alive(), 4);
}

#[test]
fn direct_join_and_foreign_correction_read_only_the_selected_chains_history() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    life.fill(event(&registry, 10, 101, Some(11), Some(1), &[("id", "A")]))
        .unwrap();
    life.fill(event(&registry, 20, 201, Some(22), Some(2), &[("id", "B")]))
        .unwrap();
    let direct = life
        .fill(event(
            &registry,
            30,
            102,
            Some(11),
            Some(2),
            &[("a", "B"), ("b", "ATTACHED")],
        ))
        .unwrap();
    assert_pair(&direct, Some((10, 101)));
    let other = life
        .fill(event(&registry, 40, 202, None, Some(2), &[("id", "B")]))
        .unwrap();
    assert_eq!(uuid(&other, PUUID_TAG_NAME.0), Uuid::new(22));
    assert_pair(&other, Some((20, 201)));
    let corrected = life
        .fill(event(
            &registry,
            50,
            103,
            Some(999),
            Some(2),
            &[("id", "ATTACHED")],
        ))
        .unwrap();
    assert_eq!(uuid(&corrected, PUUID_TAG_NAME.0), Uuid::new(11));
    assert_pair(&corrected, Some((30, 102)));
    assert_eq!(life.alive(), 2);
}

#[test]
fn previous_timestamp_uses_the_capture_clock_with_its_nanoseconds_not_uuid_time() {
    let registry = super::committed_registry();
    let codec = FixCodec::new(Arc::clone(&registry));
    let mut first = codec
        .sole_line(
            b"8=FIX.4.4|35=D|11=CAPTURE-1|60=19700101-00:00:01.000002|10=0|",
            false,
        )
        .unwrap();
    first
        .set(TIMESTAMP_TAG_NAME.0, clock(5_000_000_123))
        .unwrap();
    let expected_uuid = Uuid::from_v7(
        1_000_002,
        yggdryl::hashing::xxhash::xxh3(&first.digest().to_be_bytes()),
    )
    .unwrap();
    let mut life = FixLifecycle::new(registry);
    let first = life.fill(first).unwrap();
    assert_pair(&first, None);
    assert_eq!(uuid(&first, UUID_TAG_NAME.0), expected_uuid);
    let mut second = codec
        .sole_line(
            b"8=FIX.4.4|35=D|11=CAPTURE-1|60=19700101-00:00:02.000004|10=0|",
            false,
        )
        .unwrap();
    second
        .set(TIMESTAMP_TAG_NAME.0, clock(6_000_000_789))
        .unwrap();
    let second = life.fill(second).unwrap();
    assert_eq!(
        second.by_tag(PREVTIMESTAMP_TAG_NAME.0).unwrap(),
        &clock(5_000_000_123),
    );
    assert_eq!(uuid(&second, PREVUUID_TAG_NAME.0), expected_uuid);
    assert_eq!(
        second.by_tag(TIMESTAMP_TAG_NAME.0).unwrap(),
        &clock(6_000_000_789),
    );
}

#[test]
fn current_history_clock_reuses_market_fallback_and_restates_once_to_nanoseconds() {
    let registry = Arc::new(FixRegistry::new());
    let micros = Scalar::datetime64(17, TimeUnit::Microsecond, Timezone::UTC).unwrap();
    let millis = Scalar::datetime64(19, TimeUnit::Millisecond, Timezone::UTC).unwrap();
    for (capture, wire, expected) in [
        (None, None, 0),
        (Some(Scalar::Null), None, 0),
        (Some(micros), Some(millis.clone()), 17_000),
        (Some(Scalar::Null), Some(millis.clone()), 19_000_000),
        (None, Some(millis), 19_000_000),
    ] {
        let mut cells = vec![
            (UUID_TAG_NAME.0, Scalar::Uuid(Uuid::new(101))),
            (PUUID_TAG_NAME.0, Scalar::Uuid(Uuid::new(7))),
        ];
        cells.extend(capture.map(|value| (TIMESTAMP_TAG_NAME.0, value)));
        cells.extend(wire.map(|value| (60, value)));
        let mut life = FixLifecycle::new(Arc::clone(&registry));
        let first = life.fill(row(&registry, cells)).unwrap();
        assert_pair(&first, None);
        let next = life
            .fill(event(&registry, 20_000_000, 102, Some(7), None, &[]))
            .unwrap();
        assert_pair(&next, Some((expected, 101)));
    }
}

#[test]
fn terminal_clear_and_reopening_forget_every_scopes_previous_pair() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    life.fill(event(&registry, 10, 101, Some(11), Some(1), &[("id", "A")]))
        .unwrap();
    let attached = life
        .fill(event(&registry, 20, 102, Some(11), Some(2), &[("id", "B")]))
        .unwrap();
    assert_pair(&attached, Some((10, 101)));
    let mut terminal = event(&registry, 30, 103, Some(999), Some(2), &[("id", "B")]);
    terminal.set(39, Scalar::from("2")).unwrap();
    let terminal = life.fill(terminal).unwrap();
    assert_eq!(uuid(&terminal, PUUID_TAG_NAME.0), Uuid::new(11));
    assert_pair(&terminal, Some((20, 102)));
    assert_eq!(life.alive(), 0);
    let reopened = life
        .fill(event(&registry, 40, 104, Some(11), Some(1), &[("id", "A")]))
        .unwrap();
    assert_pair(&reopened, None);
    let attached = life
        .fill(event(&registry, 50, 105, Some(11), Some(2), &[("id", "B")]))
        .unwrap();
    assert_pair(&attached, Some((40, 104)));
    life.clear();
    assert_eq!(life.alive(), 0);
    let fresh = life
        .fill(event(&registry, 60, 106, Some(11), Some(1), &[("id", "A")]))
        .unwrap();
    assert_pair(&fresh, None);
    let mut first_terminal = event(&registry, 70, 107, Some(12), None, &[("id", "C")]);
    first_terminal.set(39, Scalar::from("2")).unwrap();
    assert_pair(&life.fill(first_terminal).unwrap(), None);
    assert_eq!(life.alive(), 1);
    let reopened = life
        .fill(event(&registry, 80, 108, Some(12), None, &[("id", "C")]))
        .unwrap();
    assert_pair(&reopened, None);
    assert_eq!(life.alive(), 2);
}

#[test]
fn an_earlier_message_into_advanced_state_is_an_arrival_not_a_rewind() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let first = life
        .fill(event(&registry, 10, 101, Some(7), None, &[]))
        .unwrap();
    life.fill(event(&registry, 20, 102, Some(7), None, &[]))
        .unwrap();
    life.fill(event(&registry, 30, 103, Some(7), None, &[]))
        .unwrap();
    let arrived_again = life.fill(first).unwrap();
    assert_pair(&arrived_again, Some((30, 103)));
    let next = life
        .fill(event(&registry, 40, 104, Some(7), None, &[]))
        .unwrap();
    assert_pair(&next, Some((10, 101)));
}

#[test]
fn malformed_stated_previous_values_neither_advance_attach_nor_close() {
    let registry = Arc::new(FixRegistry::new());
    let invalid = [
        (PREVTIMESTAMP_TAG_NAME, Scalar::from("界".repeat(128))),
        (PREVTIMESTAMP_TAG_NAME, Scalar::date32(0)),
        (
            PREVTIMESTAMP_TAG_NAME,
            Scalar::datetime64(0, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
        ),
        (
            PREVTIMESTAMP_TAG_NAME,
            Scalar::datetime64(0, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
        ),
        (
            PREVUUID_TAG_NAME,
            Scalar::from("00000000-0000-0000-0000-000000000001"),
        ),
        (PREVUUID_TAG_NAME, Scalar::from(vec![0_u8; 16])),
        (PREVUUID_TAG_NAME, Scalar::from(1_i64)),
        (PREVUUID_TAG_NAME, clock(0)),
    ];
    for ((tag, name), value) in invalid {
        for terminal in [false, true] {
            let mut life = FixLifecycle::new(Arc::clone(&registry));
            life.fill(event(&registry, 10, 101, Some(11), None, &[("id", "LIVE")]))
                .unwrap();
            let mut cells = vec![
                (TIMESTAMP_TAG_NAME.0, clock(20)),
                (UUID_TAG_NAME.0, Scalar::Uuid(Uuid::new(102))),
                (PUUID_TAG_NAME.0, Scalar::Uuid(Uuid::new(11))),
                identifiers(&[("a", "LIVE"), ("b", "NEW")]),
                (tag, value.clone()),
            ];
            if terminal {
                cells.push((39, Scalar::from("2")));
            }
            located(
                life.fill(row(&registry, cells)).unwrap_err(),
                &format!("$.{name}"),
            );
            assert_eq!(life.alive(), 1);
            let retained = life
                .fill(event(&registry, 30, 103, None, None, &[("id", "LIVE")]))
                .unwrap();
            assert_pair(&retained, Some((10, 101)));
            assert_eq!(uuid(&retained, PUUID_TAG_NAME.0), Uuid::new(11));
            let independent = life
                .fill(event(&registry, 40, 104, None, None, &[("id", "NEW")]))
                .unwrap();
            assert_pair(&independent, None);
            assert_ne!(uuid(&independent, PUUID_TAG_NAME.0), Uuid::new(11));
            assert_eq!(life.alive(), 2);
        }
    }
}

#[test]
fn an_unrepresentable_live_history_clock_refuses_before_opening_or_advancing() {
    let registry = Arc::new(FixRegistry::new());
    let micros = i64::MAX / 1_000 + 1;
    assert!(Uuid::from_v7(micros, 0).is_ok());
    for existing in [false, true] {
        let mut life = FixLifecycle::new(Arc::clone(&registry));
        if existing {
            life.fill(event(&registry, 10, 101, Some(11), None, &[("id", "LIVE")]))
                .unwrap();
        }
        let refused = row(
            &registry,
            [
                (
                    TIMESTAMP_TAG_NAME.0,
                    Scalar::datetime64(micros, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
                ),
                (PUUID_TAG_NAME.0, Scalar::Uuid(Uuid::new(11))),
                identifiers(&[("a", "LIVE"), ("b", "NEW")]),
            ],
        );
        located(life.fill(refused).unwrap_err(), "$.timestamp");
        assert_eq!(life.alive(), usize::from(existing));
        let retained = life
            .fill(event(&registry, 30, 103, None, None, &[("id", "LIVE")]))
            .unwrap();
        assert_pair(&retained, existing.then_some((10, 101)));
        let independent = life
            .fill(event(&registry, 40, 104, None, None, &[("id", "NEW")]))
            .unwrap();
        assert_pair(&independent, None);
        assert_ne!(
            uuid(&independent, PUUID_TAG_NAME.0),
            uuid(&retained, PUUID_TAG_NAME.0),
        );
    }
}

#[test]
fn orphan_and_terminal_messages_keep_uuid_range_without_retaining_history() {
    let registry = Arc::new(FixRegistry::new());
    let micros = i64::MAX / 1_000 + 1;
    for (terminal, existing) in [(false, false), (true, false), (true, true)] {
        let mut life = FixLifecycle::new(Arc::clone(&registry));
        if existing {
            life.fill(event(&registry, 10, 101, Some(11), None, &[("id", "LIVE")]))
                .unwrap();
        }
        let mut cells = vec![(
            TIMESTAMP_TAG_NAME.0,
            Scalar::datetime64(micros, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
        )];
        if terminal {
            cells.extend([
                (PUUID_TAG_NAME.0, Scalar::Uuid(Uuid::new(11))),
                (39, Scalar::from("2")),
                identifiers(&[("id", "LIVE")]),
            ]);
        }
        let message = row(&registry, cells);
        let expected = Uuid::from_v7(
            micros,
            yggdryl::hashing::xxhash::xxh3(&message.digest().to_be_bytes()),
        )
        .unwrap();
        let message = life.fill(message).unwrap();
        assert_eq!(uuid(&message, UUID_TAG_NAME.0), expected);
        assert_pair(&message, existing.then_some((10, 101)));
        assert_eq!(life.alive(), 0);
    }
}

#[test]
fn both_capture_doors_replay_previous_pairs_through_arrow_at_every_row_boundary() {
    let codec = super::dataset::codec();
    let lines = super::dataset::text_lines();
    assert_eq!(lines.len(), 129);
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    for (enrich, expected_alive) in [(false, 4), (true, 3)] {
        let mut life = FixLifecycle::new(Arc::clone(codec.registry()));
        let mut trace = Vec::new();
        for line in &lines {
            for message in codec.parse_text_line(line).unwrap() {
                let message = message.unwrap();
                let message = if enrich {
                    codec.enrich_message(message).unwrap()
                } else {
                    message
                };
                let entries = message.entries().to_vec();
                let digest = message.digest();
                let wire = message.into_bytes(b'|');
                let stamped = life.fill(message).unwrap();
                assert_eq!(stamped.entries(), entries);
                assert_eq!(stamped.digest(), digest);
                assert_eq!(stamped.into_bytes(b'|'), wire);
                trace.push((stamped, life.alive()));
            }
        }
        assert_eq!(trace.len(), 83);
        assert_eq!(life.alive(), expected_alive);
        life.clear();
        for (message, alive) in &trace {
            assert_eq!(life.fill(message.clone()).unwrap(), *message);
            assert_eq!(life.alive(), *alive);
        }

        let outgoing: Vec<_> = trace
            .iter()
            .map(|(message, _)| Ok(message.clone()))
            .collect();
        let reader = codec.arrow_reader(schema.clone(), outgoing).unwrap();
        let arrow_schema = reader.schema();
        let batches = reader.collect::<Result<Vec<_>, _>>().unwrap();
        for boundary in [1, 2, 7] {
            let slices: Vec<_> = batches
                .iter()
                .flat_map(|batch| {
                    (0..batch.num_rows()).step_by(boundary).map(move |start| {
                        batch.slice(start, boundary.min(batch.num_rows() - start))
                    })
                })
                .collect();
            assert!(slices.iter().all(|batch| batch.num_rows() <= boundary));
            assert_eq!(
                slices.iter().map(|batch| batch.num_rows()).sum::<usize>(),
                83
            );
            let restored = codec
                .messages(arrow::batch_reader(Arc::clone(&arrow_schema), slices))
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(restored.len(), trace.len());
            let mut replay = FixLifecycle::new(Arc::clone(codec.registry()));
            for (message, (original, alive)) in restored.into_iter().zip(&trace) {
                let expected_row = original.into_row(&schema).unwrap();
                assert_eq!(message.into_row(&schema).unwrap(), expected_row);
                for tag in [PREVTIMESTAMP_TAG_NAME.0, PREVUUID_TAG_NAME.0] {
                    assert_eq!(message.by_tag(tag).unwrap(), original.by_tag(tag).unwrap());
                }
                assert_eq!(message.entries(), original.entries());
                assert_eq!(message.digest(), original.digest());
                assert_eq!(message.into_bytes(b'|'), original.into_bytes(b'|'));
                let replayed = replay.fill(message.clone()).unwrap();
                // The fixed row is a projection, so full equality is against
                // that restored message, not the original dynamic schema.
                assert_eq!(replayed, message);
                assert_eq!(replayed.into_row(&schema).unwrap(), expected_row);
                assert_eq!(replayed.entries(), original.entries());
                assert_eq!(replayed.digest(), original.digest());
                assert_eq!(replayed.into_bytes(b'|'), original.into_bytes(b'|'));
                assert_eq!(replay.alive(), *alive);
            }
            assert_eq!(replay.alive(), expected_alive);
        }
    }
}
