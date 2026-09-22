//! `rust/src/fix/codes.rs`: one code set, named once, however many fields read
//! by it.
//!
//! A vocabulary is registry-owned: a field names the set it reads by and the
//! document lives once beside the fields. Every door here is one a caller has,
//! so this reaches the crate through `yggdryl::` alone.

use super::SoleMessage;
use super::committed_registry;
use super::fixed_codec;
use super::path;

use yggdryl::holder::Holder;
use yggdryl::local::Folder;
use yggdryl::{DataType, Error, Field, FixCategory, FixCode, FixRegistry};

/// One field reading by one named set, and the set beside it.
fn dictionary(name: &str, tag: i32, codes: &[FixCode]) -> (FixRegistry, Field) {
    let mut registry = FixRegistry::new();
    registry.set_codeset(name, codes).unwrap();
    let mut field = DataType::utf8().nullable_field(format!("field{tag}"));
    field.as_fix_mut().set_tag(tag).unwrap();
    field.as_fix_mut().set_codeset(name).unwrap();
    registry.insert(field.clone()).unwrap();
    (registry, field)
}

#[test]
fn a_field_may_not_read_by_a_set_the_dictionary_does_not_hold() {
    let mut registry = FixRegistry::new();
    let mut field = DataType::utf8().nullable_field("side");
    field.as_fix_mut().set_tag(54).unwrap();
    field.as_fix_mut().set_codeset("sidecodeset").unwrap();
    let refused = registry.insert(field.clone()).unwrap_err();
    assert!(
        matches!(&refused, Error::Absent { expected, path }
            if *expected == "codesets" && path == "sidecodeset"),
        "{refused}"
    );
    // And the refusal left nothing behind.
    assert!(registry.get_field_by_tag(54).is_none());

    registry
        .set_codeset("sidecodeset", &[FixCode::new("Buy", "1")])
        .unwrap();
    registry.insert(field).unwrap();
    assert_eq!(
        registry.codeset_of(registry.field_by_tag(54).unwrap()),
        registry.get_codeset("sidecodeset"),
    );
}

#[test]
fn one_set_is_named_once_however_many_fields_read_by_it() {
    let (mut registry, _) = dictionary("unitcodeset", 996, &[FixCode::new("Bbl", "Bbl")]);
    let mut other = DataType::utf8().nullable_field("legunitofmeasure");
    other.as_fix_mut().set_tag(999).unwrap();
    other.as_fix_mut().set_codeset("unitcodeset").unwrap();
    registry.insert(other).unwrap();

    assert_eq!(registry.codesets().len(), 2);
    for tag in [996, 999] {
        let set = registry
            .codeset_of(registry.field_by_tag(tag).unwrap())
            .expect("the set");
        assert_eq!(set.code_name("Bbl"), Some("Bbl"));
    }
    // Stating one more member is one edit, and both fields read it.
    registry
        .merge_codeset("unitcodeset", &[FixCode::new("Gal", "Gal")])
        .unwrap();
    for tag in [996, 999] {
        let set = registry
            .codeset_of(registry.field_by_tag(tag).unwrap())
            .expect("the set");
        assert_eq!(set.codes().count(), 2);
    }
}

#[test]
fn a_set_a_field_reads_by_is_not_one_a_removal_may_take_away() {
    let (mut registry, mut field) = dictionary("sidecodeset", 54, &[FixCode::new("Buy", "1")]);
    let refused = registry.remove_codeset("sidecodeset").unwrap_err();
    assert!(matches!(refused, Error::Conflict { .. }), "{refused}");
    assert!(registry.get_codeset("sidecodeset").is_some());

    // `update` folds rather than replaces, so the reference it dropped
    // would come back; the definition door is the one that replaces a
    // field whole.
    field.as_fix_mut().remove_codeset();
    registry
        .update_definition(FixCategory::Fields, field)
        .unwrap();
    let taken = registry.remove_codeset("sidecodeset").unwrap();
    assert_eq!(taken, Some(vec![FixCode::new("Buy", "1")]));
    assert!(registry.get_codeset("sidecodeset").is_none());
}

