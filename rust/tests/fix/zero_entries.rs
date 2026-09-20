//! One arrival tree, with zero reserved for unresolved keys.

use std::sync::Arc;

use super::SoleMessage;
use yggdryl::graph::Element;
use yggdryl::xxhash::xxh128;
use yggdryl::{DataType, Error, Field, FixEntry, FixRegistry, Scalar, StructType};

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
            ("FIX:tag", field.as_fix_mut().set_tag(tag)),
            ("FIX:counter", field.as_fix_mut().set_counter(tag)),
            ("FIX:tags", field.as_fix_mut().set_tags(&[4, tag])),
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
    assert!(field.get_metadata("FIX:tags").is_none());
}

#[test]
fn externally_stated_zero_identity_is_refused_without_mutating_the_registry() {
    // The alternates are one JSON array, and their elements are held to the
    // same shape as the two scalar tags.
    for key in ["FIX:tag", "FIX:counter", "FIX:tags"] {
        for digits in ["0", "000", "-1", "+1", "2147483648"] {
            let text = if key == "FIX:tags" {
                format!("[{digits}]")
            } else {
                digits.to_owned()
            };
            let mut field = tagged("incoming", 90_001);
            field.insert_metadata(key, text.as_str()).unwrap();
            let error = match key {
                "FIX:tag" => field.as_fix().tag().unwrap_err(),
                "FIX:counter" => field.as_fix().counter().unwrap_err(),
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
    for key in ["FIX:tag", "FIX:counter"] {
        field.insert_metadata(key, "0001").unwrap();
    }
    field.insert_metadata("FIX:tags", "[1]").unwrap();
    assert_eq!(field.as_fix().tag().unwrap(), Some(1));
    assert_eq!(field.as_fix().counter().unwrap(), Some(1));
    assert_eq!(field.as_fix().tags().unwrap(), [1]);
    field.insert_metadata("FIX:tags", "[0001]").unwrap();
    let error = field.as_fix().tags().unwrap_err();
    assert!(
        matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "FIX:tags"),
        "{error}"
    );
}

#[test]
fn unresolved_arrivals_keep_their_keys_order_and_dynamic_columns() {
    let codec = super::fixed_codec(super::committed_registry());
    let wire = b"35=D|999999=one|0999999=two|OwnThing=three|0=zero|2147483648=wide|55=SYNTH|10=0|";
    let message = codec.sole_line(wire).unwrap();
    // The type and the checksum are the frame's, never entries: the entries
    // are the content row alone, the day order the dictionary derives for
    // an order among them.
    assert_eq!(
        message
            .entries()
            .iter()
            .map(FixEntry::tag)
            .collect::<Vec<_>>(),
        [0, 0, 0, 0, 0, 55, 59]
    );
    // An unresolved key is an entry under its own spelling, folded as every
    // name is, and a child of the row under it.
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
        b"8=FIX.4.4|35=D|999999=one|0999999=two|ownthing=three|0=zero|2147483648=wide|55=SYNTH|59=0|10=0|"
    );
    let entry = FixEntry::new(0, "999999", Some("one".into()));
    assert_eq!(entry, message.entries()[0]);
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
        StructType::from_fields([tagged("scopedvalue", 90_002)])
            .map(DataType::from)
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
fn an_unresolved_counter_at_the_root_heads_what_arrived_under_it() {
    let codec = super::fixed_codec(super::committed_registry());
    let message = codec
        .parse_pairs(
            [
                ("999999", "2"),
                ("999999[0].OwnThing", "a"),
                ("999999[1].999998", "b"),
            ]
            .map(|(key, value)| (key.as_bytes(), value.as_bytes())),
        )
        .unwrap();
    let text = |path: &str| {
        message
            .get_by_path(&super::path(path))
            .as_ref()
            .and_then(Scalar::as_str)
            .map(ToOwned::to_owned)
    };
    // A group built from indexed keys is sorted by what each occurrence
    // states, so the one stating only the later member comes first: each
    // occurrence carries the members its own index stated, and no other's.
    assert_eq!(text("999999[1].ownthing").as_deref(), Some("a"));
    assert_eq!(text("999999[0].\"999998\"").as_deref(), Some("b"));
    assert_eq!(text("999999[1].\"999998\""), None);
}

#[test]
fn a_header_tag_is_the_headers_fact_and_stays_out_of_the_content_code() {
    let read = |sequence: &str| {
        super::fixed_codec(super::committed_registry())
            .parse_pairs([
                (b"34".as_slice(), sequence.as_bytes()),
                (b"11".as_slice(), b"A".as_slice()),
            ])
            .unwrap()
    };
    // The session layer is the envelope around what a message says, so the
    // sequence number is the header's own fact: it is no entry of the
    // content row, it goes back on the wire from the header, and it is the
    // frame's and not the content's - a capture logs one message at every
    // hop it passes, each hop framing it in a session of its own - so two
    // messages differing in it alone digest to one content code.
    assert_eq!(read("7").header().msgseqnum(), Some(7));
    assert_eq!(read("8").header().msgseqnum(), Some(8));
    assert!(!read("7").entries().iter().any(|entry| entry.tag() == 34));
    assert_eq!(read("7").into_bytes(b'|'), b"8=FIX.4.4|34=7|11=A|");
    assert_eq!(read("7").get_currhashcode(), read("8").get_currhashcode());
}
