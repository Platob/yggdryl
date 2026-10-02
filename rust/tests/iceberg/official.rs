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

#[test]
fn a_schema_document_keeps_its_v3_columns_whatever_id_it_states() -> yggdryl::Result<()> {
    // The official model has no spelling for either v3 type, and each crosses
    // it as a placeholder its spelling alone restores: no schema id is
    // remembered, so an absent one reads back as a stated one does.
    for id in ["", r#""schema-id":0,"#, r#""schema-id":7,"#] {
        let document = yggdryl::json::from_utf8(&format!(
            r#"{{"type":"struct",{id}"fields":[
                {{"id":1,"name":"later","required":false,"type":"unknown"}},
                {{"id":2,"name":"payload","required":false,"type":"variant"}},
                {{"id":3,"name":"tags","required":false,"type":{{"type":"list","element-id":4,"element":"unknown","element-required":false}}}},
                {{"id":5,"name":"bytes","required":false,"type":"binary"}}
            ]}}"#
        ))?;
        let schema = yggdryl::iceberg::schema_from_json("row", &document)?;
        let fields = schema.fields();
        assert_eq!(fields[0].dtype(), &DataType::Variant, "{id}");
        assert!(fields[0].as_iceberg().is_unknown(), "{id}");
        assert_eq!(fields[1].dtype(), &DataType::Variant, "{id}");
        assert!(!fields[1].as_iceberg().is_unknown(), "{id}");
        let element = fields[2].get_field_at(0).expect("a list has its element");
        assert_eq!(element.dtype(), &DataType::Variant, "{id}");
        assert!(element.as_iceberg().is_unknown(), "{id}");
        assert_eq!(fields[3].dtype(), &DataType::binary(), "{id}");
        let written = yggdryl::json::into_utf8(&yggdryl::iceberg::schema_into_json(&schema)?)?;
        assert!(!written.contains("fixed"), "{written}");
    }
    Ok(())
}

#[test]
fn a_document_spelling_a_placeholder_width_is_refused_rather_than_read_as_a_v3_type() {
    // A width no document this crate reads can state is what each v3 type
    // crosses the official model as; one spelled in a document would come
    // back as `unknown` or `variant`, so it is refused by name instead.
    for width in [
        "18446744073709551615",
        "18446744073709551614",
        "018446744073709551615",
    ] {
        let document = yggdryl::json::from_utf8(&format!(
            r#"{{"type":"struct","schema-id":0,"fields":[
                {{"id":1,"name":"wide","required":false,"type":"fixed[{width}]"}}
            ]}}"#
        ))
        .unwrap();
        let message = yggdryl::iceberg::schema_from_json("row", &document)
            .unwrap_err()
            .to_string();
        assert!(message.contains(width), "{width}: {message}");
        assert!(message.contains("4294967295"), "{width}: {message}");
    }
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

