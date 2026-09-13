//! Decision 23: UUID-owned chains and instrument-scoped identifier keys.

use std::sync::Arc;

use super::SoleMessage;
use yggdryl::types::Uuid;
use yggdryl::types::nested::{Mapping, Nested};
use yggdryl::{
    ALTIDS_TAG_NAME, DataType, Error, Field, FixCategory, FixCodec, FixLifecycle, FixMsg,
    FixRegistry, INSTUUID_TAG_NAME, PUUID_TAG_NAME, Scalar, TIMESTAMP_TAG_NAME, TimeUnit, Timezone,
    UUID_TAG_NAME,
};

fn field(name: &str, tag: i32, dtype: DataType) -> Field {
    let mut field = dtype.nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

pub(super) fn row(
    registry: &Arc<FixRegistry>,
    cells: impl IntoIterator<Item = (i32, Scalar)>,
) -> FixMsg {
    let mut fields = Vec::new();
    let mut values = Vec::new();
    for (tag, value) in cells {
        let name = if tag == ALTIDS_TAG_NAME.0 {
            ALTIDS_TAG_NAME.1.to_owned()
        } else {
            registry
                .get_field_by_tag(tag)
                .map_or_else(|| tag.to_string(), |field| field.name().to_owned())
        };
        // The row states its actual shape; lifecycle must reject an invalid
        // UUID or Map instead of letting the registry coerce it first.
        let dtype = if value.as_mapping().is_some_and(|entries| entries.is_empty()) {
            DataType::map_of(DataType::utf8(), DataType::utf8(), false).unwrap()
        } else {
            value.dtype().unwrap()
        };
        fields.push(field(&name, tag, dtype));
        values.push(value);
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

fn mapping(entries: &[(&str, &str)]) -> Scalar {
    Scalar::from_mapping(
        entries
            .iter()
            .map(|(key, value)| (Scalar::from(*key), Scalar::from(*value))),
    )
    .unwrap()
}

fn event(
    registry: &Arc<FixRegistry>,
    scope: Option<Uuid>,
    persistent: Option<Uuid>,
    time: i64,
    entries: &[(&str, &str)],
) -> FixMsg {
    let mut cells = vec![
        (ALTIDS_TAG_NAME.0, mapping(entries)),
        (
            TIMESTAMP_TAG_NAME.0,
            Scalar::datetime64(time, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
        ),
    ];
    cells.extend(scope.map(|value| (INSTUUID_TAG_NAME.0, Scalar::Uuid(value))));
    cells.extend(persistent.map(|value| (PUUID_TAG_NAME.0, Scalar::Uuid(value))));
    row(registry, cells)
}

fn uuid(message: &FixMsg, tag: i32) -> Option<Uuid> {
    message
        .get_by_tag(tag)
        .filter(|value| !value.is_null())
        .map(|value| {
            let Scalar::Uuid(value) = value else {
                panic!("tag {tag} must retain its native UUID, got {value:?}");
            };
            *value
        })
}

fn chain(message: &FixMsg) -> Uuid {
    uuid(message, PUUID_TAG_NAME.0).expect("a chain")
}

fn located(error: Error, expected: &str) {
    let Error::InvalidRecord { path, .. } = error else {
        panic!("expected a located value refusal, got {error}");
    };
    assert_eq!(path, expected);
}

#[test]
fn stated_uuid_opens_without_keys_and_direct_join_outranks_identifier_lookup() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let direct = Uuid::new(0);
    let other = Uuid::new(7);
    let first = life
        .fill(event(&registry, None, Some(direct), 0, &[]))
        .unwrap();
    assert_eq!(chain(&first), direct, "nil is a stated UUID, not absence");
    life.fill(event(&registry, None, Some(other), 1, &[("a", "OWNED")]))
        .unwrap();
    let joined = life
        .fill(event(
            &registry,
            None,
            Some(direct),
            2,
            &[("a", "OWNED"), ("b", "NEW")],
        ))
        .unwrap();
    assert_eq!(chain(&joined), direct);
    assert_eq!(life.alive(), 2);
    let owner = life
        .fill(event(&registry, None, None, 3, &[("a", "OWNED")]))
        .unwrap();
    let attached = life
        .fill(event(&registry, None, None, 4, &[("a", "NEW")]))
        .unwrap();
    assert_eq!(
        chain(&owner),
        other,
        "a direct join cannot steal an owned key"
    );
    assert_eq!(chain(&attached), direct);
}

#[test]
fn scoped_identifier_corrects_a_foreign_stated_puuid_but_preserves_other_uuids() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let scope = Uuid::new(123);
    let original = life
        .fill(event(&registry, Some(scope), None, 0, &[("id", "A")]))
        .unwrap();
    let mut message = event(
        &registry,
        Some(scope),
        Some(Uuid::new(456)),
        1,
        &[("id", "A")],
    );
    let own_uuid = Uuid::new(u128::MAX);
    message
        .set(UUID_TAG_NAME.0, Scalar::Uuid(own_uuid))
        .unwrap();
    let corrected = life.fill(message).unwrap();
    assert_eq!(chain(&corrected), chain(&original));
    assert_eq!(uuid(&corrected, INSTUUID_TAG_NAME.0), Some(scope));
    assert_eq!(uuid(&corrected, UUID_TAG_NAME.0), Some(own_uuid));
    assert_eq!(life.alive(), 1);
}

#[test]
fn absent_nil_and_distinct_stated_scopes_have_distinct_generated_chains() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let mut chains = Vec::new();
    for scope in [
        None,
        Some(Uuid::new(0)),
        Some(Uuid::new(1)),
        Some(Uuid::new(2)),
    ] {
        let message = life
            .fill(event(&registry, scope, None, 456, &[("id", "SAME")]))
            .unwrap();
        let mut input = vec![u8::from(scope.is_some())];
        if let Some(scope) = scope {
            input.extend_from_slice(&scope.into_bytes());
        }
        input.extend_from_slice(b"\x1fSAME");
        let expected = Uuid::from_v7(456, yggdryl::xxhash::xxh3(&input)).unwrap();
        assert_eq!(chain(&message), expected);
        assert_eq!(uuid(&message, INSTUUID_TAG_NAME.0), scope);
        assert!(!chains.contains(&expected));
        chains.push(expected);
    }
    assert_eq!(life.alive(), 4);
    let mut null_scope = event(&registry, None, None, 456, &[("id", "SAME")]);
    null_scope.set(INSTUUID_TAG_NAME.0, Scalar::Null).unwrap();
    null_scope.set(PUUID_TAG_NAME.0, Scalar::Null).unwrap();
    assert_eq!(chain(&life.fill(null_scope).unwrap()), chains[0]);
    assert_eq!(life.alive(), 4);
}

#[test]
fn stated_scope_controls_identity_independently_of_raw_instrument_fields() {
    let registry = super::committed_registry();
    let scope = Uuid::new(17);
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let mut first = event(&registry, Some(scope), None, 0, &[("id", "A")]);
    first.set(55, Scalar::from("ALPHA")).unwrap();
    let first = life.fill(first).unwrap();
    let mut second = event(&registry, Some(scope), None, 1, &[("id", "A")]);
    second.set(55, Scalar::from("BETA")).unwrap();
    let second = life.fill(second).unwrap();
    assert_eq!(chain(&first), chain(&second));
    let mut elsewhere = event(&registry, Some(Uuid::new(18)), None, 0, &[("id", "A")]);
    elsewhere.set(55, Scalar::from("ALPHA")).unwrap();
    let elsewhere = life.fill(elsewhere).unwrap();
    assert_ne!(chain(&first), chain(&elsewhere));
    assert_eq!(life.alive(), 2);
}

#[test]
fn member_name_priority_wins_without_merging_or_stealing_aliases() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let older = life
        .fill(event(&registry, None, None, 0, &[("id", "OLDER")]))
        .unwrap();
    let newer = life
        .fill(event(&registry, None, None, 1, &[("id", "NEWER")]))
        .unwrap();
    let conflict = life
        .fill(event(
            &registry,
            None,
            None,
            2,
            &[("a", "NEWER"), ("b", "OLDER"), ("c", "ADDED")],
        ))
        .unwrap();
    assert_eq!(
        chain(&conflict),
        chain(&newer),
        "map name order, not chain age"
    );
    assert_eq!(life.alive(), 2);
    for (key, expected) in [
        ("OLDER", chain(&older)),
        ("NEWER", chain(&newer)),
        ("ADDED", chain(&newer)),
    ] {
        assert_eq!(
            chain(
                &life
                    .fill(event(&registry, None, None, 3, &[("id", key)]))
                    .unwrap()
            ),
            expected
        );
    }
    let mut terminal = event(&registry, None, None, 4, &[("a", "NEWER"), ("b", "OLDER")]);
    terminal.set(39, Scalar::from("2")).unwrap();
    assert_eq!(chain(&life.fill(terminal).unwrap()), chain(&newer));
    assert_eq!(life.alive(), 1);
    assert_eq!(
        chain(
            &life
                .fill(event(&registry, None, None, 5, &[("id", "OLDER")]))
                .unwrap()
        ),
        chain(&older)
    );
    assert_ne!(
        chain(
            &life
                .fill(event(&registry, None, None, 6, &[("id", "ADDED")]))
                .unwrap()
        ),
        chain(&newer)
    );
}

