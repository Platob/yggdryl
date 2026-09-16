//! One arrival tree, with zero reserved for unresolved keys.

use std::sync::Arc;

use super::SoleMessage;
use yggdryl::hashing::xxhash::xxh128;
use yggdryl::media::text::TextBytes;
use yggdryl::{DataType, Error, Field, FixCategory, FixEntry, FixRegistry, Scalar};

fn tagged(name: &str, tag: i32) -> Field {
    let mut field = DataType::utf8().nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

#[test]
fn registry_tag_writers_refuse_nonpositive_values_atomically() {
    let mut field = tagged("positive", 1);
    field.as_fix_mut().set_counter(2).unwrap();
    field.as_fix_mut().set_tags(&[3, i32::MAX]).unwrap();
    let before = field.clone();
    for tag in [0, -1, i32::MIN] {
        for (key, result) in [
            ("fix:tag", field.as_fix_mut().set_tag(tag)),
            ("fix:counter", field.as_fix_mut().set_counter(tag)),
            ("fix:tags", field.as_fix_mut().set_tags(&[4, tag])),
        ] {
            let error = result.unwrap_err();
            assert!(
                matches!(&error, Error::InvalidMetadataValue { key: actual, .. } if actual == key),
                "{error}"
            );
            assert!(
                error.to_string().contains("from 1 to 2147483647"),
                "{error}"
            );
            assert_eq!(field, before, "{key}={tag}");
        }
    }
    field.as_fix_mut().set_tag(i32::MAX).unwrap();
    field.as_fix_mut().set_counter(i32::MAX).unwrap();
    assert_eq!(field.as_fix().tag().unwrap(), Some(i32::MAX));
    assert_eq!(field.as_fix().counter().unwrap(), Some(i32::MAX));
    field.as_fix_mut().set_tags(&[]).unwrap();
    assert!(field.get_metadata("fix:tags").is_none());
}

#[test]
fn externally_stated_zero_identity_is_refused_without_mutating_the_registry() {
    // The alternates are one JSON array, and their elements are held to the
    // same shape as the two scalar tags.
    for key in ["fix:tag", "fix:counter", "fix:tags"] {
        for digits in ["0", "000", "-1", "+1", "2147483648"] {
            let text = if key == "fix:tags" {
                format!("[{digits}]")
            } else {
                digits.to_owned()
            };
            let mut field = tagged("incoming", 90_001);
            field.insert_metadata(key, text.as_str()).unwrap();
            let error = match key {
                "fix:tag" => field.as_fix().tag().unwrap_err(),
                "fix:counter" => field.as_fix().counter().unwrap_err(),
                _ => field.as_fix().tags().unwrap_err(),
            };
            assert!(
                matches!(&error, Error::InvalidMetadataValue { key: actual, .. } if actual == key),
                "{key}={text}: {error}"
            );
            let mut registry = FixRegistry::new();
            let before = registry.clone();
            assert!(registry.insert(field).is_err(), "{key}={text}");
            assert_eq!(registry, before, "{key}={text}");
        }
    }
    // Leading zeros are still the tag on the two bare decimals, and never in
    // the array: a JSON number spells none, and the array is JSON.
    let mut field = tagged("positive", 1);
    for key in ["fix:tag", "fix:counter"] {
        field.insert_metadata(key, "0001").unwrap();
    }
    field.insert_metadata("fix:tags", "[1]").unwrap();
    assert_eq!(field.as_fix().tag().unwrap(), Some(1));
    assert_eq!(field.as_fix().counter().unwrap(), Some(1));
    assert_eq!(field.as_fix().tags().unwrap(), [1]);
    field.insert_metadata("fix:tags", "[0001]").unwrap();
    let error = field.as_fix().tags().unwrap_err();
    assert!(
        matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "fix:tags"),
        "{error}"
    );
}

