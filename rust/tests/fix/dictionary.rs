//! The committed dictionary the generator writes, read back.
//!
//! `config/fix` is a contract rather than a code path: it is the seed every
//! test in these phases loads, and the one path this suite names.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use yggdryl::holder::local::Folder;
use yggdryl::{
    DataType, Field, FixCategory, FixRegistry, STANDARD_HEADER_TAGS, STANDARD_TRAILER_TAGS,
    TimeUnit, Timezone, Version,
};

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
    assert_eq!(newest.version(), version("5.0.2"));
    assert!(newest.ep().is_some());
    assert_ne!(newest.version(), Version::MAX);
    let versions = registry.versions();
    assert!(versions.contains(&version("2.7")), "{versions:?}");
    assert!(versions.contains(&version("4.4")), "{versions:?}");
}

#[test]
fn the_committed_dictionary_is_no_dialects_member_and_a_field_is_its_tag_and_its_name() {
    let registry = seed();
    // The shipped dictionary never declared a branch, so nothing it holds
    // carries a membership and it lists no dialect.
    assert!(registry.dialects().is_empty(), "{:?}", registry.dialects());
    for category in FixCategory::ALL {
        for field in registry.definitions(category) {
            assert_eq!(
                field.as_fix().branches().count(),
                0,
                "{category}/{} carries a membership",
                field.name()
            );
        }
    }
    // A field's identity is derived from its tag and its name, never stored:
    // the id every read answers is the one the pair hashes to, under the fold
    // every name lookup already applies.
    let msgtype = registry.field_by_tag(35).expect("tag 35");
    let id = msgtype.as_fix().id().unwrap().expect("an identity");
    assert_eq!(id, yggdryl::FixId::of(35, "MsgType").unwrap());
    assert_eq!(id, yggdryl::FixId::of(35, "Msg_Type").unwrap());
    assert_eq!(id, yggdryl::FixId::of(35, "msgtype").unwrap());
    assert_ne!(id, yggdryl::FixId::of(36, "MsgType").unwrap());
    assert_ne!(id, yggdryl::FixId::of(35, "MsgSeqNum").unwrap());
    assert_eq!(registry.field_by_id(id).unwrap().name(), "msgtype");
    assert!(
        msgtype.as_metadata().get("fix:id").is_none(),
        "an id is derived, never stored"
    );
}

#[test]
fn every_generated_name_is_folded_and_no_two_collide() {
    let registry = seed();
    let scalar_names: BTreeSet<_> = registry.iter().map(Field::name).collect();
    // Across every category, not within one: a derived tag is the definition's
    // identity in the whole catalog.
    let mut derived_tags = BTreeSet::new();
    for category in FixCategory::ALL {
        let mut seen = BTreeSet::new();
        for field in registry.definitions(category) {
            let name = field.name();
            assert!(
                !name.bytes().any(|byte| byte.is_ascii_uppercase()),
                "{name} holds an uppercase byte"
            );
            assert!(!name.contains('_'), "{name} holds an underscore");
            assert!(seen.insert(name), "duplicate {category}/{name}");
            if category != FixCategory::Fields {
                assert!(
                    !scalar_names.contains(name),
                    "{category}/{name} collides with a wire field"
                );
                // Every named definition carries a tag of its own, derived
                // into the block nothing published claims, and no two share
                // one - the `seen` set below proves the names, this the tags.
                let derived = field
                    .as_fix()
                    .tag()
                    .expect("valid tag")
                    .expect("a derived definition tag");
                assert!(
                    yggdryl::FixId::is_definition_tag(derived),
                    "{category}/{name} tag {derived}"
                );
                assert!(
                    derived_tags.insert(derived),
                    "{category}/{name} repeats derived tag {derived}"
                );
            } else {
                assert!(!field.dtype().is_nested(), "wire field {name} is nested");
            }
        }
        assert!(!seen.is_empty(), "the {category} category is missing");
    }
}

