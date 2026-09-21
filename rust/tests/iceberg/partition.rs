//! `rust/src/iceberg/partition.rs`: what a partition document must spell.
//!
//! A spec read from someone else's catalog is only as safe as the reading is
//! strict, so these pin the identifiers, the one synthesizing rule v1 allows,
//! and the duplicates a spec refuses. All of it is `yggdryl::iceberg` API.

use yggdryl::iceberg::{PartitionField, PartitionSpec};

#[test]
fn modern_partition_json_requires_exact_identifiers() {
    for text in [
        r#"{"name":"day","source-id":1,"transform":"identity"}"#,
        r#"{"name":"day","source-id":1,"field-id":2147483648,"transform":"identity"}"#,
    ] {
        let document = yggdryl::json::from_utf8(text).unwrap();
        assert!(PartitionField::from_json(&document).is_err(), "{text}");
    }
    for text in [
        r#"{"fields":[]}"#,
        r#"{"spec-id":2147483648,"fields":[]}"#,
        r#"{"spec-id":1,"fields":[{"name":"day","source-id":1,"transform":"identity"}]}"#,
    ] {
        let document = yggdryl::json::from_utf8(text).unwrap();
        assert!(PartitionSpec::from_json(&document).is_err(), "{text}");
    }
}

#[test]
fn only_the_v1_array_synthesizes_field_ids() {
    let document =
        yggdryl::json::from_utf8(r#"[{"name":"day","source-id":1,"transform":"identity"}]"#)
            .unwrap();
    let spec = PartitionSpec::from_json(&document).unwrap();
    assert_eq!(spec.spec_id, 0);
    assert_eq!(spec.fields[0].field_id, 1000);
}

#[test]
fn a_spec_rejects_duplicate_field_ids_and_names() {
    for fields in [
        r#"[{"name":"a","source-id":1,"field-id":1000,"transform":"identity"},{"name":"b","source-id":2,"field-id":1000,"transform":"identity"}]"#,
        r#"[{"name":"a","source-id":1,"field-id":1000,"transform":"identity"},{"name":"a","source-id":2,"field-id":1001,"transform":"identity"}]"#,
    ] {
        let document =
            yggdryl::json::from_utf8(&format!(r#"{{"spec-id":1,"fields":{fields}}}"#)).unwrap();
        assert!(PartitionSpec::from_json(&document).is_err(), "{fields}");
    }
}
