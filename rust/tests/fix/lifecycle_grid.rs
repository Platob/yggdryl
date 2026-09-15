//! Decisions 26–27: one normalized transition and first-created live incarnations.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use super::{identity_scalar, identity_text, numbered_identity, persistent_identity};
use yggdryl::{
    ALTIDS_TAG_NAME, CODE_TAG_NAME, CREATEDAT_TAG_NAME, DataType, Error, FixCodec, FixLifecycle,
    FixMsg, FixRegistry, INSTUUID_TAG_NAME, PREVMSGHASH_TAG_NAME, PREVUPDATEDAT_TAG_NAME,
    SNAPSHOTAT_TAG_NAME, STATE_TAG_NAME, Scalar, TimeUnit, Timezone, UPDATEDAT_TAG_NAME,
};

fn clock(nanos: i64) -> Scalar {
    Scalar::datetime64(nanos, TimeUnit::Nanosecond, Timezone::UTC).unwrap()
}

fn event(
    registry: &Arc<FixRegistry>,
    nanos: i64,
    code: &str,
    scope: Option<[u8; 16]>,
    identifiers: &[(&str, &str)],
) -> FixMsg {
    let mut fields = [
        52,
        UPDATEDAT_TAG_NAME.0,
        CREATEDAT_TAG_NAME.0,
        SNAPSHOTAT_TAG_NAME.0,
        CODE_TAG_NAME.0,
    ]
    .map(|tag| registry.get_field_by_tag(tag).unwrap().clone())
    .to_vec();
    fields.push(
        registry
            .get_group_by_tag(ALTIDS_TAG_NAME.0)
            .unwrap()
            .clone(),
    );
    let mut values = vec![
        clock(nanos),
        clock(nanos),
        clock(nanos),
        clock(nanos),
        Scalar::from(code),
        Scalar::from_mapping(
            identifiers
                .iter()
                .map(|(name, value)| (Scalar::from(*name), Scalar::from(*value))),
        )
        .unwrap(),
    ];
    if let Some(scope) = scope {
        fields.push(
            registry
                .get_field_by_tag(INSTUUID_TAG_NAME.0)
                .unwrap()
                .clone(),
        );
        values.push(identity_scalar(scope));
    }
    FixMsg::with_registry(
        Arc::clone(registry),
        DataType::from_fields(fields)
            .unwrap()
            .required_field("event"),
        Scalar::from_sequence(values),
    )
    .unwrap()
}

fn lifecycle(registry: &Arc<FixRegistry>) -> FixLifecycle {
    FixLifecycle::new(Arc::clone(registry))
        .try_with_interval_ns(10)
        .unwrap()
}

fn previous(message: &FixMsg, expected: Option<&FixMsg>) {
    assert_eq!(
        message.by_tag(PREVUPDATEDAT_TAG_NAME.0).unwrap(),
        expected.map_or(&Scalar::Null, FixMsg::updatedat),
    );
    assert_eq!(
        message.by_tag(PREVMSGHASH_TAG_NAME.0).unwrap(),
        expected.map_or(&Scalar::Null, FixMsg::msghash),
    );
}

fn located(error: Error, expected: &str) {
    let Error::InvalidRecord { path, reason } = error else {
        panic!("expected a located refusal, got {error}");
    };
    assert_eq!(path, expected);
    assert!(reason.len() < 256, "{reason}");
}

fn terminal(message: FixMsg) -> FixMsg {
    message
        .with_value(STATE_TAG_NAME.0, Scalar::from("Filled"))
        .unwrap()
}

