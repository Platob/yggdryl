//! `rust/fix/src/registry.rs`: the five indexes behind one namespace.
//!
//! A caller asks a registry for a tag, an identity or a name; what it is made
//! of - the seeded digests, the position indexes, the compiled derivations and
//! the lifted-name cache - is reached through `yggdryl_fix::internals`, because a
//! digest collision and a warm cache cannot be staged from outside. What the
//! lookups answer is pinned through `yggdryl_fix::` in the suites beside this one.

use super::crated_components;
use super::definitions;
use super::msgtypes;
use super::path;
use super::scalars;
use super::seeded_fields;

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::{DataType, Error, Field};
    use yggdryl_fix::internals::registry::{
        ALIAS_SEED, NAME_SEED, canonical_id, fields, force_alias_index, force_id_index,
        force_name_index, name_digest,
    };
    use yggdryl_fix::{FixFieldMut, FixRegistry};

    fn tagged(name: &str, tag: i32) -> Field {
        let mut field = DataType::utf8().nullable_field(name);
        FixFieldMut::new(&mut field).set_tag(tag).unwrap();
        field
    }

    #[test]
    fn a_forced_name_digest_collision_is_a_miss_then_a_conflict() {
        crate::install::installed();
        let held = tagged("Held", 1);
        let incoming = tagged("Incoming", 2);
        let mut registry = FixRegistry::from_fields([held.clone()]).unwrap();
        // The crate's own fields sit in front of it, so its position is
        // found rather than assumed to be the first.
        let at = fields(&registry)
            .iter()
            .position(|field| field.name() == held.name())
            .expect("the held field");
        let collided = name_digest(incoming.name(), NAME_SEED);
        force_name_index(&mut registry, collided, at);

        assert!(
            registry.get_field_by_name(incoming.name()).is_none(),
            "a digest hit is rechecked against the canonical name"
        );
        let before = fields(&registry).to_vec();
        let error = registry.insert(incoming).unwrap_err();
        assert!(
            matches!(
                &error,
                Error::Conflict { path, .. }
                    if path.contains("Incoming") && path.contains("Held")
            ),
            "{error}"
        );
        assert_eq!(fields(&registry), before);
    }

    #[test]
    fn a_forced_identity_collision_is_a_conflict_and_a_hit_is_rechecked() {
        crate::install::installed();
        let held = tagged("Held", 1);
        let incoming = tagged("Incoming", 2);
        let mut registry = FixRegistry::from_fields([held.clone()]).unwrap();
        let at = fields(&registry)
            .iter()
            .position(|field| field.name() == held.name())
            .expect("the held field");
        // The incoming identity is forced onto the held field's position, as
        // a 32-bit digest collision would land it.
        let collided = canonical_id(&incoming).unwrap();
        force_id_index(&mut registry, collided, at);

        let hit = registry
            .get_field_by_id(collided)
            .expect("the position answers");
        assert_eq!(
            hit.name(),
            "Held",
            "an identity hit answers the field indexed under it"
        );
        let before = fields(&registry).to_vec();
        let error = registry.insert(incoming).unwrap_err();
        assert!(
            matches!(
                &error,
                Error::Conflict { path, .. }
                    if path.contains("Incoming") && path.contains("Held")
            ),
            "{error}"
        );
        assert_eq!(fields(&registry), before);
    }

    #[test]
    fn a_colliding_alias_digest_is_rechecked_and_the_words_answer() {
        crate::install::installed();
        let offer = tagged("offerpx", 1);
        let unrelated = tagged("Unrelated", 2);
        let mut registry = FixRegistry::from_fields([offer, unrelated]).unwrap();
        let unrelated_at = fields(&registry)
            .iter()
            .position(|field| field.name() == "Unrelated")
            .expect("the unrelated holder");
        force_alias_index(
            &mut registry,
            name_digest("askpx", ALIAS_SEED),
            unrelated_at,
        );

        // The forced hit is rechecked and refused, so the spelling reaches
        // the field its words name rather than the one the digest landed on.
        assert_eq!(
            registry.get_field_by_name("askpx").map(Field::name),
            Some("offerpx")
        );
    }
}

mod lenient {
    use super::path as fpath;

    use yggdryl::{DataType, Error, Field, StructType};
    use yggdryl_fix::{FixCategory, FixField, FixFieldMut, FixId, FixRegistry};

    fn tagged(name: &str, tag: i32, dtype: DataType) -> Field {
        let mut field = dtype.nullable_field(name);
        FixFieldMut::new(&mut field).set_tag(tag).unwrap();
        field
    }

    /// A catalog whose `Party` component holds `PartyID` and then `members`, a
    /// `Parties` group restating the component, and a `NewOrderSingle` message
    /// restating the group: every kind of reference between two definitions.
    fn catalog_with(members: impl IntoIterator<Item = Field>) -> FixRegistry {
        let mut registry = FixRegistry::from_fields([
            tagged("NoPartyIDs", 453, DataType::Int32),
            tagged("PartyID", 448, DataType::utf8()),
        ])
        .unwrap();
        let mut partyid = registry.field(448).unwrap().clone();
        FixFieldMut::new(&mut partyid)
            .set_field_ref("PartyID")
            .unwrap();
        let component = StructType::from_fields(std::iter::once(partyid).chain(members))
            .map(DataType::from)
            .unwrap()
            .required_field("Party");
        registry.insert(component).unwrap();
        let component = registry.field_by_name("Party").unwrap().clone();
        let mut group = DataType::serie(component).nullable_field("Parties");
        FixFieldMut::new(&mut group).set_counter(453).unwrap();
        FixFieldMut::new(&mut group).set_component("Party").unwrap();
        registry.insert(group).unwrap();
        let mut group = registry.field_by_name("Parties").unwrap().clone();
        FixFieldMut::new(&mut group).set_group("Parties").unwrap();
        let mut message = StructType::from_fields([group])
            .map(DataType::from)
            .unwrap()
            .required_field("NewOrderSingle");
        FixFieldMut::new(&mut message).set_msgtype("D").unwrap();
        registry.insert(message).unwrap();
        registry
    }

    fn catalog() -> FixRegistry {
        catalog_with([])
    }

    /// The names of a Struct field's direct children, in order.
    fn names(field: &Field) -> Vec<&str> {
        field.fields().iter().map(Field::name).collect()
    }

    /// The occurrence a group's serie holds.
    fn occurrence(group: &Field) -> &Field {
        let (DataType::Serie(item) | DataType::LargeSerie(item)) = group.dtype() else {
            panic!("a group serie")
        };
        item
    }

    #[test]
    fn a_field_whose_name_folds_to_a_stored_name_merges_into_that_field() {
        crate::install::installed();
        let mut symbol = tagged("Symbol", 55, DataType::utf8());
        FixFieldMut::new(&mut symbol).set_tags(&[65]).unwrap();
        FixFieldMut::new(&mut symbol).set_names(["Ticker"]).unwrap();
        FixFieldMut::new(&mut symbol)
            .set_description("stored")
            .unwrap();
        let mut registry =
            FixRegistry::from_fields([symbol, tagged("Price", 44, DataType::Float64)]).unwrap();

        // Another spelling of tag 55's field: its own tag, its own alternates and
        // aliases, one of which is the stored alias in another case.
        let mut incoming = tagged("symbol", 9001, DataType::utf8());
        FixFieldMut::new(&mut incoming).set_tags(&[66]).unwrap();
        FixFieldMut::new(&mut incoming)
            .set_names(["Sym", "TICKER"])
            .unwrap();
        FixFieldMut::new(&mut incoming)
            .set_description("incoming")
            .unwrap();
        assert!(!registry.add_field(incoming.clone()).unwrap());
        assert_eq!(super::scalars(&registry), 2 + super::seeded_fields());

        // The stored field keeps its identity and spelling; the union is stored
        // order first, then what only the incoming field stated, then its tag.
        let stored = registry.field_by_tag(55).unwrap();
        assert_eq!(stored.name(), "Symbol");
        assert_eq!(
            FixField::new(stored).id().unwrap(),
            Some(FixId::of(55, "Symbol").unwrap())
        );
        assert_eq!(FixField::new(stored).tags().unwrap(), [65, 66, 9001]);
        assert_eq!(
            FixField::new(stored).names().collect::<Vec<_>>(),
            ["Ticker", "Sym"]
        );
        assert_eq!(stored.description(), Some("incoming"));

        // The incoming tags resolve to the merged field as alternates, and the
        // merged field keeps the one identity it had: the identity the incoming
        // field arrived under names no field, because an alternate tag is a
        // spelling of the holder and not a second identity.
        for alternate in [9001, 66, 65] {
            assert!(
                std::ptr::eq(registry.get_field_by_tag(alternate).unwrap(), stored),
                "{alternate}"
            );
        }
        assert!(std::ptr::eq(
            registry
                .get_field_by_id(FixId::of(55, "symbol").unwrap())
                .unwrap(),
            stored
        ));
        assert!(
            registry
                .get_field_by_id(FixId::of(9001, "symbol").unwrap())
                .is_none()
        );
        assert_eq!(registry.field("sym").unwrap().name(), "Symbol");
        assert_eq!(registry.field(65).unwrap().name(), "Symbol");

        // Folding the same definition again changes nothing.
        let before = registry.clone();
        assert!(!registry.add_field(incoming).unwrap());
        assert_eq!(registry, before);
    }

    #[test]
    fn a_name_that_is_a_stored_alias_folds_into_the_alias_holder() {
        crate::install::installed();
        let mut symbol = tagged("Symbol", 55, DataType::utf8());
        FixFieldMut::new(&mut symbol).set_names(["Ticker"]).unwrap();
        let mut registry = FixRegistry::from_fields([symbol]).unwrap();

        assert!(
            !registry
                .add_field(tagged("ticker", 9001, DataType::utf8()))
                .unwrap()
        );
        let stored = registry.field_by_tag(9001).unwrap();
        assert_eq!(stored.name(), "Symbol");
        assert_eq!(FixField::new(stored).tags().unwrap(), [9001]);
        assert_eq!(
            FixField::new(stored).names().collect::<Vec<_>>(),
            ["Ticker"]
        );
        assert_eq!(registry.field("TICKER").unwrap().name(), "Symbol");
        assert_eq!(super::scalars(&registry), 1 + super::seeded_fields());
    }

    #[test]
    fn a_tag_another_field_answers_is_not_taken_by_a_name_fold() {
        crate::install::installed();
        let mut price = tagged("Price", 44, DataType::Float64);
        FixFieldMut::new(&mut price).set_tags(&[9001]).unwrap();
        let mut registry =
            FixRegistry::from_fields([price, tagged("Symbol", 55, DataType::utf8())]).unwrap();

        // The name folds, the tag is `Price`'s: the field merges, the tag stays.
        assert!(
            !registry
                .add_field(tagged("SYMBOL", 9001, DataType::utf8()))
                .unwrap()
        );
        assert_eq!(registry.field_by_tag(9001).unwrap().name(), "Price");
        assert!(
            FixField::new(registry.field_by_tag(55).unwrap())
                .tags()
                .unwrap()
                .is_empty()
        );

        // An alternate tag another field holds as its alternate is the conflict
        // `update` raises.
        let before = registry.clone();
        let mut clash = tagged("symbol", 9002, DataType::utf8());
        FixFieldMut::new(&mut clash).set_tags(&[9001]).unwrap();
        let error = registry.add_field(clash).unwrap_err();
        assert!(error.is_conflict(), "{error}");
        assert_eq!(registry, before);

        // A canonical tag another field holds canonically under another name
        // would be a field of its own beside the holder - but its name is
        // `Symbol`'s canonical one, and a name reaching a held field is rule
        // 4 before rule 5: this is `Symbol` spelled with `Price`'s number, so
        // it merges into `Symbol`, which gains nothing it already has, and 44
        // stays `Price`'s.
        assert!(
            !registry
                .add_field(tagged("symbol", 44, DataType::utf8()))
                .unwrap()
        );
        assert_eq!(registry, before);
        assert_eq!(registry.field_by_tag(44).unwrap().name(), "Price");
    }

    #[test]
    fn a_held_tag_under_a_held_name_merges_into_the_holder_of_the_name_and_the_tag_stays() {
        crate::install::installed();
        // `MaturityDate` holds 541 and `MaturityDate2` holds 9999. A source
        // calling 541 `MaturityDate2` names a field the dictionary holds
        // under another number - rule 4, the same field spelled with another
        // tag - and is no field beside the holder of 541, whose name would be
        // `MaturityDate2`'s: it merges into `MaturityDate2`, which gains
        // nothing it already has, and 541 stays `MaturityDate`'s.
        let registry = FixRegistry::from_fields([
            tagged("MaturityDate", 541, DataType::Date32),
            tagged("MaturityDate2", 9999, DataType::Date32),
        ])
        .unwrap();
        let arriving = tagged("maturitydate2", 541, DataType::Date32);

        let mut folded = registry.clone();
        assert!(!folded.add_field(arriving.clone()).unwrap());
        assert_eq!(folded, registry);
        assert_eq!(folded.field_by_tag(541).unwrap().name(), "MaturityDate");
        let second = folded.field_by_name("MaturityDate2").unwrap();
        assert_eq!(FixField::new(second).tag().unwrap(), Some(9999));
        assert!(FixField::new(second).tags().unwrap().is_empty());
        assert_eq!(super::scalars(&folded), 2 + super::seeded_fields());

        // The same through a fold with another dictionary: merged, clean.
        let other = FixRegistry::from_fields([arriving.clone()]).unwrap();
        let mut folded = registry.clone();
        let merge = folded.merge_with(&other).unwrap();
        assert!(merge.is_clean(), "{:?}", merge.dropped);
        assert_eq!(merge.added, 0);
        assert_eq!(folded, registry);

        // A holder of the tag named by nothing but its digits changes
        // nothing: the name is held already, so the arrival is the named
        // field spelled with another number, never the holder's name.
        let mut unnamed = FixRegistry::from_fields([
            tagged("541", 541, DataType::Date32),
            tagged("MaturityDate2", 9999, DataType::Date32),
        ])
        .unwrap();
        let before = unnamed.clone();
        assert!(!unnamed.add_field(arriving).unwrap());
        assert_eq!(unnamed, before);
        assert_eq!(unnamed.field_by_tag(541).unwrap().name(), "541");
    }

    #[test]
    fn a_datatype_disagreement_by_name_is_refused_and_writes_nothing() {
        crate::install::installed();
        let mut registry =
            FixRegistry::from_fields([tagged("Symbol", 55, DataType::utf8())]).unwrap();
        let before = registry.clone();
        let error = registry
            .add_field(tagged("symbol", 9001, DataType::Int32))
            .unwrap_err();
        assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
        let message = error.to_string();
        assert!(
            message.contains("utf8") && message.contains("int32"),
            "{message}"
        );
        assert_eq!(registry, before);
        assert!(registry.get_field_by_tag(9001).is_none());
    }

    #[test]
    fn a_crate_field_is_neither_added_nor_merged() {
        crate::install::installed();
        let mut registry = FixRegistry::new();
        let before = registry.clone();
        let own = yggdryl_fix::fix_crate_fields().unwrap()[0].clone();
        assert!(!registry.add_field(own).unwrap());
        assert_eq!(registry, before);
    }

    #[test]
    fn nested_fields_redirect_to_the_category_their_shape_names() {
        crate::install::installed();
        let mut registry =
            FixRegistry::from_fields([tagged("NoPartyIDs", 453, DataType::Int32)]).unwrap();

        let mut message =
            DataType::from(StructType::from_fields([]).unwrap()).required_field("Order");
        FixFieldMut::new(&mut message).set_msgtype("D").unwrap();
        assert!(registry.add_field(message).unwrap());
        assert_eq!(registry.msgtype("D").unwrap().name(), "Order");

        let item = StructType::from_fields([DataType::utf8().nullable_field("PartyID")])
            .map(DataType::from)
            .unwrap()
            .required_field("Party");
        assert!(registry.add_field(item.clone()).unwrap());
        assert_eq!(registry.field_by_name("Party").unwrap().field_len(), 1);

        let mut group = DataType::serie(item.clone()).nullable_field("Parties");
        FixFieldMut::new(&mut group).set_counter(453).unwrap();
        assert!(registry.add_field(group).unwrap());
        let mut hops = DataType::large_serie(item.clone()).nullable_field("Hops");
        FixFieldMut::new(&mut hops).set_counter(453).unwrap();
        assert!(registry.add_field(hops).unwrap());
        for name in ["Parties", "Hops"] {
            assert_eq!(
                FixField::new(registry.field_by_name(name).unwrap())
                    .counter()
                    .unwrap(),
                Some(453),
                "{name}"
            );
        }
        // A definition is a field of the registry, reached through the one set
        // of field doors, and the insert verb files a scalar as a scalar.
        assert!(registry.get_field("Party").is_some());
        assert!(
            registry
                .add_field(tagged("Symbol", 55, DataType::utf8()))
                .unwrap()
        );
        assert_eq!(registry.field(55).unwrap().name(), "Symbol");

        // A nested datatype that is no definition is refused as a scalar is.
        let before = registry.clone();
        let nullable = DataType::from(StructType::from_fields([]).unwrap()).nullable_field("Loose");
        for refused in [
            DataType::serie(nullable).nullable_field("Occurrences"),
            DataType::serie(DataType::utf8().nullable_field("Text")).nullable_field("Texts"),
        ] {
            let error = registry.add_field(refused).unwrap_err();
            assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
            assert!(error.to_string().contains("scalar"), "{error}");
            assert_eq!(registry, before);
        }
    }

