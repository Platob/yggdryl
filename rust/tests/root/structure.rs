//! `rust/src/structure.rs`.

mod nested {
    use yggdryl::{DataType, Field, StructType};

    #[test]
    fn wide_struct_validation_accepts_unique_names_and_reports_a_late_duplicate() {
        let fields = (0..1_024)
            .map(|index| Field::new(format!("column_{index:04}"), DataType::Int64, false))
            .collect::<Vec<_>>();
        let dtype = DataType::from(StructType::from_fields(fields.clone()).unwrap());
        assert_eq!(dtype.field_len(), 1_024);

        let mut duplicate = fields;
        duplicate.push(Field::new("column_0001", DataType::utf8(), true));
        let error = StructType::from_fields(duplicate)
            .map(DataType::from)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("duplicate field name \"column_0001\"")
        );
    }
}
