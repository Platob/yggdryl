//! Lifecycle stamps require each resolved target's own native layout.

use std::sync::Arc;

use yggdryl::types::Uuid;
use yggdryl::{
    ALTIDS_TAG_NAME, DataType, Error, Field, FixLifecycle, FixMsg, FixRegistry, INSTUUID_TAG_NAME,
    PREVUPDATEDAT_TAG_NAME, PREVUUID_TAG_NAME, PUUID_TAG_NAME, Scalar, TimeUnit, Timezone,
    UPDATEDAT_TAG_NAME, UUID_TAG_NAME,
};

const IDENTITIES: [(i32, &str); 3] = [UUID_TAG_NAME, PUUID_TAG_NAME, INSTUUID_TAG_NAME];

fn field((tag, name): (i32, &str), dtype: DataType) -> Field {
    let mut field = dtype.nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

fn event(
    registry: Arc<FixRegistry>,
    identifiers: &[(&str, &str)],
    target: Option<(Field, Scalar)>,
    terminal: bool,
) -> yggdryl::Result<FixMsg> {
    let mut columns = vec![
        field((55, "symbol"), DataType::utf8()),
        registry
            .get_group_by_tag(ALTIDS_TAG_NAME.0)
            .unwrap()
            .clone(),
        registry.get_field_by_tag(52).unwrap().clone(),
    ];
    let mut values = vec![
        Scalar::from("ALPHA"),
        Scalar::from_mapping(
            identifiers
                .iter()
                .map(|(name, value)| (Scalar::from(*name), Scalar::from(*value))),
        )
        .unwrap(),
        clock(0),
    ];
    if let Some((field, value)) = target {
        columns.push(field);
        values.push(value);
    }
    if terminal {
        columns.push(field((39, "ordstatus"), DataType::utf8()));
        values.push(Scalar::from("2"));
    }
    FixMsg::with_registry(
        registry,
        DataType::from_fields(columns)
            .unwrap()
            .required_field("event"),
        Scalar::from_sequence(values),
    )
}

fn custom_registry(base: &FixRegistry, target: &Field, registered: bool) -> Arc<FixRegistry> {
    let mut registry = base.clone();
    let tag = target.as_fix().tag().unwrap().unwrap();
    assert!(registry.remove(tag).is_some());
    if registered {
        assert!(registry.insert(target.clone()).unwrap().is_none());
    }
    Arc::new(registry)
}

fn uuid(message: &FixMsg, tag: i32) -> Uuid {
    let Scalar::Uuid(value) = message.by_tag(tag).unwrap() else {
        panic!("tag {tag} must hold a native UUID");
    };
    *value
}

fn located(error: Error, name: &str, expected: &DataType) {
    let Error::InvalidRecord { path, reason } = error else {
        panic!("expected a located native target refusal, got {error}");
    };
    assert_eq!(path, format!("$.{name}"));
    assert!(
        reason.starts_with(&format!("expected {expected}, got ")),
        "{reason}"
    );
}

#[test]
fn mandatory_intake_and_instrument_stamps_refuse_coercible_uuid_targets() {
    let registry = Arc::new(FixRegistry::new());
    for identity in IDENTITIES {
        for dtype in [DataType::utf8(), DataType::binary()] {
            let coerced = dtype.scalar(Scalar::Uuid(Uuid::new(7))).unwrap();
            assert!(
                !matches!(coerced, Scalar::Uuid(_)),
                "the generic value contract permits this coercion"
            );
            let target = field(identity, dtype);
            for registered in [false, true] {
                let custom = custom_registry(&registry, &target, registered);
                let mut life = FixLifecycle::new(Arc::clone(&registry))
                    .try_with_interval_ns(1)
                    .unwrap();
                let message = event(
                    custom,
                    &[("id", "NEW")],
                    Some((target.clone(), Scalar::Null)),
                    false,
                );
                let error = if identity == INSTUUID_TAG_NAME {
                    life.fill(message.unwrap()).unwrap_err()
                } else {
                    message.unwrap_err()
                };
                if !registered && identity != INSTUUID_TAG_NAME {
                    let Error::InvalidRecord { path, reason } = error else {
                        panic!("expected a located missing definition refusal, got {error}");
                    };
                    assert_eq!(path, format!("$.{}", identity.1));
                    assert_eq!(
                        reason,
                        "expected a registered mandatory definition, got missing definition"
                    );
                } else {
                    located(error, identity.1, &DataType::Uuid);
                }
                assert_eq!(life.alive(), 0);
                let accepted = life
                    .fill(event(Arc::clone(&registry), &[("id", "NEW")], None, false).unwrap())
                    .unwrap();
                for (tag, _) in IDENTITIES {
                    uuid(&accepted, tag);
                }
                assert_eq!(life.alive(), 1);
            }
        }
    }
}

#[test]
fn refused_uuid_targets_neither_attach_aliases_nor_close_a_corrected_chain() {
    let registry = Arc::new(FixRegistry::new());
    for identity in IDENTITIES {
        for dtype in [DataType::utf8(), DataType::binary()] {
            let target = field(identity, dtype);
            let custom = custom_registry(&registry, &target, true);
            for terminal in [false, true] {
                let mut life = FixLifecycle::new(Arc::clone(&registry))
                    .try_with_interval_ns(1)
                    .unwrap();
                let live = life
                    .fill(event(Arc::clone(&registry), &[("id", "LIVE")], None, false).unwrap())
                    .unwrap();
                let persistent = uuid(&live, PUUID_TAG_NAME.0);
                let message = event(
                    Arc::clone(&custom),
                    &[("a", "LIVE"), ("b", "NEW")],
                    Some((target.clone(), Scalar::Null)),
                    terminal,
                );
                let error = if identity == INSTUUID_TAG_NAME {
                    life.fill(message.unwrap()).unwrap_err()
                } else {
                    message.unwrap_err()
                };
                located(error, identity.1, &DataType::Uuid);
                assert_eq!(life.alive(), 1, "a refusal cannot close the live chain");
                let independent = life
                    .fill(event(Arc::clone(&registry), &[("id", "NEW")], None, false).unwrap())
                    .unwrap();
                assert_ne!(uuid(&independent, PUUID_TAG_NAME.0), persistent);
                assert_eq!(life.alive(), 2, "a refusal cannot attach the new alias");
                let unchanged = life
                    .fill(event(Arc::clone(&registry), &[("id", "LIVE")], None, false).unwrap())
                    .unwrap();
                assert_eq!(uuid(&unchanged, PUUID_TAG_NAME.0), persistent);
                assert_eq!(life.alive(), 2);
            }
        }
    }
}

#[test]
fn mandatory_null_columns_require_native_layout_but_optional_instrument_can_retype() {
    let registry = Arc::new(FixRegistry::new());
    for identity in IDENTITIES {
        for dtype in [DataType::utf8(), DataType::binary()] {
            let target = field(identity, dtype);
            let message = event(
                Arc::clone(&registry),
                &[("id", "NEW")],
                Some((target, Scalar::Null)),
                false,
            );
            if identity != INSTUUID_TAG_NAME {
                located(message.unwrap_err(), identity.1, &DataType::Uuid);
                continue;
            }
            let mut life = FixLifecycle::new(Arc::clone(&registry))
                .try_with_interval_ns(1)
                .unwrap();
            let message = life.fill(message.unwrap()).unwrap();
            uuid(&message, identity.0);
            assert_eq!(
                message.as_field().get_field(identity.1).unwrap().dtype(),
                &DataType::Uuid,
            );
            assert_eq!(life.alive(), 1);
            life.clear();
            assert_eq!(life.alive(), 0);
            assert_eq!(life.fill(message.clone()).unwrap(), message);
            assert_eq!(life.alive(), 1);
        }
    }
}

fn clock_type() -> DataType {
    DataType::DateTime64 {
        unit: TimeUnit::Nanosecond,
        timezone: Timezone::UTC,
    }
}

fn clock(instant: i64) -> Scalar {
    Scalar::datetime64(instant, TimeUnit::Nanosecond, Timezone::UTC).unwrap()
}

fn timed_event(
    registry: Arc<FixRegistry>,
    identifiers: &[(&str, &str)],
    target: Option<(Field, Scalar)>,
    terminal: bool,
    instant: i64,
    sequence: i64,
) -> FixMsg {
    let mut message = event(registry, identifiers, target, terminal).unwrap();
    message
        .set_many([
            (UPDATEDAT_TAG_NAME.0, clock(instant)),
            (34, Scalar::from(sequence)),
        ])
        .unwrap();
    message
}

fn previous(message: &FixMsg, expected: Option<(i64, Uuid)>) {
    let (timestamp, uuid) = expected.map_or((Scalar::Null, Scalar::Null), |(instant, uuid)| {
        (clock(instant), Scalar::Uuid(uuid))
    });
    assert_eq!(
        message.by_tag(PREVUPDATEDAT_TAG_NAME.0).unwrap(),
        &timestamp
    );
    assert_eq!(message.by_tag(PREVUUID_TAG_NAME.0).unwrap(), &uuid);
}

fn invalid_previous_targets() -> impl Iterator<Item = ((i32, &'static str), DataType)> {
    [
        (
            PREVUPDATEDAT_TAG_NAME,
            vec![
                DataType::Uuid,
                DataType::utf8(),
                DataType::binary(),
                DataType::Int64,
                DataType::DateTime64 {
                    unit: TimeUnit::Microsecond,
                    timezone: Timezone::UTC,
                },
                DataType::DateTime64 {
                    unit: TimeUnit::Nanosecond,
                    timezone: Timezone::NAIVE,
                },
            ],
        ),
        (
            PREVUUID_TAG_NAME,
            vec![
                clock_type(),
                DataType::utf8(),
                DataType::binary(),
                DataType::Int64,
            ],
        ),
    ]
    .into_iter()
    .flat_map(|(identity, types)| types.into_iter().map(move |dtype| (identity, dtype)))
}

#[test]
fn first_null_previous_stamps_refuse_wrong_layouts_before_opening_a_chain() {
    let registry = Arc::new(FixRegistry::new());
    for (identity, dtype) in invalid_previous_targets() {
        let expected = if identity == PREVUPDATEDAT_TAG_NAME {
            clock_type()
        } else {
            DataType::Uuid
        };
        for name in [identity.1, "custom_previous"] {
            let target = field((identity.0, name), dtype.clone());
            for registered in [false, true] {
                let custom = custom_registry(&registry, &target, registered);
                for has_chain in [false, true] {
                    let mut life = FixLifecycle::new(Arc::clone(&registry))
                        .try_with_interval_ns(1)
                        .unwrap();
                    let ids = if has_chain { &[("id", "NEW")][..] } else { &[] };
                    let message = timed_event(
                        Arc::clone(&custom),
                        ids,
                        Some((target.clone(), Scalar::Null)),
                        false,
                        101,
                        101,
                    );
                    located(life.fill(message).unwrap_err(), name, &expected);
                    assert_eq!(life.alive(), 0);
                    let accepted = life
                        .fill(timed_event(
                            Arc::clone(&registry),
                            &[("id", "NEW")],
                            None,
                            false,
                            202,
                            202,
                        ))
                        .unwrap();
                    previous(&accepted, None);
                    assert_eq!(life.alive(), 1);
                }
            }
        }
    }
}

#[test]
fn refused_previous_targets_do_not_advance_history_attach_keys_or_close() {
    let registry = Arc::new(FixRegistry::new());
    for (identity, dtype) in invalid_previous_targets() {
        let expected = if identity == PREVUPDATEDAT_TAG_NAME {
            clock_type()
        } else {
            DataType::Uuid
        };
        for name in [identity.1, "custom_previous"] {
            let target = field((identity.0, name), dtype.clone());
            for registered in [false, true] {
                let custom = custom_registry(&registry, &target, registered);
                for terminal in [false, true] {
                    let mut life = FixLifecycle::new(Arc::clone(&registry))
                        .try_with_interval_ns(1)
                        .unwrap();
                    let live = life
                        .fill(timed_event(
                            Arc::clone(&registry),
                            &[("id", "LIVE")],
                            None,
                            false,
                            101,
                            101,
                        ))
                        .unwrap();
                    let persistent = uuid(&live, PUUID_TAG_NAME.0);
                    previous(&live, None);
                    let rejected = timed_event(
                        Arc::clone(&custom),
                        &[("a", "LIVE"), ("b", "NEW")],
                        Some((target.clone(), Scalar::Null)),
                        terminal,
                        202,
                        202,
                    );
                    located(life.fill(rejected).unwrap_err(), name, &expected);
                    assert_eq!(life.alive(), 1, "a refused terminal cannot close");
                    let next = life
                        .fill(timed_event(
                            Arc::clone(&registry),
                            &[("id", "LIVE")],
                            None,
                            false,
                            303,
                            303,
                        ))
                        .unwrap();
                    assert_eq!(uuid(&next, PUUID_TAG_NAME.0), persistent);
                    previous(&next, Some((101, uuid(&live, UUID_TAG_NAME.0))));
                    let independent = life
                        .fill(timed_event(
                            Arc::clone(&registry),
                            &[("id", "NEW")],
                            None,
                            false,
                            404,
                            404,
                        ))
                        .unwrap();
                    assert_ne!(uuid(&independent, PUUID_TAG_NAME.0), persistent);
                    previous(&independent, None);
                    assert_eq!(life.alive(), 2, "a refusal cannot attach an identifier");
                }
            }
        }
    }
}

#[test]
fn previous_stamps_use_the_input_tag_role_not_the_resolved_columns_name() {
    let registry = Arc::new(FixRegistry::new());
    for (identity, dtype) in [
        (PREVUPDATEDAT_TAG_NAME, clock_type()),
        (PREVUUID_TAG_NAME, DataType::Uuid),
    ] {
        let target = field((identity.0, "custom_previous"), dtype.clone());
        for registered in [false, true] {
            let custom = custom_registry(&registry, &target, registered);
            let mut life = FixLifecycle::new(Arc::clone(&registry))
                .try_with_interval_ns(1)
                .unwrap();
            let mut expected = None;
            for instant in [101, 202] {
                let message = timed_event(
                    Arc::clone(&custom),
                    &[("id", "LIVE")],
                    Some((target.clone(), Scalar::Null)),
                    false,
                    instant,
                    instant,
                );
                let message = life.fill(message).unwrap();
                previous(&message, expected);
                expected = Some((instant, uuid(&message, UUID_TAG_NAME.0)));
                assert_eq!(
                    message
                        .as_field()
                        .get_field("custom_previous")
                        .unwrap()
                        .dtype(),
                    &dtype,
                );
                assert_eq!(life.alive(), 1);
            }
        }
    }
}
