//! Decision 26: settled clocks and canonical named content own identity.

use std::sync::Arc;

use super::{SoleMessage, identity_bytes, identity_scalar, numbered_identity, persistent_identity};
use yggdryl::hashing::txhash::TxHash;
use yggdryl::{
    CODE_TAG_NAME, CREATEDAT_TAG_NAME, DataType, DigestAlgorithm, Error, Field, FixCodec, FixKey,
    FixMsg, FixRegistry, MSGHASH_TAG_NAME, MSGPHASH_TAG_NAME, SNAPSHOTAT_TAG_NAME, Scalar,
    TimeUnit, Timezone, UPDATEDAT_TAG_NAME,
};

const CLOCK: i64 = 123_456_789;

/// The columns whose *value* every message carries.
///
/// `snapshotat` is not one: a row that is not a snapshot says so by leaving
/// it empty, so it is held rather than mandatory.
const BUNDLE: [i32; 6] = [
    UPDATEDAT_TAG_NAME.0,
    CREATEDAT_TAG_NAME.0,
    MSGHASH_TAG_NAME.0,
    MSGPHASH_TAG_NAME.0,
    CODE_TAG_NAME.0,
    52,
];

/// The columns the replay bundle *holds*: [`BUNDLE`] and the snapshot clock.
///
/// A holder is never removed and is always reached by its tag under whatever
/// name it was renamed to, whether or not a message fills it.
const HELD: [i32; 7] = [
    UPDATEDAT_TAG_NAME.0,
    CREATEDAT_TAG_NAME.0,
    MSGHASH_TAG_NAME.0,
    MSGPHASH_TAG_NAME.0,
    CODE_TAG_NAME.0,
    SNAPSHOTAT_TAG_NAME.0,
    52,
];

fn clock(count: i64) -> Scalar {
    Scalar::datetime64(count, TimeUnit::Nanosecond, Timezone::UTC).unwrap()
}

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    DataType::from_fields(fields)
        .unwrap()
        .required_field("event")
}

fn declared(registry: &FixRegistry, tag: i32) -> Field {
    registry.get_field_by_tag(tag).unwrap().clone()
}

fn message(extra: impl IntoIterator<Item = (Field, Scalar)>) -> FixMsg {
    let registry = Arc::new(FixRegistry::new());
    let (fields, values): (Vec<_>, Vec<_>) =
        std::iter::once((declared(&registry, 52), clock(CLOCK)))
            .chain(extra)
            .unzip();
    FixMsg::with_registry(registry, root(fields), Scalar::from_sequence(values)).unwrap()
}

fn payload(value: Scalar) -> (Field, Scalar) {
    (DataType::utf8().nullable_field("payload"), value)
}

fn aliased_message() -> FixMsg {
    let mut registry = FixRegistry::new();
    for tag in HELD {
        let mut field = declared(&registry, tag);
        field
            .as_fix_mut()
            .set_names([format!("alias_{tag}")])
            .unwrap();
        registry.update(field).unwrap();
    }
    let field = root([declared(&registry, 52)]);
    FixMsg::with_registry(
        Arc::new(registry),
        field,
        Scalar::from_sequence([clock(CLOCK)]),
    )
    .unwrap()
}

fn position(message: &FixMsg, tag: i32) -> usize {
    message
        .as_field()
        .fields()
        .iter()
        .position(|field| field.as_fix().tag().unwrap() == Some(tag))
        .unwrap()
}

fn located(error: Error, path: &str) {
    let Error::InvalidRecord {
        path: actual,
        reason,
    } = error
    else {
        panic!("expected a located record refusal, got {error}");
    };
    assert_eq!(actual, path);
    assert!(reason.starts_with("expected "), "{reason}");
    assert!(reason.contains(", got "), "{reason}");
}

fn assert_mirrors(message: &FixMsg) {
    for (tag, hard) in [
        (UPDATEDAT_TAG_NAME.0, message.updatedat()),
        (CREATEDAT_TAG_NAME.0, message.createdat()),
        (MSGHASH_TAG_NAME.0, message.msghash()),
        (MSGPHASH_TAG_NAME.0, message.msgphash()),
    ] {
        assert!(std::ptr::eq(message.get_by_tag(tag).unwrap(), hard));
        assert_eq!(message.as_value().get(position(message, tag)), Some(hard));
        assert!(!message.as_field().fields()[position(message, tag)].is_nullable());
    }
    for value in [message.updatedat(), message.createdat()] {
        assert!(
            matches!(value.as_datetime64(), Some((_, TimeUnit::Nanosecond, zone)) if *zone == Timezone::UTC)
        );
    }
    identity_bytes(message.msghash());
    identity_bytes(message.msgphash());
}