#[test]
fn the_standard_declares_its_code_sets_and_the_generator_honours_them() {
    let registry = seed();

    // Each field carries its enum metadata over the scalar datatype. Message codes
    // remain unrestricted text; Side keeps its generic ASCII datatype.
    let msgtype = registry.field_by_tag(35).expect("tag 35");
    assert_eq!(msgtype.dtype(), &DataType::utf8());
    assert_eq!(msgtype.as_fix().code_name("D"), Some("NewOrderSingle"));
    assert_eq!(msgtype.as_fix().code_value("NewOrderSingle"), Some("D"));

    let side = registry.field_by_tag(54).expect("tag 54");
    assert_eq!(side.dtype(), &DataType::Side);
    assert_eq!(side.as_fix().code_name("1"), Some("Buy"));
    assert_eq!(side.as_fix().code_value("buy"), Some("1"));
    // The order's state is declared twice, as `OrdStatus` and as `ExecType`,
    // and both take the crate's `state`: their code sets agree on every value
    // they share, and the type reads either. The code set stays on the field
    // that declares it.
    for tag in [39, 150] {
        let state = registry.field_by_tag(tag).expect("a state tag");
        assert_eq!(state.dtype(), &DataType::State, "tag {tag}");
        assert!(state.as_fix().codes().count() > 5, "tag {tag}");
    }
    assert_eq!(
        registry.field_by_tag(39).unwrap().as_fix().code_name("1"),
        Some("PartiallyFilled")
    );
    // Every other code set keeps its base type, and one code set declared by
    // two fields is stored whole on each.
    let ord_type = registry.field_by_tag(40).expect("tag 40");
    assert_eq!(ord_type.dtype(), &DataType::utf8());
    assert!(ord_type.as_fix().codes().count() > 5);
    let source = registry.field_by_tag(22).expect("SecurityIDSource");
    let alternative = registry.field_by_tag(456).expect("SecurityAltIDSource");
    assert_eq!(
        source.as_metadata().get("fix:codes"),
        alternative.as_metadata().get("fix:codes")
    );
    assert!(source.as_metadata().get("fix:codes").is_some());
    assert!(source.as_metadata().get("fix:codeset").is_none());

    // The float family is what the specification says it is.
    for tag in [31, 38, 44, 6] {
        let field = registry.field_by_tag(tag).expect("a float-family tag");
        assert_eq!(field.dtype(), &DataType::Float64, "tag {tag}");
    }
    // And the ones it types otherwise keep those types.
    assert_eq!(registry.field_by_tag(34).unwrap().dtype(), &DataType::Int64);
    assert_eq!(
        registry.field_by_tag(10).unwrap().dtype(),
        &DataType::utf8()
    );
    assert_eq!(registry.field_by_tag(9).unwrap().dtype(), &DataType::Int32);
}