#[test]
fn unresolved_arrivals_keep_raw_bytes_order_and_dynamic_columns() {
    let codec = super::fixed_codec(super::committed_registry());
    let wire = b"35=D|999999=one|0999999=two|OwnThing=three|0=zero|2147483648=wide|55=SYNTH|10=0|";
    let message = codec.sole_line(wire, false).unwrap();
    assert_eq!(message.entries().len(), 8);
    assert_eq!(
        message
            .entries()
            .iter()
            .map(FixEntry::tag)
            .collect::<Vec<_>>(),
        [35, 0, 0, 0, 0, 0, 55, 10]
    );
    for (entry, (key, value)) in message.entries()[1..6].iter().zip([
        ("999999", "one"),
        ("0999999", "two"),
        ("OwnThing", "three"),
        ("0", "zero"),
        ("2147483648", "wide"),
    ]) {
        assert_eq!(entry.key().as_bytes(), key.as_bytes());
        assert_eq!(entry.value().as_bytes(), value.as_bytes());
        assert!(entry.children().is_empty());
        assert_eq!(
            message.get_by_name(key).and_then(Scalar::as_str),
            Some(value)
        );
    }
    assert_eq!(
        message.get_by_tag(999_999).and_then(Scalar::as_str),
        Some("one")
    );
    assert_eq!(message.into_bytes(b'|'), wire);

    let entry = FixEntry::new(
        0,
        TextBytes::from_bytes("999999").unwrap(),
        TextBytes::from_bytes("one").unwrap(),
    );
    assert_eq!(entry, message.entries()[1]);
    let lossy = codec.sole_line(b"35=D|999999=\xff|10=0|", false).unwrap();
    assert!(matches!(
        lossy.anomalies().next(),
        Some(yggdryl::FixAnomaly::Lossy {
            tag: 0,
            key: "999999"
        })
    ));
    assert_eq!(lossy.into_bytes(b'|'), b"35=D|999999=\xff|10=0|");

    // Signed keys are not part of the line scanner's grammar. Native pairs
    // reach the builder directly, where neither sign becomes a numeric tag.
    let pairs = codec
        .parse_pairs(
            [("-1", "negative"), ("+35", "signed")]
                .map(|(key, value)| (key.as_bytes(), value.as_bytes())),
        )
        .unwrap();
    assert_eq!(pairs.entries().len(), 2);
    for (entry, (key, value)) in pairs
        .entries()
        .iter()
        .zip([("-1", "negative"), ("+35", "signed")])
    {
        assert_eq!(entry.tag(), 0);
        assert_eq!(entry.key().as_str(), Some(key));
        assert_eq!(entry.value().as_str(), Some(value));
        assert_eq!(pairs.get_by_name(key).and_then(Scalar::as_str), Some(value));
    }
    assert_eq!(pairs.into_bytes(b'|'), b"-1=negative|+35=signed|");
}

#[test]
fn indexed_unknowns_keep_each_arrival_and_their_existing_value_shape() {
    let codec = super::fixed_codec(super::committed_registry());
    let message = codec
        .parse_pairs(
            [("999999[0]", "first"), ("999999[2]", "third")]
                .map(|(key, value)| (key.as_bytes(), value.as_bytes())),
        )
        .unwrap();
    assert_eq!(message.entries().len(), 2);
    for (entry, key) in message.entries().iter().zip(["999999[0]", "999999[2]"]) {
        assert_eq!(entry.tag(), 0);
        assert_eq!(entry.key().as_str(), Some(key));
    }
    assert_eq!(
        message.get_by_name("999999").unwrap(),
        &Scalar::from_sequence([Scalar::from("first"), Scalar::Null, Scalar::from("third")])
    );
}

