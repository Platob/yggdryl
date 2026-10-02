//! `rust/src/iceberg/metadata.rs`: what a metadata document must spell.
//!
//! A table document arrives from someone else's writer, so these pin the
//! readings that must fail - duplicate descriptor ids, a negative counter, a
//! v1 `refs` map that disagrees with the current snapshot, a layout bound to a
//! schema the table no longer retains - rather than the round trip, which is
//! pinned in `rust/tests/iceberg/mod_.rs`.

use smol_str::SmolStr;

use yggdryl::iceberg::{
    FormatVersion, PartitionField, PartitionSpec, Snapshot, SnapshotRef, SortField, SortOrder,
    TableMetadata, Transform, assign_field_ids,
};
use yggdryl::{DataType, Field, Scalar, StructType, TimeUnit, Timezone};

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
        .iter()
        .map(std::borrow::Cow::into_owned)
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
        .iter()
        .map(std::borrow::Cow::into_owned)
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

/// `id` (1) an `int64`, `venue` (2) a `utf8`, `day` (3) a `date32` and `ts`
/// (4) a microsecond timestamp.
fn period_sources() -> Field {
    let mut schema = StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
        DataType::date32().nullable_field("day"),
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

fn period_spec(spec_id: i32, source_id: i32, transform: Transform) -> PartitionSpec {
    PartitionSpec {
        spec_id,
        fields: vec![PartitionField {
            source_id,
            field_id: 1000 + spec_id,
            name: format!("period_{spec_id}").into(),
            transform,
        }],
    }
}

fn period_order(order_id: i64, source_id: i32, transform: Transform) -> SortOrder {
    SortOrder {
        order_id,
        fields: vec![SortField {
            source_id,
            transform,
            direction: SmolStr::new_static("asc"),
            null_order: SmolStr::new_static("nulls-first"),
        }],
    }
}

/// The crate's own transforms cross the official model as buckets, which
/// bind to an `int64` or a `utf8` as readily as to a timestamp; the source
/// each of them reads is judged as the specification's own time transforms
/// are, wherever a spec or an order is bound to a schema - a table built, a
/// document read, a spec or an order added - and refused naming the
/// transform and the source's type.
#[test]
fn a_transform_of_the_crates_own_binds_only_to_a_source_it_reads() {
    let refusals = [
        // `id`, `venue`: no period reads a number or a text.
        (1, Transform::Minutes(15), "int64"),
        (1, Transform::Week, "int64"),
        (1, Transform::Quarter, "int64"),
        (2, Transform::Minutes(15), "utf8"),
        (2, Transform::Week, "utf8"),
        (2, Transform::Quarter, "utf8"),
        // `day`: a date has no clock to cut into minutes.
        (3, Transform::Minutes(15), "date32"),
    ];
    let named = |message: &str, transform: Transform, dtype: &str| {
        assert!(message.contains(&transform.to_string()), "{message}");
        assert!(message.contains(dtype), "{message}");
    };
    for (source_id, transform, dtype) in refusals {
        let message = TableMetadata::new(
            FormatVersion::V2,
            "file:///tmp/period-sources",
            period_sources(),
            period_spec(0, source_id, transform),
        )
        .unwrap_err()
        .to_string();
        named(&message, transform, dtype);

        let mut metadata = TableMetadata::new(
            FormatVersion::V2,
            "file:///tmp/period-sources",
            period_sources(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let before = metadata.clone();
        let message = metadata
            .add_spec(period_spec(1, source_id, transform))
            .unwrap_err()
            .to_string();
        named(&message, transform, dtype);
        let message = metadata
            .add_sort_order(period_order(1, source_id, transform))
            .unwrap_err()
            .to_string();
        named(&message, transform, dtype);
        assert_eq!(metadata, before, "a refused update changes nothing");

        // A document stating the same spec or order is refused as it reads.
        let document = before.clone().into_json().unwrap();
        let specs =
            Scalar::from_sequence([period_spec(0, source_id, transform).into_json().unwrap()]);
        let spelled = document
            .with_key("partition-specs", specs)
            .unwrap()
            .with_key("last-partition-id", 1000_i64)
            .unwrap();
        let message = TableMetadata::from_json(&spelled).unwrap_err().to_string();
        named(&message, transform, dtype);
        let orders = Scalar::from_sequence([
            SortOrder::unsorted().into_json().unwrap(),
            period_order(1, source_id, transform).into_json().unwrap(),
        ]);
        let spelled = document.with_key("sort-orders", orders).unwrap();
        let message = TableMetadata::from_json(&spelled).unwrap_err().to_string();
        named(&message, transform, dtype);
    }

    // The sources each one reads bind on every path.
    for (source_id, transform) in [
        (4, Transform::Minutes(15)),
        (4, Transform::Week),
        (4, Transform::Quarter),
        (3, Transform::Week),
        (3, Transform::Quarter),
    ] {
        let metadata = TableMetadata::new(
            FormatVersion::V2,
            "file:///tmp/period-sources",
            period_sources(),
            period_spec(0, source_id, transform),
        )
        .unwrap();
        let read = TableMetadata::from_json(&metadata.clone().into_json().unwrap()).unwrap();
        assert_eq!(read.default_spec().unwrap().fields[0].transform, transform);
        let mut evolved = metadata;
        evolved
            .add_spec(period_spec(1, source_id, transform))
            .unwrap();
        evolved
            .add_sort_order(period_order(2, source_id, transform))
            .unwrap();
    }
}

/// A v3 table's metadata over one column of each v3 type beside a `long`
/// and a `binary`, read from the document that spells them.
fn v3_types_metadata(version: FormatVersion) -> TableMetadata {
    let document = yggdryl::json::from_utf8(
        r#"{"type":"struct","schema-id":0,"fields":[
            {"id":1,"name":"id","required":true,"type":"long"},
            {"id":2,"name":"later","required":false,"type":"unknown"},
            {"id":3,"name":"payload","required":false,"type":"variant"},
            {"id":4,"name":"tags","required":false,"type":{"type":"list","element-id":5,"element":"unknown","element-required":false}},
            {"id":6,"name":"bytes","required":false,"type":"binary"}
        ]}"#,
    )
    .unwrap();
    let schema = yggdryl::iceberg::schema_from_json("row", &document).unwrap();
    TableMetadata::new(
        version,
        "file:///tmp/v3-types",
        schema,
        PartitionSpec::unpartitioned(),
    )
    .unwrap()
}

/// The current schema with one column replaced by `column`, its id kept.
fn with_column(metadata: &TableMetadata, column: Field) -> Field {
    let mut schema = metadata.current_schema().unwrap().clone();
    let mut column = column;
    let id = schema
        .get_field(column.name())
        .and_then(|field| field.parquet_field_id().unwrap())
        .unwrap();
    column.set_parquet_field_id(id);
    let name = column.name().to_owned();
    schema.set_field(name.as_str(), column).unwrap();
    schema
}

/// The type the current schema's metadata document spells for one column.
fn spelled(metadata: &TableMetadata, name: &str) -> String {
    let document = metadata.clone().into_json().unwrap();
    let current = document
        .get_key_str("current-schema-id")
        .and_then(Scalar::as_i64)
        .unwrap();
    let schema = document
        .get_key_str("schemas")
        .unwrap()
        .iter()
        .find(|schema| schema.get_key_str("schema-id").and_then(Scalar::as_i64) == Some(current))
        .unwrap()
        .into_owned();
    let field = schema
        .get_key_str("fields")
        .unwrap()
        .iter()
        .find(|field| field.get_key_str("name").and_then(Scalar::as_str) == Some(name))
        .unwrap()
        .into_owned();
    yggdryl::json::into_utf8(field.get_key_str("type").unwrap()).unwrap()
}

#[test]
fn a_schema_evolution_keeps_every_unknown_and_variant_column_as_it_is() {
    // Adding a column is a new schema the official builder numbers, and each
    // v3 type it keeps comes back spelled as it was - never as the binary
    // the official model once read both as.
    let mut metadata = v3_types_metadata(FormatVersion::V3);
    let mut schema = metadata.current_schema().unwrap().clone();
    let mut fields = schema.fields().to_vec();
    fields.push(DataType::utf8().nullable_field("note"));
    schema
        .set_dtype(DataType::from(StructType::from_fields(fields).unwrap()))
        .unwrap();
    let id = metadata.add_schema(schema).unwrap();
    assert_eq!(id, 1);
    metadata.set_current_schema(id).unwrap();

    let current = metadata.current_schema().unwrap();
    let later = current.get_field("later").unwrap();
    assert_eq!(later.dtype(), &DataType::Variant);
    assert!(later.as_iceberg().is_unknown());
    let payload = current.get_field("payload").unwrap();
    assert_eq!(payload.dtype(), &DataType::Variant);
    assert!(!payload.as_iceberg().is_unknown());
    let element = current.get_field("tags").unwrap().get_field_at(0).unwrap();
    assert!(element.as_iceberg().is_unknown());
    assert_eq!(spelled(&metadata, "later"), r#""unknown""#);
    assert_eq!(spelled(&metadata, "payload"), r#""variant""#);
    assert_eq!(spelled(&metadata, "bytes"), r#""binary""#);
    let text = yggdryl::json::into_utf8(&metadata.clone().into_json().unwrap()).unwrap();
    assert!(!text.contains("fixed["), "{text}");

    // A reopened document reads the same.
    let reopened = TableMetadata::from_json(&metadata.into_json().unwrap()).unwrap();
    assert!(
        reopened
            .current_schema()
            .unwrap()
            .get_field("later")
            .unwrap()
            .as_iceberg()
            .is_unknown()
    );
}

#[test]
fn an_unknown_column_promotes_to_any_type_as_a_new_schema() {
    // Each promotion is a change the official model sees, so it is a new
    // schema rather than the current one handed back unchanged.
    let mut promoted_binary = v3_types_metadata(FormatVersion::V3);
    let schema = with_column(&promoted_binary, DataType::binary().nullable_field("later"));
    let id = promoted_binary.add_schema(schema).unwrap();
    assert_eq!(id, 1);
    promoted_binary.set_current_schema(id).unwrap();
    let later = promoted_binary
        .current_schema()
        .unwrap()
        .get_field("later")
        .unwrap();
    assert_eq!(later.dtype(), &DataType::binary());
    assert!(!later.as_iceberg().is_unknown());
    assert_eq!(spelled(&promoted_binary, "later"), r#""binary""#);

    // To variant: the datatype stays, the declaration goes.
    let mut promoted_variant = v3_types_metadata(FormatVersion::V3);
    let schema = with_column(&promoted_variant, DataType::Variant.nullable_field("later"));
    let id = promoted_variant.add_schema(schema).unwrap();
    assert_eq!(id, 1);
    promoted_variant.set_current_schema(id).unwrap();
    let later = promoted_variant
        .current_schema()
        .unwrap()
        .get_field("later")
        .unwrap();
    assert_eq!(later.dtype(), &DataType::Variant);
    assert!(!later.as_iceberg().is_unknown());
    assert_eq!(spelled(&promoted_variant, "later"), r#""variant""#);
}

#[test]
fn nothing_else_changes_into_or_out_of_the_v3_types() {
    let mut unknown = DataType::Variant.nullable_field("payload");
    unknown.as_iceberg_mut().set_unknown(true).unwrap();
    let mut bytes_unknown = DataType::Variant.nullable_field("bytes");
    bytes_unknown.as_iceberg_mut().set_unknown(true).unwrap();
    for (column, from, to) in [
        (
            DataType::binary().nullable_field("payload"),
            "variant",
            "binary",
        ),
        (unknown, "variant", "unknown"),
        (
            DataType::Variant.nullable_field("bytes"),
            "binary",
            "variant",
        ),
        (bytes_unknown, "binary", "unknown"),
    ] {
        let mut metadata = v3_types_metadata(FormatVersion::V3);
        let schema = with_column(&metadata, column);
        let message = metadata.add_schema(schema).unwrap_err().to_string();
        assert!(message.contains("promotion"), "{from} to {to}: {message}");
    }
}

#[test]
fn a_v1_or_v2_table_takes_no_v3_type_through_evolution_either() {
    let document = r#"{"type":"struct","schema-id":0,"fields":[
        {"id":1,"name":"id","required":true,"type":"long"}
    ]}"#;
    let schema =
        yggdryl::iceberg::schema_from_json("row", &yggdryl::json::from_utf8(document).unwrap())
            .unwrap();
    for version in [FormatVersion::V1, FormatVersion::V2] {
        for (column, spelling) in [
            (DataType::Variant.nullable_field("later"), "variant"),
            (DataType::Null.nullable_field("later"), "unknown"),
        ] {
            let mut metadata = TableMetadata::new(
                version,
                "file:///tmp/v2-evolution",
                schema.clone(),
                PartitionSpec::unpartitioned(),
            )
            .unwrap();
            let mut evolved = schema.clone();
            let mut fields = evolved.fields().to_vec();
            fields.push(column);
            evolved
                .set_dtype(DataType::from(StructType::from_fields(fields).unwrap()))
                .unwrap();
            let message = metadata.add_schema(evolved).unwrap_err().to_string();
            assert!(
                message.contains("later") && message.contains(spelling) && message.contains("v3"),
                "{version:?} {spelling}: {message}"
            );
        }
    }
}

mod declared_order {
    //! A sort order read from, and written as, a schema's `SORT:by`.

    use yggdryl::expression::Ordering;
    use yggdryl::iceberg::{SortField, SortOrder, Transform, assign_field_ids};
    use yggdryl::{DataType, Field, SortOptions, StructType, TimeUnit, Timezone};

    fn schema() -> Field {
        let mut schema = DataType::from(
            StructType::from_fields([
                DataType::utf8().required_field("venue"),
                DataType::DateTime64 {
                    unit: TimeUnit::Microsecond,
                    timezone: Timezone::NAIVE,
                }
                .required_field("ts"),
                DataType::Float64.nullable_field("price"),
            ])
            .unwrap(),
        )
        .required_field("row");
        assign_field_ids(&mut schema, 1).unwrap();
        schema
    }

    #[test]
    fn a_declaration_reads_as_an_order_and_an_order_writes_its_keys() {
        let mut declared = schema();
        declared
            .as_sort_mut()
            .set_by_texts(["venue", "days(ts) desc", "price desc nulls first"])
            .unwrap();
        let order = SortOrder::from_schema(1, &declared).unwrap();
        assert_eq!(
            order,
            SortOrder {
                order_id: 1,
                fields: vec![
                    SortField {
                        source_id: 1,
                        transform: Transform::Identity,
                        direction: "asc".into(),
                        null_order: "nulls-last".into(),
                    },
                    SortField {
                        source_id: 2,
                        transform: Transform::Day,
                        direction: "desc".into(),
                        null_order: "nulls-last".into(),
                    },
                    SortField {
                        source_id: 3,
                        transform: Transform::Identity,
                        direction: "desc".into(),
                        null_order: "nulls-first".into(),
                    },
                ],
            }
        );
        let keys = order.into_orderings(&schema()).unwrap();
        assert_eq!(keys, declared.as_sort().by().unwrap().unwrap());
        assert_eq!(
            keys[2],
            Ordering::new(
                "price".parse().unwrap(),
                SortOptions::descending().with_nulls_first(true)
            )
        );
        // A schema declaring nothing is the unsorted order; a bucket has no
        // key to spell and is refused by name on the way out.
        assert_eq!(
            SortOrder::from_schema(1, &schema()).unwrap(),
            SortOrder::unsorted()
        );
        let bucketed = SortOrder {
            order_id: 2,
            fields: vec![SortField {
                source_id: 1,
                transform: Transform::Bucket(4),
                direction: "asc".into(),
                null_order: "nulls-last".into(),
            }],
        };
        let error = bucketed.into_orderings(&schema()).unwrap_err().to_string();
        assert!(error.contains("bucket[4]"), "{error}");
        for entry in ["lower(venue)", "truncate(venue, 0)", "absent desc"] {
            let mut declared = schema();
            declared.as_sort_mut().set_by_texts([entry]).unwrap();
            let error = SortOrder::from_schema(1, &declared)
                .unwrap_err()
                .to_string();
            assert!(error.contains(entry), "{entry}: {error}");
        }
    }
}
