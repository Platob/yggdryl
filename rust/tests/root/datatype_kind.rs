//! `rust/src/datatype_kind.rs`.

mod nested {
    use yggdryl::{DataType, StructType};

    #[test]
    fn the_nested_family_stands_for_a_sequence_a_mapping_and_a_record() {
        use std::collections::BTreeMap;
        use std::sync::Arc;

        use yggdryl::{DataTypeKind, Map, Scalar, Serie, Struct};

        let sequence = Serie::new(Arc::from([Scalar::from(1_i64), Scalar::from(2_i64)]));
        let mapping = Map::new(Arc::from([(Scalar::from("k"), Scalar::from(1_i64))]));
        let record = Struct::new(Arc::new(BTreeMap::from([(
            "id".into(),
            Scalar::from(1_i64),
        )])));

        crate::scalar::assert_family_round_trip(
            vec![
                (
                    Scalar::Serie(sequence.clone()),
                    yggdryl::Value::dtype(&sequence).unwrap(),
                ),
                (
                    Scalar::Map(mapping.clone()),
                    yggdryl::Value::dtype(&mapping).unwrap(),
                ),
                (
                    Scalar::Struct(record.clone()),
                    yggdryl::Value::dtype(&record).unwrap(),
                ),
            ],
            DataTypeKind::Nested,
            &Scalar::from(1_i64),
        );

        // A nested value answers the datatype its children name: a serie of the
        // items, a map of the keys and values, a struct of the fields.
        assert_eq!(
            Scalar::Serie(sequence).dtype().unwrap(),
            DataType::serie(DataType::Int64.required_field("item"))
        );
        assert_eq!(
            Scalar::Map(mapping).dtype().unwrap(),
            DataType::map_of(DataType::utf8(), DataType::Int64, false).unwrap()
        );
        assert_eq!(
            Scalar::Struct(record).dtype().unwrap(),
            DataType::from(
                StructType::from_fields([DataType::Int64.required_field("id")]).unwrap()
            )
        );
    }
}

mod names {
    use yggdryl::DataTypeKind;

    #[test]
    fn names_round_trip_case_insensitively() {
        for kind in DataTypeKind::ALL {
            assert_eq!(DataTypeKind::from_str(kind.as_str()).unwrap(), kind);
            assert_eq!(
                DataTypeKind::from_str(&kind.as_str().to_uppercase()).unwrap(),
                kind
            );
        }
    }

    #[test]
    fn unknown_name_reports_the_input_and_vocabulary() {
        let error = DataTypeKind::from_str("int32").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("\"int32\""), "{message}");
        assert!(message.contains("integer"), "{message}");
    }

    #[test]
    fn categories_are_unique() {
        let mut names: Vec<_> = DataTypeKind::ALL.iter().map(|kind| kind.as_str()).collect();
        names.sort_unstable();
        let total = names.len();
        names.dedup();
        assert_eq!(names.len(), total);
    }
}