#[test]
fn a_repeating_group_has_a_scalar_counter_and_a_separately_named_component() {
    let registry = seed();
    let counter = registry.field_by_tag(453).expect("NoPartyIDs");
    assert_eq!(counter.name(), "nopartyids");
    assert_eq!(counter.dtype(), &DataType::Int32);
    let parties = registry
        .definition(FixCategory::Groups, "Parties")
        .expect("Parties group");
    assert_eq!(parties.name(), "parties");
    assert_eq!(parties.display(), Some("Parties"));
    assert_eq!(parties.as_fix().counter().unwrap(), Some(453));
    // The counter it heads is the published 453; its own identity is derived,
    // and the two are never the same number.
    let derived = parties.as_fix().tag().unwrap().expect("a derived tag");
    assert!(yggdryl::FixId::is_definition_tag(derived), "{derived}");
    assert_ne!(derived, 453);
    let DataType::List(item) = parties.dtype() else {
        panic!("a list, got {}", parties.dtype());
    };
    assert_eq!(item.name(), "party");
    assert_eq!(item.as_fix().component(), Some("party"));
    assert!(!item.is_nullable());
    let component = registry
        .definition(FixCategory::Components, "Party")
        .expect("Party component");
    assert_eq!(component.dtype(), item.dtype());
    let members: Vec<&str> = item
        .dtype()
        .as_fields()
        .expect("a struct item")
        .iter()
        .map(yggdryl::Field::name)
        .collect();
    assert!(members.contains(&"partyid"), "{members:?}");
    assert!(members.contains(&"partyrole"), "{members:?}");
    assert!(!members.contains(&"nopartyids"), "{members:?}");
    assert_eq!(
        registry
            .definition(FixCategory::Groups, "Parties")
            .unwrap()
            .get_field_by_path("party.partyid")
            .map(Field::name),
        Some("partyid")
    );
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
    }
    for field in registry.iter() {
        assert!(field.as_metadata().get("fix:codeset").is_none());
        for entry in field.as_fix().lineage() {
            entry.unwrap_or_else(|error| panic!("{}: {error}", field.name()));
            entries += 1;
        }
    }
    // A dictionary this size is the point: a truncation that hides one code
    // in twenty thousand is exactly what nobody notices by reading.
    assert!(with_codes > 900, "{with_codes} fields with inline enums");
    assert!(
        codes > 10_000,
        "{codes} enum records stored with their fields"
    );
    // Fewer than there are fields, and deliberately: a field whose only
    // history is "as it is now" states none (P3).
    assert!(entries > 1_500, "{entries} lineage entries in all");

    // The spelling the truncation hid, end to end.
    let role = registry.field_by_tag(452).expect("PartyRole");
    assert_eq!(role.as_fix().code_value("ClearingFirm"), Some("4"));
    assert_eq!(role.as_fix().code_name("4"), Some("ClearingFirm"));
}