/// The transforms the official model has no spelling for cross it as
/// reserved bucket counts and come back as themselves: in the document a
/// table writes, in a spec or a sort order the official builder adds, and
/// from a document spelling an alias. Two of them on one source column - and
/// two steps of `minutes[n]` on one - is the case a one-name placeholder
/// would have refused as a redundant partition.
#[test]
fn the_crates_own_transforms_cross_the_official_model_and_come_back() -> yggdryl::Result<()> {
    let spec = PartitionSpec {
        spec_id: 0,
        fields: vec![
            field("ts_minutes", 1000, Transform::Minutes(15)),
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
    assert!(text.contains("\"minutes[15]\""), "{text}");
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
        vec![Transform::Minutes(15), Transform::Week]
    );

    let added = metadata.add_spec(PartitionSpec {
        spec_id: 1,
        fields: vec![
            field("ts_minute", 1002, Transform::Minutes(1)),
            field("ts_half_hour", 1003, Transform::Minutes(30)),
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
        vec![
            Transform::Minutes(1),
            Transform::Minutes(30),
            Transform::Quarter
        ]
    );
    let order_id = metadata.add_sort_order(SortOrder {
        order_id: 1,
        fields: vec![
            SortField {
                source_id: 2,
                transform: Transform::Minutes(15),
                direction: SmolStr::new_static("asc"),
                null_order: SmolStr::new_static("nulls-first"),
            },
            SortField {
                source_id: 2,
                transform: Transform::Quarter,
                direction: SmolStr::new_static("asc"),
                null_order: SmolStr::new_static("nulls-first"),
            },
        ],
    })?;
    let order = metadata
        .sort_orders()
        .iter()
        .find(|order| order.order_id == order_id)
        .expect("the added order");
    assert_eq!(
        order
            .fields
            .iter()
            .map(|field| field.transform)
            .collect::<Vec<_>>(),
        vec![Transform::Minutes(15), Transform::Quarter]
    );
    let text = yggdryl::json::into_utf8(&metadata.clone().into_json()?)?;
    assert!(!text.contains("bucket"), "{text}");
    for name in ["minutes[1]", "minutes[15]", "minutes[30]", "quarter"] {
        assert!(text.contains(&format!("\"{name}\"")), "{text}");
    }

    // A document spelling Spark's plural reads it as an intake spelling,
    // and writes the one canonical name back.
    let aliased = text.replace("\"week\"", "\"weeks\"");
    let read = TableMetadata::from_json(&yggdryl::json::from_utf8(&aliased)?)?;
    assert_eq!(read.default_spec()?.fields[1].transform, Transform::Week);
    assert!(yggdryl::json::into_utf8(&read.into_json()?)?.contains("\"week\""));

    // A step the reserved counts cannot carry, or none, or any spelling but
    // the bracketed one, is refused by its spelling rather than read as a
    // bucket.
    for refused in [
        "minutes[0]",
        "minutes[2147483646]",
        "minutes",
        "minutes(15)",
        "minutes[-15]",
    ] {
        let spelled = text.replace("\"minutes[15]\"", &format!("\"{refused}\""));
        assert!(
            TableMetadata::from_json(&yggdryl::json::from_utf8(&spelled)?).is_err(),
            "{refused}"
        );
    }
    Ok(())
}

/// A real bucket keeps its count across the model. A count above `i32::MAX`
/// is one no table can state, and it is exactly the space the crate's own
/// transforms cross the official model in, so one stated by a caller or by
/// a document is refused by its count before anything is bridged - never
/// read back as the `minutes[n]`, `week` or `quarter` that count carries.
#[test]
fn a_reserved_bucket_count_is_refused_by_its_count_never_read_as_a_bridged_transform()
-> yggdryl::Result<()> {
    let spec = |transform| PartitionSpec {
        spec_id: 0,
        fields: vec![PartitionField {
            source_id: 1,
            field_id: 1000,
            name: "id_bucket".into(),
            transform,
        }],
    };
    let metadata = TableMetadata::new(
        FormatVersion::V2,
        "file:///tmp/official-bucket",
        timestamp_schema(),
        spec(Transform::Bucket(i32::MAX as u32)),
    )?;
    let text = yggdryl::json::into_utf8(&metadata.clone().into_json()?)?;
    let read = TableMetadata::from_json(&metadata.clone().into_json()?)?;
    assert_eq!(
        read.default_spec()?.fields[0].transform,
        Transform::Bucket(i32::MAX as u32)
    );

    // `bucket[2147483663]` is what `minutes[15]` crosses as, 4294967294
    // `quarter` and 4294967295 `week`; 2147483648 carries nothing.
    for count in [2_147_483_648_u32, 2_147_483_663, u32::MAX - 1, u32::MAX] {
        let built = TableMetadata::new(
            FormatVersion::V2,
            "file:///tmp/official-bucket",
            timestamp_schema(),
            spec(Transform::Bucket(count)),
        )
        .unwrap_err()
        .to_string();
        assert!(built.contains(&format!("bucket[{count}]")), "{built}");

        let spelled = text.replace("\"bucket[2147483647]\"", &format!("\"bucket[{count}]\""));
        assert_ne!(spelled, text, "the document states the bucket");
        let read = TableMetadata::from_json(&yggdryl::json::from_utf8(&spelled)?)
            .unwrap_err()
            .to_string();
        assert!(read.contains(&format!("bucket[{count}]")), "{read}");

        let mut evolved = metadata.clone();
        let added = evolved
            .add_spec(PartitionSpec {
                spec_id: 1,
                fields: vec![PartitionField {
                    source_id: 1,
                    field_id: 1001,
                    name: "id_wide".into(),
                    transform: Transform::Bucket(count),
                }],
            })
            .unwrap_err()
            .to_string();
        assert!(added.contains(&format!("bucket[{count}]")), "{added}");
        let ordered = evolved
            .add_sort_order(SortOrder {
                order_id: 2,
                fields: vec![SortField {
                    source_id: 1,
                    transform: Transform::Bucket(count),
                    direction: SmolStr::new_static("asc"),
                    null_order: SmolStr::new_static("nulls-first"),
                }],
            })
            .unwrap_err()
            .to_string();
        assert!(ordered.contains(&format!("bucket[{count}]")), "{ordered}");
        assert_eq!(evolved, metadata, "a refused update changes nothing");
    }
    Ok(())
}

/// No other implementation knows the crate's own transforms, and the one
/// this crate validates with - iceberg-rust 0.10 - does not read a name it
/// does not know as `unknown`: it refuses the whole document. That is the
/// interoperability the Iceberg page states, so it is pinned against the
/// official crate itself, on the document as this crate writes it to disk.
#[test]
fn the_official_crate_refuses_a_document_naming_the_crates_own_transforms() -> yggdryl::Result<()> {
    use iceberg_official::spec::{
        TableMetadata as OfficialTableMetadata, Transform as OfficialTransform,
    };

    for (transform, name) in [
        (Transform::Minutes(15), "minutes[15]"),
        (Transform::Week, "week"),
        (Transform::Quarter, "quarter"),
    ] {
        let metadata = TableMetadata::new(
            FormatVersion::V2,
            "file:///tmp/official-unbridged",
            timestamp_schema(),
            PartitionSpec {
                spec_id: 0,
                fields: vec![field("ts_period", 1000, transform)],
            },
        )?;
        let text = yggdryl::json::into_utf8(&metadata.into_json()?)?;
        assert!(text.contains(&format!("\"{name}\"")), "{text}");
        assert!(
            serde_json::from_str::<OfficialTableMetadata>(&text).is_err(),
            "{name}: the official crate reads a document naming it"
        );
        let refusal = name.parse::<OfficialTransform>().unwrap_err().to_string();
        assert!(refusal.contains("is invalid"), "{name}: {refusal}");
        assert!(refusal.contains(name), "{name}: {refusal}");

        // The same document under a transform the specification names is
        // one the official crate reads.
        let standard = text.replace(&format!("\"{name}\""), "\"day\"");
        let read: OfficialTableMetadata =
            serde_json::from_str(&standard).expect("a document naming day");
        assert_eq!(
            read.default_partition_spec().fields()[0].transform,
            OfficialTransform::Day
        );
    }
    Ok(())
}
