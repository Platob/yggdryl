//! The last finalized full message owns each live chain's previous pair.

use std::sync::Arc;

use super::SoleMessage;
use super::lifecycle_chains::{clock, row, try_row};
use super::{identity_scalar, numbered_identity};
use yggdryl::{
    ALTIDS_TAG_NAME, CODE_TAG_NAME, Error, FixLifecycle, FixMsg, FixRegistry, INSTUUID_TAG_NAME,
    PREVMSGHASH_TAG_NAME, PREVUPDATEDAT_TAG_NAME, SNAPSHOTAT_TAG_NAME, Scalar, TimeUnit, Timezone,
    UPDATEDAT_TAG_NAME, arrow, fix_schema,
};

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
    code: Option<&str>,
    scope: Option<u128>,
    entries: &[(&str, &str)],
) -> FixMsg {
    let mut cells = vec![(UPDATEDAT_TAG_NAME.0, clock(nanos)), identifiers(entries)];
    cells.extend(code.map(|value| (CODE_TAG_NAME.0, Scalar::from(value))));
    cells.extend(scope.map(|value| {
        (
            INSTUUID_TAG_NAME.0,
            identity_scalar(numbered_identity(value)),
        )
    }));
    row(registry, cells)
}

fn lifecycle(registry: &Arc<FixRegistry>) -> FixLifecycle {
    FixLifecycle::new(Arc::clone(registry))
        .try_with_interval_ns(1)
        .unwrap()
}

fn assert_pair(message: &FixMsg, expected: Option<&FixMsg>) {
    assert_eq!(
        message.by_tag(PREVUPDATEDAT_TAG_NAME.0).unwrap(),
        expected.map_or(&Scalar::Null, FixMsg::updatedat)
    );
    assert_eq!(
        message.by_tag(PREVMSGHASH_TAG_NAME.0).unwrap(),
        expected.map_or(&Scalar::Null, FixMsg::msghash)
    );
}

fn located(error: Error, expected: &str) {
    let Error::InvalidRecord { path, reason } = error else {
        panic!("expected a located refusal, got {error}");
    };
    assert_eq!(path, expected);
    assert!(reason.len() < 256, "{reason}");
}

#[test]
fn three_messages_follow_arrival_order_without_sorting_their_clocks() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = lifecycle(&registry);
    let mut previous = None;
    for time in [30, 10, 20] {
        let message = life
            .fill(event(&registry, time, Some("A"), None, &[("id", "A")]))
            .unwrap();
        assert_pair(&message, previous.as_ref());
        assert_eq!(message.updatedat(), &clock(time));
        if let Some(previous) = previous.as_ref() {
            assert_eq!(message.msgphash(), previous.msgphash());
        }
        previous = Some(message);
    }
    assert_eq!(life.alive(), 1);
}

#[test]
fn previous_fields_are_independent_statements_not_the_stored_current_pair() {
    let registry = Arc::new(FixRegistry::new());
    for (stated_time, stated_msghash) in [
        (None, None),
        (Some(900), None),
        (None, Some(901)),
        (Some(900), Some(901)),
    ] {
        let mut life = lifecycle(&registry);
        let mut first = event(&registry, 10, Some("A"), None, &[]);
        first
            .set_many([
                (PREVUPDATEDAT_TAG_NAME.0, clock(800)),
                (
                    PREVMSGHASH_TAG_NAME.0,
                    identity_scalar(numbered_identity(801)),
                ),
            ])
            .unwrap();
        let first = life.fill(first).unwrap();
        assert_eq!(first.by_tag(PREVUPDATEDAT_TAG_NAME.0).unwrap(), &clock(800));
        assert_eq!(
            first.by_tag(PREVMSGHASH_TAG_NAME.0).unwrap(),
            &identity_scalar(numbered_identity(801))
        );
        let mut second = event(&registry, 20, Some("A"), None, &[]);
        second
            .set_many([
                (
                    PREVUPDATEDAT_TAG_NAME.0,
                    stated_time.map_or(Scalar::Null, clock),
                ),
                (
                    PREVMSGHASH_TAG_NAME.0,
                    stated_msghash
                        .map_or(Scalar::Null, |id| identity_scalar(numbered_identity(id))),
                ),
            ])
            .unwrap();
        let second = life.fill(second).unwrap();
        let expected_clock = stated_time.map_or_else(|| first.updatedat().clone(), clock);
        let expected_msghash = stated_msghash.map_or_else(
            || first.msghash().clone(),
            |id| identity_scalar(numbered_identity(id)),
        );
        assert_eq!(
            second.by_tag(PREVUPDATEDAT_TAG_NAME.0).unwrap(),
            &expected_clock
        );
        assert_eq!(
            second.by_tag(PREVMSGHASH_TAG_NAME.0).unwrap(),
            &expected_msghash
        );
        let third = life
            .fill(event(&registry, 30, Some("A"), None, &[]))
            .unwrap();
        assert_pair(&third, Some(&second));
    }
}