/// The public Scalar record feed is the independent framing oracle.
fn expected_msghash(message: &FixMsg) -> Scalar {
    let content = Scalar::from_record(
        message
            .as_field()
            .fields()
            .iter()
            .zip(message.as_value().as_sequence().unwrap())
            .filter(|(field, _)| {
                field.name() != yggdryl::fix::FIXENTRIES_COLUMN
                    && !field.as_fix().tag().unwrap().is_some_and(|tag| {
                        // What `identity::outside_content` leaves out: the
                        // identity and the clocks it is computed against,
                        // and the three facts about the *capture* rather
                        // than the message.
                        [
                            MSGHASH_TAG_NAME.0,
                            UPDATEDAT_TAG_NAME.0,
                            CREATEDAT_TAG_NAME.0,
                            yggdryl::SOURCEURL_TAG_NAME.0,
                            yggdryl::RECORDEDAT_TAG_NAME.0,
                            yggdryl::NOFIXENTRIES_TAG_NAME.0,
                        ]
                        .contains(&tag)
                    })
            })
            .map(|(field, value)| (field.name(), value.clone())),
    )
    .unwrap();
    identity_scalar(
        TxHash::new_in(
            message
                .updatedat()
                .temporal_count_at(TimeUnit::Nanosecond)
                .unwrap(),
            TimeUnit::Nanosecond,
            content.digest(DigestAlgorithm::Xxh64),
        )
        .unwrap()
        .into_ordered_bytes()
        .unwrap(),
    )
}

#[test]
fn fixed_intake_clock_settles_native_hard_values_and_replays_exactly() {
    let codec = FixCodec::new(Arc::new(FixRegistry::new()))
        .try_with_default_sending_time(Some(clock(CLOCK)))
        .unwrap();
    let bytes = b"8=FIX.4.4|35=0|10=0|";
    let first = codec.sole_line(bytes, false).unwrap();
    let second = codec.sole_line(bytes, false).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.clone(), first);
    assert_eq!(first.updatedat(), &clock(CLOCK));
    assert_eq!(first.createdat(), &clock(CLOCK));
    // A read is not a snapshot: only `FixLifecycle::snapshot` stamps that
    // clock, so an ordinary message leaves it empty.
    assert!(first.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap().is_null());
    assert_eq!(first.by_tag(52).unwrap(), &clock(CLOCK));
    assert_eq!(first.by_tag(CODE_TAG_NAME.0).unwrap().as_str(), Some(""));
    assert_eq!(first.into_bytes(b'|'), bytes);
    assert_eq!(first.digest(), second.digest());
    assert_mirrors(&first);
    assert_eq!(first.msghash(), &expected_msghash(&first));
}

#[test]
fn message_sending_and_transact_clocks_precede_the_fixed_default() {
    let codec = FixCodec::new(Arc::new(FixRegistry::new()))
        .try_with_default_sending_time(Some(clock(CLOCK)))
        .unwrap();
    let message = codec
        .sole_line(
            b"8=FIX.4.4|35=0|52=19700101-00:00:02.123456789|60=19700101-00:00:03.987654321|10=0|",
            false,
        )
        .unwrap();
    assert_eq!(message.by_tag(52).unwrap(), &clock(2_123_456_789));
    assert_eq!(message.by_tag(60).unwrap(), &clock(3_987_654_321));
    assert!(message.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap().is_null());
    assert_eq!(message.updatedat(), &clock(3_987_654_321));
    assert_eq!(message.createdat(), &clock(3_987_654_321));
    assert_mirrors(&message);
}

#[test]
fn explicitly_stated_event_grid_and_creation_clocks_are_independent() {
    let registry = FixRegistry::new();
    let held = message([
        (declared(&registry, 60), clock(3)),
        (declared(&registry, SNAPSHOTAT_TAG_NAME.0), clock(9)),
        (declared(&registry, UPDATEDAT_TAG_NAME.0), clock(7)),
        (declared(&registry, CREATEDAT_TAG_NAME.0), clock(6)),
        (
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC,
            }
            .required_field("timestamp"),
            clock(999),
        ),
    ]);
    assert_eq!(held.updatedat(), &clock(7));
    assert_eq!(held.createdat(), &clock(6));
    assert_eq!(held.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap(), &clock(9));
    assert_eq!(held.by_name("timestamp").unwrap(), &clock(999));
    assert_eq!(
        held.as_field()
            .get_field("timestamp")
            .unwrap()
            .as_fix()
            .tag()
            .unwrap(),
        None
    );
    assert_mirrors(&held);
}

