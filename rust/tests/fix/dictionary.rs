//! The committed dictionary the generator writes, read back.
//!
//! `config/fix` is a contract rather than a code path: it is the seed every
//! test in these phases loads, and the one path this suite names.

use std::collections::BTreeSet;

use yggdryl::SequenceType;
use yggdryl::local::Folder;
use yggdryl::{
    DataType, DateTimeType, Field, FixCategory, FixRegistry, STANDARD_HEADER_TAGS,
    STANDARD_TRAILER_TAGS, Scalar, TimeType, TimeUnit, Timezone,
};

fn seed() -> FixRegistry {
    super::committed_registry().as_ref().clone()
}

/// The registry's fields of one category: a definition is filed by the
/// shape it has - a Struct is a component, a List or a Map a group - and
/// everything else is a wire field.
fn definitions(registry: &FixRegistry, category: FixCategory) -> impl Iterator<Item = &Field> {
    registry
        .iter()
        .filter(move |field| category_of(field) == category)
}

fn category_of(field: &Field) -> FixCategory {
    match field.dtype() {
        DataType::Struct(_) => FixCategory::Components,
        dtype if dtype.is_nested() => FixCategory::Groups,
        _ => FixCategory::Fields,
    }
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
    assert_eq!(last_qty.dtype(), &DataType::DECIMAL);
    assert_eq!(last_qty.as_metadata().get("display"), Some("LastQty"));

    // The dictionary holds one reading of the tag, under one name and one
    // datatype; the spellings earlier versions used reach it as aliases.
    let view = last_qty.as_fix();
    assert_eq!(view.names().collect::<Vec<_>>(), ["lastshares"]);

    // A query by either spelling answers the same field, and the
    // specification's own casing still resolves.
    for spelling in ["lastqty", "LastQty", "LastShares", "lastshares"] {
        assert_eq!(
            registry.field(spelling).expect(spelling).name(),
            "lastqty",
            "{spelling}"
        );
    }
}

