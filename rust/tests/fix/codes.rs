//! `rust/src/fix/codes.rs`: one code set, named once, however many fields read
//! by it.
//!
//! A vocabulary is registry-owned: a field names the set it reads by and the
//! document lives once beside the fields. Every door here is one a caller has,
//! so this reaches the crate through `yggdryl::` - but for a stored document
//! no caller can render, which `mod internal` files through
//! `yggdryl::internals` the way a store's load does.

use super::SoleMessage;
use super::committed_registry;
use super::fixed_codec;
use super::path;

use yggdryl::holder::Holder;
use yggdryl::local::LocalFolder;
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

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::fix_codes::create_codeset;
    use yggdryl::{DataType, FixCode, FixRegistry};

    /// A store files the document it reads without re-rendering it, so a set
    /// stating one spelling on two codes - which no caller's `set_codeset`
    /// renders - can be held, and every fold that meets it heals it rather
    /// than refusing the whole source with "expected each code name once".
    #[test]
    fn a_stored_set_stating_one_name_twice_merges_with_any_set() {
        let document = r#"[{"value":"8","name":"pending"},{"value":"Z","name":"pending"}]"#;
        let held = || {
            let mut registry = FixRegistry::new();
            create_codeset(&mut registry, "ordstatuscodeset", document.to_owned()).unwrap();
            let mut field = DataType::utf8().nullable_field("ordstatus");
            field.as_fix_mut().set_tag(39).unwrap();
            field.as_fix_mut().set_codeset("ordstatuscodeset").unwrap();
            registry.insert(field).unwrap();
            registry
        };
        // The first code keeps the name, the second keeps its value.
        let assert_healed = |registry: &FixRegistry, values: &[&str]| {
            let set = registry.codeset("ordstatuscodeset").unwrap();
            assert_eq!(
                set.codes()
                    .map(|code| code.unwrap().value().to_owned())
                    .collect::<Vec<_>>(),
                values
            );
            assert_eq!(set.code_name("8"), Some("pending"));
            assert_eq!(set.code_name("Z"), Some("Z"));
            assert_eq!(set.code_value("pending"), Some("8"));
            assert_eq!(set.code_value("Z"), Some("Z"));
            assert_eq!(
                &FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
                registry
            );
        };

        let mut registry = held();
        registry
            .merge_codeset("ordstatuscodeset", &[FixCode::new("Filled", "2")])
            .unwrap();
        assert_healed(&registry, &["8", "Z", "2"]);
        let set = registry.codeset("ordstatuscodeset").unwrap();
        assert_eq!(set.code_name("2"), Some("Filled"));

        // A set that states nothing new heals it all the same, and so does a
        // whole dictionary folded in.
        let mut registry = held();
        registry
            .merge_codeset("ordstatuscodeset", &[FixCode::new("Z", "Z")])
            .unwrap();
        assert_healed(&registry, &["8", "Z"]);
        let mut registry = held();
        let mut other = FixRegistry::new();
        other
            .set_codeset("ordstatuscodeset", &[FixCode::new("New", "0")])
            .unwrap();
        registry.merge_with(&other).unwrap();
        assert_healed(&registry, &["8", "Z", "0"]);
    }
}

#[test]
fn a_name_spelling_nothing_claims_nothing() {
    // `None`, `NULL` and a blank name nothing, so they claim nothing: two
    // codes may both be called by one, beside a code named after its own
    // value `NONE`, and the writer refuses none of it. A sentinel still
    // reaches the one code it is the only spelling of.
    let (registry, field) = dictionary(
        "daterollconventioncodeset",
        40922,
        &[
            FixCode::new("NONE", "NONE"),
            FixCode::new("None", "0"),
            FixCode::new("NULL", "1"),
        ],
    );
    let set = registry.codeset_of(&field).expect("the set");
    assert_eq!(
        set.code_value("NONE"),
        Some("NONE"),
        "a wire value, exactly"
    );
    assert_eq!(set.code_value("none"), Some("0"));
    assert_eq!(set.code_value("null"), Some("1"));
    let mut twice = FixRegistry::new();
    twice
        .set_codeset(
            "twicecodeset",
            &[FixCode::new("none", "0"), FixCode::new("None", "1")],
        )
        .expect("two codes may both name nothing");
    let set = twice.codeset("twicecodeset").unwrap();
    assert_eq!(
        set.code_value("none"),
        None,
        "one spelling, two codes, neither"
    );
}