#[test]
fn merging_two_dictionaries_folds_their_sets_and_keeps_every_enrichment() {
    // What the specification says: the values, named and documented.
    let (mut held, _) = dictionary(
        "msgtypecodeset",
        35,
        &[
            FixCode::new("Heartbeat", "0").with_description("Heartbeat"),
            FixCode::new("6", "6"),
        ],
    );
    // What a venue's own dictionary says about the same set: one value it
    // alone declares, one spelling for a value both hold, and a real name
    // for the one the specification left standing for itself.
    let (venue, _) = dictionary(
        "msgtypecodeset",
        35,
        &[
            FixCode::new("HB", "0"),
            FixCode::new("IOI", "6"),
            FixCode::new("VenueOwn", "ZZ").with_description("A type only this venue sends"),
        ],
    );

    held.merge_with(&venue).unwrap();
    let set = held.codeset("msgtypecodeset").unwrap();
    assert_eq!(set.codes().count(), 3);
    // The held name leads and the incoming one reaches the same code.
    assert_eq!(set.code_name("0"), Some("Heartbeat"));
    assert_eq!(set.code_value("hb"), Some("0"));
    // A code named after its own value takes the real name the venue gave
    // it, which is the enrichment a fold exists for.
    assert_eq!(set.code_name("6"), Some("IOI"));
    // And what only the venue declared arrived whole, its wording with it.
    assert_eq!(set.code_value("VenueOwn"), Some("ZZ"));
    assert_eq!(
        set.code("ZZ").and_then(|code| code.parse_doc().unwrap()),
        Some("A type only this venue sends".to_owned())
    );
}

#[test]
fn a_field_keeps_the_set_it_reads_by_when_another_dictionary_names_another() {
    let (mut held, _) = dictionary("heldcodeset", 54, &[FixCode::new("Buy", "1")]);
    let (venue, _) = dictionary("venuecodeset", 54, &[FixCode::new("Sell", "2")]);

    held.merge_with(&venue).unwrap();
    // The field keeps its own vocabulary's name, and what the other
    // dictionary's set declared is in that vocabulary rather than in one
    // no field reads by: a merge widens a set and never narrows one.
    let field = held.field_by_tag(54).unwrap();
    assert_eq!(field.as_fix().codeset(), Some("heldcodeset"));
    let set = held.codeset_of(field).expect("the set");
    assert_eq!(set.code_value("Buy"), Some("1"));
    assert_eq!(set.code_value("Sell"), Some("2"));
    // The incoming name is still a set of its own: a name is an identity,
    // and folding its members into another does not retire it.
    assert_eq!(held.codesets().len(), 3);
    assert!(held.get_codeset("venuecodeset").is_some());
}

