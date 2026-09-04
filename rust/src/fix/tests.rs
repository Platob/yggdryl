//! Focused edge cases of the FIX module, driven with explicit inputs.

use std::path::PathBuf;
use std::sync::Arc;

use super::global::autoload;
use super::registry::Folded;
use super::store::shard_of;
use crate::local::Folder;
use crate::{DataType, Error, Field, FixKey, FixMsg, FixRegistry, Scalar};

/// A nullable text field carrying one canonical tag.
fn tagged(name: &str, tag: i32) -> Field {
    let mut field = DataType::Utf8.nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
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
    [
        registry.get_field_by_tag(tag).map(Field::name),
        registry.get_field_by_tag(alternate).map(Field::name),
        registry.get_field_by_name(name).map(Field::name),
        registry.get_field_by_name(alias).map(Field::name),
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
fn a_corrupt_stored_property_is_reported_under_its_full_key() {
    let cases = [
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
        let error = if key == "fix:tag" {
            field.as_fix().tag().unwrap_err()
        } else {
            field.as_fix().tags().unwrap_err()
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
            registry.field_by_name(query).unwrap().name(),
            "Symbol",
            "{query}"
        );
        assert_eq!(registry.field(query).unwrap().name(), "Symbol", "{query}");
        assert!(registry.contains(query), "{query}");
    }
    assert_eq!(
        registry.field_by_name("clientorderid").unwrap().name(),
        "ClOrdID"
    );
    assert!(registry.get_field_by_name("Symbols").is_none());
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
        assert_eq!(registry.field_by_name("px").unwrap().name(), "Px");
        assert_eq!(registry.field_by_name("Price").unwrap().name(), "Px");
        assert_eq!(
            registry.field_by_name("LastPrice").unwrap().name(),
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
    assert_eq!(registry.field_by_name("35").unwrap().name(), "35");
    assert!(registry.get_field_by_name("1").is_none());
    assert!(registry.get_field_by_tag(2).is_none());
}

#[test]
fn an_insert_conflict_names_both_fields_for_each_key_kind() {
    let stored = full("Symbol", 55, &[65], &["Ticker"]);
    let registry = FixRegistry::from_fields([stored]).unwrap();
    let cases = [
        (full("SymbolSfx", 55, &[], &[]), "tag 55"),
        (full("symbol", 56, &[], &[]), "name \"symbol\""),
        (full("SymbolSfx", 56, &[65], &[]), "alternate tag 65"),
        (full("SymbolSfx", 56, &[], &["TICKER"]), "alias \"TICKER\""),
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
    assert_eq!(registry.field_by_name("Ticker").unwrap().name(), "Ticker");
    assert_eq!(registry.field_by_name("Symbol").unwrap().name(), "Symbol");
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
    assert_eq!(registry.field_by_name("symbol").unwrap().name(), "SYMBOL");
    assert_eq!(registry.field_by_tag(66).unwrap().name(), "SYMBOL");
    assert_eq!(registry.field_by_name("Sym").unwrap().name(), "SYMBOL");
    assert!(registry.get_field_by_tag(65).is_none());
    assert!(registry.get_field_by_name("Ticker").is_none());
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
        matches!(&error, Error::Conflict { path, .. } if path == "alias \"px\" of Symbol, held by Price"),
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
            registry.field_by_name(name).unwrap().name(),
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

    // An unknown tag is an absence, not a silent insert.
    let error = registry.update(tagged("Text", 58)).unwrap_err();
    assert!(
        matches!(&error, Error::Absent { path, .. } if path == "tag 58"),
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
            registry.get_field_by_name(name),
            "{name}"
        );
        assert_eq!(
            registry.get_field(&name.to_owned()),
            registry.get_field_by_name(name)
        );
        assert_eq!(
            registry.field(name).map(Field::name).ok(),
            registry.field_by_name(name).map(Field::name).ok()
        );
        assert_eq!(
            registry.contains(name),
            registry.get_field_by_name(name).is_some()
        );
    }

    // The failing halves name the key the way it was asked.
    let by_tag = registry.field_by_tag(1).unwrap_err();
    assert!(matches!(&by_tag, Error::Absent { expected: "fix field", path } if path == "tag 1"));
    let by_name = registry.field_by_name("absent").unwrap_err();
    assert!(matches!(&by_name, Error::Absent { path, .. } if path == "name \"absent\""));
    let by_path = registry.field_by_path("Symbol.absent").unwrap_err();
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
            .field_by_path("NoPartyIDs")
            .unwrap()
            .as_fix()
            .tag()
            .unwrap(),
        Some(453)
    );
    assert_eq!(
        registry.field_by_path("NoPartyIDs.PartyID").unwrap(),
        &party_id
    );
    assert_eq!(
        registry
            .field_by_path("nopartyids.item.PartyRole")
            .unwrap()
            .name(),
        "PartyRole"
    );
    assert_eq!(
        registry.field_by_path("Instrument.Symbol").unwrap().name(),
        "Symbol"
    );
    assert_eq!(
        registry.field_by_path("instr.SecurityID").unwrap().name(),
        "SecurityID"
    );
    assert_eq!(
        registry.get_field("Instrument.Symbol"),
        registry.get_field_by_path("Instrument.Symbol")
    );
    assert!(registry.contains("NoPartyIDs.PartyID"));
    // A member is reached through its parent only: the registry does not
    // index it, and the remainder of a path is an exact child name.
    assert!(registry.get_field_by_name("PartyID").is_none());
    assert!(registry.get_field_by_path("NoPartyIDs.partyid").is_none());
    assert!(registry.get_field_by_path("Instrument.Absent").is_none());
    assert!(registry.get_field_by_path("Absent.Symbol").is_none());
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
    // only the last tag it saw sees every field exactly once.
    let mut walked = Vec::new();
    let mut cursor = None;
    while let Some(field) = registry.next_field_after(cursor) {
        walked.push(field.name());
        cursor = field.as_fix().tag().unwrap();
    }
    assert_eq!(walked, ["Account", "Symbol", "Text"]);
    assert!(registry.next_field_after(Some(i32::MAX)).is_none());
    assert!(FixRegistry::new().next_field_after(None).is_none());
    // An alternate tag is an index entry, never a cursor stop.
    let mut aliased = tagged("MsgType", 35);
    aliased.as_fix_mut().set_tags(&[2]).unwrap();
    let with_alternate = FixRegistry::from_fields([aliased, tagged("Account", 1)]).unwrap();
    assert_eq!(
        with_alternate
            .next_field_after(Some(1))
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

    // Debug renders the fields under their tags, in order.
    let rendered = format!("{registry:?}");
    assert!(rendered.starts_with("{1: "), "{rendered}");
    assert!(rendered.find("55: ").unwrap() < rendered.find("58: ").unwrap());
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
    std::fs::write(location.join("records").join("0.json"), b"not json").unwrap();
    let error = autoload(Some(&location.to_string_lossy()), None).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("0.json"), "{message}");
    std::fs::write(home.join(".config/fix/records/0.json"), b"[1]").unwrap();
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