#[test]
fn a_group_keeps_resolved_members_and_unknown_children_under_the_stated_counter() {
    let mut registry = FixRegistry::new();
    let mut counter = DataType::Int32.nullable_field("norows");
    counter.as_fix_mut().set_tag(90_001).unwrap();
    registry.insert(counter).unwrap();
    let mut group = DataType::list(
        DataType::from_fields([tagged("scopedvalue", 90_002)])
            .unwrap()
            .required_field("row"),
    )
    .nullable_field("rows");
    group.as_fix_mut().set_counter(90_001).unwrap();
    registry
        .create_definition(FixCategory::Groups, group)
        .unwrap();
    assert!(registry.get_field_by_tag(90_002).is_none());
    let codec = super::fixed_codec(Arc::new(registry));
    let message = codec
        .parse_pairs(
            [
                ("NoRows", "1"),
                ("Rows[0].ScopedValue", "known"),
                ("Rows[0].999999", "numeric"),
                ("Rows[0].OwnThing", "named"),
            ]
            .map(|(key, value)| (key.as_bytes(), value.as_bytes())),
        )
        .unwrap();
    assert_eq!(message.entries().len(), 1);
    let counter = &message.entries()[0];
    assert_eq!(counter.tag(), 90_001);
    assert_eq!(counter.children().len(), 3);
    assert_eq!(
        counter
            .children()
            .iter()
            .map(FixEntry::tag)
            .collect::<Vec<_>>(),
        [90_002, 0, 0]
    );
    for (entry, key) in
        counter
            .children()
            .iter()
            .zip(["Rows[0].ScopedValue", "Rows[0].999999", "Rows[0].OwnThing"])
    {
        assert_eq!(entry.key().as_str(), Some(key));
    }
    for (path, expected) in [
        ("rows[0].scopedvalue", "known"),
        ("rows[0].\"999999\"", "numeric"),
        ("rows[0].ownthing", "named"),
    ] {
        assert_eq!(
            message
                .get_by_path(&super::path(path))
                .and_then(Scalar::as_str),
            Some(expected),
            "{path}"
        );
    }
    assert_eq!(
        message.into_bytes(b'|'),
        b"NoRows=1|Rows[0].ScopedValue=known|Rows[0].999999=numeric|Rows[0].OwnThing=named|"
    );
}

#[test]
fn numeric_and_named_aliases_keep_canonical_positive_arrival_tags() {
    let mut registry = super::committed_registry().as_ref().clone();
    let mut symbol = registry.field_by_tag(55).unwrap().clone();
    symbol.as_fix_mut().set_tags(&[9_000_001]).unwrap();
    symbol
        .as_fix_mut()
        .set_names(["SyntheticSymbol"])
        .unwrap();
    registry.insert(symbol).unwrap();
    let codec = super::fixed_codec(Arc::new(registry));
    let canonical = codec.sole_line(b"35=D|55=SYNTH|10=0|", false).unwrap();
    for key in ["55", "00055", "9000001", "SyntheticSymbol"] {
        let wire = format!("35=D|{key}=SYNTH|10=0|");
        let message = codec.sole_line(wire.as_bytes(), false).unwrap();
        let entry = &message.entries()[1];
        assert_eq!(entry.tag(), 55, "{key}");
        assert_eq!(entry.key().as_str(), Some(key));
        assert_eq!(message.digest(), canonical.digest(), "{key}");
        assert_eq!(message.into_bytes(b'|'), wire.as_bytes());
    }
}

#[test]
fn an_intrinsic_map_has_no_numeric_scalar_wire_grammar() {
    let codec = super::fixed_codec(super::committed_registry());
    let wire = b"35=D|65020=opaque|10=0|";
    let message = codec.sole_line(wire, false).unwrap();
    assert_eq!(message.entries()[1].tag(), 0);
    assert_eq!(message.entries()[1].key().as_str(), Some("65020"));
    assert_eq!(
        message.get_by_name("65020").and_then(Scalar::as_str),
        Some("opaque")
    );
    assert!(message.get_by_name("altids").is_none());
    assert_eq!(message.into_bytes(b'|'), wire);
}

