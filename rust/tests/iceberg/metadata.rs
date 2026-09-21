//! `rust/src/iceberg/metadata.rs`: what a metadata document must spell.
//!
//! A table document arrives from someone else's writer, so these pin the
//! readings that must fail - duplicate descriptor ids, a negative counter, a
//! v1 `refs` map that disagrees with the current snapshot, a layout bound to a
//! schema the table no longer retains - rather than the round trip, which is
//! pinned in `rust/tests/iceberg/mod_.rs`.

use smol_str::SmolStr;

use yggdryl::iceberg::{
    FormatVersion, PartitionSpec, Snapshot, SnapshotRef, SortOrder, TableMetadata,
};
use yggdryl::{DataType, Scalar, StructType};

fn document(version: FormatVersion) -> Scalar {
    let schema = StructType::from_fields([DataType::Int64.required_field("id")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    TableMetadata::new(
        version,
        "file:///tmp/strict-metadata",
        schema,
        PartitionSpec::unpartitioned(),
    )
    .unwrap()
    .into_json()
    .unwrap()
}

#[test]
fn normalization_cannot_hide_duplicate_statistics_or_key_ids() {
    for collection in ["statistics", "partition-statistics"] {
        let duplicates =
            yggdryl::json::from_utf8(r#"[{"snapshot-id":7},{"snapshot-id":7}]"#).unwrap();
        let candidate = document(FormatVersion::V2)
            .with_key(collection, duplicates)
            .unwrap();
        let message = TableMetadata::from_json(&candidate)
            .unwrap_err()
            .to_string();
        assert!(message.contains(collection), "{message}");
        assert!(message.contains("more than once"), "{message}");
    }

    let duplicates = yggdryl::json::from_utf8(r#"[{"key-id":"k"},{"key-id":"k"}]"#).unwrap();
    let candidate = document(FormatVersion::V3)
        .with_key("encryption-keys", duplicates)
        .unwrap();
    let message = TableMetadata::from_json(&candidate)
        .unwrap_err()
        .to_string();
    assert!(message.contains("encryption-keys"), "{message}");
    assert!(message.contains("more than once"), "{message}");
}

#[test]
fn sequence_counters_must_be_non_negative() {
    let candidate = document(FormatVersion::V2)
        .with_key("last-sequence-number", -1_i64)
        .unwrap();
    let message = TableMetadata::from_json(&candidate)
        .unwrap_err()
        .to_string();
    assert!(
        message.contains("non-negative last-sequence-number"),
        "{message}"
    );
}

#[test]
fn v1_accepts_only_the_derived_main_ref_emitted_by_pyiceberg() {
    let mut metadata = TableMetadata::from_json(&document(FormatVersion::V1)).unwrap();
    metadata
        .set_current_snapshot(Snapshot {
            snapshot_id: 7,
            parent_snapshot_id: None,
            sequence_number: None,
            timestamp_ms: metadata.last_updated_ms() + 1,
            manifest_list: SmolStr::new_static("file:///tmp/manifest-list.avro"),
            manifests: None,
            summary: vec![(
                SmolStr::new_static("operation"),
                SmolStr::new_static("append"),
            )],
            schema_id: Some(metadata.current_schema_id()),
            encryption_key_id: None,
            first_row_id: None,
            added_rows: None,
        })
        .unwrap();
    let document = metadata.into_json().unwrap();
    assert!(document.get_key_str("refs").is_none());

    let empty = document
        .clone()
        .with_key("refs", yggdryl::json::from_utf8("{}").unwrap())
        .unwrap();
    assert!(TableMetadata::from_json(&empty).is_ok());

    let refs = Scalar::from_mapping([(
        Scalar::from("main"),
        SnapshotRef::branch(7).into_json().unwrap(),
    )])
    .unwrap();
    let candidate = document.clone().with_key("refs", refs).unwrap();
    let loaded = TableMetadata::from_json(&candidate).unwrap();
    assert!(loaded.refs().is_empty());

    let wrong_refs = Scalar::from_mapping([(
        Scalar::from("main"),
        SnapshotRef::branch(8).into_json().unwrap(),
    )])
    .unwrap();
    let message = TableMetadata::from_json(&document.with_key("refs", wrong_refs).unwrap())
        .unwrap_err()
        .to_string();
    assert!(message.contains("current-snapshot-id"), "{message}");
}

#[test]
fn sort_order_json_has_no_implicit_fields_or_options() {
    for text in [
        r#"{"order-id":0}"#,
        r#"{"order-id":1,"fields":[{"source-id":1,"transform":"identity","direction":"asc"}]}"#,
        r#"{"order-id":1,"fields":[{"source-id":2147483648,"transform":"identity","direction":"asc","null-order":"nulls-first"}]}"#,
        r#"{"order-id":0,"fields":[{"source-id":1,"transform":"identity","direction":"asc","null-order":"nulls-first"}]}"#,
    ] {
        let value = yggdryl::json::from_utf8(text).unwrap();
        assert!(SortOrder::from_json(&value).is_err(), "{text}");
    }
}

#[test]
fn every_historical_layout_must_bind_to_a_retained_schema() {
    let mut partition_document = document(FormatVersion::V2);
    let mut specs: Vec<Scalar> = partition_document
        .get_key_str("partition-specs")
        .unwrap()
        .sequence_iter()
        .cloned()
        .collect();
    specs.push(
        yggdryl::json::from_utf8(
            r#"{"spec-id":1,"fields":[{"source-id":999,"field-id":1000,"name":"missing","transform":"identity"}]}"#,
        )
        .unwrap(),
    );
    partition_document = partition_document
        .with_key("partition-specs", Scalar::from_sequence(specs))
        .unwrap();
    let message = TableMetadata::from_json(&partition_document)
        .unwrap_err()
        .to_string();
    assert!(message.contains("partition spec 1"), "{message}");
    assert!(message.contains("retained schema"), "{message}");

    let mut sort_document = document(FormatVersion::V2);
    let mut orders: Vec<Scalar> = sort_document
        .get_key_str("sort-orders")
        .unwrap()
        .sequence_iter()
        .cloned()
        .collect();
    orders.push(
        yggdryl::json::from_utf8(
            r#"{"order-id":1,"fields":[{"source-id":999,"transform":"identity","direction":"asc","null-order":"nulls-first"}]}"#,
        )
        .unwrap(),
    );
    sort_document = sort_document
        .with_key("sort-orders", Scalar::from_sequence(orders))
        .unwrap();
    let message = TableMetadata::from_json(&sort_document)
        .unwrap_err()
        .to_string();
    assert!(message.contains("sort order 1"), "{message}");
    assert!(message.contains("retained schema"), "{message}");
}