#[test]
fn terminal_cleanup_removes_all_directly_attached_scopes_and_clear_replays() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let persistent = Uuid::new(91);
    for (scope, key) in [
        (Some(Uuid::new(1)), "A"),
        (Some(Uuid::new(2)), "B"),
        (None, "C"),
    ] {
        life.fill(event(&registry, scope, Some(persistent), 0, &[("id", key)]))
            .unwrap();
    }
    assert_eq!(life.alive(), 1);
    let mut terminal = event(&registry, None, Some(persistent), 1, &[]);
    terminal.set(39, Scalar::from("2")).unwrap();
    life.fill(terminal).unwrap();
    assert_eq!(life.alive(), 0);
    let mut reopened = Vec::new();
    for (scope, key) in [
        (Some(Uuid::new(1)), "A"),
        (Some(Uuid::new(2)), "B"),
        (None, "C"),
    ] {
        let message = life
            .fill(event(&registry, scope, None, 2, &[("id", key)]))
            .unwrap();
        assert_ne!(chain(&message), persistent);
        reopened.push(message);
    }
    assert_eq!(life.alive(), 3);
    life.clear();
    assert_eq!(life.alive(), 0);
    for message in reopened {
        assert_eq!(life.fill(message.clone()).unwrap(), message);
    }
    assert_eq!(life.alive(), 3);
    let mut terminal = event(
        &registry,
        None,
        Some(Uuid::new(999)),
        3,
        &[("id", "FIRST-TERMINAL")],
    );
    terminal.set(39, Scalar::from("2")).unwrap();
    assert_eq!(chain(&life.fill(terminal).unwrap()), Uuid::new(999));
    assert_eq!(life.alive(), 3, "a first terminal event leaves no chain");
    assert_ne!(
        chain(
            &life
                .fill(event(&registry, None, None, 4, &[("id", "FIRST-TERMINAL")]))
                .unwrap()
        ),
        Uuid::new(999)
    );
}

