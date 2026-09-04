//! Focused edge cases of the FIX module, driven with explicit inputs.

use std::path::PathBuf;
use std::sync::Arc;

use super::global::autoload;
use super::registry::{BranchedName, Folded};
use super::store::shard_of;
use crate::local::Folder;
use crate::{DataType, Error, Field, FixBranch, FixId, FixKey, FixMsg, FixRegistry, Scalar};

/// The venue dictionary every branched case is written against.
fn cme() -> FixBranch {
    FixBranch::from_str("cme").unwrap()
}

/// A nullable text field carrying one canonical tag.
fn tagged(name: &str, tag: i32) -> Field {
    let mut field = DataType::Utf8.nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

/// A nullable text field carrying one canonical identifier.
fn identified(name: &str, id: &FixId) -> Field {
    let mut field = DataType::Utf8.nullable_field(name);
    field.as_fix_mut().set_id(id).unwrap();
    field
}

/// A field carrying every `fix:` property.
fn full(name: &str, tag: i32, tags: &[i32], aliases: &[&str]) -> Field {
    let mut field = tagged(name, tag);
    field.as_fix_mut().set_tags(tags).unwrap();
    field.as_fix_mut().set_aliases(aliases).unwrap();
    field
        .as_fix_mut()
        .set_description(format!("{name} described"))
        .unwrap();
    field
}

/// A nullable Struct component - a FIX component - carrying one canonical tag.
fn component(name: &str, tag: i32) -> Field {
    let mut field = DataType::from_fields([tagged("Member", 9_001)])
        .unwrap()
        .nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

/// A nullable List of Struct - a FIX repeating group - carrying one canonical
/// tag.
fn group(name: &str, tag: i32) -> Field {
    let item = DataType::from_fields([tagged("Member", 9_002)])
        .unwrap()
        .required_field("item");
    let mut field = DataType::list(item).nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

/// A fresh directory of this test's own under the platform temporary root.
fn scratch(label: &str) -> PathBuf {
    let path = Folder::temporary()
        .unwrap()
        .path()
        .unwrap()
        .join(format!("yggdryl-fix-unit-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    path
}

/// The stored keys of one registry, probed through every index.
fn probe<'registry>(
    registry: &'registry FixRegistry,
    tag: i32,
    alternate: i32,
    name: &str,
    alias: &str,
) -> [Option<&'registry str>; 4] {
    let standard = FixBranch::STANDARD;
    [
        registry.get_field_by_tag(tag).map(Field::name),
        registry.get_field_by_tag(alternate).map(Field::name),
        registry.get_field_by_name(&standard, name).map(Field::name),
        registry
            .get_field_by_name(&standard, alias)
            .map(Field::name),
    ]
}

#[test]
fn folded_text_compares_and_hashes_without_case() {
    use std::hash::{BuildHasher, RandomState};

    let state = RandomState::new();
    assert_eq!(Folded::probe("MsgType"), Folded::probe("MSGTYPE"));
    assert_eq!(
        state.hash_one(Folded::probe("MsgType")),
        state.hash_one(Folded::probe("msgtype"))
    );
    assert_ne!(Folded::probe("MsgType"), Folded::probe("MsgTypes"));
    // Folding is ASCII only: a non-ASCII byte is compared as it is.
    assert_ne!(Folded::probe("Größe"), Folded::probe("GRÖSSE"));

    // A name index key is that folding inside one branch: the name still
    // folds, the branch is compared exactly, and the separator between
    // them keeps the concatenation unambiguous.
    let standard = FixBranch::STANDARD;
    let cme = cme();
    assert_eq!(
        BranchedName::probe(&cme, "MsgType"),
        BranchedName::probe(&cme, "MSGTYPE")
    );
    assert_eq!(
        state.hash_one(BranchedName::probe(&cme, "MsgType")),
        state.hash_one(BranchedName::probe(&cme, "msgtype"))
    );
    assert_ne!(
        BranchedName::probe(&cme, "MsgType"),
        BranchedName::probe(&standard, "MsgType")
    );
    assert_ne!(
        state.hash_one(BranchedName::probe(&cme, "MsgType")),
        state.hash_one(BranchedName::probe(&standard, "MsgType"))
    );
    // `cme` + `x` and `cm` + `ex` are two keys, not one.
    let cm = FixBranch::from_str("cm").unwrap();
    assert_ne!(
        BranchedName::probe(&cme, "x"),
        BranchedName::probe(&cm, "ex")
    );
    assert_ne!(
        state.hash_one(BranchedName::probe(&cme, "x")),
        state.hash_one(BranchedName::probe(&cm, "ex"))
    );
}

#[test]
fn a_branch_folds_once_and_refuses_what_it_cannot_hold() {
    assert_eq!(FixBranch::from_str("CME").unwrap().as_str(), "cme");
    assert_eq!(FixBranch::from_str("cme").unwrap(), cme());
    assert_eq!(
        FixBranch::from_str("STANDARD").unwrap(),
        FixBranch::STANDARD
    );
    assert!(FixBranch::STANDARD.is_standard());
    assert!(!cme().is_standard());
    assert_eq!(FixBranch::default(), FixBranch::STANDARD);
    assert_eq!(FixBranch::STANDARD.to_string(), "standard");
    assert_eq!(FixBranch::from_str("a-b.c_9").unwrap().as_str(), "a-b.c_9");

    let cases = [
        ("", 0_usize),
        (" cme", 0),
        ("2cme", 0),
        (".cme", 0),
        ("cme:1", 3),
        ("cm e", 2),
        ("cm,e", 2),
        ("cme/x", 3),
        ("aaaaaaaaaaaaaaaaaaaaaaaa", FixBranch::MAX_LENGTH),
    ];
    for (text, position) in cases {
        let error = FixBranch::from_str(text).unwrap_err();
        match &error {
            Error::Parse {
                target: "fix branch",
                position: at,
                ..
            } => assert_eq!(*at, position, "{text:?}"),
            other => panic!("{text:?}: {other}"),
        }
    }
    // The bound is exactly `smol_str`'s inline capacity, which is what the
    // allocation test holds it to.
    assert_eq!(FixBranch::MAX_LENGTH, 23);
    assert!(FixBranch::from_str(&"a".repeat(FixBranch::MAX_LENGTH)).is_ok());
}

#[test]
fn an_identifier_renders_and_parses_branch_colon_tag() {
    assert_eq!(FixId::standard(35).to_string(), "standard:35");
    assert_eq!(
        FixId::from_parts(cme(), 5001).unwrap().to_string(),
        "cme:5001"
    );
    for text in ["standard:35", "cme:5001", "standard:0", "cme:2147483647"] {
        let id = FixId::from_str(text).unwrap();
        assert_eq!(id.to_string(), text, "{text}");
        assert_eq!(text.parse::<FixId>().unwrap(), id, "{text}");
    }
    // Case folds on the way in, so one dictionary has one spelling.
    assert_eq!(
        FixId::from_str("CME:5001").unwrap(),
        FixId::from_parts(cme(), 5001).unwrap()
    );
    let id = FixId::from_str("cme:5001").unwrap();
    assert_eq!(id.branch(), &cme());
    assert_eq!(id.tag(), 5001);
    assert!(!id.is_standard());
    assert!(FixId::standard(35).is_standard());

    // Ordering is branch-major, then by tag.
    let mut ids = [
        FixId::from_str("standard:1").unwrap(),
        FixId::from_str("cme:9000").unwrap(),
        FixId::from_str("standard:0").unwrap(),
        FixId::from_str("cme:5000").unwrap(),
    ];
    ids.sort();
    assert_eq!(
        ids.map(|id| id.to_string()),
        ["cme:5000", "cme:9000", "standard:0", "standard:1"]
    );

    // A bare tag is not an identifier, and neither half may be empty or
    // signed.
    for text in ["35", "", "cme", "cme:", ":5001", "cme:+5", "cme:-5"] {
        let error = FixId::from_str(text).unwrap_err();
        assert!(matches!(&error, Error::Parse { .. }), "{text:?}: {error}");
    }
    for (text, position) in [("35", 2_usize), ("cme:abc", 4), ("cme:5 0", 4)] {
        match FixId::from_str(text).unwrap_err() {
            Error::Parse {
                target: "fix identifier",
                position: at,
                ..
            } => assert_eq!(at, position, "{text:?}"),
            other => panic!("{text:?}: {other}"),
        }
    }
    // An over-long branch is refused as a branch, in the identifier's
    // own coordinates because the branch is its prefix.
    let long = format!("{}:5001", "a".repeat(24));
    match FixId::from_str(&long).unwrap_err() {
        Error::Parse {
            target: "fix branch",
            position,
            ..
        } => assert_eq!(position, FixBranch::MAX_LENGTH),
        other => panic!("{other}"),
    }
    // Parsed text is held to the same rule as constructed parts.
    assert!(
        FixId::from_str("cme:35")
            .unwrap_err()
            .to_string()
            .contains("fix:branch")
    );
}

#[test]
fn properties_round_trip_including_empty_and_single_element_lists() {
    let mut field = DataType::Int64.required_field("OrderQty");
    let view = field.as_fix();
    assert_eq!(view.tag().unwrap(), None);
    assert_eq!(view.tags().unwrap(), Vec::<i32>::new());
    assert_eq!(view.aliases().count(), 0);
    assert_eq!(view.description(), None);

    field.as_fix_mut().set_tag(38).unwrap();
    field.as_fix_mut().set_tags(&[152]).unwrap();
    field.as_fix_mut().set_aliases(["Qty"]).unwrap();
    field
        .as_fix_mut()
        .set_description("Quantity ordered.")
        .unwrap();
    assert_eq!(field.as_fix().tag().unwrap(), Some(38));
    assert_eq!(field.as_fix().tags().unwrap(), [152]);
    assert_eq!(field.as_fix().aliases().collect::<Vec<_>>(), ["Qty"]);
    assert_eq!(field.as_fix().description(), Some("Quantity ordered."));
    assert_eq!(field.get_metadata("fix:tag"), Some("38"));
    assert_eq!(field.get_metadata("fix:tags"), Some("152"));
    assert_eq!(field.get_metadata("fix:aliases"), Some("Qty"));

    // Order is priority and is kept; the aliases walk both ways.
    field.as_fix_mut().set_tags(&[3, 1, 2]).unwrap();
    field
        .as_fix_mut()
        .set_aliases(["Quantity", "Qty", "OrderQuantity"])
        .unwrap();
    assert_eq!(field.as_fix().tags().unwrap(), [3, 1, 2]);
    assert_eq!(field.get_metadata("fix:tags"), Some("3,1,2"));
    assert_eq!(
        field.as_fix().aliases().collect::<Vec<_>>(),
        ["Quantity", "Qty", "OrderQuantity"]
    );
    assert_eq!(
        field.as_fix().aliases().rev().collect::<Vec<_>>(),
        ["OrderQuantity", "Qty", "Quantity"]
    );

    // An empty list removes the property rather than storing "".
    field.as_fix_mut().set_tags(&[]).unwrap();
    field.as_fix_mut().set_aliases(Vec::<&str>::new()).unwrap();
    assert!(!field.has_metadata("fix:tags"));
    assert!(!field.has_metadata("fix:aliases"));
    assert_eq!(field.as_fix().tags().unwrap(), Vec::<i32>::new());
    assert_eq!(field.as_fix().aliases().count(), 0);

    // The value outlives the view it was read through.
    let description = field.as_fix().description();
    assert_eq!(description, Some("Quantity ordered."));
}

#[test]
fn a_property_write_rejects_bad_elements_and_leaves_the_field_unchanged() {
    let mut field = tagged("Symbol", 55);
    field.as_fix_mut().set_aliases(["Ticker"]).unwrap();
    let before = field.clone();

    let refusals = [
        field.as_fix_mut().set_tag(-1).unwrap_err(),
        field.as_fix_mut().set_tags(&[1, -2]).unwrap_err(),
        field.as_fix_mut().set_tags(&[1, 2, 1]).unwrap_err(),
        field.as_fix_mut().set_aliases(["Sym", ""]).unwrap_err(),
        field.as_fix_mut().set_aliases(["Sym,bol"]).unwrap_err(),
        field.as_fix_mut().set_aliases(["Sym", "SYM"]).unwrap_err(),
    ];
    for (index, error) in refusals.iter().enumerate() {
        assert!(
            matches!(error, Error::InvalidMetadataValue { key, .. } if key.starts_with("fix:")),
            "refusal {index}: {error}"
        );
        assert!(error.to_string().contains("expected"), "{error}");
    }
    assert_eq!(field, before, "a refusal changes nothing");
}

#[test]
fn the_branch_round_trips_and_the_standard_one_is_never_stored() {
    let cme = cme();
    let mut field = DataType::Utf8.nullable_field("TradeID");

    // Absent means standard, and no identity without a tag.
    assert_eq!(field.as_fix().branch().unwrap(), FixBranch::STANDARD);
    assert_eq!(field.as_fix().id().unwrap(), None);
    assert!(!field.has_metadata("fix:branch"));

    field.as_fix_mut().set_branch(&cme).unwrap();
    assert_eq!(field.as_fix().branch().unwrap(), cme);
    assert_eq!(field.get_metadata("fix:branch"), Some("cme"));
    assert_eq!(field.as_fix().id().unwrap(), None, "still no tag");

    field.as_fix_mut().set_tag(5001).unwrap();
    let id = field.as_fix().id().unwrap().unwrap();
    assert_eq!(id.to_string(), "cme:5001");
    assert_eq!(id, FixId::from_str("cme:5001").unwrap());

    // The canonical answer is the folded spelling, whatever was written.
    let mut shouted = DataType::Utf8.nullable_field("TradeID");
    shouted
        .as_fix_mut()
        .set_branch(&FixBranch::from_str("CME").unwrap())
        .unwrap();
    assert_eq!(shouted.get_metadata("fix:branch"), Some("cme"));

    // Setting the standard branch removes the property rather than
    // storing "standard", so one declaration has one stored form.
    field.as_fix_mut().set_branch(&FixBranch::STANDARD).unwrap();
    assert!(!field.has_metadata("fix:branch"));
    assert_eq!(field.as_fix().branch().unwrap(), FixBranch::STANDARD);
    assert_eq!(
        field.as_fix().id().unwrap(),
        Some(FixId::standard(5001)),
        "the identity follows both halves"
    );
}

#[test]
fn a_specification_tag_belongs_to_the_standard_branch_at_every_door() {
    let cme = cme();
    let refusal = |error: &Error| {
        let Error::InvalidMetadataValue { key, reason } = error else {
            panic!("{error}");
        };
        assert_eq!(key, "fix:branch");
        assert!(reason.contains("5000"), "{reason}");
        assert!(reason.contains("\"cme\""), "{reason}");
    };

    // The constructor is the one implementation, and 5000 is the boundary.
    refusal(&FixId::from_parts(cme.clone(), 35).unwrap_err());
    refusal(&FixId::from_parts(cme.clone(), 4_999).unwrap_err());
    assert!(FixId::from_parts(cme.clone(), 5_000).is_ok());
    assert_eq!(FixId::STANDARD_TAG_LIMIT, 5_000);
    // The rule is one-way: the standard branch holds any tag.
    assert!(FixId::from_parts(FixBranch::STANDARD, 10_000).is_ok());
    assert!(FixId::from_parts(FixBranch::STANDARD, 0).is_ok());

    // `set_branch` on a field whose canonical tag is a specification one.
    let mut field = tagged("Symbol", 55);
    let before = field.clone();
    refusal(&field.as_fix_mut().set_branch(&cme).unwrap_err());
    assert_eq!(field, before, "a refusal changes nothing");

    // The same for a field whose *alternate* tag is one: an alternate
    // resolves as strongly as a canonical tag.
    let mut alternate = identified("TradeID", &FixId::from_parts(cme.clone(), 5001).unwrap());
    alternate.as_fix_mut().set_tags(&[5002]).unwrap();
    let mut standard = DataType::Utf8.nullable_field("TradeID");
    standard.as_fix_mut().set_tag(5001).unwrap();
    standard.as_fix_mut().set_tags(&[35, 5002]).unwrap();
    let before = standard.clone();
    refusal(&standard.as_fix_mut().set_branch(&cme).unwrap_err());
    assert_eq!(standard, before);

    // `set_tag` in a vendor branch: refused, never a silent renamespacing.
    let mut vendor = identified("TradeID", &FixId::from_parts(cme.clone(), 5001).unwrap());
    let before = vendor.clone();
    refusal(&vendor.as_fix_mut().set_tag(35).unwrap_err());
    assert_eq!(vendor, before);
    // `set_tags` likewise.
    refusal(&vendor.as_fix_mut().set_tags(&[5002, 35]).unwrap_err());
    assert_eq!(vendor, before);
    assert!(vendor.as_fix_mut().set_tags(&[5002]).is_ok());

    // Read back from raw metadata a hand edit could have written: refused at
    // the door, so nothing corrupt is ever indexed.
    let mut edited = tagged("MsgType", 35);
    edited.insert_metadata("fix:branch", "cme").unwrap();
    refusal(&edited.as_fix().id().unwrap_err());
    let error = FixRegistry::new().insert(edited).unwrap_err();
    refusal(&error);
}

#[test]
fn set_id_moves_both_halves_in_either_direction_and_atomically() {
    let cme = cme();
    let vendor = FixId::from_parts(cme.clone(), 5001).unwrap();

    // Standard to vendor: the order `set_tag` then `set_branch` refuses.
    let mut field = tagged("Symbol", 55);
    assert!(field.as_fix_mut().set_branch(&cme).is_err());
    field.as_fix_mut().set_id(&vendor).unwrap();
    assert_eq!(field.as_fix().id().unwrap(), Some(vendor.clone()));
    assert_eq!(field.get_metadata("fix:branch"), Some("cme"));
    assert_eq!(field.get_metadata("fix:tag"), Some("5001"));

    // Vendor back to standard, which the other single setter refuses too.
    assert!(field.as_fix_mut().set_tag(35).is_err());
    field.as_fix_mut().set_id(&FixId::standard(35)).unwrap();
    assert_eq!(field.as_fix().id().unwrap(), Some(FixId::standard(35)));
    assert!(!field.has_metadata("fix:branch"));

    // A refused tag restores the branch entry it had already written.
    let mut vendored = identified("TradeID", &vendor);
    let before = vendored.clone();
    let error = vendored
        .as_fix_mut()
        .set_id(&FixId::standard(-1))
        .unwrap_err();
    assert!(error.to_string().contains("fix:tag"), "{error}");
    assert_eq!(vendored, before, "the branch came back");
    assert_eq!(vendored.get_metadata("fix:branch"), Some("cme"));

    // The same unwinding from a field that declared no branch: the
    // removal is undone by leaving the property absent. The tag's own shape
    // is the only half of a legal `FixId` that can still be refused, because
    // the identifier carries the branch rule already.
    let mut plain = tagged("Symbol", 55);
    let before = plain.clone();
    assert!(plain.as_fix_mut().set_id(&FixId::standard(-1)).is_err());
    assert_eq!(plain, before);
    assert!(!plain.has_metadata("fix:branch"));
    assert!(
        FixId::from_parts(cme, -1).is_err(),
        "and never in a vendor one"
    );
}

#[test]
fn two_branches_may_hold_the_same_tag_and_the_same_name() {
    let cme = cme();
    let standard = FixBranch::STANDARD;
    let mut venue = identified("Symbol", &FixId::from_parts(cme.clone(), 5055).unwrap());
    venue.as_fix_mut().set_aliases(["Ticker"]).unwrap();
    venue.as_fix_mut().set_tags(&[9055]).unwrap();
    let mut spec = tagged("Symbol", 5055);
    spec.as_fix_mut().set_aliases(["Ticker"]).unwrap();
    spec.as_fix_mut().set_tags(&[9055]).unwrap();

    let registry = FixRegistry::from_fields([venue.clone(), spec.clone()]).unwrap();
    assert_eq!(registry.len(), 2);

    // Each identifier answers its own field, and each name in its own
    // dictionary.
    assert_eq!(registry.field_by_id(&FixId::standard(5055)).unwrap(), &spec);
    assert_eq!(
        registry
            .field_by_id(&FixId::from_parts(cme.clone(), 5055).unwrap())
            .unwrap(),
        &venue
    );
    assert_eq!(
        registry
            .field_by_id(&FixId::from_parts(cme.clone(), 9055).unwrap())
            .unwrap(),
        &venue,
        "an alternate identifier resolves in its branch too"
    );
    assert_eq!(registry.field_by_name(&cme, "SYMBOL").unwrap(), &venue);
    assert_eq!(registry.field_by_name(&standard, "symbol").unwrap(), &spec);
    assert_eq!(registry.field_by_name(&cme, "ticker").unwrap(), &venue);
    assert_eq!(registry.field_by_name(&standard, "ticker").unwrap(), &spec);

    // A bare tag and a bare name are the standard branch and never reach
    // the venue field.
    assert_eq!(registry.get_field_by_tag(5055), Some(&spec));
    assert_eq!(registry.get_field_by_tag(9055), Some(&spec));
    assert_eq!(registry.get_field("Symbol"), Some(&spec));
    assert_eq!(registry.get_field("Ticker"), Some(&spec));
    // A colon-bearing string is a name, never an identifier.
    assert!(registry.get_field("cme:5055").is_none());
    assert!(registry.get_field_by_name(&cme, "absent").is_none());
    assert!(
        registry
            .get_field_by_id(&FixId::from_parts(cme.clone(), 6000).unwrap())
            .is_none()
    );

    // A conflict inside one branch is still a conflict, and it names that
    // branch; the same key in the other branch is not.
    let mut twice = identified("VenueSym", &FixId::from_parts(cme.clone(), 5099).unwrap());
    twice.as_fix_mut().set_aliases(["TICKER"]).unwrap();
    let mut probed = registry.clone();
    let error = probed.insert(twice.clone()).unwrap_err();
    assert!(
        matches!(&error, Error::Conflict { path, .. }
            if path == "alias \"TICKER\" in branch \"cme\" of VenueSym, held by Symbol"),
        "{error}"
    );
    assert_eq!(probed, registry);
    let mut moved = twice;
    moved.as_fix_mut().set_id(&FixId::standard(5099)).unwrap();
    let error = probed.insert(moved).unwrap_err();
    assert!(
        matches!(&error, Error::Conflict { path, .. }
            if path.contains("in branch \"standard\"")),
        "{error}"
    );
    assert_eq!(probed, registry);

    // The failing halves name the key the way it was asked.
    let by_id = registry
        .field_by_id(&FixId::from_parts(cme, 6000).unwrap())
        .unwrap_err();
    assert!(
        matches!(&by_id, Error::Absent { expected: "fix field", path } if path == "identifier cme:6000"),
        "{by_id}"
    );
    // The specialized and generic pairs answer alike for an identifier.
    let id = FixId::standard(5055);
    assert_eq!(registry.get_field(&id), registry.get_field_by_id(&id));
    assert_eq!(registry.get_field(FixKey::Id(&id)), Some(&spec));
    assert_eq!(
        registry.field(&id).map(Field::name).ok(),
        registry.field_by_id(&id).map(Field::name).ok()
    );
    assert!(registry.contains(&id));
}

#[test]
fn a_corrupt_stored_property_is_reported_under_its_full_key() {
    let cases = [
        ("fix:branch", "2cme"),
        ("fix:branch", ""),
        ("fix:branch", "c me"),
        ("fix:branch", "cme:1"),
        ("fix:branch", "aaaaaaaaaaaaaaaaaaaaaaaa"),
        ("fix:tag", "3x"),
        ("fix:tag", "+35"),
        ("fix:tag", "-35"),
        ("fix:tag", ""),
        ("fix:tags", "1,,2"),
        ("fix:tags", "1,1"),
        ("fix:tags", "1, 2"),
        ("fix:tags", "1,-2"),
    ];
    for (key, stored) in cases {
        let mut field = tagged("Symbol", 55);
        field.insert_metadata(key, stored).unwrap();
        let error = match key {
            "fix:branch" => field.as_fix().branch().unwrap_err(),
            "fix:tag" => field.as_fix().tag().unwrap_err(),
            _ => field.as_fix().tags().unwrap_err(),
        };
        match &error {
            Error::InvalidMetadataValue { key: named, reason } => {
                assert_eq!(named, key);
                assert!(reason.contains(&format!("{stored:?}")), "{reason}");
            }
            other => panic!("{key}={stored:?}: {other}"),
        }
        // A field whose tag is corrupt never enters a registry.
        let error = FixRegistry::new().insert(field).unwrap_err();
        assert!(
            matches!(error, Error::InvalidMetadataValue { .. }),
            "{error}"
        );
    }

    // A stored empty alias element is skipped on read, never reported.
    let mut field = tagged("Symbol", 55);
    field
        .insert_metadata("fix:aliases", ",Ticker,,Sym,")
        .unwrap();
    assert_eq!(
        field.as_fix().aliases().collect::<Vec<_>>(),
        ["Ticker", "Sym"]
    );
}

#[test]
fn a_field_without_a_tag_never_enters() {
    let error = FixRegistry::new()
        .insert(DataType::Utf8.nullable_field("Symbol"))
        .unwrap_err();
    assert!(error.is_absent(), "{error}");
    let message = error.to_string();
    assert!(message.contains("fix:tag"), "{message}");
    assert!(message.contains("Symbol"), "{message}");

    let error = FixRegistry::new()
        .update(DataType::Utf8.nullable_field("Symbol"))
        .unwrap_err();
    assert!(error.is_absent(), "{error}");
}

#[test]
fn a_name_or_alias_resolves_in_any_case_to_the_canonical_spelling() {
    let registry = FixRegistry::from_fields([
        full("Symbol", 55, &[], &["Ticker", "SecuritySymbol"]),
        full("ClOrdID", 11, &[], &["ClientOrderID"]),
    ])
    .unwrap();

    for query in [
        "Symbol",
        "SYMBOL",
        "symbol",
        "Ticker",
        "TICKER",
        "securitysymbol",
    ] {
        assert_eq!(
            registry
                .field_by_name(&FixBranch::STANDARD, query)
                .unwrap()
                .name(),
            "Symbol",
            "{query}"
        );
        assert_eq!(registry.field(query).unwrap().name(), "Symbol", "{query}");
        assert!(registry.contains(query), "{query}");
    }
    assert_eq!(
        registry
            .field_by_name(&FixBranch::STANDARD, "clientorderid")
            .unwrap()
            .name(),
        "ClOrdID"
    );
    assert!(
        registry
            .get_field_by_name(&FixBranch::STANDARD, "Symbols")
            .is_none()
    );
    assert!(!registry.contains("Symbols"));
}

#[test]
fn tier_order_never_lets_an_alternate_key_shadow_a_canonical_one() {
    // `Px` is Price's canonical name and also an alias LastPx declares; the
    // canonical claim wins whatever order the fields entered in. The same
    // holds for a tag: 31 is LastPx's own tag and Price lists it as an
    // alternate.
    let price = full("Px", 44, &[31], &["Price"]);
    let last = full("LastPx", 31, &[], &["Px", "LastPrice"]);
    for order in [[price.clone(), last.clone()], [last, price]] {
        let registry = FixRegistry::from_fields(order).unwrap();
        assert_eq!(
            registry
                .field_by_name(&FixBranch::STANDARD, "px")
                .unwrap()
                .name(),
            "Px"
        );
        assert_eq!(
            registry
                .field_by_name(&FixBranch::STANDARD, "Price")
                .unwrap()
                .name(),
            "Px"
        );
        assert_eq!(
            registry
                .field_by_name(&FixBranch::STANDARD, "LastPrice")
                .unwrap()
                .name(),
            "LastPx"
        );
        assert_eq!(registry.field_by_tag(31).unwrap().name(), "LastPx");
        assert_eq!(registry.field_by_tag(44).unwrap().name(), "Px");
    }
}

#[test]
fn a_tag_query_never_consults_names_and_a_name_query_never_consults_tags() {
    let registry = FixRegistry::from_fields([tagged("35", 1), tagged("MsgType", 35)]).unwrap();
    assert_eq!(registry.field_by_tag(35).unwrap().name(), "MsgType");
    assert_eq!(registry.field_by_tag(1).unwrap().name(), "35");
    assert_eq!(
        registry
            .field_by_name(&FixBranch::STANDARD, "35")
            .unwrap()
            .name(),
        "35"
    );
    assert!(
        registry
            .get_field_by_name(&FixBranch::STANDARD, "1")
            .is_none()
    );
    assert!(registry.get_field_by_tag(2).is_none());
}

#[test]
fn an_insert_conflict_names_both_fields_for_each_key_kind() {
    let stored = full("Symbol", 55, &[65], &["Ticker"]);
    let registry = FixRegistry::from_fields([stored]).unwrap();
    let cases = [
        (full("SymbolSfx", 55, &[], &[]), "identifier standard:55"),
        (
            full("symbol", 56, &[], &[]),
            "name \"symbol\" in branch \"standard\"",
        ),
        (
            full("SymbolSfx", 56, &[65], &[]),
            "alternate identifier standard:65",
        ),
        (
            full("SymbolSfx", 56, &[], &["TICKER"]),
            "alias \"TICKER\" in branch \"standard\"",
        ),
    ];
    for (incoming, key) in cases {
        let mut probed = registry.clone();
        let error = probed.insert(incoming).unwrap_err();
        let Error::Conflict { path, .. } = &error else {
            panic!("{key}: {error}");
        };
        assert!(path.contains(key), "{path}");
        assert!(
            path.contains("SymbolSfx") || path.contains("symbol"),
            "{path}"
        );
        assert!(path.ends_with(", held by Symbol"), "{path}");
        assert_eq!(probed, registry, "{key}: a refusal changes nothing");
        assert_eq!(format!("{probed:?}"), format!("{registry:?}"));
        assert_eq!(probed.len(), 1);
    }

    // Overlap across tiers is not a conflict.
    let mut registry = registry;
    assert_eq!(
        registry
            .insert(full("Ticker", 56, &[55], &["Symbol"]))
            .unwrap(),
        None
    );
    assert_eq!(
        registry
            .field_by_name(&FixBranch::STANDARD, "Ticker")
            .unwrap()
            .name(),
        "Ticker"
    );
    assert_eq!(
        registry
            .field_by_name(&FixBranch::STANDARD, "Symbol")
            .unwrap()
            .name(),
        "Symbol"
    );
    assert_eq!(registry.field_by_tag(55).unwrap().name(), "Symbol");
}

#[test]
fn reinserting_the_same_identity_replaces_wholesale() {
    let mut registry = FixRegistry::from_fields([
        full("Symbol", 55, &[65], &["Ticker"]),
        full("Price", 44, &[], &["Px"]),
    ])
    .unwrap();

    // Same tag, same folded name: the prior definition comes back whole and
    // its old keys are gone.
    let replacement = full("SYMBOL", 55, &[66], &["Sym"]);
    let prior = registry.insert(replacement.clone()).unwrap().unwrap();
    assert_eq!(prior.name(), "Symbol");
    assert_eq!(prior.as_fix().tags().unwrap(), [65]);
    assert_eq!(registry.field_by_tag(55).unwrap(), &replacement);
    assert_eq!(
        registry
            .field_by_name(&FixBranch::STANDARD, "symbol")
            .unwrap()
            .name(),
        "SYMBOL"
    );
    assert_eq!(registry.field_by_tag(66).unwrap().name(), "SYMBOL");
    assert_eq!(
        registry
            .field_by_name(&FixBranch::STANDARD, "Sym")
            .unwrap()
            .name(),
        "SYMBOL"
    );
    assert!(registry.get_field_by_tag(65).is_none());
    assert!(
        registry
            .get_field_by_name(&FixBranch::STANDARD, "Ticker")
            .is_none()
    );
    assert_eq!(registry.len(), 2);

    // A tag matching one field and a name matching another is never a
    // replacement, and a new key another field holds refuses the whole thing.
    let before = registry.clone();
    let error = registry.insert(full("Price", 55, &[], &[])).unwrap_err();
    assert!(error.is_conflict(), "{error}");
    let error = registry
        .insert(full("Symbol", 55, &[], &["px"]))
        .unwrap_err();
    assert!(
        matches!(&error, Error::Conflict { path, .. } if path == "alias \"px\" in branch \"standard\" of Symbol, held by Price"),
        "{error}"
    );
    assert_eq!(registry, before);
    assert_eq!(
        probe(&registry, 55, 66, "SYMBOL", "Sym"),
        [Some("SYMBOL"); 4]
    );
}

#[test]
fn a_merge_follows_the_truth_table() {
    let mut stored = full("Symbol", 55, &[65, 66], &["Ticker", "Sym"]);
    stored.insert_metadata("display", "Symbol").unwrap();
    stored.insert_metadata("owner", "stored").unwrap();
    let mut registry = FixRegistry::from_fields([stored]).unwrap();

    let mut incoming = DataType::Utf8.required_field("SYMBOL");
    incoming.as_fix_mut().set_tag(55).unwrap();
    incoming.as_fix_mut().set_tags(&[67, 66]).unwrap();
    incoming
        .as_fix_mut()
        .set_aliases(["Instrument", "sym"])
        .unwrap();
    incoming
        .insert_metadata("display", "Ticker symbol")
        .unwrap();
    incoming.insert_metadata("source", "incoming").unwrap();
    registry.update(incoming).unwrap();

    let merged = registry.field_by_tag(55).unwrap();
    // The incoming field wins the spelling, nullability and every shared key.
    assert_eq!(merged.name(), "SYMBOL");
    assert!(!merged.is_nullable());
    assert_eq!(merged.display(), Some("Ticker symbol"));
    // The stored field keeps what only it declared.
    assert_eq!(merged.get_metadata("owner"), Some("stored"));
    assert_eq!(merged.get_metadata("source"), Some("incoming"));
    assert_eq!(merged.as_fix().description(), Some("Symbol described"));
    // Lists concatenate, incoming first, deduplicated with case folded.
    assert_eq!(merged.as_fix().tags().unwrap(), [67, 66, 65]);
    assert_eq!(
        merged.as_fix().aliases().collect::<Vec<_>>(),
        ["Instrument", "sym", "Ticker"]
    );
    // Every key, old and new, resolves to the merged field.
    for tag in [55, 65, 66, 67] {
        assert_eq!(
            registry.field_by_tag(tag).unwrap().name(),
            "SYMBOL",
            "{tag}"
        );
    }
    for name in ["symbol", "ticker", "SYM", "instrument"] {
        assert_eq!(
            registry
                .field_by_name(&FixBranch::STANDARD, name)
                .unwrap()
                .name(),
            "SYMBOL",
            "{name}"
        );
    }
    assert_eq!(registry.len(), 1);

    // A merge that adds nothing is a no-op.
    let before = registry.clone();
    registry.update(tagged("symbol", 55)).unwrap();
    assert_eq!(
        registry.field_by_tag(55).unwrap().as_fix().tags().unwrap(),
        [67, 66, 65]
    );
    assert_eq!(registry.field_by_tag(55).unwrap().name(), "symbol");
    assert_eq!(registry.len(), before.len());
}

#[test]
fn a_rejected_merge_leaves_the_registry_untouched() {
    let mut registry = FixRegistry::from_fields([
        full("Symbol", 55, &[65], &["Ticker"]),
        full("Price", 44, &[31], &["Px"]),
    ])
    .unwrap();
    let before = registry.clone();
    let snapshot = format!("{registry:?}");

    // A datatype disagreement names both datatypes and never widens.
    let mut widened = tagged("Symbol", 55);
    widened.set_dtype(DataType::LargeUtf8).unwrap();
    let error = registry.update(widened).unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("utf8") && message.contains("large_utf8"),
        "{message}"
    );

    // A name disagreement names both spellings.
    let error = registry.update(tagged("Sym", 55)).unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("\"Symbol\"") && message.contains("\"Sym\""),
        "{message}"
    );

    // A merged alternate key another field holds is a conflict naming both.
    let error = registry.update(full("Symbol", 55, &[31], &[])).unwrap_err();
    assert!(
        matches!(&error, Error::Conflict { path, .. } if path.ends_with(", held by Price")),
        "{error}"
    );
    let error = registry
        .update(full("Symbol", 55, &[], &["PX"]))
        .unwrap_err();
    assert!(error.is_conflict(), "{error}");

    // An unknown identifier is an absence, not a silent insert.
    let error = registry.update(tagged("Text", 58)).unwrap_err();
    assert!(
        matches!(&error, Error::Absent { path, .. } if path == "identifier standard:58"),
        "{error}"
    );

    // A branch disagreement is that same absence: the branch is half
    // of the identity, so the incoming field names no stored one.
    let error = registry
        .update(identified(
            "Symbol",
            &FixId::from_parts(cme(), 5055).unwrap(),
        ))
        .unwrap_err();
    assert!(
        matches!(&error, Error::Absent { path, .. } if path == "identifier cme:5055"),
        "{error}"
    );

    assert_eq!(registry, before);
    assert_eq!(format!("{registry:?}"), snapshot);
    assert_eq!(
        probe(&registry, 55, 65, "symbol", "ticker"),
        [Some("Symbol"); 4]
    );
    assert_eq!(probe(&registry, 44, 31, "price", "px"), [Some("Price"); 4]);
    assert_eq!(registry.len(), 2);
}

#[test]
fn removal_keeps_every_position_consistent() {
    let mut registry = FixRegistry::from_fields([
        full("Symbol", 55, &[65], &["Ticker"]),
        full("Price", 44, &[45], &["Px"]),
        full("Text", 58, &[59], &["FreeText"]),
    ])
    .unwrap();

    // Removing the first field moves the last into its slot.
    let removed = registry.remove(55).unwrap();
    assert_eq!(removed.name(), "Symbol");
    assert_eq!(registry.len(), 2);
    assert_eq!(probe(&registry, 55, 65, "symbol", "ticker"), [None; 4]);
    assert_eq!(probe(&registry, 44, 45, "price", "px"), [Some("Price"); 4]);
    assert_eq!(
        probe(&registry, 58, 59, "text", "freetext"),
        [Some("Text"); 4]
    );
    assert_eq!(
        registry.iter().map(Field::name).collect::<Vec<_>>(),
        ["Price", "Text"]
    );

    // A name key removes through the alias tier too; a path never does.
    assert!(registry.remove("Symbol").is_none());
    assert_eq!(registry.remove("PX").unwrap().name(), "Price");
    assert_eq!(probe(&registry, 44, 45, "price", "px"), [None; 4]);
    assert_eq!(
        probe(&registry, 58, 59, "text", "freetext"),
        [Some("Text"); 4]
    );
    assert_eq!(registry.len(), 1);
    assert_eq!(registry.remove("FreeText").unwrap().name(), "Text");
    assert!(registry.is_empty());
    assert!(registry.remove(58).is_none());

    // A removed key can be claimed again.
    registry
        .insert(full("Text", 58, &[59], &["FreeText"]))
        .unwrap();
    assert_eq!(
        probe(&registry, 58, 59, "text", "freetext"),
        [Some("Text"); 4]
    );
}

#[test]
fn specialized_and_generic_accessors_answer_alike_for_every_key() {
    let registry = FixRegistry::from_fields([
        full("Symbol", 55, &[65], &["Ticker"]),
        full("Price", 44, &[], &[]),
    ])
    .unwrap();

    // Tag hit, alternate-tag hit, and a tag miss.
    for tag in [55, 65, 44, 1] {
        assert_eq!(
            registry.get_field(tag),
            registry.get_field_by_tag(tag),
            "{tag}"
        );
        assert_eq!(
            registry.get_field(FixKey::Tag(tag)),
            registry.get_field_by_tag(tag)
        );
        assert_eq!(
            registry.field(tag).map(Field::name).ok(),
            registry.field_by_tag(tag).map(Field::name).ok()
        );
        assert_eq!(
            registry.contains(tag),
            registry.get_field_by_tag(tag).is_some()
        );
    }
    // Name hit, alias hit, and a name miss.
    for name in ["symbol", "TICKER", "price", "absent"] {
        assert_eq!(
            registry.get_field(name),
            registry.get_field_by_name(&FixBranch::STANDARD, name),
            "{name}"
        );
        assert_eq!(
            registry.get_field(&name.to_owned()),
            registry.get_field_by_name(&FixBranch::STANDARD, name)
        );
        assert_eq!(
            registry.field(name).map(Field::name).ok(),
            registry
                .field_by_name(&FixBranch::STANDARD, name)
                .map(Field::name)
                .ok()
        );
        assert_eq!(
            registry.contains(name),
            registry
                .get_field_by_name(&FixBranch::STANDARD, name)
                .is_some()
        );
    }

    // The failing halves name the key the way it was asked.
    let by_tag = registry.field_by_tag(1).unwrap_err();
    assert!(matches!(&by_tag, Error::Absent { expected: "fix field", path } if path == "tag 1"));
    let by_name = registry
        .field_by_name(&FixBranch::STANDARD, "absent")
        .unwrap_err();
    assert!(matches!(&by_name, Error::Absent { path, .. } if path == "name \"absent\""));
    let by_path = registry
        .field_by_path(&FixBranch::STANDARD, "Symbol.absent")
        .unwrap_err();
    assert!(matches!(&by_path, Error::Absent { path, .. } if path == "path \"Symbol.absent\""));
    assert_eq!(
        registry.field(1).unwrap_err().to_string(),
        by_tag.to_string()
    );
    assert_eq!(
        registry.field("Symbol.absent").unwrap_err().to_string(),
        by_path.to_string()
    );
}

#[test]
fn a_path_reaches_a_component_member_and_a_repeating_group_member() {
    let mut party_id = DataType::Utf8.nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448).unwrap();
    let mut role = DataType::Int32.nullable_field("PartyRole");
    role.as_fix_mut().set_tag(452).unwrap();
    let mut group = DataType::list(
        DataType::from_fields([party_id.clone(), role])
            .unwrap()
            .required_field("item"),
    )
    .nullable_field("NoPartyIDs");
    group.as_fix_mut().set_tag(453).unwrap();
    let mut instrument = DataType::from_fields([tagged("Symbol", 55), tagged("SecurityID", 48)])
        .unwrap()
        .nullable_field("Instrument");
    instrument.as_fix_mut().set_tag(1000).unwrap();
    instrument.as_fix_mut().set_aliases(["Instr"]).unwrap();

    let registry = FixRegistry::from_fields([group, instrument]).unwrap();
    assert_eq!(
        registry
            .field_by_path(&FixBranch::STANDARD, "NoPartyIDs")
            .unwrap()
            .as_fix()
            .tag()
            .unwrap(),
        Some(453)
    );
    assert_eq!(
        registry
            .field_by_path(&FixBranch::STANDARD, "NoPartyIDs.PartyID")
            .unwrap(),
        &party_id
    );
    assert_eq!(
        registry
            .field_by_path(&FixBranch::STANDARD, "nopartyids.item.PartyRole")
            .unwrap()
            .name(),
        "PartyRole"
    );
    assert_eq!(
        registry
            .field_by_path(&FixBranch::STANDARD, "Instrument.Symbol")
            .unwrap()
            .name(),
        "Symbol"
    );
    assert_eq!(
        registry
            .field_by_path(&FixBranch::STANDARD, "instr.SecurityID")
            .unwrap()
            .name(),
        "SecurityID"
    );
    assert_eq!(
        registry.get_field("Instrument.Symbol"),
        registry.get_field_by_path(&FixBranch::STANDARD, "Instrument.Symbol")
    );
    assert!(registry.contains("NoPartyIDs.PartyID"));
    // A member is reached through its parent only: the registry does not
    // index it, and the remainder of a path is an exact child name.
    assert!(
        registry
            .get_field_by_name(&FixBranch::STANDARD, "PartyID")
            .is_none()
    );
    assert!(
        registry
            .get_field_by_path(&FixBranch::STANDARD, "NoPartyIDs.partyid")
            .is_none()
    );
    assert!(
        registry
            .get_field_by_path(&FixBranch::STANDARD, "Instrument.Absent")
            .is_none()
    );
    assert!(
        registry
            .get_field_by_path(&FixBranch::STANDARD, "Absent.Symbol")
            .is_none()
    );
}

#[test]
fn iteration_follows_the_canonical_tag_and_equality_ignores_order() {
    let fields = [
        tagged("Text", 58),
        tagged("Symbol", 55),
        tagged("Account", 1),
    ];
    let registry = FixRegistry::from_fields(fields.clone()).unwrap();
    let mut reversed = fields.clone();
    reversed.reverse();
    let other = FixRegistry::from_fields(reversed).unwrap();

    let mut iter = registry.iter();
    assert_eq!(iter.len(), 3);
    assert_eq!(iter.next().map(Field::name), Some("Account"));
    assert_eq!(iter.next_back().map(Field::name), Some("Text"));
    assert_eq!(iter.len(), 1);
    assert_eq!(iter.next().map(Field::name), Some("Symbol"));
    assert!(iter.next().is_none());
    assert_eq!(
        (&registry).into_iter().map(Field::name).collect::<Vec<_>>(),
        ["Account", "Symbol", "Text"]
    );
    // The cursor form walks the same order, and a binding advancing it with
    // only the last identifier it saw sees every field exactly once.
    let mut walked = Vec::new();
    let mut cursor = None;
    while let Some(field) = registry.next_field_after(cursor.as_ref()) {
        walked.push(field.name());
        cursor = field.as_fix().id().unwrap();
    }
    assert_eq!(walked, ["Account", "Symbol", "Text"]);
    assert!(
        registry
            .next_field_after(Some(&FixId::standard(i32::MAX)))
            .is_none()
    );
    assert!(FixRegistry::new().next_field_after(None).is_none());
    // An alternate tag is an index entry, never a cursor stop.
    let mut aliased = tagged("MsgType", 35);
    aliased.as_fix_mut().set_tags(&[2]).unwrap();
    let with_alternate = FixRegistry::from_fields([aliased, tagged("Account", 1)]).unwrap();
    assert_eq!(
        with_alternate
            .next_field_after(Some(&FixId::standard(1)))
            .map(Field::name)
            .unwrap(),
        "MsgType"
    );

    assert_eq!(registry, other);
    assert_ne!(registry, FixRegistry::new());
    assert_eq!(FixRegistry::new(), FixRegistry::default());
    assert!(FixRegistry::new().is_empty());
    assert_eq!(FixRegistry::new().iter().len(), 0);

    // A conflict anywhere fails the whole build.
    let error = FixRegistry::from_fields([tagged("Text", 58), tagged("Symbol", 58)]).unwrap_err();
    assert!(error.is_conflict(), "{error}");

    // Debug renders the fields under their identifiers, in order.
    let rendered = format!("{registry:?}");
    assert!(rendered.starts_with("{\"standard:1\": "), "{rendered}");
    assert!(rendered.find("\"standard:55\"").unwrap() < rendered.find("\"standard:58\"").unwrap());
}

#[test]
fn iteration_and_the_cursor_are_branch_major() {
    let cme = cme();
    let registry = FixRegistry::from_fields([
        tagged("Account", 1),
        identified("TradeID", &FixId::from_parts(cme.clone(), 5001).unwrap()),
        tagged("MsgType", 35),
        identified("Venue", &FixId::from_parts(cme.clone(), 9000).unwrap()),
    ])
    .unwrap();
    // `cme` sorts before `standard`, and each branch ascends by tag.
    assert_eq!(
        registry.iter().map(Field::name).collect::<Vec<_>>(),
        ["TradeID", "Venue", "Account", "MsgType"]
    );
    let mut walked = Vec::new();
    let mut cursor = None;
    while let Some(field) = registry.next_field_after(cursor.as_ref()) {
        walked.push(field.name());
        cursor = field.as_fix().id().unwrap();
    }
    assert_eq!(walked, ["TradeID", "Venue", "Account", "MsgType"]);
    let rendered = format!("{registry:?}");
    assert!(rendered.starts_with("{\"cme:5001\": "), "{rendered}");
}

#[test]
fn nestedness_routes_a_field_by_the_core_predicate_alone() {
    // `DataType::is_nested` is the whole rule, and it already unwraps a
    // dictionary to its value type: a dictionary-encoded Struct is nested and
    // a dictionary-encoded Utf8 is not.
    let encoded_text = {
        let mut field = DataType::dictionary(DataType::Int32, DataType::Utf8)
            .unwrap()
            .nullable_field("Coded");
        field.as_fix_mut().set_tag(60).unwrap();
        field
    };
    let encoded_struct = {
        let inner = DataType::from_fields([tagged("Member", 9_003)]).unwrap();
        let mut field = DataType::dictionary(DataType::Int32, inner)
            .unwrap()
            .nullable_field("Boxed");
        field.as_fix_mut().set_tag(61).unwrap();
        field
    };
    let fields = [
        (tagged("Symbol", 55), false),
        (encoded_text, false),
        (component("Instrument", 1_000), true),
        (group("NoPartyIDs", 453), true),
        (encoded_struct, true),
    ];
    let registry = FixRegistry::from_fields(fields.iter().map(|(field, _)| field.clone())).unwrap();
    for (field, nested) in &fields {
        let id = field.as_fix().id().unwrap().unwrap();
        assert_eq!(
            field.dtype().is_nested(),
            *nested,
            "{} is the core predicate's answer",
            field.name()
        );
        assert_eq!(
            registry.indexed_as_nested(&id),
            Some(*nested),
            "{} landed in the wrong half",
            field.name()
        );
    }
    assert_eq!(registry.len(), 5);
}

#[test]
fn the_primitive_half_is_read_first_without_reordering_the_tiers() {
    // The split partitions each index; it is not a fifth tier above them. A
    // canonical key of one half therefore still beats an alternate key of the
    // other, in both directions.
    let mut shadowing = tagged("Shadow", 5);
    shadowing.as_fix_mut().set_tags(&[453]).unwrap();
    shadowing.as_fix_mut().set_aliases(["Parties"]).unwrap();
    let mut parties = group("Parties", 453);
    parties.as_fix_mut().set_aliases(["Group"]).unwrap();
    let registry = FixRegistry::from_fields([shadowing.clone(), parties.clone()]).unwrap();
    // The nested field holds 453 and the name "Parties" canonically; the
    // primitive one holds them as an alternate tag and an alias.
    assert_eq!(
        registry.get_field_by_tag(453).map(Field::name),
        Some("Parties")
    );
    assert_eq!(
        registry
            .get_field_by_name(&FixBranch::STANDARD, "parties")
            .map(Field::name),
        Some("Parties")
    );
    assert_eq!(
        registry.get_field_by_tag(5).map(Field::name),
        Some("Shadow")
    );

    // The mirror: the primitive field holds them canonically and the nested
    // one as an alternate tag and an alias.
    let mut price = tagged("Price", 44);
    price.as_fix_mut().set_aliases(["Rate"]).unwrap();
    let mut legs = group("NoLegs", 555);
    legs.as_fix_mut().set_tags(&[44]).unwrap();
    legs.as_fix_mut().set_aliases(["Price"]).unwrap();
    let registry = FixRegistry::from_fields([legs.clone(), price.clone()]).unwrap();
    assert_eq!(
        registry.get_field_by_tag(44).map(Field::name),
        Some("Price")
    );
    assert_eq!(
        registry
            .get_field_by_name(&FixBranch::STANDARD, "PRICE")
            .map(Field::name),
        Some("Price")
    );
    assert_eq!(
        registry
            .get_field_by_name(&FixBranch::STANDARD, "rate")
            .map(Field::name),
        Some("Price"),
        "an alias the nested half does not hold still resolves"
    );
    assert_eq!(
        registry.get_field_by_tag(555).map(Field::name),
        Some("NoLegs")
    );
}

#[test]
fn a_nested_field_can_never_claim_a_primitive_field_key() {
    // The identity space is not split, so every conflict a pair of primitives
    // would raise, a primitive and a nested field raise too - naming both.
    let stored = full("Symbol", 55, &[65], &["Ticker"]);
    let registry = FixRegistry::from_fields([stored]).unwrap();
    let mut same_alternate = component("Legs", 5_556);
    same_alternate.as_fix_mut().set_tags(&[65]).unwrap();
    let mut same_alias = component("Legs", 5_557);
    same_alias.as_fix_mut().set_aliases(["ticker"]).unwrap();
    let cases = [
        (group("Parties", 55), "identifier standard:55"),
        (
            group("symbol", 5_555),
            "name \"symbol\" in branch \"standard\"",
        ),
        (same_alternate, "alternate identifier standard:65"),
        (same_alias, "alias \"ticker\" in branch \"standard\""),
    ];
    for (claimant, key) in cases {
        let claiming = claimant.name().to_owned();
        let mut probed = registry.clone();
        let error = probed.insert(claimant).unwrap_err();
        let Error::Conflict { path, .. } = &error else {
            panic!("{key}: {error}");
        };
        assert!(path.contains(key), "{path}");
        assert!(path.contains(&claiming), "{path}");
        assert!(path.ends_with(", held by Symbol"), "{path}");
        // Nothing was written: the registry still holds the one field.
        assert_eq!(probed, registry, "{key}: a refusal changes nothing");
        assert_eq!(probed.len(), 1);
    }

    // And the other direction: a primitive claiming a nested field's key.
    let mut registry = FixRegistry::from_fields([group("NoPartyIDs", 453)]).unwrap();
    let error = registry.insert(tagged("Parties", 453)).unwrap_err();
    let Error::Conflict { path, .. } = &error else {
        panic!("{error}");
    };
    assert!(path.contains("identifier standard:453"), "{path}");
    assert!(path.ends_with(", held by NoPartyIDs"), "{path}");
    assert_eq!(registry.len(), 1);
}

#[test]
fn iteration_and_the_cursor_merge_both_halves_in_identifier_order() {
    let registry = FixRegistry::from_fields([
        tagged("Account", 1),
        group("NoPartyIDs", 453),
        tagged("Symbol", 55),
        component("Instrument", 1_000),
        tagged("Text", 58),
    ])
    .unwrap();
    let expected = ["Account", "Symbol", "Text", "NoPartyIDs", "Instrument"];
    assert_eq!(
        registry.iter().map(Field::name).collect::<Vec<_>>(),
        expected
    );
    assert_eq!(registry.iter().len(), 5);
    assert_eq!(registry.len(), 5);
    assert!(!registry.is_empty());

    // The same order from the back, and the two ends meet without yielding an
    // entry twice or losing one between them.
    let mut backwards: Vec<&str> = registry.iter().rev().map(Field::name).collect();
    backwards.reverse();
    assert_eq!(backwards, expected);
    let mut iter = registry.iter();
    let mut ends = Vec::new();
    while let Some(front) = iter.next() {
        ends.push(front.name());
        if let Some(back) = iter.next_back() {
            ends.push(back.name());
        }
    }
    ends.sort_unstable();
    let mut sorted = expected;
    sorted.sort_unstable();
    assert_eq!(ends, sorted);

    // The cursor a binding advances walks the merge too.
    let mut walked = Vec::new();
    let mut cursor = None;
    while let Some(field) = registry.next_field_after(cursor.as_ref()) {
        walked.push(field.name());
        cursor = field.as_fix().id().unwrap();
    }
    assert_eq!(walked, expected);

    // Equality and Debug span both halves.
    let mut reversed: Vec<Field> = registry.iter().cloned().collect();
    reversed.reverse();
    assert_eq!(registry, FixRegistry::from_fields(reversed).unwrap());
    let rendered = format!("{registry:?}");
    assert!(rendered.starts_with("{\"standard:1\": "), "{rendered}");
    assert!(
        rendered.find("\"standard:453\"").unwrap() < rendered.find("\"standard:1000\"").unwrap(),
        "{rendered}"
    );

    // One half alone still iterates whole.
    let nested_only =
        FixRegistry::from_fields([group("NoPartyIDs", 453), component("Instrument", 1_000)])
            .unwrap();
    assert_eq!(
        nested_only.iter().map(Field::name).collect::<Vec<_>>(),
        ["NoPartyIDs", "Instrument"]
    );
    assert_eq!(
        nested_only
            .iter()
            .rev()
            .map(Field::name)
            .collect::<Vec<_>>(),
        ["Instrument", "NoPartyIDs"]
    );
}

#[test]
fn shard_arithmetic_picks_the_one_shard_that_holds_a_tag() {
    assert_eq!(shard_of(0), 0);
    assert_eq!(shard_of(99), 0);
    assert_eq!(shard_of(100), 1);
    assert_eq!(shard_of(101), 1);
    assert_eq!(shard_of(10_000), 100);
    assert_eq!(shard_of(i32::MAX), 21_474_836);
}

#[test]
fn the_default_resolves_in_the_documented_order_from_explicit_inputs() {
    let root = scratch("autoload");
    let location = root.join("location");
    let home = root.join("home");
    let config = Folder::new(home.join(".config")).unwrap();

    // Nothing configured, or no home at all: the empty registry.
    assert!(autoload(None, Some(config.clone())).unwrap().is_empty());
    assert!(autoload(None, None).unwrap().is_empty());

    // A present dictionary under the configuration directory loads.
    let mut configured = Folder::new(home.join(".config").join("fix")).unwrap();
    FixRegistry::from_fields([tagged("Symbol", 55)])
        .unwrap()
        .write_into(&mut configured)
        .unwrap();
    let loaded = autoload(None, Some(config.clone())).unwrap();
    assert_eq!(loaded.field_by_tag(55).unwrap().name(), "Symbol");

    // The explicit location beats it, spelled as a path or as a URL.
    let mut located = Folder::new(&location).unwrap();
    FixRegistry::from_fields([tagged("Price", 44)])
        .unwrap()
        .write_into(&mut located)
        .unwrap();
    let as_path = location.to_string_lossy().into_owned();
    let as_url = located.url().to_string();
    for spelling in [as_path, as_url] {
        let loaded = autoload(Some(&spelling), Some(config.clone())).unwrap();
        assert_eq!(loaded.len(), 1, "{spelling}");
        assert_eq!(
            loaded.field_by_tag(44).unwrap().name(),
            "Price",
            "{spelling}"
        );
        assert!(loaded.get_field_by_tag(55).is_none(), "{spelling}");
    }

    // A location that is set but names nothing, or a scheme this crate has no
    // backend for, is an error - never the empty registry.
    let missing = root.join("missing").to_string_lossy().into_owned();
    let error = autoload(Some(&missing), Some(config.clone())).unwrap_err();
    assert!(error.is_absent(), "{error}");
    let error = autoload(Some("mem://1/fix"), Some(config.clone())).unwrap_err();
    assert!(error.to_string().contains("scheme mem"), "{error}");

    // A malformed shard anywhere is an error naming the shard.
    std::fs::write(
        location.join("primitive").join("standard").join("0.json"),
        b"not json",
    )
    .unwrap();
    let error = autoload(Some(&location.to_string_lossy()), None).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("0.json"), "{message}");
    std::fs::write(home.join(".config/fix/primitive/standard/0.json"), b"[1]").unwrap();
    let error = autoload(None, Some(config)).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("0.json"), "{message}");
    assert!(message.contains("field mapping"), "{message}");

    let _ = std::fs::remove_dir_all(&root);
}

