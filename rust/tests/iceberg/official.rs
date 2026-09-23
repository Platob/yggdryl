//! `rust/src/iceberg/official.rs`: the bridge a caller's document crosses
//! into the official Iceberg model.
//!
//! The bridge reads the caller's document before the official parser does, so
//! these pin that it reads every spelling of an array that document can hold -
//! a run or a column - through `yggdryl::iceberg::TableMetadata::from_json`.

use smol_str::SmolStr;

use yggdryl::iceberg::{FormatVersion, PartitionSpec, Snapshot, TableMetadata};
use yggdryl::{DataType, Field, Scalar, Serie, StructType};

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