#[test]
fn cadence_is_positive_atomic_and_retained_by_clear() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    assert_eq!(FixLifecycle::DEFAULT_INTERVAL_NS, 1_000_000_000);
    assert_eq!(life.interval_ns(), FixLifecycle::DEFAULT_INTERVAL_NS);
    for invalid in [0, -1, i64::MIN] {
        located(life.set_interval_ns(invalid).unwrap_err(), "$.interval_ns");
        assert_eq!(life.interval_ns(), FixLifecycle::DEFAULT_INTERVAL_NS);
        assert_eq!(life.alive(), 0);
    }
    life.set_interval_ns(10).unwrap();
    life.fill(event(&registry, 1, "A", None, &[])).unwrap();
    life.set_interval_ns(10).unwrap();
    for invalid in [0, -1, 20] {
        located(life.set_interval_ns(invalid).unwrap_err(), "$.interval_ns");
        assert_eq!(life.interval_ns(), 10);
        assert_eq!(life.alive(), 1);
    }
    life.clear();
    assert_eq!(life.interval_ns(), 10);
    assert_eq!(life.alive(), 0);
    life.set_interval_ns(i64::MAX).unwrap();
    assert_eq!(life.interval_ns(), i64::MAX);
    located(
        FixLifecycle::new(registry)
            .try_with_interval_ns(0)
            .unwrap_err(),
        "$.interval_ns",
    );
}

#[test]
fn epoch_floor_uses_negative_buckets_and_boundary_belongs_to_the_bucket_it_opens() {
    let registry = Arc::new(FixRegistry::new());
    for (time, interval, grid) in [
        (-11, 10, -20),
        (-10, 10, -10),
        (-1, 10, -10),
        (0, 10, 0),
        (9, 10, 0),
        (10, 10, 10),
        (11, 10, 10),
        (i64::MIN, 1, i64::MIN),
        (i64::MAX, 1, i64::MAX),
        (i64::MAX, i64::MAX, i64::MAX),
    ] {
        let make = || {
            FixLifecycle::new(Arc::clone(&registry))
                .try_with_interval_ns(interval)
                .unwrap()
        };
        let raw = event(&registry, time, "A", None, &[]);
        let filled = make().fill(raw.clone()).unwrap();
        assert_eq!(filled.updatedat(), &clock(grid));
        assert_eq!(filled.createdat(), &clock(time));
        assert_eq!(filled.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap(), &clock(time));
        assert_eq!(
            make().snapshot(raw).unwrap(),
            (time != grid).then_some(filled),
        );
    }
}

#[test]
fn creation_is_the_first_arrivals_statement_not_the_minimum_or_grid() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = lifecycle(&registry);
    let mut other_creation = lifecycle(&registry);
    let mut last = None;
    for (time, stated_creation) in [(21, 987), (1, -123), (31, 432)] {
        let raw = event(&registry, time, "A", None, &[])
            .with_value(CREATEDAT_TAG_NAME.0, clock(stated_creation))
            .unwrap();
        let comparison = other_creation
            .fill(
                raw.clone()
                    .with_value(CREATEDAT_TAG_NAME.0, clock(654))
                    .unwrap(),
            )
            .unwrap();
        let filled = life.fill(raw).unwrap();
        assert_eq!(filled.createdat(), &clock(987));
        assert_eq!(comparison.createdat(), &clock(654));
        assert_eq!(filled.updatedat(), &clock(time.div_euclid(10) * 10));
        assert_eq!(filled.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap(), &clock(time));
        assert_eq!(filled.msghash(), comparison.msghash());
        assert_eq!(filled.msgphash(), comparison.msgphash());
        previous(&filled, last.as_ref());
        last = Some(filled);
    }
}

#[test]
fn full_and_filtered_doors_share_finalized_history_and_consume_aligned_buckets() {
    let registry = Arc::new(FixRegistry::new());
    let mut full = lifecycle(&registry);
    let mut filtered = lifecycle(&registry);
    let mut last = None;
    for (time, grid, emit) in [
        (1, 0, true),
        (7, 0, false),
        (10, 10, false),
        (11, 10, false),
        (21, 20, true),
    ] {
        let raw = event(&registry, time, "A", None, &[]);
        let filled = full.fill(raw.clone()).unwrap();
        previous(&filled, last.as_ref());
        assert_eq!(filled.updatedat(), &clock(grid));
        assert_eq!(filled.createdat(), &clock(1));
        assert_eq!(filled.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap(), &clock(time));
        assert_eq!(
            filtered.snapshot(raw).unwrap(),
            emit.then(|| filled.clone())
        );
        last = Some(filled);
    }
    assert_eq!(full.alive(), 1);
    assert_eq!(filtered.alive(), 1);

    let mut aligned = lifecycle(&registry);
    assert!(
        aligned
            .snapshot(
                event(&registry, 10, "B", None, &[])
                    .with_value(CREATEDAT_TAG_NAME.0, clock(77))
                    .unwrap(),
            )
            .unwrap()
            .is_none()
    );
    assert!(
        aligned
            .snapshot(event(&registry, 19, "B", None, &[]))
            .unwrap()
            .is_none()
    );
    let after_aligned = aligned
        .snapshot(event(&registry, 21, "B", None, &[]))
        .unwrap()
        .unwrap();
    assert_eq!(after_aligned.createdat(), &clock(77));
}