/// A registry, a root and a value the message tests share.
fn order() -> (Arc<FixRegistry>, Field, Scalar) {
    let mut party_id = DataType::Utf8.nullable_field("PartyID");
    party_id.as_fix_mut().set_tag(448).unwrap();
    let mut role = DataType::Int32.nullable_field("PartyRole");
    role.as_fix_mut().set_tag(452).unwrap();
    let item = DataType::from_fields([party_id, role])
        .unwrap()
        .required_field("item");
    let mut group = DataType::list(item).nullable_field("NoPartyIDs");
    group.as_fix_mut().set_tag(453).unwrap();
    let mut instrument = DataType::from_fields([tagged("Symbol", 55)])
        .unwrap()
        .nullable_field("Instrument");
    instrument.as_fix_mut().set_tag(1000).unwrap();
    let mut qty = DataType::Int64.required_field("OrderQty");
    qty.as_fix_mut().set_tag(38).unwrap();
    qty.as_fix_mut().set_aliases(["Qty"]).unwrap();
    let registry = Arc::new(
        FixRegistry::from_fields([
            group.clone(),
            instrument.clone(),
            qty.clone(),
            tagged("Symbol", 55),
        ])
        .unwrap(),
    );
    let root = DataType::from_fields([
        qty,
        instrument,
        group,
        DataType::Utf8.nullable_field("9999"),
    ])
    .unwrap()
    .required_field("NewOrderSingle");
    let value = Scalar::from_record([
        ("OrderQty", Scalar::I64(100)),
        (
            "Instrument",
            Scalar::from_record([("Symbol", Scalar::from("AAPL"))]).unwrap(),
        ),
        (
            "NoPartyIDs",
            Scalar::from_sequence([
                Scalar::from_record([
                    ("PartyID", Scalar::from("BROKER")),
                    ("PartyRole", Scalar::I64(1)),
                ])
                .unwrap(),
                Scalar::from_record([
                    ("PartyID", Scalar::from("CLIENT")),
                    ("PartyRole", Scalar::I64(3)),
                ])
                .unwrap(),
            ]),
        ),
        ("9999", Scalar::from("custom")),
    ])
    .unwrap();
    (registry, root, value)
}