#[test]
fn absent_nil_and_distinct_scopes_keep_separate_previous_pairs() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = lifecycle(&registry);
    let mut firsts = Vec::new();
    for (scope, time) in [(None, 10), (Some(0), 20), (Some(1), 30), (Some(2), 40)] {
        let first = life
            .fill(event(&registry, time, None, scope, &[("id", "SAME")]))
            .unwrap();
        assert_pair(&first, None);
        firsts.push((scope, time, first));
    }
    for (scope, time, first) in firsts {
        let next = life
            .fill(event(&registry, time + 100, None, scope, &[("id", "SAME")]))
            .unwrap();
        assert_eq!(next.msgphash(), first.msgphash());
        assert_pair(&next, Some(&first));
    }
    assert_eq!(life.alive(), 4);
}

#[test]
fn direct_code_and_scoped_identifier_join_read_only_the_selected_chains_history() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = lifecycle(&registry);
    let a = life
        .fill(event(&registry, 10, Some("A"), Some(1), &[("id", "A")]))
        .unwrap();
    let b = life
        .fill(event(&registry, 20, Some("B"), Some(2), &[("id", "B")]))
        .unwrap();
    let direct = life
        .fill(event(
            &registry,
            30,
            Some("A"),
            Some(2),
            &[("a", "B"), ("b", "ATTACHED")],
        ))
        .unwrap();
    assert_pair(&direct, Some(&a));
    let other = life
        .fill(event(&registry, 40, None, Some(2), &[("id", "B")]))
        .unwrap();
    assert_eq!(other.msgphash(), b.msgphash());
    assert_pair(&other, Some(&b));
    let attached = life
        .fill(event(&registry, 50, None, Some(2), &[("id", "ATTACHED")]))
        .unwrap();
    assert_eq!(attached.msgphash(), a.msgphash());
    assert_pair(&attached, Some(&direct));
    assert_eq!(life.alive(), 2);
}

#[test]
fn previous_timestamp_is_the_normalized_update_not_the_real_event_clock() {
    let registry = super::committed_registry();
    let codec = super::fixed_codec(Arc::clone(&registry));
    let mut first = codec
        .sole_line(
            b"8=FIX.4.4|35=D|11=EVENT-1|60=19700101-00:00:01.000002|10=0|",
            false,
        )
        .unwrap();
    first
        .set(UPDATEDAT_TAG_NAME.0, clock(5_000_000_123))
        .unwrap();
    let mut life = FixLifecycle::new(registry)
        .try_with_interval_ns(10)
        .unwrap();
    let first = life.fill(first).unwrap();
    assert_pair(&first, None);
    assert_eq!(first.updatedat(), &clock(5_000_000_120));
    assert_eq!(first.createdat(), &clock(1_000_002_000));
    assert!(first.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap().is_null());
    let mut second = codec
        .sole_line(
            b"8=FIX.4.4|35=D|11=EVENT-1|60=19700101-00:00:02.000004|10=0|",
            false,
        )
        .unwrap();
    second
        .set(UPDATEDAT_TAG_NAME.0, clock(6_000_000_789))
        .unwrap();
    let second = life.fill(second).unwrap();
    assert_pair(&second, Some(&first));
    assert_eq!(second.updatedat(), &clock(6_000_000_780));
    // The event clock stays where the message stated it; `createdat` belongs
    // to the chain's first incarnation, and no snapshot was taken of either.
    assert_eq!(second.by_tag(60).unwrap(), &clock(2_000_004_000));
    assert_eq!(second.createdat(), &clock(1_000_002_000));
    assert!(second.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap().is_null());
}

