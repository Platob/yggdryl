//! The committed dictionary the generator writes, read back.
//!
//! `config/fix` is a contract rather than a code path: it is the seed every
//! test in these phases loads, and the one path this suite names.

use std::path::PathBuf;

use yggdryl::holder::local::Folder;
use yggdryl::{DataType, FixRegistry, STANDARD_HEADER_TAGS, STANDARD_TRAILER_TAGS, Version};

fn seed() -> FixRegistry {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let folder = Folder::new(root).expect("the seed folder is a local path");
    FixRegistry::from_handle(&folder).expect("the committed dictionary loads")
}

fn version(text: &str) -> Version {
    text.parse().expect("a valid version")
}

#[test]
fn the_committed_dictionary_answers_the_worked_case_end_to_end() {
    let registry = seed();
    assert!(
        registry.len() > 5_000,
        "a whole registry, got {}",
        registry.len()
    );

    // Tag 32: `LastShares` typed `int` in 4.0, `LastShares` typed `Qty` from
    // 4.2, and `LastQty` from 4.3 on.
    let last_qty = registry.field_by_tag(32).expect("tag 32");
    assert_eq!(last_qty.name(), "lastqty");
    assert_eq!(last_qty.dtype(), &DataType::Float64);
    assert_eq!(last_qty.as_metadata().get("display"), Some("LastQty"));

    let view = last_qty.as_fix();
    assert_eq!(view.since(), Some(version("2.7")));
    assert_eq!(view.name_at(version("4.2")), Some("lastshares"));
    assert_eq!(view.name_at(version("4.4")), Some("lastqty"));
    assert_eq!(
        view.dtype_at(version("4.0")).unwrap(),
        Some(DataType::Int32)
    );
    assert_eq!(view.aliases().collect::<Vec<_>>(), ["lastshares"]);

    // A query by either spelling answers the same field, and the
    // specification's own casing still resolves.
    for spelling in ["lastqty", "LastQty", "LastShares", "lastshares"] {
        assert_eq!(
            registry.field(spelling).expect(spelling).name(),
            "lastqty",
            "{spelling}"
        );
    }

    // "FIX Latest" is a real pedigree the dictionary carries, never a
    // sentinel at the top of the value space.
    let newest = registry.newest().expect("a dated dictionary");
    assert_eq!(newest.version(), version("5.0SP2"));
    assert!(newest.ep().is_some());
    assert_ne!(newest.version(), Version::MAX);
    let versions = registry.versions();
    assert!(versions.contains(&version("2.7")), "{versions:?}");
    assert!(versions.contains(&version("4.4")), "{versions:?}");
}

#[test]
fn every_generated_name_is_folded_and_no_two_collide() {
    let registry = seed();
    let mut seen: Vec<String> = Vec::with_capacity(registry.len());
    for field in registry.iter() {
        let name = field.name();
        assert!(
            !name.bytes().any(|byte| byte.is_ascii_uppercase()),
            "{name} holds an uppercase byte"
        );
        assert!(!name.contains('_'), "{name} holds an underscore");
        seen.push(name.to_owned());
    }
    // No two FIX fields differ only by case, which is what makes folding the
    // stored name lossless - and it is what the separator fold leans on too.
    seen.sort_unstable();
    let before = seen.len();
    seen.dedup();
    assert_eq!(seen.len(), before, "two fields fold to one name");
}

#[test]
fn the_standard_declares_its_code_sets_and_the_generator_honours_them() {
    let registry = seed();

    // Two tags the standard itself declares as code sets take the datatype
    // the crate gives that code set. That is honouring a declaration, not the
    // narrowing a generator must not do.
    let msgtype = registry.field_by_tag(35).expect("tag 35");
    assert_eq!(msgtype.dtype(), &DataType::MsgType);
    assert_eq!(msgtype.as_fix().code_name("D"), Some("NewOrderSingle"));
    assert_eq!(msgtype.as_fix().code_value("NewOrderSingle"), Some("D"));

    let side = registry.field_by_tag(54).expect("tag 54");
    assert_eq!(side.dtype(), &DataType::Side);
    assert_eq!(side.as_fix().code_name("1"), Some("Buy"));
    assert_eq!(side.as_fix().code_value("buy"), Some("1"));
    // A code set stays on the field that declares it; every other one keeps
    // its base type.
    let ord_status = registry.field_by_tag(39).expect("tag 39");
    assert_eq!(ord_status.dtype(), &DataType::Utf8);
    assert!(ord_status.as_fix().codes().count() > 5);

    // The float family is what the specification says it is.
    for tag in [31, 38, 44, 6] {
        let field = registry.field_by_tag(tag).expect("a float-family tag");
        assert_eq!(field.dtype(), &DataType::Float64, "tag {tag}");
    }
    // And the ones it types otherwise keep those types.
    assert_eq!(registry.field_by_tag(34).unwrap().dtype(), &DataType::Int64);
    assert_eq!(registry.field_by_tag(10).unwrap().dtype(), &DataType::Utf8);
    assert_eq!(registry.field_by_tag(9).unwrap().dtype(), &DataType::Int32);
}