#[test]
fn unknown_numeric_digests_include_the_raw_key_in_the_existing_zero_tag_frame() {
    let codec = super::fixed_codec(super::committed_registry());
    let mut digests = Vec::new();
    for key in ["999999", "0999999", "999998", "OwnThing"] {
        let message = codec
            .parse_pairs([(key.as_bytes(), b"x".as_slice())])
            .unwrap();
        assert_eq!(message.entries().len(), 1);
        assert_eq!(message.entries()[0].tag(), 0);
        // Independent framing: tag, key length/key, value length/value, children.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0_i32.to_be_bytes());
        bytes.extend_from_slice(&(key.len() as u32).to_be_bytes());
        bytes.extend_from_slice(key.as_bytes());
        bytes.extend_from_slice(&1_u32.to_be_bytes());
        bytes.push(b'x');
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        assert_eq!(message.digest(), xxh128(&bytes), "{key}");
        assert!(!digests.contains(&message.digest()), "{key}");
        digests.push(message.digest());
    }
}

#[test]
fn unresolved_counters_keep_their_members_in_arrival_order() {
    let codec = super::fixed_codec(super::committed_registry());
    let pairs = |pairs: &[(&'static str, &'static str)]| {
        codec
            .parse_pairs(
                pairs
                    .iter()
                    .map(|(key, value)| (key.as_bytes(), value.as_bytes())),
            )
            .unwrap()
    };
    let shape = |entries: &[FixEntry]| {
        fn walk(entries: &[FixEntry], out: &mut Vec<(usize, i32, String)>, depth: usize) {
            for entry in entries {
                out.push((depth, entry.tag(), entry.key().as_str().unwrap().to_owned()));
                walk(entry.children(), out, depth + 1);
            }
        }
        let mut out = Vec::new();
        walk(entries, &mut out, 0);
        out
    };

    // At the top: an unregistered numeric counter heads what arrived under it.
    let top = pairs(&[
        ("999999", "2"),
        ("999999[0].OwnThing", "a"),
        ("999999[1].999998", "b"),
    ]);
    assert_eq!(
        shape(top.entries()),
        [
            (0, 0, "999999".to_owned()),
            (1, 0, "999999[0].OwnThing".to_owned()),
            (1, 0, "999999[1].999998".to_owned()),
        ]
    );
    assert_eq!(
        top.into_bytes(b'|'),
        b"999999=2|999999[0].OwnThing=a|999999[1].999998=b|"
    );

    // Inside a resolved group: the unresolved sub-counter keeps its member,
    // and the resolved member after it still follows it on the wire.
    let nested = pairs(&[
        ("NoPartyIDs", "1"),
        ("NoPartyIDs[0].999999", "1"),
        ("NoPartyIDs[0].999999[0].999998", "A"),
        ("NoPartyIDs[0].PartyRole", "3"),
    ]);
    assert_eq!(
        shape(nested.entries()),
        [
            (0, 453, "NoPartyIDs".to_owned()),
            (1, 0, "NoPartyIDs[0].999999".to_owned()),
            (2, 0, "NoPartyIDs[0].999999[0].999998".to_owned()),
            (1, 452, "NoPartyIDs[0].PartyRole".to_owned()),
        ]
    );
    assert_eq!(
        nested.into_bytes(b'|'),
        b"NoPartyIDs=1|NoPartyIDs[0].999999=1|NoPartyIDs[0].999999[0].999998=A|NoPartyIDs[0].PartyRole=3|"
    );
}

#[test]
fn envelope_exclusion_reads_resolved_tags_only() {
    let digest = |registry: Arc<FixRegistry>, sequence: &str| {
        super::fixed_codec(registry)
            .parse_pairs([
                (b"34".as_slice(), sequence.as_bytes()),
                (b"11".as_slice(), b"A".as_slice()),
            ])
            .unwrap()
            .digest()
    };
    // A dictionary defining MsgSeqNum resolves 34 and leaves it out.
    let committed = super::committed_registry();
    assert_eq!(
        digest(Arc::clone(&committed), "7"),
        digest(Arc::clone(&committed), "8")
    );
    // A bare registry does not: 34 is an unresolved arrival, hashed by its key.
    let bare = Arc::new(FixRegistry::new());
    assert_ne!(digest(Arc::clone(&bare), "7"), digest(bare, "8"));
}