#[test]
fn the_committed_dictionary_is_no_dialects_member_and_a_field_is_its_tag_and_its_name() {
    let registry = seed();
    // The shipped dictionary never declared a branch, so nothing it holds
    // carries a membership and it lists no dialect.
    assert!(registry.dialects().is_empty(), "{:?}", registry.dialects());
    for category in FixCategory::ALL {
        for field in definitions(&registry, category) {
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
        msgtype.as_metadata().get("FIX:id").is_none(),
        "an id is derived, never stored"
    );
}

#[test]
fn every_generated_name_is_folded_and_no_two_collide() {
    let registry = seed();
    let scalar_names: BTreeSet<_> = definitions(&registry, FixCategory::Fields)
        .map(Field::name)
        .collect();
    // Across every category, not within one: a derived tag is the definition's
    // identity in the whole catalog.
    let mut derived_tags = BTreeSet::new();
    for category in FixCategory::ALL {
        let mut seen = BTreeSet::new();
        for field in definitions(&registry, category) {
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
                    yggdryl::FixId::is_definition_tag(derived) || yggdryl::is_crate_tag(derived),
                    "{category}/{name} tag {derived}"
                );
                // A crate tag on a named definition means the definition is
                // one of this crate's own columns, reached by the tag the
                // fixed row files it under: `identifiers` and `metadata` are
                // the Maps whose counter is that tag.
                if yggdryl::is_crate_tag(derived) {
                    assert!(
                        yggdryl::fix_crate_fields()
                            .expect("the crate's own fields")
                            .iter()
                            .any(|own| own.name() == name
                                && own.as_fix().tag().unwrap() == Some(derived)),
                        "{category}/{name} holds crate tag {derived} without being one"
                    );
                }
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

    // Each field names the set its values are drawn from, over the scalar
    // datatype the generator gave it. Message codes remain unrestricted
    // text; Side keeps its generic ASCII datatype.
    let msgtype = registry.field_by_tag(35).expect("tag 35");
    assert_eq!(msgtype.dtype(), &DataType::utf8());
    let codes = registry.codeset_of(msgtype).expect("the MsgType code set");
    assert_eq!(codes.name(), "msgtypecodeset");
    assert_eq!(codes.code_name("D"), Some("NewOrderSingle"));
    assert_eq!(codes.code_value("NewOrderSingle"), Some("D"));

    let side = registry.field_by_tag(54).expect("tag 54");
    assert_eq!(side.dtype(), &DataType::Side);
    let codes = registry.codeset_of(side).expect("the Side code set");
    assert_eq!(codes.name(), "sidecodeset");
    assert_eq!(codes.code_name("1"), Some("Buy"));
    assert_eq!(codes.code_value("buy"), Some("1"));
    // The order's state is declared twice, as `OrdStatus` and as `ExecType`,
    // each naming a code set of its own; only the crate's `state` column
    // is typed as a state, and it reads either.
    for tag in [39, 150] {
        let state = registry.field_by_tag(tag).expect("a state tag");
        assert_eq!(state.dtype(), &DataType::utf8(), "tag {tag}");
        let codes = registry
            .codeset_of(state)
            .unwrap_or_else(|| panic!("tag {tag}"));
        assert!(codes.codes().count() > 5, "tag {tag}");
    }
    assert_eq!(
        registry
            .codeset_of(registry.field_by_tag(39).unwrap())
            .unwrap()
            .code_name("1"),
        Some("PartiallyFilled")
    );
    // Every other code set keeps its base type, and one code set two fields
    // declare is held once under the one name both of them name.
    let ord_type = registry.field_by_tag(40).expect("tag 40");
    assert_eq!(ord_type.dtype(), &DataType::utf8());
    assert!(registry.codeset_of(ord_type).unwrap().codes().count() > 5);
    let source = registry.field_by_tag(22).expect("SecurityIDSource");
    let alternative = registry.field_by_tag(456).expect("SecurityAltIDSource");
    assert_eq!(
        source.as_metadata().get("FIX:codeset"),
        alternative.as_metadata().get("FIX:codeset")
    );
    assert_eq!(
        source.as_metadata().get("FIX:codeset"),
        Some("securityidsourcecodeset")
    );
    assert_eq!(
        registry.codeset_of(source),
        registry.codeset_of(alternative)
    );
    assert!(source.as_metadata().get("FIX:codes").is_none());

    // A price, a quantity, a price offset and an amount are exact numbers,
    // at the one decimal width this crate keeps them at; a percentage and
    // FIX's own `float` stay the floating count the specification names.
    for tag in [31, 38, 44, 6] {
        let field = registry.field_by_tag(tag).expect("an exact-number tag");
        assert_eq!(field.dtype(), &DataType::DECIMAL, "tag {tag}");
    }
    for tag in [155, 231, 211] {
        let field = registry.field_by_tag(tag).expect("a float tag");
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
    let parties = registry.field_by_name("Parties").expect("Parties group");
    assert_eq!(parties.name(), "parties");
    assert_eq!(parties.display(), Some("Parties"));
    assert_eq!(parties.as_fix().counter().unwrap(), Some(453));
    // The counter it heads is the published 453; its own identity is derived,
    // and the two are never the same number.
    let derived = parties.as_fix().tag().unwrap().expect("a derived tag");
    assert!(yggdryl::FixId::is_definition_tag(derived), "{derived}");
    assert_ne!(derived, 453);
    let DataType::Sequence(SequenceType::List(item)) = parties.dtype() else {
        panic!("a list, got {}", parties.dtype());
    };
    assert_eq!(item.name(), "party");
    assert_eq!(item.as_fix().component(), Some("party"));
    assert!(!item.is_nullable());
    let component = registry.field_by_name("Party").expect("Party component");
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
        parties.get_field_by_path("party.partyid").map(Field::name),
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
    for field in registry.iter() {
        // A refusal ends a borrowed walk, and the walk is what resolution
        // reads - so one malformed record does not fail loudly, it silently
        // removes every record after it. Orchestra's `addedEP="-1"` did
        // exactly that to 169 code sets: `ClearingFirm` stopped resolving
        // because a code 60 records earlier would not parse. Nothing here can
        // catch that but walking every document to its end.
        let mut seen = 0_usize;
        if let Some(set) = registry.codeset_of(field) {
            for code in set.codes() {
                code.unwrap_or_else(|error| panic!("{}: {error}", field.name()));
                seen += 1;
            }
        }
        if seen > 0 {
            with_codes += 1;
        }
        codes += seen;
    }
    // And every set the store holds, whether or not a field reads by it:
    // the documents live in `codesets/` now, one file per name.
    for set in registry.codesets() {
        for code in set.codes() {
            code.unwrap_or_else(|error| panic!("{}: {error}", set.name()));
        }
    }
    for field in registry.iter() {
        assert!(field.as_metadata().get("FIX:codes").is_none());
        assert!(field.as_metadata().get("FIX:lineage").is_none());
    }
    // A dictionary this size is the point: a truncation that hides one code
    // in twenty thousand is exactly what nobody notices by reading.
    assert!(with_codes > 900, "{with_codes} fields name a code set");
    assert!(
        codes > 10_000,
        "{codes} enum records reached through the fields that name them"
    );
    // The spelling the truncation hid, end to end.
    let role = registry.field_by_tag(452).expect("PartyRole");
    let set = registry.codeset_of(role).expect("the PartyRole code set");
    assert_eq!(set.code_value("ClearingFirm"), Some("4"));
    assert_eq!(set.code_name("4"), Some("ClearingFirm"));
}

/// Every field the dictionary holds, occurrences and group members included.
fn every_field(registry: &FixRegistry) -> Vec<Field> {
    fn walk(field: &Field, out: &mut Vec<Field>) {
        out.push(field.clone());
        match field.dtype() {
            DataType::Sequence(SequenceType::List(item))
            | DataType::Sequence(SequenceType::LargeList(item)) => walk(item, out),
            DataType::Struct(fields) => {
                for held in fields.iter() {
                    walk(held, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for field in registry.iter() {
        walk(field, &mut out);
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
            DataType::Date(_) => {
                panic!("{} is still a day rather than an instant", field.name())
            }
            DataType::Time(TimeType::Time32(_)) => {
                panic!("{} is still typed to a second", field.name())
            }
            DataType::Time(TimeType::Time64(unit)) => {
                assert_eq!(*unit, TimeUnit::Nanosecond, "{}", field.name())
            }
            DataType::DateTime(DateTimeType::DateTime64 { unit, timezone }) => {
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
    let clock = DataType::DateTime(DateTimeType::DateTime64 {
        unit: TimeUnit::Nanosecond,
        timezone: Timezone::UTC,
    });
    let mut times = 0_usize;
    let mut naive = 0_usize;
    let mut utc = 0_usize;
    for field in registry.iter() {
        match field.dtype() {
            DataType::Time(TimeType::Time64(_)) => times += 1,
            DataType::DateTime(DateTimeType::DateTime64 { timezone, .. })
                if timezone.is_naive() =>
            {
                naive += 1
            }
            DataType::DateTime(_) => utc += 1,
            _ => {}
        }
    }
    assert_eq!(times, 57, "zone-less times of day");
    assert_eq!(naive, 369, "local values, stating no zone");
    // Sixty-eight shipped fields, plus the crate's five clocks: `currunix`,
    // `creaunix`, `prevunix`, `snapunix` and `exprtime`.
    let crated = registry
        .iter()
        .filter(|field| {
            field.dtype() == &clock
                && field
                    .as_fix()
                    .tag()
                    .ok()
                    .flatten()
                    .is_some_and(yggdryl::is_crate_tag)
        })
        .count();
    assert_eq!(crated, 5, "the crate's own clocks");
    assert_eq!(utc, 68 + crated, "instants stated in UTC");
}

/// A field FIX Latest removed is still the dictionary's, marked so: a
/// message stating it is read, and its value restated to the field that
/// replaced it, so a capture of every version lands in one row.
#[test]
fn a_removed_field_is_kept_and_marked_deprecated() {
    let registry = seed();
    // `MaxFloor(111)` went in 5.0. Its canonical metadata records the one
    // specification replacement, so every version restates it as DisplayQty.
    let floor = registry.field_by_tag(111).expect("MaxFloor");
    assert_eq!(floor.as_fix().deprecated(), Some("5.0"), "{floor:?}");
    let mut replacements = floor.as_fix().replacements();
    assert_eq!(
        replacements
            .next()
            .expect("one rule")
            .expect("a valid rule")
            .plan(),
        "select maxfloor as displayqty"
    );
    assert!(replacements.next().is_none(), "one canonical replacement");
    // `Signature(89)` went with it and nothing replaces it: the mark stands
    // on its own.
    let signature = registry.field_by_tag(89).expect("Signature");
    assert_eq!(signature.as_fix().deprecated(), Some("5.0"));
    assert!(signature.as_fix().replacements().next().is_none());
    assert!(
        registry
            .field_by_tag(11)
            .unwrap()
            .as_fix()
            .deprecated()
            .is_none(),
        "ClOrdID stands"
    );
    let deprecated = definitions(&registry, FixCategory::Fields)
        .filter(|field| field.as_fix().deprecated().is_some())
        .count();
    assert_eq!(deprecated, 56);
}

/// A component's member is a reference to the field or group it holds,
/// and it names the tag beside the name, so a reader resolves the member
/// by identity without the shard in hand.
#[test]
fn a_member_reference_carries_the_field_and_its_tag() {
    let registry = seed();
    let party = registry.field_by_name("Party").expect("Party component");
    let partyid = party
        .get_field_by_path("partyid")
        .expect("the PartyID member");
    assert_eq!(partyid.as_fix().field_ref(), Some("partyid"));
    assert_eq!(partyid.as_fix().tag().unwrap(), Some(448));
    let subgroup = party
        .get_field_by_path("ptyssubgrp")
        .expect("the sub-party group member");
    assert_eq!(subgroup.as_fix().group(), Some("ptyssubgrp"));
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let document = std::fs::read_to_string(root.join("components/party.json")).unwrap();
    let stored = yggdryl::from_json_scalar(&document).unwrap();
    let member = stored
        .get_key_str("dtype")
        .and_then(|dtype| dtype.get_key_str("fields"))
        .and_then(Scalar::as_sequence)
        .and_then(|members| members.first())
        .and_then(|member| member.get_key_str("metadata"))
        .expect("the first member's metadata");
    assert_eq!(
        member.get_key_str("FIX:field").and_then(Scalar::as_str),
        Some("partyid")
    );
    assert_eq!(
        member.get_key_str("FIX:tag").and_then(Scalar::as_str),
        Some("448")
    );
    // A field's shard is its tag over a hundred, named nine digits wide:
    // tag 55 sits in the first, tag 50000 in the five-hundredth.
    assert!(root.join("fields/000000000.json").exists());
    assert!(root.join("fields/000000500.json").exists());
    assert!(!root.join("fields/0.json").exists(), "the old shard name");
}

/// The committed dictionary's hash, pinned as a literal.
///
/// The registry hash walks scalar fields, then the components and the groups
/// in name order, so a change to that walk or to any shipped document moves
/// this number on purpose, in the commit that says why. It last moved when
/// every protocol key took its scheme upper case - `FIX:tag`, `FIX:codeset`
/// and `FIX:branches` beside `ARROW:extension:name` and `PARQUET:field_id` -
/// so every stored key the dictionary hashes changed its spelling. It last
/// moved when
/// the capture's clock stopped being a crate field: `recordedat` (65028) is
/// retired, a message's instant being what it states - `SendingTime(52)` and
/// the settled `currunix` - and the plugin that logged a line is
/// `msgpluginid` (`MsgPluginId`, tag 65009 unchanged), spelled like
/// `msgctxid` and `msgsessionid`, and the instant a message was created is
/// `creaunix` (`CreaUnix`, tag 65023 unchanged), spelled like `currunix`,
/// `prevunix` and `snapunix`. It last moved when
/// the message became a typed market event: the crate's columns are the
/// event's facts - `currunix`, `creaunix`, `currhashcode`, `crosshashcode`,
/// `crosscode`, the identities as UUIDs, the lanes, the two Map groups
/// `identifiers` and `metadata` - the shards are named nine digits wide,
/// every member reference carries its `FIX:tag`, and a field FIX Latest
/// removed is marked `FIX:deprecated`. It last moved when the market numbers
/// merged onto the event: `Price(44)`, `OrderQty(38)` and `Quantity(53)` stop
/// being columns of their own and the crate's `px` and `qty` answer for them,
/// and `prevpx`, `prevqty`, `tradable` and `symbolticker` join the block. It
/// last moved when the capture's own columns stopped being facts of a
/// message: `recordedat` states no `FIX:derivation`, because when a capture
/// wrote a line down is whoever read it to say and never the message's own
/// `SendingTime`. It last moved when the event's instant and its own digest
/// took the names their columns carry: `unix` became `currunix` and
/// `hashcode` became `currhashcode`, so the crate's tags 65003 and 65017
/// read as the current instant and the current code next to `crosshashcode`.
/// It last moved when every FIX quantity, price, price offset and amount
/// became an exact number: 478 fields are `decimal128(38, 18)` where they
/// were `float64`, a percentage and FIX's own `float` stay floating, and
/// the seven derivations that multiply or add across the two state each
/// operand's exact scale. It last moved when the crate stopped owning a
/// market column: eighteen of its thirty-eight definitions are gone - the
/// price, the quantity, the unit, the instrument's codes, the market, the
/// state, the lanes' currencies and units, what a price moved from, whether
/// it could trade and the ticker - because every one of them restated a FIX
/// field the traits now read, and the two rules that read `isincode` read
/// `SecurityID(48)` under its source and the `SecurityAltID` group instead.
/// It last moved when the nested datatypes became families: `DataType` derives
/// its hash, so a family variant contributes its leaf's discriminant too, and
/// every sequence, struct and mapping in the dictionary hashes one level
/// deeper than it did. `identifiers` and `metadata` also declare sorted keys, so they are
/// `sorted_map` rather than `map` beside a flag, and their entries the struct
/// of two children every mapping's are. It
/// last moved when a field became an enum over its leaves and the dictionary
/// sidecar moved onto the one leaf that has it: a field hashes the datatype
/// its leaf holds rather than the `DataType` it widens to, and it hashes one
/// sidecar - nothing at all for a field that is not dictionary-encoded -
/// rather than an identifier and a flag every field carried. It last moved
/// when the four decimal widths became one `Decimal` family variant: every
/// datatype declared after them in the enum shifted by three discriminants,
/// and a decimal hashes its leaf one level deeper. It last moved when uuid
/// became a family: a uuid column hashes the leaf that says which RFC 9562
/// versions it admits, where it used to hash a variant with nothing in it. It
/// last moved when the byte family became six leaves: a byte column hashes
/// one leaf that already carries its count, where it used to hash a layout
/// and an optional bound beside it. It last moved when the two sets of moves
/// above met: the families hash the dictionary main settled on, so neither
/// side's pinned number survives the merge and this one is what the merged
/// tree answers. It last moved when the string family became eighteen
/// leaves: a string column hashes one leaf that already carries its charset
/// and its count, where it used to hash a layout, a charset and an optional
/// bound beside them. It last moved when the eight temporal variants became
/// five families: a temporal column hashes its family's leaf one level
/// deeper, where it used to hash a variant of its own. It last moved when
/// uuid became one parameter-free datatype again: a uuid column hashes a
/// variant with nothing in it, where it used to hash the leaf that said which
/// RFC 9562 versions it admitted. It last moved when the crate's own block
/// gained `srcuuids`: a twentieth definition, the list of the elements a
/// message was read from, hashes beside the nineteen and as a member of the
/// fixed row. It last moved when the datatype identifiers were laid out by
/// family and a mapping's entries became the plain struct of two children:
/// every datatype tag in the hash is the family-laid byte, and `identifiers`
/// and `metadata` hash their entries as a struct rather than a leaf of their
/// own. It last moved when the crate's own block gained `state` and
/// `exprtime`: the two lifecycle facts a walk folds forward hash as a
/// twenty-first and a twenty-second definition, each at its event column's
/// datatype - a ranked state and a nanosecond clock - and as members of the
/// fixed row, the state ahead of `OrdStatus` and the expiry beside the clocks.
/// It last moved when what the specification retired became the crate's own
/// table: the thirty-seven fields that carried a `FIX:replacements` document
/// hash without it, one metadata entry fewer each, and the dictionary states
/// no rule of its own.
/// It last moved when `sourceurl` left the fixed row: the object a line was
/// read from is the reader's word about the line and not the message's about
/// itself, so it travels as one of the capture's own columns beside the row,
/// the definition still hashes as a crate field of its own, and the fixed
/// row hashes one member fewer. It last moved when those two settled changes
/// met in this merge: the thirty-seven retired `FIX:replacements` entries are
/// absent and `sourceurl` remains a crate field while leaving the fixed row,
/// so their combined dictionary is the value pinned here. It moved again when
/// a field's vocabulary became a named code set: `FIX:codeset` now names the
/// set, and the dictionary hashes every named set once beside its fields.
/// It also moved when expiry became `exprtime`, the 181 message components
/// gained `FIX:msgcat`, and MsgCat plus five normalized identifier definitions
/// joined the fixed row. CFI keeps standard tag 461; the expiry description
/// now states that a newer explicit deadline replaces the preceding one.
/// It last moved when the merged store centralized 735 published vocabularies
/// and MsgCat's crate vocabulary under `FIX:codeset`, including the shared
/// PartyIDSource `proprietary/customcode` alias. FIGICode adds one crate
/// definition and one fixed-row member; `nofixentries` now describes the
/// residual count rather than the complete in-memory content.
/// It last moved when the identifiers group described the synthesized
/// `msgsectxid` capture pair beside the message's ordinary identifiers.
#[test]
fn the_committed_dictionary_hashes_to_one_pinned_value() {
    let registry = seed();
    assert_eq!(registry.stable_hash(), 10_264_503_129_809_217_507);
    let messages = definitions(&registry, FixCategory::Components)
        .filter(|component| component.as_fix().msgtype().is_some())
        .count();
    assert_eq!(messages, 181);
    assert_eq!(
        definitions(&registry, FixCategory::Components).count(),
        928 + super::crated_components()
    );
    // Every group the store ships resolves, the crate's two Map groups
    // among them: the census is the directory rather than a literal, so a
    // generated group added or dropped moves nothing here.
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config/fix/groups");
    for entry in std::fs::read_dir(root).expect("the generated group directory") {
        let path = entry.expect("a group document").path();
        let name = path
            .file_stem()
            .expect("a group name")
            .to_str()
            .expect("ASCII");
        assert!(
            definitions(&registry, FixCategory::Groups).any(|group| group.name() == name),
            "{name} resolves as a group"
        );
    }
    assert_eq!(
        registry.len(),
        definitions(&registry, FixCategory::Fields).count()
            + definitions(&registry, FixCategory::Components).count()
            + definitions(&registry, FixCategory::Groups).count()
    );
}
