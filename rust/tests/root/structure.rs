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

mod partition_by {
    use yggdryl::expression::Projection;
    use yggdryl::{DataType, Field, StructType, TimeUnit, Timezone};

    fn rows() -> Field {
        DataType::from(
            StructType::from_fields([
                DataType::utf8().required_field("venue"),
                DataType::DateTime64 {
                    unit: TimeUnit::Microsecond,
                    timezone: Timezone::NAIVE,
                }
                .required_field("ts"),
                DataType::utf8().required_field("name"),
                DataType::Int64.required_field("price"),
            ])
            .unwrap(),
        )
        .required_field("row")
    }

    fn entries(texts: &[&str]) -> Vec<Projection> {
        texts.iter().map(|text| text.parse().unwrap()).collect()
    }

    #[test]
    fn a_derived_entry_is_named_by_its_alias_or_the_singular_convention() {
        let partitioned = rows()
            .with_partition_by(entries(&[
                "venue",
                "years(ts)",
                "days(ts)",
                "minutes(ts, 15)",
                "truncate(name, 4) as prefix",
            ]))
            .unwrap();
        assert_eq!(
            partitioned.partition_field_names().collect::<Vec<_>>(),
            ["venue", "ts_year", "ts_day", "ts_minutes", "prefix"]
        );
        assert_eq!(
            partitioned.get_metadata("PARTITION:by"),
            Some(
                r#"["venue","years(ts)","days(ts)","minutes(ts, 15)","truncate(name, 4) as prefix"]"#
            )
        );
        // The identity column carries only its mark; a derived column is a
        // marked transform typed by its term.
        let venue = partitioned.get_field_by_path("venue").unwrap();
        assert!(venue.is_partition());
        assert!(!venue.as_transform().is_derived());
        let day = partitioned.get_field_by_path("ts_day").unwrap();
        assert_eq!(day.dtype(), &DataType::date32());
        assert!(day.is_partition());
        assert_eq!(
            day.as_transform()
                .term()
                .unwrap()
                .map(|term| term.to_string()),
            Some("days(ts)".to_owned())
        );
        let prefix = partitioned.get_field_by_path("prefix").unwrap();
        assert_eq!(prefix.dtype(), &DataType::utf8());
        assert_eq!(
            prefix
                .as_transform()
                .term()
                .unwrap()
                .map(|term| term.to_string()),
            Some("truncate(name, 4)".to_owned())
        );
        assert_eq!(
            partitioned
                .partition_by()
                .unwrap()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            [
                "venue",
                "years(ts)",
                "days(ts)",
                "minutes(ts, 15)",
                "truncate(name, 4) as prefix"
            ]
        );
        // Every ordinary column is still there, unmarked.
        assert_eq!(partitioned.field_len(), 8);
        assert!(
            !partitioned
                .get_field_by_path("price")
                .unwrap()
                .is_partition()
        );
    }