/// Every field the dictionary holds, occurrences and group members included.
fn every_field(registry: &FixRegistry) -> Vec<Field> {
    fn walk(field: &Field, out: &mut Vec<Field>) {
        out.push(field.clone());
        match field.dtype() {
            DataType::List(item) | DataType::LargeList(item) => walk(item, out),
            DataType::Struct(fields) => {
                for held in fields.iter() {
                    walk(held, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for category in FixCategory::ALL {
        for field in registry.definitions(category) {
            walk(field, &mut out);
        }
    }
    out
}

#[test]
fn every_date_is_an_instant_and_every_zone_is_the_one_its_name_states() {
    let registry = seed();
    // Nothing anywhere is a day or a second, occurrences and group members
    // included: a FIX date is that day's midnight, so a capture joining a
    // settlement date to a transact time compares them without a cast, and
    // `LocalMktTime` and `UTCTimeOnly` are one type for the same reason.
    for field in every_field(&registry) {
        match field.dtype() {
            DataType::Date32 | DataType::Date64 => {
                panic!("{} is still a day rather than an instant", field.name())
            }
            DataType::Time32(_) => panic!("{} is still typed to a second", field.name()),
            DataType::Time64(unit) => assert_eq!(*unit, TimeUnit::Nanosecond, "{}", field.name()),
            DataType::DateTime64 { unit, timezone } => {
                assert_eq!(*unit, TimeUnit::Nanosecond, "{}", field.name());
                // Two zones, and only two: what the datatype's own name says.
                // A `UTCTimestamp` is UTC and a `LocalMktDate` states no zone,
                // so neither reads as the other.
                assert!(
                    *timezone == Timezone::UTC || timezone.is_naive(),
                    "{} states {timezone}",
                    field.name()
                );
            }
            _ => {}
        }
    }

    // The registry's own entries, where each field is counted once.
    let mut times = 0_usize;
    let mut naive = 0_usize;
    let mut utc = 0_usize;
    for field in registry.iter() {
        match field.dtype() {
            DataType::Time64(_) => times += 1,
            DataType::DateTime64 { timezone, .. } if timezone.is_naive() => naive += 1,
            DataType::DateTime64 { .. } => utc += 1,
            _ => {}
        }
    }
    assert_eq!(times, 57, "zone-less times of day");
    assert_eq!(naive, 369, "local values, stating no zone");
    // Sixty-eight of the seed's, and the crate's own `timestamp`.
    assert_eq!(utc, 69, "instants stated in UTC");
}

#[test]
fn the_committed_lineage_keeps_only_the_retypes_that_are_real() {
    let registry = seed();
    let mut census: BTreeMap<(String, String), usize> = BTreeMap::new();
    for field in registry.iter() {
        let view = field.as_fix();
        let held: Vec<_> = view
            .lineage()
            .map(|entry| entry.expect("a readable entry"))
            .collect();
        for pair in held.windows(2) {
            let (Some(older), Some(newer)) = (
                pair[0].parse_dtype().expect("a resolvable type"),
                pair[1].parse_dtype().expect("a resolvable type"),
            ) else {
                continue;
            };
            if older == newer {
                continue;
            }
            // A text field that later declared itself temporal was always
            // carrying the instant, so it no longer reads as a retype. A
            // string-to-string retype cannot survive at all, because two
            // spellings of one type are now one value and this arm is only
            // reached where the types differ.
            assert!(
                !(older.id().is_string() && newer.id().is_temporal()),
                "{} still states a string before a temporal",
                field.name()
            );
            // One temporal is restated as another only where the *zone*
            // changed, which is a real change and not a spelling: a field the
            // specification moved from `UTCDateOnly` to `LocalMktDate` stopped
            // stating a zone, and the reading has to say so. Precision never
            // drifts.
            if older.id().is_temporal() && newer.id().is_temporal() {
                let (
                    DataType::DateTime64 {
                        unit: older_unit, ..
                    },
                    DataType::DateTime64 {
                        unit: newer_unit, ..
                    },
                ) = (&older, &newer)
                else {
                    panic!("{} retypes one temporal family to another", field.name())
                };
                assert_eq!(older_unit, newer_unit, "{}", field.name());
            }
            *census
                .entry((older.to_string(), newer.to_string()))
                .or_default() += 1;
        }
    }
    let total: usize = census.values().sum();
    // 66 before decision 14: tag 385 was retyped to `msgdirection` at 5.0.2
    // and is text carrying its code set again, so the retype is gone.
    assert_eq!(total, 65, "surviving retypes: {census:?}");
    assert_eq!(
        census
            .iter()
            .map(|((older, newer), count)| (older.as_str(), newer.as_str(), *count))
            .collect::<Vec<_>>(),
        [
            // A UTC date restated as a local market date: the zone went away,
            // and the reading says so rather than calling both a day.
            ("datetime64(ns,\"UTC\")", "datetime64(ns)", 12),
            ("float64", "utf8", 1),
            ("int32", "float64", 13),
            ("int32", "int64", 6),
            ("int32", "utf8", 8),
            ("utf8", "boolean", 9),
            ("utf8", "country", 1),
            ("utf8", "currency", 3),
            ("utf8", "int32", 6),
            ("utf8", "mic", 3),
            ("utf8", "side", 1),
            // `OrdStatus` and `ExecType`: the order's state, read as one type.
            ("utf8", "state", 2),
        ]
    );
}

/// The committed dictionary's hash, pinned as a literal (decision 13).
///
/// The registry hash walks `[Components, Groups]` with the messages among
/// the components in name order, so a change to that walk - or to any
/// shipped document - moves this number on purpose, in the commit that
/// says why.
#[test]
fn the_committed_dictionary_hashes_to_one_pinned_value() {
    let registry = seed();
    assert_eq!(registry.stable_hash(), 15_614_738_859_645_635_265);
    assert_eq!(registry.msgtypes().count(), 181);
    assert_eq!(registry.definitions(FixCategory::Components).count(), 928);
    assert_eq!(registry.definitions(FixCategory::Groups).count(), 580);
}
