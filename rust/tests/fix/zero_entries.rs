//! One arrival tree, with zero reserved for unresolved keys.

use std::sync::Arc;

use super::SoleMessage;
use yggdryl::graph::Element;
use yggdryl::hashing::xxhash::xxh128;
use yggdryl::{DataType, Error, Field, FixEntry, FixRegistry, Scalar};

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
fn unresolved_arrivals_keep_their_keys_order_and_dynamic_columns() {
    let codec = super::fixed_codec(super::committed_registry());
    let wire = b"35=D|999999=one|0999999=two|OwnThing=three|0=zero|2147483648=wide|55=SYNTH|10=0|";
    let message = codec.sole_line(wire).unwrap();
    // The type is the header's, never an entry; the dictionary's own
    // `TimeInForce` closes the order's entries.
    assert_eq!(
        message
            .entries()
            .iter()
            .map(FixEntry::tag)
            .collect::<Vec<_>>(),
        [0, 0, 0, 0, 0, 55, 10, 59]
    );
    // An unresolved key is an entry under its own spelling, folded as
    // every name is, and a child of the row under it.
    for (entry, (key, folded, value)) in message.entries()[..5].iter().zip([
        ("999999", "999999", "one"),
        ("0999999", "0999999", "two"),
        ("OwnThing", "ownthing", "three"),
        ("0", "0", "zero"),
        ("2147483648", "2147483648", "wide"),
    ]) {
        assert_eq!(entry.name(), folded);
        assert_eq!(entry.value(), Some(value));
        assert!(entry.entries().is_empty());
        assert_eq!(
            message.get_by_name(key).as_ref().and_then(Scalar::as_str),
            Some(value)
        );
    }
    assert_eq!(
        message
            .get_by_tag(999_999)
            .as_ref()
            .and_then(Scalar::as_str),
        Some("one")
    );
    assert_eq!(
        message.into_bytes(b'|'),
        b"8=FIX.4.4|35=D|999999=one|0999999=two|ownthing=three|0=zero|2147483648=wide|55=SYNTH|10=0|59=0|"
    );
    let entry = FixEntry::new(0, "999999", Some("one".into()));
    assert_eq!(entry, message.entries()[0]);
}

/// A value no text holds still arrives: the row spells it as the decode a
/// column can read, and never as nothing.
#[test]
fn an_unresolved_value_that_is_not_text_is_kept_as_its_decode() {
    let codec = super::fixed_codec(super::committed_registry());
    let lossy = codec.sole_line(b"35=D|999999=\xff|10=0|").unwrap();
    assert_eq!(
        lossy
            .get_by_name("999999")
            .as_ref()
            .and_then(Scalar::as_str),
        Some("\u{FFFD}")
    );
    assert_eq!(lossy.entries()[0].value(), Some("\u{FFFD}"));
}

/// Signed keys are not part of the line scanner's grammar. Native pairs
/// reach the builder directly, where neither sign becomes a numeric tag.
#[test]
fn a_signed_key_names_no_tag() {
    let codec = super::fixed_codec(super::committed_registry());
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
        assert_eq!(entry.tag(), 0, "{key}");
        assert_eq!(entry.name(), key);
        assert_eq!(entry.value(), Some(value));
        assert_eq!(
            pairs.get_by_name(key).as_ref().and_then(Scalar::as_str),
            Some(value)
        );
    }
    assert_eq!(pairs.into_bytes(b'|'), b"8=FIX.4.4|-1=negative|+35=signed|");
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
    // One child, a list, read as one entry heading an entry per stated
    // element.
    assert_eq!(
        message.get_by_name("999999").unwrap(),
        Scalar::from_sequence([Scalar::from("first"), Scalar::Null, Scalar::from("third")])
    );
    assert_eq!(message.entries().len(), 1);
    let list = &message.entries()[0];
    assert_eq!((list.tag(), list.name(), list.value()), (0, "999999", None));
    assert_eq!(
        list.entries()
            .iter()
            .map(|element| (element.tag(), element.name(), element.value()))
            .collect::<Vec<_>>(),
        [(0, "999999", Some("first")), (0, "999999", Some("third"))]
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
    registry.insert(group).unwrap();
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
    // The group is one entry under its counter, counting its occurrence;
    // the occurrence states nothing of its own and holds its members, the
    // resolved one under its tag and the unknown ones under their keys.
    assert_eq!(message.entries().len(), 1);
    let counter = &message.entries()[0];
    assert_eq!(
        (counter.tag(), counter.name(), counter.value()),
        (90_001, "rows", Some("1"))
    );
    assert_eq!(counter.entries().len(), 1);
    let occurrence = &counter.entries()[0];
    assert_eq!(occurrence.value(), None);
    assert_eq!(
        occurrence
            .entries()
            .iter()
            .map(|member| (member.tag(), member.name(), member.value()))
            .collect::<Vec<_>>(),
        [
            (90_002, "scopedvalue", Some("known")),
            (0, "999999", Some("numeric")),
            (0, "ownthing", Some("named")),
        ]
    );
    for (path, expected) in [
        ("rows[0].scopedvalue", "known"),
        ("rows[0].\"999999\"", "numeric"),
        ("rows[0].ownthing", "named"),
    ] {
        assert_eq!(
            message
                .get_by_path(&super::path(path))
                .as_ref()
                .and_then(Scalar::as_str),
            Some(expected),
            "{path}"
        );
    }
    assert_eq!(
        message.into_bytes(b'|'),
        b"8=FIX.4.4|90001=1|90002=known|999999=numeric|ownthing=named|"
    );
}