#[test]
fn a_dictionary_holding_only_the_crate_set_reads_back_equal() {
    // The crate's MsgCat vocabulary is registry-owned like every other
    // set, so even a fresh dictionary persists that one intrinsic set.
    let path = Folder::temporary()
        .unwrap()
        .path()
        .unwrap()
        .join(format!("yggdryl-codesets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    let mut root = Holder::local(path.clone()).unwrap();
    let registry = FixRegistry::new();
    assert_eq!(registry.codesets().len(), 1);
    registry.write_into(root.as_io_mut()).unwrap();
    assert_eq!(FixRegistry::from_handle(root.as_io()).unwrap(), registry);

    // And one that gains a set writes the folder, while one that loses it
    // again takes the folder away rather than leaving a stale document.
    let mut held = registry.clone();
    held.set_codeset("sidecodeset", &[FixCode::new("Buy", "1")])
        .unwrap();
    held.write_into(root.as_io_mut()).unwrap();
    let read = FixRegistry::from_handle(root.as_io()).unwrap();
    assert_eq!(read, held);
    assert_eq!(read.codeset("sidecodeset").unwrap().codes().count(), 1);

    registry.write_into(root.as_io_mut()).unwrap();
    assert_eq!(FixRegistry::from_handle(root.as_io()).unwrap(), registry);
    let _ = std::fs::remove_dir_all(&path);

    // The other persistence door states the sets under a key of their
    // own, read before the fields that name them.
    let document = held.into_json().unwrap();
    assert!(document.contains("\"codesets\""), "{document:.120}");
    assert_eq!(FixRegistry::from_json(&document).unwrap(), held);
    let refused = FixRegistry::from_json(
        r#"{"codesets":[],"fields":[],"components":[],"groups":[],"messages":[]}"#,
    )
    .unwrap_err();
    assert!(
        matches!(&refused, Error::InvalidRecord { path, reason }
            if path == "messages" && reason.contains("codesets")),
        "{refused}"
    );
}

#[test]
fn a_name_no_store_can_file_is_refused_before_anything_is_written() {
    let mut registry = FixRegistry::new();
    for name in ["", "..", "one/two", "a b"] {
        let refused = registry.set_codeset(name, &[FixCode::new("Buy", "1")]);
        assert!(refused.is_err() || name.is_empty(), "{name:?}");
    }
    assert_eq!(registry.codesets().len(), 1);
}

mod party_source {
    use super::{SoleMessage, committed_registry, fixed_codec, path};
    use yggdryl::{FixCodec, Scalar};

    fn reader() -> FixCodec {
        fixed_codec(committed_registry()).with_exclude_msgtypes::<[&str; 0], &str>([])
    }

    #[test]
    fn party_source_family_resolves_the_bridge_proprietary_alias_to_d() {
        let registry = committed_registry();
        for tag in [447, 525] {
            let field = registry.field_by_tag(tag).expect("a PartyIDSource field");
            let codes = registry
                .codeset_of(field)
                .expect("the PartyIDSource code set");
            assert_eq!(codes.code_value("proprietary/customcode"), Some("D"));
            assert_eq!(
                codes.code("D").expect("the canonical code").name(),
                "Proprietary"
            );
        }
    }

    #[test]
    fn a_nested_bridge_party_source_is_typed_to_d_and_writes_the_wire_value() {
        let reader = reader();
        let parsed = reader
            .sole_line(
                b"MSGTYPE=D|#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=SYNTH-01\x04\x03PARTYIDSOURCE=proprietary/customcode\x04\x03PARTYROLE=1",
            )
            .expect("a bridge party");
        assert_eq!(
            parsed
                .get_by_path(&path("Parties[0].PartyIDSource"))
                .as_ref()
                .and_then(Scalar::as_str),
            Some("D")
        );

        let emitted = parsed.into_bytes(b'|');
        assert!(
            String::from_utf8_lossy(&emitted).contains("|447=D|"),
            "{}",
            String::from_utf8_lossy(&emitted)
        );
        let reread = reader.sole_line(&emitted).expect("the emitted FIX row");
        assert_eq!(
            reread
                .get_by_path(&path("Parties[0].PartyIDSource"))
                .as_ref()
                .and_then(Scalar::as_str),
            Some("D")
        );
    }
}

/// A code whose name is its own wire value is matched exactly, so two codes
/// differing only in case are two codes.
///
/// The crate's one fold serves names - `NewOrderSingle`, `new_order_single`
/// and `NEW ORDER SINGLE` are one spelling - and a code carrying no name of
/// its own is named after its value. Folding there would make the fold decide
/// what a counterparty meant by a byte it was explicit about: tag 35 `b` is
/// MassQuoteAcknowledgement and `B` is News.
#[test]
fn a_codes_own_wire_value_is_the_one_spelling_the_fold_does_not_reach() {
    let pairs = [
        FixCode::new("b", "b"),
        FixCode::new("B", "B"),
        FixCode::new("c", "c"),
        FixCode::new("C", "C"),
    ];
    let (registry, field) = dictionary("msgtypecodeset", 35, &pairs);
    let set = registry.codeset_of(&field).expect("the set");
    assert_eq!(set.codes().count(), 4);
    for value in ["b", "B", "c", "C"] {
        assert_eq!(set.code_value(value), Some(value), "{value:?}");
        assert_eq!(set.code_name(value), Some(value), "{value:?}");
        assert_eq!(
            set.code_by_name(value).map(|code| code.value()),
            Some(value),
            "{value:?}"
        );
    }
    // A real name still folds, which is what a name is for.
    let named = [
        FixCode::new("MassQuoteAcknowledgement", "b"),
        FixCode::new("News", "B"),
    ];
    let (registry, field) = dictionary("namedcodeset", 35, &named);
    let set = registry.codeset_of(&field).expect("the set");
    for spelling in [
        "MassQuoteAcknowledgement",
        "massquoteacknowledgement",
        "MASS_QUOTE-ACKNOWLEDGEMENT",
    ] {
        assert_eq!(set.code_value(spelling), Some("b"), "{spelling:?}");
    }
    assert_eq!(set.code_value("news"), Some("B"));
    // And two real names one fold reaches are still refused, because two
    // codes one spelling reaches resolve to neither.
    let mut contended = FixRegistry::new();
    let error = contended
        .set_codeset(
            "contendedcodeset",
            &[FixCode::new("News", "B"), FixCode::new("n e w s", "b")],
        )
        .expect_err("two names one fold reaches");
    assert!(error.to_string().contains("twice"), "{error}");
}

/// Two dictionaries each holding one case of a letter fold to a set holding
/// both, with the members only the second states appended.
#[test]
fn folding_two_dictionaries_keeps_both_cases_of_one_message_code() {
    let (mut held, _) = dictionary("msgtypecodeset", 35, &[FixCode::new("News", "B")]);
    let (incoming, _) = dictionary(
        "msgtypecodeset",
        35,
        &[
            FixCode::new("News", "B"),
            FixCode::new("MassQuoteAcknowledgement", "b"),
        ],
    );
    held.merge_with(&incoming).expect("the two sets fold");
    let set = held.codeset("msgtypecodeset").expect("the folded set");
    assert_eq!(
        set.codes()
            .map(|code| code.unwrap().value())
            .collect::<Vec<_>>(),
        ["B", "b"]
    );
    assert_eq!(set.code_value("b"), Some("b"));
    assert_eq!(set.code_value("B"), Some("B"));
    assert_eq!(set.code_value("massquoteacknowledgement"), Some("b"));
    assert_eq!(set.code_value("news"), Some("B"));
}