#[test]
fn missing_or_null_updatedat_uses_the_event_clock_settled_at_intake() {
    let registry = Arc::new(FixRegistry::new());
    for (updated, event_time, expected) in [
        (None, None, 0),
        (Some(Scalar::Null), None, 0),
        (Some(clock(17_000)), Some(clock(19_000_000)), 17_000),
        (Some(Scalar::Null), Some(clock(19_000_000)), 19_000_000),
        (None, Some(clock(19_000_000)), 19_000_000),
    ] {
        let mut cells = vec![(CODE_TAG_NAME.0, Scalar::from("A"))];
        cells.extend(updated.map(|value| (UPDATEDAT_TAG_NAME.0, value)));
        cells.extend(event_time.map(|value| (60, value)));
        let mut life = lifecycle(&registry);
        let first = life.fill(row(&registry, cells)).unwrap();
        assert_pair(&first, None);
        assert_eq!(first.updatedat(), &clock(expected));
        let next = life
            .fill(event(&registry, 20_000_000, Some("A"), None, &[]))
            .unwrap();
        assert_pair(&next, Some(&first));
    }
}

#[test]
fn terminal_clear_and_reopening_forget_every_scopes_previous_pair() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = lifecycle(&registry);
    let first = life
        .fill(event(&registry, 10, Some("A"), Some(1), &[("id", "A")]))
        .unwrap();
    let attached = life
        .fill(event(&registry, 20, Some("A"), Some(2), &[("id", "B")]))
        .unwrap();
    assert_pair(&attached, Some(&first));
    let mut terminal = event(&registry, 30, None, Some(2), &[("id", "B")]);
    terminal.set(39, Scalar::from("2")).unwrap();
    let terminal = life.fill(terminal).unwrap();
    assert_eq!(terminal.msgphash(), first.msgphash());
    assert_pair(&terminal, Some(&attached));
    assert_eq!(life.alive(), 0);
    let reopened = life
        .fill(event(&registry, 40, Some("A"), Some(1), &[("id", "A")]))
        .unwrap();
    assert_pair(&reopened, None);
    let attached = life
        .fill(event(&registry, 50, Some("A"), Some(2), &[("id", "B")]))
        .unwrap();
    assert_pair(&attached, Some(&reopened));
    life.clear();
    assert_eq!(life.alive(), 0);
    let fresh = life
        .fill(event(&registry, 60, Some("A"), Some(1), &[("id", "A")]))
        .unwrap();
    assert_pair(&fresh, None);
    let mut first_terminal = event(&registry, 70, Some("B"), None, &[("id", "C")]);
    first_terminal.set(39, Scalar::from("2")).unwrap();
    assert_pair(&life.fill(first_terminal).unwrap(), None);
    assert_eq!(life.alive(), 1);
    let reopened = life
        .fill(event(&registry, 80, Some("B"), None, &[("id", "C")]))
        .unwrap();
    assert_pair(&reopened, None);
    assert_eq!(life.alive(), 2);
}

#[test]
fn an_earlier_message_into_advanced_state_is_an_arrival_not_a_rewind() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = lifecycle(&registry);
    let first = life
        .fill(event(&registry, 10, Some("A"), None, &[]))
        .unwrap();
    life.fill(event(&registry, 20, Some("A"), None, &[]))
        .unwrap();
    let last = life
        .fill(event(&registry, 30, Some("A"), None, &[]))
        .unwrap();
    let arrived_again = life.fill(first).unwrap();
    assert_pair(&arrived_again, Some(&last));
    let next = life
        .fill(event(&registry, 40, Some("A"), None, &[]))
        .unwrap();
    assert_pair(&next, Some(&arrived_again));
}