#[test]
fn null_empty_duplicate_case_and_whitespace_values_keep_their_exact_meanings() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let no_ids = Scalar::from_mapping([
        (Scalar::from("a"), Scalar::Null),
        (Scalar::from("b"), Scalar::from("")),
    ])
    .unwrap();
    let empty = life
        .fill(row(&registry, [(ALTIDS_TAG_NAME.0, no_ids)]))
        .unwrap();
    assert_eq!(uuid(&empty, PUUID_TAG_NAME.0), None);
    assert_eq!(life.alive(), 0);
    let mut chains = Vec::new();
    for value in ["ID", "id", " ID ", " "] {
        let message = life
            .fill(event(
                &registry,
                None,
                None,
                0,
                &[("a", value), ("b", value)],
            ))
            .unwrap();
        assert!(!chains.contains(&chain(&message)));
        chains.push(chain(&message));
    }
    assert_eq!(life.alive(), 4);
    for (value, expected) in ["ID", "id", " ID ", " "].into_iter().zip(chains) {
        assert_eq!(
            chain(
                &life
                    .fill(event(&registry, None, None, 1, &[("renamed", value)]))
                    .unwrap()
            ),
            expected
        );
    }
}

#[test]
fn stated_altids_including_empty_overrides_compiled_fallback_and_null_does_not() {
    let registry = super::committed_registry();
    let make = |altids| {
        row(
            &registry,
            [
                (35, Scalar::from("D")),
                (11, Scalar::from("RAW")),
                (ALTIDS_TAG_NAME.0, altids),
            ],
        )
    };
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let empty = life.fill(make(mapping(&[]))).unwrap();
    assert_eq!(uuid(&empty, PUUID_TAG_NAME.0), None);
    assert_eq!(empty.by_tag(ALTIDS_TAG_NAME.0).unwrap(), &mapping(&[]));
    let raw = life.fill(make(Scalar::Null)).unwrap();
    assert_eq!(life.alive(), 1);
    let stated = mapping(&[("custom", "STATED")]);
    let mapped = life.fill(make(stated.clone())).unwrap();
    assert_ne!(chain(&raw), chain(&mapped));
    assert_eq!(mapped.by_tag(ALTIDS_TAG_NAME.0).unwrap(), &stated);
    assert_eq!(
        chain(
            &life
                .fill(event(&registry, None, None, 1, &[("id", "RAW")]))
                .unwrap()
        ),
        chain(&raw)
    );
    assert_eq!(life.alive(), 2);
}

