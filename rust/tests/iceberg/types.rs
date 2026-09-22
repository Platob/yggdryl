//! `rust/src/iceberg/types.rs`: the Iceberg contract a caller has:
//! expression-driven scans, partition keys as the primary keys, sorted data
//! files, parallel partition writes, and the v3 `unknown` and `variant`
//! types.
//!
//! Everything here reaches the crate through `yggdryl::`; the plan counts and
//! grouping the crate alone can see are pinned in
//! `rust/src/iceberg/tests.rs`.

mod iceberg {
    use arrow_array::{Array, BinaryArray, Int64Array, NullArray, RecordBatch};
    use std::sync::Arc;

    use yggdryl::iceberg::{
        FormatVersion, PartitionSpec, PrimitiveType, Table, schema_from_json, schema_into_json,
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
    fn unknown_is_the_null_column_and_variant_is_the_semi_structured_one() {
        // The type mapping, both directions, and why the two are not one thing:
        // `unknown` has no values, `variant` has values that carry their type.
        assert_eq!(
            PrimitiveType::from_str("unknown")
                .unwrap()
                .into_dtype()
                .unwrap(),
            DataType::Null
        );
        assert_eq!(
            PrimitiveType::from_str("variant")
                .unwrap()
                .into_dtype()
                .unwrap(),
            DataType::Variant
        );
        assert_eq!(
            PrimitiveType::from_dtype(&DataType::Null)
                .unwrap()
                .to_string(),
            "unknown"
        );
        assert_eq!(
            PrimitiveType::from_dtype(&DataType::Variant)
                .unwrap()
                .to_string(),
            "variant"
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
        assert_eq!(schema.fields()[1].dtype(), &DataType::Null);
        assert_eq!(schema.fields()[2].dtype(), &DataType::Variant);
        assert_eq!(schema_into_json(&schema).unwrap(), document);

        // A v2 table refuses either by name.
        let v2 = root("v3-types-v2");
        let message = Table::create(
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

        // A v3 table stores the variant, omits the unknown, and reads both back.
        let v3 = root("v3-types-v3");
        let mut table = Table::create(
            LocalFolder::new(&v3).unwrap(),
            FormatVersion::V3,
            schema.clone(),
            PartitionSpec::unpartitioned(),
        )
        .unwrap();
        let arrow = schema.into_arrow_schema().unwrap();
        let arrow_schema::DataType::Struct(children) = arrow.field(2).data_type() else {
            panic!(
                "a variant lays out as the struct of its two binaries, got {}",
                arrow.field(2).data_type()
            );
        };
        let variants = [
            yggdryl::Variant::encode(&Scalar::from(12_i64)).unwrap(),
            yggdryl::Variant::encode(&Scalar::from("twelve")).unwrap(),
        ];
        let payload = arrow_array::StructArray::new(
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
        );
        let batch = RecordBatch::try_new(
            Arc::clone(&arrow),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(NullArray::new(2)),
                Arc::new(payload),
            ],
        )
        .unwrap();
        table
            .commit_append(yggdryl::arrow::batch_reader(
                batch.schema(),
                [batch.clone()],
            ))
            .unwrap();
        let mut reader = table.scan(None).unwrap();
        let read = reader.next().unwrap().unwrap();
        assert_eq!(read, batch);
        assert_eq!(read.column(1).logical_null_count(), 2);

        let reopened = Table::open(LocalFolder::new(&v3).unwrap()).unwrap();
        let stored = reopened.schema().unwrap();
        assert_eq!(stored.fields()[1].dtype(), &DataType::Null);
        assert_eq!(stored.fields()[2].dtype(), &DataType::Variant);
        reopened.metadata().validate().unwrap();

        let _ = std::fs::remove_dir_all(&v2);
        let _ = std::fs::remove_dir_all(&v3);
    }
}
