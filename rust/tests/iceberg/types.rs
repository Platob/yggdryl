//! `rust/src/iceberg/types.rs`: the Iceberg contract a caller has:
//! expression-driven scans, partition keys as the primary keys, sorted data
//! files, parallel partition writes, and the v3 `unknown` and `variant`
//! types, both read as the variant column.
//!
//! Everything here reaches the crate through `yggdryl::`; the plan counts and
//! grouping the crate alone can see are pinned in
//! `rust/src/iceberg/tests.rs`.

mod iceberg {
    use arrow_array::{Array, BinaryArray, Int64Array, NullArray, RecordBatch};
    use std::sync::Arc;

    use yggdryl::iceberg::{
        FormatVersion, IcebergTable, PartitionSpec, PrimitiveType, schema_from_json,
        schema_into_json,
    };
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, Scalar};

    /// A scratch directory unique to this test and this process.
    fn root(label: &str) -> std::path::PathBuf {
        let mut path = LocalFolder::temporary().unwrap().path().unwrap();
        path.push(format!(
            "yggdryl-iceberg-contract-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn unknown_and_variant_both_read_as_the_variant_column() {
        // Both v3 spellings read as the variant datatype; what keeps an
        // `unknown` column `unknown` is its field's declaration, which a
        // datatype cannot carry.
        for name in ["unknown", "variant"] {
            assert_eq!(
                PrimitiveType::from_str(name).unwrap().into_dtype().unwrap(),
                DataType::Variant,
                "{name}"
            );
        }
        assert_eq!(
            PrimitiveType::from_dtype(&DataType::Variant)
                .unwrap()
                .to_string(),
            "variant"
        );
        // A column of nulls is the datatype a caller may state `unknown` in.
        assert_eq!(
            PrimitiveType::from_dtype(&DataType::Null)
                .unwrap()
                .to_string(),
            "unknown"
        );

        let document: Scalar = yggdryl::json::from_utf8(
            r#"{"type":"struct","schema-id":0,"fields":[
            {"id":1,"name":"id","required":true,"type":"long"},
            {"id":2,"name":"later","required":false,"type":"unknown"},
            {"id":3,"name":"payload","required":false,"type":"variant"}
        ]}"#,
        )
        .unwrap();
        let schema = schema_from_json("row", &document).unwrap();
        let (later, payload) = (&schema.fields()[1], &schema.fields()[2]);
        assert_eq!(later.dtype(), &DataType::Variant);
        assert!(later.as_iceberg().is_unknown());
        assert_eq!(later.get_metadata("ICEBERG:type"), Some("unknown"));
        assert_eq!(payload.dtype(), &DataType::Variant);
        assert!(!payload.as_iceberg().is_unknown());
        assert_eq!(schema_into_json(&schema).unwrap(), document);

        // A v2 table refuses either by name.
        let v2 = root("v3-types-v2");
        let message = IcebergTable::create(
            LocalFolder::new(&v2).unwrap(),
            FormatVersion::V2,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            message.contains("later") && message.contains("unknown"),
            "{message}"
        );

        // A v3 table stores the variant, omits the unknown, and reads both
        // back: the unknown as a variant column of nulls, whatever null type
        // it was written in.
        let v3 = root("v3-types-v3");
        let mut table = IcebergTable::create(
            LocalFolder::new(&v3).unwrap(),
            FormatVersion::V3,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let arrow = schema.into_arrow_schema().unwrap();
        let payload = variant_payload(
            arrow.field(2),
            &[Scalar::from(12_i64), Scalar::from("twelve")],
        );
        let input = Arc::new(arrow_schema::Schema::new(vec![
            arrow.field(0).clone(),
            arrow_schema::Field::new("later", arrow_schema::DataType::Null, true),
            arrow.field(2).clone(),
        ]));
        let batch = RecordBatch::try_new(
            input,
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(NullArray::new(2)),
                Arc::clone(&payload),
            ],
        )
        .unwrap();
        table
            .commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
        let mut reader = table.scan(None).unwrap();
        let read = reader.next().unwrap().unwrap();
        assert_eq!(
            read.schema().field(1).data_type(),
            arrow.field(1).data_type()
        );
        assert_eq!(read.column(1).logical_null_count(), 2);
        assert_eq!(read.column(2), &payload);

        let reopened = IcebergTable::open(LocalFolder::new(&v3).unwrap()).unwrap();
        let stored = reopened.schema().unwrap();
        assert!(stored.fields()[1].as_iceberg().is_unknown());
        assert_eq!(stored.fields()[1].dtype(), &DataType::Variant);
        assert!(!stored.fields()[2].as_iceberg().is_unknown());
        reopened.metadata().unwrap().validate().unwrap();

        let _ = std::fs::remove_dir_all(&v2);
        let _ = std::fs::remove_dir_all(&v3);
    }

    #[test]
    fn an_unknown_column_takes_only_nulls_and_refuses_a_value_by_name() {
        let document: Scalar = yggdryl::json::from_utf8(
            r#"{"type":"struct","schema-id":0,"fields":[
            {"id":1,"name":"id","required":true,"type":"long"},
            {"id":2,"name":"later","required":false,"type":"unknown"}
        ]}"#,
        )
        .unwrap();
        let schema = schema_from_json("row", &document).unwrap();
        let path = root("unknown-values");
        let mut table = IcebergTable::create(
            LocalFolder::new(&path).unwrap(),
            FormatVersion::V3,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let arrow = schema.into_arrow_schema().unwrap();
        let batch = |later: arrow_array::ArrayRef| {
            RecordBatch::try_new(
                Arc::clone(&arrow),
                vec![Arc::new(Int64Array::from(vec![1, 2])), later],
            )
            .unwrap()
        };

        // An absent cell and the variant null are both no value.
        let nulls = variant_payload(arrow.field(1), &[Scalar::Null, Scalar::Null]);
        let absent = arrow_array::new_null_array(arrow.field(1).data_type(), 2);
        for later in [nulls, absent] {
            table
                .commit_append(yggdryl::arrow::batch_reader(
                    Arc::clone(&arrow),
                    [batch(later)],
                ))
                .unwrap();
        }
        let read: usize = table
            .scan(None)
            .unwrap()
            .map(|batch| {
                let batch = batch.unwrap();
                assert_eq!(batch.column(1).logical_null_count(), batch.num_rows());
                batch.num_rows()
            })
            .sum();
        assert_eq!(read, 4);

        // A value would be dropped with the column, so it is refused instead,
        // and nothing is committed.
        let snapshot = table
            .current_snapshot()
            .unwrap()
            .map(|snapshot| snapshot.snapshot_id);
        let value = variant_payload(arrow.field(1), &[Scalar::Null, Scalar::from(7_i64)]);
        let message = table
            .commit_append(yggdryl::arrow::batch_reader(
                Arc::clone(&arrow),
                [batch(value)],
            ))
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("later") && message.contains("unknown") && message.contains('7'),
            "{message}"
        );
        assert_eq!(
            table
                .current_snapshot()
                .unwrap()
                .map(|snapshot| snapshot.snapshot_id),
            snapshot
        );
        let _ = std::fs::remove_dir_all(&path);
    }

    /// One variant column holding each value's encoding, a bare null as the
    /// variant null.
    fn variant_payload(field: &arrow_schema::Field, values: &[Scalar]) -> arrow_array::ArrayRef {
        let arrow_schema::DataType::Struct(children) = field.data_type() else {
            panic!(
                "a variant lays out as the struct of its two binaries, got {}",
                field.data_type()
            );
        };
        let variants: Vec<yggdryl::Variant> = values
            .iter()
            .map(|value| yggdryl::Variant::encode(value).unwrap())
            .collect();
        Arc::new(arrow_array::StructArray::new(
            children.clone(),
            vec![
                Arc::new(BinaryArray::from_iter_values(
                    variants.iter().map(yggdryl::Variant::metadata),
                )) as arrow_array::ArrayRef,
                Arc::new(BinaryArray::from_iter_values(
                    variants.iter().map(yggdryl::Variant::value),
                )) as arrow_array::ArrayRef,
            ],
            None,
        ))
    }

    #[test]
    fn a_decimal_spelled_past_its_width_is_refused_by_the_parameter_that_does_not_fit() {
        // The precision and the scale are narrowed by one rule wherever a
        // foreign schema spells them, and the refusal names which one.
        let refused = |text: &str| PrimitiveType::from_str(text).unwrap_err().to_string();
        let precision = refused("decimal(300, 0)");
        assert!(precision.contains("precision fitting u8"), "{precision}");
        assert!(precision.contains("300"), "{precision}");
        let scale = refused("decimal(9, 200)");
        assert!(scale.contains("scale fitting i8"), "{scale}");
        assert!(scale.contains("200"), "{scale}");
        // What fits the width and is still no decimal is the datatype's
        // refusal, asked where the type is made.
        assert!(
            PrimitiveType::from_str("decimal(200, 0)")
                .and_then(PrimitiveType::into_dtype)
                .is_err()
        );
    }

    #[test]
    fn the_datatype_grammar_reads_every_iceberg_primitive_and_member_as_the_reader_does() {
        // `PrimitiveType::into_dtype` is the one Iceberg mapping, and the
        // grammar reads each name a schema document or a dump writes as the
        // datatype that mapping answers, so a type string never needs
        // translating before `DataType::from_str`.
        for name in [
            "boolean",
            "int",
            "long",
            "float",
            "double",
            "decimal(9, 2)",
            "decimal(9,2)",
            "decimal(38, 18)",
            "date",
            "time",
            "timestamp",
            "timestamptz",
            "timestamp_ns",
            "timestamptz_ns",
            "unknown",
            "variant",
            "string",
            "uuid",
            "fixed[16]",
            "fixed(16)",
            "binary",
        ] {
            let mapped = PrimitiveType::from_str(name)
                .and_then(PrimitiveType::into_dtype)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(
                DataType::from_str(name).unwrap_or_else(|error| panic!("{name}: {error}")),
                mapped,
                "{name}"
            );
        }

        // The reference implementations' struct rendering reads as the schema
        // reader reads the document it renders: one id and one nullability
        // per column, the id under `PARQUET:field_id`.
        let document = yggdryl::json::from_utf8(
            r#"{
                "type": "struct",
                "fields": [
                    {"id": 1, "name": "at", "required": false, "type": "timestamptz"},
                    {"id": 2, "name": "key", "required": true, "type": "fixed[16]"},
                    {"id": 3, "name": "px", "required": false, "type": "decimal(9, 2)"},
                    {"id": 4, "name": "later", "required": false, "type": "unknown"}
                ]
            }"#,
        )
        .unwrap();
        let mut schema = schema_from_json("row", &document).unwrap();
        let rendered = DataType::from_str(
            "struct<1: at: optional timestamptz, 2: key: required fixed[16], \
             3: px: optional decimal(9, 2), 4: later: optional unknown>",
        )
        .unwrap();
        // A type string has no slot for the declaration that keeps a column
        // `unknown`: it reads the variant, which only a schema document
        // declares `unknown`.
        assert!(schema.fields()[3].as_iceberg().is_unknown());
        assert!(!rendered.get_field(3).unwrap().as_iceberg().is_unknown());
        let mut later = schema.fields()[3].clone();
        later.as_iceberg_mut().set_unknown(false).unwrap();
        schema.set_field("later", later).unwrap();
        assert_eq!(&rendered, schema.dtype());
    }
}