fn declared_registry(dtype: DataType) -> Arc<FixRegistry> {
    let first = field("zidentifier", 71_001, dtype.clone());
    let second = field("aidentifier", 71_002, dtype);
    let mut registry = FixRegistry::from_fields([
        field("msgtype", 35, DataType::utf8()),
        first.clone(),
        second.clone(),
    ])
    .unwrap();
    let mut component = DataType::from_fields([first, second])
        .unwrap()
        .required_field("custom");
    component.as_fix_mut().set_msgtype("ZID").unwrap();
    component
        .as_fix_mut()
        .set_identifiers(["zidentifier", "aidentifier"])
        .unwrap();
    registry
        .create_definition(FixCategory::Components, component)
        .unwrap();
    Arc::new(registry)
}

#[test]
fn compiled_integer_identifiers_match_enrichments_sorted_map_and_priority() {
    let registry = declared_registry(DataType::Int64);
    let codec = FixCodec::new(Arc::clone(&registry));
    let message = row(
        &registry,
        [
            (35, Scalar::from("ZID")),
            (71_001, Scalar::from(91_i64)),
            (71_002, Scalar::from(42_i64)),
        ],
    );
    let enriched = codec.enrich_message(message.clone()).unwrap();
    assert_eq!(
        enriched.by_tag(ALTIDS_TAG_NAME.0).unwrap(),
        &mapping(&[("aidentifier", "42"), ("zidentifier", "91")])
    );
    for message in [message, enriched] {
        let mut life = FixLifecycle::new(Arc::clone(&registry));
        let later_name = life
            .fill(event(&registry, None, None, 0, &[("id", "91")]))
            .unwrap();
        let earlier_name = life
            .fill(event(&registry, None, None, 1, &[("id", "42")]))
            .unwrap();
        assert_ne!(chain(&later_name), chain(&earlier_name));
        assert_eq!(chain(&life.fill(message).unwrap()), chain(&earlier_name));
        assert_eq!(life.alive(), 2);
    }
}

#[test]
fn invalid_identifier_text_refuses_atomically_at_the_declared_member() {
    let registry = declared_registry(DataType::binary());
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let live = life
        .fill(event(
            &registry,
            None,
            Some(Uuid::new(8)),
            0,
            &[("id", "LIVE")],
        ))
        .unwrap();
    let invalid = row(
        &registry,
        [
            (35, Scalar::from("ZID")),
            (71_001, Scalar::from(vec![0xff_u8])),
            (PUUID_TAG_NAME.0, Scalar::Uuid(chain(&live))),
            (39, Scalar::from("2")),
        ],
    );
    let error = life.fill(invalid).unwrap_err();
    located(error, "$.zidentifier");
    assert_eq!(life.alive(), 1);
    assert_eq!(
        chain(
            &life
                .fill(event(&registry, None, None, 1, &[("id", "LIVE")]))
                .unwrap()
        ),
        chain(&live)
    );
}