#[test]
fn a_repeating_group_is_a_list_of_one_component_struct_keyed_by_its_counter() {
    let registry = seed();
    let parties = registry.field_by_tag(453).expect("NoPartyIDs");
    assert_eq!(parties.name(), "nopartyids");
    let DataType::List(item) = parties.dtype() else {
        panic!("a list, got {}", parties.dtype());
    };
    // One occurrence of the component the counter heads: `NoPartyIDs` holds
    // `PartyID`s, and the member of that name is still reached through it.
    assert_eq!(item.name(), "partyid");
    assert!(!item.is_nullable());
    let members: Vec<&str> = item
        .dtype()
        .as_fields()
        .expect("a struct item")
        .iter()
        .map(yggdryl::Field::name)
        .collect();
    assert!(members.contains(&"partyid"), "{members:?}");
    assert!(members.contains(&"partyrole"), "{members:?}");
    // The counter exists only as the group's own tag, never as a leaf beside
    // it: one tag, one definition.
    assert_eq!(parties.as_fix().tag().unwrap(), Some(453));
    assert!(!members.contains(&"nopartyids"), "{members:?}");
}

#[test]
fn every_header_and_trailer_tag_resolves_in_the_generated_dictionary() {
    let registry = seed();
    for tag in STANDARD_HEADER_TAGS {
        assert!(registry.contains(tag), "header tag {tag} does not resolve");
    }
    for tag in STANDARD_TRAILER_TAGS {
        assert!(registry.contains(tag), "trailer tag {tag} does not resolve");
    }
    // The union across versions, not FIX Latest alone: Latest no longer lists
    // `SecureDataLen(90)` in the header nor `SignatureLength(93)` in the
    // trailer, and a 4.2 message carries both.
    assert!(
        STANDARD_HEADER_TAGS.contains(&90),
        "{STANDARD_HEADER_TAGS:?}"
    );
    assert!(
        STANDARD_TRAILER_TAGS.contains(&93),
        "{STANDARD_TRAILER_TAGS:?}"
    );
    assert_eq!(STANDARD_HEADER_TAGS[..3], [8, 9, 35]);
    assert_eq!(STANDARD_TRAILER_TAGS.last(), Some(&10));
}

#[test]
fn the_dictionary_writes_back_byte_identically() {
    let registry = seed();
    let scratch = std::env::temp_dir().join("yggdryl-fix-roundtrip");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("a scratch folder");
    let mut folder = Folder::new(scratch.clone()).expect("a local folder");
    registry
        .write_into(&mut folder)
        .expect("the dictionary writes");

    let written = FixRegistry::from_handle(&Folder::new(scratch.clone()).unwrap())
        .expect("what was written loads");
    assert_eq!(written.len(), registry.len());
    assert_eq!(written, registry);
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn every_stored_document_walks_to_its_end() {
    let registry = seed();
    let mut with_codes = 0_usize;
    let mut codes = 0_usize;
    let mut entries = 0_usize;
    for field in registry.iter() {
        let view = field.as_fix();
        // A refusal ends a borrowed walk, and the walk is what resolution
        // reads - so one malformed record does not fail loudly, it silently
        // removes every record after it. Orchestra's `addedEP="-1"` did
        // exactly that to 169 code sets: `ClearingFirm` stopped resolving
        // because a code 60 records earlier would not parse. Nothing here can
        // catch that but walking every document to its end.
        let mut seen = 0_usize;
        for code in view.codes() {
            code.unwrap_or_else(|error| panic!("{}: {error}", field.name()));
            seen += 1;
        }
        if seen > 0 {
            with_codes += 1;
        }
        codes += seen;
        for entry in view.lineage() {
            entry.unwrap_or_else(|error| panic!("{}: {error}", field.name()));
            entries += 1;
        }
    }
    // A dictionary this size is the point: a truncation that hides one code
    // in twenty thousand is exactly what nobody notices by reading.
    assert!(with_codes > 900, "{with_codes} fields carry a code set");
    assert!(codes > 20_000, "{codes} codes in all");
    // Fewer than there are fields, and deliberately: a field whose only
    // history is "as it is now" states none (P3).
    assert!(entries > 1_500, "{entries} lineage entries in all");

    // The spelling the truncation hid, end to end.
    let role = registry.field_by_tag(452).expect("PartyRole");
    assert_eq!(role.as_fix().code_value("ClearingFirm"), Some("4"));
    assert_eq!(role.as_fix().code_name("4"), Some("ClearingFirm"));
}
