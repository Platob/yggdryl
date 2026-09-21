//! `rust/src/mapping.rs`.

mod nested {
    use yggdryl::{DataType, StructType};

    #[test]
    fn a_map_key_is_not_a_borrowed_schema_child() {
        let row =
            StructType::from_fields([DataType::map_of(DataType::utf8(), DataType::Int64, false)
                .unwrap()
                .required_field("mapping")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");

        assert_eq!(
            row.field_by_path("mapping.entries.value").unwrap().name(),
            "value"
        );
        let key = "mapping['entries']";
        assert!(row.get_field_by_path(key).is_none());
        assert!(row.field_by_path(key).is_err());

        let mut set = row.clone();
        assert!(
            set.set_field_by_path(key, DataType::utf8().required_field("replacement"))
                .is_err()
        );
        assert_eq!(set, row);

        let mut removed = row.clone();
        assert!(removed.remove_field_by_path(key).is_err());
        assert_eq!(removed, row);
    }
}