#[test]
fn unknown_message_types_have_no_hard_tag_or_nested_identifier_fallback() {
    let registry = super::committed_registry();
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let unknown = row(
        &registry,
        [
            (35, Scalar::from("UNKNOWN-CUSTOM")),
            (11, Scalar::from("A")),
            (37, Scalar::from("B")),
        ],
    );
    assert_eq!(uuid(&life.fill(unknown).unwrap(), PUUID_TAG_NAME.0), None);

    let nested_member = field("innerid", 71_003, DataType::utf8());
    let mut item = DataType::from_fields([nested_member])
        .unwrap()
        .required_field("item");
    item.as_fix_mut().set_identifiers(["innerid"]).unwrap();
    let nested = DataType::list(item).nullable_field("nested");
    let mut definition = DataType::from_fields([nested.clone()])
        .unwrap()
        .required_field("nestedonly");
    definition.as_fix_mut().set_msgtype("ZNEST").unwrap();
    let mut custom = FixRegistry::from_fields([field("msgtype", 35, DataType::utf8())]).unwrap();
    custom
        .create_definition(FixCategory::Components, definition)
        .unwrap();
    let custom = Arc::new(custom);
    let message = FixMsg::with_registry(
        Arc::clone(&custom),
        DataType::from_fields([field("msgtype", 35, DataType::utf8()), nested])
            .unwrap()
            .required_field("nestedonly"),
        Scalar::from_sequence([
            Scalar::from("ZNEST"),
            Scalar::from_sequence([Scalar::from_sequence([Scalar::from("NESTED-ID")])]),
        ]),
    )
    .unwrap();
    let mut life = FixLifecycle::new(custom);
    assert_eq!(uuid(&life.fill(message).unwrap(), PUUID_TAG_NAME.0), None);
    assert_eq!(life.alive(), 0);
}

#[test]
fn malformed_maps_and_uuid_shapes_refuse_before_direct_join_or_terminal_cleanup() {
    let registry = Arc::new(FixRegistry::new());
    let invalid_maps = [
        (Scalar::from("not a map"), "$.altids"),
        (
            Scalar::from_mapping([(Scalar::from(1_i64), Scalar::from("NEW"))]).unwrap(),
            "$.altids[0].key",
        ),
        (
            Scalar::from_mapping([(Scalar::from("a"), Scalar::from(7_i64))]).unwrap(),
            "$.altids[0].value",
        ),
        (
            Scalar::from_mapping([(Scalar::from("a"), Scalar::from(vec![0xff_u8]))]).unwrap(),
            "$.altids[0].value",
        ),
        (mapping(&[("b", "NEW"), ("a", "LIVE")]), "$.altids[1].key"),
    ];
    let invalid = invalid_maps
        .into_iter()
        .map(|(value, path)| (ALTIDS_TAG_NAME.0, value, path))
        .chain([
            (
                INSTUUID_TAG_NAME.0,
                Scalar::from("00000000-0000-0000-0000-000000000000"),
                "$.instuuid",
            ),
            (PUUID_TAG_NAME.0, Scalar::from(vec![0_u8; 16]), "$.puuid"),
            (UUID_TAG_NAME.0, Scalar::from(1_i64), "$.uuid"),
        ]);
    for (tag, value, path) in invalid {
        let mut life = FixLifecycle::new(Arc::clone(&registry));
        let live = life
            .fill(event(
                &registry,
                None,
                Some(Uuid::new(8)),
                0,
                &[("id", "LIVE")],
            ))
            .unwrap();
        let mut cells = vec![(39, Scalar::from("2")), (tag, value)];
        if tag != PUUID_TAG_NAME.0 {
            cells.push((PUUID_TAG_NAME.0, Scalar::Uuid(chain(&live))));
        }
        if tag != ALTIDS_TAG_NAME.0 {
            cells.push((ALTIDS_TAG_NAME.0, mapping(&[("a", "LIVE"), ("b", "NEW")])));
        }
        located(life.fill(row(&registry, cells)).unwrap_err(), path);
        assert_eq!(life.alive(), 1, "{path}");
        let independent = life
            .fill(event(&registry, None, None, 1, &[("id", "NEW")]))
            .unwrap();
        assert_ne!(
            chain(&independent),
            chain(&live),
            "{path}: no alias was published"
        );
        assert_eq!(life.alive(), 2);
    }
}