#[test]
fn numeric_and_named_aliases_reach_the_canonical_field_and_re_emit_its_tag() {
    let mut registry = super::committed_registry().as_ref().clone();
    let mut symbol = registry.field_by_tag(55).unwrap().clone();
    symbol.as_fix_mut().set_tags(&[9_000_001]).unwrap();
    symbol.as_fix_mut().set_names(["SyntheticSymbol"]).unwrap();
    registry.insert(symbol).unwrap();
    let codec = super::fixed_codec(Arc::new(registry));
    let canonical = codec.sole_line(b"35=D|55=SYNTH|10=0|").unwrap();
    for key in ["55", "00055", "9000001", "SyntheticSymbol"] {
        let wire = format!("35=D|{key}=SYNTH|10=0|");
        let message = codec.sole_line(wire.as_bytes()).unwrap();
        let entry = &message.entries()[0];
        assert_eq!(entry.tag(), 55, "{key}");
        assert_eq!(entry.name(), "symbol", "{key}");
        assert_eq!(message.digest(), canonical.digest(), "{key}");
        // Every spelling is one field, and one field re-emits under its
        // canonical tag.
        assert_eq!(
            message.into_bytes(b'|'),
            canonical.into_bytes(b'|'),
            "{key}"
        );
    }
}

/// A crate tag on the wire names a fact the message holds typed, and its
/// Map takes no scalar: the pair is kept as the unresolved arrival it is
/// rather than dropped.
#[test]
fn an_intrinsic_map_has_no_numeric_scalar_wire_grammar() {
    let codec = super::fixed_codec(super::committed_registry());
    let wire = b"35=D|65020=opaque|10=0|";
    let message = codec.sole_line(wire).unwrap();
    assert!(message.get_identifiers().is_empty());
    assert_eq!(
        message
            .get_by_name("65020")
            .as_ref()
            .and_then(Scalar::as_str),
        Some("opaque")
    );
    let entry = &message.entries()[0];
    assert_eq!(
        (entry.tag(), entry.name(), entry.value()),
        (0, "65020", Some("opaque"))
    );
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
        // Independent framing: tag, name length/name, value length/value,
        // children - the name under the fold every key resolves by.
        let name = message.entries()[0].name();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0_i32.to_be_bytes());
        bytes.extend_from_slice(&(name.len() as u32).to_be_bytes());
        bytes.extend_from_slice(name.as_bytes());
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
    let text = |message: &yggdryl::FixMsg, path: &str| {
        message
            .get_by_path(&super::path(path))
            .as_ref()
            .and_then(Scalar::as_str)
            .map(ToOwned::to_owned)
    };

    // At the top: an unregistered numeric counter heads what arrived under
    // it, each occurrence where its index put it.
    let top = pairs(&[
        ("999999", "2"),
        ("999999[0].OwnThing", "a"),
        ("999999[1].999998", "b"),
    ]);
    assert_eq!(text(&top, "999999[0].ownthing").as_deref(), Some("a"));
    assert_eq!(text(&top, "999999[1].\"999998\"").as_deref(), Some("b"));
    assert_eq!(text(&top, "999999[0].\"999998\""), None);

    // Inside a resolved group: the unresolved sub-counter keeps its member,
    // and the resolved member after it is still the occurrence's.
    let nested = pairs(&[
        ("NoPartyIDs", "1"),
        ("NoPartyIDs[0].999999", "1"),
        ("NoPartyIDs[0].999999[0].999998", "A"),
        ("NoPartyIDs[0].PartyRole", "3"),
    ]);
    assert_eq!(
        nested
            .get_by_path(&super::path("parties[0].partyrole"))
            .as_ref()
            .and_then(Scalar::as_i128),
        Some(3)
    );
    assert_eq!(
        text(&nested, "parties[0].\"999999\"[0].\"999998\"").as_deref(),
        Some("A")
    );
}

/// The session layer is the envelope, whatever dictionary reads it: a
/// header tag the dictionary does not type is still the header's.
#[test]
fn envelope_exclusion_reads_the_header_whatever_the_dictionary_holds() {
    let read = |registry: Arc<FixRegistry>, sequence: &str| {
        super::fixed_codec(registry)
            .parse_pairs([
                (b"34".as_slice(), sequence.as_bytes()),
                (b"11".as_slice(), b"A".as_slice()),
            ])
            .unwrap()
    };
    let committed = super::committed_registry();
    assert_eq!(
        read(Arc::clone(&committed), "7").digest(),
        read(Arc::clone(&committed), "8").digest()
    );
    // A bare registry types no `MsgSeqNum`, and the header still holds it:
    // the digest leaves it out, and the fact is not lost.
    let bare = Arc::new(FixRegistry::new());
    assert_eq!(
        read(Arc::clone(&bare), "7").digest(),
        read(Arc::clone(&bare), "8").digest()
    );
    assert_eq!(read(bare, "7").header().msgseqnum(), Some(7));
}