#[test]
fn a_merge_never_answers_a_set_the_writer_refuses() {
    // `EOM` is named after its own value, so a lookup reaches it by that
    // value exactly - and a new code named `eom` would be the same name to
    // the writer. The merge keeps the new value under no name and refuses
    // nothing, where it used to answer a set the writer then refused.
    let (mut registry, _) = dictionary("rollcodeset", 40922, &[FixCode::new("EOM", "EOM")]);
    let (merged, warnings) = super::warned::during(|| {
        registry.merge_codeset("rollcodeset", &[FixCode::new("eom", "99")])
    });
    merged.expect("the merge keeps the value");
    let set = registry.codeset("rollcodeset").unwrap();
    assert_eq!(set.code_name("99"), Some("99"));
    assert_eq!(set.code_value("EOM"), Some("EOM"));
    assert!(
        warnings.iter().any(|warning| warning
            .contains(r#"value "99" keeps no name, "eom" already names value "EOM""#)),
        "{warnings:?}"
    );
}

#[test]
fn a_sentinel_is_claimed_by_nothing_whichever_side_holds_it() {
    // A no-break space trims away, so `\u{a0}none` spells nothing, while
    // `_\u{a0}none` keeps a separator the trim does not reach and is a name
    // - one the crate's fold joins to the first. Neither side's sentinel
    // takes part in the one-name rule, so the writer renders the two in
    // either order rather than refusing the one that happens to come second.
    for codes in [
        [
            FixCode::new("\u{a0}none", "1"),
            FixCode::new("_\u{a0}none", "2"),
        ],
        [
            FixCode::new("_\u{a0}none", "2"),
            FixCode::new("\u{a0}none", "1"),
        ],
    ] {
        let mut registry = FixRegistry::new();
        registry
            .set_codeset("spacecodeset", &codes)
            .expect("a sentinel collides with nothing");
        assert_eq!(registry.codeset("spacecodeset").unwrap().codes().count(), 2);
    }
}

#[test]
fn an_alias_two_held_codes_share_names_neither_after_a_merge() {
    // Rendering holds names to one code each and aliases to nothing, so a set
    // may state one alias on two codes - and then it names neither. A merge
    // widening that set keeps both codes' aliases as they are: folding the
    // alias onto the first would make it name one code where it named none.
    let (mut registry, _) = dictionary(
        "venuecodeset",
        9001,
        &[
            FixCode::new("Primary", "1").with_aliases(["shared"]),
            FixCode::new("Secondary", "2").with_aliases(["shared"]),
        ],
    );
    assert_eq!(
        registry
            .codeset("venuecodeset")
            .unwrap()
            .code_value("shared"),
        None
    );
    registry
        .merge_codeset("venuecodeset", &[FixCode::new("Tertiary", "3")])
        .unwrap();
    let set = registry.codeset("venuecodeset").unwrap();
    assert_eq!(set.codes().count(), 3);
    assert_eq!(
        set.code_value("shared"),
        None,
        "the shared alias names neither"
    );
    let aliases: Vec<Vec<String>> = set
        .codes()
        .map(|code| code.unwrap().aliases().map(str::to_owned).collect())
        .collect();
    assert_eq!(aliases, [vec!["shared"], vec!["shared"], vec![]]);
    assert_eq!(set.code_value("Tertiary"), Some("3"));
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

    // The set named here, beside the crate's MsgCat, state and market data
    // type sets.
    assert_eq!(registry.codesets().len(), 4);
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
    assert_eq!(held.codesets().len(), 5);
    assert!(held.get_codeset("venuecodeset").is_some());
}

#[test]
fn a_dictionary_holding_only_the_crate_set_reads_back_equal() {
    // The crate's MsgCat and state vocabularies are registry-owned like
    // every other set, so even a fresh dictionary persists those two
    // intrinsic sets.
    let path = LocalFolder::temporary()
        .unwrap()
        .path()
        .unwrap()
        .join(format!("yggdryl-codesets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    let mut root = Holder::local(path.clone()).unwrap();
    let registry = FixRegistry::new();
    assert_eq!(registry.codesets().len(), 3);
    registry.commit(root.as_io_mut()).unwrap();
    assert_eq!(FixRegistry::from_handle(root.as_io()).unwrap(), registry);

    // And one that gains a set writes the folder, while one that loses it
    // again takes the folder away rather than leaving a stale document.
    let mut held = registry.clone();
    held.set_codeset("sidecodeset", &[FixCode::new("Buy", "1")])
        .unwrap();
    held.commit(root.as_io_mut()).unwrap();
    let read = FixRegistry::from_handle(root.as_io()).unwrap();
    assert_eq!(read, held);
    assert_eq!(read.codeset("sidecodeset").unwrap().codes().count(), 1);

    registry.commit(root.as_io_mut()).unwrap();
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
fn the_state_set_is_the_crates_own_and_refuses_every_change() {
    // Every member of `State`, its code the value a `state` column stores.
    let mut registry = FixRegistry::new();
    let held = registry.codeset("statecodeset").expect("the intrinsic set");
    assert_eq!(held.codes().count(), yggdryl::State::ALL.len());
    assert_eq!(held.code_name("2001"), Some("NEW"));
    assert_eq!(held.code_value("PARTIALLY_FILLED"), Some("4001"));

    let refusals = [
        registry
            .set_codeset("statecodeset", &[FixCode::new("NEW", "2001")])
            .unwrap_err(),
        registry
            .merge_codeset("statecodeset", &[FixCode::new("NEW", "7")])
            .unwrap_err(),
        registry.remove_codeset("statecodeset").unwrap_err(),
    ];
    for refused in refusals {
        assert!(
            refused.to_string().contains("the fixed state codes"),
            "{refused}"
        );
    }
    assert_eq!(
        registry.codeset("statecodeset").unwrap().codes().count(),
        yggdryl::State::ALL.len()
    );
}

#[test]
fn a_name_no_store_can_file_is_refused_before_anything_is_written() {
    let mut registry = FixRegistry::new();
    for name in ["", "..", "one/two", "a b"] {
        let refused = registry.set_codeset(name, &[FixCode::new("Buy", "1")]);
        assert!(refused.is_err() || name.is_empty(), "{name:?}");
    }
    assert_eq!(registry.codesets().len(), 3);
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
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    assert!(
        error.to_string().contains("expected each code name once")
            && error
                .to_string()
                .contains(r#""n e w s" for value "b" beside "News" for value "B""#),
        "{error}"
    );
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

/// A new value arriving under a name another value already holds keeps its
/// value under no name, and the fold says so.
///
/// Two maps of one field disagree about what a name means - a venue's
/// `ORDSTATUS` calls `Z` what FIX's `OrdStatus` calls `8` - and a spelling
/// reaching two codes resolves to neither. The value is a fact about the wire
/// the set keeps whatever it is called; the name stays with the code that
/// held it.
#[test]
fn a_code_whose_name_another_value_holds_keeps_its_value_unnamed() {
    let (mut registry, field) =
        dictionary("ordstatuscodeset", 39, &[FixCode::new("Rejected", "8")]);
    let (merged, warnings) = super::warned::during(|| {
        registry.merge_codeset("ordstatuscodeset", &[FixCode::new("rejected", "Z")])
    });
    merged.unwrap();
    let set = registry.codeset_of(&field).expect("the set");
    assert_eq!(
        set.codes()
            .map(|code| code.unwrap().value().to_owned())
            .collect::<Vec<_>>(),
        ["8", "Z"]
    );
    assert_eq!(set.code_name("8"), Some("Rejected"));
    assert_eq!(
        set.code_name("Z"),
        Some("Z"),
        "unnamed: its name is its value"
    );
    assert_eq!(set.code_value("rejected"), Some("8"));
    assert_eq!(set.code_value("Z"), Some("Z"));
    assert!(
        warnings.iter().any(|warning| warning.contains("\"Z\"")
            && warning.contains("\"rejected\"")
            && warning.contains("\"8\"")),
        "{warnings:?}"
    );

    // Folding the same codes again finds the value held and changes nothing.
    let before = registry.clone();
    registry
        .merge_codeset("ordstatuscodeset", &[FixCode::new("rejected", "Z")])
        .unwrap();
    assert_eq!(registry, before);
}
