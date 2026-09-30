//! `rust/src/fix/field.rs`: what a field states under `FIX:parents` - the
//! identifier types holding the parents of the identifier it states, nearest
//! first - written folded and once each, read without a copy, and held to
//! that form by every registry that takes the field.

use yggdryl::{DataType, Error, Field, FixRegistry};

fn orderid() -> Field {
    let mut field = DataType::utf8().nullable_field("OrderID");
    field.as_fix_mut().set_tag(37).expect("a tag");
    field
}

fn parents(field: &Field) -> Vec<&str> {
    field.as_fix().parents().collect()
}

#[test]
fn a_list_is_written_folded_in_the_order_given_and_read_back() {
    let mut field = orderid();
    assert!(parents(&field).is_empty(), "a field states none until told");
    field
        .as_fix_mut()
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
    field
        .as_fix_mut()
        .set_parents(["origorderid"])
        .expect("one type");
    assert_eq!(parents(&field), ["origorderid"]);
}

#[test]
fn an_empty_list_removes_the_property_and_remove_answers_what_it_held() {
    let mut field = orderid();
    field
        .as_fix_mut()
        .set_parents(["origorderid"])
        .expect("one type");
    assert_eq!(
        field.get_metadata("FIX:parents"),
        Some(r#"["origorderid"]"#)
    );
    field
        .as_fix_mut()
        .set_parents::<[&str; 0], &str>([])
        .expect("none");
    assert_eq!(field.get_metadata("FIX:parents"), None);
    assert!(parents(&field).is_empty());

    field
        .as_fix_mut()
        .set_parents(["parentorderid"])
        .expect("one type");
    assert_eq!(
        field.as_fix_mut().remove_parents().as_deref(),
        Some(r#"["parentorderid"]"#)
    );
    assert_eq!(field.as_fix_mut().remove_parents(), None);
    assert!(parents(&field).is_empty());
}

#[test]
fn a_spelling_no_type_folds_from_and_a_repeat_are_refused_and_the_field_stands() {
    let mut field = orderid();
    field
        .as_fix_mut()
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
        let refusal = field
            .as_fix_mut()
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
    field
        .as_fix_mut()
        .set_parents(["parentorderid", "origorderid"])
        .expect("two types");
    assert!(FixRegistry::new().add_field(field).expect("a field"));
}

#[test]
fn a_store_writes_the_list_as_the_json_it_is_and_reads_it_back() {
    let mut registry = FixRegistry::new();
    let mut field = orderid();
    field
        .as_fix_mut()
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