#[test]
fn explicit_codes_are_global_and_never_steal_scoped_identifier_ownership() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = lifecycle(&registry);
    let scope = numbered_identity(1);
    let first = life
        .fill(event(&registry, 1, "A", Some(scope), &[("id", "OWNED")]))
        .unwrap();
    let other = life
        .fill(event(&registry, 2, "B", Some(scope), &[("id", "OWNED")]))
        .unwrap();
    assert_ne!(first.msgphash(), other.msgphash());
    assert_eq!(first.createdat(), &clock(1));
    assert_eq!(other.createdat(), &clock(2));
    previous(&other, None);
    assert_eq!(life.alive(), 2);
    let alias = life
        .fill(event(&registry, 11, "", Some(scope), &[("id", "OWNED")]))
        .unwrap();
    assert_eq!(alias.by_tag(CODE_TAG_NAME.0).unwrap().as_str(), Some("A"));
    previous(&alias, Some(&first));
    assert_eq!(alias.createdat(), first.createdat());

    let direct = life
        .fill(event(
            &registry,
            21,
            "A",
            Some(numbered_identity(2)),
            &[("id", "NEW")],
        ))
        .unwrap();
    previous(&direct, Some(&alias));
    assert_eq!(direct.msgphash(), first.msgphash());
    assert_eq!(direct.createdat(), first.createdat());
    let attached = life
        .fill(event(
            &registry,
            31,
            "",
            Some(numbered_identity(2)),
            &[("id", "NEW")],
        ))
        .unwrap();
    previous(&attached, Some(&direct));
    assert_eq!(attached.createdat(), first.createdat());
    assert_eq!(life.alive(), 2);

    // When two aliases reach different chains, canonical name order wins.
    let b = life
        .fill(event(&registry, 41, "B", Some(scope), &[("id", "OTHER")]))
        .unwrap();
    let joined = life
        .fill(event(
            &registry,
            51,
            "",
            Some(scope),
            &[("a", "OTHER"), ("z", "OWNED")],
        ))
        .unwrap();
    previous(&joined, Some(&b));
    assert_eq!(joined.msgphash(), other.msgphash());
    assert_eq!(b.createdat(), other.createdat());
    assert_eq!(joined.createdat(), other.createdat());
    let still_a = life
        .fill(event(&registry, 61, "", Some(scope), &[("id", "OWNED")]))
        .unwrap();
    previous(&still_a, Some(&attached));
    assert_eq!(still_a.createdat(), first.createdat());
}

