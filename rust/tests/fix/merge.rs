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
        .definition(FixCategory::Components, "Party", None)
        .unwrap()
        .clone();
    let mut group = DataType::list(component).nullable_field("Parties");
    group.as_fix_mut().set_counter(453).unwrap();
    group.as_fix_mut().set_component("Party").unwrap();
    registry
        .create_definition(FixCategory::Groups, group)
        .unwrap();
    let mut group = registry
        .definition(FixCategory::Groups, "Parties", None)
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
    assert_eq!(stored.as_fix().id().unwrap(), Some(FixId::standard(55)));
    assert_eq!(stored.as_fix().tags().unwrap(), [65, 66, 9001]);
    assert_eq!(
        stored.as_fix().aliases().collect::<Vec<_>>(),
        ["Ticker", "Sym"]
    );
    assert_eq!(stored.description(), Some("incoming"));

    // The incoming identity resolves to the merged field, by tag and by id.
    assert!(std::ptr::eq(
        registry.get_field_by_tag(9001).unwrap(),
        stored
    ));
    assert!(std::ptr::eq(
        registry.get_field_by_id(FixId::standard(9001)).unwrap(),
        stored
    ));
    assert!(std::ptr::eq(
        registry.get_field_by_id(FixId::standard(66)).unwrap(),
        stored
    ));
    assert_eq!(registry.field("sym").unwrap().name(), "Symbol");
    assert_eq!(
        registry.field(FixId::standard(65)).unwrap().name(),
        "Symbol"
    );

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

    // A canonical tag another field holds canonically is that field's
    // identity, and the stored name refuses the incoming one.
    let error = registry
        .add_field(tagged("symbol", 44, DataType::Utf8))
        .unwrap_err();
    assert!(matches!(error, Error::InvalidRecord { .. }), "{error}");
    assert!(error.to_string().contains("Price"), "{error}");
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
    assert_eq!(registry.msgtype("D", None).unwrap().name(), "Order");

    let item = DataType::from_fields([DataType::Utf8.nullable_field("PartyID")])
        .unwrap()
        .required_field("Party");
    assert!(registry.add_field(item.clone()).unwrap());
    assert_eq!(
        registry
            .definition(FixCategory::Components, "Party", None)
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
                .definition(FixCategory::Groups, name, None)
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
        .definition(FixCategory::Components, "Party", None)
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
        .definition(FixCategory::Components, "Party", None)
        .unwrap();
    assert_eq!(names(party), ["PartyID", "PartyNote"]);
    for path in [
        "Party.PartyNote",
        "Parties.PartyNote",
        "NewOrderSingle.Parties.PartyNote",
    ] {
        let member = registry.field_by_path(&fpath(path), None).unwrap();
        assert_eq!(member.as_fix().tag().unwrap(), Some(9002), "{path}");
        assert_eq!(member.as_fix().field_ref(), Some("partynote"), "{path}");
    }
    let parties = registry
        .field_by_path(&fpath("NewOrderSingle.Parties"), None)
        .unwrap();
    assert_eq!(names(occurrence(parties)), ["PartyID", "PartyNote"]);
    assert_eq!(
        registry
            .msgtype("D", None)
            .unwrap()
            .get_group_by_counter(FixId::standard(453))
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
    let mut order = registry.msgtype("D", None).unwrap().as_field().clone();
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
    let order = registry.msgtype("D", None).unwrap();
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
        .definition(FixCategory::Components, "Party", None)
        .unwrap();
    assert_eq!(names(party), ["PartyID", "PartyNote"]);
    assert_eq!(
        registry
            .field_by_path(&fpath("NewOrderSingle.Parties.PartyNote"), None)
            .unwrap()
            .dtype(),
        &DataType::Utf8
    );
    let parties = registry
        .definition(FixCategory::Groups, "Parties", None)
        .unwrap();
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
    let hops = registry
        .definition(FixCategory::Groups, "Hops", None)
        .unwrap();
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
    let mut recoded = registry.msgtype("D", None).unwrap().as_field().clone();
    recoded.as_fix_mut().set_msgtype("E").unwrap();
    let error = registry
        .add_definition(FixCategory::Messages, recoded)
        .unwrap_err();
    assert!(error.is_conflict(), "{error}");
    assert_eq!(registry, before);
    let mut recounted = registry
        .definition(FixCategory::Groups, "Parties", None)
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
        .definition(FixCategory::Components, "Instrument", None)
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
        .definition(FixCategory::Components, "Party", None)
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
        .definition(FixCategory::Components, "Party", None)
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
    let party = target
        .definition(FixCategory::Components, "Party", None)
        .unwrap();
    assert_eq!(names(party), ["PartyID", "PartyNote"]);
    assert_eq!(party.description(), Some("stored wording"));
    assert_eq!(
        target
            .field_by_path(&fpath("NewOrderSingle.Parties.PartyNote"), None)
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
                .definition(FixCategory::Components, "Plain", None)
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
        .definition(FixCategory::Components, "Party", None)
        .unwrap();
    assert_eq!(names(party), ["PartyID", "PartyNote"]);
    assert_eq!(party.fields()[0].as_fix().field_ref(), Some("partyid"));
    assert_eq!(
        registry
            .field_by_path(&fpath("NewOrderSingle.Parties.PartyNote"), None)
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
        .definition(FixCategory::Components, "Instrument", None)
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
        let member = registry.field_by_path(&fpath(path), None).unwrap();
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
    assert_eq!(
        registry.field(FixId::standard(9001)).unwrap().name(),
        "Symbol"
    );
    assert_eq!(registry.field(44).unwrap().as_fix().tags().unwrap(), [9001]);

    let mut ticker = tagged("Ticker", 55, DataType::Utf8);
    ticker.as_fix_mut().set_tags(&[44]).unwrap();
    assert!(registry.add_field(ticker).unwrap());
    assert_eq!(registry.field(FixId::standard(44)).unwrap().name(), "Price");
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
        .definition(FixCategory::Components, "Party", None)
        .unwrap();
    assert_eq!(names(party), ["PartyID", "PartyNote"]);
    assert_eq!(party.description(), None);
    let parties = registry
        .definition(FixCategory::Groups, "Parties", None)
        .unwrap();
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
        .definition(FixCategory::Groups, "Hops", None)
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
    let route = target
        .definition(FixCategory::Components, "Route", None)
        .unwrap();
    assert_eq!(names(route), ["Hops", "RouteID"]);
    assert!(route.fields()[0].as_fix().group().is_none(), "kept inline");
    assert_eq!(
        target
            .definition(FixCategory::Groups, "Hops", None)
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