#[test]
fn fixed_default_clock_intake_is_exact_and_atomic() {
    let mut codec = FixCodec::new(Arc::new(FixRegistry::new()))
        .try_with_default_sending_time(Some(clock(CLOCK)))
        .unwrap();
    for value in [
        Scalar::Null,
        Scalar::from("1970-01-01T00:00:00Z"),
        Scalar::datetime64(0, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
        Scalar::datetime64(0, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
    ] {
        located(
            codec.set_default_sending_time(Some(value)).unwrap_err(),
            "$.default_sending_time",
        );
        assert_eq!(codec.default_sending_time(), Some(&clock(CLOCK)));
    }
    codec.set_default_sending_time(None).unwrap();
    assert_eq!(codec.default_sending_time(), None);
}

#[test]
fn the_persistent_identity_hashes_only_exact_code_bytes_including_the_empty_name() {
    let registry = FixRegistry::new();
    let mut identities = Vec::new();
    for code in ["", " ", "alpha", "alpha ", "é"] {
        let mut held = message([(declared(&registry, CODE_TAG_NAME.0), Scalar::from(code))]);
        let expected = identity_scalar(persistent_identity(code));
        assert_eq!(held.msgphash(), &expected);
        assert_ne!(held.msgphash(), &identity_scalar(numbered_identity(0)));
        held.set(UPDATEDAT_TAG_NAME.0, clock(-1)).unwrap();
        held.set(7777, Scalar::from("different content")).unwrap();
        assert_eq!(held.msgphash(), &expected);
        assert_eq!(held.msghash(), &expected_msghash(&held));
        identities.push(expected);
    }
    identities.sort();
    identities.dedup();
    assert_eq!(identities.len(), 5);
}

#[test]
fn canonical_named_content_ignores_root_order_and_metadata_not_names_or_nulls() {
    let pairs = [
        payload(Scalar::from("value")),
        (DataType::Int32.required_field("Z"), Scalar::from(7_i32)),
        (DataType::utf8().required_field("é"), Scalar::from("text")),
    ];
    let first = message(pairs.clone());
    let mut reversed = pairs;
    reversed.reverse();
    for (field, _) in &mut reversed {
        field
            .insert_metadata("example:note", "not content")
            .unwrap();
    }
    let second = message(reversed);
    assert_eq!(first.msghash(), second.msghash());
    assert_ne!(first, second, "message equality still includes the schema");
    assert_eq!(first.msghash(), &expected_msghash(&first));
    assert_eq!(second.msghash(), &expected_msghash(&second));
    let absent = message([]);
    let null = message([payload(Scalar::Null)]);
    let empty = message([payload(Scalar::from(""))]);
    let renamed = message([(DataType::utf8().nullable_field("renamed"), Scalar::Null)]);
    let mut identities = vec![
        absent.msghash(),
        null.msghash(),
        empty.msghash(),
        renamed.msghash(),
    ];
    identities.sort();
    identities.dedup();
    assert_eq!(identities.len(), 4);
}

#[test]
fn nested_sequence_order_is_content_and_uses_the_existing_scalar_feed() {
    let field = DataType::list(DataType::Int32.required_field("item")).required_field("values");
    let first = message([(
        field.clone(),
        Scalar::from_sequence([Scalar::from(1_i32), Scalar::from(2_i32)]),
    )]);
    let second = message([(
        field,
        Scalar::from_sequence([Scalar::from(2_i32), Scalar::from(1_i32)]),
    )]);
    assert_ne!(first.msghash(), second.msghash());
    assert_eq!(first.msghash(), &expected_msghash(&first));
    assert_eq!(second.msghash(), &expected_msghash(&second));
}

#[test]
fn ordinary_mutation_recomputes_identity_and_excludes_only_the_owned_clocks() {
    let mut held = message([payload(Scalar::from("first"))]);
    let before = held.clone();
    held.set(CREATEDAT_TAG_NAME.0, clock(-10)).unwrap();
    assert_eq!(
        held.msghash(),
        before.msghash(),
        "creation time is excluded"
    );
    assert_ne!(held, before);
    held.set(UPDATEDAT_TAG_NAME.0, clock(CLOCK + 1)).unwrap();
    assert_ne!(
        held.msghash(),
        before.msghash(),
        "one nanosecond remains visible"
    );
    let old = held.msghash().clone();
    held.set("payload", Scalar::from("second")).unwrap();
    assert_ne!(held.msghash(), &old);
    let settled = held.updatedat().clone();
    let old = held.msghash().clone();
    held.set(52, clock(CLOCK + 2)).unwrap();
    assert_ne!(held.msghash(), &old);
    assert_eq!(
        held.updatedat(),
        &settled,
        "mutating SendingTime does not reread or reset clocks"
    );
    let old = held.msghash().clone();
    held.set(SNAPSHOTAT_TAG_NAME.0, clock(CLOCK + 3)).unwrap();
    assert_ne!(held.msghash(), &old);
    assert_eq!(held.msgphash(), before.msgphash());
    assert_mirrors(&held);
    assert_eq!(held.msghash(), &expected_msghash(&held));
}

#[test]
fn explicit_identity_writes_assert_the_complete_candidate_atomically() {
    let mut held = message([payload(Scalar::from("first"))]);
    let before = held.clone();
    for (tag, name) in [MSGHASH_TAG_NAME, MSGPHASH_TAG_NAME] {
        located(
            held.set(tag, identity_scalar(numbered_identity(0)))
                .unwrap_err(),
            &format!("$.{name}"),
        );
        assert_eq!(held, before);
    }
    let mut expected = held.clone();
    expected
        .set(CODE_TAG_NAME.0, Scalar::from("chain"))
        .unwrap();
    located(
        held.set_many([
            (CODE_TAG_NAME.0, Scalar::from("chain")),
            (MSGPHASH_TAG_NAME.0, before.msgphash().clone()),
        ])
        .unwrap_err(),
        "$.msgphash",
    );
    assert_eq!(held, before);
    held.set_many([
        (MSGHASH_TAG_NAME.0, expected.msghash().clone()),
        (MSGPHASH_TAG_NAME.0, expected.msgphash().clone()),
        (CODE_TAG_NAME.0, Scalar::from("chain")),
    ])
    .unwrap();
    assert_eq!(
        held, expected,
        "assertions use the final candidate, not write order"
    );
    assert_mirrors(&held);
}

#[test]
fn mandatory_null_writes_and_removals_refuse_without_changing_any_state() {
    let mut held = message([payload(Scalar::from("first"))]);
    for tag in HELD {
        let before = held.clone();
        let name = held.as_field().fields()[position(&held, tag)]
            .name()
            .to_owned();
        if BUNDLE.contains(&tag) {
            located(
                held.set(tag, Scalar::Null).unwrap_err(),
                &format!("$.{name}"),
            );
            assert_eq!(held, before);
        }
        located(held.remove(tag).unwrap_err(), &format!("$.{name}"));
        assert_eq!(held, before);
        assert_mirrors(&held);
    }
    // The snapshot clock is held rather than mandatory: the column is not
    // removable, and clearing it is how a row says no snapshot took it. A
    // stamp and a clear return the row it started as, identity included.
    let before = held.clone();
    assert!(before.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap().is_null());
    held.set(SNAPSHOTAT_TAG_NAME.0, clock(CLOCK)).unwrap();
    assert_eq!(held.by_tag(SNAPSHOTAT_TAG_NAME.0).unwrap(), &clock(CLOCK));
    assert_ne!(held.msghash(), before.msghash());
    held.set(SNAPSHOTAT_TAG_NAME.0, Scalar::Null).unwrap();
    assert_eq!(held, before);
    let old = held.msghash().clone();
    assert_eq!(held.remove("payload").unwrap(), Some(Scalar::from("first")));
    assert_ne!(held.msghash(), &old);
    let before = held.clone();
    assert_eq!(held.remove("absent").unwrap(), None);
    assert_eq!(held, before);
}

#[test]
fn mandatory_names_numeric_spellings_and_renamed_tags_resolve_once_in_place() {
    let original = message([]);
    for tag in HELD {
        for style in 0..3 {
            let at = position(&original, tag);
            let mut fields = original.as_field().fields().to_vec();
            let mut values = original.as_value().as_sequence().unwrap().to_vec();
            values[position(&original, MSGHASH_TAG_NAME.0)] = Scalar::Null;
            match style {
                0 => {
                    fields[at].remove_metadata("fix:tag");
                }
                1 => {
                    fields[at].set_name(tag.to_string());
                    fields[at].remove_metadata("fix:tag");
                }
                _ => fields[at].set_name("renamed_role"),
            }
            let name = fields[at].name().to_owned();
            let held = FixMsg::with_registry(
                Arc::clone(original.registry()),
                root(fields),
                Scalar::from_sequence(values),
            )
            .unwrap();
            assert_eq!(position(&held, tag), at);
            assert_eq!(held.as_field().fields()[at].name(), name);
            assert_eq!(
                held.as_field().fields().len(),
                original.as_field().fields().len()
            );
            assert_eq!(
                held.as_field().fields()[at].as_fix().tag().unwrap(),
                Some(tag)
            );
            assert_mirrors(&held);
            assert_eq!(held.msghash(), &expected_msghash(&held));
        }
    }
}

#[test]
fn mandatory_registered_alias_plans_are_scoped_to_the_resolving_registry() {
    let mut first = FixRegistry::new();
    let mut updated = declared(&first, UPDATEDAT_TAG_NAME.0);
    updated.as_fix_mut().set_names(["clock_alias"]).unwrap();
    first.update(updated).unwrap();
    let mut second = FixRegistry::new();
    let mut snapshot = declared(&second, SNAPSHOTAT_TAG_NAME.0);
    snapshot.as_fix_mut().set_names(["clock_alias"]).unwrap();
    second.update(snapshot).unwrap();
    let field = root([
        declared(&first, 52),
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::UTC,
        }
        .required_field("clock_alias"),
    ]);
    let value = Scalar::from_sequence([clock(CLOCK), clock(7)]);
    let first = FixMsg::with_registry(Arc::new(first), field.clone(), value.clone()).unwrap();
    let second = FixMsg::with_registry(Arc::new(second), field, value).unwrap();
    assert_eq!(
        first.as_field().fields()[1].as_fix().tag().unwrap(),
        Some(UPDATEDAT_TAG_NAME.0)
    );
    assert_eq!(
        second.as_field().fields()[1].as_fix().tag().unwrap(),
        Some(SNAPSHOTAT_TAG_NAME.0)
    );
    assert_eq!(first.createdat(), &clock(CLOCK));
    assert_eq!(second.createdat(), &clock(7));
    assert_mirrors(&first);
    assert_mirrors(&second);
}

#[test]
fn tagless_replay_holders_keep_resolved_roles_and_repeat_the_same_export() {
    let original = aliased_message();
    for style in 0..3 {
        let mut fields = original.as_field().fields().to_vec();
        for tag in HELD {
            let field = &mut fields[position(&original, tag)];
            match style {
                1 => field.set_name(tag.to_string()),
                2 => field.set_name(format!("alias_{tag}")),
                _ => {}
            }
            field.remove_metadata("fix:tag");
        }
        let schema = root(fields);
        let row = original.into_row(&schema).unwrap();
        let held = FixMsg::from_row(Arc::clone(original.registry()), &schema, &row).unwrap();
        for tag in HELD {
            let at = position(&original, tag);
            let known = declared(original.registry(), tag);
            let alias = format!("alias_{tag}");
            let expected = row.get(at);
            assert_eq!(held.get_by_tag(tag), expected, "style={style}, tag={tag}");
            assert_eq!(held.get_by_name(known.name()), expected);
            assert_eq!(held.get_by_name(&alias), expected);
            assert_eq!(
                held.get_by_id(known.as_fix().id().unwrap().unwrap()),
                expected
            );
            assert_eq!(
                held.as_field().fields()[at].name(),
                schema.fields()[at].name()
            );
        }
        assert!(std::ptr::eq(
            held.get_by_tag(UPDATEDAT_TAG_NAME.0).unwrap(),
            held.updatedat()
        ));
        assert!(std::ptr::eq(
            held.get_by_tag(MSGHASH_TAG_NAME.0).unwrap(),
            held.msghash()
        ));
        assert_eq!(held.into_row(&schema).unwrap(), row, "style={style}");
    }
}

#[test]
fn every_key_spelling_keeps_a_renamed_mandatory_holder_and_refuses_its_removal() {
    let original = aliased_message();
    for tag in HELD {
        let known = declared(original.registry(), tag);
        let alias = format!("alias_{tag}");
        let at = position(&original, tag);
        let mut fields = original.as_field().fields().to_vec();
        fields[at].set_name("holder");
        let mut values = original.as_value().as_sequence().unwrap().to_vec();
        values[position(&original, MSGHASH_TAG_NAME.0)] = Scalar::Null;
        let original = FixMsg::with_registry(
            Arc::clone(original.registry()),
            root(fields),
            Scalar::from_sequence(values),
        )
        .unwrap();
        let id = known.as_fix().id().unwrap().unwrap();
        for key in [
            FixKey::Tag(tag),
            FixKey::Name(known.name()),
            FixKey::Name(&alias),
            FixKey::Id(id),
        ] {
            let mut held = original.clone();
            held.set(key, original.by_tag(tag).unwrap().clone())
                .unwrap();
            assert_eq!(
                held, original,
                "a same-value write preserves the holder: {key}"
            );
            assert_eq!(position(&held, tag), at);
            assert_eq!(held.as_field().fields()[at].name(), "holder");
            assert_eq!(held.get_by_name(known.name()), held.get_by_tag(tag));
            assert_eq!(held.get_by_name(&alias), held.get_by_tag(tag));
            assert_eq!(held.get_by_id(id), held.get_by_tag(tag));
            for name in [known.name(), alias.as_str(), "holder"] {
                let path = yggdryl::FieldPath::from_str(name).unwrap();
                assert_eq!(held.get_by_path(&path), held.get_by_tag(tag));
            }
            located(held.remove(key).unwrap_err(), "$.holder");
            assert_eq!(held, original, "removal is atomic: {key}");
            if BUNDLE.contains(&tag) {
                located(held.set(key, Scalar::Null).unwrap_err(), "$.holder");
                assert_eq!(held, original, "null refusal is atomic: {key}");
            }
            assert_mirrors(&held);
        }
    }
}

#[test]
fn conflicting_malformed_duplicate_and_mistyped_mandatory_declarations_refuse() {
    let original = message([]);
    let at = position(&original, MSGHASH_TAG_NAME.0);
    let mut fields = original.as_field().fields().to_vec();
    fields[at]
        .as_fix_mut()
        .set_tag(MSGPHASH_TAG_NAME.0)
        .unwrap();
    located(
        FixMsg::with_registry(
            Arc::clone(original.registry()),
            root(fields),
            original.as_value().clone(),
        )
        .unwrap_err(),
        "$.msghash",
    );
    let mut fields = original.as_field().fields().to_vec();
    fields[at].insert_metadata("fix:tag", "not-a-tag").unwrap();
    let error = FixMsg::with_registry(
        Arc::clone(original.registry()),
        root(fields),
        original.as_value().clone(),
    )
    .unwrap_err();
    assert!(
        matches!(error, Error::InvalidMetadataValue { .. }),
        "{error}"
    );
    for tag in HELD {
        let at = position(&original, tag);
        for value in [Scalar::Null, original.as_value().get(at).unwrap().clone()] {
            let mut fields = original.as_field().fields().to_vec();
            fields.push(fields[at].clone().with_name("duplicate"));
            let mut values = original.as_value().as_sequence().unwrap().to_vec();
            values.push(value);
            located(
                FixMsg::with_registry(
                    Arc::clone(original.registry()),
                    root(fields),
                    Scalar::from_sequence(values),
                )
                .unwrap_err(),
                "$.duplicate",
            );
        }
        let mut fields = original.as_field().fields().to_vec();
        let name = fields[at].name().to_owned();
        fields[at].set_dtype(DataType::Int64).unwrap();
        located(
            FixMsg::with_registry(
                Arc::clone(original.registry()),
                root(fields),
                original.as_value().clone(),
            )
            .unwrap_err(),
            &format!("$.{name}"),
        );
    }
}

#[test]
fn replay_bundle_is_required_before_record_defaults_can_supply_a_value() {
    let original = message([payload(Scalar::from("value"))]);
    for tag in HELD {
        let at = position(&original, tag);
        let name = original.as_field().fields()[at].name();
        let path = format!("$.{name}");
        let mut fields = original.as_field().fields().to_vec();
        let mut values = original.as_value().as_sequence().unwrap().to_vec();
        fields.remove(at);
        values.remove(at);
        let schema = root(fields);
        located(original.into_row(&schema).unwrap_err(), &path);
        located(
            FixMsg::from_row(
                Arc::clone(original.registry()),
                &schema,
                &Scalar::from_sequence(values),
            )
            .unwrap_err(),
            &path,
        );
        // The rest is about the *value*, which the snapshot clock need not
        // have: a nullable column holding null is exactly how a row that no
        // snapshot took says so.
        if !BUNDLE.contains(&tag) {
            continue;
        }
        let mut fields = original.as_field().fields().to_vec();
        fields[at].set_nullable(true);
        located(
            FixMsg::from_row(
                Arc::clone(original.registry()),
                &root(fields),
                original.as_value(),
            )
            .unwrap_err(),
            &path,
        );
        let mut values = original.as_value().as_sequence().unwrap().to_vec();
        values[at] = Scalar::Null;
        located(
            FixMsg::from_row(
                Arc::clone(original.registry()),
                original.as_field(),
                &Scalar::from_sequence(values),
            )
            .unwrap_err(),
            &path,
        );
        let missing = Scalar::from_record(
            original
                .as_field()
                .fields()
                .iter()
                .zip(original.as_value().as_sequence().unwrap())
                .enumerate()
                .filter(|(index, _)| *index != at)
                .map(|(_, (field, value))| (field.name(), value.clone())),
        )
        .unwrap();
        located(
            FixMsg::from_row(
                Arc::clone(original.registry()),
                original.as_field(),
                &missing,
            )
            .unwrap_err(),
            &path,
        );
    }
}

#[test]
fn replay_refuses_tampered_identity_and_non_native_mandatory_values() {
    let original = message([]);
    for tag in HELD {
        let at = position(&original, tag);
        let mut values = original.as_value().as_sequence().unwrap().to_vec();
        values[at] = match tag {
            tag if tag == MSGHASH_TAG_NAME.0 || tag == MSGPHASH_TAG_NAME.0 => {
                identity_scalar(numbered_identity(0))
            }
            tag if tag == CODE_TAG_NAME.0 => Scalar::from(7_i32),
            _ => Scalar::datetime64(0, TimeUnit::Microsecond, Timezone::UTC).unwrap(),
        };
        located(
            FixMsg::from_row(
                Arc::clone(original.registry()),
                original.as_field(),
                &Scalar::from_sequence(values),
            )
            .unwrap_err(),
            &format!("$.{}", original.as_field().fields()[at].name()),
        );
    }
}

#[test]
fn full_projection_preserves_identity_and_lossy_projection_stabilizes_on_second_export() {
    let original = message([payload(Scalar::from("value"))]);
    let full = original.into_row(original.as_field()).unwrap();
    assert_eq!(&full, original.as_value());
    let restored =
        FixMsg::from_row(Arc::clone(original.registry()), original.as_field(), &full).unwrap();
    assert_eq!(restored, original);
    let narrow = root(
        original
            .as_field()
            .fields()
            .iter()
            .filter(|field| field.name() != "payload")
            .cloned(),
    );
    let first = original.into_row(&narrow).unwrap();
    let held = FixMsg::from_row(Arc::clone(original.registry()), &narrow, &first).unwrap();
    assert_ne!(held.msghash(), original.msghash());
    assert_eq!(held.msgphash(), original.msgphash());
    assert_eq!(held.into_row(&narrow).unwrap(), first);
    assert_eq!(
        original.into_row(original.as_field()).unwrap(),
        full,
        "source unchanged"
    );
    assert_eq!(held.msghash(), &expected_msghash(&held));
    let padded = root(
        original
            .as_field()
            .fields()
            .iter()
            .cloned()
            .chain([DataType::utf8().nullable_field("padding")]),
    );
    let first = original.into_row(&padded).unwrap();
    let held = FixMsg::from_row(Arc::clone(original.registry()), &padded, &first).unwrap();
    assert_ne!(held.msghash(), original.msghash());
    assert_eq!(held.into_row(&padded).unwrap(), first);
}

#[test]
fn projection_canonicalizes_actual_values_and_propagates_unrepresentable_values() {
    let mut field = DataType::Int64.required_field("quantity");
    field.as_fix_mut().set_tag(9001).unwrap();
    let original = message([(field, Scalar::from(300_i64))]);
    let at = original.as_field().index_of("quantity").unwrap();
    let mut fields = original.as_field().fields().to_vec();
    fields[at].set_dtype(DataType::Int32).unwrap();
    let schema = root(fields.clone());
    let row = original.into_row(&schema).unwrap();
    assert_eq!(row.get(at).unwrap().dtype().unwrap(), DataType::Int32);
    let held = FixMsg::from_row(Arc::clone(original.registry()), &schema, &row).unwrap();
    assert_eq!(
        held.msghash(),
        original.msghash(),
        "equal exact-width values share the canonical feed"
    );
    assert_eq!(held.into_row(&schema).unwrap(), row);
    fields[at].set_dtype(DataType::Int8).unwrap();
    let expected = fields[at].scalar(Scalar::from(300_i64)).unwrap_err();
    let error = original.into_row(&root(fields)).unwrap_err();
    assert_eq!(error.to_string(), expected.to_string());
}

#[test]
fn projection_nulls_a_stated_list_in_a_string_column_without_losing_the_source() {
    let codec = super::fixed_codec(super::committed_registry());
    let source = b"8=FIX.4.4|35=D|11=A|Symbol[0]=ALPHA|Symbol[1]=BETA|10=0|";
    let message = codec.sole_line(source, false).unwrap();
    let original = message.clone();
    let symbols = message.by_tag(55).unwrap();
    assert_eq!(
        symbols.as_sequence().unwrap(),
        &[Scalar::from("ALPHA"), Scalar::from("BETA")]
    );
    let row = message.into_row(message.as_field()).unwrap();
    let schema = yggdryl::fix_schema(codec.registry(), "fix").unwrap();

    // Two symbols are no symbol a single column can answer with, so the
    // fixed row says so with a null rather than ending the capture on it.
    let projected = message.into_row(&schema).unwrap();
    let at = yggdryl::fix_column_of(&schema, 55).expect("a symbol column");
    assert!(projected.as_sequence().unwrap()[at].is_null());

    // Nothing was taken from the message to say it: the source is unchanged,
    // the line is rebuilt byte for byte, and the arrival record the row
    // carries still holds both spellings.
    assert_eq!(message, original);
    assert_eq!(message.into_bytes(b'|'), source);
    assert_eq!(message.into_row(message.as_field()).unwrap(), row);
    let entries = projected.as_sequence().unwrap().last().unwrap();
    let spelled: Vec<&str> = entries
        .as_sequence()
        .unwrap()
        .iter()
        .filter_map(|entry| entry.get(3).and_then(Scalar::as_str))
        .collect();
    assert!(
        spelled.contains(&"Symbol[0]") && spelled.contains(&"Symbol[1]"),
        "{spelled:?}"
    );
}

#[test]
fn fresh_intake_and_replay_refuse_non_struct_roots_before_settling_fields() {
    let registry = Arc::new(FixRegistry::new());
    let list = DataType::list(DataType::Int64.required_field("item"));
    for (dtype, value) in [
        (DataType::Int64, Scalar::from(1_i64)),
        (list.clone(), Scalar::from_sequence(Vec::<Scalar>::new())),
        (list, Scalar::from_sequence([Scalar::from(1_i64)])),
    ] {
        let field = dtype.required_field("event");
        for error in [
            FixMsg::with_registry(Arc::clone(&registry), field.clone(), value.clone()).unwrap_err(),
            FixMsg::from_row(Arc::clone(&registry), &field, &value).unwrap_err(),
        ] {
            located(error, "$.event");
        }
    }
}

#[test]
fn projection_restates_temporal_parameters_even_when_the_datatype_id_matches() {
    let mut field = DataType::DateTime64 {
        unit: TimeUnit::Nanosecond,
        timezone: Timezone::UTC,
    }
    .required_field("otherclock");
    field.as_fix_mut().set_tag(9002).unwrap();
    let original = message([(field, clock(2_000_000))]);
    let at = original.as_field().index_of("otherclock").unwrap();
    let mut fields = original.as_field().fields().to_vec();
    fields[at]
        .set_dtype(DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone: Timezone::UTC,
        })
        .unwrap();
    let schema = root(fields);
    let row = original.into_row(&schema).unwrap();
    assert_eq!(
        row.get(at).unwrap().as_datetime64(),
        Some((2_000, TimeUnit::Microsecond, &Timezone::UTC))
    );
    let held = FixMsg::from_row(Arc::clone(original.registry()), &schema, &row).unwrap();
    assert_eq!(held.msghash(), &expected_msghash(&held));
    assert_eq!(held.into_row(&schema).unwrap(), row);
}

#[test]
fn enrichment_keeps_settled_clocks_and_arrival_record_and_finalizes_once_per_result() {
    let codec = FixCodec::new(super::committed_registry())
        .try_with_default_sending_time(Some(clock(CLOCK)))
        .unwrap();
    let raw = codec
        .sole_line(b"8=FIX.4.4|35=D|11=A1|55=ALPHA|54=1|10=0|", false)
        .unwrap();
    let enriched = codec.enrich_message(raw.clone()).unwrap();
    assert_eq!(enriched.updatedat(), raw.updatedat());
    assert_eq!(enriched.createdat(), raw.createdat());
    assert_eq!(enriched.by_tag(52).unwrap(), raw.by_tag(52).unwrap());
    assert_eq!(enriched.entries(), raw.entries());
    assert_eq!(enriched.into_bytes(b'|'), raw.into_bytes(b'|'));
    assert_eq!(enriched.digest(), raw.digest());
    assert_eq!(enriched.msghash(), &expected_msghash(&enriched));
    assert_eq!(codec.enrich_message(enriched.clone()).unwrap(), enriched);
    assert_mirrors(&enriched);
}
