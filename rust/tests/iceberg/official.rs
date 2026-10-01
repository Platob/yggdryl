//! `rust/src/iceberg/official.rs`: the bridge a caller's document crosses
//! into the official Iceberg model.
//!
//! The bridge reads the caller's document before the official parser does, so
//! these pin that it reads every spelling of an array that document can hold -
//! a run or a column - through `yggdryl::iceberg::TableMetadata::from_json`.

use smol_str::SmolStr;

use yggdryl::iceberg::{
    FormatVersion, PartitionField, PartitionSpec, Snapshot, SortField, SortOrder, TableMetadata,
    Transform, assign_field_ids,
};
use yggdryl::{DataType, Field, Scalar, Serie, StructType, TimeUnit, Timezone};

/// A v1 table whose one snapshot states its manifests directly.
fn v1_document() -> Scalar {
    let schema = StructType::from_fields([DataType::Int64.required_field("id")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let mut metadata = TableMetadata::new(
        FormatVersion::V1,
        "file:///tmp/official",
        schema,
        PartitionSpec::unpartitioned(),
    )
    .unwrap();
    let snapshot = Snapshot {
        snapshot_id: 7,
        parent_snapshot_id: None,
        sequence_number: None,
        timestamp_ms: metadata.last_updated_ms() + 1,
        manifest_list: SmolStr::new_static(""),
        manifests: Some(vec![SmolStr::new_static("file:///tmp/official/m.avro")]),
        summary: vec![(
            SmolStr::new_static("operation"),
            SmolStr::new_static("append"),
        )],
        schema_id: Some(0),
        encryption_key_id: None,
        first_row_id: None,
        added_rows: None,
    };
    metadata.set_current_snapshot(snapshot).unwrap();
    metadata.into_json().unwrap()
}

#[test]
fn v1_direct_manifests_read_the_same_as_a_column() -> yggdryl::Result<()> {
    let document = v1_document();
    let snapshots = document
        .get_key_str("snapshots")
        .expect("the v1 document states its snapshots");
    let columned = snapshots
        .iter()
        .map(|snapshot| {
            let paths = snapshot.get_key_str("manifests").expect("direct manifests");
            let column = Serie::from_scalars(
                Field::new("item", DataType::utf8(), false),
                paths.iter().map(std::borrow::Cow::into_owned),
            )?;
            snapshot.with_key("manifests", Scalar::from(column))
        })
        .collect::<yggdryl::Result<Vec<_>>>()?;
    let columned = document.with_key("snapshots", Scalar::from_sequence(columned))?;

    let read = TableMetadata::from_json(&columned)?;
    assert_eq!(read, TableMetadata::from_json(&document)?);
    assert_eq!(
        read.current_snapshot()
            .and_then(|snapshot| snapshot.manifests.clone()),
        Some(vec![SmolStr::new_static("file:///tmp/official/m.avro")])
    );
    Ok(())
}

/// A schema with the one timestamp column every time transform reads.
fn timestamp_schema() -> Field {
    let mut schema = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::DateTime64 {
            unit: TimeUnit::Microsecond,
            timezone: Timezone::NAIVE,
        }
        .required_field("ts"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    assign_field_ids(&mut schema, 1).unwrap();
    schema
}

fn field(name: &str, field_id: i32, transform: Transform) -> PartitionField {
    PartitionField {
        source_id: 2,
        field_id,
        name: name.into(),
        transform,
    }
}

/// The five transforms the official model has no spelling for cross it as
/// reserved bucket counts and come back as themselves: in the document a
/// table writes, in a spec or a sort order the official builder adds, and
/// from a document spelling an alias. Two of them on one source column is
/// the case a one-name placeholder would have refused as a redundant
/// partition.
#[test]
fn the_crates_own_transforms_cross_the_official_model_and_come_back() -> yggdryl::Result<()> {
    let spec = PartitionSpec {
        spec_id: 0,
        fields: vec![
            field("ts_qhour", 1000, Transform::QuarterHour),
            field("ts_week", 1001, Transform::Week),
        ],
    };
    let mut metadata = TableMetadata::new(
        FormatVersion::V2,
        "file:///tmp/official-transforms",
        timestamp_schema(),
        spec,
    )?;
    let document = metadata.clone().into_json()?;
    let text = yggdryl::json::into_utf8(&document)?;
    assert!(text.contains("\"qhour\""), "{text}");
    assert!(text.contains("\"week\""), "{text}");
    assert!(!text.contains("bucket"), "{text}");

    let read = TableMetadata::from_json(&document)?;
    assert_eq!(read, metadata);
    assert_eq!(
        read.default_spec()?
            .fields
            .iter()
            .map(|field| field.transform)
            .collect::<Vec<_>>(),
        vec![Transform::QuarterHour, Transform::Week]
    );

    let added = metadata.add_spec(PartitionSpec {
        spec_id: 1,
        fields: vec![
            field("ts_minute", 1002, Transform::Minute),
            field("ts_hhour", 1003, Transform::HalfHour),
            field("ts_quarter", 1004, Transform::Quarter),
        ],
    })?;
    assert_eq!(
        metadata
            .spec_by_id(added)
            .expect("the added spec")
            .fields
            .iter()
            .map(|field| field.transform)
            .collect::<Vec<_>>(),
        vec![Transform::Minute, Transform::HalfHour, Transform::Quarter]
    );
    let order_id = metadata.add_sort_order(SortOrder {
        order_id: 1,
        fields: vec![SortField {
            source_id: 2,
            transform: Transform::Quarter,
            direction: SmolStr::new_static("asc"),
            null_order: SmolStr::new_static("nulls-first"),
        }],
    })?;
    let order = metadata
        .sort_orders()
        .iter()
        .find(|order| order.order_id == order_id)
        .expect("the added order");
    assert_eq!(order.fields[0].transform, Transform::Quarter);
    let text = yggdryl::json::into_utf8(&metadata.clone().into_json()?)?;
    assert!(!text.contains("bucket"), "{text}");
    for name in ["minute", "hhour", "quarter"] {
        assert!(text.contains(&format!("\"{name}\"")), "{text}");
    }

    // A document spelling an alias reads, and writes back canonically.
    let aliased = text.replace("\"qhour\"", "\"quarter_hour\"");
    let read = TableMetadata::from_json(&yggdryl::json::from_utf8(&aliased)?)?;
    assert_eq!(
        read.default_spec()?.fields[0].transform,
        Transform::QuarterHour
    );
    assert!(yggdryl::json::into_utf8(&read.into_json()?)?.contains("\"qhour\""));
    Ok(())
}

/// A real bucket keeps its count across the model: only the reserved counts
/// above `i32::MAX`, which no spec can state, carry a transform.
#[test]
fn a_bucket_count_is_never_mistaken_for_a_bridged_transform() -> yggdryl::Result<()> {
    let spec = PartitionSpec {
        spec_id: 0,
        fields: vec![PartitionField {
            source_id: 1,
            field_id: 1000,
            name: "id_bucket".into(),
            transform: Transform::Bucket(i32::MAX as u32),
        }],
    };
    let metadata = TableMetadata::new(
        FormatVersion::V2,
        "file:///tmp/official-bucket",
        timestamp_schema(),
        spec,
    )?;
    let read = TableMetadata::from_json(&metadata.clone().into_json()?)?;
    assert_eq!(
        read.default_spec()?.fields[0].transform,
        Transform::Bucket(i32::MAX as u32)
    );
    assert!(
        Transform::Bucket(i32::MAX as u32 + 1)
            .result_type(&DataType::Int64)
            .is_err(),
        "a count past i32::MAX is refused before any bridge reads it"
    );
    Ok(())
}
