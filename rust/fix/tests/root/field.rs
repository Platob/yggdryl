//! `rust/fix/src/field.rs`: the FIX view of a field - `FixField` and
//! `FixFieldMut`, minted here under `Scheme::FIX` by the core's exported view
//! builder, borrowed with `FixField::new(&field)` and
//! `FixFieldMut::new(&mut field)` - and what a field states under
//! `FIX:parents`: the identifier types holding the parents of the identifier
//! it states, nearest first, written folded and once each, read without a
//! copy, and held to that form by every registry that takes the field.

use yggdryl::{DataType, Error, Field, Scheme};
use yggdryl_fix::{FixField, FixFieldMut, FixRegistry};

fn orderid() -> Field {
    let mut field = DataType::utf8().nullable_field("OrderID");
    FixFieldMut::new(&mut field).set_tag(37).expect("a tag");
    field
}

fn parents(field: &Field) -> Vec<&str> {
    FixField::new(field).parents().collect()
}

#[test]
fn a_list_is_written_folded_in_the_order_given_and_read_back() {
    crate::install::installed();
    let mut field = orderid();
    assert!(parents(&field).is_empty(), "a field states none until told");
    FixFieldMut::new(&mut field)
        .set_parents(["ParentOrderID", "Grand_Parent-Order ID", "OrigOrderID"])
        .expect("three types");
    assert_eq!(
        field.get_metadata("FIX:parents"),
        Some(r#"["parentorderid","grandparentorderid","origorderid"]"#)
    );
    assert_eq!(
        parents(&field),
        ["parentorderid", "grandparentorderid", "origorderid"]
    );
    // A second statement replaces the first whole.
    FixFieldMut::new(&mut field)
        .set_parents(["origorderid"])
        .expect("one type");
    assert_eq!(parents(&field), ["origorderid"]);
}

#[test]
fn an_empty_list_removes_the_property_and_remove_answers_what_it_held() {
    crate::install::installed();
    let mut field = orderid();
    FixFieldMut::new(&mut field)
        .set_parents(["origorderid"])
        .expect("one type");
    assert_eq!(
        field.get_metadata("FIX:parents"),
        Some(r#"["origorderid"]"#)
    );
    FixFieldMut::new(&mut field)
        .set_parents::<[&str; 0], &str>([])
        .expect("none");
    assert_eq!(field.get_metadata("FIX:parents"), None);
    assert!(parents(&field).is_empty());

    FixFieldMut::new(&mut field)
        .set_parents(["parentorderid"])
        .expect("one type");
    assert_eq!(
        FixFieldMut::new(&mut field).remove_parents().as_deref(),
        Some(r#"["parentorderid"]"#)
    );
    assert_eq!(FixFieldMut::new(&mut field).remove_parents(), None);
    assert!(parents(&field).is_empty());
}

#[test]
fn a_spelling_no_type_folds_from_and_a_repeat_are_refused_and_the_field_stands() {
    crate::install::installed();
    let mut field = orderid();
    FixFieldMut::new(&mut field)
        .set_parents(["parentorderid"])
        .expect("one type");
    for stated in [
        // A byte no word holds, and nothing at all.
        vec!["origorderid", "not:a:type"],
        vec!["origorderid", ""],
        vec!["origorderid", "\u{e9}tat"],
        // One type under two spellings is one type twice.
        vec!["origorderid", "Orig_Order-ID"],
    ] {
        let refusal = FixFieldMut::new(&mut field)
            .set_parents(stated.iter().copied())
            .expect_err("refused");
        assert!(
            matches!(&refusal, Error::InvalidMetadataValue { key, .. } if key == "FIX:parents"),
            "{stated:?}: {refusal}"
        );
        assert_eq!(
            field.get_metadata("FIX:parents"),
            Some(r#"["parentorderid"]"#),
            "{stated:?} leaves the field unchanged"
        );
    }
}

#[test]
fn a_hand_edited_document_is_refused_by_the_registry_that_would_take_it() {
    crate::install::installed();
    for stored in [
        "not json",
        r#"{"parentorderid":1}"#,
        "[1]",
        // Each type is the folded word its spelling reads as, once.
        r#"["ParentOrderID"]"#,
        r#"["origorderid","origorderid"]"#,
        r#"["not:a:type"]"#,
    ] {
        let mut field = orderid();
        field
            .insert_metadata("FIX:parents", stored)
            .expect("inert text");
        let refusal = FixRegistry::new()
            .add_field(field)
            .expect_err("a registry holds fields to what the setter writes");
        assert!(
            refusal.to_string().contains("FIX:parents"),
            "{stored}: {refusal}"
        );
    }
    // What the setter writes is taken.
    let mut field = orderid();
    FixFieldMut::new(&mut field)
        .set_parents(["parentorderid", "origorderid"])
        .expect("two types");
    assert!(FixRegistry::new().add_field(field).expect("a field"));
}

#[test]
fn a_store_writes_the_list_as_the_json_it_is_and_reads_it_back() {
    crate::install::installed();
    let mut registry = FixRegistry::new();
    let mut field = orderid();
    FixFieldMut::new(&mut field)
        .set_parents(["parentorderid", "origorderid"])
        .expect("two types");
    registry.add_field(field).expect("a field");
    let json = registry.into_json().expect("a snapshot");
    assert!(
        json.contains(r#""FIX:parents":["parentorderid","origorderid"]"#),
        "{json}"
    );
    let again = FixRegistry::from_json(&json).expect("the snapshot reads back");
    let read = again.field_by_tag(37).expect("the field");
    assert_eq!(parents(read), ["parentorderid", "origorderid"]);
}

#[test]
fn a_stored_transient_flag_reads_every_spelling_the_crate_reads_for_a_flag() {
    crate::install::installed();
    let transient = |stored: Option<&str>| {
        let mut field = orderid();
        if let Some(stored) = stored {
            field
                .insert_metadata("FIX:transient", stored)
                .expect("inert text");
        }
        FixField::new(&field).is_transient()
    };
    assert!(
        transient(None).expect("a field says nothing"),
        "a field is carried unless it says it is not"
    );
    for stored in ["true", "TRUE", "yes", "Y", "on", "1"] {
        assert!(transient(Some(stored)).expect(stored), "{stored}");
    }
    for stored in ["false", "False", "no", "N", "off", "0"] {
        assert!(!transient(Some(stored)).expect(stored), "{stored}");
    }
    // Text no boolean spells is still a named refusal.
    for stored in ["maybe", "2", "n/a"] {
        let refusal = transient(Some(stored)).expect_err(stored).to_string();
        assert!(refusal.contains("FIX:transient"), "{stored}: {refusal}");
        assert!(
            refusal.contains("true/false, yes/no, y/n, on/off or 1/0"),
            "{stored}: {refusal}"
        );
    }
}

/// What `rust/tests/root/protocol.rs` pins of every view the core mints, pinned
/// of the FIX one the exported builder mints under `Scheme::FIX`: it spells its
/// own prefix and reads back what its mutable twin wrote.
#[test]
fn the_fix_view_spells_its_own_scheme_prefix() {
    crate::install::installed();
    let mut field = DataType::Int64.required_field("probe");
    FixFieldMut::new(&mut field)
        .insert("x", "1")
        .expect("the FIX protocol accepts its own property");

    assert_eq!(field.get_metadata("FIX:x"), Some("1"));
    let view = FixField::new(&field);
    assert_eq!(view.prefix(), "FIX");
    assert_eq!(view.key("x"), "FIX:x");
    assert_eq!(view.get("x"), Some("1"));
    assert_eq!((&view).into_iter().collect::<Vec<_>>(), [("x", "1")]);
    assert_eq!(FixFieldMut::new(&mut field).prefix(), "FIX");
}

/// The named view is the scheme door with the protocol chosen: the same
/// properties as `Field::protocol(&Scheme::FIX)`, and `Scheme::FIX` stays the
/// core's, since a known scheme's properties are read without allocating.
#[test]
fn the_fix_view_is_the_scheme_door_with_the_protocol_chosen() {
    crate::install::installed();
    let mut field = orderid();
    FixFieldMut::new(&mut field)
        .set_parents(["origorderid"])
        .expect("one type");
    FixFieldMut::new(&mut field)
        .insert("x", "1")
        .expect("a property");

    let named = FixField::new(&field);
    let door = field.protocol(&Scheme::FIX);
    assert_eq!(named.prefix(), door.prefix());
    assert_eq!(named.len(), door.len());
    assert_eq!(
        named.iter().collect::<Vec<_>>(),
        door.iter().collect::<Vec<_>>()
    );
    assert_eq!(named.get("tag"), Some("37"));
    assert_eq!(named.get("parents"), Some(r#"["origorderid"]"#));
}

/// The mutable view hands back the typed read view of the field it borrows,
/// so a typed remover reads its own typed prior value, and lends the field.
#[test]
fn the_mutable_fix_view_reads_back_typed_and_lends_its_field() {
    crate::install::installed();
    let mut field = DataType::utf8().nullable_field("OrderID");
    let mut view = FixFieldMut::new(&mut field);
    view.set_tag(37).expect("a tag");
    view.set_parents(["parentorderid", "origorderid"])
        .expect("two types");
    assert_eq!(view.as_protocol().tag().expect("a tag"), Some(37));
    assert_eq!(
        view.as_protocol().parents().collect::<Vec<_>>(),
        ["parentorderid", "origorderid"]
    );
    let lent: &Field = view.as_ref();
    assert_eq!(lent.name(), "OrderID");
    assert_eq!(lent.get_metadata("FIX:tag"), Some("37"));
}

/// `Field` carries no FIX accessor and a metadata snapshot neither: a
/// snapshot reads the FIX keys through the scheme door, by bare name.
#[test]
fn a_metadata_snapshot_reads_the_fix_keys_through_the_scheme() {
    crate::install::installed();
    let mut field = orderid();
    FixFieldMut::new(&mut field)
        .set_parents(["origorderid"])
        .expect("one type");
    field
        .insert_metadata("SPARK:x", "1")
        .expect("another protocol's property");

    let metadata = field.as_metadata();
    let fix = metadata.protocol(&Scheme::FIX);
    assert_eq!(fix.prefix(), "FIX");
    assert_eq!(fix.key("tag"), "FIX:tag");
    assert_eq!(fix.get("tag"), Some("37"));
    assert_eq!(fix.get("parents"), Some(r#"["origorderid"]"#));
    assert_eq!(fix.get("x"), None, "another protocol's key is not FIX's");
    assert_eq!(fix.len(), 2);
}