#[test]
fn malformed_identifier_and_uuid_reasons_bound_ascii_and_multibyte_payloads() {
    let registry = Arc::new(FixRegistry::new());
    for unit in ["x", "界🦀"] {
        for repeats in [128, 4_096] {
            let long = unit.repeat(repeats);
            let first = format!("z{long}");
            let second = format!("a{long}");
            let invalid = [
                (
                    ALTIDS_TAG_NAME.0,
                    Scalar::from(long.clone()),
                    "$.altids",
                    "a sorted identifier Map",
                    true,
                ),
                (
                    INSTUUID_TAG_NAME.0,
                    Scalar::from(long.clone()),
                    "$.instuuid",
                    "a native UUID or null",
                    true,
                ),
                (
                    PUUID_TAG_NAME.0,
                    Scalar::from(long.clone()),
                    "$.puuid",
                    "a native UUID or null",
                    true,
                ),
                (
                    UUID_TAG_NAME.0,
                    Scalar::from(long.clone()),
                    "$.uuid",
                    "a native UUID or null",
                    true,
                ),
                (
                    ALTIDS_TAG_NAME.0,
                    Scalar::from_mapping([(
                        Scalar::from(long.as_bytes().to_vec()),
                        Scalar::from("NEW"),
                    )])
                    .unwrap(),
                    "$.altids[0].key",
                    "a UTF-8 identifier name",
                    false,
                ),
                (
                    ALTIDS_TAG_NAME.0,
                    Scalar::from_mapping([(
                        Scalar::from("id"),
                        Scalar::from(long.as_bytes().to_vec()),
                    )])
                    .unwrap(),
                    "$.altids[0].value",
                    "a UTF-8 identifier or null",
                    false,
                ),
                (
                    ALTIDS_TAG_NAME.0,
                    mapping(&[(&first, "NEW"), (&second, "LIVE")]),
                    "$.altids[1].key",
                    "unique identifier names in ascending order",
                    true,
                ),
            ];
            for (tag, value, expected_path, expected, retains_text) in invalid {
                let mut life = FixLifecycle::new(Arc::clone(&registry));
                let live = Uuid::new(8);
                life.fill(event(&registry, None, Some(live), 0, &[("id", "LIVE")]))
                    .unwrap();
                let mut cells = vec![(39, Scalar::from("2")), (tag, value)];
                if tag != PUUID_TAG_NAME.0 {
                    cells.push((PUUID_TAG_NAME.0, Scalar::Uuid(live)));
                }
                if tag != ALTIDS_TAG_NAME.0 {
                    cells.push((ALTIDS_TAG_NAME.0, mapping(&[("a", "LIVE"), ("b", "NEW")])));
                }
                let error = life.fill(row(&registry, cells)).unwrap_err();
                let Error::InvalidRecord { path, reason } = error else {
                    panic!("expected a located value refusal, got {error}");
                };
                assert_eq!(path, expected_path);
                let prefix = format!("expected {expected}, got ");
                let actual = reason
                    .strip_prefix(prefix.as_str())
                    .expect("the refusal retains both expected and actual");
                // The shared error renderer admits 64 payload bytes plus its
                // three-byte ellipsis, independent of input size or encoding.
                assert!(actual.len() <= 64 + '…'.len_utf8(), "{reason}");
                assert!(actual.ends_with('…'), "{reason}");
                if retains_text {
                    assert!(actual.contains(unit), "{reason}");
                }
                assert!(!actual.contains('\u{fffd}'), "{reason}");
                assert_eq!(life.alive(), 1);
                let retained = life
                    .fill(event(&registry, None, None, 1, &[("id", "LIVE")]))
                    .unwrap();
                assert_eq!(chain(&retained), live);
                let independent = life
                    .fill(event(&registry, None, None, 2, &[("id", "NEW")]))
                    .unwrap();
                assert_ne!(chain(&independent), live);
                assert_eq!(life.alive(), 2);
            }
        }
    }
}