#[test]
fn a_message_resolves_values_through_its_registry() {
    let (registry, root, value) = order();
    let msg = FixMsg::with_registry(Arc::clone(&registry), root.clone(), value).unwrap();
    assert!(Arc::ptr_eq(msg.registry(), &registry));
    assert_eq!(msg.as_field(), &root);

    // A record input canonicalizes to the ordered sequence the root declares.
    let row = msg.as_value().as_sequence().unwrap();
    assert_eq!(row.len(), 4);
    assert_eq!(row[0], Scalar::I64(100));
    assert_eq!(row[3], Scalar::from("custom"));

    // By tag, through the registry's canonical name.
    assert_eq!(msg.by_tag(38).unwrap(), &Scalar::I64(100));
    // By name, folded through the registry, and by alias.
    assert_eq!(msg.by_name("orderqty").unwrap(), &Scalar::I64(100));
    assert_eq!(msg.by_name("QTY").unwrap(), &Scalar::I64(100));
    // An unknown tag is kept under its rendered name.
    assert_eq!(msg.by_tag(9999).unwrap(), &Scalar::from("custom"));
    assert_eq!(msg.by_name("9999").unwrap(), &Scalar::from("custom"));
    // A path descends a component by name and a group by index.
    assert_eq!(
        msg.by_path("Instrument.symbol").unwrap(),
        &Scalar::from("AAPL")
    );
    assert_eq!(
        msg.by_path("NoPartyIDs.1.PartyID").unwrap(),
        &Scalar::from("CLIENT")
    );
    assert_eq!(
        msg.by_path("nopartyids.0.PartyRole").unwrap(),
        &Scalar::I32(1)
    );
    assert_eq!(msg.by_path("NoPartyIDs").unwrap().len(), 2);
    assert!(
        msg.get_by_path("NoPartyIDs.PartyID").is_none(),
        "a group member needs its index"
    );
    assert!(msg.get_by_path("NoPartyIDs.2.PartyID").is_none());
    assert!(msg.get_by_path("OrderQty.deeper").is_none());
    assert!(
        msg.get_by_tag(55).is_none(),
        "Symbol is nested, not a root child"
    );
    // A member the registry does not know still matches its exact spelling.
    assert_eq!(
        msg.by_path("NoPartyIDs.0.PartyID").unwrap(),
        &Scalar::from("BROKER")
    );
    assert!(msg.get_by_path("NoPartyIDs.0.partyid").is_none());
    assert!(msg.get_by_tag(-1).is_none());

    // The generic pair matches the specialized one for every key.
    for tag in [38, 9999, 55, 453] {
        assert_eq!(msg.get(tag), msg.get_by_tag(tag), "{tag}");
        assert_eq!(msg.value(tag).ok(), msg.by_tag(tag).ok(), "{tag}");
    }
    for name in [
        "OrderQty",
        "qty",
        "Instrument.Symbol",
        "NoPartyIDs.1.PartyID",
        "absent",
    ] {
        assert_eq!(msg.get(name), msg.get_by_path(name), "{name}");
        assert_eq!(msg.value(name).ok(), msg.by_path(name).ok(), "{name}");
    }
    let error = msg.by_tag(55).unwrap_err();
    assert!(matches!(&error, Error::Absent { expected: "fix value", path } if path == "tag 55"));
    let error = msg.by_path("absent.x").unwrap_err();
    assert!(matches!(&error, Error::Absent { path, .. } if path == "path \"absent.x\""));

    // Equality and hashing follow the schema and the value.
    let same = FixMsg::with_registry(Arc::clone(&registry), root, msg.as_value().clone()).unwrap();
    assert_eq!(msg, same);
    assert_eq!(crate::stable_hash_of(&msg), crate::stable_hash_of(&same));
}

