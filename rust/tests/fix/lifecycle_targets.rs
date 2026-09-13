//! Lifecycle UUID stamps require the resolved target's native UUID layout.

use std::sync::Arc;

use yggdryl::types::Uuid;
use yggdryl::{
    ALTIDS_TAG_NAME, DataType, Error, Field, FixLifecycle, FixMsg, FixRegistry, INSTUUID_TAG_NAME,
    PUUID_TAG_NAME, Scalar, UUID_TAG_NAME,
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
) -> FixMsg {
    let mut columns = vec![
        field((55, "symbol"), DataType::utf8()),
        registry
            .get_group_by_counter(ALTIDS_TAG_NAME.0)
            .unwrap()
            .clone(),
    ];
    let mut values = vec![
        Scalar::from("ALPHA"),
        Scalar::from_mapping(
            identifiers
                .iter()
                .map(|(name, value)| (Scalar::from(*name), Scalar::from(*value))),
        )
        .unwrap(),
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
    .unwrap()
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

fn located(error: Error, name: &str) {
    let Error::InvalidRecord { path, reason } = error else {
        panic!("expected a located UUID target refusal, got {error}");
    };
    assert_eq!(path, format!("$.{name}"));
    assert!(reason.contains("uuid"), "{reason}");
}

#[test]
fn generated_uuid_stamps_refuse_coercible_registry_and_row_targets() {
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
                let mut life = FixLifecycle::new(Arc::clone(&registry));
                let message = event(
                    custom,
                    &[("id", "NEW")],
                    Some((target.clone(), Scalar::Null)),
                    false,
                );
                located(life.fill(message).unwrap_err(), identity.1);
                assert_eq!(life.alive(), 0);
                let accepted = life
                    .fill(event(Arc::clone(&registry), &[("id", "NEW")], None, false))
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
                let mut life = FixLifecycle::new(Arc::clone(&registry));
                let live = life
                    .fill(event(Arc::clone(&registry), &[("id", "LIVE")], None, false))
                    .unwrap();
                let persistent = uuid(&live, PUUID_TAG_NAME.0);
                let target = if identity == PUUID_TAG_NAME {
                    let foreign = Uuid::new(999);
                    assert_ne!(foreign, persistent);
                    // The row carries a native stated value, but its registry
                    // directs the correction into a coercible non-UUID field.
                    (field(identity, DataType::Uuid), Scalar::Uuid(foreign))
                } else {
                    (target.clone(), Scalar::Null)
                };
                let message = event(
                    Arc::clone(&custom),
                    &[("a", "LIVE"), ("b", "NEW")],
                    Some(target),
                    terminal,
                );
                located(life.fill(message).unwrap_err(), identity.1);
                assert_eq!(life.alive(), 1, "a refusal cannot close the live chain");
                let independent = life
                    .fill(event(Arc::clone(&registry), &[("id", "NEW")], None, false))
                    .unwrap();
                assert_ne!(uuid(&independent, PUUID_TAG_NAME.0), persistent);
                assert_eq!(life.alive(), 2, "a refusal cannot attach the new alias");
                let unchanged = life
                    .fill(event(Arc::clone(&registry), &[("id", "LIVE")], None, false))
                    .unwrap();
                assert_eq!(uuid(&unchanged, PUUID_TAG_NAME.0), persistent);
                assert_eq!(life.alive(), 2);
            }
        }
    }
}

#[test]
fn registered_uuid_targets_retype_null_row_columns_without_erasing_uuid_values() {
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
            let mut life = FixLifecycle::new(Arc::clone(&registry));
            let message = life.fill(message).unwrap();
            uuid(&message, identity.0);
            assert_eq!(
                message.as_field().get_field(identity.1).unwrap().dtype(),
                &DataType::Uuid,
            );
            assert_eq!(life.alive(), 1);
            assert_eq!(life.fill(message.clone()).unwrap(), message);
            assert_eq!(life.alive(), 1);
        }
    }
}
