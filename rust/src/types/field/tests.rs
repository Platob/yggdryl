use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};

use super::Field;
use crate::{AsciiEnum, DataType, Error};

#[test]
fn a_field_declares_the_enum_its_ascii_values_name() {
    let side = AsciiEnum::from_members("Side", [("BUY", "B"), ("SELL", "S")]).unwrap();
    let field = Field::new("side", DataType::FixedAscii(4), false)
        .try_with_ascii_enum(&side)
        .unwrap();

    // One reserved document, readable through the `field:` protocol view
    // and through the typed accessor that owns it.
    assert_eq!(
        field.get_metadata("field:enum"),
        Some(r#"{"members":{"BUY":"B","SELL":"S"},"name":"Side"}"#)
    );
    assert_eq!(
        field.as_field_properties().get("enum"),
        field.get_metadata("field:enum")
    );
    assert_eq!(field.ascii_enum().unwrap(), Some(side.clone()));
    assert_eq!(
        field.as_metadata().as_field_properties().get("enum"),
        field.get_metadata("field:enum")
    );

    // The members carry the packed codes of this field's own width.
    assert_eq!(
        side.into_members(field.dtype()).unwrap(),
        [("BUY".into(), 0x4200_0000), ("SELL".into(), 0x5300_0000)]
    );

    // Metadata canonicalizes the document, so one enum is one stored text
    // whichever spelling reached the field.
    let restated = Field::new("side", DataType::FixedAscii(4), false)
        .try_with_metadata(
            "field:enum",
            r#"{"name":"Side","members":{"SELL":"S","BUY":"B"}}"#,
        )
        .unwrap();
    assert_eq!(
        restated.get_metadata("field:enum"),
        field.get_metadata("field:enum")
    );
    assert_eq!(restated.stable_hash(), field.stable_hash());

    // A declaration the width could not store is refused whole.
    let wide = AsciiEnum::from_members("Venue", [("LONG", "EUREX")]).unwrap();
    let mut narrow = Field::new("venue", DataType::FixedAscii(4), false);
    let refused = narrow.set_ascii_enum(&wide).unwrap_err().to_string();
    assert!(refused.contains("at most 4 bytes"), "{refused}");
    assert_eq!(narrow.ascii_enum().unwrap(), None);
    assert!(
        Field::new("venue", DataType::Utf8, false)
            .set_ascii_enum(&wide)
            .is_err()
    );

    // A stored document that is not one is refused where it is written.
    let refused = Field::new("side", DataType::FixedAscii(4), false)
        .try_with_metadata("field:enum", "[]")
        .unwrap_err()
        .to_string();
    assert!(refused.contains("field:enum"), "{refused}");

    let mut removed = field.clone();
    assert_eq!(removed.remove_ascii_enum().unwrap(), Some(side));
    assert_eq!(removed.remove_ascii_enum().unwrap(), None);
    assert_eq!(removed, Field::new("side", DataType::FixedAscii(4), false));
}

#[test]
fn canonical_display_json_and_arrow_round_trip() {
    let field = Field::new(
        "items",
        DataType::list(Field::new("item", DataType::Utf8, true)),
        false,
    )
    .try_with_metadata("source", "a, b")
    .unwrap();

    assert_eq!(Field::from_str(&field.to_string()).unwrap(), field);
    assert_eq!(
        Field::from_json(&field.clone().into_json().unwrap()).unwrap(),
        field
    );
    let arrow = field.clone().into_arrow_ref().unwrap();
    assert_eq!(arrow, field.clone().into_arrow_ref().unwrap());
    assert_eq!(Field::from_arrow(arrow.as_ref()).unwrap(), field);
}

#[test]
fn sql_hive_and_wrapped_forms_parse() {
    assert_eq!(
        Field::from_str("id bigint not null").unwrap().dtype(),
        &DataType::Int64
    );
    assert!(!Field::from_str("id bigint not null").unwrap().is_nullable());
    assert_eq!(
        Field::from_str("['events': array<struct<id:bigint,name:string>>]")
            .unwrap()
            .name(),
        "events"
    );
    assert!(
        !Field::from_str("id bigint  NOT \t NULL")
            .unwrap()
            .is_nullable()
    );
    assert_eq!(Field::from_str("'it''s': string").unwrap().name(), "it's");
    assert_eq!(Field::from_str(r#""a""b": string"#).unwrap().name(), "a\"b");
    assert_eq!(Field::from_str("[a]]b] string").unwrap().name(), "a]b");
}

#[test]
#[allow(deprecated)]
fn arrow_display_and_dictionary_state_round_trip_after_cache_invalidation() {
    let arrow = ArrowField::new_dict(
        "codes",
        ArrowDataType::Dictionary(
            Box::new(ArrowDataType::Int16),
            Box::new(ArrowDataType::Utf8),
        ),
        true,
        42,
        true,
    )
    .with_metadata(std::collections::HashMap::from([(
        "source".to_owned(),
        "ipc".to_owned(),
    )]));
    let field = Field::from_arrow(&arrow).unwrap();
    assert_eq!(Field::from_str(&arrow.to_string()).unwrap(), field);
    assert_eq!(Field::from_str(&field.to_string()).unwrap(), field);
    assert_eq!(field.dictionary_id(), Some(42));
    assert_eq!(field.dictionary_is_ordered(), Some(true));

    let cached = Arc::new(field.clone().into_arrow().unwrap());
    let mut field = Field::from_arrow_ref(Arc::clone(&cached)).unwrap();
    assert!(Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));
    field.set_dictionary_options(42, true).unwrap();
    assert!(Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));
    field.set_dictionary_options(7, false).unwrap();
    assert!(!Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));
    field.set_dictionary_options(42, true).unwrap();

    field.set_name("renamed");
    let rebuilt = field.into_arrow().unwrap();
    assert_eq!(rebuilt.dict_id(), Some(42));
    assert_eq!(rebuilt.dict_is_ordered(), Some(true));

    let shared = Arc::new(arrow);
    let imported = Field::from_arrow_ref(Arc::clone(&shared)).unwrap();
    assert!(Arc::ptr_eq(&shared, &imported.into_arrow_ref().unwrap()));
}