#[test]
fn a_message_rejects_a_value_its_field_refuses() {
    let (registry, root, _) = order();
    let error = FixMsg::with_registry(
        Arc::clone(&registry),
        root.clone(),
        Scalar::from_record([("OrderQty", Scalar::from("many"))]).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    assert!(error.to_string().contains("OrderQty"), "{error}");

    let error = FixMsg::with_registry(
        registry,
        DataType::Int64.required_field("scalar"),
        Scalar::I64(1),
    )
    .unwrap_err();
    assert!(error.to_string().contains("struct root"), "{error}");
}

#[test]
fn a_message_resolves_a_bare_tag_in_its_own_branch_then_the_standard_one() {
    let cme = cme();
    // Tag 5001 is defined in both dictionaries; 35 only in the standard one.
    let mut venue_trade = identified("TradeID", &FixId::from_parts(cme.clone(), 5001).unwrap());
    venue_trade.as_fix_mut().set_aliases(["TID"]).unwrap();
    let mut spec_trade = tagged("SecondaryTradeID", 5001);
    spec_trade.as_fix_mut().set_aliases(["STID"]).unwrap();
    let msg_type = tagged("MsgType", 35);
    let registry = Arc::new(
        FixRegistry::from_fields([venue_trade.clone(), spec_trade.clone(), msg_type.clone()])
            .unwrap(),
    );

    // The root declares the venue's branch, so the message speaks it.
    let mut root = DataType::from_fields([venue_trade.clone(), msg_type.clone()])
        .unwrap()
        .required_field("VenueExecutionReport");
    root.as_fix_mut().set_branch(&cme).unwrap();
    let value = Scalar::from_record([
        ("TradeID", Scalar::from("T-1")),
        ("MsgType", Scalar::from("8")),
    ])
    .unwrap();
    let msg = FixMsg::with_registry(Arc::clone(&registry), root, value).unwrap();
    assert_eq!(msg.branch(), &cme);

    // Step one: the message's own branch.
    assert_eq!(msg.by_tag(5001).unwrap(), &Scalar::from("T-1"));
    assert_eq!(msg.by_name("tid").unwrap(), &Scalar::from("T-1"));
    // Step two: the standard branch, so MsgType stays reachable.
    assert_eq!(msg.by_tag(35).unwrap(), &Scalar::from("8"));
    assert_eq!(msg.by_name("msgtype").unwrap(), &Scalar::from("8"));
    // The standard field 5001 names a root child this message does not hold,
    // so its alias misses rather than answering the venue's value.
    assert!(msg.get_by_name("stid").is_none());

    // An identifier is exact and never tiers.
    let venue_id = FixId::from_parts(cme.clone(), 5001).unwrap();
    assert_eq!(msg.by_id(&venue_id).unwrap(), &Scalar::from("T-1"));
    assert!(
        msg.get_by_id(&FixId::standard(5001)).is_none(),
        "a foreign branch misses"
    );
    assert_eq!(msg.get(&venue_id), msg.get_by_id(&venue_id));
    assert_eq!(msg.value(&venue_id).unwrap(), msg.by_id(&venue_id).unwrap());
    let error = msg.by_id(&FixId::standard(5001)).unwrap_err();
    assert!(
        matches!(&error, Error::Absent { expected: "fix value", path } if path == "identifier standard:5001"),
        "{error}"
    );

    // A standard message resolves only in the standard branch.
    let plain_root = DataType::from_fields([spec_trade, msg_type])
        .unwrap()
        .required_field("ExecutionReport");
    let plain = FixMsg::with_registry(
        registry,
        plain_root,
        Scalar::from_record([
            ("SecondaryTradeID", Scalar::from("S-1")),
            ("MsgType", Scalar::from("8")),
        ])
        .unwrap(),
    )
    .unwrap();
    assert_eq!(plain.branch(), &FixBranch::STANDARD);
    assert_eq!(plain.by_tag(5001).unwrap(), &Scalar::from("S-1"));
    assert_eq!(plain.by_name("stid").unwrap(), &Scalar::from("S-1"));
    assert!(plain.get_by_name("tid").is_none());
    assert!(plain.get_by_id(&venue_id).is_none());
}

#[test]
fn a_message_rejects_a_root_whose_branch_is_corrupt() {
    let mut root = DataType::from_fields([tagged("MsgType", 35)])
        .unwrap()
        .required_field("row");
    root.insert_metadata("fix:branch", "2cme").unwrap();
    let error = FixMsg::with_registry(
        Arc::new(FixRegistry::new()),
        root,
        Scalar::from_record([("MsgType", Scalar::from("8"))]).unwrap(),
    )
    .unwrap_err();
    assert!(
        matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "fix:branch"),
        "{error}"
    );
}
