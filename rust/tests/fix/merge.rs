//! The lenient registry verbs: `add_field`, `add_fields`, `add_definition`
//! and `merge_with`, which fold into what is stored where the strict verbs
//! replace or refuse.

use super::path as fpath;

use yggdryl::{DataType, Error, Field, FixCategory, FixId, FixRegistry};

fn tagged(name: &str, tag: i32, dtype: DataType) -> Field {
    let mut field = dtype.nullable_field(name);
    field.as_fix_mut().set_tag(tag).unwrap();
    field
}

/// A catalog whose `Party` component holds `PartyID` and then `members`, a
/// `Parties` group restating the component, and a `NewOrderSingle` message
/// restating the group: every kind of reference between two definitions.
fn catalog_with(members: impl IntoIterator<Item = Field>) -> FixRegistry {
    let mut registry = FixRegistry::from_fields([
        tagged("NoPartyIDs", 453, DataType::Int32),
        tagged("PartyID", 448, DataType::Utf8),
    ])
    .unwrap();
    let mut partyid = registry.field(448).unwrap().clone();
    partyid.as_fix_mut().set_field_ref("PartyID").unwrap();
    let component = DataType::from_fields(std::iter::once(partyid).chain(members))
        .unwrap()
        .required_field("Party");
    registry
        .create_definition(FixCategory::Components, component)
        .unwrap();
    let component = registry
        .definition(FixCategory::Components, "Party")
        .unwrap()
        .clone();
    let mut group = DataType::list(component).nullable_field("Parties");
    group.as_fix_mut().set_counter(453).unwrap();
    group.as_fix_mut().set_component("Party").unwrap();
    registry
        .create_definition(FixCategory::Groups, group)
        .unwrap();
    let mut group = registry
        .definition(FixCategory::Groups, "Parties")
        .unwrap()
        .clone();
    group.as_fix_mut().set_group("Parties").unwrap();
    let mut counter = registry.field(453).unwrap().clone();
    counter.as_fix_mut().set_field_ref("NoPartyIDs").unwrap();
    let mut message = DataType::from_fields([counter, group])
        .unwrap()
        .required_field("NewOrderSingle");
    message.as_fix_mut().set_msgtype("D").unwrap();
    registry
        .create_definition(FixCategory::Messages, message)
        .unwrap();
    registry
}

fn catalog() -> FixRegistry {
    catalog_with([])
}

/// The names of a Struct field's direct children, in order.
fn names(field: &Field) -> Vec<&str> {
    field.fields().iter().map(Field::name).collect()
}

/// The occurrence a group's list holds.
fn occurrence(group: &Field) -> &Field {
    let (DataType::List(item) | DataType::LargeList(item)) = group.dtype() else {
        panic!("a group list")
    };
    item
}