#[test]
fn wrappers_are_bounded_and_nested_errors_use_field_offsets() {
    let accepted = format!(
        "{}id:int64{}",
        "(".repeat(DataType::PARSE_RECURSION_LIMIT),
        ")".repeat(DataType::PARSE_RECURSION_LIMIT)
    );
    assert_eq!(Field::from_str(&accepted).unwrap().name(), "id");
    let rejected_depth = DataType::PARSE_RECURSION_LIMIT + 1;
    let rejected = format!(
        "{}id:int64{}",
        "(".repeat(rejected_depth),
        ")".repeat(rejected_depth)
    );
    assert!(Field::from_str(&rejected).is_err());

    let error = Field::from_str("id: struct<x: definitely_bad>").unwrap_err();
    assert!(matches!(
        error,
        Error::Parse {
            target: "field",
            position: 3..,
            ..
        }
    ));
}

#[test]
fn metadata_updates_are_sorted_atomic_and_cache_aware() {
    let mut field = Field::new("id", DataType::Int64, false);
    field
        .update_metadata([("z", "last"), ("a", "first")])
        .unwrap();
    assert_eq!(
        field.metadata_iter().collect::<Vec<_>>(),
        vec![("a", "first"), ("z", "last")]
    );
    let cached = Arc::new(field.clone().into_arrow().unwrap());
    let mut field = Field::from_arrow_ref(Arc::clone(&cached)).unwrap();
    field.insert_metadata("a", "first").unwrap();
    assert!(Arc::ptr_eq(
        &cached,
        &field.clone().into_arrow_ref().unwrap()
    ));
    assert!(field.update_metadata([("", "bad")]).is_err());
    assert_eq!(field.metadata_len(), 2);
}

#[test]
#[allow(clippy::mutable_key_type)]
fn native_order_hash_and_stable_hash_ignore_cache() {
    let first = Field::new("a", DataType::Int64, false);
    let second = Field::new("b", DataType::Int64, false);
    let mut ordered = BTreeSet::new();
    ordered.insert(second.clone());
    ordered.insert(first.clone());
    assert_eq!(ordered.into_iter().next().unwrap(), first);
    let mut hashed = HashSet::new();
    hashed.insert(second.clone());
    assert!(hashed.contains(&second));
    let before = second.stable_hash();
    second.clone().into_arrow_ref().unwrap();
    assert_eq!(before, second.stable_hash());
}