#[test]
fn malformed_stated_previous_values_neither_advance_attach_nor_close() {
    let registry = Arc::new(FixRegistry::new());
    let invalid = [
        (PREVUPDATEDAT_TAG_NAME, Scalar::from("界".repeat(128))),
        (PREVUPDATEDAT_TAG_NAME, Scalar::date32(0)),
        (
            PREVUPDATEDAT_TAG_NAME,
            Scalar::datetime64(0, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
        ),
        (
            PREVUPDATEDAT_TAG_NAME,
            Scalar::datetime64(0, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
        ),
        (
            PREVMSGHASH_TAG_NAME,
            Scalar::from("00000000-0000-0000-0000-000000000001"),
        ),
        (PREVMSGHASH_TAG_NAME, Scalar::from(vec![0_u8; 16])),
        (PREVMSGHASH_TAG_NAME, Scalar::from(1_i64)),
        (PREVMSGHASH_TAG_NAME, clock(0)),
    ];
    for ((tag, name), value) in invalid {
        for terminal in [false, true] {
            let mut life = lifecycle(&registry);
            let first = life
                .fill(event(&registry, 10, Some("A"), None, &[("id", "LIVE")]))
                .unwrap();
            let mut cells = vec![
                (UPDATEDAT_TAG_NAME.0, clock(20)),
                (CODE_TAG_NAME.0, Scalar::from("A")),
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
                .fill(event(&registry, 30, None, None, &[("id", "LIVE")]))
                .unwrap();
            assert_pair(&retained, Some(&first));
            assert_eq!(retained.msgphash(), first.msgphash());
            let independent = life
                .fill(event(&registry, 40, None, None, &[("id", "NEW")]))
                .unwrap();
            assert_pair(&independent, None);
            assert_ne!(independent.msgphash(), first.msgphash());
            assert_eq!(life.alive(), 2);
        }
    }
}

#[test]
fn an_unrepresentable_history_clock_refuses_at_intake_before_opening_or_advancing() {
    let registry = Arc::new(FixRegistry::new());
    let micros = i64::MAX / 1_000 + 1;
    for existing in [false, true] {
        let mut life = lifecycle(&registry);
        let first = existing.then(|| {
            life.fill(event(&registry, 10, Some("A"), None, &[("id", "LIVE")]))
                .unwrap()
        });
        let refused = try_row(
            &registry,
            [
                (
                    UPDATEDAT_TAG_NAME.0,
                    Scalar::datetime64(micros, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
                ),
                (CODE_TAG_NAME.0, Scalar::from("A")),
                identifiers(&[("a", "LIVE"), ("b", "NEW")]),
            ],
        );
        located(refused.unwrap_err(), "$.updatedat");
        assert_eq!(life.alive(), usize::from(existing));
        let retained = life
            .fill(event(&registry, 30, Some("A"), None, &[("id", "LIVE")]))
            .unwrap();
        assert_pair(&retained, first.as_ref());
        let independent = life
            .fill(event(&registry, 40, None, None, &[("id", "NEW")]))
            .unwrap();
        assert_pair(&independent, None);
        assert_ne!(independent.msgphash(), retained.msgphash());
    }
}

#[test]
fn orphan_and_terminal_messages_keep_full_signed_nanosecond_range_without_history() {
    let registry = Arc::new(FixRegistry::new());
    for instant in [i64::MIN, -1, i64::MAX] {
        for (terminal, existing) in [(false, false), (true, false), (true, true)] {
            let mut life = lifecycle(&registry);
            let first = existing.then(|| {
                life.fill(event(&registry, 10, Some("A"), None, &[("id", "LIVE")]))
                    .unwrap()
            });
            let mut message = event(&registry, instant, terminal.then_some("A"), None, &[]);
            if terminal {
                message.set(39, Scalar::from("2")).unwrap();
            }
            let message = life.fill(message).unwrap();
            assert_eq!(message.updatedat(), &clock(instant));
            assert_pair(&message, first.as_ref());
            assert_eq!(life.alive(), 0);
        }
    }
}

#[test]
fn both_capture_doors_replay_previous_pairs_through_arrow_at_every_row_boundary() {
    let codec = super::dataset::codec();
    let lines = super::dataset::text_lines();
    assert_eq!(lines.len(), 144);
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    // The lifecycle suite says why each door ends on its count: four and
    // three of the first 129 lines' chains, the one the cancel request opens
    // that its instrument-less reject never meets, and the three the bridge
    // named itself where the dictionary declared no identifier.
    for (enrich, expected_alive) in [(false, 8), (true, 7)] {
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
        assert_eq!(trace.len(), 95);
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
                95
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
                for tag in [PREVUPDATEDAT_TAG_NAME.0, PREVMSGHASH_TAG_NAME.0] {
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