#[test]
fn derived_codes_preserve_scope_identifier_text_and_empty_is_not_whitespace() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = lifecycle(&registry);
    let text = "Mixed/Case/界";
    let mut ids = Vec::new();
    let scopes = [None, Some(numbered_identity(0)), Some(numbered_identity(1))];
    for (time, scope) in (1..).zip(scopes) {
        let expected = scope.map_or_else(
            || format!("-/{text}"),
            |id| format!("{}/{text}", identity_text(&id)),
        );
        let value = life
            .fill(event(&registry, time, "", scope, &[("id", text)]))
            .unwrap();
        assert_eq!(
            value.by_tag(CODE_TAG_NAME.0).unwrap().as_str(),
            Some(expected.as_str())
        );
        assert_eq!(
            value.msgphash(),
            &identity_scalar(persistent_identity(&expected))
        );
        previous(&value, None);
        assert_eq!(value.createdat(), &clock(time));
        assert!(!ids.contains(value.msgphash()));
        ids.push(value.msgphash().clone());
    }
    assert_eq!(life.alive(), 3);
    for ((creation, scope), persistent) in (1..).zip(scopes).zip(&ids) {
        let joined = life
            .fill(event(&registry, 31, "", scope, &[("id", text)]))
            .unwrap();
        assert_eq!(joined.createdat(), &clock(creation));
        assert_eq!(joined.msgphash(), persistent);
    }
    let unknown = event(&registry, 1, "", None, &[]);
    let filled = life.fill(unknown.clone()).unwrap();
    assert_eq!(filled.msgphash(), &identity_scalar(persistent_identity("")));
    assert_eq!(filled.updatedat(), &clock(0));
    assert_eq!(filled.createdat(), unknown.createdat());
    previous(&filled, None);
    assert!(life.snapshot(unknown).unwrap().is_none());
    let next_unnamed = life.fill(event(&registry, 2, "", None, &[])).unwrap();
    assert_eq!(next_unnamed.createdat(), &clock(2));
    previous(&next_unnamed, None);
    assert_eq!(life.alive(), 3);
    let whitespace = life
        .snapshot(event(&registry, 1, " ", None, &[]))
        .unwrap()
        .unwrap();
    assert_eq!(
        whitespace.by_tag(CODE_TAG_NAME.0).unwrap().as_str(),
        Some(" ")
    );
    assert_eq!(life.alive(), 4);
}

#[test]
fn late_messages_advance_previous_history_without_lowering_the_bucket_high_water() {
    let registry = Arc::new(FixRegistry::new());
    let mut full = lifecycle(&registry);
    let mut filtered = lifecycle(&registry);
    let mut last = None;
    for (time, emit) in [(21, true), (1, false), (29, false), (31, true)] {
        let raw = event(&registry, time, "A", None, &[]);
        let filled = full.fill(raw.clone()).unwrap();
        previous(&filled, last.as_ref());
        assert_eq!(filled.createdat(), &clock(21));
        assert_eq!(filled.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap(), &clock(time));
        assert_eq!(
            filtered.snapshot(raw).unwrap(),
            emit.then(|| filled.clone())
        );
        last = Some(filled);
    }
}

#[test]
fn suppressed_terminal_closes_and_same_bucket_reopening_starts_fresh() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = lifecycle(&registry);
    let mut full = lifecycle(&registry);
    let raw = event(&registry, 1, "A", None, &[("id", "OLD")]);
    let first = life.snapshot(raw.clone()).unwrap().unwrap();
    assert_eq!(full.fill(raw).unwrap(), first);
    let ending = terminal(event(&registry, 2, "A", None, &[]));
    let closed = full.fill(ending.clone()).unwrap();
    assert_eq!(closed.createdat(), first.createdat());
    previous(&closed, Some(&first));
    assert_eq!(full.alive(), 0);
    assert!(life.snapshot(ending).unwrap().is_none());
    assert_eq!(life.alive(), 0);
    let reopened = life
        .snapshot(event(&registry, 3, "A", None, &[]))
        .unwrap()
        .unwrap();
    previous(&reopened, None);
    assert_eq!(reopened.msgphash(), first.msgphash());
    assert_eq!(reopened.createdat(), &clock(3));
    let old_key = life
        .fill(event(&registry, 4, "", None, &[("id", "OLD")]))
        .unwrap();
    assert_ne!(old_key.msgphash(), reopened.msgphash());
    previous(&old_key, None);
    assert_eq!(old_key.createdat(), &clock(4));
    assert_eq!(life.alive(), 2);
    life.clear();
    assert_eq!(life.interval_ns(), 10);
    for time in [5, 6] {
        let standalone = life
            .snapshot(terminal(event(&registry, time, "A", None, &[])))
            .unwrap()
            .unwrap();
        previous(&standalone, None);
        assert_eq!(standalone.createdat(), &clock(time));
        assert_eq!(life.alive(), 0, "no closed-chain tombstones");
    }
    assert!(
        life.snapshot(terminal(event(&registry, 10, "A", None, &[])))
            .unwrap()
            .is_none()
    );
    assert_eq!(life.alive(), 0);
}