#[test]
fn a_field_whose_name_folds_to_a_stored_name_merges_into_that_field() {
    let mut symbol = tagged("Symbol", 55, DataType::Utf8);
    symbol.as_fix_mut().set_tags(&[65]).unwrap();
    symbol.as_fix_mut().set_aliases(["Ticker"]).unwrap();
    symbol.as_fix_mut().set_description("stored").unwrap();
    let mut registry =
        FixRegistry::from_fields([symbol, tagged("Price", 44, DataType::Float64)]).unwrap();

    // Another spelling of tag 55's field: its own tag, its own alternates and
    // aliases, one of which is the stored alias in another case.
    let mut incoming = tagged("symbol", 9001, DataType::Utf8);
    incoming.as_fix_mut().set_tags(&[66]).unwrap();
    incoming
        .as_fix_mut()
        .set_aliases(["Sym", "TICKER"])
        .unwrap();
    incoming.as_fix_mut().set_description("incoming").unwrap();
    assert!(!registry.add_field(incoming.clone()).unwrap());
    assert_eq!(registry.len(), 2 + super::crated());

    // The stored field keeps its identity and spelling; the union is stored
    // order first, then what only the incoming field stated, then its tag.
    let stored = registry.field_by_tag(55).unwrap();
    assert_eq!(stored.name(), "Symbol");
    assert_eq!(
        stored.as_fix().id().unwrap(),
        Some(FixId::of(55, "Symbol").unwrap())
    );
    assert_eq!(stored.as_fix().tags().unwrap(), [65, 66, 9001]);
    assert_eq!(
        stored.as_fix().aliases().collect::<Vec<_>>(),
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
    let mut symbol = tagged("Symbol", 55, DataType::Utf8);
    symbol.as_fix_mut().set_aliases(["Ticker"]).unwrap();
    let mut registry = FixRegistry::from_fields([symbol]).unwrap();

    assert!(
        !registry
            .add_field(tagged("ticker", 9001, DataType::Utf8))
            .unwrap()
    );
    let stored = registry.field_by_tag(9001).unwrap();
    assert_eq!(stored.name(), "Symbol");
    assert_eq!(stored.as_fix().tags().unwrap(), [9001]);
    assert_eq!(stored.as_fix().aliases().collect::<Vec<_>>(), ["Ticker"]);
    assert_eq!(registry.field("TICKER").unwrap().name(), "Symbol");
    assert_eq!(registry.len(), 1 + super::crated());
}

#[test]
fn a_tag_another_field_answers_is_not_taken_by_a_name_fold() {
    let mut price = tagged("Price", 44, DataType::Float64);
    price.as_fix_mut().set_tags(&[9001]).unwrap();
    let mut registry =
        FixRegistry::from_fields([price, tagged("Symbol", 55, DataType::Utf8)]).unwrap();

    // The name folds, the tag is `Price`'s: the field merges, the tag stays.
    assert!(
        !registry
            .add_field(tagged("SYMBOL", 9001, DataType::Utf8))
            .unwrap()
    );
    assert_eq!(registry.field_by_tag(9001).unwrap().name(), "Price");
    assert!(
        registry
            .field_by_tag(55)
            .unwrap()
            .as_fix()
            .tags()
            .unwrap()
            .is_empty()
    );

    // An alternate tag another field holds as its alternate is the conflict
    // `update` raises.
    let before = registry.clone();
    let mut clash = tagged("symbol", 9002, DataType::Utf8);
    clash.as_fix_mut().set_tags(&[9001]).unwrap();
    let error = registry.add_field(clash).unwrap_err();
    assert!(error.is_conflict(), "{error}");
    assert_eq!(registry, before);

    // A canonical tag another field holds canonically under another name
    // is a field of its own, added beside the holder - but its name is
    // `Symbol`'s canonical one, and a canonical name is one field's: the
    // conflict names the holder of the name, not the holder of the tag.
    let error = registry
        .add_field(tagged("symbol", 44, DataType::Utf8))
        .unwrap_err();
    assert!(error.is_conflict(), "{error}");
    assert!(error.to_string().contains("Symbol"), "{error}");
    assert_eq!(registry, before);
}

#[test]
fn a_datatype_disagreement_by_name_is_refused_and_writes_nothing() {
    let mut registry = FixRegistry::from_fields([tagged("Symbol", 55, DataType::Utf8)]).unwrap();
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
    let mut registry = FixRegistry::new();
    let before = registry.clone();
    let own = yggdryl::fix_crate_fields().unwrap()[0].clone();
    assert!(!registry.add_field(own).unwrap());
    assert_eq!(registry, before);
}

#[test]
fn nested_fields_redirect_to_the_category_their_shape_names() {
    let mut registry =
        FixRegistry::from_fields([tagged("NoPartyIDs", 453, DataType::Int32)]).unwrap();

    let mut message = DataType::from_fields([]).unwrap().required_field("Order");
    message.as_fix_mut().set_msgtype("D").unwrap();
    assert!(registry.add_field(message).unwrap());
    assert_eq!(registry.msgtype("D").unwrap().name(), "Order");

    let item = DataType::from_fields([DataType::Utf8.nullable_field("PartyID")])
        .unwrap()
        .required_field("Party");
    assert!(registry.add_field(item.clone()).unwrap());
    assert_eq!(
        registry
            .definition(FixCategory::Components, "Party")
            .unwrap()
            .field_len(),
        1
    );

    let mut group = DataType::list(item.clone()).nullable_field("Parties");
    group.as_fix_mut().set_counter(453).unwrap();
    assert!(registry.add_field(group).unwrap());
    let mut hops = DataType::large_list(item.clone()).nullable_field("Hops");
    hops.as_fix_mut().set_counter(453).unwrap();
    assert!(registry.add_field(hops).unwrap());
    for name in ["Parties", "Hops"] {
        assert_eq!(
            registry
                .definition(FixCategory::Groups, name)
                .unwrap()
                .as_fix()
                .counter()
                .unwrap(),
            Some(453),
            "{name}"
        );
    }
    // A definition is not a field, and the category verb redirects a scalar.
    assert!(registry.get_field("Party").is_none());
    assert!(
        registry
            .add_definition(FixCategory::Fields, tagged("Symbol", 55, DataType::Utf8))
            .unwrap()
    );
    assert_eq!(registry.field(55).unwrap().name(), "Symbol");

    // A nested datatype that is no definition is refused as a scalar is.
    let before = registry.clone();
    let nullable = DataType::from_fields([]).unwrap().nullable_field("Loose");
    for refused in [
        DataType::list(nullable).nullable_field("Occurrences"),
        DataType::list(DataType::Utf8.nullable_field("Text")).nullable_field("Texts"),
    ] {
        let error = registry.add_field(refused).unwrap_err();
        assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
        assert!(error.to_string().contains("scalar"), "{error}");
        assert_eq!(registry, before);
    }
}

#[test]
fn a_component_extended_by_a_member_is_seen_extended_by_every_reference() {
    let mut registry = catalog();
    assert!(
        registry
            .add_field(tagged("PartyNote", 9002, DataType::Utf8))
            .unwrap()
    );
    let mut note = registry.field(9002).unwrap().clone();
    note.as_fix_mut().set_field_ref("PartyNote").unwrap();
    let mut extended = registry
        .definition(FixCategory::Components, "Party")
        .unwrap()
        .clone();
    extended
        .set_dtype(DataType::from_fields(extended.fields().iter().cloned().chain([note])).unwrap())
        .unwrap();

    assert!(
        !registry
            .add_definition(FixCategory::Components, extended.clone())
            .unwrap()
    );
    let party = registry
        .definition(FixCategory::Components, "Party")
        .unwrap();
    assert_eq!(names(party), ["PartyID", "PartyNote"]);
    for path in [
        "Party.PartyNote",
        "Parties.PartyNote",
        "NewOrderSingle.Parties.PartyNote",
    ] {
        let member = registry.field_by_path(&fpath(path)).unwrap();
        assert_eq!(member.as_fix().tag().unwrap(), Some(9002), "{path}");
        assert_eq!(member.as_fix().field_ref(), Some("partynote"), "{path}");
    }
    let parties = registry
        .field_by_path(&fpath("NewOrderSingle.Parties"))
        .unwrap();
    assert_eq!(names(occurrence(parties)), ["PartyID", "PartyNote"]);
    assert_eq!(
        registry
            .msgtype("D")
            .unwrap()
            .get_group_by_counter(453)
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
    assert!(
        !registry
            .add_definition(FixCategory::Components, extended.clone())
            .unwrap()
    );
    assert_eq!(registry, before);
    assert!(
        registry
            .create_definition(FixCategory::Components, extended)
            .is_err()
    );
    assert_eq!(registry, before);

    // A message extends the same way, keeping its code.
    let mut order = registry.msgtype("D").unwrap().as_field().clone();
    order
        .set_dtype(
            DataType::from_fields(
                order
                    .fields()
                    .iter()
                    .cloned()
                    .chain([DataType::Utf8.nullable_field("Text")]),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(
        !registry
            .add_definition(FixCategory::Messages, order)
            .unwrap()
    );
    let order = registry.msgtype("D").unwrap();
    assert_eq!(order.as_str(), "D");
    assert_eq!(names(order.as_field()), ["NoPartyIDs", "Parties", "Text"]);
}

#[test]
fn a_group_occurrence_is_extended_where_its_members_live() {
    let mut registry = catalog();

    // A bare list stating one more member of the occurrence: the stored
    // occurrence is the component's, so the member lands there and the
    // group keeps its markers.
    let member = DataType::from_fields([DataType::Utf8.nullable_field("PartyNote")])
        .unwrap()
        .required_field("party");
    let mut group = DataType::list(member).nullable_field("parties");
    group.as_fix_mut().set_counter(453).unwrap();
    assert!(!registry.add_field(group).unwrap());
    let party = registry
        .definition(FixCategory::Components, "Party")
        .unwrap();
    assert_eq!(names(party), ["PartyID", "PartyNote"]);
    assert_eq!(
        registry
            .field_by_path(&fpath("NewOrderSingle.Parties.PartyNote"))
            .unwrap()
            .dtype(),
        &DataType::Utf8
    );
    let parties = registry.definition(FixCategory::Groups, "Parties").unwrap();
    assert_eq!(parties.name(), "Parties");
    assert_eq!(parties.as_fix().counter().unwrap(), Some(453));
    assert_eq!(parties.as_fix().component(), Some("party"));
    assert_eq!(occurrence(parties).as_fix().component(), Some("party"));

    // An inline occurrence is appended to in place.
    registry
        .add_field(tagged("NoHops", 627, DataType::Int32))
        .unwrap();
    let hop = DataType::from_fields([DataType::Utf8.nullable_field("HopID")])
        .unwrap()
        .required_field("Hop");
    let mut hops = DataType::list(hop).nullable_field("Hops");
    hops.as_fix_mut().set_counter(627).unwrap();
    assert!(
        registry
            .add_definition(FixCategory::Groups, hops.clone())
            .unwrap()
    );
    let more = DataType::from_fields([
        DataType::Utf8.nullable_field("HopNote"),
        DataType::Utf8.nullable_field("hopid"),
    ])
    .unwrap()
    .required_field("Hop");
    hops.set_dtype(DataType::list(more)).unwrap();
    assert!(!registry.add_definition(FixCategory::Groups, hops).unwrap());
    let hops = registry.definition(FixCategory::Groups, "Hops").unwrap();
    assert_eq!(names(occurrence(hops)), ["HopID", "HopNote"]);
    assert_eq!(hops.as_fix().counter().unwrap(), Some(627));
    assert_eq!(
        FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
        registry
    );
}

#[test]
fn definition_merges_refuse_a_member_that_disagrees_atomically() {
    let mut registry = catalog_with([DataType::Int32.nullable_field("Extra")]);
    let before = registry.clone();

    // The same member under another datatype.
    let changed = DataType::from_fields([DataType::Int64.nullable_field("extra")])
        .unwrap()
        .required_field("Party");
    let error = registry
        .add_definition(FixCategory::Components, changed)
        .unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    let message = error.to_string();
    assert!(
        message.contains("Party.Extra") && message.contains("int32") && message.contains("int64"),
        "{message}"
    );
    assert_eq!(registry, before);

    // A reference restated inline under another datatype is the same
    // disagreement, and the refusal names the reference and both datatypes.
    let inline = DataType::from_fields([tagged("PartyID", 448, DataType::Int32)])
        .unwrap()
        .required_field("Party");
    let error = registry
        .add_definition(FixCategory::Components, inline)
        .unwrap_err();
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
    // the conflicts the `fix:` merge raises for a second identity.
    let mut recoded = registry.msgtype("D").unwrap().as_field().clone();
    recoded.as_fix_mut().set_msgtype("E").unwrap();
    let error = registry
        .add_definition(FixCategory::Messages, recoded)
        .unwrap_err();
    assert!(error.is_conflict(), "{error}");
    assert_eq!(registry, before);
    let mut recounted = registry
        .definition(FixCategory::Groups, "Parties")
        .unwrap()
        .clone();
    recounted.as_fix_mut().set_counter(627).unwrap();
    let error = registry
        .add_definition(FixCategory::Groups, recounted)
        .unwrap_err();
    assert!(error.is_conflict(), "{error}");
    assert_eq!(registry, before);
}

#[test]
fn add_fields_counts_what_arrived_and_what_folded() {
    let mut symbol = tagged("Symbol", 55, DataType::Utf8);
    symbol.as_fix_mut().set_aliases(["Ticker"]).unwrap();
    let mut registry =
        FixRegistry::from_fields([symbol, tagged("Price", 44, DataType::Float64)]).unwrap();
    registry
        .create_definition(
            FixCategory::Components,
            DataType::from_fields([])
                .unwrap()
                .required_field("Instrument"),
        )
        .unwrap();

    let mut described = tagged("SYMBOL", 55, DataType::Utf8);
    described
        .as_fix_mut()
        .set_description("by identity")
        .unwrap();
    let mut extended = DataType::from_fields([DataType::Utf8.nullable_field("Symbol")])
        .unwrap()
        .required_field("instrument");
    extended.as_fix_mut().set_description("by name").unwrap();
    let (added, merged) = registry
        .add_fields([
            yggdryl::fix_crate_fields().unwrap()[0].clone(),
            described,
            tagged("ticker", 9001, DataType::Utf8),
            tagged("TransactTime", 60, DataType::Utf8),
            DataType::from_fields([]).unwrap().required_field("Header"),
            extended,
        ])
        .unwrap();
    assert_eq!((added, merged), (2, 3));
    assert_eq!(registry.len(), 3 + super::crated());
    assert_eq!(
        registry.field(55).unwrap().description(),
        Some("by identity")
    );
    assert_eq!(registry.field(9001).unwrap().name(), "Symbol");
    let instrument = registry
        .definition(FixCategory::Components, "Instrument")
        .unwrap();
    assert_eq!(instrument.description(), Some("by name"));
    assert_eq!(names(instrument), ["Symbol"]);
    assert_eq!(registry.definitions(FixCategory::Components).count(), 2);

    // One mutation: a refusal in the middle writes nothing.
    let before = registry.clone();
    let error = registry
        .add_fields([
            tagged("Text", 58, DataType::Utf8),
            tagged("symbol", 9002, DataType::Int32),
            tagged("Account", 1, DataType::Utf8),
        ])
        .unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    assert_eq!(registry, before);
}

#[test]
fn merging_a_dictionary_folds_its_definitions_rather_than_replacing_them() {
    let mut target = catalog();
    let mut described = target
        .definition(FixCategory::Components, "Party")
        .unwrap()
        .clone();
    described
        .as_fix_mut()
        .set_description("stored wording")
        .unwrap();
    target
        .update_definition(FixCategory::Components, described)
        .unwrap();

    let mut source = catalog();
    source
        .add_field(tagged("PartyNote", 9002, DataType::Utf8))
        .unwrap();
    let mut note = source.field(9002).unwrap().clone();
    note.as_fix_mut().set_field_ref("PartyNote").unwrap();
    let mut extended = source
        .definition(FixCategory::Components, "Party")
        .unwrap()
        .clone();
    extended
        .set_dtype(DataType::from_fields(extended.fields().iter().cloned().chain([note])).unwrap())
        .unwrap();
    source
        .add_definition(FixCategory::Components, extended)
        .unwrap();
    let before_source = source.clone();

    assert_eq!(target.merge_with(&source).unwrap(), (1, 2));
    let party = target.definition(FixCategory::Components, "Party").unwrap();
    assert_eq!(names(party), ["PartyID", "PartyNote"]);
    assert_eq!(party.description(), Some("stored wording"));
    assert_eq!(
        target
            .field_by_path(&fpath("NewOrderSingle.Parties.PartyNote"))
            .unwrap()
            .as_fix()
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
    assert_eq!(target.merge_with(&source).unwrap(), (0, 3));
    assert_eq!(target, before);

    // A member that disagrees refuses the whole merge.
    let disagreeing = catalog_with([DataType::Int64.nullable_field("Extra")]);
    let mut target = catalog_with([DataType::Int32.nullable_field("Extra")]);
    let before = target.clone();
    let error = target.merge_with(&disagreeing).unwrap_err();
    assert!(error.to_string().contains("Party.Extra"), "{error}");
    assert_eq!(target, before);
}

#[test]
fn the_strict_verbs_keep_refusing_and_replacing() {
    let mut registry = FixRegistry::from_fields([tagged("Symbol", 55, DataType::Utf8)]).unwrap();
    registry
        .create_definition(
            FixCategory::Components,
            DataType::from_fields([DataType::Int32.nullable_field("Count")])
                .unwrap()
                .required_field("Plain"),
        )
        .unwrap();
    let before = registry.clone();

    let error = registry
        .insert(tagged("symbol", 9001, DataType::Utf8))
        .unwrap_err();
    assert!(error.is_conflict(), "{error}");
    assert_eq!(registry, before);
    let error = registry
        .update(tagged("symbol", 9001, DataType::Utf8))
        .unwrap_err();
    assert!(error.is_absent(), "{error}");
    assert_eq!(registry, before);
    let error = registry
        .create_definition(
            FixCategory::Components,
            DataType::from_fields([]).unwrap().required_field("plain"),
        )
        .unwrap_err();
    assert!(error.is_conflict(), "{error}");
    assert_eq!(registry, before);

    // `insert_definition` replaces the members wholesale, where the lenient
    // verb would have kept `Count`.
    registry
        .insert_definition(
            FixCategory::Components,
            DataType::from_fields([DataType::Utf8.nullable_field("Other")])
                .unwrap()
                .required_field("Plain"),
        )
        .unwrap();
    assert_eq!(
        names(
            registry
                .definition(FixCategory::Components, "Plain")
                .unwrap()
        ),
        ["Other"]
    );
}

#[test]
fn a_member_stated_inline_agrees_with_the_reference_stored_for_it() {
    let mut registry = catalog();

    // A dictionary built in memory states `PartyID` inline where the loaded
    // one references it: both describe tag 448 as utf8, so the stored
    // reference stays and only the new member arrives.
    let inline = DataType::from_fields([
        tagged("partyid", 448, DataType::Utf8),
        DataType::Utf8.nullable_field("PartyNote"),
    ])
    .unwrap()
    .required_field("Party");
    assert!(
        !registry
            .add_definition(FixCategory::Components, inline)
            .unwrap()
    );
    let party = registry
        .definition(FixCategory::Components, "Party")
        .unwrap();
    assert_eq!(names(party), ["PartyID", "PartyNote"]);
    assert_eq!(party.fields()[0].as_fix().field_ref(), Some("partyid"));
    assert_eq!(
        registry
            .field_by_path(&fpath("NewOrderSingle.Parties.PartyNote"))
            .unwrap()
            .dtype(),
        &DataType::Utf8
    );

    // The other way round: a stored inline member, restated by a reference
    // to the field of that datatype, is kept inline; a reference to a field
    // of another datatype is refused.
    registry
        .add_field(tagged("Symbol", 55, DataType::Utf8))
        .unwrap();
    registry
        .create_definition(
            FixCategory::Components,
            DataType::from_fields([DataType::Utf8.nullable_field("Symbol")])
                .unwrap()
                .required_field("Instrument"),
        )
        .unwrap();
    let mut symbol = registry.field(55).unwrap().clone();
    symbol.as_fix_mut().set_field_ref("Symbol").unwrap();
    let restated = DataType::from_fields([symbol])
        .unwrap()
        .required_field("Instrument");
    assert!(
        !registry
            .add_definition(FixCategory::Components, restated)
            .unwrap()
    );
    let instrument = registry
        .definition(FixCategory::Components, "Instrument")
        .unwrap();
    assert!(instrument.fields()[0].as_fix().field_ref().is_none());
    let before = registry.clone();
    let mut count = tagged("Symbol", 9003, DataType::Int32);
    count.as_fix_mut().set_field_ref("NoPartyIDs").unwrap();
    let disagreeing = DataType::from_fields([count])
        .unwrap()
        .required_field("Instrument");
    let error = registry
        .add_definition(FixCategory::Components, disagreeing)
        .unwrap_err();
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
    let mut registry = catalog();
    let mut respelled = DataType::Utf8.required_field("partyid");
    respelled.as_fix_mut().set_tag(9001).unwrap();
    assert!(!registry.add_field(respelled).unwrap());

    // The stored shape stays, and every reference to the field carries the
    // merged metadata.
    let stored = registry.field_by_tag(448).unwrap();
    assert!(stored.is_nullable());
    assert_eq!(stored.as_fix().tags().unwrap(), [9001]);
    assert!(std::ptr::eq(
        registry.get_field_by_tag(9001).unwrap(),
        stored
    ));
    for path in ["Party.PartyID", "NewOrderSingle.Parties.PartyID"] {
        let member = registry.field_by_path(&fpath(path)).unwrap();
        assert_eq!(member.as_fix().tags().unwrap(), [9001], "{path}");
        assert!(member.is_nullable(), "{path}");
    }
    assert_eq!(
        FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
        registry
    );
}

#[test]
fn a_separator_respelling_is_a_spelling_of_the_stored_name() {
    let mut symbol = tagged("Symbol", 55, DataType::Utf8);
    symbol.as_fix_mut().set_aliases(["Ticker"]).unwrap();
    let mut registry = FixRegistry::from_fields([symbol]).unwrap();

    // The crate's one fold drops `_`, `-` and space beside the case, so a
    // name lookup, the fold by name and the alias dedupe all read `Sym_bol`
    // as `Symbol` and `Tick-er` as `Ticker`.
    assert_eq!(registry.field("sym_bol").unwrap().name(), "Symbol");
    assert_eq!(registry.field("Tick-er").unwrap().name(), "Symbol");
    let mut respelled = tagged("Sym_bol", 9001, DataType::Utf8);
    respelled
        .as_fix_mut()
        .set_aliases(["Tick-er", "SYM"])
        .unwrap();
    assert!(!registry.add_field(respelled).unwrap());
    let stored = registry.field_by_tag(9001).unwrap();
    assert_eq!(stored.name(), "Symbol");
    assert_eq!(stored.as_fix().tags().unwrap(), [9001]);
    assert_eq!(
        stored.as_fix().aliases().collect::<Vec<_>>(),
        ["Ticker", "SYM"]
    );
    assert_eq!(registry.len(), 1 + super::crated());

    // By identity the same respelling is the stored name too.
    let mut described = tagged("sym-bol", 55, DataType::Utf8);
    described.as_fix_mut().set_description("respelled").unwrap();
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
            .insert(tagged("SYM_BOL", 55, DataType::Utf8))
            .unwrap()
            .unwrap()
            .name(),
        "Symbol"
    );
    assert_eq!(registry.field(55).unwrap().name(), "Symbol");
    let before = registry.clone();
    let error = registry
        .insert(tagged("Sym_bol", 9002, DataType::Utf8))
        .unwrap_err();
    assert!(error.is_conflict(), "{error}");
    assert_eq!(registry, before);
}

#[test]
fn a_canonical_identity_supersedes_the_alternate_another_field_lists() {
    // FIX itself does this: `QuoteAckStatus` is tag 1865, and `QuoteStatus`
    // lists 1865 as the tag it superseded. The canonical holder answers the
    // identifier whichever arrived first, exactly as a canonical name
    // answers over an alias - the alternate is a fallback, never a claim.
    let mut price = tagged("Price", 44, DataType::Float64);
    price.as_fix_mut().set_tags(&[9001]).unwrap();
    let mut registry = FixRegistry::from_fields([price]).unwrap();
    assert!(
        registry
            .add_field(tagged("Symbol", 9001, DataType::Utf8))
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
    assert_eq!(registry.field(44).unwrap().as_fix().tags().unwrap(), [9001]);

    let mut ticker = tagged("Ticker", 55, DataType::Utf8);
    ticker.as_fix_mut().set_tags(&[44]).unwrap();
    assert!(registry.add_field(ticker).unwrap());
    assert_eq!(registry.field(44).unwrap().name(), "Price");
    assert_eq!(
        registry
            .field(FixId::of(44, "Price").unwrap())
            .unwrap()
            .name(),
        "Price"
    );
    assert_eq!(registry.field(55).unwrap().as_fix().tags().unwrap(), [44]);
    assert_eq!(registry.len(), 3 + super::crated());
}

#[test]
fn a_group_occurrence_folds_its_members_into_the_component_and_nothing_else() {
    let mut registry = catalog();
    let mut member = DataType::from_fields([DataType::Utf8.nullable_field("PartyNote")])
        .unwrap()
        .required_field("party");
    member
        .as_fix_mut()
        .set_description("occurrence wording")
        .unwrap();
    let mut group = DataType::list(member).nullable_field("parties");
    group.as_fix_mut().set_counter(453).unwrap();
    group.as_fix_mut().set_description("group wording").unwrap();
    assert!(!registry.add_definition(FixCategory::Groups, group).unwrap());

    // The occurrence's root describes the group's occurrence, not the
    // component it happens to be: the member arrives there, the wording
    // does not, and the group takes its own.
    let party = registry
        .definition(FixCategory::Components, "Party")
        .unwrap();
    assert_eq!(names(party), ["PartyID", "PartyNote"]);
    assert_eq!(party.description(), None);
    let parties = registry.definition(FixCategory::Groups, "Parties").unwrap();
    assert_eq!(parties.description(), Some("group wording"));
    assert_eq!(occurrence(parties).description(), None);
}

#[test]
fn a_reference_to_a_definition_arriving_in_the_same_merge_restates_an_inline_member() {
    // The target states the group inline inside its component; the source
    // holds the group as a definition and references it from the component.
    // Both describe one shape, and the group arrives in the same merge as
    // the reference to it, after the components in category order.
    let fields = [
        tagged("NoHops", 627, DataType::Int32),
        tagged("HopID", 628, DataType::Utf8),
    ];
    let hop = DataType::from_fields([DataType::Utf8.nullable_field("HopID")])
        .unwrap()
        .required_field("Hop");
    let mut target = FixRegistry::from_fields(fields.clone()).unwrap();
    target
        .create_definition(
            FixCategory::Components,
            DataType::from_fields([DataType::list(hop.clone()).nullable_field("Hops")])
                .unwrap()
                .required_field("Route"),
        )
        .unwrap();

    let mut source = FixRegistry::from_fields(fields).unwrap();
    let mut hops = DataType::list(hop).nullable_field("Hops");
    hops.as_fix_mut().set_counter(627).unwrap();
    source.create_definition(FixCategory::Groups, hops).unwrap();
    let mut restated = source
        .definition(FixCategory::Groups, "Hops")
        .unwrap()
        .clone();
    restated.as_fix_mut().set_group("Hops").unwrap();
    source
        .create_definition(
            FixCategory::Components,
            DataType::from_fields([restated, DataType::Utf8.nullable_field("RouteID")])
                .unwrap()
                .required_field("Route"),
        )
        .unwrap();

    assert_eq!(target.merge_with(&source).unwrap(), (0, 2));
    let route = target.definition(FixCategory::Components, "Route").unwrap();
    assert_eq!(names(route), ["Hops", "RouteID"]);
    assert!(route.fields()[0].as_fix().group().is_none(), "kept inline");
    assert_eq!(
        target
            .definition(FixCategory::Groups, "Hops")
            .unwrap()
            .as_fix()
            .counter()
            .unwrap(),
        Some(627)
    );
    assert_eq!(
        FixRegistry::from_json(&target.into_json().unwrap()).unwrap(),
        target
    );
}

/// The registry a fold-table fixture starts from: `Symbol` on 55, spoken by
/// `fix44`, `Price` on 44 and `MsgType` on 35.
fn holders() -> FixRegistry {
    let mut symbol = tagged("Symbol", 55, DataType::Utf8);
    symbol.as_fix_mut().set_branches(["fix44"]).unwrap();
    FixRegistry::from_fields([
        symbol,
        tagged("Price", 44, DataType::Float64),
        tagged("MsgType", 35, DataType::Utf8),
    ])
    .unwrap()
}

/// The four rows of the fold table as one arrival each, every one spoken
/// by `venue`: the same identity respelled, a held tag under another name,
/// a held name under another tag, and a field nothing holds.
fn arrivals() -> [Field; 4] {
    let mut respelled = tagged("Msg_Type", 35, DataType::Utf8);
    respelled.as_fix_mut().set_description("respelled").unwrap();
    let mut arrivals = [
        respelled,
        tagged("VenueSymbol", 55, DataType::Utf8),
        tagged("price", 9001, DataType::Float64),
        tagged("Account", 1, DataType::Utf8),
    ];
    for arrival in &mut arrivals {
        arrival.as_fix_mut().set_branches(["venue"]).unwrap();
    }
    arrivals
}

/// What every row of the fold table leaves behind, whichever verb folded it.
fn assert_fold_table(registry: &FixRegistry) {
    assert_eq!(registry.len(), 5 + super::crated());

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
    assert_eq!(msgtype.as_fix().branches().collect::<Vec<_>>(), ["venue"]);

    // Row 2: a held tag under another name is a second field beside the
    // holder. The bare tag keeps answering the first holder, which gained
    // the arrival's name as an alias; the newcomer is reached by its name
    // and by its id, and holds the tag canonically too.
    let symbol = registry.field_by_tag(55).unwrap();
    assert_eq!(symbol.name(), "Symbol");
    assert_eq!(
        symbol.as_fix().aliases().collect::<Vec<_>>(),
        ["VenueSymbol"]
    );
    assert!(symbol.as_fix().tags().unwrap().is_empty());
    assert_eq!(
        symbol.as_fix().branches().collect::<Vec<_>>(),
        ["fix44"],
        "an alias lent to the holder is not a membership"
    );
    let venue = registry.field_by_name("VenueSymbol").unwrap();
    assert!(!std::ptr::eq(venue, symbol));
    assert_eq!(venue.name(), "VenueSymbol");
    assert_eq!(venue.as_fix().tag().unwrap(), Some(55));
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
    assert_eq!(venue.as_fix().branches().collect::<Vec<_>>(), ["venue"]);

    // Row 3: a held name under another tag is the holder spelled with
    // another number: the tag becomes the holder's alternate and no second
    // field exists.
    let price = registry.field_by_tag(44).unwrap();
    assert_eq!(price.name(), "Price");
    assert_eq!(price.as_fix().tags().unwrap(), [9001]);
    assert!(std::ptr::eq(registry.field_by_tag(9001).unwrap(), price));
    assert!(
        registry
            .get_field_by_id(FixId::of(9001, "price").unwrap())
            .is_none()
    );
    assert_eq!(price.as_fix().branches().collect::<Vec<_>>(), ["venue"]);

    // Row 4: neither, so it arrived as it was.
    let account = registry.field_by_tag(1).unwrap();
    assert_eq!(account.name(), "Account");
    assert_eq!(account.as_fix().branches().collect::<Vec<_>>(), ["venue"]);

    // Membership is provenance, listed and never resolved through.
    assert_eq!(registry.dialects(), ["fix44", "venue"]);

    // Tag-major, the holder first: the two fields on 55 are adjacent, the
    // holder of the bare tag leads whatever the ids say, and
    // `next_field_after` walks the same order.
    let order: Vec<(i32, FixId)> = registry
        .iter()
        .map(|field| {
            (
                field.as_fix().tag().unwrap().unwrap(),
                field.as_fix().id().unwrap().unwrap(),
            )
        })
        .filter(|(tag, _)| *tag < FixId::DEFINITION_TAG_MIN && !(65_000..65_100).contains(tag))
        .collect();
    let tags: Vec<i32> = order.iter().map(|(tag, _)| *tag).collect();
    let mut sorted = tags.clone();
    sorted.sort_unstable();
    assert_eq!(tags, sorted);
    assert_eq!(tags, [1, 35, 44, 55, 55]);
    let (first, second) = (order[3].1, order[4].1);
    assert_eq!(
        first,
        registry
            .field_by_tag(55)
            .unwrap()
            .as_fix()
            .id()
            .unwrap()
            .unwrap()
    );
    assert_eq!(
        registry
            .next_field_after(Some(first))
            .unwrap()
            .as_fix()
            .id()
            .unwrap(),
        Some(second)
    );
}

#[test]
fn the_fold_table_holds_through_add_field() {
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
    // The snapshot writes fields in iteration order - tag-major, then id -
    // and reads them back in that order, so which of two fields on one tag
    // the bare tag answers, and which of them lent its name to the other,
    // has to be what the writer held: `Symbol` was first and holds
    // `VenueSymbol` as an alias; `VenueSymbol` holds no alias.
    let mut registry = holders();
    let [_, venue, _, _] = arrivals();
    assert!(registry.add_field(venue).unwrap());
    assert_eq!(registry.field_by_tag(55).unwrap().name(), "Symbol");
    assert!(
        registry
            .field_by_name("VenueSymbol")
            .unwrap()
            .as_fix()
            .aliases()
            .next()
            .is_none()
    );

    let restored = FixRegistry::from_json(&registry.into_json().unwrap()).unwrap();
    assert_eq!(restored.field_by_tag(55).unwrap().name(), "Symbol");
    assert_eq!(
        restored
            .field_by_name("Symbol")
            .unwrap()
            .as_fix()
            .aliases()
            .collect::<Vec<_>>(),
        ["VenueSymbol"]
    );
    assert!(
        restored
            .field_by_name("VenueSymbol")
            .unwrap()
            .as_fix()
            .aliases()
            .next()
            .is_none()
    );
    assert_eq!(restored, registry);
}

#[test]
fn the_fold_table_holds_through_merge_with() {
    let mut registry = holders();
    let source = FixRegistry::from_fields(arrivals()).unwrap();
    let before_source = source.clone();
    assert_eq!(registry.merge_with(&source).unwrap(), (2, 2));
    assert_fold_table(&registry);
    assert_eq!(source, before_source);

    // The other way round says the same thing with the roles swapped: the
    // source's `VenueSymbol` is then the first holder of 55, and `Symbol`
    // arrives beside it.
    let mut reversed = FixRegistry::from_fields(arrivals()).unwrap();
    assert_eq!(reversed.merge_with(&holders()).unwrap(), (1, 2));
    assert_eq!(reversed.len(), 5 + super::crated());
    assert_eq!(reversed.field_by_tag(55).unwrap().name(), "VenueSymbol");
    assert_eq!(
        reversed
            .field_by_tag(55)
            .unwrap()
            .as_fix()
            .aliases()
            .collect::<Vec<_>>(),
        ["Symbol"]
    );
    assert_eq!(reversed.field_by_name("Symbol").unwrap().name(), "Symbol");
    assert_eq!(reversed.field_by_tag(9001).unwrap().name(), "price");
    assert_eq!(
        reversed
            .field_by_tag(9001)
            .unwrap()
            .as_fix()
            .branches()
            .collect::<Vec<_>>(),
        ["venue"]
    );
    assert_eq!(reversed.dialects(), ["fix44", "venue"]);
}

#[test]
fn a_merged_membership_is_the_union_of_what_each_side_spoke() {
    let mut symbol = tagged("Symbol", 55, DataType::Utf8);
    symbol.as_fix_mut().set_branches(["fix44"]).unwrap();
    let mut registry = FixRegistry::from_fields([symbol.clone()]).unwrap();

    // By identity, by name and by tag, the dialects union, folded once,
    // deduplicated and sorted, however they were spelled.
    let mut respelled = tagged("SYMBOL", 55, DataType::Utf8);
    respelled
        .as_fix_mut()
        .set_branches(["Venue", "FIX44"])
        .unwrap();
    assert!(!registry.add_field(respelled).unwrap());
    let mut alternate = tagged("symbol", 9001, DataType::Utf8);
    alternate.as_fix_mut().set_branches(["other"]).unwrap();
    assert!(!registry.add_field(alternate).unwrap());
    let stored = registry.field_by_tag(55).unwrap();
    assert_eq!(
        stored.as_fix().branches().collect::<Vec<_>>(),
        ["fix44", "other", "venue"]
    );
    assert!(stored.as_fix().has_branch("VENUE"));
    assert!(!stored.as_fix().has_branch("standard"));
    assert_eq!(registry.dialects(), ["fix44", "other", "venue"]);

    // A field spoken by nobody stays spoken by nobody, and a source that
    // speaks it says so after the merge.
    let mut target = FixRegistry::from_fields([tagged("Price", 44, DataType::Float64)]).unwrap();
    assert!(
        target
            .field_by_tag(44)
            .unwrap()
            .as_fix()
            .branches()
            .next()
            .is_none()
    );
    assert!(target.dialects().is_empty());
    let mut price = tagged("Price", 44, DataType::Float64);
    price.as_fix_mut().set_branches(["venue"]).unwrap();
    let source = FixRegistry::from_fields([price, symbol]).unwrap();
    assert_eq!(target.merge_with(&source).unwrap(), (1, 1));
    assert_eq!(
        target
            .field_by_tag(44)
            .unwrap()
            .as_fix()
            .branches()
            .collect::<Vec<_>>(),
        ["venue"]
    );
    assert_eq!(
        target
            .field_by_tag(55)
            .unwrap()
            .as_fix()
            .branches()
            .collect::<Vec<_>>(),
        ["fix44"]
    );
    assert_eq!(target.dialects(), ["fix44", "venue"]);
}

#[test]
fn message_codes_live_in_one_namespace() {
    let mut registry = catalog();

    // A second message on a held code under another name is a second
    // message: the bare code keeps answering the first holder, the newcomer
    // is reached by its name, and its code is the held one.
    let mut venue = DataType::from_fields([DataType::Utf8.nullable_field("VenueID")])
        .unwrap()
        .required_field("VenueOrder");
    venue.as_fix_mut().set_msgtype("D").unwrap();
    assert!(
        registry
            .add_definition(FixCategory::Messages, venue.clone())
            .unwrap()
    );
    assert_eq!(registry.msgtype("D").unwrap().name(), "NewOrderSingle");
    assert_eq!(registry.msgtype("VenueOrder").unwrap().as_str(), "D");
    assert_eq!(
        names(registry.msgtype("venue_order").unwrap().as_field()),
        ["VenueID"]
    );
    assert_eq!(registry.msgtypes().count(), 2);
    assert_eq!(
        registry
            .definition(FixCategory::Messages, "VenueOrder")
            .unwrap()
            .as_fix()
            .msgtype(),
        Some("D")
    );

    // A re-declaration under the same folded name folds into the stored
    // message, keeping its spelling and its code.
    let mut restated = DataType::from_fields([DataType::Utf8.nullable_field("Text")])
        .unwrap()
        .required_field("new_order_single");
    restated.as_fix_mut().set_msgtype("D").unwrap();
    assert!(
        !registry
            .add_definition(FixCategory::Messages, restated)
            .unwrap()
    );
    let order = registry.msgtype("D").unwrap();
    assert_eq!(order.name(), "NewOrderSingle");
    assert_eq!(names(order.as_field()), ["NoPartyIDs", "Parties", "Text"]);
    assert_eq!(registry.msgtypes().count(), 2);
    assert_eq!(
        FixRegistry::from_json(&registry.into_json().unwrap()).unwrap(),
        registry
    );

    // `merge_with` reads the namespace the same way.
    let mut target = catalog();
    let mut source = FixRegistry::new();
    source
        .create_definition(FixCategory::Messages, venue)
        .unwrap();
    assert_eq!(target.merge_with(&source).unwrap(), (0, 0));
    assert_eq!(target.msgtype("D").unwrap().name(), "NewOrderSingle");
    assert_eq!(target.msgtype("VenueOrder").unwrap().as_str(), "D");
    assert_eq!(target.msgtypes().count(), 2);
    assert_eq!(
        names(target.msgtype("D").unwrap().as_field()),
        ["NoPartyIDs", "Parties"]
    );
}

#[test]
fn a_bare_code_answers_the_message_the_code_set_names_else_the_first_in_name_order() {
    // Two messages on `D`: which one the bare code answers is a fact of the
    // catalog's content, never of the order it was built in, so a fold, a
    // store and a load all answer the same one. Without a code set naming
    // `D`, name order decides: `AlgoOrder` sorts before `NewOrderSingle`
    // however late it arrives.
    let mut registry = catalog();
    let mut algo = DataType::from_fields([DataType::Utf8.nullable_field("AlgoID")])
        .unwrap()
        .required_field("AlgoOrder");
    algo.as_fix_mut().set_msgtype("D").unwrap();
    assert!(
        registry
            .add_definition(FixCategory::Messages, algo.clone())
            .unwrap()
    );
    assert_eq!(registry.msgtype("AlgoOrder").unwrap().as_str(), "D");
    assert_eq!(registry.msgtypes().count(), 2);
    assert_eq!(registry.msgtype("D").unwrap().name(), "AlgoOrder");
    assert_eq!(registry.msgtype("NewOrderSingle").unwrap().as_str(), "D");

    // Tag 35's code set names `D` `NewOrderSingle`: that message answers the
    // bare code, whichever name sorts first and whichever arrived first, and
    // the code set arriving after both re-decides it.
    let mut msgtype = tagged("MsgType", 35, DataType::Utf8);
    msgtype
        .as_fix_mut()
        .set_codes(&[yggdryl::FixCode::new("NewOrderSingle", "D")])
        .unwrap();
    assert!(registry.add_field(msgtype.clone()).unwrap());
    assert_eq!(registry.msgtype("D").unwrap().name(), "NewOrderSingle");
    assert_eq!(registry.msgtype("AlgoOrder").unwrap().as_str(), "D");

    let mut target = catalog();
    assert!(target.add_field(msgtype).unwrap());
    let mut source = FixRegistry::new();
    source
        .create_definition(FixCategory::Messages, algo)
        .unwrap();
    assert_eq!(target.merge_with(&source).unwrap(), (0, 0));
    assert_eq!(target.msgtype("AlgoOrder").unwrap().as_str(), "D");
    assert_eq!(target.msgtype("D").unwrap().name(), "NewOrderSingle");
    let restored = FixRegistry::from_json(&target.into_json().unwrap()).unwrap();
    assert_eq!(restored.msgtype("D").unwrap().name(), "NewOrderSingle");
}