#[test]
fn a_generated_uuid_collision_does_not_join_an_unrelated_explicit_chain() {
    let registry = Arc::new(FixRegistry::new());
    let mut life = FixLifecycle::new(Arc::clone(&registry));
    let colliding = Uuid::from_v7(0, yggdryl::xxhash::xxh3(b"\0\x1fNEW")).unwrap();
    life.fill(event(
        &registry,
        None,
        Some(colliding),
        0,
        &[("id", "UNRELATED")],
    ))
    .unwrap();
    located(
        life.fill(event(&registry, None, None, 0, &[("id", "NEW")]))
            .unwrap_err(),
        "$.puuid",
    );
    assert_eq!(life.alive(), 1);
    let later = life
        .fill(event(&registry, None, None, 1, &[("id", "NEW")]))
        .unwrap();
    assert_ne!(chain(&later), colliding);
    assert_eq!(life.alive(), 2);
}

#[test]
fn duplicate_mapping_keys_are_refused_before_a_message_can_reach_lifecycle() {
    let registry = Arc::new(FixRegistry::new());
    let mapping = Scalar::Nested(Nested::Mapping(Mapping::new(vec![
        (Scalar::from("id"), Scalar::from("A")),
        (Scalar::from("id"), Scalar::from("B")),
    ])));
    let column = field(
        ALTIDS_TAG_NAME.1,
        ALTIDS_TAG_NAME.0,
        DataType::map_of(DataType::utf8(), DataType::utf8(), false).unwrap(),
    );
    let result = FixMsg::with_registry(
        registry,
        DataType::from_fields([column])
            .unwrap()
            .required_field("event"),
        Scalar::from_sequence([mapping]),
    );
    located(result.unwrap_err(), "$.event.altids[1].key");
}

#[test]
fn a_fresh_replay_rebuilds_direct_and_corrected_chains_without_touching_arrivals() {
    let registry = super::committed_registry();
    let codec = FixCodec::new(Arc::clone(&registry));
    let specs = [
        (Some(Uuid::new(1)), Some(Uuid::new(20)), "A", false),
        (Some(Uuid::new(2)), Some(Uuid::new(20)), "B", false),
        (None, Some(Uuid::new(30)), "C", false),
        (Some(Uuid::new(2)), Some(Uuid::new(999)), "B", true),
        (Some(Uuid::new(1)), None, "A", false),
    ];
    let mut input = Vec::new();
    for (at, (scope, persistent, key, terminal)) in specs.into_iter().enumerate() {
        let line = format!("8=FIX.4.4|35=D|11={key}|60=20260102-10:15:3{at}|10=0|");
        let mut message = codec.sole_line(line.as_bytes(), false).unwrap();
        let mut writes = vec![(ALTIDS_TAG_NAME.0, mapping(&[("id", key)]))];
        writes.extend(scope.map(|scope| (INSTUUID_TAG_NAME.0, Scalar::Uuid(scope))));
        writes.extend(persistent.map(|persistent| (PUUID_TAG_NAME.0, Scalar::Uuid(persistent))));
        if terminal {
            writes.push((39, Scalar::from("2")));
        }
        message.set_many(writes).unwrap();
        input.push(message);
    }
    let mut first = FixLifecycle::new(Arc::clone(&registry));
    let mut once = Vec::new();
    for (message, alive) in input.into_iter().zip([1, 1, 2, 1, 2]) {
        let entries = message.entries().to_vec();
        let digest = message.digest();
        let wire = message.into_bytes(b'|');
        let message = first.fill(message).unwrap();
        assert_eq!(first.alive(), alive);
        assert_eq!(message.entries(), entries);
        assert_eq!(message.digest(), digest);
        assert_eq!(message.into_bytes(b'|'), wire);
        once.push(message);
    }
    let mut second = FixLifecycle::new(registry);
    for (message, alive) in once.into_iter().zip([1, 1, 2, 1, 2]) {
        let replayed = second.fill(message.clone()).unwrap();
        assert_eq!(replayed, message);
        assert_eq!(replayed.entries(), message.entries());
        assert_eq!(replayed.digest(), message.digest());
        assert_eq!(replayed.into_bytes(b'|'), message.into_bytes(b'|'));
        assert_eq!(second.alive(), alive);
    }
}