#[test]
fn grid_underflow_refuses_before_opening_attaching_advancing_or_closing() {
    let registry = Arc::new(FixRegistry::new());
    for existing in [false, true] {
        let mut life = lifecycle(&registry);
        let first = existing.then(|| {
            life.fill(event(&registry, 1, "A", None, &[("id", "LIVE")]))
                .unwrap()
        });
        let failed = terminal(event(&registry, i64::MIN, "A", None, &[("id", "NEW")]));
        located(life.snapshot(failed).unwrap_err(), "$.updatedat");
        assert_eq!(life.alive(), usize::from(existing));
        let accepted = life
            .snapshot(event(&registry, 11, "A", None, &[]))
            .unwrap()
            .unwrap();
        previous(&accepted, first.as_ref());
        assert_eq!(accepted.createdat(), &clock(if existing { 1 } else { 11 }));
        let free = life
            .fill(event(&registry, 12, "", None, &[("id", "NEW")]))
            .unwrap();
        previous(&free, None);
        assert_eq!(free.createdat(), &clock(12));
        assert_ne!(free.msgphash(), accepted.msgphash());
    }
}

fn wrong_previous_registry(base: &FixRegistry, tag: i32, dtype: DataType) -> Arc<FixRegistry> {
    let mut registry = base.clone();
    let mut field = registry.remove(tag).unwrap();
    field.set_dtype(dtype).unwrap();
    field.set_nullable(true);
    registry.insert(field).unwrap();
    Arc::new(registry)
}

#[test]
fn native_previous_target_failures_cannot_consume_a_bucket_or_close_and_attach() {
    let registry = Arc::new(FixRegistry::new());
    for ((tag, name), dtype) in [
        (PREVMSGHASH_TAG_NAME, DataType::utf8()),
        (PREVMSGHASH_TAG_NAME, DataType::binary()),
        (PREVMSGHASH_TAG_NAME, clock(0).dtype().unwrap()),
        (PREVUPDATEDAT_TAG_NAME, super::identity_dtype()),
        (PREVUPDATEDAT_TAG_NAME, DataType::utf8()),
    ] {
        let custom = wrong_previous_registry(&registry, tag, dtype);
        for existing in [false, true] {
            let mut life = lifecycle(&registry);
            let first = existing.then(|| {
                life.fill(event(&registry, 1, "A", None, &[("id", "LIVE")]))
                    .unwrap()
            });
            let failed = terminal(event(
                &custom,
                11,
                "A",
                None,
                &[("a", "LIVE"), ("b", "NEW")],
            ))
            .with_value(tag, Scalar::Null)
            .unwrap();
            located(life.snapshot(failed).unwrap_err(), &format!("$.{name}"));
            assert_eq!(life.alive(), usize::from(existing));
            let accepted = life
                .snapshot(event(&registry, 12, "A", None, &[]))
                .unwrap()
                .unwrap();
            previous(&accepted, first.as_ref());
            assert_eq!(accepted.createdat(), &clock(if existing { 1 } else { 12 }));
            let free = life
                .fill(event(&registry, 13, "", None, &[("id", "NEW")]))
                .unwrap();
            previous(&free, None);
            assert_eq!(free.createdat(), &clock(13));
            assert_ne!(free.msgphash(), accepted.msgphash());
        }
    }
}

#[test]
fn independently_stated_previous_values_are_not_used_as_current_history() {
    let registry = Arc::new(FixRegistry::new());
    for (stated_clock, stated_msghash) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let mut life = lifecycle(&registry);
        let first = life.fill(event(&registry, 1, "A", None, &[])).unwrap();
        let mut second = event(&registry, 11, "A", None, &[]);
        second
            .set_many([
                (
                    PREVUPDATEDAT_TAG_NAME.0,
                    if stated_clock {
                        clock(987)
                    } else {
                        Scalar::Null
                    },
                ),
                (
                    PREVMSGHASH_TAG_NAME.0,
                    if stated_msghash {
                        identity_scalar(numbered_identity(987))
                    } else {
                        Scalar::Null
                    },
                ),
            ])
            .unwrap();
        let second = life.fill(second).unwrap();
        let expected_clock = if stated_clock {
            clock(987)
        } else {
            first.updatedat().clone()
        };
        let expected_msghash = if stated_msghash {
            identity_scalar(numbered_identity(987))
        } else {
            first.msghash().clone()
        };
        assert_eq!(
            second.by_tag(PREVUPDATEDAT_TAG_NAME.0).unwrap(),
            &expected_clock,
        );
        assert_eq!(
            second.by_tag(PREVMSGHASH_TAG_NAME.0).unwrap(),
            &expected_msghash,
        );
        let third = life.fill(event(&registry, 21, "A", None, &[])).unwrap();
        previous(&third, Some(&second));
    }
}