    #[test]
    fn a_component_extended_by_a_member_is_seen_extended_by_every_reference() {
        crate::install::installed();
        let mut registry = catalog();
        assert!(
            registry
                .add_field(tagged("PartyNote", 9002, DataType::utf8()))
                .unwrap()
        );
        let mut note = registry.field(9002).unwrap().clone();
        FixFieldMut::new(&mut note)
            .set_field_ref("PartyNote")
            .unwrap();
        let mut extended = registry.field_by_name("Party").unwrap().clone();
        extended
            .set_dtype(DataType::from(
                StructType::from_fields(extended.fields().iter().cloned().chain([note])).unwrap(),
            ))
            .unwrap();

        assert!(!registry.add_field(extended.clone()).unwrap());
        let party = registry.field_by_name("Party").unwrap();
        assert_eq!(names(party), ["PartyID", "PartyNote"]);
        for path in [
            "Party.PartyNote",
            "Parties.PartyNote",
            "NewOrderSingle.Parties.PartyNote",
        ] {
            let member = registry.field_by_path(&fpath(path)).unwrap();
            assert_eq!(FixField::new(member).tag().unwrap(), Some(9002), "{path}");
            assert_eq!(
                FixField::new(member).field_ref(),
                Some("partynote"),
                "{path}"
            );
        }
        let parties = registry
            .field_by_path(&fpath("NewOrderSingle.Parties"))
            .unwrap();
        assert_eq!(names(occurrence(parties)), ["PartyID", "PartyNote"]);
        assert_eq!(
            registry
                .msgtype("D")
                .unwrap()
                .get_group_by_tag(453)
                .unwrap()
                .name(),
            "Parties"
        );
        assert_eq!(
            FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
            registry
        );

        // Folding the same definition again changes nothing, and the strict
        // verb still refuses the name it holds.
        let before = registry.clone();
        assert!(!registry.add_field(extended.clone()).unwrap());
        assert_eq!(registry, before);
        // The strict verb replaces the definition the name holds rather than
        // refusing it, and replacing it by itself leaves the registry as it was.
        assert!(registry.insert(extended).unwrap().is_some());
        assert_eq!(registry, before);

        // A message extends the same way, keeping its code.
        let mut order = registry.msgtype("D").unwrap().as_field().clone();
        order
            .set_dtype(
                StructType::from_fields(
                    order
                        .fields()
                        .iter()
                        .cloned()
                        .chain([DataType::utf8().nullable_field("Text")]),
                )
                .map(DataType::from)
                .unwrap(),
            )
            .unwrap();
        assert!(!registry.add_field(order).unwrap());
        let order = registry.msgtype("D").unwrap();
        assert_eq!(order.as_str(), "D");
        assert_eq!(names(order.as_field()), ["Parties", "Text"]);
    }