    #[test]
    fn redeclaring_replaces_the_marks_and_an_empty_declaration_removes_them() {
        let first = rows()
            .with_partition_by(entries(&["venue", "years(ts)"]))
            .unwrap();
        let second = first
            .with_partition_by(entries(&["weeks(ts) as week"]))
            .unwrap();
        assert_eq!(second.partition_field_names().collect::<Vec<_>>(), ["week"]);
        // The column the first declaration derived stays a column, unmarked.
        let year = second.get_field_by_path("ts_year").unwrap();
        assert!(!year.is_partition());
        assert!(year.as_transform().is_derived());
        assert_eq!(
            second.get_metadata("PARTITION:by"),
            Some(r#"["weeks(ts) as week"]"#)
        );

        let none = second.with_partition_by(Vec::new()).unwrap();
        assert!(!none.has_partition_fields());
        assert_eq!(none.get_metadata("PARTITION:by"), None);
        assert!(none.partition_by().unwrap().is_empty());

        // Re-declaring the same entry is the same schema.
        assert_eq!(
            first
                .with_partition_by(entries(&["venue", "years(ts)"]))
                .unwrap(),
            first
        );
    }

    #[test]
    fn with_partition_fields_is_the_declaration_over_bare_columns() {
        let marked = rows().with_partition_fields(&["venue", "name"]).unwrap();
        assert_eq!(
            marked,
            rows()
                .with_partition_by(entries(&["venue", "name"]))
                .unwrap()
        );
        assert_eq!(
            marked.get_metadata("PARTITION:by"),
            Some(r#"["venue","name"]"#)
        );
        // Marks alone are a declaration too: a layout read off a folder
        // marks without declaring, and the marks are what it partitions by.
        let unmarked_declaration = DataType::from(
            StructType::from_fields([
                DataType::utf8()
                    .required_field("venue")
                    .with_partition(true),
                DataType::Int64.required_field("price"),
            ])
            .unwrap(),
        )
        .required_field("row");
        assert_eq!(unmarked_declaration.get_metadata("PARTITION:by"), None);
        assert_eq!(
            unmarked_declaration.partition_by().unwrap(),
            [Projection::column("venue")]
        );
    }

    #[test]
    fn the_declaration_refuses_what_it_cannot_name_or_find() {
        let error = rows()
            .with_partition_by(entries(&["missing"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("missing"), "{error}");
        let error = rows()
            .with_partition_by(entries(&["price * 2"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("alias"), "{error}");
        let error = rows()
            .with_partition_by(entries(&["years(ts)", "weeks(ts) as ts_year"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("ts_year"), "{error}");
        let error = rows()
            .with_partition_by(entries(&["lower(absent)"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("absent"), "{error}");
        assert!(
            DataType::Int64
                .required_field("id")
                .with_partition_by(Vec::new())
                .is_err()
        );
    }

    #[test]
    fn marks_that_contradict_the_declaration_are_refused_naming_both() {
        // The declaration names `ts`, the marks say `venue`.
        let marked = DataType::from(
            StructType::from_fields([
                DataType::utf8()
                    .required_field("venue")
                    .with_partition(true),
                DataType::Int64.required_field("ts"),
            ])
            .unwrap(),
        );
        let error = Field::from_parts("row", marked, false, [("PARTITION:by", r#"["ts"]"#)])
            .unwrap_err()
            .to_string();
        assert!(error.contains("PARTITION:by [ts]"), "{error}");
        assert!(
            error.contains("\"venue\" the declaration does not name"),
            "{error}"
        );
        let marked = rows().with_partition_fields(&["venue"]).unwrap();
        let error = Field::from_parts(
            "row",
            marked.dtype().clone(),
            false,
            [("PARTITION:by", r#"["years(ts) as venue2"]"#)],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("$.venue"), "{error}");
        // The other way round contradicts nothing: a declared column may be
        // unmarked or absent - a lake leaf stores the rows minus the partition
        // columns under the whole declaration, and an Iceberg table keeps a
        // derived entry's value in its manifest - because `with_partition_by`
        // is what marks, and it is asked for.
        let dtype = rows().dtype().clone();
        for declaration in [r#"["venue"]"#, r#"["absent"]"#, r#"["years(ts)"]"#] {
            Field::from_parts("row", dtype.clone(), false, [("PARTITION:by", declaration)])
                .unwrap();
        }
        let unmarked =
            Field::from_parts("row", dtype, false, [("PARTITION:by", r#"["venue"]"#)]).unwrap();
        assert!(!unmarked.has_partition_fields());
        assert_eq!(
            unmarked.partition_by().unwrap(),
            [Projection::column("venue")]
        );
        assert_eq!(
            unmarked
                .with_partition_by(unmarked.partition_by().unwrap())
                .unwrap()
                .partition_field_names()
                .collect::<Vec<_>>(),
            ["venue"]
        );
    }

    #[test]
    fn removing_columns_keeps_the_declaration_consistent() {
        let partitioned = rows()
            .with_partition_by(entries(&["venue", "years(ts)"]))
            .unwrap();
        // What a leaf stores: the rows minus the partition columns, and no
        // declaration - the folder's layout is the declaration.
        let stored = partitioned.without_partition_fields().unwrap();
        assert_eq!(stored.field_len(), 3);
        assert_eq!(stored.get_metadata("PARTITION:by"), None);
        assert!(!stored.has_partition_fields());
        // Removing one partition column drops its entry alone.
        let narrowed = partitioned.without_fields(&["venue"]).unwrap();
        assert_eq!(
            narrowed.get_metadata("PARTITION:by"),
            Some(r#"["years(ts)"]"#)
        );
        assert_eq!(
            narrowed.partition_field_names().collect::<Vec<_>>(),
            ["ts_year"]
        );
        let kept = partitioned.only_partition_fields().unwrap();
        assert_eq!(kept.field_len(), 2);
        assert_eq!(
            kept.get_metadata("PARTITION:by"),
            Some(r#"["venue","years(ts)"]"#)
        );
    }
}