#[test]
fn snapshot_stream_is_lazy_keeps_per_item_errors_and_fuses_only_exhaustion() {
    let registry = Arc::new(FixRegistry::new());
    let custom = wrong_previous_registry(&registry, PREVMSGHASH_TAG_NAME.0, DataType::utf8());
    let bad = event(&custom, 11, "A", None, &[])
        .with_value(PREVMSGHASH_TAG_NAME.0, Scalar::Null)
        .unwrap();
    let mut input = [
        Ok(event(&registry, 1, "A", None, &[])),
        Err(Error::InvalidRecord {
            path: "$.source".into(),
            reason: "source item failed".into(),
        }),
        Ok(bad),
        Ok(event(&registry, 12, "A", None, &[])),
        Ok(event(&registry, 20, "A", None, &[])),
        Ok(event(&registry, 29, "A", None, &[])),
        Ok(event(&registry, 31, "A", None, &[])),
    ]
    .into_iter();
    let pulls = Rc::new(Cell::new(0));
    let count = Rc::clone(&pulls);
    let source = std::iter::from_fn(move || {
        count.set(count.get() + 1);
        input.next()
    });
    let mut snapshots = lifecycle(&registry).snapshots(source);
    assert_eq!(pulls.get(), 0);
    let first = snapshots.next().unwrap().unwrap();
    assert_eq!(pulls.get(), 1);
    located(snapshots.next().unwrap().unwrap_err(), "$.source");
    assert_eq!(pulls.get(), 2);
    located(snapshots.next().unwrap().unwrap_err(), "$.prevmsghash");
    assert_eq!(pulls.get(), 3);
    let recovered = snapshots.next().unwrap().unwrap();
    previous(&recovered, Some(&first));
    assert_eq!(recovered.createdat(), first.createdat());
    assert_eq!(
        pulls.get(),
        4,
        "failed transition did not consume bucket ten"
    );
    assert_eq!(snapshots.next().unwrap().unwrap().updatedat(), &clock(30));
    assert_eq!(
        pulls.get(),
        7,
        "aligned and equal buckets were processed, not emitted"
    );
    assert!(snapshots.next().is_none());
    assert_eq!(pulls.get(), 8);
    assert!(snapshots.next().is_none());
    assert_eq!(pulls.get(), 8);

    let mut resumed = 0;
    let source = std::iter::from_fn(move || {
        resumed += 1;
        (resumed != 1).then(|| Ok(event(&registry, 1, "LATE", None, &[])))
    });
    let mut snapshots = FixLifecycle::new(Arc::new(FixRegistry::new())).snapshots(source);
    assert!(snapshots.next().is_none());
    assert!(
        snapshots.next().is_none(),
        "a source cannot resume after exhaustion"
    );
}