    #[test]
    fn a_group_occurrence_is_extended_where_its_members_live() {
        crate::install::installed();
        let mut registry = catalog();

        // A bare list stating one more member of the occurrence: the stored
        // occurrence is the component's, so the member lands there and the
        // group keeps its markers.
        let member = StructType::from_fields([DataType::utf8().nullable_field("PartyNote")])
            .map(DataType::from)
            .unwrap()
            .required_field("party");
        let mut group = DataType::serie(member).nullable_field("parties");
        FixFieldMut::new(&mut group).set_counter(453).unwrap();
        assert!(!registry.add_field(group).unwrap());
        let party = registry.field_by_name("Party").unwrap();
        assert_eq!(names(party), ["PartyID", "PartyNote"]);
        assert_eq!(
            registry
                .field_by_path(&fpath("NewOrderSingle.Parties.PartyNote"))
                .unwrap()
                .dtype(),
            &DataType::utf8()
        );
        let parties = registry.field_by_name("Parties").unwrap();
        assert_eq!(parties.name(), "Parties");
        assert_eq!(FixField::new(parties).counter().unwrap(), Some(453));
        assert_eq!(FixField::new(parties).component(), Some("party"));
        assert_eq!(
            FixField::new(occurrence(parties)).component(),
            Some("party")
        );

        // An inline occurrence is appended to in place.
        registry
            .add_field(tagged("NoHops", 627, DataType::Int32))
            .unwrap();
        let hop = StructType::from_fields([DataType::utf8().nullable_field("HopID")])
            .map(DataType::from)
            .unwrap()
            .required_field("Hop");
        let mut hops = DataType::serie(hop).nullable_field("Hops");
        FixFieldMut::new(&mut hops).set_counter(627).unwrap();
        assert!(registry.add_field(hops.clone()).unwrap());
        let more = StructType::from_fields([
            DataType::utf8().nullable_field("HopNote"),
            DataType::utf8().nullable_field("hopid"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("Hop");
        hops.set_dtype(DataType::serie(more)).unwrap();
        assert!(!registry.add_field(hops).unwrap());
        let hops = registry.field_by_name("Hops").unwrap();
        assert_eq!(names(occurrence(hops)), ["HopID", "HopNote"]);
        assert_eq!(FixField::new(hops).counter().unwrap(), Some(627));
        assert_eq!(
            FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
            registry
        );
    }

    #[test]
    fn definition_merges_refuse_a_member_that_disagrees_atomically() {
        crate::install::installed();
        let mut registry = catalog_with([DataType::Int32.nullable_field("Extra")]);
        let before = registry.clone();

        // The same member under another datatype.
        let changed = StructType::from_fields([DataType::Int64.nullable_field("extra")])
            .map(DataType::from)
            .unwrap()
            .required_field("Party");
        let error = registry.add_field(changed).unwrap_err();
        assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
        let message = error.to_string();
        assert!(
            message.contains("Party.Extra")
                && message.contains("int32")
                && message.contains("int64"),
            "{message}"
        );
        assert_eq!(registry, before);

        // A reference restated inline under another datatype is the same
        // disagreement, and the refusal names the reference and both datatypes.
        let inline = StructType::from_fields([tagged("PartyID", 448, DataType::Int32)])
            .map(DataType::from)
            .unwrap()
            .required_field("Party");
        let error = registry.add_field(inline).unwrap_err();
        assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
        let message = error.to_string();
        assert!(
            message.contains("Party.PartyID")
                && message.contains("reference")
                && message.contains("utf8")
                && message.contains("int32"),
            "{message}"
        );
        assert_eq!(registry, before);

        // A message under another code, and a group under another counter, are
        // the conflicts the `FIX:` merge raises for a second identity.
        let mut recoded = registry.msgtype("D").unwrap().as_field().clone();
        FixFieldMut::new(&mut recoded).set_msgtype("E").unwrap();
        let error = registry.add_field(recoded).unwrap_err();
        assert!(error.is_conflict(), "{error}");
        assert_eq!(registry, before);
        let mut recounted = registry.field_by_name("Parties").unwrap().clone();
        FixFieldMut::new(&mut recounted).set_counter(627).unwrap();
        let error = registry.add_field(recounted).unwrap_err();
        assert!(error.is_conflict(), "{error}");
        assert_eq!(registry, before);
    }

    #[test]
    fn add_fields_counts_what_arrived_and_what_folded() {
        crate::install::installed();
        let mut symbol = tagged("Symbol", 55, DataType::utf8());
        FixFieldMut::new(&mut symbol).set_names(["Ticker"]).unwrap();
        let mut registry =
            FixRegistry::from_fields([symbol, tagged("Price", 44, DataType::Float64)]).unwrap();
        registry
            .insert(
                StructType::from_fields([])
                    .map(DataType::from)
                    .unwrap()
                    .required_field("Instrument"),
            )
            .unwrap();

        let mut described = tagged("SYMBOL", 55, DataType::utf8());
        FixFieldMut::new(&mut described)
            .set_description("by identity")
            .unwrap();
        let mut extended = StructType::from_fields([DataType::utf8().nullable_field("Symbol")])
            .map(DataType::from)
            .unwrap()
            .required_field("instrument");
        FixFieldMut::new(&mut extended)
            .set_description("by name")
            .unwrap();
        let (added, merged) = registry
            .add_fields([
                yggdryl_fix::fix_crate_fields().unwrap()[0].clone(),
                described,
                tagged("ticker", 9001, DataType::utf8()),
                tagged("Text", 58, DataType::utf8()),
                DataType::from(StructType::from_fields([]).unwrap()).required_field("Header"),
                extended,
            ])
            .unwrap();
        assert_eq!((added, merged), (2, 3));
        assert_eq!(super::scalars(&registry), 3 + super::seeded_fields());
        assert_eq!(
            registry.field(55).unwrap().description(),
            Some("by identity")
        );
        assert_eq!(registry.field(9001).unwrap().name(), "Symbol");
        let instrument = registry.field_by_name("Instrument").unwrap();
        assert_eq!(instrument.description(), Some("by name"));
        assert_eq!(names(instrument), ["Symbol"]);
        // The crate's own message is a component too, and so is its `instids`,
        // so both count here beside `Instrument` and `Header`.
        assert_eq!(
            super::definitions(&registry, FixCategory::Components).count(),
            2 + super::crated_components()
        );

        // One mutation: a refusal in the middle writes nothing.
        let before = registry.clone();
        let error = registry
            .add_fields([
                tagged("Text", 58, DataType::utf8()),
                tagged("symbol", 9002, DataType::Int32),
                tagged("Account", 1, DataType::utf8()),
            ])
            .unwrap_err();
        assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
        assert_eq!(registry, before);
    }

    #[test]
    fn merging_a_dictionary_folds_its_definitions_rather_than_replacing_them() {
        crate::install::installed();
        let mut target = catalog();
        let mut described = target.field_by_name("Party").unwrap().clone();
        FixFieldMut::new(&mut described)
            .set_description("stored wording")
            .unwrap();
        target.update(described).unwrap();

        let mut source = catalog();
        source
            .add_field(tagged("PartyNote", 9002, DataType::utf8()))
            .unwrap();
        let mut note = source.field(9002).unwrap().clone();
        FixFieldMut::new(&mut note)
            .set_field_ref("PartyNote")
            .unwrap();
        let mut extended = source.field_by_name("Party").unwrap().clone();
        extended
            .set_dtype(DataType::from(
                StructType::from_fields(extended.fields().iter().cloned().chain([note])).unwrap(),
            ))
            .unwrap();
        source.add_field(extended).unwrap();
        let before_source = source.clone();

        // The two standard clock seeds are ordinary definitions and merge too.
        assert_eq!(
            target
                .merge_with(&source)
                .map(|merge| (merge.added, merge.merged))
                .unwrap(),
            (1, 4)
        );
        let party = target.field_by_name("Party").unwrap();
        assert_eq!(names(party), ["PartyID", "PartyNote"]);
        assert_eq!(party.description(), Some("stored wording"));
        assert_eq!(
            FixField::new(
                target
                    .field_by_path(&fpath("NewOrderSingle.Parties.PartyNote"))
                    .unwrap()
            )
            .tag()
            .unwrap(),
            Some(9002)
        );
        assert_eq!(source, before_source);
        assert_eq!(
            FixRegistry::from_json(&target.into_json().unwrap()).unwrap(),
            target
        );
        let before = target.clone();
        assert_eq!(
            target
                .merge_with(&source)
                .map(|merge| (merge.added, merge.merged))
                .unwrap(),
            (0, 5)
        );
        assert_eq!(target, before);

        // A member that disagrees is passed over and named; the member held
        // stays, and so does everything else.
        let disagreeing = catalog_with([DataType::Int64.nullable_field("Extra")]);
        let mut target = catalog_with([DataType::Int32.nullable_field("Extra")]);
        let before = target.clone();
        let merge = target.merge_with(&disagreeing).unwrap();
        assert_eq!(merge.dropped.len(), 1);
        let drop = &merge.dropped[0];
        assert_eq!(drop.incoming.name(), "Extra");
        assert_eq!(drop.incoming.dtype(), &DataType::Int64);
        assert!(
            drop.reason.contains("Party.Extra") && drop.reason.contains("int32"),
            "{drop}"
        );
        assert_eq!(target, before);
    }

    #[test]
    fn a_group_one_dialect_draws_from_another_component_widens_the_held_one() {
        crate::install::installed();
        // The source's `Parties` counts 453 as the target's does, but draws
        // its occurrences from a component of its own: one repeating group
        // read two ways. Its component's members fold into the component the
        // held group draws from, and nothing is passed over.
        let mut target = catalog();
        let mut source = FixRegistry::from_fields([
            tagged("NoPartyIDs", 453, DataType::Int32),
            tagged("PartyID", 448, DataType::utf8()),
            tagged("PartyRole", 452, DataType::Int32),
        ])
        .unwrap();
        let members: Vec<Field> = [(448, "PartyID"), (452, "PartyRole")]
            .into_iter()
            .map(|(tag, name)| {
                let mut member = source.field(tag).unwrap().clone();
                FixFieldMut::new(&mut member).set_field_ref(name).unwrap();
                member
            })
            .collect();
        source
            .insert(
                StructType::from_fields(members)
                    .map(DataType::from)
                    .unwrap()
                    .required_field("PartyExtra"),
            )
            .unwrap();
        let component = source.field_by_name("PartyExtra").unwrap().clone();
        let mut group = DataType::serie(component).nullable_field("Parties");
        FixFieldMut::new(&mut group).set_counter(453).unwrap();
        FixFieldMut::new(&mut group)
            .set_component("PartyExtra")
            .unwrap();
        source.insert(group).unwrap();

        let before = target.field_by_name("Parties").unwrap().clone();
        let merge = target.merge_with(&source).unwrap();
        assert!(merge.is_clean(), "{:?}", merge.dropped);
        let party = target.field_by_name("Party").unwrap();
        assert_eq!(names(party), ["PartyID", "PartyRole"]);
        let parties = target.field_by_name("Parties").unwrap();
        assert_eq!(
            FixField::new(parties).component(),
            FixField::new(&before).component()
        );
        assert_eq!(names(occurrence(parties)), ["PartyID", "PartyRole"]);
        // Once its members folded into `Party`, `PartyExtra` states the
        // structure `Party` holds: one structure is one definition, so it is
        // `Party` and arrives as nothing of its own.
        assert!(
            target.get_field_by_name("PartyExtra").is_none(),
            "it is the component it widened"
        );
        assert_eq!(
            target
                .field_by_path(&fpath("NewOrderSingle.Parties.PartyRole"))
                .unwrap()
                .dtype(),
            &DataType::Int32
        );
    }

    #[test]
    fn a_member_reading_an_arrival_passed_over_onto_a_holder_reads_the_name_it_takes_later() {
        crate::install::installed();
        // The target holds 9001 under no name. The source holds two fields on
        // it - `Flag`, a boolean the held count contradicts, as the tag's
        // holder, then `VenueRef`, text restating the count, beside it - and
        // a component reading `Flag`. Flag is passed over onto the holder and
        // VenueRef then names it, so the member reads the holder under that
        // name rather than the bare tag nothing answers to by the time the
        // definitions fold, and the source still folds.
        let mut target = FixRegistry::from_fields([tagged("9001", 9001, DataType::Int32)]).unwrap();
        let mut source = FixRegistry::from_fields([
            tagged("Flag", 9001, DataType::Boolean),
            tagged("VenueRef", 9001, DataType::utf8()),
        ])
        .unwrap();
        let mut member = source.field_by_name("Flag").unwrap().clone();
        FixFieldMut::new(&mut member).set_field_ref("Flag").unwrap();
        source
            .insert(
                StructType::from_fields([member])
                    .map(DataType::from)
                    .unwrap()
                    .required_field("Venue"),
            )
            .unwrap();

        let merge = target
            .merge_with(&source)
            .expect("one contradiction refuses nothing");
        let passed: Vec<String> = merge.dropped.iter().map(ToString::to_string).collect();
        assert_eq!(passed.len(), 1, "{passed:?}");
        assert!(passed[0].contains("at Flag"), "{passed:?}");
        let held = target.field(9001).unwrap();
        assert!(held.name().eq_ignore_ascii_case("VenueRef"), "{held:?}");
        assert_eq!(held.dtype(), &DataType::Int32);
        let venue = target.field_by_name("Venue").unwrap();
        assert_eq!(
            FixField::new(&venue.fields()[0]).field_ref(),
            Some("venueref")
        );
    }

    #[test]
    fn the_strict_verbs_keep_refusing_and_replacing() {
        crate::install::installed();
        let mut registry =
            FixRegistry::from_fields([tagged("Symbol", 55, DataType::utf8())]).unwrap();
        registry
            .insert(
                StructType::from_fields([DataType::Int32.nullable_field("Count")])
                    .map(DataType::from)
                    .unwrap()
                    .required_field("Plain"),
            )
            .unwrap();
        let before = registry.clone();

        let error = registry
            .insert(tagged("symbol", 9001, DataType::utf8()))
            .unwrap_err();
        assert!(error.is_conflict(), "{error}");
        assert_eq!(registry, before);
        let error = registry
            .update(tagged("symbol", 9001, DataType::utf8()))
            .unwrap_err();
        assert!(error.is_absent(), "{error}");
        assert_eq!(registry, before);
        // A definition insert replaces what the fold names, and the stored
        // spelling stays whatever the incoming one spelled.
        let replaced = registry
            .insert(DataType::from(StructType::from_fields([]).unwrap()).required_field("plain"))
            .unwrap();
        assert_eq!(
            replaced.as_ref().map(Field::name),
            Some("Plain"),
            "the definition it replaced"
        );
        assert_eq!(registry.field_by_name("Plain").unwrap().name(), "Plain");

        // `insert` replaces the members wholesale, where the lenient verb would
        // have kept `Count`.
        registry
            .insert(
                StructType::from_fields([DataType::utf8().nullable_field("Other")])
                    .map(DataType::from)
                    .unwrap()
                    .required_field("Plain"),
            )
            .unwrap();
        assert_eq!(names(registry.field_by_name("Plain").unwrap()), ["Other"]);
    }

    #[test]
    fn a_member_stated_inline_agrees_with_the_reference_stored_for_it() {
        crate::install::installed();
        let mut registry = catalog();

        // A dictionary built in memory states `PartyID` inline where the loaded
        // one references it: both describe tag 448 as utf8, so the stored
        // reference stays and only the new member arrives.
        let inline = StructType::from_fields([
            tagged("partyid", 448, DataType::utf8()),
            DataType::utf8().nullable_field("PartyNote"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("Party");
        assert!(!registry.add_field(inline).unwrap());
        let party = registry.field_by_name("Party").unwrap();
        assert_eq!(names(party), ["PartyID", "PartyNote"]);
        assert_eq!(
            FixField::new(&party.fields()[0]).field_ref(),
            Some("partyid")
        );
        assert_eq!(
            registry
                .field_by_path(&fpath("NewOrderSingle.Parties.PartyNote"))
                .unwrap()
                .dtype(),
            &DataType::utf8()
        );

        // The other way round: a stored inline member, restated by a reference
        // to the field of that datatype, is kept inline; a reference to a field
        // of another datatype is refused.
        registry
            .add_field(tagged("Symbol", 55, DataType::utf8()))
            .unwrap();
        registry
            .insert(
                StructType::from_fields([DataType::utf8().nullable_field("Symbol")])
                    .map(DataType::from)
                    .unwrap()
                    .required_field("Instrument"),
            )
            .unwrap();
        let mut symbol = registry.field(55).unwrap().clone();
        FixFieldMut::new(&mut symbol)
            .set_field_ref("Symbol")
            .unwrap();
        let restated = StructType::from_fields([symbol])
            .map(DataType::from)
            .unwrap()
            .required_field("Instrument");
        assert!(!registry.add_field(restated).unwrap());
        let instrument = registry.field_by_name("Instrument").unwrap();
        assert!(FixField::new(&instrument.fields()[0]).field_ref().is_none());
        let before = registry.clone();
        let mut count = tagged("Symbol", 9003, DataType::Int32);
        FixFieldMut::new(&mut count)
            .set_field_ref("NoPartyIDs")
            .unwrap();
        let disagreeing = StructType::from_fields([count])
            .map(DataType::from)
            .unwrap()
            .required_field("Instrument");
        let error = registry.add_field(disagreeing).unwrap_err();
        assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
        let message = error.to_string();
        assert!(
            message.contains("Instrument.Symbol") && message.contains("int32"),
            "{message}"
        );
        assert_eq!(registry, before);
    }

    #[test]
    fn a_required_spelling_folds_into_a_nullable_referenced_field_keeping_its_shape() {
        crate::install::installed();
        let mut registry = catalog();
        let mut respelled = DataType::utf8().required_field("partyid");
        FixFieldMut::new(&mut respelled).set_tag(9001).unwrap();
        assert!(!registry.add_field(respelled).unwrap());

        // The stored shape stays, and every reference to the field carries the
        // merged metadata.
        let stored = registry.field_by_tag(448).unwrap();
        assert!(stored.is_nullable());
        assert_eq!(FixField::new(stored).tags().unwrap(), [9001]);
        assert!(std::ptr::eq(
            registry.get_field_by_tag(9001).unwrap(),
            stored
        ));
        for path in ["Party.PartyID", "NewOrderSingle.Parties.PartyID"] {
            let member = registry.field_by_path(&fpath(path)).unwrap();
            assert_eq!(FixField::new(member).tags().unwrap(), [9001], "{path}");
            assert!(member.is_nullable(), "{path}");
        }
        assert_eq!(
            FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
            registry
        );
    }

    #[test]
    fn a_separator_respelling_is_a_spelling_of_the_stored_name() {
        crate::install::installed();
        let mut symbol = tagged("Symbol", 55, DataType::utf8());
        FixFieldMut::new(&mut symbol).set_names(["Ticker"]).unwrap();
        let mut registry = FixRegistry::from_fields([symbol]).unwrap();

        // The crate's one fold drops `_`, `-` and space beside the case, so a
        // name lookup, the fold by name and the alias dedupe all read `Sym_bol`
        // as `Symbol` and `Tick-er` as `Ticker`.
        assert_eq!(registry.field("sym_bol").unwrap().name(), "Symbol");
        assert_eq!(registry.field("Tick-er").unwrap().name(), "Symbol");
        let mut respelled = tagged("Sym_bol", 9001, DataType::utf8());
        FixFieldMut::new(&mut respelled)
            .set_names(["Tick-er", "SYM"])
            .unwrap();
        assert!(!registry.add_field(respelled).unwrap());
        let stored = registry.field_by_tag(9001).unwrap();
        assert_eq!(stored.name(), "Symbol");
        assert_eq!(FixField::new(stored).tags().unwrap(), [9001]);
        assert_eq!(
            FixField::new(stored).names().collect::<Vec<_>>(),
            ["Ticker", "SYM"]
        );
        assert_eq!(super::scalars(&registry), 1 + super::seeded_fields());

        // By identity the same respelling is the stored name too.
        let mut described = tagged("sym-bol", 55, DataType::utf8());
        FixFieldMut::new(&mut described)
            .set_description("respelled")
            .unwrap();
        assert!(!registry.add_field(described.clone()).unwrap());
        assert_eq!(registry.field(55).unwrap().description(), Some("respelled"));
        assert_eq!(registry.field(55).unwrap().name(), "Symbol");

        // The strict verbs read the fold the same way: a replacement keeps the
        // stored spelling, and a second identity under the stored name is the
        // conflict it always was.
        registry.update(described).unwrap();
        assert_eq!(registry.field(55).unwrap().name(), "Symbol");
        assert_eq!(
            registry
                .insert(tagged("SYM_BOL", 55, DataType::utf8()))
                .unwrap()
                .unwrap()
                .name(),
            "Symbol"
        );
        assert_eq!(registry.field(55).unwrap().name(), "Symbol");
        let before = registry.clone();
        let error = registry
            .insert(tagged("Sym_bol", 9002, DataType::utf8()))
            .unwrap_err();
        assert!(error.is_conflict(), "{error}");
        assert_eq!(registry, before);
    }

    #[test]
    fn a_canonical_identity_supersedes_the_alternate_another_field_lists() {
        crate::install::installed();
        // FIX itself does this: `QuoteAckStatus` is tag 1865, and `QuoteStatus`
        // lists 1865 as the tag it superseded. The canonical holder answers the
        // identifier whichever arrived first, exactly as a canonical name
        // answers over an alias - the alternate is a fallback, never a claim.
        let mut price = tagged("Price", 44, DataType::Float64);
        FixFieldMut::new(&mut price).set_tags(&[9001]).unwrap();
        let mut registry = FixRegistry::from_fields([price]).unwrap();
        assert!(
            registry
                .add_field(tagged("Symbol", 9001, DataType::utf8()))
                .unwrap()
        );
        assert_eq!(registry.field(9001).unwrap().name(), "Symbol");
        assert_eq!(
            registry
                .field(FixId::of(9001, "Symbol").unwrap())
                .unwrap()
                .name(),
            "Symbol"
        );
        assert_eq!(
            FixField::new(registry.field(44).unwrap()).tags().unwrap(),
            [9001]
        );

        let mut ticker = tagged("Ticker", 55, DataType::utf8());
        FixFieldMut::new(&mut ticker).set_tags(&[44]).unwrap();
        assert!(registry.add_field(ticker).unwrap());
        assert_eq!(registry.field(44).unwrap().name(), "Price");
        assert_eq!(
            registry
                .field(FixId::of(44, "Price").unwrap())
                .unwrap()
                .name(),
            "Price"
        );
        assert_eq!(
            FixField::new(registry.field(55).unwrap()).tags().unwrap(),
            [44]
        );
        assert_eq!(super::scalars(&registry), 3 + super::seeded_fields());
    }

    #[test]
    fn a_group_occurrence_folds_its_members_into_the_component_and_nothing_else() {
        crate::install::installed();
        let mut registry = catalog();
        let mut member = StructType::from_fields([DataType::utf8().nullable_field("PartyNote")])
            .map(DataType::from)
            .unwrap()
            .required_field("party");
        FixFieldMut::new(&mut member)
            .set_description("occurrence wording")
            .unwrap();
        let mut group = DataType::serie(member).nullable_field("parties");
        FixFieldMut::new(&mut group).set_counter(453).unwrap();
        FixFieldMut::new(&mut group)
            .set_description("group wording")
            .unwrap();
        assert!(!registry.add_field(group).unwrap());

        // The occurrence's root describes the group's occurrence, not the
        // component it happens to be: the member arrives there, the wording
        // does not, and the group takes its own.
        let party = registry.field_by_name("Party").unwrap();
        assert_eq!(names(party), ["PartyID", "PartyNote"]);
        assert_eq!(party.description(), None);
        let parties = registry.field_by_name("Parties").unwrap();
        assert_eq!(parties.description(), Some("group wording"));
        assert_eq!(occurrence(parties).description(), None);
    }

    #[test]
    fn a_reference_to_a_definition_arriving_in_the_same_merge_restates_an_inline_member() {
        crate::install::installed();
        // The target states the group inline inside its component; the source
        // holds the group as a definition and references it from the component.
        // Both describe one shape, and the group arrives in the same merge as
        // the reference to it, after the components in category order.
        let fields = [
            tagged("NoHops", 627, DataType::Int32),
            tagged("HopID", 628, DataType::utf8()),
        ];
        let hop = StructType::from_fields([DataType::utf8().nullable_field("HopID")])
            .map(DataType::from)
            .unwrap()
            .required_field("Hop");
        let mut target = FixRegistry::from_fields(fields.clone()).unwrap();
        target
            .insert(
                StructType::from_fields([DataType::serie(hop.clone()).nullable_field("Hops")])
                    .map(DataType::from)
                    .unwrap()
                    .required_field("Route"),
            )
            .unwrap();

        let mut source = FixRegistry::from_fields(fields).unwrap();
        let mut hops = DataType::serie(hop).nullable_field("Hops");
        FixFieldMut::new(&mut hops).set_counter(627).unwrap();
        source.insert(hops).unwrap();
        let mut restated = source.field_by_name("Hops").unwrap().clone();
        FixFieldMut::new(&mut restated).set_group("Hops").unwrap();
        source
            .insert(
                StructType::from_fields([restated, DataType::utf8().nullable_field("RouteID")])
                    .map(DataType::from)
                    .unwrap()
                    .required_field("Route"),
            )
            .unwrap();

        assert_eq!(
            target
                .merge_with(&source)
                .map(|merge| (merge.added, merge.merged))
                .unwrap(),
            (0, 4)
        );
        let route = target.field_by_name("Route").unwrap();
        assert_eq!(names(route), ["Hops", "RouteID"]);
        assert!(
            FixField::new(&route.fields()[0]).group().is_none(),
            "kept inline"
        );
        assert_eq!(
            FixField::new(target.field_by_name("Hops").unwrap())
                .counter()
                .unwrap(),
            Some(627)
        );
        assert_eq!(
            FixRegistry::from_json(&target.into_json().unwrap()).unwrap(),
            target
        );
    }

    #[test]
    fn an_inline_group_alike_to_a_reference_arriving_in_the_same_merge_keeps_its_statement() {
        crate::install::installed();
        // As above, with the target's inline group stating its counter: the
        // target's Route and the source's are then of one structure, and so
        // is a component the source names otherwise. The pass over alike
        // definitions meets a group stated inline against a member reading
        // a group of that structure - a reference, which the fold's
        // documents state as a placeholder - and keeps the inline statement
        // rather than refusing the fold: the merge is clean, Route keeps its
        // own nullability and its inline group, and the component the source
        // names otherwise is Route.
        let fields = [
            tagged("NoHops", 627, DataType::Int32),
            tagged("HopID", 628, DataType::utf8()),
        ];
        let hop = StructType::from_fields([DataType::utf8().nullable_field("HopID")])
            .map(DataType::from)
            .unwrap()
            .required_field("Hop");
        for name in ["Route", "Path"] {
            let mut target = FixRegistry::from_fields(fields.clone()).unwrap();
            let mut inline = DataType::serie(hop.clone()).nullable_field("Hops");
            FixFieldMut::new(&mut inline).set_counter(627).unwrap();
            target
                .insert(
                    StructType::from_fields([inline])
                        .map(DataType::from)
                        .unwrap()
                        .required_field("Route"),
                )
                .unwrap();

            let mut source = FixRegistry::from_fields(fields.clone()).unwrap();
            let mut hops = DataType::serie(hop.clone()).nullable_field("Hops");
            FixFieldMut::new(&mut hops).set_counter(627).unwrap();
            source.insert(hops).unwrap();
            let mut restated = source.field_by_name("Hops").unwrap().clone();
            FixFieldMut::new(&mut restated).set_group("Hops").unwrap();
            source
                .insert(
                    StructType::from_fields([restated])
                        .map(DataType::from)
                        .unwrap()
                        .nullable_field(name),
                )
                .unwrap();

            let merge = target
                .merge_with(&source)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(merge.is_clean(), "{name}: {:?}", merge.dropped);
            let route = target.field_by_name("Route").unwrap();
            assert_eq!(names(route), ["Hops"], "{name}");
            assert!(
                FixField::new(&route.fields()[0]).group().is_none(),
                "{name}: kept inline"
            );
            assert!(
                !route.is_nullable(),
                "{name}: a definition's own nullability is the held one's"
            );
            assert!(
                target
                    .get_definition(FixCategory::Components, "Path")
                    .is_none(),
                "{name}: one structure is one definition, Route"
            );
            assert_eq!(
                FixField::new(target.field_by_name("Hops").unwrap())
                    .counter()
                    .unwrap(),
                Some(627),
                "{name}"
            );
            assert_eq!(
                FixRegistry::from_json(&target.into_json().unwrap()).unwrap(),
                target,
                "{name}"
            );
        }
    }

    /// A component of one member reading `HopID(628)`, the structure the
    /// three cases below declare under two names.
    fn hop_of(registry: &FixRegistry, name: &str) -> Field {
        let mut hopid = registry.field(628).unwrap().clone();
        FixFieldMut::new(&mut hopid).set_field_ref("HopID").unwrap();
        StructType::from_fields([hopid])
            .map(DataType::from)
            .unwrap()
            .required_field(name)
    }

    /// A group on 627 drawing its occurrences from the held component
    /// `component`.
    fn hops_of(registry: &FixRegistry, name: &str, component: &str) -> Field {
        let item = registry.field_by_name(component).unwrap().clone();
        let mut group = DataType::serie(item).nullable_field(name);
        FixFieldMut::new(&mut group).set_counter(627).unwrap();
        FixFieldMut::new(&mut group)
            .set_component(component)
            .unwrap();
        group
    }

    #[test]
    fn a_reference_inside_an_inline_group_follows_the_component_it_folded_into() {
        crate::install::installed();
        // The target holds `Hop`; the source holds `Stop`, of Hop's
        // structure, and `Route`, whose `Hops` is a group stated inline on
        // 627 with its occurrence reading Stop. Stop is Hop, so every
        // reference to it reads Hop once the fold settles - the one in
        // Route's inline occurrence included, which no walk of Route's
        // members reaches: left naming the removed Stop, it would refuse
        // the whole merge.
        let fields = [
            tagged("NoHops", 627, DataType::Int32),
            tagged("HopID", 628, DataType::utf8()),
        ];
        let mut target = FixRegistry::from_fields(fields.clone()).unwrap();
        target.insert(hop_of(&target, "Hop")).unwrap();
        let mut source = FixRegistry::from_fields(fields).unwrap();
        source.insert(hop_of(&source, "Stop")).unwrap();
        let mut item = source.field_by_name("Stop").unwrap().clone();
        FixFieldMut::new(&mut item).set_component("Stop").unwrap();
        let mut hops = DataType::serie(item).nullable_field("Hops");
        FixFieldMut::new(&mut hops).set_counter(627).unwrap();
        source
            .insert(
                StructType::from_fields([hops])
                    .map(DataType::from)
                    .unwrap()
                    .required_field("Route"),
            )
            .unwrap();

        let merge = target
            .merge_with(&source)
            .expect("Stop is Hop, read as Hop wherever Stop was read");
        assert!(merge.is_clean(), "{:?}", merge.dropped);
        assert!(
            target
                .get_definition(FixCategory::Components, "Stop")
                .is_none(),
            "one structure is one definition, Hop"
        );
        let route = target.field_by_name("Route").unwrap();
        assert_eq!(names(route), ["Hops"]);
        let hops = &route.fields()[0];
        assert!(FixField::new(hops).group().is_none(), "stated inline");
        assert_eq!(FixField::new(hops).counter().unwrap(), Some(627));
        assert!(
            FixField::new(occurrence(hops))
                .component()
                .is_some_and(|name| name.eq_ignore_ascii_case("Hop")),
            "the inline occurrence reads the component Stop folded into: {:?}",
            FixField::new(occurrence(hops)).component()
        );
        assert_eq!(names(occurrence(hops)), ["HopID"]);
        assert_eq!(
            FixRegistry::from_json(&target.into_json().unwrap()).unwrap(),
            target
        );
    }

    #[test]
    fn two_definitions_of_one_structure_arriving_with_one_source_stay_two() {
        crate::install::installed();
        // The source states Hop and Stop, two components of one structure,
        // and Hops and Stops, two groups on 627 drawing on each - as FIX
        // Latest states InstrmtLegSecList beside SecLstUpdRelSymsLeg and
        // their groups on 555. A dictionary stating two definitions of one
        // structure states two, and a fold keeps what its source states:
        // folded into a dictionary holding neither, all four stand, so a
        // dictionary folded into an empty one is that dictionary. Only what
        // was held before the fold makes two alike: folded into one holding
        // Hop and Hops, Stop is Hop and Stops is Hops.
        let fields = [
            tagged("NoHops", 627, DataType::Int32),
            tagged("HopID", 628, DataType::utf8()),
        ];
        let mut source = FixRegistry::from_fields(fields.clone()).unwrap();
        source.insert(hop_of(&source, "Hop")).unwrap();
        source.insert(hop_of(&source, "Stop")).unwrap();
        source.insert(hops_of(&source, "Hops", "Hop")).unwrap();
        source.insert(hops_of(&source, "Stops", "Stop")).unwrap();

        let mut empty = FixRegistry::from_fields(fields.clone()).unwrap();
        let merge = empty.merge_with(&source).unwrap();
        assert!(merge.is_clean(), "{:?}", merge.dropped);
        for (category, name) in [
            (FixCategory::Components, "Hop"),
            (FixCategory::Components, "Stop"),
            (FixCategory::Groups, "Hops"),
            (FixCategory::Groups, "Stops"),
        ] {
            assert!(
                empty.get_definition(category, name).is_some(),
                "{name} stands: the source states two"
            );
        }
        for (group, component) in [("Hops", "Hop"), ("Stops", "Stop")] {
            let held = empty.field_by_name(group).unwrap();
            assert!(
                FixField::new(held)
                    .component()
                    .is_some_and(|name| name.eq_ignore_ascii_case(component)),
                "{group} draws on {component}: {:?}",
                FixField::new(held).component()
            );
        }
        assert_eq!(
            FixRegistry::from_json(&empty.into_json().unwrap()).unwrap(),
            empty
        );

        let mut held = FixRegistry::from_fields(fields).unwrap();
        held.insert(hop_of(&held, "Hop")).unwrap();
        held.insert(hops_of(&held, "Hops", "Hop")).unwrap();
        let merge = held.merge_with(&source).unwrap();
        assert!(merge.is_clean(), "{:?}", merge.dropped);
        for (category, name) in [
            (FixCategory::Components, "Stop"),
            (FixCategory::Groups, "Stops"),
        ] {
            assert!(
                held.get_definition(category, name).is_none(),
                "{name} is the definition held before the fold"
            );
        }
        assert_eq!(names(held.field_by_name("Hop").unwrap()), ["HopID"]);
    }

    #[test]
    fn a_name_held_for_another_structure_merges_by_name_and_folding_it_again_changes_nothing() {
        crate::install::installed();
        // The target holds Party (448, 447) under Parties on 453, and
        // NestedParty (524, 525) under NestedParties on 539, each spoken by
        // `a`. The source, spoken by `c`, declares NestedParty as 448 and
        // 447 - Party's structure under NestedParty's name - NestedParties on
        // 453 drawing on it, and a message G reading that group. Every
        // definition folds by name first: NestedParty is widened by the
        // source's members, as a name both dialects spell always merges, and
        // the group on 453 stands beside the held NestedParties on 539 as
        // NestedParties_453, drawing on the widened NestedParty - a structure
        // Parties does not state, so nothing folds into Parties, which keeps
        // its own source. Folding the source a second time finds every name
        // where the first fold left it and changes nothing.
        let mut target = FixRegistry::from_fields([
            tagged("NoPartyIDs", 453, DataType::Int32),
            tagged("PartyID", 448, DataType::utf8()),
            tagged("PartyIDSource", 447, DataType::utf8()),
            tagged("NoNestedPartyIDs", 539, DataType::Int32),
            tagged("NestedPartyID", 524, DataType::utf8()),
            tagged("NestedPartyIDSource", 525, DataType::utf8()),
        ])
        .unwrap();
        let party_of = |registry: &FixRegistry, name: &str, tags: [i32; 2], source: &str| {
            let members: Vec<Field> = tags
                .into_iter()
                .map(|tag| {
                    let mut member = registry.field(tag).unwrap().clone();
                    let spelling = member.name().to_owned();
                    FixFieldMut::new(&mut member)
                        .set_field_ref(&spelling)
                        .unwrap();
                    member
                })
                .collect();
            let mut component = StructType::from_fields(members)
                .map(DataType::from)
                .unwrap()
                .required_field(name);
            FixFieldMut::new(&mut component)
                .set_sources([source])
                .unwrap();
            component
        };
        let parties_of =
            |registry: &FixRegistry, name: &str, counter: i32, component: &str, source: &str| {
                let item = registry.field_by_name(component).unwrap().clone();
                let mut group = DataType::serie(item).nullable_field(name);
                FixFieldMut::new(&mut group).set_counter(counter).unwrap();
                FixFieldMut::new(&mut group)
                    .set_component(component)
                    .unwrap();
                FixFieldMut::new(&mut group).set_sources([source]).unwrap();
                group
            };
        target
            .insert(party_of(&target, "Party", [448, 447], "a"))
            .unwrap();
        target
            .insert(party_of(&target, "NestedParty", [524, 525], "a"))
            .unwrap();
        target
            .insert(parties_of(&target, "Parties", 453, "Party", "a"))
            .unwrap();
        target
            .insert(parties_of(
                &target,
                "NestedParties",
                539,
                "NestedParty",
                "a",
            ))
            .unwrap();

        let mut source = FixRegistry::from_fields([
            tagged("NoPartyIDs", 453, DataType::Int32),
            tagged("PartyID", 448, DataType::utf8()),
            tagged("PartyIDSource", 447, DataType::utf8()),
        ])
        .unwrap();
        source
            .insert(party_of(&source, "NestedParty", [448, 447], "c"))
            .unwrap();
        source
            .insert(parties_of(
                &source,
                "NestedParties",
                453,
                "NestedParty",
                "c",
            ))
            .unwrap();
        let mut member = source.field_by_name("NestedParties").unwrap().clone();
        FixFieldMut::new(&mut member)
            .set_group("NestedParties")
            .unwrap();
        let mut message = StructType::from_fields([member])
            .map(DataType::from)
            .unwrap()
            .required_field("Message47");
        FixFieldMut::new(&mut message).set_msgtype("G").unwrap();
        source.insert(message).unwrap();

        let merge = target.merge_with(&source).unwrap();
        assert!(merge.is_clean(), "{:?}", merge.dropped);
        let nested = target.field_by_name("NestedParty").unwrap();
        assert_eq!(
            names(nested),
            [
                "NestedPartyID",
                "NestedPartyIDSource",
                "PartyID",
                "PartyIDSource"
            ],
            "a name both dialects spell merges by name"
        );
        assert_eq!(
            FixField::new(nested).sources().collect::<Vec<_>>(),
            ["a", "c"]
        );
        let beside = target
            .get_definition(FixCategory::Groups, "NestedParties_453")
            .expect("the group on 453 stands beside the one on 539");
        assert_eq!(FixField::new(beside).counter().unwrap(), Some(453));
        assert!(
            FixField::new(beside)
                .component()
                .is_some_and(|component| component.eq_ignore_ascii_case("NestedParty"))
        );
        let party = target.field_by_name("Party").unwrap();
        assert_eq!(names(party), ["PartyID", "PartyIDSource"]);
        assert_eq!(
            FixField::new(party).sources().collect::<Vec<_>>(),
            ["a"],
            "no structure the fold made is Party's"
        );
        let message = target.msgtype("G").unwrap().as_field();
        let members: Vec<(&str, Option<&str>)> = message
            .fields()
            .iter()
            .map(|member| (member.name(), FixField::new(member).group()))
            .collect();
        assert_eq!(members.len(), 1, "{members:?}");
        assert!(
            members[0]
                .1
                .is_some_and(|group| group.eq_ignore_ascii_case("NestedParties_453")),
            "G reads the group on its own counter: {members:?}"
        );
        assert_eq!(
            FixRegistry::from_json(&target.into_json().unwrap()).unwrap(),
            target
        );

        let once = target.clone();
        let again = target.merge_with(&source).unwrap();
        assert!(again.is_clean(), "{:?}", again.dropped);
        assert_eq!(target, once, "folding one source twice changes nothing");
    }

    #[test]
    fn a_held_definition_the_fold_widens_into_another_s_structure_is_one_definition_under_its_name()
    {
        crate::install::installed();
        // The target holds `Leg` (600) and `LegFull` (600, 624), each spoken
        // by `a`, and a message X reading `LegFull`. The source, spoken by
        // `c`, declares `Leg` as 600 and 624: merged by name, `Leg` now
        // states `LegFull`'s structure - a pair the fold made, so it is one
        // definition. The name the source widened keeps answering for it,
        // `LegFull` folds into it, X reads `Leg`, and both sources are
        // listed. Folding the source a second time changes nothing.
        let mut target = FixRegistry::from_fields([
            tagged("LegSymbol", 600, DataType::utf8()),
            tagged("LegSide", 624, DataType::utf8()),
        ])
        .unwrap();
        let component = |registry: &FixRegistry, name: &str, tags: &[i32], source: &str| {
            let members: Vec<Field> = tags
                .iter()
                .map(|tag| {
                    let mut member = registry.field(*tag).unwrap().clone();
                    let spelling = member.name().to_owned();
                    FixFieldMut::new(&mut member)
                        .set_field_ref(&spelling)
                        .unwrap();
                    member
                })
                .collect();
            let mut component = StructType::from_fields(members)
                .map(DataType::from)
                .unwrap()
                .required_field(name);
            FixFieldMut::new(&mut component)
                .set_sources([source])
                .unwrap();
            component
        };
        target
            .insert(component(&target, "Leg", &[600], "a"))
            .unwrap();
        target
            .insert(component(&target, "LegFull", &[600, 624], "a"))
            .unwrap();
        let mut member = target.field_by_name("LegFull").unwrap().clone();
        member.set_name("Legs");
        FixFieldMut::new(&mut member)
            .set_component("LegFull")
            .unwrap();
        let mut message = StructType::from_fields([member])
            .map(DataType::from)
            .unwrap()
            .required_field("MessageX");
        FixFieldMut::new(&mut message).set_msgtype("X").unwrap();
        target.insert(message).unwrap();

        let mut source = FixRegistry::from_fields([
            tagged("LegSymbol", 600, DataType::utf8()),
            tagged("LegSide", 624, DataType::utf8()),
        ])
        .unwrap();
        source
            .insert(component(&source, "Leg", &[600, 624], "c"))
            .unwrap();

        let merge = target.merge_with(&source).unwrap();
        assert!(merge.is_clean(), "{:?}", merge.dropped);
        assert!(
            target
                .get_definition(FixCategory::Components, "LegFull")
                .is_none(),
            "one structure is one definition"
        );
        let leg = target.field_by_name("Leg").unwrap();
        assert_eq!(names(leg), ["LegSymbol", "LegSide"]);
        assert_eq!(FixField::new(leg).sources().collect::<Vec<_>>(), ["a", "c"]);
        let message = target.msgtype("X").unwrap().as_field();
        assert!(
            FixField::new(&message.fields()[0])
                .component()
                .is_some_and(|component| component.eq_ignore_ascii_case("Leg")),
            "X reads the definition the other folded into: {:?}",
            message.fields()[0]
        );
        assert_eq!(
            FixRegistry::from_json(&target.into_json().unwrap()).unwrap(),
            target
        );

        let once = target.clone();
        let again = target.merge_with(&source).unwrap();
        assert!(again.is_clean(), "{:?}", again.dropped);
        assert_eq!(target, once, "folding one source twice changes nothing");
    }

    /// The members of `component`, as `(name, tags)` - each a reference to the
    /// field on that tag - under one group on 453 each, read by one message
    /// per `(msgtype, group)`: a dictionary of party groups a CBlock would
    /// bring, every definition stating `source`.
    fn parties_dictionary(
        components: &[(&str, &[i32])],
        groups: &[(&str, &str)],
        messages: &[(&str, &str)],
        source: &str,
    ) -> FixRegistry {
        let mut registry = FixRegistry::from_fields([
            tagged("NoPartyIDs", 453, DataType::Int32),
            tagged("PartyIDSource", 447, DataType::utf8()),
            tagged("PartyID", 448, DataType::utf8()),
            tagged("PartyRole", 452, DataType::utf8()),
            tagged("PartySubID", 523, DataType::utf8()),
        ])
        .unwrap();
        for (name, tags) in components {
            let members: Vec<Field> = tags
                .iter()
                .map(|tag| {
                    let mut member = registry.field(*tag).unwrap().clone();
                    let spelling = member.name().to_owned();
                    FixFieldMut::new(&mut member)
                        .set_field_ref(&spelling)
                        .unwrap();
                    member
                })
                .collect();
            let mut component = StructType::from_fields(members)
                .map(DataType::from)
                .unwrap()
                .required_field(*name);
            FixFieldMut::new(&mut component)
                .set_sources([source])
                .unwrap();
            registry.insert(component).unwrap();
        }
        for (name, component) in groups {
            let item = registry.field_by_name(component).unwrap().clone();
            let mut group = DataType::serie(item).nullable_field(*name);
            FixFieldMut::new(&mut group).set_counter(453).unwrap();
            FixFieldMut::new(&mut group)
                .set_component(component)
                .unwrap();
            FixFieldMut::new(&mut group).set_sources([source]).unwrap();
            registry.insert(group).unwrap();
        }
        for (msgtype, group) in messages {
            let mut member = registry.field_by_name(group).unwrap().clone();
            FixFieldMut::new(&mut member).set_group(group).unwrap();
            let mut message = StructType::from_fields([member])
                .map(DataType::from)
                .unwrap()
                .required_field(format!("Message{msgtype}"));
            FixFieldMut::new(&mut message).set_msgtype(msgtype).unwrap();
            registry.insert(message).unwrap();
        }
        registry
    }

    /// The tags message `msgtype` reads through the one group it reads, in
    /// the order its component states them.
    fn tags_read(registry: &FixRegistry, msgtype: &str) -> Vec<i32> {
        let message = registry.msgtype(msgtype).unwrap().as_field();
        let group = FixField::new(&message.fields()[0])
            .group()
            .unwrap()
            .to_owned();
        let group = FixField::new(registry.definition(FixCategory::Groups, &group).unwrap())
            .component()
            .unwrap()
            .to_owned();
        registry
            .definition(FixCategory::Components, &group)
            .unwrap()
            .fields()
            .iter()
            .map(|member| FixField::new(member).tag().unwrap().unwrap())
            .collect()
    }

    #[test]
    fn a_member_folds_what_the_source_states_for_its_target_whichever_definition_sorts_first() {
        crate::install::installed();
        // `a` reads `Parties` (452) in E and `Dealers` (452, 523) in D, both
        // on 453; `b` reads `Parties` (447, 452, 523) in D. D's one member on
        // 453 is read two ways, so what `b` states for the group it reads
        // folds into the group the held D reads - what `b` states, never the
        // `Parties` held before `b`'s own `Parties` merged by name. `MessageD`
        // sorts before `Party`, so it folds first: read off the held
        // dictionary, the target would be `a`'s `Party` (452) and D would
        // lose 447 until `b` folded a second time.
        let a = || {
            parties_dictionary(
                &[("Dealer", &[452, 523]), ("Party", &[452])],
                &[("Dealers", "Dealer"), ("Parties", "Party")],
                &[("D", "Dealers"), ("E", "Parties")],
                "a",
            )
        };
        let b = || {
            parties_dictionary(
                &[("Party", &[447, 452, 523])],
                &[("Parties", "Party")],
                &[("D", "Parties")],
                "b",
            )
        };
        for (first, second, order) in [(a(), b(), "a then b"), (b(), a(), "b then a")] {
            let mut target = FixRegistry::new();
            assert!(target.merge_with(&first).unwrap().is_clean(), "{order}");
            let merge = target.merge_with(&second).unwrap();
            assert!(merge.is_clean(), "{order}: {:?}", merge.dropped);
            let mut d = tags_read(&target, "D");
            d.sort_unstable();
            assert_eq!(d, [447, 452, 523], "{order}: D reads what each file states");
            assert!(
                tags_read(&target, "E").contains(&452),
                "{order}: E reads what a states"
            );

            let once = target.clone();
            let again = target.merge_with(&second).unwrap();
            assert!(again.is_clean(), "{order}: {:?}", again.dropped);
            assert_eq!(
                target, once,
                "{order}: folding one source twice changes nothing"
            );
        }
    }

    #[test]
    fn a_split_a_source_widens_takes_what_the_source_states_and_the_held_group_keeps_its_name() {
        crate::install::installed();
        // The target holds `Party` (448, 447, 452, 523) under `Parties`, spoken
        // by `spec`, and a split one dialect `d` took for message E,
        // `Party_E` (448, 447) under `Parties_E`, which E reads. The source,
        // spoken by `e`, states `Party` as 448, 447, 452 and reads it in its
        // own E. E's one member on 453 is read two ways, so what `e` states
        // for it folds into the split E reads: the split states 448, 447 and
        // 452 - what `e` states, never the held `Party` that `e`'s `Party`
        // merged into by name - so it states no structure the dictionary
        // holds, and `Party` keeps its name and every member it held.
        // Widened by the held `Party` instead, the split would state its
        // structure, and the fold would file the dictionary's own `Party`
        // under a dialect's split, every message reading it renamed.
        let mut target = parties_dictionary(
            &[("Party", &[448, 447, 452, 523])],
            &[("Parties", "Party")],
            &[],
            "spec",
        );
        target
            .merge_with(&parties_dictionary(
                &[("Party_E", &[448, 447])],
                &[("Parties_E", "Party_E")],
                &[("E", "Parties_E")],
                "d",
            ))
            .unwrap();
        let source = parties_dictionary(
            &[("Party", &[448, 447, 452])],
            &[("Parties", "Party")],
            &[("E", "Parties")],
            "e",
        );

        let merge = target.merge_with(&source).unwrap();
        assert!(merge.is_clean(), "{:?}", merge.dropped);
        let party = target
            .get_definition(FixCategory::Components, "Party")
            .expect("the held group keeps its name");
        assert_eq!(
            names(party),
            ["PartyID", "PartyIDSource", "PartyRole", "PartySubID"]
        );
        assert_eq!(
            FixField::new(party).sources().collect::<Vec<_>>(),
            ["e", "spec"]
        );
        assert!(
            target
                .get_definition(FixCategory::Groups, "Parties")
                .is_some()
        );
        let split = target
            .get_definition(FixCategory::Components, "Party_E")
            .expect("the split states no structure the dictionary holds");
        assert_eq!(names(split), ["PartyID", "PartyIDSource", "PartyRole"]);
        assert_eq!(tags_read(&target, "E"), [448, 447, 452]);
        assert_eq!(
            FixRegistry::from_json(&target.into_json().unwrap()).unwrap(),
            target
        );

        let once = target.clone();
        let again = target.merge_with(&source).unwrap();
        assert!(again.is_clean(), "{:?}", again.dropped);
        assert_eq!(target, once, "folding one source twice changes nothing");
    }

    /// The registry a fold-table fixture starts from: `Symbol` on 55, spoken by
    /// `fix44`, `Price` on 44 and `MsgType` on 35.
    fn holders() -> FixRegistry {
        let mut symbol = tagged("Symbol", 55, DataType::utf8());
        FixFieldMut::new(&mut symbol)
            .set_sources(["fix44"])
            .unwrap();
        FixRegistry::from_fields([
            symbol,
            tagged("Price", 44, DataType::Float64),
            tagged("MsgType", 35, DataType::utf8()),
        ])
        .unwrap()
    }

    /// The four rows of the fold table as one arrival each, every one spoken
    /// by `venue`: the same identity respelled, a held tag under another name,
    /// a held name under another tag, and a field nothing holds.
    fn arrivals() -> [Field; 4] {
        let mut respelled = tagged("Msg_Type", 35, DataType::utf8());
        FixFieldMut::new(&mut respelled)
            .set_description("respelled")
            .unwrap();
        let mut arrivals = [
            respelled,
            tagged("VenueSymbol", 55, DataType::utf8()),
            tagged("price", 9001, DataType::Float64),
            tagged("Account", 1, DataType::utf8()),
        ];
        for arrival in &mut arrivals {
            FixFieldMut::new(arrival).set_sources(["venue"]).unwrap();
        }
        arrivals
    }

    /// What every row of the fold table leaves behind, whichever verb folded it.
    fn assert_fold_table(registry: &FixRegistry) {
        assert_eq!(super::scalars(registry), 5 + super::seeded_fields());

        // Row 1: the same tag under the same folded name is the same field.
        // `Msg_Type`, `msgtype` and `MsgType` are one id, so the respelling
        // merged and the stored spelling stayed.
        let msgtype = registry.field_by_tag(35).unwrap();
        assert_eq!(msgtype.name(), "MsgType");
        assert_eq!(msgtype.description(), Some("respelled"));
        for spelling in ["Msg_Type", "msgtype", "MsgType"] {
            let id = FixId::of(35, spelling).unwrap();
            assert_eq!(id, FixId::of(35, "MsgType").unwrap(), "{spelling}");
            assert!(
                std::ptr::eq(registry.field_by_id(id).unwrap(), msgtype),
                "{spelling}"
            );
        }
        assert_eq!(
            FixField::new(msgtype).sources().collect::<Vec<_>>(),
            ["venue"]
        );

        // Row 2: a held tag under another name is a second field beside the
        // holder, and neither learns the other's name: two fields sharing a
        // tag are two fields, not two spellings of one. The bare tag keeps
        // answering the first holder; the newcomer is reached by its name and
        // by its id, and holds the tag canonically too.
        let symbol = registry.field_by_tag(55).unwrap();
        assert_eq!(symbol.name(), "Symbol");
        assert!(FixField::new(symbol).names().next().is_none());
        assert!(FixField::new(symbol).tags().unwrap().is_empty());
        assert_eq!(
            FixField::new(symbol).sources().collect::<Vec<_>>(),
            ["fix44"],
            "the arrival's membership is its own"
        );
        let venue = registry.field_by_name("VenueSymbol").unwrap();
        assert!(!std::ptr::eq(venue, symbol));
        assert_eq!(venue.name(), "VenueSymbol");
        assert!(FixField::new(venue).names().next().is_none());
        assert_eq!(FixField::new(venue).tag().unwrap(), Some(55));
        assert!(std::ptr::eq(
            registry
                .field_by_id(FixId::of(55, "venue_symbol").unwrap())
                .unwrap(),
            venue
        ));
        assert!(std::ptr::eq(
            registry
                .field_by_id(FixId::of(55, "Symbol").unwrap())
                .unwrap(),
            symbol
        ));
        assert_eq!(
            FixField::new(venue).sources().collect::<Vec<_>>(),
            ["venue"]
        );

        // Row 3: a held name under another tag is the holder spelled with
        // another number: the tag becomes the holder's alternate and no second
        // field exists.
        let price = registry.field_by_tag(44).unwrap();
        assert_eq!(price.name(), "Price");
        assert_eq!(FixField::new(price).tags().unwrap(), [9001]);
        assert!(std::ptr::eq(registry.field_by_tag(9001).unwrap(), price));
        assert!(
            registry
                .get_field_by_id(FixId::of(9001, "price").unwrap())
                .is_none()
        );
        assert_eq!(
            FixField::new(price).sources().collect::<Vec<_>>(),
            ["venue"]
        );

        // Row 4: neither, so it arrived as it was.
        let account = registry.field_by_tag(1).unwrap();
        assert_eq!(account.name(), "Account");
        assert_eq!(
            FixField::new(account).sources().collect::<Vec<_>>(),
            ["venue"]
        );

        // Membership is provenance, listed and never resolved through.
        assert_eq!(registry.dialects(), ["fix44", "venue"]);

        // Tag-major, the holder first: the two fields on 55 are adjacent, the
        // holder of the bare tag leads whatever the ids say, and
        // `next_field_after` walks the same order.
        let order: Vec<(i32, FixId)> = registry
            .iter()
            .map(|field| {
                (
                    FixField::new(field).tag().unwrap().unwrap(),
                    FixField::new(field).id().unwrap().unwrap(),
                )
            })
            .filter(|(tag, _)| *tag < FixId::DEFINITION_TAG_MIN && !(65_000..65_100).contains(tag))
            .collect();
        let tags: Vec<i32> = order.iter().map(|(tag, _)| *tag).collect();
        let mut sorted = tags.clone();
        sorted.sort_unstable();
        assert_eq!(tags, sorted);
        assert_eq!(tags, [1, 35, 44, 52, 55, 55, 60]);
        let (first, second) = (order[4].1, order[5].1);
        assert_eq!(
            first,
            FixField::new(registry.field_by_tag(55).unwrap())
                .id()
                .unwrap()
                .unwrap()
        );
        assert_eq!(
            FixField::new(registry.next_field_after(Some(first)).unwrap())
                .id()
                .unwrap(),
            Some(second)
        );
    }

    #[test]
    fn the_fold_table_holds_through_add_field() {
        crate::install::installed();
        let mut registry = holders();
        let [respelled, venue, price, account] = arrivals();
        assert!(!registry.add_field(respelled).unwrap());
        assert!(registry.add_field(venue).unwrap());
        assert!(!registry.add_field(price).unwrap());
        assert!(registry.add_field(account).unwrap());
        assert_fold_table(&registry);

        // The same four again change nothing: the newcomer on 55 is now the
        // identity it holds, and the rest fold onto themselves.
        let before = registry.clone();
        assert_eq!(registry.add_fields(arrivals()).unwrap(), (0, 4));
        assert_eq!(registry, before);
    }

    #[test]
    fn two_fields_on_one_tag_survive_a_snapshot_round_trip() {
        crate::install::installed();
        // The snapshot writes fields in iteration order - tag-major, then id -
        // and reads them back in that order, so which of two fields on one tag
        // the bare tag answers has to be what the writer held: `Symbol` was
        // first. Neither holds the other's name as an alias, before the round
        // trip or after it.
        let mut registry = holders();
        let [_, venue, _, _] = arrivals();
        assert!(registry.add_field(venue).unwrap());
        let unaliased = |registry: &FixRegistry| {
            assert_eq!(registry.field_by_tag(55).unwrap().name(), "Symbol");
            for name in ["Symbol", "VenueSymbol"] {
                let field = registry.field_by_name(name).unwrap();
                assert_eq!(field.name(), name);
                assert_eq!(FixField::new(field).tag().unwrap(), Some(55), "{name}");
                assert!(FixField::new(field).names().next().is_none(), "{name}");
            }
        };
        unaliased(&registry);

        let restored = FixRegistry::from_json(&registry.into_json().unwrap()).unwrap();
        unaliased(&restored);
        assert_eq!(restored, registry);
    }

    #[test]
    fn the_fold_table_holds_through_merge_with() {
        crate::install::installed();
        let mut registry = holders();
        let source = FixRegistry::from_fields(arrivals()).unwrap();
        let before_source = source.clone();
        assert_eq!(
            registry
                .merge_with(&source)
                .map(|merge| (merge.added, merge.merged))
                .unwrap(),
            (2, 4)
        );
        assert_fold_table(&registry);
        assert_eq!(source, before_source);

        // The other way round says the same thing with the roles swapped: the
        // source's `VenueSymbol` is then the first holder of 55, and `Symbol`
        // arrives beside it, neither learning the other's name.
        let mut reversed = FixRegistry::from_fields(arrivals()).unwrap();
        assert_eq!(
            reversed
                .merge_with(&holders())
                .map(|merge| (merge.added, merge.merged))
                .unwrap(),
            (1, 4)
        );
        assert_eq!(super::scalars(&reversed), 5 + super::seeded_fields());
        assert_eq!(reversed.field_by_tag(55).unwrap().name(), "VenueSymbol");
        for name in ["VenueSymbol", "Symbol"] {
            let field = reversed.field_by_name(name).unwrap();
            assert_eq!(field.name(), name);
            assert!(FixField::new(field).names().next().is_none(), "{name}");
        }
        assert_eq!(reversed.field_by_tag(9001).unwrap().name(), "price");
        assert_eq!(
            FixField::new(reversed.field_by_tag(9001).unwrap())
                .sources()
                .collect::<Vec<_>>(),
            ["venue"]
        );
        assert_eq!(reversed.dialects(), ["fix44", "venue"]);
    }

    #[test]
    fn a_merged_membership_is_the_union_of_what_each_side_spoke() {
        crate::install::installed();
        let mut symbol = tagged("Symbol", 55, DataType::utf8());
        FixFieldMut::new(&mut symbol)
            .set_sources(["fix44"])
            .unwrap();
        let mut registry = FixRegistry::from_fields([symbol.clone()]).unwrap();

        // By identity, by name and by tag, the dialects union, folded once,
        // deduplicated and sorted, however they were spelled.
        let mut respelled = tagged("SYMBOL", 55, DataType::utf8());
        FixFieldMut::new(&mut respelled)
            .set_sources(["Venue", "FIX44"])
            .unwrap();
        assert!(!registry.add_field(respelled).unwrap());
        let mut alternate = tagged("symbol", 9001, DataType::utf8());
        FixFieldMut::new(&mut alternate)
            .set_sources(["other"])
            .unwrap();
        assert!(!registry.add_field(alternate).unwrap());
        let stored = registry.field_by_tag(55).unwrap();
        assert_eq!(
            FixField::new(stored).sources().collect::<Vec<_>>(),
            ["fix44", "other", "venue"]
        );
        assert!(FixField::new(stored).has_source("VENUE"));
        assert!(!FixField::new(stored).has_source("standard"));
        assert_eq!(registry.dialects(), ["fix44", "other", "venue"]);

        // A field spoken by nobody stays spoken by nobody, and a source that
        // speaks it says so after the merge.
        let mut target =
            FixRegistry::from_fields([tagged("Price", 44, DataType::Float64)]).unwrap();
        assert!(
            FixField::new(target.field_by_tag(44).unwrap())
                .sources()
                .next()
                .is_none()
        );
        assert!(target.dialects().is_empty());
        let mut price = tagged("Price", 44, DataType::Float64);
        FixFieldMut::new(&mut price).set_sources(["venue"]).unwrap();
        let source = FixRegistry::from_fields([price, symbol]).unwrap();
        assert_eq!(
            target
                .merge_with(&source)
                .map(|merge| (merge.added, merge.merged))
                .unwrap(),
            (1, 3)
        );
        assert_eq!(
            FixField::new(target.field_by_tag(44).unwrap())
                .sources()
                .collect::<Vec<_>>(),
            ["venue"]
        );
        assert_eq!(
            FixField::new(target.field_by_tag(55).unwrap())
                .sources()
                .collect::<Vec<_>>(),
            ["fix44"]
        );
        assert_eq!(target.dialects(), ["fix44", "venue"]);
    }

    #[test]
    fn message_codes_live_in_one_namespace() {
        crate::install::installed();
        let mut registry = catalog();

        // A second message on a held code under another name is a second
        // message: the bare code keeps answering the first holder, the newcomer
        // is reached by its name, and its code is the held one.
        let mut venue = StructType::from_fields([DataType::utf8().nullable_field("VenueID")])
            .map(DataType::from)
            .unwrap()
            .required_field("VenueOrder");
        FixFieldMut::new(&mut venue).set_msgtype("D").unwrap();
        assert!(registry.add_field(venue.clone()).unwrap());
        assert_eq!(registry.msgtype("D").unwrap().name(), "NewOrderSingle");
        assert_eq!(registry.msgtype("VenueOrder").unwrap().as_str(), "D");
        assert_eq!(
            names(registry.msgtype("venue_order").unwrap().as_field()),
            ["VenueID"]
        );
        assert_eq!(super::msgtypes(&registry).count(), 2);
        assert_eq!(
            FixField::new(registry.field_by_name("VenueOrder").unwrap()).msgtype(),
            Some("D")
        );

        // A re-declaration under the same folded name folds into the stored
        // message, keeping its spelling and its code.
        let mut restated = StructType::from_fields([DataType::utf8().nullable_field("Text")])
            .map(DataType::from)
            .unwrap()
            .required_field("new_order_single");
        FixFieldMut::new(&mut restated).set_msgtype("D").unwrap();
        assert!(!registry.add_field(restated).unwrap());
        let order = registry.msgtype("D").unwrap();
        assert_eq!(order.name(), "NewOrderSingle");
        assert_eq!(names(order.as_field()), ["Parties", "Text"]);
        assert_eq!(super::msgtypes(&registry).count(), 2);
        assert_eq!(
            FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
            registry
        );

        // `merge_with` reads the namespace the same way.
        let mut target = catalog();
        let mut source = FixRegistry::new();
        source.insert(venue).unwrap();
        assert_eq!(
            target
                .merge_with(&source)
                .map(|merge| (merge.added, merge.merged))
                .unwrap(),
            (0, 2)
        );
        assert_eq!(target.msgtype("D").unwrap().name(), "NewOrderSingle");
        assert_eq!(target.msgtype("VenueOrder").unwrap().as_str(), "D");
        assert_eq!(super::msgtypes(&target).count(), 2);
        assert_eq!(names(target.msgtype("D").unwrap().as_field()), ["Parties"]);
    }

    #[test]
    fn a_bare_code_answers_the_message_the_code_set_names_else_the_first_in_name_order() {
        crate::install::installed();
        // Two messages on `D`: which one the bare code answers is a fact of the
        // catalog's content, never of the order it was built in, so a fold, a
        // store and a load all answer the same one. Without a code set naming
        // `D`, name order decides: `AlgoOrder` sorts before `NewOrderSingle`
        // however late it arrives.
        let mut registry = catalog();
        let mut algo = StructType::from_fields([DataType::utf8().nullable_field("AlgoID")])
            .map(DataType::from)
            .unwrap()
            .required_field("AlgoOrder");
        FixFieldMut::new(&mut algo).set_msgtype("D").unwrap();
        assert!(registry.add_field(algo.clone()).unwrap());
        assert_eq!(registry.msgtype("AlgoOrder").unwrap().as_str(), "D");
        assert_eq!(super::msgtypes(&registry).count(), 2);
        assert_eq!(registry.msgtype("D").unwrap().name(), "AlgoOrder");
        assert_eq!(registry.msgtype("NewOrderSingle").unwrap().as_str(), "D");

        // Tag 35's code set names `D` `NewOrderSingle`: that message answers the
        // bare code, whichever name sorts first and whichever arrived first, and
        // the code set arriving after both re-decides it.
        let mut msgtype = tagged("MsgType", 35, DataType::utf8());
        FixFieldMut::new(&mut msgtype)
            .set_codeset("msgtypecodeset")
            .unwrap();
        // The field names the set and the dictionary holds its members, so the
        // set is stated before the field that reads by it arrives.
        registry
            .set_codeset(
                "msgtypecodeset",
                &[yggdryl_fix::FixCode::new("NewOrderSingle", "D")],
            )
            .unwrap();
        assert!(registry.add_field(msgtype.clone()).unwrap());
        assert_eq!(registry.msgtype("D").unwrap().name(), "NewOrderSingle");
        assert_eq!(registry.msgtype("AlgoOrder").unwrap().as_str(), "D");

        let mut target = catalog();
        target
            .set_codeset(
                "msgtypecodeset",
                &[yggdryl_fix::FixCode::new("NewOrderSingle", "D")],
            )
            .unwrap();
        assert!(target.add_field(msgtype).unwrap());
        let mut source = FixRegistry::new();
        source.insert(algo).unwrap();
        assert_eq!(
            target
                .merge_with(&source)
                .map(|merge| (merge.added, merge.merged))
                .unwrap(),
            (0, 2)
        );
        assert_eq!(target.msgtype("AlgoOrder").unwrap().as_str(), "D");
        assert_eq!(target.msgtype("D").unwrap().name(), "NewOrderSingle");
        let restored = FixRegistry::from_json(&target.into_json().unwrap()).unwrap();
        assert_eq!(restored.msgtype("D").unwrap().name(), "NewOrderSingle");
    }

    /// Merging a dictionary that names one wire field differently keeps one member.
    ///
    /// Two dictionaries reach tag 448 under two spellings of one name, and each
    /// states the same component's member under its own. The merge folds the
    /// field - `party_id` and `PartyID` are one identity, so the arrival merges
    /// into the holder under the held spelling - and the component keeps one
    /// member, not two. Folding the same dictionary again changes nothing,
    /// which is the property a reload rests on.
    #[test]
    fn merging_two_spellings_of_one_field_keeps_one_member() {
        crate::install::installed();
        let mut registry =
            FixRegistry::from_fields([tagged("PartyID", 448, DataType::utf8())]).unwrap();
        let mut member = registry.field(448).unwrap().clone();
        FixFieldMut::new(&mut member)
            .set_field_ref("PartyID")
            .unwrap();
        registry
            .insert(
                StructType::from_fields([member])
                    .map(DataType::from)
                    .unwrap()
                    .required_field("Party"),
            )
            .unwrap();

        let mut other =
            FixRegistry::from_fields([tagged("party_id", 448, DataType::utf8())]).unwrap();
        let mut member = other.field(448).unwrap().clone();
        FixFieldMut::new(&mut member)
            .set_field_ref("party_id")
            .unwrap();
        other
            .insert(
                StructType::from_fields([member])
                    .map(DataType::from)
                    .unwrap()
                    .required_field("Party"),
            )
            .unwrap();

        registry.merge_with(&other).unwrap();
        assert_eq!(
            registry.field_by_name("party_id").unwrap().name(),
            "PartyID"
        );
        let party = registry.field_by_name("Party").unwrap();
        assert_eq!(party.fields().len(), 1, "one member, not two");
        assert_eq!(party.fields()[0].name(), "PartyID");

        registry.merge_with(&other).unwrap();
        registry.merge_with(&other).unwrap();
        assert_eq!(registry.field_by_name("Party").unwrap().fields().len(), 1);
    }

    /// A merge reads the other dictionary in the order it answers in.
    ///
    /// The fold's precedence is its input order, and a caller merging a registry
    /// supplies no order of its own - so the one a registry has is the one it
    /// publishes, tag-major and the tag's holder first. Storage order is neither
    /// that nor stable: a removal swaps the last field into the hole, and a
    /// store round trip writes in `iter` order and loads in file order. Reading
    /// it would make a merge depend on a permutation `PartialEq` deliberately
    /// does not compare, so two dictionaries that compare equal would merge to
    /// two different answers - and one file read twice would not answer twice
    /// the same.
    #[test]
    fn a_merge_reads_the_other_dictionary_in_the_order_it_answers_in() {
        crate::install::installed();
        let described = |name: &str, tag: i32, description: &str| {
            let mut field = tagged(name, tag, DataType::utf8());
            FixFieldMut::new(&mut field)
                .set_description(description)
                .unwrap();
            field
        };
        let symbol = described("Symbol", 55, "from the first");
        let ticker = described("Ticker", 9001, "from the second");

        // Two sources that are equal as registries and stored differently: the
        // second held one more field, and removing it swapped the last into the
        // hole it left.
        let one = FixRegistry::from_fields([symbol.clone(), ticker.clone()]).unwrap();
        let mut two =
            FixRegistry::from_fields([tagged("Scratch", 9999, DataType::utf8()), symbol, ticker])
                .unwrap();
        two.remove(FixId::of(9999, "Scratch").unwrap()).unwrap();
        assert_eq!(one, two, "equal as dictionaries");

        // The target answers to both spellings, so both of the other's fields
        // reach one stored field and the order decides which description lands.
        let target = || {
            let mut field = tagged("Symbol", 55, DataType::utf8());
            FixFieldMut::new(&mut field).set_names(["Ticker"]).unwrap();
            FixRegistry::from_fields([field]).unwrap()
        };
        let mut left = target();
        left.merge_with(&one).unwrap();
        let mut right = target();
        right.merge_with(&two).unwrap();

        assert_eq!(left, right, "equal dictionaries merge alike");
        assert_eq!(
            left.field_by_tag(55).unwrap().description(),
            right.field_by_tag(55).unwrap().description()
        );
    }

    fn dtype(spelling: &str) -> DataType {
        spelling
            .parse()
            .unwrap_or_else(|error| panic!("{spelling}: {error}"))
    }

    /// Five fields a dictionary holds, each beside a source's coarser
    /// statement of it: a price as the text every FIX datatype derives from,
    /// an exact amount as a float, a sequence number at the narrower width,
    /// a zone-less date as a UTC instant, and a flag as text.
    fn precisions() -> [(&'static str, i32, DataType, DataType); 5] {
        [
            ("Price", 44, DataType::Float64, DataType::utf8()),
            ("AvgPx", 6, dtype("decimal128(38, 18)"), DataType::Float64),
            ("MsgSeqNum", 34, DataType::Int64, DataType::Int32),
            (
                "TradeDate",
                75,
                dtype("datetime64(ns)"),
                dtype("datetime64(ns, UTC)"),
            ),
            ("PossDupFlag", 43, DataType::Boolean, DataType::utf8()),
        ]
    }

    #[test]
    fn merge_with_restates_a_coarser_datatype_under_the_held_one() {
        crate::install::installed();
        let mut held = Vec::new();
        let mut declared = Vec::new();
        for (name, tag, stored, incoming) in precisions() {
            let mut field = tagged(name, tag, stored);
            FixFieldMut::new(&mut field).set_sources(["fix44"]).unwrap();
            held.push(field);
            let mut field = tagged(name, tag, incoming);
            FixFieldMut::new(&mut field).set_sources(["venue"]).unwrap();
            FixFieldMut::new(&mut field)
                .set_description(format!("{name} per venue"))
                .unwrap();
            declared.push(field);
        }
        let mut registry = FixRegistry::from_fields(held).unwrap();
        let source = FixRegistry::from_fields(declared.clone()).unwrap();

        // Each folds under the declaration held and is counted among the
        // merged - beside the two standard clock seeds - as restated.
        let merge = registry.merge_with(&source).unwrap();
        assert!(merge.is_clean(), "{:?}", merge.dropped);
        assert_eq!((merge.added, merge.merged, merge.restated), (0, 7, 5));
        assert_eq!(super::scalars(&registry), 5 + super::seeded_fields());
        for (name, tag, stored, _) in precisions() {
            let field = registry.field_by_tag(tag).unwrap();
            assert_eq!(field.name(), name);
            assert_eq!(field.dtype(), &stored, "{name}: the held datatype stays");
            assert_eq!(
                FixField::new(field).sources().collect::<Vec<_>>(),
                ["fix44", "venue"],
                "{name}: the membership is the union"
            );
            assert_eq!(
                field.description(),
                Some(format!("{name} per venue").as_str()),
                "{name}: the rest of the declaration folds as any does"
            );
        }

        // The same source again restates the same five and changes nothing.
        let before = registry.clone();
        let merge = registry.merge_with(&source).unwrap();
        assert_eq!((merge.merged, merge.restated), (7, 5));
        assert_eq!(registry, before);

        // The strict doors state no precision of their own: a differing
        // datatype is refused whole, whichever door it knocks on.
        for field in &declared {
            let error = registry.add_field(field.clone()).unwrap_err();
            assert!(
                matches!(error, Error::InvalidRecord { .. }),
                "add_field {}: {error}",
                field.name()
            );
            let error = registry.update(field.clone()).unwrap_err();
            assert!(
                matches!(error, Error::InvalidRecord { .. }),
                "update {}: {error}",
                field.name()
            );
            assert_eq!(registry, before);
        }
        let error = registry
            .add_fields([tagged("Account", 1, DataType::utf8()), declared[1].clone()])
            .unwrap_err();
        assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
        let message = error.to_string();
        assert!(
            message.contains("decimal128") && message.contains("float64"),
            "{message}"
        );
        assert_eq!(registry, before, "the field before it did not arrive");
        // `insert` replaces an identity rather than folding into it, so what
        // it holds afterwards is the datatype the caller stated, never a
        // restatement of the one it replaced.
        let mut replaced = registry.clone();
        let prior = replaced.insert(declared[0].clone()).unwrap();
        assert_eq!(prior.as_ref().map(Field::dtype), Some(&DataType::Float64));
        assert_eq!(
            replaced.field_by_tag(44).unwrap().dtype(),
            &DataType::utf8()
        );
    }

    #[test]
    fn merge_with_passes_a_contradiction_over_rather_than_restating_it() {
        crate::install::installed();
        // A flag is no count, a time of day is no instant, and two bounded
        // strings of two widths are two layouts.
        let contradictions = [
            ("PossDupFlag", 43, DataType::Boolean, DataType::Int32),
            (
                "MDEntryTime",
                273,
                dtype("time64(ns)"),
                dtype("datetime64(ns)"),
            ),
            (
                "Account",
                1,
                dtype("fixed_ascii(8)"),
                dtype("sized_utf8(32)"),
            ),
        ];
        let mut registry = FixRegistry::from_fields(
            contradictions
                .iter()
                .map(|(name, tag, stored, _)| tagged(name, *tag, stored.clone())),
        )
        .unwrap();
        let source = FixRegistry::from_fields(
            contradictions
                .iter()
                .map(|(name, tag, _, incoming)| tagged(name, *tag, incoming.clone()))
                .chain([tagged("Text", 58, DataType::utf8())]),
        )
        .unwrap();

        let merge = registry.merge_with(&source).unwrap();
        assert_eq!((merge.added, merge.merged, merge.restated), (1, 2, 0));
        assert_eq!(merge.dropped.len(), 3, "{:?}", merge.dropped);
        for (name, tag, stored, incoming) in &contradictions {
            let drop = merge
                .dropped
                .iter()
                .find(|drop| drop.incoming.name() == *name)
                .unwrap_or_else(|| panic!("{name} passed over"));
            assert_eq!(drop.incoming.dtype(), incoming, "{name}");
            assert!(
                drop.reason.contains(&stored.to_string())
                    && drop.reason.contains(&incoming.to_string()),
                "{name}: {drop}"
            );
            assert_eq!(registry.field_by_tag(*tag).unwrap().dtype(), stored);
        }
        assert_eq!(registry.field_by_tag(58).unwrap().name(), "Text");
    }

    #[test]
    fn a_field_named_by_its_bare_tag_merges_into_the_holder_of_that_tag() {
        crate::install::installed();
        let mut registry = FixRegistry::from_fields([maturity()]).unwrap();

        // A CBlock declaring the tag with no name of its own names it after
        // the digits: a placeholder, which folds into whatever holds the tag.
        let mut unnamed = tagged("541", 541, DataType::utf8());
        FixFieldMut::new(&mut unnamed)
            .set_sources(["cblock"])
            .unwrap();
        FixFieldMut::new(&mut unnamed)
            .set_description("a tag the file never named")
            .unwrap();
        assert_eq!(registry.add_fields([unnamed.clone()]).unwrap(), (0, 1));
        assert_eq!(super::scalars(&registry), 1 + super::seeded_fields());
        let stored = registry.field_by_tag(541).unwrap();
        assert_eq!(stored.name(), "maturitydate");
        assert_eq!(stored.display(), Some("MaturityDate"));
        assert_eq!(
            FixField::new(stored).id().unwrap(),
            Some(FixId::of(541, "maturitydate").unwrap())
        );
        assert_eq!(
            FixField::new(stored).sources().collect::<Vec<_>>(),
            ["cblock", "fix44"]
        );
        assert_eq!(stored.description(), Some("a tag the file never named"));
        assert!(
            FixField::new(stored).names().next().is_none(),
            "the digits are no name to answer to"
        );
        let before = registry.clone();
        assert_eq!(registry.add_fields([unnamed]).unwrap(), (0, 1));
        assert_eq!(registry, before);
    }

    #[test]
    fn a_source_member_reading_a_bare_tag_reads_the_held_field_after_a_merge() {
        crate::install::installed();
        // The source's own members reading the digits read the held field:
        // one member, under the held name, and nothing passed over - what a
        // CBlock beside the standard used to refuse with "expected a
        // reference to fields "541" stored for it, got a reference to fields
        // "maturitydate"".
        let mut registry = FixRegistry::from_fields([maturity()]).unwrap();
        let mut member = registry.field(541).unwrap().clone();
        FixFieldMut::new(&mut member)
            .set_field_ref("maturitydate")
            .unwrap();
        registry
            .insert(
                StructType::from_fields([member])
                    .map(DataType::from)
                    .unwrap()
                    .required_field("Instrument"),
            )
            .unwrap();
        let mut source = FixRegistry::from_fields([tagged("541", 541, DataType::utf8())]).unwrap();
        let mut member = source.field(541).unwrap().clone();
        FixFieldMut::new(&mut member).set_field_ref("541").unwrap();
        source
            .insert(
                StructType::from_fields([member])
                    .map(DataType::from)
                    .unwrap()
                    .required_field("Instrument"),
            )
            .unwrap();
        let merge = registry.merge_with(&source).unwrap();
        assert!(merge.is_clean(), "{:?}", merge.dropped);
        let instrument = registry.field_by_name("Instrument").unwrap();
        assert_eq!(names(instrument), ["maturitydate"]);
        assert_eq!(
            FixField::new(&instrument.fields()[0]).field_ref(),
            Some("maturitydate")
        );
        assert_eq!(
            FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
            registry
        );
    }

    #[test]
    fn a_member_pairs_with_the_held_member_reading_its_field_before_the_one_of_its_name() {
        crate::install::installed();
        // A dialect spelling `Urgency` over 61 and 9252 names each by its
        // decimal, and its message carries the spelling on both members:
        // `urgency` reading 61, `urgency2` reading 9252. Another names 9252
        // `Urgency` and its message reads it under that name. A member is the
        // field it reads before the name it carries, so the one reading 9252
        // is the held one reading 9252 whatever either is called, and one
        // reading 61 under a name a held member reads 9252 by stands beside
        // it: nothing is passed over, and the message reads both tags
        // whichever dictionary folds into the other.
        fn message(members: Vec<Field>) -> Field {
            let mut message = StructType::from_fields(members)
                .map(DataType::from)
                .unwrap()
                .required_field("NewOrderSingle");
            FixFieldMut::new(&mut message).set_msgtype("D").unwrap();
            message
        }
        fn contended() -> FixRegistry {
            let mut first = tagged("61", 61, DataType::utf8());
            FixFieldMut::new(&mut first).set_tags(&[9252]).unwrap();
            let mut second = tagged("9252", 9252, DataType::utf8());
            FixFieldMut::new(&mut second).set_tags(&[61]).unwrap();
            let mut registry = FixRegistry::from_fields([first, second]).unwrap();
            let mut urgency = registry.field(61).unwrap().clone();
            urgency.set_name("urgency");
            FixFieldMut::new(&mut urgency).set_field_ref("61").unwrap();
            let mut urgency2 = registry.field(9252).unwrap().clone();
            urgency2.set_name("urgency2");
            FixFieldMut::new(&mut urgency2)
                .set_field_ref("9252")
                .unwrap();
            registry.insert(message(vec![urgency, urgency2])).unwrap();
            registry
        }
        fn named() -> FixRegistry {
            let mut registry =
                FixRegistry::from_fields([tagged("Urgency", 9252, DataType::utf8())]).unwrap();
            let mut member = registry.field(9252).unwrap().clone();
            FixFieldMut::new(&mut member)
                .set_field_ref("Urgency")
                .unwrap();
            registry.insert(message(vec![member])).unwrap();
            registry
        }
        for (mut held, other) in [(contended(), named()), (named(), contended())] {
            let merge = held.merge_with(&other).unwrap();
            assert!(merge.is_clean(), "{:?}", merge.dropped);
            let message = held.msgtype("D").unwrap().as_field();
            let mut read: Vec<(Option<i32>, String)> = message
                .fields()
                .iter()
                .map(|member| {
                    (
                        FixField::new(member).tag().unwrap(),
                        FixField::new(member)
                            .field_ref()
                            .unwrap()
                            .to_ascii_lowercase(),
                    )
                })
                .collect();
            read.sort_unstable();
            assert_eq!(
                read,
                [
                    (Some(61), "61".to_owned()),
                    (Some(9252), "urgency".to_owned())
                ]
            );
            assert_eq!(
                FixRegistry::from_json(&held.into_json().unwrap()).unwrap(),
                held
            );
        }
    }

    /// A dictionary whose tag 541 is named by nothing but its digits, and a
    /// component reading it under them.
    fn unnamed_with_reader() -> FixRegistry {
        let mut unnamed = tagged("541", 541, DataType::utf8());
        FixFieldMut::new(&mut unnamed)
            .set_sources(["cblock"])
            .unwrap();
        let mut registry = FixRegistry::from_fields([unnamed]).unwrap();
        let mut member = registry.field(541).unwrap().clone();
        FixFieldMut::new(&mut member).set_field_ref("541").unwrap();
        registry
            .insert(
                StructType::from_fields([member])
                    .map(DataType::from)
                    .unwrap()
                    .required_field("Instrument"),
            )
            .unwrap();
        registry
    }

    /// Tag 541 as the standard names it.
    fn maturity() -> Field {
        let mut named = tagged("maturitydate", 541, DataType::utf8());
        named.set_display("MaturityDate").unwrap();
        FixFieldMut::new(&mut named).set_sources(["fix44"]).unwrap();
        named
    }

    /// What a named arrival leaves on a tag an unnamed field held: one field,
    /// under the arrival's name and display, the membership the union, and
    /// the component reading it under that name.
    fn assert_named_holder(registry: &FixRegistry) {
        assert_eq!(super::scalars(registry), 1 + super::seeded_fields());
        let stored = registry.field_by_tag(541).unwrap();
        assert_eq!(stored.name(), "maturitydate");
        assert_eq!(stored.display(), Some("MaturityDate"));
        assert_eq!(
            FixField::new(stored).id().unwrap(),
            Some(FixId::of(541, "maturitydate").unwrap())
        );
        assert!(
            registry
                .get_field_by_id(FixId::of(541, "541").unwrap())
                .is_none(),
            "the placeholder identity is gone"
        );
        assert_eq!(
            FixField::new(stored).sources().collect::<Vec<_>>(),
            ["cblock", "fix44"]
        );
        let instrument = registry.field_by_name("Instrument").unwrap();
        assert_eq!(names(instrument), ["maturitydate"]);
        let member = registry
            .field_by_path(&fpath("Instrument.maturitydate"))
            .unwrap();
        assert_eq!(FixField::new(member).tag().unwrap(), Some(541));
        assert_eq!(FixField::new(member).field_ref(), Some("maturitydate"));
        assert_eq!(
            &FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
            registry
        );
    }

    #[test]
    fn a_named_arrival_on_a_tag_an_unnamed_field_holds_renames_the_holder_through_add_fields() {
        crate::install::installed();
        let mut registry = unnamed_with_reader();
        assert_eq!(registry.add_fields([maturity()]).unwrap(), (0, 1));
        assert_named_holder(&registry);
        let before = registry.clone();
        assert_eq!(registry.add_fields([maturity()]).unwrap(), (0, 1));
        assert_eq!(registry, before);
    }

    #[test]
    fn a_named_arrival_on_a_tag_an_unnamed_field_holds_renames_the_holder_through_add_field() {
        crate::install::installed();
        let mut registry = unnamed_with_reader();
        assert!(!registry.add_field(maturity()).unwrap());
        assert_named_holder(&registry);
    }

    #[test]
    fn a_named_arrival_on_a_tag_an_unnamed_field_holds_renames_the_holder_through_merge_with() {
        crate::install::installed();
        let mut registry = unnamed_with_reader();
        let merge = registry
            .merge_with(&FixRegistry::from_fields([maturity()]).unwrap())
            .unwrap();
        assert!(merge.is_clean(), "{:?}", merge.dropped);
        assert_eq!((merge.added, merge.merged), (0, 3));
        assert_named_holder(&registry);
    }

    #[test]
    fn an_unnamed_arrival_on_a_tag_nobody_holds_arrives_as_it_is() {
        crate::install::installed();
        let mut registry = FixRegistry::from_fields([maturity()]).unwrap();
        let unnamed = tagged("9999", 9999, DataType::utf8());
        assert_eq!(registry.add_fields([unnamed.clone()]).unwrap(), (1, 0));
        assert_eq!(registry.field_by_tag(9999).unwrap(), &unnamed);
        assert_eq!(registry.field_by_tag(541).unwrap(), &maturity());
    }

    #[test]
    fn the_sources_catalog_takes_an_entry_once_and_keeps_what_a_field_names() {
        crate::install::installed();
        use yggdryl_fix::FixSource;
        let mut registry = FixRegistry::new();
        assert_eq!(registry.sources().len(), 0);
        assert!(registry.add_source(FixSource::new("Venue").unwrap()));
        assert!(
            !registry.add_source(FixSource::new("VENUE").unwrap().with_file("Venue.cfb")),
            "a held id keeps its entry"
        );
        assert_eq!(
            registry.get_source("venue").unwrap().file(),
            Some("Venue.cfb"),
            "and takes the file it lacked"
        );
        assert!(!registry.add_source(FixSource::new("venue").unwrap().with_file("other.cfb")));
        assert_eq!(
            registry.get_source("Venue").unwrap().file(),
            Some("Venue.cfb"),
            "a stated file is kept"
        );
        assert!(registry.add_source(FixSource::new("desk").unwrap()));
        assert_eq!(
            registry.sources().map(FixSource::id).collect::<Vec<_>>(),
            ["desk", "venue"]
        );
        assert!(registry.get_source("nobody").is_none());
        // The fold a field's list is deduplicated under reaches the entry too.
        assert_eq!(registry.get_source("VE_NUE").unwrap().id(), "venue");

        // The catalog and the fields are two statements: an entry nothing
        // names stands, and an id no entry holds is the field's alone.
        let mut field = tagged("VenueTrade", 5001, DataType::utf8());
        FixFieldMut::new(&mut field)
            .set_sources(["venue", "ghost"])
            .unwrap();
        registry.insert(field).unwrap();
        assert_eq!(registry.dialects(), ["ghost", "venue"]);
        assert!(registry.get_source("ghost").is_none());

        // Removal is refused while a field names the id, by that field, and
        // removes nothing; an entry nothing names comes back; absence is
        // nothing.
        let error = registry.remove_source("VENUE").unwrap_err();
        assert!(matches!(error, Error::Conflict { .. }), "{error}");
        let text = error.to_string();
        assert!(
            text.contains("VenueTrade") && text.contains("venue"),
            "{text}"
        );
        assert_eq!(
            registry.get_source("venue").unwrap().file(),
            Some("Venue.cfb")
        );
        assert_eq!(
            registry.remove_source("desk").unwrap().unwrap().id(),
            "desk"
        );
        assert_eq!(registry.remove_source("desk").unwrap(), None);
        assert_eq!(registry.sources().len(), 1);
    }

    /// A held entry takes the plugin role it lacked - `UKNW` is none
    /// stated - and keeps the one it states: two files of one dialect
    /// disagreeing on the role keep the first, and a file stating none
    /// says nothing against it.
    #[test]
    fn the_catalog_takes_a_plugin_role_it_lacked_and_keeps_a_stated_one() {
        crate::install::installed();
        use yggdryl_fix::FixSource;
        use yggdryl_market::Side;
        let mut registry = FixRegistry::new();
        assert!(registry.add_source(FixSource::new("venue").unwrap()));
        assert_eq!(
            registry.get_source("venue").unwrap().pluginside(),
            Side::Unknown
        );
        assert!(!registry.add_source(FixSource::new("VENUE").unwrap().with_pluginside(Side::Sell)));
        assert_eq!(
            registry.get_source("venue").unwrap().pluginside(),
            Side::Sell,
            "a role the entry lacked is taken"
        );
        assert!(!registry.add_source(FixSource::new("venue").unwrap().with_pluginside(Side::Buy)));
        assert_eq!(
            registry.get_source("venue").unwrap().pluginside(),
            Side::Sell,
            "a stated role is kept over a disagreeing one"
        );
        assert!(!registry.add_source(FixSource::new("venue").unwrap()));
        assert_eq!(
            registry.get_source("venue").unwrap().pluginside(),
            Side::Sell,
            "none stated says nothing"
        );
        // The fold of two registries unions the catalogs under the same
        // rule, so the role rides a merge.
        let mut other = FixRegistry::new();
        other.add_source(FixSource::new("desk").unwrap().with_pluginside(Side::Buy));
        other.add_source(FixSource::new("venue").unwrap());
        registry.merge_with(&other).unwrap();
        assert_eq!(
            registry
                .sources()
                .map(|source| (source.id(), source.pluginside()))
                .collect::<Vec<_>>(),
            [("desk", Side::Buy), ("venue", Side::Sell)]
        );
    }

    #[test]
    fn the_catalog_holds_one_entry_per_id_under_the_fold_a_field_reads_by() {
        crate::install::installed();
        use yggdryl_fix::FixSource;
        let mut registry = FixRegistry::new();
        assert!(registry.add_source(FixSource::new("venue").unwrap()));
        // A spelling the fold reads as a held id is that entry: it arrives
        // nowhere, keeps the held spelling, and hands over the file the
        // held entry lacked.
        assert!(!registry.add_source(FixSource::new("VE_NUE").unwrap().with_file("x.cfb")));
        assert!(!registry.add_source(FixSource::new("ve-nue").unwrap().with_file("y.cfb")));
        assert_eq!(registry.sources().len(), 1);
        assert_eq!(
            registry
                .sources()
                .map(|source| (source.id(), source.file()))
                .collect::<Vec<_>>(),
            [("venue", Some("x.cfb"))]
        );
        assert_eq!(registry.get_source("ve nue").unwrap().id(), "venue");
        // A field stating the other spelling names that entry, so the
        // catalog and the field agree on what one id is: the removal is
        // refused by the field, and the entry stands.
        let mut field = tagged("VenueTrade", 5001, DataType::utf8());
        FixFieldMut::new(&mut field)
            .set_sources(["ve_nue"])
            .unwrap();
        registry.insert(field).unwrap();
        assert!(FixField::new(registry.field_by_tag(5001).unwrap()).has_source("venue"));
        let error = registry.remove_source("venue").unwrap_err();
        assert!(matches!(error, Error::Conflict { .. }), "{error}");
        assert_eq!(registry.sources().len(), 1);
    }

    #[test]
    fn a_sources_text_the_setter_never_writes_is_refused_where_a_definition_enters() {
        crate::install::installed();
        // The read walks a hand edit as nothing; the doors a definition
        // enters a registry through refuse it instead, naming the key and
        // what the setter writes.
        for (stored, expected) in [
            ("venue", "a JSON array of source ids"),
            (r#"["Venue"]"#, "each source id ASCII lowercase"),
            (r#"["b","a"]"#, "the source ids sorted"),
            (r#"["venue","venue"]"#, "each source once"),
            (r#"["ve_nue","venue"]"#, "each source once"),
        ] {
            let mut field = tagged("VenueTrade", 5001, DataType::utf8());
            field.insert_metadata("FIX:sources", stored).unwrap();
            for error in [
                FixRegistry::from_fields([field.clone()]).unwrap_err(),
                FixRegistry::new().insert(field.clone()).unwrap_err(),
            ] {
                assert!(
                    matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "FIX:sources"),
                    "{stored}: {error}"
                );
                assert!(error.to_string().contains(expected), "{stored}: {error}");
            }
            // A named definition is held to the same text.
            let mut registry =
                FixRegistry::from_fields([tagged("PartyID", 448, DataType::utf8())]).unwrap();
            let mut member = registry.field_by_tag(448).unwrap().clone();
            FixFieldMut::new(&mut member)
                .set_field_ref("PartyID")
                .unwrap();
            let mut party =
                DataType::from(StructType::from_fields([member]).unwrap()).required_field("Party");
            party.insert_metadata("FIX:sources", stored).unwrap();
            let error = registry
                .create_definition(FixCategory::Components, party)
                .unwrap_err();
            assert!(
                matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "FIX:sources"),
                "{stored}: {error}"
            );
            assert!(
                registry
                    .definition(FixCategory::Components, "Party")
                    .is_err()
            );
        }
        // The retired membership key is refused by its own name rather than
        // loaded as inert text with the membership gone.
        let mut field = tagged("VenueTrade", 5001, DataType::utf8());
        field.insert_metadata("FIX:branches", "venue").unwrap();
        let error = FixRegistry::from_fields([field]).unwrap_err();
        assert!(
            matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "FIX:branches"),
            "{error}"
        );
        let text = error.to_string();
        assert!(
            text.contains("retired")
                && text.contains("FIX:sources")
                && text.contains("sources.json"),
            "{text}"
        );
    }

    #[test]
    fn merging_unions_the_sources_catalogs_as_it_unions_the_ids_on_a_field() {
        crate::install::installed();
        use yggdryl_fix::FixSource;
        let mut held = FixRegistry::new();
        held.add_source(FixSource::new("cme").unwrap());
        held.add_source(FixSource::new("venue").unwrap().with_file("held.cfb"));
        let mut other = FixRegistry::new();
        other.add_source(FixSource::new("venue").unwrap().with_file("other.cfb"));
        other.add_source(FixSource::new("blp").unwrap().with_file("blp.cfb"));
        let mut symbol = tagged("Symbol", 55, DataType::utf8());
        FixFieldMut::new(&mut symbol).set_sources(["blp"]).unwrap();
        other.insert(symbol).unwrap();
        held.merge_with(&other).unwrap();
        assert_eq!(
            held.sources()
                .map(|source| (source.id(), source.file()))
                .collect::<Vec<_>>(),
            [
                ("blp", Some("blp.cfb")),
                ("cme", None),
                ("venue", Some("held.cfb"))
            ],
            "a held entry keeps its file; what only the other holds arrives"
        );
        assert_eq!(
            FixField::new(held.field_by_tag(55).unwrap())
                .sources()
                .collect::<Vec<_>>(),
            ["blp"]
        );
        // Equality and the hash read the catalog: two dictionaries whose
        // fields agree and whose catalogs do not are two dictionaries.
        let mut twin = held.clone();
        assert_eq!(twin, held);
        assert_eq!(twin.stable_hash(), held.stable_hash());
        twin.add_source(FixSource::new("desk").unwrap());
        assert_ne!(twin, held);
        assert_ne!(twin.stable_hash(), held.stable_hash());
    }
}

/// Every name lookup reads the four word pairs - `offer`/`ask`,
/// `size`/`qty`, `bid`/`demand`, `px`/`price` - either way, wherever one
/// stands in the folded name, after the names a field holds exactly; a
/// spelling the words make reaching two fields reaches none.
mod word_aliases {
    use yggdryl::{DataType, Field};
    use yggdryl_fix::{FixField, FixFieldMut, FixRegistry};

    fn tagged(name: &str, tag: i32) -> Field {
        let mut field = DataType::utf8().nullable_field(name);
        FixFieldMut::new(&mut field).set_tag(tag).unwrap();
        field
    }

    fn tag_of(registry: &FixRegistry, name: &str) -> Option<i32> {
        registry
            .get_field_by_name(name)
            .and_then(|field| FixField::new(field).tag().ok().flatten())
    }

    #[test]
    fn every_pair_reads_either_way_in_any_spelling() {
        crate::install::installed();
        let registry = FixRegistry::from_fields([
            tagged("BidPx", 132),
            tagged("OfferPx", 133),
            tagged("BidSize", 134),
            tagged("OfferSize", 135),
        ])
        .unwrap();
        for (spelled, tag) in [
            ("AskPx", 133),
            ("ask_px", 133),
            ("ASKPX", 133),
            ("AskPrice", 133),
            ("OfferPrice", 133),
            ("AskSize", 135),
            ("AskQty", 135),
            ("BidPrice", 132),
            ("DemandPx", 132),
            ("DEMAND-QTY", 134),
            ("bidqty", 134),
        ] {
            assert_eq!(tag_of(&registry, spelled), Some(tag), "{spelled}");
        }
        // A spelling no word reaches a field through stays unresolved.
        assert_eq!(tag_of(&registry, "AskCurrency"), None);
        assert_eq!(tag_of(&registry, "Quantity"), None);
    }

    #[test]
    fn an_exact_name_wins_and_an_ambiguous_alias_reaches_nothing() {
        crate::install::installed();
        // `AskPx` is a field of its own here: the exact name answers it.
        let registry =
            FixRegistry::from_fields([tagged("OfferPx", 133), tagged("AskPx", 9001)]).unwrap();
        assert_eq!(tag_of(&registry, "AskPx"), Some(9001));
        assert_eq!(tag_of(&registry, "OfferPx"), Some(133));
        // `AskPrice` reads as `OfferPrice` - none - `AskPx` and `OfferPx`: two
        // fields, so it reaches neither.
        assert_eq!(tag_of(&registry, "AskPrice"), None);
    }
}

/// The parents of an identifier type, nearest first, and the base a parent
/// type belongs to: the list a field states under `FIX:parents`, else the one
/// the names say - `orderid`'s are `parentorderid` then `origorderid`.
mod parents {
    use yggdryl::{DataType, Field};
    use yggdryl_fix::{FixField, FixFieldMut, FixRegistry};
    use yggdryl_market::IdType;

    fn word(text: &str) -> IdType {
        text.parse().expect("a type")
    }

    fn tagged(name: &str, tag: i32) -> Field {
        let mut field = DataType::utf8().nullable_field(name);
        FixFieldMut::new(&mut field).set_tag(tag).unwrap();
        field
    }

    #[test]
    fn the_committed_dictionary_states_a_client_orders_previous_identifier_as_its_parent() {
        crate::install::installed();
        let registry = crate::committed_registry();
        let clordid = registry.field_by_tag(11).expect("ClOrdID(11)");
        assert_eq!(
            FixField::new(clordid).parents().collect::<Vec<_>>(),
            ["origclordid"]
        );
        let stated = registry
            .parent_sources()
            .iter()
            .find(|(base, _)| *base == IdType::ClOrdId)
            .expect("the list ClOrdID(11) states");
        assert_eq!(stated.1.as_ref(), [IdType::OrigClOrdId]);
        assert_eq!(
            registry.parents_of(&IdType::ClOrdId).as_ref(),
            [IdType::OrigClOrdId]
        );
        assert_eq!(
            registry.parent_of(&IdType::OrigClOrdId),
            Some((IdType::ClOrdId, 0))
        );
        assert_eq!(registry.parent_of(&IdType::ClOrdId), None);
    }

    #[test]
    fn every_list_the_dictionary_states_reads_back_through_parent_of() {
        crate::install::installed();
        let registry = crate::committed_registry();
        let sources = registry.parent_sources();
        assert!(!sources.is_empty());
        for (base, listed) in sources {
            assert_eq!(
                registry.parents_of(base).as_ref(),
                listed.as_ref(),
                "{base}"
            );
            for (at, parent) in listed.iter().enumerate() {
                assert_eq!(
                    registry.parent_of(parent),
                    Some((base.clone(), at)),
                    "{parent}"
                );
                // A parent has none: parentage never nests.
                assert!(registry.parents_of(parent).is_empty(), "{parent}");
            }
        }
    }

    #[test]
    fn a_base_stating_no_list_has_the_one_its_name_says() {
        crate::install::installed();
        let registry = crate::committed_registry();
        assert_eq!(
            registry.parents_of(&IdType::OrderId).as_ref(),
            [word("parentorderid"), word("origorderid")]
        );
        assert_eq!(
            registry.parent_of(&word("parentorderid")),
            Some((IdType::OrderId, 0))
        );
        assert_eq!(
            registry.parent_of(&word("origorderid")),
            Some((IdType::OrderId, 1))
        );
        // `clordid`'s one parent is spelled `origclordid` alone: a bridge's
        // `parentclordid`, the hierarchy parent child orders share, is a
        // word of its own and a parent of nothing. A word spelled `origin` or
        // `original` before an identifier names no parent at all, and
        // neither does a parent spelling of a type that names no chain.
        assert!(matches!(word("parentclordid"), IdType::Other(_)));
        assert_eq!(registry.parent_of(&word("parentclordid")), None);
        assert_eq!(registry.parent_of(&word("parentexecid")), None);
        assert!(registry.parents_of(&IdType::ExecId).is_empty());
        assert!(registry.parents_of(&IdType::Isin).is_empty());
        assert_eq!(registry.parent_of(&word("originorderid")), None);
        assert_eq!(registry.parent_of(&word("originalorderid")), None);
        assert_eq!(registry.parent_of(&word("account")), None);
    }

    #[test]
    fn a_trade_report_takes_its_parent_from_the_vocabulary_and_no_field_states_it() {
        crate::install::installed();
        // `TradeReportRefID(572)` is `TradeReportID(571)`'s one parent by the
        // type it is, not by a name opening as a parent's: loaded at once,
        // field by field in either order, or committed, no field states a
        // list for it and every reading answers the vocabulary's.
        let bulk = FixRegistry::from_fields([
            tagged("TradeReportID", 571),
            tagged("TradeReportRefID", 572),
        ])
        .unwrap();
        let mut forward = FixRegistry::from_fields([tagged("TradeReportID", 571)]).unwrap();
        assert!(forward.add_field(tagged("TradeReportRefID", 572)).unwrap());
        let mut backward = FixRegistry::from_fields([tagged("TradeReportRefID", 572)]).unwrap();
        assert!(backward.add_field(tagged("TradeReportID", 571)).unwrap());
        let committed = crate::committed_registry();
        for registry in [&bulk, &forward, &backward, committed.as_ref()] {
            assert_eq!(
                FixField::new(registry.field_by_tag(571).unwrap())
                    .parents()
                    .count(),
                0
            );
            assert_eq!(
                registry.parents_of(&IdType::TradeReportId).as_ref(),
                [IdType::TradeReportRefId]
            );
            assert_eq!(
                registry.parent_of(&IdType::TradeReportRefId),
                Some((IdType::TradeReportId, 0))
            );
            assert_eq!(registry.parent_of(&word("parenttradereportid")), None);
            assert_eq!(registry.parent_of(&word("origtradereportid")), None);
        }
    }

    #[test]
    fn a_parent_field_answers_to_its_own_name_alone() {
        crate::install::installed();
        // No other spelling is written as fields arrive: `OrigClOrdID(41)`
        // is named `origclordid` alone, so a bridge's `ParentClOrdID` reaches
        // no field...
        let mut registry =
            FixRegistry::from_fields([tagged("ClOrdID", 11), tagged("OrigClOrdID", 41)]).unwrap();
        assert!(
            FixField::new(registry.field_by_tag(41).unwrap())
                .names()
                .next()
                .is_none()
        );
        assert!(registry.field_by_name("ParentClOrdID").is_err());
        // ...and arrives as a field of its own, which names no parent.
        assert!(registry.add_field(tagged("ParentClOrdID", 9003)).unwrap());
        assert_eq!(
            FixField::new(registry.field_by_name("ParentClOrdID").unwrap())
                .tag()
                .unwrap(),
            Some(9003)
        );
        assert_eq!(
            FixField::new(registry.field_by_name("OrigClOrdID").unwrap())
                .tag()
                .unwrap(),
            Some(41)
        );
        assert_eq!(
            FixField::new(registry.field_by_tag(11).unwrap())
                .parents()
                .collect::<Vec<_>>(),
            ["origclordid"]
        );
        // A one-parent base of the name rule: `OrigOrderID` alone answers to
        // its own name, and `ParentOrderID` arriving is the second parent.
        let mut venue =
            FixRegistry::from_fields([tagged("OrderID", 37), tagged("OrigOrderID", 9001)]).unwrap();
        assert!(venue.field_by_name("ParentOrderID").is_err());
        assert!(venue.add_field(tagged("ParentOrderID", 9002)).unwrap());
        assert_eq!(
            FixField::new(venue.field_by_tag(37).unwrap())
                .parents()
                .collect::<Vec<_>>(),
            ["parentorderid", "origorderid"]
        );
        assert!(
            FixField::new(venue.field_by_tag(9001).unwrap())
                .names()
                .next()
                .is_none()
        );
    }

    #[test]
    fn a_registry_states_its_own_list_and_it_wins_over_the_name() {
        crate::install::installed();
        let mut order = tagged("OrderID", 37);
        FixFieldMut::new(&mut order)
            .set_parents(["parentorderid", "grandparentorderid", "origorderid"])
            .unwrap();
        let registry = FixRegistry::from_fields([order]).unwrap();
        assert_eq!(
            registry.parents_of(&IdType::OrderId).as_ref(),
            [
                word("parentorderid"),
                word("grandparentorderid"),
                word("origorderid")
            ]
        );
        assert_eq!(
            registry.parent_of(&word("parentorderid")),
            Some((IdType::OrderId, 0))
        );
        assert_eq!(
            registry.parent_of(&word("grandparentorderid")),
            Some((IdType::OrderId, 1))
        );
        // The list places `origorderid` third where its name says second.
        assert_eq!(
            registry.parent_of(&word("origorderid")),
            Some((IdType::OrderId, 2))
        );
        // Another base still follows its name.
        assert_eq!(
            registry.parents_of(&IdType::TradeId).as_ref(),
            [word("parenttradeid"), word("origtradeid")]
        );
        assert_eq!(
            registry.parent_of(&word("origtradeid")),
            Some((IdType::TradeId, 1))
        );
    }

    #[test]
    fn a_base_stating_a_list_refuses_the_parents_the_name_would_have_added() {
        crate::install::installed();
        let mut order = tagged("OrderID", 37);
        FixFieldMut::new(&mut order)
            .set_parents(["parentorderid"])
            .unwrap();
        let registry = FixRegistry::from_fields([order]).unwrap();
        assert_eq!(
            registry.parent_of(&word("parentorderid")),
            Some((IdType::OrderId, 0))
        );
        assert_eq!(
            registry.parent_of(&word("origorderid")),
            None,
            "the stated list is the whole list"
        );
    }

    #[test]
    fn a_field_added_by_a_parents_name_needs_no_metadata() {
        crate::install::installed();
        let registry =
            FixRegistry::from_fields([tagged("OrderID", 37), tagged("ParentOrderID", 9001)])
                .unwrap();
        assert_eq!(
            registry.parent_of(&word("parentorderid")),
            Some((IdType::OrderId, 0))
        );
        assert_eq!(
            registry.parents_of(&IdType::OrderId).first(),
            Some(&word("parentorderid"))
        );
    }

    #[test]
    fn adding_a_field_forgets_what_the_lists_were_read_from() {
        crate::install::installed();
        let mut registry = FixRegistry::from_fields([tagged("OrderID", 37)]).unwrap();
        // Read once: `tradeid` states no list yet, so its name's answers.
        assert_eq!(
            registry.parents_of(&IdType::TradeId).as_ref(),
            [word("parenttradeid"), word("origtradeid")]
        );
        let mut trade = tagged("TradeID", 1003);
        FixFieldMut::new(&mut trade)
            .set_parents(["parenttradeid"])
            .unwrap();
        assert!(registry.add_field(trade).unwrap());
        assert_eq!(
            registry.parents_of(&IdType::TradeId).as_ref(),
            [word("parenttradeid")]
        );
        assert_eq!(
            registry.parent_of(&word("origtradeid")),
            None,
            "the newly stated list is the whole list"
        );
    }
}