#[test]
fn fresh_replay_is_exact_and_preprocessed_snapshot_replay_emits_nothing() {
    let registry = Arc::new(FixRegistry::new());
    let raw: Vec<_> = [1, 7, 11, 21]
        .map(|time| event(&registry, time, "A", None, &[]))
        .into();
    let fill = |life: &mut FixLifecycle, messages: Vec<FixMsg>| {
        messages
            .into_iter()
            .map(|message| life.fill(message))
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap()
    };
    let mut full = lifecycle(&registry);
    let filled = fill(&mut full, raw.clone());
    assert_eq!(filled.len(), raw.len());
    assert!(
        filled
            .iter()
            .all(|message| message.createdat() == &clock(1))
    );
    let mut fresh = lifecycle(&registry);
    assert_eq!(fill(&mut fresh, raw.clone()), filled);
    let mut fresh = lifecycle(&registry);
    assert_eq!(fill(&mut fresh, filled.clone()), filled);
    assert_eq!(fresh.alive(), full.alive());
    full.clear();
    assert_eq!(fill(&mut full, raw.clone()), filled);
    full.clear();
    assert_eq!(fill(&mut full, filled.clone()), filled);
    let snapshots = || {
        lifecycle(&registry)
            .snapshots(raw.clone().into_iter().map(Ok))
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap()
    };
    let first = snapshots();
    assert_eq!(first.len(), 3);
    assert_eq!(snapshots(), first);
    let mut filtered = lifecycle(&registry);
    let first_pass: Vec<_> = raw
        .iter()
        .cloned()
        .filter_map(|message| filtered.snapshot(message).unwrap())
        .collect();
    assert_eq!(first_pass, first);
    filtered.clear();
    let cleared_pass: Vec<_> = raw
        .into_iter()
        .filter_map(|message| filtered.snapshot(message).unwrap())
        .collect();
    assert_eq!(cleared_pass, first);
    filtered.clear();
    for message in &filled {
        assert!(filtered.snapshot(message.clone()).unwrap().is_none());
    }
    assert_eq!(filtered.alive(), full.alive());
    let next = event(&registry, 31, "A", None, &[]);
    assert_eq!(
        filtered.fill(next.clone()).unwrap(),
        full.fill(next).unwrap()
    );
    assert_eq!(
        lifecycle(&registry)
            .snapshots(filled.into_iter().map(Ok))
            .count(),
        0
    );
}

#[test]
fn snapshot_iterator_crosses_arrow_without_codec_lifetimes_or_a_second_transition() {
    let registry = Arc::new(FixRegistry::new());
    let raw: Vec<_> = [1, 7, 11]
        .map(|time| event(&registry, time, "A", None, &[]))
        .into();
    let schema = raw[0].as_field().clone();
    let codec = FixCodec::new(Arc::clone(&registry));
    let input = codec
        .arrow_reader(schema.clone(), raw.clone().into_iter().map(Ok))
        .unwrap();
    let messages = codec.messages(input);
    let snapshots = lifecycle(&registry).snapshots(messages);
    let reader = codec.arrow_reader(schema, snapshots).unwrap();
    drop(codec);
    let reader_codec = FixCodec::new(Arc::clone(&registry));
    let actual = reader_codec
        .messages(reader)
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(actual.len(), 2);
    assert_eq!(actual[0].updatedat(), &clock(0));
    assert_eq!(actual[1].updatedat(), &clock(10));
    assert!(
        actual
            .iter()
            .all(|message| message.createdat() == &clock(1))
    );
    for message in &actual {
        let row = message.into_row(message.as_field()).unwrap();
        let replayed = FixMsg::from_row(Arc::clone(&registry), message.as_field(), &row).unwrap();
        assert_eq!(replayed, *message);
        assert_eq!(replayed.into_row(message.as_field()).unwrap(), row);
    }
    // Compare against the same emitted row projection: projection itself owns
    // its identity and may omit the newly appended previous-message columns.
    let expected_reader = reader_codec
        .arrow_reader(
            raw[0].as_field().clone(),
            lifecycle(&registry).snapshots(raw.into_iter().map(Ok)),
        )
        .unwrap();
    let expected = reader_codec
        .messages(expected_reader)
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn lifecycle_normalization_keeps_wire_entries_digest_and_real_clock() {
    let registry = Arc::new(FixRegistry::new());
    let codec = FixCodec::new(Arc::clone(&registry));
    let wire = b"8=FIX.4.4|35=D|52=20260102-10:15:30.125|60=20260102-10:15:30.123456789|10=0|";
    let raw = codec
        .parse_fix_line(wire)
        .unwrap()
        .with_value(CODE_TAG_NAME.0, Scalar::from("A"))
        .unwrap();
    let filled = FixLifecycle::new(registry).fill(raw.clone()).unwrap();
    assert_eq!(filled.into_bytes(b'|'), raw.into_bytes(b'|'));
    assert_eq!(filled.entries(), raw.entries());
    assert_eq!(filled.digest(), raw.digest());
    assert_eq!(
        filled.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap(),
        raw.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap()
    );
    assert_eq!(filled.createdat(), raw.createdat());
    assert_ne!(filled.updatedat(), raw.updatedat());
}
